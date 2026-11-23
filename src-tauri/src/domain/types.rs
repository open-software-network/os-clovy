use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: None,
        }
    }
}

impl From<sqlx::error::Error> for AppError {
    fn from(value: sqlx::error::Error) -> Self {
        Self::new("storage_unavailable", value.to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapResponse {
    pub folders: Vec<FolderDto>,
    pub notes: Vec<NoteListItemDto>,
    pub active_recoveries: Vec<RecoverableRecordingDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_recording: Option<RecordingStatusDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListNotesResponse {
    pub items: Vec<NoteListItemDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDto {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default)]
    pub memory_disabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeProjectCandidateDto {
    pub name: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    pub already_added: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemoryDto {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<String>,
    pub content: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MemorySettingsDto {
    #[serde(default = "memory_enabled_by_default")]
    pub enabled: bool,
}

impl Default for MemorySettingsDto {
    fn default() -> Self {
        Self { enabled: true }
    }
}

fn memory_enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteListItemDto {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub processing_status: ProcessingStatus,
    pub folder_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Monotonic compare-and-swap revision for remote-safe edits.
    pub revision: u64,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteDto {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub processing_status: ProcessingStatus,
    pub folder_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Monotonic compare-and-swap revision for remote-safe edits.
    pub revision: u64,
    pub duration_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_event: Option<NoteCalendarEventDto>,
    #[serde(default)]
    pub video_analysis: Option<String>,
    pub generated_content: Option<String>,
    pub edited_content: Option<String>,
    pub transcript: Option<TranscriptDto>,
    #[serde(default)]
    pub transcript_coverage: Option<TranscriptCoverageDto>,
    #[serde(default)]
    pub source_transcripts: Vec<TranscriptDto>,
    pub recording: Option<RecordingSessionDto>,
    pub audio: Option<AudioArtifactDto>,
    #[serde(default)]
    pub audio_sources: Vec<AudioArtifactDto>,
    pub active_tab: Option<String>,
    pub last_error: Option<String>,
    /// Recordings queued behind the one currently processing for this note
    /// (0 when nothing extra is waiting). Populated from the in-memory
    /// processing queue at the command layer, not persisted.
    #[serde(default)]
    pub queued_recordings: i64,
    /// Exact recording session selected by the durable retry policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_recording_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NoteProcessingStage {
    Transcribing,
    Generating,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscardedTranscriptSpanDto {
    pub session_id: String,
    pub source: RecordingSource,
    pub start_ms: i64,
    pub end_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteProcessingProgressDto {
    pub note_id: String,
    pub recording_session_id: String,
    pub stage: NoteProcessingStage,
    pub processing_status: ProcessingStatus,
    /// The note row's `updated_at` value after this transition.
    pub revision: String,
    #[serde(default)]
    pub discarded_spans: Vec<DiscardedTranscriptSpanDto>,
}

/// The note-row fields returned by the single-statement `update_note` path.
///
/// Existing callers can keep requesting the fully hydrated [`NoteDto`].
/// Renderer autosave opts into this smaller response so an ordinary edit does
/// not re-read transcripts, folders, and audio after every persisted patch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NotePatchDto {
    pub id: String,
    pub title: String,
    pub preview: String,
    pub edited_content: Option<String>,
    pub active_tab: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum UpdateNoteResponse {
    Note(Box<NoteDto>),
    Patch(NotePatchDto),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteCalendarEventDto {
    pub event_id: String,
    pub title: String,
    pub start_at: String,
    pub end_at: String,
    pub account_email: String,
    /// Google's own event URL (htmlLink), persisted so the frontend's
    /// open-in-Google-Calendar link doesn't depend on reconstructing Google's
    /// undocumented eid scheme. Absent on events matched before this field
    /// shipped; the frontend then falls back to the constructed link.
    #[serde(default)]
    pub html_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptCoverageDto {
    pub detected_speech_ms: i64,
    pub transcribed_ms: i64,
    pub warning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateNoteRequest {
    pub folder_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListNotesRequest {
    pub folder_id: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetNoteRequest {
    pub note_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadNoteAudioRequest {
    pub note_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DownloadNoteAudioResponse {
    pub path: String,
    pub file_name: String,
    pub source_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteNoteRequest {
    pub note_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteNotesRequest {
    pub note_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateNoteRequest {
    pub note_id: String,
    pub title: Option<String>,
    pub edited_content: Option<String>,
    pub active_tab: Option<String>,
    /// Additive opt-in for the single-query patch response. Absent/false keeps
    /// the original fully hydrated NoteDto response for existing callers.
    #[serde(default)]
    pub patch_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFolderRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportClaudeProjectsRequest {
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFolderRequest {
    pub folder_id: String,
    pub delete_notes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameFolderRequest {
    pub folder_id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignNoteToFolderRequest {
    pub note_id: String,
    pub folder_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveNoteFromFolderRequest {
    pub note_id: String,
    pub folder_id: String,
}

// Records which project (folder) a Clovy-owned agent session is filed under.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionFolderDto {
    pub session_id: String,
    pub folder_id: String,
}

// Records which Clovy profile a session was created under.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionProfileDto {
    pub session_id: String,
    pub profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDataSummaryDto {
    pub notes: u32,
    pub dictation: u32,
    pub folders: u32,
    pub sessions: u32,
    pub memories: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignSessionToFolderRequest {
    pub session_id: String,
    pub folder_id: String,
}

// Clovy records completion locally, keyed by the stored session id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletedSessionDto {
    pub session_id: String,
    pub completed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSessionCompletedRequest {
    pub session_id: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignSessionToProfileRequest {
    pub session_id: String,
    pub profile: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveSessionFromFolderRequest {
    pub session_id: String,
    pub folder_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryEntryDto {
    pub id: String,
    pub phrase: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationHistoryItemDto {
    pub app_bundle_id: Option<String>,
    pub id: String,
    pub text: String,
    pub language: Option<String>,
    pub provider: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDictationHistoryResponse {
    pub items: Vec<DictationHistoryItemDto>,
    pub retention_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateDictionaryEntryRequest {
    pub phrase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateDictionaryEntryRequest {
    pub entry_id: String,
    pub phrase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDictionaryEntryRequest {
    pub entry_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRecordingRequest {
    pub note_id: String,
    #[serde(default)]
    pub source_mode: Option<RecordingSourceMode>,
    /// Pinned by the host for agent requests; interactive callers may omit it.
    #[serde(default)]
    pub expected_profile: Option<String>,
    #[serde(default)]
    pub agent_request_id: Option<String>,
    #[serde(default)]
    pub video: Option<RecordingVideoRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMeetingRecordingRequest {
    pub request_id: String,
    #[serde(default)]
    pub source_mode: Option<RecordingSourceMode>,
    #[serde(default)]
    pub video: Option<RecordingVideoRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingVideoRequest {
    pub window_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingWindowDto {
    pub window_id: u32,
    pub app_name: String,
    pub bundle_id: String,
    pub title: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingOrigin {
    Other,
    MeetingPrompt,
}

impl RecordingOrigin {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Other => "other",
            Self::MeetingPrompt => "meeting_prompt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingOriginMetadata {
    pub origin: RecordingOrigin,
    pub meeting_app_bundle_families: Vec<String>,
    pub auto_finish_eligible: bool,
}

impl Default for RecordingOriginMetadata {
    fn default() -> Self {
        Self {
            origin: RecordingOrigin::Other,
            meeting_app_bundle_families: Vec::new(),
            auto_finish_eligible: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRequest {
    #[serde(default)]
    pub expected_profile: Option<String>,
    #[serde(default)]
    pub agent_request_id: Option<String>,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishRecordingResponse {
    pub note: NoteDto,
    pub recording: RecordingSessionDto,
    pub validation: AudioValidationDto,
    #[serde(default)]
    pub validations: Vec<SourceValidationDto>,
    pub processing_started: bool,
    #[serde(default)]
    pub warnings: Vec<SourceWarningDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryProcessingRequest {
    #[serde(default)]
    pub expected_profile: Option<String>,
    #[serde(default)]
    pub agent_request_id: Option<String>,
    pub note_id: String,
    pub step: Option<String>,
    #[serde(default)]
    pub recording_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverRecordingRequest {
    #[serde(default)]
    pub expected_profile: Option<String>,
    #[serde(default)]
    pub agent_request_id: Option<String>,
    pub session_id: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophonePermissionResponse {
    pub state: String,
    pub recovery_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckRecordingSourceReadinessRequest {
    pub source_mode: RecordingSourceMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSourceReadinessDto {
    pub source_mode: RecordingSourceMode,
    pub ready: bool,
    pub checked_at: String,
    pub sources: Vec<SourceReadinessDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceReadinessDto {
    pub source: RecordingSource,
    pub required: bool,
    pub ready: bool,
    pub permission_state: String,
    pub device_available: bool,
    pub capture_available: bool,
    pub recovery_action: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenPrivacySettingsRequest {
    pub pane: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptDto {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recording_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span_id: Option<String>,
    pub text: String,
    pub source_mode: Option<RecordingSourceMode>,
    pub source: Option<String>,
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub turn_index: Option<i64>,
    pub language: Option<String>,
    pub status: String,
    pub last_error: Option<String>,
    #[serde(default)]
    pub recorded_silence: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSessionDto {
    pub id: String,
    pub note_id: String,
    pub source_mode: RecordingSourceMode,
    pub state: RecordingState,
    pub started_at: String,
    pub elapsed_ms: i64,
    pub device_label: Option<String>,
    pub level: AudioLevelDto,
    #[serde(default)]
    pub live_preview_enabled: bool,
    #[serde(default)]
    pub sources: Vec<SourceStatusDto>,
    #[serde(default)]
    pub warnings: Vec<SourceWarningDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStatusDto {
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    pub source_mode: RecordingSourceMode,
    pub state: RecordingState,
    pub elapsed_ms: i64,
    pub level: AudioLevelDto,
    pub silence_warning: bool,
    pub bytes_written: i64,
    #[serde(default)]
    pub live_preview_enabled: bool,
    #[serde(default)]
    pub sources: Vec<SourceStatusDto>,
    #[serde(default)]
    pub warnings: Vec<SourceWarningDto>,
}

/// High-frequency recording data shared by the main renderer and meeting HUD.
/// Stable session metadata, byte counters, and artifact details stay on the
/// command DTOs so this event remains cheap to serialize and deliver.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingTelemetryDto {
    pub session_id: String,
    pub state: RecordingState,
    pub elapsed_ms: i64,
    pub level: AudioLevelDto,
    pub silence_warning: bool,
    pub sources: Vec<RecordingSourceTelemetryDto>,
    pub warnings: Vec<SourceWarningDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingSourceTelemetryDto {
    pub source: RecordingSource,
    pub state: SourceState,
    pub elapsed_ms: i64,
    pub level: AudioLevelDto,
    pub silence_warning: bool,
}

impl From<&RecordingStatusDto> for RecordingTelemetryDto {
    fn from(status: &RecordingStatusDto) -> Self {
        Self {
            session_id: status.session_id.clone(),
            state: status.state,
            elapsed_ms: status.elapsed_ms,
            level: status.level.clone(),
            silence_warning: status.silence_warning,
            sources: status
                .sources
                .iter()
                .map(|source| RecordingSourceTelemetryDto {
                    source: source.source,
                    state: source.state,
                    elapsed_ms: source.elapsed_ms,
                    level: source.level.clone(),
                    silence_warning: source.silence_warning,
                })
                .collect(),
            warnings: status.warnings.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioArtifactDto {
    pub id: String,
    pub source: String,
    pub format: String,
    pub duration_ms: i64,
    pub size_bytes: i64,
    pub checksum: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NoteTranscriptionJobKind {
    Turn,
    SourceFallback,
}

impl NoteTranscriptionJobKind {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Turn => "turn",
            Self::SourceFallback => "source_fallback",
        }
    }
}

impl From<&str> for NoteTranscriptionJobKind {
    fn from(value: &str) -> Self {
        match value {
            "source_fallback" | "sourceFallback" => Self::SourceFallback,
            _ => Self::Turn,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum NoteTranscriptionJobStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Superseded,
}

impl NoteTranscriptionJobStatus {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Superseded => "superseded",
        }
    }
}

impl From<&str> for NoteTranscriptionJobStatus {
    fn from(value: &str) -> Self {
        match value {
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "superseded" => Self::Superseded,
            _ => Self::Pending,
        }
    }
}

/// Complete, output-affecting plan for one durable saved-audio Source span.
/// `configuration_fingerprint` is supplied by processing and covers language,
/// dictionary, and other context revisions that repositories cannot derive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteTranscriptionJobPlan {
    pub span_id: String,
    pub audio_artifact_id: String,
    pub source: String,
    pub job_kind: NoteTranscriptionJobKind,
    pub start_ms: i64,
    pub end_ms: i64,
    pub turn_index: i64,
    pub provider: String,
    pub max_chunk_ms: Option<i64>,
    pub pipeline_version: String,
    pub configuration_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NoteTranscriptionJobRecord {
    pub id: String,
    pub note_id: String,
    pub recording_session_id: String,
    pub audio_artifact_id: String,
    pub source: String,
    pub source_mode: RecordingSourceMode,
    pub job_kind: NoteTranscriptionJobKind,
    pub start_ms: i64,
    pub end_ms: i64,
    pub turn_index: i64,
    pub input_fingerprint: String,
    pub configuration_fingerprint: String,
    pub operation_id: String,
    pub provider: String,
    pub max_chunk_ms: Option<i64>,
    pub pipeline_version: String,
    pub status: NoteTranscriptionJobStatus,
    pub attempt_count: i64,
    pub transcript_id: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatusDto {
    pub source: RecordingSource,
    pub state: SourceState,
    pub elapsed_ms: i64,
    pub bytes_written: i64,
    #[serde(default)]
    pub dropped_samples: u64,
    pub level: AudioLevelDto,
    pub silence_warning: bool,
    pub path_finalized: bool,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceValidationDto {
    pub source: RecordingSource,
    pub file_exists: bool,
    pub non_zero_size: bool,
    pub readable_audio: bool,
    pub expected_duration_ms: i64,
    pub actual_duration_ms: Option<i64>,
    pub duration_within_tolerance: bool,
    pub non_silent_signal: bool,
    pub recorded_silence: bool,
    pub peak_amplitude: Option<f32>,
    pub rms_amplitude: Option<f32>,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceWarningDto {
    pub source: RecordingSource,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioValidationDto {
    pub file_exists: bool,
    pub non_zero_size: bool,
    pub readable_audio: bool,
    pub expected_duration_ms: i64,
    pub actual_duration_ms: i64,
    pub duration_within_tolerance: bool,
    pub non_silent_signal: bool,
    #[serde(default)]
    pub recorded_silence: bool,
    pub peak_amplitude: f32,
    pub rms_amplitude: f32,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AudioLevelDto {
    pub peak: f32,
    pub rms: f32,
    pub recent_peaks: Vec<f32>,
}

impl Default for AudioLevelDto {
    fn default() -> Self {
        Self {
            peak: 0.0,
            rms: 0.0,
            recent_peaks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableRecordingDto {
    pub session_id: String,
    pub note_id: String,
    pub source_mode: RecordingSourceMode,
    pub started_at: String,
    pub partial_path_present: bool,
    pub final_path_present: bool,
    pub bytes_found: i64,
    #[serde(default)]
    pub sources: Vec<RecoverableSourceDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverableSourceDto {
    pub source: RecordingSource,
    pub partial_path_present: bool,
    pub final_path_present: bool,
    pub bytes_found: i64,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestAgentSessionTitleRequest {
    pub prompt: String,
    #[serde(default)]
    pub response: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestAgentSessionTitleResponse {
    pub title: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProcessingStatus {
    Draft,
    Recording,
    Validating,
    Transcribing,
    Generating,
    Ready,
    Failed,
    Recoverable,
}

impl ProcessingStatus {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Recording => "recording",
            Self::Validating => "validating",
            Self::Transcribing => "transcribing",
            Self::Generating => "generating",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Recoverable => "recoverable",
        }
    }
}

impl From<&str> for ProcessingStatus {
    fn from(value: &str) -> Self {
        match value {
            "recording" => Self::Recording,
            "validating" => Self::Validating,
            "transcribing" => Self::Transcribing,
            "generating" => Self::Generating,
            "ready" => Self::Ready,
            "failed" => Self::Failed,
            "recoverable" => Self::Recoverable,
            _ => Self::Draft,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecordingSourceMode {
    #[default]
    MicrophoneOnly,
    MicrophonePlusSystem,
}

impl RecordingSourceMode {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::MicrophoneOnly => "microphone_only",
            Self::MicrophonePlusSystem => "microphone_plus_system",
        }
    }

    pub fn required_sources(self) -> Vec<RecordingSource> {
        match self {
            Self::MicrophoneOnly => vec![RecordingSource::Microphone],
            Self::MicrophonePlusSystem => {
                vec![RecordingSource::Microphone, RecordingSource::System]
            }
        }
    }
}

impl From<&str> for RecordingSourceMode {
    fn from(value: &str) -> Self {
        match value {
            "microphone_plus_system" | "microphonePlusSystem" => Self::MicrophonePlusSystem,
            _ => Self::MicrophoneOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecordingSource {
    Microphone,
    System,
}

impl RecordingSource {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Microphone => "microphone",
            Self::System => "system",
        }
    }
}

impl From<&str> for RecordingSource {
    fn from(value: &str) -> Self {
        match value {
            "system" => Self::System,
            _ => Self::Microphone,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecordingState {
    Idle,
    PermissionDenied,
    Starting,
    Recording,
    Paused,
    Finalizing,
    Validating,
    PartiallyValid,
    Invalid,
    Ready,
    Failed,
    Recoverable,
}

impl RecordingState {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::PermissionDenied => "permission_denied",
            Self::Starting => "starting",
            Self::Recording => "recording",
            Self::Paused => "paused",
            Self::Finalizing => "finalizing",
            Self::Validating => "validating",
            Self::PartiallyValid => "partially_valid",
            Self::Invalid => "invalid",
            Self::Ready => "valid",
            Self::Failed => "failed",
            Self::Recoverable => "recoverable",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SourceState {
    Pending,
    PermissionDenied,
    Unavailable,
    Starting,
    Recording,
    Paused,
    Finalizing,
    Finalized,
    Valid,
    Invalid,
    Recoverable,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::MemoryDto;

    fn memory(folder_id: Option<&str>) -> MemoryDto {
        MemoryDto {
            id: "memory-1".to_string(),
            folder_id: folder_id.map(str::to_string),
            content: "Remember this".to_string(),
            source: "user".to_string(),
            created_at: "2026-07-14T00:00:00Z".to_string(),
            updated_at: "2026-07-14T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn memory_folder_id_is_omitted_for_global_memory_and_present_for_scoped_memory() {
        let global = serde_json::to_value(memory(None)).expect("serialize global memory");
        assert!(global.get("folderId").is_none());

        let scoped =
            serde_json::to_value(memory(Some("folder-1"))).expect("serialize folder-scoped memory");
        assert_eq!(scoped["folderId"], "folder-1");
    }
}
