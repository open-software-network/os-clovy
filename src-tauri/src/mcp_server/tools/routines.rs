//! Routine management uses durable claims and the unattended CLI domain path.
use super::{ToolCtx, ToolGroup};
use crate::approval_broker::{ApprovalKind, ApprovalRequest, ApprovalTurn};
use crate::domain::types::AppError;
use crate::mcp_server::turns::TURN_REQUIRED;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tauri::Emitter;

pub struct RoutineTools;
const NAMES: [&str; 8] = [
    "list_routines",
    "create_routine",
    "update_routine",
    "pause_routine",
    "resume_routine",
    "delete_routine",
    "run_routine",
    "list_routine_runs",
];
fn read_only(name: &str) -> bool {
    matches!(name, "list_routines" | "list_routine_runs")
}

impl ToolGroup for RoutineTools {
    fn owns(&self, name: &str) -> bool {
        NAMES.contains(&name)
    }
    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        NAMES.into_iter().map(|name| {
            let mut properties = json!({});
            let mut required = Vec::new();
            if matches!(name, "create_routine" | "update_routine") {
                properties = json!({
                    "name":{"type":"string","minLength":1}, "prompt":{"type":"string","minLength":1},
                    "schedule":{"type":"string","description":"RFC 3339, every <n>m/h/d, or a five-field cron expression."},
                    "timezone":{"type":"string","description":"IANA timezone, for example America/Sao_Paulo; defaults to UTC."},
                    "model":{"type":"string","description":"auto or an installed CLI chat engine model id. Execution resolves to the supported unattended CLI, never an endpoint model."},
                    "safetyMode":{"type":"string","enum":["sandboxed","unrestricted"]},
                    "enabledToolsets":{"type":"array","items":{"type":"string"},"uniqueItems":true,"description":"Capability selection only. Does not create trust, grants, browser access or approval exemptions."},
                    "repeat":{"type":"string"},"deliver":{"type":"string","enum":["local"]},
                    "enabled":{"type":"boolean"},"state":{"type":"string","enum":["scheduled","paused","completed","needs_review"]}
                });
            }
            if !matches!(name, "list_routines" | "create_routine") {
                properties["routineId"] = json!({"type":"string","minLength":1});
                if name != "list_routine_runs" { required.push("routineId"); }
            }
            if name == "create_routine" { required.extend(["prompt", "schedule", "safetyMode"]); }
            if name == "list_routine_runs" {
                properties["limit"] = json!({"type":"integer","minimum":1,"maximum":100});
                properties["offset"] = json!({"type":"integer","minimum":0});
            }
            json!({"name":name,"description":description(name),"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":read_only(name),"openWorldHint":name=="run_routine"}})
        }).collect()
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let binding = ctx.binding.ok_or_else(|| {
                AppError::new(TURN_REQUIRED, "Routine tools require a Clovy turn.")
            })?;
            ctx.revalidate().await?;
            validate_arguments(self, ctx, name, arguments)?;
            // Resolve objects before and after the approval wait. A stale id
            // cannot turn an approved change into a different operation.
            if !matches!(
                name,
                "list_routines" | "create_routine" | "list_routine_runs"
            ) {
                crate::routines::get(&ctx.data.notes, routine_id(arguments)?).await?;
            }
            if !read_only(name) {
                ctx.broker()?
                    .request(
                        &ApprovalTurn::from(binding),
                        ApprovalRequest {
                            tool: name.into(),
                            call_id: Some(uuid::Uuid::new_v4().to_string()),
                            kind: ApprovalKind::Approval {
                                title: title(name).into(),
                                description: description(name).into(),
                                arguments: arguments.clone(),
                                preview: None,
                            },
                        },
                    )
                    .await
                    .map_err(|denial| AppError::new(denial.code(), denial.message()))?;
                ctx.revalidate().await?;
            }
            let pool = &ctx.data.notes;
            let turn = Some((
                ctx.data.profile.as_str(),
                binding.session_id.as_str(),
                binding.run_id.as_str(),
                &binding.authority,
            ));
            let result = match name {
                "list_routines" | "list_routine_runs" => {
                    return read(ctx, name, arguments, false).await
                }
                "create_routine" => serde_json::to_value(
                    crate::routines::create_for_turn(
                        pool,
                        serde_json::from_value(arguments.clone()).map_err(invalid_arguments)?,
                        turn,
                    )
                    .await?,
                ),
                "update_routine" => serde_json::to_value(
                    crate::routines::update_for_turn(
                        pool,
                        serde_json::from_value(arguments.clone()).map_err(invalid_arguments)?,
                        turn,
                    )
                    .await?,
                ),
                "pause_routine" => serde_json::to_value(
                    crate::routines::pause_for_turn(pool, routine_id(arguments)?, turn).await?,
                ),
                "resume_routine" => serde_json::to_value(
                    crate::routines::resume_for_turn(pool, routine_id(arguments)?, turn).await?,
                ),
                "delete_routine" => {
                    let id = routine_id(arguments)?;
                    crate::routines::delete_for_turn(pool, id, turn).await?;
                    Ok(json!({"deleted":true,"routineId":id}))
                }
                "run_routine" => {
                    let app = ctx.app.ok_or_else(|| {
                        AppError::new(
                            "mcp_app_unavailable",
                            "Routine execution requires the running Clovy app.",
                        )
                    })?;
                    let id = routine_id(arguments)?;
                    let run =
                        crate::routines::trigger_and_start_for_turn(app, pool, id, turn).await?;
                    match run {
                        Some(run) => serde_json::to_value(run),
                        None => return Err(crate::routines::trigger_blocked_error(pool, id).await?),
                    }
                }
                _ => return Err(AppError::new("mcp_tool_unknown", "Unknown routine tool.")),
            }
            .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))?;
            if !read_only(name) {
                if let Some(app) = ctx.app {
                    let _ = app.emit("clovy-data-changed", json!({"profile":ctx.data.profile,"kind":"routines","id":arguments.get("routineId")}));
                }
            }
            Ok(result)
        })
    }
}
/// Routine status is installation-wide. External projection omits the saved
/// prompt, grants, execution metadata, and linked conversation/run IDs.
pub(super) async fn read(
    ctx: &ToolCtx<'_>,
    name: &str,
    arguments: &Value,
    external: bool,
) -> Result<Value, AppError> {
    let pool = &ctx.data.notes;
    if name == "list_routines" {
        let routines = crate::routines::list(pool).await?;
        if !external {
            return serde_json::to_value(routines)
                .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()));
        }
        return Ok(json!(routines.into_iter().map(|routine| json!({
            "id":routine.id,"name":routine.name,"schedule":routine.schedule,
            "timezone":routine.timezone,"state":routine.state,"enabled":routine.enabled,
            "nextRunAt":routine.next_run_at,"lastRunAt":routine.last_run_at,"lastStatus":routine.last_status
        })).collect::<Vec<_>>()));
    }
    let id = arguments.get("routineId").and_then(Value::as_str);
    if let Some(id) = id {
        crate::routines::get(pool, id).await?;
    }
    let limit = arguments
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(20)
        .clamp(1, 100);
    let offset = arguments
        .get("offset")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    let mut runs = crate::routines::list_runs_page(pool, id, limit + 1, offset).await?;
    let has_more = runs.len() > limit as usize;
    runs.truncate(limit as usize);
    let items = if external {
        json!(runs
            .into_iter()
            .map(|run| json!({
                "id":run.id,"routineId":run.routine_id,"triggerKind":run.trigger_kind,
                "status":run.status,"scheduledFor":run.scheduled_for,"startedAt":run.started_at,
                "completedAt":run.completed_at,"model":run.model,"errorCode":run.error_code
            }))
            .collect::<Vec<_>>())
    } else {
        json!(runs)
    };
    Ok(
        json!({"items":items,"offset":offset,"hasMore":has_more,"nextOffset":has_more.then_some(offset.saturating_add(limit))}),
    )
}

fn invalid_arguments(error: serde_json::Error) -> AppError {
    AppError::new("invalid_arguments", error.to_string())
}
fn routine_id(arguments: &Value) -> Result<&str, AppError> {
    arguments
        .get("routineId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| AppError::new("agent_tool_arguments_invalid", "routineId is required."))
}
fn title(name: &str) -> &'static str {
    match name {
        "create_routine" => "Create a routine",
        "update_routine" => "Update a routine",
        "pause_routine" => "Pause a routine",
        "resume_routine" => "Resume a routine",
        "delete_routine" => "Delete a routine",
        "run_routine" => "Run a routine",
        _ => "List routines",
    }
}
fn description(name: &str) -> &'static str {
    match name {
        "create_routine" => "Create a Clovy routine with the requested instructions and timing.",
        "update_routine" => "Update routine configuration. Toolset selection does not grant authority.",
        "pause_routine" => "Pause future runs of the selected Clovy routine.",
        "resume_routine" => "Resume scheduled runs of the selected Clovy routine.",
        "delete_routine" => "Permanently delete the selected Clovy routine.",
        "run_routine" => "Start the selected routine through its single-flight unattended CLI path and saved grants. May incur the CLI provider's usage costs. Paused or active routines refuse execution.",
        "list_routine_runs" => "Read paginated routine execution history, status, CLI model and errors.",
        _ => "List Clovy routines and their schedules.",
    }
}

fn validate_arguments(
    group: &RoutineTools,
    ctx: &ToolCtx<'_>,
    name: &str,
    arguments: &Value,
) -> Result<(), AppError> {
    let object = arguments.as_object().ok_or_else(|| {
        AppError::new("invalid_arguments", "Routine arguments must be an object.")
    })?;
    let descriptor = group
        .descriptors(ctx)
        .into_iter()
        .find(|tool| tool["name"] == name)
        .ok_or_else(|| AppError::new("mcp_tool_unknown", "Unknown routine tool."))?;
    let properties = descriptor["inputSchema"]["properties"]
        .as_object()
        .ok_or_else(|| AppError::new("invalid_arguments", "Routine schema is unavailable."))?;
    if object.keys().any(|key| !properties.contains_key(key)) {
        return Err(AppError::new(
            "invalid_arguments",
            "Unknown routine argument. Host ids, metadata and grants cannot be supplied.",
        ));
    }
    Ok(())
}
