#[cfg(target_os = "windows")]
use crate::audio::turns::normalize_wav_for_transcription;
use crate::domain::{
    processing::{build_transcription_dictionary, TranscriptionDictionary},
    types::{AppError, ListDictationHistoryResponse},
};
use crate::providers::PROVIDER_GROQ;
use crate::providers::{
    groq::{dictate_transcribe_with, DictateTranscribeRequest, TranscriptionProviderResult},
    openrouter::{dictate_cleanup, DictateCleanupRequestParams, DEFAULT_DICTATION_CLEANUP_MODEL},
};
use chrono::Utc;
use futures_util::{StreamExt, TryStreamExt};
use serde::{Deserialize, Deserializer, Serialize};
use std::{
    fs,
    future::Future,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, ChildStdin, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};

/// Cleanup regenerates the whole transcript through the LLM, so its latency
/// grows with dictation length. A flat budget silently degraded long
/// dictations: the call timed out, cleanup was skipped, and the raw ASR text
/// (capitalized but unpunctuated) was pasted (JUN-212). Scale the budget with
/// the transcript instead: the base covers short dictations plus network
/// overhead, each byte of transcript buys generation time, and the cap keeps
/// a hung request from wedging finalization for minutes.
const DICTATION_CLEANUP_BASE_TIMEOUT_MS: u64 = 15_000;
/// ~5 ms per input byte ≈ 20 ms per regenerated token (≈4 bytes/token), a
/// conservative floor of ~50 tokens/s for the cleanup model.
const DICTATION_CLEANUP_TIMEOUT_MS_PER_BYTE: u64 = 5;
const DICTATION_CLEANUP_MAX_TIMEOUT_MS: u64 = 60_000;
/// Bound each optional cleanup request to 30 seconds. The overall budget
/// above can span multiple sequential chunks; a failed chunk preserves its
/// original transcript so cleanup cannot hold the paste indefinitely.
const DICTATION_CLEANUP_REQUEST_MAX_TIMEOUT_MS: u64 = 30_000;
/// Above this size, cleanup runs chunked. Measured against the production
/// prompt (JUN-212): the cleanup model punctuates ~800-byte passages of
/// filler-heavy run-on speech reliably, while at 2 KB and beyond it degrades
/// into echoing the transcript back with no punctuation at all.
const DICTATION_CLEANUP_CHUNK_TARGET_BYTES: usize = 800;
/// Takes with less audio than this never reach Whisper (design D5).
const DICTATION_MIN_UPLOAD_SECONDS: f32 = 0.5;
/// Chunks of one long transcript cleaned at the same time (design D7).
const DICTATION_CLEANUP_PARALLEL_CHUNKS: usize = 4;
/// App-context slug sent with dictation cleanup when the paste target is a
/// known kind of app, so the cleaned text is laid out for that surface.
/// Email is the only recognized context today.
const APP_CONTEXT_EMAIL: &str = "email";
/// Native email clients by bundle id. Browser webmail (Gmail, Outlook web)
/// needs focused-tab detection and is a deliberate follow-up.
const EMAIL_APP_BUNDLE_IDS: &[&str] = &[
    "com.apple.mail",
    "com.microsoft.Outlook",
    "com.readdle.SparkDesktop",
    "com.readdle.smartemail-Mac",
    "it.bloop.airmail2",
    "com.mimestream.Mimestream",
    "com.superhuman.electron",
];
const DICTATION_AUDIO_ACTIVITY_THRESHOLD: f32 = 0.04;
/// Sound Analysis reports a confidence for its trained `speech` class. When
/// both this score and the helper's independent peak/RMS meter stay low, Clovy
/// withholds automatic paste but keeps the transcript recoverable. This is not
/// a destructive silence test: quiet speech can also fall below both cutoffs.
const DICTATION_SPEECH_CONFIDENCE_THRESHOLD: f32 = 0.4;
const DICTATION_EVENT_LOG: &str = "dictation-events.log";
#[cfg(target_os = "macos")]
static DICTATION_TOOLTIP_PANEL: OnceLock<usize> = OnceLock::new();
/// AppKit cannot resolve the webview's CSS custom properties. These native
/// boundary values mirror the named Dictation HUD tokens in
/// `src/styles/tokens.css` and the forced-dark colors in
/// `src/styles/hud.css`; keep the pair together when those tokens change.
#[cfg(target_os = "macos")]
/// Mirrors `--control-lg`.
const DICTATION_TOOLTIP_WIDTH_LOGICAL: f64 = 32.0;
#[cfg(target_os = "macos")]
/// Mirrors `--control-hud`.
const DICTATION_TOOLTIP_HEIGHT_LOGICAL: f64 = 24.0;
#[cfg(target_os = "macos")]
/// Mirrors `--sp-2`.
const DICTATION_TOOLTIP_GAP_LOGICAL: f64 = 6.0;
#[cfg(target_os = "macos")]
/// Mirrors `--border-width`.
const DICTATION_TOOLTIP_BORDER_WIDTH_LOGICAL: f64 = 1.0;
#[cfg(target_os = "macos")]
/// Mirrors `--r-sm`.
const DICTATION_TOOLTIP_CORNER_RADIUS_LOGICAL: f64 = 6.0;
#[cfg(target_os = "macos")]
/// Mirrors `--fs-xs`. Separate HUD webviews intentionally keep font scale 1.
const DICTATION_TOOLTIP_FONT_SIZE_LOGICAL: f64 = 11.0;
#[cfg(target_os = "macos")]
/// AppKit's native equivalent of `--fw-medium` (600).
const DICTATION_TOOLTIP_FONT_WEIGHT_MEDIUM: f64 = 0.23;
#[cfg(target_os = "macos")]
/// sRGB conversion of the forced-dark `--hud-bg`.
const DICTATION_TOOLTIP_BACKGROUND_RED: f64 = 0.083;
#[cfg(target_os = "macos")]
const DICTATION_TOOLTIP_BACKGROUND_GREEN: f64 = 0.082;
#[cfg(target_os = "macos")]
const DICTATION_TOOLTIP_BACKGROUND_BLUE: f64 = 0.079;
#[cfg(target_os = "macos")]
const DICTATION_TOOLTIP_BACKGROUND_ALPHA: f64 = 0.92;
#[cfg(target_os = "macos")]
/// `--hud-border` resolves to a white alpha on the forced-dark HUD.
const DICTATION_TOOLTIP_BORDER_ALPHA: f64 = 0.12;
#[cfg(target_os = "macos")]
/// sRGB conversion of the forced-dark `--muted-foreground`.
const DICTATION_TOOLTIP_TEXT_RED: f64 = 0.633;
#[cfg(target_os = "macos")]
const DICTATION_TOOLTIP_TEXT_GREEN: f64 = 0.631;
#[cfg(target_os = "macos")]
const DICTATION_TOOLTIP_TEXT_BLUE: f64 = 0.627;
// NSTextField has no tokenized vertical-centering primitive. These two
// one-off AppKit baseline metrics center the token-sized label in the panel.
#[cfg(target_os = "macos")]
const DICTATION_TOOLTIP_TEXT_Y_LOGICAL: f64 = 3.0;
#[cfg(target_os = "macos")]
const DICTATION_TOOLTIP_TEXT_HEIGHT_LOGICAL: f64 = 17.0;

static SETTINGS_CACHE: OnceLock<Mutex<DictationSettings>> = OnceLock::new();

/// True from the moment a dictation take starts until it ends, HOWEVER it ends.
/// Mirrors the frontend's `nextDictationWorkflowActive` (see
/// `indicator_action_for_event`) so the native menu-bar indicator stays correct
/// with no webview attached. Read only via the compare-and-swap in
/// `sync_dictation_indicator`.
static DICTATION_ACTIVE: AtomicBool = AtomicBool::new(false);

pub struct HelperProcess {
    child: Child,
    stdin: ChildStdin,
}

struct ComposerDeliveryRegistration {
    request_id: String,
    window_handle: isize,
}

pub struct HelperState {
    agent_take: Mutex<Option<AgentDictationTake>>,
    agent_changed: tokio::sync::Notify,
    process: Mutex<Option<HelperProcess>>,
    /// One composer request registered while the main-window editor owns
    /// focus. A matching helper-confirmed recording start consumes it.
    composer_registration: Mutex<Option<ComposerDeliveryRegistration>>,
    /// Set only on intentional teardown (app quit / [`stop_helper`]) so the
    /// supervisor can tell a deliberate stop from a crash and skip respawning.
    shutting_down: AtomicBool,
    /// Consecutive rapid respawn attempts, used to cap a crash loop. Reset once
    /// a respawned helper survives long enough to be considered healthy.
    respawn_failures: AtomicU32,
}

impl Default for HelperState {
    fn default() -> Self {
        Self {
            agent_take: Mutex::new(None),
            agent_changed: tokio::sync::Notify::new(),
            process: Mutex::new(None),
            composer_registration: Mutex::new(None),
            shutting_down: AtomicBool::new(false),
            respawn_failures: AtomicU32::new(0),
        }
    }
}

/// A host-owned dictation destination. No arbitrary app or window target is accepted.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentDictationTake {
    pub take_id: String,
    pub composer_request_id: String,
    pub conversation_id: String,
    pub profile: String,
    pub window: String,
    pub phase: String,
    pub error: Option<String>,
    pub error_code: Option<String>,
    #[serde(skip)]
    pub binding: Option<crate::mcp_server::turns::TurnBinding>,
}

#[derive(Clone, Copy)]
pub(crate) enum AgentDictationAction {
    Start,
    Stop,
    Cancel,
}

pub(crate) fn agent_dictation_target(profile: &str, conversation_id: &str) -> AgentDictationTake {
    AgentDictationTake {
        take_id: uuid::Uuid::new_v4().to_string(),
        composer_request_id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.into(),
        profile: profile.into(),
        window: "main".into(),
        phase: "starting".into(),
        error: None,
        error_code: None,
        binding: None,
    }
}

pub(crate) fn agent_dictation_status(
    app: &AppHandle,
    profile: &str,
    conversation_id: &str,
) -> Result<Option<AgentDictationTake>, AppError> {
    let state = app.state::<HelperState>();
    let take = state
        .agent_take
        .lock()
        .map_err(|_| AppError::new("dictation_unavailable", "Dictation state is unavailable."))?;
    Ok(take
        .as_ref()
        .filter(|take| take.profile == profile && take.conversation_id == conversation_id)
        .cloned())
}

/// Cancelling a caller while waiting also revokes the take's delivery authority.
struct AgentDictationOperationLease {
    app: AppHandle,
    take_id: String,
    complete: bool,
}
impl Drop for AgentDictationOperationLease {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        if self
            .app
            .state::<HelperState>()
            .agent_take
            .lock()
            .ok()
            .is_some_and(|active| {
                active.as_ref().is_some_and(|take| {
                    take.take_id == self.take_id && take.phase == "deliveryClaimed"
                })
            })
        {
            return;
        }
        cancel_dictation_take(&self.app, Some(&self.take_id));
        let state = self.app.state::<HelperState>();
        set_agent_dictation_phase(
            &state,
            &self.take_id,
            "failed",
            Some("The controlling chat turn ended before dictation completed.".into()),
        );
        let _ = send_helper_command(
            &state,
            serde_json::json!({"type":"discard_recording","takeId":self.take_id}),
        );
    }
}

/// A main-window delivery claim validates the original host binding again
/// immediately before text is written to a composer, even after navigation.
#[tauri::command]
pub async fn claim_agent_dictation_delivery(
    app: AppHandle,
    window: WebviewWindow,
    take_id: String,
    composer_request_id: String,
) -> Result<(), AppError> {
    if window.label() != "main" {
        return Err(AppError::new(
            "dictation_delivery_forbidden",
            "Only the destination window may claim delivery.",
        ));
    }
    let target = app
        .state::<HelperState>()
        .agent_take
        .lock()
        .map_err(|_| AppError::new("dictation_unavailable", "Dictation state is unavailable."))?
        .as_ref()
        .filter(|target| {
            target.take_id == take_id
                && target.composer_request_id == composer_request_id
                && target.phase == "delivering"
        })
        .cloned()
        .ok_or_else(|| {
            AppError::new(
                "dictation_take_not_found",
                "The dictation delivery expired.",
            )
        })?;
    let binding = target
        .binding
        .as_ref()
        .ok_or_else(|| AppError::new("turn_expired", "The dictation has no controlling turn."))?;
    let repos = crate::commands::repositories(&app).await?;
    crate::mcp_server::tools::revalidate_binding(Some(&app), &repos.pool, binding, &target.profile)
        .await?;
    let state = app.state::<HelperState>();
    let mut active = state
        .agent_take
        .lock()
        .map_err(|_| AppError::new("dictation_unavailable", "Dictation state is unavailable."))?;
    let take = active
        .as_mut()
        .filter(|current| {
            current.take_id == take_id
                && current.composer_request_id == composer_request_id
                && current.phase == "delivering"
        })
        .ok_or_else(|| {
            AppError::new(
                "dictation_take_not_found",
                "The dictation delivery expired.",
            )
        })?;
    // Claim commits the fixed composer insertion. Cancellation may win before
    // this point; afterwards it must report too late rather than false success.
    take.phase = "deliveryClaimed".into();
    Ok(())
}

/// Dispatches a closed lifecycle operation and awaits the actual helper/UI acknowledgement.
pub(crate) async fn agent_dictation_control(
    app: &AppHandle,
    action: AgentDictationAction,
    target: AgentDictationTake,
) -> Result<AgentDictationTake, AppError> {
    let state = app.state::<HelperState>();
    if matches!(action, AgentDictationAction::Start)
        && app
            .try_state::<ShortcutActivationState>()
            .and_then(|state| {
                state.controller.lock().ok().map(|controller| {
                    controller.helper_is_listening || controller.pending_helper_take_id.is_some()
                })
            })
            .unwrap_or(false)
    {
        return Err(AppError::new(
            "dictation_busy",
            "Another dictation take is starting or listening.",
        ));
    }
    {
        let mut active = state.agent_take.lock().map_err(|_| {
            AppError::new("dictation_unavailable", "Dictation state is unavailable.")
        })?;
        match action {
            AgentDictationAction::Start => {
                if DICTATION_ACTIVE.load(Ordering::SeqCst)
                    || active.as_ref().is_some_and(|take| {
                        !matches!(take.phase.as_str(), "delivered" | "cancelled" | "failed")
                    })
                {
                    return Err(AppError::new(
                        "dictation_busy",
                        "Another dictation take is active.",
                    ));
                }
                *active = Some(target.clone());
            }
            AgentDictationAction::Stop | AgentDictationAction::Cancel => {
                let take = active
                    .as_mut()
                    .filter(|take| {
                        take.take_id == target.take_id
                            && take.profile == target.profile
                            && take.conversation_id == target.conversation_id
                            && take.composer_request_id == target.composer_request_id
                    })
                    .ok_or_else(|| {
                        AppError::new(
                            "dictation_take_not_found",
                            "The bound dictation take is no longer active.",
                        )
                    })?;
                if take.phase == "deliveryClaimed" {
                    return Err(AppError::new("dictation_delivery_in_progress","The fixed composer delivery has already been claimed and cannot be cancelled."));
                }
                if matches!(take.phase.as_str(), "delivered" | "cancelled" | "failed") {
                    return Err(AppError::new(
                        "dictation_take_finished",
                        "The dictation take has already finished.",
                    ));
                }
                take.binding = target.binding.clone();
                take.phase = if matches!(action, AgentDictationAction::Cancel) {
                    "cancelling"
                } else {
                    "finalizing"
                }
                .into();
            }
        }
    }
    let mut lease = AgentDictationOperationLease {
        app: app.clone(),
        take_id: target.take_id.clone(),
        complete: false,
    };
    let window = app.get_webview_window(&target.window).ok_or_else(|| {
        AppError::new(
            "dictation_window_unavailable",
            "The destination window is unavailable.",
        )
    })?;
    let kind = match action {
        AgentDictationAction::Start => "start_listening",
        AgentDictationAction::Stop => "stop_and_paste",
        AgentDictationAction::Cancel => "discard_recording",
    };
    let authority = target
        .binding
        .as_ref()
        .map(|binding| &binding.authority)
        .ok_or_else(|| {
            AppError::new(
                "turn_expired",
                "The dictation command has no host authority.",
            )
        })?;
    if let Err(error) = dictation_helper_command_with_authority(
        app.clone(),
        window,
        app.state::<HelperState>(),
        serde_json::json!({"type":kind,"takeId":target.take_id}),
        Some(authority),
    ) {
        set_agent_dictation_phase(
            &state,
            &target.take_id,
            "failed",
            Some(error.message.clone()),
        );
        return Err(error);
    }
    let completed = async {
        loop {
            let notified = state.agent_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let current = agent_dictation_status(app, &target.profile, &target.conversation_id)?
                .filter(|take| take.take_id == target.take_id)
                .ok_or_else(|| {
                    AppError::new(
                        "dictation_take_replaced",
                        "The dictation take was replaced.",
                    )
                })?;
            if current.phase == "failed" {
                return Err(AppError::new(
                    current
                        .error_code
                        .unwrap_or_else(|| "dictation_failed".into()),
                    current.error.unwrap_or_else(|| "Dictation failed.".into()),
                ));
            }
            if current.phase == "cancelled" && !matches!(action, AgentDictationAction::Cancel) {
                return Err(AppError::new(
                    "dictation_cancelled",
                    "The dictation take was discarded before delivery.",
                ));
            }
            let done = match action {
                AgentDictationAction::Start => current.phase == "listening",
                AgentDictationAction::Stop => current.phase == "delivered",
                AgentDictationAction::Cancel => current.phase == "cancelled",
            };
            if done {
                return Ok(current);
            }
            notified.await;
        }
    };
    match tokio::time::timeout(
        Duration::from_secs(if matches!(action, AgentDictationAction::Stop) {
            180
        } else {
            20
        }),
        completed,
    )
    .await
    {
        Ok(result) => {
            lease.complete = result.is_ok();
            result
        }
        Err(_) => {
            cancel_dictation_take(app, Some(&target.take_id));
            let _ = send_helper_command(
                &state,
                serde_json::json!({"type":"discard_recording","takeId":target.take_id}),
            );
            set_agent_dictation_phase(
                &state,
                &target.take_id,
                "failed",
                Some("The dictation acknowledgement timed out.".into()),
            );
            let mut error=AppError::new("dictation_timeout","The dictation acknowledgement timed out; cancellation was requested. Inspect the take before retrying.");
            error.details = Some(
                serde_json::json!({"target":target,"delivery":"unknown","effect":"unknown","retrySafe":false}),
            );
            Err(error)
        }
    }
}

fn set_agent_dictation_phase(state: &HelperState, id: &str, phase: &str, error: Option<String>) {
    if let Ok(mut active) = state.agent_take.lock() {
        if let Some(take) = active.as_mut().filter(|take| {
            take.take_id == id
                && !matches!(
                    take.phase.as_str(),
                    "delivered" | "cancelled" | "failed" | "deliveryClaimed"
                )
        }) {
            take.phase = phase.into();
            take.error = error;
            state.agent_changed.notify_waiters();
        }
    }
}
fn fail_agent_dictation(state: &HelperState, id: &str, code: &str, message: &str) {
    if let Ok(mut active) = state.agent_take.lock() {
        if let Some(take) = active.as_mut().filter(|take| {
            take.take_id == id
                && !matches!(
                    take.phase.as_str(),
                    "delivered" | "cancelled" | "failed" | "deliveryClaimed"
                )
        }) {
            take.phase = "failed".into();
            take.error_code = Some(code.into());
            take.error = Some(message.into());
            state.agent_changed.notify_waiters();
        }
    }
}
fn observe_agent_dictation_event(app: &AppHandle, event: &serde_json::Value) {
    let Some(state) = app.try_state::<HelperState>() else {
        return;
    };
    let id = dictation_take_id_from_event(event);
    let phase = match event.get("type").and_then(serde_json::Value::as_str) {
        Some("listening_started") => "listening",
        Some("recording_discarded") => "cancelled",
        Some("error" | "helper_unavailable") => "failed",
        _ => return,
    };
    let event_error = event.get("payload");
    if let Some(id) = id {
        if phase == "listening" {
            let replaced = state.agent_take.lock().ok().and_then(|active| {
                active
                    .as_ref()
                    .filter(|target| target.take_id != id)
                    .map(|target| target.take_id.clone())
            });
            if let Some(replaced) = replaced {
                set_agent_dictation_phase(
                    &state,
                    &replaced,
                    "failed",
                    Some("A newer dictation take replaced this take.".into()),
                );
            }
        }

        // Discard after finalization is a helper cleanup, not a user cancellation:
        // the correlated composer delivery still owns completion.
        if phase == "cancelled"
            && state
                .agent_take
                .lock()
                .ok()
                .and_then(|active| {
                    active.as_ref().map(|take| {
                        take.take_id == id
                            && (take.phase == "deliveryClaimed"
                                || (take.phase == "delivering"
                                    && dictation_discard_reason_from_event(event)
                                        != Some("escape")))
                    })
                })
                .unwrap_or(false)
        {
            return;
        }
        if phase == "failed" {
            let code = event_error
                .and_then(|payload| payload.get("code"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("dictation_helper_failed");
            let message = event_error
                .and_then(|payload| payload.get("message"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("The dictation helper reported an error.");
            fail_agent_dictation(&state, id, code, message);
        } else {
            set_agent_dictation_phase(&state, id, phase, None);
        }
    } else if phase == "failed" {
        let id = state
            .agent_take
            .lock()
            .ok()
            .and_then(|active| active.as_ref().map(|take| take.take_id.clone()));
        if let Some(id) = id {
            fail_agent_dictation(
                &state,
                &id,
                "dictation_helper_unavailable",
                "The dictation helper became unavailable.",
            );
        }
    }
}

/// Profile-qualified deletion shared by UI and MCP; ids alone confer no access.
pub(crate) async fn delete_history_for_profile(
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
    id: &str,
) -> Result<(), AppError> {
    let result = sqlx::query::query("DELETE FROM dictation_history WHERE id = ? AND profile = ?")
        .bind(id)
        .bind(profile)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::new(
            "dictation_not_found",
            "The dictation was not found in this data partition.",
        ));
    }
    Ok(())
}

/// MCP deletion claims the approved partition under SQLite's write lock.
pub(crate) async fn delete_history_authorized(
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
    id: &str,
    authority: crate::db::repositories::McpMutation<'_>,
) -> Result<(), AppError> {
    use crate::db::repositories::McpMutation;
    let mut tx = if matches!(authority, McpMutation::External(_)) {
        pool.begin_with("BEGIN IMMEDIATE").await?
    } else {
        pool.begin().await?
    };
    authority.guard(&mut tx, profile).await?;
    let expected = authority
        .external_target()
        .and_then(|target| target["createdAt"].as_str());
    let result = sqlx::query::query(
        "DELETE FROM dictation_history WHERE id=? AND profile=? AND (? IS NULL OR created_at = ?)",
    )
    .bind(id)
    .bind(profile)
    .bind(expected)
    .bind(expected)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::new(
            "dictation_not_found",
            "The dictation was not found in this data partition.",
        ));
    }
    authority.before_commit(profile)?;
    tx.commit().await?;
    Ok(())
}

const DICTATION_TAKE_CAPTURING: u8 = 0;
const DICTATION_TAKE_PROCESSING: u8 = 1;
const DICTATION_TAKE_CANCELLED: u8 = 2;
const DICTATION_TAKE_DELIVERING: u8 = 3;

#[derive(Clone)]
struct DictationTakeToken {
    inner: Arc<DictationTakeTokenInner>,
}

struct DictationTakeTokenInner {
    id: String,
    transcription_settings: crate::providers::ProviderModelSettings,
    state: AtomicU8,
    cancellation: tokio::sync::watch::Sender<bool>,
}

impl DictationTakeToken {
    fn new(id: String, transcription_settings: crate::providers::ProviderModelSettings) -> Self {
        let (cancellation, _) = tokio::sync::watch::channel(false);
        Self {
            inner: Arc::new(DictationTakeTokenInner {
                id,
                // Reuse admission's selection; credentials are resolved only at transcription.
                transcription_settings,
                state: AtomicU8::new(DICTATION_TAKE_CAPTURING),
                cancellation,
            }),
        }
    }

    fn id(&self) -> &str {
        &self.inner.id
    }

    fn cancel(&self) -> bool {
        loop {
            let state = self.inner.state.load(Ordering::SeqCst);
            if !matches!(state, DICTATION_TAKE_CAPTURING | DICTATION_TAKE_PROCESSING) {
                return false;
            }
            if self
                .inner
                .state
                .compare_exchange(
                    state,
                    DICTATION_TAKE_CANCELLED,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                )
                .is_ok()
            {
                break;
            }
        }
        self.inner.cancellation.send_replace(true);
        // A cancelled take never delivers, and its transcription task returns
        // without an event, so it stops holding an update install here. A
        // take already delivering cannot be cancelled; its paste ends it.
        crate::update_gate::UPDATE_GATE.end_dictation_take(self.id());
        true
    }

    fn try_begin_processing(&self) -> bool {
        self.inner
            .state
            .compare_exchange(
                DICTATION_TAKE_CAPTURING,
                DICTATION_TAKE_PROCESSING,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    fn try_begin_delivery(&self) -> Option<DictationDeliveryClaim> {
        self.inner
            .state
            .compare_exchange(
                DICTATION_TAKE_PROCESSING,
                DICTATION_TAKE_DELIVERING,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .ok()
            .map(|_| DictationDeliveryClaim { take: self.clone() })
    }

    fn state(&self) -> u8 {
        self.inner.state.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    fn same_take(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    async fn cancelled(&self) {
        let mut cancellation = self.inner.cancellation.subscribe();
        while !*cancellation.borrow_and_update() {
            if cancellation.changed().await.is_err() {
                return;
            }
        }
    }
}

struct DictationDeliveryClaim {
    take: DictationTakeToken,
}

enum DictationFinalizedTake {
    Start(DictationTakeToken),
    Cancelled,
    Duplicate,
    Stale,
}

/// The one helper-owned take that may still produce text. A cancelled token is
/// retained by its dictation transcription task after a replacement take begins,
/// so late completion cannot inherit the replacement's delivery authority.
#[derive(Default)]
pub struct DictationTakeState {
    current: Mutex<Option<DictationTakeToken>>,
    admitted_settings:
        Mutex<std::collections::HashMap<String, crate::providers::ProviderModelSettings>>,
    helper_command_generation: Mutex<u64>,
    #[cfg(any(target_os = "macos", test))]
    last_dictation: Mutex<Option<(String, String)>>,
}

impl DictationTakeState {
    // Retain only a final, authorized result, before helper I/O can fail.
    // The delivery claim remains the cancellation point of no return (ADR-0049).
    #[cfg(any(target_os = "macos", test))]
    fn remember_last_dictation(
        &self,
        claim: &DictationDeliveryClaim,
        profile: &str,
        outcome: &DictationTranscriptionOutcome,
    ) {
        // A preclaimed result may finish after a replacement started. Never let
        // it overwrite that replacement's cache, even though delivery was authorized.
        let current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !current
            .as_ref()
            .is_some_and(|take| Arc::ptr_eq(&take.inner, &claim.take.inner))
        {
            return;
        }
        let eligible = outcome.quarantine_transcript.is_none()
            && (outcome.helper_command["type"] == "paste_text"
                || outcome
                    .event
                    .as_ref()
                    .is_some_and(|event| event["type"] == "agent_session_prompt"));
        if let Some(text) = outcome
            .transcript
            .as_ref()
            .filter(|transcript| eligible && !transcript.text.trim().is_empty())
            .map(|transcript| &transcript.text)
        {
            *self
                .last_dictation
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some((profile.to_string(), text.clone()));
        }
    }

    #[cfg(any(target_os = "macos", test))]
    fn last_dictation_response(
        &self,
        profile: &str,
        request: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let request_id = request.get("payload")?.get("requestId")?.as_str()?;
        if request_id.is_empty() || request_id.len() > 128 {
            return None;
        }
        let last = self
            .last_dictation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let text = last
            .as_ref()
            .filter(|(owner, _)| owner == profile)
            .map(|(_, text)| text);
        Some(
            serde_json::json!({"type": "repeat_last_dictation", "requestId": request_id, "text": text}),
        )
    }

    fn remember_admitted_settings(
        &self,
        id: &str,
        settings: crate::providers::ProviderModelSettings,
    ) {
        self.admitted_settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(id.to_string())
            .or_insert(settings);
    }

    fn take_admitted_settings(&self, id: &str) -> crate::providers::ProviderModelSettings {
        let mut admitted = self
            .admitted_settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        admitted
            .remove(id)
            .or_else(|| admitted.remove(UNTAGGED_DICTATION_TAKE))
            .unwrap_or_else(crate::providers::effective_current_settings)
    }

    fn forget_admitted_settings(&self, id: Option<&str>) {
        let mut admitted = self
            .admitted_settings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(id) = id {
            admitted.remove(id);
        } else {
            admitted.clear();
        }
    }

    fn begin(&self, id: String) -> DictationTakeToken {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = current
            .as_ref()
            .filter(|existing| existing.id() == id.as_str())
        {
            return existing.clone();
        }
        if let Some(previous) = current.take() {
            previous.cancel();
        }
        let settings = self.take_admitted_settings(&id);
        let token = DictationTakeToken::new(id, settings);
        *current = Some(token.clone());
        token
    }

    fn claim_finalized(&self, id: Option<&str>) -> DictationFinalizedTake {
        let mut current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(existing) = current.as_ref() {
            if let Some(id) = id.filter(|id| existing.id() != *id) {
                // Dropped untranscribed: nothing else will end it in the gate.
                crate::update_gate::UPDATE_GATE.end_dictation_take(id);
                return DictationFinalizedTake::Stale;
            }
            if existing.try_begin_processing() {
                return DictationFinalizedTake::Start(existing.clone());
            }
            return match existing.state() {
                DICTATION_TAKE_CANCELLED => DictationFinalizedTake::Cancelled,
                DICTATION_TAKE_PROCESSING | DICTATION_TAKE_DELIVERING => {
                    DictationFinalizedTake::Duplicate
                }
                _ => {
                    crate::update_gate::UPDATE_GATE.end_dictation_take(existing.id());
                    DictationFinalizedTake::Stale
                }
            };
        }
        let id = id
            .map(str::to_string)
            .unwrap_or_else(|| format!("legacy-{}", uuid::Uuid::new_v4()));
        let settings = self.take_admitted_settings(&id);
        let token = DictationTakeToken::new(id, settings);
        let processing_started = token.try_begin_processing();
        debug_assert!(processing_started);
        *current = Some(token.clone());
        DictationFinalizedTake::Start(token)
    }

    fn cancel(&self, id: Option<&str>) -> bool {
        let current = self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if id.is_some_and(|id| current.as_ref().is_some_and(|current| current.id() != id)) {
            return false;
        }
        current.as_ref().is_some_and(DictationTakeToken::cancel)
    }

    fn current_take_id(&self) -> Option<String> {
        self.current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .map(|take| take.id().to_string())
    }

    fn run_new_helper_command<T>(&self, operation: impl FnOnce(u64) -> T) -> (u64, T) {
        let mut generation = self
            .helper_command_generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *generation = generation.wrapping_add(1);
        let current = *generation;
        (current, operation(current))
    }

    fn run_new_helper_command_if<P, T>(
        &self,
        prepare: impl FnOnce() -> Option<P>,
        operation: impl FnOnce(u64, P) -> T,
    ) -> Option<(u64, T)> {
        let mut generation = self
            .helper_command_generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prepared = prepare()?;
        *generation = generation.wrapping_add(1);
        let current = *generation;
        Some((current, operation(current, prepared)))
    }

    fn lock_helper_commands(&self) -> std::sync::MutexGuard<'_, u64> {
        self.helper_command_generation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

async fn run_dictation_take_step<F, T>(take: &DictationTakeToken, future: F) -> Option<T>
where
    F: Future<Output = T>,
{
    tokio::select! {
        biased;
        _ = take.cancelled() => None,
        output = future => Some(output),
    }
}

pub struct LastDictationEvent {
    event: Mutex<Option<String>>,
}

pub struct DictationSettingsState {
    path: PathBuf,
    settings: Mutex<DictationSettings>,
}

pub struct HotkeyStatus {
    event: Mutex<serde_json::Value>,
}

/// Only an explicit user drag is remembered within this process. Resizes and
/// native startup moves must never replace the configured screen-edge anchor.
pub struct HudPosition {
    inner: Mutex<HudPositionState>,
}

#[derive(Default)]
struct HudPositionState {
    saved: Option<(i32, i32)>,
    drag_origin: Option<(i32, i32)>,
}

impl HudPositionState {
    fn begin_drag(&mut self, position: (i32, i32)) {
        self.drag_origin = Some(position);
    }

    fn observe_drag(&mut self, position: (i32, i32), mouse_down: bool) {
        if mouse_down {
            return;
        }
        if let Some(origin) = self.drag_origin.take() {
            if position != origin {
                self.saved = Some(position);
            }
        }
    }

    fn cancel_drag(&mut self) {
        self.drag_origin = None;
    }

    fn reset(&mut self) {
        self.cancel_drag();
        self.saved = None;
    }
}

/// Stop capture before any programmatic movement. A queued release sample
/// must never mistake a resize, shake, or meeting prompt for a user's drag.
fn cancel_hud_drag(hud: &WebviewWindow) {
    if let Some(state) = hud.app_handle().try_state::<HudPosition>() {
        if let Ok(mut position) = state.inner.lock() {
            position.cancel_drag();
        }
    }
}

fn poll_hud_drag(app: &AppHandle, hud: &WebviewWindow) {
    let dragging = app
        .try_state::<HudPosition>()
        .and_then(|state| {
            state
                .inner
                .lock()
                .ok()
                .map(|position| position.drag_origin.is_some())
        })
        .unwrap_or(false);
    if !dragging {
        return;
    }
    let app_for_drag = app.clone();
    let hud = hud.clone();
    // Read the frame and button state together on the main thread. In
    // particular, never hold the position mutex while waiting for a native
    // frame getter on another thread.
    let _ = app.run_on_main_thread(move || {
        let Ok(position) = hud.outer_position() else {
            return;
        };
        let mouse_down = hud_left_mouse_button_down();
        if let Some(state) = app_for_drag.try_state::<HudPosition>() {
            if let Ok(mut saved) = state.inner.lock() {
                saved.observe_drag((position.x, position.y), mouse_down);
            }
        }
    });
}

#[cfg(target_os = "macos")]
fn hud_left_mouse_button_down() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
    }
    // Combined session state, left button: AppKit need not deliver mouseUp
    // to this process after performWindowDragWithEvent returns immediately.
    unsafe { CGEventSourceButtonState(0, 0) }
}

#[cfg(target_os = "windows")]
fn hud_left_mouse_button_down() -> bool {
    #[link(name = "user32")]
    extern "system" {
        fn GetAsyncKeyState(v_key: i32) -> i16;
    }
    unsafe { GetAsyncKeyState(1) < 0 }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn hud_left_mouse_button_down() -> bool {
    false
}

pub struct ShortcutActivationState {
    controller: Mutex<ShortcutActivationController>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HudClientRect {
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
}

/// Hover state for HUD controls. Polled by [`spawn_hud_hover_thread`]; we
/// don't poll from JS because WebKit throttles timers on the non-key HUD
/// panel (and CSS :hover is unreliable there for the same reason). The
/// stop button, cancel button, and the meeting prompt controls each get a rect.
/// The window includes a small transparent gutter for the CSS shadow, but
/// hover still keys off the controls themselves rather than the full frame.
pub struct HudHoverState {
    stop_bounds: Mutex<Option<HudClientRect>>,
    last_hover: std::sync::atomic::AtomicBool,
    cancel_bounds: Mutex<Option<HudClientRect>>,
    last_cancel_hover: std::sync::atomic::AtomicBool,
    dismiss_bounds: Mutex<Option<HudClientRect>>,
    last_dismiss_hover: std::sync::atomic::AtomicBool,
    record_bounds: Mutex<Option<HudClientRect>>,
    last_record_hover: std::sync::atomic::AtomicBool,
}

pub fn configured_transcription_language() -> Option<String> {
    settings_store()
        .lock()
        .ok()
        .and_then(|settings| settings.language.clone())
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DictationHudPlacement {
    #[default]
    Bottom,
    Top,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationSettings {
    pub push_to_talk_shortcut: DictationShortcutSetting,
    pub toggle_shortcut: DictationShortcutSetting,
    pub microphone: DictationMicrophoneSetting,
    pub style: DictationStyle,
    pub hud_placement: DictationHudPlacement,
    pub language: Option<String>,
    /// Off by default: the Whisper transcript is pasted with no LLM call.
    pub cleanup_enabled: bool,
    /// OpenRouter model id for the cleanup chat completion.
    pub cleanup_model: String,
    /// Records the one-time move to optional cleanup (ditado-rapido D1):
    /// a file saved before it lacks the field and migrates to cleanup off.
    pub cleanup_migrated: bool,
}

impl Default for DictationSettings {
    fn default() -> Self {
        Self::default_for_platform(cfg!(target_os = "windows"))
    }
}

impl DictationSettings {
    /// Windows dictates with Right Ctrl alone by default (there is no Fn
    /// there); a saved shortcut is kept as it is.
    fn default_for_platform(windows: bool) -> Self {
        Self {
            push_to_talk_shortcut: default_shortcut_for(DictationShortcutKind::PushToTalk, windows),
            toggle_shortcut: DictationShortcutSetting::control_option_t(),
            microphone: DictationMicrophoneSetting::default(),
            style: DictationStyle::Standard,
            hud_placement: DictationHudPlacement::default(),
            language: None,
            cleanup_enabled: false,
            cleanup_model: DEFAULT_DICTATION_CLEANUP_MODEL.to_string(),
            cleanup_migrated: true,
        }
    }
}

const MAX_DICTATION_CLEANUP_MODEL_CHARS: usize = 200;

/// One-time move to optional cleanup: settings saved before it (cleanup
/// always ran, on Clovy API, an endpoint, or a CLI) start with cleanup off.
/// Returns whether the settings changed and should be written back.
fn migrate_dictation_cleanup(settings: &mut DictationSettings) -> bool {
    if settings.cleanup_migrated {
        return false;
    }
    settings.cleanup_enabled = false;
    settings.cleanup_migrated = true;
    true
}

/// Accepts an OpenRouter catalog id (`vendor/model`, optional `:variant`);
/// CLI, endpoint, and internal option ids are refused.
fn validate_dictation_cleanup_model(model: &str) -> Result<String, AppError> {
    let model = model.trim();
    let valid = model.chars().count() <= MAX_DICTATION_CLEANUP_MODEL_CHARS
        && !model.starts_with("__")
        && !model.chars().any(char::is_whitespace)
        && model
            .split_once('/')
            .is_some_and(|(vendor, name)| !vendor.is_empty() && !name.is_empty())
        && !model
            .split('/')
            .next()
            .is_some_and(|vendor| vendor.contains(':'));
    if valid {
        Ok(model.to_string())
    } else {
        Err(AppError::new(
            "dictation_cleanup_model_invalid",
            "Choose an OpenRouter model for dictation cleanup.",
        ))
    }
}

impl<'de> Deserialize<'de> for DictationSettings {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SettingsValue {
            push_to_talk_shortcut: Option<DictationShortcutSetting>,
            toggle_shortcut: Option<DictationShortcutSetting>,
            microphone: Option<DictationMicrophoneSetting>,
            style: Option<DictationStyle>,
            hud_placement: Option<DictationHudPlacement>,
            language: Option<String>,
            cleanup_enabled: Option<bool>,
            cleanup_model: Option<String>,
            cleanup_migrated: Option<bool>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        let microphone = value
            .get("microphone")
            .cloned()
            .and_then(|microphone| serde_json::from_value(microphone).ok())
            .unwrap_or_default();

        if value.get("shortcut").is_some() || value.get("activationMode").is_some() {
            return Ok(Self {
                microphone,
                ..Self::default()
            });
        }

        let settings: SettingsValue =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;

        Ok(Self {
            push_to_talk_shortcut: settings
                .push_to_talk_shortcut
                .map(|shortcut| shortcut.normalized_or_default(DictationShortcutKind::PushToTalk))
                .unwrap_or_else(|| {
                    default_shortcut_for(
                        DictationShortcutKind::PushToTalk,
                        cfg!(target_os = "windows"),
                    )
                }),
            toggle_shortcut: settings
                .toggle_shortcut
                .map(|shortcut| shortcut.normalized_or_default(DictationShortcutKind::Toggle))
                .unwrap_or_else(DictationShortcutSetting::control_option_t),
            microphone: settings.microphone.unwrap_or(microphone),
            style: settings.style.unwrap_or_default(),
            hud_placement: settings.hud_placement.unwrap_or_default(),
            language: normalize_language(settings.language),
            cleanup_enabled: settings.cleanup_enabled.unwrap_or(false),
            cleanup_model: settings
                .cleanup_model
                .and_then(|model| validate_dictation_cleanup_model(&model).ok())
                .unwrap_or_else(|| DEFAULT_DICTATION_CLEANUP_MODEL.to_string()),
            cleanup_migrated: settings.cleanup_migrated.unwrap_or(false),
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DictationStyle {
    #[default]
    Standard,
    CasualLowercase,
    Formal,
}

impl DictationStyle {
    fn instruction(self) -> &'static str {
        match self {
            Self::Standard => {
                "Writing style: standard. Standard sentence capitalization and punctuation. Keep the speaker's wording and tone exactly as dictated."
            }
            Self::CasualLowercase => {
                "Writing style: casual lowercase. Use lowercase wherever grammatically possible, including the beginning of sentences, while preserving proper nouns, acronyms, brand names, and code exactly when capitalization matters. Keep the speaker's wording exactly as dictated."
            }
            Self::Formal => {
                "Writing style: formal. Conventional capitalization, complete punctuation, and full words: expand casual contractions, for example don't becomes do not. Beyond that, keep the speaker's own words and sentence structure; do not reword, shorten, or polish their phrasing."
            }
        }
    }

    /// Whether this style capitalizes the start of a sentence. CasualLowercase
    /// keeps sentence starts lowercase, so the deterministic filler backstop
    /// must not re-capitalize a surviving first word under it.
    fn capitalizes_sentence_starts(self) -> bool {
        !matches!(self, Self::CasualLowercase)
    }
}

fn control_option_label(key: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("Ctrl+Alt+{key}")
    } else {
        format!("Ctrl+Opt+{key}")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationShortcutSetting {
    pub key_code: u32,
    pub code: String,
    pub modifiers: DictationShortcutModifiers,
    pub label: String,
    pub press_count: u8,
}

impl<'de> Deserialize<'de> for DictationShortcutSetting {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ShortcutSettingValue {
            key_code: u32,
            code: String,
            modifiers: DictationShortcutModifiers,
            label: String,
            press_count: Option<u8>,
        }

        let value = serde_json::Value::deserialize(deserializer)?;
        if let Some(shortcut) = value.as_str() {
            return legacy_shortcut_setting(shortcut)
                .ok_or_else(|| serde::de::Error::custom("unknown legacy shortcut"));
        }

        let shortcut: ShortcutSettingValue =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        let press_count = shortcut.press_count.unwrap_or(1);
        Ok(Self {
            key_code: shortcut.key_code,
            code: shortcut.code,
            modifiers: shortcut.modifiers,
            label: shortcut.label,
            press_count: if press_count == 2 { 2 } else { 1 },
        })
    }
}

impl DictationShortcutSetting {
    fn bare_fn() -> Self {
        Self {
            key_code: 0,
            code: "Fn".to_string(),
            modifiers: DictationShortcutModifiers {
                function: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Fn".to_string(),
            press_count: 1,
        }
    }

    fn modifier_only(modifiers: DictationShortcutModifiers) -> Self {
        Self {
            key_code: 0,
            code: "Modifiers".to_string(),
            label: modifier_only_label(&modifiers),
            modifiers,
            press_count: 1,
        }
    }

    /// Right Ctrl alone: the Windows push-to-talk default.
    fn right_ctrl() -> Self {
        Self {
            key_code: 0,
            code: "ControlRight".to_string(),
            modifiers: DictationShortcutModifiers::default(),
            label: "Right Ctrl".to_string(),
            press_count: 1,
        }
    }

    fn control_option_d() -> Self {
        Self {
            key_code: 0x02,
            code: "KeyD".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                option: true,
                ..DictationShortcutModifiers::default()
            },
            label: control_option_label("D"),
            press_count: 1,
        }
    }

    fn control_option_t() -> Self {
        Self {
            key_code: 0x11,
            code: "KeyT".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                option: true,
                ..DictationShortcutModifiers::default()
            },
            label: control_option_label("T"),
            press_count: 1,
        }
    }

    fn same_trigger_as(&self, other: &Self) -> bool {
        self.key_code == other.key_code
            && self.code == other.code
            && self.modifiers == other.modifiers
            && self.press_count == other.press_count
    }

    fn normalized_or_default(self, kind: DictationShortcutKind) -> Self {
        self.normalized_for_platform(kind, cfg!(target_os = "windows"))
    }

    fn normalized_for_platform(mut self, kind: DictationShortcutKind, windows: bool) -> Self {
        let double_tap =
            self.press_count == 2 && is_windows_lone_modifier(&self.code, &self.modifiers);
        if windows
            && (self.press_count == 1 || double_tap)
            && is_windows_lone_key(&self.code, &self.modifiers)
        {
            self.key_code = key_code_for_code(&self.code).unwrap_or(0);
            return self;
        }
        if windows
            && (self.press_count != 1
                || self.modifiers.function
                || is_modifier_only_input(&self.code, &self.modifiers))
        {
            return default_shortcut_for(kind, windows);
        }
        if self.is_bare_fn() {
            return DictationShortcutSetting::bare_fn();
        }
        if is_modifier_only_input(&self.code, &self.modifiers) {
            return DictationShortcutSetting::modifier_only(self.modifiers);
        }
        let Some(key_code) = key_code_for_platform(&self.code, windows) else {
            return default_shortcut_for(kind, windows);
        };
        if !self.modifiers.has_any() {
            return default_shortcut_for(kind, windows);
        }
        self.key_code = key_code;
        self.press_count = if self.press_count == 2 { 2 } else { 1 };
        self
    }

    fn is_bare_fn(&self) -> bool {
        is_bare_fn_input(&self.code, &self.modifiers)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationShortcutModifiers {
    pub command: bool,
    pub control: bool,
    pub option: bool,
    pub shift: bool,
    pub function: bool,
}

impl DictationShortcutModifiers {
    fn has_any(&self) -> bool {
        self.command || self.control || self.option || self.shift || self.function
    }

    fn count(&self) -> usize {
        [
            self.command,
            self.control,
            self.option,
            self.shift,
            self.function,
        ]
        .into_iter()
        .filter(|enabled| *enabled)
        .count()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationShortcutInput {
    pub code: String,
    pub modifiers: DictationShortcutModifiers,
    pub label: String,
    pub press_count: Option<u8>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DictationShortcutKind {
    PushToTalk,
    Toggle,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DictationMicrophoneSetting {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationCapabilities {
    pub available: bool,
    pub platform: &'static str,
    pub shortcuts: bool,
    pub paste: bool,
    pub microphone_selection: bool,
    pub accessibility_permission: bool,
    pub system_audio: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationCapabilitiesResponse {
    pub capabilities: DictationCapabilities,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationSettingsResponse {
    pub settings: DictationSettings,
}

/// A push released faster than this is a graze or an aborted press, not a
/// dictation: recording starts on the down edge now (the helper no longer
/// holds unambiguous presses back), so quick releases must discard instead of
/// transcribing a fraction of a second of noise. Mirrors the helper's old
/// holdThreshold, which used to absorb these by delaying the start.
const PUSH_TO_TALK_MIN_HOLD: Duration = Duration::from_millis(160);
const TOGGLE_SHORTCUT_DEBOUNCE: Duration = Duration::from_millis(275);
/// A toggle ack normally arrives within milliseconds; if the helper dies (or
/// a permission prompt never calls back) no ack event ever comes, and an
/// unexpiring in-flight flag would wedge the shortcut until app restart.
const TOGGLE_ACK_EXPIRY: Duration = Duration::from_secs(8);
/// Finalizing normally resolves in a few seconds (one dictation round-trip);
/// a helper that dies mid-finalize emits no terminal event, so the
/// suppression must expire rather than wedge the shortcut.
const FINALIZING_SUPPRESSION_EXPIRY: Duration = Duration::from_secs(30);

#[derive(Debug, Default)]
struct ShortcutActivationController {
    active_mode: Option<DictationShortcutKind>,
    /// Updated only from helper lifecycle events. Unlike `active_mode`, this
    /// also reflects recordings started by the composer button.
    helper_is_listening: bool,
    /// The take the helper has acknowledged with `listening_started`.
    helper_take_id: Option<String>,
    /// A start command written to the helper but not yet acknowledged. Keeping
    /// it separate from confirmed ownership lets a correlated rejection roll
    /// back the attempted start without losing control of the prior take.
    pending_helper_take_id: Option<String>,
    /// Set after this helper process emits any take-correlated lifecycle
    /// event. Once negotiated, an untagged take-owned event cannot terminate a
    /// tagged active take; untagged global errors remain event-only.
    helper_uses_take_ids: bool,
    push_to_talk_is_down: bool,
    push_started_at: Option<Instant>,
    toggle_command_in_flight: bool,
    last_toggle_command_at: Option<Instant>,
    helper_finalizing_since: Option<Instant>,
    pending_composer_request_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortcutKeyEdge {
    Down,
    Up,
    /// Another key joined a lone-key shortcut while it was held (Windows
    /// Right Ctrl + C): whatever the press started is dropped.
    Cancel,
}

fn shortcut_key_edge(event_type: &str) -> Option<ShortcutKeyEdge> {
    match event_type {
        "fn_key_down" | "shortcut_key_down" => Some(ShortcutKeyEdge::Down),
        "fn_key_up" | "shortcut_key_up" => Some(ShortcutKeyEdge::Up),
        "shortcut_key_cancel" => Some(ShortcutKeyEdge::Cancel),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DictationCommand {
    StartListening,
    StopAndPaste,
    DiscardListening,
    ToggleListening,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelperLifecycleDisposition {
    Apply,
    EmitOnly,
    Ignore,
}

#[derive(Debug, PartialEq, Eq)]
struct MalformedRecordingReadyCleanup {
    take_id: Option<String>,
    send_helper_discard: bool,
}

impl ShortcutActivationController {
    fn handle_edge(
        &mut self,
        edge: ShortcutKeyEdge,
        kind: DictationShortcutKind,
        now: Instant,
    ) -> Option<DictationCommand> {
        match (kind, edge) {
            (DictationShortcutKind::PushToTalk, ShortcutKeyEdge::Down) => {
                if self.active_mode.is_some()
                    || self.push_to_talk_is_down
                    || self.helper_is_finalizing(now)
                {
                    return None;
                }
                self.active_mode = Some(DictationShortcutKind::PushToTalk);
                self.push_to_talk_is_down = true;
                self.push_started_at = Some(now);
                Some(DictationCommand::StartListening)
            }
            (DictationShortcutKind::PushToTalk, ShortcutKeyEdge::Up) => {
                let started_at = self.push_started_at.take();
                if self.active_mode == Some(DictationShortcutKind::PushToTalk)
                    && self.push_to_talk_is_down
                {
                    self.active_mode = None;
                    self.push_to_talk_is_down = false;
                    let grazed =
                        started_at.is_some_and(|at| now.duration_since(at) < PUSH_TO_TALK_MIN_HOLD);
                    Some(if grazed {
                        DictationCommand::DiscardListening
                    } else {
                        DictationCommand::StopAndPaste
                    })
                } else {
                    self.push_to_talk_is_down = false;
                    None
                }
            }
            (DictationShortcutKind::Toggle, ShortcutKeyEdge::Down) => match self.active_mode {
                Some(DictationShortcutKind::PushToTalk) => None,
                Some(DictationShortcutKind::Toggle) if self.can_send_toggle_command(now) => {
                    self.active_mode = None;
                    self.mark_toggle_command_sent(now);
                    Some(DictationCommand::ToggleListening)
                }
                Some(DictationShortcutKind::Toggle) => None,
                None if self.can_send_toggle_command(now) => {
                    self.active_mode = Some(DictationShortcutKind::Toggle);
                    self.mark_toggle_command_sent(now);
                    Some(DictationCommand::ToggleListening)
                }
                None => None,
            },
            (DictationShortcutKind::Toggle, ShortcutKeyEdge::Up) => None,
            (DictationShortcutKind::PushToTalk, ShortcutKeyEdge::Cancel) => {
                self.push_started_at = None;
                let active = self.active_mode == Some(DictationShortcutKind::PushToTalk)
                    && self.push_to_talk_is_down;
                self.push_to_talk_is_down = false;
                if active {
                    self.active_mode = None;
                    Some(DictationCommand::DiscardListening)
                } else {
                    None
                }
            }
            (DictationShortcutKind::Toggle, ShortcutKeyEdge::Cancel) => None,
        }
    }

    fn can_send_toggle_command(&self, now: Instant) -> bool {
        let in_flight = self.toggle_command_in_flight
            && self
                .last_toggle_command_at
                .map_or(true, |last| now.duration_since(last) < TOGGLE_ACK_EXPIRY);
        !in_flight
            && !self.helper_is_finalizing(now)
            && self.last_toggle_command_at.map_or(true, |last| {
                now.duration_since(last) >= TOGGLE_SHORTCUT_DEBOUNCE
            })
    }

    fn helper_is_finalizing(&self, now: Instant) -> bool {
        self.helper_finalizing_since
            .is_some_and(|since| now.duration_since(since) < FINALIZING_SUPPRESSION_EXPIRY)
    }

    fn mark_toggle_command_sent(&mut self, now: Instant) {
        self.toggle_command_in_flight = true;
        self.last_toggle_command_at = Some(now);
    }

    fn acknowledge_toggle_command(&mut self) {
        self.toggle_command_in_flight = false;
    }

    fn mark_helper_finalizing(&mut self) {
        self.helper_finalizing_since = Some(Instant::now());
    }

    fn clear_helper_finalizing(&mut self) {
        self.helper_finalizing_since = None;
    }

    fn set_helper_listening(&mut self, listening: bool, take_id: Option<&str>) {
        self.helper_is_listening = listening;
        if listening {
            self.helper_uses_take_ids |= take_id.is_some();
            self.helper_take_id = take_id.map(str::to_string);
            if self.pending_helper_take_id.as_deref() == take_id {
                self.pending_helper_take_id = None;
            }
        }
    }

    fn prepare_helper_start(&mut self, take_id: &str) {
        self.pending_helper_take_id = Some(take_id.to_string());
    }

    fn promote_untagged_helper_start(&mut self, take_id: &str) {
        self.helper_is_listening = true;
        self.helper_take_id = Some(take_id.to_string());
        if self.pending_helper_take_id.as_deref() == Some(take_id) {
            self.pending_helper_take_id = None;
        }
    }

    fn malformed_recording_ready_cleanup(
        &self,
        event_take_id: Option<&str>,
        current_take_id: Option<&str>,
    ) -> MalformedRecordingReadyCleanup {
        let take_id = event_take_id
            .map(str::to_string)
            .or_else(|| self.helper_take_id.clone())
            .or_else(|| current_take_id.map(str::to_string));
        MalformedRecordingReadyCleanup {
            take_id,
            // A tagged discard is safe because a current helper authorizes
            // its target. An older helper ignores tags, so do not send its
            // untagged cleanup after a replacement start is pending.
            send_helper_discard: event_take_id.is_some() || self.pending_helper_take_id.is_none(),
        }
    }

    fn prepare_renderer_command(
        &mut self,
        command_type: &str,
        requested_take_id: Option<String>,
        current_take_id: Option<String>,
    ) -> (bool, Option<String>) {
        let starts_recording = command_type == "start_listening"
            || (command_type == "toggle_listening"
                && !self.helper_is_listening
                && self.pending_helper_take_id.is_none());
        let active_take_id = self
            .pending_helper_take_id
            .clone()
            .or_else(|| self.helper_take_id.clone())
            .or(current_take_id);
        let take_id = requested_take_id.or_else(|| {
            starts_recording
                .then(|| uuid::Uuid::new_v4().to_string())
                .or(active_take_id)
        });

        if helper_command_type_resets_shortcut_activation(command_type) {
            let targets_input_activation = take_id.as_deref().map_or(true, |take_id| {
                self.pending_helper_take_id.as_deref() == Some(take_id)
                    || (self.pending_helper_take_id.is_none()
                        && self.helper_take_id.as_deref() == Some(take_id))
            });
            if targets_input_activation {
                self.reset_input_activation();
            }
        }
        if starts_recording {
            if let Some(take_id) = take_id.as_deref() {
                self.prepare_helper_start(take_id);
            }
        }

        (starts_recording, take_id)
    }

    fn matches_helper_take(&self, take_id: &str, current_take_id: Option<&str>) -> bool {
        self.pending_helper_take_id.as_deref() == Some(take_id)
            || self.helper_take_id.as_deref() == Some(take_id)
            || (self.pending_helper_take_id.is_none()
                && self.helper_take_id.is_none()
                && current_take_id == Some(take_id))
    }

    fn matches_delivery_take(&self, take_id: &str, current_take_id: Option<&str>) -> bool {
        if let Some(pending_take_id) = self.pending_helper_take_id.as_deref() {
            return pending_take_id == take_id;
        }
        self.helper_take_id.as_deref() == Some(take_id)
            || (self.helper_take_id.is_none() && current_take_id == Some(take_id))
    }

    fn reset_for_helper_discard(
        &mut self,
        take_id: Option<&str>,
        reason: Option<&str>,
        current_take_id: Option<&str>,
    ) -> HelperLifecycleDisposition {
        self.helper_uses_take_ids |= take_id.is_some();
        let matches_active_take = match take_id {
            Some(take_id) => self.matches_helper_take(take_id, current_take_id),
            // New helpers explain untagged terminal events such as a
            // start-pending stop. A payload with neither field is an older
            // helper, where resetting remains the compatible behavior.
            None => reason.is_none() && !self.helper_uses_take_ids,
        };
        if !matches_active_take {
            return HelperLifecycleDisposition::Ignore;
        }
        let preserves_confirmed_take = if let Some(take_id) = take_id {
            self.finish_helper_take(take_id, current_take_id)
        } else {
            self.reset();
            false
        };
        if preserves_confirmed_take {
            HelperLifecycleDisposition::EmitOnly
        } else {
            HelperLifecycleDisposition::Apply
        }
    }

    fn finish_helper_take(&mut self, take_id: &str, current_take_id: Option<&str>) -> bool {
        let finished_pending = self.pending_helper_take_id.as_deref() == Some(take_id);
        let finished_confirmed = self.helper_take_id.as_deref() == Some(take_id);
        let preserves_confirmed_take = finished_pending
            && self
                .helper_take_id
                .as_deref()
                .is_some_and(|confirmed| confirmed != take_id);
        if finished_pending {
            self.pending_helper_take_id = None;
            self.reset_input_activation();
        }
        if finished_confirmed {
            self.helper_take_id = None;
            self.helper_is_listening = false;
            self.helper_finalizing_since = None;
            if self.pending_helper_take_id.is_none() {
                self.reset_input_activation();
            }
        }
        if !finished_pending
            && !finished_confirmed
            && self.pending_helper_take_id.is_none()
            && self.helper_take_id.is_none()
            && current_take_id == Some(take_id)
        {
            self.reset();
        }
        preserves_confirmed_take
    }

    fn finish_for_helper_event(
        &mut self,
        take_id: Option<&str>,
        current_take_id: Option<&str>,
    ) -> HelperLifecycleDisposition {
        let Some(take_id) = take_id else {
            self.reset();
            return HelperLifecycleDisposition::Apply;
        };
        self.helper_uses_take_ids = true;
        if !self.matches_helper_take(take_id, current_take_id) {
            return HelperLifecycleDisposition::Ignore;
        }
        if self.finish_helper_take(take_id, current_take_id) {
            HelperLifecycleDisposition::EmitOnly
        } else {
            HelperLifecycleDisposition::Apply
        }
    }

    fn clear_helper_take(&mut self) {
        self.helper_take_id = None;
        self.pending_helper_take_id = None;
    }

    fn clear_helper_protocol(&mut self) {
        self.clear_helper_take();
        self.helper_uses_take_ids = false;
    }

    fn untagged_take_event_disposition(
        &self,
        event_type: Option<&str>,
    ) -> HelperLifecycleDisposition {
        if !self.helper_uses_take_ids
            || (self.helper_take_id.is_none() && self.pending_helper_take_id.is_none())
        {
            return HelperLifecycleDisposition::Apply;
        }
        if event_type == Some("error") {
            HelperLifecycleDisposition::EmitOnly
        } else {
            HelperLifecycleDisposition::Ignore
        }
    }

    fn command_starts_recording(&self, command: DictationCommand) -> bool {
        matches!(command, DictationCommand::StartListening)
            || (matches!(command, DictationCommand::ToggleListening)
                && !self.helper_is_listening
                && self.pending_helper_take_id.is_none())
    }

    fn track_composer_request(&mut self, request_id: Option<String>) {
        self.pending_composer_request_id = request_id;
    }

    fn clear_composer_request(&mut self, request_id: &str) {
        if self.pending_composer_request_id.as_deref() == Some(request_id) {
            self.pending_composer_request_id = None;
        }
    }

    fn reset_for_rejected_composer_request(&mut self, request_id: &str) -> bool {
        if self.pending_composer_request_id.as_deref() != Some(request_id) {
            return false;
        }
        self.pending_composer_request_id = None;
        self.pending_helper_take_id = None;
        self.reset_input_activation();
        true
    }

    fn reset(&mut self) {
        self.reset_input_activation();
        self.helper_take_id = None;
        self.pending_helper_take_id = None;
        self.helper_finalizing_since = None;
        self.pending_composer_request_id = None;
    }

    fn reset_input_activation(&mut self) {
        self.active_mode = None;
        self.push_to_talk_is_down = false;
        self.push_started_at = None;
        self.toggle_command_in_flight = false;
        self.last_toggle_command_at = None;
    }
}

impl DictationCommand {
    fn helper_command(self, shortcut_label: &str) -> serde_json::Value {
        match self {
            Self::StartListening => serde_json::json!({ "type": "start_listening" }),
            Self::StopAndPaste => serde_json::json!({ "type": "stop_and_paste" }),
            Self::DiscardListening => serde_json::json!({ "type": "discard_recording" }),
            Self::ToggleListening => serde_json::json!({
                "type": "toggle_listening",
                "shortcut": shortcut_label,
            }),
        }
    }
}

impl DictationShortcutInput {
    fn into_setting(self) -> Result<DictationShortcutSetting, AppError> {
        self.into_setting_for_platform(cfg!(target_os = "windows"))
    }

    fn into_setting_for_platform(
        self,
        windows: bool,
    ) -> Result<DictationShortcutSetting, AppError> {
        if self.code.trim().is_empty() || self.label.trim().is_empty() {
            return Err(AppError::new(
                "dictation_shortcut_incomplete",
                "Shortcut is incomplete.",
            ));
        }

        let requested_press_count = self.press_count.unwrap_or(1);
        // On Windows a double tap exists only for a lone Ctrl, Alt or Shift,
        // the Mac Fn toggle gesture; the helper's hook counts the taps.
        let double_tap_allowed = is_windows_lone_modifier(&self.code, &self.modifiers);
        if windows
            && requested_press_count != 1
            && !(requested_press_count == 2 && double_tap_allowed)
        {
            return Err(AppError::new(
                "dictation_shortcut_unsupported",
                "Double-press shortcuts are not supported on Windows.",
            ));
        }
        let press_count = if windows && double_tap_allowed && requested_press_count == 2 {
            2
        } else {
            1
        };

        if windows && is_windows_lone_key(&self.code, &self.modifiers) {
            return Ok(DictationShortcutSetting {
                key_code: key_code_for_code(&self.code).unwrap_or(0),
                code: self.code,
                modifiers: self.modifiers,
                label: self.label,
                press_count,
            });
        }
        if windows && !self.modifiers.has_any() && is_windows_key_code(&self.code) {
            return Err(AppError::new(
                "dictation_shortcut_unsupported",
                "The Windows key alone opens Start. Use Ctrl, Alt, or Shift alone, or add a key.",
            ));
        }

        if is_bare_fn_input(&self.code, &self.modifiers) {
            if windows {
                return Err(AppError::new(
                    "dictation_shortcut_unsupported",
                    "The Fn key is not supported for Windows dictation shortcuts.",
                ));
            }
            return Ok(DictationShortcutSetting {
                press_count,
                label: "Fn".to_string(),
                ..DictationShortcutSetting::bare_fn()
            });
        }

        if is_modifier_only_input(&self.code, &self.modifiers) {
            if windows {
                return Err(AppError::new(
                    "dictation_shortcut_unsupported",
                    "Modifier-only shortcuts are not supported on Windows.",
                ));
            }
            return Ok(DictationShortcutSetting::modifier_only(self.modifiers));
        }

        if windows && self.modifiers.function {
            return Err(AppError::new(
                "dictation_shortcut_unsupported",
                "The Fn key is not supported for Windows dictation shortcuts.",
            ));
        }

        if windows
            && !self.modifiers.command
            && !self.modifiers.control
            && !self.modifiers.option
            && !self.modifiers.shift
        {
            return Err(AppError::new(
                "dictation_shortcut_modifier_required",
                "Shortcut must include at least one modifier key (Ctrl, Alt, Shift, Win).",
            ));
        }

        let key_code = key_code_for_platform(&self.code, windows).ok_or_else(|| {
            AppError::new(
                "dictation_shortcut_unsupported",
                "Shortcut must include a supported non-modifier key.",
            )
        })?;

        if !self.modifiers.has_any() {
            return Err(AppError::new(
                "dictation_shortcut_modifier_required",
                "Shortcut must include at least one modifier key.",
            ));
        }

        Ok(DictationShortcutSetting {
            key_code,
            code: self.code,
            modifiers: self.modifiers,
            label: self.label,
            press_count,
        })
    }
}

fn is_bare_fn_input(code: &str, modifiers: &DictationShortcutModifiers) -> bool {
    code.eq_ignore_ascii_case("Fn") && modifiers.function && no_standard_modifiers(modifiers)
}

fn is_modifier_only_input(code: &str, modifiers: &DictationShortcutModifiers) -> bool {
    code.eq_ignore_ascii_case("Modifiers")
        && modifiers.has_any()
        && (is_bare_fn_input("Fn", modifiers) || modifiers.count() >= 2)
}

fn modifier_only_label(modifiers: &DictationShortcutModifiers) -> String {
    [
        (modifiers.command, "Cmd"),
        (modifiers.control, "Ctrl"),
        (modifiers.option, "Opt"),
        (modifiers.shift, "Shift"),
        (modifiers.function, "Fn"),
    ]
    .into_iter()
    .filter_map(|(enabled, label)| enabled.then_some(label))
    .collect::<Vec<_>>()
    .join("+")
}

fn legacy_shortcut_setting(id: &str) -> Option<DictationShortcutSetting> {
    match id {
        "bare_fn" | "fn" => Some(DictationShortcutSetting::bare_fn()),
        "fn_space" => Some(DictationShortcutSetting {
            key_code: 0x31,
            code: "Space".to_string(),
            modifiers: DictationShortcutModifiers {
                function: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Fn+Space".to_string(),
            press_count: 1,
        }),
        "control_option_space" => Some(DictationShortcutSetting::control_option_t()),
        _ => None,
    }
}

fn no_standard_modifiers(modifiers: &DictationShortcutModifiers) -> bool {
    !modifiers.command && !modifiers.control && !modifiers.option && !modifiers.shift
}

fn default_shortcut_for(kind: DictationShortcutKind, windows: bool) -> DictationShortcutSetting {
    match kind {
        DictationShortcutKind::PushToTalk if windows => DictationShortcutSetting::right_ctrl(),
        DictationShortcutKind::PushToTalk => DictationShortcutSetting::control_option_d(),
        DictationShortcutKind::Toggle => DictationShortcutSetting::control_option_t(),
    }
}

/// Ctrl, Alt and Shift of either side: watched by the helper's hooks with the
/// Mac Fn gesture (hold alone to dictate, double tap to toggle).
const WINDOWS_LONE_MODIFIERS: [&str; 6] = [
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "AltRight",
    "ShiftLeft",
    "ShiftRight",
];

/// Keys that work alone as a Windows shortcut: the modifiers above and keys
/// no app uses on their own. Mirrors the helper's `solo_key::SOLO_KEYS`.
const WINDOWS_LONE_KEYS: [&str; 23] = [
    "ControlLeft",
    "ControlRight",
    "AltLeft",
    "AltRight",
    "ShiftLeft",
    "ShiftRight",
    "F13",
    "F14",
    "F15",
    "F16",
    "F17",
    "F18",
    "F19",
    "F20",
    "F21",
    "F22",
    "F23",
    "F24",
    "Pause",
    "ScrollLock",
    "Insert",
    "PrintScreen",
    "ContextMenu",
];

fn is_windows_lone_key(code: &str, modifiers: &DictationShortcutModifiers) -> bool {
    !modifiers.has_any() && WINDOWS_LONE_KEYS.contains(&code)
}

fn is_windows_lone_modifier(code: &str, modifiers: &DictationShortcutModifiers) -> bool {
    !modifiers.has_any() && WINDOWS_LONE_MODIFIERS.contains(&code)
}

/// The Mac key code of `code`, or on Windows `0` for a lone key that is not
/// a modifier (F13 to F24, Pause, Insert...): the helper also takes it in a
/// chord (Ctrl+F13), and it maps keys by `code`, not by Mac key code.
fn key_code_for_platform(code: &str, windows: bool) -> Option<u32> {
    key_code_for_code(code).or_else(|| {
        (windows && WINDOWS_LONE_KEYS.contains(&code) && !WINDOWS_LONE_MODIFIERS.contains(&code))
            .then_some(0)
    })
}

fn is_windows_key_code(code: &str) -> bool {
    matches!(code, "MetaLeft" | "MetaRight")
}

pub fn setup(app: &mut tauri::App) {
    app.manage(HudPosition {
        inner: Mutex::new(HudPositionState::default()),
    });
    app.manage(HudHoverState {
        stop_bounds: Mutex::new(None),
        last_hover: std::sync::atomic::AtomicBool::new(false),
        cancel_bounds: Mutex::new(None),
        last_cancel_hover: std::sync::atomic::AtomicBool::new(false),
        dismiss_bounds: Mutex::new(None),
        last_dismiss_hover: std::sync::atomic::AtomicBool::new(false),
        record_bounds: Mutex::new(None),
        last_record_hover: std::sync::atomic::AtomicBool::new(false),
    });
    app.manage(HudFrameLock::default());
    spawn_hud_hover_thread(app.handle().clone());
    if let Err(error) = configure_hud_window(app.handle()) {
        tracing::warn!(%error, "failed to configure dictation HUD");
    }
    manage_settings(app.handle());
    // Keys, token, and the transcription connection are ready before the
    // first press (ditado-rapido D4).
    crate::providers::transcription::warm_dictation_route_if_idle();
    app.manage(LastDictationEvent {
        event: Mutex::new(None),
    });
    app.manage(ShortcutActivationState {
        controller: Mutex::new(ShortcutActivationController::default()),
    });
    app.manage(DictationTakeState::default());

    // Install state before spawning so a helper that exits immediately can be
    // observed by its stdout supervisor instead of disappearing during setup.
    app.manage(HelperState::default());
    let helper_started = match spawn_helper(app.handle()) {
        Ok(helper) => {
            let helper_state = app.state::<HelperState>();
            matches!(
                store_respawned_helper(&helper_state, helper),
                StoreRespawnedHelper::Stored
            )
        }
        Err(error) => {
            tracing::warn!(
                error = %error.message,
                "failed to start dictation helper during setup; retrying in background"
            );
            false
        }
    };

    if helper_started {
        let settings = app.state::<DictationSettingsState>();
        let helper_state = app.state::<HelperState>();
        let settings = settings
            .settings
            .lock()
            .ok()
            .map(|settings| settings.clone());
        if let Some(settings) = settings {
            let _ = apply_microphone_setting(&helper_state, &settings.microphone);
            let _ = apply_shortcut_settings(&helper_state, &settings);
        }
    }

    let settings = app
        .state::<DictationSettingsState>()
        .settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    let helper_expected = cfg!(any(target_os = "macos", target_os = "windows"));
    let event = if helper_started || !helper_expected {
        initial_hotkey_event(&settings)
    } else {
        helper_unavailable_event("restarting", "Dictation is starting.")
    };
    app.manage(HotkeyStatus {
        event: Mutex::new(event.clone()),
    });
    let _ = app.emit("dictation-event", event.to_string());

    if helper_expected && !helper_started {
        let app_handle = app.handle().clone();
        thread::spawn(move || supervise_initial_helper_start(&app_handle));
    }
}

#[tauri::command]
pub fn dictation_capabilities() -> DictationCapabilitiesResponse {
    DictationCapabilitiesResponse {
        capabilities: DictationCapabilities {
            available: cfg!(any(target_os = "macos", target_os = "windows")),
            platform: if cfg!(target_os = "macos") {
                "macos"
            } else if cfg!(target_os = "windows") {
                "windows"
            } else {
                "unsupported"
            },
            shortcuts: cfg!(any(target_os = "macos", target_os = "windows")),
            paste: cfg!(any(target_os = "macos", target_os = "windows")),
            microphone_selection: cfg!(any(target_os = "macos", target_os = "windows")),
            accessibility_permission: cfg!(target_os = "macos"),
            system_audio: cfg!(target_os = "macos"),
        },
    }
}

#[tauri::command]
pub fn dictation_settings(
    state: State<'_, DictationSettingsState>,
) -> Result<DictationSettingsResponse, AppError> {
    let settings = state
        .settings
        .lock()
        .map_err(|_| AppError::new("dictation_settings_unavailable", "Settings lock failed."))?
        .clone();
    Ok(DictationSettingsResponse { settings })
}

#[tauri::command]
pub async fn list_dictation_history(
    app: AppHandle,
) -> Result<ListDictationHistoryResponse, AppError> {
    let profile = crate::commands::active_profile(&app);
    crate::commands::repositories(&app)
        .await?
        .list_dictation_history(&profile, 200)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn delete_dictation_history_item(app: AppHandle, id: String) -> Result<(), AppError> {
    let profile = crate::commands::active_profile(&app);
    let repos = crate::commands::repositories(&app).await?;
    delete_history_for_profile(&repos.pool, &profile, &id).await?;
    let _ = app.emit(
        "clovy-data-changed",
        serde_json::json!({"profile":profile,"kind":"dictations","id":id,"deleted":true}),
    );
    Ok(())
}

#[tauri::command]
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn set_dictation_shortcut(
    app: AppHandle,
    state: State<'_, DictationSettingsState>,
    helper_state: State<'_, HelperState>,
    _hotkey_status: State<'_, HotkeyStatus>,
    kind: DictationShortcutKind,
    shortcut: DictationShortcutInput,
) -> Result<DictationSettings, AppError> {
    let shortcut = shortcut.into_setting()?;
    let current_settings = state
        .settings
        .lock()
        .map_err(|_| AppError::new("dictation_settings_unavailable", "Settings lock failed."))?
        .clone();

    validate_shortcut_update(&current_settings, kind, &shortcut)?;

    let current_shortcut = match kind {
        DictationShortcutKind::PushToTalk => &current_settings.push_to_talk_shortcut,
        DictationShortcutKind::Toggle => &current_settings.toggle_shortcut,
    };

    if current_shortcut == &shortcut {
        #[cfg(target_os = "macos")]
        set_hotkey_status(&_hotkey_status, hotkey_ready_event(&current_settings));
        apply_shortcut_settings(&helper_state, &current_settings)?;
        return Ok(current_settings);
    }

    let settings = update_settings(&state, |settings| match kind {
        DictationShortcutKind::PushToTalk => settings.push_to_talk_shortcut = shortcut,
        DictationShortcutKind::Toggle => settings.toggle_shortcut = shortcut,
    })?;
    reset_shortcut_activation(&app);
    apply_shortcut_settings(&helper_state, &settings)?;

    #[cfg(target_os = "macos")]
    set_hotkey_status(&_hotkey_status, hotkey_ready_event(&settings));
    Ok(settings)
}

#[tauri::command]
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn set_dictation_shortcut(
    kind: DictationShortcutKind,
    shortcut: DictationShortcutInput,
) -> Result<DictationSettings, AppError> {
    let _ = (kind, shortcut);
    Err(AppError::new(
        "dictation_shortcut_unsupported",
        "Dictation shortcuts are not supported on this platform.",
    ))
}

#[tauri::command]
pub fn set_dictation_microphone(
    state: State<'_, DictationSettingsState>,
    helper_state: State<'_, HelperState>,
    id: Option<String>,
    name: Option<String>,
) -> Result<DictationSettings, AppError> {
    let settings = update_settings(&state, |settings| {
        settings.microphone = DictationMicrophoneSetting { id, name };
    })?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if helper_state
        .process
        .lock()
        .map(|process| process.is_none())
        .unwrap_or(true)
    {
        return Ok(settings);
    }

    apply_microphone_setting(&helper_state, &settings.microphone)?;
    Ok(settings)
}

#[tauri::command]
pub fn set_dictation_style(
    state: State<'_, DictationSettingsState>,
    style: DictationStyle,
) -> Result<DictationSettings, AppError> {
    update_settings(&state, |settings| {
        settings.style = style;
    })
}

#[tauri::command]
pub fn set_dictation_hud_placement(
    app: AppHandle,
    state: State<'_, DictationSettingsState>,
    placement: DictationHudPlacement,
) -> Result<DictationSettings, AppError> {
    let settings = update_settings(&state, |settings| {
        settings.hud_placement = placement;
    })?;
    if let Some(position) = app.try_state::<HudPosition>() {
        if let Ok(mut saved) = position.inner.lock() {
            saved.reset();
        }
    }
    // Apply on the next dictation show. The shared window may currently
    // contain a top-anchored meeting prompt, not dictation content.
    Ok(settings)
}

#[tauri::command]
pub fn set_dictation_cleanup(
    state: State<'_, DictationSettingsState>,
    enabled: bool,
    model: Option<String>,
) -> Result<DictationSettings, AppError> {
    let model = model
        .as_deref()
        .map(validate_dictation_cleanup_model)
        .transpose()?;
    update_settings(&state, |settings| {
        settings.cleanup_enabled = enabled;
        if let Some(model) = model {
            settings.cleanup_model = model;
        }
    })
}

#[tauri::command]
pub fn set_dictation_language(
    state: State<'_, DictationSettingsState>,
    language: Option<String>,
) -> Result<DictationSettings, AppError> {
    let language = normalize_language(language);
    update_settings(&state, |settings| {
        settings.language = language;
    })
}

fn validate_renderer_dictation_command(command: &serde_json::Value) -> Result<(), AppError> {
    if command.get("type").and_then(serde_json::Value::as_str) == Some("repeat_last_dictation") {
        return Err(AppError::new(
            "invalid_input",
            "Dictation reinsertion requires its native shortcut.",
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn dictation_helper_command(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, HelperState>,
    command: serde_json::Value,
) -> Result<(), AppError> {
    dictation_helper_command_with_authority(app, window, state, command, None)
}

fn dictation_helper_command_with_authority(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, HelperState>,
    command: serde_json::Value,
    authority: Option<&crate::domain::turn_authority::TurnAuthority>,
) -> Result<(), AppError> {
    validate_renderer_dictation_command(&command)?;
    if command.get("type").and_then(serde_json::Value::as_str) == Some("agent_composer_delivered") {
        if window.label() != "main" {
            return Err(AppError::new(
                "dictation_delivery_forbidden",
                "Only the destination window can acknowledge delivery.",
            ));
        }
        let mut active = state.agent_take.lock().map_err(|_| {
            AppError::new("dictation_unavailable", "Dictation state is unavailable.")
        })?;
        let take = active
            .as_mut()
            .filter(|take| {
                command.get("takeId").and_then(serde_json::Value::as_str)
                    == Some(take.take_id.as_str())
                    && command
                        .get("composerRequestId")
                        .and_then(serde_json::Value::as_str)
                        == Some(take.composer_request_id.as_str())
                    && take.phase == "deliveryClaimed"
            })
            .ok_or_else(|| {
                AppError::new(
                    "dictation_take_not_found",
                    "The composer delivery has expired.",
                )
            })?;
        take.phase = if command.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
            "delivered"
        } else {
            "failed"
        }
        .into();
        let take_id = take.take_id.clone();
        let delivered = take.phase == "delivered";
        if !delivered {
            take.error_code = Some("dictation_composer_delivery_failed".into());
            take.error=Some("The destination composer could not accept this dictation. Saved text remains in dictation history.".into());
        }
        drop(active);
        state.agent_changed.notify_waiters();
        let event = if delivered {
            serde_json::json!({"type":"paste_completed","payload":{"composerRequestId":command.get("composerRequestId")}})
        } else {
            app_error_event(AppError::new("dictation_composer_delivery_failed","The destination composer could not accept the dictation. Its saved text remains in dictation history."))
        };
        emit_dictation_take_id_event(&app, &take_id, event);
        return Ok(());
    }
    if let Some(request_id) = command.get("composerRequestId") {
        let valid = request_id
            .as_str()
            .map(str::trim)
            .is_some_and(|id| !id.is_empty() && id.len() <= 128);
        if !valid {
            return Err(AppError::new(
                "dictation_composer_request_invalid",
                "The dictation composer request was invalid.",
            ));
        }
    }
    if let Some(take_id) = command.get("takeId") {
        let valid = take_id
            .as_str()
            .map(str::trim)
            .is_some_and(|id| !id.is_empty() && id.len() <= 128);
        if !valid {
            return Err(AppError::new(
                "dictation_take_invalid",
                "The dictation take identifier was invalid.",
            ));
        }
    }
    let command_type = command
        .get("type")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    if matches!(
        command_type.as_str(),
        "register_composer_delivery" | "unregister_composer_delivery"
    ) {
        if window.label() != "main" {
            return Err(AppError::new(
                "dictation_composer_registration_forbidden",
                "Only Clovy's main window can register composer delivery.",
            ));
        }
        let request_id = command
            .get("composerRequestId")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AppError::new(
                    "dictation_composer_request_invalid",
                    "The dictation composer request was invalid.",
                )
            })?;
        let mut registration = state.composer_registration.lock().map_err(|_| {
            AppError::new(
                "dictation_composer_registration_unavailable",
                "The dictation composer registration was unavailable.",
            )
        })?;
        if command_type == "register_composer_delivery" {
            *registration = Some(ComposerDeliveryRegistration {
                request_id: request_id.to_string(),
                window_handle: webview_window_handle(&window)?,
            });
        } else if registration
            .as_ref()
            .is_some_and(|registration| registration.request_id == request_id)
        {
            *registration = None;
        }
        return Ok(());
    }
    if command.get("composerRequestId").is_some()
        && matches!(
            command_type.as_str(),
            "start_listening" | "toggle_listening"
        )
        && window.label() != "main"
    {
        return Err(AppError::new(
            "dictation_composer_registration_forbidden",
            "Only Clovy's main window can start composer delivery.",
        ));
    }
    #[cfg(target_os = "windows")]
    let command = {
        let mut command = command;
        if command.get("composerRequestId").is_some() {
            if let Some(object) = command.as_object_mut() {
                object.insert(
                    "clovyProcessId".into(),
                    serde_json::json!(std::process::id()),
                );
                object.insert(
                    // Compatibility for released Windows helpers.
                    "juneProcessId".into(),
                    serde_json::json!(std::process::id()),
                );
                object.insert(
                    "clovyWindowHandle".into(),
                    serde_json::json!(webview_window_handle(&window)?),
                );
                object.insert(
                    // Compatibility for released Windows helpers.
                    "juneWindowHandle".into(),
                    serde_json::json!(webview_window_handle(&window)?),
                );
            }
        }
        command
    };

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if state
        .process
        .lock()
        .map(|process| process.is_none())
        .unwrap_or(true)
    {
        return handle_missing_helper_command(&app, &command);
    }

    if matches!(
        command_type.as_str(),
        "start_listening" | "stop_and_paste" | "discard_recording" | "toggle_listening"
    ) {
        if let Some(take_state) = app.try_state::<DictationTakeState>() {
            let (_, result) = take_state.run_new_helper_command(|_| {
                if let Some(authority) = authority {
                    authority.check_live()?;
                }
                let current_take_id = take_state.current_take_id();
                let requested_take_id = command
                    .get("takeId")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string);
                let (starts_recording, take_id) = app
                    .try_state::<ShortcutActivationState>()
                    .and_then(|activation| {
                        activation.controller.lock().ok().map(|mut controller| {
                            controller.prepare_renderer_command(
                                &command_type,
                                requested_take_id.clone(),
                                current_take_id.clone(),
                            )
                        })
                    })
                    .unwrap_or_else(|| {
                        let starts_recording = matches!(
                            command_type.as_str(),
                            "start_listening" | "toggle_listening"
                        );
                        let take_id = requested_take_id.or_else(|| {
                            starts_recording
                                .then(|| uuid::Uuid::new_v4().to_string())
                                .or(current_take_id)
                        });
                        (starts_recording, take_id)
                    });
                if starts_recording {
                    admit_dictation_start(&app, take_id.as_deref())?;
                }

                let mut command = command;
                if let (Some(take_id), Some(object)) = (take_id.as_deref(), command.as_object_mut())
                {
                    object.insert("takeId".into(), serde_json::json!(take_id));
                }
                if command_type == "discard_recording" {
                    cancel_dictation_take(&app, take_id.as_deref());
                }
                send_helper_command_checked(&state, command, authority).inspect_err(|_| {
                    if starts_recording {
                        release_unsent_dictation_take(&app, take_id.as_deref());
                    }
                })
            });
            return result;
        }
        return send_helper_command_checked(&state, command, authority);
    }

    send_helper_command_checked(&state, command, authority)
}

#[cfg(target_os = "windows")]
fn webview_window_handle(window: &WebviewWindow) -> Result<isize, AppError> {
    window
        .hwnd()
        .map(|handle| handle.0 as isize)
        .map_err(|error| {
            AppError::new(
                "dictation_composer_window_unavailable",
                format!("Clovy could not identify the composer window: {error}"),
            )
        })
}

#[cfg(not(target_os = "windows"))]
fn webview_window_handle(_window: &WebviewWindow) -> Result<isize, AppError> {
    Ok(0)
}

#[cfg(target_os = "windows")]
fn is_foreground_window_handle(window_handle: isize) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;

    unsafe { GetForegroundWindow().0 as isize == window_handle }
}

#[cfg(not(target_os = "windows"))]
fn is_foreground_window_handle(_window_handle: isize) -> bool {
    false
}

fn helper_command_type_resets_shortcut_activation(command_type: &str) -> bool {
    matches!(
        command_type,
        "stop_and_paste" | "discard_recording" | "toggle_listening"
    )
}

#[tauri::command]
pub fn dictation_hotkey_status(state: State<'_, HotkeyStatus>) -> serde_json::Value {
    state
        .event
        .lock()
        .map(|event| event.clone())
        .unwrap_or_else(|_| hotkey_error_event("Hotkey status lock was poisoned.".to_string()))
}

#[tauri::command]
pub fn latest_dictation_event(state: State<'_, LastDictationEvent>) -> Option<String> {
    state.event.lock().ok().and_then(|event| event.clone())
}

/// Cursor position in logical points (top-left origin) read directly from
/// the window server. `WebviewWindow::cursor_position()` is not used here
/// because on macOS it only refreshes while the window is key, and the HUD
/// is a non-activating NSPanel.
#[cfg(target_os = "macos")]
fn cursor_position_via_cg() -> Option<(f64, f64)> {
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventCreate(source: *const std::ffi::c_void) -> *mut std::ffi::c_void;
        fn CGEventGetLocation(event: *const std::ffi::c_void) -> CGPoint;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *const std::ffi::c_void);
    }

    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return None;
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Some((point.x, point.y))
    }
}

#[tauri::command]
pub fn dictation_hud_set_stop_bounds(state: State<'_, HudHoverState>, rect: Option<HudClientRect>) {
    if let Ok(mut guard) = state.stop_bounds.lock() {
        *guard = rect;
    }
}

#[tauri::command]
pub fn dictation_hud_set_cancel_bounds(
    state: State<'_, HudHoverState>,
    rect: Option<HudClientRect>,
) {
    if let Ok(mut guard) = state.cancel_bounds.lock() {
        *guard = rect;
    }
}

#[tauri::command]
pub fn dictation_hud_set_dismiss_bounds(
    state: State<'_, HudHoverState>,
    rect: Option<HudClientRect>,
) {
    if let Ok(mut guard) = state.dismiss_bounds.lock() {
        *guard = rect;
    }
}

#[tauri::command]
pub fn dictation_hud_set_record_bounds(
    state: State<'_, HudHoverState>,
    rect: Option<HudClientRect>,
) {
    if let Ok(mut guard) = state.record_bounds.lock() {
        *guard = rect;
    }
}

#[tauri::command]
pub fn dictation_hud_preferred_error_placement(app: AppHandle) -> String {
    let Some(hud) = app.get_webview_window("hud") else {
        return "below".to_string();
    };
    let Ok(window_size) = hud.outer_size() else {
        return "below".to_string();
    };
    let Some(position) = current_or_next_hud_position(&app, &hud, window_size) else {
        return "below".to_string();
    };

    let center_x = position.x as f64 + window_size.width as f64 / 2.0;
    let center_y = position.y as f64 + window_size.height as f64 / 2.0;
    let monitor = hud
        .monitor_from_point(center_x, center_y)
        .ok()
        .flatten()
        .or_else(|| hud.current_monitor().ok().flatten())
        .or_else(|| hud.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return "below".to_string();
    };

    let work_area = monitor.work_area();
    let work_mid_y = work_area.position.y as f64 + work_area.size.height as f64 / 2.0;
    if center_y > work_mid_y {
        "above".to_string()
    } else {
        "below".to_string()
    }
}

/// Shows the macOS Esc hint in its own click-through panel. Keeping the hint
/// outside the Dictation HUD means its transparent vertical slot cannot
/// intercept clicks in the app underneath. Other platforms retain the in-DOM
/// fallback and return `false`.
#[tauri::command]
pub fn dictation_hud_set_tooltip_visible(
    app: AppHandle,
    visible: bool,
    placement: Option<String>,
) -> bool {
    #[cfg(target_os = "macos")]
    {
        if !visible {
            hide_dictation_tooltip(&app);
            return true;
        }

        let Some(hud) = app.get_webview_window("hud") else {
            hide_dictation_tooltip(&app);
            return true;
        };
        if !hud.is_visible().unwrap_or(false) {
            hide_dictation_tooltip(&app);
            return true;
        }
        let cancel_rect = app
            .try_state::<HudHoverState>()
            .and_then(|state| state.cancel_bounds.lock().ok()?.clone());
        let Some(cancel_rect) = cancel_rect else {
            hide_dictation_tooltip(&app);
            return true;
        };
        let Ok(hud_handle) = hud.ns_window() else {
            hide_dictation_tooltip(&app);
            return true;
        };
        if hud_handle.is_null() {
            hide_dictation_tooltip(&app);
            return true;
        }
        let placement = if placement.as_deref() == Some("above") {
            HudTooltipPlacement::Above
        } else {
            HudTooltipPlacement::Below
        };
        let hud_pointer = hud_handle as usize;
        let _ = app.run_on_main_thread(move || {
            show_dictation_tooltip(hud_pointer, &cancel_rect, placement);
        });
        true
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, visible, placement);
        false
    }
}

#[cfg(target_os = "macos")]
#[derive(Clone, Copy)]
enum HudTooltipPlacement {
    Above,
    Below,
}

#[cfg(target_os = "macos")]
fn hud_tooltip_origin(
    hud_frame_x: f64,
    hud_frame_y: f64,
    hud_content_height: f64,
    cancel_rect: &HudClientRect,
    placement: HudTooltipPlacement,
) -> (f64, f64) {
    let control_center_x = hud_frame_x + (cancel_rect.left + cancel_rect.right) / 2.0;
    let x = control_center_x - DICTATION_TOOLTIP_WIDTH_LOGICAL / 2.0;
    let control_top = hud_frame_y + hud_content_height - cancel_rect.top;
    let control_bottom = hud_frame_y + hud_content_height - cancel_rect.bottom;
    let y = match placement {
        HudTooltipPlacement::Above => control_top + DICTATION_TOOLTIP_GAP_LOGICAL,
        HudTooltipPlacement::Below => {
            control_bottom - DICTATION_TOOLTIP_GAP_LOGICAL - DICTATION_TOOLTIP_HEIGHT_LOGICAL
        }
    };
    (x, y)
}

/// Serializes window-frame motion (resize morphs, the error shake) so two
/// concurrent commands never fight over the window's position.
pub struct HudFrameLock(Mutex<()>);

impl Default for HudFrameLock {
    fn default() -> Self {
        Self(Mutex::new(()))
    }
}

/// Resize the HUD window to the pill's measured CSS size. The pill's width
/// varies by state (error text, meeting prompt), and with the frosted surface
/// being a window-filling NSVisualEffectView the window must track the pill
/// exactly. Re-anchors horizontally so the pill's center stays put; when the
/// window is visible the frame eases over rather than popping (the webview
/// crossfades the pill content around the same beat). Blocks until the motion
/// finishes so the webview can sequence its fade-in off the resolved invoke.
#[tauri::command]
pub fn dictation_hud_set_size(app: AppHandle, width: f64, height: f64, animate: bool) {
    let Some(hud) = app.get_webview_window("hud") else {
        return;
    };
    let lock = app.try_state::<HudFrameLock>();
    let _guard = lock.as_ref().and_then(|lock| lock.0.lock().ok());
    let Ok(scale) = hud.scale_factor() else {
        return;
    };
    let new_size = PhysicalSize::new(
        (width * scale).round() as u32,
        (height * scale).round() as u32,
    );
    animate_frame_to(&hud, new_size, animate);
}

/// Arm capture before handing the gesture to the native window server.
/// macOS returns immediately and may not send mouseUp; the existing HUD
/// poll commits the final frame once the session button state is released.
#[tauri::command]
pub fn dictation_hud_start_drag(app: AppHandle) {
    let Some(hud) = app.get_webview_window("hud") else {
        return;
    };
    let app_for_drag = app.clone();
    let _ = app.run_on_main_thread(move || {
        let (Ok(position), Some(state)) = (
            hud.outer_position(),
            app_for_drag.try_state::<HudPosition>(),
        ) else {
            return;
        };
        let Ok(mut saved) = state.inner.lock() else {
            return;
        };
        saved.begin_drag((position.x, position.y));
        drop(saved);
        if hud.start_dragging().is_err() {
            cancel_hud_drag(&hud);
        }
    });
}

/// Ease the window to `new_size`, keeping the nearest vertical edge anchored.
/// Hidden windows are positioned by show, not resized around their startup frame.
fn animate_frame_to(hud: &WebviewWindow, new_size: PhysicalSize<u32>, animate: bool) {
    cancel_hud_drag(hud);
    let (Ok(position), Ok(old_size)) = (hud.outer_position(), hud.outer_size()) else {
        return;
    };
    if new_size == old_size {
        return;
    }
    if !hud.is_visible().unwrap_or(false) {
        // Before the first show, the OS chooses an arbitrary initial frame.
        // Centering a resize there used to emit Moved and masquerade as a drag.
        let _ = hud.set_size(new_size);
        return;
    }
    let monitor = hud
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| hud.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let (target_x, target_y) =
        dictation_hud_resized_origin(*monitor.work_area(), position, old_size, new_size);

    if animate && hud.is_visible().unwrap_or(false) {
        const STEPS: u32 = 12;
        const STEP_MS: u64 = 12;
        for step in 1..STEPS {
            let t = f64::from(step) / f64::from(STEPS);
            // ease-out cubic — fast start, soft landing.
            let e = 1.0 - (1.0 - t).powi(3);
            let w = f64::from(old_size.width)
                + f64::from(new_size.width as i32 - old_size.width as i32) * e;
            let h = f64::from(old_size.height)
                + f64::from(new_size.height as i32 - old_size.height as i32) * e;
            let x = f64::from(position.x) + f64::from(target_x - position.x) * e;
            let y = f64::from(position.y) + f64::from(target_y - position.y) * e;
            let _ = hud.set_size(PhysicalSize::new(w.round() as u32, h.round() as u32));
            let _ = hud.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
            thread::sleep(Duration::from_millis(STEP_MS));
        }
    }
    let _ = hud.set_size(new_size);
    let _ = hud.set_position(PhysicalPosition::new(target_x, target_y));
}

fn dictation_hud_resized_origin(
    work_area: tauri::PhysicalRect<i32, u32>,
    position: PhysicalPosition<i32>,
    old_size: PhysicalSize<u32>,
    new_size: PhysicalSize<u32>,
) -> (i32, i32) {
    let x = position.x + (old_size.width as i32 - new_size.width as i32) / 2;
    let below_middle = position.y + old_size.height as i32 / 2
        > work_area.position.y + work_area.size.height as i32 / 2;
    let y = if below_middle {
        position.y + old_size.height as i32 - new_size.height as i32
    } else {
        position.y
    };
    let max_x = work_area.position.x + (work_area.size.width as i32 - new_size.width as i32).max(0);
    let max_y =
        work_area.position.y + (work_area.size.height as i32 - new_size.height as i32).max(0);
    (
        x.clamp(work_area.position.x, max_x),
        y.clamp(work_area.position.y, max_y),
    )
}

/// Set the native window alpha. The exit dissolve is driven from the webview
/// (rAF over ~160ms) but must fade the NSWindow itself — CSS opacity can't
/// touch the vibrancy view or the native shadow.
#[tauri::command]
pub fn dictation_hud_set_alpha(app: AppHandle, alpha: f64) {
    let Some(hud) = app.get_webview_window("hud") else {
        return;
    };
    let _ = app.run_on_main_thread(move || {
        set_window_alpha(&hud, alpha.clamp(0.0, 1.0));
    });
}

#[cfg(target_os = "macos")]
fn set_window_alpha(hud: &WebviewWindow, alpha: f64) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;
    let Ok(handle) = hud.ns_window() else {
        return;
    };
    if handle.is_null() {
        return;
    }
    unsafe {
        let window = handle as *mut AnyObject;
        let _: () = msg_send![window, setAlphaValue: alpha];
    }
}

#[cfg(not(target_os = "macos"))]
fn set_window_alpha(_hud: &WebviewWindow, _alpha: f64) {}

/// Show the HUD window. Invoked by the webview from showHud() once it has
/// measured the pill and resized the window to match — the window must never
/// become visible before that resize, or it flashes up at a stale frame
/// (bare frost, then a clipped pill) until the next visible-state resize.
///
/// `enter` marks a fresh meeting-prompt show: the window is placed at the
/// default top-center spot (the saved drag position is the dictation
/// pill's, not the prompt's) and — unless `animate` is false — slides down
/// from 8px above it while its alpha ramps up. The motion is native: a CSS
/// translate would slide the tinted card off the stationary window
/// chrome, flashing bare edges. Ignored when the window is already up
/// with content, where it would blink the visible pill. Blocks until the
/// motion settles; returns whether this was a fresh entrance.
#[tauri::command]
pub fn dictation_hud_show(app: AppHandle, enter: Option<bool>, animate: Option<bool>) -> bool {
    let Some(hud) = app.get_webview_window("hud") else {
        return false;
    };
    cancel_hud_drag(&hud);
    // A window woken at alpha 0 (wake_hud_window) reports visible but the
    // user can't see it — that still counts as a fresh entrance.
    let entering = enter.unwrap_or(false);
    let was_hidden = !hud.is_visible().unwrap_or(false)
        || HUD_WOKEN_FADED.load(std::sync::atomic::Ordering::Relaxed);
    if !entering {
        position_hud_window(&app, &hud);
    }
    HUD_WOKEN_FADED.store(false, std::sync::atomic::Ordering::Relaxed);

    if !(was_hidden && entering) {
        // Plain show. An interrupted exit fade may have left the native
        // alpha low; restore it.
        let alpha_hud = hud.clone();
        let _ = app.run_on_main_thread(move || set_window_alpha(&alpha_hud, 1.0));
        let _ = hud.show();
        return was_hidden;
    }

    // The entrance is the meeting prompt, which always starts top-center.
    // Compute the target directly and anchor the motion on it — window
    // setters apply asynchronously on the main thread, so set_position
    // followed by outer_position() races and can hand the motion a stale
    // anchor, landing the prompt wherever the pill last sat.
    let target = hud
        .outer_size()
        .ok()
        .and_then(|size| hud_position_for_placement(&hud, size, DictationHudPlacement::Top));
    let Some((x, y)) = target else {
        let alpha_hud = hud.clone();
        let _ = app.run_on_main_thread(move || set_window_alpha(&alpha_hud, 1.0));
        let _ = hud.show();
        return was_hidden;
    };

    if !animate.unwrap_or(true) {
        // Reduced motion: right spot, no slide.
        let _ = hud.set_position(PhysicalPosition::new(x, y));
        let alpha_hud = hud.clone();
        let _ = app.run_on_main_thread(move || set_window_alpha(&alpha_hud, 1.0));
        let _ = hud.show();
        return was_hidden;
    }

    // Hold the frame lock so a concurrent resize morph or shake can't
    // capture a mid-entrance position as its anchor.
    let lock = app.try_state::<HudFrameLock>();
    let _guard = lock.as_ref().and_then(|lock| lock.0.lock().ok());
    let offset = hud
        .scale_factor()
        .map(|scale| (HUD_ENTER_OFFSET_LOGICAL * scale).round() as i32)
        .unwrap_or(0);
    let _ = hud.set_position(PhysicalPosition::new(x, y - offset));
    {
        let alpha_hud = hud.clone();
        let _ = app.run_on_main_thread(move || set_window_alpha(&alpha_hud, 0.0));
    }
    let _ = hud.show();
    // Position and alpha land in one main-thread closure per frame so the
    // slide and the fade can't desync.
    const STEPS: u32 = 18;
    const STEP_MS: u64 = 12;
    for step in 1..STEPS {
        let t = f64::from(step) / f64::from(STEPS);
        // ease-out cubic — fast start, soft landing.
        let e = 1.0 - (1.0 - t).powi(3);
        let frame_y = f64::from(y) - f64::from(offset) * (1.0 - e);
        let frame_hud = hud.clone();
        let frame = PhysicalPosition::new(x, frame_y.round() as i32);
        let _ = app.run_on_main_thread(move || {
            let _ = frame_hud.set_position(frame);
            set_window_alpha(&frame_hud, e);
        });
        thread::sleep(Duration::from_millis(STEP_MS));
    }
    let _ = hud.set_position(PhysicalPosition::new(x, y));
    let alpha_hud = hud.clone();
    let _ = app.run_on_main_thread(move || set_window_alpha(&alpha_hud, 1.0));
    was_hidden
}

/// Logical pixels the meeting prompt travels during its native entrance
/// and exit motion.
const HUD_ENTER_OFFSET_LOGICAL: f64 = 8.0;

/// Keep the HUD window on frostless chrome. The webview paints its own CSS
/// surface and shadow into a transparent, click-through gutter, matching the
/// agent HUD and avoiding native vibrancy rims or end-of-exit repaint flashes.
#[tauri::command]
pub fn dictation_hud_set_chrome(app: AppHandle, _frostless: bool) {
    let Some(hud) = app.get_webview_window("hud") else {
        return;
    };
    let _ = hud.set_shadow(false);
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::clear_vibrancy;
        if let Err(error) = clear_vibrancy(&hud) {
            tracing::warn!(%error, "failed to clear dictation HUD vibrancy");
        }
    }
}

/// Native exit for the meeting prompt: the mirror of the entrance — the
/// window slides up toward the top edge while its alpha ramps down, then
/// hides. The frame and alpha are restored afterwards so the saved drag
/// position doesn't creep upward and the next show starts clean. Blocks
/// until hidden.
#[tauri::command]
pub fn dictation_hud_exit(app: AppHandle) {
    hide_dictation_tooltip(&app);
    let Some(hud) = app.get_webview_window("hud") else {
        return;
    };
    cancel_hud_drag(&hud);
    if !hud.is_visible().unwrap_or(false) {
        return;
    }
    let lock = app.try_state::<HudFrameLock>();
    let _guard = lock.as_ref().and_then(|lock| lock.0.lock().ok());
    let (Ok(base), Ok(scale)) = (hud.outer_position(), hud.scale_factor()) else {
        let _ = hud.hide();
        return;
    };
    let offset = (HUD_ENTER_OFFSET_LOGICAL * scale).round() as i32;
    // Long enough to read as a fade, matching the entrance; position and
    // alpha land in one main-thread closure per frame so they can't
    // desync.
    const STEPS: u32 = 18;
    const STEP_MS: u64 = 12;
    for step in 1..STEPS {
        let t = f64::from(step) / f64::from(STEPS);
        let e = 1.0 - (1.0 - t).powi(3);
        let y = f64::from(base.y) - f64::from(offset) * e;
        let frame_hud = hud.clone();
        let frame = PhysicalPosition::new(base.x, y.round() as i32);
        let _ = app.run_on_main_thread(move || {
            let _ = frame_hud.set_position(frame);
            set_window_alpha(&frame_hud, 1.0 - e);
        });
        thread::sleep(Duration::from_millis(STEP_MS));
    }
    let _ = hud.hide();
    let _ = hud.set_position(base);
    let alpha_hud = hud.clone();
    let _ = app.run_on_main_thread(move || set_window_alpha(&alpha_hud, 1.0));
}

/// Error-state shake. Pre-vibrancy this was a CSS translateX on the pill, but
/// the pill now fills the window — a CSS nudge would slide the tint off the
/// stationary frost. Wobble the window itself instead, tracing the same
/// decaying curve as the old `hud-shake` keyframes.
#[tauri::command]
pub fn dictation_hud_shake(app: AppHandle) {
    let Some(hud) = app.get_webview_window("hud") else {
        return;
    };
    thread::spawn(move || {
        // Hold the frame lock so a concurrent resize morph can't capture a
        // mid-wobble position as its anchor (and vice versa).
        let lock = app.try_state::<HudFrameLock>();
        let _guard = lock.as_ref().and_then(|lock| lock.0.lock().ok());
        cancel_hud_drag(&hud);
        const DURATION_MS: f64 = 380.0;
        const FRAME_MS: u64 = 16;
        // (progress, logical x offset) — the old CSS keyframes.
        const CURVE: [(f64, f64); 8] = [
            (0.0, 0.0),
            (0.12, -3.0),
            (0.28, 3.0),
            (0.44, -2.0),
            (0.60, 2.0),
            (0.76, -1.0),
            (0.90, 1.0),
            (1.0, 0.0),
        ];
        let (Ok(base), Ok(scale)) = (hud.outer_position(), hud.scale_factor()) else {
            return;
        };
        let mut elapsed = 0.0;
        while elapsed < DURATION_MS {
            let t = elapsed / DURATION_MS;
            let offset = CURVE
                .windows(2)
                .find(|w| t >= w[0].0 && t <= w[1].0)
                .map(|w| {
                    let (t0, x0) = w[0];
                    let (t1, x1) = w[1];
                    x0 + (x1 - x0) * ((t - t0) / (t1 - t0))
                })
                .unwrap_or(0.0);
            let x = base.x + (offset * scale).round() as i32;
            let _ = hud.set_position(PhysicalPosition::new(x, base.y));
            thread::sleep(Duration::from_millis(FRAME_MS));
            elapsed += FRAME_MS as f64;
        }
        let _ = hud.set_position(base);
    });
}

fn rect_contains(
    rect: &HudClientRect,
    position: PhysicalPosition<i32>,
    scale_factor: f64,
    cx: f64,
    cy: f64,
) -> bool {
    let left = position.x as f64 + rect.left * scale_factor;
    let right = position.x as f64 + rect.right * scale_factor;
    let top = position.y as f64 + rect.top * scale_factor;
    let bottom = position.y as f64 + rect.bottom * scale_factor;
    cx >= left && cx <= right && cy >= top && cy <= bottom
}

/// Polls the cursor against the cached control bounds (stop button, cancel
/// button, meeting dismiss) and emits hover state changes. Short-circuits when no bounds
/// are registered.
/// Hover intent for the Esc tooltip, in 33ms polls (~330ms). The cancel X's own
/// hover event is immediate; only this teaching affordance waits for intent.
const CANCEL_TOOLTIP_IN_POLLS: u32 = 10;
/// Grace before the Esc tooltip closes, in polls (~130ms). The X's own hover
/// still clears immediately.
const CANCEL_TOOLTIP_OUT_POLLS: u32 = 4;

#[derive(Default)]
struct CancelTooltipIntent {
    inside_streak: u32,
    outside_streak: u32,
    hovered: bool,
}

impl CancelTooltipIntent {
    fn update(&mut self, inside: bool) -> Option<bool> {
        if inside {
            self.inside_streak = self.inside_streak.saturating_add(1);
            self.outside_streak = 0;
        } else {
            self.outside_streak = self.outside_streak.saturating_add(1);
            self.inside_streak = 0;
        }

        let next = if self.hovered {
            self.outside_streak < CANCEL_TOOLTIP_OUT_POLLS
        } else {
            self.inside_streak >= CANCEL_TOOLTIP_IN_POLLS
        };
        if next == self.hovered {
            return None;
        }

        self.hovered = next;
        Some(next)
    }

    fn reset(&mut self) -> Option<bool> {
        self.inside_streak = 0;
        self.outside_streak = 0;
        if !self.hovered {
            return None;
        }
        self.hovered = false;
        Some(false)
    }
}

fn spawn_hud_hover_thread(app: AppHandle) {
    thread::spawn(move || {
        let tick = Duration::from_millis(33);
        let mut cancel_tooltip_intent = CancelTooltipIntent::default();
        loop {
            thread::sleep(tick);

            let hover_state = match app.try_state::<HudHoverState>() {
                Some(state) => state,
                None => continue,
            };

            let Some(hud) = app.get_webview_window("hud") else {
                continue;
            };
            poll_hud_drag(&app, &hud);
            let visible = hud.is_visible().unwrap_or(false);

            let stop_rect = hover_state.stop_bounds.lock().ok().and_then(|g| g.clone());
            let cancel_rect = hover_state
                .cancel_bounds
                .lock()
                .ok()
                .and_then(|g| g.clone());
            let dismiss_rect = hover_state
                .dismiss_bounds
                .lock()
                .ok()
                .and_then(|g| g.clone());
            let record_rect = hover_state
                .record_bounds
                .lock()
                .ok()
                .and_then(|g| g.clone());

            // Hidden or no hoverable controls on screen: drop stale hover.
            if !visible
                || (stop_rect.is_none()
                    && cancel_rect.is_none()
                    && dismiss_rect.is_none()
                    && record_rect.is_none())
            {
                if hover_state.last_hover.swap(false, Ordering::Relaxed) {
                    let _ = app.emit("hud-stop-hover", false);
                }
                if hover_state.last_cancel_hover.swap(false, Ordering::Relaxed) {
                    let _ = app.emit("hud-cancel-hover", false);
                }
                if let Some(hovered) = cancel_tooltip_intent.reset() {
                    let _ = app.emit("hud-cancel-tooltip-hover", hovered);
                }
                if hover_state
                    .last_dismiss_hover
                    .swap(false, Ordering::Relaxed)
                {
                    let _ = app.emit("hud-dismiss-hover", false);
                }
                if hover_state.last_record_hover.swap(false, Ordering::Relaxed) {
                    let _ = app.emit("hud-record-hover", false);
                }
                continue;
            }

            let (Ok(position), Ok(scale_factor)) = (hud.outer_position(), hud.scale_factor())
            else {
                continue;
            };

            #[cfg(target_os = "macos")]
            let cursor =
                cursor_position_via_cg().map(|(x, y)| (x * scale_factor, y * scale_factor));
            #[cfg(not(target_os = "macos"))]
            let cursor = hud.cursor_position().ok().map(|p| (p.x, p.y));

            let Some((cx, cy)) = cursor else { continue };

            let stop_hovered = match stop_rect.as_ref() {
                Some(rect) => rect_contains(rect, position, scale_factor, cx, cy),
                None => false,
            };
            if stop_hovered != hover_state.last_hover.load(Ordering::Relaxed) {
                hover_state
                    .last_hover
                    .store(stop_hovered, Ordering::Relaxed);
                let _ = app.emit("hud-stop-hover", stop_hovered);
            }

            let cancel_inside = match cancel_rect.as_ref() {
                Some(rect) => rect_contains(rect, position, scale_factor, cx, cy),
                None => false,
            };
            let last_cancel = hover_state.last_cancel_hover.load(Ordering::Relaxed);
            if cancel_inside != last_cancel {
                hover_state
                    .last_cancel_hover
                    .store(cancel_inside, Ordering::Relaxed);
                let _ = app.emit("hud-cancel-hover", cancel_inside);
            }
            if let Some(hovered) = cancel_tooltip_intent.update(cancel_inside) {
                let _ = app.emit("hud-cancel-tooltip-hover", hovered);
            }

            let dismiss_hovered = match dismiss_rect.as_ref() {
                Some(rect) => rect_contains(rect, position, scale_factor, cx, cy),
                None => false,
            };
            if dismiss_hovered != hover_state.last_dismiss_hover.load(Ordering::Relaxed) {
                hover_state
                    .last_dismiss_hover
                    .store(dismiss_hovered, Ordering::Relaxed);
                let _ = app.emit("hud-dismiss-hover", dismiss_hovered);
            }

            let record_hovered = match record_rect.as_ref() {
                Some(rect) => rect_contains(rect, position, scale_factor, cx, cy),
                None => false,
            };
            if record_hovered != hover_state.last_record_hover.load(Ordering::Relaxed) {
                hover_state
                    .last_record_hover
                    .store(record_hovered, Ordering::Relaxed);
                let _ = app.emit("hud-record-hover", record_hovered);
            }
        }
    });
}

pub fn stop_helper(app: &AppHandle) {
    cancel_current_dictation_take(app);
    let state = app.state::<HelperState>();
    // Mark the teardown as intentional before killing so the supervisor thread
    // (woken by the resulting stdout EOF) skips respawning instead of fighting
    // app quit.
    state.shutting_down.store(true, Ordering::SeqCst);
    // Take the helper out under a bounded lock, then stop it outside the
    // critical section. A command thread stuck while writing to the helper
    // must not hold app shutdown forever.
    let helper = match crate::shutdown::try_lock_for(&state.process, HELPER_SHUTDOWN_MUTEX_TIMEOUT)
    {
        Some(mut guard) => guard.take(),
        None => {
            tracing::warn!(
                timeout_ms = HELPER_SHUTDOWN_MUTEX_TIMEOUT.as_millis(),
                "dictation helper shutdown timed out acquiring the process lock"
            );
            None
        }
    };
    if let Some(helper) = helper {
        abandon_helper(helper);
    }
    // A command thread can be blocked writing to a wedged helper while it owns
    // `state.process`, or the child handle can race out of the slot before the
    // bounded reaper confirms exit. Dropping the app in either state detaches
    // the child and leaves its global event tap alive. On macOS the per-instance
    // ownership record gives us a lock-independent handle to the exact helper
    // this process spawned. Always verify that record after the normal path so
    // shutdown does not return until the helper is gone (or ownership fails
    // closed). This also removes the now-stale record after a clean exit.
    #[cfg(target_os = "macos")]
    if !reap_current_owned_helper_without_process_lock(app) {
        tracing::warn!("dictation helper shutdown fallback could not confirm the helper exited");
    }
}

#[cfg(target_os = "macos")]
fn decide_owned_shutdown_action(
    recorded_app_pid: u32,
    current_app_pid: u32,
    owner: OwnerLiveness,
    helper: HelperMatch,
) -> RecordAction {
    if recorded_app_pid != current_app_pid || owner != OwnerLiveness::Alive {
        return RecordAction::Abort;
    }
    match helper {
        HelperMatch::MatchesRecord => RecordAction::Reap,
        HelperMatch::GoneOrReused => RecordAction::DeleteStale,
        HelperMatch::Unknown => RecordAction::Abort,
    }
}

/// Reaps this app instance's helper without acquiring [`HelperState::process`].
///
/// This is only a shutdown fallback for a process lock held by a blocked helper
/// write. The ownership record is keyed to the current app pid, and both the app
/// and helper start times are revalidated before the helper is signalled.
#[cfg(target_os = "macos")]
fn reap_current_owned_helper_without_process_lock(app: &AppHandle) -> bool {
    let Some(path) = helper_ownership_path(app) else {
        return true;
    };
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return true,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "dictation shutdown fallback could not read its ownership record"
            );
            return false;
        }
    };
    let record = match serde_json::from_str::<HelperOwnershipRecord>(&raw) {
        Ok(record) => record,
        Err(error) => {
            tracing::warn!(
                path = %path.display(),
                %error,
                "dictation shutdown fallback found an invalid ownership record"
            );
            return false;
        }
    };
    let owner = owner_liveness(&probe_start_time(record.app_pid), &record.app_start);
    let helper = helper_match(
        &probe_start_time(record.helper_pid),
        &record.helper_start,
        &inspect_helper_identity(record.helper_pid),
    );
    match decide_owned_shutdown_action(record.app_pid, std::process::id(), owner, helper) {
        RecordAction::Reap => {
            kill_pid(record.helper_pid);
            if !wait_for_pid_exit(record.helper_pid, HELPER_ORPHAN_EXIT_TIMEOUT) {
                tracing::warn!(
                    helper_pid = record.helper_pid,
                    "dictation shutdown fallback helper would not exit"
                );
                return false;
            }
            let _ = fs::remove_file(path);
            true
        }
        RecordAction::DeleteStale => {
            let _ = fs::remove_file(path);
            true
        }
        RecordAction::Skip | RecordAction::Abort => {
            tracing::warn!(
                app_pid = record.app_pid,
                helper_pid = record.helper_pid,
                "dictation shutdown fallback could not validate helper ownership"
            );
            false
        }
    }
}

pub(crate) fn dictation_helper_pid(app: &AppHandle) -> Option<u32> {
    app.try_state::<HelperState>().and_then(|state| {
        state
            .process
            .lock()
            .ok()?
            .as_ref()
            .map(|process| process.child.id())
    })
}

pub(crate) fn write_helper_command(
    writer: &mut impl Write,
    command: &serde_json::Value,
    authority: Option<&crate::domain::turn_authority::TurnAuthority>,
) -> Result<(), AppError> {
    let mut line = serde_json::to_string(command)
        .map_err(|error| AppError::new("dictation_helper_command_invalid", error.to_string()))?;
    line.push('\n');
    if let Some(authority) = authority {
        authority.check_live()?;
    }
    writer
        .write_all(line.as_bytes())
        .map_err(|error| AppError::new("dictation_helper_write_failed", error.to_string()))?;
    writer
        .flush()
        .map_err(|error| AppError::new("dictation_helper_write_failed", error.to_string()))
}

fn send_helper_command(state: &HelperState, command: serde_json::Value) -> Result<(), AppError> {
    send_helper_command_checked(state, command, None)
}
fn send_helper_command_checked(
    state: &HelperState,
    command: serde_json::Value,
    authority: Option<&crate::domain::turn_authority::TurnAuthority>,
) -> Result<(), AppError> {
    let mut guard = state
        .process
        .lock()
        .map_err(|_| AppError::new("dictation_helper_unavailable", "Helper lock failed."))?;
    let process = guard.as_mut().ok_or_else(|| {
        AppError::new(
            "dictation_helper_unavailable",
            "Dictation helper process is not running.",
        )
    })?;
    write_helper_command(&mut process.stdin, &command, authority)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn handle_missing_helper_command(
    app: &AppHandle,
    command: &serde_json::Value,
) -> Result<(), AppError> {
    match command.get("type").and_then(serde_json::Value::as_str) {
        Some(
            "get_permission_status"
            | "request_microphone_permission"
            | "request_accessibility_permission",
        ) => {
            emit_dictation_event_value(app, non_macos_permission_status_event());
            Ok(())
        }
        Some("list_microphones") => {
            emit_dictation_event_value(
                app,
                serde_json::json!({
                    "type": "microphone_devices",
                    "payload": {
                        "devices": [],
                        "selectedID": "",
                    },
                }),
            );
            Ok(())
        }
        Some(
            "cancel_shortcut_capture"
            | "start_shortcut_capture"
            | "set_microphone"
            | "discard_mic_test",
        ) => Ok(()),
        _ => Err(AppError::new(
            "dictation_helper_unavailable",
            "Dictation recording is not supported on this platform.",
        )),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn non_macos_permission_status_event() -> serde_json::Value {
    let (microphone, _) = crate::audio::capture::microphone_permission_state();
    serde_json::json!({
        "type": "permission_status",
        "payload": {
            "microphone": microphone,
            "accessibility": "granted",
        },
    })
}

fn apply_microphone_setting(
    helper_state: &HelperState,
    microphone: &DictationMicrophoneSetting,
) -> Result<(), AppError> {
    send_helper_command(
        helper_state,
        serde_json::json!({
            "type": "set_microphone",
            "id": microphone.id,
            "name": microphone.name,
        }),
    )
}

fn apply_shortcut_setting(
    helper_state: &HelperState,
    kind: DictationShortcutKind,
    shortcut: &DictationShortcutSetting,
) -> Result<(), AppError> {
    send_helper_command(
        helper_state,
        serde_json::json!({
            "type": "set_shortcut",
            "shortcut": {
                "keyCode": shortcut.key_code,
                "code": shortcut.code,
                "label": shortcut.label,
                "kind": kind,
                "pressCount": shortcut.press_count,
                "modifiers": shortcut.modifiers,
            },
        }),
    )
}

fn apply_shortcut_settings(
    helper_state: &HelperState,
    settings: &DictationSettings,
) -> Result<(), AppError> {
    apply_shortcut_setting(
        helper_state,
        DictationShortcutKind::PushToTalk,
        &settings.push_to_talk_shortcut,
    )?;
    apply_shortcut_setting(
        helper_state,
        DictationShortcutKind::Toggle,
        &settings.toggle_shortcut,
    )
}

fn validate_shortcut_update(
    settings: &DictationSettings,
    kind: DictationShortcutKind,
    shortcut: &DictationShortcutSetting,
) -> Result<(), AppError> {
    let other = match kind {
        DictationShortcutKind::PushToTalk => &settings.toggle_shortcut,
        DictationShortcutKind::Toggle => &settings.push_to_talk_shortcut,
    };

    if shortcut.same_trigger_as(other) {
        return Err(AppError::new(
            "dictation_shortcut_duplicate",
            "Push-to-talk and toggle dictation shortcuts must be different.",
        ));
    }

    Ok(())
}

fn handle_shortcut_key_event(
    app: &AppHandle,
    event_type: Option<&str>,
    event: Option<&serde_json::Value>,
) {
    let Some(event_type) = event_type else {
        return;
    };

    let Some(edge) = shortcut_key_edge(event_type) else {
        return;
    };
    if edge == ShortcutKeyEdge::Down {
        // A pooled connection idle for over a minute is reopened while the
        // user speaks, not after release.
        crate::providers::transcription::warm_dictation_route_if_idle();
    }

    let Some(kind) = shortcut_kind_from_event(event) else {
        reset_shortcut_activation(app);
        return;
    };

    let Some(settings) = current_dictation_settings(app) else {
        reset_shortcut_activation(app);
        return;
    };

    let shortcut = match kind {
        DictationShortcutKind::PushToTalk => settings.push_to_talk_shortcut,
        DictationShortcutKind::Toggle => settings.toggle_shortcut,
    };

    let prepare_action = || {
        app.try_state::<ShortcutActivationState>()
            .and_then(|state| {
                state.controller.lock().ok().and_then(|mut state| {
                    let now = Instant::now();
                    let held = state.push_started_at.map(|at| now.duration_since(at));
                    let command = state.handle_edge(edge, kind, now);
                    append_dictation_event_log(
                        app,
                        Some("shortcut_edge"),
                        &shortcut_edge_log_event(kind, edge, command, held),
                    );
                    let command = command?;
                    let starts_recording = state.command_starts_recording(command);
                    let command_take_id = if starts_recording {
                        Some(uuid::Uuid::new_v4().to_string())
                    } else {
                        state
                            .pending_helper_take_id
                            .clone()
                            .or_else(|| state.helper_take_id.clone())
                    };
                    if let Some(take_id) = command_take_id.as_deref().filter(|_| starts_recording) {
                        state.prepare_helper_start(take_id);
                    }
                    Some((command, starts_recording, command_take_id))
                })
            })
    };

    if let Some(take_state) = app.try_state::<DictationTakeState>() {
        take_state.run_new_helper_command_if(prepare_action, |_, action| {
            let (command, starts_recording, command_take_id) = action;
            let command_take_id = command_take_id.or_else(|| take_state.current_take_id());
            send_dictation_command(
                app,
                command,
                &shortcut.label,
                starts_recording,
                command_take_id,
            );
        });
        return;
    }

    if let Some((command, starts_recording, command_take_id)) = prepare_action() {
        send_dictation_command(
            app,
            command,
            &shortcut.label,
            starts_recording,
            command_take_id,
        );
    }
}

fn shortcut_kind_from_event(event: Option<&serde_json::Value>) -> Option<DictationShortcutKind> {
    let kind = event?
        .get("payload")
        .and_then(|payload| payload.get("kind"))
        .and_then(serde_json::Value::as_str)?;
    match kind {
        "push_to_talk" => Some(DictationShortcutKind::PushToTalk),
        "toggle" => Some(DictationShortcutKind::Toggle),
        _ => None,
    }
}

fn current_dictation_settings(app: &AppHandle) -> Option<DictationSettings> {
    app.try_state::<DictationSettingsState>()
        .and_then(|state| state.settings.lock().ok().map(|settings| settings.clone()))
}

fn reset_shortcut_activation(app: &AppHandle) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.reset();
        }
    }
}

fn dictation_take_id_from_event(event: &serde_json::Value) -> Option<&str> {
    event
        .get("payload")
        .and_then(|payload| payload.get("takeId"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.len() <= 128)
}

fn dictation_discard_reason_from_event(event: &serde_json::Value) -> Option<&str> {
    event
        .get("payload")
        .and_then(|payload| payload.get("reason"))
        .and_then(serde_json::Value::as_str)
}

fn helper_discard_cancellation_target(event: &serde_json::Value) -> Option<Option<&str>> {
    let take_id = dictation_take_id_from_event(event);
    // New helpers include a reason on terminal events that do not belong to
    // an active take. Do not let that idle echo cancel crash-recovery work
    // still owned by Rust. A reasonless event retains legacy compatibility.
    (take_id.is_some() || dictation_discard_reason_from_event(event).is_none()).then_some(take_id)
}

fn pending_helper_start_take_id(app: &AppHandle) -> Option<String> {
    app.try_state::<ShortcutActivationState>()
        .and_then(|state| {
            state
                .controller
                .lock()
                .ok()
                .and_then(|controller| controller.pending_helper_take_id.clone())
        })
}

fn malformed_recording_ready_cleanup(
    app: &AppHandle,
    event: &serde_json::Value,
) -> MalformedRecordingReadyCleanup {
    let event_take_id = dictation_take_id_from_event(event);
    let current_take_id = app
        .try_state::<DictationTakeState>()
        .and_then(|state| state.current_take_id());
    app.try_state::<ShortcutActivationState>()
        .and_then(|state| {
            state.controller.lock().ok().map(|controller| {
                controller
                    .malformed_recording_ready_cleanup(event_take_id, current_take_id.as_deref())
            })
        })
        .unwrap_or_else(|| MalformedRecordingReadyCleanup {
            take_id: event_take_id.map(str::to_string).or(current_take_id),
            send_helper_discard: true,
        })
}

fn dictation_start_take_id(event: &serde_json::Value, pending_take_id: Option<&str>) -> String {
    dictation_take_id_from_event(event)
        .or(pending_take_id)
        .map(str::to_string)
        .unwrap_or_else(|| format!("legacy-{}", uuid::Uuid::new_v4()))
}

fn begin_dictation_take(app: &AppHandle, event: &serde_json::Value) -> Option<String> {
    let state = app.try_state::<DictationTakeState>()?;
    // Older helpers omit the ID that Rust already sent with the start
    // command. Keep that identity for the cancellation token.
    let id = dictation_start_take_id(event, pending_helper_start_take_id(app).as_deref());
    state.begin(id.clone());
    Some(id)
}

fn cancel_dictation_take(app: &AppHandle, take_id: Option<&str>) {
    if let Some(state) = app.try_state::<DictationTakeState>() {
        state.cancel(take_id);
    }
}

fn cancel_current_dictation_take(app: &AppHandle) {
    cancel_dictation_take(app, None);
}

fn acknowledge_shortcut_toggle(app: &AppHandle) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.acknowledge_toggle_command();
        }
    }
}

fn update_shortcut_helper_finalizing(app: &AppHandle, finalizing: bool) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            if finalizing {
                controller.mark_helper_finalizing();
            } else {
                controller.clear_helper_finalizing();
            }
        }
    }
}

fn promote_untagged_shortcut_helper_start(app: &AppHandle, take_id: &str) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.promote_untagged_helper_start(take_id);
        }
    }
}

fn update_shortcut_helper_listening(app: &AppHandle, listening: bool, take_id: Option<&str>) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.set_helper_listening(listening, take_id);
        }
    }
}

fn clear_shortcut_helper_take(app: &AppHandle) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.clear_helper_take();
        }
    }
}

fn clear_shortcut_helper_protocol(app: &AppHandle) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.clear_helper_protocol();
        }
    }
}

fn reset_shortcut_for_helper_discard(
    app: &AppHandle,
    event: &serde_json::Value,
) -> HelperLifecycleDisposition {
    let take_id = dictation_take_id_from_event(event);
    let reason = dictation_discard_reason_from_event(event);
    let current_take_id = app
        .try_state::<DictationTakeState>()
        .and_then(|state| state.current_take_id());
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            return controller.reset_for_helper_discard(
                take_id,
                reason,
                current_take_id.as_deref(),
            );
        }
    }
    HelperLifecycleDisposition::Ignore
}

fn finish_shortcut_for_helper_event(
    app: &AppHandle,
    event: &serde_json::Value,
) -> HelperLifecycleDisposition {
    let take_id = dictation_take_id_from_event(event);
    let current_take_id = app
        .try_state::<DictationTakeState>()
        .and_then(|state| state.current_take_id());
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            return controller.finish_for_helper_event(take_id, current_take_id.as_deref());
        }
    }
    if take_id.is_some() && current_take_id.as_deref() != take_id {
        HelperLifecycleDisposition::Ignore
    } else {
        HelperLifecycleDisposition::Apply
    }
}

fn helper_event_matches_active_take(app: &AppHandle, event: &serde_json::Value) -> bool {
    let Some(take_id) = dictation_take_id_from_event(event) else {
        return true;
    };
    let current_take_id = app
        .try_state::<DictationTakeState>()
        .and_then(|state| state.current_take_id());
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(controller) = state.controller.lock() {
            return controller.matches_helper_take(take_id, current_take_id.as_deref());
        }
    }
    current_take_id.as_deref() == Some(take_id)
}

fn dictation_delivery_matches_active_take(app: &AppHandle, take_id: &str) -> bool {
    let current_take_id = app
        .try_state::<DictationTakeState>()
        .and_then(|state| state.current_take_id());
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(controller) = state.controller.lock() {
            return controller.matches_delivery_take(take_id, current_take_id.as_deref());
        }
    }
    current_take_id.as_deref() == Some(take_id)
}

fn helper_audio_level_matches_active_take(app: &AppHandle, event: &serde_json::Value) -> bool {
    let Some(take_id) = dictation_take_id_from_event(event) else {
        return true;
    };
    app.try_state::<ShortcutActivationState>()
        .and_then(|state| {
            state.controller.lock().ok().map(|controller| {
                controller.helper_is_listening
                    && controller.helper_take_id.as_deref() == Some(take_id)
            })
        })
        .unwrap_or(false)
}

fn note_helper_take_protocol(app: &AppHandle, event: Option<&serde_json::Value>) {
    if event.and_then(dictation_take_id_from_event).is_none() {
        return;
    }
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.helper_uses_take_ids = true;
        }
    }
}

fn helper_event_serializes_with_commands(event_type: Option<&str>) -> bool {
    matches!(
        event_type,
        Some(
            "listening_started"
                | "focus_target"
                | "audio_level"
                | "recording_ready"
                | "recording_discarded"
                | "finalizing_transcript"
                | "final_transcript"
                | "paste_target"
                | "paste_completed"
                | "error"
        )
    )
}

fn helper_event_requires_take_match(event_type: Option<&str>) -> bool {
    matches!(
        event_type,
        Some(
            "listening_started"
                | "finalizing_transcript"
                | "final_transcript"
                | "paste_target"
                | "paste_completed"
                | "error"
        )
    )
}

fn untagged_take_event_disposition(
    app: &AppHandle,
    event_type: Option<&str>,
    event: Option<&serde_json::Value>,
) -> HelperLifecycleDisposition {
    if event.and_then(dictation_take_id_from_event).is_some()
        || !matches!(
            event_type,
            Some(
                "audio_level"
                    | "focus_target"
                    | "finalizing_transcript"
                    | "final_transcript"
                    | "paste_target"
                    | "paste_completed"
                    | "error"
            )
        )
    {
        return HelperLifecycleDisposition::Apply;
    }
    app.try_state::<ShortcutActivationState>()
        .and_then(|state| {
            state
                .controller
                .lock()
                .ok()
                .map(|controller| controller.untagged_take_event_disposition(event_type))
        })
        .unwrap_or(HelperLifecycleDisposition::Apply)
}

fn emit_helper_event_without_lifecycle(app: &AppHandle, mut event: serde_json::Value) {
    sync_update_gate_take(app, &event);
    if let Some(payload) = event
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
    {
        payload.insert(
            "preserveActiveTake".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    annotate_silent_error(&mut event);
    let event_type = event.get("type").and_then(serde_json::Value::as_str);
    append_dictation_event_log(app, event_type, &event);
    let _ = app.emit("dictation-event", event.to_string());
}

fn send_dictation_command(
    app: &AppHandle,
    command: DictationCommand,
    shortcut_label: &str,
    starts_recording: bool,
    command_take_id: Option<String>,
) {
    let forwarded = forward_dictation_command(
        app,
        command,
        shortcut_label,
        starts_recording,
        command_take_id.as_deref(),
    );
    if !forwarded && matches!(command, DictationCommand::ToggleListening) {
        reset_shortcut_activation(app);
    }
}

fn forward_dictation_command(
    app: &AppHandle,
    command: DictationCommand,
    shortcut_label: &str,
    starts_recording: bool,
    command_take_id: Option<&str>,
) -> bool {
    if matches!(command, DictationCommand::DiscardListening) {
        cancel_dictation_take(app, command_take_id);
    }
    if starts_recording {
        if let Err(error) = admit_dictation_start(app, command_take_id) {
            emit_dictation_event_value(app, app_error_event(error));
            return false;
        }
    }
    let Some(state) = app.try_state::<HelperState>() else {
        if starts_recording {
            release_unsent_dictation_take(app, command_take_id);
        }
        emit_dictation_event_value(
            app,
            app_error_event(AppError::new(
                "dictation_helper_unavailable",
                "Dictation helper process is not running.",
            )),
        );
        return false;
    };

    let mut helper_command = command.helper_command(shortcut_label);
    if let (Some(take_id), Some(object)) = (command_take_id, helper_command.as_object_mut()) {
        object.insert("takeId".into(), serde_json::json!(take_id));
    }
    let mut composer_request_id = None;
    if starts_recording {
        if let Ok(registration) = state.composer_registration.lock() {
            if let Some(registration) = registration
                .as_ref()
                .filter(|registration| is_foreground_window_handle(registration.window_handle))
            {
                if let Some(object) = helper_command.as_object_mut() {
                    composer_request_id = Some(registration.request_id.clone());
                    object.insert(
                        "composerRequestId".into(),
                        serde_json::json!(registration.request_id),
                    );
                    #[cfg(target_os = "windows")]
                    {
                        object.insert(
                            "clovyProcessId".into(),
                            serde_json::json!(std::process::id()),
                        );
                        object.insert(
                            // Compatibility for released Windows helpers.
                            "juneProcessId".into(),
                            serde_json::json!(std::process::id()),
                        );
                        object.insert(
                            "clovyWindowHandle".into(),
                            serde_json::json!(registration.window_handle),
                        );
                        object.insert(
                            // Compatibility for released Windows helpers.
                            "juneWindowHandle".into(),
                            serde_json::json!(registration.window_handle),
                        );
                    }
                }
            }
        }
        if let Some(activation) = app.try_state::<ShortcutActivationState>() {
            if let Ok(mut controller) = activation.controller.lock() {
                controller.track_composer_request(composer_request_id.clone());
            }
        }
    }
    if let Err(error) = send_helper_command(&state, helper_command) {
        if starts_recording {
            release_unsent_dictation_take(app, command_take_id);
        }
        if let Some(request_id) = composer_request_id.as_deref() {
            clear_shortcut_composer_request(app, request_id);
        }
        emit_dictation_event_value(app, app_error_event(error));
        return false;
    }
    true
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    crate::app_paths::app_config_dir(app)
        .ok()
        .map(|directory| directory.join("dictation-settings.json"))
}

fn manage_settings(app: &AppHandle) {
    let path = settings_path(app).unwrap_or_else(|| PathBuf::from("dictation-settings.json"));
    let mut settings = load_settings(app);
    let migrated = migrate_dictation_cleanup(&mut settings);
    replace_current_settings(settings.clone());
    let state = DictationSettingsState {
        path,
        settings: Mutex::new(settings.clone()),
    };
    if migrated {
        if let Err(error) = save_settings(&state, &settings) {
            tracing::warn!(code = %error.code, "dictation cleanup migration not saved; retries next launch");
        }
    }
    app.manage(state);
}

fn load_settings(app: &AppHandle) -> DictationSettings {
    let Some(path) = settings_path(app) else {
        return DictationSettings::default();
    };

    fs::read_to_string(path)
        .ok()
        .and_then(|settings| serde_json::from_str::<DictationSettings>(&settings).ok())
        .unwrap_or_default()
}

fn normalize_language(language: Option<String>) -> Option<String> {
    language
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| {
            value.len() == 2
                && value
                    .chars()
                    .all(|character| character.is_ascii_lowercase())
        })
}

fn settings_store() -> &'static Mutex<DictationSettings> {
    SETTINGS_CACHE.get_or_init(|| Mutex::new(DictationSettings::default()))
}

fn replace_current_settings(settings: DictationSettings) {
    if let Ok(mut current) = settings_store().lock() {
        *current = settings;
    }
}

fn update_settings(
    state: &DictationSettingsState,
    update: impl FnOnce(&mut DictationSettings),
) -> Result<DictationSettings, AppError> {
    let mut settings = state
        .settings
        .lock()
        .map_err(|_| AppError::new("dictation_settings_unavailable", "Settings lock failed."))?;
    update(&mut settings);
    save_settings(state, &settings)?;
    replace_current_settings(settings.clone());
    Ok(settings.clone())
}

fn save_settings(
    state: &DictationSettingsState,
    settings: &DictationSettings,
) -> Result<(), AppError> {
    if let Some(parent) = state.path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| AppError::new("dictation_settings_save_failed", error.to_string()))?;
    }

    let serialized = serde_json::to_string_pretty(settings)
        .map_err(|error| AppError::new("dictation_settings_save_failed", error.to_string()))?;
    fs::write(&state.path, serialized)
        .map_err(|error| AppError::new("dictation_settings_save_failed", error.to_string()))
}

fn helper_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // Debug builds use the helper build.rs prepares and signs in this checkout,
    // resolved at compile time like the Computer use and system audio helpers:
    // `pnpm tauri:dev` runs the signed dev bundle without Cargo's runtime
    // environment. Release builds keep the runtime `CARGO_MANIFEST_DIR`
    // fallback ahead of their bundled copies.
    #[cfg(debug_assertions)]
    let manifest_dir = Some(std::path::Path::new(env!("CARGO_MANIFEST_DIR")));
    #[cfg(not(debug_assertions))]
    let runtime_manifest_dir = std::env::var("CARGO_MANIFEST_DIR").ok().map(PathBuf::from);
    #[cfg(not(debug_assertions))]
    let manifest_dir = runtime_manifest_dir.as_deref();
    if let Some(manifest_dir) = manifest_dir {
        #[cfg(target_os = "macos")]
        if let Some(repo_dir) = manifest_dir.parent() {
            paths.push(
                repo_dir
                    .join(".tauri-helper")
                    .join("Clovy Dictation Helper.app")
                    .join("Contents")
                    .join("MacOS")
                    .join("clovy-dictation-helper"),
            );
        }
        #[cfg(target_os = "windows")]
        paths.push(
            manifest_dir
                .join("native")
                .join("bin")
                .join("clovy-dictation-helper.exe"),
        );
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            #[cfg(target_os = "windows")]
            {
                paths.push(exe_dir.join("clovy-dictation-helper.exe"));
                paths.push(
                    exe_dir
                        .join("native")
                        .join("bin")
                        .join("clovy-dictation-helper.exe"),
                );
                paths.push(
                    exe_dir
                        .join("resources")
                        .join("native")
                        .join("bin")
                        .join("clovy-dictation-helper.exe"),
                );
            }
            #[cfg(target_os = "macos")]
            {
                paths.push(exe_dir.join("clovy-dictation-helper"));
                paths.push(exe_dir.join("../Resources/clovy-dictation-helper"));
            }
        }
    }

    if let Ok(resource_dir) = app.path().resource_dir() {
        #[cfg(target_os = "windows")]
        {
            paths.push(resource_dir.join("clovy-dictation-helper.exe"));
            paths.push(
                resource_dir
                    .join("native")
                    .join("bin")
                    .join("clovy-dictation-helper.exe"),
            );
        }
        #[cfg(target_os = "macos")]
        {
            paths.push(resource_dir.join("clovy-dictation-helper"));
            paths.push(
                resource_dir
                    .join("native")
                    .join("bin")
                    .join("Clovy Dictation Helper.app")
                    .join("Contents")
                    .join("MacOS")
                    .join("clovy-dictation-helper"),
            );
            paths.push(
                resource_dir
                    .join("Clovy Dictation Helper.app")
                    .join("Contents")
                    .join("MacOS")
                    .join("clovy-dictation-helper"),
            );
        }
    }

    paths
}

/// The dictation helper binary's basename, used by the ownership-aware orphan
/// reap below to confirm a recorded pid is still a dictation helper.
#[cfg(target_os = "macos")]
const DICTATION_HELPER_NAME: &str = "clovy-dictation-helper";

/// Filename prefix for the per-instance ownership records. One record per owning
/// app pid (never a single shared file), so concurrent instances — allowed in
/// debug builds — cannot clobber each other's pointer to their helper.
#[cfg(target_os = "macos")]
const HELPER_OWNER_RECORD_PREFIX: &str = "dictation-helper-owner-";

/// How long the reap waits for a killed orphan to actually exit before giving
/// up. `kill` only submits the signal; the old helper can still hold its global
/// CGEventTap for a moment, which would collide with the replacement helper.
#[cfg(target_os = "macos")]
const HELPER_ORPHAN_EXIT_TIMEOUT: Duration = Duration::from_millis(500);

/// Records which app instance owns the currently-spawned helper. Identity is
/// bound to `(pid, start_time)` for BOTH the owner and the helper, never a bare
/// pid: pids are recycled, and sequential reuse would otherwise let a new app
/// mistake a dead owner's record for its own (or let a stale helper pid, now
/// reused by another live instance's helper, be killed). The start time is the
/// non-reusable half of the identity.
#[cfg(target_os = "macos")]
#[derive(serde::Serialize, serde::Deserialize)]
struct HelperOwnershipRecord {
    /// The app process that spawned the helper (`std::process::id()`).
    app_pid: u32,
    /// The owning app's process start time (`ps -o lstart=`), the non-reusable
    /// half of its identity.
    app_start: String,
    /// The spawned helper process.
    helper_pid: u32,
    /// The helper's process start time, so a reused helper pid is not mistaken
    /// for the recorded orphan.
    helper_start: String,
}

/// The result of probing a pid's start time: distinguishes "the pid is gone"
/// (definitive) from "could not run `ps`" (unknown), so callers can fail closed
/// on the latter instead of assuming the process is absent.
#[cfg(target_os = "macos")]
enum StartTimeProbe {
    /// `ps` could not be run — identity cannot be established.
    Unknown,
    /// No process with this pid exists.
    Gone,
    /// The process exists and started at this (`ps -o lstart=`) time.
    StartedAt(String),
}

/// Tri-state executable-identity check, kept distinct from "gone" so the caller
/// can fail closed when `ps` itself could not run.
#[cfg(target_os = "macos")]
enum HelperIdentity {
    /// `ps` could not be run — identity cannot be established.
    Unknown,
    IsHelper,
    NotHelper,
}

/// Whether the recorded owner is still the same live process.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OwnerLiveness {
    Alive,
    Dead,
    /// Could not be determined (e.g. `ps` failed).
    Unknown,
}

/// Whether the recorded helper pid still refers to the exact recorded process.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HelperMatch {
    /// The pid is alive, is a dictation helper, and its start time matches.
    MatchesRecord,
    /// The pid is gone, or was reused for a different process.
    GoneOrReused,
    /// Could not be determined (e.g. `ps` failed).
    Unknown,
}

/// What to do with one ownership record.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RecordAction {
    /// Owner still owns it — leave the record and its helper alone.
    Skip,
    /// Owner is gone and the helper is gone/reused — drop the stale record.
    DeleteStale,
    /// Owner is gone and the helper is the recorded orphan — kill it.
    Reap,
    /// Identity could not be established — fail closed: abort the spawn and keep
    /// the record for the next attempt.
    Abort,
}

#[cfg(target_os = "macos")]
fn helper_ownership_dir(app: &AppHandle) -> Option<PathBuf> {
    crate::app_paths::app_config_dir(app).ok()
}

/// This instance's own record path, keyed by our app pid so we never overwrite
/// another live instance's record.
#[cfg(target_os = "macos")]
fn helper_ownership_path(app: &AppHandle) -> Option<PathBuf> {
    helper_ownership_dir(app).map(|directory| {
        directory.join(format!(
            "{HELPER_OWNER_RECORD_PREFIX}{}.json",
            std::process::id()
        ))
    })
}

/// Probes a pid's start time. `ps` exits non-zero with no output for an absent
/// pid (definitive `Gone`); a failure to run `ps` at all is `Unknown`.
#[cfg(target_os = "macos")]
fn probe_start_time(pid: u32) -> StartTimeProbe {
    let output = crate::hide_console::command("/bin/ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("lstart=")
        .stdin(Stdio::null())
        .output();
    match output {
        Err(_) => StartTimeProbe::Unknown,
        Ok(output) if !output.status.success() => StartTimeProbe::Gone,
        Ok(output) => {
            let started = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if started.is_empty() {
                StartTimeProbe::Gone
            } else {
                StartTimeProbe::StartedAt(started)
            }
        }
    }
}

/// Whether a `ps -o comm=` line names the dictation helper. Pure and
/// truncation-tolerant: `comm=` may return the kernel `p_comm`, truncated to 15
/// printable chars (`june-dictation-`) instead of the full 21-char name, so a
/// basename that is a non-empty prefix of length ≥ 15 (the truncation length)
/// counts as a match. No other expected process shares a 15-char prefix of
/// `clovy-dictation-helper`, so this cannot false-positive. Handles both a bare
/// `comm` and a full path (`rsplit('/')`).
#[cfg(target_os = "macos")]
fn comm_line_is_dictation_helper(comm_line: &str) -> bool {
    let Some(basename) = comm_line
        .trim()
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
    else {
        return false;
    };
    basename == DICTATION_HELPER_NAME
        || (basename.len() >= 15 && DICTATION_HELPER_NAME.starts_with(basename))
}

/// Tri-state executable-identity check via `ps -o comm=`. `Unknown` when `ps`
/// could not run; `NotHelper` when the pid is gone or is a different program.
#[cfg(target_os = "macos")]
fn inspect_helper_identity(pid: u32) -> HelperIdentity {
    let output = crate::hide_console::command("/bin/ps")
        .arg("-p")
        .arg(pid.to_string())
        .arg("-o")
        .arg("comm=")
        .stdin(Stdio::null())
        .output();
    match output {
        Err(_) => HelperIdentity::Unknown,
        Ok(output) if !output.status.success() => HelperIdentity::NotHelper,
        Ok(output) => {
            if comm_line_is_dictation_helper(&String::from_utf8_lossy(&output.stdout)) {
                HelperIdentity::IsHelper
            } else {
                HelperIdentity::NotHelper
            }
        }
    }
}

/// Classifies the recorded owner against a live probe. Pure, so the pid-reuse
/// cases are unit-testable without processes.
#[cfg(target_os = "macos")]
fn owner_liveness(probe: &StartTimeProbe, recorded_start: &str) -> OwnerLiveness {
    match probe {
        StartTimeProbe::Unknown => OwnerLiveness::Unknown,
        StartTimeProbe::Gone => OwnerLiveness::Dead,
        // Same pid, but a *different* start time means the pid was reused by an
        // unrelated process: the original owner is gone.
        StartTimeProbe::StartedAt(started) => {
            if started == recorded_start {
                OwnerLiveness::Alive
            } else {
                OwnerLiveness::Dead
            }
        }
    }
}

/// Classifies the recorded helper against a live probe and identity check. Pure.
#[cfg(target_os = "macos")]
fn helper_match(
    probe: &StartTimeProbe,
    recorded_start: &str,
    identity: &HelperIdentity,
) -> HelperMatch {
    match probe {
        StartTimeProbe::Unknown => HelperMatch::Unknown,
        StartTimeProbe::Gone => HelperMatch::GoneOrReused,
        StartTimeProbe::StartedAt(started) => {
            if started != recorded_start {
                // Pid reused by a different process (possibly another live
                // instance's helper) — never touch it.
                HelperMatch::GoneOrReused
            } else {
                match identity {
                    HelperIdentity::Unknown => HelperMatch::Unknown,
                    HelperIdentity::IsHelper => HelperMatch::MatchesRecord,
                    // Exact recorded (pid, start_time) but the executable does
                    // not read as the helper. With the truncation-tolerant
                    // identity check above, a real helper reads IsHelper, so this
                    // is only a genuine same-second pid reuse by a non-helper —
                    // rare. Fail closed (abort + retry) rather than delete the
                    // record and risk racing an orphan.
                    HelperIdentity::NotHelper => HelperMatch::Unknown,
                }
            }
        }
    }
}

/// The full reap decision for one record. Pure and total, so every combination
/// is unit-testable. Fails closed (`Abort`) whenever identity is `Unknown`.
#[cfg(target_os = "macos")]
fn decide_record_action(owner: OwnerLiveness, helper: HelperMatch) -> RecordAction {
    match owner {
        OwnerLiveness::Alive => RecordAction::Skip,
        OwnerLiveness::Unknown => RecordAction::Abort,
        OwnerLiveness::Dead => match helper {
            HelperMatch::MatchesRecord => RecordAction::Reap,
            HelperMatch::GoneOrReused => RecordAction::DeleteStale,
            HelperMatch::Unknown => RecordAction::Abort,
        },
    }
}

/// Records the just-spawned helper's `(pid, start_time)` alongside our own
/// `(pid, start_time)`, so the next instance's [`reap_orphaned_helpers`] can
/// establish ownership against non-reusable identities. Written atomically (temp
/// file + fsync + rename) so a crash or partial write never leaves an
/// unparseable record. Returns an error on any failure — including being unable
/// to read our own or the helper's start time — so the caller reaps the just-
/// spawned child rather than leaving it untracked (fail closed).
#[cfg(target_os = "macos")]
fn record_helper_ownership(app: &AppHandle, helper_pid: u32) -> Result<(), AppError> {
    let path = helper_ownership_path(app).ok_or_else(|| {
        AppError::new(
            "dictation_helper_owner_unrecordable",
            "Could not resolve the dictation helper ownership path.",
        )
    })?;
    let StartTimeProbe::StartedAt(app_start) = probe_start_time(std::process::id()) else {
        return Err(AppError::new(
            "dictation_helper_owner_unrecordable",
            "Could not read this app's process start time.",
        ));
    };
    let StartTimeProbe::StartedAt(helper_start) = probe_start_time(helper_pid) else {
        return Err(AppError::new(
            "dictation_helper_owner_unrecordable",
            "Could not read the dictation helper's process start time.",
        ));
    };
    let record = HelperOwnershipRecord {
        app_pid: std::process::id(),
        app_start,
        helper_pid,
        helper_start,
    };
    let serialized = serde_json::to_string(&record)
        .map_err(|error| AppError::new("dictation_helper_owner_unrecordable", error.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::new("dictation_helper_owner_unrecordable", error.to_string())
        })?;
    }
    let tmp = path.with_extension("json.tmp");
    let write = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(serialized.as_bytes())?;
        file.sync_all()?;
        fs::rename(&tmp, &path)
    })();
    write.map_err(|error| {
        let _ = fs::remove_file(&tmp);
        AppError::new("dictation_helper_owner_unrecordable", error.to_string())
    })
}

#[cfg(not(target_os = "macos"))]
fn record_helper_ownership(_app: &AppHandle, _helper_pid: u32) -> Result<(), AppError> {
    Ok(())
}

/// Reaps any dictation helper orphaned by a *prior* app instance before a fresh
/// one is spawned. A stale helper — most often left by an in-app update relaunch
/// that skipped teardown (JUN-338) — keeps the global CGEventTap and its stdio,
/// so a newly spawned helper collides with it and never delivers a clean
/// permission status.
///
/// Ownership-aware, never a name-wide sweep, and keyed on non-reusable
/// `(pid, start_time)` identities: it kills only a pid a prior instance
/// recorded, only when that instance is proven gone AND the pid is still the
/// exact recorded helper. A helper a concurrently-running instance owns has a
/// live owner (or a mismatching start time), so it is never touched — which
/// matters because debug builds allow multiple instances.
///
/// Fails closed: if a confirmed orphan will not die, or identity cannot be
/// established (`ps`/fs failure), it returns an error that aborts the
/// replacement spawn and keeps the record, rather than spawning over a helper
/// that may still hold the tap. macOS-only.
#[cfg(target_os = "macos")]
fn reap_orphaned_helpers(app: &AppHandle) -> Result<(), AppError> {
    let Some(dir) = helper_ownership_dir(app) else {
        // No config dir means no record was ever written: nothing to reap.
        tracing::warn!("dictation reap: no app config dir; skipping orphan reap");
        return Ok(());
    };
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            // The dir exists but is unreadable: we cannot prove there is no
            // orphan, so fail closed.
            tracing::warn!(%error, "dictation reap: ownership dir unreadable; aborting spawn");
            return Err(AppError::new(
                "dictation_helper_reap_failed",
                "Could not read the dictation helper ownership records.",
            ));
        }
    };
    for entry in entries {
        // A directory entry we cannot even enumerate might be a record naming a
        // live orphan: fail closed rather than skip it as absent.
        let entry = entry.map_err(|error| {
            tracing::warn!(%error, "dictation reap: ownership entry unreadable; aborting spawn");
            AppError::new(
                "dictation_helper_reap_failed",
                "Could not enumerate the dictation helper ownership records.",
            )
        })?;
        let path = entry.path();
        let is_record = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with(HELPER_OWNER_RECORD_PREFIX) && name.ends_with(".json")
            });
        if !is_record {
            continue;
        }
        // Distinguish a read I/O error from malformed content. An unreadable
        // record may name a live orphan still holding the event tap, so retain
        // it and fail closed; only a successfully-read-but-unparseable record is
        // the (atomic-write-guarded, near-impossible) delete-stale case.
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "dictation reap: ownership record unreadable; aborting spawn");
                return Err(AppError::new(
                    "dictation_helper_reap_failed",
                    "Could not read a dictation helper ownership record.",
                ));
            }
        };
        let Ok(record) = serde_json::from_str::<HelperOwnershipRecord>(&raw) else {
            tracing::warn!(path = %path.display(), "dictation reap: dropping unparseable ownership record");
            let _ = fs::remove_file(&path);
            continue;
        };
        let owner = owner_liveness(&probe_start_time(record.app_pid), &record.app_start);
        let action = if owner == OwnerLiveness::Alive {
            RecordAction::Skip
        } else {
            let helper = helper_match(
                &probe_start_time(record.helper_pid),
                &record.helper_start,
                &inspect_helper_identity(record.helper_pid),
            );
            decide_record_action(owner, helper)
        };
        match action {
            RecordAction::Skip => {}
            RecordAction::DeleteStale => {
                let _ = fs::remove_file(&path);
            }
            RecordAction::Reap => {
                kill_pid(record.helper_pid);
                if !wait_for_pid_exit(record.helper_pid, HELPER_ORPHAN_EXIT_TIMEOUT) {
                    tracing::warn!(
                        helper_pid = record.helper_pid,
                        "dictation reap: orphaned helper would not exit; aborting spawn"
                    );
                    return Err(AppError::new(
                        "dictation_helper_orphan_stuck",
                        "An orphaned dictation helper would not exit; retrying.",
                    ));
                }
                let _ = fs::remove_file(&path);
            }
            RecordAction::Abort => {
                tracing::warn!(
                    app_pid = record.app_pid,
                    helper_pid = record.helper_pid,
                    "dictation reap: could not establish ownership; aborting spawn"
                );
                return Err(AppError::new(
                    "dictation_helper_reap_failed",
                    "Could not establish dictation helper ownership; retrying.",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn reap_orphaned_helpers(_app: &AppHandle) -> Result<(), AppError> {
    Ok(())
}

/// What to do with a *running* helper process found by name enumeration (the
/// legacy/migration sweep) rather than by an ownership record.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum UnrecordedAction {
    /// Claimed by a live-owner record (a sibling instance's healthy helper) or
    /// not confirmably a helper — leave it alone.
    Skip,
    /// An unclaimed, confirmed helper — a legacy/leftover orphan; kill it.
    Reap,
    /// Identity could not be established — fail closed: abort the spawn.
    Abort,
}

/// The decision for one enumerated helper pid. Pure and total, so unit-testable.
/// A pid claimed by a *live* owner is a running instance's helper (never
/// touched); an unclaimed pid is reaped only when confirmed to still be a
/// dictation helper, and an indeterminate identity fails closed.
#[cfg(target_os = "macos")]
fn decide_unrecorded_action(
    claimed_by_live_owner: bool,
    identity: &HelperIdentity,
) -> UnrecordedAction {
    if claimed_by_live_owner {
        return UnrecordedAction::Skip;
    }
    match identity {
        HelperIdentity::IsHelper => UnrecordedAction::Reap,
        // Enumeration matched the name but `ps` disagrees on identity: do not
        // kill an unconfirmed process.
        HelperIdentity::NotHelper => UnrecordedAction::Skip,
        HelperIdentity::Unknown => UnrecordedAction::Abort,
    }
}

/// The helper pids that a *live-owner* ownership record currently claims. Used
/// by the legacy sweep to avoid killing a concurrently-running sibling
/// instance's healthy helper (debug multi-instance). Fails closed (error) on any
/// inability to enumerate or read a record, mirroring [`reap_orphaned_helpers`].
#[cfg(target_os = "macos")]
fn live_owner_claimed_helper_pids(app: &AppHandle) -> Result<Vec<u32>, AppError> {
    let Some(dir) = helper_ownership_dir(app) else {
        return Ok(Vec::new());
    };
    let entries = match fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            tracing::warn!(%error, "dictation legacy reap: ownership dir unreadable; aborting spawn");
            return Err(AppError::new(
                "dictation_helper_reap_failed",
                "Could not read the dictation helper ownership records.",
            ));
        }
    };
    let mut claimed = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| {
            tracing::warn!(%error, "dictation legacy reap: ownership entry unreadable; aborting spawn");
            AppError::new(
                "dictation_helper_reap_failed",
                "Could not enumerate the dictation helper ownership records.",
            )
        })?;
        let path = entry.path();
        let is_record = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with(HELPER_OWNER_RECORD_PREFIX) && name.ends_with(".json")
            });
        if !is_record {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "dictation legacy reap: ownership record unreadable; aborting spawn");
                return Err(AppError::new(
                    "dictation_helper_reap_failed",
                    "Could not read a dictation helper ownership record.",
                ));
            }
        };
        // Unparseable records name no live orphan; the record-driven reap that
        // ran first already dropped them. Ignore here.
        let Ok(record) = serde_json::from_str::<HelperOwnershipRecord>(&raw) else {
            continue;
        };
        if owner_liveness(&probe_start_time(record.app_pid), &record.app_start)
            == OwnerLiveness::Alive
        {
            claimed.push(record.helper_pid);
        }
    }
    Ok(claimed)
}

/// Enumerates running dictation-helper pids by matching the FULL command line
/// (`pgrep -f`), which carries the untruncated binary name/path and so is robust
/// to the `comm` truncation the record-driven path handles. Best-effort: `pgrep`
/// exits non-zero with no matches, and any failure yields an empty list.
#[cfg(target_os = "macos")]
fn enumerate_running_helper_pids() -> Vec<u32> {
    let output = crate::hide_console::command("/usr/bin/pgrep")
        .arg("-f")
        .arg(DICTATION_HELPER_NAME)
        .stdin(Stdio::null())
        .output();
    match output {
        Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .filter_map(|token| token.parse::<u32>().ok())
            .collect(),
        _ => Vec::new(),
    }
}

/// One-time migration sweep for helpers left by a *pre-record* build. Those
/// orphans have no `dictation-helper-owner-*.json`, so the record-driven
/// [`reap_orphaned_helpers`] cannot see them: on the very upgrade that fixes
/// JUN-338 the legacy helper would otherwise survive, keep the CGEventTap, and
/// leave permissions wedged.
///
/// SAFETY / TRADEOFF (call out for review): this deliberately reintroduces the
/// name-based enumeration that an earlier round replaced with ownership records.
/// It is justified ONLY for the unrecorded/migration case, and made safe by
/// skipping any pid a *live-owner* record claims — so a concurrently-running
/// sibling instance's healthy helper is skipped once that sibling has recorded
/// ownership. A debug-only TOCTOU remains: a sibling that has spawned its helper
/// but not yet written its record (spawn precedes the record write) could be
/// reaped by another instance's sweep. Production is single instance, so at
/// startup-before-spawn there is no sibling and we have not spawned yet — any
/// running helper is a legacy orphan and is reaped. Fails closed on record-read
/// ambiguity but never livelocks: a genuine
/// legacy orphan reads `IsHelper` and is killed so dictation recovers.
#[cfg(target_os = "macos")]
fn reap_unrecorded_helpers(app: &AppHandle) -> Result<(), AppError> {
    let pids = enumerate_running_helper_pids();
    if pids.is_empty() {
        return Ok(());
    }
    let claimed = live_owner_claimed_helper_pids(app)?;
    for pid in pids {
        let claimed_by_live_owner = claimed.contains(&pid);
        let identity = inspect_helper_identity(pid);
        match decide_unrecorded_action(claimed_by_live_owner, &identity) {
            UnrecordedAction::Skip => {}
            UnrecordedAction::Reap => {
                kill_pid(pid);
                if !wait_for_pid_exit(pid, HELPER_ORPHAN_EXIT_TIMEOUT) {
                    tracing::warn!(
                        helper_pid = pid,
                        "dictation legacy reap: unrecorded helper would not exit; aborting spawn"
                    );
                    return Err(AppError::new(
                        "dictation_helper_orphan_stuck",
                        "An orphaned dictation helper would not exit; retrying.",
                    ));
                }
            }
            UnrecordedAction::Abort => {
                tracing::warn!(
                    helper_pid = pid,
                    "dictation legacy reap: could not establish helper identity; aborting spawn"
                );
                return Err(AppError::new(
                    "dictation_helper_reap_failed",
                    "Could not establish a running dictation helper's identity; retrying.",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn reap_unrecorded_helpers(_app: &AppHandle) -> Result<(), AppError> {
    Ok(())
}

/// Submits SIGKILL to a single pid (best-effort; the caller confirms exit).
#[cfg(target_os = "macos")]
fn kill_pid(pid: u32) {
    let _ = crate::hide_console::command("/bin/kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Polls until `pid` is no longer alive, or the timeout elapses. Returns whether
/// it exited, so the caller can refuse to spawn a replacement while an orphan
/// still holds the global CGEventTap.
#[cfg(target_os = "macos")]
fn wait_for_pid_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if !process_alive(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether a macOS `ps` state represents a process that can still own runtime
/// resources. `E` (exiting) and `Z` (zombie) entries may remain in the process
/// table and pass `kill -0`, but cannot own Clovy's global event tap.
#[cfg(target_os = "macos")]
fn process_state_is_terminal(state: &str) -> bool {
    state.trim().chars().any(|flag| matches!(flag, 'E' | 'Z'))
}

/// Whether a pid is still a functioning process. Check macOS process state
/// before `kill -0`: an exiting (`E`) or zombie (`Z`) entry can remain
/// signalable indefinitely even though it is inert.
#[cfg(target_os = "macos")]
fn process_alive(pid: u32) -> bool {
    if let Ok(output) = crate::hide_console::command("/bin/ps")
        .arg("-p")
        .arg(pid.to_string())
        .args(["-o", "state="])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
    {
        if output.status.success() {
            let state = String::from_utf8_lossy(&output.stdout);
            if state.trim().is_empty() || process_state_is_terminal(&state) {
                return false;
            }
        } else if output.status.code() == Some(1) {
            // BSD ps uses status 1 when the selected pid no longer exists.
            return false;
        }
    }

    crate::hide_console::command("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn spawn_helper(app: &AppHandle) -> Result<HelperProcess, AppError> {
    // Abort the spawn if a prior instance's orphaned helper is still holding the
    // global CGEventTap; racing it would reproduce the permission collision.
    // Record-driven reap first, then the one-time legacy sweep for helpers left
    // by pre-record builds (no ownership record exists for those) — JUN-338.
    reap_orphaned_helpers(app)?;
    reap_unrecorded_helpers(app)?;

    let helper_path = helper_candidates(app)
        .into_iter()
        .find(|path| path.exists())
        .ok_or_else(|| {
            AppError::new(
                "dictation_helper_missing",
                "Could not find bundled dictation helper binary.",
            )
        })?;

    let mut command = crate::hide_console::command(&helper_path);
    #[cfg(target_os = "macos")]
    command.env("CLOVY_OWNER_PID", std::process::id().to_string());
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            AppError::new(
                "dictation_helper_start_failed",
                format!(
                    "Failed to start helper at {}: {error}",
                    helper_path.display()
                ),
            )
        })?;

    let stdin = child.stdin.take().ok_or_else(|| {
        AppError::new(
            "dictation_helper_start_failed",
            "Helper stdin was unavailable.",
        )
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        AppError::new(
            "dictation_helper_start_failed",
            "Helper stdout was unavailable.",
        )
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        AppError::new(
            "dictation_helper_start_failed",
            "Helper stderr was unavailable.",
        )
    })?;

    // Record ownership so the next instance can reap this exact helper if we
    // exit without tearing it down (e.g. an update relaunch), and never touch a
    // helper another live instance owns (JUN-338). If it cannot be persisted,
    // do not leave an untracked helper running: kill it and fail the spawn.
    if let Err(error) = record_helper_ownership(app, child.id()) {
        tracing::warn!(
            error = %error.message,
            "dictation: could not record helper ownership; killing untracked helper"
        );
        let _ = crate::shutdown::terminate_child(child, Duration::ZERO, HELPER_FORCED_REAP_TIMEOUT);
        return Err(error);
    }

    // Recorded before handing stdout to the reader so the supervisor can tell a
    // healthy helper (ran a while, then died) from a crash loop.
    let spawn_instant = Instant::now();

    let output_app = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            handle_helper_event_line(&output_app, line);
        }
        // The helper's event stream lives for the whole process lifetime, so
        // stdout closing means the helper exited (crash, SIGKILL after an
        // update swaps the bundle, or an intentional stop). Hand off to the
        // supervisor to reap it and decide whether to respawn.
        supervise_helper_exit(&output_app, spawn_instant);
    });

    let error_app = app.clone();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let _ = error_app.emit("dictation-helper-stderr", line);
        }
    });

    Ok(HelperProcess { child, stdin })
}

/// How many consecutive rapid respawns we attempt before giving up and showing
/// the "relaunch to recover" notice. Bounds a genuine crash loop so it can't
/// burn CPU forever, while still recovering from any isolated helper death.
const HELPER_MAX_RESPAWN_ATTEMPTS: u32 = 5;
/// Backoff before the first respawn; doubles each attempt up to the cap below.
const HELPER_RESPAWN_BASE_BACKOFF: Duration = Duration::from_millis(500);
const HELPER_RESPAWN_MAX_BACKOFF: Duration = Duration::from_secs(8);
/// A helper that ran at least this long before dying is treated as healthy: its
/// death starts a fresh failure count instead of counting toward the cap, so
/// repeated manual kills (or a death long after launch) always recover.
const HELPER_HEALTHY_RUNTIME: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Copy)]
struct RespawnPolicy {
    max_attempts: u32,
    base_backoff: Duration,
    max_backoff: Duration,
    healthy_runtime: Duration,
}

impl Default for RespawnPolicy {
    fn default() -> Self {
        Self {
            max_attempts: HELPER_MAX_RESPAWN_ATTEMPTS,
            base_backoff: HELPER_RESPAWN_BASE_BACKOFF,
            max_backoff: HELPER_RESPAWN_MAX_BACKOFF,
            healthy_runtime: HELPER_HEALTHY_RUNTIME,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum RespawnAction {
    /// Intentional shutdown: leave the helper down.
    Skip,
    /// Relaunch the helper after `backoff`. `attempt` is 1-based.
    Respawn { attempt: u32, backoff: Duration },
    /// Too many rapid failures: surface the unavailable notice and stop trying.
    GiveUp,
}

/// Pure decision for what to do after the dictation helper process exits.
///
/// `prior_failures` is the count of consecutive rapid respawns so far;
/// `survived` is how long the process that just exited had been running.
/// Returns the action to take plus the new consecutive-failure count to persist
/// (so the caller stays a thin, side-effecting shell around this logic).
fn decide_respawn(
    shutting_down: bool,
    prior_failures: u32,
    survived: Duration,
    policy: &RespawnPolicy,
) -> (RespawnAction, u32) {
    if shutting_down {
        return (RespawnAction::Skip, prior_failures);
    }
    let attempt = if survived >= policy.healthy_runtime {
        1
    } else {
        prior_failures.saturating_add(1)
    };
    if attempt > policy.max_attempts {
        return (RespawnAction::GiveUp, attempt);
    }
    (
        RespawnAction::Respawn {
            attempt,
            backoff: respawn_backoff(attempt, policy),
        },
        attempt,
    )
}

fn respawn_backoff(attempt: u32, policy: &RespawnPolicy) -> Duration {
    // 1-based attempts: the first respawn waits `base_backoff`, each later one
    // doubles, all clamped to `max_backoff`.
    let shift = attempt.saturating_sub(1).min(16);
    policy
        .base_backoff
        .checked_mul(1u32 << shift)
        .unwrap_or(policy.max_backoff)
        .min(policy.max_backoff)
}

fn helper_unavailable_event(reason: &str, message: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "helper_unavailable",
        "payload": {
            "reason": reason,
            "message": message,
        },
    })
}

/// Retries the first helper launch. Previously only helpers that had already
/// launched were supervised; any setup-time failure left dictation silently
/// unavailable for the entire app session.
fn supervise_initial_helper_start(app: &AppHandle) {
    retry_helper_spawn(app, Duration::ZERO, true);
}

/// Runs on the helper's stdout reader thread once that stream closes (the helper
/// exited or was killed). Reaps the child, then respawns with backoff unless the
/// exit was an intentional shutdown or the retry cap is exhausted. A successful
/// respawn re-applies settings and re-arms the hotkey, and its own reader thread
/// takes over supervision, so this thread simply returns.
fn supervise_helper_exit(app: &AppHandle, spawn_instant: Instant) {
    // A helper that dies mid-listening emits no terminal event, so restore
    // any ducked meeting mic here — the crash path must not swallow the rest
    // of the meeting's mic track.
    crate::audio::capture::set_mic_ducked(false);

    let Some(state) = app.try_state::<HelperState>() else {
        return;
    };

    // Reap the exited child so it does not linger as a zombie, and clear the
    // stale handle so commands correctly see the helper as unavailable while
    // it is down.
    if !reap_exited_helper(&state) {
        return;
    }
    reset_shortcut_activation(app);

    retry_helper_spawn(app, spawn_instant.elapsed(), false);
}

fn retry_helper_spawn(app: &AppHandle, mut survived: Duration, initial_start: bool) {
    let Some(state) = app.try_state::<HelperState>() else {
        return;
    };
    let policy = RespawnPolicy::default();
    loop {
        let (action, next_failures) = decide_and_record_respawn(&state, survived, &policy);

        match action {
            RespawnAction::Skip => return,
            RespawnAction::GiveUp => {
                tracing::error!(
                    attempts = next_failures,
                    initial_start,
                    "dictation helper recovery exhausted; giving up until relaunch"
                );
                let message = if initial_start {
                    "Dictation could not start. Relaunch Clovy to try again."
                } else {
                    "Dictation stopped and could not restart. Relaunch Clovy to restore it."
                };
                emit_helper_unavailable(app, "exhausted", message);
                return;
            }
            RespawnAction::Respawn { attempt, backoff } => {
                tracing::warn!(
                    attempt,
                    ?backoff,
                    initial_start,
                    "dictation helper unavailable; respawning"
                );
                // Surface the notice while the helper is down so the hotkey is
                // never silently dead; a successful respawn clears it by
                // re-emitting the hotkey-ready event.
                let message = if initial_start {
                    "Dictation is starting."
                } else {
                    "Dictation stopped and is restarting."
                };
                emit_helper_unavailable(app, "restarting", message);
                thread::sleep(backoff);
                if state.shutting_down.load(Ordering::SeqCst) {
                    return;
                }
                match spawn_helper(app) {
                    Ok(helper) => match store_respawned_helper(&state, helper) {
                        StoreRespawnedHelper::Stored => {
                            reapply_helper_settings(app);
                            // The new helper is live and its reader thread now
                            // supervises.
                            return;
                        }
                        StoreRespawnedHelper::ExitedBeforeStore => {
                            survived = Duration::ZERO;
                        }
                        StoreRespawnedHelper::Abandoned => return,
                    },
                    Err(error) => {
                        tracing::warn!(
                            error = %error.message,
                            "failed to respawn dictation helper; retrying"
                        );
                        // A spawn failure (e.g. the bundle is still mid-swap)
                        // counts as an immediate, unhealthy failure so the cap
                        // still applies.
                        survived = Duration::ZERO;
                    }
                }
            }
        }
    }
}

fn decide_and_record_respawn(
    state: &HelperState,
    survived: Duration,
    policy: &RespawnPolicy,
) -> (RespawnAction, u32) {
    loop {
        let shutting_down = state.shutting_down.load(Ordering::SeqCst);
        let prior = state.respawn_failures.load(Ordering::SeqCst);
        let (action, next_failures) = decide_respawn(shutting_down, prior, survived, policy);
        if matches!(action, RespawnAction::Skip) {
            return (action, next_failures);
        }
        if state
            .respawn_failures
            .compare_exchange(prior, next_failures, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return (action, next_failures);
        }
    }
}

fn reap_exited_helper(state: &HelperState) -> bool {
    if let Ok(mut guard) = state.process.lock() {
        if let Some(process) = guard.take() {
            let _ = crate::shutdown::terminate_child(
                process.child,
                Duration::from_millis(100),
                HELPER_FORCED_REAP_TIMEOUT,
            );
            return true;
        }
    }
    false
}

#[derive(Debug, PartialEq, Eq)]
enum StoreRespawnedHelper {
    Stored,
    ExitedBeforeStore,
    Abandoned,
}

/// Installs a freshly respawned helper as the active process, unless an
/// intentional shutdown raced in first, in which case the new helper is stopped
/// so it does not leak.
fn store_respawned_helper(state: &HelperState, mut helper: HelperProcess) -> StoreRespawnedHelper {
    let Ok(mut guard) = state.process.lock() else {
        // Poisoned lock: stop the orphan we just created rather than leak it.
        abandon_helper(helper);
        return StoreRespawnedHelper::Abandoned;
    };
    if state.shutting_down.load(Ordering::SeqCst) {
        // Shutdown won the race. Drop the lock before the blocking kill so
        // other command threads are not stalled on wait().
        drop(guard);
        abandon_helper(helper);
        return StoreRespawnedHelper::Abandoned;
    }
    if matches!(helper.child.try_wait(), Ok(Some(_))) {
        return StoreRespawnedHelper::ExitedBeforeStore;
    }
    *guard = Some(helper);
    StoreRespawnedHelper::Stored
}

const HELPER_SHUTDOWN_GRACE: Duration = Duration::from_millis(750);
const HELPER_FORCED_REAP_TIMEOUT: Duration = Duration::from_millis(250);
const HELPER_SHUTDOWN_MUTEX_TIMEOUT: Duration = Duration::from_millis(250);

/// Stops a helper we spawned but will not install (shutdown raced the respawn).
/// Dropping a [`Child`] only detaches it, so we kill explicitly to avoid an
/// orphaned helper holding the global hotkey tap. Give a cooperative shutdown a
/// short grace period first so the helper can restore clipboard state and
/// finalize an in-flight WAV before the forced-kill fallback. The aggregate app
/// deadline may still cut off a wedged finalize; that trades the incomplete
/// current artifact for a relaunch that cannot be held hostage by the helper.
fn abandon_helper(helper: HelperProcess) {
    let HelperProcess { child, stdin } = helper;
    let helper_pid = child.id();
    send_helper_shutdown_nonblocking(stdin);
    let outcome =
        crate::shutdown::terminate_child(child, HELPER_SHUTDOWN_GRACE, HELPER_FORCED_REAP_TIMEOUT);
    if matches!(
        outcome,
        crate::shutdown::ChildTermination::TimedOut | crate::shutdown::ChildTermination::WaitFailed
    ) {
        tracing::warn!(
            ?outcome,
            helper_pid,
            "dictation helper could not be reaped before shutdown continued"
        );
    }
}

/// The helper normally needs one final command to restore clipboard and
/// recording state. Mark the pipe nonblocking before the single small write so
/// a wedged helper with a full stdin pipe cannot wedge app shutdown too.
#[cfg(unix)]
fn send_helper_shutdown_nonblocking(mut stdin: ChildStdin) {
    use std::os::fd::AsRawFd;

    let fd = stdin.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        tracing::warn!("dictation helper shutdown could not make stdin nonblocking");
        return;
    }
    let _ = stdin.write(b"{\"type\":\"shutdown\"}\n");
}

#[cfg(not(unix))]
fn send_helper_shutdown_nonblocking(mut stdin: ChildStdin) {
    // Windows anonymous pipes do not expose a portable nonblocking flag. Move
    // the best-effort write onto its own detached thread; the bounded child
    // reaper below remains independent of whether this write ever returns.
    let _ = thread::Builder::new()
        .name("dictation-shutdown-pipe".to_string())
        .spawn(move || {
            let _ = stdin.write_all(b"{\"type\":\"shutdown\"}\n");
        });
}

/// Re-applies the persisted microphone and shortcut settings to a just-respawned
/// helper and re-arms the hotkey, mirroring the one-time wiring in [`setup`]. The
/// hotkey-ready event also clears the "dictation restarting" notice in the UI.
fn reapply_helper_settings(app: &AppHandle) {
    let Some(helper_state) = app.try_state::<HelperState>() else {
        return;
    };
    let settings = app
        .try_state::<DictationSettingsState>()
        .and_then(|state| state.settings.lock().ok().map(|settings| settings.clone()));
    let Some(settings) = settings else {
        return;
    };
    let _ = apply_microphone_setting(&helper_state, &settings.microphone);
    let _ = apply_shortcut_settings(&helper_state, &settings);
    #[cfg(target_os = "macos")]
    {
        let event = hotkey_ready_event(&settings);
        if let Some(status) = app.try_state::<HotkeyStatus>() {
            set_hotkey_status(&status, event.clone());
        }
        emit_dictation_event_value(app, event);
    }
}

fn emit_helper_unavailable(app: &AppHandle, reason: &str, message: &str) {
    update_shortcut_helper_listening(app, false, None);
    clear_shortcut_helper_protocol(app);
    let event = helper_unavailable_event(reason, message);
    if let Some(status) = app.try_state::<HotkeyStatus>() {
        set_hotkey_status(&status, event.clone());
    }
    emit_dictation_event_value(app, event);
}

fn handle_helper_event_line(app: &AppHandle, line: String) {
    let mut event = serde_json::from_str::<serde_json::Value>(&line).ok();
    let event_type = event
        .as_ref()
        .and_then(|event| event.get("type"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let event_type = event_type.as_deref();
    #[cfg(target_os = "macos")]
    if event_type == Some("repeat_last_dictation_requested") {
        if let Some(request) = event.as_ref() {
            let state = app.state::<DictationTakeState>();
            let _guard = state.lock_helper_commands();
            let profile = crate::commands::active_profile(app);
            if let Some(response) = state.last_dictation_response(&profile, request) {
                if let Err(error) = send_helper_command(&app.state::<HelperState>(), response) {
                    tracing::warn!(code = %error.code, "dictation repeat response could not be sent");
                }
            }
        }
        return;
    }
    #[cfg(target_os = "windows")]
    if let Some(event) = event.as_mut() {
        complete_windows_target(event, |hwnd, pid| {
            crate::activity::windows::window::app_of_window(hwnd as isize, pid)
        });
    }
    if event_type == Some("shortcut_trigger_diagnostic") {
        // What the Windows hook saw: the event log only, never the HUD.
        if let Some(event) = event.as_ref() {
            append_dictation_event_log(app, event_type, event);
        }
        return;
    }
    let take_state = app.try_state::<DictationTakeState>();
    // A helper outcome and a replacement command must have one total order.
    // If the outcome wins, its whole lifecycle transition and frontend emit
    // finish before the replacement is prepared. If the command wins, its
    // take correlation is already installed and the stale outcome is ignored.
    let _helper_command_guard = helper_event_serializes_with_commands(event_type)
        .then(|| {
            take_state
                .as_ref()
                .map(|state| state.lock_helper_commands())
        })
        .flatten();
    // The filters below may drop this event for the HUD and the take
    // lifecycle; the update gate still has to see a take end.
    if let Some(event) = event.as_ref() {
        end_gate_take_for_helper_event(&crate::update_gate::UPDATE_GATE, event);
        forget_admission_for_terminal_event(app, event);
    }
    note_helper_take_protocol(app, event.as_ref());

    if matches!(event_type, Some("audio_level" | "focus_target"))
        && event
            .as_ref()
            .is_some_and(|event| !helper_audio_level_matches_active_take(app, event))
    {
        return;
    }
    if helper_event_requires_take_match(event_type)
        && event
            .as_ref()
            .is_some_and(|event| !helper_event_matches_active_take(app, event))
    {
        if event_type == Some("error") {
            handle_shortcut_composer_response(app, event_type, event.as_ref());
            if let Some(event) = event.as_ref().cloned() {
                emit_helper_event_without_lifecycle(app, event);
            }
        }
        tracing::info!(
            event_type,
            take_id = event.as_ref().and_then(dictation_take_id_from_event),
            "ignoring helper lifecycle event that does not belong to the active dictation take"
        );
        return;
    }
    let lifecycle_disposition =
        match untagged_take_event_disposition(app, event_type, event.as_ref()) {
            HelperLifecycleDisposition::Apply => match (event_type, event.as_ref()) {
                (Some("recording_discarded"), Some(event)) => {
                    reset_shortcut_for_helper_discard(app, event)
                }
                (Some("final_transcript" | "error"), Some(event))
                    if dictation_take_id_from_event(event).is_some() =>
                {
                    finish_shortcut_for_helper_event(app, event)
                }
                _ => HelperLifecycleDisposition::Apply,
            },
            disposition => disposition,
        };
    if lifecycle_disposition == HelperLifecycleDisposition::Ignore {
        tracing::info!(
            event_type,
            take_id = event.as_ref().and_then(dictation_take_id_from_event),
            "ignoring helper lifecycle event that does not belong to the active dictation take"
        );
        return;
    }

    let listening_take_id = if event_type == Some("listening_started") {
        event
            .as_ref()
            .and_then(|event| begin_dictation_take(app, event))
    } else {
        None
    };
    match (event_type, event.as_ref()) {
        (Some("recording_discarded"), Some(event)) => {
            if let Some(take_id) = helper_discard_cancellation_target(event) {
                cancel_dictation_take(app, take_id);
            }
        }
        (Some("recording_discarded"), None) => cancel_current_dictation_take(app),
        _ => {}
    }

    if let (Some(state), Some(event)) = (app.try_state::<HelperState>(), event.as_ref()) {
        consume_started_composer_registration(&state, event_type, event);
    }
    handle_shortcut_composer_response(app, event_type, event.as_ref());
    if lifecycle_disposition == HelperLifecycleDisposition::EmitOnly {
        if let Some(event) = event {
            emit_helper_event_without_lifecycle(app, event);
        }
        return;
    }

    if matches!(
        event_type,
        Some("hotkey_trigger_ready" | "hotkey_trigger_unavailable")
    ) {
        if let (Some(status), Some(event)) = (app.try_state::<HotkeyStatus>(), event.as_ref()) {
            set_hotkey_status(&status, event.clone());
        }
    }

    handle_shortcut_key_event(app, event_type, event.as_ref());
    let correlated_terminal_transition = matches!(
        event_type,
        Some("recording_discarded" | "final_transcript" | "error")
    ) && event
        .as_ref()
        .and_then(dictation_take_id_from_event)
        .is_some();
    if !correlated_terminal_transition
        && matches!(
            event_type,
            Some(
                "listening_started"
                    | "finalizing_transcript"
                    | "recording_discarded"
                    | "final_transcript"
                    | "error"
            )
        )
    {
        acknowledge_shortcut_toggle(app);
    }
    match event_type {
        Some("listening_started") => {
            let helper_reported_take_id = event.as_ref().and_then(dictation_take_id_from_event);
            update_shortcut_helper_listening(app, true, helper_reported_take_id);
            if helper_reported_take_id.is_none() {
                if let Some(take_id) = listening_take_id.as_deref() {
                    // Preserve legacy protocol negotiation while promoting the
                    // coordinator-owned pending ID to confirmed ownership.
                    promote_untagged_shortcut_helper_start(app, take_id);
                }
            }
            update_shortcut_helper_finalizing(app, false);
        }
        Some("finalizing_transcript") => {
            update_shortcut_helper_listening(app, false, None);
            update_shortcut_helper_finalizing(app, true);
        }
        Some("recording_discarded" | "final_transcript" | "error")
            if !correlated_terminal_transition =>
        {
            update_shortcut_helper_listening(app, false, None);
            update_shortcut_helper_finalizing(app, false);
        }
        _ => {}
    }

    // A matching Escape/X cancel was reset before generic lifecycle handling,
    // so a stale terminal event cannot clear the replacement take's flags.
    match (event_type, event.as_ref()) {
        (Some("recording_discarded"), None) => reset_shortcut_activation(app),
        (Some("final_transcript" | "error"), _) if !correlated_terminal_transition => {
            clear_shortcut_helper_take(app)
        }
        _ => {}
    }

    if matches!(event_type, Some("recording_ready")) {
        if let Some(event) = event.as_ref() {
            match recording_ready_info_from_event(event) {
                Ok(info) => {
                    let finalized_take = app
                        .try_state::<DictationTakeState>()
                        .map(|state| state.claim_finalized(info.take_id.as_deref()))
                        .unwrap_or(DictationFinalizedTake::Stale);
                    match finalized_take {
                        DictationFinalizedTake::Start(take) => {
                            let app = app.clone();
                            tauri::async_runtime::spawn(async move {
                                transcribe_recording_ready(app, info, take).await;
                            });
                        }
                        DictationFinalizedTake::Cancelled | DictationFinalizedTake::Stale => {
                            tracing::info!(
                                take_id = info.take_id.as_deref(),
                                "discarding recording from a cancelled or stale dictation take"
                            );
                            let _ = std::fs::remove_file(&info.audio_path);
                            return;
                        }
                        DictationFinalizedTake::Duplicate => {
                            tracing::warn!(
                                take_id = info.take_id.as_deref(),
                                "ignoring duplicate ready event for a dictation take"
                            );
                            return;
                        }
                    }
                }
                Err(error) => {
                    let cleanup = malformed_recording_ready_cleanup(app, event);
                    cancel_dictation_take(app, cleanup.take_id.as_deref());
                    if cleanup.send_helper_discard {
                        let mut command = serde_json::json!({ "type": "discard_recording" });
                        if let (Some(take_id), Some(object)) =
                            (cleanup.take_id.as_deref(), command.as_object_mut())
                        {
                            object.insert("takeId".into(), serde_json::json!(take_id));
                        }
                        let state = app.state::<HelperState>();
                        let _ = send_helper_command(&state, command);
                    }
                    if let Some(take_id) = cleanup.take_id.as_deref() {
                        // `recording_ready` already holds the helper-command
                        // lock, so correlate directly instead of recursively
                        // acquiring it through `emit_dictation_take_id_event`.
                        let mut event = app_error_event(error);
                        add_dictation_take_id_to_event(&mut event, take_id);
                        if dictation_delivery_matches_active_take(app, take_id) {
                            emit_dictation_event_value(app, event);
                        } else {
                            emit_helper_event_without_lifecycle(app, event);
                        }
                    } else {
                        emit_dictation_event_value(app, app_error_event(error));
                    }
                }
            }
        }
    }

    // Route through emit_dictation_event_value so error events get the
    // `payload.silent` annotation from a single classification site.
    if let (Some(take_id), Some(event)) = (listening_take_id.as_deref(), event.as_mut()) {
        if dictation_take_id_from_event(event).is_none() {
            // Normalize only after internal protocol handling so an older
            // helper is not mistaken for one that emits take IDs itself.
            add_dictation_take_id_to_event(event, take_id);
        }
    }
    if let Some(event) = event {
        emit_dictation_event_value(app, event);
    } else {
        update_latest_event(app, event_type, Some(line.clone()));
        update_hud_window(app, event_type, None);
        let _ = app.emit("dictation-event", line);
    }
}

fn handle_shortcut_composer_response(
    app: &AppHandle,
    event_type: Option<&str>,
    event: Option<&serde_json::Value>,
) {
    let Some(payload) = event.and_then(|event| event.get("payload")) else {
        return;
    };
    let Some(request_id) = payload
        .get("composerRequestId")
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    if event_type == Some("listening_started") {
        clear_shortcut_composer_request(app, request_id);
    } else if event_type == Some("error")
        && matches!(
            payload.get("code").and_then(serde_json::Value::as_str),
            Some("composer_delivery_busy" | "recording_start_busy" | "recording_start_failed")
        )
    {
        if let Some(state) = app.try_state::<ShortcutActivationState>() {
            if let Ok(mut controller) = state.controller.lock() {
                controller.reset_for_rejected_composer_request(request_id);
            }
        }
    }
}

fn clear_shortcut_composer_request(app: &AppHandle, request_id: &str) {
    if let Some(state) = app.try_state::<ShortcutActivationState>() {
        if let Ok(mut controller) = state.controller.lock() {
            controller.clear_composer_request(request_id);
        }
    }
}

fn consume_started_composer_registration(
    state: &HelperState,
    event_type: Option<&str>,
    event: &serde_json::Value,
) {
    if event_type != Some("listening_started") {
        return;
    }
    let Some(request_id) = event
        .get("payload")
        .and_then(|payload| payload.get("composerRequestId"))
        .and_then(serde_json::Value::as_str)
    else {
        return;
    };
    if let Ok(mut registration) = state.composer_registration.lock() {
        if registration
            .as_ref()
            .is_some_and(|registration| registration.request_id == request_id)
        {
            *registration = None;
        }
    }
}

async fn transcribe_recording_ready(
    app: AppHandle,
    recording: RecordingReadyInfo,
    take: DictationTakeToken,
) {
    transcribe_recording_ready_inner(&app, recording, &take).await;
}

fn add_dictation_take_id(command: &mut serde_json::Value, claim: &DictationDeliveryClaim) {
    if let Some(object) = command.as_object_mut() {
        object.insert("takeId".into(), serde_json::json!(claim.take.id()));
    }
}

fn add_dictation_take_id_to_event(event: &mut serde_json::Value, take_id: &str) {
    if let Some(payload) = event
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
    {
        payload.insert("takeId".into(), serde_json::json!(take_id));
    }
}

fn emit_dictation_take_event(app: &AppHandle, take: &DictationTakeToken, event: serde_json::Value) {
    emit_dictation_take_id_event(app, take.id(), event);
}

fn emit_dictation_take_id_event(app: &AppHandle, take_id: &str, mut event: serde_json::Value) {
    add_dictation_take_id_to_event(&mut event, take_id);
    let take_state = app.try_state::<DictationTakeState>();
    let _helper_command_guard = take_state
        .as_ref()
        .map(|state| state.lock_helper_commands());
    if dictation_delivery_matches_active_take(app, take_id) {
        emit_dictation_event_value(app, event);
    } else {
        emit_helper_event_without_lifecycle(app, event);
    }
}

fn send_claimed_dictation_take_command(
    app: &AppHandle,
    claim: &DictationDeliveryClaim,
    mut command: serde_json::Value,
) -> Result<(), AppError> {
    add_dictation_take_id(&mut command, claim);
    let state = app.state::<HelperState>();
    send_helper_command(&state, command)
}

fn send_dictation_take_command(
    app: &AppHandle,
    take: &DictationTakeToken,
    command: serde_json::Value,
) -> Option<Result<(), AppError>> {
    let claim = take.try_begin_delivery()?;
    Some(send_claimed_dictation_take_command(app, &claim, command))
}

fn remove_cancelled_dictation_audio(
    recording_path: &std::path::Path,
    prepared_path: Option<&std::path::Path>,
) {
    if let Some(prepared_path) = prepared_path.filter(|path| *path != recording_path) {
        let _ = fs::remove_file(prepared_path);
    }
    let _ = fs::remove_file(recording_path);
}

/// Preserve the target captured by the helper before asynchronous transcription.
fn history_target(recording: &RecordingReadyInfo) -> Option<String> {
    recording
        .target_bundle_id
        .clone()
        .filter(|id| !id.trim().is_empty())
}

async fn transcribe_recording_ready_inner(
    app: &AppHandle,
    recording: RecordingReadyInfo,
    take: &DictationTakeToken,
) {
    let audio_path = recording.audio_path.clone();
    // Resolve the paste target before the first await. Prefer the bundle id
    // the helper captured with the recording: the helper pins that same app
    // when the recording stops and pastes into it once we return, so layout
    // and paste share one source of truth even though the awaits below can
    // take many seconds. Fall back to the frontmost app for helper builds
    // that predate the field.
    let app_context = recording
        .target_bundle_id
        .as_deref()
        .map(|bundle_id| is_email_app_bundle(bundle_id).then(|| APP_CONTEXT_EMAIL.to_string()))
        .unwrap_or_else(frontmost_app_context);
    // Pin the owning profile now, like the paste target above: dictation
    // processing can take seconds, and a profile switch mid-flight must not
    // relabel this dictation's history row to the new profile.
    let history_profile = app
        .state::<HelperState>()
        .agent_take
        .lock()
        .ok()
        .and_then(|active| {
            active
                .as_ref()
                .filter(|target| target.take_id == take.id())
                .map(|target| target.profile.clone())
        })
        .unwrap_or_else(|| crate::commands::active_profile(app));
    let history_app = history_target(&recording);
    let low_speech_evidence = recording_has_low_speech_evidence(&recording);
    if low_speech_evidence {
        tracing::info!(
            observed_audio_level = recording.observed_audio_level,
            speech_confidence = recording.speech_confidence,
            speech_analysis_status = ?recording.speech_analysis_status,
            "dictation capture had low speech evidence; automatic paste will be suppressed"
        );
    }
    let selection = &take.inner.transcription_settings;
    let (provider, transcriber) = match dictation_transcription_provider(
        selection.transcription_provider.clone(),
    )
    .and_then(|provider| {
        crate::providers::transcription::effective_transcription_backend(
            selection,
            crate::providers::transcription::ready_directory(),
            crate::providers::local_transcription_supported(),
            || crate::providers::provider_api_key(crate::providers::BuiltinProvider::Groq),
        )
        .map(|backend| (provider, ServiceDictationTranscriber { backend }))
    }) {
        Ok(selected) => selected,
        Err(error) => {
            remove_windows_dictation_audio(&recording.audio_path);
            if send_dictation_take_command(
                app,
                take,
                serde_json::json!({ "type": "discard_recording" }),
            )
            .is_some()
            {
                update_shortcut_helper_finalizing(app, false);
                emit_dictation_take_event(app, take, app_error_event(error));
            }
            let _ = std::fs::remove_file(&audio_path);
            return;
        }
    };
    let current_settings = current_dictation_settings(app).unwrap_or_default();
    let cleanup = DictationCleanupConfig::from_settings(&current_settings);
    let language = current_settings.language;
    let Some(dictionary) =
        run_dictation_take_step(take, transcription_dictionary_for_app(app)).await
    else {
        remove_cancelled_dictation_audio(&audio_path, None);
        return;
    };
    let session_id = dictation_session_id();
    let utterance_id = uuid::Uuid::new_v4().to_string();
    let transcription_audio = match prepare_dictation_audio(&recording.audio_path, &utterance_id) {
        Ok(path) => path,
        Err(error) => {
            remove_windows_dictation_audio(&recording.audio_path);
            if send_dictation_take_command(
                app,
                take,
                serde_json::json!({ "type": "discard_recording" }),
            )
            .is_some()
            {
                update_shortcut_helper_finalizing(app, false);
                emit_dictation_take_event(app, take, app_error_event(error));
            }
            return;
        }
    };
    let pipeline = run_dictation_take_step(
        take,
        run_dictation_pipeline(
            &transcriber,
            &ServiceDictationCleaner,
            DictateTranscribeRequest {
                audio_path: transcription_audio.clone(),
                // Whisper's prompt only steers spelling: it gets the short
                // dictionary hint, and cleanup gets the full dictionary.
                context: dictionary
                    .as_ref()
                    .and_then(|dictionary| dictionary.asr_hint.clone()),
                language,
                session_id,
                utterance_id: utterance_id.clone(),
            },
            &cleanup,
            DictationCleanupContext {
                dictionary_context: dictionary.map(|dictionary| dictionary.context),
                app_context,
            },
            recording.observed_audio_level,
            recording.audio_seconds,
        ),
    )
    .await;
    let Some(pipeline) = pipeline else {
        tracing::info!(
            take_id = take.id(),
            utterance_id,
            "dictation take cancelled after metered dictation transcription started"
        );
        remove_cancelled_dictation_audio(&audio_path, Some(&transcription_audio));
        return;
    };
    if transcription_audio != recording.audio_path {
        let _ = fs::remove_file(&transcription_audio);
    }
    remove_windows_dictation_audio(&recording.audio_path);
    if let Some(error) = pipeline.cleanup_skipped.as_ref() {
        emit_dictation_cleanup_skipped(app, &provider, error);
    }
    let outcome = pipeline.outcome;
    let mut delivery_claim = None;
    let agent_owned = app
        .state::<HelperState>()
        .agent_take
        .lock()
        .ok()
        .is_some_and(|active| {
            active
                .as_ref()
                .is_some_and(|target| target.take_id == take.id())
        });
    let history_already_persisted = if low_speech_evidence || agent_owned {
        let recovery_transcript = outcome
            .transcript
            .as_ref()
            .or(outcome.quarantine_transcript.as_ref());
        if let Some(transcript) = recovery_transcript {
            let staged = run_dictation_take_step(
                take,
                stage_dictation_history_item(
                    app,
                    transcript,
                    &history_profile,
                    history_app.as_deref(),
                ),
            )
            .await;
            let Some(staged) = staged else {
                remove_cancelled_dictation_audio(&audio_path, None);
                return;
            };
            match staged {
                Ok(Some(pending)) => {
                    let Some(claim) = take.try_begin_delivery() else {
                        if let Err(error) = pending.rollback().await {
                            tracing::error!(
                                %error,
                                take_id = take.id(),
                                "cancelled dictation history rollback failed"
                            );
                        }
                        remove_cancelled_dictation_audio(&audio_path, None);
                        return;
                    };
                    match pending.commit().await {
                        Ok(item) => {
                            let _=app.emit("clovy-data-changed",serde_json::json!({"profile":history_profile,"kind":"dictations","id":item.id}));
                            delivery_claim = Some(claim);
                            true
                        }
                        Err(error) => {
                            let error = dictation_history_save_error(error);
                            if agent_owned {
                                retain_agent_history_failure(app, &claim, &error);
                            } else {
                                retain_low_speech_evidence_recording(
                                    app,
                                    &claim,
                                    &audio_path,
                                    transcript,
                                    &error,
                                );
                            }
                            return;
                        }
                    }
                }
                Ok(None) => false,
                Err(error) => {
                    let Some(claim) = take.try_begin_delivery() else {
                        remove_cancelled_dictation_audio(&audio_path, None);
                        return;
                    };
                    if agent_owned {
                        retain_agent_history_failure(app, &claim, &error);
                    } else {
                        retain_low_speech_evidence_recording(
                            app,
                            &claim,
                            &audio_path,
                            transcript,
                            &error,
                        );
                    }
                    return;
                }
            }
        } else {
            false
        }
    } else {
        false
    };
    let outcome = quarantine_low_speech_evidence_outcome(outcome, low_speech_evidence);
    deliver_dictation_outcome(
        app,
        take,
        delivery_claim,
        &audio_path,
        outcome,
        history_already_persisted,
        (&history_profile, history_app.as_deref()),
        recording.composer_request_id.as_deref(),
    );
}

fn retain_agent_history_failure(app: &AppHandle, claim: &DictationDeliveryClaim, error: &AppError) {
    // Keep the stopped take's audio for recovery; do not send clipboard or app
    // insertion commands when an agent's durable history write failed.
    set_agent_dictation_phase(
        &app.state::<HelperState>(),
        claim.take.id(),
        "failed",
        Some(error.message.clone()),
    );
    update_shortcut_helper_finalizing(app, false);
    emit_dictation_take_event(app, &claim.take, app_error_event(error.clone()));
}
fn agent_composer_text(outcome: &DictationTranscriptionOutcome) -> Option<&str> {
    if outcome
        .helper_command
        .get("type")
        .and_then(serde_json::Value::as_str)
        == Some("paste_text")
    {
        return outcome
            .helper_command
            .get("text")
            .and_then(serde_json::Value::as_str)
            .filter(|text| !text.trim().is_empty());
    }
    if outcome
        .event
        .as_ref()
        .and_then(|event| event.get("type"))
        .and_then(serde_json::Value::as_str)
        == Some("agent_session_prompt")
    {
        return outcome
            .transcript
            .as_ref()
            .map(|transcript| transcript.text.as_str());
    }
    None
}

fn deliver_dictation_outcome(
    app: &AppHandle,
    take: &DictationTakeToken,
    delivery_claim: Option<DictationDeliveryClaim>,
    audio_path: &std::path::Path,
    outcome: DictationTranscriptionOutcome,
    history_already_persisted: bool,
    history: (&str, Option<&str>),
    composer_request_id: Option<&str>,
) {
    let mut helper_command = outcome.helper_command.clone();
    let claim = delivery_claim.or_else(|| take.try_begin_delivery());
    let Some(claim) = claim else {
        let _ = std::fs::remove_file(audio_path);
        return;
    };
    // Low-evidence transcripts reach this point only after a successful,
    // awaited history write. Normal transcripts keep the existing best-effort
    // background write so their paste path does not gain database latency.
    if let Some(transcript) = outcome.transcript.as_ref() {
        if !history_already_persisted {
            spawn_dictation_history_write(
                app.clone(),
                transcript.clone(),
                history.0.to_string(),
                history.1.map(str::to_owned),
            );
        }
    }
    let agent_target = app
        .state::<HelperState>()
        .agent_take
        .lock()
        .ok()
        .and_then(|active| {
            active
                .as_ref()
                .filter(|target| target.take_id == take.id())
                .cloned()
        });
    if let Some(target) = agent_target {
        if matches!(target.phase.as_str(), "cancelling" | "cancelled" | "failed") {
            let _ = std::fs::remove_file(audio_path);
            return;
        }
        // Never paste an agent-owned take to the foreground app. Native cleanup
        // and composer delivery are separate, correlated operations.
        let Some(text) = agent_composer_text(&outcome) else {
            set_agent_dictation_phase(
                &app.state::<HelperState>(),
                &target.take_id,
                "failed",
                Some("No deliverable dictation text was produced.".into()),
            );
            let _ = send_claimed_dictation_take_command(
                app,
                &claim,
                serde_json::json!({"type":"discard_recording","takeId":target.take_id}),
            );
            let _ = std::fs::remove_file(audio_path);
            return;
        };
        #[cfg(target_os = "macos")]
        app.state::<DictationTakeState>()
            .remember_last_dictation(&claim, history.0, &outcome);
        set_agent_dictation_phase(
            &app.state::<HelperState>(),
            &target.take_id,
            "delivering",
            None,
        );
        let _ = send_claimed_dictation_take_command(
            app,
            &claim,
            serde_json::json!({"type":"discard_recording","takeId":target.take_id}),
        );

        let result = app.get_webview_window(&target.window).ok_or_else(|| AppError::new("dictation_window_unavailable", "The destination window is unavailable.")).and_then(|window| window.emit("clovy://agent-dictation-delivery", serde_json::json!({"takeId":target.take_id,"composerRequestId":target.composer_request_id,"conversationId":target.conversation_id,"profile":target.profile,"text":text})).map_err(|error| AppError::new("dictation_delivery_failed",error.to_string())));
        if let Err(error) = result {
            set_agent_dictation_phase(
                &app.state::<HelperState>(),
                &target.take_id,
                "failed",
                Some(error.message),
            );
        }
        update_shortcut_helper_finalizing(app, false);
        let _ = std::fs::remove_file(audio_path);
        return;
    }
    if helper_command
        .get("type")
        .and_then(serde_json::Value::as_str)
        == Some("paste_text")
    {
        if let (Some(request_id), Some(object)) =
            (composer_request_id, helper_command.as_object_mut())
        {
            object.insert("composerRequestId".into(), serde_json::json!(request_id));
        }
    }
    #[cfg(target_os = "macos")]
    app.state::<DictationTakeState>()
        .remember_last_dictation(&claim, history.0, &outcome);
    if let Err(error) = send_claimed_dictation_take_command(app, &claim, helper_command) {
        update_shortcut_helper_finalizing(app, false);
        emit_dictation_take_event(app, &claim.take, app_error_event(error));
        let _ = std::fs::remove_file(audio_path);
        return;
    }
    // The terminal command and any shortcut command written after it are
    // consumed by the helper in order. Re-arm as soon as that handoff
    // succeeds instead of waiting for the helper to echo a terminal event:
    // discard outcomes used to emit no echo once recording had finalized,
    // leaving Fn suppressed until the 30-second safety timeout expired.
    update_shortcut_helper_finalizing(app, false);
    if let Some(event) = outcome.event {
        emit_dictation_take_event(app, &claim.take, event);
    }
    let _ = std::fs::remove_file(audio_path);
}

fn retain_low_speech_evidence_recording(
    app: &AppHandle,
    claim: &DictationDeliveryClaim,
    audio_path: &std::path::Path,
    transcript: &TranscriptionProviderResult,
    history_error: &AppError,
) {
    tracing::error!(
        code = %history_error.code,
        audio_path = %audio_path.display(),
        "dictation history save failed; retaining low-evidence recording"
    );
    let command = serde_json::json!({
        "type": "copy_text_for_recovery",
        "text": transcript.text,
        "keepRecordingFile": true,
    });
    match send_claimed_dictation_take_command(app, claim, command) {
        Ok(()) => {
            update_shortcut_helper_finalizing(app, false);
        }
        Err(error) => {
            update_shortcut_helper_finalizing(app, false);
            emit_dictation_take_event(
                app,
                &claim.take,
                app_error_event(AppError::new(
                    "dictation_recovery_unavailable",
                    "Clovy couldn't save this dictation. The recording was kept for recovery.",
                )),
            );
            tracing::error!(
                code = %error.code,
                audio_path = %audio_path.display(),
                "could not hand retained dictation to helper"
            );
        }
    }
}

fn prepare_dictation_audio(
    input_path: &std::path::Path,
    utterance_id: &str,
) -> Result<PathBuf, AppError> {
    #[cfg(target_os = "windows")]
    {
        let output_path =
            std::env::temp_dir().join(format!("clovy-dictation-{utterance_id}-normalized.wav"));
        return normalize_wav_for_transcription(input_path, &output_path);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = utterance_id;
        Ok(input_path.to_path_buf())
    }
}

#[cfg(target_os = "windows")]
fn remove_windows_dictation_audio(path: &std::path::Path) {
    let _ = fs::remove_file(path);
}

#[cfg(not(target_os = "windows"))]
fn remove_windows_dictation_audio(_path: &std::path::Path) {}

fn dictation_session_id() -> String {
    use std::sync::OnceLock;
    static SESSION_ID: OnceLock<String> = OnceLock::new();
    SESSION_ID
        .get_or_init(|| uuid::Uuid::new_v4().to_string())
        .clone()
}

fn dictation_transcription_provider(provider: String) -> Result<String, AppError> {
    if provider == crate::providers::PROVIDER_LOCAL {
        crate::providers::transcription::require_local_ready()?;
        return Ok(provider);
    }
    if provider != PROVIDER_GROQ {
        return Err(AppError::new(
            "dictation_provider_not_configured",
            "Dictation requires a Groq transcription model.",
        ));
    }
    Ok(provider)
}

fn is_email_app_bundle(bundle_id: &str) -> bool {
    EMAIL_APP_BUNDLE_IDS
        .iter()
        .any(|known| bundle_id.eq_ignore_ascii_case(known))
}

/// The app-context slug for the app the user is dictating into, read when
/// dictation stops (the frontmost app is the paste target). None when the
/// frontmost app is not a recognized context or cannot be determined.
#[cfg(target_os = "macos")]
fn frontmost_app_context() -> Option<String> {
    let bundle_id = frontmost_bundle_id()?;
    is_email_app_bundle(&bundle_id).then(|| APP_CONTEXT_EMAIL.to_string())
}

#[cfg(not(target_os = "macos"))]
fn frontmost_app_context() -> Option<String> {
    None
}

#[cfg(target_os = "macos")]
fn frontmost_bundle_id() -> Option<String> {
    let workspace = objc2_app_kit::NSWorkspace::sharedWorkspace();
    let front = workspace.frontmostApplication()?;
    let bundle_id = front.bundleIdentifier()?;
    Some(bundle_id.to_string())
}

async fn transcription_dictionary_for_app(app: &AppHandle) -> Option<TranscriptionDictionary> {
    let repos = crate::commands::repositories(app).await.ok()?;
    let entries = repos.list_dictionary_entries().await.ok()?;
    build_transcription_dictionary(&entries)
}

/// The cleanup choice one take reads once (ditado-rapido D1, D7).
#[derive(Clone, Debug)]
struct DictationCleanupConfig {
    enabled: bool,
    /// OpenRouter model id for the cleanup chat completion.
    model: String,
    style: DictationStyle,
    /// Budget for the whole cleanup of a transcript of this many bytes.
    timeout: fn(usize) -> Duration,
}

impl DictationCleanupConfig {
    fn from_settings(settings: &DictationSettings) -> Self {
        Self {
            enabled: settings.cleanup_enabled,
            model: settings.cleanup_model.clone(),
            style: settings.style,
            timeout: dictation_cleanup_timeout,
        }
    }
}

/// What the cleanup model gets besides the transcript.
#[derive(Clone, Debug, Default)]
struct DictationCleanupContext {
    dictionary_context: Option<String>,
    app_context: Option<String>,
}

/// One cleanup chat completion for one chunk of a transcript.
#[derive(Clone, Debug)]
struct DictationCleanupCall {
    text: String,
    dictionary_context: Option<String>,
    app_context: Option<String>,
    style: DictationStyle,
    model: String,
    session_id: String,
    utterance_id: String,
}

/// Turns one finished recording into text using the captured backend.
trait DictationTranscriber {
    fn transcribe(
        &self,
        request: DictateTranscribeRequest,
    ) -> impl Future<Output = Result<TranscriptionProviderResult, AppError>> + Send;
}

/// Cleans one chunk of a transcript with a single chat completion.
trait DictationCleaner {
    fn clean(
        &self,
        call: DictationCleanupCall,
    ) -> impl Future<Output = Result<String, AppError>> + Send;
}

/// The production transcriber.
struct ServiceDictationTranscriber {
    backend: crate::providers::transcription::TranscriptionBackend,
}

impl DictationTranscriber for ServiceDictationTranscriber {
    async fn transcribe(
        &self,
        request: DictateTranscribeRequest,
    ) -> Result<TranscriptionProviderResult, AppError> {
        match &self.backend {
            crate::providers::transcription::TranscriptionBackend::Groq { key, model } => {
                dictate_transcribe_with(Some(key), model.clone(), request).await
            }
            crate::providers::transcription::TranscriptionBackend::Local { model_dir } => {
                // Deselecting Local may have stopped its helper (D4). Keep this
                // take's backend and propagate that local error to the HUD;
                // never resolve the new provider and upload the captured audio.
                crate::providers::local_transcriber::transcribe(
                    model_dir,
                    &request.audio_path,
                    request.language.as_deref(),
                )
                .await?
                .plain()
            }
        }
    }
}

/// The production cleaner calls the chosen OpenRouter model directly.
struct ServiceDictationCleaner;

impl DictationCleaner for ServiceDictationCleaner {
    fn clean(
        &self,
        call: DictationCleanupCall,
    ) -> impl Future<Output = Result<String, AppError>> + Send {
        dictate_cleanup(
            DictateCleanupRequestParams {
                text: call.text,
                dictionary_context: call.dictionary_context,
                app_context: call.app_context,
                style: call.style.instruction().to_string(),
                session_id: call.session_id,
                utterance_id: call.utterance_id,
            },
            call.model,
        )
    }
}

struct DictationPipelineResult {
    outcome: DictationTranscriptionOutcome,
    /// Why cleanup was skipped; the raw transcript was used instead.
    cleanup_skipped: Option<AppError>,
}

/// Transcribe one take and, only when cleanup is on, clean the transcript.
/// A failed or timed-out cleanup keeps the whole raw transcript.
async fn run_dictation_pipeline<T, C>(
    transcriber: &T,
    cleaner: &C,
    request: DictateTranscribeRequest,
    cleanup: &DictationCleanupConfig,
    context: DictationCleanupContext,
    observed_audio_level: Option<f32>,
    audio_seconds: Option<f32>,
) -> DictationPipelineResult
where
    T: DictationTranscriber + Sync,
    C: DictationCleaner + Sync,
{
    if audio_seconds.is_some_and(|seconds| seconds < DICTATION_MIN_UPLOAD_SECONDS) {
        // Whisper invents text ("Thank you.") for near-empty audio; such a
        // take is no speech without an upload (design D5, R7).
        tracing::info!(audio_seconds, "dictation take too short to transcribe");
        return DictationPipelineResult {
            outcome: whisper_no_speech_outcome(observed_audio_level),
            cleanup_skipped: None,
        };
    }
    let session_id = request.session_id.clone();
    let utterance_id = request.utterance_id.clone();
    let mut cleanup_skipped = None;
    let result = match transcriber.transcribe(request).await {
        Ok(mut transcript) if cleanup.enabled && !is_blank_transcript(&transcript.text) => {
            let transcript_bytes = transcript.text.trim().len();
            tracing::info!(
                model = %cleanup.model,
                style = ?cleanup.style,
                app_context = context.app_context.as_deref(),
                transcript_bytes,
                timeout_ms = (cleanup.timeout)(transcript_bytes).as_millis() as u64,
                "dictation cleanup starting",
            );
            let started_at = Instant::now();
            match clean_dictation_transcript(
                cleaner,
                &transcript.text,
                cleanup,
                &context,
                &session_id,
                &utterance_id,
            )
            .await
            {
                Ok(cleaned) => {
                    if !cleaned.trim().is_empty() {
                        transcript.text = cleaned;
                    }
                    tracing::info!(
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        "dictation cleanup applied",
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        code = %error.code,
                        message = %error.message,
                        transcript_bytes,
                        elapsed_ms = started_at.elapsed().as_millis() as u64,
                        "dictation cleanup skipped; pasting raw transcript",
                    );
                    cleanup_skipped = Some(error);
                }
            }
            Ok(transcript)
        }
        other => other,
    };
    DictationPipelineResult {
        outcome: if cleanup.enabled {
            outcome_from_transcription_result(result, observed_audio_level, cleanup.style)
        } else {
            outcome_from_raw_transcription_result(result, observed_audio_level)
        },
        cleanup_skipped,
    }
}

/// Budget for one cleanup round-trip, scaled to the transcript so long
/// dictations get the generation time they need instead of silently falling
/// back to raw ASR text. Byte length is a fine proxy: multibyte scripts also
/// cost more tokens to regenerate.
fn dictation_cleanup_timeout(text_bytes: usize) -> Duration {
    let scaled = DICTATION_CLEANUP_BASE_TIMEOUT_MS
        .saturating_add((text_bytes as u64).saturating_mul(DICTATION_CLEANUP_TIMEOUT_MS_PER_BYTE));
    Duration::from_millis(scaled.min(DICTATION_CLEANUP_MAX_TIMEOUT_MS))
}

/// Clean a transcript within its budget. Long transcripts are cleaned a
/// word-boundary chunk at a time, up to [`DICTATION_CLEANUP_PARALLEL_CHUNKS`]
/// at once, and rejoined in order: the cleanup model reliably punctuates
/// short passages but degrades into an unpunctuated verbatim echo on long
/// run-on speech (JUN-212). Each chunk gets its own utterance id suffix so
/// metering treats it as its own unit of work. Any chunk that fails or runs
/// out of the budget fails the whole cleanup, so the caller pastes the raw
/// transcript rather than a mix (ditado-rapido D7).
async fn clean_dictation_transcript<C: DictationCleaner + Sync>(
    cleaner: &C,
    text: &str,
    config: &DictationCleanupConfig,
    context: &DictationCleanupContext,
    session_id: &str,
    utterance_id: &str,
) -> Result<String, AppError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(String::new());
    }
    let call = |chunk: &str, utterance_id: String| DictationCleanupCall {
        text: chunk.to_string(),
        dictionary_context: context.dictionary_context.clone(),
        app_context: context.app_context.clone(),
        style: config.style,
        model: config.model.clone(),
        session_id: session_id.to_string(),
        utterance_id,
    };
    let chunks = split_dictation_cleanup_chunks(text, DICTATION_CLEANUP_CHUNK_TARGET_BYTES);
    let work = async {
        if chunks.len() == 1 {
            let cleaned =
                cleanup_dictation_call(cleaner, call(text, utterance_id.to_string())).await?;
            return Ok(trim_cleaned_dictation(&cleaned));
        }
        // Built eagerly: a lazily mapped stream trips the higher-ranked
        // `Send` check of the spawned take future.
        let calls: Vec<_> = chunks
            .iter()
            .enumerate()
            .map(|(index, chunk)| {
                cleanup_dictation_call(cleaner, call(chunk, format!("{utterance_id}-c{index}")))
            })
            .collect();
        let cleaned: Vec<String> = futures_util::stream::iter(calls)
            .buffered(DICTATION_CLEANUP_PARALLEL_CHUNKS)
            .map_ok(|cleaned| trim_cleaned_dictation_chunk(&cleaned))
            .try_collect()
            .await?;
        tracing::info!(chunk_count = chunks.len(), "dictation cleanup ran chunked");
        Ok(join_cleaned_dictation_chunks(&cleaned))
    };
    tokio::time::timeout((config.timeout)(text.len()), work)
        .await
        .unwrap_or_else(|_| {
            Err(AppError::new(
                "dictation_cleanup_timeout",
                "Dictation cleanup timed out.",
            ))
        })
}

/// Trim a single-call cleanup result (the historical behavior).
fn trim_cleaned_dictation(cleaned: &str) -> String {
    cleaned.trim().to_string()
}

/// Trim a cleaned chunk for joining: drop surrounding spaces but keep
/// newlines on either edge, so a line or paragraph break the model placed at
/// a chunk boundary survives the join whether it landed at the end of one
/// chunk or the start of the next.
fn trim_cleaned_dictation_chunk(cleaned: &str) -> String {
    cleaned
        .trim_start_matches([' ', '\t'])
        .trim_end_matches([' ', '\t'])
        .to_string()
}

/// Join cleaned chunks with a single space, except across a boundary that
/// already carries a line break (a space would indent the line) or where the
/// next chunk starts with punctuation that attaches to the previous word
/// (a spoken "comma" or "period" right after the split point).
fn join_cleaned_dictation_chunks(chunks: &[String]) -> String {
    let mut joined = String::new();
    for chunk in chunks {
        if chunk.is_empty() {
            continue;
        }
        if !joined.is_empty()
            && !joined.ends_with('\n')
            && !chunk.starts_with('\n')
            && !chunk.starts_with([',', '.', '!', '?', ':', ';', ')', ']'])
        {
            joined.push(' ');
        }
        joined.push_str(chunk);
    }
    joined
}

/// One cleanup chat completion plus content-quality checks, capped at the
/// per-request billing hold (`DICTATION_CLEANUP_REQUEST_MAX_TIMEOUT_MS`).
async fn cleanup_dictation_call<C: DictationCleaner + Sync>(
    cleaner: &C,
    call: DictationCleanupCall,
) -> Result<String, AppError> {
    let cleaned = tokio::time::timeout(
        Duration::from_millis(DICTATION_CLEANUP_REQUEST_MAX_TIMEOUT_MS),
        cleaner.clean(call),
    )
    .await
    .unwrap_or_else(|_| {
        Err(AppError::new(
            "dictation_cleanup_timeout",
            "Dictation cleanup timed out.",
        ))
    })?;
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return Err(AppError::new(
            "provider_response_invalid",
            "Dictation cleanup response did not contain text output.",
        ));
    }
    if looks_like_instruction_response(trimmed) {
        return Err(AppError::new(
            "dictation_cleanup_invalid",
            "Dictation cleanup returned an instruction response.",
        ));
    }
    // Callers trim for their context: fully for a single-call cleanup,
    // newline-preserving for chunks about to be joined.
    Ok(cleaned)
}

/// Split a transcript into cleanup-sized chunks at whitespace boundaries.
/// Short transcripts come back as a single borrowed chunk (the exact input),
/// keeping the single-call path byte-identical to the unchunked behavior.
/// A chunk may run past the target until the next whitespace; text with no
/// whitespace at all (e.g. unsegmented CJK) stays one chunk.
fn split_dictation_cleanup_chunks(text: &str, target_bytes: usize) -> Vec<&str> {
    if text.len() <= target_bytes {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut chunk_start = 0;
    let mut last_whitespace: Option<usize> = None;
    for (index, character) in text.char_indices() {
        if character.is_whitespace() {
            last_whitespace = Some(index);
        }
        if index - chunk_start >= target_bytes {
            if let Some(cut) = last_whitespace.filter(|cut| *cut > chunk_start) {
                chunks.push(text[chunk_start..cut].trim());
                chunk_start = cut + character_width_at(text, cut);
                last_whitespace = None;
            }
        }
    }
    if chunk_start < text.len() {
        let tail = text[chunk_start..].trim();
        if !tail.is_empty() {
            chunks.push(tail);
        }
    }
    if chunks.is_empty() {
        vec![text]
    } else {
        chunks
    }
}

fn character_width_at(text: &str, index: usize) -> usize {
    text[index..].chars().next().map_or(1, char::len_utf8)
}

/// The paste fell back to the raw ASR transcript; the pipeline already
/// logged why. This event lets the HUD show a discreet notice.
fn emit_dictation_cleanup_skipped(app: &AppHandle, provider: &str, error: &AppError) {
    emit_dictation_event_value(
        app,
        serde_json::json!({
            "type": "dictation_cleanup_skipped",
            "payload": {
                "provider": provider,
                "code": &error.code,
                "message": &error.message,
            },
        }),
    );
}

fn looks_like_instruction_response(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    looks_like_report_summary_response(&normalized)
        || normalized.starts_with("sure")
        || looks_like_here_prefaced_instruction_response(&normalized)
        || normalized.starts_with("summary:")
        || normalized.starts_with("the transcript ")
        || normalized.starts_with("the user expresses")
        || normalized.starts_with("the user did")
        || normalized.starts_with("the user asks")
        || normalized.starts_with("i can")
        || normalized.starts_with("i'll")
        || normalized.starts_with("i will")
        || normalized.contains(" the transcript ")
        || normalized.contains(" the user expresses")
        || normalized.contains(" the user did")
        || normalized.contains(" the user asks")
        || normalized.contains(" did not ask ")
        || normalized.contains(" only shared ")
        || normalized.contains(" writing style ")
        || normalized.contains(" tone is ")
        || normalized.contains(" filler sounds ")
        || normalized.contains(" custom terms ")
        || normalized.contains(" spelled correctly ")
        || normalized.contains("rewritten text")
        || normalized.contains("normalized transcript")
}

fn looks_like_here_prefaced_instruction_response(normalized: &str) -> bool {
    let normalized = normalized.replace('’', "'");
    let normalized = normalized.as_str();
    let structural_preamble_end = [":", "\n", " - ", " — ", " – ", "—", "–"]
        .iter()
        .filter_map(|separator| normalized.find(separator))
        .min();
    if structural_preamble_end
        .is_some_and(|end| is_terminal_here_instruction_preamble(&normalized[..end]))
    {
        return true;
    }
    let punctuation_preamble_end = [". ", "! ", "? "]
        .iter()
        .filter_map(|separator| normalized.find(separator))
        .min();
    if punctuation_preamble_end
        .is_some_and(|end| is_terminal_here_instruction_preamble(&normalized[..end]))
    {
        return true;
    }
    if normalized
        .find(", ")
        .is_some_and(|end| is_terminal_here_instruction_preamble(&normalized[..end]))
    {
        return true;
    }
    is_terminal_here_instruction_preamble(normalized)
}

fn is_terminal_here_instruction_preamble(normalized: &str) -> bool {
    let preamble = normalized.trim_end_matches(['.', '!', '?']).trim_end();
    if is_generic_here_instruction_preamble(preamble) {
        return true;
    }
    let Some(subject) = ["here is ", "here's ", "here are "]
        .iter()
        .find_map(|prefix| preamble.strip_prefix(prefix))
    else {
        return false;
    };
    let words: Vec<&str> = subject
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    words.iter().copied().any(|word| {
        is_here_instruction_subject_marker(word) || is_here_instruction_cleanup_modifier(word)
    }) && words.iter().all(|word| {
        is_here_instruction_subject_marker(word)
            || is_here_instruction_cleanup_modifier(word)
            || matches!(
                *word,
                "a" | "an"
                    | "and"
                    | "as"
                    | "below"
                    | "for"
                    | "per"
                    | "request"
                    | "requested"
                    | "the"
                    | "up"
                    | "you"
                    | "your"
            )
    })
}

fn is_here_instruction_subject_marker(word: &str) -> bool {
    matches!(
        word,
        "transcript"
            | "transcription"
            | "correction"
            | "corrections"
            | "dictation"
            | "text"
            | "notes"
            | "version"
            | "result"
            | "content"
            | "output"
            | "message"
            | "email"
            | "response"
            | "copy"
            | "draft"
            | "improvements"
            | "rewrite"
            | "summary"
    )
}

fn is_here_instruction_cleanup_modifier(word: &str) -> bool {
    matches!(
        word,
        "clean"
            | "cleaned"
            | "corrected"
            | "final"
            | "normalized"
            | "polished"
            | "punctuated"
            | "rewritten"
    )
}

fn is_generic_here_instruction_preamble(preamble: &str) -> bool {
    if matches!(
        preamble,
        "here you go"
            | "here it is"
            | "here you are"
            | "here is what i heard"
            | "here's what i heard"
            | "here is what i got"
            | "here's what i got"
            | "here is what i have"
            | "here's what i have"
            | "here is what you said"
            | "here's what you said"
            | "here is what you asked for"
            | "here's what you asked for"
            | "here is what you requested"
            | "here's what you requested"
            | "here is what you dictated"
            | "here's what you dictated"
    ) {
        return true;
    }
    if [
        "here it is with ",
        "here you go with ",
        "here you are with ",
    ]
    .iter()
    .any(|prefix| preamble.starts_with(prefix))
    {
        return true;
    }
    if let Some(subject) = preamble.strip_prefix("here it is ") {
        let words: Vec<&str> = subject
            .split(|character: char| !character.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        if words
            .iter()
            .copied()
            .any(is_here_instruction_cleanup_modifier)
            && words.iter().all(|word| {
                is_here_instruction_cleanup_modifier(word)
                    || matches!(*word, "as" | "for" | "requested" | "up" | "you")
            })
        {
            return true;
        }
    }
    (preamble.starts_with("here, i") || preamble.starts_with("here i"))
        && preamble
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|word| {
                matches!(
                    word,
                    "cleaned"
                        | "corrected"
                        | "normalized"
                        | "polished"
                        | "punctuated"
                        | "rewritten"
                )
            })
}

fn looks_like_report_summary_response(normalized: &str) -> bool {
    normalized.starts_with("bug report assessment")
        || normalized.starts_with("user report summary")
        || normalized.starts_with("what likely happened")
        || normalized.starts_with("likely affected component")
        || normalized.starts_with("notes for the team")
        || normalized.contains(" user report summary")
        || normalized.contains(" bug report assessment")
        || normalized.contains(" what likely happened")
        || normalized.contains(" likely affected component")
        || normalized.contains(" notes for the team")
        || ((normalized.starts_with("user performed ")
            || normalized.starts_with("user reported ")
            || normalized.starts_with("the user performed")
            || normalized.starts_with("the user reported"))
            && (normalized.contains("long dictation")
                || normalized.contains("instead of producing")
                || normalized.contains("summary-like")
                || normalized.contains("transcription pipeline")))
}

fn spawn_dictation_history_write(
    app: AppHandle,
    transcript: TranscriptionProviderResult,
    profile: String,
    app_bundle_id: Option<String>,
) {
    tauri::async_runtime::spawn(async move {
        store_dictation_history_item(&app, &transcript, &profile, app_bundle_id.as_deref()).await;
    });
}

fn recording_ready_info_from_event(
    event: &serde_json::Value,
) -> Result<RecordingReadyInfo, AppError> {
    let path = event
        .get("payload")
        .and_then(|payload| payload.get("path"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "dictation_recording_path_missing",
                "Dictation helper did not provide a recording path.",
            )
        })?;
    Ok(RecordingReadyInfo {
        audio_path: PathBuf::from(path),
        take_id: dictation_take_id_from_event(event).map(str::to_string),
        observed_audio_level: observed_audio_level_from_event(event),
        audio_seconds: finite_f32_from_event_payload(event, "audioDurationSeconds")
            .filter(|seconds| *seconds >= 0.0),
        speech_confidence: speech_confidence_from_event(event),
        speech_analysis_status: speech_analysis_status_from_event(event),
        target_bundle_id: event
            .get("payload")
            .and_then(|payload| payload.get("targetBundleIdentifier"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|bundle_id| !bundle_id.is_empty())
            .map(str::to_string),
        composer_request_id: event
            .get("payload")
            .and_then(|payload| payload.get("composerRequestId"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty() && id.len() <= 128)
            .map(str::to_string),
    })
}

fn observed_audio_level_from_event(event: &serde_json::Value) -> Option<f32> {
    let level = finite_f32_from_event_payload(event, "observedAudioLevel")?;
    (0.0..=1.0).contains(&level).then_some(level)
}

fn speech_confidence_from_event(event: &serde_json::Value) -> Option<f32> {
    let confidence = finite_f32_from_event_payload(event, "speechConfidence")?;
    (0.0..=1.0).contains(&confidence).then_some(confidence)
}

fn speech_analysis_status_from_event(event: &serde_json::Value) -> Option<SpeechAnalysisStatus> {
    let status = event
        .get("payload")
        .and_then(|payload| payload.get("speechAnalysisStatus"))
        .and_then(serde_json::Value::as_str)?;
    match status {
        "ok" => Some(SpeechAnalysisStatus::Ok),
        "no_complete_window" => Some(SpeechAnalysisStatus::NoCompleteWindow),
        "unavailable" => Some(SpeechAnalysisStatus::Unavailable),
        _ => None,
    }
}

fn finite_f32_from_event_payload(event: &serde_json::Value, key: &str) -> Option<f32> {
    let value = event.get("payload").and_then(|payload| payload.get(key))?;
    let level = value
        .as_f64()
        .map(|level| level as f32)
        .or_else(|| value.as_str().and_then(|level| level.parse::<f32>().ok()))?;
    if level.is_finite() {
        Some(level)
    } else {
        None
    }
}

#[derive(Debug)]
struct DictationTranscriptionOutcome {
    helper_command: serde_json::Value,
    event: Option<serde_json::Value>,
    transcript: Option<TranscriptionProviderResult>,
    quarantine_transcript: Option<TranscriptionProviderResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpeechAnalysisStatus {
    Ok,
    NoCompleteWindow,
    Unavailable,
}

#[derive(Debug)]
struct RecordingReadyInfo {
    audio_path: PathBuf,
    /// Length of the finished file. Missing from older helpers and Windows.
    audio_seconds: Option<f32>,
    /// Opaque helper-generated identity for this capture. Older helper builds
    /// omit it and use the coordinator's single active legacy token.
    take_id: Option<String>,
    observed_audio_level: Option<f32>,
    /// Maximum confidence for the macOS built-in `speech` class over the
    /// finalized capture. Missing when analysis failed, produced no complete
    /// window, or came from a helper version/platform without Sound Analysis.
    speech_confidence: Option<f32>,
    speech_analysis_status: Option<SpeechAnalysisStatus>,
    /// Bundle id of the paste target the helper pinned when the recording
    /// stopped: the same app it will paste into once we hand back a
    /// transcript. None with older helper builds that predate the field.
    target_bundle_id: Option<String>,
    /// Exact in-app delivery request retained by the Windows helper.
    composer_request_id: Option<String>,
}

fn recording_has_low_speech_evidence(recording: &RecordingReadyInfo) -> bool {
    match recording.speech_analysis_status {
        // A sub-window capture has no classifier evidence in either direction.
        // Quarantine it regardless of meter level rather than allowing a loud
        // transient to bypass the incomplete analysis.
        Some(SpeechAnalysisStatus::NoCompleteWindow) => true,
        Some(SpeechAnalysisStatus::Unavailable) => false,
        Some(SpeechAnalysisStatus::Ok) | None => {
            recording
                .speech_confidence
                .is_some_and(|confidence| confidence < DICTATION_SPEECH_CONFIDENCE_THRESHOLD)
                && recording
                    .observed_audio_level
                    .is_some_and(|level| level < DICTATION_AUDIO_ACTIVITY_THRESHOLD)
        }
    }
}

fn quarantine_low_speech_evidence_outcome(
    mut outcome: DictationTranscriptionOutcome,
    low_speech_evidence: bool,
) -> DictationTranscriptionOutcome {
    if !low_speech_evidence {
        return outcome;
    }
    let Some(transcript) = outcome
        .transcript
        .clone()
        .or_else(|| outcome.quarantine_transcript.clone())
    else {
        return outcome;
    };

    // A generic speech classifier cannot prove that a quiet capture contains
    // no real words. Keep the ASR result in history, but leave the clipboard
    // and active app untouched and dismiss the no-audio path silently.
    outcome.helper_command = serde_json::json!({ "type": "discard_recording" });
    outcome.event = Some(silent_no_speech_event());
    outcome.transcript = Some(transcript);
    outcome.quarantine_transcript = None;
    outcome
}

fn outcome_from_transcription_result(
    result: Result<TranscriptionProviderResult, AppError>,
    observed_audio_level: Option<f32>,
    style: DictationStyle,
) -> DictationTranscriptionOutcome {
    match result {
        // Whisper heard nothing (design D5): the only "no speech" signal.
        // The meter level is logged for diagnosis and decides nothing.
        Ok(transcript) if is_blank_transcript(&transcript.text) => {
            whisper_no_speech_outcome(observed_audio_level)
        }
        Err(error) if is_whisper_no_speech_error(&error) => {
            whisper_no_speech_outcome(observed_audio_level)
        }
        Ok(mut transcript) => {
            let had_speech_before_cleaning = !transcript.text.trim().is_empty();
            transcript.text = clean_dictation_fillers(&transcript.text, style);
            if had_speech_before_cleaning && transcript.text.trim().is_empty() {
                // Filler-only capture (e.g. "Um, uh."): the provider DID return
                // speech that we stripped to nothing. Discard quietly with a
                // silent no_speech event, regardless of the observed audio
                // level — the audio-detected-but-no-text error is meant for
                // genuine silence misheard as loud audio, not for fillers.
                return silent_no_speech_outcome();
            }
            outcome_from_clean_transcript(transcript, observed_audio_level)
        }
        Err(error) => transcription_error_outcome(error, observed_audio_level),
    }
}

/// Cleanup off (spec "Limpeza desligada"): the Whisper text, only trimmed.
/// No filler backstop and no style; an empty transcript is still no speech.
fn outcome_from_raw_transcription_result(
    result: Result<TranscriptionProviderResult, AppError>,
    observed_audio_level: Option<f32>,
) -> DictationTranscriptionOutcome {
    match result {
        Ok(transcript) if is_blank_transcript(&transcript.text) => {
            whisper_no_speech_outcome(observed_audio_level)
        }
        Err(error) if is_whisper_no_speech_error(&error) => {
            whisper_no_speech_outcome(observed_audio_level)
        }
        Ok(mut transcript) => {
            transcript.text = transcript.text.trim().to_string();
            outcome_from_clean_transcript(transcript, observed_audio_level)
        }
        Err(error) => transcription_error_outcome(error, observed_audio_level),
    }
}

fn transcription_error_outcome(
    error: AppError,
    observed_audio_level: Option<f32>,
) -> DictationTranscriptionOutcome {
    let event =
        promote_silent_error_if_audio_detected(app_error_event(error), observed_audio_level);
    DictationTranscriptionOutcome {
        helper_command: serde_json::json!({ "type": "discard_recording" }),
        event: Some(event),
        transcript: None,
        quarantine_transcript: None,
    }
}

/// No letter or digit in any script: empty, whitespace, or punctuation only.
fn is_blank_transcript(text: &str) -> bool {
    !text.chars().any(char::is_alphanumeric)
}

/// Accept the no-speech error spellings found in historical dictation results.
fn is_whisper_no_speech_error(error: &AppError) -> bool {
    let message = error.message.to_ascii_lowercase();
    error.code == "no_speech"
        || message.contains("no_speech")
        || message.contains("dictation_text_empty")
}

fn whisper_no_speech_outcome(observed_audio_level: Option<f32>) -> DictationTranscriptionOutcome {
    tracing::info!(
        observed_audio_level,
        "dictation transcript was empty; reporting no speech"
    );
    DictationTranscriptionOutcome {
        helper_command: serde_json::json!({ "type": "discard_recording" }),
        event: Some(whisper_no_speech_event()),
        transcript: None,
        quarantine_transcript: None,
    }
}

/// Shown to the user ("No speech detected."), unlike the silent no-speech
/// dismissal of filler-only or quarantined captures.
fn whisper_no_speech_event() -> serde_json::Value {
    serde_json::json!({
        "type": "error",
        "payload": {
            "code": "no_speech",
            "message": "No speech detected.",
            "silent": false,
        },
    })
}

fn silent_no_speech_outcome() -> DictationTranscriptionOutcome {
    DictationTranscriptionOutcome {
        helper_command: serde_json::json!({ "type": "discard_recording" }),
        event: Some(silent_no_speech_event()),
        transcript: None,
        quarantine_transcript: None,
    }
}

fn outcome_from_clean_transcript(
    transcript: TranscriptionProviderResult,
    observed_audio_level: Option<f32>,
) -> DictationTranscriptionOutcome {
    match transcript {
        // Recorded silence / nothing to type is not a failure — discard quietly
        // and emit a `no_speech` event (classified silent, so the HUD just
        // dismisses instead of flashing an error). Don't store an empty item.
        transcript if transcript.text.trim().is_empty() => DictationTranscriptionOutcome {
            helper_command: serde_json::json!({ "type": "discard_recording" }),
            event: Some(no_text_event_for_observed_audio(observed_audio_level)),
            transcript: None,
            quarantine_transcript: None,
        },
        transcript if looks_like_instruction_response(&transcript.text) => {
            DictationTranscriptionOutcome {
                helper_command: serde_json::json!({ "type": "discard_recording" }),
                event: Some(serde_json::json!({
                    "type": "error",
                    "payload": {
                        "code": "dictation_response_invalid",
                        "message": "Dictation returned a summary instead of dictated text. Try again.",
                    },
                })),
                transcript: None,
                quarantine_transcript: Some(transcript),
            }
        }
        transcript => DictationTranscriptionOutcome {
            helper_command: serde_json::json!({
                "type": "paste_text",
                "text": transcript.text.clone(),
            }),
            event: None,
            transcript: Some(transcript),
            quarantine_transcript: None,
        },
    }
}

fn clean_dictation_fillers(text: &str, style: DictationStyle) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }

    let stripped = remove_standalone_fillers(text);
    let cleaned = normalize_after_filler_removal(&stripped.text, stripped.removed);
    if stripped.removed && !contains_meaningful_dictation_word(&cleaned) {
        String::new()
    } else if stripped.removed_leading && style.capitalizes_sentence_starts() {
        // The transcript capitalized the leading filler ("Um, can you…"), so
        // once it's stripped the surviving first word would start lowercase.
        // Restore the capital — but only when the filler actually led the
        // transcript (a mid-sentence filler must not touch a brand/code token
        // like "iPhone"), and only for styles that capitalize sentence starts
        // so we never fight CasualLowercase. Mid-sentence sentence-starts are
        // left as-is; reflowing those is the LLM cleanup pass's job, not this
        // deterministic backstop's.
        capitalize_leading_letter(&cleaned)
    } else {
        cleaned
    }
}

/// Uppercase the first character when it's a lowercase letter, leaving
/// everything else (already-capitalized words, leading punctuation, digits)
/// untouched. Unicode-aware, so accented starts like "é" capitalize too.
fn capitalize_leading_letter(text: &str) -> String {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return text.to_string();
    };
    if !first.is_lowercase() {
        return text.to_string();
    }
    let mut output = String::with_capacity(text.len());
    output.extend(first.to_uppercase());
    output.push_str(chars.as_str());
    output
}

fn remove_standalone_fillers(text: &str) -> RemovedFillers {
    let mut output = String::with_capacity(text.len());
    let mut removed = false;
    let mut removed_leading = false;
    let mut emitted_word = false;
    let mut needs_word_boundary = false;
    let mut index = 0;

    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };

        if !ch.is_ascii_alphabetic() {
            // Honor a pending boundary here too: a filler can be followed by a
            // digit or opening quote/paren (e.g. "um 3", "um \"hi\""), which
            // must keep the separating space the filler removal swallowed.
            if needs_word_boundary && !ch.is_whitespace() && !output.is_empty() {
                output.push(' ');
            }
            needs_word_boundary = false;
            output.push(ch);
            index += ch.len_utf8();
            continue;
        }

        let start = index;
        index += ch.len_utf8();
        while index < text.len() {
            let Some(next) = text[index..].chars().next() else {
                break;
            };
            if !next.is_ascii_alphabetic() {
                break;
            }
            index += next.len_utf8();
        }

        let word = &text[start..index];
        let previous = text[..start].chars().next_back();
        let next = text[index..].chars().next();
        if is_dictation_filler_word(word) && previous != Some('-') && next != Some('-') {
            removed = true;
            // A filler removed before any real word means the transcript led
            // with it ("Um, …"); that's the only case where re-capitalizing the
            // surviving first word is correct.
            if !emitted_word {
                removed_leading = true;
            }
            trim_soft_filler_prefix(&mut output);
            // Removing a filler swallows the whitespace on both sides of it, so
            // re-insert a separator before the next word. A standalone filler
            // sentence ("… Um. …") leaves the prior sentence's terminal
            // punctuation as the last char, so guard on that too — otherwise
            // "Send it. Um. Now." would collapse to "Send it.Now.".
            needs_word_boundary = output
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '!' | '?'));
            index = skip_soft_filler_suffix(text, index);
        } else {
            if needs_word_boundary && !output.ends_with(' ') && !output.is_empty() {
                output.push(' ');
            }
            needs_word_boundary = false;
            emitted_word = true;
            output.push_str(word);
        }
    }

    RemovedFillers {
        text: output,
        removed,
        removed_leading,
    }
}

struct RemovedFillers {
    text: String,
    /// Any standalone filler was stripped.
    removed: bool,
    /// A filler led the transcript and was stripped, exposing a new first word.
    removed_leading: bool,
}

fn trim_soft_filler_prefix(output: &mut String) {
    while output.chars().next_back().is_some_and(char::is_whitespace) {
        output.pop();
    }
    if output.ends_with(',') || output.ends_with(':') || output.ends_with(';') {
        output.pop();
    }
    while output.chars().next_back().is_some_and(char::is_whitespace) {
        output.pop();
    }
}

fn skip_soft_filler_suffix(text: &str, mut index: usize) -> usize {
    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        if ch.is_whitespace() || matches!(ch, ',' | ':' | ';' | '.' | '!' | '?') {
            index += ch.len_utf8();
        } else {
            break;
        }
    }
    index
}

fn normalize_after_filler_removal(text: &str, removed_filler: bool) -> String {
    let mut normalized = collapse_dictation_whitespace(text);
    if removed_filler {
        normalized = normalized
            .trim_start_matches(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ':' | ';'))
            .trim()
            .to_string();
    }
    normalized
}

/// Collapse runs of horizontal whitespace to a single space, but preserve line
/// breaks (and a single blank line between paragraphs). The cleanup prompt now
/// emits paragraph-grouped, line-broken text, and spoken "new line" formatting
/// becomes real breaks — flattening all whitespace would paste/store those as
/// one flat line.
fn collapse_dictation_whitespace(text: &str) -> String {
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    while lines.first().is_some_and(String::is_empty) {
        lines.remove(0);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let mut collapsed: Vec<String> = Vec::with_capacity(lines.len());
    let mut previous_blank = false;
    for line in lines {
        let blank = line.is_empty();
        if blank && previous_blank {
            continue;
        }
        previous_blank = blank;
        collapsed.push(line);
    }
    collapsed.join("\n")
}

fn contains_meaningful_dictation_word(text: &str) -> bool {
    let mut index = 0;
    while index < text.len() {
        let Some(ch) = text[index..].chars().next() else {
            break;
        };
        // Digits are meaningful on their own — an OTP, extension, or amount
        // dictated after a hesitation ("um 123456") must still paste, not
        // discard as if it were filler-only.
        if ch.is_ascii_digit() {
            return true;
        }
        if !ch.is_ascii_alphabetic() {
            index += ch.len_utf8();
            continue;
        }
        let start = index;
        index += ch.len_utf8();
        while index < text.len() {
            let Some(next) = text[index..].chars().next() else {
                break;
            };
            if !next.is_ascii_alphabetic() {
                break;
            }
            index += next.len_utf8();
        }
        let word = &text[start..index];
        if !is_dictation_filler_word(word) && !is_dictation_connector_word(word) {
            return true;
        }
    }
    false
}

fn is_dictation_filler_word(word: &str) -> bool {
    // An all-caps multi-letter token is almost certainly a dictated acronym
    // (e.g. "ER"), not a hesitation — keep it. The capitalized hesitation form
    // ("Er, no.") isn't all-caps, so it still strips.
    if word.len() >= 2 && word.chars().all(|ch| ch.is_ascii_uppercase()) {
        return false;
    }
    matches!(
        word.to_ascii_lowercase().as_str(),
        "um" | "ums"
            | "umm"
            | "ummm"
            | "uh"
            | "uhs"
            | "uhh"
            | "uhhh"
            | "er"
            | "erm"
            | "ah"
            | "ahh"
            | "hmm"
            | "hm"
    )
}

fn is_dictation_connector_word(word: &str) -> bool {
    matches!(word.to_ascii_lowercase().as_str(), "and" | "or" | "then")
}

fn no_text_event_for_observed_audio(observed_audio_level: Option<f32>) -> serde_json::Value {
    if probable_speech_detected(observed_audio_level) {
        return audio_detected_but_no_text_event(None, None, observed_audio_level);
    }
    silent_no_speech_event()
}

fn silent_no_speech_event() -> serde_json::Value {
    serde_json::json!({
        "type": "error",
        "payload": { "code": "no_speech", "message": "No speech detected." },
    })
}

fn promote_silent_error_if_audio_detected(
    event: serde_json::Value,
    observed_audio_level: Option<f32>,
) -> serde_json::Value {
    if probable_speech_detected(observed_audio_level) && is_silent_transcription_error(&event) {
        let payload = event.get("payload");
        let underlying_code = payload
            .and_then(|payload| payload.get("code"))
            .and_then(serde_json::Value::as_str);
        let underlying_message = payload
            .and_then(|payload| payload.get("message"))
            .and_then(serde_json::Value::as_str);
        return audio_detected_but_no_text_event(
            underlying_code,
            underlying_message,
            observed_audio_level,
        );
    }
    event
}

fn audio_detected_but_no_text_event(
    underlying_code: Option<&str>,
    underlying_message: Option<&str>,
    observed_audio_level: Option<f32>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "code": "dictation_audio_without_text",
        "message": "Dictation detected audio but produced no text. Try again.",
    });
    if let Some(level) = observed_audio_level {
        payload["observedAudioLevel"] = serde_json::json!(level);
    }
    if let Some(code) = underlying_code {
        payload["underlyingCode"] = serde_json::json!(code);
    }
    if let Some(message) = underlying_message {
        payload["underlyingMessage"] = serde_json::json!(message);
    }
    serde_json::json!({
        "type": "error",
        "payload": payload,
    })
}

fn probable_speech_detected(observed_audio_level: Option<f32>) -> bool {
    observed_audio_level.is_some_and(|level| level >= DICTATION_AUDIO_ACTIVITY_THRESHOLD)
}

async fn store_dictation_history_item(
    app: &AppHandle,
    transcript: &TranscriptionProviderResult,
    profile: &str,
    app_bundle_id: Option<&str>,
) {
    if let Err(error) =
        persist_dictation_history_item(app, transcript, profile, app_bundle_id).await
    {
        tracing::error!(
            code = %error.code,
            "dictation history save failed after transcript delivery"
        );
    }
}

async fn persist_dictation_history_item(
    app: &AppHandle,
    transcript: &TranscriptionProviderResult,
    profile: &str,
    app_bundle_id: Option<&str>,
) -> Result<(), AppError> {
    let repos = crate::commands::repositories(app).await.map_err(|error| {
        tracing::error!(code = %error.code, "dictation history repository unavailable");
        AppError::new(
            "dictation_history_save_failed",
            "Dictation history could not save this transcript.",
        )
    })?;
    persist_history_with_repositories(&repos, transcript, profile, app_bundle_id).await
}

async fn persist_history_with_repositories(
    repos: &crate::db::repositories::Repositories,
    transcript: &TranscriptionProviderResult,
    profile: &str,
    app_bundle_id: Option<&str>,
) -> Result<(), AppError> {
    repos
        .create_dictation_history_item_with_app(
            profile,
            &transcript.text,
            transcript.language.clone(),
            &transcript.provider,
            app_bundle_id,
        )
        .await
        .map_err(dictation_history_save_error)?;
    Ok(())
}

async fn stage_dictation_history_item(
    app: &AppHandle,
    transcript: &TranscriptionProviderResult,
    data_partition: &str,
    app_bundle_id: Option<&str>,
) -> Result<Option<crate::db::repositories::PendingDictationHistoryItem>, AppError> {
    let repos = crate::commands::repositories(app).await.map_err(|error| {
        tracing::error!(code = %error.code, "dictation history repository unavailable");
        AppError::new(
            "dictation_history_save_failed",
            "Dictation history could not save this transcript.",
        )
    })?;
    stage_history_with_repositories(&repos, transcript, data_partition, app_bundle_id).await
}

async fn stage_history_with_repositories(
    repos: &crate::db::repositories::Repositories,
    transcript: &TranscriptionProviderResult,
    data_partition: &str,
    app_bundle_id: Option<&str>,
) -> Result<Option<crate::db::repositories::PendingDictationHistoryItem>, AppError> {
    repos
        .stage_dictation_history_item_with_app(
            data_partition,
            &transcript.text,
            transcript.language.clone(),
            &transcript.provider,
            app_bundle_id,
        )
        .await
        .map_err(dictation_history_save_error)
}

fn dictation_history_save_error(error: sqlx::error::Error) -> AppError {
    tracing::error!(%error, "dictation history insert failed");
    AppError::new(
        "dictation_history_save_failed",
        "Dictation history could not save this transcript.",
    )
}

fn app_error_event(error: AppError) -> serde_json::Value {
    serde_json::json!({
        "type": "error",
        "payload": {
            "code": error.code,
            "message": error.message,
        },
    })
}

fn emit_dictation_event_value(app: &AppHandle, mut event: serde_json::Value) {
    observe_agent_dictation_event(app, &event);
    sync_update_gate_take(app, &event);
    annotate_silent_error(&mut event);
    let event_type = event.get("type").and_then(serde_json::Value::as_str);
    sync_capture_mic_duck(event_type);
    sync_dictation_indicator(app, event_type);
    append_dictation_event_log(app, event_type, &event);
    let line = event.to_string();
    update_latest_event(app, event_type, Some(line.clone()));
    update_hud_window(app, event_type, Some(&event));
    let _ = app.emit("dictation-event", line);
}

/// Keeps a live meeting recording's microphone out of the dictation's way:
/// while the helper listens, the user's voice belongs to the dictation, not
/// the note, so the capture's mic channel is ducked (silence-filled, keeping
/// the mic and system tracks aligned — see capture::set_mic_ducked) and
/// restored the moment listening ends, HOWEVER it ends. Every helper event
/// funnels through emit_dictation_event_value, so this is the single seam.
fn sync_capture_mic_duck(event_type: Option<&str>) {
    if let Some(ducked) = mic_duck_action_for_event(event_type) {
        crate::audio::capture::set_mic_ducked(ducked);
    }
}

/// The duck decision for a helper event: `Some(true)` starts a duck (listening
/// began), `Some(false)` lifts it (listening ended, however it ended),
/// `None` leaves the current state untouched. The un-duck set is deliberately
/// broad — every workflow-terminal, finalizing, or post-listening event lifts
/// the duck — because a leaked duck would silently swallow the rest of the
/// meeting's mic track, and re-lifting an already-lifted duck is a harmless
/// no-op.
/// Intermediate events like `audio_level` return `None` so the duck holds for
/// the whole listening window. System audio is never touched.
fn mic_duck_action_for_event(event_type: Option<&str>) -> Option<bool> {
    match event_type {
        Some("listening_started") => Some(true),
        Some(
            "recording_ready"
            | "recording_discarded"
            | "finalizing_transcript"
            | "final_transcript"
            | "paste_completed"
            | "error"
            | "helper_unavailable"
            | "shutdown_ack",
        ) => Some(false),
        _ => None,
    }
}

/// How a dictation event moves the update gate's record of takes in flight
/// (`crate::update_gate`). This is the gate's only notion of "a dictation
/// is running"; it does not reuse the menu-bar indicator.
#[derive(Debug, PartialEq, Eq)]
enum GateTakeTransition<'a> {
    Started(&'a str),
    Ended(&'a str),
    AllEnded,
}

fn gate_take_transition<'a>(
    event_type: Option<&str>,
    take_id: Option<&'a str>,
) -> Option<GateTakeTransition<'a>> {
    let take_id = take_id.unwrap_or(UNTAGGED_DICTATION_TAKE);
    match event_type {
        Some("helper_unavailable" | "shutdown_ack") => Some(GateTakeTransition::AllEnded),
        Some("recording_ready" | "listening_started") => Some(GateTakeTransition::Started(take_id)),
        // The real end of a take. final_transcript and paste_target are not:
        // the Windows helper still writes the clipboard, focuses and pastes
        // after them, and a composer delivery waits for the composer's answer
        // (paste_completed or a composer error).
        Some("recording_discarded" | "paste_completed" | "agent_session_prompt" | "error") => {
            Some(GateTakeTransition::Ended(take_id))
        }
        _ => None,
    }
}

/// Keeps the update gate in step with every dictation event, whichever emit
/// path it takes. A take the helper starts while an update installs is
/// discarded at once.
fn sync_update_gate_take(app: &AppHandle, event: &serde_json::Value) {
    forget_admission_for_terminal_event(app, event);
    let event_type = event.get("type").and_then(serde_json::Value::as_str);
    let gate = &crate::update_gate::UPDATE_GATE;
    match gate_take_transition(event_type, dictation_take_id_from_event(event)) {
        Some(GateTakeTransition::Started(take_id)) => {
            if gate.observe_dictation_start(take_id) == crate::update_gate::DictationStart::Stop {
                stop_take_during_update(app, take_id.to_string());
            }
        }
        Some(GateTakeTransition::Ended(take_id)) => gate.end_dictation_take(take_id),
        Some(GateTakeTransition::AllEnded) => gate.end_all_dictation_takes(),
        None => {}
    }
}

/// Applies a helper event's end of take to the update gate by the event's own
/// take id. Runs before any active-take or interface filter, which may drop
/// the event for the HUD and the take lifecycle but must never keep a finished
/// take holding the install.
fn end_gate_take_for_helper_event(
    gate: &crate::update_gate::UpdateGate,
    event: &serde_json::Value,
) {
    let event_type = event.get("type").and_then(serde_json::Value::as_str);
    match gate_take_transition(event_type, dictation_take_id_from_event(event)) {
        Some(GateTakeTransition::Ended(take_id)) => gate.end_dictation_take(take_id),
        Some(GateTakeTransition::AllEnded) => gate.end_all_dictation_takes(),
        Some(GateTakeTransition::Started(_)) | None => {}
    }
}

/// Discards a take the helper started while an update installs. Sent off the
/// event thread so it never waits on the helper lock from inside an event.
fn stop_take_during_update(app: &AppHandle, take_id: String) {
    let app = app.clone();
    std::thread::spawn(move || {
        tracing::warn!("discarding a dictation take that started during an update install");
        cancel_dictation_take(&app, Some(&take_id));
        if let Some(state) = app.try_state::<HelperState>() {
            let _ = send_helper_command(
                &state,
                serde_json::json!({ "type": "discard_recording", "takeId": take_id }),
            );
        }
    });
}

/// Lets a take start unless an update is installing; a refused start clears
/// the shortcut state it prepared.
fn admit_dictation_start(app: &AppHandle, take_id: Option<&str>) -> Result<(), AppError> {
    let state = app.try_state::<DictationTakeState>().ok_or_else(|| {
        reset_shortcut_activation(app);
        AppError::new(
            "dictation_helper_unavailable",
            "Dictation state is not initialized.",
        )
    })?;
    let settings = crate::providers::effective_current_settings();
    crate::providers::transcription::effective_transcription_backend(
        &settings,
        crate::providers::transcription::ready_directory(),
        crate::providers::local_transcription_supported(),
        || crate::providers::provider_api_key(crate::providers::BuiltinProvider::Groq),
    )
    .inspect_err(|_| reset_shortcut_activation(app))?;
    let id = take_id.unwrap_or(UNTAGGED_DICTATION_TAKE);
    crate::update_gate::UPDATE_GATE
        .begin_dictation_take(id)
        .inspect_err(|_| reset_shortcut_activation(app))?;
    state.remember_admitted_settings(id, settings);
    Ok(())
}

/// An admitted start never reached the helper, so no event will end it.
fn release_unsent_dictation_take(app: &AppHandle, take_id: Option<&str>) {
    if let Some(state) = app.try_state::<DictationTakeState>() {
        state.forget_admitted_settings(Some(take_id.unwrap_or(UNTAGGED_DICTATION_TAKE)));
    }
    let current = app
        .try_state::<DictationTakeState>()
        .and_then(|state| state.current_take_id());
    if let (Some(take_id), Some(activation)) = (take_id, app.try_state::<ShortcutActivationState>())
    {
        if let Ok(mut controller) = activation.controller.lock() {
            controller.finish_helper_take(take_id, current.as_deref());
        }
    }
    crate::update_gate::UPDATE_GATE.end_dictation_take(take_id.unwrap_or(UNTAGGED_DICTATION_TAKE));
}

fn forget_admission_for_terminal_event(app: &AppHandle, event: &serde_json::Value) {
    let Some(state) = app.try_state::<DictationTakeState>() else {
        return;
    };
    match gate_take_transition(
        event.get("type").and_then(serde_json::Value::as_str),
        dictation_take_id_from_event(event),
    ) {
        Some(GateTakeTransition::Ended(id)) => state.forget_admitted_settings(Some(id)),
        Some(GateTakeTransition::AllEnded) => state.forget_admitted_settings(None),
        _ => {}
    }
}

/// The gate's key for a take whose start or events carry no take id.
const UNTAGGED_DICTATION_TAKE: &str = "untagged";

/// Pushes the menu-bar dictation indicator to match the current take, but only
/// on an actual edge. `audio_level` fires continuously while listening and is in
/// the "active" set, so without the compare-and-swap every audio frame would
/// marshal a blocking tray update to the main thread (the same reason
/// `mic_duck_action_for_event` returns `None` for `audio_level`).
fn sync_dictation_indicator(app: &AppHandle, event_type: Option<&str>) {
    let Some(active) = indicator_action_for_event(event_type) else {
        return;
    };
    if DICTATION_ACTIVE.swap(active, Ordering::SeqCst) == active {
        return;
    }
    crate::menu_bar::set_dictation_active(app, active);
}

/// Maps a helper event to "a dictation take is in flight": `Some(true)` while a
/// take is running, `Some(false)` the moment it ends (however it ends),
/// `None` to leave the indicator untouched.
///
/// This MUST stay in step with `DICTATION_ACTIVE_EVENTS` /
/// `DICTATION_FINISHED_EVENTS` in `src/lib/dictation-events.ts` — the menu bar
/// (this fn) and the in-app state (that file) read the same event stream, so if
/// they classify differently the native indicator and the webview disagree.
/// Deliberately does NOT reuse `dictation_event_visibility`: that classifier
/// tracks HUD visibility and would miss the start (`recording_ready`), miss the
/// abort (`recording_discarded`), and leave the indicator on after
/// `final_transcript`.
fn indicator_action_for_event(event_type: Option<&str>) -> Option<bool> {
    match event_type {
        Some(
            "recording_ready"
            | "listening_started"
            | "audio_level"
            | "finalizing_transcript"
            | "paste_target",
        ) => Some(true),
        Some(
            "recording_discarded"
            | "final_transcript"
            | "paste_completed"
            | "agent_session_prompt"
            | "error"
            | "helper_unavailable"
            | "shutdown_ack",
        ) => Some(false),
        _ => None,
    }
}

fn append_dictation_event_log(
    app: &AppHandle,
    event_type: Option<&str>,
    event: &serde_json::Value,
) {
    let Some(event_type) = event_type else {
        return;
    };
    if event_type == "audio_level" {
        return;
    }
    let entry = dictation_event_log_entry(event_type, event);
    let Ok(directory) = crate::app_paths::app_data_dir(app) else {
        return;
    };
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join(DICTATION_EVENT_LOG))
    else {
        return;
    };
    let _ = writeln!(file, "{entry}");
}

fn dictation_event_log_entry(event_type: &str, event: &serde_json::Value) -> serde_json::Value {
    let payload = event.get("payload");
    let text_chars = payload
        .and_then(|payload| payload.get("text"))
        .and_then(serde_json::Value::as_str)
        .map(|text| text.chars().count());
    let prompt_chars = payload
        .and_then(|payload| payload.get("prompt"))
        .and_then(serde_json::Value::as_str)
        .map(|prompt| prompt.chars().count());
    serde_json::json!({
        "timestamp": Utc::now().to_rfc3339(),
        "type": event_type,
        "code": payload
            .and_then(|payload| payload.get("code"))
            .and_then(serde_json::Value::as_str),
        "underlyingCode": payload
            .and_then(|payload| payload.get("underlyingCode"))
            .and_then(serde_json::Value::as_str),
        "message": payload
            .and_then(|payload| payload.get("message"))
            .and_then(serde_json::Value::as_str),
        "underlyingMessage": payload
            .and_then(|payload| payload.get("underlyingMessage"))
            .and_then(serde_json::Value::as_str),
        "silent": payload
            .and_then(|payload| payload.get("silent"))
            .and_then(serde_json::Value::as_bool),
        "path": payload
            .and_then(|payload| payload.get("path"))
            .and_then(serde_json::Value::as_str),
        "observedAudioLevel": payload
            .and_then(|payload| payload.get("observedAudioLevel"))
            .cloned(),
        "speechConfidence": payload
            .and_then(|payload| payload.get("speechConfidence"))
            .cloned(),
        "speechAnalysisStatus": payload
            .and_then(|payload| payload.get("speechAnalysisStatus"))
            .cloned(),
        "pasteTarget": payload
            .and_then(|payload| payload.get("app"))
            .and_then(serde_json::Value::as_str),
        "inputSubmitted": payload
            .and_then(|payload| payload.get("inputSubmitted"))
            .and_then(serde_json::Value::as_bool),
        "deliveryConfirmed": payload
            .and_then(|payload| payload.get("deliveryConfirmed"))
            .and_then(serde_json::Value::as_bool),
        "eventsSubmitted": payload
            .and_then(|payload| payload.get("eventsSubmitted"))
            .and_then(serde_json::Value::as_u64),
        "provider": payload
            .and_then(|payload| payload.get("provider"))
            .and_then(serde_json::Value::as_str),
        "textChars": text_chars,
        "promptChars": prompt_chars,
        "kind": payload
            .and_then(|payload| payload.get("kind"))
            .and_then(serde_json::Value::as_str),
        "trigger": trigger_log_fields(payload),
    })
}

/// The Windows helper names the pinned window only by handle and process;
/// the dictation indicator needs `target` (app name and identity, for the
/// icon), as the Mac helper sends it. Fills it for the start and paste
/// events when `describe` can identify the app.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn complete_windows_target(
    event: &mut serde_json::Value,
    describe: impl Fn(i64, u32) -> Option<(String, String)>,
) {
    let event_type = event.get("type").and_then(serde_json::Value::as_str);
    // The stop pins the window the text is pasted into (it may not be the
    // one dictation started in), and the indicator pins what this names.
    if !matches!(
        event_type,
        Some("listening_started" | "finalizing_transcript" | "paste_target")
    ) {
        return;
    }
    let Some(payload) = event
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    if payload
        .get("target")
        .is_some_and(|target| !target.is_null())
    {
        return;
    }
    let hwnd = payload
        .get("targetWindowHandle")
        .and_then(serde_json::Value::as_i64);
    let pid = payload
        .get("targetProcessId")
        .and_then(serde_json::Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok());
    let (Some(hwnd), Some(pid)) = (hwnd, pid) else {
        return;
    };
    let Some((app, bundle_identifier)) = describe(hwnd, pid) else {
        return;
    };
    payload.insert(
        "target".to_string(),
        serde_json::json!({
            "app": app,
            "bundleIdentifier": bundle_identifier,
            "processIdentifier": pid,
        }),
    );
}

/// A log line for what the app did with a shortcut edge.
fn shortcut_edge_log_event(
    kind: DictationShortcutKind,
    edge: ShortcutKeyEdge,
    command: Option<DictationCommand>,
    held: Option<Duration>,
) -> serde_json::Value {
    let action = match command {
        None => "none",
        Some(DictationCommand::StartListening) => "start",
        Some(DictationCommand::StopAndPaste) => "stop_and_paste",
        Some(DictationCommand::DiscardListening) => "discard",
        Some(DictationCommand::ToggleListening) => "toggle",
    };
    let reason = match (edge, command) {
        (ShortcutKeyEdge::Up, Some(DictationCommand::DiscardListening)) => "short_hold",
        (ShortcutKeyEdge::Cancel, Some(DictationCommand::DiscardListening)) => "chord",
        (_, None) => "busy_or_not_held",
        _ => "shortcut",
    };
    serde_json::json!({
        "type": "shortcut_edge",
        "payload": {
            "kind": match kind {
                DictationShortcutKind::PushToTalk => "push_to_talk",
                DictationShortcutKind::Toggle => "toggle",
            },
            "trigger": {
                "edge": match edge {
                    ShortcutKeyEdge::Down => "down",
                    ShortcutKeyEdge::Up => "up",
                    ShortcutKeyEdge::Cancel => "cancel",
                },
                "heldMs": held.map(|held| held.as_millis() as u64),
                "action": action,
                "reason": reason,
            },
        },
    })
}

/// The trigger diagnostic fields worth keeping: the helper's own bounded
/// vocabulary (key codes of modifiers only, decisions, reasons), never more.
fn trigger_log_fields(payload: Option<&serde_json::Value>) -> serde_json::Value {
    let Some(trigger) = payload
        .and_then(|payload| payload.get("trigger"))
        .and_then(serde_json::Value::as_object)
    else {
        return serde_json::Value::Null;
    };
    let mut fields = serde_json::Map::new();
    for name in [
        "key", "edge", "injected", "alone", "heldMs", "decision", "action", "reason",
    ] {
        let Some(value) = trigger.get(name) else {
            continue;
        };
        let kept = match value {
            serde_json::Value::String(text) => {
                serde_json::Value::String(text.chars().take(40).collect())
            }
            serde_json::Value::Bool(_) | serde_json::Value::Number(_) => value.clone(),
            _ => continue,
        };
        fields.insert(name.to_string(), kept);
    }
    serde_json::Value::Object(fields)
}

/// Tag error events with `payload.silent: bool` so the HUD doesn't have to
/// re-derive the classification — Rust is the single source of truth.
fn annotate_silent_error(event: &mut serde_json::Value) {
    if event.get("type").and_then(serde_json::Value::as_str) != Some("error") {
        return;
    }
    let silent = is_silent_transcription_error(event);
    if let Some(payload) = event
        .get_mut("payload")
        .and_then(serde_json::Value::as_object_mut)
    {
        // An event that already decided (Whisper's visible no speech) keeps it.
        payload
            .entry("silent")
            .or_insert(serde_json::Value::Bool(silent));
    }
}

fn update_hud_window(app: &AppHandle, event_type: Option<&str>, event: Option<&serde_json::Value>) {
    match dictation_event_visibility(event_type) {
        // The webview owns showing for dictation events: hud.ts resizes the
        // window to the measured pill, then invokes dictation_hud_show.
        // Showing eagerly from here raced that resize — the window came up at
        // a stale frame as a gray bar, then a clipped pill, until the next
        // visible-state resize healed it.
        DictationEventVisibility::Show => {}
        DictationEventVisibility::Hide => {
            schedule_hud_hide(app, hud_hide_delay_for_event(event_type, event))
        }
        DictationEventVisibility::Ignore => {}
    }
}

fn update_latest_event(app: &AppHandle, event_type: Option<&str>, line: Option<String>) {
    if let Some(state) = app.try_state::<LastDictationEvent>() {
        if let Ok(mut event) = state.event.lock() {
            *event = latest_hud_snapshot(event.as_deref(), event_type, line.as_deref());
        }
    }
}

/// Retain a replayable lifecycle snapshot, not a lone meter/focus update.
/// This lets a late HUD subscriber start with the current take's destination.
fn latest_hud_snapshot(
    previous: Option<&str>,
    event_type: Option<&str>,
    line: Option<&str>,
) -> Option<String> {
    if dictation_event_visibility(event_type) == DictationEventVisibility::Hide {
        return None;
    }
    let prior = previous.and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok());
    let incoming = line.and_then(|line| serde_json::from_str::<serde_json::Value>(line).ok());
    if matches!(event_type, Some("audio_level" | "focus_target")) {
        let (Some(mut prior), Some(incoming)) = (prior, incoming) else {
            return previous.map(str::to_owned);
        };
        if prior["type"] == "listening_started"
            && dictation_take_id_from_event(&prior) == dictation_take_id_from_event(&incoming)
        {
            if let Some(target) = incoming["payload"].get("target") {
                if let Some(payload) = prior
                    .get_mut("payload")
                    .and_then(serde_json::Value::as_object_mut)
                {
                    payload.insert("target".to_string(), target.clone());
                }
            }
        }
        return Some(prior.to_string());
    }
    if dictation_event_visibility(event_type) != DictationEventVisibility::Show {
        return previous.map(str::to_owned);
    }
    let Some(mut incoming) = incoming else {
        return line.map(str::to_owned);
    };
    if event_type != Some("listening_started") {
        if let Some(prior) = prior {
            if dictation_take_id_from_event(&prior) == dictation_take_id_from_event(&incoming)
                && incoming["payload"].get("target").is_none()
            {
                if let Some(target) = prior["payload"].get("target") {
                    if incoming.get("payload").is_none() {
                        incoming["payload"] = serde_json::json!({});
                    }
                    if let Some(payload) = incoming
                        .get_mut("payload")
                        .and_then(serde_json::Value::as_object_mut)
                    {
                        payload.insert("target".to_string(), target.clone());
                    }
                }
            }
        }
    }
    Some(incoming.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DictationEventVisibility {
    Show,
    Hide,
    Ignore,
}

fn dictation_event_visibility(event_type: Option<&str>) -> DictationEventVisibility {
    match event_type {
        Some(
            "listening_started"
            | "audio_level"
            | "finalizing_transcript"
            | "final_transcript"
            | "paste_target",
        ) => DictationEventVisibility::Show,
        Some(
            "paste_completed"
            | "recording_discarded"
            | "agent_session_prompt"
            | "error"
            | "shutdown_ack"
            | "helper_unavailable",
        ) => DictationEventVisibility::Hide,
        _ => DictationEventVisibility::Ignore,
    }
}

fn hud_hide_delay_for_event(
    event_type: Option<&str>,
    event: Option<&serde_json::Value>,
) -> Duration {
    match event_type {
        Some("shutdown_ack") => Duration::ZERO,
        // The webview normally hides immediately after paste, but keeps a
        // missing-cleanup-key notice for 3 s before its exit animation.
        // This native fallback must not cut that notice short.
        Some("paste_completed") => Duration::from_millis(3500),
        Some("error") if event.is_some_and(is_silent_transcription_error) => {
            Duration::from_millis(900)
        }
        Some("error") => Duration::from_millis(2200),
        _ => Duration::from_millis(900),
    }
}

fn is_silent_transcription_error(event: &serde_json::Value) -> bool {
    let payload = event.get("payload");
    let code = payload
        .and_then(|payload| payload.get("code"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let message = payload
        .and_then(|payload| payload.get("message"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let normalized_message = message.to_ascii_lowercase();

    matches!(
        code,
        "missing_recording"
            | "no_speech"
            | "no_transcription"
            | "empty_transcript"
            | "transcription_empty"
    ) || normalized_message.contains("empty transcript")
        || normalized_message.contains("no transcript")
        || normalized_message.contains("no speech")
        || normalized_message.contains("no recorded audio")
        || normalized_message.contains("audio file is too short")
        || normalized_message.contains("did not return any transcript")
        // Historical no-speech results mean nothing was captured.
        || normalized_message.contains("no_speech")
        || normalized_message.contains("dictation_text_empty")
}

/// True while the window has been shown fully transparent by
/// wake_hud_window and the webview hasn't revealed it properly yet —
/// is_visible() can't tell a transparent window from a shown one.
static HUD_WOKEN_FADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Wake the HUD window for the meeting prompt without revealing it. The
/// webview of a long-hidden window may be suspended and slow to process the
/// detection event, but showing it eagerly at full alpha flashes a stale
/// frame — bare frost, then a clipped pill — until the webview's resize
/// lands. Show it fully transparent instead; the webview sizes the window
/// and fades it in via dictation_hud_show.
pub(crate) fn wake_hud_window(app: &AppHandle) {
    if let Some(hud) = app.get_webview_window("hud") {
        if hud.is_visible().unwrap_or(false) {
            // Already up — either with real content (leave it alone) or
            // from an earlier wake (already transparent).
            return;
        }
        // Meeting prompts always come up top-center; dictation_hud_show
        // re-centers precisely once the webview has sized the window.
        position_hud_window_top_center(&hud);
        HUD_WOKEN_FADED.store(true, std::sync::atomic::Ordering::Relaxed);
        let alpha_hud = hud.clone();
        let _ = app.run_on_main_thread(move || set_window_alpha(&alpha_hud, 0.0));
        let _ = hud.show();
    }
}

fn schedule_hud_hide(app: &AppHandle, delay: Duration) {
    let app = app.clone();
    thread::spawn(move || {
        if !delay.is_zero() {
            thread::sleep(delay);
        }

        if let Some(state) = app.try_state::<LastDictationEvent>() {
            if state
                .event
                .lock()
                .ok()
                .and_then(|event| event.clone())
                .is_some()
            {
                return;
            }
        }

        if let Some(hud) = app.get_webview_window("hud") {
            let _ = hud.hide();
        }
        hide_dictation_tooltip(&app);
    });
}

fn position_hud_window(app: &AppHandle, hud: &WebviewWindow) {
    cancel_hud_drag(hud);
    let Ok(window_size) = hud.outer_size() else {
        return;
    };

    // Restore the in-memory drag position if it's still on-screen. This
    // doesn't persist across app restarts — that's deliberate; quitting
    // Clovy resets the HUD to the configured edge so it can't end up lost on a
    // monitor that's no longer connected.
    if let Some(state) = app.try_state::<HudPosition>() {
        let saved = state.inner.lock().ok().and_then(|guard| guard.saved);
        if let Some((x, y)) = saved {
            if let Some((x, y)) = restored_hud_position(hud, x, y, window_size) {
                let _ = hud.set_position(PhysicalPosition::new(x, y));
                return;
            }
        }
    }

    if let Some((x, y)) = default_hud_position(hud, window_size) {
        let _ = hud.set_position(PhysicalPosition::new(x, y));
    }
}

fn current_or_next_hud_position(
    app: &AppHandle,
    hud: &WebviewWindow,
    window_size: PhysicalSize<u32>,
) -> Option<PhysicalPosition<i32>> {
    if hud.is_visible().unwrap_or(false)
        && !HUD_WOKEN_FADED.load(std::sync::atomic::Ordering::Relaxed)
    {
        if let Ok(position) = hud.outer_position() {
            return Some(position);
        }
    }

    if let Some(state) = app.try_state::<HudPosition>() {
        let saved = state.inner.lock().ok().and_then(|guard| guard.saved);
        if let Some((x, y)) = saved {
            if let Some((x, y)) = restored_hud_position(hud, x, y, window_size) {
                return Some(PhysicalPosition::new(x, y));
            }
        }
    }

    default_hud_position(hud, window_size)
        .map(|(x, y)| PhysicalPosition::new(x, y))
        .or_else(|| hud.outer_position().ok())
}

/// Meeting prompts always enter at the default top-center spot. They're
/// transient notifications, not the user's parked dictation pill, so the
/// saved drag position doesn't apply to them.
fn position_hud_window_top_center(hud: &WebviewWindow) {
    cancel_hud_drag(hud);
    let Ok(window_size) = hud.outer_size() else {
        return;
    };
    if let Some((x, y)) = hud_position_for_placement(hud, window_size, DictationHudPlacement::Top) {
        let _ = hud.set_position(PhysicalPosition::new(x, y));
    }
}

fn default_hud_position(hud: &WebviewWindow, window_size: PhysicalSize<u32>) -> Option<(i32, i32)> {
    let placement = settings_store().lock().ok()?.hud_placement;
    hud_position_for_placement(hud, window_size, placement)
}

fn hud_position_for_placement(
    hud: &WebviewWindow,
    window_size: PhysicalSize<u32>,
    placement: DictationHudPlacement,
) -> Option<(i32, i32)> {
    let monitor = hud
        .cursor_position()
        .ok()
        .and_then(|cursor| hud.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| hud.current_monitor().ok().flatten())
        .or_else(|| hud.primary_monitor().ok().flatten())?;

    Some(dictation_hud_origin(
        *monitor.work_area(),
        window_size,
        monitor.scale_factor(),
        placement,
    ))
}

fn dictation_hud_origin(
    work_area: tauri::PhysicalRect<i32, u32>,
    window_size: PhysicalSize<u32>,
    scale: f64,
    placement: DictationHudPlacement,
) -> (i32, i32) {
    let margin = (12.0 * scale).round() as i32;
    let x = work_area.position.x + (work_area.size.width as i32 - window_size.width as i32) / 2;
    let top = work_area.position.y + margin;
    let bottom =
        work_area.position.y + work_area.size.height as i32 - window_size.height as i32 - margin;
    let y = match placement {
        DictationHudPlacement::Bottom => bottom.max(top),
        DictationHudPlacement::Top => top,
    };
    (x, y)
}

/// Restore only onto a connected monitor, clamping the current window size
/// without rewriting the manual anchor. A pill can grow after the drag.
fn restored_hud_position(
    hud: &WebviewWindow,
    x: i32,
    y: i32,
    window_size: PhysicalSize<u32>,
) -> Option<(i32, i32)> {
    const MIN_OVERLAP_PX: i32 = 24;
    let monitors = hud.available_monitors().ok()?;
    let pill_w = window_size.width as i32;
    let pill_h = window_size.height as i32;
    monitors.iter().find_map(|monitor| {
        let work = monitor.work_area();
        let mx = work.position.x;
        let my = work.position.y;
        let mw = work.size.width as i32;
        let mh = work.size.height as i32;
        let overlap_x = (x + pill_w).min(mx + mw) - x.max(mx);
        let overlap_y = (y + pill_h).min(my + mh) - y.max(my);
        if overlap_x < MIN_OVERLAP_PX || overlap_y < MIN_OVERLAP_PX {
            return None;
        }
        Some(dictation_hud_resized_origin(
            *work,
            PhysicalPosition::new(x, y),
            window_size,
            window_size,
        ))
    })
}

fn configure_hud_window(app: &AppHandle) -> Result<(), String> {
    if let Some(hud) = app.get_webview_window("hud") {
        hud.set_always_on_top(true)
            .map_err(|error| error.to_string())?;
        hud.set_visible_on_all_workspaces(true)
            .map_err(|error| error.to_string())?;
        hud.set_focusable(false)
            .map_err(|error| error.to_string())?;
        hud.set_skip_taskbar(true)
            .map_err(|error| error.to_string())?;
        // The webview paints the HUD surface and shadow into a transparent
        // gutter, matching the agent HUD. Keep native shadow/vibrancy off so
        // no frosted rim or large repaint can appear during transitions.
        hud.set_shadow(false).map_err(|error| error.to_string())?;

        #[cfg(target_os = "macos")]
        {
            make_hud_nonactivating(&hud);
            use window_vibrancy::clear_vibrancy;
            if let Err(error) = clear_vibrancy(&hud) {
                tracing::warn!(%error, "failed to clear dictation HUD vibrancy");
            }
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn show_dictation_tooltip(
    hud_pointer: usize,
    cancel_rect: &HudClientRect,
    placement: HudTooltipPlacement,
) {
    use objc2::{msg_send, runtime::AnyObject, MainThreadMarker};
    use objc2_foundation::{NSPoint, NSRect};

    let Some(_mtm) = MainThreadMarker::new() else {
        return;
    };
    let hud = hud_pointer as *mut AnyObject;
    if hud.is_null() {
        return;
    }
    let Some(panel) = dictation_tooltip_panel() else {
        return;
    };

    unsafe {
        let frame: NSRect = msg_send![hud, frame];
        let content: *mut AnyObject = msg_send![hud, contentView];
        if content.is_null() {
            return;
        }
        let bounds: NSRect = msg_send![content, bounds];
        let (x, y) = hud_tooltip_origin(
            frame.origin.x,
            frame.origin.y,
            bounds.size.height,
            cancel_rect,
            placement,
        );
        let _: () = msg_send![panel, setFrameOrigin: NSPoint::new(x, y)];
        let _: () = msg_send![panel, orderFrontRegardless];
    }
}

#[cfg(target_os = "macos")]
fn dictation_tooltip_panel() -> Option<*mut objc2::runtime::AnyObject> {
    use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let pointer = *DICTATION_TOOLTIP_PANEL.get_or_init(|| unsafe {
        let Some(panel_class) = AnyClass::get(c"NSPanel") else {
            return 0;
        };
        let allocated: *mut AnyObject = msg_send![panel_class, alloc];
        let panel: *mut AnyObject = msg_send![
            allocated,
            initWithContentRect: NSRect::new(
                NSPoint::new(-200.0, -200.0),
                NSSize::new(
                    DICTATION_TOOLTIP_WIDTH_LOGICAL,
                    DICTATION_TOOLTIP_HEIGHT_LOGICAL,
                ),
            ),
            styleMask: (1u64 << 7),
            backing: 2u64,
            defer: false
        ];
        if panel.is_null() {
            return 0;
        }
        let _: () = msg_send![panel, setOpaque: false];
        let _: () = msg_send![panel, setHasShadow: true];
        let _: () = msg_send![panel, setIgnoresMouseEvents: true];
        let _: () = msg_send![panel, setAcceptsMouseMovedEvents: false];
        let _: () = msg_send![panel, setBecomesKeyOnlyIfNeeded: true];
        let _: () = msg_send![panel, setLevel: 25i64];
        let _: () = msg_send![panel, setCollectionBehavior: (1u64 | (1 << 4) | (1 << 8))];
        let _: () = msg_send![panel, setReleasedWhenClosed: false];
        let _: () = msg_send![panel, setHidesOnDeactivate: false];

        if let Some(color_class) = AnyClass::get(c"NSColor") {
            let clear: *mut AnyObject = msg_send![color_class, clearColor];
            let _: () = msg_send![panel, setBackgroundColor: clear];
        }

        let content: *mut AnyObject = msg_send![panel, contentView];
        if content.is_null() {
            return 0;
        }
        let _: () = msg_send![content, setWantsLayer: true];
        let layer: *mut AnyObject = msg_send![content, layer];
        if !layer.is_null() {
            if let Some(color_class) = AnyClass::get(c"NSColor") {
                let background: *mut AnyObject = msg_send![
                    color_class,
                    colorWithSRGBRed: DICTATION_TOOLTIP_BACKGROUND_RED,
                    green: DICTATION_TOOLTIP_BACKGROUND_GREEN,
                    blue: DICTATION_TOOLTIP_BACKGROUND_BLUE,
                    alpha: DICTATION_TOOLTIP_BACKGROUND_ALPHA
                ];
                let border: *mut AnyObject = msg_send![
                    color_class,
                    colorWithSRGBRed: 1.0_f64,
                    green: 1.0_f64,
                    blue: 1.0_f64,
                    alpha: DICTATION_TOOLTIP_BORDER_ALPHA
                ];
                let background_cg: *mut AnyObject = msg_send![background, CGColor];
                let border_cg: *mut AnyObject = msg_send![border, CGColor];
                let _: () = msg_send![layer, setBackgroundColor: background_cg];
                let _: () = msg_send![layer, setBorderColor: border_cg];
            }
            let _: () = msg_send![layer, setBorderWidth: DICTATION_TOOLTIP_BORDER_WIDTH_LOGICAL];
            let _: () = msg_send![layer, setCornerRadius: DICTATION_TOOLTIP_CORNER_RADIUS_LOGICAL];
            let _: () = msg_send![layer, setMasksToBounds: true];
        }

        let Some(text_field_class) = AnyClass::get(c"NSTextField") else {
            return 0;
        };
        let field: *mut AnyObject = msg_send![text_field_class, alloc];
        let field: *mut AnyObject = msg_send![
            field,
            initWithFrame: NSRect::new(
                NSPoint::new(0.0, DICTATION_TOOLTIP_TEXT_Y_LOGICAL),
                NSSize::new(
                    DICTATION_TOOLTIP_WIDTH_LOGICAL,
                    DICTATION_TOOLTIP_TEXT_HEIGHT_LOGICAL,
                ),
            )
        ];
        if field.is_null() {
            return 0;
        }
        if let Some(string_class) = AnyClass::get(c"NSString") {
            let text: *mut AnyObject =
                msg_send![string_class, stringWithUTF8String: c"Esc".as_ptr()];
            let _: () = msg_send![field, setStringValue: text];
        }
        let _: () = msg_send![field, setEditable: false];
        let _: () = msg_send![field, setSelectable: false];
        let _: () = msg_send![field, setBezeled: false];
        let _: () = msg_send![field, setDrawsBackground: false];
        let _: () = msg_send![field, setAlignment: 1isize];
        if let Some(font_class) = AnyClass::get(c"NSFont") {
            // `--font-sans` falls back through system-ui/-apple-system. ABC
            // Diatype is a WKWebView webfont rather than an installed AppKit
            // face, so NSFont's system sans is the first native member of the
            // same token stack.
            let font: *mut AnyObject = msg_send![
                font_class,
                systemFontOfSize: DICTATION_TOOLTIP_FONT_SIZE_LOGICAL,
                weight: DICTATION_TOOLTIP_FONT_WEIGHT_MEDIUM
            ];
            let _: () = msg_send![field, setFont: font];
        }
        if let Some(color_class) = AnyClass::get(c"NSColor") {
            let text_color: *mut AnyObject = msg_send![
                color_class,
                colorWithSRGBRed: DICTATION_TOOLTIP_TEXT_RED,
                green: DICTATION_TOOLTIP_TEXT_GREEN,
                blue: DICTATION_TOOLTIP_TEXT_BLUE,
                alpha: 1.0_f64
            ];
            let _: () = msg_send![field, setTextColor: text_color];
        }
        let _: () = msg_send![content, addSubview: field];
        panel as usize
    });
    (pointer != 0).then_some(pointer as *mut AnyObject)
}

fn hide_dictation_tooltip(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let _ = app.run_on_main_thread(|| {
            use objc2::{msg_send, runtime::AnyObject, MainThreadMarker};

            let Some(_mtm) = MainThreadMarker::new() else {
                return;
            };
            let Some(pointer) = DICTATION_TOOLTIP_PANEL
                .get()
                .copied()
                .filter(|pointer| *pointer != 0)
            else {
                return;
            };
            let panel = pointer as *mut AnyObject;
            unsafe {
                let _: () = msg_send![panel, orderOut: std::ptr::null_mut::<AnyObject>()];
            }
        });
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// macOS: reclass the HUD's NSWindow to NSPanel and set the
/// `NSWindowStyleMaskNonactivatingPanel` style bit, so clicking the drag
/// handle or stop button doesn't steal focus from whichever app the user is
/// dictating into. Without this, every interaction with the HUD activates
/// Clovy and yanks the text cursor out of the user's document.
#[cfg(target_os = "macos")]
fn make_hud_nonactivating(hud: &WebviewWindow) {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    let Ok(handle) = hud.ns_window() else {
        return;
    };
    if handle.is_null() {
        return;
    }
    let Some(panel_class) = AnyClass::get(c"NSPanel") else {
        return;
    };

    unsafe {
        let window = handle as *mut AnyObject;
        // NSWindow → NSPanel; required because the non-activating style mask
        // is only honored on NSPanel instances.
        objc2::ffi::object_setClass(window, panel_class as *const _ as *const _);

        // NSWindowStyleMaskNonactivatingPanel = 1 << 7.
        const NON_ACTIVATING: usize = 1 << 7;
        let current_mask: usize = msg_send![window, styleMask];
        let _: () = msg_send![window, setStyleMask: current_mask | NON_ACTIVATING];

        // Allow mouseMoved + brief key acquisition without activating the
        // app, so native drag and any AppKit-routed mouse events still work.
        let _: () = msg_send![window, setAcceptsMouseMovedEvents: true];
        let _: () = msg_send![window, setBecomesKeyOnlyIfNeeded: true];
    }
}

fn hotkey_ready_event(settings: &DictationSettings) -> serde_json::Value {
    let shortcuts = format!(
        "Push to talk: {}; Toggle: {}",
        settings.push_to_talk_shortcut.label, settings.toggle_shortcut.label
    );
    serde_json::json!({
        "type": "hotkey_trigger_ready",
        "payload": {
            "shortcut": settings.push_to_talk_shortcut.label,
            "pushToTalkShortcut": settings.push_to_talk_shortcut.label,
            "toggleShortcut": settings.toggle_shortcut.label,
            "shortcuts": shortcuts,
        },
    })
}

#[cfg(target_os = "macos")]
fn initial_hotkey_event(settings: &DictationSettings) -> serde_json::Value {
    hotkey_ready_event(settings)
}

#[cfg(target_os = "windows")]
fn initial_hotkey_event(_settings: &DictationSettings) -> serde_json::Value {
    serde_json::json!({
        "type": "hotkey_trigger_unavailable",
        "payload": {
            "reason": "registering",
            "message": "Windows is registering dictation shortcuts.",
        },
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn initial_hotkey_event(_settings: &DictationSettings) -> serde_json::Value {
    serde_json::json!({
        "type": "hotkey_trigger_unavailable",
        "payload": {
            "reason": "unsupported",
            "message": "Dictation shortcuts are not supported on this platform.",
        },
    })
}

fn hotkey_error_event(error: String) -> serde_json::Value {
    serde_json::json!({
        "type": "error",
        "payload": {
            "code": "hotkey_registration_failed",
            "message": error,
        },
    })
}

fn set_hotkey_status(state: &HotkeyStatus, event: serde_json::Value) {
    if let Ok(mut status) = state.event.lock() {
        *status = event;
    }
}

pub fn key_code_for_code(code: &str) -> Option<u32> {
    Some(match code {
        "KeyA" => 0x00,
        "KeyS" => 0x01,
        "KeyD" => 0x02,
        "KeyF" => 0x03,
        "KeyH" => 0x04,
        "KeyG" => 0x05,
        "KeyZ" => 0x06,
        "KeyX" => 0x07,
        "KeyC" => 0x08,
        "KeyV" => 0x09,
        "KeyB" => 0x0b,
        "KeyQ" => 0x0c,
        "KeyW" => 0x0d,
        "KeyE" => 0x0e,
        "KeyR" => 0x0f,
        "KeyY" => 0x10,
        "KeyT" => 0x11,
        "Digit1" => 0x12,
        "Digit2" => 0x13,
        "Digit3" => 0x14,
        "Digit4" => 0x15,
        "Digit6" => 0x16,
        "Digit5" => 0x17,
        "Equal" => 0x18,
        "Digit9" => 0x19,
        "Digit7" => 0x1a,
        "Minus" => 0x1b,
        "Digit8" => 0x1c,
        "Digit0" => 0x1d,
        "BracketRight" => 0x1e,
        "KeyO" => 0x1f,
        "KeyU" => 0x20,
        "BracketLeft" => 0x21,
        "KeyI" => 0x22,
        "KeyP" => 0x23,
        "Enter" => 0x24,
        "KeyL" => 0x25,
        "KeyJ" => 0x26,
        "Quote" => 0x27,
        "KeyK" => 0x28,
        "Semicolon" => 0x29,
        "Backslash" => 0x2a,
        "Comma" => 0x2b,
        "Slash" => 0x2c,
        "KeyN" => 0x2d,
        "KeyM" => 0x2e,
        "Period" => 0x2f,
        "Tab" => 0x30,
        "Space" => 0x31,
        "Backquote" => 0x32,
        "Backspace" => 0x33,
        "Escape" => 0x35,
        "F1" => 0x7a,
        "F2" => 0x78,
        "F3" => 0x63,
        "F4" => 0x76,
        "F5" => 0x60,
        "F6" => 0x61,
        "F7" => 0x62,
        "F8" => 0x64,
        "F9" => 0x65,
        "F10" => 0x6d,
        "F11" => 0x67,
        "F12" => 0x6f,
        "ArrowLeft" => 0x7b,
        "ArrowRight" => 0x7c,
        "ArrowDown" => 0x7d,
        "ArrowUp" => 0x7e,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_dictation_outcome(text: &str) -> DictationTranscriptionOutcome {
        DictationTranscriptionOutcome {
            helper_command: serde_json::json!({"type":"paste_text", "text":text}),
            event: None,
            transcript: Some(TranscriptionProviderResult {
                text: text.into(),
                language: None,
                provider: "test".into(),
            }),
            quarantine_transcript: None,
        }
    }

    fn last_dictation_read(state: &DictationTakeState, profile: &str) -> serde_json::Value {
        state
            .last_dictation_response(
                profile,
                &serde_json::json!({"payload":{"requestId":"request-1"}}),
            )
            .unwrap()["text"]
            .clone()
    }

    fn last_dictation_claim(
        state: &DictationTakeState,
        take: &DictationTakeToken,
        delivery_claim: Option<DictationDeliveryClaim>,
        profile: &str,
        outcome: &DictationTranscriptionOutcome,
    ) -> Option<DictationDeliveryClaim> {
        let claim = delivery_claim.or_else(|| take.try_begin_delivery())?;
        state.remember_last_dictation(&claim, profile, outcome);
        Some(claim)
    }

    #[test]
    fn last_dictation_renderer_cannot_supply_repeat_text() {
        assert!(validate_renderer_dictation_command(&serde_json::json!({"type":"repeat_last_dictation","requestId":"guessed","text":"untrusted"})).is_err());
        assert!(validate_renderer_dictation_command(
            &serde_json::json!({"type":"start_listening"})
        )
        .is_ok());
    }

    #[test]
    fn last_dictation_preclaimed_stale_result_cannot_overwrite_new_take() {
        let state = DictationTakeState::default();
        let old = state.begin("old".into());
        assert!(old.try_begin_processing());
        let old_claim = old.try_begin_delivery().unwrap();
        last_dictation_deliver(&state, "new", "a", &last_dictation_outcome("new"));
        state.remember_last_dictation(&old_claim, "a", &last_dictation_outcome("old"));
        assert_eq!(last_dictation_read(&state, "a"), "new");
    }

    fn last_dictation_deliver(
        state: &DictationTakeState,
        id: &str,
        profile: &str,
        outcome: &DictationTranscriptionOutcome,
    ) {
        let take = state.begin(id.into());
        assert!(take.try_begin_processing());
        assert!(last_dictation_claim(state, &take, None, profile, outcome).is_some());
    }

    #[test]
    fn last_dictation_initial_and_restart_are_empty() {
        assert!(last_dictation_read(&DictationTakeState::default(), "a").is_null());
    }

    #[test]
    fn last_dictation_valid_result_replaces_and_paste_failure_retains() {
        let state = DictationTakeState::default();
        last_dictation_deliver(&state, "one", "a", &last_dictation_outcome("primeiro"));
        assert_eq!(last_dictation_read(&state, "a"), "primeiro");
        // Cache admission precedes helper I/O: no successful paste acknowledgement is needed.
        last_dictation_deliver(&state, "two", "a", &last_dictation_outcome("segundo"));
        assert_eq!(last_dictation_read(&state, "a"), "segundo");
        assert!(last_dictation_read(&DictationTakeState::default(), "a").is_null());
    }

    #[test]
    fn last_dictation_cancelled_and_late_results_do_not_replace() {
        let state = DictationTakeState::default();
        last_dictation_deliver(&state, "one", "a", &last_dictation_outcome("keep"));
        let cancelled = state.begin("cancelled".into());
        assert!(cancelled.try_begin_processing());
        assert!(cancelled.cancel());
        assert!(last_dictation_claim(
            &state,
            &cancelled,
            None,
            "a",
            &last_dictation_outcome("lost")
        )
        .is_none());
        let late = state.begin("late".into());
        assert!(late.try_begin_processing());
        state.begin("replacement".into());
        assert!(
            last_dictation_claim(&state, &late, None, "a", &last_dictation_outcome("late"))
                .is_none()
        );
        assert_eq!(last_dictation_read(&state, "a"), "keep");
    }

    #[test]
    fn last_dictation_empty_rejected_and_quarantined_do_not_replace() {
        let state = DictationTakeState::default();
        last_dictation_deliver(&state, "one", "a", &last_dictation_outcome("keep"));
        let mut empty = last_dictation_outcome("  ");
        let mut rejected = last_dictation_outcome("rejected");
        rejected.helper_command = serde_json::json!({"type":"discard_recording"});
        let mut quarantined = last_dictation_outcome("quarantined");
        quarantined.quarantine_transcript = quarantined.transcript.take();
        for (id, outcome) in [
            ("empty", &mut empty),
            ("rejected", &mut rejected),
            ("quarantined", &mut quarantined),
        ] {
            last_dictation_deliver(&state, id, "a", outcome);
            assert_eq!(last_dictation_read(&state, "a"), "keep");
        }
    }

    #[test]
    fn last_dictation_profile_isolation_and_single_slot() {
        let state = DictationTakeState::default();
        last_dictation_deliver(&state, "one", "a", &last_dictation_outcome("private a"));
        assert!(last_dictation_read(&state, "b").is_null());
        last_dictation_deliver(&state, "two", "b", &last_dictation_outcome("private b"));
        assert!(last_dictation_read(&state, "a").is_null());
        assert_eq!(last_dictation_read(&state, "b"), "private b");
    }

    #[test]
    fn last_dictation_response_is_correlated_and_does_not_start_take() {
        let state = DictationTakeState::default();
        last_dictation_deliver(
            &state,
            "one",
            "a",
            &last_dictation_outcome("ação 🧑🏽‍💻\n".repeat(5000).as_str()),
        );
        let request = serde_json::json!({"payload":{"requestId":"fresh"}});
        let response = state.last_dictation_response("a", &request).unwrap();
        assert_eq!(response["requestId"], "fresh");
        assert_eq!(response["text"], "ação 🧑🏽‍💻\n".repeat(5000));
        assert_eq!(
            state.last_dictation_response("a", &request).unwrap(),
            response
        );
        assert_eq!(state.current_take_id().as_deref(), Some("one"));
        assert_eq!(
            state.current.lock().unwrap().as_ref().unwrap().state(),
            DICTATION_TAKE_DELIVERING
        );
        assert!(state
            .last_dictation_response("a", &serde_json::json!({"payload":{"requestId":""}}))
            .is_none());
        assert!(state
            .last_dictation_response(
                "a",
                &serde_json::json!({"payload":{"requestId":"x".repeat(129)}})
            )
            .is_none());
    }

    #[test]
    fn last_dictation_voice_command_repeats_as_literal_text() {
        let state = DictationTakeState::default();
        let mut outcome = last_dictation_outcome("Hey Clovy faça algo");
        outcome.helper_command = serde_json::json!({"type":"discard_recording"});
        outcome.event = Some(
            serde_json::json!({"type":"agent_session_prompt", "payload":{"prompt":"faça algo"}}),
        );
        last_dictation_deliver(&state, "one", "a", &outcome);
        let response = state
            .last_dictation_response("a", &serde_json::json!({"payload":{"requestId":"fresh"}}))
            .unwrap();
        assert_eq!(response["type"], "repeat_last_dictation");
        assert_eq!(response["text"], "Hey Clovy faça algo");
        assert!(response.get("event").is_none());
    }

    #[test]
    fn dictation_hud_drag_start_does_not_replace_the_last_completed_drag() {
        let mut state = HudPositionState {
            saved: Some((500, 900)),
            ..Default::default()
        };
        state.begin_drag((864, 1120));
        assert_eq!(state.saved, Some((500, 900)));
    }

    #[test]
    fn dictation_hud_prompt_resize_does_not_replace_the_saved_dictation_drag() {
        let mut state = HudPositionState {
            saved: Some((500, 900)),
            ..Default::default()
        };
        state.cancel_drag();
        state.observe_drag((749, 42), false);
        assert_eq!(state.saved, Some((500, 900)));
    }

    #[test]
    fn dictation_hud_drag_commits_the_release_position_not_the_start_or_last_held_sample() {
        let mut state = HudPositionState::default();
        state.begin_drag((864, 1120));
        state.observe_drag((700, 1000), true);
        assert_eq!(state.saved, None);
        state.observe_drag((620, 940), false);
        assert_eq!(state.saved, Some((620, 940)));
        state.observe_drag((749, 42), false);
        assert_eq!(state.saved, Some((620, 940)));
    }

    #[test]
    fn dictation_hud_drag_release_without_movement_keeps_the_previous_anchor() {
        let mut state = HudPositionState {
            saved: Some((620, 940)),
            ..Default::default()
        };
        state.begin_drag((864, 1120));
        state.observe_drag((864, 1120), false);
        assert_eq!(state.saved, Some((620, 940)));
    }

    #[test]
    fn dictation_hud_automatic_motion_cancels_an_inflight_drag_without_saving_the_prompt() {
        let mut state = HudPositionState {
            saved: Some((620, 940)),
            ..Default::default()
        };
        state.begin_drag((864, 1120));
        state.observe_drag((700, 1000), true);
        state.cancel_drag();
        state.observe_drag((749, 42), false);
        assert_eq!(state.saved, Some((620, 940)));
    }

    #[test]
    fn dictation_hud_preference_change_discards_the_drag_so_next_dictation_uses_the_new_edge() {
        let mut state = HudPositionState {
            saved: Some((620, 940)),
            ..Default::default()
        };
        state.begin_drag((864, 1120));
        state.reset();
        state.observe_drag((700, 1000), false);
        assert_eq!(state.saved, None);
    }

    #[test]
    fn dictation_hud_origin_uses_the_selected_work_area_edge_at_each_scale() {
        // Physical work areas: menu excluded, with bottom/side/no Dock and a
        // secondary display to the left and above the primary one.
        let cases = [
            ((0, 30, 1920, 1170), (200, 68), 1.0, (860, 1120), (860, 42)),
            ((0, 30, 1920, 1090), (200, 68), 1.0, (860, 1040), (860, 42)),
            (
                (0, 60, 3840, 2340),
                (400, 136),
                2.0,
                (1720, 2240),
                (1720, 84),
            ),
            (
                (0, 60, 3840, 2180),
                (400, 136),
                2.0,
                (1720, 2080),
                (1720, 84),
            ),
            ((80, 30, 1840, 1170), (200, 68), 1.0, (900, 1120), (900, 42)),
            (
                (-2560, -1380, 2560, 1380),
                (400, 136),
                2.0,
                (-1480, -160),
                (-1480, -1356),
            ),
        ];
        for ((x, y, width, height), (w, h), scale, bottom, top) in cases {
            let work_area = tauri::PhysicalRect {
                position: PhysicalPosition::new(x, y),
                size: PhysicalSize::new(width, height),
            };
            for (placement, expected) in [("bottom", bottom), ("top", top)] {
                let settings: DictationSettings =
                    serde_json::from_value(serde_json::json!({"hudPlacement": placement})).unwrap();
                assert_eq!(
                    dictation_hud_origin(
                        work_area,
                        PhysicalSize::new(w, h),
                        scale,
                        settings.hud_placement
                    ),
                    expected,
                    "{placement} at {scale}x in {work_area:?}",
                );
            }
        }
    }

    #[test]
    fn dictation_hud_origin_keeps_growing_pills_above_the_dock() {
        let work_area = tauri::PhysicalRect {
            position: PhysicalPosition::new(0, 60),
            size: PhysicalSize::new(3840, 2180),
        };
        let settings: DictationSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(
            dictation_hud_origin(
                work_area,
                PhysicalSize::new(400, 136),
                2.0,
                settings.hud_placement
            ),
            (1720, 2080),
        );
        assert_eq!(
            dictation_hud_origin(
                work_area,
                PhysicalSize::new(800, 400),
                2.0,
                settings.hud_placement
            ),
            (1520, 1816),
        );
    }

    #[test]
    fn dictation_hud_settings_preserve_the_anchor_across_save_and_reload() {
        let legacy: DictationSettings = serde_json::from_str(r#"{"style":"formal"}"#).unwrap();
        let old_json = serde_json::to_value(&legacy).unwrap();
        assert_eq!(old_json["hudPlacement"], "bottom");
        for placement in ["top", "bottom"] {
            let settings: DictationSettings = serde_json::from_value(serde_json::json!({
                "style": "formal", "hudPlacement": placement
            }))
            .unwrap();
            let reloaded: DictationSettings =
                serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
            assert_eq!(
                serde_json::to_value(&reloaded).unwrap()["hudPlacement"],
                placement
            );
            assert_eq!(reloaded.style, DictationStyle::Formal);
        }
    }

    #[test]
    fn dictation_hud_resize_keeps_the_nearest_edge_and_clamps_dragged_pills() {
        let work = tauri::PhysicalRect {
            position: PhysicalPosition::new(-2560, -1380),
            size: PhysicalSize::new(2560, 1380),
        };
        let short = PhysicalSize::new(400, 136);
        let tall = PhysicalSize::new(800, 400);
        assert_eq!(
            dictation_hud_resized_origin(work, PhysicalPosition::new(-1480, -160), short, tall),
            (-1680, -424),
        );
        assert_eq!(
            dictation_hud_resized_origin(work, PhysicalPosition::new(-1480, -1356), short, tall),
            (-1680, -1356),
        );
        assert_eq!(
            dictation_hud_resized_origin(work, PhysicalPosition::new(-300, -100), short, tall),
            (-800, -400),
        );
    }

    #[test]
    fn cancel_tooltip_intent_has_independent_open_and_close_delays() {
        let mut intent = CancelTooltipIntent::default();

        for _ in 1..CANCEL_TOOLTIP_IN_POLLS {
            assert_eq!(intent.update(true), None);
        }
        assert_eq!(intent.update(true), Some(true));

        for _ in 1..CANCEL_TOOLTIP_OUT_POLLS {
            assert_eq!(intent.update(false), None);
        }
        assert_eq!(intent.update(false), Some(false));
        assert_eq!(intent.update(false), None);
    }

    #[test]
    fn cancel_tooltip_intent_reset_closes_only_an_open_tooltip() {
        let mut intent = CancelTooltipIntent::default();
        assert_eq!(intent.reset(), None);

        for _ in 0..CANCEL_TOOLTIP_IN_POLLS {
            intent.update(true);
        }
        assert_eq!(intent.reset(), Some(false));
        assert_eq!(intent.reset(), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_tooltip_tracks_the_cancel_control_above_and_below() {
        let cancel_rect = HudClientRect {
            left: 18.0,
            right: 42.0,
            top: 18.0,
            bottom: 50.0,
        };

        assert_eq!(
            hud_tooltip_origin(100.0, 200.0, 68.0, &cancel_rect, HudTooltipPlacement::Below,),
            (114.0, 188.0)
        );
        assert_eq!(
            hud_tooltip_origin(100.0, 200.0, 68.0, &cancel_rect, HudTooltipPlacement::Above,),
            (114.0, 256.0)
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn comm_line_identity_is_truncation_tolerant() {
        // Full path (comm= returned the executable path).
        assert!(comm_line_is_dictation_helper(
            "/Applications/Clovy Dictation Helper.app/Contents/MacOS/clovy-dictation-helper"
        ));
        // Bare full name.
        assert!(comm_line_is_dictation_helper("clovy-dictation-helper"));
        // Kernel p_comm truncated to 15 chars: the real orphan MUST still read
        // as the helper, or it is deleted-not-killed and JUN-338 reopens.
        assert!(comm_line_is_dictation_helper("clovy-dictation"));
        // Trailing whitespace/newline from ps is tolerated.
        assert!(comm_line_is_dictation_helper("clovy-dictation-helper\n"));
        // A shorter prefix (< 15) is ambiguous and must NOT match.
        assert!(!comm_line_is_dictation_helper("clovy-dictatio"));
        // Unrelated process and empty output never match.
        assert!(!comm_line_is_dictation_helper("/usr/bin/login"));
        assert!(!comm_line_is_dictation_helper("bash"));
        assert!(!comm_line_is_dictation_helper(""));
        assert!(!comm_line_is_dictation_helper("   "));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn owner_liveness_uses_start_time_to_defeat_pid_reuse() {
        // Same pid, same start time -> the owner is genuinely still alive.
        assert_eq!(
            owner_liveness(&StartTimeProbe::StartedAt("t0".into()), "t0"),
            OwnerLiveness::Alive
        );
        // Same pid, DIFFERENT start time -> the pid was recycled (e.g. a new
        // Clovy with a sequentially-reused pid); the original owner is dead, so
        // its record is NOT treated as the new instance's own. This is the
        // JUN-338 reopening the round-3 review caught.
        assert_eq!(
            owner_liveness(&StartTimeProbe::StartedAt("t1".into()), "t0"),
            OwnerLiveness::Dead
        );
        assert_eq!(
            owner_liveness(&StartTimeProbe::Gone, "t0"),
            OwnerLiveness::Dead
        );
        // Probe failure is never treated as "dead" -> fail closed.
        assert_eq!(
            owner_liveness(&StartTimeProbe::Unknown, "t0"),
            OwnerLiveness::Unknown
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn helper_match_skips_reused_pid_and_requires_identity() {
        // Exact recorded process (pid + start time) that is still a helper.
        assert_eq!(
            helper_match(
                &StartTimeProbe::StartedAt("t0".into()),
                "t0",
                &HelperIdentity::IsHelper
            ),
            HelperMatch::MatchesRecord
        );
        // Same pid, different start time -> reused by another process (possibly
        // another live instance's helper); never a kill target.
        assert_eq!(
            helper_match(
                &StartTimeProbe::StartedAt("t9".into()),
                "t0",
                &HelperIdentity::IsHelper
            ),
            HelperMatch::GoneOrReused
        );
        // Matching pid+start but the executable does not read as a helper. With
        // the truncation-tolerant identity check this is only a genuine
        // same-second pid reuse by a non-helper -> fail closed (abort), never
        // delete-and-race.
        assert_eq!(
            helper_match(
                &StartTimeProbe::StartedAt("t0".into()),
                "t0",
                &HelperIdentity::NotHelper
            ),
            HelperMatch::Unknown
        );
        // Gone/reused pid (start-time mismatch or dead) with a non-helper -> the
        // record names no live orphan, so it is safe to treat as stale.
        assert_eq!(
            helper_match(&StartTimeProbe::Gone, "t0", &HelperIdentity::NotHelper),
            HelperMatch::GoneOrReused
        );
        // Identity indeterminate -> fail closed.
        assert_eq!(
            helper_match(
                &StartTimeProbe::StartedAt("t0".into()),
                "t0",
                &HelperIdentity::Unknown
            ),
            HelperMatch::Unknown
        );
        assert_eq!(
            helper_match(&StartTimeProbe::Unknown, "t0", &HelperIdentity::IsHelper),
            HelperMatch::Unknown
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn decide_record_action_reaps_only_proven_orphans_and_fails_closed() {
        // Live owner -> leave it (covers "this is my own record" and "another
        // live instance's record").
        assert_eq!(
            decide_record_action(OwnerLiveness::Alive, HelperMatch::MatchesRecord),
            RecordAction::Skip
        );
        // Proven-dead owner + the recorded helper still alive -> reap.
        assert_eq!(
            decide_record_action(OwnerLiveness::Dead, HelperMatch::MatchesRecord),
            RecordAction::Reap
        );
        // Dead owner, helper already gone/reused -> just drop the stale record.
        assert_eq!(
            decide_record_action(OwnerLiveness::Dead, HelperMatch::GoneOrReused),
            RecordAction::DeleteStale
        );
        // Any indeterminate identity -> fail closed (abort the replacement spawn).
        assert_eq!(
            decide_record_action(OwnerLiveness::Unknown, HelperMatch::MatchesRecord),
            RecordAction::Abort
        );
        assert_eq!(
            decide_record_action(OwnerLiveness::Dead, HelperMatch::Unknown),
            RecordAction::Abort
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shutdown_fallback_reaps_only_the_current_instances_helper() {
        assert_eq!(
            decide_owned_shutdown_action(42, 42, OwnerLiveness::Alive, HelperMatch::MatchesRecord,),
            RecordAction::Reap
        );
        assert_eq!(
            decide_owned_shutdown_action(42, 42, OwnerLiveness::Alive, HelperMatch::GoneOrReused,),
            RecordAction::DeleteStale
        );
        // A sibling instance's ownership record is never a shutdown target.
        assert_eq!(
            decide_owned_shutdown_action(41, 42, OwnerLiveness::Alive, HelperMatch::MatchesRecord,),
            RecordAction::Abort
        );
        // Pid reuse, an already-dead owner, or any uncertain helper identity
        // fails closed instead of signalling an unverified process.
        assert_eq!(
            decide_owned_shutdown_action(42, 42, OwnerLiveness::Dead, HelperMatch::MatchesRecord,),
            RecordAction::Abort
        );
        assert_eq!(
            decide_owned_shutdown_action(42, 42, OwnerLiveness::Alive, HelperMatch::Unknown,),
            RecordAction::Abort
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn decide_unrecorded_action_reaps_only_unclaimed_confirmed_helpers() {
        // Enumerated helper with no matching record (claimed=false) -> reap.
        // This is the legacy/migration orphan from a pre-record build.
        assert_eq!(
            decide_unrecorded_action(false, &HelperIdentity::IsHelper),
            UnrecordedAction::Reap
        );
        // A dead-owner record does not add to the live-claimed set, so its
        // helper reads claimed=false here -> reap (same as no record).
        //
        // A live-owner record DOES claim its helper (claimed=true) -> skip, so a
        // running sibling instance's healthy helper is never killed.
        assert_eq!(
            decide_unrecorded_action(true, &HelperIdentity::IsHelper),
            UnrecordedAction::Skip
        );
        // pgrep matched the name but `ps` says it is not a helper -> do not kill.
        assert_eq!(
            decide_unrecorded_action(false, &HelperIdentity::NotHelper),
            UnrecordedAction::Skip
        );
        // Identity indeterminate on an unclaimed pid -> fail closed.
        assert_eq!(
            decide_unrecorded_action(false, &HelperIdentity::Unknown),
            UnrecordedAction::Abort
        );
        // Claimed by a live owner always wins, regardless of identity probe.
        assert_eq!(
            decide_unrecorded_action(true, &HelperIdentity::Unknown),
            UnrecordedAction::Skip
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn terminal_macos_process_states_are_not_treated_as_live() {
        // The field can contain multiple flags. The reported production case
        // was `UE`: uninterruptible + exiting, which still passed kill -0.
        for state in ["E", "UE", "Z", "Z+", "  UE\n"] {
            assert!(process_state_is_terminal(state), "state={state:?}");
        }
        for state in ["R", "R+", "S", "U", "  S+\n", ""] {
            assert!(!process_state_is_terminal(state), "state={state:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn wait_for_pid_exit_confirms_a_kill_and_bounds_a_survivor() {
        // A dead pid is observed as exited.
        let mut victim = std::process::Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn victim");
        let victim_pid = victim.id();
        kill_pid(victim_pid);
        let _ = victim.wait();
        assert!(wait_for_pid_exit(victim_pid, Duration::from_secs(2)));

        // ...and a live process is NOT declared exited, so the reap aborts the
        // replacement spawn rather than racing a helper that still holds the tap.
        let mut survivor = std::process::Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn survivor");
        assert!(!wait_for_pid_exit(
            survivor.id(),
            Duration::from_millis(100)
        ));
        let _ = survivor.kill();
        let _ = survivor.wait();
    }

    #[test]
    fn mic_duck_starts_on_listening_and_holds_through_levels() {
        assert_eq!(
            mic_duck_action_for_event(Some("listening_started")),
            Some(true)
        );
        // Mid-listening chatter must not lift the duck, or the tail of a long
        // dictation would leak back into the note.
        assert_eq!(mic_duck_action_for_event(Some("audio_level")), None);
        assert_eq!(mic_duck_action_for_event(Some("paste_target")), None);
    }

    #[test]
    fn mic_duck_lifts_on_every_way_listening_can_end() {
        // Success, discard, and each failure/terminal path all restore the mic
        // — a leaked duck would silently swallow the rest of the meeting.
        for event in [
            "recording_ready",
            "recording_discarded",
            "finalizing_transcript",
            "final_transcript",
            "paste_completed",
            "error",
            "helper_unavailable",
            "shutdown_ack",
        ] {
            assert_eq!(
                mic_duck_action_for_event(Some(event)),
                Some(false),
                "{event} should lift the mic duck"
            );
        }
    }

    #[test]
    fn mic_duck_ignores_unrelated_events() {
        assert_eq!(mic_duck_action_for_event(Some("permission_status")), None);
        assert_eq!(mic_duck_action_for_event(None), None);
    }

    #[test]
    fn the_gate_take_outlives_the_transcript_until_delivery_ends() {
        // On Windows the helper emits final_transcript before it writes the
        // clipboard, focuses the target and pastes; a composer delivery then
        // waits for the composer's answer. None of that may be cut.
        for event_type in ["final_transcript", "paste_target", "finalizing_transcript"] {
            assert_eq!(
                gate_take_transition(Some(event_type), Some("take-a")),
                None,
                "{event_type} must not end the take"
            );
        }
        for event_type in [
            "paste_completed",
            "recording_discarded",
            "agent_session_prompt",
            "error",
        ] {
            assert_eq!(
                gate_take_transition(Some(event_type), Some("take-a")),
                Some(GateTakeTransition::Ended("take-a")),
                "{event_type} ends the take"
            );
        }
    }

    #[test]
    fn a_late_terminal_event_for_an_older_take_still_ends_it_in_the_gate() {
        let gate = crate::update_gate::UpdateGate::new();
        let event = |event_type: &str, take_id: &str| serde_json::json!({ "type": event_type, "payload": { "takeId": take_id } });
        // final_transcript(A), then B starts, then A's paste completes late:
        // the active-take filter drops paste_completed(A) for the HUD.
        gate.begin_dictation_take("take-a").unwrap();
        end_gate_take_for_helper_event(&gate, &event("final_transcript", "take-a"));
        gate.begin_dictation_take("take-b").unwrap();
        end_gate_take_for_helper_event(&gate, &event("paste_completed", "take-a"));
        end_gate_take_for_helper_event(&gate, &event("paste_completed", "take-b"));
        assert!(gate.try_begin_install(|| false).is_ok());
    }

    #[test]
    fn every_terminal_event_of_a_stale_take_ends_it_in_the_gate() {
        for event_type in ["recording_discarded", "error", "agent_session_prompt"] {
            let gate = crate::update_gate::UpdateGate::new();
            gate.begin_dictation_take("take-stale").unwrap();
            end_gate_take_for_helper_event(
                &gate,
                &serde_json::json!({ "type": event_type, "payload": { "takeId": "take-stale" } }),
            );
            assert!(
                gate.try_begin_install(|| false).is_ok(),
                "{event_type} left the take in the gate"
            );
        }
        let gate = crate::update_gate::UpdateGate::new();
        gate.begin_dictation_take("take-x").unwrap();
        end_gate_take_for_helper_event(&gate, &serde_json::json!({ "type": "helper_unavailable" }));
        assert!(gate.try_begin_install(|| false).is_ok());
    }

    #[test]
    fn a_take_rust_cancels_leaves_the_gate() {
        // A superseded or discarded take produces no delivery, and its
        // transcription task returns without emitting anything.
        let gate = &crate::update_gate::UPDATE_GATE;
        let state = DictationTakeState::default();
        gate.begin_dictation_take("gate-cancel-a").unwrap();
        let take_a = state.begin("gate-cancel-a".into());
        assert!(take_a.try_begin_processing());
        gate.begin_dictation_take("gate-cancel-b").unwrap();
        state.begin("gate-cancel-b".into());
        assert!(!gate.has_dictation_take("gate-cancel-a"));
        assert!(state.cancel(Some("gate-cancel-b")));
        assert!(!gate.has_dictation_take("gate-cancel-b"));
    }

    #[test]
    fn a_stale_recording_leaves_the_gate() {
        // Rust drops a recording that is not the current take's without
        // transcribing it or sending the helper anything back.
        let gate = &crate::update_gate::UPDATE_GATE;
        let state = DictationTakeState::default();
        state.begin("gate-current".into());
        gate.begin_dictation_take("gate-stale").unwrap();
        assert!(matches!(
            state.claim_finalized(Some("gate-stale")),
            DictationFinalizedTake::Stale
        ));
        assert!(!gate.has_dictation_take("gate-stale"));
        gate.end_dictation_take("gate-current");
    }

    #[test]
    fn a_take_already_delivering_stays_in_the_gate_when_superseded() {
        let gate = &crate::update_gate::UPDATE_GATE;
        let state = DictationTakeState::default();
        gate.begin_dictation_take("gate-delivering-a").unwrap();
        let take_a = state.begin("gate-delivering-a".into());
        assert!(take_a.try_begin_processing());
        let _claim = take_a.try_begin_delivery().unwrap();
        state.begin("gate-delivering-b".into());
        // Its paste is in flight; paste_completed or an error will end it.
        assert!(gate.has_dictation_take("gate-delivering-a"));
        gate.end_dictation_take("gate-delivering-a");
        gate.end_dictation_take("gate-delivering-b");
    }

    #[test]
    fn the_gate_take_starts_and_the_helper_going_away_ends_every_take() {
        for event_type in ["recording_ready", "listening_started"] {
            assert_eq!(
                gate_take_transition(Some(event_type), Some("take-a")),
                Some(GateTakeTransition::Started("take-a"))
            );
        }
        for event_type in ["helper_unavailable", "shutdown_ack"] {
            assert_eq!(
                gate_take_transition(Some(event_type), None),
                Some(GateTakeTransition::AllEnded)
            );
        }
        assert_eq!(
            gate_take_transition(Some("audio_level"), Some("take-a")),
            None
        );
    }

    #[test]
    fn indicator_turns_on_for_every_active_phase() {
        // Must match DICTATION_ACTIVE_EVENTS in src/lib/dictation-events.ts.
        for event in [
            "recording_ready",
            "listening_started",
            "audio_level",
            "finalizing_transcript",
            "paste_target",
        ] {
            assert_eq!(
                indicator_action_for_event(Some(event)),
                Some(true),
                "{event} should light the menu-bar indicator"
            );
        }
    }

    #[test]
    fn indicator_clears_on_every_way_a_take_can_end() {
        // Must match DICTATION_FINISHED_EVENTS in src/lib/dictation-events.ts.
        // Success, discard, and each failure/terminal path all clear the menu
        // bar — a stuck indicator claims Clovy is listening when it is not.
        for event in [
            "recording_discarded",
            "final_transcript",
            "paste_completed",
            "agent_session_prompt",
            "error",
            "helper_unavailable",
            "shutdown_ack",
        ] {
            assert_eq!(
                indicator_action_for_event(Some(event)),
                Some(false),
                "{event} should clear the menu-bar indicator"
            );
        }
    }

    #[test]
    fn indicator_ignores_unrelated_events() {
        assert_eq!(indicator_action_for_event(Some("permission_status")), None);
        assert_eq!(indicator_action_for_event(None), None);
    }

    #[test]
    fn listening_start_is_ordered_and_correlated_with_its_start_command() {
        assert!(helper_event_serializes_with_commands(Some(
            "listening_started"
        )));
        assert!(helper_event_requires_take_match(Some("listening_started")));

        let mut controller = ShortcutActivationController::default();
        controller.prepare_helper_start("take-b");
        assert!(controller.matches_helper_take("take-b", None));
    }

    #[test]
    fn untagged_listening_start_promotes_the_pending_coordinator_take() {
        let event = serde_json::json!({
            "type": "listening_started",
            "payload": { "microphone": "Built-in Microphone", "escapeCancelAvailable": true }
        });
        let mut controller = ShortcutActivationController::default();
        controller.prepare_helper_start("take-pending");

        assert_eq!(
            dictation_start_take_id(&event, controller.pending_helper_take_id.as_deref()),
            "take-pending"
        );
        controller.promote_untagged_helper_start("take-pending");

        assert_eq!(controller.helper_take_id.as_deref(), Some("take-pending"));
        assert!(controller.pending_helper_take_id.is_none());
        assert!(!controller.helper_uses_take_ids);
    }

    #[test]
    fn malformed_legacy_ready_does_not_discard_a_pending_replacement() {
        assert!(helper_event_serializes_with_commands(Some(
            "recording_ready"
        )));

        let mut controller = ShortcutActivationController::default();
        controller.promote_untagged_helper_start("take-a");
        assert_eq!(
            controller.malformed_recording_ready_cleanup(None, Some("take-a")),
            MalformedRecordingReadyCleanup {
                take_id: Some("take-a".to_string()),
                send_helper_discard: true,
            }
        );

        controller.prepare_helper_start("take-b");
        assert_eq!(
            controller.malformed_recording_ready_cleanup(None, Some("take-a")),
            MalformedRecordingReadyCleanup {
                take_id: Some("take-a".to_string()),
                send_helper_discard: false,
            }
        );
        assert!(
            controller
                .malformed_recording_ready_cleanup(Some("take-a"), Some("take-a"))
                .send_helper_discard
        );
    }

    fn test_policy() -> RespawnPolicy {
        RespawnPolicy {
            max_attempts: 3,
            base_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(1),
            healthy_runtime: Duration::from_secs(30),
        }
    }

    #[test]
    fn intentional_shutdown_never_respawns() {
        let policy = test_policy();
        let (action, failures) = decide_respawn(true, 0, Duration::ZERO, &policy);
        assert_eq!(action, RespawnAction::Skip);
        // The failure count is left untouched on an intentional stop.
        assert_eq!(failures, 0);

        // Even mid-crash-loop, a shutdown wins and stops respawning.
        let (action, _) = decide_respawn(true, 2, Duration::ZERO, &policy);
        assert_eq!(action, RespawnAction::Skip);
    }

    #[test]
    fn unexpected_exit_respawns_and_counts_up() {
        let policy = test_policy();
        let (action, failures) = decide_respawn(false, 0, Duration::ZERO, &policy);
        assert_eq!(
            action,
            RespawnAction::Respawn {
                attempt: 1,
                backoff: Duration::from_millis(100),
            }
        );
        assert_eq!(failures, 1);

        let (action, failures) = decide_respawn(false, 1, Duration::ZERO, &policy);
        assert_eq!(
            action,
            RespawnAction::Respawn {
                attempt: 2,
                backoff: Duration::from_millis(200),
            }
        );
        assert_eq!(failures, 2);
    }

    #[test]
    fn retry_cap_exhaustion_gives_up() {
        let policy = test_policy();
        // The last allowed attempt still respawns.
        let (action, failures) = decide_respawn(false, 2, Duration::ZERO, &policy);
        assert!(matches!(action, RespawnAction::Respawn { attempt: 3, .. }));
        assert_eq!(failures, 3);

        // One past the cap gives up instead of respawning.
        let (action, _) = decide_respawn(false, 3, Duration::ZERO, &policy);
        assert_eq!(action, RespawnAction::GiveUp);
    }

    #[test]
    fn healthy_runtime_resets_the_failure_count() {
        let policy = test_policy();
        // A helper that had exhausted its retries but then ran healthily is
        // treated as a fresh first failure, so an isolated later kill recovers.
        let (action, failures) = decide_respawn(
            false,
            3,
            policy.healthy_runtime + Duration::from_secs(1),
            &policy,
        );
        assert_eq!(
            action,
            RespawnAction::Respawn {
                attempt: 1,
                backoff: Duration::from_millis(100),
            }
        );
        assert_eq!(failures, 1);
    }

    #[test]
    fn respawn_backoff_doubles_and_clamps() {
        let policy = test_policy();
        assert_eq!(respawn_backoff(1, &policy), Duration::from_millis(100));
        assert_eq!(respawn_backoff(2, &policy), Duration::from_millis(200));
        assert_eq!(respawn_backoff(3, &policy), Duration::from_millis(400));
        // Clamped to max_backoff once doubling would overshoot it.
        assert_eq!(respawn_backoff(20, &policy), policy.max_backoff);
    }

    #[test]
    fn respawn_decision_records_failure_count() {
        let state = HelperState::default();
        let policy = test_policy();

        let (action, failures) = decide_and_record_respawn(&state, Duration::ZERO, &policy);
        assert!(matches!(action, RespawnAction::Respawn { attempt: 1, .. }));
        assert_eq!(failures, 1);
        assert_eq!(state.respawn_failures.load(Ordering::SeqCst), 1);

        let (action, failures) = decide_and_record_respawn(&state, Duration::ZERO, &policy);
        assert!(matches!(action, RespawnAction::Respawn { attempt: 2, .. }));
        assert_eq!(failures, 2);
        assert_eq!(state.respawn_failures.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn default_settings_use_keyboard_shortcuts_without_bare_fn() {
        let settings = DictationSettings::default();

        assert_eq!(
            settings.push_to_talk_shortcut,
            DictationShortcutSetting::control_option_d()
        );
        assert_eq!(
            settings.toggle_shortcut,
            DictationShortcutSetting::control_option_t()
        );
        assert_eq!(settings.style, DictationStyle::Standard);
        assert_eq!(settings.language, None);
    }

    #[test]
    fn default_settings_leave_dictation_cleanup_off_on_the_default_openrouter_model() {
        let settings = DictationSettings::default();
        assert!(!settings.cleanup_enabled);
        assert_eq!(settings.cleanup_model, DEFAULT_DICTATION_CLEANUP_MODEL);
        assert!(settings.cleanup_migrated);
    }

    #[test]
    fn settings_saved_before_optional_cleanup_migrate_to_off_once() {
        // A file from before ditado-rapido: cleanup always ran (on the CLI,
        // an endpoint, or Clovy API). It migrates to off, once.
        let mut settings: DictationSettings = serde_json::from_str(
            r#"{"pushToTalkShortcut":{"keyCode":0,"code":"Fn","modifiers":{"command":false,"control":false,"option":false,"shift":false,"function":true},"label":"Fn","pressCount":1},"style":"formal","cleanupEnabled":true}"#,
        )
        .expect("legacy settings should deserialize");
        assert!(!settings.cleanup_migrated);
        assert!(migrate_dictation_cleanup(&mut settings));
        assert!(!settings.cleanup_enabled);
        assert!(settings.cleanup_migrated);
        assert_eq!(settings.style, DictationStyle::Formal);

        // After the migration the user turns cleanup on; the next launch
        // reads it back and the migration does not run again.
        settings.cleanup_enabled = true;
        let stored = serde_json::to_string(&settings).unwrap();
        let mut reloaded: DictationSettings = serde_json::from_str(&stored).unwrap();
        assert!(!migrate_dictation_cleanup(&mut reloaded));
        assert!(reloaded.cleanup_enabled);
        assert_eq!(reloaded.cleanup_model, DEFAULT_DICTATION_CLEANUP_MODEL);
    }

    #[test]
    fn dictation_cleanup_model_accepts_only_openrouter_model_ids() {
        assert_eq!(
            validate_dictation_cleanup_model(" openai/gpt-5-mini ").unwrap(),
            "openai/gpt-5-mini"
        );
        assert_eq!(
            validate_dictation_cleanup_model("nvidia/nemotron-3-nano-30b-a3b:free").unwrap(),
            "nvidia/nemotron-3-nano-30b-a3b:free"
        );
        for invalid in [
            "",
            "   ",
            "claude",
            "cli:claude",
            "endpoint:ep",
            "/model",
            "vendor/",
            "vendor/model name",
            "__june_local_generation__:x/y",
        ] {
            assert_eq!(
                validate_dictation_cleanup_model(invalid).unwrap_err().code,
                "dictation_cleanup_model_invalid",
                "{invalid:?}"
            );
        }
        assert!(validate_dictation_cleanup_model(&format!("a/{}", "b".repeat(200))).is_err());
    }

    /// A transcriber that returns a fixed result and counts its calls.
    struct FakeTranscriber {
        result: Result<String, AppError>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl FakeTranscriber {
        fn text(text: &str) -> Self {
            Self {
                result: Ok(text.to_string()),
                calls: Default::default(),
            }
        }
    }

    impl DictationTranscriber for FakeTranscriber {
        async fn transcribe(
            &self,
            _request: DictateTranscribeRequest,
        ) -> Result<TranscriptionProviderResult, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.clone().map(|text| TranscriptionProviderResult {
                text,
                language: None,
                provider: "groq".to_string(),
            })
        }
    }

    /// A cleaner that records every call and the peak number of calls in
    /// flight. `fail_on` makes calls whose text contains it fail; `delay`
    /// keeps each call open long enough to observe parallel chunks.
    #[derive(Default)]
    struct FakeCleaner {
        calls: Mutex<Vec<DictationCleanupCall>>,
        in_flight: std::sync::atomic::AtomicUsize,
        peak_in_flight: std::sync::atomic::AtomicUsize,
        delay: Duration,
        fail_on: Option<String>,
        failure_code: Option<&'static str>,
        /// Calls whose text contains it answer with empty text.
        blank_on: Option<String>,
        /// Answers without any sentence punctuation.
        unpunctuated: bool,
    }

    impl DictationCleaner for FakeCleaner {
        async fn clean(&self, call: DictationCleanupCall) -> Result<String, AppError> {
            let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak_in_flight.fetch_max(now, Ordering::SeqCst);
            self.calls.lock().unwrap().push(call.clone());
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.in_flight.fetch_sub(1, Ordering::SeqCst);
            if self
                .fail_on
                .as_deref()
                .is_some_and(|marker| call.text.contains(marker))
            {
                return Err(AppError::new(
                    self.failure_code.unwrap_or("clovy_request_failed"),
                    "upstream failed",
                ));
            }
            if self
                .blank_on
                .as_deref()
                .is_some_and(|marker| call.text.contains(marker))
            {
                return Ok(String::new());
            }
            if self.unpunctuated {
                return Ok(call.text.to_uppercase());
            }
            Ok(format!("{}.", call.text.to_uppercase()))
        }
    }

    fn pipeline_request() -> DictateTranscribeRequest {
        DictateTranscribeRequest {
            audio_path: PathBuf::from("/tmp/take.m4a"),
            context: None,
            language: None,
            session_id: "session".to_string(),
            utterance_id: "utterance".to_string(),
        }
    }

    fn cleanup_config(enabled: bool) -> DictationCleanupConfig {
        DictationCleanupConfig {
            enabled,
            model: "openai/gpt-5-mini".to_string(),
            style: DictationStyle::Standard,
            timeout: dictation_cleanup_timeout,
        }
    }

    fn pasted_text(result: &DictationPipelineResult) -> Option<&str> {
        let command = &result.outcome.helper_command;
        (command["type"] == "paste_text").then(|| command["text"].as_str().unwrap())
    }

    /// Words long enough that the transcript splits into several cleanup
    /// chunks; `marker` tags the one word a failing cleaner trips on.
    fn long_transcript(chunks: usize, marker: &str) -> String {
        let mut words = Vec::new();
        for index in 0..chunks * 100 {
            if index == chunks * 50 {
                words.push(marker.to_string());
            } else {
                words.push(format!("word{index}"));
            }
        }
        words.join(" ")
    }

    #[tokio::test]
    async fn dictation_with_cleanup_off_pastes_the_transcript_without_calling_the_cleaner() {
        let transcriber = FakeTranscriber::text("send it to samir");
        let cleaner = FakeCleaner::default();
        let result = run_dictation_pipeline(
            &transcriber,
            &cleaner,
            pipeline_request(),
            &cleanup_config(false),
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        assert_eq!(transcriber.calls.load(Ordering::SeqCst), 1);
        assert!(cleaner.calls.lock().unwrap().is_empty());
        assert!(result.cleanup_skipped.is_none());
        // No LLM and no filler backstop: the Whisper text, trimmed.
        assert_eq!(pasted_text(&result), Some("send it to samir"));
    }

    #[tokio::test]
    async fn dictation_with_cleanup_off_pastes_whisper_text_only_trimmed() {
        // Spec "Limpeza desligada": no filler backstop, no style either.
        let transcriber = FakeTranscriber::text("  um, so Send it to Samir, uh  ");
        let result = run_dictation_pipeline(
            &transcriber,
            &FakeCleaner::default(),
            pipeline_request(),
            &DictationCleanupConfig {
                style: DictationStyle::CasualLowercase,
                ..cleanup_config(false)
            },
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        assert_eq!(pasted_text(&result), Some("um, so Send it to Samir, uh"));
        assert_eq!(
            result.outcome.transcript.as_ref().map(|t| t.text.as_str()),
            Some("um, so Send it to Samir, uh")
        );
    }

    #[tokio::test]
    async fn dictation_with_cleanup_on_pastes_the_cleaned_text_from_the_chosen_model() {
        let transcriber = FakeTranscriber::text("send it to samir");
        let cleaner = FakeCleaner::default();
        let result = run_dictation_pipeline(
            &transcriber,
            &cleaner,
            pipeline_request(),
            &DictationCleanupConfig {
                style: DictationStyle::Formal,
                ..cleanup_config(true)
            },
            DictationCleanupContext {
                dictionary_context: Some("Samir".to_string()),
                app_context: Some(APP_CONTEXT_EMAIL.to_string()),
            },
            Some(0.3),
            None,
        )
        .await;
        let calls = cleaner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].model, "openai/gpt-5-mini");
        assert_eq!(calls[0].style, DictationStyle::Formal);
        assert_eq!(calls[0].dictionary_context.as_deref(), Some("Samir"));
        assert_eq!(calls[0].app_context.as_deref(), Some(APP_CONTEXT_EMAIL));
        assert_eq!(calls[0].utterance_id, "utterance");
        assert_eq!(pasted_text(&result), Some("SEND IT TO SAMIR."));
        assert!(result.cleanup_skipped.is_none());
    }

    #[tokio::test]
    async fn failed_cleanup_pastes_the_raw_transcript_and_reports_the_skip() {
        let transcriber = FakeTranscriber::text("send it to samir");
        let cleaner = FakeCleaner {
            fail_on: Some("samir".to_string()),
            ..FakeCleaner::default()
        };
        let result = run_dictation_pipeline(
            &transcriber,
            &cleaner,
            pipeline_request(),
            &cleanup_config(true),
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        assert_eq!(pasted_text(&result), Some("send it to samir"));
        assert_eq!(
            result.cleanup_skipped.map(|error| error.code).as_deref(),
            Some("clovy_request_failed")
        );
    }

    #[tokio::test]
    async fn timed_out_cleanup_pastes_the_raw_transcript() {
        let transcriber = FakeTranscriber::text("send it to samir");
        let cleaner = FakeCleaner {
            delay: Duration::from_millis(500),
            ..FakeCleaner::default()
        };
        let result = run_dictation_pipeline(
            &transcriber,
            &cleaner,
            pipeline_request(),
            &DictationCleanupConfig {
                timeout: |_| Duration::from_millis(20),
                ..cleanup_config(true)
            },
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        assert_eq!(pasted_text(&result), Some("send it to samir"));
        assert_eq!(
            result.cleanup_skipped.map(|error| error.code).as_deref(),
            Some("dictation_cleanup_timeout")
        );
    }

    #[test]
    fn paste_completion_fallback_keeps_the_cleanup_notice_readable() {
        // hud.ts keeps the missing-key notice for 3 s, then allows 240 ms
        // for its exit animation plus a 60 ms timeout margin.
        let notice_and_exit = Duration::from_millis(3300);
        assert!(hud_hide_delay_for_event(Some("paste_completed"), None) >= notice_and_exit);
        assert_eq!(
            hud_hide_delay_for_event(Some("recording_discarded"), None),
            Duration::from_millis(900)
        );
        assert_eq!(
            hud_hide_delay_for_event(Some("shutdown_ack"), None),
            Duration::ZERO
        );
    }

    #[tokio::test]
    async fn missing_openrouter_key_pastes_raw_dictation_with_a_reason_for_the_notice() {
        let transcriber = FakeTranscriber::text("send it to samir");
        let cleaner = FakeCleaner {
            fail_on: Some("samir".to_string()),
            failure_code: Some("openrouter_api_key_missing"),
            ..FakeCleaner::default()
        };
        let result = run_dictation_pipeline(
            &transcriber,
            &cleaner,
            pipeline_request(),
            &cleanup_config(true),
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        assert_eq!(pasted_text(&result), Some("send it to samir"));
        assert_eq!(
            result.cleanup_skipped.map(|error| error.code).as_deref(),
            Some("openrouter_api_key_missing")
        );
    }

    #[tokio::test]
    async fn long_dictation_cleans_chunks_in_parallel_up_to_four_and_keeps_their_order() {
        let transcript = long_transcript(7, "word350");
        let chunks =
            split_dictation_cleanup_chunks(&transcript, DICTATION_CLEANUP_CHUNK_TARGET_BYTES);
        assert!(chunks.len() > 4, "{} chunks", chunks.len());
        let expected = chunks
            .iter()
            .map(|chunk| format!("{}.", chunk.to_uppercase()))
            .collect::<Vec<_>>()
            .join(" ");
        let transcriber = FakeTranscriber::text(&transcript);
        let cleaner = FakeCleaner {
            delay: Duration::from_millis(40),
            ..FakeCleaner::default()
        };
        let result = run_dictation_pipeline(
            &transcriber,
            &cleaner,
            pipeline_request(),
            &cleanup_config(true),
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        assert_eq!(cleaner.calls.lock().unwrap().len(), chunks.len());
        let peak = cleaner.peak_in_flight.load(Ordering::SeqCst);
        assert!((2..=4).contains(&peak), "peak in flight {peak}");
        assert_eq!(pasted_text(&result), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn each_chunk_gets_exactly_one_cleanup_call() {
        // D7: one chat completion per chunk, no content-quality retries.
        let transcript = long_transcript(6, "word300");
        let chunks =
            split_dictation_cleanup_chunks(&transcript, DICTATION_CLEANUP_CHUNK_TARGET_BYTES);
        let cleaner = FakeCleaner {
            unpunctuated: true,
            ..FakeCleaner::default()
        };
        run_dictation_pipeline(
            &FakeTranscriber::text(&transcript),
            &cleaner,
            pipeline_request(),
            &cleanup_config(true),
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        assert_eq!(cleaner.calls.lock().unwrap().len(), chunks.len());
    }

    #[tokio::test]
    async fn a_chunk_with_an_invalid_answer_pastes_the_whole_raw_transcript() {
        let transcript = long_transcript(6, "blankme");
        let chunks =
            split_dictation_cleanup_chunks(&transcript, DICTATION_CLEANUP_CHUNK_TARGET_BYTES);
        let cleaner = FakeCleaner {
            blank_on: Some("blankme".to_string()),
            ..FakeCleaner::default()
        };
        let result = run_dictation_pipeline(
            &FakeTranscriber::text(&transcript),
            &cleaner,
            pipeline_request(),
            &cleanup_config(true),
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        let pasted = pasted_text(&result).unwrap();
        assert!(!pasted.contains('.'), "no cleaned chunk survives: {pasted}");
        assert_eq!(
            result.cleanup_skipped.map(|error| error.code).as_deref(),
            Some("provider_response_invalid")
        );
        assert!(cleaner.calls.lock().unwrap().len() <= chunks.len());
    }

    #[tokio::test]
    async fn one_failed_chunk_pastes_the_whole_raw_transcript() {
        let transcript = long_transcript(6, "explode");
        let transcriber = FakeTranscriber::text(&transcript);
        let cleaner = FakeCleaner {
            fail_on: Some("explode".to_string()),
            ..FakeCleaner::default()
        };
        let result = run_dictation_pipeline(
            &transcriber,
            &cleaner,
            pipeline_request(),
            &cleanup_config(true),
            DictationCleanupContext::default(),
            Some(0.3),
            None,
        )
        .await;
        let pasted = pasted_text(&result).unwrap();
        assert!(!pasted.contains('.'), "no cleaned chunk survives: {pasted}");
        assert!(pasted.contains("explode"));
        assert!(result.cleanup_skipped.is_some());
    }

    fn assert_visible_no_speech(result: &DictationPipelineResult) {
        assert_eq!(
            result.outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(result.outcome.transcript.is_none());
        let mut event = result
            .outcome
            .event
            .clone()
            .expect("no speech emits an event");
        annotate_silent_error(&mut event);
        assert_eq!(event["payload"]["code"], "no_speech");
        assert_eq!(event["payload"]["message"], "No speech detected.");
        // The user sees "No speech detected"; the HUD does not just vanish.
        assert_eq!(event["payload"]["silent"], false);
    }

    #[tokio::test]
    async fn an_empty_whisper_transcript_is_no_speech_whatever_the_audio_level() {
        // The meter level is diagnostic only (design D5): loud, quiet, or
        // unknown, an empty transcript means no speech, and nothing is cleaned.
        for level in [Some(0.9), Some(0.001), None] {
            for text in ["", "   ", " . , ? ", "…"] {
                let transcriber = FakeTranscriber::text(text);
                let cleaner = FakeCleaner::default();
                let result = run_dictation_pipeline(
                    &transcriber,
                    &cleaner,
                    pipeline_request(),
                    &cleanup_config(true),
                    DictationCleanupContext::default(),
                    level,
                    None,
                )
                .await;
                assert_visible_no_speech(&result);
                assert!(cleaner.calls.lock().unwrap().is_empty(), "{text:?}");
            }
        }
    }

    #[tokio::test]
    async fn a_take_under_half_a_second_never_reaches_whisper() {
        // R7a: Whisper invents text for near-empty audio; such a take is no
        // speech without an upload. Unknown length (older helpers) uploads.
        for (seconds, uploads) in [
            (Some(0.2), false),
            (Some(0.49), false),
            (Some(0.5), true),
            (Some(3.0), true),
            (None, true),
        ] {
            let transcriber = FakeTranscriber::text("Thank you.");
            let result = run_dictation_pipeline(
                &transcriber,
                &FakeCleaner::default(),
                pipeline_request(),
                &cleanup_config(false),
                DictationCleanupContext::default(),
                Some(0.3),
                seconds,
            )
            .await;
            assert_eq!(
                transcriber.calls.load(Ordering::SeqCst) == 1,
                uploads,
                "{seconds:?}"
            );
            if !uploads {
                assert_visible_no_speech(&result);
            }
        }
    }

    #[test]
    fn recording_ready_info_reads_the_audio_length() {
        let event = |value: serde_json::Value| {
            serde_json::json!({
                "type": "recording_ready",
                "payload": { "path": "/tmp/take.m4a", "audioDurationSeconds": value },
            })
        };
        let seconds = |value| {
            recording_ready_info_from_event(&event(value))
                .unwrap()
                .audio_seconds
        };
        assert_eq!(seconds(serde_json::json!("0.4200")), Some(0.42));
        assert_eq!(seconds(serde_json::json!(2.5)), Some(2.5));
        assert_eq!(seconds(serde_json::json!("-1")), None);
        assert_eq!(seconds(serde_json::json!("nope")), None);
    }

    #[tokio::test]
    async fn clovy_apis_empty_dictation_answer_is_the_same_no_speech() {
        for message in ["no_speech", "dictation_text_empty"] {
            let transcriber = FakeTranscriber {
                result: Err(AppError::new("clovy_request_failed", message)),
                calls: Default::default(),
            };
            let result = run_dictation_pipeline(
                &transcriber,
                &FakeCleaner::default(),
                pipeline_request(),
                &cleanup_config(false),
                DictationCleanupContext::default(),
                Some(0.5),
                None,
            )
            .await;
            assert_visible_no_speech(&result);
        }
    }

    #[test]
    fn legacy_settings_reset_to_defaults_and_preserve_microphone() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"shortcut":"control_option_space","activationMode":"toggle","microphone":{"id":"usb","name":"USB Mic"}}"#,
        )
        .expect("legacy shortcut should deserialize");

        assert_eq!(
            settings.push_to_talk_shortcut,
            DictationShortcutSetting::control_option_d()
        );
        assert_eq!(
            settings.toggle_shortcut,
            DictationShortcutSetting::control_option_t()
        );
        assert_eq!(settings.microphone.id.as_deref(), Some("usb"));
        assert_eq!(settings.microphone.name.as_deref(), Some("USB Mic"));
        assert_eq!(settings.style, DictationStyle::Standard);
        assert_eq!(settings.language, None);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn deserializes_new_shortcut_settings_and_preserves_bare_fn_push_to_talk() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"pushToTalkShortcut":{"keyCode":0,"code":"Fn","modifiers":{"command":false,"control":false,"option":false,"shift":false,"function":true},"label":"Fn","pressCount":1},"toggleShortcut":{"keyCode":49,"code":"Space","modifiers":{"command":false,"control":true,"option":true,"shift":false,"function":false},"label":"Ctrl+Opt+Space","pressCount":1},"microphone":{"id":null,"name":null}}"#,
        )
        .expect("new settings should deserialize");

        assert_eq!(
            settings.push_to_talk_shortcut,
            DictationShortcutSetting::bare_fn()
        );
        assert_eq!(settings.push_to_talk_shortcut.press_count, 1);
        assert_eq!(settings.toggle_shortcut.key_code, 0x31);
        assert_eq!(settings.toggle_shortcut.code, "Space");
        assert_eq!(settings.toggle_shortcut.label, "Ctrl+Opt+Space");
        assert_eq!(settings.toggle_shortcut.press_count, 1);
        assert_eq!(settings.style, DictationStyle::Standard);
        assert_eq!(settings.language, None);
    }

    #[test]
    fn deserializes_control_letter_and_digit_shortcuts() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"pushToTalkShortcut":{"keyCode":999,"code":"KeyT","modifiers":{"command":false,"control":true,"option":false,"shift":false,"function":false},"label":"Ctrl+T","pressCount":1},"toggleShortcut":{"keyCode":999,"code":"Digit1","modifiers":{"command":false,"control":true,"option":false,"shift":false,"function":false},"label":"Ctrl+1","pressCount":1},"microphone":{"id":null,"name":null}}"#,
        )
        .expect("control shortcuts should deserialize");

        assert_eq!(settings.push_to_talk_shortcut.key_code, 0x11);
        assert_eq!(settings.push_to_talk_shortcut.code, "KeyT");
        assert_eq!(settings.push_to_talk_shortcut.label, "Ctrl+T");
        assert_eq!(settings.toggle_shortcut.key_code, 0x12);
        assert_eq!(settings.toggle_shortcut.code, "Digit1");
        assert_eq!(settings.toggle_shortcut.label, "Ctrl+1");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn deserializes_modifier_only_shortcuts() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"pushToTalkShortcut":{"keyCode":0,"code":"Modifiers","modifiers":{"command":false,"control":true,"option":true,"shift":false,"function":false},"label":"Ctrl+Opt","pressCount":1},"toggleShortcut":{"keyCode":999,"code":"Digit1","modifiers":{"command":false,"control":true,"option":false,"shift":false,"function":false},"label":"Ctrl+1","pressCount":1},"microphone":{"id":null,"name":null}}"#,
        )
        .expect("modifier-only shortcuts should deserialize");

        assert_eq!(settings.push_to_talk_shortcut.key_code, 0);
        assert_eq!(settings.push_to_talk_shortcut.code, "Modifiers");
        assert_eq!(settings.push_to_talk_shortcut.label, "Ctrl+Opt");
        assert!(settings.push_to_talk_shortcut.modifiers.control);
        assert!(settings.push_to_talk_shortcut.modifiers.option);
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn shortcut_updates_are_written_in_reloadable_settings_file() {
        let directory = tempfile::tempdir().expect("settings tempdir should be created");
        let state = DictationSettingsState {
            path: directory.path().join("dictation-settings.json"),
            settings: Mutex::new(DictationSettings::default()),
        };

        let settings = update_settings(&state, |settings| {
            settings.push_to_talk_shortcut = DictationShortcutSetting::bare_fn();
            settings.toggle_shortcut = DictationShortcutSetting {
                key_code: 0x31,
                code: "Space".to_string(),
                modifiers: DictationShortcutModifiers {
                    control: true,
                    option: true,
                    ..DictationShortcutModifiers::default()
                },
                label: "Ctrl+Opt+Space".to_string(),
                press_count: 1,
            };
        })
        .expect("shortcut settings should save");

        let stored = std::fs::read_to_string(&state.path).expect("settings file should be written");
        let reloaded: DictationSettings =
            serde_json::from_str(&stored).expect("settings file should reload");

        assert_eq!(
            reloaded.push_to_talk_shortcut,
            settings.push_to_talk_shortcut
        );
        assert_eq!(reloaded.toggle_shortcut, settings.toggle_shortcut);
    }

    #[test]
    fn deserializes_unsupported_modifier_only_shortcuts_to_defaults() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"pushToTalkShortcut":{"keyCode":59,"code":"ControlLeft","modifiers":{"command":false,"control":true,"option":true,"shift":false,"function":false},"label":"Ctrl+Opt","pressCount":1},"toggleShortcut":{"keyCode":58,"code":"OptionLeft","modifiers":{"command":false,"control":false,"option":true,"shift":false,"function":false},"label":"Opt","pressCount":1},"microphone":{"id":null,"name":null}}"#,
        )
        .expect("settings should deserialize");

        assert_eq!(
            settings.push_to_talk_shortcut,
            DictationShortcutSetting::control_option_d()
        );
        assert_eq!(
            settings.toggle_shortcut,
            DictationShortcutSetting::control_option_t()
        );
    }

    #[test]
    fn deserializes_dictation_style() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"pushToTalkShortcut":{"keyCode":0,"code":"Fn","modifiers":{"command":false,"control":false,"option":false,"shift":false,"function":true},"label":"Fn","pressCount":1},"toggleShortcut":{"keyCode":49,"code":"Space","modifiers":{"command":false,"control":true,"option":true,"shift":false,"function":false},"label":"Ctrl+Opt+Space","pressCount":1},"microphone":{"id":null,"name":null},"style":"casualLowercase"}"#,
        )
        .expect("style settings should deserialize");

        assert_eq!(settings.style, DictationStyle::CasualLowercase);
    }

    #[test]
    fn deserializes_default_transcription_language() {
        let settings: DictationSettings = serde_json::from_str(
            r#"{"pushToTalkShortcut":{"keyCode":0,"code":"Fn","modifiers":{"command":false,"control":false,"option":false,"shift":false,"function":true},"label":"Fn","pressCount":1},"toggleShortcut":{"keyCode":49,"code":"Space","modifiers":{"command":false,"control":true,"option":true,"shift":false,"function":false},"label":"Ctrl+Opt+Space","pressCount":1},"microphone":{"id":null,"name":null},"language":"ES"}"#,
        )
        .expect("language settings should deserialize");

        assert_eq!(settings.language.as_deref(), Some("es"));
    }

    #[test]
    fn ignores_invalid_default_transcription_language() {
        assert_eq!(normalize_language(Some("english".to_string())), None);
        assert_eq!(normalize_language(Some(" e ".to_string())), None);
        assert_eq!(
            normalize_language(Some(" en ".to_string())).as_deref(),
            Some("en")
        );
    }

    #[test]
    fn shortcut_input_accepts_bare_fn() {
        let shortcut = DictationShortcutInput {
            code: "Fn".to_string(),
            modifiers: DictationShortcutModifiers {
                function: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Fn".to_string(),
            press_count: None,
        }
        .into_setting_for_platform(false)
        .expect("bare Fn should be accepted");

        assert_eq!(shortcut, DictationShortcutSetting::bare_fn());
    }

    #[test]
    fn shortcut_input_normalizes_double_bare_fn_to_single_press() {
        let shortcut = DictationShortcutInput {
            code: "Fn".to_string(),
            modifiers: DictationShortcutModifiers {
                function: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Fn".to_string(),
            press_count: Some(2),
        }
        .into_setting_for_platform(false)
        .expect("bare Fn should be accepted");

        assert_eq!(shortcut, DictationShortcutSetting::bare_fn());
    }

    #[test]
    fn shortcut_input_accepts_control_letter_and_normalizes_press_count() {
        let shortcut = DictationShortcutInput {
            code: "KeyT".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+T".to_string(),
            press_count: Some(2),
        }
        .into_setting_for_platform(false)
        .expect("control letter should be accepted");

        assert_eq!(shortcut.key_code, 0x11);
        assert_eq!(shortcut.code, "KeyT");
        assert!(shortcut.modifiers.control);
        assert_eq!(shortcut.press_count, 1);
    }

    #[test]
    fn windows_shortcut_input_rejects_bare_fn() {
        let err = DictationShortcutInput {
            code: "Fn".to_string(),
            modifiers: DictationShortcutModifiers {
                function: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Fn".to_string(),
            press_count: Some(1),
        }
        .into_setting_for_platform(true)
        .expect_err("Windows must reject bare Fn");

        assert_eq!(err.code, "dictation_shortcut_unsupported");
    }

    #[test]
    fn windows_shortcut_input_rejects_modifier_only_combo() {
        let err = DictationShortcutInput {
            code: "Modifiers".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                option: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+Alt".to_string(),
            press_count: Some(1),
        }
        .into_setting_for_platform(true)
        .expect_err("Windows must reject modifier-only shortcuts");

        assert_eq!(err.code, "dictation_shortcut_unsupported");
    }

    #[test]
    fn windows_shortcut_input_rejects_fn_chord() {
        let err = DictationShortcutInput {
            code: "KeyD".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                function: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+Fn+D".to_string(),
            press_count: Some(1),
        }
        .into_setting_for_platform(true)
        .expect_err("Windows must reject Fn chords");

        assert_eq!(err.code, "dictation_shortcut_unsupported");
    }

    #[test]
    fn windows_shortcut_input_rejects_double_press() {
        let err = DictationShortcutInput {
            code: "KeyD".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                option: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+Alt+D".to_string(),
            press_count: Some(2),
        }
        .into_setting_for_platform(true)
        .expect_err("Windows must reject double-press shortcuts");

        assert_eq!(err.code, "dictation_shortcut_unsupported");
    }

    fn lone(code: &str, label: &str) -> DictationShortcutInput {
        DictationShortcutInput {
            code: code.to_string(),
            modifiers: DictationShortcutModifiers::default(),
            label: label.to_string(),
            press_count: Some(1),
        }
    }

    #[test]
    fn windows_shortcut_input_accepts_a_lone_right_modifier() {
        for (code, label) in [
            ("ControlRight", "Right Ctrl"),
            ("AltRight", "Right Alt"),
            ("ShiftRight", "Right Shift"),
        ] {
            let shortcut = lone(code, label)
                .into_setting_for_platform(true)
                .unwrap_or_else(|error| panic!("{code}: {error:?}"));
            assert_eq!(shortcut.code, code);
            assert_eq!(shortcut.label, label);
            assert_eq!(shortcut.modifiers, DictationShortcutModifiers::default());
        }
    }

    #[test]
    fn windows_shortcut_input_accepts_a_lone_key_windows_sees() {
        for code in ["F13", "F24", "Pause", "ScrollLock", "Insert"] {
            lone(code, code)
                .into_setting_for_platform(true)
                .unwrap_or_else(|error| panic!("{code}: {error:?}"));
        }
    }

    #[test]
    fn windows_shortcut_input_needs_a_modifier_for_ordinary_keys() {
        for code in ["KeyD", "F5", "Space"] {
            let error = lone(code, code)
                .into_setting_for_platform(true)
                .expect_err(code);
            assert_eq!(error.code, "dictation_shortcut_modifier_required", "{code}");
        }
    }

    #[test]
    fn windows_shortcut_input_accepts_ctrl_alt_and_shift_of_either_side() {
        for (code, label) in [
            ("ControlLeft", "Left Ctrl"),
            ("AltLeft", "Left Alt"),
            ("ShiftLeft", "Left Shift"),
            ("ControlRight", "Right Ctrl"),
            ("AltRight", "Right Alt"),
            ("ShiftRight", "Right Shift"),
        ] {
            let shortcut = lone(code, label)
                .into_setting_for_platform(true)
                .unwrap_or_else(|error| panic!("{code}: {error:?}"));
            assert_eq!(shortcut.code, code);
            assert_eq!(shortcut.press_count, 1);
        }
    }

    /// The Mac Fn toggle gesture: a double tap of the same lone modifier.
    #[test]
    fn windows_lone_modifiers_take_a_double_tap_and_nothing_else_does() {
        let double = |code: &str, label: &str| DictationShortcutInput {
            press_count: Some(2),
            ..lone(code, label)
        };
        let shortcut = double("ControlLeft", "Left Ctrl+Left Ctrl")
            .into_setting_for_platform(true)
            .expect("a double-tapped lone modifier");
        assert_eq!(shortcut.press_count, 2);
        for (code, label) in [("F13", "F13+F13"), ("Pause", "Pause+Pause")] {
            let error = double(code, label)
                .into_setting_for_platform(true)
                .expect_err(code);
            assert_eq!(error.code, "dictation_shortcut_unsupported", "{code}");
        }
    }

    #[test]
    fn windows_shortcut_input_refuses_the_windows_key_alone() {
        for code in ["MetaLeft", "MetaRight"] {
            let error = lone(code, code)
                .into_setting_for_platform(true)
                .expect_err(code);
            assert_eq!(error.code, "dictation_shortcut_unsupported", "{code}");
        }
    }

    fn chord(
        code: &str,
        label: &str,
        modifiers: DictationShortcutModifiers,
    ) -> DictationShortcutInput {
        DictationShortcutInput {
            code: code.to_string(),
            modifiers,
            label: label.to_string(),
            press_count: Some(1),
        }
    }

    /// The helper captures chords on the keys Windows has beyond the Mac
    /// table (F13 to F24, Pause, Insert...); they save and survive loading
    /// on Windows only.
    #[test]
    fn windows_shortcut_input_accepts_chords_on_extended_keys() {
        let control = DictationShortcutModifiers {
            control: true,
            ..DictationShortcutModifiers::default()
        };
        let shift = DictationShortcutModifiers {
            shift: true,
            ..DictationShortcutModifiers::default()
        };
        for (code, label, modifiers) in [
            ("F13", "Ctrl+F13", &control),
            ("Insert", "Shift+Insert", &shift),
            ("Pause", "Ctrl+Pause", &control),
            ("F24", "Ctrl+F24", &control),
            ("ScrollLock", "Shift+Scroll Lock", &shift),
        ] {
            let saved = chord(code, label, modifiers.clone())
                .into_setting_for_platform(true)
                .unwrap_or_else(|error| panic!("{label}: {error:?}"));
            assert_eq!(saved.code, code);
            assert_eq!(&saved.modifiers, modifiers);
            assert_eq!(saved.label, label);
            for kind in [
                DictationShortcutKind::PushToTalk,
                DictationShortcutKind::Toggle,
            ] {
                assert_eq!(
                    saved.clone().normalized_for_platform(kind, true),
                    saved,
                    "{label} survives loading"
                );
            }
            assert!(
                chord(code, label, modifiers.clone())
                    .into_setting_for_platform(false)
                    .is_err(),
                "{label} stays unsupported on macOS"
            );
        }
        // A right-hand modifier is only a shortcut on its own.
        assert!(chord("ControlRight", "Shift+Right Ctrl", shift)
            .into_setting_for_platform(true)
            .is_err());
    }

    #[test]
    fn mac_shortcut_input_keeps_its_own_lone_key_rules() {
        assert!(lone("ControlRight", "Right Ctrl")
            .into_setting_for_platform(false)
            .is_err());
    }

    #[test]
    fn windows_default_push_to_talk_is_right_ctrl() {
        let windows = DictationSettings::default_for_platform(true);
        assert_eq!(windows.push_to_talk_shortcut.code, "ControlRight");
        assert_eq!(windows.push_to_talk_shortcut.label, "Right Ctrl");
        assert_eq!(windows.toggle_shortcut.code, "KeyT");
        let mac = DictationSettings::default_for_platform(false);
        assert_eq!(mac.push_to_talk_shortcut.code, "KeyD");
    }

    #[test]
    fn windows_saved_lone_shortcuts_survive_loading() {
        for code in ["ControlRight", "ControlLeft", "AltLeft", "F13", "Pause"] {
            let saved = lone(code, code).into_setting_for_platform(true).unwrap();
            let loaded = saved
                .clone()
                .normalized_for_platform(DictationShortcutKind::PushToTalk, true);
            assert_eq!(loaded, saved, "{code}");
        }
        let double_tap = DictationShortcutInput {
            press_count: Some(2),
            ..lone("ShiftLeft", "Left Shift+Left Shift")
        }
        .into_setting_for_platform(true)
        .unwrap();
        assert_eq!(
            double_tap
                .clone()
                .normalized_for_platform(DictationShortcutKind::Toggle, true),
            double_tap
        );
        // A saved chord stays a saved chord: an existing install keeps it.
        let chord = DictationShortcutSetting::control_option_d();
        assert_eq!(
            chord
                .clone()
                .normalized_for_platform(DictationShortcutKind::PushToTalk, true),
            chord
        );
    }

    #[test]
    fn push_to_talk_cancel_discards_the_take() {
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down
            ),
            Some(DictationCommand::StartListening)
        );
        let later = down + PUSH_TO_TALK_MIN_HOLD * 3;
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Cancel,
                DictationShortcutKind::PushToTalk,
                later
            ),
            Some(DictationCommand::DiscardListening)
        );
        // The release that follows does nothing more.
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Up,
                DictationShortcutKind::PushToTalk,
                later
            ),
            None
        );
    }

    #[test]
    fn helper_cancel_event_maps_to_the_cancel_edge() {
        assert_eq!(
            shortcut_key_edge("shortcut_key_cancel"),
            Some(ShortcutKeyEdge::Cancel)
        );
        assert_eq!(
            shortcut_key_edge("shortcut_key_down"),
            Some(ShortcutKeyEdge::Down)
        );
    }

    #[test]
    fn windows_shortcut_input_accepts_supported_chord() {
        let shortcut = DictationShortcutInput {
            code: "F12".to_string(),
            modifiers: DictationShortcutModifiers {
                command: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Win+F12".to_string(),
            press_count: Some(1),
        }
        .into_setting_for_platform(true)
        .expect("Windows should accept a supported modifier plus key chord");

        assert_eq!(shortcut.code, "F12");
        assert_eq!(shortcut.press_count, 1);
    }

    #[test]
    fn shortcut_input_accepts_control_digit() {
        let shortcut = DictationShortcutInput {
            code: "Digit1".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+1".to_string(),
            press_count: Some(1),
        }
        .into_setting()
        .expect("control digit should be accepted");

        assert_eq!(shortcut.key_code, 0x12);
        assert_eq!(shortcut.code, "Digit1");
        assert!(shortcut.modifiers.control);
    }

    #[test]
    fn shortcut_input_accepts_modifier_only_combo() {
        let shortcut = DictationShortcutInput {
            code: "Modifiers".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                option: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+Opt".to_string(),
            press_count: Some(1),
        }
        .into_setting_for_platform(false)
        .expect("modifier-only combo should be accepted");

        assert_eq!(shortcut.key_code, 0);
        assert_eq!(shortcut.code, "Modifiers");
        assert_eq!(shortcut.label, "Ctrl+Opt");
        assert!(shortcut.modifiers.control);
        assert!(shortcut.modifiers.option);
    }

    #[test]
    fn shortcut_input_rejects_single_modifier_only_shortcut() {
        let err = DictationShortcutInput {
            code: "Modifiers".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl".to_string(),
            press_count: Some(1),
        }
        .into_setting()
        .expect_err("single modifier-only shortcut should fail");

        assert_eq!(err.code, "dictation_shortcut_unsupported");
    }

    #[test]
    fn shortcut_input_requires_modifier() {
        let err = DictationShortcutInput {
            code: "KeyT".to_string(),
            modifiers: DictationShortcutModifiers::default(),
            label: "T".to_string(),
            press_count: Some(1),
        }
        .into_setting()
        .expect_err("shortcut without modifiers should fail");

        assert_eq!(err.code, "dictation_shortcut_modifier_required");
    }

    #[test]
    fn shortcut_input_rejects_unsupported_key() {
        let err = DictationShortcutInput {
            code: "NumpadEnter".to_string(),
            modifiers: DictationShortcutModifiers {
                control: true,
                ..DictationShortcutModifiers::default()
            },
            label: "Ctrl+NumpadEnter".to_string(),
            press_count: Some(1),
        }
        .into_setting()
        .expect_err("unsupported key should fail");

        assert_eq!(err.code, "dictation_shortcut_unsupported");
    }

    #[test]
    fn duplicate_validation_rejects_same_key_combo() {
        let settings = DictationSettings {
            push_to_talk_shortcut: DictationShortcutSetting {
                key_code: 0x31,
                code: "Space".to_string(),
                modifiers: DictationShortcutModifiers {
                    control: true,
                    ..DictationShortcutModifiers::default()
                },
                label: "Ctrl+Space".to_string(),
                press_count: 1,
            },
            toggle_shortcut: DictationShortcutSetting {
                key_code: 0x31,
                code: "Space".to_string(),
                modifiers: DictationShortcutModifiers {
                    control: true,
                    ..DictationShortcutModifiers::default()
                },
                label: "Ctrl+Space".to_string(),
                press_count: 1,
            },
            microphone: DictationMicrophoneSetting::default(),
            style: DictationStyle::Standard,
            language: None,
            ..DictationSettings::default()
        };

        assert!(validate_shortcut_update(
            &settings,
            DictationShortcutKind::PushToTalk,
            &settings.push_to_talk_shortcut
        )
        .is_err());
    }

    #[test]
    fn push_to_talk_starts_on_down_and_stops_on_up() {
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        let up = down + PUSH_TO_TALK_MIN_HOLD;

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down
            ),
            Some(DictationCommand::StartListening)
        );
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, up),
            Some(DictationCommand::StopAndPaste)
        );
    }

    #[test]
    fn push_to_talk_graze_discards_instead_of_transcribing() {
        // Recording starts on the down edge now, so an accidental brush of
        // the key produces a real (tiny) recording. Releasing inside the
        // minimum hold must discard it rather than paste transcribed noise.
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        let up = down + PUSH_TO_TALK_MIN_HOLD - Duration::from_millis(1);

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down
            ),
            Some(DictationCommand::StartListening)
        );
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, up),
            Some(DictationCommand::DiscardListening)
        );

        // The graze fully releases the press: the next hold is a fresh start.
        let next_down = up + Duration::from_millis(5);
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                next_down
            ),
            Some(DictationCommand::StartListening)
        );
    }

    #[test]
    fn shortcut_reset_after_cancel_ignores_later_release() {
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        let up = down + PUSH_TO_TALK_MIN_HOLD;

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down
            ),
            Some(DictationCommand::StartListening)
        );

        controller.reset();

        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, up),
            None
        );
    }

    #[test]
    fn confirmed_previous_discard_preserves_a_pending_replacement_shortcut() {
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        let up = down + PUSH_TO_TALK_MIN_HOLD;

        controller.set_helper_listening(true, Some("take-a"));
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down,
            ),
            Some(DictationCommand::StartListening)
        );
        controller.prepare_helper_start("take-b");

        assert_eq!(
            controller.reset_for_helper_discard(Some("take-a"), Some("command"), Some("take-a"),),
            HelperLifecycleDisposition::Apply
        );
        assert!(controller.helper_take_id.is_none());
        assert_eq!(controller.pending_helper_take_id.as_deref(), Some("take-b"));
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, up),
            Some(DictationCommand::StopAndPaste)
        );
    }

    #[test]
    fn pending_start_rejection_preserves_the_confirmed_take() {
        let mut controller = ShortcutActivationController::default();
        controller.set_helper_listening(true, Some("take-a"));
        controller.prepare_helper_start("take-b");

        assert_eq!(
            controller.finish_for_helper_event(Some("take-b"), Some("take-a")),
            HelperLifecycleDisposition::EmitOnly
        );
        assert_eq!(controller.helper_take_id.as_deref(), Some("take-a"));
        assert!(controller.pending_helper_take_id.is_none());
        assert!(controller.helper_is_listening);
    }

    #[test]
    fn stale_terminal_event_cannot_finish_a_confirmed_replacement() {
        let mut controller = ShortcutActivationController::default();
        controller.set_helper_listening(true, Some("take-b"));

        assert_eq!(
            controller.finish_for_helper_event(Some("take-a"), Some("take-a")),
            HelperLifecycleDisposition::Ignore
        );
        assert_eq!(controller.helper_take_id.as_deref(), Some("take-b"));
        assert!(controller.helper_is_listening);
    }

    #[test]
    fn pending_replacement_owns_new_delivery_lifecycle_events() {
        let mut controller = ShortcutActivationController::default();
        controller.set_helper_listening(true, Some("take-a"));
        controller.prepare_helper_start("take-b");

        assert!(!controller.matches_delivery_take("take-a", Some("take-a")));
        assert!(controller.matches_delivery_take("take-b", Some("take-a")));
    }

    #[test]
    fn matching_helper_discard_resets_the_active_shortcut() {
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        let up = down + PUSH_TO_TALK_MIN_HOLD;

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down,
            ),
            Some(DictationCommand::StartListening)
        );
        controller.prepare_helper_start("take-b");
        controller.set_helper_listening(true, Some("take-b"));

        assert_eq!(
            controller.reset_for_helper_discard(Some("take-b"), Some("escape"), Some("take-b"),),
            HelperLifecycleDisposition::Apply
        );
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, up),
            None
        );
    }

    #[test]
    fn reasoned_untagged_discard_does_not_reset_a_replacement_shortcut() {
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        let up = down + PUSH_TO_TALK_MIN_HOLD;

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down,
            ),
            Some(DictationCommand::StartListening)
        );
        controller.prepare_helper_start("take-b");

        assert_eq!(
            controller.reset_for_helper_discard(None, Some("start_cancelled"), Some("take-b"),),
            HelperLifecycleDisposition::Ignore
        );
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, up),
            Some(DictationCommand::StopAndPaste)
        );
    }

    #[test]
    fn reasoned_untagged_discard_does_not_cancel_crash_recovery_work() {
        let idle_echo = serde_json::json!({
            "type": "recording_discarded",
            "payload": { "reason": "not_listening" },
        });
        let tagged = serde_json::json!({
            "type": "recording_discarded",
            "payload": { "takeId": "take-a", "reason": "command" },
        });
        let legacy = serde_json::json!({
            "type": "recording_discarded",
            "payload": {},
        });

        assert_eq!(helper_discard_cancellation_target(&idle_echo), None);
        assert_eq!(
            helper_discard_cancellation_target(&tagged),
            Some(Some("take-a"))
        );
        assert_eq!(helper_discard_cancellation_target(&legacy), Some(None));
    }

    #[test]
    fn legacy_untagged_discard_still_resets_the_shortcut() {
        let mut controller = ShortcutActivationController::default();
        let down = Instant::now();
        let up = down + PUSH_TO_TALK_MIN_HOLD;

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                down,
            ),
            Some(DictationCommand::StartListening)
        );
        assert_eq!(
            controller.reset_for_helper_discard(None, None, Some("take-b")),
            HelperLifecycleDisposition::Apply
        );
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, up),
            None
        );
    }

    #[test]
    fn negotiated_take_protocol_rejects_untagged_take_outcomes() {
        let mut controller = ShortcutActivationController::default();
        controller.set_helper_listening(true, Some("take-a"));

        assert_eq!(
            controller.untagged_take_event_disposition(Some("final_transcript")),
            HelperLifecycleDisposition::Ignore
        );
        assert_eq!(
            controller.untagged_take_event_disposition(Some("error")),
            HelperLifecycleDisposition::EmitOnly
        );
        assert_eq!(
            controller.reset_for_helper_discard(None, None, Some("take-a")),
            HelperLifecycleDisposition::Ignore
        );
        assert_eq!(controller.helper_take_id.as_deref(), Some("take-a"));
    }

    #[test]
    fn toggle_mode_toggles_on_down_and_ignores_up() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::Toggle, now),
            None
        );
        controller.acknowledge_toggle_command();
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + TOGGLE_SHORTCUT_DEBOUNCE
            ),
            Some(DictationCommand::ToggleListening)
        );
    }

    #[test]
    fn rapid_toggle_edges_send_one_command_until_acknowledged() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        let commands = [
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            controller.handle_edge(
                ShortcutKeyEdge::Up,
                DictationShortcutKind::Toggle,
                now + Duration::from_millis(20),
            ),
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + Duration::from_millis(100),
            ),
            controller.handle_edge(
                ShortcutKeyEdge::Up,
                DictationShortcutKind::Toggle,
                now + Duration::from_millis(120),
            ),
        ];

        assert_eq!(
            commands
                .into_iter()
                .flatten()
                .filter(|command| *command == DictationCommand::ToggleListening)
                .count(),
            1
        );
    }

    #[test]
    fn acknowledged_toggle_still_respects_debounce_window() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
        controller.acknowledge_toggle_command();

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + TOGGLE_SHORTCUT_DEBOUNCE - Duration::from_millis(1)
            ),
            None
        );
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + TOGGLE_SHORTCUT_DEBOUNCE
            ),
            Some(DictationCommand::ToggleListening)
        );
    }

    #[test]
    fn unacknowledged_toggle_expires_instead_of_wedging_the_shortcut() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
        // No ack ever arrives (helper died / permission prompt never called
        // back). Within the expiry the toggle stays suppressed...
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + TOGGLE_SHORTCUT_DEBOUNCE
            ),
            None
        );
        // ...and past the expiry the shortcut recovers on its own.
        controller.last_toggle_command_at = Some(now - TOGGLE_ACK_EXPIRY);
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
    }

    #[test]
    fn stuck_finalizing_suppression_expires_instead_of_wedging_the_shortcut() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        controller.helper_finalizing_since = Some(now - FINALIZING_SUPPRESSION_EXPIRY);
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
    }

    #[test]
    fn toggle_start_detection_follows_helper_confirmed_listening_state() {
        let mut controller = ShortcutActivationController::default();

        assert!(controller.command_starts_recording(DictationCommand::ToggleListening));
        controller.set_helper_listening(true, Some("take-1"));
        assert!(!controller.command_starts_recording(DictationCommand::ToggleListening));
        controller.reset();
        assert!(!controller.command_starts_recording(DictationCommand::ToggleListening));
        controller.set_helper_listening(false, None);
        assert!(controller.command_starts_recording(DictationCommand::ToggleListening));
        assert!(controller.command_starts_recording(DictationCommand::StartListening));
        assert!(!controller.command_starts_recording(DictationCommand::StopAndPaste));
    }

    #[test]
    fn push_to_talk_during_helper_finalizing_is_dropped() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        controller.helper_finalizing_since = Some(now);
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now
            ),
            None
        );
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now + FINALIZING_SUPPRESSION_EXPIRY
            ),
            Some(DictationCommand::StartListening)
        );
    }

    #[test]
    fn completed_outcome_immediately_rearms_push_to_talk() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        controller.mark_helper_finalizing();
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now
            ),
            None
        );

        controller.clear_helper_finalizing();
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now + Duration::from_millis(1)
            ),
            Some(DictationCommand::StartListening)
        );
    }

    #[test]
    fn toggle_during_helper_finalizing_is_dropped_without_parity_flip() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
        controller.acknowledge_toggle_command();
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + TOGGLE_SHORTCUT_DEBOUNCE
            ),
            Some(DictationCommand::ToggleListening)
        );
        controller.acknowledge_toggle_command();
        controller.mark_helper_finalizing();

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + TOGGLE_SHORTCUT_DEBOUNCE + Duration::from_secs(1)
            ),
            None
        );
        controller.clear_helper_finalizing();
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::Toggle,
                now + TOGGLE_SHORTCUT_DEBOUNCE + Duration::from_secs(2)
            ),
            Some(DictationCommand::ToggleListening)
        );
    }

    #[test]
    fn toggle_gate_never_drops_push_to_talk_release() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now
            ),
            Some(DictationCommand::StartListening)
        );
        controller.toggle_command_in_flight = true;
        controller.last_toggle_command_at = Some(now);

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Up,
                DictationShortcutKind::PushToTalk,
                now + PUSH_TO_TALK_MIN_HOLD
            ),
            Some(DictationCommand::StopAndPaste)
        );
    }

    #[test]
    fn shortcut_activation_ignores_push_while_toggle_active() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now
            ),
            None
        );
        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Up, DictationShortcutKind::PushToTalk, now),
            None
        );
    }

    #[test]
    fn external_hands_free_stop_allows_push_to_talk_again() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();

        assert_eq!(
            controller.handle_edge(ShortcutKeyEdge::Down, DictationShortcutKind::Toggle, now),
            Some(DictationCommand::ToggleListening)
        );
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now + Duration::from_millis(1)
            ),
            None
        );

        controller.reset();

        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now + Duration::from_millis(2)
            ),
            Some(DictationCommand::StartListening)
        );
    }

    #[test]
    fn direct_helper_listening_commands_reset_shortcut_activation() {
        assert!(helper_command_type_resets_shortcut_activation(
            "stop_and_paste"
        ));
        assert!(helper_command_type_resets_shortcut_activation(
            "discard_recording"
        ));
        assert!(helper_command_type_resets_shortcut_activation(
            "toggle_listening"
        ));
        assert!(!helper_command_type_resets_shortcut_activation(
            "start_shortcut_capture"
        ));
    }

    #[test]
    fn direct_terminal_command_keeps_the_active_take_id_before_reset() {
        let mut controller = ShortcutActivationController::default();
        controller.set_helper_listening(true, Some("take-a"));

        let (starts_recording, take_id) =
            controller.prepare_renderer_command("discard_recording", None, None);

        assert!(!starts_recording);
        assert_eq!(take_id.as_deref(), Some("take-a"));
        assert_eq!(controller.helper_take_id.as_deref(), Some("take-a"));
        assert!(controller.helper_is_listening);
        assert_eq!(
            controller.reset_for_helper_discard(Some("take-a"), Some("command"), Some("take-a"),),
            HelperLifecycleDisposition::Apply
        );
    }

    #[test]
    fn stale_renderer_terminal_command_does_not_reset_the_replacement_take() {
        let mut controller = ShortcutActivationController::default();
        let now = Instant::now();
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Down,
                DictationShortcutKind::PushToTalk,
                now
            ),
            Some(DictationCommand::StartListening)
        );
        controller.prepare_helper_start("take-b");

        let (_, take_id) = controller.prepare_renderer_command(
            "discard_recording",
            Some("take-a".to_string()),
            Some("take-a".to_string()),
        );
        assert_eq!(take_id.as_deref(), Some("take-a"));
        assert!(controller.helper_take_id.is_none());
        assert_eq!(controller.pending_helper_take_id.as_deref(), Some("take-b"));
        assert_eq!(
            controller.handle_edge(
                ShortcutKeyEdge::Up,
                DictationShortcutKind::PushToTalk,
                now + PUSH_TO_TALK_MIN_HOLD,
            ),
            Some(DictationCommand::StopAndPaste)
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn dictation_audio_is_normalized_for_transcription() {
        let directory = tempfile::tempdir().expect("tempdir");
        let input = directory.path().join("captured.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&input, spec).expect("create captured wav");
        for sample in [500_i16, 500, -500, -500] {
            writer.write_sample(sample).expect("write sample");
        }
        writer.finalize().expect("finalize captured wav");

        let prepared = prepare_dictation_audio(&input, "test-utterance").expect("normalize wav");
        let reader = hound::WavReader::open(&prepared).expect("open normalized wav");
        assert_ne!(prepared, input);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.spec().sample_rate, 16_000);
        let _ = fs::remove_file(prepared);
    }

    #[test]
    fn successful_transcription_maps_to_paste_command() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Paste this transcript.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({
                "type": "paste_text",
                "text": "Paste this transcript.",
            })
        );
        assert!(outcome.event.is_none());
        assert_eq!(
            outcome.transcript.as_ref().map(|item| item.text.as_str()),
            Some("Paste this transcript.")
        );
    }

    #[test]
    fn literal_dictation_starting_with_heres_maps_to_paste_command() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Here's the API key to Poncho.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({
                "type": "paste_text",
                "text": "Here's the API key to Poncho.",
            })
        );
        assert!(outcome.event.is_none());
        assert_eq!(
            outcome.transcript.as_ref().map(|item| item.text.as_str()),
            Some("Here's the API key to Poncho.")
        );
    }

    #[test]
    fn summary_like_transcription_discards_with_visible_error() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "User performed a very long dictation session. Instead of producing a full transcription, the app emitted an error.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome.transcript.is_none());
        let event = outcome.event.expect("summary-like text emits an error");
        assert_eq!(event["payload"]["code"], "dictation_response_invalid");
        assert!(!is_silent_transcription_error(&event));
    }

    #[test]
    fn filler_only_transcription_discards_silently() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Ums and uhs.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome.transcript.is_none());
        let event = outcome.event.expect("filler-only capture emits an event");
        assert!(is_silent_transcription_error(&event));
    }

    #[test]
    fn filler_only_transcription_stays_silent_even_with_detected_audio() {
        // Speaking "um" registers real audio, but a filler-only capture must
        // still discard quietly — never the visible audio-without-text error.
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Um, uh.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            Some(DICTATION_AUDIO_ACTIVITY_THRESHOLD),
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome.transcript.is_none());
        let event = outcome.event.expect("filler-only capture emits an event");
        assert!(is_silent_transcription_error(&event));
    }

    #[test]
    fn all_caps_acronym_is_not_treated_as_filler() {
        assert_eq!(
            clean_dictation_fillers("Send the patient to the ER.", DictationStyle::Standard),
            "Send the patient to the ER."
        );
        // The capitalized hesitation form still strips.
        assert_eq!(
            clean_dictation_fillers("Er, no.", DictationStyle::Standard),
            "No."
        );
    }

    #[test]
    fn mixed_filler_transcription_pastes_clean_text() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Um, uh, Send this, please.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({
                "type": "paste_text",
                "text": "Send this, please.",
            })
        );
        assert!(outcome.event.is_none());
        assert_eq!(
            outcome.transcript.as_ref().map(|item| item.text.as_str()),
            Some("Send this, please.")
        );
    }

    #[test]
    fn mid_sentence_fillers_are_removed_without_touching_articles() {
        assert_eq!(
            clean_dictation_fillers(
                "I, uh, need a test and an example.",
                DictationStyle::Standard
            ),
            "I need a test and an example."
        );
    }

    #[test]
    fn leading_filler_removal_recapitalizes_surviving_first_word() {
        assert_eq!(
            clean_dictation_fillers("Um, can you send this email?", DictationStyle::Standard),
            "Can you send this email?"
        );
        assert_eq!(
            clean_dictation_fillers("Hmm, that is interesting.", DictationStyle::Standard),
            "That is interesting."
        );
        // Mid-sentence fillers don't disturb an already-capitalized start.
        assert_eq!(
            clean_dictation_fillers("The um value is um five.", DictationStyle::Standard),
            "The value is five."
        );
        // Unicode-aware: an accented surviving first word still capitalizes.
        assert_eq!(
            clean_dictation_fillers("um, élan is nice.", DictationStyle::Standard),
            "Élan is nice."
        );
    }

    #[test]
    fn mid_sentence_filler_does_not_recapitalize_brand_or_code_token() {
        // A filler removed mid-sentence must leave the original first word's
        // casing alone — only a *leading* filler triggers re-capitalization.
        assert_eq!(
            clean_dictation_fillers("iPhone, um, battery is low.", DictationStyle::Standard),
            "iPhone battery is low."
        );
        assert_eq!(
            clean_dictation_fillers("camelCase, uh, value.", DictationStyle::Standard),
            "camelCase value."
        );
    }

    #[test]
    fn numeric_dictation_after_filler_is_kept() {
        // "um 123456" must paste the number, not discard as filler-only.
        assert_eq!(
            clean_dictation_fillers("um 123456", DictationStyle::Standard),
            "123456"
        );
        assert_eq!(
            clean_dictation_fillers("Call, uh, 5551234.", DictationStyle::Standard),
            "Call 5551234."
        );
    }

    #[test]
    fn line_breaks_and_paragraphs_are_preserved() {
        // Cleanup-prompt paragraph grouping and spoken "new line" must survive.
        assert_eq!(
            clean_dictation_fillers("First line.\nSecond line.", DictationStyle::Standard),
            "First line.\nSecond line."
        );
        assert_eq!(
            clean_dictation_fillers("Paragraph one.\n\nParagraph two.", DictationStyle::Standard),
            "Paragraph one.\n\nParagraph two."
        );
        // Horizontal whitespace still collapses within a line.
        assert_eq!(
            clean_dictation_fillers("Too    many   spaces.", DictationStyle::Standard),
            "Too many spaces."
        );
    }

    #[test]
    fn filler_before_non_alpha_token_keeps_separator() {
        // The space the filler swallowed must be restored even when the next
        // token is a digit or an opening quote, not just an alphabetic word.
        assert_eq!(
            clean_dictation_fillers("I need um 3 things.", DictationStyle::Standard),
            "I need 3 things."
        );
        assert_eq!(
            clean_dictation_fillers("I said, um, \"hello\".", DictationStyle::Standard),
            "I said \"hello\"."
        );
    }

    #[test]
    fn email_app_bundles_map_to_the_email_context() {
        assert!(is_email_app_bundle("com.apple.mail"));
        assert!(is_email_app_bundle("COM.APPLE.MAIL"));
        assert!(is_email_app_bundle("com.microsoft.Outlook"));
        assert!(!is_email_app_bundle("com.google.Chrome"));
        assert!(!is_email_app_bundle(""));
    }

    #[test]
    fn casual_lowercase_style_keeps_stripped_first_word_lowercase() {
        // The backstop must never re-capitalize under CasualLowercase, even
        // when stripping a leading filler exposes a lowercase first word.
        assert_eq!(
            clean_dictation_fillers(
                "um, can you send this email?",
                DictationStyle::CasualLowercase
            ),
            "can you send this email?"
        );
        // Standard and Formal do restore the capital.
        assert_eq!(
            clean_dictation_fillers("um, can you send this email?", DictationStyle::Formal),
            "Can you send this email?"
        );
    }

    #[test]
    fn standalone_filler_sentence_keeps_neighboring_sentences_separated() {
        assert_eq!(
            clean_dictation_fillers("Let me think. Um. Yeah, do it.", DictationStyle::Standard),
            "Let me think. Yeah, do it."
        );
        // "also" stays lowercase on purpose: this deterministic backstop only
        // restores the casing of a *leading* filler's surviving word, not every
        // sentence start. Re-flowing mid-transcript sentence casing is the LLM
        // cleanup pass's job; the backstop just avoids merging words.
        assert_eq!(
            clean_dictation_fillers("That works. Uh, also add tests.", DictationStyle::Standard),
            "That works. also add tests."
        );
        assert_eq!(
            clean_dictation_fillers("Done! Um. Ship it?", DictationStyle::Standard),
            "Done! Ship it?"
        );
    }

    #[test]
    fn hey_clovy_transcription_maps_to_agent_session_event() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Hey, Clovy, summarize the open document.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert_eq!(
            outcome.event,
            Some(serde_json::json!({
                "type": "agent_session_prompt",
                "payload": { "prompt": "summarize the open document." },
            }))
        );
        assert_eq!(
            outcome.transcript.as_ref().map(|item| item.text.as_str()),
            Some("Hey, Clovy, summarize the open document.")
        );
    }

    #[test]
    fn agent_session_prompt_hides_dictation_hud() {
        assert!(matches!(
            dictation_event_visibility(Some("agent_session_prompt")),
            DictationEventVisibility::Hide
        ));
    }

    #[test]
    fn helper_unavailable_hides_dictation_hud() {
        assert!(matches!(
            dictation_event_visibility(Some("helper_unavailable")),
            DictationEventVisibility::Hide
        ));
    }

    #[test]
    fn recording_discarded_hides_dictation_hud() {
        assert!(matches!(
            dictation_event_visibility(Some("recording_discarded")),
            DictationEventVisibility::Hide
        ));
    }

    #[test]
    fn empty_transcription_discards_with_a_visible_no_speech() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "   ".to_string(),
                language: None,
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome.transcript.is_none());
        let event = outcome.event.expect("empty capture emits an event");
        assert_eq!(event["payload"]["code"], "no_speech");
        assert_eq!(event["payload"]["silent"], false);
    }

    #[test]
    fn empty_transcription_with_detected_audio_is_still_no_speech() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "   ".to_string(),
                language: None,
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            Some(DICTATION_AUDIO_ACTIVITY_THRESHOLD),
            DictationStyle::Standard,
        );

        // The level is diagnostic only (ditado-rapido D5).
        let event = outcome.event.expect("empty capture emits an event");
        assert_eq!(event["payload"]["code"], "no_speech");
        assert_eq!(event["payload"]["message"], "No speech detected.");
        assert_eq!(event["payload"]["silent"], false);
    }

    #[test]
    fn hud_snapshot_replays_candidate_and_pinned_target_without_audio_restarting_it() {
        let a = serde_json::json!({"app": "Finder", "bundleIdentifier": "com.apple.finder", "processIdentifier": 1});
        let b = serde_json::json!({"app": "Terminal", "bundleIdentifier": "com.apple.Terminal", "processIdentifier": 2});
        let event = |kind: &str, target: serde_json::Value| {
            serde_json::json!({"type": kind, "payload": {"takeId": "take-1", "target": target}})
                .to_string()
        };
        let listening = event("listening_started", a);
        let focus = event("focus_target", b.clone());
        let current =
            latest_hud_snapshot(Some(&listening), Some("focus_target"), Some(&focus)).unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&current).unwrap();
        assert_eq!(snapshot["type"], "listening_started");
        assert_eq!(snapshot["payload"]["target"], b);
        let finalizing = event("finalizing_transcript", b.clone());
        let current = latest_hud_snapshot(
            Some(&current),
            Some("finalizing_transcript"),
            Some(&finalizing),
        )
        .unwrap();
        let audio = serde_json::json!({"type": "audio_level", "payload": {"takeId": "take-1", "level": "0.2"}}).to_string();
        let current =
            latest_hud_snapshot(Some(&current), Some("audio_level"), Some(&audio)).unwrap();
        let transcript =
            serde_json::json!({"type": "final_transcript", "payload": {"takeId": "take-1"}})
                .to_string();
        let current =
            latest_hud_snapshot(Some(&current), Some("final_transcript"), Some(&transcript))
                .unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&current).unwrap();
        assert_eq!(snapshot["type"], "final_transcript");
        assert_eq!(snapshot["payload"]["target"], b);
        assert!(latest_hud_snapshot(Some(&current), Some("recording_discarded"), None).is_none());
        let next =
            serde_json::json!({"type": "listening_started", "payload": {"takeId": "take-2"}})
                .to_string();
        let current =
            latest_hud_snapshot(Some(&current), Some("listening_started"), Some(&next)).unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&current).unwrap();
        assert!(snapshot["payload"].get("target").is_none());
    }

    #[test]
    fn hud_focus_snapshot_never_changes_another_take_or_processing_destination() {
        let prior = serde_json::json!({"type": "listening_started", "payload": {"takeId": "new", "target": null}}).to_string();
        let stale = serde_json::json!({"type": "focus_target", "payload": {"takeId": "old", "target": {"app": "stale"}}}).to_string();
        let current =
            latest_hud_snapshot(Some(&prior), Some("focus_target"), Some(&stale)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&current).unwrap()["payload"]["target"],
            serde_json::Value::Null
        );
        let prior = prior.replace("listening_started", "finalizing_transcript");
        let focus = stale.replace("old", "new");
        let current =
            latest_hud_snapshot(Some(&prior), Some("focus_target"), Some(&focus)).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&current).unwrap()["payload"]["target"],
            serde_json::Value::Null
        );
        assert!(helper_event_serializes_with_commands(Some("focus_target")));
        assert_eq!(indicator_action_for_event(Some("focus_target")), None);
    }

    #[tokio::test]
    async fn history_uses_recording_ready_target_for_staged_and_normal_writes() {
        let event = serde_json::json!({ "type": "recording_ready", "payload": {
            "path": "/tmp/fixture.m4a", "targetBundleIdentifier": "org.example.first"
        }});
        let recording = recording_ready_info_from_event(&event).unwrap();
        let pinned = history_target(&recording);
        // A later event from another app must not change the pinned target.
        let later = serde_json::json!({ "type": "recording_ready", "payload": {
            "path": "/tmp/fixture.m4a", "targetBundleIdentifier": "org.example.second"
        }});
        assert_ne!(
            history_target(&recording_ready_info_from_event(&later).unwrap()),
            pinned
        );
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::migrations::run_migrations(&pool).await.unwrap();
        let repos = crate::db::repositories::Repositories::new(pool);
        let transcript = TranscriptionProviderResult {
            text: "short".into(),
            language: None,
            provider: "test".into(),
        };
        stage_history_with_repositories(&repos, &transcript, "default", pinned.as_deref())
            .await
            .unwrap()
            .unwrap()
            .commit()
            .await
            .unwrap();
        let transcript = TranscriptionProviderResult {
            text: "normal".into(),
            ..transcript
        };
        persist_history_with_repositories(&repos, &transcript, "default", pinned.as_deref())
            .await
            .unwrap();
        let rows = repos
            .list_dictation_history("default", 10)
            .await
            .unwrap()
            .items;
        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .all(|row| row.app_bundle_id.as_deref() == Some("org.example.first")));
    }

    #[test]
    fn recording_ready_info_carries_the_paste_target_bundle_id() {
        let event = serde_json::json!({
            "type": "recording_ready",
            "payload": {
                "path": "/tmp/clovy-dictation-test.m4a",
                "targetBundleIdentifier": "com.apple.mail",
            }
        });
        let info = recording_ready_info_from_event(&event).expect("info parses");
        assert_eq!(info.target_bundle_id.as_deref(), Some("com.apple.mail"));

        // Older helpers omit the field (or send it empty): no target.
        let legacy = serde_json::json!({
            "type": "recording_ready",
            "payload": { "path": "/tmp/clovy-dictation-test.m4a", "targetBundleIdentifier": "" }
        });
        let info = recording_ready_info_from_event(&legacy).expect("info parses");
        assert_eq!(info.target_bundle_id, None);
    }

    #[test]
    fn recording_ready_info_includes_observed_audio_level() {
        let event = serde_json::json!({
            "type": "recording_ready",
            "payload": {
                "path": "/tmp/clovy-dictation-test.m4a",
                "observedAudioLevel": "0.1732",
            }
        });

        let info = recording_ready_info_from_event(&event).expect("recording info");

        assert_eq!(
            info.audio_path,
            PathBuf::from("/tmp/clovy-dictation-test.m4a")
        );
        assert_eq!(info.observed_audio_level, Some(0.1732));
    }

    #[test]
    fn recording_ready_info_includes_valid_speech_confidence() {
        let event = serde_json::json!({
            "type": "recording_ready",
            "payload": {
                "path": "/tmp/clovy-dictation-test.m4a",
                "speechConfidence": "0.2630",
                "speechAnalysisStatus": "ok",
            }
        });
        let info = recording_ready_info_from_event(&event).expect("recording info");
        assert_eq!(info.speech_confidence, Some(0.263));
        assert_eq!(info.speech_analysis_status, Some(SpeechAnalysisStatus::Ok));
        let log_entry = dictation_event_log_entry("recording_ready", &event);
        assert_eq!(log_entry["speechConfidence"], "0.2630");
        assert_eq!(log_entry["speechAnalysisStatus"], "ok");

        let invalid = serde_json::json!({
            "type": "recording_ready",
            "payload": {
                "path": "/tmp/clovy-dictation-test.m4a",
                "speechConfidence": "1.2",
            }
        });
        let info = recording_ready_info_from_event(&invalid).expect("recording info");
        assert_eq!(info.speech_confidence, None);
    }

    #[test]
    fn observed_audio_level_rejects_values_outside_the_normalized_range() {
        for value in ["-0.1", "1.1", "NaN", "inf"] {
            let event = serde_json::json!({
                "type": "recording_ready",
                "payload": {
                    "path": "/tmp/clovy-dictation-test.m4a",
                    "observedAudioLevel": value,
                }
            });
            let info = recording_ready_info_from_event(&event).expect("recording info");
            assert_eq!(info.observed_audio_level, None, "accepted {value}");
        }

        let event = serde_json::json!({
            "type": "recording_ready",
            "payload": {
                "path": "/tmp/clovy-dictation-test.m4a",
                "observedAudioLevel": "0",
            }
        });
        let info = recording_ready_info_from_event(&event).expect("recording info");
        assert_eq!(info.observed_audio_level, Some(0.0));
    }

    #[test]
    fn recording_ready_accepts_only_bounded_non_empty_composer_request_ids() {
        let event = |request_id: serde_json::Value| {
            serde_json::json!({
                "type": "recording_ready",
                "payload": {
                    "path": "/tmp/clovy-dictation-test.wav",
                    "composerRequestId": request_id,
                }
            })
        };
        let valid = recording_ready_info_from_event(&event(serde_json::json!("request-1")))
            .expect("recording info");
        assert_eq!(valid.composer_request_id.as_deref(), Some("request-1"));
        let empty = recording_ready_info_from_event(&event(serde_json::json!("   ")))
            .expect("recording info");
        assert_eq!(empty.composer_request_id, None);
        let oversized = recording_ready_info_from_event(&event(serde_json::json!("x".repeat(129))))
            .expect("recording info");
        assert_eq!(oversized.composer_request_id, None);
    }

    #[test]
    fn recording_ready_carries_only_a_bounded_non_empty_take_id() {
        let event = |take_id: serde_json::Value| {
            serde_json::json!({
                "type": "recording_ready",
                "payload": {
                    "path": "/tmp/clovy-dictation-test.m4a",
                    "takeId": take_id,
                }
            })
        };

        let valid = recording_ready_info_from_event(&event(serde_json::json!("take-1")))
            .expect("recording info");
        let empty = recording_ready_info_from_event(&event(serde_json::json!("   ")))
            .expect("recording info");
        let oversized = recording_ready_info_from_event(&event(serde_json::json!("x".repeat(129))))
            .expect("recording info");

        assert_eq!(valid.take_id.as_deref(), Some("take-1"));
        assert_eq!(empty.take_id, None);
        assert_eq!(oversized.take_id, None);
    }

    #[test]
    fn cancelling_a_finalized_take_prevents_late_delivery() {
        let state = DictationTakeState::default();
        state.begin("take-1".to_string());
        let DictationFinalizedTake::Start(take) = state.claim_finalized(Some("take-1")) else {
            panic!("first ready event should own processing");
        };

        assert!(state.cancel(Some("take-1")));
        assert!(take.try_begin_delivery().is_none());
    }

    #[test]
    fn concurrent_take_cancellations_have_one_winner() {
        for processing in [false, true] {
            let id = format!("cancel-race-{}", uuid::Uuid::new_v4());
            let gate = &crate::update_gate::UPDATE_GATE;
            gate.begin_dictation_take(&id).unwrap();
            let take = DictationTakeState::default().begin(id.clone());
            if processing {
                assert!(take.try_begin_processing());
            }
            let cancellation = take.inner.cancellation.subscribe();
            let barrier = std::sync::Barrier::new(4);
            let winners = std::thread::scope(|scope| {
                let attempts: Vec<_> = (0..4)
                    .map(|_| {
                        scope.spawn(|| {
                            barrier.wait();
                            take.cancel()
                        })
                    })
                    .collect();
                attempts
                    .into_iter()
                    .map(|attempt| attempt.join().unwrap())
                    .filter(|cancelled| *cancelled)
                    .count()
            });
            assert_eq!(winners, 1);
            assert_eq!(take.state(), DICTATION_TAKE_CANCELLED);
            assert!(*cancellation.borrow());
            assert!(!gate.has_dictation_take(&id));
            assert!(!take.try_begin_processing());
            assert!(take.try_begin_delivery().is_none());
            assert!(!take.cancel());
        }
    }

    #[test]
    fn take_cancellation_races_processing_and_delivery_without_reopening_authority() {
        for delivery in [false, true] {
            for _ in 0..16 {
                let id = format!("transition-race-{}", uuid::Uuid::new_v4());
                let gate = &crate::update_gate::UPDATE_GATE;
                gate.begin_dictation_take(&id).unwrap();
                let take = DictationTakeState::default().begin(id.clone());
                if delivery {
                    assert!(take.try_begin_processing());
                }
                let cancellation = take.inner.cancellation.subscribe();
                let barrier = std::sync::Barrier::new(2);
                let (cancelled, transitioned) = std::thread::scope(|scope| {
                    let cancel = scope.spawn(|| {
                        barrier.wait();
                        take.cancel()
                    });
                    let transition = scope.spawn(|| {
                        barrier.wait();
                        if delivery {
                            take.try_begin_delivery().is_some()
                        } else {
                            take.try_begin_processing()
                        }
                    });
                    (cancel.join().unwrap(), transition.join().unwrap())
                });
                if delivery {
                    assert_ne!(cancelled, transitioned);
                } else {
                    // A Processing winner still leaves the take cancellable.
                    assert!(cancelled);
                }
                assert_eq!(*cancellation.borrow(), cancelled);
                assert_eq!(gate.has_dictation_take(&id), !cancelled);
                assert_eq!(
                    take.state(),
                    if cancelled {
                        DICTATION_TAKE_CANCELLED
                    } else {
                        DICTATION_TAKE_DELIVERING
                    }
                );
                assert!(!take.cancel());
                assert!(!take.try_begin_processing());
                assert!(take.try_begin_delivery().is_none());
                if !cancelled {
                    // Only this synthetic paste still owns a gate entry.
                    gate.end_dictation_take(&id);
                }
            }
        }
    }

    #[test]
    fn cancellation_refuses_delivering_terminal_and_unknown_take_states() {
        for phase in [
            DICTATION_TAKE_DELIVERING,
            DICTATION_TAKE_CANCELLED,
            4,
            u8::MAX,
        ] {
            let id = format!("cancel-refusal-{}", uuid::Uuid::new_v4());
            let gate = &crate::update_gate::UPDATE_GATE;
            gate.begin_dictation_take(&id).unwrap();
            let take = DictationTakeState::default().begin(id.clone());
            take.inner.state.store(phase, Ordering::SeqCst);
            let cancellation = take.inner.cancellation.subscribe();
            assert!(!take.cancel());
            assert_eq!(take.state(), phase);
            assert!(!*cancellation.borrow());
            assert!(gate.has_dictation_take(&id));
            gate.end_dictation_take(&id);
        }
    }

    #[test]
    fn a_late_ready_event_recovers_the_terminal_cancelled_take() {
        let state = DictationTakeState::default();
        let cancelled = state.begin("take-1".to_string());
        assert!(state.cancel(Some("take-1")));

        assert!(matches!(
            state.claim_finalized(Some("take-1")),
            DictationFinalizedTake::Cancelled
        ));
        assert!(cancelled.try_begin_delivery().is_none());
    }

    #[test]
    fn starting_a_replacement_take_retires_the_terminal_take() {
        let state = DictationTakeState::default();
        let cancelled = state.begin("take-1".to_string());
        assert!(state.cancel(Some("take-1")));

        let replacement = state.begin("take-2".to_string());
        let DictationFinalizedTake::Start(finalized) = state.claim_finalized(Some("take-2")) else {
            panic!("replacement take should own processing");
        };

        assert!(!finalized.same_take(&cancelled));
        assert!(finalized.same_take(&replacement));
        assert!(finalized.try_begin_delivery().is_some());
    }

    #[test]
    fn duplicate_ready_event_cannot_start_a_second_pipeline() {
        let state = DictationTakeState::default();
        state.begin("take-1".to_string());

        assert!(matches!(
            state.claim_finalized(Some("take-1")),
            DictationFinalizedTake::Start(_)
        ));
        assert!(matches!(
            state.claim_finalized(Some("take-1")),
            DictationFinalizedTake::Duplicate
        ));
    }

    #[test]
    fn replacement_start_waits_for_a_terminal_control_write() {
        use std::sync::mpsc;

        let state = Arc::new(DictationTakeState::default());
        state.begin("take-a".to_string());
        let (terminal_entered_tx, terminal_entered_rx) = mpsc::channel();
        let (release_terminal_tx, release_terminal_rx) = mpsc::channel();
        let terminal_state = Arc::clone(&state);
        let terminal = thread::spawn(move || {
            terminal_state
                .run_new_helper_command(|_| {
                    let take_id = terminal_state.current_take_id();
                    terminal_entered_tx
                        .send(())
                        .expect("test should observe the terminal control");
                    release_terminal_rx
                        .recv()
                        .expect("test should release the terminal control");
                    take_id
                })
                .1
        });
        terminal_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("terminal control should acquire the command generation lock");

        let (start_entered_tx, start_entered_rx) = mpsc::channel();
        let replacement_state = Arc::clone(&state);
        let replacement = thread::spawn(move || {
            replacement_state.run_new_helper_command(|_| {
                start_entered_tx
                    .send(())
                    .expect("test should observe the replacement start");
            });
        });
        assert!(matches!(
            start_entered_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_terminal_tx
            .send(())
            .expect("test should release the terminal control");
        assert_eq!(
            terminal
                .join()
                .expect("terminal control thread should finish")
                .as_deref(),
            Some("take-a")
        );
        start_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("replacement start should run after the terminal write");
        replacement
            .join()
            .expect("replacement start thread should finish");
    }

    #[test]
    fn replacement_start_waits_for_a_correlated_helper_lifecycle_event() {
        use std::sync::mpsc;

        let state = Arc::new(DictationTakeState::default());
        let (event_entered_tx, event_entered_rx) = mpsc::channel();
        let (release_event_tx, release_event_rx) = mpsc::channel();
        let event_state = Arc::clone(&state);
        let event = thread::spawn(move || {
            let _guard = event_state.lock_helper_commands();
            event_entered_tx
                .send(())
                .expect("test should observe the helper lifecycle event");
            release_event_rx
                .recv()
                .expect("test should release the helper lifecycle event");
        });
        event_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("helper lifecycle event should acquire the command lock");

        let (start_entered_tx, start_entered_rx) = mpsc::channel();
        let replacement_state = Arc::clone(&state);
        let replacement = thread::spawn(move || {
            replacement_state.run_new_helper_command(|_| {
                start_entered_tx
                    .send(())
                    .expect("test should observe the replacement start");
            });
        });
        assert!(matches!(
            start_entered_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));

        release_event_tx
            .send(())
            .expect("test should release the helper lifecycle event");
        event
            .join()
            .expect("helper lifecycle event thread should finish");
        start_entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("replacement start should run after the lifecycle event");
        replacement
            .join()
            .expect("replacement start thread should finish");
    }

    #[tokio::test]
    async fn cancellation_interrupts_an_in_flight_dictation_step() {
        let state = DictationTakeState::default();
        state.begin("take-1".to_string());
        let DictationFinalizedTake::Start(take) = state.claim_finalized(Some("take-1")) else {
            panic!("first ready event should own processing");
        };
        let pending = async {
            std::future::pending::<()>().await;
            "transcribed"
        };
        let cancellation = take.clone();
        let in_flight = take.clone();

        let result = tokio::time::timeout(Duration::from_secs(1), async move {
            tokio::spawn(async move {
                tokio::task::yield_now().await;
                cancellation.cancel();
            });
            run_dictation_take_step(&in_flight, pending).await
        })
        .await
        .expect("cancellation should interrupt the pending request");

        assert!(result.is_none());
        assert!(take.try_begin_delivery().is_none());
    }

    #[test]
    fn composer_registration_waits_for_its_confirmed_recording_start() {
        let state = HelperState::default();
        *state.composer_registration.lock().expect("registration") =
            Some(ComposerDeliveryRegistration {
                request_id: "request-1".to_string(),
                window_handle: 42,
            });
        let event = serde_json::json!({
            "type": "listening_started",
            "payload": { "composerRequestId": "request-1" },
        });

        // A toggle that stopped an existing external recording emits a
        // terminal event, not a new listening_started acknowledgement. Keep
        // the focused composer's registration armed for its next shortcut.
        consume_started_composer_registration(&state, Some("finalizing_transcript"), &event);
        assert!(state
            .composer_registration
            .lock()
            .expect("registration")
            .is_some());

        consume_started_composer_registration(
            &state,
            Some("listening_started"),
            &serde_json::json!({
                "type": "listening_started",
                "payload": { "composerRequestId": "newer-request" },
            }),
        );
        assert!(state
            .composer_registration
            .lock()
            .expect("registration")
            .is_some());

        consume_started_composer_registration(&state, Some("listening_started"), &event);
        assert!(state
            .composer_registration
            .lock()
            .expect("registration")
            .is_none());
    }

    #[test]
    fn composer_busy_reset_is_correlated_to_the_shortcut_request() {
        let mut controller = ShortcutActivationController {
            active_mode: Some(DictationShortcutKind::Toggle),
            pending_composer_request_id: Some("request-1".to_string()),
            ..ShortcutActivationController::default()
        };

        assert!(!controller.reset_for_rejected_composer_request("stale-request"));
        assert_eq!(controller.active_mode, Some(DictationShortcutKind::Toggle));
        assert!(controller.reset_for_rejected_composer_request("request-1"));
        assert_eq!(controller.active_mode, None);
        assert_eq!(controller.pending_composer_request_id, None);
    }

    #[test]
    fn composer_start_ack_clears_only_the_matching_shortcut_request() {
        let mut controller = ShortcutActivationController {
            pending_composer_request_id: Some("request-1".to_string()),
            ..ShortcutActivationController::default()
        };

        controller.clear_composer_request("stale-request");
        assert_eq!(
            controller.pending_composer_request_id.as_deref(),
            Some("request-1")
        );
        controller.clear_composer_request("request-1");
        assert_eq!(controller.pending_composer_request_id, None);
    }

    #[test]
    fn low_speech_evidence_requires_both_low_audio_signals() {
        let info =
            |speech_confidence, speech_analysis_status, observed_audio_level| RecordingReadyInfo {
                audio_seconds: None,
                audio_path: PathBuf::from("/tmp/clovy-dictation-test.m4a"),
                take_id: None,
                observed_audio_level,
                speech_confidence,
                speech_analysis_status,
                target_bundle_id: None,
                composer_request_id: None,
            };

        assert!(recording_has_low_speech_evidence(&info(
            Some(0.263),
            Some(SpeechAnalysisStatus::Ok),
            Some(0.0305)
        )));
        assert!(!recording_has_low_speech_evidence(&info(
            Some(DICTATION_SPEECH_CONFIDENCE_THRESHOLD),
            Some(SpeechAnalysisStatus::Ok),
            Some(0.0305)
        )));
        assert!(!recording_has_low_speech_evidence(&info(
            Some(0.263),
            Some(SpeechAnalysisStatus::Ok),
            Some(DICTATION_AUDIO_ACTIVITY_THRESHOLD)
        )));
        assert!(!recording_has_low_speech_evidence(&info(
            None,
            None,
            Some(0.0305)
        )));
        assert!(!recording_has_low_speech_evidence(&info(
            Some(0.263),
            Some(SpeechAnalysisStatus::Ok),
            None
        )));
    }

    #[test]
    fn short_capture_is_quarantined_regardless_of_level_but_analysis_failure_fails_open() {
        let info = |status, speech_confidence| RecordingReadyInfo {
            audio_seconds: None,
            audio_path: PathBuf::from("/tmp/clovy-dictation-test.m4a"),
            take_id: None,
            observed_audio_level: Some(0.0305),
            speech_confidence,
            speech_analysis_status: Some(status),
            target_bundle_id: None,
            composer_request_id: None,
        };

        assert!(recording_has_low_speech_evidence(&info(
            SpeechAnalysisStatus::NoCompleteWindow,
            None,
        )));
        let loud_short_capture = RecordingReadyInfo {
            audio_seconds: None,
            audio_path: PathBuf::from("/tmp/clovy-dictation-test.m4a"),
            take_id: None,
            observed_audio_level: Some(DICTATION_AUDIO_ACTIVITY_THRESHOLD),
            speech_confidence: None,
            speech_analysis_status: Some(SpeechAnalysisStatus::NoCompleteWindow),
            target_bundle_id: None,
            composer_request_id: None,
        };
        assert!(recording_has_low_speech_evidence(&loud_short_capture));
        let unmetered_short_capture = RecordingReadyInfo {
            observed_audio_level: None,
            ..loud_short_capture
        };
        assert!(recording_has_low_speech_evidence(&unmetered_short_capture));
        assert!(!recording_has_low_speech_evidence(&info(
            SpeechAnalysisStatus::Unavailable,
            Some(0.0),
        )));
    }

    #[test]
    fn low_speech_evidence_dismisses_silently_and_keeps_history_item() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Oh, yeah.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            Some(0.0305),
            DictationStyle::Standard,
        );

        let outcome = quarantine_low_speech_evidence_outcome(outcome, true);

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome
            .event
            .as_ref()
            .is_some_and(is_silent_transcription_error));
        assert_eq!(
            outcome.transcript.as_ref().map(|item| item.text.as_str()),
            Some("Oh, yeah.")
        );
    }

    #[test]
    fn greeting_is_plain_dictation_and_low_evidence_blocks_paste() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "Hey Clovy open settings.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            Some(0.0305),
            DictationStyle::Standard,
        );
        assert!(outcome.event.is_none());
        assert_eq!(
            outcome.helper_command,
            serde_json::json!({"type":"paste_text","text":"Hey Clovy open settings."})
        );

        let outcome = quarantine_low_speech_evidence_outcome(outcome, true);

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome
            .event
            .as_ref()
            .is_some_and(is_silent_transcription_error));
        assert!(outcome.transcript.is_some());
    }

    #[test]
    fn low_speech_evidence_preserves_summary_like_transcription_in_history() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "User report summary: the user said hello.".to_string(),
                language: Some("en".to_string()),
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            Some(0.0305),
            DictationStyle::Standard,
        );
        assert!(outcome.transcript.is_none());
        assert!(outcome.quarantine_transcript.is_some());

        let outcome = quarantine_low_speech_evidence_outcome(outcome, true);

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome
            .event
            .as_ref()
            .is_some_and(is_silent_transcription_error));
        assert_eq!(
            outcome.transcript.as_ref().map(|item| item.text.as_str()),
            Some("User report summary: the user said hello.")
        );
        assert!(outcome.quarantine_transcript.is_none());
    }

    #[test]
    fn low_speech_evidence_keeps_empty_transcription_silent() {
        let outcome = outcome_from_transcription_result(
            Ok(TranscriptionProviderResult {
                text: "   ".to_string(),
                language: None,
                provider: crate::providers::PROVIDER_GROQ.to_string(),
            }),
            Some(0.0305),
            DictationStyle::Standard,
        );

        let outcome = quarantine_low_speech_evidence_outcome(outcome, true);

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert!(outcome.transcript.is_none());
        assert!(outcome
            .event
            .as_ref()
            .is_some_and(is_silent_transcription_error));
    }

    #[test]
    fn dictation_text_empty_error_is_silent() {
        let event = app_error_event(AppError::new("june_request_failed", "dictation_text_empty"));
        assert!(is_silent_transcription_error(&event));
    }

    #[test]
    fn accessibility_permission_missing_error_is_visible() {
        // The dictation helper emits this when Accessibility trust is missing,
        // so the synthetic Cmd+V paste can't fire. The transcript is left on
        // the clipboard and the user must be told: it must render as a real
        // HUD error, never be swallowed as a "nothing recorded" silent case.
        let mut event = serde_json::json!({
            "type": "error",
            "payload": {
                "code": "accessibility_permission_missing",
                "message": "Clovy couldn't paste automatically. Your transcript is on the clipboard, so you can paste it with Cmd+V.",
            }
        });
        assert!(!is_silent_transcription_error(&event));
        annotate_silent_error(&mut event);
        assert_eq!(event["payload"]["silent"], false);
    }

    #[test]
    fn recovery_clipboard_error_is_visible() {
        let mut event = serde_json::json!({
            "type": "error",
            "payload": {
                "code": "dictation_recovery_clipboard",
                "message": "Could not save this dictation. Use Cmd+V to keep the transcript.",
            }
        });
        assert!(!is_silent_transcription_error(&event));
        annotate_silent_error(&mut event);
        assert_eq!(event["payload"]["silent"], false);
    }

    #[test]
    fn paste_target_unavailable_error_is_visible() {
        // The dictation helper emits this when the app that was frontmost as
        // the recording stopped has since quit, so there is nowhere safe to
        // send the synthetic Cmd+V. Like the Accessibility case, the
        // transcript is left on the clipboard and the user must be told.
        let mut event = serde_json::json!({
            "type": "error",
            "payload": {
                "code": "paste_target_unavailable",
                "message": "Clovy couldn't paste automatically. Your transcript is on the clipboard, so you can paste it with Cmd+V.",
            }
        });
        assert!(!is_silent_transcription_error(&event));
        annotate_silent_error(&mut event);
        assert_eq!(event["payload"]["silent"], false);
    }

    #[test]
    fn dictation_event_log_redacts_transcript_text() {
        let event = serde_json::json!({
            "type": "final_transcript",
            "payload": {
                "text": "Sensitive dictated sentence."
            }
        });

        let entry = dictation_event_log_entry("final_transcript", &event);

        assert_eq!(entry["type"], "final_transcript");
        assert_eq!(entry["textChars"], 28);
        assert!(entry.to_string().contains("textChars"));
        assert!(!entry.to_string().contains("Sensitive dictated sentence"));
    }

    fn notepad(hwnd: i64, pid: u32) -> Option<(String, String)> {
        (hwnd == 5678 && pid == 1234)
            .then(|| ("Bloco de Notas".to_string(), "notepad.exe".to_string()))
    }

    #[test]
    fn windows_start_and_paste_events_name_the_target_app() {
        for event_type in ["listening_started", "paste_target"] {
            let mut event = serde_json::json!({
                "type": event_type,
                "payload": {
                    "takeId": "take-1",
                    "targetProcessId": 1234,
                    "targetWindowHandle": 5678,
                    "targetWindowTitle": "notas.txt - Bloco de Notas",
                }
            });
            complete_windows_target(&mut event, notepad);
            assert_eq!(
                event["payload"]["target"],
                serde_json::json!({
                    "app": "Bloco de Notas",
                    "bundleIdentifier": "notepad.exe",
                    "processIdentifier": 1234,
                }),
                "{event_type}"
            );
        }
    }

    /// Started in app A, stopped in app B: the helper pins B at the stop and
    /// says so in `finalizing_transcript`, which pins the indicator.
    #[test]
    fn the_windows_stop_names_the_app_the_text_goes_to() {
        let mut event = serde_json::json!({
            "type": "finalizing_transcript",
            "payload": {
                "takeId": "take-1",
                "targetProcessId": 1234,
                "targetWindowHandle": 5678,
            }
        });
        complete_windows_target(&mut event, notepad);
        assert_eq!(event["payload"]["target"]["app"], "Bloco de Notas");
        assert_eq!(
            event["payload"]["target"]["bundleIdentifier"],
            "notepad.exe"
        );
    }

    #[test]
    fn a_windows_target_is_left_alone_when_unknown_or_already_named() {
        let mut unknown = serde_json::json!({
            "type": "listening_started",
            "payload": { "targetProcessId": 1, "targetWindowHandle": 2 }
        });
        complete_windows_target(&mut unknown, notepad);
        assert!(unknown["payload"].get("target").is_none());

        let named = serde_json::json!({"app": "Terminal", "bundleIdentifier": "wt.exe"});
        let mut already = serde_json::json!({
            "type": "paste_target",
            "payload": { "targetProcessId": 1234, "targetWindowHandle": 5678, "target": named }
        });
        complete_windows_target(&mut already, notepad);
        assert_eq!(already["payload"]["target"]["app"], "Terminal");

        let mut other = serde_json::json!({
            "type": "recording_ready",
            "payload": { "targetProcessId": 1234, "targetWindowHandle": 5678 }
        });
        complete_windows_target(&mut other, notepad);
        assert!(other["payload"].get("target").is_none());
    }

    #[test]
    fn dictation_event_log_keeps_trigger_diagnostics_without_typed_content() {
        let event = serde_json::json!({
            "type": "shortcut_trigger_diagnostic",
            "payload": {
                "kind": "push_to_talk",
                "trigger": {
                    "key": "ControlLeft",
                    "edge": "up",
                    "injected": false,
                    "alone": true,
                    "heldMs": 240,
                    "decision": "up",
                    "action": "push_end",
                    "reason": "clean",
                    "typed": "secret"
                }
            }
        });

        let entry = dictation_event_log_entry("shortcut_trigger_diagnostic", &event);

        assert_eq!(entry["kind"], "push_to_talk");
        assert_eq!(entry["trigger"]["key"], "ControlLeft");
        assert_eq!(entry["trigger"]["heldMs"], 240);
        assert_eq!(entry["trigger"]["action"], "push_end");
        assert_eq!(entry["trigger"]["injected"], false);
        assert!(!entry.to_string().contains("secret"));
    }

    #[test]
    fn a_shortcut_edge_logs_what_the_app_decided_and_why() {
        let short = shortcut_edge_log_event(
            DictationShortcutKind::PushToTalk,
            ShortcutKeyEdge::Up,
            Some(DictationCommand::DiscardListening),
            Some(Duration::from_millis(90)),
        );
        let entry = dictation_event_log_entry("shortcut_edge", &short);
        assert_eq!(entry["kind"], "push_to_talk");
        assert_eq!(entry["trigger"]["edge"], "up");
        assert_eq!(entry["trigger"]["action"], "discard");
        assert_eq!(entry["trigger"]["reason"], "short_hold");
        assert_eq!(entry["trigger"]["heldMs"], 90);

        let ignored = shortcut_edge_log_event(
            DictationShortcutKind::PushToTalk,
            ShortcutKeyEdge::Down,
            None,
            None,
        );
        let entry = dictation_event_log_entry("shortcut_edge", &ignored);
        assert_eq!(entry["trigger"]["action"], "none");
    }

    #[test]
    fn dictation_event_log_preserves_unconfirmed_input_submission_details() {
        let event = serde_json::json!({
            "type": "paste_completed",
            "payload": {
                "inputSubmitted": true,
                "deliveryConfirmed": false,
                "eventsSubmitted": 4
            }
        });

        let entry = dictation_event_log_entry("paste_completed", &event);

        assert_eq!(entry["inputSubmitted"], true);
        assert_eq!(entry["deliveryConfirmed"], false);
        assert_eq!(entry["eventsSubmitted"], 4);
    }

    #[test]
    fn dictation_event_log_records_silent_error_code() {
        let mut event = serde_json::json!({
            "type": "error",
            "payload": {
                "code": "missing_recording",
                "message": "No recorded audio was available to transcribe."
            }
        });
        annotate_silent_error(&mut event);

        let entry = dictation_event_log_entry("error", &event);

        assert_eq!(entry["code"], "missing_recording");
        assert_eq!(entry["silent"], true);
        assert_eq!(
            entry["message"],
            "No recorded audio was available to transcribe."
        );
    }

    #[test]
    fn failed_transcription_maps_to_discard_and_error() {
        let outcome = outcome_from_transcription_result(
            Err(AppError::new(
                "transcription_failed",
                "The provider failed.",
            )),
            None,
            DictationStyle::Standard,
        );

        assert_eq!(
            outcome.helper_command,
            serde_json::json!({ "type": "discard_recording" })
        );
        assert_eq!(
            outcome.event,
            Some(serde_json::json!({
                "type": "error",
                "payload": {
                    "code": "transcription_failed",
                    "message": "The provider failed.",
                },
            }))
        );
        assert!(outcome.transcript.is_none());
    }

    #[test]
    fn no_speech_error_with_detected_audio_is_visible_no_speech() {
        let outcome = outcome_from_transcription_result(
            Err(AppError::new("june_request_failed", "no_speech")),
            Some(0.2),
            DictationStyle::Standard,
        );

        let event = outcome.event.expect("no speech emits an event");
        assert_eq!(event["payload"]["code"], "no_speech");
        assert_eq!(event["payload"]["silent"], false);
    }

    #[test]
    fn dictation_rejects_unsupported_provider() {
        let err = dictation_transcription_provider("mock".to_string())
            .expect_err("dictation should only accept supported transcripts");

        assert_eq!(err.code, "dictation_provider_not_configured");
    }

    #[test]
    fn parakeet_review_admission_selection_survives_helper_acknowledgement() {
        for (id, missing_started) in [
            ("admitted-local", false),
            ("admitted-local", true),
            (UNTAGGED_DICTATION_TAKE, false),
        ] {
            let state = DictationTakeState::default();
            let mut settings = crate::providers::default_settings_for_tests();
            settings.transcription_provider = "local".into();
            settings.transcription_model = "parakeet-ultra".into();
            state.remember_admitted_settings(id, settings);
            // The helper acknowledges later, while the current selection is Groq.
            let take = if missing_started {
                match state.claim_finalized(Some("admitted-local")) {
                    DictationFinalizedTake::Start(take) => take,
                    _ => panic!("admitted take must be claimed"),
                }
            } else {
                state.begin("admitted-local".into())
            };
            assert_eq!(
                take.inner.transcription_settings.transcription_provider,
                "local"
            );
            assert_eq!(
                take.inner.transcription_settings.transcription_model,
                "parakeet-ultra"
            );
            let backend = crate::providers::transcription::effective_transcription_backend(
                &take.inner.transcription_settings,
                Some("/fixture/model".into()),
                true,
                || panic!("Local admission must not consult Groq credentials"),
            )
            .unwrap();
            assert!(matches!(
                backend,
                crate::providers::transcription::TranscriptionBackend::Local { .. }
            ));
            assert!(state.admitted_settings.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn parakeet_review_admission_does_not_replace_current_take_before_acknowledgement() {
        let state = DictationTakeState::default();
        let current = state.begin("current".into());
        let mut local = crate::providers::default_settings_for_tests();
        local.transcription_provider = "local".into();
        local.transcription_model = "parakeet-ultra".into();
        state.remember_admitted_settings("replacement", local.clone());
        assert_eq!(state.current_take_id().as_deref(), Some("current"));
        assert_eq!(current.state(), DICTATION_TAKE_CAPTURING);
        // A rejected send forgets only its admission, without cancelling the running take.
        state.remember_admitted_settings("unsent", local);
        state.forget_admitted_settings(Some("unsent"));
        assert_eq!(state.admitted_settings.lock().unwrap().len(), 1);
        assert_eq!(current.state(), DICTATION_TAKE_CAPTURING);
        let replacement = state.begin("replacement".into());
        assert_eq!(current.state(), DICTATION_TAKE_CANCELLED);
        assert_eq!(
            replacement
                .inner
                .transcription_settings
                .transcription_provider,
            "local"
        );
        assert!(state.admitted_settings.lock().unwrap().is_empty());
        state.remember_admitted_settings("pending", crate::providers::default_settings_for_tests());
        state.forget_admitted_settings(None);
        assert!(state.admitted_settings.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn parakeet_review_dictation_keeps_its_backend_across_an_await() {
        use crate::providers::local_transcriber::{
            NetworkSentinel, LOCAL_TRANSCRIBER_FAILED_MESSAGE,
        };
        use crate::providers::{
            local_transcriber::{FakeLocalTranscriber, TEST_LOCAL},
            transcription::{TranscriptionBackend, TEST_BACKEND},
        };
        let sentinel = NetworkSentinel::start().await;
        for fail in [false, true] {
            let fake = Arc::new(FakeLocalTranscriber {
                fail,
                ..Default::default()
            });
            crate::providers::TEST_ORIGIN
                .scope(
                    sentinel.origin.clone(),
                    TEST_BACKEND.scope(
                        TranscriptionBackend::Local {
                            model_dir: "/fixture/model".into(),
                        },
                        async {
                            let transcriber = ServiceDictationTranscriber {
                                backend: crate::providers::transcription::current_backend()
                                    .unwrap(),
                            };
                            tokio::task::yield_now().await;
                            TEST_BACKEND
                                .scope(
                                    TranscriptionBackend::Groq {
                                        key: "fixture-key".into(),
                                        model: "whisper-large-v3".into(),
                                    },
                                    TEST_LOCAL.scope(fake.clone(), async {
                                        let result = transcriber
                                            .transcribe(DictateTranscribeRequest {
                                                audio_path: "/fixture/take.m4a".into(),
                                                language: Some("pt".into()),
                                                context: None,
                                                session_id: "session".into(),
                                                utterance_id: "take".into(),
                                            })
                                            .await;
                                        if fail {
                                            // D4 ends the helper on a switch. Its failure must
                                            // remain actionable in the HUD, never become Groq.
                                            let outcome = outcome_from_raw_transcription_result(
                                                result,
                                                Some(0.2),
                                            );
                                            assert!(outcome.transcript.is_none());
                                            assert_eq!(
                                                outcome.helper_command,
                                                serde_json::json!({"type":"discard_recording"})
                                            );
                                            let event = outcome
                                                .event
                                                .expect("Local failure must be visible");
                                            assert_eq!(
                                                event["payload"]["code"],
                                                "local_transcriber_failed"
                                            );
                                            assert_eq!(
                                                event["payload"]["message"],
                                                LOCAL_TRANSCRIBER_FAILED_MESSAGE
                                            );
                                        } else {
                                            let transcript = result.expect(
                                                "the captured Local backend must be retained",
                                            );
                                            assert_eq!(transcript.provider, "local");
                                            assert_eq!(transcript.text, "Olá mundo");
                                        }
                                        assert_eq!(fake.calls.lock().unwrap().len(), 1);
                                    }),
                                )
                                .await;
                        },
                    ),
                )
                .await;
        }
        sentinel.assert_unused();
    }

    #[tokio::test]
    async fn local_dictation_passes_m4a_and_language_without_groq_requests() {
        use crate::providers::{
            local_transcriber::{FakeLocalTranscriber, NetworkSentinel, TEST_LOCAL},
            transcription::{TranscriptionBackend, TEST_BACKEND},
            TEST_ORIGIN,
        };
        let sentinel = NetworkSentinel::start().await;
        for fail in [false, true] {
            let fake = Arc::new(FakeLocalTranscriber {
                fail,
                ..Default::default()
            });
            TEST_ORIGIN
                .scope(
                    sentinel.origin.clone(),
                    TEST_BACKEND.scope(
                        TranscriptionBackend::Local {
                            model_dir: "/fixture/model".into(),
                        },
                        TEST_LOCAL.scope(fake.clone(), async {
                            let result = ServiceDictationTranscriber {
                                backend: crate::providers::transcription::current_backend()
                                    .unwrap(),
                            }
                            .transcribe(DictateTranscribeRequest {
                                audio_path: "/fixture/take.m4a".into(),
                                language: Some("pt".into()),
                                context: None,
                                session_id: "session".into(),
                                utterance_id: "utterance".into(),
                            })
                            .await;
                            if fail {
                                assert_eq!(result.unwrap_err().code, "local_transcriber_failed");
                            } else {
                                assert_eq!(result.unwrap().text, "Olá mundo");
                            }
                            assert_eq!(
                                *fake.calls.lock().unwrap(),
                                vec![(PathBuf::from("/fixture/take.m4a"), Some("pt".into()))]
                            );
                        }),
                    ),
                )
                .await;
        }
        sentinel.assert_unused();
    }

    #[test]
    fn dictation_local_without_model_reports_install_action() {
        let error = dictation_transcription_provider("local".into()).unwrap_err();
        assert_eq!(
            error.code,
            if crate::providers::local_transcription_supported() {
                "local_model_not_installed"
            } else {
                "local_transcription_unsupported"
            }
        );
    }

    #[test]
    fn dictation_accepts_groq_provider() {
        assert_eq!(
            dictation_transcription_provider(crate::providers::PROVIDER_GROQ.to_string())
                .expect("groq should be accepted"),
            crate::providers::PROVIDER_GROQ
        );
    }

    #[test]
    fn detects_instruction_responses_from_cleanup() {
        assert!(looks_like_instruction_response(
            "Sure, here is the rewritten text: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the normalized transcript: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected transcript: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here’s the corrected transcript: Hello."
        ));
        assert!(looks_like_instruction_response("Here you go: Hello."));
        assert!(looks_like_instruction_response(
            "Here is the transcript: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here are the cleaned notes: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here's the cleaned-up version: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected transcript. Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here are the cleaned notes - Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected text\nHello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected transcript, Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected, punctuated transcript: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the cleaned and punctuated transcript. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here you go, Hello. World."
        ));
        assert!(looks_like_instruction_response(
            "Here's the cleaned-up version—Hello."
        ));
        assert!(looks_like_instruction_response("Here it is: Hello."));
        assert!(looks_like_instruction_response("Here you are: Hello."));
        assert!(looks_like_instruction_response(
            "Here is the cleaned dictation: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the result: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the cleaned content: Hello."
        ));
        assert!(looks_like_instruction_response(
            "Here is the cleaned output: Hello."
        ));
        assert!(looks_like_instruction_response("Here is the transcript."));
        assert!(looks_like_instruction_response("Here you go."));
        assert!(looks_like_instruction_response("Here you go!"));
        assert!(looks_like_instruction_response("Here is the transcript?"));
        assert!(looks_like_instruction_response(
            "Here's the cleaned-up message: Hello."
        ));
        assert!(looks_like_instruction_response("Here is the email: Hello."));
        assert!(!looks_like_instruction_response(
            "Here's your text message from John."
        ));
        assert!(!looks_like_instruction_response(
            "Here's your text message from John. Call him back."
        ));
        assert!(!looks_like_instruction_response(
            "Here's your text message from John, call him back."
        ));
        assert!(!looks_like_instruction_response(
            "Here's my email: jane@example.com"
        ));
        assert!(!looks_like_instruction_response(
            "Here's the text message from John: call him back"
        ));
        assert!(!looks_like_instruction_response(
            "Here's the result of the game."
        ));
        assert!(!looks_like_instruction_response(
            "Here's my notes on the trip."
        ));
        assert!(looks_like_instruction_response(
            "Here is what I heard: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is the cleaned transcript below:\nSend it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is what I have:\nSend it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's what I got: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here, I've cleaned it up: Send it today."
        ));
        assert!(!looks_like_instruction_response(
            "Here, I think we should send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's a cleaned-up copy: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's the rewritten draft: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here are the improvements: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here are the corrections: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is what you said: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's what you asked for: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's what you dictated: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is a summary: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here it is with punctuation: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here it is, punctuated: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here it is cleaned up: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here it is punctuated. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is the final transcript. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's the polished version. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here I've cleaned it up: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's your corrected transcript. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected transcript as requested."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected transcript for your request. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here's the cleaned-up transcript for you. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is the corrected transcript as you requested. Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is the transcription: Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here you go! Send it today."
        ));
        assert!(looks_like_instruction_response(
            "Here is the transcript? Send it today."
        ));
        assert!(looks_like_instruction_response(
            "The transcript ends here without additional context. The user did not ask a question."
        ));
        assert!(looks_like_instruction_response(
            "Bug Report Assessment: The user performed a long dictation session."
        ));
        assert!(looks_like_instruction_response(
            "User report summary: User performed a long dictation session."
        ));
        assert!(!looks_like_instruction_response("Hello, \"testing\"."));
        assert!(!looks_like_instruction_response(
            "The user story for this sprint covers the dictation page."
        ));
        assert!(!looks_like_instruction_response(
            "User reported the bug to support."
        ));
        assert!(!looks_like_instruction_response(
            "Here's the API key to Poncho."
        ));
        assert!(!looks_like_instruction_response(
            "Here’s the API key to Poncho."
        ));
        assert!(!looks_like_instruction_response("I’ll send the agenda."));
    }

    #[test]
    fn cleanup_timeout_keeps_base_budget_for_short_dictations() {
        // A quick one-liner should not wait longer than the historical 15s.
        assert_eq!(
            dictation_cleanup_timeout("Send the doc to Sarah today".len()),
            Duration::from_millis(DICTATION_CLEANUP_BASE_TIMEOUT_MS + 27 * 5),
        );
        assert_eq!(
            dictation_cleanup_timeout(0),
            Duration::from_millis(DICTATION_CLEANUP_BASE_TIMEOUT_MS),
        );
    }

    #[test]
    fn cleanup_timeout_scales_with_long_rambling_dictations() {
        // JUN-212: a long rambling dictation timed out of the flat 15s cleanup
        // budget and pasted raw, unpunctuated ASR text. Model the reported
        // transcript: minutes of continuous filler-heavy speech, several
        // thousand bytes long.
        let reported_fragment = "side nav and everything Like it looks really sharp you \
             know meeting should kind of be or medium should kind of be used like sparingly ";
        let long_rambling_transcript = reported_fragment.repeat(40);
        let budget = dictation_cleanup_timeout(long_rambling_transcript.len());

        assert!(
            budget > Duration::from_millis(DICTATION_CLEANUP_BASE_TIMEOUT_MS),
            "a long dictation must get more than the base cleanup budget"
        );
        assert!(
            budget <= Duration::from_millis(DICTATION_CLEANUP_MAX_TIMEOUT_MS),
            "the cleanup budget must stay bounded"
        );
    }

    /// The JUN-212 transcript shape: minutes of unpunctuated, filler-heavy,
    /// run-on speech, with stray mid-text capitals from the ASR pass.
    fn jun_212_style_transcript() -> String {
        "side nav and everything Like it looks really sharp you know meeting \
         should kind of be or medium should kind of be used like sparingly \
         because when everything is bold nothing is bold right and then I was \
         thinking about the onboarding flow because we still have that drop \
         off on the second step "
            .repeat(12)
            .trim()
            .to_string()
    }

    #[test]
    fn short_dictations_stay_one_cleanup_chunk() {
        let text = "just a quick note about the design review tomorrow";
        assert_eq!(
            split_dictation_cleanup_chunks(text, DICTATION_CLEANUP_CHUNK_TARGET_BYTES),
            vec![text]
        );
    }

    #[test]
    fn long_rambling_dictations_split_at_word_boundaries_and_keep_every_word() {
        let transcript = jun_212_style_transcript();
        let chunks =
            split_dictation_cleanup_chunks(&transcript, DICTATION_CLEANUP_CHUNK_TARGET_BYTES);

        assert!(chunks.len() > 1, "a long dictation must be chunked");
        for chunk in &chunks {
            // A chunk may overrun the target only until the next whitespace.
            assert!(
                chunk.len() <= DICTATION_CLEANUP_CHUNK_TARGET_BYTES + 40,
                "chunk of {} bytes overruns the target too far",
                chunk.len()
            );
            assert!(!chunk.is_empty());
            assert!(!chunk.starts_with(char::is_whitespace));
            assert!(!chunk.ends_with(char::is_whitespace));
        }
        // Chunking must never cost the speaker a single word.
        assert_eq!(chunks.join(" "), transcript);
    }

    #[test]
    fn whitespace_free_text_stays_one_cleanup_chunk() {
        // Unsegmented text (e.g. CJK with no spaces) has no safe cut points.
        let text = "x".repeat(2_000);
        assert_eq!(
            split_dictation_cleanup_chunks(&text, DICTATION_CLEANUP_CHUNK_TARGET_BYTES),
            vec![text.as_str()]
        );
    }

    #[test]
    fn joining_cleaned_chunks_preserves_boundary_line_breaks() {
        let chunks = vec![
            "First topic wrapped up.\n\n".to_string(),
            "Second topic starts here.".to_string(),
            String::new(),
            "And a closing thought.".to_string(),
        ];
        assert_eq!(
            join_cleaned_dictation_chunks(&chunks),
            "First topic wrapped up.\n\nSecond topic starts here. And a closing thought."
        );

        // A break the model emitted at the START of the next chunk survives
        // too, without picking up a stray space before it.
        let leading_break = vec![
            "First topic wrapped up.".to_string(),
            "\n\nSecond topic starts here.".to_string(),
        ];
        assert_eq!(
            join_cleaned_dictation_chunks(&leading_break),
            "First topic wrapped up.\n\nSecond topic starts here."
        );
    }

    #[test]
    fn joining_cleaned_chunks_attaches_leading_punctuation() {
        // A spoken "comma" or "period" right after the split point makes the
        // next cleaned chunk start with the mark; it must attach to the
        // previous word instead of floating after a space.
        let chunks = vec![
            "we shipped the fix".to_string(),
            ", and the tests are green.".to_string(),
        ];
        assert_eq!(
            join_cleaned_dictation_chunks(&chunks),
            "we shipped the fix, and the tests are green."
        );
    }

    #[test]
    fn chunk_trim_keeps_edge_line_breaks_but_drops_spaces() {
        assert_eq!(
            trim_cleaned_dictation_chunk("  New paragraph next.\n\n \t"),
            "New paragraph next.\n\n"
        );
        assert_eq!(
            trim_cleaned_dictation_chunk("\n\nStarts on a new paragraph."),
            "\n\nStarts on a new paragraph."
        );
        assert_eq!(trim_cleaned_dictation_chunk(" plain text "), "plain text");
    }

    #[test]
    fn cleanup_timeout_is_capped_at_the_maximum() {
        // MAX_DICTATION_TEXT_CHARS upstream is 20k; even that never waits
        // longer than the cap, and absurd lengths cannot overflow.
        assert_eq!(
            dictation_cleanup_timeout(20_000),
            Duration::from_millis(DICTATION_CLEANUP_MAX_TIMEOUT_MS),
        );
        assert_eq!(
            dictation_cleanup_timeout(usize::MAX),
            Duration::from_millis(DICTATION_CLEANUP_MAX_TIMEOUT_MS),
        );
    }

    #[test]
    fn treats_empty_provider_transcript_as_silent_error() {
        let event = serde_json::json!({
            "type": "error",
            "payload": {
                "code": "transcription_empty",
                "message": "Groq did not return any transcript text."
            }
        });

        assert!(is_silent_transcription_error(&event));
    }
}

#[cfg(test)]
mod agent_management_tests {
    use super::*;
    #[tokio::test]
    async fn deleting_history_is_atomic_and_profile_qualified() {
        let pool = sqlx_sqlite::SqlitePool::connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query::query("CREATE TABLE dictation_history(id TEXT PRIMARY KEY,profile TEXT NOT NULL,text TEXT NOT NULL)").execute(&pool).await.unwrap();
        sqlx::query::query(
            "INSERT INTO dictation_history VALUES('same-id','owner','Synthetic private text')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert_eq!(
            delete_history_for_profile(&pool, "other", "same-id")
                .await
                .unwrap_err()
                .code,
            "dictation_not_found"
        );
        let count: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM dictation_history")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
        delete_history_for_profile(&pool, "owner", "same-id")
            .await
            .unwrap();
        let count: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM dictation_history")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn composer_delivery_never_runs_wake_word_actions_or_pastes_quarantined_text() {
        let transcript = TranscriptionProviderResult {
            text: "Hey Clovy inspect notes".into(),
            language: None,
            provider: "fixture".into(),
        };
        let outcome = DictationTranscriptionOutcome {
            helper_command: serde_json::json!({"type":"discard_recording"}),
            event: Some(
                serde_json::json!({"type":"agent_session_prompt","payload":{"prompt":"inspect notes"}}),
            ),
            transcript: Some(transcript),
            quarantine_transcript: None,
        };
        assert_eq!(
            agent_composer_text(&outcome),
            Some("Hey Clovy inspect notes")
        );
        let outcome = quarantine_low_speech_evidence_outcome(outcome, true);
        assert_eq!(agent_composer_text(&outcome), None);
    }
    #[test]
    fn wrong_take_and_late_confirmations_cannot_resurrect_cancelled_delivery() {
        let state = HelperState::default();
        let mut target = agent_dictation_target("owner", "conversation-a");
        target.phase = "listening".into();
        *state.agent_take.lock().unwrap() = Some(target.clone());
        set_agent_dictation_phase(&state, "other-take", "delivering", None);
        assert_eq!(
            state.agent_take.lock().unwrap().as_ref().unwrap().phase,
            "listening"
        );
        set_agent_dictation_phase(&state, &target.take_id, "cancelled", None);
        set_agent_dictation_phase(&state, &target.take_id, "listening", None);
        set_agent_dictation_phase(&state, &target.take_id, "delivering", None);
        let current = state.agent_take.lock().unwrap().clone().unwrap();
        assert_eq!(current.phase, "cancelled");
        assert_eq!(current.composer_request_id, target.composer_request_id);
        assert_eq!(current.conversation_id, "conversation-a");
    }
}

#[cfg(all(test, unix))]
mod serialized_agent_write_tests {
    use super::*;
    use crate::domain::turn_authority::TurnAuthority;
    use std::io::Read;
    use std::os::unix::net::UnixStream;

    fn denied_after_serialization(wait_for_generation: bool) {
        let authority = TurnAuthority::new();
        let (writer, mut reader) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let process = Arc::new(Mutex::new(writer));
        let generation = Arc::new(Mutex::new(0_u64));
        let generation_held = wait_for_generation.then(|| generation.lock().unwrap());
        let process_held = (!wait_for_generation).then(|| process.lock().unwrap());
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let worker = {
            let process = process.clone();
            let generation = generation.clone();
            let authority = authority.clone();
            std::thread::spawn(move || {
                let mut entered_tx = Some(entered_tx);
                if wait_for_generation {
                    entered_tx.take().unwrap().send(()).unwrap();
                }
                let _generation = generation.lock().unwrap();
                if let Some(entered_tx) = entered_tx {
                    entered_tx.send(()).unwrap();
                }
                let mut process = process.lock().unwrap();
                write_helper_command(
                    &mut *process,
                    &serde_json::json!({"type":"start_listening","takeId":"serialized-proof"}),
                    Some(&authority),
                )
            })
        };
        entered_rx.recv().unwrap();
        authority.revoke();
        drop(generation_held);
        drop(process_held);
        assert_eq!(worker.join().unwrap().unwrap_err().code, "turn_expired");
        let mut byte = [0_u8];
        assert_eq!(
            reader.read(&mut byte).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    #[test]
    fn revoked_while_waiting_for_process_lock_writes_no_start_command() {
        denied_after_serialization(false);
    }
    #[test]
    fn revoked_while_waiting_for_generation_lock_writes_no_start_command() {
        denied_after_serialization(true);
    }
    #[test]
    fn live_authority_writes_one_real_ipc_command() {
        let authority = TurnAuthority::new();
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        write_helper_command(
            &mut writer,
            &serde_json::json!({"type":"start_listening","takeId":"live-proof"}),
            Some(&authority),
        )
        .unwrap();
        let mut bytes = [0_u8; 256];
        let count = reader.read(&mut bytes).unwrap();
        let command: serde_json::Value = serde_json::from_slice(&bytes[..count]).unwrap();
        assert_eq!(command["type"], "start_listening");
        assert_eq!(command["takeId"], "live-proof");
        assert_eq!(bytes[count - 1], b'\n');
    }
}
