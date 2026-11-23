//! The resolution path for CLI items: `resolve_agent_interruption` (the
//! command the cards already call) hands an item with `engine: "cli"` here
//! instead of resuming the sidecar.

use super::{expired_error, store, ApprovalBroker, Decision};
use crate::agent_sessions::{AgentRepository, AgentRunDto};
use crate::domain::types::AppError;
use serde_json::Value;
use sqlx_sqlite::SqlitePool;
use tauri::{AppHandle, Manager};

/// Resolves `interruption_id` of `run_id` through the broker when the
/// broker owns it. `Ok(None)` means the item is not a CLI item: the caller
/// continues with the native path.
pub async fn resolve_cli_interruption(
    app: &AppHandle,
    run_id: &str,
    interruption_id: &str,
    resolution: &Value,
) -> Result<Option<AgentRunDto>, AppError> {
    let pool = crate::commands::repositories(app).await?.pool;
    let Some(broker) = app.try_state::<ApprovalBroker>() else {
        // Without a broker nothing can be waiting on a CLI item.
        return match store::find_cli_interruption(&pool, run_id, interruption_id).await? {
            Some(_) => Err(expired_error()),
            None => Ok(None),
        };
    };
    if !resolve(&broker, &pool, run_id, interruption_id, resolution).await? {
        return Ok(None);
    }
    Ok(Some(AgentRepository::new(pool).get_run(run_id).await?))
}

/// Applies one resolution to a CLI item. Returns `false` when the item is not
/// a CLI item. A second resolution of the same item, or one for an item
/// nobody waits on any more, fails with `agent_interruption_expired`.
pub(crate) async fn resolve(
    broker: &ApprovalBroker,
    pool: &SqlitePool,
    run_id: &str,
    interruption_id: &str,
    resolution: &Value,
) -> Result<bool, AppError> {
    let Some(item) = store::find_cli_interruption(pool, run_id, interruption_id).await? else {
        return Ok(false);
    };
    if item.payload.get("status").and_then(Value::as_str) != Some("pending") {
        return Err(expired_error());
    }
    let decision = decision_for(&item.payload, resolution)?;
    broker.decide(pool, interruption_id, decision).await?;
    tracing::info!(
        interruption_id,
        stored_session_id = %item.session_id,
        "resolved CLI approval request"
    );
    Ok(true)
}

/// Validates a resolution against the pending item, like the native path:
/// the kinds must match, an approval is `once` or `deny`, and a
/// clarification carries `answer`. Standing scopes are not supported.
fn decision_for(item: &Value, resolution: &Value) -> Result<Decision, AppError> {
    let item_kind = item
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("approval");
    let resolution_kind = resolution
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            AppError::new(
                "agent_interruption_resolution_invalid",
                "The interruption response kind is required.",
            )
        })?;
    if resolution_kind != item_kind {
        return Err(AppError::new(
            "agent_interruption_resolution_invalid",
            "The interruption response does not match the pending request.",
        ));
    }
    match item_kind {
        "approval" => match resolution.get("choice").and_then(Value::as_str) {
            Some("once") => Ok(Decision::Approve),
            Some("deny") => Ok(Decision::Deny),
            _ => Err(invalid()),
        },
        "clarification" => resolution
            .get("answer")
            .and_then(Value::as_str)
            .map(|answer| Decision::Answer(answer.to_string()))
            .ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

fn invalid() -> AppError {
    AppError::new(
        "agent_interruption_resolution_invalid",
        "The interruption response is invalid.",
    )
}
