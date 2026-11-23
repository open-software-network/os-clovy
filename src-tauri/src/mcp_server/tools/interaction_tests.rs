//! The interaction group (parity slice 2.11): `request_clarification`
//! returns the user's answer from a clarification card, and
//! `approve_action` answers Claude Code's permission prompt in the format
//! claude reads (spike S1), end to end through the real socket and relay
//! with the call claude sends.

#[cfg(any(unix, windows))]
use std::path::Path;
#[cfg(any(unix, windows))]
use std::sync::{Arc, Mutex};
#[cfg(any(unix, windows))]
use std::time::Duration;

use serde_json::{json, Value};

use super::interaction::InteractionTools;
use super::{ActivityAccess, McpData, ToolCtx, ToolGroup, TurnBinding};
use crate::approval_broker::testing::{Scripted, TestBroker};
#[cfg(any(unix, windows))]
use crate::approval_broker::ApprovalBroker;
use crate::llm::cli::CliKind;
#[cfg(any(unix, windows))]
use crate::mcp_server::tests::by_id;
#[cfg(any(unix, windows))]
use crate::mcp_server::turns::TurnRegistry;
#[cfg(any(unix, windows))]
use crate::mcp_server::{channel, protocol, stdio};

fn data(broker: &TestBroker) -> McpData {
    crate::mcp_server::tests::data(broker.pool().clone(), ActivityAccess::Off)
}

fn ctx<'a>(data: &'a McpData, broker: &'a TestBroker, binding: &'a TurnBinding) -> ToolCtx<'a> {
    ToolCtx {
        data,
        app: None,
        binding: Some(binding),
        broker: Some(broker.broker()),
    }
}

fn names(tools: &[Value]) -> Vec<&str> {
    tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect()
}

/// What claude sends to its permission prompt tool (S1).
fn bash_permission() -> Value {
    json!({
        "tool_name": "Bash",
        "input": { "command": "rm -rf build", "description": "Remove the build folder" },
        "tool_use_id": "toolu_01",
    })
}

#[tokio::test]
async fn interaction_clarification_returns_the_users_answer() {
    let broker = TestBroker::new([Scripted::Answer("Tuesday".into())]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let data = data(&broker);
    let answer = InteractionTools
        .call(
            &ctx(&data, &broker, &binding),
            "request_clarification",
            &json!({ "question": " Which day? ", "choices": ["Monday", "Tuesday"] }),
        )
        .await
        .unwrap();
    assert_eq!(answer, json!({ "answer": "Tuesday" }));
    let cards = broker.requests();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["kind"], "clarification");
    assert_eq!(cards[0]["question"], "Which day?");
    assert_eq!(cards[0]["choices"], json!(["Monday", "Tuesday"]));
    assert_eq!(cards[0]["engine"], "cli");
}

#[tokio::test]
async fn interaction_clarification_unattended_or_invalid_asks_nobody() {
    let broker = TestBroker::new([Scripted::Answer("unused".into())]).await;
    let routine = broker.turn(CliKind::Claude, false).await;
    let data = data(&broker);
    let error = InteractionTools
        .call(
            &ctx(&data, &broker, &routine),
            "request_clarification",
            &json!({ "question": "Which day?", "choices": [] }),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "unattended_requires_approval");

    let attended = broker.turn(CliKind::Claude, true).await;
    for arguments in [
        json!({ "question": "  ", "choices": [] }),
        json!({ "question": "Which?", "choices": "Monday" }),
    ] {
        let error = InteractionTools
            .call(
                &ctx(&data, &broker, &attended),
                "request_clarification",
                &arguments,
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "agent_tool_arguments_invalid", "{arguments}");
    }
    assert!(broker.requests().is_empty());
}

#[tokio::test]
async fn interaction_approve_action_is_claude_only_and_answers_allow_or_deny() {
    let broker = TestBroker::new([Scripted::Approve, Scripted::Deny]).await;
    let claude = broker.turn(CliKind::Claude, true).await;
    let codex = broker.turn(CliKind::Codex, true).await;
    let data = data(&broker);

    assert_eq!(
        names(&InteractionTools.descriptors(&ctx(&data, &broker, &claude))),
        ["request_clarification", "approve_action"]
    );
    assert_eq!(
        names(&InteractionTools.descriptors(&ctx(&data, &broker, &codex))),
        ["request_clarification"]
    );
    let refused = InteractionTools
        .call(
            &ctx(&data, &broker, &codex),
            "approve_action",
            &bash_permission(),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, "approve_action_claude_only");

    let claude_ctx = ctx(&data, &broker, &claude);
    let allowed = InteractionTools
        .call(&claude_ctx, "approve_action", &bash_permission())
        .await
        .unwrap();
    assert_eq!(
        allowed,
        json!({ "behavior": "allow", "updatedInput": bash_permission()["input"] })
    );
    let denied = InteractionTools
        .call(&claude_ctx, "approve_action", &bash_permission())
        .await
        .unwrap();
    assert_eq!(denied["behavior"], "deny");
    assert!(denied["message"]
        .as_str()
        .unwrap()
        .ends_with("(user_denied)"));

    let cards = broker.requests();
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0]["toolName"], "Bash");
    assert!(uuid::Uuid::parse_str(cards[0]["toolCallId"].as_str().unwrap()).is_ok());
    assert_eq!(cards[0]["title"], "Allow Claude Code to use Bash");
    assert!(cards[0]["command"]
        .as_str()
        .unwrap()
        .contains("rm -rf build"));

    // Nobody to ask (no broker, or an unattended turn): deny, never allow.
    let no_broker = ToolCtx {
        broker: None,
        ..claude_ctx
    };
    let answer = InteractionTools
        .call(&no_broker, "approve_action", &bash_permission())
        .await
        .unwrap();
    assert_eq!(answer["behavior"], "deny");
    assert!(answer["message"]
        .as_str()
        .unwrap()
        .ends_with("(approval_unavailable)"));
    let routine = broker.turn(CliKind::Claude, false).await;
    let answer = InteractionTools
        .call(
            &ctx(&data, &broker, &routine),
            "approve_action",
            &bash_permission(),
        )
        .await
        .unwrap();
    assert_eq!(answer["behavior"], "deny");
    assert_eq!(broker.requests().len(), 2);
}

#[tokio::test]
async fn interaction_unbound_clients_neither_see_nor_reach_the_tools() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let data = data(&broker);
    let unbound = ToolCtx::unbound(&data);
    let listed = super::descriptors(&unbound).await;
    for name in ["request_clarification", "approve_action"] {
        assert!(!names(&listed).contains(&name), "{name}");
        let error = super::call(&unbound, name, &bash_permission())
            .await
            .unwrap_err();
        assert_eq!(error.code, "turn_required", "{name}");
    }
    assert!(broker.requests().is_empty());
}

/// The app's handler shape, with the test broker as the app's broker.
#[cfg(any(unix, windows))]
fn turn_handler(data: McpData, broker: ApprovalBroker) -> channel::TurnHandler {
    let data = Arc::new(data);
    Arc::new(move |message: Value, binding: Option<Arc<TurnBinding>>| {
        let (data, broker) = (Arc::clone(&data), broker.clone());
        Box::pin(async move {
            let ctx = ToolCtx {
                data: &data,
                app: None,
                binding: binding.as_deref(),
                broker: Some(&broker),
            };
            protocol::handle_in(&ctx, &message).await
        })
    })
}

/// A fake claude turn: the real relay with `--turn`, sending what claude
/// sends for a permission prompt and for a question.
#[tokio::test]
#[cfg(any(unix, windows))]
async fn interaction_claude_permission_prompt_through_the_socket_and_relay() {
    let broker = TestBroker::new([
        Scripted::Approve,
        Scripted::Deny,
        Scripted::Answer("Monday".into()),
    ])
    .await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let turns = TurnRegistry::default();
    let token = turns.issue(binding.clone());
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let _listener = channel::Listener::start_with_turns(
        &dir,
        turn_handler(data(&broker), broker.broker().clone()),
        turns.clone(),
    )
    .await
    .unwrap();

    let call = |id: i64, name: &str, arguments: Value| json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call", "params": { "name": name, "arguments": arguments } });
    let lines = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        call(2, "approve_action", bash_permission()),
        call(
            3,
            "approve_action",
            json!({ "tool_name": "Write", "input": { "file_path": "/tmp/x.txt", "content": "x" }, "tool_use_id": "toolu_02" }),
        ),
        call(
            4,
            "request_clarification",
            json!({ "question": "Which day?", "choices": ["Monday", "Friday"] }),
        ),
    ];
    // One relay connection per call, so the cards come one after another
    // (the scripted decisions apply in order); the token holds for each.
    let mut messages = Vec::new();
    for batch in [&lines[..2], &lines[2..3], &lines[3..]] {
        messages.extend(relay(&dir, &token, batch).await);
    }

    let listed = names(by_id(&messages, 1)["result"]["tools"].as_array().unwrap())
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    for name in [
        "approve_action",
        "request_clarification",
        "generate_image",
        "start_recording",
    ] {
        assert!(listed.iter().any(|tool| tool == name), "{name}: {listed:?}");
    }
    // The tool result claude parses: text holding the decision as JSON.
    let decision = |id: i64| -> Value {
        let result = &by_id(&messages, id)["result"];
        assert_eq!(result["isError"], false, "{result}");
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    assert_eq!(
        decision(2),
        json!({ "behavior": "allow", "updatedInput": bash_permission()["input"] })
    );
    let denied = decision(3);
    assert_eq!(denied["behavior"], "deny");
    assert!(denied["message"]
        .as_str()
        .unwrap()
        .ends_with("(user_denied)"));
    assert_eq!(decision(4), json!({ "answer": "Monday" }));
    assert_eq!(broker.requests().len(), 3);
}

/// Stdin of a CLI: the requests, then open until the test closes it (the
/// relay drops answers still in flight when its input ends).
#[cfg(any(unix, windows))]
struct OpenInput {
    lines: std::io::Cursor<Vec<u8>>,
    gate: std::sync::mpsc::Receiver<()>,
}

#[cfg(any(unix, windows))]
impl std::io::Read for OpenInput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.lines.read(buf)?;
        if read == 0 {
            let _ = self.gate.recv();
        }
        Ok(read)
    }
}

#[derive(Clone, Default)]
#[cfg(any(unix, windows))]
struct Output(Arc<Mutex<Vec<u8>>>);

#[cfg(any(unix, windows))]
impl std::io::Write for Output {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(any(unix, windows))]
impl Output {
    fn messages(&self) -> Vec<Value> {
        String::from_utf8_lossy(&self.0.lock().unwrap())
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
}

/// Runs the real relay with the turn token on `requests`, keeping its input
/// open until every request is answered.
#[cfg(any(unix, windows))]
async fn relay(dir: &Path, token: &str, requests: &[Value]) -> Vec<Value> {
    let input: String = requests.iter().map(|line| format!("{line}\n")).collect();
    let output = Output::default();
    let (gate, gated) = std::sync::mpsc::channel();
    let (dir, token, sink) = (dir.to_path_buf(), token.to_string(), output.clone());
    let relay = tokio::task::spawn_blocking(move || {
        stdio::relay_turn(
            &dir,
            Some(&token),
            std::io::BufReader::new(OpenInput {
                lines: std::io::Cursor::new(input.into_bytes()),
                gate: gated,
            }),
            Box::new(sink),
        );
    });
    for _ in 0..500 {
        if output.messages().len() >= requests.len() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = gate.send(());
    relay.await.unwrap();
    let messages = output.messages();
    assert_eq!(messages.len(), requests.len(), "{messages:?}");
    messages
}
