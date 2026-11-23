//! What keeps two writers apart: a lock per target shared by every launch,
//! and checks that the old app and stale lock holders are really gone.

use std::io::Write;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::{fail, MigrationError};

/// `<target name><suffix>` beside the target.
pub(super) fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    target.with_file_name(name)
}

/// Held for the whole migration of one target: decision, cleanup of stale
/// staging, credentials and publication. It is an operating-system lock on a
/// file that stays on disk: the system releases it when the holder exits or
/// dies, so there is no owner record to read, trust or take over.
pub(super) struct MigrationLock {
    _file: std::fs::File,
}

impl MigrationLock {
    pub(super) fn acquire(target: &Path, wait: Duration) -> Result<Self, MigrationError> {
        let path = sibling(target, ".identity-migration.lock");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| fail(format!("creating {}", parent.display()), e))?;
        }
        let deadline = Instant::now() + wait;
        loop {
            match try_lock(&path) {
                Ok(Some(mut file)) => {
                    // For a person looking at the file; nothing reads it.
                    let _ = file
                        .set_len(0)
                        .and_then(|()| file.write_all(std::process::id().to_string().as_bytes()));
                    return Ok(Self { _file: file });
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                Ok(None) => {
                    return Err(MigrationError {
                        message: format!(
                            "another Clovy launch is migrating {}; reopen Clovy when it finishes",
                            target.display()
                        ),
                        blocked: true,
                    })
                }
                Err(error) => {
                    return Err(MigrationError {
                        message: format!(
                            "could not take the migration lock {}: {error}",
                            path.display()
                        ),
                        blocked: true,
                    })
                }
            }
        }
    }
}

/// The lock file, exclusively locked; `None` while another process holds it.
fn try_lock(path: &Path) -> std::io::Result<Option<std::fs::File>> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    // SAFETY: a valid descriptor owned by `file`, unlocked when it closes.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(None)
    } else {
        Err(error)
    }
}

/// The pid at the end of a staging name (`<target>.identity-migration-<pid>`,
/// optionally followed by `.scratch`).
pub(super) fn staging_owner(name: &str, prefix: &str) -> Option<u32> {
    let rest = name.strip_prefix(prefix)?;
    rest.split('.').next()?.parse().ok()
}

/// Whether `pid` is a running process.
pub(super) fn is_alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the process exists.
    let result = unsafe { libc::kill(pid, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// The executables of `names` that are running now. A check that cannot run
/// is an error: copying under a live writer is never assumed safe.
pub(super) fn running_executables(names: &[String]) -> Result<Vec<String>, MigrationError> {
    let mut running = Vec::new();
    for name in names {
        let status = crate::hide_console::command("/usr/bin/pgrep")
            .args(["-x", name])
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| blocked_check(name, e))?;
        match status.code() {
            Some(0) => running.push(name.clone()),
            Some(1) => {}
            _ => return Err(blocked_check(name, format!("pgrep exited with {status}"))),
        }
    }
    Ok(running)
}

fn blocked_check(name: &str, error: impl std::fmt::Display) -> MigrationError {
    MigrationError {
        message: format!("could not check whether {name} is running: {error}"),
        blocked: true,
    }
}
