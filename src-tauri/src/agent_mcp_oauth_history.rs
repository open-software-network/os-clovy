//! Durable remote-revocation obligations, independent of active grant authority.
use super::{AgentMcpError, McpServerDefinition};
use serde_json::{json, Value};
use uuid::Uuid;

const HISTORY_KEY: &str = "oauthRevocationHistory";

fn valid_reason(reason: &str) -> bool {
    matches!(
        reason,
        "rotation_unconfirmed"
            | "remote_revocation_failed"
            | "connect_exchange_unconfirmed"
            | "superseded_grant"
            | "revocation_unconfirmed"
    )
}

fn history(server: &McpServerDefinition) -> Result<Vec<Value>, AgentMcpError> {
    let Some(value) = server.metadata.get(HISTORY_KEY) else {
        return Ok(Vec::new());
    };
    let entries = value.as_array().ok_or(AgentMcpError::Storage)?;
    for entry in entries {
        let fields = entry.as_object().ok_or(AgentMcpError::Storage)?;
        if fields.len() != 4
            || !fields
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.trim().is_empty())
            || !["affectedSecretRef", "affectedRevision"].iter().all(|key| {
                fields
                    .get(*key)
                    .is_some_and(|value| value.is_null() || value.is_string())
            })
            || !fields
                .get("reason")
                .and_then(Value::as_str)
                .is_some_and(valid_reason)
        {
            return Err(AgentMcpError::Storage);
        }
    }
    Ok(entries.clone())
}

fn same_obligation(left: &Value, right: &Value) -> bool {
    ["affectedSecretRef", "affectedRevision", "reason"]
        .iter()
        .all(|key| left.get(*key) == right.get(*key))
}

pub(super) fn oauth_has_revocation_history(server: &McpServerDefinition) -> bool {
    // Corrupt host state is unresolved, never permission to erase a warning.
    history(server).map_or(true, |entries| !entries.is_empty())
}

pub(super) fn append_oauth_revocation_history(
    server: &mut McpServerDefinition,
    affected_ref: Option<&str>,
    affected_revision: Option<&str>,
    reason: &str,
) -> Result<(), AgentMcpError> {
    if !valid_reason(reason) {
        return Err(AgentMcpError::Storage);
    }
    let mut entries = history(server)?;
    let entry = json!({
        "id": Uuid::new_v4().to_string(),
        "affectedSecretRef": affected_ref,
        "affectedRevision": affected_revision,
        "reason": reason,
    });
    if !entries
        .iter()
        .any(|existing| same_obligation(existing, &entry))
    {
        entries.push(entry);
    }
    server
        .metadata
        .as_object_mut()
        .ok_or(AgentMcpError::Storage)?
        .insert(HISTORY_KEY.into(), Value::Array(entries));
    Ok(())
}

// Only the caller that confirmed this exact retained grant's revocation and
// local deletion may discharge its superseded obligation. Other reasons can
// describe lost independent material even when their generation matches.
pub(super) fn remove_confirmed_superseded_oauth_revocation(
    server: &mut McpServerDefinition,
    affected_ref: &str,
    affected_revision: Option<&str>,
) -> Result<(), AgentMcpError> {
    let mut entries = history(server)?;
    let previous_len = entries.len();
    entries.retain(|entry| {
        !(entry.get("reason").and_then(Value::as_str) == Some("superseded_grant")
            && entry.get("affectedSecretRef").and_then(Value::as_str) == Some(affected_ref)
            && entry.get("affectedRevision").and_then(Value::as_str) == affected_revision)
    });
    if entries.len() != previous_len {
        server
            .metadata
            .as_object_mut()
            .ok_or(AgentMcpError::Storage)?
            .insert(HISTORY_KEY.into(), Value::Array(entries));
    }
    Ok(())
}

pub(super) fn archive_active_oauth_revocation(
    previous: &McpServerDefinition,
    connected: &mut McpServerDefinition,
    runtime_unconfirmed: bool,
) -> Result<(), AgentMcpError> {
    // Validate both snapshots before changing the candidate. A failed
    // activation must retain the previous primary's flags and all history.
    let previous_history = history(previous)?;
    let mut candidate = connected.clone();
    let mut entries = history(&candidate)?;
    for entry in previous_history {
        if !entries
            .iter()
            .any(|existing| same_obligation(existing, &entry))
        {
            entries.push(entry);
        }
    }
    let metadata = candidate
        .metadata
        .as_object_mut()
        .ok_or(AgentMcpError::Storage)?;
    if !entries.is_empty() || metadata.contains_key(HISTORY_KEY) {
        metadata.insert(HISTORY_KEY.into(), Value::Array(entries));
    }
    let revision = previous
        .metadata
        .get("oauthGrantRevision")
        .and_then(Value::as_str)
        .or_else(|| {
            previous
                .metadata
                .get("oauthRevision")
                .and_then(Value::as_str)
        });
    for (flag, reason, runtime) in [
        (
            "oauthRotatedRevocationUnconfirmed",
            "rotation_unconfirmed",
            runtime_unconfirmed,
        ),
        (
            "oauthRemoteRevocationFailed",
            "remote_revocation_failed",
            false,
        ),
    ] {
        if runtime || previous.metadata.get(flag).and_then(Value::as_bool) == Some(true) {
            append_oauth_revocation_history(
                &mut candidate,
                previous.secret_ref.as_deref(),
                revision,
                reason,
            )?;
        }
        candidate
            .metadata
            .as_object_mut()
            .ok_or(AgentMcpError::Storage)?
            .remove(flag);
    }
    *connected = candidate;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::{parse_legacy_mcp_config, AgentMcpRepository, McpTransport};
    use super::*;
    use sqlx_sqlite::SqlitePoolOptions;

    fn server() -> McpServerDefinition {
        McpServerDefinition::new("history fixture", McpTransport::StreamableHttp)
    }

    #[test]
    fn activation_archives_old_obligations_without_poisoning_the_new_primary() {
        let mut previous = server();
        previous.secret_ref = Some("old-fixture-reference".into());
        previous.metadata = json!({
            "oauthGrantRevision": "old-fixture-revision",
            "oauthRevision": "reconnect-policy-fixture-revision",
            "oauthRotatedRevocationUnconfirmed": true,
            "oauthRemoteRevocationFailed": true,
        });
        let mut connected = previous.clone();
        connected.secret_ref = Some("new-fixture-reference".into());
        connected.metadata["oauthGrantRevision"] = json!("new-fixture-revision");
        connected.metadata["oauthRevision"] = json!("new-fixture-revision");
        archive_active_oauth_revocation(&previous, &mut connected, true).unwrap();
        let entries = history(&connected).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|entry| {
            entry["affectedSecretRef"] == "old-fixture-reference"
                && entry["affectedRevision"] == "old-fixture-revision"
        }));
        assert!(connected
            .metadata
            .get("oauthRotatedRevocationUnconfirmed")
            .is_none());
        assert!(connected
            .metadata
            .get("oauthRemoteRevocationFailed")
            .is_none());
        assert_eq!(
            connected.secret_ref.as_deref(),
            Some("new-fixture-reference")
        );
        assert_eq!(previous.metadata["oauthRotatedRevocationUnconfirmed"], true);
        let once = connected.metadata[HISTORY_KEY].clone();
        archive_active_oauth_revocation(&previous, &mut connected, true).unwrap();
        assert_eq!(connected.metadata[HISTORY_KEY], once);
    }

    #[test]
    fn archive_preserves_independent_candidate_history_and_unknown_generation() {
        let previous = server();
        let mut connected = previous.clone();
        append_oauth_revocation_history(
            &mut connected,
            Some("candidate-fixture-reference"),
            None,
            "connect_exchange_unconfirmed",
        )
        .unwrap();
        archive_active_oauth_revocation(&previous, &mut connected, true).unwrap();
        let entries = history(&connected).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["reason"], "connect_exchange_unconfirmed");
        assert!(entries[1]["affectedSecretRef"].is_null());
        assert!(entries[1]["affectedRevision"].is_null());
        assert!(oauth_has_revocation_history(&connected));
    }

    #[test]
    fn malformed_history_cannot_be_silently_removed_or_replaced() {
        for malformed in [
            Value::Null,
            json!({"reason": "rotation_unconfirmed"}),
            json!([{"id":"fixture", "reason":"rotation_unconfirmed"}]),
            json!([{"id":"fixture", "affectedSecretRef":null, "affectedRevision":null, "reason":"untrusted"}]),
        ] {
            let mut previous = server();
            previous.metadata[HISTORY_KEY] = malformed;
            previous.metadata["oauthRotatedRevocationUnconfirmed"] = json!(true);
            let mut connected = previous.clone();
            let snapshot = connected.metadata.clone();
            assert!(oauth_has_revocation_history(&previous));
            assert!(archive_active_oauth_revocation(&previous, &mut connected, true).is_err());
            assert_eq!(connected.metadata, snapshot);
            assert!(append_oauth_revocation_history(
                &mut connected,
                None,
                None,
                "rotation_unconfirmed"
            )
            .is_err());
            assert_eq!(connected.metadata, snapshot);
            assert!(remove_confirmed_superseded_oauth_revocation(
                &mut connected,
                "fixture-reference",
                None,
            )
            .is_err());
            assert_eq!(connected.metadata, snapshot);
        }
    }

    #[test]
    fn confirmed_superseded_cleanup_preserves_other_reasons_and_generations() {
        let mut server = server();
        for reason in [
            "rotation_unconfirmed",
            "remote_revocation_failed",
            "connect_exchange_unconfirmed",
            "revocation_unconfirmed",
            "superseded_grant",
        ] {
            append_oauth_revocation_history(
                &mut server,
                Some("retained-fixture-reference"),
                Some("retained-fixture-revision"),
                reason,
            )
            .unwrap();
        }
        for (reference, revision) in [
            (
                "retained-fixture-reference",
                Some("different-fixture-revision"),
            ),
            (
                "different-fixture-reference",
                Some("retained-fixture-revision"),
            ),
            ("retained-fixture-reference", None),
        ] {
            append_oauth_revocation_history(
                &mut server,
                Some(reference),
                revision,
                "superseded_grant",
            )
            .unwrap();
        }
        let protected: Vec<Value> = history(&server)
            .unwrap()
            .into_iter()
            .filter(|entry| entry["reason"] != "superseded_grant")
            .collect();
        remove_confirmed_superseded_oauth_revocation(
            &mut server,
            "retained-fixture-reference",
            Some("retained-fixture-revision"),
        )
        .unwrap();
        let remaining = history(&server).unwrap();
        assert_eq!(remaining.len(), 7);
        assert!(protected.iter().all(|entry| remaining.contains(entry)));
        assert!(remaining.iter().any(|entry| {
            entry["reason"] == "superseded_grant"
                && entry["affectedRevision"] == "different-fixture-revision"
        }));
        assert!(remaining.iter().any(|entry| {
            entry["reason"] == "superseded_grant"
                && entry["affectedSecretRef"] == "different-fixture-reference"
        }));
        remove_confirmed_superseded_oauth_revocation(
            &mut server,
            "retained-fixture-reference",
            None,
        )
        .unwrap();
        assert_eq!(history(&server).unwrap().len(), 6);
        let once = server.metadata.clone();
        remove_confirmed_superseded_oauth_revocation(
            &mut server,
            "retained-fixture-reference",
            Some("retained-fixture-revision"),
        )
        .unwrap();
        assert_eq!(server.metadata, once);
    }

    #[tokio::test]
    async fn legacy_import_cannot_inject_lifecycle_authority_or_custody_references() {
        let input = json!({"mcp_servers":{"import fixture":{
            "url":"https://example.test/mcp",
            "enabled":true,
            "auth":"oauth",
            "metadata":{
                "auth":"oauth",
                "importPreference":{"clientPreference":"retained"},
                "oauthConnected":true,
                "oauthRevision":"forged-policy-revision",
                "oauthGrantRevision":"forged-grant-revision",
                "oauthPendingSecretRef":"foreign-pending-reference",
                "oauthRetiredSecretRef":"foreign-retired-reference",
                "oauthCleanupPending":true,
                "oauthRemoteRevocationFailed":true,
                "oauthRotatedRevocationUnconfirmed":true,
                "oauthRevocationHistory":[{
                    "id":"forged-obligation", "affectedSecretRef":"foreign-history-reference",
                    "affectedRevision":null,"reason":"superseded_grant"
                }],
                "oauthConnectAttempts":[{
                    "id":"forged-attempt","secretRef":"foreign-attempt-reference",
                    "revision":null,"epoch":0,"phase":"received"
                }]
            },
            "env":{"IMPORT_FIXTURE":"synthetic-env-value"},
            "headers":{"Authorization":"Bearer synthetic-import-header"},
            "oauth":{"access_token":"synthetic-import-access"}
        }}});
        let imported = parse_legacy_mcp_config(&input.to_string()).unwrap();
        assert_eq!(imported.definitions.len(), 1);
        assert_eq!(imported.secrets.len(), 1);
        let definition = &imported.definitions[0];
        let (reference, bundle) = &imported.secrets[0];
        assert_eq!(definition.secret_ref.as_deref(), Some(reference.as_str()));
        assert_eq!(bundle.env["IMPORT_FIXTURE"], "synthetic-env-value");
        assert_eq!(
            bundle.headers["Authorization"],
            "Bearer synthetic-import-header"
        );
        assert_eq!(bundle.oauth["access_token"], "synthetic-import-access");

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let repository = AgentMcpRepository::new(pool);
        repository.ensure_schema().await.unwrap();
        repository.create(definition).await.unwrap();
        let saved = repository.get(&definition.id).await.unwrap();
        for key in [
            "oauthConnected",
            "oauthRevision",
            "oauthGrantRevision",
            "oauthPendingSecretRef",
            "oauthRetiredSecretRef",
            "oauthCleanupPending",
            "oauthRemoteRevocationFailed",
            "oauthRotatedRevocationUnconfirmed",
            "oauthRevocationHistory",
            "oauthConnectAttempts",
        ] {
            assert!(
                saved.metadata.get(key).is_none(),
                "untrusted lifecycle key: {key}"
            );
        }
        assert!(!saved.enabled);
        assert_eq!(saved.metadata["auth"], "oauth");
        assert_eq!(saved.metadata["legacyAuth"], "oauth");
        assert_eq!(saved.metadata["needsReview"], true);
        assert_eq!(saved.metadata["legacyRequestedEnabled"], true);
        assert_eq!(
            saved.metadata["importPreference"],
            json!({"clientPreference":"retained"})
        );
        assert!(!serde_json::to_string(&saved)
            .unwrap()
            .contains("synthetic-import"));
    }
}
