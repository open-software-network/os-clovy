//! Private agent-browser client. The stdio pipes are inherited capabilities;
//! no command socket, CDP port, global CLI or user browser is involved.
#[path = "../../../scripts/agent-browser/private-host/clovy_browser_grant.rs"]
#[allow(dead_code)] // The same module also contains the host-side ownership admission.
mod clovy_browser_grant;
#[path = "../../../scripts/agent-browser/private-host/clovy_browser_process_identity.rs"]
mod clovy_browser_process_identity;
#[path = "../../../scripts/agent-browser/private-host/clovy_browser_recovery.rs"]
mod clovy_browser_recovery;
use super::{
    policy::{validate_final_public_url, PolicyConfig, Resolver, SystemResolver},
    proxy::PinningProxy,
};
#[cfg(test)]
use clovy_browser_grant::open_existing_plain;
use clovy_browser_grant::{lock_profile, Grant, Owned};
use clovy_browser_recovery::{RecoveryProfile, RecoveryRoot};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

const WIRE_LIMIT: u64 = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone)]
pub struct EngineError {
    pub code: String,
    pub message: String,
}
impl EngineError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}
type Result<T> = std::result::Result<T, EngineError>;
fn closed() -> EngineError {
    EngineError::new("browser_session_closed", "The browser session has ended.")
}
fn unknown() -> EngineError {
    EngineError::new(
        "browser_outcome_unknown",
        "The browser outcome is unknown. The operation may have happened; do not repeat it.",
    )
}
fn post_network_error(read_only: bool, error: EngineError) -> EngineError {
    if read_only {
        return error;
    }
    // validate_url normalizes every policy/resolver failure to this safe cause.
    // A successful mutation response precedes this check, so reporting refusal
    // would hide possible delivery and invite the caller to replay the effect.
    EngineError::new(
        "browser_outcome_unknown",
        "The browser operation may have happened, but its post-action network validation failed (browser_network_blocked). Its task has been stopped. Do not repeat it.",
    )
}
fn failed() -> EngineError {
    EngineError::new(
        "browser_start_failed",
        "Clovy could not start its browser. Prepare or reinstall the bundled browser resources.",
    )
}

fn verify_host_ready(
    value: &Value,
    token: &str,
    grant: &Grant,
    persistent: bool,
    child_pid: u32,
    child_birth: u64,
    job_name: Option<&str>,
) -> Result<()> {
    if value["protocol"] != 1 || value["token"].as_str() != Some(token) {
        return Err(failed());
    }
    if persistent {
        if value.get("owned").is_some() {
            return Err(failed());
        }
    } else {
        let owned: Owned = serde_json::from_value(value.get("owned").cloned().ok_or_else(failed)?)
            .map_err(|_| failed())?;
        grant
            .verify_owned(&owned, child_pid, child_birth, job_name)
            .map_err(|_| failed())?;
    }
    Ok(())
}

#[derive(Clone)]
pub struct EngineConfig {
    pub engine: PathBuf,
    pub chrome: PathBuf,
    pub persistent_profile: PathBuf,
    pub temporary_root: PathBuf,
    pub persistent: bool,
    pub policy: PolicyConfig,
    pub resolver: Arc<dyn Resolver>,
}
impl EngineConfig {
    pub(crate) fn chrome_namespace(&self) -> Option<PathBuf> {
        self.engine.parent().map(|parent| parent.join("chrome"))
    }

    pub(crate) fn production_paths(resources: PathBuf) -> (PathBuf, PathBuf) {
        let platform = if cfg!(target_os = "windows") {
            "win64"
        } else if cfg!(target_os = "macos") {
            if cfg!(target_arch = "aarch64") {
                "mac-arm64"
            } else {
                "mac-x64"
            }
        } else {
            "linux64"
        };
        let chrome = match platform {
            "win64" => "chrome/chrome-win64/chrome.exe",
            "mac-arm64" => "chrome/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            "mac-x64" => "chrome/chrome-mac-x64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing",
            _ => "chrome/chrome-linux64/chrome",
        };
        let resources = resources.join(platform);
        (
            resources.join(if cfg!(windows) {
                "clovy-agent-browser.exe"
            } else {
                "clovy-agent-browser"
            }),
            resources.join(chrome),
        )
    }

    pub fn production(resources: PathBuf, data: PathBuf, persistent: bool) -> Self {
        let (engine, chrome) = Self::production_paths(resources);
        Self {
            engine,
            chrome,
            persistent_profile: data.join("browser-profile"),
            temporary_root: data.join("browser-temporary"),
            persistent,
            policy: PolicyConfig::default(),
            resolver: Arc::new(SystemResolver),
        }
    }
}

struct Request {
    value: Value,
    reply: oneshot::Sender<Result<Value>>,
}
struct Process {
    child: Child,
    #[cfg(windows)]
    job: crate::process_group::windows_job::Job,
}
impl Process {
    fn stop(&mut self, grace: Duration) {
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGTERM);
        }
        #[cfg(windows)]
        self.job.terminate();
        let end = Instant::now() + grace.min(Duration::from_millis(250));
        while Instant::now() < end {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        // Always kill the group even if the leader has exited first.
        #[cfg(unix)]
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    fn tree_exited(&self) -> bool {
        #[cfg(windows)]
        {
            self.job.is_empty().unwrap_or(false)
        }
        #[cfg(unix)]
        {
            unix_group_exited(self.child.id() as i32)
        }
    }
}
struct Slot {
    cancelled: AtomicBool,
    suspended: AtomicBool,
    visible: AtomicBool,
    started: AtomicBool,
    sequence: AtomicU64,
    process: Mutex<Option<Process>>,
    sender: Mutex<Option<mpsc::UnboundedSender<Request>>>,
    proxy: Mutex<Option<PinningProxy>>,
    lease: Mutex<Option<File>>,
    recovery: Mutex<Option<RecoveryProfile>>,
    profile: PathBuf,
    persistent: bool,
    last_used: Mutex<Instant>,
    /// Capture/dispatch and takeover share this lock. Stop never acquires it.
    operation: tokio::sync::Mutex<()>,
}
impl Slot {
    fn stop(&self) {
        self.stop_with_grace(Duration::from_millis(250));
    }
    fn stop_with_grace(&self, grace: Duration) {
        if self.cancelled.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Some(sender) = self.sender.lock().unwrap().as_ref() {
            let (reply, _) = oneshot::channel();
            let _=sender.send(Request{value:json!({"id":self.sequence.fetch_add(1,Ordering::SeqCst),"action":"shutdown"}),reply});
        }
        let process = self.process.lock().unwrap().take();
        let lease = self.lease.lock().unwrap().take();
        let recovery = self.recovery.lock().unwrap().take();
        let nonce = self
            .profile
            .file_name()
            .and_then(|s| s.to_str())
            .map(str::to_owned);
        if let Some(mut process) = process {
            process.stop(grace);
            if !process.tree_exited() {
                // Tree termination can be asynchronous. The reaper owns the
                // group/Job and lease until every descendant has exited, without
                // holding a mutex needed by Stop or another browser call.
                std::thread::spawn(move || {
                    while !process.tree_exited() {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    if let (Some(recovery), Some(nonce)) = (recovery, nonce) {
                        delete_stopped_profile(recovery, lease, &nonce);
                    }
                });
                if let Some(proxy) = self.proxy.lock().unwrap().take() {
                    proxy.shutdown();
                }
                self.sender.lock().unwrap().take();
                return;
            }
        }
        self.sender.lock().unwrap().take();
        if let Some(proxy) = self.proxy.lock().unwrap().take() {
            proxy.shutdown();
        }
        // The lease is released only after the entire owned process group/job exits.
        if let (Some(recovery), Some(nonce)) = (recovery, nonce) {
            delete_stopped_profile(recovery, lease, &nonce);
        }
    }
}
impl Drop for Slot {
    fn drop(&mut self) {
        self.stop();
    }
}

pub struct AgentBrowserTransport {
    config: EngineConfig,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
}
impl AgentBrowserTransport {
    pub fn new(config: EngineConfig) -> Arc<Self> {
        recover_reserved_profiles(&config);
        let transport = Arc::new(Self {
            config,
            slots: Mutex::new(HashMap::new()),
        });
        let weak = Arc::downgrade(&transport);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(transport) = weak.upgrade() else {
                    break;
                };
                transport.reap_idle();
            }
        });
        transport
    }
    fn reap_idle(&self) {
        let expired: Vec<_> = self
            .slots
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, slot)| {
                !slot.persistent
                    && slot.last_used.lock().unwrap().elapsed() >= Duration::from_secs(300)
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.terminate(&id);
        }
    }
    pub fn reserve(&self, id: &str) -> Result<()> {
        let mut slots = self.slots.lock().unwrap();
        slots.retain(|_, slot| !slot.cancelled.load(Ordering::SeqCst));
        if slots.contains_key(id) {
            return Err(closed());
        }
        if !self.config.persistent && slots.len() >= 2 {
            return Err(EngineError::new(
                "browser_session_limit",
                "Two routine browser sessions are already running.",
            ));
        }
        let profile = if self.config.persistent {
            self.config.persistent_profile.clone()
        } else {
            self.config
                .temporary_root
                .join(uuid::Uuid::new_v4().to_string())
        };
        let recovery = if self.config.persistent {
            fs::create_dir_all(&profile).map_err(|_| failed())?;
            None
        } else {
            if !self.config.temporary_root.is_absolute() || roots_overlap(&self.config) {
                return Err(failed());
            }
            let root =
                RecoveryRoot::open_or_create(&self.config.temporary_root).map_err(|_| failed())?;
            if roots_overlap(&self.config) {
                return Err(failed());
            }
            Some(
                root.create_profile(profile.file_name().ok_or_else(failed)?)
                    .map_err(|_| failed())?,
            )
        };
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if recovery.is_none() {
                fs::set_permissions(&profile, fs::Permissions::from_mode(0o700))
                    .map_err(|_| failed())?;
            }
        }
        let lease = match &recovery {
            Some(recovery) => recovery.create_file(".clovy-lease").map_err(|_| failed())?,
            None => OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(profile.join(".clovy-lease"))
                .map_err(|_| failed())?,
        };
        lock_profile(&lease).map_err(|_| {
            EngineError::new(
                "browser_profile_busy",
                "Clovy's browser profile is in use by another task.",
            )
        })?;
        if let Some(recovery) = &recovery {
            publish_reserved(
                recovery,
                profile
                    .file_name()
                    .and_then(|s| s.to_str())
                    .ok_or_else(failed)?,
            )
            .map_err(|_| failed())?;
        }
        slots.insert(
            id.into(),
            Arc::new(Slot {
                cancelled: AtomicBool::new(false),
                suspended: AtomicBool::new(false),
                visible: AtomicBool::new(false),
                started: AtomicBool::new(false),
                sequence: AtomicU64::new(1),
                process: Mutex::new(None),
                sender: Mutex::new(None),
                proxy: Mutex::new(None),
                lease: Mutex::new(Some(lease)),
                recovery: Mutex::new(recovery),
                profile,
                persistent: self.config.persistent,
                last_used: Mutex::new(Instant::now()),
                operation: tokio::sync::Mutex::new(()),
            }),
        );
        Ok(())
    }
    fn slot(&self, id: &str) -> Result<Arc<Slot>> {
        self.slots
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .filter(|s| !s.cancelled.load(Ordering::SeqCst))
            .ok_or_else(closed)
    }
    pub fn terminate(&self, id: &str) {
        self.terminate_with_grace(id, Duration::from_millis(250));
    }
    pub fn terminate_with_grace(&self, id: &str, grace: Duration) {
        if let Some(slot) = self.slots.lock().unwrap().remove(id) {
            slot.stop_with_grace(grace);
        }
    }
    pub fn shutdown(&self) {
        let slots: Vec<_> = self
            .slots
            .lock()
            .unwrap()
            .drain()
            .map(|(_, slot)| slot)
            .collect();
        for slot in slots {
            slot.stop();
        }
    }
    pub fn state(&self, id: &str) -> Result<Value> {
        let slot = self.slot(id)?;
        Ok(
            json!({"sessionId":id,"mode":if slot.visible.load(Ordering::SeqCst) {"visible"} else {"background"},"suspended":slot.suspended.load(Ordering::SeqCst)}),
        )
    }
    async fn start(&self, slot: &Arc<Slot>, headed: bool) -> Result<Value> {
        if slot.cancelled.load(Ordering::SeqCst) {
            return Err(closed());
        }
        if slot.started.swap(true, Ordering::SeqCst) {
            return Err(closed());
        }
        if !self.config.engine.is_file() || !self.config.chrome.is_file() {
            return Err(failed());
        }
        verify_bundle(&self.config)?;
        let proxy = PinningProxy::start(self.config.resolver.clone(), self.config.policy.clone())
            .await
            .map_err(|_| failed())?;
        let port = proxy.port();
        {
            let mut guard = slot.proxy.lock().unwrap();
            if slot.cancelled.load(Ordering::SeqCst) {
                proxy.shutdown();
                return Err(closed());
            }
            *guard = Some(proxy);
        }
        let mut command = Command::new(&self.config.engine);
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Keep the normal HOME/native credential store. None of upstream's
        // AGENT_BROWSER_* launch/attach/plugin/environment controls survive.
        for key in [
            "HOME",
            "USERPROFILE",
            "LOCALAPPDATA",
            "APPDATA",
            "SystemRoot",
            "WINDIR",
            "TEMP",
            "TMP",
            "DISPLAY",
            "XAUTHORITY",
            "DBUS_SESSION_BUS_ADDRESS",
            "XDG_RUNTIME_DIR",
        ] {
            if let Some(value) = std::env::var_os(key) {
                command.env(key, value);
            }
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x00000004 | 0x08000000);
        }
        // This mutex covers only spawn/publication, never CDP or wire I/O.
        let (stdin, stdout, grant, child_pid, child_birth, job_name) = {
            let mut process_guard = slot.process.lock().unwrap();
            if slot.cancelled.load(Ordering::SeqCst) {
                return Err(closed());
            }
            if !slot.persistent {
                let recovery = slot.recovery.lock().unwrap();
                mark_starting(
                    recovery.as_ref().ok_or_else(failed)?,
                    slot.profile
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or_else(failed)?,
                )
                .map_err(|_| failed())?;
            }
            let grant = Grant::new(&slot.profile, slot.persistent).map_err(|_| failed())?;
            #[cfg(windows)]
            let job_name = (!slot.persistent).then(|| grant.browser_job_name());
            #[cfg(not(windows))]
            let job_name: Option<String> = None;
            #[cfg(windows)]
            let job = match job_name.as_deref() {
                Some(name) => crate::process_group::windows_job::Job::new_named(name),
                None => crate::process_group::windows_job::Job::new(),
            }
            .map_err(|_| failed())?;
            let mut child = command.spawn().map_err(|_| failed())?;
            #[cfg(windows)]
            {
                use std::os::windows::io::AsRawHandle;
                if job
                    .assign(child.as_raw_handle())
                    .and_then(|_| {
                        crate::process_group::windows_job::resume_first_thread(child.id())
                    })
                    .is_err()
                {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(failed());
                }
            }
            let child_pid = child.id();
            let child_birth = match clovy_browser_process_identity::process_birth(child_pid) {
                Ok(birth) => birth,
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(failed());
                }
            };
            let Some(stdin) = child.stdin.take() else {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failed());
            };
            let Some(stdout) = child.stdout.take() else {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failed());
            };
            *process_guard = Some(Process {
                child,
                #[cfg(windows)]
                job,
            });
            (stdin, stdout, grant, child_pid, child_birth, job_name)
        };
        let (sender, mut receiver) = mpsc::unbounded_channel::<Request>();
        *slot.sender.lock().unwrap() = Some(sender);
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let (ready_tx, ready_rx) = oneshot::channel();
        let weak = Arc::downgrade(slot);
        let persistent = slot.persistent;
        std::thread::spawn(move || {
            let mut stdin = stdin;
            let mut stdout = BufReader::new(stdout);
            let handoff = if persistent {
                Ok(())
            } else {
                weak.upgrade().ok_or_else(closed).and_then(|slot| {
                    if slot.cancelled.load(Ordering::SeqCst) {
                        return Err(closed());
                    }
                    let lease = slot.lease.lock().unwrap().take().ok_or_else(closed)?;
                    drop(lease);
                    Ok(())
                })
            };
            let ready = handoff
                .and_then(|()| {
                    write_line(
                        &mut stdin,
                        &json!({"protocol":1,"token":token,"grant":grant,"jobName":job_name}),
                    )
                })
                .and_then(|_| read_line(&mut stdout));
            let ready = ready.and_then(|value| {
                verify_host_ready(
                    &value,
                    &token,
                    &grant,
                    persistent,
                    child_pid,
                    child_birth,
                    job_name.as_deref(),
                )
            });
            let okay = ready.is_ok();
            let _ = ready_tx.send(ready);
            if okay {
                while let Some(request) = receiver.blocking_recv() {
                    let id = request.value["id"].clone();
                    let response = write_line(&mut stdin, &request.value).and_then(|_| read_line(&mut stdout)).and_then(|value| {
                        if value["id"] != id { return Err(unknown()); }
                        match value["delivery"].as_str() {
                            Some("unknown") => Err(unknown()),
                            Some("not_attempted" | "delivered") => {
                                if let Some(error) = value.get("error") {
                                    let code = safe_code(error["code"].as_str().unwrap_or("browser_failed"));
                                    // Error text is host-authored. Never forward page/Chrome errors.
                                    let message = if code == "browser_network_blocked" {
                                        "This page is outside the browser's public web scope. Open a public web page before continuing."
                                    } else if code == "browser_human_takeover_required" {
                                        if persistent {
                                            "First call request_clarification and keep it pending while the user takes over the dedicated browser and resumes control. Then list tabs and inspect a fresh snapshot; never repeat sensitive input."
                                        } else {
                                            "This routine uses an anonymous browser and cannot enter sensitive fields or use a login profile."
                                        }
                                    } else if code == "browser_screenshot_requires_visible" {
                                        if persistent {
                                            "A screenshot of a hidden tab is unavailable. Read a fresh snapshot instead. If an image is required, call request_clarification and keep it pending while the user takes over the dedicated browser and resumes control. Then list tabs and inspect a fresh snapshot."
                                        } else {
                                            "A screenshot of a hidden tab is unavailable. This routine can read a fresh snapshot but cannot request human takeover."
                                        }
                                    } else {
                                        "The browser operation did not complete. Inspect its state before continuing."
                                    };
                                    Err(EngineError::new(code, message))
                                } else { Ok(value.get("data").cloned().unwrap_or(Value::Null)) }
                            }, _ => Err(unknown())
                        }
                    });
                    let lost = response
                        .as_ref()
                        .err()
                        .is_some_and(|e| e.code == "browser_outcome_unknown");
                    let _ = request.reply.send(response);
                    if lost {
                        break;
                    }
                }
            }
            if let Some(slot) = weak.upgrade() {
                slot.stop();
            }
        });
        tokio::time::timeout(Duration::from_secs(10), ready_rx)
            .await
            .map_err(|_| failed())?
            .map_err(|_| failed())??;
        let data = self.request(slot, json!({"action":"start", "chromePath":self.config.chrome, "profilePath":slot.profile,
            "persistent":slot.persistent,"proxyPort":port,"headed":headed})).await?;
        slot.visible.store(headed, Ordering::SeqCst);
        Ok(data)
    }
    async fn request(&self, slot: &Arc<Slot>, mut value: Value) -> Result<Value> {
        if slot.cancelled.load(Ordering::SeqCst) {
            return Err(closed());
        }
        value["id"] = json!(slot.sequence.fetch_add(1, Ordering::SeqCst));
        let (reply, receiver) = oneshot::channel();
        slot.sender
            .lock()
            .unwrap()
            .as_ref()
            .ok_or_else(closed)?
            .send(Request { value, reply })
            .map_err(|_| closed())?;
        *slot.last_used.lock().unwrap() = Instant::now();
        match tokio::time::timeout(REQUEST_TIMEOUT, receiver).await {
            Ok(Ok(result)) => result,
            _ => {
                slot.stop();
                Err(unknown())
            }
        }
    }
    pub async fn execute(&self, id: &str, tool: &str, arguments: Value) -> Result<Value> {
        let slot = self.slot(id)?;
        let _operation = slot.operation.lock().await;
        if slot.suspended.load(Ordering::SeqCst) {
            return Err(EngineError::new(
                "browser_human_takeover",
                "Browser control is paused for you. Resume control explicitly to continue.",
            ));
        }
        if tool == "start_session" {
            return match self.start(&slot, false).await {
                Ok(value) => Ok(value),
                Err(error) => {
                    slot.stop();
                    Err(error)
                }
            };
        }
        let action = match tool {
            "open_tab" => "tab_open",
            "list_tabs" => "tab_list",
            "switch_tab" => "tab_switch",
            "close_tab" => "tab_close",
            "click" | "fill" | "press" => "guarded_action",
            "navigate" | "back" | "snapshot" | "screenshot" | "inspect_reference" => tool,
            _ => {
                return Err(EngineError::new(
                    "unknown_browser_tool",
                    "Unknown browser operation.",
                ))
            }
        };
        if let Some(raw) = arguments.get("url").and_then(Value::as_str) {
            self.validate_url(raw).await?;
        }
        let mut request = arguments;
        if let Some(object) = request.as_object_mut() {
            object.remove("session_id");
        }
        request["action"] = json!(action);
        if action == "guarded_action" {
            request["operation"] = json!(tool);
        }
        let data = self.request(&slot, request).await?;
        let read_only = matches!(
            tool,
            "snapshot" | "screenshot" | "list_tabs" | "inspect_reference"
        );
        if slot.suspended.load(Ordering::SeqCst) {
            if !read_only {
                // Takeover may have been requested while an operation was
                // already being delivered. A pause is not a refusal of that
                // effect; invalidate the task instead of inviting a replay.
                slot.stop();
                return Err(unknown());
            }
            return Err(EngineError::new(
                "browser_human_takeover",
                "Browser control is paused for you. Resume explicitly to continue.",
            ));
        }
        if matches!(tool, "snapshot" | "screenshot")
            && data.get("url").and_then(Value::as_str).is_none()
        {
            slot.stop();
            return Err(EngineError::new(
                "browser_artifact_invalid",
                "The browser capture has no verifiable page scope. Its task has been stopped.",
            ));
        }
        if let Some(raw) = data.get("url").and_then(Value::as_str) {
            if let Err(error) = self.validate_url(raw).await {
                slot.stop();
                return Err(post_network_error(read_only, error));
            }
        }
        if let Some(tabs) = data.get("tabs").and_then(Value::as_array) {
            for tab in tabs {
                if let Some(raw) = tab.get("url").and_then(Value::as_str) {
                    if let Err(error) = self.validate_url(raw).await {
                        slot.stop();
                        return Err(post_network_error(read_only, error));
                    }
                }
            }
        }
        Ok(data)
    }
    async fn validate_url(&self, raw: &str) -> Result<()> {
        if raw == "about:blank" {
            return Ok(());
        }
        validate_final_public_url(raw, self.config.resolver.as_ref(), &self.config.policy)
            .await
            .map_err(|_| {
                EngineError::new(
                    "browser_network_blocked",
                    "The browser destination is blocked by its network policy.",
                )
            })
    }
    /// Called only by the local human UI. No model tool exposes this method.
    pub async fn takeover(&self, id: &str) -> Result<Value> {
        let slot = self.slot(id)?;
        if !slot.persistent {
            return Err(EngineError::new(
                "browser_takeover_unavailable",
                "Routine browsers cannot use the dedicated login profile.",
            ));
        }
        slot.suspended.store(true, Ordering::SeqCst);
        let _operation = slot.operation.lock().await;
        // The private host closes/flushed Chrome and reopens the same profile
        // headed, in the same supervised process group/job. Old refs/tabs die.
        let data = self.request(&slot, json!({"action":"takeover"})).await?;
        slot.visible.store(true, Ordering::SeqCst);
        Ok(data)
    }
    pub async fn resume(&self, id: &str) -> Result<()> {
        let slot = self.slot(id)?;
        let _operation = slot.operation.lock().await;
        self.request(&slot, json!({"action":"resume"})).await?;
        slot.suspended.store(false, Ordering::SeqCst);
        Ok(())
    }
}
impl Drop for AgentBrowserTransport {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn write_line(writer: &mut impl Write, value: &Value) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| unknown())?;
    if bytes.len() as u64 > WIRE_LIMIT {
        return Err(EngineError::new(
            "browser_request_too_large",
            "The browser request is too large.",
        ));
    }
    writer
        .write_all(&bytes)
        .and_then(|_| writer.write_all(b"\n"))
        .and_then(|_| writer.flush())
        .map_err(|_| unknown())
}
fn read_line(reader: &mut impl BufRead) -> Result<Value> {
    let mut line = Vec::new();
    reader
        .take(WIRE_LIMIT + 1)
        .read_until(b'\n', &mut line)
        .map_err(|_| unknown())?;
    if line.is_empty() || line.len() as u64 > WIRE_LIMIT || line.last() != Some(&b'\n') {
        return Err(unknown());
    }
    serde_json::from_slice(&line).map_err(|_| unknown())
}

fn safe_code(code: &str) -> &'static str {
    match code {
        "stale_reference" => "stale_reference",
        "tab_gone" => "tab_gone",
        "sensitive_field" => "browser_human_takeover_required",
        "human_takeover" => "browser_human_takeover",
        "unsupported_key" => "browser_unsupported_key",
        "unsupported_operation" => "browser_unsupported_operation",
        "no_history" => "browser_no_history",
        "invalid_url" => "browser_network_blocked",
        "screenshot_hidden" => "browser_screenshot_requires_visible",
        _ => "browser_operation_failed",
    }
}

/// Provenance checks are shared by release packaging and runtime startup.
pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_bundle(config: &EngineConfig) -> Result<()> {
    let root = config.engine.parent().ok_or_else(failed)?;
    let metadata: Value =
        serde_json::from_slice(&fs::read(root.join("provenance.json")).map_err(|_| failed())?)
            .map_err(|_| failed())?;
    if metadata["source"]["commit"] != "5868f7c05020e047c00e68e4abf9253cce864f41"
        || metadata["protocol"] != "clovy-agent-browser/1"
        || metadata["chrome"]["version"] != "156.0.8078.80"
    {
        return Err(failed());
    }
    // A debug binary must also reject resources built from an older private
    // guard/wire revision of the same upstream commit, even outside the normal
    // dev preparation hook. Release signing then protects the matching tree.
    let inputs: &[(&str, &[u8])] = &[
        (
            "0001-no-reference-recovery.patch",
            include_bytes!("../../../scripts/agent-browser/0001-no-reference-recovery.patch"),
        ),
        (
            "0002-private-host.patch",
            include_bytes!("../../../scripts/agent-browser/0002-private-host.patch"),
        ),
        (
            "private-host/clovy_private_main.rs",
            include_bytes!("../../../scripts/agent-browser/private-host/clovy_private_main.rs"),
        ),
        (
            "private-host/clovy_private_client.rs",
            include_bytes!("../../../scripts/agent-browser/private-host/clovy_private_client.rs"),
        ),
        (
            "private-host/clovy_private_launch.rs",
            include_bytes!("../../../scripts/agent-browser/private-host/clovy_private_launch.rs"),
        ),
        (
            "private-host/clovy_browser_process_identity.rs",
            include_bytes!(
                "../../../scripts/agent-browser/private-host/clovy_browser_process_identity.rs"
            ),
        ),
        (
            "private-host/clovy_browser_grant.rs",
            include_bytes!("../../../scripts/agent-browser/private-host/clovy_browser_grant.rs"),
        ),
        (
            "private-host/clovy_browser_recovery.rs",
            include_bytes!("../../../scripts/agent-browser/private-host/clovy_browser_recovery.rs"),
        ),
        (
            "private-host/clovy_private_guard.js",
            include_bytes!("../../../scripts/agent-browser/private-host/clovy_private_guard.js"),
        ),
        (
            "preparation-lock.mjs",
            include_bytes!("../../../scripts/agent-browser/preparation-lock.mjs"),
        ),
        (
            "windows-signing.mjs",
            include_bytes!("../../../scripts/agent-browser/windows-signing.mjs"),
        ),
    ];
    for &(name, bytes) in inputs {
        if metadata["source"]["inputSha256"][name].as_str()
            != Some(format!("{:x}", Sha256::digest(bytes)).as_str())
        {
            return Err(EngineError::new("browser_resource_modified","The private browser revision is stale. Prepare or reinstall its matching resources."));
        }
    }
    for path in [&config.engine, &config.chrome] {
        let relative = path
            .strip_prefix(root)
            .map_err(|_| failed())?
            .to_str()
            .ok_or_else(failed)?
            .replace('\\', "/");
        let expected = metadata["files"][&relative]["sha256"]
            .as_str()
            .ok_or_else(failed)?;
        if sha256_file(path).map_err(|_| failed())? != expected {
            return Err(EngineError::new("browser_resource_modified","Clovy's bundled browser resources changed. Prepare or reinstall them before using Browser use."));
        }
    }
    Ok(())
}

const RESERVED_RECEIPT: &str = ".clovy-reserved.json";
const STARTING_MARKER: &str = ".clovy-starting";

#[cfg(test)]
fn marker_absent(profile: &std::path::Path) -> bool {
    matches!(fs::symlink_metadata(profile.join(STARTING_MARKER)), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
}

fn delete_stopped_profile(profile: RecoveryProfile, held: Option<File>, nonce: &str) {
    if let Err(error) = profile.delete_stopped(held, nonce) {
        eprintln!("Browser anonymous profile retained: {error}");
    }
}

fn publish_reserved(profile: &RecoveryProfile, name: &str) -> std::io::Result<()> {
    if uuid::Uuid::parse_str(name).ok().map(|id| id.to_string()) != Some(name.to_owned()) {
        return Err(std::io::Error::other("invalid profile nonce"));
    }
    let mut file = profile.create_file(RESERVED_RECEIPT)?;
    serde_json::to_writer(
        &mut file,
        &json!({"version":1,"nonce":name,"ownerProcessId":std::process::id(),"ownerBirth":clovy_browser_process_identity::process_birth(std::process::id())?,"phase":"Reserved", "rootIdentity":profile.root_identity(), "profileIdentity":profile.identity()}),
    )?;
    file.sync_all()?;
    drop(file);
    profile.sync()
}

fn mark_starting(profile: &RecoveryProfile, name: &str) -> std::io::Result<()> {
    let mut receipt_file = profile.open_file(RESERVED_RECEIPT, false)?;
    let mut bytes = Vec::new();
    (&mut receipt_file).take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(std::io::Error::other("invalid reservation"));
    }
    let mut receipt: Value = serde_json::from_slice(&bytes)?;
    if receipt["version"] != 1
        || receipt["nonce"] != name
        || receipt["phase"] != "Reserved"
        || receipt["ownerProcessId"] != std::process::id()
        || receipt["ownerBirth"]
            != clovy_browser_process_identity::process_birth(std::process::id())?
        || receipt["rootIdentity"] != json!(profile.root_identity())
        || receipt["profileIdentity"] != json!(profile.identity())
        || !profile.same_file(RESERVED_RECEIPT, &receipt_file)
    {
        return Err(std::io::Error::other("invalid reservation"));
    }
    // Existence itself is conservative: even a crash during this write prevents
    // Reserved recovery from treating a possibly spawned engine as unused.
    let mut file = profile.create_file(STARTING_MARKER)?;
    file.write_all(b"Starting\n")?;
    file.sync_all()?;
    profile.sync()?;
    receipt["phase"] = json!("Starting");
    drop(receipt_file);
    profile.replace_receipt(&serde_json::to_vec(&receipt)?)
}

#[cfg(test)]
fn process_definitely_absent(pid: u32) -> bool {
    if pid == 0 || pid > i32::MAX as u32 {
        return false;
    }
    #[cfg(unix)]
    {
        (unsafe { libc::kill(pid as i32, 0) }) < 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    }
    #[cfg(windows)]
    {
        use windows::Win32::{
            Foundation::CloseHandle,
            System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
        };
        match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(handle) => {
                let _ = unsafe { CloseHandle(handle) };
                false
            }
            Err(error) => error.code().0 == 0x80070057u32 as i32,
        }
    }
}

fn roots_overlap(config: &EngineConfig) -> bool {
    let root = &config.temporary_root;
    let persistent = &config.persistent_profile;
    if persistent.starts_with(root) || root.starts_with(persistent) {
        return true;
    }
    let root = match fs::canonicalize(root) {
        Ok(root) => root,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    match fs::canonicalize(persistent) {
        Ok(persistent) => persistent.starts_with(&root) || root.starts_with(&persistent),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // A dangling symlink or an inaccessible path is not proof of separation.
            !matches!(fs::symlink_metadata(persistent), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
        }
        Err(_) => true,
    }
}

fn recover_reserved_profiles(config: &EngineConfig) {
    if roots_overlap(config) {
        return;
    }
    let Ok(root) = RecoveryRoot::open(&config.temporary_root) else {
        return;
    };
    match root.recover() {
        Ok(diagnostics) => {
            for diagnostic in diagnostics {
                eprintln!("Browser recovery retained {diagnostic}");
            }
        }
        Err(error) => eprintln!("Browser recovery enumeration failed: {error}"),
    }
}

#[cfg(unix)]
fn unix_group_exited(group: i32) -> bool {
    clovy_browser_process_identity::unix_group_exited(group)
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    fn physical_tempdir() -> tempfile::TempDir {
        // Resolve platform temp aliases before creating fixtures or adversarial links.
        let base = std::env::temp_dir().canonicalize().unwrap();
        tempfile::tempdir_in(base).unwrap()
    }
    fn test_profile(path: &std::path::Path) -> RecoveryProfile {
        RecoveryRoot::open(path.parent().unwrap())
            .unwrap()
            .profile(path.file_name().unwrap())
            .unwrap()
    }

    fn test_mark_starting(path: &std::path::Path) -> std::io::Result<()> {
        mark_starting(
            &test_profile(path),
            path.file_name().unwrap().to_str().unwrap(),
        )
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "requires a separately built real private host"]
    fn real_parent_crash_preserves_starting_and_owned_authority() {
        use std::os::{
            fd::{AsRawFd, FromRawFd},
            unix::process::CommandExt,
        };
        const MARKER: &str = "CLOVY_CRASH_STATE=";
        if let Some(root) = std::env::var_os("CLOVY_CRASH_TEST_ROOT") {
            let root = PathBuf::from(root);
            let mode = std::env::var("CLOVY_CRASH_TEST_MODE").unwrap();
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _enter = runtime.enter();
            let config = EngineConfig::production(root.join("resources"), root.join("data"), false);
            let transport = AgentBrowserTransport::new(config);
            transport.reserve("crash-owner").unwrap();
            let slot = transport.slot("crash-owner").unwrap();
            test_mark_starting(&slot.profile).unwrap();
            let grant = Grant::new(&slot.profile, false).unwrap();
            let mut host =
                Command::new(std::env::var_os("CLOVY_BROWSER_HOST_TEST_BINARY").unwrap())
                    .env_clear()
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .process_group(0)
                    .spawn()
                    .unwrap();
            let pid = host.id();
            let birth = clovy_browser_process_identity::process_birth(pid).unwrap();
            let token = "b".repeat(64);
            if mode != "owned" {
                assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGSTOP) }, 0);
                let mut status = 0;
                assert_eq!(
                    unsafe { libc::waitpid(pid as i32, &mut status, libc::WUNTRACED) },
                    pid as i32
                );
                assert!(libc::WIFSTOPPED(status));
            }
            if mode == "sent" || mode == "owned" {
                if mode == "owned" {
                    drop(slot.lease.lock().unwrap().take().unwrap());
                }
                let input = host.stdin.as_mut().unwrap();
                serde_json::to_writer(
                    &mut *input,
                    &json!({"protocol":1,"token":token,"grant":grant}),
                )
                .unwrap();
                input.write_all(b"\n").unwrap();
                input.flush().unwrap();
            }
            if mode == "owned" {
                let mut output = BufReader::new(host.stdout.take().unwrap());
                let mut line = String::new();
                assert!(output.read_line(&mut line).unwrap() > 0);
                let ack: Value = serde_json::from_str(&line).unwrap();
                let owned: Owned = serde_json::from_value(ack["owned"].clone()).unwrap();
                grant.verify_owned(&owned, pid, birth, None).unwrap();
            }
            println!(
                "{MARKER}{}",
                json!({"profile":slot.profile,"hostPid":pid,"mode":mode})
            );
            std::io::stdout().flush().unwrap();
            let mut byte = [0u8];
            std::io::stdin().read_exact(&mut byte).unwrap();
            std::process::exit(0); // Simulate the controlling process dying without Drop.
        }
        for mode in ["before", "sent", "owned"] {
            let root = tempfile::tempdir().unwrap();
            let mut parent=Command::new(std::env::current_exe().unwrap())
                .args(["--exact","engine::lifecycle_tests::real_parent_crash_preserves_starting_and_owned_authority","--ignored","--nocapture"])
                .env("CLOVY_CRASH_TEST_ROOT",root.path()).env("CLOVY_CRASH_TEST_MODE",mode)
                .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            let mut output = BufReader::new(parent.stdout.take().unwrap());
            let state: Value = loop {
                let mut line = String::new();
                assert!(
                    output.read_line(&mut line).unwrap() > 0,
                    "fixture exited before barrier"
                );
                if let Some((_, value)) = line.split_once(MARKER) {
                    break serde_json::from_str(value.trim()).unwrap();
                }
            };
            let host_pid = state["hostPid"].as_u64().unwrap() as i32;
            let profile = PathBuf::from(state["profile"].as_str().unwrap());
            let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, host_pid, 0) } as i32;
            assert!(
                fd >= 0,
                "pidfd_open failed: {}",
                std::io::Error::last_os_error()
            );
            let pidfd = unsafe { File::from_raw_fd(fd) };
            parent.stdin.as_mut().unwrap().write_all(b"x").unwrap();
            assert!(parent.wait().unwrap().success());
            if mode != "owned" {
                let result = unsafe {
                    libc::syscall(
                        libc::SYS_pidfd_send_signal,
                        pidfd.as_raw_fd(),
                        libc::SIGCONT,
                        0,
                        0,
                    )
                };
                if result != 0 {
                    let error = std::io::Error::last_os_error();
                    assert_eq!(
                        error.raw_os_error(),
                        Some(libc::ESRCH),
                        "pidfd SIGCONT failed: {error}"
                    );
                }
            }
            let mut pollfd = libc::pollfd {
                fd: pidfd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            assert_eq!(
                unsafe { libc::poll(&mut pollfd, 1, 5000) },
                1,
                "host did not exit"
            );
            let receipt: Value =
                serde_json::from_slice(&fs::read(profile.join(RESERVED_RECEIPT)).unwrap()).unwrap();
            assert_eq!(
                receipt["phase"],
                if mode == "owned" { "Owned" } else { "Starting" }
            );
            let lease = open_existing_plain(&profile.join(".clovy-lease"), true).unwrap();
            lock_profile(&lease).unwrap();
            drop(lease);
            let runtime = tokio::runtime::Runtime::new().unwrap();
            let _enter = runtime.enter();
            let config = EngineConfig::production(
                root.path().join("resources"),
                root.path().join("data"),
                false,
            );
            let _next = AgentBrowserTransport::new(config);
            assert!(!profile.exists(), "eligible abandoned profile was retained");
        }
    }
    #[cfg(unix)]
    #[test]
    #[ignore = "requires a separately built real private host"]
    fn real_private_host_admits_only_bound_grants() {
        use std::os::fd::AsRawFd;
        use std::os::unix::process::CommandExt;
        fn host() -> (Child, BufReader<std::process::ChildStdout>) {
            let binary = std::env::var_os("CLOVY_BROWSER_HOST_TEST_BINARY")
                .expect("real host binary required");
            let mut child = Command::new(binary)
                .env_clear()
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .process_group(0)
                .spawn()
                .unwrap();
            let output = BufReader::new(child.stdout.take().unwrap());
            (child, output)
        }
        fn send(child: &mut Child, value: Value) {
            let input = child.stdin.as_mut().unwrap();
            serde_json::to_writer(&mut *input, &value).unwrap();
            input.write_all(b"\n").unwrap();
            input.flush().unwrap();
        }
        fn line(output: &mut BufReader<std::process::ChildStdout>) -> Option<Value> {
            let mut pollfd = libc::pollfd {
                fd: output.get_ref().as_raw_fd(),
                events: libc::POLLIN | libc::POLLHUP,
                revents: 0,
            };
            assert_eq!(
                unsafe { libc::poll(&mut pollfd, 1, 5000) },
                1,
                "host did not respond or exit"
            );
            let mut text = String::new();
            if output.read_line(&mut text).unwrap() == 0 {
                None
            } else {
                Some(serde_json::from_str(&text).unwrap())
            }
        }
        fn rejected(value: Value) {
            let (mut child, mut output) = host();
            send(&mut child, value);
            assert!(
                line(&mut output).is_none(),
                "invalid grant was acknowledged"
            );
            assert!(!child.wait().unwrap().success());
        }
        let root = tempfile::tempdir().unwrap();
        let config = EngineConfig::production(
            root.path().join("resources"),
            root.path().join("data"),
            false,
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _enter = runtime.enter();
        let transport = AgentBrowserTransport::new(config);
        transport.reserve("owned-fixture").unwrap();
        let slot = transport.slot("owned-fixture").unwrap();
        test_mark_starting(&slot.profile).unwrap();
        let grant_value = Grant::new(&slot.profile, false).unwrap();
        let grant = serde_json::to_value(&grant_value).unwrap();
        let token = "a".repeat(64);
        rejected(json!({"protocol":1,"token":token}));
        for field in [
            "nonce",
            "parentBirth",
            "profile",
            "rootIdentity",
            "profileIdentity",
        ] {
            let mut invalid = grant.clone();
            invalid[field] = match field {
                "nonce" => json!(uuid::Uuid::new_v4().to_string()),
                "parentBirth" => json!(grant[field].as_u64().unwrap() ^ 1),
                "rootIdentity" | "profileIdentity" => json!([0, 0]),
                _ => json!(root.path().join("missing")),
            };
            rejected(json!({"protocol":1,"token":token,"grant":invalid}));
        }
        transport.reserve("missing-fixture").unwrap();
        let missing = transport.slot("missing-fixture").unwrap();
        test_mark_starting(&missing.profile).unwrap();
        let missing_grant =
            serde_json::to_value(Grant::new(&missing.profile, false).unwrap()).unwrap();
        fs::remove_dir_all(&missing.profile).unwrap();
        rejected(json!({"protocol":1,"token":token,"grant":missing_grant}));
        transport.terminate("missing-fixture");
        transport.reserve("link-fixture").unwrap();
        let linked = transport.slot("link-fixture").unwrap();
        test_mark_starting(&linked.profile).unwrap();
        let linked_grant =
            serde_json::to_value(Grant::new(&linked.profile, false).unwrap()).unwrap();
        let moved = linked.profile.with_extension("moved");
        fs::rename(&linked.profile, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &linked.profile).unwrap();
        rejected(json!({"protocol":1,"token":token,"grant":linked_grant}));
        fs::remove_file(&linked.profile).unwrap();
        fs::rename(&moved, &linked.profile).unwrap();
        transport.terminate("link-fixture");
        // While root owns the kernel lease, a valid grant must not be acknowledged.
        rejected(json!({"protocol":1,"token":token,"grant":grant}));
        let (mut child, mut output) = host();
        let host_pid = child.id();
        let host_birth = clovy_browser_process_identity::process_birth(host_pid).unwrap();
        drop(slot.lease.lock().unwrap().take().unwrap());
        send(
            &mut child,
            json!({"protocol":1,"token":token,"grant":grant}),
        );
        let ack = line(&mut output).unwrap();
        assert_eq!(ack["protocol"], 1);
        assert_eq!(ack["token"], token);
        let owned: Owned = serde_json::from_value(ack["owned"].clone()).unwrap();
        verify_host_ready(
            &ack,
            &token,
            &grant_value,
            false,
            host_pid,
            host_birth,
            None,
        )
        .unwrap();
        assert!(verify_host_ready(
            &json!({"protocol":1,"token":token}),
            &token,
            &grant_value,
            false,
            host_pid,
            host_birth,
            None
        )
        .is_err());
        for field in [
            "hostPid",
            "hostBirth",
            "nonce",
            "group",
            "profileIdentity",
            "boot",
        ] {
            let mut altered = ack.clone();
            altered["owned"][field] = match field {
                "hostPid" => json!(host_pid ^ 1),
                "hostBirth" => json!(host_birth ^ 1),
                "group" => json!(host_pid ^ 1),
                "profileIdentity" => json!([0, 0]),
                _ => json!("invalid"),
            };
            assert!(
                verify_host_ready(
                    &altered,
                    &token,
                    &grant_value,
                    false,
                    host_pid,
                    host_birth,
                    None
                )
                .is_err(),
                "altered {field} was accepted"
            );
        }
        grant_value
            .verify_owned(&owned, host_pid, host_birth, None)
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(
                &fs::read(slot.profile.join(RESERVED_RECEIPT)).unwrap()
            )
            .unwrap()["phase"],
            "Owned"
        );
        let mut wrong_pid = owned.clone();
        wrong_pid.host_pid ^= 1;
        assert!(grant_value
            .verify_owned(&wrong_pid, host_pid, host_birth, None)
            .is_err());
        let mut wrong_birth = owned.clone();
        wrong_birth.host_birth ^= 1;
        assert!(grant_value
            .verify_owned(&wrong_birth, host_pid, host_birth, None)
            .is_err());
        let competing = open_existing_plain(&slot.profile.join(".clovy-lease"), true).unwrap();
        assert!(
            lock_profile(&competing).is_err(),
            "host must hold the lease after the root releases it"
        );
        for (id, profile, persistent) in [
            (1, root.path().join("other"), false),
            (2, slot.profile.clone(), true),
            (
                5,
                PathBuf::from(format!("{}/.", slot.profile.display())),
                false,
            ),
        ] {
            send(
                &mut child,
                json!({"id":id,"action":"start","chromePath":root.path().join("absent-chrome"),"profilePath":profile,"proxyPort":1,"persistent":persistent,"headed":false}),
            );
            let result = line(&mut output).unwrap();
            assert_eq!(result["id"], id);
            assert_eq!(result["delivery"], "not_attempted");
            assert!(result.get("error").is_some());
        }
        send(
            &mut child,
            json!({"id":3,"action":"start","chromePath":root.path().join("absent-chrome"),"profilePath":slot.profile,"proxyPort":1,"persistent":false,"headed":false}),
        );
        let matching = line(&mut output).unwrap();
        assert_eq!(matching["id"], 3);
        assert_eq!(matching["delivery"], "unknown");
        send(&mut child, json!({"id":4,"action":"shutdown"}));
        assert_eq!(line(&mut output).unwrap()["id"], 4);
        assert!(child.wait().unwrap().success());
        lock_profile(&competing).expect("lease remains held after host exit");
        drop(competing);
        transport.terminate("owned-fixture");
        transport.reserve("eof-fixture").unwrap();
        let eof_slot = transport.slot("eof-fixture").unwrap();
        test_mark_starting(&eof_slot.profile).unwrap();
        let eof_grant = Grant::new(&eof_slot.profile, false).unwrap();
        let (mut eof_child, mut eof_output) = host();
        let eof_pid = eof_child.id();
        let eof_birth = clovy_browser_process_identity::process_birth(eof_pid).unwrap();
        drop(eof_slot.lease.lock().unwrap().take().unwrap());
        send(
            &mut eof_child,
            json!({"protocol":1,"token":token,"grant":eof_grant}),
        );
        let eof_ack = line(&mut eof_output).unwrap();
        let eof_owned: Owned = serde_json::from_value(eof_ack["owned"].clone()).unwrap();
        eof_grant
            .verify_owned(&eof_owned, eof_pid, eof_birth, None)
            .unwrap();
        let eof_competing =
            open_existing_plain(&eof_slot.profile.join(".clovy-lease"), true).unwrap();
        assert!(lock_profile(&eof_competing).is_err());
        // A real, inert member of the host's group keeps the host's lease
        // alive after EOF, even after the direct browser process has closed.
        let mut member = Command::new("/bin/sleep")
            .arg("30")
            .process_group(eof_pid as i32)
            .spawn()
            .unwrap();
        drop(eof_child.stdin.take());
        std::thread::sleep(Duration::from_millis(150));
        assert!(eof_child.try_wait().unwrap().is_none());
        assert!(lock_profile(&eof_competing).is_err());
        member.kill().unwrap();
        member.wait().unwrap();
        assert!(eof_child.wait().unwrap().success());
        lock_profile(&eof_competing).expect("lease remains held after host EOF");
        drop(eof_competing);
        transport.shutdown();
        let persistent_config = EngineConfig::production(
            root.path().join("resources"),
            root.path().join("persistent-data"),
            true,
        );
        let persistent_transport = AgentBrowserTransport::new(persistent_config.clone());
        persistent_transport.reserve("persistent").unwrap();
        fs::write(
            persistent_config.persistent_profile.join("sentinel"),
            b"synthetic",
        )
        .unwrap();
        let persistent_grant = Grant::new(&persistent_config.persistent_profile, true).unwrap();
        let mut former_parent = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let former_pid = former_parent.id();
        let former_birth = clovy_browser_process_identity::process_birth(former_pid).unwrap();
        former_parent.kill().unwrap();
        former_parent.wait().unwrap();
        let mut expired = serde_json::to_value(&persistent_grant).unwrap();
        expired["parentPid"] = json!(former_pid);
        expired["parentBirth"] = json!(former_birth);
        rejected(json!({"protocol":1,"token":token,"grant":expired}));
        let (mut child, mut output) = host();
        send(
            &mut child,
            json!({"protocol":1,"token":token,"grant":persistent_grant}),
        );
        assert_eq!(
            line(&mut output).unwrap(),
            json!({"protocol":1,"token":token})
        );
        send(
            &mut child,
            json!({"id":1,"action":"start","chromePath":root.path().join("absent-chrome"),"profilePath":persistent_config.persistent_profile,"proxyPort":1,"persistent":false,"headed":false}),
        );
        assert_eq!(line(&mut output).unwrap()["delivery"], "not_attempted");
        send(
            &mut child,
            json!({"id":2,"action":"start","chromePath":root.path().join("absent-chrome"),"profilePath":persistent_config.persistent_profile,"proxyPort":1,"persistent":true,"headed":false}),
        );
        assert_eq!(line(&mut output).unwrap()["delivery"], "unknown");
        send(&mut child, json!({"id":3,"action":"shutdown"}));
        assert_eq!(line(&mut output).unwrap()["id"], 3);
        assert!(child.wait().unwrap().success());
        persistent_transport.shutdown();
        assert_eq!(
            fs::read(persistent_config.persistent_profile.join("sentinel")).unwrap(),
            b"synthetic"
        );
        let original = root.path().join("login-target");
        let replacement = root.path().join("different-login");
        fs::create_dir(&original).unwrap();
        fs::create_dir(&replacement).unwrap();
        let link = root.path().join("login-link");
        std::os::unix::fs::symlink(&original, &link).unwrap();
        let mut linked_config = persistent_config.clone();
        linked_config.persistent_profile = link.clone();
        let linked_transport = AgentBrowserTransport::new(linked_config);
        linked_transport.reserve("linked").unwrap();
        let linked_grant = Grant::new(&link, true).unwrap();
        let (mut child, mut output) = host();
        send(
            &mut child,
            json!({"protocol":1,"token":token,"grant":linked_grant}),
        );
        assert_eq!(
            line(&mut output).unwrap(),
            json!({"protocol":1,"token":token})
        );
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&replacement, &link).unwrap();
        send(
            &mut child,
            json!({"id":1,"action":"start","chromePath":root.path().join("absent-chrome"),"profilePath":link,"proxyPort":1,"persistent":true,"headed":false}),
        );
        assert_eq!(line(&mut output).unwrap()["delivery"], "not_attempted");
        send(&mut child, json!({"id":2,"action":"shutdown"}));
        assert_eq!(line(&mut output).unwrap()["id"], 2);
        assert!(child.wait().unwrap().success());
        linked_transport.shutdown();
    }
    #[tokio::test]
    async fn starting_refuses_a_different_parent_generation() {
        let root = physical_tempdir();
        let config = EngineConfig::production(
            root.path().join("resources"),
            root.path().join("data"),
            false,
        );
        let transport = AgentBrowserTransport::new(config);
        transport.reserve("generation-test").unwrap();
        let profile = transport.slot("generation-test").unwrap().profile.clone();
        let receipt_path = profile.join(RESERVED_RECEIPT);
        let mut receipt: Value = serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        let birth = receipt["ownerBirth"].as_u64().unwrap();
        receipt["ownerBirth"] = json!(birth ^ 1);
        fs::write(&receipt_path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert!(test_mark_starting(&profile).is_err());
        assert!(marker_absent(&profile));
        transport.shutdown();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn reserved_recovery_preserves_live_busy_starting_legacy_and_links() {
        use std::os::unix::fs::symlink;
        let root = physical_tempdir();
        let config = EngineConfig::production(
            root.path().join("unicode-á-😀/resources"),
            root.path().join("unicode-á-😀/data"),
            false,
        );
        fs::create_dir_all(&config.temporary_root).unwrap();
        fs::create_dir_all(&config.persistent_profile).unwrap();
        fs::write(config.persistent_profile.join("sentinel"), b"persistent").unwrap();
        let external = root.path().join("external");
        fs::create_dir(&external).unwrap();
        fs::write(external.join("sentinel"), b"external").unwrap();
        let link = config.temporary_root.join(uuid::Uuid::new_v4().to_string());
        symlink(&external, &link).unwrap();

        let mut dead = Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .unwrap();
        let dead_pid = dead.id();
        assert!(dead.wait().unwrap().success());
        assert!(process_definitely_absent(dead_pid));
        let make = |dead_owner: bool| {
            let profile = config.temporary_root.join(uuid::Uuid::new_v4().to_string());
            fs::create_dir(&profile).unwrap();
            File::create(profile.join(".clovy-lease")).unwrap();
            publish_reserved(
                &test_profile(&profile),
                profile.file_name().unwrap().to_str().unwrap(),
            )
            .unwrap();
            if dead_owner {
                let path = profile.join(RESERVED_RECEIPT);
                let mut receipt: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                receipt["ownerProcessId"] = json!(dead_pid);
                fs::write(path, serde_json::to_vec(&receipt).unwrap()).unwrap();
            }
            profile
        };
        let eligible = make(true);
        let live = make(false);
        let busy = make(true);
        let busy_lease = open_existing_plain(&busy.join(".clovy-lease"), true).unwrap();
        lock_profile(&busy_lease).unwrap();
        let starting = make(false);
        test_mark_starting(&starting).unwrap();
        let starting_receipt = starting.join(RESERVED_RECEIPT);
        let mut starting_value: Value =
            serde_json::from_slice(&fs::read(&starting_receipt).unwrap()).unwrap();
        assert_eq!(starting_value["phase"], "Starting");
        starting_value["ownerProcessId"] = json!(dead_pid);
        fs::write(
            starting_receipt,
            serde_json::to_vec(&starting_value).unwrap(),
        )
        .unwrap();
        let corrupt = make(true);
        fs::write(corrupt.join(RESERVED_RECEIPT), b"not-json").unwrap();
        let linked_receipt = make(true);
        fs::remove_file(linked_receipt.join(RESERVED_RECEIPT)).unwrap();
        symlink(
            external.join("sentinel"),
            linked_receipt.join(RESERVED_RECEIPT),
        )
        .unwrap();
        let linked_lease = make(true);
        fs::remove_file(linked_lease.join(".clovy-lease")).unwrap();
        symlink(external.join("sentinel"), linked_lease.join(".clovy-lease")).unwrap();
        let legacy = config.temporary_root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir(&legacy).unwrap();
        fs::write(legacy.join("sentinel"), b"legacy").unwrap();
        let moved = make(true);
        let alternate_root = root.path().join("alternate-root");
        fs::create_dir(&alternate_root).unwrap();
        let moved_destination = alternate_root.join(moved.file_name().unwrap());
        fs::rename(moved, &moved_destination).unwrap();
        let mut alternate_config = config.clone();
        alternate_config.temporary_root = alternate_root;
        let _alternate = AgentBrowserTransport::new(alternate_config);
        assert!(moved_destination.is_dir());

        let _transport = AgentBrowserTransport::new(config.clone());
        assert!(!eligible.exists());
        assert!(!starting.exists());
        for profile in [
            &live,
            &busy,
            &corrupt,
            &legacy,
            &linked_receipt,
            &linked_lease,
        ] {
            assert!(profile.is_dir());
        }
        assert_eq!(fs::read(external.join("sentinel")).unwrap(), b"external");
        assert_eq!(
            fs::read(config.persistent_profile.join("sentinel")).unwrap(),
            b"persistent"
        );
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
        drop(busy_lease);
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn process_group_is_reaped_before_releasing_its_authority() {
        use std::os::unix::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("child-pid");
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "sleep 30 & printf '%s' \"$!\" > \"$1\"; wait",
                "fixture",
            ])
            .arg(&marker)
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = command.spawn().unwrap();
        let mut process = Process { child };
        for _ in 0..100 {
            if marker.is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(marker.is_file());
        assert!(!process.tree_exited());
        process.stop(Duration::ZERO);
        for _ in 0..100 {
            if process.tree_exited() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("A live descendant survived the supervised group termination");
    }
    #[tokio::test]
    async fn persistent_profile_has_one_lease_across_transport_instances() {
        let root = tempfile::tempdir().unwrap();
        let config = EngineConfig::production(
            root.path().join("resources"),
            root.path().join("data"),
            true,
        );
        let first = AgentBrowserTransport::new(config.clone());
        let second = AgentBrowserTransport::new(config.clone());
        first.reserve("conversation-a").unwrap();
        assert_eq!(
            second.reserve("conversation-b").unwrap_err().code,
            "browser_profile_busy"
        );
        fs::write(
            config.persistent_profile.join("synthetic-login-state"),
            b"retained",
        )
        .unwrap();
        first.terminate("conversation-a");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match second.reserve("new-run") {
                Ok(()) => break,
                Err(error) if error.code == "browser_profile_busy" && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("Profile lease did not become available: {error:?}"),
            }
        }
        assert_eq!(
            fs::read(config.persistent_profile.join("synthetic-login-state")).unwrap(),
            b"retained"
        );
        second.shutdown();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn profile_lease_stays_busy_until_the_owned_group_exits() {
        use std::os::unix::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let config = EngineConfig::production(
            root.path().join("resources"),
            root.path().join("data"),
            true,
        );
        let first = AgentBrowserTransport::new(config.clone());
        let second = AgentBrowserTransport::new(config);
        first.reserve("owner").unwrap();
        let marker = root.path().join("descendant");
        let child = Command::new("/bin/sh")
            .args([
                "-c",
                "sleep 30 & printf '%s' \"$!\" > \"$1\"; wait",
                "fixture",
            ])
            .arg(&marker)
            .process_group(0)
            .spawn()
            .unwrap();
        let group = child.id() as i32;
        *first.slot("owner").unwrap().process.lock().unwrap() = Some(Process { child });
        for _ in 0..100 {
            if marker.is_file() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(marker.is_file());
        assert!(!unix_group_exited(group));
        assert_eq!(
            second.reserve("next").unwrap_err().code,
            "browser_profile_busy"
        );
        first.terminate_with_grace("owner", Duration::ZERO);
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match second.reserve("next") {
                Ok(()) => break,
                Err(error) if error.code == "browser_profile_busy" && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("Profile was not released after Stop: {error:?}"),
            }
        }
        assert!(unix_group_exited(group), "lease released before group exit");
        second.shutdown();
    }
    #[tokio::test]
    async fn anonymous_profiles_are_capped_reaped_and_deleted_without_touching_login_profile() {
        let root = physical_tempdir();
        let config = EngineConfig::production(
            root.path().join("resources"),
            root.path().join("data"),
            false,
        );
        fs::create_dir_all(&config.persistent_profile).unwrap();
        fs::write(
            config.persistent_profile.join("synthetic-login"),
            b"preserve",
        )
        .unwrap();
        let transport = AgentBrowserTransport::new(config.clone());
        transport.reserve("one").unwrap();
        transport.reserve("two").unwrap();
        assert_eq!(
            transport.reserve("three").unwrap_err().code,
            "browser_session_limit"
        );
        let slot = transport.slot("one").unwrap();
        let profile = slot.profile.clone();
        *slot.last_used.lock().unwrap() = Instant::now() - Duration::from_secs(301);
        transport.reap_idle();
        assert!(!profile.exists());
        assert!(config.persistent_profile.join("synthetic-login").exists());
        transport.reserve("three").unwrap();
        transport.shutdown();
        assert_eq!(fs::read_dir(&config.temporary_root).unwrap().count(), 0);
    }
}
