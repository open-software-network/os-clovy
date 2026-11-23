//! Authenticated private desktop helper. The only automation surface is the
//! fourteen Clovy operations below. The upstream CLI/daemon is never exposed.
#[path = "app_identity.rs"]
#[allow(dead_code)]
mod app_identity;
mod computer_use_cancel;
mod computer_use_engine;
mod computer_use_host;
#[cfg(target_os = "macos")]
#[allow(dead_code)] // Shared helper permissions and main process-group ownership.
mod computer_use_macos;
#[allow(dead_code)] // Shared host/helper source; the helper uses only its relevant entry points.
mod computer_use_owned_runtime;
#[allow(dead_code)] // Shared with the host; not every policy is used by the helper.
mod computer_use_policy;
#[cfg(target_os = "windows")]
#[allow(dead_code)] // Shared parent/client and helper/server implementation.
mod computer_use_windows;

#[cfg(target_os = "macos")]
use core_foundation::{base::TCFType, data::CFData};
#[cfg(target_os = "macos")]
use security_framework::os::macos::code_signing::{
    Flags as CodeSigningFlags, GuestAttributes, SecCode, SecRequirement,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(target_os = "macos")]
use std::os::fd::AsRawFd;
use std::{
    io,
    path::PathBuf,
    sync::{Arc, Mutex},
};
#[cfg(target_os = "macos")]
use std::{
    io::Read,
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
#[cfg(target_os = "macos")]
use tokio::net::{UnixListener, UnixStream};

const UPSTREAM_VERSION: &str = "1.0.2";
const UPSTREAM_COMMIT: &str = "9eb9b296192fcb70bda25e918cd6dd66abfa3d0d";
const MAX_REQUEST_BYTES: usize = 256 * 1024;
#[cfg(target_os = "macos")]
const SIGNATURE_DETAILS_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(target_os = "macos")]
const CHILD_TERMINATION_GRACE: Duration = Duration::from_millis(250);
#[cfg(target_os = "macos")]
const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(1);
#[cfg(target_os = "macos")]
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(target_os = "macos")]
struct BoundedChildOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}
#[derive(Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Value>,
}
impl Response {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }
    fn error(id: Value, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(json!({"code":code,"message":message.into()})),
        }
    }
    fn parse_error() -> Self {
        Self::error(Value::Null, -32700, "Invalid private request")
    }
}
#[derive(Debug, PartialEq, Eq)]
enum BoundedLine {
    Eof,
    Line(String),
    TooLarge,
}
async fn read_bounded_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    max_bytes: usize,
) -> io::Result<BoundedLine> {
    let mut bytes = Vec::new();
    let read = reader
        .take((max_bytes as u64).saturating_add(1))
        .read_until(b'\n', &mut bytes)
        .await?;
    if read == 0 {
        return Ok(BoundedLine::Eof);
    }
    if read > max_bytes || bytes.last() != Some(&b'\n') {
        return Ok(BoundedLine::TooLarge);
    }
    String::from_utf8(bytes)
        .map(BoundedLine::Line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
const ALLOWED_TOOLS: &[&str] = &[
    "check_permissions",
    "list_windows",
    "launch_app",
    "join_current_stage",
    "get_window_state",
    "click",
    "double_click",
    "right_click",
    "drag",
    "scroll",
    "type_text",
    "press_key",
    "hotkey",
    "set_value",
];

#[cfg(any(target_os = "macos", target_os = "windows"))]
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--version"] {
        println!("clovy-computer-use-driver {UPSTREAM_VERSION} {UPSTREAM_COMMIT}");
        return;
    }
    #[cfg(target_os = "macos")]
    let result = match args.as_slice() {
        [mode] if mode == "mcp-daemon" => serve_daemon().await,
        _ => Err(anyhow::anyhow!("This helper is private to Clovy")),
    };
    #[cfg(target_os = "windows")]
    let result = match args.as_slice() {
        [mode] if mode == "mcp" => match computer_use_windows::verify_parent() {
            Ok(()) => match computer_use_windows::connect_private_transport() {
                Ok((reader, writer)) => serve(reader, writer).await,
                Err(error) => Err(error),
            },
            Err(e) => Err(e),
        },
        _ => Err(anyhow::anyhow!("This helper is private to Clovy")),
    };
    if let Err(error) = result {
        eprintln!("Computer use helper stopped: {error}");
        std::process::exit(1);
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn main() {
    eprintln!("Clovy desktop control requires macOS or Windows");
    std::process::exit(69);
}

#[cfg(target_os = "macos")]
async fn serve_daemon() -> anyhow::Result<()> {
    isolate_process_group()?;
    let socket_path = PathBuf::from(std::env::var("CLOVY_COMPUTER_USE_SOCKET")?);
    if !socket_path.is_absolute() {
        anyhow::bail!("Private socket must be absolute");
    }
    let run_root = PathBuf::from(
        std::env::var_os("CLOVY_COMPUTER_USE_RUN_DIR")
            .ok_or_else(|| anyhow::anyhow!("Missing private run root"))?,
    );
    if !run_root.is_absolute() || socket_path.parent() != Some(run_root.as_path()) {
        anyhow::bail!("Socket must belong to the parent-owned private run root");
    }
    use std::os::unix::fs::MetadataExt;
    let root_identity = std::fs::symlink_metadata(&run_root)?;
    if !root_identity.is_dir() || root_identity.file_type().is_symlink() {
        anyhow::bail!("Invalid private run root");
    }
    let root_alive = || {
        std::fs::symlink_metadata(&run_root).is_ok_and(|current| {
            current.is_dir()
                && current.dev() == root_identity.dev()
                && current.ino() == root_identity.ino()
        })
    };
    let listener = UnixListener::bind(&socket_path)?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600))?;
    }
    let accepting = listener.accept();
    tokio::pin!(accepting);
    let deadline = tokio::time::sleep(Duration::from_secs(15));
    tokio::pin!(deadline);
    let (stream, _) = loop {
        tokio::select! {
            result=&mut accepting => {if !root_alive(){anyhow::bail!("Clovy cancelled helper startup");}break result?;},
            _=&mut deadline=>anyhow::bail!("Clovy did not connect to the private helper"),
            _=tokio::time::sleep(Duration::from_millis(50))=>if !root_alive(){anyhow::bail!("Clovy cancelled helper startup");}
        }
    };
    let pid = stream
        .peer_cred()?
        .pid()
        .ok_or_else(|| anyhow::anyhow!("Missing peer PID"))?;
    let token = socket_peer_audit_token(&stream)?;
    if !process_is_clovy(pid, Some(&token)) {
        anyhow::bail!("Private helper peer identity rejected");
    }
    let (reader, writer) = stream.into_split();
    let result = serve(reader, writer).await;
    let _ = std::fs::remove_file(socket_path);
    result
}

#[derive(Default)]
struct Connection {
    closed: bool,
    cancel: Option<computer_use_host::HostCancel>,
}
async fn serve<R, W>(reader: R, writer: W) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin,
{
    let namespace = PathBuf::from(
        std::env::var_os("CLOVY_COMPUTER_USE_BROWSER_NAMESPACE")
            .ok_or_else(|| anyhow::anyhow!("Missing browser authority"))?,
    );
    let image = PathBuf::from(
        std::env::var_os("CLOVY_COMPUTER_USE_BROWSER_IMAGE")
            .ok_or_else(|| anyhow::anyhow!("Missing browser authority"))?,
    );
    if !namespace.is_absolute() || !image.is_absolute() {
        anyhow::bail!("Invalid browser authority");
    }
    let browser_runtime = Arc::new(computer_use_owned_runtime::OwnedBrowserRuntime::new(
        namespace, image,
    ));
    let connection = Arc::new(Mutex::new(Connection::default()));
    let reader_connection = connection.clone();
    let (sender, mut messages) = tokio::sync::mpsc::channel(16);
    let reader_task = tokio::spawn(async move {
        let mut reader = BufReader::new(reader);
        loop {
            match read_bounded_line(&mut reader, MAX_REQUEST_BYTES).await {
                Ok(BoundedLine::Line(line)) => {
                    if sender.try_send(Ok(line)).is_err() {
                        break;
                    }
                }
                Ok(BoundedLine::Eof) => break,
                _ => {
                    let _ = sender.try_send(Err(()));
                    break;
                }
            }
        }
        let mut connection = reader_connection.lock().unwrap_or_else(|e| e.into_inner());
        connection.closed = true;
        if let Some(cancel) = &connection.cancel {
            cancel.cancel();
        }
    });
    let mut writer = tokio::io::BufWriter::new(writer);
    let mut authenticated = false;
    let mut adapter = None;
    while let Some(line) = messages.recv().await {
        let request = match line
            .ok()
            .and_then(|line| serde_json::from_str::<Request>(&line).ok())
        {
            Some(request) => request,
            None => {
                write_response(&mut writer, Response::parse_error()).await?;
                break;
            }
        };
        let Some(id) = request.id else {
            continue;
        };
        let response = match request.method.as_str() {
            "initialize" if !authenticated => {
                if !initialize_capability(request.params.as_ref()).is_some_and(valid_capability) {
                    write_response(
                        &mut writer,
                        Response::error(id, -32001, "Clovy capability required"),
                    )
                    .await?;
                    break;
                }
                authenticated = true;
                #[cfg(target_os = "macos")]
                {
                    let closed = connection
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .closed;
                    if closed || computer_use_cancel::current_run_cancelled() {
                        anyhow::bail!(
                            "Clovy cancelled helper startup before the permission request"
                        );
                    }
                    computer_use_macos::request_startup_permission();
                }
                Response::ok(
                    id,
                    json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"Clovy Computer Use Driver","version":UPSTREAM_VERSION},"instructions":"Private helper. All actions require the Clovy policy broker."}),
                )
            }
            _ if !authenticated => Response::error(id, -32001, "Clovy capability required"),
            "tools/list" => Response::ok(id, allowed_tool_list()),
            "tools/call" => {
                let params = request.params.unwrap_or(Value::Null);
                let name = params.get("name").and_then(Value::as_str).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                if !ALLOWED_TOOLS.contains(&name) {
                    Response::error(id, -32601, "Driver operation is not allowed")
                } else if !arguments_are_narrow(name, &args) {
                    Response::error(id, -32602, "Fields outside the Clovy contract")
                } else if name == "check_permissions" {
                    Response::ok(id, permission_result())
                } else {
                    if adapter.is_none() {
                        let executable = engine_executable()?;
                        let run_root = PathBuf::from(
                            std::env::var_os("CLOVY_COMPUTER_USE_RUN_DIR").ok_or_else(|| {
                                anyhow::anyhow!("Missing parent-owned run namespace")
                            })?,
                        );
                        if !run_root.is_absolute()
                            || !run_root.is_dir()
                            || std::fs::symlink_metadata(&run_root)?
                                .file_type()
                                .is_symlink()
                        {
                            anyhow::bail!("Invalid parent-owned run namespace");
                        }
                        let home = run_root.join(format!("engine-{}", uuid::Uuid::new_v4()));
                        let host = computer_use_host::CommandHost::start(
                            executable,
                            home,
                            Arc::clone(&browser_runtime),
                        )
                        .await?;
                        let mut state = connection.lock().unwrap_or_else(|e| e.into_inner());
                        state.cancel = Some(host.cancel_handle());
                        if state.closed {
                            host.cancel_handle().cancel();
                            anyhow::bail!("Clovy disconnected");
                        }
                        adapter = Some(computer_use_engine::EngineAdapter::new(host));
                    }
                    Response::ok(id, adapter.as_mut().unwrap().invoke(name, args).await)
                }
            }
            _ => Response::error(id, -32601, "Private method is unavailable"),
        };
        write_response(&mut writer, response).await?;
    }
    reader_task.abort();
    if let Some(cancel) = &connection.lock().unwrap_or_else(|e| e.into_inner()).cancel {
        cancel.cancel();
    }
    Ok(())
}
fn private_contract() -> &'static Value {
    static CONTRACT: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    CONTRACT.get_or_init(|| {
        serde_json::from_str(include_str!("../computer-use-contract.json"))
            .expect("packaged Computer use contract is valid JSON")
    })
}
fn allowed_tool_list() -> Value {
    let contract = private_contract();
    let tools: Vec<Value> = ALLOWED_TOOLS.iter().map(|name| {
        let definition = &contract["driverTools"][*name];
        let properties: serde_json::Map<String,Value> = definition["properties"].as_object().expect("private tool properties").iter().map(|(field,kind)| {
            let schema = if *name == "get_window_state" && field == "capture_mode" {
                json!({"type":kind,"enum":["som","vision","ax"]})
            } else if kind == "array" { json!({"type":"array","items":{"type":"string"},"maxItems":16}) } else { json!({"type":kind}) };
            (field.clone(),schema)
        }).collect();
        json!({"name":name,"description":"Private broker-authorized Clovy desktop operation","inputSchema":{"type":"object","properties":properties,"required":definition["required"],"additionalProperties":false}})
    }).collect();
    json!({"tools":tools})
}
fn engine_executable() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_exe()?
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Helper directory missing"))?
        .join(if cfg!(target_os = "windows") {
            "agent-desktop.exe"
        } else {
            "agent-desktop"
        }))
}
fn permission_result() -> Value {
    let engine_check = (|| -> anyhow::Result<()> {
        let executable = engine_executable()?;
        #[cfg(target_os = "windows")]
        let _image_lock = computer_use_windows::lock_image(
            &executable,
            option_env!("CLOVY_AGENT_DESKTOP_EXECUTABLE_SHA256"),
        )?;
        computer_use_host::verify_engine(&executable)?;
        #[cfg(target_os = "windows")]
        computer_use_windows::verify_signed_peer(&executable)?;
        Ok(())
    })();
    if let Err(error) = engine_check {
        return json!({"isError":true,"content":[{"type":"text","text":format!("The packaged Computer use engine is missing or mismatched. {error}")}],"structuredContent":{"code":"computer_use_engine_mismatch","disposition":{"delivery":"not_delivered","retry":"safe"}}});
    }
    #[cfg(target_os = "macos")]
    {
        computer_use_macos::permission_result()
    }
    #[cfg(target_os = "windows")]
    {
        computer_use_windows::permission_result()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        json!({"isError":true,"content":[{"type":"text","text":"Desktop platform unavailable"}]})
    }
}
async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut tokio::io::BufWriter<W>,
    response: Response,
) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(&response)?;
    writer.write_all(&bytes).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}
#[cfg(target_os = "macos")]
fn process_is_clovy(pid: libc::pid_t, audit_token: Option<&[u8]>) -> bool {
    let Some(process_path) = process_path(pid) else {
        return false;
    };
    let Ok(helper_path) = std::env::current_exe() else {
        return false;
    };

    // Packaged build: the helper is nested under the signed outer app.
    // Require its direct parent to be that exact outer bundle executable.
    let app_ancestors: Vec<PathBuf> = helper_path
        .ancestors()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
        })
        .map(PathBuf::from)
        .collect();
    if let (Some(helper_app), Some(outer_app)) = (app_ancestors.first(), app_ancestors.get(1)) {
        // Packaged peers must arrive over the private daemon socket so their
        // non-reusable audit token is available. Direct stdio remains a
        // development-only path below.
        return same_file(&process_path, &packaged_main_executable(outer_app))
            && audit_token
                .is_some_and(|token| packaged_process_signature_matches(token, helper_app));
    }

    // Development builds are not nested. Accept only `clovy` built in one of
    // this checkout's Cargo profile directories: Cargo's own binary, or the
    // main executable of the dev runner's signed bundle directly inside that
    // directory (`target/debug/Clovy Dev.app/Contents/MacOS/clovy`). The path
    // marker is only a location check; every development peer must also pass
    // the same audit-token, compiled identifier and same-Team signature gate.
    cfg!(debug_assertions)
        && development_process_path_is_clovy(
            &process_path,
            &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"),
        )
        && app_ancestors.first().is_some_and(|helper_app| {
            audit_token.is_some_and(|token| packaged_process_signature_matches(token, helper_app))
        })
}

#[cfg(target_os = "macos")]
fn packaged_main_executable(outer_app: &std::path::Path) -> PathBuf {
    outer_app
        .join("Contents")
        .join("MacOS")
        .join(app_identity::MAIN_EXECUTABLE)
}

#[cfg(target_os = "macos")]
fn development_process_path_is_clovy(
    process_path: &std::path::Path,
    target_root: &std::path::Path,
) -> bool {
    use std::ffi::OsStr;

    let Ok(relative) = process_path.strip_prefix(target_root) else {
        return false;
    };
    // The deepest accepted shape is
    // `<triple>/<profile>/<bundle>.app/Contents/MacOS/clovy`.
    let mut names = [OsStr::new(""); 6];
    let mut depth = 0;
    for component in relative.components() {
        let std::path::Component::Normal(name) = component else {
            return false;
        };
        let Some(slot) = names.get_mut(depth) else {
            return false;
        };
        *slot = name;
        depth += 1;
    }
    let names = &names[..depth];
    let is_main_executable = |rest: &[&OsStr]| match rest {
        [executable] => *executable == OsStr::new(app_identity::MAIN_EXECUTABLE),
        [bundle, contents, macos, executable] => {
            std::path::Path::new(bundle)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
                && *contents == OsStr::new("Contents")
                && *macos == OsStr::new("MacOS")
                && *executable == OsStr::new(app_identity::MAIN_EXECUTABLE)
        }
        _ => false,
    };
    // Cargo marks every profile directory (`debug`, `<triple>/release`, ...)
    // with `.cargo-lock`; any other directory under target/ is not a build
    // output this helper trusts. `prefix` is how many names lead to it.
    let is_profile_dir = |prefix: usize| {
        process_path
            .ancestors()
            .nth(depth - prefix)
            .is_some_and(|profile| profile.join(".cargo-lock").is_file())
    };
    match names {
        [_profile, rest @ ..] if is_main_executable(rest) => is_profile_dir(1),
        [triple, _profile, rest @ ..] => {
            triple
                .to_str()
                .is_some_and(|triple| triple.ends_with("-apple-darwin"))
                && is_main_executable(rest)
                && is_profile_dir(2)
        }
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn socket_peer_audit_token(stream: &UnixStream) -> io::Result<[u8; 32]> {
    let mut token = [0u8; 32];
    let mut token_len = libc::socklen_t::try_from(token.len())
        .map_err(|_| io::Error::other("invalid macOS audit token length"))?;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_LOCAL,
            libc::LOCAL_PEERTOKEN,
            token.as_mut_ptr().cast(),
            &mut token_len,
        )
    };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    if usize::try_from(token_len).ok() != Some(token.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the private Clovy peer returned an invalid audit token",
        ));
    }
    Ok(token)
}

#[cfg(target_os = "macos")]
fn packaged_process_signature_matches(audit_token: &[u8], helper_app: &std::path::Path) -> bool {
    let Some(expected_identifier) = option_env!("CLOVY_COMPUTER_USE_HELPER_IDENTIFIER")
        .filter(|identifier| valid_signing_identifier(identifier))
    else {
        return false;
    };
    let Some(helper) = signature_details(helper_app, expected_identifier) else {
        return false;
    };
    let Some(requirement) = clovy_process_requirement(&helper.team_identifier) else {
        return false;
    };
    let audit_token = CFData::from_buffer(audit_token);
    let mut attributes = GuestAttributes::new();
    attributes.set_audit_token(audit_token.as_concrete_TypeRef());
    let Ok(peer) = SecCode::copy_guest_with_attribues(None, &attributes, CodeSigningFlags::NONE)
    else {
        return false;
    };
    peer.check_validity(
        CodeSigningFlags::STRICT_VALIDATE | CodeSigningFlags::NO_NETWORK_ACCESS,
        &requirement,
    )
    .is_ok()
}

#[cfg(target_os = "macos")]
struct VerifiedSignature {
    team_identifier: String,
}

#[cfg(target_os = "macos")]
fn signature_details(
    path: &std::path::Path,
    expected_identifier: &str,
) -> Option<VerifiedSignature> {
    let mut verification = Command::new("/usr/bin/codesign");
    verification.args(["--verify", "--strict"]).arg(path);
    let verification = bounded_child_output(verification, SIGNATURE_DETAILS_TIMEOUT).ok()??;
    if !verification.status.success() {
        return None;
    }

    let mut command = Command::new("/usr/bin/codesign");
    command.args(["-dv", "--verbose=4"]).arg(path);
    let details = bounded_child_output(command, SIGNATURE_DETAILS_TIMEOUT).ok()??;
    if !details.status.success() {
        return None;
    }
    let details = format!(
        "{}\n{}",
        String::from_utf8_lossy(&details.stdout),
        String::from_utf8_lossy(&details.stderr)
    );
    let identifier = signature_value(&details, "Identifier")?;
    let team_identifier = signature_value(&details, "TeamIdentifier")?;
    if identifier != expected_identifier || team_identifier == "not set" {
        return None;
    }
    Some(VerifiedSignature {
        team_identifier: team_identifier.to_string(),
    })
}

#[cfg(target_os = "macos")]
fn clovy_process_requirement(team_identifier: &str) -> Option<SecRequirement> {
    if team_identifier.is_empty()
        || team_identifier.len() > 64
        || !team_identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    if !valid_signing_identifier(app_identity::APP_IDENTIFIER) {
        return None;
    }
    let identifier = app_identity::APP_IDENTIFIER;
    let identity = if cfg!(debug_assertions) {
        format!("(identifier \"{identifier}\" or identifier \"{identifier}.dev\")")
    } else {
        format!("identifier \"{identifier}\"")
    };
    format!("anchor apple generic and {identity} and certificate leaf[subject.OU] = \"{team_identifier}\"").parse().ok()
}

#[cfg(target_os = "macos")]
fn valid_signing_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier.len() <= 255
        && identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

#[cfg(target_os = "macos")]
fn signature_value<'a>(details: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    details
        .lines()
        .find_map(|line| line.trim().strip_prefix(prefix.as_str()))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

#[cfg(target_os = "macos")]
fn isolate_process_group() -> anyhow::Result<()> {
    let pid = unsafe { libc::getpid() };
    if unsafe { libc::getpgrp() } == pid || unsafe { libc::setpgid(0, 0) } == 0 {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "the private helper could not isolate its process group: {}",
        io::Error::last_os_error()
    ))
}

#[cfg(target_os = "macos")]
fn bounded_child_output(
    mut command: Command,
    timeout: Duration,
) -> io::Result<Option<BoundedChildOutput>> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_reader = thread::spawn(move || read_pipe(stdout));
    let stderr_reader = thread::spawn(move || read_pipe(stderr));

    let status = match poll_child_exit(&mut child, timeout)? {
        Some(status) => status,
        None => {
            if !terminate_child_and_reap(child) {
                // The detached reaper and pipe readers retain ownership until
                // the late exit. Do not block helper authentication on them.
                return Ok(None);
            }
            return Ok(None);
        }
    };
    let stdout = stdout_reader.join().unwrap_or_else(|_| Ok(Vec::new()))?;
    let stderr = stderr_reader.join().unwrap_or_else(|_| Ok(Vec::new()))?;
    Ok(Some(BoundedChildOutput {
        status,
        stdout,
        stderr,
    }))
}

#[cfg(target_os = "macos")]
fn read_pipe<R: Read>(pipe: Option<R>) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if let Some(mut pipe) = pipe {
        pipe.read_to_end(&mut bytes)?;
    }
    Ok(bytes)
}

#[cfg(target_os = "macos")]
fn poll_child_exit(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait()? {
            Some(status) => return Ok(Some(status)),
            None if Instant::now() < deadline => thread::sleep(CHILD_POLL_INTERVAL),
            None => return Ok(None),
        }
    }
}

#[cfg(target_os = "macos")]
fn terminate_child_and_reap(mut child: Child) -> bool {
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        let _ = unsafe { libc::kill(pid, libc::SIGTERM) };
    }
    if let Ok(Some(_)) = poll_child_exit(&mut child, CHILD_TERMINATION_GRACE) {
        return true;
    }

    let _ = child.kill();
    match poll_child_exit(&mut child, CHILD_REAP_TIMEOUT) {
        Ok(Some(_)) => true,
        Ok(None) | Err(_) => {
            let _ = thread::Builder::new()
                .name("clovy-helper-child-reaper".to_string())
                .spawn(move || {
                    let _ = child.wait();
                });
            false
        }
    }
}

#[cfg(target_os = "macos")]
fn process_path(pid: libc::pid_t) -> Option<PathBuf> {
    const PROC_PIDPATHINFO_MAXSIZE: usize = 4096;
    let mut buffer = vec![0u8; PROC_PIDPATHINFO_MAXSIZE];
    let length = unsafe {
        proc_pidpath(
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
fn same_file(left: &std::path::Path, right: &std::path::Path) -> bool {
    use std::os::unix::fs::MetadataExt;

    let (Ok(left), Ok(right)) = (std::fs::metadata(left), std::fs::metadata(right)) else {
        return false;
    };
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(target_os = "macos")]
#[link(name = "proc")]
extern "C" {
    fn proc_pidpath(pid: libc::pid_t, buffer: *mut libc::c_void, buffersize: u32) -> libc::c_int;
}

fn initialize_capability(params: Option<&Value>) -> Option<&str> {
    let experimental = params?.get("capabilities")?.get("experimental")?;
    experimental.get("clovyComputerUseCapability")?.as_str()
}

fn valid_capability(candidate: &str) -> bool {
    let expected = std::env::var("CLOVY_COMPUTER_USE_HELPER_CAPABILITY").unwrap_or_default();
    if expected.len() < 32 || candidate.len() != expected.len() {
        return false;
    }
    candidate
        .as_bytes()
        .iter()
        .zip(expected.as_bytes())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

fn arguments_are_narrow(tool: &str, arguments: &Value) -> bool {
    let Some(arguments) = arguments.as_object() else {
        return false;
    };
    let Some(definition) = private_contract()["driverTools"].get(tool) else {
        return false;
    };
    let Some(properties) = definition["properties"].as_object() else {
        return false;
    };
    if !definition["required"].as_array().is_some_and(|required| {
        required.iter().all(|field| {
            field
                .as_str()
                .is_some_and(|field| arguments.contains_key(field))
        })
    }) {
        return false;
    }
    for (field, value) in arguments {
        let Some(kind) = properties.get(field).and_then(Value::as_str) else {
            return false;
        };
        let valid = match kind {
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.as_f64().is_some_and(f64::is_finite),
            "string" => value.is_string(),
            "boolean" => value.is_boolean(),
            "array" => value
                .as_array()
                .is_some_and(|values| values.len() <= 16 && values.iter().all(Value::is_string)),
            _ => false,
        };
        if !valid {
            return false;
        }
    }
    match tool {
        "launch_app" => {
            arguments.len() == 1
                && arguments
                    .get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| !name.trim().is_empty() && name.chars().count() <= 200)
        }
        "join_current_stage" => {
            arguments.len() == 2
                && arguments
                    .get("pid")
                    .and_then(Value::as_i64)
                    .is_some_and(|pid| pid > 0 && i32::try_from(pid).is_ok())
                && arguments
                    .get("window_id")
                    .and_then(Value::as_u64)
                    .is_some_and(|window_id| {
                        window_id > 0
                            && usize::try_from(window_id).is_ok()
                            && (!cfg!(target_os = "macos") || u32::try_from(window_id).is_ok())
                    })
        }
        _ => true,
    }
}

#[cfg(test)]
mod private_protocol_tests {
    use super::*;

    #[test]
    fn window_state_schema_advertises_supported_capture_modes() {
        let tool_list = allowed_tool_list();
        let capture = tool_list["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "get_window_state")
            .expect("get_window_state is advertised");
        let schema = &capture["inputSchema"];
        let capture_mode = &schema["properties"]["capture_mode"];
        assert_eq!(capture_mode["type"], "string");
        let modes = capture_mode["enum"]
            .as_array()
            .expect("capture_mode advertises its supported modes");
        assert_eq!(modes.len(), 3);
        for mode in ["som", "vision", "ax"] {
            assert!(modes.contains(&json!(mode)), "missing capture mode {mode}");
        }
        assert_eq!(schema["additionalProperties"], false);
    }

    #[tokio::test]
    async fn oversized_or_unterminated_frames_close_the_private_channel() {
        for bytes in [b"12345\n".as_slice(), b"1234".as_slice()] {
            let mut reader = BufReader::new(bytes);
            assert_eq!(
                read_bounded_line(&mut reader, 4).await.unwrap(),
                BoundedLine::TooLarge
            );
        }
        let mut reader = BufReader::new(b"\"ok\"\nrest\n".as_slice());
        assert_eq!(
            read_bounded_line(&mut reader, 10).await.unwrap(),
            BoundedLine::Line("\"ok\"\n".into())
        );
        assert_eq!(
            read_bounded_line(&mut reader, 10).await.unwrap(),
            BoundedLine::Line("rest\n".into())
        );
    }
    #[test]
    fn private_operations_reject_extra_engine_authority() {
        for name in ALLOWED_TOOLS {
            assert!(!arguments_are_narrow(
                name,
                &json!({"session":"other","socket":"public","command":"arbitrary"})
            ));
        }
        assert!(!arguments_are_narrow(
            "launch_app",
            &json!({"name":"x","args":["bad"]})
        ));
        assert!(arguments_are_narrow(
            "launch_app",
            &json!({"name":"TextEdit"})
        ));
        assert!(!arguments_are_narrow(
            "join_current_stage",
            &json!({"pid":1,"window_id":0})
        ));
        assert_eq!(ALLOWED_TOOLS.len(), 14);
    }
}
