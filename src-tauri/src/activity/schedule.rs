//! Cadence and capture state. The engine ticks every 2 s; this module decides
//! what a tick does and whether capture is active, paused (and why), or
//! blocked. Pure functions, driven by the engine with real or simulated time.

use std::time::Duration;

use serde::Serialize;

use super::platform::{ActivityPermission, PermissionSnapshot};
use super::settings::ActivitySettings;
use super::store::PauseReason;

pub const TICK: Duration = Duration::from_secs(2);
/// Secondary displays are sampled every 5th tick (~10 s).
pub const SECONDARY_EVERY_TICKS: u64 = 5;
/// Free disk space is checked every 30th tick (~1 min).
pub const DISK_CHECK_EVERY_TICKS: u64 = 30;
/// Retention runs on the first tick and then every 1800th (~1 h).
pub const RETENTION_EVERY_TICKS: u64 = 1_800;
/// Capture pauses below 1 GiB free and resumes above 2 GiB (hysteresis, so a
/// volume hovering at the threshold does not flap).
pub const LOW_DISK_PAUSE_BYTES: u64 = 1 << 30;
pub const LOW_DISK_RESUME_BYTES: u64 = 2 << 30;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TickPlan {
    pub secondary: bool,
    pub disk_check: bool,
    pub retention: bool,
}

pub fn plan_tick(tick: u64, settings: &ActivitySettings) -> TickPlan {
    TickPlan {
        secondary: settings.secondary_monitors && tick % SECONDARY_EVERY_TICKS == 0,
        disk_check: tick % DISK_CHECK_EVERY_TICKS == 0,
        retention: tick % RETENTION_EVERY_TICKS == 0,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CaptureState {
    Off,
    Active,
    Paused { reason: PauseReason },
    NeedsPermissions { missing: Vec<ActivityPermission> },
    KeyMissing,
    Error { message: String },
}

impl CaptureState {
    pub fn pause_reason(&self) -> Option<PauseReason> {
        match self {
            CaptureState::Paused { reason } => Some(*reason),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreStatus {
    Ready,
    KeyMissing,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct GateInputs {
    pub enabled: bool,
    pub store: StoreStatus,
    pub permissions: PermissionSnapshot,
    pub manual_pause: bool,
    pub within_work_hours: bool,
    pub low_disk: bool,
    pub protected_video: bool,
}

/// Accessibility and Screen Recording are required; Input Monitoring only
/// gates click/key events and never blocks frames.
pub fn missing_required(permissions: &PermissionSnapshot) -> Vec<ActivityPermission> {
    let mut missing = Vec::new();
    if !permissions.accessibility.is_granted() {
        missing.push(ActivityPermission::Accessibility);
    }
    if !permissions.screen_recording.is_granted() {
        missing.push(ActivityPermission::ScreenRecording);
    }
    missing
}

/// Precedence: off, storage problems, missing permissions, then pause
/// reasons from the most to the least deliberate.
pub fn evaluate(inputs: &GateInputs) -> CaptureState {
    if !inputs.enabled {
        return CaptureState::Off;
    }
    match &inputs.store {
        StoreStatus::Ready => {}
        StoreStatus::KeyMissing => return CaptureState::KeyMissing,
        StoreStatus::Failed(message) => {
            return CaptureState::Error {
                message: message.clone(),
            }
        }
    }
    let missing = missing_required(&inputs.permissions);
    if !missing.is_empty() {
        return CaptureState::NeedsPermissions { missing };
    }
    let reason = if inputs.manual_pause {
        PauseReason::Manual
    } else if !inputs.within_work_hours {
        PauseReason::WorkHours
    } else if inputs.low_disk {
        PauseReason::LowDisk
    } else if inputs.protected_video {
        PauseReason::ProtectedVideo
    } else {
        return CaptureState::Active;
    };
    CaptureState::Paused { reason }
}

/// Next low-disk flag from the previous one and the measured free space.
/// An unknown measurement keeps the previous state.
pub fn low_disk_next(previous: bool, free_bytes: Option<u64>) -> bool {
    match free_bytes {
        None => previous,
        Some(free) if previous => free < LOW_DISK_RESUME_BYTES,
        Some(free) => free < LOW_DISK_PAUSE_BYTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::platform::PermissionState;

    fn granted() -> PermissionSnapshot {
        PermissionSnapshot {
            accessibility: PermissionState::Granted,
            screen_recording: PermissionState::Granted,
            input_monitoring: PermissionState::Denied,
        }
    }

    fn inputs() -> GateInputs {
        GateInputs {
            enabled: true,
            store: StoreStatus::Ready,
            permissions: granted(),
            manual_pause: false,
            within_work_hours: true,
            low_disk: false,
            protected_video: false,
        }
    }

    #[test]
    fn secondary_monitors_off_never_schedule_secondary_samples() {
        let settings = ActivitySettings::default();
        assert!((0..100).all(|tick| !plan_tick(tick, &settings).secondary));
        let on = ActivitySettings {
            secondary_monitors: true,
            ..ActivitySettings::default()
        };
        let sampled: Vec<u64> = (0..12)
            .filter(|tick| plan_tick(*tick, &on).secondary)
            .collect();
        assert_eq!(sampled, vec![0, 5, 10]);
    }

    #[test]
    fn retention_runs_first_and_then_hourly() {
        let settings = ActivitySettings::default();
        let runs: Vec<u64> = (0..3_700)
            .filter(|tick| plan_tick(*tick, &settings).retention)
            .collect();
        assert_eq!(runs, vec![0, 1_800, 3_600]);
    }

    #[test]
    fn missing_permissions_block_capture_without_input_monitoring_mattering() {
        let mut gate = inputs();
        gate.permissions.accessibility = PermissionState::Denied;
        gate.permissions.screen_recording = PermissionState::NotDetermined;
        assert_eq!(
            evaluate(&gate),
            CaptureState::NeedsPermissions {
                missing: vec![
                    ActivityPermission::Accessibility,
                    ActivityPermission::ScreenRecording
                ]
            }
        );
        // Input Monitoring denied alone does not block.
        assert_eq!(evaluate(&inputs()), CaptureState::Active);
    }

    #[test]
    fn off_and_key_missing_take_precedence() {
        let mut gate = inputs();
        gate.store = StoreStatus::KeyMissing;
        assert_eq!(evaluate(&gate), CaptureState::KeyMissing);
        gate.enabled = false;
        assert_eq!(evaluate(&gate), CaptureState::Off);
    }

    #[test]
    fn pause_reasons_follow_precedence() {
        let mut gate = inputs();
        gate.protected_video = true;
        assert_eq!(
            evaluate(&gate).pause_reason(),
            Some(PauseReason::ProtectedVideo)
        );
        gate.low_disk = true;
        assert_eq!(evaluate(&gate).pause_reason(), Some(PauseReason::LowDisk));
        gate.within_work_hours = false;
        assert_eq!(evaluate(&gate).pause_reason(), Some(PauseReason::WorkHours));
        gate.manual_pause = true;
        assert_eq!(evaluate(&gate).pause_reason(), Some(PauseReason::Manual));
    }

    #[test]
    fn low_disk_uses_hysteresis() {
        let gib = 1_u64 << 30;
        assert!(low_disk_next(false, Some(gib / 2)));
        assert!(!low_disk_next(false, Some(gib + 1)));
        assert!(low_disk_next(true, Some(gib + gib / 2)));
        assert!(!low_disk_next(true, Some(3 * gib)));
        assert!(low_disk_next(true, None));
    }

    #[test]
    fn state_serializes_for_the_frontend_contract() {
        let json = serde_json::to_value(CaptureState::Paused {
            reason: PauseReason::WorkHours,
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "paused", "reason": "workHours"})
        );
        let json = serde_json::to_value(CaptureState::NeedsPermissions {
            missing: vec![ActivityPermission::ScreenRecording],
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "needsPermissions", "missing": ["screenRecording"]})
        );
    }
}
