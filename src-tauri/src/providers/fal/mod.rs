//! Direct, user-key-only fal media. The public DTOs retain the desktop command
//! contract while transport and job state are independent of the UI.
//!
//! Keys are read from the existing credential store for each operation and
//! resumed poll; no job stores credentials and no service-key fallback exists.
mod client;
mod image_edit;
mod queue;

use crate::domain::types::AppError;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImageDto {
    pub image_base64: String,
    pub mime_type: String,
    pub model: String,
    pub provider: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoJobDto {
    pub job_id: String,
}

#[derive(Clone, Debug)]
pub struct VideoGenerateParams {
    pub prompt: String,
    pub model: String,
    pub request_id: Option<String>,
    pub duration: String,
    pub resolution: Option<String>,
    pub aspect_ratio: Option<String>,
    pub audio: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "status",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum VideoStatusDto {
    Processing {
        average_execution_ms: u64,
        execution_ms: u64,
    },
    Completed {
        path: String,
        mime_type: String,
        size_bytes: u64,
        model: String,
    },
    Failed {
        reason: String,
    },
}

static CLIENT: std::sync::OnceLock<Result<client::FalClient, AppError>> =
    std::sync::OnceLock::new();

/// Installs synthetic peers in an isolated test process before its first media
/// operation. Release builds have neither this hook nor configurable origins.
#[cfg(test)]
pub(crate) fn install_test_client(queue_origin: &str, media_origin: &str) -> Result<(), AppError> {
    CLIENT
        .set(Ok(client::FalClient::for_test(
            queue_origin,
            Some(media_origin),
            client::Limits::default(),
        )))
        .map_err(|_| {
            AppError::new(
                "fal_test_client_already_initialized",
                "Install the fake fal client before the first media operation.",
            )
        })
}

fn client() -> Result<&'static client::FalClient, AppError> {
    CLIENT
        .get_or_init(client::FalClient::new)
        .as_ref()
        .map_err(Clone::clone)
}
fn fal_key() -> Result<Option<String>, AppError> {
    crate::providers::provider_api_key(crate::providers::BuiltinProvider::Fal)
}

pub async fn generate_image(
    prompt: String,
    model: String,
    safe_mode: Option<bool>,
    request_id: Option<String>,
) -> Result<GeneratedImageDto, AppError> {
    let key = fal_key()?;
    client()?
        .generate_image(
            key.as_deref(),
            client::ImageRequest {
                prompt,
                model,
                safe_mode,
                request_id,
            },
        )
        .await
}

pub async fn edit_image(
    image: String,
    prompt: String,
    mime_type: Option<String>,
    model: Option<String>,
    safe_mode: Option<bool>,
    request_id: Option<String>,
) -> Result<GeneratedImageDto, AppError> {
    let key = fal_key()?;
    client()?
        .edit_image(
            key.as_deref(),
            client::EditRequest {
                image,
                prompt,
                mime_type,
                model,
                safe_mode,
                request_id,
            },
        )
        .await
}

pub async fn video_generate(params: VideoGenerateParams) -> Result<VideoJobDto, AppError> {
    let key = fal_key()?;
    client()?.video_generate(key.as_deref(), params).await
}

pub async fn video_status(app: &AppHandle, job_id: String) -> Result<VideoStatusDto, AppError> {
    let key = fal_key()?;
    match client()?.video_status(key.as_deref(), &job_id).await? {
        client::VideoPoll::Processing => Ok(VideoStatusDto::Processing {
            average_execution_ms: 0,
            execution_ms: 0,
        }),
        client::VideoPoll::Failed { reason } => Ok(VideoStatusDto::Failed { reason }),
        client::VideoPoll::Completed {
            bytes,
            mime_type,
            model,
        } => {
            let directory = crate::app_paths::app_data_dir(app)
                .map_err(|error| AppError::new("video_write_failed", error.to_string()))?
                .join("agent-workspaces")
                .join("generated-media")
                .join("videos");
            tokio::fs::create_dir_all(&directory)
                .await
                .map_err(write_error)?;
            let extension = if mime_type == "video/webm" {
                "webm"
            } else {
                "mp4"
            };
            let path = directory.join(video_file_name(&job_id, extension)?);
            tokio::fs::write(&path, &bytes).await.map_err(write_error)?;
            Ok(VideoStatusDto::Completed {
                path: path.to_string_lossy().into_owned(),
                mime_type,
                size_bytes: bytes.len() as u64,
                model,
            })
        }
    }
}
fn video_file_name(job_id: &str, extension: &str) -> Result<String, AppError> {
    let id = uuid::Uuid::parse_str(job_id).map_err(|_| {
        AppError::new(
            "video_job_unavailable",
            "This video is not available to continue.",
        )
    })?;
    Ok(format!("generated-video-{id}.{extension}"))
}

fn write_error(error: std::io::Error) -> AppError {
    AppError::new("video_write_failed", error.to_string())
}
fn upstream_error() -> AppError {
    AppError::new(
        "upstream_provider_failed",
        "fal could not complete the media request. Try again.",
    )
}

#[cfg(test)]
mod tests;
