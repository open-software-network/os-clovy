//! The media group (parity slice 2.3): the artifact lands in the bound
//! session's workspace and in its artifact list, a video is followed within
//! the CLI's tool timeout and continued by job id, and unbound clients never
//! reach it. Media replies are faked in process for job logic, and a fake
//! HTTP queue exercises the direct fal client in an isolated child process.

use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx_sqlite::SqlitePool;

use super::media::{self, MediaApi, MediaTools, VideoPolling, GENERATE_IMAGE};
use super::{ActivityAccess, McpData, ToolCtx, ToolGroup, TurnBinding};
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::approval_broker::testing::TestBroker;
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;
use crate::providers::fal::{GeneratedImageDto, VideoJobDto, VideoStatusDto};
use crate::providers::{EditImageRequest, GenerateImageRequest, GenerateVideoRequest};

const PNG_BYTES: &[u8] = b"\x89PNG\r\n\x1a\nfake image";

/// Fake media responses: records each request, answers images with fixed bytes
/// and video polls from a script, then done (or still rendering, `stuck`).
#[derive(Default)]
struct FakeMediaApi {
    calls: Mutex<Vec<String>>,
    statuses: Mutex<VecDeque<Result<VideoStatusDto, AppError>>>,
    /// The generated image's bytes (default: a small PNG).
    image_bytes: Option<Vec<u8>>,
    /// With the script used up: still rendering (`true`) or done.
    stuck: Mutex<bool>,
    video_file: Option<std::path::PathBuf>,
}

impl FakeMediaApi {
    fn polling(video_file: &Path, statuses: Vec<Result<VideoStatusDto, AppError>>) -> Self {
        Self {
            statuses: Mutex::new(statuses.into()),
            video_file: Some(video_file.to_path_buf()),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn image(&self) -> GeneratedImageDto {
        GeneratedImageDto {
            image_base64: BASE64.encode(self.image_bytes.as_deref().unwrap_or(PNG_BYTES)),
            mime_type: "image/png".into(),
            model: "fixture-image".into(),
            provider: "fal".into(),
        }
    }

    fn completed(&self) -> VideoStatusDto {
        VideoStatusDto::Completed {
            path: self
                .video_file
                .as_ref()
                .expect("a video file")
                .to_string_lossy()
                .into_owned(),
            mime_type: "video/mp4".into(),
            size_bytes: 9,
            model: "fixture-video".into(),
        }
    }
}

fn processing() -> Result<VideoStatusDto, AppError> {
    Ok(VideoStatusDto::Processing {
        average_execution_ms: 60_000,
        execution_ms: 1_000,
    })
}

impl MediaApi for FakeMediaApi {
    fn generate_image(
        &self,
        request: GenerateImageRequest,
    ) -> BoxFuture<'_, Result<GeneratedImageDto, AppError>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("image:{}", request.prompt));
        Box::pin(async move { Ok(self.image()) })
    }

    fn edit_image(
        &self,
        request: EditImageRequest,
    ) -> BoxFuture<'_, Result<GeneratedImageDto, AppError>> {
        self.calls.lock().unwrap().push(format!(
            "edit:{}:{}:{}",
            request.prompt,
            request.mime_type.unwrap_or_default(),
            request.image
        ));
        Box::pin(async move { Ok(self.image()) })
    }

    fn video_generate(
        &self,
        request: GenerateVideoRequest,
    ) -> BoxFuture<'_, Result<VideoJobDto, AppError>> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("video:{}", request.prompt));
        Box::pin(async move {
            Ok(VideoJobDto {
                job_id: "job-1".into(),
            })
        })
    }

    fn video_status(&self, job_id: String) -> BoxFuture<'_, Result<VideoStatusDto, AppError>> {
        self.calls.lock().unwrap().push(format!("poll:{job_id}"));
        Box::pin(async move {
            let next = self.statuses.lock().unwrap().pop_front();
            match next {
                Some(status) => status,
                None if *self.stuck.lock().unwrap() => processing(),
                None => Ok(self.completed()),
            }
        })
    }
}

/// A turn whose session workspace is a fresh folder.
struct Turn {
    broker: TestBroker,
    binding: TurnBinding,
    workspace: tempfile::TempDir,
}

impl Turn {
    async fn new(safety_mode: AgentSafetyMode) -> Self {
        let broker = TestBroker::new([]).await;
        let mut binding = broker.turn(CliKind::Codex, true).await;
        binding.safety_mode = safety_mode;
        let workspace = tempfile::tempdir().unwrap();
        sqlx::query::query("UPDATE agent_sessions SET workspace_path = ? WHERE id = ?")
            .bind(workspace.path().to_string_lossy().as_ref())
            .bind(&binding.session_id)
            .execute(broker.pool())
            .await
            .unwrap();
        Self {
            broker,
            binding,
            workspace,
        }
    }

    fn pool(&self) -> &SqlitePool {
        self.broker.pool()
    }

    fn data(&self) -> McpData {
        crate::mcp_server::tests::data(self.pool().clone(), ActivityAccess::Off)
    }

    async fn call(
        &self,
        api: &FakeMediaApi,
        polling: VideoPolling,
        name: &str,
        arguments: Value,
    ) -> Result<Value, AppError> {
        media::call(self.pool(), &self.binding, api, polling, name, &arguments).await
    }

    /// The session's artifacts, as `list_agent_artifacts` reads them.
    async fn artifacts(&self) -> Vec<crate::agent_sessions::AgentArtifactDto> {
        AgentRepository::new(self.pool().clone())
            .artifacts(&self.binding.session_id)
            .await
            .unwrap()
    }
}

fn quick(wait: Duration) -> VideoPolling {
    VideoPolling {
        interval: Duration::from_millis(5),
        wait: Some(wait),
    }
}

fn names(tools: &[Value]) -> Vec<&str> {
    tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect()
}

/// The MCP content blocks the CLI receives for a tool's return value.
fn content(result: &Value) -> Vec<Value> {
    crate::mcp_server::protocol::tool_result(result)["content"]
        .as_array()
        .unwrap()
        .clone()
}

/// The JSON of an image result's text block.
fn image_text(result: &Value) -> Value {
    serde_json::from_str(content(result)[0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn media_image_over_the_block_limit_stays_in_the_artifact() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    // 2.5 MB of bytes: about 3.3 MiB of base64, over the 3 MiB limit.
    let big = vec![7u8; 2_500_000];
    let api = FakeMediaApi {
        image_bytes: Some(big.clone()),
        ..FakeMediaApi::default()
    };
    let result = turn
        .call(
            &api,
            quick(Duration::from_secs(5)),
            GENERATE_IMAGE,
            json!({ "prompt": "a big lake" }),
        )
        .await
        .unwrap();
    let content = content(&result);
    assert_eq!(content.len(), 1, "no image block over the limit");
    let text = content[0]["text"].as_str().unwrap();
    assert!(
        text.len() < 2_000,
        "no base64 in the text: {} bytes",
        text.len()
    );
    let result: Value = serde_json::from_str(text).unwrap();
    assert_eq!(result["imageAttached"], false);
    assert_eq!(
        std::fs::read(result["path"].as_str().unwrap()).unwrap(),
        big
    );
    assert_eq!(
        turn.artifacts().await[0].id,
        result["artifactId"].as_str().unwrap()
    );
}

#[tokio::test]
async fn media_generate_image_lands_in_the_session_workspace_and_artifact_list() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    let api = FakeMediaApi::default();

    let result = turn
        .call(
            &api,
            quick(Duration::from_secs(5)),
            GENERATE_IMAGE,
            json!({ "prompt": "  a calm lake " }),
        )
        .await
        .unwrap();

    assert_eq!(api.calls(), ["image:a calm lake"]);
    // What the CLI receives: the artifact as text (no base64), the image as
    // an MCP image block.
    let content = content(&result);
    assert_eq!(content.len(), 2);
    assert_eq!(
        content[1],
        json!({ "type": "image", "data": BASE64.encode(PNG_BYTES), "mimeType": "image/png" })
    );
    let text = content[0]["text"].as_str().unwrap();
    assert!(!text.contains(&BASE64.encode(PNG_BYTES)), "{text}");
    let result: Value = serde_json::from_str(text).unwrap();
    assert!(result.get("dataUrl").is_none(), "{result}");
    assert_eq!(result["imageAttached"], true);
    assert_eq!(result["mediaType"], "image");
    assert_eq!(result["prompt"], "a calm lake");
    assert_eq!(result["mimeType"], "image/png");
    let path = Path::new(result["path"].as_str().unwrap());
    let artifacts = turn
        .workspace
        .path()
        .canonicalize()
        .unwrap()
        .join("artifacts");
    assert_eq!(path.parent().unwrap(), artifacts);
    assert_eq!(std::fs::read(path).unwrap(), PNG_BYTES);
    let listed = turn.artifacts().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(Path::new(&listed[0].path), path);
    assert_eq!(
        listed[0].run_id.as_deref(),
        Some(turn.binding.run_id.as_str())
    );
    assert_eq!(listed[0].id, result["artifactId"].as_str().unwrap());
    assert_eq!(listed[0].mime_type.as_deref(), Some("image/png"));
    assert_eq!(listed[0].action, "created");
    assert_eq!(listed[0].size_bytes, Some(PNG_BYTES.len() as i64));
}

#[tokio::test]
async fn media_tools_are_bound_only_listed_without_read_only_hint_and_refused_unbound() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    let data = turn.data();
    let unbound = ToolCtx::unbound(&data);
    let unbound_names = super::descriptors(&unbound).await;
    for name in ["generate_image", "edit_image", "generate_video"] {
        assert!(!names(&unbound_names).contains(&name), "{name} unbound");
        let refused = super::call(&unbound, name, &json!({ "prompt": "x" }))
            .await
            .unwrap_err();
        assert_eq!(refused.code, "turn_required");
    }
    let bound = ToolCtx {
        binding: Some(&turn.binding),
        ..unbound
    };
    let listed = MediaTools.descriptors(&bound);
    assert_eq!(
        names(&listed),
        ["generate_image", "edit_image", "generate_video"]
    );
    for tool in &listed {
        assert_eq!(tool["annotations"]["readOnlyHint"], false, "{tool}");
        assert_eq!(tool["annotations"]["openWorldHint"], true, "{tool}");
    }
    assert!(names(&super::descriptors(&bound).await).contains(&"generate_video"));
    assert!(turn.artifacts().await.is_empty());
}

#[tokio::test]
async fn media_edit_image_reads_sources_in_the_workspace_and_only_there_when_sandboxed() {
    let outside = tempfile::tempdir().unwrap();
    let outside_source = outside.path().join("photo.png");
    std::fs::write(&outside_source, PNG_BYTES).unwrap();

    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    std::fs::write(turn.workspace.path().join("photo.png"), PNG_BYTES).unwrap();
    let api = FakeMediaApi::default();
    let polling = quick(Duration::from_secs(5));
    let edited = turn
        .call(
            &api,
            polling,
            "edit_image",
            json!({ "sourcePath": "photo.png", "instruction": "make it blue" }),
        )
        .await
        .unwrap();
    let edited = image_text(&edited);
    assert_eq!(edited["mediaType"], "image");
    assert_eq!(edited["prompt"], "make it blue");
    assert_eq!(
        api.calls(),
        [format!(
            "edit:make it blue:image/png:{}",
            BASE64.encode(PNG_BYTES)
        )]
    );
    let denied = turn
        .call(
            &api,
            polling,
            "edit_image",
            json!({ "sourcePath": outside_source.to_string_lossy(), "instruction": "blue" }),
        )
        .await
        .unwrap_err();
    assert_eq!(denied.code, "agent_path_denied");
    std::fs::write(turn.workspace.path().join("notes.txt"), "text").unwrap();
    let invalid = turn
        .call(
            &api,
            polling,
            "edit_image",
            json!({ "sourcePath": "notes.txt", "instruction": "blue" }),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid.code, "image_source_invalid");
    assert_eq!(api.calls().len(), 1, "refused sources never reach fal");
    assert_eq!(turn.artifacts().await.len(), 1);

    let unrestricted = Turn::new(AgentSafetyMode::Unrestricted).await;
    let api = FakeMediaApi::default();
    unrestricted
        .call(
            &api,
            polling,
            "edit_image",
            json!({ "sourcePath": outside_source.to_string_lossy(), "instruction": "blue" }),
        )
        .await
        .unwrap();
    assert_eq!(unrestricted.artifacts().await.len(), 1);
}

#[tokio::test]
async fn media_generate_video_polls_until_done_and_records_the_video() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    let downloads = tempfile::tempdir().unwrap();
    let video = downloads.path().join("generated-video.mp4");
    std::fs::write(&video, b"fake mp4!").unwrap();
    let api = FakeMediaApi::polling(&video, vec![processing(), processing()]);

    let result = turn
        .call(
            &api,
            quick(Duration::from_secs(5)),
            "generate_video",
            json!({ "prompt": "waves", "duration": "5", "aspectRatio": "16:9" }),
        )
        .await
        .unwrap();

    assert_eq!(
        api.calls(),
        ["video:waves", "poll:job-1", "poll:job-1", "poll:job-1"]
    );
    assert_eq!(result["mediaType"], "video");
    assert_eq!(result["mimeType"], "video/mp4");
    assert_eq!(result["prompt"], "waves");
    let path = Path::new(result["path"].as_str().unwrap());
    assert!(path.starts_with(
        turn.workspace
            .path()
            .canonicalize()
            .unwrap()
            .join("artifacts")
    ));
    assert_eq!(std::fs::read(path).unwrap(), b"fake mp4!");
    let listed = turn.artifacts().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].mime_type.as_deref(), Some("video/mp4"));
}

#[tokio::test]
async fn media_generate_video_answers_processing_before_the_cli_timeout_and_continues_by_job_id() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    let downloads = tempfile::tempdir().unwrap();
    let video = downloads.path().join("generated-video.mp4");
    std::fs::write(&video, b"fake mp4!").unwrap();
    // Still rendering when this call's wait runs out.
    let api = FakeMediaApi::polling(&video, vec![]);
    *api.stuck.lock().unwrap() = true;
    let waiting = turn
        .call(
            &api,
            quick(Duration::from_millis(60)),
            "generate_video",
            json!({ "prompt": "waves" }),
        )
        .await
        .unwrap();
    assert_eq!(waiting["status"], "processing");
    assert_eq!(waiting["jobId"], "job-1");
    assert!(turn.artifacts().await.is_empty());

    // The model continues the same job: no second submit.
    *api.stuck.lock().unwrap() = false;
    let done = turn
        .call(
            &api,
            quick(Duration::from_secs(5)),
            "generate_video",
            json!({ "jobId": "job-1" }),
        )
        .await
        .unwrap();
    assert_eq!(done["mediaType"], "video");
    assert_eq!(done["prompt"], Value::Null);
    let calls = api.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| call.starts_with("video:"))
            .count(),
        1
    );
    assert!(
        calls[1..].iter().all(|call| call == "poll:job-1"),
        "{calls:?}"
    );
    assert_eq!(turn.artifacts().await.len(), 1);

    // The production wait leaves the CLI a minute (spike S3).
    assert_eq!(
        crate::chat_engine::policy::CLI_TOOL_TIMEOUT - media::MCP_VIDEO_WAIT,
        Duration::from_secs(60)
    );
}

#[tokio::test]
async fn media_generate_video_stops_when_the_run_ends() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    let video = tempfile::NamedTempFile::new().unwrap();
    let api = FakeMediaApi::polling(video.path(), vec![]);
    *api.stuck.lock().unwrap() = true;
    let repository = AgentRepository::new(turn.pool().clone());
    let run_id = turn.binding.run_id.clone();
    let cancel = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        repository
            .update_run_status(&run_id, "cancelled", None, None, None)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(
        turn.call(
            &api,
            quick(Duration::from_secs(30)),
            "generate_video",
            json!({ "prompt": "waves" }),
        ),
        cancel
    );
    assert_eq!(result.unwrap_err().code, "agent_tool_cancelled");
    assert!(turn.artifacts().await.is_empty());
}

#[tokio::test]
async fn media_generate_video_waiting_for_the_fal_key_is_a_normal_result() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    let video = tempfile::NamedTempFile::new().unwrap();
    let api = FakeMediaApi::polling(
        video.path(),
        vec![Err(AppError::new("fal_api_key_rejected", "rejected"))],
    );
    let result = turn
        .call(
            &api,
            quick(Duration::from_secs(5)),
            "generate_video",
            json!({ "prompt": "waves" }),
        )
        .await
        .unwrap();
    assert_eq!(result["status"], "waitingForProviderKey");
    assert_eq!(result["provider"], "fal");
    assert_eq!(result["jobId"], "job-1");
    assert_eq!(result["reason"], "fal_api_key_rejected");
    assert!(turn.artifacts().await.is_empty());

    let failed = FakeMediaApi::polling(
        video.path(),
        vec![Ok(VideoStatusDto::Failed {
            reason: "content_rejected".into(),
        })],
    );
    let error = turn
        .call(
            &failed,
            quick(Duration::from_secs(5)),
            "generate_video",
            json!({ "prompt": "waves" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "video_generation_failed");
}

#[tokio::test]
async fn media_routine_session_refuses_tools_its_routine_does_not_enable() {
    let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
    let now = "2026-10-15T00:00:00Z";
    sqlx::query::query("INSERT INTO routines (id, name, prompt, schedule, created_at, updated_at, metadata_json, tool_catalog_version) VALUES ('routine-1', 'Daily', 'Summarise', 'every 1d', ?, ?, '{\"enabledToolsets\":[\"web\"]}', 1)")
        .bind(now)
        .bind(now)
        .execute(turn.pool())
        .await
        .unwrap();
    sqlx::query::query("INSERT INTO routine_runs (id, routine_id, agent_session_id, claim_token, trigger_kind, status, model, safety_mode, created_at, updated_at) VALUES ('routine-run-1', 'routine-1', ?, 'claim-1', 'schedule', 'running', 'codex', 'sandboxed', ?, ?)")
        .bind(&turn.binding.session_id)
        .bind(now)
        .bind(now)
        .execute(turn.pool())
        .await
        .unwrap();
    let api = FakeMediaApi::default();
    let error = turn
        .call(
            &api,
            quick(Duration::from_secs(5)),
            GENERATE_IMAGE,
            json!({ "prompt": "x" }),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "routine_tool_not_enabled");
    assert!(api.calls().is_empty());
    assert!(turn.artifacts().await.is_empty());
}

/// The production MCP/provider path against a fake fal queue and media host.
/// Isolated process state prevents synthetic keys from affecting other tests.
#[tokio::test]
async fn media_generate_image_reaches_fal_directly_over_http() {
    const CHILD: &str = "CLOVY_MEDIA_TOOLS_HTTP_CHILD";
    const ORIGIN: &str = "CLOVY_TEST_FAL_ORIGIN";
    if std::env::var(CHILD).is_ok() {
        let origin = std::env::var(ORIGIN).unwrap();
        crate::providers::fal::install_test_client(&origin, &origin).unwrap();
        crate::llm::secrets::store()
            .set(
                crate::providers::BuiltinProvider::Fal.secret_account(),
                "fixture-fal-key",
            )
            .unwrap();
        let mut settings = crate::providers::default_settings_for_tests();
        settings.provider_api_keys_configured.fal = true;
        crate::providers::replace_current_settings_for_tests(settings);
        let turn = Turn::new(AgentSafetyMode::Sandboxed).await;
        let data = turn.data();
        let ctx = ToolCtx {
            binding: Some(&turn.binding),
            ..ToolCtx::unbound(&data)
        };
        let result = MediaTools
            .call(&ctx, GENERATE_IMAGE, &json!({ "prompt": "a lighthouse" }))
            .await
            .unwrap();
        let result = image_text(&result);
        assert_eq!(result["model"], crate::providers::DEFAULT_IMAGE_MODEL);
        let bytes = std::fs::read(result["path"].as_str().unwrap()).unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 2));
        assert_eq!(turn.artifacts().await.len(), 1);
        return;
    }
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let media_url = format!("{origin}/asset.png");
    let mut image = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(2, 2)
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
    let png = image.into_inner();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let (head, body) = loop {
                let mut buffer = [0; 8192];
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "connection closed before request completed");
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request).into_owned();
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let length = head
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if body.len() >= length {
                        break (head.to_string(), body.to_string());
                    }
                }
            };
            observed.lock().unwrap().push((head.clone(), body.clone()));
            let path = head.split_whitespace().nth(1).unwrap();
            let (status, content_type, payload) = match path {
                "/fal-ai/flux/dev" => {
                    assert_eq!(
                        serde_json::from_str::<Value>(&body).unwrap()["prompt"],
                        "a lighthouse"
                    );
                    (
                        200,
                        "application/json",
                        json!({"request_id":"fal-image-id"})
                            .to_string()
                            .into_bytes(),
                    )
                }
                "/fal-ai/flux/requests/fal-image-id/status" => (
                    200,
                    "application/json",
                    json!({"status":"COMPLETED"}).to_string().into_bytes(),
                ),
                "/fal-ai/flux/requests/fal-image-id" => (
                    200,
                    "application/json",
                    json!({"images":[{"url":media_url}]})
                        .to_string()
                        .into_bytes(),
                ),
                "/asset.png" => (200, "image/png", png.clone()),
                _ => (404, "application/json", b"{}".to_vec()),
            };
            let response = format!("HTTP/1.1 {status} Response\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", payload.len());
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.write_all(&payload).await.unwrap();
        }
    });
    let output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "mcp_server::tools::media_tests::media_generate_image_reaches_fal_directly_over_http", "--nocapture"])
            .env(CHILD, "1")
            .env(ORIGIN, &origin)
            .output().unwrap()
    }).await.unwrap();
    server.abort();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    for (head, _) in requests.iter() {
        let lower = head.to_ascii_lowercase();
        if head.starts_with("GET /asset.png ") {
            assert!(!lower.contains("authorization:"));
        } else {
            assert!(lower.contains("authorization: key fixture-fal-key"));
        }
        assert!(!head.contains("/v1/"));
    }
}
