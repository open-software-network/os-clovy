//! End-to-end connect fixtures: production OAuth callback, HTTP transport,
//! registry persistence and secret-store boundary, with synthetic data only.
use super::*;
use base64::Engine;
use sqlx_sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;
use tokio::net::{TcpListener, TcpStream};

#[derive(Clone, Default)]
struct FixtureStore(Arc<Mutex<BTreeMap<String, McpSecretBundle>>>);

impl McpSecretStore for FixtureStore {
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

#[derive(Default)]
struct Observed {
    registration: Option<Value>,
    authorization: Option<BTreeMap<String, String>>,
    exchange: Option<BTreeMap<String, String>>,
    methods: Vec<String>,
    callback_statuses: Vec<u16>,
    revocations: Vec<String>,
}

struct HttpFixture {
    origin: String,
    observed: Arc<Mutex<Observed>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for HttpFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_fixture_request(stream: &mut TcpStream) -> (String, String, String) {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0, "fixture request ended before its body");
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
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
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                return (
                    path,
                    headers.into_owned(),
                    String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap(),
                );
            }
        }
    }
}

async fn reply_fixture(stream: &mut TcpStream, status: &str, body: Value) {
    let body = body.to_string();
    let wire = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    stream.write_all(wire.as_bytes()).await.unwrap();
}

impl HttpFixture {
    async fn start(valid_catalog: bool) -> Self {
        Self::start_with_catalog(if valid_catalog {
            json!({"tools": [{"name": "list_projects", "description": "Synthetic project listing", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}}]})
        } else {
            json!({"tools": "invalid-catalog"})
        }).await
    }

    async fn start_with_catalog(catalog: Value) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let observed = Arc::new(Mutex::new(Observed::default()));
        let seen = observed.clone();
        let host = origin.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (path, headers, body) = read_fixture_request(&mut stream).await;
                let result = match path.as_str() {
                    "/.well-known/oauth-protected-resource/mcp" => json!({
                        "resource": format!("{host}/mcp"),
                        "authorization_servers": [host],
                        "scopes_supported": ["openid"]
                    }),
                    "/.well-known/oauth-authorization-server" => json!({
                        "issuer": host,
                        "authorization_endpoint": format!("{host}/authorize"),
                        "token_endpoint": format!("{host}/token"),
                        "registration_endpoint": format!("{host}/register"),
                        "revocation_endpoint": format!("{host}/revoke"),
                        "response_types_supported": ["code"],
                        "grant_types_supported": ["authorization_code", "refresh_token"],
                        "code_challenge_methods_supported": ["S256"],
                        "token_endpoint_auth_methods_supported": ["none"]
                    }),
                    "/register" => {
                        seen.lock().unwrap().registration =
                            Some(serde_json::from_str(&body).unwrap());
                        json!({"client_id": "clovy-fixture-client"})
                    }
                    "/token" => {
                        let form: BTreeMap<String, String> =
                            url::form_urlencoded::parse(body.as_bytes())
                                .into_owned()
                                .collect();
                        let refreshed =
                            form.get("grant_type").map(String::as_str) == Some("refresh_token");
                        seen.lock().unwrap().exchange = Some(form);
                        json!({"access_token": if refreshed { "connect-fixture-access-refreshed" } else { "connect-fixture-access" }, "refresh_token": "connect-fixture-refresh", "expires_in": 3600, "scope": "openid", "token_type": "Bearer"})
                    }
                    "/revoke" => {
                        let form: BTreeMap<String, String> =
                            url::form_urlencoded::parse(body.as_bytes())
                                .into_owned()
                                .collect();
                        assert_eq!(form["token"], "connect-fixture-refresh");
                        seen.lock().unwrap().revocations.push(form["token"].clone());
                        Value::Null
                    }
                    "/mcp" => {
                        assert!(
                            headers.lines().any(|line| line.eq_ignore_ascii_case(
                                "authorization: Bearer connect-fixture-access"
                            )),
                            "catalog requires the exchanged synthetic bearer"
                        );
                        let request: Value = serde_json::from_str(&body).unwrap();
                        let method = request["method"].as_str().unwrap();
                        seen.lock().unwrap().methods.push(method.into());
                        let result = match method {
                            "initialize" => {
                                json!({"protocolVersion": MCP_PROTOCOL_VERSION, "capabilities": {"tools": {}}, "serverInfo": {"name": "connect-fixture", "version": "1"}})
                            }
                            "notifications/initialized" => {
                                reply_fixture(&mut stream, "202 Accepted", Value::Null).await;
                                continue;
                            }
                            "tools/list" => catalog.clone(),
                            _ => panic!("unexpected MCP fixture method"),
                        };
                        json!({"jsonrpc": "2.0", "id": request["id"], "result": result})
                    }
                    _ => panic!("unexpected HTTP fixture route"),
                };
                reply_fixture(&mut stream, "200 OK", result).await;
            }
        });
        Self {
            origin,
            observed,
            task,
        }
    }

    fn assert_authorization_contract(&self) {
        let observed = self.observed.lock().unwrap();
        let registration = observed.registration.as_ref().unwrap();
        assert_eq!(registration["client_name"], "Clovy");
        assert_eq!(registration["token_endpoint_auth_method"], "none");
        let authorization = observed.authorization.as_ref().unwrap();
        assert_eq!(authorization["response_type"], "code");
        assert_eq!(authorization["client_id"], "clovy-fixture-client");
        assert_eq!(authorization["code_challenge_method"], "S256");
        assert_eq!(authorization["scope"], "openid");
        assert_eq!(authorization["resource"], format!("{}/mcp", self.origin));
        assert!(authorization["state"].len() >= 32);
        let exchange = observed.exchange.as_ref().unwrap();
        assert_eq!(exchange["grant_type"], "authorization_code");
        assert_eq!(exchange["code"], "connect-fixture-code");
        assert_eq!(exchange["client_id"], authorization["client_id"]);
        assert_eq!(exchange["redirect_uri"], authorization["redirect_uri"]);
        assert_eq!(
            registration["redirect_uris"][0],
            authorization["redirect_uri"]
        );
        assert_eq!(exchange["resource"], authorization["resource"]);
        assert_eq!(
            authorization["code_challenge"],
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(Sha256::digest(exchange["code_verifier"].as_bytes()))
        );
        assert!(!exchange.contains_key("client_secret"));
    }

    async fn callback_statuses(&self, count: usize) -> Vec<u16> {
        timeout(Duration::from_secs(2), async {
            loop {
                let statuses = self.observed.lock().unwrap().callback_statuses.clone();
                if statuses.len() == count {
                    return statuses;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
}

#[derive(Clone, Copy)]
enum Callback {
    Success,
    Denied,
    ProviderError(&'static str),
    InvalidThenSuccess,
    InvalidOnly,
    Wait,
}

async fn callback_request(redirect: &str, state: &str, denied: bool) -> u16 {
    let mut callback = reqwest::Url::parse(redirect).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("state", state)
        .append_pair(
            if denied { "error" } else { "code" },
            if denied {
                "access_denied"
            } else {
                "connect-fixture-code"
            },
        );
    reqwest::Client::new()
        .get(callback)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn callback_error_request(redirect: &str, state: &str, error: &str) -> u16 {
    let mut callback = reqwest::Url::parse(redirect).unwrap();
    callback
        .query_pairs_mut()
        .append_pair("state", state)
        .append_pair("error", error)
        .append_pair("error_description", "fixture-private-description")
        .append_pair("code", "fixture-private-code");
    reqwest::Client::new()
        .get(callback)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn connect_fixture(
    repository: &AgentMcpRepository,
    store: &FixtureStore,
    definition: &McpServerDefinition,
    fixture: &HttpFixture,
    callback: Callback,
    deadline: Duration,
    opened: Arc<tokio::sync::Notify>,
) -> Result<McpServerDefinition, crate::domain::types::AppError> {
    connect_fixture_with_operation(
        repository, store, definition, fixture, callback, deadline, opened, None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn connect_fixture_with_operation(
    repository: &AgentMcpRepository,
    store: &FixtureStore,
    definition: &McpServerDefinition,
    fixture: &HttpFixture,
    callback: Callback,
    deadline: Duration,
    opened: Arc<tokio::sync::Notify>,
    operation_id: Option<&str>,
) -> Result<McpServerDefinition, crate::domain::types::AppError> {
    let seen = fixture.observed.clone();
    connect_oauth_with(
        repository,
        store,
        &definition.id,
        operation_id,
        deadline,
        move |server, bundle| async move {
            authorize_oauth_server_with(&server, &bundle, move |authorization_url| {
                let parameters: BTreeMap<String, String> = reqwest::Url::parse(authorization_url)
                    .unwrap()
                    .query_pairs()
                    .into_owned()
                    .collect();
                seen.lock().unwrap().authorization = Some(parameters.clone());
                opened.notify_one();
                if !matches!(callback, Callback::Wait) {
                    tokio::spawn(async move {
                        let redirect = &parameters["redirect_uri"];
                        let state = &parameters["state"];
                        if matches!(
                            callback,
                            Callback::InvalidThenSuccess | Callback::InvalidOnly
                        ) {
                            let status =
                                callback_request(redirect, "invalid-fixture-state", false).await;
                            seen.lock().unwrap().callback_statuses.push(status);
                            if matches!(callback, Callback::InvalidOnly) {
                                return;
                            }
                        }
                        let status = match callback {
                            Callback::ProviderError(error) => {
                                callback_error_request(redirect, state, error).await
                            }
                            _ => {
                                callback_request(
                                    redirect,
                                    state,
                                    matches!(callback, Callback::Denied),
                                )
                                .await
                            }
                        };
                        seen.lock().unwrap().callback_statuses.push(status);
                    });
                }
                Ok(())
            })
            .await
        },
    )
    .await
}

async fn setup_connect(
    fixture: &HttpFixture,
) -> (AgentMcpRepository, FixtureStore, McpServerDefinition) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::from_str("sqlite::memory:").unwrap())
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    let mut definition = McpServerDefinition::new("connect-fixture", McpTransport::StreamableHttp);
    definition.url = Some(format!("{}/mcp", fixture.origin));
    definition.enabled = false;
    definition.metadata = json!({"auth": "oauth"});
    repository.create(&definition).await.unwrap();
    (repository, FixtureStore::default(), definition)
}

async fn assert_no_grant(
    repository: &AgentMcpRepository,
    store: &FixtureStore,
    definition: &McpServerDefinition,
) {
    let persisted = repository.get(&definition.id).await.unwrap();
    assert!(!persisted.enabled);
    assert!(persisted.secret_ref.is_none());
    assert_ne!(persisted.metadata.get("oauthConnected"), Some(&json!(true)));
    assert!(persisted.metadata.get("oauthPendingSecretRef").is_none());
    assert!(store.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn connect_persists_only_after_valid_catalog_and_reloads_gateway_tools() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let connected = connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Success,
        Duration::from_secs(10),
        Arc::default(),
    )
    .await
    .unwrap();
    let persisted = repository.get(&definition.id).await.unwrap();
    assert!(persisted.enabled);
    assert_eq!(persisted.metadata["oauthConnected"], true);
    assert!(persisted.metadata["oauthRevision"].as_str().is_some());
    assert_eq!(persisted.secret_ref, connected.secret_ref);
    let bundle = store
        .get(persisted.secret_ref.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(bundle.oauth["access_token"], "connect-fixture-access");
    assert_eq!(bundle.oauth["refresh_token"], "connect-fixture-refresh");
    assert!(!serde_json::to_string(&persisted)
        .unwrap()
        .contains("connect-fixture-access"));
    assert!(fixture
        .observed
        .lock()
        .unwrap()
        .methods
        .iter()
        .any(|method| method == "tools/list"));
    fixture.assert_authorization_contract();
    let subsystem = AgentMcpSubsystem::new(repository, store);
    let tools = subsystem.refresh_registry_for(false).await.unwrap();
    let tool = tools
        .iter()
        .find(|tool| tool.name == "mcp_connect_fixture_list_projects")
        .unwrap();
    assert_eq!(tool.id, format!("mcp:{}/list_projects", definition.id));
    assert_eq!(tool.parameters["type"], "object");
    assert_eq!(tool.annotations.read_only_hint(), Some(true));
    retire_server_sessions(&definition.id).await;
}

#[tokio::test]
async fn oauth_candidate_uses_the_new_grant_and_keeps_unrelated_headers() {
    for header in ["Authorization", "aUtHoRiZaTiOn"] {
        let fixture = HttpFixture::start(true).await;
        let (repository, store, mut definition) = setup_connect(&fixture).await;
        definition.secret_ref = Some("old-manual-fixture".into());
        repository.replace(&definition).await.unwrap();
        let mut old = McpSecretBundle::default();
        old.headers
            .insert(header.into(), "Bearer synthetic-account-a".into());
        old.headers.insert("x-fixture".into(), "kept".into());
        store.put("old-manual-fixture", &old).unwrap();
        let connected = connect_fixture(
            &repository,
            &store,
            &definition,
            &fixture,
            Callback::Success,
            Duration::from_secs(10),
            Arc::default(),
        )
        .await
        .unwrap();
        let bundle = store
            .get(connected.secret_ref.as_deref().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(bundle.oauth["access_token"], "connect-fixture-access");
        assert!(!bundle
            .headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization")));
        assert_eq!(bundle.headers["x-fixture"], "kept");
        assert!(fixture
            .observed
            .lock()
            .unwrap()
            .methods
            .iter()
            .any(|method| method == "tools/list"));
        retire_server_sessions(&definition.id).await;
    }
}

#[tokio::test]
async fn failed_oauth_candidate_keeps_the_previous_manual_bundle() {
    let fixture = HttpFixture::start(false).await;
    let (repository, store, mut definition) = setup_connect(&fixture).await;
    definition.secret_ref = Some("old-manual-fixture".into());
    repository.replace(&definition).await.unwrap();
    let mut original = McpSecretBundle::default();
    original
        .headers
        .insert("aUtHoRiZaTiOn".into(), "Bearer synthetic-account-a".into());
    original.headers.insert("x-fixture".into(), "kept".into());
    store.put("old-manual-fixture", &original).unwrap();
    assert!(connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Success,
        Duration::from_secs(10),
        Arc::default()
    )
    .await
    .is_err());
    let persisted = repository.get(&definition.id).await.unwrap();
    assert_eq!(persisted.secret_ref.as_deref(), Some("old-manual-fixture"));
    assert_eq!(store.get("old-manual-fixture").unwrap(), Some(original));
}

#[tokio::test]
async fn denied_consent_never_exchanges_code_or_persists_grant() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let failure = connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Denied,
        Duration::from_secs(10),
        Arc::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.code, "agent_mcp_oauth_provider_rejected");
    assert_eq!(failure.details.unwrap()["phase"], "connect");
    assert!(fixture.observed.lock().unwrap().exchange.is_none());
    assert_no_grant(&repository, &store, &definition).await;
}

#[tokio::test]
async fn settings_invalid_grant_restores_prior_warning_and_real_reconnect_can_be_tested() {
    for previous_remote_failure in [false, true] {
        let fixture = HttpFixture::start(true).await;
        let (repository, store, definition) = setup_connect(&fixture).await;
        let mut connected = connect_fixture(
            &repository,
            &store,
            &definition,
            &fixture,
            Callback::Success,
            Duration::from_secs(10),
            Arc::default(),
        )
        .await
        .unwrap();
        if previous_remote_failure {
            connected.metadata["oauthRemoteRevocationFailed"] = json!(true);
            repository.replace(&connected).await.unwrap();
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let token_endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let rejected = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (path, _, body) = read_fixture_request(&mut stream).await;
            assert_eq!(path, "/token");
            let form: BTreeMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect();
            assert_eq!(form["grant_type"], "refresh_token");
            reply_fixture(
                &mut stream,
                "400 Bad Request",
                json!({"error":"invalid_grant"}),
            )
            .await;
        });
        let reference = connected.secret_ref.as_deref().unwrap();
        let mut expired = store.get(reference).unwrap().unwrap();
        expired.oauth.insert("expires_at_unix".into(), "0".into());
        expired
            .oauth
            .insert("token_endpoint".into(), token_endpoint);
        store.put(reference, &expired).unwrap();
        let error = test_mcp_definition(&repository, &store, &connected.id)
            .await
            .unwrap_err();
        rejected.await.unwrap();
        assert_eq!(error.code, "agent_mcp_reconnect_required");
        let persisted = repository.get(&connected.id).await.unwrap();
        assert!(persisted
            .metadata
            .get("oauthRotatedRevocationUnconfirmed")
            .is_none());
        assert_eq!(
            persisted
                .metadata
                .get("oauthRemoteRevocationFailed")
                .and_then(Value::as_bool),
            previous_remote_failure.then_some(true)
        );
        assert!(!oauth_lifecycle(&connected.id)
            .unconfirmed_revocation
            .load(Ordering::SeqCst));
        assert_eq!(store.get(reference).unwrap(), Some(expired));
        let reconnected = connect_fixture(
            &repository,
            &store,
            &persisted,
            &fixture,
            Callback::Success,
            Duration::from_secs(10),
            Arc::default(),
        )
        .await
        .unwrap();
        assert!(reconnected.enabled);
        let tools = test_mcp_definition(&repository, &store, &connected.id)
            .await
            .unwrap();
        assert!(tools.iter().any(|tool| tool.name == "list_projects"));
        retire_server_sessions(&connected.id).await;
    }
}

#[tokio::test]
async fn callback_client_and_provider_failures_keep_their_actual_cause_and_action() {
    for (callback_error, expected_code, expected_action) in [
        ("access_denied", "access_denied", "review_provider_consent"),
        (
            "unauthorized_client",
            "unauthorized_client",
            "check_provider_client_approval",
        ),
        ("server_error", "server_error", "configure"),
        (
            "temporarily_unavailable",
            "temporarily_unavailable",
            "configure",
        ),
        (
            "fixture-private-error-word",
            "unknown_oauth_error",
            "configure",
        ),
    ] {
        let fixture = HttpFixture::start(true).await;
        let (repository, store, definition) = setup_connect(&fixture).await;
        let failure = connect_fixture(
            &repository,
            &store,
            &definition,
            &fixture,
            Callback::ProviderError(callback_error),
            Duration::from_secs(10),
            Arc::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(failure.code, "agent_mcp_oauth_provider_rejected");
        let diagnostic = failure.details.as_ref().unwrap();
        assert_eq!(diagnostic["phase"], "connect");
        assert_eq!(diagnostic["action"], expected_action);
        assert!(diagnostic["message"]
            .as_str()
            .unwrap()
            .contains(expected_code));
        assert!(failure.message.contains(expected_code));
        assert!(failure.message.contains(expected_action));
        let encoded = serde_json::to_string(&failure).unwrap();
        for private in [
            "fixture-private-error-word",
            "fixture-private-description",
            "fixture-private-code",
        ] {
            assert!(!encoded.contains(private), "{encoded}");
        }
        if expected_code != "access_denied" {
            assert_ne!(diagnostic["action"], "review_provider_consent");
        }
        assert!(fixture.observed.lock().unwrap().exchange.is_none());
        assert_no_grant(&repository, &store, &definition).await;
    }
}

#[tokio::test]
async fn invalid_state_is_rejected_before_a_valid_callback_can_complete() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::InvalidThenSuccess,
        Duration::from_secs(10),
        Arc::default(),
    )
    .await
    .unwrap();
    assert_eq!(fixture.callback_statuses(2).await, vec![400, 200]);
    fixture.assert_authorization_contract();
    retire_server_sessions(&definition.id).await;
}

#[tokio::test]
async fn invalid_state_without_valid_callback_times_out_without_grant() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let failure = connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::InvalidOnly,
        Duration::from_secs(2),
        Arc::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.code, "agent_mcp_timeout");
    assert_eq!(
        fixture.observed.lock().unwrap().callback_statuses,
        vec![400]
    );
    assert!(fixture.observed.lock().unwrap().exchange.is_none());
    assert_no_grant(&repository, &store, &definition).await;
}

#[tokio::test]
async fn waiting_for_browser_consent_has_a_global_deadline() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let failure = connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Wait,
        Duration::from_secs(2),
        Arc::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.code, "agent_mcp_timeout");
    assert!(fixture.observed.lock().unwrap().authorization.is_some());
    assert!(fixture.observed.lock().unwrap().exchange.is_none());
    assert_no_grant(&repository, &store, &definition).await;
}

#[tokio::test]
async fn cancel_after_browser_open_closes_connect_without_saving_grant() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let opened = Arc::new(tokio::sync::Notify::new());
    let operation = connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Wait,
        Duration::from_secs(10),
        opened.clone(),
    );
    let cancellation = async {
        opened.notified().await;
        invalidate_oauth(&oauth_lifecycle(&definition.id));
    };
    let (result, ()) = tokio::join!(operation, cancellation);
    assert_eq!(result.unwrap_err().code, "agent_mcp_oauth_cancelled");
    assert!(fixture.observed.lock().unwrap().exchange.is_none());
    assert_no_grant(&repository, &store, &definition).await;
}

#[tokio::test]
async fn token_exchange_without_valid_catalog_never_marks_connection_successful() {
    let fixture = HttpFixture::start(false).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let failure = connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Success,
        Duration::from_secs(10),
        Arc::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.code, "agent_mcp_protocol_error");
    assert_eq!(failure.details.unwrap()["phase"], "discover");
    fixture.assert_authorization_contract();
    assert_eq!(fixture.observed.lock().unwrap().revocations.len(), 1);
    assert_no_grant(&repository, &store, &definition).await;
    retire_server_sessions(&definition.id).await;
}

#[tokio::test]
async fn final_database_commit_failure_removes_staged_credential_and_preserves_previous_bundle() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, mut definition) = setup_connect(&fixture).await;
    definition.secret_ref = Some("previous-fixture-bundle".into());
    repository.replace(&definition).await.unwrap();
    let mut previous = McpSecretBundle::default();
    previous
        .env
        .insert("FIXTURE_ENV".into(), "retained-fixture-value".into());
    store.put("previous-fixture-bundle", &previous).unwrap();
    query("CREATE TRIGGER reject_connected_commit BEFORE UPDATE ON agent_mcp_servers WHEN json_extract(NEW.metadata_json, '$.oauthConnected') = 1 BEGIN SELECT RAISE(FAIL, 'synthetic commit rejection'); END")
        .execute(&repository.pool).await.unwrap();
    let failure = connect_fixture(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Success,
        Duration::from_secs(10),
        Arc::default(),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.code, "agent_mcp_storage_failed");
    assert_eq!(fixture.observed.lock().unwrap().revocations.len(), 1);
    let persisted = repository.get(&definition.id).await.unwrap();
    assert!(!persisted.enabled);
    assert_eq!(persisted.secret_ref, definition.secret_ref);
    assert!(persisted.metadata.get("oauthPendingSecretRef").is_none());
    assert_ne!(persisted.metadata.get("oauthConnected"), Some(&json!(true)));
    assert_eq!(store.0.lock().unwrap().len(), 1);
    assert_eq!(
        store.get("previous-fixture-bundle").unwrap(),
        Some(previous)
    );
    retire_server_sessions(&definition.id).await;
}

#[tokio::test]
async fn cancellation_before_operation_registration_never_opens_authorization() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let operation_id = Uuid::new_v4().to_string();
    cancel_oauth_operation(definition.id.clone(), Some(operation_id.clone())).await;
    let failure = connect_fixture_with_operation(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Success,
        Duration::from_secs(10),
        Arc::default(),
        Some(&operation_id),
    )
    .await
    .unwrap_err();
    assert_eq!(failure.code, "agent_mcp_oauth_cancelled");
    {
        let observed = fixture.observed.lock().unwrap();
        assert!(observed.registration.is_none());
        assert!(observed.authorization.is_none());
        assert!(observed.exchange.is_none());
    }
    assert_no_grant(&repository, &store, &definition).await;
}

#[tokio::test]
async fn cancellation_after_completion_does_not_invalidate_connection_or_refresh() {
    let fixture = HttpFixture::start(true).await;
    let (repository, store, definition) = setup_connect(&fixture).await;
    let operation_id = Uuid::new_v4().to_string();
    let connected = connect_fixture_with_operation(
        &repository,
        &store,
        &definition,
        &fixture,
        Callback::Success,
        Duration::from_secs(10),
        Arc::default(),
        Some(&operation_id),
    )
    .await
    .unwrap();
    fixture.assert_authorization_contract();
    let lifecycle = oauth_lifecycle(&definition.id);
    let completed_epoch = lifecycle.epoch.load(Ordering::SeqCst);
    cancel_oauth_operation(definition.id.clone(), Some(operation_id)).await;
    assert_eq!(lifecycle.epoch.load(Ordering::SeqCst), completed_epoch);
    let (refreshed, did_refresh) = load_oauth_secrets(&repository, &store, &connected, true)
        .await
        .unwrap();
    assert!(did_refresh);
    assert_eq!(
        refreshed.oauth["access_token"],
        "connect-fixture-access-refreshed"
    );
    assert_eq!(
        store.get(connected.secret_ref.as_deref().unwrap()).unwrap(),
        Some(refreshed)
    );
    assert!(repository.get(&definition.id).await.unwrap().enabled);
    retire_server_sessions(&definition.id).await;
}

#[tokio::test]
async fn malformed_tool_contracts_never_persist_an_oauth_connection() {
    for catalog in [
        json!({"tools": [{"name": 42, "inputSchema": {"type": "object"}}]}),
        json!({"tools": [{"name": "", "inputSchema": {"type": "object"}}]}),
        json!({"tools": [{"name": "lookup", "inputSchema": null}]}),
        json!({"tools": [{"name": "lookup", "inputSchema": {"type": "array"}}]}),
        json!({"tools": [{"name": "lookup", "inputSchema": {"type": "object"}, "annotations": "invalid"}]}),
        json!({"tools": [{"name": "lookup", "inputSchema": {"type": "object"}, "description": 42}]}),
        json!({"tools": [
            {"name": "list.projects", "inputSchema": {"type": "object"}},
            {"name": "list-projects", "inputSchema": {"type": "object"}}
        ]}),
    ] {
        let fixture = HttpFixture::start_with_catalog(catalog).await;
        let (repository, store, definition) = setup_connect(&fixture).await;
        let failure = connect_fixture(
            &repository,
            &store,
            &definition,
            &fixture,
            Callback::Success,
            Duration::from_secs(10),
            Arc::default(),
        )
        .await
        .unwrap_err();
        assert_eq!(failure.code, "agent_mcp_protocol_error");
        assert_eq!(failure.details.unwrap()["phase"], "discover");
        assert_no_grant(&repository, &store, &definition).await;
        retire_server_sessions(&definition.id).await;
    }
}
