use std::process::Command;

fn main() {
    if let Err(error) = run() {
        eprintln!("Windows development runner: {error}");
        std::process::exit(1);
    }
}

fn run() -> std::io::Result<()> {
    #[cfg(windows)]
    retain_process_job()?;
    let mut command = Command::new(env!("CLOVY_WINDOWS_DEV_NODE"));
    command
        .arg(env!("CLOVY_WINDOWS_DEV_SCRIPT"))
        .args(std::env::args_os().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec())
    }
    #[cfg(windows)]
    {
        let result = command.status()?;
        std::process::exit(result.code().unwrap_or(1));
    }
}

// Tauri kills only its direct runner process during rebuild. Retain a
// non-inherited job handle until process exit so Node/Cargo/Clovy cannot outlive
// that runner. Assign this process before any child exists; descendants inherit
// the job without a CreateProcess/AssignProcess race or breakaway permission.
#[cfg(windows)]
fn retain_process_job() -> std::io::Result<()> {
    use std::ffi::c_void;
    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        process_time: i64,
        job_time: i64,
        flags: u32,
        minimum_working_set: usize,
        maximum_working_set: usize,
        active_processes: u32,
        affinity: usize,
        priority: u32,
        scheduling: u32,
    }
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }
    #[cfg(target_pointer_width = "64")]
    const _: [(); 144] = [(); std::mem::size_of::<ExtendedLimits>()];
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            data: *const c_void,
            size: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(std::io::Error::last_os_error());
    }
    let mut limits = ExtendedLimits::default();
    limits.basic.flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE; no breakaway.
    if unsafe {
        SetInformationJobObject(
            job,
            9,
            (&limits as *const ExtendedLimits).cast(),
            std::mem::size_of::<ExtendedLimits>() as u32,
        )
    } == 0
        || unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } == 0
    {
        let error = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(job);
        }
        return Err(error);
    }
    // Deliberately keep the handle open until OS process teardown. Closing it
    // here would terminate the runner itself before preserving the child code.
    Ok(())
}
