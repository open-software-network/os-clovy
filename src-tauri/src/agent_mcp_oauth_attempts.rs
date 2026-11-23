//! Operation-owned custody between code exchange and primary activation.
//! Metadata contains references and outcomes only; OAuth material stays in the store.
use super::partition::CandidatePartition;
use super::volatile_custody::{
    retain_candidate, update_volatile_candidate, volatile_candidate, ConnectAttempt, ConnectPhase,
};
use super::*;
#[derive(Clone)]
pub(super) struct OAuthAttemptContext {
    repository: AgentMcpRepository,
    server_id: String,
    attempt: ConnectAttempt,
    lifecycle: Arc<OAuthLifecycle>,
    admitted: Arc<std::sync::atomic::AtomicBool>,
    definitive_refusal: Arc<std::sync::atomic::AtomicBool>,
    partition: Arc<CandidatePartition>,
}
static LIVE_CONNECT_ATTEMPTS: OnceLock<Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>> =
    OnceLock::new();

tokio::task_local! { static OAUTH_CONNECT_ATTEMPT: OAuthAttemptContext; }
pub(super) struct OAuthAttemptOwner(OAuthAttemptContext);
impl Drop for OAuthAttemptOwner {
    fn drop(&mut self) {
        if let Ok(mut live) = LIVE_CONNECT_ATTEMPTS.get_or_init(Default::default).lock() {
            live.remove(&self.0.attempt.id);
        }
    }
}
fn attempts(server: &McpServerDefinition) -> Result<Vec<ConnectAttempt>, AgentMcpError> {
    let Some(value) = server.metadata.get("oauthConnectAttempts") else {
        return Ok(Vec::new());
    };
    let parsed: Vec<ConnectAttempt> =
        serde_json::from_value(value.clone()).map_err(|_| AgentMcpError::Storage)?;
    let mut ids = BTreeSet::new();
    let mut refs = BTreeSet::new();
    if parsed.iter().any(|entry| {
        entry.id.is_empty()
            || entry.secret_ref.is_empty()
            || !ids.insert(&entry.id)
            || !refs.insert(&entry.secret_ref)
            || server.secret_ref.as_deref() == Some(&entry.secret_ref)
    }) {
        return Err(AgentMcpError::Storage);
    }
    Ok(parsed)
}
fn set_attempts(
    server: &mut McpServerDefinition,
    entries: &[ConnectAttempt],
) -> Result<(), AgentMcpError> {
    let metadata = server
        .metadata
        .as_object_mut()
        .ok_or(AgentMcpError::Storage)?;
    if entries.is_empty() {
        metadata.remove("oauthConnectAttempts");
    } else {
        metadata.insert(
            "oauthConnectAttempts".into(),
            serde_json::to_value(entries).map_err(|_| AgentMcpError::Storage)?,
        );
    }
    Ok(())
}
async fn write_attempts(
    repository: &AgentMcpRepository,
    before: &McpServerDefinition,
    entries: &[ConnectAttempt],
) -> Result<McpServerDefinition, AgentMcpError> {
    let mut next = before.clone();
    set_attempts(&mut next, entries)?;
    write_oauth_refresh_metadata(repository, before, &next.metadata).await?;
    Ok(next)
}
pub(super) async fn reserve_oauth_attempt(
    repository: &AgentMcpRepository,
    existing: &McpServerDefinition,
    lifecycle: Arc<OAuthLifecycle>,
    epoch: u64,
) -> Result<(OAuthAttemptContext, OAuthAttemptOwner), AgentMcpError> {
    let mut entries = attempts(existing)?;
    let attempt = ConnectAttempt {
        id: Uuid::new_v4().to_string(),
        secret_ref: Uuid::new_v4().to_string(),
        revision: existing
            .metadata
            .get("oauthRevision")
            .and_then(Value::as_str)
            .map(str::to_owned),
        epoch,
        phase: ConnectPhase::Authorizing,
        superseded: false,
    };
    entries.push(attempt.clone());
    write_attempts(repository, existing, &entries).await?;
    let context = OAuthAttemptContext {
        repository: repository.clone(),
        server_id: existing.id.clone(),
        attempt,
        lifecycle,
        admitted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        definitive_refusal: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        partition: Arc::new(CandidatePartition::new(&repository.pool).await),
    };
    LIVE_CONNECT_ATTEMPTS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| AgentMcpError::Storage)?
        .insert(context.attempt.id.clone(), context.admitted.clone());
    Ok((context.clone(), OAuthAttemptOwner(context)))
}
pub(super) async fn in_oauth_attempt<F: Future>(
    context: OAuthAttemptContext,
    future: F,
) -> F::Output {
    OAUTH_CONNECT_ATTEMPT.scope(context, future).await
}
pub(super) fn oauth_exchange_was_admitted(context: &OAuthAttemptContext) -> bool {
    context.admitted.load(Ordering::SeqCst)
}
pub(super) fn record_oauth_exchange_refusal(status: u16, error: &AgentMcpError) {
    let definitive = (400..500).contains(&status)
        && matches!(
            error,
            AgentMcpError::OauthProviderRejected(
                "invalid_request"
                    | "invalid_client"
                    | "unauthorized_client"
                    | "access_denied"
                    | "unsupported_grant_type"
                    | "invalid_scope"
                    | "invalid_grant"
            )
        );
    let _ = OAUTH_CONNECT_ATTEMPT.try_with(|context| {
        context
            .definitive_refusal
            .store(definitive, Ordering::SeqCst)
    });
}
pub(super) fn oauth_exchange_definitively_refused(context: &OAuthAttemptContext) -> bool {
    context.definitive_refusal.load(Ordering::SeqCst)
}
pub(super) fn oauth_has_pending_connect_attempts(server: &McpServerDefinition) -> bool {
    attempts(server)
        .map(|entries| entries.iter().any(|entry| !entry.superseded))
        .unwrap_or(true)
}
// The protocol helper has no host lifecycle when used outside managed Connect.
pub(super) async fn prepare_oauth_code_exchange() -> Result<(), AgentMcpError> {
    let Some(context) = OAUTH_CONNECT_ATTEMPT.try_with(Clone::clone).ok() else {
        return Ok(());
    };
    let _gate = context.lifecycle.gate.lock().await;
    check_oauth_epoch(&context.lifecycle, context.attempt.epoch)?;
    let current = context.repository.get(&context.server_id).await?;
    let mut entries = attempts(&current)?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == context.attempt.id)
        .ok_or(AgentMcpError::ConnectionChanged)?;
    if entry.phase != ConnectPhase::Authorizing {
        return Err(AgentMcpError::ConnectionChanged);
    }
    entry.phase = ConnectPhase::ExchangePending;
    write_attempts(&context.repository, &current, &entries).await?;
    check_oauth_epoch(&context.lifecycle, context.attempt.epoch)?;
    Ok(())
}
pub(super) fn admit_oauth_code_exchange() -> Result<(), AgentMcpError> {
    OAUTH_CONNECT_ATTEMPT
        .try_with(|context| {
            let _admission = context
                .lifecycle
                .admission
                .lock()
                .map_err(|_| AgentMcpError::Storage)?;
            check_oauth_epoch(&context.lifecycle, context.attempt.epoch)?;
            context.admitted.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_or(Ok(()))
}
pub(super) fn put_oauth_candidate<S: McpSecretStore>(
    store: &S,
    context: &OAuthAttemptContext,
    bundle: &McpSecretBundle,
) -> Result<(), AgentMcpError> {
    let result = store.put(&context.attempt.secret_ref, bundle);
    if result.is_err() {
        retain_candidate(
            &context.partition,
            &context.server_id,
            &context.attempt,
            bundle,
        );
    }
    result
}
pub(super) fn current_oauth_attempt() -> Option<OAuthAttemptContext> {
    OAUTH_CONNECT_ATTEMPT.try_with(Clone::clone).ok()
}
pub(super) async fn mark_oauth_candidate_received(
    context: &OAuthAttemptContext,
) -> Result<(), AgentMcpError> {
    let _gate = context.lifecycle.gate.lock().await;
    mark_oauth_candidate_received_locked(context).await
}
pub(super) async fn mark_oauth_candidate_received_locked(
    context: &OAuthAttemptContext,
) -> Result<(), AgentMcpError> {
    let current = context.repository.get(&context.server_id).await?;
    let mut entries = attempts(&current)?;
    let entry = entries
        .iter_mut()
        .find(|entry| entry.id == context.attempt.id)
        .ok_or(AgentMcpError::ConnectionChanged)?;
    if entry.superseded
        || !matches!(
            entry.phase,
            ConnectPhase::Authorizing | ConnectPhase::ExchangePending | ConnectPhase::Received
        )
    {
        return Err(AgentMcpError::ConnectionChanged);
    }
    entry.phase = ConnectPhase::Received;
    write_attempts(&context.repository, &current, &entries).await?;
    Ok(())
}
#[derive(Default)]
pub(super) struct AttemptCleanup {
    pub remote_uncertain: bool,
    pub remote_confirmed: bool,
    pub local_pending: bool,
}
fn attempt_live(id: &str) -> bool {
    LIVE_CONNECT_ATTEMPTS
        .get_or_init(Default::default)
        .lock()
        .map(|live| live.contains_key(id))
        .unwrap_or(true)
}
fn attempt_admitted(id: &str) -> bool {
    LIVE_CONNECT_ATTEMPTS
        .get_or_init(Default::default)
        .lock()
        .map(|live| {
            live.get(id)
                .map_or(true, |admitted| admitted.load(Ordering::SeqCst))
        })
        .unwrap_or(true)
}
async fn settle_attempt<S: McpSecretStore>(
    repository: &AgentMcpRepository,
    store: &S,
    server: &mut McpServerDefinition,
    attempt: &ConnectAttempt,
    received: Option<&McpSecretBundle>,
    live: bool,
) -> Result<AttemptCleanup, AgentMcpError> {
    let partition = CandidatePartition::new(&repository.pool).await;
    let retained = volatile_candidate(&partition, &server.id, attempt)?;
    let retained_confirmed = retained.as_ref().is_some_and(|(_, confirmed)| *confirmed);
    let bundle = match received {
        Some(bundle) => Some(bundle.clone()),
        None => match retained {
            Some((bundle, _)) => Some(bundle),
            None => store.get(&attempt.secret_ref)?,
        },
    };
    let mut entries = attempts(server)?;
    let Some(position) = entries.iter().position(|entry| entry.id == attempt.id) else {
        return Ok(AttemptCleanup::default());
    };
    if let Some(bundle) = bundle {
        let confirmed = retained_confirmed
            || attempt.phase == ConnectPhase::RevokedButLocalCleanupPending
            || revoke_oauth_bundle(&bundle.oauth).await == Some(true);
        if confirmed {
            update_volatile_candidate(&partition, &server.id, attempt, false);
        }
        entries[position].phase = if confirmed {
            ConnectPhase::RevokedButLocalCleanupPending
        } else {
            ConnectPhase::RevocationFailed
        };
        *server = write_attempts(repository, server, &entries).await?;
        if !confirmed {
            // A failed initial put may be transient. Retain this exact bundle;
            // never reauthorize or repeat a code/token request.
            if store.put(&attempt.secret_ref, &bundle).is_ok() {
                update_volatile_candidate(&partition, &server.id, attempt, true);
            }
            return Ok(AttemptCleanup {
                remote_uncertain: true,
                remote_confirmed: false,
                local_pending: true,
            });
        }
        update_volatile_candidate(&partition, &server.id, attempt, true);
        if store.delete(&attempt.secret_ref).is_err() {
            return Ok(AttemptCleanup {
                remote_uncertain: false,
                remote_confirmed: true,
                local_pending: true,
            });
        }
        entries.remove(position);
        let before = server.clone();
        let mut next = server.clone();
        if attempt.superseded {
            remove_confirmed_superseded_oauth_revocation(
                &mut next,
                &attempt.secret_ref,
                attempt.revision.as_deref(),
            )?;
        }
        set_attempts(&mut next, &entries)?;
        write_oauth_refresh_metadata(repository, &before, &next.metadata).await?;
        *server = next;
        return Ok(AttemptCleanup {
            remote_confirmed: true,
            ..AttemptCleanup::default()
        });
    }
    if live
        && attempt.phase != ConnectPhase::Authorizing
        && attempt.phase != ConnectPhase::RevokedButLocalCleanupPending
    {
        return Ok(AttemptCleanup {
            remote_uncertain: true,
            remote_confirmed: false,
            local_pending: true,
        });
    }
    if attempt.phase != ConnectPhase::Authorizing
        && attempt.phase != ConnectPhase::RevokedButLocalCleanupPending
    {
        let before = server.clone();
        let mut next = server.clone();
        if !attempt.superseded {
            append_oauth_revocation_history(
                &mut next,
                Some(&attempt.secret_ref),
                attempt.revision.as_deref(),
                "connect_exchange_unconfirmed",
            )?;
        }
        entries.remove(position);
        set_attempts(&mut next, &entries)?;
        write_oauth_refresh_metadata(repository, &before, &next.metadata).await?;
        *server = next;
        return Ok(AttemptCleanup {
            remote_uncertain: true,
            remote_confirmed: false,
            local_pending: false,
        });
    }
    entries.remove(position);
    let before = server.clone();
    let mut next = server.clone();
    if attempt.superseded && attempt.phase == ConnectPhase::RevokedButLocalCleanupPending {
        remove_confirmed_superseded_oauth_revocation(
            &mut next,
            &attempt.secret_ref,
            attempt.revision.as_deref(),
        )?;
    }
    set_attempts(&mut next, &entries)?;
    write_oauth_refresh_metadata(repository, &before, &next.metadata).await?;
    *server = next;
    Ok(AttemptCleanup {
        remote_confirmed: attempt.phase == ConnectPhase::RevokedButLocalCleanupPending,
        ..AttemptCleanup::default()
    })
}
// Old activation journals can already name an issued grant. Migrate their
// custody before any cleanup, rather than treating OAuth as disposable bytes.
pub(super) async fn migrate_legacy_oauth_pending<S: McpSecretStore>(
    repository: &AgentMcpRepository,
    store: &S,
    server: &mut McpServerDefinition,
) -> Result<(), AgentMcpError> {
    let revision = server
        .metadata
        .get("oauthRevision")
        .and_then(Value::as_str)
        .map(str::to_owned);
    migrate_legacy_oauth_pending_with_revision(repository, store, server, revision).await
}
pub(super) async fn migrate_legacy_oauth_pending_with_revision<S: McpSecretStore>(
    repository: &AgentMcpRepository,
    store: &S,
    server: &mut McpServerDefinition,
    revision: Option<String>,
) -> Result<(), AgentMcpError> {
    let Some(value) = server.metadata.get("oauthPendingSecretRef") else {
        return Ok(());
    };
    let reference = value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or(AgentMcpError::Storage)?
        .to_owned();
    if server.secret_ref.as_deref() == Some(&reference) {
        return Err(AgentMcpError::Storage);
    }
    let bundle = store.get(&reference)?;
    let has_grant = bundle.as_ref().is_some_and(|bundle| {
        bundle.oauth.contains_key("access_token") || bundle.oauth.contains_key("refresh_token")
    });
    let before = server.clone();
    let mut next = server.clone();
    if bundle.is_some() && !has_grant {
        // Manual env/header-only staging never issued an OAuth grant.
        store.delete(&reference)?;
    } else {
        let mut entries = attempts(server)?;
        if !entries.iter().any(|entry| entry.secret_ref == reference) {
            entries.push(ConnectAttempt {
                id: Uuid::new_v4().to_string(),
                secret_ref: reference,
                revision: revision.clone(),
                epoch: 0,
                superseded: false,
                phase: if has_grant {
                    ConnectPhase::Received
                } else {
                    ConnectPhase::ExchangePending
                },
            });
        }
        set_attempts(&mut next, &entries)?;
    }
    next.metadata
        .as_object_mut()
        .ok_or(AgentMcpError::Storage)?
        .remove("oauthPendingSecretRef");
    write_oauth_refresh_metadata(repository, &before, &next.metadata).await?;
    *server = next;
    Ok(())
}
// Older activations used one untyped retired pointer for OAuth and manual
// credentials. OAuth grants must become explicit, nonblocking obligations;
// ordinary inventory recovery must never revoke their possible shared family.
pub(super) async fn migrate_legacy_oauth_retired<S: McpSecretStore>(
    repository: &AgentMcpRepository,
    store: &S,
    server: &mut McpServerDefinition,
) -> Result<(), AgentMcpError> {
    let Some(value) = server.metadata.get("oauthRetiredSecretRef") else {
        return Ok(());
    };
    let reference = value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or(AgentMcpError::Storage)?
        .to_owned();
    if server.secret_ref.as_deref() == Some(&reference) {
        return Err(AgentMcpError::Storage);
    }
    let stored = store.get(&reference)?;
    if stored.as_ref().is_some_and(|bundle| {
        !bundle.oauth.contains_key("access_token") && !bundle.oauth.contains_key("refresh_token")
    }) {
        return Ok(()); // Known manual-only cleanup keeps its existing path.
    }
    let mut next = server.clone();
    if stored.is_some() {
        let mut entries = attempts(server)?;
        if let Some(entry) = entries.iter().find(|entry| entry.secret_ref == reference) {
            if !entry.superseded {
                return Err(AgentMcpError::Storage);
            }
        } else {
            entries.push(ConnectAttempt {
                id: Uuid::new_v4().to_string(),
                secret_ref: reference.clone(),
                revision: None,
                epoch: 0,
                phase: ConnectPhase::Received,
                superseded: true,
            });
        }
        append_oauth_revocation_history(&mut next, Some(&reference), None, "superseded_grant")?;
        set_attempts(&mut next, &entries)?;
    } else {
        append_oauth_revocation_history(
            &mut next,
            Some(&reference),
            None,
            "revocation_unconfirmed",
        )?;
    }
    next.metadata
        .as_object_mut()
        .ok_or(AgentMcpError::Storage)?
        .remove("oauthRetiredSecretRef");
    write_oauth_refresh_metadata(repository, server, &next.metadata).await?;
    *server = next;
    Ok(())
}
// Caller owns the lifecycle gate. Live owners are only retired by Disconnect.
pub(super) async fn recover_oauth_attempts<S: McpSecretStore>(
    repository: &AgentMcpRepository,
    store: &S,
    server: &mut McpServerDefinition,
    retire_live: bool,
) -> Result<AttemptCleanup, AgentMcpError> {
    let mut outcome = AttemptCleanup::default();
    for mut attempt in attempts(server)? {
        let live = attempt_live(&attempt.id);
        if (live || attempt.superseded) && !retire_live {
            continue;
        }
        if live && !attempt_admitted(&attempt.id) {
            // The durable pre-wire intent may have committed just before
            // invalidation. A still-live owner proves admission never occurred.
            attempt.phase = ConnectPhase::Authorizing;
        }
        let result = settle_attempt(repository, store, server, &attempt, None, live).await?;
        outcome.remote_uncertain |= result.remote_uncertain;
        outcome.remote_confirmed |= result.remote_confirmed;
        outcome.local_pending |= result.local_pending;
    }
    Ok(outcome)
}
pub(super) async fn compensate_oauth_attempt<S: McpSecretStore>(
    repository: &AgentMcpRepository,
    store: &S,
    context: &OAuthAttemptContext,
    received: Option<&McpSecretBundle>,
    definitive_refusal: bool,
) -> Result<AttemptCleanup, AgentMcpError> {
    let _gate = context.lifecycle.gate.lock().await;
    let mut current = repository.get(&context.server_id).await?;
    let Some(mut attempt) = attempts(&current)?
        .into_iter()
        .find(|entry| entry.id == context.attempt.id)
    else {
        return Ok(AttemptCleanup::default());
    };
    if received.is_none() && (!oauth_exchange_was_admitted(context) || definitive_refusal) {
        attempt.phase = ConnectPhase::Authorizing;
    }
    settle_attempt(repository, store, &mut current, &attempt, received, false).await
}
pub(super) async fn activate_oauth_candidate<S: McpSecretStore>(
    repository: &AgentMcpRepository,
    store: &S,
    existing: &McpServerDefinition,
    context: &OAuthAttemptContext,
) -> Result<McpServerDefinition, crate::domain::types::AppError> {
    check_oauth_epoch(&context.lifecycle, context.attempt.epoch).map_err(app_error)?;
    let current = repository.get(&existing.id).await.map_err(app_error)?;
    check_oauth_epoch(&context.lifecycle, context.attempt.epoch).map_err(app_error)?;
    if custom_mcp_server_fingerprint(&current).map_err(app_error)?
        != custom_mcp_server_fingerprint(existing).map_err(app_error)?
    {
        return Err(app_error(AgentMcpError::OauthReconnectRequired));
    }
    let mut entries = attempts(&current).map_err(app_error)?;
    let position = entries
        .iter()
        .position(|entry| entry.id == context.attempt.id && entry.phase == ConnectPhase::Received)
        .ok_or_else(|| app_error(AgentMcpError::OauthCleanupPending))?;
    if store
        .get(&context.attempt.secret_ref)
        .map_err(app_error)?
        .is_none()
    {
        return Err(app_error(AgentMcpError::OauthCleanupPending));
    }
    let mut connected = current.clone();
    connected.secret_ref = Some(context.attempt.secret_ref.clone());
    connected.enabled = true;
    archive_active_oauth_revocation(
        &current,
        &mut connected,
        context
            .lifecycle
            .unconfirmed_revocation
            .load(Ordering::SeqCst),
    )
    .map_err(app_error)?;
    let metadata = connected
        .metadata
        .as_object_mut()
        .ok_or_else(|| app_error(AgentMcpError::Storage))?;
    for key in ["legacyAuth", "needsReview", "migrationWarning"] {
        metadata.remove(key);
    }
    metadata.insert("auth".into(), json!("oauth"));
    metadata.insert("oauthConnected".into(), json!(true));
    let grant_revision = Uuid::new_v4().to_string();
    metadata.insert("oauthRevision".into(), json!(grant_revision));
    metadata.insert("oauthGrantRevision".into(), json!(grant_revision));
    let previous = match current.secret_ref.as_deref() {
        Some(reference) => Some((
            reference.to_owned(),
            store.get(reference).map_err(app_error)?,
        )),
        None => None,
    };
    let old_oauth = previous.as_ref().is_some_and(|(_, bundle)| {
        bundle.as_ref().map_or(
            current
                .metadata
                .get("oauthConnected")
                .and_then(Value::as_bool)
                == Some(true),
            |bundle| {
                bundle.oauth.contains_key("access_token")
                    || bundle.oauth.contains_key("refresh_token")
            },
        )
    });
    entries.remove(position);
    if let Some((reference, _)) = previous.as_ref() {
        if old_oauth {
            let revision = current
                .metadata
                .get("oauthGrantRevision")
                .or_else(|| current.metadata.get("oauthRevision"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            append_oauth_revocation_history(
                &mut connected,
                Some(reference),
                revision.as_deref(),
                "superseded_grant",
            )
            .map_err(app_error)?;
            entries.push(ConnectAttempt {
                id: Uuid::new_v4().to_string(),
                secret_ref: reference.clone(),
                revision,
                epoch: context.attempt.epoch,
                phase: ConnectPhase::Received,
                superseded: true,
            });
        } else {
            connected
                .metadata
                .as_object_mut()
                .ok_or_else(|| app_error(AgentMcpError::Storage))?
                .insert("oauthRetiredSecretRef".into(), json!(reference));
        }
    }
    set_attempts(&mut connected, &entries).map_err(app_error)?;
    repository.replace(&connected).await.map_err(app_error)?;
    context
        .lifecycle
        .unconfirmed_revocation
        .store(false, Ordering::SeqCst);
    // Reconnect never revokes a potentially shared OAuth family. Keep the
    // old grant as nonblocking custody until explicit Disconnect/Delete.
    if !old_oauth {
        if let Some((reference, _)) = previous {
            if store.delete(&reference).is_err() {
                return Err(app_error(AgentMcpError::OauthCleanupPending));
            }
            connected
                .metadata
                .as_object_mut()
                .ok_or_else(|| app_error(AgentMcpError::Storage))?
                .remove("oauthRetiredSecretRef");
            repository.replace(&connected).await.map_err(app_error)?;
        }
    }
    Ok(connected)
}
