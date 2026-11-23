//! The Clovy MCP server: lets local MCP clients (Claude Code, Cursor, and the
//! CLI chat engines) read the current data partition's notes, dictations, and
//! memories, and the installation's activity. On by default; the switch is
//! `mcpServer` in `activity-settings.json`, changed only from Settings, MCP.
//! Contract and protocol: `docs/mcp-server.md`.
//!
//! Layout: `channel` (Unix socket + installation secret), `stdio` (the
//! `clovy-mcp` relay binary), `protocol` (MCP JSON-RPC and resources),
//! `tools` (tool groups: the read group for every client, bound-only groups
//! for CLI turns), `turns` (which CLI turn a connection belongs to). The binary never opens a database: it
//! relays to this process, which answers with the same queries and filters
//! the app uses.

#[cfg(test)]
mod bound_partition_tests;
pub mod channel;
pub mod protocol;
#[cfg(all(test, any(unix, windows)))]
#[path = "red_tests.rs"]
mod red_tests;
pub mod stdio;
#[cfg(test)]
pub(crate) mod tests;
pub mod tools;
#[cfg(all(test, any(unix, windows)))]
#[path = "turn_tests.rs"]
mod turn_tests;
pub mod turns;
pub mod wire_names;

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use crate::domain::types::AppError;
use tools::{ActivityAccess, McpData, ToolCtx};

/// The relay binary (`[[bin]]` in Cargo.toml, bundled under
/// `Resources/native/bin/`).
#[cfg(not(windows))]
pub const BINARY_NAME: &str = "clovy-mcp";
#[cfg(windows)]
pub const BINARY_NAME: &str = "clovy-mcp.exe";
/// The server's key in `mcpServers` configuration objects.
pub const SERVER_KEY: &str = "clovy";

/// The command an MCP client runs to reach Clovy. Needs no environment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpLaunch {
    pub command: PathBuf,
    pub args: Vec<String>,
}

pub fn launch(binary: &Path, channel_dir: &Path) -> McpLaunch {
    McpLaunch {
        command: binary.to_path_buf(),
        args: vec!["--dir".into(), channel_dir.display().to_string()],
    }
}

impl McpLaunch {
    /// The same command bound to a CLI turn (`--turn <token>`, a token from
    /// [`turns::TurnRegistry::issue`]).
    pub fn for_turn(&self, token: &str) -> McpLaunch {
        let mut args = self.args.clone();
        args.extend(["--turn".to_string(), token.to_string()]);
        McpLaunch {
            command: self.command.clone(),
            args,
        }
    }
}

/// `{"mcpServers": {"clovy": {...}}}` as Claude Code reads it (`.mcp.json`,
/// `--mcp-config`); also what CLI chat engines get for a child process.
pub fn claude_code_config(launch: &McpLaunch) -> Value {
    json!({
        "mcpServers": {
            SERVER_KEY: {
                "type": "stdio",
                "command": launch.command.display().to_string(),
                "args": launch.args,
            }
        }
    })
}

/// `~/.cursor/mcp.json` (or a project's `.cursor/mcp.json`).
pub fn cursor_config(launch: &McpLaunch) -> Value {
    json!({
        "mcpServers": {
            SERVER_KEY: {
                "command": launch.command.display().to_string(),
                "args": launch.args,
            }
        }
    })
}

fn shell_quote(value: &str) -> String {
    #[cfg(windows)]
    {
        return format!("'{}'", value.replace('\'', "''"));
    }
    #[cfg(not(windows))]
    {
        if !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/._-=:@%+,".contains(&byte))
        {
            return value.to_string();
        }
        format!("'{}'", value.replace('\'', r"'\''"))
    }
}

/// `claude mcp add` for the user scope (every project).
pub fn claude_code_command(launch: &McpLaunch) -> String {
    let mut parts = vec![
        "claude".to_string(),
        "mcp".into(),
        "add".into(),
        "--scope".into(),
        "user".into(),
        SERVER_KEY.into(),
        "--".into(),
        shell_quote(&launch.command.display().to_string()),
    ];
    parts.extend(launch.args.iter().map(|arg| shell_quote(arg)));
    parts.join(" ")
}

/// Where the bundled or freshly built relay lives, first existing first.
fn binary_candidates(resource_dir: Option<&Path>, exe: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(resources) = resource_dir {
        candidates.push(resources.join("native").join("bin").join(BINARY_NAME));
    }
    if let Some(exe_dir) = exe.and_then(Path::parent) {
        candidates.push(exe_dir.join(BINARY_NAME));
    }
    candidates
}

fn resolve_binary(app: &AppHandle) -> (PathBuf, bool) {
    let resource_dir = app.path().resource_dir().ok();
    let exe = std::env::current_exe().ok();
    let candidates = binary_candidates(resource_dir.as_deref(), exe.as_deref());
    match candidates.iter().find(|path| path.is_file()) {
        Some(found) => (
            dunce::canonicalize(found).unwrap_or_else(|_| found.clone()),
            true,
        ),
        None => (
            candidates
                .into_iter()
                .next()
                .unwrap_or_else(|| PathBuf::from(BINARY_NAME)),
            false,
        ),
    }
}

fn channel_dir(app: &AppHandle) -> Result<PathBuf, AppError> {
    crate::app_paths::app_data_dir(app)
        .map(|data_dir| channel::channel_dir(&data_dir))
        .map_err(|error| AppError::new("mcp_server_paths", error.to_string()))
}

/// The launch command for a child process (the CLI chat engine passes
/// `claude_code_config(&launch)` to CLIs that accept MCP configuration).
/// The server must be on for its tools to answer.
pub fn child_process_launch(app: &AppHandle) -> Result<McpLaunch, AppError> {
    let (binary, found) = resolve_binary(app);
    if !found {
        return Err(AppError::new(
            "mcp_server_binary_missing",
            format!("The {BINARY_NAME} executable was not found next to Clovy."),
        ));
    }
    Ok(launch(&binary, &channel_dir(app)?))
}

/// The running listener and the last start error, behind an async lock so
/// start and stop never interleave, and the live CLI turns (kept across
/// restarts of the listener).
#[derive(Default)]
pub struct McpServerState {
    slot: tokio::sync::Mutex<Slot>,
    turns: turns::TurnRegistry,
}

#[derive(Default)]
struct Slot {
    #[cfg(any(unix, windows))]
    listener: Option<channel::Listener>,
    error: Option<String>,
}

impl McpServerState {
    /// The live CLI turns: the chat engine issues and revokes their tokens.
    pub fn turns(&self) -> &turns::TurnRegistry {
        &self.turns
    }

    /// Starts the listener in the directory `dir` resolves, with `handler`,
    /// when `enabled` and none runs; stops it when not `enabled`. `dir` and
    /// `handler` run only when the listener starts.
    #[cfg(any(unix, windows))]
    pub(crate) async fn apply_switch<H>(
        &self,
        enabled: bool,
        dir: impl FnOnce() -> Result<PathBuf, AppError>,
        handler: impl FnOnce() -> H,
    ) where
        H: Into<channel::Service>,
    {
        let mut slot = self.slot.lock().await;
        if enabled
            && !slot
                .listener
                .as_ref()
                .is_some_and(channel::Listener::is_running)
        {
            if let Some(listener) = slot.listener.take() {
                listener.stop_and_wait().await;
            }
            let started = match dir() {
                Ok(dir) => channel::Listener::start_with_turns(&dir, handler(), self.turns.clone())
                    .await
                    .map_err(|error| error.to_string()),
                Err(error) => Err(error.message),
            };
            match started {
                Ok(listener) => {
                    tracing::info!(socket = %listener.socket().display(), "Clovy MCP server listening");
                    slot.listener = Some(listener);
                    slot.error = None;
                }
                Err(error) => {
                    tracing::warn!(%error, "Clovy MCP server could not start");
                    slot.error = Some(error);
                }
            }
        } else if !enabled {
            if let Some(listener) = slot.listener.take() {
                listener.stop_and_wait().await;
            }
            slot.error = None;
        }
    }

    /// Whether the listener runs, and the last start error, read together.
    pub(crate) async fn listener_status(&self) -> (bool, Option<String>) {
        let slot = self.slot.lock().await;
        #[cfg(any(unix, windows))]
        let running = slot
            .listener
            .as_ref()
            .is_some_and(channel::Listener::is_running);
        #[cfg(not(any(unix, windows)))]
        let running = false;
        let error = slot.error.clone();
        #[cfg(any(unix, windows))]
        let error = error.or_else(|| {
            (slot.listener.is_some() && !running)
                .then(|| "The MCP listener stopped. Turn the server off and on to retry.".into())
        });
        (running, error)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatusDto {
    /// macOS or Windows with activity settings available.
    pub supported: bool,
    pub enabled: bool,
    pub running: bool,
    pub error: Option<String>,
    pub binary_path: String,
    pub binary_found: bool,
    pub claude_code_command: String,
    pub claude_code_config: String,
    pub cursor_config: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMcpServerEnabledRequest {
    pub enabled: bool,
}

fn supported(app: &AppHandle) -> bool {
    cfg!(any(target_os = "macos", windows)) && crate::activity::activity_settings_supported(app)
}

async fn status(app: &AppHandle) -> Result<McpServerStatusDto, AppError> {
    let (binary, binary_found) = resolve_binary(app);
    let launch = launch(&binary, &channel_dir(app)?);
    let state = app.state::<McpServerState>();
    let (running, error) = state.listener_status().await;
    let pretty = |value: Value| serde_json::to_string_pretty(&value).unwrap_or_default();
    Ok(McpServerStatusDto {
        supported: supported(app),
        enabled: crate::activity::mcp_server_enabled(app),
        running,
        error,
        binary_path: binary.display().to_string(),
        binary_found,
        claude_code_command: claude_code_command(&launch),
        claude_code_config: pretty(claude_code_config(&launch)),
        cursor_config: pretty(cursor_config(&launch)),
    })
}

/// Starts or stops the listener to match the stored switch.
async fn reconcile(app: &AppHandle) {
    let enabled = crate::activity::mcp_server_enabled(app);
    let state = app.state::<McpServerState>();
    #[cfg(any(unix, windows))]
    state
        .apply_switch(enabled, || channel_dir(app), || app_handler(app.clone()))
        .await;
    #[cfg(not(any(unix, windows)))]
    {
        let _ = enabled;
        state.slot.lock().await.error = None;
    }
}

/// Registers the state and starts the listener when the switch is on.
pub fn setup(app: &mut tauri::App) {
    app.manage(McpServerState::default());
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move { reconcile(&handle).await });
}

#[tauri::command]
pub async fn mcp_server_status(app: AppHandle) -> Result<McpServerStatusDto, AppError> {
    status(&app).await
}

#[tauri::command]
pub async fn mcp_server_set_enabled(
    app: AppHandle,
    request: SetMcpServerEnabledRequest,
) -> Result<McpServerStatusDto, AppError> {
    if !supported(&app) {
        return Err(AppError::new(
            "mcp_server_unsupported",
            "The Clovy MCP server requires macOS or Windows with activity settings available.",
        ));
    }
    crate::activity::set_mcp_server_enabled(&app, request.enabled)?;
    if !request.enabled {
        if let Some(broker) =
            app.try_state::<crate::approval_broker::external::ExternalApprovalBroker>()
        {
            broker.invalidate();
        }
    }
    reconcile(&app).await;
    status(&app).await
}

#[cfg(any(unix, windows))]
fn app_handler(app: AppHandle) -> channel::AuthenticatedHandler {
    use channel::Connection;
    std::sync::Arc::new(move |message: Value, connection| {
        let app = app.clone();
        Box::pin(async move {
            let binding = match &connection {
                Connection::Turn(binding) => Some(binding.as_ref()),
                Connection::External(_) => None,
            };
            match app_data(&app, &message, binding).await {
                Ok(data) => {
                    let broker = app.try_state::<crate::approval_broker::ApprovalBroker>();
                    let ctx = ToolCtx {
                        data: &data,
                        app: Some(&app),
                        binding,
                        broker: broker.as_deref(),
                    };
                    if let Connection::External(connection) = &connection {
                        if let Some(external) = app.try_state::<crate::approval_broker::external::ExternalApprovalBroker>() {
                            let settings = crate::commands::memory_settings_path(&app).ok();
                            let recordings = crate::app_paths::app_data_dir(&app).ok();
                            return protocol::handle_external(&ctx, connection, &external, settings.as_deref(), recordings.as_deref(), &message).await;
                        }
                    }
                    protocol::handle_in(&ctx, &message).await
                }
                Err(error) => protocol::request_id(&message).map(|id| {
                    protocol::error_response(id, protocol::SERVER_UNAVAILABLE, error.message)
                }),
            }
        })
    })
}

/// Resolve bound requests from the host-owned session, not the currently visible view.
async fn app_data(
    app: &AppHandle,
    message: &Value,
    binding: Option<&turns::TurnBinding>,
) -> Result<McpData, AppError> {
    let notes = crate::commands::repositories(app).await?.pool;
    let profile = if let Some(binding) = binding {
        bound_profile(&notes, binding).await?
    } else {
        crate::commands::active_profile(app)
    };
    let memory_enabled = crate::commands::memory_settings_path(app)
        .map(|path| crate::commands::load_memory_settings(&path).enabled)
        .unwrap_or(false);
    let activity = if !crate::activity::timeline::capture_enabled(app) {
        ActivityAccess::Off
    } else if protocol::calls_activity_tool(message) {
        match crate::activity::timeline::store_for_tools(app).await {
            Ok((store, settings)) => ActivityAccess::Open { store, settings },
            Err(error) => ActivityAccess::Unavailable(error),
        }
    } else {
        ActivityAccess::On
    };
    Ok(McpData {
        notes,
        profile,
        memory_enabled,
        activity,
        now: Utc::now(),
    })
}

async fn bound_profile(
    pool: &sqlx_sqlite::SqlitePool,
    binding: &turns::TurnBinding,
) -> Result<String, AppError> {
    binding.authority.check_live()?;
    crate::agent_sessions::run_partition::revalidate(
        pool,
        &binding.session_id,
        &binding.run_id,
        &binding.data_partition,
    )
    .await?;
    binding.authority.check_live()?;
    Ok(binding.data_partition.clone())
}
