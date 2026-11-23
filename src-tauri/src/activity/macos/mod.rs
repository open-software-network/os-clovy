//! macOS platform implementation for activity capture.
//!
//! Exposes `MacActivityPlatform`, which implements `ActivityPlatform` using native
//! macOS Accessibility APIs, ScreenCaptureKit, Apple Vision OCR, CoreGraphics
//! event taps, and NSPasteboard change tracking.

pub mod ax;
pub mod clipboard;
pub mod disk;
pub mod input_tap;
pub(crate) mod native;
pub mod ocr;
pub mod permissions;
pub mod screens;

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use objc2::rc::autoreleasepool;

use crate::activity::platform::{
    ActivityPermission, ActivityPlatform, PermissionSnapshot, RawInputEvent, WindowDescriptor,
};
use input_tap::InputTapController;

const K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE: i32 = 1;
const K_CG_ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventSourceSecondsSinceLastEventType(state_id: i32, event_type: u32) -> f64;
}

/// The macOS native activity capture platform adapter.
pub struct MacActivityPlatform {
    chromium_settled_pids: HashSet<i32>,
    last_clipboard_change_count: isize,
    input_buffer: Arc<Mutex<VecDeque<RawInputEvent>>>,
    input_controller: Option<InputTapController>,
    input_capture_enabled: bool,
}

impl MacActivityPlatform {
    /// Constructs a new macOS activity platform instance.
    pub fn new() -> Self {
        Self {
            chromium_settled_pids: HashSet::new(),
            last_clipboard_change_count: -1,
            input_buffer: Arc::new(Mutex::new(VecDeque::new())),
            input_controller: None,
            input_capture_enabled: false,
        }
    }
}

impl Default for MacActivityPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivityPlatform for MacActivityPlatform {
    fn permissions(&self) -> PermissionSnapshot {
        permissions::permissions()
    }

    fn request_permission(&self, permission: ActivityPermission) {
        permissions::request_permission(permission);
    }

    fn focused_window(&mut self) -> Option<WindowDescriptor> {
        autoreleasepool(|_| ax::focused_window(&mut self.chromium_settled_pids))
    }

    fn system_idle_ms(&self) -> Option<u64> {
        // Aggregate duration, not an event tap: no input contents or
        // input-monitoring request are involved.
        let seconds = unsafe {
            CGEventSourceSecondsSinceLastEventType(
                K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE,
                K_CG_ANY_INPUT_EVENT_TYPE,
            )
        };
        let idle = std::time::Duration::try_from_secs_f64(seconds).ok()?;
        u64::try_from(idle.as_millis()).ok()
    }

    fn secondary_windows(&mut self) -> Vec<WindowDescriptor> {
        autoreleasepool(|_| screens::secondary_windows(&mut self.chromium_settled_pids))
    }

    fn accessibility_text(&mut self, window: &WindowDescriptor) -> Option<String> {
        autoreleasepool(|_| ax::accessibility_text(window))
    }

    fn ocr_text(&mut self, window: &WindowDescriptor) -> Option<String> {
        autoreleasepool(|_| ocr::ocr_text(window))
    }

    fn clipboard_text_if_changed(&mut self) -> Option<String> {
        autoreleasepool(|_| {
            clipboard::clipboard_text_if_changed(&mut self.last_clipboard_change_count)
        })
    }

    fn set_input_capture(&mut self, enabled: bool) {
        self.input_capture_enabled = enabled;
        if enabled {
            if self.input_controller.is_none() {
                let perms = self.permissions();
                if perms.input_monitoring.is_granted() {
                    self.input_controller =
                        InputTapController::start(Arc::clone(&self.input_buffer));
                }
            } else if let Some(ctrl) = &self.input_controller {
                ctrl.set_enabled(true);
            }
        } else if let Some(ctrl) = &self.input_controller {
            ctrl.set_enabled(false);
        }
    }

    fn drain_input_events(&mut self) -> Vec<RawInputEvent> {
        if let Some(ctrl) = &self.input_controller {
            ctrl.drain()
        } else {
            let mut q = match self.input_buffer.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            q.drain(..).collect()
        }
    }

    fn free_disk_bytes(&self, path: &Path) -> Option<u64> {
        disk::free_disk_bytes(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mac_activity_platform_instantiation() {
        let mut platform = MacActivityPlatform::new();
        let _ = platform.permissions();
        assert_eq!(platform.drain_input_events().len(), 0);
        assert!(platform.free_disk_bytes(Path::new("/")).is_some());
    }
}
