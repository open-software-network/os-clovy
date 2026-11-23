//! Memory mutations share the native settings-locked creation path and the
//! repository's transactional deletion/tombstone path.
use super::{ToolCtx, ToolGroup};
use crate::approval_broker::{external::ExternalCall, ApprovalKind, ApprovalRequest};
use crate::db::repositories::{mcp_project_target, Repositories};
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::{query::query, row::Row};
use std::path::Path;
use tauri::Emitter;

pub struct MemoryTools;
impl ToolGroup for MemoryTools {
    fn owns(&self, name: &str) -> bool {
        matches!(
            name,
            "save_memory" | "forget_memory" | "update_memory" | "search_memories"
        )
    }
    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        [
            ("save_memory","Save a durable fact, preference, or decision in Clovy's memory store.",json!({"content":{"type":"string","maxLength":4000},"projectId":{"type":"string"}}),vec!["content"]),
            ("search_memories","Search authorized memories, filtering scope and disabled projects before pagination.",json!({"query":{"type":"string","minLength":1},"projectId":{"type":"string"},"includeGlobal":{"type":"boolean","default":true},"limit":{"type":"integer","minimum":1,"maximum":50},"offset":{"type":"integer","minimum":0}}),vec!["query"]),
            ("update_memory","Update memory content while preserving its id, source, creation time and project scope.",json!({"id":{"type":"string"},"content":{"type":"string","maxLength":4000}}),vec!["id","content"]),
            ("forget_memory","Permanently forget one Clovy memory by id.",json!({"id":{"type":"string"}}),vec!["id"]),
        ].into_iter().map(|(name,description,properties,required)| json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":name=="search_memories","openWorldHint":false}})).collect()
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let app = ctx.app.ok_or_else(|| {
                AppError::new(
                    "approval_unavailable",
                    "Clovy's memory settings are unavailable.",
                )
            })?;
            let path = crate::commands::memory_settings_path(app)?;
            call_with_settings(ctx, name, arguments, &path).await
        })
    }
}

/// The app resolves its settings path per call. Tests supply a temporary file,
/// keeping the same broker, validation and persistence path without Tauri.
pub(super) async fn call_with_settings(
    ctx: &ToolCtx<'_>,
    name: &str,
    arguments: &Value,
    settings_path: &Path,
) -> Result<Value, AppError> {
    let repos = Repositories::new(ctx.data.notes.clone());
    call_authorized(ctx, None, name, arguments, settings_path, &repos).await
}

/// The same MCP admission path with an explicit repository, so tests can pause
/// real persistence boundaries without replacing approval or turn validation.
#[cfg(test)]
pub(super) async fn call_with_repositories(
    ctx: &ToolCtx<'_>,
    name: &str,
    arguments: &Value,
    settings_path: &Path,
    repos: &Repositories,
) -> Result<Value, AppError> {
    call_authorized(ctx, None, name, arguments, settings_path, repos).await
}

pub(super) async fn call_external(
    ctx: &ToolCtx<'_>,
    external: &ExternalCall<'_>,
    name: &str,
    arguments: &Value,
    settings_path: &Path,
) -> Result<Value, AppError> {
    let repos = Repositories::new(ctx.data.notes.clone());
    call_authorized(ctx, Some(external), name, arguments, settings_path, &repos).await
}

async fn call_authorized(
    ctx: &ToolCtx<'_>,
    external: Option<&ExternalCall<'_>>,
    name: &str,
    arguments: &Value,
    settings_path: &Path,
    repos: &Repositories,
) -> Result<Value, AppError> {
    ctx.revalidate_local(external).await?;
    ensure_enabled(ctx, settings_path).await?;
    let project_id = match arguments.get("projectId") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    AppError::new("invalid_arguments", "projectId must be a non-empty string.")
                })?,
        ),
    };
    let id = if matches!(name, "forget_memory" | "update_memory") {
        Some(required(arguments, "id")?)
    } else {
        None
    };
    let content = if matches!(name, "save_memory" | "update_memory") {
        Some(crate::commands::validated_memory_content(required(
            arguments, "content",
        )?)?)
    } else {
        None
    };
    check_scope(ctx, project_id, id).await?;
    if name == "search_memories" {
        return search_memories(ctx, arguments, project_id).await;
    }
    let target = match (external, id, project_id) {
        (Some(_), Some(id), _) => Some(memory_target(ctx, id).await?),
        (Some(_), None, Some(project)) => {
            Some(mcp_project_target(&ctx.data.notes, &ctx.data.profile, project).await?)
        }
        _ => None,
    };
    let authority = ctx
        .approve_local(
            external,
            ApprovalRequest {
                tool: name.into(),
                call_id: Some(uuid::Uuid::new_v4().to_string()),
                kind: ApprovalKind::Approval {
                    title: if name == "save_memory" {
                        "Save a memory"
                    } else if name == "update_memory" {
                        "Update a memory"
                    } else {
                        "Forget a memory"
                    }
                    .into(),
                    description: if name != "forget_memory" {
                        "Save the requested fact in Clovy's memory store."
                    } else {
                        "Permanently delete the selected Clovy memory."
                    }
                    .into(),
                    arguments: arguments.clone(),
                    preview: None,
                },
            },
            target.as_ref(),
        )
        .await?;
    ctx.revalidate_local(external).await?;
    // Settings or project membership may change while the card waits.
    ensure_enabled(ctx, settings_path).await?;
    check_scope(ctx, project_id, id).await?;
    let result = match name {
        "save_memory" => {
            let memory = crate::commands::create_memory_with_settings_authorized(
                repos,
                settings_path,
                &ctx.data.profile,
                project_id,
                content.unwrap_or_default(),
                "agent",
                Some(authority),
            )
            .await?;
            serde_json::to_value(memory)
                .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
        }
        "update_memory" => {
            let memory = crate::commands::update_memory_with_settings_authorized(
                repos,
                settings_path,
                &ctx.data.profile,
                id.unwrap_or_default(),
                content.unwrap_or_default(),
                Some(authority),
            )
            .await?;
            serde_json::to_value(memory)
                .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
        }
        "forget_memory" => {
            let id = id.unwrap_or_default();
            crate::commands::forget_memory_with_settings_authorized(
                repos,
                settings_path,
                &ctx.data.profile,
                id,
                Some(authority),
            )
            .await?;
            Ok(json!({"forgotten":true,"id":id}))
        }
        _ => Err(AppError::new("mcp_tool_unknown", "Unknown memory tool.")),
    }?;
    if let Some(app) = ctx.app {
        let _ = app.emit(
            "clovy-data-changed",
            json!({"profile":ctx.data.profile,"kind":"memories","id":id}),
        );
    }
    Ok(result)
}

async fn memory_target(ctx: &ToolCtx<'_>, id: &str) -> Result<Value, AppError> {
    let row =
        query("SELECT folder_id, content, updated_at FROM memories WHERE id = ? AND profile = ?")
            .bind(id)
            .bind(&ctx.data.profile)
            .fetch_optional(&ctx.data.notes)
            .await?
            .ok_or_else(|| AppError::new("memory_not_found", "Memory was not found."))?;
    Ok(
        json!({"id":id,"projectId":row.get::<Option<String>,_>("folder_id"),"content":row.get::<String,_>("content"),"updatedAt":row.get::<String,_>("updated_at")}),
    )
}

async fn ensure_enabled(ctx: &ToolCtx<'_>, path: &Path) -> Result<(), AppError> {
    if ctx.data.memory_enabled && crate::commands::load_memory_settings(path).enabled {
        Ok(())
    } else {
        Err(AppError::new(
            "memory_disabled",
            "Memory is disabled for this scope.",
        ))
    }
}
async fn check_scope(
    ctx: &ToolCtx<'_>,
    project_id: Option<&str>,
    memory_id: Option<&str>,
) -> Result<(), AppError> {
    let stored_project: Option<String> = if let Some(id) = memory_id {
        let row = query("SELECT folder_id FROM memories WHERE id = ? AND profile = ?")
            .bind(id)
            .bind(&ctx.data.profile)
            .fetch_optional(&ctx.data.notes)
            .await?
            .ok_or_else(|| AppError::new("memory_not_found", "Memory was not found."))?;
        row.get("folder_id")
    } else {
        project_id.map(str::to_string)
    };
    if let Some(project) = stored_project {
        let row = query("SELECT memory_disabled FROM folders WHERE id = ? AND profile = ? AND deleted_at IS NULL").bind(project).bind(&ctx.data.profile).fetch_optional(&ctx.data.notes).await?.ok_or_else(|| AppError::new("folder_not_found","Project was not found or has already been deleted."))?;
        if row.get::<i64, _>("memory_disabled") != 0 {
            return Err(AppError::new(
                "memory_disabled",
                "Memory is disabled for this scope.",
            ));
        }
    }
    Ok(())
}
fn required<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, AppError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::new(
                "agent_tool_arguments_invalid",
                format!("{key} is required."),
            )
        })
}

async fn search_memories(
    ctx: &ToolCtx<'_>,
    arguments: &Value,
    project_id: Option<&str>,
) -> Result<Value, AppError> {
    let needle = required(arguments, "query")?.trim();
    if needle.is_empty() {
        return Err(AppError::new(
            "invalid_arguments",
            "query must not be empty.",
        ));
    }
    let include_global = arguments
        .get("includeGlobal")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let limit = arguments
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(20)
        .clamp(1, 50);
    let offset = arguments
        .get("offset")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0);
    // Escape LIKE metacharacters so search is literal. Scope, project existence
    // and enablement are part of the SQL before LIMIT/OFFSET.
    let pattern = format!(
        "%{}%",
        needle
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let rows = query("SELECT m.id, m.folder_id, m.content, m.source, m.created_at, m.updated_at FROM memories m WHERE m.profile = ? AND ((m.folder_id IS NULL AND (? OR ? IS NULL)) OR (m.folder_id IS NOT NULL AND (? IS NOT NULL OR ?) AND (? IS NULL OR m.folder_id = ?) AND EXISTS (SELECT 1 FROM folders f WHERE f.id = m.folder_id AND f.profile = m.profile AND f.deleted_at IS NULL AND f.memory_disabled = 0))) AND m.content LIKE ? ESCAPE '\\' ORDER BY m.created_at DESC, m.id DESC LIMIT ? OFFSET ?")
        .bind(&ctx.data.profile).bind(include_global).bind(project_id).bind(project_id).bind(include_global).bind(project_id).bind(project_id).bind(pattern).bind(limit + 1).bind(offset)
        .fetch_all(&ctx.data.notes).await?;
    let has_more = rows.len() > limit as usize;
    let items = rows.into_iter().take(limit as usize).map(|row| json!({"id":row.get::<String,_>("id"),"projectId":row.get::<Option<String>,_>("folder_id"),"content":row.get::<String,_>("content"),"source":row.get::<String,_>("source"),"createdAt":row.get::<String,_>("created_at"),"updatedAt":row.get::<String,_>("updated_at")})).collect::<Vec<_>>();
    Ok(
        json!({"items":items,"count":items.len(),"offset":offset,"hasMore":has_more,"nextOffset":has_more.then_some(offset + items.len() as i64)}),
    )
}
