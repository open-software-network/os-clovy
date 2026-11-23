//! One CLI turn: resume lookup, prompt, the child process in its own process
//! group, stream translation, and the run events persisted through the same
//! session event path (`agent_sessions::events::persist_runtime_event`).

use super::cli::{self, EngineEvent, StreamTranslator};
use super::policy::TurnPermissions;
use super::{engine_model_id, FrameSink};
use crate::agent_sessions::protocol::{RpcFrame, PROTOCOL_VERSION};
use crate::agent_sessions::AgentSafetyMode;
use crate::agent_sessions::{AgentItemPayload, AgentRepository};
use crate::llm::cli::CliKind;
use crate::mcp_server::turns::{TurnBinding, TurnRegistry};
use crate::mcp_server::McpLaunch;
use crate::process_group::{GroupSignal, ProcessTree};
use serde_json::{json, Value};
use sqlx::row::Row;
use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::watch;
use uuid::Uuid;

/// Longest stdout line kept; longer lines (a huge tool result) are skipped.
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Tail of stderr kept for error messages.
const STDERR_TAIL_BYTES: usize = 16 * 1024;
/// Earlier conversation handed to a CLI that starts without a resume id.
const MAX_HISTORY_CHARS: usize = 24_000;
/// Time a cancelled CLI gets to save its session after SIGTERM.
pub(crate) const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// Output still buffered after the CLI exited (and its group was swept) is
/// read for at most this long.
const OUTPUT_DRAIN_GRACE: Duration = Duration::from_secs(1);

/// Everything one turn needs, resolved by the caller.
#[derive(Clone, Debug)]
pub struct TurnContext {
    pub session_id: String,
    pub run_id: String,
    /// The data partition snapshotted when the host created this run.
    pub data_partition: String,
    /// A retried attempt whose conversation and messages must not be reused.
    pub previous_run_id: Option<String>,
    pub kind: CliKind,
    pub program: PathBuf,
    /// The complete child environment (the login-shell environment).
    pub env: BTreeMap<String, String>,
    /// App paths captured from the real host, independent of LoginEnv/HOME.
    pub agy: Option<crate::llm::agy::HostContext>,
    pub workspace: PathBuf,
    pub input: String,
    /// Clovy's MCP server, for CLIs that accept it.
    pub mcp: Option<McpLaunch>,
    /// Where the turn's MCP connections are bound (`McpServerState::turns`):
    /// with it, the server gets `--turn <token>` for this run only. Without
    /// it, the server runs unbound (read-only tools).
    pub turns: Option<TurnRegistry>,
    /// The session's safety mode.
    pub safety_mode: AgentSafetyMode,
    /// A person watches the turn (`false` for routines).
    pub attended: bool,
    pub surface: String,
    pub routine_tools: Option<super::policy::RoutineBuiltinTools>,
    /// Image attachments of the message (absolute paths in the workspace).
    pub images: Vec<PathBuf>,
}

impl TurnContext {
    pub fn permissions(&self) -> TurnPermissions {
        TurnPermissions {
            safety_mode: self.safety_mode,
            attended: self.attended,
            routine_tools: self.routine_tools,
        }
    }

    /// The binding the turn's token stands for.
    pub fn binding(&self) -> TurnBinding {
        TurnBinding {
            authority: crate::domain::turn_authority::TurnAuthority::new(),
            wire_names: None,
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            data_partition: self.data_partition.clone(),
            cli: self.kind,
            safety_mode: self.safety_mode,
            attended: self.attended,
            surface: self.surface.clone(),
        }
    }
}

/// The turn token of one run: issued before the CLI starts and revoked when
/// dropped (the turn ended, failed, or was cancelled; the app is quitting),
/// which closes the run's MCP connections.
pub struct TurnGrant {
    turns: TurnRegistry,
    token: String,
}

impl TurnGrant {
    pub fn issue(turns: &TurnRegistry, binding: TurnBinding) -> Self {
        Self {
            token: turns.issue(binding),
            turns: turns.clone(),
        }
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for TurnGrant {
    fn drop(&mut self) {
        self.turns.revoke(&self.token);
    }
}

impl std::fmt::Debug for TurnGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token never reaches logs.
        formatter.write_str("TurnGrant(..)")
    }
}

// Revocation must precede filesystem cleanup on every early return/unwind.
#[derive(Default)]
struct TurnResources {
    grant: Option<TurnGrant>,
    execution: Option<crate::llm::agy::Execution>,
}
impl TurnResources {
    fn finish_terminal(&mut self, mut params: Value) -> Value {
        if let Err(error) = self.finish() {
            let cleanup = error.to_string();
            if let Some(cause) = params.get("error").and_then(Value::as_str) {
                params["error"] = json!(format!("{cause} Workspace MCP cleanup: {cleanup}"));
            }
            params["cleanupError"] = json!(cleanup);
        }
        params
    }

    fn finish(&mut self) -> std::io::Result<()> {
        drop(self.grant.take());
        if let Some(mut execution) = self.execution.take() {
            execution.finish()?;
        }
        Ok(())
    }
}
impl Drop for TurnResources {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

enum TurnEnd {
    Exited { success: bool, code: Option<i32> },
    Cancelled,
    SpawnFailed(String),
}

/// Numbers and publishes the run's events in order.
struct RunFrames<'a, S: FrameSink> {
    sink: &'a S,
    session_id: &'a str,
    run_id: &'a str,
    sequence: i64,
}

impl<S: FrameSink> RunFrames<'_, S> {
    async fn emit(&mut self, method: &str, params: Value) {
        if matches!(method, "run.completed" | "run.failed" | "run.cancelled") {
            self.cleanup_diagnostic(&params).await;
        }
        self.publish(method, params).await;
    }
    async fn cleanup_diagnostic(&mut self, params: &Value) {
        if let Some(error) = params.get("cleanupError").and_then(Value::as_str) {
            self.publish("run.diagnostic", json!({"message": format!(
                "Workspace MCP cleanup needs recovery: {error} Inspect the retained workspace configuration before retrying."
            )})).await;
        }
    }
    async fn publish(&mut self, method: &str, params: Value) {
        self.sequence += 1;
        self.sink
            .publish(RpcFrame {
                jsonrpc: "2.0".into(),
                protocol_version: PROTOCOL_VERSION,
                session_id: self.session_id.to_string(),
                run_id: self.run_id.to_string(),
                sequence: self.sequence,
                id: None,
                event_id: Some(Uuid::new_v4().to_string()),
                method: Some(method.to_string()),
                params: Some(params),
                result: None,
                error: None,
            })
            .await;
    }
}

/// Runs one turn to its end and publishes `run.started`, the stream, and
/// exactly one terminal event (`run.completed`, `run.failed`, or
/// `run.cancelled`). `cancel` turning `true` stops the CLI's whole process
/// group.
pub async fn execute_turn<S: FrameSink>(
    repository: &AgentRepository,
    sink: &S,
    context: TurnContext,
    mut cancel: watch::Receiver<bool>,
) {
    let mut state = TurnState {
        frames: RunFrames {
            sink,
            session_id: &context.session_id,
            run_id: &context.run_id,
            sequence: 0,
        },
        repository,
        context: &context,
        text: String::new(),
        reported_failure: None,
        conversation_saved: false,
        runtime_profile_id: None,
    };
    let frames = &mut state.frames;
    let name = context.kind.display_name();
    frames
        .emit(
            "run.started",
            json!({ "model": engine_model_id(context.kind) }),
        )
        .await;

    if let Some(previous_run_id) = context.previous_run_id.as_deref() {
        if let Err(error) = repository
            .set_run_retry_parent(&context.run_id, previous_run_id)
            .await
        {
            tracing::warn!(%error, "could not retain CLI retry ancestry");
            frames
                .emit(
                    "run.failed",
                    failure("Could not prepare the retried conversation."),
                )
                .await;
            return;
        }
    }

    let mut resources = TurnResources::default();
    if context.kind == CliKind::Agy {
        let prepared = context
            .agy
            .as_ref()
            .ok_or_else(|| std::io::Error::other("Clovy's Antigravity profile context is missing."))
            .and_then(|host| host.chat(&context.session_id));
        match prepared {
            Ok(execution) => {
                state.runtime_profile_id = Some(execution.profile_id().to_string());
                resources.execution = Some(execution);
            }
            Err(error) => {
                frames
                    .emit(
                        "run.failed",
                        failure(&format!("Could not prepare {name}: {error}")),
                    )
                    .await;
                return;
            }
        }
    }
    let resumed = match previous_conversation(
        repository,
        &context,
        state.runtime_profile_id.as_deref(),
    )
    .await
    {
        Ok(found) => found,
        Err(error) => {
            tracing::warn!(%error, run_id = %context.run_id, "could not read the previous CLI conversation id");
            None
        }
    };
    let prompt = if resumed.is_some() {
        context.input.clone()
    } else {
        with_earlier_messages(repository, &context).await
    };
    let conversation = resumed
        .or_else(|| cli::preassigns_conversation(context.kind).then(|| Uuid::new_v4().to_string()));
    if let Some(id) = conversation.as_deref() {
        state.conversation_saved = save_conversation(
            repository,
            &context,
            id,
            state.runtime_profile_id.as_deref(),
        )
        .await;
    }
    let frames = &mut state.frames;

    let scratch = match tempfile::Builder::new().prefix("clovy-chat-").tempdir() {
        Ok(scratch) => scratch,
        Err(error) => {
            frames
                .emit(
                    "run.failed",
                    failure(&format!("Could not prepare {name}: {error}")),
                )
                .await;
            return;
        }
    };
    // The token lives exactly as long as this turn: every return below
    // drops it, which revokes it.
    let mut wire_names = None;
    let mcp = context
        .mcp
        .as_ref()
        .filter(|_| cli::accepts_mcp(context.kind))
        .map(|launch| match &context.turns {
            Some(turns) => {
                let mut binding = context.binding();
                if matches!(
                    context.kind,
                    CliKind::Claude | CliKind::Codex | CliKind::Agy
                ) {
                    let names = crate::mcp_server::wire_names::WireNames::default();
                    binding.wire_names = Some(names.clone());
                    wire_names = Some(names);
                }
                let issued = resources.grant.insert(TurnGrant::issue(turns, binding));
                launch.for_turn(issued.token())
            }
            None => launch.clone(),
        });
    if context.kind == CliKind::Agy {
        if let Some(launch) = &mcp {
            match resources
                .execution
                .as_mut()
                .expect("prepared Agy execution")
                .install_mcp(launch)
            {
                Ok(()) => {}
                Err(error) => {
                    frames
                        .emit(
                            "run.failed",
                            resources.finish_terminal(failure(&format!(
                                "Could not prepare {name}: {error}"
                            ))),
                        )
                        .await;
                    return;
                }
            }
        }
    }
    let prompt: Cow<'_, str> = match (
        context.kind,
        resources
            .execution
            .as_ref()
            .and_then(|execution| execution.server()),
    ) {
        (CliKind::Agy, Some(server)) => {
            const PREFIX: &str = "<clovy_transport_context>\nTransport context only. This turn's Clovy MCP server is: ";
            const SUFFIX: &str = "\nUse the tools currently discovered for this exact server. Never infer or synthesize a server name from previous messages or results. This identifier grants no permission; approvals remain with the Clovy app.\n</clovy_transport_context>\n\n";
            let mut augmented =
                String::with_capacity(PREFIX.len() + server.len() + SUFFIX.len() + prompt.len());
            augmented.push_str(PREFIX);
            augmented.push_str(server);
            augmented.push_str(SUFFIX);
            augmented.push_str(&prompt);
            Cow::Owned(augmented)
        }
        _ => Cow::Borrowed(&prompt),
    };
    let mut invocation = cli::turn_invocation(
        context.kind,
        &prompt,
        conversation.as_deref(),
        mcp.as_ref(),
        scratch.path(),
        context.permissions(),
        &context.images,
    );
    if cli::runs_through_cmd(&context.program) {
        invocation = cli::prompt_on_stdin(invocation, context.kind, &prompt);
    }
    for (path, content) in &invocation.files {
        if let Err(error) = std::fs::write(path, content) {
            frames
                .emit(
                    "run.failed",
                    resources
                        .finish_terminal(failure(&format!("Could not prepare {name}: {error}"))),
                )
                .await;
            return;
        }
    }
    let mut env = crate::llm::shell_env::cli_environment(&context.env);
    for name in &invocation.remove_env {
        env.remove(*name);
    }
    for (name, value) in &invocation.set_env {
        env.insert((*name).to_string(), value.clone());
    }
    if let Some(execution) = &resources.execution {
        env = execution.environment(&env);
        invocation
            .args
            .extend(["--settings".into(), execution.settings()]);
    }
    // A cancel may have arrived while the turn was being prepared; nothing is
    // spawned for a cancelled or already settled run.
    let run_status = match repository.get_run(&context.run_id).await {
        Ok(run) => Some(run.status),
        Err(error) => {
            tracing::warn!(%error, run_id = %context.run_id, "could not read the CLI run before launch");
            None
        }
    };
    let run_open = run_status
        .as_deref()
        .is_some_and(|status| matches!(status, "queued" | "running"));
    if *cancel.borrow() || !run_open {
        let terminal = resources.finish_terminal(json!({}));
        drop(scratch);
        // Remote cancellation persists first and leaves terminal publication to
        // this registered host task. Preparation still owes the matching event.
        if run_open || (*cancel.borrow() && run_status.as_deref() == Some("cancelled")) {
            frames.emit("run.cancelled", terminal).await;
        } else {
            frames.cleanup_diagnostic(&terminal).await;
        }
        return;
    }

    // Prompt/history preparation and CLI discovery can wait while the user
    // moves the conversation. Never send the prepared context to that CLI
    // under a different partition, even before its first MCP call.
    let partition = crate::agent_sessions::run_partition::revalidate(
        &repository.pool,
        &context.session_id,
        &context.run_id,
        &context.data_partition,
    )
    .await;
    // This authorization query is the last asynchronous preparation before
    // spawn and stdin. Stop can win after SQLite read an active snapshot,
    // including when the query itself returns an error.
    if *cancel.borrow() {
        frames
            .emit("run.cancelled", resources.finish_terminal(json!({})))
            .await;
        return;
    }
    if let Err(error) = partition {
        frames
            .emit(
                "run.failed",
                resources.finish_terminal(failure(&error.message)),
            )
            .await;
        return;
    }

    let mut translator = StreamTranslator::new(context.kind)
        .with_turn_mcp_server(
            resources
                .execution
                .as_ref()
                .and_then(|execution| execution.server()),
        )
        .with_wire_names(wire_names);
    if let Some(execution) = &resources.execution {
        if let Err(error) = execution.validate() {
            frames
                .emit(
                    "run.failed",
                    resources.finish_terminal(failure(&error.to_string())),
                )
                .await;
            return;
        }
    }
    let mut stderr_tail = String::new();
    let end = run_process(
        &invocation,
        env,
        resources
            .execution
            .as_ref()
            .map(|execution| execution.cwd())
            .unwrap_or(&context.workspace),
        &mut translator,
        &mut cancel,
        &mut stderr_tail,
        &mut state,
    )
    .await;
    drop(scratch);
    let cleanup_error = resources.finish().err().map(|error| error.to_string());

    let TurnState {
        mut frames,
        text,
        reported_failure,
        ..
    } = state;
    if !text.is_empty() {
        frames
            .emit("message.completed", json!({ "text": text }))
            .await;
    }
    match end {
        TurnEnd::Cancelled => {
            frames
                .emit("run.cancelled", json!({"cleanupError": cleanup_error}))
                .await
        }
        TurnEnd::SpawnFailed(error) => {
            let mut params = failure(&format!(
                "Could not start {name}: {error}{}",
                cleanup_error
                    .as_ref()
                    .map(|error| format!(" Workspace MCP cleanup: {error}"))
                    .unwrap_or_default()
            ));
            params["cleanupError"] = json!(cleanup_error);
            frames.emit("run.failed", params).await
        }
        TurnEnd::Exited { success, code } => {
            let failed = reported_failure
                .or_else(|| translator.unresolved_failure())
                .or_else(|| {
                    (!success).then(|| {
                        last_line(&stderr_tail).map_or_else(
                            || match code {
                                Some(code) => format!("{name} exited with code {code}."),
                                None => format!("{name} stopped unexpectedly."),
                            },
                            |line| format!("{name}: {line}"),
                        )
                    })
                });
            let cleanup_diagnostic = cleanup_error.clone();
            let failed = match (failed, cleanup_error) {
                (Some(error), Some(cleanup)) => {
                    Some(format!("{error} Workspace MCP cleanup: {cleanup}"))
                }
                (error, cleanup) => error.or(cleanup),
            };
            // A Windows sandbox failure may reach only stderr; it says more
            // than the stream's own "ended without completing".
            let sandbox_line = last_line(&stderr_tail)
                .filter(|line| cli::is_codex_windows_sandbox_failure(line))
                .map(str::to_string);
            let failed = failed.map(|message| match context.kind {
                CliKind::Codex => match &sandbox_line {
                    Some(line) if !cli::is_codex_windows_sandbox_failure(&message) => {
                        cli::with_codex_sandbox_hint(line)
                    }
                    _ => cli::with_codex_sandbox_hint(&message),
                },
                _ => message,
            });
            match failed {
                Some(message) => {
                    let mut params = failure(&message);
                    params["cleanupError"] = json!(cleanup_diagnostic);
                    frames.emit("run.failed", params).await;
                }
                None => frames.emit("run.completed", json!({})).await,
            }
        }
    }
}

fn failure(message: &str) -> Value {
    json!({
        "error": message,
        "category": "provider",
        "code": "agent_cli_failed",
        "retryable": true,
    })
}

fn last_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).rfind(|line| !line.is_empty())
}

async fn run_process<S: FrameSink>(
    invocation: &cli::TurnInvocation,
    env: BTreeMap<String, String>,
    cwd: &std::path::Path,
    translator: &mut StreamTranslator,
    cancel: &mut watch::Receiver<bool>,
    stderr_tail: &mut String,
    state: &mut TurnState<'_, S>,
) -> TurnEnd {
    let context = state.context;
    let program = if context.kind == CliKind::Agy {
        match std::fs::canonicalize(&context.program) {
            Ok(program) => program,
            Err(error) => return TurnEnd::SpawnFailed(error.to_string()),
        }
    } else {
        context.program.clone()
    };
    if let Err(crate::llm::LlmError::CliFailed { detail, .. }) =
        crate::llm::cli::check_command_line(context.kind, &program, &invocation.args, cfg!(windows))
    {
        return TurnEnd::SpawnFailed(detail);
    }
    let mut command = crate::hide_console::tokio_command(&program);
    command
        .args(&invocation.args)
        .env_clear()
        .envs(&env)
        .current_dir(cwd)
        .stdin(if invocation.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let (mut child, tree) = match ProcessTree::spawn(&mut command) {
        Ok(spawned) => spawned,
        Err(error) => return TurnEnd::SpawnFailed(error.to_string()),
    };
    #[cfg(test)]
    testing::record_spawn(&context.run_id, child.id());
    if let (Some(input), Some(mut stdin)) = (invocation.stdin.clone(), child.stdin.take()) {
        tokio::spawn(async move {
            let _ = stdin.write_all(input.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let stderr_task = child.stderr.take().map(|stderr| {
        tokio::spawn(async move {
            let mut reader = stderr;
            let mut kept = Vec::new();
            let mut buffer = [0_u8; 8 * 1024];
            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => {
                        kept.extend_from_slice(&buffer[..read]);
                        if kept.len() > STDERR_TAIL_BYTES {
                            kept.drain(..kept.len() - STDERR_TAIL_BYTES);
                        }
                    }
                }
            }
            String::from_utf8_lossy(&kept).into_owned()
        })
    });
    let Some(stdout) = child.stdout.take() else {
        stop_group(&mut child, &tree).await;
        return TurnEnd::SpawnFailed("stdout was unavailable".into());
    };
    let mut reader = BufReader::new(stdout);
    let mut line = Vec::new();
    let mut oversized = false;
    // The CLI's exit is watched alongside its stdout: a helper that inherited
    // stdout can keep the pipe open after the CLI is gone.
    let mut exited = None;
    loop {
        tokio::select! {
            biased;
            () = cancelled(cancel) => {
                stop_group(&mut child, &tree).await;
                return TurnEnd::Cancelled;
            }
            status = child.wait() => {
                exited = Some(status);
                break;
            }
            read = read_line_capped(&mut reader, &mut line, &mut oversized) => {
                match read {
                    Ok(true) => {
                        if !oversized {
                            let text_line = String::from_utf8_lossy(&line).into_owned();
                            for event in translator.translate_line(&text_line) {
                                state.apply(event).await;
                            }
                        }
                        line.clear();
                        oversized = false;
                    }
                    Ok(false) | Err(_) => break,
                }
            }
        }
    }
    let status = match exited {
        Some(status) => status,
        None => tokio::select! {
            biased;
            () = cancelled(cancel) => {
                stop_group(&mut child, &tree).await;
                return TurnEnd::Cancelled;
            }
            status = child.wait() => status,
        },
    };
    // Sweep helpers the CLI left in its group (MCP servers it spawned), then
    // read what is still buffered in the pipe, for a bounded time.
    tree.signal(GroupSignal::Kill);
    let drain_until = tokio::time::Instant::now() + OUTPUT_DRAIN_GRACE;
    while let Ok(Ok(true)) = tokio::time::timeout_at(
        drain_until,
        read_line_capped(&mut reader, &mut line, &mut oversized),
    )
    .await
    {
        if !oversized {
            let text_line = String::from_utf8_lossy(&line).into_owned();
            for event in translator.translate_line(&text_line) {
                state.apply(event).await;
            }
        }
        line.clear();
        oversized = false;
    }
    if let Some(task) = stderr_task {
        if let Ok(Ok(tail)) = tokio::time::timeout(OUTPUT_DRAIN_GRACE, task).await {
            *stderr_tail = tail;
        }
    }
    match status {
        Ok(status) => TurnEnd::Exited {
            success: status.success(),
            code: status.code(),
        },
        Err(error) => TurnEnd::SpawnFailed(error.to_string()),
    }
}

/// The run's frames and what the turn has gathered so far.
struct TurnState<'a, S: FrameSink> {
    frames: RunFrames<'a, S>,
    repository: &'a AgentRepository,
    context: &'a TurnContext,
    /// The whole answer, published again as `message.completed`.
    text: String,
    reported_failure: Option<String>,
    conversation_saved: bool,
    runtime_profile_id: Option<String>,
}

impl<S: FrameSink> TurnState<'_, S> {
    async fn apply(&mut self, event: EngineEvent) {
        let name = self.context.kind.display_name();
        match event {
            EngineEvent::Conversation(id) => {
                if !self.conversation_saved {
                    self.conversation_saved = save_conversation(
                        self.repository,
                        self.context,
                        &id,
                        self.runtime_profile_id.as_deref(),
                    )
                    .await;
                }
            }
            EngineEvent::TextDelta(delta) => {
                self.text.push_str(&delta);
                self.frames
                    .emit("message.delta", json!({ "delta": delta }))
                    .await;
            }
            EngineEvent::Text(block) => {
                let delta = if self.text.is_empty() || self.text.ends_with("\n\n") {
                    block
                } else if self.text.ends_with('\n') {
                    format!("\n{block}")
                } else {
                    format!("\n\n{block}")
                };
                self.text.push_str(&delta);
                self.frames
                    .emit("message.delta", json!({ "delta": delta }))
                    .await;
            }
            EngineEvent::Reasoning(delta) => {
                self.frames
                    .emit("reasoning.delta", json!({ "delta": delta }))
                    .await;
            }
            EngineEvent::ToolStarted {
                call_id,
                name: tool,
                arguments,
            } => {
                self.frames
                    .emit(
                        "tool.started",
                        json!({ "callId": call_id, "name": tool, "arguments": arguments }),
                    )
                    .await;
            }
            EngineEvent::ToolFinished {
                call_id,
                name: tool,
                output,
                error: Some(error),
                needs_approval,
            } => {
                let error = if needs_approval {
                    format!(
                        "{name} needs your approval for this action, and a chat turn cannot ask for it. Allow it in {name}'s own settings to let it run. ({})",
                        error.trim()
                    )
                } else {
                    error
                };
                self.frames
                    .emit(
                        "tool.failed",
                        json!({ "callId": call_id, "name": tool, "error": error, "needsApproval": needs_approval,
                            "output": if output.is_null() { Value::String(error.clone()) } else { output } }),
                    )
                    .await;
            }
            EngineEvent::ToolFinished {
                call_id,
                name: tool,
                output,
                error: None,
                ..
            } => {
                self.frames
                    .emit(
                        "tool.completed",
                        json!({ "callId": call_id, "name": tool, "output": output }),
                    )
                    .await;
            }
            EngineEvent::Failed(message) => {
                self.reported_failure.get_or_insert(message);
            }
        }
    }
}

/// Reads one LF-terminated line into `line` (without the LF). Bytes past
/// [`MAX_LINE_BYTES`] are dropped and `oversized` is set. `Ok(false)` at EOF
/// with nothing read.
async fn read_line_capped<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    line: &mut Vec<u8>,
    oversized: &mut bool,
) -> std::io::Result<bool> {
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(!line.is_empty() || *oversized);
        }
        let (chunk, done) = match available.iter().position(|byte| *byte == b'\n') {
            Some(end) => (&available[..end], Some(end + 1)),
            None => (available, None),
        };
        if line.len() + chunk.len() > MAX_LINE_BYTES {
            *oversized = true;
        } else if !*oversized {
            line.extend_from_slice(chunk);
        }
        let consumed = done.unwrap_or(available.len());
        reader.consume(consumed);
        if done.is_some() {
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(true);
        }
    }
}

async fn cancelled(cancel: &mut watch::Receiver<bool>) {
    loop {
        if *cancel.borrow_and_update() {
            return;
        }
        if cancel.changed().await.is_err() {
            // Nobody can cancel any more; never resolve.
            std::future::pending::<()>().await;
        }
    }
}

/// SIGTERM to the whole group (the CLI may save its session), then SIGKILL
/// after a short grace period; the child is reaped.
async fn stop_group(child: &mut tokio::process::Child, tree: &ProcessTree) {
    tree.signal(GroupSignal::Terminate);
    if tokio::time::timeout(CANCEL_GRACE, child.wait())
        .await
        .is_err()
    {
        let _ = child.start_kill();
    }
    tree.signal(GroupSignal::Kill);
    let _ = child.wait().await;
}

/// The CLI conversation to resume: the id saved by the latest earlier turn of
/// this session on the same CLI, as long as no turn on another engine came in
/// between (that CLI would not know those messages).
async fn previous_conversation(
    repository: &AgentRepository,
    context: &TurnContext,
    profile_id: Option<&str>,
) -> Result<Option<String>, sqlx::Error> {
    // A resume would restore the rejected answer from the CLI's own history,
    // even if it is excluded from the local replay below.
    if context.previous_run_id.is_some() {
        return Ok(None);
    }
    let engine = engine_model_id(context.kind);
    let rows = sqlx::query::query(
        "SELECT model, run_config_json FROM agent_runs
         WHERE session_id = ? AND id <> ?
         ORDER BY started_at DESC, rowid DESC",
    )
    .bind(&context.session_id)
    .bind(&context.run_id)
    .fetch_all(&repository.pool)
    .await?;
    for row in rows {
        if row.get::<String, _>("model") != engine {
            return Ok(None);
        }
        let Some(config) = row
            .get::<Option<String>, _>("run_config_json")
            .and_then(|config| serde_json::from_str::<Value>(&config).ok())
        else {
            continue;
        };
        if config.get("engine").and_then(Value::as_str) == Some("cli")
            && config.get("cli").and_then(Value::as_str) == Some(context.kind.id())
        {
            if context.kind == CliKind::Agy
                && (profile_id.is_none()
                    || config.get("cliProfileId").and_then(Value::as_str) != profile_id)
            {
                return Ok(None);
            }
            if let Some(id) = config.get("conversationId").and_then(Value::as_str) {
                return Ok(Some(id.to_string()));
            }
        }
        // A retry without a conversation id may have failed before startup.
        // Older CLI history still contains the answer that retry rejected.
        if config.get("retryOfRunId").and_then(Value::as_str).is_some() {
            return Ok(None);
        }
    }
    Ok(None)
}

/// Saves the CLI's conversation id on this run (once), for the next turn.
async fn save_conversation(
    repository: &AgentRepository,
    context: &TurnContext,
    conversation_id: &str,
    profile_id: Option<&str>,
) -> bool {
    match repository
        .set_cli_conversation(
            &context.run_id,
            context.kind.id(),
            conversation_id,
            profile_id,
        )
        .await
    {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, run_id = %context.run_id, "could not save the CLI conversation id");
            false
        }
    }
}

/// A CLI starting a new conversation in a session that already has messages
/// (another engine answered before, or the session was branched) gets them
/// as context ahead of the new message.
async fn with_earlier_messages(repository: &AgentRepository, context: &TurnContext) -> String {
    let items = match repository.items(&context.session_id).await {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(%error, "could not read earlier session messages");
            return context.input.clone();
        }
    };
    let excluded = match excluded_retry_runs(repository, context).await {
        Ok(excluded) => excluded,
        Err(error) => {
            tracing::warn!(%error, "could not read rejected retry history");
            return context.input.clone();
        }
    };
    // Results and starts are separate canonical items. Join only within the
    // exact persisted run/call/name; CLIs reuse step ids in later turns.
    let call_arguments: BTreeMap<_, _> = items
        .iter()
        .filter_map(|item| {
            let AgentItemPayload::ToolCall(tool) = &item.payload else {
                return None;
            };
            Some((
                (
                    item.run_id.as_deref()?,
                    tool.tool_call_id.as_deref()?,
                    tool.tool_name.as_deref(),
                ),
                tool.arguments.as_ref()?,
            ))
        })
        .collect();
    let mut lines = Vec::new();
    for item in &items {
        if item.run_id.as_deref() == Some(context.run_id.as_str())
            || item.run_id.as_ref().is_some_and(|id| excluded.contains(id))
        {
            continue;
        }
        match &item.payload {
            AgentItemPayload::UserMessage(message) => {
                let message = crate::agent_sessions::api::cli_message_with_attachments(
                    message.content.trim(),
                    &message.attachments,
                );
                if !message.trim().is_empty() {
                    lines.push(format!("User: {message}"));
                }
            }
            AgentItemPayload::AssistantMessage(message) => {
                let message = crate::agent_sessions::api::cli_message_with_attachments(
                    message.content.trim(),
                    &message.attachments,
                );
                if !message.trim().is_empty() {
                    lines.push(format!("Assistant: {message}"));
                }
            }
            AgentItemPayload::ToolResult(tool) | AgentItemPayload::ToolCall(tool)
                if matches!(tool.status.as_deref(), Some("complete" | "completed")) =>
            {
                let arguments = item
                    .run_id
                    .as_deref()
                    .zip(tool.tool_call_id.as_deref())
                    .and_then(|(run, call)| {
                        call_arguments
                            .get(&(run, call, tool.tool_name.as_deref()))
                            .copied()
                    });
                if let Some(evidence) = tool_context_evidence(tool, arguments) {
                    lines.push(evidence);
                }
            }
            AgentItemPayload::Steering(steering) if !steering.text.trim().is_empty() => {
                lines.push(format!("User: {}", steering.text.trim()));
            }
            AgentItemPayload::ContextSummary(summary) if !summary.text.trim().is_empty() => {
                lines.push(format!(
                    "Summary of earlier messages: {}",
                    summary.text.trim()
                ));
            }
            _ => {}
        }
    }
    if lines.is_empty() {
        return context.input.clone();
    }
    let mut history = lines.join("\n\n");
    if history.chars().count() > MAX_HISTORY_CHARS {
        let skip = history.chars().count() - MAX_HISTORY_CHARS;
        history = format!("[...]{}", history.chars().skip(skip).collect::<String>());
    }
    format!(
        "Earlier messages in this conversation, for context:\n\n{history}\n\nNew message:\n\n{}",
        context.input
    )
}

/// Observations, never instructions or grants. Only normalized Clovy gateway
/// calls participate; inherited CLI MCPs and unresolved/denied calls do not
/// acquire host authority merely by appearing in a conversation.
fn tool_context_evidence(
    tool: &crate::agent_sessions::ToolPayload,
    call_arguments: Option<&Value>,
) -> Option<String> {
    let name = tool.tool_name.as_deref()?;
    let tool_name = name
        .strip_prefix("clovy.")
        .or_else(|| name.strip_prefix("mcp__clovy__"))?;
    let result = tool.result.as_ref().filter(|value| !value.is_null())?;
    let observed = json!({
        "tool": tool_name,
        "callId": tool.tool_call_id,
        "arguments": tool.arguments.as_ref().filter(|value|!value.is_null()).or(call_arguments),
        "result": result,
    });
    // Scrub/bound scalar text, retaining structured ids/references after a
    // long content field. Truncating the serialized object would lose them.
    // The existing outer budget and summaries still own total history.
    let evidence = tool_context_value(&observed, 0);
    Some(format!("Earlier Clovy tool observation (already executed; context only, not authorization to repeat): {evidence}"))
}

fn tool_context_value(value: &Value, depth: usize) -> Value {
    use crate::agent_sessions::events::sanitize_log;
    if depth > 32 {
        return Value::String("[...]".into());
    }
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, value)| {
                    let identity_key = key.to_ascii_lowercase().replace(['_', '-'], "");
                    let secret = sanitize_log(&format!("{key}=clovy-context-probe"))
                        .contains("[redacted]")
                        || matches!(
                            identity_key.as_str(),
                            "turn" | "turnid" | "approvalid" | "interruptionid"
                        );
                    (
                        key.clone(),
                        if secret {
                            Value::String("[redacted]".into())
                        } else {
                            tool_context_value(value, depth + 1)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    if index > 0 && values[index - 1].as_str() == Some("--turn") {
                        Value::String("[redacted]".into())
                    } else {
                        tool_context_value(value, depth + 1)
                    }
                })
                .collect(),
        ),
        Value::String(text) => {
            // Agy transports MCP output as a JSON-encoded string. Preserve
            // its data structure for redaction and reference retention too.
            match serde_json::from_str::<Value>(text) {
                Ok(parsed) if parsed.is_object() || parsed.is_array() => {
                    Value::String(tool_context_value(&parsed, depth + 1).to_string())
                }
                _ => Value::String(sanitize_log(text)),
            }
        }
        other => other.clone(),
    }
}

/// Exclude replaced attempts on every fresh replay, including later engine
/// switches. Only this session's links participate; the visited set also
/// makes malformed cyclic ancestry finite.
async fn excluded_retry_runs(
    repository: &AgentRepository,
    context: &TurnContext,
) -> Result<HashSet<String>, sqlx::Error> {
    let rows =
        sqlx::query::query("SELECT id, run_config_json FROM agent_runs WHERE session_id = ?")
            .bind(&context.session_id)
            .fetch_all(&repository.pool)
            .await?;
    let mut parents = BTreeMap::new();
    for row in rows {
        let config = row
            .get::<Option<String>, _>("run_config_json")
            .map(|raw| {
                serde_json::from_str::<Value>(&raw)
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))
            })
            .transpose()?;
        let parent = config.and_then(|config| {
            config
                .get("retryOfRunId")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        parents.insert(row.get::<String, _>("id"), parent);
    }
    let mut pending = parents.values().flatten().cloned().collect::<Vec<_>>();
    pending.extend(context.previous_run_id.iter().cloned());
    let mut excluded = HashSet::new();
    while let Some(run_id) = pending.pop() {
        if !parents.contains_key(&run_id) || !excluded.insert(run_id.clone()) {
            continue;
        }
        if let Some(Some(parent)) = parents.get(&run_id) {
            pending.push(parent.clone());
        }
    }
    Ok(excluded)
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashMap;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex, OnceLock,
    };

    static SPAWNS: OnceLock<Mutex<HashMap<String, Arc<AtomicUsize>>>> = OnceLock::new();

    pub(crate) fn watch_spawns(run_id: &str) -> Arc<AtomicUsize> {
        let count = Arc::new(AtomicUsize::new(0));
        SPAWNS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .insert(run_id.to_owned(), count.clone());
        count
    }

    pub(crate) fn clear_spawns(run_id: &str) {
        SPAWNS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .remove(run_id);
    }

    pub(super) fn record_spawn(run_id: &str, pid: Option<u32>) {
        if let Some(count) = SPAWNS
            .get_or_init(Default::default)
            .lock()
            .unwrap()
            .get(run_id)
        {
            assert!(pid.is_some());
            count.fetch_add(1, Ordering::SeqCst);
        }
    }
}
