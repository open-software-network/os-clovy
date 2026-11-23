//! macOS TCC permission probes and request prompts.

use crate::activity::platform::{ActivityPermission, PermissionSnapshot, PermissionState};

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOHIDCheckAccess(requestType: u32) -> u32;
    fn IOHIDRequestAccess(requestType: u32) -> bool;
}

const K_IOHID_REQUEST_TYPE_LISTEN_EVENT: u32 = 1;
const K_IOHID_ACCESS_TYPE_GRANTED: u32 = 0;
const K_IOHID_ACCESS_TYPE_DENIED: u32 = 1;
const K_IOHID_ACCESS_TYPE_UNKNOWN: u32 = 2;

/// Probes live TCC permission states for accessibility, screen recording,
/// and input monitoring.
pub fn permissions() -> PermissionSnapshot {
    let accessibility = if super::native::accessibility_granted() {
        PermissionState::Granted
    } else {
        PermissionState::Denied
    };

    let screen_recording = if super::native::screen_recording_granted() {
        PermissionState::Granted
    } else {
        PermissionState::Denied
    };

    let raw_iohid = unsafe { IOHIDCheckAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT) };
    let input_monitoring = match raw_iohid {
        K_IOHID_ACCESS_TYPE_GRANTED => PermissionState::Granted,
        K_IOHID_ACCESS_TYPE_DENIED => PermissionState::Denied,
        K_IOHID_ACCESS_TYPE_UNKNOWN => PermissionState::NotDetermined,
        _ => PermissionState::NotDetermined,
    };

    PermissionSnapshot {
        accessibility,
        screen_recording,
        input_monitoring,
    }
}

/// Raises the system prompt or navigates to settings for `permission`.
pub fn request_permission(permission: ActivityPermission) {
    match permission {
        ActivityPermission::Accessibility => {
            let _ = super::native::request_accessibility();
        }
        ActivityPermission::ScreenRecording => {
            let _ = super::native::request_screen_recording();
        }
        ActivityPermission::InputMonitoring => unsafe {
            let _ = IOHIDRequestAccess(K_IOHID_REQUEST_TYPE_LISTEN_EVENT);
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permissions_smoke() {
        let snap = permissions();
        // Just verify it returns valid variants and doesn't panic.
        assert!(matches!(
            snap.accessibility,
            PermissionState::Granted | PermissionState::Denied
        ));
        assert!(matches!(
            snap.screen_recording,
            PermissionState::Granted | PermissionState::Denied
        ));
        assert!(matches!(
            snap.input_monitoring,
            PermissionState::Granted | PermissionState::Denied | PermissionState::NotDetermined
        ));
    }
}
