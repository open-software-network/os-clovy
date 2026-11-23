//! Exercise the same generic command implementation called by Settings Test.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Default)]
struct TestSecrets {
    values: Mutex<BTreeMap<String, McpSecretBundle>>,
    fail_rotated_put: AtomicBool,
}
impl McpSecretStore for TestSecrets {
    fn put(&self, key: &str, bundle: &McpSecretBundle) -> Result<(), AgentMcpError> {
        if self.fail_rotated_put.load(Ordering::SeqCst)
            && bundle.oauth.get("access_token").map(String::as_str) == Some("test-rotated-access")
        {
            return Err(AgentMcpError::SecureStorage);
        }
        self.values
            .lock()
            .unwrap()
            .insert(key.into(), bundle.clone());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<(), AgentMcpError> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

async fn test_request(stream: &mut tokio::net::TcpStream) -> (String, String) {
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = stream.read(&mut chunk).await.unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&chunk[..count]);
        let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&bytes[..end]);
        let size = headers
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        if bytes.len() >= end + 4 + size {
            return (
                headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .into(),
                String::from_utf8(bytes[end + 4..end + 4 + size].to_vec()).unwrap(),
            );
        }
    }
}

async fn command_rotation_case(disconnect_during_refresh: bool) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    let store = Arc::new(TestSecrets::default());
    store
        .fail_rotated_put
        .store(!disconnect_during_refresh, Ordering::SeqCst);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let fresh_revocations = Arc::new(AtomicUsize::new(0));
    let requests = fresh_revocations.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let fixture = tokio::spawn(async move {
        let mut started = Some(started_tx);
        let mut release = Some(release_rx);
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (path, body) = test_request(&mut stream).await;
            let form: BTreeMap<String, String> = url::form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect();
            let (status, body) = match path.as_str() {
                "/token" => {
                    assert_eq!(form["refresh_token"], "test-burned-refresh");
                    started.take().unwrap().send(()).unwrap();
                    release.take().unwrap().await.unwrap();
                    (
                        "200 OK",
                        json!({"access_token":"test-rotated-access","refresh_token":"test-rotated-refresh","expires_in":3600}),
                    )
                }
                "/revoke"
                    if form.get("token").map(String::as_str) == Some("test-rotated-refresh") =>
                {
                    requests.fetch_add(1, Ordering::SeqCst);
                    ("503 Service Unavailable", json!({}))
                }
                "/revoke" => {
                    assert_eq!(form["token"], "test-burned-refresh");
                    ("200 OK", json!({}))
                }
                _ => panic!("failed Test refresh must not reach catalog discovery"),
            };
            let body = body.to_string();
            stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    let mut server =
        McpServerDefinition::new("test-command-rotation", McpTransport::StreamableHttp);
    server.url = Some(format!("{origin}/mcp"));
    server.secret_ref = Some(format!("test-secret-{}", Uuid::new_v4()));
    server.metadata = json!({"auth":"oauth","oauthConnected":true});
    repository.create(&server).await.unwrap();
    let bundle = McpSecretBundle {
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        oauth: BTreeMap::from([
            ("access_token".into(), "test-old-access".into()),
            ("refresh_token".into(), "test-burned-refresh".into()),
            ("token_endpoint".into(), format!("{origin}/token")),
            ("revocation_endpoint".into(), format!("{origin}/revoke")),
            ("client_id".into(), "test-own-client".into()),
            ("expires_at_unix".into(), "0".into()),
        ]),
    };
    store
        .put(server.secret_ref.as_deref().unwrap(), &bundle)
        .unwrap();
    let worker_repo = repository.clone();
    let worker_store = store.clone();
    let id = server.id.clone();
    let testing =
        tokio::spawn(
            async move { test_mcp_definition(&worker_repo, worker_store.as_ref(), &id).await },
        );
    started_rx.await.unwrap();
    let disconnected = if disconnect_during_refresh {
        let worker_repo = repository.clone();
        let worker_store = store.clone();
        let id = server.id.clone();
        let disconnect = tokio::spawn(async move {
            disconnect_oauth(&worker_repo, worker_store.as_ref(), &id).await
        });
        wait_oauth_invalidation(&oauth_lifecycle(&server.id), 0).await;
        release_tx.send(()).unwrap();
        assert!(testing.await.unwrap().is_err());
        disconnect.await.unwrap().unwrap()
    } else {
        release_tx.send(()).unwrap();
        assert_eq!(
            testing.await.unwrap().unwrap_err().code,
            "agent_mcp_secure_storage_failed"
        );
        disconnect_oauth(&repository, store.as_ref(), &server.id)
            .await
            .unwrap()
    };
    let fresh = fresh_revocations.load(Ordering::SeqCst);
    fixture.abort();
    assert_eq!(
        fresh, 1,
        "Test must compensate the grant it actually received"
    );
    assert!(disconnected.local_cleared);
    assert_eq!(disconnected.remote_revoked, Some(false));
    assert_eq!(
        disconnected.server.diagnostic.unwrap().action,
        "revoke_at_provider"
    );
    assert!(store
        .get(server.secret_ref.as_deref().unwrap())
        .unwrap()
        .is_none());
    let persisted = repository.get(&server.id).await.unwrap();
    assert!(!persisted.enabled);
    assert_eq!(
        persisted.metadata["oauthRotatedRevocationUnconfirmed"],
        true
    );
}

#[tokio::test]
async fn settings_test_compensates_rotation_when_the_vault_put_fails() {
    tokio::time::timeout(Duration::from_secs(10), command_rotation_case(false))
        .await
        .unwrap();
}

#[tokio::test]
async fn settings_test_compensates_rotation_when_disconnect_invalidates_refresh() {
    tokio::time::timeout(Duration::from_secs(10), command_rotation_case(true))
        .await
        .unwrap();
}
