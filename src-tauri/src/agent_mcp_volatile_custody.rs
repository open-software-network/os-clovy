//! In-process custody of a received grant that could not be stored securely.
use super::{partition::CandidatePartition, AgentMcpError, McpSecretBundle};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ConnectPhase {
    Authorizing,
    ExchangePending,
    Received,
    RevocationFailed,
    RevokedButLocalCleanupPending,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ConnectAttempt {
    pub(super) id: String,
    pub(super) secret_ref: String,
    pub(super) revision: Option<String>,
    pub(super) epoch: u64,
    pub(super) phase: ConnectPhase,
    #[serde(default)]
    pub(super) superseded: bool,
}
struct VolatileCandidate {
    partition: Arc<CandidatePartition>,
    server: String,
    attempt: ConnectAttempt,
    bundle: McpSecretBundle,
    confirmed: bool,
}
static VOLATILE_CANDIDATES: OnceLock<Mutex<Vec<VolatileCandidate>>> = OnceLock::new();
fn key_matches(entry: &VolatileCandidate, server: &str, attempt: &ConnectAttempt) -> bool {
    entry.server == server
        && entry.attempt.id == attempt.id
        && entry.attempt.secret_ref == attempt.secret_ref
        && entry.attempt.revision == attempt.revision
        && entry.attempt.epoch == attempt.epoch
        && entry.attempt.superseded == attempt.superseded
}
pub(super) fn retain_candidate(
    partition: &Arc<CandidatePartition>,
    server: &str,
    attempt: &ConnectAttempt,
    bundle: &McpSecretBundle,
) {
    let mut candidates = VOLATILE_CANDIDATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(entry) = candidates
        .iter_mut()
        .find(|entry| key_matches(entry, server, attempt) && entry.partition.same(partition))
    {
        entry.bundle = bundle.clone();
    } else {
        candidates.push(VolatileCandidate {
            partition: Arc::clone(partition),
            server: server.to_owned(),
            attempt: attempt.clone(),
            bundle: bundle.clone(),
            confirmed: false,
        });
    }
}
pub(super) fn volatile_candidate(
    partition: &CandidatePartition,
    server: &str,
    attempt: &ConnectAttempt,
) -> Result<Option<(McpSecretBundle, bool)>, AgentMcpError> {
    let candidates = VOLATILE_CANDIDATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for entry in candidates
        .iter()
        .filter(|entry| key_matches(entry, server, attempt))
    {
        if entry.partition.same(partition) {
            return Ok(Some((entry.bundle.clone(), entry.confirmed)));
        }
        if !entry.partition.definitely_different(partition) {
            return Err(AgentMcpError::Storage);
        }
    }
    Ok(None)
}
pub(super) fn update_volatile_candidate(
    partition: &CandidatePartition,
    server: &str,
    attempt: &ConnectAttempt,
    remove: bool,
) {
    let mut candidates = VOLATILE_CANDIDATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(position) = candidates
        .iter()
        .position(|entry| key_matches(entry, server, attempt) && entry.partition.same(partition))
    {
        if remove {
            candidates.remove(position);
        } else {
            candidates[position].confirmed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx_sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    #[tokio::test]
    async fn exact_candidate_survives_reopen_without_crossing_partitions_or_revisions() {
        let root = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            SqliteConnectOptions::new()
                .filename(root.path().join(name))
                .create_if_missing(true)
        };
        let first = SqlitePoolOptions::new()
            .connect_with(make("a.sqlite"))
            .await
            .unwrap();
        let other = SqlitePoolOptions::new()
            .connect_with(make("b.sqlite"))
            .await
            .unwrap();
        let partition = Arc::new(CandidatePartition::new(&first).await);
        let second = CandidatePartition::new(&other).await;
        let attempt = ConnectAttempt {
            id: "attempt-a".into(),
            secret_ref: "ref-a".into(),
            revision: Some("revision-a".into()),
            epoch: 1,
            phase: ConnectPhase::ExchangePending,
            superseded: false,
        };
        let mut bundle = McpSecretBundle::default();
        bundle
            .oauth
            .insert("refresh_token".into(), "synthetic-candidate-a".into());
        retain_candidate(&partition, "server", &attempt, &bundle);
        assert!(volatile_candidate(&second, "server", &attempt)
            .unwrap()
            .is_none());
        let mut different = attempt.clone();
        different.revision = Some("revision-b".into());
        assert!(volatile_candidate(&partition, "server", &different)
            .unwrap()
            .is_none());
        first.close().await;
        let reopened = SqlitePoolOptions::new()
            .connect_with(make("a.sqlite"))
            .await
            .unwrap();
        let recovered = CandidatePartition::new(&reopened).await;
        let (found, confirmed) = volatile_candidate(&recovered, "server", &attempt)
            .unwrap()
            .unwrap();
        assert_eq!(
            found.oauth.get("refresh_token").map(String::as_str),
            Some("synthetic-candidate-a")
        );
        assert!(!confirmed);
        update_volatile_candidate(&recovered, "server", &attempt, false);
        assert!(
            volatile_candidate(&recovered, "server", &attempt)
                .unwrap()
                .unwrap()
                .1
        );
        update_volatile_candidate(&recovered, "server", &attempt, true);
        assert!(volatile_candidate(&recovered, "server", &attempt)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn volatile_candidate_is_not_shared_with_new_process() {
        let attempt = ConnectAttempt {
            id: "process-attempt".into(),
            secret_ref: "process-ref".into(),
            revision: Some("process-revision".into()),
            epoch: 1,
            phase: ConnectPhase::ExchangePending,
            superseded: false,
        };
        if let Some(path) = std::env::var_os("CLOVY_CUSTODY_TEST_DB") {
            let pool = SqlitePoolOptions::new()
                .connect_with(SqliteConnectOptions::new().filename(path))
                .await
                .unwrap();
            let partition = CandidatePartition::new(&pool).await;
            assert!(volatile_candidate(&partition, "process-server", &attempt)
                .unwrap()
                .is_none());
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("process.sqlite");
        let pool = SqlitePoolOptions::new()
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let partition = Arc::new(CandidatePartition::new(&pool).await);
        let mut bundle = McpSecretBundle::default();
        bundle
            .oauth
            .insert("refresh_token".into(), "synthetic-process-candidate".into());
        retain_candidate(&partition, "process-server", &attempt, &bundle);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "agent_mcp::volatile_custody::tests::volatile_candidate_is_not_shared_with_new_process"])
            .env("CLOVY_CUSTODY_TEST_DB", &path).status().unwrap();
        assert!(status.success());
        assert!(volatile_candidate(&partition, "process-server", &attempt)
            .unwrap()
            .is_some());
        update_volatile_candidate(&partition, "process-server", &attempt, true);
    }
}
