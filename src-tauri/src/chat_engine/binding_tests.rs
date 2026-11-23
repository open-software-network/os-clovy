//! The turn binding seen from the engine (task 0.4): each CLI turn gets its
//! own `--turn` token in the MCP configuration it receives, the token binds
//! the run's MCP connections while the turn lives and is revoked at its end,
//! and a write tool that asks the approval broker over that binding is
//! approved, denied, or cancelled with the run.
//!
//! The fake CLI is a shell script, as in `tests.rs`. It cannot speak the
//! socket protocol itself, so when it "calls" a Clovy tool it hands the call
//! to the test, which runs the relay code `clovy-mcp` runs
//! (`stdio::relay_turn`) with the token the CLI received, against a real
//! socket listener bound to the same `TurnRegistry` the turn issued it in.

use super::tests::{Harness, TestSink};
use super::turn::{execute_turn, TurnContext};
use super::*;
use crate::approval_broker::{
    commands, ApprovalBroker, ApprovalKind, ApprovalRequest, ApprovalTurn, BrokerIo, Clock,
    TokioClock, APPROVAL_TIMEOUT,
};
use crate::mcp_server::channel::{self, ClientError};
use crate::mcp_server::tools::{ActivityAccess, McpData, ToolCtx, ToolGroup};
use crate::mcp_server::turns::{TurnBinding, TurnRegistry, TURN_REQUIRED};
use crate::mcp_server::{protocol, stdio, McpLaunch};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx_sqlite::SqlitePool;
use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

const PROBE_WRITE: &str = "probe_write";

/// A write tool as a parity slice writes one: bound-only, and it acts only
/// after the context's broker approved the call for the connection's turn.
#[derive(Clone, Copy)]
struct ProbeWrite;

impl ToolGroup for ProbeWrite {
    fn owns(&self, name: &str) -> bool {
        name == PROBE_WRITE
    }

    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        vec![json!({
            "name": PROBE_WRITE,
            "description": "Test only: writes after the user approves.",
            "inputSchema": { "type": "object" },
        })]
    }

    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let binding = ctx.binding.expect("bound-only groups run with a turn");
            let request = ApprovalRequest {
                tool: name.to_string(),
                call_id: None,
                kind: ApprovalKind::Approval {
                    title: "Write a test file".into(),
                    description: "Clovy writes one file for this test.".into(),
                    arguments: arguments.clone(),
                    preview: None,
                },
            };
            ctx.broker()?
                .request(&ApprovalTurn::from(binding), request)
                .await
                .map_err(|denial| AppError::new(denial.code(), denial.message()))?;
            Ok(json!({ "written": true, "runId": binding.run_id }))
        })
    }
}

/// The app's handler shape with the probe group in front of the read tools.
fn handler(data: McpData, broker: ApprovalBroker) -> channel::TurnHandler {
    let data = Arc::new(data);
    let probe = ProbeWrite;
    Arc::new(move |message: Value, binding: Option<Arc<TurnBinding>>| {
        let (data, broker) = (Arc::clone(&data), broker.clone());
        Box::pin(async move {
            let ctx = ToolCtx {
                data: &data,
                app: None,
                binding: binding.as_deref(),
                broker: Some(&broker),
            };
            let name = message["params"]["name"].as_str().unwrap_or_default();
            if message["method"] == "tools/list" && ctx.binding.is_some() {
                let mut listed = protocol::handle_in(&ctx, &message).await?;
                if let Some(tools) = listed["result"]["tools"].as_array_mut() {
                    tools.extend(probe.descriptors(&ctx));
                }
                return Some(listed);
            }
            if message["method"] != "tools/call" || !probe.owns(name) {
                return protocol::handle_in(&ctx, &message).await;
            }
            let result = if ctx.binding.is_none() {
                protocol::tool_error(&format!("Needs a Clovy turn. ({TURN_REQUIRED})"))
            } else {
                match probe
                    .call(&ctx, name, &message["params"]["arguments"])
                    .await
                {
                    Ok(value) => protocol::tool_result(&value),
                    Err(error) => {
                        protocol::tool_error(&format!("{} ({})", error.message, error.code))
                    }
                }
            };
            Some(json!({ "jsonrpc": "2.0", "id": message["id"], "result": result }))
        })
    })
}

#[derive(Clone)]
struct RecordingIo {
    pool: SqlitePool,
    events: Arc<StdMutex<Vec<String>>>,
}

impl BrokerIo for RecordingIo {
    fn pool(&self) -> BoxFuture<'_, Result<SqlitePool, AppError>> {
        Box::pin(async move { Ok(self.pool.clone()) })
    }

    fn emit(&self, _session_id: &str, _run_id: &str, _sequence: i64, method: &str, _data: Value) {
        self.events.lock().unwrap().push(method.to_string());
    }
}

#[derive(Clone, Default)]
struct Output(Arc<StdMutex<Vec<u8>>>);

impl std::io::Write for Output {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Output {
    fn messages(&self) -> Vec<Value> {
        String::from_utf8_lossy(&self.0.lock().unwrap())
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

/// Stdin of a CLI: the requests, then open until the test closes it.
struct OpenInput {
    lines: std::io::Cursor<Vec<u8>>,
    gate: std::sync::mpsc::Receiver<()>,
}

impl std::io::Read for OpenInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.lines.read(buf)?;
        if read == 0 {
            let _ = self.gate.recv();
        }
        Ok(read)
    }
}

async fn wait_for(what: &str, mut ready: impl FnMut() -> bool) {
    for _ in 0..500 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The MCP server's `--dir` and `--turn` values in the arguments a CLI got.
fn dir_and_turn(args: &Value) -> (String, String) {
    let args = args
        .as_array()
        .unwrap_or_else(|| panic!("no args: {args}"))
        .iter()
        .map(|arg| arg.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    let after = |flag: &str| {
        let at = args.iter().position(|arg| arg == flag).unwrap();
        args[at + 1].clone()
    };
    (after("--dir"), after("--turn"))
}

struct Rig {
    harness: Harness,
    turns: TurnRegistry,
    broker: ApprovalBroker,
    events: Arc<StdMutex<Vec<String>>>,
    dir: tempfile::TempDir,
    _listener: channel::Listener,
}

impl Rig {
    async fn new(kind: CliKind) -> Self {
        Self::with_clock(kind, TokioClock).await
    }

    async fn with_clock(kind: CliKind, clock: impl Clock) -> Self {
        let harness = Harness::new(kind).await;
        let events = Arc::default();
        let broker = ApprovalBroker::new(
            RecordingIo {
                pool: harness.repository.pool.clone(),
                events: Arc::clone(&events),
            },
            clock,
            APPROVAL_TIMEOUT,
        );
        let turns = TurnRegistry::default();
        let dir = tempfile::tempdir().unwrap();
        let data = crate::mcp_server::tests::data(
            crate::mcp_server::tests::notes_pool().await,
            ActivityAccess::Off,
        );
        let listener = channel::Listener::start_with_turns(
            dir.path(),
            handler(data, broker.clone()),
            turns.clone(),
        )
        .await
        .unwrap();
        Self {
            harness,
            turns,
            broker,
            events,
            dir,
            _listener: listener,
        }
    }

    fn launch(&self) -> McpLaunch {
        crate::mcp_server::launch(Path::new("/bin/clovy-mcp"), self.dir.path())
    }

    fn context(&self, kind: CliKind, program: &Path, run_id: &str) -> TurnContext {
        let mut context =
            self.harness
                .context(kind, program, run_id, "Write it", Some(self.launch()));
        context.turns = Some(self.turns.clone());
        context
    }

    fn pool(&self) -> &SqlitePool {
        &self.harness.repository.pool
    }

    fn log(&self, name: &str) -> std::path::PathBuf {
        self.harness.log.path().join(name)
    }

    /// Runs the real relay with `turn` and the given requests, keeping its
    /// input open (as a CLI does) until the answer to the last one arrived.
    async fn relay(&self, turn: &str, requests: &[Value]) -> Output {
        let input: String = requests.iter().map(|line| format!("{line}\n")).collect();
        let last_id = requests.last().map(|request| request["id"].clone());
        let output = Output::default();
        let (gate, gated) = std::sync::mpsc::channel();
        let (dir, turn, sink) = (
            self.dir.path().to_path_buf(),
            turn.to_string(),
            output.clone(),
        );
        let relay = tokio::task::spawn_blocking(move || {
            stdio::relay_turn(
                &dir,
                Some(&turn),
                std::io::BufReader::new(OpenInput {
                    lines: std::io::Cursor::new(input.into_bytes()),
                    gate: gated,
                }),
                Box::new(sink),
            );
        });
        let answered = output.clone();
        wait_for("the relay's last answer", move || {
            answered
                .messages()
                .iter()
                .any(|message| Some(&message["id"]) == last_id.as_ref())
        })
        .await;
        let _ = gate.send(());
        relay.await.unwrap();
        output
    }
}

fn initialize() -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "fake", "version": "1" } } })
}

fn call_probe() -> Value {
    json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": PROBE_WRITE, "arguments": { "path": "notes.txt" } } })
}

/// A fake claude that copies its MCP configuration, announces a call to the
/// probe tool, waits for the test to hand it the tool's outcome (one word),
/// and finishes the turn with it.
const FAKE_CLAUDE_CALLING: &str = r#"
cfg=$(sed -n '/^--mcp-config$/{n;p;}' "$LOG/$n.args"); cp "$cfg" "$LOG/mcp.json"
echo '{"type":"system","subtype":"init","session_id":"claude-conv-1"}'
echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"call-1","name":"mcp__clovy__probe_write","input":{"path":"notes.txt"}}]}}'
: > "$LOG/calling"
printf '%s' "$MCP_TOOL_TIMEOUT" > "$LOG/configured-timeout"
while [ ! -f "$LOG/outcome" ]; do
  if [ -f "$LOG/elapsed" ]; then
    elapsed=$(cat "$LOG/elapsed"); elapsed=${elapsed:-0}
    if [ "$elapsed" -ge "${MCP_TOOL_TIMEOUT:-0}" ]; then
      : > "$LOG/cli-timeout"
      exit 12
    fi
    printf '%s' "$elapsed" > "$LOG/observed-elapsed"
  fi
  sleep 0.02
done
outcome=$(cat "$LOG/outcome")
echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"call-1","content":"'"$outcome"'","is_error":false}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"'"$outcome"'"}'
"#;

#[derive(Debug, Clone, Copy)]
enum Answer {
    Approve,
    ApproveNearDeadline,
    Deny,
    Cancel,
}

#[derive(Clone)]
struct ManualApprovalClock {
    elapsed: Arc<tokio::sync::watch::Sender<Duration>>,
    armed: Arc<std::sync::atomic::AtomicBool>,
}

impl ManualApprovalClock {
    fn new() -> Self {
        Self {
            elapsed: Arc::new(tokio::sync::watch::channel(Duration::ZERO).0),
            armed: Arc::default(),
        }
    }
}

impl Clock for ManualApprovalClock {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        let mut elapsed = self.elapsed.subscribe();
        let deadline = *elapsed.borrow() + duration;
        self.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        Box::pin(async move {
            let _ = elapsed.wait_for(|elapsed| *elapsed >= deadline).await;
        })
    }
}

async fn tool_call_through_the_turn_binding(answer: Answer) {
    let kind = CliKind::Claude;
    let clock = ManualApprovalClock::new();
    let rig = if matches!(answer, Answer::ApproveNearDeadline) {
        Rig::with_clock(kind, clock.clone()).await
    } else {
        Rig::new(kind).await
    };
    let program = rig.harness.fake_cli(kind, FAKE_CLAUDE_CALLING);
    let run_id = rig.harness.new_run(kind, "Write it").await;
    let host = Arc::new(ChatEngineHost::default());
    let ticket = host.register(&run_id).unwrap();
    let sink = Arc::new(TestSink {
        repository: rig.harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    });
    let turn = {
        let (repository, sink, context) = (
            rig.harness.repository.clone(),
            Arc::clone(&sink),
            rig.context(kind, &program, &run_id),
        );
        let TurnTicket { cancel, _done } = ticket;
        tokio::spawn(async move {
            execute_turn(&repository, sink.as_ref(), context, cancel).await;
            drop(_done);
        })
    };

    // The CLI got a token for this run in its MCP configuration.
    let calling = rig.log("calling");
    wait_for("the CLI's tool call", || calling.exists()).await;
    let config: Value =
        serde_json::from_str(&std::fs::read_to_string(rig.log("mcp.json")).unwrap()).unwrap();
    let (dir, token) = dir_and_turn(&config["mcpServers"]["clovy"]["args"]);
    assert_eq!(dir, rig.dir.path().display().to_string());
    assert_eq!(token.len(), 64);
    let bound = rig
        .turns
        .binding(&token)
        .expect("token live during the turn");
    assert_eq!(bound.run_id, run_id);
    assert_eq!(bound.session_id, rig.harness.session_id);
    assert_eq!(bound.cli, kind);
    assert!(bound.attended);

    // The CLI's MCP client calls the write tool; it waits on the broker.
    let relay = {
        let rig_turn = token.clone();
        let requests = [initialize(), call_probe()];
        let rig = &rig;
        async move { rig.relay(&rig_turn, &requests).await }
    };
    let decide = async {
        let broker = rig.broker.clone();
        wait_for("the approval card", || !broker.waiting_ids().is_empty()).await;
        let interruption_id = rig.broker.waiting_ids().remove(0);
        if matches!(answer, Answer::ApproveNearDeadline) {
            wait_for("the broker timer", || {
                clock.armed.load(std::sync::atomic::Ordering::SeqCst)
            })
            .await;
            let elapsed = APPROVAL_TIMEOUT - Duration::from_secs(1);
            clock.elapsed.send_replace(elapsed);
            std::fs::write(rig.log("elapsed"), elapsed.as_millis().to_string()).unwrap();
            wait_for("the fake CLI's controlled clock", || {
                rig.log("cli-timeout").exists()
                    || std::fs::read_to_string(rig.log("observed-elapsed"))
                        .ok()
                        .as_deref()
                        == Some(elapsed.as_millis().to_string().as_str())
            })
            .await;
            assert!(!rig.log("cli-timeout").exists());
            assert_eq!(
                std::fs::read_to_string(rig.log("configured-timeout")).unwrap(),
                policy::CLI_TOOL_TIMEOUT.as_millis().to_string()
            );
            assert!(rig.broker.is_waiting(&interruption_id));
        }
        assert_eq!(
            rig.harness
                .repository
                .get_run(&run_id)
                .await
                .unwrap()
                .status,
            "waiting_for_user"
        );
        match answer {
            Answer::Approve | Answer::ApproveNearDeadline | Answer::Deny => {
                let choice = if matches!(answer, Answer::Approve | Answer::ApproveNearDeadline) {
                    "once"
                } else {
                    "deny"
                };
                assert!(commands::resolve(
                    &rig.broker,
                    rig.pool(),
                    &run_id,
                    &interruption_id,
                    &json!({ "kind": "approval", "choice": choice }),
                )
                .await
                .unwrap());
            }
            Answer::Cancel => {
                assert!(cancel_cli_run(&host, Some(&rig.broker), Some(&rig.turns), &run_id).await);
                // Revoked at once, before the CLI has even stopped.
                assert!(rig.turns.binding(&token).is_none());
            }
        }
        interruption_id
    };
    let (output, interruption_id) = tokio::join!(relay, decide);
    let messages = output.messages();
    let result = messages
        .iter()
        .find(|message| message["id"] == 2)
        .unwrap_or_else(|| panic!("no tool answer: {messages:?}"));
    let text = result["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let item = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let item = crate::approval_broker::store::find_cli_interruption(
                rig.pool(),
                &run_id,
                &interruption_id,
            )
            .await
            .unwrap()
            .expect("the card was recorded");
            if !matches!(answer, Answer::Cancel) || item.payload["status"] != "pending" {
                break item;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the card settles");
    match answer {
        Answer::Approve | Answer::ApproveNearDeadline => {
            assert_eq!(result["result"]["isError"], false, "{text}");
            let written: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(written, json!({ "written": true, "runId": run_id }));
            assert_eq!(item.payload["status"], "resolved");
        }
        Answer::Deny => {
            assert_eq!(result["result"]["isError"], true);
            assert!(text.ends_with("(user_denied)"), "{text}");
            assert_eq!(item.payload["status"], "resolved");
        }
        Answer::Cancel => {
            // The pending call gets the broker's answer before the revoked
            // token closes the connection.
            assert_eq!(result["result"]["isError"], true);
            assert!(text.ends_with("(run_cancelled)"), "{text}");
            assert_eq!(item.payload["status"], "cancelled");
        }
    }
    if !matches!(answer, Answer::Cancel) {
        let outcome = if matches!(answer, Answer::Approve | Answer::ApproveNearDeadline) {
            "written"
        } else {
            "denied"
        };
        std::fs::write(rig.log("outcome"), outcome).unwrap();
    }
    tokio::time::timeout(Duration::from_secs(10), turn)
        .await
        .expect("the turn ends")
        .unwrap();

    let run = rig.harness.repository.get_run(&run_id).await.unwrap();
    let expected = if matches!(answer, Answer::Cancel) {
        "cancelled"
    } else {
        "completed"
    };
    assert_eq!(run.status, expected);
    let events = rig.events.lock().unwrap().clone();
    assert_eq!(
        events,
        ["interruption.requested", "interruption.updated"],
        "{answer:?}"
    );

    // The token died with the turn: the registry forgot it and the socket
    // refuses it.
    assert!(rig.turns.binding(&token).is_none());
    let dir = rig.dir.path().to_path_buf();
    let refused = tokio::task::spawn_blocking(move || channel::connect_turn(&dir, Some(&token)))
        .await
        .unwrap();
    assert!(
        matches!(refused, Err(ClientError::TurnExpired)),
        "{answer:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn policy_fake_cli_receives_tool_result_when_approval_is_near_deadline() {
    tool_call_through_the_turn_binding(Answer::ApproveNearDeadline).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_engine_turn_binding_tool_call_approved_runs_the_tool() {
    tool_call_through_the_turn_binding(Answer::Approve).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_engine_turn_binding_tool_call_denied_returns_user_denied() {
    tool_call_through_the_turn_binding(Answer::Deny).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_engine_turn_binding_cancel_settles_the_card_and_revokes_the_token() {
    tool_call_through_the_turn_binding(Answer::Cancel).await;
}

/// The phase 2 test path: a write group run directly with a scripted broker,
/// no socket and no Tauri runtime.
#[tokio::test]
async fn chat_engine_write_group_runs_against_a_scripted_broker() {
    use crate::approval_broker::testing::{Scripted, TestBroker};
    let broker = TestBroker::new([Scripted::Approve, Scripted::Deny]).await;
    let data = crate::mcp_server::tests::data(
        crate::mcp_server::tests::notes_pool().await,
        ActivityAccess::Off,
    );
    let attended = broker.turn(CliKind::Codex, true).await;
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&attended),
        broker: Some(broker.broker()),
    };
    let args = json!({ "path": "notes.txt" });
    assert_eq!(
        ProbeWrite.call(&ctx, PROBE_WRITE, &args).await.unwrap(),
        json!({ "written": true, "runId": attended.run_id })
    );
    let denied = ProbeWrite.call(&ctx, PROBE_WRITE, &args).await.unwrap_err();
    assert_eq!(denied.code, "user_denied");
    let cards = broker.requests();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0]["toolName"], PROBE_WRITE);
    assert_eq!(cards[0]["title"], "Write a test file");
    assert_eq!(cards[0]["cli"], "codex");

    // A routine is refused without a card.
    let routine = broker.turn(CliKind::Claude, false).await;
    let ctx_routine = ToolCtx {
        binding: Some(&routine),
        ..ctx
    };
    let refused = ProbeWrite
        .call(&ctx_routine, PROBE_WRITE, &args)
        .await
        .unwrap_err();
    assert_eq!(refused.code, "unattended_requires_approval");
    assert_eq!(broker.requests().len(), 2);

    // Without a broker the tool refuses instead of acting.
    let no_broker = ToolCtx {
        broker: None,
        ..ctx
    };
    let unavailable = ProbeWrite
        .call(&no_broker, PROBE_WRITE, &args)
        .await
        .unwrap_err();
    assert_eq!(unavailable.code, "approval_unavailable");

    // A card nobody answers ends with the run.
    let waiting = TestBroker::new([Scripted::Leave]).await;
    let turn = waiting.turn(CliKind::Claude, true).await;
    let ctx_waiting = ToolCtx {
        binding: Some(&turn),
        broker: Some(waiting.broker()),
        ..ctx
    };
    let call = ProbeWrite.call(&ctx_waiting, PROBE_WRITE, &args);
    let cancel = async {
        wait_for("the card", || !waiting.broker().waiting_ids().is_empty()).await;
        waiting.broker().cancel_run(&turn.run_id).await
    };
    let (outcome, cancelled) = tokio::join!(call, cancel);
    assert_eq!(cancelled, 1);
    assert_eq!(outcome.unwrap_err().code, "run_cancelled");
}

/// Revocation cancels a handler before it can continue, while the CLI still
/// receives a cancellation result.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_engine_revoked_turn_answers_its_pending_call_before_closing() {
    let turns = TurnRegistry::default();
    let dir = tempfile::tempdir().unwrap();
    let release = Arc::new(tokio::sync::Notify::new());
    let entered = Arc::new(tokio::sync::Notify::new());
    let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handler: channel::TurnHandler = {
        let (release, entered, effects) = (
            Arc::clone(&release),
            Arc::clone(&entered),
            Arc::clone(&effects),
        );
        Arc::new(move |message: Value, _binding: Option<Arc<TurnBinding>>| {
            let (release, entered, effects) = (
                Arc::clone(&release),
                Arc::clone(&entered),
                Arc::clone(&effects),
            );
            Box::pin(async move {
                if message["method"] != "tools/call" {
                    return Some(json!({ "jsonrpc": "2.0", "id": message["id"], "result": {} }));
                }
                entered.notify_one();
                release.notified().await;
                effects.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Some(json!({
                    "jsonrpc": "2.0",
                    "id": message["id"],
                    "result": protocol::tool_error("The run was cancelled. (run_cancelled)"),
                }))
            })
        })
    };
    let _listener = channel::Listener::start_with_turns(dir.path(), handler, turns.clone())
        .await
        .unwrap();
    let token = turns.issue(TurnBinding {
        authority: crate::domain::turn_authority::TurnAuthority::new(),
        wire_names: None,
        session_id: "session-1".into(),
        run_id: "run-1".into(),
        data_partition: "default".into(),
        cli: CliKind::Claude,
        safety_mode: AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "chat".into(),
    });
    let (gate, gated) = std::sync::mpsc::channel::<()>();
    let output = Output::default();
    let relay = {
        let (dir, token, sink) = (dir.path().to_path_buf(), token.clone(), output.clone());
        let input = format!("{}\n{}\n", initialize(), call_probe());
        tokio::task::spawn_blocking(move || {
            stdio::relay_turn(
                &dir,
                Some(&token),
                std::io::BufReader::new(OpenInput {
                    lines: std::io::Cursor::new(input.into_bytes()),
                    gate: gated,
                }),
                Box::new(sink),
            );
        })
    };
    tokio::time::timeout(Duration::from_secs(10), entered.notified())
        .await
        .expect("the call reached the app");
    // The run is cancelled: the token goes first, the answer right after.
    assert_eq!(turns.revoke_run("run-1"), 1);
    let answered = output.clone();
    wait_for("the call's answer", move || {
        answered.messages().iter().any(|message| message["id"] == 2)
    })
    .await;
    release.notify_one();
    assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 0);
    let _ = gate.send(());
    relay.await.unwrap();
    let answer = output
        .messages()
        .into_iter()
        .find(|message| message["id"] == 2)
        .unwrap();
    let text = answer["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.ends_with("(run_cancelled)"), "{text}");
    // And the token opens nothing any more.
    let dir = dir.path().to_path_buf();
    let refused = tokio::task::spawn_blocking(move || channel::connect_turn(&dir, Some(&token)))
        .await
        .unwrap();
    assert!(matches!(refused, Err(ClientError::TurnExpired)));
}

/// A fake CLI that copies the MCP configuration it got (claude, copilot) and
/// waits for the test before ending.
fn waiting_reader(kind: CliKind) -> &'static str {
    match kind {
        CliKind::Claude => {
            r#"cfg=$(sed -n '/^--mcp-config$/{n;p;}' "$LOG/$n.args"); cp "$cfg" "$LOG/mcp.json"
: > "$LOG/calling"; while [ ! -f "$LOG/outcome" ]; do sleep 0.02; done
echo '{"type":"system","subtype":"init","session_id":"c"}'
echo '{"type":"result","subtype":"success","is_error":false,"result":""}'"#
        }
        CliKind::Codex => {
            r#": > "$LOG/calling"; while [ ! -f "$LOG/outcome" ]; do sleep 0.02; done
echo '{"type":"thread.started","thread_id":"t"}'
echo '{"type":"turn.completed"}'"#
        }
        CliKind::Copilot => {
            r#"cfg=$(sed -n 's/^--additional-mcp-config=@//p' "$LOG/$n.args"); cp "$cfg" "$LOG/mcp.json"
: > "$LOG/calling"; while [ ! -f "$LOG/outcome" ]; do sleep 0.02; done
echo '{"type":"result","exitCode":0}'"#
        }
        other => panic!("{other:?} takes no MCP server"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_engine_each_turn_gets_its_own_token_revoked_at_its_end() {
    for kind in [CliKind::Claude, CliKind::Codex, CliKind::Copilot] {
        let rig = Rig::new(kind).await;
        let program = rig.harness.fake_cli(kind, waiting_reader(kind));
        let mut tokens = Vec::new();
        for turn_number in 1..=2 {
            let run_id = rig.harness.new_run(kind, "Hello").await;
            let _ = std::fs::remove_file(rig.log("calling"));
            let _ = std::fs::remove_file(rig.log("outcome"));
            let sink = TestSink {
                repository: rig.harness.repository.clone(),
                emitted: StdMutex::new(Vec::new()),
            };
            let (_cancel, receiver) = tokio::sync::watch::channel(false);
            let context = rig.context(kind, &program, &run_id);
            let repository = rig.harness.repository.clone();
            let turn = tokio::spawn(async move {
                execute_turn(&repository, &sink, context, receiver).await;
            });
            let calling = rig.log("calling");
            wait_for("the CLI to start", || calling.exists()).await;
            let args = rig.harness.args(turn_number);
            let server_args = match kind {
                CliKind::Codex => {
                    let value = args
                        .iter()
                        .find_map(|arg| arg.strip_prefix("mcp_servers.clovy.args="))
                        .unwrap_or_else(|| panic!("{args:?}"));
                    serde_json::from_str::<Value>(value).unwrap()
                }
                _ => {
                    let config: Value = serde_json::from_str(
                        &std::fs::read_to_string(rig.log("mcp.json")).unwrap(),
                    )
                    .unwrap();
                    config["mcpServers"]["clovy"]["args"].clone()
                }
            };
            let (_, token) = dir_and_turn(&server_args);
            let bound = rig
                .turns
                .binding(&token)
                .unwrap_or_else(|| panic!("{kind:?}: token not live during the turn"));
            assert_eq!(bound.run_id, run_id, "{kind:?}");
            // The token is only in the server's arguments, never in the
            // CLI's own environment or stdin.
            assert!(!rig.harness.stdin(turn_number).contains(&token));
            std::fs::write(rig.log("outcome"), "done").unwrap();
            turn.await.unwrap();
            assert_eq!(
                rig.harness
                    .repository
                    .get_run(&run_id)
                    .await
                    .unwrap()
                    .status,
                "completed",
                "{kind:?}"
            );
            assert!(rig.turns.binding(&token).is_none(), "{kind:?}");
            tokens.push(token);
        }
        assert_ne!(tokens[0], tokens[1], "{kind:?}: a token is never reused");
    }
}

#[tokio::test]
async fn chat_engine_turn_without_the_registry_runs_the_server_unbound() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(
        kind,
        r#"cfg=$(sed -n '/^--mcp-config$/{n;p;}' "$LOG/$n.args"); cp "$cfg" "$LOG/mcp.json"
echo '{"type":"system","subtype":"init","session_id":"c"}'
echo '{"type":"result","subtype":"success","is_error":false,"result":""}'"#,
    );
    let launch = crate::mcp_server::launch(Path::new("/bin/clovy-mcp"), Path::new("/data/mcp"));
    harness
        .turn(kind, &program, "Hello", Some(launch.clone()))
        .await;
    let config: Value = serde_json::from_str(
        &std::fs::read_to_string(harness.log.path().join("mcp.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(config["mcpServers"]["clovy"]["args"], json!(launch.args));
}

/// The policy's argv on the installed claude and codex, with the real
/// `clovy-mcp` relay bound to the turn and the approval broker approving
/// the probe tool's card. Writes each CLI's argv, stdin, and the run's
/// events under `$CLOVY_POLICY_SMOKE_DIR` when set. Build the relay first
/// (`cargo build --bin clovy-mcp`), then run apart with
/// `cargo test chat_engine_real_policy -- --ignored --nocapture`; it uses the
/// user's own CLI login and quota (codex reads a temporary `CODEX_HOME` with a
/// copy of `auth.json`, removed afterwards).
mod real {
    use super::*;

    /// A 1x1 PNG.
    const PIXEL: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f,
        0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8,
        0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99, 0x3d, 0x1d, 0x00, 0x00,
        0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];

    fn relay_binary() -> std::path::PathBuf {
        let exe = std::env::current_exe().unwrap();
        let binary = exe
            .parent()
            .and_then(Path::parent)
            .unwrap()
            .join(crate::mcp_server::BINARY_NAME);
        assert!(
            binary.exists(),
            "build the relay first: cargo build --bin clovy-mcp ({})",
            binary.display()
        );
        binary
    }

    async fn smoke(kind: CliKind, safety_mode: AgentSafetyMode, image: bool) {
        let login = crate::llm::shell_env::login_env().await;
        let real = login
            .which(kind.id())
            .unwrap_or_else(|| panic!("{} is not installed", kind.id()));
        let rig = Rig::new(kind).await;
        // The prelude logs argv and stdin; the real CLI gets both back.
        let program = rig.harness.fake_cli(
            kind,
            &format!("exec '{}' \"$@\" < \"$LOG/$n.stdin\"", real.display()),
        );
        let mut env = login.vars().clone();
        let codex_home = tempfile::tempdir().unwrap();
        if kind == CliKind::Codex {
            let home = std::path::PathBuf::from(env.get("HOME").unwrap());
            std::fs::copy(
                home.join(".codex/auth.json"),
                codex_home.path().join("auth.json"),
            )
            .unwrap();
            env.insert("CODEX_HOME".into(), codex_home.path().display().to_string());
        } else {
            env.insert("ANTHROPIC_MODEL".into(), "haiku".into());
        }
        let input = "Call the tool probe_write of the clovy MCP server once with {\"path\": \"smoke.txt\"}. Then reply with only the word done.";
        let run_id = rig.harness.new_run(kind, input).await;
        let mut context = rig.context(kind, &program, &run_id);
        context.input = input.to_string();
        context.env = env;
        context.safety_mode = safety_mode;
        context.mcp = Some(crate::mcp_server::launch(&relay_binary(), rig.dir.path()));
        if image {
            let path = rig.harness.workspace.path().join("attachments/pixel.png");
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, PIXEL).unwrap();
            context.images = vec![path];
        }
        let sink = Arc::new(TestSink {
            repository: rig.harness.repository.clone(),
            emitted: StdMutex::new(Vec::new()),
        });
        let turn = {
            let (repository, sink) = (rig.harness.repository.clone(), Arc::clone(&sink));
            let (_cancel, receiver) = tokio::sync::watch::channel(false);
            tokio::spawn(async move {
                let _keep = _cancel;
                execute_turn(&repository, sink.as_ref(), context, receiver).await;
            })
        };
        // Approve every card the turn raises until it ends.
        let mut approved = 0;
        while !turn.is_finished() {
            for id in rig.broker.waiting_ids() {
                if commands::resolve(
                    &rig.broker,
                    rig.pool(),
                    &run_id,
                    &id,
                    &json!({ "kind": "approval", "choice": "once" }),
                )
                .await
                .is_ok()
                {
                    approved += 1;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        turn.await.unwrap();
        let run = rig.harness.repository.get_run(&run_id).await.unwrap();
        let emitted = sink.emitted.lock().unwrap().clone();
        let args = rig.harness.args(1);
        let label = format!(
            "{}-{}{}",
            kind.id(),
            safety_mode.as_db(),
            if image { "-image" } else { "" }
        );
        if let Ok(dir) = std::env::var("CLOVY_POLICY_SMOKE_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            // The turn token is dead now; it is still masked in the record.
            let masked = |text: String| {
                let re_token = text
                    .split(['"', ',', ' ', '[', ']', '\n'])
                    .filter(|part| part.len() == 64 && part.bytes().all(|b| b.is_ascii_hexdigit()))
                    .map(str::to_string)
                    .collect::<Vec<_>>();
                re_token
                    .iter()
                    .fold(text, |text, token| text.replace(token, "<turn-token>"))
            };
            std::fs::write(dir.join(format!("{label}.args")), masked(args.join("\n"))).unwrap();
            let config = std::fs::read_to_string(rig.log("mcp.json")).unwrap_or_default();
            if !config.is_empty() {
                std::fs::write(dir.join(format!("{label}.mcp.json")), masked(config)).unwrap();
            }
            let events = emitted
                .iter()
                .map(|(method, data)| format!("{method} {data}"))
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(
                dir.join(format!("{label}.events")),
                format!(
                    "status: {}\napproved cards: {approved}\nbroker events: {:?}\n\n{events}\n",
                    run.status,
                    rig.events.lock().unwrap()
                ),
            )
            .unwrap();
        }
        println!("{label}: {} ({approved} approved)", run.status);
        assert_eq!(run.status, "completed", "{label}: {emitted:?}");
        assert!(
            approved >= 1,
            "{label}: the probe tool never asked: {emitted:?}"
        );
        assert!(emitted
            .iter()
            .any(|(method, data)| method == "tool.completed"
                && data.to_string().contains("probe_write")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "runs the installed claude and codex CLIs"]
    async fn chat_engine_real_policy_smoke() {
        smoke(CliKind::Claude, AgentSafetyMode::Sandboxed, false).await;
        smoke(CliKind::Claude, AgentSafetyMode::Unrestricted, false).await;
        smoke(CliKind::Codex, AgentSafetyMode::Sandboxed, true).await;
        smoke(CliKind::Codex, AgentSafetyMode::Unrestricted, false).await;
    }
}
