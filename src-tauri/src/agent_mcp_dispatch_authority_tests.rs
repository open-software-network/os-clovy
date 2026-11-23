//! Actual custom invocation after refresh retires its transport. The fixture
//! gates real HTTP requests, rather than simulating eligibility flags.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;

#[derive(Default)]
struct AuthorityStore(Mutex<BTreeMap<String, McpSecretBundle>>);

impl McpSecretStore for AuthorityStore {
    fn put(&self, reference: &str, bundle: &McpSecretBundle) -> Result<(), AgentMcpError> {
        self.0
            .lock()
            .unwrap()
            .insert(reference.into(), bundle.clone());
        Ok(())
    }
    fn get(&self, reference: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        Ok(self.0.lock().unwrap().get(reference).cloned())
    }
    fn delete(&self, reference: &str) -> Result<(), AgentMcpError> {
        self.0.lock().unwrap().remove(reference);
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq)]
enum AuthorityHttpGate {
    Open,
    Initialize,
    ToolResponse,
    RefreshToken,
}

#[derive(Default)]
struct AuthorityRequests {
    initializes: usize,
    refreshes: usize,
    calls: usize,
}

struct AuthorityHttpFixture {
    origin: String,
    requests: Arc<Mutex<AuthorityRequests>>,
    started: Option<tokio::sync::oneshot::Receiver<()>>,
    release: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for AuthorityHttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn authority_http_request(stream: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        assert!(bytes.len() < 64 * 1024);
        let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..end]);
        let length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            let path = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .to_owned();
            return (path, bytes[end + 4..end + 4 + length].to_vec());
        }
    }
}

async fn authority_http_reply(stream: &mut tokio::net::TcpStream, status: &str, body: Value) {
    let body = body.to_string();
    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
}

impl AuthorityHttpFixture {
    async fn start(gate: AuthorityHttpGate) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(AuthorityRequests::default()));
        let observed = requests.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut started = Some(started_tx);
            let mut release = Some(release_rx);
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (path, body) = authority_http_request(&mut stream).await;
                if path == "/token" {
                    let form: BTreeMap<_, _> =
                        url::form_urlencoded::parse(&body).into_owned().collect();
                    assert_eq!(form["grant_type"], "refresh_token");
                    assert_eq!(form["refresh_token"], "synthetic-authority-old-refresh");
                    observed.lock().unwrap().refreshes += 1;
                    if gate == AuthorityHttpGate::RefreshToken {
                        started.take().unwrap().send(()).unwrap();
                        release.take().unwrap().await.unwrap();
                    }
                    authority_http_reply(&mut stream, "200 OK", json!({"access_token":"synthetic-authority-new-access","refresh_token":"synthetic-authority-new-refresh","expires_in":3600})).await;
                    continue;
                }
                assert_eq!(path, "/mcp");
                let request: Value = serde_json::from_slice(&body).unwrap();
                let id = request.get("id").cloned().unwrap_or(Value::Null);
                match request["method"].as_str().unwrap() {
                    "initialize" => {
                        let should_wait = {
                            let mut requests = observed.lock().unwrap();
                            requests.initializes += 1;
                            gate == AuthorityHttpGate::Initialize && requests.initializes == 2
                        };
                        if should_wait {
                            started.take().unwrap().send(()).unwrap();
                            release.take().unwrap().await.unwrap();
                        }
                        authority_http_reply(&mut stream, "200 OK", json!({"jsonrpc":"2.0","id":id,"result":{"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"authority-fixture","version":"1"}}})).await;
                    }
                    "notifications/initialized" => {
                        stream.write_all(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
                    }
                    "tools/list" => {
                        authority_http_reply(&mut stream, "200 OK", json!({"jsonrpc":"2.0","id":id,"result":{"tools":[{"name":"mutate","description":"Increment a synthetic counter","inputSchema":{"type":"object","properties":{}},"annotations":{"readOnlyHint":false}}]}})).await;
                    }
                    "tools/call" => {
                        observed.lock().unwrap().calls += 1;
                        if gate == AuthorityHttpGate::ToolResponse {
                            started.take().unwrap().send(()).unwrap();
                            release.take().unwrap().await.unwrap();
                        }
                        authority_http_reply(&mut stream, "200 OK", json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":"synthetic mutation applied"}]}})).await;
                    }
                    _ => panic!("unexpected authority fixture method"),
                }
            }
        });
        Self {
            origin,
            requests,
            started: Some(started_rx),
            release: Some(release_tx),
            task,
        }
    }
}

async fn authority_invocation_case(gate: AuthorityHttpGate) {
    let mut fixture = AuthorityHttpFixture::start(gate).await;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    let store = Arc::new(AuthorityStore::default());
    let mut server = McpServerDefinition::new("dispatch-authority", McpTransport::StreamableHttp);
    server.url = Some(format!("{}/mcp", fixture.origin));
    server.secret_ref = Some("authority-fixture".into());
    server.metadata = json!({"auth":"oauth","oauthConnected":true});
    let bundle = McpSecretBundle {
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        oauth: BTreeMap::from([
            (
                "access_token".into(),
                "synthetic-authority-old-access".into(),
            ),
            (
                "refresh_token".into(),
                "synthetic-authority-old-refresh".into(),
            ),
            ("client_id".into(), "synthetic-authority-client".into()),
            ("token_endpoint".into(), format!("{}/token", fixture.origin)),
            ("expires_at_unix".into(), "0".into()),
        ]),
    };
    store.put("authority-fixture", &bundle).unwrap();
    repository.create(&server).await.unwrap();
    // Seed a real initialized transport. The actual invocation must rotate
    // the expired grant, retire this transport, and initialize a new one.
    let tools = discover_server(&server, &bundle, None).await.unwrap();
    let mut registry = McpToolRegistry::default();
    registry.register(&server, tools).unwrap();
    let registered = registry
        .resolve("mcp_dispatch_authority_mutate")
        .unwrap()
        .clone();
    let worker_repository = repository.clone();
    let worker_store = store.clone();
    let captured = registered.clone();
    let invocation = tokio::spawn(async move {
        invoke_custom_mcp(
            &worker_repository,
            worker_store.as_ref(),
            &captured,
            json!({}),
            false,
            None,
            None,
        )
        .await
    });
    let result = if gate == AuthorityHttpGate::Open {
        invocation.await.unwrap()
    } else {
        fixture.started.as_mut().unwrap().await.unwrap();
        let disconnect_repository = repository.clone();
        let disconnect_store = store.clone();
        let id = server.id.clone();
        let disconnect = tokio::spawn(async move {
            disconnect_oauth(&disconnect_repository, disconnect_store.as_ref(), &id).await
        });
        wait_oauth_invalidation(&oauth_lifecycle(&server.id), registered.lifecycle_epoch).await;
        fixture.release.take().unwrap().send(()).unwrap();
        let result = invocation.await.unwrap();
        assert!(disconnect.await.unwrap().unwrap().local_cleared);
        result
    };
    let (initializes, refreshes, calls) = {
        let requests = fixture.requests.lock().unwrap();
        (requests.initializes, requests.refreshes, requests.calls)
    };
    assert_eq!(initializes, 2);
    assert_eq!(refreshes, 1);
    match gate {
        AuthorityHttpGate::Initialize => {
            assert_eq!(
                calls, 0,
                "a mutation must not leave after authority was withdrawn while initializing"
            );
            assert!(matches!(result, Err(AgentMcpError::ConnectionChanged)));
        }
        AuthorityHttpGate::Open => {
            assert_eq!(calls, 1);
            assert!(result.is_ok());
        }
        AuthorityHttpGate::ToolResponse => {
            assert_eq!(
                calls, 1,
                "an already delivered mutation must not be replayed"
            );
            assert!(matches!(result, Err(AgentMcpError::DeliveryUncertain(_))));
        }
        AuthorityHttpGate::RefreshToken => {
            panic!("refresh concurrency uses its own consumer scenario")
        }
    }
    retire_server_sessions(&server.id).await;
}

#[tokio::test]
async fn c3_held_http_initialize_does_not_dispatch_after_disconnect() {
    tokio::time::timeout(
        Duration::from_secs(15),
        authority_invocation_case(AuthorityHttpGate::Initialize),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn c3_valid_rotated_connection_dispatches_once_after_transport_reinitializes() {
    tokio::time::timeout(
        Duration::from_secs(15),
        authority_invocation_case(AuthorityHttpGate::Open),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn c3_delivered_mutation_remains_uncertain_without_replay_after_disconnect() {
    tokio::time::timeout(
        Duration::from_secs(15),
        authority_invocation_case(AuthorityHttpGate::ToolResponse),
    )
    .await
    .unwrap();
}

async fn refresh_snapshot_setup(
    fixture: &AuthorityHttpFixture,
) -> (
    AgentMcpRepository,
    Arc<AuthorityStore>,
    McpServerDefinition,
    McpSecretBundle,
) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    let store = Arc::new(AuthorityStore::default());
    let mut server = McpServerDefinition::new("refresh-snapshot", McpTransport::StreamableHttp);
    server.url = Some(format!("{}/mcp", fixture.origin));
    server.secret_ref = Some("refresh-snapshot-fixture".into());
    server.metadata = json!({"auth":"oauth","oauthConnected":true});
    let bundle = McpSecretBundle {
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        oauth: BTreeMap::from([
            (
                "access_token".into(),
                "synthetic-authority-old-access".into(),
            ),
            (
                "refresh_token".into(),
                "synthetic-authority-old-refresh".into(),
            ),
            ("client_id".into(), "synthetic-authority-client".into()),
            ("token_endpoint".into(), format!("{}/token", fixture.origin)),
            ("expires_at_unix".into(), "0".into()),
        ]),
    };
    store
        .put(server.secret_ref.as_deref().unwrap(), &bundle)
        .unwrap();
    repository.create(&server).await.unwrap();
    (repository, store, server, bundle)
}

#[tokio::test]
async fn transient_refresh_journal_does_not_invalidate_a_catalog_captured_while_token_waits() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let mut fixture = AuthorityHttpFixture::start(AuthorityHttpGate::RefreshToken).await;
        let (repository, store, server, bundle) = refresh_snapshot_setup(&fixture).await;
        let tools = discover_server(&server, &bundle, None).await.unwrap();
        let initial_fingerprint = custom_mcp_server_fingerprint(&server).unwrap();
        let test_repository = repository.clone();
        let test_store = store.clone();
        let id = server.id.clone();
        let testing = tokio::spawn(async move {
            test_mcp_definition(&test_repository, test_store.as_ref(), &id).await
        });
        fixture.started.as_mut().unwrap().await.unwrap();
        let observed = repository.get(&server.id).await.unwrap();
        assert_eq!(observed.metadata["oauthRotatedRevocationUnconfirmed"], true);
        assert_eq!(observed.metadata["oauthRemoteRevocationFailed"], true);
        assert_eq!(
            custom_mcp_server_fingerprint(&observed).unwrap(),
            initial_fingerprint
        );
        let mut registry = McpToolRegistry::default();
        registry.register(&observed, tools).unwrap();
        let registered = registry
            .resolve("mcp_refresh_snapshot_mutate")
            .unwrap()
            .clone();
        let call_repository = repository.clone();
        let call_store = store.clone();
        let invocation = tokio::spawn(async move {
            invoke_custom_mcp(
                &call_repository,
                call_store.as_ref(),
                &registered,
                json!({}),
                false,
                None,
                None,
            )
            .await
        });
        fixture.release.take().unwrap().send(()).unwrap();
        assert!(testing.await.unwrap().is_ok());
        assert!(invocation.await.unwrap().is_ok());
        {
            let requests = fixture.requests.lock().unwrap();
            assert_eq!(requests.refreshes, 1);
            assert_eq!(requests.calls, 1);
        }
        let current = repository.get(&server.id).await.unwrap();
        assert!(current
            .metadata
            .get("oauthRotatedRevocationUnconfirmed")
            .is_none());
        assert_eq!(
            custom_mcp_server_fingerprint(&current).unwrap(),
            initial_fingerprint
        );
        retire_server_sessions(&server.id).await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn permanent_refresh_uncertainty_still_blocks_all_wire_despite_health_normalization() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let fixture = AuthorityHttpFixture::start(AuthorityHttpGate::Open).await;
        let (repository, store, mut server, _) = refresh_snapshot_setup(&fixture).await;
        server.metadata["oauthRotatedRevocationUnconfirmed"] = json!(true);
        repository.replace(&server).await.unwrap();
        let mut registry = McpToolRegistry::default();
        registry
            .register(
                &server,
                vec![McpDiscoveredTool {
                    name: "mutate".into(),
                    description: "Synthetic effect".into(),
                    input_schema: json!({"type":"object"}),
                    annotations: McpToolAnnotations::default(),
                }],
            )
            .unwrap();
        let registered = registry.resolve("mcp_refresh_snapshot_mutate").unwrap();
        let error = invoke_custom_mcp(
            &repository,
            store.as_ref(),
            registered,
            json!({}),
            false,
            None,
            None,
        )
        .await
        .unwrap_err();
        let diagnostic = error.diagnostic(&server.name, Some("mutate"), "call");
        assert_eq!(diagnostic.code, "agent_mcp_oauth_revoke_unconfirmed");
        assert_eq!(diagnostic.action, "revoke_at_provider");
        let requests = fixture.requests.lock().unwrap();
        assert_eq!(requests.initializes, 0);
        assert_eq!(requests.refreshes, 0);
        assert_eq!(requests.calls, 0);
    })
    .await
    .unwrap();
}
