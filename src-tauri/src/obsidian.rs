use crate::domain::types::AppError;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use tauri::AppHandle;

const OBSIDIAN_CONFIG_FILE: &str = "obsidian.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianStatus {
    pub connected: bool,
    pub available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vault_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObsidianDiscovery {
    pub connected: bool,
    pub available: bool,
    pub vault: Option<ObsidianDiscoveryVault>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ObsidianDiscoveryVault {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObsidianConfigureRequest {
    pub vault_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ObsidianConfig {
    vault_path: String,
}

#[tauri::command]
pub fn obsidian_status(app: AppHandle) -> Result<ObsidianStatus, AppError> {
    status_for_app(&app)
}

#[tauri::command]
pub fn obsidian_configure(
    app: AppHandle,
    request: ObsidianConfigureRequest,
) -> Result<ObsidianStatus, AppError> {
    reject_unsafe_path(request.vault_path.trim())?;
    let vault_path = validate_vault_path(Path::new(request.vault_path.trim()))?;
    ensure_readable(&vault_path)?;
    ensure_writable(&vault_path)?;
    let config = ObsidianConfig {
        vault_path: vault_path.to_string_lossy().into_owned(),
    };
    write_config(&app, &config)?;
    Ok(status_from_path(vault_path))
}

#[tauri::command]
pub fn obsidian_disconnect(app: AppHandle) -> Result<ObsidianStatus, AppError> {
    let path = config_path(&app)?;
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(AppError::new(
                "obsidian_config_unavailable",
                format!("Could not disconnect Obsidian. {error}"),
            ));
        }
    }
    Ok(disconnected_status())
}

/// Resolves the selected vault at the point the MCP request is handled. The
/// MCP adapter deliberately does not interpret `obsidian.json` itself, so all
/// validation and unavailable-vault privacy behavior remain Clovy-owned.
pub(crate) fn discovery_for_app(app: &AppHandle) -> Result<ObsidianDiscovery, AppError> {
    let Some(config) = read_config_optional(app)? else {
        return Ok(ObsidianDiscovery {
            connected: false,
            available: false,
            vault: None,
        });
    };
    Ok(discovery_from_config(&config))
}

fn discovery_from_config(config: &ObsidianConfig) -> ObsidianDiscovery {
    let name = vault_name(&config.vault_path);
    match validate_vault_path(Path::new(&config.vault_path)) {
        Ok(path) => ObsidianDiscovery {
            connected: true,
            available: true,
            vault: Some(ObsidianDiscoveryVault {
                name,
                path: Some(path.to_string_lossy().into_owned()),
            }),
        },
        Err(error) => {
            tracing::warn!(?error, "configured Obsidian vault is unavailable");
            ObsidianDiscovery {
                connected: true,
                available: false,
                // The saved path is sensitive. A temporarily unavailable vault
                // is identifiable by name only; its absolute path is disclosed
                // exclusively when the current validation succeeds.
                vault: Some(ObsidianDiscoveryVault { name, path: None }),
            }
        }
    }
}

fn status_for_app(app: &AppHandle) -> Result<ObsidianStatus, AppError> {
    let Some(config) = read_config_optional(app)? else {
        return Ok(disconnected_status());
    };
    Ok(status_from_config(&config))
}

fn disconnected_status() -> ObsidianStatus {
    ObsidianStatus {
        connected: false,
        available: false,
        vault_path: None,
        vault_name: None,
    }
}

fn status_from_config(config: &ObsidianConfig) -> ObsidianStatus {
    match validate_vault_path(Path::new(&config.vault_path)) {
        Ok(path) => status_from_path(path),
        Err(error) => {
            tracing::warn!(
                ?error,
                "configured Obsidian vault is unavailable in Settings"
            );
            status_from_saved_path(&config.vault_path, false)
        }
    }
}

fn status_from_path(path: PathBuf) -> ObsidianStatus {
    status_from_saved_path(&path.to_string_lossy(), true)
}

fn status_from_saved_path(path: &str, available: bool) -> ObsidianStatus {
    ObsidianStatus {
        connected: true,
        available,
        vault_path: Some(path.to_string()),
        vault_name: Some(vault_name(path)),
    }
}

fn vault_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Vault")
        .to_string()
}

fn config_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    crate::app_paths::app_config_dir(app)
        .map(|dir| dir.join(OBSIDIAN_CONFIG_FILE))
        .map_err(|error| AppError::new("obsidian_config_unavailable", error.to_string()))
}

fn read_config_optional(app: &AppHandle) -> Result<Option<ObsidianConfig>, AppError> {
    let path = config_path(app)?;
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(AppError::new(
                "obsidian_config_unavailable",
                error.to_string(),
            ));
        }
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| AppError::new("obsidian_config_invalid", error.to_string()))
}

fn write_config(app: &AppHandle, config: &ObsidianConfig) -> Result<(), AppError> {
    let path = config_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| AppError::new("obsidian_config_unavailable", error.to_string()))?;
    }
    let text = serde_json::to_string_pretty(config)
        .map_err(|error| AppError::new("obsidian_config_unavailable", error.to_string()))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| AppError::new("obsidian_config_unavailable", error.to_string()))
}

fn reject_unsafe_path(value: &str) -> Result<(), AppError> {
    if value.contains(['\0', '\r', '\n']) {
        return Err(AppError::new(
            "obsidian_vault_invalid",
            "Choose a vault path without line breaks.",
        ));
    }
    Ok(())
}

fn validate_vault_path(path: &Path) -> Result<PathBuf, AppError> {
    if !path.is_absolute() {
        return Err(AppError::new(
            "obsidian_vault_invalid",
            "Choose an absolute Obsidian vault folder.",
        ));
    }
    let canonical = path.canonicalize().map_err(|_| {
        AppError::new(
            "obsidian_vault_invalid",
            "Choose an existing Obsidian vault folder.",
        )
    })?;
    if !canonical.is_dir() {
        return Err(AppError::new(
            "obsidian_vault_invalid",
            "Choose an existing Obsidian vault folder.",
        ));
    }
    let obsidian_dir = canonical.join(".obsidian");
    let obsidian_metadata = fs::symlink_metadata(&obsidian_dir).map_err(|_| {
        AppError::new(
            "obsidian_vault_invalid",
            "Choose a folder that contains an .obsidian directory.",
        )
    })?;
    if obsidian_metadata.file_type().is_symlink() || !obsidian_metadata.is_dir() {
        return Err(AppError::new(
            "obsidian_vault_invalid",
            "Choose a folder that contains an .obsidian directory.",
        ));
    }
    Ok(crate::filesystem::normalize_path_for_external_use(
        &canonical,
    ))
}

fn ensure_readable(path: &Path) -> Result<(), AppError> {
    fs::read_dir(path)
        .map(|_| ())
        .map_err(|_| AppError::new("obsidian_vault_unreadable", "Clovy cannot read this vault."))
}

fn ensure_writable(path: &Path) -> Result<(), AppError> {
    let probe = path.join(format!(
        ".clovy-obsidian-write-probe-{}",
        std::process::id()
    ));
    let result = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
        .and_then(|mut file| file.write_all(b"ok"));
    let _ = fs::remove_file(&probe);
    result.map(|_| ()).map_err(|_| {
        AppError::new(
            "obsidian_vault_unwritable",
            "Clovy cannot write to this vault.",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{discovery_from_config, status_from_config, validate_vault_path, ObsidianConfig};

    #[test]
    fn validates_real_vault_and_returns_external_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let vault = temp.path().join("My Vault");
        std::fs::create_dir_all(vault.join(".obsidian")).expect("vault");
        let validated = validate_vault_path(&vault).expect("valid vault");
        let expected = crate::filesystem::normalize_path_for_external_use(
            &vault.canonicalize().expect("canonical"),
        );
        assert_eq!(validated, expected);
    }

    #[test]
    fn discovery_discloses_only_a_current_available_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let vault = temp.path().join("Work");
        std::fs::create_dir_all(vault.join(".obsidian")).expect("vault");
        let connected = discovery_from_config(&ObsidianConfig {
            vault_path: vault.to_string_lossy().into_owned(),
        });
        assert!(connected.connected);
        assert!(connected.available);
        let expected = crate::filesystem::normalize_path_for_external_use(
            &vault.canonicalize().expect("canonical"),
        )
        .to_string_lossy()
        .into_owned();
        assert_eq!(connected.vault.and_then(|vault| vault.path), Some(expected));

        let unavailable = discovery_from_config(&ObsidianConfig {
            vault_path: "/missing/Work".to_string(),
        });
        assert!(unavailable.connected);
        assert!(!unavailable.available);
        assert_eq!(unavailable.vault.and_then(|vault| vault.path), None);
    }

    #[test]
    fn status_keeps_an_unavailable_configured_vault_disconnectable() {
        let config = ObsidianConfig {
            vault_path: "/missing/Moved Vault".to_string(),
        };
        let status = status_from_config(&config);
        assert!(status.connected);
        assert!(!status.available);
        assert_eq!(status.vault_name.as_deref(), Some("Moved Vault"));
    }

    #[test]
    fn rejects_non_obsidian_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let err = validate_vault_path(temp.path()).expect_err("not a vault");
        assert_eq!(err.code, "obsidian_vault_invalid");
    }
}
