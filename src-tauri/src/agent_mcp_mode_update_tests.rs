//! Exercise the same generic update path used by the Settings Tauri command.
//! SQLite and a synthetic secret store cover behavior, not OS credential APIs.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;
use std::sync::atomic::{AtomicBool, AtomicUsize};

#[derive(Default)]
struct FixtureStore {
    bundles: Mutex<BTreeMap<String, McpSecretBundle>>,
    writes: AtomicUsize,
    deletes: AtomicUsize,
    fail_get: AtomicBool,
}

impl McpSecretStore for FixtureStore {
    fn put(&self, reference: &str, bundle: &McpSecretBundle) -> Result<(), AgentMcpError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.bundles
            .lock()
            .unwrap()
            .insert(reference.into(), bundle.clone());
        Ok(())
    }
    fn get(&self, reference: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        if self.fail_get.load(Ordering::SeqCst) {
            return Err(AgentMcpError::SecureStorage);
        }
        Ok(self.bundles.lock().unwrap().get(reference).cloned())
    }
    fn delete(&self, reference: &str) -> Result<(), AgentMcpError> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.bundles.lock().unwrap().remove(reference);
        Ok(())
    }
}

async fn repository() -> AgentMcpRepository {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let repository = AgentMcpRepository::new(pool);
    repository.ensure_schema().await.unwrap();
    repository
}

async fn disconnected_oauth(repository: &AgentMcpRepository) -> McpServerDefinition {
    let mut server = McpServerDefinition::new("mode fixture", McpTransport::StreamableHttp);
    server.url = Some("https://example.test/mcp".into());
    server.enabled = false;
    server.metadata = json!({"auth": "oauth", "oauthConnected": false});
    repository.create(&server).await.unwrap();
    server
}

fn settings_input(
    server: &McpServerDefinition,
    metadata: Value,
    enabled: bool,
) -> AgentMcpServerInput {
    serde_json::from_value(json!({
        "id": server.id,
        "name": server.name,
        "enabled": enabled,
        "transport": server.transport,
        "command": server.command,
        "args": server.args,
        "url": server.url,
        "metadata": metadata,
        "toolVisibility": server.tool_visibility,
        "safety": server.safety
    }))
    .unwrap()
}

fn header_bundle() -> McpSecretBundle {
    let mut bundle = McpSecretBundle::default();
    bundle.headers.insert(
        "Authorization".into(),
        "Bearer synthetic-header-fixture".into(),
    );
    bundle
}

#[tokio::test]
async fn oauth_settings_refuse_manual_authorization_without_changing_active_credentials() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.secret_ref = Some("oauth-fixture".into());
    server.metadata["oauthConnected"] = json!(true);
    repository.replace(&server).await.unwrap();
    let mut original = McpSecretBundle::default();
    original
        .oauth
        .insert("access_token".into(), "synthetic-account-b".into());
    original
        .headers
        .insert("x-fixture".into(), "retained".into());
    store.put("oauth-fixture", &original).unwrap();
    for header in ["Authorization", "aUtHoRiZaTiOn"] {
        let mut input = settings_input(&server, json!({"auth":"oauth"}), false);
        let mut replacement = original.clone();
        replacement.oauth.clear();
        replacement
            .headers
            .insert(header.into(), "Bearer synthetic-account-a".into());
        input.secrets = Some(replacement);
        assert!(update_mcp_definition(&repository, &store, input)
            .await
            .is_err());
        assert_eq!(store.get("oauth-fixture").unwrap(), Some(original.clone()));
        assert_eq!(
            repository.get(&server.id).await.unwrap().metadata,
            server.metadata
        );
    }
}

#[tokio::test]
async fn settings_can_remove_oauth_after_complete_teardown_and_enable_custom_headers() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let server = disconnected_oauth(&repository).await;
    let mut input = settings_input(&server, json!({}), true);
    input.secrets = Some(header_bundle());
    let saved = update_mcp_definition(&repository, &store, input)
        .await
        .unwrap();
    assert!(saved.enabled);
    assert!(saved.metadata.get("auth").is_none());
    assert_ne!(saved.metadata.get("oauthConnected"), Some(&json!(true)));
    let bundle = store
        .get(saved.secret_ref.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert!(bundle.oauth.is_empty());
    assert_eq!(bundle.headers, header_bundle().headers);
    assert!(repository.get(&server.id).await.unwrap().enabled);
}

#[tokio::test]
async fn cleared_oauth_bundle_can_keep_unrelated_headers_when_switching_endpoint_and_mode() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.secret_ref = Some("cleared-oauth-fixture".into());
    repository.replace(&server).await.unwrap();
    let original = header_bundle();
    store.put("cleared-oauth-fixture", &original).unwrap();
    let mut input = settings_input(&server, json!({}), true);
    input.url = Some("https://other.example.test/mcp".into());
    let saved = update_mcp_definition(&repository, &store, input)
        .await
        .unwrap();
    assert!(saved.metadata.get("auth").is_none());
    assert!(saved.enabled);
    assert_eq!(saved.url.as_deref(), Some("https://other.example.test/mcp"));
    assert_eq!(store.get("cleared-oauth-fixture").unwrap(), Some(original));
}

#[tokio::test]
async fn unchecking_oauth_does_not_teardown_or_hide_a_live_grant() {
    for enabled in [false, true] {
        let repository = repository().await;
        let store = FixtureStore::default();
        let mut server = disconnected_oauth(&repository).await;
        server.enabled = true;
        server.secret_ref = Some("live-oauth-fixture".into());
        server.metadata["oauthConnected"] = json!(true);
        repository.replace(&server).await.unwrap();
        let mut original = header_bundle();
        original
            .oauth
            .insert("access_token".into(), "synthetic-live-grant".into());
        store.put("live-oauth-fixture", &original).unwrap();
        let writes = store.writes.load(Ordering::SeqCst);
        let input = settings_input(&server, json!({}), enabled);
        assert!(update_mcp_definition(&repository, &store, input)
            .await
            .is_err());
        assert_eq!(store.get("live-oauth-fixture").unwrap(), Some(original));
        let persisted = repository.get(&server.id).await.unwrap();
        assert_eq!(persisted.metadata, server.metadata);
        assert_eq!(persisted.enabled, server.enabled);
        assert_eq!(store.writes.load(Ordering::SeqCst), writes);
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn mode_removal_rejects_all_unresolved_custody_and_remote_warning_markers() {
    for (key, value) in [
        ("oauthConnected", json!(true)),
        ("oauthCleanupPending", json!(true)),
        ("oauthPendingSecretRef", json!("pending-fixture")),
        ("oauthRetiredSecretRef", json!("retired-fixture")),
        ("oauthRemoteRevocationFailed", json!(true)),
        ("oauthRotatedRevocationUnconfirmed", json!(true)),
    ] {
        let repository = repository().await;
        let store = FixtureStore::default();
        let mut server = disconnected_oauth(&repository).await;
        server.metadata[key] = value;
        repository.replace(&server).await.unwrap();
        let input = settings_input(&server, json!({}), false);
        assert!(
            update_mcp_definition(&repository, &store, input)
                .await
                .is_err(),
            "unresolved marker: {key}"
        );
        assert_eq!(
            repository.get(&server.id).await.unwrap().metadata,
            server.metadata
        );
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn an_oauth_bundle_blocks_mode_removal_even_if_connected_metadata_is_false() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.secret_ref = Some("unconfirmed-oauth-fixture".into());
    repository.replace(&server).await.unwrap();
    let mut original = McpSecretBundle::default();
    original
        .oauth
        .insert("refresh_token".into(), "synthetic-unconfirmed-grant".into());
    store.put("unconfirmed-oauth-fixture", &original).unwrap();
    let input = settings_input(&server, json!({}), true);
    assert!(update_mcp_definition(&repository, &store, input)
        .await
        .is_err());
    assert_eq!(
        store.get("unconfirmed-oauth-fixture").unwrap(),
        Some(original)
    );
    assert_eq!(
        repository.get(&server.id).await.unwrap().metadata,
        server.metadata
    );
}

#[tokio::test]
async fn unavailable_vault_cannot_be_assumed_to_contain_no_oauth() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.secret_ref = Some("unreadable-oauth-fixture".into());
    repository.replace(&server).await.unwrap();
    store.fail_get.store(true, Ordering::SeqCst);
    let input = settings_input(&server, json!({}), true);
    assert!(update_mcp_definition(&repository, &store, input)
        .await
        .is_err());
    assert_eq!(
        repository.get(&server.id).await.unwrap().metadata,
        server.metadata
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn official_vercel_preset_cannot_drop_oauth_or_its_identity() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.url = Some("https://mcp.vercel.com".into());
    server.metadata["preset"] = json!("vercel");
    repository.replace(&server).await.unwrap();
    let input = settings_input(&server, json!({}), true);
    assert!(update_mcp_definition(&repository, &store, input)
        .await
        .is_err());
    assert_eq!(
        repository.get(&server.id).await.unwrap().metadata,
        server.metadata
    );
    let input = settings_input(&server, json!({"auth": "oauth"}), false);
    let saved = update_mcp_definition(&repository, &store, input)
        .await
        .unwrap();
    assert_eq!(saved.metadata["preset"], "vercel");
    assert_eq!(saved.metadata["auth"], "oauth");
}

#[tokio::test]
async fn mode_removal_cannot_invent_a_connected_grant_through_metadata() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let server = disconnected_oauth(&repository).await;
    let mut input = settings_input(
        &server,
        json!({"oauthConnected": true, "oauthRevision": "forged-fixture", "oauthGrantRevision": "forged-grant-fixture"}),
        true,
    );
    input.secrets = Some(header_bundle());
    let saved = update_mcp_definition(&repository, &store, input)
        .await
        .unwrap();
    assert!(saved.metadata.get("auth").is_none());
    assert_ne!(saved.metadata.get("oauthConnected"), Some(&json!(true)));
    assert!(saved.metadata.get("oauthRevision").is_none());
    assert!(saved.metadata.get("oauthGrantRevision").is_none());
}

#[tokio::test]
async fn ordinary_oauth_disable_still_uses_full_disconnect_and_keeps_mode() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.secret_ref = Some("disconnect-fixture".into());
    server.enabled = true;
    server.metadata["oauthConnected"] = json!(true);
    repository.replace(&server).await.unwrap();
    let mut original = header_bundle();
    original
        .oauth
        .insert("access_token".into(), "synthetic-disconnect-grant".into());
    store.put("disconnect-fixture", &original).unwrap();
    let input = settings_input(&server, server.metadata.clone(), false);
    let saved = update_mcp_definition(&repository, &store, input)
        .await
        .unwrap();
    assert!(!saved.enabled);
    assert_eq!(saved.metadata["auth"], "oauth");
    assert_eq!(saved.metadata["oauthConnected"], false);
    let cleared = store
        .get(saved.secret_ref.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert!(cleared.oauth.is_empty());
    assert_eq!(cleared.headers, original.headers);
}

#[tokio::test]
async fn saving_a_disabled_name_keeps_remote_revocation_diagnostic() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.metadata["oauthRemoteRevocationFailed"] = json!(true);
    repository.replace(&server).await.unwrap();
    let mut input = settings_input(&server, server.metadata.clone(), false);
    input.name = "renamed fixture".into();
    let saved = update_mcp_definition(&repository, &store, input)
        .await
        .unwrap();
    assert_eq!(saved.name, "renamed fixture");
    assert_eq!(saved.metadata["oauthRemoteRevocationFailed"], true);
    let diagnostic = saved.diagnostic.unwrap();
    assert_eq!(diagnostic.action, "revoke_at_provider");
    assert_eq!(diagnostic.phase, "disconnect");
    assert_eq!(diagnostic.server, saved.name);
}

#[tokio::test]
async fn settings_preserves_host_history_and_attempts_against_forged_replacements() {
    let repository = repository().await;
    let store = FixtureStore::default();
    let mut server = disconnected_oauth(&repository).await;
    server.metadata["oauthGrantRevision"] = json!("host-fixture-grant-revision");
    server.metadata["oauthRevocationHistory"] = json!([{
        "id": "historical-fixture-obligation",
        "affectedSecretRef": "historical-fixture-reference",
        "affectedRevision": "historical-fixture-revision",
        "reason": "rotation_unconfirmed"
    }]);
    server.metadata["oauthConnectAttempts"] = json!([{
        "id": "host-fixture-attempt",
        "secretRef": "reserved-fixture-reference",
        "phase": "authorizing"
    }]);
    repository.replace(&server).await.unwrap();
    let mut input = settings_input(
        &server,
        json!({"auth":"oauth", "oauthRevocationHistory":[], "oauthConnectAttempts":[], "oauthGrantRevision":"forged-fixture-grant-revision"}),
        false,
    );
    input.enabled = None;
    input.name = "renamed history fixture".into();
    let saved = update_mcp_definition(&repository, &store, input)
        .await
        .unwrap();
    assert_eq!(saved.name, "renamed history fixture");
    for key in [
        "oauthRevocationHistory",
        "oauthConnectAttempts",
        "oauthGrantRevision",
    ] {
        assert_eq!(saved.metadata[key], server.metadata[key]);
    }
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    assert_eq!(store.deletes.load(Ordering::SeqCst), 0);

    let empty = disconnected_oauth(&repository).await;
    let mut forged = settings_input(&empty, saved.metadata.clone(), false);
    forged.enabled = None;
    forged.name = "forged history fixture".into();
    let saved = update_mcp_definition(&repository, &store, forged)
        .await
        .unwrap();
    assert!(saved.metadata.get("oauthRevocationHistory").is_none());
    assert!(saved.metadata.get("oauthConnectAttempts").is_none());
    assert!(saved.metadata.get("oauthGrantRevision").is_none());
}

#[tokio::test]
async fn settings_cannot_remove_oauth_with_historical_or_pending_obligations() {
    for (key, value, expected_code) in [
        (
            "oauthRevocationHistory",
            Value::Null,
            "agent_mcp_oauth_revoke_unconfirmed",
        ),
        (
            "oauthRevocationHistory",
            json!([{
                "id":"historical-fixture",
                "affectedSecretRef":"old-fixture-reference",
                "affectedRevision":null,
                "reason":"remote_revocation_failed"
            }]),
            "agent_mcp_oauth_revoke_unconfirmed",
        ),
        (
            "oauthConnectAttempts",
            Value::Null,
            "agent_mcp_oauth_cleanup_pending",
        ),
        (
            "oauthConnectAttempts",
            json!([{"id":"pending-fixture", "phase":"exchange_pending"}]),
            "agent_mcp_oauth_cleanup_pending",
        ),
    ] {
        let repository = repository().await;
        let store = FixtureStore::default();
        let mut server = disconnected_oauth(&repository).await;
        server.metadata[key] = value;
        repository.replace(&server).await.unwrap();
        let mut input = settings_input(&server, json!({}), true);
        input.secrets = Some(header_bundle());
        let error = update_mcp_definition(&repository, &store, input)
            .await
            .unwrap_err();
        assert_eq!(error.code, expected_code);
        let persisted = repository.get(&server.id).await.unwrap();
        assert_eq!(persisted.metadata, server.metadata);
        assert_eq!(persisted.secret_ref, server.secret_ref);
        assert!(!persisted.enabled);
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
        assert_eq!(store.deletes.load(Ordering::SeqCst), 0);
    }
}
