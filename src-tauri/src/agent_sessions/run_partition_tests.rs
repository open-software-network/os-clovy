use super::{resolve, revalidate};
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::db::repositories::{guard_bound_turn, Repositories};
use crate::domain::turn_authority::TurnAuthority;
use sqlx::query::query;
use sqlx_sqlite::{SqlitePool, SqlitePoolOptions};

async fn fixture(pool: SqlitePool, partition: &str) -> (AgentRepository, String, String) {
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let repo = AgentRepository::new(pool);
    let session = repo
        .create_session_in_profile(
            "Caller",
            "test",
            AgentSafetyMode::Sandboxed,
            None,
            partition,
        )
        .await
        .unwrap();
    let run = repo.create_run(&session.id, "test", None).await.unwrap();
    (repo, session.id, run.id)
}

async fn memory() -> SqlitePool {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap()
}

#[tokio::test]
async fn run_snapshot_rejects_reassignment_and_new_run_uses_new_partition() {
    let (repo, session, run) = fixture(memory().await, "A").await;
    assert_eq!(resolve(&repo.pool, &session, &run).await.unwrap(), "A");
    let foreign = repo
        .create_session("Foreign", "test", AgentSafetyMode::Sandboxed, None)
        .await
        .unwrap();
    assert_eq!(
        resolve(&repo.pool, &foreign.id, &run)
            .await
            .unwrap_err()
            .code,
        "turn_expired"
    );
    // Later metadata may not replace host-owned authority.
    repo.set_run_config(
        &run,
        &serde_json::json!({"dataPartition":"B", "surface":"home"}),
    )
    .await
    .unwrap();
    assert_eq!(resolve(&repo.pool, &session, &run).await.unwrap(), "A");
    Repositories::new(repo.pool.clone())
        .assign_session_to_profile(&session, "B")
        .await
        .unwrap();
    assert_eq!(
        resolve(&repo.pool, &session, &run).await.unwrap_err().code,
        "turn_expired"
    );
    let next = repo.create_run(&session, "test", None).await.unwrap();
    assert_eq!(resolve(&repo.pool, &session, &next.id).await.unwrap(), "B");
    assert_eq!(
        revalidate(&repo.pool, &session, &next.id, "A")
            .await
            .unwrap_err()
            .code,
        "turn_expired"
    );
}

#[tokio::test]
async fn invalid_snapshots_never_adopt_the_current_mapping_at_reads_or_writes() {
    for config in [
        None,
        Some("{}"),
        Some("null"),
        Some("{"),
        Some(r#"{"dataPartition":null}"#),
        Some(r#"{"dataPartition":3}"#),
        Some(r#"{"dataPartition":""}"#),
        Some(r#"{"dataPartition":" A "}"#),
        Some(r#"{"dataPartition":"B"}"#),
    ] {
        let (repo, session, run) = fixture(memory().await, "A").await;
        query("UPDATE agent_runs SET run_config_json=? WHERE id=?")
            .bind(config)
            .bind(&run)
            .execute(&repo.pool)
            .await
            .unwrap();
        assert_eq!(
            resolve(&repo.pool, &session, &run).await.unwrap_err().code,
            "turn_expired"
        );
        if config != Some("{") {
            repo.set_run_config(
                &run,
                &serde_json::json!({"dataPartition":"A", "surface":"home"}),
            )
            .await
            .unwrap();
            assert_eq!(
                resolve(&repo.pool, &session, &run).await.unwrap_err().code,
                "turn_expired"
            );
        }
        let mut tx = repo.pool.begin().await.unwrap();
        let result = guard_bound_turn(&mut tx, "A", &session, &run, &TurnAuthority::new()).await;
        assert_eq!(result.unwrap_err().code, "turn_expired");
        tx.rollback().await.unwrap();
    }
}

#[tokio::test]
async fn default_unprofiled_and_routine_snapshots_keep_their_real_origin() {
    let pool = memory().await;
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let repo = AgentRepository::new(pool);
    for source in ["user", "routine"] {
        let session = repo
            .create_session("Unprofiled", "test", AgentSafetyMode::Sandboxed, None)
            .await
            .unwrap();
        query("UPDATE agent_sessions SET source=? WHERE id=?")
            .bind(source)
            .bind(&session.id)
            .execute(&repo.pool)
            .await
            .unwrap();
        let run = repo.create_run(&session.id, "test", None).await.unwrap();
        assert_eq!(
            resolve(&repo.pool, &session.id, &run.id).await.unwrap(),
            "default"
        );
        assert_eq!(
            run.approval_mode,
            crate::agent_sessions::AgentApprovalMode::Approvals
        );
        Repositories::new(repo.pool.clone())
            .assign_session_to_profile(&session.id, "B")
            .await
            .unwrap();
        assert_eq!(
            resolve(&repo.pool, &session.id, &run.id)
                .await
                .unwrap_err()
                .code,
            "turn_expired"
        );
    }
    let (_, session, run) = fixture(repo.pool.clone(), "default").await;
    assert_eq!(
        resolve(&repo.pool, &session, &run).await.unwrap(),
        "default"
    );
}

#[tokio::test]
async fn missing_association_run_and_terminal_status_refuse_old_authority() {
    for change in ["mapping", "run", "session", "cancelled", "completed"] {
        let (repo, session, run) = fixture(memory().await, "A").await;
        let sql = match change {
            "mapping" => "DELETE FROM session_profiles WHERE session_id=?",
            "run" => "DELETE FROM agent_runs WHERE session_id=?",
            "session" => "DELETE FROM agent_sessions WHERE id=?",
            "cancelled" => "UPDATE agent_runs SET status='cancelled' WHERE session_id=?",
            _ => "UPDATE agent_runs SET status='completed' WHERE session_id=?",
        };
        query(sql).bind(&session).execute(&repo.pool).await.unwrap();
        assert_eq!(
            resolve(&repo.pool, &session, &run).await.unwrap_err().code,
            "turn_expired"
        );
    }
}

#[tokio::test]
async fn writer_wait_rechecks_snapshot_mapping_and_stop_before_sentinel_effect() {
    for change in ["mapping", "snapshot", "malformed", "stop"] {
        let directory = tempfile::tempdir().unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(
                sqlx_sqlite::SqliteConnectOptions::new()
                    .filename(directory.path().join("fixture.sqlite3"))
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        let (repo, session, run) = fixture(pool, "A").await;
        let sentinel = repo
            .create_session_in_profile("B sentinel", "test", AgentSafetyMode::Sandboxed, None, "B")
            .await
            .unwrap();
        let authority = TurnAuthority::new();
        let mut writer = repo.pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
        let (admit, attempted) = tokio::sync::oneshot::channel();
        let effect = async {
            let mut tx = repo.pool.begin().await.unwrap();
            admit.send(()).unwrap();
            guard_bound_turn(&mut tx, "A", &session, &run, &authority).await?;
            query("UPDATE agent_sessions SET title='Changed' WHERE id=?")
                .bind(&sentinel.id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            Ok::<_, crate::domain::types::AppError>(())
        };
        let mutate = async {
            attempted.await.unwrap();
            match change {
                "mapping" => {
                    query("UPDATE session_profiles SET profile='B' WHERE session_id=?")
                        .bind(&session)
                        .execute(&mut *writer)
                        .await
                        .unwrap();
                }
                "snapshot" => {
                    query("UPDATE agent_runs SET run_config_json='{\"dataPartition\":\"B\"}' WHERE id=?").bind(&run).execute(&mut *writer).await.unwrap();
                }
                "malformed" => {
                    query("UPDATE agent_runs SET run_config_json='{' WHERE id=?")
                        .bind(&run)
                        .execute(&mut *writer)
                        .await
                        .unwrap();
                }
                _ => authority.revoke(),
            }
            writer.commit().await.unwrap();
        };
        let (result, ()) = tokio::join!(effect, mutate);
        assert_eq!(result.unwrap_err().code, "turn_expired");
        assert_eq!(
            repo.get_session(&sentinel.id).await.unwrap().title,
            "B sentinel"
        );
    }
}

#[derive(Clone)]
struct ApprovalIo(SqlitePool);
impl crate::approval_broker::BrokerIo for ApprovalIo {
    fn pool(
        &self,
    ) -> futures_util::future::BoxFuture<'_, Result<SqlitePool, crate::domain::types::AppError>>
    {
        Box::pin(async { Ok(self.0.clone()) })
    }
    fn emit(&self, _: &str, _: &str, _: i64, _: &str, _: serde_json::Value) {}
}

#[tokio::test]
async fn real_approval_wait_cannot_authorize_a_reassigned_run() {
    use crate::approval_broker::{
        ApprovalBroker, ApprovalKind, ApprovalRequest, ApprovalTurn, Decision, TokioClock,
    };
    let (repo, session, run) = fixture(memory().await, "A").await;
    let sentinel = repo
        .create_session_in_profile("B sentinel", "test", AgentSafetyMode::Sandboxed, None, "B")
        .await
        .unwrap();
    let broker = ApprovalBroker::new(
        ApprovalIo(repo.pool.clone()),
        TokioClock,
        std::time::Duration::from_secs(5),
    );
    let approval = ApprovalTurn {
        session_id: session.clone(),
        run_id: run.clone(),
        cli: "claude".into(),
        safety_mode: AgentSafetyMode::Sandboxed,
        attended: true,
    };
    let authority = TurnAuthority::new();
    let effect = async {
        broker
            .request(
                &approval,
                ApprovalRequest {
                    tool: "delete_conversation".into(),
                    call_id: None,
                    kind: ApprovalKind::Approval {
                        title: "Delete conversation".into(),
                        description: "Synthetic test".into(),
                        arguments: serde_json::json!({"conversationId":sentinel.id}),
                        preview: None,
                    },
                },
            )
            .await
            .unwrap();
        // The same domain consumer called after tool approval, with the
        // original host partition. Its writer admission must refuse the run.
        repo.delete_session_for_turn(&sentinel.id, Some(("A", &session, &run, &authority)))
            .await
    };
    let reassign = async {
        let request = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(id) = broker.waiting_ids().first() {
                    break id.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        Repositories::new(repo.pool.clone())
            .assign_session_to_profile(&session, "B")
            .await
            .unwrap();
        assert_eq!(
            resolve(&repo.pool, &session, &run).await.unwrap_err().code,
            "turn_expired"
        );
        broker
            .decide(&repo.pool, &request, Decision::Approve)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(effect, reassign);
    assert_eq!(result.unwrap_err().code, "turn_expired");
    assert_eq!(
        repo.get_session(&sentinel.id).await.unwrap().title,
        "B sentinel"
    );
    let settled: String = sqlx::query_scalar::query_scalar("SELECT json_extract(payload_json,'$.status') FROM agent_items WHERE run_id=? AND kind='interruption'").bind(&run).fetch_one(&repo.pool).await.unwrap();
    assert_eq!(settled, "resolved");
    let approved: bool = sqlx::query_scalar::query_scalar("SELECT json_extract(payload_json,'$.approved') FROM agent_items WHERE run_id=? AND kind='interruption'").bind(&run).fetch_one(&repo.pool).await.unwrap();
    assert!(approved);
}
