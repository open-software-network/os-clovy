//! Optional video runs independently from the audio callback and startup handshake.
use crate::{
    db::repositories::Repositories,
    domain::types::{AppError, MeetingWindowDto},
};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
#[path = "mp4_recovery.rs"]
pub mod mp4_recovery;
#[path = "video_window_selection.rs"]
mod selection;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoResult {
    #[serde(default)]
    pub offset_ms: i64,
    #[serde(default)]
    pub app: String,
    #[serde(default)]
    pub title: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub duration_ms: Option<i64>,
    pub failure_reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    event: String,
    pid: u32,
    started_at_host_ms: Option<i64>,
    process_start_sec: Option<u64>,
    process_start_usec: Option<u64>,
    capture_nonce: Option<String>,
    audio_offset_ms: Option<i64>,
    #[serde(flatten)]
    result: VideoResult,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelperProcessRecord {
    pid: u32,
    process_start_sec: Option<u64>,
    process_start_usec: Option<u64>,
    capture_nonce: Option<String>,
}
fn read_owned_sidecar(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > 16_384 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(16_385).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= 16_384).then_some(bytes)
}
fn read_process_record(path: &Path) -> Option<HelperProcessRecord> {
    let text = String::from_utf8(read_owned_sidecar(path)?).ok()?;
    serde_json::from_str(&text).ok().or_else(|| {
        text.trim().parse().ok().map(|pid| HelperProcessRecord {
            pid,
            process_start_sec: None,
            process_start_usec: None,
            capture_nonce: None,
        })
    })
}
#[derive(Clone, Debug)]
enum HelperProcessProbe {
    Gone,
    Unknown,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Running {
        start_sec: u64,
        start_usec: u64,
        executable: PathBuf,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelperProcessMatch {
    Matches,
    GoneOrReused,
    Unknown,
}
fn match_helper_process(
    record: &HelperProcessRecord,
    probe: HelperProcessProbe,
) -> HelperProcessMatch {
    match probe {
        HelperProcessProbe::Gone => HelperProcessMatch::GoneOrReused,
        HelperProcessProbe::Unknown => HelperProcessMatch::Unknown,
        HelperProcessProbe::Running {
            start_sec,
            start_usec,
            executable,
        } => {
            if executable
                .file_name()
                .map_or(true, |name| name != "clovy-video-capture")
            {
                return HelperProcessMatch::GoneOrReused;
            }
            match (record.process_start_sec, record.process_start_usec) {
                (Some(sec), Some(usec)) if (sec, usec) == (start_sec, start_usec) => {
                    HelperProcessMatch::Matches
                }
                (Some(_), Some(_)) => HelperProcessMatch::GoneOrReused,
                _ => HelperProcessMatch::Unknown,
            }
        }
    }
}
fn helper_process_active(record: &HelperProcessRecord, probe: HelperProcessProbe) -> bool {
    match_helper_process(record, probe) != HelperProcessMatch::GoneOrReused
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HelperSignal {
    Pause,
    Resume,
    Terminate,
    Kill,
}
trait HelperProcessControl {
    fn ownership(&self, _: &HelperProcessRecord) -> bool {
        true
    }
    fn identity(&self, record: &HelperProcessRecord) -> HelperProcessMatch;
    fn send(&self, pid: u32, number: HelperSignal);
}
fn record_from_status(status: &Status) -> HelperProcessRecord {
    HelperProcessRecord {
        pid: status.pid,
        process_start_sec: status.process_start_sec,
        process_start_usec: status.process_start_usec,
        capture_nonce: status.capture_nonce.clone(),
    }
}
fn same_attempt_process(left: &HelperProcessRecord, right: &HelperProcessRecord) -> bool {
    left.pid == right.pid
        && left.process_start_sec == right.process_start_sec
        && left.process_start_usec == right.process_start_usec
        && left.capture_nonce == right.capture_nonce
}

struct NativeHelperProcessControl<'a> {
    expected_nonce: &'a str,
}
impl HelperProcessControl for NativeHelperProcessControl<'_> {
    fn ownership(&self, record: &HelperProcessRecord) -> bool {
        record.capture_nonce.as_deref() == Some(self.expected_nonce)
    }
    fn identity(&self, record: &HelperProcessRecord) -> HelperProcessMatch {
        if !self.ownership(record) {
            return HelperProcessMatch::GoneOrReused;
        }
        match_helper_process(record, probe_helper_process(record.pid))
    }
    fn send(&self, pid: u32, number: HelperSignal) {
        kernel_signal(pid, number);
    }
}
fn signal_verified(
    record: &HelperProcessRecord,
    number: HelperSignal,
    process: &impl HelperProcessControl,
) {
    if record.pid > 1
        && process.ownership(record)
        && process.identity(record) == HelperProcessMatch::Matches
    {
        process.send(record.pid, number);
    }
}
#[cfg(target_os = "macos")]
fn probe_helper_process(pid: u32) -> HelperProcessProbe {
    let Ok(pid) = i32::try_from(pid) else {
        return HelperProcessProbe::Gone;
    };
    // Kernel calls are bounded; no process spawning or permission UI participates in recovery.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let found = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            size,
        )
    };
    if found != size {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            HelperProcessProbe::Gone
        } else {
            HelperProcessProbe::Unknown
        };
    }
    let mut path = [0_u8; 4096];
    let len = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
    if len <= 0 {
        return HelperProcessProbe::Unknown;
    }
    let end = path
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(path.len());
    let executable = PathBuf::from(String::from_utf8_lossy(&path[..end]).into_owned());
    HelperProcessProbe::Running {
        start_sec: info.pbi_start_tvsec,
        start_usec: info.pbi_start_tvusec,
        executable,
    }
}
#[cfg(not(target_os = "macos"))]
fn probe_helper_process(pid: u32) -> HelperProcessProbe {
    if running(pid) {
        HelperProcessProbe::Unknown
    } else {
        HelperProcessProbe::Gone
    }
}

#[derive(Clone, Copy)]
enum Action {
    Pause(i64),
    Resume(i64),
    Stop,
}
/// Fixed writes are <= PIPE_BUF: O_NONBLOCK either queues one complete record or fails video.
/// Creation happens only after audio is capturing; pause/resume perform no filesystem operation.
struct DirectControlPipe {
    #[cfg(unix)]
    file: Option<std::fs::File>,
    sequence: AtomicU64,
    failed: Arc<AtomicBool>,
}
impl DirectControlPipe {
    fn create(partial: &Path, failed: Arc<AtomicBool>) -> Self {
        #[cfg(unix)]
        let file = (|| -> std::io::Result<std::fs::File> {
            use std::os::unix::{
                ffi::OsStrExt,
                fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
            };
            let path = partial.with_extension("control.pipe");
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            let name = std::ffi::CString::new(path.as_os_str().as_bytes())?;
            if unsafe { libc::mkfifo(name.as_ptr(), 0o600) } != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?;
            let metadata = file.metadata()?;
            if !metadata.file_type().is_fifo()
                || metadata.mode() & 0o777 != 0o600
                || metadata.uid() != unsafe { libc::geteuid() }
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "invalid video control pipe",
                ));
            }
            Ok(file)
        })()
        .ok();
        #[cfg(unix)]
        if file.is_none() {
            failed.store(true, Ordering::Release);
        }
        #[cfg(not(unix))]
        {
            let _ = partial;
            failed.store(true, Ordering::Release);
        }
        Self {
            #[cfg(unix)]
            file,
            sequence: AtomicU64::new(0),
            failed,
        }
    }
    fn publish(&self, paused: bool, at: i64) {
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let mut record = [0_u8; 24];
        record[..8].copy_from_slice(&sequence.to_le_bytes());
        record[8..16].copy_from_slice(&at.to_le_bytes());
        record[16..].copy_from_slice(&u64::from(paused).to_le_bytes());
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let written = self.file.as_ref().map(|file| unsafe {
                libc::write(file.as_raw_fd(), record.as_ptr().cast(), record.len())
            });
            if written != Some(record.len() as isize) {
                self.failed.store(true, Ordering::Release);
            }
        }
    }
}

pub struct VideoCapture {
    controls: Arc<DirectControlPipe>,
    sender: mpsc::Sender<Action>,
    worker: Option<thread::JoinHandle<()>>,
}
impl VideoCapture {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        repos: Repositories,
        session_id: String,
        partial: PathBuf,
        final_path: PathBuf,
        window_id: u32,
        offset_ms: i64,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let (sender, receiver) = mpsc::channel();
        let audio_started_ms = host_ms().saturating_sub(offset_ms);
        let cancelled = Arc::new(AtomicBool::new(false));
        let controls = Arc::new(DirectControlPipe::create(&partial, cancelled.clone()));
        let worker_controls = controls.clone();
        let (done_sender, done_receiver) = tokio::sync::oneshot::channel();
        runtime.spawn(monitor_video_owner(
            repos.clone(),
            session_id.clone(),
            sender.clone(),
            done_receiver,
            cancelled.clone(),
            partial.with_extension("lease"),
        ));
        let worker = thread::spawn(move || {
            let _keep_control_pipe_alive = worker_controls;
            let capture_nonce = uuid::Uuid::new_v4().to_string();
            let mut result = run_helper(
                &partial,
                &final_path,
                window_id,
                &session_id,
                &capture_nonce,
                audio_started_ms,
                receiver,
                &cancelled,
                |result| {
                    // Keep first-frame timing durable even if the app later crashes.
                    let result = result.clone();
                    let checkpoint_repos = repos.clone();
                    let checkpoint_session_id = session_id.clone();
                    runtime.spawn(async move {
                        let checkpoint=tokio::time::timeout(
                            Duration::from_secs(2),
                            sqlx::query::query("UPDATE video_artifacts SET offset_ms=?,app=?,title=?,width=?,height=? WHERE recording_session_id=?")
                                .bind(result.offset_ms)
                                .bind(&result.app)
                                .bind(&result.title)
                                .bind(result.width)
                                .bind(result.height)
                                .bind(&checkpoint_session_id)
                                .execute(&checkpoint_repos.pool),
                        ).await;
                        if !matches!(checkpoint, Ok(Ok(_))) { eprintln!("failed to checkpoint video timing: {checkpoint:?}"); }
                    });
                },
            );
            if cancelled.load(Ordering::Acquire) {
                result.failure_reason = Some("capture_failed".into());
            }
            runtime.block_on(finalize_video_worker(
                &repos,
                &session_id,
                &partial,
                &final_path,
                &result,
                &cancelled,
            ));
            let _ = done_sender.send(());
        });
        Self {
            controls,
            sender,
            worker: Some(worker),
        }
    }
    pub fn pause(&self) {
        let at = host_ms();
        self.controls.publish(true, at);
        let _ = self.sender.send(Action::Pause(at));
    }
    pub fn resume(&self) {
        let at = host_ms();
        self.controls.publish(false, at);
        let _ = self.sender.send(Action::Resume(at));
    }
    #[cfg(all(test, unix))]
    pub(super) fn test_control_pipe(partial: &Path) -> Self {
        let (sender, _receiver) = mpsc::channel();
        Self {
            controls: Arc::new(DirectControlPipe::create(
                partial,
                Arc::new(AtomicBool::new(false)),
            )),
            sender,
            worker: None,
        }
    }
    pub fn stop(self) {
        drop(self);
    }
}
impl Drop for VideoCapture {
    fn drop(&mut self) {
        let _ = self.sender.send(Action::Stop);
        // The worker owns a bounded helper shutdown. Never join it on the audio path.
        let _ = self.worker.take();
    }
}
/// Deletion ownership is polled separately so a busy database never delays capture controls.
async fn monitor_video_owner(
    repos: Repositories,
    session_id: String,
    sender: mpsc::Sender<Action>,
    mut done: tokio::sync::oneshot::Receiver<()>,
    cancelled: Arc<AtomicBool>,
    lease: PathBuf,
) {
    loop {
        let owner = tokio::select! {
            _ = &mut done => return,
            result = tokio::time::timeout(Duration::from_secs(2), repos.video_artifact_exists(&session_id)) => result,
        };
        if matches!(owner, Ok(Ok(false))) {
            cancelled.store(true, Ordering::Release);
            let _ = sender.send(Action::Stop);
            let _ =
                tokio::time::timeout(Duration::from_secs(2), tokio::fs::remove_file(&lease)).await;
            return;
        }
        // Errors/timeouts are not evidence of deletion; keep the recording and its files.
        tokio::select! {
            _ = &mut done => return,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {},
        }
    }
}

async fn finalize_video_worker(
    repos: &Repositories,
    session_id: &str,
    partial: &Path,
    final_path: &Path,
    result: &VideoResult,
    cancelled: &AtomicBool,
) {
    // A very late LaunchServices child must observe revocation before creating any output.
    let _ = std::fs::remove_file(partial.with_extension("lease"));
    let saved = tokio::time::timeout(
        Duration::from_secs(2),
        repos.finalize_video_artifact(session_id, result),
    )
    .await;
    if matches!(saved, Ok(Ok(false))) || cancelled.load(Ordering::Acquire) {
        let _ = std::fs::remove_file(partial);
        let _ = std::fs::remove_file(final_path);
    } else if !matches!(saved, Ok(Ok(true))) {
        eprintln!("failed to persist video artifact: {saved:?}");
        return;
    }
    // run_helper has observed a terminal status or revoked the startup lease before returning.
    let _ = std::fs::remove_file(partial.with_extension("status.json"));
    let _ = std::fs::remove_file(partial.with_extension("pid"));
    let _ = std::fs::remove_file(partial.with_extension("control.json"));
    let _ = std::fs::remove_file(partial.with_extension("control.tmp"));
    let _ = std::fs::remove_file(partial.with_extension("control.pipe"));
    let _ = std::fs::remove_file(partial.with_extension("timing.json"));
    let _ = std::fs::remove_file(partial.with_extension("capture.json"));
    let _ = std::fs::remove_file(partial.with_extension("capture.tmp"));
}

#[derive(Default)]
struct PauseTimeline {
    active: Option<i64>,
    intervals: Vec<(i64, i64)>,
}
impl PauseTimeline {
    fn pause(&mut self, at: i64) {
        if self.active.is_none() {
            self.active = Some(at);
        }
    }
    fn resume(&mut self, at: i64) {
        if let Some(start) = self.active.take() {
            self.intervals.push((start, at.max(start)));
        }
    }
    fn duration_before(&self, at: i64) -> i64 {
        self.intervals
            .iter()
            .map(|(start, end)| at.min(*end).saturating_sub(*start).max(0))
            .sum::<i64>()
            + self
                .active
                .map_or(0, |start| at.saturating_sub(start).max(0))
    }
}
#[cfg(target_os = "macos")]
fn host_ms() -> i64 {
    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }
    extern "C" {
        fn mach_absolute_time() -> u64;
        fn mach_timebase_info(info: *mut Timebase) -> i32;
    }
    static SCALE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
    let (numer, denom) = *SCALE.get_or_init(|| {
        let mut info = Timebase { numer: 1, denom: 1 };
        if unsafe { mach_timebase_info(&mut info) } != 0 || info.denom == 0 {
            (1, 1)
        } else {
            (info.numer, info.denom)
        }
    });
    ((unsafe { mach_absolute_time() } as u128 * numer as u128 / denom as u128) / 1_000_000)
        .min(i64::MAX as u128) as i64
}
#[cfg(not(target_os = "macos"))]
fn host_ms() -> i64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(i64::MAX as u128) as i64
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VideoControlEvent {
    sequence: u64,
    paused: bool,
    at_host_ms: i64,
}
#[derive(Default)]
struct ControlLedger {
    events: Vec<VideoControlEvent>,
}
impl ControlLedger {
    fn push(&mut self, paused: bool, at_host_ms: i64) {
        self.events.push(VideoControlEvent {
            sequence: self.events.len() as u64 + 1,
            paused,
            at_host_ms,
        });
    }
    fn publish(&self, path: &Path) -> std::io::Result<()> {
        let bytes = serde_json::to_vec(&self.events).map_err(std::io::Error::other)?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)
    }
}

fn helper_path() -> PathBuf {
    if let Ok(executable) = std::env::current_exe() {
        if let Some(contents) = executable
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == "Contents"))
        {
            let bundled = contents.join("Resources/native/bin/Clovy Video Capture.app");
            if bundled.is_dir() || !cfg!(debug_assertions) {
                return bundled;
            }
        }
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .join(".tauri-helper/Clovy Video Capture.app")
}

#[allow(clippy::too_many_arguments)]
fn run_helper(
    partial: &Path,
    final_path: &Path,
    window_id: u32,
    recording_session_id: &str,
    capture_nonce: &str,
    audio_started_ms: i64,
    receiver: mpsc::Receiver<Action>,
    cancelled: &AtomicBool,
    checkpoint: impl Fn(&VideoResult),
) -> VideoResult {
    let mut result = VideoResult::default();
    if !cfg!(target_os = "macos") {
        result.failure_reason = Some("capture_failed".into());
        return result;
    }
    let helper = helper_path();
    if !helper.is_dir() {
        result.failure_reason = Some("capture_failed".into());
        return result;
    }
    let microphone = crate::meeting_detection::active_input_processes()
        .unwrap_or_default()
        .into_iter()
        .map(|process| process.bundle_id)
        .collect::<Vec<_>>();
    let selected = enumerate_windows()
        .unwrap_or_default()
        .into_iter()
        .find(|window| window.id == window_id);
    let Some(selected) = selected.filter(|window| selection::is_candidate(window, &microphone))
    else {
        result.failure_reason = Some("capture_failed".into());
        return result;
    };
    let microphone_owned = selection::microphone_owned(&selected, &microphone);
    if cancelled.load(Ordering::Acquire)
        || write_capture_binding(
            partial,
            recording_session_id,
            capture_nonce,
            audio_started_ms,
        )
        .is_err()
    {
        result.failure_reason = Some("capture_failed".into());
        return result;
    }
    run_helper_with_launcher_and_process(
        partial,
        final_path,
        audio_started_ms,
        receiver,
        cancelled,
        checkpoint,
        |status_path, pid_path| {
            let child = std::process::Command::new("/usr/bin/open")
                .args(["-n", "-a"])
                .arg(helper)
                .arg("--args")
                .arg("--recording-session")
                .arg(recording_session_id)
                .arg("--capture-nonce")
                .arg(capture_nonce)
                .arg("--audio-started-host-ms")
                .arg(audio_started_ms.to_string())
                .arg("--parent")
                .arg(std::process::id().to_string())
                .arg("--window")
                .arg(window_id.to_string())
                .arg("--microphone-owned")
                .arg(microphone_owned.to_string())
                .arg("--output")
                .arg(partial)
                .arg("--status")
                .arg(status_path)
                .arg("--pid")
                .arg(pid_path)
                .arg("--lease")
                .arg(partial.with_extension("lease"))
                .arg("--controls")
                .arg(partial.with_extension("control.json"))
                .spawn();
            let Ok(mut child) = child else { return false };
            let deadline = Instant::now();
            while deadline.elapsed() < Duration::from_secs(3) {
                match child.try_wait() {
                    Ok(Some(status)) => return status.success(),
                    Err(_) => break,
                    Ok(None) => thread::sleep(Duration::from_millis(20)),
                }
            }
            let _ = child.kill();
            let _ = child.wait();
            false
        },
        &NativeHelperProcessControl {
            expected_nonce: capture_nonce,
        },
    )
}
// Existing fake-helper tests exercise orchestration with their explicitly owned shell process.
// Production always uses NativeHelperProcessControl and requires strict kernel identity.
#[cfg(test)]
struct FixtureProcessControl;
#[cfg(test)]
impl HelperProcessControl for FixtureProcessControl {
    fn identity(&self, record: &HelperProcessRecord) -> HelperProcessMatch {
        if running(record.pid) {
            HelperProcessMatch::Matches
        } else {
            HelperProcessMatch::GoneOrReused
        }
    }
    fn send(&self, pid: u32, number: HelperSignal) {
        kernel_signal(pid, number);
    }
}
#[cfg(test)]
fn run_helper_with_launcher(
    partial: &Path,
    final_path: &Path,
    audio_started_ms: i64,
    receiver: mpsc::Receiver<Action>,
    cancelled: &AtomicBool,
    checkpoint: impl Fn(&VideoResult),
    launch: impl FnOnce(&Path, &Path) -> bool,
) -> VideoResult {
    run_helper_with_launcher_and_process(
        partial,
        final_path,
        audio_started_ms,
        receiver,
        cancelled,
        checkpoint,
        launch,
        &FixtureProcessControl,
    )
}
#[allow(clippy::too_many_arguments)]
fn run_helper_with_launcher_and_process(
    partial: &Path,
    final_path: &Path,
    audio_started_ms: i64,
    receiver: mpsc::Receiver<Action>,
    cancelled: &AtomicBool,
    checkpoint: impl Fn(&VideoResult),
    launch: impl FnOnce(&Path, &Path) -> bool,
    process: &impl HelperProcessControl,
) -> VideoResult {
    let mut result = VideoResult::default();
    let status_path = partial.with_extension("status.json");
    let pid_path = partial.with_extension("pid");
    let _ = std::fs::remove_file(&status_path);
    let _ = std::fs::remove_file(&pid_path);
    let mut paused = false;
    let mut pauses = PauseTimeline::default();
    let mut controls = ControlLedger::default();
    let controls_path = partial.with_extension("control.json");
    let mut control_failed = false;
    for action in receiver.try_iter() {
        match action {
            Action::Pause(at) => {
                paused = true;
                pauses.pause(at);
                controls.push(true, at);
            }
            Action::Resume(at) => {
                paused = false;
                pauses.resume(at);
                controls.push(false, at);
            }
            Action::Stop => {
                result.failure_reason = Some("capture_failed".into());
                return result;
            }
        }
    }
    let lease = partial.with_extension("lease");
    if cancelled.load(Ordering::Acquire) || std::fs::write(&lease, b"active").is_err() {
        result.failure_reason = Some("capture_failed".into());
        return result;
    }
    if cancelled.load(Ordering::Acquire) {
        let _ = std::fs::remove_file(&lease);
        result.failure_reason = Some("capture_failed".into());
        return result;
    }
    if controls.publish(&controls_path).is_err() {
        let _ = std::fs::remove_file(&lease);
        result.failure_reason = Some("capture_failed".into());
        return result;
    }
    if !launch(&status_path, &pid_path) {
        let _ = std::fs::remove_file(&lease);
        result.failure_reason = Some("capture_failed".into());
        return result;
    }
    let launched_at = Instant::now();
    let launched_at_ms = host_ms();
    let mut stop_at = None;
    let mut record: Option<HelperProcessRecord> = None;
    let mut ready = false;
    let mut signaled_pid = None;
    loop {
        if cancelled.load(Ordering::Acquire) {
            let _ = std::fs::remove_file(&lease);
            if stop_at.is_none() {
                stop_at = Some(Instant::now());
            }
        }
        if record.is_none() {
            record = read_process_record(&pid_path);
        }
        if record
            .as_ref()
            .is_some_and(|record| !process.ownership(record))
        {
            result.failure_reason = Some("capture_failed".into());
            control_failed = true;
            let _ = std::fs::remove_file(&lease);
            break;
        }
        let pid = record.as_ref().map(|record| record.pid);
        if pid != signaled_pid {
            if let Some(record) = record.as_ref() {
                if paused {
                    signal_verified(record, HelperSignal::Pause, process);
                }
                if stop_at.is_some() {
                    signal_verified(record, HelperSignal::Terminate, process);
                }
            }
            signaled_pid = pid;
        }
        for action in std::iter::once(receiver.recv_timeout(Duration::from_millis(100)))
            .chain(receiver.try_iter().map(Ok))
        {
            match action {
                Ok(Action::Pause(at)) => {
                    pauses.pause(at);
                    paused = true;
                    controls.push(true, at);
                    if controls.publish(&controls_path).is_err() {
                        control_failed = true;
                        stop_at = Some(Instant::now());
                    }
                    if let Some(record) = record.as_ref() {
                        signal_verified(record, HelperSignal::Pause, process);
                    }
                }
                Ok(Action::Resume(at)) => {
                    pauses.resume(at);
                    paused = false;
                    controls.push(false, at);
                    if controls.publish(&controls_path).is_err() {
                        control_failed = true;
                        stop_at = Some(Instant::now());
                    }
                    if let Some(record) = record.as_ref() {
                        signal_verified(record, HelperSignal::Resume, process);
                    }
                }
                Ok(Action::Stop) => {
                    if stop_at.is_none() {
                        stop_at = Some(Instant::now());
                        if let Some(record) = record.as_ref() {
                            signal_verified(record, HelperSignal::Terminate, process);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    if stop_at.is_none() {
                        stop_at = Some(Instant::now());
                        if let Some(record) = record.as_ref() {
                            signal_verified(record, HelperSignal::Terminate, process);
                        }
                    }
                    thread::sleep(Duration::from_millis(100));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        if let Some(status) = read_helper_status(&status_path) {
            let status_record = record_from_status(&status);
            if !process.ownership(&status_record)
                || record
                    .as_ref()
                    .is_some_and(|old| !same_attempt_process(old, &status_record))
            {
                result.failure_reason = Some("capture_failed".into());
                control_failed = true;
                let _ = std::fs::remove_file(&lease);
                break;
            }
            record = Some(status_record);
            result = result_from_status(&status, audio_started_ms, &pauses);
            if !ready && status.started_at_host_ms.is_some() {
                ready = true;
                checkpoint(&result);
                if paused {
                    if let Some(record) = record.as_ref() {
                        signal_verified(record, HelperSignal::Pause, process);
                    }
                }
            }
            if status.event == "stopped" || status.event == "failed" {
                // Status publication precedes the helper's final lease checks. Keep
                // the lease (and partial path) until its verified process exits so
                // it cannot erase metadata needed after a database commit failure.
                stop_at.get_or_insert_with(Instant::now);
            }
        }
        if let Some(record) = record.as_ref() {
            match process.identity(record) {
                HelperProcessMatch::GoneOrReused => {
                    if let Some(status) = read_helper_status(&status_path).filter(|s| {
                        (s.event == "stopped" || s.event == "failed")
                            && process.ownership(&record_from_status(s))
                            && same_attempt_process(record, &record_from_status(s))
                    }) {
                        result = result_from_status(&status, audio_started_ms, &pauses);
                    } else if stop_at.is_some()
                        && result.failure_reason.is_none()
                        && !control_failed
                        && !cancelled.load(Ordering::Acquire)
                    {
                        // The helper's bounded normal shutdown can exit before publishing
                        // terminal status. Only verified, requested termination may salvage
                        // same-attempt durable timing and committed fragments.
                        if let Some(recovered) =
                            recover_controlled_exit(partial, audio_started_ms, record)
                        {
                            result = recovered;
                        } else {
                            result.failure_reason = Some("capture_failed".into());
                        }
                    } else {
                        result.failure_reason = Some("capture_failed".into());
                    }
                    break;
                }
                HelperProcessMatch::Unknown => {
                    // Unknown is protected from signals. Revoking the lease stops the owned helper.
                    let _ = std::fs::remove_file(&lease);
                    result.failure_reason = Some("capture_failed".into());
                    control_failed = true;
                    stop_at.get_or_insert_with(Instant::now);
                }
                HelperProcessMatch::Matches => {}
            }
        }
        if let Some(stop) = stop_at {
            if cancelled.load(Ordering::Acquire) && record.is_none() {
                break;
            }
            // Stop can precede the helper writing its PID. Repeat until acknowledged.
            if let Some(record) = record.as_ref() {
                signal_verified(record, HelperSignal::Terminate, process);
            }
            if stop.elapsed() > Duration::from_secs(6) {
                if let Some(record) = record.as_ref() {
                    signal_verified(record, HelperSignal::Kill, process);
                    let settled = Instant::now();
                    while process.identity(record) == HelperProcessMatch::Matches
                        && settled.elapsed() < Duration::from_millis(500)
                    {
                        thread::sleep(Duration::from_millis(20));
                    }
                }
                result.failure_reason = Some("capture_failed".into());
                break;
            }
        } else if !ready
            && active_startup_elapsed(launched_at.elapsed(), launched_at_ms, host_ms(), &pauses)
                > Duration::from_secs(30)
        {
            stop_at = Some(Instant::now());
            result.failure_reason = Some("capture_failed".into());
        }
    }
    if std::fs::metadata(partial).is_ok_and(|m| m.len() > 32) {
        if std::fs::rename(partial, final_path).is_err() {
            result.failure_reason = Some("capture_failed".into());
        }
    } else if result.failure_reason.is_none() {
        result.failure_reason = Some("capture_failed".into());
    }
    let _ = std::fs::remove_file(&lease);
    if control_failed {
        result.failure_reason = Some("capture_failed".into());
    }
    result
}
/// Caller has verified that the requested helper has terminated and no capture/control
/// failure was reported. These are worker-owned session paths, not imported artifact paths.
fn recover_controlled_exit(
    partial: &Path,
    audio_started_ms: i64,
    record: &HelperProcessRecord,
) -> Option<VideoResult> {
    let binding = read_capture_binding(partial)?;
    if binding.partial_path != partial.to_string_lossy()
        || binding.audio_started_host_ms != audio_started_ms
        || record.capture_nonce.as_deref() != Some(binding.capture_nonce.as_str())
    {
        return None;
    }
    let timing = recovery_timing(
        partial,
        &binding.recording_session_id,
        &binding.partial_path,
    )?;
    mp4_recovery::recover(partial).then_some(timing)
}

fn read_helper_status(path: &Path) -> Option<Status> {
    serde_json::from_slice(&read_owned_sidecar(path)?).ok()
}
fn result_from_status(
    status: &Status,
    audio_started_ms: i64,
    pauses: &PauseTimeline,
) -> VideoResult {
    let mut result = status.result.clone();
    if status.event == "failed" && result.failure_reason.is_none() {
        result.failure_reason = Some("capture_failed".into());
    }
    if let Some(offset) = status.audio_offset_ms {
        result.offset_ms = offset;
    } else if let Some(first_frame_ms) = status.started_at_host_ms {
        result.offset_ms = first_frame_ms
            .saturating_sub(audio_started_ms)
            .saturating_sub(pauses.duration_before(first_frame_ms))
            .max(0);
    }
    result
}
fn active_startup_elapsed(
    elapsed: Duration,
    launched_at_ms: i64,
    now_ms: i64,
    pauses: &PauseTimeline,
) -> Duration {
    let paused_ms = pauses
        .duration_before(now_ms)
        .saturating_sub(pauses.duration_before(launched_at_ms))
        .max(0) as u64;
    elapsed.saturating_sub(Duration::from_millis(paused_ms))
}

fn kernel_signal(pid: u32, number: HelperSignal) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        if pid > 1 {
            let number = match number {
                HelperSignal::Pause => libc::SIGUSR1,
                HelperSignal::Resume => libc::SIGUSR2,
                HelperSignal::Terminate => libc::SIGTERM,
                HelperSignal::Kill => libc::SIGKILL,
            };
            unsafe {
                libc::kill(pid, number);
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (pid, number);
}

#[cfg(any(not(target_os = "macos"), test))]
fn running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        i32::try_from(pid).is_ok_and(|pid| {
            pid > 1
                && (unsafe { libc::kill(pid, 0) } == 0
                    || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
        })
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

/// Probe a bounded first second, including a decoded frame, before promoting crash fragments.
#[cfg(test)]
pub async fn playable_mp4(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let env = crate::llm::shell_env::login_env().await;
    let Some(binary) = env.which("ffprobe") else {
        return false;
    };
    let probe = tokio::process::Command::new(binary)
        .args([
            "-v",
            "error",
            "-read_intervals",
            "%+1",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=width,height,nb_read_frames",
            "-of",
            "json",
        ])
        .arg(path)
        .kill_on_drop(true)
        .output();
    let Ok(Ok(output)) = tokio::time::timeout(Duration::from_secs(5), probe).await else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return false;
    };
    value["streams"].as_array().is_some_and(|streams| {
        streams.iter().any(|stream| {
            stream["width"].as_u64().is_some_and(|n| n > 0)
                && stream["height"].as_u64().is_some_and(|n| n > 0)
                && stream["nb_read_frames"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .is_some_and(|n| n > 0)
        })
    })
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CaptureBinding {
    schema: u8,
    recording_session_id: String,
    partial_path: String,
    capture_nonce: String,
    audio_started_host_ms: i64,
    #[serde(default)]
    owner_pid: Option<u32>,
    #[serde(default)]
    owner_start_sec: Option<u64>,
    #[serde(default)]
    owner_start_usec: Option<u64>,
}
/// Background-worker I/O only, committed before LaunchServices can create a capture helper.
pub fn write_capture_binding(
    partial: &Path,
    session_id: &str,
    nonce: &str,
    audio_start: i64,
) -> std::io::Result<()> {
    use std::io::Write;
    if session_id.is_empty() || uuid::Uuid::parse_str(nonce).is_err() {
        return Err(std::io::Error::other("invalid capture binding"));
    }
    let owner_pid = std::process::id();
    let owner_birth = match probe_helper_process(owner_pid) {
        HelperProcessProbe::Running {
            start_sec,
            start_usec,
            ..
        } => Some((start_sec, start_usec)),
        #[cfg(target_os = "macos")]
        _ => return Err(std::io::Error::other("capture owner identity unavailable")),
        // Video launch is unsupported here; retain portable fixture metadata without
        // pretending an unknown host identity proves that a pending launch has died.
        #[cfg(not(target_os = "macos"))]
        _ => None,
    };
    let binding = CaptureBinding {
        schema: 1,
        recording_session_id: session_id.into(),
        partial_path: partial.to_string_lossy().into_owned(),
        capture_nonce: nonce.into(),
        audio_started_host_ms: audio_start,
        owner_pid: Some(owner_pid),
        owner_start_sec: owner_birth.map(|birth| birth.0),
        owner_start_usec: owner_birth.map(|birth| birth.1),
    };
    let bytes = serde_json::to_vec(&binding)?;
    let temporary = partial.with_extension("capture.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, partial.with_extension("capture.json"))?;
    #[cfg(unix)]
    std::fs::File::open(
        partial
            .parent()
            .ok_or_else(|| std::io::Error::other("capture parent missing"))?,
    )?
    .sync_all()?;
    Ok(())
}
fn read_capture_binding(partial: &Path) -> Option<CaptureBinding> {
    let binding: CaptureBinding = serde_json::from_slice(&read_owned_sidecar(
        &partial.with_extension("capture.json"),
    )?)
    .ok()?;
    (binding.schema == 1
        && !binding.recording_session_id.is_empty()
        && uuid::Uuid::parse_str(&binding.capture_nonce).is_ok())
    .then_some(binding)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecoveryTiming {
    schema: u8,
    recording_session_id: String,
    partial_path: String,
    capture_nonce: String,
    audio_started_host_ms: i64,
    started_at_host_ms: i64,
    paused_before_first_ms: i64,
    offset_ms: i64,
    app: String,
    title: String,
    width: i64,
    height: i64,
}
/// AppPaths must validate this exact session and all sidecars before this bounded read.
pub fn recovery_timing(
    partial: &Path,
    session_id: &str,
    stored_partial: &str,
) -> Option<VideoResult> {
    let path = partial.with_extension("timing.json");
    let timing: RecoveryTiming = serde_json::from_slice(&read_owned_sidecar(&path)?).ok()?;
    let binding = read_capture_binding(partial)?;
    let offset = timing
        .started_at_host_ms
        .checked_sub(timing.audio_started_host_ms)?
        .checked_sub(timing.paused_before_first_ms)?;
    if timing.schema != 1
        || timing.recording_session_id != session_id
        || timing.partial_path != stored_partial
        || binding.recording_session_id != session_id
        || binding.partial_path != stored_partial
        || binding.audio_started_host_ms != timing.audio_started_host_ms
        || timing.capture_nonce != binding.capture_nonce
        || uuid::Uuid::parse_str(&timing.capture_nonce).is_err()
        || timing.paused_before_first_ms < 0
        || offset < 0
        || timing.offset_ms != offset
        || timing.width <= 0
        || timing.height <= 0
        || timing.width.max(timing.height) > 1920
        || timing.title.trim().is_empty()
    {
        return None;
    }
    Some(VideoResult {
        offset_ms: offset,
        app: timing.app,
        title: timing.title,
        width: Some(timing.width),
        height: Some(timing.height),
        ..Default::default()
    })
}

pub enum VideoRecoveryState {
    Active,
    Finished(Option<String>),
}
/// Called only after AppPaths has validated the session and sidecar paths.
pub fn video_recovery_state(partial: &Path, stored_partial: &str) -> VideoRecoveryState {
    video_recovery_state_with_binding(partial, stored_partial, probe_helper_process)
}
#[cfg(test)]
fn video_recovery_state_with_probe(
    partial: &Path,
    probe: impl FnMut(u32) -> HelperProcessProbe,
) -> VideoRecoveryState {
    video_recovery_state_with_binding(partial, &partial.to_string_lossy(), probe)
}
fn video_recovery_state_with_binding(
    partial: &Path,
    stored_partial: &str,
    mut probe: impl FnMut(u32) -> HelperProcessProbe,
) -> VideoRecoveryState {
    let binding = read_capture_binding(partial);
    let record = read_process_record(&partial.with_extension("pid")).filter(|record| {
        record.pid > 1
            && binding.as_ref().map_or(true, |binding| {
                record.capture_nonce.as_deref() == Some(binding.capture_nonce.as_str())
            })
    });
    let status = read_helper_status(&partial.with_extension("status.json")).filter(|status| {
        if let Some(binding) = &binding {
            let candidate = record_from_status(status);
            status.capture_nonce.as_deref() == Some(binding.capture_nonce.as_str())
                && record
                    .as_ref()
                    .map_or(true, |record| same_attempt_process(record, &candidate))
        } else {
            status.event != "stopped" && status.event != "failed"
        }
    });
    // A terminal helper status does not release the Rust worker's ownership.
    // It still has to rename the partial file before revoking this lease.
    if (record.is_some() || status.is_some()) && partial.with_extension("lease").exists() {
        if let Some(binding) = binding.as_ref() {
            let owner_active = match (
                binding.owner_pid,
                binding.owner_start_sec,
                binding.owner_start_usec,
            ) {
                (Some(pid), Some(sec), Some(usec)) if pid > 1 => match probe(pid) {
                    HelperProcessProbe::Gone => false,
                    HelperProcessProbe::Unknown => true,
                    HelperProcessProbe::Running {
                        start_sec,
                        start_usec,
                        ..
                    } => (sec, usec) == (start_sec, start_usec),
                },
                _ => true,
            };
            if owner_active {
                return VideoRecoveryState::Active;
            }
        }
    }
    if let Some(status) = status
        .as_ref()
        .filter(|status| status.event == "stopped" || status.event == "failed")
    {
        let terminal_record = record_from_status(status);
        if terminal_record.pid > 1
            && helper_process_active(&terminal_record, probe(terminal_record.pid))
        {
            return VideoRecoveryState::Active;
        }
        let reason = status
            .result
            .failure_reason
            .clone()
            .or_else(|| (status.event == "failed").then(|| "capture_failed".into()));
        return VideoRecoveryState::Finished(reason);
    }
    let record = record
        .or_else(|| status.map(|status| record_from_status(&status)))
        .filter(|record| record.pid > 1);
    if let Some(record) = record {
        if helper_process_active(&record, probe(record.pid)) {
            VideoRecoveryState::Active
        } else {
            VideoRecoveryState::Finished(None)
        }
    } else if partial.with_extension("lease").exists() {
        recover_pending_launch(partial, stored_partial, binding.as_ref(), &mut probe)
    } else {
        VideoRecoveryState::Finished(None)
    }
}

fn recover_pending_launch(
    partial: &Path,
    stored_partial: &str,
    binding: Option<&CaptureBinding>,
    probe: &mut impl FnMut(u32) -> HelperProcessProbe,
) -> VideoRecoveryState {
    let Some(binding) = binding.filter(|binding| binding.partial_path == stored_partial) else {
        return VideoRecoveryState::Active;
    };
    let (Some(pid), Some(sec), Some(usec)) = (
        binding.owner_pid,
        binding.owner_start_sec,
        binding.owner_start_usec,
    ) else {
        // Older or unreadable owner identities cannot prove that launch is abandoned.
        return VideoRecoveryState::Active;
    };
    if pid <= 1 {
        return VideoRecoveryState::Active;
    }
    let ended = match probe(pid) {
        HelperProcessProbe::Gone => true,
        HelperProcessProbe::Unknown => false,
        HelperProcessProbe::Running {
            start_sec,
            start_usec,
            ..
        } => (sec, usec) != (start_sec, start_usec),
    };
    if !ended {
        return VideoRecoveryState::Active;
    }
    // Revoke before deciding capture is inactive: a queued LaunchServices launch must
    // observe this before creating output. Permission/identity uncertainty preserves it.
    match std::fs::remove_file(partial.with_extension("lease")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return VideoRecoveryState::Active,
    }
    // Registration can race the first read and owner probe. Even after revocation,
    // let a live/unknown helper settle before recovery can mutate its partial file.
    let record = read_process_record(&partial.with_extension("pid")).filter(|record| {
        record.pid > 1 && record.capture_nonce.as_deref() == Some(binding.capture_nonce.as_str())
    });
    let status = read_helper_status(&partial.with_extension("status.json")).filter(|status| {
        status.capture_nonce.as_deref() == Some(binding.capture_nonce.as_str())
            && record.as_ref().map_or(true, |record| {
                same_attempt_process(record, &record_from_status(status))
            })
    });
    if let Some(status) = status
        .as_ref()
        .filter(|status| status.event == "stopped" || status.event == "failed")
    {
        let terminal_record = record_from_status(status);
        if terminal_record.pid > 1
            && helper_process_active(&terminal_record, probe(terminal_record.pid))
        {
            return VideoRecoveryState::Active;
        }
        return VideoRecoveryState::Finished(
            status
                .result
                .failure_reason
                .clone()
                .or_else(|| (status.event == "failed").then(|| "capture_failed".into())),
        );
    }
    let record = record
        .or_else(|| status.map(|status| record_from_status(&status)))
        .filter(|record| record.pid > 1);
    if record.is_some_and(|record| helper_process_active(&record, probe(record.pid))) {
        VideoRecoveryState::Active
    } else {
        VideoRecoveryState::Finished(None)
    }
}

pub fn detect_meeting_window() -> Result<Option<MeetingWindowDto>, AppError> {
    let microphone = crate::meeting_detection::active_input_processes()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.bundle_id)
        .collect::<Vec<_>>();
    Ok(
        selection::choose(enumerate_windows()?, &microphone).map(|w| MeetingWindowDto {
            window_id: w.id,
            app_name: w.app,
            bundle_id: w.bundle,
            title: w.title,
        }),
    )
}
#[cfg(not(target_os = "macos"))]
fn enumerate_windows() -> Result<Vec<selection::Window>, AppError> {
    Ok(vec![])
}
#[cfg(target_os = "macos")]
fn enumerate_windows() -> Result<Vec<selection::Window>, AppError> {
    use block2::RcBlock;
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::NSString;
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
    }
    // Detection never prompts while video is still off. The existing screen permission settings own this grant.
    if !unsafe { CGPreflightScreenCaptureAccess() } {
        return Ok(Vec::new());
    }
    let Some(class) = AnyClass::get(c"SCShareableContent") else {
        return Ok(vec![]);
    };
    let (sender, receiver) = mpsc::sync_channel(1);
    let callback = RcBlock::new(move |content: *mut AnyObject, _error: *mut AnyObject| {
        let mut result = Vec::new();
        if !content.is_null() {
            unsafe {
                let windows: *mut AnyObject = msg_send![content, windows];
                let count: usize = msg_send![windows, count];
                for index in 0..count {
                    let window: *mut AnyObject = msg_send![windows,objectAtIndex:index];
                    let id: u32 = msg_send![window, windowID];
                    let layer: isize = msg_send![window, windowLayer];
                    if layer != 0 {
                        continue;
                    }
                    let app: *mut AnyObject = msg_send![window, owningApplication];
                    if app.is_null() {
                        continue;
                    }
                    let title: *mut NSString = msg_send![window, title];
                    let name: *mut NSString = msg_send![app, applicationName];
                    let bundle: *mut NSString = msg_send![app, bundleIdentifier];
                    result.push(selection::Window {
                        id,
                        app: name.as_ref().map(ToString::to_string).unwrap_or_default(),
                        bundle: bundle.as_ref().map(ToString::to_string).unwrap_or_default(),
                        title: title.as_ref().map(ToString::to_string).unwrap_or_default(),
                    });
                }
            }
        }
        let _ = sender.try_send(result);
    });
    unsafe {
        let _: () = msg_send![class,getShareableContentExcludingDesktopWindows:true,onScreenWindowsOnly:true,completionHandler:&*callback];
    }
    let mut windows = receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap_or_default();
    // CoreGraphics documents front-to-back order; SCShareableContent does not.
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(
            options: u32,
            relative: u32,
        ) -> core_foundation::base::CFTypeRef;
    }
    let mut ordered = Vec::new();
    unsafe {
        let info = CGWindowListCopyWindowInfo(1 | 16, 0) as *mut AnyObject;
        if !info.is_null() {
            let count: usize = msg_send![info, count];
            let key = NSString::from_str("kCGWindowNumber");
            for index in 0..count {
                let item: *mut AnyObject = msg_send![info, objectAtIndex:index];
                let number: *mut AnyObject = msg_send![item, objectForKey:&*key];
                if !number.is_null() {
                    let id: u32 = msg_send![number, unsignedIntValue];
                    ordered.push(id);
                }
            }
            let _: () = msg_send![info, release];
        }
    }
    windows.sort_by_key(|window| {
        ordered
            .iter()
            .position(|id| *id == window.id)
            .unwrap_or(usize::MAX)
    });
    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_startup_timeout_excludes_pause_before_first_frame() {
        let mut pauses = PauseTimeline::default();
        pauses.pause(2000);
        assert_eq!(
            active_startup_elapsed(Duration::from_secs(71), 1000, 72000, &pauses),
            Duration::from_secs(1)
        );
        pauses.resume(70000);
        assert_eq!(
            active_startup_elapsed(Duration::from_secs(71), 1000, 72000, &pauses),
            Duration::from_secs(3)
        );
        assert_eq!(
            active_startup_elapsed(Duration::from_secs(100), 1000, 101000, &pauses),
            Duration::from_secs(32)
        );
    }

    #[test]
    fn video_process_exit_rereads_terminal_status_and_duration() {
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("window.partial.mp4");
        let (_sender, receiver) = mpsc::channel();
        let result = run_helper_with_launcher(
            &partial,
            &temp.path().join("window.mp4"),
            1000,
            receiver,
            &AtomicBool::new(false),
            |_| {
                std::fs::write(
                    partial.with_extension("status.json"),
                    r#"{"event":"stopped","pid":2147483647,"startedAtHostMs":1250,"durationMs":5000}"#,
                )
                .unwrap();
            },
            |status, _| {
                std::fs::write(&partial, [1_u8; 64]).unwrap();
                std::fs::write(
                    status,
                    r#"{"event":"recording","pid":2147483647,"startedAtHostMs":1250,"durationMs":100}"#,
                )
                .unwrap();
                true
            },
        );
        assert!(result.failure_reason.is_none());
        assert_eq!(result.duration_ms, Some(5000));
        assert_eq!(result.offset_ms, 250);
    }

    #[tokio::test]
    async fn video_worker_recovers_controlled_deadline_exit_with_durable_fragments() {
        struct DeadlineProcess {
            ended: AtomicBool,
            failed: bool,
            status: PathBuf,
            nonce: String,
        }
        impl HelperProcessControl for DeadlineProcess {
            fn ownership(&self, record: &HelperProcessRecord) -> bool {
                record.capture_nonce.as_deref() == Some(self.nonce.as_str())
            }
            fn identity(&self, _: &HelperProcessRecord) -> HelperProcessMatch {
                if self.ended.load(Ordering::Acquire) {
                    HelperProcessMatch::GoneOrReused
                } else {
                    HelperProcessMatch::Matches
                }
            }
            fn send(&self, _: u32, signal: HelperSignal) {
                if signal == HelperSignal::Terminate {
                    if self.failed {
                        std::fs::write(&self.status, serde_json::json!({"event":"failed","pid":42,"processStartSec":10,"processStartUsec":20,"captureNonce":self.nonce,"startedAtHostMs":2250,"failureReason":"capture_failed"}).to_string()).unwrap();
                    }
                    self.ended.store(true, Ordering::Release);
                }
            }
        }
        for mode in [
            "recover",
            "failed",
            "missing_timing",
            "bad_timing",
            "corrupt_mp4",
            "unrequested",
        ] {
            let f = OwnerFixture::new().await;
            let original = if mode == "corrupt_mp4" {
                vec![1_u8; 64]
            } else {
                torn_container_fixture()
            };
            let process = DeadlineProcess {
                ended: AtomicBool::new(mode == "unrequested"),
                failed: mode == "failed",
                status: f.partial.with_extension("status.json"),
                nonce: "d0bee8db-9cf8-4fc3-94d9-e0c911e6db13".into(),
            };
            let (sender, receiver) = mpsc::channel();
            let result = run_helper_with_launcher_and_process(
                &f.partial,
                &f.final_path,
                1000,
                receiver,
                &AtomicBool::new(false),
                |_| {
                    if mode != "unrequested" {
                        sender.send(Action::Stop).unwrap();
                    }
                },
                |status, pid| {
                    std::fs::write(&f.partial, &original).unwrap();
                    f.seed_timing();
                    if mode == "missing_timing" {
                        std::fs::remove_file(f.partial.with_extension("timing.json")).unwrap();
                    }
                    if mode == "bad_timing" {
                        let timing = f.partial.with_extension("timing.json");
                        let mut value: serde_json::Value =
                            serde_json::from_slice(&std::fs::read(&timing).unwrap()).unwrap();
                        value["offsetMs"] = 0.into();
                        std::fs::write(timing, serde_json::to_vec(&value).unwrap()).unwrap();
                    }
                    let identity = serde_json::json!({"pid":42,"processStartSec":10,"processStartUsec":20,"captureNonce":process.nonce});
                    std::fs::write(pid, identity.to_string()).unwrap();
                    let mut value = identity;
                    value["event"] = "recording".into();
                    value["startedAtHostMs"] = 2250.into();
                    std::fs::write(status, value.to_string()).unwrap();
                    true
                },
                &process,
            );
            if mode == "recover" {
                assert!(
                    result.failure_reason.is_none(),
                    "verified controlled shutdown must salvage committed fragments"
                );
                assert_eq!(result.offset_ms, 1250);
                assert_eq!(result.title, "Meet - recovery");
                assert!(
                    std::fs::metadata(&f.final_path).unwrap().len() < original.len() as u64,
                    "repair must drop the incomplete tail"
                );
                assert!(mp4_recovery::recover(&f.final_path));
                f.repos
                    .finalize_video_artifact(&f.session_id, &result)
                    .await
                    .unwrap();
                let input = f
                    .repos
                    .note_video_input(&f.session_id, &f.paths)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(input.failure_reason.is_none() && !input.path.is_empty());
            } else {
                assert_eq!(
                    result.failure_reason.as_deref(),
                    Some("capture_failed"),
                    "{mode}"
                );
                assert_eq!(
                    std::fs::read(&f.final_path).unwrap(),
                    original,
                    "{mode} must never repair/promote failed or unverified input"
                );
                f.repos
                    .finalize_video_artifact(&f.session_id, &result)
                    .await
                    .unwrap();
                let input = f
                    .repos
                    .note_video_input(&f.session_id, &f.paths)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(input.path.is_empty());
                assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
            }
        }
    }

    fn torn_container_fixture() -> Vec<u8> {
        fn atom(tag: &[u8; 4], body: &[u8]) -> Vec<u8> {
            let mut bytes = ((body.len() + 8) as u32).to_be_bytes().to_vec();
            bytes.extend(tag);
            bytes.extend(body);
            bytes
        }
        let hdlr = atom(b"hdlr", &[&[0_u8; 8][..], b"vide"].concat());
        let trex = atom(
            b"trex",
            &[0_u32, 1, 1, 200, 0, 0]
                .into_iter()
                .flat_map(u32::to_be_bytes)
                .collect::<Vec<_>>(),
        );
        let movie = atom(
            b"moov",
            &[atom(b"mvex", &trex), atom(b"trak", &atom(b"mdia", &hdlr))].concat(),
        );
        let tfhd = atom(b"tfhd", &[0, 2, 0, 0x10, 0, 0, 0, 1, 0, 0, 0, 6]);
        let make_fragment = |offset: i32| {
            atom(
                b"moof",
                &atom(
                    b"traf",
                    &[
                        tfhd.clone(),
                        atom(
                            b"trun",
                            &[&[0, 0, 0, 1, 0, 0, 0, 1][..], &offset.to_be_bytes()].concat(),
                        ),
                    ]
                    .concat(),
                ),
            )
        };
        let fragment = make_fragment((make_fragment(0).len() + 8) as i32);
        [
            atom(b"ftyp", b"isom0000"),
            movie,
            fragment.clone(),
            atom(b"mdat", b"sample"),
            fragment,
        ]
        .concat()
    }

    #[tokio::test]
    async fn video_input_rejects_source_profile_and_wrong_session_without_touching_bytes() {
        for location in ["source-profile", "wrong-session"] {
            let f = OwnerFixture::new().await;
            let dir = if location == "source-profile" {
                f._temp
                    .path()
                    .join("source-profile/recordings/note/session")
            } else {
                f.paths
                    .recording_session_dir(&f.note_id, "another-session")
                    .unwrap()
            };
            std::fs::create_dir_all(&dir).unwrap();
            let outside = dir.join("window.mp4");
            let outside_partial = dir.join("window.partial.mp4");
            let original = torn_container_fixture();
            std::fs::write(&outside, &original).unwrap();
            std::fs::write(&outside_partial, &original).unwrap();
            assert!(mp4_recovery::recoverable(&outside));
            for state in ["recording", "valid"] {
                sqlx::query::query("UPDATE video_artifacts SET path=?,partial_path=?,status=? WHERE recording_session_id=?")
                    .bind(outside.to_string_lossy().as_ref()).bind(outside_partial.to_string_lossy().as_ref()).bind(state).bind(&f.session_id).execute(&f.repos.pool).await.unwrap();
                let input = f
                    .repos
                    .note_video_input(&f.session_id, &f.paths)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(input.path.is_empty());
                assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
                assert_eq!(std::fs::read(&outside).unwrap(), original);
                assert_eq!(std::fs::read(&outside_partial).unwrap(), original);
            }
        }
    }

    #[tokio::test]
    async fn video_recovery_requires_durable_timing_instead_of_default_zero() {
        let f = OwnerFixture::new().await;
        std::fs::write(&f.partial, torn_container_fixture()).unwrap();
        let input = f
            .repos
            .note_video_input(&f.session_id, &f.paths)
            .await
            .unwrap()
            .unwrap();
        assert!(
            input.path.is_empty(),
            "missing critical timing must not expose a zero-offset recovered video"
        );
        assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
    }
    #[tokio::test]
    async fn video_recovery_restores_paused_startup_timing_before_async_db_checkpoint() {
        let f = OwnerFixture::new().await;
        std::fs::write(&f.partial, torn_container_fixture()).unwrap();
        let nonce = "d0bee8db-9cf8-4fc3-94d9-e0c911e6db13";
        write_capture_binding(&f.partial, &f.session_id, nonce, 1000).unwrap();
        std::fs::write(
            f.partial.with_extension("pid"),
            serde_json::json!({"pid":2147483647,"captureNonce":nonce}).to_string(),
        )
        .unwrap();
        std::fs::write(f.partial.with_extension("timing.json"),serde_json::json!({"schema":1,"recordingSessionId":f.session_id,"partialPath":f.partial,"captureNonce":nonce,"audioStartedHostMs":1000,"startedAtHostMs":5000,"pausedBeforeFirstMs":1500,"offsetMs":2500,"app":"Chrome","title":"Meet - recovery","width":1920,"height":1080}).to_string()).unwrap();
        let input = f
            .repos
            .note_video_input(&f.session_id, &f.paths)
            .await
            .unwrap()
            .unwrap();
        assert!(input.failure_reason.is_none());
        assert_eq!(input.offset_ms, 2500);
        assert_eq!(input.app, "Chrome");
        let row = sqlx::query::query(
            "SELECT offset_ms FROM video_artifacts WHERE recording_session_id=?",
        )
        .bind(&f.session_id)
        .fetch_one(&f.repos.pool)
        .await
        .unwrap();
        use sqlx::row::Row;
        assert_eq!(row.get::<i64, _>("offset_ms"), 2500);
    }

    #[tokio::test]
    async fn video_recovery_rejects_unowned_or_invalid_timing_before_repair() {
        for key in [
            "schema",
            "captureNonce",
            "recordingSessionId",
            "partialPath",
            "offsetMs",
        ] {
            let f = OwnerFixture::new().await;
            let original = torn_container_fixture();
            std::fs::write(&f.partial, &original).unwrap();
            f.seed_timing();
            let checkpoint = f.partial.with_extension("timing.json");
            let mut value: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
            value[key] = match key {
                "schema" => 2.into(),
                "offsetMs" => 42.into(),
                _ => "wrong-owner".into(),
            };
            std::fs::write(checkpoint, serde_json::to_vec(&value).unwrap()).unwrap();
            let input = f
                .repos
                .note_video_input(&f.session_id, &f.paths)
                .await
                .unwrap()
                .unwrap();
            assert!(input.path.is_empty(), "{key}");
            assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
            assert_eq!(
                std::fs::read(&f.partial).unwrap(),
                original,
                "{key} must reject before truncation"
            );
            assert!(!f.final_path.exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn video_metadata_reads_reject_symlinks_and_nonregular_files_without_waiting() {
        use std::os::unix::{ffi::OsStrExt, fs::symlink};
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"{}").unwrap();
        let linked = dir.path().join("linked");
        symlink(&target, &linked).unwrap();
        assert!(read_owned_sidecar(&linked).is_none());
        let fifo = dir.path().join("timing.json");
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(read_owned_sidecar(&fifo).is_none());
        let oversized = dir.path().join("oversized");
        std::fs::write(&oversized, vec![b'0'; 16_385]).unwrap();
        assert!(read_owned_sidecar(&oversized).is_none());
    }

    struct TestProcessControl {
        expected_nonce: Option<&'static str>,
        identity: HelperProcessMatch,
        signals: std::sync::Mutex<Vec<(u32, HelperSignal)>>,
    }
    impl HelperProcessControl for TestProcessControl {
        fn ownership(&self, record: &HelperProcessRecord) -> bool {
            self.expected_nonce.map_or(true, |expected| {
                record.capture_nonce.as_deref() == Some(expected)
            })
        }
        fn identity(&self, _: &HelperProcessRecord) -> HelperProcessMatch {
            self.identity
        }
        fn send(&self, pid: u32, number: HelperSignal) {
            self.signals.lock().unwrap().push((pid, number));
        }
    }
    #[test]
    fn video_worker_rejects_stale_attempt_before_ready_checkpoint_or_signals() {
        for event in ["starting", "recording", "stopped"] {
            let temp = tempfile::tempdir().unwrap();
            let partial = temp.path().join("window.partial.mp4");
            let (sender, receiver) = mpsc::channel();
            sender.send(Action::Pause(host_ms())).unwrap();
            let control = TestProcessControl {
                expected_nonce: Some("d0bee8db-9cf8-4fc3-94d9-e0c911e6db13"),
                identity: HelperProcessMatch::Matches,
                signals: Default::default(),
            };
            let checkpointed = AtomicBool::new(false);
            let result = run_helper_with_launcher_and_process(
                &partial,
                &temp.path().join("window.mp4"),
                host_ms(),
                receiver,
                &AtomicBool::new(false),
                |_| {
                    checkpointed.store(true, Ordering::Release);
                },
                |status, pid| {
                    let stale = "eed34621-6d0a-4d34-b1d7-9bb1a2f617fa";
                    std::fs::write(
                        pid,
                        serde_json::json!({"pid":42,"captureNonce":stale}).to_string(),
                    )
                    .unwrap();
                    std::fs::write(status,serde_json::json!({"event":event,"pid":42,"captureNonce":stale,"startedAtHostMs":5000}).to_string()).unwrap();
                    true
                },
                &control,
            );
            assert_eq!(result.failure_reason.as_deref(), Some("capture_failed"));
            assert!(!checkpointed.load(Ordering::Acquire));
            assert!(control.signals.lock().unwrap().is_empty());
        }
    }
    #[tokio::test]
    async fn video_recovery_rejects_sidecars_that_agree_on_a_different_attempt() {
        let f = OwnerFixture::new().await;
        let original = torn_container_fixture();
        std::fs::write(&f.partial, &original).unwrap();
        f.seed_timing();
        let stale = "eed34621-6d0a-4d34-b1d7-9bb1a2f617fa";
        std::fs::write(
            f.partial.with_extension("pid"),
            serde_json::json!({"pid":2147483647,"captureNonce":stale}).to_string(),
        )
        .unwrap();
        let checkpoint = f.partial.with_extension("timing.json");
        let mut timing: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&checkpoint).unwrap()).unwrap();
        timing["captureNonce"] = stale.into();
        std::fs::write(checkpoint, serde_json::to_vec(&timing).unwrap()).unwrap();
        let input = f
            .repos
            .note_video_input(&f.session_id, &f.paths)
            .await
            .unwrap()
            .unwrap();
        assert!(input.path.is_empty());
        assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
        assert_eq!(std::fs::read(&f.partial).unwrap(), original);
    }

    #[test]
    fn video_signals_require_strict_identity_and_never_target_unknown_or_reused_pid() {
        let record = HelperProcessRecord {
            pid: 42,
            process_start_sec: Some(10),
            process_start_usec: Some(20),
            capture_nonce: None,
        };
        for identity in [
            HelperProcessMatch::Matches,
            HelperProcessMatch::GoneOrReused,
            HelperProcessMatch::Unknown,
        ] {
            let control = TestProcessControl {
                expected_nonce: None,
                identity,
                signals: Default::default(),
            };
            signal_verified(&record, HelperSignal::Terminate, &control);
            signal_verified(&record, HelperSignal::Kill, &control);
            assert_eq!(
                control.signals.lock().unwrap().len(),
                if identity == HelperProcessMatch::Matches {
                    2
                } else {
                    0
                }
            );
        }
    }
    #[test]
    fn video_paused_startup_detects_dead_helper_before_first_frame() {
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("window.partial.mp4");
        let (sender, receiver) = mpsc::channel();
        sender.send(Action::Pause(host_ms())).unwrap();
        let control = TestProcessControl {
            expected_nonce: None,
            identity: HelperProcessMatch::GoneOrReused,
            signals: Default::default(),
        };
        let result = run_helper_with_launcher_and_process(
            &partial,
            &temp.path().join("window.mp4"),
            host_ms(),
            receiver,
            &AtomicBool::new(false),
            |_| {},
            |status, pid| {
                std::fs::write(
                    pid,
                    r#"{"pid":42,"processStartSec":10,"processStartUsec":20}"#,
                )
                .unwrap();
                std::fs::write(
                    status,
                    r#"{"event":"starting","pid":42,"processStartSec":10,"processStartUsec":20}"#,
                )
                .unwrap();
                true
            },
            &control,
        );
        assert_eq!(result.failure_reason.as_deref(), Some("capture_failed"));
        assert!(control.signals.lock().unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn video_control_pipe_replays_startup_records_and_fails_without_waiting_when_full() {
        use std::{io::Read, os::unix::fs::OpenOptionsExt};
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("window.partial.mp4");
        let failed = Arc::new(AtomicBool::new(false));
        let controls = DirectControlPipe::create(&partial, failed.clone());
        controls.publish(true, 1000);
        controls.publish(false, 2000);
        // No helper/worker existed when the records were published.
        let mut reader = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW)
            .open(partial.with_extension("control.pipe"))
            .unwrap();
        let mut records = [0_u8; 48];
        reader.read_exact(&mut records).unwrap();
        assert_eq!(u64::from_le_bytes(records[..8].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(records[24..32].try_into().unwrap()), 2);
        assert_eq!(u64::from_le_bytes(records[16..24].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(records[40..48].try_into().unwrap()), 0);
        for _ in 0..100_000 {
            controls.publish(true, 3000);
            if failed.load(Ordering::Acquire) {
                break;
            }
        }
        assert!(
            failed.load(Ordering::Acquire),
            "full channel must fail video instead of waiting for a reader"
        );
        let mut queued = Vec::new();
        let error = reader.read_to_end(&mut queued).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert_eq!(queued.len() % 24, 0, "every queued record is complete");
    }

    fn pending_owner_binding(partial: &Path) {
        std::fs::write(
            partial.with_extension("capture.json"),
            serde_json::json!({
                "schema":1,"recordingSessionId":"session","partialPath":partial,
                "audioStartedHostMs":1000,"captureNonce":"d0bee8db-9cf8-4fc3-94d9-e0c911e6db13",
                "ownerPid":77,"ownerStartSec":10,"ownerStartUsec":20
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(partial.with_extension("lease"), b"active").unwrap();
    }
    #[test]
    fn video_recovery_revokes_dead_owner_pending_launch() {
        for observation in [
            HelperProcessProbe::Gone,
            HelperProcessProbe::Running {
                start_sec: 11,
                start_usec: 20,
                executable: "/unrelated/reused-owner".into(),
            },
        ] {
            let dir = tempfile::tempdir().unwrap();
            let partial = dir.path().join("window.partial.mp4");
            pending_owner_binding(&partial);
            assert!(matches!(
                video_recovery_state_with_probe(&partial, |_| observation.clone()),
                VideoRecoveryState::Finished(None)
            ));
            assert!(
                !partial.with_extension("lease").exists(),
                "revoke before a delayed helper can open capture output"
            );
            if partial.with_extension("lease").exists() {
                std::fs::write(&partial, b"late helper video").unwrap();
            }
            assert!(!partial.exists());
        }
    }
    #[test]
    fn video_recovery_protects_live_or_unknown_pending_owner() {
        for observation in [
            HelperProcessProbe::Unknown,
            HelperProcessProbe::Running {
                start_sec: 10,
                start_usec: 20,
                executable: "/owned/clovy".into(),
            },
        ] {
            let dir = tempfile::tempdir().unwrap();
            let partial = dir.path().join("window.partial.mp4");
            pending_owner_binding(&partial);
            assert!(matches!(
                video_recovery_state_with_probe(&partial, |_| observation.clone()),
                VideoRecoveryState::Active
            ));
            assert!(partial.with_extension("lease").exists());
        }
    }
    #[test]
    fn video_recovery_rechecks_helper_registration_after_revoking_dead_owner() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("window.partial.mp4");
        pending_owner_binding(&partial);
        let state = video_recovery_state_with_probe(&partial, |pid| {
            if pid == 77 {
                std::fs::write(partial.with_extension("pid"), serde_json::json!({"pid":42,"processStartSec":30,"processStartUsec":40,"captureNonce":"d0bee8db-9cf8-4fc3-94d9-e0c911e6db13"}).to_string()).unwrap();
                HelperProcessProbe::Gone
            } else {
                assert_eq!(pid, 42);
                HelperProcessProbe::Running {
                    start_sec: 30,
                    start_usec: 40,
                    executable: "/owned/clovy-video-capture".into(),
                }
            }
        });
        assert!(
            matches!(state, VideoRecoveryState::Active),
            "do not repair while a concurrently registered helper can still be settling"
        );
        assert!(!partial.with_extension("lease").exists());
    }
    #[test]
    fn video_recovery_preserves_terminal_failure_published_during_owner_probe() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("window.partial.mp4");
        pending_owner_binding(&partial);
        let state = video_recovery_state_with_probe(&partial, |pid| {
            if pid == 77 {
                std::fs::write(partial.with_extension("status.json"), serde_json::json!({"event":"failed","pid":42,"processStartSec":30,"processStartUsec":40,"captureNonce":"d0bee8db-9cf8-4fc3-94d9-e0c911e6db13","failureReason":"capture_failed"}).to_string()).unwrap();
            }
            HelperProcessProbe::Gone
        });
        assert!(
            matches!(state, VideoRecoveryState::Finished(Some(reason)) if reason == "capture_failed")
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn video_capture_binding_durably_records_original_host_birth_before_launch() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("window.partial.mp4");
        write_capture_binding(
            &partial,
            "session",
            "d0bee8db-9cf8-4fc3-94d9-e0c911e6db13",
            1000,
        )
        .unwrap();
        let binding = read_capture_binding(&partial).unwrap();
        assert_eq!(binding.owner_pid, Some(std::process::id()));
        let HelperProcessProbe::Running {
            start_sec,
            start_usec,
            ..
        } = probe_helper_process(std::process::id())
        else {
            panic!("current host birth unavailable");
        };
        assert_eq!(
            (binding.owner_start_sec, binding.owner_start_usec),
            (Some(start_sec), Some(start_usec))
        );
        assert!(!partial.with_extension("lease").exists());
        assert!(!partial.with_extension("capture.tmp").exists());
    }
    #[tokio::test]
    async fn video_recovery_dead_pending_owner_persists_audio_only_fallback() {
        let f = OwnerFixture::new().await;
        pending_owner_binding(&f.partial);
        let path = f.partial.with_extension("capture.json");
        let mut binding: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        binding["ownerPid"] = serde_json::json!(i32::MAX);
        std::fs::write(path, serde_json::to_vec(&binding).unwrap()).unwrap();
        let input = f
            .repos
            .note_video_input(&f.session_id, &f.paths)
            .await
            .unwrap()
            .unwrap();
        assert!(input.path.is_empty());
        assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
        assert!(!f.partial.with_extension("lease").exists());
        let row =
            sqlx::query::query("SELECT status FROM video_artifacts WHERE recording_session_id=?")
                .bind(&f.session_id)
                .fetch_one(&f.repos.pool)
                .await
                .unwrap();
        assert_eq!(sqlx::row::Row::get::<String, _>(&row, "status"), "failed");
    }
    #[test]
    fn video_recovery_ignores_stale_terminal_status_while_current_helper_is_live() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("window.partial.mp4");
        let nonce = "d0bee8db-9cf8-4fc3-94d9-e0c911e6db13";
        std::fs::write(partial.with_extension("capture.json"),serde_json::json!({"schema":1,"recordingSessionId":"session","partialPath":partial,"audioStartedHostMs":1000,"captureNonce":nonce}).to_string()).unwrap();
        std::fs::write(partial.with_extension("pid"),serde_json::json!({"pid":42,"processStartSec":10,"processStartUsec":20,"captureNonce":nonce}).to_string()).unwrap();
        std::fs::write(partial.with_extension("status.json"),r#"{"event":"stopped","pid":42,"processStartSec":9,"processStartUsec":0,"captureNonce":"eed34621-6d0a-4d34-b1d7-9bb1a2f617fa"}"#).unwrap();
        assert!(matches!(
            video_recovery_state_with_probe(&partial, |_| HelperProcessProbe::Running {
                start_sec: 10,
                start_usec: 20,
                executable: PathBuf::from("/owned/clovy-video-capture")
            }),
            VideoRecoveryState::Active
        ));
    }

    #[test]
    fn video_recovery_binds_pid_to_start_time_and_helper_executable() {
        let temp = tempfile::tempdir().unwrap();
        let partial = temp.path().join("window.partial.mp4");
        let pid_path = partial.with_extension("pid");
        std::fs::write(partial.with_extension("lease"), b"active").unwrap();
        std::fs::write(
            &pid_path,
            r#"{"pid":42,"processStartSec":10,"processStartUsec":20}"#,
        )
        .unwrap();
        let helper = |sec, usec, name: &str| HelperProcessProbe::Running {
            start_sec: sec,
            start_usec: usec,
            executable: PathBuf::from(name),
        };
        for (probe, expected_active) in [
            (
                helper(
                    10,
                    20,
                    "/owned/Clovy Video Capture.app/Contents/MacOS/clovy-video-capture",
                ),
                true,
            ),
            (
                helper(
                    10,
                    21,
                    "/owned/Clovy Video Capture.app/Contents/MacOS/clovy-video-capture",
                ),
                false,
            ),
            (
                helper(
                    11,
                    20,
                    "/owned/Clovy Video Capture.app/Contents/MacOS/clovy-video-capture",
                ),
                false,
            ),
            (helper(10, 20, "/bin/sleep"), false),
            (HelperProcessProbe::Unknown, true),
            (HelperProcessProbe::Gone, false),
        ] {
            assert_eq!(
                matches!(
                    video_recovery_state_with_probe(&partial, |_| probe.clone()),
                    VideoRecoveryState::Active
                ),
                expected_active
            );
        }
        // Legacy records cannot verify start time but must still verify executable identity.
        std::fs::write(&pid_path, b"42").unwrap();
        assert!(matches!(
            video_recovery_state_with_probe(&partial, |_| helper(
                10,
                20,
                "/owned/clovy-video-capture"
            )),
            VideoRecoveryState::Active
        ));
        assert!(matches!(
            video_recovery_state_with_probe(&partial, |_| helper(10, 20, "/bin/sleep")),
            VideoRecoveryState::Finished(_)
        ));
        // A missing PID sidecar must not discard the identity mirrored in nonterminal status.
        std::fs::remove_file(&pid_path).unwrap();
        std::fs::write(
            partial.with_extension("status.json"),
            r#"{"event":"recording","pid":42,"processStartSec":10,"processStartUsec":20}"#,
        )
        .unwrap();
        assert!(matches!(
            video_recovery_state_with_probe(&partial, |_| helper(
                11,
                20,
                "/owned/clovy-video-capture"
            )),
            VideoRecoveryState::Finished(_)
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn video_process_probe_reads_kernel_start_time_and_executable_without_spawning() {
        assert!(matches!(
            probe_helper_process(i32::MAX as u32),
            HelperProcessProbe::Gone
        ));
        let HelperProcessProbe::Running {
            start_sec,
            executable,
            ..
        } = probe_helper_process(std::process::id())
        else {
            panic!("own process must be observable");
        };
        assert!(start_sec > 0);
        assert!(executable.file_name().is_some());
        assert!(!helper_process_active(
            &HelperProcessRecord {
                pid: std::process::id(),
                process_start_sec: None,
                process_start_usec: None,
                capture_nonce: None,
            },
            probe_helper_process(std::process::id())
        ));
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn video_input_recovers_when_pid_was_reused_by_unrelated_process() {
        let f = OwnerFixture::new().await;
        std::fs::write(&f.partial, torn_container_fixture()).unwrap();
        std::fs::write(f.partial.with_extension("lease"), b"active").unwrap();
        let nonce = f.seed_timing();
        // Model a crashed app owner, independently of the reused helper PID.
        let mut binding = read_capture_binding(&f.partial).unwrap();
        binding.owner_pid = Some(i32::MAX as u32);
        std::fs::write(
            f.partial.with_extension("capture.json"),
            serde_json::to_vec(&binding).unwrap(),
        )
        .unwrap();
        // This test process is alive, but it is not the video capture helper.
        std::fs::write(
            f.partial.with_extension("pid"),
            serde_json::json!({"pid":std::process::id(),"captureNonce":nonce}).to_string(),
        )
        .unwrap();
        let input = f
            .repos
            .note_video_input(&f.session_id, &f.paths)
            .await
            .unwrap()
            .unwrap();
        assert!(
            !input.path.is_empty(),
            "unrelated reused PID must allow crash recovery"
        );
        assert!(input.failure_reason.is_none());
        assert!(f.final_path.exists());
    }

    #[tokio::test]
    async fn video_input_never_repairs_pending_helper_on_generation_timeout() {
        {
            let f = OwnerFixture::new().await;
            let original = torn_container_fixture();
            std::fs::write(&f.partial, &original).unwrap();
            std::fs::write(f.partial.with_extension("lease"), b"active").unwrap();
            let input = f
                .repos
                .note_video_input(&f.session_id, &f.paths)
                .await
                .unwrap()
                .unwrap();
            assert!(input.path.is_empty());
            assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
            assert_eq!(std::fs::read(&f.partial).unwrap(), original);
            assert!(!f.final_path.exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn video_input_rejects_symlink_before_repair_or_analysis() {
        let f = OwnerFixture::new().await;
        let outside = f._temp.path().join("source-video.mp4");
        let original = torn_container_fixture();
        std::fs::write(&outside, &original).unwrap();
        std::os::unix::fs::symlink(&outside, &f.final_path).unwrap();
        for state in ["recording", "valid"] {
            sqlx::query::query("UPDATE video_artifacts SET status=? WHERE recording_session_id=?")
                .bind(state)
                .bind(&f.session_id)
                .execute(&f.repos.pool)
                .await
                .unwrap();
            let input = f
                .repos
                .note_video_input(&f.session_id, &f.paths)
                .await
                .unwrap()
                .unwrap();
            assert!(input.path.is_empty());
            assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
            assert_eq!(std::fs::read(&outside).unwrap(), original);
        }
    }

    #[test]
    fn video_control_ledger_keeps_all_pauses_when_notifications_coalesce() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("control.json");
        let mut ledger = ControlLedger::default();
        ledger.push(true, 10000);
        ledger.publish(&path).unwrap();
        ledger.push(false, 15000);
        ledger.publish(&path).unwrap();
        let events: Vec<VideoControlEvent> =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].sequence, 1);
        assert!(events[0].paused);
        assert_eq!(events[1].sequence, 2);
        assert!(!events[1].paused);
        assert_eq!(events[1].at_host_ms - events[0].at_host_ms, 5000);
    }

    #[tokio::test]
    async fn video_atomic_note_batch_profile_delete_includes_late_registered_artifacts() {
        for deletion in ["note", "batch", "profile"] {
            let f = OwnerFixture::new().await;
            let audio_files = f.seed_audio_files().await;
            sqlx::query::query("DELETE FROM video_artifacts WHERE recording_session_id=?")
                .bind(&f.session_id)
                .execute(&f.repos.pool)
                .await
                .unwrap();
            let stale = if deletion == "profile" {
                f.repos
                    .audio_artifact_paths_for_profile("video-owner-test")
                    .await
                    .unwrap()
            } else {
                f.repos
                    .audio_artifact_paths_for_note(&f.note_id)
                    .await
                    .unwrap()
            };
            assert!(!stale
                .iter()
                .any(|path| path == &f.final_path.to_string_lossy()));
            // This artifact is registered and completed after the old out-of-transaction snapshot.
            f.repos
                .create_video_artifact(
                    &f.note_id,
                    &f.session_id,
                    &f.partial.to_string_lossy(),
                    &f.final_path.to_string_lossy(),
                )
                .await
                .unwrap();
            for file in f.owned_files() {
                std::fs::write(file, b"late owned video").unwrap();
            }
            assert!(f
                .repos
                .finalize_video_artifact(&f.session_id, &VideoResult::default())
                .await
                .unwrap());
            let files = match deletion {
                "note" => f
                    .repos
                    .delete_note_with_artifact_paths(&f.note_id)
                    .await
                    .unwrap(),
                "batch" => f
                    .repos
                    .delete_notes_with_artifact_paths(std::slice::from_ref(&f.note_id))
                    .await
                    .unwrap(),
                _ => f
                    .repos
                    .delete_profile_data_with_artifact_paths("video-owner-test")
                    .await
                    .unwrap(),
            };
            assert!(files.first().unwrap().ends_with(".lease"));
            assert!(files
                .iter()
                .any(|path| path == &f.final_path.to_string_lossy()));
            for file in files {
                let _ = f.paths.remove_recording_file(file);
            }
            assert!(f.owned_files().iter().all(|path| !path.exists()));
            assert!(!f.repos.video_artifact_exists(&f.session_id).await.unwrap());
            assert!(audio_files.iter().all(|path| !path.exists()));
        }
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "current_thread")]
    async fn video_recovery_yields_while_repair_is_blocked_on_io() {
        use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};

        let f = OwnerFixture::new().await;
        f.seed_timing();
        std::fs::write(&f.partial, torn_container_fixture()).unwrap();
        let fifo = std::ffi::CString::new(f.final_path.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        let (release, waiting) = mpsc::channel();
        let final_path = f.final_path.clone();
        let writer = thread::spawn(move || {
            let timed_out = waiting.recv_timeout(Duration::from_secs(2)).is_err();
            let deadline = Instant::now() + Duration::from_secs(2);
            let _writer = loop {
                match std::fs::OpenOptions::new()
                    .write(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&final_path)
                {
                    Ok(writer) => break writer,
                    Err(error) if error.raw_os_error() == Some(libc::ENXIO) => {
                        assert!(Instant::now() < deadline, "recovery never opened the FIFO");
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("could not release repair: {error}"),
                }
            };
            timed_out
        });
        let heartbeat = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            release.send(()).unwrap();
        };
        let (input, ()) =
            tokio::join!(f.repos.note_video_input(&f.session_id, &f.paths), heartbeat);
        assert!(!writer.join().unwrap(), "repair blocked the Tokio worker");
        let input = input.unwrap().unwrap();
        assert!(input.failure_reason.is_none());
        assert_eq!(input.offset_ms, 1250);
        assert!(mp4_recovery::recoverable(&f.final_path));
        assert!(!f.partial.exists());
    }

    struct OwnerFixture {
        _temp: tempfile::TempDir,
        paths: crate::app_paths::AppPaths,
        repos: Repositories,
        note_id: String,
        session_id: String,
        partial: PathBuf,
        final_path: PathBuf,
    }
    impl OwnerFixture {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let paths =
                crate::app_paths::AppPaths::from_data_dir(temp.path().join("data")).unwrap();
            let pool = sqlx_sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap();
            crate::db::migrations::run_migrations(&pool).await.unwrap();
            let repos = Repositories::new(pool);
            let note = repos.create_note("video-owner-test", None).await.unwrap();
            let session_id = uuid::Uuid::new_v4().to_string();
            let dir = paths.recording_session_dir(&note.id, &session_id).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let partial = dir.join("window.partial.mp4");
            let final_path = dir.join("window.mp4");
            repos
                .create_recording_session(
                    &note.id,
                    &session_id,
                    crate::domain::types::RecordingSourceMode::MicrophoneOnly,
                    &dir.join("microphone.partial.wav").to_string_lossy(),
                    &dir.join("microphone.wav").to_string_lossy(),
                    None,
                )
                .await
                .unwrap();
            repos
                .create_video_artifact(
                    &note.id,
                    &session_id,
                    &partial.to_string_lossy(),
                    &final_path.to_string_lossy(),
                )
                .await
                .unwrap();
            Self {
                _temp: temp,
                paths,
                repos,
                note_id: note.id,
                session_id,
                partial,
                final_path,
            }
        }
        fn seed_timing(&self) -> &'static str {
            let nonce = "d0bee8db-9cf8-4fc3-94d9-e0c911e6db13";
            write_capture_binding(&self.partial, &self.session_id, nonce, 1000).unwrap();
            std::fs::write(
                self.partial.with_extension("pid"),
                serde_json::json!({"pid":2147483647,"captureNonce":nonce}).to_string(),
            )
            .unwrap();
            std::fs::write(self.partial.with_extension("timing.json"),serde_json::json!({"schema":1,"recordingSessionId":self.session_id,"partialPath":self.partial,"captureNonce":nonce,"audioStartedHostMs":1000,"startedAtHostMs":2250,"pausedBeforeFirstMs":0,"offsetMs":1250,"app":"Chrome","title":"Meet - recovery","width":64,"height":64}).to_string()).unwrap();
            nonce
        }
        async fn seed_audio_files(&self) -> Vec<PathBuf> {
            let dir = self.partial.parent().unwrap();
            let files = [
                "microphone.partial.wav",
                "microphone.wav",
                "source.partial.wav",
                "source.wav",
            ]
            .map(|name| dir.join(name))
            .to_vec();
            self.repos
                .create_pending_source_artifact(
                    &self.note_id,
                    &self.session_id,
                    "microphone",
                    &files[2].to_string_lossy(),
                    &files[3].to_string_lossy(),
                )
                .await
                .unwrap();
            for file in &files {
                std::fs::write(file, b"owned audio").unwrap();
            }
            files
        }
        fn owned_files(&self) -> Vec<PathBuf> {
            vec![
                self.partial.clone(),
                self.final_path.clone(),
                self.partial.with_extension("lease"),
                self.partial.with_extension("status.json"),
                self.partial.with_extension("pid"),
                self.partial.with_extension("control.json"),
                self.partial.with_extension("control.pipe"),
                self.partial.with_extension("control.tmp"),
                self.partial.with_extension("timing.json"),
                self.partial.with_extension("capture.json"),
                self.partial.with_extension("capture.tmp"),
            ]
        }
    }

    #[tokio::test]
    async fn video_owner_monitor_stops_note_batch_profile_and_folder_deletions() {
        for deletion in ["note", "batch", "profile", "folder"] {
            let f = OwnerFixture::new().await;
            let lease = f.partial.with_extension("lease");
            std::fs::write(&lease, b"active").unwrap();
            let (sender, receiver) = mpsc::channel();
            let (_done_sender, done) = tokio::sync::oneshot::channel();
            let cancelled = Arc::new(AtomicBool::new(false));
            let monitor = tokio::spawn(monitor_video_owner(
                f.repos.clone(),
                f.session_id.clone(),
                sender,
                done,
                cancelled.clone(),
                lease.clone(),
            ));
            match deletion {
                "note" => f.repos.delete_note(&f.note_id).await.unwrap(),
                "batch" => f
                    .repos
                    .delete_notes(std::slice::from_ref(&f.note_id))
                    .await
                    .unwrap(),
                "profile" => f
                    .repos
                    .delete_profile_data("video-owner-test")
                    .await
                    .unwrap(),
                _ => {
                    let folder = f
                        .repos
                        .create_folder("video-owner-test", "Project", None)
                        .await
                        .unwrap();
                    f.repos
                        .assign_note_to_folder("video-owner-test", &f.note_id, &folder.id)
                        .await
                        .unwrap();
                    f.repos
                        .delete_folder_with_artifact_paths(&folder.id, true)
                        .await
                        .unwrap();
                }
            }
            assert!(matches!(
                tokio::task::spawn_blocking(move || receiver.recv_timeout(Duration::from_secs(3)))
                    .await
                    .unwrap()
                    .unwrap(),
                Action::Stop
            ));
            monitor.await.unwrap();
            assert!(cancelled.load(Ordering::Acquire));
            assert!(!lease.exists());
        }
    }

    #[tokio::test]
    async fn video_monitor_database_errors_preserve_files_and_control_delivery() {
        let f = OwnerFixture::new().await;
        std::fs::write(&f.final_path, b"keep video").unwrap();
        f.repos.pool.close().await;
        let (sender, receiver) = mpsc::channel();
        let (done_sender, done) = tokio::sync::oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let monitor = tokio::spawn(monitor_video_owner(
            f.repos.clone(),
            f.session_id.clone(),
            sender.clone(),
            done,
            cancelled.clone(),
            f.partial.with_extension("lease"),
        ));
        sender.send(Action::Pause(1000)).unwrap();
        sender.send(Action::Resume(2000)).unwrap();
        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(100)).unwrap(),
            Action::Pause(1000)
        ));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(100)).unwrap(),
            Action::Resume(2000)
        ));
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(!cancelled.load(Ordering::Acquire));
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        finalize_video_worker(
            &f.repos,
            &f.session_id,
            &f.partial,
            &f.final_path,
            &VideoResult::default(),
            &cancelled,
        )
        .await;
        assert!(f.final_path.exists());
        let _ = done_sender.send(());
        monitor.await.unwrap();
    }

    #[tokio::test]
    async fn video_discard_revalidates_authority_before_revoking_ownership() {
        let f = OwnerFixture::new().await;
        for file in f.owned_files() {
            std::fs::write(file, b"owned fixture").unwrap();
        }
        let info = f
            .repos
            .recording_recovery_info(&f.session_id)
            .await
            .unwrap()
            .unwrap();
        let denied = async { Err(AppError::new("turn_expired", "Stopped")) };
        let error =
            crate::commands::discard_recoverable_recording(&f.repos, &f.paths, &info, denied)
                .await
                .unwrap_err();
        assert_eq!(error.code, "turn_expired");
        assert!(f.owned_files().iter().all(|file| file.exists()));
        assert!(f.repos.video_artifact_exists(&f.session_id).await.unwrap());
    }

    #[tokio::test]
    async fn video_discard_transaction_failure_preserves_files_and_ownership() {
        for trigger in [
            "CREATE TRIGGER reject_discard BEFORE DELETE ON video_artifacts BEGIN SELECT RAISE(ABORT, 'injected discard failure'); END",
            "CREATE TRIGGER reject_discard BEFORE UPDATE ON notes BEGIN SELECT RAISE(ABORT, 'injected discard failure'); END",
        ] {
            let f = OwnerFixture::new().await;
            for file in f.owned_files() { std::fs::write(file, b"owned fixture").unwrap(); }
            let original: String = sqlx::query_scalar::query_scalar("SELECT status FROM recording_sessions WHERE id=?")
                .bind(&f.session_id).fetch_one(&f.repos.pool).await.unwrap();
            let info = f.repos.recording_recovery_info(&f.session_id).await.unwrap().unwrap();
            sqlx::query::query(trigger).execute(&f.repos.pool).await.unwrap();
            assert!(crate::commands::discard_recoverable_recording(&f.repos, &f.paths, &info, std::future::ready(Ok(()))).await.is_err());
            assert!(f.owned_files().iter().all(|file| file.exists()));
            assert!(f.repos.video_artifact_exists(&f.session_id).await.unwrap());
            let after: String = sqlx::query_scalar::query_scalar("SELECT status FROM recording_sessions WHERE id=?")
                .bind(&f.session_id).fetch_one(&f.repos.pool).await.unwrap();
            assert_eq!(after, original, "every earlier write must roll back");
            sqlx::query::query("DROP TRIGGER reject_discard").execute(&f.repos.pool).await.unwrap();
            crate::commands::discard_recoverable_recording(&f.repos, &f.paths, &info, std::future::ready(Ok(()))).await.unwrap();
            assert!(f.owned_files().iter().all(|file| !file.exists()));
            assert!(!f.repos.video_artifact_exists(&f.session_id).await.unwrap());
        }
    }

    #[tokio::test]
    async fn video_deleted_between_unlinks_and_rename_cannot_resurrect() {
        for discard in [false, true] {
            let f = OwnerFixture::new().await;
            let (_sender, receiver) = mpsc::channel();
            std::fs::write(&f.partial, [1_u8; 64]).unwrap();
            std::fs::write(
                f.partial.with_extension("status.json"),
                r#"{"event":"stopped","pid":0,"startedAtHostMs":1000}"#,
            )
            .unwrap();
            let files = f
                .repos
                .audio_artifact_paths_for_note(&f.note_id)
                .await
                .unwrap();
            if discard {
                f.repos
                    .mark_recording_discarded(&f.session_id, &f.note_id)
                    .await
                    .unwrap();
            } else {
                f.repos.delete_note(&f.note_id).await.unwrap();
            }
            // Deletion visited final before it existed, then the worker wins the rename race.
            let _ = f.paths.remove_recording_file(&f.final_path);
            let result = run_helper_with_launcher(
                &f.partial,
                &f.final_path,
                0,
                receiver,
                &AtomicBool::new(false),
                |_| {},
                |status, _| {
                    std::fs::write(
                        status,
                        r#"{"event":"stopped","pid":0,"startedAtHostMs":1000}"#,
                    )
                    .unwrap();
                    true
                },
            );
            assert!(f.final_path.exists());
            let _ = f.paths.remove_recording_file(&f.partial);
            finalize_video_worker(
                &f.repos,
                &f.session_id,
                &f.partial,
                &f.final_path,
                &result,
                &AtomicBool::new(false),
            )
            .await;
            assert!(f.owned_files().iter().all(|path| !path.exists()));
            assert!(files.iter().any(|path| path.ends_with(".lease")));
        }
    }

    #[tokio::test]
    async fn video_delayed_helper_after_deletion_observes_revoked_lease() {
        let f = OwnerFixture::new().await;
        let (sender, receiver) = mpsc::channel();
        let (_done_sender, done) = tokio::sync::oneshot::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let monitor = tokio::spawn(monitor_video_owner(
            f.repos.clone(),
            f.session_id.clone(),
            sender,
            done,
            cancelled.clone(),
            f.partial.with_extension("lease"),
        ));
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (open_tx, open_rx) = mpsc::channel();
        let partial = f.partial.clone();
        let final_path = f.final_path.clone();
        let cancelled_worker = cancelled.clone();
        let worker = tokio::task::spawn_blocking(move || {
            let mut helper = None;
            let result = run_helper_with_launcher(
                &partial,
                &final_path,
                0,
                receiver,
                &cancelled_worker,
                |_| {},
                |status, pid| {
                    let lease = partial.with_extension("lease");
                    let output = partial.clone();
                    let status = status.to_path_buf();
                    let pid = pid.to_path_buf();
                    helper = Some(thread::spawn(move || {
                        // Mimic LaunchServices returning before the process executes its first instruction.
                        open_rx.recv_timeout(Duration::from_secs(3)).unwrap();
                        assert!(!lease.exists());
                        assert!(!output.exists());
                        assert!(!status.exists());
                        assert!(!pid.exists());
                    }));
                    let _ = started_tx.send(());
                    true
                },
            );
            (result, helper.unwrap())
        });
        started_rx.await.unwrap();
        f.repos.delete_note(&f.note_id).await.unwrap();
        monitor.await.unwrap();
        let (result, helper) = worker.await.unwrap();
        // Let the late process run only after the worker has completed its orphan cleanup.
        finalize_video_worker(
            &f.repos,
            &f.session_id,
            &f.partial,
            &f.final_path,
            &result,
            &cancelled,
        )
        .await;
        open_tx.send(()).unwrap();
        helper.join().unwrap();
        assert!(f.owned_files().iter().all(|path| !path.exists()));
    }

    #[tokio::test]
    async fn video_recovery_does_not_rename_a_live_workers_partial_after_helper_exit() {
        for event in ["recording", "stopped"] {
            let f = OwnerFixture::new().await;
            let nonce = f.seed_timing();
            let bytes = torn_container_fixture();
            std::fs::write(&f.partial, &bytes).unwrap();
            std::fs::write(f.partial.with_extension("lease"), b"active").unwrap();
            std::fs::write(
                f.partial.with_extension("status.json"),
                serde_json::json!({"event":event,"pid":2147483647,"captureNonce":nonce})
                    .to_string(),
            )
            .unwrap();
            let input = f
                .repos
                .note_video_input(&f.session_id, &f.paths)
                .await
                .unwrap()
                .unwrap();
            assert!(input.failure_reason.is_some());
            assert_eq!(std::fs::read(&f.partial).unwrap(), bytes);
            assert!(
                !f.final_path.exists(),
                "recovery must leave the worker's rename alone"
            );
            // The worker can still complete normally, with no persistent failed/recovered result.
            std::fs::rename(&f.partial, &f.final_path).unwrap();
            std::fs::remove_file(f.partial.with_extension("lease")).unwrap();
            assert!(f
                .repos
                .finalize_video_artifact(&f.session_id, &VideoResult::default())
                .await
                .unwrap());
            let input = f
                .repos
                .note_video_input(&f.session_id, &f.paths)
                .await
                .unwrap()
                .unwrap();
            assert!(input.failure_reason.is_none() && !input.path.is_empty());
        }
    }

    #[test]
    fn video_terminal_status_does_not_allow_recovery_while_helper_is_alive() {
        let dir = tempfile::tempdir().unwrap();
        let partial = dir.path().join("window.partial.mp4");
        pending_owner_binding(&partial);
        std::fs::write(
            partial.with_extension("status.json"),
            serde_json::json!({
                "event":"stopped", "pid":42, "processStartSec":30,"processStartUsec":40,
                "captureNonce":"d0bee8db-9cf8-4fc3-94d9-e0c911e6db13"
            })
            .to_string(),
        )
        .unwrap();
        assert!(matches!(
            video_recovery_state_with_probe(&partial, |pid| {
                if pid == 77 {
                    HelperProcessProbe::Gone
                } else {
                    HelperProcessProbe::Running {
                        start_sec: 30,
                        start_usec: 40,
                        executable: "/owned/clovy-video-capture".into(),
                    }
                }
            }),
            VideoRecoveryState::Active
        ));
        assert!(matches!(
            video_recovery_state_with_probe(&partial, |_| HelperProcessProbe::Gone),
            VideoRecoveryState::Finished(None)
        ));
    }

    #[test]
    fn video_terminal_status_keeps_lease_until_helper_exit() {
        struct FinishingProcess {
            partial: PathBuf,
            probes: std::sync::atomic::AtomicUsize,
        }
        impl HelperProcessControl for FinishingProcess {
            fn identity(&self, _: &HelperProcessRecord) -> HelperProcessMatch {
                let probe = self.probes.fetch_add(1, Ordering::SeqCst);
                // Model Swift's final lease check after publishing terminal status.
                assert!(self.partial.with_extension("lease").exists());
                assert!(self.partial.exists());
                if probe == 0 {
                    HelperProcessMatch::Matches
                } else {
                    HelperProcessMatch::GoneOrReused
                }
            }
            fn send(&self, _: u32, _: HelperSignal) {}
        }
        for event in ["stopped", "failed"] {
            let dir = tempfile::tempdir().unwrap();
            let partial = dir.path().join("window.partial.mp4");
            let final_path = dir.path().join("window.mp4");
            let process = FinishingProcess {
                partial: partial.clone(),
                probes: Default::default(),
            };
            let (_sender, receiver) = mpsc::channel();
            run_helper_with_launcher_and_process(
                &partial,
                &final_path,
                0,
                receiver,
                &AtomicBool::new(false),
                |_| {},
                |status, _| {
                    std::fs::write(&partial, [1_u8; 64]).unwrap();
                    std::fs::write(partial.with_extension("capture.json"), "binding").unwrap();
                    std::fs::write(partial.with_extension("timing.json"), "timing").unwrap();
                    std::fs::write(
                        status,
                        serde_json::json!({"event":event,"pid":42}).to_string(),
                    )
                    .unwrap();
                    true
                },
                &process,
            );
            assert!(process.probes.load(Ordering::SeqCst) >= 2);
            assert!(final_path.exists());
            assert!(!partial.with_extension("lease").exists());
            assert_eq!(
                std::fs::read(partial.with_extension("capture.json")).unwrap(),
                b"binding"
            );
            assert_eq!(
                std::fs::read(partial.with_extension("timing.json")).unwrap(),
                b"timing"
            );
        }
    }

    #[tokio::test]
    async fn video_normal_stop_preserves_output_until_note_is_deleted() {
        let f = OwnerFixture::new().await;
        let (_sender, receiver) = mpsc::channel();
        let result = run_helper_with_launcher(
            &f.partial,
            &f.final_path,
            0,
            receiver,
            &AtomicBool::new(false),
            |_| {},
            |status, _| {
                std::fs::write(&f.partial, [1_u8; 64]).unwrap();
                std::fs::write(
                    status,
                    r#"{"event":"stopped","pid":0,"startedAtHostMs":1000}"#,
                )
                .unwrap();
                true
            },
        );
        finalize_video_worker(
            &f.repos,
            &f.session_id,
            &f.partial,
            &f.final_path,
            &result,
            &AtomicBool::new(false),
        )
        .await;
        assert!(f.final_path.exists());
        assert!(!f.partial.with_extension("lease").exists());
        let files = f
            .repos
            .audio_artifact_paths_for_note(&f.note_id)
            .await
            .unwrap();
        f.repos.delete_note(&f.note_id).await.unwrap();
        for file in files {
            let _ = f.paths.remove_recording_file(&file);
        }
        assert!(f.owned_files().iter().all(|path| !path.exists()));
    }

    #[tokio::test]
    async fn video_folder_delete_cleans_owned_files_and_false_keeps_them() {
        for delete_notes in [false, true] {
            let f = OwnerFixture::new().await;
            let audio_files = f.seed_audio_files().await;
            for file in f.owned_files() {
                std::fs::write(file, b"owned").unwrap();
            }
            let folder = f
                .repos
                .create_folder("video-owner-test", "Project", None)
                .await
                .unwrap();
            f.repos
                .assign_note_to_folder("video-owner-test", &f.note_id, &folder.id)
                .await
                .unwrap();
            let files = f
                .repos
                .delete_folder_with_artifact_paths(&folder.id, delete_notes)
                .await
                .unwrap();
            if delete_notes {
                assert!(files.first().unwrap().ends_with(".lease"));
                for file in files {
                    let _ = f.paths.remove_recording_file(&file);
                }
                assert!(f.owned_files().iter().all(|path| !path.exists()));
                assert!(!f.repos.video_artifact_exists(&f.session_id).await.unwrap());
                assert!(audio_files.iter().all(|path| !path.exists()));
            } else {
                assert!(files.is_empty());
                assert!(audio_files.iter().all(|path| path.exists()));
                assert!(f.owned_files().iter().all(|path| path.exists()));
                assert!(f.repos.video_artifact_exists(&f.session_id).await.unwrap());
            }
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn swift_video_lease_late_output_cleanup_and_timing() {
        let temp = tempfile::tempdir().unwrap();
        let native = Path::new(env!("CARGO_MANIFEST_DIR")).join("native/mac-video-capture");
        let binary = temp.path().join("lease-tests");
        // Compile the actual pure protocol/timing definitions without the capture app entry point.
        let source = std::fs::read_to_string(native.join("main.swift")).unwrap();
        let model = temp.path().join("model.swift");
        std::fs::write(
            &model,
            source.split("// The main actor owns").next().unwrap(),
        )
        .unwrap();
        assert!(std::process::Command::new("/usr/bin/xcrun")
            .args(["swiftc", "-parse-as-library", "-module-cache-path"])
            .arg(
                std::env::var_os("CARGO_TARGET_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("target"))
                    .join("swift-module-cache"),
            )
            .arg(&model)
            .arg(native.join("lease-tests.swift"))
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new(binary)
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn video_denial_mid_capture_failure_and_early_close_preserve_the_real_wav() {
        use crate::audio::capture_buffer::MicrophoneWriter;
        use std::sync::{atomic::AtomicBool, Arc, Mutex};
        for scenario in ["permission_denied", "capture_failed", "closed"] {
            let temp = tempfile::tempdir().unwrap();
            let wav_path = temp.path().join("microphone.wav");
            let wav = hound::WavWriter::create(
                &wav_path,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 48000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            let (input, writer) =
                MicrophoneWriter::start(wav, 48000, 1, None, Arc::new(Mutex::new(None))).unwrap();
            let paused = Arc::new(AtomicBool::new(false));
            let ducked = Arc::new(AtomicBool::new(false));
            crate::audio::capture::write_input_data(
                std::iter::repeat(0.25_f32).take(4800),
                &input,
                &paused,
                &ducked,
            );
            let (_sender, receiver) = mpsc::channel();
            let partial = temp.path().join("window.partial.mp4");
            let result = run_helper_with_launcher(
                &partial,
                &temp.path().join("window.mp4"),
                0,
                receiver,
                &AtomicBool::new(false),
                |_| {},
                |status, _| {
                    let bytes = if scenario == "closed" {
                        std::fs::write(&partial, [1_u8; 64]).unwrap();
                        r#"{"event":"stopped","pid":0,"startedAtHostMs":1000}"#.to_string()
                    } else {
                        format!(r#"{{"event":"failed","pid":0,"failureReason":"{scenario}"}}"#)
                    };
                    std::fs::write(status, bytes).unwrap();
                    true
                },
            );
            assert_eq!(result.failure_reason.is_none(), scenario == "closed");
            crate::audio::capture::write_input_data(
                std::iter::repeat(-0.25_f32).take(4800),
                &input,
                &paused,
                &ducked,
            );
            let health = writer.finish().unwrap();
            assert_eq!(health.written_bytes(), 19200);
            let mut reader = hound::WavReader::open(&wav_path).unwrap();
            assert_eq!(reader.duration(), 9600);
            let samples = reader
                .samples::<i16>()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            assert!(samples[..4800].iter().all(|s| *s > 0));
            assert!(samples[4800..].iter().all(|s| *s < 0));
        }
    }

    #[tokio::test]
    async fn video_parent_exit_recovers_playable_fragments_but_genuine_capture_failure_stays_failed(
    ) {
        let _settings_lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let env = crate::llm::shell_env::login_env().await;
        let Some(ffmpeg) = env.which("ffmpeg") else {
            eprintln!("video fixture needs ffmpeg; skipped");
            return;
        };
        if env.which("ffprobe").is_none() {
            eprintln!("video fixture needs ffprobe; skipped");
            return;
        }
        for genuine_failure in [false, true] {
            let f = OwnerFixture::new().await;
            assert!(tokio::process::Command::new(&ffmpeg)
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=size=64x64:rate=5",
                    "-t",
                    "3",
                    "-an",
                    "-c:v",
                    "libx264",
                    "-movflags",
                    "frag_keyframe+empty_moov+default_base_moof",
                    "-frag_duration",
                    "500000",
                    "-f",
                    "mp4"
                ])
                .arg(&f.partial)
                .status()
                .await
                .unwrap()
                .success());
            let bytes = std::fs::read(&f.partial).unwrap();
            std::fs::write(&f.partial, &bytes[..bytes.len() - 32]).unwrap();
            let nonce = f.seed_timing();
            std::fs::write(f.partial.with_extension("status.json"),serde_json::json!({
                "event":if genuine_failure {"failed"}else{"stopped"}, "pid":2147483647,"captureNonce":nonce,
                "failureReason":genuine_failure.then_some("capture_failed")
            }).to_string()).unwrap();
            let input = f
                .repos
                .note_video_input(&f.session_id, &f.paths)
                .await
                .unwrap()
                .unwrap();
            if genuine_failure {
                assert!(input.path.is_empty());
                assert_eq!(input.failure_reason.as_deref(), Some("capture_failed"));
                assert_eq!(
                    std::fs::read(&f.partial).unwrap(),
                    bytes[..bytes.len() - 32]
                );
                assert!(!f.final_path.exists());
            } else {
                assert!(input.failure_reason.is_none());
                assert_eq!(input.offset_ms, 1250);
                assert!(playable_mp4(&f.final_path).await);
                assert!(!f.partial.exists());
            }
        }
    }

    #[tokio::test]
    async fn recovery_accepts_decoded_fragmented_mp4_and_rejects_garbage() {
        let _settings_lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let env = crate::llm::shell_env::login_env().await;
        let Some(ffmpeg) = env.which("ffmpeg") else {
            eprintln!("video fixture needs ffmpeg; skipped");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let video = temp.path().join("fragmented.mp4");
        assert!(tokio::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=size=64x64:rate=5",
                "-t",
                "3",
                "-c:v",
                "libx264",
                "-movflags",
                "frag_keyframe+empty_moov+default_base_moof",
                "-frag_duration",
                "500000",
                "-f",
                "mp4"
            ])
            .arg(&video)
            .status()
            .await
            .unwrap()
            .success());
        assert!(playable_mp4(&video).await);
        assert!(mp4_recovery::recoverable(&video));
        let bytes = std::fs::read(&video).unwrap();
        let truncated = temp.path().join("truncated.mp4");
        std::fs::write(&truncated, &bytes[..bytes.len() - 32]).unwrap();
        assert!(playable_mp4(&truncated).await);
        assert!(mp4_recovery::recoverable(&truncated));
        assert!(mp4_recovery::recover(&truncated));
        assert!(playable_mp4(&truncated).await);
        let garbage = temp.path().join("garbage.mp4");
        std::fs::write(&garbage, [1_u8; 64]).unwrap();
        assert!(!playable_mp4(&garbage).await);
    }

    #[test]
    fn pause_before_first_frame_excludes_only_actual_pause() {
        let mut timeline = PauseTimeline::default();
        timeline.pause(2000);
        timeline.resume(5000);
        // First frame arrives at 7000 after audio started at 1000: 3s active offset.
        assert_eq!(7000 - 1000 - timeline.duration_before(7000), 3000);
        assert_eq!(timeline.duration_before(3000), 1000);
    }
    #[test]
    fn helper_first_frame_offset_drains_pre_ready_pause_and_resume() {
        let temp = tempfile::tempdir().unwrap();
        let (sender, receiver) = mpsc::channel();
        let partial = temp.path().join("window.partial.mp4");
        let result = run_helper_with_launcher(
            &partial,
            &temp.path().join("window.mp4"),
            1000,
            receiver,
            &AtomicBool::new(false),
            |_| {},
            |status, _| {
                sender.send(Action::Pause(1100)).unwrap();
                sender.send(Action::Resume(1200)).unwrap();
                std::fs::write(&partial, [1_u8; 64]).unwrap();
                std::fs::write(
                    status,
                    r#"{"event":"stopped","pid":0,"startedAtHostMs":1250}"#,
                )
                .unwrap();
                true
            },
        );
        assert_eq!(result.offset_ms, 150);
    }

    #[cfg(unix)]
    #[test]
    fn fake_helper_receives_pause_resume_and_stop() {
        let temp = tempfile::tempdir().unwrap();
        let (sender, receiver) = mpsc::channel();
        let partial = temp.path().join("window.partial.mp4");
        let log = temp.path().join("signals.log");
        let controls = thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            sender.send(Action::Pause(host_ms())).unwrap();
            thread::sleep(Duration::from_millis(300));
            sender.send(Action::Resume(host_ms())).unwrap();
            thread::sleep(Duration::from_millis(300));
            sender.send(Action::Stop).unwrap();
        });
        let mut child = None;
        let result = run_helper_with_launcher(
            &partial,
            &temp.path().join("window.mp4"),
            0,
            receiver,
            &AtomicBool::new(false),
            |_| {},
            |status, pid| {
                std::fs::write(&partial, [1_u8; 64]).unwrap();
                let script = r#"
                trap 'echo pause >> "$VIDEO_TEST_LOG"' USR1
                trap 'echo resume >> "$VIDEO_TEST_LOG"' USR2
                finish() { echo stop >> "$VIDEO_TEST_LOG"; printf '{"event":"stopped","pid":%s,"startedAtHostMs":1000}' "$$" > "$VIDEO_TEST_STATUS"; exit 0; }
                trap finish TERM
                echo $$ > "$VIDEO_TEST_PID"
                printf '{"event":"recording","pid":%s,"startedAtHostMs":1000}' "$$" > "$VIDEO_TEST_STATUS"
                while :; do sleep 0.05; done
            "#;
                let mut helper = std::process::Command::new("/bin/sh")
                    .arg("-c")
                    .arg(script)
                    .env("VIDEO_TEST_LOG", &log)
                    .env("VIDEO_TEST_STATUS", status)
                    .env("VIDEO_TEST_PID", pid)
                    .spawn()
                    .unwrap();
                // Reap concurrently: kill(pid, 0) also reports an unreaped child
                // as live, while production's launched helper is reaped by the OS.
                child = Some(thread::spawn(move || helper.wait().unwrap()));
                true
            },
        );
        controls.join().unwrap();
        child.unwrap().join().unwrap();
        assert!(result.failure_reason.is_none());
        let events = std::fs::read_to_string(log).unwrap();
        assert!(events.contains("pause"));
        assert!(events.contains("resume"));
        assert!(events.contains("stop"));
    }

    #[test]
    fn permission_denied_is_metadata_and_never_an_audio_error() {
        let temp = tempfile::tempdir().unwrap();
        let (sender, receiver) = mpsc::channel();
        let partial = temp.path().join("window.partial.mp4");
        let result = run_helper_with_launcher(
            &partial,
            &temp.path().join("window.mp4"),
            0,
            receiver,
            &AtomicBool::new(false),
            |_| {},
            |status, _| {
                std::fs::write(
                    status,
                    r#"{"event":"failed","pid":0,"failureReason":"permission_denied"}"#,
                )
                .unwrap();
                true
            },
        );
        drop(sender);
        assert_eq!(result.failure_reason.as_deref(), Some("permission_denied"));
    }
    #[test]
    fn early_window_close_saves_completed_video_and_offset() {
        let temp = tempfile::tempdir().unwrap();
        let (_sender, receiver) = mpsc::channel();
        let partial = temp.path().join("window.partial.mp4");
        let final_path = temp.path().join("window.mp4");
        let result = run_helper_with_launcher(
            &partial,
            &final_path,
            1000,
            receiver,
            &AtomicBool::new(false),
            |_| {},
            |status, _| {
                std::fs::write(&partial, [1_u8; 64]).unwrap();
                std::fs::write(status,r#"{"event":"stopped","pid":0,"startedAtHostMs":1250,"app":"Chrome","title":"Meet","durationMs":5000}"#).unwrap();
                true
            },
        );
        assert!(result.failure_reason.is_none());
        assert_eq!(result.offset_ms, 250);
        assert_eq!(result.duration_ms, Some(5000));
        assert!(final_path.is_file());
        assert!(!partial.exists());
    }
    #[test]
    fn video_launch_failure_is_reported_without_a_capture_error() {
        let temp = tempfile::tempdir().unwrap();
        let (_sender, receiver) = mpsc::channel();
        let result = run_helper_with_launcher(
            &temp.path().join("window.partial.mp4"),
            &temp.path().join("window.mp4"),
            0,
            receiver,
            &AtomicBool::new(false),
            |_| {},
            |_, _| false,
        );
        assert_eq!(result.failure_reason.as_deref(), Some("capture_failed"));
    }
}
