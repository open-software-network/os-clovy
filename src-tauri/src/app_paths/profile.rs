//! Profile location and debug/release guard shared by desktop and relay.
use std::{ffi::OsStr, path::PathBuf};

const USE_PROD_DATA_DIR_ENV: &str = "OS_CLOVY_USE_PROD_DATA_DIR";

/// The installed app's data directory, used by debug helpers to avoid
/// accessing the release profile when they do not have an AppHandle.
pub(crate) fn production_app_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    return dirs::data_dir().map(|root| root.join(crate::app_identity::RELEASE_APP_IDENTIFIER));
    #[cfg(not(windows))]
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join(crate::app_identity::RELEASE_APP_IDENTIFIER)
    })
}

/// Public because migration validation (`db::migrations`) relaxes its
/// newer-schema guard only for dev builds on the isolated dev data dir, and
/// must not relax it when a dev build is pointed at production data.
pub fn use_prod_data_dir() -> bool {
    let value = std::env::var_os(USE_PROD_DATA_DIR_ENV);
    env_value_truthy(value.as_deref())
}

pub(super) fn env_value_truthy(value: Option<&OsStr>) -> bool {
    value.and_then(OsStr::to_str).is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}
