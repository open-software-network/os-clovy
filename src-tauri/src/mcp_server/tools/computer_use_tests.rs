//! `computer_use` over the approval broker (`TestBroker`) and a fake driver:
//! one card per call answered once on the Mac, the action run only after it,
//! and the lease the turn binding grants.

use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::computer_use::{ComputerUseHost, ComputerUseTools, COMPUTER_USE_TOOL};
use super::{ActivityAccess, McpData, ToolCtx, ToolGroup};
use crate::approval_broker::testing::{Scripted, TestBroker};
use crate::computer_use::{
    begin_turn_lease_in, check_eligibility, clear_leases_for_test, end_turn_lease_in,
    has_attended_run, watch_turn_in, BoxedBool, ComputerUseState, EligibilityProbe,
};
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;
use crate::mcp_server::protocol;
use crate::mcp_server::turns::TurnBinding;

/// A fake Computer use driver: records what the group asked.
#[derive(Default)]
struct FakeHost {
    driver_result: Mutex<Option<Value>>,
    leases: Mutex<Vec<String>>,
    executed: Mutex<Vec<Value>>,
}

impl FakeHost {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn executed(&self) -> Vec<Value> {
        self.executed.lock().unwrap().clone()
    }
}

impl ComputerUseHost for FakeHost {
    fn begin_turn(&self, binding: &TurnBinding) -> Result<(), AppError> {
        self.leases.lock().unwrap().push(binding.run_id.clone());
        Ok(())
    }

    fn execute(&self, _run_id: String, arguments: Value) -> BoxFuture<'_, Value> {
        Box::pin(async move {
            self.executed.lock().unwrap().push(arguments);
            self.driver_result
                .lock()
                .unwrap()
                .clone()
                .unwrap_or_else(|| {
                    json!({
                        "content": [
                            { "type": "text", "text": "[1] button \"Save\"" },
                            { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" }
                        ],
                        "isError": false,
                        "structuredContent": { "target_app": "TextEdit" }
                    })
                })
        })
    }
}

fn data(broker: &TestBroker) -> McpData {
    McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    }
}

fn ctx<'a>(data: &'a McpData, broker: &'a TestBroker, binding: &'a TurnBinding) -> ToolCtx<'a> {
    ToolCtx {
        data,
        app: None,
        binding: Some(binding),
        broker: Some(broker.broker()),
    }
}

fn capture() -> Value {
    json!({ "action": "capture", "app": "TextEdit" })
}

/// A desktop answer through the resolution command's broker path.
async fn desktop_answer(broker: &TestBroker, card: &Value, choice: &str) -> Result<bool, AppError> {
    crate::approval_broker::commands::resolve(
        broker.broker(),
        broker.pool(),
        card["runId"].as_str().unwrap(),
        card["id"].as_str().unwrap(),
        &json!({ "kind": "approval", "choice": choice }),
    )
    .await
}

#[tokio::test]
async fn computer_use_is_listed_only_for_attended_turns() {
    let broker = TestBroker::new([]).await;
    let data = data(&broker);
    let chat = broker.turn(CliKind::Claude, true).await;
    let routine = broker.turn(CliKind::Codex, false).await;
    async fn listed(ctx: &ToolCtx<'_>) -> Vec<Value> {
        super::descriptors(ctx)
            .await
            .into_iter()
            .filter(|tool| tool["name"] == COMPUTER_USE_TOOL)
            .collect()
    }
    assert!(listed(&ToolCtx::unbound(&data)).await.is_empty());
    assert!(listed(&ctx(&data, &broker, &routine)).await.is_empty());
    let tool = listed(&ctx(&data, &broker, &chat)).await;
    assert_eq!(tool.len(), 1);
    assert_eq!(tool[0]["annotations"]["readOnlyHint"], false);
    assert_eq!(tool[0]["inputSchema"]["required"], json!(["action"]));
}

#[tokio::test]
async fn unbound_and_unattended_calls_get_no_card_lease_or_action() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let data = data(&broker);
    let unbound = super::call(&ToolCtx::unbound(&data), COMPUTER_USE_TOOL, &capture())
        .await
        .unwrap_err();
    assert_eq!(unbound.code, "turn_required");

    let host = FakeHost::new();
    let group = ComputerUseTools::with_host(host.clone());
    let routine = broker.turn(CliKind::Codex, false).await;
    let refused = group
        .call(
            &ctx(&data, &broker, &routine),
            COMPUTER_USE_TOOL,
            &capture(),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code, "unattended_requires_approval");
    assert!(broker.requests().is_empty());
    assert!(host.leases.lock().unwrap().is_empty());
    assert!(host.executed().is_empty());
}

#[tokio::test]
async fn desktop_approval_runs_one_action_and_a_second_answer_is_refused() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let data = data(&broker);
    let chat = broker.turn(CliKind::Claude, true).await;
    let host = FakeHost::new();
    let group = ComputerUseTools::with_host(host.clone());

    let result = group
        .call(&ctx(&data, &broker, &chat), COMPUTER_USE_TOOL, &capture())
        .await
        .unwrap();
    let cards = broker.requests();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0]["toolName"], COMPUTER_USE_TOOL);
    assert_eq!(cards[0]["engine"], "cli");
    let executed = host.executed();
    assert_eq!(executed, [capture()]);
    assert_eq!(
        *host.leases.lock().unwrap(),
        std::slice::from_ref(&chat.run_id)
    );
    // The MCP result carries the capture as an image block, beside the
    // driver's text and structured content.
    let mcp = crate::mcp_server::protocol::tool_result(&result);
    assert_eq!(mcp["isError"], false);
    assert_eq!(
        mcp["content"],
        json!([
            { "type": "text", "text": "[1] button \"Save\"" },
            { "type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png" },
            { "type": "text", "text": "{\"target_app\":\"TextEdit\"}" }
        ])
    );

    let second = desktop_answer(&broker, &cards[0], "once")
        .await
        .unwrap_err();
    assert_eq!(second.code, "agent_interruption_expired");
    assert_eq!(host.executed().len(), 1);
}

#[tokio::test]
async fn denied_or_invalid_calls_never_reach_the_driver() {
    let broker = TestBroker::new([Scripted::Deny]).await;
    let data = data(&broker);
    let chat = broker.turn(CliKind::Codex, true).await;
    let host = FakeHost::new();
    let group = ComputerUseTools::with_host(host.clone());
    let ctx = ctx(&data, &broker, &chat);

    let denied = group
        .call(&ctx, COMPUTER_USE_TOOL, &capture())
        .await
        .unwrap_err();
    assert_eq!(denied.code, "user_denied");
    assert!(host.executed().is_empty());

    let invalid = group
        .call(&ctx, COMPUTER_USE_TOOL, &json!({ "action": "rm -rf" }))
        .await
        .unwrap_err();
    assert_eq!(invalid.code, "computer_use_action_invalid");
    assert_eq!(broker.requests().len(), 1);
    assert!(host.executed().is_empty());
}

#[tokio::test]
async fn driver_failure_is_a_tool_error() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let data = data(&broker);
    let chat = broker.turn(CliKind::Claude, true).await;
    let host = FakeHost::new();
    *host.driver_result.lock().unwrap() = Some(json!({
        "content": [{ "type": "text", "text": "{\"error\":\"Computer use access to this app was denied, cancelled, or timed out.\"}" }],
        "isError": true,
    }));
    let group = ComputerUseTools::with_host(host.clone());
    let error = group
        .call(&ctx(&data, &broker, &chat), COMPUTER_USE_TOOL, &capture())
        .await
        .unwrap_err();
    assert_eq!(error.code, "computer_use_failed");
    assert!(error.message.contains("denied"), "{}", error.message);
}

#[tokio::test]
async fn driver_error_keeps_code_and_delivery_evidence() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let data = data(&broker);
    let chat = broker.turn(CliKind::Claude, true).await;
    let host = FakeHost::new();
    *host.driver_result.lock().unwrap() = Some(json!({
        "isError": true,
        "structuredContent": { "error": {
            "code": "TIMEOUT", "message": "Action result unavailable.",
            "details": { "disposition": { "delivery": "unknown", "retry": "unknown" } }
        }}
    }));
    let group = ComputerUseTools::with_host(host);
    let error = group
        .call(&ctx(&data, &broker, &chat), COMPUTER_USE_TOOL, &capture())
        .await
        .unwrap_err();
    assert_eq!(error.code, "TIMEOUT");
    assert_eq!(error.details.unwrap()["disposition"]["retry"], "unknown");
}

fn binding(run_id: &str, attended: bool) -> TurnBinding {
    TurnBinding {
        authority: crate::domain::turn_authority::TurnAuthority::new(),
        wire_names: None,
        session_id: "session-1".into(),
        run_id: run_id.into(),
        data_partition: "default".into(),
        cli: CliKind::Claude,
        safety_mode: crate::agent_sessions::AgentSafetyMode::Sandboxed,
        attended,
        surface: if attended { "chat" } else { "routine" }.into(),
    }
}

#[test]
fn the_turn_binding_grants_the_lease_and_stop_stays_sticky_for_the_run() {
    let state = ComputerUseState::default();
    assert!(!has_attended_run(&state));
    assert_eq!(
        begin_turn_lease_in(&state, &binding("run-a", false))
            .unwrap_err()
            .code,
        "computer_use_attended_run_required"
    );
    assert!(!has_attended_run(&state));

    assert!(begin_turn_lease_in(&state, &binding("run-a", true)).unwrap());
    assert!(has_attended_run(&state));
    // Later calls of the same run keep the lease without reopening it.
    assert!(!begin_turn_lease_in(&state, &binding("run-a", true)).unwrap());

    // Stop clears the lease; the same run cannot take it back.
    clear_leases_for_test(&state);
    assert_eq!(
        begin_turn_lease_in(&state, &binding("run-a", true))
            .unwrap_err()
            .code,
        "computer_use_stopped"
    );
    assert!(!has_attended_run(&state));

    // The run's end forgets it; a new turn gets a fresh lease.
    assert_eq!(end_turn_lease_in(&state, "run-a").unwrap(), None);
    assert!(begin_turn_lease_in(&state, &binding("run-b", true)).unwrap());
    assert!(end_turn_lease_in(&state, "run-b").unwrap().is_some());
    assert!(!has_attended_run(&state));
}

#[test]
fn a_run_that_ends_before_its_lease_is_granted_never_gets_one() {
    let state = ComputerUseState::default();
    let turn = binding("run-c", true);
    // The end listener is registered first ...
    assert!(watch_turn_in(&state, &turn).unwrap());
    // ... and the run ends before the lease is granted.
    assert_eq!(end_turn_lease_in(&state, "run-c").unwrap(), None);
    assert_eq!(
        begin_turn_lease_in(&state, &turn).unwrap_err().code,
        "computer_use_turn_ended"
    );
    assert!(!has_attended_run(&state));
    assert_eq!(
        watch_turn_in(&state, &turn).unwrap_err().code,
        "computer_use_turn_ended"
    );

    // The normal order: watched, granted, then ended.
    let turn = binding("run-d", true);
    assert!(watch_turn_in(&state, &turn).unwrap());
    assert!(begin_turn_lease_in(&state, &turn).unwrap());
    assert!(!watch_turn_in(&state, &turn).unwrap());
    assert!(has_attended_run(&state));
    assert!(end_turn_lease_in(&state, "run-d").unwrap().is_some());
    assert!(!has_attended_run(&state));
}

/// D4: only the platform, the grant and the driver are consulted; there is
/// nothing about a plan, a vision model or a remote rollout to ask.
#[derive(Default)]
struct LocalProbe {
    asked: Mutex<Vec<&'static str>>,
}

impl EligibilityProbe for LocalProbe {
    fn platform_supported(&self) -> bool {
        true
    }
    fn grant_enabled(&self) -> BoxedBool<'_> {
        self.asked.lock().unwrap().push("grant");
        Box::pin(async { true })
    }
    fn driver(&self) -> Result<(), AppError> {
        self.asked.lock().unwrap().push("driver");
        Ok(())
    }
}

#[tokio::test]
async fn eligibility_asks_only_the_grant_and_the_driver() {
    let probe = LocalProbe::default();
    check_eligibility(&probe).await.unwrap();
    assert_eq!(*probe.asked.lock().unwrap(), ["grant", "driver"]);
}

#[test]
fn content_values_become_mcp_blocks_and_plain_values_stay_one_text_block() {
    let plain = json!({ "saved": true });
    assert_eq!(
        protocol::tool_result(&plain),
        json!({ "content": [{ "type": "text", "text": "{\"saved\":true}" }], "isError": false })
    );
    let blocks = protocol::content_value(vec![
        protocol::text_block("caption"),
        protocol::image_block("AAAA", "image/png"),
    ]);
    assert_eq!(
        protocol::tool_result(&blocks)["content"][1],
        json!({ "type": "image", "data": "AAAA", "mimeType": "image/png" })
    );
    // Anything but well-formed text and image blocks stays JSON text.
    for odd in [
        json!({ protocol::CONTENT_KEY: [{ "type": "audio", "data": "AAAA" }] }),
        json!({ protocol::CONTENT_KEY: [{ "type": "image", "data": "AAAA", "mimeType": "text/html" }] }),
        json!({ protocol::CONTENT_KEY: [], }),
        json!({ protocol::CONTENT_KEY: [protocol::text_block("x")], "other": 1 }),
    ] {
        let result = protocol::tool_result(&odd);
        assert_eq!(result["content"].as_array().unwrap().len(), 1);
        assert_eq!(result["content"][0]["type"], "text");
    }
}
