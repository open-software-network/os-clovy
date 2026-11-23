//! Local application preferences, independent of experimental settings.
use crate::domain::types::AppError;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct AppPreferences {
    pub hide_from_dock: bool,
}

pub struct AppPreferencesState {
    path: PathBuf,
    preferences: Mutex<AppPreferences>,
}

pub fn setup(app: &mut tauri::App) -> Result<(), tauri::Error> {
    let path = crate::app_paths::app_config_dir(app.handle())?.join("app-preferences.json");
    let preferences = load(&path);
    app.manage(AppPreferencesState {
        path,
        preferences: Mutex::new(preferences),
    });
    Ok(())
}

pub fn get(state: &AppPreferencesState) -> Result<AppPreferences, AppError> {
    state
        .preferences
        .lock()
        .map(|value| value.clone())
        .map_err(|_| {
            AppError::new(
                "app_preferences_unavailable",
                "App preferences lock failed.",
            )
        })
}

pub fn set(
    state: &AppPreferencesState,
    preferences: AppPreferences,
) -> Result<AppPreferences, AppError> {
    let mut current = state.preferences.lock().map_err(|_| {
        AppError::new(
            "app_preferences_unavailable",
            "App preferences lock failed.",
        )
    })?;
    save(&state.path, &preferences)?;
    *current = preferences.clone();
    Ok(preferences)
}

pub fn hide_from_dock(app: &AppHandle) -> bool {
    app.try_state::<AppPreferencesState>()
        .and_then(|state| get(state.inner()).ok())
        .unwrap_or_default()
        .hide_from_dock
}

fn load(path: &Path) -> AppPreferences {
    fs::read_to_string(path)
        .ok()
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default()
}

fn save(path: &Path, preferences: &AppPreferences) -> Result<(), AppError> {
    let fail =
        |error: std::io::Error| AppError::new("app_preferences_save_failed", error.to_string());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(fail)?;
    }
    let data = serde_json::to_vec_pretty(preferences)
        .map_err(|error| AppError::new("app_preferences_save_failed", error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, data).map_err(fail)?;
    crate::filesystem::replace_file(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        fail(error)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_and_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app-preferences.json");
        assert_eq!(load(&path), AppPreferences::default());
        fs::write(&path, "invalid").unwrap();
        assert_eq!(load(&path), AppPreferences::default());
        let preferences = AppPreferences {
            hide_from_dock: true,
        };
        save(&path, &preferences).unwrap();
        assert_eq!(load(&path), preferences);
    }
}
