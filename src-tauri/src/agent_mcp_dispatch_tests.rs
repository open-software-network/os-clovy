//! Synthetic HTTP failures exercise the actual custom-MCP dispatch path.
//! A server may have applied tools/call before any of these responses arrive.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;
use std::sync::atomic::AtomicUsize;
use tokio::net::{TcpListener, TcpStream};

#[derive(Default)]
struct DispatchSecrets(Mutex<BTreeMap<String, McpSecretBundle>>);
impl McpSecretStore for DispatchSecrets {
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

#[derive(Clone, Copy, Debug)]
enum FailedDelivery {
    Unauthorized,
    ServerError,
    MalformedResponse,
    ToolFailure,
}

struct DispatchHttp {
    origin: String,
    requests: Arc<AtomicUsize>,
    tool_calls: Arc<AtomicUsize>,
    refreshes: Arc<AtomicUsize>,
    arguments: Arc<Mutex<Vec<Value>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for DispatchHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn read_dispatch_request(stream: &mut TcpStream) -> (String, Value) {
    let mut request = Vec::new();
    let mut bytes = [0; 4096];
    loop {
        let count = stream.read(&mut bytes).await.unwrap();
        assert!(count > 0, "dispatch fixture request ended early");
        request.extend_from_slice(&bytes[..count]);
        if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_owned();
                let body = if path == "/token" {
                    Value::Null
                } else {
                    serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap()
                };
                return (path, body);
            }
        }
    }
}

impl DispatchHttp {
    async fn start(failure: FailedDelivery) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let requests = Arc::new(AtomicUsize::new(0));
        let tool_calls = Arc::new(AtomicUsize::new(0));
        let refreshes = Arc::new(AtomicUsize::new(0));
        let arguments = Arc::new(Mutex::new(Vec::new()));
        let observed_calls = tool_calls.clone();
        let observed_requests = requests.clone();
        let observed_refreshes = refreshes.clone();
        let observed_arguments = arguments.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (path, frame) = read_dispatch_request(&mut stream).await;
                observed_requests.fetch_add(1, Ordering::SeqCst);
                let (status, body) = if path == "/token" {
                    // Make a mistaken refresh/replay capable of completing;
                    // the counters must catch it rather than a fixture outage.
                    observed_refreshes.fetch_add(1, Ordering::SeqCst);
                    ("200 OK", json!({"access_token":"dispatch-fixture-refreshed","refresh_token":"dispatch-fixture-refresh","token_type":"Bearer","expires_in":3600}).to_string())
                } else {
                    match frame["method"].as_str().unwrap() {
                        "initialize" => ("200 OK", json!({"jsonrpc":"2.0","id":frame["id"],"result":{"protocolVersion":MCP_PROTOCOL_VERSION,"capabilities":{"tools":{}},"serverInfo":{"name":"dispatch-fixture","version":"1"}}}).to_string()),
                        "notifications/initialized" => ("202 Accepted", String::new()),
                        "tools/call" => {
                            assert_eq!(frame["params"]["name"], "mutate");
                            observed_calls.fetch_add(1, Ordering::SeqCst);
                            observed_arguments.lock().unwrap().push(frame["params"]["arguments"].clone());
                            match failure {
                                FailedDelivery::Unauthorized => ("401 Unauthorized", String::new()),
                                FailedDelivery::ServerError => ("500 Internal Server Error", String::new()),
                                FailedDelivery::MalformedResponse => ("200 OK", "{malformed-json".into()),
                                FailedDelivery::ToolFailure => ("200 OK", json!({"jsonrpc":"2.0","id":frame["id"],"result":{"isError":true,"content":[{"type":"text","text":"dispatch-fixture-private-payload"}]}}).to_string()),
                            }
                        }
                        _ => panic!("unexpected MCP method in dispatch fixture"),
                    }
                };
                let response = format!("HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
                stream.write_all(response.as_bytes()).await.unwrap();
            }
        });
        Self {
            origin,
            requests,
            tool_calls,
            refreshes,
            arguments,
            task,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum RegisteredChange {
    Url,
    SecretReference,
    Policy,
    Visibility,
    MetadataRevision,
    Name,
    Enabled,
    Generation,
}

#[tokio::test]
async fn custom_dispatch_rejects_completed_connection_changes_before_any_request() {
    for change in [
        RegisteredChange::Url,
        RegisteredChange::SecretReference,
        RegisteredChange::Policy,
        RegisteredChange::Visibility,
        RegisteredChange::MetadataRevision,
        RegisteredChange::Name,
        RegisteredChange::Enabled,
        RegisteredChange::Generation,
    ] {
        let http = DispatchHttp::start(FailedDelivery::Unauthorized).await;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let repository = AgentMcpRepository::new(pool);
        repository.ensure_schema().await.unwrap();
        let mut server = McpServerDefinition::new(
            format!("snapshot-{}", Uuid::new_v4().simple()),
            McpTransport::StreamableHttp,
        );
        server.url = Some(format!("{}/mcp", http.origin));
        server.secret_ref = Some(format!("snapshot-secret-{}", Uuid::new_v4().simple()));
        repository.create(&server).await.unwrap();
        let store = DispatchSecrets::default();
        let mut bundle = McpSecretBundle::default();
        bundle
            .oauth
            .insert("access_token".into(), "snapshot-fixture-access".into());
        store
            .put(server.secret_ref.as_deref().unwrap(), &bundle)
            .unwrap();
        let mut registry = McpToolRegistry::default();
        registry
            .register(
                &server,
                vec![McpDiscoveredTool {
                    name: "mutate".into(),
                    description: "Synthetic consequential action".into(),
                    input_schema: json!({"type":"object"}),
                    annotations: McpToolAnnotations::from_hints(Some(false), Some(true)),
                }],
            )
            .unwrap();
        let runtime_name = runtime_tool_name(&server.name, "mutate").unwrap();
        let registered = registry.resolve(&runtime_name).unwrap();
        match change {
            RegisteredChange::Url => server.url = Some(format!("{}/replacement", http.origin)),
            RegisteredChange::SecretReference => {
                server.secret_ref = Some("snapshot-new-reference".into());
                store.put("snapshot-new-reference", &bundle).unwrap();
            }
            RegisteredChange::Policy => server.safety.requires_approval = false,
            RegisteredChange::Visibility => server.tool_visibility.exclude.push("mutate".into()),
            RegisteredChange::MetadataRevision => {
                server.metadata["revision"] = json!("reconnected")
            }
            RegisteredChange::Name => server.name.push_str("-renamed"),
            RegisteredChange::Enabled => server.enabled = false,
            RegisteredChange::Generation => invalidate_oauth(&oauth_lifecycle(&server.id)),
        }
        repository.replace(&server).await.unwrap();
        let error = invoke_custom_mcp(
            &repository,
            &store,
            registered,
            json!({"value":17}),
            false,
            None,
            None,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, AgentMcpError::ConnectionChanged),
            "{change:?}"
        );
        assert_eq!(http.requests.load(Ordering::SeqCst), 0, "{change:?}");
        assert_eq!(http.tool_calls.load(Ordering::SeqCst), 0, "{change:?}");
        assert_eq!(http.refreshes.load(Ordering::SeqCst), 0, "{change:?}");
        retire_server_sessions(&server.id).await;
    }
}

#[tokio::test]
async fn custom_dispatch_never_replays_a_mutation_after_ambiguous_http_responses() {
    for (failure, expected_status) in [
        (FailedDelivery::Unauthorized, Some(401)),
        (FailedDelivery::ServerError, Some(500)),
        (FailedDelivery::MalformedResponse, None),
        (FailedDelivery::ToolFailure, None),
    ] {
        let http = DispatchHttp::start(failure).await;
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let repository = AgentMcpRepository::new(pool);
        repository.ensure_schema().await.unwrap();
        let mut server = McpServerDefinition::new(
            format!("dispatch-{}", Uuid::new_v4().simple()),
            McpTransport::StreamableHttp,
        );
        server.url = Some(format!("{}/mcp", http.origin));
        server.secret_ref = Some(format!("dispatch-secret-{}", Uuid::new_v4().simple()));
        repository.create(&server).await.unwrap();
        let store = DispatchSecrets::default();
        let mut bundle = McpSecretBundle::default();
        bundle.oauth = BTreeMap::from([
            ("access_token".into(), "dispatch-fixture-access".into()),
            ("refresh_token".into(), "dispatch-fixture-refresh".into()),
            ("token_endpoint".into(), format!("{}/token", http.origin)),
            ("client_id".into(), "dispatch-fixture-clovy".into()),
            ("resource".into(), format!("{}/mcp", http.origin)),
            ("expires_at_unix".into(), i64::MAX.to_string()),
        ]);
        store
            .put(server.secret_ref.as_deref().unwrap(), &bundle)
            .unwrap();
        let mut registry = McpToolRegistry::default();
        registry
            .register(
                &server,
                vec![McpDiscoveredTool {
                    name: "mutate".into(),
                    description: "Synthetic consequential action".into(),
                    input_schema: json!({"type":"object","properties":{"value":{"type":"integer"}}}),
                    annotations: McpToolAnnotations::from_hints(Some(false), Some(true)),
                }],
            )
            .unwrap();
        let runtime_name = runtime_tool_name(&server.name, "mutate").unwrap();
        let registered = registry.resolve(&runtime_name).unwrap();
        assert_eq!(registered.remote_name, "mutate");
        assert_eq!(registered.descriptor.requires_approval, Some(true));
        let arguments = json!({"value":17});
        let error = invoke_custom_mcp(
            &repository,
            &store,
            registered,
            arguments.clone(),
            false,
            None,
            None,
        )
        .await
        .unwrap_err();
        let diagnostic = error.diagnostic(&server.name, Some(&runtime_name), "call");
        assert_eq!(
            diagnostic.code,
            if matches!(failure, FailedDelivery::ToolFailure) {
                "agent_mcp_tool_failed"
            } else {
                "agent_mcp_delivery_uncertain"
            }
        );
        assert_eq!(diagnostic.remote_code, expected_status);
        assert_eq!(diagnostic.action, "inspect_server_before_retry");
        assert!(!serde_json::to_string(&diagnostic)
            .unwrap()
            .contains("dispatch-fixture-private-payload"));
        if matches!(failure, FailedDelivery::ToolFailure) {
            assert!(matches!(error, AgentMcpError::ToolReportedError));
        } else {
            assert!(
                matches!(error, AgentMcpError::DeliveryUncertain(code) if code == expected_status)
            );
        }
        assert_eq!(http.tool_calls.load(Ordering::SeqCst), 1, "{failure:?}");
        assert_eq!(http.refreshes.load(Ordering::SeqCst), 0, "{failure:?}");
        assert_eq!(*http.arguments.lock().unwrap(), [arguments]);
        let current = repository.get(&server.id).await.unwrap();
        assert!(current.enabled);
        assert_eq!(current.secret_ref, server.secret_ref);
        assert_eq!(
            store
                .get(server.secret_ref.as_deref().unwrap())
                .unwrap()
                .unwrap()
                .oauth["access_token"],
            "dispatch-fixture-access"
        );
        retire_server_sessions(&server.id).await;
    }
}
