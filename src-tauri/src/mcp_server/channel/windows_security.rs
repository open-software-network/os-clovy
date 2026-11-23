//! Windows objects are private at creation, then opened without following
//! reparse points and checked through their retained handles. Never repair an
//! existing object's ACL or replace an unreadable/malformed secret.
use std::io::{self, Read, Write};
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path};
use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, LocalFree, HANDLE, HLOCAL};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SE_FILE_OBJECT,
};
use windows::Win32::Security::*;
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Threading::*;

pub(super) fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Clovy MCP requires private local objects owned by the current user",
    )
}
pub(crate) fn win(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    if code & 0xffff0000 == 0x80070000 {
        io::Error::from_raw_os_error((code & 0xffff) as i32)
    } else {
        io::Error::other(error)
    }
}
pub(crate) fn wide(value: &std::ffi::OsStr) -> io::Result<Vec<u16>> {
    let mut value: Vec<_> = value.encode_wide().collect();
    if value.contains(&0) {
        return Err(denied());
    }
    value.push(0);
    Ok(value)
}
pub(crate) struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Local(HLOCAL);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(self.0));
        }
    }
}

// GetTokenInformation returns pointers into this aligned, retained buffer.
fn account(process: HANDLE) -> io::Result<Vec<usize>> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).map_err(win)?;
        let token = Handle(token);
        let mut needed = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut needed);
        if !(size_of::<TOKEN_USER>() as u32..=65536).contains(&needed) {
            return Err(denied());
        }
        let mut bytes = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        let capacity = (bytes.len() * size_of::<usize>()) as u32;
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(bytes.as_mut_ptr().cast()),
            capacity,
            &mut needed,
        )
        .map_err(win)?;
        if needed < size_of::<TOKEN_USER>() as u32 || needed > capacity {
            return Err(denied());
        }
        let sid = (*(bytes.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        let start = bytes.as_ptr() as usize;
        let address = sid.0 as usize;
        // Validate the SID header before asking Win32 to inspect its length.
        if address < start
            || address
                .checked_add(8)
                .map_or(true, |end| end > start + needed as usize)
            || address
                .checked_add(GetLengthSid(sid) as usize)
                .map_or(true, |end| end > start + needed as usize)
            || !IsValidSid(sid).as_bool()
        {
            return Err(denied());
        }
        Ok(bytes)
    }
}
pub(super) fn same_user(pid: u32) -> io::Result<Handle> {
    unsafe {
        let peer = Handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(win)?);
        let own = account(GetCurrentProcess())?;
        let other = account(peer.0)?;
        EqualSid(
            (*(own.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            (*(other.as_ptr().cast::<TOKEN_USER>())).User.Sid,
        )
        .map_err(win)?;
        // Retain the process while qualifying it, rather than trusting a PID
        // that could be reused between checks.
        Ok(peer)
    }
}
fn own_sid() -> io::Result<String> {
    unsafe {
        let account = account(GetCurrentProcess())?;
        let mut string = PWSTR::null();
        ConvertSidToStringSidW(
            (*(account.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            &mut string,
        )
        .map_err(win)?;
        let _string = Local(HLOCAL(string.0.cast()));
        string.to_string().map_err(|_| denied())
    }
}
pub(crate) struct Security {
    _descriptor: Local,
    pub attributes: SECURITY_ATTRIBUTES,
}
impl Security {
    pub fn new() -> io::Result<Self> {
        let sddl = format!("D:P(A;;FA;;;SY)(A;;FA;;;{})", own_sid()?);
        let sddl = wide(std::ffi::OsStr::new(&sddl))?;
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                1,
                &mut descriptor,
                None,
            )
            .map_err(win)?;
            Ok(Self {
                _descriptor: Local(HLOCAL(descriptor.0)),
                attributes: SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor.0,
                    bInheritHandle: false.into(),
                },
            })
        }
    }
}

pub(crate) fn object_info(handle: HANDLE, directory: bool) -> io::Result<()> {
    unsafe {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(handle, &mut info).map_err(win)?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
            || (info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0) != directory
            || (!directory && info.nNumberOfLinks != 1)
        {
            return Err(denied());
        }
        Ok(())
    }
}
pub(crate) fn private_acl(handle: HANDLE) -> io::Result<()> {
    unsafe {
        let mut owner = PSID::default();
        let mut dacl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            Some(&mut descriptor),
        )
        .ok()
        .map_err(win)?;
        let _descriptor = Local(HLOCAL(descriptor.0));
        let own = account(GetCurrentProcess())?;
        let own = (*(own.as_ptr().cast::<TOKEN_USER>())).User.Sid;
        EqualSid(owner, own).map_err(win)?;
        let mut control = 0u16;
        let mut revision = 0;
        GetSecurityDescriptorControl(descriptor, &mut control, &mut revision).map_err(win)?;
        if control & SE_DACL_PROTECTED.0 == 0 || dacl.is_null() {
            return Err(denied());
        }
        let mut info = ACL_SIZE_INFORMATION::default();
        GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
        .map_err(win)?;
        if info.AceCount == 0 || info.AceCount > 2 {
            return Err(denied());
        }
        let mut has_own = false;
        for index in 0..info.AceCount {
            let mut ace = std::ptr::null_mut();
            GetAce(dacl, index, &mut ace).map_err(win)?;
            let header = &*ace.cast::<ACE_HEADER>();
            if header.AceType != 0
                || header.AceFlags != 0
                || (header.AceSize as usize) < size_of::<ACCESS_ALLOWED_ACE>()
            {
                return Err(denied());
            }
            let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
            let sid = PSID((&allowed.SidStart as *const u32).cast_mut().cast());
            if 8 + GetLengthSid(sid) > header.AceSize as u32
                || !IsValidSid(sid).as_bool()
                || allowed.Mask != FILE_ALL_ACCESS.0
            {
                return Err(denied());
            }
            if EqualSid(sid, own).is_ok() {
                has_own = true;
            } else if !IsWellKnownSid(sid, WinLocalSystemSid).as_bool() {
                return Err(denied());
            }
        }
        if !has_own {
            return Err(denied());
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn test_pipe_acl(handle: HANDLE) -> io::Result<()> {
    private_acl(handle)
}

pub(crate) fn open_dir(path: &Path) -> io::Result<Handle> {
    let path = wide(path.as_os_str())?;
    unsafe {
        let handle = Handle(
            CreateFileW(
                PCWSTR(path.as_ptr()),
                READ_CONTROL.0 | FILE_READ_ATTRIBUTES.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                None,
            )
            .map_err(win)?,
        );
        object_info(handle.0, true)?;
        Ok(handle)
    }
}
/// Pin all local ancestors against rename/delete while accessing the secret.
/// Never change their ACLs. UNC paths, relative paths, '..' and reparses fail.
pub(crate) fn ancestors(dir: &Path) -> io::Result<Vec<Handle>> {
    let mut components = dir.components();
    if !matches!(components.next(), Some(Component::Prefix(p)) if matches!(p.kind(), std::path::Prefix::Disk(_) | std::path::Prefix::VerbatimDisk(_)))
        || !matches!(components.next(), Some(Component::RootDir))
    {
        return Err(denied());
    }
    if components.any(|part| !matches!(part, Component::Normal(_))) {
        return Err(denied());
    }
    let mut paths: Vec<_> = dir.ancestors().skip(1).collect();
    paths.reverse();
    paths.into_iter().map(open_dir).collect()
}
pub(crate) fn directory(dir: &Path, create: bool) -> io::Result<Vec<Handle>> {
    let mut held = ancestors(dir)?;
    if create {
        let security = Security::new()?;
        let name = wide(dir.as_os_str())?;
        match unsafe { CreateDirectoryW(PCWSTR(name.as_ptr()), Some(&security.attributes)) } {
            Ok(()) => {}
            Err(error) if error.code().0 as u32 == 0x800700b7 => {}
            Err(error) => return Err(win(error)),
        }
    }
    let directory = open_dir(dir)?;
    private_acl(directory.0)?;
    held.push(directory);
    Ok(held)
}
pub(super) fn secret(dir: &Path, create: bool) -> io::Result<String> {
    let _ancestors = directory(dir, create)?;
    let name = wide(super::super::secret_path(dir).as_os_str())?;
    let security = Security::new()?;
    let mut created = false;
    let open = |disposition, access| unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            access,
            FILE_SHARE_READ,
            Some(&security.attributes),
            disposition,
            FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
        .map_err(win)
    };
    let handle = match open(OPEN_EXISTING, FILE_GENERIC_READ.0 | READ_CONTROL.0) {
        Ok(handle) => handle,
        Err(e) if create && e.kind() == io::ErrorKind::NotFound => {
            created = true;
            open(
                CREATE_NEW,
                FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0 | READ_CONTROL.0,
            )?
        }
        Err(e) => return Err(e),
    };
    // File takes ownership before any fallible validation.
    let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
    object_info(HANDLE(file.as_raw_handle()), false)?;
    private_acl(HANDLE(file.as_raw_handle()))?;
    if !created && file.metadata()?.len() > 128 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the Clovy MCP secret file is malformed",
        ));
    }
    if created {
        use rand::RngCore;
        let mut bytes = [0; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let value: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        file.write_all(value.as_bytes())?;
        file.sync_all()?;
        return Ok(value);
    }
    let mut value = String::new();
    file.take(128).read_to_string(&mut value)?;
    let value = value.trim().to_owned();
    if !super::super::valid_secret(&value) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the Clovy MCP secret file is malformed",
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qualifies_actual_own_process_token_and_fails_closed_for_unknown_peer() {
        let own = unsafe { GetCurrentProcessId() };
        assert!(same_user(own).is_ok());
        assert!(same_user(0).is_err());
    }
    #[test]
    #[ignore = "Requires a Windows host with symlink creation already permitted; never changes host policy"]
    fn refuses_reparse_ancestors_without_changing_target_or_acl() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let alias = root.path().join("alias");
        std::os::windows::fs::symlink_dir(&target, &alias).unwrap();
        assert!(secret(&alias.join("mcp"), true).is_err());
        assert!(!target.join("mcp").exists());
    }
}
