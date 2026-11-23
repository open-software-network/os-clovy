//! Conversation lifecycle mutations approved for the bound turn.
use super::{ToolCtx, ToolGroup};
use crate::approval_broker::{ApprovalKind, ApprovalRequest, ApprovalTurn};
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::query::query;

pub struct ConversationMutations;
const NAMES: &[&str] = &[
    "create_conversation",
    "rename_conversation",
    "archive_conversation",
    "restore_conversation",
    "delete_conversation",
    "cancel_conversation_run",
];
impl ToolGroup for ConversationMutations {
    fn owns(&self, name: &str) -> bool {
        NAMES.contains(&name)
    }
    fn descriptors(&self, _: &ToolCtx<'_>) -> Vec<Value> {
        NAMES.iter().map(|name| {
            let (properties,required,description)=match *name {
                "create_conversation"=>(json!({"title":{"type":"string","minLength":1,"maxLength":200}}),vec!["title"],"Create a conversation in the bound data partition; does not start a run."),
                "rename_conversation"=>(json!({"conversationId":{"type":"string"},"title":{"type":"string","minLength":1,"maxLength":200}}),vec!["conversationId","title"],"Rename a conversation."),
                "archive_conversation"=>(json!({"conversationId":{"type":"string"}}),vec!["conversationId"],"Archive a conversation."),
                "restore_conversation"=>(json!({"conversationId":{"type":"string"}}),vec!["conversationId"],"Restore an archived conversation."),
                "cancel_conversation_run"=>(json!({"conversationId":{"type":"string"},"runId":{"type":"string"}}),vec!["conversationId","runId"],"Cancel another run belonging to the specified conversation. A turn cannot cancel itself."),
                _=>(json!({"conversationId":{"type":"string"}}),vec!["conversationId"],"Delete a conversation without an active run."),
            };
            json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":false,"openWorldHint":false,"destructiveHint":*name=="delete_conversation"}})
        }).collect()
    }
    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let Some(binding) = ctx.binding else {
                return vec![];
            };
            let mut result = vec![];
            for tool in self.descriptors(ctx) {
                if let Some(name) = tool["name"].as_str() {
                    if super::routine_tool_allowed(&ctx.data.notes, binding, name)
                        .await
                        .unwrap_or(false)
                    {
                        result.push(tool);
                    }
                }
            }
            result
        })
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move { mutate(ctx, name, args).await })
    }
}
async fn mutate(ctx: &ToolCtx<'_>, name: &str, args: &Value) -> Result<Value, AppError> {
    ctx.revalidate().await?;
    let binding = ctx
        .binding
        .ok_or_else(|| AppError::new("turn_required", "A bound turn is required."))?;
    if !super::routine_tool_allowed(&ctx.data.notes, binding, name).await? {
        return Err(AppError::new(
            "routine_tool_not_enabled",
            "This tool is not enabled for the routine.",
        ));
    }
    let object = args.as_object().ok_or_else(|| {
        AppError::new(
            "agent_tool_arguments_invalid",
            "Arguments must be an object.",
        )
    })?;
    let allowed: &[&str] = match name {
        "create_conversation" => &["title"],
        "rename_conversation" => &["conversationId", "title"],
        "cancel_conversation_run" => &["conversationId", "runId"],
        "archive_conversation" | "restore_conversation" | "delete_conversation" => {
            &["conversationId"]
        }
        _ => {
            return Err(AppError::new(
                "mcp_tool_unknown",
                "Unknown conversation tool.",
            ))
        }
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(AppError::new(
            "agent_tool_arguments_invalid",
            "Unexpected argument.",
        ));
    }
    let target = if name == "create_conversation" {
        None
    } else {
        Some(
            args.get("conversationId")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| {
                    AppError::new(
                        "agent_tool_arguments_invalid",
                        "conversationId is required.",
                    )
                })?,
        )
    };
    let title = if matches!(name, "create_conversation" | "rename_conversation") {
        Some(
            args.get("title")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && s.chars().count() <= 200)
                .ok_or_else(|| {
                    AppError::new(
                        "agent_tool_arguments_invalid",
                        "title must contain 1 to 200 characters.",
                    )
                })?,
        )
    } else {
        None
    };
    if name == "delete_conversation" && target == Some(binding.session_id.as_str()) {
        return Err(AppError::new(
            "conversation_self_delete",
            "A turn cannot delete its own conversation.",
        ));
    }
    if let Some(id) = target {
        ensure(ctx, id).await?;
    }
    let cancel_run = if name == "cancel_conversation_run" {
        let run = args
            .get("runId")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::new("agent_tool_arguments_invalid", "runId is required."))?;
        if run == binding.run_id {
            return Err(AppError::new(
                "conversation_self_cancel",
                "A turn cannot cancel its own run.",
            ));
        }
        if ctx.app.is_none() {
            return Err(AppError::new(
                "agent_runtime_unavailable",
                "The chat engine host is unavailable.",
            ));
        }
        let found: i64 = sqlx::query_scalar::query_scalar(
            "SELECT COUNT(*) FROM agent_runs WHERE id=? AND session_id=?",
        )
        .bind(run)
        .bind(target)
        .fetch_one(&ctx.data.notes)
        .await?;
        if found == 0 {
            return Err(AppError::new(
                "run_not_found",
                "Run was not found in this conversation.",
            ));
        }
        Some(run)
    } else {
        None
    };
    ctx.broker()?
        .request(
            &ApprovalTurn::from(binding),
            ApprovalRequest {
                tool: name.into(),
                call_id: Some(uuid::Uuid::new_v4().to_string()),
                kind: ApprovalKind::Approval {
                    title: match name {
                        "create_conversation" => "Create a conversation",
                        "rename_conversation" => "Rename a conversation",
                        "archive_conversation" => "Archive a conversation",
                        "restore_conversation" => "Restore a conversation",
                        "cancel_conversation_run" => "Cancel a conversation run",
                        _ => "Delete a conversation",
                    }
                    .into(),
                    description: "Change a conversation in the bound data partition.".into(),
                    arguments: args.clone(),
                    preview: None,
                },
            },
        )
        .await
        .map_err(|e| AppError::new(e.code(), e.message()))?;
    ctx.revalidate().await?;
    if !super::routine_tool_allowed(&ctx.data.notes, binding, name).await? {
        return Err(AppError::new(
            "routine_tool_not_enabled",
            "This tool is not enabled for the routine.",
        ));
    }
    if let Some(run_id) = cancel_run {
        return crate::agent_sessions::api::cancel_conversation_run_in_profile(
            ctx.app.ok_or_else(|| {
                AppError::new(
                    "agent_runtime_unavailable",
                    "The chat engine host is unavailable.",
                )
            })?,
            &ctx.data.notes,
            &ctx.data.profile,
            &binding.session_id,
            &binding.run_id,
            target.unwrap_or_default(),
            run_id,
            &binding.authority,
        )
        .await;
    }
    let mut tx = ctx.data.notes.begin().await?;
    crate::db::repositories::guard_bound_turn(
        &mut tx,
        &ctx.data.profile,
        &binding.session_id,
        &binding.run_id,
        &binding.authority,
    )
    .await?;
    let now = crate::db::repositories::timestamp();
    let id = if let Some(id) = target {
        let count:i64=sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_sessions s LEFT JOIN session_profiles p ON p.session_id=s.id WHERE s.id=? AND COALESCE(p.profile,'default')=?").bind(id).bind(&ctx.data.profile).fetch_one(&mut *tx).await?;
        if count == 0 {
            return Err(AppError::new(
                "conversation_not_found",
                "Conversation was not found.",
            ));
        }
        id.to_string()
    } else {
        create_conversation(&mut tx, &ctx.data.profile, title.unwrap_or_default(), &now).await?
    };
    match name {
        "rename_conversation" => {
            query("UPDATE agent_sessions SET title=?,updated_at=? WHERE id=?")
                .bind(title.unwrap_or_default())
                .bind(&now)
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        }
        "archive_conversation" => {
            query("INSERT INTO completed_sessions (session_id,completed_at) VALUES (?,?) ON CONFLICT(session_id) DO NOTHING").bind(&id).bind(&now).execute(&mut *tx).await?;
        }
        "restore_conversation" => {
            query("DELETE FROM completed_sessions WHERE session_id=?")
                .bind(&id)
                .execute(&mut *tx)
                .await?;
        }
        "delete_conversation" => {
            crate::agent_sessions::AgentRepository::delete_session_in_transaction(
                &mut tx,
                &id,
                Some((
                    ctx.data.profile.as_str(),
                    binding.session_id.as_str(),
                    binding.run_id.as_str(),
                    &binding.authority,
                )),
            )
            .await?;
        }
        _ => {}
    }
    tx.commit().await?;
    if let Some(app) = ctx.app {
        use tauri::Emitter;
        let _=app.emit("clovy-data-changed",json!({"profile":ctx.data.profile,"kind":"conversations","id":id,"deleted":name=="delete_conversation"}));
    }
    Ok(json!({"id":id,"action":name}))
}
async fn ensure(ctx: &ToolCtx<'_>, id: &str) -> Result<(), AppError> {
    let exists:i64=sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_sessions s LEFT JOIN session_profiles p ON p.session_id=s.id WHERE s.id=? AND COALESCE(p.profile,'default')=?").bind(id).bind(&ctx.data.profile).fetch_one(&ctx.data.notes).await?;
    if exists == 0 {
        Err(AppError::new(
            "conversation_not_found",
            "Conversation was not found.",
        ))
    } else {
        Ok(())
    }
}

async fn create_conversation(
    tx: &mut sqlx::transaction::Transaction<'_, sqlx_sqlite::Sqlite>,
    profile: &str,
    title: &str,
    now: &str,
) -> Result<String, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let model = crate::providers::selected_chat_engine_model();
    query("INSERT INTO agent_sessions (id,title,status,model,safety_mode,source,created_at,updated_at) VALUES (?,?,'idle',?,'sandboxed','user',?,?)").bind(&id).bind(title).bind(model).bind(now).bind(now).execute(&mut **tx).await?;
    query("INSERT INTO session_profiles (session_id,profile,assigned_at) VALUES (?,?,?)")
        .bind(&id)
        .bind(profile)
        .bind(now)
        .execute(&mut **tx)
        .await?;
    Ok(id)
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crate::llm::{cli::CliKind, registry::ProviderRef};
    #[tokio::test]
    async fn selection41_conversation_creation_uses_selected_chat_including_unavailable_choice() {
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        for chosen in [CliKind::Agy, CliKind::Codex] {
            let mut settings = crate::providers::default_settings_for_tests();
            settings.llm_usage.chat = ProviderRef::Cli { id: chosen };
            settings.llm_cli_default_applied = true;
            crate::providers::replace_current_settings_for_tests(settings);
            let pool = sqlx_sqlite::SqlitePoolOptions::new()
                .connect("sqlite::memory:")
                .await
                .unwrap();
            query("CREATE TABLE agent_sessions(id TEXT PRIMARY KEY,title TEXT,status TEXT,model TEXT,safety_mode TEXT,source TEXT,created_at TEXT,updated_at TEXT)").execute(&pool).await.unwrap();
            query("CREATE TABLE session_profiles(session_id TEXT,profile TEXT,assigned_at TEXT)")
                .execute(&pool)
                .await
                .unwrap();
            let mut tx = pool.begin().await.unwrap();
            let id = create_conversation(&mut tx, "synthetic", "New conversation", "now")
                .await
                .unwrap();
            tx.commit().await.unwrap();
            let model: String =
                sqlx::query_scalar::query_scalar("SELECT model FROM agent_sessions WHERE id=?")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(model, crate::chat_engine::engine_model_id(chosen));
        }
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
    }
}
