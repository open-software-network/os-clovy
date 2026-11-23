//! Projects are plural folder associations within the host-bound partition.
use super::{ToolCtx, ToolGroup};
use crate::approval_broker::{external::ExternalCall, ApprovalKind, ApprovalRequest};
use crate::db::repositories::McpMutation;
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::{query::query, row::Row, transaction::Transaction};
use sqlx_sqlite::Sqlite;

pub struct ProjectTools;
const NAMES: &[&str] = &[
    "list_projects",
    "get_project",
    "create_project",
    "update_project",
    "delete_project",
    "assign_note_project",
    "remove_note_project",
    "assign_conversation_project",
    "remove_conversation_project",
];
impl ToolGroup for ProjectTools {
    fn owns(&self, name: &str) -> bool {
        NAMES.contains(&name)
    }
    fn descriptors(&self, _: &ToolCtx<'_>) -> Vec<Value> {
        NAMES.iter().map(|name| {
            let mut props = json!({"projectId":{"type":"string","minLength":1}});
            let required = match *name {
                "list_projects" => { props=json!({"limit":{"type":"integer","minimum":1,"maximum":100},"offset":{"type":"integer","minimum":0}}); vec![] },
                "create_project" | "update_project" => {
                    props["name"]=json!({"type":"string","minLength":1,"maxLength":200});
                    props["description"]=json!({"type":["string","null"],"maxLength":4000});
                    props["instructions"]=json!({"type":["string","null"],"maxLength":4000});
                    props["memoryDisabled"]=json!({"type":"boolean"});
                    if *name=="create_project" { props.as_object_mut().unwrap().remove("projectId"); vec!["name"] } else {vec!["projectId"]}
                },
                "delete_project" => {props["deleteNotes"]=json!({"type":"boolean"}); vec!["projectId","deleteNotes"]},
                "assign_note_project" | "remove_note_project" => {props["noteId"]=json!({"type":"string","minLength":1}); vec!["projectId","noteId"]},
                "assign_conversation_project" | "remove_conversation_project" => {props["conversationId"]=json!({"type":"string","minLength":1}); vec!["projectId","conversationId"]},
                _ => vec!["projectId"],
            };
            json!({"name":name,"description":match *name {
                "list_projects"=>"Page projects in the bound data partition.",
                "get_project"=>"Read a project and its association counts.",
                "create_project"=>"Create a project without granting access outside the bound partition.",
                "update_project"=>"Atomically update project name, description, instructions and memory setting.",
                "delete_project"=>"Delete a project after previewing affected notes and counts. deleteNotes is explicit; false preserves notes. Removes project memories and associations; conversations are preserved.",
                "assign_note_project"|"assign_conversation_project"=>"Add a project association, preserving associations with other projects.",
                _=>"Remove only this project association; preserve the object and its other projects."
            },"inputSchema":{"type":"object","properties":props,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":matches!(*name,"list_projects"|"get_project"),"openWorldHint":false,"destructiveHint":*name=="delete_project"}})
        }).collect()
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move { dispatch(ctx, None, name, args).await })
    }
}
fn invalid(message: &str) -> AppError {
    AppError::new("agent_tool_arguments_invalid", message)
}
fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.len() <= 512)
        .ok_or_else(|| invalid(&format!("{key} is required.")))
}
fn validate(name: &str, args: &Value) -> Result<(), AppError> {
    let obj = args
        .as_object()
        .ok_or_else(|| invalid("Arguments must be an object."))?;
    let allowed: &[&str] = match name {
        "list_projects" => &["limit", "offset"],
        "get_project" => &["projectId"],
        "create_project" => &["name", "description", "instructions", "memoryDisabled"],
        "update_project" => &[
            "projectId",
            "name",
            "description",
            "instructions",
            "memoryDisabled",
        ],
        "delete_project" => &["projectId", "deleteNotes"],
        "assign_note_project" | "remove_note_project" => &["projectId", "noteId"],
        "assign_conversation_project" | "remove_conversation_project" => {
            &["projectId", "conversationId"]
        }
        _ => return Err(AppError::new("mcp_tool_unknown", "Unknown project tool.")),
    };
    if obj.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid("Unexpected argument."));
    }
    for (key, max) in [("name", 200), ("description", 4000), ("instructions", 4000)] {
        if let Some(value) = obj.get(key) {
            if value.is_null() && key != "name" {
                continue;
            }
            let s = value
                .as_str()
                .ok_or_else(|| invalid("Project text fields must be strings."))?;
            if s.chars().count() > max || (key == "name" && s.trim().is_empty()) {
                return Err(invalid("Project text field is empty or too long."));
            }
        }
    }
    if name == "create_project" && !obj.contains_key("name") {
        return Err(invalid("name is required."));
    }
    if name == "update_project" && obj.len() == 1 {
        return Err(invalid("At least one project field is required."));
    }
    for key in ["deleteNotes", "memoryDisabled"] {
        if let Some(v) = obj.get(key) {
            if !v.is_boolean() {
                return Err(invalid("Project flags must be boolean."));
            }
        }
    }
    if name == "delete_project" && !obj.contains_key("deleteNotes") {
        return Err(invalid("deleteNotes must be explicit."));
    }
    if let Some(v) = obj.get("limit") {
        if !v.as_u64().is_some_and(|v| (1..=100).contains(&v)) {
            return Err(invalid("limit must be between 1 and 100."));
        }
    }
    if let Some(v) = obj.get("offset") {
        if !v.as_u64().is_some_and(|v| v <= i64::MAX as u64) {
            return Err(invalid("offset must be a non-negative integer."));
        }
    }
    Ok(())
}
fn project(row: &sqlx_sqlite::SqliteRow) -> Value {
    json!({"id":row.get::<String,_>("id"),"name":row.get::<String,_>("name"),"description":row.get::<Option<String>,_>("description"),"instructions":row.get::<Option<String>,_>("instructions"),"memoryDisabled":row.get::<i64,_>("memory_disabled")!=0,"localPath":row.get::<Option<String>,_>("local_path"),"createdAt":row.get::<String,_>("created_at"),"updatedAt":row.get::<String,_>("updated_at")})
}
async fn ensure_project(
    tx: &mut Transaction<'_, Sqlite>,
    profile: &str,
    id: &str,
) -> Result<Value, AppError> {
    let row=query("SELECT id,name,description,instructions,memory_disabled,local_path,created_at,updated_at FROM folders WHERE id=? AND profile=? AND deleted_at IS NULL").bind(id).bind(profile).fetch_optional(&mut **tx).await?.ok_or_else(||AppError::new("folder_not_found","Project was not found."))?;
    Ok(project(&row))
}
async fn note_snapshot(
    tx: &mut Transaction<'_, Sqlite>,
    profile: &str,
    id: &str,
) -> Result<Vec<(String, String)>, AppError> {
    let rows=query("SELECT n.id,n.title FROM notes n JOIN note_folders nf ON nf.note_id=n.id WHERE nf.folder_id=? AND n.profile=? ORDER BY n.id").bind(id).bind(profile).fetch_all(&mut **tx).await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get("id"), r.get("title")))
        .collect())
}
async fn ensure_object(
    tx: &mut Transaction<'_, Sqlite>,
    profile: &str,
    name: &str,
    args: &Value,
) -> Result<(), AppError> {
    let count: i64 = if name.contains("note_project") {
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM notes WHERE id=? AND profile=?")
            .bind(required(args, "noteId")?)
            .bind(profile)
            .fetch_one(&mut **tx)
            .await?
    } else {
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_sessions s LEFT JOIN session_profiles p ON p.session_id=s.id WHERE s.id=? AND COALESCE(p.profile,'default')=?").bind(required(args,"conversationId")?).bind(profile).fetch_one(&mut **tx).await?
    };
    if count == 0 {
        return Err(AppError::new(
            "project_object_not_found",
            "The object was not found in the bound data partition.",
        ));
    }
    Ok(())
}
pub(super) async fn dispatch(
    ctx: &ToolCtx<'_>,
    external: Option<&ExternalCall<'_>>,
    name: &str,
    args: &Value,
) -> Result<Value, AppError> {
    ctx.revalidate_local(external).await?;
    validate(name, args)?;
    ensure_routine_allowed(ctx, name).await?;
    let profile = &ctx.data.profile;
    if name == "list_projects" {
        let limit = args.get("limit").and_then(Value::as_i64).unwrap_or(20);
        let offset = args.get("offset").and_then(Value::as_i64).unwrap_or(0);
        let rows=query("SELECT id,name,description,instructions,memory_disabled,local_path,created_at,updated_at FROM folders WHERE profile=? AND deleted_at IS NULL ORDER BY created_at,id LIMIT ? OFFSET ?").bind(profile).bind(limit+1).bind(offset).fetch_all(&ctx.data.notes).await?;
        return Ok(
            json!({"items":rows.iter().take(limit as usize).map(|row| project_projection(project(row), external.is_some())).collect::<Vec<_>>(),"nextOffset":if rows.len()>limit as usize {Some(offset.saturating_add(limit))}else{None}}),
        );
    }
    let id = if name == "create_project" {
        uuid::Uuid::new_v4().to_string()
    } else {
        required(args, "projectId")?.to_string()
    };
    let mut tx = ctx.data.notes.begin().await?;
    let old = if name != "create_project" {
        Some(ensure_project(&mut tx, profile, &id).await?)
    } else {
        None
    };
    if name == "get_project" {
        let mut result = old.unwrap();
        if external.is_some() {
            let note_count: i64 = sqlx::query_scalar::query_scalar(
                "SELECT COUNT(*) FROM note_folders nf JOIN notes n ON n.id = nf.note_id WHERE nf.folder_id = ? AND n.profile = ?",
            ).bind(&id).bind(profile).fetch_one(&mut *tx).await?;
            let memory_count: i64 = sqlx::query_scalar::query_scalar(
                "SELECT COUNT(*) FROM memories WHERE folder_id = ? AND profile = ?",
            )
            .bind(&id)
            .bind(profile)
            .fetch_one(&mut *tx)
            .await?;
            result["noteCount"] = json!(note_count);
            result["memoryCount"] = json!(memory_count);
        } else {
            for (field, table) in [
                ("noteCount", "note_folders"),
                ("conversationCount", "session_folders"),
                ("memoryCount", "memories"),
            ] {
                // Table names are host constants, never input.
                let count: i64 = sqlx::query_scalar::query_scalar(&format!(
                    "SELECT COUNT(*) FROM {table} WHERE folder_id=?"
                ))
                .bind(&id)
                .fetch_one(&mut *tx)
                .await?;
                result[field] = json!(count);
            }
        }
        tx.commit().await?;
        return Ok(project_projection(result, external.is_some()));
    }
    if name.contains("_project") && (name.starts_with("assign_") || name.starts_with("remove_")) {
        ensure_object(&mut tx, profile, name, args).await?;
    }
    let delete_snapshot = if name == "delete_project" {
        Some(note_snapshot(&mut tx, profile, &id).await?)
    } else {
        None
    };
    let target = if external.is_some() {
        external_target(&mut tx, name, args, &id, profile, old.as_ref()).await?
    } else {
        None
    };
    tx.commit().await?;
    let preview=delete_snapshot.as_ref().map(|notes|json!({"project":old,"deleteNotes":args["deleteNotes"],"noteCount":notes.len(),"notes":notes.iter().take(100).map(|(id,title)|json!({"id":id,"title":title})).collect::<Vec<_>>(),"previewTruncated":notes.len()>100,"conversationsPreserved":true,"projectMemoriesDeleted":true}).to_string());
    // Resolve storage before approval and before any database effect.
    let recording_data = if name == "delete_project" && args["deleteNotes"] == true {
        Some(ctx.recording_data(external)?)
    } else {
        None
    };
    let authority = ctx
        .approve_local(
            external,
            ApprovalRequest {
                tool: name.into(),
                call_id: Some(uuid::Uuid::new_v4().to_string()),
                kind: ApprovalKind::Approval {
                    title: match name {
                        "create_project" => "Create a project",
                        "update_project" => "Update a project",
                        "delete_project" => "Delete a project",
                        _ => "Change a project association",
                    }
                    .into(),
                    description: if name == "delete_project" {
                        "Permanently remove this project, its listed note associations and its listed memories. Delete the listed notes and recordings only when deleteNotes is true."
                    } else {
                        "Change projects in the selected data partition."
                    }.into(),
                    arguments: args.clone(),
                    preview,
                },
            },
            target.as_ref(),
        )
        .await?;
    ctx.revalidate_local(external).await?;
    ensure_routine_allowed(ctx, name).await?;
    let cleanup_paths = recording_data
        .map(crate::app_paths::AppPaths::from_data_dir)
        .transpose()
        .map_err(|e| AppError::new("storage_unavailable", e.to_string()))?;
    let result = apply(
        ProjectMutation {
            pool: &ctx.data.notes,
            profile,
            authority,
        },
        name,
        args,
        &id,
        delete_snapshot.as_deref(),
    )
    .await?;
    let mut cleanup_failures = 0;
    if let Some(paths) = cleanup_paths {
        for file in &result.1 {
            if let Err(error) = paths.remove_recording_file(file) {
                cleanup_failures += 1;
                tracing::warn!(%error,"Failed to remove deleted project note audio");
            }
        }
    }
    if let Some(app) = ctx.app {
        use tauri::Emitter;
        for kind in ["projects", "notes", "conversations", "memories"] {
            let _ = app.emit(
                "clovy-data-changed",
                json!({
                    "profile":profile,"kind":kind,
                    "id":if kind=="projects" {Some(&id)} else {None},
                    "deleted":name=="delete_project" && kind=="projects"
                }),
            );
        }
        if let Some(deleted_notes) = result.0.get("deletedNoteIds").and_then(Value::as_array) {
            for note_id in deleted_notes {
                let _ = app.emit(
                    "clovy-data-changed",
                    json!({
                        "profile":profile,"kind":"notes","id":note_id,"deleted":true
                    }),
                );
            }
        }
    }
    let mut value = result.0;
    if name == "delete_project" {
        value["audioCleanupFailures"] = json!(cleanup_failures);
    }
    Ok(project_projection(value, external.is_some()))
}

async fn ensure_routine_allowed(ctx: &ToolCtx<'_>, name: &str) -> Result<(), AppError> {
    if let Some(binding) = ctx.binding {
        if !super::routine_tool_allowed(&ctx.data.notes, binding, name).await? {
            return Err(AppError::new(
                "routine_tool_not_enabled",
                "This tool is not enabled for the routine.",
            ));
        }
    }
    Ok(())
}

fn project_projection(mut value: Value, external: bool) -> Value {
    if external {
        if let Some(object) = value.as_object_mut() {
            // A project ID is not a filesystem or conversation grant.
            object.remove("localPath");
            object.remove("conversationCount");
        }
    }
    value
}

async fn external_target(
    tx: &mut Transaction<'_, Sqlite>,
    name: &str,
    args: &Value,
    id: &str,
    profile: &str,
    project: Option<&Value>,
) -> Result<Option<Value>, AppError> {
    let Some(project) = project else {
        return Ok(None);
    };
    let mut target = json!({"project":project_projection(project.clone(), true)});
    if matches!(name, "assign_note_project" | "remove_note_project") {
        let row = query("SELECT id, title, revision FROM notes WHERE id = ? AND profile = ?")
            .bind(required(args, "noteId")?)
            .bind(profile)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| AppError::new("note_not_found", "The note was not found."))?;
        target["note"] = json!({"id":row.get::<String,_>("id"),"title":row.get::<String,_>("title"),"revision":row.get::<i64,_>("revision")});
    }
    if name == "delete_project" {
        let notes = query("SELECT n.id,n.title,n.revision,n.updated_at FROM notes n JOIN note_folders nf ON nf.note_id=n.id WHERE nf.folder_id=? AND n.profile=? ORDER BY n.id LIMIT 101").bind(id).bind(profile).fetch_all(&mut **tx).await?;
        let memories = query("SELECT id,content,updated_at FROM memories WHERE folder_id=? AND profile=? ORDER BY id LIMIT 101").bind(id).bind(profile).fetch_all(&mut **tx).await?;
        if notes.len() > 100 || memories.len() > 100 {
            return Err(AppError::new(
                "mcp_external_preview_too_large",
                "Clovy could not preview the entire project deletion for approval.",
            ));
        }
        let mut approved_notes = Vec::with_capacity(notes.len());
        for row in notes {
            let note_id: String = row.get("id");
            let preview = if args["deleteNotes"] == true {
                crate::db::repositories::mcp_note_delete_target(tx, profile, &note_id).await?
            } else {
                json!({"id":note_id,"title":row.get::<String,_>("title"),"revision":row.get::<i64,_>("revision"),"updatedAt":row.get::<String,_>("updated_at")})
            };
            approved_notes.push(preview);
        }
        target["notes"] = json!(approved_notes);
        target["memories"] = json!(memories.iter().map(|row| json!({"id":row.get::<String,_>("id"),"content":row.get::<String,_>("content"),"updatedAt":row.get::<String,_>("updated_at")})).collect::<Vec<_>>());
    }
    Ok(Some(target))
}

#[derive(Clone, Copy)]
struct ProjectMutation<'a> {
    pool: &'a sqlx_sqlite::SqlitePool,
    profile: &'a str,
    authority: McpMutation<'a>,
}

#[cfg(test)]
fn project_turn<'a>(ctx: &'a ToolCtx<'_>) -> Result<ProjectMutation<'a>, AppError> {
    let binding = ctx
        .binding
        .ok_or_else(|| AppError::new("turn_required", "A bound turn is required."))?;
    Ok(ProjectMutation {
        pool: &ctx.data.notes,
        profile: &ctx.data.profile,
        authority: McpMutation::Turn(&binding.session_id, &binding.run_id, &binding.authority),
    })
}

/// The production write boundary is also directly exercised by database tests.
async fn apply(
    mutation: ProjectMutation<'_>,
    name: &str,
    args: &Value,
    id: &str,
    snapshot: Option<&[(String, String)]>,
) -> Result<(Value, Vec<String>), AppError> {
    let profile = mutation.profile;
    let mut tx = if matches!(mutation.authority, McpMutation::External(_)) {
        mutation.pool.begin_with("BEGIN IMMEDIATE").await?
    } else {
        mutation.pool.begin().await?
    };
    mutation.authority.guard(&mut tx, profile).await?;
    let old = if name != "create_project" {
        Some(ensure_project(&mut tx, profile, id).await?)
    } else {
        None
    };
    if let Some(expected) = mutation.authority.external_target() {
        let current = external_target(&mut tx, name, args, id, profile, old.as_ref()).await?;
        if current.as_ref() != Some(expected) {
            return Err(AppError::new(
                "project_conflict",
                "The project or its approved target changed while the approval was pending.",
            ));
        }
    }
    let now = crate::db::repositories::timestamp();
    let mut files = Vec::new();
    match name {
        "create_project" => {
            query("INSERT INTO folders (id,profile,name,description,instructions,memory_disabled,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?)").bind(id).bind(profile).bind(args["name"].as_str().unwrap().trim()).bind(args.get("description").and_then(Value::as_str).map(str::trim)).bind(args.get("instructions").and_then(Value::as_str).map(str::trim)).bind(args.get("memoryDisabled").and_then(Value::as_bool).unwrap_or(false)).bind(&now).bind(&now).execute(&mut *tx).await?;
        }
        "update_project" => {
            let mut merged = old.unwrap();
            for key in ["name", "description", "instructions", "memoryDisabled"] {
                if let Some(value) = args.get(key) {
                    merged[key] = value.clone();
                }
            }
            query("UPDATE folders SET name=?,description=?,instructions=?,memory_disabled=?,updated_at=? WHERE id=? AND profile=? AND deleted_at IS NULL").bind(merged["name"].as_str().unwrap().trim()).bind(merged["description"].as_str().map(str::trim)).bind(merged["instructions"].as_str().map(str::trim)).bind(merged["memoryDisabled"].as_bool().unwrap_or(false)).bind(&now).bind(id).bind(profile).execute(&mut *tx).await?;
        }
        "delete_project" => {
            let notes = note_snapshot(&mut tx, profile, id).await?;
            if snapshot != Some(notes.as_slice()) {
                return Err(AppError::new(
                    "project_conflict",
                    "Project notes changed after the preview. Read and approve the deletion again.",
                ));
            }
            if args["deleteNotes"] == true {
                for (note, _) in &notes {
                    let active:i64=sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM recording_sessions WHERE note_id=? AND status IN ('recording','paused')").bind(note).fetch_one(&mut *tx).await?;
                    if active != 0 {
                        return Err(AppError::new(
                            "note_recording_active",
                            "Stop the recording before deleting its note.",
                        ));
                    }
                    let paths =
                        crate::db::repositories::note_artifact_paths_for_deletion(&mut tx, note)
                            .await?;
                    files.extend(crate::db::repositories::with_video_capture_metadata(paths));
                    crate::db::repositories::delete_note_records(&mut tx, note).await?;
                }
            }
            if matches!(mutation.authority, McpMutation::External(_)) {
                // Keep historical conversation memberships and any legacy
                // cross-partition links outside this external approval.
                query("DELETE FROM note_folders WHERE folder_id=? AND note_id IN (SELECT id FROM notes WHERE profile=?)")
                    .bind(id).bind(profile).execute(&mut *tx).await?;
                query("INSERT OR REPLACE INTO memory_tombstones (id,deleted_at) SELECT id,? FROM memories WHERE folder_id=? AND profile=?")
                    .bind(&now).bind(id).bind(profile).execute(&mut *tx).await?;
                query("DELETE FROM memories WHERE folder_id=? AND profile=?")
                    .bind(id)
                    .bind(profile)
                    .execute(&mut *tx)
                    .await?;
            } else {
                query("DELETE FROM note_folders WHERE folder_id=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                query("DELETE FROM session_folders WHERE folder_id=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                query("INSERT OR REPLACE INTO memory_tombstones (id,deleted_at) SELECT id,? FROM memories WHERE folder_id=?").bind(&now).bind(id).execute(&mut *tx).await?;
                query("DELETE FROM memories WHERE folder_id=?")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            query("UPDATE folders SET deleted_at=?,updated_at=? WHERE id=? AND profile=?")
                .bind(&now)
                .bind(&now)
                .bind(id)
                .bind(profile)
                .execute(&mut *tx)
                .await?;
        }
        _ => {
            ensure_object(&mut tx, profile, name, args).await?;
            let (table, column, key) = if name.contains("note_project") {
                ("note_folders", "note_id", "noteId")
            } else {
                ("session_folders", "session_id", "conversationId")
            };
            if name.starts_with("assign_") {
                query(&format!(
                    "INSERT OR IGNORE INTO {table} ({column},folder_id,assigned_at) VALUES (?,?,?)"
                ))
                .bind(required(args, key)?)
                .bind(id)
                .bind(&now)
                .execute(&mut *tx)
                .await?;
            } else {
                query(&format!(
                    "DELETE FROM {table} WHERE {column}=? AND folder_id=?"
                ))
                .bind(required(args, key)?)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            }
        }
    }
    let result = if name == "delete_project" {
        json!({"id":id,"deleted":true,"deletedNoteIds":if args["deleteNotes"]==true {snapshot.unwrap_or_default().iter().map(|(id,_)|id).collect::<Vec<_>>()}else{vec![]}})
    } else {
        ensure_project(&mut tx, profile, id).await?
    };
    mutation.authority.before_commit(profile)?;
    tx.commit().await?;
    Ok((result, files))
}

#[cfg(test)]
#[path = "projects_tests.rs"]
mod tests;
