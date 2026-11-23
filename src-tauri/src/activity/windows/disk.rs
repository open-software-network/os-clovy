//! Filesystem statistics for activity capture storage volumes on Windows.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

/// Returns the free disk bytes available to unprivileged callers on the
/// filesystem volume containing `path`.
pub(crate) fn free_bytes(path: &Path) -> Option<u64> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free_bytes_available: u64 = 0;

    let res = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(wide.as_ptr()),
            Some(&mut free_bytes_available),
            None,
            None,
        )
    };

    if res.is_ok() {
        Some(free_bytes_available)
    } else {
        // Fallback: If path is a file or doesn't exist yet, try its parent directory
        if let Some(parent) = path.parent() {
            let parent_wide: Vec<u16> = parent.as_os_str().encode_wide().chain(Some(0)).collect();
            let res = unsafe {
                GetDiskFreeSpaceExW(
                    PCWSTR(parent_wide.as_ptr()),
                    Some(&mut free_bytes_available),
                    None,
                    None,
                )
            };
            if res.is_ok() {
                return Some(free_bytes_available);
            }
        }
        None
    }
}
