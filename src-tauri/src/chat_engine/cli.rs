//! Per-CLI turn contracts for the CLI chat engine: the argv, stdin, and
//! environment of one conversation turn (with the CLI's own resume id), and
//! the translation of each CLI's JSON stream into [`EngineEvent`]s.
//!
//! Formats measured on the installed CLIs (claude 2.1, codex 0.160, pi 1.0,
//! agy 1.2, cursor-agent 2026.09) and, for copilot (not installed), taken from
//! its programmatic reference (`--output-format json` session events):
//!
//! | CLI | Turn | Resume id | Clovy MCP server |
//! |---|---|---|---|
//! | claude | `-p --output-format stream-json --verbose --include-partial-messages`, prompt on stdin | `session_id` of `system/init`, `--resume <id>` | `--mcp-config <file>` |
//! | codex | `exec [resume] --json --skip-git-repo-check [--image <path>]... -c <policy>... [<id>] -`, prompt on stdin | `thread_id` of `thread.started` | `-c mcp_servers.clovy.*` |
//! | pi | `--print --mode json --session-id <id>`, prompt on stdin | chosen by Clovy (UUID) | no MCP support |
//! | agy | `--print=<prompt> --output-format stream-json` | `conversation_id` of `init`, `--conversation <id>` | workspace `.agents/mcp_config.json`, leased per turn |
//! | cursor-agent | `-p --output-format stream-json --stream-partial-output --trust -- <prompt>` | `session_id` of `system/init`, `--resume <id>` | project servers need approval in Cursor, not offered |
//! | copilot | `--prompt=<prompt> --output-format json --session-id <id>` | chosen by Clovy (UUID) | `--additional-mcp-config @<file>` |
//!
//! On Windows a `.cmd`/`.bat` launcher gives cursor-agent and copilot the
//! prompt on stdin instead ([`prompt_on_stdin`]).
//!
//! Permission flags, configuration overrides, and environment come from
//! [`super::policy`] (by CLI, safety mode, and whether a person watches the
//! turn); nothing here adds its own. No flag that skips or pre-grants
//! approvals is ever passed. An action the CLI refuses for lack of approval
//! comes back as a failed tool marked `needs_approval`.

use super::policy::{self, TurnPermissions};
use crate::llm::cli::CliKind;
use crate::mcp_server::{McpLaunch, SERVER_KEY};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// What a CLI stream means for the session.
#[derive(Clone, Debug, PartialEq)]
pub enum EngineEvent {
    /// The CLI's own conversation id, used to resume the next turn.
    Conversation(String),
    /// Streamed answer text.
    TextDelta(String),
    /// A complete answer block that was not streamed as deltas.
    Text(String),
    /// Streamed (or complete) reasoning text.
    Reasoning(String),
    ToolStarted {
        call_id: String,
        name: String,
        arguments: Value,
    },
    ToolFinished {
        call_id: String,
        name: String,
        output: Value,
        error: Option<String>,
        /// The CLI refused the action because it needs the user's approval,
        /// which a non-interactive turn cannot give.
        needs_approval: bool,
    },
    /// The CLI reported that the turn failed.
    Failed(String),
}

/// Whether Clovy can hand its MCP server to the CLI for one turn without
/// touching the CLI's own configuration.
pub fn accepts_mcp(kind: CliKind) -> bool {
    matches!(
        kind,
        CliKind::Claude | CliKind::Codex | CliKind::Agy | CliKind::Copilot
    )
}

/// CLIs whose conversation id Clovy chooses (a fresh UUID) on the first turn.
pub fn preassigns_conversation(kind: CliKind) -> bool {
    matches!(kind, CliKind::Pi | CliKind::Copilot)
}

/// CLIs whose tools Clovy can confine to the unattended set per run.
pub fn supports_unattended(kind: CliKind) -> bool {
    matches!(kind, CliKind::Claude | CliKind::Codex)
}

/// One turn, ready to spawn in the session workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnInvocation {
    pub args: Vec<String>,
    pub stdin: Option<String>,
    pub remove_env: Vec<&'static str>,
    /// Environment set on the CLI process (the policy's tool timeout).
    pub set_env: Vec<(&'static str, String)>,
    /// Files to write before spawning (MCP configuration), path and content.
    pub files: Vec<(PathBuf, String)>,
}

/// Builds one turn. `conversation` is the id to resume (claude, codex, agy,
/// cursor-agent) or the id Clovy chose (pi, copilot; always `Some` for them).
/// `mcp` is offered only to CLIs that [`accepts_mcp`]; configuration files go
/// to `scratch`. `permissions` select the policy ([`policy::turn_policy`]);
/// callers refuse unattended turns on CLIs that do not
/// [`supports_unattended`]. `images` reach the CLIs that take image files
/// ([`policy::image_args`]).
pub fn turn_invocation(
    kind: CliKind,
    prompt: &str,
    conversation: Option<&str>,
    mcp: Option<&McpLaunch>,
    scratch: &Path,
    permissions: TurnPermissions,
    images: &[PathBuf],
) -> TurnInvocation {
    let policy = policy::turn_policy(kind, permissions);
    let mut files = Vec::new();
    let mut remove_env = Vec::new();
    let mut stdin = None;
    let args = match kind {
        CliKind::Claude => {
            let mut args = strings(&[
                "-p",
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
            ]);
            if let Some(id) = conversation {
                args.extend(strings(&["--resume", id]));
            }
            if let Some(launch) = mcp {
                let file = scratch.join("clovy-mcp.json");
                files.push((
                    file.clone(),
                    crate::mcp_server::claude_code_config(launch).to_string(),
                ));
                args.push("--mcp-config".into());
                args.push(file.display().to_string());
                args.extend(policy.mcp_args.iter().cloned());
            }
            args.extend(policy.args.iter().cloned());
            // Same rule as one-shot calls: a stray API key in the profile
            // would move the user from their subscription to metered billing.
            remove_env.push("ANTHROPIC_API_KEY");
            stdin = Some(prompt.to_string());
            args
        }
        CliKind::Codex => {
            let mut args = vec!["exec".to_string()];
            if conversation.is_some() {
                args.push("resume".into());
            }
            args.extend(strings(&["--json", "--skip-git-repo-check"]));
            args.extend(policy::image_args(kind, images));
            args.extend(policy.args.iter().cloned());
            if let Some(launch) = mcp {
                args.push("-c".into());
                args.push(format!(
                    "mcp_servers.{SERVER_KEY}.command={}",
                    Value::String(launch.command.display().to_string())
                ));
                args.push("-c".into());
                args.push(format!(
                    "mcp_servers.{SERVER_KEY}.args={}",
                    json!(launch.args)
                ));
                args.extend(policy.mcp_args.iter().cloned());
            }
            if let Some(id) = conversation {
                args.push(id.to_string());
            }
            args.push("-".into());
            stdin = Some(prompt.to_string());
            args
        }
        CliKind::Pi => {
            let mut args = strings(&["--print", "--mode", "json"]);
            if let Some(id) = conversation {
                args.extend(strings(&["--session-id", id]));
            }
            stdin = Some(prompt.to_string());
            args
        }
        CliKind::Agy => {
            let mut args = vec![
                format!("--print={prompt}"),
                "--output-format".into(),
                "stream-json".into(),
            ];
            if let Some(id) = conversation {
                args.extend(strings(&["--conversation", id]));
            }
            args
        }
        CliKind::CursorAgent => {
            // `--trust` only trusts the session workspace, a directory Clovy
            // created for this session; it grants no tool permission.
            let mut args = strings(&[
                "-p",
                "--output-format",
                "stream-json",
                "--stream-partial-output",
                "--trust",
            ]);
            if let Some(id) = conversation {
                args.extend(strings(&["--resume", id]));
            }
            args.push("--".into());
            args.push(prompt.to_string());
            args
        }
        CliKind::Copilot => {
            let mut args = vec![
                format!("--prompt={prompt}"),
                "--output-format".into(),
                "json".into(),
            ];
            if let Some(id) = conversation {
                args.extend(strings(&["--session-id", id]));
            }
            if let Some(launch) = mcp {
                let file = scratch.join("clovy-mcp.json");
                let config = json!({
                    "mcpServers": {
                        SERVER_KEY: {
                            "type": "local",
                            "command": launch.command.display().to_string(),
                            "args": launch.args,
                            "tools": ["*"],
                        }
                    }
                });
                files.push((file.clone(), config.to_string()));
                args.push(format!("--additional-mcp-config=@{}", file.display()));
            }
            args
        }
    };
    TurnInvocation {
        args,
        stdin,
        remove_env,
        set_env: policy.env,
        files,
    }
}

/// Whether `program` is a Windows batch file (`.cmd`, `.bat`): npm shims and
/// the Cursor agent launcher. Windows starts those through `cmd.exe`, which
/// cannot carry an argument with a line break (the spawn fails) or a command
/// line over 8191 characters, so their prompt goes on stdin
/// ([`prompt_on_stdin`]).
pub fn runs_through_cmd(program: &Path) -> bool {
    cfg!(windows)
        && program
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
            })
}

/// Moves the prompt of a CLI that takes it as an argument to stdin, for CLIs
/// that read it there: cursor-agent reads stdin when `-p` has no prompt
/// argument, and copilot takes a piped prompt in place of `--prompt`. agy has
/// no stdin prompt; its installer ships an `.exe`.
pub fn prompt_on_stdin(
    mut invocation: TurnInvocation,
    kind: CliKind,
    prompt: &str,
) -> TurnInvocation {
    let args = &mut invocation.args;
    match kind {
        CliKind::CursorAgent => {
            if let Some(at) = args.iter().position(|arg| arg == "--") {
                if args.get(at + 1).map(String::as_str) == Some(prompt) {
                    args.drain(at..at + 2);
                    invocation.stdin = Some(prompt.to_string());
                }
            }
        }
        CliKind::Copilot => {
            let flag = format!("--prompt={prompt}");
            if let Some(at) = args.iter().position(|arg| *arg == flag) {
                args.remove(at);
                invocation.stdin = Some(prompt.to_string());
            }
        }
        _ => {}
    }
    invocation
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

/// Turns one CLI's JSON lines into [`EngineEvent`]s.
pub struct StreamTranslator {
    kind: CliKind,
    tool_names: HashMap<String, String>,
    started_tools: HashSet<String>,
    /// Text of the current message arrived as deltas (claude, pi).
    text_streamed: bool,
    reasoning_streamed: bool,
    /// cursor-agent: text arrived as deltas since the last tool call, so the
    /// whole-segment copies that follow are dropped.
    segment_streamed: bool,
    /// copilot: message and reasoning ids that streamed deltas.
    streamed_ids: HashSet<String>,
    /// pi and codex report failures that a retry may still recover from.
    pending_failure: Option<String>,
    completed: bool,
    /// Only this workspace lease's server can claim Clovy ownership in Agy.
    turn_mcp_server: Option<String>,
    wire_names: Option<crate::mcp_server::wire_names::WireNames>,
    agy_finished_tools: HashSet<String>,
    agy_text: String,
}

impl StreamTranslator {
    pub fn new(kind: CliKind) -> Self {
        Self {
            kind,
            tool_names: HashMap::new(),
            started_tools: HashSet::new(),
            text_streamed: false,
            reasoning_streamed: false,
            segment_streamed: false,
            streamed_ids: HashSet::new(),
            pending_failure: None,
            completed: false,
            turn_mcp_server: None,
            wire_names: None,
            agy_finished_tools: HashSet::new(),
            agy_text: String::new(),
        }
    }

    pub fn with_turn_mcp_server(mut self, server: Option<&str>) -> Self {
        self.turn_mcp_server = server.map(str::to_string);
        self
    }

    pub fn with_wire_names(
        mut self,
        names: Option<crate::mcp_server::wire_names::WireNames>,
    ) -> Self {
        self.wire_names = names;
        self
    }

    /// Called only after the transport's server identity has been checked.
    /// Cached names already contain the canonical identity and never reenter
    /// this decoder when the corresponding result arrives.
    fn managed_tool_name(&self, tool: &str, prefix: &str) -> Option<String> {
        match &self.wire_names {
            Some(names) => names
                .resolve(tool)
                .map(|canonical| format!("{prefix}{}", canonical.as_ref())),
            None => Some(format!("{prefix}{tool}")),
        }
    }

    fn claude_tool_name(&self, observed: &str) -> String {
        match observed
            .strip_prefix("mcp__")
            .and_then(|name| name.split_once("__"))
        {
            Some((SERVER_KEY, tool)) => self
                .managed_tool_name(tool, "mcp__clovy__")
                .unwrap_or_else(|| format!("external_mcp.{SERVER_KEY}.{tool}")),
            _ => observed.to_string(),
        }
    }

    fn codex_tool(&self, item: &Value) -> Option<(String, Value)> {
        if str_at(item, "type") == Some("mcp_tool_call")
            && str_at(item, "server") == Some(SERVER_KEY)
        {
            let tool = str_at(item, "tool").unwrap_or("tool");
            return Some((
                self.managed_tool_name(tool, "clovy.")
                    .unwrap_or_else(|| format!("external_mcp.{SERVER_KEY}.{tool}")),
                item["arguments"].clone(),
            ));
        }
        codex_tool(item)
    }

    fn codex_mcp_error(&self, item: &Value, name: &str, error: &Value) -> String {
        if self.wire_names.is_none() {
            return codex_mcp_error(item, error);
        }
        let server = str_at(item, "server").unwrap_or("unknown");
        let owned = server == SERVER_KEY && name.starts_with("clovy.");
        let tool = if owned {
            name.strip_prefix("clovy.").unwrap_or("unknown")
        } else {
            str_at(item, "tool").unwrap_or("unknown")
        };
        cli_mcp_error_from(server, tool, error, "The tool call failed.", owned)
    }

    /// Translates one stdout line; lines that are not JSON objects (warnings
    /// some CLIs print) are ignored.
    pub fn translate_line(&mut self, line: &str) -> Vec<EngineEvent> {
        let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
            return Vec::new();
        };
        if !event.is_object() {
            return Vec::new();
        }
        let mut out = Vec::new();
        match self.kind {
            CliKind::Claude => self.claude(&event, &mut out),
            CliKind::Codex => self.codex(&event, &mut out),
            CliKind::Pi => self.pi(&event, &mut out),
            CliKind::Agy => self.agy(&event, &mut out),
            CliKind::CursorAgent => self.cursor(&event, &mut out),
            CliKind::Copilot => self.copilot(&event, &mut out),
        }
        out
    }

    /// A failure that the stream reported without a final verdict (codex
    /// `error` without `turn.completed`, pi's last assistant message ending
    /// in error).
    pub fn unresolved_failure(&self) -> Option<String> {
        match self.kind {
            CliKind::Codex if !self.completed => Some(
                self.pending_failure
                    .clone()
                    .unwrap_or_else(|| "Codex ended without completing the turn.".into()),
            ),
            CliKind::Agy if !self.completed => Some(
                self.pending_failure
                    .clone()
                    .unwrap_or_else(|| "Antigravity ended without completing the turn.".into()),
            ),
            CliKind::Pi => self.pending_failure.clone(),
            _ => None,
        }
    }

    fn start_tool(
        &mut self,
        call_id: &str,
        name: &str,
        arguments: Value,
        out: &mut Vec<EngineEvent>,
    ) {
        if !self.started_tools.insert(call_id.to_string()) {
            return;
        }
        self.tool_names
            .insert(call_id.to_string(), name.to_string());
        out.push(EngineEvent::ToolStarted {
            call_id: call_id.to_string(),
            name: name.to_string(),
            arguments,
        });
    }

    fn finish_tool(
        &mut self,
        call_id: &str,
        name: Option<&str>,
        output: Value,
        error: Option<String>,
        needs_approval: bool,
        out: &mut Vec<EngineEvent>,
    ) {
        let name = name
            .map(str::to_string)
            .or_else(|| self.tool_names.get(call_id).cloned())
            .unwrap_or_else(|| "tool".into());
        if !self.started_tools.contains(call_id) {
            self.start_tool(call_id, &name, Value::Null, out);
        }
        out.push(EngineEvent::ToolFinished {
            call_id: call_id.to_string(),
            name,
            output,
            error,
            needs_approval,
        });
    }

    fn claude(&mut self, event: &Value, out: &mut Vec<EngineEvent>) {
        // Subagent (Task tool) traffic belongs to the tool call, not the answer.
        if event
            .get("parent_tool_use_id")
            .is_some_and(|parent| parent.is_string())
        {
            return;
        }
        match str_at(event, "type") {
            Some("system") if str_at(event, "subtype") == Some("init") => {
                if let Some(id) = str_at(event, "session_id") {
                    out.push(EngineEvent::Conversation(id.to_string()));
                }
            }
            Some("stream_event") => {
                let inner = &event["event"];
                match str_at(inner, "type") {
                    Some("message_start") => {
                        self.text_streamed = false;
                        self.reasoning_streamed = false;
                    }
                    Some("content_block_delta") => {
                        let delta = &inner["delta"];
                        match str_at(delta, "type") {
                            Some("text_delta") => {
                                if let Some(text) = non_empty(delta, "text") {
                                    self.text_streamed = true;
                                    out.push(EngineEvent::TextDelta(text.to_string()));
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(text) = non_empty(delta, "thinking") {
                                    self.reasoning_streamed = true;
                                    out.push(EngineEvent::Reasoning(text.to_string()));
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
            Some("assistant") => {
                for block in content_blocks(&event["message"]) {
                    match str_at(block, "type") {
                        Some("text") if !self.text_streamed => {
                            if let Some(text) = non_empty(block, "text") {
                                out.push(EngineEvent::Text(text.to_string()));
                            }
                        }
                        Some("thinking") if !self.reasoning_streamed => {
                            if let Some(text) = non_empty(block, "thinking") {
                                out.push(EngineEvent::Reasoning(text.to_string()));
                            }
                        }
                        Some("tool_use") => {
                            if let (Some(id), Some(name)) =
                                (str_at(block, "id"), str_at(block, "name"))
                            {
                                let name = self.claude_tool_name(name);
                                self.start_tool(id, &name, block["input"].clone(), out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("user") => {
                for block in content_blocks(&event["message"]) {
                    if str_at(block, "type") != Some("tool_result") {
                        continue;
                    }
                    let Some(id) = str_at(block, "tool_use_id") else {
                        continue;
                    };
                    let text = content_text(&block["content"]);
                    if block.get("is_error").and_then(Value::as_bool) == Some(true) {
                        // Measured: "Claude requested permissions to write to
                        // <path>, but you haven't granted it yet."
                        let needs_approval = text.contains("requested permissions");
                        let detail = self
                            .tool_names
                            .get(id)
                            .and_then(|name| claude_mcp_error(name, block, &text))
                            .unwrap_or(text);
                        self.finish_tool(id, None, Value::Null, Some(detail), needs_approval, out);
                    } else {
                        // Keep the real MCP content boundaries. The structured JSON
                        // block after a long screen tree is context too; flattening
                        // it into one string loses its provenance and references.
                        let clovy_mcp = self
                            .tool_names
                            .get(id)
                            .and_then(|name| name.strip_prefix("mcp__"))
                            .and_then(|name| name.split_once("__"))
                            .is_some_and(|(server, _)| server == SERVER_KEY);
                        let output = match &block["content"] {
                            Value::Array(blocks) if clovy_mcp => Value::Array(blocks.clone()),
                            _ => Value::String(text),
                        };
                        self.finish_tool(id, None, output, None, false, out);
                    }
                }
            }
            Some("result") => {
                let is_error = event.get("is_error").and_then(Value::as_bool) == Some(true);
                let subtype = str_at(event, "subtype");
                if is_error || !matches!(subtype, None | Some("success")) {
                    let detail = non_empty(event, "result")
                        .or(subtype)
                        .unwrap_or("Claude Code reported an error.");
                    out.push(EngineEvent::Failed(detail.to_string()));
                }
            }
            _ => {}
        }
    }

    fn codex(&mut self, event: &Value, out: &mut Vec<EngineEvent>) {
        match str_at(event, "type") {
            Some("thread.started") => {
                if let Some(id) = str_at(event, "thread_id") {
                    out.push(EngineEvent::Conversation(id.to_string()));
                }
            }
            Some("item.started") => {
                let item = &event["item"];
                if let (Some(id), Some((name, arguments))) =
                    (str_at(item, "id"), self.codex_tool(item))
                {
                    self.start_tool(id, &name, arguments, out);
                }
            }
            Some("item.completed") => {
                let item = &event["item"];
                let Some(id) = str_at(item, "id") else {
                    return;
                };
                match str_at(item, "type") {
                    Some("agent_message") => {
                        if let Some(text) = non_empty(item, "text") {
                            out.push(EngineEvent::Text(text.to_string()));
                        }
                    }
                    Some("reasoning") => {
                        if let Some(text) = non_empty(item, "text") {
                            out.push(EngineEvent::Reasoning(format!("{text}\n")));
                        }
                    }
                    _ => {
                        let Some((name, arguments)) = self.codex_tool(item) else {
                            return;
                        };
                        self.start_tool(id, &name, arguments, out);
                        let status = str_at(item, "status");
                        let (output, error, needs_approval) = match str_at(item, "type") {
                            Some("command_execution") => {
                                let output = str_at(item, "aggregated_output").unwrap_or_default();
                                let exit = item.get("exit_code").and_then(Value::as_i64);
                                let declined = status == Some("declined");
                                if declined
                                    || status == Some("failed")
                                    || exit.is_some_and(|code| code != 0)
                                {
                                    let detail = if output.trim().is_empty() {
                                        exit.map_or_else(
                                            || "The command did not run.".to_string(),
                                            |code| format!("Exit code {code}."),
                                        )
                                    } else {
                                        with_codex_sandbox_hint(output)
                                    };
                                    (Value::Null, Some(detail), declined)
                                } else {
                                    (Value::String(output.to_string()), None, false)
                                }
                            }
                            Some("mcp_tool_call") => {
                                match item.get("error").filter(|error| !error.is_null()) {
                                    Some(error) => (
                                        Value::Null,
                                        Some(self.codex_mcp_error(item, &name, error)),
                                        false,
                                    ),
                                    None if item["result"]["isError"].as_bool() == Some(true) => (
                                        Value::Null,
                                        Some(self.codex_mcp_error(item, &name, &item["result"])),
                                        false,
                                    ),
                                    None if status == Some("failed") => (
                                        Value::Null,
                                        Some(self.codex_mcp_error(item, &name, &Value::Null)),
                                        false,
                                    ),
                                    None => (item["result"].clone(), None, false),
                                }
                            }
                            _ if matches!(status, Some("failed" | "declined")) => (
                                Value::Null,
                                Some("The change was not applied.".into()),
                                status == Some("declined"),
                            ),
                            _ => (Value::Null, None, false),
                        };
                        self.finish_tool(id, Some(&name), output, error, needs_approval, out);
                    }
                }
            }
            Some("turn.completed") => {
                self.completed = true;
                self.pending_failure = None;
            }
            Some("turn.failed") => {
                let detail = event
                    .get("error")
                    .and_then(|error| str_at(error, "message"))
                    .unwrap_or("Codex could not complete the turn.");
                out.push(EngineEvent::Failed(detail.to_string()));
            }
            Some("error") => {
                // Also used for transient notices (reconnecting); final only
                // if the turn never completes.
                if let Some(message) = non_empty(event, "message") {
                    self.pending_failure = Some(message.to_string());
                }
            }
            _ => {}
        }
    }

    fn pi(&mut self, event: &Value, out: &mut Vec<EngineEvent>) {
        match str_at(event, "type") {
            Some("session") => {
                if let Some(id) = str_at(event, "id") {
                    out.push(EngineEvent::Conversation(id.to_string()));
                }
            }
            Some("message_start") if is_assistant(&event["message"]) => {
                self.text_streamed = false;
                self.reasoning_streamed = false;
            }
            Some("message_update") => {
                let update = &event["assistantMessageEvent"];
                match str_at(update, "type") {
                    Some("text_delta") => {
                        if let Some(text) = non_empty(update, "delta") {
                            self.text_streamed = true;
                            out.push(EngineEvent::TextDelta(text.to_string()));
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(text) = non_empty(update, "delta") {
                            self.reasoning_streamed = true;
                            out.push(EngineEvent::Reasoning(text.to_string()));
                        }
                    }
                    _ => {}
                }
            }
            Some("message_end") if is_assistant(&event["message"]) => {
                let message = &event["message"];
                if matches!(str_at(message, "stopReason"), Some("error" | "aborted")) {
                    self.pending_failure = Some(
                        non_empty(message, "errorMessage")
                            .unwrap_or("Pi stopped with an error.")
                            .to_string(),
                    );
                    return;
                }
                self.pending_failure = None;
                for block in content_blocks(message) {
                    match str_at(block, "type") {
                        Some("text") if !self.text_streamed => {
                            if let Some(text) = non_empty(block, "text") {
                                out.push(EngineEvent::Text(text.to_string()));
                            }
                        }
                        Some("thinking") if !self.reasoning_streamed => {
                            if let Some(text) = non_empty(block, "thinking") {
                                out.push(EngineEvent::Reasoning(text.to_string()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("tool_execution_start") => {
                if let (Some(id), Some(name)) =
                    (str_at(event, "toolCallId"), str_at(event, "toolName"))
                {
                    self.start_tool(id, name, event["args"].clone(), out);
                }
            }
            Some("tool_execution_end") => {
                if let Some(id) = str_at(event, "toolCallId") {
                    let result = &event["result"];
                    if event.get("isError").and_then(Value::as_bool) == Some(true) {
                        let detail = content_text(&result["content"]);
                        self.finish_tool(
                            id,
                            str_at(event, "toolName"),
                            Value::Null,
                            Some(detail),
                            false,
                            out,
                        );
                    } else {
                        let output = Value::String(content_text(&result["content"]));
                        self.finish_tool(id, str_at(event, "toolName"), output, None, false, out);
                    }
                }
            }
            _ => {}
        }
    }

    fn agy(&mut self, event: &Value, out: &mut Vec<EngineEvent>) {
        // Terminal records and repeated DONE updates must not duplicate a
        // result/effect in the common projection. An error remains an error.
        if self.completed {
            return;
        }
        match str_at(event, "event") {
            Some("init") => {
                if let Some(id) = str_at(event, "conversation_id") {
                    out.push(EngineEvent::Conversation(id.to_string()));
                }
            }
            Some("step_update") => {
                let step = &event["step_update"];
                match str_at(step, "step_type") {
                    Some("agent_response") => {
                        if let Some(text) = non_empty(step, "thinking_delta") {
                            out.push(EngineEvent::Reasoning(text.to_string()));
                        }
                        if let Some(text) = non_empty(step, "text_delta") {
                            self.agy_text.push_str(text);
                            out.push(EngineEvent::TextDelta(text.to_string()));
                        }
                    }
                    Some("tool") => {
                        let Some(index) = step.get("step_index").and_then(Value::as_u64) else {
                            out.push(EngineEvent::Failed(
                                "Antigravity omitted the tool step identity.".into(),
                            ));
                            return;
                        };
                        let call_id = format!("agy-step-{index}");
                        if self.agy_finished_tools.contains(&call_id) {
                            return;
                        }
                        let info = &step["tool_info"];
                        let wrapper = str_at(step, "tool_name")
                            .or_else(|| str_at(info, "name"))
                            .unwrap_or("tool");
                        let parameters = &info["parameters"];
                        let mcp = (wrapper == "call_mcp_tool")
                            .then(|| {
                                Some((
                                    str_at(parameters, "ServerName")?,
                                    str_at(parameters, "ToolName")?,
                                ))
                            })
                            .flatten();
                        let managed_name = mcp.and_then(|(server, tool)| {
                            (self.turn_mcp_server.as_deref() == Some(server))
                                .then(|| self.managed_tool_name(tool, "clovy."))
                                .flatten()
                        });
                        let owned = managed_name.is_some();
                        let name = match (mcp, managed_name) {
                            (Some(_), Some(name)) => name,
                            // Keep an explicit external origin, even if the
                            // inherited server itself calls itself `clovy`.
                            (Some((server, tool)), _) => format!("external_mcp.{server}.{tool}"),
                            (None, _) => wrapper.to_string(),
                        };
                        let arguments = if mcp.is_some() {
                            parameters["Arguments"].clone()
                        } else {
                            parameters.clone()
                        };
                        self.start_tool(&call_id, &name, arguments, out);
                        let state = str_at(step, "state");
                        if !matches!(state, Some("DONE" | "ERROR")) {
                            return;
                        }
                        let output = info
                            .get("result")
                            .or_else(|| info.get("output"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        // Agy can wrap an MCP error in a DONE step. Inspect the
                        // observed envelope, including its string encoding.
                        let envelope = output
                            .as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or_else(|| output.clone());
                        let error = if state == Some("ERROR") {
                            let detail = info
                                .get("error")
                                .and_then(|error| {
                                    str_at(error, "message").or_else(|| error.as_str())
                                })
                                .unwrap_or("The tool failed.");
                            Some(
                                if mcp.is_some() && detail.starts_with("permission check failed") {
                                    format!("Antigravity CLI permission check: {detail}")
                                } else {
                                    detail.to_string()
                                },
                            )
                        } else if envelope.get("isError").and_then(Value::as_bool) == Some(true) {
                            Some(match mcp {
                                Some((server, tool)) => cli_mcp_error_from(
                                    server,
                                    if owned {
                                        name.strip_prefix("clovy.").unwrap_or(tool)
                                    } else {
                                        tool
                                    },
                                    &envelope,
                                    "The tool call failed.",
                                    owned,
                                ),
                                None => content_text(&envelope["content"]),
                            })
                        } else {
                            None
                        };
                        // A host denial is final evidence, not a request to
                        // widen the CLI's permissions in its global settings.
                        let needs_approval = mcp.is_none()
                            && error
                                .as_ref()
                                .is_some_and(|error| error.contains("user denied permission"));
                        self.agy_finished_tools.insert(call_id.clone());
                        self.finish_tool(&call_id, Some(&name), output, error, needs_approval, out);
                    }
                    _ => {}
                }
            }
            Some("result") => {
                let result = &event["result"];
                if let Some(id) = str_at(result, "conversation_id") {
                    out.push(EngineEvent::Conversation(id.to_string()));
                }
                if let Some(response) = non_empty(result, "response") {
                    if self.agy_text.is_empty() {
                        out.push(EngineEvent::TextDelta(response.to_string()));
                        self.agy_text.push_str(response);
                    } else if let Some(suffix) = response
                        .strip_prefix(&self.agy_text)
                        .filter(|suffix| !suffix.is_empty())
                    {
                        out.push(EngineEvent::TextDelta(suffix.to_string()));
                        self.agy_text = response.to_string();
                    }
                }
                match str_at(result, "status") {
                    Some("SUCCESS") => {
                        self.completed = true;
                        if self.agy_text.trim().is_empty() {
                            let denied = result
                                .get("denied_actions")
                                .and_then(Value::as_array)
                                .filter(|actions| !actions.is_empty());
                            let failure=match denied {
                                Some(actions)=>format!("Antigravity reported SUCCESS without an answer after its headless permission check denied actions: {}. These are CLI denials, not a Clovy approval decision.",json!(actions)),
                                None=>"Antigravity completed without an answer.".into(),
                            };
                            out.push(EngineEvent::Failed(failure));
                        }
                    }
                    Some(status) => {
                        self.completed = true;
                        let detail = result
                            .get("error")
                            .and_then(|error| str_at(error, "message").or_else(|| error.as_str()))
                            .unwrap_or(status);
                        out.push(EngineEvent::Failed(detail.to_string()));
                    }
                    None => {
                        self.pending_failure =
                            Some("Antigravity returned a result without a terminal status.".into());
                    }
                }
            }
            _ => {}
        }
    }

    fn cursor(&mut self, event: &Value, out: &mut Vec<EngineEvent>) {
        match str_at(event, "type") {
            Some("system") if str_at(event, "subtype") == Some("init") => {
                if let Some(id) = str_at(event, "session_id") {
                    out.push(EngineEvent::Conversation(id.to_string()));
                }
            }
            Some("thinking") if str_at(event, "subtype") == Some("delta") => {
                if let Some(text) = non_empty(event, "text") {
                    out.push(EngineEvent::Reasoning(text.to_string()));
                }
            }
            Some("assistant") => {
                let text = content_blocks(&event["message"])
                    .filter(|block| str_at(block, "type") == Some("text"))
                    .filter_map(|block| str_at(block, "text"))
                    .collect::<String>();
                if text.is_empty() {
                    return;
                }
                // With --stream-partial-output text arrives as deltas
                // (`timestamp_ms`) and each segment again whole: before a tool
                // call as a record that also carries `model_call_id`, at the
                // end without `timestamp_ms`. A whole copy is shown only when
                // nothing was streamed since the last tool call.
                let aggregate =
                    event.get("model_call_id").is_some() || event.get("timestamp_ms").is_none();
                if !aggregate {
                    self.segment_streamed = true;
                    out.push(EngineEvent::TextDelta(text));
                } else if !self.segment_streamed {
                    out.push(EngineEvent::Text(text));
                }
            }
            Some("tool_call") => {
                let Some(call_id) = str_at(event, "call_id") else {
                    return;
                };
                let Some((key, call)) =
                    event
                        .get("tool_call")
                        .and_then(Value::as_object)
                        .and_then(|object| {
                            object
                                .iter()
                                .find(|(key, value)| key.ends_with("ToolCall") && value.is_object())
                        })
                else {
                    return;
                };
                let name = cursor_tool_name(key, call);
                match str_at(event, "subtype") {
                    Some("started") => {
                        self.segment_streamed = false;
                        self.start_tool(call_id, &name, call["args"].clone(), out);
                    }
                    Some("completed") => {
                        let result = &call["result"];
                        if let Some(success) = result.get("success") {
                            self.finish_tool(
                                call_id,
                                Some(&name),
                                success.clone(),
                                None,
                                false,
                                out,
                            );
                        } else if let Some(rejected) = result.get("rejected") {
                            let reason = non_empty(rejected, "reason")
                                .unwrap_or("Cursor Agent did not get approval to run this.");
                            self.finish_tool(
                                call_id,
                                Some(&name),
                                Value::Null,
                                Some(reason.to_string()),
                                true,
                                out,
                            );
                        } else {
                            let detail = result
                                .as_object()
                                .and_then(|object| {
                                    object.values().find_map(|value| {
                                        str_at(value, "error").or_else(|| str_at(value, "message"))
                                    })
                                })
                                .unwrap_or("The tool failed.");
                            self.finish_tool(
                                call_id,
                                Some(&name),
                                Value::Null,
                                Some(detail.to_string()),
                                false,
                                out,
                            );
                        }
                    }
                    _ => {}
                }
            }
            Some("result") if event.get("is_error").and_then(Value::as_bool) == Some(true) => {
                let detail =
                    non_empty(event, "result").unwrap_or("Cursor Agent reported an error.");
                out.push(EngineEvent::Failed(detail.to_string()));
            }
            _ => {}
        }
    }

    fn copilot(&mut self, event: &Value, out: &mut Vec<EngineEvent>) {
        let data = &event["data"];
        match str_at(event, "type") {
            Some("assistant.message_delta") => {
                if let Some(id) = str_at(data, "messageId") {
                    self.streamed_ids.insert(id.to_string());
                }
                if let Some(text) = non_empty(data, "deltaContent") {
                    out.push(EngineEvent::TextDelta(text.to_string()));
                }
            }
            Some("assistant.message") => {
                let streamed =
                    str_at(data, "messageId").is_some_and(|id| self.streamed_ids.contains(id));
                if !streamed {
                    let text = content_text(&data["content"]);
                    if !text.trim().is_empty() {
                        out.push(EngineEvent::Text(text));
                    }
                }
            }
            Some("assistant.reasoning_delta") => {
                if let Some(id) = str_at(data, "reasoningId") {
                    self.streamed_ids.insert(format!("reasoning:{id}"));
                }
                if let Some(text) = non_empty(data, "deltaContent") {
                    out.push(EngineEvent::Reasoning(text.to_string()));
                }
            }
            Some("assistant.reasoning") => {
                let streamed = str_at(data, "reasoningId")
                    .is_some_and(|id| self.streamed_ids.contains(&format!("reasoning:{id}")));
                if !streamed {
                    if let Some(text) = non_empty(data, "content") {
                        out.push(EngineEvent::Reasoning(text.to_string()));
                    }
                }
            }
            Some("tool.execution_start") => {
                if let (Some(id), Some(name)) =
                    (str_at(data, "toolCallId"), str_at(data, "toolName"))
                {
                    self.start_tool(id, name, data["arguments"].clone(), out);
                }
            }
            Some("tool.execution_complete") => {
                let Some(id) = str_at(data, "toolCallId") else {
                    return;
                };
                if data.get("success").and_then(Value::as_bool) == Some(false) {
                    let error = &data["error"];
                    let detail = str_at(error, "message")
                        .unwrap_or("The tool failed.")
                        .to_string();
                    let code = str_at(error, "code").unwrap_or_default();
                    let needs_approval = detail.to_ascii_lowercase().contains("permission")
                        || code.contains("denied")
                        || code.contains("permission");
                    self.finish_tool(id, None, Value::Null, Some(detail), needs_approval, out);
                } else {
                    let result = &data["result"];
                    let output = str_at(result, "detailedContent")
                        .map(str::to_string)
                        .unwrap_or_else(|| content_text(&result["content"]));
                    self.finish_tool(id, None, Value::String(output), None, false, out);
                }
            }
            Some("session.error") => {
                let detail = non_empty(data, "message").unwrap_or("Copilot reported an error.");
                out.push(EngineEvent::Failed(detail.to_string()));
            }
            Some("result") => {
                let exit = event
                    .get("exitCode")
                    .or_else(|| data.get("exitCode"))
                    .and_then(Value::as_i64);
                if exit.is_some_and(|code| code != 0) {
                    out.push(EngineEvent::Failed(format!(
                        "Copilot ended with exit code {}.",
                        exit.unwrap_or_default()
                    )));
                }
            }
            _ => {}
        }
    }
}

/// How codex-rs words a Windows sandbox failure: around a failed command
/// (`core/src/exec.rs`) and in the TUI's setup messages.
const CODEX_WINDOWS_SANDBOX_PHRASES: [&str; 4] = [
    "windows sandbox: ",
    "windows sandbox join error:",
    "invalid windows sandbox workspace roots:",
    "windows sandbox setup failed",
];

/// The setup codes of codex-rs `windows-sandbox-rs/src/setup_error.rs`.
const CODEX_WINDOWS_SANDBOX_SETUP_CODES: [&str; 28] = [
    "orchestrator_sandbox_dir_create_failed",
    "orchestrator_elevation_check_failed",
    "orchestrator_elevation_required",
    "orchestrator_payload_serialize_failed",
    "orchestrator_helper_launch_failed",
    "orchestrator_helper_launch_canceled",
    "orchestrator_helper_exit_nonzero",
    "orchestrator_helper_report_read_failed",
    "orchestrator_helper_incomplete",
    "helper_request_args_failed",
    "helper_sandbox_dir_create_failed",
    "helper_log_failed",
    "helper_user_provision_failed",
    "helper_users_group_create_failed",
    "helper_user_create_or_update_failed",
    "helper_dpapi_protect_failed",
    "helper_users_file_write_failed",
    "helper_setup_marker_write_failed",
    "helper_sid_resolve_failed",
    "helper_capability_sid_failed",
    "helper_firewall_com_init_failed",
    "helper_firewall_policy_access_failed",
    "helper_firewall_policy_ineffective",
    "helper_firewall_rule_create_or_add_failed",
    "helper_firewall_rule_verify_failed",
    "helper_read_acl_helper_spawn_failed",
    "helper_sandbox_lock_failed",
    "helper_unknown_error",
];

/// What Codex reports when its Windows sandbox is not set up or its setup
/// failed, on Windows only: one of codex-rs's own phrases, or a complete
/// setup code (not a word that merely starts like one).
pub(crate) fn codex_windows_sandbox_failure(text: &str, windows: bool) -> bool {
    if !windows {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    CODEX_WINDOWS_SANDBOX_PHRASES
        .iter()
        .any(|phrase| lower.contains(phrase))
        || lower
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .any(|word| CODEX_WINDOWS_SANDBOX_SETUP_CODES.contains(&word))
}

pub(crate) fn is_codex_windows_sandbox_failure(text: &str) -> bool {
    codex_windows_sandbox_failure(text, cfg!(windows))
}

/// Codex's own text, preceded by what to set up when it is a Windows
/// sandbox failure on Windows.
pub(crate) fn with_codex_sandbox_hint(text: &str) -> String {
    codex_sandbox_hint(text, cfg!(windows))
}

pub(crate) fn codex_sandbox_hint(text: &str, windows: bool) -> String {
    if !codex_windows_sandbox_failure(text, windows) {
        return text.to_string();
    }
    format!(
        "Codex could not use its Windows sandbox. In a terminal opened as administrator, run `codex sandbox setup --elevated --current-user` (it saves `sandbox = \"elevated\"` under `[windows]` in the Codex config), then try again. Codex said: {}",
        text.trim()
    )
}

fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn non_empty<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    str_at(value, key).filter(|text| !text.is_empty())
}

fn is_assistant(message: &Value) -> bool {
    str_at(message, "role") == Some("assistant")
}

fn content_blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// Text of a tool result or message content: a string, or the `text` of an
/// array of content blocks.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| str_at(block, "text"))
            .collect::<Vec<_>>()
            .join("\n"),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// codex `exec --json` items that are tool activity, as (name, arguments).
fn codex_tool(item: &Value) -> Option<(String, Value)> {
    match str_at(item, "type")? {
        "command_execution" => Some(("shell".into(), json!({ "command": item["command"] }))),
        "mcp_tool_call" => Some((
            format!(
                "{}.{}",
                str_at(item, "server").unwrap_or("mcp"),
                str_at(item, "tool").unwrap_or("tool")
            ),
            item["arguments"].clone(),
        )),
        "file_change" => Some(("apply_patch".into(), json!({ "changes": item["changes"] }))),
        "web_search" => Some(("web_search".into(), json!({ "query": item["query"] }))),
        _ => None,
    }
}

/// A CLI-owned MCP connection has its own credentials and catalog. Do not
/// infer that a failure belongs to an unrelated native Clovy provider.
fn codex_mcp_error(item: &Value, error: &Value) -> String {
    let server = str_at(item, "server").unwrap_or("unknown");
    let tool = str_at(item, "tool").unwrap_or("unknown");
    cli_mcp_error(server, tool, error, "The tool call failed.")
}

fn claude_mcp_error(name: &str, block: &Value, message: &str) -> Option<String> {
    if let Some((server, tool)) = name
        .strip_prefix("external_mcp.")
        .and_then(|name| name.split_once('.'))
    {
        return Some(cli_mcp_error_from(
            server,
            tool,
            &block["error"],
            message,
            false,
        ));
    }
    let (server, tool) = name.strip_prefix("mcp__")?.split_once("__")?;
    if server.is_empty() || tool.is_empty() {
        return None;
    }
    Some(cli_mcp_error(server, tool, &block["error"], message))
}

fn cli_mcp_error(server: &str, tool: &str, error: &Value, fallback: &str) -> String {
    cli_mcp_error_from(server, tool, error, fallback, server == SERVER_KEY)
}

fn cli_mcp_error_from(
    server: &str,
    tool: &str,
    error: &Value,
    fallback: &str,
    owned: bool,
) -> String {
    let origin = if owned {
        "clovy_gateway"
    } else {
        "external_cli"
    };
    // Only Clovy's relay owns these typed diagnostics. A CLI-inherited
    // server cannot claim Clovy ownership by returning the same metadata.
    if let Some(diagnostic) = error
        .pointer("/_meta/clovy/diagnostic")
        .filter(|_| owned)
        .and_then(|value| {
            serde_json::from_value::<crate::agent_mcp::AgentMcpDiagnostic>(value.clone()).ok()
        })
        .filter(|diagnostic| diagnostic.origin == "clovy_managed")
    {
        return diagnostic.into_app_error().message;
    }
    let content = content_text(&error["content"]);
    let message = str_at(error, "message")
        .or_else(|| error.as_str())
        .or_else(|| (!content.is_empty()).then_some(content.as_str()))
        .unwrap_or(fallback);
    let remote_code = error
        .get("code")
        .and_then(|value| match value {
            Value::String(code) => Some(code.clone()),
            Value::Number(code) => Some(code.to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "none".to_string());
    let action = if owned {
        "inspect_clovy_connection"
    } else if remote_code.eq_ignore_ascii_case("unauthorized")
        || matches!(remote_code.as_str(), "401" | "403")
    {
        "check_external_connection_and_permissions"
    } else if remote_code.eq_ignore_ascii_case("unknown_tool") || remote_code == "-32601" {
        "inspect_external_catalog"
    } else {
        "inspect_external_server"
    };
    format!("MCP origin: {origin}; server: {server}; tool: {tool}; phase: call; code: mcp_tool_failed; remote code: {remote_code}; state: unavailable; action: {action}. {message}")
}

/// `shellToolCall` -> `shell`; MCP calls carry the tool's own name.
fn cursor_tool_name(key: &str, call: &Value) -> String {
    let base = key.trim_end_matches("ToolCall");
    if base == "mcp" {
        let args = &call["args"];
        if let Some(tool) = str_at(args, "toolName").or_else(|| str_at(args, "name")) {
            return match str_at(args, "providerIdentifier") {
                Some(server) => format!("{server}.{tool}"),
                None => tool.to_string(),
            };
        }
    }
    base.to_string()
}

#[cfg(test)]
mod mcp_origin_tests {
    use super::*;

    fn transport_events(kind: CliKind, server: &str, tool: &str, result: Value) -> Vec<Value> {
        match kind {
            CliKind::Claude => vec![
                json!({"type":"assistant","message":{"content":[{
                    "type":"tool_use","id":"wire-call","name":format!("mcp__{server}__{tool}"),"input":{"id":"fixture"}
                }]}}),
                json!({"type":"user","message":{"content":[{
                    "type":"tool_result","tool_use_id":"wire-call", "is_error":result["isError"],
                    "content":result["content"], "error":result
                }]}}),
            ],
            CliKind::Codex => vec![
                json!({"type":"item.started","item":{
                    "id":"wire-call","type":"mcp_tool_call","server":server,"tool":tool,
                    "arguments":{"id":"fixture"},"status":"in_progress"
                }}),
                json!({"type":"item.completed","item":{
                    "id":"wire-call","type":"mcp_tool_call","server":server,"tool":tool,
                    "arguments":{"id":"fixture"},"status":"completed","result":result
                }}),
            ],
            CliKind::Agy => vec![
                json!({"event":"step_update","step_update":{
                    "step_index":12,"step_type":"tool","state":"RUNNING","tool_name":"call_mcp_tool",
                    "tool_info":{"parameters":{"ServerName":server,"ToolName":tool,"Arguments":{"id":"fixture"}}}
                }}),
                json!({"event":"step_update","step_update":{
                    "step_index":12,"step_type":"tool","state":"DONE","tool_name":"call_mcp_tool",
                    "tool_info":{"parameters":{"ServerName":server,"ToolName":tool,"Arguments":{"id":"fixture"}},"output":result}
                }}),
            ],
            _ => unreachable!("only managed transports"),
        }
    }

    fn translate_transport(
        kind: CliKind,
        names: crate::mcp_server::wire_names::WireNames,
        server: &str,
        tool: &str,
        result: Value,
    ) -> Vec<EngineEvent> {
        let mut translator = StreamTranslator::new(kind)
            .with_wire_names(Some(names))
            .with_turn_mcp_server(Some("c0123456789012345678901"));
        transport_events(kind, server, tool, result)
            .into_iter()
            .flat_map(|event| translator.translate_line(&event.to_string()))
            .collect()
    }

    #[test]
    fn managed_wire_names_restore_full_canonical_identity_in_each_protocol() {
        let names = crate::mcp_server::wire_names::WireNames::default();
        let long = format!("mcp_fixture_{}", "x".repeat(116));
        let mut first = [json!({"name":long})];
        names.publish(&mut first).unwrap();
        // The second canonical name is itself an advertised wire alias for
        // the first tool. A second decode would incorrectly select that tool.
        // Every protocol receives a separate start and result update.
        let chained = first[0]["name"].as_str().unwrap().to_string();
        for canonical in [long, chained] {
            assert!(canonical.len() <= 128);
            let mut descriptors = [
                json!({"name":canonical,"description":"Synthetic method","inputSchema":{"type":"object"}}),
            ];
            names.publish(&mut descriptors).unwrap();
            let wire = descriptors[0]["name"].as_str().unwrap();
            assert_ne!(wire, canonical);
            for kind in [CliKind::Claude, CliKind::Codex, CliKind::Agy] {
                let server = if kind == CliKind::Agy {
                    "c0123456789012345678901"
                } else {
                    SERVER_KEY
                };
                let events = translate_transport(
                    kind,
                    names.clone(),
                    server,
                    wire,
                    json!({"isError":false,"content":[{"type":"text","text":"fixture state"}]}),
                );
                let expected = if kind == CliKind::Claude {
                    format!("mcp__clovy__{canonical}")
                } else {
                    format!("clovy.{canonical}")
                };
                assert_eq!(events.iter().filter(|event| matches!(event, EngineEvent::ToolStarted { name, arguments, .. } if name == &expected && arguments == &json!({"id":"fixture"}))).count(), 1);
                assert_eq!(events.iter().filter(|event| matches!(event, EngineEvent::ToolFinished { name, error:None, .. } if name == &expected)).count(), 1);
            }
        }
    }

    #[test]
    fn only_current_resolved_wire_names_can_claim_a_managed_diagnostic() {
        let canonical = format!("mcp_fixture_{}", "x".repeat(116));
        let names = crate::mcp_server::wire_names::WireNames::default();
        let mut descriptors = [json!({"name":canonical})];
        names.publish(&mut descriptors).unwrap();
        let wire = descriptors[0]["name"].as_str().unwrap();
        let diagnostic = crate::agent_mcp::AgentMcpError::OauthReconnectRequired.diagnostic(
            "fixture",
            Some(&canonical),
            "refresh",
        );
        let result = json!({"isError":true,"content":[{"type":"text","text":"native failure"}],"_meta":{"clovy":{"diagnostic":diagnostic}}});
        for kind in [CliKind::Claude, CliKind::Codex, CliKind::Agy] {
            let active = if kind == CliKind::Agy {
                "c0123456789012345678901"
            } else {
                SERVER_KEY
            };
            for (metadata, server, observed, owned) in [
                (names.clone(), active, wire, true),
                (names.clone(), "foreign", wire, false),
                (names.clone(), active, "_c_unadvertised", false),
                (names.clone(), active, canonical.as_str(), false),
                (
                    crate::mcp_server::wire_names::WireNames::default(),
                    active,
                    wire,
                    false,
                ),
            ] {
                let events = translate_transport(kind, metadata, server, observed, result.clone());
                let (name, error) = events
                    .iter()
                    .find_map(|event| match event {
                        EngineEvent::ToolFinished {
                            name,
                            error: Some(error),
                            ..
                        } => Some((name, error)),
                        _ => None,
                    })
                    .unwrap();
                if owned {
                    assert!(name.ends_with(&canonical), "{name}");
                    assert!(error.contains("agent_mcp_reconnect_required"), "{error}");
                    assert!(error.contains(&canonical), "{error}");
                } else {
                    assert!(
                        !name.starts_with("clovy.") && !name.starts_with("mcp__clovy__"),
                        "{name}"
                    );
                    assert!(error.contains("origin: external_cli"), "{error}");
                    assert!(error.contains("native failure"), "{error}");
                    assert!(!error.contains("agent_mcp_reconnect_required"), "{error}");
                }
            }
            if kind == CliKind::Agy {
                for previous in ["clovy", "c_previous_turn"] {
                    let events =
                        translate_transport(kind, names.clone(), previous, wire, result.clone());
                    assert!(events.iter().any(|event| matches!(event, EngineEvent::ToolFinished { name, error:Some(error), .. } if name.starts_with("external_mcp.") && error.contains("origin: external_cli") && !error.contains("agent_mcp_reconnect_required"))));
                }
            }
        }
    }

    #[test]
    fn codex_external_failure_keeps_origin_without_changing_healthy_managed_result() {
        let mut translator = StreamTranslator::new(CliKind::Codex);
        let healthy = translator.translate_line(
            &json!({"type":"item.completed","item":{
                "id":"healthy", "type":"mcp_tool_call", "server":SERVER_KEY,
                "tool":"mcp_private_lookup", "status":"completed", "result":{"value":7}
            }})
            .to_string(),
        );
        assert!(healthy.iter().any(|event| matches!(event,
            EngineEvent::ToolFinished {call_id, output, error:None, ..}
                if call_id == "healthy" && output["value"] == 7
        )));
        let failed = translator.translate_line(
            &json!({"type":"item.completed","item":{
                "id":"external", "type":"mcp_tool_call", "server":"codex_apps",
                "tool":"atlassian.atlassianUserInfo", "status":"failed",
                "error":{"code":-32601,"message":"Unknown tool"}
            }})
            .to_string(),
        );
        let detail = failed
            .iter()
            .find_map(|event| match event {
                EngineEvent::ToolFinished {
                    error: Some(error), ..
                } => Some(error),
                _ => None,
            })
            .unwrap();
        for part in [
            "external_cli",
            "codex_apps",
            "atlassian.atlassianUserInfo",
            "-32601",
            "Unknown tool",
            "phase: call",
            "inspect_external_catalog",
        ] {
            assert!(detail.contains(part), "{detail}");
        }
        assert!(!detail.contains("reconnect"));
    }

    #[test]
    fn same_vercel_provider_keeps_managed_success_separate_from_inherited_auth_failure() {
        let mut translator = StreamTranslator::new(CliKind::Codex);
        let projects = json!({"projects":[{"id":"synthetic-project"}]});
        let healthy_item = |id: &str| {
            json!({"type":"item.completed","item":{
                "id":id, "type":"mcp_tool_call", "server":SERVER_KEY,
                "tool":"mcp_vercel_list_projects", "status":"completed", "result":projects
            }})
            .to_string()
        };
        let healthy_before = translator.translate_line(&healthy_item("managed-before"));
        assert!(healthy_before.iter().any(|event| matches!(event,
            EngineEvent::ToolFinished {call_id, name, output, error:None, ..}
                if call_id == "managed-before"
                    && name == "clovy.mcp_vercel_list_projects"
                    && output == &projects
        )));
        let inherited = translator.translate_line(
            &json!({"type":"item.completed","item":{
                "id":"inherited-vercel", "type":"mcp_tool_call", "server":"vercel",
                "tool":"list_projects", "status":"failed",
                "error":{"code":"UNAUTHORIZED","message":"Authorization rejected"}
            }})
            .to_string(),
        );
        let detail = inherited
            .iter()
            .find_map(|event| match event {
                EngineEvent::ToolFinished {
                    call_id,
                    name,
                    error: Some(error),
                    ..
                } if call_id == "inherited-vercel" && name == "vercel.list_projects" => Some(error),
                _ => None,
            })
            .unwrap();
        for part in [
            "origin: external_cli",
            "server: vercel",
            "tool: list_projects",
            "phase: call",
            "code: mcp_tool_failed",
            "remote code: UNAUTHORIZED",
            "state: unavailable",
            "action: check_external_connection_and_permissions",
        ] {
            assert!(detail.contains(part), "{detail}");
        }
        for unrelated_recovery in ["native", "key", "disconnect", "reconnect"] {
            assert!(!detail.contains(unrelated_recovery), "{detail}");
        }
        let healthy_after = translator.translate_line(&healthy_item("managed-after"));
        assert!(healthy_after.iter().any(|event| matches!(event,
            EngineEvent::ToolFinished {call_id, name, output, error:None, ..}
                if call_id == "managed-after"
                    && name == "clovy.mcp_vercel_list_projects"
                    && output == &projects
        )));
    }

    #[test]
    fn managed_remote_http_status_survives_the_final_tool_failure() {
        let diagnostic = crate::agent_mcp::AgentMcpError::DeliveryUncertain(Some(403)).diagnostic(
            "vercel",
            Some("mcp_vercel_update_project"),
            "call",
        );
        let mut translator = StreamTranslator::new(CliKind::Codex);
        let events = translator.translate_line(
            &json!({"type":"item.completed","item":{
                "id":"forbidden", "type":"mcp_tool_call", "server":SERVER_KEY,
                "tool":"mcp_vercel_update_project", "status":"completed",
                "result":{"isError":true,"content":[{"type":"text","text":"fixture-private-provider-body"}],
                    "_meta":{"clovy":{"diagnostic":diagnostic}}}
            }}).to_string(),
        );
        let error = events
            .iter()
            .find_map(|event| match event {
                EngineEvent::ToolFinished {
                    error: Some(error), ..
                } => Some(error),
                _ => None,
            })
            .unwrap();
        for part in [
            "origin: clovy_managed",
            "server: vercel",
            "tool: mcp_vercel_update_project",
            "phase: call",
            "code: agent_mcp_delivery_uncertain",
            "remote code: 403",
            "inspect_server_before_retry",
        ] {
            assert!(error.contains(part), "{error}");
        }
        assert!(!error.contains("fixture-private-provider-body"));
    }

    #[test]
    fn codex_completed_mcp_error_result_is_a_failed_tool_with_host_cause() {
        let diagnostic = crate::agent_mcp::AgentMcpError::OauthReconnectRequired.diagnostic(
            "Vercel",
            Some("mcp_vercel_projects"),
            "refresh",
        );
        for status in ["completed", "failed"] {
            let mut translator = StreamTranslator::new(CliKind::Codex);
            let events = translator.translate_line(
                &json!({"type":"item.completed","item":{
                    "id":"denied", "type":"mcp_tool_call", "server":SERVER_KEY,
                    "tool":"mcp_vercel_projects", "status":status,
                    "result":{"isError":true,"content":[{"type":"text","text":"Reconnect"}],
                        "_meta":{"clovy":{"diagnostic":diagnostic}}}
                }})
                .to_string(),
            );
            let error = events
                .iter()
                .find_map(|event| match event {
                    EngineEvent::ToolFinished {
                        error: Some(error),
                        output,
                        ..
                    } => {
                        assert_eq!(*output, Value::Null);
                        Some(error)
                    }
                    _ => None,
                })
                .unwrap();
            for part in [
                "clovy_managed",
                "Vercel",
                "phase: refresh",
                "agent_mcp_reconnect_required",
            ] {
                assert!(error.contains(part), "{status}: {error}");
            }
        }
    }

    #[test]
    fn external_metadata_cannot_claim_clovy_ownership() {
        let diagnostic = crate::agent_mcp::AgentMcpError::OauthReconnectRequired
            .diagnostic("Vercel", None, "refresh");
        let detail = codex_mcp_error(
            &json!({"server":"codex_apps","tool":"lookup"}),
            &json!({"code":"UNAUTHORIZED", "message":"Not authorized",
                "_meta":{"clovy":{"diagnostic":diagnostic}}}),
        );
        assert!(detail.contains("origin: external_cli"));
        assert!(detail.contains("remote code: UNAUTHORIZED"));
        assert!(detail.contains("check_external_connection_and_permissions"));
        assert!(!detail.contains("reconnect_required"));
    }

    #[test]
    fn claude_inherited_mcp_failure_is_attributed_using_the_observed_tool_name() {
        let mut translator = StreamTranslator::new(CliKind::Claude);
        translator.translate_line(&json!({"type":"assistant","message":{"content":[{
            "type":"tool_use","id":"external","name":"mcp__codex_apps__atlassianUserInfo","input":{}
        }]}}).to_string());
        let events = translator.translate_line(
            &json!({"type":"user","message":{"content":[{
                "type":"tool_result","tool_use_id":"external","is_error":true,
                "content":"Unknown tool in external catalog"
            }]}})
            .to_string(),
        );
        let detail = events
            .iter()
            .find_map(|event| match event {
                EngineEvent::ToolFinished {
                    error: Some(error), ..
                } => Some(error),
                _ => None,
            })
            .unwrap();
        for part in [
            "origin: external_cli",
            "server: codex_apps",
            "tool: atlassianUserInfo",
            "Unknown tool",
        ] {
            assert!(detail.contains(part), "{detail}");
        }
        assert!(!detail.contains("reconnect"));
    }
}
