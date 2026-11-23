//! Sticky, owner-only cooperative cancellation for native continuous input.
//! The native engine acknowledges only after releasing held buttons/modifiers.
use std::{
    fs::{self, OpenOptions},
    path::Path,
    time::{Duration, Instant, SystemTime},
};
const GRACE: Duration = Duration::from_millis(120);
pub(crate) fn cooperative_cancel(run_root: &Path) -> bool {
    if !run_root.is_absolute()
        || !fs::symlink_metadata(run_root)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return false;
    }
    let marker = run_root.join("cancel");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&marker)
    {
        Ok(file) => drop(file),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return false,
    }
    let Ok(metadata) = fs::symlink_metadata(&marker) else {
        return false;
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return false;
    }
    let modified = metadata.modified().unwrap_or_else(|_| SystemTime::now());
    let elapsed = SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default();
    let deadline = Instant::now() + GRACE.saturating_sub(elapsed);
    let acknowledged = || {
        fs::symlink_metadata(run_root.join("cancel-ack")).is_ok_and(|metadata| {
            metadata.is_file()
                && !metadata.file_type().is_symlink()
                && metadata.modified().is_ok_and(|ack| ack >= modified)
        })
    };
    loop {
        if acknowledged() {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        std::thread::sleep(remaining.min(Duration::from_millis(2)));
    }
}
/// Native helper effects require the authenticated parent's live namespace.
/// Linux's standalone proof harness is not a Clovy desktop-control surface.
pub(crate) fn current_run_cancelled() -> bool {
    let Some(root) = std::env::var_os("CLOVY_COMPUTER_USE_RUN_DIR").map(std::path::PathBuf::from)
    else {
        return cfg!(any(target_os = "macos", target_os = "windows"));
    };
    if !root.is_absolute()
        || !fs::symlink_metadata(&root)
            .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return true;
    }
    match fs::symlink_metadata(root.join("cancel")) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_is_sticky_and_acknowledgement_is_regular() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("cancel"), b"original").unwrap();
        fs::write(root.path().join("cancel-ack"), b"released").unwrap();
        assert!(cooperative_cancel(root.path()));
        assert_eq!(fs::read(root.path().join("cancel")).unwrap(), b"original");
    }
    #[test]
    fn unacknowledged_release_has_a_bounded_grace_period() {
        let root = tempfile::tempdir().unwrap();
        let start = Instant::now();
        assert!(!cooperative_cancel(root.path()));
        assert!(start.elapsed() < Duration::from_secs(1));
        let repeated = Instant::now();
        assert!(!cooperative_cancel(root.path()));
        assert!(repeated.elapsed() < Duration::from_millis(80));
    }
    #[cfg(unix)]
    #[test]
    fn cancellation_never_overwrites_or_trusts_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        fs::write(outside.path(), b"protected").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("cancel")).unwrap();
        assert!(!cooperative_cancel(root.path()));
        assert_eq!(fs::read(outside.path()).unwrap(), b"protected");
        fs::remove_file(root.path().join("cancel")).unwrap();
        fs::write(root.path().join("cancel"), b"stop").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("cancel-ack")).unwrap();
        assert!(!cooperative_cancel(root.path()));
    }
}
