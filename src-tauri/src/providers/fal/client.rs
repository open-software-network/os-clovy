use super::image_edit;
use super::queue::{FalError, ImageAsset, Queue, QueueState};
use super::{GeneratedImageDto, VideoGenerateParams, VideoJobDto};
use crate::domain::types::AppError;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const IMAGE_REPLAY_TTL: Duration = Duration::from_secs(10 * 60);
const JOB_RETAIN_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_OPERATIONS: usize = 32;
const MAX_IMAGE_CACHE_BYTES: usize = 100 * 1024 * 1024;
const EDIT_MODEL: &str = "fal-ai/flux-2/edit";

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub image_bytes: u64,
    pub image_cache_bytes: usize,
    pub video_bytes: u64,
    pub queue_json_bytes: u64,
    pub image_timeout: Duration,
    pub video_timeout: Duration,
    pub poll_interval: Duration,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            image_bytes: 50 * 1024 * 1024,
            image_cache_bytes: MAX_IMAGE_CACHE_BYTES,
            video_bytes: 100 * 1024 * 1024,
            queue_json_bytes: 1024 * 1024,
            image_timeout: Duration::from_secs(120),
            video_timeout: Duration::from_secs(15 * 60),
            poll_interval: Duration::from_millis(500),
        }
    }
}
#[derive(Clone)]
pub(super) struct ImageRequest {
    pub prompt: String,
    pub model: String,
    pub safe_mode: Option<bool>,
    pub request_id: Option<String>,
}
#[derive(Clone)]
pub(super) struct EditRequest {
    pub image: String,
    pub prompt: String,
    pub mime_type: Option<String>,
    pub model: Option<String>,
    pub safe_mode: Option<bool>,
    pub request_id: Option<String>,
}
#[derive(Debug)]
pub(super) enum VideoPoll {
    Processing,
    Completed {
        bytes: Vec<u8>,
        mime_type: String,
        model: String,
    },
    Failed {
        reason: String,
    },
}
struct ImageEntry {
    created: Instant,
    result: Mutex<Option<Result<GeneratedImageDto, AppError>>>,
    complete: AtomicBool,
    bytes: AtomicUsize,
}
struct VideoEntry {
    model: String,
    provider_id: String,
    created: Instant,
    failed: Option<String>,
}
struct VideoCreate {
    created: Instant,
    result: Mutex<Option<Result<VideoJobDto, AppError>>>,
}
#[derive(Default)]
struct State {
    images: HashMap<String, Arc<ImageEntry>>,
    creates: HashMap<String, Arc<VideoCreate>>,
    videos: HashMap<String, Arc<Mutex<VideoEntry>>>,
}
pub(super) struct FalClient {
    queue: Queue,
    limits: Limits,
    state: Mutex<State>,
}
impl FalClient {
    pub(super) fn new() -> Result<Self, AppError> {
        let limits = Limits::default();
        Ok(Self {
            queue: Queue::new("https://queue.fal.run", limits)
                .map_err(|_| super::upstream_error())?,
            limits,
            state: Mutex::default(),
        })
    }
    #[cfg(test)]
    pub(super) fn for_test(queue_origin: &str, media_origin: Option<&str>, limits: Limits) -> Self {
        Self {
            queue: Queue::new(queue_origin, limits)
                .unwrap()
                .with_media_origin(media_origin),
            limits,
            state: Mutex::default(),
        }
    }

    #[cfg(test)]
    pub(super) async fn retained_image_bytes(&self) -> usize {
        self.state
            .lock()
            .await
            .images
            .values()
            .map(|entry| entry.bytes.load(Ordering::Relaxed))
            .sum()
    }

    async fn image_entry(&self, operation: String) -> Result<Arc<ImageEntry>, AppError> {
        let mut state = self.state.lock().await;
        prune_expired_images(&mut state.images);
        if let Some(entry) = state.images.get(&operation) {
            return Ok(Arc::clone(entry));
        }
        // Prune completed cached results before accepting a new operation.
        // In-flight entries cannot be evicted into a duplicate submit.
        loop {
            let bytes: usize = state
                .images
                .values()
                .map(|entry| entry.bytes.load(Ordering::Relaxed))
                .sum();
            if state.images.len() < MAX_OPERATIONS && bytes < self.limits.image_cache_bytes {
                break;
            }
            let oldest = state
                .images
                .iter()
                // A caller holding an entry may be retrying a rejected POST.
                // Evict only idle entries so that retry keeps its shared lock.
                .filter(|(_, entry)| {
                    entry.complete.load(Ordering::Acquire) && Arc::strong_count(entry) == 1
                })
                .min_by_key(|(_, entry)| entry.created)
                .map(|(key, _)| key.clone());
            if let Some(oldest) = oldest {
                state.images.remove(&oldest);
            } else {
                return Err(AppError::new(
                    "media_busy",
                    "Too many media operations are active. Wait for them to finish.",
                ));
            }
        }
        let entry = Arc::new(ImageEntry {
            created: Instant::now(),
            result: Mutex::new(None),
            complete: AtomicBool::new(false),
            bytes: AtomicUsize::new(0),
        });
        state.images.insert(operation, Arc::clone(&entry));
        Ok(entry)
    }

    pub(super) async fn generate_image(
        &self,
        key: Option<&str>,
        request: ImageRequest,
    ) -> Result<GeneratedImageDto, AppError> {
        let key = required_key(key)?;
        let prompt = nonempty(&request.prompt, "image_prompt_required")?;
        if !crate::providers::KNOWN_IMAGE_MODELS.contains(&request.model.as_str()) {
            return Err(invalid("image_model_unsupported"));
        }
        let safe = screened_safe_mode(prompt, request.safe_mode);
        let mut input =
            json!({"prompt": prompt, "num_images": 1, "output_format": "png", "sync_mode": true});
        if let Some(safe) = safe {
            input["enable_safety_checker"] = json!(safe);
        }
        let operation = operation_id(
            "image",
            request.request_id.as_deref(),
            &json!({"model": request.model, "input": input}),
        );
        let entry = self.image_entry(operation).await?;
        let mut stored = entry.result.lock().await;
        if let Some(result) = &*stored {
            return result.clone();
        }
        // Cancellation or an ambiguous send leaves this marker; another
        // invocation cannot turn the same operation into a second paid POST.
        *stored = Some(Err(ambiguous_submit()));
        let mut submission_rejected = false;
        let result = self
            .image_leg(&request.model, key, &input, &mut submission_rejected)
            .await
            .map(|asset| image_dto(asset, request.model));
        finish_image_entry(self, &entry, &mut stored, &result, submission_rejected).await;
        result
    }

    pub(super) async fn edit_image(
        &self,
        key: Option<&str>,
        request: EditRequest,
    ) -> Result<GeneratedImageDto, AppError> {
        let key = required_key(key)?;
        let prompt = nonempty(&request.prompt, "image_prompt_required")?;
        let model = request.model.as_deref().unwrap_or(EDIT_MODEL);
        if model != EDIT_MODEL {
            return Err(invalid("image_edit_model_unsupported"));
        }
        if request.mime_type.as_deref().is_some_and(|mime| {
            !matches!(
                mime,
                "image/png"
                    | "image/jpeg"
                    | "image/jpg"
                    | "image/webp"
                    | "image/gif"
                    | "image/bmp"
                    | "image/tiff"
            )
        }) {
            return Err(invalid("image_edit_source_unsupported"));
        }
        let operation = operation_id(
            "edit",
            request.request_id.as_deref(),
            &json!({"model": model, "prompt": prompt, "image": digest(request.image.as_bytes()), "mime": request.mime_type, "safe": request.safe_mode}),
        );
        let entry = self.image_entry(operation).await?;
        let mut stored = entry.result.lock().await;
        if let Some(result) = &*stored {
            return result.clone();
        }
        *stored = Some(Err(ambiguous_submit()));
        let mut submission_rejected = false;
        let result = tokio::time::timeout(self.limits.image_timeout, async {
            let plan = image_edit::prepare_request(request.image, self.limits.image_bytes).await?;
            let input = plan.input(prompt, screened_safe_mode(prompt, request.safe_mode));
            let asset = self
                .image_leg_inner(model, key, &input, &mut submission_rejected)
                .await
                .map_err(|error| image_error(error, true))?;
            let asset = plan
                .finish(asset)
                .await
                .map_err(|error| image_error(error, true))?;
            if asset.bytes.len() as u64 > self.limits.image_bytes {
                return Err(invalid("image_too_large"));
            }
            Ok(image_dto(asset, model.to_owned()))
        })
        .await
        .unwrap_or_else(|_| Err(invalid("image_generation_timeout")));
        finish_image_entry(self, &entry, &mut stored, &result, submission_rejected).await;
        result
    }

    async fn image_leg(
        &self,
        model: &str,
        key: &str,
        input: &Value,
        submission_rejected: &mut bool,
    ) -> Result<ImageAsset, AppError> {
        tokio::time::timeout(
            self.limits.image_timeout,
            self.image_leg_inner(model, key, input, submission_rejected),
        )
        .await
        .map_err(|_| invalid("image_generation_timeout"))?
        .map_err(|error| image_error(error, false))
    }
    async fn image_leg_inner(
        &self,
        model: &str,
        key: &str,
        input: &Value,
        submission_rejected: &mut bool,
    ) -> Result<ImageAsset, FalError> {
        let id = self
            .queue
            .submit(model, key, input, self.limits.image_timeout)
            .await
            .inspect_err(|error| *submission_rejected = error.definitive_rejection())?;
        loop {
            match self.queue.status(model, &id, key).await {
                Ok(QueueState::Pending) => {}
                Ok(QueueState::Completed {
                    failed,
                    content_policy,
                }) => {
                    if content_policy {
                        return Err(FalError::ContentPolicy);
                    }
                    let cap =
                        self.limits.image_bytes.div_ceil(3) * 4 + self.limits.queue_json_bytes;
                    match self.queue.output(model, &id, key, cap).await {
                        Ok(output) if !failed => return self.queue.image(output).await,
                        Ok(_) => return Err(FalError::GenerationFailed),
                        Err(error) if !failed && error.transient() => {}
                        Err(error) => {
                            return Err(if failed && error.transient() {
                                FalError::GenerationFailed
                            } else {
                                error
                            })
                        }
                    }
                }
                Err(error) if error.transient() => {}
                Err(error) => return Err(error),
            }
            tokio::time::sleep(self.limits.poll_interval).await;
        }
    }

    pub(super) async fn video_generate(
        &self,
        key: Option<&str>,
        request: VideoGenerateParams,
    ) -> Result<VideoJobDto, AppError> {
        let key = required_key(key)?;
        let input = video_input(&request)?;
        let operation = operation_id(
            "video",
            request.request_id.as_deref(),
            &json!({"model": request.model, "input": input}),
        );
        let create = {
            let mut state = self.state.lock().await;
            state.creates.retain(|_, value| {
                // Keep accepted/ambiguous submissions for replay protection.
                // A definitive rejection has no stored result and can release
                // admission only when no caller still holds its shared entry.
                Arc::strong_count(value) > 1
                    || value.result.try_lock().map_or(true, |result| {
                        value.created.elapsed() < JOB_RETAIN_TTL && result.is_some()
                    })
            });
            state.videos.retain(|_, value| {
                value
                    .try_lock()
                    .map_or(true, |job| job.created.elapsed() < JOB_RETAIN_TTL)
            });
            if let Some(existing) = state.creates.get(&operation) {
                Arc::clone(existing)
            } else {
                if state.creates.len() >= MAX_OPERATIONS || state.videos.len() >= MAX_OPERATIONS {
                    return Err(AppError::new(
                        "media_busy",
                        "Too many video jobs are retained. Wait before starting another.",
                    ));
                }
                let create = Arc::new(VideoCreate {
                    created: Instant::now(),
                    result: Mutex::new(None),
                });
                state.creates.insert(operation, Arc::clone(&create));
                create
            }
        };
        let mut stored = create.result.lock().await;
        if let Some(result) = &*stored {
            return result.clone();
        }
        *stored = Some(Err(ambiguous_submit()));
        let started = Instant::now();
        let result = tokio::time::timeout(
            self.limits.video_timeout,
            self.queue
                .submit(&request.model, key, &input, self.limits.video_timeout),
        )
        .await;
        let provider_id = match result {
            Ok(Ok(id)) => id,
            Ok(Err(error)) => {
                let rejected = error.definitive_rejection();
                let error = video_error(error);
                *stored = if rejected {
                    None
                } else {
                    Some(Err(error.clone()))
                };
                return Err(error);
            }
            Err(_) => return Err(ambiguous_submit()),
        };
        let job = VideoJobDto {
            job_id: uuid::Uuid::new_v4().to_string(),
        };
        self.state.lock().await.videos.insert(
            job.job_id.clone(),
            Arc::new(Mutex::new(VideoEntry {
                model: request.model,
                provider_id,
                created: started,
                failed: None,
            })),
        );
        *stored = Some(Ok(job.clone()));
        Ok(job)
    }

    pub(super) async fn video_status(
        &self,
        key: Option<&str>,
        job_id: &str,
    ) -> Result<VideoPoll, AppError> {
        let key = required_key(key)?;
        let job = self
            .state
            .lock()
            .await
            .videos
            .get(job_id)
            .cloned()
            .ok_or_else(video_unavailable)?;
        let mut job = job.lock().await;
        if job.created.elapsed() >= self.limits.video_timeout {
            return Err(video_timeout());
        }
        if let Some(reason) = &job.failed {
            return Ok(VideoPoll::Failed {
                reason: reason.clone(),
            });
        }
        let remaining = self
            .limits
            .video_timeout
            .saturating_sub(job.created.elapsed());
        let result = tokio::time::timeout(remaining, self.video_poll(&job, key)).await;
        match result {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(error)) if error.transient() => Ok(VideoPoll::Processing),
            Ok(Err(FalError::NotFound)) => {
                job.failed = Some("media_expired".into());
                Err(video_unavailable())
            }
            Ok(Err(error)) => {
                let mapped = video_error(error);
                if !matches!(
                    error,
                    FalError::UserKeyRejected | FalError::RequestInaccessible
                ) {
                    job.failed = Some(mapped.code.clone());
                }
                Err(mapped)
            }
            Err(_) => Err(video_timeout()),
        }
    }
    async fn video_poll(&self, job: &VideoEntry, key: &str) -> Result<VideoPoll, FalError> {
        match self.queue.status(&job.model, &job.provider_id, key).await? {
            QueueState::Pending => Ok(VideoPoll::Processing),
            QueueState::Completed {
                failed,
                content_policy,
            } => {
                if content_policy {
                    return Err(FalError::ContentPolicy);
                }
                let result = self
                    .queue
                    .output(
                        &job.model,
                        &job.provider_id,
                        key,
                        self.limits.queue_json_bytes,
                    )
                    .await;
                if failed {
                    return Err(match result {
                        Err(FalError::ContentPolicy) => FalError::ContentPolicy,
                        _ => FalError::GenerationFailed,
                    });
                }
                let output = result?;
                let file = output.get("video").ok_or(FalError::MediaUnavailable)?;
                if file
                    .get("file_size")
                    .and_then(Value::as_u64)
                    .is_some_and(|size| size > self.limits.video_bytes)
                {
                    return Err(FalError::TooLarge);
                }
                let url = file
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or(FalError::MediaUnavailable)?;
                let bytes = self.queue.download(url, self.limits.video_bytes).await?;
                let mime_type = if bytes.get(4..8) == Some(&b"ftyp"[..]) {
                    "video/mp4"
                } else if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
                    "video/webm"
                } else {
                    return Err(FalError::MediaUnavailable);
                };
                Ok(VideoPoll::Completed {
                    bytes,
                    mime_type: mime_type.into(),
                    model: job.model.clone(),
                })
            }
        }
    }
}

fn prune_expired_images(images: &mut HashMap<String, Arc<ImageEntry>>) {
    // Expiry must preserve the same active-call guard as capacity eviction:
    // a corrected-key retry may still be using the rejected entry's mutex.
    images.retain(|_, entry| {
        Arc::strong_count(entry) > 1 || entry.created.elapsed() < IMAGE_REPLAY_TTL
    });
}

async fn finish_image_entry(
    client: &FalClient,
    entry: &ImageEntry,
    stored: &mut Option<Result<GeneratedImageDto, AppError>>,
    result: &Result<GeneratedImageDto, AppError>,
    submission_rejected: bool,
) {
    if submission_rejected {
        // A definitive POST rejection created no job. Keep the shared entry
        // and its mutex so a corrected retry remains serialized.
        *stored = None;
        entry.complete.store(true, Ordering::Release);
        return;
    }
    // Serialize completion accounting as well as admission: several in-flight
    // images can finish after each observed an empty cache. The first caller
    // still receives its image even when its replay payload cannot be retained.
    let state = client.state.lock().await;
    let retained: usize = state
        .images
        .values()
        .map(|item| item.bytes.load(Ordering::Relaxed))
        .sum();
    let bytes = result.as_ref().map_or(0, |image| image.image_base64.len());
    if retained.saturating_add(bytes) <= client.limits.image_cache_bytes {
        entry.bytes.store(bytes, Ordering::Relaxed);
        *stored = Some(result.clone());
    } else {
        // Keep the outcome marker: a replay must not start another paid POST.
        *stored = Some(Err(AppError::new(
            "image_result_not_cached",
            "The image was generated, but its result is no longer cached.",
        )));
    }
    entry.complete.store(true, Ordering::Release);
}
fn screened_safe_mode(prompt: &str, selected: Option<bool>) -> Option<bool> {
    // The local word check is advisory; an explicit user choice remains the
    // authority. No model-based classifier or second provider request runs.
    let explicit = crate::image_safety::may_request_explicit_content(prompt);
    selected.or_else(|| explicit.then_some(true))
}
fn image_dto(asset: ImageAsset, model: String) -> GeneratedImageDto {
    GeneratedImageDto {
        image_base64: BASE64.encode(asset.bytes),
        mime_type: asset.mime_type.into(),
        model,
        provider: "fal".into(),
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn operation_id(kind: &str, id: Option<&str>, shape: &Value) -> String {
    let id = id
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    format!(
        "{kind}:{}:{}",
        digest(id.as_bytes()),
        digest(shape.to_string().as_bytes())
    )
}
fn required_key(key: Option<&str>) -> Result<&str, AppError> {
    key.map(str::trim).filter(|key| !key.is_empty()).ok_or_else(|| AppError::new("fal_api_key_required", "Add your fal API key in Settings > Models > More options > Provider API keys to generate images and videos."))
}
fn nonempty<'a>(value: &'a str, code: &str) -> Result<&'a str, AppError> {
    let value = value.trim();
    if value.is_empty() {
        Err(invalid(code))
    } else {
        Ok(value)
    }
}
fn invalid(code: &str) -> AppError {
    AppError::new(code, match code {
    "fal_api_key_rejected" => "fal rejected your API key. Update it in Settings > Models > More options > Provider API keys.",
    "fal_request_inaccessible" => "This fal key cannot access the video. Save a key from the account that started it, then continue the same video.",
    "image_content_violation" | "video_content_violation" => "fal refused this request under its content policy.",
    "image_too_large" | "video_too_large" => "The generated media exceeds the supported file size.",
    "image_generation_timeout" => "Image generation timed out. The request was not submitted again.",
    _ => "fal could not process this media request. Check the selected model, source and options.",
})
}
fn ambiguous_submit() -> AppError {
    AppError::new(
        "media_submit_unknown",
        "The media request could not be confirmed. It has not been submitted again.",
    )
}
fn video_timeout() -> AppError {
    AppError::new(
        "video_generation_timeout",
        "Video generation timed out. Try again.",
    )
}
fn video_unavailable() -> AppError {
    AppError::new(
        "video_job_unavailable",
        "This video is not available to continue. Starting it again creates a new video.",
    )
}
fn image_error(error: FalError, edit: bool) -> AppError {
    match error {
        FalError::UserKeyRejected => invalid("fal_api_key_rejected"),
        FalError::ContentPolicy => invalid("image_content_violation"),
        FalError::TooLarge => invalid("image_too_large"),
        FalError::PayloadTooLarge if edit => invalid("image_edit_source_too_large"),
        FalError::Rejected | FalError::PayloadTooLarge => invalid(if edit {
            "image_edit_rejected"
        } else {
            "image_generation_rejected"
        }),
        _ => super::upstream_error(),
    }
}
fn video_error(error: FalError) -> AppError {
    invalid(match error {
        FalError::UserKeyRejected => "fal_api_key_rejected",
        FalError::RequestInaccessible => "fal_request_inaccessible",
        FalError::ContentPolicy => "video_content_violation",
        FalError::TooLarge => "video_too_large",
        FalError::PayloadTooLarge => "video_payload_too_large",
        FalError::MediaUnavailable => "video_media_unavailable",
        _ => "video_generation_failed",
    })
}
fn video_input(request: &VideoGenerateParams) -> Result<Value, AppError> {
    let prompt = nonempty(&request.prompt, "video_prompt_required")?;
    let wan3 = match request.model.as_str() {
        "fal-ai/wan/v2.2-a14b/text-to-video" => false,
        "alibaba/wan-3.0/text-to-video" => true,
        _ => return Err(invalid("video_model_unsupported")),
    };
    let raw = request.duration.trim();
    let seconds: u32 = raw
        .strip_suffix(['s', 'S'])
        .unwrap_or(raw)
        .trim()
        .parse()
        .map_err(|_| invalid("video_duration_unsupported"))?;
    if !(if wan3 { 2..=30 } else { 1..=10 }).contains(&seconds) {
        return Err(invalid("video_duration_unsupported"));
    }
    let resolution = request
        .resolution
        .as_deref()
        .unwrap_or("720p")
        .trim()
        .to_ascii_lowercase();
    if !(if wan3 {
        &["480p", "720p", "1080p"][..]
    } else {
        &["480p", "580p", "720p"][..]
    })
    .contains(&resolution.as_str())
    {
        return Err(invalid("video_resolution_unsupported"));
    }
    if !wan3 && request.audio == Some(true) {
        return Err(invalid("video_audio_unsupported"));
    }
    let mut input = json!({"prompt": prompt, "resolution": resolution});
    if let Some(ratio) = request
        .aspect_ratio
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if !(if wan3 {
            &["adaptive", "16:9", "4:3", "1:1", "3:4", "9:16"][..]
        } else {
            &["16:9", "9:16", "1:1"][..]
        })
        .contains(&ratio)
        {
            return Err(invalid("video_aspect_ratio_unsupported"));
        }
        input["aspect_ratio"] = json!(ratio);
    }
    if wan3 {
        input["duration"] = json!(seconds);
        if let Some(audio) = request.audio {
            input["audio"] = json!(audio);
        }
    } else {
        input["num_frames"] = json!(seconds * 16 + 1);
    }
    Ok(input)
}

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[tokio::test]
    async fn active_image_retry_keeps_shared_entry_past_original_expiry() {
        let retry = Arc::new(ImageEntry {
            created: Instant::now() - IMAGE_REPLAY_TTL - Duration::from_secs(1),
            result: Mutex::new(Some(Err(ambiguous_submit()))),
            complete: AtomicBool::new(true),
            bytes: AtomicUsize::new(0),
        });
        let mut images = HashMap::from([("corrected-key-retry".to_string(), Arc::clone(&retry))]);
        let active = retry.result.lock().await;
        prune_expired_images(&mut images);
        assert!(images
            .get("corrected-key-retry")
            .is_some_and(|entry| Arc::ptr_eq(entry, &retry)));
        drop(active);
        drop(retry);
        prune_expired_images(&mut images);
        assert!(
            images.is_empty(),
            "idle expired entries still release capacity"
        );
    }
}
