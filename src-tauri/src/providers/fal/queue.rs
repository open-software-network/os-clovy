//! Credential-bearing queue requests and credential-free, bounded media reads.
use super::client::Limits;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use reqwest::{redirect::Policy, StatusCode, Url};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone, Copy, Debug)]
pub(super) enum FalError {
    Rejected,
    ContentPolicy,
    PayloadTooLarge,
    UserKeyRejected,
    RequestInaccessible,
    NotFound,
    TooLarge,
    MediaUnavailable,
    GenerationFailed,
    Upstream { retryable: bool },
}
impl FalError {
    pub(super) fn definitive_rejection(self) -> bool {
        matches!(
            self,
            Self::Rejected
                | Self::ContentPolicy
                | Self::PayloadTooLarge
                | Self::UserKeyRejected
                | Self::RequestInaccessible
        )
    }

    pub(super) fn transient(self) -> bool {
        matches!(self, Self::Upstream { retryable: true })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ImageSize {
    pub width: u32,
    pub height: u32,
}
pub(super) struct ImageAsset {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
}
pub(super) enum QueueState {
    Pending,
    Completed { failed: bool, content_policy: bool },
}

pub(super) struct Queue {
    http: reqwest::Client,
    base: String,
    limits: Limits,
    #[cfg(test)]
    media_origin: Option<Url>,
}
impl Queue {
    pub(super) fn new(base: &str, limits: Limits) -> Result<Self, FalError> {
        let http = reqwest::Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| FalError::Upstream { retryable: false })?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_owned(),
            limits,
            #[cfg(test)]
            media_origin: None,
        })
    }

    #[cfg(test)]
    pub(super) fn with_media_origin(mut self, origin: Option<&str>) -> Self {
        self.media_origin = origin.map(|value| Url::parse(value).expect("test media origin"));
        self
    }

    /// Submit exactly once. A lost response leaves the paid outcome unknown.
    pub(super) async fn submit(
        &self,
        model: &str,
        key: &str,
        input: &Value,
        budget: Duration,
    ) -> Result<String, FalError> {
        let url = format!("{}/{model}", self.base);
        let response = self
            .http
            .post(&url)
            .header("Authorization", format!("Key {key}"))
            .header(
                "X-Fal-Request-Start-Timeout",
                budget.as_secs().max(1).to_string(),
            )
            .header("x-app-fal-disable-fallback", "true")
            .json(input)
            .send()
            .await
            .map_err(transport_error)?;
        let body = self.json(response, self.limits.queue_json_bytes).await?;
        let id = body
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if !valid_request_id(id) {
            return Err(FalError::MediaUnavailable);
        }
        Ok(id.to_owned())
    }

    fn address(&self, model: &str, id: &str, status: bool) -> Result<String, FalError> {
        if !valid_request_id(id) {
            return Err(FalError::MediaUnavailable);
        }
        let mut parts = model.split('/');
        let owner = parts.next().ok_or(FalError::MediaUnavailable)?;
        let alias = parts.next().ok_or(FalError::MediaUnavailable)?;
        let suffix = if status { "/status" } else { "" };
        Ok(format!(
            "{}/{owner}/{alias}/requests/{id}{suffix}",
            self.base
        ))
    }

    async fn get(&self, url: String, key: &str, cap: u64) -> Result<Value, FalError> {
        let response = self
            .http
            .get(url)
            .header("Authorization", format!("Key {key}"))
            .send()
            .await
            .map_err(transport_error)?;
        self.json(response, cap).await
    }

    async fn json(&self, response: reqwest::Response, cap: u64) -> Result<Value, FalError> {
        let status = response.status();
        if !status.is_success() {
            let body = read_capped(response, self.limits.queue_json_bytes).await?;
            let value: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
            return Err(classify(status, &value));
        }
        let bytes = read_capped(response, cap).await?;
        serde_json::from_slice(&bytes).map_err(|_| FalError::MediaUnavailable)
    }

    pub(super) async fn status(
        &self,
        model: &str,
        id: &str,
        key: &str,
    ) -> Result<QueueState, FalError> {
        let value = self
            .get(
                self.address(model, id, true)?,
                key,
                self.limits.queue_json_bytes,
            )
            .await?;
        match value.get("status").and_then(Value::as_str) {
            Some("IN_QUEUE" | "IN_PROGRESS") => Ok(QueueState::Pending),
            Some("COMPLETED") => Ok(QueueState::Completed {
                failed: value.get("error").is_some_and(|error| !error.is_null())
                    || value
                        .get("error_type")
                        .is_some_and(|error| !error.is_null()),
                content_policy: has_error(&value, "content_policy_violation"),
            }),
            _ => Err(FalError::MediaUnavailable),
        }
    }

    pub(super) async fn output(
        &self,
        model: &str,
        id: &str,
        key: &str,
        cap: u64,
    ) -> Result<Value, FalError> {
        self.get(self.address(model, id, false)?, key, cap).await
    }

    pub(super) async fn download(&self, raw: &str, cap: u64) -> Result<Vec<u8>, FalError> {
        let parsed = Url::parse(raw).map_err(|_| FalError::MediaUnavailable)?;
        #[cfg(test)]
        let fake_origin = self.media_origin.as_ref().is_some_and(|origin| {
            parsed.origin() == origin.origin()
                && parsed.username().is_empty()
                && parsed.password().is_none()
        });
        #[cfg(not(test))]
        let fake_origin = false;
        if !fake_origin && !is_fal_cdn_url(&parsed) {
            return Err(FalError::MediaUnavailable);
        }
        let builder = if fake_origin {
            reqwest::Client::builder()
                .redirect(Policy::none())
                .no_proxy()
        } else {
            // Resolution and address pinning preserve the desktop download
            // boundary, including rejection of private/rebound addresses.
            let url = parsed.as_str().to_owned();
            let (validated, addresses) = tokio::task::spawn_blocking(move || {
                crate::video_download_url::validate_video_download_url(&url)
            })
            .await
            .map_err(|_| FalError::Upstream { retryable: true })?
            .map_err(download_validation_error)?;
            crate::video_download_url::video_download_client_builder(&validated, &addresses)
                .map_err(|_| FalError::MediaUnavailable)?
        };
        let http = builder
            .retry(reqwest::retry::never())
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|_| FalError::Upstream { retryable: false })?;
        // No provider headers, cookies or capability URL logging.
        let response = http.get(parsed).send().await.map_err(transport_error)?;
        let status = response.status();
        if status == StatusCode::NOT_FOUND || status == StatusCode::GONE {
            return Err(FalError::NotFound);
        }
        if !status.is_success() {
            return Err(FalError::Upstream {
                retryable: status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS,
            });
        }
        read_capped(response, cap).await
    }

    pub(super) async fn image(&self, output: Value) -> Result<ImageAsset, FalError> {
        if output
            .get("has_nsfw_concepts")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Err(FalError::ContentPolicy);
        }
        let url = output
            .get("images")
            .and_then(Value::as_array)
            .and_then(|images| images.first())
            .and_then(|image| image.get("url"))
            .and_then(Value::as_str)
            .ok_or(FalError::MediaUnavailable)?;
        let bytes = if url.starts_with("data:") {
            let (header, payload) = url.split_once(',').ok_or(FalError::MediaUnavailable)?;
            if !header.ends_with(";base64") {
                return Err(FalError::MediaUnavailable);
            }
            if payload.len() as u64 > self.limits.image_bytes.div_ceil(3) * 4 {
                return Err(FalError::TooLarge);
            }
            let bytes = BASE64
                .decode(payload)
                .map_err(|_| FalError::MediaUnavailable)?;
            if bytes.len() as u64 > self.limits.image_bytes {
                return Err(FalError::TooLarge);
            }
            bytes
        } else {
            self.download(url, self.limits.image_bytes).await?
        };
        let mime_type = sniff_image(&bytes).ok_or(FalError::MediaUnavailable)?;
        Ok(ImageAsset { bytes, mime_type })
    }
}

pub(super) fn valid_request_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}
fn is_fal_cdn_url(url: &Url) -> bool {
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url
            .host_str()
            .is_some_and(|host| host == "fal.media" || host.ends_with(".fal.media"))
}
fn transport_error(error: reqwest::Error) -> FalError {
    FalError::Upstream {
        retryable: error.is_timeout()
            || error.is_connect()
            || error.is_body()
            || error.is_request(),
    }
}
fn has_error(value: &Value, expected: &str) -> bool {
    value.get("error_type").and_then(Value::as_str) == Some(expected)
        || value
            .get("detail")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|item| item.get("type").and_then(Value::as_str) == Some(expected))
            })
}
fn classify(status: StatusCode, value: &Value) -> FalError {
    if has_error(value, "content_policy_violation") {
        return FalError::ContentPolicy;
    }
    if status == StatusCode::PAYLOAD_TOO_LARGE || has_error(value, "file_too_large") {
        return FalError::PayloadTooLarge;
    }
    match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => FalError::UserKeyRejected,
        StatusCode::NOT_FOUND | StatusCode::GONE => FalError::RequestInaccessible,
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => FalError::Rejected,
        _ => FalError::Upstream {
            retryable: status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS,
        },
    }
}
async fn read_capped(mut response: reqwest::Response, cap: u64) -> Result<Vec<u8>, FalError> {
    if response.content_length().is_some_and(|length| length > cap) {
        return Err(FalError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if (bytes.len() as u64).saturating_add(chunk.len() as u64) > cap {
            return Err(FalError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.get(..4) == Some(&b"RIFF"[..]) && bytes.get(8..12) == Some(&b"WEBP"[..]) {
        Some("image/webp")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_https_fal_cdn_hosts_are_allowed() {
        for url in ["https://fal.media/image.png", "https://v3.fal.media/a.mp4"] {
            assert!(is_fal_cdn_url(&Url::parse(url).unwrap()));
        }
        for url in [
            "https://fal.media.evil.example/a",
            "https://evilfal.media/a",
            "http://fal.media/a",
            "https://user:secret@fal.media/a",
            "https://fal.media:444/a",
            "https://127.0.0.1/a",
        ] {
            assert!(!is_fal_cdn_url(&Url::parse(url).unwrap()));
        }
    }
}

fn download_validation_error(error: crate::video_download_url::DownloadUrlError) -> FalError {
    match error {
        crate::video_download_url::DownloadUrlError::Resolution => {
            FalError::Upstream { retryable: true }
        }
        crate::video_download_url::DownloadUrlError::Rejected(_) => FalError::MediaUnavailable,
    }
}

#[cfg(test)]
mod validation_tests {
    use super::*;
    #[test]
    fn dns_errors_are_recoverable_but_unsafe_destinations_are_not() {
        use crate::video_download_url::DownloadUrlError;
        assert!(download_validation_error(DownloadUrlError::Resolution).transient());
        assert!(
            !download_validation_error(DownloadUrlError::Rejected("private address".into()))
                .transient()
        );
    }
}
