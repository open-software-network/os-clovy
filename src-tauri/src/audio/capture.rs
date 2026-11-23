use crate::{
    app_paths::AppPaths,
    audio::{
        capture_buffer::{CaptureInput, MicrophoneWriter, MicrophoneWriterIssue},
        live_preview::{
            start_live_transcript_preview, start_system_live_transcript_preview,
            LivePreviewController, SystemLivePreviewController,
        },
        system_audio::{SystemAudioCapture, SystemAudioFailure, SystemAudioStopResult},
    },
    domain::types::{
        AppError, AudioLevelDto, RecordingSessionDto, RecordingSource, RecordingSourceMode,
        RecordingState, RecordingStatusDto, SourceState, SourceStatusDto, SourceWarningDto,
    },
};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use hound::{SampleFormat, WavSpec, WavWriter};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock, Mutex,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

#[path = "../../native/video_client.rs"]
pub mod video;

pub const DEFAULT_SILENCE_THRESHOLD: f32 = 0.012;
// Healthy devices open quickly; this bounds CPAL/CoreAudio hangs during device handoff.
pub const CAPTURE_START_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a first-run start waits on the macOS microphone prompt; the
/// agent recorder lease budgets against it.
pub const MICROPHONE_PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// Start handshake between the capture builder and its timeout watchdog.
/// The builder commits `Published` in the same critical section that
/// publishes `ACTIVE_RECORDING`, so the watchdog can never declare a timeout
/// for a capture that is (or is about to be) active — it either abandons a
/// still-pending build or accepts a published one as success.
pub use super::control_admission::CaptureStartState;

pub type CaptureStartHandshake = Mutex<CaptureStartState>;

/// Host-owned admission, checked after preparation and final publication locks.
pub(crate) use super::control_admission::CaptureStartAuthorization;

fn check_capture_authorization(
    authorization: Option<&CaptureStartAuthorization>,
) -> Result<(), AppError> {
    if let Some(authorization) = authorization {
        authorization()?;
    }
    Ok(())
}

pub(crate) fn admit_capture_start(
    state: &mut CaptureStartState,
    already_active: bool,
    authorization: Option<&CaptureStartAuthorization>,
    start: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    if *state == CaptureStartState::Abandoned {
        return Err(capture_start_timeout_error());
    }
    if already_active {
        return Err(recording_already_active_error());
    }
    check_capture_authorization(authorization)?;
    start()?;
    super::control_admission::publish_capture_start(state, already_active, authorization)
}

fn capture_start_abandoned(handshake: &CaptureStartHandshake) -> bool {
    handshake
        .lock()
        .map(|state| *state == CaptureStartState::Abandoned)
        .unwrap_or(false)
}
pub const RECOVERY_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(500);
const MICROPHONE_STALL_THRESHOLD: Duration = Duration::from_secs(3);
const MICROPHONE_STREAM_WARNING_MESSAGE: &str =
    "Microphone input stopped unexpectedly. Audio after this point may be missing.";
const MICROPHONE_WRITER_WARNING_MESSAGE: &str =
    "Clovy stopped saving microphone audio. Stop the recording now to preserve the audio already written.";
const MICROPHONE_BUFFER_OVERFLOW_WARNING_MESSAGE: &str =
    "Microphone recording could not keep up. Some audio may be missing.";

/// Held while a recording taken out of `ACTIVE_RECORDING` is finalized, so a
/// new capture cannot start until the previous one released its devices.
/// Lock order: this first, then `ACTIVE_RECORDING`.
static RECORDING_FINALIZATION: Mutex<()> = Mutex::new(());
// A timed-out native start can still be inside CPAL. Keep its reservation until
// that worker has cleaned up, without blocking the watchdog or active state.
static CAPTURE_START_RESERVATION: Mutex<()> = Mutex::new(());
static ACTIVE_RECORDING: LazyLock<Mutex<Option<ActiveRecording>>> =
    LazyLock::new(|| Mutex::new(None));

pub struct StartedRecording {
    pub session_id: String,
    pub note_id: String,
    pub source_mode: RecordingSourceMode,
    pub partial_path: PathBuf,
    pub final_path: PathBuf,
    pub sources: Vec<StartedSource>,
    pub device_label: Option<String>,
    pub status: RecordingStatusDto,
}

pub struct FinishedRecording {
    pub session_id: String,
    pub note_id: String,
    pub source_mode: RecordingSourceMode,
    pub final_path: PathBuf,
    pub sources: Vec<FinishedSource>,
    pub elapsed_ms: i64,
    pub recording: RecordingSessionDto,
}

pub struct StartedSource {
    pub source: RecordingSource,
    pub partial_path: PathBuf,
    pub final_path: PathBuf,
}

pub struct FinishedSource {
    pub source: RecordingSource,
    pub final_path: PathBuf,
    pub elapsed_ms: i64,
    pub dropped_samples: u64,
    pub capture_issue: Option<MicrophoneStreamIssue>,
    pub failure: Option<SystemAudioFailure>,
}

pub struct CaptureRecoverySnapshot {
    pub status: RecordingStatusDto,
    pub should_persist: bool,
}

/// Minimal durable checkpoint data collected independently from UI telemetry.
/// Capture levels stay on lock-free atomics; recovery carries only the
/// overflow counter needed for durable diagnostics.
pub struct RecordingRecoveryCheckpoint {
    pub session_id: String,
    pub state: RecordingState,
    pub elapsed_ms: i64,
    pub dropped_samples: u64,
    pub capture_issue: Option<MicrophoneStreamIssue>,
}

struct ActiveRecording {
    session_id: String,
    note_id: String,
    partial_path: PathBuf,
    final_path: PathBuf,
    source_mode: RecordingSourceMode,
    system_final_path: Option<PathBuf>,
    system_capture: Option<SystemAudioCapture>,
    video_capture: Option<video::VideoCapture>,
    started: Instant,
    active_since: Option<Instant>,
    accumulated_active: Duration,
    paused: bool,
    paused_flag: Arc<AtomicBool>,
    /// While set, the mic callback writes silence instead of samples (see
    /// write_input_data) so the user's voice stays out of the note while the
    /// dictation helper owns it. Independent of pause: the recording keeps
    /// running and both tracks keep growing.
    mic_duck_flag: Arc<AtomicBool>,
    // Keep the stream before the writer so implicit field teardown stops the
    // producer before MicrophoneWriter::drop joins its consumer thread.
    _stream: cpal::Stream,
    writer: MicrophoneWriter,
    stream_error: Arc<Mutex<Option<MicrophoneStreamIssue>>>,
    last_callback_at: Arc<Mutex<Option<Instant>>>,
    // First stall ever observed, kept until finish: a transient stall whose
    // callbacks later resume must still leave a capture_stream_error trail.
    stall_latch: Arc<Mutex<Option<MicrophoneStreamIssue>>>,
    last_recovery_snapshot_elapsed_ms: i64,
    live_preview: Option<LivePreviewController>,
    system_live_preview: Option<SystemLivePreviewController>,
    live_preview_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MicrophoneStreamIssue {
    pub code: String,
    pub message: String,
    pub elapsed_ms: i64,
}

// The MVP permits only one active recording and guards it behind a process-wide
// mutex. CPAL's CoreAudio stream is intentionally conservative about Send across
// platforms; on macOS it is safe for this usage because commands only hold/drop
// the stream through this single recorder lifecycle.
unsafe impl Send for ActiveRecording {}

pub fn microphone_permission_state() -> (String, Option<String>) {
    microphone_permission_state_from_authorization(microphone_authorization_status())
}

fn microphone_permission_state_from_authorization(
    status: MicrophoneAuthorizationStatus,
) -> (String, Option<String>) {
    match status {
        MicrophoneAuthorizationStatus::Granted => ("granted".to_string(), None),
        MicrophoneAuthorizationStatus::Denied => {
            ("denied".to_string(), Some(microphone_permission_hint()))
        }
        MicrophoneAuthorizationStatus::Restricted => {
            ("restricted".to_string(), Some(microphone_permission_hint()))
        }
        MicrophoneAuthorizationStatus::NotDetermined => (
            "not_determined".to_string(),
            Some(microphone_permission_hint()),
        ),
        MicrophoneAuthorizationStatus::Unknown => {
            ("unknown".to_string(), Some(microphone_permission_hint()))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MicrophoneAuthorizationStatus {
    Granted,
    Denied,
    Restricted,
    NotDetermined,
    Unknown,
}

#[cfg(target_os = "macos")]
fn microphone_authorization_status() -> MicrophoneAuthorizationStatus {
    macos_microphone_authorization_status().unwrap_or(MicrophoneAuthorizationStatus::Unknown)
}

#[cfg(target_os = "macos")]
fn macos_microphone_authorization_status() -> Option<MicrophoneAuthorizationStatus> {
    use objc2::{msg_send, runtime::AnyClass};
    use objc2_foundation::NSString;

    let device_class = AnyClass::get(c"AVCaptureDevice")?;
    // AVMediaTypeAudio's raw value is "soun"; this matches
    // AVCaptureDevice.authorizationStatus(for: .audio) in the dictation helper.
    let media_type = NSString::from_str("soun");
    let raw_status: isize =
        unsafe { msg_send![device_class, authorizationStatusForMediaType: &*media_type] };
    Some(match raw_status {
        0 => MicrophoneAuthorizationStatus::NotDetermined,
        1 => MicrophoneAuthorizationStatus::Restricted,
        2 => MicrophoneAuthorizationStatus::Denied,
        3 => MicrophoneAuthorizationStatus::Granted,
        _ => MicrophoneAuthorizationStatus::Unknown,
    })
}

#[cfg(not(target_os = "macos"))]
fn microphone_authorization_status() -> MicrophoneAuthorizationStatus {
    let host = cpal::default_host();
    if host.default_input_device().is_some() {
        MicrophoneAuthorizationStatus::Granted
    } else {
        MicrophoneAuthorizationStatus::Denied
    }
}

/// TCC microphone grants are bundle-scoped: onboarding prompts through the
/// dictation helper (`dev.guilhermexp.clovy.dictation-helper`), whose grant
/// never covers the main app bundle. The main app must trigger its own TCC
/// prompt before capture, or a fresh install records only zeros.
#[cfg(target_os = "macos")]
pub fn request_microphone_permission_blocking() -> (String, Option<String>) {
    use objc2::{msg_send, runtime::AnyClass};
    use objc2_foundation::NSString;

    if microphone_authorization_status() != MicrophoneAuthorizationStatus::NotDetermined {
        return microphone_permission_state();
    }
    let Some(device_class) = AnyClass::get(c"AVCaptureDevice") else {
        return microphone_permission_state();
    };
    let (sender, receiver) = std::sync::mpsc::channel::<bool>();
    let handler = block2::RcBlock::new(move |granted: objc2::runtime::Bool| {
        let _ = sender.send(granted.as_bool());
    });
    let media_type = NSString::from_str("soun");
    let _: () = unsafe {
        msg_send![
            device_class,
            requestAccessForMediaType: &*media_type,
            completionHandler: &*handler
        ]
    };
    // The prompt has no OS-side timeout; a user who walks away must not hang
    // the start command forever.
    match receiver.recv_timeout(MICROPHONE_PROMPT_TIMEOUT) {
        Ok(_) => microphone_permission_state(),
        Err(_) => (
            "not_determined".to_string(),
            Some("Approve the macOS microphone prompt, then try again.".to_string()),
        ),
    }
}

pub fn microphone_device_available() -> bool {
    cpal::default_host().default_input_device().is_some()
}

pub fn microphone_device_hint() -> String {
    "No microphone input device is available.".to_string()
}

fn microphone_permission_hint() -> String {
    #[cfg(target_os = "macos")]
    {
        "Enable microphone access in macOS Privacy & Security settings.".to_string()
    }
    #[cfg(target_os = "windows")]
    {
        "Enable microphone access in Windows privacy settings.".to_string()
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        "Enable microphone access in your system privacy settings.".to_string()
    }
}

pub fn start_capture(
    app: tauri::AppHandle,
    paths: &AppPaths,
    note_id: String,
    source_mode: RecordingSourceMode,
) -> Result<StartedRecording, AppError> {
    start_capture_with_cancel(
        app,
        paths,
        note_id,
        source_mode,
        Arc::new(Mutex::new(CaptureStartState::Pending)),
        None,
    )
}

pub fn start_capture_with_cancel(
    app: tauri::AppHandle,
    paths: &AppPaths,
    note_id: String,
    source_mode: RecordingSourceMode,
    handshake: Arc<CaptureStartHandshake>,
    authorization: Option<CaptureStartAuthorization>,
) -> Result<StartedRecording, AppError> {
    let _reservation = super::control_admission::reserve_capture_start(&CAPTURE_START_RESERVATION)?;
    // Holds an update install back until this recording is published in
    // `ACTIVE_RECORDING` (or fails); refused while an update installs.
    let _update_gate = crate::update_gate::UPDATE_GATE.begin_recording_start()?;
    {
        // Waits for a recording that is still finalizing to release its
        // devices; that recording is no longer in `ACTIVE_RECORDING`.
        let _finalization = lock_finalization()?;
        let active = ACTIVE_RECORDING.lock().map_err(|_| {
            AppError::new("recording_lock_failed", "Recording state is unavailable.")
        })?;
        if active.is_some() {
            return Err(recording_already_active_error());
        }
        check_capture_authorization(authorization.as_ref())?;
    }

    let host = cpal::default_host();
    let device = host.default_input_device().ok_or_else(|| {
        AppError::new(
            "microphone_unavailable",
            "No microphone input device is available.",
        )
    })?;
    let device_label = device.name().ok();
    let config = device
        .default_input_config()
        .map_err(|error| AppError::new("microphone_unavailable", error.to_string()))?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();

    let session_id = Uuid::new_v4().to_string();
    let note_dir = paths
        .recording_session_dir(&note_id, &session_id)
        .map_err(|error| AppError::new("invalid_recording_path", error.to_string()))?;
    std::fs::create_dir_all(&note_dir)
        .map_err(|error| AppError::new("audio_writer_failed", error.to_string()))?;
    let partial_path = note_dir.join("microphone.partial.wav");
    let final_path = note_dir.join("microphone.wav");
    let system_partial_path = (source_mode == RecordingSourceMode::MicrophonePlusSystem)
        .then(|| note_dir.join("system.partial.wav"));
    let system_final_path = (source_mode == RecordingSourceMode::MicrophonePlusSystem)
        .then(|| note_dir.join("system.wav"));
    let writer = WavWriter::create(
        &partial_path,
        WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        },
    )
    .map_err(|error| AppError::new("audio_writer_failed", error.to_string()))?;

    let stream_error = Arc::new(Mutex::new(None));
    let last_callback_at = Arc::new(Mutex::new(None));
    let stall_latch = Arc::new(Mutex::new(None));
    let paused_flag = Arc::new(AtomicBool::new(false));
    let mic_duck_flag = Arc::new(AtomicBool::new(false));
    // The user's Live transcription setting gates both preview lanes at the
    // source: when off, no preview audio leaves the device and nothing is
    // billed (JUN-375).
    let live_preview_available = crate::providers::live_transcription()
        && crate::providers::transcription::current_backend().is_ok();
    let live_preview = if live_preview_available {
        Some(start_live_transcript_preview(
            app.clone(),
            note_id.clone(),
            session_id.clone(),
            source_mode,
            RecordingSource::Microphone,
            sample_rate,
            channels,
        ))
    } else {
        None
    };
    let preview_sink = live_preview.as_ref().map(LivePreviewController::sink);
    let (capture_input, writer) = match MicrophoneWriter::start(
        writer,
        sample_rate,
        channels,
        preview_sink,
        Arc::clone(&last_callback_at),
    ) {
        Ok(writer) => writer,
        Err(error) => {
            if let Some(live_preview) = live_preview {
                live_preview.cancel();
            }
            let _ = std::fs::remove_file(&partial_path);
            return Err(AppError::new("audio_writer_failed", error));
        }
    };
    let input_for_callback = capture_input.clone();
    let paused_for_callback = Arc::clone(&paused_flag);
    let mic_duck_for_callback = Arc::clone(&mic_duck_flag);
    // Diagnostic anchor only, captured before stream construction so err_fn
    // can attribute early errors. The recording clock (`started`, below) is
    // anchored after stream.play(): it feeds elapsed() and the finish-time
    // duration validation, which must not count stream setup time.
    let capture_setup_started = Instant::now();
    let stream_error_for_callback = Arc::clone(&stream_error);
    let err_fn = move |error| {
        // Always leave a raw trace, then record the first structured issue.
        // A blocking lock is required: this is CPAL's error callback (not the
        // realtime data callback), and a try_lock could silently drop the one
        // error whenever a status poll holds the mutex at that instant.
        eprintln!("audio stream error: {error}");
        let mut issue = match stream_error_for_callback.lock() {
            Ok(issue) => issue,
            Err(poisoned) => poisoned.into_inner(),
        };
        if issue.is_none() {
            *issue = Some(MicrophoneStreamIssue {
                code: "microphone_stream_error".to_string(),
                message: format!("Microphone input stopped unexpectedly: {error}"),
                elapsed_ms: capture_setup_started
                    .elapsed()
                    .as_millis()
                    .min(i64::MAX as u128) as i64,
            });
        }
    };

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_input_stream(
            &config.clone().into(),
            move |data: &[f32], _| {
                write_input_data(
                    data.iter().copied(),
                    &input_for_callback,
                    &paused_for_callback,
                    &mic_duck_for_callback,
                )
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_input_stream(
            &config.clone().into(),
            move |data: &[i16], _| {
                write_input_data(
                    data.iter().map(|sample| *sample as f32 / i16::MAX as f32),
                    &input_for_callback,
                    &paused_for_callback,
                    &mic_duck_for_callback,
                )
            },
            err_fn,
            None,
        ),
        cpal::SampleFormat::U16 => device.build_input_stream(
            &config.clone().into(),
            move |data: &[u16], _| {
                write_input_data(
                    data.iter()
                        .map(|sample| (*sample as f32 - 32768.0) / 32768.0),
                    &input_for_callback,
                    &paused_for_callback,
                    &mic_duck_for_callback,
                )
            },
            err_fn,
            None,
        ),
        _ => {
            return Err(AppError::new(
                "microphone_unavailable",
                "Unsupported microphone sample format.",
            ))
        }
    }
    .map_err(|error| AppError::new("audio_writer_failed", error.to_string()))?;
    let mut system_live_preview = None;
    let system_capture = if let (Some(system_partial_path), Some(system_final_path)) =
        (system_partial_path.clone(), system_final_path.clone())
    {
        if let Err(error) = check_capture_authorization(authorization.as_ref()) {
            cleanup_abandoned_capture(
                writer,
                partial_path,
                Some(system_partial_path),
                live_preview,
                None,
                None,
                None,
            );
            return Err(error);
        }
        match SystemAudioCapture::start(
            system_partial_path.clone(),
            system_final_path.clone(),
            Duration::ZERO,
            authorization.clone(),
        ) {
            Ok(capture) => {
                if let Err(error) = check_capture_authorization(authorization.as_ref()) {
                    cleanup_abandoned_capture(
                        writer,
                        partial_path,
                        Some(system_partial_path),
                        live_preview,
                        None,
                        Some(capture),
                        None,
                    );
                    return Err(error);
                }
                if live_preview_available {
                    system_live_preview = Some(start_system_live_transcript_preview(
                        app.clone(),
                        note_id.clone(),
                        session_id.clone(),
                        source_mode,
                        system_partial_path,
                    ));
                }
                Some(capture)
            }
            Err(error) => {
                drop(writer);
                let _ = std::fs::remove_file(&partial_path);
                return Err(error);
            }
        }
    } else {
        None
    };
    let live_preview_enabled = live_preview.is_some() || system_live_preview.is_some();
    if capture_start_abandoned(&handshake) {
        cleanup_abandoned_capture(
            writer,
            partial_path,
            system_partial_path,
            live_preview,
            system_live_preview,
            system_capture,
            None,
        );
        return Err(capture_start_timeout_error());
    }
    let mut started = Instant::now();
    let status = RecordingStatusDto {
        session_id: session_id.clone(),
        note_id: Some(note_id.clone()),
        source_mode,
        state: RecordingState::Recording,
        elapsed_ms: 0,
        level: AudioLevelDto {
            peak: 0.0,
            rms: 0.0,
            recent_peaks: Vec::new(),
        },
        silence_warning: false,
        bytes_written: 0,
        live_preview_enabled,
        sources: source_statuses(
            source_mode,
            RecordingState::Recording,
            0,
            MicrophoneStatusInput {
                level: AudioLevelDto::default(),
                bytes_written: 0,
                dropped_samples: 0,
                silence_warning: false,
                capture_issue: None,
            },
            system_capture.as_ref(),
        ),
        warnings: Vec::new(),
    };
    // The watchdog must be able to abandon while the native driver is stuck.
    // Check authority immediately before the native effect, then recheck under
    // the publication locks after it returns.
    let started_result = super::control_admission::start_native_capture(
        &ACTIVE_RECORDING,
        &handshake,
        authorization.as_ref(),
        || {
            stream
                .play()
                .map_err(|error| AppError::new("audio_writer_failed", error.to_string()))?;
            started = Instant::now();
            Ok(())
        },
    );
    if let Err(error) = started_result {
        cleanup_abandoned_capture(
            writer,
            partial_path,
            system_partial_path,
            live_preview,
            system_live_preview,
            system_capture,
            Some(stream),
        );
        return Err(error);
    }
    let mut active = match ACTIVE_RECORDING.lock() {
        Ok(active) => active,
        Err(_) => {
            cleanup_abandoned_capture(
                writer,
                partial_path,
                system_partial_path,
                live_preview,
                system_live_preview,
                system_capture,
                Some(stream),
            );
            return Err(AppError::new(
                "recording_lock_failed",
                "Recording state is unavailable.",
            ));
        }
    };
    // Commit-or-abandon happens atomically with the publish below (lock
    // order: ACTIVE_RECORDING, then the handshake; the watchdog only ever
    // takes the handshake). Past this block the watchdog treats the build as
    // successfully published and returns it as success.
    {
        let Ok(mut state) = handshake.lock() else {
            drop(active);
            cleanup_abandoned_capture(
                writer,
                partial_path,
                system_partial_path,
                live_preview,
                system_live_preview,
                system_capture,
                Some(stream),
            );
            return Err(AppError::new(
                "recording_lock_failed",
                "Recording state is unavailable.",
            ));
        };
        if let Err(error) =
            admit_capture_start(&mut state, active.is_some(), authorization.as_ref(), || {
                Ok(())
            })
        {
            drop(state);
            drop(active);
            cleanup_abandoned_capture(
                writer,
                partial_path,
                system_partial_path,
                live_preview,
                system_live_preview,
                system_capture,
                Some(stream),
            );
            return Err(error);
        }
    }

    *active = Some(ActiveRecording {
        session_id: session_id.clone(),
        note_id: note_id.clone(),
        partial_path: partial_path.clone(),
        final_path: final_path.clone(),
        source_mode,
        system_final_path: system_final_path.clone(),
        system_capture,
        video_capture: None,
        started,
        active_since: Some(started),
        accumulated_active: Duration::ZERO,
        paused: false,
        paused_flag,
        mic_duck_flag,
        writer,
        stream_error,
        last_callback_at,
        stall_latch,
        last_recovery_snapshot_elapsed_ms: 0,
        live_preview,
        system_live_preview,
        live_preview_enabled,
        _stream: stream,
    });

    Ok(StartedRecording {
        session_id,
        note_id,
        source_mode,
        partial_path,
        final_path,
        sources: started_sources(source_mode, &note_dir),
        device_label,
        status,
    })
}

pub fn capture_start_timeout_error() -> AppError {
    AppError::new(
        "capture_start_timeout",
        "Could not start the microphone. Try again, or check the selected input device.",
    )
}

fn recording_already_active_error() -> AppError {
    AppError::new(
        "recording_already_active",
        "A previous recording is still active. Clovy attempted to save it locally; please try again.",
    )
}

fn cleanup_abandoned_capture(
    writer: MicrophoneWriter,
    partial_path: PathBuf,
    system_partial_path: Option<PathBuf>,
    live_preview: Option<LivePreviewController>,
    system_live_preview: Option<SystemLivePreviewController>,
    system_capture: Option<SystemAudioCapture>,
    stream: Option<cpal::Stream>,
) {
    drop(stream);
    if let Some(live_preview) = live_preview {
        live_preview.cancel();
    }
    if let Some(system_live_preview) = system_live_preview {
        system_live_preview.cancel();
    }
    if let Some(system_capture) = system_capture {
        let _ = system_capture.stop();
    }
    drop(writer);
    let _ = std::fs::remove_file(partial_path);
    if let Some(system_partial_path) = system_partial_path {
        let _ = std::fs::remove_file(system_partial_path);
    }
}

/// Ducks or restores the active recording's microphone channel. While ducked
/// the mic callback writes silence in place of samples, so the user's voice
/// never lands in the note but the mic track keeps growing in lockstep with
/// system audio. Dictation owns this: the helper's listening window ducks the
/// mic so dictating into Clovy mid-meeting doesn't contaminate the meeting
/// note. Best-effort and idempotent — no active recording is a no-op, and the
/// flag dies with the session, so a stop mid-dictation can't leak a duck.
pub fn set_mic_ducked(ducked: bool) {
    let Ok(mut active) = lock_active() else {
        return;
    };
    if let Some(recording) = active.as_mut() {
        recording.mic_duck_flag.store(ducked, Ordering::Release);
    }
}

/// Attach only after audio and its durable rows are ready. The worker never delays microphone startup.
pub fn attach_video(
    session_id: &str,
    repos: crate::db::repositories::Repositories,
    partial: PathBuf,
    final_path: PathBuf,
    window_id: u32,
    runtime: tokio::runtime::Handle,
) -> Result<(), AppError> {
    let mut active = lock_active()?;
    let recording = active_for_session(active.as_mut(), session_id)?;
    let capture = video::VideoCapture::start(
        repos,
        session_id.to_string(),
        partial,
        final_path,
        window_id,
        recording.elapsed().as_millis().min(i64::MAX as u128) as i64,
        runtime,
    );
    if recording.paused {
        capture.pause();
    }
    recording.video_capture = Some(capture);
    Ok(())
}

// ACTIVE_RECORDING serializes transitions, so the fixed control records retain call order.
fn publish_video_before_audio(
    video: Option<&video::VideoCapture>,
    paused: bool,
    transition: impl FnOnce(),
) {
    if let Some(video) = video {
        if paused {
            video.pause();
        } else {
            video.resume();
        }
    }
    transition();
}

pub(crate) fn pause_capture(
    session_id: &str,
    authorization: Option<&CaptureStartAuthorization>,
) -> Result<CaptureRecoverySnapshot, AppError> {
    super::control_admission::with_control_admission(&ACTIVE_RECORDING, authorization, |active| {
        let recording = active_for_session(active.as_mut(), session_id)?;
        if !recording.paused {
            crate::meeting_assist::invalidate_preview(false);
            if let Some(active_since) = recording.active_since.take() {
                recording.accumulated_active += active_since.elapsed();
            }
            recording.paused = true;
            publish_video_before_audio(recording.video_capture.as_ref(), true, || {
                recording.paused_flag.store(true, Ordering::Release);
                if let Some(system) = recording.system_capture.as_mut() {
                    system.pause();
                }
            });
        }
        Ok(recording.recovery_snapshot(RecoverySnapshotMode::Force))
    })
}

pub(crate) fn resume_capture(
    session_id: &str,
    authorization: Option<&CaptureStartAuthorization>,
) -> Result<CaptureRecoverySnapshot, AppError> {
    super::control_admission::with_control_admission(&ACTIVE_RECORDING, authorization, |active| {
        let recording = active_for_session(active.as_mut(), session_id)?;
        if recording.paused {
            recording.active_since = Some(Instant::now());
            recording.paused = false;
            publish_video_before_audio(recording.video_capture.as_ref(), false, || {
                recording.paused_flag.store(false, Ordering::Release);
                if let Some(system) = recording.system_capture.as_mut() {
                    system.resume();
                }
            });
        }
        Ok(recording.recovery_snapshot(RecoverySnapshotMode::Force))
    })
}

pub fn capture_status(session_id: &str) -> Result<RecordingStatusDto, AppError> {
    let active = lock_active()?;
    let recording = active_for_session(active.as_ref(), session_id)?;
    Ok(recording.status())
}

pub fn capture_recovery_checkpoint(
    session_id: &str,
) -> Result<Option<RecordingRecoveryCheckpoint>, AppError> {
    let mut active = lock_active()?;
    let recording = active_for_session(active.as_mut(), session_id)?;
    Ok(recording.recovery_checkpoint(RecoverySnapshotMode::Throttled))
}

pub fn is_capture_active() -> bool {
    ACTIVE_RECORDING
        .lock()
        .map(|active| active.is_some())
        .unwrap_or(false)
}

/// A finished recording is still writing its files (or a start is waiting on
/// that lock). An update install must not end the app in between.
pub fn is_capture_finalizing() -> bool {
    matches!(
        RECORDING_FINALIZATION.try_lock(),
        Err(std::sync::TryLockError::WouldBlock)
    )
}

/// Takes the active recording and finalizes it. Blocks for as long as the
/// writer and system capture take to stop, so async callers run it on a
/// blocking thread; status readers are not held up because the recording
/// leaves `ACTIVE_RECORDING` before finalization starts.
pub fn finish_active_capture() -> Result<Option<FinishedRecording>, AppError> {
    let _finalization = lock_finalization()?;
    let recording = {
        let mut active = lock_active()?;
        let Some(recording) = active.take() else {
            return Ok(None);
        };
        crate::meeting_assist::invalidate_preview(true);
        recording
    };
    finalize_recording(recording).map(Some)
}

/// Snapshot of whatever recording is currently active, regardless of session
/// id. The meeting HUD supervisor mirrors the live recording without holding
/// the session id the way the React layer does, so it can't go through
/// [`capture_status`]. Returns `None` when nothing is recording.
pub fn current_status() -> Option<RecordingStatusDto> {
    let active = lock_active().ok()?;
    active.as_ref().map(|recording| recording.status())
}

/// Takes and finalizes the recording for `session_id`; see
/// [`finish_active_capture`] for why it blocks and what it does not block.
pub(crate) fn finish_capture(
    session_id: &str,
    authorization: Option<&CaptureStartAuthorization>,
) -> Result<FinishedRecording, AppError> {
    let _finalization = lock_finalization()?;
    let recording = super::control_admission::with_control_admission(
        &ACTIVE_RECORDING,
        authorization,
        |active| {
            let Some(recording) = active.take() else {
                return Err(AppError::new(
                    "recording_not_found",
                    "Recording session was not found.",
                ));
            };
            if recording.session_id != session_id {
                let status = recording.status();
                *active = Some(recording);
                return Err(AppError::new(
                    "recording_not_found",
                    format!(
                        "Active recording is {}, not {}",
                        status.session_id, session_id
                    ),
                ));
            }
            crate::meeting_assist::invalidate_preview(true);
            Ok(recording)
        },
    )?;
    finalize_recording(recording)
}

fn finalize_recording(recording: ActiveRecording) -> Result<FinishedRecording, AppError> {
    let status = recording.status_with_state(RecordingState::Validating);
    let mut recording_dto = RecordingSessionDto {
        id: recording.session_id.clone(),
        note_id: recording.note_id.clone(),
        source_mode: recording.source_mode,
        state: status.state,
        started_at: crate::db::repositories::timestamp(),
        elapsed_ms: status.elapsed_ms,
        device_label: None,
        level: status.level,
        live_preview_enabled: recording.live_preview_enabled,
        sources: status.sources,
        warnings: status.warnings,
    };
    let ActiveRecording {
        session_id,
        note_id,
        partial_path,
        final_path,
        source_mode,
        system_capture,
        video_capture,
        system_final_path,
        stream_error,
        last_callback_at,
        stall_latch,
        writer,
        paused_flag,
        live_preview,
        system_live_preview,
        _stream,
        ..
    } = recording;
    paused_flag.store(true, Ordering::Release);
    if let Some(video) = video_capture {
        video.stop();
    }
    drop(_stream);
    let dropped_samples = writer.stats().dropped_samples;
    let microphone_finalized = (|| -> Result<_, AppError> {
        let final_writer_health = writer
            .finish()
            .map_err(|error| AppError::new("audio_finalization_failed", error))?;
        std::fs::rename(&partial_path, &final_path)
            .map_err(|error| AppError::new("audio_finalization_failed", error.to_string()))?;
        Ok(final_writer_health)
    })();
    if let Some(live_preview) = live_preview {
        live_preview.cancel();
    }
    if let Some(system_live_preview) = system_live_preview {
        system_live_preview.cancel();
    }
    // Stop the system-audio backend even when microphone finalization failed:
    // dropping `SystemAudioCapture` without `stop()` could leave capture
    // running in the background.
    let system_stopped = system_capture.map(SystemAudioCapture::stop);
    let final_writer_health = microphone_finalized?;
    set_final_microphone_bytes(
        &mut recording_dto.sources,
        final_writer_health.written_bytes(),
    );
    let writer_capture_issue =
        microphone_writer_issue(recording_dto.elapsed_ms, final_writer_health.issue);
    let mut sources = vec![FinishedSource {
        source: RecordingSource::Microphone,
        final_path: final_path.clone(),
        elapsed_ms: recording_dto.elapsed_ms,
        dropped_samples,
        capture_issue: writer_capture_issue
            .or_else(|| {
                microphone_stream_issue(
                    recording_dto.elapsed_ms,
                    RecordingState::Validating,
                    &stream_error,
                    &last_callback_at,
                    false,
                )
            })
            .or_else(|| stall_latch.lock().ok().and_then(|latch| latch.clone())),
        failure: None,
    }];
    if let Some(system_result) = system_stopped {
        match system_result {
            SystemAudioStopResult::Stopped(system_path) => {
                sources.push(FinishedSource {
                    source: RecordingSource::System,
                    final_path: system_final_path.unwrap_or(system_path.clone()),
                    elapsed_ms: recording_dto.elapsed_ms,
                    dropped_samples: 0,
                    capture_issue: None,
                    failure: None,
                });
            }
            SystemAudioStopResult::Failed(failure) => {
                if let Some(system_path) = system_final_path {
                    sources.push(FinishedSource {
                        source: RecordingSource::System,
                        final_path: system_path,
                        elapsed_ms: recording_dto.elapsed_ms,
                        dropped_samples: 0,
                        capture_issue: None,
                        failure: Some(failure),
                    });
                }
            }
        }
    }
    Ok(FinishedRecording {
        session_id,
        note_id,
        source_mode,
        final_path,
        sources,
        elapsed_ms: recording_dto.elapsed_ms,
        recording: recording_dto,
    })
}

fn write_input_data<I>(
    data: I,
    input: &CaptureInput,
    paused: &Arc<AtomicBool>,
    mic_ducked: &Arc<AtomicBool>,
) where
    I: Iterator<Item = f32>,
{
    // Ducked input becomes silence rather than disappearing so microphone and
    // system Sources remain time-aligned. CaptureInput performs only bounded
    // stack work and atomics; all allocation, preview fan-out, and WAV I/O live
    // on the writer task.
    input.write(data, paused, mic_ducked);
}

fn lock_finalization() -> Result<std::sync::MutexGuard<'static, ()>, AppError> {
    RECORDING_FINALIZATION
        .lock()
        .map_err(|_| AppError::new("recording_lock_failed", "Recording state is unavailable."))
}

fn lock_active() -> Result<std::sync::MutexGuard<'static, Option<ActiveRecording>>, AppError> {
    ACTIVE_RECORDING
        .lock()
        .map_err(|_| AppError::new("recording_lock_failed", "Recording state is unavailable."))
}

fn active_for_session<'a, T>(active: Option<T>, session_id: &str) -> Result<T, AppError>
where
    T: AsRecording<'a>,
{
    let Some(recording) = active else {
        return Err(AppError::new(
            "recording_not_found",
            "Recording session was not found.",
        ));
    };
    if recording.session_id() != session_id {
        return Err(AppError::new(
            "recording_not_found",
            "Recording session was not found.",
        ));
    }
    Ok(recording)
}

trait AsRecording<'a> {
    fn session_id(&self) -> &str;
}

impl<'a> AsRecording<'a> for &'a ActiveRecording {
    fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl<'a> AsRecording<'a> for &'a mut ActiveRecording {
    fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl ActiveRecording {
    fn elapsed(&self) -> Duration {
        if self.paused {
            self.accumulated_active
        } else {
            self.accumulated_active
                + self
                    .active_since
                    .map(|instant| instant.elapsed())
                    .unwrap_or_default()
        }
    }

    fn status(&self) -> RecordingStatusDto {
        self.status_with_state(if self.paused {
            RecordingState::Paused
        } else {
            RecordingState::Recording
        })
    }

    fn status_with_state(&self, state: RecordingState) -> RecordingStatusDto {
        let stats = self.writer.stats();
        let peak = stats.peak;
        let rms = stats.rms;
        let recent_peaks = stats.recent_peaks;
        let writer_health = self.writer.health();
        let bytes_written = writer_health.written_bytes();
        let dropped_samples = stats.dropped_samples;
        let elapsed_ms = self.elapsed().as_millis() as i64;
        let microphone_capture_issue = microphone_writer_issue(elapsed_ms, writer_health.issue)
            .or_else(|| {
                microphone_stream_issue(
                    elapsed_ms,
                    state,
                    &self.stream_error,
                    &self.last_callback_at,
                    self.writer.has_unobserved_callback(),
                )
            });
        latch_stall(&self.stall_latch, microphone_capture_issue.as_ref());
        // A dead stream must not keep animating the last healthy peaks: the
        // waveform reads level, not silence_warning, so zero it on an issue.
        let level = if microphone_capture_issue.is_some() {
            AudioLevelDto {
                peak: 0.0,
                rms: 0.0,
                recent_peaks: Vec::new(),
            }
        } else {
            AudioLevelDto {
                peak,
                rms,
                recent_peaks: recent_peaks.clone(),
            }
        };
        RecordingStatusDto {
            session_id: self.session_id.clone(),
            note_id: Some(self.note_id.clone()),
            source_mode: self.source_mode,
            state,
            elapsed_ms,
            level: level.clone(),
            silence_warning: self.started.elapsed() >= Duration::from_secs(10)
                && rms < DEFAULT_SILENCE_THRESHOLD,
            bytes_written,
            live_preview_enabled: self.live_preview_enabled,
            sources: source_statuses(
                self.source_mode,
                state,
                elapsed_ms,
                MicrophoneStatusInput {
                    level,
                    bytes_written,
                    dropped_samples,
                    silence_warning: self.started.elapsed() >= Duration::from_secs(10)
                        && rms < DEFAULT_SILENCE_THRESHOLD,
                    capture_issue: microphone_capture_issue.clone(),
                },
                self.system_capture.as_ref(),
            ),
            warnings: source_warnings(microphone_capture_issue.as_ref(), dropped_samples),
        }
    }

    fn recovery_snapshot(&mut self, mode: RecoverySnapshotMode) -> CaptureRecoverySnapshot {
        let status = self.status();
        let should_persist = self.recovery_checkpoint(mode).is_some();
        CaptureRecoverySnapshot {
            status,
            should_persist,
        }
    }

    fn recovery_checkpoint(
        &mut self,
        mode: RecoverySnapshotMode,
    ) -> Option<RecordingRecoveryCheckpoint> {
        let elapsed_ms = self.elapsed().as_millis().min(i64::MAX as u128) as i64;
        let should_persist = match mode {
            RecoverySnapshotMode::Force => true,
            RecoverySnapshotMode::Throttled => {
                elapsed_ms - self.last_recovery_snapshot_elapsed_ms
                    >= RECOVERY_SNAPSHOT_INTERVAL.as_millis() as i64
            }
        };
        if !should_persist {
            return None;
        }
        self.last_recovery_snapshot_elapsed_ms = elapsed_ms;
        self.writer.flush_for_recovery();
        let state = if self.paused {
            RecordingState::Paused
        } else {
            RecordingState::Recording
        };
        let capture_issue = microphone_writer_issue(elapsed_ms, self.writer.health().issue)
            .or_else(|| {
                microphone_stream_issue(
                    elapsed_ms,
                    state,
                    &self.stream_error,
                    &self.last_callback_at,
                    self.writer.has_unobserved_callback(),
                )
            });
        latch_stall(&self.stall_latch, capture_issue.as_ref());
        Some(RecordingRecoveryCheckpoint {
            session_id: self.session_id.clone(),
            state,
            elapsed_ms,
            dropped_samples: self.writer.stats().dropped_samples,
            capture_issue,
        })
    }
}

enum RecoverySnapshotMode {
    Throttled,
    Force,
}

fn started_sources(
    source_mode: RecordingSourceMode,
    note_dir: &std::path::Path,
) -> Vec<StartedSource> {
    let mut sources = vec![StartedSource {
        source: RecordingSource::Microphone,
        partial_path: note_dir.join("microphone.partial.wav"),
        final_path: note_dir.join("microphone.wav"),
    }];
    if source_mode == RecordingSourceMode::MicrophonePlusSystem {
        sources.push(StartedSource {
            source: RecordingSource::System,
            partial_path: note_dir.join("system.partial.wav"),
            final_path: note_dir.join("system.wav"),
        });
    }
    sources
}

struct MicrophoneStatusInput {
    level: AudioLevelDto,
    bytes_written: i64,
    dropped_samples: u64,
    silence_warning: bool,
    capture_issue: Option<MicrophoneStreamIssue>,
}

fn source_statuses(
    source_mode: RecordingSourceMode,
    state: RecordingState,
    elapsed_ms: i64,
    microphone: MicrophoneStatusInput,
    system_capture: Option<&SystemAudioCapture>,
) -> Vec<SourceStatusDto> {
    let source_state = match state {
        RecordingState::Paused => SourceState::Paused,
        RecordingState::Validating | RecordingState::Finalizing => SourceState::Finalizing,
        RecordingState::Invalid => SourceState::Invalid,
        RecordingState::Failed => SourceState::Failed,
        RecordingState::Recoverable => SourceState::Recoverable,
        _ => SourceState::Recording,
    };
    let mut sources = vec![SourceStatusDto {
        source: RecordingSource::Microphone,
        state: source_state,
        elapsed_ms,
        bytes_written: microphone.bytes_written,
        dropped_samples: microphone.dropped_samples,
        level: microphone.level,
        silence_warning: microphone.silence_warning || microphone.capture_issue.is_some(),
        path_finalized: false,
        last_error: microphone
            .capture_issue
            .map(|issue| issue.message)
            .or_else(|| {
                (microphone.dropped_samples > 0)
                    .then(|| MICROPHONE_BUFFER_OVERFLOW_WARNING_MESSAGE.to_string())
            }),
    }];
    if source_mode == RecordingSourceMode::MicrophonePlusSystem {
        let (level, bytes_written, last_error) = system_capture
            .map(|capture| capture.status())
            .unwrap_or_default();
        let system_silence_warning = elapsed_ms >= 10_000 && level.peak < DEFAULT_SILENCE_THRESHOLD;
        sources.push(SourceStatusDto {
            source: RecordingSource::System,
            state: source_state,
            elapsed_ms,
            bytes_written,
            dropped_samples: 0,
            level,
            silence_warning: system_silence_warning,
            path_finalized: false,
            last_error,
        });
    }
    sources
}

fn set_final_microphone_bytes(sources: &mut [SourceStatusDto], bytes_written: i64) {
    if let Some(microphone) = sources
        .iter_mut()
        .find(|source| source.source == RecordingSource::Microphone)
    {
        microphone.bytes_written = bytes_written;
    }
}

fn latch_stall(
    latch: &Arc<Mutex<Option<MicrophoneStreamIssue>>>,
    issue: Option<&MicrophoneStreamIssue>,
) {
    let Some(issue) = issue else { return };
    if let Ok(mut latch) = latch.lock() {
        if latch.is_none() {
            *latch = Some(issue.clone());
        }
    }
}

fn microphone_writer_issue(
    elapsed_ms: i64,
    issue: Option<MicrophoneWriterIssue>,
) -> Option<MicrophoneStreamIssue> {
    let code = match issue? {
        MicrophoneWriterIssue::Io => "microphone_writer_io_error",
        MicrophoneWriterIssue::Panicked => "microphone_writer_panicked",
        MicrophoneWriterIssue::StoppedUnexpectedly => "microphone_writer_stopped",
        MicrophoneWriterIssue::Stalled => "microphone_writer_stalled",
    };
    Some(MicrophoneStreamIssue {
        code: code.to_string(),
        message: MICROPHONE_WRITER_WARNING_MESSAGE.to_string(),
        elapsed_ms,
    })
}

fn microphone_stream_issue(
    elapsed_ms: i64,
    state: RecordingState,
    stream_error: &Arc<Mutex<Option<MicrophoneStreamIssue>>>,
    last_callback_at: &Arc<Mutex<Option<Instant>>>,
    callback_ahead_of_monitor: bool,
) -> Option<MicrophoneStreamIssue> {
    if let Ok(issue) = stream_error.lock() {
        if issue.is_some() {
            return issue.clone();
        }
    }
    if state == RecordingState::Paused {
        return None;
    }
    // The WAV task can be delayed by disk I/O. A newer callback heartbeat is
    // direct evidence that capture is still healthy even if the writer has not
    // yet refreshed its non-real-time timestamp.
    if callback_ahead_of_monitor {
        return None;
    }
    let last_callback_at = last_callback_at.lock().ok().and_then(|instant| *instant);
    let stalled = match last_callback_at {
        Some(at) => at.elapsed() > MICROPHONE_STALL_THRESHOLD,
        // No input callback ever fired: a stream that opened but never
        // delivers data is stalled too, measured from recording start.
        None => elapsed_ms > MICROPHONE_STALL_THRESHOLD.as_millis() as i64,
    };
    if !stalled {
        return None;
    }
    Some(MicrophoneStreamIssue {
        code: "microphone_stream_stalled".to_string(),
        message: MICROPHONE_STREAM_WARNING_MESSAGE.to_string(),
        elapsed_ms,
    })
}

fn source_warnings(
    microphone_capture_issue: Option<&MicrophoneStreamIssue>,
    dropped_samples: u64,
) -> Vec<SourceWarningDto> {
    let mut warnings = Vec::new();
    if let Some(issue) = microphone_capture_issue {
        warnings.push(SourceWarningDto {
            source: RecordingSource::Microphone,
            code: issue.code.clone(),
            message: if issue.code.starts_with("microphone_writer_") {
                MICROPHONE_WRITER_WARNING_MESSAGE.to_string()
            } else {
                MICROPHONE_STREAM_WARNING_MESSAGE.to_string()
            },
        });
    }
    if dropped_samples > 0 {
        warnings.push(SourceWarningDto {
            source: RecordingSource::Microphone,
            code: "microphone_buffer_overflow".to_string(),
            message: MICROPHONE_BUFFER_OVERFLOW_WARNING_MESSAGE.to_string(),
        });
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn microphone_permission_mapping_reports_granted_only_for_authorized() {
        let (state, hint) =
            microphone_permission_state_from_authorization(MicrophoneAuthorizationStatus::Granted);

        assert_eq!(state, "granted");
        assert_eq!(hint, None);
    }

    #[test]
    fn microphone_permission_mapping_reports_denied_or_restricted_as_not_granted() {
        let (denied_state, denied_hint) =
            microphone_permission_state_from_authorization(MicrophoneAuthorizationStatus::Denied);
        let (restricted_state, restricted_hint) = microphone_permission_state_from_authorization(
            MicrophoneAuthorizationStatus::Restricted,
        );

        assert_eq!(denied_state, "denied");
        assert!(denied_hint.is_some());
        assert_eq!(restricted_state, "restricted");
        assert!(restricted_hint.is_some());
    }

    #[test]
    fn microphone_permission_mapping_keeps_not_determined_not_ready() {
        let (state, hint) = microphone_permission_state_from_authorization(
            MicrophoneAuthorizationStatus::NotDetermined,
        );

        assert_eq!(state, "not_determined");
        assert!(hint.is_some());
    }

    #[test]
    fn microphone_permission_mapping_keeps_unknown_not_ready() {
        let (state, hint) =
            microphone_permission_state_from_authorization(MicrophoneAuthorizationStatus::Unknown);

        assert_eq!(state, "unknown");
        assert!(hint.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn video_control_is_visible_before_audio_pause_without_worker_progress() {
        use std::{io::Read, os::unix::fs::OpenOptionsExt};
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("window.partial.mp4");
        let video = video::VideoCapture::test_control_pipe(&partial);
        let mut reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(partial.with_extension("control.pipe"))
            .unwrap();
        let audio_paused = AtomicBool::new(false);
        publish_video_before_audio(Some(&video), true, || {
            assert!(!audio_paused.load(Ordering::Acquire));
            let mut event = [0_u8; 24];
            reader.read_exact(&mut event).unwrap();
            assert_eq!(u64::from_le_bytes(event[16..24].try_into().unwrap()), 1);
            audio_paused.store(true, Ordering::Release);
        });
        assert!(audio_paused.load(Ordering::Acquire));
        publish_video_before_audio(Some(&video), false, || {
            let mut event = [0_u8; 24];
            reader.read_exact(&mut event).unwrap();
            assert_eq!(u64::from_le_bytes(event[16..24].try_into().unwrap()), 0);
            assert!(audio_paused.load(Ordering::Acquire));
            audio_paused.store(false, Ordering::Release);
        });
    }

    #[cfg(unix)]
    #[test]
    fn video_control_failure_never_prevents_audio_transition() {
        let dir = tempfile::tempdir().unwrap();
        let video =
            video::VideoCapture::test_control_pipe(&dir.path().join("absent/session.partial.mp4"));
        let audio_paused = AtomicBool::new(false);
        publish_video_before_audio(Some(&video), true, || {
            audio_paused.store(true, Ordering::Release)
        });
        assert!(audio_paused.load(Ordering::Acquire));
        publish_video_before_audio(Some(&video), false, || {
            audio_paused.store(false, Ordering::Release)
        });
        assert!(!audio_paused.load(Ordering::Acquire));
    }

    fn test_writer(dir: &tempfile::TempDir) -> (CaptureInput, MicrophoneWriter) {
        let writer = WavWriter::create(
            dir.path().join("mic.wav"),
            WavSpec {
                channels: 1,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            },
        )
        .expect("create wav writer");
        MicrophoneWriter::start(writer, 48_000, 1, None, Arc::new(Mutex::new(None)))
            .expect("start microphone writer")
    }

    /// A ducked mic writes SILENCE, never drops frames: the sample count must
    /// match the input exactly (the mic track has to stay time-aligned with a
    /// concurrently-rolling system track) while every written sample is zero.
    #[test]
    fn ducked_mic_writes_aligned_silence_not_dropped_frames() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (input, writer) = test_writer(&dir);
        let paused = Arc::new(AtomicBool::new(false));
        let ducked = Arc::new(AtomicBool::new(true));

        write_input_data([0.5_f32, -0.25, 0.9].into_iter(), &input, &paused, &ducked);

        let stats = input.stats();
        assert_eq!(
            stats.samples, 3,
            "every input frame is written while ducked"
        );
        // The writer is 16-bit PCM (bits_per_sample: 16), so 2 bytes/sample.
        const BYTES_PER_SAMPLE: i64 = 2;
        assert_eq!(
            stats.samples as i64 * BYTES_PER_SAMPLE,
            3 * BYTES_PER_SAMPLE
        );
        assert_eq!(stats.peak, 0.0, "ducked samples are pure silence");
        writer.finish().expect("finish WAV writer");
        let samples = hound::WavReader::open(dir.path().join("mic.wav"))
            .expect("open ducked WAV")
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .expect("read ducked samples");
        assert_eq!(samples, vec![0, 0, 0]);
    }

    #[test]
    fn unducked_mic_records_real_samples() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (input, writer) = test_writer(&dir);
        let paused = Arc::new(AtomicBool::new(false));
        let ducked = Arc::new(AtomicBool::new(false));

        write_input_data([0.5_f32, -0.25].into_iter(), &input, &paused, &ducked);

        let stats = input.stats();
        assert_eq!(stats.samples, 2);
        assert!(stats.peak > 0.4, "real samples keep their level");
        writer.finish().expect("finish WAV writer");
    }

    /// Pause drops frames outright (both tracks stop together) — the duck
    /// must not change that precedence.
    #[test]
    fn paused_mic_still_drops_frames_even_when_ducked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (input, writer) = test_writer(&dir);
        let paused = Arc::new(AtomicBool::new(true));
        let ducked = Arc::new(AtomicBool::new(true));

        write_input_data([0.5_f32].into_iter(), &input, &paused, &ducked);

        assert_eq!(input.stats().samples, 0);
        writer.finish().expect("finish WAV writer");
    }

    fn issue_state_at(
        elapsed_ms: i64,
        issue: Option<MicrophoneStreamIssue>,
        last_callback_at: Option<Instant>,
        state: RecordingState,
    ) -> Option<MicrophoneStreamIssue> {
        microphone_stream_issue(
            elapsed_ms,
            state,
            &Arc::new(Mutex::new(issue)),
            &Arc::new(Mutex::new(last_callback_at)),
            false,
        )
    }

    fn issue_state(
        issue: Option<MicrophoneStreamIssue>,
        last_callback_at: Option<Instant>,
        state: RecordingState,
    ) -> Option<MicrophoneStreamIssue> {
        issue_state_at(12_345, issue, last_callback_at, state)
    }

    #[test]
    fn stall_latch_keeps_first_stall_after_callbacks_resume() {
        let latch = Arc::new(Mutex::new(None));
        let stale_callback = Instant::now() - (MICROPHONE_STALL_THRESHOLD + Duration::from_secs(1));

        // A stall is observed and latched.
        let stalled = issue_state(None, Some(stale_callback), RecordingState::Recording)
            .expect("stale callback reports a stall");
        latch_stall(&latch, Some(&stalled));

        // Callbacks resume: the live issue clears, the latch does not.
        let recovered = issue_state(None, Some(Instant::now()), RecordingState::Recording);
        assert_eq!(recovered, None);
        latch_stall(&latch, recovered.as_ref());
        let latched = latch.lock().unwrap().clone().expect("latch persists");
        assert_eq!(latched.code, "microphone_stream_stalled");

        // A later different issue does not overwrite the first.
        let later = MicrophoneStreamIssue {
            code: "microphone_stream_error".to_string(),
            message: "later".to_string(),
            elapsed_ms: 99,
        };
        latch_stall(&latch, Some(&later));
        assert_eq!(
            latch.lock().unwrap().clone().unwrap().code,
            "microphone_stream_stalled"
        );
    }

    #[test]
    fn microphone_stream_error_appears_in_status_source_and_warnings() {
        let issue = MicrophoneStreamIssue {
            code: "microphone_stream_error".to_string(),
            message: "Microphone input stopped unexpectedly: device disconnected".to_string(),
            elapsed_ms: 2_000,
        };

        let sources = source_statuses(
            RecordingSourceMode::MicrophoneOnly,
            RecordingState::Recording,
            2_000,
            MicrophoneStatusInput {
                level: AudioLevelDto::default(),
                bytes_written: 1_024,
                dropped_samples: 0,
                silence_warning: false,
                capture_issue: Some(issue.clone()),
            },
            None,
        );
        let warnings = source_warnings(Some(&issue), 0);

        let microphone = sources
            .iter()
            .find(|source| source.source == RecordingSource::Microphone)
            .expect("microphone source status exists");
        assert_eq!(microphone.last_error, Some(issue.message));
        assert!(microphone.silence_warning);
        assert_eq!(
            warnings,
            vec![SourceWarningDto {
                source: RecordingSource::Microphone,
                code: "microphone_stream_error".to_string(),
                message: MICROPHONE_STREAM_WARNING_MESSAGE.to_string(),
            }]
        );
    }

    #[test]
    fn microphone_writer_failure_appears_in_status_and_warnings() {
        let issue = microphone_writer_issue(2_000, Some(MicrophoneWriterIssue::Io))
            .expect("writer failure becomes a capture issue");
        let sources = source_statuses(
            RecordingSourceMode::MicrophoneOnly,
            RecordingState::Recording,
            2_000,
            MicrophoneStatusInput {
                level: AudioLevelDto::default(),
                bytes_written: 512,
                dropped_samples: 0,
                silence_warning: false,
                capture_issue: Some(issue.clone()),
            },
            None,
        );
        let warnings = source_warnings(Some(&issue), 0);

        assert_eq!(sources[0].bytes_written, 512);
        assert_eq!(
            sources[0].last_error.as_deref(),
            Some(MICROPHONE_WRITER_WARNING_MESSAGE)
        );
        assert!(sources[0].silence_warning);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "microphone_writer_io_error");
        assert_eq!(warnings[0].message, MICROPHONE_WRITER_WARNING_MESSAGE);

        let panic_issue = microphone_writer_issue(2_500, Some(MicrophoneWriterIssue::Panicked))
            .expect("worker panic becomes a capture issue");
        assert_eq!(panic_issue.code, "microphone_writer_panicked");
        assert_eq!(panic_issue.message, MICROPHONE_WRITER_WARNING_MESSAGE);
    }

    #[test]
    fn buffer_overflow_count_appears_in_status_and_warnings() {
        let sources = source_statuses(
            RecordingSourceMode::MicrophoneOnly,
            RecordingState::Recording,
            2_000,
            MicrophoneStatusInput {
                level: AudioLevelDto::default(),
                bytes_written: 1_024,
                dropped_samples: 37,
                silence_warning: false,
                capture_issue: None,
            },
            None,
        );
        let warnings = source_warnings(None, 37);

        assert_eq!(sources[0].dropped_samples, 37);
        assert_eq!(
            sources[0].last_error.as_deref(),
            Some(MICROPHONE_BUFFER_OVERFLOW_WARNING_MESSAGE)
        );
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "microphone_buffer_overflow");
    }

    #[test]
    fn final_microphone_bytes_replace_the_pre_drain_snapshot_only_for_the_microphone() {
        let mut sources = source_statuses(
            RecordingSourceMode::MicrophonePlusSystem,
            RecordingState::Finalizing,
            2_000,
            MicrophoneStatusInput {
                level: AudioLevelDto::default(),
                bytes_written: 128,
                dropped_samples: 0,
                silence_warning: false,
                capture_issue: None,
            },
            None,
        );
        let system_bytes = sources[1].bytes_written;

        set_final_microphone_bytes(&mut sources, 4_096);

        assert_eq!(sources[0].bytes_written, 4_096);
        assert_eq!(
            sources[1].bytes_written, system_bytes,
            "microphone drain progress must not replace system-source metadata"
        );
    }

    #[test]
    fn microphone_stall_requires_unpaused_recording_past_threshold_after_callback() {
        let recent_callback = Instant::now() - (MICROPHONE_STALL_THRESHOLD / 2);
        let stale_callback = Instant::now() - (MICROPHONE_STALL_THRESHOLD + Duration::from_secs(1));

        // Never-delivered first callback: healthy only within the startup
        // window, stalled once the recording elapsed passes the threshold.
        assert_eq!(
            issue_state_at(1_000, None, None, RecordingState::Recording),
            None
        );
        let never_started = issue_state_at(12_345, None, None, RecordingState::Recording)
            .expect("zero-callback stream past the threshold should report a stall");
        assert_eq!(never_started.code, "microphone_stream_stalled");
        assert_eq!(
            issue_state_at(12_345, None, None, RecordingState::Paused),
            None
        );
        assert_eq!(
            issue_state(None, Some(recent_callback), RecordingState::Recording),
            None
        );

        let stalled = issue_state(None, Some(stale_callback), RecordingState::Recording)
            .expect("stale active stream should report a stall");
        assert_eq!(stalled.code, "microphone_stream_stalled");
        assert_eq!(stalled.message, MICROPHONE_STREAM_WARNING_MESSAGE);
        assert_eq!(stalled.elapsed_ms, 12_345);
    }

    #[test]
    fn paused_recording_suppresses_microphone_stall_warning() {
        let stale_callback = Instant::now() - (MICROPHONE_STALL_THRESHOLD + Duration::from_secs(1));

        assert_eq!(
            issue_state(None, Some(stale_callback), RecordingState::Paused),
            None
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod avfoundation_link_tests {
    // If AVFoundation stops being linked, AnyClass::get returns None and
    // readiness silently degrades to "unknown" (never granted) — this probe
    // turns that silent degradation into a test failure.
    #[test]
    fn avcapturedevice_class_resolves() {
        assert!(super::macos_microphone_authorization_status().is_some());
    }
}

#[cfg(test)]
mod host_admission_tests {
    use super::*;
    use crate::domain::turn_authority::TurnAuthority;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn revoked_start_waiting_for_final_lock_never_starts_or_publishes() {
        let authority = TurnAuthority::new();
        let authority_for_gate = authority.clone();
        let gate: CaptureStartAuthorization = Arc::new(move || authority_for_gate.check_live());
        let effects = Arc::new(AtomicUsize::new(0));
        let final_lock = Arc::new(Mutex::new(()));
        let held = final_lock.lock().unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let worker = {
            let final_lock = final_lock.clone();
            let effects = effects.clone();
            std::thread::spawn(move || {
                entered_tx.send(()).unwrap();
                let _final_lock = final_lock.lock().unwrap();
                let mut state = CaptureStartState::Pending;
                let result = admit_capture_start(&mut state, false, Some(&gate), || {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                });
                (state, result)
            })
        };
        entered_rx.recv().unwrap();
        authority.revoke();
        drop(held);
        let (state, result) = worker.join().unwrap();
        assert_eq!(result.unwrap_err().code, "turn_expired");
        assert_eq!(state, CaptureStartState::Pending);
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn live_start_publishes_once_and_later_revocation_does_not_undo_the_effect() {
        let authority = TurnAuthority::new();
        let copy = authority.clone();
        let gate: CaptureStartAuthorization = Arc::new(move || copy.check_live());
        let effects = AtomicUsize::new(0);
        let mut state = CaptureStartState::Pending;
        admit_capture_start(&mut state, false, Some(&gate), || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        authority.revoke();
        assert_eq!(state, CaptureStartState::Published);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn watchdog_abandonment_cannot_cross_the_same_effect_gate() {
        let mut state = CaptureStartState::Abandoned;
        let mut effects = 0;
        let result = admit_capture_start(&mut state, false, None, || {
            effects += 1;
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, "capture_start_timeout");
        assert_eq!(effects, 0);
    }
}
