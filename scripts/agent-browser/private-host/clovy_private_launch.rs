//! Chrome inherits the supervisor's process group / Job. Never detach or create a Job here.
#[cfg(unix)]
use std::process::Child;
use std::{
    fs::File,
    path::Path,
    process::{Command, Stdio},
};
pub struct Chrome {
    #[cfg(unix)]
    child: Child,
    #[cfg(windows)]
    child: File,
}
impl Chrome {
    pub fn wait_for_exit(&mut self, timeout: std::time::Duration) -> bool {
        #[cfg(unix)]
        {
            let deadline = std::time::Instant::now() + timeout;
            while std::time::Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                return windows_sys::Win32::System::Threading::WaitForSingleObject(
                    self.child.as_raw_handle() as _,
                    timeout.as_millis() as u32,
                ) == windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            }
        }
        #[cfg(unix)]
        {
            false
        }
    }
}
impl Drop for Chrome {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows_sys::Win32::System::Threading::TerminateProcess(
                    self.child.as_raw_handle() as _,
                    1,
                );
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    self.child.as_raw_handle() as _,
                    2000,
                );
            }
        }
    }
}
pub fn launch(
    chrome: &str,
    profile: &str,
    proxy: u16,
    persistent: bool,
    headed: bool,
) -> Result<(Chrome, File, File), String> {
    if !Path::new(chrome).is_absolute() || !Path::new(profile).is_absolute() || proxy == 0 {
        return Err("Invalid host launch parameters".into());
    }
    let mut cmd = Command::new(chrome);
    cmd.args([
        "--remote-debugging-pipe",
        "--no-first-run",
        "--no-default-browser-check",
        "--no-startup-window",
        "--disable-extensions",
        "--disable-component-update",
        "--disable-background-networking",
        "--disable-quic",
        "--dns-prefetch-disable",
        "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",
        "--disable-sync",
        "--disable-default-apps",
        "--disable-breakpad",
        "--disable-updater-scheduler",
        "--disable-features=WebMCP,MediaRouter,OptimizationHints",
        "--force-webrtc-ip-handling-policy=disable_non_proxied_udp",
        "--proxy-bypass-list=<-loopback>",
        "--window-size=1280,900",
    ])
    .arg(format!("--user-data-dir={profile}"))
    .arg(format!("--proxy-server=http://127.0.0.1:{proxy}"));
    if !persistent {
        // Run-owned anonymous profiles must not request the owner's native key storage.
        cmd.args(["--password-store=basic", "--use-mock-keychain"]);
    }
    if !headed {
        cmd.arg("--headless=new");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        };
        fn pipe() -> Result<(File, File), String> {
            let mut f = [0; 2];
            if unsafe { libc::pipe(f.as_mut_ptr()) } != 0 {
                return Err("Cannot create private CDP pipe".into());
            }
            let files = unsafe { (File::from_raw_fd(f[0]), File::from_raw_fd(f[1])) };
            for fd in [files.0.as_raw_fd(), files.1.as_raw_fd()] {
                if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                    return Err("Cannot protect CDP handles".into());
                }
            }
            Ok(files)
        }
        let (chrome_read, host_write) = pipe()?;
        let (host_read, chrome_write) = pipe()?;
        // Duplicate above descriptors 3/4 before pre_exec, preventing dup2 overlap.
        let r = unsafe { libc::fcntl(chrome_read.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
        let w = unsafe { libc::fcntl(chrome_write.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 10) };
        if r < 0 || w < 0 {
            if r >= 0 {
                unsafe {
                    libc::close(r);
                }
            }
            if w >= 0 {
                unsafe {
                    libc::close(w);
                }
            }
            return Err("Cannot duplicate CDP handles".into());
        }
        let (r, w) = unsafe { (File::from_raw_fd(r), File::from_raw_fd(w)) };
        let (rf, wf) = (r.as_raw_fd(), w.as_raw_fd());
        unsafe {
            cmd.pre_exec(move || {
                if libc::dup2(rf, 3) < 0 || libc::dup2(wf, 4) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|_| "Cannot start private Chrome")?;
        Ok((Chrome { child }, host_read, host_write))
    }
    #[cfg(windows)]
    {
        launch_windows(cmd)
    }
}
#[cfg(windows)]
fn launch_windows(cmd: Command) -> Result<(Chrome, File, File), String> {
    // Rust's std::process whitelist does not inherit arbitrary CDP handles.
    // Use CreateProcessW with inheritable anonymous pipes, retaining the inherited Job.
    use std::{
        mem::{size_of, zeroed},
        os::windows::{
            ffi::OsStrExt,
            io::{AsRawHandle, FromRawHandle},
        },
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, SetHandleInformation, HANDLE_FLAG_INHERIT},
        Security::SECURITY_ATTRIBUTES,
        System::{
            Pipes::CreatePipe,
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
                ResumeThread, UpdateProcThreadAttribute, CREATE_NO_WINDOW, CREATE_SUSPENDED,
                EXTENDED_STARTUPINFO_PRESENT, PROCESS_INFORMATION,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
            },
        },
    };
    unsafe {
        let mut sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: std::ptr::null_mut(),
            bInheritHandle: 1,
        };
        let (mut cr, mut hw, mut hr, mut cw) = (0, 0, 0, 0);
        if CreatePipe(&mut cr, &mut hw, &mut sa, 0) == 0 {
            return Err("Cannot create CDP pipe".into());
        }
        if CreatePipe(&mut hr, &mut cw, &mut sa, 0) == 0 {
            CloseHandle(cr);
            CloseHandle(hw);
            return Err("Cannot create CDP pipe".into());
        }
        let (cr, hw, hr, cw) = (
            File::from_raw_handle(cr as _),
            File::from_raw_handle(hw as _),
            File::from_raw_handle(hr as _),
            File::from_raw_handle(cw as _),
        );
        if SetHandleInformation(hw.as_raw_handle() as _, HANDLE_FLAG_INHERIT, 0) == 0
            || SetHandleInformation(hr.as_raw_handle() as _, HANDLE_FLAG_INHERIT, 0) == 0
        {
            return Err("Cannot protect parent CDP handles".into());
        }
        let quote = |v: &std::ffi::OsStr| -> String {
            let s = v.to_string_lossy();
            let mut out = String::from("\"");
            let mut slashes = 0;
            for c in s.chars() {
                if c == '\\' {
                    slashes += 1;
                    continue;
                }
                if c == '"' {
                    out.push_str(&"\\".repeat(slashes * 2 + 1));
                } else {
                    out.push_str(&"\\".repeat(slashes));
                }
                slashes = 0;
                out.push(c);
            }
            out.push_str(&"\\".repeat(slashes * 2));
            out.push('"');
            out
        };
        let mut line = quote(cmd.get_program());
        for a in cmd.get_args() {
            line.push(' ');
            line.push_str(&quote(a));
        }
        line.push_str(&format!(
            " --remote-debugging-io-pipes={},{}",
            cr.as_raw_handle() as usize,
            cw.as_raw_handle() as usize
        ));
        let mut wide: Vec<u16> = std::ffi::OsStr::new(&line)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let app: Vec<u16> = cmd.get_program().encode_wide().chain(Some(0)).collect();
        let null = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("NUL")
            .map_err(|_| "Cannot open null handle")?;
        if SetHandleInformation(
            null.as_raw_handle() as _,
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        ) == 0
        {
            return Err("Cannot inherit null handle".into());
        }
        let handles = [
            cr.as_raw_handle() as isize,
            cw.as_raw_handle() as isize,
            null.as_raw_handle() as isize,
        ];
        let mut bytes = 0usize;
        InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
        let mut storage = vec![0usize; bytes.div_ceil(size_of::<usize>())];
        let list = storage.as_mut_ptr() as _;
        if InitializeProcThreadAttributeList(list, 1, 0, &mut bytes) == 0 {
            return Err("Cannot initialize handle whitelist".into());
        }
        if UpdateProcThreadAttribute(
            list,
            0,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
            handles.as_ptr() as _,
            std::mem::size_of_val(&handles),
            std::ptr::null_mut(),
            std::ptr::null(),
        ) == 0
        {
            DeleteProcThreadAttributeList(list);
            return Err("Cannot whitelist CDP handles".into());
        }
        let mut si: STARTUPINFOEXW = zeroed();
        si.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        si.lpAttributeList = list;
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = null.as_raw_handle() as _;
        si.StartupInfo.hStdOutput = null.as_raw_handle() as _;
        si.StartupInfo.hStdError = null.as_raw_handle() as _;
        let mut pi: PROCESS_INFORMATION = zeroed();
        let created = CreateProcessW(
            app.as_ptr(),
            wide.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_SUSPENDED | CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
            std::ptr::null(),
            std::ptr::null(),
            &si.StartupInfo,
            &mut pi,
        );
        DeleteProcThreadAttributeList(list);
        if created == 0 {
            return Err("Cannot start private Chrome".into());
        }
        if ResumeThread(pi.hThread) == u32::MAX {
            windows_sys::Win32::System::Threading::TerminateProcess(pi.hProcess, 1);
            CloseHandle(pi.hThread);
            CloseHandle(pi.hProcess);
            return Err("Cannot resume private Chrome".into());
        }
        CloseHandle(pi.hThread);
        // Child is represented separately on Windows because std Child cannot adopt a HANDLE.
        let handle = File::from_raw_handle(pi.hProcess as _);
        Ok((Chrome { child: handle }, hr, hw))
    }
}
