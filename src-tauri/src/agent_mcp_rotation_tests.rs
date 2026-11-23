//! A provider can burn the old refresh token before disconnect starts. A 200
//! for revoking that old token must not hide failed revocation of the fresh
//! grant returned to an already invalidated refresh operation.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;

#[derive(Default)]
struct RotationStore(Mutex<BTreeMap<String, McpSecretBundle>>);

impl McpSecretStore for RotationStore {
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

async fn read_rotation_request(
    stream: &mut tokio::net::TcpStream,
) -> (String, BTreeMap<String, String>) {
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
            let form = url::form_urlencoded::parse(&bytes[end + 4..end + 4 + length])
                .into_owned()
                .collect();
            return (path, form);
        }
    }
}

async fn reply_rotation(stream: &mut tokio::net::TcpStream, status: &str, body: Value) {
    let body = body.to_string();
    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
}

async fn rotation_disconnect_case(fresh_revocation_succeeds: bool) {
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite:{}?mode=rwc",
        directory.path().join("rotation.sqlite3").display()
    );
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    let store = Arc::new(RotationStore::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let mut server = McpServerDefinition::new("rotation-disconnect", McpTransport::StreamableHttp);
    server.url = Some(format!("{origin}/mcp"));
    server.secret_ref = Some("rotation-fixture".into());
    server.metadata = json!({"auth":"oauth","oauthConnected":true});
    repository.create(&server).await.unwrap();
    let bundle = McpSecretBundle {
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        oauth: BTreeMap::from([
            ("access_token".into(), "synthetic-old-access".into()),
            ("refresh_token".into(), "synthetic-burned-refresh".into()),
            ("client_id".into(), "synthetic-clovy-client".into()),
            ("token_endpoint".into(), format!("{origin}/token")),
            ("revocation_endpoint".into(), format!("{origin}/revoke")),
            ("expires_at_unix".into(), "0".into()),
        ]),
    };
    store.put("rotation-fixture", &bundle).unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (path, form) = read_rotation_request(&mut stream).await;
        assert_eq!(path, "/token");
        assert_eq!(form["refresh_token"], "synthetic-burned-refresh");
        started_tx.send(()).unwrap();
        release_rx.await.unwrap();
        reply_rotation(&mut stream, "200 OK", json!({"access_token":"synthetic-rotated-access","refresh_token":"synthetic-rotated-refresh","expires_in":3600})).await;
        // The stale refresh must compensate using the newly returned token,
        // before disconnect acquires the gate and reads the old stored token.
        let (mut stream, _) = listener.accept().await.unwrap();
        let (path, form) = read_rotation_request(&mut stream).await;
        assert_eq!(path, "/revoke");
        assert_eq!(form["token"], "synthetic-rotated-refresh");
        reply_rotation(
            &mut stream,
            if fresh_revocation_succeeds {
                "200 OK"
            } else {
                "503 Service Unavailable"
            },
            json!({}),
        )
        .await;
        let (mut stream, _) = listener.accept().await.unwrap();
        let (path, form) = read_rotation_request(&mut stream).await;
        assert_eq!(path, "/revoke");
        assert_eq!(form["token"], "synthetic-burned-refresh");
        // RFC 7009 permits success for a nonexistent/burned token. The helper
        // must retain the fresh-grant failure despite this successful status.
        reply_rotation(&mut stream, "200 OK", json!({})).await;
    });
    let worker_repository = repository.clone();
    let worker_store = store.clone();
    let worker_server = server.clone();
    let refresh = tokio::spawn(async move {
        load_oauth_secrets(
            &worker_repository,
            worker_store.as_ref(),
            &worker_server,
            false,
        )
        .await
    });
    started_rx.await.unwrap();
    let disconnect_repository = repository.clone();
    let disconnect_store = store.clone();
    let id = server.id.clone();
    let disconnect = tokio::spawn(async move {
        disconnect_oauth(&disconnect_repository, disconnect_store.as_ref(), &id).await
    });
    wait_oauth_invalidation(&oauth_lifecycle(&server.id), 0).await;
    release_tx.send(()).unwrap();
    assert!(matches!(
        refresh.await.unwrap(),
        Err(AgentMcpError::OauthReconnectRequired)
    ));
    let disconnected = disconnect.await.unwrap().unwrap();
    fixture.await.unwrap();
    assert!(disconnected.local_cleared);
    assert!(!disconnected.server.enabled);
    assert_eq!(disconnected.remote_revoked, Some(fresh_revocation_succeeds));
    assert!(store.get("rotation-fixture").unwrap().is_none());
    let persisted = repository.get(&server.id).await.unwrap();
    if !fresh_revocation_succeeds {
        assert_eq!(
            persisted.metadata["oauthRotatedRevocationUnconfirmed"],
            true
        );
        assert_eq!(persisted.metadata["oauthRemoteRevocationFailed"], true);
        let diagnostic = disconnected.server.diagnostic.unwrap();
        assert_eq!(diagnostic.code, "agent_mcp_oauth_revoke_unconfirmed");
        assert_eq!(diagnostic.action, "revoke_at_provider");
    }
    repository.pool.close().await;
    // Recreate the run-time lifecycle as well as the DB connection. The
    // durable marker, rather than the previous process's atomic fallback,
    // must preserve the unconfirmed remote-grant state on the next load.
    OAUTH_LIFECYCLES
        .get()
        .unwrap()
        .lock()
        .unwrap()
        .remove(&server.id);
    let reopened_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(&database_url)
        .await
        .unwrap();
    let reopened = AgentMcpRepository::new(reopened_pool);
    let persisted = reopened.get(&server.id).await.unwrap();
    assert!(!persisted.enabled);
    assert!(persisted.secret_ref.is_none());
    if !fresh_revocation_succeeds {
        assert_eq!(
            persisted.metadata["oauthRotatedRevocationUnconfirmed"],
            true
        );
        let retried = disconnect_oauth(&reopened, store.as_ref(), &server.id)
            .await
            .unwrap();
        assert!(retried.local_cleared);
        assert_eq!(retried.remote_revoked, Some(false));
    }
}

#[tokio::test]
async fn failed_fresh_grant_revocation_is_not_hidden_by_success_for_burned_token() {
    tokio::time::timeout(Duration::from_secs(10), rotation_disconnect_case(false))
        .await
        .unwrap();
}

#[tokio::test]
async fn stale_refresh_revokes_fresh_grant_before_disconnect_clears_old_credentials() {
    tokio::time::timeout(Duration::from_secs(10), rotation_disconnect_case(true))
        .await
        .unwrap();
}
