use super::skill_identity::{canonical_skill_id, canonical_skill_ids};
use super::{
    AgentItemDto, AgentItemPayload, AgentRepository, AgentSafetyMode, MessageAttachmentPayload,
};
use crate::domain::types::AppError;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Component,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::{Duration, SystemTime},
};
use tauri::{AppHandle, Manager};

const MAX_STAGED_AGENT_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;
// Composer attachments may remain recoverable across restarts. Reap only
// abandoned UUID staging directories that have been untouched for seven days.
const STAGED_AGENT_ATTACHMENT_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
// Eight legal 50 MiB attachments fit at once, with headroom for one composer
// being abandoned while another begins staging.
const MAX_STAGED_AGENT_ATTACHMENT_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_STAGED_AGENT_ATTACHMENT_DIRECTORIES: usize = 128;
const STAGED_AGENT_ATTACHMENTS_DIR: &str = "agent-attachment-staging";
static STAGED_AGENT_ATTACHMENTS_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub title: Option<String>,
    pub model: String,
    pub safety_mode: AgentSafetyMode,
    #[serde(default = "default_data_partition")]
    pub profile: String,
}

fn default_data_partition() -> String {
    "default".to_string()
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameSessionRequest {
    pub session_id: String,
    pub title: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunRequest {
    pub session_id: String,
    pub prompt: String,
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub safety_mode: AgentSafetyMode,
    #[serde(default)]
    pub surface: Option<String>,
    pub workspace_path: String,
    #[serde(default)]
    pub enabled_skill_ids: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<String>,
    #[serde(default)]
    pub attachment_metadata: Vec<AttachmentMetadataInput>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMetadataInput {
    pub name: String,
    pub media_type: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveInterruptionRequest {
    pub run_id: String,
    pub interruption_id: String,
    pub resolution: Value,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSkillEnabledRequest {
    pub skill_id: String,
    pub enabled: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSkillRequest {
    pub skill_id: String,
    pub content: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadArtifactRequest {
    pub path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscardStagedAgentAttachmentsRequest {
    pub paths: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneStagedAgentAttachmentsRequest {
    pub protected_paths: Vec<String>,
}

async fn repository(app: &AppHandle) -> Result<AgentRepository, AppError> {
    Ok(AgentRepository::new(
        crate::commands::repositories(app).await?.pool,
    ))
}

#[tauri::command]
pub async fn list_agent_sessions(app: AppHandle) -> Result<Vec<Value>, AppError> {
    Ok(repository(&app)
        .await?
        .list_sessions()
        .await?
        .into_iter()
        .map(session_json)
        .collect())
}

#[tauri::command]
pub async fn get_agent_session(app: AppHandle, session_id: String) -> Result<Value, AppError> {
    Ok(session_json(
        repository(&app).await?.get_session(&session_id).await?,
    ))
}

#[tauri::command]
pub async fn get_latest_agent_run(
    app: AppHandle,
    session_id: String,
) -> Result<Option<Value>, AppError> {
    let repository = repository(&app).await?;
    let row = sqlx::query::query(
        "SELECT id FROM agent_runs WHERE session_id = ? ORDER BY started_at DESC LIMIT 1",
    )
    .bind(session_id)
    .fetch_optional(&repository.pool)
    .await?;
    use sqlx::row::Row;
    match row {
        Some(row) => Ok(Some(run_json(
            repository.get_run(&row.get::<String, _>("id")).await?,
        ))),
        None => Ok(None),
    }
}

/// Stages a DOM-dropped attachment until the next agent run copies it into the
/// session workspace. WKWebView exposes a dropped file's bytes, not its local
/// filesystem path, so this command deliberately accepts Tauri's raw payload.
#[tauri::command]
pub fn stage_agent_attachment_bytes(
    app: AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<String, AppError> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(AppError::new(
            "agent_attachment_staging_invalid",
            "Expected the dropped file's contents as a binary payload.",
        ));
    };
    validate_staged_attachment_size(bytes.len())?;
    let raw_name = request
        .headers()
        .get("x-file-name")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| urlencoding::decode(value).ok())
        .map(|value| value.into_owned())
        .unwrap_or_default();
    let name = validate_staged_attachment_file_name(&raw_name)?;
    let _guard = STAGED_AGENT_ATTACHMENTS_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            AppError::new(
                "agent_attachment_staging_failed",
                "The attachment staging lock is unavailable.",
            )
        })?;
    let root = staged_agent_attachments_root(&app)?;
    prepare_staged_agent_attachments_root(&root, bytes.len() as u64)?;
    let parent = root.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir_all(&parent).map_err(staging_error)?;
    let destination = parent.join(name);
    if let Err(error) = fs::write(&destination, bytes) {
        let _ = fs::remove_dir_all(&parent);
        return Err(staging_error(error));
    }
    Ok(destination.to_string_lossy().into_owned())
}

/// Removes temporary DOM-drop files after they have been copied into a session
/// workspace. Picker paths and every path outside the staging root are ignored.
#[tauri::command]
pub fn discard_staged_agent_attachments(
    app: AppHandle,
    request: DiscardStagedAgentAttachmentsRequest,
) -> Result<(), AppError> {
    let _guard = STAGED_AGENT_ATTACHMENTS_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            AppError::new(
                "agent_attachment_staging_failed",
                "The attachment staging lock is unavailable.",
            )
        })?;
    let root = staged_agent_attachments_root(&app)?;
    for path in request.paths {
        let Some(parent) = staged_attachment_parent_for_path(&root, Path::new(&path)) else {
            continue;
        };
        match fs::remove_dir_all(parent) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(staging_error(error)),
        }
    }
    Ok(())
}

/// Reaps expired staging directories while preserving exact staged file paths
/// still referenced by live, queued, or recoverable frontend state.
#[tauri::command]
pub fn prune_staged_agent_attachments(
    app: AppHandle,
    request: PruneStagedAgentAttachmentsRequest,
) -> Result<(), AppError> {
    let _guard = STAGED_AGENT_ATTACHMENTS_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| {
            AppError::new(
                "agent_attachment_staging_failed",
                "The attachment staging lock is unavailable.",
            )
        })?;
    let root = staged_agent_attachments_root(&app)?;
    fs::create_dir_all(&root).map_err(staging_error)?;
    let protected = protected_staging_parents(&root, &request.protected_paths);
    prune_expired_staged_agent_attachments(&root, SystemTime::now(), &protected)
}

#[tauri::command]
pub async fn create_agent_session(
    app: AppHandle,
    request: CreateSessionRequest,
) -> Result<Value, AppError> {
    let repository = repository(&app).await?;
    let model = normalize_agent_model(&request.model);
    let workspace = session_workspace(&app, None)?;
    tokio::fs::create_dir_all(&workspace)
        .await
        .map_err(io_error)?;
    let session = repository
        .create_session_in_profile(
            request.title.as_deref().unwrap_or("New session"),
            &model,
            request.safety_mode,
            workspace.to_str(),
            &request.profile,
        )
        .await?;
    let final_workspace = session_workspace(&app, Some(&session.id))?;
    tokio::fs::create_dir_all(&final_workspace)
        .await
        .map_err(io_error)?;
    sqlx::query::query("UPDATE agent_sessions SET workspace_path = ? WHERE id = ?")
        .bind(final_workspace.to_string_lossy().as_ref())
        .bind(&session.id)
        .execute(&repository.pool)
        .await?;
    Ok(session_json(repository.get_session(&session.id).await?))
}

#[tauri::command]
pub async fn rename_agent_session(
    app: AppHandle,
    request: RenameSessionRequest,
) -> Result<Value, AppError> {
    Ok(session_json(
        repository(&app)
            .await?
            .rename_session(&request.session_id, &request.title)
            .await?,
    ))
}

#[tauri::command]
pub async fn branch_agent_session(
    app: AppHandle,
    session_id: String,
    item_id: String,
) -> Result<Value, AppError> {
    let repository = repository(&app).await?;
    let source = repository.get_session(&session_id).await?;
    let row = sqlx::query::query(
        "SELECT sequence, created_at FROM agent_items WHERE id = ? AND session_id = ?",
    )
    .bind(&item_id)
    .bind(&session_id)
    .fetch_optional(&repository.pool)
    .await?
    .ok_or_else(|| {
        AppError::new(
            "agent_branch_point_missing",
            "The selected message is not available to branch from.",
        )
    })?;
    use sqlx::row::Row;
    let sequence: i64 = row.get("sequence");
    let cutoff_created_at: String = row.get("created_at");
    let workspace = session_workspace(&app, None)?;
    tokio::fs::create_dir_all(&workspace)
        .await
        .map_err(io_error)?;
    let branch = repository
        .create_session(
            &format!("{} branch", source.title),
            &source.model,
            source.safety_mode,
            workspace.to_str(),
        )
        .await?;
    let final_workspace = session_workspace(&app, Some(&branch.id))?;
    tokio::fs::create_dir_all(&final_workspace)
        .await
        .map_err(io_error)?;
    let mut transaction = repository.pool.begin().await?;
    sqlx::query::query("UPDATE agent_sessions SET workspace_path = ? WHERE id = ?")
        .bind(final_workspace.to_string_lossy().as_ref())
        .bind(&branch.id)
        .execute(&mut *transaction)
        .await?;
    inherit_session_profile(
        &mut transaction,
        &session_id,
        &branch.id,
        &crate::db::repositories::timestamp(),
    )
    .await?;
    let item_ids = copy_branch_items(&mut transaction, &session_id, &branch.id, sequence).await?;
    copy_branch_artifacts(
        &mut transaction,
        &session_id,
        &branch.id,
        &cutoff_created_at,
        &final_workspace,
        &item_ids,
    )
    .await?;
    transaction.commit().await?;
    Ok(session_json(repository.get_session(&branch.id).await?))
}

async fn copy_branch_artifacts(
    transaction: &mut sqlx::transaction::Transaction<'_, sqlx_sqlite::Sqlite>,
    source_session_id: &str,
    branch_session_id: &str,
    cutoff_created_at: &str,
    final_workspace: &Path,
    item_ids: &HashMap<String, String>,
) -> Result<(), AppError> {
    use sqlx::row::Row;
    let artifacts = sqlx::query::query(
        "SELECT id, item_id, provenance, action, path, original_path, display_name, mime_type, size_bytes, available, created_at
         FROM agent_artifacts WHERE session_id = ? AND created_at <= ? ORDER BY created_at ASC",
    )
    .bind(source_session_id)
    .bind(cutoff_created_at)
    .fetch_all(&mut **transaction)
    .await?;
    let artifact_root = final_workspace.join("artifacts");
    tokio::fs::create_dir_all(&artifact_root)
        .await
        .map_err(io_error)?;
    let mut path_replacements = Vec::new();
    for artifact in artifacts {
        let source_item_id: Option<String> = artifact.get("item_id");
        // A retained retry can still reference an attachment from a replaced
        // message. Keep the existing artifact-copy contract and rewrite paths
        // in retained payloads even when the original item was omitted.
        let source_path = PathBuf::from(artifact.get::<String, _>("path"));
        let available = artifact.get::<i64, _>("available") != 0 && source_path.is_file();
        let destination = if available {
            let name = source_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("artifact");
            let destination = artifact_root.join(format!("{}-{name}", uuid::Uuid::new_v4()));
            tokio::fs::copy(&source_path, &destination)
                .await
                .map_err(io_error)?;
            path_replacements.push((
                source_path.to_string_lossy().into_owned(),
                destination.to_string_lossy().into_owned(),
            ));
            destination
        } else {
            artifact_root.join(
                source_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("missing-artifact"),
            )
        };
        sqlx::query::query(
            "INSERT INTO agent_artifacts
             (id, session_id, item_id, provenance, action, path, original_path, display_name, mime_type, size_bytes, available, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(branch_session_id)
        .bind(source_item_id.and_then(|id| item_ids.get(&id).cloned()))
        .bind(artifact.get::<String, _>("provenance"))
        .bind(artifact.get::<String, _>("action"))
        .bind(destination.to_string_lossy().as_ref())
        .bind(artifact.get::<Option<String>, _>("original_path"))
        .bind(artifact.get::<Option<String>, _>("display_name"))
        .bind(artifact.get::<Option<String>, _>("mime_type"))
        .bind(artifact.get::<Option<i64>, _>("size_bytes"))
        .bind(i64::from(available))
        .bind(artifact.get::<String, _>("created_at"))
        .execute(&mut **transaction)
        .await?;
    }
    if !path_replacements.is_empty() {
        let branch_items = sqlx::query::query(
            "SELECT id, payload_json FROM agent_items WHERE session_id = ? ORDER BY sequence ASC",
        )
        .bind(branch_session_id)
        .fetch_all(&mut **transaction)
        .await?;
        for item in branch_items {
            let item_id: String = item.get("id");
            let mut payload: Value = serde_json::from_str(&item.get::<String, _>("payload_json"))
                .map_err(|error| {
                AppError::new("agent_branch_payload_invalid", error.to_string())
            })?;
            replace_json_paths(&mut payload, &path_replacements);
            sqlx::query::query("UPDATE agent_items SET payload_json = ? WHERE id = ?")
                .bind(payload.to_string())
                .bind(item_id)
                .execute(&mut **transaction)
                .await?;
        }
    }
    Ok(())
}

// Branches intentionally omit run IDs. Remove replaced attempts before that
// provenance is lost, considering only retries reached at the branch point.
async fn copy_branch_items(
    transaction: &mut sqlx::transaction::Transaction<'_, sqlx_sqlite::Sqlite>,
    source_session_id: &str,
    branch_session_id: &str,
    cutoff_sequence: i64,
) -> Result<HashMap<String, String>, sqlx::Error> {
    use sqlx::row::Row;
    let items = sqlx::query::query(
        "SELECT item.id, item.sequence, item.kind, item.payload_json, item.created_at
         FROM agent_items AS item
         WHERE item.session_id = ? AND item.sequence <= ?
           AND NOT EXISTS (
             SELECT 1 FROM agent_runs AS retry
             WHERE retry.session_id = item.session_id
               AND json_extract(retry.run_config_json, '$.retryOfRunId') = item.run_id
               AND EXISTS (
                 SELECT 1 FROM agent_items AS retry_item
                 WHERE retry_item.session_id = item.session_id
                   AND retry_item.run_id = retry.id AND retry_item.sequence <= ?
               )
           )
         ORDER BY item.sequence ASC",
    )
    .bind(source_session_id)
    .bind(cutoff_sequence)
    .bind(cutoff_sequence)
    .fetch_all(&mut **transaction)
    .await?;
    let mut item_ids = HashMap::new();
    for item in items {
        let source_item_id: String = item.get("id");
        let branch_item_id = uuid::Uuid::new_v4().to_string();
        sqlx::query::query(
            "INSERT INTO agent_items
             (id, session_id, sequence, kind, payload_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&branch_item_id)
        .bind(branch_session_id)
        .bind(item.get::<i64, _>("sequence"))
        .bind(item.get::<String, _>("kind"))
        .bind(item.get::<String, _>("payload_json"))
        .bind(item.get::<String, _>("created_at"))
        .execute(&mut **transaction)
        .await?;
        item_ids.insert(source_item_id, branch_item_id);
    }
    Ok(item_ids)
}

fn replace_json_paths(value: &mut Value, replacements: &[(String, String)]) {
    match value {
        Value::String(text) => {
            if let Some((_, replacement)) = replacements.iter().find(|(source, _)| source == text) {
                *text = replacement.clone();
            }
        }
        Value::Array(values) => {
            for value in values {
                replace_json_paths(value, replacements);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                replace_json_paths(value, replacements);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;

    #[test]
    fn branch_payload_paths_follow_copied_artifacts_without_changing_other_text() {
        let mut payload = json!({
            "output": {
                "path": "/source/session/image.png",
                "caption": "Keep /source/session/image.png in prose unchanged"
            },
            "attachments": [{ "path": "/source/session/image.png" }]
        });
        replace_json_paths(
            &mut payload,
            &[(
                "/source/session/image.png".into(),
                "/branch/session/artifacts/image.png".into(),
            )],
        );

        assert_eq!(
            payload["output"]["path"],
            "/branch/session/artifacts/image.png"
        );
        assert_eq!(
            payload["attachments"][0]["path"],
            "/branch/session/artifacts/image.png"
        );
        assert_eq!(
            payload["output"]["caption"],
            "Keep /source/session/image.png in prose unchanged"
        );
    }
}

#[tauri::command]
pub async fn delete_agent_session(app: AppHandle, session_id: String) -> Result<(), AppError> {
    let repository = repository(&app).await?;
    repository.delete_session(&session_id).await?;
    Ok(())
}

#[tauri::command]
pub async fn list_agent_items(app: AppHandle, session_id: String) -> Result<Vec<Value>, AppError> {
    let repository = repository(&app).await?;
    let active_run_id = repository
        .latest_run(&session_id)
        .await
        .ok()
        .filter(|run| {
            matches!(
                run.status.as_str(),
                "queued" | "running" | "waiting_for_user"
            )
        })
        .map(|run| run.id);
    repository
        .items(&session_id)
        .await?
        .into_iter()
        .map(|item| item_json_with_active_run(item, active_run_id.as_deref()))
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportLegacyHomeTurnsRequest {
    pub session_id: String,
    /// The turns old Home builds kept in local storage, as stored.
    pub turns: Vec<Value>,
}

/// Imports, once, the Home conversation older builds kept only in local
/// storage into the Home session (`super::home_import`, ADR-0064).
#[tauri::command]
pub async fn import_legacy_home_turns(
    app: AppHandle,
    request: ImportLegacyHomeTurnsRequest,
) -> Result<super::home_import::LegacyHomeImport, AppError> {
    let repository = repository(&app).await?;
    let session = repository.get_session(&request.session_id).await?;
    if matches!(session.status.as_str(), "running" | "waiting_for_user") {
        return Err(AppError::new(
            "agent_run_active",
            "This session already has an active run.",
        ));
    }
    Ok(repository
        .import_legacy_home_turns(&session.id, &request.turns)
        .await?)
}

#[tauri::command]
pub async fn start_agent_run(app: AppHandle, request: StartRunRequest) -> Result<Value, AppError> {
    let repository = repository(&app).await?;
    let session = repository.get_session(&request.session_id).await?;
    let model = normalize_agent_model(&request.model);
    if session.status == "running" || session.status == "waiting_for_user" {
        return Err(AppError::new(
            "agent_run_active",
            "This session already has an active run.",
        ));
    }
    let workspace = if request.workspace_path.trim().is_empty() {
        session.workspace_path.clone().unwrap_or(
            session_workspace(&app, Some(&session.id))?
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        request.workspace_path.clone()
    };
    tokio::fs::create_dir_all(&workspace)
        .await
        .map_err(io_error)?;
    sqlx::query::query(
        "UPDATE agent_sessions SET model = ?, safety_mode = ?, workspace_path = ? WHERE id = ?",
    )
    .bind(&model)
    .bind(request.safety_mode.as_db())
    .bind(&workspace)
    .bind(&session.id)
    .execute(&repository.pool)
    .await?;
    let prepared_attachments = prepare_attachments(
        &request.attachments,
        &request.attachment_metadata,
        std::path::Path::new(&workspace),
    )
    .await?;
    let available_skills = agent_skill_catalog(&app, &repository).await?;
    let requested_skills =
        canonical_skill_ids(request.enabled_skill_ids.iter().map(String::as_str))
            .into_iter()
            .filter(|id| {
                available_skills.iter().any(|skill| {
                    skill.get("id").and_then(Value::as_str) == Some(id.as_str())
                        && skill.get("enabled").and_then(Value::as_bool) == Some(true)
                })
            })
            .collect::<Vec<_>>();
    let run = repository.create_run(&session.id, &model, None).await?;
    let surface = if request.surface.as_deref() == Some("home") {
        "home"
    } else {
        "chat"
    };
    if let Err(error) = repository
        .set_run_config(
            &run.id,
            &json!({"surface": surface, "workspace": workspace}),
        )
        .await
    {
        let error = AppError::from(error);
        mark_dispatch_failed(&repository, &run.id, &error).await;
        return Err(error);
    }
    if let Err(error) = repository
        .set_run_enabled_skills(&run.id, &requested_skills)
        .await
    {
        let error = AppError::from(error);
        mark_dispatch_failed(&repository, &run.id, &error).await;
        return Err(error);
    }
    let kind = crate::chat_engine::cli_for_model(&model).expect("normalized CLI model");
    start_cli_run(
        &app,
        &repository,
        CliRunStart {
            session_id: &session.id,
            run_id: &run.id,
            previous_run_id: None,
            kind,
            workspace: &workspace,
            prompt: &request.prompt,
            attachments: &prepared_attachments,
            original_paths: Some(&request.attachments),
            safety_mode: request.safety_mode,
            surface,
        },
    )
    .await
}

struct CliRunStart<'a> {
    session_id: &'a str,
    run_id: &'a str,
    previous_run_id: Option<&'a str>,
    kind: crate::llm::cli::CliKind,
    workspace: &'a str,
    prompt: &'a str,
    attachments: &'a [MessageAttachmentPayload],
    /// The picked paths of a new message (absent on retry).
    original_paths: Option<&'a [String]>,
    safety_mode: AgentSafetyMode,
    surface: &'a str,
}

/// Persists the user message of a run on a CLI chat engine and starts the
/// turn (`crate::chat_engine`), with Clovy tools available through MCP.
async fn start_cli_run(
    app: &AppHandle,
    repository: &AgentRepository,
    start: CliRunStart<'_>,
) -> Result<Value, AppError> {
    // Take cancellation ownership before the first await, so a cancel that
    // arrives while the message is saved or the CLI is resolved stops the
    // turn instead of finding nothing to cancel.
    let engines = app.state::<crate::chat_engine::ChatEngineHost>();
    let ticket = match engines.register(start.run_id) {
        Ok(ticket) => ticket,
        Err(error) => {
            mark_dispatch_failed(repository, start.run_id, &error).await;
            return Err(error);
        }
    };
    let dispatched = async {
        let user_item = repository
            .append_item(
                start.session_id,
                Some(start.run_id),
                0,
                &AgentItemPayload::UserMessage(super::MessagePayload {
                    role: "user".into(),
                    content: start.prompt.to_string(),
                    attachments: start.attachments.to_vec(),
                }),
                Some(&format!("user:{}", start.run_id)),
            )
            .await?
            .ok_or_else(|| {
                AppError::new(
                    "agent_message_persist_failed",
                    "The user message could not be persisted.",
                )
            })?;
        if let Some(original_paths) = start.original_paths {
            persist_attachments(
                repository,
                start.session_id,
                start.run_id,
                &user_item.id,
                start.attachments,
                original_paths,
            )
            .await?;
        }
        crate::chat_engine::start_cli_turn(
            app,
            repository.clone(),
            crate::chat_engine::CliTurnRequest {
                session_id: start.session_id.to_string(),
                run_id: start.run_id.to_string(),
                previous_run_id: start.previous_run_id.map(str::to_string),
                kind: start.kind,
                workspace: PathBuf::from(start.workspace),
                input: crate::chat_engine::with_note_reference_context(
                    &cli_message_with_attachments(start.prompt, start.attachments),
                ),
                images: cli_image_attachments(start.attachments),
                safety_mode: start.safety_mode,
                attended: true,
                surface: start.surface.into(),
                routine_tools: None,
            },
            ticket,
        )
        .await
    }
    .await;
    crate::chat_engine::finish_cli_dispatch(
        app,
        repository,
        start.session_id,
        start.run_id,
        dispatched,
    )
    .await?;
    Ok(run_json(repository.get_run(start.run_id).await?))
}

/// The attachment manifest a CLI turn receives: the CLI reads the files
/// with its own tools.
pub(crate) fn cli_message_with_attachments(
    message: &str,
    attachments: &[MessageAttachmentPayload],
) -> String {
    let manifest = attachments
        .iter()
        .filter(|attachment| attachment.available)
        .map(|attachment| format!("- {} ({})", attachment.name, attachment.path))
        .collect::<Vec<_>>()
        .join("\n");
    if manifest.is_empty() {
        return message.to_string();
    }
    format!(
        "[Clovy attachment manifest v1]\nThe following files are in your workspace. Read them when needed:\n{manifest}\n\n{message}"
    )
}

/// The message's available image attachments, for CLIs that take images.
fn cli_image_attachments(attachments: &[MessageAttachmentPayload]) -> Vec<PathBuf> {
    attachments
        .iter()
        .filter(|attachment| {
            attachment.available
                && attachment
                    .mime_type
                    .as_deref()
                    .is_some_and(is_supported_vision_mime_type)
        })
        .map(|attachment| PathBuf::from(&attachment.path))
        .collect()
}

#[tauri::command]
pub async fn cancel_agent_run(app: AppHandle, run_id: String) -> Result<(), AppError> {
    let repository = repository(&app).await?;
    let run = repository.get_run(&run_id).await?;
    let host = app.state::<crate::chat_engine::ChatEngineHost>();
    // Stop and every bound mutation share SQLite's write lock. Persist Stop
    // before releasing the writer. Signal while that writer is still held so
    // preparation cannot observe cancelled without owning its terminal event.
    let mcp = app.try_state::<crate::mcp_server::McpServerState>();
    let (changed, host_owned_terminal) = claim_ui_run_cancellation_for_host(
        &repository.pool,
        &run.session_id,
        &run.id,
        &host,
        mcp.as_ref().map(|state| state.turns()),
    )
    .await?;
    if !changed {
        return Ok(());
    }
    let broker = app.try_state::<crate::approval_broker::ApprovalBroker>();
    let running = crate::chat_engine::cancel_cli_run(
        &host,
        broker.as_deref(),
        mcp.as_ref().map(|state| state.turns()),
        &run.id,
    )
    .await;
    retire_conversation_run_interruptions(&repository.pool, &run.session_id, &run.id).await?;
    if let Err(error) = crate::routines::mark_agent_run_terminal(&repository.pool, &run.id).await {
        tracing::warn!(error_code=%error.code, "routine cancellation projection failed");
    }
    if !host_owned_terminal && !running {
        let cancelled = repository.get_run(&run.id).await?;
        super::events::emit_persisted_run_cancelled(&app, &repository, &cancelled).await?;
    }
    Ok(())
}

/// The production UI Stop claim, including synchronous host revocation while
/// its winning SQLite writer is still held.
pub(crate) async fn claim_ui_run_cancellation_for_host(
    pool: &sqlx_sqlite::SqlitePool,
    conversation_id: &str,
    run_id: &str,
    host: &crate::chat_engine::ChatEngineHost,
    turns: Option<&crate::mcp_server::turns::TurnRegistry>,
) -> Result<(bool, bool), AppError> {
    claim_ui_run_cancellation_with_signal(pool, conversation_id, run_id, || {
        crate::chat_engine::signal_run_cancellation(host, turns, run_id)
    })
    .await
}

/// Pure admission boundary used by the UI Stop command before host teardown.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn claim_ui_run_cancellation(
    pool: &sqlx_sqlite::SqlitePool,
    conversation_id: &str,
    run_id: &str,
) -> Result<bool, AppError> {
    claim_ui_run_cancellation_with_signal(pool, conversation_id, run_id, || false)
        .await
        .map(|(changed, _)| changed)
}

pub(crate) async fn claim_ui_run_cancellation_with_signal(
    pool: &sqlx_sqlite::SqlitePool,
    conversation_id: &str,
    run_id: &str,
    on_claim: impl FnOnce() -> bool,
) -> Result<(bool, bool), AppError> {
    let mut tx = pool.begin().await?;
    let changed = cancel_active_run_in_transaction(&mut tx, conversation_id, run_id).await?;
    let host_owned_terminal = changed && on_claim();
    tx.commit().await?;
    Ok((changed, host_owned_terminal))
}

async fn cancel_active_run_in_transaction(
    tx: &mut sqlx::transaction::Transaction<'_, sqlx_sqlite::Sqlite>,
    conversation_id: &str,
    run_id: &str,
) -> Result<bool, AppError> {
    let now = crate::db::repositories::timestamp();
    let changed = sqlx::query::query("UPDATE agent_runs SET status='cancelled',updated_at=?,completed_at=?,error_code=NULL,error_message=NULL WHERE id=? AND session_id=? AND status IN ('queued','running','waiting_for_user')")
        .bind(&now).bind(&now).bind(run_id).bind(conversation_id).execute(&mut **tx).await?.rows_affected() != 0;
    if changed {
        sqlx::query::query("UPDATE agent_sessions SET status='idle',updated_at=?,last_error=NULL WHERE id=? AND NOT EXISTS (SELECT 1 FROM agent_runs WHERE session_id=? AND status IN ('queued','running','waiting_for_user'))")
            .bind(&now).bind(conversation_id).bind(conversation_id).execute(&mut **tx).await?;
    }
    Ok(changed)
}

/// Cancel an exact run in a host-selected partition. The database claim and
/// caller validation share the write lock; no UI profile is consulted.
#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn claim_conversation_run_cancellation(
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
    caller_session: &str,
    caller_run: &str,
    conversation_id: &str,
    run_id: &str,
    authority: &crate::domain::turn_authority::TurnAuthority,
) -> Result<bool, AppError> {
    claim_conversation_run_cancellation_with_signal(
        pool,
        profile,
        caller_session,
        caller_run,
        conversation_id,
        run_id,
        authority,
        || false,
    )
    .await
    .map(|(changed, _)| changed)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn claim_conversation_run_cancellation_with_signal(
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
    caller_session: &str,
    caller_run: &str,
    conversation_id: &str,
    run_id: &str,
    authority: &crate::domain::turn_authority::TurnAuthority,
    on_claim: impl FnOnce() -> bool,
) -> Result<(bool, bool), AppError> {
    if run_id == caller_run {
        return Err(AppError::new(
            "conversation_self_cancel",
            "A turn cannot cancel its own run.",
        ));
    }
    let mut tx = pool.begin().await?;
    crate::db::repositories::guard_bound_turn(
        &mut tx,
        profile,
        caller_session,
        caller_run,
        authority,
    )
    .await?;
    let status: Option<String> = sqlx::query_scalar::query_scalar("SELECT r.status FROM agent_runs r JOIN agent_sessions s ON s.id=r.session_id LEFT JOIN session_profiles p ON p.session_id=s.id WHERE r.id=? AND r.session_id=? AND COALESCE(p.profile,'default')=?")
        .bind(run_id).bind(conversation_id).bind(profile).fetch_optional(&mut *tx).await?;
    let status = status
        .ok_or_else(|| AppError::new("run_not_found", "Run was not found in this conversation."))?;
    if !matches!(status.as_str(), "queued" | "running" | "waiting_for_user") {
        tx.commit().await?;
        return Ok((false, false));
    }
    let changed = cancel_active_run_in_transaction(&mut tx, conversation_id, run_id).await?;
    crate::routines::project_agent_run_terminal(
        &mut tx,
        run_id,
        &crate::db::repositories::timestamp(),
    )
    .await?;
    let host_owned_terminal = changed && on_claim();
    tx.commit().await?;
    Ok((changed, host_owned_terminal))
}

/// The production remote Stop claim. Caller validation, the durable winner
/// and the live capability revocation share the claim's admission boundary.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn claim_conversation_run_cancellation_for_host(
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
    caller_session: &str,
    caller_run: &str,
    conversation_id: &str,
    run_id: &str,
    authority: &crate::domain::turn_authority::TurnAuthority,
    host: &crate::chat_engine::ChatEngineHost,
    turns: Option<&crate::mcp_server::turns::TurnRegistry>,
) -> Result<(bool, bool), AppError> {
    claim_conversation_run_cancellation_with_signal(
        pool,
        profile,
        caller_session,
        caller_run,
        conversation_id,
        run_id,
        authority,
        || crate::chat_engine::signal_run_cancellation(host, turns, run_id),
    )
    .await
}

/// Retire durable input cards only after the live approval broker has settled
/// its waiters, preserving its cancellation receipts and UI updates.
pub(crate) async fn retire_conversation_run_interruptions(
    pool: &sqlx_sqlite::SqlitePool,
    conversation_id: &str,
    run_id: &str,
) -> Result<(), AppError> {
    sqlx::query::query("UPDATE agent_items SET payload_json=json_set(payload_json,'$.status','expired','$.resolvedAt',?) WHERE session_id=? AND run_id=? AND kind='interruption' AND json_extract(payload_json,'$.status')='pending' AND EXISTS (SELECT 1 FROM agent_runs r WHERE r.id=? AND r.session_id=? AND r.status='cancelled')")
        .bind(crate::db::repositories::timestamp()).bind(conversation_id).bind(run_id).bind(run_id).bind(conversation_id).execute(pool).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn cancel_conversation_run_in_profile(
    app: &AppHandle,
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
    caller_session: &str,
    caller_run: &str,
    conversation_id: &str,
    run_id: &str,
    authority: &crate::domain::turn_authority::TurnAuthority,
) -> Result<Value, AppError> {
    let host = app
        .try_state::<crate::chat_engine::ChatEngineHost>()
        .ok_or_else(|| {
            AppError::new(
                "agent_runtime_unavailable",
                "The chat engine host is unavailable.",
            )
        })?;
    let mcp = app.try_state::<crate::mcp_server::McpServerState>();
    let (changed, host_owned_terminal) = claim_conversation_run_cancellation_for_host(
        pool,
        profile,
        caller_session,
        caller_run,
        conversation_id,
        run_id,
        authority,
        &host,
        mcp.as_ref().map(|state| state.turns()),
    )
    .await?;
    if changed {
        let broker = app.try_state::<crate::approval_broker::ApprovalBroker>();
        let running = crate::chat_engine::cancel_cli_run(
            &host,
            broker.as_deref(),
            mcp.as_ref().map(|state| state.turns()),
            run_id,
        )
        .await;
        retire_conversation_run_interruptions(pool, conversation_id, run_id).await?;
        if !host_owned_terminal && !running {
            let run = AgentRepository::new(pool.clone()).get_run(run_id).await?;
            super::events::emit_persisted_run_cancelled(
                app,
                &AgentRepository::new(pool.clone()),
                &run,
            )
            .await?;
        }
    }
    let run = AgentRepository::new(pool.clone()).get_run(run_id).await?;
    Ok(
        json!({"conversationId":conversation_id,"runId":run_id,"cancelled":run.status=="cancelled","changed":changed,"status":run.status}),
    )
}

#[tauri::command]
pub async fn retry_agent_run(app: AppHandle, run_id: String) -> Result<Value, AppError> {
    let repository = repository(&app).await?;
    let previous = repository.get_run(&run_id).await?;
    let session = repository.get_session(&previous.session_id).await?;
    let message =
        retry_message(repository.items(&session.id).await?, &previous.id).ok_or_else(|| {
            AppError::new(
                "agent_retry_unavailable",
                "No user message is available to retry.",
            )
        })?;
    let prompt = message.content;
    let attachments = message.attachments;
    let workspace = session.workspace_path.clone().unwrap_or(
        session_workspace(&app, Some(&session.id))?
            .to_string_lossy()
            .into_owned(),
    );
    tokio::fs::create_dir_all(&workspace)
        .await
        .map_err(io_error)?;
    let model = persist_retry_session(&repository, &session, &workspace).await?;
    let enabled_skill_ids = repository.run_enabled_skills(&previous.id).await?;
    let prior_config = repository.run_config(&previous.id).await?;
    let run = repository.create_run(&session.id, &model, None).await?;
    if let Err(error) = repository.set_run_retry_parent(&run.id, &previous.id).await {
        let error = AppError::from(error);
        mark_dispatch_failed(&repository, &run.id, &error).await;
        return Err(error);
    }
    if let Err(error) = repository
        .set_run_enabled_skills(&run.id, &enabled_skill_ids)
        .await
    {
        let error = AppError::from(error);
        mark_dispatch_failed(&repository, &run.id, &error).await;
        return Err(error);
    }
    let kind = crate::chat_engine::cli_for_model(&model).expect("normalized CLI model");
    let surface = if prior_config
        .as_ref()
        .and_then(|v| v.get("surface"))
        .and_then(Value::as_str)
        == Some("home")
    {
        "home"
    } else {
        "chat"
    };
    if let Err(error) = repository
        .set_run_config(
            &run.id,
            &json!({"surface": surface, "workspace": workspace}),
        )
        .await
    {
        let error = AppError::from(error);
        mark_dispatch_failed(&repository, &run.id, &error).await;
        return Err(error);
    }
    start_cli_run(
        &app,
        &repository,
        CliRunStart {
            session_id: &session.id,
            run_id: &run.id,
            previous_run_id: Some(&previous.id),
            kind,
            workspace: &workspace,
            prompt: &prompt,
            attachments: &attachments,
            original_paths: None,
            safety_mode: session.safety_mode,
            surface,
        },
    )
    .await
}

async fn persist_retry_session(
    repository: &AgentRepository,
    session: &super::AgentSessionDto,
    workspace: &str,
) -> Result<String, AppError> {
    let model = normalize_agent_model(&session.model);
    if model != session.model || session.workspace_path.is_none() {
        sqlx::query::query("UPDATE agent_sessions SET model = ?, workspace_path = ? WHERE id = ?")
            .bind(&model)
            .bind(workspace)
            .bind(&session.id)
            .execute(&repository.pool)
            .await?;
    }
    Ok(model)
}

fn retry_message(items: Vec<AgentItemDto>, run_id: &str) -> Option<super::MessagePayload> {
    items.into_iter().rev().find_map(|item| match item.payload {
        AgentItemPayload::UserMessage(message) if item.run_id.as_deref() == Some(run_id) => {
            Some(message)
        }
        _ => None,
    })
}

#[tauri::command]
pub async fn resolve_agent_interruption(
    app: AppHandle,
    request: ResolveInterruptionRequest,
) -> Result<Value, AppError> {
    // Items asked by a CLI turn (`engine: "cli"`) wait in the approval
    // broker for the lifetime of the local CLI turn.
    if let Some(run) = crate::approval_broker::commands::resolve_cli_interruption(
        &app,
        &request.run_id,
        &request.interruption_id,
        &request.resolution,
    )
    .await?
    {
        return Ok(run_json(run));
    }
    Err(AppError::new(
        "agent_interruption_retired",
        "This request belongs to the retired chat engine. Retry the conversation with Claude Code.",
    ))
}

async fn mark_dispatch_failed(repository: &AgentRepository, run_id: &str, error: &AppError) {
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
        tracing::warn!(
            error = %persist_error,
            run_id,
            "failed to persist agent dispatch failure"
        );
    }
}

#[tauri::command]
pub async fn list_agent_artifacts(
    app: AppHandle,
    session_id: String,
) -> Result<Vec<Value>, AppError> {
    let repository = repository(&app).await?;
    Ok(repository.artifacts(&session_id).await?.into_iter().map(|artifact| {
        let name = artifact_display_name(
            &artifact.path,
            artifact.original_path.as_deref(),
            &artifact.provenance,
            artifact.display_name.as_deref(),
        );
        json!({ "id": artifact.id, "sessionId": artifact.session_id, "runId": artifact.run_id, "itemId": artifact.item_id, "name": name, "path": artifact.path, "mimeType": artifact.mime_type, "sizeBytes": artifact.size_bytes, "action": artifact.action, "available": artifact.available, "createdAt": artifact.created_at })
    }).collect())
}

fn artifact_display_name(
    path: &str,
    original_path: Option<&str>,
    provenance: &str,
    persisted_name: Option<&str>,
) -> String {
    if let Some(name) = persisted_name {
        return name.to_string();
    }
    let stored_name = PathBuf::from(path)
        .file_name()
        .map(|value| value.to_string_lossy().into_owned());
    if provenance == "attachment" {
        let original_name = original_path
            .and_then(|value| PathBuf::from(value).file_name().map(|name| name.to_owned()))
            .map(|value| value.to_string_lossy().into_owned());
        if let Some(name) = stored_name
            .as_deref()
            .and_then(|name| copied_attachment_name(name, original_name.as_deref()))
        {
            return name.to_string();
        }
        if let Some(name) = original_name {
            return name;
        }
    }
    stored_name.unwrap_or_else(|| "Artifact".into())
}

fn copied_attachment_name<'a>(
    stored_name: &'a str,
    original_name: Option<&str>,
) -> Option<&'a str> {
    let mut name = stored_name;
    let mut stripped = false;
    while let Some(remainder) = strip_copy_prefix(name) {
        stripped = true;
        name = remainder;
        if original_name == Some(name) {
            break;
        }
    }
    stripped.then_some(name)
}

fn strip_copy_prefix(name: &str) -> Option<&str> {
    let uuid_prefix = name.get(..36)?;
    let remainder = name.get(37..)?;
    (name.as_bytes().get(36) == Some(&b'-')
        && uuid::Uuid::parse_str(uuid_prefix).is_ok()
        && !remainder.is_empty())
    .then_some(remainder)
}

#[tauri::command]
pub async fn read_agent_artifact_preview(
    app: AppHandle,
    request: ReadArtifactRequest,
) -> Result<Option<String>, AppError> {
    let path = authorized_artifact_path(&app, &request.path).await?;
    let Some(mime_type) = image_mime_type(&path) else {
        return Ok(None);
    };
    let bytes = tokio::fs::read(&path).await.map_err(io_error)?;
    if bytes.len() > 10 * 1024 * 1024 {
        return Ok(None);
    }
    Ok(Some(format!(
        "data:{mime_type};base64,{}",
        BASE64.encode(bytes)
    )))
}

#[tauri::command]
pub async fn read_agent_artifact_text(
    app: AppHandle,
    request: ReadArtifactRequest,
) -> Result<Option<String>, AppError> {
    let path = authorized_artifact_path(&app, &request.path).await?;
    let metadata = tokio::fs::metadata(&path).await.map_err(io_error)?;
    if metadata.len() > 1024 * 1024 {
        return Ok(None);
    }
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => Ok(None),
        Err(error) => Err(io_error(error)),
    }
}

#[tauri::command]
pub async fn download_agent_artifact(
    app: AppHandle,
    request: ReadArtifactRequest,
) -> Result<String, AppError> {
    let source = authorized_artifact_path(&app, &request.path).await?;
    let downloads = app
        .path()
        .download_dir()
        .map_err(|error| AppError::new("agent_artifact_download_failed", error.to_string()))?;
    tokio::fs::create_dir_all(&downloads)
        .await
        .map_err(|error| AppError::new("agent_artifact_download_failed", error.to_string()))?;
    let destination = unique_download_path(&downloads, &source)?;
    tokio::fs::copy(&source, &destination)
        .await
        .map_err(|error| AppError::new("agent_artifact_download_failed", error.to_string()))?;
    Ok(destination.to_string_lossy().into_owned())
}

fn unique_download_path(downloads: &Path, source: &Path) -> Result<PathBuf, AppError> {
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            AppError::new(
                "agent_artifact_download_failed",
                "The Clovy file does not have a downloadable filename.",
            )
        })?;
    let candidate = downloads.join(file_name);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let stem = source
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("download");
    let extension = source.extension().and_then(|name| name.to_str());
    for index in 1..1000 {
        let file_name = match extension {
            Some(extension) if !extension.is_empty() => format!("{stem} ({index}).{extension}"),
            _ => format!("{stem} ({index})"),
        };
        let candidate = downloads.join(file_name);
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(AppError::new(
        "agent_artifact_download_failed",
        "Could not find an available Downloads filename.",
    ))
}

async fn authorized_artifact_path(app: &AppHandle, requested: &str) -> Result<PathBuf, AppError> {
    use sqlx::row::Row;
    let repository = repository(app).await?;
    let row = sqlx::query::query(
        "SELECT path FROM agent_artifacts WHERE path = ? AND available = 1 LIMIT 1",
    )
    .bind(requested)
    .fetch_optional(&repository.pool)
    .await?;
    let path: String = row
        .ok_or_else(|| AppError::new("agent_artifact_unavailable", "Artifact is unavailable."))?
        .get("path");
    PathBuf::from(path)
        .canonicalize()
        .map_err(|_| AppError::new("agent_artifact_unavailable", "Artifact is unavailable."))
}

fn image_mime_type(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "bmp" => Some("image/bmp"),
        "tif" | "tiff" => Some("image/tiff"),
        _ => None,
    }
}

#[tauri::command]
pub async fn list_agent_skills(app: AppHandle) -> Result<Vec<Value>, AppError> {
    let repository = repository(&app).await?;
    agent_skill_catalog(&app, &repository).await
}

#[tauri::command]
pub async fn read_agent_skill(app: AppHandle, skill_id: String) -> Result<Value, AppError> {
    super::skills::read_from_roots(&super::skills::skill_roots(&app), &skill_id).await
}

#[tauri::command]
pub async fn update_agent_skill(
    app: AppHandle,
    request: UpdateSkillRequest,
) -> Result<Value, AppError> {
    super::skills::update_from_roots(
        &super::skills::skill_roots(&app),
        &request.skill_id,
        &request.content,
    )
    .await?;
    let repository = repository(&app).await?;
    agent_skill_catalog(&app, &repository)
        .await?
        .into_iter()
        .find(|skill| skill["id"] == canonical_skill_id(&request.skill_id))
        .ok_or_else(|| AppError::new("agent_skill_not_found", "Skill was not found."))
}

pub(crate) async fn agent_skill_catalog(
    app: &AppHandle,
    repository: &AgentRepository,
) -> Result<Vec<Value>, AppError> {
    let overrides = super::skill_identity::skill_enabled_overrides(
        repository
            .skills()
            .await?
            .into_iter()
            .map(|skill| (skill.id, skill.enabled)),
    );
    agent_skill_catalog_from_roots(super::skills::skill_roots(app), &overrides).await
}

async fn agent_skill_catalog_from_roots(
    roots: Vec<(PathBuf, bool)>,
    overrides: &HashMap<String, bool>,
) -> Result<Vec<Value>, AppError> {
    super::skills::catalog_from_roots(roots, overrides).await
}

#[tauri::command]
pub async fn set_agent_skill_enabled(
    app: AppHandle,
    request: SetSkillEnabledRequest,
) -> Result<Value, AppError> {
    let repository = repository(&app).await?;
    let catalog = agent_skill_catalog(&app, &repository).await?;
    let id = canonical_skill_id(&request.skill_id);
    if !super::skill_identity::valid_skill_id(&request.skill_id) {
        return Err(AppError::new("agent_skill_invalid", "Skill id is invalid."));
    }
    let skill = catalog
        .iter()
        .find(|skill| skill["id"] == id)
        .ok_or_else(|| AppError::new("agent_skill_not_found", "Skill was not found."))?;
    if skill["editable"] != true {
        return Err(AppError::new(
            "agent_skill_read_only",
            "User-global skills are read-only in Clovy.",
        ));
    }
    super::skills::set_enabled_from_roots(
        &repository.pool,
        &super::skills::skill_roots(&app),
        id,
        request.enabled,
        None,
    )
    .await?;
    agent_skill_catalog(&app, &repository)
        .await?
        .into_iter()
        .find(|skill| skill["id"] == id)
        .ok_or_else(|| {
            AppError::new(
                "agent_skill_not_found",
                "Skill was not found after changing its setting.",
            )
        })
}

#[cfg(test)]
use super::{skill_catalog::skill_description, skill_identity::skill_file};

/// Historical native/endpoint sessions continue through the selected local CLI.
fn normalize_agent_model(model: &str) -> String {
    if let Some(kind) = crate::chat_engine::cli_for_model(model) {
        crate::chat_engine::engine_model_id(kind)
    } else {
        crate::providers::selected_chat_engine_model()
    }
}

fn session_json(session: super::AgentSessionDto) -> Value {
    json!({ "id": session.id, "title": session.title, "status": session.status, "model": session.model, "safetyMode": session.safety_mode, "workspacePath": session.workspace_path.unwrap_or_default(), "source": match session.source.as_str() { "legacy_routine" => "legacy_routine", "routine" => "routine", "user" => "user", _ => "legacy_task" }, "createdAt": session.created_at, "updatedAt": session.updated_at, "error": session.last_error })
}
pub(crate) fn run_json(run: super::AgentRunDto) -> Value {
    json!({ "id": run.id, "sessionId": run.session_id, "status": run.status, "model": run.model, "approvalMode": run.approval_mode, "reasoningEffort": run.reasoning_effort, "startedAt": run.started_at, "completedAt": run.completed_at, "usage": run.usage, "error": run.error_message })
}

pub(crate) fn item_json_with_active_run(
    item: AgentItemDto,
    active_run_id: Option<&str>,
) -> Result<Value, AppError> {
    let is_active_run = item.run_id.as_deref() == active_run_id;
    let stable_stream_id = is_active_run
        .then_some(item.external_id.as_deref())
        .flatten()
        .filter(|external_id| {
            external_id.starts_with("assistant:") || external_id.starts_with("reasoning:")
        });
    let stable_steering_id = item
        .external_id
        .as_deref()
        .filter(|external_id| external_id.starts_with("steering:"));
    let public_item_id = stable_steering_id
        .or(stable_stream_id)
        .unwrap_or(&item.id)
        .to_string();
    let stream_status =
        if stable_stream_id.is_some_and(|external_id| external_id.starts_with("assistant:")) {
            "streaming"
        } else {
            "complete"
        };
    let base = json!({ "id": public_item_id, "sessionId": item.session_id, "runId": item.run_id, "sequence": item.sequence, "createdAt": item.created_at });
    let mut object = base.as_object().cloned().expect("base object");
    let fields = match item.payload {
        AgentItemPayload::UserMessage(v)
        | AgentItemPayload::AssistantMessage(v)
        | AgentItemPayload::SystemMessage(v) => {
            let attachments = v
                .attachments
                .into_iter()
                .map(|attachment| {
                    json!({
                        "id": attachment.id,
                        "sessionId": &item.session_id,
                        "runId": &item.run_id,
                        "itemId": &item.id,
                        "name": attachment.name,
                        "path": attachment.path,
                        "mimeType": attachment.mime_type,
                        "sizeBytes": attachment.size_bytes,
                        "action": "imported",
                        "available": attachment.available,
                        "createdAt": attachment.created_at
                    })
                })
                .collect::<Vec<_>>();
            json!({ "kind": "message", "role": v.role, "text": v.content, "status": stream_status, "attachments": attachments })
        }
        AgentItemPayload::Reasoning(v) => {
            json!({ "kind": "reasoning", "text": v.text, "status": stream_status })
        }
        AgentItemPayload::Steering(v) => json!({ "kind": "steering", "text": v.text }),
        AgentItemPayload::ContextSummary(v) => {
            json!({ "kind": "context_summary", "text": v.text, "metadata": v.metadata })
        }
        AgentItemPayload::ToolCall(v) => {
            json!({ "kind": "tool_call", "callId": v.tool_call_id.unwrap_or_default(), "name": v.tool_name.unwrap_or_default(), "arguments": v.arguments, "status": v.status.unwrap_or_else(|| "complete".into()) })
        }
        AgentItemPayload::ToolResult(v) => {
            let is_error = v.status.as_deref() == Some("failed");
            let output = if is_error {
                v.error.map(Value::String).or(v.result)
            } else {
                v.result
            };
            json!({ "kind": "tool_result", "callId": v.tool_call_id.unwrap_or_default(), "name": v.tool_name.unwrap_or_default(), "output": output, "isError": is_error })
        }
        AgentItemPayload::Interruption(v) => json!({ "kind": "interruption", "interruption": v }),
        AgentItemPayload::Error(v) => {
            json!({ "kind": "error", "message": v.get("message").cloned().unwrap_or_else(|| json!("Agent run failed.")), "category": v.get("category").cloned().unwrap_or_else(|| json!("provider")), "code": v.get("code").cloned().unwrap_or_else(|| json!("agent_run_failed")), "retryable": v.get("retryable").cloned().unwrap_or(Value::Bool(true)) })
        }
    };
    object.extend(fields.as_object().cloned().expect("fields object"));
    Ok(Value::Object(object))
}

fn session_workspace(app: &AppHandle, session_id: Option<&str>) -> Result<PathBuf, AppError> {
    let root = crate::app_paths::app_data_dir(app)
        .map_err(|error| AppError::new("agent_workspace_failed", error.to_string()))?
        .join("agent-workspaces");
    Ok(session_id.map_or_else(
        || root.join(uuid::Uuid::new_v4().to_string()),
        |id| root.join(id),
    ))
}

fn staged_agent_attachments_root(app: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(crate::app_paths::app_data_dir(app)
        .map_err(|error| AppError::new("agent_attachment_staging_failed", error.to_string()))?
        .join(STAGED_AGENT_ATTACHMENTS_DIR))
}

fn validate_staged_attachment_size(size: usize) -> Result<(), AppError> {
    if size > MAX_STAGED_AGENT_ATTACHMENT_BYTES {
        return Err(AppError::new(
            "agent_attachment_staging_denied",
            "Dropped files must be 50 MB or smaller.",
        ));
    }
    Ok(())
}

fn validate_staged_attachment_file_name(raw_name: &str) -> Result<String, AppError> {
    let name = raw_name;
    let is_safe_bare_name = !name.trim().is_empty()
        && name.len() <= 255
        && name.chars().count() <= 255
        && !name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
        && matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
        && Path::new(name).components().count() == 1;
    if !is_safe_bare_name {
        return Err(AppError::new(
            "agent_attachment_staging_invalid",
            "The dropped file does not have a usable filename.",
        ));
    }
    Ok(name.to_string())
}

fn prepare_staged_agent_attachments_root(root: &Path, incoming_bytes: u64) -> Result<(), AppError> {
    fs::create_dir_all(root).map_err(staging_error)?;
    if count_canonical_staging_directories(root)? >= MAX_STAGED_AGENT_ATTACHMENT_DIRECTORIES {
        return Err(AppError::new(
            "agent_attachment_staging_denied",
            "Clovy has too many temporary attachments. Remove attachments or try again later.",
        ));
    }
    let staged_bytes = directory_bytes_without_following_symlinks(root)?;
    if staged_bytes.saturating_add(incoming_bytes) > MAX_STAGED_AGENT_ATTACHMENT_TOTAL_BYTES {
        return Err(AppError::new(
            "agent_attachment_staging_denied",
            "Clovy's temporary attachment storage is full. Remove attachments or try again later.",
        ));
    }
    Ok(())
}

fn count_canonical_staging_directories(root: &Path) -> Result<usize, AppError> {
    let mut count = 0;
    for entry in fs::read_dir(root).map_err(staging_error)? {
        let entry = entry.map_err(staging_error)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_canonical_staging_uuid(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(staging_error)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            count += 1;
        }
    }
    Ok(count)
}

fn protected_staging_parents(root: &Path, paths: &[String]) -> HashSet<PathBuf> {
    paths
        .iter()
        .filter_map(|path| staged_attachment_parent_for_path(root, Path::new(path)))
        .collect()
}

fn prune_expired_staged_agent_attachments(
    root: &Path,
    now: SystemTime,
    protected: &HashSet<PathBuf>,
) -> Result<(), AppError> {
    let canonical_root = root.canonicalize().map_err(staging_error)?;
    for entry in fs::read_dir(&canonical_root).map_err(staging_error)? {
        let entry = entry.map_err(staging_error)?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !is_canonical_staging_uuid(&name) {
            continue;
        }
        let metadata = fs::symlink_metadata(entry.path()).map_err(staging_error)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        if protected.contains(&entry.path()) {
            continue;
        }
        let expired = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age >= STAGED_AGENT_ATTACHMENT_TTL);
        if expired {
            fs::remove_dir_all(entry.path()).map_err(staging_error)?;
        }
    }
    Ok(())
}

fn directory_bytes_without_following_symlinks(path: &Path) -> Result<u64, AppError> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(staging_error)? {
        let entry = entry.map_err(staging_error)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(staging_error)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            total =
                total.saturating_add(directory_bytes_without_following_symlinks(&entry.path())?);
        }
    }
    Ok(total)
}

fn is_canonical_staging_uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}

fn staged_attachment_parent_for_path(root: &Path, path: &Path) -> Option<PathBuf> {
    let canonical_root = root.canonicalize().ok()?;
    let canonical_path = path.canonicalize().ok()?;
    let mut components = canonical_path
        .strip_prefix(&canonical_root)
        .ok()?
        .components();
    let parent = components.next()?.as_os_str().to_str()?;
    is_canonical_staging_uuid(parent).then_some(())?;
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return None;
    }
    Some(canonical_root.join(parent))
}

fn staging_error(error: std::io::Error) -> AppError {
    AppError::new("agent_attachment_staging_failed", error.to_string())
}

fn io_error(error: std::io::Error) -> AppError {
    AppError::new("agent_workspace_failed", error.to_string())
}

async fn inherit_session_profile(
    transaction: &mut sqlx::transaction::Transaction<'_, sqlx_sqlite::Sqlite>,
    source_session_id: &str,
    branch_session_id: &str,
    assigned_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query::query(
        "INSERT INTO session_profiles (session_id, profile, assigned_at)
         VALUES (?, COALESCE((SELECT profile FROM session_profiles WHERE session_id = ?), 'default'), ?)",
    )
    .bind(branch_session_id)
    .bind(source_session_id)
    .bind(assigned_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn prepare_attachments(
    source_paths: &[String],
    supplied_metadata: &[AttachmentMetadataInput],
    workspace: &std::path::Path,
) -> Result<Vec<MessageAttachmentPayload>, AppError> {
    if source_paths.is_empty() {
        return Ok(Vec::new());
    }
    if !supplied_metadata.is_empty() && supplied_metadata.len() != source_paths.len() {
        return Err(AppError::new(
            "agent_attachment_invalid",
            "Attachment metadata does not match the selected files.",
        ));
    }
    let destination_root = workspace.join("attachments");
    tokio::fs::create_dir_all(&destination_root)
        .await
        .map_err(io_error)?;
    let canonical_workspace = workspace.canonicalize().map_err(io_error)?;
    let mut attachments = Vec::with_capacity(source_paths.len());
    for (index, source_path) in source_paths.iter().enumerate() {
        let source = PathBuf::from(source_path)
            .canonicalize()
            .map_err(io_error)?;
        if !source.is_file() {
            return Err(AppError::new(
                "agent_attachment_invalid",
                "Attachment source is not a file.",
            ));
        }
        let name = supplied_metadata
            .get(index)
            .map(|metadata| metadata.name.trim())
            .or_else(|| source.file_name().and_then(|value| value.to_str()))
            .filter(|value| {
                !value.is_empty()
                    && value.len() <= 255
                    && !value
                        .chars()
                        .any(|character| character.is_control() || matches!(character, '/' | '\\'))
            })
            .ok_or_else(|| {
                AppError::new(
                    "agent_attachment_invalid",
                    "Attachment source has no valid file name.",
                )
            })?
            .to_string();
        let destination = if source.starts_with(&canonical_workspace) {
            source
        } else {
            let destination = destination_root.join(format!("{}-{name}", uuid::Uuid::new_v4()));
            tokio::fs::copy(&source, &destination)
                .await
                .map_err(io_error)?;
            destination
        };
        let metadata = tokio::fs::metadata(&destination).await.map_err(io_error)?;
        attachments.push(MessageAttachmentPayload {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            path: destination.to_string_lossy().into_owned(),
            mime_type: supplied_metadata
                .get(index)
                .and_then(|metadata| metadata.media_type.clone())
                .or_else(|| attachment_mime_type(&destination).map(str::to_string)),
            size_bytes: metadata.len() as i64,
            available: true,
            created_at: crate::db::repositories::timestamp(),
        });
    }
    Ok(attachments)
}

async fn persist_attachments(
    repository: &AgentRepository,
    session_id: &str,
    run_id: &str,
    item_id: &str,
    attachments: &[MessageAttachmentPayload],
    original_paths: &[String],
) -> Result<(), AppError> {
    for (index, attachment) in attachments.iter().enumerate() {
        sqlx::query::query(
            "INSERT INTO agent_artifacts (
                id, session_id, run_id, item_id, provenance, action, path,
                original_path, display_name, mime_type, size_bytes, available, created_at
             ) VALUES (?, ?, ?, ?, 'attachment', 'imported', ?, ?, ?, ?, ?, 1, ?)",
        )
        .bind(&attachment.id)
        .bind(session_id)
        .bind(run_id)
        .bind(item_id)
        .bind(&attachment.path)
        .bind(original_paths.get(index))
        .bind(&attachment.name)
        .bind(&attachment.mime_type)
        .bind(attachment.size_bytes)
        .bind(&attachment.created_at)
        .execute(&repository.pool)
        .await?;
    }
    Ok(())
}

fn is_supported_vision_mime_type(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp"
    )
}

fn attachment_mime_type(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "tif" | "tiff" => Some("image/tiff"),
        "pdf" => Some("application/pdf"),
        "json" => Some("application/json"),
        "csv" => Some("text/csv"),
        "md" => Some("text/markdown"),
        "txt" => Some("text/plain"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn review_delta_branch_retry_keeps_independent_attachment_copy() {
        let broker = crate::approval_broker::testing::TestBroker::new([]).await;
        let repository = AgentRepository::new(broker.pool().clone());
        let source = repository
            .create_session("Source", "cli:claude", AgentSafetyMode::Sandboxed, None)
            .await
            .unwrap();
        let branch = repository
            .create_session("Branch", "cli:claude", AgentSafetyMode::Sandboxed, None)
            .await
            .unwrap();
        let files = tempfile::tempdir().unwrap();
        let source_path = files.path().join("source.txt");
        std::fs::write(&source_path, "independent attachment").unwrap();
        let attachment = MessageAttachmentPayload {
            id: "source-artifact".into(),
            name: "source.txt".into(),
            path: source_path.to_string_lossy().into_owned(),
            mime_type: Some("text/plain".into()),
            size_bytes: 22,
            available: true,
            created_at: source.created_at.clone(),
        };
        let payload = super::super::AgentItemPayload::UserMessage(super::super::MessagePayload {
            role: "user".into(),
            content: "Read the attachment".into(),
            attachments: vec![attachment],
        });
        let original = repository
            .create_run(&source.id, "cli:claude", None)
            .await
            .unwrap();
        let first = repository
            .append_item(&source.id, Some(&original.id), 0, &payload, None)
            .await
            .unwrap()
            .unwrap();
        let retry = repository
            .create_run(&source.id, "cli:claude", None)
            .await
            .unwrap();
        repository
            .set_run_retry_parent(&retry.id, &original.id)
            .await
            .unwrap();
        let last = repository
            .append_item(&source.id, Some(&retry.id), 0, &payload, None)
            .await
            .unwrap()
            .unwrap();
        sqlx::query::query("INSERT INTO agent_artifacts (id, session_id, item_id, provenance, action, path, available, created_at) VALUES ('source-artifact', ?, ?, 'user', 'attachment', ?, 1, ?)")
            .bind(&source.id).bind(&first.id).bind(source_path.to_str().unwrap()).bind(&first.created_at).execute(&repository.pool).await.unwrap();
        let mut tx = repository.pool.begin().await.unwrap();
        let mapping = copy_branch_items(&mut tx, &source.id, &branch.id, last.sequence)
            .await
            .unwrap();
        copy_branch_artifacts(
            &mut tx,
            &source.id,
            &branch.id,
            &last.created_at,
            &files.path().join("branch"),
            &mapping,
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        let artifacts = repository.artifacts(&branch.id).await.unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_ne!(artifacts[0].path, source_path.to_string_lossy());
        std::fs::remove_file(&source_path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&artifacts[0].path).unwrap(),
            "independent attachment"
        );
        let copied = repository.items(&branch.id).await.unwrap();
        assert_eq!(copied.len(), 1);
        let super::super::AgentItemPayload::UserMessage(message) = &copied[0].payload else {
            panic!("user message");
        };
        assert_eq!(message.attachments[0].path, artifacts[0].path);
    }

    #[tokio::test]
    async fn branches_exclude_replaced_attempts_only_up_to_the_branch_point() {
        let broker = crate::approval_broker::testing::TestBroker::new([]).await;
        let repository = AgentRepository::new(broker.pool().clone());
        let source = repository
            .create_session("Source", "cli:claude", AgentSafetyMode::Sandboxed, None)
            .await
            .unwrap();
        let mut runs: Vec<String> = Vec::new();
        let mut items = Vec::new();
        for content in ["rejected first", "rejected second", "accepted third"] {
            let run = repository
                .create_run(&source.id, "cli:claude", None)
                .await
                .unwrap();
            if let Some(previous) = runs.last() {
                repository
                    .set_run_retry_parent(&run.id, previous)
                    .await
                    .unwrap();
            }
            let item = repository
                .append_item(
                    &source.id,
                    Some(&run.id),
                    0,
                    &super::super::AgentItemPayload::AssistantMessage(
                        super::super::MessagePayload {
                            role: "assistant".into(),
                            content: content.into(),
                            attachments: Vec::new(),
                        },
                    ),
                    None,
                )
                .await
                .unwrap()
                .unwrap();
            runs.push(run.id);
            items.push(item);
        }
        for (cutoff, expected) in [(0, 0), (1, 1), (2, 2)] {
            let branch = repository
                .create_session("Branch", "cli:claude", AgentSafetyMode::Sandboxed, None)
                .await
                .unwrap();
            let mut transaction = repository.pool.begin().await.unwrap();
            let mapping = copy_branch_items(&mut transaction, &source.id, &branch.id, cutoff)
                .await
                .unwrap();
            transaction.commit().await.unwrap();
            let copied = repository.items(&branch.id).await.unwrap();
            assert_eq!(
                copied.len(),
                1,
                "cutoff {cutoff} must not resurrect replaced attempts"
            );
            assert_eq!(
                copied[0].payload.value().unwrap(),
                items[expected].payload.value().unwrap()
            );
            assert_eq!(mapping.len(), 1);
            assert!(mapping.contains_key(&items[expected].id));
            assert!(copied[0].run_id.is_none());
        }
    }

    #[tokio::test]
    async fn retry_persists_missing_workspace_for_an_unchanged_cli_engine() {
        let broker = crate::approval_broker::testing::TestBroker::new([]).await;
        let repository = AgentRepository::new(broker.pool().clone());
        let model = crate::chat_engine::engine_model_id(crate::llm::cli::CliKind::Claude);
        let session = repository
            .create_session(
                "Imported CLI session",
                &model,
                AgentSafetyMode::Sandboxed,
                None,
            )
            .await
            .unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let workspace = workspace.path().to_str().unwrap();
        assert_eq!(
            persist_retry_session(&repository, &session, workspace)
                .await
                .unwrap(),
            model
        );
        let saved = repository.get_session(&session.id).await.unwrap();
        assert_eq!(saved.workspace_path.as_deref(), Some(workspace));
        assert_eq!(saved.model, model);
    }

    #[test]
    fn historical_native_and_endpoint_models_retry_on_a_cli() {
        for model in [
            "auto",
            "open-software/auto",
            "openai/gpt-4.1",
            "__clovy_endpoint__:local",
        ] {
            assert!(crate::chat_engine::cli_for_model(&normalize_agent_model(model)).is_some());
        }
        let codex = crate::chat_engine::engine_model_id(crate::llm::cli::CliKind::Codex);
        assert_eq!(normalize_agent_model(&codex), codex);
    }

    #[test]
    fn attachment_display_name_strips_the_internal_copy_prefix() {
        assert_eq!(
            artifact_display_name(
                "/workspace/attachments/1fb34e2e-e4e7-45d7-b538-a87aefa4c8e4-Pool Day.pdf",
                Some("/tmp/companion/content"),
                "attachment",
                None,
            ),
            "Pool Day.pdf"
        );
    }

    #[test]
    fn attachment_display_name_falls_back_to_the_original_path() {
        assert_eq!(
            artifact_display_name(
                "/workspace/attachments/staged-content",
                Some("/Users/andrew/Documents/Pool Day.pdf"),
                "attachment",
                None,
            ),
            "Pool Day.pdf"
        );
    }

    #[test]
    fn attachment_display_name_strips_copy_prefixes_from_repeated_branches() {
        assert_eq!(
            artifact_display_name(
                "/workspace/artifacts/9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-1fb34e2e-e4e7-45d7-b538-a87aefa4c8e4-Pool Day.pdf",
                Some("/Users/andrew/Documents/Pool Day.pdf"),
                "attachment",
                None,
            ),
            "Pool Day.pdf"
        );
    }

    #[test]
    fn attachment_display_name_preserves_a_uuid_prefixed_original_name() {
        assert_eq!(
            artifact_display_name(
                "/workspace/attachments/1fb34e2e-e4e7-45d7-b538-a87aefa4c8e4-9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-notes.pdf",
                Some("/Users/andrew/Documents/9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-notes.pdf"),
                "attachment",
                None,
            ),
            "9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-notes.pdf"
        );
    }

    #[test]
    fn attachment_display_name_prefers_the_persisted_metadata_name() {
        let path = "/workspace/attachments/1fb34e2e-e4e7-45d7-b538-a87aefa4c8e4-9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-notes.pdf";
        let persisted_name = "9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-notes.pdf";

        assert_eq!(
            artifact_display_name(
                path,
                Some("/tmp/companion/content"),
                "attachment",
                Some(persisted_name),
            ),
            persisted_name
        );
    }

    #[test]
    fn generated_artifact_display_name_keeps_its_workspace_name() {
        assert_eq!(
            artifact_display_name(
                "/workspace/artifacts/generated-report.pdf",
                Some("/Users/andrew/Documents/source.pdf"),
                "tool",
                None,
            ),
            "generated-report.pdf"
        );
    }

    #[test]
    fn staged_attachment_file_names_must_be_safe_bare_names() {
        for name in ["brief.pdf", "Résumé.pdf", " spaced name.txt "] {
            assert!(
                validate_staged_attachment_file_name(name).is_ok(),
                "{name:?}"
            );
        }
        for name in [
            "",
            "..",
            "../brief.pdf",
            "folder/brief.pdf",
            "folder\\brief.pdf",
            "a\n.txt",
        ] {
            assert!(
                validate_staged_attachment_file_name(name).is_err(),
                "{name:?} should be rejected"
            );
        }
        assert!(validate_staged_attachment_file_name(&"a".repeat(256)).is_err());
        assert!(validate_staged_attachment_file_name(&"é".repeat(128)).is_err());
    }

    #[test]
    fn staged_attachment_bytes_are_limited_to_50_mib() {
        assert!(validate_staged_attachment_size(MAX_STAGED_AGENT_ATTACHMENT_BYTES).is_ok());
        assert!(validate_staged_attachment_size(MAX_STAGED_AGENT_ATTACHMENT_BYTES + 1).is_err());
    }

    #[test]
    fn only_uuid_staging_parents_can_be_discarded() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join(STAGED_AGENT_ATTACHMENTS_DIR);
        let id = uuid::Uuid::new_v4();
        let parent = root.join(id.to_string());
        std::fs::create_dir_all(&parent).expect("staging parent");
        let attachment = parent.join("brief.pdf");
        std::fs::write(&attachment, b"brief").expect("staged file");

        assert_eq!(
            staged_attachment_parent_for_path(&root, &attachment),
            Some(parent.canonicalize().expect("canonical staging parent"))
        );

        let non_staged = root.join("not-a-uuid").join("brief.pdf");
        std::fs::create_dir_all(non_staged.parent().expect("parent")).expect("non-staged parent");
        std::fs::write(&non_staged, b"brief").expect("non-staged file");
        assert_eq!(staged_attachment_parent_for_path(&root, &non_staged), None);

        let external = temporary.path().join("picker.pdf");
        std::fs::write(&external, b"picker").expect("picker file");
        assert_eq!(staged_attachment_parent_for_path(&root, &external), None);
    }

    #[test]
    fn staging_prunes_only_expired_unprotected_uuid_directories() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join(STAGED_AGENT_ATTACHMENTS_DIR);
        fs::create_dir_all(&root).expect("staging root");
        let expired = root.join(uuid::Uuid::new_v4().to_string());
        let protected = root.join(uuid::Uuid::new_v4().to_string());
        let invalid = root.join("keep-this-directory");
        let noncanonical_uuid = root.join(uuid::Uuid::new_v4().simple().to_string());
        fs::create_dir_all(&expired).expect("expired staging directory");
        fs::create_dir_all(&protected).expect("protected staging directory");
        fs::create_dir_all(&invalid).expect("invalid staging directory");
        fs::create_dir_all(&noncanonical_uuid).expect("noncanonical UUID directory");
        fs::write(expired.join("brief.pdf"), b"brief").expect("expired attachment");
        let protected_attachment = protected.join("live.pdf");
        fs::write(&protected_attachment, b"live").expect("protected attachment");
        let external_attachment = temporary.path().join("external.pdf");
        fs::write(&external_attachment, b"external").expect("external attachment");
        fs::write(invalid.join("notes.txt"), b"notes").expect("invalid attachment");
        let modified = fs::symlink_metadata(&expired)
            .expect("expired metadata")
            .modified()
            .expect("modified time");

        let protected_parents = protected_staging_parents(
            &root,
            &[
                protected_attachment.to_string_lossy().into_owned(),
                external_attachment.to_string_lossy().into_owned(),
                expired
                    .join("nested")
                    .join("malformed.pdf")
                    .to_string_lossy()
                    .into_owned(),
            ],
        );
        prune_expired_staged_agent_attachments(
            &root,
            modified + STAGED_AGENT_ATTACHMENT_TTL + Duration::from_secs(1),
            &protected_parents,
        )
        .expect("prune staging");

        assert!(!expired.exists());
        assert!(protected.exists());
        assert!(invalid.exists());
        assert!(noncanonical_uuid.exists());
    }

    #[test]
    fn staging_quota_allows_a_legal_eight_file_batch_and_rejects_overflow() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join(STAGED_AGENT_ATTACHMENTS_DIR);
        fs::create_dir_all(&root).expect("staging root");
        let parent = root.join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&parent).expect("staging directory");
        let staged = fs::File::create(parent.join("existing.bin")).expect("staged file");
        staged
            .set_len(MAX_STAGED_AGENT_ATTACHMENT_TOTAL_BYTES - 8 * 50 * 1024 * 1024)
            .expect("sparse staged file");

        prepare_staged_agent_attachments_root(&root, 8 * 50 * 1024 * 1024)
            .expect("legal batch fits");
        assert!(prepare_staged_agent_attachments_root(&root, 8 * 50 * 1024 * 1024 + 1).is_err());
    }

    #[test]
    fn staging_directory_count_allows_128_total_and_rejects_more() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join(STAGED_AGENT_ATTACHMENTS_DIR);
        fs::create_dir_all(&root).expect("staging root");
        let mut canonical_directories = Vec::new();
        for _ in 0..MAX_STAGED_AGENT_ATTACHMENT_DIRECTORIES {
            let directory = root.join(uuid::Uuid::new_v4().to_string());
            fs::create_dir(&directory).expect("canonical staging directory");
            canonical_directories.push(directory);
        }
        fs::create_dir(root.join("not-a-staging-directory")).expect("invalid directory");

        assert_eq!(
            count_canonical_staging_directories(&root).expect("count staging directories"),
            MAX_STAGED_AGENT_ATTACHMENT_DIRECTORIES
        );
        assert!(prepare_staged_agent_attachments_root(&root, 0).is_err());

        fs::remove_dir(canonical_directories.pop().expect("canonical directory"))
            .expect("remove one staging directory");
        prepare_staged_agent_attachments_root(&root, 0)
            .expect("one more staging directory is legal");
    }

    #[cfg(unix)]
    #[test]
    fn staging_cleanup_does_not_follow_uuid_symlinks() {
        use std::os::unix::fs::symlink;

        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary.path().join(STAGED_AGENT_ATTACHMENTS_DIR);
        let external = temporary.path().join("outside");
        fs::create_dir_all(&root).expect("staging root");
        fs::create_dir_all(&external).expect("external directory");
        fs::write(external.join("keep.txt"), b"keep").expect("external file");
        let link = root.join(uuid::Uuid::new_v4().to_string());
        symlink(&external, &link).expect("uuid symlink");

        prune_expired_staged_agent_attachments(
            &root,
            SystemTime::now() + STAGED_AGENT_ATTACHMENT_TTL + Duration::from_secs(1),
            &HashSet::new(),
        )
        .expect("prune staging");

        assert!(link.exists());
        assert_eq!(
            fs::read(external.join("keep.txt")).expect("external file"),
            b"keep"
        );
        assert_eq!(
            directory_bytes_without_following_symlinks(&root).unwrap(),
            0
        );
        assert_eq!(count_canonical_staging_directories(&root).unwrap(), 0);
    }

    #[test]
    fn retry_uses_the_prompt_owned_by_the_selected_run() {
        let item = |id: &str, run_id: &str, content: &str, sequence: i64| AgentItemDto {
            id: id.into(),
            session_id: "session-1".into(),
            run_id: Some(run_id.into()),
            sequence,
            payload: AgentItemPayload::UserMessage(super::super::MessagePayload {
                role: "user".into(),
                content: content.into(),
                attachments: vec![],
            }),
            external_id: None,
            created_at: "2026-07-25T00:00:00Z".into(),
        };
        let items = vec![
            item("older", "run-failed", "Retry this", 0),
            item("newer", "run-later", "Do not retry this", 1),
        ];

        assert_eq!(
            retry_message(items, "run-failed").unwrap().content,
            "Retry this"
        );
    }

    #[tokio::test]
    async fn branches_inherit_the_source_data_partition() {
        use sqlx::{query::query, row::Row};
        use sqlx_sqlite::SqlitePoolOptions;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory database");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repository = AgentRepository::new(pool.clone());
        let source = repository
            .create_session_in_profile(
                "Source",
                "auto",
                AgentSafetyMode::Sandboxed,
                None,
                "private",
            )
            .await
            .expect("source");
        let branch = repository
            .create_session("Branch", "auto", AgentSafetyMode::Sandboxed, None)
            .await
            .expect("branch");
        let mut transaction = pool.begin().await.expect("transaction");
        inherit_session_profile(&mut transaction, &source.id, &branch.id, "now")
            .await
            .expect("inherit profile");
        transaction.commit().await.expect("commit");

        let profile: String = query("SELECT profile FROM session_profiles WHERE session_id = ?")
            .bind(&branch.id)
            .fetch_one(&pool)
            .await
            .expect("branch profile")
            .get("profile");
        assert_eq!(profile, "private");
        assert_eq!(
            repository
                .get_session(&branch.id)
                .await
                .unwrap()
                .safety_mode,
            AgentSafetyMode::Sandboxed
        );
    }

    #[tokio::test]
    async fn supplied_attachment_name_and_media_type_survive_workspace_copy() {
        let source_directory = tempfile::tempdir().expect("source directory");
        let workspace = tempfile::tempdir().expect("workspace");
        let source = source_directory.path().join("brief.md");
        tokio::fs::write(&source, "# Brief")
            .await
            .expect("source attachment");

        let attachments = prepare_attachments(
            &[source.to_string_lossy().into_owned()],
            &[AttachmentMetadataInput {
                name: "launch-brief.custom".to_string(),
                media_type: Some("application/x-clovy-brief".to_string()),
            }],
            workspace.path(),
        )
        .await
        .expect("prepared attachments");

        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "launch-brief.custom");
        assert_eq!(
            attachments[0].mime_type.as_deref(),
            Some("application/x-clovy-brief")
        );
        assert!(PathBuf::from(&attachments[0].path).starts_with(workspace.path()));
        assert_eq!(
            tokio::fs::read_to_string(&attachments[0].path)
                .await
                .expect("copied attachment"),
            "# Brief"
        );
        let input = cli_message_with_attachments("Summarize this.", &attachments);
        assert!(input.starts_with("[Clovy attachment manifest v1]"));
        assert!(input.contains("launch-brief.custom"));
        assert!(input.contains(&attachments[0].path));
        assert!(input.ends_with("Summarize this."));
    }

    #[test]
    fn skill_catalog_uses_frontmatter_description_instead_of_metadata_keys() {
        assert_eq!(
            skill_description(
                "---\nname: pr920-proof\ndescription: \"Readable managed skill summary.\"\n---\n\n# Proof\nBody"
            )
            .as_deref(),
            Some("Readable managed skill summary.")
        );
    }

    #[tokio::test]
    async fn obsidian_aliases_collapse_to_one_canonical_catalog_entry() {
        let root = tempfile::tempdir().expect("managed skill root");
        for (skill_id, description) in [
            ("clovy-obsidian", "Canonical instructions"),
            ("june-obsidian", "Legacy instructions"),
        ] {
            let path = skill_file(root.path(), skill_id);
            std::fs::create_dir_all(path.parent().expect("skill parent")).expect("skill directory");
            std::fs::write(
                path,
                format!("---\nname: {skill_id}\ndescription: {description}\n---\n"),
            )
            .expect("skill file");
        }
        let overrides = HashMap::from([("clovy-obsidian".to_string(), false)]);

        let catalog =
            agent_skill_catalog_from_roots(vec![(root.path().to_path_buf(), true)], &overrides)
                .await
                .expect("skill catalog");

        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0]["id"], "clovy-obsidian");
        assert_eq!(catalog[0]["name"], "clovy-obsidian");
        assert_eq!(catalog[0]["description"], "Canonical instructions");
        assert_eq!(catalog[0]["enabled"], false);
    }

    #[test]
    fn bundled_obsidian_skill_has_correct_tool_reference_and_frontmatter() {
        let skill_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("resources")
            .join("agent-skills")
            .join("clovy-obsidian")
            .join("SKILL.md");
        let content = std::fs::read_to_string(&skill_path)
            .unwrap_or_else(|_| panic!("skill file should exist at {}", skill_path.display()));
        assert!(
            content.contains("get_obsidian_vault"),
            "skill must reference the unprefixed get_obsidian_vault host tool"
        );
        assert!(
            !content.contains("june_obsidian.get_obsidian_vault"),
            "skill must not reference the retired june_obsidian MCP server prefix"
        );
        assert!(content.contains("name: clovy-obsidian"));
        assert_eq!(
            skill_description(&content).as_deref(),
            Some("Work with the Obsidian vault currently selected in Clovy.")
        );
    }

    #[test]
    fn persisted_attachment_items_expose_complete_artifact_identity() {
        let item = AgentItemDto {
            id: "message-1".into(),
            session_id: "session-1".into(),
            run_id: Some("run-1".into()),
            sequence: 1,
            payload: AgentItemPayload::UserMessage(super::super::MessagePayload {
                role: "user".into(),
                content: "Read this.".into(),
                attachments: vec![MessageAttachmentPayload {
                    id: "attachment-1".into(),
                    name: "brief.md".into(),
                    path: "/workspace/attachments/brief.md".into(),
                    mime_type: Some("text/markdown".into()),
                    size_bytes: 42,
                    available: true,
                    created_at: "2026-07-24T12:00:00Z".into(),
                }],
            }),
            external_id: Some("user:run-1".into()),
            created_at: "2026-07-24T12:00:00Z".into(),
        };

        let value = item_json_with_active_run(item, None).expect("public item");
        assert_eq!(value["attachments"][0]["sessionId"], "session-1");
        assert_eq!(value["attachments"][0]["runId"], "run-1");
        assert_eq!(value["attachments"][0]["itemId"], "message-1");
        assert_eq!(value["attachments"][0]["action"], "imported");
        assert_eq!(
            value["attachments"][0]["path"],
            "/workspace/attachments/brief.md"
        );
        assert_eq!(value["attachments"][0]["mimeType"], "text/markdown");
        assert_eq!(value["attachments"][0]["sizeBytes"], 42);
    }

    #[test]
    fn active_stream_items_keep_runtime_identity_and_streaming_status() {
        let item = AgentItemDto {
            id: "database-item-1".into(),
            session_id: "session-1".into(),
            run_id: Some("run-1".into()),
            sequence: 1,
            payload: AgentItemPayload::AssistantMessage(super::super::MessagePayload {
                role: "assistant".into(),
                content: "Everything emitted so far".into(),
                attachments: vec![],
            }),
            external_id: Some("assistant:run-1".into()),
            created_at: "2026-07-24T12:00:00Z".into(),
        };

        let active =
            item_json_with_active_run(item.clone(), Some("run-1")).expect("active public item");
        assert_eq!(active["id"], "assistant:run-1");
        assert_eq!(active["status"], "streaming");
        assert_eq!(active["text"], "Everything emitted so far");

        let completed = item_json_with_active_run(item, None).expect("completed public item");
        assert_eq!(completed["id"], "database-item-1");
        assert_eq!(completed["status"], "complete");

        let user = item_json_with_active_run(
            AgentItemDto {
                id: "user-item-1".into(),
                session_id: "session-1".into(),
                run_id: Some("run-1".into()),
                sequence: 0,
                payload: AgentItemPayload::UserMessage(super::super::MessagePayload {
                    role: "user".into(),
                    content: "Keep this complete".into(),
                    attachments: vec![],
                }),
                external_id: Some("user:run-1".into()),
                created_at: "2026-07-24T11:59:59Z".into(),
            },
            Some("run-1"),
        )
        .expect("active user item");
        assert_eq!(user["id"], "user-item-1");
        assert_eq!(user["status"], "complete");
    }

    #[test]
    fn cli_turns_get_a_manifest_without_clovy_file_tools_and_their_images() {
        let attachment = |name: &str, mime: &str, available: bool| MessageAttachmentPayload {
            id: name.into(),
            name: name.into(),
            path: format!("/workspace/attachments/{name}"),
            mime_type: Some(mime.into()),
            size_bytes: 42,
            available,
            created_at: "2026-10-15T12:00:00Z".into(),
        };
        let attachments = [
            attachment("brief.md", "text/markdown", true),
            attachment("chart.png", "image/png", true),
            attachment("gone.jpg", "image/jpeg", false),
        ];
        let message = cli_message_with_attachments("Read this.", &attachments);
        assert!(!message.contains("Clovy's file tools"), "{message}");
        assert!(message.contains("- brief.md (/workspace/attachments/brief.md)"));
        assert!(!message.contains("gone.jpg"));
        assert!(message.ends_with("Read this."));
        assert_eq!(
            cli_image_attachments(&attachments),
            [PathBuf::from("/workspace/attachments/chart.png")]
        );
        assert_eq!(cli_message_with_attachments("Hi", &[]), "Hi");
    }
}

#[cfg(test)]
mod selection41_tests {
    use super::*;
    #[tokio::test]
    async fn selection41_legacy_retry_uses_chat_and_persists_explicit_engine() {
        use crate::llm::{cli::CliKind, registry::ProviderRef};
        let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
        let mut settings = crate::providers::default_settings_for_tests();
        settings.llm_usage.chat = ProviderRef::Cli { id: CliKind::Agy };
        settings.llm_cli_default_applied = true;
        crate::providers::replace_current_settings_for_tests(settings);
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query::query(
            "CREATE TABLE agent_sessions(id TEXT PRIMARY KEY,model TEXT,workspace_path TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repository = AgentRepository::new(pool.clone());
        for (id, model, expected) in [
            ("legacy", "old-native-model", "__clovy_cli_engine__:agy"),
            (
                "explicit",
                "__clovy_cli_engine__:codex",
                "__clovy_cli_engine__:codex",
            ),
        ] {
            sqlx::query::query("INSERT INTO agent_sessions VALUES (?,?,NULL)")
                .bind(id)
                .bind(model)
                .execute(&pool)
                .await
                .unwrap();
            let session:super::super::AgentSessionDto=serde_json::from_value(json!({"id":id,"title":"Synthetic","status":"idle","model":model,
                "safetyMode":"sandboxed","workspacePath":null,"source":"user","createdAt":"now","updatedAt":"now","completedAt":null,"lastError":null})).unwrap();
            assert_eq!(
                persist_retry_session(&repository, &session, "/synthetic/workspace")
                    .await
                    .unwrap(),
                expected
            );
            let saved: String =
                sqlx::query_scalar::query_scalar("SELECT model FROM agent_sessions WHERE id=?")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(saved, expected);
        }
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
    }
}
