//! Clovy's environment: the local `.env` file and typed lookups.

use std::sync::OnceLock;

static LOCAL_ENV_LOADED: OnceLock<()> = OnceLock::new();

/// Loads the first `.env` found beside the working directory or the crate,
/// once per process. Variables already set win over the file.
pub(crate) fn load_local_env() {
    LOCAL_ENV_LOADED.get_or_init(|| {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(current_dir) = std::env::current_dir() {
            candidates.push(current_dir.join(".env"));
            if let Some(parent) = current_dir.parent() {
                candidates.push(parent.join(".env"));
            }
        }
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        candidates.push(manifest_dir.join(".env"));
        if let Some(parent) = manifest_dir.parent() {
            candidates.push(parent.join(".env"));
        }
        for candidate in candidates {
            if candidate.exists() {
                let _ = dotenvy::from_path(&candidate);
                break;
            }
        }
    });
}

pub(crate) fn trimmed(name: &str) -> String {
    std::env::var(name)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

pub(crate) fn truthy(name: &str) -> bool {
    matches!(
        trimmed(name).to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
