use super::echo;
use crate::domain::types::AppError;
use hound::{SampleFormat, WavReader, WavSpec, WavWriter};
use std::path::{Path, PathBuf};

const WINDOW_MS: i64 = 30;
const TRANSCRIPTION_COHERENCE_GAP_MS: i64 = 2_500;
const NORMALIZE_TARGET_PEAK: f32 = 0.75;
const NORMALIZE_MIN_GAIN: f32 = 1.25;
const NORMALIZE_MAX_GAIN: f32 = 32.0;
const TRANSCRIPTION_SAMPLE_RATE: u32 = 16_000;
const TRANSCRIPTION_CHANNELS: u16 = 1;
/// Raw interleaved input held per normalization pass: 32 KiB of PCM16.
const NORMALIZATION_CHUNK_SAMPLE_BUDGET: usize = 16 * 1024;
/// The longest audio one final-transcription request carries: the cap on
/// same-source turn coalescing, on a pack of speech turns, and on fixed
/// splitting. 600 s of normalized PCM16 mono at 16 kHz is 19.2 MB, under
/// Groq's 25 MB upload cap; it is the chunk Groq's own long-audio example
/// uses, and an hour of one Source's speech is six requests instead of 120.
pub const FINAL_TRANSCRIPTION_SPAN_MS: i64 = 600 * 1000;
/// Audio a fixed split repeats at the start of the next chunk, as in Groq's
/// long-audio example, so a word cut at the boundary is heard whole once;
/// the repeated text is dropped when the chunks are joined.
pub const SPLIT_OVERLAP_MS: i64 = 10 * 1000;
/// The span cap before Groq rate limits shaped it. A session whose turns were
/// already transcribed in this layout keeps it, so no finished turn is sent
/// again (durable job ids come from the turn's bounds).
pub const LEGACY_TRANSCRIPTION_SPAN_MS: i64 = 30 * 1000;
/// Pre-roll backfilled ahead of a detected turn onset when extracting its WAV,
/// so the first phonemes that fall below the VAD activity threshold (or before
/// the sustained-activity run) are still transcribed. See JUN-110.
const TURN_PRE_ROLL_MS: i64 = 150;
/// RMS below which a normalized audio window carries no transcribable speech.
/// Applied to already-gained audio (chunk-skip before API calls), so a track
/// this quiet after up to 32x normalization is genuinely silent.
const SILENCE_RMS_FLOOR: f32 = 0.012;
/// A single loud callback, click, or device-start transient is not speech. A
/// source must sustain signal across this many consecutive RMS windows before
/// Clovy sends it to a transcription provider. This remains short enough for a
/// brief reply such as "yes" while rejecting the startup spike seen in broken
/// CoreAudio tap captures.
const MIN_TRANSCRIBABLE_ACTIVITY_MS: i64 = 180;
/// Activity `min_rms` for the system lane's turn detection. The pre-transcription
/// silence pre-filter must not drop below what detection can still find, so both
/// derive from this one constant.
pub const SYSTEM_DETECTION_MIN_RMS: f32 = 0.006;
/// How much louder the system source must be than the microphone, both measured
/// over the same overlapping span, to treat that span of the microphone turn as
/// echo of system audio. Echo re-captured through the air is heavily attenuated,
/// so a clearly louder system source means the microphone only heard the speaker,
/// not the user. Kept conservative so genuine simultaneous mic speech (comparable
/// energy over the overlap) is kept. Level evidence only: used per frame as the
/// tie-breaker when correlation is ambiguous, and per overlap span when no echo
/// lag could be established at all.
const ECHO_DOMINANCE_RATIO: f32 = 3.0;
/// Per-frame |NCC| against the lag-aligned system reference at or above which
/// the frame is speaker bleed regardless of level: bleed is a scaled copy of
/// the reference, so it correlates highly even when quiet.
const ECHO_SIMILARITY_TRIM: f32 = 0.55;
/// Per-frame |NCC| at or below which the frame is genuine microphone speech
/// regardless of level: the user's own voice does not correlate with the
/// system reference, even when the system source is much louder.
const ECHO_SIMILARITY_KEEP: f32 = 0.25;
/// Energy loss after cancelling the system reference out of a microphone
/// frame at or above which the frame was speaker bleed (echo). Genuine
/// microphone speech is roughly orthogonal to the reference and survives the
/// cancellation nearly intact.
const ECHO_ERLE_TRIM_DB: f32 = 10.0;
/// Minimum length for the genuine remainder of a trimmed microphone turn.
/// Deliberately far below the detector's `min_turn_ms`: a remainder already
/// sits inside a detected turn AND the echo evidence judged it genuine, so
/// holding it to the detector's floor (which guards against room-noise false
/// positives) would silently delete short real replies ("yes", "next") that
/// follow trimmed bleed. This floor only suppresses verdict-smoothing
/// slivers at trim boundaries.
const ECHO_TRIM_REMAINDER_MIN_MS: i64 = 300;
/// GCC-PHAT peak-over-mean below which the estimated echo lag is not trusted
/// and echo rejection falls back to level evidence only. Independent signals
/// measure well under this; a real echo path measures well over it.
const ECHO_LAG_MIN_CONFIDENCE: f32 = 8.0;
/// Minimum probe length for a usable GCC-PHAT estimate.
const ECHO_LAG_PROBE_MIN_MS: i64 = 400;
/// Probe cap: longer spans sharpen the estimate with diminishing returns.
const ECHO_LAG_PROBE_MAX_MS: i64 = 4_000;
const ERLE_RMS_FLOOR: f32 = 1.0e-6;

#[derive(Debug, Clone)]
pub struct DetectionSource {
    pub artifact_id: String,
    pub source: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioTurn {
    pub artifact_id: String,
    pub source: String,
    pub source_path: PathBuf,
    pub extraction_start_ms: i64,
    pub start_ms: i64,
    pub end_ms: i64,
    pub turn_index: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SourceActivityEvidence {
    pub max_rms: f32,
    pub active_ms: i64,
    pub longest_active_ms: i64,
    pub has_transcribable_activity: bool,
}

#[derive(Debug, Clone, Copy)]
struct SourceDetectionConfig {
    start_active_ms: i64,
    pre_roll_ms: i64,
    end_silence_ms: i64,
    min_turn_ms: i64,
    merge_gap_ms: i64,
    min_rms: f32,
    noise_multiplier: f32,
}

/// A source's detected turns paired with the audio they came from: the WAV
/// path (for content-similarity evidence) and the per-window RMS energy (for
/// level evidence), so cross-source attribution (echo rejection) can compare
/// the microphone and system sources over the same wall-clock interval.
struct DetectedSource {
    source: String,
    path: PathBuf,
    windows: Vec<f32>,
    turns: Vec<AudioTurn>,
}

/// Cross-source echo evidence for one (microphone, system) pair: the trusted
/// GCC-PHAT lag, plus residual energy windows from the adaptive canceller.
/// The residuals are computed lazily on first use — only recordings that
/// actually produce ambiguous similarity frames pay for the two cancellation
/// passes over the saved sources.
struct EchoEvidence {
    lag_ms: i64,
    mic_path: PathBuf,
    system_path: PathBuf,
    residual_windows: std::cell::OnceCell<Option<Vec<f32>>>,
}

impl EchoEvidence {
    fn residual_windows(&self) -> Option<&[f32]> {
        self.residual_windows
            .get_or_init(|| {
                echo::residual_rms_windows(&self.mic_path, &self.system_path, self.lag_ms)
            })
            .as_deref()
    }
}

/// Session-level trace of echo rejection. Trimming silently rewrites the
/// user's audio timeline, so what was trimmed, under which lag, must be
/// diagnosable after the fact (the processing pipeline records this as a
/// checkpoint alongside `silent_source_dropped`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EchoRejectionReport {
    /// Trusted session echo lag per (microphone, system) source pair, in the
    /// pair iteration order; `None` when no probe was trusted (no
    /// corroborated echo path — nothing was trimmed for that pair).
    pub pair_lags_ms: Vec<Option<i64>>,
    /// Microphone turns that lost at least one span to trimming.
    pub trimmed_turn_count: usize,
    /// Total microphone audio removed as speaker bleed.
    pub trimmed_ms: i64,
    /// Microphone turns trimmed away entirely.
    pub dropped_turn_count: usize,
    /// Artifacts whose detector found turns before echo rejection ran. A
    /// source in this list that ends up with zero turns had them rejected
    /// deliberately — downstream full-file fallbacks must not resurrect it.
    pub detected_turn_artifact_ids: Vec<String>,
    /// Every microphone span trimmed as speaker bleed, on the microphone
    /// timeline. Transcription coalescing must not merge kept turns across
    /// these, or the contiguous segment extraction re-includes the trimmed
    /// audio.
    pub microphone_echo_spans: Vec<(i64, i64)>,
    /// Exact detected-turn intervals removed by trimming, including discarded short remainders.
    pub discarded_microphone_spans: Vec<(i64, i64)>,
    /// Artifacts that lost audio to trimming. Full-source transcription
    /// fallbacks (missing-source resurrection, all-remainders-failed lane
    /// retry) must never run for these: the raw file contains the trimmed
    /// bleed verbatim.
    pub trimmed_artifact_ids: Vec<String>,
}

impl EchoRejectionReport {
    /// Whether echo rejection had a (microphone, system) pair to examine.
    pub fn attempted(&self) -> bool {
        !self.pair_lags_ms.is_empty()
    }
}

pub fn detect_turns(sources: &[DetectionSource]) -> Result<Vec<AudioTurn>, AppError> {
    detect_turns_with_report(sources).map(|(turns, _)| turns)
}

pub fn detect_turns_with_report(
    sources: &[DetectionSource],
) -> Result<(Vec<AudioTurn>, EchoRejectionReport), AppError> {
    let mut detected = Vec::with_capacity(sources.len());
    for source in sources {
        let config = config_for_source(&source.source);
        let (windows, source_turns) = detect_source_turns(source, config)?;
        detected.push(DetectedSource {
            source: source.source.clone(),
            path: source.path.clone(),
            windows,
            turns: source_turns,
        });
    }
    let (mut turns, report) = reject_speaker_echo_turns(detected);
    turns.sort_by(|left, right| {
        left.start_ms
            .cmp(&right.start_ms)
            .then_with(|| source_order(&left.source).cmp(&source_order(&right.source)))
            .then_with(|| left.end_ms.cmp(&right.end_ms))
    });
    for (index, turn) in turns.iter_mut().enumerate() {
        turn.turn_index = index as i64;
    }
    Ok((turns, report))
}

/// Whether a WAV never sustains signal above the silence floor long enough to
/// be speech. A one-window maximum is deliberately insufficient: device-start
/// transients otherwise turn minutes of zeroes into provider hallucinations.
/// If the file can't be read we return `false` so the audio is still attempted
/// rather than silently dropped.
pub fn source_is_effectively_silent(path: &Path) -> bool {
    source_is_effectively_silent_with_floor(path, SILENCE_RMS_FLOOR)
}

/// Like [`source_is_effectively_silent`] but with a caller-supplied floor, so
/// the pre-transcription drop can use the detector's floor while the normalized
/// chunk-skip keeps the higher default.
pub fn source_is_effectively_silent_with_floor(path: &Path, floor: f32) -> bool {
    source_activity_evidence_with_floor(path, floor)
        .is_some_and(|evidence| !evidence.has_transcribable_activity)
}

pub fn source_activity_evidence_with_floor(
    path: &Path,
    floor: f32,
) -> Option<SourceActivityEvidence> {
    let windows = read_rms_windows(path).ok()?;
    let required_windows = windows_for_ms(MIN_TRANSCRIBABLE_ACTIVITY_MS);
    let mut active_run = 0_i64;
    let mut longest_active_run = 0_i64;
    let mut active_windows = 0_i64;
    let mut max_rms = 0.0_f32;
    for rms in &windows {
        max_rms = max_rms.max(*rms);
        if *rms >= floor {
            active_run += 1;
            active_windows += 1;
            longest_active_run = longest_active_run.max(active_run);
        } else {
            active_run = 0;
        }
    }
    Some(SourceActivityEvidence {
        max_rms,
        active_ms: active_windows.saturating_mul(WINDOW_MS),
        longest_active_ms: longest_active_run.saturating_mul(WINDOW_MS),
        has_transcribable_activity: longest_active_run >= required_windows,
    })
}

/// The loudest 30ms RMS window of a source, or `None` if it can't be read.
pub fn source_max_rms(path: &Path) -> Option<f32> {
    read_rms_windows(path)
        .ok()
        .map(|windows| windows.iter().copied().fold(0.0_f32, f32::max))
}

pub fn coalesce_turns_for_transcription(turns: Vec<AudioTurn>) -> Vec<AudioTurn> {
    coalesce_turns_for_transcription_avoiding_echo(turns, &[])
}

/// Coalesce adjacent same-source turns for transcription without bridging
/// trimmed speaker-bleed spans. Segment extraction is contiguous
/// (`write_turn_wav` cuts `[extraction_start_ms, end_ms]`), so merging two
/// kept microphone remainders across a trimmed span would put the bleed
/// audio right back into the transcribed segment, silently undoing echo
/// rejection for every interior bleed shorter than the coherence gap.
pub fn coalesce_turns_for_transcription_avoiding_echo(
    turns: Vec<AudioTurn>,
    microphone_echo_spans: &[(i64, i64)],
) -> Vec<AudioTurn> {
    coalesce_turns_for_transcription_with_max_span(
        turns,
        microphone_echo_spans,
        FINAL_TRANSCRIPTION_SPAN_MS,
    )
}

/// [`coalesce_turns_for_transcription_avoiding_echo`] with an explicit span
/// cap, for a session that keeps the legacy layout.
pub fn coalesce_turns_for_transcription_with_max_span(
    mut turns: Vec<AudioTurn>,
    microphone_echo_spans: &[(i64, i64)],
    max_span_ms: i64,
) -> Vec<AudioTurn> {
    turns.sort_by(|left, right| {
        left.start_ms
            .cmp(&right.start_ms)
            .then_with(|| source_order(&left.source).cmp(&source_order(&right.source)))
            .then_with(|| left.end_ms.cmp(&right.end_ms))
    });
    let mut coalesced: Vec<AudioTurn> = Vec::new();
    for turn in turns {
        if let Some(last) = coalesced.last_mut() {
            let gap_ms = turn.start_ms - last.end_ms;
            let merged_end_ms = last.end_ms.max(turn.end_ms);
            let merged_duration_ms = merged_end_ms - last.start_ms;
            let bridges_trimmed_bleed = turn.source == "microphone"
                && microphone_echo_spans.iter().any(|(span_start, span_end)| {
                    *span_start < turn.start_ms && *span_end > last.end_ms
                });
            if last.source == turn.source
                && gap_ms <= TRANSCRIPTION_COHERENCE_GAP_MS
                && merged_duration_ms <= max_span_ms
                && !bridges_trimmed_bleed
            {
                last.end_ms = merged_end_ms;
                continue;
            }
        }
        coalesced.push(turn);
    }
    for (index, turn) in coalesced.iter_mut().enumerate() {
        turn.turn_index = index as i64;
    }
    coalesced
}

pub fn write_turn_wav(turn: &AudioTurn, output_path: &Path) -> Result<(), AppError> {
    let mut reader = WavReader::open(&turn.source_path)
        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    let spec = reader.spec();
    if spec.sample_format != SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(AppError::new(
            "audio_turn_failed",
            "Only 16-bit PCM WAV turn extraction is supported.",
        ));
    }
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate.max(1) as i64;
    let extraction_start_ms = turn.extraction_start_ms.max(0);
    let start_frame = ((extraction_start_ms * sample_rate) / 1000) as usize;
    let end_frame = ((turn.end_ms.max(extraction_start_ms) * sample_rate) / 1000) as usize;
    let sample_count = end_frame
        .saturating_sub(start_frame)
        .saturating_mul(channels);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    }
    let mut writer = WavWriter::create(output_path, spec)
        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    // seek() repositions by byte offset. The previous .skip() pulled every
    // sample before the turn through the decoder, so extracting turn N
    // re-decoded the recording from its start each time — across a meeting's
    // worth of turns that is quadratic in the recording length.
    let start_frame = u32::try_from(start_frame)
        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    reader
        .seek(start_frame)
        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    for sample in reader.samples::<i16>().take(sample_count) {
        writer
            .write_sample(sample.unwrap_or(0))
            .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    }
    writer
        .finalize()
        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    Ok(())
}

pub fn normalize_wav_for_transcription(
    input_path: &Path,
    output_path: &Path,
) -> Result<PathBuf, AppError> {
    normalize_wav_for_transcription_with_chunk_sample_budget(
        input_path,
        output_path,
        NORMALIZATION_CHUNK_SAMPLE_BUDGET,
    )
}

fn normalize_wav_for_transcription_with_chunk_sample_budget(
    input_path: &Path,
    output_path: &Path,
    chunk_sample_budget: usize,
) -> Result<PathBuf, AppError> {
    let analysis = analyze_wav_for_normalization(input_path, chunk_sample_budget)?;
    let spec = analysis.spec;
    let output_spec = WavSpec {
        channels: TRANSCRIPTION_CHANNELS,
        sample_rate: TRANSCRIPTION_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let already_transcription_ready =
        spec.channels == TRANSCRIPTION_CHANNELS && spec.sample_rate == TRANSCRIPTION_SAMPLE_RATE;
    if analysis.peak <= f32::EPSILON && already_transcription_ready {
        return Ok(input_path.to_path_buf());
    }
    let gain = if analysis.peak <= f32::EPSILON {
        1.0
    } else {
        (NORMALIZE_TARGET_PEAK / analysis.peak).min(NORMALIZE_MAX_GAIN)
    };
    if gain < NORMALIZE_MIN_GAIN && already_transcription_ready {
        return Ok(input_path.to_path_buf());
    }
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
    }
    let temporary_output = (input_path == output_path).then(|| {
        output_path.with_file_name(format!(".normalize-{}.tmp.wav", uuid::Uuid::new_v4()))
    });
    let write_path = temporary_output.as_deref().unwrap_or(output_path);
    let write_result = write_streaming_normalized_wav(
        input_path,
        write_path,
        output_spec,
        analysis.mono_sample_count,
        spec.sample_rate,
        gain,
        chunk_sample_budget,
    );
    if let Err(error) = write_result {
        if temporary_output.is_some() {
            let _ = std::fs::remove_file(write_path);
        }
        return Err(error);
    }
    if temporary_output.is_some() {
        if let Err(error) = crate::filesystem::replace_file(write_path, output_path) {
            let _ = std::fs::remove_file(write_path);
            return Err(AppError::new("audio_normalize_failed", error.to_string()));
        }
    }
    Ok(output_path.to_path_buf())
}

#[derive(Debug, Clone, Copy)]
struct NormalizationAnalysis {
    spec: WavSpec,
    mono_sample_count: usize,
    peak: f32,
}

fn analyze_wav_for_normalization(
    input_path: &Path,
    chunk_sample_budget: usize,
) -> Result<NormalizationAnalysis, AppError> {
    let mut mono_sample_count = 0_usize;
    let mut peak = 0.0_f32;
    let spec = for_each_downmixed_sample(input_path, chunk_sample_budget, |sample| {
        mono_sample_count += 1;
        peak = peak.max(sample.unsigned_abs() as f32 / i16::MAX as f32);
        Ok(())
    })?;
    Ok(NormalizationAnalysis {
        spec,
        mono_sample_count,
        peak,
    })
}

fn write_streaming_normalized_wav(
    input_path: &Path,
    output_path: &Path,
    output_spec: WavSpec,
    mono_sample_count: usize,
    input_rate: u32,
    gain: f32,
    chunk_sample_budget: usize,
) -> Result<(), AppError> {
    let mut writer = WavWriter::create(output_path, output_spec)
        .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
    let mut resampler =
        StreamingLinearResampler::new(mono_sample_count, input_rate, TRANSCRIPTION_SAMPLE_RATE);
    for_each_downmixed_sample(input_path, chunk_sample_budget, |sample| {
        resampler.push(sample, &mut writer, gain)
    })?;
    resampler.ensure_complete()?;
    writer
        .finalize()
        .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
    Ok(())
}

struct StreamingLinearResampler {
    // Global indexes deliberately reproduce the former whole-buffer linear
    // interpolation. Resetting either index at a chunk seam changes bytes and
    // can introduce an audible click.
    ratio: f64,
    total_input_samples: usize,
    output_len: usize,
    next_output_index: usize,
    next_input_index: usize,
    previous_sample: Option<i16>,
}

impl StreamingLinearResampler {
    fn new(total_input_samples: usize, input_rate: u32, output_rate: u32) -> Self {
        let ratio = input_rate as f64 / output_rate as f64;
        let output_len = if total_input_samples == 0 {
            0
        } else if input_rate == output_rate {
            total_input_samples
        } else {
            ((total_input_samples as f64) / ratio).ceil().max(1.0) as usize
        };
        Self {
            ratio,
            total_input_samples,
            output_len,
            next_output_index: 0,
            next_input_index: 0,
            previous_sample: None,
        }
    }

    fn push(
        &mut self,
        sample: i16,
        writer: &mut WavWriter<std::io::BufWriter<std::fs::File>>,
        gain: f32,
    ) -> Result<(), AppError> {
        let current_input_index = self.next_input_index;
        while self.next_output_index < self.output_len {
            let source_pos = self.next_output_index as f64 * self.ratio;
            let left_index = source_pos.floor() as usize;
            let right_index = (left_index + 1).min(self.total_input_samples - 1);
            if right_index > current_input_index {
                break;
            }
            let left = self.sample_at(left_index, current_input_index, sample)? as f64;
            let right = self.sample_at(right_index, current_input_index, sample)? as f64;
            let fraction = source_pos - left_index as f64;
            let resampled = (left + ((right - left) * fraction))
                .round()
                .clamp(i16::MIN as f64, i16::MAX as f64) as i16;
            let amplified = (resampled as f32 * gain).round();
            writer
                .write_sample(amplified.clamp(i16::MIN as f32, i16::MAX as f32) as i16)
                .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
            self.next_output_index += 1;
        }
        self.previous_sample = Some(sample);
        self.next_input_index += 1;
        Ok(())
    }

    fn sample_at(
        &self,
        requested_index: usize,
        current_input_index: usize,
        current_sample: i16,
    ) -> Result<i16, AppError> {
        if requested_index == current_input_index {
            return Ok(current_sample);
        }
        if requested_index + 1 == current_input_index {
            return self.previous_sample.ok_or_else(|| {
                AppError::new(
                    "audio_normalize_failed",
                    "Streaming resampler lost its previous sample.",
                )
            });
        }
        Err(AppError::new(
            "audio_normalize_failed",
            "Streaming resampler advanced past a required sample.",
        ))
    }

    fn ensure_complete(&self) -> Result<(), AppError> {
        if self.next_input_index == self.total_input_samples
            && self.next_output_index == self.output_len
        {
            return Ok(());
        }
        Err(AppError::new(
            "audio_normalize_failed",
            "Streaming resampler did not consume the expected sample counts.",
        ))
    }
}

fn for_each_downmixed_sample(
    input_path: &Path,
    chunk_sample_budget: usize,
    mut on_sample: impl FnMut(i16) -> Result<(), AppError>,
) -> Result<WavSpec, AppError> {
    let mut reader = WavReader::open(input_path)
        .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
    let spec = reader.spec();
    ensure_normalizable_spec(spec)?;
    let channel_count = spec.channels.max(1) as usize;
    let chunk_sample_budget = chunk_sample_budget.max(1);
    let mut chunk = Vec::with_capacity(chunk_sample_budget);
    let mut frame_sum = 0_i32;
    let mut samples_in_frame = 0_usize;
    let mut samples = reader.samples::<i16>();
    loop {
        chunk.clear();
        for _ in 0..chunk_sample_budget {
            let Some(sample) = samples.next() else {
                break;
            };
            chunk.push(sample.unwrap_or(0));
        }
        if chunk.is_empty() {
            break;
        }
        for sample in chunk.iter().copied() {
            frame_sum += sample as i32;
            samples_in_frame += 1;
            if samples_in_frame == channel_count {
                on_sample(
                    (frame_sum / samples_in_frame as i32).clamp(i16::MIN as i32, i16::MAX as i32)
                        as i16,
                )?;
                frame_sum = 0;
                samples_in_frame = 0;
            }
        }
    }
    if samples_in_frame > 0 {
        on_sample(
            (frame_sum / samples_in_frame as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        )?;
    }
    Ok(spec)
}

pub fn split_wav_for_transcription(
    input_path: &Path,
    output_dir: &Path,
    stem: &str,
) -> Result<Vec<PathBuf>, AppError> {
    split_wav_for_transcription_with_overlap(
        input_path,
        output_dir,
        stem,
        FINAL_TRANSCRIPTION_SPAN_MS,
        SPLIT_OVERLAP_MS,
    )
}

pub fn split_wav_for_transcription_with_max_duration(
    input_path: &Path,
    output_dir: &Path,
    stem: &str,
    max_chunk_ms: i64,
) -> Result<Vec<PathBuf>, AppError> {
    split_wav_for_transcription_with_overlap(input_path, output_dir, stem, max_chunk_ms, 0)
}

/// Splits into chunks of at most `max_chunk_ms`, each starting `overlap_ms`
/// before the previous one ends. Streams the input once, writing an
/// overlapped span to both chunks.
pub fn split_wav_for_transcription_with_overlap(
    input_path: &Path,
    output_dir: &Path,
    stem: &str,
    max_chunk_ms: i64,
    overlap_ms: i64,
) -> Result<Vec<PathBuf>, AppError> {
    let mut reader = WavReader::open(input_path)
        .map_err(|error| AppError::new("audio_chunk_failed", error.to_string()))?;
    let spec = reader.spec();
    ensure_normalizable_spec(spec)?;
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate.max(1) as i64;
    let total_frames = reader.duration() as usize;
    let duration_ms = (total_frames as i64 * 1000) / sample_rate;
    let max_chunk_ms = max_chunk_ms.max(1);
    if duration_ms <= max_chunk_ms {
        return Ok(vec![input_path.to_path_buf()]);
    }

    std::fs::create_dir_all(output_dir)
        .map_err(|error| AppError::new("audio_chunk_failed", error.to_string()))?;
    let frames_per_chunk = ((sample_rate * max_chunk_ms) / 1000).max(1) as usize;
    let overlap_frames = ((sample_rate * overlap_ms.max(0)) / 1000) as usize;
    let step_frames = frames_per_chunk
        .saturating_sub(overlap_frames)
        .max(frames_per_chunk / 2)
        .max(1);
    let mut ranges = Vec::new();
    let mut start = 0_usize;
    loop {
        let end = (start + frames_per_chunk).min(total_frames);
        ranges.push((start, end));
        if end == total_frames {
            break;
        }
        start += step_frames;
    }

    let chunks = (0..ranges.len())
        .map(|index| output_dir.join(format!("{stem}-chunk-{index:03}.wav")))
        .collect::<Vec<_>>();
    let mut open: Vec<(usize, WavWriter<std::io::BufWriter<std::fs::File>>)> = Vec::new();
    let mut next_range = 0_usize;
    let mut frame = 0_usize;
    let mut channel_index = 0_usize;

    for sample in reader.samples::<i16>() {
        if channel_index == 0 {
            while next_range < ranges.len() && ranges[next_range].0 == frame {
                let writer = WavWriter::create(&chunks[next_range], spec)
                    .map_err(|error| AppError::new("audio_chunk_failed", error.to_string()))?;
                open.push((ranges[next_range].1, writer));
                next_range += 1;
            }
        }
        let sample = sample.unwrap_or(0);
        for (_, writer) in open.iter_mut() {
            writer
                .write_sample(sample)
                .map_err(|error| AppError::new("audio_chunk_failed", error.to_string()))?;
        }
        channel_index += 1;
        if channel_index == channels {
            channel_index = 0;
            frame += 1;
            let mut index = 0;
            while index < open.len() {
                if open[index].0 == frame {
                    let (_, writer) = open.remove(index);
                    writer
                        .finalize()
                        .map_err(|error| AppError::new("audio_chunk_failed", error.to_string()))?;
                } else {
                    index += 1;
                }
            }
        }
    }

    for (_, writer) in open {
        writer
            .finalize()
            .map_err(|error| AppError::new("audio_chunk_failed", error.to_string()))?;
    }

    debug_assert!(
        !chunks.is_empty(),
        "split_wav_for_transcription: no chunks produced for audio longer than max_chunk_ms"
    );
    Ok(chunks)
}

fn ensure_normalizable_spec(spec: WavSpec) -> Result<(), AppError> {
    if spec.sample_format == SampleFormat::Int && spec.bits_per_sample == 16 {
        return Ok(());
    }
    Err(AppError::new(
        "audio_normalize_failed",
        "Only 16-bit PCM WAV normalization is supported.",
    ))
}

#[cfg(test)]
fn downmix_to_mono(samples: &[i16], channels: u16) -> Vec<i16> {
    let channel_count = channels.max(1) as usize;
    if channel_count == 1 {
        return samples.to_vec();
    }
    samples
        .chunks(channel_count)
        .map(|frame| {
            let sum = frame.iter().map(|sample| *sample as i32).sum::<i32>();
            (sum / frame.len().max(1) as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
        })
        .collect()
}

#[cfg(test)]
fn resample_linear(samples: &[i16], input_rate: u32, output_rate: u32) -> Vec<i16> {
    if samples.is_empty() || input_rate == output_rate {
        return samples.to_vec();
    }
    let ratio = input_rate as f64 / output_rate as f64;
    let output_len = ((samples.len() as f64) / ratio).ceil().max(1.0) as usize;
    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len {
        let source_pos = index as f64 * ratio;
        let left_index = source_pos.floor() as usize;
        let right_index = (left_index + 1).min(samples.len() - 1);
        let fraction = source_pos - left_index as f64;
        let left = samples[left_index.min(samples.len() - 1)] as f64;
        let right = samples[right_index] as f64;
        let sample = left + ((right - left) * fraction);
        output.push(sample.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16);
    }
    output
}

fn detect_source_turns(
    source: &DetectionSource,
    config: SourceDetectionConfig,
) -> Result<(Vec<f32>, Vec<AudioTurn>), AppError> {
    let windows = read_rms_windows(&source.path)?;
    if windows.is_empty() {
        return Ok((windows, Vec::new()));
    }
    let threshold = activity_threshold(&windows, config);
    let start_windows = windows_for_ms(config.start_active_ms);
    let silence_windows = windows_for_ms(config.end_silence_ms);
    let mut turns = Vec::new();
    let mut active_run = 0_i64;
    let mut silence_run = 0_i64;
    let mut current_start: Option<i64> = None;

    for (index, rms) in windows.iter().enumerate() {
        let window_start = index as i64 * WINDOW_MS;
        if *rms >= threshold {
            active_run += 1;
            silence_run = 0;
            if current_start.is_none() && active_run >= start_windows {
                current_start = Some(window_start - ((start_windows - 1) * WINDOW_MS));
            }
        } else {
            active_run = 0;
            if current_start.is_some() {
                silence_run += 1;
                if silence_run >= silence_windows {
                    let end_ms = window_start - ((silence_windows - 1) * WINDOW_MS);
                    push_turn_if_long_enough(
                        &mut turns,
                        source,
                        current_start.take().unwrap(),
                        end_ms,
                        config,
                    );
                    silence_run = 0;
                }
            }
        }
    }

    if let Some(start_ms) = current_start {
        push_turn_if_long_enough(
            &mut turns,
            source,
            start_ms,
            windows.len() as i64 * WINDOW_MS,
            config,
        );
    }
    let turns = merge_close_turns(turns, config.merge_gap_ms);
    debug_assert!(
        config.pre_roll_ms < config.merge_gap_ms,
        "turn pre-roll must be smaller than the merge gap to avoid overlapping extracted audio"
    );
    Ok((
        windows,
        apply_extraction_pre_roll(turns, config.pre_roll_ms),
    ))
}

fn read_rms_windows(path: &Path) -> Result<Vec<f32>, AppError> {
    let mut reader = WavReader::open(path)
        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
    let spec = reader.spec();
    if spec.sample_format != SampleFormat::Int || spec.bits_per_sample != 16 {
        return Err(AppError::new(
            "audio_turn_failed",
            "Only 16-bit PCM WAV turn detection is supported.",
        ));
    }
    let channels = spec.channels.max(1) as usize;
    let sample_rate = spec.sample_rate.max(1) as usize;
    let frames_per_window = ((sample_rate as i64 * WINDOW_MS) / 1000).max(1) as usize;
    let mut windows = Vec::new();
    let mut sum_square = 0.0_f64;
    let mut frames = 0_usize;
    let mut channel_index = 0_usize;
    for sample in reader.samples::<i16>() {
        let normalized = sample.unwrap_or(0) as f32 / i16::MAX as f32;
        sum_square += (normalized as f64).powi(2);
        channel_index += 1;
        if channel_index == channels {
            channel_index = 0;
            frames += 1;
            if frames == frames_per_window {
                windows.push((sum_square / (frames * channels) as f64).sqrt() as f32);
                sum_square = 0.0;
                frames = 0;
            }
        }
    }
    if frames > 0 {
        windows.push((sum_square / (frames * channels) as f64).sqrt() as f32);
    }
    Ok(windows)
}

fn activity_threshold(windows: &[f32], config: SourceDetectionConfig) -> f32 {
    let mut sorted = windows.to_vec();
    sorted.sort_by(|left, right| left.total_cmp(right));
    let percentile_index = ((sorted.len().saturating_sub(1)) as f32 * 0.2).round() as usize;
    let noise_floor = sorted.get(percentile_index).copied().unwrap_or(0.0);
    (noise_floor * config.noise_multiplier)
        .max(noise_floor + config.min_rms)
        .max(config.min_rms)
}

fn push_turn_if_long_enough(
    turns: &mut Vec<AudioTurn>,
    source: &DetectionSource,
    start_ms: i64,
    end_ms: i64,
    config: SourceDetectionConfig,
) {
    if end_ms - start_ms < config.min_turn_ms {
        return;
    }
    turns.push(AudioTurn {
        artifact_id: source.artifact_id.clone(),
        source: source.source.clone(),
        source_path: source.path.clone(),
        extraction_start_ms: start_ms.max(0),
        start_ms: start_ms.max(0),
        end_ms: end_ms.max(start_ms),
        turn_index: 0,
    });
}

fn merge_close_turns(turns: Vec<AudioTurn>, merge_gap_ms: i64) -> Vec<AudioTurn> {
    let mut merged: Vec<AudioTurn> = Vec::new();
    for turn in turns {
        if let Some(last) = merged.last_mut() {
            if turn.start_ms - last.end_ms <= merge_gap_ms {
                last.end_ms = last.end_ms.max(turn.end_ms);
                last.extraction_start_ms = last.extraction_start_ms.min(turn.extraction_start_ms);
                continue;
            }
        }
        merged.push(turn);
    }
    merged
}

fn apply_extraction_pre_roll(mut turns: Vec<AudioTurn>, pre_roll_ms: i64) -> Vec<AudioTurn> {
    for turn in &mut turns {
        turn.extraction_start_ms = (turn.start_ms - pre_roll_ms).max(0);
    }
    turns
}

/// Each source is detected independently, so the microphone source's detector
/// happily emits a turn whenever it crosses its threshold — including when the
/// only thing it heard was a remote participant's voice played through the
/// speakers and bled back into the mic. That misattributes system audio to the
/// microphone. ("Speaker" throughout means the loudspeaker, not a person:
/// attribution stays source-based and no speaker identity is ever inferred,
/// per the Turn glossary rule.) Trim the speaker-bleed spans out of each
/// microphone turn, keeping the genuine remainder; the bled-over speech stays
/// attributed to the system source. A mic turn entirely covered by bleed is
/// trimmed to nothing.
///
/// Bleed is identified by three tiers of evidence:
/// 1. Content similarity (preferred): per-frame normalized cross-correlation
///    against the system source, lag-aligned by a GCC-PHAT estimate of the
///    session's echo path. Bleed is a delayed copy of the reference, so it
///    correlates highly no matter how quiet; the user's own voice does not
///    correlate no matter how loud the system source is.
/// 2. Cancellation depth (ambiguous similarity only): an adaptive canceller
///    learns the reverberant echo path and checks whether the microphone frame
///    collapses after removing the system reference.
/// 3. Level dominance (fallback, when the lag is trusted but the audio cannot
///    be scored): the system turn is clearly louder over the overlap. With no
///    trusted lag at all nothing is trimmed — no corroborated echo path.
fn reject_speaker_echo_turns(
    detected: Vec<DetectedSource>,
) -> (Vec<AudioTurn>, EchoRejectionReport) {
    let system_sources: Vec<&DetectedSource> = detected
        .iter()
        .filter(|candidate| candidate.source == "system")
        .collect();
    let mut turns = Vec::new();
    let mut report = EchoRejectionReport::default();
    report.detected_turn_artifact_ids.extend(
        detected
            .iter()
            .filter(|source| !source.turns.is_empty())
            .map(|source| source.turns[0].artifact_id.clone()),
    );
    for source in &detected {
        if source.source == "microphone" && !system_sources.is_empty() && !source.turns.is_empty() {
            let config = config_for_source(&source.source);
            // One echo-path lag per (microphone, system) pair, reused across
            // every turn: the delay is a property of the session's playback
            // and capture chain, not of any single turn.
            let evidence: Vec<Option<EchoEvidence>> = system_sources
                .iter()
                .map(|system| {
                    let lag_ms = estimate_echo_lag_ms(source, system)?;
                    Some(EchoEvidence {
                        lag_ms,
                        mic_path: source.path.clone(),
                        system_path: system.path.clone(),
                        residual_windows: std::cell::OnceCell::new(),
                    })
                })
                .collect();
            report
                .pair_lags_ms
                .extend(evidence.iter().map(|pair| pair.as_ref().map(|e| e.lag_ms)));
            for turn in &source.turns {
                let echo_spans: Vec<(i64, i64)> = system_sources
                    .iter()
                    .zip(&evidence)
                    .flat_map(|(system, evidence)| {
                        echo_evidence_spans(turn, source, system, evidence.as_ref())
                    })
                    .collect();
                let kept = trim_echo_from_microphone_turn(
                    turn,
                    &echo_spans,
                    ECHO_TRIM_REMAINDER_MIN_MS,
                    config.pre_roll_ms,
                );
                let kept_ms: i64 = kept.iter().map(|kept| kept.end_ms - kept.start_ms).sum();
                let trimmed_ms = (turn.end_ms - turn.start_ms) - kept_ms;
                if trimmed_ms > 0 {
                    report.trimmed_turn_count += 1;
                    report.trimmed_ms += trimmed_ms;
                    if !report.trimmed_artifact_ids.contains(&turn.artifact_id) {
                        report.trimmed_artifact_ids.push(turn.artifact_id.clone());
                    }
                    if kept.is_empty() {
                        report.dropped_turn_count += 1;
                    }
                    report
                        .discarded_microphone_spans
                        .extend(discarded_turn_spans(turn, &kept));
                    report.microphone_echo_spans.extend(
                        echo_spans
                            .iter()
                            .map(|(span_start, span_end)| {
                                (
                                    (*span_start).max(turn.start_ms),
                                    (*span_end).min(turn.end_ms),
                                )
                            })
                            .filter(|(span_start, span_end)| span_end > span_start),
                    );
                }
                turns.extend(kept);
            }
        } else {
            turns.extend(source.turns.iter().cloned());
        }
    }
    (turns, report)
}

/// Estimate the session echo-path lag (how far the system audio's arrival in
/// the microphone trails the system source) by probing system turns with
/// GCC-PHAT until one is trusted. Probing is self-selecting: spans where the
/// microphone holds mostly bleed give a sharp, high-confidence peak, while
/// double-talk or bleed-free spans give a diffuse one. All system turns are
/// candidates — an echo path can appear at any point of the session — but
/// probing stops at the first trusted peak, so ordinary sessions pay for one
/// or two probes. Returns `None` when no probe is trustworthy.
fn estimate_echo_lag_ms(mic: &DetectedSource, system: &DetectedSource) -> Option<i64> {
    for system_turn in system.turns.iter() {
        let probe_end = system_turn
            .end_ms
            .min(system_turn.start_ms + ECHO_LAG_PROBE_MAX_MS);
        if probe_end - system_turn.start_ms < ECHO_LAG_PROBE_MIN_MS {
            continue;
        }
        if let Some(lag_ms) = probe_window_lag_ms(mic, system, system_turn.start_ms, probe_end) {
            return Some(lag_ms);
        }
    }
    None
}

/// Spans of `mic_turn` that are speaker bleed of `system` audio. Trimming
/// requires a corroborated echo path: without one (headphones are the common
/// case), level dominance alone would delete a quiet user's genuine speech —
/// the double-talk failure this gate exists to prevent. Corroboration is the
/// session lag when one was trusted, or a per-overlap probe of a dominated
/// overlap when none was (an echo path can appear mid-session). With a
/// trusted lag, content similarity decides; level dominance covers only
/// spans the content evidence cannot score.
fn echo_evidence_spans(
    mic_turn: &AudioTurn,
    mic: &DetectedSource,
    system: &DetectedSource,
    evidence: Option<&EchoEvidence>,
) -> Vec<(i64, i64)> {
    let mut spans = Vec::new();
    for system_turn in &system.turns {
        let start = mic_turn.start_ms.max(system_turn.start_ms);
        let level_end = mic_turn.end_ms.min(system_turn.end_ms);
        let Some(evidence) = evidence else {
            // No session-wide echo path was corroborated — but one can
            // appear mid-session (headphones unplugged, output routed to
            // speakers). A dominated overlap earns its own probe: genuine
            // quiet speech probes to nothing and stays; late bleed probes
            // to a trusted lag and is scored like any other overlap.
            if level_end > start
                && rms_span_is_echo(&mic.windows, &system.windows, start, level_end)
            {
                if let Some(late_lag_ms) = probe_span_lag_ms(mic, system, start, level_end) {
                    let late_evidence = EchoEvidence {
                        lag_ms: late_lag_ms,
                        mic_path: mic.path.clone(),
                        system_path: system.path.clone(),
                        residual_windows: std::cell::OnceCell::new(),
                    };
                    let similarity_end = mic_turn.end_ms.min(system_turn.end_ms + late_lag_ms);
                    if similarity_end > start {
                        if let Some(similarity_spans) = similarity_echo_spans(
                            mic,
                            system,
                            start,
                            similarity_end,
                            &late_evidence,
                        ) {
                            spans.extend(similarity_spans);
                        }
                    }
                }
            }
            continue;
        };
        // Bleed of system content at time t reaches the microphone at
        // t + lag, so the microphone's bleed trails the system turn by
        // the echo lag; score that tail too or it stays misattributed.
        let similarity_end = mic_turn.end_ms.min(system_turn.end_ms + evidence.lag_ms);
        if similarity_end > start {
            if let Some(similarity_spans) = scored_spans_with_stale_lag_recovery(
                mic,
                system,
                start,
                similarity_end,
                level_end,
                evidence,
            ) {
                spans.extend(similarity_spans);
                continue;
            }
        }
        // The overlap could not be scored (unreadable audio, nothing above
        // the silence floor); the trusted lag corroborates an echo path, so
        // level dominance may decide. It compares the two sources over the
        // same wall-clock interval, so it stays on the unshifted overlap.
        if level_end > start && rms_span_is_echo(&mic.windows, &system.windows, start, level_end) {
            spans.push((start, level_end));
        }
    }
    spans
}

/// Score one overlap with the session lag, recovering from a stale lag: if
/// content similarity clears the whole overlap as genuine while the system
/// source is dominantly louder, that pattern is equally consistent with real
/// bleed decorrelated by a lag that no longer matches the echo path (output
/// device switched mid-session, clock drift over a long recording). Re-probe
/// the overlap itself; a trusted lag that differs re-scores it before the
/// spans are believed.
fn scored_spans_with_stale_lag_recovery(
    mic: &DetectedSource,
    system: &DetectedSource,
    start_ms: i64,
    similarity_end_ms: i64,
    level_end_ms: i64,
    evidence: &EchoEvidence,
) -> Option<Vec<(i64, i64)>> {
    let spans = similarity_echo_spans(mic, system, start_ms, similarity_end_ms, evidence)?;
    let suspicious = spans.is_empty()
        && level_end_ms > start_ms
        && rms_span_is_echo(&mic.windows, &system.windows, start_ms, level_end_ms);
    if !suspicious {
        return Some(spans);
    }
    let Some(reprobed_lag_ms) = probe_span_lag_ms(mic, system, start_ms, level_end_ms) else {
        // No trusted lag on this overlap either: genuine quiet double-talk
        // also looks like this (that is the money case), so believe the
        // content verdict and keep the speech.
        return Some(spans);
    };
    if reprobed_lag_ms == evidence.lag_ms {
        return Some(spans);
    }
    let reprobed_evidence = EchoEvidence {
        lag_ms: reprobed_lag_ms,
        mic_path: mic.path.clone(),
        system_path: system.path.clone(),
        residual_windows: std::cell::OnceCell::new(),
    };
    let similarity_end_ms = similarity_end_ms.max(level_end_ms + reprobed_lag_ms);
    similarity_echo_spans(mic, system, start_ms, similarity_end_ms, &reprobed_evidence)
        .or(Some(spans))
}

/// GCC-PHAT over one span. Bleed can begin partway through a span (an output
/// device switched while the system source kept playing), so up to three
/// windows are probed — the span's head, middle, and tail — and the first
/// trusted peak wins.
fn probe_span_lag_ms(
    mic: &DetectedSource,
    system: &DetectedSource,
    start_ms: i64,
    end_ms: i64,
) -> Option<i64> {
    let window_ms = (end_ms - start_ms).min(ECHO_LAG_PROBE_MAX_MS);
    if window_ms < ECHO_LAG_PROBE_MIN_MS {
        return None;
    }
    let last_offset = (end_ms - start_ms) - window_ms;
    let mut probed = Vec::with_capacity(3);
    for offset in [0, last_offset / 2, last_offset] {
        if probed.contains(&offset) {
            continue;
        }
        probed.push(offset);
        let window_start = start_ms + offset;
        if let Some(lag_ms) =
            probe_window_lag_ms(mic, system, window_start, window_start + window_ms)
        {
            return Some(lag_ms);
        }
    }
    None
}

/// One GCC-PHAT probe window; the single trust gate every lag estimate in
/// this module passes through.
fn probe_window_lag_ms(
    mic: &DetectedSource,
    system: &DetectedSource,
    start_ms: i64,
    end_ms: i64,
) -> Option<i64> {
    let reference = echo::read_span_mono_16k(&system.path, start_ms, end_ms)?;
    let capture = echo::read_span_mono_16k(&mic.path, start_ms, end_ms + echo::ECHO_MAX_LAG_MS)?;
    let max_delay = (echo::ECHO_MAX_LAG_MS * echo::SIMILARITY_SAMPLE_RATE as i64 / 1000) as usize;
    let estimate = echo::gcc_phat_delay(&reference, &capture, max_delay)?;
    (estimate.confidence >= ECHO_LAG_MIN_CONFIDENCE)
        .then(|| (estimate.delay_samples as i64 * 1000) / echo::SIMILARITY_SAMPLE_RATE as i64)
}

/// Level evidence: over `[start_ms, end_ms)` the system source is so much
/// louder than the microphone that the microphone plausibly only heard the
/// speaker. Compare both sources over the same span: the question is whether,
/// where they coincide, the microphone carries anything of its own.
fn rms_span_is_echo(
    mic_windows: &[f32],
    system_windows: &[f32],
    start_ms: i64,
    end_ms: i64,
) -> bool {
    let mic_energy = mean_rms(mic_windows, start_ms, end_ms);
    let system_energy = mean_rms(system_windows, start_ms, end_ms);
    system_energy >= mic_energy * ECHO_DOMINANCE_RATIO
}

/// Longest stretch scored per read while collecting similarity frames. A
/// continuous-bleed webinar can hold one mic turn open for hours; reading the
/// whole overlap at once would buffer the native-rate audio of both sources
/// in full (gigabytes for a long session), so score in bounded chunks.
const SIMILARITY_READ_CHUNK_MS: i64 = 60_000;

/// Content evidence for one overlap span: score lag-aligned 30 ms frames by
/// normalized cross-correlation, smooth the per-frame verdicts, and return the
/// bleed sub-spans. `None` (unreadable audio, nothing scorable) sends the
/// caller to the level fallback.
fn similarity_echo_spans(
    mic: &DetectedSource,
    system: &DetectedSource,
    start_ms: i64,
    end_ms: i64,
    evidence: &EchoEvidence,
) -> Option<Vec<(i64, i64)>> {
    let mut frames = Vec::new();
    let mut chunk_start_ms = start_ms;
    while chunk_start_ms < end_ms {
        let chunk_end_ms = end_ms.min(chunk_start_ms + SIMILARITY_READ_CHUNK_MS);
        let capture = echo::read_span_mono_16k(&mic.path, chunk_start_ms, chunk_end_ms)?;
        // The microphone hears the system audio `lag_ms` late, so the
        // reference for mic content at t is the system source at t - lag.
        let reference = echo::read_span_mono_16k(
            &system.path,
            chunk_start_ms - evidence.lag_ms,
            chunk_end_ms - evidence.lag_ms,
        )?;
        let chunk_offset_samples =
            ((chunk_start_ms - start_ms) * echo::SIMILARITY_SAMPLE_RATE as i64 / 1000) as usize;
        frames.extend(
            echo::windowed_ncc_frames(&capture, &reference)
                .into_iter()
                .map(|mut frame| {
                    frame.start_sample += chunk_offset_samples;
                    frame
                }),
        );
        chunk_start_ms = chunk_end_ms;
    }
    if frames.is_empty() {
        return None;
    }
    let verdicts = smooth_verdicts(&frame_verdicts(&frames, start_ms, evidence));
    Some(echo_runs_to_spans(&frames, &verdicts, start_ms))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameVerdict {
    Echo,
    Genuine,
}

fn frame_verdicts(
    frames: &[echo::SimilarityFrame],
    span_start_ms: i64,
    evidence: &EchoEvidence,
) -> Vec<FrameVerdict> {
    let mut verdicts = Vec::with_capacity(frames.len());
    let mut previous = FrameVerdict::Genuine;
    for frame in frames {
        let verdict = match frame.ncc {
            // A silent microphone frame has nothing to attribute either way;
            // extend the neighboring verdict so it never splits a span.
            None => previous,
            Some(similarity) if similarity >= ECHO_SIMILARITY_TRIM => FrameVerdict::Echo,
            Some(similarity) if similarity <= ECHO_SIMILARITY_KEEP => FrameVerdict::Genuine,
            // Ambiguous correlation: cancellation depth is stronger evidence
            // when available; otherwise let level evidence break the tie.
            Some(_) => erle_frame_verdict(frame, span_start_ms, evidence)
                .unwrap_or_else(|| level_frame_verdict(frame)),
        };
        verdicts.push(verdict);
        previous = verdict;
    }
    verdicts
}

fn erle_frame_verdict(
    frame: &echo::SimilarityFrame,
    span_start_ms: i64,
    evidence: &EchoEvidence,
) -> Option<FrameVerdict> {
    // First ambiguous frame of the recording pays for the cancellation
    // passes; recordings that never get here skip them entirely.
    let residual_windows = evidence.residual_windows()?;
    let frame_start_ms =
        span_start_ms + (frame.start_sample as i64 * 1000) / echo::SIMILARITY_SAMPLE_RATE as i64;
    let window_index = (frame_start_ms.max(0) / WINDOW_MS) as usize;
    let residual_rms = *residual_windows.get(window_index)?;
    if frame.capture_rms <= ERLE_RMS_FLOOR {
        return None;
    }
    if residual_rms <= ERLE_RMS_FLOOR {
        return Some(FrameVerdict::Echo);
    }
    let erle_db = 20.0 * (frame.capture_rms / residual_rms).log10();
    if erle_db >= ECHO_ERLE_TRIM_DB {
        Some(FrameVerdict::Echo)
    } else {
        Some(FrameVerdict::Genuine)
    }
}

fn level_frame_verdict(frame: &echo::SimilarityFrame) -> FrameVerdict {
    if frame.reference_rms >= frame.capture_rms * ECHO_DOMINANCE_RATIO {
        FrameVerdict::Echo
    } else {
        FrameVerdict::Genuine
    }
}

/// Majority vote over a centered five-frame (75 ms) window so single-frame
/// flicker does not fragment trim spans.
fn smooth_verdicts(verdicts: &[FrameVerdict]) -> Vec<FrameVerdict> {
    const HALF_WINDOW: usize = 2;
    verdicts
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let from = index.saturating_sub(HALF_WINDOW);
            let to = (index + HALF_WINDOW + 1).min(verdicts.len());
            let echo_votes = verdicts[from..to]
                .iter()
                .filter(|verdict| **verdict == FrameVerdict::Echo)
                .count();
            if echo_votes * 2 > to - from {
                FrameVerdict::Echo
            } else {
                FrameVerdict::Genuine
            }
        })
        .collect()
}

/// Convert consecutive echo-verdict frames back to wall-clock spans relative
/// to the scored span's start. Overlapping frames (the hop is half a frame)
/// merge into contiguous spans.
fn echo_runs_to_spans(
    frames: &[echo::SimilarityFrame],
    verdicts: &[FrameVerdict],
    span_start_ms: i64,
) -> Vec<(i64, i64)> {
    let mut spans: Vec<(i64, i64)> = Vec::new();
    for (frame, verdict) in frames.iter().zip(verdicts) {
        if *verdict != FrameVerdict::Echo {
            continue;
        }
        let frame_start = span_start_ms
            + (frame.start_sample as i64 * 1000) / echo::SIMILARITY_SAMPLE_RATE as i64;
        let frame_end = frame_start + echo::SIMILARITY_FRAME_MS;
        match spans.last_mut() {
            Some((_, last_end)) if frame_start <= *last_end => {
                *last_end = (*last_end).max(frame_end)
            }
            _ => spans.push((frame_start, frame_end)),
        }
    }
    spans
}

fn discarded_turn_spans(turn: &AudioTurn, kept: &[AudioTurn]) -> Vec<(i64, i64)> {
    let mut cursor = turn.start_ms;
    let mut removed = Vec::new();
    for remainder in kept {
        if remainder.start_ms > cursor {
            removed.push((cursor, remainder.start_ms));
        }
        cursor = cursor.max(remainder.end_ms);
    }
    if cursor < turn.end_ms {
        removed.push((cursor, turn.end_ms));
    }
    removed
}

/// Cut the echo spans out of `mic_turn` and return the remaining genuine
/// stretches that are still long enough to stand as their own turns. An empty
/// span list returns the turn unchanged; spans covering the whole turn return
/// nothing (pure echo, fully dropped).
fn trim_echo_from_microphone_turn(
    mic_turn: &AudioTurn,
    echo_spans: &[(i64, i64)],
    min_remainder_ms: i64,
    pre_roll_ms: i64,
) -> Vec<AudioTurn> {
    if echo_spans.is_empty() {
        return vec![mic_turn.clone()];
    }
    let mut spans = echo_spans.to_vec();
    spans.sort_by_key(|(start, _)| *start);
    let mut kept = Vec::new();
    let mut cursor = mic_turn.start_ms;
    let mut at_turn_head = true;
    for (start, end) in spans {
        if start > cursor {
            push_microphone_remainder(
                &mut kept,
                mic_turn,
                cursor,
                start,
                min_remainder_ms,
                if at_turn_head { pre_roll_ms } else { 0 },
            );
        }
        if end > cursor {
            cursor = end;
            at_turn_head = false;
        }
    }
    if cursor < mic_turn.end_ms {
        push_microphone_remainder(
            &mut kept,
            mic_turn,
            cursor,
            mic_turn.end_ms,
            min_remainder_ms,
            if at_turn_head { pre_roll_ms } else { 0 },
        );
    }
    kept
}

/// Push `[start_ms, end_ms)` of `mic_turn` as a microphone turn when it is at
/// least `min_remainder_ms` long. Remainders below the floor are smoothing
/// slivers at trim boundaries, not speech the detector stood behind.
/// `pre_roll_ms` must be zero for remainders that follow a trimmed span:
/// pre-roll exists to recover soft turn onsets out of silence, but a trim
/// boundary is preceded by the remote speaker's voice by construction, so
/// rolling back would put bleed at the head of every extracted remainder.
fn push_microphone_remainder(
    turns: &mut Vec<AudioTurn>,
    mic_turn: &AudioTurn,
    start_ms: i64,
    end_ms: i64,
    min_remainder_ms: i64,
    pre_roll_ms: i64,
) {
    if end_ms - start_ms < min_remainder_ms {
        return;
    }
    turns.push(AudioTurn {
        artifact_id: mic_turn.artifact_id.clone(),
        source: mic_turn.source.clone(),
        source_path: mic_turn.source_path.clone(),
        extraction_start_ms: (start_ms - pre_roll_ms).max(0),
        start_ms,
        end_ms,
        turn_index: 0,
    });
}

/// Mean RMS of the windows spanning `[start_ms, end_ms)`. Returns 0 when the
/// range is empty or falls outside the captured windows.
fn mean_rms(windows: &[f32], start_ms: i64, end_ms: i64) -> f32 {
    if windows.is_empty() || end_ms <= start_ms {
        return 0.0;
    }
    let start = (start_ms.max(0) / WINDOW_MS) as usize;
    let end = (windows_for_ms(end_ms.max(0)) as usize).min(windows.len());
    if end <= start || start >= windows.len() {
        return 0.0;
    }
    let slice = &windows[start..end];
    slice.iter().copied().sum::<f32>() / slice.len() as f32
}

fn windows_for_ms(duration_ms: i64) -> i64 {
    ((duration_ms + WINDOW_MS - 1) / WINDOW_MS).max(1)
}

fn config_for_source(source: &str) -> SourceDetectionConfig {
    if source == "system" {
        SourceDetectionConfig {
            start_active_ms: 180,
            pre_roll_ms: TURN_PRE_ROLL_MS,
            end_silence_ms: 2_000,
            min_turn_ms: 600,
            merge_gap_ms: 1_200,
            min_rms: SYSTEM_DETECTION_MIN_RMS,
            noise_multiplier: 3.0,
        }
    } else {
        SourceDetectionConfig {
            start_active_ms: 300,
            pre_roll_ms: TURN_PRE_ROLL_MS,
            end_silence_ms: 1_800,
            min_turn_ms: 700,
            merge_gap_ms: 900,
            min_rms: 0.012,
            noise_multiplier: 4.0,
        }
    }
}

fn source_order(source: &str) -> i32 {
    match source {
        "microphone" => 0,
        "system" => 1,
        _ => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::WavSpec;

    #[test]
    fn discarded_spans_include_short_remainders_but_never_kept_speech() {
        let turn = make_turn("microphone", 0, 3000);
        let kept = vec![make_turn("microphone", 1000, 2000)];
        assert_eq!(
            discarded_turn_spans(&turn, &kept),
            vec![(0, 1000), (2000, 3000)]
        );
        assert_eq!(discarded_turn_spans(&turn, &[]), vec![(0, 3000)]);
        assert!(discarded_turn_spans(&turn, std::slice::from_ref(&turn)).is_empty());
    }

    #[test]
    fn normalization_boosts_quiet_wav_without_touching_original() {
        let dir =
            std::env::temp_dir().join(format!("clovy-normalize-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("quiet.wav");
        let output = dir.join("normalized.wav");
        write_samples(&input, &[100, -120, 90, -80]);

        let prepared = normalize_wav_for_transcription(&input, &output).unwrap();

        assert_eq!(prepared, output);
        let original = read_samples(&input);
        let normalized = read_samples(&output);
        assert_eq!(original, vec![100, -120, 90, -80]);
        assert!(
            normalized.iter().map(|sample| sample.abs()).max().unwrap()
                > original.iter().map(|sample| sample.abs()).max().unwrap() * 10
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalization_reuses_loud_enough_wav() {
        let dir =
            std::env::temp_dir().join(format!("clovy-normalize-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("loud.wav");
        let output = dir.join("normalized.wav");
        write_samples(&input, &[20_000, -18_000]);

        let prepared = normalize_wav_for_transcription(&input, &output).unwrap();

        assert_eq!(prepared, input);
        assert!(!output.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn normalization_downmixes_and_downsamples_for_transcription() {
        let dir =
            std::env::temp_dir().join(format!("clovy-normalize-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("stereo.wav");
        let output = dir.join("prepared.wav");
        write_stereo_48k_samples(&input, &[8_000, 8_000, -8_000, -8_000, 4_000, 4_000]);

        let prepared = normalize_wav_for_transcription(&input, &output).unwrap();

        assert_eq!(prepared, output);
        let reader = WavReader::open(&output).unwrap();
        let spec = reader.spec();
        assert_eq!(spec.channels, 1);
        assert_eq!(spec.sample_rate, 16_000);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn streaming_normalization_is_byte_identical_to_buffered_reference() {
        struct Case {
            name: &'static str,
            spec: WavSpec,
            samples: Vec<i16>,
            chunk_sample_budget: usize,
        }

        let mono_16k = WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let stereo_16k = WavSpec {
            channels: 2,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let stereo_48k = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mono_8k = WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let stereo_44k = WavSpec {
            channels: 2,
            sample_rate: 44_100,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let cases = [
            Case {
                name: "mono-passthrough",
                spec: mono_16k,
                samples: vec![20_000, -18_000, 15_000],
                chunk_sample_budget: 3,
            },
            Case {
                name: "mono-gain-without-resampling",
                spec: mono_16k,
                samples: vec![100, -120, 90, -80, 60],
                chunk_sample_budget: 2,
            },
            Case {
                name: "empty-mono-passthrough",
                spec: mono_16k,
                samples: Vec::new(),
                chunk_sample_budget: 1,
            },
            Case {
                name: "empty-stereo-resample",
                spec: stereo_48k,
                samples: Vec::new(),
                chunk_sample_budget: 3,
            },
            Case {
                name: "stereo-without-resampling-across-odd-chunk-boundary",
                spec: stereo_16k,
                samples: vec![300, -100, 500, -300, 700, -500],
                chunk_sample_budget: 3,
            },
            Case {
                name: "downsample-across-odd-chunk-boundaries",
                spec: stereo_48k,
                samples: vec![
                    8_000, 4_000, -6_000, -2_000, 12_000, 8_000, -10_000, -4_000, 2_000, 1_000,
                    -3_000, -1_000, 5_000, -5_000,
                ],
                chunk_sample_budget: 5,
            },
            Case {
                name: "upsample-across-chunk-boundaries",
                spec: mono_8k,
                samples: vec![1_000, -2_000, 3_000, -4_000, 5_000],
                chunk_sample_budget: 3,
            },
            Case {
                name: "extreme-peaks",
                spec: stereo_44k,
                samples: vec![
                    i16::MIN,
                    i16::MIN,
                    i16::MAX,
                    i16::MAX,
                    i16::MIN,
                    i16::MAX,
                    1,
                    -1,
                ],
                chunk_sample_budget: 4,
            },
        ];

        let dir = std::env::temp_dir().join(format!(
            "clovy-normalize-equivalence-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for case in cases {
            let reference_input = dir.join(format!("{}-reference-input.wav", case.name));
            let streaming_input = dir.join(format!("{}-streaming-input.wav", case.name));
            let reference_output = dir.join(format!("{}-reference-output.wav", case.name));
            let streaming_output = dir.join(format!("{}-streaming-output.wav", case.name));
            write_wav_samples(&reference_input, case.spec, &case.samples);
            write_wav_samples(&streaming_input, case.spec, &case.samples);

            let reference_path = normalize_wav_for_transcription_buffered_reference(
                &reference_input,
                &reference_output,
            )
            .unwrap();
            let streaming_path = normalize_wav_for_transcription_with_chunk_sample_budget(
                &streaming_input,
                &streaming_output,
                case.chunk_sample_budget,
            )
            .unwrap();

            assert_eq!(
                std::fs::read(reference_path).unwrap(),
                std::fs::read(streaming_path).unwrap(),
                "normalization bytes differed for {}",
                case.name
            );
            assert_eq!(
                reference_output.exists(),
                streaming_output.exists(),
                "reuse decision differed for {}",
                case.name
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn turn_extraction_cuts_the_exact_segment() {
        // Pins the seek-based extraction: the segment must start at the
        // turn's first frame and contain exactly the turn's samples, byte
        // offsets and decoder state agreeing with the old skip-based path.
        let dir = std::env::temp_dir().join(format!("clovy-turn-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("source.wav");
        let output = dir.join("turn.wav");
        // 16 kHz mono: 1 ms = 16 frames. A ramp makes every frame unique.
        let samples = (0..480).map(|value| value as i16).collect::<Vec<_>>();
        write_samples(&input, &samples);

        let turn = AudioTurn {
            artifact_id: "artifact".to_string(),
            source: "microphone".to_string(),
            source_path: input.clone(),
            extraction_start_ms: 10,
            start_ms: 20,
            end_ms: 30,
            turn_index: 0,
        };
        write_turn_wav(&turn, &output).unwrap();

        let extracted = read_samples(&output);
        assert_eq!(extracted, (160..480).map(|v| v as i16).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn flags_silent_track_and_keeps_audible_one() {
        let dir = std::env::temp_dir().join(format!("clovy-silence-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let silent = dir.join("silent.wav");
        let audible = dir.join("audible.wav");
        write_samples(&silent, &[0, 0, 0, 0, 1, -1]);
        let audible_samples = [20_000, -18_000, 19_000, -20_000]
            .into_iter()
            .cycle()
            .take(16_000)
            .collect::<Vec<_>>();
        write_samples(&audible, &audible_samples);

        assert!(source_is_effectively_silent(&silent));
        assert!(!source_is_effectively_silent(&audible));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn silence_gate_rejects_an_isolated_device_start_transient() {
        let dir = std::env::temp_dir().join(format!(
            "clovy-transient-silence-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let transient = dir.join("transient.wav");
        let mut samples = vec![0_i16; 16_000 * 10];
        // Five 30 ms windows cross the floor, but the six-window speech
        // requirement rejects this short startup transient.
        samples[..2_400].fill(10_000);
        write_samples(&transient, &samples);

        assert!(source_is_effectively_silent(&transient));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn silence_gate_keeps_a_short_sustained_reply() {
        let dir =
            std::env::temp_dir().join(format!("clovy-short-reply-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let short_reply = dir.join("short-reply.wav");
        let mut samples = vec![0_i16; 16_000];
        // Six 30 ms windows is the minimum sustained activity accepted by the
        // gate, preserving short real replies.
        samples[..2_880].fill(10_000);
        write_samples(&short_reply, &samples);

        assert!(!source_is_effectively_silent(&short_reply));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn quiet_system_track_survives_detection_floor_but_is_silent_at_default_floor() {
        let dir = std::env::temp_dir().join(format!("clovy-floor-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let quiet = dir.join("quiet-system.wav");
        // Constant amplitude ~0.008 RMS: between the system detection floor
        // (0.006) and the normalized-chunk silence floor (0.012).
        let amplitude = (0.008 * i16::MAX as f32).round() as i16;
        let samples = vec![amplitude; 16_000];
        write_samples(&quiet, &samples);

        let max_rms = source_max_rms(&quiet).unwrap();
        assert!(
            max_rms > SYSTEM_DETECTION_MIN_RMS && max_rms < SILENCE_RMS_FLOOR,
            "expected RMS between floors, got {max_rms}"
        );
        assert!(!source_is_effectively_silent_with_floor(
            &quiet,
            SYSTEM_DETECTION_MIN_RMS
        ));
        assert!(source_is_effectively_silent(&quiet));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn truly_silent_track_is_dropped_at_detection_floor() {
        let dir = std::env::temp_dir().join(format!("clovy-floor-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let silent = dir.join("silent-system.wav");
        write_samples(&silent, &vec![0_i16; 16_000]);

        assert!(source_is_effectively_silent_with_floor(
            &silent,
            SYSTEM_DETECTION_MIN_RMS
        ));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pre_roll_does_not_change_merge_gap_decisions() {
        let source_path = PathBuf::from("source.wav");
        let turns = vec![
            AudioTurn {
                artifact_id: "artifact".to_string(),
                source: "microphone".to_string(),
                source_path: source_path.clone(),
                extraction_start_ms: 0,
                start_ms: 0,
                end_ms: 1_000,
                turn_index: 0,
            },
            AudioTurn {
                artifact_id: "artifact".to_string(),
                source: "microphone".to_string(),
                source_path,
                extraction_start_ms: 1_950,
                start_ms: 1_950,
                end_ms: 3_000,
                turn_index: 0,
            },
        ];

        let merged = merge_close_turns(turns, 900);
        let pre_rolled = apply_extraction_pre_roll(merged, TURN_PRE_ROLL_MS);

        assert_eq!(pre_rolled.len(), 2);
        assert_eq!(pre_rolled[1].start_ms, 1_950);
        assert_eq!(pre_rolled[1].extraction_start_ms, 1_800);
    }

    #[test]
    fn source_configs_keep_pre_roll_below_merge_gap() {
        for config in [config_for_source("microphone"), config_for_source("system")] {
            assert!(config.pre_roll_ms < config.merge_gap_ms);
        }
    }

    #[test]
    fn microphone_echo_of_system_audio_is_not_attributed_to_microphone() {
        // Hands-free meeting: a remote participant speaks, captured loud and
        // clean on the system source and bled quietly into the microphone.
        // Without echo rejection the mic detector emits a "microphone" turn for
        // that bleed, misattributing system audio to the user. A later,
        // system-free mic turn (the user actually speaking) must still survive.
        let dir = std::env::temp_dir().join(format!("clovy-echo-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let lag_samples = 1_280_usize; // 80 ms echo path
        let speech = centered_splitmix_noise(7, 3 * rate / 2);
        let own_speech = centered_splitmix_noise(99, 3 * rate / 2);

        // System: 1.5s loud remote speech, then silence.
        let mut system = vec![0.0_f32; 36 * rate / 10];
        system[..3 * rate / 2].copy_from_slice(&speech);
        write_samples(&system_path, &to_i16(&system));

        // Mic: the quiet delayed bleed of that speech, 2.1s of silence, then
        // 1.5s of the user genuinely speaking.
        let mut mic = vec![0.0_f32; 51 * rate / 10];
        for (index, sample) in speech.iter().enumerate() {
            mic[index + lag_samples] += 0.1 * sample;
        }
        for (offset, sample) in own_speech.iter().enumerate() {
            mic[(36 * rate / 10) + offset] += 0.5 * sample;
        }
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        // The remote participant's speech is retained on the system source...
        assert!(turns
            .iter()
            .any(|turn| turn.source == "system" && turn.start_ms < 1_000));
        // ...the genuine, system-free microphone speech survives...
        assert!(turns
            .iter()
            .any(|turn| turn.source == "microphone" && turn.start_ms > 2_500));
        // ...and no microphone turn is attributed to the echoed system speech.
        assert!(turns
            .iter()
            .filter(|turn| turn.source == "microphone")
            .all(|turn| turn.start_ms > 2_500));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn similarity_gate_trims_correlated_bleed_and_keeps_quiet_double_talk() {
        // A hands-free meeting where level evidence alone gets BOTH calls
        // wrong: the remote participant's bleed lands only ~2x quieter than
        // the system source (level says genuine), while the user's own quiet
        // reply is >3x quieter (level says bleed). Content similarity decides
        // correctly: bleed correlates with the lag-aligned system reference no
        // matter its level, the user's voice does not.
        let dir =
            std::env::temp_dir().join(format!("clovy-similarity-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let lag_samples = 1_280_usize; // 80 ms echo path
        let speech = echo::test_signals::splitmix_noise(7, 4 * rate);
        let reply = echo::test_signals::splitmix_noise(99, 4 * rate);

        // System: 4s of remote speech, then 2s of silence so both detectors
        // can close their turns.
        let mut system = vec![0.0_f32; 6 * rate];
        system[..4 * rate].copy_from_slice(&speech);

        // Microphone: what the room heard, delayed by the echo path.
        // - a faint bleed bed (5%) while the speaker plays
        // - loud bleed [0.5s, 1.7s]: remote speech at ~55% of the system level
        // - the user's quiet reply [2.5s, 3.9s]: independent content at ~30%
        let mut mic = vec![0.0_f32; 6 * rate];
        for (index, sample) in speech.iter().enumerate() {
            mic[index + lag_samples] += 0.05 * sample;
        }
        for index in (rate / 2)..(17 * rate / 10) {
            mic[index + lag_samples] += 0.5 * speech[index];
        }
        for (offset, sample) in reply[..(39 * rate / 10) - (5 * rate / 2)]
            .iter()
            .enumerate()
        {
            mic[(5 * rate / 2) + offset] += 0.3 * sample;
        }

        write_samples(&system_path, &to_i16(&system));
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        // The user's quiet reply survives as a microphone turn...
        assert!(
            turns.iter().any(|turn| turn.source == "microphone"
                && turn.start_ms >= 2_300
                && turn.start_ms <= 2_700
                && turn.end_ms >= 3_700),
            "expected the quiet double-talk reply to survive, got {turns:?}"
        );
        // ...every bleed span is trimmed, including the one only ~2x quieter
        // than the system source...
        assert!(
            turns
                .iter()
                .filter(|turn| turn.source == "microphone")
                .all(|turn| turn.start_ms >= 2_300),
            "expected all bleed to be trimmed from the microphone, got {turns:?}"
        );
        // ...and the remote speech stays attributed to the system source.
        assert!(turns
            .iter()
            .any(|turn| turn.source == "system" && turn.start_ms < 500));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn erle_tier_trims_reverberant_bleed_and_keeps_quiet_reply() {
        // Reverberant speaker bleed spreads across several delays. A single
        // lag-aligned NCC frame is therefore only ambiguous, and the total
        // level is not 3x below the system source, so pre-ERLE level fallback
        // would keep the bleed as a microphone turn. The bleed runs whenever
        // the system plays (a room's echo path is stationary; an on/off bleed
        // would force the canceller to track a time-varying path, which is
        // not the scenario this tier targets), and the user's reply lands
        // after the remote speech ends.
        let dir = std::env::temp_dir().join(format!("clovy-erle-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let speech = centered_splitmix_noise(7, 4 * rate);
        let reply = centered_splitmix_noise(99, 4 * rate);
        let taps = [
            (80 * rate / 1_000, 0.28_f32),
            (110 * rate / 1_000, 0.27_f32),
            (140 * rate / 1_000, 0.27_f32),
            (170 * rate / 1_000, 0.27_f32),
            (200 * rate / 1_000, 0.27_f32),
        ];

        // System: 4s of remote speech, then 4s of silence so turns close.
        let mut system = vec![0.0_f32; 8 * rate];
        system[..4 * rate].copy_from_slice(&speech);

        // Mic: the room's stationary echo of everything the system played
        // (bleed spans ~[0.08s, 4.2s]), then the user's quiet reply
        // [4.5s, 5.9s] while the system is silent.
        let mut mic = vec![0.0_f32; 8 * rate];
        for (index, sample) in speech.iter().enumerate() {
            for (delay, gain) in taps {
                mic[index + delay] += gain * sample;
            }
        }
        for (offset, sample) in reply[..(59 * rate / 10) - (45 * rate / 10)]
            .iter()
            .enumerate()
        {
            mic[(45 * rate / 10) + offset] += 0.3 * sample;
        }

        // Fixture self-check: at first-tap alignment the steady-state bleed
        // must land in the ambiguous NCC band, or this test would exercise
        // the similarity tier instead of the ERLE tier.
        let aligned_bleed_start = rate + taps[0].0;
        let aligned_bleed_end = 3 * rate + taps[0].0;
        let bleed_frames = echo::windowed_ncc_frames(
            &mic[aligned_bleed_start..aligned_bleed_end],
            &system[rate..3 * rate],
        );
        let mut bleed_nccs: Vec<f32> = bleed_frames.iter().filter_map(|frame| frame.ncc).collect();
        bleed_nccs.sort_by(f32::total_cmp);
        let median_bleed_ncc = bleed_nccs[bleed_nccs.len() / 2];
        assert!(
            median_bleed_ncc > ECHO_SIMILARITY_KEEP && median_bleed_ncc < ECHO_SIMILARITY_TRIM,
            "fixture must exercise the ambiguous NCC band, got median NCC {median_bleed_ncc:.3}"
        );

        write_samples(&system_path, &to_i16(&system));
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        // The evidence window ends at the system turn's end plus the echo
        // lag, so up to (last tap - lag) ~ 120 ms of decaying reverb tail can
        // remain at the kept remainder's head - a documented residue, not
        // misattributed speech.
        assert!(
            turns
                .iter()
                .filter(|turn| turn.source == "microphone")
                .all(|turn| turn.start_ms >= 4_050),
            "expected reverberant bleed to be trimmed from the microphone, got {turns:?}"
        );
        assert!(
            turns.iter().any(|turn| turn.source == "microphone"
                && turn.start_ms >= 4_050
                && turn.start_ms <= 4_600
                && turn.end_ms >= 5_800),
            "expected the quiet reply to survive as microphone speech, got {turns:?}"
        );
        assert!(
            turns
                .iter()
                .any(|turn| turn.source == "system" && turn.start_ms < 500),
            "expected the system turn to survive, got {turns:?}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bleed_tail_past_system_turn_end_is_still_trimmed() {
        // Bleed of system content at time t reaches the microphone at t + lag,
        // so the last `lag` of bleed falls after the system turn's end. With a
        // large echo lag that tail is longer than the remainder floor and must
        // still be scored and trimmed, not kept because it left the unshifted
        // overlap window.
        let dir = std::env::temp_dir().join(format!("clovy-tail-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let lag_samples = 2 * rate / 5; // 400 ms echo path
        let speech = centered_splitmix_noise(7, 2 * rate);

        // System: 2s of remote speech, then 3s of silence so both detectors
        // close their turns instead of running to the end of the file.
        let mut system = vec![0.0_f32; 5 * rate];
        system[..2 * rate].copy_from_slice(&speech);

        // Mic: only the delayed bleed, quiet enough that level evidence alone
        // would trim the overlap but leave the 400 ms tail untouched.
        let mut mic = vec![0.0_f32; 5 * rate];
        for (index, sample) in speech.iter().enumerate() {
            mic[index + lag_samples] += 0.3 * sample;
        }
        write_samples(&system_path, &to_i16(&system));
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        assert!(
            turns.iter().all(|turn| turn.source != "microphone"),
            "expected the bleed tail to be trimmed with the rest, got {turns:?}"
        );
        assert!(turns.iter().any(|turn| turn.source == "system"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn headphones_session_never_trims_quiet_genuine_speech() {
        // On headphones there is no echo path: nothing the system plays
        // reaches the microphone, so no lag probe can succeed. A quiet user
        // interjecting while the remote participant talks sits more than 3x
        // below the near-full-scale system source, so level dominance alone
        // would delete their genuine words. Without a corroborated echo path
        // nothing may be trimmed.
        let dir =
            std::env::temp_dir().join(format!("clovy-headphones-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let speech = centered_splitmix_noise(7, 2 * rate);
        let interjection = centered_splitmix_noise(99, rate);

        // System: 2s of loud remote speech, then silence.
        let mut system = vec![0.0_f32; 5 * rate];
        system[..2 * rate].copy_from_slice(&speech);

        // Mic: only the user's quiet interjection [0.5s, 1.5s], >3x below the
        // system level, with zero bleed anywhere.
        let mut mic = vec![0.0_f32; 5 * rate];
        for (offset, sample) in interjection.iter().enumerate() {
            mic[rate / 2 + offset] += 0.15 * sample;
        }
        write_samples(&system_path, &to_i16(&system));
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        assert!(
            turns
                .iter()
                .any(|turn| turn.source == "microphone" && turn.start_ms < 1_000),
            "expected the user's quiet interjection to survive, got {turns:?}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn echo_path_appearing_on_a_late_system_turn_is_still_trimmed() {
        // The session starts on headphones: the first three system turns
        // produce no bleed, so a probe cap on early turns would never latch a
        // lag. The user then unplugs, and the fourth system turn bleeds into
        // the microphone. The bleed must still be trimmed — otherwise the
        // original misattribution returns for the rest of the recording —
        // while the genuine quiet interjection from the headphones phase
        // survives.
        let dir =
            std::env::temp_dir().join(format!("clovy-late-echo-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let interjection = centered_splitmix_noise(99, 9 * rate / 10);
        let late_speech = centered_splitmix_noise(31, 3 * rate);

        // System: three short headphones-phase turns, then the speakers-phase
        // turn at [12s, 15s]; gaps exceed the detector's end-silence window.
        let mut system = vec![0.0_f32; 18 * rate];
        for (turn_index, seed) in [(0_usize, 7_u64), (1, 11), (2, 13)] {
            let speech = centered_splitmix_noise(seed, 3 * rate / 2);
            let start = turn_index * 4 * rate;
            system[start..start + speech.len()].copy_from_slice(&speech);
        }
        system[12 * rate..15 * rate].copy_from_slice(&late_speech);

        // Mic: a quiet genuine interjection during the first turn (no bleed
        // exists on headphones), then quiet bleed of the late turn at a
        // 120 ms path; both are >3x below the system level.
        let mut mic = vec![0.0_f32; 18 * rate];
        for (offset, sample) in interjection.iter().enumerate() {
            mic[rate / 2 + offset] += 0.15 * sample;
        }
        for (index, sample) in late_speech.iter().enumerate() {
            mic[12 * rate + index + (12 * rate / 100)] += 0.25 * sample;
        }
        write_samples(&system_path, &to_i16(&system));
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        assert!(
            turns
                .iter()
                .any(|turn| turn.source == "microphone" && turn.start_ms < 1_000),
            "expected the headphones-phase interjection to survive, got {turns:?}"
        );
        assert!(
            turns
                .iter()
                .filter(|turn| turn.source == "microphone")
                .all(|turn| turn.end_ms <= 2_000),
            "expected the late-turn bleed to be trimmed, got {turns:?}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn echo_path_appearing_inside_one_long_system_turn_is_still_trimmed() {
        // Continuous system playback, and the output device switches to
        // speakers partway through: the single system turn's head carries no
        // bleed, so no session lag can be latched from turn heads at all. The
        // dominated overlap must earn its own probe (scanning past the head)
        // and be trimmed.
        let dir =
            std::env::temp_dir().join(format!("clovy-mid-turn-echo-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let speech = centered_splitmix_noise(7, 10 * rate);

        // System: one 10s turn of continuous speech.
        let mut system = vec![0.0_f32; 13 * rate];
        system[..10 * rate].copy_from_slice(&speech);

        // Mic: bleed of the second half only (the switch happens at 5s), at
        // a 120 ms path, >3x below the system level.
        let mut mic = vec![0.0_f32; 13 * rate];
        for (index, sample) in speech[5 * rate..].iter().enumerate() {
            mic[5 * rate + index + (12 * rate / 100)] += 0.25 * sample;
        }
        write_samples(&system_path, &to_i16(&system));
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        assert!(
            turns.iter().all(|turn| turn.source != "microphone"),
            "expected the mid-turn bleed to be trimmed, got {turns:?}"
        );
        assert!(turns.iter().any(|turn| turn.source == "system"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_session_lag_is_reprobed_per_overlap() {
        // The output device switches mid-session: the echo path jumps from
        // 80 ms to 350 ms. Whichever lag the session probe locks onto, the
        // other segment's bleed decorrelates at that alignment and content
        // similarity would confidently call it genuine — bringing back the
        // original misattribution. The all-genuine-but-dominated pattern must
        // trigger a per-overlap re-probe that recovers the right lag.
        let dir =
            std::env::temp_dir().join(format!("clovy-stale-lag-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");

        let rate = 16_000_usize;
        let speech_one = centered_splitmix_noise(7, 3 * rate);
        let speech_two = centered_splitmix_noise(31, 3 * rate);

        // System: 3s of speech, 2.5s of silence (closes both detectors'
        // turns), then 3s of speech after the device switch, then silence.
        let mut system = vec![0.0_f32; 11 * rate];
        system[..3 * rate].copy_from_slice(&speech_one);
        system[(55 * rate / 10)..(85 * rate / 10)].copy_from_slice(&speech_two);

        // Mic: quiet bleed of segment one at an 80 ms path, of segment two at
        // a 350 ms path; >3x below the system level throughout, no genuine
        // speech at all.
        let mut mic = vec![0.0_f32; 11 * rate];
        for (index, sample) in speech_one.iter().enumerate() {
            mic[index + (8 * rate / 100)] += 0.25 * sample;
        }
        for (index, sample) in speech_two.iter().enumerate() {
            mic[(55 * rate / 10) + index + (35 * rate / 100)] += 0.25 * sample;
        }
        write_samples(&system_path, &to_i16(&system));
        write_samples(&mic_path, &to_i16(&mic));

        let turns = detect_turns(&[
            DetectionSource {
                artifact_id: "mic".to_string(),
                source: "microphone".to_string(),
                path: mic_path,
            },
            DetectionSource {
                artifact_id: "sys".to_string(),
                source: "system".to_string(),
                path: system_path,
            },
        ])
        .unwrap();

        assert!(
            turns.iter().all(|turn| turn.source != "microphone"),
            "expected bleed from both echo paths to be trimmed, got {turns:?}"
        );
        assert!(turns.iter().filter(|turn| turn.source == "system").count() >= 2);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn echo_decision_trims_quiet_overlap_but_keeps_loud_or_isolated_mic() {
        // 30ms windows: window index == ms / 30. 200 windows == 6s.
        let mut mic_windows = vec![0.0_f32; 200];
        let mut system_windows = vec![0.0_f32; 200];
        for window in 0..50 {
            mic_windows[window] = 0.05; // quiet echo
            system_windows[window] = 0.5; // loud system speech
        }
        mic_windows[120..170].fill(0.5); // genuine mic speech, no system
        let system_turns = vec![make_turn("system", 0, 1_500)];
        let config = config_for_source("microphone");
        let min_remainder_ms = ECHO_TRIM_REMAINDER_MIN_MS;
        let pre_roll_ms = config.pre_roll_ms;

        // A trusted lag exists but the audio is unscorable (empty paths), so
        // evidence falls to level dominance over each overlap.
        let evidence = make_unscorable_evidence(80);
        let system = make_detected("system", system_windows.clone(), system_turns.clone());

        // Quiet mic fully covered by louder system audio is all echo, trimmed away.
        let echo = make_turn("microphone", 0, 1_500);
        let mic = make_detected("microphone", mic_windows.clone(), vec![echo.clone()]);
        let echo_spans = echo_evidence_spans(&echo, &mic, &system, Some(&evidence));
        assert_eq!(echo_spans, vec![(0, 1_500)]);
        assert!(
            trim_echo_from_microphone_turn(&echo, &echo_spans, min_remainder_ms, pre_roll_ms)
                .is_empty()
        );

        // A mic turn with no overlapping system audio is genuine speech, kept whole.
        let genuine = make_turn("microphone", 3_600, 5_100);
        let genuine_spans = echo_evidence_spans(&genuine, &mic, &system, Some(&evidence));
        assert!(genuine_spans.is_empty());
        assert_eq!(
            trim_echo_from_microphone_turn(&genuine, &genuine_spans, min_remainder_ms, pre_roll_ms),
            vec![genuine]
        );

        // Simultaneous speech with comparable mic energy is not echo, kept whole.
        let mut loud_mic_windows = vec![0.0_f32; 200];
        loud_mic_windows[0..50].fill(0.45);
        let loud_mic = make_detected("microphone", loud_mic_windows, vec![echo.clone()]);
        assert!(echo_evidence_spans(&echo, &loud_mic, &system, Some(&evidence)).is_empty());

        // Without a trusted lag no echo path is corroborated: nothing is
        // trimmed even when the system source dominates (headphones case).
        assert!(echo_evidence_spans(&echo, &mic, &system, None).is_empty());
    }

    #[test]
    fn echo_trim_preserves_user_reply_merged_into_one_microphone_turn() {
        // A remote participant speaks, then the user replies shortly after. The
        // echo of the remote speech and the user's reply land in ONE mic turn
        // (the gap is shorter than the detector's silence/merge windows). The
        // echo must be trimmed off while the user's reply survives, instead of
        // dropping the whole turn.
        let config = config_for_source("microphone");
        let min_remainder_ms = ECHO_TRIM_REMAINDER_MIN_MS;
        let pre_roll_ms = config.pre_roll_ms;
        let mic_turn = make_turn("microphone", 0, 3_300);
        let system_turns = vec![make_turn("system", 0, 2_000)];

        // 30ms windows: 0..2000ms quiet echo, 2000..3300ms loud genuine reply.
        let mut mic_windows = vec![0.0_f32; 110];
        let mut system_windows = vec![0.0_f32; 110];
        for window in 0..67 {
            mic_windows[window] = 0.05; // echo of the remote participant
            system_windows[window] = 0.5; // remote participant, loud and clean
        }
        mic_windows[67..110].fill(0.5); // the user's own reply, no system audio

        let mic = make_detected("microphone", mic_windows, vec![mic_turn.clone()]);
        let system = make_detected("system", system_windows, system_turns);
        let evidence = make_unscorable_evidence(80);
        let echo_spans = echo_evidence_spans(&mic_turn, &mic, &system, Some(&evidence));
        let kept =
            trim_echo_from_microphone_turn(&mic_turn, &echo_spans, min_remainder_ms, pre_roll_ms);

        // The echoed front is removed; the user's reply remains a mic turn.
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].source, "microphone");
        assert!(kept[0].start_ms >= 2_000);
        assert_eq!(kept[0].end_ms, 3_300);
    }

    #[test]
    fn echo_trim_keeps_short_genuine_reply_after_bleed() {
        // A one-word reply ("yes", "next") right after trimmed bleed is well
        // under the detector's 700 ms minimum turn length, but it sits inside
        // an already-detected turn and the evidence judged it genuine. The
        // remainder floor must keep it while still dropping smoothing slivers.
        let mic_turn = make_turn("microphone", 0, 2_400);

        // Bleed covers [0, 2_000); the 400 ms remainder is the reply.
        let kept = trim_echo_from_microphone_turn(
            &mic_turn,
            &[(0, 2_000)],
            ECHO_TRIM_REMAINDER_MIN_MS,
            TURN_PRE_ROLL_MS,
        );
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].start_ms, kept[0].end_ms), (2_000, 2_400));

        // A 90 ms remainder is a smoothing sliver, not a reply: still dropped.
        let sliver_turn = make_turn("microphone", 0, 2_090);
        assert!(trim_echo_from_microphone_turn(
            &sliver_turn,
            &[(0, 2_000)],
            ECHO_TRIM_REMAINDER_MIN_MS,
            TURN_PRE_ROLL_MS,
        )
        .is_empty());
    }

    fn to_i16(samples: &[f32]) -> Vec<i16> {
        samples
            .iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
            .collect()
    }

    fn centered_splitmix_noise(seed: u64, len: usize) -> Vec<f32> {
        let mut samples = echo::test_signals::splitmix_noise(seed, len);
        let mean = samples.iter().copied().sum::<f32>() / samples.len().max(1) as f32;
        for sample in &mut samples {
            *sample -= mean;
        }
        samples
    }

    fn make_detected(source: &str, windows: Vec<f32>, turns: Vec<AudioTurn>) -> DetectedSource {
        DetectedSource {
            source: source.to_string(),
            path: PathBuf::new(),
            windows,
            turns,
        }
    }

    /// Evidence with a trusted lag but unreadable audio (empty paths), so
    /// content similarity is unscorable and callers exercise the level
    /// fallback with a corroborated echo path.
    fn make_unscorable_evidence(lag_ms: i64) -> EchoEvidence {
        EchoEvidence {
            lag_ms,
            mic_path: PathBuf::new(),
            system_path: PathBuf::new(),
            residual_windows: std::cell::OnceCell::new(),
        }
    }

    fn make_turn(source: &str, start_ms: i64, end_ms: i64) -> AudioTurn {
        AudioTurn {
            artifact_id: "artifact".to_string(),
            source: source.to_string(),
            source_path: PathBuf::new(),
            extraction_start_ms: start_ms.max(0),
            start_ms,
            end_ms,
            turn_index: 0,
        }
    }
    fn write_samples(path: &Path, samples: &[i16]) {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        write_wav_samples(path, spec, samples);
    }

    fn write_stereo_48k_samples(path: &Path, samples: &[i16]) {
        let spec = WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        write_wav_samples(path, spec, samples);
    }

    /// Groq's upload cap on the free tier.
    const GROQ_MAX_UPLOAD_BYTES: i64 = 26_214_400;

    #[test]
    fn a_final_transcription_span_stays_under_groqs_upload_cap() {
        assert_eq!(FINAL_TRANSCRIPTION_SPAN_MS, 600_000);
        assert_eq!(SPLIT_OVERLAP_MS, 10_000);
        // A normalized span is PCM16 mono at 16 kHz plus a 44-byte header.
        let span_bytes =
            FINAL_TRANSCRIPTION_SPAN_MS * i64::from(TRANSCRIPTION_SAMPLE_RATE) / 1000 * 2 + 44;
        assert_eq!(span_bytes, 19_200_044);
        assert!(span_bytes < GROQ_MAX_UPLOAD_BYTES);
    }

    fn normalized_spec() -> WavSpec {
        WavSpec {
            channels: TRANSCRIPTION_CHANNELS,
            sample_rate: TRANSCRIPTION_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        }
    }

    fn chunk_durations_ms(chunks: &[PathBuf]) -> Vec<i64> {
        chunks
            .iter()
            .map(|chunk| {
                let reader = WavReader::open(chunk).unwrap();
                i64::from(reader.duration()) * 1000 / i64::from(TRANSCRIPTION_SAMPLE_RATE)
            })
            .collect()
    }

    #[test]
    fn an_overlapped_split_repeats_the_overlap_at_the_start_of_the_next_chunk() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.wav");
        // 130 s whose samples count the second, so each chunk's first sample
        // says where in the source it starts.
        let samples = (0..16_000 * 130)
            .map(|index| (index / 16_000) as i16)
            .collect::<Vec<_>>();
        write_wav_samples(&input, normalized_spec(), &samples);

        let chunks = split_wav_for_transcription_with_overlap(
            &input,
            &dir.path().join("chunks"),
            "source",
            60_000,
            10_000,
        )
        .unwrap();

        // Starts at 0, 50 and 100 s; the last one ends with the source.
        assert_eq!(chunk_durations_ms(&chunks), vec![60_000, 60_000, 30_000]);
        let first_seconds = chunks
            .iter()
            .map(|chunk| read_samples(chunk)[0])
            .collect::<Vec<_>>();
        assert_eq!(first_seconds, vec![0, 50, 100]);
    }

    #[test]
    fn a_split_without_overlap_keeps_back_to_back_chunks_under_the_upload_cap() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("source.wav");
        write_wav_samples(&input, normalized_spec(), &vec![1_000_i16; 16_000 * 70]);

        let chunks = split_wav_for_transcription_with_max_duration(
            &input,
            &dir.path().join("chunks"),
            "source",
            30_000,
        )
        .unwrap();

        assert_eq!(chunk_durations_ms(&chunks), vec![30_000, 30_000, 10_000]);
        for chunk in &chunks {
            let bytes = std::fs::metadata(chunk).unwrap().len() as i64;
            assert!(bytes < GROQ_MAX_UPLOAD_BYTES, "{bytes}");
        }
    }

    #[test]
    fn same_source_turns_group_up_to_the_final_span_without_crossing_the_other_source() {
        let turn = |source: &str, start_ms: i64, end_ms: i64| AudioTurn {
            artifact_id: format!("artifact-{source}"),
            source: source.to_string(),
            source_path: PathBuf::from(source),
            extraction_start_ms: start_ms,
            start_ms,
            end_ms,
            turn_index: 0,
        };
        let mut turns = (0..12)
            .map(|index| turn("microphone", index * 10_000, index * 10_000 + 9_000))
            .collect::<Vec<_>>();
        turns.push(turn("system", 125_000, 130_000));
        turns.push(turn("microphone", 131_000, 140_000));

        let grouped = coalesce_turns_for_transcription(turns);

        assert_eq!(
            grouped
                .iter()
                .map(|turn| (turn.source.as_str(), turn.start_ms, turn.end_ms))
                .collect::<Vec<_>>(),
            vec![
                ("microphone", 0, 119_000),
                ("system", 125_000, 130_000),
                ("microphone", 131_000, 140_000),
            ]
        );
    }

    fn write_wav_samples(path: &Path, spec: WavSpec, samples: &[i16]) {
        let mut writer = WavWriter::create(path, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn read_samples(path: &Path) -> Vec<i16> {
        let mut reader = WavReader::open(path).unwrap();
        reader
            .samples::<i16>()
            .map(|sample| sample.unwrap())
            .collect()
    }

    fn normalize_wav_for_transcription_buffered_reference(
        input_path: &Path,
        output_path: &Path,
    ) -> Result<PathBuf, AppError> {
        let mut reader = WavReader::open(input_path)
            .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
        let spec = reader.spec();
        ensure_normalizable_spec(spec)?;
        let input_samples = reader
            .samples::<i16>()
            .map(|sample| sample.unwrap_or(0))
            .collect::<Vec<_>>();
        let mono_samples = downmix_to_mono(&input_samples, spec.channels.max(1));
        let peak = mono_samples
            .iter()
            .map(|sample| sample.unsigned_abs() as f32 / i16::MAX as f32)
            .fold(0.0_f32, f32::max);
        let output_spec = WavSpec {
            channels: TRANSCRIPTION_CHANNELS,
            sample_rate: TRANSCRIPTION_SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let already_transcription_ready = spec.channels == TRANSCRIPTION_CHANNELS
            && spec.sample_rate == TRANSCRIPTION_SAMPLE_RATE;
        if peak <= f32::EPSILON && already_transcription_ready {
            return Ok(input_path.to_path_buf());
        }
        let gain = if peak <= f32::EPSILON {
            1.0
        } else {
            (NORMALIZE_TARGET_PEAK / peak).min(NORMALIZE_MAX_GAIN)
        };
        if gain < NORMALIZE_MIN_GAIN && already_transcription_ready {
            return Ok(input_path.to_path_buf());
        }
        if let Some(parent) = output_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
        }
        let prepared_samples =
            resample_linear(&mono_samples, spec.sample_rate, TRANSCRIPTION_SAMPLE_RATE);
        let mut writer = WavWriter::create(output_path, output_spec)
            .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
        for sample in prepared_samples {
            let amplified = (sample as f32 * gain).round();
            writer
                .write_sample(amplified.clamp(i16::MIN as f32, i16::MAX as f32) as i16)
                .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
        }
        writer
            .finalize()
            .map_err(|error| AppError::new("audio_normalize_failed", error.to_string()))?;
        Ok(output_path.to_path_buf())
    }
}
