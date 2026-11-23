//! Current-grant consumers stay usable after reconnect, while an older grant's
//! unresolved revocation survives successful checks and a file-backed reopen.
//! HTTP, callback, repository and vault are synthetic; consumers are production.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;
use tokio::net::{TcpListener, TcpStream};

const OLD_REF: &str = "history-fixture-old-ref";
const OLD_REVISION: &str = "history-fixture-old-revision";

#[derive(Clone, Default)]
struct HistoryStore {
    values: Arc<Mutex<BTreeMap<String, McpSecretBundle>>>,
    fail_old_put: Arc<std::sync::atomic::AtomicBool>,
}

impl McpSecretStore for HistoryStore {
    fn put(&self, reference: &str, bundle: &McpSecretBundle) -> Result<(), AgentMcpError> {
        if reference == OLD_REF && self.fail_old_put.load(Ordering::SeqCst) {
            return Err(AgentMcpError::Storage);
        }
        self.values
            .lock()
            .unwrap()
            .insert(reference.into(), bundle.clone());
        Ok(())
    }
    fn get(&self, reference: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        Ok(self.values.lock().unwrap().get(reference).cloned())
    }
    fn delete(&self, reference: &str) -> Result<(), AgentMcpError> {
        self.values.lock().unwrap().remove(reference);
        Ok(())
    }
}

#[derive(Default)]
struct HistoryObserved {
    refreshes: usize,
    exchanges: usize,
    revoked: Vec<String>,
    mcp: Vec<(String, String)>,
    callback_statuses: Vec<u16>,
}

struct HistoryHttp {
    origin: String,
    observed: Arc<Mutex<HistoryObserved>>,
    reject_refresh: Arc<std::sync::atomic::AtomicBool>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HistoryHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn history_read(stream: &mut TcpStream) -> (String, String, String) {
    let mut bytes = Vec::new();
    loop {
        let mut chunk = [0u8; 4096];
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 64 * 1024);
        let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            return (
                headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .into(),
                headers.into_owned(),
                String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap(),
            );
        }
    }
}

async fn history_reply(stream: &mut TcpStream, status: &str, body: Value) {
    let body = body.to_string();
    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
}

impl HistoryHttp {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let observed = Arc::new(Mutex::new(HistoryObserved::default()));
        let reject_refresh = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let seen = observed.clone();
        let rejected = reject_refresh.clone();
        let host = origin.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (path, headers, body) = history_read(&mut stream).await;
                let result = match path.as_str() {
                    "/.well-known/oauth-protected-resource/mcp" => json!({
                        "resource": format!("{host}/mcp"),
                        "authorization_servers": [host], "scopes_supported": ["openid"]
                    }),
                    "/.well-known/oauth-authorization-server" => json!({
                        "issuer": host, "authorization_endpoint": format!("{host}/authorize"),
                        "token_endpoint": format!("{host}/token"),
                        "registration_endpoint": format!("{host}/register"),
                        "revocation_endpoint": format!("{host}/revoke"),
                        "response_types_supported": ["code"],
                        "grant_types_supported": ["authorization_code", "refresh_token"],
                        "code_challenge_methods_supported": ["S256"],
                        "token_endpoint_auth_methods_supported": ["none"]
                    }),
                    "/register" => json!({"client_id": "clovy-history-fixture-client"}),
                    "/token" => {
                        let form: BTreeMap<String, String> =
                            url::form_urlencoded::parse(body.as_bytes())
                                .into_owned()
                                .collect();
                        assert_eq!(form["resource"], format!("{host}/mcp"));
                        if form["grant_type"] == "refresh_token" {
                            assert_eq!(form["refresh_token"], "synthetic-history-r0-refresh");
                            seen.lock().unwrap().refreshes += 1;
                            if rejected.load(Ordering::SeqCst) {
                                history_reply(
                                    &mut stream,
                                    "400 Bad Request",
                                    json!({"error":"invalid_grant"}),
                                )
                                .await;
                                continue;
                            }
                            json!({"access_token":"synthetic-history-r1-access", "refresh_token":"synthetic-history-r1-refresh", "expires_in":3600, "token_type":"Bearer"})
                        } else {
                            assert_eq!(form["grant_type"], "authorization_code");
                            assert_eq!(form["code"], "synthetic-history-code");
                            assert!(!form["code_verifier"].is_empty());
                            seen.lock().unwrap().exchanges += 1;
                            json!({"access_token":"synthetic-history-r2-access", "refresh_token":"synthetic-history-r2-refresh", "expires_in":3600, "token_type":"Bearer"})
                        }
                    }
                    "/revoke" => {
                        let form: BTreeMap<String, String> =
                            url::form_urlencoded::parse(body.as_bytes())
                                .into_owned()
                                .collect();
                        seen.lock().unwrap().revoked.push(form["token"].clone());
                        if form["token"] == "synthetic-history-r1-refresh" {
                            history_reply(&mut stream, "503 Service Unavailable", json!({})).await;
                            continue;
                        }
                        json!({})
                    }
                    "/mcp" => {
                        let bearer = headers
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("authorization")
                                    .then(|| value.trim().to_owned())
                            })
                            .unwrap();
                        assert_eq!(bearer, "Bearer synthetic-history-r2-access");
                        let request: Value = serde_json::from_str(&body).unwrap();
                        let method = request["method"].as_str().unwrap();
                        seen.lock().unwrap().mcp.push((method.into(), bearer));
                        let result = match method {
                            "initialize" => {
                                json!({"protocolVersion": MCP_PROTOCOL_VERSION, "capabilities":{"tools":{}}, "serverInfo":{"name":"grant-history-fixture","version":"1"}})
                            }
                            "notifications/initialized" => {
                                history_reply(&mut stream, "202 Accepted", Value::Null).await;
                                continue;
                            }
                            "tools/list" => {
                                json!({"tools":[{"name":"list_projects", "description":"Synthetic projects", "inputSchema":{"type":"object","properties":{}}, "annotations":{"readOnlyHint":true}}]})
                            }
                            "tools/call" => {
                                assert_eq!(request["params"]["name"], "list_projects");
                                json!({"content":[{"type":"text","text":"synthetic-history-project"}],"isError":false})
                            }
                            _ => panic!("unexpected MCP method {method}"),
                        };
                        json!({"jsonrpc":"2.0","id":request["id"],"result":result})
                    }
                    _ => panic!("unexpected route {path}"),
                };
                history_reply(&mut stream, "200 OK", result).await;
            }
        });
        Self {
            origin,
            observed,
            reject_refresh,
            task,
        }
    }
}

async fn history_repository(database_url: &str) -> AgentMcpRepository {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    repository
}

async fn history_seed(
    repository: &AgentMcpRepository,
    store: &HistoryStore,
    fixture: &HistoryHttp,
) -> McpServerDefinition {
    let mut server =
        McpServerDefinition::new("grant-history-fixture", McpTransport::StreamableHttp);
    server.url = Some(format!("{}/mcp", fixture.origin));
    server.secret_ref = Some(OLD_REF.into());
    server.metadata = json!({"auth":"oauth","oauthConnected":true,"oauthRevision":OLD_REVISION});
    repository.create(&server).await.unwrap();
    let bundle = McpSecretBundle {
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        oauth: BTreeMap::from([
            ("access_token".into(), "synthetic-history-r0-access".into()),
            (
                "refresh_token".into(),
                "synthetic-history-r0-refresh".into(),
            ),
            ("client_id".into(), "clovy-history-fixture-client".into()),
            ("client_owner".into(), "clovy".into()),
            ("resource".into(), format!("{}/mcp", fixture.origin)),
            ("token_endpoint".into(), format!("{}/token", fixture.origin)),
            (
                "authorization_endpoint".into(),
                format!("{}/authorize", fixture.origin),
            ),
            (
                "revocation_endpoint".into(),
                format!("{}/revoke", fixture.origin),
            ),
            ("expires_at_unix".into(), "0".into()),
        ]),
    };
    store.put(OLD_REF, &bundle).unwrap();
    server
}

async fn history_connect(
    repository: &AgentMcpRepository,
    store: &HistoryStore,
    id: &str,
    fixture: &HistoryHttp,
    denied: bool,
) -> Result<McpServerDefinition, crate::domain::types::AppError> {
    let seen = fixture.observed.clone();
    connect_oauth_with(
        repository,
        store,
        id,
        None,
        Duration::from_secs(10),
        move |server, bundle| async move {
            authorize_oauth_server_with(&server, &bundle, move |url| {
                let parameters: BTreeMap<String, String> = reqwest::Url::parse(url)
                    .unwrap()
                    .query_pairs()
                    .into_owned()
                    .collect();
                assert_eq!(parameters["code_challenge_method"], "S256");
                assert!(parameters["state"].len() >= 32);
                let mut callback = reqwest::Url::parse(&parameters["redirect_uri"]).unwrap();
                callback
                    .query_pairs_mut()
                    .append_pair("state", &parameters["state"])
                    .append_pair(
                        if denied { "error" } else { "code" },
                        if denied {
                            "access_denied"
                        } else {
                            "synthetic-history-code"
                        },
                    );
                tokio::spawn(async move {
                    let status = reqwest::Client::new()
                        .get(callback)
                        .send()
                        .await
                        .unwrap()
                        .status()
                        .as_u16();
                    seen.lock().unwrap().callback_statuses.push(status);
                });
                Ok(())
            })
            .await
        },
    )
    .await
}

fn history_entries(server: &McpServerDefinition) -> Vec<Value> {
    server
        .metadata
        .get("oauthRevocationHistory")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn assert_prior_history(server: &McpServerDefinition) {
    let entries = history_entries(server);
    assert!(entries
        .iter()
        .any(|entry| entry["affectedSecretRef"] == OLD_REF
            && entry["affectedRevision"] == OLD_REVISION
            && entry["reason"] == "rotation_unconfirmed"));
    assert!(entries
        .iter()
        .all(|entry| entry.as_object().unwrap().len() == 4
            && entry["id"].as_str().is_some_and(|id| !id.is_empty())));
    assert_ne!(
        server.metadata.get("oauthRotatedRevocationUnconfirmed"),
        Some(&json!(true))
    );
    assert_ne!(
        server.metadata.get("oauthRemoteRevocationFailed"),
        Some(&json!(true))
    );
}

async fn history_consumer_chain(
    repository: &AgentMcpRepository,
    store: &HistoryStore,
    id: &str,
) -> RegisteredMcpTool {
    let server = repository.get(id).await.unwrap();
    let (bundle, refreshed) = load_oauth_secrets(repository, store, &server, false)
        .await
        .unwrap();
    assert!(!refreshed);
    assert_eq!(bundle.oauth["access_token"], "synthetic-history-r2-access");
    let discovered = discover_server(&server, &bundle, None).await.unwrap();
    let mut registry = McpToolRegistry::default();
    registry.register(&server, discovered).unwrap();
    let name = registry.descriptors().into_iter().next().unwrap().name;
    let registered = registry.resolve(&name).unwrap().clone();
    let result = invoke_custom_mcp(repository, store, &registered, json!({}), false, None, None)
        .await
        .unwrap();
    assert!(result.to_string().contains("synthetic-history-project"));
    registered
}

#[tokio::test]
async fn uncertain_rotation_reconnects_and_all_consumers_use_new_grant_after_reopen() {
    timeout(Duration::from_secs(20), async {
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!(
            "sqlite:{}?mode=rwc",
            directory.path().join("grant-history.sqlite3").display()
        );
        let repository = history_repository(&database_url).await;
        let fixture = HistoryHttp::start().await;
        let store = HistoryStore::default();
        let server = history_seed(&repository, &store, &fixture).await;
        store.fail_old_put.store(true, Ordering::SeqCst);
        assert!(test_mcp_definition(&repository, &store, &server.id)
            .await
            .is_err());
        store.fail_old_put.store(false, Ordering::SeqCst);
        let uncertain = repository.get(&server.id).await.unwrap();
        assert_eq!(
            uncertain.metadata["oauthRotatedRevocationUnconfirmed"],
            true
        );
        assert!(oauth_lifecycle(&server.id)
            .unconfirmed_revocation
            .load(Ordering::SeqCst));
        assert_eq!(
            fixture.observed.lock().unwrap().revoked,
            vec!["synthetic-history-r1-refresh"]
        );

        // A failed browser consent cannot clear the active generation's warning.
        assert!(
            history_connect(&repository, &store, &server.id, &fixture, true)
                .await
                .is_err()
        );
        let after_denial = repository.get(&server.id).await.unwrap();
        assert_eq!(after_denial.secret_ref.as_deref(), Some(OLD_REF));
        assert_eq!(
            after_denial.metadata["oauthRotatedRevocationUnconfirmed"],
            true
        );
        assert!(oauth_lifecycle(&server.id)
            .unconfirmed_revocation
            .load(Ordering::SeqCst));
        let connected = history_connect(&repository, &store, &server.id, &fixture, false)
            .await
            .unwrap();
        assert_ne!(connected.secret_ref.as_deref(), Some(OLD_REF));
        // RED on immutable2568: Connect returns success, but its actual Test
        // consumer rejects the new token because the old warning is global.
        assert_eq!(
            test_mcp_definition(&repository, &store, &server.id)
                .await
                .expect("new committed grant must be usable despite prior uncertainty")
                .len(),
            1
        );
        let after_activation = repository.get(&server.id).await.unwrap();
        assert_prior_history(&after_activation);
        let entries = history_entries(&after_activation);
        assert!(!oauth_lifecycle(&server.id)
            .unconfirmed_revocation
            .load(Ordering::SeqCst));
        let registered = history_consumer_chain(&repository, &store, &server.id).await;
        let healthy = repository.get(&server.id).await.unwrap();
        assert_eq!(history_entries(&healthy), entries);
        assert_ne!(
            healthy.metadata.get("oauthRotatedRevocationUnconfirmed"),
            Some(&json!(true))
        );

        // A stale registry cannot dispatch even though the current grant is valid.
        let calls_before = fixture
            .observed
            .lock()
            .unwrap()
            .mcp
            .iter()
            .filter(|(method, _)| method == "tools/call")
            .count();
        let mut changed = healthy.clone();
        changed.metadata["oauthRevision"] = json!(Uuid::new_v4().to_string());
        repository.replace(&changed).await.unwrap();
        assert!(matches!(
            invoke_custom_mcp(
                &repository,
                &store,
                &registered,
                json!({}),
                false,
                None,
                None
            )
            .await,
            Err(AgentMcpError::ConnectionChanged)
        ));
        assert_eq!(
            fixture
                .observed
                .lock()
                .unwrap()
                .mcp
                .iter()
                .filter(|(method, _)| method == "tools/call")
                .count(),
            calls_before
        );
        retire_server_sessions(&server.id).await;
        repository.pool.close().await;
        OAUTH_LIFECYCLES
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .remove(&server.id);
        if let Some(diagnostics) = MCP_DIAGNOSTICS.get() {
            diagnostics.lock().unwrap().remove(&server.id);
        }
        let reopened = history_repository(&database_url).await;
        assert_eq!(
            test_mcp_definition(&reopened, &store, &server.id)
                .await
                .unwrap()
                .len(),
            1
        );
        history_consumer_chain(&reopened, &store, &server.id).await;
        assert_eq!(
            history_entries(&reopened.get(&server.id).await.unwrap()),
            entries
        );
        {
            let observed = fixture.observed.lock().unwrap();
            assert_eq!(observed.refreshes, 1);
            assert_eq!(observed.exchanges, 1);
            assert_eq!(
                observed
                    .mcp
                    .iter()
                    .filter(|(method, _)| method == "tools/call")
                    .count(),
                calls_before + 1
            );
            assert!(observed
                .mcp
                .iter()
                .all(|(_, bearer)| bearer == "Bearer synthetic-history-r2-access"));
        }
        // Explicit teardown can confirm R0 and R2, but neither is the lost R1.
        // Matching the old reference alone must not discharge that obligation.
        let disconnected = disconnect_oauth(&reopened, &store, &server.id)
            .await
            .unwrap();
        assert!(disconnected.local_cleared);
        assert_eq!(disconnected.remote_revoked, Some(false));
        assert_prior_history(&disconnected.server);
        {
            let observed = fixture.observed.lock().unwrap();
            assert!(observed
                .revoked
                .iter()
                .any(|token| token == "synthetic-history-r0-refresh"));
            assert!(observed
                .revoked
                .iter()
                .any(|token| token == "synthetic-history-r2-refresh"));
        }
        retire_server_sessions(&server.id).await;
        reopened.pool.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn successful_first_grant_does_not_invent_a_prior_revocation_obligation() {
    let repository = history_repository("sqlite::memory:").await;
    let fixture = HistoryHttp::start().await;
    let store = HistoryStore::default();
    let mut server =
        McpServerDefinition::new("history-clean-fixture", McpTransport::StreamableHttp);
    server.url = Some(format!("{}/mcp", fixture.origin));
    server.enabled = false;
    server.metadata = json!({"auth":"oauth"});
    repository.create(&server).await.unwrap();
    history_connect(&repository, &store, &server.id, &fixture, false)
        .await
        .unwrap();
    assert_eq!(
        test_mcp_definition(&repository, &store, &server.id)
            .await
            .unwrap()
            .len(),
        1
    );
    history_consumer_chain(&repository, &store, &server.id).await;
    assert!(history_entries(&repository.get(&server.id).await.unwrap()).is_empty());
    assert!(fixture.observed.lock().unwrap().revoked.is_empty());
    retire_server_sessions(&server.id).await;
    repository.pool.close().await;
}

#[tokio::test]
async fn invalid_grant_stays_fail_closed_without_fabricating_history() {
    let repository = history_repository("sqlite::memory:").await;
    let fixture = HistoryHttp::start().await;
    fixture.reject_refresh.store(true, Ordering::SeqCst);
    let store = HistoryStore::default();
    let server = history_seed(&repository, &store, &fixture).await;
    let error = test_mcp_definition(&repository, &store, &server.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, "agent_mcp_reconnect_required");
    let current = repository.get(&server.id).await.unwrap();
    assert_eq!(current.secret_ref.as_deref(), Some(OLD_REF));
    assert!(history_entries(&current).is_empty());
    assert_ne!(
        current.metadata.get("oauthRotatedRevocationUnconfirmed"),
        Some(&json!(true))
    );
    assert!(!oauth_lifecycle(&server.id)
        .unconfirmed_revocation
        .load(Ordering::SeqCst));
    assert!(fixture.observed.lock().unwrap().mcp.is_empty());
    assert!(fixture.observed.lock().unwrap().revoked.is_empty());
    assert_eq!(fixture.observed.lock().unwrap().refreshes, 1);
    repository.pool.close().await;
}
