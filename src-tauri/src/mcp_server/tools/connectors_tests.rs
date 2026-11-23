use super::{ActivityAccess, McpData, ToolCtx};
use crate::approval_broker::testing::TestBroker;
use serde_json::json;

#[tokio::test]
async fn unbound_connector_calls_require_a_turn_and_are_not_listed() {
    let broker = TestBroker::new([]).await;
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx::unbound(&data);
    for name in ["send_email", "list_unread", "notion-update-page"] {
        assert_eq!(
            super::call(&ctx, name, &json!({})).await.unwrap_err().code,
            "turn_required"
        );
        assert!(!super::descriptors(&ctx)
            .await
            .iter()
            .any(|tool| tool["name"] == name));
    }
    assert!(broker.requests().is_empty());
}

use super::connectors::{ConnectorDriver, ConnectorTools, PreparedCall};
use super::ToolGroup;
use crate::approval_broker::testing::Scripted;
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;
use futures_util::future::BoxFuture;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A loopback transport only; requests are kept so a denied write cannot
/// accidentally pass by returning a synthetic successful result.
pub(crate) struct HttpFixture {
    pub url: String,
    pub calls: Arc<Mutex<Vec<(String, Value)>>>,
    pub active: Arc<AtomicUsize>,
    pub effects: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl HttpFixture {
    pub async fn new() -> Self {
        Self::with_elicitation(false).await
    }
    pub async fn with_elicitation(elicit: bool) -> Self {
        Self::configured(elicit, None).await
    }
    /// A real loopback MCP server with mutable state, for protocol-boundary
    /// tests that must observe an approved effect through a later read.
    pub async fn with_stateful_methods(methods: Vec<String>) -> Self {
        Self::configured(false, Some(methods)).await
    }
    async fn configured(elicit: bool, methods: Option<Vec<String>>) -> Self {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/mcp", listener.local_addr().unwrap());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let recorded = calls.clone();
        let active = Arc::new(AtomicUsize::new(0));
        let active_requests = active.clone();
        let effects = Arc::new(Mutex::new(Vec::<Value>::new()));
        let written = effects.clone();
        let answers = Arc::new(Mutex::new(std::collections::HashMap::<
            String,
            Arc<tokio::sync::Notify>,
        >::new()));
        let question_ids = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let calls = recorded.clone();
                let active = active_requests.clone();
                let answers = answers.clone();
                let question_ids = question_ids.clone();
                let methods = methods.clone();
                let effects = written.clone();
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let (start, size) = loop {
                        let mut buf = [0; 4096];
                        let n = stream.read(&mut buf).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buf[..n]);
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers = String::from_utf8_lossy(&bytes[..end]);
                            let size = headers
                                .lines()
                                .find_map(|l| {
                                    let (k, v) = l.split_once(':')?;
                                    k.eq_ignore_ascii_case("content-length")
                                        .then(|| v.trim().parse::<usize>().unwrap())
                                })
                                .unwrap_or(0);
                            if bytes.len() >= end + 4 + size {
                                break (end + 4, size);
                            }
                        }
                    };
                    let headers = String::from_utf8_lossy(&bytes[..start]).to_string();
                    if size == 0 {
                        let _ = stream.write_all(b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                        return;
                    }
                    let body: Value = serde_json::from_slice(&bytes[start..start + size]).unwrap();
                    calls.lock().unwrap().push((headers, body.clone()));
                    if elicit && body.get("method").is_none() && body.get("result").is_some() {
                        if let Some(answered) = body["id"]
                            .as_str()
                            .and_then(|id| answers.lock().unwrap().get(id).cloned())
                        {
                            answered.notify_one();
                        }
                        stream.write_all(b"HTTP/1.1 202 Accepted\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
                        return;
                    }
                    if elicit && body["method"] == "tools/call" {
                        active.fetch_add(1, Ordering::SeqCst);
                        let question_id =
                            format!("ask-team-{}", question_ids.fetch_add(1, Ordering::SeqCst));
                        let answered = Arc::new(tokio::sync::Notify::new());
                        answers
                            .lock()
                            .unwrap()
                            .insert(question_id.clone(), answered.clone());
                        let question = format!(
                            "data: {}\n\n",
                            json!({"jsonrpc":"2.0","id":question_id,"method":"elicitation/create","params":{"message":"Which team?","requestedSchema":{"type":"object","properties":{"team":{"type":"string"}}}}})
                        );
                        let result = format!(
                            "data: {}\n\n",
                            json!({"jsonrpc":"2.0","id":body["id"],"result":{"content":[{"type":"text","text":"selected core"}]}})
                        );
                        let head = format!("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",question.len()+result.len());
                        stream.write_all(head.as_bytes()).await.unwrap();
                        stream.write_all(question.as_bytes()).await.unwrap();
                        let mut eof = [0_u8; 1];
                        tokio::select! {
                            _=answered.notified()=> {let _=stream.write_all(result.as_bytes()).await;},
                            _=stream.read(&mut eof)=>{},
                            _=tokio::time::sleep(std::time::Duration::from_secs(10))=>{},
                        }
                        active.fetch_sub(1, Ordering::SeqCst);
                        return;
                    }
                    let result = match body["method"].as_str() {
                        Some("initialize") => {
                            json!({"protocolVersion":"2025-06-18", "capabilities":{}, "serverInfo":{"name":"fixture", "version":"1"}})
                        }
                        Some("tools/list") if methods.is_some() => {
                            json!({"tools":methods.as_ref().unwrap().iter().map(|name| json!({
                                "name":name,
                                "description":format!("Stateful fixture operation {name}"),
                                "inputSchema":{"type":"object","properties":{"stamp":{"type":"string"}}},
                                "annotations":{"readOnlyHint":name == "read_state"}
                            })).collect::<Vec<_>>()})
                        }
                        Some("tools/call") if methods.is_some() => {
                            let name = body["params"]["name"].as_str().unwrap();
                            assert!(methods
                                .as_ref()
                                .unwrap()
                                .iter()
                                .any(|method| method == name));
                            let mut effects = effects.lock().unwrap();
                            if name != "read_state" {
                                effects.push(body["params"]["arguments"].clone());
                            }
                            json!({"content":[{"type":"text","text":serde_json::to_string(&*effects).unwrap()}]})
                        }
                        Some("tools/list") => {
                            json!({"tools":[{"name":"send", "description":"Send fixture", "inputSchema":{"type":"object","properties":{"text":{"type":"string"}}}},{"name":"read", "inputSchema":{"type":"object"}}]})
                        }
                        _ => {
                            json!({"content":[{"type":"text","text":"fixture sent"}],"arguments":body["params"]["arguments"]})
                        }
                    };
                    let response =
                        json!({"jsonrpc":"2.0", "id":body["id"], "result":result}).to_string();
                    let wire = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}", response.len(), response);
                    stream.write_all(wire.as_bytes()).await.unwrap();
                });
            }
        });
        Self {
            url,
            calls,
            active,
            effects,
            task,
        }
    }
    pub fn tool_calls(&self) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, b)| b["method"] == "tools/call")
            .count()
    }
}

struct HttpConnector {
    url: String,
    allowed: Arc<AtomicBool>,
    discoveries: Arc<AtomicUsize>,
}
impl ConnectorDriver for HttpConnector {
    fn catalog<'a>(&'a self, _ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Result<Vec<Value>, AppError>> {
        Box::pin(async move {
            self.discoveries.fetch_add(1, Ordering::SeqCst);
            Ok(vec![
                json!({"name":"send_email", "description":"Send email", "parameters":{"type":"object"}, "requiresApproval":true}),
                json!({"name":"list_unread", "parameters":{"type":"object"}, "requiresApproval":false}),
            ])
        })
    }
    fn invoke<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        call: PreparedCall,
        approval_id: Option<String>,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            if !self.allowed.load(Ordering::SeqCst) {
                return Err(AppError::new("connector_scope_missing", "Scope revoked"));
            }
            if let Some(action) = call.notion {
                action
                    .claim(
                        &ctx.data.notes,
                        ctx.binding.unwrap(),
                        approval_id.as_deref().unwrap(),
                    )
                    .await?;
            }
            reqwest::Client::new().post(&self.url).json(&json!({"method":"tools/call","params":{"name":call.name,"arguments":call.arguments}})).send().await.unwrap().json().await.map_err(|e| AppError::new("fixture", e.to_string()))
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
fn driver(http: &HttpFixture) -> HttpConnector {
    HttpConnector {
        url: http.url.clone(),
        allowed: Arc::new(AtomicBool::new(true)),
        discoveries: Arc::new(AtomicUsize::new(0)),
    }
}

#[tokio::test]
async fn connector_approval_denial_and_unattended_never_send_http() {
    let http = HttpFixture::new().await;
    let broker = TestBroker::new([Scripted::Approve, Scripted::Deny]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = ConnectorTools::new(driver(&http));
    let args = json!({"accountId":"fake-google", "to":["test@example.invalid"], "subject":"Fixture", "bodyText":"Text"});
    assert!(group.call(&ctx, "send_email", &args).await.is_ok());
    assert_eq!(http.tool_calls(), 1);
    assert_eq!(
        group
            .call(&ctx, "send_email", &args)
            .await
            .unwrap_err()
            .code,
        "user_denied"
    );
    let unattended = broker.turn(CliKind::Codex, false).await;
    assert_eq!(
        group
            .call(
                &ToolCtx {
                    binding: Some(&unattended),
                    ..ctx
                },
                "send_email",
                &args
            )
            .await
            .unwrap_err()
            .code,
        "unattended_requires_approval"
    );
    assert_eq!(http.tool_calls(), 1);
    assert_eq!(broker.requests().len(), 2);
    assert_eq!(
        group
            .call(
                &ToolCtx {
                    broker: None,
                    ..ctx
                },
                "send_email",
                &args
            )
            .await
            .unwrap_err()
            .code,
        "approval_unavailable"
    );
    assert_eq!(http.tool_calls(), 1);
}

#[tokio::test]
async fn connector_reads_need_no_card_and_catalog_is_cached_per_turn() {
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
    let group = ConnectorTools::new(driver(&http));
    let first = group.catalog(&ctx).await;
    assert_eq!(first[0]["annotations"]["readOnlyHint"], false);
    assert_eq!(first[1]["annotations"]["readOnlyHint"], true);
    assert_eq!(first[0]["inputSchema"], json!({"type":"object"}));
    assert_eq!(group.catalog(&ctx).await, first);
    assert_eq!(group.driver.discoveries.load(Ordering::SeqCst), 1);
    let other = broker.turn(CliKind::Codex, true).await;
    group
        .catalog(&ToolCtx {
            binding: Some(&other),
            ..ctx
        })
        .await;
    assert_eq!(group.driver.discoveries.load(Ordering::SeqCst), 2);
    group
        .call(&ctx, "list_unread", &json!({"accountId":"fake-google"}))
        .await
        .unwrap();
    assert!(broker.requests().is_empty());
}

#[tokio::test]
async fn connector_scope_is_revalidated_after_a_pending_approval() {
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
    let group = ConnectorTools::new(driver(&http));
    let args = json!({"accountId":"fake-google"});
    let effect = group.call(&ctx, "send_email", &args);
    let revoke = async {
        while broker.broker().waiting_ids().is_empty() {
            tokio::task::yield_now().await;
        }
        group.driver.allowed.store(false, Ordering::SeqCst);
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
    let (result, ()) = tokio::join!(effect, revoke);
    assert_eq!(result.unwrap_err().code, "connector_scope_missing");
    assert_eq!(http.tool_calls(), 0);
}

#[tokio::test]
async fn notion_broker_receipt_is_bound_to_tool_turn_and_consumed_once() {
    use crate::connectors::notion::BrokerAction;
    let broker = TestBroker::new([Scripted::Approve]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let other = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let args = json!({"page_id":"fake-page", "command":"update_properties", "properties":{"title":"Fixture"}});
    let action = BrokerAction::prepare("notion-update-page", &args).unwrap();
    let (title, description, preview) = action.presentation();
    let id = super::connectors::approve(
        &ctx,
        "notion-update-page",
        &args,
        title,
        description,
        preview,
    )
    .await
    .unwrap();
    assert_eq!(
        action
            .claim(broker.pool(), &other, &id)
            .await
            .unwrap_err()
            .code,
        "notion_action_binding_missing"
    );
    let create_args = json!({"parent":{"page_id":"fake-parent"}, "pages":[{"properties":{"title":"Fixture"}, "content":"Text"}]});
    let create = BrokerAction::prepare("notion-create-pages", &create_args).unwrap();
    assert_eq!(
        create
            .claim(broker.pool(), &binding, &id)
            .await
            .unwrap_err()
            .code,
        "notion_action_binding_missing"
    );
    action.claim(broker.pool(), &binding, &id).await.unwrap();
    assert_eq!(
        action
            .claim(broker.pool(), &binding, &id)
            .await
            .unwrap_err()
            .code,
        "notion_action_binding_missing"
    );
}

#[tokio::test]
async fn notion_bypass_receipt_is_still_bound_and_consumed_once() {
    use crate::connectors::notion::BrokerAction;
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let other = broker.turn(CliKind::Claude, true).await;
    sqlx::query::query("UPDATE agent_runs SET run_config_json = json_set(run_config_json, '$.approvalMode', 'bypass') WHERE id = ?")
        .bind(&binding.run_id).execute(broker.pool()).await.unwrap();
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let args = json!({"page_id":"fake-page", "command":"update_properties", "properties":{"title":"Fixture"}});
    let action = BrokerAction::prepare("notion-update-page", &args).unwrap();
    let (title, description, preview) = action.presentation();
    let id = super::connectors::approve(
        &ctx,
        "notion-update-page",
        &args,
        title,
        description,
        preview,
    )
    .await
    .unwrap();
    assert_eq!(
        action
            .claim(broker.pool(), &other, &id)
            .await
            .unwrap_err()
            .code,
        "notion_action_binding_missing"
    );
    action.claim(broker.pool(), &binding, &id).await.unwrap();
    assert_eq!(
        action
            .claim(broker.pool(), &binding, &id)
            .await
            .unwrap_err()
            .code,
        "notion_action_binding_missing"
    );
}

#[tokio::test]
async fn every_connector_mutation_is_denied_before_external_dispatch() {
    let names = [
        "create_draft",
        "send_email",
        "modify_labels",
        "archive",
        "create_event",
        "respond_to_invite",
        "create_issue",
        "update_issue",
        "add_comment",
        "notion-create-pages",
        "notion-update-page",
    ];
    let http = HttpFixture::new().await;
    let broker = TestBroker::new(names.iter().map(|_| Scripted::Deny)).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = ConnectorTools::new(driver(&http));
    for name in names {
        let args = match name {
            "notion-create-pages" => {
                json!({"parent":{"page_id":"fake-parent"},"pages":[{"properties":{"title":"Fixture"},"content":"Text"}]})
            }
            "notion-update-page" => {
                json!({"page_id":"fake-page","command":"update_properties","properties":{"title":"Fixture"}})
            }
            _ => json!({"accountId":"fake-account"}),
        };
        assert_eq!(
            group.call(&ctx, name, &args).await.unwrap_err().code,
            "user_denied",
            "{name}"
        );
    }
    assert_eq!(http.tool_calls(), 0);
    assert_eq!(broker.requests().len(), 11);
}

#[tokio::test]
async fn connector_catalog_expires_and_discovers_again() {
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
    let group = ConnectorTools::new(driver(&http));
    group.catalog(&ctx).await;
    tokio::time::sleep(std::time::Duration::from_secs(16)).await;
    group.catalog(&ctx).await;
    assert_eq!(group.driver.discoveries.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn connector_timeout_and_cancellation_never_dispatch() {
    let http = HttpFixture::new().await;
    let broker =
        TestBroker::with_timeout([Scripted::Leave], std::time::Duration::from_millis(20)).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = ConnectorTools::new(driver(&http));
    assert_eq!(
        group
            .call(&ctx, "send_email", &json!({}))
            .await
            .unwrap_err()
            .code,
        "approval_timed_out"
    );
    let args = json!({});
    let call = group.call(&ctx, "send_email", &args);
    let cancel = async {
        while broker.broker().waiting_ids().is_empty() {
            tokio::task::yield_now().await;
        }
        broker.broker().cancel_run(&binding.run_id).await;
    };
    let (result, ()) = tokio::join!(call, cancel);
    assert_eq!(result.unwrap_err().code, "run_cancelled");
    assert_eq!(http.tool_calls(), 0);
}

/// U4: a "via gh" account's write goes through the real broker and the real
/// GitHub dispatch, with the token from the fake `gh`.
struct GhAccess {
    repos: crate::db::repositories::Repositories,
    env: crate::llm::shell_env::LoginEnv,
}
impl crate::connectors::agent_tools::GithubAccess for GhAccess {
    fn accounts(
        &self,
    ) -> BoxFuture<'_, Result<Vec<crate::connectors::ConnectorAccount>, AppError>> {
        Box::pin(async move {
            Ok(self
                .repos
                .list_connector_accounts()
                .await?
                .into_iter()
                .map(crate::connectors::github_cli::account_from_record)
                .collect())
        })
    }
    fn token<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, Result<String, AppError>> {
        Box::pin(crate::connectors::github_cli::access_token(
            &self.repos,
            &self.env,
            account_id,
        ))
    }
}
struct GhDriver(GhAccess);
impl ConnectorDriver for GhDriver {
    fn catalog<'a>(&'a self, _ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Result<Vec<Value>, AppError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn invoke<'a>(
        &'a self,
        _ctx: &'a ToolCtx<'a>,
        call: PreparedCall,
        _approval_id: Option<String>,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            crate::connectors::agent_tools::dispatch_github(&self.0, &call.name, call.arguments)
                .await
                .unwrap_or_else(|| {
                    Err(AppError::new("mcp_tool_unknown", "Unknown connector tool."))
                })
        })
    }
}

#[cfg(unix)]
#[tokio::test]
async fn gh_account_write_needs_the_bundle_and_an_approval_before_it_reaches_github() {
    use crate::connectors::github_cli::{self, ConnectMode};
    use crate::connectors::github_cli_fixtures::*;
    let gh = FakeGh::new();
    gh.log_in();
    let api = FakeGithub::start().await;
    let repos = repositories().await;
    let prefs = tempfile::tempdir().unwrap();
    github_cli::detect_and_connect(&repos, &gh.env(), prefs.path(), ConnectMode::Automatic)
        .await
        .unwrap()
        .unwrap();

    let broker = TestBroker::new([Scripted::Approve, Scripted::Approve]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(&broker);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let group = ConnectorTools::new(GhDriver(GhAccess {
        repos: repos.clone(),
        env: gh.env(),
    }));
    let args = json!({
        "accountId": USER_ID,
        "provider": "github",
        "owner": "octo",
        "repo": "hello",
        "title": "Fixture issue",
    });
    let posts = |api: &FakeGithub| {
        api.requests()
            .iter()
            .filter(|request| request.starts_with("POST "))
            .count()
    };

    let refused = group.call(&ctx, "create_issue", &args).await.unwrap_err();
    assert_eq!(refused.code, "connector_scope_missing");
    assert_eq!(broker.requests().len(), 1, "the approval is asked first");
    assert_eq!(posts(&api), 0);

    let write = crate::connectors::policy::ScopeBundle::from_name("github_write").unwrap();
    github_cli::grant_bundles(&repos, USER_ID, &[write])
        .await
        .unwrap();
    let created = group.call(&ctx, "create_issue", &args).await.unwrap();

    assert_eq!(created["number"], 7);
    assert_eq!(broker.requests().len(), 2);
    assert_eq!(
        api.requests()
            .iter()
            .filter(|request| *request == "POST /repos/octo/hello/issues")
            .count(),
        1
    );
}
