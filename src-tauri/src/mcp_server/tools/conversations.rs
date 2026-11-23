//! Partition-scoped conversation reads. Model-supplied IDs never select a partition.
use super::{ToolCtx, ToolGroup};
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::{query::query, row::Row};

pub struct ConversationTools;
const NAMES: &[&str] = &[
    "list_conversations",
    "get_conversation",
    "get_conversation_run",
    "list_conversation_messages",
    "search_conversation_messages",
    "list_conversation_artifacts",
    "read_conversation_artifact",
];

pub(crate) fn is_conversation_read_tool(name: &str) -> bool {
    NAMES.contains(&name)
}

impl ToolGroup for ConversationTools {
    fn owns(&self, name: &str) -> bool {
        NAMES.contains(&name)
    }
    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        NAMES.iter().map(|name| {
            let properties = match *name {
                "list_conversations" => json!({"limit":{"type":"integer","minimum":1,"maximum":100},"offset":{"type":"integer","minimum":0}}),
                "get_conversation" => json!({"conversationId":{"type":"string"}}),
                "get_conversation_run" => json!({"conversationId":{"type":"string"},"runId":{"type":"string"}}),
                "list_conversation_messages" => json!({"conversationId":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":50},"cursor":{"type":["string","null"]},"textOffset":{"type":"integer","minimum":0}}),
                "search_conversation_messages" => json!({"conversationId":{"type":"string"},"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":50},"cursor":{"type":["string","null"]},"textOffset":{"type":"integer","minimum":0}}),
                "list_conversation_artifacts" => json!({"conversationId":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":100},"offset":{"type":"integer","minimum":0}}),
                _ => json!({"conversationId":{"type":"string"},"artifactId":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":4,"maximum":40000}}),
            };
            let required = match *name { "list_conversations"=>vec![], "get_conversation_run"=>vec!["conversationId","runId"], "search_conversation_messages"=>vec!["conversationId","query"], "read_conversation_artifact"=>vec!["conversationId","artifactId"], _=>vec!["conversationId"] };
            json!({"name":name,"description":match *name { "list_conversations"=>"List conversations in the bound data partition.","get_conversation"=>"Read a conversation's metadata.","get_conversation_run"=>"Read a run belonging to a conversation.","list_conversation_messages"=>"Page user and assistant messages, ordered by run and sequence.","search_conversation_messages"=>"Search user and assistant message text in a conversation.","list_conversation_artifacts"=>"List artifacts belonging to a conversation, without exposing paths.",_=>"Read a conversation artifact as bounded UTF-8 text pages; binary files are rejected."},"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":true,"idempotentHint":true,"openWorldHint":false}})
        }).collect()
    }
    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let Some(binding) = ctx.binding else {
                return vec![];
            };
            let mut result = vec![];
            for descriptor in self.descriptors(ctx) {
                if let Some(name) = descriptor["name"].as_str() {
                    if super::routine_tool_allowed(&ctx.data.notes, binding, name)
                        .await
                        .unwrap_or(false)
                    {
                        result.push(descriptor);
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
        Box::pin(async move {
            let binding = ctx
                .binding
                .ok_or_else(|| AppError::new("turn_required", "A bound turn is required."))?;
            if !super::routine_tool_allowed(&ctx.data.notes, binding, name).await? {
                return Err(AppError::new(
                    "routine_tool_not_enabled",
                    "This tool is not enabled for the routine.",
                ));
            }
            dispatch(ctx, name, args).await
        })
    }
}
fn required<'a>(v: &'a Value, key: &str) -> Result<&'a str, AppError> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            AppError::new(
                "agent_tool_arguments_invalid",
                format!("{key} is required."),
            )
        })
}
async fn ensure_session(ctx: &ToolCtx<'_>, id: &str) -> Result<(), AppError> {
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
async fn dispatch(ctx: &ToolCtx<'_>, name: &str, args: &Value) -> Result<Value, AppError> {
    let object = args.as_object().ok_or_else(|| {
        AppError::new(
            "agent_tool_arguments_invalid",
            "Arguments must be an object.",
        )
    })?;
    let allowed: &[&str] = match name {
        "list_conversations" => &["limit", "offset"],
        "get_conversation" => &["conversationId"],
        "get_conversation_run" => &["conversationId", "runId"],
        "list_conversation_messages" => &["conversationId", "limit", "cursor", "textOffset"],
        "search_conversation_messages" => {
            &["conversationId", "query", "limit", "cursor", "textOffset"]
        }
        "list_conversation_artifacts" => &["conversationId", "limit", "offset"],
        "read_conversation_artifact" => &["conversationId", "artifactId", "offset", "limit"],
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
    for key in ["conversationId", "runId", "artifactId", "query"] {
        if let Some(value) = object.get(key) {
            let max = if key == "query" { 4_000 } else { 512 };
            if !value
                .as_str()
                .is_some_and(|s| !s.trim().is_empty() && s.chars().count() <= max)
            {
                return Err(AppError::new(
                    "agent_tool_arguments_invalid",
                    format!("{key} must be a non-empty string."),
                ));
            }
        }
    }
    if let Some(value) = object.get("cursor") {
        if !value.is_null() && !value.is_string() {
            return Err(AppError::new(
                "agent_tool_arguments_invalid",
                "cursor must be a string or null.",
            ));
        }
    }
    for key in ["offset", "textOffset"] {
        if let Some(value) = object.get(key) {
            if !value.as_u64().is_some_and(|n| n <= i64::MAX as u64) {
                return Err(AppError::new(
                    "agent_tool_arguments_invalid",
                    format!("{key} must be a non-negative integer."),
                ));
            }
        }
    }
    if let Some(value) = object.get("limit") {
        let max = if name.contains("messages") {
            50
        } else if name == "read_conversation_artifact" {
            40_000
        } else {
            100
        };
        let min = if name == "read_conversation_artifact" {
            4
        } else {
            1
        };
        if !value.as_u64().is_some_and(|n| (min..=max).contains(&n)) {
            return Err(AppError::new(
                "agent_tool_arguments_invalid",
                "limit is out of range.",
            ));
        }
    }
    let pool = &ctx.data.notes;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as i64;
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(i64::MAX as u64) as i64;
    if name == "list_conversations" {
        let rows=query("SELECT s.id,s.title,s.status,s.created_at,s.updated_at,c.completed_at FROM agent_sessions s LEFT JOIN session_profiles p ON p.session_id=s.id LEFT JOIN completed_sessions c ON c.session_id=s.id WHERE COALESCE(p.profile,'default')=? ORDER BY s.updated_at DESC,s.id ASC LIMIT ? OFFSET ?").bind(&ctx.data.profile).bind(limit+1).bind(offset).fetch_all(pool).await?;
        let more = rows.len() > limit as usize;
        let items:Vec<Value>=rows.into_iter().take(limit as usize).map(|r|json!({"id":r.get::<String,_>("id"),"title":r.get::<String,_>("title"),"status":r.get::<String,_>("status"),"createdAt":r.get::<String,_>("created_at"),"updatedAt":r.get::<String,_>("updated_at"),"completedAt":r.get::<Option<String>,_>("completed_at")})).collect();
        return Ok(
            json!({"items":items,"nextOffset":if more {Some(offset.saturating_add(limit))}else{None}}),
        );
    }
    let id = required(args, "conversationId")?;
    ensure_session(ctx, id).await?;
    match name {
        "get_conversation" => {
            let row=query("SELECT s.id,s.title,s.status,s.created_at,s.updated_at,c.completed_at FROM agent_sessions s LEFT JOIN session_profiles p ON p.session_id=s.id LEFT JOIN completed_sessions c ON c.session_id=s.id WHERE s.id=? AND COALESCE(p.profile,'default')=?").bind(id).bind(&ctx.data.profile).fetch_optional(pool).await?.ok_or_else(||AppError::new("conversation_not_found","Conversation was not found."))?;
            Ok(
                json!({"id":row.get::<String,_>("id"),"title":row.get::<String,_>("title"),"status":row.get::<String,_>("status"),"createdAt":row.get::<String,_>("created_at"),"updatedAt":row.get::<String,_>("updated_at"),"completedAt":row.get::<Option<String>,_>("completed_at")}),
            )
        }
        "get_conversation_run" => {
            let run = required(args, "runId")?;
            let row=query("SELECT r.id,r.status,r.model,r.started_at,r.updated_at,r.completed_at,r.error_code FROM agent_runs r JOIN agent_sessions s ON s.id=r.session_id LEFT JOIN session_profiles p ON p.session_id=s.id WHERE r.id=? AND r.session_id=? AND COALESCE(p.profile,'default')=?").bind(run).bind(id).bind(&ctx.data.profile).fetch_optional(pool).await?.ok_or_else(||AppError::new("run_not_found","Run was not found."))?;
            Ok(
                json!({"id":row.get::<String,_>("id"),"conversationId":id,"status":row.get::<String,_>("status"),"model":row.get::<String,_>("model"),"startedAt":row.get::<String,_>("started_at"),"updatedAt":row.get::<String,_>("updated_at"),"completedAt":row.get::<Option<String>,_>("completed_at"),"errorCode":row.get::<Option<String>,_>("error_code")}),
            )
        }
        "list_conversation_messages" | "search_conversation_messages" => {
            messages(ctx, id, name, args).await
        }
        "list_conversation_artifacts" => {
            let rows=query("SELECT a.id,a.run_id,a.display_name,a.mime_type,a.size_bytes,a.available,a.created_at FROM agent_artifacts a JOIN agent_sessions s ON s.id=a.session_id LEFT JOIN session_profiles p ON p.session_id=s.id WHERE a.session_id=? AND COALESCE(p.profile,'default')=? ORDER BY a.created_at,a.id LIMIT ? OFFSET ?").bind(id).bind(&ctx.data.profile).bind(limit+1).bind(offset).fetch_all(pool).await?;
            let more = rows.len() > limit as usize;
            let items:Vec<Value>=rows.into_iter().take(limit as usize).map(|r|json!({"id":r.get::<String,_>("id"),"runId":r.get::<Option<String>,_>("run_id"),"name":r.get::<Option<String>,_>("display_name"),"mimeType":r.get::<Option<String>,_>("mime_type"),"sizeBytes":r.get::<Option<i64>,_>("size_bytes"),"available":r.get::<i64,_>("available")!=0,"createdAt":r.get::<String,_>("created_at")})).collect();
            Ok(
                json!({"items":items,"nextOffset":if more {Some(offset.saturating_add(limit))} else {None}}),
            )
        }
        "read_conversation_artifact" => artifact(ctx, id, args).await,
        _ => Err(AppError::new(
            "mcp_tool_unknown",
            "Unknown conversation tool.",
        )),
    }
}

async fn messages(
    ctx: &ToolCtx<'_>,
    conversation_id: &str,
    name: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let cursor = args.get("cursor").and_then(Value::as_str);
    let key: (String, String, i64, String) = match cursor {
        Some(raw) if raw.len() <= 2048 => serde_json::from_str(raw)
            .map_err(|_| AppError::new("agent_tool_arguments_invalid", "Invalid cursor."))?,
        Some(_) => {
            return Err(AppError::new(
                "agent_tool_arguments_invalid",
                "Invalid cursor.",
            ))
        }
        None => (String::new(), String::new(), i64::MIN, String::new()),
    };
    let text_offset = super::notes::character_offset(args, "textOffset")?;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 50) as usize;
    let pattern = if name == "search_conversation_messages" {
        let value = required(args, "query")?;
        Some(format!(
            "%{}%",
            value
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        ))
    } else {
        None
    };
    let rows=query("SELECT i.id,i.run_id,i.sequence,i.kind,i.payload_json,i.created_at,COALESCE(r.started_at,i.created_at) AS run_start,COALESCE(i.run_id,'') AS run_key FROM agent_items i JOIN agent_sessions s ON s.id=i.session_id LEFT JOIN session_profiles p ON p.session_id=s.id LEFT JOIN agent_runs r ON r.id=i.run_id AND r.session_id=i.session_id WHERE i.session_id=? AND COALESCE(p.profile,'default')=? AND i.kind IN ('user_message','assistant_message') AND (? IS NULL OR json_extract(i.payload_json,'$.content') LIKE ? ESCAPE '\\') AND (COALESCE(r.started_at,i.created_at),COALESCE(i.run_id,''),i.sequence,i.id) > (?,?,?,?) ORDER BY run_start,run_key,i.sequence,i.id LIMIT ?")
        .bind(conversation_id).bind(&ctx.data.profile).bind(pattern.as_deref()).bind(pattern.as_deref()).bind(&key.0).bind(&key.1).bind(key.2).bind(&key.3).bind((limit+1) as i64).fetch_all(&ctx.data.notes).await?;
    let mut items = Vec::new();
    let mut next_cursor = None;
    let mut next_text_offset = None;
    let mut previous = cursor.map(str::to_string);
    for (index, row) in rows.iter().enumerate().take(limit) {
        let payload: Value = serde_json::from_str(&row.get::<String, _>("payload_json"))
            .map_err(|_| AppError::new("conversation_invalid", "Stored message is invalid."))?;
        let text = payload.get("content").and_then(Value::as_str).unwrap_or("");
        let start = if index == 0 { text_offset } else { 0 };
        if start > text.chars().count() {
            return Err(AppError::new(
                "agent_tool_arguments_invalid",
                "textOffset is past the end of the message.",
            ));
        }
        let (page, total, next) = super::notes::character_page(text, start, 40_000);
        items.push(json!({"id":row.get::<String,_>("id"),"runId":row.get::<Option<String>,_>("run_id"),"sequence":row.get::<i64,_>("sequence"),"role":if row.get::<String,_>("kind")=="user_message" {"user"}else{"assistant"},"text":page,"textOffset":start,"textTotalChars":total,"createdAt":row.get::<String,_>("created_at")}));
        if let Some(next) = next {
            next_cursor = previous;
            next_text_offset = Some(next);
            break;
        }
        let new_key = (
            row.get::<String, _>("run_start"),
            row.get::<String, _>("run_key"),
            row.get::<i64, _>("sequence"),
            row.get::<String, _>("id"),
        );
        previous = Some(
            serde_json::to_string(&new_key)
                .map_err(|e| AppError::new("agent_tool_response_invalid", e.to_string()))?,
        );
        next_cursor = if index + 1 < rows.len() {
            previous.clone()
        } else {
            None
        };
    }
    Ok(json!({"items":items,"nextCursor":next_cursor,"nextTextOffset":next_text_offset}))
}

async fn artifact(
    ctx: &ToolCtx<'_>,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let artifact_id = required(args, "artifactId")?;
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let row=query("SELECT a.path,a.mime_type FROM agent_artifacts a JOIN agent_sessions s ON s.id=a.session_id LEFT JOIN session_profiles p ON p.session_id=s.id WHERE a.id=? AND a.session_id=? AND a.available=1 AND COALESCE(p.profile,'default')=?").bind(artifact_id).bind(conversation_id).bind(&ctx.data.profile).fetch_optional(&ctx.data.notes).await?;
    let row = row.ok_or_else(|| {
        AppError::new(
            "agent_artifact_unavailable",
            "Artifact was not found or is unavailable.",
        )
    })?;
    let path: String = row.get("path");
    let mime: Option<String> = row.get("mime_type");
    if mime.as_deref().is_some_and(binary_mime) {
        return Err(AppError::new(
            "agent_artifact_binary",
            "Artifact is not UTF-8 text.",
        ));
    }
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(40_000) as usize;
    let mut result = artifact_page(&path, offset, limit).await?;
    let still_authorized: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_artifacts a JOIN agent_sessions s ON s.id=a.session_id LEFT JOIN session_profiles p ON p.session_id=s.id WHERE a.id=? AND a.session_id=? AND a.path=? AND a.available=1 AND COALESCE(p.profile,'default')=?")
        .bind(artifact_id).bind(conversation_id).bind(&path).bind(&ctx.data.profile).fetch_one(&ctx.data.notes).await?;
    ctx.revalidate().await?;
    if still_authorized == 0 {
        return Err(AppError::new(
            "agent_artifact_unavailable",
            "Artifact is unavailable.",
        ));
    }
    result["artifactId"] = json!(artifact_id);
    Ok(result)
}

fn binary_mime(mime: &str) -> bool {
    let mime = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    mime.starts_with("image/")
        || mime.starts_with("audio/")
        || mime.starts_with("video/")
        || matches!(
            mime.as_str(),
            "application/pdf"
                | "application/zip"
                | "application/gzip"
                | "application/x-7z-compressed"
        )
}

/// Bounded text IO after the artifact's conversation and partition were checked.
async fn artifact_page(path: &str, offset: u64, limit: usize) -> Result<Value, AppError> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    if !(4..=40_000).contains(&limit) {
        return Err(AppError::new(
            "agent_tool_arguments_invalid",
            "limit must be between 4 and 40000 bytes.",
        ));
    }
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|_| AppError::new("agent_artifact_unavailable", "Artifact is unavailable."))?;
    let initial_metadata = file
        .metadata()
        .await
        .map_err(|_| AppError::new("agent_artifact_unavailable", "Artifact is unavailable."))?;
    let size = initial_metadata.len();
    if offset > size {
        return Err(AppError::new(
            "agent_tool_arguments_invalid",
            "Offset is past the end of the artifact.",
        ));
    }
    file.seek(std::io::SeekFrom::Start(offset))
        .await
        .map_err(|_| AppError::new("agent_artifact_unavailable", "Artifact is unavailable."))?;
    let mut bytes = Vec::with_capacity(limit + 4);
    (&mut file)
        .take((limit + 4) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| AppError::new("agent_artifact_unavailable", "Artifact is unavailable."))?;
    let final_metadata = file
        .metadata()
        .await
        .map_err(|_| AppError::new("agent_artifact_unavailable", "Artifact is unavailable."))?;
    if final_metadata.len() != size
        || final_metadata.modified().ok() != initial_metadata.modified().ok()
        || (bytes.is_empty() && offset < size)
    {
        return Err(AppError::new(
            "agent_artifact_changed",
            "Artifact changed while reading. Start reading it again.",
        ));
    }
    if bytes.contains(&0) {
        return Err(AppError::new(
            "agent_artifact_binary",
            "Artifact is not UTF-8 text.",
        ));
    }
    let page_end = bytes.len().min(limit);
    let end = match std::str::from_utf8(&bytes[..page_end]) {
        Ok(_) => page_end,
        Err(error)
            if error.error_len().is_none()
                && error.valid_up_to() > 0
                && offset + (page_end as u64) < size =>
        {
            error.valid_up_to()
        }
        Err(_) => {
            return Err(AppError::new(
                "agent_artifact_binary",
                "Artifact is not UTF-8 text or the offset is not a character boundary.",
            ))
        }
    };
    let text = std::str::from_utf8(&bytes[..end])
        .map_err(|_| AppError::new("agent_artifact_binary", "Artifact is not UTF-8 text."))?;
    let next = offset + end as u64;
    Ok(
        json!({"text":text,"offset":offset,"nextOffset":if next<size {Some(next)}else{None},"totalBytes":size}),
    )
}
