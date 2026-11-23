//! Private app-owned objects. Never repair permissions or follow a symlink.
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

#[cfg(target_os = "macos")]
#[path = "agy_private/darwin_acl.rs"]
mod darwin_acl;

pub(super) fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "The Clovy Antigravity profile or execution directory is not private. Its files were preserved.")
}

#[cfg(test)]
pub(super) fn crash_point(point: &str) {
    if std::env::var("CLOVY_AGY_RF1_CRASH_POINT").as_deref() == Ok(point) {
        std::process::exit(87);
    }
}

pub(super) struct Directory {
    path: PathBuf,
    require_private: bool,
    #[cfg(unix)]
    file: File,
    #[cfg(windows)]
    _held: Vec<File>,
}

fn component(name: &str) -> io::Result<()> {
    if name.is_empty()
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
        || name.contains(['/', '\\', '\0'])
    {
        return Err(denied());
    }
    Ok(())
}

impl Directory {
    /// The app resolved this root before changing any child environment.
    /// Existing ancestors are structural anchors, never chmod targets.
    pub(super) fn root(path: &Path) -> io::Result<Self> {
        if !path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
        {
            return Err(denied());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open("/")?;
            for part in path.components() {
                if let Component::Normal(name) = part {
                    file = unix_open(&file, name, libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
                }
            }
            let metadata = file.metadata()?;
            if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o022 != 0 {
                return Err(denied());
            }
            Ok(Self {
                path: path.to_path_buf(),
                file,
                require_private: false,
            })
        }
        #[cfg(windows)]
        {
            use crate::mcp_server::channel::windows::security;
            let mut held = security::ancestors(path)?;
            held.push(security::open_dir(path)?);
            Ok(Self {
                path: path.to_path_buf(),
                _held: windows_files(held),
                require_private: false,
            })
        }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn identity(&self) -> io::Result<Vec<u64>> {
        #[cfg(unix)]
        {
            identity(&self.file)
        }
        #[cfg(windows)]
        {
            use windows::Win32::Storage::FileSystem::*;
            let mut info = BY_HANDLE_FILE_INFORMATION::default();
            use std::os::windows::io::AsRawHandle;
            let handle = windows::Win32::Foundation::HANDLE(
                self._held.last().ok_or_else(denied)?.as_raw_handle(),
            );
            unsafe { GetFileInformationByHandle(handle, &mut info) }
                .map_err(crate::mcp_server::channel::windows::security::win)?;
            Ok(vec![
                info.dwVolumeSerialNumber.into(),
                info.nFileIndexHigh.into(),
                info.nFileIndexLow.into(),
            ])
        }
    }

    pub(super) fn validate(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let current = Self::root(&self.path)?;
            let (a, b) = (current.file.metadata()?, self.file.metadata()?);
            if (a.dev(), a.ino()) != (b.dev(), b.ino()) {
                return Err(denied());
            }
            if self.require_private {
                private(&current.file, true)?;
            }
        }
        #[cfg(windows)]
        if self.require_private {
            private(self._held.last().ok_or_else(denied)?, true)?;
        }
        // Windows retained handles deny rename/delete on every ancestor.
        Ok(())
    }

    pub(super) fn child(&self, name: &str) -> io::Result<Self> {
        component(name)?;
        self.validate()?;
        let path = self.path.join(name);
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|_| denied())?;
            // The parent is app-owned. Creation stays anchored to its fd;
            // inherited ACLs are checked before any payload, never cleared.
            if unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::AlreadyExists {
                    return Err(error);
                }
            }
            let file = unix_open(
                &self.file,
                std::ffi::OsStr::new(name.to_str().map_err(|_| denied())?),
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?;
            private(&file, true)?;
            self.validate()?;
            Ok(Self {
                path,
                file,
                require_private: true,
            })
        }
        #[cfg(windows)]
        {
            let held = crate::mcp_server::channel::windows::security::directory(&path, true)?;
            Ok(Self {
                path,
                _held: windows_files(held),
                require_private: true,
            })
        }
    }

    /// Validation and cleanup must never create a missing object.
    pub(super) fn existing_child(&self, name: &str) -> io::Result<Self> {
        component(name)?;
        self.validate()?;
        let path = self.path.join(name);
        #[cfg(unix)]
        let child = Self {
            path,
            file: unix_open(
                &self.file,
                std::ffi::OsStr::new(name),
                libc::O_RDONLY | libc::O_DIRECTORY,
                0,
            )?,
            require_private: true,
        };
        #[cfg(windows)]
        let child = Self {
            _held: windows_files(crate::mcp_server::channel::windows::security::directory(
                &path, false,
            )?),
            path,
            require_private: true,
        };
        child.validate()?;
        self.validate()?;
        Ok(child)
    }

    pub(super) fn new_child(&self, name: &str) -> io::Result<Self> {
        component(name)?;
        self.validate()?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|_| denied())?;
            if unsafe { libc::mkdirat(self.file.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(windows)]
        {
            use crate::mcp_server::channel::windows::security;
            let security = security::Security::new()?;
            let path = security::wide(self.path.join(name).as_os_str())?;
            // Unlike reopening an existing child, creation must prove that
            // this invocation created the directory, including on Windows.
            unsafe {
                windows::Win32::Storage::FileSystem::CreateDirectoryW(
                    windows::core::PCWSTR(path.as_ptr()),
                    Some(&security.attributes),
                )
            }
            .map_err(crate::mcp_server::channel::windows::security::win)?;
        }
        self.existing_child(name)
    }

    pub(super) fn names(&self) -> io::Result<Vec<std::ffi::OsString>> {
        self.validate()?;
        // Enumerate names only. Every object used below is admitted separately
        // without following links; enumeration never traverses its entries.
        let names = std::fs::read_dir(&self.path)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        self.validate()?;
        Ok(names)
    }

    /// Remove an identified, empty private directory, never a recursive tree.
    pub(super) fn remove_child(&self, name: &str, expected: &[u64]) -> io::Result<()> {
        let child = self.existing_child(name)?;
        if child.identity()? != expected || !child.names()?.is_empty() {
            return Err(denied());
        }
        drop(child); // Windows directory handles deny deletion while held.
        self.validate()?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|_| denied())?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) }
                != 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(windows)]
        std::fs::remove_dir(self.path.join(name))?;
        self.sync()
    }

    pub(super) fn file(&self, name: &str, create: bool) -> io::Result<File> {
        component(name)?;
        self.validate()?;
        #[cfg(unix)]
        let file = unix_open(
            &self.file,
            std::ffi::OsStr::new(name),
            libc::O_RDWR
                | libc::O_NONBLOCK
                | if create {
                    libc::O_CREAT | libc::O_EXCL
                } else {
                    0
                },
            0o600,
        )?;
        #[cfg(windows)]
        let file = windows_file(&self.path.join(name), create)?;
        private(&file, false)?;
        self.validate()?;
        Ok(file)
    }

    pub(super) fn read(&self, name: &str) -> io::Result<Option<(Vec<u8>, Vec<u64>)>> {
        let file = match self.file(name, false) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let stamp = identity(&file)?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(denied());
        }
        Ok(Some((bytes, stamp)))
    }

    pub(super) fn write_new(&self, name: &str, bytes: &[u8]) -> io::Result<Vec<u64>> {
        let mut file = self.file(name, true)?;
        let stamp = identity(&file)?;
        #[cfg(test)]
        {
            let point = match name {
                "HEAD" => Some("boundary-file-partial"),
                ".clovy-workspace-boundary.json" => Some("boundary-receipt-partial"),
                _ => None,
            };
            if let Some(point) = point {
                if std::env::var("CLOVY_AGY_RF1_CRASH_POINT").as_deref() == Ok(point) {
                    file.write_all(&bytes[..bytes.len() / 2])?;
                    crash_point(point);
                }
            }
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        self.sync()?;
        Ok(stamp)
    }

    /// An unpublished private candidate never occupies the SDK's config name.
    /// Before its receipt is published, a crash can leave only an unreferenced
    /// candidate. A filename alone is not ownership proof for later deletion.
    pub(super) fn stage(&self, role: &str, bytes: &[u8]) -> io::Result<(String, Vec<u64>)> {
        component(role)?;
        let name = format!(".clovy-mcp-{role}-{}", uuid::Uuid::new_v4().simple());
        let mut file = self.file(&name, true)?;
        let stamp = identity(&file)?;
        {
            #[cfg(test)]
            {
                crash_point(&format!("{role}-created"));
                if std::env::var("CLOVY_AGY_RF1_CRASH_POINT").ok().as_deref()
                    == Some(format!("{role}-partial").as_str())
                {
                    file.write_all(&bytes[..bytes.len() / 2])?;
                    crash_point(&format!("{role}-partial"));
                }
            }
            file.write_all(bytes)?;
            file.sync_all()?;
            self.sync()?;
        }
        drop(file);
        #[cfg(test)]
        crash_point(&format!("{role}-ready"));
        Ok((name, stamp))
    }

    pub(super) fn publish_new(&self, name: &str, bytes: &[u8], role: &str) -> io::Result<Vec<u64>> {
        let (candidate, identity) = self.stage(role, bytes)?;
        self.publish(&candidate, name, &identity)?;
        Ok(identity)
    }

    /// Publish a complete unit without replacing an existing name. Files stay
    /// in this held private directory; publication preserves filesystem identity.
    pub(super) fn publish(&self, candidate: &str, name: &str, expected: &[u64]) -> io::Result<()> {
        component(candidate)?;
        component(name)?;
        let file = self.file(candidate, false)?;
        if identity(&file)? != expected {
            return Err(denied());
        }
        self.publish_name(candidate, name)
    }

    pub(super) fn publish_directory(
        &self,
        candidate: &str,
        name: &str,
        expected: &[u64],
    ) -> io::Result<()> {
        let directory = self.existing_child(candidate)?;
        if directory.identity()? != expected {
            return Err(denied());
        }
        // Windows handles deliberately deny directory rename/delete. Close this
        // owned candidate handle before moving; keep the private parent pinned.
        drop(directory);
        self.publish_name(candidate, name)
    }

    fn publish_name(&self, candidate: &str, name: &str) -> io::Result<()> {
        component(candidate)?;
        component(name)?;
        self.validate()?;
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let from = std::ffi::CString::new(candidate).map_err(|_| denied())?;
            let to = std::ffi::CString::new(name).map_err(|_| denied())?;
            let fd = self.file.as_raw_fd();
            #[cfg(target_os = "linux")]
            let result = unsafe {
                libc::renameat2(fd, from.as_ptr(), fd, to.as_ptr(), libc::RENAME_NOREPLACE)
            };
            #[cfg(target_os = "macos")]
            let result = unsafe {
                libc::renameatx_np(fd, from.as_ptr(), fd, to.as_ptr(), libc::RENAME_EXCL)
            };
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Atomic private publication is unavailable on this platform.",
            ));
            #[cfg(any(target_os = "linux", target_os = "macos"))]
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(windows)]
        {
            use crate::mcp_server::channel::windows::security;
            use windows::{
                core::PCWSTR,
                Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH},
            };
            let from = security::wide(self.path.join(candidate).as_os_str())?;
            let to = security::wide(self.path.join(name).as_os_str())?;
            unsafe {
                MoveFileExW(
                    PCWSTR(from.as_ptr()),
                    PCWSTR(to.as_ptr()),
                    MOVEFILE_WRITE_THROUGH,
                )
            }
            .map_err(security::win)?;
        }
        self.sync()
    }

    pub(super) fn remove(&self, name: &str, expected: &[u64]) -> io::Result<()> {
        let file = self.file(name, false)?;
        if identity(&file)? != expected {
            return Err(denied());
        }
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let name = std::ffi::CString::new(name).map_err(|_| denied())?;
            if unsafe { libc::unlinkat(self.file.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        #[cfg(windows)]
        {
            drop(file);
            std::fs::remove_file(self.path.join(name))?;
        }
        self.sync()
    }

    pub(super) fn sync(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.file.sync_all()?;
        }
        Ok(())
    }
}

#[cfg(unix)]
fn unix_open(parent: &File, name: &std::ffi::OsStr, flags: i32, mode: u32) -> io::Result<File> {
    use std::os::unix::{
        ffi::OsStrExt,
        io::{AsRawFd, FromRawFd},
    };
    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| denied())?;
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode,
        )
    };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}

pub(super) fn private(file: &File, directory: bool) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        if m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o777 != if directory { 0o700 } else { 0o600 }
            || m.is_dir() != directory
            || (!directory && (!m.is_file() || m.nlink() != 1))
        {
            return Err(denied());
        }
        #[cfg(target_os = "macos")]
        darwin_acl::require_empty(file)?;
    }
    #[cfg(windows)]
    {
        use crate::mcp_server::channel::windows::security;
        use std::os::windows::io::AsRawHandle;
        let handle = windows::Win32::Foundation::HANDLE(file.as_raw_handle());
        security::object_info(handle, directory)?;
        security::private_acl(handle)?;
    }
    Ok(())
}

pub(super) fn identity(file: &File) -> io::Result<Vec<u64>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        Ok(vec![m.dev(), m.ino()])
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*};
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
            .map_err(crate::mcp_server::channel::windows::security::win)?;
        Ok(vec![
            info.dwVolumeSerialNumber.into(),
            info.nFileIndexHigh.into(),
            info.nFileIndexLow.into(),
        ])
    }
}

#[cfg(unix)]
pub(super) fn private_executable(file: &File) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata()?;
    if m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o777 != 0o700
        || !m.is_file()
        || m.nlink() != 1
    {
        return Err(denied());
    }
    #[cfg(target_os = "macos")]
    darwin_acl::require_empty(file)?;
    Ok(())
}

#[cfg(windows)]
fn windows_files(handles: Vec<crate::mcp_server::channel::windows::security::Handle>) -> Vec<File> {
    use std::os::windows::io::FromRawHandle;
    handles
        .into_iter()
        .map(|handle| {
            let raw = handle.0;
            std::mem::forget(handle);
            // Transfer exclusive CloseHandle ownership to std's Send/Sync wrapper.
            unsafe { File::from_raw_handle(raw.0) }
        })
        .collect()
}

pub(super) fn lease(directory: &Directory) -> io::Result<File> {
    let file = match directory.file(".clovy-turn.lock", true) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            directory.file(".clovy-turn.lock", false)?
        }
        Err(e) => return Err(e),
    };
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Another Clovy turn is using this Antigravity session.",
            ));
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, Storage::FileSystem::*, System::IO::OVERLAPPED};
        let mut overlapped = OVERLAPPED::default();
        unsafe {
            LockFileEx(
                HANDLE(file.as_raw_handle()),
                LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
                None,
                1,
                0,
                &mut overlapped,
            )
        }
        .map_err(crate::mcp_server::channel::windows::security::win)?;
    }
    Ok(file)
}

#[cfg(windows)]
fn windows_file(path: &Path, create: bool) -> io::Result<File> {
    use crate::mcp_server::channel::windows::security;
    use std::os::windows::io::FromRawHandle;
    use windows::{
        core::PCWSTR,
        Win32::{Foundation::GENERIC_READ, Foundation::GENERIC_WRITE, Storage::FileSystem::*},
    };
    let name = security::wide(path.as_os_str())?;
    let security = security::Security::new()?;
    let handle = unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0 | READ_CONTROL.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            Some(&security.attributes),
            if create { CREATE_NEW } else { OPEN_EXISTING },
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(crate::mcp_server::channel::windows::security::win)?;
    Ok(unsafe { File::from_raw_handle(handle.0) })
}
