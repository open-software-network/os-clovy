//! Private, task-owned execution of the pinned agent-desktop engine.
//!
//! No upstream socket is created. Cancellation never waits for the request
//! mutex: the owner kills its entire controller group/job before invalidating
//! the run. Real applications are launched through the OS application broker.
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{io::AsyncReadExt, process::Command};

#[cfg(unix)]
use tokio::process::Child;
#[cfg(target_os = "macos")]
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
};

const MAX_RESPONSE: usize = 24 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(40);

#[derive(Clone)]
pub struct HostCancel(Arc<CancelState>);
struct CancelState {
    cancelled: AtomicBool,
    spawn_gate: std::sync::Mutex<()>,
    run_root: PathBuf,
    #[cfg(unix)]
    group: i32,
    #[cfg(target_os = "windows")]
    job: crate::computer_use_windows::OwnedJob,
}
impl HostCancel {
    pub fn cancel(&self) {
        let _gate = self
            .0
            .spawn_gate
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.0.cancelled.swap(true, Ordering::AcqRel) {
            crate::computer_use_cancel::cooperative_cancel(&self.0.run_root);
            #[cfg(unix)]
            unsafe {
                libc::kill(-self.0.group, libc::SIGKILL);
            }
            #[cfg(target_os = "windows")]
            self.0.job.terminate();
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
}

pub struct CommandHost {
    #[cfg(not(target_os = "macos"))]
    browser_runtime: Arc<crate::computer_use_owned_runtime::OwnedBrowserRuntime>,
    #[cfg(not(target_os = "macos"))]
    executable: PathBuf,
    home: PathBuf,
    cancel: HostCancel,
    #[cfg(not(target_os = "windows"))]
    child: Child,
    #[cfg(target_os = "macos")]
    stdin: ChildStdin,
    #[cfg(target_os = "macos")]
    stdout: BufReader<ChildStdout>,
}
impl CommandHost {
    pub async fn start(
        executable: PathBuf,
        home: PathBuf,
        browser_runtime: Arc<crate::computer_use_owned_runtime::OwnedBrowserRuntime>,
    ) -> Result<Self> {
        let (namespace, image) = browser_runtime.paths();
        if !executable.is_absolute()
            || !executable.is_file()
            || !home.is_absolute()
            || !namespace.is_absolute()
            || !image.is_absolute()
        {
            bail!("Invalid private engine paths");
        }
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        verify_engine(&executable)?;
        // The caller supplies a fresh random run/epoch namespace. Refuse a
        // reused directory, including a pre-created symlink.
        std::fs::create_dir(&home).context("Create fresh engine namespace")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700))?;
        }
        #[cfg(target_os = "windows")]
        {
            crate::computer_use_windows::verify_same_signer(
                &std::env::current_exe()?,
                &executable,
            )?;
            let cancel = HostCancel(Arc::new(CancelState {
                cancelled: AtomicBool::new(false),
                spawn_gate: std::sync::Mutex::new(()),
                run_root: home
                    .parent()
                    .context("Missing run namespace")?
                    .to_path_buf(),
                job: crate::computer_use_windows::OwnedJob::new()?,
            }));
            Ok(Self {
                browser_runtime,
                executable,
                home,
                cancel,
            })
        }
        #[cfg(target_os = "macos")]
        {
            let group = unsafe { libc::getpgrp() };
            if group != unsafe { libc::getpid() } {
                bail!("Helper must own its isolated process group");
            }
            let mut command = Command::new(&executable);
            configure(&mut command, &home, &browser_runtime);
            command.env("AGENT_DESKTOP_INTERNAL_COMMAND_HOST", "1");
            let mut child = command
                .spawn()
                .context("Start private engine command host")?;
            if !crate::computer_use_macos::verify_engine_process(
                child.id().context("Engine process identity")?,
            ) {
                // The helper owns this group from startup. Abort before even
                // one typed command-host frame can reach a swapped image.
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
                bail!("The mapped Computer use engine does not match the signed helper");
            }
            if let Err(error) = verify_engine(&executable) {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
                return Err(error);
            }
            let stdin = child.stdin.take().context("Engine input pipe")?;
            let stdout = BufReader::new(child.stdout.take().context("Engine output pipe")?);
            let cancel = HostCancel(Arc::new(CancelState {
                cancelled: AtomicBool::new(false),
                spawn_gate: std::sync::Mutex::new(()),
                run_root: home
                    .parent()
                    .context("Missing run namespace")?
                    .to_path_buf(),
                group,
            }));
            Ok(Self {
                home,
                cancel,
                child,
                stdin,
                stdout,
            })
        }
        #[cfg(target_os = "linux")]
        {
            // Linux is only an executable verification surface for this module,
            // not a supported Clovy platform. A keepalive owns the controller
            // group across CLI invocations. The spawn gate serializes each
            // synchronous spawn with Stop so no new member can miss the kill.
            let mut keeper = Command::new("/bin/sleep");
            configure(&mut keeper, &home, &browser_runtime);
            keeper
                .arg("31536000")
                .process_group(0)
                .kill_on_drop(true)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            let child = keeper.spawn()?;
            let group = i32::try_from(child.id().context("Supervisor PID")?)?;
            let cancel = HostCancel(Arc::new(CancelState {
                cancelled: AtomicBool::new(false),
                spawn_gate: std::sync::Mutex::new(()),
                run_root: home
                    .parent()
                    .context("Missing run namespace")?
                    .to_path_buf(),
                group,
            }));
            Ok(Self {
                browser_runtime,
                executable,
                home,
                cancel,
                child,
            })
        }
    }
    pub fn cancel_handle(&self) -> HostCancel {
        self.cancel.clone()
    }
    #[cfg(target_os = "windows")]
    pub(crate) fn browser_runtime(
        &self,
    ) -> &Arc<crate::computer_use_owned_runtime::OwnedBrowserRuntime> {
        &self.browser_runtime
    }
    pub async fn execute(&mut self, argv: Vec<String>) -> Result<Value> {
        if self.cancel.is_cancelled() {
            bail!("Computer use run was stopped");
        }
        // Only the adapter constructs argv. Enforce frame and namespace bounds
        // here as a second boundary. Flag-like user text remains literal argv;
        // only the adapter chooses the command and global options.
        if argv.iter().any(|arg| arg.as_bytes().contains(&0)) {
            bail!("Invalid private engine argument");
        }
        let request_bytes = serde_json::to_vec(&argv)?;
        if request_bytes.len() > 256 * 1024 {
            bail!("Engine request exceeds limit");
        }
        struct InvocationGuard {
            cancel: HostCancel,
            armed: bool,
        }
        impl Drop for InvocationGuard {
            fn drop(&mut self) {
                if self.armed {
                    self.cancel.cancel();
                }
            }
        }
        let mut invocation = InvocationGuard {
            cancel: self.cancel.clone(),
            armed: true,
        };
        #[cfg(target_os = "macos")]
        let result = tokio::time::timeout(REQUEST_TIMEOUT, async {
            self.stdin.write_all(&request_bytes).await?;
            self.stdin.write_all(b"\n").await?;
            self.stdin.flush().await?;
            let mut bytes = Vec::new();
            let count = (&mut self.stdout)
                .take((MAX_RESPONSE + 1) as u64)
                .read_until(b'\n', &mut bytes)
                .await?;
            if count == 0 || count > MAX_RESPONSE || bytes.last() != Some(&b'\n') {
                bail!("Engine command host closed or exceeded its response budget");
            }
            Ok::<Value, anyhow::Error>(serde_json::from_slice(&bytes)?)
        })
        .await;
        #[cfg(target_os = "windows")]
        let result = tokio::time::timeout(REQUEST_TIMEOUT, async {
            let mut command = Command::new(&self.executable);
            configure(&mut command, &self.home, &self.browser_runtime);
            command.args(&argv);
            let mut child = {
                let _gate = self
                    .cancel
                    .0
                    .spawn_gate
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if self.cancel.is_cancelled() {
                    bail!("Computer use run was stopped");
                }
                let _image_lock = crate::computer_use_windows::lock_image(
                    &self.executable,
                    option_env!("CLOVY_AGENT_DESKTOP_EXECUTABLE_SHA256"),
                )?;
                verify_engine(&self.executable)?;
                crate::computer_use_windows::verify_same_signer(
                    &std::env::current_exe()?,
                    &self.executable,
                )?;
                crate::computer_use_windows::spawn_in_job(command, &self.cancel.0.job)?
            };
            let stdout = child.stdout.take().context("Engine output pipe")?;
            let mut bytes = Vec::new();
            stdout
                .take((MAX_RESPONSE + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            let status = child.wait().await?;
            if bytes.len() > MAX_RESPONSE {
                bail!("Engine response exceeds limit");
            }
            let response: Value =
                serde_json::from_slice(&bytes).context("Engine returned invalid JSON")?;
            if !status.success() && response.get("error").is_none() {
                bail!("Engine exited without a delivery receipt");
            }
            Ok::<Value, anyhow::Error>(response)
        })
        .await;
        #[cfg(target_os = "linux")]
        let result = tokio::time::timeout(REQUEST_TIMEOUT, async {
            let mut command = Command::new(&self.executable);
            configure(&mut command, &self.home, &self.browser_runtime);
            command.args(&argv).process_group(self.cancel.0.group);
            let mut child = {
                let _gate = self
                    .cancel
                    .0
                    .spawn_gate
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if self.cancel.is_cancelled() {
                    bail!("Computer use run was stopped");
                }
                command
                    .spawn()
                    .context("Start group-owned Linux proof invocation")?
            };
            let stdout = child.stdout.take().context("Engine output pipe")?;
            let mut bytes = Vec::new();
            stdout
                .take((MAX_RESPONSE + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            let status = child.wait().await?;
            if bytes.len() > MAX_RESPONSE {
                bail!("Engine response exceeds limit");
            }
            let response: Value =
                serde_json::from_slice(&bytes).context("Engine returned invalid JSON")?;
            if !status.success() && response.get("error").is_none() {
                bail!("Engine exited without a delivery receipt");
            }
            Ok::<Value, anyhow::Error>(response)
        })
        .await;
        match result {
            Ok(Ok(value)) if !self.cancel.is_cancelled() => {
                invocation.armed = false;
                Ok(value)
            }
            Ok(Err(error)) => {
                self.cancel.cancel();
                Err(error)
            }
            Ok(Ok(_)) => bail!("Computer use run was stopped"),
            Err(_) => {
                self.cancel.cancel();
                bail!("Computer use timed out; delivery may be uncertain");
            }
        }
    }
}
fn configure(
    command: &mut Command,
    home: &std::path::Path,
    browser_runtime: &crate::computer_use_owned_runtime::OwnedBrowserRuntime,
) {
    for (name, _) in std::env::vars_os() {
        let normalized = name.to_string_lossy().to_ascii_uppercase();
        if normalized.starts_with("AGENT_DESKTOP_")
            || normalized.starts_with("CLOVY_AGENT_DESKTOP_")
            || normalized.starts_with("CLOVY_COMPUTER_USE_")
            || normalized.starts_with("DYLD_")
            || normalized == "LD_PRELOAD"
            || normalized == "LD_LIBRARY_PATH"
        {
            command.env_remove(name);
        }
    }
    let (namespace, image) = browser_runtime.paths();
    command
        .env("CLOVY_COMPUTER_USE_BROWSER_NAMESPACE", namespace)
        .env("CLOVY_COMPUTER_USE_BROWSER_IMAGE", image)
        .env("AGENT_DESKTOP_HOME", home)
        .env("AGENT_DESKTOP_SESSION", "clovy")
        .env("CLOVY_AGENT_DESKTOP_SUPERVISED", "1")
        .env(
            "CLOVY_AGENT_DESKTOP_CANCEL_FILE",
            home.parent().expect("private run directory").join("cancel"),
        )
        .env(
            "CLOVY_AGENT_DESKTOP_CANCEL_ACK_FILE",
            home.parent()
                .expect("private run directory")
                .join("cancel-ack"),
        )
        .env_remove("AGENT_DESKTOP_INTERNAL_HOST_SOCKET")
        .env_remove("AGENT_DESKTOP_INTERNAL_COMMAND_HOST")
        .env_remove("CLOVY_COMPUTER_USE_HELPER_CAPABILITY")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(false);
}
/// Verify the packaged provenance and actual executable bytes before startup.
/// The signed helper bundle/Authenticode peer validation anchors the manifest;
/// the digest also refuses accidental mismatched builds or stale resources.
pub fn verify_engine(executable: &std::path::Path) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let pin: Value = serde_json::from_str(include_str!("../agent-desktop-pin.json"))?;
    let directory = executable.parent().context("Missing engine directory")?;
    let manifest_path = if cfg!(target_os = "macos") {
        directory
            .parent()
            .context("Missing helper Contents")?
            .join("Resources/agent-desktop-manifest.json")
    } else {
        directory.join("agent-desktop-manifest.json")
    };
    let metadata = std::fs::metadata(&manifest_path)?;
    if metadata.len() > 64 * 1024 {
        bail!("Engine manifest exceeds limit");
    }
    let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    for field in [
        "source",
        "sourceCommit",
        "version",
        "protocolVersion",
        "cargoLockSha256",
        "license",
        "licenseSha256",
        "patchSha256",
        "ownedRuntimeSourceSha256",
        "sourceReviewSha256",
    ] {
        let expected = pin[field]
            .as_str()
            .filter(|value| !value.is_empty())
            .context("Engine pin field missing or empty")?;
        if manifest[field].as_str() != Some(expected) {
            bail!("Engine manifest {field} does not match the compiled pin");
        }
    }
    let name = if cfg!(target_os = "windows") {
        "agent-desktop.exe"
    } else {
        "agent-desktop"
    };
    if executable.file_name() != Some(std::ffi::OsStr::new(name)) || manifest["executable"] != name
    {
        bail!("Engine executable identity mismatch");
    }
    let target = manifest["target"]
        .as_str()
        .context("Engine target missing")?;
    let valid_target = if cfg!(target_os = "macos") {
        target == "universal-apple-darwin"
            || target == format!("{}-apple-darwin", std::env::consts::ARCH)
    } else if cfg!(target_os = "windows") {
        target == format!("{}-pc-windows-msvc", std::env::consts::ARCH)
    } else {
        target == format!("{}-unknown-linux-gnu", std::env::consts::ARCH)
    };
    if !valid_target {
        bail!("Engine target does not match this helper");
    }
    let mut file = std::fs::File::open(executable)?;
    let mut hash = Sha256::new();
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    let actual_digest = format!("{:x}", hash.finalize());
    if manifest["executableSha256"].as_str() != Some(actual_digest.as_str()) {
        bail!("Engine executable digest mismatch");
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let compiled_digest = option_env!("CLOVY_AGENT_DESKTOP_EXECUTABLE_SHA256")
            .context("The helper has no compiled engine digest")?;
        if actual_digest != compiled_digest {
            bail!("Engine bytes do not match the signed helper's compiled digest");
        }
    }
    #[cfg(target_os = "macos")]
    if crate::computer_use_macos::compiled_engine_cdhash().is_none() {
        bail!("The helper has no compiled engine mapped-image identity");
    }
    Ok(())
}
impl Drop for CommandHost {
    fn drop(&mut self) {
        self.cancel.cancel();
        #[cfg(not(target_os = "windows"))]
        {
            let _ = self.child.start_kill();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn fixture() -> (
        PathBuf,
        PathBuf,
        PathBuf,
        Arc<crate::computer_use_owned_runtime::OwnedBrowserRuntime>,
    ) {
        let root = std::env::temp_dir().join(format!(
            "clovy-supervisor-proof-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let namespace = root.join("chrome");
        std::fs::create_dir(&namespace).unwrap();
        let image = namespace.join("browser");
        std::fs::write(&image, b"browser fixture").unwrap();
        let browser_runtime = Arc::new(
            crate::computer_use_owned_runtime::OwnedBrowserRuntime::new(namespace, image),
        );
        let engine = root.join("blocking-controller");
        // A real controller fixture starts a descendant, then blocks while an
        // invocation is in flight. It deliberately never fabricates receipts.
        std::fs::write(&engine,"#!/bin/sh\nsleep 120 &\nprintf '%s' \"$!\" > \"$AGENT_DESKTOP_HOME/descendant.pid\"\nread -r request\nwait\n").unwrap();
        std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o700)).unwrap();
        (engine, root.join("fresh-owner"), root, browser_runtime)
    }
    fn running(pid: i32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        stat.rsplit_once(") ")
            .is_some_and(|(_, tail)| !tail.starts_with('Z'))
    }
    #[tokio::test]
    async fn cancellation_reaches_descendants_while_execute_is_blocked() {
        let (engine, home, root, browser_runtime) = fixture();
        let mut host = CommandHost::start(engine, home.clone(), browser_runtime)
            .await
            .unwrap();
        let cancel = host.cancel_handle();
        let blocked = tokio::spawn(async move { host.execute(vec!["snapshot".into()]).await });
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(pid) = std::fs::read_to_string(home.join("descendant.pid")) {
                    if let Ok(pid) = pid.trim().parse::<i32>() {
                        break pid;
                    }
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(running(pid));
        cancel.cancel();
        assert!(tokio::time::timeout(Duration::from_secs(2), blocked)
            .await
            .unwrap()
            .unwrap()
            .is_err());
        tokio::time::timeout(Duration::from_secs(2), async {
            while running(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        cancel.cancel(); // A cloned/stale Stop is idempotent.
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn existing_namespace_is_rejected_before_controller_start() {
        let (engine, home, root, browser_runtime) = fixture();
        std::fs::create_dir(&home).unwrap();
        assert!(CommandHost::start(engine, home.clone(), browser_runtime)
            .await
            .is_err());
        assert!(!home.join("descendant.pid").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn dropping_an_inflight_invocation_cancels_its_controllers() {
        let (engine, home, root, browser_runtime) = fixture();
        let mut host = CommandHost::start(engine, home.clone(), browser_runtime)
            .await
            .unwrap();
        let mut invocation = Box::pin(host.execute(vec!["snapshot".into()]));
        let pid = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                tokio::select! {
                    _=&mut invocation=>panic!("controller fixture unexpectedly completed"),
                    _=tokio::time::sleep(Duration::from_millis(5))=>{
                        if let Ok(value)=std::fs::read_to_string(home.join("descendant.pid")) {
                            if let Ok(pid)=value.trim().parse::<i32>(){break pid;}
                        }
                    }
                }
            }
        })
        .await
        .unwrap();
        assert!(running(pid));
        drop(invocation);
        assert!(host.cancel_handle().is_cancelled());
        tokio::time::timeout(Duration::from_secs(2), async {
            while running(pid) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(host.execute(vec!["snapshot".into()]).await.is_err());
        drop(host);
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(all(test, target_os = "linux"))]
mod artifact_tests {
    use super::*;
    use serde_json::json;
    use sha2::{Digest, Sha256};
    #[test]
    fn executable_bytes_and_pinned_provenance_are_both_required() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("agent-desktop");
        std::fs::write(&executable, b"synthetic artifact bytes").unwrap();
        let mut manifest: Value =
            serde_json::from_str(include_str!("../agent-desktop-pin.json")).unwrap();
        manifest["target"] = json!(format!("{}-unknown-linux-gnu", std::env::consts::ARCH));
        manifest["executable"] = json!("agent-desktop");
        manifest["executableSha256"] =
            json!(format!("{:x}", Sha256::digest(b"synthetic artifact bytes")));
        let path = directory.path().join("agent-desktop-manifest.json");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        verify_engine(&executable).unwrap();
        let runtime_source = manifest["ownedRuntimeSourceSha256"].clone();
        manifest
            .as_object_mut()
            .unwrap()
            .remove("ownedRuntimeSourceSha256");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(verify_engine(&executable).is_err());
        manifest["ownedRuntimeSourceSha256"] = json!("different-generation");
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(verify_engine(&executable).is_err());
        manifest["ownedRuntimeSourceSha256"] = runtime_source;
        std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        std::fs::write(&executable, b"tampered artifact bytes").unwrap();
        assert!(verify_engine(&executable)
            .unwrap_err()
            .to_string()
            .contains("digest"));
        std::fs::write(&executable, b"synthetic artifact bytes").unwrap();
        manifest["sourceCommit"] = json!("different-source");
        std::fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(verify_engine(&executable)
            .unwrap_err()
            .to_string()
            .contains("sourceCommit"));
    }
}
