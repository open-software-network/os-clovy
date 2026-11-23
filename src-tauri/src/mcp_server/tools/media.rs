//! The media group: `generate_image`, `edit_image`, and `generate_video` for
//! a CLI turn, with the same arguments, limits, and results as the native
//! agent's tools. Each call uses the user's fal key directly and
//! writes its file into `artifacts/` of the bound session's workspace and a
//! row of `agent_artifacts`, so the group needs a turn but no approval (parity
//! with the native engine; names fixed in the `clovy-mcp-server` delta of the
//! OpenSpec change `cli-substitui-agente-nativo`).
//!
//! An image comes back as MCP content blocks (`protocol::content_value`): the
//! artifact (id, path, metadata) as text, never its base64, and the image as
//! an `image` block while it fits the 3 MiB block limit.
//!
//! The helpers below the group are shared with the native tools
//! (`agent_runtime/tools.rs`) until that engine is removed: the video job
//! loop, the waiting-for-key result, and the artifact writes.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx_sqlite::SqlitePool;
use tauri::AppHandle;

use super::computer_use::MAX_IMAGE_BASE64;
use super::{ToolCtx, ToolGroup, TurnBinding};
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::domain::types::AppError;
use crate::mcp_server::protocol;
use crate::mcp_server::turns::TURN_REQUIRED;
use crate::providers::fal::{GeneratedImageDto, VideoJobDto, VideoStatusDto};
use crate::providers::{EditImageRequest, GenerateImageRequest, GenerateVideoRequest};

pub const GENERATE_IMAGE: &str = "generate_image";
pub const EDIT_IMAGE: &str = "edit_image";
pub const GENERATE_VIDEO: &str = "generate_video";
const MEDIA_TOOLS: [&str; 3] = [GENERATE_IMAGE, EDIT_IMAGE, GENERATE_VIDEO];

/// Largest source image `edit_image` sends.
const MAX_EDIT_SOURCE_BYTES: usize = 20 * 1024 * 1024;
/// Time between two polls of a video job.
pub(crate) const VIDEO_POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long one MCP `generate_video` call follows its job before answering
/// `processing` with the job id (the model continues it with `jobId`): a
/// minute under the CLI's tool timeout (spike S3), so the CLI gets this
/// answer and never its own timeout.
pub const MCP_VIDEO_WAIT: Duration = Duration::from_secs(14 * 60);
const _: () =
    assert!(MCP_VIDEO_WAIT.as_secs() < crate::chat_engine::policy::CLI_TOOL_TIMEOUT.as_secs());

/// The tools of this file as a [`ToolGroup`].
pub struct MediaTools;

impl ToolGroup for MediaTools {
    fn owns(&self, name: &str) -> bool {
        MEDIA_TOOLS.contains(&name)
    }

    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        descriptors()
    }

    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let binding = ctx.binding.ok_or_else(turn_required)?;
            let api = FalMedia { app: ctx.app };
            let polling = VideoPolling {
                interval: VIDEO_POLL_INTERVAL,
                wait: Some(MCP_VIDEO_WAIT),
            };
            call(&ctx.data.notes, binding, &api, polling, name, arguments).await
        })
    }
}

fn turn_required() -> AppError {
    AppError::new(
        TURN_REQUIRED,
        "Media tools are available only inside a Clovy chat turn.",
    )
}

/// Generates media for the bound session (it writes, so it is never
/// read-only, and it reaches a paid service).
fn media(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false,
            "openWorldHint": true,
        },
    })
}

fn descriptors() -> Vec<Value> {
    vec![
        media(
            GENERATE_IMAGE,
            "Generate an image from a text description. The file is saved in the session's artifacts and shown in the conversation.",
            json!({ "type": "object", "properties": { "prompt": { "type": "string" } }, "required": ["prompt"], "additionalProperties": false }),
        ),
        media(
            EDIT_IMAGE,
            "Edit an image file in the Clovy session workspace (PNG, JPEG, WebP, or GIF, up to 20 MB). The result is saved in the session's artifacts and shown in the conversation.",
            json!({ "type": "object", "properties": { "sourcePath": { "type": "string", "description": "Path of the image, relative to the session workspace or absolute." }, "instruction": { "type": "string" } }, "required": ["sourcePath", "instruction"], "additionalProperties": false }),
        ),
        media(
            GENERATE_VIDEO,
            "Generate a short video from a text description and show it in the conversation. Pass prompt to start a new video. If an earlier call returned status processing or waitingForProviderKey, continue that same video by calling again with only its jobId (for waitingForProviderKey, once the user has saved a valid fal key); never start a new video for it.",
            json!({ "type": "object", "properties": { "prompt": { "type": "string" }, "duration": { "type": "string" }, "aspectRatio": { "type": "string" }, "audio": { "type": "boolean" }, "jobId": { "type": "string" } }, "required": [], "additionalProperties": false }),
        ),
    ]
}

/// The media operations used by MCP tools, with an injectable test adapter.
pub(crate) trait MediaApi: Send + Sync {
    fn generate_image(
        &self,
        request: GenerateImageRequest,
    ) -> BoxFuture<'_, Result<GeneratedImageDto, AppError>>;
    fn edit_image(
        &self,
        request: EditImageRequest,
    ) -> BoxFuture<'_, Result<GeneratedImageDto, AppError>>;
    fn video_generate(
        &self,
        request: GenerateVideoRequest,
    ) -> BoxFuture<'_, Result<VideoJobDto, AppError>>;
    fn video_status(&self, job_id: String) -> BoxFuture<'_, Result<VideoStatusDto, AppError>>;
}

/// The direct fal provider commands shared with the media screens.
pub(crate) struct FalMedia<'a> {
    /// Where finished videos are written; `None` without a Tauri runtime.
    pub app: Option<&'a AppHandle>,
}

impl MediaApi for FalMedia<'_> {
    fn generate_image(
        &self,
        request: GenerateImageRequest,
    ) -> BoxFuture<'_, Result<GeneratedImageDto, AppError>> {
        Box::pin(crate::providers::generate_image(request))
    }

    fn edit_image(
        &self,
        request: EditImageRequest,
    ) -> BoxFuture<'_, Result<GeneratedImageDto, AppError>> {
        Box::pin(crate::providers::edit_image(request))
    }

    fn video_generate(
        &self,
        request: GenerateVideoRequest,
    ) -> BoxFuture<'_, Result<VideoJobDto, AppError>> {
        Box::pin(crate::providers::video_generate(request))
    }

    fn video_status(&self, job_id: String) -> BoxFuture<'_, Result<VideoStatusDto, AppError>> {
        Box::pin(async move {
            let app = self.app.ok_or_else(|| {
                AppError::new(
                    "media_unavailable",
                    "Clovy cannot follow this video right now.",
                )
            })?;
            crate::providers::video_status(
                app.clone(),
                crate::providers::VideoStatusRequest { job_id },
            )
            .await
        })
    }
}

/// One media call for the bound session. `polling` bounds how long a video
/// call follows its job.
pub(crate) async fn call(
    pool: &SqlitePool,
    binding: &TurnBinding,
    api: &dyn MediaApi,
    polling: VideoPolling,
    name: &str,
    arguments: &Value,
) -> Result<Value, AppError> {
    // A routine session runs only the tools its routine enables, as in the
    // native engine.
    if !super::routine_tool_allowed(pool, binding, name).await? {
        return Err(AppError::new(
            "routine_tool_not_enabled",
            "This tool is not enabled for the routine.",
        ));
    }
    let workspace = session_workspace(pool, &binding.session_id).await?;
    let target = ArtifactTarget {
        pool,
        workspace: &workspace,
        session_id: &binding.session_id,
        run_id: &binding.run_id,
    };
    match name {
        GENERATE_IMAGE => {
            let prompt = required_string(arguments, "prompt")?.trim();
            let generated = api
                .generate_image(GenerateImageRequest {
                    prompt: prompt.to_string(),
                    model: None,
                    request_id: None,
                    safe_mode: None,
                })
                .await?;
            mcp_image_result(&target, prompt, generated).await
        }
        EDIT_IMAGE => {
            let requested = required_string(arguments, "sourcePath")?;
            let source = resolve_requested_path(&workspace, requested, true)?;
            if binding.safety_mode == AgentSafetyMode::Sandboxed
                && !source.starts_with(workspace.canonicalize().map_err(io_error)?)
            {
                return Err(AppError::new(
                    "agent_path_denied",
                    "Path is outside this session's workspace.",
                ));
            }
            let instruction = required_string(arguments, "instruction")?.trim();
            let (image, mime_type) = read_edit_source(&source).await?;
            let generated = api
                .edit_image(EditImageRequest {
                    image,
                    prompt: instruction.to_string(),
                    request_id: None,
                    mime_type: Some(mime_type.to_string()),
                    model: None,
                })
                .await?;
            mcp_image_result(&target, instruction, generated).await
        }
        GENERATE_VIDEO => {
            let call = parse_video_call(arguments, None)?;
            let prompt = call.prompt();
            let (job_id, outcome) = follow_video_call(
                call,
                |request| api.video_generate(request),
                |job_id| api.video_status(job_id),
                // The CLI's call outlives a cancelled turn (the server
                // answers in a detached task), so the run ending stops it.
                run_ended(pool, &binding.run_id, polling.interval),
                polling,
            )
            .await?;
            finish_video_call(&target, job_id, outcome, prompt, None).await
        }
        other => Err(AppError::new(
            "mcp_tool_unknown",
            format!("Unknown tool `{other}`."),
        )),
    }
}

/// Completes once the run has ended (completed, cancelled, failed, or
/// interrupted), checked every `interval`; a run that cannot be read counts
/// as ended.
async fn run_ended(pool: &SqlitePool, run_id: &str, interval: Duration) {
    let repository = AgentRepository::new(pool.clone());
    loop {
        tokio::time::sleep(interval).await;
        match repository.get_run(run_id).await {
            Ok(run)
                if !matches!(
                    run.status.as_str(),
                    "completed" | "cancelled" | "failed" | "interrupted"
                ) => {}
            _ => return,
        }
    }
}

/// The bound session's workspace (`agent_sessions.workspace_path`).
async fn session_workspace(pool: &SqlitePool, session_id: &str) -> Result<PathBuf, AppError> {
    let session = AgentRepository::new(pool.clone())
        .get_session(session_id)
        .await
        .map_err(|_| {
            AppError::new(
                "agent_session_not_found",
                "The chat session of this turn was not found.",
            )
        })?;
    session
        .workspace_path
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            AppError::new(
                "agent_workspace_unavailable",
                "This chat session has no workspace for media.",
            )
        })
}

// Shared with the native tools (`agent_runtime/tools.rs`).

/// Where a tool's artifact goes: a session workspace and its rows.
pub(crate) struct ArtifactTarget<'a> {
    pub pool: &'a SqlitePool,
    pub workspace: &'a Path,
    pub session_id: &'a str,
    pub run_id: &'a str,
}

/// One `generate_video` call: start a new video, or continue a job an
/// earlier call left processing or waiting for the user's fal key. A
/// continuation carries only the job id, so nothing in it can start another
/// generation.
pub(crate) enum VideoCall {
    Create(GenerateVideoRequest),
    Resume { job_id: String },
}

impl VideoCall {
    pub(crate) fn prompt(&self) -> Option<String> {
        match self {
            VideoCall::Create(request) => Some(request.prompt.clone()),
            VideoCall::Resume { .. } => None,
        }
    }
}

pub(crate) fn parse_video_call(
    arguments: &Value,
    call_id: Option<String>,
) -> Result<VideoCall, AppError> {
    if let Some(job_id) = optional_nonempty_string(arguments, "jobId")? {
        return Ok(VideoCall::Resume { job_id });
    }
    Ok(VideoCall::Create(GenerateVideoRequest {
        prompt: required_string(arguments, "prompt")?.trim().to_string(),
        model: None,
        request_id: call_id,
        duration: optional_nonempty_string(arguments, "duration")?,
        resolution: None,
        aspect_ratio: optional_nonempty_string(arguments, "aspectRatio")?,
        audio: arguments.get("audio").and_then(Value::as_bool),
    }))
}

/// Where following one video job ended, short of an error.
#[derive(Debug, PartialEq)]
pub(crate) enum VideoJobOutcome {
    Completed {
        path: String,
        mime_type: String,
        model: String,
    },
    /// The job stays on the server, but this desktop cannot reach it until the
    /// user saves a valid key from the fal account that started it.
    WaitingForProviderKey { reason: String },
    /// The job was still rendering when the call's wait ran out
    /// ([`VideoPolling::wait`]); the next call continues it by id.
    StillProcessing,
}

/// How a call follows its job.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VideoPolling {
    pub interval: Duration,
    /// `None` follows the job until it ends; `Some` answers
    /// `StillProcessing` after this long.
    pub wait: Option<Duration>,
}

/// Runs one call. A new video sends its single submit and follows the job it
/// gets back; a continuation follows the named job with no submit. A failed
/// submit is an error, never a job. Returns the job id with the outcome so a
/// waiting result can name it. `cancelled` ends the call with
/// `agent_tool_cancelled` when it completes.
pub(crate) async fn follow_video_call<S, SFut, P, PFut, C>(
    call: VideoCall,
    submit: S,
    mut poll: P,
    cancelled: C,
    polling: VideoPolling,
) -> Result<(String, VideoJobOutcome), AppError>
where
    S: FnOnce(GenerateVideoRequest) -> SFut,
    SFut: Future<Output = Result<VideoJobDto, AppError>>,
    P: FnMut(String) -> PFut,
    PFut: Future<Output = Result<VideoStatusDto, AppError>>,
    C: Future<Output = ()>,
{
    let job_id = match call {
        VideoCall::Create(request) => submit(request).await?.job_id,
        VideoCall::Resume { job_id } => job_id,
    };
    tokio::pin!(cancelled);
    let deadline = polling.wait.map(|wait| tokio::time::Instant::now() + wait);
    let wait_over = async {
        match deadline {
            Some(deadline) => tokio::time::sleep_until(deadline).await,
            None => std::future::pending().await,
        }
    };
    tokio::pin!(wait_over);
    loop {
        let result = tokio::select! {
            result = poll(job_id.clone()) => result,
            _ = &mut cancelled => return Err(video_cancelled()),
            _ = &mut wait_over => return Ok((job_id, VideoJobOutcome::StillProcessing)),
        };
        match result {
            Ok(VideoStatusDto::Processing { .. }) => {
                tokio::select! {
                    _ = tokio::time::sleep(polling.interval) => {}
                    _ = &mut cancelled => return Err(video_cancelled()),
                    _ = &mut wait_over => return Ok((job_id, VideoJobOutcome::StillProcessing)),
                }
            }
            Ok(VideoStatusDto::Failed { reason }) => {
                return Err(AppError::new("video_generation_failed", reason));
            }
            Ok(VideoStatusDto::Completed {
                path,
                mime_type,
                model,
                ..
            }) => {
                let outcome = VideoJobOutcome::Completed {
                    path,
                    mime_type,
                    model,
                };
                return Ok((job_id, outcome));
            }
            Err(error) if video_poll_waits_for_provider_key(&error) => {
                let outcome = VideoJobOutcome::WaitingForProviderKey { reason: error.code };
                return Ok((job_id, outcome));
            }
            Err(error) => return Err(error),
        }
    }
}

/// The tool result for a followed job: the finished video copied into the
/// session's artifacts, or a normal (non-error) result naming the job to
/// continue.
pub(crate) async fn finish_video_call(
    target: &ArtifactTarget<'_>,
    job_id: String,
    outcome: VideoJobOutcome,
    prompt: Option<String>,
    request_id: Option<String>,
) -> Result<Value, AppError> {
    let (path, mime_type, model) = match outcome {
        VideoJobOutcome::Completed {
            path,
            mime_type,
            model,
        } => (path, mime_type, model),
        VideoJobOutcome::WaitingForProviderKey { reason } => {
            // A normal result, not a failure: the run goes on, and the model
            // keeps the job id to continue this same job once the key is fixed.
            let message = waiting_for_provider_key_message(&reason);
            return Ok(json!({
                "status": "waitingForProviderKey",
                "provider": crate::providers::BuiltinProvider::Fal.id(),
                "jobId": job_id,
                "reason": reason,
                "message": message,
            }));
        }
        VideoJobOutcome::StillProcessing => {
            return Ok(json!({
                "status": "processing",
                "jobId": job_id,
                "message": "The video is still rendering. Call generate_video again with only this jobId to keep waiting for it; do not start a new video.",
            }));
        }
    };
    let destination = artifact_path(
        target.workspace,
        &format!("generated-video-{}.mp4", uuid::Uuid::new_v4().simple()),
    )
    .await?;
    tokio::fs::copy(PathBuf::from(path), &destination)
        .await
        .map_err(io_error)?;
    record_artifact(target, &destination, "created", None, Some(&mime_type)).await?;
    Ok(json!({
        "mediaType": "video",
        "path": crate::filesystem::normalize_path_for_external_use(&destination),
        "mimeType": mime_type,
        "model": model,
        "prompt": prompt,
        "requestId": request_id,
        "name": destination.file_name().map(|name| name.to_string_lossy().into_owned())
    }))
}

pub(crate) fn video_cancelled() -> AppError {
    AppError::new("agent_tool_cancelled", "Video generation was cancelled.")
}

/// Poll failures that preserve the fal job until the user saves a valid key
/// from the account that started it (including a rotated key). An unreadable
/// local key also preserves the job without sending a request.
fn video_poll_waits_for_provider_key(error: &AppError) -> bool {
    matches!(
        error.code.as_str(),
        "fal_api_key_required"
            | "fal_api_key_rejected"
            | "fal_request_inaccessible"
            | "provider_api_key_unavailable"
    )
}

fn waiting_for_provider_key_message(reason: &str) -> &'static str {
    if reason == "provider_api_key_unavailable" {
        "Clovy could not read your saved fal key, so it could not check on this video. Save a valid key from the fal account that started it in Settings, then ask to continue this video."
    } else {
        "fal could not continue this video with the key Clovy has now. Save a valid key from the fal account that started it in Settings, then ask to continue this video."
    }
}

/// Writes a generated image into the session's artifacts: its path and
/// artifact id.
async fn store_generated_image(
    target: &ArtifactTarget<'_>,
    generated: &GeneratedImageDto,
) -> Result<(PathBuf, String), AppError> {
    let bytes = BASE64
        .decode(&generated.image_base64)
        .map_err(|error| AppError::new("image_response_invalid", error.to_string()))?;
    let extension = match generated.mime_type.as_str() {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    };
    let path = artifact_path(
        target.workspace,
        &format!(
            "generated-image-{}.{}",
            uuid::Uuid::new_v4().simple(),
            extension
        ),
    )
    .await?;
    tokio::fs::write(&path, &bytes).await.map_err(io_error)?;
    let artifact_id =
        record_artifact(target, &path, "created", None, Some(&generated.mime_type)).await?;
    Ok((path, artifact_id))
}

/// The MCP image result: a text block with the artifact (id, path,
/// metadata; no base64), then the image as an MCP `image` block when its
/// base64 fits the content-block limit (`MAX_IMAGE_BASE64`); a larger image
/// stays in the artifact and the text says so.
async fn mcp_image_result(
    target: &ArtifactTarget<'_>,
    prompt: &str,
    generated: GeneratedImageDto,
) -> Result<Value, AppError> {
    let (path, artifact_id) = store_generated_image(target, &generated).await?;
    let inline = generated.image_base64.len() <= MAX_IMAGE_BASE64;
    let mut text = json!({
        "mediaType": "image",
        "artifactId": artifact_id,
        "path": crate::filesystem::normalize_path_for_external_use(&path),
        "name": path.file_name().map(|name| name.to_string_lossy().into_owned()),
        "mimeType": generated.mime_type,
        "model": generated.model,
        "prompt": prompt,
        "imageAttached": inline,
    });
    if !inline {
        text["note"] = json!("The image is too large to attach; read it from path.");
    }
    let mut blocks = vec![protocol::text_block(text.to_string())];
    if inline {
        blocks.push(protocol::image_block(
            generated.image_base64,
            generated.mime_type,
        ));
    }
    Ok(protocol::content_value(blocks))
}

/// Reads a source image for `edit_image`: base64 and media type, refusing
/// files over 20 MB and anything but PNG, JPEG, WebP, or GIF.
pub(crate) async fn read_edit_source(source: &Path) -> Result<(String, &'static str), AppError> {
    let bytes = tokio::fs::read(source).await.map_err(io_error)?;
    if bytes.len() > MAX_EDIT_SOURCE_BYTES {
        return Err(AppError::new(
            "image_source_too_large",
            "The source image exceeds the 20 MB edit limit.",
        ));
    }
    let mime_type = image_mime_type(source).ok_or_else(|| {
        AppError::new(
            "image_source_invalid",
            "The source must be a PNG, JPEG, WebP, or GIF image.",
        )
    })?;
    Ok((BASE64.encode(bytes), mime_type))
}

fn image_mime_type(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("webp") => Some("image/webp"),
        Some("gif") => Some("image/gif"),
        _ => None,
    }
}

/// `artifacts/<file_name>` in the workspace (canonical), with the folder
/// created.
async fn artifact_path(workspace: &Path, file_name: &str) -> Result<PathBuf, AppError> {
    let folder = workspace.join("artifacts");
    tokio::fs::create_dir_all(&folder).await.map_err(io_error)?;
    Ok(folder.canonicalize().map_err(io_error)?.join(file_name))
}

/// Records a file a tool created as one of the session's artifacts; the
/// artifact id.
pub(crate) async fn record_artifact(
    target: &ArtifactTarget<'_>,
    path: &Path,
    action: &str,
    original: Option<&Path>,
    mime_type: Option<&str>,
) -> Result<String, AppError> {
    let metadata = tokio::fs::metadata(path).await.map_err(io_error)?;
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query::query("INSERT INTO agent_artifacts(id, session_id, run_id, provenance, action, path, original_path, mime_type, size_bytes, available, created_at) VALUES (?, ?, ?, 'tool', ?, ?, ?, ?, ?, 1, ?)")
        .bind(&id).bind(target.session_id).bind(target.run_id)
        .bind(action).bind(path.to_string_lossy().as_ref()).bind(original.map(|value| value.to_string_lossy().into_owned()))
        .bind(mime_type).bind(metadata.len() as i64).bind(crate::db::repositories::timestamp()).execute(target.pool).await?;
    Ok(id)
}

/// A requested path made absolute against the workspace and canonical (a
/// path that does not exist yet resolves through its nearest existing
/// ancestor). Containment is the caller's check.
pub(crate) fn resolve_requested_path(
    workspace: &Path,
    requested: &str,
    must_exist: bool,
) -> Result<PathBuf, AppError> {
    let requested = Path::new(requested);
    let joined = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        workspace.join(requested)
    };
    if must_exist || joined.exists() {
        return joined.canonicalize().map_err(io_error);
    }
    let mut existing = joined.as_path();
    while !existing.exists() {
        existing = existing
            .parent()
            .ok_or_else(|| AppError::new("agent_path_invalid", "Path has no existing ancestor."))?;
    }
    let canonical_existing = existing.canonicalize().map_err(io_error)?;
    let suffix = joined
        .strip_prefix(existing)
        .map_err(|_| AppError::new("agent_path_invalid", "Path could not be resolved."))?;
    Ok(canonical_existing.join(suffix))
}

pub(crate) fn required_string<'a>(value: &'a Value, key: &str) -> Result<&'a str, AppError> {
    value
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

/// An optional argument that, when present, is a non-empty string.
fn optional_nonempty_string(arguments: &Value, key: &str) -> Result<Option<String>, AppError> {
    let Some(value) = arguments.get(key) else {
        return Ok(None);
    };
    let invalid = || {
        AppError::new(
            "invalid_arguments",
            format!("{key} must be a non-empty string."),
        )
    };
    let value = value.as_str().ok_or_else(invalid)?.trim();
    if value.is_empty() {
        return Err(invalid());
    }
    Ok(Some(value.to_string()))
}

pub(crate) fn io_error(error: std::io::Error) -> AppError {
    AppError::new("agent_tool_io_failed", error.to_string())
}
