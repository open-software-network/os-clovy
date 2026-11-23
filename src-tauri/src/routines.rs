//! Clovy-owned Routine persistence and dispatch.
//!
//! A Routine is a durable schedule and policy. A `routine_runs` row is first
//! claimed transactionally, then linked to the Clovy-owned agent session/run
//! created for that execution. The claim is the single-active-run boundary: a
//! second scheduler tick, a manual trigger, or a restarted app cannot start a
//! duplicate while the original claim is live.

use crate::{
    agent_sessions::{AgentItemPayload, AgentRepository, AgentSafetyMode, MessagePayload},
    browser_broker::{BrowserBroker, RoutineBrowserGrant},
    commands::repositories,
    db::repositories::RoutineBrowserGrantRecord,
    domain::types::AppError,
};
use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::{query::query, query_scalar::query_scalar, row::Row, transaction::Transaction};
use sqlx_sqlite::{Sqlite, SqlitePool};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

type BoundRoutineTurn<'a> = Option<(
    &'a str,
    &'a str,
    &'a str,
    &'a crate::domain::turn_authority::TurnAuthority,
)>;

const CLAIM_STALE_AFTER_MINUTES: i64 = 10;
const SCHEDULER_TICK_SECONDS: u64 = 30;
const ROUTINE_SELECT_BY_ID_SQL: &str = "SELECT id, legacy_job_id, name, prompt, schedule, timezone, repeat, deliver, model, safety_mode, state, enabled, created_at, updated_at, next_run_at, last_run_at, last_status, last_error, last_delivery_error, metadata_json FROM routines WHERE id = ?";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRoutineDto {
    /// Stable Clovy routine id. Legacy imports may deliberately preserve the
    /// retired Hermes job id here, while also storing it in `legacy_job_id`.
    pub id: String,
    pub legacy_job_id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub schedule: String,
    pub timezone: String,
    pub repeat: String,
    pub deliver: String,
    pub model: String,
    pub safety_mode: AgentSafetyMode,
    pub state: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
    pub next_run_at: Option<String>,
    pub last_run_at: Option<String>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
    pub last_delivery_error: Option<String>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRoutineRunDto {
    pub id: String,
    pub routine_id: String,
    pub agent_session_id: Option<String>,
    pub agent_run_id: Option<String>,
    pub trigger_kind: String,
    pub status: String,
    pub scheduled_for: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    /// Model snapshotted from the routine or current generation selection at claim time.
    pub model: String,
    pub safety_mode: AgentSafetyMode,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRoutineRequest {
    pub id: Option<String>,
    pub legacy_job_id: Option<String>,
    pub name: Option<String>,
    pub prompt: String,
    pub schedule: String,
    #[serde(default = "utc_timezone")]
    pub timezone: String,
    pub repeat: Option<String>,
    pub deliver: Option<String>,
    #[serde(default = "auto_model")]
    pub model: String,
    #[serde(default)]
    pub safety_mode: Option<AgentSafetyMode>,
    pub state: Option<String>,
    pub enabled: Option<bool>,
    #[serde(default)]
    pub metadata: Value,
    pub enabled_toolsets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentRoutineRequest {
    pub routine_id: String,
    pub name: Option<String>,
    pub prompt: Option<String>,
    pub schedule: Option<String>,
    pub timezone: Option<String>,
    pub repeat: Option<String>,
    pub deliver: Option<String>,
    pub model: Option<String>,
    pub safety_mode: Option<AgentSafetyMode>,
    pub state: Option<String>,
    pub enabled: Option<bool>,
    pub metadata: Option<Value>,
    pub enabled_toolsets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineIdRequest {
    pub routine_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAgentRoutineRunsRequest {
    pub routine_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineBrowserAccessStatus {
    pub enabled: bool,
    pub server_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRoutineBrowserAccessRequest {
    pub job_id: String,
    pub enabled: bool,
}

fn utc_timezone() -> String {
    "UTC".into()
}
fn auto_model() -> String {
    "auto".into()
}

fn app_error(error: sqlx::Error) -> AppError {
    AppError::new("routine_storage_failed", error.to_string())
}

fn validate_state(state: &str) -> Result<(), AppError> {
    if matches!(state, "scheduled" | "paused" | "completed" | "needs_review") {
        Ok(())
    } else {
        Err(AppError::new(
            "routine_state_invalid",
            "Unknown routine state.",
        ))
    }
}

fn resolve_routine_model(
    stored_model: &str,
    installed: &[crate::llm::cli::CliKind],
) -> Result<String, AppError> {
    crate::llm::registry::resolve_unattended_cli(stored_model, |kind| installed.contains(&kind))
        .map(crate::chat_engine::engine_model_id)
}

async fn guard_routine_turn(
    transaction: &mut Transaction<'_, Sqlite>,
    turn: BoundRoutineTurn<'_>,
) -> Result<(), AppError> {
    if let Some((profile, session, run, authority)) = turn {
        crate::db::repositories::guard_bound_turn(transaction, profile, session, run, authority)
            .await?;
    }
    Ok(())
}

/// Create a durable Routine. The first next run is derived before insertion so
/// the scheduler can claim it without re-parsing creation data.
pub async fn create(
    pool: &SqlitePool,
    request: CreateAgentRoutineRequest,
) -> Result<AgentRoutineDto, AppError> {
    create_for_turn(pool, request, None).await
}

pub(crate) async fn create_for_turn(
    pool: &SqlitePool,
    request: CreateAgentRoutineRequest,
    turn: BoundRoutineTurn<'_>,
) -> Result<AgentRoutineDto, AppError> {
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    guard_routine_turn(&mut transaction, turn).await?;
    let name = request
        .name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| prompt_title(&request.prompt));
    let prompt = request.prompt.trim();
    let schedule = request.schedule.trim();
    if prompt.is_empty() || schedule.is_empty() {
        return Err(AppError::new(
            "invalid_arguments",
            "A routine needs a name, instructions, and schedule.",
        ));
    }
    let state = request.state.unwrap_or_else(|| "scheduled".into());
    validate_state(&state)?;
    let enabled = request.enabled.unwrap_or(state == "scheduled");
    let safety_mode = request.safety_mode.unwrap_or(AgentSafetyMode::Sandboxed);
    let timezone = nonempty(request.timezone, "UTC");
    let timestamp = crate::db::repositories::timestamp();
    let next = next_run_after(schedule, &timezone, Utc::now())?;
    let next_run_at = if enabled && state == "scheduled" {
        next.map(format_time)
    } else {
        None
    };
    let id = request.id.unwrap_or_else(|| Uuid::new_v4().to_string());
    let mut metadata = request.metadata;
    set_enabled_toolsets(
        &mut metadata,
        Some(request.enabled_toolsets.unwrap_or_default()),
    );
    query("INSERT INTO routines (id, legacy_job_id, name, prompt, schedule, timezone, repeat, deliver, model, safety_mode, state, enabled, created_at, updated_at, next_run_at, metadata_json, tool_catalog_version) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)")
        .bind(&id).bind(request.legacy_job_id).bind(name).bind(prompt).bind(schedule)
        .bind(timezone).bind(request.repeat.unwrap_or_else(|| "forever".into()))
        .bind(request.deliver.unwrap_or_else(|| "local".into())).bind(nonempty(request.model, "auto"))
        .bind(safety_mode.as_db()).bind(&state).bind(enabled).bind(&timestamp).bind(&timestamp)
        .bind(next_run_at).bind(metadata_json(metadata)).execute(&mut *transaction).await.map_err(app_error)?;
    let result = get_in_transaction(&mut transaction, &id).await?;
    transaction.commit().await.map_err(app_error)?;
    Ok(result)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<AgentRoutineDto>, AppError> {
    query("SELECT id, legacy_job_id, name, prompt, schedule, timezone, repeat, deliver, model, safety_mode, state, enabled, created_at, updated_at, next_run_at, last_run_at, last_status, last_error, last_delivery_error, metadata_json FROM routines ORDER BY updated_at DESC, id ASC")
        .fetch_all(pool).await.map_err(app_error).map(|rows| rows.into_iter().map(routine_from_row).collect())
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<AgentRoutineDto, AppError> {
    query(ROUTINE_SELECT_BY_ID_SQL)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|error| {
            if matches!(error, sqlx::Error::RowNotFound) {
                AppError::new("routine_not_found", "Routine was not found.")
            } else {
                app_error(error)
            }
        })
        .map(routine_from_row)
}

async fn get_in_transaction(
    transaction: &mut Transaction<'_, Sqlite>,
    id: &str,
) -> Result<AgentRoutineDto, AppError> {
    query(ROUTINE_SELECT_BY_ID_SQL)
        .bind(id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| {
            if matches!(error, sqlx::Error::RowNotFound) {
                AppError::new("routine_not_found", "Routine was not found.")
            } else {
                app_error(error)
            }
        })
        .map(routine_from_row)
}

pub async fn update(
    pool: &SqlitePool,
    request: UpdateAgentRoutineRequest,
) -> Result<AgentRoutineDto, AppError> {
    update_for_turn(pool, request, None).await
}

pub(crate) async fn update_for_turn(
    pool: &SqlitePool,
    request: UpdateAgentRoutineRequest,
    turn: BoundRoutineTurn<'_>,
) -> Result<AgentRoutineDto, AppError> {
    // Routine edits derive state and metadata from the stored row. Serialize
    // that read with terminal claim release so a stale edit cannot overwrite
    // finite-repeat progress or re-enable a just-completed routine.
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    guard_routine_turn(&mut transaction, turn).await?;
    let current = get_in_transaction(&mut transaction, &request.routine_id).await?;
    let routine_id = request.routine_id;
    let state = request.state.unwrap_or(current.state);
    validate_state(&state)?;
    let enabled = request.enabled.unwrap_or(current.enabled);
    if requires_legacy_execution_review(&current.metadata) && state == "scheduled" && enabled {
        return Err(AppError::new(
            "routine_legacy_execution_review_required",
            "This imported routine contains legacy execution settings. Create a new routine after reviewing its script and safety mode.",
        ));
    }
    let schedule = request.schedule.unwrap_or(current.schedule);
    let timezone = request.timezone.unwrap_or(current.timezone);
    if schedule.trim().is_empty() {
        return Err(AppError::new(
            "invalid_arguments",
            "A routine schedule is required.",
        ));
    }
    let next = next_run_after(&schedule, &timezone, Utc::now())?;
    if request
        .prompt
        .as_ref()
        .is_some_and(|prompt| prompt.trim().is_empty())
        || request
            .name
            .as_ref()
            .is_some_and(|name| name.trim().is_empty())
    {
        return Err(AppError::new(
            "invalid_arguments",
            "Routine name and instructions must not be empty.",
        ));
    }
    let next_run_at = if enabled && state == "scheduled" {
        next.map(format_time)
    } else {
        None
    };
    let mut metadata = request.metadata.unwrap_or(current.metadata);
    let tool_catalog_updated = request.enabled_toolsets.is_some();
    set_enabled_toolsets(&mut metadata, request.enabled_toolsets);
    query("UPDATE routines SET name = ?, prompt = ?, schedule = ?, timezone = ?, repeat = ?, deliver = ?, model = ?, safety_mode = ?, state = ?, enabled = ?, updated_at = ?, next_run_at = ?, metadata_json = ?, tool_catalog_version = CASE WHEN ? THEN 1 ELSE tool_catalog_version END WHERE id = ?")
        .bind(request.name.unwrap_or(current.name).trim()).bind(request.prompt.unwrap_or(current.prompt).trim())
        .bind(&schedule).bind(timezone).bind(request.repeat.unwrap_or(current.repeat))
        .bind(request.deliver.unwrap_or(current.deliver)).bind(request.model.unwrap_or(current.model))
        .bind(request.safety_mode.unwrap_or(current.safety_mode).as_db()).bind(&state).bind(enabled).bind(crate::db::repositories::timestamp())
        .bind(next_run_at).bind(metadata_json(metadata)).bind(tool_catalog_updated)
        .bind(&routine_id)
        .execute(&mut *transaction).await.map_err(app_error)?;
    transaction.commit().await.map_err(app_error)?;
    get(pool, &routine_id).await
}

pub async fn pause(pool: &SqlitePool, id: &str) -> Result<AgentRoutineDto, AppError> {
    pause_for_turn(pool, id, None).await
}

pub(crate) async fn pause_for_turn(
    pool: &SqlitePool,
    id: &str,
    turn: BoundRoutineTurn<'_>,
) -> Result<AgentRoutineDto, AppError> {
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    guard_routine_turn(&mut transaction, turn).await?;
    // Pausing stops future scheduling but does not cancel a claimed run. Keep
    // its claim association until terminal projection records status and
    // repeat progress, then releases the claim transactionally.
    let changed = query("UPDATE routines SET state = 'paused', enabled = 0, next_run_at = NULL, updated_at = ? WHERE id = ?")
        .bind(crate::db::repositories::timestamp()).bind(id).execute(&mut *transaction).await.map_err(app_error)?;
    if changed.rows_affected() == 0 {
        return Err(AppError::new("routine_not_found", "Routine was not found."));
    }
    let result = get_in_transaction(&mut transaction, id).await?;
    transaction.commit().await.map_err(app_error)?;
    Ok(result)
}

pub async fn resume(pool: &SqlitePool, id: &str) -> Result<AgentRoutineDto, AppError> {
    resume_for_turn(pool, id, None).await
}

pub(crate) async fn resume_for_turn(
    pool: &SqlitePool,
    id: &str,
    turn: BoundRoutineTurn<'_>,
) -> Result<AgentRoutineDto, AppError> {
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    guard_routine_turn(&mut transaction, turn).await?;
    let current = get_in_transaction(&mut transaction, id).await?;
    if requires_legacy_execution_review(&current.metadata) {
        return Err(AppError::new(
            "routine_legacy_execution_review_required",
            "This imported routine contains legacy execution settings. Create a new routine after reviewing its script and safety mode.",
        ));
    }
    let next = next_run_after(&current.schedule, &current.timezone, Utc::now())?.map(format_time);
    query("UPDATE routines SET state = 'scheduled', enabled = 1, next_run_at = ?, updated_at = ? WHERE id = ?")
        .bind(next).bind(crate::db::repositories::timestamp()).bind(id).execute(&mut *transaction).await.map_err(app_error)?;
    transaction.commit().await.map_err(app_error)?;
    get(pool, id).await
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<(), AppError> {
    delete_for_turn(pool, id, None).await
}

pub(crate) async fn delete_for_turn(
    pool: &SqlitePool,
    id: &str,
    turn: BoundRoutineTurn<'_>,
) -> Result<(), AppError> {
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    guard_routine_turn(&mut transaction, turn).await?;
    let deleted = query("DELETE FROM routines WHERE id = ?")
        .bind(id)
        .execute(&mut *transaction)
        .await
        .map_err(app_error)?;
    if deleted.rows_affected() == 0 {
        Err(AppError::new("routine_not_found", "Routine was not found."))
    } else {
        transaction.commit().await.map_err(app_error)?;
        Ok(())
    }
}

pub async fn list_runs(
    pool: &SqlitePool,
    routine_id: Option<&str>,
) -> Result<Vec<AgentRoutineRunDto>, AppError> {
    let rows = if let Some(routine_id) = routine_id {
        query("SELECT id, routine_id, agent_session_id, agent_run_id, trigger_kind, status, scheduled_for, started_at, completed_at, model, safety_mode, error_code, error_message, created_at, updated_at FROM routine_runs WHERE routine_id = ? ORDER BY created_at DESC, id DESC")
            .bind(routine_id).fetch_all(pool).await.map_err(app_error)?
    } else {
        query("SELECT id, routine_id, agent_session_id, agent_run_id, trigger_kind, status, scheduled_for, started_at, completed_at, model, safety_mode, error_code, error_message, created_at, updated_at FROM routine_runs ORDER BY created_at DESC, id DESC")
            .fetch_all(pool).await.map_err(app_error)?
    };
    Ok(rows.into_iter().map(run_from_row).collect())
}

/// Bounded execution history with a deterministic total order across ties.
pub async fn list_runs_page(
    pool: &SqlitePool,
    routine_id: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<AgentRoutineRunDto>, AppError> {
    let rows = query("SELECT id, routine_id, agent_session_id, agent_run_id, trigger_kind, status, scheduled_for, started_at, completed_at, model, safety_mode, error_code, error_message, created_at, updated_at FROM routine_runs WHERE (? IS NULL OR routine_id = ?) ORDER BY created_at DESC, id DESC LIMIT ? OFFSET ?")
        .bind(routine_id).bind(routine_id).bind(limit.clamp(1, 101)).bind(offset.max(0)).fetch_all(pool).await.map_err(app_error)?;
    Ok(rows.into_iter().map(run_from_row).collect())
}

/// A failed claim is never represented as a successful run. Eligibility is
/// re-read because a scheduler may win between approval and manual dispatch.
pub async fn trigger_blocked_error(pool: &SqlitePool, id: &str) -> Result<AppError, AppError> {
    let routine = get(pool, id).await?;
    let active: i64 = query_scalar("SELECT EXISTS (SELECT 1 FROM routines WHERE id = ? AND claim_token IS NOT NULL) OR EXISTS (SELECT 1 FROM routine_runs WHERE routine_id = ? AND status IN ('queued','running','waiting_for_user'))")
        .bind(id).bind(id).fetch_one(pool).await.map_err(app_error)?;
    Ok(if active != 0 {
        AppError::new("routine_busy", "This routine already has an active run.")
    } else {
        AppError::new(
            "routine_blocked",
            format!(
                "This routine cannot start in state {} (enabled: {}).",
                routine.state, routine.enabled
            ),
        )
    })
}

/// Atomically claim an immediate or due execution. It deliberately does not
/// start the harness: callers can persist/reconcile the claim before external
/// work begins, and a crash leaves one recoverable queued row rather than two
/// model calls.
pub async fn claim(
    pool: &SqlitePool,
    routine_id: &str,
    trigger_kind: &str,
    require_due: bool,
) -> Result<Option<Claim>, AppError> {
    claim_with_connector_guard(pool, routine_id, trigger_kind, require_due, None).await
}

struct ConnectorClaimGuard<'a> {
    trigger_id: &'a str,
    kind: &'a str,
    account_id: &'a str,
}

async fn claim_with_connector_guard(
    pool: &SqlitePool,
    routine_id: &str,
    trigger_kind: &str,
    require_due: bool,
    connector: Option<ConnectorClaimGuard<'_>>,
) -> Result<Option<Claim>, AppError> {
    let env = crate::llm::shell_env::login_env().await;
    claim_with_connector_guard_for_turn(
        pool,
        routine_id,
        trigger_kind,
        require_due,
        connector,
        None,
        &env,
    )
    .await
}

async fn claim_with_connector_guard_for_turn(
    pool: &SqlitePool,
    routine_id: &str,
    trigger_kind: &str,
    require_due: bool,
    connector: Option<ConnectorClaimGuard<'_>>,
    turn: BoundRoutineTurn<'_>,
    env: &crate::llm::shell_env::LoginEnv,
) -> Result<Option<Claim>, AppError> {
    // Resolve executable presence before acquiring the SQLite writer. No CLI
    // process or synchronous login-shell resolution occurs inside the claim.
    let installed: Vec<_> = crate::llm::registry::UNATTENDED_CLIS
        .into_iter()
        .filter(|kind| env.which(kind.id()).is_some())
        .collect();
    // Serialize the eligibility read with the claim write. A deferred
    // transaction can fail with SQLITE_BUSY_SNAPSHOT when a scheduler,
    // connector, and manual trigger race after reading the same free row.
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    guard_routine_turn(&mut transaction, turn).await?;
    let trigger_id = connector.as_ref().map(|guard| guard.trigger_id);
    let trigger_guard_kind = connector.as_ref().map(|guard| guard.kind);
    let trigger_account_id = connector.as_ref().map(|guard| guard.account_id);
    let routine = query("SELECT id, name, prompt, schedule, timezone, model, safety_mode, next_run_at, metadata_json, tool_catalog_version FROM routines WHERE id = ? AND state = 'scheduled' AND enabled = 1 AND claim_token IS NULL AND NOT EXISTS (SELECT 1 FROM routine_runs active WHERE active.routine_id = routines.id AND active.status IN ('queued', 'running', 'waiting_for_user')) AND (? IS NULL OR EXISTS (SELECT 1 FROM connector_triggers trigger WHERE trigger.id = ? AND trigger.job_id = routines.id AND trigger.kind = ? AND trigger.account_id = ?))")
        .bind(routine_id).bind(trigger_id).bind(trigger_id).bind(trigger_guard_kind).bind(trigger_account_id)
        .fetch_optional(&mut *transaction).await.map_err(app_error)?;
    let Some(routine) = routine else {
        transaction.commit().await.map_err(app_error)?;
        return Ok(None);
    };
    let scheduled_for: Option<String> = routine.get("next_run_at");
    if require_due
        && scheduled_for
            .as_deref()
            .is_some_and(|value| parse_time(value).is_ok_and(|time| time > Utc::now()))
    {
        transaction.commit().await.map_err(app_error)?;
        return Ok(None);
    }
    let token = Uuid::new_v4().to_string();
    let run_id = Uuid::new_v4().to_string();
    let timestamp = crate::db::repositories::timestamp();
    let stored_model: String = routine.get("model");
    let model = resolve_routine_model(&stored_model, &installed)?;
    let safety_mode = AgentSafetyMode::from(routine.get::<String, _>("safety_mode").as_str());
    let metadata = serde_json::from_str::<Value>(&routine.get::<String, _>("metadata_json"))
        .unwrap_or_else(|_| json!({}));
    let enabled_toolsets = enabled_toolsets_from_metadata(
        &metadata,
        routine.get::<i64, _>("tool_catalog_version") == 0,
    );
    let claimed = query("UPDATE routines SET claim_token = ?, claimed_at = ?, updated_at = ? WHERE id = ? AND claim_token IS NULL AND state = 'scheduled' AND enabled = 1")
        .bind(&token).bind(&timestamp).bind(&timestamp).bind(routine_id).execute(&mut *transaction).await.map_err(app_error)?;
    if claimed.rows_affected() != 1 {
        transaction.commit().await.map_err(app_error)?;
        return Ok(None);
    }
    query("INSERT INTO routine_runs (id, routine_id, claim_token, trigger_kind, status, scheduled_for, model, safety_mode, created_at, updated_at) VALUES (?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?)")
        .bind(&run_id).bind(routine_id).bind(&token).bind(trigger_kind).bind(scheduled_for).bind(&model).bind(safety_mode.as_db()).bind(&timestamp).bind(&timestamp)
        .execute(&mut *transaction).await.map_err(app_error)?;
    transaction.commit().await.map_err(app_error)?;
    Ok(Some(Claim {
        routine_id: routine.get("id"),
        routine_name: routine.get("name"),
        prompt: routine.get("prompt"),
        schedule: routine.get("schedule"),
        timezone: routine.get("timezone"),
        trigger_kind: trigger_kind.to_string(),
        model,
        safety_mode,
        enabled_toolsets,
        token,
        routine_run_id: run_id,
    }))
}

#[cfg(test)]
mod bound_claim_tests {
    use super::*;
    use crate::{approval_broker::testing::TestBroker, llm::cli::CliKind};
    #[tokio::test]
    async fn manual_claim_rechecks_stopped_caller_before_admission() {
        let broker = TestBroker::new([]).await;
        let binding = broker.turn(CliKind::Claude, true).await;
        let routine = create(
            broker.pool(),
            serde_json::from_value(json!({"prompt":"Keep", "schedule":"every 1d"})).unwrap(),
        )
        .await
        .unwrap();
        query("UPDATE agent_runs SET status='cancelled' WHERE id=?")
            .bind(&binding.run_id)
            .execute(broker.pool())
            .await
            .unwrap();
        let env = crate::llm::shell_env::LoginEnv::from_vars(Default::default());
        let result = claim_with_connector_guard_for_turn(
            broker.pool(),
            &routine.id,
            "manual",
            false,
            None,
            Some((
                "default",
                &binding.session_id,
                &binding.run_id,
                &binding.authority,
            )),
            &env,
        )
        .await;
        assert_eq!(result.unwrap_err().code, "turn_expired");
        assert!(list_runs(broker.pool(), Some(&routine.id))
            .await
            .unwrap()
            .is_empty());
        // The refused admission leaves the claim available to the scheduler.
        let (_directory, env) = super::selection_tests::environment(&[CliKind::Claude]);
        assert!(claim_with_connector_guard_for_turn(
            broker.pool(),
            &routine.id,
            "schedule",
            false,
            None,
            None,
            &env
        )
        .await
        .unwrap()
        .is_some());
    }
}

/// Claim every due routine. The caller should then call `start_claim` for each
/// result; keeping the operations separate makes scheduler tests deterministic
/// and allows bounded concurrent dispatch at the app integration layer.
pub async fn claim_due(pool: &SqlitePool) -> Result<Vec<Claim>, AppError> {
    let env = crate::llm::shell_env::login_env().await;
    claim_due_with_env(pool, &env).await
}

async fn claim_due_with_env(
    pool: &SqlitePool,
    env: &crate::llm::shell_env::LoginEnv,
) -> Result<Vec<Claim>, AppError> {
    let ids: Vec<String> = query("SELECT id FROM routines WHERE state = 'scheduled' AND enabled = 1 AND claim_token IS NULL AND next_run_at IS NOT NULL AND next_run_at <= ? ORDER BY next_run_at ASC, id ASC")
        .bind(crate::db::repositories::timestamp()).fetch_all(pool).await.map_err(app_error)?.into_iter().map(|row| row.get("id")).collect();
    let mut claims = Vec::new();
    for id in ids {
        match claim_with_connector_guard_for_turn(pool, &id, "schedule", true, None, None, env)
            .await
        {
            Ok(Some(claim)) => claims.push(claim),
            Ok(None) => {}
            Err(error)
                if matches!(
                    error.code.as_str(),
                    "routine_engine_unavailable"
                        | "routine_engine_not_allowed"
                        | "llm_cli_not_installed"
                ) =>
            {
                // A refusal belongs to this routine. Keep earlier committed
                // claims dispatchable and continue the remaining due work.
                tracing::warn!(routine_id = %id, error_code = %error.code, error = %error.message,
                    "routine engine selection refused");
            }
            Err(error) => return Err(error),
        }
    }
    Ok(claims)
}

async fn scheduler_tick(pool: &SqlitePool) -> Result<Vec<Claim>, AppError> {
    // The terminal event path is best effort after the agent run is durable.
    // Reconcile on every tick so a transient projection failure cannot retain
    // a completed Routine's single-flight claim until the next app restart.
    reconcile(pool).await?;
    claim_due(pool).await
}

/// Reconciles terminal agent runs and releases abandoned queued claims during
/// scheduler maintenance and restart recovery. A still-running agent run
/// remains claimed; an unstarted claim only becomes eligible again after a
/// bounded lease expiry.
pub async fn reconcile(pool: &SqlitePool) -> Result<(), AppError> {
    let timestamp = crate::db::repositories::timestamp();
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    query("UPDATE routine_runs SET status = (SELECT status FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), completed_at = COALESCE((SELECT completed_at FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), completed_at), error_code = COALESCE((SELECT error_code FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), error_code), error_message = COALESCE((SELECT error_message FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), error_message), updated_at = ? WHERE agent_run_id IS NOT NULL AND status IN ('running', 'waiting_for_user') AND EXISTS (SELECT 1 FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id AND agent_runs.status IN ('completed', 'cancelled', 'interrupted', 'failed'))")
        .bind(&timestamp).execute(&mut *transaction).await.map_err(app_error)?;
    let stale_before = format_time(Utc::now() - Duration::minutes(CLAIM_STALE_AFTER_MINUTES));
    query("UPDATE routine_runs SET status = 'interrupted', completed_at = ?, error_code = 'routine_claim_expired', error_message = 'Routine dispatch did not start before its claim expired.', updated_at = ? WHERE status = 'queued' AND agent_run_id IS NULL AND updated_at < ?")
        .bind(&timestamp).bind(&timestamp).bind(stale_before).execute(&mut *transaction).await.map_err(app_error)?;
    release_terminal_claims(&mut transaction, &timestamp, None).await?;
    transaction.commit().await.map_err(app_error)
}

async fn release_terminal_claims(
    transaction: &mut Transaction<'_, Sqlite>,
    timestamp: &str,
    agent_run_id: Option<&str>,
) -> Result<(), AppError> {
    // A terminal run releases exactly its matching routine claim. Advance the
    // schedule here rather than leaving a past `next_run_at`, which would
    // otherwise immediately re-run a completed routine on the next tick.
    let terminal = query("SELECT routines.id, routines.schedule, routines.timezone, routines.repeat, routines.metadata_json, routines.state, routines.enabled, routine_runs.claim_token, routine_runs.status, routine_runs.completed_at, routine_runs.error_message FROM routines JOIN routine_runs ON routine_runs.routine_id = routines.id AND routine_runs.claim_token = routines.claim_token WHERE routines.claim_token IS NOT NULL AND routine_runs.status IN ('completed', 'cancelled', 'interrupted', 'failed') AND (? IS NULL OR routine_runs.agent_run_id = ?)")
        .bind(agent_run_id)
        .bind(agent_run_id)
        .fetch_all(&mut **transaction).await.map_err(app_error)?;
    for row in terminal {
        let state: String = row.get("state");
        let enabled = row.get::<i64, _>("enabled") != 0;
        let mut metadata = parse_metadata(row.get::<String, _>("metadata_json"));
        let repeat_completed = advance_repeat(&row.get::<String, _>("repeat"), &mut metadata);
        let next_run_at = if state == "scheduled" && enabled && !repeat_completed {
            next_run_after(
                &row.get::<String, _>("schedule"),
                &row.get::<String, _>("timezone"),
                Utc::now(),
            )?
            .map(format_time)
        } else {
            None
        };
        let status: String = row.get("status");
        query("UPDATE routines SET claim_token = NULL, claimed_at = NULL, next_run_at = ?, state = CASE WHEN ? THEN 'completed' ELSE state END, enabled = CASE WHEN ? THEN 0 ELSE enabled END, metadata_json = ?, last_run_at = COALESCE(?, ?), last_status = ?, last_error = ?, updated_at = ? WHERE id = ? AND claim_token = ?")
            .bind(next_run_at)
            .bind(repeat_completed)
            .bind(repeat_completed)
            .bind(metadata_json(metadata))
            .bind(row.get::<Option<String>, _>("completed_at"))
            .bind(timestamp)
            .bind(if status == "completed" { "ok" } else { "error" })
            .bind(row.get::<Option<String>, _>("error_message"))
            .bind(timestamp)
            .bind(row.get::<String, _>("id"))
            .bind(row.get::<String, _>("claim_token"))
            .execute(&mut **transaction).await.map_err(app_error)?;
    }
    Ok(())
}

fn advance_repeat(repeat: &str, metadata: &mut Value) -> bool {
    let maximum = match repeat.trim().to_ascii_lowercase().as_str() {
        "forever" | "" => None,
        "once" => Some(1_u64),
        value => value.parse::<u64>().ok(),
    };
    let completed = metadata
        .get("repeatState")
        .and_then(|state| state.get("completed"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .saturating_add(1);
    if !metadata.is_object() {
        *metadata = json!({});
    }
    if !metadata["repeatState"].is_object() {
        metadata["repeatState"] = json!({});
    }
    metadata["repeatState"]["completed"] = json!(completed);
    maximum.is_some_and(|maximum| completed >= maximum)
}

/// Reconcile claims left by a previous desktop process before starting a new
/// scheduler. The runtime child is process-owned and cannot survive an app
/// restart, so an old `running`/`waiting_for_user` run must not retain the
/// single-flight lease forever. This is deliberately separate from `reconcile`
/// because calling it during normal scheduler ticks would interrupt live work.
pub async fn reconcile_after_restart(pool: &SqlitePool) -> Result<(), AppError> {
    crate::agent_sessions::AgentRepository::new(pool.clone())
        .reconcile_non_routine_runs_after_restart()
        .await
        .map_err(app_error)?;
    let timestamp = crate::db::repositories::timestamp();
    // The agent run is the source of truth at a process boundary. Mirroring
    // the active state first closes either crash window between the two normal
    // persistence writes: a stored waiting run stays resumable, while a stored
    // running run is interrupted below even if its routine projection was
    // still waiting.
    query(
        "UPDATE routine_runs
         SET status = (
           SELECT status FROM agent_runs
           WHERE agent_runs.id = routine_runs.agent_run_id
         ), updated_at = ?
         WHERE agent_run_id IS NOT NULL
           AND status IN ('running', 'waiting_for_user')
           AND EXISTS (
             SELECT 1 FROM agent_runs
             WHERE agent_runs.id = routine_runs.agent_run_id
               AND agent_runs.status IN ('queued', 'running', 'waiting_for_user')
           )",
    )
    .bind(&timestamp)
    .execute(pool)
    .await
    .map_err(app_error)?;
    // Waiting runs retain their serialized SDK state and routine claim so the
    // pending approval or clarification remains resumable after relaunch.
    // Ordinary running work cannot survive the process boundary and is
    // interrupted, which releases its claim through `reconcile`.
    query("UPDATE agent_runs SET status = 'interrupted', completed_at = COALESCE(completed_at, ?), error_code = COALESCE(error_code, 'routine_runtime_restarted'), error_message = COALESCE(error_message, 'Clovy restarted before this routine completed.'), updated_at = ? WHERE id IN (SELECT agent_run_id FROM routine_runs WHERE agent_run_id IS NOT NULL AND status IN ('queued', 'running'))")
        .bind(&timestamp).bind(&timestamp).execute(pool).await.map_err(app_error)?;
    query("UPDATE routine_runs SET status = 'interrupted', completed_at = COALESCE(completed_at, ?), error_code = COALESCE(error_code, 'routine_runtime_restarted'), error_message = COALESCE(error_message, 'Clovy restarted before this routine completed.'), updated_at = ? WHERE status IN ('queued', 'running')")
        .bind(&timestamp).bind(&timestamp).execute(pool).await.map_err(app_error)?;
    reconcile(pool).await
}

pub async fn mark_agent_run_waiting(pool: &SqlitePool, agent_run_id: &str) -> Result<(), AppError> {
    query(
        "UPDATE routine_runs
         SET status = 'waiting_for_user', updated_at = ?
         WHERE agent_run_id = ? AND status = 'running'",
    )
    .bind(crate::db::repositories::timestamp())
    .bind(agent_run_id)
    .execute(pool)
    .await
    .map_err(app_error)?;
    Ok(())
}

pub async fn mark_agent_run_resumed(pool: &SqlitePool, agent_run_id: &str) -> Result<(), AppError> {
    query(
        "UPDATE routine_runs
         SET status = 'running', updated_at = ?
         WHERE agent_run_id = ? AND status = 'waiting_for_user'",
    )
    .bind(crate::db::repositories::timestamp())
    .bind(agent_run_id)
    .execute(pool)
    .await
    .map_err(app_error)?;
    Ok(())
}

pub async fn mark_agent_run_terminal(
    pool: &SqlitePool,
    agent_run_id: &str,
) -> Result<(), AppError> {
    // Most terminal agent events are not Routine runs. Avoid taking SQLite's
    // RESERVED write lock for those unrelated chat and dictation completions.
    let is_routine_run: i64 = query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM routine_runs
             WHERE agent_run_id = ? AND status IN ('running', 'waiting_for_user')
         )",
    )
    .bind(agent_run_id)
    .fetch_one(pool)
    .await
    .map_err(app_error)?;
    if is_routine_run == 0 {
        return Ok(());
    }

    let timestamp = crate::db::repositories::timestamp();
    // Project the terminal run and release its routine claim under one write
    // reservation. A concurrent routine edit therefore lands wholly before or
    // after both updates and cannot erase the matching claim between them.
    let mut transaction = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(app_error)?;
    project_agent_run_terminal(&mut transaction, agent_run_id, &timestamp).await?;
    transaction.commit().await.map_err(app_error)
}

pub(crate) async fn project_agent_run_terminal(
    transaction: &mut Transaction<'_, Sqlite>,
    agent_run_id: &str,
    timestamp: &str,
) -> Result<(), AppError> {
    let changed = query("UPDATE routine_runs SET status = (SELECT status FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), completed_at = COALESCE((SELECT completed_at FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), completed_at), error_code = COALESCE((SELECT error_code FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), error_code), error_message = COALESCE((SELECT error_message FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id), error_message), updated_at = ? WHERE agent_run_id = ? AND status IN ('running', 'waiting_for_user') AND EXISTS (SELECT 1 FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id AND agent_runs.status IN ('completed', 'cancelled', 'interrupted', 'failed'))")
        .bind(timestamp)
        .bind(agent_run_id)
        .execute(&mut **transaction)
        .await
        .map_err(app_error)?;
    if changed.rows_affected() != 0 {
        release_terminal_claims(transaction, timestamp, Some(agent_run_id)).await?;
    }
    Ok(())
}

/// Start the single local scheduler for Clovy-owned routines. Claims are stored
/// before dispatch, so overlapping ticks, manual runs, and connector wakes all
/// share the same single-flight boundary.
pub fn start_scheduler(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let repositories = match repositories(&app).await {
            Ok(repositories) => repositories,
            Err(error) => {
                tracing::warn!(error_code = %error.code, "routine scheduler could not open storage");
                return;
            }
        };
        let pool = repositories.pool;
        if let Err(error) = reconcile_after_restart(&pool).await {
            tracing::warn!(error_code = %error.code, "routine restart reconciliation failed");
        }
        loop {
            match scheduler_tick(&pool).await {
                Ok(claims) => {
                    for claim in claims {
                        if let Err(error) = start_claim(&app, &pool, claim).await {
                            tracing::warn!(error_code = %error.code, "routine scheduled dispatch failed");
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(error_code = %error.code, "routine scheduler claim failed")
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(SCHEDULER_TICK_SECONDS)).await;
        }
    });
}

/// Starts a previously persisted claim with the unattended tool contract.
/// There is intentionally no Computer use, clarification, shell, write, or
/// approval-required tool in this set.
pub async fn start_claim(
    app: &AppHandle,
    pool: &SqlitePool,
    claim: Claim,
) -> Result<AgentRoutineRunDto, AppError> {
    let repository = AgentRepository::new(pool.clone());
    let session_id = Uuid::new_v4().to_string();
    let workspace = routine_workspace(app, &session_id)?;
    tokio::fs::create_dir_all(&workspace)
        .await
        .map_err(|error| AppError::new("routine_workspace_failed", error.to_string()))?;
    let timestamp = crate::db::repositories::timestamp();
    let mut created_agent_run_id = None;
    let cli =
        crate::chat_engine::cli_for_model(&claim.model).unwrap_or(crate::llm::cli::CliKind::Claude);
    let started = async {
        let run_id = persist_claim_run(
            pool,
            &repository,
            &claim,
            &session_id,
            &workspace,
            &timestamp,
            &mut created_agent_run_id,
        )
        .await?;
        start_cli_claim(
            app,
            &repository,
            &claim,
            cli,
            &session_id,
            &run_id,
            &workspace,
        )
        .await?;
        Ok::<_, AppError>(run_id)
    }
    .await;
    if let Err(error) = started {
        if let Some(run_id) = created_agent_run_id.as_deref() {
            if let Err(persist_error) = repository
                .update_run_status(
                    run_id,
                    "failed",
                    None,
                    None,
                    Some((&error.code, &error.message)),
                )
                .await
            {
                tracing::warn!(%persist_error, "failed to terminalize a routine agent run after dispatch failure");
            }
        }
        let next_run_at = next_run_after_dispatch_failure(&claim, Utc::now());
        record_dispatch_failure(pool, &claim, &error, next_run_at).await;
        return Err(error);
    }
    list_runs(pool, Some(&claim.routine_id))
        .await?
        .into_iter()
        .find(|run| run.id == claim.routine_run_id)
        .ok_or_else(|| AppError::new("routine_run_missing", "Routine run was not persisted."))
}

async fn record_dispatch_failure(
    pool: &SqlitePool,
    claim: &Claim,
    error: &AppError,
    next_run_at: Option<String>,
) {
    // A failed preparation may arrive after the event path already projected
    // Stop. Preserve that winner in both the run history and routine summary.
    let _ = query("UPDATE routine_runs SET status = 'failed', completed_at = ?, error_code = ?, error_message = ?, updated_at = ? WHERE id = ? AND status IN ('queued','running','waiting_for_user') AND NOT EXISTS (SELECT 1 FROM agent_runs WHERE agent_runs.id = routine_runs.agent_run_id AND agent_runs.status IN ('completed','cancelled','interrupted'))")
        .bind(crate::db::repositories::timestamp()).bind(&error.code).bind(&error.message).bind(crate::db::repositories::timestamp()).bind(&claim.routine_run_id).execute(pool).await;
    let _ = query("UPDATE routines SET claim_token = NULL, claimed_at = NULL, next_run_at = COALESCE(?, next_run_at), last_run_at = ?, last_status = 'error', last_error = ?, updated_at = ? WHERE id = ? AND claim_token = ? AND EXISTS (SELECT 1 FROM routine_runs rr WHERE rr.id = ? AND rr.routine_id = routines.id AND rr.status = 'failed')")
        .bind(next_run_at).bind(crate::db::repositories::timestamp()).bind(&error.message).bind(crate::db::repositories::timestamp()).bind(&claim.routine_id).bind(&claim.token).bind(&claim.routine_run_id).execute(pool).await;
}

fn next_run_after_dispatch_failure(claim: &Claim, after: DateTime<Utc>) -> Option<String> {
    (claim.trigger_kind == "schedule")
        .then(|| {
            next_run_after(&claim.schedule, &claim.timezone, after)
                .ok()
                .flatten()
                .unwrap_or_else(|| after + Duration::minutes(5))
        })
        .map(format_time)
}

pub async fn trigger_and_start(
    app: &AppHandle,
    pool: &SqlitePool,
    routine_id: &str,
) -> Result<Option<AgentRoutineRunDto>, AppError> {
    trigger_and_start_for_turn(app, pool, routine_id, None).await
}

pub(crate) async fn trigger_and_start_for_turn(
    app: &AppHandle,
    pool: &SqlitePool,
    routine_id: &str,
    turn: BoundRoutineTurn<'_>,
) -> Result<Option<AgentRoutineRunDto>, AppError> {
    reconcile(pool).await?;
    let env = crate::llm::shell_env::login_env().await;
    match claim_with_connector_guard_for_turn(pool, routine_id, "manual", false, None, turn, &env)
        .await?
    {
        Some(claim) => start_claim(app, pool, claim).await.map(Some),
        None => Ok(None),
    }
}

/// Shared entry point for connector event triggers. `false` means the routine
/// was paused, missing, or already active. Callers decide whether that is an
/// acknowledged wake or one that should remain pending for their event type.
pub async fn trigger_from_connector(
    app: &AppHandle,
    routine_id: &str,
    trigger_id: &str,
    kind: &str,
    account_id: &str,
) -> Result<bool, AppError> {
    let pool = repositories(app).await?.pool;
    reconcile(&pool).await?;
    let guard = ConnectorClaimGuard {
        trigger_id,
        kind,
        account_id,
    };
    match claim_with_connector_guard(&pool, routine_id, kind, false, Some(guard)).await? {
        Some(claim) => start_claim(app, &pool, claim).await.map(|_| true),
        None => Ok(false),
    }
}

/// The routine's session, its agent run, the routine prompt as the user
/// message, and the claim mapping: the same for both engines. The run id is
/// written to `created` as soon as the run exists, so a later failure can
/// terminalize it.
async fn persist_claim_run(
    pool: &SqlitePool,
    repository: &AgentRepository,
    claim: &Claim,
    session_id: &str,
    workspace: &std::path::Path,
    timestamp: &str,
    created: &mut Option<String>,
) -> Result<String, AppError> {
    query("INSERT INTO agent_sessions (id, title, status, model, safety_mode, workspace_path, source, created_at, updated_at) VALUES (?, ?, 'idle', ?, ?, ?, 'routine', ?, ?)")
        .bind(session_id).bind(&claim.routine_name).bind(&claim.model).bind(claim.safety_mode.as_db()).bind(workspace.to_string_lossy().as_ref()).bind(timestamp).bind(timestamp).execute(pool).await.map_err(app_error)?;
    let run = repository
        .create_run(session_id, &claim.model, Some("medium"))
        .await
        .map_err(app_error)?;
    *created = Some(run.id.clone());
    repository
        .append_item(
            session_id,
            Some(&run.id),
            0,
            &AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: claim.prompt.clone(),
                attachments: vec![],
            }),
            Some(&format!("routine:{}", claim.routine_run_id)),
        )
        .await
        .map_err(app_error)?;
    attach_run_mapping(
        pool,
        &claim.routine_run_id,
        &claim.token,
        session_id,
        &run.id,
        timestamp,
    )
    .await?;
    Ok(run.id)
}

/// What a routine's CLI turn receives: the routine prompt, framed so the CLI
/// knows nobody is there and which tools it has.
fn routine_cli_prompt(claim: &Claim) -> String {
    format!(
        "This is the Clovy routine \"{name}\", running unattended: nobody can answer questions or approve actions. You can read files, search and fetch the web, and use the Clovy MCP tools; you cannot run commands or change files. Do the task and give the result as your final answer.\n\n{prompt}",
        name = claim.routine_name,
        prompt = crate::chat_engine::with_note_reference_context(&claim.prompt),
    )
}

/// Runs a claimed routine on a CLI engine through the chat engine path
/// (`crate::chat_engine::start_cli_turn`) under the unattended policy. Its
/// events are persisted like a chat turn's, which projects the routine run's
/// end (`mark_agent_run_terminal`).
async fn start_cli_claim(
    app: &AppHandle,
    repository: &AgentRepository,
    claim: &Claim,
    kind: crate::llm::cli::CliKind,
    session_id: &str,
    run_id: &str,
    workspace: &std::path::Path,
) -> Result<(), AppError> {
    repository
        .set_run_config(
            run_id,
            &serde_json::json!({"workspace": workspace.to_string_lossy(), "surface": "routine"}),
        )
        .await?;
    let engines = app.state::<crate::chat_engine::ChatEngineHost>();
    let ticket = engines.register(run_id)?;
    let started = crate::chat_engine::start_cli_turn(
        app,
        repository.clone(),
        crate::chat_engine::CliTurnRequest {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            previous_run_id: None,
            kind,
            workspace: workspace.to_path_buf(),
            input: routine_cli_prompt(claim),
            images: Vec::new(),
            safety_mode: claim.safety_mode,
            attended: false,
            surface: "routine".into(),
            routine_tools: Some(
                crate::chat_engine::policy::RoutineBuiltinTools::from_toolsets(
                    &claim.enabled_toolsets,
                ),
            ),
        },
        ticket,
    )
    .await;
    crate::chat_engine::finish_cli_dispatch(app, repository, session_id, run_id, started).await
}

/// Attach a created agent session/run to the already-durable claim. Keeping
/// this as a small unit makes restart recovery and the mapping invariant easy
/// to test independently of the runtime host.
async fn attach_run_mapping(
    pool: &SqlitePool,
    routine_run_id: &str,
    claim_token: &str,
    session_id: &str,
    agent_run_id: &str,
    timestamp: &str,
) -> Result<(), AppError> {
    let changed = query("UPDATE routine_runs SET agent_session_id = ?, agent_run_id = ?, status = 'running', started_at = ?, updated_at = ? WHERE id = ? AND claim_token = ? AND status = 'queued'")
        .bind(session_id).bind(agent_run_id).bind(timestamp).bind(timestamp).bind(routine_run_id).bind(claim_token).execute(pool).await.map_err(app_error)?;
    if changed.rows_affected() == 1 {
        Ok(())
    } else {
        Err(AppError::new(
            "routine_claim_lost",
            "Routine claim is no longer active.",
        ))
    }
}

#[tauri::command]
pub async fn list_agent_routines(app: AppHandle) -> Result<Vec<AgentRoutineDto>, AppError> {
    list(&repositories(&app).await?.pool).await
}
#[tauri::command]
pub async fn create_agent_routine(
    app: AppHandle,
    request: CreateAgentRoutineRequest,
) -> Result<AgentRoutineDto, AppError> {
    create(&repositories(&app).await?.pool, request).await
}
#[tauri::command]
pub async fn update_agent_routine(
    app: AppHandle,
    request: UpdateAgentRoutineRequest,
) -> Result<AgentRoutineDto, AppError> {
    update(&repositories(&app).await?.pool, request).await
}
#[tauri::command]
pub async fn pause_agent_routine(
    app: AppHandle,
    routine_id: String,
) -> Result<AgentRoutineDto, AppError> {
    pause(&repositories(&app).await?.pool, &routine_id).await
}
#[tauri::command]
pub async fn resume_agent_routine(
    app: AppHandle,
    routine_id: String,
) -> Result<AgentRoutineDto, AppError> {
    resume(&repositories(&app).await?.pool, &routine_id).await
}
#[tauri::command]
pub async fn trigger_agent_routine(
    app: AppHandle,
    routine_id: String,
) -> Result<Option<AgentRoutineRunDto>, AppError> {
    trigger_and_start(&app, &repositories(&app).await?.pool, &routine_id).await
}
#[tauri::command]
pub async fn delete_agent_routine(app: AppHandle, routine_id: String) -> Result<(), AppError> {
    delete(&repositories(&app).await?.pool, &routine_id).await
}
#[tauri::command]
pub async fn list_agent_routine_runs(
    app: AppHandle,
    request: Option<ListAgentRoutineRunsRequest>,
) -> Result<Vec<AgentRoutineRunDto>, AppError> {
    list_runs(
        &repositories(&app).await?.pool,
        request.and_then(|value| value.routine_id).as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn routine_browser_access_get(
    app: AppHandle,
    job_id: String,
) -> Result<RoutineBrowserAccessStatus, AppError> {
    let grant = repositories(&app)
        .await?
        .routine_browser_grant(&job_id)
        .await?;
    Ok(RoutineBrowserAccessStatus {
        enabled: grant.as_ref().is_some_and(|grant| grant.enabled),
        server_name: grant.map(|grant| grant.server_name),
    })
}

/// Persist and immediately apply a routine-specific managed-browser grant.
/// The broker remains the request-time policy boundary; a database record is
/// merely opt-in metadata and never grants unattended computer control.
#[tauri::command]
pub(crate) async fn routine_browser_access_set(
    app: AppHandle,
    broker: State<'_, std::sync::Arc<BrowserBroker>>,
    request: SetRoutineBrowserAccessRequest,
) -> Result<RoutineBrowserAccessStatus, AppError> {
    let job_id = request.job_id.trim();
    if job_id.is_empty() {
        return Err(AppError::new(
            "invalid_arguments",
            "A routine id is required.",
        ));
    }
    let repos = repositories(&app).await?;
    let previous = repos.routine_browser_grant(job_id).await?;
    if request.enabled {
        let grant = previous
            .clone()
            .unwrap_or_else(|| RoutineBrowserGrantRecord {
                job_id: job_id.to_string(),
                server_name: format!("clovy_browser_routine_{}", Uuid::new_v4().simple()),
                token: Uuid::new_v4().to_string(),
                enabled: true,
            });
        let grant = RoutineBrowserGrantRecord {
            enabled: true,
            ..grant
        };
        broker.set_routine_grant(broker_grant(&grant));
        if let Err(error) = repos.set_routine_browser_grant(&grant).await {
            restore_browser_grant(&broker, previous, job_id);
            return Err(error);
        }
        Ok(RoutineBrowserAccessStatus {
            enabled: true,
            server_name: Some(grant.server_name),
        })
    } else {
        if let Some(previous) = previous {
            let disabled = RoutineBrowserGrantRecord {
                enabled: false,
                ..previous.clone()
            };
            broker.set_routine_grant(broker_grant(&disabled));
            // Revoke live control before awaiting database persistence. A
            // failed save may restore the opt-in, never the ended handles.
            broker.revoke_routine_sessions(job_id).await;
            if let Err(error) = repos.set_routine_browser_grant(&disabled).await {
                broker.set_routine_grant(broker_grant(&previous));
                return Err(error);
            }
        }
        broker.revoke_routine_sessions(job_id).await;
        Ok(RoutineBrowserAccessStatus {
            enabled: false,
            server_name: None,
        })
    }
}

fn broker_grant(grant: &RoutineBrowserGrantRecord) -> RoutineBrowserGrant {
    RoutineBrowserGrant {
        job_id: grant.job_id.clone(),
        enabled: grant.enabled,
    }
}

fn restore_browser_grant(
    broker: &BrowserBroker,
    previous: Option<RoutineBrowserGrantRecord>,
    job_id: &str,
) {
    if let Some(previous) = previous {
        broker.set_routine_grant(broker_grant(&previous));
    } else {
        broker.remove_routine_grant(job_id);
    }
}

#[derive(Debug, Clone)]
pub struct Claim {
    pub routine_id: String,
    pub routine_name: String,
    pub prompt: String,
    pub schedule: String,
    pub timezone: String,
    pub trigger_kind: String,
    pub model: String,
    pub safety_mode: AgentSafetyMode,
    pub enabled_toolsets: Vec<String>,
    pub token: String,
    pub routine_run_id: String,
}

fn routine_from_row(row: sqlx_sqlite::SqliteRow) -> AgentRoutineDto {
    AgentRoutineDto {
        id: row.get("id"),
        legacy_job_id: row.get("legacy_job_id"),
        name: row.get("name"),
        prompt: row.get("prompt"),
        schedule: row.get("schedule"),
        timezone: row.get("timezone"),
        repeat: row.get("repeat"),
        deliver: row.get("deliver"),
        model: row.get("model"),
        safety_mode: AgentSafetyMode::from(row.get::<String, _>("safety_mode").as_str()),
        state: row.get("state"),
        enabled: row.get::<i64, _>("enabled") != 0,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        next_run_at: row.get("next_run_at"),
        last_run_at: row.get("last_run_at"),
        last_status: row.get("last_status"),
        last_error: row.get("last_error"),
        last_delivery_error: row.get("last_delivery_error"),
        metadata: parse_metadata(row.get::<String, _>("metadata_json")),
    }
}
fn run_from_row(row: sqlx_sqlite::SqliteRow) -> AgentRoutineRunDto {
    AgentRoutineRunDto {
        id: row.get("id"),
        routine_id: row.get("routine_id"),
        agent_session_id: row.get("agent_session_id"),
        agent_run_id: row.get("agent_run_id"),
        trigger_kind: row.get("trigger_kind"),
        status: row.get("status"),
        scheduled_for: row.get("scheduled_for"),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
        model: row.get("model"),
        safety_mode: AgentSafetyMode::from(row.get::<String, _>("safety_mode").as_str()),
        error_code: row.get("error_code"),
        error_message: row.get("error_message"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}
fn parse_metadata(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or_else(|_| json!({}))
}
fn requires_legacy_execution_review(metadata: &Value) -> bool {
    metadata
        .get("legacyExecutionNeedsReview")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}
fn metadata_json(value: Value) -> String {
    if value.is_object() {
        value.to_string()
    } else {
        "{}".into()
    }
}
fn set_enabled_toolsets(metadata: &mut Value, enabled_toolsets: Option<Vec<String>>) {
    let Some(enabled_toolsets) = enabled_toolsets else {
        return;
    };
    if !metadata.is_object() {
        *metadata = json!({});
    }
    metadata["enabledToolsets"] = json!(enabled_toolsets);
}
fn prompt_title(prompt: &str) -> &str {
    let prompt = prompt.trim();
    let end = prompt
        .char_indices()
        .nth(50)
        .map(|(index, _)| index)
        .unwrap_or(prompt.len());
    prompt[..end].trim()
}
fn nonempty(value: String, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.into()
    } else {
        trimmed.into()
    }
}
fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
fn parse_time(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| AppError::new("routine_schedule_invalid", "Routine time is not RFC 3339."))
}
fn routine_workspace(app: &AppHandle, session_id: &str) -> Result<PathBuf, AppError> {
    Ok(crate::app_paths::app_data_dir(app)
        .map_err(|error| AppError::new("routine_workspace_failed", error.to_string()))?
        .join("agent-workspaces")
        .join(session_id))
}

fn enabled_toolsets_from_metadata(metadata: &Value, legacy_catalog: bool) -> Vec<String> {
    let has_explicit_catalog = matches!(metadata.get("enabledToolsets"), Some(Value::Array(_)));
    let imported_catalog =
        metadata.get("importedFrom").and_then(Value::as_str) == Some("legacy_hermes");
    let mut toolsets = match metadata.get("enabledToolsets") {
        Some(Value::Array(toolsets)) => toolsets
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        // Only rows durably marked as pre-catalog retain the historical Clovy
        // context, web, and read-only workspace contract. New routines persist
        // an explicit empty catalog until the user selects tools.
        _ if legacy_catalog => ["context_engine", "session_search", "web", "file"]
            .into_iter()
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    };

    // Only rows durably marked as pre-catalog may gain a toolset that was not
    // explicitly saved. A current explicit catalog can match an old default
    // exactly, so inferring its provenance from the selected entries would
    // override a user's decision to keep skills disabled.
    if !imported_catalog
        && legacy_catalog
        && !has_explicit_catalog
        && !toolsets.iter().any(|toolset| toolset == "skills")
    {
        toolsets.push("skills".to_string());
    }

    toolsets
}

async fn routine_trust(pool: &SqlitePool, routine_id: &str) -> (Vec<String>, Option<String>) {
    query("SELECT autonomous_tools, trust_mode FROM routine_trust WHERE job_id = ?")
        .bind(routine_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|row| {
            let tools =
                serde_json::from_str(&row.get::<String, _>("autonomous_tools")).unwrap_or_default();
            let mode = row.try_get::<String, _>("trust_mode").ok();
            (tools, mode)
        })
        .unwrap_or_default()
}

fn routine_base_tool_allowed(name: &str, enabled_toolsets: &[String]) -> bool {
    let has = |toolset: &str| enabled_toolsets.iter().any(|value| value == toolset);
    match name {
        name if matches!(
            name,
            "search_june" | "get_dictation" | "list_notes" | "list_note_transcripts"
        ) || crate::mcp_server::tools::is_read_tool(name)
            || crate::mcp_server::tools::is_conversation_read_tool(name) =>
        {
            has("context_engine") || has("memory") || has("session_search")
        }
        "get_obsidian_vault" | "approve_action" => true,
        "web_search" | "web_fetch" => has("web"),
        "list_files" | "read_file" | "preview_file" | "search_files" => has("file"),
        "list_skills" | "load_skill" => has("skills"),
        "search_memories" => has("memory") || has("context_engine"),
        "list_routines" | "list_routine_runs" => has("routines"),
        _ => false,
    }
}

fn routine_mcp_server_enabled(
    server_id: &str,
    server_name: &str,
    requires_approval: bool,
    enabled_toolsets: &[String],
) -> bool {
    if server_id == crate::agent_mcp::MANAGED_LINEAR_SERVER_ID {
        let has = |expected: &str| enabled_toolsets.iter().any(|toolset| toolset == expected);
        return if requires_approval {
            has("june_linear_actions")
        } else {
            has("june_linear")
        };
    }
    enabled_toolsets
        .iter()
        .any(|toolset| toolset == server_name)
}

pub async fn routine_tool_allowed_for_session(
    pool: &SqlitePool,
    session_id: &str,
    name: &str,
) -> Result<Option<bool>, AppError> {
    let row = query(
        "SELECT routines.id, routines.metadata_json, routines.tool_catalog_version
         FROM routine_runs
         JOIN routines ON routines.id = routine_runs.routine_id
         WHERE routine_runs.agent_session_id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(app_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let routine_id: String = row.get("id");
    let metadata = serde_json::from_str::<Value>(&row.get::<String, _>("metadata_json"))
        .unwrap_or_else(|_| json!({}));
    let enabled_toolsets =
        enabled_toolsets_from_metadata(&metadata, row.get::<i64, _>("tool_catalog_version") == 0);
    let (autonomous_tools, trust_mode) = routine_trust(pool, &routine_id).await;
    Ok(Some(
        routine_base_tool_allowed(name, &enabled_toolsets)
            || crate::connectors::agent_tools::routine_tool_allowed(
                name,
                &enabled_toolsets,
                &autonomous_tools,
                trust_mode.as_deref(),
            )
            || name == "request_clarification",
    ))
}

pub async fn routine_mcp_server_allowed_for_session(
    pool: &SqlitePool,
    session_id: &str,
    server_id: &str,
    server_name: &str,
    requires_approval: bool,
) -> Result<Option<bool>, AppError> {
    let row = query(
        "SELECT routines.metadata_json, routines.tool_catalog_version
         FROM routine_runs
         JOIN routines ON routines.id = routine_runs.routine_id
         WHERE routine_runs.agent_session_id = ?",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(app_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let metadata = serde_json::from_str::<Value>(&row.get::<String, _>("metadata_json"))
        .unwrap_or_else(|_| json!({}));
    let enabled_toolsets =
        enabled_toolsets_from_metadata(&metadata, row.get::<i64, _>("tool_catalog_version") == 0);
    Ok(Some(routine_mcp_server_enabled(
        server_id,
        server_name,
        requires_approval,
        &enabled_toolsets,
    )))
}

/// Supports the legacy routine schedule forms Clovy surfaced: one-shot RFC3339,
/// `every 30m|2h|1d`, macros, and standard five-field cron. Cron fields are
/// evaluated in the routine's IANA timezone, including daylight-saving
/// transitions, while persisted instants remain UTC.
fn next_run_after(
    schedule: &str,
    timezone: &str,
    after: DateTime<Utc>,
) -> Result<Option<DateTime<Utc>>, AppError> {
    let timezone = timezone.parse::<Tz>().map_err(|_| {
        AppError::new(
            "routine_timezone_invalid",
            "Routine timezone must be a valid IANA timezone.",
        )
    })?;
    let normalized = schedule.trim().to_ascii_lowercase();
    let source = match normalized.as_str() {
        "@hourly" => "0 * * * *".to_string(),
        "@daily" | "@midnight" => "0 0 * * *".to_string(),
        "@weekly" => "0 0 * * 0".to_string(),
        "@monthly" => "0 0 1 * *".to_string(),
        value => value.to_string(),
    };
    if let Ok(instant) = DateTime::parse_from_rfc3339(&source) {
        return Ok((instant.with_timezone(&Utc) > after).then_some(instant.with_timezone(&Utc)));
    }
    if let Some(interval) = source.strip_prefix("every ").and_then(parse_interval) {
        return Ok(Some(after + interval));
    }
    let fields: Vec<&str> = source.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(AppError::new(
            "routine_schedule_invalid",
            "Routine schedules must be RFC 3339, every <n>m/h/d, or five-field cron.",
        ));
    }
    let mut candidate = after + Duration::minutes(1);
    candidate = candidate
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .unwrap_or(candidate);
    for _ in 0..=527_040 {
        if cron_matches(&fields, candidate.with_timezone(&timezone)) {
            return Ok(Some(candidate));
        }
        candidate += Duration::minutes(1);
    }
    Err(AppError::new(
        "routine_schedule_invalid",
        "Routine schedule has no occurrence in the next year.",
    ))
}
fn parse_interval(value: &str) -> Option<Duration> {
    let value = value.trim();
    let (number, unit) = value.split_at(value.len().checked_sub(1)?);
    let number = number.parse::<i64>().ok()?.max(1);
    match unit {
        "m" => Some(Duration::minutes(number)),
        "h" => Some(Duration::hours(number)),
        "d" => Some(Duration::days(number)),
        _ => None,
    }
}
fn cron_matches<T: TimeZone>(fields: &[&str], value: DateTime<T>) -> bool {
    cron_field(fields[0], value.minute() as i32, 0, 59)
        && cron_field(fields[1], value.hour() as i32, 0, 23)
        && cron_field(fields[2], value.day() as i32, 1, 31)
        && cron_field(fields[3], value.month() as i32, 1, 12)
        && cron_field(
            fields[4],
            value.weekday().num_days_from_sunday() as i32,
            0,
            7,
        )
}
fn cron_field(field: &str, value: i32, min: i32, max: i32) -> bool {
    field.split(',').any(|part| {
        let (base, step) = part.split_once('/').map_or((part, 1), |(base, step)| {
            (base, step.parse::<i32>().unwrap_or(0))
        });
        if step < 1 {
            return false;
        }
        let matches_base = if base == "*" {
            true
        } else if let Some((start, end)) = base.split_once('-') {
            start
                .parse::<i32>()
                .ok()
                .zip(end.parse::<i32>().ok())
                .is_some_and(|(start, end)| value >= start && value <= end)
        } else {
            base.parse::<i32>()
                .ok()
                .is_some_and(|wanted| wanted == value || (max == 7 && wanted == 7 && value == 0))
        };
        if !matches_base {
            return false;
        }
        if base == "*" {
            (value - min) % step == 0
        } else if let Some((start, _)) = base.split_once('-') {
            start
                .parse::<i32>()
                .ok()
                .is_some_and(|start| (value - start) % step == 0)
        } else {
            true
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx_sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};
    use std::time::Duration as StdDuration;

    async fn claim(
        pool: &SqlitePool,
        id: &str,
        trigger: &str,
        due: bool,
    ) -> Result<Option<Claim>, AppError> {
        let (_directory, env) =
            super::selection_tests::environment(&[crate::llm::cli::CliKind::Claude]);
        super::claim_with_connector_guard_for_turn(pool, id, trigger, due, None, None, &env).await
    }

    async fn claim_due(pool: &SqlitePool) -> Result<Vec<Claim>, AppError> {
        let (_directory, env) =
            super::selection_tests::environment(&[crate::llm::cli::CliKind::Claude]);
        super::claim_due_with_env(pool, &env).await
    }

    async fn initialize_pool(pool: &SqlitePool) {
        // The routine migration only needs these parent keys for FK coverage.
        // Running the full runtime migration in memory would also require the
        // retired Hermes tables it deliberately imports from.
        for statement in [
            "CREATE TABLE agent_sessions (id TEXT PRIMARY KEY, status TEXT, updated_at TEXT, last_error TEXT)",
            "CREATE TABLE agent_runs (id TEXT PRIMARY KEY, session_id TEXT, status TEXT, updated_at TEXT, completed_at TEXT, interrupted_state_json TEXT, error_code TEXT, error_message TEXT)",
            "CREATE TABLE connector_triggers (id TEXT PRIMARY KEY, job_id TEXT, kind TEXT, account_id TEXT)",
        ] {
            query(statement).execute(pool).await.unwrap();
        }
        for statement in include_str!("../migrations/026_routines.sql")
            .split(';')
            .filter(|statement| !statement.trim().is_empty())
        {
            query(statement).execute(pool).await.unwrap();
        }
    }

    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        initialize_pool(&pool).await;
        pool
    }

    async fn concurrent_pool() -> (tempfile::TempDir, SqlitePool) {
        let directory = tempfile::tempdir().expect("temporary routine database directory");
        let options = SqliteConnectOptions::new()
            .filename(directory.path().join("routine-concurrency.sqlite3"))
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(StdDuration::from_secs(2));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .expect("file-backed routine sqlite");
        initialize_pool(&pool).await;
        (directory, pool)
    }
    fn create_request(schedule: &str) -> CreateAgentRoutineRequest {
        CreateAgentRoutineRequest {
            id: Some("routine-1".into()),
            legacy_job_id: Some("legacy-1".into()),
            name: Some("Daily brief".into()),
            prompt: "Summarize notes".into(),
            schedule: schedule.into(),
            timezone: "UTC".into(),
            repeat: None,
            deliver: None,
            model: "auto".into(),
            safety_mode: Some(AgentSafetyMode::Sandboxed),
            state: None,
            enabled: None,
            metadata: json!({"origin":"test"}),
            enabled_toolsets: None,
        }
    }

    async fn claimed_terminal_run(
        pool: &SqlitePool,
        routine_id: &str,
        session_id: &str,
        agent_run_id: &str,
    ) -> Claim {
        let mut request = create_request("every 1h");
        request.id = Some(routine_id.to_string());
        request.legacy_job_id = Some(format!("legacy-{routine_id}"));
        request.repeat = Some("3".into());
        create(pool, request).await.unwrap();
        let claimed = claim(pool, routine_id, "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES (?)")
            .bind(session_id)
            .execute(pool)
            .await
            .unwrap();
        query(
            "INSERT INTO agent_runs
             (id, status, completed_at)
             VALUES (?, 'completed', '2026-07-28T08:00:00.000Z')",
        )
        .bind(agent_run_id)
        .execute(pool)
        .await
        .unwrap();
        attach_run_mapping(
            pool,
            &claimed.routine_run_id,
            &claimed.token,
            session_id,
            agent_run_id,
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();
        claimed
    }

    #[test]
    fn cron_schedule_uses_the_routine_iana_timezone() {
        let summer = Utc.with_ymd_and_hms(2026, 7, 24, 12, 30, 0).unwrap();
        let winter = Utc.with_ymd_and_hms(2026, 1, 24, 13, 30, 0).unwrap();
        assert_eq!(
            next_run_after("0 9 * * *", "America/New_York", summer)
                .unwrap()
                .unwrap(),
            Utc.with_ymd_and_hms(2026, 7, 24, 13, 0, 0).unwrap()
        );
        assert_eq!(
            next_run_after("0 9 * * *", "America/New_York", winter)
                .unwrap()
                .unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 24, 14, 0, 0).unwrap()
        );
    }

    #[test]
    fn finite_repeat_completes_after_the_preserved_run_count() {
        let mut metadata = json!({"repeatState":{"completed":1}});
        assert!(!advance_repeat("3", &mut metadata));
        assert_eq!(metadata["repeatState"]["completed"], 2);
        assert!(advance_repeat("3", &mut metadata));
        assert_eq!(metadata["repeatState"]["completed"], 3);
        assert!(!advance_repeat("forever", &mut metadata));
    }

    #[test]
    fn routines_without_installed_eligible_engines_fail_before_claiming() {
        for model in ["", "auto", "open-software/auto"] {
            assert_eq!(
                resolve_routine_model(model, &[]).unwrap_err().code,
                "routine_engine_unavailable"
            );
        }
        assert_eq!(
            resolve_routine_model("explicit-model", &[])
                .unwrap_err()
                .code,
            "routine_engine_not_allowed"
        );
        assert_eq!(
            resolve_routine_model(
                "__clovy_cli_engine__:claude",
                &[crate::llm::cli::CliKind::Codex]
            )
            .unwrap_err()
            .code,
            "llm_cli_not_installed"
        );
        assert_eq!(
            resolve_routine_model("auto", &[crate::llm::cli::CliKind::Codex]).unwrap(),
            "__clovy_cli_engine__:codex"
        );
    }

    #[test]
    fn historical_linear_toolsets_preserve_read_and_action_boundaries() {
        let managed_id = crate::agent_mcp::MANAGED_LINEAR_SERVER_ID;
        let managed_name = crate::agent_mcp::MANAGED_LINEAR_SERVER_NAME;
        assert!(routine_mcp_server_enabled(
            managed_id,
            managed_name,
            false,
            &["june_linear".into()],
        ));
        assert!(!routine_mcp_server_enabled(
            managed_id,
            managed_name,
            true,
            &["june_linear".into()],
        ));
        assert!(routine_mcp_server_enabled(
            managed_id,
            managed_name,
            true,
            &["june_linear_actions".into()],
        ));
        assert!(!routine_mcp_server_enabled(
            managed_id,
            managed_name,
            false,
            &["june_linear_actions".into()],
        ));
        for requires_approval in [false, true] {
            assert!(!routine_mcp_server_enabled(
                managed_id,
                managed_name,
                requires_approval,
                &["linear".into()],
            ));
        }
        assert!(!routine_mcp_server_enabled(
            managed_id,
            managed_name,
            false,
            &["web".into()],
        ));
        assert!(!routine_mcp_server_enabled(
            "custom-linear",
            "linear",
            false,
            &["june_linear".into()],
        ));
        assert!(routine_mcp_server_enabled(
            "custom-linear",
            "linear",
            false,
            &["linear".into()],
        ));
    }

    #[tokio::test]
    async fn routine_session_gate_maps_saved_linear_toolsets_to_hosted_policy() {
        let pool = pool().await;
        let mut request = create_request("every 1h");
        request.enabled_toolsets = Some(vec!["june_linear".into()]);
        create(&pool, request).await.unwrap();
        let claimed = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-linear')")
            .execute(&pool)
            .await
            .unwrap();
        query("INSERT INTO agent_runs (id) VALUES ('run-linear')")
            .execute(&pool)
            .await
            .unwrap();
        attach_run_mapping(
            &pool,
            &claimed.routine_run_id,
            &claimed.token,
            "session-linear",
            "run-linear",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();

        assert_eq!(
            routine_mcp_server_allowed_for_session(
                &pool,
                "session-linear",
                crate::agent_mcp::MANAGED_LINEAR_SERVER_ID,
                crate::agent_mcp::MANAGED_LINEAR_SERVER_NAME,
                false,
            )
            .await
            .unwrap(),
            Some(true)
        );
        assert_eq!(
            routine_mcp_server_allowed_for_session(
                &pool,
                "session-linear",
                crate::agent_mcp::MANAGED_LINEAR_SERVER_ID,
                crate::agent_mcp::MANAGED_LINEAR_SERVER_NAME,
                true,
            )
            .await
            .unwrap(),
            Some(false)
        );
        assert_eq!(
            routine_mcp_server_allowed_for_session(
                &pool,
                "session-linear",
                "custom-linear",
                "linear",
                false,
            )
            .await
            .unwrap(),
            Some(false)
        );
    }

    #[test]
    fn scheduled_dispatch_failure_advances_instead_of_reclaiming_the_due_time() {
        let now = Utc.with_ymd_and_hms(2026, 7, 25, 12, 0, 0).unwrap();
        let claim = Claim {
            routine_id: "routine-1".into(),
            routine_name: "Brief".into(),
            prompt: "Summarize".into(),
            schedule: "every 1h".into(),
            timezone: "UTC".into(),
            trigger_kind: "schedule".into(),
            model: "private-auto".into(),
            safety_mode: AgentSafetyMode::Sandboxed,
            enabled_toolsets: vec![],
            token: "token".into(),
            routine_run_id: "run-1".into(),
        };

        let next = parse_time(&next_run_after_dispatch_failure(&claim, now).unwrap()).unwrap();
        assert!(next > now);
    }

    #[test]
    fn invalid_schedule_dispatch_failure_uses_a_bounded_backoff() {
        let now = Utc.with_ymd_and_hms(2026, 7, 25, 12, 0, 0).unwrap();
        let claim = Claim {
            routine_id: "routine-1".into(),
            routine_name: "Brief".into(),
            prompt: "Summarize".into(),
            schedule: "not a schedule".into(),
            timezone: "UTC".into(),
            trigger_kind: "schedule".into(),
            model: "private-auto".into(),
            safety_mode: AgentSafetyMode::Sandboxed,
            enabled_toolsets: vec![],
            token: "token".into(),
            routine_run_id: "run-1".into(),
        };

        let next = parse_time(&next_run_after_dispatch_failure(&claim, now).unwrap()).unwrap();
        assert_eq!(next, now + Duration::minutes(5));
    }

    #[tokio::test]
    async fn crud_persists_the_legacy_compatible_contract() {
        let pool = pool().await;
        let created = create(&pool, create_request("every 1h")).await.unwrap();
        assert_eq!(created.id, "routine-1");
        assert_eq!(created.legacy_job_id.as_deref(), Some("legacy-1"));
        assert!(created.next_run_at.is_some());
        let updated = update(
            &pool,
            UpdateAgentRoutineRequest {
                routine_id: created.id.clone(),
                name: Some("Updated brief".into()),
                prompt: None,
                schedule: None,
                timezone: None,
                repeat: None,
                deliver: None,
                model: Some("model-x".into()),
                safety_mode: Some(AgentSafetyMode::Unrestricted),
                state: None,
                enabled: None,
                metadata: Some(json!({"edited":true})),
                enabled_toolsets: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "Updated brief");
        assert_eq!(updated.model, "model-x");
        assert_eq!(updated.safety_mode, AgentSafetyMode::Unrestricted);
        assert_eq!(pause(&pool, &created.id).await.unwrap().state, "paused");
        assert_eq!(resume(&pool, &created.id).await.unwrap().state, "scheduled");
        delete(&pool, &created.id).await.unwrap();
        assert!(list(&pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn imported_legacy_execution_routine_cannot_be_resumed_or_reenabled() {
        let pool = pool().await;
        let mut request = create_request("every 1h");
        request.state = Some("needs_review".into());
        request.enabled = Some(false);
        request.metadata = json!({"legacyExecutionNeedsReview": true});
        create(&pool, request).await.unwrap();
        let resume_error = resume(&pool, "routine-1").await.unwrap_err();
        assert_eq!(
            resume_error.code,
            "routine_legacy_execution_review_required"
        );
        let update_error = update(
            &pool,
            UpdateAgentRoutineRequest {
                routine_id: "routine-1".into(),
                name: None,
                prompt: None,
                schedule: None,
                timezone: None,
                repeat: None,
                deliver: None,
                model: None,
                safety_mode: None,
                state: Some("scheduled".into()),
                enabled: Some(true),
                metadata: Some(json!({})),
                enabled_toolsets: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            update_error.code,
            "routine_legacy_execution_review_required"
        );
    }

    #[tokio::test]
    async fn due_claim_is_single_flight_and_reconciles_an_expired_restart_claim() {
        let pool = pool().await;
        create(&pool, create_request("every 1m")).await.unwrap();
        query("UPDATE routines SET next_run_at = ? WHERE id = 'routine-1'")
            .bind(format_time(Utc::now() - Duration::minutes(1)))
            .execute(&pool)
            .await
            .unwrap();
        let first = claim_due(&pool).await.unwrap();
        assert_eq!(first.len(), 1);
        assert!(claim_due(&pool).await.unwrap().is_empty());
        query("UPDATE routine_runs SET updated_at = ? WHERE id = ?")
            .bind(format_time(
                Utc::now() - Duration::minutes(CLAIM_STALE_AFTER_MINUTES + 1),
            ))
            .bind(&first[0].routine_run_id)
            .execute(&pool)
            .await
            .unwrap();
        reconcile(&pool).await.unwrap();
        assert_eq!(
            list_runs(&pool, Some("routine-1")).await.unwrap()[0].status,
            "interrupted"
        );
    }

    #[tokio::test]
    async fn event_style_manual_claim_ignores_far_future_schedule_and_keeps_toolsets() {
        let pool = pool().await;
        let mut request = create_request("2099-01-01T09:00:00Z");
        request.enabled_toolsets = Some(vec!["web".into(), "june_gmail".into()]);
        create(&pool, request).await.unwrap();

        let claim = claim(&pool, "routine-1", "email_received", false)
            .await
            .unwrap()
            .expect("active event routine should be claimable");
        assert_eq!(claim.enabled_toolsets, vec!["web", "june_gmail"]);
        assert_eq!(claim.routine_id, "routine-1");
    }

    #[tokio::test]
    async fn connector_claim_rejects_a_removed_poll_snapshot() {
        let pool = pool().await;
        create(&pool, create_request("2099-01-01T09:00:00Z"))
            .await
            .unwrap();
        query(
            "INSERT INTO connector_triggers (id, job_id, kind, account_id)
             VALUES ('trigger-1', 'routine-1', 'email_received', 'user@example.com')",
        )
        .execute(&pool)
        .await
        .unwrap();
        query("DELETE FROM connector_triggers WHERE id = 'trigger-1'")
            .execute(&pool)
            .await
            .unwrap();

        let guard = ConnectorClaimGuard {
            trigger_id: "trigger-1",
            kind: "email_received",
            account_id: "user@example.com",
        };
        assert!(claim_with_connector_guard(
            &pool,
            "routine-1",
            "email_received",
            false,
            Some(guard),
        )
        .await
        .unwrap()
        .is_none());
        assert!(list_runs(&pool, Some("routine-1"))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn new_routines_without_a_catalog_have_no_tools() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();

        let claim = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        assert!(claim.enabled_toolsets.is_empty());
    }

    #[tokio::test]
    async fn pre_catalog_routines_keep_the_legacy_safe_toolset() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        query(
            "UPDATE routines
             SET metadata_json = json_remove(metadata_json, '$.enabledToolsets'),
                 tool_catalog_version = 0
             WHERE id = 'routine-1'",
        )
        .execute(&pool)
        .await
        .unwrap();

        let claim = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            claim.enabled_toolsets,
            vec!["context_engine", "session_search", "web", "file", "skills"]
        );
    }

    #[tokio::test]
    async fn explicit_empty_pre_catalog_routines_keep_no_tools() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        query("UPDATE routines SET tool_catalog_version = 0 WHERE id = 'routine-1'")
            .execute(&pool)
            .await
            .unwrap();

        let claim = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        assert!(claim.enabled_toolsets.is_empty());
    }

    #[test]
    fn explicit_sandbox_catalogs_preserve_disabled_skill_loading() {
        let metadata = json!({
            "enabledToolsets": [
                "web",
                "vision",
                "todo",
                "memory",
                "session_search",
                "context_engine",
                "june_notion"
            ]
        });

        let toolsets = enabled_toolsets_from_metadata(&metadata, false);
        assert!(!toolsets.iter().any(|toolset| toolset == "skills"));
        assert!(toolsets.iter().any(|toolset| toolset == "june_notion"));
        assert!(!toolsets
            .iter()
            .any(|toolset| toolset == "june_notion_actions"));
    }

    #[test]
    fn explicit_sandbox_catalogs_without_memory_preserve_disabled_skills() {
        let metadata = json!({
            "enabledToolsets": [
                "web",
                "vision",
                "todo",
                "session_search",
                "context_engine"
            ]
        });

        let toolsets = enabled_toolsets_from_metadata(&metadata, false);
        assert!(!toolsets.iter().any(|toolset| toolset == "skills"));
        assert!(!toolsets.iter().any(|toolset| toolset == "memory"));
    }

    #[test]
    fn imported_empty_catalogs_remain_deny_by_default() {
        let metadata = json!({
            "importedFrom": "legacy_hermes",
            "enabledToolsets": []
        });

        let toolsets = enabled_toolsets_from_metadata(&metadata, false);
        assert!(toolsets.is_empty());
    }

    #[test]
    fn imported_partial_catalogs_preserve_exact_toolset_selection() {
        let metadata = json!({
            "importedFrom": "legacy_hermes",
            "enabledToolsets": ["web"]
        });

        let toolsets = enabled_toolsets_from_metadata(&metadata, false);
        assert_eq!(toolsets, vec!["web"]);
    }

    #[test]
    fn sandboxed_routines_can_load_skill_instructions_without_machine_tools() {
        let toolsets = crate::connectors::policy::SANDBOXED_ROUTINE_BASE_TOOLSETS
            .iter()
            .map(|name| name.to_string())
            .collect::<Vec<_>>();
        assert!(routine_base_tool_allowed("list_skills", &toolsets));
        assert!(routine_base_tool_allowed("load_skill", &toolsets));
        assert!(!routine_base_tool_allowed("write_file", &toolsets));
        assert!(!routine_base_tool_allowed("run_shell", &toolsets));
    }

    #[tokio::test]
    async fn claimed_run_snapshots_the_resolved_cli_model_and_safety_mode() {
        let _guard = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let pool = pool().await;
        let mut request = create_request("every 1h");
        request.model = "auto".into();
        request.safety_mode = Some(AgentSafetyMode::Unrestricted);
        create(&pool, request).await.unwrap();
        let claim = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            claim.model,
            crate::chat_engine::engine_model_id(crate::llm::cli::CliKind::Claude)
        );
        assert_eq!(claim.safety_mode, AgentSafetyMode::Unrestricted);
        let run = list_runs(&pool, Some("routine-1"))
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(
            run.model,
            crate::chat_engine::engine_model_id(crate::llm::cli::CliKind::Claude)
        );
        assert_eq!(run.safety_mode, AgentSafetyMode::Unrestricted);
    }

    #[tokio::test]
    async fn routine_run_maps_one_agent_session_and_run() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claim = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-1')")
            .execute(&pool)
            .await
            .unwrap();
        query("INSERT INTO agent_runs (id) VALUES ('run-1')")
            .execute(&pool)
            .await
            .unwrap();
        attach_run_mapping(
            &pool,
            &claim.routine_run_id,
            &claim.token,
            "session-1",
            "run-1",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();
        let run = list_runs(&pool, Some("routine-1"))
            .await
            .unwrap()
            .pop()
            .unwrap();
        assert_eq!(run.agent_session_id.as_deref(), Some("session-1"));
        assert_eq!(run.agent_run_id.as_deref(), Some("run-1"));
        assert_eq!(run.status, "running");
    }

    #[tokio::test]
    async fn terminal_agent_event_immediately_reconciles_the_routine_run() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claimed = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-terminal')")
            .execute(&pool)
            .await
            .unwrap();
        query(
            "INSERT INTO agent_runs
             (id, status, completed_at, error_code, error_message)
             VALUES ('run-terminal', 'failed', '2026-07-28T08:00:00.000Z',
                     'model_not_priced', 'model_not_priced')",
        )
        .execute(&pool)
        .await
        .unwrap();
        attach_run_mapping(
            &pool,
            &claimed.routine_run_id,
            &claimed.token,
            "session-terminal",
            "run-terminal",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();

        mark_agent_run_terminal(&pool, "run-terminal")
            .await
            .unwrap();

        let run = list_runs(&pool, Some("routine-1")).await.unwrap().remove(0);
        assert_eq!(run.status, "failed");
        assert_eq!(run.error_code.as_deref(), Some("model_not_priced"));
        let routine = get(&pool, "routine-1").await.unwrap();
        assert_eq!(routine.last_status.as_deref(), Some("error"));
        assert_eq!(routine.last_error.as_deref(), Some("model_not_priced"));
        assert!(claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn concurrent_edit_cannot_resurrect_a_completed_finite_repeat() {
        let (_directory, pool) = concurrent_pool().await;
        for iteration in 0..12 {
            let routine_id = format!("routine-concurrent-{iteration}");
            let session_id = format!("session-concurrent-{iteration}");
            let agent_run_id = format!("run-concurrent-{iteration}");
            let mut request = create_request("every 1h");
            request.id = Some(routine_id.clone());
            request.legacy_job_id = Some(format!("legacy-concurrent-{iteration}"));
            request.repeat = Some("once".into());
            create(&pool, request).await.unwrap();
            let claimed = claim(&pool, &routine_id, "manual", false)
                .await
                .unwrap()
                .unwrap();
            query("INSERT INTO agent_sessions (id) VALUES (?)")
                .bind(&session_id)
                .execute(&pool)
                .await
                .unwrap();
            query(
                "INSERT INTO agent_runs
                 (id, status, completed_at)
                 VALUES (?, 'completed', '2026-07-28T08:00:00.000Z')",
            )
            .bind(&agent_run_id)
            .execute(&pool)
            .await
            .unwrap();
            attach_run_mapping(
                &pool,
                &claimed.routine_run_id,
                &claimed.token,
                &session_id,
                &agent_run_id,
                &crate::db::repositories::timestamp(),
            )
            .await
            .unwrap();

            let edit = UpdateAgentRoutineRequest {
                routine_id: routine_id.clone(),
                name: Some(format!("Edited routine {iteration}")),
                prompt: None,
                schedule: None,
                timezone: None,
                repeat: None,
                deliver: None,
                model: None,
                safety_mode: None,
                state: None,
                enabled: None,
                metadata: None,
                enabled_toolsets: None,
            };
            let (edit_result, terminal_result) = tokio::join!(
                update(&pool, edit),
                mark_agent_run_terminal(&pool, &agent_run_id)
            );
            edit_result.unwrap();
            terminal_result.unwrap();

            let routine = get(&pool, &routine_id).await.unwrap();
            assert_eq!(routine.state, "completed");
            assert!(!routine.enabled);
            assert_eq!(
                routine.metadata["repeatState"]["completed"].as_u64(),
                Some(1)
            );
            assert_eq!(routine.last_status.as_deref(), Some("ok"));
            let run = list_runs(&pool, Some(&routine_id)).await.unwrap().remove(0);
            assert_eq!(run.status, "completed");
        }
    }

    #[tokio::test]
    async fn pausing_before_terminal_projection_preserves_the_current_run() {
        let pool = pool().await;
        let claimed =
            claimed_terminal_run(&pool, "routine-paused", "session-paused", "run-paused").await;

        pause(&pool, "routine-paused").await.unwrap();
        let retained_claim: Option<String> =
            query("SELECT claim_token FROM routines WHERE id = 'routine-paused'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("claim_token");
        assert_eq!(retained_claim.as_deref(), Some(claimed.token.as_str()));
        mark_agent_run_terminal(&pool, "run-paused").await.unwrap();

        let routine = get(&pool, "routine-paused").await.unwrap();
        assert_eq!(routine.state, "paused");
        assert!(!routine.enabled);
        assert_eq!(routine.last_status.as_deref(), Some("ok"));
        assert_eq!(
            routine.metadata["repeatState"]["completed"].as_u64(),
            Some(1)
        );
        let run = list_runs(&pool, Some("routine-paused"))
            .await
            .unwrap()
            .remove(0);
        assert_eq!(run.status, "completed");
        let claim_token: Option<String> =
            query("SELECT claim_token FROM routines WHERE id = 'routine-paused'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("claim_token");
        assert_eq!(claim_token, None);
    }

    #[tokio::test]
    async fn disabling_by_update_before_terminal_projection_preserves_the_current_run() {
        let pool = pool().await;
        let claimed = claimed_terminal_run(
            &pool,
            "routine-updated-paused",
            "session-updated-paused",
            "run-updated-paused",
        )
        .await;
        update(
            &pool,
            UpdateAgentRoutineRequest {
                routine_id: "routine-updated-paused".into(),
                name: None,
                prompt: None,
                schedule: None,
                timezone: None,
                repeat: None,
                deliver: None,
                model: None,
                safety_mode: None,
                state: Some("paused".into()),
                enabled: Some(false),
                metadata: None,
                enabled_toolsets: None,
            },
        )
        .await
        .unwrap();
        let retained_claim: Option<String> =
            query("SELECT claim_token FROM routines WHERE id = 'routine-updated-paused'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("claim_token");
        assert_eq!(retained_claim.as_deref(), Some(claimed.token.as_str()));

        mark_agent_run_terminal(&pool, "run-updated-paused")
            .await
            .unwrap();

        let routine = get(&pool, "routine-updated-paused").await.unwrap();
        assert_eq!(routine.state, "paused");
        assert!(!routine.enabled);
        assert_eq!(routine.last_status.as_deref(), Some("ok"));
        assert_eq!(
            routine.metadata["repeatState"]["completed"].as_u64(),
            Some(1)
        );
        assert_eq!(
            list_runs(&pool, Some("routine-updated-paused"))
                .await
                .unwrap()[0]
                .status,
            "completed"
        );
    }

    #[tokio::test]
    async fn terminal_projection_rolls_back_when_claim_release_fails() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claimed = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-atomic')")
            .execute(&pool)
            .await
            .unwrap();
        query(
            "INSERT INTO agent_runs
             (id, status, completed_at, error_code, error_message)
             VALUES ('run-atomic', 'failed', '2026-07-28T08:00:00.000Z',
                     'model_not_priced', 'model_not_priced')",
        )
        .execute(&pool)
        .await
        .unwrap();
        attach_run_mapping(
            &pool,
            &claimed.routine_run_id,
            &claimed.token,
            "session-atomic",
            "run-atomic",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();
        query("UPDATE routines SET schedule = 'not a schedule' WHERE id = 'routine-1'")
            .execute(&pool)
            .await
            .unwrap();

        let error = mark_agent_run_terminal(&pool, "run-atomic")
            .await
            .unwrap_err();

        assert_eq!(error.code, "routine_schedule_invalid");
        let run = list_runs(&pool, Some("routine-1")).await.unwrap().remove(0);
        assert_eq!(run.status, "running");
        assert_eq!(run.error_code, None);
        let routine = get(&pool, "routine-1").await.unwrap();
        assert_eq!(routine.last_status, None);
        let claim_token: Option<String> =
            query("SELECT claim_token FROM routines WHERE id = 'routine-1'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("claim_token");
        assert_eq!(claim_token.as_deref(), Some(claimed.token.as_str()));
    }

    #[tokio::test]
    async fn scheduler_tick_retries_a_missed_terminal_projection() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claimed = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-retry')")
            .execute(&pool)
            .await
            .unwrap();
        query(
            "INSERT INTO agent_runs
             (id, status, completed_at)
             VALUES ('run-retry', 'completed', '2026-07-28T08:00:00.000Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        attach_run_mapping(
            &pool,
            &claimed.routine_run_id,
            &claimed.token,
            "session-retry",
            "run-retry",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();

        assert!(scheduler_tick(&pool).await.unwrap().is_empty());

        let run = list_runs(&pool, Some("routine-1")).await.unwrap().remove(0);
        assert_eq!(run.status, "completed");
        let routine = get(&pool, "routine-1").await.unwrap();
        assert_eq!(routine.last_status.as_deref(), Some("ok"));
        let claim_token: Option<String> =
            query("SELECT claim_token FROM routines WHERE id = 'routine-1'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("claim_token");
        assert_eq!(claim_token, None);
    }

    #[tokio::test]
    async fn non_routine_terminal_event_does_not_release_an_unrelated_routine_claim() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claimed = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("UPDATE routine_runs SET status = 'failed' WHERE id = ?")
            .bind(&claimed.routine_run_id)
            .execute(&pool)
            .await
            .unwrap();
        query("UPDATE routines SET schedule = 'not a schedule' WHERE id = 'routine-1'")
            .execute(&pool)
            .await
            .unwrap();

        mark_agent_run_terminal(&pool, "unrelated-agent-run")
            .await
            .unwrap();

        assert!(claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn restart_releases_an_active_agent_run_for_a_future_schedule() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claim = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-restart')")
            .execute(&pool)
            .await
            .unwrap();
        query("INSERT INTO agent_runs (id, status) VALUES ('run-restart', 'running')")
            .execute(&pool)
            .await
            .unwrap();
        attach_run_mapping(
            &pool,
            &claim.routine_run_id,
            &claim.token,
            "session-restart",
            "run-restart",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();
        reconcile_after_restart(&pool).await.unwrap();
        assert_eq!(
            list_runs(&pool, Some("routine-1")).await.unwrap()[0].status,
            "interrupted"
        );
        let routine = get(&pool, "routine-1").await.unwrap();
        assert_eq!(routine.last_status.as_deref(), Some("error"));
        assert!(routine.next_run_at.is_some());
    }

    #[tokio::test]
    async fn restart_preserves_a_waiting_run_and_its_single_flight_claim() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claimed_run = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-waiting')")
            .execute(&pool)
            .await
            .unwrap();
        query(
            "INSERT INTO agent_runs (id, status, interrupted_state_json)
             VALUES ('run-waiting', 'waiting_for_user', '\"serialized-state\"')",
        )
        .execute(&pool)
        .await
        .unwrap();
        attach_run_mapping(
            &pool,
            &claimed_run.routine_run_id,
            &claimed_run.token,
            "session-waiting",
            "run-waiting",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();
        mark_agent_run_waiting(&pool, "run-waiting").await.unwrap();

        reconcile_after_restart(&pool).await.unwrap();

        assert_eq!(
            list_runs(&pool, Some("routine-1")).await.unwrap()[0].status,
            "waiting_for_user"
        );
        let claim_token: Option<String> =
            query("SELECT claim_token FROM routines WHERE id = 'routine-1'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("claim_token");
        assert_eq!(claim_token.as_deref(), Some(claimed_run.token.as_str()));
        assert!(claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .is_none());
        let agent_run = query("SELECT status, interrupted_state_json FROM agent_runs WHERE id = ?")
            .bind("run-waiting")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(agent_run.get::<String, _>("status"), "waiting_for_user");
        assert_eq!(
            agent_run.get::<Option<String>, _>("interrupted_state_json"),
            Some("\"serialized-state\"".to_string())
        );
    }

    #[tokio::test]
    async fn restart_repairs_a_partially_persisted_waiting_transition() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claimed_run = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-partial-wait')")
            .execute(&pool)
            .await
            .unwrap();
        query(
            "INSERT INTO agent_runs (id, status, interrupted_state_json)
             VALUES ('run-partial-wait', 'waiting_for_user', '\"serialized-state\"')",
        )
        .execute(&pool)
        .await
        .unwrap();
        attach_run_mapping(
            &pool,
            &claimed_run.routine_run_id,
            &claimed_run.token,
            "session-partial-wait",
            "run-partial-wait",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();

        reconcile_after_restart(&pool).await.unwrap();

        assert_eq!(
            list_runs(&pool, Some("routine-1")).await.unwrap()[0].status,
            "waiting_for_user"
        );
        assert!(claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn restart_repairs_a_partially_persisted_resume_transition() {
        let pool = pool().await;
        create(&pool, create_request("every 1h")).await.unwrap();
        let claimed_run = claim(&pool, "routine-1", "manual", false)
            .await
            .unwrap()
            .unwrap();
        query("INSERT INTO agent_sessions (id) VALUES ('session-partial-resume')")
            .execute(&pool)
            .await
            .unwrap();
        query("INSERT INTO agent_runs (id, status) VALUES ('run-partial-resume', 'running')")
            .execute(&pool)
            .await
            .unwrap();
        attach_run_mapping(
            &pool,
            &claimed_run.routine_run_id,
            &claimed_run.token,
            "session-partial-resume",
            "run-partial-resume",
            &crate::db::repositories::timestamp(),
        )
        .await
        .unwrap();
        query("UPDATE routine_runs SET status = 'waiting_for_user' WHERE id = ?")
            .bind(&claimed_run.routine_run_id)
            .execute(&pool)
            .await
            .unwrap();

        reconcile_after_restart(&pool).await.unwrap();

        assert_eq!(
            list_runs(&pool, Some("routine-1")).await.unwrap()[0].status,
            "interrupted"
        );
        assert!(get(&pool, "routine-1").await.unwrap().next_run_at.is_some());
    }

    #[test]
    fn automatic_routines_resolve_only_installed_eligible_engines() {
        use crate::llm::cli::CliKind;
        for stored_model in ["", "auto", crate::providers::LEGACY_AUTO_GENERATION_MODEL] {
            assert_eq!(
                resolve_routine_model(stored_model, &[CliKind::Claude]).unwrap(),
                "__clovy_cli_engine__:claude"
            );
        }
        assert_eq!(
            resolve_routine_model("explicit-model", &[CliKind::Codex])
                .unwrap_err()
                .code,
            "routine_engine_not_allowed"
        );
        assert_eq!(
            resolve_routine_model("auto", &[CliKind::Agy])
                .unwrap_err()
                .code,
            "routine_engine_unavailable"
        );
    }

    /// Persists like the app does (routine projection included).
    #[cfg(unix)]
    struct RoutineSink {
        repository: AgentRepository,
    }

    #[cfg(unix)]
    impl crate::chat_engine::FrameSink for RoutineSink {
        async fn publish(&self, frame: crate::agent_sessions::protocol::RpcFrame) {
            crate::agent_sessions::events::persist_runtime_event(&self.repository, &frame)
                .await
                .expect("event persists");
        }
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn routine_on_a_cli_engine_runs_unattended_and_records_its_result() {
        use crate::llm::cli::CliKind;
        use std::os::unix::fs::PermissionsExt;

        for (toolsets, expected_tools) in [
            (vec![], ""),
            (vec!["file"], "Read,Grep,Glob"),
            (vec!["web"], "WebSearch,WebFetch"),
            (vec!["file", "web"], "Read,Grep,Glob,WebSearch,WebFetch"),
        ] {
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap();
            crate::db::migrations::run_migrations(&pool).await.unwrap();
            let mut request = create_request("every 1h");
            request.model = crate::chat_engine::engine_model_id(CliKind::Claude);
            request.enabled_toolsets = Some(toolsets.into_iter().map(str::to_string).collect());
            create(&pool, request).await.unwrap();
            let claimed = claim(&pool, "routine-1", "manual", false)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(claimed.model, "__clovy_cli_engine__:claude");

            let repository = AgentRepository::new(pool.clone());
            let workspace = tempfile::tempdir().unwrap();
            let session_id = Uuid::new_v4().to_string();
            let mut created = None;
            let run_id = persist_claim_run(
                &pool,
                &repository,
                &claimed,
                &session_id,
                workspace.path(),
                &crate::db::repositories::timestamp(),
                &mut created,
            )
            .await
            .unwrap();
            assert_eq!(created.as_deref(), Some(run_id.as_str()));

            let bin = tempfile::tempdir().unwrap();
            let log = bin.path().join("args");
            let program = bin.path().join("claude");
            std::fs::write(
            &program,
            format!(
                "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > '{log}'\ncat > '{log}.stdin'\necho '{{\"type\":\"system\",\"subtype\":\"init\",\"session_id\":\"s-1\"}}'\necho '{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"Three notes mention the launch.\"}}]}}}}'\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"Three notes mention the launch.\"}}'\n",
                log = log.display()
            ),
        )
        .unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            let context = crate::chat_engine::turn::TurnContext {
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                data_partition: "default".into(),
                previous_run_id: None,
                kind: CliKind::Claude,
                program,
                env: std::collections::BTreeMap::from([(
                    "PATH".to_string(),
                    "/usr/bin:/bin".to_string(),
                )]),
                agy: None,
                workspace: workspace.path().to_path_buf(),
                input: routine_cli_prompt(&claimed),
                mcp: None,
                turns: None,
                safety_mode: claimed.safety_mode,
                attended: false,
                surface: "routine".into(),
                routine_tools: Some(
                    crate::chat_engine::policy::RoutineBuiltinTools::from_toolsets(
                        &claimed.enabled_toolsets,
                    ),
                ),
                images: Vec::new(),
            };
            let sink = RoutineSink {
                repository: repository.clone(),
            };
            let (_cancel, receiver) = tokio::sync::watch::channel(false);
            crate::chat_engine::turn::execute_turn(&repository, &sink, context, receiver).await;

            let args = std::fs::read_to_string(&log).unwrap();
            assert!(args.lines().any(|line| line == "--tools"), "{args}");
            let args_list = args.lines().collect::<Vec<_>>();
            let tools_at = args_list
                .iter()
                .position(|value| *value == "--tools")
                .unwrap();
            assert_eq!(args_list[tools_at + 1], expected_tools);
            let allowed_at = args_list
                .iter()
                .position(|value| *value == "--allowedTools")
                .unwrap();
            let expected_allowed = if expected_tools.is_empty() {
                "mcp__clovy__*".to_string()
            } else {
                format!("{expected_tools},mcp__clovy__*")
            };
            assert_eq!(args_list[allowed_at + 1], expected_allowed);
            assert!(
                args.lines().any(|line| line == "--strict-mcp-config"),
                "{args}"
            );
            let stdin = std::fs::read_to_string(format!("{}.stdin", log.display())).unwrap();
            assert!(stdin.contains("running unattended"), "{stdin}");
            assert!(stdin.ends_with("Summarize notes"), "{stdin}");

            // The routine history records the CLI's result like a Clovy run's.
            let runs = list_runs(&pool, Some("routine-1")).await.unwrap();
            assert_eq!(runs.len(), 1);
            assert_eq!(runs[0].status, "completed");
            assert_eq!(runs[0].agent_run_id.as_deref(), Some(run_id.as_str()));
            let answer = repository
                .items(&session_id)
                .await
                .unwrap()
                .into_iter()
                .find_map(|item| match item.payload {
                    AgentItemPayload::AssistantMessage(message) => Some(message.content),
                    _ => None,
                });
            assert_eq!(answer.as_deref(), Some("Three notes mention the launch."));
            let routine =
                query("SELECT last_status, claim_token FROM routines WHERE id = 'routine-1'")
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(routine.get::<Option<String>, _>("claim_token"), None);
            assert_eq!(
                routine.get::<Option<String>, _>("last_status").as_deref(),
                Some("ok")
            );
        }
    }
}

#[cfg(test)]
#[path = "routine_dispatch_failure_tests.rs"]
mod dispatch_failure_tests;

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::llm::{cli::CliKind, registry::ProviderRef, shell_env::LoginEnv};
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    pub(super) fn environment(installed: &[CliKind]) -> (tempfile::TempDir, LoginEnv) {
        let directory = tempfile::tempdir().unwrap();
        for kind in installed {
            let path = directory.path().join(if cfg!(windows) {
                format!("{}.exe", kind.id())
            } else {
                kind.id().to_string()
            });
            std::fs::write(&path, "#!/bin/sh\nexit 91\n").unwrap();
            #[cfg(unix)]
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let env = LoginEnv::from_vars(BTreeMap::from([
            ("PATH".into(), directory.path().display().to_string()),
            ("PATHEXT".into(), ".EXE".into()),
        ]));
        (directory, env)
    }
    async fn fixture(model: &str) -> SqlitePool {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE agent_sessions(id TEXT PRIMARY KEY)",
            "CREATE TABLE agent_runs(id TEXT PRIMARY KEY)",
            "CREATE TABLE connector_triggers(id TEXT,job_id TEXT,kind TEXT,account_id TEXT)",
        ] {
            query(sql).execute(&pool).await.unwrap();
        }
        for sql in include_str!("../migrations/026_routines.sql")
            .split(';')
            .filter(|sql| !sql.trim().is_empty())
        {
            query(sql).execute(&pool).await.unwrap();
        }
        query("INSERT INTO routines(id,name,prompt,schedule,model,created_at,updated_at,metadata_json,tool_catalog_version) VALUES ('selection','Synthetic','Keep notes','every 1h',?,'now','now','{}',1)")
            .bind(model).execute(&pool).await.unwrap();
        pool
    }
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            crate::providers::replace_current_settings_for_tests(
                crate::providers::default_settings_for_tests(),
            );
        }
    }
    #[tokio::test]
    async fn selection41_routine_claim_uses_only_installed_eligible_cli_independently_of_chat_and_notes(
    ) {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let _restore = Restore;
        let mut settings = crate::providers::default_settings_for_tests();
        settings.llm_usage.chat = ProviderRef::Cli { id: CliKind::Agy };
        settings.llm_usage.notes = ProviderRef::Cli { id: CliKind::Codex };
        settings.llm_cli_default_applied = true;
        crate::providers::replace_current_settings_for_tests(settings);
        for only in [CliKind::Claude, CliKind::Codex] {
            let (_directory, env) = environment(&[only]);
            let pool = fixture("auto").await;
            let claimed = claim_with_connector_guard_for_turn(
                &pool,
                "selection",
                "manual",
                false,
                None,
                None,
                &env,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(claimed.model, crate::chat_engine::engine_model_id(only));
            assert_eq!(
                query("SELECT model FROM routine_runs")
                    .fetch_one(&pool)
                    .await
                    .unwrap()
                    .get::<String, _>("model"),
                claimed.model
            );
            assert_eq!(
                crate::llm::provider_for(crate::llm::registry::LlmUsage::Chat),
                ProviderRef::Cli { id: CliKind::Agy }
            );
            assert_eq!(
                crate::llm::provider_for(crate::llm::registry::LlmUsage::Notes),
                ProviderRef::Cli { id: CliKind::Codex }
            );
        }
    }
    #[tokio::test]
    async fn selection41_refused_engine_leaves_no_claim_run_or_effect() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let _restore = Restore;
        for (model, installed, code) in [
            ("auto", vec![], "routine_engine_unavailable"),
            ("auto", vec![CliKind::Agy], "routine_engine_unavailable"),
            (
                "__clovy_cli_engine__:agy",
                vec![CliKind::Agy, CliKind::Codex],
                "routine_engine_not_allowed",
            ),
            (
                "__clovy_cli_engine__:claude",
                vec![CliKind::Codex],
                "llm_cli_not_installed",
            ),
            (
                "__clovy_cli_engine__:codex",
                vec![CliKind::Claude],
                "llm_cli_not_installed",
            ),
            (
                "__clovy_cli_engine__:pi",
                vec![CliKind::Pi, CliKind::Claude],
                "routine_engine_not_allowed",
            ),
            (
                "explicit-remote-model",
                vec![CliKind::Codex],
                "routine_engine_not_allowed",
            ),
        ] {
            let (_directory, env) = environment(&installed);
            let pool = fixture(model).await;
            let error = claim_with_connector_guard_for_turn(
                &pool,
                "selection",
                "manual",
                false,
                None,
                None,
                &env,
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, code);
            let row = query("SELECT claim_token,model FROM routines")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(row.get::<Option<String>, _>("claim_token"), None);
            assert_eq!(row.get::<String, _>("model"), model);
            for table in ["routine_runs", "agent_sessions", "agent_runs"] {
                assert_eq!(
                    query(&format!("SELECT COUNT(*) AS n FROM {table}"))
                        .fetch_one(&pool)
                        .await
                        .unwrap()
                        .get::<i64, _>("n"),
                    0
                );
            }
        }
    }

    #[derive(Clone, Default)]
    struct SelectionDiagnostics(std::sync::Arc<std::sync::Mutex<Vec<BTreeMap<String, String>>>>);
    impl tracing::Subscriber for SelectionDiagnostics {
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            *metadata.level() == tracing::Level::WARN
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Fields(BTreeMap<String, String>);
            impl tracing::field::Visit for Fields {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    self.0
                        .insert(field.name().to_string(), format!("{value:?}"));
                }
            }
            let mut fields = Fields(BTreeMap::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }

    async fn assert_batch_continues_after_refusal(model: &str, code: &str) {
        use tracing::instrument::WithSubscriber;
        let (_directory, env) = environment(&[CliKind::Codex]);
        let pool = fixture("auto").await;
        query("DELETE FROM routines").execute(&pool).await.unwrap();
        for (id, model) in [
            ("A", "auto"),
            ("B", model),
            ("C", "__clovy_cli_engine__:codex"),
        ] {
            query("INSERT INTO routines(id,name,prompt,schedule,model,next_run_at,created_at,updated_at,metadata_json,tool_catalog_version) VALUES (?,?,'Keep notes','every 1h',?,'2000-01-01T00:00:00Z','now','now','{}',1)")
                .bind(id).bind(id).bind(model).execute(&pool).await.unwrap();
        }
        let diagnostics = SelectionDiagnostics::default();
        let result = claim_due_with_env(&pool, &env)
            .with_subscriber(diagnostics.clone())
            .await;
        // A is already committed even on the old failing path; dropping its
        // returned batch would strand this exact durable claim.
        let first = query("SELECT claim_token FROM routines WHERE id='A'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(first.get::<Option<String>, _>("claim_token").is_some());
        let claims = result.expect("B's selection refusal must not discard A or prevent C");
        let delivered = claims
            .into_iter()
            .map(|claim| {
                (
                    claim.routine_id,
                    claim.routine_run_id,
                    claim.token,
                    claim.model,
                )
            })
            .collect::<Vec<_>>();
        let rows = query(
            "SELECT routine_id,id,claim_token,model,status FROM routine_runs ORDER BY routine_id",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let persisted = rows
            .iter()
            .map(|row| {
                assert_eq!(row.get::<String, _>("status"), "queued");
                (
                    row.get::<String, _>("routine_id"),
                    row.get::<String, _>("id"),
                    row.get::<String, _>("claim_token"),
                    row.get::<String, _>("model"),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(delivered, persisted);
        assert_eq!(
            delivered
                .iter()
                .map(|claim| claim.0.as_str())
                .collect::<Vec<_>>(),
            ["A", "C"]
        );
        for (id, _, token, resolved) in &delivered {
            assert_eq!(resolved, "__clovy_cli_engine__:codex");
            assert!(!token.is_empty());
            let saved: String = query("SELECT claim_token FROM routines WHERE id=?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("claim_token");
            assert_eq!(&saved, token);
        }
        let refused = query("SELECT model,claim_token FROM routines WHERE id='B'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(refused.get::<String, _>("model"), model);
        assert_eq!(refused.get::<Option<String>, _>("claim_token"), None);
        {
            let events = diagnostics.0.lock().unwrap();
            let event = events
                .iter()
                .find(|fields| fields.get("routine_id").is_some_and(|id| id == "B"))
                .expect("refusal must emit an identified diagnostic");
            assert_eq!(event.get("error_code").map(String::as_str), Some(code));
            assert!(event.get("error").is_some_and(|cause| !cause.is_empty()));
        }
        assert!(claim_due_with_env(&pool, &env)
            .with_subscriber(diagnostics)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            query("SELECT COUNT(*) AS n FROM routine_runs")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<i64, _>("n"),
            2
        );
    }

    #[tokio::test]
    async fn post_review_f12_missing_explicit_engine_does_not_strand_confirmed_or_later_claims() {
        assert_batch_continues_after_refusal(
            "__clovy_cli_engine__:claude",
            "llm_cli_not_installed",
        )
        .await;
    }

    #[tokio::test]
    async fn post_review_f12_ineligible_engine_does_not_strand_confirmed_or_later_claims() {
        assert_batch_continues_after_refusal(
            "__clovy_cli_engine__:agy",
            "routine_engine_not_allowed",
        )
        .await;
    }

    #[tokio::test]
    async fn post_review_f12_storage_failure_is_not_an_engine_refusal() {
        use tracing::instrument::WithSubscriber;
        let (_directory, env) = environment(&[CliKind::Codex]);
        let pool = fixture("auto").await;
        query("UPDATE routines SET next_run_at='2000-01-01T00:00:00Z'")
            .execute(&pool)
            .await
            .unwrap();
        query("CREATE TRIGGER reject_claim BEFORE UPDATE OF claim_token ON routines BEGIN SELECT RAISE(ABORT, 'synthetic storage failure'); END")
            .execute(&pool).await.unwrap();
        let diagnostics = SelectionDiagnostics::default();
        let error = claim_due_with_env(&pool, &env)
            .with_subscriber(diagnostics.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code, "routine_storage_failed");
        assert!(error.message.contains("synthetic storage failure"));
        assert!(diagnostics
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|event| !event.contains_key("routine_id")));
        assert_eq!(
            query("SELECT COUNT(*) AS n FROM routine_runs")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<i64, _>("n"),
            0
        );
        assert_eq!(
            query("SELECT claim_token FROM routines")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<Option<String>, _>("claim_token"),
            None
        );
    }
}
