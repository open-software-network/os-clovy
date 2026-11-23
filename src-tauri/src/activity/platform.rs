//! The OS seam of activity capture. The engine (`engine.rs`) owns every
//! privacy and scheduling decision; a platform only describes windows, reads
//! text, and buffers raw input. Keeping the decisions out of this trait lets
//! the engine run against a scripted fake in tests and keeps pixels inside the
//! platform implementation: nothing an implementation returns is an image.

use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;

/// Live TCC state of one permission.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionState {
    Granted,
    Denied,
    /// The system has not asked yet (Input Monitoring reports this).
    NotDetermined,
    /// The platform has no such permission or capture is unsupported.
    Unsupported,
}

impl PermissionState {
    pub fn is_granted(self) -> bool {
        self == PermissionState::Granted
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionSnapshot {
    pub accessibility: PermissionState,
    pub screen_recording: PermissionState,
    pub input_monitoring: PermissionState,
}

impl PermissionSnapshot {
    pub const UNSUPPORTED: PermissionSnapshot = PermissionSnapshot {
        accessibility: PermissionState::Unsupported,
        screen_recording: PermissionState::Unsupported,
        input_monitoring: PermissionState::Unsupported,
    };
}

/// A permission the user can grant from the Activity tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActivityPermission {
    Accessibility,
    ScreenRecording,
    InputMonitoring,
}

/// What the platform knows about a window before any text is read. The engine
/// filters on this descriptor first, so excluded apps, ignored domains, and
/// private windows never have their text extracted at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowDescriptor {
    pub pid: i32,
    /// `NSRunningApplication.localizedName`.
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
    /// Address of the focused browser tab, when the app exposes one.
    pub browser_url: Option<String>,
    /// Incognito/private browsing window. Never captured, not even by OCR.
    pub private_window: bool,
    /// CGWindowID, needed for OCR of exactly this window.
    pub window_id: Option<u32>,
    /// CGDirectDisplayID of the display the window sits on.
    pub display_id: Option<u32>,
}

/// Raw input as the platform observed it. The key variant can carry typed
/// characters on platforms that expose them; the normalizer (`input.rs`)
/// discards them before anything is stored. The macOS event tap never reads
/// them in the first place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawInputEvent {
    Click {
        at: DateTime<Utc>,
    },
    Key {
        at: DateTime<Utc>,
        characters: Option<String>,
    },
}

impl RawInputEvent {
    pub fn at(&self) -> DateTime<Utc> {
        match self {
            RawInputEvent::Click { at } | RawInputEvent::Key { at, .. } => *at,
        }
    }
}

/// Every method is called from the dedicated capture thread only. Calls are
/// blocking and must stay bounded (the tick is 2 s).
pub trait ActivityPlatform {
    /// Current TCC state. Cheap; called every tick.
    fn permissions(&self) -> PermissionSnapshot;

    /// Raises the system prompt for `permission` when the OS supports one.
    fn request_permission(&self, permission: ActivityPermission);

    /// The focused window of the frontmost app on the main display.
    fn focused_window(&mut self) -> Option<WindowDescriptor>;

    /// Milliseconds since any system input, without recording its contents.
    /// Platforms without this signal keep the legacy useful-frame rule.
    fn system_idle_ms(&self) -> Option<u64> {
        None
    }

    /// The topmost window of every secondary display (one per display).
    fn secondary_windows(&mut self) -> Vec<WindowDescriptor>;

    /// Text from the accessibility tree of `window`, enabling Chromium/Electron
    /// accessibility when needed. `None` or empty when the app exposes none.
    fn accessibility_text(&mut self, window: &WindowDescriptor) -> Option<String>;

    /// Local OCR of `window`, without persisting images. A timed-out native
    /// operation may retain one in-memory image until cancellation completes.
    fn ocr_text(&mut self, window: &WindowDescriptor) -> Option<String>;

    /// Clipboard text when it changed since the previous call.
    fn clipboard_text_if_changed(&mut self) -> Option<String>;

    /// Starts or stops the input event tap. Without Input Monitoring the tap
    /// cannot start and this is a no-op; frames keep flowing.
    fn set_input_capture(&mut self, enabled: bool);

    /// Input observed since the previous drain.
    fn drain_input_events(&mut self) -> Vec<RawInputEvent>;

    /// Free bytes on the volume holding `path`.
    fn free_disk_bytes(&self, path: &Path) -> Option<u64>;
}
