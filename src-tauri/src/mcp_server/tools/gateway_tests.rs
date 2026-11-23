use super::{ActivityAccess, McpData, ToolCtx};
use crate::approval_broker::testing::TestBroker;
use serde_json::json;

#[tokio::test]
async fn unbound_gateway_calls_require_a_turn_and_are_not_listed() {
    let broker = TestBroker::new([]).await;
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx::unbound(&data);
    for name in ["mcp_private_send", "mcp_linear_create_issue"] {
        assert_eq!(
            super::call(&ctx, name, &json!({})).await.unwrap_err().code,
            "turn_required"
        );
        assert!(!super::descriptors(&ctx)
            .await
            .iter()
            .any(|tool| tool["name"] == name));
    }
}

use super::connectors_tests::HttpFixture;
use super::gateway::{GatewayDriver, GatewayTools};
use super::ToolGroup;
use crate::agent_mcp::{
    AgentMcpError, AgentMcpRepository, McpSecretBundle, McpSecretStore, McpServerDefinition,
    McpTransport,
};
use crate::approval_broker::testing::Scripted;
use crate::llm::cli::CliKind;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const WIRE_FIXTURE_SERVER: &str = "wirefixture";
// One leading c plus 22 URL-safe characters, matching the native namespace's
// measured size without touching an installed engine or its configuration.
const WIRE_NATIVE_SERVER: &str = "c0123456789012345678901";

fn wire_fixture_methods() -> Vec<String> {
    let prefix = format!("mcp_{WIRE_FIXTURE_SERVER}_").len();
    [36, 37, 128]
        .into_iter()
        .map(|length| format!("write_{}", "x".repeat(length - prefix - 6)))
        .chain(std::iter::once("read_state".into()))
        .collect()
}

async fn wire_fixture_server(broker: &TestBroker, http: &HttpFixture) {
    let mut server = McpServerDefinition::new(WIRE_FIXTURE_SERVER, McpTransport::StreamableHttp);
    server.url = Some(http.url.clone());
    server.safety.requires_approval = true;
    // No secret reference: this fixture exercises the production dispatcher
    // without accessing the machine's credential store.
    AgentMcpRepository::new(broker.pool().clone())
        .create(&server)
        .await
        .unwrap();
}

async fn wire_protocol(
    ctx: &ToolCtx<'_>,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    crate::mcp_server::protocol::handle_in(
        ctx,
        &json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
    )
    .await
    .unwrap()
}

fn wire_advertised<'a>(listing: &'a serde_json::Value, remote: &str) -> &'a serde_json::Value {
    listing["result"]["tools"]
        .as_array()
        .expect("protocol tools/list must succeed")
        .iter()
        .find(|tool| {
            tool["_meta"]["clovy"]["id"]
                .as_str()
                .is_some_and(|id| id.ends_with(&format!("/{remote}")))
        })
        .expect("every admitted fixture method must remain advertised")
}

async fn wire_pending(broker: &TestBroker) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting = broker.broker().waiting_ids();
            if let Some(id) = broker.requests().iter().find_map(|card| {
                let id = card["id"].as_str()?;
                waiting
                    .iter()
                    .any(|waiting| waiting == id)
                    .then(|| id.to_owned())
            }) {
                return id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wire call must reach a real pending approval")
}

#[tokio::test]
async fn f18_protocol_advertised_names_fit_native_bound_and_call_every_canonical_method() {
    for cli in [CliKind::Claude, CliKind::Codex, CliKind::Agy] {
        let methods = wire_fixture_methods();
        let http = HttpFixture::with_stateful_methods(methods.clone()).await;
        let broker = TestBroker::new([]).await;
        let binding = broker.turn(cli, true).await;
        let data = data(&broker);
        wire_fixture_server(&broker, &http).await;
        let ctx = ToolCtx {
            data: &data,
            app: None,
            binding: Some(&binding),
            broker: Some(broker.broker()),
        };
        let listing = wire_protocol(&ctx, "tools/list", json!({})).await;
        let mut expected_state = Vec::new();
        let mut canonical_names = Vec::new();
        for remote in &methods {
            let canonical =
                crate::agent_mcp::runtime_tool_name(WIRE_FIXTURE_SERVER, remote).unwrap();
            canonical_names.push(canonical.clone());
            let descriptor = wire_advertised(&listing, remote);
            let wire = descriptor["name"].as_str().unwrap();
            let qualified = format!("mcp_{WIRE_NATIVE_SERVER}_{wire}");
            assert!(
                qualified.len() <= 64
                    && qualified
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
                "{cli:?}: advertised tool rejected by native SDK: {qualified}"
            );
            if canonical.len() <= 36 {
                assert_eq!(wire, canonical);
            } else {
                assert_ne!(wire, canonical);
            }
            let arguments = if remote == "read_state" {
                json!({})
            } else {
                json!({"stamp":format!("{cli:?}-{}", canonical.len())})
            };
            let request = wire_protocol(
                &ctx,
                "tools/call",
                json!({"name":wire,"arguments":arguments}),
            );
            let decide = async {
                let id = wire_pending(&broker).await;
                assert_eq!(*http.effects.lock().unwrap(), expected_state);
                assert_eq!(broker.requests().last().unwrap()["toolName"], canonical);
                broker
                    .broker()
                    .decide(
                        broker.pool(),
                        &id,
                        crate::approval_broker::Decision::Approve,
                    )
                    .await
                    .unwrap();
            };
            let (result, ()) = tokio::join!(request, decide);
            assert_eq!(result["result"]["isError"], false, "{result}");
            if remote != "read_state" {
                expected_state.push(arguments);
            } else {
                for effect in &expected_state {
                    assert!(
                        result
                            .to_string()
                            .contains(effect["stamp"].as_str().unwrap()),
                        "read did not observe approved state: {result}"
                    );
                }
            }
            assert_eq!(*http.effects.lock().unwrap(), expected_state);
        }
        assert_eq!(http.tool_calls(), methods.len());
        assert!(canonical_names.iter().any(|name| name.len() == 128));
        let mut snapshots: Vec<String> = sqlx::query_scalar::query_scalar(
            "SELECT tool_name FROM agent_run_mcp_policies WHERE run_id = ? ORDER BY tool_name",
        )
        .bind(&binding.run_id)
        .fetch_all(broker.pool())
        .await
        .unwrap();
        canonical_names.sort();
        snapshots.sort();
        assert_eq!(snapshots, canonical_names);
        assert_eq!(wire_protocol(&ctx, "tools/list", json!({})).await, listing);
    }
}

#[tokio::test]
async fn f18_wire_denial_and_revocation_never_apply_the_connected_effect() {
    for revoke in [false, true] {
        let methods = wire_fixture_methods();
        let remote = &methods[2];
        let http = HttpFixture::with_stateful_methods(methods.clone()).await;
        let broker = TestBroker::new([]).await;
        let binding = broker.turn(CliKind::Agy, true).await;
        let data = data(&broker);
        wire_fixture_server(&broker, &http).await;
        let ctx = ToolCtx {
            data: &data,
            app: None,
            binding: Some(&binding),
            broker: Some(broker.broker()),
        };
        let listing = wire_protocol(&ctx, "tools/list", json!({})).await;
        let canonical = crate::agent_mcp::runtime_tool_name(WIRE_FIXTURE_SERVER, remote).unwrap();
        let wire = wire_advertised(&listing, remote)["name"].as_str().unwrap();
        assert_ne!(
            wire, canonical,
            "long identity must be encoded at publication"
        );
        let request = wire_protocol(
            &ctx,
            "tools/call",
            json!({"name":wire,"arguments":{"stamp":"must not apply"}}),
        );
        let decide = async {
            let id = wire_pending(&broker).await;
            assert!(http.effects.lock().unwrap().is_empty());
            assert_eq!(http.tool_calls(), 0);
            assert_eq!(broker.requests().last().unwrap()["toolName"], canonical);
            if revoke {
                binding.authority.revoke();
            }
            broker
                .broker()
                .decide(
                    broker.pool(),
                    &id,
                    if revoke {
                        crate::approval_broker::Decision::Approve
                    } else {
                        crate::approval_broker::Decision::Deny
                    },
                )
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(request, decide);
        assert_eq!(result["result"]["isError"], true, "{result}");
        assert!(
            result.to_string().contains(if revoke {
                "turn_expired"
            } else {
                "user_denied"
            }),
            "{result}"
        );
        assert!(http.effects.lock().unwrap().is_empty());
        assert_eq!(http.tool_calls(), 0);
    }
}

#[tokio::test]
async fn f18_raw_and_unadvertised_wire_names_cannot_reuse_another_grant() {
    let methods = wire_fixture_methods();
    let http = HttpFixture::with_stateful_methods(methods.clone()).await;
    let broker = TestBroker::new([Scripted::Deny, Scripted::Deny]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let data = data(&broker);
    wire_fixture_server(&broker, &http).await;
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let listing = wire_protocol(&ctx, "tools/list", json!({})).await;
    let canonical = crate::agent_mcp::runtime_tool_name(WIRE_FIXTURE_SERVER, &methods[2]).unwrap();
    let wire = wire_advertised(&listing, &methods[2])["name"]
        .as_str()
        .unwrap();
    for name in [canonical.as_str(), "_c_unadvertised"] {
        let refused = wire_protocol(&ctx, "tools/call", json!({"name":name,"arguments":{}})).await;
        assert!(refused.get("error").is_some(), "{refused}");
    }
    let fresh = broker.turn(CliKind::Codex, true).await;
    let refused = wire_protocol(
        &ToolCtx {
            binding: Some(&fresh),
            ..ctx
        },
        "tools/call",
        json!({"name":wire,"arguments":{}}),
    )
    .await;
    assert!(refused.get("error").is_some(), "{refused}");
    assert!(broker.requests().is_empty());
    assert!(http.effects.lock().unwrap().is_empty());
    assert_eq!(http.tool_calls(), 0);
}

#[derive(Clone, Default)]
struct MemorySecrets(Arc<Mutex<HashMap<String, McpSecretBundle>>>);
impl McpSecretStore for MemorySecrets {
    fn put(&self, id: &str, b: &McpSecretBundle) -> Result<(), AgentMcpError> {
        self.0.lock().unwrap().insert(id.into(), b.clone());
        Ok(())
    }
    fn get(&self, id: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        if id == "fixture-failing-keychain" {
            return Err(AgentMcpError::SecureStorage);
        }
        Ok(self.0.lock().unwrap().get(id).cloned())
    }
    fn delete(&self, id: &str) -> Result<(), AgentMcpError> {
        self.0.lock().unwrap().remove(id);
        Ok(())
    }
}
struct MemoryDriver(MemorySecrets);
impl GatewayDriver for MemoryDriver {
    type Secrets = MemorySecrets;
    fn secrets(&self) -> MemorySecrets {
        self.0.clone()
    }
}
fn data(b: &TestBroker) -> McpData {
    McpData {
        notes: b.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    }
}
async fn group(broker: &TestBroker, http: &HttpFixture) -> GatewayTools<MemoryDriver> {
    let repo = AgentMcpRepository::new(broker.pool().clone());
    let mut server = McpServerDefinition::new("private", McpTransport::StreamableHttp);
    server.url = Some(http.url.clone());
    server.secret_ref = Some("test-keychain-reference".into());
    server.safety.requires_approval = true;
    repo.create(&server).await.unwrap();
    let secrets = MemorySecrets::default();
    let mut bundle = McpSecretBundle::default();
    bundle
        .headers
        .insert("Authorization".into(), "Bearer fixture-only-token".into());
    secrets.put("test-keychain-reference", &bundle).unwrap();
    GatewayTools::new(MemoryDriver(secrets))
}

#[tokio::test]
async fn gateway_approval_and_keychain_header_stay_on_host_transport() {
    let http = HttpFixture::new().await;
    let broker = TestBroker::new([Scripted::Approve, Scripted::Deny]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let catalog = group.catalog(&ctx).await;
    assert_eq!(catalog.len(), 2);
    assert_eq!(catalog[0]["annotations"]["readOnlyHint"], false);
    assert_eq!(catalog[0]["_meta"]["clovy"]["origin"], "clovy_managed");
    assert!(catalog[0]["_meta"]["clovy"]["id"]
        .as_str()
        .unwrap()
        .starts_with("mcp:"));
    assert!(!serde_json::to_string(&catalog)
        .unwrap()
        .contains("fixture-only-token"));
    let listings = http
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, b)| b["method"] == "tools/list")
        .count();
    assert_eq!(group.catalog(&ctx).await, catalog);
    assert_eq!(
        http.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, b)| b["method"] == "tools/list")
            .count(),
        listings
    );
    let args = json!({"text":"Approved fixture"});
    let result = group.call(&ctx, "mcp_private_send", &args).await.unwrap();
    assert_eq!(result["arguments"], args);
    assert_eq!(http.tool_calls(), 1);
    assert!(http
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, b)| b["method"] == "tools/call")
        .all(|(headers, _)| headers
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-only-token")));
    assert_eq!(
        group
            .call(&ctx, "mcp_private_send", &args)
            .await
            .unwrap_err()
            .code,
        "user_denied"
    );
    assert_eq!(http.tool_calls(), 1);
    let unattended = broker.turn(CliKind::Codex, false).await;
    assert_eq!(
        group
            .call(
                &ToolCtx {
                    binding: Some(&unattended),
                    ..ctx
                },
                "mcp_private_send",
                &args
            )
            .await
            .unwrap_err()
            .code,
        "unattended_requires_approval"
    );
    assert_eq!(http.tool_calls(), 1);
    assert!(!serde_json::to_string(&broker.requests())
        .unwrap()
        .contains("fixture-only-token"));
}

#[tokio::test]
async fn gateway_server_policy_change_during_approval_prevents_dispatch() {
    let http = HttpFixture::new().await;
    let broker = TestBroker::new([Scripted::Leave]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let args = json!({"text":"must not send"});
    let call = group.call(&ctx, "mcp_private_send", &args);
    let change = async {
        while broker.broker().waiting_ids().is_empty() {
            tokio::task::yield_now().await;
        }
        sqlx::query::query("UPDATE agent_mcp_servers SET enabled = 0, updated_at = 'changed' WHERE name = 'private'").execute(broker.pool()).await.unwrap();
        let id = broker.broker().waiting_ids().pop().unwrap();
        broker
            .broker()
            .decide(
                broker.pool(),
                &id,
                crate::approval_broker::Decision::Approve,
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(call, change);
    assert_eq!(result.unwrap_err().code, "agent_mcp_policy_changed");
    assert_eq!(http.tool_calls(), 0);
}

#[tokio::test]
async fn gateway_discovery_failure_omits_only_the_failed_server() {
    let http = HttpFixture::new().await;
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: None,
    };
    let group = group(&broker, &http).await;
    let mut bad = McpServerDefinition::new("broken", McpTransport::StreamableHttp);
    bad.url = Some("http://127.0.0.1:1/mcp".into());
    bad.safety.timeout_ms = 100;
    AgentMcpRepository::new(broker.pool().clone())
        .create(&bad)
        .await
        .unwrap();
    assert_eq!(group.catalog(&ctx).await.len(), 2);
    let error = group
        .call(&ctx, "mcp_broken_lookup", &json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.code, "agent_mcp_transport_failed");
    let diagnostic = error.details.unwrap();
    assert_eq!(diagnostic["origin"], "clovy_managed");
    assert_eq!(diagnostic["server"], "broken");
    assert_eq!(diagnostic["phase"], "discover");
    assert_eq!(diagnostic["tool"], "mcp_broken_lookup");
    assert_eq!(diagnostic["state"], "unavailable");
    assert_eq!(diagnostic["action"], "check_connection");
    assert_eq!(group.catalog(&ctx).await.len(), 2);
}

#[cfg(unix)]
#[tokio::test]
async fn fake_clis_receive_only_relay_configuration_without_mcp_credentials() {
    use crate::agent_sessions::AgentSafetyMode;
    use crate::chat_engine::{cli::turn_invocation, policy::TurnPermissions};
    let http = HttpFixture::new().await;
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: None,
    };
    let group = group(&broker, &http).await;
    assert_eq!(group.catalog(&ctx).await.len(), 2);
    let scratch = tempfile::tempdir().unwrap();
    let fake = scratch.path().join("fake-cli.py");
    std::fs::write(&fake,"import json,os,sys,pathlib\nprint(json.dumps({'argv':sys.argv[1:],'env':dict(os.environ),'files':[p.read_text() for p in pathlib.Path('.').glob('*.json')]}))\n").unwrap();
    let launch = crate::mcp_server::McpLaunch {
        command: "/fixture/clovy-mcp".into(),
        args: vec![
            "--dir".into(),
            "/fixture/mcp".into(),
            "--turn".into(),
            "fixture-turn".into(),
        ],
    };
    for cli in [CliKind::Claude, CliKind::Codex] {
        let invocation = turn_invocation(
            cli,
            "Fixture",
            None,
            Some(&launch),
            scratch.path(),
            TurnPermissions {
                safety_mode: AgentSafetyMode::Sandboxed,
                attended: true,
                routine_tools: None,
            },
            &[],
        );
        for (path, contents) in &invocation.files {
            std::fs::write(path, contents).unwrap();
        }
        let mut command = tokio::process::Command::new("/usr/bin/python3");
        command
            .arg(&fake)
            .args(&invocation.args)
            .current_dir(scratch.path())
            .env_clear();
        for (key, value) in &invocation.set_env {
            command.env(key, value);
        }
        let output = command.output().await.unwrap();
        assert!(output.status.success());
        let captured = String::from_utf8(output.stdout).unwrap();
        assert!(captured.contains("clovy-mcp"));
        assert!(!captured.contains("fixture-only-token"));
        assert!(!captured.contains("Authorization"));
    }
}

#[tokio::test]
async fn gateway_elicitation_uses_clarification_and_resumes_without_resubmitting() {
    let http = HttpFixture::with_elicitation(true).await;
    let broker = TestBroker::new([
        Scripted::Approve,
        Scripted::Answer("{\"team\":\"core\"}".into()),
    ])
    .await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let result = group
        .call(
            &ctx,
            "mcp_private_send",
            &json!({"text":"eliciting fixture"}),
        )
        .await
        .unwrap();
    assert_eq!(result["content"][0]["text"], "selected core");
    assert_eq!(http.tool_calls(), 1);
    assert_eq!(broker.requests().len(), 2);
    assert_eq!(broker.requests()[1]["kind"], "clarification");
    assert!(broker.requests()[1]["question"]
        .as_str()
        .unwrap()
        .contains("Which team?"));
    assert!(http
        .calls
        .lock()
        .unwrap()
        .iter()
        .any(|(_, b)| b["result"]["content"]["team"] == "core"));
}

#[tokio::test]
async fn denied_clarification_preserves_post_dispatch_uncertainty() {
    let http = HttpFixture::with_elicitation(true).await;
    let broker = TestBroker::new([Scripted::Approve, Scripted::Deny]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let error = group
        .call(&ctx, "mcp_private_send", &json!({"text":"synthetic"}))
        .await
        .unwrap_err();
    let details = error.details.unwrap();
    assert_eq!(details["action"], "inspect_server_before_retry");
    assert_eq!(details["delivery"], "unknown");
    assert_eq!(details["retrySafe"], false);
    assert_eq!(http.tool_calls(), 1);
}

#[tokio::test]
async fn gateway_failed_credentials_and_namespace_collision_do_not_hide_healthy_tools() {
    let http = HttpFixture::new().await;
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: None,
    };
    let group = group(&broker, &http).await;
    let repo = AgentMcpRepository::new(broker.pool().clone());
    let mut bad = McpServerDefinition::new("bad credentials", McpTransport::StreamableHttp);
    bad.url = Some(http.url.clone());
    bad.secret_ref = Some("fixture-failing-keychain".into());
    repo.create(&bad).await.unwrap();
    let mut collision = McpServerDefinition::new("private.", McpTransport::StreamableHttp);
    collision.url = Some(http.url.clone());
    repo.create(&collision).await.unwrap();
    let tools = group.catalog(&ctx).await;
    assert_eq!(tools.len(), 2);
    assert!(tools
        .iter()
        .all(|tool| tool["name"].as_str().unwrap().starts_with("mcp_private_")));
    let error = group
        .call(&ctx, "mcp_bad_credentials_lookup", &json!({}))
        .await
        .unwrap_err();
    assert_eq!(error.code, "agent_mcp_secure_storage_failed");
    let diagnostic = error.details.unwrap();
    assert_eq!(diagnostic["server"], "bad credentials");
    assert_eq!(diagnostic["phase"], "refresh");
    assert_eq!(diagnostic["action"], "check_local_storage");
    assert_eq!(group.catalog(&ctx).await.len(), 2);
}

#[tokio::test]
async fn gateway_lost_parked_session_fails_without_replaying_the_tool() {
    let http = HttpFixture::with_elicitation(true).await;
    let broker = TestBroker::new([Scripted::Approve, Scripted::Leave]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let args = json!({"text":"never replay"});
    let call = group.call(&ctx, "mcp_private_send", &args);
    let retire = async {
        loop {
            if broker
                .requests()
                .iter()
                .any(|card| card["kind"] == "clarification")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
        let server = AgentMcpRepository::new(broker.pool().clone())
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|s| s.name == "private")
            .unwrap();
        crate::agent_mcp::retire_server_sessions(&server.id).await;
        let id = broker.broker().waiting_ids().pop().unwrap();
        broker
            .broker()
            .decide(
                broker.pool(),
                &id,
                crate::approval_broker::Decision::Answer("{\"team\":\"core\"}".into()),
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(call, retire);
    let error = result.unwrap_err();
    assert_eq!(error.code, "agent_mcp_delivery_uncertain");
    assert_eq!(
        error.details.unwrap()["action"],
        "inspect_server_before_retry"
    );
    assert_eq!(http.tool_calls(), 1);
}

#[tokio::test]
async fn gateway_and_legacy_calls_cannot_answer_each_others_elicitation() {
    use crate::agent_mcp::AgentMcpSubsystem;
    let http = HttpFixture::with_elicitation(true).await;
    let broker = TestBroker::new([
        Scripted::Approve,
        Scripted::Answer("{\"team\":\"core\"}".into()),
    ])
    .await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let secrets = MemorySecrets::default();
    let mut bundle = McpSecretBundle::default();
    bundle
        .headers
        .insert("Authorization".into(), "Bearer fixture-only-token".into());
    secrets.put("test-keychain-reference", &bundle).unwrap();
    let native = AgentMcpSubsystem::new(AgentMcpRepository::new(broker.pool().clone()), secrets);
    native
        .refresh_registry_for_workspace(true, Some(std::path::Path::new("/tmp")))
        .await
        .unwrap();
    assert!(matches!(
        native
            .invoke_in_workspace(
                "mcp_private_send",
                json!({"text":"legacy original"}),
                true,
                Some(std::path::Path::new("/tmp"))
            )
            .await,
        Err(AgentMcpError::ElicitationRequired(_))
    ));
    let result = group
        .call(&ctx, "mcp_private_send", &json!({"text":"CLI original"}))
        .await
        .unwrap();
    assert_eq!(result["content"][0]["text"], "selected core");
    assert_eq!(http.tool_calls(), 2);
    // The native request is still parked, and only its own path answers it.
    native
        .invoke_in_workspace_with_elicitation(
            "mcp_private_send",
            json!({"text":"legacy original"}),
            true,
            Some(std::path::Path::new("/tmp")),
            Some("{\"team\":\"core\"}"),
        )
        .await
        .unwrap();
    assert_eq!(http.tool_calls(), 2);
}

#[tokio::test]
async fn gateway_two_invocations_in_one_turn_keep_their_own_calls() {
    let http = HttpFixture::with_elicitation(true).await;
    let broker = TestBroker::new([
        Scripted::Approve,
        Scripted::Answer("{\"team\":\"core\"}".into()),
        Scripted::Approve,
        Scripted::Answer("{\"team\":\"ops\"}".into()),
    ])
    .await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let first = json!({"text":"first original"});
    let second = json!({"text":"second original"});
    let (a, b) = tokio::join!(
        group.call(&ctx, "mcp_private_send", &first),
        group.call(&ctx, "mcp_private_send", &second)
    );
    assert!(a.is_ok());
    assert!(b.is_ok());
    let calls = http.calls.lock().unwrap();
    let submitted = calls
        .iter()
        .filter(|(_, b)| b["method"] == "tools/call")
        .map(|(_, b)| b["params"]["arguments"].clone())
        .collect::<Vec<_>>();
    assert_eq!(submitted.len(), 2);
    assert!(submitted.contains(&first));
    assert!(submitted.contains(&second));
}

#[tokio::test]
async fn gateway_dropped_call_closes_only_its_transport_and_keeps_legacy_pending() {
    use crate::agent_mcp::AgentMcpSubsystem;
    use std::sync::atomic::Ordering;
    let http = HttpFixture::with_elicitation(true).await;
    let broker = TestBroker::new([Scripted::Approve, Scripted::Leave]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = group(&broker, &http).await;
    let secrets = MemorySecrets::default();
    let mut bundle = McpSecretBundle::default();
    bundle
        .headers
        .insert("Authorization".into(), "Bearer fixture-only-token".into());
    secrets.put("test-keychain-reference", &bundle).unwrap();
    let native = AgentMcpSubsystem::new(AgentMcpRepository::new(broker.pool().clone()), secrets);
    native
        .refresh_registry_for_workspace(true, Some(std::path::Path::new("/tmp")))
        .await
        .unwrap();
    let legacy_args = json!({"text":"legacy stays parked"});
    assert!(matches!(
        native
            .invoke_in_workspace(
                "mcp_private_send",
                legacy_args.clone(),
                true,
                Some(std::path::Path::new("/tmp"))
            )
            .await,
        Err(AgentMcpError::ElicitationRequired(_))
    ));
    let args = json!({"text":"abandoned CLI call"});
    let mut call = group.call(&ctx, "mcp_private_send", &args);
    let pending = async {
        loop {
            if broker
                .requests()
                .iter()
                .any(|c| c["kind"] == "clarification")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    };
    tokio::select! {_=&mut call=>panic!("call should wait for clarification"),_=pending=>{}}
    assert_eq!(http.active.load(Ordering::SeqCst), 2);
    drop(call);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while http.active.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    native
        .invoke_in_workspace_with_elicitation(
            "mcp_private_send",
            legacy_args,
            true,
            Some(std::path::Path::new("/tmp")),
            Some("{\"team\":\"core\"}"),
        )
        .await
        .unwrap();
    assert_eq!(http.tool_calls(), 2);
}
