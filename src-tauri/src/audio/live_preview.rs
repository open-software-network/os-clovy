use crate::{
    audio::turns::normalize_wav_for_transcription,
    domain::types::{RecordingSource, RecordingSourceMode},
    providers::groq::TranscriptionRequest,
};
use hound::{SampleFormat, WavSpec, WavWriter};
use serde::Serialize;
use std::{
    fs::File,
    io::{ErrorKind, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc;
use uuid::Uuid;

pub const LIVE_TRANSCRIPT_EVENT: &str = "live-transcript-event";

const PREVIEW_BATCH_BUFFER: usize = 512;
const PREVIEW_STALE_BATCH_THRESHOLD: usize = PREVIEW_BATCH_BUFFER / 2;
pub const PREVIEW_CHUNK_MS: i64 = 8_000;
const PREVIEW_SILENCE_RMS_FLOOR: f32 = 0.001;
const PREVIEW_ACTIVITY_WINDOW_MS: i64 = 30;
const PREVIEW_MIN_SUSTAINED_ACTIVITY_MS: i64 = 180;
const SYSTEM_PREVIEW_POLL_MS: u64 = 500;
// The system lane tails a growing WAV rather than draining a bounded channel,
// so it has no natural backpressure. If the transcription round-trip runs
// slower than real time the buffered backlog would grow without bound and the
// preview would fall further and further behind the live meeting. Cap the
// backlog so the system preview skips stale audio and stays near real time,
// mirroring the microphone lane's stale-batch dropping and ADR 0002's guardrail
// that preview audio may be dropped to keep up. Two chunks keeps a little
// context without letting the lag accumulate.
const SYSTEM_PREVIEW_MAX_BACKLOG_CHUNKS: usize = 2;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveTranscriptEventDto {
    pub note_id: String,
    pub session_id: String,
    pub source_mode: RecordingSourceMode,
    pub source: RecordingSource,
    pub segment_id: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
    pub language: Option<String>,
    pub stability: &'static str,
}

#[derive(Debug)]
struct LivePreviewBatch {
    start_sample: u64,
    samples: Vec<i16>,
}

#[derive(Clone)]
pub struct LivePreviewSink {
    sender: mpsc::Sender<LivePreviewBatch>,
    next_sample_index: Arc<AtomicU64>,
}

impl LivePreviewSink {
    pub fn try_send(&self, samples: Vec<i16>) -> bool {
        if samples.is_empty() {
            return false;
        }
        let start_sample = self
            .next_sample_index
            .fetch_add(samples.len() as u64, Ordering::Relaxed);
        self.sender
            .try_send(LivePreviewBatch {
                start_sample,
                samples,
            })
            .is_ok()
    }
}

pub struct LivePreviewController {
    cancelled: Arc<AtomicBool>,
    sink: LivePreviewSink,
}

pub struct SystemLivePreviewController {
    cancelled: Arc<AtomicBool>,
}

struct LivePreviewWorkerParams {
    app: AppHandle,
    note_id: String,
    session_id: String,
    source_mode: RecordingSourceMode,
    source: RecordingSource,
    sample_rate: u32,
    channels: u16,
    receiver: mpsc::Receiver<LivePreviewBatch>,
    cancelled: Arc<AtomicBool>,
}

struct PreviewChunkRequest<'a> {
    note_id: &'a str,
    session_id: &'a str,
    source_mode: RecordingSourceMode,
    source: RecordingSource,
    segment_id: &'a str,
    start_ms: i64,
    end_ms: i64,
    sample_rate: u32,
    channels: u16,
    samples: &'a [i16],
}

impl LivePreviewController {
    pub fn sink(&self) -> LivePreviewSink {
        self.sink.clone()
    }

    pub fn cancel(self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Drop for LivePreviewController {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl SystemLivePreviewController {
    pub fn cancel(self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

impl Drop for SystemLivePreviewController {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

pub fn start_live_transcript_preview(
    app: AppHandle,
    note_id: String,
    session_id: String,
    source_mode: RecordingSourceMode,
    source: RecordingSource,
    sample_rate: u32,
    channels: u16,
) -> LivePreviewController {
    let (sender, receiver) = mpsc::channel(PREVIEW_BATCH_BUFFER);
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    tauri::async_runtime::spawn(async move {
        run_live_preview_worker(LivePreviewWorkerParams {
            app,
            note_id,
            session_id,
            source_mode,
            source,
            sample_rate: sample_rate.max(1),
            channels: channels.max(1),
            receiver,
            cancelled: worker_cancelled,
        })
        .await;
    });
    LivePreviewController {
        cancelled,
        sink: LivePreviewSink {
            sender,
            next_sample_index: Arc::new(AtomicU64::new(0)),
        },
    }
}

pub fn start_system_live_transcript_preview(
    app: AppHandle,
    note_id: String,
    session_id: String,
    source_mode: RecordingSourceMode,
    partial_path: PathBuf,
) -> SystemLivePreviewController {
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    tauri::async_runtime::spawn(async move {
        run_system_live_preview_worker(
            app,
            note_id,
            session_id,
            source_mode,
            partial_path,
            worker_cancelled,
        )
        .await;
    });
    SystemLivePreviewController { cancelled }
}

async fn run_live_preview_worker(params: LivePreviewWorkerParams) {
    let LivePreviewWorkerParams {
        app,
        note_id,
        session_id,
        source_mode,
        source,
        sample_rate,
        channels,
        mut receiver,
        cancelled,
    } = params;
    let chunk_samples = samples_for_ms(sample_rate, channels, PREVIEW_CHUNK_MS);
    if chunk_samples == 0 {
        return;
    }
    let mut buffer: Vec<i16> = Vec::with_capacity(chunk_samples * 2);
    let mut buffer_start_sample = 0_u64;
    let mut segment_index = 0_i64;

    while let Some(batch) = receiver.recv().await {
        if cancelled.load(Ordering::Acquire) {
            return;
        }
        let batch = if should_drop_stale_batches(receiver.len()) {
            newest_preview_batch(batch, &mut receiver)
        } else {
            batch
        };
        let expected_start = buffer_start_sample.saturating_add(buffer.len() as u64);
        if buffer.is_empty() || batch.start_sample != expected_start {
            buffer.clear();
            buffer_start_sample = batch.start_sample;
        }
        buffer.extend(batch.samples);
        while buffer.len() >= chunk_samples {
            let chunk_start_sample = buffer_start_sample;
            let samples = buffer.drain(..chunk_samples).collect::<Vec<_>>();
            buffer_start_sample = buffer_start_sample.saturating_add(chunk_samples as u64);
            let start_ms = sample_offset_ms(chunk_start_sample, sample_rate, channels);
            let end_ms = sample_offset_ms(buffer_start_sample, sample_rate, channels);
            if is_effectively_silent(&samples, sample_rate, channels) {
                segment_index += 1;
                continue;
            }
            // The Live transcription setting is re-read per chunk so turning
            // it off mid-recording stops new preview requests (and their
            // billing) promptly; turning it back on resumes the stream.
            // (JUN-375)
            let Ok(epoch) = crate::meeting_assist::preview_chunk_epoch(&note_id, &session_id)
            else {
                segment_index += 1;
                continue;
            };
            let segment_id = preview_segment_id(source, segment_index);
            if let Some(event) = transcribe_preview_chunk(PreviewChunkRequest {
                note_id: &note_id,
                session_id: &session_id,
                source_mode,
                source,
                segment_id: &segment_id,
                start_ms,
                end_ms,
                sample_rate,
                channels,
                samples: &samples,
            })
            .await
            {
                if cancelled.load(Ordering::Acquire) {
                    return;
                }
                crate::meeting_assist::publish_preview(event, epoch, |event| {
                    app.emit(LIVE_TRANSCRIPT_EVENT, event).is_ok()
                });
            }
            segment_index += 1;
        }
    }
}

async fn run_system_live_preview_worker(
    app: AppHandle,
    note_id: String,
    session_id: String,
    source_mode: RecordingSourceMode,
    partial_path: PathBuf,
    cancelled: Arc<AtomicBool>,
) {
    let mut reader = WavTailReader::new(partial_path);
    let mut buffer = Vec::new();
    let mut buffer_start_sample = 0_u64;
    let mut segment_index = 0_i64;
    let mut current_format = None;

    while !cancelled.load(Ordering::Acquire) {
        let mut had_samples = false;
        match reader.read_new_samples() {
            Ok(Some(read)) if !read.samples.is_empty() => {
                had_samples = true;
                let format = (read.sample_rate, read.channels);
                if current_format != Some(format) {
                    current_format = Some(format);
                    buffer.clear();
                    buffer_start_sample = read.start_sample;
                }

                let expected_start = buffer_start_sample.saturating_add(buffer.len() as u64);
                if buffer.is_empty() || read.start_sample != expected_start {
                    buffer.clear();
                    buffer_start_sample = read.start_sample;
                }
                buffer.extend(read.samples);

                let chunk_samples =
                    samples_for_ms(read.sample_rate, read.channels, PREVIEW_CHUNK_MS);
                let dropped = trim_stale_system_backlog(&mut buffer, chunk_samples);
                if dropped > 0 {
                    buffer_start_sample = buffer_start_sample.saturating_add(dropped as u64);
                    segment_index += (dropped / chunk_samples) as i64;
                }
                while chunk_samples > 0 && buffer.len() >= chunk_samples {
                    let chunk_start_sample = buffer_start_sample;
                    let samples = buffer.drain(..chunk_samples).collect::<Vec<_>>();
                    buffer_start_sample = buffer_start_sample.saturating_add(chunk_samples as u64);
                    let start_ms =
                        sample_offset_ms(chunk_start_sample, read.sample_rate, read.channels);
                    let end_ms =
                        sample_offset_ms(buffer_start_sample, read.sample_rate, read.channels);
                    if is_effectively_silent(&samples, read.sample_rate, read.channels) {
                        segment_index += 1;
                        continue;
                    }

                    // Same live setting check as the microphone lane: turning
                    // Live transcription off mid-recording stops new preview
                    // requests promptly (JUN-375).
                    let Ok(epoch) =
                        crate::meeting_assist::preview_chunk_epoch(&note_id, &session_id)
                    else {
                        segment_index += 1;
                        continue;
                    };
                    let segment_id = preview_segment_id(RecordingSource::System, segment_index);
                    if let Some(event) = transcribe_preview_chunk(PreviewChunkRequest {
                        note_id: &note_id,
                        session_id: &session_id,
                        source_mode,
                        source: RecordingSource::System,
                        segment_id: &segment_id,
                        start_ms,
                        end_ms,
                        sample_rate: read.sample_rate,
                        channels: read.channels,
                        samples: &samples,
                    })
                    .await
                    {
                        if cancelled.load(Ordering::Acquire) {
                            return;
                        }
                        crate::meeting_assist::publish_preview(event, epoch, |event| {
                            app.emit(LIVE_TRANSCRIPT_EVENT, event).is_ok()
                        });
                    }
                    segment_index += 1;
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => {}
            Err(error) => {
                eprintln!("live transcript preview failed to read system audio: {error}");
            }
        }

        if had_samples {
            continue;
        }
        tokio::time::sleep(Duration::from_millis(SYSTEM_PREVIEW_POLL_MS)).await;
    }
}

fn newest_preview_batch(
    mut batch: LivePreviewBatch,
    receiver: &mut mpsc::Receiver<LivePreviewBatch>,
) -> LivePreviewBatch {
    while let Ok(next) = receiver.try_recv() {
        batch = next;
    }
    batch
}

fn should_drop_stale_batches(queued_batches: usize) -> bool {
    queued_batches >= PREVIEW_STALE_BATCH_THRESHOLD
}

/// Drop whole stale chunks off the front of the system-preview buffer once the
/// backlog exceeds [`SYSTEM_PREVIEW_MAX_BACKLOG_CHUNKS`], returning how many
/// samples were discarded. Dropping in whole-chunk units keeps the retained
/// buffer chunk-aligned so timestamps and segment ids stay correct, and lets
/// the preview jump forward to recent audio instead of lagging the meeting.
fn trim_stale_system_backlog(buffer: &mut Vec<i16>, chunk_samples: usize) -> usize {
    if chunk_samples == 0 {
        return 0;
    }
    let max_len = chunk_samples.saturating_mul(SYSTEM_PREVIEW_MAX_BACKLOG_CHUNKS);
    if buffer.len() <= max_len {
        return 0;
    }
    let excess = buffer.len() - max_len;
    // Round the excess up to a whole number of chunks so we never leave a
    // partial chunk that would shift every later frame boundary.
    let excess_chunks = excess.div_ceil(chunk_samples);
    // The .min is defensive only: with max_len = 2 * chunk_samples the excess
    // rounded up to whole chunks always fits inside the buffer.
    let drop = excess_chunks
        .saturating_mul(chunk_samples)
        .min(buffer.len());
    buffer.drain(..drop);
    drop
}

struct WavTailRead {
    start_sample: u64,
    sample_rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

struct WavTailReader {
    path: PathBuf,
    data_offset: Option<u64>,
    sample_rate: u32,
    channels: u16,
    next_byte_offset: u64,
}

struct WavLayout {
    data_offset: u64,
    sample_rate: u32,
    channels: u16,
}

impl WavTailReader {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            data_offset: None,
            sample_rate: 0,
            channels: 0,
            next_byte_offset: 0,
        }
    }

    fn read_new_samples(&mut self) -> std::io::Result<Option<WavTailRead>> {
        if self.data_offset.is_none() {
            let Some(layout) = read_wav_layout(&self.path)? else {
                return Ok(None);
            };
            self.data_offset = Some(layout.data_offset);
            self.sample_rate = layout.sample_rate;
            self.channels = layout.channels;
            self.next_byte_offset = layout.data_offset;
        }

        let data_offset = self.data_offset.unwrap_or(0);
        let file_len = std::fs::metadata(&self.path)?.len();
        if file_len < data_offset || file_len < self.next_byte_offset {
            self.data_offset = None;
            self.next_byte_offset = 0;
            return Ok(None);
        }

        let available_bytes = file_len.saturating_sub(self.next_byte_offset);
        let aligned_bytes = available_bytes - (available_bytes % 2);
        if aligned_bytes == 0 {
            return Ok(None);
        }

        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.next_byte_offset))?;
        let mut bytes = vec![0_u8; aligned_bytes as usize];
        match file.read_exact(&mut bytes) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }

        let start_sample = self.next_byte_offset.saturating_sub(data_offset) / 2;
        self.next_byte_offset = self.next_byte_offset.saturating_add(aligned_bytes);
        let samples = bytes
            .chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();

        Ok(Some(WavTailRead {
            start_sample,
            sample_rate: self.sample_rate.max(1),
            channels: self.channels.max(1),
            samples,
        }))
    }
}

fn read_wav_layout(path: &Path) -> std::io::Result<Option<WavLayout>> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len < 44 {
        return Ok(None);
    }

    let mut riff = [0_u8; 12];
    file.read_exact(&mut riff)?;
    if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
        return Ok(None);
    }

    // Walk the chunk table by seeking rather than buffering a fixed prefix.
    // AVAudioFile (the macOS system-audio helper) page-aligns the PCM data with
    // a large `FLLR`/`JUNK` padding chunk, so the `data` chunk can sit well past
    // any fixed scan window. A bounded scan silently returned `Ok(None)` for
    // those files, which quietly disabled the whole system live-preview lane.
    let mut offset = 12_u64;
    let mut sample_rate = None;
    let mut channels = None;

    while offset.saturating_add(8) <= file_len {
        file.seek(SeekFrom::Start(offset))?;
        let mut header = [0_u8; 8];
        match file.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => break,
            Err(error) => return Err(error),
        }
        let chunk_id = &header[0..4];
        let chunk_size = u64::from(u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]));
        let chunk_data_start = offset + 8;

        if chunk_id == b"data" {
            // The `data` header can be flushed before `fmt` in a partially
            // written file; only report a layout once both are known.
            return match (sample_rate, channels) {
                (Some(sample_rate), Some(channels)) => Ok(Some(WavLayout {
                    data_offset: chunk_data_start,
                    sample_rate,
                    channels,
                })),
                _ => Ok(None),
            };
        }

        if chunk_id == b"fmt " {
            if chunk_data_start.saturating_add(16) > file_len {
                return Ok(None);
            }
            let mut fmt = [0_u8; 16];
            file.read_exact(&mut fmt)?;
            let audio_format = u16::from_le_bytes([fmt[0], fmt[1]]);
            let parsed_channels = u16::from_le_bytes([fmt[2], fmt[3]]);
            let parsed_sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
            let bits_per_sample = u16::from_le_bytes([fmt[14], fmt[15]]);
            if !matches!(audio_format, 1 | 0xfffe)
                || bits_per_sample != 16
                || parsed_channels == 0
                || parsed_sample_rate == 0
            {
                return Ok(None);
            }
            channels = Some(parsed_channels);
            sample_rate = Some(parsed_sample_rate);
        }

        let padded_size = chunk_size.saturating_add(chunk_size % 2);
        offset = chunk_data_start.saturating_add(padded_size);
    }

    Ok(None)
}

async fn transcribe_preview_chunk(
    request: PreviewChunkRequest<'_>,
) -> Option<LiveTranscriptEventDto> {
    let PreviewChunkRequest {
        note_id,
        session_id,
        source_mode,
        source,
        segment_id,
        start_ms,
        end_ms,
        sample_rate,
        channels,
        samples,
    } = request;
    // The preview shares the provider account with the final transcription:
    // it takes a request only when one is free now and within its part of
    // the hour's audio, and otherwise drops the chunk (ADR-0002 lets preview
    // audio be dropped). The chunk then goes with the same key whose pacer
    // gave it the request.
    let route = crate::domain::processing::live_preview_route(end_ms - start_ms)?;
    let temp_path = preview_chunk_path(session_id, segment_id);
    if let Err(error) = write_preview_wav(&temp_path, sample_rate, channels, samples) {
        let _ = std::fs::remove_file(&temp_path);
        eprintln!("live transcript preview failed to write chunk: {error}");
        return None;
    }

    let normalized_path = preview_normalized_path(&temp_path);
    let audio_path = match normalize_preview_audio(&temp_path) {
        Ok(path) => path,
        Err(error) => {
            let _ = std::fs::remove_file(&temp_path);
            let _ = std::fs::remove_file(&normalized_path);
            eprintln!(
                "live transcript preview failed to normalize {} chunk: {} ({})",
                source.as_db(),
                error.message,
                error.code
            );
            return None;
        }
    };

    let request = preview_transcription_request(audio_path.clone(), session_id, segment_id);
    let result = crate::domain::processing::transcribe_on_route(route, request).await;
    let _ = std::fs::remove_file(&temp_path);
    if audio_path != temp_path {
        let _ = std::fs::remove_file(&audio_path);
    }
    match result {
        Ok(transcript) => {
            let text = transcript.text.trim().to_string();
            if text.is_empty() {
                return None;
            }
            Some(LiveTranscriptEventDto {
                note_id: note_id.to_string(),
                session_id: session_id.to_string(),
                source_mode,
                source,
                segment_id: segment_id.to_string(),
                start_ms,
                end_ms,
                text,
                language: transcript.language,
                stability: "final",
            })
        }
        Err(error) => {
            eprintln!(
                "live transcript preview transcription failed: {} ({})",
                error.message, error.code
            );
            None
        }
    }
}

fn preview_normalized_path(input_path: &Path) -> PathBuf {
    input_path.with_extension("normalized.wav")
}

fn normalize_preview_audio(input_path: &Path) -> Result<PathBuf, crate::domain::types::AppError> {
    normalize_wav_for_transcription(input_path, &preview_normalized_path(input_path))
}

fn preview_transcription_request(
    audio_path: PathBuf,
    session_id: &str,
    segment_id: &str,
) -> TranscriptionRequest {
    TranscriptionRequest {
        provider: crate::providers::configured_transcription_provider(),
        audio_path,
        title: "Live transcript preview".to_string(),
        // Never feed unverified preview guesses back into ASR. A single
        // hallucinated sparse System chunk otherwise primes every following
        // chunk and produces a coherent but fictional conversation.
        context: None,
        language: crate::dictation::configured_transcription_language(),
        operation_id: Some(format!("live-preview-{session_id}-{segment_id}")),
        preview: true,
    }
}

fn preview_chunk_path(session_id: &str, segment_id: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "clovy-live-preview-{}-{}-{}.wav",
        session_id,
        segment_id,
        Uuid::new_v4()
    ))
}

fn preview_segment_id(source: RecordingSource, segment_index: i64) -> String {
    format!("{}-{segment_index}", source.as_db())
}

fn write_preview_wav(
    path: &Path,
    sample_rate: u32,
    channels: u16,
    samples: &[i16],
) -> Result<(), hound::Error> {
    let spec = WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()
}

fn samples_for_ms(sample_rate: u32, channels: u16, duration_ms: i64) -> usize {
    ((u64::from(sample_rate) * u64::from(channels) * duration_ms.max(0) as u64) / 1000) as usize
}

#[cfg(test)]
fn duration_ms(sample_count: usize, sample_rate: u32, channels: u16) -> i64 {
    let frames = sample_count as u64 / u64::from(channels.max(1));
    ((frames * 1000) / u64::from(sample_rate.max(1))) as i64
}

fn sample_offset_ms(sample_index: u64, sample_rate: u32, channels: u16) -> i64 {
    let frames = sample_index / u64::from(channels.max(1));
    let millis = frames
        .saturating_mul(1000)
        .checked_div(u64::from(sample_rate.max(1)))
        .unwrap_or(0);
    millis.min(i64::MAX as u64) as i64
}

fn is_effectively_silent(samples: &[i16], sample_rate: u32, channels: u16) -> bool {
    if samples.is_empty() {
        return true;
    }
    let samples_per_window =
        samples_for_ms(sample_rate, channels, PREVIEW_ACTIVITY_WINDOW_MS).max(1);
    let required_windows = ((PREVIEW_MIN_SUSTAINED_ACTIVITY_MS + PREVIEW_ACTIVITY_WINDOW_MS - 1)
        / PREVIEW_ACTIVITY_WINDOW_MS)
        .max(1);
    let mut active_run = 0_i64;
    for window in samples.chunks(samples_per_window) {
        let sum_square = window
            .iter()
            .map(|sample| {
                let normalized = *sample as f64 / i16::MAX as f64;
                normalized * normalized
            })
            .sum::<f64>();
        let rms = (sum_square / window.len() as f64).sqrt() as f32;
        if rms >= PREVIEW_SILENCE_RMS_FLOOR {
            active_run += 1;
            if active_run >= required_windows {
                return false;
            }
        } else {
            active_run = 0;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use crate::domain::types::RecordingSource;

    use super::{
        duration_ms, is_effectively_silent, newest_preview_batch, normalize_preview_audio,
        preview_chunk_path, preview_segment_id, preview_transcription_request, read_wav_layout,
        sample_offset_ms, samples_for_ms, should_drop_stale_batches, trim_stale_system_backlog,
        write_preview_wav, LivePreviewBatch, LivePreviewSink, WavTailReader,
        PREVIEW_STALE_BATCH_THRESHOLD, SYSTEM_PREVIEW_MAX_BACKLOG_CHUNKS,
    };
    use std::{
        io::Write,
        sync::{
            atomic::{AtomicU64, Ordering},
            Arc,
        },
    };
    use tokio::sync::mpsc;

    /// Build a WAV header shaped like the one `AVAudioFile(forWriting:)` writes
    /// for the macOS system-audio helper: a `JUNK` chunk, a PCM `fmt ` chunk, an
    /// `FLLR` padding chunk that page-aligns the audio, and a `data` chunk whose
    /// declared size stays `0` while the file is still being recorded.
    fn avaudiofile_style_wav(
        sample_rate: u32,
        channels: u16,
        fllr_len: usize,
        samples: &[i16],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(b"RIFF");
        // AVAudioFile leaves the RIFF size small/stale while recording; the
        // reader must not trust it.
        buf.extend_from_slice(&0_u32.to_le_bytes());
        buf.extend_from_slice(b"WAVE");
        // JUNK padding chunk (28 bytes, exactly what AVAudioFile emits).
        buf.extend_from_slice(b"JUNK");
        buf.extend_from_slice(&28_u32.to_le_bytes());
        buf.extend(std::iter::repeat(0_u8).take(28));
        // PCM fmt chunk.
        buf.extend_from_slice(b"fmt ");
        buf.extend_from_slice(&16_u32.to_le_bytes());
        buf.extend_from_slice(&1_u16.to_le_bytes());
        buf.extend_from_slice(&channels.to_le_bytes());
        buf.extend_from_slice(&sample_rate.to_le_bytes());
        buf.extend_from_slice(&(sample_rate * u32::from(channels) * 2).to_le_bytes());
        buf.extend_from_slice(&(channels * 2).to_le_bytes());
        buf.extend_from_slice(&16_u16.to_le_bytes());
        // FLLR page-alignment padding chunk.
        buf.extend_from_slice(b"FLLR");
        buf.extend_from_slice(&(fllr_len as u32).to_le_bytes());
        buf.extend(std::iter::repeat(0_u8).take(fllr_len));
        // data chunk with a zero declared size (still recording).
        buf.extend_from_slice(b"data");
        buf.extend_from_slice(&0_u32.to_le_bytes());
        for sample in samples {
            buf.extend_from_slice(&sample.to_le_bytes());
        }
        buf
    }

    fn write_temp_wav(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = preview_chunk_path("avaudiofile", name);
        let mut file = std::fs::File::create(&path).expect("create wav");
        file.write_all(bytes).expect("write wav");
        path
    }

    #[test]
    fn computes_chunk_sample_counts_for_interleaved_audio() {
        assert_eq!(samples_for_ms(16_000, 1, 8_000), 128_000);
        assert_eq!(samples_for_ms(48_000, 2, 1_000), 96_000);
        assert_eq!(duration_ms(96_000, 48_000, 2), 1_000);
        assert_eq!(sample_offset_ms(96_000, 48_000, 2), 1_000);
    }

    #[test]
    fn silence_gate_requires_sustained_preview_activity() {
        let mut transient = vec![0_i16; 16_000];
        transient[..2_400].fill(2_000);
        let mut short_reply = vec![0_i16; 16_000];
        short_reply[..2_880].fill(2_000);

        assert!(is_effectively_silent(&[0; 16_000], 16_000, 1));
        assert!(is_effectively_silent(&[10; 16_000], 16_000, 1));
        assert!(is_effectively_silent(&transient, 16_000, 1));
        assert!(!is_effectively_silent(&short_reply, 16_000, 1));
        assert!(!is_effectively_silent(&[2_000; 16_000], 16_000, 1));
    }

    #[test]
    fn preview_sink_drops_batches_when_buffer_is_full() {
        let (sender, _receiver) = mpsc::channel(1);
        let sink = LivePreviewSink {
            sender,
            next_sample_index: Arc::new(AtomicU64::new(0)),
        };

        assert!(sink.try_send(vec![1]));
        assert!(!sink.try_send(vec![2]));
    }

    #[test]
    fn preview_sink_accounts_for_dropped_batches_in_sample_offsets() {
        let (sender, mut receiver) = mpsc::channel(1);
        let sink = LivePreviewSink {
            sender,
            next_sample_index: Arc::new(AtomicU64::new(0)),
        };

        assert!(sink.try_send(vec![1, 2]));
        assert!(!sink.try_send(vec![3, 4, 5]));
        let first = receiver.try_recv().expect("first batch");
        assert_eq!(first.start_sample, 0);

        assert!(sink.try_send(vec![6]));
        let second = receiver.try_recv().expect("second batch");
        assert_eq!(second.start_sample, 5);
        assert_eq!(sink.next_sample_index.load(Ordering::Relaxed), 6);
    }

    #[test]
    fn newest_preview_batch_drops_queued_stale_batches() {
        let (sender, mut receiver) = mpsc::channel(4);
        sender
            .try_send(LivePreviewBatch {
                start_sample: 0,
                samples: vec![1],
            })
            .expect("send first");
        sender
            .try_send(LivePreviewBatch {
                start_sample: 1,
                samples: vec![2],
            })
            .expect("send second");
        sender
            .try_send(LivePreviewBatch {
                start_sample: 2,
                samples: vec![3],
            })
            .expect("send third");

        let first = receiver.try_recv().expect("queued first");
        let newest = newest_preview_batch(first, &mut receiver);

        assert_eq!(newest.start_sample, 2);
        assert_eq!(newest.samples, vec![3]);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn stale_preview_batches_are_dropped_only_after_real_backlog() {
        assert!(!should_drop_stale_batches(0));
        assert!(!should_drop_stale_batches(1));
        assert!(!should_drop_stale_batches(
            PREVIEW_STALE_BATCH_THRESHOLD - 1
        ));
        assert!(should_drop_stale_batches(PREVIEW_STALE_BATCH_THRESHOLD));
    }

    #[test]
    fn preview_segment_ids_include_audio_source() {
        assert_eq!(
            preview_segment_id(RecordingSource::Microphone, 7),
            "microphone-7"
        );
        assert_eq!(preview_segment_id(RecordingSource::System, 3), "system-3");
    }

    #[test]
    fn preview_audio_is_normalized_and_never_primed_by_preview_text() {
        let input_path = preview_chunk_path("normalization", "system-stereo");
        let samples = (0..48_000)
            .flat_map(|index| {
                let sample = if index % 200 < 100 { 4_000 } else { -4_000 };
                [sample, sample]
            })
            .collect::<Vec<_>>();
        write_preview_wav(&input_path, 48_000, 2, &samples).expect("write stereo preview");

        let normalized_path = normalize_preview_audio(&input_path).expect("normalize preview");
        let reader = hound::WavReader::open(&normalized_path).expect("read normalized preview");
        assert_eq!(reader.spec().sample_rate, 16_000);
        assert_eq!(reader.spec().channels, 1);

        let request =
            preview_transcription_request(normalized_path.clone(), "recording-session", "system-0");
        assert!(request.context.is_none());
        assert_eq!(
            request.operation_id.as_deref(),
            Some("live-preview-recording-session-system-0")
        );

        let _ = std::fs::remove_file(input_path);
        let _ = std::fs::remove_file(normalized_path);
    }

    #[test]
    fn wav_tail_reader_reads_samples_once() {
        let path = preview_chunk_path("tail-test", "reader");
        let samples = vec![1, -2, 300, -400];
        write_preview_wav(&path, 16_000, 2, &samples).expect("write wav");

        let mut reader = WavTailReader::new(path.clone());
        let read = reader
            .read_new_samples()
            .expect("read samples")
            .expect("samples");

        assert_eq!(read.start_sample, 0);
        assert_eq!(read.sample_rate, 16_000);
        assert_eq!(read.channels, 2);
        assert_eq!(read.samples, samples);
        assert!(reader.read_new_samples().expect("read none").is_none());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn wav_layout_waits_for_complete_header() {
        let path = preview_chunk_path("tail-test", "incomplete");
        std::fs::write(&path, b"RIFF").expect("write incomplete wav");

        let layout = read_wav_layout(&path).expect("read layout");

        assert!(layout.is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn reads_avaudiofile_style_partial_header() {
        // Mirrors the real macOS helper file mid-recording: JUNK + fmt + FLLR
        // padding + a zero-sized `data` chunk, followed by interleaved PCM.
        let samples = vec![1, -2, 300, -400, 5, -6];
        let bytes = avaudiofile_style_wav(48_000, 2, 4_008, &samples);
        let path = write_temp_wav("partial-header", &bytes);

        let layout = read_wav_layout(&path)
            .expect("read layout")
            .expect("layout present");
        assert_eq!(layout.sample_rate, 48_000);
        assert_eq!(layout.channels, 2);

        let mut reader = WavTailReader::new(path.clone());
        let read = reader
            .read_new_samples()
            .expect("read samples")
            .expect("samples");
        assert_eq!(read.start_sample, 0);
        assert_eq!(read.sample_rate, 48_000);
        assert_eq!(read.channels, 2);
        assert_eq!(read.samples, samples);
        assert!(reader.read_new_samples().expect("read none").is_none());

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn finds_data_chunk_beyond_a_64kb_header() {
        // Regression: a bounded 64KB header scan returned `Ok(None)` forever
        // when AVAudioFile's page-alignment padding pushed the `data` chunk past
        // the window, silently disabling the whole system live-preview lane. The
        // seek-based walk must still locate it.
        let samples = vec![7, -8, 9, -10];
        let bytes = avaudiofile_style_wav(44_100, 1, 70_000, &samples);
        assert!(
            bytes.len() > 64 * 1024,
            "padding must push data past the old scan window"
        );
        let path = write_temp_wav("data-past-64k", &bytes);

        let layout = read_wav_layout(&path)
            .expect("read layout")
            .expect("layout present past 64KB header");
        assert_eq!(layout.sample_rate, 44_100);
        assert_eq!(layout.channels, 1);
        assert!(layout.data_offset > 64 * 1024);

        let mut reader = WavTailReader::new(path.clone());
        let read = reader
            .read_new_samples()
            .expect("read samples")
            .expect("samples past 64KB header");
        assert_eq!(read.samples, samples);

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn trims_stale_system_backlog_to_whole_chunks() {
        // Five chunks buffered; the cap keeps only the most recent chunks so the
        // preview jumps to current audio instead of transcribing stale windows.
        let chunk = 4;
        let mut buffer: Vec<i16> = (0..20).collect();
        let dropped = trim_stale_system_backlog(&mut buffer, chunk);
        let expected_drop = 20 - chunk * SYSTEM_PREVIEW_MAX_BACKLOG_CHUNKS;
        assert_eq!(dropped, expected_drop);
        assert_eq!(dropped % chunk, 0, "must drop whole chunks only");
        assert_eq!(buffer, (expected_drop as i16..20).collect::<Vec<i16>>());
    }

    #[test]
    fn keeps_system_backlog_within_the_cap_intact() {
        let chunk = 4;
        let mut buffer: Vec<i16> = (0..chunk as i16 * 2).collect();
        assert_eq!(trim_stale_system_backlog(&mut buffer, chunk), 0);
        assert_eq!(buffer.len(), chunk * 2);
        // A zero chunk size (unknown format) must be a no-op, never a divide.
        let mut empty: Vec<i16> = vec![1, 2, 3];
        assert_eq!(trim_stale_system_backlog(&mut empty, 0), 0);
        assert_eq!(empty, vec![1, 2, 3]);
    }
}
