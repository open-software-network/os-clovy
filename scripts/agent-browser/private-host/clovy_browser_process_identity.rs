//! Native process creation identity used by the browser host and supervisor.
use std::io;

/// Birth mismatch (PID reuse), denied/partial observations and a live process
/// are all conservative. An unreaped original zombie has already exited.
pub(crate) fn process_definitely_exited(pid: u32, birth: u64) -> bool {
    if pid == 0 || pid > i32::MAX as u32 || birth == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        if unsafe { libc::kill(pid as i32, 0) } < 0 {
            return io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH);
        }
        if process_birth(pid).ok() != Some(birth) {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                return false;
            };
            stat.rsplit_once(')')
                .and_then(|(_, tail)| tail.split_whitespace().next())
                .is_some_and(|state| matches!(state, "Z" | "X"))
                && process_birth(pid).ok() == Some(birth)
        }
        #[cfg(target_os = "macos")]
        {
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as i32;
            (unsafe {
                libc::proc_pidinfo(
                    pid as i32,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    std::ptr::from_mut(&mut info).cast(),
                    size,
                )
            }) == size
                && info.pbi_status == libc::SZOMB
                && process_birth(pid).ok() == Some(birth)
        }
    }
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn WaitForSingleObject(handle: *mut std::ffi::c_void, timeout: u32) -> u32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        let handle = unsafe { OpenProcess(0x00101000, 0, pid) };
        if handle.is_null() {
            return io::Error::last_os_error().raw_os_error() == Some(87);
        }
        let exited = process_birth(pid).ok() == Some(birth)
            && unsafe { WaitForSingleObject(handle, 0) } == 0;
        unsafe {
            CloseHandle(handle);
        }
        exited
    }
}

#[cfg(unix)]
pub(crate) fn unix_group_exited(group: i32) -> bool {
    if group <= 0 {
        return false;
    }
    // Zombies have already exited and cannot use the profile. Waiting for an
    // unrelated init process to reap them would keep a lease forever on some
    // containers. Live descendants, including a stopped one, keep the lease.
    if unsafe { libc::kill(-group, 0) } < 0
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return false;
        };
        for entry in entries {
            let Ok(entry) = entry else { return false };
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<i32>().ok())
            else {
                continue;
            };
            let stat = match std::fs::read_to_string(entry.path().join("stat")) {
                Ok(stat) => stat,
                Err(_) => {
                    let observed = unsafe { libc::getpgid(pid) };
                    if observed == group
                        || (observed < 0
                            && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
                    {
                        return false;
                    }
                    continue;
                }
            };
            let Some((_, tail)) = stat.rsplit_once(')') else {
                return false;
            };
            let mut fields = tail.split_whitespace();
            let (Some(state), Some(_parent), Some(observed_group)) = (
                fields.next(),
                fields.next(),
                fields.next().and_then(|value| value.parse::<i32>().ok()),
            ) else {
                return false;
            };
            if observed_group == group && !matches!(state, "Z" | "X") {
                return false;
            }
        }
        true
    }
    #[cfg(target_os = "macos")]
    {
        // libproc returns a PID count (not a byte count) for this wrapper.
        let mut pids = vec![0i32; 4096];
        let count = unsafe {
            browser_list_process_group(
                group,
                pids.as_mut_ptr().cast(),
                std::mem::size_of_val(pids.as_slice()) as i32,
            )
        };
        if count <= 0 || count as usize >= pids.len() {
            return false;
        }
        for pid in pids.into_iter().take(count as usize) {
            let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
            let size = std::mem::size_of_val(&info) as i32;
            let read = unsafe {
                browser_process_info(
                    pid,
                    libc::PROC_PIDTBSDINFO,
                    0,
                    std::ptr::from_mut(&mut info).cast(),
                    size,
                )
            };
            if read != size {
                let observed = unsafe { libc::getpgid(pid) };
                if observed == group
                    || (observed < 0
                        && std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH))
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
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        false
    }
}
#[cfg(target_os = "macos")]
#[link(name = "proc")]
extern "C" {
    #[link_name = "proc_listpgrppids"]
    fn browser_list_process_group(
        group: libc::pid_t,
        buffer: *mut libc::c_void,
        size: libc::c_int,
    ) -> libc::c_int;
    #[link_name = "proc_pidinfo"]
    fn browser_process_info(
        pid: libc::c_int,
        flavor: libc::c_int,
        arg: u64,
        buffer: *mut libc::c_void,
        size: libc::c_int,
    ) -> libc::c_int;
}

pub(crate) fn process_birth(pid: u32) -> io::Result<u64> {
    if pid == 0 {
        return Err(io::Error::other("invalid process identity"));
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let tail = stat
            .rsplit_once(')')
            .ok_or_else(|| io::Error::other("invalid process stat"))?
            .1;
        let birth = tail
            .split_whitespace()
            .nth(19)
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value != 0)
            .ok_or_else(|| io::Error::other("invalid process birth"))?;
        Ok(birth)
    }
    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(pid).map_err(|_| io::Error::other("invalid process identity"))?;
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        if unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        } != size
        {
            return Err(io::Error::last_os_error());
        }
        let info = unsafe { info.assume_init() };
        if info.pbi_pid != pid as u32 || info.pbi_start_tvusec >= 1_000_000 {
            return Err(io::Error::other("invalid process birth"));
        }
        info.pbi_start_tvsec
            .checked_mul(1_000_000)
            .and_then(|v| v.checked_add(info.pbi_start_tvusec))
            .filter(|v| *v != 0)
            .ok_or_else(|| io::Error::other("invalid process birth"))
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct FileTime {
            low: u32,
            high: u32,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
            fn GetProcessTimes(
                process: *mut std::ffi::c_void,
                created: *mut FileTime,
                exited: *mut FileTime,
                kernel: *mut FileTime,
                user: *mut FileTime,
            ) -> i32;
            fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        }
        let process = unsafe { OpenProcess(0x1000, 0, pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        let (mut created, mut exited, mut kernel, mut user) = (
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
            FileTime::default(),
        );
        let ok =
            unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user) };
        let error = if ok == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        unsafe {
            CloseHandle(process);
        }
        if let Some(error) = error {
            return Err(error);
        }
        let birth = ((created.high as u64) << 32) | created.low as u64;
        if birth == 0 {
            Err(io::Error::other("invalid process birth"))
        } else {
            Ok(birth)
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn current_and_child_births_are_stable_for_live_processes() {
        let own = process_birth(std::process::id()).unwrap();
        assert_eq!(process_birth(std::process::id()).unwrap(), own);
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let child_birth = process_birth(child.id()).unwrap();
        assert_eq!(process_birth(child.id()).unwrap(), child_birth);
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
