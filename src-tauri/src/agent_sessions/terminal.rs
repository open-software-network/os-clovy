//! Arbitrate the persisted winner and its single terminal publication together.
use super::protocol::RpcFrame;
use crate::domain::types::AppError;
use serde_json::{json, Value};
use sqlx::{query::query, row::Row};
use sqlx_sqlite::SqlitePool;
use uuid::Uuid;

pub(crate) struct TerminalEvent {
    pub method: String,
    pub event_id: String,
    pub sequence: i64,
    pub data: Value,
}

/// Stop and completion contend for the same SQLite writer. Once a terminal
/// status wins it is immutable; the publication claim must use that winner,
/// even when the caller finished its CLI/output work before Stop arrived.
/// The internal run config marker also arbitrates host/fallback publishers.
pub(crate) async fn persist_terminal_event(
    pool: &SqlitePool,
    frame: &RpcFrame,
    sanitize: fn(&str) -> String,
) -> Result<Option<TerminalEvent>, AppError> {
    let proposed = match frame.method.as_deref() {
        Some("run.completed") => "completed",
        Some("run.cancelled") => "cancelled",
        Some("run.failed") => "failed",
        _ => return Ok(None),
    };
    let params = frame.params.as_ref();
    let field = |key| {
        params
            .and_then(|value| value.get(key))
            .and_then(Value::as_str)
    };
    let message = field("error")
        .map(sanitize)
        .unwrap_or_else(|| "Agent run failed.".into());
    let code = field("code")
        .filter(|value| value.starts_with("agent_") || *value == "runtime_crashed")
        .unwrap_or("agent_runtime_failed");
    let now = crate::db::repositories::timestamp();
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = query(
        "UPDATE agent_runs SET status=?, updated_at=?, completed_at=?, error_code=?, error_message=?
         WHERE id=? AND session_id=? AND status NOT IN ('completed','cancelled','failed','interrupted')",
    )
    .bind(proposed)
    .bind(&now)
    .bind(&now)
    .bind((proposed == "failed").then_some(code))
    .bind((proposed == "failed").then_some(message.as_str()))
    .bind(&frame.run_id)
    .bind(&frame.session_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let row = query(
        "SELECT status, completed_at, error_code, error_message, last_sequence,
                json_extract(run_config_json,'$.clovyTerminalEvent.runId') AS publication_run
         FROM agent_runs WHERE id=? AND session_id=?",
    )
    .bind(&frame.run_id)
    .bind(&frame.session_id)
    .fetch_one(&mut *tx)
    .await?;
    let status: String = row.get("status");
    let method = match status.as_str() {
        "completed" => "run.completed",
        "cancelled" => "run.cancelled",
        "failed" | "interrupted" => "run.failed",
        _ => {
            return Err(AppError::new(
                "agent_terminal_invalid",
                "Run is not terminal.",
            ))
        }
    };
    // Stop may already have claimed the persisted terminal winner. Settle its
    // unresolved outer CLI calls under the same writer, independently of who
    // changed the run or previously published its terminal event. An absent
    // result says nothing about whether the tool's effect committed.
    let interrupted = query(
        "UPDATE agent_items SET payload_json=json_set(payload_json,'$.status','interrupted')
         WHERE session_id=? AND run_id=? AND kind='tool_call'
           AND json_extract(payload_json,'$.status')='running'
           AND json_extract(payload_json,'$.result') IS NULL
           AND NOT EXISTS (
             SELECT 1 FROM agent_items AS result
             WHERE result.session_id=agent_items.session_id AND result.run_id=agent_items.run_id
               AND result.kind='tool_result'
               AND json_extract(result.payload_json,'$.toolCallId')=
                   json_extract(agent_items.payload_json,'$.toolCallId'))
         RETURNING CASE WHEN json_type(payload_json,'$.toolCallId')='text'
                   THEN json_extract(payload_json,'$.toolCallId') END AS call_id",
    )
    .bind(&frame.session_id)
    .bind(&frame.run_id)
    .fetch_all(&mut *tx)
    .await?;
    if row.get::<Option<String>, _>("publication_run").as_deref() == Some(frame.run_id.as_str()) {
        tx.commit().await?;
        return Ok(None);
    }
    let completed_at: Option<String> = row.get("completed_at");
    let mut data = if status == "failed" || status == "interrupted" {
        let error: Option<String> = row.get("error_message");
        let stored_code: Option<String> = row.get("error_code");
        let category = if changed > 0 {
            field("category")
                .filter(|value| matches!(*value, "tool" | "provider" | "runtime" | "context"))
                .unwrap_or("runtime")
        } else {
            "runtime"
        };
        let retryable = changed == 0
            || params
                .and_then(|value| value.get("retryable"))
                .and_then(Value::as_bool)
                .unwrap_or(true);
        json!({"message": sanitize(error.as_deref().unwrap_or("Agent run failed.")),
            "code": stored_code.as_deref().unwrap_or("agent_runtime_failed"),
            "category": category, "retryable": retryable})
    } else if status == proposed {
        frame.params.clone().unwrap_or_else(|| json!({}))
    } else {
        json!({})
    };
    // Settlement authority comes only from the persisted calls above, never
    // from an incoming CLI frame or a broker's separate approval UUID.
    let object = data.as_object_mut().ok_or_else(|| {
        AppError::new(
            "agent_terminal_invalid",
            "Terminal event data must be an object.",
        )
    })?;
    object.remove("interruptedCallIds");
    let mut interrupted_call_ids = interrupted
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("call_id").ok())
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
    interrupted_call_ids.sort();
    interrupted_call_ids.dedup();
    if !interrupted_call_ids.is_empty() {
        object.insert("interruptedCallIds".into(), json!(interrupted_call_ids));
    }
    data["completedAt"] = json!(completed_at);
    let sequence = frame
        .sequence
        .max(row.get::<i64, _>("last_sequence").saturating_add(1));
    let event_id = frame
        .event_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    query(
        "UPDATE agent_runs SET last_sequence=?,
            run_config_json=json_set(COALESCE(run_config_json,'{}'),'$.clovyTerminalEvent',json(?))
         WHERE id=? AND session_id=?",
    )
    .bind(sequence)
    .bind(
        json!({"runId":frame.run_id,"eventId":event_id,"method":method,"sequence":sequence})
            .to_string(),
    )
    .bind(&frame.run_id)
    .bind(&frame.session_id)
    .execute(&mut *tx)
    .await?;
    if changed > 0 {
        query(
            "UPDATE agent_sessions SET status=?,updated_at=?,last_error=? WHERE id=?
             AND NOT EXISTS (SELECT 1 FROM agent_runs WHERE session_id=?
                             AND status IN ('queued','running','waiting_for_user'))",
        )
        .bind(if status == "failed" { "failed" } else { "idle" })
        .bind(&now)
        .bind((status == "failed").then_some(message.as_str()))
        .bind(&frame.session_id)
        .bind(&frame.session_id)
        .execute(&mut *tx)
        .await?;
    }
    if method == "run.failed" {
        let item_sequence: i64 = query(
            "INSERT INTO agent_items(id,session_id,run_id,sequence,kind,payload_json,external_id,created_at)
             SELECT ?,?,?,COALESCE(MAX(sequence),-1)+1,'error',?,?,? FROM agent_items WHERE session_id=?
             RETURNING sequence",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&frame.session_id)
        .bind(&frame.run_id)
        .bind(data.to_string())
        .bind(&event_id)
        .bind(&now)
        .bind(&frame.session_id)
        .fetch_one(&mut *tx)
        .await?
        .get("sequence");
        data["itemSequence"] = json!(item_sequence);
    }
    tx.commit().await?;
    Ok(Some(TerminalEvent {
        method: method.into(),
        event_id,
        sequence,
        data,
    }))
}

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
