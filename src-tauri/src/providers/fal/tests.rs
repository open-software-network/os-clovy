//! Behavioral contract for the direct fal client. All network peers are local
//! fakes; these tests neither read credentials nor authorize provider usage.
use super::client::{EditRequest, FalClient, ImageRequest, Limits, VideoPoll};
use super::image_edit::{plan_geometry, validate_result_size};
use super::VideoGenerateParams;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const KEY: &str = "synthetic-fal-key";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nsynthetic-image";
const MP4: &[u8] = b"\0\0\0\x18ftypmp42synthetic-video";

#[derive(Clone, Debug)]
struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}
impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}
struct Reply {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    headers: Vec<(String, String)>,
    chunked: bool,
}
impl Reply {
    fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: serde_json::to_vec(&body).unwrap(),
            headers: vec![],
            chunked: false,
        }
    }
    fn bytes(body: &[u8]) -> Self {
        Self {
            status: 200,
            content_type: "application/octet-stream",
            body: body.to_vec(),
            headers: vec![],
            chunked: false,
        }
    }
}
struct Server {
    url: String,
    requests: Arc<Mutex<Vec<Request>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Server {
    async fn start(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let recorded = Arc::clone(&recorded);
                let handler = Arc::clone(&handler);
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let mut buffer = [0_u8; 4096];
                    let header_end = loop {
                        let count = socket.read(&mut buffer).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buffer[..count]);
                        if let Some(position) =
                            bytes.windows(4).position(|part| part == b"\r\n\r\n")
                        {
                            break position + 4;
                        }
                        assert!(bytes.len() < 65536);
                    };
                    let head = String::from_utf8_lossy(&bytes[..header_end]);
                    let mut lines = head.lines();
                    let mut first = lines.next().unwrap().split_whitespace();
                    let method = first.next().unwrap().to_owned();
                    let path = first.next().unwrap().to_owned();
                    let headers: Vec<_> = lines
                        .filter_map(|line| line.split_once(':'))
                        .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
                        .collect();
                    let length = headers
                        .iter()
                        .find(|(key, _)| key == "content-length")
                        .map(|(_, value)| value.parse::<usize>().unwrap())
                        .unwrap_or(0);
                    while bytes.len() < header_end + length {
                        let count = socket.read(&mut buffer).await.unwrap();
                        if count == 0 {
                            return;
                        }
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    let request = Request {
                        method,
                        path,
                        headers,
                        body: serde_json::from_slice(&bytes[header_end..header_end + length])
                            .unwrap_or(Value::Null),
                    };
                    recorded.lock().unwrap().push(request.clone());
                    let reply = handler(&request);
                    let mut head = format!(
                        "HTTP/1.1 {} Test\r\nContent-Type: {}\r\nConnection: close\r\n",
                        reply.status, reply.content_type
                    );
                    for (name, value) in reply.headers {
                        head.push_str(&format!("{name}: {value}\r\n"));
                    }
                    if reply.chunked {
                        head.push_str("Transfer-Encoding: chunked\r\n\r\n");
                    } else {
                        head.push_str(&format!("Content-Length: {}\r\n\r\n", reply.body.len()));
                    }
                    if socket.write_all(head.as_bytes()).await.is_err() {
                        return;
                    }
                    if reply.chunked {
                        let _ = socket
                            .write_all(format!("{:x}\r\n", reply.body.len()).as_bytes())
                            .await;
                    }
                    let _ = socket.write_all(&reply.body).await;
                    if reply.chunked {
                        let _ = socket.write_all(b"\r\n0\r\n\r\n").await;
                    }
                });
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }
    fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn image_request() -> ImageRequest {
    ImageRequest {
        prompt: "a blue teapot".into(),
        model: "fal-ai/flux/dev".into(),
        safe_mode: Some(true),
        request_id: Some("image-operation-1".into()),
    }
}
fn video_request() -> VideoGenerateParams {
    VideoGenerateParams {
        prompt: "a blue teapot spinning".into(),
        model: "fal-ai/wan/v2.2-a14b/text-to-video".into(),
        request_id: Some("video-operation-1".into()),
        duration: "5s".into(),
        resolution: Some("720p".into()),
        aspect_ratio: Some("16:9".into()),
        audio: Some(false),
    }
}
fn image_reply(request: &Request) -> Reply {
    if request.method == "POST" {
        Reply::json(200, json!({"request_id": "fal-image-1"}))
    } else if request.path.ends_with("/status") {
        Reply::json(200, json!({"status": "COMPLETED"}))
    } else {
        Reply::json(
            200,
            json!({"images": [{"url": format!("data:image/png;base64,{}", BASE64.encode(PNG))}], "has_nsfw_concepts": [false]}),
        )
    }
}
fn client(server: &Server) -> FalClient {
    FalClient::for_test(
        &server.url,
        None,
        Limits {
            poll_interval: Duration::from_millis(1),
            ..Limits::default()
        },
    )
}

#[tokio::test]
async fn missing_fal_key_never_contacts_any_server() {
    let queue = Server::start(image_reply).await;
    let retired = Server::start(|_| Reply::json(500, json!({}))).await;
    let client = client(&queue);
    let error = client
        .generate_image(None, image_request())
        .await
        .unwrap_err();
    assert_eq!(error.code, "fal_api_key_required");
    assert!(error.message.contains("Settings"));
    let error = client
        .video_generate(None, video_request())
        .await
        .unwrap_err();
    assert_eq!(error.code, "fal_api_key_required");
    assert!(queue.requests().is_empty());
    assert!(retired.requests().is_empty());
}

#[tokio::test]
async fn image_uses_one_submit_and_only_the_fal_credential() {
    let queue = Server::start(image_reply).await;
    let retired = Server::start(|_| Reply::json(500, json!({}))).await;
    let image = client(&queue)
        .generate_image(Some(KEY), image_request())
        .await
        .unwrap();
    assert_eq!(BASE64.decode(image.image_base64).unwrap(), PNG);
    assert_eq!(image.mime_type, "image/png");
    let requests = queue.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert_eq!(requests[0].path, "/fal-ai/flux/dev");
    assert_eq!(requests[1].path, "/fal-ai/flux/requests/fal-image-1/status");
    assert_eq!(requests[2].path, "/fal-ai/flux/requests/fal-image-1");
    assert_eq!(requests[0].body["enable_safety_checker"], true);
    assert_eq!(
        requests[0].header("x-app-fal-disable-fallback"),
        Some("true")
    );
    for request in requests {
        assert_eq!(
            request.header("authorization"),
            Some("Key synthetic-fal-key")
        );
        assert!(request
            .headers
            .iter()
            .all(|(key, _)| !key.starts_with("x-groq") && !key.starts_with("x-openrouter")));
    }
    assert!(retired.requests().is_empty());
}

#[tokio::test]
async fn same_image_request_id_replays_without_a_second_submit() {
    let queue = Server::start(image_reply).await;
    let client = client(&queue);
    let (first, second) = tokio::join!(
        client.generate_image(Some(KEY), image_request()),
        client.generate_image(Some(KEY), image_request())
    );
    assert_eq!(first.unwrap().image_base64, second.unwrap().image_base64);
    client
        .generate_image(Some(KEY), image_request())
        .await
        .unwrap();
    assert_eq!(
        queue
            .requests()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn ambiguous_submit_is_not_retried() {
    let queue = Server::start(|_| Reply::json(503, json!({"detail": "busy"}))).await;
    let client = client(&queue);
    assert!(client
        .generate_image(Some(KEY), image_request())
        .await
        .is_err());
    assert_eq!(queue.requests().len(), 1);
}

#[tokio::test]
async fn redirects_never_forward_a_submit_or_credential() {
    let target = Server::start(image_reply).await;
    let location = target.url.clone();
    let queue = Server::start(move |_| {
        let mut reply = Reply::json(307, json!({}));
        reply.headers.push(("Location".into(), location.clone()));
        reply
    })
    .await;
    assert!(client(&queue)
        .generate_image(Some(KEY), image_request())
        .await
        .is_err());
    assert_eq!(queue.requests().len(), 1);
    assert!(target.requests().is_empty());
}

#[tokio::test]
async fn downloaded_image_has_no_authorization() {
    let media = Server::start(|_| Reply::bytes(PNG)).await;
    let url = format!("{}/image.png", media.url);
    let queue = Server::start(move |request| {
        if request.method == "POST" || request.path.ends_with("/status") {
            image_reply(request)
        } else {
            Reply::json(200, json!({"images": [{"url": url}]}))
        }
    })
    .await;
    let client = FalClient::for_test(&queue.url, Some(&media.url), Limits::default());
    client
        .generate_image(Some(KEY), image_request())
        .await
        .unwrap();
    let requests = media.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].header("authorization").is_none());
}

#[tokio::test]
async fn download_outside_the_allowed_origin_is_never_requested() {
    let target = Server::start(|_| Reply::bytes(PNG)).await;
    let url = format!("{}/image.png", target.url);
    let queue = Server::start(move |request| {
        if request.method == "POST" || request.path.ends_with("/status") {
            image_reply(request)
        } else {
            Reply::json(200, json!({"images": [{"url": url}]}))
        }
    })
    .await;
    assert_eq!(
        client(&queue)
            .generate_image(Some(KEY), image_request())
            .await
            .unwrap_err()
            .code,
        "upstream_provider_failed"
    );
    assert_eq!(queue.requests().len(), 3);
    assert!(target.requests().is_empty());
}

#[tokio::test]
async fn inline_and_chunked_images_obey_the_decoded_byte_limit() {
    let queue = Server::start(image_reply).await;
    let tiny = Limits {
        image_bytes: 8,
        ..Limits::default()
    };
    assert_eq!(
        FalClient::for_test(&queue.url, None, tiny)
            .generate_image(Some(KEY), image_request())
            .await
            .unwrap_err()
            .code,
        "image_too_large"
    );
    assert_eq!(queue.requests().len(), 3);
    let media = Server::start(|_| {
        let mut reply = Reply::bytes(PNG);
        reply.chunked = true;
        reply
    })
    .await;
    let url = format!("{}/large.png", media.url);
    let queue = Server::start(move |request| {
        if request.method == "POST" || request.path.ends_with("/status") {
            image_reply(request)
        } else {
            Reply::json(200, json!({"images": [{"url": url}]}))
        }
    })
    .await;
    assert_eq!(
        FalClient::for_test(&queue.url, Some(&media.url), tiny)
            .generate_image(Some(KEY), image_request())
            .await
            .unwrap_err()
            .code,
        "image_too_large"
    );
    assert_eq!(queue.requests().len(), 3);
    assert_eq!(media.requests().len(), 1);
}

#[tokio::test]
async fn invalid_model_and_malicious_fal_request_id_never_reach_a_read_url() {
    let queue = Server::start(|_| Reply::json(200, json!({"request_id": "../other?secret"}))).await;
    let client = client(&queue);
    let mut request = image_request();
    request.model = "https://example.invalid/model".into();
    assert!(client.generate_image(Some(KEY), request).await.is_err());
    assert!(queue.requests().is_empty());
    assert!(client
        .generate_image(Some(KEY), image_request())
        .await
        .is_err());
    assert_eq!(queue.requests().len(), 1);
}

#[tokio::test]
async fn a_pending_image_reaches_its_timeout_without_resubmitting() {
    let queue = Server::start(|request| {
        if request.method == "POST" {
            image_reply(request)
        } else {
            Reply::json(200, json!({"status": "IN_QUEUE"}))
        }
    })
    .await;
    let limits = Limits {
        image_timeout: Duration::from_millis(500),
        poll_interval: Duration::from_millis(1),
        ..Limits::default()
    };
    assert!(FalClient::for_test(&queue.url, None, limits)
        .generate_image(Some(KEY), image_request())
        .await
        .is_err());
    let requests = queue.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert!(
        requests
            .iter()
            .filter(|request| request.method == "GET")
            .count()
            > 1
    );
}

#[tokio::test]
async fn video_keeps_its_job_during_queueing_and_credential_recovery() {
    let media = Server::start(|_| Reply::bytes(MP4)).await;
    let url = format!("{}/clip.mp4", media.url);
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polls);
    let queue = Server::start(move |request| {
        if request.method == "POST" {
            return Reply::json(200, json!({"request_id": "fal-video-1"}));
        }
        if request.header("authorization") == Some("Key rejected-key") {
            return Reply::json(401, json!({}));
        }
        if request.path.ends_with("/status") {
            let count = counter.fetch_add(1, Ordering::SeqCst);
            return Reply::json(
                200,
                json!({"status": if count < 2 { "IN_QUEUE" } else { "COMPLETED" }}),
            );
        }
        Reply::json(200, json!({"video": {"url": url, "file_size": MP4.len()}}))
    })
    .await;
    let client = FalClient::for_test(&queue.url, Some(&media.url), Limits::default());
    let job = client
        .video_generate(Some(KEY), video_request())
        .await
        .unwrap();
    let same = client
        .video_generate(Some(KEY), video_request())
        .await
        .unwrap();
    assert_eq!(job.job_id, same.job_id);
    assert_eq!(
        client
            .video_status(None, &job.job_id)
            .await
            .unwrap_err()
            .code,
        "fal_api_key_required"
    );
    assert_eq!(
        client
            .video_status(Some("rejected-key"), &job.job_id)
            .await
            .unwrap_err()
            .code,
        "fal_api_key_rejected"
    );
    for _ in 0..2 {
        assert!(matches!(
            client.video_status(Some(KEY), &job.job_id).await.unwrap(),
            VideoPoll::Processing
        ));
    }
    match client.video_status(Some(KEY), &job.job_id).await.unwrap() {
        VideoPoll::Completed {
            bytes,
            mime_type,
            model,
        } => {
            assert_eq!(bytes, MP4);
            assert_eq!(mime_type, "video/mp4");
            assert_eq!(model, video_request().model);
        }
        other => panic!("expected completed video, got {other:?}"),
    }
    let requests = queue.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert_eq!(requests[0].body["num_frames"], 81);
    assert!(requests
        .iter()
        .filter(|request| request.method == "GET")
        .all(|request| request.path.starts_with("/fal-ai/wan/requests/fal-video-1")));
    assert!(media.requests()[0].header("authorization").is_none());
}

#[tokio::test]
async fn video_options_are_validated_before_submission() {
    let queue = Server::start(image_reply).await;
    let client = client(&queue);
    let mut request = video_request();
    request.duration = "99s".into();
    assert!(client.video_generate(Some(KEY), request).await.is_err());
    let mut request = video_request();
    request.audio = Some(true);
    assert!(client.video_generate(Some(KEY), request).await.is_err());
    assert!(queue.requests().is_empty());
    assert_eq!(
        client
            .video_status(Some(KEY), "unknown-job")
            .await
            .unwrap_err()
            .code,
        "video_job_unavailable"
    );
}

#[tokio::test]
async fn invalid_edit_source_is_rejected_before_network() {
    let queue = Server::start(image_reply).await;
    let request = EditRequest {
        image: "not-base64".into(),
        prompt: "make it blue".into(),
        mime_type: Some("image/png".into()),
        model: None,
        safe_mode: Some(true),
        request_id: Some("edit-operation-1".into()),
    };
    assert_eq!(
        client(&queue)
            .edit_image(Some(KEY), request)
            .await
            .unwrap_err()
            .code,
        "image_edit_source_unsupported"
    );
    assert!(queue.requests().is_empty());
}

#[test]
fn edit_geometry_preserves_orientation_ratio_and_pixel_budget() {
    let landscape = plan_geometry(1536, 1024, 1).unwrap();
    assert_eq!((landscape.width, landscape.height), (1254, 836));
    let portrait = plan_geometry(1536, 1024, 6).unwrap();
    assert_eq!((portrait.width, portrait.height), (836, 1254));
    assert!(portrait.canvas);
    let extreme = plan_geometry(8000, 100, 1).unwrap();
    assert_eq!((extreme.width, extreme.height), (2048, 512));
    assert!(extreme.canvas);
    for geometry in [landscape, portrait, extreme] {
        assert!(u64::from(geometry.width) * u64::from(geometry.height) <= 1024 * 1024);
    }
}

#[test]
fn only_the_flux_edit_grid_exception_accepts_a_different_result_size() {
    let geometry = plan_geometry(1536, 1024, 1).unwrap();
    assert!(validate_result_size(&geometry, 1254, 836).is_ok());
    assert!(validate_result_size(&geometry, 1280, 864).is_ok());
    assert!(validate_result_size(&geometry, 1255, 836).is_err());
    assert!(validate_result_size(&geometry, 2048, 2048).is_err());
}

#[tokio::test]
async fn changed_image_shape_is_a_distinct_operation() {
    let queue = Server::start(image_reply).await;
    let client = client(&queue);
    client
        .generate_image(Some(KEY), image_request())
        .await
        .unwrap();
    let mut changed = image_request();
    changed.prompt = "a red teapot".into();
    client.generate_image(Some(KEY), changed).await.unwrap();
    assert_eq!(
        queue
            .requests()
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        2
    );
}

#[tokio::test]
async fn key_and_content_errors_keep_their_meaning() {
    for (status, body, expected) in [
        (401, json!({}), "fal_api_key_rejected"),
        (403, json!({}), "fal_api_key_rejected"),
        (
            422,
            json!({"detail": [{"type": "content_policy_violation"}]}),
            "image_content_violation",
        ),
    ] {
        let queue = Server::start(move |_| Reply::json(status, body.clone())).await;
        let error = client(&queue)
            .generate_image(Some(KEY), image_request())
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(queue.requests().len(), 1);
    }
}

#[tokio::test]
async fn an_inaccessible_video_can_resume_with_a_rotated_key_without_resubmit() {
    let queue = Server::start(|request| {
        if request.method == "POST" {
            Reply::json(200, json!({"request_id": "fal-video-1"}))
        } else if request.header("authorization") == Some("Key wrong-account") {
            Reply::json(404, json!({}))
        } else {
            Reply::json(200, json!({"status": "IN_PROGRESS"}))
        }
    })
    .await;
    let client = client(&queue);
    let job = client
        .video_generate(Some(KEY), video_request())
        .await
        .unwrap();
    let error = client
        .video_status(Some("wrong-account"), &job.job_id)
        .await
        .unwrap_err();
    assert_eq!(error.code, "fal_request_inaccessible");
    assert!(matches!(
        client
            .video_status(Some("rotated-same-account"), &job.job_id)
            .await
            .unwrap(),
        VideoPoll::Processing
    ));
    let requests = queue.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(
        requests[2].header("authorization"),
        Some("Key rotated-same-account")
    );
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn video_deadline_is_not_reset_by_resuming() {
    let queue = Server::start(|request| {
        if request.method == "POST" {
            Reply::json(200, json!({"request_id": "fal-video-1"}))
        } else {
            Reply::json(200, json!({"status": "IN_QUEUE"}))
        }
    })
    .await;
    let limits = Limits {
        video_timeout: Duration::from_millis(100),
        ..Limits::default()
    };
    let client = FalClient::for_test(&queue.url, None, limits);
    let job = client
        .video_generate(Some(KEY), video_request())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let error = client
        .video_status(Some(KEY), &job.job_id)
        .await
        .unwrap_err();
    assert_eq!(error.code, "video_generation_timeout");
    assert_eq!(queue.requests().len(), 1);
}

#[tokio::test]
async fn oversized_video_result_is_rejected_before_download() {
    let media = Server::start(|_| Reply::bytes(MP4)).await;
    let url = format!("{}/large.mp4", media.url);
    let queue = Server::start(move |request| {
        if request.method == "POST" {
            Reply::json(200, json!({"request_id": "fal-video-1"}))
        } else if request.path.ends_with("/status") {
            Reply::json(200, json!({"status": "COMPLETED"}))
        } else {
            Reply::json(200, json!({"video": {"url": url, "file_size": 1000}}))
        }
    })
    .await;
    let limits = Limits {
        video_bytes: 100,
        ..Limits::default()
    };
    let client = FalClient::for_test(&queue.url, Some(&media.url), limits);
    let job = client
        .video_generate(Some(KEY), video_request())
        .await
        .unwrap();
    let error = client
        .video_status(Some(KEY), &job.job_id)
        .await
        .unwrap_err();
    assert_eq!(error.code, "video_too_large");
    assert!(media.requests().is_empty());
}

#[tokio::test]
async fn transient_queue_reads_retry_the_same_job_without_resubmitting() {
    let polls = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&polls);
    let queue = Server::start(move |request| {
        if request.path.ends_with("/status") && counter.fetch_add(1, Ordering::SeqCst) == 0 {
            Reply::json(503, json!({"detail": "temporarily unavailable"}))
        } else {
            image_reply(request)
        }
    })
    .await;
    client(&queue)
        .generate_image(Some(KEY), image_request())
        .await
        .unwrap();
    let requests = queue.requests();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.method == "POST")
            .count(),
        1
    );
    assert_eq!(polls.load(Ordering::SeqCst), 2);
    assert!(requests
        .iter()
        .filter(|request| request.method == "GET")
        .all(|request| request.path.contains("/requests/fal-image-1")));
}

#[tokio::test]
async fn media_redirects_never_forward_a_request_or_key() {
    let target = Server::start(|_| Reply::bytes(PNG)).await;
    let location = format!("{}/image.png", target.url);
    let media = Server::start(move |_| {
        let mut response = Reply::json(302, json!({}));
        response.headers.push(("Location".into(), location.clone()));
        response
    })
    .await;
    let url = format!("{}/redirect", media.url);
    let queue = Server::start(move |request| {
        if request.method == "POST" || request.path.ends_with("/status") {
            image_reply(request)
        } else {
            Reply::json(200, json!({"images": [{"url": url}]}))
        }
    })
    .await;
    let error = FalClient::for_test(&queue.url, Some(&media.url), Limits::default())
        .generate_image(Some(KEY), image_request())
        .await
        .unwrap_err();
    assert_eq!(error.code, "upstream_provider_failed");
    assert_eq!(media.requests().len(), 1);
    assert!(media.requests()[0].header("authorization").is_none());
    assert!(target.requests().is_empty());
}

#[tokio::test]
async fn chunked_video_is_bounded_without_a_declared_file_size() {
    let media = Server::start(|_| {
        let mut response = Reply::bytes(MP4);
        response.chunked = true;
        response
    })
    .await;
    let url = format!("{}/clip.mp4", media.url);
    let queue = Server::start(move |request| {
        if request.method == "POST" {
            Reply::json(200, json!({"request_id":"fal-video-1"}))
        } else if request.path.ends_with("/status") {
            Reply::json(200, json!({"status":"COMPLETED"}))
        } else {
            Reply::json(200, json!({"video":{"url":url}}))
        }
    })
    .await;
    let client = FalClient::for_test(
        &queue.url,
        Some(&media.url),
        Limits {
            video_bytes: 8,
            ..Limits::default()
        },
    );
    let job = client
        .video_generate(Some(KEY), video_request())
        .await
        .unwrap();
    assert_eq!(
        client
            .video_status(Some(KEY), &job.job_id)
            .await
            .unwrap_err()
            .code,
        "video_too_large"
    );
    assert_eq!(media.requests().len(), 1);
    assert!(media.requests()[0].header("authorization").is_none());
    assert!(matches!(
        client.video_status(Some(KEY), &job.job_id).await.unwrap(),
        VideoPoll::Failed { .. }
    ));
    assert_eq!(media.requests().len(), 1);
}

#[test]
fn completed_video_file_name_is_stable_and_rejects_path_components() {
    let job = uuid::Uuid::new_v4().to_string();
    let first = super::video_file_name(&job, "mp4").unwrap();
    assert_eq!(first, super::video_file_name(&job, "mp4").unwrap());
    assert_eq!(first, format!("generated-video-{job}.mp4"));
    assert!(super::video_file_name("../outside", "mp4").is_err());
}

#[tokio::test]
async fn concurrent_images_respect_completed_cache_budget() {
    let queue = Server::start(image_reply).await;
    let budget = BASE64.encode(PNG).len() + 1;
    let client = FalClient::for_test(
        &queue.url,
        None,
        Limits {
            image_cache_bytes: budget,
            ..Limits::default()
        },
    );
    let first = image_request();
    let mut second = image_request();
    second.request_id = Some("image-operation-2".into());
    let (one, two) = tokio::join!(
        client.generate_image(Some(KEY), first.clone()),
        client.generate_image(Some(KEY), second.clone())
    );
    assert_eq!(BASE64.decode(one.unwrap().image_base64).unwrap(), PNG);
    assert_eq!(BASE64.decode(two.unwrap().image_base64).unwrap(), PNG);
    assert!(
        client.retained_image_bytes().await <= budget,
        "concurrent completions must not retain images above the cache budget"
    );
    // Replay may return a lightweight expired-result marker, never a paid resubmit.
    let _ = client.generate_image(Some(KEY), first).await;
    let _ = client.generate_image(Some(KEY), second).await;
    assert_eq!(
        queue
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        2
    );
}

#[tokio::test]
async fn rejected_media_submissions_can_retry_without_replaying_accepted_jobs() {
    for status in [401, 403, 422, 413] {
        for video in [false, true] {
            let posts = Arc::new(AtomicUsize::new(0));
            let count = Arc::clone(&posts);
            let queue = Server::start(move |request| {
                if request.method == "POST" && count.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Reply::json(status, json!({}));
                }
                image_reply(request)
            })
            .await;
            let client = client(&queue);
            if video {
                assert!(client
                    .video_generate(Some(KEY), video_request())
                    .await
                    .is_err());
                let job = client
                    .video_generate(Some("corrected-fixture-key"), video_request())
                    .await
                    .unwrap();
                assert_eq!(
                    client
                        .video_generate(Some(KEY), video_request())
                        .await
                        .unwrap()
                        .job_id,
                    job.job_id
                );
            } else {
                assert!(client
                    .generate_image(Some(KEY), image_request())
                    .await
                    .is_err());
                client
                    .generate_image(Some("corrected-fixture-key"), image_request())
                    .await
                    .unwrap();
                client
                    .generate_image(Some(KEY), image_request())
                    .await
                    .unwrap();
            }
            assert_eq!(posts.load(Ordering::SeqCst), 2);
        }
    }
}

#[tokio::test]
async fn a_key_rejected_after_image_acceptance_never_resubmits() {
    let queue = Server::start(|request| {
        if request.method == "POST" {
            image_reply(request)
        } else {
            Reply::json(401, json!({}))
        }
    })
    .await;
    let client = client(&queue);
    assert!(client
        .generate_image(Some(KEY), image_request())
        .await
        .is_err());
    assert!(client
        .generate_image(Some("corrected-fixture-key"), image_request())
        .await
        .is_err());
    assert_eq!(
        queue
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
}

#[tokio::test]
async fn rejected_images_do_not_exhaust_admission_after_correcting_the_key() {
    let queue = Server::start(|request| {
        if request.header("authorization") == Some("Key corrected-fixture-key") {
            image_reply(request)
        } else {
            Reply::json(401, json!({}))
        }
    })
    .await;
    let client = client(&queue);
    for index in 0..40 {
        let mut request = image_request();
        request.request_id = Some(format!("rejected-{index}"));
        let error = client.generate_image(Some(KEY), request).await.unwrap_err();
        assert_eq!(error.code, "fal_api_key_rejected", "attempt {index}");
    }
    let mut request = image_request();
    request.request_id = Some("rejected-39".into());
    client
        .generate_image(Some("corrected-fixture-key"), request)
        .await
        .unwrap();
    assert_eq!(
        queue
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        41
    );
}

#[tokio::test]
async fn review_delta_rejected_videos_release_admission_after_key_correction() {
    let queue = Server::start(|request| {
        if request.header("authorization") == Some("Key corrected-fixture-key") {
            image_reply(request)
        } else {
            Reply::json(401, json!({}))
        }
    })
    .await;
    let client = client(&queue);
    for index in 0..40 {
        let mut request = video_request();
        request.request_id = Some(format!("rejected-{index}"));
        let error = client.video_generate(Some(KEY), request).await.unwrap_err();
        assert_eq!(error.code, "fal_api_key_rejected", "attempt {index}");
    }
    let mut request = video_request();
    request.request_id = Some("rejected-39".into());
    client
        .video_generate(Some("corrected-fixture-key"), request)
        .await
        .unwrap();
    assert_eq!(
        queue
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        41
    );
}
