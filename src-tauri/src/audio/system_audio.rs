use crate::domain::types::AppError;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct SystemAudioFailure {
    pub code: String,
    pub message: String,
}

impl From<AppError> for SystemAudioFailure {
    fn from(error: AppError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

pub enum SystemAudioStopResult {
    Failed(SystemAudioFailure),
    Stopped(PathBuf),
}

/// The host grant is checked at the platform's prepared-to-start boundary.
pub(crate) fn admit_system_audio_start<T>(
    authorization: Option<&crate::audio::capture::CaptureStartAuthorization>,
    effect: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    if let Some(authorization) = authorization {
        authorization()?;
    }
    effect()
}

#[cfg(target_os = "macos")]
pub use crate::audio::system_macos::{
    helper_permission_check, system_audio_readiness, SystemAudioCapture,
    SYSTEM_AUDIO_PERMISSION_PROBE_TIMEOUT,
};

#[cfg(target_os = "windows")]
pub use crate::audio::system_windows::{
    helper_permission_check, system_audio_readiness, SystemAudioCapture,
    SYSTEM_AUDIO_PERMISSION_PROBE_TIMEOUT,
};

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
pub use unsupported::{
    helper_permission_check, system_audio_readiness, SystemAudioCapture,
    SYSTEM_AUDIO_PERMISSION_PROBE_TIMEOUT,
};

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
mod unsupported {
    use super::SystemAudioStopResult;
    use crate::domain::types::{AppError, AudioLevelDto, RecordingSource, SourceReadinessDto};
    use std::{path::PathBuf, time::Duration};

    pub const SYSTEM_AUDIO_PERMISSION_PROBE_TIMEOUT: Duration = Duration::from_secs(1);

    pub struct SystemAudioCapture;

    impl SystemAudioCapture {
        pub fn start(
            _partial_path: PathBuf,
            _final_path: PathBuf,
            _timeline_offset: Duration,
            _authorization: Option<crate::audio::capture::CaptureStartAuthorization>,
        ) -> Result<Self, AppError> {
            Err(unsupported_error())
        }

        pub fn pause(&mut self) {}

        pub fn resume(&mut self) {}

        pub fn status(&self) -> (AudioLevelDto, i64, Option<String>) {
            (AudioLevelDto::default(), 0, Some(unsupported_message()))
        }

        pub fn stop(self) -> SystemAudioStopResult {
            SystemAudioStopResult::Failed(unsupported_error().into())
        }
    }

    pub fn system_audio_readiness() -> SourceReadinessDto {
        SourceReadinessDto {
            source: RecordingSource::System,
            required: true,
            ready: false,
            permission_state: "unsupported".to_string(),
            device_available: false,
            capture_available: false,
            recovery_action: None,
            message: Some(unsupported_message()),
        }
    }

    pub fn helper_permission_check() -> Result<(), AppError> {
        Err(unsupported_error())
    }

    fn unsupported_error() -> AppError {
        AppError::new("system_audio_unsupported", unsupported_message())
    }

    fn unsupported_message() -> String {
        "System audio capture is not supported on this platform.".to_string()
    }
}

#[cfg(test)]
mod guarded_platform_start_tests {
    use super::*;
    use crate::audio::capture::CaptureStartAuthorization;
    use crate::domain::turn_authority::TurnAuthority;
    use std::sync::{Arc, Mutex};
    #[test]
    fn revoked_after_platform_preparation_cannot_dispatch_the_start() {
        let authority = TurnAuthority::new();
        let copy = authority.clone();
        let authorization: CaptureStartAuthorization = Arc::new(move || copy.check_live());
        let preparation = Arc::new(Mutex::new(()));
        let held = preparation.lock().unwrap();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let dispatches = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker = {
            let preparation = preparation.clone();
            let dispatches = dispatches.clone();
            std::thread::spawn(move || {
                entered_tx.send(()).unwrap();
                let _prepared = preparation.lock().unwrap();
                admit_system_audio_start(Some(&authorization), || {
                    dispatches.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
            })
        };
        entered_rx.recv().unwrap();
        authority.revoke();
        drop(held);
        assert_eq!(worker.join().unwrap().unwrap_err().code, "turn_expired");
        assert_eq!(dispatches.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
