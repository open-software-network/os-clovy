//! The in-app updater: one stable channel, release builds only.
//!
//! The manifest endpoint and the public key live in `plugins.updater` in
//! `tauri.conf.json` (the owner's own releases, ADR-0069), so this module
//! never names a URL. A development build never checks or installs: it must
//! not pull a release over the development bundle.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_updater::UpdaterExt;

use crate::domain::types::AppError;
use crate::update_gate::UPDATE_GATE;

/// How often an install waiting for a capture checks again.
const CAPTURE_POLL: Duration = Duration::from_millis(500);

/// A meeting recording runs or is still writing its files. Dictation takes
/// are tracked inside the gate itself.
fn capture_active() -> bool {
    crate::audio::capture::is_capture_active() || crate::audio::capture::is_capture_finalizing()
}

/// Whether this build checks for and installs updates. The frontend reads it
/// to decide whether to check at launch and to show the Settings action.
#[tauri::command]
pub fn updates_enabled() -> bool {
    !cfg!(debug_assertions)
}

/// Registers the slot for the update found by the last check.
pub fn setup(app: &mut tauri::App) {
    app.manage(PendingUpdate::default());
}

/// What `fetch_update` reports to the frontend: just enough to offer it.
/// Mirrors the fields `update-decision.ts` reads off an update (`version`,
/// `body`); the live `Update` handle stays in Rust (see `PendingUpdate`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMeta {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// Download progress streamed to the frontend over an IPC `Channel`. The shape
/// is adjacently tagged (`{ event, data }`) so the frontend throttling can read
/// it as an event stream.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum DownloadEvent {
    #[serde(rename_all = "camelCase")]
    Started {
        #[serde(skip_serializing_if = "Option::is_none")]
        content_length: Option<u64>,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        chunk_length: usize,
    },
    Finished,
    /// The package is downloaded and waits for a recording or dictation to
    /// end before it installs.
    WaitingForCapture,
    /// The install holds its slot: it can no longer be postponed, and on
    /// Windows the app exits as it finishes.
    Installing,
}

/// Holds the `Update` returned by the most recent `fetch_update` so that a
/// follow-up `install_update` can run it. The handle is not serializable and
/// must stay Rust-side, so the frontend drives install through commands rather
/// than holding the update itself.
#[derive(Default)]
pub struct PendingUpdate(pub Mutex<Option<tauri_plugin_updater::Update>>);

fn disabled(code: &str) -> AppError {
    AppError::new(code, "Updates are off in development builds.")
}

/// Checks the configured manifest for a newer version. The plugin verifies
/// the package signature against the configured public key at install time,
/// so a package signed by another key never installs. The found `Update` is
/// stashed for `install_update`; a `None` result clears any stale handle.
#[tauri::command]
pub async fn fetch_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<Option<UpdateMeta>, AppError> {
    if !updates_enabled() {
        return Err(disabled("update_check_failed"));
    }
    let updater = app.updater_builder();
    #[cfg(windows)]
    let updater = {
        let exit_app = app.clone();
        updater.on_before_exit(move || {
            crate::shutdown::finalize_updater_exit(&exit_app);
        })
    };
    let update = updater
        .build()
        .map_err(|error| AppError::new("update_check_failed", error.to_string()))?
        .check()
        .await
        .map_err(|error| AppError::new("update_check_failed", error.to_string()))?;

    let meta = update.as_ref().map(|update| UpdateMeta {
        version: update.version.clone(),
        body: update.body.clone(),
    });

    *pending
        .0
        .lock()
        .map_err(|_| AppError::new("update_check_failed", "Update lock failed."))? = update;

    Ok(meta)
}

/// Downloads the update staged by `fetch_update`, waits for any recording or
/// dictation to end, then installs it. Progress and the wait stream over
/// `on_event`. The update is consumed; a failed or postponed install requires a
/// fresh `fetch_update` (the frontend re-checks before retrying).
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
    on_event: Channel<DownloadEvent>,
) -> Result<(), AppError> {
    if !updates_enabled() {
        return Err(disabled("update_install_failed"));
    }
    let update = pending
        .0
        .lock()
        .map_err(|_| AppError::new("update_install_failed", "Update lock failed."))?
        .take()
        .ok_or_else(|| AppError::new("update_install_failed", "No update is staged."))?;
    UPDATE_GATE.begin_install_attempt();

    let progress_channel = on_event.clone();
    let finished_channel = on_event.clone();
    let waiting_channel = on_event.clone();
    let installing_channel = on_event;
    let mut started = false;

    // Download first: installing exits the process on Windows (and the macOS
    // restart follows it), so it must not start while a capture runs.
    let bytes = update
        .download(
            move |chunk_length, content_length| {
                // The Rust API has no separate "started" callback, so
                // synthesize it from the first chunk.
                if !started {
                    started = true;
                    let _ = progress_channel.send(DownloadEvent::Started { content_length });
                }
                let _ = progress_channel.send(DownloadEvent::Progress { chunk_length });
            },
            move || {
                let _ = finished_channel.send(DownloadEvent::Finished);
            },
        )
        .await
        .map_err(|error| AppError::new("update_install_failed", error.to_string()))?;

    // Held until the process exits; a failed install drops it and captures
    // can start again.
    let _install_slot = UPDATE_GATE
        .wait_for_install_slot(capture_active, CAPTURE_POLL, move || {
            let _ = waiting_channel.send(DownloadEvent::WaitingForCapture);
        })
        .await
        .map_err(|_| AppError::new("update_install_postponed", "The update was postponed."))?;
    let _ = installing_channel.send(DownloadEvent::Installing);

    #[cfg(windows)]
    {
        // `Update::install` exits the process and its before-exit callback
        // cannot cancel that action. Finish Clovy's fallible cleanup while an
        // error can still keep the renderer alive.
        crate::shutdown::prepare_for_updater_exit(&app)?;
        if let Err(error) = update.install(bytes) {
            crate::shutdown::cancel_updater_exit(&app);
            return Err(AppError::new("update_install_failed", error.to_string()));
        }
    }

    #[cfg(not(windows))]
    {
        let _ = &app;
        update
            .install(bytes)
            .map_err(|error| AppError::new("update_install_failed", error.to_string()))?;
    }

    Ok(())
}

/// Stops an install that is waiting for a capture to end. The downloaded
/// package is dropped; the next attempt checks again. Refused once the
/// install has begun.
#[tauri::command]
pub fn postpone_update_install() -> Result<(), AppError> {
    UPDATE_GATE.postpone_install()
}

/// Relaunches Clovy after an in-app update has been staged through the same
/// idempotent shutdown coordinator used by ordinary app quit.
///
/// A plain process relaunch (and `AppHandle::restart()` on the main thread)
/// restarts without a guaranteed pass through the `RunEvent::Exit` cleanup that
/// reaps Clovy's children. On an update the `.app` bundle is swapped, so a
/// skipped teardown orphans the dictation helper, which keeps the global
/// CGEventTap and its stdio, and the relaunched instance then cannot bring up a
/// clean helper. The coordinator latches Restart, performs teardown off the
/// main event loop, and schedules the final restart on the main thread after
/// cleanup or its hard aggregate deadline. The command resolves with that
/// outcome: a restart cancelled by a failed note-save barrier rejects with
/// `update_relaunch_cancelled`, and the app stays open for another try.
#[tauri::command]
pub async fn relaunch_for_update(app: AppHandle) -> Result<(), AppError> {
    if !updates_enabled() {
        return Err(disabled("update_relaunch_failed"));
    }
    // The restart ends the process too: no capture may run or start until it
    // finalizes. A cancelled restart drops the slot and resolves as an error.
    let _restart_slot = UPDATE_GATE.try_begin_install(capture_active).map_err(|_| {
        AppError::new(
            "update_capture_active",
            "A recording or dictation is running. Restart when it ends.",
        )
    })?;
    crate::shutdown::request_restart(&app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_are_disabled_in_dev_builds() {
        // Tests run as a development build: it never checks or installs.
        const { assert!(cfg!(debug_assertions)) };
        assert!(!updates_enabled());
    }

    #[test]
    fn the_endpoint_lives_only_in_the_tauri_config() {
        let source = include_str!("updates.rs");
        assert!(!source.contains(concat!("https", "://")));
        assert!(!source.contains(concat!("Release", "Channel")));
    }

    // The frontend's download-progress throttling (update-decision.ts) reads
    // `event.event` and `event.data.contentLength` / `event.data.chunkLength`
    // verbatim, so these serde shapes are a hard wire contract, not cosmetics.
    #[test]
    fn started_event_carries_content_length_under_data() {
        let json = serde_json::to_string(&DownloadEvent::Started {
            content_length: Some(100),
        })
        .unwrap();
        assert_eq!(json, r#"{"event":"Started","data":{"contentLength":100}}"#);
    }

    #[test]
    fn started_event_omits_content_length_when_unknown() {
        let json = serde_json::to_string(&DownloadEvent::Started {
            content_length: None,
        })
        .unwrap();
        assert_eq!(json, r#"{"event":"Started","data":{}}"#);
    }

    #[test]
    fn progress_event_carries_chunk_length_under_data() {
        let json = serde_json::to_string(&DownloadEvent::Progress { chunk_length: 5 }).unwrap();
        assert_eq!(json, r#"{"event":"Progress","data":{"chunkLength":5}}"#);
    }

    #[test]
    fn finished_event_has_no_data_field() {
        let json = serde_json::to_string(&DownloadEvent::Finished).unwrap();
        assert_eq!(json, r#"{"event":"Finished"}"#);
    }

    #[test]
    fn installing_event_has_no_data_field() {
        let json = serde_json::to_string(&DownloadEvent::Installing).unwrap();
        assert_eq!(json, r#"{"event":"Installing"}"#);
    }

    #[test]
    fn waiting_for_capture_event_has_no_data_field() {
        let json = serde_json::to_string(&DownloadEvent::WaitingForCapture).unwrap();
        assert_eq!(json, r#"{"event":"WaitingForCapture"}"#);
    }

    #[test]
    fn update_meta_exposes_version_and_notes_to_the_frontend() {
        let json = serde_json::to_string(&UpdateMeta {
            version: "0.1.1".into(),
            body: Some("notes".into()),
        })
        .unwrap();
        assert_eq!(json, r#"{"version":"0.1.1","body":"notes"}"#);
    }

    #[test]
    fn update_meta_omits_absent_notes() {
        let json = serde_json::to_string(&UpdateMeta {
            version: "0.1.1".into(),
            body: None,
        })
        .unwrap();
        assert_eq!(json, r#"{"version":"0.1.1"}"#);
    }
}
