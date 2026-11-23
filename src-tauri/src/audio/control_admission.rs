use crate::domain::types::AppError;
use std::sync::{Arc, Mutex};

pub(crate) type CaptureStartAuthorization = Arc<dyn Fn() -> Result<(), AppError> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStartState {
    Pending,
    Published,
    Abandoned,
}

pub(crate) fn reserve_capture_start(
    reservation: &Mutex<()>,
) -> Result<std::sync::MutexGuard<'_, ()>, AppError> {
    reservation.try_lock().map_err(|_| {
        AppError::new(
            "recording_already_active",
            "A previous recording is still active. Clovy attempted to save it locally; please try again.",
        )
    })
}

pub(crate) fn start_native_capture<T>(
    active: &Mutex<Option<T>>,
    handshake: &Mutex<CaptureStartState>,
    authorization: Option<&CaptureStartAuthorization>,
    start: impl FnOnce() -> Result<(), AppError>,
) -> Result<(), AppError> {
    {
        let active = active.lock().map_err(|_| {
            AppError::new("recording_lock_failed", "Recording state is unavailable.")
        })?;
        let state = handshake.lock().map_err(|_| {
            AppError::new("recording_lock_failed", "Recording state is unavailable.")
        })?;
        if *state == CaptureStartState::Abandoned {
            return Err(AppError::new(
                "capture_start_timeout",
                "Could not start the microphone. Try again, or check the selected input device.",
            ));
        }
        if active.is_some() {
            return Err(AppError::new("recording_already_active", "A previous recording is still active. Clovy attempted to save it locally; please try again."));
        }
        if let Some(authorization) = authorization {
            authorization()?;
        }
    }
    start()
}

pub(crate) fn publish_capture_start(
    state: &mut CaptureStartState,
    already_active: bool,
    authorization: Option<&CaptureStartAuthorization>,
) -> Result<(), AppError> {
    if *state == CaptureStartState::Abandoned {
        return Err(AppError::new(
            "capture_start_timeout",
            "Could not start the microphone. Try again, or check the selected input device.",
        ));
    }
    if already_active {
        return Err(AppError::new("recording_already_active", "A previous recording is still active. Clovy attempted to save it locally; please try again."));
    }
    if let Some(authorization) = authorization {
        authorization()?;
    }
    *state = CaptureStartState::Published;
    Ok(())
}

pub(crate) fn with_control_admission<T, R>(
    state: &Mutex<T>,
    authorization: Option<&CaptureStartAuthorization>,
    effect: impl FnOnce(&mut T) -> Result<R, AppError>,
) -> Result<R, AppError> {
    let mut state = state
        .lock()
        .map_err(|_| AppError::new("recording_lock_failed", "Recording state is unavailable."))?;
    if let Some(authorization) = authorization {
        authorization()?;
    }
    effect(&mut state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::turn_authority::TurnAuthority;

    #[test]
    fn blocked_native_start_does_not_hold_watchdog_or_active_locks() {
        let active = Arc::new(Mutex::new(None::<()>));
        let handshake = Arc::new(Mutex::new(CaptureStartState::Pending));
        let reservation = Arc::new(Mutex::new(()));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = {
            let active = active.clone();
            let handshake = handshake.clone();
            let reservation = reservation.clone();
            std::thread::spawn(move || {
                let _reserved = reserve_capture_start(&reservation).unwrap();
                start_native_capture(&active, &handshake, None, || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
                let mut active = active.lock().unwrap();
                let mut state = handshake.lock().unwrap();
                let result = publish_capture_start(&mut state, active.is_some(), None);
                if result.is_ok() {
                    *active = Some(());
                }
                result
            })
        };
        entered_rx.recv().unwrap();
        *handshake.try_lock().expect("watchdog blocked") = CaptureStartState::Abandoned;
        assert!(active.try_lock().expect("active control blocked").is_none());
        assert_eq!(
            reserve_capture_start(&reservation).unwrap_err().code,
            "recording_already_active"
        );
        release_tx.send(()).unwrap();
        assert_eq!(
            worker.join().unwrap().unwrap_err().code,
            "capture_start_timeout"
        );
        assert!(active.lock().unwrap().is_none());
        assert!(reserve_capture_start(&reservation).is_ok());
    }
    #[test]
    fn abandoned_or_revoked_start_never_calls_native_start() {
        let active = Mutex::new(None::<()>);
        let handshake = Mutex::new(CaptureStartState::Abandoned);
        let called = std::sync::atomic::AtomicBool::new(false);
        let result = start_native_capture(&active, &handshake, None, || {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, "capture_start_timeout");
        *handshake.lock().unwrap() = CaptureStartState::Pending;
        let authority = TurnAuthority::new();
        authority.revoke();
        let gate: CaptureStartAuthorization = Arc::new(move || authority.check_live());
        let result = start_native_capture(&active, &handshake, Some(&gate), || {
            called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(result.unwrap_err().code, "turn_expired");
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }
    #[test]
    fn successful_start_publishes_once() {
        let active = Mutex::new(None::<()>);
        let handshake = Mutex::new(CaptureStartState::Pending);
        let calls = std::sync::atomic::AtomicUsize::new(0);
        start_native_capture(&active, &handshake, None, || {
            calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        let mut active = active.lock().unwrap();
        let mut state = handshake.lock().unwrap();
        publish_capture_start(&mut state, active.is_some(), None).unwrap();
        *active = Some(());
        assert_eq!(*state, CaptureStartState::Published);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn revocation_during_lock_wait_prevents_the_control_effect() {
        let state = Arc::new(Mutex::new(false));
        let held = state.lock().unwrap();
        let authority = TurnAuthority::new();
        let check = authority.clone();
        let gate: CaptureStartAuthorization = Arc::new(move || check.check_live());
        let (started, waiting) = std::sync::mpsc::channel();
        let worker = {
            let state = state.clone();
            std::thread::spawn(move || {
                started.send(()).unwrap();
                with_control_admission(&state, Some(&gate), |paused| {
                    *paused = true;
                    Ok(())
                })
            })
        };
        waiting.recv().unwrap();
        authority.revoke();
        drop(held);
        assert_eq!(worker.join().unwrap().unwrap_err().code, "turn_expired");
        assert!(!*state.lock().unwrap());
        with_control_admission(&state, None, |paused| {
            *paused = true;
            Ok(())
        })
        .unwrap();
        assert!(*state.lock().unwrap());
    }
}
