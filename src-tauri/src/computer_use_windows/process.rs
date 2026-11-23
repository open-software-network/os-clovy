//! Raw Windows controller spawn: only three selected handles cross the process
//! boundary; the child belongs to its job from CreateProcess and starts paused.
use super::*;
use std::{
    collections::BTreeMap,
    ffi::{OsStr, OsString},
    mem::size_of_val,
    os::windows::io::{FromRawHandle, OwnedHandle},
    process::ExitStatus,
};
use windows::Win32::{
    Foundation::{
        LocalFree, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HLOCAL, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_USER,
    },
    Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, PIPE_ACCESS_INBOUND,
        PIPE_ACCESS_OUTBOUND,
    },
    System::{
        Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE,
            PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
        Threading::{
            CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
            InitializeProcThreadAttributeList, ResumeThread, UpdateProcThreadAttribute,
            WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
            EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
            PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, STARTF_USESHOWWINDOW, STARTF_USESTDHANDLES,
            STARTUPINFOEXW,
        },
    },
};

struct Local(HLOCAL);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(self.0));
        }
    }
}
struct Attributes {
    storage: Vec<usize>,
    initialized: bool,
}
impl Attributes {
    fn pointer(&self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        LPPROC_THREAD_ATTRIBUTE_LIST(self.storage.as_ptr() as *mut c_void)
    }
}
impl Drop for Attributes {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                DeleteProcThreadAttributeList(self.pointer());
            }
        }
    }
}

pub(crate) struct WindowsChild {
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    process: Handle,
    pid: u32,
    job: OwnedJob,
    run_root: Option<PathBuf>,
}
impl WindowsChild {
    pub fn id(&self) -> Option<u32> {
        Some(self.pid)
    }
    pub async fn wait(&mut self) -> std::io::Result<ExitStatus> {
        use std::os::windows::process::ExitStatusExt;
        loop {
            match unsafe { WaitForSingleObject(self.process.0, 0) } {
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    unsafe { GetExitCodeProcess(self.process.0, &mut code) }?;
                    return Ok(ExitStatus::from_raw(code));
                }
                WAIT_TIMEOUT => tokio::time::sleep(Duration::from_millis(10)).await,
                _ => return Err(std::io::Error::last_os_error()),
            }
        }
    }
}
impl Drop for WindowsChild {
    fn drop(&mut self) {
        if unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_TIMEOUT {
            if let Some(root) = &self.run_root {
                crate::computer_use_cancel::cooperative_cancel(root);
            }
            self.job.terminate();
        }
    }
}

fn security() -> Result<(Local, SECURITY_ATTRIBUTES)> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
        let token = Handle(token);
        let mut needed = 0;
        let _ = GetTokenInformation(token.0, TokenUser, None, 0, &mut needed);
        let mut bytes = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(bytes.as_mut_ptr().cast()),
            (bytes.len() * size_of::<usize>()) as u32,
            &mut needed,
        )?;
        let user = &*(bytes.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text)?;
        let _text = Local(HLOCAL(text.0.cast()));
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;{})", text.to_string()?);
        let encoded: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(encoded.as_ptr()),
            1,
            &mut descriptor,
            None,
        )?;
        let local = Local(HLOCAL(descriptor.0));
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: BOOL(0),
        };
        Ok((local, attributes))
    }
}
fn pipe(parent_writes: bool) -> Result<(OwnedHandle, Handle)> {
    let name: Vec<u16> = format!(r"\\.\pipe\clovy-desktop-{}", uuid::Uuid::new_v4())
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let (_descriptor, attributes) = security()?;
    let access = if parent_writes {
        PIPE_ACCESS_OUTBOUND
    } else {
        PIPE_ACCESS_INBOUND
    };
    let parent = unsafe {
        CreateNamedPipeW(
            PCWSTR(name.as_ptr()),
            access | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            Some(&attributes),
        )
    };
    if parent.is_invalid() {
        return Err(std::io::Error::last_os_error().into());
    }
    let parent = Handle(parent);
    let child_attributes = SECURITY_ATTRIBUTES {
        bInheritHandle: BOOL(1),
        ..attributes
    };
    let child = Handle(unsafe {
        CreateFileW(
            PCWSTR(name.as_ptr()),
            if parent_writes {
                GENERIC_READ.0
            } else {
                GENERIC_WRITE.0
            },
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            Some(&child_attributes),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )?
    });
    let owned = unsafe { OwnedHandle::from_raw_handle(parent.0 .0) };
    std::mem::forget(parent);
    let owned = accept(
        Listener {
            name: String::from_utf16(&name[..name.len() - 1])?,
            server: owned,
        },
        unsafe { GetCurrentProcessId() },
    )?;
    Ok((owned, child))
}
fn quote(argument: &OsStr, out: &mut Vec<u16>) -> Result<()> {
    let bytes: Vec<u16> = argument.encode_wide().collect();
    if bytes.contains(&0) {
        bail!("NUL in controller argument");
    }
    out.push(b'"' as u16);
    let mut slashes = 0;
    for character in bytes {
        if character == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        if character == b'"' as u16 {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2 + 1));
        } else {
            out.extend(std::iter::repeat_n(b'\\' as u16, slashes));
        }
        slashes = 0;
        out.push(character);
    }
    out.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    out.push(b'"' as u16);
    Ok(())
}
fn environment(command: &std::process::Command) -> Result<Vec<u16>> {
    let mut values: BTreeMap<String, (OsString, OsString)> = std::env::vars_os()
        .map(|(name, value)| (name.to_string_lossy().to_uppercase(), (name, value)))
        .collect();
    for (name, value) in command.get_envs() {
        let key = name.to_string_lossy().to_uppercase();
        if let Some(value) = value {
            values.insert(key, (name.to_owned(), value.to_owned()));
        } else {
            values.remove(&key);
        }
    }
    encode_environment(values)
}
fn encode_environment(values: BTreeMap<String, (OsString, OsString)>) -> Result<Vec<u16>> {
    let mut encoded = Vec::new();
    for (_, (name, value)) in values {
        let name: Vec<u16> = name.encode_wide().collect();
        let value: Vec<u16> = value.encode_wide().collect();
        if name.contains(&0) || value.contains(&0) {
            bail!("NUL in controller environment");
        }
        encoded.extend(name);
        encoded.push(b'=' as u16);
        encoded.extend(value);
        encoded.push(0);
    }
    encoded.push(0);
    if encoded.len() == 1 {
        encoded.push(0);
    }
    Ok(encoded)
}

pub(super) struct ApplicationEnvironment {
    encoded: Vec<u16>,
    pub(super) compatibility_layer: Option<String>,
}
pub(super) fn application_environment() -> Result<ApplicationEnvironment> {
    application_environment_from_values(std::env::vars_os())
}
fn application_environment_from_values(
    values: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<ApplicationEnvironment> {
    let mut snapshot = BTreeMap::new();
    for (name, value) in values {
        let key = name
            .to_str()
            .context("A launch environment name is invalid")?;
        if crate::computer_use_policy::private_windows_launch_variable(key) {
            continue;
        }
        if snapshot.insert(key.to_uppercase(), (name, value)).is_some() {
            bail!("The application environment contains ambiguous duplicate names");
        }
    }
    let values = snapshot;
    let compatibility_layer = values
        .get("__COMPAT_LAYER")
        .map(|(_, value)| {
            value
                .to_str()
                .context("The launch compatibility environment is invalid")
                .map(str::to_owned)
        })
        .transpose()?;
    let encoded = encode_environment(values)?;
    if encoded.len() > 1024 * 1024 {
        bail!("The application launch environment exceeds its bound");
    }
    Ok(ApplicationEnvironment {
        encoded,
        compatibility_layer,
    })
}

pub(super) struct ApplicationProcess {
    pub(super) handle: Handle,
    pub(super) pid: u32,
}
/// Only real user applications take this path. The verified Explorer handle
/// supplies the token/job parent; controllers retain their explicit JOB_LIST.
/// CreateProcess itself evaluates effective AppCompat/Fusion/installer policy
/// and refuses elevation instead of entering ShellExecute's AppInfo/UAC path.
pub(super) fn launch_application(
    path: &Path,
    explorer: HANDLE,
    environment: &ApplicationEnvironment,
    before_dispatch: impl FnOnce() -> Result<()>,
) -> Result<ApplicationProcess> {
    let executable = wide(path);
    let mut line = Vec::new();
    quote(path.as_os_str(), &mut line)?;
    line.push(0);
    if !path.is_absolute() || line.len() > 32767 {
        bail!("The application launch path is invalid");
    }
    let directory = wide(
        path.parent()
            .context("The application has no parent directory")?,
    );
    let mut size = 0;
    let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) };
    if size == 0 {
        bail!("The application launch attributes are unavailable");
    }
    let mut attributes = Attributes {
        storage: vec![0usize; size.div_ceil(size_of::<usize>())],
        initialized: false,
    };
    unsafe {
        InitializeProcThreadAttributeList(Some(attributes.pointer()), 1, None, &mut size)?;
    }
    attributes.initialized = true;
    unsafe {
        UpdateProcThreadAttribute(
            attributes.pointer(),
            0,
            PROC_THREAD_ATTRIBUTE_PARENT_PROCESS as usize,
            Some((&explorer as *const HANDLE).cast()),
            size_of::<HANDLE>(),
            None,
            None,
        )?;
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESHOWWINDOW;
    startup.StartupInfo.wShowWindow = SW_SHOWNOACTIVATE.0 as u16;
    startup.lpAttributeList = attributes.pointer();
    let mut info = PROCESS_INFORMATION::default();
    before_dispatch()?;
    unsafe {
        CreateProcessW(
            PCWSTR(executable.as_ptr()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            Some(environment.encoded.as_ptr().cast()),
            PCWSTR(directory.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )?;
    }
    let process = Handle(info.hProcess);
    let _thread = Handle(info.hThread);
    Ok(ApplicationProcess {
        handle: process,
        pid: info.dwProcessId,
    })
}
pub(super) fn spawn(command: Command, job: &OwnedJob) -> Result<WindowsChild> {
    spawn_inner(command, job, true)
}
pub(super) fn spawn_helper(command: Command, job: &OwnedJob) -> Result<WindowsChild> {
    spawn_inner(command, job, false)
}
fn spawn_inner(command: Command, job: &OwnedJob, inherited_stdio: bool) -> Result<WindowsChild> {
    let command = command.as_std();
    let program = Path::new(command.get_program());
    if !program.is_absolute() {
        bail!("Controller executable must be absolute");
    }
    let executable = wide(program);
    let mut line = Vec::new();
    quote(command.get_program(), &mut line)?;
    for argument in command.get_args() {
        line.push(b' ' as u16);
        quote(argument, &mut line)?;
    }
    line.push(0);
    if line.len() > 32767 {
        bail!("Windows controller command exceeds limit");
    }
    let environment = environment(command)?;
    let directory = wide(
        command
            .get_current_dir()
            .or_else(|| program.parent())
            .context("Missing controller directory")?,
    );
    // Main->helper has no inheritable handles at all. Only the isolated
    // helper uses inherited stdio for its serialized engine invocations.
    let stdin = if inherited_stdio {
        Some(pipe(true)?)
    } else {
        None
    };
    let stdout = if inherited_stdio {
        Some(pipe(false)?)
    } else {
        None
    };
    let null = if inherited_stdio {
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            bInheritHandle: BOOL(1),
            ..Default::default()
        };
        Some(Handle(unsafe {
            CreateFileW(
                windows::core::w!("NUL"),
                GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                Some(&attributes),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )?
        }))
    } else {
        None
    };
    let handles = if let (Some(stdin), Some(stdout), Some(null)) = (&stdin, &stdout, &null) {
        vec![stdin.1 .0, stdout.1 .0, null.0]
    } else {
        vec![]
    };
    let jobs = [job.0 .0];
    let attribute_count = if inherited_stdio { 2 } else { 1 };
    let mut size = 0;
    let _ = unsafe { InitializeProcThreadAttributeList(None, attribute_count, None, &mut size) };
    if size == 0 {
        bail!("Windows controller attributes unavailable");
    }
    let mut attributes = Attributes {
        storage: vec![0usize; size.div_ceil(size_of::<usize>())],
        initialized: false,
    };
    unsafe {
        InitializeProcThreadAttributeList(
            Some(attributes.pointer()),
            attribute_count,
            None,
            &mut size,
        )?;
    }
    attributes.initialized = true;
    unsafe {
        if inherited_stdio {
            UpdateProcThreadAttribute(
                attributes.pointer(),
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                size_of_val(handles.as_slice()),
                None,
                None,
            )?;
        }
        UpdateProcThreadAttribute(
            attributes.pointer(),
            0,
            PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
            Some(jobs.as_ptr().cast()),
            size_of_val(&jobs),
            None,
            None,
        )?;
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    if inherited_stdio {
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = handles[0];
        startup.StartupInfo.hStdOutput = handles[1];
        startup.StartupInfo.hStdError = handles[2];
    }
    startup.lpAttributeList = attributes.pointer();
    let mut info = PROCESS_INFORMATION::default();
    unsafe {
        CreateProcessW(
            PCWSTR(executable.as_ptr()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            inherited_stdio,
            CREATE_SUSPENDED
                | CREATE_NO_WINDOW
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT,
            Some(environment.as_ptr().cast()),
            PCWSTR(directory.as_ptr()),
            &startup.StartupInfo,
            &mut info,
        )?;
    }
    let process = Handle(info.hProcess);
    let thread = Handle(info.hThread);
    let mut child = WindowsChild {
        stdin: None,
        stdout: None,
        process,
        pid: info.dwProcessId,
        job: job.clone(),
        run_root: command
            .get_envs()
            .find(|(name, _)| {
                *name == "CLOVY_AGENT_DESKTOP_CANCEL_FILE" || *name == "CLOVY_COMPUTER_USE_RUN_DIR"
            })
            .and_then(|(name, value)| {
                value.map(|value| {
                    if name == "CLOVY_COMPUTER_USE_RUN_DIR" {
                        PathBuf::from(value)
                    } else {
                        Path::new(value)
                            .parent()
                            .unwrap_or(Path::new(value))
                            .to_path_buf()
                    }
                })
            }),
    };
    if !same_path(&process_identity(child.pid)?.path, program) {
        job.terminate();
        bail!("Controller image mismatch");
    }
    // JOB_LIST made job ownership atomic with CreateProcess; there is no
    // orphan interval if the owner dies before this suspended-thread resume.
    if unsafe { ResumeThread(thread.0) } == u32::MAX {
        job.terminate();
        bail!("Cannot resume owned controller");
    }
    if let Some((parent_stdin, _)) = stdin {
        child.stdin = Some(ChildStdin::from_std(std::process::ChildStdin::from(
            parent_stdin,
        ))?);
    }
    if let Some((parent_stdout, _)) = stdout {
        child.stdout = Some(ChildStdout::from_std(std::process::ChildStdout::from(
            parent_stdout,
        ))?);
    }
    Ok(child)
}

pub(super) struct Listener {
    pub name: String,
    server: OwnedHandle,
}
pub(super) fn listener(parent_writes: bool) -> Result<Listener> {
    let name = format!(r"\\.\pipe\clovy-desktop-{}", uuid::Uuid::new_v4());
    let encoded: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let (_descriptor, attributes) = security()?;
    let access = if parent_writes {
        PIPE_ACCESS_OUTBOUND
    } else {
        PIPE_ACCESS_INBOUND
    };
    let server = unsafe {
        CreateNamedPipeW(
            PCWSTR(encoded.as_ptr()),
            access | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            65536,
            65536,
            0,
            Some(&attributes),
        )
    };
    if server.is_invalid() {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(Listener {
        name,
        server: unsafe { OwnedHandle::from_raw_handle(server.0) },
    })
}
pub(super) fn accept(listener: Listener, pid: u32) -> Result<OwnedHandle> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::ERROR_IO_PENDING,
        System::{
            Threading::CreateEventW,
            IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED},
        },
    };
    let pipe = HANDLE(listener.server.as_raw_handle());
    let event = Handle(unsafe { CreateEventW(None, true, false, PCWSTR::null())? });
    let mut overlapped = OVERLAPPED {
        hEvent: event.0,
        ..Default::default()
    };
    if let Err(error) = unsafe { ConnectNamedPipe(pipe, Some(&mut overlapped)) } {
        if error.code() == windows::core::HRESULT::from_win32(ERROR_IO_PENDING.0) {
            // This runs in a blocking worker that owns the OVERLAPPED storage
            // until cancellation has completed. Dropping the async startup
            // owner terminates the Job immediately, independently of this wait.
            if unsafe { WaitForSingleObject(event.0, 5000) } != WAIT_OBJECT_0 {
                unsafe {
                    let _ = CancelIoEx(pipe, Some(&overlapped));
                }
                let mut transferred = 0;
                let _ = unsafe { GetOverlappedResult(pipe, &overlapped, &mut transferred, true) };
                bail!("The authenticated helper did not connect to its private pipe");
            }
            let mut transferred = 0;
            unsafe {
                GetOverlappedResult(pipe, &overlapped, &mut transferred, false)?;
            }
        } else if error.code() != windows::core::HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) {
            return Err(error.into());
        }
    }
    let mut peer = 0;
    unsafe {
        GetNamedPipeClientProcessId(pipe, &mut peer)?;
    }
    if peer != pid {
        bail!("Private helper pipe client identity mismatch");
    }
    Ok(listener.server)
}
pub(super) fn connect_private_transport() -> Result<(ChildStdout, ChildStdin)> {
    use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
    let parent = parent_pid(unsafe { GetCurrentProcessId() })?;
    let open = |key: &str, access: u32| -> Result<OwnedHandle> {
        let name = std::env::var(key)?;
        if !name.starts_with(r"\\.\pipe\clovy-desktop-") || name.len() > 128 {
            bail!("Invalid private helper pipe identity");
        }
        let encoded: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = Handle(unsafe {
            CreateFileW(
                PCWSTR(encoded.as_ptr()),
                access,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )?
        });
        let mut peer = 0;
        unsafe {
            GetNamedPipeServerProcessId(handle.0, &mut peer)?;
        }
        if peer != parent {
            bail!("Private helper pipe server identity mismatch");
        }
        let owned = unsafe { OwnedHandle::from_raw_handle(handle.0 .0) };
        std::mem::forget(handle);
        Ok(owned)
    };
    let input = open("CLOVY_COMPUTER_USE_INPUT_PIPE", GENERIC_READ.0)?;
    let output = open("CLOVY_COMPUTER_USE_OUTPUT_PIPE", GENERIC_WRITE.0)?;
    verify_parent()?; // The still-live signed parent must survive both opens.
    Ok((
        ChildStdout::from_std(std::process::ChildStdout::from(input))?,
        ChildStdin::from_std(std::process::ChildStdin::from(output))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn application_environment_keeps_effective_layers_and_drops_private_authority() {
        let environment = application_environment_from_values([
            (
                OsString::from("__COMPAT_LAYER"),
                OsString::from("~ RUNASADMIN"),
            ),
            (
                OsString::from("PATH"),
                OsString::from("C:\\synthetic path\\😀"),
            ),
            (
                OsString::from("CLOVY_COMPUTER_USE_CAPABILITY"),
                OsString::from("synthetic"),
            ),
            (
                OsString::from("CLOVY_COMPUTER_USE_RUN_DIR"),
                OsString::from("C:\\synthetic-run"),
            ),
            (
                OsString::from("AGENT_DESKTOP_HOME"),
                OsString::from("C:\\engine-run"),
            ),
        ])
        .unwrap();
        assert_eq!(
            environment.compatibility_layer.as_deref(),
            Some("~ RUNASADMIN")
        );
        assert!(
            !crate::computer_use_policy::windows_compatibility_layers_allowed(
                environment.compatibility_layer.as_deref(),
            )
        );
        assert!(environment.encoded.ends_with(&[0, 0]));
        let entries: std::collections::BTreeSet<String> = environment
            .encoded
            .split(|unit| *unit == 0)
            .filter(|entry| !entry.is_empty())
            .map(|entry| String::from_utf16(entry).unwrap())
            .collect();
        assert_eq!(
            entries,
            ["__COMPAT_LAYER=~ RUNASADMIN", "PATH=C:\\synthetic path\\😀"]
                .into_iter()
                .map(str::to_owned)
                .collect()
        );
        assert!(application_environment_from_values([
            (
                OsString::from("__COMPAT_LAYER"),
                OsString::from("RUNASADMIN")
            ),
            (
                OsString::from("__compat_layer"),
                OsString::from("HIGHDPIAWARE")
            ),
        ])
        .is_err());
    }
    #[test]
    fn typed_arguments_roundtrip_the_native_windows_parser() {
        let arguments = [
            "agent-desktop.exe",
            "",
            "--session",
            "literal \"quoted\" \\path\\",
            "emoji 😀 é\nline",
            "{\"x\":\"\\\\\"}",
        ];
        let mut command = Vec::new();
        for argument in arguments {
            if !command.is_empty() {
                command.push(b' ' as u16);
            }
            quote(OsStr::new(argument), &mut command).unwrap();
        }
        command.push(0);
        let mut count = 0;
        let parsed = unsafe {
            windows::Win32::UI::Shell::CommandLineToArgvW(PCWSTR(command.as_ptr()), &mut count)
        };
        assert!(!parsed.is_null());
        let _owner = Local(HLOCAL(parsed.cast()));
        let result: Vec<String> = (0..count)
            .map(|index| unsafe { (*parsed.add(index as usize)).to_string().unwrap() })
            .collect();
        assert_eq!(result, arguments);
    }
    #[test]
    #[ignore = "private subprocess fixture; invoked by the parent handle-isolation test"]
    fn child_probes_inherited_event() {
        let handle = std::env::var("CLOVY_TEST_PRIVATE_HANDLE")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        // The parent checks its exact event object, so a recycled numeric slot
        // in the child cannot make this test accidentally pass or fail.
        let _ =
            unsafe { windows::Win32::System::Threading::SetEvent(HANDLE(handle as *mut c_void)) };
    }
    #[tokio::test]
    async fn inheritable_parent_handles_do_not_cross_the_allowlist() {
        let security = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            bInheritHandle: BOOL(1),
            ..Default::default()
        };
        let event = Handle(unsafe {
            windows::Win32::System::Threading::CreateEventW(
                Some(&security),
                true,
                false,
                PCWSTR::null(),
            )
            .unwrap()
        });
        let job = OwnedJob::new().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "computer_use_windows::process::tests::child_probes_inherited_event",
            ])
            .env(
                "CLOVY_TEST_PRIVATE_HANDLE",
                (event.0 .0 as usize).to_string(),
            );
        let mut child = spawn(command, &job).unwrap();
        assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
        assert_eq!(unsafe { WaitForSingleObject(event.0, 0) }, WAIT_TIMEOUT);
    }

    #[test]
    #[ignore = "private subprocess fixture for the authorized Windows parent/job test"]
    fn application_launch_job_probe() {
        let parent_pid: u32 = std::env::var("CLOVY_TEST_APP_PARENT_PID")
            .unwrap()
            .parse()
            .unwrap();
        let parent = Handle(unsafe {
            OpenProcess(
                PROCESS_CREATE_PROCESS | PROCESS_QUERY_LIMITED_INFORMATION,
                false,
                parent_pid,
            )
            .unwrap()
        });
        let executable = PathBuf::from(std::env::var_os("CLOVY_TEST_APP_EXECUTABLE").unwrap());
        let receipt = PathBuf::from(std::env::var_os("CLOVY_TEST_APP_RECEIPT").unwrap());
        let environment = application_environment().unwrap();
        assert!(
            crate::computer_use_policy::windows_compatibility_layers_allowed(
                environment.compatibility_layer.as_deref()
            )
        );
        let created = launch_application(&executable, parent.0, &environment, || Ok(())).unwrap();
        let identity = process_identity_from_handle(created.handle.0).unwrap();
        let value =
            json!({"pid":created.pid,"path":identity.path,"creation_time":identity.creation_time});
        let pending = receipt.with_extension("pending");
        std::fs::write(&pending, serde_json::to_vec(&value).unwrap()).unwrap();
        std::fs::rename(pending, receipt).unwrap();
        // The outer test terminates this controller job. The created fixture
        // is bounded even if the outer fixture fails before it can clean up.
        std::thread::sleep(Duration::from_secs(30));
    }

    #[tokio::test]
    #[ignore = "authorized native Windows process fixture; requires the installed Rust compiler"]
    async fn application_parent_is_outside_the_controller_job() {
        use windows::Win32::System::{
            JobObjects::IsProcessInJob,
            Threading::{TerminateProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE},
        };
        struct FixtureProcess(Handle);
        impl Drop for FixtureProcess {
            fn drop(&mut self) {
                unsafe {
                    let _ = TerminateProcess(self.0 .0, 0);
                    let _ = WaitForSingleObject(self.0 .0, 2000);
                }
            }
        }
        let directory = tempfile::Builder::new()
            .prefix("clovy-owned-app-job-")
            .tempdir()
            .unwrap();
        let source = directory.path().join("owned_fixture.rs");
        let executable = directory.path().join("owned app's fixture.exe");
        // This app performs no I/O, UI, privilege or user-data operation.
        std::fs::write(
            &source,
            "#![windows_subsystem = \"windows\"]\nfn main() { std::thread::sleep(std::time::Duration::from_secs(20)); }",
        )
        .unwrap();
        let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| OsString::from("rustc"));
        let compiled = std::process::Command::new(compiler)
            .args([
                "--edition",
                "2021",
                "--crate-name",
                "clovy_owned_app_job_fixture",
                "-C",
                "debuginfo=0",
            ])
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );

        let receipt = directory.path().join("receipt.json");
        let job = OwnedJob::new().unwrap();
        let mut parent_is_owned = BOOL(0);
        unsafe {
            IsProcessInJob(GetCurrentProcess(), Some(job.0 .0), &mut parent_is_owned).unwrap();
        }
        assert!(!parent_is_owned.as_bool());
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "computer_use_windows::process::tests::application_launch_job_probe",
            ])
            .env(
                "CLOVY_TEST_APP_PARENT_PID",
                unsafe { GetCurrentProcessId() }.to_string(),
            )
            .env("CLOVY_TEST_APP_EXECUTABLE", &executable)
            .env("CLOVY_TEST_APP_RECEIPT", &receipt);
        let mut controller = spawn(command, &job).unwrap();
        let value: Value = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(bytes) = std::fs::read(&receipt) {
                    if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                        break value;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let pid = u32::try_from(value["pid"].as_u64().unwrap()).unwrap();
        let handle = Handle(unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
                false,
                pid,
            )
            .unwrap()
        });
        let identity = process_identity_from_handle(handle.0).unwrap();
        assert!(same_path(&identity.path, &executable));
        assert_eq!(
            Some(identity.creation_time),
            value["creation_time"].as_u64()
        );
        // Ownership is established before this cleanup guard can terminate it.
        let fixture = FixtureProcess(handle);
        let mut in_job = BOOL(0);
        unsafe {
            IsProcessInJob(controller.process.0, Some(job.0 .0), &mut in_job).unwrap();
        }
        assert!(in_job.as_bool());
        unsafe {
            IsProcessInJob(fixture.0 .0, Some(job.0 .0), &mut in_job).unwrap();
        }
        assert!(!in_job.as_bool());
        job.terminate();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), controller.wait())
                .await
                .unwrap()
                .is_ok()
        );
        assert_eq!(
            unsafe { WaitForSingleObject(fixture.0 .0, 0) },
            WAIT_TIMEOUT
        );
    }
}
