//! Real SQLite and isolated HTTP proofs of credential custody. Secrets are
//! task-owned synthetic values, never OS credentials or account fixtures.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
struct FixtureStore {
    bundles: Mutex<BTreeMap<String, McpSecretBundle>>,
    fail_delete: AtomicBool,
}
impl McpSecretStore for FixtureStore {
    fn put(&self, id: &str, bundle: &McpSecretBundle) -> Result<(), AgentMcpError> {
        self.bundles
            .lock()
            .unwrap()
            .insert(id.into(), bundle.clone());
        Ok(())
    }
    fn get(&self, id: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        Ok(self.bundles.lock().unwrap().get(id).cloned())
    }
    fn delete(&self, id: &str) -> Result<(), AgentMcpError> {
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(AgentMcpError::SecureStorage);
        }
        self.bundles.lock().unwrap().remove(id);
        Ok(())
    }
}
async fn fixture_repository() -> AgentMcpRepository {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    repository
}
async fn saved_server(
    repository: &AgentMcpRepository,
    store: &FixtureStore,
) -> McpServerDefinition {
    let mut server = McpServerDefinition::new("custody", McpTransport::StreamableHttp);
    server.url = Some("https://example.test/mcp".into());
    server.secret_ref = Some("previous".into());
    store.put("previous", &token_bundle("old-fixture")).unwrap();
    repository.create(&server).await.unwrap();
    server
}
fn token_bundle(value: &str) -> McpSecretBundle {
    let mut bundle = McpSecretBundle::default();
    bundle.oauth.insert("access_token".into(), value.into());
    bundle
}
async fn revocable_token_bundle(value: &str) -> (McpSecretBundle, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let mut bundle = token_bundle(value);
    bundle.oauth.insert(
        "revocation_endpoint".into(),
        format!("http://{}/revoke", listener.local_addr().unwrap()),
    );
    let expected = value.to_owned();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut bytes = [0; 2048];
            let count = stream.read(&mut bytes).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&bytes[..count]);
            if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() < end + 4 + length {
                    continue;
                }
                let form: BTreeMap<String, String> =
                    url::form_urlencoded::parse(&request[end + 4..end + 4 + length])
                        .into_owned()
                        .collect();
                assert_eq!(form["token"], expected);
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
                break;
            }
        }
    });
    (bundle, task)
}
async fn reject_activation(repository: &AgentMcpRepository) {
    query("CREATE TRIGGER reject_activation BEFORE UPDATE ON agent_mcp_servers WHEN NEW.secret_ref != OLD.secret_ref BEGIN SELECT RAISE(ABORT, 'fixture activation failure'); END")
        .execute(&repository.pool).await.unwrap();
}

#[tokio::test]
async fn activation_rollback_cleans_new_entry_and_preserves_previous_grant() {
    let repository = fixture_repository().await;
    let store = FixtureStore::default();
    let server = saved_server(&repository, &store).await;
    reject_activation(&repository).await;
    let (candidate, revocation) = revocable_token_bundle("new-fixture").await;
    let error = persist_oauth_connection(&repository, &store, &server, &candidate)
        .await
        .unwrap_err();
    revocation.await.unwrap();
    assert_eq!(error.code, "agent_mcp_storage_failed");
    let persisted = repository.get(&server.id).await.unwrap();
    assert_eq!(persisted.secret_ref, server.secret_ref);
    assert!(persisted.metadata.get("oauthPendingSecretRef").is_none());
    assert_eq!(store.bundles.lock().unwrap().len(), 1);
    assert_eq!(
        store.get("previous").unwrap().unwrap().oauth["access_token"],
        "old-fixture"
    );
}

#[tokio::test]
async fn failed_rollback_keeps_durable_pointer_and_restart_recovers() {
    let repository = fixture_repository().await;
    let store = FixtureStore::default();
    let server = saved_server(&repository, &store).await;
    reject_activation(&repository).await;
    store.fail_delete.store(true, Ordering::SeqCst);
    let (candidate, revocation) = revocable_token_bundle("new-fixture").await;
    let error = persist_oauth_connection(&repository, &store, &server, &candidate)
        .await
        .unwrap_err();
    revocation.await.unwrap();
    assert_eq!(error.code, "agent_mcp_oauth_rollback_failed");
    let mut persisted = repository.get(&server.id).await.unwrap();
    let attempts = persisted.metadata["oauthConnectAttempts"]
        .as_array()
        .unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0]["phase"], "revoked_but_local_cleanup_pending");
    let pending = attempts[0]["secretRef"].as_str().unwrap().to_owned();
    assert!(store.get(&pending).unwrap().is_some());
    assert_eq!(
        load_oauth_secrets(&repository, &store, &persisted, false)
            .await
            .unwrap()
            .0
            .oauth["access_token"],
        "old-fixture"
    );
    store.fail_delete.store(false, Ordering::SeqCst);
    // Recreate the repository owner exactly as a process restart would, using
    // the persisted journal rather than any in-memory cleanup task.
    let restarted = AgentMcpRepository::new(repository.pool.clone());
    recover_pending_oauth(&restarted, &store, &mut persisted)
        .await
        .unwrap();
    assert!(store.get(&pending).unwrap().is_none());
    assert!(restarted
        .get(&server.id)
        .await
        .unwrap()
        .metadata
        .get("oauthConnectAttempts")
        .is_none());
    assert_eq!(store.bundles.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn crash_before_or_after_vault_put_is_recoverable_without_active_entry_deletion() {
    for put_completed in [false, true] {
        let repository = fixture_repository().await;
        let store = FixtureStore::default();
        let mut server = saved_server(&repository, &store).await;
        server.metadata["oauthPendingSecretRef"] = json!("pending-fixture");
        repository.replace(&server).await.unwrap();
        let revocation = if put_completed {
            let (candidate, task) = revocable_token_bundle("new-fixture").await;
            store.put("pending-fixture", &candidate).unwrap();
            Some(task)
        } else {
            None
        };
        recover_pending_oauth(&repository, &store, &mut server)
            .await
            .unwrap();
        recover_pending_oauth(&repository, &store, &mut server)
            .await
            .unwrap();
        if let Some(revocation) = revocation {
            revocation.await.unwrap();
        }
        assert!(store.get("pending-fixture").unwrap().is_none());
        assert_eq!(oauth_has_revocation_history(&server), !put_completed);
        assert!(store.get("previous").unwrap().is_some());
        assert!(server.metadata.get("oauthPendingSecretRef").is_none());
    }
}

#[tokio::test]
async fn disconnect_preserves_non_oauth_credentials_and_cleans_pending_journal() {
    let repository = fixture_repository().await;
    let store = FixtureStore::default();
    let mut server = saved_server(&repository, &store).await;
    let mut bundle = token_bundle("old-fixture");
    bundle
        .headers
        .insert("x-fixture".into(), "synthetic-header".into());
    store.put("previous", &bundle).unwrap();
    server.metadata["oauthPendingSecretRef"] = json!("pending-fixture");
    let (candidate, revocation) = revocable_token_bundle("new-fixture").await;
    store.put("pending-fixture", &candidate).unwrap();
    repository.replace(&server).await.unwrap();
    let result = disconnect_oauth(&repository, &store, &server.id)
        .await
        .unwrap();
    revocation.await.unwrap();
    assert!(result.local_cleared);
    assert!(!result.server.enabled);
    let retained = store.get("previous").unwrap().unwrap();
    assert!(retained.oauth.is_empty());
    assert_eq!(retained.headers["x-fixture"], "synthetic-header");
    assert!(store.get("pending-fixture").unwrap().is_none());
}

#[tokio::test]
async fn refresh_finishing_after_disconnect_cannot_resurrect_tokens() {
    let repository = fixture_repository().await;
    let store = Arc::new(FixtureStore::default());
    let server = saved_server(&repository, &store).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let mut bundle = token_bundle("old-fixture");
    bundle.oauth.extend(BTreeMap::from([
        ("refresh_token".into(), "refresh-fixture".into()),
        ("client_id".into(), "clovy-fixture".into()),
        ("token_endpoint".into(), format!("{origin}/token")),
        ("expires_at_unix".into(), "0".into()),
    ]));
    store.put("previous", &bundle).unwrap();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(stream.read(&mut bytes).await.unwrap() > 0);
        started_tx.send(()).unwrap();
        release_rx.await.unwrap();
        let body = r#"{"access_token":"rotated-fixture","refresh_token":"rotated-refresh","expires_in":3600}"#;
        let wire = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        stream.write_all(wire.as_bytes()).await.unwrap();
    });
    let worker_repo = repository.clone();
    let worker_store = store.clone();
    let worker_server = server.clone();
    let refresh = tokio::spawn(async move {
        load_oauth_secrets(&worker_repo, worker_store.as_ref(), &worker_server, false).await
    });
    started_rx.await.unwrap();
    let disconnect_repo = repository.clone();
    let disconnect_store = store.clone();
    let id = server.id.clone();
    let disconnect = tokio::spawn(async move {
        disconnect_oauth(&disconnect_repo, disconnect_store.as_ref(), &id).await
    });
    wait_oauth_invalidation(&oauth_lifecycle(&server.id), 0).await;
    release_tx.send(()).unwrap();
    assert!(matches!(
        refresh.await.unwrap(),
        Err(AgentMcpError::OauthReconnectRequired)
    ));
    assert!(disconnect.await.unwrap().unwrap().local_cleared);
    fixture.await.unwrap();
    assert!(store.get("previous").unwrap().is_none());
    assert!(!repository.get(&server.id).await.unwrap().enabled);
}

#[tokio::test]
async fn concurrent_consumers_use_one_rotated_refresh_grant() {
    let repository = fixture_repository().await;
    let store = FixtureStore::default();
    let server = saved_server(&repository, &store).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut bundle = token_bundle("old-fixture");
    bundle.oauth.extend(BTreeMap::from([
        ("refresh_token".into(), "refresh-fixture".into()),
        ("client_id".into(), "clovy-fixture".into()),
        (
            "token_endpoint".into(),
            format!("http://{}/token", listener.local_addr().unwrap()),
        ),
        ("expires_at_unix".into(), "0".into()),
    ]));
    store.put("previous", &bundle).unwrap();
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        assert!(stream.read(&mut bytes).await.unwrap() > 0);
        let body = r#"{"access_token":"rotated-fixture","refresh_token":"rotated-refresh","expires_in":3600}"#;
        let wire = format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
        stream.write_all(wire.as_bytes()).await.unwrap();
        // The listener is dropped after the only grant is redeemed. A second
        // redemption would fail the consumer assertions instead of being hidden.
    });
    let (first, second) = tokio::join!(
        load_oauth_secrets(&repository, &store, &server, false),
        load_oauth_secrets(&repository, &store, &server, false),
    );
    let (first, refreshed_first) = first.unwrap();
    let (second, refreshed_second) = second.unwrap();
    assert_eq!(first.oauth, second.oauth);
    assert_ne!(refreshed_first, refreshed_second);
    assert_eq!(first.oauth["refresh_token"], "rotated-refresh");
    fixture.await.unwrap();
}

#[tokio::test]
async fn file_backed_journal_survives_pool_close_and_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("oauth-fixture.sqlite3");
    let options = sqlx_sqlite::SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone())
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    let store = FixtureStore::default();
    let mut server = saved_server(&repository, &store).await;
    server.metadata["oauthPendingSecretRef"] = json!("pending-fixture");
    repository.replace(&server).await.unwrap();
    let (candidate, revocation) = revocable_token_bundle("new-fixture").await;
    store.put("pending-fixture", &candidate).unwrap();
    repository.pool.close().await;
    let restarted = AgentMcpRepository::new(
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap(),
    );
    let mut persisted = restarted.get(&server.id).await.unwrap();
    recover_pending_oauth(&restarted, &store, &mut persisted)
        .await
        .unwrap();
    revocation.await.unwrap();
    assert!(store.get("pending-fixture").unwrap().is_none());
    assert!(store.get("previous").unwrap().is_some());
    assert!(restarted
        .get(&server.id)
        .await
        .unwrap()
        .metadata
        .get("oauthPendingSecretRef")
        .is_none());
}

#[tokio::test]
async fn local_server_removal_reports_remote_uncertainty_without_an_impossible_retry() {
    let repository = fixture_repository().await;
    let store = FixtureStore::default();
    let mut server = saved_server(&repository, &store).await;
    server.metadata["oauthRemoteRevocationFailed"] = json!(true);
    server.metadata["oauthRotatedRevocationUnconfirmed"] = json!(true);
    repository.replace(&server).await.unwrap();
    let result = delete_mcp_definition(&repository, &store, &server.id)
        .await
        .unwrap();
    assert!(result.local_deleted);
    assert_eq!(result.remote_revoked, Some(false));
    assert!(matches!(
        repository.get(&server.id).await,
        Err(AgentMcpError::NotFound)
    ));
    assert!(store.get("previous").unwrap().is_none());
}

#[tokio::test]
async fn local_server_removal_retains_retry_path_when_secure_cleanup_fails() {
    let repository = fixture_repository().await;
    let store = FixtureStore::default();
    let server = saved_server(&repository, &store).await;
    store.fail_delete.store(true, Ordering::SeqCst);
    let error = delete_mcp_definition(&repository, &store, &server.id)
        .await
        .unwrap_err();
    assert!(matches!(error, AgentMcpError::OauthCleanupPending));
    let retained = repository.get(&server.id).await.unwrap();
    assert!(!retained.enabled);
    assert_eq!(retained.secret_ref, server.secret_ref);
    assert!(store.get("previous").unwrap().is_some());
}
