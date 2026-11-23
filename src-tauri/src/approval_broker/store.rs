//! Persistence of CLI approval requests: the `interruption` items in
//! `agent_items` and the run/session status they hold at
//! `waiting_for_user`.
//!
//! The broker writes its items without touching `agent_runs.last_sequence`:
//! the CLI turn numbers its own events (`chat_engine::turn`) and keeps
//! publishing them while a tool waits, so the broker must not advance that
//! counter. The frontend applies CLI interruption events outside the run's
//! sequence gate for the same reason (`src/lib/agent-runtime-adapter.ts`).

use super::{ApprovalTurn, CLI_ENGINE};
use serde_json::{json, Value};
use sqlx::row::Row;
use sqlx_sqlite::SqlitePool;
use uuid::Uuid;

/// A recorded item, as emitted to the webview.
#[derive(Debug)]
pub struct StoredInterruption {
    pub external_id: String,
    /// The run's last event sequence when the item was written.
    pub sequence: i64,
    /// The item's persisted position across the whole session.
    pub item_sequence: i64,
    pub payload: Value,
}

#[derive(Debug)]
pub enum InsertError {
    /// The run is finished (or missing): nobody can answer for it.
    RunNotActive,
    Database(sqlx::Error),
}

impl From<sqlx::Error> for InsertError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

pub fn external_id(run_id: &str, interruption_id: &str) -> String {
    format!("interruption:{run_id}:{interruption_id}")
}

/// Record an automatic authorization only while the stored run still carries
/// the bypass snapshot and remains active.
pub async fn insert_bypass_receipt(
    pool: &SqlitePool,
    turn: &ApprovalTurn,
    payload: &Value,
) -> Result<(), InsertError> {
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let run = sqlx::query::query("SELECT 1 FROM agent_runs WHERE id = ? AND session_id = ? AND status IN ('queued', 'running', 'waiting_for_user') AND json_extract(run_config_json, '$.approvalMode') = 'bypass'")
        .bind(&turn.run_id).bind(&turn.session_id).fetch_optional(&mut *transaction).await?;
    if run.is_none() {
        transaction.rollback().await?;
        return Err(InsertError::RunNotActive);
    }
    let id = payload
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    sqlx::query::query("INSERT INTO agent_items (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at) SELECT ?, ?, ?, COALESCE(MAX(sequence), -1) + 1, 'interruption', ?, ?, ? FROM agent_items WHERE session_id = ?")
        .bind(Uuid::new_v4().to_string()).bind(&turn.session_id).bind(&turn.run_id).bind(payload.to_string())
        .bind(external_id(&turn.run_id, id)).bind(crate::db::repositories::timestamp()).bind(&turn.session_id)
        .execute(&mut *transaction).await?;
    transaction.commit().await?;
    Ok(())
}

/// Records a pending request and moves its run and session to
/// `waiting_for_user`, in one transaction.
pub async fn insert_pending(
    pool: &SqlitePool,
    turn: &ApprovalTurn,
    payload: &Value,
) -> Result<StoredInterruption, InsertError> {
    let interruption_id = payload
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let external_id = external_id(&turn.run_id, interruption_id);
    let now = crate::db::repositories::timestamp();
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let run = sqlx::query::query(
        "SELECT status, last_sequence FROM agent_runs WHERE id = ? AND session_id = ?",
    )
    .bind(&turn.run_id)
    .bind(&turn.session_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(run) = run else {
        transaction.rollback().await?;
        return Err(InsertError::RunNotActive);
    };
    let status: String = run.get("status");
    if !matches!(status.as_str(), "queued" | "running" | "waiting_for_user") {
        transaction.rollback().await?;
        return Err(InsertError::RunNotActive);
    }
    let sequence: i64 = run.get("last_sequence");
    let display_sequence: i64 = sqlx::query::query(
        "SELECT COALESCE(MAX(sequence), -1) + 1 AS next_sequence FROM agent_items WHERE session_id = ?",
    )
    .bind(&turn.session_id)
    .fetch_one(&mut *transaction)
    .await?
    .get("next_sequence");
    sqlx::query::query(
        "INSERT INTO agent_items
         (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at)
         VALUES (?, ?, ?, ?, 'interruption', ?, ?, ?)",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(&turn.session_id)
    .bind(&turn.run_id)
    .bind(display_sequence)
    .bind(payload.to_string())
    .bind(&external_id)
    .bind(&now)
    .execute(&mut *transaction)
    .await?;
    sqlx::query::query(
        "UPDATE agent_runs SET status = 'waiting_for_user', updated_at = ?
         WHERE id = ? AND status IN ('queued', 'running', 'waiting_for_user')",
    )
    .bind(&now)
    .bind(&turn.run_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query::query(
        "UPDATE agent_sessions SET status = 'waiting_for_user', updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(&turn.session_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(StoredInterruption {
        external_id,
        sequence,
        item_sequence: display_sequence,
        payload: payload.clone(),
    })
}

/// How a request left `pending`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settlement {
    Approved,
    Denied,
    Answered(String),
    Expired,
    Cancelled,
}

/// A CLI interruption item, read for the resolution command.
#[derive(Debug)]
pub struct CliInterruption {
    pub session_id: String,
    pub payload: Value,
}

/// The interruption `interruption_id` of `run_id`, if the broker wrote it.
pub async fn find_cli_interruption(
    pool: &SqlitePool,
    run_id: &str,
    interruption_id: &str,
) -> Result<Option<CliInterruption>, sqlx::Error> {
    let row = sqlx::query::query(
        "SELECT session_id, payload_json FROM agent_items
         WHERE kind = 'interruption'
           AND run_id = ?
           AND json_extract(payload_json, '$.id') = ?
           AND json_extract(payload_json, '$.engine') = ?
         ORDER BY created_at DESC
         LIMIT 1",
    )
    .bind(run_id)
    .bind(interruption_id)
    .bind(CLI_ENGINE)
    .fetch_optional(pool)
    .await?;
    Ok(row.and_then(|row| {
        let payload_json: String = row.get("payload_json");
        Some(CliInterruption {
            session_id: row.get("session_id"),
            payload: serde_json::from_str(&payload_json).ok()?,
        })
    }))
}

/// Moves one pending CLI item to its outcome. Returns `None` (no change)
/// when the item is not pending any more: an item leaves `pending` once.
/// When no request of the run is left pending, the run and its session go
/// back from `waiting_for_user` to `running` (the CLI turn is still going;
/// it publishes the run's terminal event itself).
pub async fn settle(
    pool: &SqlitePool,
    run_id: &str,
    interruption_id: &str,
    settlement: &Settlement,
) -> Result<Option<StoredInterruption>, sqlx::Error> {
    let now = crate::db::repositories::timestamp();
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row = sqlx::query::query(
        "SELECT i.id, i.session_id, i.payload_json, i.sequence AS item_sequence, r.last_sequence
         FROM agent_items i JOIN agent_runs r ON r.id = i.run_id
         WHERE i.kind = 'interruption'
           AND i.run_id = ?
           AND json_extract(i.payload_json, '$.id') = ?
           AND json_extract(i.payload_json, '$.engine') = ?
           AND json_extract(i.payload_json, '$.status') = 'pending'
           AND (? = 0 OR r.status IN ('queued', 'running', 'waiting_for_user'))
         LIMIT 1",
    )
    .bind(run_id)
    .bind(interruption_id)
    .bind(CLI_ENGINE)
    .bind(matches!(
        settlement,
        Settlement::Approved | Settlement::Answered(_)
    ))
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(row) = row else {
        transaction.rollback().await?;
        return Ok(None);
    };
    let item_id: String = row.get("id");
    let session_id: String = row.get("session_id");
    let sequence: i64 = row.get("last_sequence");
    let item_sequence: i64 = row.get("item_sequence");
    let payload_json: String = row.get("payload_json");
    let mut payload: Value = serde_json::from_str(&payload_json)
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
    apply_settlement(&mut payload, settlement, &now);
    let updated = sqlx::query::query(
        "UPDATE agent_items SET payload_json = ?
         WHERE id = ? AND json_extract(payload_json, '$.status') = 'pending'",
    )
    .bind(payload.to_string())
    .bind(&item_id)
    .execute(&mut *transaction)
    .await?
    .rows_affected();
    if updated != 1 {
        transaction.rollback().await?;
        return Ok(None);
    }
    let still_pending: i64 = sqlx::query::query(
        "SELECT COUNT(*) AS pending FROM agent_items
         WHERE run_id = ? AND kind = 'interruption'
           AND json_extract(payload_json, '$.status') = 'pending'",
    )
    .bind(run_id)
    .fetch_one(&mut *transaction)
    .await?
    .get("pending");
    if still_pending == 0 {
        let resumed = sqlx::query::query(
            "UPDATE agent_runs SET status = 'running', updated_at = ?
             WHERE id = ? AND status = 'waiting_for_user'",
        )
        .bind(&now)
        .bind(run_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if resumed == 1 {
            sqlx::query::query(
                "UPDATE agent_sessions SET status = 'running', updated_at = ?
                 WHERE id = ? AND status = 'waiting_for_user'",
            )
            .bind(&now)
            .bind(&session_id)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(Some(StoredInterruption {
        external_id: external_id(run_id, interruption_id),
        sequence,
        item_sequence,
        payload,
    }))
}

fn apply_settlement(payload: &mut Value, settlement: &Settlement, now: &str) {
    payload["resolvedAt"] = json!(now);
    match settlement {
        Settlement::Approved => {
            payload["status"] = json!("resolved");
            payload["resolution"] = json!("once");
            payload["approved"] = json!(true);
            payload["resolvedBy"] = json!("desktop");
            payload["authorizationOrigin"] = json!("user_once");
            payload["authorizationPolicyRunId"] = payload["runId"].clone();
        }
        Settlement::Denied => {
            payload["status"] = json!("resolved");
            payload["resolution"] = json!("deny");
            payload["approved"] = json!(false);
            payload["resolvedBy"] = json!("desktop");
        }
        Settlement::Answered(answer) => {
            payload["status"] = json!("resolved");
            payload["answer"] = json!(answer);
            payload["resolvedBy"] = json!("desktop");
        }
        Settlement::Expired => {
            payload["status"] = json!("expired");
            payload["retiredReason"] = json!("timeout");
        }
        Settlement::Cancelled => {
            payload["status"] = json!("cancelled");
            payload["retiredReason"] = json!("cancelled");
        }
    }
}

/// Startup: cancels CLI items left pending by a previous process (their CLI
/// died with it), except `keep` (requests of this process), and ends the
/// runs they held at `waiting_for_user`. Returns how many items changed.
pub async fn cancel_stale(pool: &SqlitePool, keep: &[String]) -> Result<u64, sqlx::Error> {
    let now = crate::db::repositories::timestamp();
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let rows = sqlx::query::query(
        "SELECT id, run_id, json_extract(payload_json, '$.id') AS interruption_id
         FROM agent_items
         WHERE kind = 'interruption'
           AND json_extract(payload_json, '$.engine') = ?
           AND json_extract(payload_json, '$.status') = 'pending'",
    )
    .bind(CLI_ENGINE)
    .fetch_all(&mut *transaction)
    .await?;
    let mut changed = 0;
    let mut runs = Vec::new();
    for row in rows {
        let interruption_id: Option<String> = row.get("interruption_id");
        if interruption_id
            .as_deref()
            .is_some_and(|id| keep.iter().any(|kept| kept == id))
        {
            continue;
        }
        let item_id: String = row.get("id");
        let run_id: Option<String> = row.get("run_id");
        changed += sqlx::query::query(
            "UPDATE agent_items
             SET payload_json = json_set(payload_json, '$.status', 'cancelled',
                 '$.retiredReason', 'restarted', '$.resolvedAt', ?)
             WHERE id = ? AND json_extract(payload_json, '$.status') = 'pending'",
        )
        .bind(&now)
        .bind(&item_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if let Some(run_id) = run_id {
            if !runs.contains(&run_id) {
                runs.push(run_id);
            }
        }
    }
    for run_id in runs {
        let session_id: Option<String> = sqlx::query::query(
            "UPDATE agent_runs
             SET status = 'cancelled', updated_at = ?, completed_at = COALESCE(completed_at, ?)
             WHERE id = ? AND status = 'waiting_for_user'
             RETURNING session_id",
        )
        .bind(&now)
        .bind(&now)
        .bind(&run_id)
        .fetch_optional(&mut *transaction)
        .await?
        .map(|row| row.get("session_id"));
        if let Some(session_id) = session_id {
            sqlx::query::query(
                "UPDATE agent_sessions SET status = 'idle', updated_at = ?
                 WHERE id = ?
                   AND NOT EXISTS (
                     SELECT 1 FROM agent_runs
                     WHERE session_id = ? AND status IN ('queued', 'running', 'waiting_for_user')
                   )",
            )
            .bind(&now)
            .bind(&session_id)
            .bind(&session_id)
            .execute(&mut *transaction)
            .await?;
        }
    }
    transaction.commit().await?;
    Ok(changed)
}
