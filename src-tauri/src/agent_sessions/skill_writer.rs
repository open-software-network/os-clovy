//! Replacement anchored to the managed directory object, not a later pathname.
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
};

pub(super) struct AnchoredSkillDirectory {
    directory: File,
}

#[cfg(test)]
thread_local! {
    static AFTER_BIND: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}
#[cfg(test)]
pub(super) fn after_bind() {
    AFTER_BIND.with(|hook| {
        if let Some(hook) = hook.borrow_mut().take() {
            hook();
        }
    });
}

#[cfg(unix)]
mod native {
    use super::*;
    use std::{
        ffi::{CString, OsStr},
        os::unix::{
            ffi::OsStrExt,
            fs::{MetadataExt, OpenOptionsExt},
            io::{AsRawFd, FromRawFd, RawFd},
        },
    };

    fn component(value: &OsStr) -> io::Result<CString> {
        CString::new(value.as_bytes())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid skill path"))
    }
    fn open_directory(parent: RawFd, name: &CString) -> io::Result<File> {
        let fd = unsafe {
            libc::openat(
                parent,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }
    impl AnchoredSkillDirectory {
        pub(crate) fn open(root: &Path, id: &str) -> io::Result<Self> {
            let expected = std::fs::metadata(root)?;
            let root = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(root)?;
            let actual = root.metadata()?;
            if expected.dev() != actual.dev() || expected.ino() != actual.ino() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Managed root changed",
                ));
            }
            let name = component(OsStr::new(id))?;
            let directory = match open_directory(root.as_raw_fd(), &name) {
                Ok(directory) => directory,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if unsafe { libc::mkdirat(root.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                        let error = io::Error::last_os_error();
                        if error.kind() != io::ErrorKind::AlreadyExists {
                            return Err(error);
                        }
                    }
                    open_directory(root.as_raw_fd(), &name)?
                }
                Err(error) => return Err(error),
            };
            Ok(Self { directory })
        }
        pub(crate) fn replace(&self, content: &str) -> io::Result<()> {
            let parent = self.directory.as_raw_fd();
            for index in 0..128 {
                let name =
                    CString::new(format!(".SKILL.{}.{index}.tmp", std::process::id())).unwrap();
                let fd = unsafe {
                    libc::openat(
                        parent,
                        name.as_ptr(),
                        libc::O_WRONLY
                            | libc::O_CREAT
                            | libc::O_EXCL
                            | libc::O_NOFOLLOW
                            | libc::O_CLOEXEC,
                        0o666,
                    )
                };
                if fd < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(error);
                }
                let mut file = unsafe { File::from_raw_fd(fd) };
                let result = file
                    .write_all(content.as_bytes())
                    .and_then(|()| file.sync_all())
                    .and_then(|()| {
                        if unsafe {
                            libc::renameat(parent, name.as_ptr(), parent, c"SKILL.md".as_ptr())
                        } != 0
                        {
                            Err(io::Error::last_os_error())
                        } else {
                            Ok(())
                        }
                    });
                if result.is_err() {
                    unsafe {
                        libc::unlinkat(parent, name.as_ptr(), 0);
                    }
                }
                return result;
            }
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Skill storage is busy",
            ))
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn replacement_stays_in_open_directory_after_symlink_swap() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let skill = root.path().join("selected");
        std::fs::create_dir(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "before").unwrap();
        std::fs::write(outside.path().join("SKILL.md"), "outside sentinel").unwrap();
        super::super::skill_identity::update_managed_skill(root.path(), "selected", "first")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(skill.join("SKILL.md")).unwrap(),
            "---\nname: selected\n---\n\nfirst"
        );
        let moved = root.path().join("original");
        let moved_for_swap = moved.clone();
        let skill_for_swap = skill.clone();
        let outside_for_swap = outside.path().to_owned();
        AFTER_BIND.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                std::fs::rename(&skill_for_swap, &moved_for_swap).unwrap();
                std::os::unix::fs::symlink(outside_for_swap, &skill_for_swap).unwrap();
            }))
        });
        super::super::skill_identity::update_managed_skill(root.path(), "selected", "after")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(moved.join("SKILL.md")).unwrap(),
            "---\nname: selected\n---\n\nafter"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("SKILL.md")).unwrap(),
            "outside sentinel"
        );
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 1);
        assert!(AnchoredSkillDirectory::open(root.path(), "selected").is_err());
    }

    #[test]
    fn failed_replacement_removes_only_its_own_temporary_file() {
        let root = tempfile::tempdir().unwrap();
        let skill = root.path().join("selected");
        std::fs::create_dir(&skill).unwrap();
        std::fs::create_dir(skill.join("SKILL.md")).unwrap();
        let anchored = AnchoredSkillDirectory::open(root.path(), "selected").unwrap();
        assert!(anchored.replace("cannot replace a directory").is_err());
        assert!(skill.join("SKILL.md").is_dir());
        assert_eq!(std::fs::read_dir(&skill).unwrap().count(), 1);
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::c_void,
        os::windows::{
            ffi::OsStrExt,
            fs::OpenOptionsExt,
            io::{AsRawHandle, FromRawHandle},
        },
        ptr,
    };
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            FileDispositionInfo, FileRenameInfo, GetFileInformationByHandle,
            SetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_DISPOSITION_INFO,
            FILE_RENAME_INFO,
        },
    };
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum_length: u16,
        buffer: *mut u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root: *mut c_void,
        name: *mut UnicodeString,
        attributes: u32,
        security: *mut c_void,
        quality: *mut c_void,
    }
    #[repr(C)]
    struct IoStatus {
        status: usize,
        information: usize,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtCreateFile(
            handle: *mut *mut c_void,
            access: u32,
            attributes: *mut ObjectAttributes,
            status: *mut IoStatus,
            allocation: *const i64,
            file_attributes: u32,
            share: u32,
            disposition: u32,
            options: u32,
            ea: *mut c_void,
            ea_length: u32,
        ) -> i32;
        fn RtlNtStatusToDosError(status: i32) -> u32;
    }
    fn information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        let mut result = BY_HANDLE_FILE_INFORMATION::default();
        unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut result) }
            .map_err(io::Error::other)?;
        if result.dwFileAttributes & 0x400 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Reparse point refused",
            ));
        }
        Ok(result)
    }
    fn discard_temporary(file: &File) {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        let _ = unsafe {
            SetFileInformationByHandle(
                HANDLE(file.as_raw_handle()),
                FileDispositionInfo,
                ptr::from_ref(&disposition).cast(),
                std::mem::size_of_val(&disposition) as u32,
            )
        };
    }
    fn root_handle(path: &Path, create_child: bool) -> io::Result<File> {
        std::fs::OpenOptions::new()
            .access_mode(0x001000a1 | if create_child { 4 } else { 0 })
            .share_mode(7)
            .custom_flags(0x02000000 | 0x00200000)
            .open(path)
    }
    fn open_relative(parent: &File, name: &str, directory: bool, create: bool) -> io::Result<File> {
        let mut wide: Vec<u16> = std::ffi::OsStr::new(name).encode_wide().collect();
        let length = u16::try_from(wide.len() * 2)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid skill name"))?;
        let mut name = UnicodeString {
            length,
            maximum_length: length,
            buffer: wide.as_mut_ptr(),
        };
        let mut attributes = ObjectAttributes {
            length: std::mem::size_of::<ObjectAttributes>() as u32,
            root: parent.as_raw_handle(),
            name: &mut name,
            attributes: 0x40,
            security: ptr::null_mut(),
            quality: ptr::null_mut(),
        };
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        let mut handle = ptr::null_mut();
        let access = if directory { 0x001000a3 } else { 0x00110082 };
        let options = 0x00200000 | 0x20 | if directory { 1 } else { 0x40 };
        let disposition = if directory {
            if create {
                3
            } else {
                1
            }
        } else {
            2
        };
        let code = unsafe {
            NtCreateFile(
                &mut handle,
                access,
                &mut attributes,
                &mut status,
                ptr::null(),
                0,
                7,
                disposition,
                options,
                ptr::null_mut(),
                0,
            )
        };
        if code < 0 {
            return Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(code) } as i32,
            ));
        }
        let file = unsafe { File::from_raw_handle(handle) };
        let info = match information(&file) {
            Ok(info) => info,
            Err(error) => {
                if !directory {
                    discard_temporary(&file);
                }
                return Err(error);
            }
        };
        if (info.dwFileAttributes & 0x10 != 0) != directory {
            if !directory {
                discard_temporary(&file);
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Unexpected skill file type",
            ));
        }
        Ok(file)
    }
    impl AnchoredSkillDirectory {
        pub(crate) fn open(root: &Path, id: &str) -> io::Result<Self> {
            let expected = root_handle(root, false)?;
            let before = information(&expected)?;
            let actual = root_handle(root, false)?;
            let after = information(&actual)?;
            if before.dwVolumeSerialNumber != after.dwVolumeSerialNumber
                || before.nFileIndexHigh != after.nFileIndexHigh
                || before.nFileIndexLow != after.nFileIndexLow
                || after.dwFileAttributes & 0x10 == 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Managed root changed",
                ));
            }
            let directory = match open_relative(&actual, id, true, false) {
                Ok(directory) => directory,
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    let creator = root_handle(root, true)?;
                    let current = information(&creator)?;
                    if current.dwVolumeSerialNumber != before.dwVolumeSerialNumber
                        || current.nFileIndexHigh != before.nFileIndexHigh
                        || current.nFileIndexLow != before.nFileIndexLow
                    {
                        return Err(io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            "Managed root changed",
                        ));
                    }
                    open_relative(&creator, id, true, true)?
                }
                Err(error) => return Err(error),
            };
            Ok(Self { directory })
        }
        pub(crate) fn replace(&self, content: &str) -> io::Result<()> {
            for index in 0..128 {
                let name = format!(".SKILL.{}.{index}.tmp", std::process::id());
                let mut file = match open_relative(&self.directory, &name, false, true) {
                    Ok(file) => file,
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                };
                let result = file
                    .write_all(content.as_bytes())
                    .and_then(|()| file.sync_all())
                    .and_then(|()| {
                        let target: Vec<u16> =
                            std::ffi::OsStr::new("SKILL.md").encode_wide().collect();
                        let offset = std::mem::offset_of!(FILE_RENAME_INFO, FileName);
                        let size = offset + target.len() * 2;
                        let mut storage = vec![0u64; size.div_ceil(std::mem::size_of::<u64>())];
                        let info = storage.as_mut_ptr().cast::<FILE_RENAME_INFO>();
                        unsafe {
                            (*info).Anonymous.ReplaceIfExists = true;
                            (*info).RootDirectory = HANDLE(self.directory.as_raw_handle());
                            (*info).FileNameLength = (target.len() * 2) as u32;
                            ptr::copy_nonoverlapping(
                                target.as_ptr(),
                                ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
                                target.len(),
                            );
                            SetFileInformationByHandle(
                                HANDLE(file.as_raw_handle()),
                                FileRenameInfo,
                                info.cast(),
                                size as u32,
                            )
                            .map_err(io::Error::other)
                        }
                    });
                if result.is_err() {
                    discard_temporary(&file);
                }
                return result;
            }
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Skill storage is busy",
            ))
        }
    }
}
