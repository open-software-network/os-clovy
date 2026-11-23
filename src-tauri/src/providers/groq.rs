//! Transcription and dictation audio sent directly to the user's Groq account.

use super::{direct::*, BuiltinProvider};
use crate::domain::types::AppError;
use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct TranscriptionRequest {
    pub provider: String,
    pub audio_path: PathBuf,
    pub title: String,
    /// Forwarded verbatim as Whisper's `prompt`: only the dictionary hint of
    /// `TranscriptionDictionary`, never previous transcript or instructions.
    pub context: Option<String>,
    pub language: Option<String>,
    pub operation_id: Option<String>,
    pub preview: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionProviderResult {
    pub text: String,
    pub language: Option<String>,
    pub provider: String,
}

#[derive(Debug, Clone)]
pub struct DictateTranscribeRequest {
    pub audio_path: PathBuf,
    /// Forwarded verbatim as Whisper's `prompt`, like `TranscriptionRequest`.
    pub context: Option<String>,
    pub language: Option<String>,
    pub session_id: String,
    pub utterance_id: String,
}

/// A stretch of a timed transcription, in milliseconds from the audio's start.
#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptSegment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

/// A transcription with its segment times: a pack of several speech turns
/// sent as one request is split back into its turns by these times.
#[derive(Debug, Clone)]
pub struct TimedTranscription {
    pub language: Option<String>,
    pub segments: Vec<TranscriptSegment>,
}

/// Missing-key guidance, byte-identical to `notes.failure.groqKeyMissing`.
pub(crate) const GROQ_API_KEY_MISSING_MESSAGE: &str =
    "Add your Groq API key in Settings > Models > More options > Provider API keys to transcribe recordings.";

/// Kept byte-identical to `notes.failure.audioTooLarge`.
pub(crate) const AUDIO_TOO_LARGE_MESSAGE: &str =
    "This recording part is larger than the 25 MB Groq accepts in one request.";

/// Groq's 25 MB upload cap.
pub(super) const GROQ_MAX_AUDIO_BYTES: usize = 26_214_400;

/// Segments at or above this `no_speech_prob` are Whisper's guesses over
/// silence (its classic "Thank you."), not words the user said.
pub(super) const GROQ_NO_SPEECH_PROB_LIMIT: f64 = 0.6;

#[derive(Deserialize)]
pub(super) struct GroqTranscription {
    text: String,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    segments: Option<Vec<GroqSegment>>,
}

#[derive(Deserialize)]
pub(super) struct GroqSegment {
    text: String,
    #[serde(default)]
    no_speech_prob: Option<f64>,
    /// Seconds from the audio's start (`verbose_json`).
    #[serde(default)]
    start: Option<f64>,
    #[serde(default)]
    end: Option<f64>,
}

/// Final turns, full-source transcription and live preview use the user's Groq key.
pub async fn transcribe_saved_audio(
    request: TranscriptionRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    let groq_key = crate::providers::provider_api_key(BuiltinProvider::Groq)?;
    transcribe_saved_audio_via(
        &production_notes_transport(),
        groq_key.as_deref(),
        crate::providers::transcription_model(),
        request,
    )
    .await
}

/// Note transcription with a Groq key and model the caller already resolved
/// for this request, so the request goes with the same key its pacer chose.
pub(crate) async fn transcribe_saved_audio_with(
    groq_key: Option<&str>,
    model: String,
    request: TranscriptionRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    transcribe_saved_audio_via(&production_notes_transport(), groq_key, model, request).await
}

/// The timed (turn pack) request with an already resolved Groq key and model.
#[cfg_attr(test, allow(dead_code))]
pub(crate) async fn transcribe_saved_audio_timed_with(
    groq_key: &str,
    model: String,
    request: TranscriptionRequest,
) -> Result<TimedTranscription, AppError> {
    transcribe_saved_audio_timed_via(&production_notes_transport(), groq_key, model, request).await
}

pub(crate) async fn transcribe_saved_audio_via(
    transport: &DirectTransport<'_>,
    groq_key: Option<&str>,
    model: String,
    request: TranscriptionRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    let audio = read_audio(&request.audio_path).await?;
    let context = request
        .context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let language = normalized_language(request.language.as_deref()).map(str::to_string);
    let groq_key = require_groq_key(groq_key)?;
    let body = send_groq_transcription(
        transport,
        groq_key,
        &model,
        &request.audio_path,
        audio,
        context,
        language,
        false,
    )
    .await?;
    let text = body.text.trim().to_string();
    if text.is_empty() {
        // Silence cannot produce a note transcript.
        return Err(AppError::new("no_speech", "no_speech"));
    }
    Ok(TranscriptionProviderResult {
        text,
        language: body.language,
        provider: BuiltinProvider::Groq.id().to_string(),
    })
}

/// One `/audio/transcriptions` request to Groq with the user's key. `timed`
/// asks for `verbose_json` with segment timestamps instead of plain `json`.
#[allow(clippy::too_many_arguments)]
pub(super) async fn send_groq_transcription(
    transport: &DirectTransport<'_>,
    groq_key: &str,
    model: &str,
    audio_path: &Path,
    audio: Vec<u8>,
    context: Option<String>,
    language: Option<String>,
    timed: bool,
) -> Result<GroqTranscription, AppError> {
    if audio.len() > GROQ_MAX_AUDIO_BYTES {
        return Err(AppError::new("audio_too_large", AUDIO_TOO_LARGE_MESSAGE));
    }
    let filename = canonical_audio_filename(audio_path);
    let url = format!("{}/audio/transcriptions", transport.groq_base);
    let response = send_direct(BuiltinProvider::Groq, "this transcription", || {
        let mut form = Form::new()
            .text("model", model.to_string())
            .text(
                "response_format",
                if timed { "verbose_json" } else { "json" },
            )
            .part("file", audio_part(audio.clone(), filename, audio_path)?);
        if timed {
            form = form.text("timestamp_granularities[]", "segment");
        }
        if let Some(context) = context.clone() {
            form = form.text("prompt", context);
        }
        if let Some(language) = language.clone() {
            form = form.text("language", language);
        }
        Ok(transport
            .direct_client
            .post(&url)
            .bearer_auth(groq_key)
            .multipart(form))
    })
    .await?;
    response
        .json()
        .await
        .map_err(|_| AppError::new("provider_response_invalid", INVALID_DIRECT_RESPONSE))
}

/// Note transcription with segment times, straight to Groq with the user's
/// key. Without the key there is no timed route: the caller sends one turn
/// per request instead.
pub async fn transcribe_saved_audio_timed(
    request: TranscriptionRequest,
) -> Result<TimedTranscription, AppError> {
    let Some(groq_key) = crate::providers::provider_api_key(BuiltinProvider::Groq)? else {
        return Err(AppError::new(
            "groq_api_key_missing",
            GROQ_API_KEY_MISSING_MESSAGE,
        ));
    };
    transcribe_saved_audio_timed_via(
        &production_notes_transport(),
        &groq_key,
        crate::providers::transcription_model(),
        request,
    )
    .await
}

pub(crate) async fn transcribe_saved_audio_timed_via(
    transport: &DirectTransport<'_>,
    groq_key: &str,
    model: String,
    request: TranscriptionRequest,
) -> Result<TimedTranscription, AppError> {
    let audio = read_audio(&request.audio_path).await?;
    let context = request
        .context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let language = normalized_language(request.language.as_deref()).map(str::to_string);
    let body = send_groq_transcription(
        transport,
        groq_key,
        &model,
        &request.audio_path,
        audio,
        context,
        language,
        true,
    )
    .await?;
    let segments = body
        .segments
        .ok_or_else(|| AppError::new("provider_response_invalid", INVALID_DIRECT_RESPONSE))?
        .into_iter()
        .filter(|segment| {
            !segment
                .no_speech_prob
                .is_some_and(|probability| probability >= GROQ_NO_SPEECH_PROB_LIMIT)
        })
        .filter_map(|segment| {
            let text = segment.text.trim().to_string();
            (!text.is_empty()).then(|| TranscriptSegment {
                start_ms: seconds_to_ms(segment.start),
                end_ms: seconds_to_ms(segment.end),
                text,
            })
        })
        .collect();
    Ok(TimedTranscription {
        language: body.language,
        segments,
    })
}

pub(super) fn seconds_to_ms(seconds: Option<f64>) -> i64 {
    seconds
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map_or(0, |seconds| (seconds * 1_000.0).round() as i64)
}

/// Canonical upload names avoid exposing local recording filenames.
pub(super) fn canonical_audio_filename(path: &Path) -> &'static str {
    match audio_mime(path) {
        "audio/mp4" => "audio.m4a",
        _ => "audio.wav",
    }
}

/// A missing key fails before capture admission or any provider request.
pub(crate) fn require_groq_key(key: Option<&str>) -> Result<&str, AppError> {
    key.filter(|key| !key.trim().is_empty())
        .ok_or_else(|| AppError::new("groq_api_key_missing", GROQ_API_KEY_MISSING_MESSAGE))
}

pub async fn dictate_transcribe(
    request: DictateTranscribeRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    let groq_key = dictation_user_key(BuiltinProvider::Groq)?;
    dictate_transcribe_with(
        groq_key.as_deref(),
        crate::providers::transcription_model(),
        request,
    )
    .await
}

pub(crate) async fn dictate_transcribe_with(
    groq_key: Option<&str>,
    model: String,
    request: DictateTranscribeRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    let result =
        dictate_transcribe_via(&production_dictation_transport(), groq_key, model, request).await;
    mark_dictation_route_used();
    result
}

pub(crate) async fn dictate_transcribe_via(
    transport: &DirectTransport<'_>,
    groq_key: Option<&str>,
    model: String,
    request: DictateTranscribeRequest,
) -> Result<TranscriptionProviderResult, AppError> {
    let groq_key = require_groq_key(groq_key)?;
    let audio = read_audio(&request.audio_path).await?;
    if audio.len() > GROQ_MAX_AUDIO_BYTES {
        return Err(AppError::new(
            "audio_too_large",
            "Audio exceeds the 25 MB transcription limit.",
        ));
    }
    let filename = canonical_audio_filename(&request.audio_path);
    let audio = audio_part(audio, filename, &request.audio_path)?;
    let context = request
        .context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let language = normalized_language(request.language.as_deref()).map(str::to_string);
    // Verbose segments let dictation discard Whisper's no-speech guesses.
    let mut form = Form::new()
        .text("model", model)
        .text("response_format", "verbose_json")
        .part("file", audio);
    if let Some(context) = context {
        form = form.text("prompt", context);
    }
    if let Some(language) = language {
        form = form.text("language", language);
    }
    let response = transport
        .direct_client
        .post(format!("{}/audio/transcriptions", transport.groq_base))
        .bearer_auth(groq_key)
        .multipart(form)
        .send()
        .await
        .map_err(network_error)?;
    let status = response.status();
    if !status.is_success() {
        tracing::warn!(status = status.as_u16(), "groq transcription failed");
        return Err(direct_provider_error(BuiltinProvider::Groq, status));
    }
    let body: GroqTranscription = response
        .json()
        .await
        .map_err(|_| AppError::new("provider_response_invalid", INVALID_DIRECT_RESPONSE))?;
    Ok(TranscriptionProviderResult {
        text: groq_speech_text(&body),
        language: body.language,
        provider: BuiltinProvider::Groq.id().to_string(),
    })
}

/// The transcript without no-speech segments; the plain text when Groq sent
/// no segments. Empty when every segment was no speech.
pub(super) fn groq_speech_text(body: &GroqTranscription) -> String {
    let Some(segments) = body.segments.as_ref() else {
        return body.text.trim().to_string();
    };
    segments
        .iter()
        .filter(|segment| {
            !segment
                .no_speech_prob
                .is_some_and(|probability| probability >= GROQ_NO_SPEECH_PROB_LIMIT)
        })
        .map(|segment| segment.text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) const DICTATION_WARM_TIMEOUT: Duration = Duration::from_secs(10);

/// reqwest drops pooled connections after 90 s idle; rewarm well before.
pub(super) const DICTATION_REWARM_AFTER_IDLE: Duration = Duration::from_secs(60);

/// Opens the connection the next take will use, without sending audio: a free
/// model listing on Groq when the user's key exists.
pub(crate) async fn warm_dictation_via(
    transport: &DirectTransport<'_>,
    groq_key: Option<&str>,
) -> Result<(), AppError> {
    let Some(key) = groq_key else {
        return Ok(());
    };
    let request = transport
        .direct_client
        .get(format!("{}/models", transport.groq_base))
        .bearer_auth(key);
    request
        .timeout(DICTATION_WARM_TIMEOUT)
        .send()
        .await
        .map_err(network_error)?;
    Ok(())
}

/// Warms dictation's route when it has been idle long enough for the pooled
/// connection to be gone: at launch, and before a press after a quiet minute.
/// Also reads the user's Groq key into the credential-store cache.
pub fn warm_dictation_route_if_idle() {
    let now = std::time::Instant::now();
    {
        let Ok(mut last) = dictation_route_last_used().lock() else {
            return;
        };
        if !dictation_route_needs_warm(*last, now) {
            return;
        }
        // Claimed now so concurrent presses do not warm twice.
        *last = Some(now);
    }
    tauri::async_runtime::spawn(async {
        let transport = production_dictation_transport();
        let groq_key = dictation_user_key(BuiltinProvider::Groq).ok().flatten();
        if let Err(error) = warm_dictation_via(&transport, groq_key.as_deref()).await {
            tracing::debug!(code = %error.code, "dictation route warm-up failed");
        }
    });
}

pub(super) fn dictation_route_needs_warm(
    last_used: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    match last_used {
        Some(last) => now.saturating_duration_since(last) > DICTATION_REWARM_AFTER_IDLE,
        None => true,
    }
}

pub(super) fn dictation_route_last_used() -> &'static Mutex<Option<std::time::Instant>> {
    static LAST_USED: OnceLock<Mutex<Option<std::time::Instant>>> = OnceLock::new();
    LAST_USED.get_or_init(|| Mutex::new(None))
}

pub(super) fn mark_dictation_route_used() {
    if let Ok(mut last) = dictation_route_last_used().lock() {
        *last = Some(std::time::Instant::now());
    }
}

pub(super) fn normalized_language(language: Option<&str>) -> Option<&str> {
    language.map(str::trim).filter(|value| {
        value.len() == 2
            && value
                .chars()
                .all(|character| character.is_ascii_lowercase())
    })
}

pub(super) async fn read_audio(path: &Path) -> Result<Vec<u8>, AppError> {
    if !path.exists() {
        return Err(AppError::new(
            "audio_artifact_missing",
            "Saved audio is missing and cannot be transcribed.",
        ));
    }
    tokio::fs::read(path)
        .await
        .map_err(|error| AppError::new("audio_artifact_missing", error.to_string()))
}

pub(super) fn audio_part(audio: Vec<u8>, filename: &str, path: &Path) -> Result<Part, AppError> {
    Part::bytes(audio)
        .file_name(filename.to_string())
        .mime_str(audio_mime(path))
        .map_err(|error| AppError::new(REQUEST_FAILED_CODE, error.to_string()))
}

pub(super) fn audio_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("m4a") | Some("mp4") => "audio/mp4",
        _ => "audio/wav",
    }
}

impl TranscriptionRequest {
    pub fn operation_id(&self) -> String {
        self.operation_id.clone().unwrap_or_else(|| {
            self.audio_path
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("recording")
                .to_string()
        })
    }
}
