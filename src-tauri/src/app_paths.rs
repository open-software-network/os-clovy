use std::{
    io::{Error, ErrorKind},
    path::{Component, Path, PathBuf},
};
use tauri::{AppHandle, Manager};

#[path = "app_paths/profile.rs"]
mod profile;
#[cfg(test)]
use profile::env_value_truthy;
pub(crate) use profile::production_app_data_dir;
pub use profile::use_prod_data_dir;

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub recordings_dir: PathBuf,
}

impl AppPaths {
    pub fn from_data_dir(data_dir: PathBuf) -> std::io::Result<Self> {
        let recordings_dir = data_dir.join("recordings");
        std::fs::create_dir_all(&recordings_dir)?;
        Ok(Self {
            database_path: data_dir.join("notes.sqlite3"),
            data_dir,
            recordings_dir,
        })
    }

    pub fn recording_session_dir(
        &self,
        note_id: &str,
        session_id: &str,
    ) -> std::io::Result<PathBuf> {
        validate_recording_component("note_id", note_id)?;
        validate_recording_component("session_id", session_id)?;
        Ok(self.recordings_dir.join(note_id).join(session_id))
    }

    pub fn contained_recording_file(&self, path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
        let path = path.as_ref();
        let canonical_path = path.canonicalize()?;
        let canonical_recordings = self.recordings_dir.canonicalize()?;
        if canonical_path.starts_with(&canonical_recordings) {
            Ok(canonical_path)
        } else {
            Err(Error::new(
                ErrorKind::PermissionDenied,
                "recording path is outside the app recordings directory",
            ))
        }
    }

    pub fn remove_recording_file(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let path = path.as_ref();
        match self.contained_recording_file(path) {
            Ok(path) => std::fs::remove_file(path),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub fn app_data_dir(app: &AppHandle) -> Result<PathBuf, tauri::Error> {
    app.path().app_data_dir().map(|data_dir| {
        app_data_dir_for_build(data_dir, cfg!(debug_assertions), use_prod_data_dir())
    })
}

pub fn app_config_dir(app: &AppHandle) -> Result<PathBuf, tauri::Error> {
    app.path().app_config_dir().map(|config_dir| {
        app_config_dir_for_build(config_dir, cfg!(debug_assertions), use_prod_data_dir())
    })
}

/// Public because the MCP relay
/// has no AppHandle and must rebuild the same dev/prod split from scratch.
pub fn app_data_dir_for_build(
    data_dir: PathBuf,
    debug_assertions: bool,
    use_prod: bool,
) -> PathBuf {
    isolated_app_dir_for_build(data_dir, debug_assertions, use_prod)
}

fn app_config_dir_for_build(
    config_dir: PathBuf,
    debug_assertions: bool,
    use_prod: bool,
) -> PathBuf {
    isolated_app_dir_for_build(config_dir, debug_assertions, use_prod)
}

fn isolated_app_dir_for_build(
    directory: PathBuf,
    debug_assertions: bool,
    use_prod: bool,
) -> PathBuf {
    if !debug_assertions || use_prod {
        return directory;
    }

    directory
        .file_name()
        .map(|name| {
            let mut dev_name = name.to_os_string();
            dev_name.push("-dev");
            directory.with_file_name(dev_name)
        })
        .unwrap_or_else(|| directory.join("dev"))
}

fn validate_recording_component(field: &'static str, value: &str) -> std::io::Result<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        || Path::new(value)
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            format!("{field} is not a valid recording path component"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{app_config_dir_for_build, app_data_dir_for_build, env_value_truthy, AppPaths};
    use std::{ffi::OsStr, path::PathBuf};

    #[test]
    fn production_data_override_accepts_only_truthy_values() {
        for value in ["1", "true", "TRUE", " yes ", "on"] {
            assert!(env_value_truthy(Some(OsStr::new(value))), "{value:?}");
        }

        for value in ["", "0", "false", "no", "off", "unexpected"] {
            assert!(!env_value_truthy(Some(OsStr::new(value))), "{value:?}");
        }
        assert!(!env_value_truthy(None));
    }

    #[test]
    fn recording_session_dir_rejects_path_traversal_components() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::from_data_dir(temp.path().join("data")).expect("paths");

        assert!(paths
            .recording_session_dir("../outside", "session")
            .is_err());
        assert!(paths
            .recording_session_dir("/tmp/outside", "session")
            .is_err());
        assert!(paths
            .recording_session_dir("note", "session/child")
            .is_err());
    }

    #[test]
    fn contained_recording_file_rejects_symlink_escape() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = AppPaths::from_data_dir(temp.path().join("data")).expect("paths");
        let outside = temp.path().join("outside.wav");
        std::fs::write(&outside, b"outside").expect("outside");
        let inside_link = paths.recordings_dir.join("linked.wav");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, &inside_link).expect("symlink");
            assert!(paths.contained_recording_file(&inside_link).is_err());
        }
    }

    #[test]
    fn release_builds_use_the_configured_data_dir() {
        let data_dir = PathBuf::from("/tmp/dev.guilhermexp.clovy");

        assert_eq!(
            app_data_dir_for_build(data_dir.clone(), false, false),
            data_dir
        );
    }

    #[test]
    fn debug_builds_use_a_separate_dev_data_dir_by_default() {
        let data_dir = PathBuf::from("/tmp/dev.guilhermexp.clovy");

        assert_eq!(
            app_data_dir_for_build(data_dir, true, false),
            PathBuf::from("/tmp/dev.guilhermexp.clovy-dev")
        );
    }

    #[test]
    fn debug_builds_can_opt_into_the_configured_data_dir() {
        let data_dir = PathBuf::from("/tmp/dev.guilhermexp.clovy");

        assert_eq!(
            app_data_dir_for_build(data_dir.clone(), true, true),
            data_dir
        );
    }

    #[test]
    fn release_builds_use_the_configured_config_dir() {
        let config_dir = PathBuf::from("/tmp/dev.guilhermexp.clovy");

        assert_eq!(
            app_config_dir_for_build(config_dir.clone(), false, false),
            config_dir
        );
    }

    #[test]
    fn debug_builds_use_a_separate_dev_config_dir_by_default() {
        let config_dir = PathBuf::from("/tmp/dev.guilhermexp.clovy");

        assert_eq!(
            app_config_dir_for_build(config_dir, true, false),
            PathBuf::from("/tmp/dev.guilhermexp.clovy-dev")
        );
    }

    #[test]
    fn debug_builds_can_opt_into_the_configured_config_dir() {
        let config_dir = PathBuf::from("/tmp/dev.guilhermexp.clovy");

        assert_eq!(
            app_config_dir_for_build(config_dir.clone(), true, true),
            config_dir
        );
    }
}
