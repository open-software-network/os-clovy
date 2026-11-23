//! Filesystem statistics for activity capture storage volumes.

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// Returns the free disk bytes available to unprivileged callers on the
/// filesystem volume containing `path`.
pub fn free_disk_bytes(path: &Path) -> Option<u64> {
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if res == 0 {
        let bytes = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
        Some(bytes)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_disk_bytes_root() {
        let free = free_disk_bytes(Path::new("/"));
        assert!(free.is_some());
        assert!(free.unwrap() > 0);
    }
}
