//! Per-install runtime overrides for features that still ship behind public
//! compile-time kill switches. These settings may widen a disabled build flag,
//! but they can never turn off a feature whose public flag is on.

use crate::domain::types::AppError;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Emitter, Manager};

pub const EXPERIMENTAL_FLAGS_CHANGED_EVENT: &str = "experimental-flags-changed";

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExperimentalSettings {
    #[serde(default)]
    pub unlocked: bool,
    #[serde(default)]
    pub browser_use: bool,
}

pub struct ExperimentalSettingsState {
    path: PathBuf,
    settings: Mutex<ExperimentalSettings>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ExperimentalSettingsSnapshot {
    #[serde(flatten)]
    pub settings: ExperimentalSettings,
}

pub fn setup(app: &mut tauri::App) -> Result<(), tauri::Error> {
    let path = experimental_settings_path(app.handle())?;
    let settings = load_settings(&path);
    app.manage(ExperimentalSettingsState {
        path,
        settings: Mutex::new(settings),
    });
    Ok(())
}

pub fn get(state: &ExperimentalSettingsState) -> Result<ExperimentalSettings, AppError> {
    state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|_| {
            AppError::new(
                "experimental_settings_unavailable",
                "Experimental settings lock failed.",
            )
        })
}

pub fn set(
    app: &AppHandle,
    state: &ExperimentalSettingsState,
    settings: ExperimentalSettings,
) -> Result<ExperimentalSettingsSnapshot, AppError> {
    {
        let mut current = state.settings.lock().map_err(|_| {
            AppError::new(
                "experimental_settings_unavailable",
                "Experimental settings lock failed.",
            )
        })?;
        save_settings(&state.path, &settings)?;
        *current = settings.clone();
    }

    if let Some(broker) = app.try_state::<std::sync::Arc<crate::browser_broker::BrowserBroker>>() {
        broker.set_feature_enabled(browser_use_enabled_with(
            crate::feature_flags::BROWSER_USE_ENABLED,
            &settings,
        ));
    }

    let snapshot = snapshot(settings);
    let _ = app.emit(EXPERIMENTAL_FLAGS_CHANGED_EVENT, snapshot.clone());
    Ok(snapshot)
}

pub fn snapshot(settings: ExperimentalSettings) -> ExperimentalSettingsSnapshot {
    ExperimentalSettingsSnapshot { settings }
}

/// Effective Browser use availability for native callers. The stored override
/// is ORed with the public kill switch, so an override can only widen access.
pub fn browser_use_enabled(app: &AppHandle) -> bool {
    let stored = app
        .try_state::<ExperimentalSettingsState>()
        .and_then(|state| get(state.inner()).ok())
        .unwrap_or_default();
    browser_use_enabled_with(crate::feature_flags::BROWSER_USE_ENABLED, &stored)
}

fn browser_use_enabled_with(kill_switch: bool, settings: &ExperimentalSettings) -> bool {
    kill_switch || settings.browser_use
}

fn experimental_settings_path(app: &AppHandle) -> Result<PathBuf, tauri::Error> {
    crate::app_paths::app_data_dir(app)
        .map(|directory| directory.join("experimental-settings.json"))
}

fn load_settings(path: &Path) -> ExperimentalSettings {
    fs::read_to_string(path)
        .ok()
        .and_then(|settings| serde_json::from_str::<ExperimentalSettings>(&settings).ok())
        .unwrap_or_default()
}

fn save_settings(path: &Path, settings: &ExperimentalSettings) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            AppError::new("experimental_settings_save_failed", error.to_string())
        })?;
    }
    let serialized = serde_json::to_string_pretty(settings)
        .map_err(|error| AppError::new("experimental_settings_save_failed", error.to_string()))?;
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, serialized)
        .map_err(|error| AppError::new("experimental_settings_save_failed", error.to_string()))?;
    crate::filesystem::replace_file(&temporary_path, path).map_err(|error| {
        let _ = fs::remove_file(&temporary_path);
        AppError::new("experimental_settings_save_failed", error.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(0);

    fn test_path() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "clovy-experimental-settings-{}-{}",
                std::process::id(),
                NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed)
            ))
            .join("experimental-settings.json")
    }

    #[test]
    fn stored_override_enables_browser_use_when_public_flag_is_false() {
        let settings = ExperimentalSettings {
            browser_use: true,
            ..ExperimentalSettings::default()
        };

        assert!(browser_use_enabled_with(false, &settings));
    }

    #[test]
    fn browser_use_stays_disabled_when_public_flag_and_override_are_false() {
        assert!(!browser_use_enabled_with(
            false,
            &ExperimentalSettings::default()
        ));
    }

    #[test]
    fn settings_load_save_round_trip() {
        let path = test_path();
        let settings = ExperimentalSettings {
            unlocked: true,
            browser_use: true,
        };

        save_settings(&path, &settings).expect("save experimental settings");
        assert_eq!(load_settings(&path), settings);

        let _ = fs::remove_dir_all(path.parent().expect("settings parent"));
    }

    #[test]
    fn malformed_settings_load_defaults() {
        let path = test_path();
        fs::create_dir_all(path.parent().expect("settings parent")).expect("create test dir");
        fs::write(&path, "not json").expect("write malformed settings");

        assert_eq!(load_settings(&path), ExperimentalSettings::default());

        let _ = fs::remove_dir_all(path.parent().expect("settings parent"));
    }
}
