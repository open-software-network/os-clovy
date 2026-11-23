//! Turn bindings over the real socket and relay: a bound-only test group
//! (`probe_turn`) stands in for the write tools the parity slices add, so
//! these tests pin the rules every such group inherits.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::tests::{by_id, data, notes_pool, Captured};
use super::tools::{ActivityAccess, McpData, ToolCtx, ToolGroup};
use super::turns::{TurnBinding, TurnRegistry, TURN_EXPIRED};
use super::*;
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;

const PROBE: &str = "probe_turn";

/// A bound-only group with one tool that echoes the connection's turn.
pub(crate) struct TurnProbe;

impl ToolGroup for TurnProbe {
    fn owns(&self, name: &str) -> bool {
        name == PROBE
    }

    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        vec![json!({
            "name": PROBE,
            "description": "Test only: the turn this connection is bound to.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        })]
    }

    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        _name: &'a str,
        _arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let binding = ctx.binding.expect("bound-only groups run with a turn");
            Ok(json!({
                "sessionId": binding.session_id,
                "runId": binding.run_id,
                "cli": binding.cli,
                "attended": binding.attended,
            }))
        })
    }
}

fn binding(run_id: &str) -> TurnBinding {
    TurnBinding {
        authority: crate::domain::turn_authority::TurnAuthority::new(),
        wire_names: None,
        session_id: "session-1".into(),
        run_id: run_id.into(),
        data_partition: "default".into(),
        cli: CliKind::Claude,
        safety_mode: AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "chat".into(),
    }
}

/// The app's handler shape: each message answered in the connection's turn.
fn turn_handler(data: McpData) -> channel::TurnHandler {
    let data = Arc::new(data);
    Arc::new(move |message: Value, binding: Option<Arc<TurnBinding>>| {
        let data = Arc::clone(&data);
        Box::pin(async move {
            let ctx = ToolCtx {
                data: &data,
                app: None,
                binding: binding.as_deref(),
                broker: None,
            };
            protocol::handle_in(&ctx, &message).await
        })
    })
}

async fn listen(dir: &Path, turns: &TurnRegistry) -> channel::Listener {
    channel::Listener::start_with_turns(
        dir,
        turn_handler(data(notes_pool().await, ActivityAccess::Off)),
        turns.clone(),
    )
    .await
    .unwrap()
}

/// The real relay (`clovy-mcp`'s code) with `--turn`, fed `lines`.
async fn relay(dir: &Path, turn: Option<&str>, lines: &[Value]) -> Captured {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let output = Captured::default();
    let (dir, turn, sink) = (dir.to_path_buf(), turn.map(str::to_string), output.clone());
    tokio::task::spawn_blocking(move || {
        stdio::relay_turn(
            &dir,
            turn.as_deref(),
            std::io::Cursor::new(input),
            Box::new(sink),
        );
    })
    .await
    .unwrap();
    output
}

fn list_and_probe() -> [Value; 2] {
    [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": PROBE, "arguments": {} } }),
    ]
}

fn names(listed: &Value) -> Vec<String> {
    listed["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("no tool list: {listed}"))
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

/// A raw connection after its handshake: the reply line and both halves
/// (the write half keeps the connection open).
type Raw = (
    Value,
    BufReader<tokio::io::ReadHalf<channel::TestStream>>,
    tokio::io::WriteHalf<channel::TestStream>,
);

async fn handshake(dir: &Path, hello: Value) -> Raw {
    let stream = channel::test_stream(dir).await;
    let (read, mut write) = tokio::io::split(stream);
    write
        .write_all(format!("{hello}\n").as_bytes())
        .await
        .unwrap();
    let mut reader = BufReader::new(read);
    let mut reply = String::new();
    reader.read_line(&mut reply).await.unwrap();
    (serde_json::from_str(&reply).unwrap(), reader, write)
}

#[test]
fn tokens_are_random_hex_and_revocation_is_per_token_or_per_run() {
    let turns = TurnRegistry::default();
    let first = turns.issue(binding("run-1"));
    let second = turns.issue(binding("run-1"));
    let other = turns.issue(binding("run-2"));
    assert_eq!(first.len(), 64);
    assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert_ne!(first, second);
    assert_eq!(turns.binding(&first).unwrap().run_id, "run-1");

    assert!(turns.revoke(&first));
    assert!(!turns.revoke(&first));
    assert!(turns.binding(&first).is_none());
    assert_eq!(turns.revoke_run("run-1"), 1);
    assert!(turns.binding(&second).is_none());
    assert!(turns.binding(&other).is_some());
    turns.revoke_all();
    assert!(turns.binding(&other).is_none());
}

#[test]
fn the_relay_reads_an_optional_turn_argument() {
    let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
    assert_eq!(stdio::parse_turn(&args(&["--dir", "/x"])), Ok(None));
    assert_eq!(
        stdio::parse_turn(&args(&["--dir", "/x", "--turn", "ab12"])),
        Ok(Some("ab12".to_string()))
    );
    assert_eq!(
        stdio::parse_turn(&args(&["--turn=ab12", "--dir", "/x"])),
        Ok(Some("ab12".to_string()))
    );
    assert!(stdio::parse_turn(&args(&["--dir", "/x", "--turn"])).is_err());
    assert!(stdio::parse_turn(&args(&["--turn="])).is_err());
    let launch = launch(Path::new("/bin/clovy-mcp"), Path::new("/data/mcp")).for_turn("ab12");
    assert_eq!(launch.args, args(&["--dir", "/data/mcp", "--turn", "ab12"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unbound_client_sees_only_the_read_catalog_and_gets_turn_required() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let listener = listen(&dir, &turns).await;

    let output = relay(&dir, None, &list_and_probe()).await;
    let messages = output.messages(2);
    let listed = names(by_id(&messages, 1));
    assert!(listed.iter().all(|name| tools::is_read_tool(name)));
    for name in [tools::SEARCH_NOTES, tools::GET_NOTE] {
        assert!(listed.iter().any(|tool| tool == name), "missing {name}");
    }
    assert!(!listed.iter().any(|name| name == PROBE));
    let refused = &by_id(&messages, 2)["result"];
    assert_eq!(refused["isError"], json!(true));
    assert!(
        refused["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("(turn_required)"),
        "{refused}"
    );
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bound_relay_runs_turn_tools_in_its_session_and_run() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let data = data(notes_pool().await, ActivityAccess::Off);
    let repo = AgentRepository::new(data.notes.clone());
    let session = repo
        .create_session_in_profile(
            "Bound relay fixture",
            "test",
            AgentSafetyMode::Sandboxed,
            None,
            &data.profile,
        )
        .await
        .unwrap();
    let run = repo.create_run(&session.id, "test", None).await.unwrap();
    let binding = TurnBinding {
        authority: crate::domain::turn_authority::TurnAuthority::new(),
        wire_names: None,
        session_id: session.id.clone(),
        run_id: run.id.clone(),
        data_partition: crate::agent_sessions::run_partition::resolve(
            &data.notes,
            &session.id,
            &run.id,
        )
        .await
        .unwrap(),
        cli: CliKind::Claude,
        safety_mode: AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "chat".into(),
    };
    let token = turns.issue(binding);
    let listener = channel::Listener::start_with_turns(&dir, turn_handler(data), turns.clone())
        .await
        .unwrap();

    let output = relay(&dir, Some(&token), &list_and_probe()).await;
    let messages = output.messages(2);
    let listed = names(by_id(&messages, 1));
    for name in [tools::SEARCH_NOTES, tools::GET_NOTE] {
        assert!(listed.iter().any(|tool| tool == name), "missing {name}");
    }
    assert_eq!(listed.iter().filter(|name| *name == PROBE).count(), 1);
    let probed: Value = serde_json::from_str(
        by_id(&messages, 2)["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        probed,
        json!({ "sessionId": session.id, "runId": run.id, "cli": "claude", "attended": true })
    );
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_or_unknown_token_is_refused_with_turn_expired() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let listener = listen(&dir, &turns).await;
    let secret = channel::read_secret(&dir).unwrap();

    let token = turns.issue(binding("run-1"));
    assert_eq!(turns.revoke_run("run-1"), 1);
    for offered in [token.as_str(), &"ab".repeat(32)] {
        let (reply, _, _) = handshake(&dir, channel::turn_handshake(&secret, Some(offered))).await;
        assert_eq!(
            reply,
            json!({ "clovyMcp": 1, "ok": false, "error": TURN_EXPIRED })
        );
    }
    // A token never stands in for the installation secret.
    let live = turns.issue(binding("run-2"));
    let (reply, _, _) =
        handshake(&dir, channel::turn_handshake(&"0".repeat(64), Some(&live))).await;
    assert_eq!(reply["error"], json!("unauthorized"));

    // Through the relay, every call says the turn ended and no tool runs.
    let output = relay(&dir, Some(&token), &list_and_probe()).await;
    let messages = output.messages(2);
    assert_eq!(
        by_id(&messages, 1)["error"]["message"],
        json!(stdio::TURN_EXPIRED_MESSAGE)
    );
    let refused = &by_id(&messages, 2)["result"];
    assert_eq!(refused["isError"], json!(true));
    assert_eq!(
        refused["content"][0]["text"],
        json!(stdio::TURN_EXPIRED_MESSAGE)
    );
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoking_a_turn_closes_its_open_connections_only() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let listener = listen(&dir, &turns).await;
    let secret = channel::read_secret(&dir).unwrap();
    let token = turns.issue(binding("run-1"));

    let (bound_reply, mut bound, _bound_write) =
        handshake(&dir, channel::turn_handshake(&secret, Some(&token))).await;
    assert_eq!(bound_reply, json!({ "clovyMcp": 1, "ok": true }));
    let (unbound_reply, mut unbound, _unbound_write) =
        handshake(&dir, channel::handshake(&secret)).await;
    assert_eq!(unbound_reply["ok"], json!(true));

    assert!(turns.revoke(&token));
    let mut rest = String::new();
    let read = tokio::time::timeout(Duration::from_secs(5), bound.read_line(&mut rest))
        .await
        .expect("the bound connection closes")
        .unwrap();
    assert_eq!(read, 0, "bound connection still open: {rest}");
    let still_open =
        tokio::time::timeout(Duration::from_millis(300), unbound.read_line(&mut rest)).await;
    assert!(still_open.is_err(), "the unbound connection was closed");
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revocation_drains_the_delivered_call_receipt_before_closing() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let delivered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let handler: channel::TurnHandler = {
        let delivered = delivered.clone();
        let release = release.clone();
        Arc::new(move |message, _| {
            let delivered = delivered.clone();
            let release = release.clone();
            Box::pin(async move {
                delivered.notify_one();
                release.notified().await;
                Some(protocol::result_response(
                    message["id"].clone(),
                    json!({"isError":true,"structuredContent":{"error":{"code":"fixture_effect_uncertain","details":{"retrySafe":false}}}}),
                ))
            })
        })
    };
    let listener = channel::Listener::start_with_turns(&dir, handler, turns.clone())
        .await
        .unwrap();
    let secret = channel::read_secret(&dir).unwrap();
    let token = turns.issue(binding("run-receipt"));
    let (_, mut reader, mut writer) =
        handshake(&dir, channel::turn_handshake(&secret, Some(&token))).await;
    writer
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":41,\"method\":\"tools/call\",\"params\":{}}\n")
        .await
        .unwrap();
    delivered.notified().await;
    assert!(turns.revoke(&token));
    release.notify_one();
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 41);
    assert_eq!(
        response["result"]["structuredContent"]["error"]["code"],
        "fixture_effect_uncertain"
    );
    assert_eq!(
        response["result"]["structuredContent"]["error"]["details"]["retrySafe"],
        false
    );
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_call_without_a_receipt_gets_correlated_uncertain_result() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let admitted = Arc::new(tokio::sync::Notify::new());
    let handler: channel::TurnHandler = {
        let admitted = admitted.clone();
        Arc::new(move |_message, _| {
            let admitted = admitted.clone();
            Box::pin(async move {
                admitted.notify_one();
                std::future::pending::<Option<Value>>().await
            })
        })
    };
    let listener = channel::Listener::start_with_turns(&dir, handler, turns.clone())
        .await
        .unwrap();
    let secret = channel::read_secret(&dir).unwrap();
    let token = turns.issue(binding("run-unknown"));
    let (_, mut reader, mut writer) =
        handshake(&dir, channel::turn_handshake(&secret, Some(&token))).await;
    writer
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"tools/call\",\"params\":{}}\n")
        .await
        .unwrap();
    admitted.notified().await;
    assert!(turns.revoke(&token));
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 42);
    let details = &response["result"]["structuredContent"]["error"]["details"];
    assert_eq!(details["callId"], 42);
    assert_eq!(details["delivery"], "unknown");
    assert_eq!(details["retrySafe"], false);
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revocation_retains_a_definitive_pre_dispatch_refusal() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let admitted = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let handler: channel::TurnHandler = {
        let admitted = admitted.clone();
        let release = release.clone();
        Arc::new(move |message, _| {
            let admitted = admitted.clone();
            let release = release.clone();
            Box::pin(async move {
                admitted.notify_one();
                release.notified().await;
                Some(protocol::result_response(
                    message["id"].clone(),
                    json!({"isError":true,"structuredContent":{"error":{"code":"turn_expired","details":{"delivery":"not_delivered","retrySafe":true}}}}),
                ))
            })
        })
    };
    let listener = channel::Listener::start_with_turns(&dir, handler, turns.clone())
        .await
        .unwrap();
    let secret = channel::read_secret(&dir).unwrap();
    let token = turns.issue(binding("run-safe"));
    let (_, mut reader, mut writer) =
        handshake(&dir, channel::turn_handshake(&secret, Some(&token))).await;
    writer
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":43,\"method\":\"tools/call\",\"params\":{}}\n")
        .await
        .unwrap();
    admitted.notified().await;
    assert!(turns.revoke(&token));
    release.notify_one();
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), reader.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(response["id"], 43);
    let details = &response["result"]["structuredContent"]["error"]["details"];
    assert_eq!(details["delivery"], "not_delivered");
    assert_eq!(details["retrySafe"], true);
    listener.stop();
}
