//! Private, host-validated authority for one browser profile.
use super::clovy_browser_process_identity::process_birth;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(crate) fn plain_metadata(path: &Path) -> io::Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::other("link rejected"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("reparse point rejected"));
        }
    }
    Ok(metadata)
}

pub(crate) fn open_existing_plain(path: &Path, write: bool) -> io::Result<File> {
    if !plain_metadata(path)?.is_file() {
        return Err(io::Error::other("regular file required"));
    }
    let mut options = OpenOptions::new();
    options.read(true).write(write);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x0020_0000);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("regular file required"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            return Err(io::Error::other("reparse point rejected"));
        }
    }
    Ok(file)
}

pub(crate) fn open_file_identity(file: &File) -> io::Result<[u64; 2]> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = file.metadata()?;
        Ok([m.dev(), m.ino()])
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        #[derive(Default)]
        struct FileInfo {
            attributes: u32,
            created: [u32; 2],
            accessed: [u32; 2],
            written: [u32; 2],
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetFileInformationByHandle(
                handle: *mut std::ffi::c_void,
                info: *mut FileInfo,
            ) -> i32;
        }
        let mut info = FileInfo::default();
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if info.attributes & 0x400 != 0 {
            return Err(io::Error::other("reparse point rejected"));
        }
        Ok([
            info.volume as u64,
            ((info.index_high as u64) << 32) | info.index_low as u64,
        ])
    }
}

pub(crate) fn profile_object_identity(path: &Path) -> io::Result<[u64; 2]> {
    let metadata = plain_metadata(path)?;
    if !metadata.is_dir() {
        return Err(io::Error::other("directory required"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok([metadata.dev(), metadata.ino()])
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let file = OpenOptions::new()
            .access_mode(0)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(path)?;
        open_file_identity(&file)
    }
}

pub(crate) fn same_open_file(file: &File, path: &Path) -> bool {
    matches!((open_file_identity(file), open_existing_plain(path, false).and_then(|current| open_file_identity(&current))), (Ok(held), Ok(current)) if held == current)
}

pub(crate) fn lock_profile(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct Overlapped {
            internal: usize,
            internal_high: usize,
            offset: u32,
            offset_high: u32,
            event: *mut std::ffi::c_void,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn LockFileEx(
                file: *mut std::ffi::c_void,
                flags: u32,
                reserved: u32,
                low: u32,
                high: u32,
                overlapped: *mut Overlapped,
            ) -> i32;
        }
        let mut overlap: Overlapped = unsafe { std::mem::zeroed() };
        if unsafe { LockFileEx(file.as_raw_handle(), 3, 0, u32::MAX, u32::MAX, &mut overlap) } == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn grant_directory_identity(path: &Path, persistent: bool) -> io::Result<[u64; 2]> {
    if persistent {
        profile_object_identity(&fs::canonicalize(path)?)
    } else {
        profile_object_identity(path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Grant {
    version: u32,
    pub(crate) profile: PathBuf,
    root: PathBuf,
    nonce: String,
    root_identity: [u64; 2],
    profile_identity: [u64; 2],
    parent_pid: u32,
    parent_birth: u64,
    pub(crate) persistent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Owned {
    pub(crate) version: u32,
    pub(crate) nonce: String,
    pub(crate) root_identity: [u64; 2],
    pub(crate) profile_identity: [u64; 2],
    pub(crate) parent_pid: u32,
    pub(crate) parent_birth: u64,
    pub(crate) host_pid: u32,
    pub(crate) host_birth: u64,
    pub(crate) group: Option<u32>,
    pub(crate) job: Option<String>,
    pub(crate) boot: Option<String>,
}

pub(crate) struct OwnedLease {
    pub(crate) identity: Owned,
    #[cfg(windows)]
    job: OwnedJob,
    lease: File,
    profile: super::clovy_browser_recovery::RecoveryProfile,
}

// Keep the lease after closing the direct browser process until its remaining
// group or Job members have exited. Unknown observations are not proof of exit.
impl Drop for OwnedLease {
    fn drop(&mut self) {
        while !self.other_processes_exited() {
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
}

impl OwnedLease {
    #[cfg(target_os = "linux")]
    fn other_processes_exited(&self) -> bool {
        let Ok(entries) = fs::read_dir("/proc") else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else { return false };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|v| v.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == self.identity.host_pid {
                continue;
            }
            let stat = match fs::read_to_string(entry.path().join("stat")) {
                Ok(stat) => stat,
                Err(_) => {
                    let observed = unsafe { libc::getpgid(pid as i32) };
                    if observed == self.identity.group.unwrap_or_default() as i32
                        || (observed < 0
                            && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
                    {
                        return false;
                    }
                    continue;
                }
            };
            let Some((_, fields)) = stat.rsplit_once(')') else {
                return false;
            };
            let mut fields = fields.split_whitespace();
            let (Some(state), Some(_parent), Some(group)) = (
                fields.next(),
                fields.next(),
                fields.next().and_then(|v| v.parse::<u32>().ok()),
            ) else {
                return false;
            };
            if Some(group) == self.identity.group && !matches!(state, "Z" | "X") {
                return false;
            }
        }
        true
    }

    #[cfg(target_os = "macos")]
    fn other_processes_exited(&self) -> bool {
        #[link(name = "proc")]
        extern "C" {
            fn proc_listpgrppids(
                group: libc::pid_t,
                buffer: *mut libc::c_void,
                size: libc::c_int,
            ) -> libc::c_int;
            fn proc_pidinfo(
                pid: libc::c_int,
                flavor: libc::c_int,
                arg: u64,
                buffer: *mut libc::c_void,
                size: libc::c_int,
            ) -> libc::c_int;
        }
        let mut pids = [0i32; 4096];
        let group = self.identity.group.unwrap_or_default() as i32;
        let count = unsafe {
            proc_listpgrppids(
                group,
                pids.as_mut_ptr().cast(),
                std::mem::size_of_val(&pids) as i32,
            )
        };
        if count <= 0 || count as usize >= pids.len() {
            return false;
        }
        for pid in pids.into_iter().take(count as usize) {
            if pid == self.identity.host_pid as i32 {
                continue;
            }
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as i32;
            if unsafe {
                proc_pidinfo(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    std::ptr::from_mut(&mut info).cast(),
                    size,
                )
            } != size
            {
                let observed = unsafe { libc::getpgid(pid) };
                if observed == group
                    || (observed < 0
                        && io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
                {
                    return false;
                }
                continue;
            }
            if info.pbi_pgid == group as u32 && info.pbi_status != libc::SZOMB {
                return false;
            }
        }
        true
    }

    #[cfg(windows)]
    fn other_processes_exited(&self) -> bool {
        #[repr(C)]
        struct Accounting {
            times: [i64; 4],
            faults: u32,
            processes: u32,
            active: u32,
            terminated: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn QueryInformationJobObject(
                job: *mut std::ffi::c_void,
                class: u32,
                data: *mut std::ffi::c_void,
                length: u32,
                returned: *mut u32,
            ) -> i32;
        }
        let mut data: Accounting = unsafe { std::mem::zeroed() };
        unsafe {
            QueryInformationJobObject(
                self.job.0,
                1,
                std::ptr::from_mut(&mut data).cast(),
                std::mem::size_of_val(&data) as u32,
                std::ptr::null_mut(),
            ) != 0
                && data.active == 1
        }
    }
}

#[cfg(windows)]
struct OwnedJob(*mut std::ffi::c_void);
#[cfg(windows)]
impl OwnedJob {
    fn validate(&self) -> io::Result<()> {
        #[link(name = "kernel32")]
        extern "system" {
            fn IsProcessInJob(
                process: *mut std::ffi::c_void,
                job: *mut std::ffi::c_void,
                result: *mut i32,
            ) -> i32;
            fn GetCurrentProcess() -> *mut std::ffi::c_void;
        }
        let mut member = 0;
        if unsafe { IsProcessInJob(GetCurrentProcess(), self.0, &mut member) } == 0 || member == 0 {
            return Err(io::Error::other("host is outside the granted job"));
        }
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for OwnedJob {
    fn drop(&mut self) {
        #[link(name = "kernel32")]
        extern "system" {
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub(crate) fn boot_identity() -> io::Result<Option<String>> {
    #[cfg(target_os = "linux")]
    {
        let value = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
        let value = value.trim();
        let id = uuid::Uuid::parse_str(value).map_err(io::Error::other)?;
        let mut buffer = uuid::Uuid::encode_buffer();
        if &*id.hyphenated().encode_lower(&mut buffer) != value {
            return Err(io::Error::other("invalid boot identity"));
        }
        Ok(Some(value.to_owned()))
    }
    #[cfg(target_os = "macos")]
    {
        let mut bytes = [0u8; 64];
        let mut size = bytes.len();
        if unsafe {
            libc::sysctlbyname(
                c"kern.bootsessionuuid".as_ptr(),
                bytes.as_mut_ptr().cast(),
                &mut size,
                std::ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let value = std::str::from_utf8(
            bytes
                .get(..size)
                .ok_or_else(|| io::Error::other("invalid boot identity"))?,
        )
        .map_err(io::Error::other)?
        .trim_end_matches('\0');
        let id = uuid::Uuid::parse_str(value).map_err(io::Error::other)?;
        if id.is_nil() {
            return Err(io::Error::other("invalid boot identity"));
        }
        Ok(Some(id.to_string()))
    }
    #[cfg(windows)]
    {
        // SystemBootEnvironmentInformation (90), PHNT ntexapi.h. Only the
        // kernel's per-boot GUID is used, never a clock-derived approximation.
        #[repr(C)]
        struct BootEnvironment {
            identifier: [u8; 16],
            firmware: u32,
            flags: u64,
        }
        #[link(name = "ntdll")]
        extern "system" {
            fn NtQuerySystemInformation(
                class: u32,
                data: *mut std::ffi::c_void,
                size: u32,
                returned: *mut u32,
            ) -> i32;
            fn RtlNtStatusToDosError(status: i32) -> u32;
        }
        let mut data: BootEnvironment = unsafe { std::mem::zeroed() };
        let mut returned = 0;
        let size = std::mem::size_of_val(&data) as u32;
        let status = unsafe {
            NtQuerySystemInformation(
                90,
                std::ptr::from_mut(&mut data).cast(),
                size,
                &mut returned,
            )
        };
        if status < 0 {
            return Err(io::Error::from_raw_os_error(
                unsafe { RtlNtStatusToDosError(status) } as i32,
            ));
        }
        let id = uuid::Uuid::from_bytes(data.identifier);
        if returned != size || id.is_nil() {
            return Err(io::Error::other("invalid boot identity"));
        }
        Ok(Some(id.to_string()))
    }
}

#[cfg(windows)]
fn open_own_job(name: &str) -> io::Result<OwnedJob> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenJobObjectW(access: u32, inherit: i32, name: *const u16) -> *mut std::ffi::c_void;
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(Some(0))
        .collect();
    let handle = unsafe { OpenJobObjectW(0x0004, 0, wide.as_ptr()) }; // JOB_OBJECT_QUERY
    if handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    let job = OwnedJob(handle);
    job.validate()?;
    Ok(job)
}

impl Grant {
    fn anonymous_profile(&self) -> io::Result<super::clovy_browser_recovery::RecoveryProfile> {
        let profile = super::clovy_browser_recovery::RecoveryRoot::open(&self.root)?
            .profile(std::ffi::OsStr::new(&self.nonce))?;
        if self.persistent
            || profile.root_identity() != self.root_identity
            || profile.identity() != self.profile_identity
        {
            return Err(io::Error::other("invalid browser profile capability"));
        }
        Ok(profile)
    }
    #[cfg(windows)]
    pub(crate) fn browser_job_name(&self) -> String {
        format!("Local\\ClovyBrowser-{}", self.nonce)
    }
    pub(crate) fn new(profile: &Path, persistent: bool) -> io::Result<Self> {
        let root = profile
            .parent()
            .ok_or_else(|| io::Error::other("missing profile root"))?;
        if profile.to_str().is_none() || root.to_str().is_none() {
            return Err(io::Error::other("profile path is not valid Unicode"));
        }
        let nonce = if persistent {
            uuid::Uuid::new_v4().to_string()
        } else {
            profile
                .file_name()
                .and_then(|v| v.to_str())
                .ok_or_else(|| io::Error::other("invalid nonce"))?
                .to_owned()
        };
        let parent_pid = std::process::id();
        let grant = Self {
            version: 1,
            profile: profile.to_owned(),
            root: root.to_owned(),
            nonce,
            root_identity: grant_directory_identity(root, persistent)?,
            profile_identity: grant_directory_identity(profile, persistent)?,
            parent_pid,
            parent_birth: process_birth(parent_pid)?,
            persistent,
        };
        grant.validate()?;
        Ok(grant)
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        self.validate_phase(None)
    }

    fn validate_phase(&self, expected: Option<&Owned>) -> io::Result<()> {
        let canonical_nonce = uuid::Uuid::parse_str(&self.nonce).is_ok_and(|id| {
            let mut buffer = uuid::Uuid::encode_buffer();
            &*id.hyphenated().encode_lower(&mut buffer) == self.nonce.as_str()
        });
        if self.version != 1
            || !self.profile.is_absolute()
            || self.profile.parent() != Some(self.root.as_path())
            || !canonical_nonce
            || (!self.persistent
                && self
                    .root
                    .ancestors()
                    .any(|p| !plain_metadata(p).is_ok_and(|m| m.is_dir())))
            || grant_directory_identity(&self.root, self.persistent)? != self.root_identity
            || grant_directory_identity(&self.profile, self.persistent)? != self.profile_identity
            || process_birth(self.parent_pid)? != self.parent_birth
        {
            return Err(io::Error::other("invalid browser grant"));
        }
        if !self.persistent {
            if self.profile.file_name().and_then(|v| v.to_str()) != Some(self.nonce.as_str()) {
                return Err(io::Error::other("invalid browser grant"));
            }
            let profile = self.anonymous_profile()?;
            let mut receipt = profile.open_file(".clovy-reserved.json", false)?;
            let mut bytes = Vec::new();
            (&mut receipt).take(4097).read_to_end(&mut bytes)?;
            if bytes.len() > 4096 || !profile.same_file(".clovy-reserved.json", &receipt) {
                return Err(io::Error::other("invalid browser grant"));
            }
            let value: Value = serde_json::from_slice(&bytes)?;
            if value["version"] != 1
                || value["phase"]
                    != if expected.is_some() {
                        "Owned"
                    } else {
                        "Starting"
                    }
                || value["nonce"] != self.nonce
                || value["ownerProcessId"] != self.parent_pid
                || value["ownerBirth"] != self.parent_birth
                || value["rootIdentity"] != json!(self.root_identity)
                || value["profileIdentity"] != json!(self.profile_identity)
                || expected.is_some_and(|owned| value["owned"] != json!(owned))
            {
                return Err(io::Error::other("invalid browser grant"));
            }
            let mut marker = profile.open_file(".clovy-starting", false)?;
            let mut content = Vec::new();
            (&mut marker).take(32).read_to_end(&mut content)?;
            if content != b"Starting\n" || !profile.same_file(".clovy-starting", &marker) {
                return Err(io::Error::other("invalid browser grant"));
            }
        } else if expected.is_some() {
            return Err(io::Error::other("persistent profiles have no Owned grant"));
        }
        if grant_directory_identity(&self.root, self.persistent)? != self.root_identity
            || grant_directory_identity(&self.profile, self.persistent)? != self.profile_identity
            || process_birth(self.parent_pid)? != self.parent_birth
        {
            return Err(io::Error::other("invalid browser grant"));
        }
        Ok(())
    }

    pub(crate) fn acquire_owned(&self, job_name: Option<&str>) -> io::Result<OwnedLease> {
        if self.persistent {
            return Err(io::Error::other("persistent lease stays with root"));
        }
        self.validate()?;
        #[cfg(test)]
        super::clovy_browser_recovery::checkpoint("grant-before-lock");
        let profile = self.anonymous_profile()?;
        let lease = profile.open_file(".clovy-lease", true)?;
        lock_profile(&lease)?;
        #[cfg(test)]
        super::clovy_browser_recovery::checkpoint("grant-after-lock");
        if !profile.same_file(".clovy-lease", &lease) {
            return Err(io::Error::other("lease changed"));
        }
        self.validate()?;
        let host_pid = std::process::id();
        #[cfg(unix)]
        let group = {
            let group = unsafe { libc::getpgrp() };
            if group <= 0 || group as u32 != host_pid || job_name.is_some() {
                return Err(io::Error::other("invalid browser process group"));
            }
            Some(group as u32)
        };
        #[cfg(windows)]
        let group = None;
        #[cfg(windows)]
        let job = {
            let name = job_name.ok_or_else(|| io::Error::other("missing browser job"))?;
            if name != self.browser_job_name() {
                return Err(io::Error::other("invalid browser job"));
            }
            open_own_job(name)?
        };
        let owned = Owned {
            version: 1,
            nonce: self.nonce.clone(),
            root_identity: self.root_identity,
            profile_identity: self.profile_identity,
            parent_pid: self.parent_pid,
            parent_birth: self.parent_birth,
            host_pid,
            host_birth: process_birth(host_pid)?,
            group,
            job: job_name.map(str::to_owned),
            boot: boot_identity()?,
        };
        let mut source = profile.open_file(".clovy-reserved.json", false)?;
        let mut bytes = Vec::new();
        (&mut source).take(4097).read_to_end(&mut bytes)?;
        if bytes.len() > 4096 || !profile.same_file(".clovy-reserved.json", &source) {
            return Err(io::Error::other("receipt changed"));
        }
        let mut receipt: Value = serde_json::from_slice(&bytes)?;
        if receipt["phase"] != "Starting" {
            return Err(io::Error::other("invalid receipt phase"));
        }
        self.validate()?;
        if !profile.same_file(".clovy-reserved.json", &source) {
            return Err(io::Error::other("receipt changed"));
        }
        receipt["phase"] = json!("Owned");
        receipt["owned"] = json!(&owned);
        #[cfg(test)]
        super::clovy_browser_recovery::checkpoint("grant-before-owned");
        drop(source);
        profile.replace_receipt(&serde_json::to_vec(&receipt)?)?;
        #[cfg(test)]
        super::clovy_browser_recovery::checkpoint("grant-owned-published");
        self.validate_phase(Some(&owned))?;
        Ok(OwnedLease {
            identity: owned,
            lease,
            profile,
            #[cfg(windows)]
            job,
        })
    }

    pub(crate) fn verify_owned(
        &self,
        owned: &Owned,
        child_pid: u32,
        child_birth: u64,
        job_name: Option<&str>,
    ) -> io::Result<()> {
        if self.persistent
            || owned.version != 1
            || owned.nonce != self.nonce
            || owned.root_identity != self.root_identity
            || owned.profile_identity != self.profile_identity
            || owned.parent_pid != self.parent_pid
            || owned.parent_birth != self.parent_birth
            || owned.host_pid != child_pid
            || owned.host_birth != child_birth
            || process_birth(child_pid)? != child_birth
            || owned.boot != boot_identity()?
            || owned.job.as_deref() != job_name
        {
            return Err(io::Error::other("invalid Owned acknowledgement"));
        }
        #[cfg(unix)]
        if owned.group != Some(child_pid)
            || unsafe { libc::getpgid(child_pid as i32) } != child_pid as i32
        {
            return Err(io::Error::other("invalid Owned group"));
        }
        #[cfg(windows)]
        if owned.group.is_some() || job_name != Some(self.browser_job_name().as_str()) {
            return Err(io::Error::other("invalid Owned job"));
        }
        self.validate_phase(Some(owned))
    }
}

impl OwnedLease {
    pub(crate) fn validate(&self, grant: &Grant) -> io::Result<()> {
        if self.identity.host_pid != std::process::id()
            || process_birth(self.identity.host_pid)? != self.identity.host_birth
            || !self.profile.same_file(".clovy-lease", &self.lease)
        {
            return Err(io::Error::other("ownership changed"));
        }
        #[cfg(unix)]
        if unsafe { libc::getpgrp() } != self.identity.host_pid as i32 {
            return Err(io::Error::other("ownership group changed"));
        }
        #[cfg(windows)]
        self.job.validate()?;
        grant.validate_phase(Some(&self.identity))
    }
}
