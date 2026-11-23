//! Consumer regressions for the irreversible boundary between remote
//! revocation, local credential loss and the final SQLite cleanup commit.
//! Every token, store, endpoint and database is owned by this test.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Default)]
struct JournalStore {
    bundles: Mutex<BTreeMap<String, McpSecretBundle>>,
    deletions: AtomicUsize,
    fail_put: AtomicBool,
    fail_delete: AtomicBool,
}

impl McpSecretStore for JournalStore {
    fn put(&self, reference: &str, bundle: &McpSecretBundle) -> Result<(), AgentMcpError> {
        if self.fail_put.load(Ordering::SeqCst) {
            return Err(AgentMcpError::SecureStorage);
        }
        self.bundles
            .lock()
            .unwrap()
            .insert(reference.into(), bundle.clone());
        Ok(())
    }
    fn get(&self, reference: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        Ok(self.bundles.lock().unwrap().get(reference).cloned())
    }
    fn delete(&self, reference: &str) -> Result<(), AgentMcpError> {
        self.deletions.fetch_add(1, Ordering::SeqCst);
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(AgentMcpError::SecureStorage);
        }
        self.bundles.lock().unwrap().remove(reference);
        Ok(())
    }
}

async fn journal_request(stream: &mut tokio::net::TcpStream) -> (String, BTreeMap<String, String>) {
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
            return (
                path,
                url::form_urlencoded::parse(&bytes[end + 4..end + 4 + length])
                    .into_owned()
                    .collect(),
            );
        }
    }
}

async fn revocation_fixture(
    statuses: Vec<u16>,
) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let observed = requests.clone();
    let task = tokio::spawn(async move {
        for status in statuses {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (path, form) = journal_request(&mut stream).await;
            assert_eq!(path, "/revoke");
            assert_eq!(form["token"], "synthetic-journal-refresh");
            observed.fetch_add(1, Ordering::SeqCst);
            let reason = if status == 200 {
                "OK"
            } else {
                "Service Unavailable"
            };
            stream.write_all(format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").as_bytes()).await.unwrap();
        }
    });
    (origin, requests, task)
}

async fn journal_repository(database_url: &str) -> AgentMcpRepository {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    repository
}

async fn journal_server(
    repository: &AgentMcpRepository,
    store: &JournalStore,
    origin: &str,
) -> McpServerDefinition {
    let mut server = McpServerDefinition::new("revocation-journal", McpTransport::StreamableHttp);
    server.url = Some(format!("{origin}/mcp"));
    server.secret_ref = Some("journal-fixture".into());
    server.metadata = json!({"auth":"oauth","oauthConnected":true});
    let bundle = McpSecretBundle {
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        oauth: BTreeMap::from([
            ("access_token".into(), "synthetic-journal-access".into()),
            ("refresh_token".into(), "synthetic-journal-refresh".into()),
            ("client_id".into(), "synthetic-journal-client".into()),
            ("revocation_endpoint".into(), format!("{origin}/revoke")),
        ]),
    };
    store.put("journal-fixture", &bundle).unwrap();
    repository.create(&server).await.unwrap();
    server
}

fn forget_journal_runtime(server_id: &str) {
    if let Some(lifecycles) = OAUTH_LIFECYCLES.get() {
        lifecycles.lock().unwrap().remove(server_id);
    }
    if let Some(diagnostics) = MCP_DIAGNOSTICS.get() {
        diagnostics.lock().unwrap().remove(server_id);
    }
}

async fn remote_failure_after_local_loss(delete_after_reopen: bool) {
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite:{}?mode=rwc",
        directory.path().join("journal.sqlite3").display()
    );
    let repository = journal_repository(&database_url).await;
    let store = JournalStore::default();
    let (origin, observed, fixture) = revocation_fixture(vec![503]).await;
    let server = journal_server(&repository, &store, &origin).await;
    // This fails the final cleanup commit only after the actual vault deletion.
    query("CREATE TRIGGER reject_final_cleanup BEFORE UPDATE ON agent_mcp_servers WHEN NEW.secret_ref IS NULL AND OLD.secret_ref IS NOT NULL BEGIN SELECT RAISE(ABORT, 'synthetic final cleanup failure'); END")
        .execute(&repository.pool).await.unwrap();
    let failed = if delete_after_reopen {
        delete_mcp_definition(&repository, &store, &server.id)
            .await
            .map(|_| ())
    } else {
        disconnect_oauth(&repository, &store, &server.id)
            .await
            .map(|_| ())
    };
    assert!(matches!(failed, Err(AgentMcpError::Storage)));
    fixture.await.unwrap();
    assert_eq!(observed.load(Ordering::SeqCst), 1);
    assert_eq!(store.deletions.load(Ordering::SeqCst), 1);
    assert!(store.get("journal-fixture").unwrap().is_none());
    repository.pool.close().await;
    forget_journal_runtime(&server.id);
    let reopened = journal_repository(&database_url).await;
    query("DROP TRIGGER reject_final_cleanup")
        .execute(&reopened.pool)
        .await
        .unwrap();
    // No flags were pre-populated by the fixture. The observed HTTP failure
    // must survive the failed final commit and the loss of in-memory state.
    let saved = reopened.get(&server.id).await.unwrap();
    assert!(!saved.enabled);
    if delete_after_reopen {
        let deleted = delete_mcp_definition(&reopened, &store, &server.id)
            .await
            .unwrap();
        assert!(deleted.local_deleted);
        assert_eq!(deleted.remote_revoked, Some(false));
        assert!(matches!(
            reopened.get(&server.id).await,
            Err(AgentMcpError::NotFound)
        ));
    } else {
        let disconnected = disconnect_oauth(&reopened, &store, &server.id)
            .await
            .unwrap();
        assert!(disconnected.local_cleared);
        assert_eq!(disconnected.remote_revoked, Some(false));
        assert_eq!(
            disconnected.server.metadata["oauthRemoteRevocationFailed"],
            true
        );
        let diagnostic = disconnected.server.diagnostic.unwrap();
        assert_eq!(diagnostic.code, "agent_mcp_oauth_revoke_failed");
        assert_eq!(diagnostic.action, "revoke_at_provider");
    }
}

#[tokio::test]
async fn c2_disconnect_retains_observed_failure_after_vault_loss_and_commit_failure() {
    tokio::time::timeout(
        Duration::from_secs(10),
        remote_failure_after_local_loss(false),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn c2_delete_retry_retains_observed_failure_after_vault_loss_and_reopen() {
    tokio::time::timeout(
        Duration::from_secs(10),
        remote_failure_after_local_loss(true),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn c2_failed_revocation_journal_keeps_current_grant_for_a_real_retry() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!("sqlite:{}?mode=rwc", directory.path().join("journal.sqlite3").display());
        let repository = journal_repository(&database_url).await;
        let store = JournalStore::default();
        let (origin, observed, fixture) = revocation_fixture(vec![503, 200]).await;
        let server = journal_server(&repository, &store, &origin).await;
        query("CREATE TRIGGER reject_revocation_journal BEFORE UPDATE ON agent_mcp_servers WHEN json_extract(NEW.metadata_json, '$.oauthRemoteRevocationFailed') = 1 BEGIN SELECT RAISE(ABORT, 'synthetic outcome journal failure'); END")
            .execute(&repository.pool).await.unwrap();
        assert!(matches!(disconnect_oauth(&repository, &store, &server.id).await, Err(AgentMcpError::Storage)));
        assert_eq!(observed.load(Ordering::SeqCst), 1);
        assert_eq!(store.deletions.load(Ordering::SeqCst), 0);
        assert!(store.get("journal-fixture").unwrap().is_some());
        repository.pool.close().await;
        forget_journal_runtime(&server.id);
        let reopened = journal_repository(&database_url).await;
        query("DROP TRIGGER reject_revocation_journal").execute(&reopened.pool).await.unwrap();
        let retried = disconnect_oauth(&reopened, &store, &server.id).await.unwrap();
        fixture.await.unwrap();
        assert_eq!(observed.load(Ordering::SeqCst), 2);
        assert!(retried.local_cleared);
        assert_eq!(retried.remote_revoked, Some(true));
        assert!(store.get("journal-fixture").unwrap().is_none());
    }).await.unwrap();
}

#[tokio::test]
async fn c2_pretoken_journal_survives_failed_compensation_and_immediate_restart() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let directory = tempfile::tempdir().unwrap();
        let database_url = format!("sqlite:{}?mode=rwc", directory.path().join("rotation-journal.sqlite3").display());
        let repository = journal_repository(&database_url).await;
        let store = JournalStore::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = journal_server(&repository, &store, &origin).await;
        let mut bundle = store.get("journal-fixture").unwrap().unwrap();
        bundle.oauth.insert("token_endpoint".into(), format!("{origin}/token"));
        bundle.oauth.insert("expires_at_unix".into(), "0".into());
        store.put("journal-fixture", &bundle).unwrap();
        let failure_pool = repository.pool.clone();
        let fixture = tokio::spawn(async move {
            for index in 0..3 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (path, form) = journal_request(&mut stream).await;
                let (status, body) = if index == 0 {
                    assert_eq!(path, "/token");
                    assert_eq!(form["refresh_token"], "synthetic-journal-refresh");
                    // Admit the real pre-token journal, then fail only later
                    // bookkeeping after the provider has consumed R0.
                    query("CREATE TRIGGER reject_rotation_journal BEFORE UPDATE ON agent_mcp_servers WHEN json_extract(NEW.metadata_json, '$.oauthRotatedRevocationUnconfirmed') = 1 BEGIN SELECT RAISE(ABORT, 'synthetic post-token journal failure'); END")
                        .execute(&failure_pool).await.unwrap();
                    ("200 OK", json!({"access_token":"synthetic-journal-rotated-access","refresh_token":"synthetic-journal-rotated-refresh","expires_in":3600}))
                } else {
                    assert_eq!(path, "/revoke");
                    assert_eq!(form["token"], if index == 1 {"synthetic-journal-rotated-refresh"} else {"synthetic-journal-refresh"});
                    (if index == 1 {"503 Service Unavailable"} else {"200 OK"}, json!({}))
                };
                let body = body.to_string();
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            }
        });
        store.fail_put.store(true, Ordering::SeqCst);
        assert!(matches!(load_oauth_secrets(&repository, &store, &server, false).await, Err(AgentMcpError::Storage)));
        store.fail_put.store(false, Ordering::SeqCst);
        // Restart immediately after failed put/revoke/bookkeeping. No
        // disconnect is allowed to repair the marker before this boundary.
        assert_eq!(repository.get(&server.id).await.unwrap().metadata["oauthRotatedRevocationUnconfirmed"], true);
        assert!(store.get("journal-fixture").unwrap().is_some());
        repository.pool.close().await;
        forget_journal_runtime(&server.id);
        let reopened = journal_repository(&database_url).await;
        query("DROP TRIGGER reject_rotation_journal").execute(&reopened.pool).await.unwrap();
        let retried = disconnect_oauth(&reopened, &store, &server.id).await.unwrap();
        fixture.await.unwrap();
        assert!(retried.local_cleared);
        assert_eq!(retried.remote_revoked, Some(false));
        assert_eq!(retried.server.metadata["oauthRotatedRevocationUnconfirmed"], true);
        assert_eq!(retried.server.diagnostic.unwrap().code, "agent_mcp_oauth_revoke_unconfirmed");
    }).await.unwrap();
}

async fn c1_refresh_interrupted_at_remote_boundary(before_token_response: bool) {
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite:{}?mode=rwc",
        directory.path().join("intent.sqlite3").display()
    );
    let repository = journal_repository(&database_url).await;
    let store = Arc::new(JournalStore::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = journal_server(&repository, store.as_ref(), &origin).await;
    let mut bundle = store.get("journal-fixture").unwrap().unwrap();
    bundle
        .oauth
        .insert("token_endpoint".into(), format!("{origin}/token"));
    bundle.oauth.insert("expires_at_unix".into(), "0".into());
    store.put("journal-fixture", &bundle).unwrap();
    store
        .fail_put
        .store(!before_token_response, Ordering::SeqCst);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    let fixture = tokio::spawn(async move {
        let mut started = Some(started_tx);
        let mut release = Some(release_rx);
        for index in 0..if before_token_response { 2 } else { 3 } {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (path, form) = journal_request(&mut stream).await;
            let hold =
                (index == 0 && before_token_response) || (index == 1 && !before_token_response);
            if hold {
                started.take().unwrap().send(()).unwrap();
                release.take().unwrap().await.unwrap();
            }
            let (status, body) = if index == 0 {
                assert_eq!(path, "/token");
                assert_eq!(form["refresh_token"], "synthetic-journal-refresh");
                (
                    "200 OK",
                    json!({"access_token":"synthetic-journal-rotated-access","refresh_token":"synthetic-journal-rotated-refresh","expires_in":3600}),
                )
            } else {
                assert_eq!(path, "/revoke");
                let fresh = !before_token_response && index == 1;
                assert_eq!(
                    form["token"],
                    if fresh {
                        "synthetic-journal-rotated-refresh"
                    } else {
                        "synthetic-journal-refresh"
                    }
                );
                (
                    if fresh {
                        "503 Service Unavailable"
                    } else {
                        "200 OK"
                    },
                    json!({}),
                )
            };
            let body = body.to_string();
            // The task-owned client is intentionally aborted at the held
            // boundary. A dropped response is part of this crash schedule.
            let _ = stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
        }
    });
    let worker_repository = repository.clone();
    let worker_store = store.clone();
    let id = server.id.clone();
    let worker = tokio::spawn(async move {
        test_mcp_definition(&worker_repository, worker_store.as_ref(), &id).await
    });
    started_rx.await.unwrap();
    // Read through a separate real connection while /token or the fresh
    // /revoke is still pending, before any observed network failure repair.
    let observer = journal_repository(&database_url).await;
    let durable_before_abort = observer.get(&server.id).await.unwrap();
    observer.pool.close().await;
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    release_tx.send(()).unwrap();
    repository.pool.close().await;
    forget_journal_runtime(&server.id);
    store.fail_put.store(false, Ordering::SeqCst);
    let reopened = journal_repository(&database_url).await;
    let blocked = test_mcp_definition(&reopened, store.as_ref(), &server.id)
        .await
        .unwrap_err();
    assert_eq!(blocked.code, "agent_mcp_oauth_revoke_unconfirmed");
    let disconnected = disconnect_oauth(&reopened, store.as_ref(), &server.id)
        .await
        .unwrap();
    fixture.await.unwrap();
    assert_eq!(
        durable_before_abort.metadata["oauthRotatedRevocationUnconfirmed"],
        true
    );
    assert!(disconnected.local_cleared);
    assert_eq!(disconnected.remote_revoked, Some(false));
}

#[tokio::test]
async fn c1_refresh_intent_survives_interruption_while_token_response_is_pending() {
    tokio::time::timeout(
        Duration::from_secs(10),
        c1_refresh_interrupted_at_remote_boundary(true),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn c1_refresh_intent_survives_interruption_while_fresh_revoke_is_pending() {
    tokio::time::timeout(
        Duration::from_secs(10),
        c1_refresh_interrupted_at_remote_boundary(false),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn c1_failed_pretoken_journal_sends_no_token_request_and_keeps_custody() {
    let directory = tempfile::tempdir().unwrap();
    let database_url = format!(
        "sqlite:{}?mode=rwc",
        directory.path().join("no-post.sqlite3").display()
    );
    let repository = journal_repository(&database_url).await;
    let store = JournalStore::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = journal_server(&repository, &store, &origin).await;
    let mut bundle = store.get("journal-fixture").unwrap().unwrap();
    bundle
        .oauth
        .insert("token_endpoint".into(), format!("{origin}/token"));
    bundle.oauth.insert("expires_at_unix".into(), "0".into());
    store.put("journal-fixture", &bundle).unwrap();
    query("CREATE TRIGGER reject_pretoken_intent BEFORE UPDATE ON agent_mcp_servers WHEN json_extract(NEW.metadata_json, '$.oauthRotatedRevocationUnconfirmed') = 1 BEGIN SELECT RAISE(ABORT, 'synthetic pre-token journal failure'); END")
        .execute(&repository.pool).await.unwrap();
    let failure = test_mcp_definition(&repository, &store, &server.id)
        .await
        .unwrap_err();
    assert_eq!(failure.code, "agent_mcp_storage_failed");
    assert!(
        tokio::time::timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_err()
    );
    assert_eq!(store.get("journal-fixture").unwrap(), Some(bundle));
    assert!(!oauth_lifecycle(&server.id)
        .unconfirmed_revocation
        .load(Ordering::SeqCst));
}
