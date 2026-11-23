//! Typed dictation management; the destination is always the calling
//! conversation's Clovy composer, pinned before asking for approval.
use super::{ToolCtx, ToolGroup};
use crate::approval_broker::{external::ExternalCall, ApprovalKind, ApprovalRequest, ApprovalTurn};
use crate::db::repositories::McpMutation;
use crate::dictation::{
    agent_dictation_control, agent_dictation_status, agent_dictation_target, AgentDictationAction,
};
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::row::Row;
use tauri::Emitter;

pub struct DictationTools;
const TOOLS: [&str; 5] = [
    "delete_dictation",
    "start_dictation",
    "stop_dictation",
    "cancel_dictation",
    "get_dictation_status",
];
impl ToolGroup for DictationTools {
    fn owns(&self, name: &str) -> bool {
        TOOLS.contains(&name)
    }
    fn descriptors(&self, _: &ToolCtx<'_>) -> Vec<Value> {
        TOOLS.iter().map(|name| {
            let (description, properties, required) = match *name {
                "delete_dictation" => ("Delete a retained dictation from the bound data partition.", json!({"id":{"type":"string","minLength":1}}), json!(["id"])),
                "start_dictation" => ("Start a visible dictation take only when explicitly requested. Delivers text to this conversation's Clovy composer, never an arbitrary foreground app. Waits for helper confirmation. Transcription may incur provider charges.", json!({}),json!([])),
                "stop_dictation" => ("Finalize the specified bound dictation take and deliver text to its fixed Clovy composer. Waits for delivery acknowledgement.",json!({"takeId":{"type":"string","minLength":1}}),json!(["takeId"])),
                "cancel_dictation" => ("Discard the specified bound dictation take. Late transcription cannot deliver text.",json!({"takeId":{"type":"string","minLength":1}}),json!(["takeId"])),
                _ => ("Read the helper-confirmed phase and fixed composer destination of this conversation's dictation take.",json!({}),json!([])),
            };
            json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":*name == "get_dictation_status","destructiveHint":matches!(*name,"delete_dictation"|"cancel_dictation"),"openWorldHint":matches!(*name,"start_dictation"|"stop_dictation")}})
        }).collect()
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            ctx.revalidate().await?;
            let binding = ctx
                .binding
                .ok_or_else(|| AppError::new("turn_required", "A bound turn is required."))?;
            let app = ctx.app.ok_or_else(|| {
                AppError::new(
                    "dictation_unavailable",
                    "The dictation service is unavailable.",
                )
            })?;
            if name == "get_dictation_status" {
                return serialize(agent_dictation_status(
                    app,
                    &ctx.data.profile,
                    &binding.session_id,
                )?);
            }
            let target = if name == "start_dictation" {
                Some(agent_dictation_target(
                    &ctx.data.profile,
                    &binding.session_id,
                ))
            } else if matches!(name, "stop_dictation" | "cancel_dictation") {
                let id = required(args, "takeId")?;
                Some(
                    agent_dictation_status(app, &ctx.data.profile, &binding.session_id)?
                        .filter(|take| take.take_id == id)
                        .ok_or_else(|| {
                            AppError::new(
                                "dictation_take_not_found",
                                "The take does not belong to this conversation.",
                            )
                        })?,
                )
            } else {
                None
            };
            let id = if name == "delete_dictation" {
                let id = required(args, "id")?.to_string();
                super::notes::read_dictation(&ctx.data.notes, &ctx.data.profile, &id, None).await?;
                Some(id)
            } else {
                None
            };
            ctx.broker()?.request(&ApprovalTurn::from(binding), ApprovalRequest { tool:name.into(),call_id:Some(uuid::Uuid::new_v4().to_string()),kind:ApprovalKind::Approval { title: "Manage dictation".into(), description: if name == "start_dictation" { "Start a visible dictation and deliver its text to this conversation's composer. Transcription uses your configured provider.".into() } else { "Change the specified dictation in this conversation's data partition.".into() }, arguments: args.clone(),preview:target.as_ref().map(|target|serde_json::json!({"takeId":target.take_id,"conversationId":target.conversation_id,"window":target.window}).to_string()) } }).await.map_err(|error|AppError::new(error.code(),error.message()))?;
            ctx.revalidate().await?;
            if let Some(id) = id {
                crate::dictation::delete_history_authorized(
                    &ctx.data.notes,
                    &ctx.data.profile,
                    &id,
                    McpMutation::Turn(&binding.session_id, &binding.run_id, &binding.authority),
                )
                .await?;
                let _ = app.emit(
                    "clovy-data-changed",
                    json!({"profile":ctx.data.profile,"kind":"dictations","id":id,"deleted":true}),
                );
                return Ok(json!({"success":true,"id":id}));
            }
            let action = match name {
                "start_dictation" => AgentDictationAction::Start,
                "stop_dictation" => AgentDictationAction::Stop,
                "cancel_dictation" => AgentDictationAction::Cancel,
                _ => return Err(AppError::new("mcp_tool_unknown", "Unknown dictation tool.")),
            };
            let mut target = target
                .ok_or_else(|| AppError::new("dictation_take_not_found", "No dictation target."))?;
            target.binding = Some(binding.clone());
            let effect_target = serde_json::to_value(&target).unwrap_or_default();
            let operation = agent_dictation_control(app, action, target);
            tokio::pin!(operation);
            let result = loop {
                tokio::select! {
                    result = &mut operation => break result?,
                    _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                        if let Err(mut error)=ctx.revalidate().await {error.details=Some(json!({"target":effect_target,"delivery":"unknown","effect":"unknown","retrySafe":false}));return Err(error);}
                    },
                }
            };
            if let Err(mut error) = ctx.revalidate().await {
                error.details = Some(
                    json!({"dictationResult":result,"effect":"completed_before_turn_revalidation","retrySafe":false}),
                );
                return Err(error);
            }
            serialize(result)
        })
    }
}
pub(super) async fn delete_external(
    ctx: &ToolCtx<'_>,
    scope: &ExternalCall<'_>,
    args: &Value,
) -> Result<Value, AppError> {
    let id = required(args, "id")?;
    let cutoff = crate::db::repositories::dictation_history_cutoff_timestamp();
    let row =
        super::notes::read_dictation(&ctx.data.notes, &ctx.data.profile, id, Some(&cutoff)).await?;
    let text = row.get::<String, _>("text");
    let target = json!({
        "id": id,
        "createdAt": row.get::<String, _>("created_at"),
        "excerpt": text.chars().take(160).collect::<String>(),
        "textLength": text.chars().count(),
    });
    let approval = scope
        .approve(
            "delete_dictation",
            "Delete a dictation",
            "Permanently delete this dictation from Clovy's history.",
            args,
            Some(&target),
        )
        .await?;
    crate::dictation::delete_history_authorized(
        &ctx.data.notes,
        &ctx.data.profile,
        id,
        McpMutation::External(approval),
    )
    .await?;
    if let Some(app) = ctx.app {
        let _ = app.emit(
            "clovy-data-changed",
            json!({"profile":ctx.data.profile,"kind":"dictations","id":id,"deleted":true}),
        );
    }
    Ok(json!({"success":true,"id":id}))
}

fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty() && v.len() <= 128)
        .ok_or_else(|| {
            AppError::new(
                "agent_tool_arguments_invalid",
                format!("{key} is required."),
            )
        })
}
fn serialize(value: impl serde::Serialize) -> Result<Value, AppError> {
    serde_json::to_value(value)
        .map_err(|e| AppError::new("agent_tool_response_invalid", e.to_string()))
}
