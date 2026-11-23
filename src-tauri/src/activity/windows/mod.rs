//! Windows adapter for the shared text-only activity capture engine.

mod clipboard;
mod disk;
mod input;
mod ocr;
mod uia;
pub(crate) mod window;

use std::path::Path;
use windows::Win32::{
    System::SystemInformation::GetTickCount,
    UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
};

use super::platform::{
    ActivityPermission, ActivityPlatform, PermissionSnapshot, PermissionState, RawInputEvent,
    WindowDescriptor,
};

pub struct WinActivityPlatform {
    ocr: ocr::OcrCapture,
    uia: Option<uia::Uia>,
    clipboard: clipboard::Clipboard,
    input: Option<input::InputCapture>,
}

impl Default for WinActivityPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl WinActivityPlatform {
    pub fn new() -> Self {
        Self {
            ocr: ocr::OcrCapture::new(),
            uia: uia::Uia::new(),
            clipboard: clipboard::Clipboard::new(),
            input: None,
        }
    }

    fn inspect_browser(&mut self, window: &mut WindowDescriptor) {
        if !super::filter::is_browser(&window.app_name, window.bundle_id.as_deref()) {
            return;
        }
        let description = self
            .uia
            .as_mut()
            .map(|uia| uia.describe(window))
            .unwrap_or_default();
        window.private_window = description.private
            || window
                .window_title
                .as_deref()
                .is_some_and(super::filter::has_private_window_marker);
        window.browser_url = if window.private_window {
            None
        } else {
            description.url
        };
    }
}

impl ActivityPlatform for WinActivityPlatform {
    fn permissions(&self) -> PermissionSnapshot {
        PermissionSnapshot {
            accessibility: PermissionState::Granted,
            screen_recording: PermissionState::Granted,
            input_monitoring: PermissionState::Granted,
        }
    }

    fn request_permission(&self, _permission: ActivityPermission) {}

    fn focused_window(&mut self) -> Option<WindowDescriptor> {
        let mut window = window::focused_window()?;
        self.inspect_browser(&mut window);
        Some(window)
    }

    fn secondary_windows(&mut self) -> Vec<WindowDescriptor> {
        let mut windows = window::secondary_windows();
        for window in &mut windows {
            self.inspect_browser(window);
        }
        windows
    }

    fn accessibility_text(&mut self, window: &WindowDescriptor) -> Option<String> {
        self.uia.as_mut()?.text(window)
    }

    fn ocr_text(&mut self, window: &WindowDescriptor) -> Option<String> {
        self.ocr.window_text(window)
    }

    fn system_idle_ms(&self) -> Option<u64> {
        let mut info = LASTINPUTINFO {
            cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
            return None;
        }
        Some(idle_elapsed_ms(unsafe { GetTickCount() }, info.dwTime))
    }

    fn clipboard_text_if_changed(&mut self) -> Option<String> {
        self.clipboard.changed_text()
    }

    fn set_input_capture(&mut self, enabled: bool) {
        if enabled && self.input.is_none() {
            self.input = Some(input::InputCapture::new());
        }
        if let Some(input) = &mut self.input {
            input.set_enabled(enabled);
        }
    }

    fn drain_input_events(&mut self) -> Vec<RawInputEvent> {
        // The shared engine drains on every tick, including while disabled.
        self.ocr.reap_completed();
        self.input
            .as_mut()
            .map(input::InputCapture::drain)
            .unwrap_or_default()
    }

    fn free_disk_bytes(&self, path: &Path) -> Option<u64> {
        disk::free_bytes(path)
    }
}

fn idle_elapsed_ms(now: u32, last_input: u32) -> u64 {
    u64::from(now.wrapping_sub(last_input))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_duration_measures_ticks_without_rollover() {
        assert_eq!(idle_elapsed_ms(1_788, 1_000), 788);
    }

    #[test]
    fn idle_duration_uses_32_bit_wraparound() {
        assert_eq!(idle_elapsed_ms(25, u32::MAX - 74), 100);
        assert_eq!(idle_elapsed_ms(0, u32::MAX), 1);
    }

    #[test]
    fn idle_duration_preserves_zero_and_full_tick_range() {
        assert_eq!(idle_elapsed_ms(0, 0), 0);
        assert_eq!(idle_elapsed_ms(u32::MAX, 0), u64::from(u32::MAX));
    }

    #[test]
    fn system_idle_is_available_without_input_capture() {
        let mut platform = WinActivityPlatform::new();
        platform.set_input_capture(false);

        assert!(
            platform.system_idle_ms().is_some(),
            "Windows must sample system idle time even when Raw Input capture is disabled"
        );
    }
}
