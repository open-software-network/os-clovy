use crate::providers::transcription::TranscriptionBackend;
use crate::{
    app_paths::AppPaths,
    audio::{
        live_preview::PREVIEW_CHUNK_MS,
        turns::{
            coalesce_turns_for_transcription_with_max_span, detect_turns_with_report,
            normalize_wav_for_transcription, split_wav_for_transcription,
            split_wav_for_transcription_with_max_duration, write_turn_wav, AudioTurn,
            DetectionSource, EchoRejectionReport, FINAL_TRANSCRIPTION_SPAN_MS,
            LEGACY_TRANSCRIPTION_SPAN_MS, SPLIT_OVERLAP_MS,
        },
    },
    db::repositories::{NoteTranscriptionChunk, Repositories},
    domain::types::{
        AppError, DictionaryEntryDto, DiscardedTranscriptSpanDto, NoteDto,
        NoteProcessingProgressDto, NoteProcessingStage, NoteTranscriptionJobKind,
        NoteTranscriptionJobPlan, NoteTranscriptionJobRecord, NoteTranscriptionJobStatus,
        ProcessingStatus, RecordingSource, RecordingSourceMode, TranscriptDto,
    },
    providers::{
        groq::{
            TimedTranscription, TranscriptSegment, TranscriptionProviderResult,
            TranscriptionRequest,
        },
        notes::{generate_note_from_transcript, GenerationRequest},
    },
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub const PROMPT_VERSION: &str = "notes-mvp-v5";
const NOTE_TRANSCRIPT_CLEANUP_TIMEOUT_MS: u64 = 5_000;
const TRANSCRIPT_COHERENCE_GAP_MS: i64 = 2_500;
/// How far a cached transcript's turn bounds may differ from the re-detected
/// turn before positional reuse is refused and the turn is re-transcribed.
/// Detection is deterministic for unchanged audio and code, so matching turns
/// agree exactly; any real drift means the turn set was reshaped.
#[cfg(test)]
const CACHED_TURN_BOUNDS_TOLERANCE_MS: i64 = 50;
const TRANSCRIPTION_CONTEXT_MAX_CHARS: usize = 1_200;
const TRANSCRIPTION_CONTEXT_MAX_TURNS: usize = 6;
const DICTIONARY_CONTEXT_MAX_ENTRIES: usize = 80;
/// Byte budget of the dictionary hint sent as Whisper's `prompt`, separators
/// included. Groq documents that prompt as limited to 224 tokens, and Whisper's
/// tiktoken BPE never encodes fewer than one byte per token, so 200 bytes is a
/// conservative local envelope. It is not a count of Groq's tokens.
const ASR_DICTIONARY_HINT_MAX_BYTES: usize = 200;
const ASR_DICTIONARY_HINT_SEPARATOR: &str = ", ";
const DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY: usize = 2;
const PREPARED_TURN_CHANNEL_CAPACITY: usize = DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY;
const NOTE_TRANSCRIPTION_PIPELINE_VERSION: &str = "saved-audio-v3";
const TRANSCRIPT_COVERAGE_WARN_RATIO: f64 = 0.8;
const TRANSCRIPT_COVERAGE_WARN_MIN_MISSING_MS: i64 = 60_000;
const TRANSIENT_TRANSCRIPTION_ATTEMPTS: usize = 3;
const SHORT_FULL_SOURCE_RECOVERY_MAX_MS: i64 = 2 * 60 * 1000;
#[cfg(not(test))]
const TRANSIENT_TRANSCRIPTION_RETRY_BASE_BACKOFF_MS: u64 = 300;
#[cfg(test)]
const TRANSIENT_TRANSCRIPTION_RETRY_BASE_BACKOFF_MS: u64 = 1;
#[cfg(not(test))]
const TRANSIENT_TRANSCRIPTION_RETRY_JITTER_MS: u64 = 200;
#[cfg(test)]
const TRANSIENT_TRANSCRIPTION_RETRY_JITTER_MS: u64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTranscriptInput {
    pub source: String,
    pub text: String,
    pub valid: bool,
    pub warning: Option<String>,
    pub recorded_silence: bool,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub turn_index: Option<i64>,
}

pub type SourceAudioForProcessing = (String, String, PathBuf, bool);

pub const NOTE_PROCESSING_PROGRESS_EVENT: &str = "note-processing-progress";

#[cfg(test)]
tokio::task_local! {
    static TEST_GENERATION_COMMITTED: tokio::sync::mpsc::UnboundedSender<()>;
}

type NoteProcessingProgressSink = Arc<dyn Fn(NoteProcessingProgressDto) + Send + Sync>;

#[derive(Clone, Default)]
pub(crate) struct NoteProcessingProgressReporter {
    sink: Option<NoteProcessingProgressSink>,
    discarded_spans: Arc<Mutex<Vec<DiscardedTranscriptSpanDto>>>,
}

impl NoteProcessingProgressReporter {
    pub(crate) fn new(sink: impl Fn(NoteProcessingProgressDto) + Send + Sync + 'static) -> Self {
        Self {
            sink: Some(Arc::new(sink)),
            discarded_spans: Arc::default(),
        }
    }

    fn scoped(&self) -> Self {
        Self {
            sink: self.sink.clone(),
            discarded_spans: Arc::default(),
        }
    }

    pub(crate) fn emit(
        &self,
        note_id: &str,
        recording_session_id: &str,
        stage: NoteProcessingStage,
        processing_status: ProcessingStatus,
        revision: String,
    ) {
        let Some(sink) = &self.sink else {
            return;
        };
        sink(NoteProcessingProgressDto {
            note_id: note_id.to_string(),
            recording_session_id: recording_session_id.to_string(),
            stage,
            processing_status,
            revision,
            discarded_spans: if stage == NoteProcessingStage::Done {
                self.discarded_spans
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone()
            } else {
                Vec::new()
            },
        });
    }
}

async fn transition_note_processing_stage(
    repos: &Repositories,
    note_id: &str,
    recording_session_id: &str,
    stage: NoteProcessingStage,
    processing_status: ProcessingStatus,
    progress: &NoteProcessingProgressReporter,
) -> Result<(), AppError> {
    let revision = repos
        .set_note_status(note_id, processing_status, None)
        .await?;
    progress.emit(
        note_id,
        recording_session_id,
        stage,
        processing_status,
        revision,
    );
    Ok(())
}

async fn complete_note_processing_run(
    repos: &Repositories,
    progress: &NoteProcessingProgressReporter,
    note_id: &str,
    recording_session_id: &str,
    result: Result<NoteDto, AppError>,
) -> Result<NoteDto, AppError> {
    match result {
        Ok(note) => {
            progress.emit(
                note_id,
                recording_session_id,
                NoteProcessingStage::Done,
                note.processing_status,
                note.updated_at.clone(),
            );
            Ok(note)
        }
        Err(error) => {
            match repos
                .set_note_status(
                    note_id,
                    ProcessingStatus::Failed,
                    Some(error.message.clone()),
                )
                .await
            {
                Ok(revision) => progress.emit(
                    note_id,
                    recording_session_id,
                    NoteProcessingStage::Done,
                    ProcessingStatus::Failed,
                    revision,
                ),
                Err(status_error) => {
                    tracing::warn!(
                        note_id,
                        recording_session_id,
                        %status_error,
                        "failed to persist terminal note processing status"
                    );
                }
            }
            Err(error)
        }
    }
}

pub fn valid_sources_for_processing(
    sources: Vec<SourceTranscriptInput>,
) -> Vec<SourceTranscriptInput> {
    sources
        .into_iter()
        .filter(|source| source.valid && !source.text.trim().is_empty())
        .collect()
}

fn source_transcript_input_from_row(row: &TranscriptDto) -> SourceTranscriptInput {
    SourceTranscriptInput {
        source: row
            .source
            .clone()
            .unwrap_or_else(|| "microphone".to_string()),
        text: row.text.clone(),
        valid: row.status == "succeeded" && !row.text.trim().is_empty(),
        warning: row.last_error.clone(),
        recorded_silence: row.recorded_silence,
        start_ms: row.start_ms,
        end_ms: row.end_ms,
        turn_index: row.turn_index,
    }
}

fn note_transcription_span_id(
    session_id: &str,
    source: &str,
    kind: NoteTranscriptionJobKind,
    start_ms: i64,
    end_ms: i64,
) -> String {
    format!("{session_id}:{source}:{}:{start_ms}:{end_ms}", kind.as_db())
}

/// What a job's transcript depends on beyond its audio: the language, the
/// title, the dictionary and the transcription model. The default model adds
/// nothing, so turns transcribed before the model was part of the identity
/// keep it and are reused; any other model is another transcription.
fn note_transcription_configuration_fingerprint(
    title: &str,
    dictionary_context: Option<&str>,
    model: &str,
) -> String {
    let mut digest = Sha256::new();
    digest.update(b"june-note-transcription-configuration-v1\0");
    let model = (model != crate::providers::DEFAULT_TRANSCRIPTION_MODEL).then_some(model);
    for value in [
        crate::dictation::configured_transcription_language().as_deref(),
        Some(title),
        dictionary_context,
    ]
    .into_iter()
    .chain(model.map(Some))
    {
        match value {
            Some(value) => {
                digest.update(b"some");
                digest.update((value.len() as u64).to_be_bytes());
                digest.update(value.as_bytes());
            }
            None => digest.update(b"none"),
        }
    }
    format!("{:x}", digest.finalize())
}

#[cfg(test)]
fn cached_turn_is_reusable(existing: &TranscriptDto, turn: &AudioTurn) -> bool {
    let bounds_match = existing
        .start_ms
        .is_some_and(|start| (start - turn.start_ms).abs() <= CACHED_TURN_BOUNDS_TOLERANCE_MS)
        && existing
            .end_ms
            .is_some_and(|end| (end - turn.end_ms).abs() <= CACHED_TURN_BOUNDS_TOLERANCE_MS);
    bounds_match && !is_short_full_source(&turn.source_path, turn.start_ms, turn.end_ms)
}

fn elapsed_ms(started: Instant) -> i64 {
    started.elapsed().as_millis().min(i64::MAX as u128) as i64
}

const UNSET_TIMING_MS: i64 = -1;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ProcessingTiming {
    done_started: Option<Instant>,
}

impl ProcessingTiming {
    pub(crate) fn from_done(done_started: Instant) -> Self {
        Self {
            done_started: Some(done_started),
        }
    }

    pub(crate) fn untracked() -> Self {
        Self::default()
    }

    fn done_to_duration_ms(self) -> Option<i64> {
        self.done_started.map(elapsed_ms)
    }

    pub(crate) fn checkpoint_details(self, mut details: serde_json::Value) -> String {
        if let (Some(duration_ms), Some(object)) =
            (self.done_to_duration_ms(), details.as_object_mut())
        {
            object.insert("doneToDurationMs".to_string(), duration_ms.into());
        }
        details.to_string()
    }
}

#[derive(Clone)]
struct FirstEventTimeline {
    timing: ProcessingTiming,
    first_request_ms: Arc<AtomicI64>,
    first_persisted_ms: Arc<AtomicI64>,
    flushed: Arc<AtomicBool>,
}

impl FirstEventTimeline {
    fn new(timing: ProcessingTiming) -> Self {
        Self {
            timing,
            first_request_ms: Arc::new(AtomicI64::new(UNSET_TIMING_MS)),
            first_persisted_ms: Arc::new(AtomicI64::new(UNSET_TIMING_MS)),
            flushed: Arc::new(AtomicBool::new(false)),
        }
    }

    fn mark_first_request(&self) {
        self.mark(&self.first_request_ms);
    }

    fn mark_first_persisted(&self) {
        self.mark(&self.first_persisted_ms);
    }

    fn mark(&self, slot: &AtomicI64) {
        if let Some(duration_ms) = self.timing.done_to_duration_ms() {
            let _ = slot.compare_exchange(
                UNSET_TIMING_MS,
                duration_ms,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }

    async fn flush(&self, repos: &Repositories, recording_session_id: &str) {
        if self.flushed.swap(true, Ordering::AcqRel) {
            return;
        }
        for (kind, duration_ms) in [
            (
                "first_note_transcription_request",
                self.first_request_ms.load(Ordering::Acquire),
            ),
            (
                "first_transcript_persisted",
                self.first_persisted_ms.load(Ordering::Acquire),
            ),
        ] {
            if duration_ms == UNSET_TIMING_MS {
                continue;
            }
            add_latency_checkpoint(
                repos,
                recording_session_id,
                kind,
                serde_json::json!({ "doneToDurationMs": duration_ms }).to_string(),
            )
            .await;
        }
    }
}

pub(crate) async fn add_latency_checkpoint(
    repos: &Repositories,
    recording_session_id: &str,
    kind: &str,
    details: String,
) {
    if let Err(error) = repos
        .add_checkpoint(recording_session_id, kind, Some(details))
        .await
    {
        tracing::warn!(recording_session_id, kind, %error, "failed to persist latency checkpoint");
    }
}

async fn add_processing_complete_checkpoint(
    repos: &Repositories,
    recording_session_id: &str,
    timing: ProcessingTiming,
    processing_started: Instant,
    status: &str,
) {
    add_latency_checkpoint(
        repos,
        recording_session_id,
        "processing_complete",
        timing.checkpoint_details(serde_json::json!({
            "durationMs": elapsed_ms(processing_started),
            "status": status,
        })),
    )
    .await;
}

async fn add_infrastructure_note_transcription_failure_checkpoint(
    repos: &Repositories,
    recording_session_id: &str,
    timing: ProcessingTiming,
    note_transcription_started: Instant,
    error_code: &str,
) {
    add_latency_checkpoint(
        repos,
        recording_session_id,
        "note_transcription_complete",
        timing.checkpoint_details(serde_json::json!({
            "durationMs": elapsed_ms(note_transcription_started),
            "status": "failed",
            "error": error_code,
        })),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn finalize_infrastructure_note_transcription_failure(
    repos: &Repositories,
    recording_session_id: &str,
    timeline: &FirstEventTimeline,
    timing: ProcessingTiming,
    note_transcription_started: Instant,
    processing_started: Instant,
    error: AppError,
) -> AppError {
    timeline.flush(repos, recording_session_id).await;
    add_infrastructure_note_transcription_failure_checkpoint(
        repos,
        recording_session_id,
        timing,
        note_transcription_started,
        &error.code,
    )
    .await;
    add_processing_complete_checkpoint(
        repos,
        recording_session_id,
        timing,
        processing_started,
        "failed",
    )
    .await;
    error
}

async fn persist_turn_preparation_checkpoint(
    repos: &Repositories,
    recording_session_id: &str,
    status: &str,
    report: Option<&TurnPreparationReport>,
    reused_transcript_count: usize,
    error: Option<&str>,
) {
    add_latency_checkpoint(
        repos,
        recording_session_id,
        "turn_wav_extraction",
        serde_json::json!({
            "durationMs": report.map(|value| value.active_preparation_duration_ms),
            "activePreparationDurationMs": report.map(|value| value.active_preparation_duration_ms),
            "producerWallDurationMs": report.map(|value| value.producer_wall_duration_ms),
            "doneToPreparationCompleteMs": report
                .and_then(|value| value.done_to_preparation_complete_ms),
            "jobCount": report.map(|value| value.prepared_count).unwrap_or(0),
            "reusedTranscriptCount": reused_transcript_count,
            "status": status,
            "error": error,
        })
        .to_string(),
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn persist_turn_pipeline_result(
    repos: &Repositories,
    recording_session_id: &str,
    pipeline_result: Result<TurnPipelineResult, TurnPipelineFailure>,
    reused_transcript_count: usize,
    timeline: &FirstEventTimeline,
    timing: ProcessingTiming,
    note_transcription_started: Instant,
    processing_started: Instant,
) -> Result<TurnPipelineResult, AppError> {
    match pipeline_result {
        Ok(pipeline) => {
            persist_turn_preparation_checkpoint(
                repos,
                recording_session_id,
                "succeeded",
                Some(&pipeline.preparation),
                reused_transcript_count,
                None,
            )
            .await;
            Ok(pipeline)
        }
        Err(failure) => {
            let error_code = failure.error.code.clone();
            persist_turn_preparation_checkpoint(
                repos,
                recording_session_id,
                "failed",
                failure.preparation.as_deref(),
                reused_transcript_count,
                Some(error_code.as_str()),
            )
            .await;
            timeline.flush(repos, recording_session_id).await;
            add_latency_checkpoint(
                repos,
                recording_session_id,
                "note_transcription_complete",
                timing.checkpoint_details(serde_json::json!({
                    "durationMs": elapsed_ms(note_transcription_started),
                    "status": "failed",
                    "error": error_code,
                })),
            )
            .await;
            add_processing_complete_checkpoint(
                repos,
                recording_session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            Err(failure.error)
        }
    }
}

struct TempDirCleanup(PathBuf);

impl Drop for TempDirCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn session_temp_dir(prefix: &str, session_id: &str) -> PathBuf {
    let safe_session_id = safe_temp_path_segment(session_id);
    std::env::temp_dir().join(format!("{prefix}-{safe_session_id}"))
}

fn safe_temp_path_segment(value: &str) -> String {
    let segment = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if segment.is_empty() {
        "unknown".to_string()
    } else {
        segment
    }
}

pub fn labeled_transcript_from_sources(sources: &[SourceTranscriptInput]) -> String {
    let mut sources = sources
        .iter()
        .filter(|source| source.valid && !source.text.trim().is_empty())
        .collect::<Vec<_>>();
    sources.sort_by(|left, right| {
        left.turn_index
            .unwrap_or(i64::MAX)
            .cmp(&right.turn_index.unwrap_or(i64::MAX))
            .then_with(|| {
                left.start_ms
                    .unwrap_or(i64::MAX)
                    .cmp(&right.start_ms.unwrap_or(i64::MAX))
            })
    });
    sources
        .into_iter()
        .map(|source| {
            let label = match source.source.as_str() {
                "system" => "System",
                _ => "Microphone",
            };
            format!("{label}: {}", source.text.trim())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Use the same authoritative selection as note reads and search. The
/// repository orders sessions before turns; sorting turn_index again here
/// would interleave separate recordings whose turn counters restart at zero.
async fn authoritative_generation_transcript(
    repos: &Repositories,
    note_id: &str,
) -> Result<(String, bool), AppError> {
    let rows = repos.source_transcripts(note_id).await?;
    let valid = rows
        .iter()
        .filter(|row| row.status == "succeeded" && !row.text.trim().is_empty())
        .collect::<Vec<_>>();
    let labels = valid
        .iter()
        .any(|row| row.source.as_deref() == Some("system"));
    let text = valid
        .into_iter()
        .map(|row| {
            if labels {
                let source = if row.source.as_deref() == Some("system") {
                    "System"
                } else {
                    "Microphone"
                };
                format!("{source}: {}", row.text.trim())
            } else {
                row.text.trim().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok((text, labels))
}

pub fn coalesce_source_transcripts(
    sources: Vec<SourceTranscriptInput>,
) -> Vec<SourceTranscriptInput> {
    let mut sources = ordered_source_transcripts(sources);
    let mut coalesced: Vec<SourceTranscriptInput> = Vec::new();
    for source in sources.drain(..) {
        if let Some(last) = coalesced.last_mut() {
            if can_coalesce_source_transcripts(last, &source) {
                last.text = join_transcript_text(&last.text, &source.text);
                last.end_ms = match (last.end_ms, source.end_ms) {
                    (Some(left), Some(right)) => Some(left.max(right)),
                    (None, value) | (value, None) => value,
                };
                continue;
            }
        }
        coalesced.push(source);
    }
    for (index, source) in coalesced.iter_mut().enumerate() {
        source.turn_index = Some(index as i64);
    }
    coalesced
}

pub fn build_transcription_context(previous: &[SourceTranscriptInput]) -> Option<String> {
    let valid = ordered_source_transcripts(previous.to_vec())
        .into_iter()
        .filter(|source| source.valid && !source.text.trim().is_empty())
        .collect::<Vec<_>>();
    if valid.is_empty() {
        return None;
    }
    let mut lines = valid
        .iter()
        .rev()
        .take(TRANSCRIPTION_CONTEXT_MAX_TURNS)
        .collect::<Vec<_>>();
    lines.reverse();
    let transcript = lines
        .into_iter()
        .map(|source| {
            let label = match source.source.as_str() {
                "system" => "System",
                _ => "Microphone",
            };
            format!("{label}: {}", source.text.trim())
        })
        .collect::<Vec<_>>()
        .join("\n");
    let transcript = tail_chars(&transcript, TRANSCRIPTION_CONTEXT_MAX_CHARS);
    Some(format!(
        "Previous transcript context:\n{transcript}\n\nPreserve the spoken language, vocabulary, names, and style when this audio continues the same conversation. Do not translate."
    ))
}

/// The custom dictionary as one transcription uses it. `context` is the full
/// dictionary block that the correction engines receive (note correction and
/// dictation cleanup); `asr_hint` is the short spelling hint the transcription
/// call itself receives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptionDictionary {
    pub context: String,
    pub asr_hint: Option<String>,
}

/// `None` when the dictionary has no terms.
pub fn build_transcription_dictionary(
    entries: &[DictionaryEntryDto],
) -> Option<TranscriptionDictionary> {
    let terms = entries
        .iter()
        .map(|entry| entry.phrase.trim())
        .filter(|phrase| !phrase.is_empty())
        .take(DICTIONARY_CONTEXT_MAX_ENTRIES)
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return None;
    }
    let lines = terms
        .iter()
        .map(|term| format!("- {term}"))
        .collect::<Vec<_>>();
    Some(TranscriptionDictionary {
        context: format!(
            "Custom dictionary terms:\n{}\n\nWhen the audio sounds like one of these words or phrases, prefer this exact spelling and capitalization.",
            lines.join("\n")
        ),
        asr_hint: asr_dictionary_hint(&terms),
    })
}

/// Whole dictionary terms in dictionary order, joined by `", "`, within
/// `ASR_DICTIONARY_HINT_MAX_BYTES` UTF-8 bytes including the separators. A term
/// that does not fit is skipped, never cut, and later terms still get their
/// chance; `None` when no term fits. No instructions, previous transcript, or
/// style ever go into the hint: the full context reaches the correction engine.
fn asr_dictionary_hint(terms: &[&str]) -> Option<String> {
    let mut hint = String::new();
    for term in terms {
        let separator = if hint.is_empty() {
            ""
        } else {
            ASR_DICTIONARY_HINT_SEPARATOR
        };
        if hint.len() + separator.len() + term.len() <= ASR_DICTIONARY_HINT_MAX_BYTES {
            hint.push_str(separator);
            hint.push_str(term);
        }
    }
    (!hint.is_empty()).then_some(hint)
}

pub fn merge_transcription_context(
    dictionary_context: Option<&str>,
    previous_context: Option<&str>,
) -> Option<String> {
    let parts = [dictionary_context, previous_context]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub fn manual_notes_for_generation(note: &NoteDto) -> Option<String> {
    let edited = note.edited_content.as_deref()?.trim();
    if edited.is_empty() {
        return None;
    }
    let Some(generated) = note.generated_content.as_deref().map(str::trim) else {
        return Some(edited.to_string());
    };
    if generated.is_empty() {
        return Some(edited.to_string());
    }
    if edited == generated {
        return None;
    }
    if let Some(rest) = edited.strip_prefix(generated) {
        let rest = rest.trim();
        return if rest.is_empty() {
            None
        } else {
            Some(rest.to_string())
        };
    }
    edited.find(generated).and_then(|index| {
        let before = edited[..index].trim();
        let after = edited[index + generated.len()..].trim();
        if !after.is_empty() {
            Some(after.to_string())
        } else if !before.is_empty() {
            Some(before.to_string())
        } else {
            None
        }
    })
}

/// The local day the recording started, for the note prompt's summary line.
/// A note can be created long before it is recorded, so the note's own
/// creation time is not used.
async fn note_recorded_on(repos: &Repositories, session_id: &str) -> Option<String> {
    repos
        .recording_session_started_at(session_id)
        .await
        .ok()
        .flatten()
        .and_then(|started| crate::providers::notes::recording_date(&started))
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) async fn process_saved_audio(
    repos: &Repositories,
    note_id: &str,
    session_id: &str,
    audio_artifact_id: &str,
    audio_path: PathBuf,
    title: String,
    existing_generated_note: Option<String>,
    manual_notes: Option<String>,
    recorded_silence: bool,
    timing: ProcessingTiming,
) -> Result<NoteDto, AppError> {
    let processing_started = Instant::now();
    let note_transcription_started = Instant::now();
    let timeline = FirstEventTimeline::new(timing);
    repos
        .set_note_status(note_id, ProcessingStatus::Transcribing, None)
        .await?;
    let temp_dir = session_temp_dir("clovy-transcription", session_id);
    let _ = std::fs::remove_dir_all(&temp_dir);
    if let Err(error) = std::fs::create_dir_all(&temp_dir) {
        let error = AppError::new("audio_normalize_failed", error.to_string());
        return Err(finalize_infrastructure_note_transcription_failure(
            repos,
            session_id,
            &timeline,
            timing,
            note_transcription_started,
            processing_started,
            error,
        )
        .await);
    }
    let _temp_dir_cleanup = TempDirCleanup(temp_dir.clone());
    let normalized_audio_path = match normalize_wav_for_transcription(
        &audio_path,
        &temp_dir.join(format!("{audio_artifact_id}-normalized.wav")),
    ) {
        Ok(path) => path,
        Err(error) => {
            return Err(finalize_infrastructure_note_transcription_failure(
                repos,
                session_id,
                &timeline,
                timing,
                note_transcription_started,
                processing_started,
                error,
            )
            .await);
        }
    };
    let transcription_provider = crate::providers::configured_transcription_provider();
    let dictionary_entries = match repos.list_dictionary_entries().await {
        Ok(entries) => entries,
        Err(error) => {
            return Err(finalize_infrastructure_note_transcription_failure(
                repos,
                session_id,
                &timeline,
                timing,
                note_transcription_started,
                processing_started,
                AppError::from(error),
            )
            .await);
        }
    };
    let dictionary = build_transcription_dictionary(&dictionary_entries);
    let transcriber = instrument_turn_transcriber(default_turn_transcriber(), timeline.clone());
    let transcription = match transcribe_prepared_audio(
        transcriber,
        TranscribePreparedAudioRequest {
            provider: transcription_provider.clone(),
            audio_path: normalized_audio_path,
            temp_dir: temp_dir.clone(),
            chunk_stem: audio_artifact_id.to_string(),
            title: title.clone(),
            asr_hint: dictionary
                .as_ref()
                .and_then(|dictionary| dictionary.asr_hint.clone()),
            operation_id: note_id.to_string(),
            max_chunk_ms: None,
        },
    )
    .await
    {
        Ok(transcription) => transcription,
        Err(error) => {
            timeline.flush(repos, session_id).await;
            add_latency_checkpoint(
                repos,
                session_id,
                "note_transcription_complete",
                timing.checkpoint_details(serde_json::json!({
                    "durationMs": elapsed_ms(note_transcription_started),
                    "status": "failed",
                    "successfulTurnCount": 0,
                    "failedTurnCount": 1,
                    "error": error.code,
                })),
            )
            .await;
            let user_message = user_facing_transcription_failure_message(
                &error.code,
                &error.message,
                recorded_silence,
                "microphone",
            );
            // A no-speech outcome is a coverage decision, not just a failure:
            // persist the zero-detected-speech checkpoint so the call is
            // documented the same way a chunked pass would document it.
            if is_no_speech_error(&error) {
                persist_microphone_only_transcript_coverage(repos, session_id, "microphone", &[])
                    .await;
            }
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            repos
                .set_note_status(
                    note_id,
                    ProcessingStatus::Failed,
                    Some(user_message.clone()),
                )
                .await?;
            return Err(AppError::new(&error.code, user_message));
        }
    };
    persist_microphone_only_transcript_coverage(
        repos,
        session_id,
        "microphone",
        &transcription.chunk_outcomes,
    )
    .await;
    let _ = std::fs::remove_dir_all(&temp_dir);
    let transcript = maybe_post_process_note_transcript(
        &transcription_provider,
        transcription.transcript,
        dictionary
            .as_ref()
            .map(|dictionary| dictionary.context.as_str()),
    )
    .await;
    let transcript_row = match repos
        .create_transcript(
            note_id,
            audio_artifact_id,
            &transcript.text,
            transcript.language.clone(),
            &transcript.provider,
        )
        .await
    {
        Ok(row) => row,
        Err(error) => {
            let error = AppError::from(error);
            timeline.flush(repos, session_id).await;
            add_infrastructure_note_transcription_failure_checkpoint(
                repos,
                session_id,
                timing,
                note_transcription_started,
                &error.code,
            )
            .await;
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            return Err(error);
        }
    };
    timeline.mark_first_persisted();
    timeline.flush(repos, session_id).await;
    add_latency_checkpoint(
        repos,
        session_id,
        "note_transcription_complete",
        timing.checkpoint_details(serde_json::json!({
            "durationMs": elapsed_ms(note_transcription_started),
            "status": "succeeded",
            "successfulTurnCount": 1,
            "failedTurnCount": 0,
        })),
    )
    .await;

    if let Err(error) = repos
        .set_note_status(note_id, ProcessingStatus::Generating, None)
        .await
    {
        let error = AppError::from(error);
        add_processing_complete_checkpoint(repos, session_id, timing, processing_started, "failed")
            .await;
        return Err(error);
    }
    let generation_started = Instant::now();
    let (generation_transcript, transcript_source_labels) =
        authoritative_generation_transcript(repos, note_id).await?;
    let generated = match generate_note_from_transcript(GenerationRequest {
        video: None,
        provider: crate::providers::generation_provider(),
        operation_id: Some(note_id.to_string()),
        title,
        existing_generated_note,
        transcript: generation_transcript,
        transcript_source_labels,
        manual_notes,
        language: transcript.language,
        recorded_on: note_recorded_on(repos, session_id).await,
    })
    .await
    {
        Ok(generated) => generated,
        Err(error) => {
            let error = note_generation_failure_for_user(error);
            add_latency_checkpoint(
                repos,
                session_id,
                "note_generation",
                timing.checkpoint_details(serde_json::json!({
                    "durationMs": elapsed_ms(generation_started),
                    "status": "failed",
                    "error": error.code,
                })),
            )
            .await;
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            repos
                .set_note_status(
                    note_id,
                    ProcessingStatus::Failed,
                    Some(error.message.clone()),
                )
                .await?;
            return Err(error);
        }
    };
    add_latency_checkpoint(
        repos,
        session_id,
        "note_generation",
        timing.checkpoint_details(serde_json::json!({
            "durationMs": elapsed_ms(generation_started),
            "status": "succeeded",
        })),
    )
    .await;
    let generation_result_id = match repos
        .create_generation_result_with_video_analysis(
            note_id,
            &transcript_row.id,
            &generated.content,
            generated.title_suggestion.clone(),
            &generated.provider,
            &generated.prompt_version,
            generated.video_analysis.as_deref(),
        )
        .await
    {
        Ok(id) => id,
        Err(error) => {
            let error = AppError::from(error);
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            return Err(error);
        }
    };
    let note = match repos
        .set_generated_note_for_session(
            note_id,
            Some(session_id),
            Some(&generation_result_id),
            generated.title_suggestion,
            generated.content,
        )
        .await
    {
        Ok(note) => note,
        Err(error) => {
            let error = AppError::from(error);
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            return Err(error);
        }
    };
    add_processing_complete_checkpoint(repos, session_id, timing, processing_started, "succeeded")
        .await;
    Ok(note)
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(crate) async fn process_saved_source_audio(
    repos: &Repositories,
    note_id: &str,
    session_id: &str,
    source_mode: RecordingSourceMode,
    sources: Vec<SourceAudioForProcessing>,
    title: String,
    existing_generated_note: Option<String>,
    manual_notes: Option<String>,
    timing: ProcessingTiming,
) -> Result<NoteDto, AppError> {
    let video_temp = tempfile::tempdir()
        .map_err(|error| AppError::new("storage_unavailable", error.to_string()))?;
    let paths = AppPaths::from_data_dir(video_temp.path().join("data"))
        .map_err(|error| AppError::new("storage_unavailable", error.to_string()))?;
    process_saved_source_audio_with_progress(
        repos,
        &paths,
        note_id,
        session_id,
        source_mode,
        sources,
        title,
        existing_generated_note,
        manual_notes,
        timing,
        NoteProcessingProgressReporter::default(),
    )
    .await
}

/// Video is optional even when its repository lookup or recovery bookkeeping fails.
/// Keep a failure input so successful transcript-only notes retain the notice.
pub(crate) async fn optional_note_video_input(
    repos: &Repositories,
    paths: &AppPaths,
    session_id: &str,
) -> Option<crate::providers::notes::NoteVideoInput> {
    match repos.note_video_input(session_id, paths).await {
        Ok(video) => video,
        Err(error) => {
            tracing::warn!(%session_id, error_code = %error.code, error_message = %error.message, "could not load meeting video input; generating from transcript");
            Some(crate::providers::notes::NoteVideoInput {
                path: String::new(),
                offset_ms: 0,
                app: String::new(),
                title: String::new(),
                failure_reason: Some("capture_failed".into()),
            })
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_saved_source_audio_with_progress(
    repos: &Repositories,
    paths: &AppPaths,
    note_id: &str,
    session_id: &str,
    source_mode: RecordingSourceMode,
    sources: Vec<SourceAudioForProcessing>,
    title: String,
    existing_generated_note: Option<String>,
    manual_notes: Option<String>,
    timing: ProcessingTiming,
    progress: NoteProcessingProgressReporter,
) -> Result<NoteDto, AppError> {
    process_saved_source_audio_with_cancellation(
        repos,
        paths,
        note_id,
        session_id,
        source_mode,
        sources,
        title,
        existing_generated_note,
        manual_notes,
        timing,
        progress,
        std::future::pending(),
    )
    .await
}

/// Cancellation is observed only after the initial status is persisted. Dropping
/// the pipeline stops further effects, while its owner still runs finalization.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_saved_source_audio_with_cancellation(
    repos: &Repositories,
    paths: &AppPaths,
    note_id: &str,
    session_id: &str,
    source_mode: RecordingSourceMode,
    sources: Vec<SourceAudioForProcessing>,
    title: String,
    existing_generated_note: Option<String>,
    manual_notes: Option<String>,
    timing: ProcessingTiming,
    progress: NoteProcessingProgressReporter,
    cancellation: impl Future<Output = AppError>,
) -> Result<NoteDto, AppError> {
    let progress = progress.scoped();
    let started = transition_note_processing_stage(
        repos,
        note_id,
        session_id,
        NoteProcessingStage::Transcribing,
        ProcessingStatus::Transcribing,
        &progress,
    )
    .await;
    let result = match started {
        Err(error) => Err(error),
        Ok(()) => {
            let outcome = {
                let pipeline = process_saved_source_audio_pipeline(
                    repos,
                    paths,
                    note_id,
                    session_id,
                    source_mode,
                    sources,
                    title,
                    existing_generated_note,
                    manual_notes,
                    timing,
                    progress.clone(),
                );
                tokio::pin!(pipeline);
                tokio::pin!(cancellation);
                tokio::select! {
                    biased;
                    error = &mut cancellation => Err(error),
                    result = &mut pipeline => Ok(result),
                }
            };
            match outcome {
                Ok(result) => result,
                Err(error) => {
                    // Generation can have committed its result before its final
                    // bookkeeping await. Never turn that completed result into failure.
                    match repos.get_note(note_id).await {
                        Ok(note) if note.processing_status == ProcessingStatus::Ready => Ok(note),
                        _ => Err(error),
                    }
                }
            }
        }
    };
    complete_note_processing_run(repos, &progress, note_id, session_id, result).await
}

#[allow(clippy::too_many_arguments)]
async fn process_saved_source_audio_pipeline(
    repos: &Repositories,
    paths: &AppPaths,
    note_id: &str,
    session_id: &str,
    source_mode: RecordingSourceMode,
    sources: Vec<SourceAudioForProcessing>,
    title: String,
    existing_generated_note: Option<String>,
    manual_notes: Option<String>,
    timing: ProcessingTiming,
    progress: NoteProcessingProgressReporter,
) -> Result<NoteDto, AppError> {
    let processing_started = Instant::now();
    let note_transcription_started = Instant::now();
    let timeline = FirstEventTimeline::new(timing);
    let transcription_provider = crate::providers::configured_transcription_provider();
    // Read once per run: every request of the run, its durable identity and
    // its kept chunks use this model.
    let transcription_model = crate::providers::transcription_model();
    let dictionary_entries = match repos.list_dictionary_entries().await {
        Ok(entries) => entries,
        Err(error) => {
            return Err(finalize_infrastructure_note_transcription_failure(
                repos,
                session_id,
                &timeline,
                timing,
                note_transcription_started,
                processing_started,
                AppError::from(error),
            )
            .await);
        }
    };
    let dictionary = build_transcription_dictionary(&dictionary_entries);
    let pipeline_setup = async {
        let detection_started = Instant::now();
        let SilentSystemDropOutcome {
            kept: sources,
            dropped,
        } = partition_silent_system_sources(sources);
        for drop in &dropped {
            if let Err(error) = repos
                .add_source_checkpoint(
                    session_id,
                    Some(drop.artifact_id.as_str()),
                    Some(drop.source.as_str()),
                    "silent_source_dropped",
                    Some(
                        serde_json::json!({
                            "source": drop.source,
                            "maxRms": drop.max_rms,
                            "activeMs": drop.active_ms,
                            "longestActiveMs": drop.longest_active_ms,
                        })
                        .to_string(),
                    ),
                )
                .await
            {
                tracing::warn!(
                    %session_id,
                    source = %drop.source,
                    %error,
                    "failed to persist silent_source_dropped checkpoint"
                );
            }
        }
        let (turns, echo_rejection) = if source_mode == RecordingSourceMode::MicrophoneOnly {
            // Microphone-only capture has no attribution problem to solve.
            // Keep it on the same durable pipeline, but represent the saved
            // WAV as one full-Source job instead of paying the dual-Source
            // VAD and echo-rejection cost.
            (
                add_full_source_turns_for_missing_sources(
                    &sources,
                    Vec::new(),
                    &EchoRejectionReport::default(),
                ),
                EchoRejectionReport::default(),
            )
        } else {
            let detection_sources = sources
                .iter()
                .map(
                    |(artifact_id, source, audio_path, _recorded_silence)| DetectionSource {
                        artifact_id: artifact_id.clone(),
                        source: source.clone(),
                        path: audio_path.clone(),
                    },
                )
                .collect::<Vec<_>>();
            // Detection carries real DSP (GCC-PHAT probes and adaptive
            // cancellation over full recordings), so it must not pin an
            // async runtime worker.
            tokio::task::spawn_blocking(move || detect_turns_with_report(&detection_sources))
                .await
                .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))??
        };
        // Echo rejection silently rewrites the microphone timeline; leave a
        // trace (which lag, how much trimmed) so missing-speech reports can be
        // diagnosed, mirroring the silent_source_dropped checkpoint.
        if echo_rejection.attempted() {
            if let Err(error) = repos
                .add_checkpoint(
                    session_id,
                    "echo_rejection",
                    Some(
                        serde_json::json!({
                            "pairLagsMs": echo_rejection.pair_lags_ms,
                            "trimmedTurnCount": echo_rejection.trimmed_turn_count,
                            "trimmedMs": echo_rejection.trimmed_ms,
                            "droppedTurnCount": echo_rejection.dropped_turn_count,
                            "durationMs": elapsed_ms(detection_started),
                        })
                        .to_string(),
                    ),
                )
                .await
            {
                tracing::warn!(%session_id, %error, "failed to persist echo_rejection checkpoint");
            }
        }
        *progress
            .discarded_spans
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = echo_rejection
            .discarded_microphone_spans
            .iter()
            .map(|&(start_ms, end_ms)| DiscardedTranscriptSpanDto {
                session_id: session_id.to_string(),
                source: RecordingSource::Microphone,
                start_ms,
                end_ms,
            })
            .collect();
        let turns = add_full_source_turns_for_missing_sources(&sources, turns, &echo_rejection);
        let done_span_ids = repos
            .certified_source_turn_transcripts_for_session(session_id)
            .await?
            .into_iter()
            .filter_map(|transcript| transcript.span_id)
            .collect::<HashSet<_>>();
        let turns = coalesce_turns_reusing_layout(
            turns,
            &echo_rejection.microphone_echo_spans,
            session_id,
            &done_span_ids,
        );
        // Coverage is measured against the post-trim turn list on purpose:
        // speaker bleed removed by echo rejection is not lost speech.
        let coverage_turns = turns.clone();
        if let Err(error) = repos
            .add_checkpoint(
                session_id,
                "turn_detection",
                Some(
                    serde_json::json!({
                        "durationMs": elapsed_ms(detection_started),
                        "sourceCount": sources.len(),
                        "turnCount": turns.len(),
                    })
                    .to_string(),
                ),
            )
            .await
        {
            tracing::warn!(%session_id, %error, "failed to persist turn_detection checkpoint");
        }

        let turn_wav_dir = session_temp_dir("clovy-turns", session_id);
        let _ = std::fs::remove_dir_all(&turn_wav_dir);
        std::fs::create_dir_all(&turn_wav_dir)
            .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
        let turn_wav_dir_cleanup = Arc::new(TempDirCleanup(turn_wav_dir.clone()));

        let mut all_preparation_jobs = Vec::new();
        for turn in turns {
            let turn_wav_path = turn_wav_dir.join(format!(
                "{:04}-{}-{}-{}.wav",
                turn.turn_index, turn.source, turn.start_ms, turn.end_ms
            ));
            let normalized_path = turn_wav_dir.join(format!(
                "{:04}-{}-{}-{}-normalized.wav",
                turn.turn_index, turn.source, turn.start_ms, turn.end_ms
            ));
            let recorded_silence = source_recorded_silence(&sources, &turn.artifact_id);
            let echo_trimmed = echo_rejection
                .trimmed_artifact_ids
                .contains(&turn.artifact_id);
            let descriptor = TurnPreparationJob {
                schedule_index: all_preparation_jobs.len(),
                turn: turn.clone(),
                temp_dir: turn_wav_dir.clone(),
                turn_wav_path,
                normalized_path,
                recorded_silence,
                echo_trimmed,
                durable_job: None,
            };
            all_preparation_jobs.push(descriptor.clone());
        }
        let configuration_fingerprint = note_transcription_configuration_fingerprint(
            &title,
            dictionary
                .as_ref()
                .map(|dictionary| dictionary.context.as_str()),
            &transcription_model,
        );
        let mut fallback_plans = build_source_fallback_plans(&all_preparation_jobs)
            .into_iter()
            .filter(SourceFallbackPlan::eligible)
            .collect::<Vec<_>>();
        for fallback in &mut fallback_plans {
            fallback.end_ms = source_wav_duration_ms(&fallback.source_path)
                .unwrap_or(fallback.end_ms)
                .max(fallback.end_ms);
        }

        let mut job_plans = all_preparation_jobs
            .iter()
            .map(|descriptor| {
                let end_ms = if descriptor.covers_full_source() {
                    source_wav_duration_ms(&descriptor.turn.source_path)
                        .unwrap_or(descriptor.turn.end_ms)
                } else {
                    descriptor.turn.end_ms
                };
                NoteTranscriptionJobPlan {
                    span_id: note_transcription_span_id(
                        session_id,
                        &descriptor.turn.source,
                        NoteTranscriptionJobKind::Turn,
                        descriptor.turn.start_ms,
                        end_ms,
                    ),
                    audio_artifact_id: descriptor.turn.artifact_id.clone(),
                    source: descriptor.turn.source.clone(),
                    job_kind: NoteTranscriptionJobKind::Turn,
                    start_ms: descriptor.turn.start_ms,
                    end_ms,
                    turn_index: descriptor.turn.turn_index,
                    provider: transcription_provider.clone(),
                    max_chunk_ms: (descriptor.covers_full_source()
                        && is_short_full_source(
                            &descriptor.turn.source_path,
                            descriptor.turn.start_ms,
                            descriptor.turn.end_ms,
                        ))
                    .then_some(PREVIEW_CHUNK_MS),
                    pipeline_version: NOTE_TRANSCRIPTION_PIPELINE_VERSION.to_string(),
                    configuration_fingerprint: configuration_fingerprint.clone(),
                }
            })
            .collect::<Vec<_>>();
        job_plans.extend(
            fallback_plans
                .iter()
                .map(|fallback| NoteTranscriptionJobPlan {
                    span_id: note_transcription_span_id(
                        session_id,
                        &fallback.source,
                        NoteTranscriptionJobKind::SourceFallback,
                        0,
                        fallback.end_ms,
                    ),
                    audio_artifact_id: fallback.artifact_id.clone(),
                    source: fallback.source.clone(),
                    job_kind: NoteTranscriptionJobKind::SourceFallback,
                    start_ms: 0,
                    end_ms: fallback.end_ms,
                    turn_index: fallback.turn_index,
                    provider: transcription_provider.clone(),
                    max_chunk_ms: None,
                    pipeline_version: NOTE_TRANSCRIPTION_PIPELINE_VERSION.to_string(),
                    configuration_fingerprint: configuration_fingerprint.clone(),
                }),
        );

        let durable_jobs = repos
            .reconcile_note_transcription_jobs(note_id, session_id, source_mode, &job_plans)
            .await?;
        let jobs_by_id = durable_jobs
            .iter()
            .cloned()
            .map(|job| (job.id.clone(), job))
            .collect::<HashMap<_, _>>();
        let existing_by_span = repos
            .certified_source_turn_transcripts_for_session(session_id)
            .await?
            .into_iter()
            .filter_map(|transcript| Some((transcript.span_id.clone()?, transcript)))
            .collect::<HashMap<_, _>>();
        let succeeded_fallback_sources = durable_jobs
            .iter()
            .filter(|job| {
                job.job_kind == NoteTranscriptionJobKind::SourceFallback
                    && job.status == NoteTranscriptionJobStatus::Succeeded
            })
            .map(|job| job.source.clone())
            .collect::<HashSet<_>>();
        let cached_candidates = durable_jobs
            .iter()
            .filter(|job| job.status == NoteTranscriptionJobStatus::Succeeded)
            .filter_map(|job| {
                let transcript = existing_by_span.get(&job.id)?;
                Some(TranscriptCandidate {
                    artifact_id: job.audio_artifact_id.clone(),
                    language: transcript.language.clone(),
                    input: SourceTranscriptInput {
                        source: job.source.clone(),
                        text: transcript.text.clone(),
                        valid: !transcript.text.trim().is_empty(),
                        warning: None,
                        recorded_silence: transcript.recorded_silence,
                        start_ms: Some(job.start_ms),
                        end_ms: Some(job.end_ms),
                        turn_index: Some(job.turn_index),
                    },
                })
            })
            .collect::<Vec<_>>();

        let mut preparation_jobs = Vec::new();
        for mut descriptor in all_preparation_jobs {
            if succeeded_fallback_sources.contains(&descriptor.turn.source) {
                continue;
            }
            let end_ms = if descriptor.covers_full_source() {
                source_wav_duration_ms(&descriptor.turn.source_path)
                    .unwrap_or(descriptor.turn.end_ms)
            } else {
                descriptor.turn.end_ms
            };
            let id = note_transcription_span_id(
                session_id,
                &descriptor.turn.source,
                NoteTranscriptionJobKind::Turn,
                descriptor.turn.start_ms,
                end_ms,
            );
            let Some(job) = jobs_by_id.get(&id) else {
                return Err(AppError::new(
                    "transcription_job_missing",
                    format!("Durable transcription job {id} was not reconciled."),
                ));
            };
            match job.status {
                NoteTranscriptionJobStatus::Pending => {
                    descriptor.durable_job = Some(job.clone());
                    preparation_jobs.push(descriptor);
                }
                NoteTranscriptionJobStatus::Succeeded | NoteTranscriptionJobStatus::Superseded => {}
                NoteTranscriptionJobStatus::Running => {
                    return Err(AppError::new(
                        "transcription_already_running",
                        "This saved recording is already being transcribed.",
                    ));
                }
                NoteTranscriptionJobStatus::Failed => {
                    return Err(AppError::new(
                        "transcription_job_invalid_state",
                        "A failed transcription job was not reset for retry.",
                    ));
                }
            }
        }
        for fallback in &mut fallback_plans {
            let id = note_transcription_span_id(
                session_id,
                &fallback.source,
                NoteTranscriptionJobKind::SourceFallback,
                0,
                fallback.end_ms,
            );
            if let Some(job) = jobs_by_id.get(&id) {
                if job.status == NoteTranscriptionJobStatus::Pending {
                    fallback.durable_job = Some(job.clone());
                }
            }
        }
        fallback_plans.retain(|fallback| fallback.durable_job.is_some());
        let preparation_jobs = interleave_turn_preparation_jobs_by_source(preparation_jobs);

        Ok::<_, AppError>((
            sources,
            coverage_turns,
            turn_wav_dir_cleanup,
            preparation_jobs,
            fallback_plans,
            cached_candidates,
        ))
    }
    .await;
    let (
        sources,
        coverage_turns,
        turn_wav_dir_cleanup,
        preparation_jobs,
        fallback_plans,
        cached_candidates,
    ) = match pipeline_setup {
        Ok(prepared) => prepared,
        Err(error) => {
            timeline.flush(repos, session_id).await;
            add_infrastructure_note_transcription_failure_checkpoint(
                repos,
                session_id,
                timing,
                note_transcription_started,
                &error.code,
            )
            .await;
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            return Err(error);
        }
    };

    let persist_repos = repos.clone();
    let persist_note_id = note_id.to_string();
    let persist_session_id = session_id.to_string();
    let persist_timeline = timeline.clone();
    let result_sink: TurnResultSink = Arc::new(move |event| {
        let repos = persist_repos.clone();
        let note_id = persist_note_id.clone();
        let session_id = persist_session_id.clone();
        let timeline = persist_timeline.clone();
        Box::pin(async move {
            persist_turn_transcription_event(&repos, &note_id, &session_id, event, timeline).await
        })
    });
    let claim_repos = repos.clone();
    let job_claimer: TurnJobClaimer = Arc::new(move |job_id| {
        let repos = claim_repos.clone();
        Box::pin(async move {
            match repos.claim_note_transcription_job(&job_id).await {
                Ok(true) => Ok(()),
                Ok(false) => Err(AppError::new(
                    "transcription_already_running",
                    "This saved-audio span is already being transcribed.",
                )),
                Err(error) => Err(AppError::from(error)),
            }
        })
    });

    let reused_transcript_count = cached_candidates.len();
    let mut transcription_outcome = TranscriptionOutcome {
        candidates: cached_candidates,
        failures: Vec::new(),
        replaced_sources: HashSet::new(),
    };
    let transcriber = retain_cleanup_during_note_transcription(
        instrument_turn_transcriber(
            note_turn_transcriber(repos.clone(), transcription_model.clone()),
            timeline.clone(),
        ),
        Arc::clone(&turn_wav_dir_cleanup),
    );
    let pipeline_result = prepare_and_transcribe_turn_jobs_with_packs(
        preparation_jobs,
        fallback_plans,
        &transcription_outcome.candidates,
        transcription_provider.clone(),
        title.clone(),
        dictionary,
        guarded_turn_preparer(Arc::clone(&turn_wav_dir_cleanup)),
        guarded_fallback_preparer(Arc::clone(&turn_wav_dir_cleanup)),
        transcriber,
        note_pack_transcriber(transcription_model.clone()),
        Some(job_claimer),
        Some(result_sink),
        DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        timing,
    )
    .await;
    let pipeline = persist_turn_pipeline_result(
        repos,
        session_id,
        pipeline_result,
        reused_transcript_count,
        &timeline,
        timing,
        note_transcription_started,
        processing_started,
    )
    .await?;
    repos
        .supersede_pending_note_transcription_fallbacks(session_id)
        .await?;
    drop(turn_wav_dir_cleanup);

    let mut fresh_outcome = pipeline.outcome;
    for source in &fresh_outcome.replaced_sources {
        transcription_outcome
            .candidates
            .retain(|candidate| candidate.input.source != *source);
        transcription_outcome
            .failures
            .retain(|failure| failure.input.source != *source);
    }
    transcription_outcome
        .candidates
        .append(&mut fresh_outcome.candidates);
    transcription_outcome
        .failures
        .append(&mut fresh_outcome.failures);
    transcription_outcome
        .replaced_sources
        .extend(fresh_outcome.replaced_sources);

    let has_valid_transcript = !transcription_outcome.candidates.is_empty();
    let visible_failures =
        visible_transcription_failures(&transcription_outcome.failures, has_valid_transcript);
    // `visible_failures` is already filtered by `should_record_source_failure`,
    // so every entry here is one we record diagnostically.
    for failure in &visible_failures {
        // The durable job already stores this failure. Do not project an empty
        // failed transcript at the same presentation index: that conflict
        // would overwrite a ledger-certified last-known-good row before a
        // replacement succeeds.
        let persistence_started = Instant::now();
        if let Err(error) = repos
            .add_source_checkpoint(
                session_id,
                Some(failure.artifact_id.as_str()),
                Some(failure.input.source.as_str()),
                "transcript_persistence",
                Some(
                    serde_json::json!({
                        "durationMs": elapsed_ms(persistence_started),
                        "status": "failed",
                        "turnIndex": failure.input.turn_index,
                    })
                    .to_string(),
                ),
            )
            .await
        {
            tracing::warn!(
                %session_id,
                %error,
                "failed to persist failed-transcript checkpoint"
            );
        }
    }

    let persisted_transcripts = match repos
        .certified_source_turn_transcripts_for_session(session_id)
        .await
    {
        Ok(transcripts) => transcripts,
        Err(error) => {
            let error = AppError::from(error);
            timeline.flush(repos, session_id).await;
            add_infrastructure_note_transcription_failure_checkpoint(
                repos,
                session_id,
                timing,
                note_transcription_started,
                &error.code,
            )
            .await;
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            return Err(error);
        }
    };
    let transcript_coverage = compute_transcript_coverage(
        &sources,
        &coverage_turns,
        &persisted_transcripts,
        &visible_failures,
    );
    // Coverage is diagnostic only and must never fail note processing: a
    // checkpoint that cannot be serialized or persisted is logged and skipped.
    match serde_json::to_string(&transcript_coverage) {
        Ok(payload) => {
            if let Err(error) = repos
                .add_checkpoint(session_id, "transcript_coverage", Some(payload))
                .await
            {
                tracing::warn!(
                    session_id,
                    %error,
                    "failed to persist transcript_coverage checkpoint"
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                session_id,
                %error,
                "failed to serialize transcript_coverage checkpoint"
            );
        }
    }
    let first_transcript_id = persisted_transcripts
        .first()
        .map(|transcript| transcript.id.clone());
    let transcript_inputs = persisted_transcripts
        .iter()
        .map(source_transcript_input_from_row)
        .collect::<Vec<_>>();
    let valid_sources = valid_sources_for_processing(transcript_inputs);
    let blocking_error = if valid_sources.is_empty() {
        let failure_message = source_failure_summary(&transcription_outcome.failures)
            .unwrap_or_else(|| "No selected source produced a usable transcript.".to_string());
        Some(AppError::new("transcription_failed", failure_message))
    } else {
        blocking_transcription_failure_summary(&visible_failures)
            .map(|failure_message| AppError::new("transcription_partially_failed", failure_message))
    }
    .map(|mut error| {
        // Durable turn failures retain the canonical message, not the provider code.
        // Preserve Local's actionable code when aggregating those saved failures.
        if visible_failures.iter().any(|failure| {
            failure.input.warning.as_deref()
                == Some(crate::providers::local_transcriber::LOCAL_TRANSCRIBER_FAILED_MESSAGE)
        }) {
            error.code = "local_transcriber_failed".into();
        } else if visible_failures.iter().any(|failure| {
            failure.input.warning.as_deref() == Some(TRANSCRIPTION_ENGINE_CHANGED_MESSAGE)
        }) {
            error.code = "transcription_engine_changed".into();
        }
        error
    });
    timeline.flush(repos, session_id).await;
    add_latency_checkpoint(
        repos,
        session_id,
        "note_transcription_complete",
        timing.checkpoint_details(serde_json::json!({
            "durationMs": elapsed_ms(note_transcription_started),
            "status": if blocking_error.is_some() { "failed" } else { "succeeded" },
            "successfulTurnCount": persisted_transcripts.len(),
            "failedTurnCount": visible_failures.len(),
        })),
    )
    .await;
    if let Some(error) = blocking_error {
        add_processing_complete_checkpoint(repos, session_id, timing, processing_started, "failed")
            .await;
        repos
            .set_note_status(
                note_id,
                ProcessingStatus::Failed,
                Some(error.message.clone()),
            )
            .await?;
        return Err(error);
    }
    let (generation_transcript, transcript_source_labels) =
        authoritative_generation_transcript(repos, note_id).await?;
    if let Err(error) = transition_note_processing_stage(
        repos,
        note_id,
        session_id,
        NoteProcessingStage::Generating,
        ProcessingStatus::Generating,
        &progress,
    )
    .await
    {
        add_processing_complete_checkpoint(repos, session_id, timing, processing_started, "failed")
            .await;
        return Err(error);
    }
    let generation_started = Instant::now();
    let generated = match generate_note_from_transcript(GenerationRequest {
        video: optional_note_video_input(repos, paths, session_id).await,
        provider: crate::providers::generation_provider(),
        operation_id: Some(note_id.to_string()),
        title,
        existing_generated_note,
        transcript: generation_transcript,
        transcript_source_labels,
        manual_notes,
        language: None,
        recorded_on: note_recorded_on(repos, session_id).await,
    })
    .await
    {
        Ok(generated) => generated,
        Err(error) => {
            let error = note_generation_failure_for_user(error);
            add_latency_checkpoint(
                repos,
                session_id,
                "note_generation",
                timing.checkpoint_details(serde_json::json!({
                    "durationMs": elapsed_ms(generation_started),
                    "status": "failed",
                    "error": error.code,
                })),
            )
            .await;
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            repos
                .set_note_status(
                    note_id,
                    ProcessingStatus::Failed,
                    Some(error.message.clone()),
                )
                .await?;
            return Err(error);
        }
    };
    add_latency_checkpoint(
        repos,
        session_id,
        "note_generation",
        timing.checkpoint_details(serde_json::json!({
            "durationMs": elapsed_ms(generation_started),
            "status": "succeeded",
            "transcriptCount": valid_sources.len(),
        })),
    )
    .await;
    let transcript_id = first_transcript_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let generation_result_id = match repos
        .create_generation_result_with_video_analysis(
            note_id,
            &transcript_id,
            &generated.content,
            generated.title_suggestion.clone(),
            &generated.provider,
            &generated.prompt_version,
            generated.video_analysis.as_deref(),
        )
        .await
    {
        Ok(id) => id,
        Err(error) => {
            let error = AppError::from(error);
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            return Err(error);
        }
    };
    let note = match repos
        .set_generated_note_for_session(
            note_id,
            Some(session_id),
            Some(&generation_result_id),
            generated.title_suggestion,
            generated.content,
        )
        .await
    {
        Ok(note) => note,
        Err(error) => {
            let error = AppError::from(error);
            add_processing_complete_checkpoint(
                repos,
                session_id,
                timing,
                processing_started,
                "failed",
            )
            .await;
            return Err(error);
        }
    };
    #[cfg(test)]
    if TEST_GENERATION_COMMITTED
        .try_with(|sender| sender.send(()))
        .is_ok()
    {
        tokio::task::yield_now().await;
    }
    add_processing_complete_checkpoint(repos, session_id, timing, processing_started, "succeeded")
        .await;
    Ok(note)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptCoverageCheckpoint {
    sources: Vec<TranscriptCoverageSource>,
    total_detected_speech_ms: i64,
    total_transcribed_ms: i64,
    total_detected_turns: i64,
    total_transcribed_turns: i64,
    total_failed_turns: i64,
    warning: bool,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct TranscriptCoverageSource {
    source: String,
    detected_speech_ms: i64,
    transcribed_ms: i64,
    detected_turns: i64,
    transcribed_turns: i64,
    failed_turns: i64,
}

#[cfg(test)]
fn compute_chunk_transcript_coverage(
    source: &str,
    chunk_outcomes: &[TranscriptionChunkOutcome],
) -> TranscriptCoverageCheckpoint {
    let detected_speech_ms = chunk_outcomes
        .iter()
        .filter(|chunk| !chunk.locally_silent)
        .map(|chunk| span_ms(chunk.start_ms, chunk.end_ms))
        .sum();
    let transcribed_ms = chunk_outcomes
        .iter()
        .filter(|chunk| !chunk.locally_silent && chunk.provider_returned_text)
        .map(|chunk| span_ms(chunk.start_ms, chunk.end_ms))
        .sum();
    let detected_turns = chunk_outcomes
        .iter()
        .filter(|chunk| !chunk.locally_silent)
        .count() as i64;
    let transcribed_turns = chunk_outcomes
        .iter()
        .filter(|chunk| !chunk.locally_silent && chunk.provider_returned_text)
        .count() as i64;
    let failed_turns = chunk_outcomes
        .iter()
        .filter(|chunk| !chunk.locally_silent && !chunk.provider_returned_text)
        .count() as i64;
    let warning = transcript_coverage_warning(detected_speech_ms, transcribed_ms);
    TranscriptCoverageCheckpoint {
        sources: vec![TranscriptCoverageSource {
            source: source.to_string(),
            detected_speech_ms,
            transcribed_ms,
            detected_turns,
            transcribed_turns,
            failed_turns,
        }],
        total_detected_speech_ms: detected_speech_ms,
        total_transcribed_ms: transcribed_ms,
        total_detected_turns: detected_turns,
        total_transcribed_turns: transcribed_turns,
        total_failed_turns: failed_turns,
        warning,
    }
}

// Coverage is diagnostic only and must never fail note processing: a
// checkpoint that cannot be serialized or persisted is logged and skipped.
#[cfg(test)]
async fn persist_microphone_only_transcript_coverage(
    repos: &Repositories,
    session_id: &str,
    source: &str,
    chunk_outcomes: &[TranscriptionChunkOutcome],
) {
    let coverage = compute_chunk_transcript_coverage(source, chunk_outcomes);
    match serde_json::to_string(&coverage) {
        Ok(payload) => {
            if let Err(error) = repos
                .add_checkpoint(session_id, "transcript_coverage", Some(payload))
                .await
            {
                tracing::warn!(
                    error = %error,
                    session_id = %session_id,
                    "failed to persist transcript_coverage checkpoint"
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                error = %error,
                session_id = %session_id,
                "failed to serialize transcript_coverage checkpoint"
            );
        }
    }
}

fn compute_transcript_coverage(
    sources: &[SourceAudioForProcessing],
    detected_turns: &[AudioTurn],
    persisted_transcripts: &[TranscriptDto],
    failed_transcripts: &[FailedTranscriptCandidate],
) -> TranscriptCoverageCheckpoint {
    let mut source_entries = sources
        .iter()
        .map(|(_artifact_id, source, source_path, _recorded_silence)| {
            let source_detected_turns = detected_turns
                .iter()
                .filter(|turn| turn.source == *source)
                .collect::<Vec<_>>();
            let source_transcripts = persisted_transcripts
                .iter()
                .filter(|transcript| transcript.source.as_deref() == Some(source.as_str()))
                .collect::<Vec<_>>();
            let failed_turns = failed_transcripts
                .iter()
                .filter(|failure| failure.input.source == *source)
                .count() as i64;
            let detected_sentinel = source_detected_turns
                .iter()
                .any(|turn| covers_full_source(turn.start_ms, turn.end_ms));
            let transcribed_sentinel = source_transcripts.iter().any(|transcript| {
                transcript
                    .start_ms
                    .zip(transcript.end_ms)
                    .is_some_and(|(start_ms, end_ms)| covers_full_source(start_ms, end_ms))
            });
            // A no-speech outcome on a full-source sentinel means both the
            // energy detector and the ASR agreed the source was silent (e.g.
            // a muted microphone): silence must never count as missing
            // speech, whether the failure row is visible or suppressed. Only
            // real (non-no-speech) failures count the WAV as uncovered.
            let non_silent_failed_turns = failed_transcripts
                .iter()
                .filter(|failure| failure.input.source == *source)
                .filter(|failure| {
                    !failure
                        .input
                        .warning
                        .as_deref()
                        .map(is_no_speech_message)
                        .unwrap_or(false)
                })
                .count() as i64;
            let detected_speech_ms = if detected_sentinel {
                if transcribed_sentinel {
                    0
                } else if non_silent_failed_turns > 0 {
                    source_wav_duration_ms(source_path).unwrap_or_default()
                } else {
                    0
                }
            } else {
                source_detected_turns
                    .iter()
                    .map(|turn| span_ms(turn.start_ms, turn.end_ms))
                    .sum()
            };
            let transcribed_ms = if transcribed_sentinel {
                if detected_sentinel {
                    detected_speech_ms
                } else {
                    source_detected_turns
                        .iter()
                        .map(|turn| span_ms(turn.start_ms, turn.end_ms))
                        .sum()
                }
            } else {
                source_transcripts
                    .iter()
                    .map(|transcript| {
                        span_ms(
                            transcript.start_ms.unwrap_or_default(),
                            transcript.end_ms.unwrap_or_default(),
                        )
                    })
                    .sum()
            };
            TranscriptCoverageSource {
                source: source.clone(),
                detected_speech_ms,
                transcribed_ms,
                detected_turns: source_detected_turns.len() as i64,
                transcribed_turns: source_transcripts.len() as i64,
                failed_turns,
            }
        })
        .collect::<Vec<_>>();

    source_entries.sort_by(|left, right| left.source.cmp(&right.source));
    let total_detected_speech_ms = source_entries
        .iter()
        .map(|source| source.detected_speech_ms)
        .sum();
    let total_transcribed_ms = source_entries
        .iter()
        .map(|source| source.transcribed_ms)
        .sum();
    let total_detected_turns = source_entries
        .iter()
        .map(|source| source.detected_turns)
        .sum();
    let total_transcribed_turns = source_entries
        .iter()
        .map(|source| source.transcribed_turns)
        .sum();
    let total_failed_turns = source_entries
        .iter()
        .map(|source| source.failed_turns)
        .sum();
    let warning = transcript_coverage_warning(total_detected_speech_ms, total_transcribed_ms);
    TranscriptCoverageCheckpoint {
        sources: source_entries,
        total_detected_speech_ms,
        total_transcribed_ms,
        total_detected_turns,
        total_transcribed_turns,
        total_failed_turns,
        warning,
    }
}

pub(crate) fn transcript_coverage_warning(detected_speech_ms: i64, transcribed_ms: i64) -> bool {
    let detected_speech_ms = detected_speech_ms.max(0);
    let transcribed_ms = transcribed_ms.max(0);
    detected_speech_ms > 0
        && (transcribed_ms as f64) < TRANSCRIPT_COVERAGE_WARN_RATIO * (detected_speech_ms as f64)
        && detected_speech_ms.saturating_sub(transcribed_ms)
            >= TRANSCRIPT_COVERAGE_WARN_MIN_MISSING_MS
}

fn covers_full_source(start_ms: i64, end_ms: i64) -> bool {
    end_ms <= start_ms
}

fn span_ms(start_ms: i64, end_ms: i64) -> i64 {
    end_ms.saturating_sub(start_ms).max(0)
}

fn source_wav_duration_ms(path: &Path) -> Option<i64> {
    let reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    let sample_rate = spec.sample_rate.max(1) as i64;
    Some(((reader.duration() as i64) * 1000) / sample_rate)
}

fn is_short_full_source(audio_path: &Path, start_ms: i64, end_ms: i64) -> bool {
    covers_full_source(start_ms, end_ms)
        && source_wav_duration_ms(audio_path)
            .is_some_and(|duration_ms| duration_ms <= SHORT_FULL_SOURCE_RECOVERY_MAX_MS)
}

fn short_full_source_chunk_ms(job: &TurnTranscriptionJob) -> Option<i64> {
    (job.covers_full_source
        && !job.source_fallback
        && is_short_full_source(&job.audio_path, job.start_ms, job.end_ms))
    .then_some(PREVIEW_CHUNK_MS)
}

#[derive(Debug, Clone)]
struct TranscriptCandidate {
    artifact_id: String,
    language: Option<String>,
    input: SourceTranscriptInput,
}

#[derive(Debug, Clone)]
struct FailedTranscriptCandidate {
    artifact_id: String,
    input: SourceTranscriptInput,
}

#[derive(Debug, Clone, Default)]
struct TranscriptionOutcome {
    candidates: Vec<TranscriptCandidate>,
    failures: Vec<FailedTranscriptCandidate>,
    replaced_sources: HashSet<String>,
}

#[derive(Debug, Clone)]
struct CompletedTurnTranscription {
    job_id: Option<String>,
    result: TurnTranscriptionResult,
    duration_ms: i64,
    replaces_source: bool,
    /// A rate-limit pause that failed this turn: the run stops after
    /// recording it, so the turns still waiting stay pending.
    pause: Option<AppError>,
}

#[derive(Debug, Clone)]
enum TurnTranscriptionResult {
    Candidate(TranscriptCandidate),
    Failure(FailedTranscriptCandidate),
}

#[derive(Debug, Clone)]
struct TurnPreparationJob {
    schedule_index: usize,
    turn: AudioTurn,
    temp_dir: PathBuf,
    turn_wav_path: PathBuf,
    normalized_path: PathBuf,
    recorded_silence: bool,
    echo_trimmed: bool,
    durable_job: Option<NoteTranscriptionJobRecord>,
}

impl TurnPreparationJob {
    fn covers_full_source(&self) -> bool {
        covers_full_source(self.turn.start_ms, self.turn.end_ms)
    }
}

#[derive(Debug)]
struct PreparedTurn {
    schedule_index: usize,
    job: TurnTranscriptionJob,
}

#[derive(Debug)]
struct TurnPreparationReport {
    prepared_count: usize,
    active_preparation_duration_ms: i64,
    producer_wall_duration_ms: i64,
    done_to_preparation_complete_ms: Option<i64>,
    error: Option<AppError>,
}

#[derive(Debug)]
struct TurnPipelineResult {
    outcome: TranscriptionOutcome,
    preparation: TurnPreparationReport,
}

#[derive(Debug)]
struct TurnPipelineFailure {
    error: AppError,
    preparation: Option<Box<TurnPreparationReport>>,
}

#[derive(Debug, Clone)]
struct SourceFallbackPlan {
    artifact_id: String,
    source: String,
    source_path: PathBuf,
    normalized_path: PathBuf,
    temp_dir: PathBuf,
    recorded_silence: bool,
    all_turns_cover_full_source: bool,
    echo_trimmed: bool,
    end_ms: i64,
    turn_index: i64,
    detected_ms: i64,
    durable_job: Option<NoteTranscriptionJobRecord>,
}

impl SourceFallbackPlan {
    fn eligible(&self) -> bool {
        !self.all_turns_cover_full_source && !self.echo_trimmed
    }
}

#[derive(Debug, Clone)]
struct TurnTranscriptionJob {
    artifact_id: String,
    source: String,
    audio_path: PathBuf,
    temp_dir: PathBuf,
    recorded_silence: bool,
    covers_full_source: bool,
    source_fallback: bool,
    start_ms: i64,
    end_ms: i64,
    turn_index: i64,
    durable_job: Option<NoteTranscriptionJobRecord>,
}

type TranscriptionFuture =
    Pin<Box<dyn Future<Output = Result<TranscriptionProviderResult, AppError>> + Send>>;
type TurnPreparer = Arc<dyn Fn(TurnPreparationJob) -> Result<PreparedTurn, AppError> + Send + Sync>;
type SourceFallbackPreparer =
    Arc<dyn Fn(SourceFallbackPlan) -> Result<TurnTranscriptionJob, AppError> + Send + Sync>;
type TurnTranscriber = Arc<dyn Fn(TranscriptionRequest) -> TranscriptionFuture + Send + Sync>;
type TurnClaimFuture = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send>>;
type TurnJobClaimer = Arc<dyn Fn(String) -> TurnClaimFuture + Send + Sync>;
type TurnResultFuture = Pin<Box<dyn Future<Output = Result<(), AppError>> + Send>>;
type TurnResultSink = Arc<dyn Fn(CompletedTurnTranscription) -> TurnResultFuture + Send + Sync>;

fn retain_cleanup_during_blocking_work<Input: 'static, Output: 'static>(
    inner: Arc<dyn Fn(Input) -> Result<Output, AppError> + Send + Sync>,
    cleanup: Arc<TempDirCleanup>,
) -> Arc<dyn Fn(Input) -> Result<Output, AppError> + Send + Sync> {
    Arc::new(move |input| {
        let _cleanup = Arc::clone(&cleanup);
        inner(input)
    })
}

fn retain_cleanup_during_turn_preparation(
    inner: TurnPreparer,
    cleanup: Arc<TempDirCleanup>,
) -> TurnPreparer {
    retain_cleanup_during_blocking_work(inner, cleanup)
}

fn guarded_turn_preparer(cleanup: Arc<TempDirCleanup>) -> TurnPreparer {
    let inner: TurnPreparer = Arc::new(prepare_turn_job);
    retain_cleanup_during_turn_preparation(inner, cleanup)
}

fn guarded_fallback_preparer(cleanup: Arc<TempDirCleanup>) -> SourceFallbackPreparer {
    let inner: SourceFallbackPreparer = Arc::new(prepare_source_fallback);
    retain_cleanup_during_blocking_work(inner, cleanup)
}

fn retain_cleanup_during_note_transcription(
    inner: TurnTranscriber,
    cleanup: Arc<TempDirCleanup>,
) -> TurnTranscriber {
    Arc::new(move |request| {
        let cleanup = Arc::clone(&cleanup);
        let future = inner(request);
        Box::pin(async move {
            let _cleanup = cleanup;
            future.await
        })
    })
}

#[cfg(test)]
fn default_turn_transcriber() -> TurnTranscriber {
    Arc::new(|request| Box::pin(crate::providers::groq::transcribe_saved_audio(request)))
}

/// A note run stopped by a provider rate limit too long to wait out; its
/// `details.resumeAtMs` says when the run continues.
pub(crate) const TRANSCRIPTION_RATE_LIMITED_CODE: &str = "transcription_rate_limited";
/// Transcription requests started per minute, summed over both Sources, the
/// live preview and every note: two below Groq's 20 per minute, which
/// dictation shares.
const GROQ_REQUESTS_PER_WINDOW: usize = 18;
const GROQ_REQUEST_WINDOW: Duration = Duration::from_secs(60);
/// Groq's audio seconds per hour (ASH), the limit two hour-long Sources
/// reach on their own.
const GROQ_AUDIO_BUDGET: Duration = Duration::from_secs(7_200);
const GROQ_AUDIO_WINDOW: Duration = Duration::from_secs(3_600);
/// The part of the hour's audio the live preview may spend: the rest stays
/// for the final transcription, which otherwise starts already over the limit.
pub(crate) const LIVE_PREVIEW_AUDIO_SHARE: Duration = Duration::from_secs(3_600);
/// Groq bills every request as at least this much audio.
const GROQ_MIN_BILLED_AUDIO: Duration = Duration::from_secs(10);
/// The longest wait a run sits through (Groq's per-minute window with
/// margin); a longer one pauses the note.
const RATE_LIMIT_MAX_INLINE_WAIT_MS: u64 = 90_000;
/// Rate-limit waits one request sits through before the note pauses.
const RATE_LIMIT_MAX_INLINE_WAITS: usize = 5;
/// The wait of a rate limit that names none, as in Groq's own example.
const RATE_LIMIT_DEFAULT_WAIT_MS: u64 = 60_000;

/// How often a waiting request looks at the clock again: a monotonic sleep
/// stops while the Mac sleeps, and the pacer's times are wall-clock times.
const PACER_RECHECK: Duration = Duration::from_secs(30);

fn pacer_ms(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX / 4)
}

fn pacer_duration(ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

/// Paces requests to one provider account: at most `max_requests` starts in
/// any `window`, at most `audio_budget` of billed audio in any
/// `audio_window`, and nothing while a provider rate limit holds every caller.
/// Times are wall-clock milliseconds, so a pause or an hour of audio ends on
/// time even across sleep; a clock set back never stretches them.
pub(crate) struct RequestPacer {
    max_requests: usize,
    window: Duration,
    audio_budget: Duration,
    audio_window: Duration,
    clock: PacerClock,
    state: std::sync::Mutex<RequestPacerState>,
}

#[derive(Default)]
struct RequestPacerState {
    starts: VecDeque<i64>,
    audio: VecDeque<(i64, Duration)>,
    paused_until: Option<i64>,
    /// The latest time read, and what is added to the raw clock after it
    /// was set back, so the pacer's time never goes backwards.
    clock_last_ms: i64,
    clock_offset_ms: i64,
}

impl RequestPacerState {
    fn prune(&mut self, now: i64, window: Duration, audio_window: Duration) {
        let window = pacer_ms(window);
        let audio_window = pacer_ms(audio_window);
        while self
            .starts
            .front()
            .is_some_and(|start| now - start >= window)
        {
            self.starts.pop_front();
        }
        while self
            .audio
            .front()
            .is_some_and(|(start, _)| now - start >= audio_window)
        {
            self.audio.pop_front();
        }
    }

    fn audio_used(&self) -> Duration {
        self.audio.iter().map(|(_, billed)| *billed).sum()
    }
}

fn wall_clock_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl RequestPacer {
    pub(crate) fn new(max_requests: usize, window: Duration) -> Self {
        Self {
            max_requests: max_requests.max(1),
            window,
            audio_budget: Duration::MAX,
            audio_window: Duration::from_secs(3_600),
            clock: Arc::new(wall_clock_ms),
            state: std::sync::Mutex::new(RequestPacerState::default()),
        }
    }

    pub(crate) fn with_audio_budget(mut self, budget: Duration, window: Duration) -> Self {
        self.audio_budget = budget;
        self.audio_window = window;
        self
    }

    #[cfg(test)]
    pub(crate) fn with_clock(mut self, clock: PacerClock) -> Self {
        self.clock = clock;
        self
    }

    fn state(&self) -> std::sync::MutexGuard<'_, RequestPacerState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The clock, never earlier than a time already read.
    fn now(&self, state: &mut RequestPacerState) -> i64 {
        let now = (self.clock)().saturating_add(state.clock_offset_ms);
        if now < state.clock_last_ms {
            state.clock_offset_ms += state.clock_last_ms - now;
            return state.clock_last_ms;
        }
        state.clock_last_ms = now;
        now
    }

    /// How long until a request billing `billed` may start; zero takes the
    /// slot now.
    fn wait_for_slot(&self, billed: Duration) -> Duration {
        let mut state = self.state();
        let now = self.now(&mut state);
        state.prune(now, self.window, self.audio_window);
        let mut wait_ms = state.paused_until.map_or(0, |until| until - now).max(0);
        if state.starts.len() >= self.max_requests {
            if let Some(oldest) = state.starts.front() {
                wait_ms = wait_ms.max(oldest + pacer_ms(self.window) - now);
            }
        }
        // A request larger than the whole budget waits for an empty window.
        let allowed = self
            .audio_budget
            .saturating_sub(billed.min(self.audio_budget));
        let mut used = state.audio_used();
        if used > allowed {
            for (start, spent) in &state.audio {
                used = used.saturating_sub(*spent);
                if used <= allowed {
                    wait_ms = wait_ms.max(start + pacer_ms(self.audio_window) - now);
                    break;
                }
            }
        }
        if wait_ms <= 0 {
            state.starts.push_back(now);
            state.audio.push_back((now, billed));
            return Duration::ZERO;
        }
        pacer_duration(wait_ms)
    }

    /// Takes a slot for a request billing `billed`, waiting while the wait is
    /// at most `max_wait`; a longer wait comes back without a slot.
    pub(crate) async fn acquire(
        &self,
        billed: Duration,
        max_wait: Duration,
    ) -> Result<(), Duration> {
        loop {
            let wait = self.wait_for_slot(billed);
            if wait.is_zero() {
                return Ok(());
            }
            if wait > max_wait {
                return Err(wait);
            }
            tokio::time::sleep(wait.min(PACER_RECHECK)).await;
        }
    }

    /// A slot only if one is free now and the window's billed audio stays
    /// within `audio_share`: the live preview never waits and never spends
    /// what the final transcription needs.
    pub(crate) fn try_acquire(&self, billed: Duration, audio_share: Duration) -> bool {
        let mut state = self.state();
        let now = self.now(&mut state);
        state.prune(now, self.window, self.audio_window);
        let free = !state.paused_until.is_some_and(|until| until > now)
            && state.starts.len() < self.max_requests
            && state.audio_used() + billed <= audio_share.min(self.audio_budget);
        if free {
            state.starts.push_back(now);
            state.audio.push_back((now, billed));
        }
        free
    }

    /// Holds every caller for `wait`, as a provider rate limit does.
    pub(crate) fn pause_for(&self, wait: Duration) {
        let mut state = self.state();
        let until = self.now(&mut state).saturating_add(pacer_ms(wait));
        state.paused_until = Some(
            state
                .paused_until
                .map_or(until, |current| current.max(until)),
        );
    }
}

/// Wall-clock milliseconds since the epoch, injectable for tests.
pub(crate) type PacerClock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// A Groq key's identity for pacing: the first 8 bytes of its SHA-256, in
/// hex. The key itself is never kept.
pub(crate) fn key_fingerprint(key: &str) -> String {
    Sha256::digest(key.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One pacer per Groq account the user's keys reach: Groq limits the
/// account, not the note, and a pause on one key never holds another.
static GROQ_DIRECT_PACERS: std::sync::LazyLock<
    std::sync::Mutex<HashMap<String, Arc<RequestPacer>>>,
> = std::sync::LazyLock::new(Default::default);

pub(crate) fn groq_direct_pacer(fingerprint: &str) -> Arc<RequestPacer> {
    let mut pacers = GROQ_DIRECT_PACERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    Arc::clone(pacers.entry(fingerprint.to_string()).or_insert_with(|| {
        Arc::new(
            RequestPacer::new(GROQ_REQUESTS_PER_WINDOW, GROQ_REQUEST_WINDOW)
                .with_audio_budget(GROQ_AUDIO_BUDGET, GROQ_AUDIO_WINDOW),
        )
    }))
}

/// The saved Groq key and its account's pacer, resolved once per request.
const TRANSCRIPTION_ENGINE_CHANGED_MESSAGE: &str = "The transcription engine changed during processing. Retry to transcribe with the current engine.";

fn note_route(model: String) -> Result<TranscriptionRoute, AppError> {
    note_route_for_backend(model, crate::providers::transcription::current_backend()?)
}

fn note_route_for_backend(
    model: String,
    backend: TranscriptionBackend,
) -> Result<TranscriptionRoute, AppError> {
    match backend {
        TranscriptionBackend::Groq { .. }
            if model == crate::providers::transcription::LOCAL_MODEL =>
        {
            // A run admitted as Local never sends its remaining audio to Groq.
            // Switching engines terminates that helper; an explicit retry uses the new choice.
            Err(AppError::new(
                "transcription_engine_changed",
                TRANSCRIPTION_ENGINE_CHANGED_MESSAGE,
            ))
        }
        TranscriptionBackend::Groq { key, .. } => note_route_from(Ok(Some(key)), model),
        TranscriptionBackend::Local { .. }
            if model != crate::providers::transcription::LOCAL_MODEL =>
        {
            // Keep the admitted model/fingerprint consistent across every request.
            Err(AppError::new(
                "transcription_engine_changed",
                TRANSCRIPTION_ENGINE_CHANGED_MESSAGE,
            ))
        }
        backend @ TranscriptionBackend::Local { .. } => Ok(TranscriptionRoute {
            backend,
            model: crate::providers::transcription::LOCAL_MODEL.into(),
            pacer: None,
        }),
    }
}

/// A missing or unreadable key fails before any audio leaves the device.
fn note_route_from(
    saved_key: Result<Option<String>, AppError>,
    model: String,
) -> Result<TranscriptionRoute, AppError> {
    let key = saved_key?;
    let groq_key = crate::providers::groq::require_groq_key(key.as_deref())?.to_string();
    let pacer = groq_direct_pacer(&key_fingerprint(&groq_key));
    Ok(TranscriptionRoute {
        backend: TranscriptionBackend::Groq {
            key: groq_key,
            model: model.clone(),
        },
        model,
        pacer: Some(pacer),
    })
}

/// Resolves a run's route for each of its requests, with the run's model.
fn note_route_resolver(model: String) -> RouteResolver {
    Arc::new(move || note_route(model.clone()))
}

/// The durable key of one chunk request of a split saved-audio job: the
/// chunk's operation id (the durable job's own, then `-chunk-N`), its audio
/// length, so another split never reuses a stale chunk, and the model and
/// language it is transcribed with, so a resume after either changed sends
/// the chunk again. `None` for a request that is not a chunk of a durable
/// job, which its job already keeps.
fn chunk_cache_key(
    route: &TranscriptionRoute,
    request: &TranscriptionRequest,
) -> Option<(String, String)> {
    let operation_id = request.operation_id.as_deref()?;
    let (job_operation_id, _) = operation_id.rsplit_once("-chunk-")?;
    // Durable operation ids start with the recording session id.
    let (session_id, _) = job_operation_id.split_once(':')?;
    let duration_ms = source_wav_duration_ms(&request.audio_path).unwrap_or_default();
    let language = request.language.as_deref().unwrap_or("auto");
    Some((
        format!("{operation_id}|{duration_ms}|{}|{language}", route.model),
        session_id.to_string(),
    ))
}

/// Keeps each finished chunk of a split job (a full-Source job, an
/// over-long turn, the full-Source fallback) in `note_transcription_chunks`:
/// a run paused by a rate limit resumes with only the chunks still missing,
/// instead of sending the whole job again into the same limit.
fn cached_chunk_transcriber(
    inner: RoutedCall<TranscriptionProviderResult>,
    repos: Repositories,
) -> RoutedCall<TranscriptionProviderResult> {
    Arc::new(move |route, request| {
        let inner = Arc::clone(&inner);
        let repos = repos.clone();
        Box::pin(async move {
            let Some((chunk_key, session_id)) = chunk_cache_key(&route, &request) else {
                return inner(route, request).await;
            };
            match repos.note_transcription_chunk(&chunk_key).await {
                Ok(Some(chunk)) if chunk.no_speech => {
                    return Err(AppError::new("no_speech", "no_speech"));
                }
                Ok(Some(chunk)) => {
                    return Ok(TranscriptionProviderResult {
                        text: chunk.text,
                        language: chunk.language,
                        provider: request.provider,
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "could not read a finished transcription chunk");
                }
            }
            let result = inner(route, request).await;
            let finished = match &result {
                Ok(transcript) => Some(NoteTranscriptionChunk {
                    text: transcript.text.clone(),
                    language: transcript.language.clone(),
                    no_speech: false,
                }),
                Err(error) if is_no_speech_error(error) => Some(NoteTranscriptionChunk {
                    text: String::new(),
                    language: None,
                    no_speech: true,
                }),
                Err(_) => None,
            };
            if let Some(chunk) = finished {
                if let Err(error) = repos
                    .save_note_transcription_chunk(&chunk_key, &session_id, &chunk)
                    .await
                {
                    tracing::warn!(%error, "could not keep a finished transcription chunk");
                }
            }
            result
        }) as ProviderFuture<TranscriptionProviderResult>
    })
}

/// The account and model one note-transcription request goes with: the
/// pacer, the kept-chunk key and the transport all use this one value.
#[derive(Clone)]
pub(crate) struct TranscriptionRoute {
    /// The user's Groq key; production routes always require it.
    pub(crate) backend: TranscriptionBackend,
    pub(crate) model: String,
    pub(crate) pacer: Option<Arc<RequestPacer>>,
}

#[cfg(test)]
impl TranscriptionRoute {
    fn groq_key(&self) -> Option<&str> {
        match &self.backend {
            TranscriptionBackend::Groq { key, .. } if !key.is_empty() => Some(key),
            _ => None,
        }
    }
}

impl std::fmt::Debug for TranscriptionRoute {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TranscriptionRoute")
            .field("backend", &self.backend)
            .field("model", &self.model)
            .finish()
    }
}

type RouteResolver = Arc<dyn Fn() -> Result<TranscriptionRoute, AppError> + Send + Sync>;
type RoutedCall<T> =
    Arc<dyn Fn(TranscriptionRoute, TranscriptionRequest) -> ProviderFuture<T> + Send + Sync>;

/// Resolves the route once when each request starts and hands that same
/// route to every layer below: the kept chunks, the pacer and the call. A
/// route that cannot be resolved fails the request with its error.
fn routed<T: Send + 'static>(inner: RoutedCall<T>, resolve: RouteResolver) -> ProviderCall<T> {
    Arc::new(move |request| match resolve() {
        Ok(route) => inner(route, request),
        Err(error) => Box::pin(async move { Err(error) }) as ProviderFuture<T>,
    })
}

/// A note run's turn transcriber: the route resolved for each request, the
/// kept chunks, the pacer and the provider call.
fn note_chain(
    call: RoutedCall<TranscriptionProviderResult>,
    resolve: RouteResolver,
    repos: Repositories,
) -> TurnTranscriber {
    routed(cached_chunk_transcriber(paced(call), repos), resolve)
}

/// The note-transcription request with the resolved Groq key and model.
fn note_provider_call() -> RoutedCall<TranscriptionProviderResult> {
    Arc::new(|route, request| {
        Box::pin(async move {
            match route.backend {
                TranscriptionBackend::Groq { key, .. } => {
                    crate::providers::groq::transcribe_saved_audio_with(
                        Some(&key),
                        route.model,
                        request,
                    )
                    .await
                }
                TranscriptionBackend::Local { model_dir } => {
                    crate::providers::local_transcriber::transcribe(
                        &model_dir,
                        &request.audio_path,
                        request.language.as_deref(),
                    )
                    .await?
                    .plain()
                }
            }
        })
    })
}

/// A timed (turn pack) request on a resolved route; only Groq with the
/// user's key answers with segment times. Tests pack with a fake instead.
#[cfg_attr(test, allow(dead_code))]
fn note_timed_call() -> RoutedCall<TimedTranscription> {
    Arc::new(|route, request| {
        Box::pin(async move {
            match route.backend {
                TranscriptionBackend::Groq { key, .. } => {
                    crate::providers::groq::transcribe_saved_audio_timed_with(
                        &key,
                        route.model,
                        request,
                    )
                    .await
                }
                TranscriptionBackend::Local { model_dir } => {
                    crate::providers::local_transcriber::transcribe(
                        &model_dir,
                        &request.audio_path,
                        request.language.as_deref(),
                    )
                    .await?
                    .timed()
                }
            }
        })
    })
}

/// The route a live preview chunk of `duration_ms` takes, when it may go
/// now (`audio/live_preview.rs`): it shares the pacer of that route, and the
/// chunk is sent with this same route.
pub(crate) fn live_preview_route(duration_ms: i64) -> Option<TranscriptionRoute> {
    // A preview chunk has no persisted run fingerprint. Capture its model and
    // backend together, so a concurrent settings change cannot mix selections.
    let route = crate::providers::transcription::current_backend().and_then(|backend| {
        let model = match &backend {
            TranscriptionBackend::Groq { model, .. } => model.clone(),
            TranscriptionBackend::Local { .. } => {
                crate::providers::transcription::LOCAL_MODEL.into()
            }
        };
        note_route_for_backend(model, backend)
    });
    live_preview_route_from(route, duration_ms)
}

fn live_preview_route_from(
    route: Result<TranscriptionRoute, AppError>,
    duration_ms: i64,
) -> Option<TranscriptionRoute> {
    let route = route.ok()?;
    route
        .pacer
        .as_ref()
        .map_or(true, |pacer| {
            pacer.try_acquire(billed_audio(duration_ms), LIVE_PREVIEW_AUDIO_SHARE)
        })
        .then_some(route)
}

pub(crate) async fn transcribe_on_route(
    route: TranscriptionRoute,
    request: TranscriptionRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    note_provider_call()(route, request).await
}

/// The audio Groq bills for a request of `duration_ms`.
fn billed_audio(duration_ms: i64) -> Duration {
    Duration::from_millis(u64::try_from(duration_ms).unwrap_or(0)).max(GROQ_MIN_BILLED_AUDIO)
}

fn is_provider_rate_limit(error: &AppError) -> bool {
    error.code.ends_with("_usage_limit_exceeded")
}

type ProviderFuture<T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send>>;
type ProviderCall<T> = Arc<dyn Fn(TranscriptionRequest) -> ProviderFuture<T> + Send + Sync>;

/// Every request through its route's pacer, billed by its audio's duration.
/// A provider rate limit holds all callers of that pacer for the wait it
/// names and repeats the same request on the same route; a wait too long to
/// sit through, the pacer's or the provider's, becomes
/// [`TRANSCRIPTION_RATE_LIMITED_CODE`].
fn paced<T: Send + 'static>(inner: RoutedCall<T>) -> RoutedCall<T> {
    Arc::new(move |route, request| {
        let inner = Arc::clone(&inner);
        let pacer = route.pacer.clone();
        Box::pin(async move {
            let Some(pacer) = pacer else {
                return inner(route, request).await;
            };
            let billed = billed_audio(source_wav_duration_ms(&request.audio_path).unwrap_or(0));
            let max_wait = Duration::from_millis(RATE_LIMIT_MAX_INLINE_WAIT_MS);
            let mut waits = 0;
            loop {
                if let Err(wait) = pacer.acquire(billed, max_wait).await {
                    let wait_ms = u64::try_from(wait.as_millis()).unwrap_or(u64::MAX);
                    tracing::warn!(
                        operation_id = %request.operation_id(),
                        wait_ms,
                        "provider budget pauses the note"
                    );
                    return Err(rate_limit_pause(wait_ms));
                }
                match inner(route.clone(), request.clone()).await {
                    Err(error) if is_provider_rate_limit(&error) => {
                        let wait_ms = retry_after_ms(&error).unwrap_or(RATE_LIMIT_DEFAULT_WAIT_MS);
                        pacer.pause_for(Duration::from_millis(wait_ms));
                        if wait_ms > RATE_LIMIT_MAX_INLINE_WAIT_MS
                            || waits >= RATE_LIMIT_MAX_INLINE_WAITS
                        {
                            tracing::warn!(
                                operation_id = %request.operation_id(),
                                wait_ms,
                                waits,
                                "provider rate limit pauses the note"
                            );
                            return Err(rate_limit_pause(wait_ms));
                        }
                        waits += 1;
                        tracing::warn!(
                            operation_id = %request.operation_id(),
                            wait_ms,
                            waits,
                            "provider rate limit; waiting before the same request"
                        );
                    }
                    other => return other,
                }
            }
        }) as ProviderFuture<T>
    })
}

/// A provider call that ignores the route: a test's fake transcriber.
#[cfg(test)]
fn ignoring_route<T: Send + 'static>(inner: ProviderCall<T>) -> RoutedCall<T> {
    Arc::new(move |_route, request| inner(request))
}

/// One fixed route with `pacer`, for tests.
#[cfg(test)]
fn fixed_route(model: &str, pacer: Arc<RequestPacer>) -> RouteResolver {
    let model = model.to_string();
    Arc::new(move || {
        Ok(TranscriptionRoute {
            backend: TranscriptionBackend::Groq {
                key: String::new(),
                model: model.clone(),
            },
            model: model.clone(),
            pacer: Some(Arc::clone(&pacer)),
        })
    })
}

#[cfg(test)]
fn paced_turn_transcriber(inner: TurnTranscriber, pacer: Arc<RequestPacer>) -> TurnTranscriber {
    routed(
        paced(ignoring_route(inner)),
        fixed_route(crate::providers::DEFAULT_TRANSCRIPTION_MODEL, pacer),
    )
}

fn rate_limit_pause(wait_ms: u64) -> AppError {
    let resume_at = chrono::Utc::now()
        + chrono::Duration::milliseconds(i64::try_from(wait_ms).unwrap_or(i64::MAX / 2));
    let mut error = AppError::new(
        TRANSCRIPTION_RATE_LIMITED_CODE,
        rate_limit_paused_message(&resume_at.with_timezone(&chrono::Local)),
    );
    error.details = Some(serde_json::json!({ "resumeAtMs": resume_at.timestamp_millis() }));
    error
}

// Translated by pattern as `notes.failure.rateLimitPaused` in
// `NoteFailureBanner`: keep the sentence and the HH:MM time in sync.
fn rate_limit_paused_message(resume_at: &chrono::DateTime<chrono::Local>) -> String {
    format!(
        "Groq usage limit reached. Clovy continues transcribing at {}. Parts already transcribed are kept.",
        resume_at.format("%H:%M")
    )
}

/// The final-turn transcriber uses Groq with the user's key and the run's
/// model. Each request keeps one resolved route for its chunks, pacer and call.
fn note_turn_transcriber(repos: Repositories, model: String) -> TurnTranscriber {
    #[cfg(test)]
    if let Ok((transcriber, pacer)) = TEST_NOTE_TRANSCRIPTION
        .try_with(|(transcriber, pacer)| (transcriber.clone(), pacer.clone()))
    {
        return note_chain(
            ignoring_route(transcriber),
            fixed_route(&model, pacer),
            repos,
        );
    }
    note_chain(note_provider_call(), note_route_resolver(model), repos)
}

#[cfg(test)]
tokio::task_local! {
    /// The transcriber and pacer a pipeline test runs instead of Groq.
    static TEST_NOTE_TRANSCRIPTION: (TurnTranscriber, Arc<RequestPacer>);
    /// The timed transcriber a pipeline test packs turns with.
    static TEST_NOTE_PACKS: (PackTranscriber, Arc<RequestPacer>);
}

/// Timed packs go straight to Groq with the user's key.
/// Each pack request reads its route once, like a turn, so its pacer and
/// its call use the same key.
fn note_pack_transcriber(model: String) -> Option<PackTranscriber> {
    // Design risk mitigation: imprecise Local token ends can bridge a 500 ms
    // pack gap under the 400 ms pause rule. Keep each turn in its own request
    // rather than assigning merged text to the wrong turn by midpoint.
    if model == crate::providers::transcription::LOCAL_MODEL {
        return None;
    }
    #[cfg(test)]
    {
        if let Ok(transcriber) = TEST_NOTE_PACKS.try_with(|(transcriber, pacer)| {
            routed(
                paced(ignoring_route(transcriber.clone())),
                fixed_route(&model, pacer.clone()),
            )
        }) {
            return Some(transcriber);
        }
        if crate::providers::local_transcriber::TEST_LOCAL
            .try_with(|_| ())
            .is_err()
        {
            return None;
        }
    }
    note_route(model.clone())
        .ok()
        .map(|_| routed(paced(note_timed_call()), note_route_resolver(model)))
}

/// Coalesces turns up to the final span, unless the session already has
/// transcribed turns in the legacy 30 s layout and that layout reuses more
/// of them: a resumed run never sends a finished turn again.
fn coalesce_turns_reusing_layout(
    turns: Vec<AudioTurn>,
    microphone_echo_spans: &[(i64, i64)],
    session_id: &str,
    done_span_ids: &HashSet<String>,
) -> Vec<AudioTurn> {
    let current = coalesce_turns_for_transcription_with_max_span(
        turns.clone(),
        microphone_echo_spans,
        FINAL_TRANSCRIPTION_SPAN_MS,
    );
    if done_span_ids.is_empty() {
        return current;
    }
    let legacy = coalesce_turns_for_transcription_with_max_span(
        turns,
        microphone_echo_spans,
        LEGACY_TRANSCRIPTION_SPAN_MS,
    );
    let reused = |grouped: &[AudioTurn]| {
        grouped
            .iter()
            .filter(|turn| done_span_ids.contains(&turn_span_id(session_id, turn)))
            .count()
    };
    if reused(&legacy) > reused(&current) {
        legacy
    } else {
        current
    }
}

/// The durable job id of a turn, as the pipeline plans it.
fn turn_span_id(session_id: &str, turn: &AudioTurn) -> String {
    let end_ms = if covers_full_source(turn.start_ms, turn.end_ms) {
        source_wav_duration_ms(&turn.source_path).unwrap_or(turn.end_ms)
    } else {
        turn.end_ms
    };
    note_transcription_span_id(
        session_id,
        &turn.source,
        NoteTranscriptionJobKind::Turn,
        turn.start_ms,
        end_ms,
    )
}

fn instrument_turn_transcriber(
    inner: TurnTranscriber,
    timeline: FirstEventTimeline,
) -> TurnTranscriber {
    Arc::new(move |request| {
        timeline.mark_first_request();
        inner(request)
    })
}

struct TranscribePreparedAudioRequest {
    provider: String,
    audio_path: PathBuf,
    temp_dir: PathBuf,
    chunk_stem: String,
    title: String,
    /// The dictionary hint sent as the transcription `prompt` of every chunk.
    /// Previous transcript text never goes there: it reaches the correction
    /// engine with the full turn context instead.
    asr_hint: Option<String>,
    operation_id: String,
    max_chunk_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TranscriptionChunkOutcome {
    start_ms: i64,
    end_ms: i64,
    locally_silent: bool,
    provider_returned_text: bool,
    provider_no_speech: bool,
}

#[derive(Debug, Clone)]
struct TranscribePreparedAudioResult {
    transcript: TranscriptionProviderResult,
    chunk_outcomes: Vec<TranscriptionChunkOutcome>,
}

fn prepare_turn_job(descriptor: TurnPreparationJob) -> Result<PreparedTurn, AppError> {
    let covers_full_source = descriptor.covers_full_source();
    let raw_path = if covers_full_source {
        descriptor.turn.source_path.clone()
    } else {
        write_turn_wav(&descriptor.turn, &descriptor.turn_wav_path)?;
        descriptor.turn_wav_path.clone()
    };
    let audio_path = normalize_wav_for_transcription(&raw_path, &descriptor.normalized_path)?;
    Ok(PreparedTurn {
        schedule_index: descriptor.schedule_index,
        job: TurnTranscriptionJob {
            artifact_id: descriptor.turn.artifact_id,
            source: descriptor.turn.source,
            audio_path,
            temp_dir: descriptor.temp_dir,
            recorded_silence: descriptor.recorded_silence,
            covers_full_source,
            source_fallback: false,
            start_ms: descriptor.turn.start_ms,
            end_ms: descriptor.turn.end_ms,
            turn_index: descriptor.turn.turn_index,
            durable_job: descriptor.durable_job,
        },
    })
}

fn prepare_source_fallback(plan: SourceFallbackPlan) -> Result<TurnTranscriptionJob, AppError> {
    let audio_path = normalize_wav_for_transcription(&plan.source_path, &plan.normalized_path)?;
    Ok(TurnTranscriptionJob {
        artifact_id: plan.artifact_id,
        source: plan.source,
        audio_path,
        temp_dir: plan.temp_dir,
        recorded_silence: plan.recorded_silence,
        covers_full_source: true,
        source_fallback: true,
        start_ms: 0,
        end_ms: plan.end_ms,
        turn_index: plan.turn_index,
        durable_job: plan.durable_job,
    })
}

fn build_source_fallback_plans(descriptors: &[TurnPreparationJob]) -> Vec<SourceFallbackPlan> {
    let mut plans = Vec::<SourceFallbackPlan>::new();
    let mut source_indices = HashMap::<String, usize>::new();
    for descriptor in descriptors {
        if let Some(index) = source_indices.get(&descriptor.turn.source).copied() {
            let plan = &mut plans[index];
            plan.all_turns_cover_full_source &= descriptor.covers_full_source();
            plan.echo_trimmed |= descriptor.echo_trimmed;
            plan.end_ms = plan.end_ms.max(descriptor.turn.end_ms);
            plan.detected_ms = plan
                .detected_ms
                .saturating_add(span_ms(descriptor.turn.start_ms, descriptor.turn.end_ms));
            continue;
        }

        source_indices.insert(descriptor.turn.source.clone(), plans.len());
        plans.push(SourceFallbackPlan {
            artifact_id: descriptor.turn.artifact_id.clone(),
            source: descriptor.turn.source.clone(),
            source_path: descriptor.turn.source_path.clone(),
            normalized_path: descriptor
                .temp_dir
                .join(format!("{}-source-normalized.wav", descriptor.turn.source)),
            temp_dir: descriptor.temp_dir.clone(),
            recorded_silence: descriptor.recorded_silence,
            all_turns_cover_full_source: descriptor.covers_full_source(),
            echo_trimmed: descriptor.echo_trimmed,
            end_ms: descriptor.turn.end_ms,
            turn_index: descriptor.turn.turn_index,
            detected_ms: span_ms(descriptor.turn.start_ms, descriptor.turn.end_ms),
            durable_job: None,
        });
    }
    // `sort_by_key` is stable, so unrecognized sources retain descriptor order.
    plans.sort_by_key(|plan| match plan.source.as_str() {
        "microphone" => 0,
        "system" => 1,
        _ => 2,
    });
    plans
}

fn interleave_turn_preparation_jobs_by_source(
    descriptors: Vec<TurnPreparationJob>,
) -> Vec<TurnPreparationJob> {
    let mut lanes = Vec::<(String, VecDeque<TurnPreparationJob>)>::new();
    let mut lane_indices = HashMap::<String, usize>::new();
    for descriptor in descriptors {
        let source = descriptor.turn.source.clone();
        let lane_index = match lane_indices.get(&source).copied() {
            Some(index) => index,
            None => {
                let index = lanes.len();
                lane_indices.insert(source.clone(), index);
                lanes.push((source, VecDeque::new()));
                index
            }
        };
        lanes[lane_index].1.push_back(descriptor);
    }
    lanes.sort_by_key(|(source, _)| match source.as_str() {
        "microphone" => 0,
        "system" => 1,
        _ => 2,
    });

    let mut interleaved = Vec::new();
    loop {
        let mut added = false;
        for (_, lane) in &mut lanes {
            if let Some(descriptor) = lane.pop_front() {
                interleaved.push(descriptor);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    interleaved
}

fn spawn_turn_preparation(
    descriptors: Vec<TurnPreparationJob>,
    preparer: TurnPreparer,
    timing: ProcessingTiming,
) -> (
    tokio::sync::mpsc::Receiver<Result<PreparedTurn, AppError>>,
    tokio::task::JoinHandle<TurnPreparationReport>,
) {
    let (sender, receiver) = tokio::sync::mpsc::channel(PREPARED_TURN_CHANNEL_CAPACITY);
    let handle = tokio::task::spawn_blocking(move || {
        let producer_started = Instant::now();
        let mut prepared_count = 0;
        let mut active_preparation_duration_ms = 0_i64;
        let mut terminal_error = None;
        for descriptor in descriptors {
            let preparation_started = Instant::now();
            let prepared = preparer(descriptor);
            active_preparation_duration_ms =
                active_preparation_duration_ms.saturating_add(elapsed_ms(preparation_started));
            match prepared {
                Ok(prepared) => {
                    prepared_count += 1;
                    if sender.blocking_send(Ok(prepared)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = sender.blocking_send(Err(error.clone()));
                    terminal_error = Some(error);
                    break;
                }
            }
        }
        TurnPreparationReport {
            prepared_count,
            active_preparation_duration_ms,
            producer_wall_duration_ms: elapsed_ms(producer_started),
            done_to_preparation_complete_ms: timing.done_to_duration_ms(),
            error: terminal_error,
        }
    });
    (receiver, handle)
}

async fn transcribe_prepared_audio(
    transcriber: TurnTranscriber,
    request: TranscribePreparedAudioRequest,
) -> Result<TranscribePreparedAudioResult, AppError> {
    let request_language = crate::dictation::configured_transcription_language();
    let chunk_dir = request.temp_dir.join("chunks");
    let audio_paths = if request.audio_path.exists() {
        if let Some(max_chunk_ms) = request.max_chunk_ms {
            split_wav_for_transcription_with_max_duration(
                &request.audio_path,
                &chunk_dir,
                &request.chunk_stem,
                max_chunk_ms,
            )?
        } else {
            split_wav_for_transcription(&request.audio_path, &chunk_dir, &request.chunk_stem)?
        }
    } else {
        vec![request.audio_path.clone()]
    };
    if audio_paths.len() == 1 {
        let audio_path = audio_paths.into_iter().next().unwrap_or(request.audio_path);
        let duration_ms = source_wav_duration_ms(&audio_path).unwrap_or_default();
        if crate::audio::turns::source_is_effectively_silent(&audio_path) {
            return Err(AppError::new("no_speech", "no_speech"));
        }
        let transcript = transcribe_with_transient_retries(
            &transcriber,
            TranscriptionRequest {
                provider: request.provider,
                audio_path,
                title: request.title,
                context: request.asr_hint,
                language: request_language,
                operation_id: Some(request.operation_id),
                preview: false,
            },
        )
        .await?;
        if transcript.text.trim().is_empty() {
            return Err(AppError::new("no_speech", "no_speech"));
        }
        return Ok(TranscribePreparedAudioResult {
            chunk_outcomes: vec![TranscriptionChunkOutcome {
                start_ms: 0,
                end_ms: duration_ms,
                locally_silent: false,
                provider_returned_text: !transcript.text.trim().is_empty(),
                provider_no_speech: false,
            }],
            transcript,
        });
    }

    let mut text_parts = Vec::new();
    let mut language = None;
    let mut provider_name = request.provider.clone();
    let mut chunk_outcomes = Vec::new();
    let mut chunk_start_ms = 0_i64;
    // The default split repeats the last 10 s of a chunk at the start of the
    // next one; an explicit chunk length splits back to back.
    let overlap_ms = if request.max_chunk_ms.is_none() {
        SPLIT_OVERLAP_MS
    } else {
        0
    };
    // Whether the last text part came from the chunk right before this one,
    // so its overlap may repeat at this chunk's start.
    let mut previous_chunk_has_text = false;
    for (index, audio_path) in audio_paths.into_iter().enumerate() {
        let duration_ms = source_wav_duration_ms(&audio_path).unwrap_or_default();
        let chunk_end_ms = chunk_start_ms.saturating_add(duration_ms);
        // Skip clearly-silent chunks before any API call. Fixed-size splitting of
        // a long (or fully silent) source leaves quiet boundary chunks, and each
        // request authorizes a credit hold that a no-speech response never
        // settles — so sending every silent chunk of a silent source would strand
        // holds until TTL and can trip `authorization_denied` on later work.
        if crate::audio::turns::source_is_effectively_silent(&audio_path) {
            chunk_outcomes.push(TranscriptionChunkOutcome {
                start_ms: chunk_start_ms,
                end_ms: chunk_end_ms,
                locally_silent: true,
                provider_returned_text: false,
                provider_no_speech: false,
            });
            chunk_start_ms = chunk_end_ms - overlap_ms;
            previous_chunk_has_text = false;
            continue;
        }
        let transcript = match transcribe_with_transient_retries(
            &transcriber,
            TranscriptionRequest {
                provider: request.provider.clone(),
                audio_path,
                title: request.title.clone(),
                context: request.asr_hint.clone(),
                language: request_language.clone(),
                operation_id: Some(format!("{}-chunk-{index}", request.operation_id)),
                preview: false,
            },
        )
        .await
        {
            Ok(transcript) => transcript,
            // Backstop for a chunk the local silence check judged audible but the
            // provider still reports as no-speech: skip it so earlier chunks' text
            // survives, rather than aborting and dropping the whole turn.
            Err(error) if is_no_speech_error(&error) => {
                chunk_outcomes.push(TranscriptionChunkOutcome {
                    start_ms: chunk_start_ms,
                    end_ms: chunk_end_ms,
                    locally_silent: false,
                    provider_returned_text: false,
                    provider_no_speech: true,
                });
                chunk_start_ms = chunk_end_ms - overlap_ms;
                previous_chunk_has_text = false;
                continue;
            }
            Err(error) => return Err(error),
        };
        if language.is_none() {
            language = transcript.language.clone();
        }
        provider_name = transcript.provider.clone();
        let text = transcript.text.trim().to_string();
        chunk_outcomes.push(TranscriptionChunkOutcome {
            start_ms: chunk_start_ms,
            end_ms: chunk_end_ms,
            locally_silent: false,
            provider_returned_text: !text.is_empty(),
            provider_no_speech: false,
        });
        chunk_start_ms = chunk_end_ms - overlap_ms;
        let text = match text_parts.last() {
            Some(previous) if overlap_ms > 0 && previous_chunk_has_text => {
                let previous: &String = previous;
                drop_repeated_overlap(previous, &text)
            }
            _ => text,
        };
        previous_chunk_has_text = !text.is_empty();
        if !text.is_empty() {
            text_parts.push(text);
        }
    }

    if text_parts.is_empty() {
        // Every chunk was silent. Report it as a no-speech turn — exactly like a
        // single silent turn — so it stays a non-blocking failure rather than a
        // generic error that would fail the whole note.
        return Err(AppError::new("no_speech", "no_speech"));
    }
    Ok(TranscribePreparedAudioResult {
        transcript: TranscriptionProviderResult {
            text: text_parts.join("\n"),
            language,
            provider: provider_name,
        },
        chunk_outcomes,
    })
}

/// Words an overlap can repeat: 10 s of fast speech is about 35.
const OVERLAP_MAX_REPEATED_WORDS: usize = 40;
/// Fewer matching words than this is coincidence, not the overlap.
const OVERLAP_MIN_REPEATED_WORDS: usize = 2;

/// `next` without the words it repeats from the end of `previous`: the
/// longest run that ends `previous` and starts `next`, compared without case
/// or punctuation.
fn drop_repeated_overlap(previous: &str, next: &str) -> String {
    let normalized = |word: &str| {
        word.chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let previous_words = previous
        .split_whitespace()
        .map(normalized)
        .collect::<Vec<_>>();
    let next_words = next.split_whitespace().collect::<Vec<_>>();
    let next_normalized = next_words
        .iter()
        .map(|word| normalized(word))
        .collect::<Vec<_>>();
    let longest = OVERLAP_MAX_REPEATED_WORDS
        .min(previous_words.len())
        .min(next_words.len());
    for count in (OVERLAP_MIN_REPEATED_WORDS..=longest).rev() {
        if previous_words[previous_words.len() - count..] == next_normalized[..count] {
            return next_words[count..].join(" ");
        }
    }
    next.to_string()
}

async fn transcribe_with_transient_retries(
    transcriber: &TurnTranscriber,
    request: TranscriptionRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    let operation_id = request.operation_id();
    for attempt in 0..TRANSIENT_TRANSCRIPTION_ATTEMPTS {
        match transcriber(request.clone()).await {
            Ok(transcript) => return Ok(transcript),
            Err(error) => {
                if attempt + 1 < TRANSIENT_TRANSCRIPTION_ATTEMPTS
                    && is_retryable_transcription_error(&error)
                {
                    let retry_delay = transient_retry_delay(&operation_id, attempt, &error);
                    tracing::warn!(
                        operation_id = %operation_id,
                        code = %error.code,
                        attempt = attempt + 1,
                        retry_delay_ms = retry_delay.as_millis(),
                        "transient transcription request failed; retrying"
                    );
                    tokio::time::sleep(retry_delay).await;
                    continue;
                }
                return Err(error);
            }
        }
    }
    unreachable!("transcription retry loop always returns")
}

fn is_retryable_transcription_error(error: &AppError) -> bool {
    let code = error.code.trim().to_ascii_lowercase();
    let message = error.message.trim().to_ascii_lowercase();
    code == "clovy_api_response_invalid"
        || code == "june_api_response_invalid"
        || code == "empty_response"
        || (matches!(
            code.as_str(),
            "clovy_request_failed" | "june_request_failed"
        ) && (message == "authorization_denied"
            || message == "timeout"
            || message.contains("connection")
            || message.contains("error sending request")))
}

fn transient_retry_delay(operation_id: &str, attempt: usize, error: &AppError) -> Duration {
    if let Some(retry_after_ms) = retry_after_ms(error) {
        return Duration::from_millis(
            retry_after_ms.saturating_add(retry_jitter_ms(operation_id, attempt)),
        );
    }
    let backoff_multiplier = 1_u64 << attempt.min(8);
    let backoff_ms =
        TRANSIENT_TRANSCRIPTION_RETRY_BASE_BACKOFF_MS.saturating_mul(backoff_multiplier);
    Duration::from_millis(backoff_ms.saturating_add(retry_jitter_ms(operation_id, attempt)))
}

fn retry_after_ms(error: &AppError) -> Option<u64> {
    error
        .details
        .as_ref()
        .and_then(|details| details.get("retryAfterMs"))
        .and_then(serde_json::Value::as_u64)
}

fn retry_jitter_ms(operation_id: &str, attempt: usize) -> u64 {
    if TRANSIENT_TRANSCRIPTION_RETRY_JITTER_MS == 0 {
        return 0;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64 ^ attempt as u64;
    for byte in operation_id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash % (TRANSIENT_TRANSCRIPTION_RETRY_JITTER_MS + 1)
}

/// The correction context of one turn: the full dictionary block plus the
/// earlier same-source turns. The transcription call never receives it.
fn source_turn_correction_context(
    dictionary_context: Option<&str>,
    source: &str,
    turn_index: i64,
    cached_candidates: &[TranscriptCandidate],
    completed_by_source: &HashMap<String, Vec<SourceTranscriptInput>>,
) -> Option<String> {
    let mut previous = cached_candidates
        .iter()
        .map(|candidate| &candidate.input)
        .chain(
            completed_by_source
                .get(source)
                .into_iter()
                .flat_map(|inputs| inputs.iter()),
        )
        .filter(|input| {
            input.source == source && input.turn_index.is_some_and(|index| index < turn_index)
        })
        .cloned()
        .collect::<Vec<_>>();
    previous.sort_by_key(|input| input.turn_index.unwrap_or(i64::MAX));
    merge_transcription_context(
        dictionary_context,
        build_transcription_context(&previous).as_deref(),
    )
}

async fn sink_and_record_completed_turn(
    event: CompletedTurnTranscription,
    result_sink: Option<&TurnResultSink>,
    completed_by_source: &mut HashMap<String, Vec<SourceTranscriptInput>>,
    outcome: &mut TranscriptionOutcome,
) -> Result<(), AppError> {
    if let Some(sink) = result_sink {
        sink(event.clone()).await?;
    }
    match event.result {
        TurnTranscriptionResult::Candidate(candidate) => {
            completed_by_source
                .entry(candidate.input.source.clone())
                .or_default()
                .push(candidate.input.clone());
            outcome.candidates.push(candidate);
        }
        TurnTranscriptionResult::Failure(failure) => {
            completed_by_source
                .entry(failure.input.source.clone())
                .or_default()
                .push(failure.input.clone());
            outcome.failures.push(failure);
        }
    }
    Ok(())
}

fn stop_stream_after_error(
    terminal_error: &mut Option<AppError>,
    receiver: &mut tokio::sync::mpsc::Receiver<Result<PreparedTurn, AppError>>,
    ready_by_source: &mut HashMap<String, VecDeque<PreparedTurn>>,
    error: AppError,
) {
    if terminal_error.is_none() {
        *terminal_error = Some(error);
    }
    receiver.close();
    ready_by_source.clear();
}

fn ready_turn_count(ready_by_source: &HashMap<String, VecDeque<PreparedTurn>>) -> usize {
    ready_by_source.values().map(VecDeque::len).sum()
}

/// Silence between packed turns, so the provider ends a segment at every
/// turn boundary.
const PACK_GAP_MS: i64 = 500;
/// Prepared turns the stream may hold while it fills packs: short turns, so
/// several hundred fit in one pack.
const PACKED_READY_TURN_CAPACITY: usize = 256;

type PackTranscriber = ProviderCall<TimedTranscription>;

/// How the stream packs turns: the timed transcriber, each prepared turn's
/// packable audio length (by schedule index), and whether a lane should wait
/// for more turns before it sends a pack that is not full yet.
struct TurnPacking<'a> {
    transcriber: &'a PackTranscriber,
    durations_ms: &'a HashMap<usize, Option<i64>>,
    hold: bool,
}

impl TurnPacking<'_> {
    fn duration_ms(&self, prepared: &PreparedTurn) -> Option<i64> {
        self.durations_ms
            .get(&prepared.schedule_index)
            .copied()
            .flatten()
    }

    /// The packable audio at the front of a lane, gaps included.
    fn front_run_ms(&self, lane: &VecDeque<PreparedTurn>) -> i64 {
        let mut total = 0_i64;
        for prepared in lane {
            let Some(duration_ms) = self.duration_ms(prepared) else {
                break;
            };
            total = total.saturating_add(duration_ms + PACK_GAP_MS);
        }
        total
    }
}

/// The audio length of a prepared turn a pack can carry: a normalized WAV of
/// one ordinary turn no longer than the final span. `None` sends it alone.
fn packable_duration_ms(job: &TurnTranscriptionJob) -> Option<i64> {
    if job.covers_full_source || job.source_fallback {
        return None;
    }
    let reader = hound::WavReader::open(&job.audio_path).ok()?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != 16_000
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return None;
    }
    let duration_ms = i64::from(reader.duration()) * 1000 / 16_000;
    (duration_ms > 0 && duration_ms <= FINAL_TRANSCRIPTION_SPAN_MS).then_some(duration_ms)
}

type TurnJoinSet =
    tokio::task::JoinSet<(String, Result<Vec<CompletedTurnTranscription>, AppError>)>;

#[allow(clippy::too_many_arguments)]
fn launch_ready_source_turns(
    ready_by_source: &mut HashMap<String, VecDeque<PreparedTurn>>,
    active_sources: &mut HashSet<String>,
    join_set: &mut TurnJoinSet,
    cached_candidates: &[TranscriptCandidate],
    completed_by_source: &HashMap<String, Vec<SourceTranscriptInput>>,
    provider: &str,
    title: &str,
    dictionary: Option<&TranscriptionDictionary>,
    transcriber: &TurnTranscriber,
    job_claimer: Option<&TurnJobClaimer>,
    max_concurrency: usize,
    packing: Option<&TurnPacking<'_>>,
) {
    while join_set.len() < max_concurrency {
        let next_source = ready_by_source
            .iter()
            .filter(|(source, lane)| !lane.is_empty() && !active_sources.contains(*source))
            // A lane whose front can be packed waits for a full pack while
            // more turns may still arrive.
            .filter(|(_, lane)| {
                let Some(packing) = packing else {
                    return true;
                };
                let front_packable = lane
                    .front()
                    .and_then(|front| packing.duration_ms(front))
                    .is_some();
                !packing.hold
                    || !front_packable
                    || packing.front_run_ms(lane) >= FINAL_TRANSCRIPTION_SPAN_MS
            })
            .filter_map(|(source, lane)| {
                lane.front()
                    .map(|prepared| (prepared.schedule_index, source.clone()))
            })
            .min_by_key(|(schedule_index, _)| *schedule_index)
            .map(|(_, source)| source);
        let Some(source) = next_source else {
            break;
        };
        let lane = ready_by_source
            .get_mut(&source)
            .expect("selected source must have a lane");
        if let Some(packing) = packing {
            let mut pack = Vec::new();
            let mut pack_ms = 0_i64;
            while let Some(duration_ms) = lane.front().and_then(|front| packing.duration_ms(front))
            {
                let next_ms = pack_ms + if pack.is_empty() { 0 } else { PACK_GAP_MS } + duration_ms;
                if next_ms > FINAL_TRANSCRIPTION_SPAN_MS {
                    break;
                }
                pack_ms = next_ms;
                pack.push(lane.pop_front().expect("front was checked"));
            }
            match pack.len() {
                0 => {}
                // One turn goes alone, on the ordinary path.
                1 => lane.push_front(pack.pop().expect("one packed turn")),
                _ => {
                    active_sources.insert(source.clone());
                    let task = transcribe_turn_pack(
                        pack.into_iter().map(|prepared| prepared.job).collect(),
                        provider.to_string(),
                        title.to_string(),
                        dictionary.cloned(),
                        cached_candidates.to_vec(),
                        completed_by_source
                            .get(&source)
                            .map(|completed| HashMap::from([(source.clone(), completed.clone())]))
                            .unwrap_or_default(),
                        Arc::clone(packing.transcriber),
                        job_claimer.cloned(),
                    );
                    join_set.spawn(async move { (source, task.await) });
                    continue;
                }
            }
        }
        let prepared = ready_by_source
            .get_mut(&source)
            .and_then(VecDeque::pop_front)
            .expect("selected source must have a prepared turn");
        let correction_context = source_turn_correction_context(
            dictionary.map(|dictionary| dictionary.context.as_str()),
            &source,
            prepared.job.turn_index,
            cached_candidates,
            completed_by_source,
        );
        let asr_hint = dictionary.and_then(|dictionary| dictionary.asr_hint.clone());
        active_sources.insert(source.clone());
        let provider = provider.to_string();
        let title = title.to_string();
        let transcriber = Arc::clone(transcriber);
        let job_claimer = job_claimer.cloned();
        join_set.spawn(async move {
            let result = transcribe_one_turn_job(
                prepared.job,
                provider,
                title,
                asr_hint,
                correction_context,
                transcriber,
                job_claimer,
            )
            .await;
            (source, result.map(|event| vec![event]))
        });
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn transcribe_prepared_turn_stream(
    receiver: tokio::sync::mpsc::Receiver<Result<PreparedTurn, AppError>>,
    total_jobs: usize,
    provider: String,
    title: String,
    dictionary: Option<TranscriptionDictionary>,
    cached_candidates: &[TranscriptCandidate],
    transcriber: TurnTranscriber,
    job_claimer: Option<TurnJobClaimer>,
    result_sink: Option<TurnResultSink>,
    max_concurrency: usize,
) -> Result<TranscriptionOutcome, AppError> {
    transcribe_prepared_turn_stream_with_packs(
        receiver,
        total_jobs,
        provider,
        title,
        dictionary,
        cached_candidates,
        transcriber,
        None,
        job_claimer,
        result_sink,
        max_concurrency,
    )
    .await
}

/// The turn stream; with `pack_transcriber`, consecutive turns of one Source
/// go as packs of up to the final span, one request each.
#[allow(clippy::too_many_arguments)]
async fn transcribe_prepared_turn_stream_with_packs(
    mut receiver: tokio::sync::mpsc::Receiver<Result<PreparedTurn, AppError>>,
    total_jobs: usize,
    provider: String,
    title: String,
    dictionary: Option<TranscriptionDictionary>,
    cached_candidates: &[TranscriptCandidate],
    transcriber: TurnTranscriber,
    pack_transcriber: Option<PackTranscriber>,
    job_claimer: Option<TurnJobClaimer>,
    result_sink: Option<TurnResultSink>,
    max_concurrency: usize,
) -> Result<TranscriptionOutcome, AppError> {
    let max_concurrency = max_concurrency.max(1);
    let ready_capacity = if pack_transcriber.is_some() {
        PACKED_READY_TURN_CAPACITY
    } else {
        PREPARED_TURN_CHANNEL_CAPACITY
    };
    let mut pack_durations_ms = HashMap::<usize, Option<i64>>::new();
    let mut join_set = TurnJoinSet::new();
    let mut ready_by_source = HashMap::<String, VecDeque<PreparedTurn>>::new();
    let mut active_sources = HashSet::<String>::new();
    let mut completed_by_source = HashMap::<String, Vec<SourceTranscriptInput>>::new();
    let mut seen_schedule_indices = HashSet::<usize>::new();
    let mut outcome = TranscriptionOutcome::default();
    let mut terminal_error = None::<AppError>;
    let mut receiver_open = true;
    let mut received_count = 0_usize;

    if total_jobs == 0 {
        return Ok(outcome);
    }

    loop {
        if terminal_error.is_none() {
            let packing = pack_transcriber.as_ref().map(|transcriber| TurnPacking {
                transcriber,
                durations_ms: &pack_durations_ms,
                hold: receiver_open
                    && received_count < total_jobs
                    && join_set.len() + ready_turn_count(&ready_by_source) < ready_capacity,
            });
            launch_ready_source_turns(
                &mut ready_by_source,
                &mut active_sources,
                &mut join_set,
                cached_candidates,
                &completed_by_source,
                &provider,
                &title,
                dictionary.as_ref(),
                &transcriber,
                job_claimer.as_ref(),
                max_concurrency,
                packing.as_ref(),
            );
        }
        if terminal_error.is_some() && join_set.is_empty() {
            break;
        }
        if terminal_error.is_none()
            && received_count == total_jobs
            && join_set.is_empty()
            && ready_turn_count(&ready_by_source) == 0
        {
            break;
        }

        let can_receive = terminal_error.is_none()
            && receiver_open
            && received_count < total_jobs
            && join_set.len() + ready_turn_count(&ready_by_source) < ready_capacity;
        let can_join = !join_set.is_empty();

        if !can_receive && !can_join {
            if terminal_error.is_none() {
                stop_stream_after_error(
                    &mut terminal_error,
                    &mut receiver,
                    &mut ready_by_source,
                    AppError::new(
                        "audio_turn_failed",
                        "turn preparation ended before every job was received",
                    ),
                );
                continue;
            }
            break;
        }

        tokio::select! {
            biased;
            prepared = receiver.recv(), if can_receive => {
                match prepared {
                    Some(Ok(prepared)) => {
                        if !seen_schedule_indices.insert(prepared.schedule_index) {
                            stop_stream_after_error(
                                &mut terminal_error,
                                &mut receiver,
                                &mut ready_by_source,
                                AppError::new("audio_turn_failed", "duplicate prepared turn"),
                            );
                            continue;
                        }
                        received_count += 1;
                        if pack_transcriber.is_some() {
                            pack_durations_ms
                                .insert(prepared.schedule_index, packable_duration_ms(&prepared.job));
                        }
                        ready_by_source
                            .entry(prepared.job.source.clone())
                            .or_default()
                            .push_back(prepared);
                    }
                    Some(Err(error)) => {
                        stop_stream_after_error(
                            &mut terminal_error,
                            &mut receiver,
                            &mut ready_by_source,
                            error,
                        );
                    }
                    None => {
                        receiver_open = false;
                        if received_count < total_jobs {
                            stop_stream_after_error(
                                &mut terminal_error,
                                &mut receiver,
                                &mut ready_by_source,
                                AppError::new(
                                    "audio_turn_failed",
                                    "turn preparation ended before every job was received",
                                ),
                            );
                        }
                    }
                }
            }
            joined = join_set.join_next(), if can_join => {
                let joined = joined.expect("join branch requires a nonempty JoinSet");
                match joined {
                    Ok((source, Ok(events))) => {
                        active_sources.remove(&source);
                        for event in events {
                            let pause = event.pause.clone();
                            let accepted = sink_and_record_completed_turn(
                                event,
                                result_sink.as_ref(),
                                &mut completed_by_source,
                                &mut outcome,
                            )
                            .await;
                            // A rate-limit pause ends the run once its turns
                            // are recorded: the turns still waiting would hit
                            // the same limit, so they stay pending for the
                            // resume.
                            if let Some(error) = accepted.err().or(pause) {
                                stop_stream_after_error(
                                    &mut terminal_error,
                                    &mut receiver,
                                    &mut ready_by_source,
                                    error,
                                );
                            }
                        }
                    }
                    Ok((source, Err(error))) => {
                        active_sources.remove(&source);
                        stop_stream_after_error(
                            &mut terminal_error,
                            &mut receiver,
                            &mut ready_by_source,
                            error,
                        );
                    }
                    Err(error) => {
                        stop_stream_after_error(
                            &mut terminal_error,
                            &mut receiver,
                            &mut ready_by_source,
                            AppError::new("transcription_failed", error.to_string()),
                        );
                    }
                }
            }
        }

        debug_assert!(join_set.len() <= max_concurrency);
        debug_assert!(active_sources.len() <= max_concurrency);
    }

    if let Some(error) = terminal_error {
        return Err(error);
    }
    if received_count != total_jobs {
        return Err(AppError::new(
            "audio_turn_failed",
            "turn preparation ended before every job was received",
        ));
    }
    sort_transcription_outcome(&mut outcome);
    Ok(outcome)
}

fn source_lane_needs_full_source_fallback(
    plan: &SourceFallbackPlan,
    candidates: &[TranscriptCandidate],
    cached_candidates: &[TranscriptCandidate],
) -> bool {
    let successful = candidates
        .iter()
        .chain(cached_candidates.iter())
        .filter(|candidate| {
            candidate.input.source == plan.source
                && candidate.input.valid
                && !candidate.input.text.trim().is_empty()
        })
        .collect::<Vec<_>>();
    if successful.is_empty() {
        return true;
    }
    let transcribed_ms = successful.iter().fold(0_i64, |total, candidate| {
        total.saturating_add(span_ms(
            candidate.input.start_ms.unwrap_or_default(),
            candidate.input.end_ms.unwrap_or_default(),
        ))
    });
    transcript_coverage_warning(plan.detected_ms, transcribed_ms)
}

#[allow(clippy::too_many_arguments)]
async fn transcribe_source_fallbacks(
    outcome: &mut TranscriptionOutcome,
    fallback_plans: Vec<SourceFallbackPlan>,
    cached_candidates: &[TranscriptCandidate],
    provider: &str,
    title: &str,
    dictionary: Option<&TranscriptionDictionary>,
    fallback_preparer: &SourceFallbackPreparer,
    transcriber: &TurnTranscriber,
    job_claimer: Option<&TurnJobClaimer>,
    result_sink: Option<&TurnResultSink>,
) -> Result<(), AppError> {
    for plan in fallback_plans {
        if !plan.eligible()
            || !source_lane_needs_full_source_fallback(
                &plan,
                &outcome.candidates,
                cached_candidates,
            )
        {
            continue;
        }
        let fallback_preparer = Arc::clone(fallback_preparer);
        let job = tokio::task::spawn_blocking(move || fallback_preparer(plan))
            .await
            .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))??;
        let event = transcribe_one_turn_job(
            job,
            provider.to_string(),
            title.to_string(),
            dictionary.and_then(|dictionary| dictionary.asr_hint.clone()),
            dictionary.map(|dictionary| dictionary.context.clone()),
            Arc::clone(transcriber),
            job_claimer.cloned(),
        )
        .await?;
        if let Some(sink) = result_sink {
            sink(event.clone()).await?;
        }
        if let Some(pause) = event.pause {
            return Err(pause);
        }
        match event.result {
            TurnTranscriptionResult::Candidate(candidate) => {
                let source = candidate.input.source.clone();
                outcome
                    .candidates
                    .retain(|existing| existing.input.source != source);
                outcome
                    .failures
                    .retain(|failure| failure.input.source != source);
                outcome.replaced_sources.insert(source);
                outcome.candidates.push(candidate);
            }
            TurnTranscriptionResult::Failure(failure) => outcome.failures.push(failure),
        }
    }
    sort_transcription_outcome(outcome);
    Ok(())
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn prepare_and_transcribe_turn_jobs_bounded(
    descriptors: Vec<TurnPreparationJob>,
    fallback_plans: Vec<SourceFallbackPlan>,
    cached_candidates: &[TranscriptCandidate],
    provider: String,
    title: String,
    dictionary: Option<TranscriptionDictionary>,
    turn_preparer: TurnPreparer,
    fallback_preparer: SourceFallbackPreparer,
    transcriber: TurnTranscriber,
    job_claimer: Option<TurnJobClaimer>,
    result_sink: Option<TurnResultSink>,
    max_concurrency: usize,
    timing: ProcessingTiming,
) -> Result<TurnPipelineResult, TurnPipelineFailure> {
    prepare_and_transcribe_turn_jobs_with_packs(
        descriptors,
        fallback_plans,
        cached_candidates,
        provider,
        title,
        dictionary,
        turn_preparer,
        fallback_preparer,
        transcriber,
        None,
        job_claimer,
        result_sink,
        max_concurrency,
        timing,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn prepare_and_transcribe_turn_jobs_with_packs(
    descriptors: Vec<TurnPreparationJob>,
    fallback_plans: Vec<SourceFallbackPlan>,
    cached_candidates: &[TranscriptCandidate],
    provider: String,
    title: String,
    dictionary: Option<TranscriptionDictionary>,
    turn_preparer: TurnPreparer,
    fallback_preparer: SourceFallbackPreparer,
    transcriber: TurnTranscriber,
    pack_transcriber: Option<PackTranscriber>,
    job_claimer: Option<TurnJobClaimer>,
    result_sink: Option<TurnResultSink>,
    max_concurrency: usize,
    timing: ProcessingTiming,
) -> Result<TurnPipelineResult, TurnPipelineFailure> {
    let total_jobs = descriptors.len();
    let (receiver, producer) = spawn_turn_preparation(descriptors, turn_preparer, timing);
    let consumer_result = transcribe_prepared_turn_stream_with_packs(
        receiver,
        total_jobs,
        provider.clone(),
        title.clone(),
        dictionary.clone(),
        cached_candidates,
        Arc::clone(&transcriber),
        pack_transcriber,
        job_claimer.clone(),
        result_sink.clone(),
        max_concurrency,
    )
    .await;
    let producer_result = producer.await;
    let preparation = match producer_result {
        Ok(report) => report,
        Err(join_error) => {
            let error = consumer_result
                .err()
                .unwrap_or_else(|| AppError::new("audio_turn_failed", join_error.to_string()));
            return Err(TurnPipelineFailure {
                error,
                preparation: None,
            });
        }
    };
    let mut outcome = match consumer_result {
        Ok(outcome) => outcome,
        Err(error) => {
            return Err(TurnPipelineFailure {
                error,
                preparation: Some(Box::new(preparation)),
            });
        }
    };
    if let Some(error) = preparation.error.clone() {
        return Err(TurnPipelineFailure {
            error,
            preparation: Some(Box::new(preparation)),
        });
    }
    if let Err(error) = transcribe_source_fallbacks(
        &mut outcome,
        fallback_plans,
        cached_candidates,
        &provider,
        &title,
        dictionary.as_ref(),
        &fallback_preparer,
        &transcriber,
        job_claimer.as_ref(),
        result_sink.as_ref(),
    )
    .await
    {
        return Err(TurnPipelineFailure {
            error,
            preparation: Some(Box::new(preparation)),
        });
    }
    Ok(TurnPipelineResult {
        outcome,
        preparation,
    })
}

#[cfg(test)]
async fn transcribe_turn_jobs_by_source_lane(
    jobs: Vec<TurnTranscriptionJob>,
    provider: String,
    title: String,
    dictionary: Option<TranscriptionDictionary>,
    transcriber: TurnTranscriber,
) -> Result<TranscriptionOutcome, AppError> {
    transcribe_turn_jobs_bounded(
        jobs,
        Vec::new(),
        &[],
        provider,
        title,
        dictionary,
        Arc::new(prepare_source_fallback),
        transcriber,
        None,
        DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
    )
    .await
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
async fn transcribe_turn_jobs_bounded(
    jobs: Vec<TurnTranscriptionJob>,
    fallback_plans: Vec<SourceFallbackPlan>,
    cached_candidates: &[TranscriptCandidate],
    provider: String,
    title: String,
    dictionary: Option<TranscriptionDictionary>,
    fallback_preparer: SourceFallbackPreparer,
    transcriber: TurnTranscriber,
    result_sink: Option<TurnResultSink>,
    max_concurrency: usize,
) -> Result<TranscriptionOutcome, AppError> {
    let total_jobs = jobs.len();
    let (sender, receiver) = tokio::sync::mpsc::channel(total_jobs.max(1));
    for (schedule_index, job) in jobs.into_iter().enumerate() {
        sender
            .send(Ok(PreparedTurn {
                schedule_index,
                job,
            }))
            .await
            .map_err(|_| AppError::new("audio_turn_failed", "prepared turn channel closed"))?;
    }
    drop(sender);
    let mut outcome = transcribe_prepared_turn_stream(
        receiver,
        total_jobs,
        provider.clone(),
        title.clone(),
        dictionary.clone(),
        cached_candidates,
        Arc::clone(&transcriber),
        None,
        result_sink.clone(),
        max_concurrency,
    )
    .await?;

    transcribe_source_fallbacks(
        &mut outcome,
        fallback_plans,
        cached_candidates,
        &provider,
        &title,
        dictionary.as_ref(),
        &fallback_preparer,
        &transcriber,
        None,
        result_sink.as_ref(),
    )
    .await?;
    Ok(outcome)
}

fn sort_transcription_outcome(outcome: &mut TranscriptionOutcome) {
    outcome.candidates.sort_by(|left, right| {
        left.input
            .turn_index
            .unwrap_or(i64::MAX)
            .cmp(&right.input.turn_index.unwrap_or(i64::MAX))
            .then_with(|| {
                left.input
                    .start_ms
                    .unwrap_or(i64::MAX)
                    .cmp(&right.input.start_ms.unwrap_or(i64::MAX))
            })
    });
    outcome.failures.sort_by(|left, right| {
        left.input
            .turn_index
            .unwrap_or(i64::MAX)
            .cmp(&right.input.turn_index.unwrap_or(i64::MAX))
            .then_with(|| {
                left.input
                    .start_ms
                    .unwrap_or(i64::MAX)
                    .cmp(&right.input.start_ms.unwrap_or(i64::MAX))
            })
    });
}

/// `asr_hint` is the transcription `prompt`; `correction_context` is the full
/// dictionary and previous same-source transcript the correction engine gets.
async fn transcribe_one_turn_job(
    job: TurnTranscriptionJob,
    provider: String,
    title: String,
    asr_hint: Option<String>,
    correction_context: Option<String>,
    transcriber: TurnTranscriber,
    job_claimer: Option<TurnJobClaimer>,
) -> Result<CompletedTurnTranscription, AppError> {
    let started = Instant::now();
    let replaces_source = job.source_fallback;
    let job_id = job.durable_job.as_ref().map(|durable| durable.id.clone());
    if let Some(job_id) = job_id.as_ref() {
        let claimer = job_claimer.as_ref().ok_or_else(|| {
            AppError::new(
                "transcription_job_claim_missing",
                "Durable transcription was started without a claim handler.",
            )
        })?;
        claimer(job_id.clone()).await?;
    }
    let operation_id = job
        .durable_job
        .as_ref()
        .map(|durable| durable.operation_id.clone())
        .unwrap_or_else(|| {
            if job.source_fallback {
                source_fallback_operation_id(&job)
            } else {
                turn_operation_id(&job)
            }
        });
    let max_chunk_ms = job
        .durable_job
        .as_ref()
        .and_then(|durable| durable.max_chunk_ms)
        .or_else(|| short_full_source_chunk_ms(&job));
    let transcription = match transcribe_prepared_audio(
        Arc::clone(&transcriber),
        TranscribePreparedAudioRequest {
            provider: provider.clone(),
            audio_path: job.audio_path.clone(),
            temp_dir: job.temp_dir.clone(),
            chunk_stem: format!("{}-turn-{}", job.source, job.turn_index),
            title,
            asr_hint,
            operation_id,
            max_chunk_ms,
        },
    )
    .await
    {
        Ok(transcription) => transcription,
        Err(error) => {
            return Ok(failed_turn_event(
                job,
                job_id,
                &error,
                elapsed_ms(started),
                replaces_source,
            ));
        }
    };
    tracing::debug!(
        source = %job.source,
        turn_index = job.turn_index,
        chunk_count = transcription.chunk_outcomes.len(),
        "saved-audio span transcription completed"
    );
    let transcript = maybe_post_process_note_transcript(
        &provider,
        transcription.transcript,
        correction_context.as_deref(),
    )
    .await;
    Ok(transcribed_turn_event(
        job,
        job_id,
        transcript,
        elapsed_ms(started),
        replaces_source,
    ))
}

/// A turn that produced no transcript. A rate-limit pause rides along, so
/// the stream stops after recording it.
fn failed_turn_event(
    job: TurnTranscriptionJob,
    job_id: Option<String>,
    error: &AppError,
    duration_ms: i64,
    replaces_source: bool,
) -> CompletedTurnTranscription {
    let pause = (error.code == TRANSCRIPTION_RATE_LIMITED_CODE).then(|| error.clone());
    let warning = user_facing_transcription_failure_message(
        &error.code,
        &error.message,
        job.recorded_silence,
        job.source.as_str(),
    );
    let input = SourceTranscriptInput {
        source: job.source,
        text: String::new(),
        valid: false,
        warning: Some(warning),
        recorded_silence: job.recorded_silence,
        start_ms: Some(job.start_ms),
        end_ms: Some(job.end_ms),
        turn_index: Some(job.turn_index),
    };
    CompletedTurnTranscription {
        job_id,
        result: TurnTranscriptionResult::Failure(FailedTranscriptCandidate {
            artifact_id: job.artifact_id,
            input,
        }),
        duration_ms,
        replaces_source,
        pause,
    }
}

fn transcribed_turn_event(
    job: TurnTranscriptionJob,
    job_id: Option<String>,
    transcript: TranscriptionProviderResult,
    duration_ms: i64,
    replaces_source: bool,
) -> CompletedTurnTranscription {
    let input = SourceTranscriptInput {
        source: job.source,
        text: transcript.text,
        valid: true,
        warning: None,
        recorded_silence: false,
        start_ms: Some(job.start_ms),
        end_ms: Some(job.end_ms),
        turn_index: Some(job.turn_index),
    };
    CompletedTurnTranscription {
        job_id,
        result: TurnTranscriptionResult::Candidate(TranscriptCandidate {
            artifact_id: job.artifact_id,
            language: transcript.language,
            input,
        }),
        duration_ms,
        replaces_source,
        pause: None,
    }
}

/// Writes `jobs`' normalized turn WAVs back to back into one WAV, with
/// [`PACK_GAP_MS`] of silence between turns, and returns each turn's span in
/// the pack.
fn write_turn_pack(
    jobs: &[TurnTranscriptionJob],
    path: &Path,
) -> Result<Vec<(i64, i64)>, AppError> {
    let failed = |error: hound::Error| AppError::new("audio_chunk_failed", error.to_string());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let gap_samples = (PACK_GAP_MS * 16) as usize;
    let mut writer = hound::WavWriter::create(path, spec).map_err(failed)?;
    let mut spans = Vec::with_capacity(jobs.len());
    let mut written = 0_usize;
    for (index, job) in jobs.iter().enumerate() {
        if index > 0 {
            for _ in 0..gap_samples {
                writer.write_sample(0_i16).map_err(failed)?;
            }
            written += gap_samples;
        }
        let start = written;
        let mut reader = hound::WavReader::open(&job.audio_path).map_err(failed)?;
        if reader.spec() != spec {
            return Err(AppError::new(
                "audio_chunk_failed",
                "A packed turn is not normalized 16 kHz mono PCM16.",
            ));
        }
        for sample in reader.samples::<i16>() {
            writer
                .write_sample(sample.map_err(failed)?)
                .map_err(failed)?;
            written += 1;
        }
        spans.push(((start / 16) as i64, (written / 16) as i64));
    }
    writer.finalize().map_err(failed)?;
    Ok(spans)
}

/// Each turn's text: the segments whose midpoint falls in the turn's span or
/// the half gap around it.
fn split_pack_segments(spans: &[(i64, i64)], segments: &[TranscriptSegment]) -> Vec<String> {
    let mut texts = vec![Vec::new(); spans.len()];
    for segment in segments {
        let midpoint = (segment.start_ms + segment.end_ms) / 2;
        let index = spans
            .iter()
            .position(|(_, end_ms)| midpoint < end_ms + PACK_GAP_MS / 2)
            .unwrap_or(spans.len().saturating_sub(1));
        if let Some(text) = texts.get_mut(index) {
            text.push(segment.text.as_str());
        }
    }
    texts.into_iter().map(|parts| parts.join(" ")).collect()
}

/// One request for several consecutive turns of one Source: the turns'
/// audio goes as one pack and comes back split by segment time, then each
/// turn is corrected and recorded as if it had been sent alone. A failure
/// fails every turn of the pack; the next run sends them again.
#[allow(clippy::too_many_arguments)]
async fn transcribe_turn_pack(
    jobs: Vec<TurnTranscriptionJob>,
    provider: String,
    title: String,
    dictionary: Option<TranscriptionDictionary>,
    cached_candidates: Vec<TranscriptCandidate>,
    mut completed_by_source: HashMap<String, Vec<SourceTranscriptInput>>,
    pack_transcriber: PackTranscriber,
    job_claimer: Option<TurnJobClaimer>,
) -> Result<Vec<CompletedTurnTranscription>, AppError> {
    let started = Instant::now();
    for job in &jobs {
        if let Some(durable) = job.durable_job.as_ref() {
            let claimer = job_claimer.as_ref().ok_or_else(|| {
                AppError::new(
                    "transcription_job_claim_missing",
                    "Durable transcription was started without a claim handler.",
                )
            })?;
            claimer(durable.id.clone()).await?;
        }
    }
    let job_ids = jobs
        .iter()
        .map(|job| job.durable_job.as_ref().map(|durable| durable.id.clone()))
        .collect::<Vec<_>>();
    let first = &jobs[0];
    let operation_id = format!(
        "{}-pack-{}",
        first
            .durable_job
            .as_ref()
            .map(|durable| durable.operation_id.clone())
            .unwrap_or_else(|| turn_operation_id(first)),
        jobs.len()
    );
    let pack_path = first
        .temp_dir
        .join(format!("{}-pack-{}.wav", first.source, first.turn_index));
    let result = match write_turn_pack(&jobs, &pack_path) {
        Ok(spans) => pack_transcriber(TranscriptionRequest {
            provider: provider.clone(),
            audio_path: pack_path.clone(),
            title,
            context: dictionary
                .as_ref()
                .and_then(|dictionary| dictionary.asr_hint.clone()),
            language: crate::dictation::configured_transcription_language(),
            operation_id: Some(operation_id),
            preview: false,
        })
        .await
        .map(|timed| (spans, timed)),
        Err(error) => Err(error),
    };
    let _ = std::fs::remove_file(&pack_path);
    let (spans, timed) = match result {
        Ok(result) => result,
        Err(error) => {
            return Ok(jobs
                .into_iter()
                .zip(job_ids)
                .map(|(job, job_id)| {
                    failed_turn_event(job, job_id, &error, elapsed_ms(started), false)
                })
                .collect());
        }
    };
    let texts = split_pack_segments(&spans, &timed.segments);
    let mut events = Vec::with_capacity(jobs.len());
    for ((job, job_id), text) in jobs.into_iter().zip(job_ids).zip(texts) {
        if text.trim().is_empty() {
            let no_speech = AppError::new("no_speech", "no_speech");
            events.push(failed_turn_event(
                job,
                job_id,
                &no_speech,
                elapsed_ms(started),
                false,
            ));
            continue;
        }
        let correction_context = source_turn_correction_context(
            dictionary
                .as_ref()
                .map(|dictionary| dictionary.context.as_str()),
            &job.source,
            job.turn_index,
            &cached_candidates,
            &completed_by_source,
        );
        let transcript = maybe_post_process_note_transcript(
            &provider,
            TranscriptionProviderResult {
                text,
                language: timed.language.clone(),
                provider: provider.clone(),
            },
            correction_context.as_deref(),
        )
        .await;
        let event = transcribed_turn_event(job, job_id, transcript, elapsed_ms(started), false);
        if let TurnTranscriptionResult::Candidate(candidate) = &event.result {
            completed_by_source
                .entry(candidate.input.source.clone())
                .or_default()
                .push(candidate.input.clone());
        }
        events.push(event);
    }
    Ok(events)
}

async fn persist_turn_transcription_event(
    repos: &Repositories,
    note_id: &str,
    session_id: &str,
    event: CompletedTurnTranscription,
    timeline: FirstEventTimeline,
) -> Result<(), AppError> {
    let job_id = event.job_id.clone().ok_or_else(|| {
        AppError::new(
            "transcription_job_missing",
            "A saved-audio result did not have a durable job identity.",
        )
    })?;
    let (artifact_id, source, start_ms, end_ms, turn_index, status) = match &event.result {
        TurnTranscriptionResult::Candidate(candidate) => (
            candidate.artifact_id.clone(),
            candidate.input.source.clone(),
            candidate.input.start_ms.unwrap_or_default(),
            candidate.input.end_ms.unwrap_or_default(),
            candidate.input.turn_index.unwrap_or_default(),
            "succeeded",
        ),
        TurnTranscriptionResult::Failure(failure) => (
            failure.artifact_id.clone(),
            failure.input.source.clone(),
            failure.input.start_ms.unwrap_or_default(),
            failure.input.end_ms.unwrap_or_default(),
            failure.input.turn_index.unwrap_or_default(),
            "failed",
        ),
    };
    let persistence_started = Instant::now();
    let transcript_id = match event.result {
        TurnTranscriptionResult::Candidate(candidate) => {
            let result = repos
                .complete_note_transcription_job_success(
                    &job_id,
                    &candidate.input.text,
                    candidate.language,
                )
                .await;
            let row = match result {
                Ok(row) => row,
                Err(error) => {
                    let _ = repos
                        .complete_note_transcription_job_failure(&job_id, &error.to_string())
                        .await;
                    return Err(AppError::from(error));
                }
            };
            timeline.mark_first_persisted();
            tracing::info!(
                %session_id,
                source = %candidate.input.source,
                turn_index = candidate.input.turn_index.unwrap_or_default(),
                transcript_id = %row.id,
                replaces_source = event.replaces_source,
                "persisted durable saved-audio transcript"
            );
            Some(row.id)
        }
        TurnTranscriptionResult::Failure(failure) => {
            let warning = failure
                .input
                .warning
                .as_deref()
                .unwrap_or("Source did not produce a usable transcript.");
            let updated = repos
                .complete_note_transcription_job_failure(&job_id, warning)
                .await?;
            if !updated {
                return Err(AppError::new(
                    "transcription_job_invalid_state",
                    "The durable transcription job was no longer running.",
                ));
            }
            persist_active_note_transcription_warning(repos, note_id, &failure.input).await?;
            None
        }
    };

    // Checkpoints are diagnostic. The durable job transition and transcript
    // projection above are the load-bearing transaction, so telemetry failure
    // must never turn a committed transcript into a user-visible failure.
    if let Err(error) = repos
        .add_source_checkpoint(
            session_id,
            Some(artifact_id.as_str()),
            Some(source.as_str()),
            "transcription_request",
            Some(
                serde_json::json!({
                    "durationMs": event.duration_ms,
                    "status": status,
                    "turnIndex": turn_index,
                    "startMs": start_ms,
                    "endMs": end_ms,
                    "jobId": job_id,
                })
                .to_string(),
            ),
        )
        .await
    {
        tracing::warn!(%session_id, %error, "failed to persist transcription_request checkpoint");
    }
    if let Err(error) = repos
        .add_source_checkpoint(
            session_id,
            Some(artifact_id.as_str()),
            Some(source.as_str()),
            "transcript_persistence",
            Some(
                serde_json::json!({
                    "durationMs": elapsed_ms(persistence_started),
                    "status": status,
                    "turnIndex": turn_index,
                    "transcriptId": transcript_id,
                    "jobId": job_id,
                })
                .to_string(),
            ),
        )
        .await
    {
        tracing::warn!(%session_id, %error, "failed to persist transcript_persistence checkpoint");
    }
    Ok(())
}

async fn persist_active_note_transcription_warning(
    repos: &Repositories,
    note_id: &str,
    failure: &SourceTranscriptInput,
) -> Result<(), AppError> {
    let Some(warning) = failure
        .warning
        .as_deref()
        .map(str::trim)
        .filter(|warning| !warning.is_empty())
    else {
        return Ok(());
    };
    if failure.recorded_silence || is_no_speech_message(warning) {
        return Ok(());
    }
    let surfaced = repos
        .set_note_transcription_warning(note_id, warning)
        .await?;
    if surfaced {
        tracing::info!(%note_id, "surfaced live note transcription warning");
    }
    Ok(())
}

fn turn_operation_id(job: &TurnTranscriptionJob) -> String {
    format!(
        "{}-{}-{}-{}-turn-{}-fallback-false-{}",
        job.artifact_id,
        job.source,
        job.start_ms,
        job.end_ms,
        job.turn_index,
        NOTE_TRANSCRIPTION_PIPELINE_VERSION,
    )
}

fn source_fallback_operation_id(job: &TurnTranscriptionJob) -> String {
    format!(
        "{}-{}-{}-{}-turn-{}-fallback-true-{}",
        job.artifact_id,
        job.source,
        job.start_ms,
        job.end_ms,
        job.turn_index,
        NOTE_TRANSCRIPTION_PIPELINE_VERSION,
    )
}

fn source_failure_summary(failures: &[FailedTranscriptCandidate]) -> Option<String> {
    let mut by_source: Vec<(&str, Vec<&str>)> = Vec::new();
    let has_microphone_failure = failures
        .iter()
        .any(|failure| failure.input.source.as_str() == "microphone");
    for failure in failures {
        let source = failure.input.source.as_str();
        let message = failure
            .input
            .warning
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("Source did not produce a usable transcript.");
        if has_microphone_failure && source == "system" && is_no_speech_message(message) {
            continue;
        }
        if let Some((_, messages)) = by_source
            .iter_mut()
            .find(|(existing_source, _)| *existing_source == source)
        {
            if !messages.contains(&message) {
                messages.push(message);
            }
        } else {
            by_source.push((source, vec![message]));
        }
    }
    if by_source.is_empty() {
        return None;
    }
    Some(
        by_source
            .into_iter()
            .map(|(source, messages)| {
                let label = match source {
                    "system" => "System",
                    _ => "Microphone",
                };
                format!("{label}: {}", messages.join("; "))
            })
            .collect::<Vec<_>>()
            .join(" | "),
    )
}

fn visible_transcription_failures(
    failures: &[FailedTranscriptCandidate],
    has_valid_transcript: bool,
) -> Vec<FailedTranscriptCandidate> {
    failures
        .iter()
        .filter(|failure| {
            let warning = failure
                .input
                .warning
                .as_deref()
                .unwrap_or("Source did not produce a usable transcript.");
            should_record_source_failure(
                failure.input.source.as_str(),
                warning,
                has_valid_transcript,
            )
        })
        .cloned()
        .collect()
}

fn blocking_transcription_failure_summary(
    failures: &[FailedTranscriptCandidate],
) -> Option<String> {
    let blocking_failures = failures
        .iter()
        .filter(|failure| {
            let warning = failure
                .input
                .warning
                .as_deref()
                .unwrap_or("Source did not produce a usable transcript.");
            // Validation already proved this whole source was recorded as
            // silence. If another source produced a usable transcript, keep
            // the silent lane visible for diagnostics without blocking note
            // generation. The user-facing warning has already been expanded
            // beyond the provider's raw `no_speech` marker at this point.
            !failure.input.recorded_silence && !is_no_speech_message(warning)
        })
        .cloned()
        .collect::<Vec<_>>();
    source_failure_summary(&blocking_failures)
}

struct DroppedSource {
    artifact_id: String,
    source: String,
    max_rms: f32,
    active_ms: i64,
    longest_active_ms: i64,
}

struct SilentSystemDropOutcome {
    kept: Vec<SourceAudioForProcessing>,
    dropped: Vec<DroppedSource>,
}

#[cfg(test)]
fn drop_silent_system_sources(
    sources: Vec<SourceAudioForProcessing>,
) -> Vec<SourceAudioForProcessing> {
    partition_silent_system_sources(sources).kept
}

/// Remove system-audio sources whose track is effectively silent, but only when
/// another source remains to carry the recording. Keeping the last source — even
/// a silent one — preserves the "no speech" failure for system-only captures.
///
/// A system track is only dropped when it never sustains activity at the floor
/// used by the system lane's turn detector (`SYSTEM_DETECTION_MIN_RMS`). This
/// keeps quiet real speech while rejecting the isolated startup spike emitted
/// by a broken CoreAudio tap.
fn partition_silent_system_sources(
    sources: Vec<SourceAudioForProcessing>,
) -> SilentSystemDropOutcome {
    let has_other_source = sources
        .iter()
        .any(|(_, source, _, _)| source.as_str() != "system");
    if !has_other_source {
        return SilentSystemDropOutcome {
            kept: sources,
            dropped: Vec::new(),
        };
    }
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    for (artifact_id, source, path, recorded_silence) in sources {
        // Decode once. `None` means the file could not be read, which must mean
        // "keep" (matching the old read-failure semantics).
        let activity = if source.as_str() == "system" {
            crate::audio::turns::source_activity_evidence_with_floor(
                &path,
                crate::audio::turns::SYSTEM_DETECTION_MIN_RMS,
            )
        } else {
            None
        };
        let silent = activity.is_some_and(|evidence| !evidence.has_transcribable_activity);
        if silent {
            let activity = activity.expect("silent system activity was decoded");
            tracing::info!(
                %source,
                path = %path.display(),
                max_rms = activity.max_rms,
                active_ms = activity.active_ms,
                longest_active_ms = activity.longest_active_ms,
                "skipping silent system source — no transcribable audio"
            );
            dropped.push(DroppedSource {
                artifact_id,
                source,
                max_rms: activity.max_rms,
                active_ms: activity.active_ms,
                longest_active_ms: activity.longest_active_ms,
            });
        } else {
            kept.push((artifact_id, source, path, recorded_silence));
        }
    }
    SilentSystemDropOutcome { kept, dropped }
}

fn add_full_source_turns_for_missing_sources(
    sources: &[SourceAudioForProcessing],
    mut turns: Vec<AudioTurn>,
    echo_rejection: &EchoRejectionReport,
) -> Vec<AudioTurn> {
    for (artifact_id, source, audio_path, _recorded_silence) in sources {
        let has_source_turn = turns
            .iter()
            .any(|turn| turn.artifact_id == *artifact_id && turn.source == *source);
        if has_source_turn {
            continue;
        }
        // A source with zero turns AFTER detection found some had them
        // rejected as speaker bleed on purpose. Resurrecting it as a
        // full-file turn would transcribe the entire raw recording —
        // re-attributing the whole remote meeting to the microphone, the
        // exact misattribution echo rejection removes.
        if echo_rejection
            .detected_turn_artifact_ids
            .iter()
            .any(|detected| detected == artifact_id)
        {
            continue;
        }
        turns.push(AudioTurn {
            artifact_id: artifact_id.clone(),
            source: source.clone(),
            source_path: audio_path.clone(),
            extraction_start_ms: 0,
            start_ms: 0,
            end_ms: 0,
            turn_index: turns.len() as i64,
        });
    }
    turns
}

fn source_recorded_silence(sources: &[SourceAudioForProcessing], artifact_id: &str) -> bool {
    sources
        .iter()
        .find(|(source_artifact_id, _, _, _)| source_artifact_id == artifact_id)
        .map(|(_, _, _, recorded_silence)| *recorded_silence)
        .unwrap_or(false)
}

/// Whether a failed source should be persisted as a visible per-source error.
/// A silent system-audio track (no_speech) is expected when the user only
/// speaks into the mic, so we drop it once any source produced a usable
/// transcript. Everything else — including system failures that aren't
/// no_speech, and the all-sources-failed case — is still recorded.
fn should_record_source_failure(source: &str, warning: &str, has_valid_transcript: bool) -> bool {
    if !has_valid_transcript {
        return true;
    }
    !(source == "system" && is_no_speech_message(warning))
}

fn is_no_speech_message(message: &str) -> bool {
    let normalized = message.trim().to_ascii_lowercase();
    normalized == "no_speech" || normalized.contains("no speech detected")
}

/// Whether a transcription error is a no-speech condition rather than a real
/// failure. The backend surfaces an empty (silent) segment as a 400 with a
/// `no_speech` reason, so it arrives on either the error code or message.
fn is_no_speech_error(error: &AppError) -> bool {
    is_no_speech_message(&error.code) || is_no_speech_message(&error.message)
}

// The note persists only these English sentences (no error code), so the
// desktop translates them by exact text: keep each one byte-identical to its
// `notes.failure.*` entry in `src/i18n/messages/notes.ts` (NoteFailureBanner).
fn user_facing_transcription_failure_message(
    code: &str,
    message: &str,
    recorded_silence: bool,
    source: &str,
) -> String {
    let normalized_code = code.trim().to_ascii_lowercase();
    let normalized_message = message.trim().to_ascii_lowercase();
    if normalized_code == "no_speech"
        || normalized_message == "no_speech"
        || normalized_message.contains("no speech")
    {
        if recorded_silence {
            if source == "system" {
                return "The system audio recorded silence for the whole session. Check that audio was playing and that system audio capture is enabled.".to_string();
            }
            return "The microphone recorded silence for the whole session. Check that the right microphone is selected in Settings and that macOS input volume is up.".to_string();
        }
        return "No speech detected. Try speaking louder or moving closer to the microphone."
            .to_string();
    }
    // Historical failures can still be present in saved processing jobs.
    // Render them as provider failures without retired account actions.
    if normalized_message.contains("metering_provider_failed")
        || normalized_code.contains("metering")
        || normalized_code.contains("insufficient_credits")
        || normalized_message.contains("insufficient_credits")
        || normalized_message.contains("balance is too low")
        || normalized_message.starts_with("billing is temporarily unavailable")
        || normalized_message.contains("upstream_provider_failed")
        || normalized_code.contains("upstream")
    {
        return "The transcription provider could not process this audio.".to_string();
    }
    // Preserve actionable retry guidance for a historical concurrency denial.
    if normalized_message.contains("authorization_denied")
        || normalized_code.contains("authorization_denied")
    {
        return "The service is busy right now. Wait a minute, then retry.".to_string();
    }
    message.trim().to_string()
}

// Same contract as `user_facing_transcription_failure_message`: the sentences
// below are translated by exact text in `notes.failure.generation*`.
fn note_generation_failure_for_user(mut error: AppError) -> AppError {
    let normalized_code = error.code.trim().to_ascii_lowercase();
    let normalized_message = error.message.trim().to_ascii_lowercase();
    error.message = if normalized_code == "openrouter_api_key_missing" {
        "Your recording was transcribed, but Clovy needs your OpenRouter API key to write the note. Add it in Settings > Models > More options > Provider API keys. Your transcript is saved."
            .to_string()
    } else if normalized_message.contains("authorization_denied")
        || normalized_code.contains("authorization_denied")
    {
        "Your recording was transcribed, but note generation is busy right now. Your transcript is saved."
            .to_string()
    } else {
        "Your recording was transcribed, but Clovy couldn't generate the note. Your transcript is saved."
            .to_string()
    };
    error
}

fn ordered_source_transcripts(
    mut sources: Vec<SourceTranscriptInput>,
) -> Vec<SourceTranscriptInput> {
    sources.sort_by(|left, right| {
        left.turn_index
            .unwrap_or(i64::MAX)
            .cmp(&right.turn_index.unwrap_or(i64::MAX))
            .then_with(|| {
                left.start_ms
                    .unwrap_or(i64::MAX)
                    .cmp(&right.start_ms.unwrap_or(i64::MAX))
            })
    });
    sources
}

fn can_coalesce_source_transcripts(
    left: &SourceTranscriptInput,
    right: &SourceTranscriptInput,
) -> bool {
    if !left.valid || !right.valid || left.source != right.source {
        return false;
    }
    match (left.end_ms, right.start_ms) {
        (Some(left_end), Some(right_start)) => {
            right_start - left_end <= TRANSCRIPT_COHERENCE_GAP_MS
        }
        _ => false,
    }
}

fn join_transcript_text(left: &str, right: &str) -> String {
    let left = left.trim();
    let right = right.trim();
    if left.is_empty() {
        return right.to_string();
    }
    if right.is_empty() {
        return left.to_string();
    }
    format!("{left} {right}")
}

/// Groq Whisper takes only a short `prompt` hint (224 tokens per Groq's
/// speech-to-text docs), so the custom dictionary and previous-turn context
/// built for a note cannot be guaranteed to reach the transcription. The
/// existing cleanup engine applies that full context to Groq transcripts, as
/// it did for the previous provider; a transcript from any other source is
/// returned unchanged.
async fn maybe_post_process_note_transcript(
    provider: &str,
    mut transcript: TranscriptionProviderResult,
    context: Option<&str>,
) -> TranscriptionProviderResult {
    if !matches!(
        provider,
        crate::providers::PROVIDER_GROQ | crate::providers::PROVIDER_LOCAL
    ) {
        return transcript;
    }
    if transcript.text.trim().is_empty() {
        return transcript;
    }
    if let Ok(cleaned) = cleanup_note_transcript_text(&transcript.text, context).await {
        if !cleaned.trim().is_empty() {
            transcript.text = cleaned;
        }
    }
    transcript
}

async fn cleanup_note_transcript_text(
    text: &str,
    context: Option<&str>,
) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    match tokio::time::timeout(
        Duration::from_millis(NOTE_TRANSCRIPT_CLEANUP_TIMEOUT_MS),
        crate::providers::notes::cleanup_text(
            crate::providers::openrouter::DictateCleanupRequestParams {
                text: text.to_string(),
                dictionary_context: context.map(str::to_string),
                app_context: None,
                style: crate::providers::notes::NOTE_TRANSCRIPT_CLEANUP_STYLE.to_string(),
                session_id: "note_transcript".to_string(),
                utterance_id: uuid::Uuid::new_v4().to_string(),
            },
        ),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(AppError::new(
            "note_transcript_cleanup_timeout",
            "Note transcript cleanup timed out.",
        )),
    }
}

fn tail_chars(value: &str, max_chars: usize) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    if chars.len() <= max_chars {
        return value.to_string();
    }
    chars[chars.len() - max_chars..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::turns::coalesce_turns_for_transcription_avoiding_echo;
    use crate::providers::groq::TranscriptionProviderResult;
    use sqlx::row::Row;
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Arc, Mutex,
        },
        time::Duration,
    };

    /// A transcription source the note cleanup engine does not run for, so
    /// pipeline tests never reach a real cleanup provider.
    const TEST_TRANSCRIPTION_PROVIDER: &str = "test";

    #[tokio::test]
    async fn generation_uses_authoritative_sessions_and_keeps_equal_legacy_records() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrations::run_migrations(&pool).await.unwrap();
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.unwrap();
        for (id, started) in [("one", "2026-01-01"), ("two", "2026-01-02")] {
            sqlx::query::query("INSERT INTO recording_sessions (id,note_id,status,started_at) VALUES (?,?,'completed',?)")
                .bind(id).bind(&note.id).bind(started).execute(&repos.pool).await.unwrap();
        }
        let artifact = repos
            .create_audio_artifact(
                &note.id,
                "one",
                "fixture.wav",
                1000,
                44,
                "synthetic-checksum",
            )
            .await
            .unwrap();
        for (id, session, source, turn, text, status) in [
            (
                "old-aggregate",
                Some("one"),
                "microphone",
                None,
                "replaced",
                "succeeded",
            ),
            (
                "first-turn",
                Some("one"),
                "microphone",
                Some(3),
                "earlier recording",
                "succeeded",
            ),
            (
                "second-turn",
                Some("two"),
                "system",
                Some(0),
                "later recording",
                "succeeded",
            ),
            (
                "legacy-one",
                None,
                "microphone",
                None,
                "repeated",
                "succeeded",
            ),
            (
                "legacy-two",
                None,
                "microphone",
                None,
                "repeated",
                "succeeded",
            ),
            (
                "failed",
                Some("two"),
                "system",
                Some(1),
                "failed text",
                "failed",
            ),
        ] {
            sqlx::query::query("INSERT INTO transcripts (id,note_id,audio_artifact_id,recording_session_id,source,turn_index,text,status,provider,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?,'test','2026-01-03','2026-01-03')")
                .bind(id).bind(&note.id).bind(&artifact.id).bind(session).bind(source).bind(turn).bind(text).bind(status)
                .execute(&repos.pool).await.unwrap();
        }
        let (text, labels) = authoritative_generation_transcript(&repos, &note.id)
            .await
            .unwrap();
        assert!(labels);
        assert_eq!(text, "Microphone: earlier recording\nSystem: later recording\nMicrophone: repeated\nMicrophone: repeated");
    }

    struct DropNotifier(Option<tokio::sync::mpsc::UnboundedSender<()>>);

    impl Drop for DropNotifier {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn video_lookup_failure_keeps_transcript_generation_and_persisted_notice() {
        use crate::llm::{
            cli::CliKind,
            registry::ProviderRef,
            shell_env::{set_login_env_for_tests, LoginEnv},
        };
        use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

        struct Restore;
        impl Drop for Restore {
            fn drop(&mut self) {
                crate::providers::replace_current_settings_for_tests(
                    crate::providers::default_settings_for_tests(),
                );
                set_login_env_for_tests(None);
            }
        }
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let _restore = Restore;
        let bin = tempfile::tempdir().unwrap();
        let prompt_log = bin.path().join("prompt.txt");
        let cli = bin.path().join("claude");
        std::fs::write(&cli, format!(
            "#!/bin/sh\ncat > '{}'\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"## Summary\\\\n- Preserve the transcript\"}}'\n",
            prompt_log.display(),
        )).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
        set_login_env_for_tests(Some(LoginEnv::from_vars(BTreeMap::from([(
            "PATH".into(),
            format!("{}:/usr/bin:/bin", bin.path().display()),
        )]))));
        let mut settings = crate::providers::default_settings_for_tests();
        settings.transcription_provider = TEST_TRANSCRIPTION_PROVIDER.into();
        settings.llm_usage.notes = ProviderRef::Cli {
            id: CliKind::Claude,
        };
        crate::providers::replace_current_settings_for_tests(settings);

        // Exercise finished-session and recovery-preflight database states. The
        // same pipeline is called after finishing, retrying, and recovering audio.
        for recovering in [false, true] {
            for case in ["video_sql_failure", "no_video", "permission_denied"] {
                let directory = tempfile::tempdir().unwrap();
                let paths = AppPaths::from_data_dir(directory.path().join("data")).unwrap();
                let pool = sqlx_sqlite::SqlitePoolOptions::new()
                    .max_connections(1)
                    .connect("sqlite::memory:")
                    .await
                    .unwrap();
                crate::db::migrations::run_migrations(&pool).await.unwrap();
                let repos = Repositories::new(pool);
                let note = repos.create_note("default", None).await.unwrap();
                let session_id = format!("video-lookup-{}", uuid::Uuid::new_v4());
                let session_dir = paths.recording_session_dir(&note.id, &session_id).unwrap();
                std::fs::create_dir_all(&session_dir).unwrap();
                let microphone = session_dir.join("microphone.wav");
                write_loud_wav(&microphone, 16_000, 32_000);
                repos
                    .create_recording_session(
                        &note.id,
                        &session_id,
                        RecordingSourceMode::MicrophoneOnly,
                        &session_dir.join("microphone.partial.wav").to_string_lossy(),
                        &microphone.to_string_lossy(),
                        None,
                    )
                    .await
                    .unwrap();
                let artifact = repos
                    .create_audio_artifact(
                        &note.id,
                        &session_id,
                        &microphone.to_string_lossy(),
                        2_000,
                        std::fs::metadata(&microphone).unwrap().len() as i64,
                        "checksum",
                    )
                    .await
                    .unwrap();
                if recovering {
                    repos
                        .mark_recording_recoverable(&session_id, &note.id)
                        .await
                        .unwrap();
                }
                if case != "no_video" {
                    repos
                        .create_video_artifact(
                            &note.id,
                            &session_id,
                            &session_dir.join("window.partial.mp4").to_string_lossy(),
                            &session_dir.join("window.mp4").to_string_lossy(),
                        )
                        .await
                        .unwrap();
                }
                if case == "permission_denied" {
                    repos
                        .finalize_video_artifact(
                            &session_id,
                            &crate::audio::capture::video::VideoResult {
                                failure_reason: Some("permission_denied".into()),
                                ..Default::default()
                            },
                        )
                        .await
                        .unwrap();
                } else if case == "video_sql_failure" {
                    // Missing critical timing causes a video-only UPDATE. Abort
                    // that operation while leaving audio and note writes healthy.
                    sqlx::query::query("CREATE TRIGGER fail_video_update BEFORE UPDATE ON video_artifacts BEGIN SELECT RAISE(ABORT, 'video-only failure'); END")
                        .execute(&repos.pool).await.unwrap();
                    let error = repos
                        .note_video_input(&session_id, &paths)
                        .await
                        .unwrap_err();
                    assert!(error.message.contains("video-only failure"), "{error:?}");
                }
                let expected = match case {
                    "no_video" => None,
                    "permission_denied" => Some("permission_denied"),
                    _ => Some("capture_failed"),
                };
                if recovering {
                    let preflight = optional_note_video_input(&repos, &paths, &session_id).await;
                    assert_eq!(
                        preflight
                            .as_ref()
                            .and_then(|video| video.failure_reason.as_deref()),
                        expected,
                    );
                    assert_eq!(preflight.is_some(), case != "no_video");
                }
                let transcriber: TurnTranscriber = Arc::new(|_| {
                    Box::pin(async { Ok(test_provider_result("We will preserve the transcript.")) })
                });
                let result = TEST_NOTE_TRANSCRIPTION
                    .scope(
                        (
                            transcriber,
                            Arc::new(RequestPacer::new(18, Duration::from_secs(60))),
                        ),
                        process_saved_source_audio_with_progress(
                            &repos,
                            &paths,
                            &note.id,
                            &session_id,
                            RecordingSourceMode::MicrophoneOnly,
                            vec![(artifact.id, "microphone".into(), microphone, false)],
                            "Video lookup".into(),
                            None,
                            None,
                            ProcessingTiming::untracked(),
                            NoteProcessingProgressReporter::default(),
                        ),
                    )
                    .await;
                let generated = result
                    .unwrap_or_else(|error| panic!("{case}, recovering={recovering}: {error:?}"));
                assert_eq!(generated.processing_status, ProcessingStatus::Ready);
                assert!(generated
                    .generated_content
                    .as_deref()
                    .unwrap()
                    .contains("Preserve the transcript"));
                let stored = repos.get_note(&note.id).await.unwrap();
                assert_eq!(stored.video_analysis.as_deref(), expected);
                assert_eq!(stored.processing_status, ProcessingStatus::Ready);
                let transcripts = repos
                    .certified_source_turn_transcripts_for_session(&session_id)
                    .await
                    .unwrap();
                assert!(transcripts
                    .iter()
                    .any(|row| row.text.contains("preserve the transcript")));
                let prompt = std::fs::read_to_string(&prompt_log).unwrap();
                assert!(prompt.contains("We will preserve the transcript."));
                assert!(!prompt.contains("window.mp4"));
                let complete: String = sqlx::query_scalar::query_scalar(
                    "SELECT details FROM recording_checkpoints WHERE recording_session_id=? AND kind='processing_complete' ORDER BY rowid DESC LIMIT 1",
                ).bind(&session_id).fetch_one(&repos.pool).await.unwrap();
                assert_eq!(
                    serde_json::from_str::<serde_json::Value>(&complete).unwrap()["status"],
                    "succeeded"
                );
            }
        }
    }

    async fn test_source_processing_repositories() -> (Repositories, String) {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.expect("note");
        let recording_session_id = format!("session-{}", uuid::Uuid::new_v4());
        repos
            .create_recording_session(
                &note.id,
                &recording_session_id,
                RecordingSourceMode::MicrophonePlusSystem,
                "microphone.partial.wav",
                "microphone.wav",
                None,
            )
            .await
            .expect("recording session");
        (repos, recording_session_id)
    }

    #[tokio::test]
    async fn cancelled_recovery_finishes_started_pipeline_and_preserves_audio() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let _server = install_fake_correction_engine(Arc::new(echo_correction)).await;
        for target in [
            NoteProcessingStage::Transcribing,
            NoteProcessingStage::Generating,
        ] {
            let (repos, session_id) = test_source_processing_repositories().await;
            let note_id: String = sqlx::query_scalar::query_scalar(
                "SELECT note_id FROM recording_sessions WHERE id = ?",
            )
            .bind(&session_id)
            .fetch_one(&repos.pool)
            .await
            .unwrap();
            let directory = tempfile::tempdir().unwrap();
            let paths = AppPaths::from_data_dir(directory.path().join("data")).unwrap();
            let wav = directory.path().join("source.wav");
            write_three_turn_microphone(&wav);
            let bytes = std::fs::read(&wav).unwrap();
            let artifact = repos
                .create_audio_artifact(
                    &note_id,
                    &session_id,
                    &wav.to_string_lossy(),
                    10_000,
                    bytes.len() as i64,
                    "checksum",
                )
                .await
                .unwrap();
            let events = Arc::new(Mutex::new(Vec::new()));
            let captured = Arc::clone(&events);
            let (tx, rx) = tokio::sync::oneshot::channel();
            let tx = Mutex::new(Some(tx));
            let progress = NoteProcessingProgressReporter::new(move |event| {
                if event.stage == target {
                    if let Some(tx) = tx.lock().unwrap().take() {
                        let _ = tx.send(());
                    }
                }
                captured.lock().unwrap().push(event);
            });
            let (transcriber, _) = scripted_transcriber(vec![Ok("A real transcript")]);
            let result = TEST_NOTE_TRANSCRIPTION
                .scope(
                    (
                        transcriber,
                        Arc::new(
                            RequestPacer::new(18, Duration::from_secs(60))
                                .with_clock(tokio_clock()),
                        ),
                    ),
                    process_saved_source_audio_with_cancellation(
                        &repos,
                        &paths,
                        &note_id,
                        &session_id,
                        RecordingSourceMode::MicrophonePlusSystem,
                        vec![(artifact.id.clone(), "microphone".into(), wav.clone(), false)],
                        "Recovery".into(),
                        None,
                        None,
                        ProcessingTiming::untracked(),
                        progress,
                        async {
                            rx.await.unwrap();
                            AppError::new("turn_expired", "Stopped")
                        },
                    ),
                )
                .await;
            assert_eq!(result.unwrap_err().code, "turn_expired");
            assert_eq!(
                repos.get_note(&note_id).await.unwrap().processing_status,
                ProcessingStatus::Failed
            );
            assert_eq!(std::fs::read(&wav).unwrap(), bytes);
            let artifacts = repos
                .valid_audio_artifact_paths_for_session(&note_id, &session_id)
                .await
                .unwrap();
            assert!(artifacts.iter().any(|row| row.0 == artifact.id));
            let events = events.lock().unwrap();
            assert!(events.iter().any(|event| event.stage == target));
            let done: Vec<_> = events
                .iter()
                .filter(|event| event.stage == NoteProcessingStage::Done)
                .collect();
            assert_eq!(done.len(), 1);
            assert_eq!(done[0].note_id, note_id);
            assert_eq!(done[0].recording_session_id, session_id);
            assert_eq!(done[0].processing_status, ProcessingStatus::Failed);
        }
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
    }

    #[tokio::test]
    async fn revoked_recovery_before_admission_leaves_the_real_note_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_data_dir(directory.path().join("data")).unwrap();
        let (repos, session_id) = test_source_processing_repositories().await;
        let note_id: String =
            sqlx::query_scalar::query_scalar("SELECT note_id FROM recording_sessions WHERE id = ?")
                .bind(&session_id)
                .fetch_one(&repos.pool)
                .await
                .unwrap();
        repos
            .set_note_status(&note_id, ProcessingStatus::Ready, None)
            .await
            .unwrap();
        let before = repos.get_note(&note_id).await.unwrap();
        let binding = crate::mcp_server::turns::TurnBinding {
            authority: crate::domain::turn_authority::TurnAuthority::new(),
            wire_names: None,
            session_id: "conversation".into(),
            run_id: "run".into(),
            data_partition: "default".into(),
            cli: crate::llm::cli::CliKind::Claude,
            safety_mode: crate::agent_sessions::domain::AgentSafetyMode::Sandboxed,
            attended: true,
            surface: "chat".into(),
        };
        binding.authority.revoke();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let effect = process_saved_source_audio_with_cancellation(
            &repos,
            &paths,
            &note_id,
            &session_id,
            RecordingSourceMode::MicrophonePlusSystem,
            Vec::new(),
            "Recovery".into(),
            None,
            None,
            ProcessingTiming::untracked(),
            NoteProcessingProgressReporter::new(move |event| captured.lock().unwrap().push(event)),
            std::future::pending(),
        );
        let error = crate::commands::run_authorized_queued_audio(
            &repos.pool,
            &binding,
            "default",
            &note_id,
            &session_id,
            effect,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, "turn_expired");
        let after = repos.get_note(&note_id).await.unwrap();
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(after).unwrap()
        );
        assert!(events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cancellation_after_generation_commit_keeps_success_and_emits_done_once() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let _server = install_fake_correction_engine(Arc::new(|request| {
            let prompt = request["messages"]
                .as_array()
                .and_then(|messages| messages.last())
                .and_then(|message| message["content"].as_str())
                .unwrap_or_default();
            if prompt.contains("<asr_transcript>") {
                echo_correction(request)
            } else {
                (
                    200,
                    crate::llm::endpoint::test_server::chat_reply("Completed meeting notes"),
                )
            }
        }))
        .await;
        let (repos, session_id) = test_source_processing_repositories().await;
        let note_id: String =
            sqlx::query_scalar::query_scalar("SELECT note_id FROM recording_sessions WHERE id = ?")
                .bind(&session_id)
                .fetch_one(&repos.pool)
                .await
                .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let paths = AppPaths::from_data_dir(directory.path().join("data")).unwrap();
        let wav = directory.path().join("source.wav");
        write_three_turn_microphone(&wav);
        let artifact = repos
            .create_audio_artifact(
                &note_id,
                &session_id,
                &wav.to_string_lossy(),
                10_000,
                0,
                "checksum",
            )
            .await
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let progress =
            NoteProcessingProgressReporter::new(move |event| captured.lock().unwrap().push(event));
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let cancellation_observed = AtomicBool::new(false);
        let (transcriber, _) = scripted_transcriber(vec![Ok("A real transcript")]);
        let result = TEST_GENERATION_COMMITTED
            .scope(
                tx,
                TEST_NOTE_TRANSCRIPTION.scope(
                    (
                        transcriber,
                        Arc::new(
                            RequestPacer::new(18, Duration::from_secs(60))
                                .with_clock(tokio_clock()),
                        ),
                    ),
                    process_saved_source_audio_with_cancellation(
                        &repos,
                        &paths,
                        &note_id,
                        &session_id,
                        RecordingSourceMode::MicrophonePlusSystem,
                        vec![(artifact.id, "microphone".into(), wav, false)],
                        "Recovery".into(),
                        None,
                        None,
                        ProcessingTiming::untracked(),
                        progress,
                        async {
                            rx.recv().await.unwrap();
                            cancellation_observed.store(true, Ordering::SeqCst);
                            AppError::new("turn_expired", "Stopped")
                        },
                    ),
                ),
            )
            .await
            .unwrap();
        assert!(cancellation_observed.load(Ordering::SeqCst));
        assert_eq!(result.processing_status, ProcessingStatus::Ready);
        assert_eq!(
            repos.get_note(&note_id).await.unwrap().generated_content,
            result.generated_content
        );
        assert!(result
            .generated_content
            .as_deref()
            .unwrap()
            .contains("Completed meeting notes"));
        let events = events.lock().unwrap();
        let done: Vec<_> = events
            .iter()
            .filter(|event| event.stage == NoteProcessingStage::Done)
            .collect();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].note_id, note_id);
        assert_eq!(done[0].recording_session_id, session_id);
        assert_eq!(done[0].processing_status, ProcessingStatus::Ready);
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
    }

    #[tokio::test]
    async fn note_date_is_the_day_the_recording_started_not_when_the_note_was_created() {
        let (repos, session_id) = test_source_processing_repositories().await;
        let started = "2026-10-28T15:00:00.000Z";
        sqlx::query::query("UPDATE notes SET created_at = '2026-01-05T15:00:00.000Z'")
            .execute(&repos.pool)
            .await
            .expect("age the note");
        sqlx::query::query("UPDATE recording_sessions SET started_at = ? WHERE id = ?")
            .bind(started)
            .bind(&session_id)
            .execute(&repos.pool)
            .await
            .expect("date the recording");

        assert_eq!(
            note_recorded_on(&repos, &session_id).await,
            crate::providers::notes::recording_date(started)
        );
        assert_eq!(note_recorded_on(&repos, "missing-session").await, None);
    }

    #[tokio::test]
    async fn processing_progress_emits_stage_transitions_and_done_revision() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.expect("note");
        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let progress = NoteProcessingProgressReporter::new(move |event| {
            captured.lock().expect("progress events").push(event);
        });

        transition_note_processing_stage(
            &repos,
            &note.id,
            "recording-1",
            NoteProcessingStage::Transcribing,
            ProcessingStatus::Transcribing,
            &progress,
        )
        .await
        .expect("transcribing transition");
        transition_note_processing_stage(
            &repos,
            &note.id,
            "recording-1",
            NoteProcessingStage::Generating,
            ProcessingStatus::Generating,
            &progress,
        )
        .await
        .expect("generating transition");
        let done_revision = repos
            .set_note_status(&note.id, ProcessingStatus::Ready, None)
            .await
            .expect("ready transition");
        let ready_note = repos.get_note(&note.id).await.expect("ready note");
        complete_note_processing_run(&repos, &progress, &note.id, "recording-1", Ok(ready_note))
            .await
            .expect("complete processing");

        let events = events.lock().expect("progress events");
        assert_eq!(events.len(), 3);
        assert_eq!(
            events
                .iter()
                .map(|event| (event.stage, event.processing_status))
                .collect::<Vec<_>>(),
            vec![
                (
                    NoteProcessingStage::Transcribing,
                    ProcessingStatus::Transcribing,
                ),
                (
                    NoteProcessingStage::Generating,
                    ProcessingStatus::Generating,
                ),
                (NoteProcessingStage::Done, ProcessingStatus::Ready),
            ]
        );
        assert_eq!(events[2].revision, done_revision);
        assert!(events.iter().all(|event| event.note_id == note.id));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn first_event_timeline_flushes_each_checkpoint_once() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.expect("note");
        let recording_session_id = format!("session-{}", uuid::Uuid::new_v4());
        repos
            .create_recording_session(
                &note.id,
                &recording_session_id,
                RecordingSourceMode::MicrophoneOnly,
                "microphone.partial.wav",
                "microphone.wav",
                None,
            )
            .await
            .expect("recording session");

        let timeline = FirstEventTimeline::new(ProcessingTiming::from_done(Instant::now()));
        let first_request = timeline.clone();
        let second_request = timeline.clone();
        let first_persisted = timeline.clone();
        let second_persisted = timeline.clone();
        let barrier = Arc::new(tokio::sync::Barrier::new(5));
        let first_request_barrier = Arc::clone(&barrier);
        let second_request_barrier = Arc::clone(&barrier);
        let first_persisted_barrier = Arc::clone(&barrier);
        let second_persisted_barrier = Arc::clone(&barrier);
        let (first_request, second_request, first_persisted, second_persisted, _) = tokio::join!(
            tokio::spawn(async move {
                first_request_barrier.wait().await;
                first_request.mark_first_request();
            }),
            tokio::spawn(async move {
                second_request_barrier.wait().await;
                second_request.mark_first_request();
            }),
            tokio::spawn(async move {
                first_persisted_barrier.wait().await;
                first_persisted.mark_first_persisted();
            }),
            tokio::spawn(async move {
                second_persisted_barrier.wait().await;
                second_persisted.mark_first_persisted();
            }),
            async {
                barrier.wait().await;
            },
        );
        first_request.expect("first request marker");
        second_request.expect("second request marker");
        first_persisted.expect("first persistence marker");
        second_persisted.expect("second persistence marker");

        let second_flush = timeline.clone();
        tokio::join!(
            timeline.flush(&repos, &recording_session_id),
            second_flush.flush(&repos, &recording_session_id),
        );

        let rows = sqlx::query::query(
            "SELECT kind, details
             FROM recording_checkpoints
             WHERE recording_session_id = ?
               AND kind IN ('first_note_transcription_request', 'first_transcript_persisted')
             ORDER BY rowid ASC",
        )
        .bind(&recording_session_id)
        .fetch_all(&repos.pool)
        .await
        .expect("first-event checkpoints");
        assert_eq!(rows.len(), 2);

        for expected_kind in [
            "first_note_transcription_request",
            "first_transcript_persisted",
        ] {
            let matching = rows
                .iter()
                .filter(|row| row.get::<String, _>("kind") == expected_kind)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "checkpoint count for {expected_kind}");
            let details = matching[0]
                .get::<Option<String>, _>("details")
                .expect("checkpoint details");
            let details: serde_json::Value =
                serde_json::from_str(&details).expect("checkpoint details JSON");
            let object = details.as_object().expect("checkpoint details object");
            assert_eq!(object.len(), 1);
            assert!(object["doneToDurationMs"].as_i64().is_some());
        }
    }

    #[tokio::test]
    async fn live_transcription_warning_persists_only_for_actionable_active_failure() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.expect("note");
        repos
            .set_note_status(&note.id, ProcessingStatus::Transcribing, None)
            .await
            .expect("start transcription");

        let actionable_failure = SourceTranscriptInput {
            source: "microphone".to_string(),
            text: String::new(),
            valid: false,
            warning: Some("The service is busy right now. Wait a minute, then retry.".to_string()),
            recorded_silence: false,
            start_ms: Some(0),
            end_ms: Some(1_000),
            turn_index: Some(0),
        };
        persist_active_note_transcription_warning(&repos, &note.id, &actionable_failure)
            .await
            .expect("persist warning");
        let warned = repos.get_note(&note.id).await.expect("warned note");
        assert_eq!(warned.processing_status, ProcessingStatus::Transcribing);
        assert_eq!(
            warned.last_error.as_deref(),
            Some("The service is busy right now. Wait a minute, then retry.")
        );

        let later_failure = SourceTranscriptInput {
            warning: Some("The transcription provider could not process this audio.".to_string()),
            turn_index: Some(1),
            ..actionable_failure.clone()
        };
        persist_active_note_transcription_warning(&repos, &note.id, &later_failure)
            .await
            .expect("keep first warning");
        assert_eq!(
            repos
                .get_note(&note.id)
                .await
                .expect("first warning note")
                .last_error
                .as_deref(),
            Some("The service is busy right now. Wait a minute, then retry.")
        );

        repos
            .set_note_status(&note.id, ProcessingStatus::Transcribing, None)
            .await
            .expect("retry transcription");
        let no_speech_failure = SourceTranscriptInput {
            warning: Some(
                "No speech detected. Try speaking louder or moving closer to the microphone."
                    .to_string(),
            ),
            ..actionable_failure.clone()
        };
        persist_active_note_transcription_warning(&repos, &note.id, &no_speech_failure)
            .await
            .expect("ignore no-speech warning");
        assert!(repos
            .get_note(&note.id)
            .await
            .expect("retried note")
            .last_error
            .is_none());

        persist_active_note_transcription_warning(&repos, &note.id, &actionable_failure)
            .await
            .expect("persist warning after retry");
        repos
            .set_note_status(&note.id, ProcessingStatus::Generating, None)
            .await
            .expect("advance after transcription success");
        persist_active_note_transcription_warning(&repos, &note.id, &later_failure)
            .await
            .expect("ignore warning after transcription");
        let generating = repos.get_note(&note.id).await.expect("generating note");
        assert_eq!(generating.processing_status, ProcessingStatus::Generating);
        assert!(generating.last_error.is_none());
    }

    #[tokio::test]
    async fn microphone_only_setup_failure_persists_terminal_checkpoints_once() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.expect("note");
        let recording_session_id = format!("session-{}", uuid::Uuid::new_v4());
        repos
            .create_recording_session(
                &note.id,
                &recording_session_id,
                RecordingSourceMode::MicrophoneOnly,
                "microphone.partial.wav",
                "microphone.wav",
                None,
            )
            .await
            .expect("recording session");

        let temp_dir = session_temp_dir("clovy-transcription", &recording_session_id);
        let _ = std::fs::remove_dir_all(&temp_dir);
        let _ = std::fs::remove_file(&temp_dir);
        std::fs::write(&temp_dir, b"block note transcription directory creation")
            .expect("blocking note transcription file");
        let expected_message = std::fs::create_dir_all(&temp_dir)
            .expect_err("regular file must block directory creation")
            .to_string();

        let result = process_saved_audio(
            &repos,
            &note.id,
            &recording_session_id,
            "microphone-artifact",
            PathBuf::from("unused-microphone.wav"),
            "Setup failure".to_string(),
            None,
            None,
            false,
            ProcessingTiming::from_done(Instant::now()),
        )
        .await;
        std::fs::remove_file(&temp_dir).expect("remove blocking note transcription file");

        let error = result.expect_err("microphone-only setup must fail");
        assert_eq!(error.code, "audio_normalize_failed");
        assert_eq!(error.message, expected_message);
        assert!(error.details.is_none());

        let rows = sqlx::query::query(
            "SELECT kind, details
             FROM recording_checkpoints
             WHERE recording_session_id = ?
               AND kind IN ('note_transcription_complete', 'processing_complete')
             ORDER BY rowid ASC",
        )
        .bind(&recording_session_id)
        .fetch_all(&repos.pool)
        .await
        .expect("terminal checkpoints");
        assert_eq!(rows.len(), 2, "one checkpoint for each terminal kind");

        let note_transcription_rows = rows
            .iter()
            .filter(|row| row.get::<String, _>("kind") == "note_transcription_complete")
            .collect::<Vec<_>>();
        assert_eq!(note_transcription_rows.len(), 1);
        let note_transcription_details = note_transcription_rows[0]
            .get::<Option<String>, _>("details")
            .expect("note transcription checkpoint details");
        let note_transcription_details: serde_json::Value =
            serde_json::from_str(&note_transcription_details)
                .expect("note transcription details JSON");
        assert_eq!(note_transcription_details["status"], "failed");
        assert_eq!(
            note_transcription_details["error"],
            "audio_normalize_failed"
        );
        assert!(note_transcription_details["durationMs"].as_i64().is_some());
        assert!(note_transcription_details["doneToDurationMs"]
            .as_i64()
            .is_some());

        let processing_rows = rows
            .iter()
            .filter(|row| row.get::<String, _>("kind") == "processing_complete")
            .collect::<Vec<_>>();
        assert_eq!(processing_rows.len(), 1);
        let processing_details = processing_rows[0]
            .get::<Option<String>, _>("details")
            .expect("processing checkpoint details");
        let processing_details: serde_json::Value =
            serde_json::from_str(&processing_details).expect("processing details JSON");
        assert_eq!(processing_details["status"], "failed");
        assert!(processing_details["durationMs"].as_i64().is_some());
        assert!(processing_details["doneToDurationMs"].as_i64().is_some());
    }

    #[tokio::test]
    async fn source_processing_dictionary_load_failure_persists_terminal_checkpoints_once() {
        let (repos, recording_session_id) = test_source_processing_repositories().await;
        let note_id: String =
            sqlx::query_scalar::query_scalar("SELECT note_id FROM recording_sessions WHERE id = ?")
                .bind(&recording_session_id)
                .fetch_one(&repos.pool)
                .await
                .expect("recording session note");
        sqlx::query::query("DROP TABLE dictionary_entries")
            .execute(&repos.pool)
            .await
            .expect("drop dictionary table");

        let error = process_saved_source_audio(
            &repos,
            &note_id,
            &recording_session_id,
            RecordingSourceMode::MicrophonePlusSystem,
            Vec::new(),
            "Dictionary failure".to_string(),
            None,
            None,
            ProcessingTiming::from_done(Instant::now()),
        )
        .await
        .expect_err("dictionary load must fail");
        assert_eq!(error.code, "storage_unavailable");

        let rows = sqlx::query::query(
            "SELECT kind, details
             FROM recording_checkpoints
             WHERE recording_session_id = ?
               AND kind IN ('note_transcription_complete', 'processing_complete')
             ORDER BY rowid ASC",
        )
        .bind(&recording_session_id)
        .fetch_all(&repos.pool)
        .await
        .expect("terminal checkpoints");
        assert_eq!(rows.len(), 2, "one checkpoint for each terminal kind");

        let details_for = |kind: &str| {
            let row = rows
                .iter()
                .find(|row| row.get::<String, _>("kind") == kind)
                .unwrap_or_else(|| panic!("missing {kind} checkpoint"));
            let details = row
                .get::<Option<String>, _>("details")
                .expect("checkpoint details");
            serde_json::from_str::<serde_json::Value>(&details).expect("checkpoint details JSON")
        };

        let note_transcription = details_for("note_transcription_complete");
        assert_eq!(note_transcription["status"], "failed");
        assert_eq!(note_transcription["error"], "storage_unavailable");
        assert!(note_transcription["durationMs"].as_i64().is_some());
        assert!(note_transcription["doneToDurationMs"].as_i64().is_some());

        let processing = details_for("processing_complete");
        assert_eq!(processing["status"], "failed");
        assert!(processing["durationMs"].as_i64().is_some());
        assert!(processing["doneToDurationMs"].as_i64().is_some());
    }

    #[tokio::test]
    async fn eager_preparation_failure_persists_terminal_checkpoints_without_counts() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.expect("note");
        let recording_session_id = format!("session-{}", uuid::Uuid::new_v4());
        repos
            .create_recording_session(
                &note.id,
                &recording_session_id,
                RecordingSourceMode::MicrophonePlusSystem,
                "microphone.partial.wav",
                "microphone.wav",
                None,
            )
            .await
            .expect("recording session");

        let source_dir = tempfile::tempdir().expect("source temp dir");
        let source_path = source_dir.path().join("microphone.wav");
        write_loud_wav(&source_path, 48_000, 48_000);

        let turn_wav_dir = session_temp_dir("clovy-turns", &recording_session_id);
        let _ = std::fs::remove_dir_all(&turn_wav_dir);
        let _ = std::fs::remove_file(&turn_wav_dir);
        std::fs::write(&turn_wav_dir, b"block turn directory creation")
            .expect("blocking turn WAV file");
        let expected_message = std::fs::create_dir_all(&turn_wav_dir)
            .expect_err("regular file must block directory creation")
            .to_string();

        let result = process_saved_source_audio(
            &repos,
            &note.id,
            &recording_session_id,
            RecordingSourceMode::MicrophonePlusSystem,
            vec![(
                "microphone-artifact".to_string(),
                "microphone".to_string(),
                source_path,
                false,
            )],
            "Preparation failure".to_string(),
            None,
            None,
            ProcessingTiming::from_done(Instant::now()),
        )
        .await;
        std::fs::remove_file(&turn_wav_dir).expect("remove blocking turn WAV file");

        let error = result.expect_err("eager preparation must fail");
        assert_eq!(error.code, "audio_turn_failed");
        assert_eq!(error.message, expected_message);
        assert!(error.details.is_none());

        let rows = sqlx::query::query(
            "SELECT kind, details
             FROM recording_checkpoints
             WHERE recording_session_id = ?
               AND kind IN ('note_transcription_complete', 'processing_complete')
             ORDER BY rowid ASC",
        )
        .bind(&recording_session_id)
        .fetch_all(&repos.pool)
        .await
        .expect("terminal checkpoints");
        assert_eq!(rows.len(), 2, "one checkpoint for each terminal kind");

        let note_transcription_rows = rows
            .iter()
            .filter(|row| row.get::<String, _>("kind") == "note_transcription_complete")
            .collect::<Vec<_>>();
        assert_eq!(note_transcription_rows.len(), 1);
        let note_transcription_details = note_transcription_rows[0]
            .get::<Option<String>, _>("details")
            .expect("note transcription checkpoint details");
        let note_transcription_details: serde_json::Value =
            serde_json::from_str(&note_transcription_details)
                .expect("note transcription details JSON");
        let note_transcription_details = note_transcription_details
            .as_object()
            .expect("note transcription details object");
        assert_eq!(note_transcription_details["status"], "failed");
        assert_eq!(note_transcription_details["error"], "audio_turn_failed");
        assert!(note_transcription_details["durationMs"].as_i64().is_some());
        assert!(note_transcription_details["doneToDurationMs"]
            .as_i64()
            .is_some());
        assert!(!note_transcription_details.contains_key("successfulTurnCount"));
        assert!(!note_transcription_details.contains_key("failedTurnCount"));

        let processing_rows = rows
            .iter()
            .filter(|row| row.get::<String, _>("kind") == "processing_complete")
            .collect::<Vec<_>>();
        assert_eq!(processing_rows.len(), 1);
        let processing_details = processing_rows[0]
            .get::<Option<String>, _>("details")
            .expect("processing checkpoint details");
        let processing_details: serde_json::Value =
            serde_json::from_str(&processing_details).expect("processing details JSON");
        assert_eq!(processing_details["status"], "failed");
        assert!(processing_details["durationMs"].as_i64().is_some());
        assert!(processing_details["doneToDurationMs"].as_i64().is_some());

        let turn_detection_count: i64 = sqlx::query_scalar::query_scalar(
            "SELECT COUNT(*)
             FROM recording_checkpoints
             WHERE recording_session_id = ? AND kind = 'turn_detection'",
        )
        .bind(&recording_session_id)
        .fetch_one(&repos.pool)
        .await
        .expect("turn-detection checkpoint count");
        assert_eq!(
            turn_detection_count, 1,
            "detection completed before failure"
        );

        let first_event_count: i64 = sqlx::query_scalar::query_scalar(
            "SELECT COUNT(*)
             FROM recording_checkpoints
             WHERE recording_session_id = ?
               AND kind IN ('first_note_transcription_request', 'first_transcript_persisted')",
        )
        .bind(&recording_session_id)
        .fetch_one(&repos.pool)
        .await
        .expect("first-event checkpoint count");
        assert_eq!(first_event_count, 0, "provider work never started");
    }

    #[tokio::test]
    async fn pipeline_failure_persists_preparation_and_terminal_checkpoints_once() {
        let (repos, recording_session_id) = test_source_processing_repositories().await;
        let timing = ProcessingTiming::from_done(Instant::now());
        let timeline = FirstEventTimeline::new(timing);
        let expected_error = AppError {
            code: "audio_turn_failed".to_string(),
            message: "planned pipeline preparation failure".to_string(),
            details: Some(serde_json::json!({ "stage": "producer" })),
        };
        let failure = TurnPipelineFailure {
            error: expected_error.clone(),
            preparation: Some(Box::new(TurnPreparationReport {
                prepared_count: 1,
                active_preparation_duration_ms: 42,
                producer_wall_duration_ms: 137,
                done_to_preparation_complete_ms: Some(211),
                error: Some(expected_error.clone()),
            })),
        };

        let returned_error = persist_turn_pipeline_result(
            &repos,
            &recording_session_id,
            Err(failure),
            3,
            &timeline,
            timing,
            Instant::now(),
            Instant::now(),
        )
        .await
        .expect_err("pipeline failure should be returned");
        assert_eq!(returned_error.code, expected_error.code);
        assert_eq!(returned_error.message, expected_error.message);
        assert_eq!(returned_error.details, expected_error.details);

        let rows = sqlx::query::query(
            "SELECT kind, details
             FROM recording_checkpoints
             WHERE recording_session_id = ?
               AND kind IN ('turn_wav_extraction', 'note_transcription_complete', 'processing_complete')
             ORDER BY rowid ASC",
        )
        .bind(&recording_session_id)
        .fetch_all(&repos.pool)
        .await
        .expect("pipeline failure checkpoints");
        assert_eq!(rows.len(), 3);

        let details_for = |kind: &str| {
            let matching = rows
                .iter()
                .filter(|row| row.get::<String, _>("kind") == kind)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "checkpoint count for {kind}");
            let details = matching[0]
                .get::<Option<String>, _>("details")
                .expect("checkpoint details");
            serde_json::from_str::<serde_json::Value>(&details).expect("checkpoint details JSON")
        };

        let preparation = details_for("turn_wav_extraction");
        assert_eq!(preparation["durationMs"], 42);
        assert_eq!(preparation["activePreparationDurationMs"], 42);
        assert_eq!(preparation["producerWallDurationMs"], 137);
        assert_eq!(preparation["doneToPreparationCompleteMs"], 211);
        assert_eq!(preparation["jobCount"], 1);
        assert_eq!(preparation["reusedTranscriptCount"], 3);
        assert_eq!(preparation["status"], "failed");
        assert_eq!(preparation["error"], "audio_turn_failed");

        let note_transcription = details_for("note_transcription_complete");
        assert_eq!(note_transcription["status"], "failed");
        assert_eq!(note_transcription["error"], "audio_turn_failed");
        assert!(note_transcription["durationMs"].as_i64().is_some());
        assert!(note_transcription["doneToDurationMs"].as_i64().is_some());
        let note_transcription = note_transcription
            .as_object()
            .expect("note transcription object");
        assert!(!note_transcription.contains_key("successfulTurnCount"));
        assert!(!note_transcription.contains_key("failedTurnCount"));

        let processing = details_for("processing_complete");
        assert_eq!(processing["status"], "failed");
        assert!(processing["durationMs"].as_i64().is_some());
        assert!(processing["doneToDurationMs"].as_i64().is_some());
    }

    #[tokio::test]
    async fn zero_descriptor_pipeline_persists_all_cached_preparation_checkpoint() {
        let (repos, recording_session_id) = test_source_processing_repositories().await;
        let timing = ProcessingTiming::from_done(Instant::now());
        let timeline = FirstEventTimeline::new(timing);
        let preparation_calls = Arc::new(AtomicUsize::new(0));
        let turn_preparer = {
            let preparation_calls = Arc::clone(&preparation_calls);
            Arc::new(move |_descriptor: TurnPreparationJob| {
                preparation_calls.fetch_add(1, Ordering::SeqCst);
                Err(AppError::new("unexpected", "unexpected preparation"))
            }) as TurnPreparer
        };
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_calls = Arc::clone(&fallback_calls);
            Arc::new(move |_plan: SourceFallbackPlan| {
                fallback_calls.fetch_add(1, Ordering::SeqCst);
                Err(AppError::new("unexpected", "unexpected fallback"))
            }) as SourceFallbackPreparer
        };
        let transcriber_calls = Arc::new(AtomicUsize::new(0));
        let transcriber = {
            let transcriber_calls = Arc::clone(&transcriber_calls);
            Arc::new(move |_request: TranscriptionRequest| {
                transcriber_calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async {
                    Err(AppError::new("unexpected", "unexpected note transcription"))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let pipeline = prepare_and_transcribe_turn_jobs_bounded(
            Vec::new(),
            Vec::new(),
            &[],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            turn_preparer,
            fallback_preparer,
            transcriber,
            None,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
            timing,
        )
        .await
        .expect("zero-descriptor pipeline should succeed");
        assert_eq!(preparation_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(transcriber_calls.load(Ordering::SeqCst), 0);
        let expected_producer_wall_ms = pipeline.preparation.producer_wall_duration_ms;
        let expected_done_to_preparation_ms = pipeline
            .preparation
            .done_to_preparation_complete_ms
            .expect("tracked zero-descriptor preparation completion");

        let returned = persist_turn_pipeline_result(
            &repos,
            &recording_session_id,
            Ok(pipeline),
            4,
            &timeline,
            timing,
            Instant::now(),
            Instant::now(),
        )
        .await
        .expect("zero-descriptor pipeline should succeed");
        assert!(returned.outcome.candidates.is_empty());

        let details: String = sqlx::query_scalar::query_scalar(
            "SELECT details
             FROM recording_checkpoints
             WHERE recording_session_id = ? AND kind = 'turn_wav_extraction'",
        )
        .bind(&recording_session_id)
        .fetch_one(&repos.pool)
        .await
        .expect("zero-descriptor preparation checkpoint");
        let details: serde_json::Value =
            serde_json::from_str(&details).expect("preparation details JSON");
        assert_eq!(details["durationMs"], 0);
        assert_eq!(details["activePreparationDurationMs"], 0);
        assert_eq!(details["producerWallDurationMs"], expected_producer_wall_ms);
        assert_eq!(
            details["doneToPreparationCompleteMs"],
            expected_done_to_preparation_ms
        );
        assert_eq!(details["jobCount"], 0);
        assert_eq!(details["reusedTranscriptCount"], 4);
        assert_eq!(details["status"], "succeeded");
        assert!(details["error"].is_null());

        let terminal_count: i64 = sqlx::query_scalar::query_scalar(
            "SELECT COUNT(*)
             FROM recording_checkpoints
             WHERE recording_session_id = ?
               AND kind IN ('note_transcription_complete', 'processing_complete')",
        )
        .bind(&recording_session_id)
        .fetch_one(&repos.pool)
        .await
        .expect("terminal checkpoint count");
        assert_eq!(terminal_count, 0);
    }

    #[test]
    fn session_temp_dir_sanitizes_untrusted_session_ids() {
        let temp_dir = session_temp_dir("clovy-turns", "../../outside/session");
        let file_name = temp_dir
            .file_name()
            .and_then(|value| value.to_str())
            .expect("temp dir file name");

        assert_eq!(file_name, "clovy-turns-______outside_session");
        assert!(!temp_dir
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir)));
    }

    #[test]
    fn missing_system_turn_gets_full_source_fallback() {
        let mic_path = PathBuf::from("microphone.wav");
        let system_path = PathBuf::from("system.wav");
        let sources = vec![
            (
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            ),
            (
                "system-artifact".to_string(),
                "system".to_string(),
                system_path.clone(),
                false,
            ),
        ];
        let detected_mic_turn = AudioTurn {
            artifact_id: "mic-artifact".to_string(),
            source: "microphone".to_string(),
            source_path: mic_path,
            extraction_start_ms: 7_020,
            start_ms: 7_020,
            end_ms: 9_180,
            turn_index: 0,
        };

        let covered = add_full_source_turns_for_missing_sources(
            &sources,
            vec![detected_mic_turn],
            &EchoRejectionReport::default(),
        );

        assert_eq!(covered.len(), 2);
        assert!(covered.iter().any(|turn| {
            turn.artifact_id == "mic-artifact" && turn.start_ms == 7_020 && turn.end_ms == 9_180
        }));
        let system_fallback = covered
            .iter()
            .find(|turn| turn.artifact_id == "system-artifact")
            .expect("system source should receive a fallback turn");
        assert_eq!(system_fallback.source, "system");
        assert_eq!(system_fallback.source_path, system_path);
        assert_eq!(system_fallback.start_ms, 0);
        assert_eq!(system_fallback.end_ms, 0);
    }

    #[test]
    fn source_coverage_does_not_duplicate_existing_turns() {
        let mic_path = PathBuf::from("microphone.wav");
        let system_path = PathBuf::from("system.wav");
        let sources = vec![
            (
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            ),
            (
                "system-artifact".to_string(),
                "system".to_string(),
                system_path.clone(),
                false,
            ),
        ];
        let turns = vec![
            AudioTurn {
                artifact_id: "mic-artifact".to_string(),
                source: "microphone".to_string(),
                source_path: mic_path,
                extraction_start_ms: 1_000,
                start_ms: 1_000,
                end_ms: 2_000,
                turn_index: 0,
            },
            AudioTurn {
                artifact_id: "system-artifact".to_string(),
                source: "system".to_string(),
                source_path: system_path,
                extraction_start_ms: 3_000,
                start_ms: 3_000,
                end_ms: 4_000,
                turn_index: 1,
            },
        ];

        let covered = add_full_source_turns_for_missing_sources(
            &sources,
            turns,
            &EchoRejectionReport::default(),
        );

        assert_eq!(covered.len(), 2);
        assert!(covered.iter().all(|turn| turn.end_ms > turn.start_ms));
    }

    #[test]
    fn all_bleed_microphone_is_not_resurrected_as_a_full_file_turn() {
        // The flagship echo-rejection scenario: the user on speakers mostly
        // listens, every detected microphone turn was bleed, and echo
        // rejection dropped them all. The full-file fallback exists for
        // sources whose detector genuinely found nothing; resurrecting this
        // microphone would transcribe the entire raw recording and
        // re-attribute the whole remote meeting to the user.
        let mic_path = PathBuf::from("microphone.wav");
        let sources = vec![(
            "mic-artifact".to_string(),
            "microphone".to_string(),
            mic_path,
            false,
        )];
        let report = EchoRejectionReport {
            detected_turn_artifact_ids: vec!["mic-artifact".to_string()],
            dropped_turn_count: 1,
            ..EchoRejectionReport::default()
        };

        let covered = add_full_source_turns_for_missing_sources(&sources, Vec::new(), &report);

        assert!(
            covered.is_empty(),
            "deliberately rejected source must not be resurrected, got {covered:?}"
        );
    }

    #[test]
    fn coalescing_does_not_bridge_trimmed_bleed_spans() {
        // Kept remainders around a trimmed interior bleed span sit closer
        // than the transcription coherence gap. Merging them would extract
        // one contiguous segment that contains the trimmed audio again.
        let turn = |start_ms: i64, end_ms: i64| AudioTurn {
            artifact_id: "mic-artifact".to_string(),
            source: "microphone".to_string(),
            source_path: PathBuf::from("microphone.wav"),
            extraction_start_ms: start_ms,
            start_ms,
            end_ms,
            turn_index: 0,
        };
        let remainders = vec![turn(0, 1_000), turn(2_000, 3_000)];

        // Without a bleed span in the gap the pair coalesces as before...
        let merged = coalesce_turns_for_transcription_avoiding_echo(remainders.clone(), &[]);
        assert_eq!(merged.len(), 1);
        assert_eq!((merged[0].start_ms, merged[0].end_ms), (0, 3_000));

        // ...but a trimmed bleed span between them forbids the bridge.
        let kept_apart =
            coalesce_turns_for_transcription_avoiding_echo(remainders, &[(1_000, 2_000)]);
        assert_eq!(kept_apart.len(), 2);
        assert_eq!(kept_apart[0].end_ms, 1_000);
        assert_eq!(kept_apart[1].start_ms, 2_000);
    }

    #[test]
    fn empty_detection_gets_full_source_fallback_for_every_source() {
        let mic_path = PathBuf::from("microphone.wav");
        let system_path = PathBuf::from("system.wav");
        let sources = vec![
            (
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            ),
            (
                "system-artifact".to_string(),
                "system".to_string(),
                system_path.clone(),
                false,
            ),
        ];

        let covered = add_full_source_turns_for_missing_sources(
            &sources,
            Vec::new(),
            &EchoRejectionReport::default(),
        );
        let covered = coalesce_turns_for_transcription_avoiding_echo(covered, &[]);

        assert_eq!(covered.len(), 2);
        let microphone = covered
            .iter()
            .find(|turn| turn.artifact_id == "mic-artifact")
            .expect("microphone source should receive a fallback turn");
        assert_eq!(microphone.source, "microphone");
        assert_eq!(microphone.source_path, mic_path);
        assert_eq!(microphone.start_ms, 0);
        assert_eq!(microphone.end_ms, 0);

        let system = covered
            .iter()
            .find(|turn| turn.artifact_id == "system-artifact")
            .expect("system source should receive a fallback turn");
        assert_eq!(system.source, "system");
        assert_eq!(system.source_path, system_path);
        assert_eq!(system.start_ms, 0);
        assert_eq!(system.end_ms, 0);
    }

    #[tokio::test]
    async fn transcribes_source_lanes_concurrently_and_keeps_turn_order() {
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let microphone_active = Arc::new(AtomicUsize::new(0));
        let same_source_overlap = Arc::new(AtomicUsize::new(0));
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            let microphone_active = Arc::clone(&microphone_active);
            let same_source_overlap = Arc::clone(&same_source_overlap);
            let operation_ids = Arc::clone(&operation_ids);
            Arc::new(move |request: TranscriptionRequest| {
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                let microphone_active = Arc::clone(&microphone_active);
                let same_source_overlap = Arc::clone(&same_source_overlap);
                let operation_ids = Arc::clone(&operation_ids);
                Box::pin(async move {
                    let path = request.audio_path.to_string_lossy().to_string();
                    let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(now_active, Ordering::SeqCst);
                    if path.starts_with('m')
                        && microphone_active.fetch_add(1, Ordering::SeqCst) != 0
                    {
                        same_source_overlap.fetch_add(1, Ordering::SeqCst);
                    }
                    // The system lane completes first. The next microphone turn
                    // must still wait for m0, whose text is its correction context.
                    tokio::time::sleep(Duration::from_millis(if path == "m0" { 30 } else { 5 }))
                        .await;
                    if path.starts_with('m') {
                        microphone_active.fetch_sub(1, Ordering::SeqCst);
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                    let operation_id = request.operation_id();
                    operation_ids.lock().unwrap().push(operation_id);
                    Ok(TranscriptionProviderResult {
                        text: path,
                        language: Some("es".to_string()),
                        provider: "test".to_string(),
                    })
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let outcome = transcribe_turn_jobs_by_source_lane(
            vec![
                test_job("m0", "microphone", 0),
                test_job("s1", "system", 1),
                test_job("m2", "microphone", 2),
            ],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            transcriber,
        )
        .await
        .expect("source lanes should transcribe");

        assert!(max_active.load(Ordering::SeqCst) > 1);
        assert_eq!(same_source_overlap.load(Ordering::SeqCst), 0);
        assert_eq!(
            outcome
                .candidates
                .iter()
                .map(|candidate| candidate.input.text.as_str())
                .collect::<Vec<_>>(),
            vec!["m0", "s1", "m2"]
        );

        let mut operation_ids = operation_ids.lock().unwrap().clone();
        operation_ids.sort();
        assert_eq!(
            operation_ids,
            vec![
                "artifact-m0-microphone-0-500-turn-0-fallback-false-saved-audio-v3",
                "artifact-m2-microphone-2000-2500-turn-2-fallback-false-saved-audio-v3",
                "artifact-s1-system-1000-1500-turn-1-fallback-false-saved-audio-v3",
            ]
        );
    }

    #[tokio::test]
    async fn groq_turns_send_only_the_dictionary_hint_and_correct_with_same_source_context() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let server = install_fake_correction_engine(Arc::new(echo_correction)).await;
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let prompts = Arc::clone(&prompts);
            Arc::new(move |request: TranscriptionRequest| {
                prompts.lock().unwrap().push(request.context.clone());
                let text = request.audio_path.to_string_lossy().to_string();
                Box::pin(async move { Ok(test_provider_result(&text)) }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let outcome = transcribe_turn_jobs_bounded(
            vec![
                test_job("m0", "microphone", 0),
                test_job("s1", "system", 1),
                test_job("m2", "microphone", 2),
            ],
            Vec::new(),
            &[],
            crate::providers::PROVIDER_GROQ.to_string(),
            "Meeting".to_string(),
            build_transcription_dictionary(&test_dictionary(&["DIM", "Clovy"])),
            Arc::new(prepare_source_fallback),
            transcriber,
            None,
            1,
        )
        .await;
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
        outcome.expect("turn jobs should transcribe");

        // Whisper only ever gets the spelling hint, never earlier turns.
        assert_eq!(
            *prompts.lock().unwrap(),
            vec![Some("DIM, Clovy".to_string()); 3]
        );
        let first = correction_request_for(&server, "m0");
        assert!(first.contains("- DIM") && first.contains("- Clovy"));
        assert!(!first.contains("Previous transcript context"));
        assert!(!correction_request_for(&server, "s1").contains("Microphone: m0"));
        let later = correction_request_for(&server, "m2");
        assert!(later.contains("- DIM") && later.contains("Microphone: m0"));
        assert!(!later.contains("System: s1"));
    }

    #[tokio::test]
    async fn groq_turn_correction_is_seeded_from_prior_cached_same_source_turns() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let server = install_fake_correction_engine(Arc::new(echo_correction)).await;
        let transcriber = Arc::new(|_request: TranscriptionRequest| {
            Box::pin(async move { Ok(test_provider_result("fresh microphone turn")) })
                as TranscriptionFuture
        }) as TurnTranscriber;
        let cached_candidates = vec![
            TranscriptCandidate {
                artifact_id: "cached-microphone".to_string(),
                language: None,
                input: SourceTranscriptInput {
                    source: "microphone".to_string(),
                    text: "cached microphone context".to_string(),
                    valid: true,
                    warning: None,
                    recorded_silence: false,
                    start_ms: Some(0),
                    end_ms: Some(500),
                    turn_index: Some(0),
                },
            },
            TranscriptCandidate {
                artifact_id: "cached-system".to_string(),
                language: None,
                input: SourceTranscriptInput {
                    source: "system".to_string(),
                    text: "must not leak across lanes".to_string(),
                    valid: true,
                    warning: None,
                    recorded_silence: false,
                    start_ms: Some(0),
                    end_ms: Some(500),
                    turn_index: Some(1),
                },
            },
        ];

        let outcome = transcribe_turn_jobs_bounded(
            vec![test_job("m2", "microphone", 2)],
            Vec::new(),
            &cached_candidates,
            crate::providers::PROVIDER_GROQ.to_string(),
            "Meeting".to_string(),
            None,
            Arc::new(prepare_source_fallback),
            transcriber,
            None,
            1,
        )
        .await;
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
        outcome.expect("cached context should seed the source lane");

        let correction = correction_request_for(&server, "fresh microphone turn");
        assert!(correction.contains("Microphone: cached microphone context"));
        assert!(!correction.contains("must not leak across lanes"));
    }

    #[tokio::test]
    async fn source_lane_failures_keep_their_source_reason() {
        let transcriber = Arc::new(move |request: TranscriptionRequest| {
            Box::pin(async move {
                if request.audio_path == std::path::Path::new("s1") {
                    Err(AppError::new(
                        "transcription_failed",
                        "System source was silent.",
                    ))
                } else {
                    Ok(TranscriptionProviderResult {
                        text: request.audio_path.to_string_lossy().to_string(),
                        language: None,
                        provider: "test".to_string(),
                    })
                }
            }) as TranscriptionFuture
        }) as TurnTranscriber;

        let outcome = transcribe_turn_jobs_by_source_lane(
            vec![test_job("m0", "microphone", 0), test_job("s1", "system", 1)],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            transcriber,
        )
        .await
        .expect("source lanes should complete despite one failed source");

        assert_eq!(outcome.candidates.len(), 1);
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].input.source, "system");
        assert_eq!(
            source_failure_summary(&outcome.failures).as_deref(),
            Some("System: System source was silent.")
        );
    }

    #[tokio::test]
    async fn transient_invalid_turn_response_retries_before_failing() {
        // Historical transient failures without a provider result recover on
        // retry rather than failing the whole note.
        let transient_errors = [
            AppError::new(
                "clovy_api_response_invalid",
                "The processing service returned an invalid response.",
            ),
            AppError::new("clovy_request_failed", "authorization_denied"),
            AppError::new("june_request_failed", "authorization_denied"),
        ];

        for transient_error in transient_errors {
            let attempts = Arc::new(AtomicUsize::new(0));
            let transcriber = {
                let attempts = Arc::clone(&attempts);
                Arc::new(move |request: TranscriptionRequest| {
                    let attempts = Arc::clone(&attempts);
                    let transient_error = transient_error.clone();
                    Box::pin(async move {
                        if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                            return Err(transient_error);
                        }
                        Ok(TranscriptionProviderResult {
                            text: request.audio_path.to_string_lossy().to_string(),
                            language: None,
                            provider: "test".to_string(),
                        })
                    }) as TranscriptionFuture
                }) as TurnTranscriber
            };

            let outcome = transcribe_turn_jobs_by_source_lane(
                vec![test_job("m0", "microphone", 0)],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                transcriber,
            )
            .await
            .expect("transient response should be retried");

            assert_eq!(attempts.load(Ordering::SeqCst), 2);
            assert_eq!(outcome.failures.len(), 0);
            assert_eq!(outcome.candidates.len(), 1);
            assert_eq!(outcome.candidates[0].input.text, "m0");
        }
    }

    #[tokio::test]
    async fn exhausted_invalid_tail_turn_stays_visible_as_failure() {
        // When a turn fails after the allowed attempt budget, or fails with a
        // non-retryable provider/metering error, it stays a visible per-turn
        // failure without dropping earlier successful turns. The surfaced
        // warning is user-facing copy, never the raw provider code.
        let cases = [
            (
                AppError::new(
                    "clovy_api_response_invalid",
                    "The processing service returned an invalid response.",
                ),
                "The processing service returned an invalid response.",
                TRANSIENT_TRANSCRIPTION_ATTEMPTS,
            ),
            (
                AppError::new("june_request_failed", "upstream_provider_failed"),
                "The transcription provider could not process this audio.",
                1,
            ),
            (
                AppError::new("june_request_failed", "metering_provider_failed"),
                "The transcription provider could not process this audio.",
                1,
            ),
        ];

        for (tail_error, expected_warning, expected_attempts) in cases {
            let tail_attempts = Arc::new(AtomicUsize::new(0));
            let transcriber = {
                let tail_attempts = Arc::clone(&tail_attempts);
                Arc::new(move |request: TranscriptionRequest| {
                    let tail_attempts = Arc::clone(&tail_attempts);
                    let tail_error = tail_error.clone();
                    Box::pin(async move {
                        if request.audio_path == std::path::Path::new("tail") {
                            tail_attempts.fetch_add(1, Ordering::SeqCst);
                            return Err(tail_error);
                        }
                        Ok(TranscriptionProviderResult {
                            text: request.audio_path.to_string_lossy().to_string(),
                            language: None,
                            provider: "test".to_string(),
                        })
                    }) as TranscriptionFuture
                }) as TurnTranscriber
            };

            let outcome = transcribe_turn_jobs_by_source_lane(
                vec![
                    test_job("intro", "microphone", 0),
                    test_job("tail", "microphone", 1),
                ],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                transcriber,
            )
            .await
            .expect("source lanes should complete despite a failed tail turn");

            assert_eq!(tail_attempts.load(Ordering::SeqCst), expected_attempts);
            assert_eq!(outcome.candidates.len(), 1);
            assert_eq!(outcome.failures.len(), 1);
            assert_eq!(outcome.candidates[0].input.text, "intro");
            assert_eq!(outcome.failures[0].input.source, "microphone");
            assert_eq!(outcome.failures[0].input.turn_index, Some(1));
            assert_eq!(
                outcome.failures[0].input.warning.as_deref(),
                Some(expected_warning)
            );
        }
    }

    #[test]
    fn retry_after_delay_keeps_server_floor_and_adds_jitter() {
        let operation_id = (0..100)
            .map(|index| format!("retry-after-jitter-{index}"))
            .find(|operation_id| retry_jitter_ms(operation_id, 0) > 0)
            .expect("test jitter should produce a non-zero candidate");
        let mut error = AppError::new("june_request_failed", "authorization_denied");
        error.details = Some(serde_json::json!({ "retryAfterMs": 2_000 }));

        assert_eq!(
            transient_retry_delay(&operation_id, 0, &error),
            Duration::from_millis(2_000 + retry_jitter_ms(&operation_id, 0))
        );
    }

    #[test]
    fn server_timeout_envelope_is_retryable_but_client_timeout_is_not() {
        assert!(is_retryable_transcription_error(&AppError::new(
            "june_request_failed",
            "timeout"
        )));
        assert!(!is_retryable_transcription_error(&AppError::new(
            "june_request_failed",
            "operation timed out"
        )));
        // `upstream_provider_failed` is not precise enough for desktop retry:
        // Historical server errors did not distinguish transient 5xx responses
        // from deterministic provider 4xx failures.
        assert!(!is_retryable_transcription_error(&AppError::new(
            "june_request_failed",
            "upstream_provider_failed"
        )));
        // `metering_provider_failed` can come from a post-ASR charge failure;
        // replaying the desktop request would redo paid upstream work.
        assert!(!is_retryable_transcription_error(&AppError::new(
            "june_request_failed",
            "metering_provider_failed"
        )));
    }

    #[test]
    fn turn_operation_id_includes_source_when_turn_indices_match() {
        let mic = test_job("m0", "microphone", 0);
        let system = test_job("s0", "system", 0);

        assert_ne!(turn_operation_id(&mic), turn_operation_id(&system));
        assert_eq!(
            turn_operation_id(&mic),
            "artifact-m0-microphone-0-500-turn-0-fallback-false-saved-audio-v3"
        );
        assert_eq!(
            turn_operation_id(&system),
            "artifact-s0-system-0-500-turn-0-fallback-false-saved-audio-v3"
        );
    }

    #[test]
    fn transcription_failure_messages_hide_provider_codes() {
        assert_eq!(
            user_facing_transcription_failure_message(
                "june_request_failed",
                "no_speech",
                false,
                "microphone"
            ),
            "No speech detected. Try speaking louder or moving closer to the microphone."
        );
        assert_eq!(
            user_facing_transcription_failure_message(
                "june_request_failed",
                "no_speech",
                true,
                "microphone",
            ),
            "The microphone recorded silence for the whole session. Check that the right microphone is selected in Settings and that macOS input volume is up."
        );
        assert_eq!(
            user_facing_transcription_failure_message(
                "june_request_failed",
                "upstream_provider_failed",
                true,
                "microphone",
            ),
            "The transcription provider could not process this audio."
        );
        assert_eq!(
            user_facing_transcription_failure_message(
                "june_request_failed",
                "metering_provider_failed",
                true,
                "microphone",
            ),
            "The transcription provider could not process this audio."
        );
        // Regression (2026-07-14): the raw deny reason leaked into the failure
        // banner as "authorization_denied".
        assert_eq!(
            user_facing_transcription_failure_message(
                "june_request_failed",
                "authorization_denied",
                false,
                "microphone",
            ),
            "The service is busy right now. Wait a minute, then retry."
        );
    }

    #[test]
    fn generation_failures_are_not_mislabeled_as_transcription_failures() {
        let failure = note_generation_failure_for_user(AppError::new(
            "june_request_failed",
            "upstream_provider_failed",
        ));
        assert_eq!(
            failure.message,
            "Your recording was transcribed, but Clovy couldn't generate the note. Your transcript is saved."
        );

        let balance_failure = note_generation_failure_for_user(AppError::new(
            "june_request_failed",
            "Your balance is too low. Upgrade to continue.",
        ));
        assert_eq!(
            balance_failure.message,
            "Your recording was transcribed, but Clovy couldn't generate the note. Your transcript is saved."
        );
    }

    #[test]
    fn retired_account_failures_use_provider_copy_without_losing_error_details() {
        for (code, message) in [
            ("metering_failed", "metering_provider_failed"),
            (
                "insufficient_credits",
                "Your balance is too low. Upgrade to continue.",
            ),
            ("provider_failed", "insufficient_credits"),
            (
                "provider_failed",
                "Billing is temporarily unavailable. Please try again in a moment.",
            ),
        ] {
            assert_eq!(
                user_facing_transcription_failure_message(code, message, false, "microphone"),
                "The transcription provider could not process this audio."
            );
            let mut original = AppError::new(code, message);
            original.details = Some(serde_json::json!({ "retryAfterMs": 1500 }));
            let error = note_generation_failure_for_user(original);
            assert_eq!(error.code, code);
            assert_eq!(
                error.details,
                Some(serde_json::json!({ "retryAfterMs": 1500 }))
            );
            assert_eq!(
                error.message,
                "Your recording was transcribed, but Clovy couldn't generate the note. Your transcript is saved."
            );
        }
        assert_eq!(
            user_facing_transcription_failure_message(
                "authorization_denied",
                "",
                false,
                "microphone"
            ),
            "The service is busy right now. Wait a minute, then retry."
        );
        assert_eq!(
            note_generation_failure_for_user(AppError::new("authorization_denied", "")).message,
            "Your recording was transcribed, but note generation is busy right now. Your transcript is saved."
        );
    }

    #[test]
    fn a_generation_without_an_openrouter_key_or_clovy_api_names_the_missing_key() {
        let failure = note_generation_failure_for_user(AppError::new(
            "openrouter_api_key_missing",
            "Add your OpenRouter API key.",
        ));
        assert_eq!(failure.code, "openrouter_api_key_missing");
        assert_eq!(
            failure.message,
            "Your recording was transcribed, but Clovy needs your OpenRouter API key to write the note. Add it in Settings > Models > More options > Provider API keys. Your transcript is saved."
        );
    }

    #[test]
    fn a_missing_groq_key_reaches_the_note_unchanged() {
        assert_eq!(
            user_facing_transcription_failure_message(
                "groq_api_key_missing",
                crate::providers::groq::GROQ_API_KEY_MISSING_MESSAGE,
                false,
                "microphone",
            ),
            crate::providers::groq::GROQ_API_KEY_MISSING_MESSAGE
        );
    }

    /// Notes persist only these English sentences (no error code column), so
    /// the desktop translates them by exact text. Every sentence must exist
    /// verbatim in the English `notes.failure.*` catalog, or that failure
    /// would silently stop translating.
    #[test]
    fn stored_failure_sentences_match_the_desktop_catalog() {
        let catalog = include_str!("../../../src/i18n/messages/notes.ts");
        let sentences = [
            user_facing_transcription_failure_message("no_speech", "", false, "microphone"),
            user_facing_transcription_failure_message("no_speech", "", true, "system"),
            user_facing_transcription_failure_message("no_speech", "", true, "microphone"),
            user_facing_transcription_failure_message("x", "metering_provider_failed", false, "m"),
            user_facing_transcription_failure_message("x", "upstream_provider_failed", false, "m"),
            user_facing_transcription_failure_message("x", "authorization_denied", false, "m"),
            note_generation_failure_for_user(AppError::new("x", "authorization_denied")).message,
            note_generation_failure_for_user(AppError::new("x", "anything else")).message,
            user_facing_transcription_failure_message(
                "groq_api_key_missing",
                crate::providers::groq::GROQ_API_KEY_MISSING_MESSAGE,
                false,
                "m",
            ),
            user_facing_transcription_failure_message(
                "audio_too_large",
                crate::providers::groq::AUDIO_TOO_LARGE_MESSAGE,
                false,
                "m",
            ),
            note_generation_failure_for_user(AppError::new("openrouter_api_key_missing", "no key"))
                .message,
        ];
        for sentence in sentences {
            assert!(
                catalog.contains(&format!("\"{sentence}\"")),
                "missing from notes.ts: {sentence}"
            );
        }
    }

    #[test]
    fn transcript_coverage_counts_normal_turn_spans() {
        let mic_path = PathBuf::from("microphone.wav");
        let coverage = compute_transcript_coverage(
            &[(
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            )],
            &[
                test_audio_turn("mic-artifact", "microphone", mic_path.clone(), 0, 0, 50_000),
                test_audio_turn("mic-artifact", "microphone", mic_path, 1, 60_000, 130_000),
            ],
            &[test_transcript("microphone", 0, 0, 50_000)],
            &[],
        );

        assert_eq!(coverage.total_detected_speech_ms, 120_000);
        assert_eq!(coverage.total_transcribed_ms, 50_000);
        assert_eq!(coverage.total_detected_turns, 2);
        assert_eq!(coverage.total_transcribed_turns, 1);
        assert!(coverage.warning);
    }

    #[test]
    fn transcript_coverage_counts_full_source_sentinel_success_as_covered() {
        let dir =
            std::env::temp_dir().join(format!("clovy-coverage-success-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        write_loud_wav(&mic_path, 16_000, 16_000 * 90);
        let coverage = compute_transcript_coverage(
            &[(
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            )],
            &[test_audio_turn(
                "mic-artifact",
                "microphone",
                mic_path,
                0,
                0,
                0,
            )],
            &[test_transcript("microphone", 0, 0, 0)],
            &[],
        );

        assert_eq!(coverage.total_detected_speech_ms, 0);
        assert_eq!(coverage.total_transcribed_ms, 0);
        assert!(!coverage.warning);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transcript_coverage_counts_full_source_sentinel_failure_from_wav_duration() {
        let dir =
            std::env::temp_dir().join(format!("clovy-coverage-failure-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        write_loud_wav(&mic_path, 16_000, 16_000 * 90);
        let coverage = compute_transcript_coverage(
            &[(
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            )],
            &[test_audio_turn(
                "mic-artifact",
                "microphone",
                mic_path,
                0,
                0,
                0,
            )],
            &[],
            &[failed_candidate(
                "microphone",
                "The transcription service was unavailable. Please try again.",
                0,
            )],
        );

        assert_eq!(coverage.total_detected_speech_ms, 90_000);
        assert_eq!(coverage.total_transcribed_ms, 0);
        assert_eq!(coverage.total_failed_turns, 1);
        assert!(coverage.warning);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transcript_coverage_treats_visible_no_speech_sentinel_as_silent() {
        // Microphone no-speech failures stay visible (only system no-speech
        // is suppressed), but a no-speech sentinel is still silence: both
        // detectors agreed there was nothing to transcribe.
        let dir =
            std::env::temp_dir().join(format!("clovy-coverage-nospeech-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        write_loud_wav(&mic_path, 16_000, 16_000 * 90);
        let coverage = compute_transcript_coverage(
            &[(
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            )],
            &[test_audio_turn(
                "mic-artifact",
                "microphone",
                mic_path,
                0,
                0,
                0,
            )],
            &[],
            &[failed_candidate(
                "microphone",
                "No speech detected. Try speaking louder or moving closer to the microphone.",
                0,
            )],
        );

        assert_eq!(coverage.total_detected_speech_ms, 0);
        assert_eq!(coverage.total_transcribed_ms, 0);
        assert_eq!(coverage.total_failed_turns, 1);
        assert!(!coverage.warning);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transcript_coverage_treats_suppressed_silent_sentinel_as_no_detected_speech() {
        // A muted microphone in a dual-source meeting: sentinel turn, no
        // persisted row, and the no-speech failure was suppressed (not
        // visible). The silent source must not warn.
        let dir =
            std::env::temp_dir().join(format!("clovy-coverage-silent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        write_loud_wav(&mic_path, 16_000, 16_000 * 90);
        let coverage = compute_transcript_coverage(
            &[(
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            )],
            &[test_audio_turn(
                "mic-artifact",
                "microphone",
                mic_path,
                0,
                0,
                0,
            )],
            &[],
            &[],
        );

        assert_eq!(coverage.total_detected_speech_ms, 0);
        assert_eq!(coverage.total_transcribed_ms, 0);
        assert_eq!(coverage.total_failed_turns, 0);
        assert!(!coverage.warning);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn transcript_coverage_excludes_unknown_duration_sentinel_failure() {
        let missing_path = PathBuf::from("missing.wav");
        let coverage = compute_transcript_coverage(
            &[(
                "mic-artifact".to_string(),
                "microphone".to_string(),
                missing_path.clone(),
                false,
            )],
            &[test_audio_turn(
                "mic-artifact",
                "microphone",
                missing_path,
                0,
                0,
                0,
            )],
            &[],
            &[],
        );

        assert_eq!(coverage.total_detected_speech_ms, 0);
        assert_eq!(coverage.total_transcribed_ms, 0);
        assert!(!coverage.warning);
    }

    #[test]
    fn transcript_coverage_clamps_invalid_spans() {
        let mic_path = PathBuf::from("microphone.wav");
        let coverage = compute_transcript_coverage(
            &[(
                "mic-artifact".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            )],
            &[test_audio_turn(
                "mic-artifact",
                "microphone",
                mic_path,
                0,
                40_000,
                10_000,
            )],
            &[test_transcript("microphone", 0, 50_000, 45_000)],
            &[],
        );

        assert_eq!(coverage.total_detected_speech_ms, 0);
        assert_eq!(coverage.total_transcribed_ms, 0);
        assert!(!coverage.warning);
    }

    #[test]
    fn transcript_coverage_warning_requires_ratio_and_absolute_floor() {
        assert!(transcript_coverage_warning(300_000, 239_999));
        assert!(!transcript_coverage_warning(300_000, 240_000));
        assert!(!transcript_coverage_warning(100_000, 41_000));
        assert!(transcript_coverage_warning(100_000, 40_000));
    }

    #[tokio::test]
    async fn microphone_only_coverage_persists_provider_no_speech_gap() {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repos = Repositories::new(pool);
        let note = repos.create_note("default", None).await.expect("note");
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        repos
            .create_recording_session(
                &note.id,
                &session_id,
                RecordingSourceMode::MicrophoneOnly,
                "microphone.partial.wav",
                "microphone.wav",
                None,
            )
            .await
            .expect("recording session");

        persist_microphone_only_transcript_coverage(
            &repos,
            &session_id,
            "microphone",
            &[
                TranscriptionChunkOutcome {
                    start_ms: 0,
                    end_ms: 30_000,
                    locally_silent: false,
                    provider_returned_text: true,
                    provider_no_speech: false,
                },
                TranscriptionChunkOutcome {
                    start_ms: 30_000,
                    end_ms: 120_000,
                    locally_silent: false,
                    provider_returned_text: false,
                    provider_no_speech: true,
                },
                TranscriptionChunkOutcome {
                    start_ms: 120_000,
                    end_ms: 150_000,
                    locally_silent: false,
                    provider_returned_text: true,
                    provider_no_speech: false,
                },
            ],
        )
        .await;

        let note = repos.get_note(&note.id).await.expect("note with coverage");
        let coverage = note.transcript_coverage.expect("coverage dto");
        assert_eq!(coverage.detected_speech_ms, 150_000);
        assert_eq!(coverage.transcribed_ms, 60_000);
        assert!(coverage.warning);

        let details: String = sqlx::query_scalar::query_scalar(
            "SELECT details FROM recording_checkpoints WHERE recording_session_id = ? AND kind = 'transcript_coverage'",
        )
        .bind(&session_id)
        .fetch_one(&repos.pool)
        .await
        .expect("checkpoint details");
        let checkpoint: serde_json::Value =
            serde_json::from_str(&details).expect("checkpoint json");
        assert_eq!(checkpoint["totalFailedTurns"], 1);
        assert_eq!(checkpoint["sources"][0]["failedTurns"], 1);
    }

    #[test]
    fn microphone_only_coverage_with_no_chunks_documents_zero_detected_speech() {
        let coverage = compute_chunk_transcript_coverage("microphone", &[]);
        assert_eq!(coverage.total_detected_speech_ms, 0);
        assert_eq!(coverage.total_transcribed_ms, 0);
        assert!(!coverage.warning);
    }

    #[test]
    fn microphone_only_coverage_ignores_locally_silent_chunks() {
        let coverage = compute_chunk_transcript_coverage(
            "microphone",
            &[
                TranscriptionChunkOutcome {
                    start_ms: 0,
                    end_ms: 30_000,
                    locally_silent: false,
                    provider_returned_text: true,
                    provider_no_speech: false,
                },
                TranscriptionChunkOutcome {
                    start_ms: 30_000,
                    end_ms: 120_000,
                    locally_silent: true,
                    provider_returned_text: false,
                    provider_no_speech: false,
                },
                TranscriptionChunkOutcome {
                    start_ms: 120_000,
                    end_ms: 150_000,
                    locally_silent: false,
                    provider_returned_text: true,
                    provider_no_speech: false,
                },
            ],
        );

        assert_eq!(coverage.total_detected_speech_ms, 60_000);
        assert_eq!(coverage.total_transcribed_ms, 60_000);
        assert_eq!(coverage.total_failed_turns, 0);
        assert!(!coverage.warning);
    }

    #[test]
    fn source_failure_summary_suppresses_silent_system_when_microphone_failed() {
        let summary = source_failure_summary(&[
            FailedTranscriptCandidate {
                artifact_id: "mic".to_string(),
                input: SourceTranscriptInput {
                    source: "microphone".to_string(),
                    text: String::new(),
                    valid: false,
                    warning: Some(
                        "The transcription provider could not process this audio.".to_string(),
                    ),
                    recorded_silence: false,
                    start_ms: Some(0),
                    end_ms: Some(0),
                    turn_index: Some(0),
                },
            },
            FailedTranscriptCandidate {
                artifact_id: "system".to_string(),
                input: SourceTranscriptInput {
                    source: "system".to_string(),
                    text: String::new(),
                    valid: false,
                    warning: Some(
                        "No speech detected. Try speaking louder or moving closer to the microphone."
                            .to_string(),
                    ),
                    recorded_silence: false,
                    start_ms: Some(0),
                    end_ms: Some(0),
                    turn_index: Some(1),
                },
            },
        ]);

        assert_eq!(
            summary.as_deref(),
            Some("Microphone: The transcription provider could not process this audio.")
        );
    }

    #[test]
    fn drops_silent_system_failure_once_a_source_succeeded() {
        // Solo mic recording: the system track is silent. With a valid
        // transcript present, that no_speech must not be recorded as a
        // per-source error (it rendered as a spurious "System" card).
        assert!(!should_record_source_failure(
            "system",
            "No speech detected. Try speaking louder or moving closer to the microphone.",
            true,
        ));
    }

    #[test]
    fn keeps_invalid_service_response_failure_once_a_source_succeeded() {
        assert!(should_record_source_failure(
            "microphone",
            "The processing service returned an invalid response.",
            true,
        ));
    }

    #[test]
    fn keeps_invalid_service_response_failure_when_nothing_succeeded() {
        assert!(should_record_source_failure(
            "microphone",
            "The processing service returned an invalid response.",
            false,
        ));
    }

    #[test]
    fn keeps_system_failure_when_nothing_else_succeeded() {
        // Everything failed (e.g. system-only capture of silence): keep it so
        // the user learns the recording produced nothing.
        assert!(should_record_source_failure("system", "no_speech", false));
    }

    #[test]
    fn keeps_non_no_speech_system_failures() {
        // A real provider error on the system track is still worth surfacing.
        assert!(should_record_source_failure(
            "system",
            "The transcription provider could not process this audio.",
            true,
        ));
    }

    #[test]
    fn no_speech_failures_do_not_block_partial_note_generation() {
        let visible = visible_transcription_failures(
            &[failed_candidate(
                "microphone",
                "No speech detected. Try speaking louder or moving closer to the microphone.",
                2,
            )],
            true,
        );

        assert_eq!(visible.len(), 1);
        assert!(blocking_transcription_failure_summary(&visible).is_none());
    }

    #[test]
    fn validated_silent_microphone_does_not_block_successful_system_transcript() {
        let visible = vec![FailedTranscriptCandidate {
            artifact_id: "mic".to_string(),
            input: SourceTranscriptInput {
                source: "microphone".to_string(),
                text: String::new(),
                valid: false,
                warning: Some(
                    "The microphone recorded silence for the whole session. Check that the right microphone is selected in Settings and that macOS input volume is up."
                        .to_string(),
                ),
                recorded_silence: true,
                start_ms: Some(0),
                end_ms: Some(0),
                turn_index: Some(0),
            },
        }];

        assert!(blocking_transcription_failure_summary(&visible).is_none());
    }

    #[test]
    fn invalid_turn_failures_block_partial_note_generation() {
        let visible = visible_transcription_failures(
            &[failed_candidate(
                "microphone",
                "The processing service returned an invalid response.",
                5,
            )],
            true,
        );

        assert_eq!(
            blocking_transcription_failure_summary(&visible).as_deref(),
            Some("Microphone: The processing service returned an invalid response.")
        );
    }

    #[test]
    fn never_drops_microphone_failures() {
        assert!(should_record_source_failure(
            "microphone",
            "no_speech",
            true
        ));
    }

    fn write_test_wav(path: &std::path::Path, samples: &[i16]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// Loud tone, above the silence floor, so every chunk survives the local
    /// silence prefilter and reaches the (mock) transcriber.
    fn write_loud_wav(path: &std::path::Path, sample_rate: u32, sample_count: usize) {
        write_segmented_wav(path, sample_rate, &[(sample_count, 20_000)]);
    }

    /// Writes consecutive `(frame_count, amplitude)` segments. Amplitude `0`
    /// yields a silent span; a large amplitude yields an audible tone.
    fn write_segmented_wav(path: &std::path::Path, sample_rate: u32, segments: &[(usize, i16)]) {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for (count, amplitude) in segments {
            for index in 0..*count {
                let sample = if index % 2 == 0 {
                    *amplitude
                } else {
                    -*amplitude
                };
                writer.write_sample(sample).unwrap();
            }
        }
        writer.finalize().unwrap();
    }

    #[test]
    fn is_no_speech_error_detects_no_speech_conditions() {
        assert!(is_no_speech_error(&AppError::new("no_speech", "no_speech")));
        assert!(is_no_speech_error(&AppError::new(
            "june_request_failed",
            "no_speech"
        )));
        assert!(!is_no_speech_error(&AppError::new(
            "clovy_api_response_invalid",
            "The processing service returned an invalid response."
        )));
    }

    #[test]
    fn short_cached_full_source_sentinel_is_not_reused() {
        let dir = std::env::temp_dir().join(format!(
            "clovy-short-cached-sentinel-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("short-source.wav");
        write_loud_wav(&audio_path, 16_000, 16_000 * 10);
        let turn = test_audio_turn("artifact", "microphone", audio_path, 0, 0, 0);
        let cached = test_transcript("microphone", 0, 0, 0);

        assert!(!cached_turn_is_reusable(&cached, &turn));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn short_full_source_vad_miss_uses_live_preview_chunk_size() {
        let dir = std::env::temp_dir().join(format!(
            "clovy-short-full-source-chunks-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("short-source.wav");
        write_loud_wav(&audio_path, 16_000, 16_000 * 17);
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let operation_ids = Arc::clone(&operation_ids);
            Arc::new(move |request: TranscriptionRequest| {
                let operation_ids = Arc::clone(&operation_ids);
                Box::pin(async move {
                    let operation_id = request.operation_id();
                    operation_ids.lock().unwrap().push(operation_id.clone());
                    Ok(test_provider_result(&operation_id))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let event = transcribe_one_turn_job(
            TurnTranscriptionJob {
                artifact_id: "artifact".to_string(),
                source: "system".to_string(),
                audio_path,
                temp_dir: dir.clone(),
                recorded_silence: false,
                covers_full_source: true,
                source_fallback: false,
                start_ms: 0,
                end_ms: 0,
                turn_index: 0,
                durable_job: None,
            },
            "test".to_string(),
            "Meeting".to_string(),
            None,
            None,
            transcriber,
            None,
        )
        .await
        .expect("short full-source recovery should transcribe");

        assert!(matches!(
            event.result,
            TurnTranscriptionResult::Candidate(_)
        ));
        let operation_ids = operation_ids.lock().unwrap();
        assert_eq!(operation_ids.len(), 3);
        assert!(operation_ids[0].ends_with("-chunk-0"));
        assert!(operation_ids[1].ends_with("-chunk-1"));
        assert!(operation_ids[2].ends_with("-chunk-2"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn durable_configuration_fingerprint_tracks_output_affecting_context() {
        let model = crate::providers::DEFAULT_TRANSCRIPTION_MODEL;
        let base = note_transcription_configuration_fingerprint("Meeting", Some("- June"), model);
        assert_eq!(
            base,
            note_transcription_configuration_fingerprint("Meeting", Some("- June"), model)
        );
        assert_ne!(
            base,
            note_transcription_configuration_fingerprint("Renamed meeting", Some("- June"), model)
        );
        assert_ne!(
            base,
            note_transcription_configuration_fingerprint("Meeting", Some("- Junho"), model)
        );
    }

    #[test]
    fn the_transcription_model_is_part_of_a_jobs_durable_identity() {
        let default = note_transcription_configuration_fingerprint(
            "Meeting",
            Some("- June"),
            crate::providers::DEFAULT_TRANSCRIPTION_MODEL,
        );
        // Turns already transcribed with the default model keep the identity
        // they had before the model was part of it, so they are reused.
        assert_eq!(
            default,
            "afe5ed88fc712a2454bbb35a912fb57f97507c86f29886c5091b027ac4ab6579"
        );
        // Another model is another transcription: no finished turn is reused.
        assert_ne!(
            default,
            note_transcription_configuration_fingerprint(
                "Meeting",
                Some("- June"),
                "whisper-large-v3"
            )
        );
    }

    #[tokio::test]
    async fn durable_job_is_claimed_before_provider_and_uses_ledger_operation_id() {
        let dir =
            std::env::temp_dir().join(format!("clovy-durable-job-claim-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("turn.wav");
        write_loud_wav(&audio_path, 16_000, 16_000);
        let claimed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let claimer = {
            let claimed = Arc::clone(&claimed);
            Arc::new(move |job_id: String| {
                let claimed = Arc::clone(&claimed);
                Box::pin(async move {
                    assert_eq!(job_id, "durable-span");
                    claimed.store(true, Ordering::SeqCst);
                    Ok(())
                }) as TurnClaimFuture
            }) as TurnJobClaimer
        };
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let claimed = Arc::clone(&claimed);
            let operation_ids = Arc::clone(&operation_ids);
            Arc::new(move |request: TranscriptionRequest| {
                assert!(claimed.load(Ordering::SeqCst));
                operation_ids.lock().unwrap().push(request.operation_id());
                Box::pin(async { Ok(test_provider_result("durable text")) }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let mut job = test_job(audio_path.to_str().unwrap(), "microphone", 0);
        job.temp_dir = dir.clone();
        job.durable_job = Some(test_durable_job(
            "durable-span",
            "durable-span:full-input-fingerprint",
        ));

        let event = transcribe_one_turn_job(
            job,
            "test".to_string(),
            "Meeting".to_string(),
            None,
            None,
            transcriber,
            Some(claimer),
        )
        .await
        .expect("durable job should transcribe");

        assert_eq!(event.job_id.as_deref(), Some("durable-span"));
        assert_eq!(
            operation_ids.lock().unwrap().as_slice(),
            ["durable-span:full-input-fingerprint"]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn multi_chunk_turn_keeps_earlier_text_when_trailing_chunk_has_no_speech() {
        // A 31s turn split at 30 s gives a 30s chunk-0 and a ~1s chunk-1. The
        // trailing chunk returns no-speech; the turn must still succeed with
        // chunk-0's text instead of aborting and discarding it.
        let dir =
            std::env::temp_dir().join(format!("clovy-chunk-nospeech-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("turn.wav");
        write_loud_wav(&audio_path, 16_000, 16_000 * 31);

        let transcriber = Arc::new(move |request: TranscriptionRequest| {
            Box::pin(async move {
                if request.operation_id().ends_with("-chunk-0") {
                    Ok(TranscriptionProviderResult {
                        text: "first chunk speech".to_string(),
                        language: Some("en".to_string()),
                        provider: "test".to_string(),
                    })
                } else {
                    Err(AppError::new("no_speech", "no_speech"))
                }
            }) as TranscriptionFuture
        }) as TurnTranscriber;

        let result = transcribe_prepared_audio(
            transcriber,
            TranscribePreparedAudioRequest {
                provider: "test".to_string(),
                audio_path,
                temp_dir: dir.clone(),
                chunk_stem: "turn-0".to_string(),
                title: "Meeting".to_string(),
                asr_hint: None,
                operation_id: "turn-0".to_string(),
                max_chunk_ms: Some(LEGACY_TRANSCRIPTION_SPAN_MS),
            },
        )
        .await
        .expect("a trailing no-speech chunk must not fail the whole turn");

        assert_eq!(result.transcript.text, "first chunk speech");
        assert_eq!(
            result.chunk_outcomes,
            vec![
                TranscriptionChunkOutcome {
                    start_ms: 0,
                    end_ms: 30_000,
                    locally_silent: false,
                    provider_returned_text: true,
                    provider_no_speech: false,
                },
                TranscriptionChunkOutcome {
                    start_ms: 30_000,
                    end_ms: 31_000,
                    locally_silent: false,
                    provider_returned_text: false,
                    provider_no_speech: true,
                },
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn multi_chunk_turn_reports_no_speech_when_every_chunk_is_silent() {
        // When no chunk has speech, the turn must fail as a no-speech condition
        // so it stays non-blocking — not a generic error that fails the note.
        let dir =
            std::env::temp_dir().join(format!("clovy-chunk-allsilent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("turn.wav");
        write_loud_wav(&audio_path, 16_000, 16_000 * 31);

        let transcriber = Arc::new(move |_request: TranscriptionRequest| {
            Box::pin(async move { Err(AppError::new("no_speech", "no_speech")) })
                as TranscriptionFuture
        }) as TurnTranscriber;

        let error = transcribe_prepared_audio(
            transcriber,
            TranscribePreparedAudioRequest {
                provider: "test".to_string(),
                audio_path,
                temp_dir: dir.clone(),
                chunk_stem: "turn-0".to_string(),
                title: "Meeting".to_string(),
                asr_hint: None,
                operation_id: "turn-0".to_string(),
                max_chunk_ms: None,
            },
        )
        .await
        .expect_err("an all-silent turn must fail");

        assert!(
            is_no_speech_error(&error),
            "all-silent turn must stay a non-blocking no-speech failure, got code={} message={}",
            error.code,
            error.message
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn single_chunk_silent_audio_is_skipped_before_reaching_the_transcriber() {
        let dir = std::env::temp_dir().join(format!(
            "clovy-single-chunk-silentskip-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("turn.wav");
        write_segmented_wav(&audio_path, 16_000, &[(16_000, 0)]);

        let calls = Arc::new(AtomicUsize::new(0));
        let transcriber = {
            let calls = Arc::clone(&calls);
            Arc::new(move |_request: TranscriptionRequest| {
                let calls = Arc::clone(&calls);
                Box::pin(async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(TranscriptionProviderResult {
                        text: "should not be called".to_string(),
                        language: None,
                        provider: "test".to_string(),
                    })
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let error = transcribe_prepared_audio(
            transcriber,
            TranscribePreparedAudioRequest {
                provider: "test".to_string(),
                audio_path,
                temp_dir: dir.clone(),
                chunk_stem: "turn-0".to_string(),
                title: "Meeting".to_string(),
                asr_hint: None,
                operation_id: "turn-0".to_string(),
                max_chunk_ms: None,
            },
        )
        .await
        .expect_err("silent single-chunk audio should report no speech");

        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(is_no_speech_error(&error));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn single_chunk_empty_provider_result_is_no_speech_not_success() {
        let dir = std::env::temp_dir().join(format!(
            "clovy-single-chunk-empty-result-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("turn.wav");
        write_segmented_wav(&audio_path, 16_000, &[(16_000, 20_000)]);

        let transcriber = Arc::new(move |_request: TranscriptionRequest| {
            Box::pin(async move {
                Ok(TranscriptionProviderResult {
                    text: "   ".to_string(),
                    language: None,
                    provider: "test".to_string(),
                })
            }) as TranscriptionFuture
        }) as TurnTranscriber;

        let error = transcribe_prepared_audio(
            transcriber,
            TranscribePreparedAudioRequest {
                provider: "test".to_string(),
                audio_path,
                temp_dir: dir.clone(),
                chunk_stem: "turn-0".to_string(),
                title: "Meeting".to_string(),
                asr_hint: None,
                operation_id: "turn-0".to_string(),
                max_chunk_ms: None,
            },
        )
        .await
        .expect_err("an empty provider result must not certify a transcript");

        assert!(is_no_speech_error(&error));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn silent_chunks_are_skipped_before_reaching_the_transcriber() {
        // 62s split at 30 s: loud 0-30s, silent 30-60s, loud 60-62s -> chunks
        // 0 and 2 audible, chunk 1 silent. The silent chunk must never reach
        // the API (no credit hold), while both audible chunks are transcribed.
        let dir =
            std::env::temp_dir().join(format!("clovy-chunk-silentskip-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio_path = dir.join("turn.wav");
        write_segmented_wav(
            &audio_path,
            16_000,
            &[
                (16_000 * 30, 20_000),
                (16_000 * 30, 0),
                (16_000 * 2, 20_000),
            ],
        );

        let calls = Arc::new(AtomicUsize::new(0));
        let transcriber = {
            let calls = Arc::clone(&calls);
            Arc::new(move |_request: TranscriptionRequest| {
                let calls = Arc::clone(&calls);
                Box::pin(async move {
                    let n = calls.fetch_add(1, Ordering::SeqCst);
                    Ok(TranscriptionProviderResult {
                        text: format!("chunk text {n}"),
                        language: None,
                        provider: "test".to_string(),
                    })
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let result = transcribe_prepared_audio(
            transcriber,
            TranscribePreparedAudioRequest {
                provider: "test".to_string(),
                audio_path,
                temp_dir: dir.clone(),
                chunk_stem: "turn-0".to_string(),
                title: "Meeting".to_string(),
                asr_hint: None,
                operation_id: "turn-0".to_string(),
                max_chunk_ms: Some(LEGACY_TRANSCRIPTION_SPAN_MS),
            },
        )
        .await
        .expect("audible chunks should transcribe");

        // Only the two audible chunks reach the API; the silent middle is skipped.
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(result.transcript.text, "chunk text 0\nchunk text 1");
        assert_eq!(
            result.chunk_outcomes,
            vec![
                TranscriptionChunkOutcome {
                    start_ms: 0,
                    end_ms: 30_000,
                    locally_silent: false,
                    provider_returned_text: true,
                    provider_no_speech: false,
                },
                TranscriptionChunkOutcome {
                    start_ms: 30_000,
                    end_ms: 60_000,
                    locally_silent: true,
                    provider_returned_text: false,
                    provider_no_speech: false,
                },
                TranscriptionChunkOutcome {
                    start_ms: 60_000,
                    end_ms: 62_000,
                    locally_silent: false,
                    provider_returned_text: true,
                    provider_no_speech: false,
                },
            ]
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drops_silent_system_source_but_keeps_microphone() {
        let dir = std::env::temp_dir().join(format!("clovy-drop-silent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");
        write_test_wav(&mic_path, &vec![20_000; 48_000]);
        write_test_wav(&system_path, &[0, 0, 0, 0, 1, -1]);

        let kept = drop_silent_system_sources(vec![
            (
                "mic".to_string(),
                "microphone".to_string(),
                mic_path.clone(),
                false,
            ),
            ("sys".to_string(), "system".to_string(), system_path, true),
        ]);

        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].1, "microphone");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_silent_system_source_when_it_is_the_only_one() {
        let dir = std::env::temp_dir().join(format!("clovy-drop-silent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let system_path = dir.join("system.wav");
        write_test_wav(&system_path, &[0, 0, 0, 0]);

        let kept = drop_silent_system_sources(vec![(
            "sys".to_string(),
            "system".to_string(),
            system_path,
            true,
        )]);

        // System-only capture of silence must survive so its "no speech"
        // failure still reaches the user.
        assert_eq!(kept.len(), 1);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_quiet_system_source_between_detection_and_silence_floors() {
        let dir = std::env::temp_dir().join(format!("clovy-drop-silent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");
        write_test_wav(&mic_path, &vec![20_000; 48_000]);
        // ~0.008 RMS: detectable by the system lane (min_rms 0.006) but under the
        // 0.012 normalized-chunk silence floor. Must survive the pre-filter so the
        // full-source fallback can still transcribe it.
        let amplitude = (0.008 * i16::MAX as f32).round() as i16;
        let quiet = vec![amplitude; 48_000];
        write_test_wav(&system_path, &quiet);

        let kept = drop_silent_system_sources(vec![
            ("mic".to_string(), "microphone".to_string(), mic_path, false),
            (
                "sys".to_string(),
                "system".to_string(),
                system_path.clone(),
                false,
            ),
        ]);

        assert_eq!(kept.len(), 2);
        // The old 0.012 floor still judges it silent — the chunk-skip guard is
        // intentionally left stricter than the pre-filter.
        assert!(crate::audio::turns::source_is_effectively_silent(
            &system_path
        ));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keeps_audible_system_source() {
        let dir = std::env::temp_dir().join(format!("clovy-drop-silent-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");
        write_test_wav(&mic_path, &vec![20_000; 48_000]);
        write_test_wav(&system_path, &vec![15_000; 48_000]);

        let kept = drop_silent_system_sources(vec![
            ("mic".to_string(), "microphone".to_string(), mic_path, false),
            ("sys".to_string(), "system".to_string(), system_path, false),
        ]);

        assert_eq!(kept.len(), 2);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn drops_system_source_with_only_a_startup_transient() {
        let dir = std::env::temp_dir().join(format!(
            "clovy-drop-transient-system-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mic_path = dir.join("microphone.wav");
        let system_path = dir.join("system.wav");
        write_test_wav(&mic_path, &vec![20_000; 48_000]);
        let mut system_samples = vec![0_i16; 48_000 * 10];
        system_samples[..7_200].fill(15_000);
        write_test_wav(&system_path, &system_samples);

        let outcome = partition_silent_system_sources(vec![
            ("mic".to_string(), "microphone".to_string(), mic_path, false),
            ("sys".to_string(), "system".to_string(), system_path, false),
        ]);

        assert_eq!(outcome.kept.len(), 1);
        assert_eq!(outcome.dropped.len(), 1);
        assert_eq!(outcome.dropped[0].longest_active_ms, 150);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn pipeline_starts_first_request_before_later_preparation_finishes() {
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
        ];
        let (later_preparation_began_tx, mut later_preparation_began_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let (release_later_preparation_tx, release_later_preparation_rx) =
            std::sync::mpsc::channel();
        let release_later_preparation_rx = Arc::new(Mutex::new(release_later_preparation_rx));
        let later_preparation_finished = Arc::new(AtomicBool::new(false));
        let turn_preparer = {
            let release_later_preparation_rx = Arc::clone(&release_later_preparation_rx);
            let later_preparation_finished = Arc::clone(&later_preparation_finished);
            Arc::new(move |descriptor: TurnPreparationJob| {
                if descriptor.schedule_index == 1 {
                    later_preparation_began_tx.send(()).unwrap();
                    release_later_preparation_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
                    later_preparation_finished.store(true, Ordering::SeqCst);
                }
                Ok(prepared_test_turn(&descriptor))
            }) as TurnPreparer
        };
        let (provider_started_tx, mut provider_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let transcriber = Arc::new(move |request: TranscriptionRequest| {
            provider_started_tx.send(request.operation_id()).unwrap();
            Box::pin(async move { Ok(test_provider_result(&request.operation_id())) })
                as TranscriptionFuture
        }) as TurnTranscriber;

        let mut pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                descriptors,
                Vec::new(),
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                turn_preparer,
                Arc::new(|plan| Ok(fake_fallback_job(plan))),
                transcriber,
                None,
                None,
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::untracked(),
            )
            .await
        });

        let preparation_began =
            tokio::time::timeout(Duration::from_secs(2), later_preparation_began_rx.recv()).await;
        let provider_started =
            tokio::time::timeout(Duration::from_secs(2), provider_started_rx.recv()).await;
        let overlapped = !later_preparation_finished.load(Ordering::SeqCst);
        let _ = release_later_preparation_tx.send(());
        let pipeline_result = tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await;
        if pipeline_result.is_err() {
            pipeline.abort();
        }

        assert!(preparation_began.is_ok(), "later preparation never began");
        assert!(
            provider_started.is_ok(),
            "the first provider request never began"
        );
        assert!(
            overlapped,
            "the first provider request waited for later preparation"
        );
        let result = pipeline_result
            .expect("pipeline timed out")
            .expect("pipeline task panicked")
            .expect("pipeline should succeed");
        assert_eq!(result.preparation.prepared_count, 2);
        assert_eq!(result.outcome.candidates.len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn streaming_scheduler_never_exceeds_two_provider_calls() {
        let descriptors = (0..5)
            .map(|index| {
                turn_preparation_job(
                    index,
                    if index % 2 == 0 {
                        "microphone"
                    } else {
                        "system"
                    },
                    index as i64,
                    false,
                )
            })
            .collect::<Vec<_>>();
        let (fifth_preparation_started_tx, mut fifth_preparation_started_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let turn_preparer = Arc::new(move |descriptor: TurnPreparationJob| {
            if descriptor.schedule_index == 4 {
                fifth_preparation_started_tx.send(()).unwrap();
            }
            Ok(prepared_test_turn(&descriptor))
        }) as TurnPreparer;
        let (receiver, mut producer) = spawn_turn_preparation(
            descriptors.clone(),
            turn_preparer,
            ProcessingTiming::untracked(),
        );
        let active = Arc::new(AtomicUsize::new(0));
        let max_active = Arc::new(AtomicUsize::new(0));
        let provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let transcriber = {
            let active = Arc::clone(&active);
            let max_active = Arc::clone(&max_active);
            let provider_gate = Arc::clone(&provider_gate);
            Arc::new(move |request: TranscriptionRequest| {
                let active = Arc::clone(&active);
                let max_active = Arc::clone(&max_active);
                let provider_gate = Arc::clone(&provider_gate);
                let started_tx = started_tx.clone();
                let operation_id = request.operation_id();
                Box::pin(async move {
                    let now_active = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_active.fetch_max(now_active, Ordering::SeqCst);
                    started_tx.send(operation_id.clone()).unwrap();
                    let permit = provider_gate.acquire_owned().await.unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                    Ok(test_provider_result(&operation_id))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let mut scheduler = tokio::spawn(transcribe_prepared_turn_stream(
            receiver,
            descriptors.len(),
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            &[],
            transcriber,
            None,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        ));

        let first_started = tokio::time::timeout(Duration::from_secs(2), started_rx.recv()).await;
        let second_started = tokio::time::timeout(Duration::from_secs(2), started_rx.recv()).await;
        let fifth_preparation_started =
            tokio::time::timeout(Duration::from_secs(2), fifth_preparation_started_rx.recv()).await;
        let third_started_before_release = started_rx.try_recv().ok();
        let active_before_release = active.load(Ordering::SeqCst);
        let max_before_release = max_active.load(Ordering::SeqCst);
        let producer_probe = tokio::time::timeout(Duration::from_millis(50), &mut producer).await;
        let producer_was_backpressured = producer_probe.is_err();
        provider_gate.add_permits(5);
        let scheduler_result = tokio::time::timeout(Duration::from_secs(2), &mut scheduler).await;
        let producer_result = match producer_probe {
            Ok(result) => Ok(result),
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut producer).await,
        };
        if scheduler_result.is_err() {
            scheduler.abort();
        }
        if producer_result.is_err() {
            producer.abort();
        }

        let mut started_before_release = vec![
            first_started
                .expect("the first provider observation timed out")
                .expect("the first provider channel closed"),
            second_started
                .expect("the second provider observation timed out")
                .expect("the second provider channel closed"),
        ];
        started_before_release.sort_unstable();
        assert_eq!(
            started_before_release,
            vec![
                "artifact-microphone-100-600-turn-0-fallback-false-saved-audio-v3".to_string(),
                "artifact-system-1100-1600-turn-1-fallback-false-saved-audio-v3".to_string(),
            ]
        );
        assert!(
            fifth_preparation_started.is_ok(),
            "the producer never prepared its fifth descriptor"
        );
        assert!(
            third_started_before_release.is_none(),
            "a third provider call started before capacity was released"
        );
        assert!(
            producer_was_backpressured,
            "the fifth send was not blocked behind two active and two buffered jobs"
        );
        assert_eq!(active_before_release, 2);
        assert_eq!(max_before_release, 2);
        let outcome = scheduler_result
            .expect("scheduler timed out")
            .expect("scheduler task panicked")
            .expect("scheduler should succeed");
        assert_eq!(outcome.candidates.len(), 5);
        let report = producer_result
            .expect("producer timed out")
            .expect("producer task panicked");
        assert_eq!(report.prepared_count, 5);
        assert!(report.error.is_none());
        assert_eq!(max_active.load(Ordering::SeqCst), 2);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn preparation_report_separates_active_time_from_backpressure() {
        const ACTIVE_WORK_MS: u64 = 15;
        const BACKPRESSURE_PROBE_MS: u64 = 60;
        const POST_PREPARATION_HOLD_MS: u64 = 60;

        let descriptors = (0..5)
            .map(|index| {
                turn_preparation_job(
                    index,
                    if index % 2 == 0 {
                        "microphone"
                    } else {
                        "system"
                    },
                    index as i64,
                    false,
                )
            })
            .collect::<Vec<_>>();
        let (fifth_active_work_finished_tx, mut fifth_active_work_finished_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let (producer_finished_tx, mut producer_finished_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let preparation_calls = Arc::new(AtomicUsize::new(0));
        let turn_preparer = {
            let preparation_calls = Arc::clone(&preparation_calls);
            let producer_finished = DropNotifier(Some(producer_finished_tx));
            Arc::new(move |descriptor: TurnPreparationJob| {
                let _keep_notifier_alive = &producer_finished;
                preparation_calls.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(ACTIVE_WORK_MS));
                if descriptor.schedule_index == 4 {
                    fifth_active_work_finished_tx.send(()).unwrap();
                }
                Ok(prepared_test_turn(&descriptor))
            }) as TurnPreparer
        };
        let provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let (provider_started_tx, mut provider_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let transcriber = {
            let provider_gate = Arc::clone(&provider_gate);
            let active = Arc::clone(&active);
            Arc::new(move |request: TranscriptionRequest| {
                let provider_gate = Arc::clone(&provider_gate);
                let active = Arc::clone(&active);
                let provider_started_tx = provider_started_tx.clone();
                let operation_id = request.operation_id();
                Box::pin(async move {
                    active.fetch_add(1, Ordering::SeqCst);
                    provider_started_tx.send(()).unwrap();
                    let permit = provider_gate.acquire_owned().await.unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    permit.forget();
                    Ok(test_provider_result(&operation_id))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let done_started = Instant::now();
        let mut pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                descriptors,
                Vec::new(),
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                turn_preparer,
                Arc::new(|plan| Ok(fake_fallback_job(plan))),
                transcriber,
                None,
                None,
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::from_done(done_started),
            )
            .await
        });

        for _ in 0..DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY {
            tokio::time::timeout(Duration::from_secs(2), provider_started_rx.recv())
                .await
                .expect("provider start timed out")
                .expect("provider-start channel closed");
        }
        tokio::time::timeout(Duration::from_secs(2), fifth_active_work_finished_rx.recv())
            .await
            .expect("fifth active preparation did not finish")
            .expect("fifth-active-work channel closed");
        assert_eq!(active.load(Ordering::SeqCst), 2);
        assert_eq!(preparation_calls.load(Ordering::SeqCst), 5);
        let producer_probe = tokio::time::timeout(
            Duration::from_millis(BACKPRESSURE_PROBE_MS),
            producer_finished_rx.recv(),
        )
        .await;
        assert!(
            producer_probe.is_err(),
            "the producer was not blocked on the fifth capacity-two send"
        );

        provider_gate.add_permits(1);
        tokio::time::timeout(Duration::from_secs(2), producer_finished_rx.recv())
            .await
            .expect("producer did not finish after channel capacity opened")
            .expect("producer-finished channel closed");
        let pipeline_probe = tokio::time::timeout(
            Duration::from_millis(POST_PREPARATION_HOLD_MS),
            &mut pipeline,
        )
        .await;
        assert!(
            pipeline_probe.is_err(),
            "pipeline completed when only one provider permit was released"
        );

        provider_gate.add_permits(8);
        let pipeline_result = match pipeline_probe {
            Ok(result) => Ok(result),
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await,
        };
        let final_done_to_ms = elapsed_ms(done_started);
        let result = pipeline_result
            .expect("pipeline timed out")
            .expect("pipeline task panicked")
            .expect("pipeline should succeed");
        let report = result.preparation;

        assert_eq!(report.prepared_count, 5);
        assert_eq!(preparation_calls.load(Ordering::SeqCst), 5);
        assert!(report.error.is_none());
        assert!(
            report.active_preparation_duration_ms >= (ACTIVE_WORK_MS * 5) as i64,
            "active preparation time omitted controlled preparer work: {:?}",
            report
        );
        assert!(
            report.producer_wall_duration_ms
                >= report.active_preparation_duration_ms + BACKPRESSURE_PROBE_MS as i64 - 10,
            "producer wall time did not include channel backpressure: {:?}",
            report
        );
        let done_to_preparation_complete_ms = report
            .done_to_preparation_complete_ms
            .expect("tracked timing should capture producer completion");
        assert!(
            final_done_to_ms
                >= done_to_preparation_complete_ms + POST_PREPARATION_HOLD_MS as i64 - 10,
            "preparation completion was measured at final pipeline return: {:?}",
            report
        );
        assert_eq!(result.outcome.candidates.len(), 5);
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn streaming_scheduler_preserves_logical_spawn_context() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let server = install_fake_correction_engine(Arc::new(echo_correction)).await;
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
            turn_preparation_job(2, "microphone", 2, false),
        ];
        let (release_second_tx, release_second_rx) = std::sync::mpsc::channel();
        let release_second_rx = Arc::new(Mutex::new(release_second_rx));
        let turn_preparer = {
            let release_second_rx = Arc::clone(&release_second_rx);
            Arc::new(move |descriptor: TurnPreparationJob| {
                if descriptor.schedule_index == 1 {
                    release_second_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
                }
                Ok(prepared_test_turn(&descriptor))
            }) as TurnPreparer
        };
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let prompts = Arc::clone(&prompts);
            Arc::new(move |request: TranscriptionRequest| {
                let text = request.audio_path.to_string_lossy().to_string();
                prompts.lock().unwrap().push(request.context);
                Box::pin(async move { Ok(test_provider_result(&text)) }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let sink = {
            let release_second_tx = release_second_tx.clone();
            Arc::new(move |event: CompletedTurnTranscription| {
                let release_second_tx = release_second_tx.clone();
                Box::pin(async move {
                    if completed_turn_index(&event) == 0 {
                        let _ = release_second_tx.send(());
                    }
                    Ok(())
                }) as TurnResultFuture
            }) as TurnResultSink
        };
        let mut pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                descriptors,
                Vec::new(),
                &[],
                crate::providers::PROVIDER_GROQ.to_string(),
                "Meeting".to_string(),
                build_transcription_dictionary(&test_dictionary(&["DIM"])),
                turn_preparer,
                Arc::new(|plan| Ok(fake_fallback_job(plan))),
                transcriber,
                None,
                Some(sink),
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::untracked(),
            )
            .await
        });

        let pipeline_result = tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await;
        if pipeline_result.is_err() {
            let _ = release_second_tx.send(());
            let _ = tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await;
        }
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
        let result = pipeline_result
            .expect("pipeline timed out")
            .expect("pipeline task panicked")
            .expect("pipeline should succeed");
        assert_eq!(result.outcome.candidates.len(), 3);

        assert_eq!(*prompts.lock().unwrap(), vec![Some("DIM".to_string()); 3]);
        let third = correction_request_for(&server, "ordinary-microphone-2.wav");
        assert!(third.contains("- DIM"));
        assert!(third.contains("Microphone: ordinary-microphone-0.wav"));
        assert!(!third.contains("System: ordinary-system-1.wav"));
    }

    #[tokio::test]
    async fn pipelined_results_are_sorted_after_reverse_completion() {
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
        ];
        let blocked_first = Arc::new(tokio::sync::Semaphore::new(0));
        let transcriber = {
            let blocked_first = Arc::clone(&blocked_first);
            Arc::new(move |request: TranscriptionRequest| {
                let blocked_first = Arc::clone(&blocked_first);
                let operation_id = request.operation_id();
                let text = request.audio_path.to_string_lossy().to_string();
                Box::pin(async move {
                    if operation_id.contains("-turn-0-") {
                        let permit = blocked_first.acquire_owned().await.unwrap();
                        drop(permit);
                    }
                    Ok(test_provider_result(&text))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let sink_order = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let sink_order = Arc::clone(&sink_order);
            let blocked_first = Arc::clone(&blocked_first);
            Arc::new(move |event: CompletedTurnTranscription| {
                let sink_order = Arc::clone(&sink_order);
                let blocked_first = Arc::clone(&blocked_first);
                Box::pin(async move {
                    let turn_index = completed_turn_index(&event);
                    sink_order.lock().unwrap().push(turn_index);
                    if turn_index == 1 {
                        blocked_first.add_permits(1);
                    }
                    Ok(())
                }) as TurnResultFuture
            }) as TurnResultSink
        };
        let turn_preparer =
            Arc::new(|descriptor: TurnPreparationJob| Ok(prepared_test_turn(&descriptor)))
                as TurnPreparer;
        let mut pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                descriptors,
                Vec::new(),
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                turn_preparer,
                Arc::new(|plan| Ok(fake_fallback_job(plan))),
                transcriber,
                None,
                Some(sink),
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::untracked(),
            )
            .await
        });

        let pipeline_result = tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await;
        if pipeline_result.is_err() {
            blocked_first.add_permits(1);
            let _ = tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await;
        }
        let result = pipeline_result
            .expect("pipeline timed out")
            .expect("pipeline task panicked")
            .expect("pipeline should succeed");
        assert_eq!(*sink_order.lock().unwrap(), vec![1, 0]);
        assert_eq!(
            result
                .outcome
                .candidates
                .iter()
                .map(|candidate| (
                    candidate.input.source.as_str(),
                    candidate.input.text.as_str(),
                    candidate.input.start_ms,
                    candidate.input.end_ms,
                    candidate.input.turn_index,
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    "microphone",
                    "ordinary-microphone-0.wav",
                    Some(100),
                    Some(600),
                    Some(0),
                ),
                (
                    "system",
                    "ordinary-system-1.wav",
                    Some(1_100),
                    Some(1_600),
                    Some(1),
                ),
            ]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn preparation_error_joins_in_flight_requests_and_skips_fallback() {
        const ACTIVE_WORK_MS: u64 = 15;
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
        ];
        let fallback_plans = build_source_fallback_plans(&descriptors);
        let (provider_began_tx, provider_began_rx) = std::sync::mpsc::channel();
        let provider_began_rx = Arc::new(Mutex::new(provider_began_rx));
        let (provider_observed_tx, mut provider_observed_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let (preparation_error_tx, mut preparation_error_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let planned_error = AppError {
            code: "audio_turn_failed".to_string(),
            message: "planned preparation failure".to_string(),
            details: Some(serde_json::json!({ "stage": "turn-preparation" })),
        };
        let expected_error = planned_error.clone();
        let turn_preparer = {
            let provider_began_rx = Arc::clone(&provider_began_rx);
            Arc::new(move |descriptor: TurnPreparationJob| {
                std::thread::sleep(Duration::from_millis(ACTIVE_WORK_MS));
                if descriptor.schedule_index == 1 {
                    provider_began_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
                    preparation_error_tx.send(()).unwrap();
                    return Err(planned_error.clone());
                }
                Ok(prepared_test_turn(&descriptor))
            }) as TurnPreparer
        };
        let active = Arc::new(AtomicUsize::new(0));
        let provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let transcriber = {
            let active = Arc::clone(&active);
            let provider_gate = Arc::clone(&provider_gate);
            Arc::new(move |request: TranscriptionRequest| {
                let active = Arc::clone(&active);
                let provider_gate = Arc::clone(&provider_gate);
                let provider_began_tx = provider_began_tx.clone();
                let provider_observed_tx = provider_observed_tx.clone();
                let operation_id = request.operation_id();
                Box::pin(async move {
                    active.fetch_add(1, Ordering::SeqCst);
                    provider_began_tx.send(()).unwrap();
                    provider_observed_tx.send(()).unwrap();
                    let permit = provider_gate.acquire_owned().await.unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                    Ok(test_provider_result(&operation_id))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let persisted_turns = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let persisted_turns = Arc::clone(&persisted_turns);
            Arc::new(move |event: CompletedTurnTranscription| {
                let persisted_turns = Arc::clone(&persisted_turns);
                Box::pin(async move {
                    persisted_turns
                        .lock()
                        .unwrap()
                        .push(completed_turn_index(&event));
                    Ok(())
                }) as TurnResultFuture
            }) as TurnResultSink
        };
        let fallback_count = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_count = Arc::clone(&fallback_count);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_count.fetch_add(1, Ordering::SeqCst);
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let done_started = Instant::now();
        let mut pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                descriptors,
                fallback_plans,
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                turn_preparer,
                fallback_preparer,
                transcriber,
                None,
                Some(sink),
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::from_done(done_started),
            )
            .await
        });

        let provider_observed =
            tokio::time::timeout(Duration::from_secs(2), provider_observed_rx.recv()).await;
        let preparation_error_observed =
            tokio::time::timeout(Duration::from_secs(2), preparation_error_rx.recv()).await;
        let pending_probe = tokio::time::timeout(Duration::from_millis(50), &mut pipeline).await;
        let pipeline_was_pending = pending_probe.is_err();
        provider_gate.add_permits(1);
        let pipeline_result = match pending_probe {
            Ok(result) => Ok(result),
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await,
        };
        if pipeline_result.is_err() {
            pipeline.abort();
        }

        assert!(provider_observed.is_ok(), "provider work never began");
        assert!(
            preparation_error_observed.is_ok(),
            "preparation never returned its error"
        );
        assert!(
            pipeline_was_pending,
            "pipeline returned before draining in-flight provider work"
        );
        let failure = pipeline_result
            .expect("pipeline timed out")
            .expect("pipeline task panicked")
            .expect_err("pipeline should return the preparation error");
        assert_eq!(failure.error.code, "audio_turn_failed");
        assert_eq!(failure.error.message, "planned preparation failure");
        assert_eq!(failure.error.details, expected_error.details);
        let preparation = failure
            .preparation
            .expect("producer report should survive preparation failure");
        assert_eq!(preparation.prepared_count, 1);
        assert!(
            preparation.active_preparation_duration_ms >= (ACTIVE_WORK_MS * 2) as i64,
            "report omitted active work from the failed preparation attempt: {:?}",
            preparation
        );
        assert!(
            preparation.producer_wall_duration_ms >= preparation.active_preparation_duration_ms,
            "producer wall time must include all active preparation: {:?}",
            preparation
        );
        assert!(preparation.done_to_preparation_complete_ms.is_some());
        let report_error = preparation.error.expect("report should keep its error");
        assert_eq!(report_error.code, "audio_turn_failed");
        assert_eq!(report_error.message, "planned preparation failure");
        assert_eq!(report_error.details, expected_error.details);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(*persisted_turns.lock().unwrap(), vec![0]);
        assert_eq!(fallback_count.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn turn_wav_temp_dir_is_removed_after_preparation_error() {
        let root = tempfile::tempdir().expect("temp root");
        let turn_wav_dir = root.path().join("session").join("turns");
        std::fs::create_dir_all(&turn_wav_dir).expect("turn WAV temp dir");
        let turn_wav_dir_cleanup = Arc::new(TempDirCleanup(turn_wav_dir.clone()));

        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
        ];
        let fallback_plans = build_source_fallback_plans(&descriptors);
        let (provider_began_tx, provider_began_rx) = std::sync::mpsc::channel();
        let provider_began_rx = Arc::new(Mutex::new(provider_began_rx));
        let (preparation_failed_tx, mut preparation_failed_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let turn_preparer = {
            let cleanup = Arc::clone(&turn_wav_dir_cleanup);
            let provider_began_rx = Arc::clone(&provider_began_rx);
            Arc::new(move |descriptor: TurnPreparationJob| {
                let _cleanup = Arc::clone(&cleanup);
                if descriptor.schedule_index == 1 {
                    provider_began_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
                    preparation_failed_tx.send(()).unwrap();
                    return Err(AppError::new(
                        "audio_turn_failed",
                        "planned preparation failure",
                    ));
                }
                Ok(prepared_test_turn(&descriptor))
            }) as TurnPreparer
        };
        let active = Arc::new(AtomicUsize::new(0));
        let provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let transcriber = {
            let cleanup = Arc::clone(&turn_wav_dir_cleanup);
            let active = Arc::clone(&active);
            let provider_gate = Arc::clone(&provider_gate);
            Arc::new(move |request: TranscriptionRequest| {
                let cleanup = Arc::clone(&cleanup);
                let active = Arc::clone(&active);
                let provider_gate = Arc::clone(&provider_gate);
                let provider_began_tx = provider_began_tx.clone();
                let operation_id = request.operation_id();
                Box::pin(async move {
                    let _cleanup = cleanup;
                    active.fetch_add(1, Ordering::SeqCst);
                    provider_began_tx.send(()).unwrap();
                    let permit = provider_gate.acquire_owned().await.unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                    Ok(test_provider_result(&operation_id))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let fallback_count = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_count = Arc::clone(&fallback_count);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_count.fetch_add(1, Ordering::SeqCst);
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let mut pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                descriptors,
                fallback_plans,
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                turn_preparer,
                fallback_preparer,
                transcriber,
                None,
                None,
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::untracked(),
            )
            .await
        });

        tokio::time::timeout(Duration::from_secs(2), preparation_failed_rx.recv())
            .await
            .expect("preparation failure observation timed out")
            .expect("preparation failure observation channel closed");
        assert_eq!(active.load(Ordering::SeqCst), 1);
        assert!(
            turn_wav_dir.exists(),
            "temp directory was removed while provider work was draining"
        );
        let pending_probe = tokio::time::timeout(Duration::from_millis(50), &mut pipeline).await;
        assert!(
            pending_probe.is_err(),
            "pipeline returned before its in-flight provider was drained"
        );

        provider_gate.add_permits(1);
        let pipeline_result = match pending_probe {
            Ok(result) => Ok(result),
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await,
        };
        let failure = pipeline_result
            .expect("pipeline timed out")
            .expect("pipeline task panicked")
            .expect_err("pipeline should return the preparation error");
        assert_eq!(failure.error.code, "audio_turn_failed");
        assert_eq!(failure.error.message, "planned preparation failure");
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_count.load(Ordering::SeqCst), 0);
        assert!(
            turn_wav_dir.exists(),
            "caller's cleanup guard should retain the directory"
        );

        drop(turn_wav_dir_cleanup);
        assert!(!turn_wav_dir.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn turn_wav_temp_dir_outlives_cancelled_blocking_preparation() {
        let root = tempfile::tempdir().expect("temp root");
        let turn_wav_dir = root.path().join("session").join("turns");
        std::fs::create_dir_all(&turn_wav_dir).expect("turn WAV temp dir");
        let turn_wav_dir_cleanup = Arc::new(TempDirCleanup(turn_wav_dir.clone()));

        let descriptor = turn_preparation_job(0, "microphone", 0, false);
        let (preparation_started_tx, mut preparation_started_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let (release_preparation_tx, release_preparation_rx) = std::sync::mpsc::channel();
        let release_preparation_rx = Arc::new(Mutex::new(release_preparation_rx));
        let (preparation_finished_tx, mut preparation_finished_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let blocking_preparer = {
            let release_preparation_rx = Arc::clone(&release_preparation_rx);
            Arc::new(move |descriptor: TurnPreparationJob| {
                preparation_started_tx.send(()).unwrap();
                release_preparation_rx
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5))
                    .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
                preparation_finished_tx.send(()).unwrap();
                Ok(prepared_test_turn(&descriptor))
            }) as TurnPreparer
        };
        let turn_preparer = retain_cleanup_during_turn_preparation(
            blocking_preparer,
            Arc::clone(&turn_wav_dir_cleanup),
        );
        let transcriber: TurnTranscriber = Arc::new(|_request| {
            Box::pin(async { Err(AppError::new("unexpected", "unexpected provider call")) })
        });
        let pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                vec![descriptor],
                Vec::new(),
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                turn_preparer,
                Arc::new(|plan| Ok(fake_fallback_job(plan))),
                transcriber,
                None,
                None,
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::untracked(),
            )
            .await
        });

        tokio::time::timeout(Duration::from_secs(2), preparation_started_rx.recv())
            .await
            .expect("blocking preparation did not start")
            .expect("preparation-start channel closed");
        pipeline.abort();
        let join_error = pipeline
            .await
            .expect_err("aborted pipeline task should be cancelled");
        assert!(join_error.is_cancelled());
        assert!(turn_wav_dir.exists());

        drop(turn_wav_dir_cleanup);
        assert!(
            turn_wav_dir.exists(),
            "blocking preparation must retain the temp directory after caller cancellation"
        );

        release_preparation_tx
            .send(())
            .expect("release blocking preparation");
        tokio::time::timeout(Duration::from_secs(2), preparation_finished_rx.recv())
            .await
            .expect("blocking preparation did not finish")
            .expect("preparation-finished channel closed");
        tokio::time::timeout(Duration::from_secs(2), async {
            while turn_wav_dir.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("turn WAV temp directory was not eventually removed");
    }

    #[tokio::test]
    async fn note_transcription_future_retains_turn_wav_temp_dir_after_wrapper_drop() {
        let root = tempfile::tempdir().expect("temp root");
        let turn_wav_dir = root.path().join("session").join("turns");
        std::fs::create_dir_all(&turn_wav_dir).expect("turn WAV temp dir");
        let turn_wav_dir_cleanup = Arc::new(TempDirCleanup(turn_wav_dir.clone()));
        let provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let (provider_started_tx, mut provider_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let inner = {
            let provider_gate = Arc::clone(&provider_gate);
            Arc::new(move |_request: TranscriptionRequest| {
                let provider_gate = Arc::clone(&provider_gate);
                let provider_started_tx = provider_started_tx.clone();
                Box::pin(async move {
                    provider_started_tx.send(()).unwrap();
                    let permit = provider_gate.acquire_owned().await.unwrap();
                    drop(permit);
                    Ok(test_provider_result("transcribed"))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let guarded =
            retain_cleanup_during_note_transcription(inner, Arc::clone(&turn_wav_dir_cleanup));
        let future = guarded(TranscriptionRequest {
            provider: TEST_TRANSCRIPTION_PROVIDER.to_string(),
            audio_path: PathBuf::from("prepared.wav"),
            title: "Meeting".to_string(),
            context: None,
            language: None,
            operation_id: Some("operation".to_string()),
            preview: false,
        });

        drop(guarded);
        drop(turn_wav_dir_cleanup);
        assert!(
            turn_wav_dir.exists(),
            "the returned note transcription future must own the cleanup guard"
        );

        let provider = tokio::spawn(future);
        tokio::time::timeout(Duration::from_secs(2), provider_started_rx.recv())
            .await
            .expect("provider future did not start")
            .expect("provider-start channel closed");
        assert!(turn_wav_dir.exists());
        provider_gate.add_permits(1);
        provider
            .await
            .expect("provider task panicked")
            .expect("provider failed");
        assert!(!turn_wav_dir.exists());
    }

    #[tokio::test]
    async fn empty_pipeline_returns_without_polling_disabled_branches() {
        let transcriber_calls = Arc::new(AtomicUsize::new(0));
        let transcriber = {
            let transcriber_calls = Arc::clone(&transcriber_calls);
            Arc::new(move |_request: TranscriptionRequest| {
                transcriber_calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err(AppError::new("unexpected", "unexpected provider call")) })
                    as TranscriptionFuture
            }) as TurnTranscriber
        };
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_calls = Arc::clone(&fallback_calls);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_calls.fetch_add(1, Ordering::SeqCst);
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let sink_calls = Arc::new(AtomicUsize::new(0));
        let sink = {
            let sink_calls = Arc::clone(&sink_calls);
            Arc::new(move |_event: CompletedTurnTranscription| {
                sink_calls.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Ok(()) }) as TurnResultFuture
            }) as TurnResultSink
        };

        let result = tokio::time::timeout(
            Duration::from_secs(2),
            prepare_and_transcribe_turn_jobs_bounded(
                Vec::new(),
                Vec::new(),
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                Arc::new(|descriptor| Ok(prepared_test_turn(&descriptor))),
                fallback_preparer,
                transcriber,
                None,
                Some(sink),
                0,
                ProcessingTiming::untracked(),
            ),
        )
        .await
        .expect("empty pipeline timed out")
        .expect("empty pipeline should succeed");

        assert!(result.outcome.candidates.is_empty());
        assert!(result.outcome.failures.is_empty());
        assert_eq!(result.preparation.prepared_count, 0);
        assert!(result.preparation.error.is_none());
        assert_eq!(transcriber_calls.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        assert_eq!(sink_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn streaming_scheduler_finishes_after_exact_job_count_with_sender_open() {
        let descriptors = [
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
        ];
        let (sender, receiver) = tokio::sync::mpsc::channel(2);
        for descriptor in &descriptors {
            sender
                .send(Ok(prepared_test_turn(descriptor)))
                .await
                .unwrap();
        }
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = successful_test_transcriber(Arc::clone(&operation_ids));

        let outcome = tokio::time::timeout(
            Duration::from_secs(2),
            transcribe_prepared_turn_stream(
                receiver,
                descriptors.len(),
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                &[],
                transcriber,
                None,
                None,
                0,
            ),
        )
        .await
        .expect("scheduler waited for channel closure")
        .expect("scheduler should succeed after the exact job count");

        assert_eq!(outcome.candidates.len(), 2);
        assert!(outcome.failures.is_empty());
        assert_eq!(operation_ids.lock().unwrap().len(), 2);
        drop(sender);
    }

    #[tokio::test]
    async fn premature_preparation_close_drains_started_provider_and_sink() {
        let descriptor = turn_preparation_job(0, "microphone", 0, false);
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        sender
            .send(Ok(prepared_test_turn(&descriptor)))
            .await
            .unwrap();
        let active = Arc::new(AtomicUsize::new(0));
        let provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let (provider_started_tx, mut provider_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let transcriber = {
            let active = Arc::clone(&active);
            let provider_gate = Arc::clone(&provider_gate);
            Arc::new(move |request: TranscriptionRequest| {
                let active = Arc::clone(&active);
                let provider_gate = Arc::clone(&provider_gate);
                let provider_started_tx = provider_started_tx.clone();
                let operation_id = request.operation_id();
                Box::pin(async move {
                    active.fetch_add(1, Ordering::SeqCst);
                    provider_started_tx.send(()).unwrap();
                    let permit = provider_gate.acquire_owned().await.unwrap();
                    active.fetch_sub(1, Ordering::SeqCst);
                    drop(permit);
                    Ok(test_provider_result(&operation_id))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let sink_turns = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let sink_turns = Arc::clone(&sink_turns);
            Arc::new(move |event: CompletedTurnTranscription| {
                let sink_turns = Arc::clone(&sink_turns);
                Box::pin(async move {
                    sink_turns
                        .lock()
                        .unwrap()
                        .push(completed_turn_index(&event));
                    Ok(())
                }) as TurnResultFuture
            }) as TurnResultSink
        };
        let mut scheduler = tokio::spawn(transcribe_prepared_turn_stream(
            receiver,
            2,
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            &[],
            transcriber,
            None,
            Some(sink),
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        ));

        let provider_started =
            tokio::time::timeout(Duration::from_secs(2), provider_started_rx.recv()).await;
        drop(sender);
        let pending_probe = tokio::time::timeout(Duration::from_millis(50), &mut scheduler).await;
        let scheduler_was_pending = pending_probe.is_err();
        provider_gate.add_permits(1);
        let scheduler_result = match pending_probe {
            Ok(result) => Ok(result),
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut scheduler).await,
        };
        if scheduler_result.is_err() {
            scheduler.abort();
        }

        assert!(provider_started.is_ok(), "provider work never began");
        assert!(
            scheduler_was_pending,
            "scheduler returned before draining its started provider"
        );
        let error = scheduler_result
            .expect("scheduler timed out")
            .expect("scheduler task panicked")
            .expect_err("premature channel closure should fail");
        assert_eq!(error.code, "audio_turn_failed");
        assert_eq!(
            error.message,
            "turn preparation ended before every job was received"
        );
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(*sink_turns.lock().unwrap(), vec![0]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn sink_error_drains_started_work_and_joins_blocked_producer() {
        let descriptors = (0..5)
            .map(|index| {
                turn_preparation_job(
                    index,
                    if index % 2 == 0 {
                        "microphone"
                    } else {
                        "system"
                    },
                    index as i64,
                    false,
                )
            })
            .collect::<Vec<_>>();
        let fallback_plans = build_source_fallback_plans(&descriptors);
        let (fifth_preparation_started_tx, mut fifth_preparation_started_rx) =
            tokio::sync::mpsc::unbounded_channel();
        let (release_fifth_preparation_tx, release_fifth_preparation_rx) =
            std::sync::mpsc::channel();
        let release_fifth_preparation_rx = Arc::new(Mutex::new(release_fifth_preparation_rx));
        let turn_preparer = {
            let release_fifth_preparation_rx = Arc::clone(&release_fifth_preparation_rx);
            Arc::new(move |descriptor: TurnPreparationJob| {
                if descriptor.schedule_index == 4 {
                    fifth_preparation_started_tx.send(()).unwrap();
                    release_fifth_preparation_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .map_err(|error| AppError::new("audio_turn_failed", error.to_string()))?;
                }
                Ok(prepared_test_turn(&descriptor))
            }) as TurnPreparer
        };
        let first_provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let second_provider_gate = Arc::new(tokio::sync::Semaphore::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let (provider_started_tx, mut provider_started_rx) = tokio::sync::mpsc::unbounded_channel();
        let transcriber = {
            let first_provider_gate = Arc::clone(&first_provider_gate);
            let second_provider_gate = Arc::clone(&second_provider_gate);
            let active = Arc::clone(&active);
            Arc::new(move |request: TranscriptionRequest| {
                let first_provider_gate = Arc::clone(&first_provider_gate);
                let second_provider_gate = Arc::clone(&second_provider_gate);
                let active = Arc::clone(&active);
                let provider_started_tx = provider_started_tx.clone();
                let operation_id = request.operation_id();
                let turn_index = if operation_id.contains("-turn-0-") {
                    0
                } else if operation_id.contains("-turn-1-") {
                    1
                } else if operation_id.contains("-turn-2-") {
                    2
                } else {
                    3
                };
                Box::pin(async move {
                    active.fetch_add(1, Ordering::SeqCst);
                    provider_started_tx.send(operation_id.clone()).unwrap();
                    match turn_index {
                        0 => {
                            let permit = first_provider_gate.acquire_owned().await.unwrap();
                            drop(permit);
                        }
                        1 => {
                            let permit = second_provider_gate.acquire_owned().await.unwrap();
                            drop(permit);
                        }
                        _ => {}
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(test_provider_result(&operation_id))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        let sink_order = Arc::new(Mutex::new(Vec::new()));
        let (sink_invoked_tx, mut sink_invoked_rx) = tokio::sync::mpsc::unbounded_channel();
        let sink = {
            let sink_order = Arc::clone(&sink_order);
            Arc::new(move |event: CompletedTurnTranscription| {
                let sink_order = Arc::clone(&sink_order);
                let sink_invoked_tx = sink_invoked_tx.clone();
                Box::pin(async move {
                    let turn_index = completed_turn_index(&event);
                    sink_order.lock().unwrap().push(turn_index);
                    sink_invoked_tx.send(turn_index).unwrap();
                    match turn_index {
                        0 => Err(AppError::new("sink_failed", "first sink failure")),
                        1 => Err(AppError::new("sink_failed", "second sink failure")),
                        _ => Ok(()),
                    }
                }) as TurnResultFuture
            }) as TurnResultSink
        };
        let fallback_count = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_count = Arc::clone(&fallback_count);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_count.fetch_add(1, Ordering::SeqCst);
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let mut pipeline = tokio::spawn(async move {
            prepare_and_transcribe_turn_jobs_bounded(
                descriptors,
                fallback_plans,
                &[],
                TEST_TRANSCRIPTION_PROVIDER.to_string(),
                "Meeting".to_string(),
                None,
                turn_preparer,
                fallback_preparer,
                transcriber,
                None,
                Some(sink),
                DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
                ProcessingTiming::untracked(),
            )
            .await
        });

        let fifth_preparation_started =
            tokio::time::timeout(Duration::from_secs(2), fifth_preparation_started_rx.recv()).await;
        let first_started =
            tokio::time::timeout(Duration::from_secs(2), provider_started_rx.recv()).await;
        let second_started =
            tokio::time::timeout(Duration::from_secs(2), provider_started_rx.recv()).await;
        first_provider_gate.add_permits(1);
        let first_sink = tokio::time::timeout(Duration::from_secs(2), sink_invoked_rx.recv()).await;
        let pending_after_first_sink_error =
            tokio::time::timeout(Duration::from_millis(50), async {
                while !pipeline.is_finished() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .is_err();
        second_provider_gate.add_permits(1);
        let second_sink =
            tokio::time::timeout(Duration::from_secs(2), sink_invoked_rx.recv()).await;
        let unexpected_provider_start = provider_started_rx.try_recv().ok();
        let pending_probe = tokio::time::timeout(Duration::from_millis(50), &mut pipeline).await;
        let pending_until_producer_joined = pending_probe.is_err();
        let _ = release_fifth_preparation_tx.send(());
        let pipeline_result = match pending_probe {
            Ok(result) => Ok(result),
            Err(_) => tokio::time::timeout(Duration::from_secs(2), &mut pipeline).await,
        };
        if pipeline_result.is_err() {
            pipeline.abort();
        }
        let late_provider_start = provider_started_rx.try_recv().ok();

        assert!(
            fifth_preparation_started.is_ok(),
            "the producer never reached the backpressured fifth descriptor"
        );
        let mut started = vec![
            first_started
                .expect("the first provider observation timed out")
                .expect("the first provider channel closed"),
            second_started
                .expect("the second provider observation timed out")
                .expect("the second provider channel closed"),
        ];
        started.sort_unstable();
        assert_eq!(
            started,
            vec![
                "artifact-microphone-100-600-turn-0-fallback-false-saved-audio-v3".to_string(),
                "artifact-system-1100-1600-turn-1-fallback-false-saved-audio-v3".to_string(),
            ]
        );
        assert!(
            unexpected_provider_start.is_none(),
            "provider work launched after the first sink error"
        );
        assert!(
            late_provider_start.is_none(),
            "provider work launched after the pipeline began draining"
        );
        assert_eq!(first_sink.expect("the first sink was not invoked"), Some(0));
        assert!(
            pending_after_first_sink_error,
            "pipeline returned before draining the second provider"
        );
        assert_eq!(
            second_sink.expect("the second sink was not invoked"),
            Some(1)
        );
        assert!(
            pending_until_producer_joined,
            "pipeline returned before its blocked producer was released and joined"
        );
        let failure = pipeline_result
            .expect("pipeline timed out")
            .expect("pipeline task panicked")
            .expect_err("the first sink error should fail the pipeline");
        assert_eq!(failure.error.code, "sink_failed");
        assert_eq!(failure.error.message, "first sink failure");
        let preparation = failure
            .preparation
            .expect("producer report should survive sink failure");
        assert_eq!(preparation.prepared_count, 5);
        assert!(preparation.error.is_none());
        assert_eq!(*sink_order.lock().unwrap(), vec![0, 1]);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(fallback_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn prepared_turn_matches_existing_audio_and_metadata() {
        let dir = std::env::temp_dir().join(format!(
            "clovy-prepared-turn-golden-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let source_path = dir.join("source.wav");
        let source_spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&source_path, source_spec).unwrap();
        for frame in 0..(48_000 * 2) {
            let left = if frame % 4 < 2 {
                12_000_i16
            } else {
                -9_000_i16
            };
            let right = if frame % 6 < 3 { 4_000_i16 } else { -7_000_i16 };
            writer.write_sample(left).unwrap();
            writer.write_sample(right).unwrap();
        }
        writer.finalize().unwrap();
        let source_bytes = std::fs::read(&source_path).unwrap();

        let turn = AudioTurn {
            artifact_id: "artifact".to_string(),
            source: "microphone".to_string(),
            source_path: source_path.clone(),
            extraction_start_ms: 250,
            start_ms: 400,
            end_ms: 1_250,
            turn_index: 7,
        };
        let reference_turn_wav_path = dir.join("reference-turn.wav");
        let reference_normalized_path = dir.join("reference-normalized.wav");
        write_turn_wav(&turn, &reference_turn_wav_path).unwrap();
        let reference_audio_path =
            normalize_wav_for_transcription(&reference_turn_wav_path, &reference_normalized_path)
                .unwrap();
        let reference_job = TurnTranscriptionJob {
            artifact_id: turn.artifact_id.clone(),
            source: turn.source.clone(),
            audio_path: reference_audio_path.clone(),
            temp_dir: dir.clone(),
            recorded_silence: true,
            covers_full_source: false,
            source_fallback: false,
            start_ms: turn.start_ms,
            end_ms: turn.end_ms,
            turn_index: turn.turn_index,
            durable_job: None,
        };

        let prepared = prepare_turn_job(TurnPreparationJob {
            schedule_index: 3,
            turn,
            temp_dir: dir.clone(),
            turn_wav_path: dir.join("actual-turn.wav"),
            normalized_path: dir.join("actual-normalized.wav"),
            recorded_silence: true,
            echo_trimmed: false,
            durable_job: None,
        })
        .unwrap();

        let mut reference_reader = hound::WavReader::open(&reference_audio_path).unwrap();
        let reference_spec = reference_reader.spec();
        let reference_samples = reference_reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let mut actual_reader = hound::WavReader::open(&prepared.job.audio_path).unwrap();
        let actual_spec = actual_reader.spec();
        let actual_samples = actual_reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(actual_spec, reference_spec);
        assert_eq!(actual_samples, reference_samples);
        assert_eq!(
            actual_spec,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            }
        );
        assert_eq!(std::fs::read(&source_path).unwrap(), source_bytes);
        assert_eq!(prepared.schedule_index, 3);
        assert_eq!(prepared.job.artifact_id, reference_job.artifact_id);
        assert_eq!(prepared.job.source, reference_job.source);
        assert_eq!(prepared.job.start_ms, reference_job.start_ms);
        assert_eq!(prepared.job.end_ms, reference_job.end_ms);
        assert_eq!(prepared.job.turn_index, reference_job.turn_index);
        assert_eq!(
            prepared.job.recorded_silence,
            reference_job.recorded_silence
        );
        assert_eq!(prepared.job.temp_dir, reference_job.temp_dir);
        assert!(!prepared.job.source_fallback);
        assert_eq!(
            turn_operation_id(&prepared.job),
            turn_operation_id(&reference_job)
        );
        assert_eq!(
            turn_operation_id(&prepared.job),
            "artifact-microphone-400-1250-turn-7-fallback-false-saved-audio-v3"
        );

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn fallback_plans_use_canonical_source_order_and_stable_unknown_order() {
        let descriptors = vec![
            turn_preparation_job(0, "system", 0, false),
            turn_preparation_job(1, "unknown-first", 1, false),
            turn_preparation_job(2, "microphone", 2, false),
            turn_preparation_job(3, "unknown-second", 3, false),
            turn_preparation_job(4, "unknown-first", 4, false),
        ];

        let fallback_plans = build_source_fallback_plans(&descriptors);

        assert_eq!(
            fallback_plans
                .iter()
                .map(|plan| plan.source.as_str())
                .collect::<Vec<_>>(),
            vec!["microphone", "system", "unknown-first", "unknown-second"]
        );
    }

    #[test]
    fn interleaves_preparation_sources_before_repeating_a_lane() {
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(2, "microphone", 2, false),
            turn_preparation_job(4, "microphone", 4, false),
            turn_preparation_job(1, "system", 1, false),
            turn_preparation_job(3, "system", 3, false),
        ];

        let interleaved = interleave_turn_preparation_jobs_by_source(descriptors);

        assert_eq!(
            interleaved
                .iter()
                .map(|descriptor| (descriptor.turn.source.as_str(), descriptor.turn.turn_index))
                .collect::<Vec<_>>(),
            vec![
                ("microphone", 0),
                ("system", 1),
                ("microphone", 2),
                ("system", 3),
                ("microphone", 4),
            ]
        );
    }

    #[tokio::test]
    async fn successful_jobs_skip_complete_source_preparation() {
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
        ];
        let fallback_plans = build_source_fallback_plans(&descriptors);
        assert_eq!(
            fallback_plans
                .iter()
                .map(|plan| plan.source.as_str())
                .collect::<Vec<_>>(),
            vec!["microphone", "system"]
        );
        let ordinary_jobs = descriptors
            .iter()
            .map(fake_ordinary_job)
            .collect::<Vec<_>>();
        let fallback_preparations = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_preparations = Arc::clone(&fallback_preparations);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_preparations.fetch_add(1, Ordering::SeqCst);
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = successful_test_transcriber(Arc::clone(&operation_ids));

        let outcome = transcribe_turn_jobs_bounded(
            ordinary_jobs,
            fallback_plans,
            &[],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            fallback_preparer,
            transcriber,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        )
        .await
        .unwrap();

        assert_eq!(fallback_preparations.load(Ordering::SeqCst), 0);
        assert_eq!(outcome.candidates.len(), 2);
        assert!(outcome.failures.is_empty());
        let mut candidate_sources = outcome
            .candidates
            .iter()
            .map(|candidate| candidate.input.source.as_str())
            .collect::<Vec<_>>();
        candidate_sources.sort_unstable();
        assert_eq!(candidate_sources, vec!["microphone", "system"]);
        let mut operation_ids = operation_ids.lock().unwrap().clone();
        operation_ids.sort();
        assert_eq!(
            operation_ids,
            vec![
                "artifact-microphone-100-600-turn-0-fallback-false-saved-audio-v3",
                "artifact-system-1100-1600-turn-1-fallback-false-saved-audio-v3",
            ]
        );
    }

    #[tokio::test]
    async fn failed_source_prepares_one_lazy_fallback_with_source_operation_id() {
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
            turn_preparation_job(2, "microphone", 2, false),
        ];
        let fallback_plans = build_source_fallback_plans(&descriptors);
        let expected_microphone_end_ms = descriptors[2].turn.end_ms;
        let raw_microphone_path = descriptors[0].turn.source_path.clone();
        let ordinary_jobs = descriptors
            .iter()
            .map(fake_ordinary_job)
            .collect::<Vec<_>>();
        let fallback_calls = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
        let fallback_preparer = {
            let fallback_calls = Arc::clone(&fallback_calls);
            Arc::new(move |plan: SourceFallbackPlan| {
                *fallback_calls
                    .lock()
                    .unwrap()
                    .entry(plan.source.clone())
                    .or_default() += 1;
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let requests = Arc::new(Mutex::new(Vec::<(String, PathBuf)>::new()));
        let transcriber = {
            let requests = Arc::clone(&requests);
            Arc::new(move |request: TranscriptionRequest| {
                let requests = Arc::clone(&requests);
                Box::pin(async move {
                    let operation_id = request.operation_id();
                    requests
                        .lock()
                        .unwrap()
                        .push((operation_id.clone(), request.audio_path.clone()));
                    if request.audio_path == std::path::Path::new("ordinary-system-1.wav")
                        || operation_id.contains("-fallback-true-")
                    {
                        Ok(test_provider_result(&operation_id))
                    } else {
                        Err(AppError::new("no_speech", "no_speech"))
                    }
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let outcome = transcribe_turn_jobs_bounded(
            ordinary_jobs,
            fallback_plans,
            &[],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            fallback_preparer,
            transcriber,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        )
        .await
        .unwrap();

        assert_eq!(
            *fallback_calls.lock().unwrap(),
            HashMap::from([("microphone".to_string(), 1)])
        );
        assert!(outcome.failures.is_empty());
        assert_eq!(outcome.candidates.len(), 2);
        let microphone = outcome
            .candidates
            .iter()
            .find(|candidate| candidate.input.source == "microphone")
            .unwrap();
        assert_eq!(microphone.input.start_ms, Some(0));
        assert_eq!(microphone.input.end_ms, Some(expected_microphone_end_ms));
        assert_eq!(microphone.input.turn_index, Some(0));

        let requests = requests.lock().unwrap();
        let mut operation_ids = requests
            .iter()
            .map(|(operation_id, _)| operation_id.clone())
            .collect::<Vec<_>>();
        operation_ids.sort();
        assert_eq!(
            operation_ids,
            vec![
                "artifact-microphone-0-2600-turn-0-fallback-true-saved-audio-v3",
                "artifact-microphone-100-600-turn-0-fallback-false-saved-audio-v3",
                "artifact-microphone-2100-2600-turn-2-fallback-false-saved-audio-v3",
                "artifact-system-1100-1600-turn-1-fallback-false-saved-audio-v3",
            ]
        );
        let fallback_path = requests
            .iter()
            .find(|(operation_id, _)| operation_id.contains("-fallback-true-"))
            .map(|(_, path)| path)
            .unwrap();
        assert_eq!(fallback_path, &PathBuf::from("prepared-microphone.wav"));
        assert_ne!(fallback_path, &raw_microphone_path);
        assert!(requests
            .iter()
            .all(|(_, audio_path)| audio_path != &raw_microphone_path));
    }

    #[tokio::test]
    async fn failed_sources_prepare_one_fallback_each() {
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "system", 1, false),
        ];
        let fallback_plans = build_source_fallback_plans(&descriptors);
        let ordinary_jobs = descriptors
            .iter()
            .map(fake_ordinary_job)
            .collect::<Vec<_>>();
        let fallback_calls = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
        let fallback_preparer = {
            let fallback_calls = Arc::clone(&fallback_calls);
            Arc::new(move |plan: SourceFallbackPlan| {
                *fallback_calls
                    .lock()
                    .unwrap()
                    .entry(plan.source.clone())
                    .or_default() += 1;
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let operation_ids = Arc::clone(&operation_ids);
            Arc::new(move |request: TranscriptionRequest| {
                let operation_ids = Arc::clone(&operation_ids);
                Box::pin(async move {
                    let operation_id = request.operation_id();
                    operation_ids.lock().unwrap().push(operation_id.clone());
                    if operation_id.contains("-fallback-true-") {
                        Ok(test_provider_result(&operation_id))
                    } else {
                        Err(AppError::new("no_speech", "no_speech"))
                    }
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let outcome = transcribe_turn_jobs_bounded(
            ordinary_jobs,
            fallback_plans,
            &[],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            fallback_preparer,
            transcriber,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        )
        .await
        .unwrap();

        assert_eq!(
            *fallback_calls.lock().unwrap(),
            HashMap::from([("microphone".to_string(), 1), ("system".to_string(), 1),])
        );
        assert!(outcome.failures.is_empty());
        assert_eq!(outcome.candidates.len(), 2);
        let operation_ids = operation_ids.lock().unwrap();
        let mut sorted_operation_ids = operation_ids.clone();
        sorted_operation_ids.sort();
        assert_eq!(
            sorted_operation_ids,
            vec![
                "artifact-microphone-0-600-turn-0-fallback-true-saved-audio-v3",
                "artifact-microphone-100-600-turn-0-fallback-false-saved-audio-v3",
                "artifact-system-0-1600-turn-1-fallback-true-saved-audio-v3",
                "artifact-system-1100-1600-turn-1-fallback-false-saved-audio-v3",
            ]
        );
        assert_eq!(
            operation_ids
                .iter()
                .filter(|operation_id| operation_id.contains("-fallback-true-"))
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "artifact-microphone-0-600-turn-0-fallback-true-saved-audio-v3",
                "artifact-system-0-1600-turn-1-fallback-true-saved-audio-v3",
            ]
        );
    }

    #[tokio::test]
    async fn severely_incomplete_lane_fallback_replaces_partial_in_memory_results() {
        let mut descriptors = vec![
            turn_preparation_job(0, "system", 0, false),
            turn_preparation_job(1, "system", 1, false),
            turn_preparation_job(2, "system", 2, false),
        ];
        for (descriptor, (start_ms, end_ms)) in
            descriptors
                .iter_mut()
                .zip([(0, 10_000), (10_000, 100_000), (100_000, 190_000)])
        {
            descriptor.turn.extraction_start_ms = start_ms;
            descriptor.turn.start_ms = start_ms;
            descriptor.turn.end_ms = end_ms;
        }
        let fallback_plans = build_source_fallback_plans(&descriptors);
        assert_eq!(fallback_plans[0].detected_ms, 190_000);
        let ordinary_jobs = descriptors
            .iter()
            .map(fake_ordinary_job)
            .collect::<Vec<_>>();
        let transcriber = Arc::new(move |request: TranscriptionRequest| {
            Box::pin(async move {
                match request.audio_path.to_string_lossy().as_ref() {
                    "ordinary-system-0.wav" => Ok(test_provider_result("partial turn")),
                    "prepared-system.wav" => Ok(test_provider_result("complete fallback")),
                    _ => Err(AppError::new("no_speech", "no_speech")),
                }
            }) as TranscriptionFuture
        }) as TurnTranscriber;

        let outcome = transcribe_turn_jobs_bounded(
            ordinary_jobs,
            fallback_plans,
            &[],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            Arc::new(|plan| Ok(fake_fallback_job(plan))),
            transcriber,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        )
        .await
        .expect("materially incomplete lane should recover");

        assert!(outcome.failures.is_empty());
        assert_eq!(outcome.candidates.len(), 1);
        assert_eq!(outcome.candidates[0].input.text, "complete fallback");
        assert_eq!(outcome.candidates[0].input.start_ms, Some(0));
        assert_eq!(outcome.candidates[0].input.end_ms, Some(190_000));
        assert!(outcome.replaced_sources.contains("system"));
    }

    #[tokio::test]
    async fn echo_trimmed_source_never_prepares_or_transcribes_fallback() {
        let descriptors = vec![
            turn_preparation_job(0, "microphone", 0, false),
            turn_preparation_job(1, "microphone", 1, true),
        ];
        let fallback_plans = build_source_fallback_plans(&descriptors);
        assert_eq!(fallback_plans.len(), 1);
        assert!(fallback_plans[0].echo_trimmed);
        assert!(!fallback_plans[0].eligible());
        let ordinary_jobs = descriptors
            .iter()
            .map(fake_ordinary_job)
            .collect::<Vec<_>>();
        let fallback_preparations = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_preparations = Arc::clone(&fallback_preparations);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_preparations.fetch_add(1, Ordering::SeqCst);
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let operation_ids = Arc::clone(&operation_ids);
            Arc::new(move |request: TranscriptionRequest| {
                let operation_ids = Arc::clone(&operation_ids);
                Box::pin(async move {
                    operation_ids.lock().unwrap().push(request.operation_id());
                    Err(AppError::new("no_speech", "no_speech"))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let outcome = transcribe_turn_jobs_bounded(
            ordinary_jobs,
            fallback_plans,
            &[],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            fallback_preparer,
            transcriber,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        )
        .await
        .unwrap();

        assert_eq!(fallback_preparations.load(Ordering::SeqCst), 0);
        assert!(outcome.candidates.is_empty());
        assert_eq!(outcome.failures.len(), 2);
        let mut operation_ids = operation_ids.lock().unwrap().clone();
        operation_ids.sort();
        assert_eq!(
            operation_ids,
            vec![
                "artifact-microphone-100-600-turn-0-fallback-false-saved-audio-v3",
                "artifact-microphone-1100-1600-turn-1-fallback-false-saved-audio-v3",
            ]
        );
        assert!(operation_ids
            .iter()
            .all(|operation_id| !operation_id.contains("-fallback-true-")));
    }

    #[tokio::test]
    async fn valid_cached_turn_suppresses_fallback_after_fresh_failures() {
        let descriptors = vec![turn_preparation_job(0, "microphone", 1, false)];
        let fallback_plans = build_source_fallback_plans(&descriptors);
        assert!(fallback_plans[0].eligible());
        let ordinary_jobs = descriptors
            .iter()
            .map(fake_ordinary_job)
            .collect::<Vec<_>>();
        let cached_candidates = vec![TranscriptCandidate {
            artifact_id: "artifact".to_string(),
            language: None,
            input: SourceTranscriptInput {
                source: "microphone".to_string(),
                text: "cached turn".to_string(),
                valid: true,
                warning: None,
                recorded_silence: false,
                start_ms: Some(100),
                end_ms: Some(600),
                turn_index: Some(0),
            },
        }];
        let fallback_preparations = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_preparations = Arc::clone(&fallback_preparations);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_preparations.fetch_add(1, Ordering::SeqCst);
                Ok(fake_fallback_job(plan))
            }) as SourceFallbackPreparer
        };
        let operation_ids = Arc::new(Mutex::new(Vec::new()));
        let transcriber = {
            let operation_ids = Arc::clone(&operation_ids);
            Arc::new(move |request: TranscriptionRequest| {
                let operation_ids = Arc::clone(&operation_ids);
                Box::pin(async move {
                    operation_ids.lock().unwrap().push(request.operation_id());
                    Err(AppError::new("no_speech", "no_speech"))
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };

        let outcome = transcribe_turn_jobs_bounded(
            ordinary_jobs,
            fallback_plans,
            &cached_candidates,
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            fallback_preparer,
            transcriber,
            None,
            DEFAULT_TURN_TRANSCRIPTION_CONCURRENCY,
        )
        .await
        .unwrap();

        assert_eq!(fallback_preparations.load(Ordering::SeqCst), 0);
        assert!(outcome.candidates.is_empty());
        assert_eq!(outcome.failures.len(), 1);
        assert_eq!(outcome.failures[0].input.source, "microphone");
        assert_eq!(
            operation_ids.lock().unwrap().as_slice(),
            ["artifact-microphone-1100-1600-turn-1-fallback-false-saved-audio-v3"]
        );
    }

    fn turn_preparation_job(
        schedule_index: usize,
        source: &str,
        turn_index: i64,
        echo_trimmed: bool,
    ) -> TurnPreparationJob {
        let start_ms = turn_index * 1_000 + 100;
        TurnPreparationJob {
            schedule_index,
            turn: AudioTurn {
                artifact_id: "artifact".to_string(),
                source: source.to_string(),
                source_path: PathBuf::from(format!("raw-{source}.wav")),
                extraction_start_ms: start_ms,
                start_ms,
                end_ms: start_ms + 500,
                turn_index,
            },
            temp_dir: PathBuf::from("turn-temp"),
            turn_wav_path: PathBuf::from(format!("turn-{source}-{turn_index}.wav")),
            normalized_path: PathBuf::from(format!("ordinary-{source}-{turn_index}.wav")),
            recorded_silence: false,
            echo_trimmed,
            durable_job: None,
        }
    }

    fn fake_ordinary_job(descriptor: &TurnPreparationJob) -> TurnTranscriptionJob {
        TurnTranscriptionJob {
            artifact_id: descriptor.turn.artifact_id.clone(),
            source: descriptor.turn.source.clone(),
            audio_path: descriptor.normalized_path.clone(),
            temp_dir: descriptor.temp_dir.clone(),
            recorded_silence: descriptor.recorded_silence,
            covers_full_source: descriptor.covers_full_source(),
            source_fallback: false,
            start_ms: descriptor.turn.start_ms,
            end_ms: descriptor.turn.end_ms,
            turn_index: descriptor.turn.turn_index,
            durable_job: None,
        }
    }

    fn prepared_test_turn(descriptor: &TurnPreparationJob) -> PreparedTurn {
        PreparedTurn {
            schedule_index: descriptor.schedule_index,
            job: fake_ordinary_job(descriptor),
        }
    }

    fn completed_turn_index(event: &CompletedTurnTranscription) -> i64 {
        match &event.result {
            TurnTranscriptionResult::Candidate(candidate) => {
                candidate.input.turn_index.unwrap_or_default()
            }
            TurnTranscriptionResult::Failure(failure) => {
                failure.input.turn_index.unwrap_or_default()
            }
        }
    }

    fn fake_fallback_job(plan: SourceFallbackPlan) -> TurnTranscriptionJob {
        let audio_path = PathBuf::from(format!("prepared-{}.wav", plan.source));
        TurnTranscriptionJob {
            artifact_id: plan.artifact_id,
            source: plan.source,
            audio_path,
            temp_dir: plan.temp_dir,
            recorded_silence: plan.recorded_silence,
            covers_full_source: true,
            source_fallback: true,
            start_ms: 0,
            end_ms: plan.end_ms,
            turn_index: plan.turn_index,
            durable_job: None,
        }
    }

    fn test_provider_result(text: &str) -> TranscriptionProviderResult {
        TranscriptionProviderResult {
            text: text.to_string(),
            language: None,
            provider: "test".to_string(),
        }
    }

    fn test_durable_job(id: &str, operation_id: &str) -> NoteTranscriptionJobRecord {
        NoteTranscriptionJobRecord {
            id: id.to_string(),
            note_id: "note".to_string(),
            recording_session_id: "session".to_string(),
            audio_artifact_id: "artifact".to_string(),
            source: "microphone".to_string(),
            source_mode: RecordingSourceMode::MicrophoneOnly,
            job_kind: NoteTranscriptionJobKind::Turn,
            start_ms: 0,
            end_ms: 1_000,
            turn_index: 0,
            input_fingerprint: "fingerprint".to_string(),
            configuration_fingerprint: "configuration".to_string(),
            operation_id: operation_id.to_string(),
            provider: "test".to_string(),
            max_chunk_ms: None,
            pipeline_version: NOTE_TRANSCRIPTION_PIPELINE_VERSION.to_string(),
            status: NoteTranscriptionJobStatus::Pending,
            attempt_count: 0,
            transcript_id: None,
            last_error: None,
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
            completed_at: None,
        }
    }

    fn successful_test_transcriber(operation_ids: Arc<Mutex<Vec<String>>>) -> TurnTranscriber {
        Arc::new(move |request: TranscriptionRequest| {
            let operation_ids = Arc::clone(&operation_ids);
            Box::pin(async move {
                let operation_id = request.operation_id();
                operation_ids.lock().unwrap().push(operation_id.clone());
                Ok(test_provider_result(&operation_id))
            }) as TranscriptionFuture
        })
    }

    fn test_job(path: &str, source: &str, turn_index: i64) -> TurnTranscriptionJob {
        TurnTranscriptionJob {
            artifact_id: format!("artifact-{path}"),
            source: source.to_string(),
            audio_path: PathBuf::from(path),
            temp_dir: std::env::temp_dir(),
            recorded_silence: false,
            covers_full_source: false,
            source_fallback: false,
            start_ms: turn_index * 1_000,
            end_ms: turn_index * 1_000 + 500,
            turn_index,
            durable_job: None,
        }
    }

    fn test_audio_turn(
        artifact_id: &str,
        source: &str,
        source_path: PathBuf,
        turn_index: i64,
        start_ms: i64,
        end_ms: i64,
    ) -> AudioTurn {
        AudioTurn {
            artifact_id: artifact_id.to_string(),
            source: source.to_string(),
            source_path,
            extraction_start_ms: start_ms,
            start_ms,
            end_ms,
            turn_index,
        }
    }

    fn test_transcript(source: &str, turn_index: i64, start_ms: i64, end_ms: i64) -> TranscriptDto {
        TranscriptDto {
            id: format!("{source}-{turn_index}"),
            recording_session_id: None,
            span_id: None,
            text: "transcript".to_string(),
            source_mode: Some(RecordingSourceMode::MicrophonePlusSystem),
            source: Some(source.to_string()),
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
            turn_index: Some(turn_index),
            language: None,
            status: "succeeded".to_string(),
            last_error: None,
            recorded_silence: false,
        }
    }

    fn failed_candidate(source: &str, warning: &str, turn_index: i64) -> FailedTranscriptCandidate {
        FailedTranscriptCandidate {
            artifact_id: format!("{source}-artifact"),
            input: SourceTranscriptInput {
                source: source.to_string(),
                text: String::new(),
                valid: false,
                warning: Some(warning.to_string()),
                recorded_silence: false,
                start_ms: Some(turn_index * 1_000),
                end_ms: Some(turn_index * 1_000 + 500),
                turn_index: Some(turn_index),
            },
        }
    }

    fn test_dictionary(phrases: &[&str]) -> Vec<DictionaryEntryDto> {
        phrases
            .iter()
            .enumerate()
            .map(|(index, phrase)| DictionaryEntryDto {
                id: format!("entry-{index}"),
                phrase: phrase.to_string(),
                created_at: "2026-05-26T00:00:00Z".to_string(),
                updated_at: "2026-05-26T00:00:00Z".to_string(),
            })
            .collect()
    }

    /// Routes the correction engine (the notes usage) to an isolated fake
    /// endpoint. Callers hold `GLOBAL_SETTINGS_TEST_LOCK` and
    /// restore the default settings when done.
    async fn install_fake_correction_engine(
        handler: crate::llm::endpoint::test_server::Handler,
    ) -> crate::llm::endpoint::test_server::FakeServer {
        use crate::llm::registry::{LlmEndpointRecord, ProviderRef};
        let server = crate::llm::endpoint::test_server::start(handler).await;
        let mut settings = crate::providers::default_settings_for_tests();
        settings.llm_endpoints.push(LlmEndpointRecord {
            id: "cleanup".to_string(),
            name: "Cleanup".to_string(),
            base_url: server.base_url.clone(),
            model_id: "local-model".to_string(),
            has_api_key: false,
            structured_output: None,
            rpm_limit: None,
        });
        // The correction runs on the notes engine (ditado-rapido R2).
        settings.llm_usage.notes = ProviderRef::Endpoint {
            id: "cleanup".to_string(),
        };
        crate::providers::replace_current_settings_for_tests(settings);
        server
    }

    /// A correction engine that returns each transcript unchanged, so a turn's
    /// text stays recognizable in later turns' correction context.
    fn echo_correction(request: &serde_json::Value) -> (u16, serde_json::Value) {
        let prompt = request["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default();
        let transcript = prompt
            .split_once("<asr_transcript>\n")
            .and_then(|(_, rest)| rest.split_once("\n</asr_transcript>"))
            .map_or("", |(transcript, _)| transcript);
        (
            200,
            crate::llm::endpoint::test_server::chat_reply(transcript),
        )
    }

    /// The serialized correction request that carried `transcript`.
    fn correction_request_for(
        server: &crate::llm::endpoint::test_server::FakeServer,
        transcript: &str,
    ) -> String {
        let marker = format!("<asr_transcript>\\n{transcript}\\n</asr_transcript>");
        server
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|request| request.to_string())
            .find(|request| request.contains(&marker))
            .unwrap_or_else(|| panic!("no correction request carried {transcript}"))
    }

    #[tokio::test]
    async fn groq_note_transcripts_get_the_full_context_correction_and_keep_the_original_on_failure(
    ) {
        use crate::llm::endpoint::test_server;
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let replies = Arc::new(Mutex::new(vec![
            (
                200,
                test_server::chat_reply("Ship it with Clovy and Jane Doe."),
            ),
            (200, test_server::chat_reply("   ")),
            (500, serde_json::json!({ "error": { "message": "down" } })),
        ]));
        let server = install_fake_correction_engine({
            let replies = Arc::clone(&replies);
            Arc::new(move |_: &serde_json::Value| replies.lock().unwrap().remove(0))
        })
        .await;
        let groq = |text: &str| TranscriptionProviderResult {
            text: text.to_string(),
            language: Some("en".to_string()),
            provider: crate::providers::PROVIDER_GROQ.to_string(),
        };
        // A full dictionary, far beyond what a 224-token Whisper hint can carry.
        let terms = (0..80)
            .map(|index| format!("- Term{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let context = format!("Custom dictionary terms:\n{terms}");

        let corrected = maybe_post_process_note_transcript(
            crate::providers::PROVIDER_GROQ,
            groq("ship it with clovee and jane do"),
            Some(&context),
        )
        .await;
        let blank_reply = maybe_post_process_note_transcript(
            crate::providers::PROVIDER_GROQ,
            groq("keep this"),
            Some(&context),
        )
        .await;
        let failed = maybe_post_process_note_transcript(
            crate::providers::PROVIDER_GROQ,
            groq("keep this too"),
            None,
        )
        .await;
        let other_source = maybe_post_process_note_transcript(
            TEST_TRANSCRIPTION_PROVIDER,
            groq("untouched"),
            Some(&context),
        )
        .await;
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );

        assert_eq!(corrected.text, "Ship it with Clovy and Jane Doe.");
        assert_eq!(blank_reply.text, "keep this");
        assert_eq!(failed.text, "keep this too");
        assert_eq!(other_source.text, "untouched");
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 3, "only Groq transcripts reach the engine");
        let first = requests[0].to_string();
        assert!(first.contains("Term0") && first.contains("Term79"));
        assert!(first.contains("ship it with clovee and jane do"));
    }

    fn groq_rate_limit(retry_after_ms: Option<u64>) -> AppError {
        let mut error = AppError::new(
            "groq_usage_limit_exceeded",
            "Groq reports that its usage limit was reached. Try again later, or check the plan behind your saved Groq key.",
        );
        error.details = retry_after_ms.map(|ms| serde_json::json!({ "retryAfterMs": ms }));
        error
    }

    /// Answers each request in order from `answers`, then succeeds with the
    /// audio path, recording every audio path it was asked for.
    fn scripted_transcriber(
        answers: Vec<Result<&'static str, AppError>>,
    ) -> (TurnTranscriber, Arc<Mutex<Vec<String>>>) {
        let requested = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
        let transcriber = {
            let requested = Arc::clone(&requested);
            Arc::new(move |request: TranscriptionRequest| {
                let path = request.audio_path.to_string_lossy().to_string();
                requested.lock().unwrap().push(path.clone());
                let answer = answers.lock().unwrap().pop_front();
                Box::pin(async move {
                    match answer {
                        Some(Ok(text)) => Ok(test_provider_result(text)),
                        Some(Err(error)) => Err(error),
                        None => Ok(test_provider_result(&path)),
                    }
                }) as TranscriptionFuture
            }) as TurnTranscriber
        };
        (transcriber, requested)
    }

    #[tokio::test(start_paused = true)]
    async fn the_request_pacer_never_starts_more_than_its_budget_per_window() {
        let pacer = RequestPacer::new(18, Duration::from_secs(60)).with_clock(tokio_clock());
        let started = tokio::time::Instant::now();
        let mut starts = Vec::new();
        for _ in 0..40 {
            pacer
                .acquire(GROQ_MIN_BILLED_AUDIO, Duration::MAX)
                .await
                .unwrap();
            starts.push(tokio::time::Instant::now() - started);
        }
        // The first window's budget goes out at once: short notes stay fast.
        assert!(starts[17] < Duration::from_secs(1), "{starts:?}");
        for window in starts.windows(19) {
            assert!(
                window[18] - window[0] >= Duration::from_secs(60),
                "19 starts inside one minute: {window:?}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_pacer_pause_holds_every_caller() {
        let pacer =
            Arc::new(RequestPacer::new(18, Duration::from_secs(60)).with_clock(tokio_clock()));
        pacer.pause_for(Duration::from_secs(30));
        let started = tokio::time::Instant::now();
        let other = {
            let pacer = Arc::clone(&pacer);
            tokio::spawn(async move {
                pacer
                    .acquire(GROQ_MIN_BILLED_AUDIO, Duration::MAX)
                    .await
                    .unwrap();
                tokio::time::Instant::now()
            })
        };
        pacer
            .acquire(GROQ_MIN_BILLED_AUDIO, Duration::MAX)
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_secs(30));
        assert!(other.await.unwrap() - started >= Duration::from_secs(30));
    }

    #[tokio::test(start_paused = true)]
    async fn a_short_groq_rate_limit_waits_and_retries_the_same_request() {
        let (inner, requested) = scripted_transcriber(vec![
            Err(groq_rate_limit(Some(20_000))),
            Ok("after the wait"),
        ]);
        let pacer =
            Arc::new(RequestPacer::new(18, Duration::from_secs(60)).with_clock(tokio_clock()));
        let transcriber = paced_turn_transcriber(inner, Arc::clone(&pacer));
        let started = tokio::time::Instant::now();

        let result = transcriber(TranscriptionRequest {
            provider: "groq".to_string(),
            audio_path: PathBuf::from("m0"),
            title: "Meeting".to_string(),
            context: None,
            language: None,
            operation_id: Some("turn-0".to_string()),
            preview: false,
        })
        .await
        .expect("a short limit is waited out");

        assert_eq!(result.text, "after the wait");
        assert_eq!(*requested.lock().unwrap(), vec!["m0", "m0"]);
        assert!(started.elapsed() >= Duration::from_secs(20));
    }

    #[tokio::test(start_paused = true)]
    async fn a_rate_limit_without_a_wait_header_waits_a_minute() {
        let (inner, requested) =
            scripted_transcriber(vec![Err(groq_rate_limit(None)), Ok("after a minute")]);
        let transcriber = paced_turn_transcriber(
            inner,
            Arc::new(RequestPacer::new(18, Duration::from_secs(60)).with_clock(tokio_clock())),
        );
        let started = tokio::time::Instant::now();

        let result = transcriber(test_transcription_request("m0")).await.unwrap();

        assert_eq!(result.text, "after a minute");
        assert_eq!(requested.lock().unwrap().len(), 2);
        assert!(started.elapsed() >= Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_groq_rate_limit_pauses_with_the_resume_time() {
        let (inner, requested) = scripted_transcriber(vec![Err(groq_rate_limit(Some(3_600_000)))]);
        let transcriber = paced_turn_transcriber(
            inner,
            Arc::new(RequestPacer::new(18, Duration::from_secs(60)).with_clock(tokio_clock())),
        );
        let before_ms = chrono::Utc::now().timestamp_millis();

        let error = transcriber(test_transcription_request("m0"))
            .await
            .expect_err("an hour-long limit pauses the note");

        assert_eq!(error.code, TRANSCRIPTION_RATE_LIMITED_CODE);
        assert_eq!(requested.lock().unwrap().len(), 1);
        let resume_at_ms = error
            .details
            .as_ref()
            .and_then(|details| details.get("resumeAtMs"))
            .and_then(serde_json::Value::as_i64)
            .expect("the pause names when it resumes");
        assert!(resume_at_ms >= before_ms + 3_600_000);
        let resume_at = chrono::DateTime::from_timestamp_millis(resume_at_ms)
            .unwrap()
            .with_timezone(&chrono::Local);
        assert_eq!(
            error.message,
            format!(
                "Groq usage limit reached. Clovy continues transcribing at {}. Parts already transcribed are kept.",
                resume_at.format("%H:%M")
            )
        );
    }

    #[tokio::test]
    async fn a_paused_turn_stops_the_stream_and_skips_the_source_fallback() {
        let mut pause = AppError::new(TRANSCRIPTION_RATE_LIMITED_CODE, "paused");
        pause.details = Some(serde_json::json!({ "resumeAtMs": 1 }));
        let (transcriber, requested) = scripted_transcriber(vec![Ok("m0 text"), Err(pause)]);
        let sunk = Arc::new(Mutex::new(Vec::new()));
        let result_sink = {
            let sunk = Arc::clone(&sunk);
            Arc::new(move |event: CompletedTurnTranscription| {
                let failed = matches!(event.result, TurnTranscriptionResult::Failure(_));
                sunk.lock().unwrap().push(failed);
                Box::pin(async { Ok(()) }) as TurnResultFuture
            }) as TurnResultSink
        };
        let fallback_calls = Arc::new(AtomicUsize::new(0));
        let fallback_preparer = {
            let fallback_calls = Arc::clone(&fallback_calls);
            Arc::new(move |plan: SourceFallbackPlan| {
                fallback_calls.fetch_add(1, Ordering::SeqCst);
                prepare_source_fallback(plan)
            }) as SourceFallbackPreparer
        };
        let fallback = SourceFallbackPlan {
            artifact_id: "artifact-m0".to_string(),
            source: "microphone".to_string(),
            source_path: PathBuf::from("m-full"),
            normalized_path: PathBuf::from("m-full-normalized"),
            temp_dir: std::env::temp_dir(),
            recorded_silence: false,
            all_turns_cover_full_source: false,
            echo_trimmed: false,
            end_ms: 10_000,
            turn_index: 9,
            detected_ms: 10_000,
            durable_job: None,
        };

        let error = transcribe_turn_jobs_bounded(
            vec![
                test_job("m0", "microphone", 0),
                test_job("s1", "system", 1),
                test_job("m2", "microphone", 2),
                test_job("s3", "system", 3),
            ],
            vec![fallback],
            &[],
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            fallback_preparer,
            transcriber,
            Some(result_sink),
            1,
        )
        .await
        .expect_err("a pause ends the run");

        assert_eq!(error.code, TRANSCRIPTION_RATE_LIMITED_CODE);
        // Nothing after the paused turn is sent, and the whole source is not
        // re-sent into the same limit.
        assert_eq!(*requested.lock().unwrap(), vec!["m0", "s1"]);
        assert_eq!(fallback_calls.load(Ordering::SeqCst), 0);
        // The paused turn is recorded as failed, so the next run resets it to
        // pending; the finished turn is kept.
        assert_eq!(*sunk.lock().unwrap(), vec![false, true]);
    }

    fn test_transcription_request(path: &str) -> TranscriptionRequest {
        TranscriptionRequest {
            provider: "groq".to_string(),
            audio_path: PathBuf::from(path),
            title: "Meeting".to_string(),
            context: None,
            language: None,
            operation_id: Some(format!("{path}-op")),
            preview: false,
        }
    }

    /// Three microphone turns, 8 s of silence apart.
    fn write_three_turn_microphone(path: &std::path::Path) {
        let tone = (32_000, 20_000);
        let gap = (128_000, 0);
        write_segmented_wav(path, 16_000, &[tone, gap, tone, gap, tone]);
    }

    async fn rate_limited_run(
        repos: &Repositories,
        note_id: &str,
        session_id: &str,
        sources: Vec<SourceAudioForProcessing>,
        transcriber: TurnTranscriber,
    ) -> Result<NoteDto, AppError> {
        TEST_NOTE_TRANSCRIPTION
            .scope(
                (
                    transcriber,
                    Arc::new(
                        RequestPacer::new(18, Duration::from_secs(60)).with_clock(tokio_clock()),
                    ),
                ),
                process_saved_source_audio(
                    repos,
                    note_id,
                    session_id,
                    RecordingSourceMode::MicrophonePlusSystem,
                    sources,
                    "Rate limited".to_string(),
                    None,
                    None,
                    ProcessingTiming::untracked(),
                ),
            )
            .await
    }

    #[tokio::test]
    async fn a_paused_note_resumes_with_only_the_turns_still_missing() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let _server = install_fake_correction_engine(Arc::new(echo_correction)).await;
        let (repos, session_id) = test_source_processing_repositories().await;
        let note_id: String =
            sqlx::query_scalar::query_scalar("SELECT note_id FROM recording_sessions WHERE id = ?")
                .bind(&session_id)
                .fetch_one(&repos.pool)
                .await
                .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let microphone = directory.path().join("microphone.wav");
        write_three_turn_microphone(&microphone);
        let artifact = repos
            .create_audio_artifact(
                &note_id,
                &session_id,
                &microphone.to_string_lossy(),
                10_000,
                0,
                "checksum",
            )
            .await
            .unwrap();
        let sources = vec![(
            artifact.id.clone(),
            "microphone".to_string(),
            microphone.clone(),
            false,
        )];
        let paused = |text: &str| {
            let mut error = AppError::new(TRANSCRIPTION_RATE_LIMITED_CODE, text);
            error.details = Some(serde_json::json!({ "resumeAtMs": 1 }));
            error
        };

        // First run: turn 0 is transcribed, turn 1 hits a long limit.
        let (first, first_requests) =
            scripted_transcriber(vec![Ok("first turn"), Err(paused("paused at 1"))]);
        let error = rate_limited_run(&repos, &note_id, &session_id, sources.clone(), first)
            .await
            .expect_err("the first run pauses");
        assert_eq!(error.code, TRANSCRIPTION_RATE_LIMITED_CODE);
        assert_eq!(first_requests.lock().unwrap().len(), 2);
        let note = repos.get_note(&note_id).await.unwrap();
        assert_eq!(note.processing_status, ProcessingStatus::Failed);
        assert_eq!(note.last_error.as_deref(), Some("paused at 1"));

        // Second run: turn 0 is reused; only turns 1 and 2 go to Groq.
        let (second, second_requests) =
            scripted_transcriber(vec![Ok("second turn"), Err(paused("paused at 2"))]);
        let error = rate_limited_run(&repos, &note_id, &session_id, sources, second)
            .await
            .expect_err("the second run pauses again");
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
        assert_eq!(error.code, TRANSCRIPTION_RATE_LIMITED_CODE);
        let second_requests = second_requests.lock().unwrap().clone();
        assert_eq!(second_requests.len(), 2, "{second_requests:?}");
        assert!(
            first_requests.lock().unwrap()[0] != second_requests[0],
            "the finished turn was sent again"
        );
        let texts = repos
            .certified_source_turn_transcripts_for_session(&session_id)
            .await
            .unwrap()
            .into_iter()
            .map(|transcript| transcript.text)
            .collect::<Vec<_>>();
        assert_eq!(texts, vec!["first turn", "second turn"]);
    }

    fn microphone_turn(start_ms: i64, end_ms: i64) -> AudioTurn {
        test_audio_turn(
            "artifact-m",
            "microphone",
            PathBuf::from("m"),
            0,
            start_ms,
            end_ms,
        )
    }

    #[test]
    fn a_new_session_groups_turns_up_to_the_final_span() {
        let turns = vec![
            microphone_turn(0, 10_000),
            microphone_turn(11_000, 20_000),
            microphone_turn(21_000, 29_000),
            microphone_turn(30_000, 40_000),
        ];

        let grouped = coalesce_turns_reusing_layout(turns, &[], "session", &HashSet::new());

        assert_eq!(
            grouped
                .iter()
                .map(|turn| (turn.start_ms, turn.end_ms))
                .collect::<Vec<_>>(),
            vec![(0, 40_000)]
        );
    }

    #[test]
    fn a_session_with_turns_from_the_30_second_layout_keeps_that_layout() {
        let turns = vec![
            microphone_turn(0, 10_000),
            microphone_turn(11_000, 20_000),
            microphone_turn(21_000, 29_000),
            microphone_turn(30_000, 40_000),
        ];
        let done = HashSet::from([note_transcription_span_id(
            "session",
            "microphone",
            NoteTranscriptionJobKind::Turn,
            0,
            29_000,
        )]);

        let grouped = coalesce_turns_reusing_layout(turns, &[], "session", &done);

        assert_eq!(
            grouped
                .iter()
                .map(|turn| (turn.start_ms, turn.end_ms))
                .collect::<Vec<_>>(),
            vec![(0, 29_000), (30_000, 40_000)]
        );
    }

    #[test]
    fn the_overlap_repeated_at_a_chunk_start_is_dropped() {
        assert_eq!(
            drop_repeated_overlap(
                "we agreed to ship the release on Friday,",
                "ship the release on friday. Then Ana takes the notes."
            ),
            "Then Ana takes the notes."
        );
        // One shared word is coincidence, not the overlap.
        assert_eq!(
            drop_repeated_overlap("see you then", "then we start"),
            "then we start"
        );
        assert_eq!(
            drop_repeated_overlap("first part", "second part"),
            "second part"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_request_pacer_keeps_an_hour_of_billed_audio_within_its_budget() {
        let pacer = RequestPacer::new(100, Duration::from_secs(60))
            .with_audio_budget(Duration::from_secs(100), Duration::from_secs(3_600))
            .with_clock(tokio_clock());
        for _ in 0..10 {
            pacer
                .acquire(Duration::from_secs(10), Duration::ZERO)
                .await
                .expect("within the hour's audio");
        }

        let wait = pacer
            .acquire(Duration::from_secs(10), Duration::from_secs(90))
            .await
            .expect_err("the hour's audio is spent");

        // The first request's audio leaves the window an hour after it started.
        assert_eq!(wait, Duration::from_secs(3_600));
    }

    #[tokio::test(start_paused = true)]
    async fn the_live_preview_never_waits_and_keeps_to_its_share_of_the_audio() {
        let pacer = RequestPacer::new(3, Duration::from_secs(60))
            .with_audio_budget(Duration::from_secs(100), Duration::from_secs(3_600))
            .with_clock(tokio_clock());
        let share = Duration::from_secs(30);

        assert!(pacer.try_acquire(Duration::from_secs(10), share));
        assert!(pacer.try_acquire(Duration::from_secs(10), share));
        assert!(pacer.try_acquire(Duration::from_secs(10), share));
        // Over its share of the audio, and over the minute's requests.
        assert!(!pacer.try_acquire(Duration::from_secs(10), share));
        tokio::time::advance(Duration::from_secs(61)).await;
        assert!(!pacer.try_acquire(Duration::from_secs(10), share));
        // The final transcription still has the rest of the budget.
        pacer
            .acquire(Duration::from_secs(60), Duration::ZERO)
            .await
            .expect("the final transcription keeps its audio");
        pacer.pause_for(Duration::from_secs(30));
        assert!(!pacer.try_acquire(Duration::from_secs(1), Duration::from_secs(100)));
    }

    #[test]
    fn pack_segments_go_back_to_the_turn_they_were_spoken_in() {
        let spans = vec![(0, 2_000), (2_500, 4_500), (5_000, 7_000)];
        let segment = |start_ms, end_ms, text: &str| TranscriptSegment {
            start_ms,
            end_ms,
            text: text.to_string(),
        };

        let texts = split_pack_segments(
            &spans,
            &[
                segment(0, 1_200, "first"),
                segment(1_200, 2_100, "turn"),
                segment(2_600, 4_400, "second"),
                segment(5_100, 7_000, "third"),
            ],
        );

        assert_eq!(texts, vec!["first turn", "second", "third"]);
    }

    fn normalized_turn_job(
        directory: &std::path::Path,
        name: &str,
        source: &str,
        turn_index: i64,
        seconds: usize,
    ) -> TurnTranscriptionJob {
        let audio_path = directory.join(format!("{name}.wav"));
        write_segmented_wav(&audio_path, 16_000, &[(16_000 * seconds, 20_000)]);
        TurnTranscriptionJob {
            audio_path,
            temp_dir: directory.to_path_buf(),
            ..test_job(name, source, turn_index)
        }
    }

    /// Answers each pack with one segment per packed turn, read from the
    /// pack's own layout: turns of `seconds_per_turn` with the pack gap.
    fn layout_pack_transcriber(
        seconds_per_turn: i64,
        answers: Vec<Result<(), AppError>>,
    ) -> (PackTranscriber, Arc<Mutex<Vec<i64>>>) {
        let pack_lengths_ms = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(VecDeque::from(answers)));
        let transcriber = {
            let pack_lengths_ms = Arc::clone(&pack_lengths_ms);
            Arc::new(move |request: TranscriptionRequest| {
                let length_ms = source_wav_duration_ms(&request.audio_path).unwrap_or_default();
                pack_lengths_ms.lock().unwrap().push(length_ms);
                let answer = answers.lock().unwrap().pop_front().unwrap_or(Ok(()));
                Box::pin(async move {
                    answer?;
                    let turn_ms = seconds_per_turn * 1_000;
                    let turns = (length_ms + PACK_GAP_MS) / (turn_ms + PACK_GAP_MS);
                    Ok(TimedTranscription {
                        language: Some("pt".to_string()),
                        segments: (0..turns)
                            .map(|index| {
                                let start_ms = index * (turn_ms + PACK_GAP_MS);
                                TranscriptSegment {
                                    start_ms,
                                    end_ms: start_ms + turn_ms,
                                    text: format!("spoken {index}"),
                                }
                            })
                            .collect(),
                    })
                }) as ProviderFuture<TimedTranscription>
            }) as PackTranscriber
        };
        (transcriber, pack_lengths_ms)
    }

    async fn transcribe_turn_jobs_packed(
        jobs: Vec<TurnTranscriptionJob>,
        transcriber: TurnTranscriber,
        pack_transcriber: PackTranscriber,
        result_sink: Option<TurnResultSink>,
        max_concurrency: usize,
    ) -> Result<TranscriptionOutcome, AppError> {
        let total_jobs = jobs.len();
        let (sender, receiver) = tokio::sync::mpsc::channel(total_jobs.max(1));
        for (schedule_index, job) in jobs.into_iter().enumerate() {
            sender
                .send(Ok(PreparedTurn {
                    schedule_index,
                    job,
                }))
                .await
                .unwrap();
        }
        drop(sender);
        transcribe_prepared_turn_stream_with_packs(
            receiver,
            total_jobs,
            TEST_TRANSCRIPTION_PROVIDER.to_string(),
            "Meeting".to_string(),
            None,
            &[],
            transcriber,
            Some(pack_transcriber),
            None,
            result_sink,
            max_concurrency,
        )
        .await
    }

    /// Each recorded turn's index and whether it failed.
    type SunkTurns = Arc<Mutex<Vec<(i64, bool)>>>;

    fn counting_sink() -> (TurnResultSink, SunkTurns) {
        let sunk = Arc::new(Mutex::new(Vec::new()));
        let sink = {
            let sunk = Arc::clone(&sunk);
            Arc::new(move |event: CompletedTurnTranscription| {
                let (turn_index, failed) = match &event.result {
                    TurnTranscriptionResult::Candidate(candidate) => {
                        (candidate.input.turn_index.unwrap_or_default(), false)
                    }
                    TurnTranscriptionResult::Failure(failure) => {
                        (failure.input.turn_index.unwrap_or_default(), true)
                    }
                };
                sunk.lock().unwrap().push((turn_index, failed));
                Box::pin(async { Ok(()) }) as TurnResultFuture
            }) as TurnResultSink
        };
        (sink, sunk)
    }

    #[tokio::test]
    async fn each_sources_speech_turns_go_as_one_pack_and_come_back_per_turn() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = vec![
            normalized_turn_job(directory.path(), "m0", "microphone", 0, 2),
            normalized_turn_job(directory.path(), "s1", "system", 1, 2),
            normalized_turn_job(directory.path(), "m2", "microphone", 2, 2),
            normalized_turn_job(directory.path(), "s3", "system", 3, 2),
            normalized_turn_job(directory.path(), "m4", "microphone", 4, 2),
        ];
        let (turn_transcriber, turn_requests) = scripted_transcriber(Vec::new());
        let (pack_transcriber, pack_lengths_ms) = layout_pack_transcriber(2, Vec::new());
        let (sink, sunk) = counting_sink();

        let outcome =
            transcribe_turn_jobs_packed(jobs, turn_transcriber, pack_transcriber, Some(sink), 2)
                .await
                .expect("packs transcribe");

        // One request per Source instead of one per turn; no silence between
        // turns beyond the pack gap.
        let mut lengths = pack_lengths_ms.lock().unwrap().clone();
        lengths.sort();
        assert_eq!(lengths, vec![4_500, 7_000]);
        assert!(turn_requests.lock().unwrap().is_empty());
        let mut texts = outcome
            .candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.input.source.as_str(),
                    candidate.input.turn_index.unwrap(),
                    candidate.input.text.as_str(),
                )
            })
            .collect::<Vec<_>>();
        texts.sort_by_key(|(_, turn_index, _)| *turn_index);
        assert_eq!(
            texts,
            vec![
                ("microphone", 0, "spoken 0"),
                ("system", 1, "spoken 0"),
                ("microphone", 2, "spoken 1"),
                ("system", 3, "spoken 1"),
                ("microphone", 4, "spoken 2"),
            ]
        );
        // Every turn is recorded on its own durable job.
        let mut sunk = sunk.lock().unwrap().clone();
        sunk.sort();
        assert_eq!(
            sunk,
            vec![(0, false), (1, false), (2, false), (3, false), (4, false)]
        );
    }

    #[tokio::test]
    async fn a_pack_never_carries_more_than_the_final_span() {
        let directory = tempfile::tempdir().unwrap();
        // Three 250 s turns: two fit in 600 s, the third goes alone.
        let jobs = (0..3)
            .map(|index| {
                normalized_turn_job(
                    directory.path(),
                    &format!("m{index}"),
                    "microphone",
                    index,
                    250,
                )
            })
            .collect::<Vec<_>>();
        let (turn_transcriber, turn_requests) = scripted_transcriber(Vec::new());
        let (pack_transcriber, pack_lengths_ms) = layout_pack_transcriber(250, Vec::new());

        transcribe_turn_jobs_packed(jobs, turn_transcriber, pack_transcriber, None, 1)
            .await
            .expect("turns transcribe");

        assert_eq!(*pack_lengths_ms.lock().unwrap(), vec![500_500]);
        assert_eq!(turn_requests.lock().unwrap().len(), 1);
        assert!(
            *pack_lengths_ms.lock().unwrap().iter().max().unwrap() <= FINAL_TRANSCRIPTION_SPAN_MS
        );
    }

    #[tokio::test]
    async fn a_paused_pack_fails_its_turns_and_stops_the_run() {
        let directory = tempfile::tempdir().unwrap();
        let jobs = vec![
            normalized_turn_job(directory.path(), "m0", "microphone", 0, 2),
            normalized_turn_job(directory.path(), "m1", "microphone", 1, 2),
            normalized_turn_job(directory.path(), "s2", "system", 2, 2),
            normalized_turn_job(directory.path(), "s3", "system", 3, 2),
        ];
        let mut pause = AppError::new(TRANSCRIPTION_RATE_LIMITED_CODE, "paused");
        pause.details = Some(serde_json::json!({ "resumeAtMs": 1 }));
        let (turn_transcriber, _) = scripted_transcriber(Vec::new());
        let (pack_transcriber, pack_lengths_ms) = layout_pack_transcriber(2, vec![Err(pause)]);
        let (sink, sunk) = counting_sink();

        let error =
            transcribe_turn_jobs_packed(jobs, turn_transcriber, pack_transcriber, Some(sink), 1)
                .await
                .expect_err("the pause ends the run");

        assert_eq!(error.code, TRANSCRIPTION_RATE_LIMITED_CODE);
        // The microphone pack is recorded as failed (pending next run); the
        // system pack is never sent into the same limit.
        assert_eq!(pack_lengths_ms.lock().unwrap().len(), 1);
        assert_eq!(*sunk.lock().unwrap(), vec![(0, true), (1, true)]);
    }

    /// A wall clock that follows tokio's (pausable) time, for paced tests.
    fn tokio_clock() -> PacerClock {
        let started = tokio::time::Instant::now();
        let base_ms = 1_000_000_000_i64;
        Arc::new(move || base_ms + started.elapsed().as_millis() as i64)
    }

    /// A wall clock the test moves by hand, in milliseconds since the epoch.
    fn manual_clock(start_ms: i64) -> (PacerClock, Arc<AtomicI64>) {
        let now_ms = Arc::new(AtomicI64::new(start_ms));
        let clock = {
            let now_ms = Arc::clone(&now_ms);
            Arc::new(move || now_ms.load(Ordering::SeqCst)) as PacerClock
        };
        (clock, now_ms)
    }

    #[tokio::test]
    async fn a_long_full_source_job_keeps_its_finished_chunks_across_hourly_pauses() {
        // Two audible hours in 600 s chunks bill more than the 7,200 s an
        // hour allows. Scaled down here: eight 30 s chunks against 100 s an
        // hour, so every run pauses before the last chunk.
        let (repos, session_id) = test_source_processing_repositories().await;
        let directory = tempfile::tempdir().unwrap();
        let audio_path = directory.path().join("microphone.wav");
        write_loud_wav(&audio_path, 16_000, 16_000 * 240);
        let (clock, now_ms) = manual_clock(1_000_000_000);
        let pacer = Arc::new(
            RequestPacer::new(18, Duration::from_secs(60))
                .with_audio_budget(Duration::from_secs(100), Duration::from_secs(3_600))
                .with_clock(clock),
        );
        let requested = Arc::new(Mutex::new(Vec::new()));
        let inner = {
            let requested = Arc::clone(&requested);
            Arc::new(move |request: TranscriptionRequest| {
                let operation_id = request.operation_id();
                requested.lock().unwrap().push(operation_id.clone());
                let index = operation_id
                    .rsplit("-chunk-")
                    .next()
                    .unwrap_or_default()
                    .to_string();
                Box::pin(async move { Ok(test_provider_result(&format!("chunk {index}"))) })
                    as TranscriptionFuture
            }) as TurnTranscriber
        };
        let transcriber = note_chain(
            ignoring_route(inner),
            fixed_route(crate::providers::DEFAULT_TRANSCRIPTION_MODEL, pacer),
            repos.clone(),
        );
        let operation_id = format!("{session_id}:microphone:turn:0:240000:fingerprint");

        let mut runs = 0;
        let result = loop {
            runs += 1;
            assert!(runs <= 4, "the note never finishes");
            match transcribe_prepared_audio(
                Arc::clone(&transcriber),
                TranscribePreparedAudioRequest {
                    provider: "test".to_string(),
                    audio_path: audio_path.clone(),
                    temp_dir: directory.path().to_path_buf(),
                    chunk_stem: "microphone-turn-0".to_string(),
                    title: "Meeting".to_string(),
                    asr_hint: None,
                    operation_id: operation_id.clone(),
                    max_chunk_ms: Some(30_000),
                },
            )
            .await
            {
                Ok(result) => break result,
                Err(error) => {
                    assert_eq!(error.code, TRANSCRIPTION_RATE_LIMITED_CODE);
                    // The resume runs once the hour's audio has left the window.
                    now_ms.fetch_add(3_600_000, Ordering::SeqCst);
                }
            }
        };

        assert_eq!(runs, 3);
        let requested = requested.lock().unwrap().clone();
        assert_eq!(
            requested.len(),
            8,
            "a finished chunk was sent again: {requested:?}"
        );
        assert_eq!(
            result.transcript.text,
            (0..8)
                .map(|index| format!("chunk {index}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[tokio::test]
    async fn a_pause_ends_by_the_wall_clock_even_when_the_mac_slept() {
        let (clock, now_ms) = manual_clock(1_000_000_000);
        let pacer = RequestPacer::new(18, Duration::from_secs(60)).with_clock(clock);
        pacer.pause_for(Duration::from_secs(30 * 60));
        assert!(pacer
            .acquire(GROQ_MIN_BILLED_AUDIO, Duration::ZERO)
            .await
            .is_err());

        // Asleep for 31 minutes: the wall clock moved, the monotonic one did not.
        now_ms.fetch_add(31 * 60 * 1_000, Ordering::SeqCst);

        pacer
            .acquire(GROQ_MIN_BILLED_AUDIO, Duration::ZERO)
            .await
            .expect("the pause ended while the Mac slept");
    }

    #[tokio::test]
    async fn a_clock_set_back_does_not_stretch_a_pause() {
        let (clock, now_ms) = manual_clock(1_000_000_000);
        let pacer = RequestPacer::new(18, Duration::from_secs(60)).with_clock(clock);
        pacer.pause_for(Duration::from_secs(60));

        now_ms.fetch_sub(2 * 3_600 * 1_000, Ordering::SeqCst);
        let wait = pacer
            .acquire(GROQ_MIN_BILLED_AUDIO, Duration::ZERO)
            .await
            .expect_err("still paused");
        assert!(wait <= Duration::from_secs(60), "{wait:?}");

        now_ms.fetch_add(61 * 1_000, Ordering::SeqCst);
        pacer
            .acquire(GROQ_MIN_BILLED_AUDIO, Duration::ZERO)
            .await
            .expect("the pause ends 60 s of clock later");
    }

    #[tokio::test]
    async fn a_pause_on_one_groq_key_does_not_hold_another_key() {
        let key_a = format!("gsk-a-{}", uuid::Uuid::new_v4());
        let key_b = format!("gsk-b-{}", uuid::Uuid::new_v4());
        groq_direct_pacer(&key_fingerprint(&key_a)).pause_for(Duration::from_secs(3_600));

        assert!(groq_direct_pacer(&key_fingerprint(&key_a))
            .acquire(GROQ_MIN_BILLED_AUDIO, Duration::ZERO)
            .await
            .is_err());
        groq_direct_pacer(&key_fingerprint(&key_b))
            .acquire(GROQ_MIN_BILLED_AUDIO, Duration::ZERO)
            .await
            .expect("another Groq account is not paused");
    }

    #[test]
    fn a_key_fingerprint_never_holds_the_key() {
        let key = "gsk-secret-value-1234567890";
        let fingerprint = key_fingerprint(key);
        assert_eq!(fingerprint.len(), 16);
        assert!(fingerprint
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        assert!(!fingerprint.contains("secret") && !key.contains(&fingerprint));
        assert_eq!(fingerprint, key_fingerprint(key));
        assert_ne!(fingerprint, key_fingerprint("gsk-other"));
    }

    async fn paused_note(repos: &Repositories, session_id: &str, message: &str) -> String {
        let note_id: String =
            sqlx::query_scalar::query_scalar("SELECT note_id FROM recording_sessions WHERE id = ?")
                .bind(session_id)
                .fetch_one(&repos.pool)
                .await
                .unwrap();
        repos
            .set_note_status(
                &note_id,
                ProcessingStatus::Failed,
                Some(message.to_string()),
            )
            .await
            .unwrap();
        note_id
    }

    #[tokio::test]
    async fn a_paused_note_is_claimed_for_its_resume_only_once() {
        let (repos, session_id) = test_source_processing_repositories().await;
        let paused = "Groq usage limit reached. Clovy continues transcribing at 14:35. Parts already transcribed are kept.";
        let note_id = paused_note(&repos, &session_id, paused).await;

        let (first, second) = tokio::join!(
            repos.claim_paused_note(&note_id, paused),
            repos.claim_paused_note(&note_id, paused)
        );

        assert_eq!(
            [first.unwrap(), second.unwrap()]
                .iter()
                .filter(|claimed| **claimed)
                .count(),
            1
        );
        let note = repos.get_note(&note_id).await.unwrap();
        assert_eq!(note.processing_status, ProcessingStatus::Transcribing);
    }

    #[tokio::test]
    async fn a_manual_retry_first_leaves_no_pause_to_claim() {
        let (repos, session_id) = test_source_processing_repositories().await;
        let paused = "Groq usage limit reached. Clovy continues transcribing at 09:05. Parts already transcribed are kept.";
        let note_id = paused_note(&repos, &session_id, paused).await;
        // Retry sets the note to transcribing before it queues its run.
        repos
            .set_note_status(&note_id, ProcessingStatus::Transcribing, None)
            .await
            .unwrap();

        assert!(!repos.claim_paused_note(&note_id, paused).await.unwrap());
        // Nor a pause other than the one the resume was scheduled for.
        repos
            .set_note_status(
                &note_id,
                ProcessingStatus::Failed,
                Some("Other".to_string()),
            )
            .await
            .unwrap();
        assert!(!repos.claim_paused_note(&note_id, paused).await.unwrap());
    }

    #[tokio::test]
    async fn parakeet_review_imprecise_pack_tokens_use_individual_local_turns() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        use crate::providers::local_transcriber::{
            FakeLocalTranscriber, LocalTranscription, NetworkSentinel, Token, TEST_LOCAL,
        };
        use crate::providers::transcription::TEST_BACKEND;
        assert_eq!(PACK_GAP_MS, 500, "the Groq pack gap must stay unchanged");
        // A 160 ms late token end bridges the artificial 500 ms pack gap.
        let timed = LocalTranscription {
            text: "Primeiro Segundo Terceiro".into(),
            tokens: [
                (0, 660, "▁Primeiro"),
                (1000, 1660, "▁Segundo"),
                (2000, 2500, "▁Terceiro"),
            ]
            .into_iter()
            .map(|(start_ms, end_ms, t)| Token {
                t: t.into(),
                start_ms,
                end_ms,
            })
            .collect(),
            duration_ms: 2500,
            language: Some("pt".into()),
        }
        .timed()
        .unwrap();
        assert_eq!(
            split_pack_segments(&[(0, 500), (1000, 1500), (2000, 2500)], &timed.segments),
            vec!["", "Primeiro Segundo Terceiro", ""]
        );
        let fake = Arc::new(FakeLocalTranscriber::default());
        let sentinel = NetworkSentinel::start().await;
        let directory = tempfile::tempdir().unwrap();
        crate::providers::TEST_ORIGIN.scope(sentinel.origin.clone(),
            TEST_BACKEND.scope(local_test_route().backend,
                TEST_LOCAL.scope(fake.clone(), async {
                    assert!(note_pack_transcriber("parakeet-ultra".into()).is_none(),
                        "the design's per-turn Local path must prevent attribution across pack boundaries");
                    let transcriber = routed(note_provider_call(), Arc::new(|| Ok(local_test_route())));
                    for index in 0..3 {
                        let job = normalized_turn_job(directory.path(), &format!("local-{index}"), "microphone", index, 1);
                        let event = transcribe_one_turn_job(job, "local".into(), "Reunião".into(),
                            None, None, transcriber.clone(), None).await.unwrap();
                        let TurnTranscriptionResult::Candidate(candidate) = event.result else {
                            panic!("each Local turn must keep its own transcript");
                        };
                        assert_eq!(candidate.input.turn_index, Some(index));
                        assert_eq!(candidate.input.text, "Olá mundo");
                    }
                })
            )
        ).await;
        assert_eq!(fake.calls.lock().unwrap().len(), 3);
        sentinel.assert_unused();
    }

    #[tokio::test]
    async fn local_forty_minute_note_keeps_chunking_without_groq_pacing_or_network() {
        use crate::providers::{
            local_transcriber::{FakeLocalTranscriber, NetworkSentinel, TEST_LOCAL},
            transcription::TEST_BACKEND,
            TEST_ORIGIN,
        };
        let sentinel = NetworkSentinel::start().await;
        let fake = Arc::new(FakeLocalTranscriber::default());
        let (repos, _) = test_source_processing_repositories().await;
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("forty-minutes.wav");
        write_loud_wav(&audio, 1000, 1000 * 40 * 60);
        TEST_ORIGIN
            .scope(
                sentinel.origin.clone(),
                TEST_BACKEND.scope(
                    local_test_route().backend,
                    TEST_LOCAL.scope(fake.clone(), async {
                        let transcriber = note_turn_transcriber(repos, "parakeet-ultra".into());
                        tokio::time::timeout(
                            Duration::from_secs(10),
                            transcribe_prepared_audio(
                                transcriber,
                                TranscribePreparedAudioRequest {
                                    provider: "local".into(),
                                    audio_path: audio,
                                    temp_dir: directory.path().into(),
                                    chunk_stem: "local-chunk".into(),
                                    title: "Reunião".into(),
                                    asr_hint: None,
                                    operation_id: "forty-minutes".into(),
                                    max_chunk_ms: Some(600_000),
                                },
                            ),
                        )
                        .await
                        .expect("Local must not wait for Groq budgets")
                        .unwrap();
                        assert!(fake.calls.lock().unwrap().len() >= 4);
                        assert!(note_pack_transcriber("parakeet-ultra".into()).is_none());
                        let preview =
                            live_preview_route(40 * 60 * 1000).expect("Local preview enabled");
                        transcribe_on_route(preview, test_transcription_request("preview"))
                            .await
                            .unwrap();
                    }),
                ),
            )
            .await;
        sentinel.assert_unused();
    }

    fn local_test_route() -> TranscriptionRoute {
        TranscriptionRoute {
            backend: TranscriptionBackend::Local {
                model_dir: PathBuf::from("/fixture/model"),
            },
            model: "parakeet-ultra".into(),
            pacer: None,
        }
    }

    #[tokio::test]
    async fn parakeet_review_groq_run_cannot_change_its_admitted_model_to_local() {
        crate::providers::transcription::TEST_BACKEND.scope(
            TranscriptionBackend::Local { model_dir: "/fixture/model".into() },
            async {
                let error = note_route("whisper-large-v3".into()).unwrap_err();
                assert_eq!(error.code, "transcription_engine_changed");
                assert_eq!(error.message, "The transcription engine changed during processing. Retry to transcribe with the current engine.");
            },
        ).await;
    }

    #[tokio::test]
    async fn local_run_cannot_forward_its_model_or_audio_to_groq_after_switching() {
        crate::providers::transcription::TEST_BACKEND
            .scope(
                TranscriptionBackend::Groq {
                    key: "fixture-key".into(),
                    model: "whisper-large-v3".into(),
                },
                async {
                    let error = note_route(crate::providers::transcription::LOCAL_MODEL.into())
                        .unwrap_err();
                    assert_eq!(error.code, "transcription_engine_changed");
                    assert_eq!(error.message, TRANSCRIPTION_ENGINE_CHANGED_MESSAGE);
                },
            )
            .await;
    }

    #[tokio::test]
    async fn local_route_transcribes_turns_packs_and_preview_without_a_groq_key() {
        use crate::providers::local_transcriber::{FakeLocalTranscriber, TEST_LOCAL};
        let fake = Arc::new(FakeLocalTranscriber::default());
        TEST_LOCAL
            .scope(fake.clone(), async {
                let plain =
                    note_provider_call()(local_test_route(), test_transcription_request("turn"))
                        .await
                        .unwrap();
                assert_eq!(plain.provider, "local");
                assert_eq!(plain.text, "Olá mundo");
                let timed =
                    note_timed_call()(local_test_route(), test_transcription_request("pack"))
                        .await
                        .unwrap();
                assert_eq!(timed.segments.len(), 3);
                let route = live_preview_route_from(Ok(local_test_route()), 40 * 60 * 1000)
                    .expect("Local has no hourly budget");
                assert_eq!(
                    transcribe_on_route(route, test_transcription_request("preview"))
                        .await
                        .unwrap()
                        .provider,
                    "local"
                );
                assert_eq!(fake.calls.lock().unwrap().len(), 3);
            })
            .await;
    }

    #[tokio::test]
    async fn local_failure_is_not_retried_and_keeps_actionable_note_message() {
        use crate::providers::local_transcriber::{
            FakeLocalTranscriber, LOCAL_TRANSCRIBER_FAILED_MESSAGE, TEST_LOCAL,
        };
        let fake = Arc::new(FakeLocalTranscriber {
            fail: true,
            ..Default::default()
        });
        TEST_LOCAL
            .scope(fake.clone(), async {
                let transcriber = routed(note_provider_call(), Arc::new(|| Ok(local_test_route())));
                let error = transcribe_with_transient_retries(
                    &transcriber,
                    test_transcription_request("failed-local"),
                )
                .await
                .unwrap_err();
                assert_eq!(error.code, "local_transcriber_failed");
                assert_eq!(fake.calls.lock().unwrap().len(), 1);
                assert_eq!(
                    user_facing_transcription_failure_message(
                        &error.code,
                        &error.message,
                        false,
                        "microphone"
                    ),
                    LOCAL_TRANSCRIBER_FAILED_MESSAGE
                );
            })
            .await;
    }

    #[tokio::test]
    async fn local_failure_persists_note_error_and_retains_audio_for_retry() {
        use crate::providers::local_transcriber::{
            FakeLocalTranscriber, NetworkSentinel, LOCAL_TRANSCRIBER_FAILED_MESSAGE, TEST_LOCAL,
        };
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let (repos, session_id) = test_source_processing_repositories().await;
        let note_id: String =
            sqlx::query_scalar::query_scalar("SELECT note_id FROM recording_sessions WHERE id = ?")
                .bind(&session_id)
                .fetch_one(&repos.pool)
                .await
                .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let microphone = directory.path().join("microphone.wav");
        write_three_turn_microphone(&microphone);
        let artifact = repos
            .create_audio_artifact(
                &note_id,
                &session_id,
                &microphone.to_string_lossy(),
                22_000,
                0,
                "checksum",
            )
            .await
            .unwrap();
        let fake = Arc::new(FakeLocalTranscriber {
            fail: true,
            ..Default::default()
        });
        let sentinel = NetworkSentinel::start().await;
        crate::providers::TEST_ORIGIN
            .scope(
                sentinel.origin.clone(),
                TEST_LOCAL.scope(fake.clone(), async {
                    let transcriber =
                        routed(note_provider_call(), Arc::new(|| Ok(local_test_route())));
                    let result = rate_limited_run(
                        &repos,
                        &note_id,
                        &session_id,
                        vec![(artifact.id, "microphone".into(), microphone.clone(), false)],
                        transcriber,
                    )
                    .await;
                    assert_eq!(result.unwrap_err().code, "local_transcriber_failed");
                    let note = repos.get_note(&note_id).await.unwrap();
                    assert_eq!(note.processing_status, ProcessingStatus::Failed);
                    assert_eq!(
                        note.last_error.as_deref(),
                        Some(format!("Microphone: {LOCAL_TRANSCRIBER_FAILED_MESSAGE}").as_str())
                    );
                    assert!(microphone.is_file(), "retry must retain the recording");
                    assert!(!fake.calls.lock().unwrap().is_empty());
                }),
            )
            .await;
        sentinel.assert_unused();
    }

    #[tokio::test]
    async fn groq_engine_switch_persists_actionable_note_error_without_network() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let (repos, session_id) = test_source_processing_repositories().await;
        let note_id: String =
            sqlx::query_scalar::query_scalar("SELECT note_id FROM recording_sessions WHERE id = ?")
                .bind(&session_id)
                .fetch_one(&repos.pool)
                .await
                .unwrap();
        let directory = tempfile::tempdir().unwrap();
        let microphone = directory.path().join("microphone.wav");
        write_three_turn_microphone(&microphone);
        let artifact = repos
            .create_audio_artifact(
                &note_id,
                &session_id,
                &microphone.to_string_lossy(),
                22_000,
                0,
                "checksum",
            )
            .await
            .unwrap();
        let sentinel = crate::providers::local_transcriber::NetworkSentinel::start().await;
        crate::providers::TEST_ORIGIN.scope(sentinel.origin.clone(), async {
            let transcriber = routed(note_provider_call(), Arc::new(|| note_route_for_backend(
                "whisper-large-v3".into(),
                TranscriptionBackend::Local { model_dir: "/fixture/model".into() },
            )));
            let error = rate_limited_run(
                &repos, &note_id, &session_id,
                vec![(artifact.id, "microphone".into(), microphone.clone(), false)],
                transcriber,
            ).await.unwrap_err();
            assert_eq!(error.code, "transcription_engine_changed");
            let note = repos.get_note(&note_id).await.unwrap();
            assert_eq!(note.processing_status, ProcessingStatus::Failed);
            assert_eq!(note.last_error.as_deref(), Some("Microphone: The transcription engine changed during processing. Retry to transcribe with the current engine."));
            assert!(microphone.is_file(), "retry must retain the recording");
        }).await;
        sentinel.assert_unused();
    }

    #[tokio::test]
    async fn local_route_accepts_audio_above_groq_upload_limit() {
        use crate::providers::local_transcriber::{
            FakeLocalTranscriber, NetworkSentinel, TEST_LOCAL,
        };
        let directory = tempfile::tempdir().unwrap();
        let audio = directory.path().join("large.wav");
        std::fs::File::create(&audio)
            .unwrap()
            .set_len(26_214_401)
            .unwrap();
        let fake = Arc::new(FakeLocalTranscriber::default());
        let sentinel = NetworkSentinel::start().await;
        crate::providers::TEST_ORIGIN
            .scope(
                sentinel.origin.clone(),
                TEST_LOCAL.scope(fake.clone(), async {
                    let mut request = test_transcription_request("large-local");
                    request.audio_path = audio;
                    assert_eq!(
                        note_provider_call()(local_test_route(), request)
                            .await
                            .unwrap()
                            .provider,
                        "local"
                    );
                }),
            )
            .await;
        assert_eq!(fake.calls.lock().unwrap().len(), 1);
        sentinel.assert_unused();
    }

    fn test_route(
        groq_key: Option<&str>,
        model: &str,
        pacer: Arc<RequestPacer>,
    ) -> TranscriptionRoute {
        TranscriptionRoute {
            backend: TranscriptionBackend::Groq {
                key: groq_key.unwrap_or_default().into(),
                model: model.into(),
            },
            model: model.to_string(),
            pacer: Some(pacer),
        }
    }

    /// A provider call that records each request's route and answers with
    /// the chunk index of its operation id.
    /// Each call's Groq key, model and operation id.
    type RoutedCalls = Arc<Mutex<Vec<(Option<String>, String, String)>>>;

    fn recording_routed_call() -> (RoutedCall<TranscriptionProviderResult>, RoutedCalls) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let call = {
            let calls = Arc::clone(&calls);
            Arc::new(
                move |route: TranscriptionRoute, request: TranscriptionRequest| {
                    let operation_id = request.operation_id();
                    calls.lock().unwrap().push((
                        route.groq_key().map(str::to_string),
                        route.model.clone(),
                        operation_id.clone(),
                    ));
                    Box::pin(async move { Ok(test_provider_result(&operation_id)) })
                        as ProviderFuture<TranscriptionProviderResult>
                },
            ) as RoutedCall<TranscriptionProviderResult>
        };
        (call, calls)
    }

    #[tokio::test]
    async fn kept_chunks_of_another_model_are_not_reused() {
        let (repos, session_id) = test_source_processing_repositories().await;
        let directory = tempfile::tempdir().unwrap();
        let audio_path = directory.path().join("microphone.wav");
        write_loud_wav(&audio_path, 16_000, 16_000 * 90);
        let (call, calls) = recording_routed_call();
        let pacer = Arc::new(RequestPacer::new(18, Duration::from_secs(60)));
        let run = |model: &'static str| {
            let transcriber = note_chain(
                Arc::clone(&call),
                {
                    let pacer = Arc::clone(&pacer);
                    Arc::new(move || Ok(test_route(Some("gsk-a"), model, Arc::clone(&pacer))))
                },
                repos.clone(),
            );
            transcribe_prepared_audio(
                transcriber,
                TranscribePreparedAudioRequest {
                    provider: "test".to_string(),
                    audio_path: audio_path.clone(),
                    temp_dir: directory.path().to_path_buf(),
                    chunk_stem: "microphone-turn-0".to_string(),
                    title: "Meeting".to_string(),
                    asr_hint: None,
                    operation_id: format!("{session_id}:microphone:turn:0:90000:fingerprint"),
                    max_chunk_ms: Some(30_000),
                },
            )
        };

        run("whisper-large-v3-turbo").await.unwrap();
        run("whisper-large-v3-turbo").await.unwrap();
        assert_eq!(calls.lock().unwrap().len(), 3, "same model: chunks reused");

        // The user switched models between the pause and the resume.
        run("whisper-large-v3").await.unwrap();
        let calls = calls.lock().unwrap().clone();
        assert_eq!(calls.len(), 6, "{calls:?}");
        assert!(calls[3..]
            .iter()
            .all(|(_, model, _)| model == "whisper-large-v3"));
    }

    #[tokio::test]
    async fn each_request_reads_its_route_once_for_the_pacer_and_the_call() {
        let (repos, _session_id) = test_source_processing_repositories().await;
        let paused = Arc::new(RequestPacer::new(18, Duration::from_secs(60)));
        paused.pause_for(Duration::from_secs(3_600));
        let free = Arc::new(RequestPacer::new(18, Duration::from_secs(60)));
        // The saved key changes from A (paused account) to B between reads.
        let routes = Arc::new(Mutex::new(VecDeque::from(vec![
            test_route(Some("gsk-a"), "whisper-large-v3-turbo", Arc::clone(&paused)),
            test_route(Some("gsk-b"), "whisper-large-v3-turbo", Arc::clone(&free)),
            test_route(Some("gsk-a"), "whisper-large-v3-turbo", Arc::clone(&paused)),
            test_route(Some("gsk-b"), "whisper-large-v3-turbo", Arc::clone(&free)),
        ])));
        let reads = Arc::new(AtomicUsize::new(0));
        let resolve = {
            let routes = Arc::clone(&routes);
            let reads = Arc::clone(&reads);
            Arc::new(move || {
                reads.fetch_add(1, Ordering::SeqCst);
                Ok(routes
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("a route per read"))
            }) as RouteResolver
        };
        let (call, calls) = recording_routed_call();
        let transcriber = note_chain(call, resolve, repos);

        let first = transcriber(test_transcription_request("m0")).await;
        let second = transcriber(test_transcription_request("m1")).await;

        // Key A's account is paused: its request waits for A, not B's slot.
        assert_eq!(
            first.expect_err("A is paused").code,
            TRANSCRIPTION_RATE_LIMITED_CODE
        );
        assert!(second.is_ok());
        assert_eq!(
            reads.load(Ordering::SeqCst),
            2,
            "one route read per request"
        );
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .map(|(key, _, _)| key.clone())
                .collect::<Vec<_>>(),
            vec![Some("gsk-b".to_string())]
        );
    }

    fn unreadable_groq_key() -> AppError {
        AppError::new(
            "provider_api_key_unavailable",
            "Clovy could not read your saved Groq key. Open Settings and save it again.",
        )
    }

    #[test]
    fn an_unreadable_saved_key_fails_the_route_instead_of_using_clovy_api() {
        let model = crate::providers::DEFAULT_TRANSCRIPTION_MODEL.to_string();

        let error = note_route_from(Err(unreadable_groq_key()), model.clone())
            .expect_err("a saved key the store cannot return is an error");
        assert_eq!(error.code, "provider_api_key_unavailable");

        let error = note_route_from(Ok(None), model.clone()).unwrap_err();
        assert_eq!(error.code, "groq_api_key_missing");
        let direct = note_route_from(Ok(Some("gsk-user".to_string())), model).unwrap();
        assert_eq!(direct.groq_key(), Some("gsk-user"));
    }

    #[tokio::test]
    async fn a_request_whose_route_fails_reaches_no_provider_and_names_the_key() {
        let (call, calls) = recording_routed_call();
        let transcriber = routed(
            call,
            Arc::new(|| {
                note_route_from(
                    Err(unreadable_groq_key()),
                    crate::providers::DEFAULT_TRANSCRIPTION_MODEL.to_string(),
                )
            }) as RouteResolver,
        );

        let error = transcriber(test_transcription_request("m0"))
            .await
            .expect_err("the route failed");

        assert_eq!(error.code, "provider_api_key_unavailable");
        assert!(calls.lock().unwrap().is_empty(), "nothing was sent");
    }

    #[test]
    fn the_live_preview_sends_nothing_when_the_saved_key_cannot_be_read() {
        let route = note_route_from(
            Err(unreadable_groq_key()),
            crate::providers::DEFAULT_TRANSCRIPTION_MODEL.to_string(),
        );
        assert!(live_preview_route_from(route, 8_000).is_none());
    }

    #[test]
    fn direct_only_missing_groq_key_does_not_start_live_preview() {
        let route = note_route_from(
            Ok(None),
            crate::providers::DEFAULT_TRANSCRIPTION_MODEL.to_string(),
        );
        assert!(live_preview_route_from(route, 8_000).is_none());
    }

    #[test]
    fn direct_only_missing_groq_key_names_the_setup_action() {
        let error = note_route_from(
            Ok(None),
            crate::providers::DEFAULT_TRANSCRIPTION_MODEL.to_string(),
        )
        .unwrap_err();
        assert_eq!(error.code, "groq_api_key_missing");
        assert!(error.message.contains("Settings"));
    }

    #[tokio::test]
    async fn a_manual_retry_only_marks_a_note_that_is_still_failed() {
        let (repos, session_id) = test_source_processing_repositories().await;
        let note_id = paused_note(&repos, &session_id, "Some failure").await;

        assert!(repos.mark_failed_note_transcribing(&note_id).await.unwrap());
        let note = repos.get_note(&note_id).await.unwrap();
        assert_eq!(note.processing_status, ProcessingStatus::Transcribing);
        assert_eq!(note.last_error, None);

        // Never over a note another run finished or is processing.
        for status in [ProcessingStatus::Ready, ProcessingStatus::Transcribing] {
            repos.set_note_status(&note_id, status, None).await.unwrap();
            assert!(!repos.mark_failed_note_transcribing(&note_id).await.unwrap());
            assert_eq!(
                repos.get_note(&note_id).await.unwrap().processing_status,
                status
            );
        }
    }

    #[tokio::test]
    async fn a_retry_suspended_while_the_resume_finished_leaves_the_note_ready() {
        use crate::domain::processing_queue::{enqueue, queued_retry_should_run, RetryOrigin};
        let (repos, session_id) = test_source_processing_repositories().await;
        let paused = "Groq usage limit reached. Clovy continues transcribing at 11:20. Parts already transcribed are kept.";
        let note_id = paused_note(&repos, &session_id, paused).await;

        // 1. The timer claims the pause.
        assert!(repos.claim_paused_note(&note_id, paused).await.unwrap());
        // 2. The manual retry queues first and is suspended before it marks
        //    the note.
        let (manual, _) = enqueue(&note_id);
        // 3. The resume queues second, takes the lock first and finishes.
        let (resume, _) = enqueue(&note_id);
        assert!(queued_retry_should_run(
            RetryOrigin::Resume,
            repos.get_note(&note_id).await.unwrap().processing_status,
            resume.runs_completed_since_enqueue(),
            false
        ));
        repos
            .set_note_status(&note_id, ProcessingStatus::Ready, None)
            .await
            .unwrap();
        resume.finish();
        // 4. The manual retry wakes up: it must not mark the ready note.
        let marked = repos.mark_failed_note_transcribing(&note_id).await.unwrap();
        let status = repos.get_note(&note_id).await.unwrap().processing_status;
        assert!(!queued_retry_should_run(
            RetryOrigin::Manual,
            status,
            manual.runs_completed_since_enqueue(),
            marked
        ));
        manual.finish();

        // 5. Skipped, it left the note as the resume left it.
        assert_eq!(
            repos.get_note(&note_id).await.unwrap().processing_status,
            ProcessingStatus::Ready
        );
    }
}
