//! The whole process tree of a spawned child.
//!
//! A child that forks helpers (dev servers, node workers, MCP servers) must
//! not leave them behind when it is cancelled or times out: killing the child
//! alone would orphan them and leave its pipes open.
//!
//! - Unix: the child is spawned with `process_group(0)`; signalling the group
//!   reaches every helper.
//! - Windows: there are no process groups. The child is created suspended,
//!   placed in a Job Object, and only then resumed, so it cannot start a
//!   process outside the job; its descendants join the job when they are
//!   created, and killing the tree terminates the job. The job is also
//!   closed with `KILL_ON_JOB_CLOSE`, so dropping the tree sweeps whatever is
//!   left (an npm `codex.cmd` runs `cmd.exe` -> `node` -> `codex.exe`). A
//!   child that cannot be placed in the job is killed before it runs and the
//!   spawn fails.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupSignal {
    Terminate,
    Kill,
}

/// The whole tree of a child started by [`ProcessTree::spawn`].
#[derive(Debug)]
pub struct ProcessTree {
    #[cfg(unix)]
    group: Option<u32>,
    #[cfg(windows)]
    job: windows_job::Job,
}

impl ProcessTree {
    /// Spawns `command` with its whole tree adopted from the child's first
    /// instruction.
    pub fn spawn(
        command: &mut tokio::process::Command,
    ) -> std::io::Result<(tokio::process::Child, Self)> {
        Self::spawn_with(command, |_| {})
    }

    /// [`Self::spawn`], running `between` once the child exists and before
    /// its tree is adopted; tests widen that window with it.
    fn spawn_with(
        command: &mut tokio::process::Command,
        between: impl FnOnce(u32),
    ) -> std::io::Result<(tokio::process::Child, Self)> {
        #[cfg(unix)]
        {
            command.process_group(0);
            let child = command.spawn()?;
            let group = child.id();
            if let Some(pid) = group {
                between(pid);
            }
            Ok((child, Self { group }))
        }
        #[cfg(windows)]
        {
            let job = windows_job::Job::new()?;
            command.creation_flags(
                crate::hide_console::CREATE_NO_WINDOW | windows_job::CREATE_SUSPENDED,
            );
            let mut child = command.spawn()?;
            if let Some(pid) = child.id() {
                between(pid);
            }
            let placed = child
                .raw_handle()
                .zip(child.id())
                .ok_or_else(|| std::io::Error::other("the process has no handle"))
                .and_then(|(handle, pid)| {
                    job.assign(handle)?;
                    windows_job::resume_first_thread(pid)
                });
            if let Err(error) = placed {
                // Still suspended: it never ran.
                let _ = child.start_kill();
                return Err(std::io::Error::other(format!(
                    "could not place the process in a job object: {error}"
                )));
            }
            Ok((child, Self { job }))
        }
        #[cfg(not(any(unix, windows)))]
        {
            let child = command.spawn()?;
            if let Some(pid) = child.id() {
                between(pid);
            }
            Ok((child, Self {}))
        }
    }

    /// Sends `signal` to the whole tree. Windows has no graceful signal for
    /// another process, so `Terminate` does nothing there and only `Kill`
    /// ends the tree.
    pub fn signal(&self, signal: GroupSignal) {
        #[cfg(unix)]
        signal_group(self.group, signal);
        #[cfg(windows)]
        if signal == GroupSignal::Kill {
            self.job.terminate();
        }
        #[cfg(not(any(unix, windows)))]
        let _ = signal;
    }

    /// Kills the whole tree.
    pub fn kill(&self) {
        self.signal(GroupSignal::Kill);
    }
}

/// Sends `signal` to the group led by `group` (the child's pid). Does nothing
/// for a missing id or for pids 0 and 1.
#[cfg(unix)]
fn signal_group(group: Option<u32>, signal: GroupSignal) {
    let Some(group) = group.and_then(|pid| i32::try_from(pid).ok()) else {
        return;
    };
    if group <= 1 {
        return;
    }
    let signal = match signal {
        GroupSignal::Terminate => libc::SIGTERM,
        GroupSignal::Kill => libc::SIGKILL,
    };
    // SAFETY: killpg only signals; the group id is the child's own pid because
    // it was spawned with `process_group(0)`, so this never reaches Clovy.
    unsafe {
        libc::killpg(group, signal);
    }
}

#[cfg(windows)]
pub(crate) mod windows_job {
    use std::io;
    use windows::core::PCWSTR;
    use windows::Win32::{
        Foundation::{
            CloseHandle, GetLastError, SetLastError, ERROR_ALREADY_EXISTS, HANDLE, WIN32_ERROR,
        },
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                THREADENTRY32,
            },
            JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicAccountingInformation,
                JobObjectExtendedLimitInformation, QueryInformationJobObject,
                SetInformationJobObject, TerminateJobObject,
                JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            },
            Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
        },
    };

    /// `CREATE_SUSPENDED`: the child's first thread waits for
    /// [`resume_first_thread`].
    pub const CREATE_SUSPENDED: u32 = 0x0000_0004;

    /// A Job Object handle that kills its processes when closed.
    #[derive(Debug)]
    pub struct Job(isize);

    // SAFETY: a job handle is a kernel handle usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// A new, empty kill-on-close job.
        pub fn new() -> io::Result<Self> {
            Self::create(PCWSTR::null(), false)
        }

        /// Browser-only named job. An existing object must never be adopted.
        pub(crate) fn new_named(name: &str) -> io::Result<Self> {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = std::ffi::OsStr::new(name)
                .encode_wide()
                .chain(Some(0))
                .collect();
            Self::create(PCWSTR(wide.as_ptr()), true)
        }

        fn create(name: PCWSTR, reject_existing: bool) -> io::Result<Self> {
            // SAFETY: the job handle is closed by `Drop`, also on the error
            // path below.
            unsafe {
                if reject_existing {
                    SetLastError(WIN32_ERROR(0));
                }
                let job = Job(CreateJobObjectW(None, name)?.0 as isize);
                if reject_existing && GetLastError() == ERROR_ALREADY_EXISTS {
                    return Err(io::Error::other("browser job already exists"));
                }
                let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                SetInformationJobObject(
                    job.handle(),
                    JobObjectExtendedLimitInformation,
                    std::ptr::from_ref(&limits).cast(),
                    std::mem::size_of_val(&limits) as u32,
                )?;
                Ok(job)
            }
        }

        /// Places the process behind `process` (its own handle, still open)
        /// in the job.
        pub fn assign(&self, process: std::os::windows::io::RawHandle) -> io::Result<()> {
            // SAFETY: both handles are open; the process handle is borrowed.
            unsafe { AssignProcessToJobObject(self.handle(), HANDLE(process)) }?;
            Ok(())
        }

        fn handle(&self) -> HANDLE {
            HANDLE(self.0 as *mut _)
        }

        pub fn terminate(&self) {
            // SAFETY: the handle is open for the lifetime of `self`.
            unsafe {
                let _ = TerminateJobObject(self.handle(), 1);
            }
        }

        /// Reaping the leader alone does not prove all descendants have
        /// exited. Browser profile leases stay held until the Job is empty.
        pub(crate) fn is_empty(&self) -> io::Result<bool> {
            let mut accounting = JOBOBJECT_BASIC_ACCOUNTING_INFORMATION::default();
            unsafe {
                QueryInformationJobObject(
                    Some(self.handle()),
                    JobObjectBasicAccountingInformation,
                    std::ptr::from_mut(&mut accounting).cast(),
                    std::mem::size_of_val(&accounting) as u32,
                    None,
                )
            }?;
            Ok(accounting.ActiveProcesses == 0)
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closing the last handle kills what is left in the job
            // (KILL_ON_JOB_CLOSE); the handle is not used afterwards.
            unsafe {
                let _ = CloseHandle(self.handle());
            }
        }
    }

    /// Resumes the first thread of the suspended process `pid`. A process
    /// created suspended has exactly that one thread.
    pub fn resume_first_thread(pid: u32) -> io::Result<()> {
        // SAFETY: the snapshot and thread handles are closed before return;
        // the entry is sized as the API requires.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)?;
            let mut entry = THREADENTRY32 {
                dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                ..Default::default()
            };
            let mut found = Thread32First(snapshot, &mut entry);
            let mut resumed = Err(io::Error::other("its first thread was not found"));
            while found.is_ok() {
                if entry.th32OwnerProcessID == pid {
                    resumed = match OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) {
                        Ok(thread) => {
                            let count = ResumeThread(thread);
                            let _ = CloseHandle(thread);
                            if count == u32::MAX {
                                Err(io::Error::last_os_error())
                            } else {
                                Ok(())
                            }
                        }
                        Err(error) => Err(error.into()),
                    };
                    break;
                }
                found = Thread32Next(snapshot, &mut entry);
            }
            let _ = CloseHandle(snapshot);
            resumed
        }
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use crate::llm::cmd_shim_tests::{
        kill_pid, npm_cmd_cli_with_child, process_alive, wait_for_pid,
    };
    use std::{process::Stdio, time::Duration};

    /// `cmd.exe` → `node` → child, where the child starts while Clovy is
    /// still between `spawn()` and placing the CLI in its job: killing the
    /// tree must still end it.
    #[tokio::test]
    async fn a_child_started_before_the_tree_is_adopted_dies_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let program = npm_cmd_cli_with_child(dir.path(), "codex", &pid_file, "started");
        let mut command = crate::hide_console::tokio_command(&program);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let window_pid = pid_file.clone();
        let (mut child, tree) = ProcessTree::spawn_with(&mut command, move |_| {
            // A wide window: a running CLI starts its child in it.
            let _ = wait_for_pid(&window_pid, Duration::from_secs(15));
        })
        .expect("spawn");
        let pid_path = pid_file.clone();
        let grandchild =
            tokio::task::spawn_blocking(move || wait_for_pid(&pid_path, Duration::from_secs(30)))
                .await
                .unwrap()
                .expect("child never started");
        tree.kill();
        let _ = child.wait().await;
        let alive = process_alive(grandchild);
        kill_pid(grandchild);
        assert!(!alive, "child {grandchild} escaped the job");
    }
}
