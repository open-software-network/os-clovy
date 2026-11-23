//! Native identity for the open database object held by OAuth custody.
use std::{fs::File, io};

#[cfg(unix)]
pub(super) fn partition_file_identity(file: &File) -> io::Result<[u64; 2]> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata()?;
    Ok([metadata.dev(), metadata.ino()])
}
#[cfg(windows)]
pub(super) fn partition_file_identity(file: &File) -> io::Result<[u64; 2]> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(io::Error::other)?;
    Ok([
        info.dwVolumeSerialNumber as u64,
        ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    ])
}
