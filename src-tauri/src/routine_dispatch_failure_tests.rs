use super::{persist_claim_run, record_dispatch_failure, Claim};
use crate::{
    agent_sessions::{protocol::RpcFrame, AgentRepository, AgentSafetyMode},
    chat_engine::ChatEngineHost,
    domain::types::AppError,
};
use sqlx::{query::query, row::Row};

#[tokio::test]
async fn dispatch_failure_cannot_replace_the_cancelled_routine_history() {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    query("INSERT INTO routines(id,name,prompt,schedule,claim_token,created_at,updated_at) VALUES ('routine','Synthetic','Synthetic','every 1d','synthetic-claim','now','now')").execute(&pool).await.unwrap();
    query("INSERT INTO routine_runs(id,routine_id,claim_token,trigger_kind,status,model,safety_mode,created_at,updated_at) VALUES ('routine-run','routine','synthetic-claim','manual','queued','test','sandboxed','now','now')").execute(&pool).await.unwrap();
    let claim = Claim {
        routine_id: "routine".into(),
        routine_name: "Synthetic".into(),
        prompt: "Synthetic".into(),
        schedule: "every 1d".into(),
        timezone: "UTC".into(),
        trigger_kind: "manual".into(),
        model: "test".into(),
        safety_mode: AgentSafetyMode::Sandboxed,
        enabled_toolsets: vec![],
        token: "synthetic-claim".into(),
        routine_run_id: "routine-run".into(),
    };
    let repo = AgentRepository::new(pool.clone());
    let dir = tempfile::tempdir().unwrap();
    let run = persist_claim_run(
        &pool,
        &repo,
        &claim,
        "routine-session",
        dir.path(),
        "now",
        &mut None,
    )
    .await
    .unwrap();
    let host = ChatEngineHost::default();
    let ticket = host.register(&run).unwrap();
    let (changed, owned) = crate::agent_sessions::api::claim_ui_run_cancellation_for_host(
        &pool,
        "routine-session",
        &run,
        &host,
        None,
    )
    .await
    .unwrap();
    assert!(changed && owned);
    let frame = RpcFrame {
        jsonrpc: "2.0".into(),
        protocol_version: 1,
        session_id: "routine-session".into(),
        run_id: run.clone(),
        sequence: 0,
        id: None,
        event_id: None,
        method: Some("run.failed".into()),
        params: Some(serde_json::json!({"error":"Synthetic resolver failure"})),
        result: None,
        error: None,
    };
    let terminal = crate::agent_sessions::terminal::persist_terminal_event(
        &pool,
        &frame,
        crate::agent_sessions::events::sanitize_log,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(terminal.method, "run.cancelled");
    // Represent an already persisted cancelled routine projection. The
    // fallback consumer must preserve that state even with a retained claim.
    query(
        "UPDATE routine_runs SET status='cancelled',completed_at='stop-won' WHERE id='routine-run'",
    )
    .execute(&pool)
    .await
    .unwrap();
    record_dispatch_failure(
        &pool,
        &claim,
        &AppError::new("turn_expired", "Synthetic resolver failure"),
        None,
    )
    .await;
    let row =
        query("SELECT status,completed_at,error_message FROM routine_runs WHERE id='routine-run'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(row.get::<String, _>("status"), "cancelled");
    assert_eq!(row.get::<String, _>("completed_at"), "stop-won");
    assert!(row.get::<Option<String>, _>("error_message").is_none());
    let last_status: Option<String> =
        sqlx::query_scalar::query_scalar("SELECT last_status FROM routines WHERE id='routine'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(last_status.is_none());
    drop(ticket);
    host.release(&run);
}

#[tokio::test]
async fn ordinary_routine_dispatch_failure_still_records_error() {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    query("INSERT INTO routines(id,name,prompt,schedule,claim_token,created_at,updated_at) VALUES ('routine','Synthetic','Synthetic','every 1d','synthetic-claim','now','now')").execute(&pool).await.unwrap();
    query("INSERT INTO routine_runs(id,routine_id,claim_token,trigger_kind,status,model,safety_mode,created_at,updated_at) VALUES ('routine-run','routine','synthetic-claim','manual','queued','test','sandboxed','now','now')").execute(&pool).await.unwrap();
    let claim = Claim {
        routine_id: "routine".into(),
        routine_name: "Synthetic".into(),
        prompt: "Synthetic".into(),
        schedule: "every 1d".into(),
        timezone: "UTC".into(),
        trigger_kind: "manual".into(),
        model: "test".into(),
        safety_mode: AgentSafetyMode::Sandboxed,
        enabled_toolsets: vec![],
        token: "synthetic-claim".into(),
        routine_run_id: "routine-run".into(),
    };
    record_dispatch_failure(
        &pool,
        &claim,
        &AppError::new("agent_fixture", "Synthetic dispatch failure"),
        None,
    )
    .await;
    let status: String =
        sqlx::query_scalar::query_scalar("SELECT status FROM routine_runs WHERE id='routine-run'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    let row = query("SELECT last_status,last_error,claim_token FROM routines WHERE id='routine'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("last_status"), "error");
    assert_eq!(
        row.get::<String, _>("last_error"),
        "Synthetic dispatch failure"
    );
    assert!(row.get::<Option<String>, _>("claim_token").is_none());
}
