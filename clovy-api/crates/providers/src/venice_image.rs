//! Venice image generation: proxies the `/image/generate` endpoint.
//!
//! Mirrors the chat/augment providers — same Venice base URL and bearer
//! credential, same bounded retry on transient upstream failures. The wire
//! shapes are confined to this file: Venice returns the generated image(s) as
//! base64 strings in an `images` array, and we surface the first one. Billing
//! lives in `ImageService`; this provider only chooses the configured key or a
//! user-supplied Venice key for the upstream call.

use crate::retry::{self, UpstreamAttemptError};
use crate::venice::{PROVIDER_NAME, user_venice_key_auth_error};
use async_trait::async_trait;
use clovy_config::UpstreamConfig;
use clovy_domain::{
    DomainError, GeneratedImage, ImageGenerationRequest, ImageGenerator, ProviderCredentials,
};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};

/// The image container we ask Venice to return. We pin `png` so the mime the
/// frontend wraps into a data URL is deterministic, rather than depending on
/// Venice's default (`webp`).
const IMAGE_FORMAT: &str = "png";
const IMAGE_MIME: &str = "image/png";

pub struct VeniceImageGenerator {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    byok_base_url: String,
    /// End-to-end budget for the whole generate leg INCLUDING retries; the
    /// hold TTL and route timeout arithmetic assume this bound (config lib.rs).
    leg_budget: std::time::Duration,
}

impl VeniceImageGenerator {
    pub fn from_config(
        http: reqwest::Client,
        config: &UpstreamConfig,
        leg_budget: std::time::Duration,
    ) -> Self {
        Self {
            http,
            api_key: config.api_key.clone(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            byok_base_url: crate::venice::byok_base_url(config),
            leg_budget,
        }
    }

    async fn generate_once(
        &self,
        url: &str,
        body: &VeniceImageRequest<'_>,
        provider_credentials: &ProviderCredentials,
    ) -> Result<VeniceImageResponse, UpstreamAttemptError> {
        let api_key = venice_api_key(&self.api_key, provider_credentials);
        let response = self
            .http
            .post(url)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await
            .map_err(|error| {
                let retryable = retry::is_retryable_transport_error(&error);
                tracing::error!(%error, %url, model = %body.model, retryable, "venice image: transport error");
                UpstreamAttemptError {
                    error: DomainError::UpstreamProvider,
                    retry_delay: retryable.then_some(retry::UPSTREAM_RETRY_BACKOFF),
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            let retry_delay = retry::response_retry_delay(status, response.headers());
            let retryable = retry::is_retryable_status(status);
            let body_text = response.text().await.unwrap_or_default();
            tracing::error!(%status, %url, model = %body.model, body_bytes = body_text.len(), retryable, retry_allowed = retry_delay.is_some(), "venice image: non-success response");
            // A 400 means Venice rejected this specific input (an unknown model,
            // an unsupported size, or a prompt its safety layer refused). Surface
            // it as a usable bad request so the client can fix it, matching the
            // augment path. Auth/billing/5xx stay provider failures.
            if status == StatusCode::BAD_REQUEST {
                return Err(UpstreamAttemptError::fatal(DomainError::InvalidInput {
                    reason: "image_generation_rejected".to_string(),
                }));
            }
            if let Some(error) = user_venice_key_auth_error(status, provider_credentials) {
                return Err(UpstreamAttemptError::fatal(error));
            }
            return Err(UpstreamAttemptError {
                error: DomainError::UpstreamProvider,
                retry_delay,
            });
        }
        response
            .json::<VeniceImageResponse>()
            .await
            .map_err(|error| {
                tracing::error!(%error, %url, model = %body.model, "venice image: response JSON parse failed");
                UpstreamAttemptError::fatal(DomainError::UpstreamProvider)
            })
    }
}

#[async_trait]
impl ImageGenerator for VeniceImageGenerator {
    async fn generate(
        &self,
        request: ImageGenerationRequest,
    ) -> Result<GeneratedImage, DomainError> {
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err(DomainError::InvalidInput {
                reason: "image_prompt_empty".to_string(),
            });
        }
        let body = VeniceImageRequest {
            model: &request.model.0,
            prompt,
            format: IMAGE_FORMAT,
            width: request.width,
            height: request.height,
            safe_mode: request.safe_mode,
        };
        let base_url = crate::venice::request_base_url(
            &self.base_url,
            &self.byok_base_url,
            &request.provider_credentials,
        );
        let url = format!("{base_url}/image/generate");
        // Bounded retry on transient failures (connection reset, 429, 5xx),
        // same as the chat and augment paths. The service settles at most one
        // Clovy charge after this call returns; a retry can cost at most one
        // extra upstream generation if a completed response was lost. The
        // whole loop runs under one end-to-end deadline: per-attempt HTTP
        // timeouts alone would let retries spend attempts x budget, breaking
        // the authorize + generate + settle <= route/hold arithmetic and
        // expiring the credit hold before settlement.
        let attempts = async {
            for attempt in 0..retry::UPSTREAM_ATTEMPTS {
                let error = match self
                    .generate_once(&url, &body, &request.provider_credentials)
                    .await
                {
                    Ok(parsed) => return image_from_response(parsed, &request.model.0),
                    Err(error) => error,
                };
                if attempt + 1 < retry::UPSTREAM_ATTEMPTS
                    && let Some(delay) = error.retry_delay
                {
                    tracing::warn!(%url, model = %request.model.0, attempt, "venice image: transient failure, retrying");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(error.error);
            }
            Err(DomainError::UpstreamProvider)
        };
        match tokio::time::timeout(self.leg_budget, attempts).await {
            Ok(result) => result,
            Err(_elapsed) => {
                tracing::error!(%url, model = %request.model.0, "venice image: generate leg budget exhausted");
                Err(DomainError::UpstreamProvider)
            }
        }
    }
}

fn venice_api_key<'a>(configured: &'a str, credentials: &'a ProviderCredentials) -> &'a str {
    credentials
        .venice_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(configured)
}

/// Maps a Venice image response to the domain type: the first non-empty base64
/// image. An empty/absent `images` array is a malformed success (or an error
/// leaked as 200) — fail closed rather than hand back a blank image.
fn image_from_response(
    response: VeniceImageResponse,
    model: &str,
) -> Result<GeneratedImage, DomainError> {
    let image_base64 = response
        .images
        .into_iter()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
        .ok_or(DomainError::UpstreamProvider)?;
    Ok(GeneratedImage {
        image_base64,
        mime_type: IMAGE_MIME.to_string(),
        model: model.to_string(),
        provider: PROVIDER_NAME.to_string(),
    })
}

#[derive(Serialize)]
struct VeniceImageRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    safe_mode: Option<bool>,
}

#[derive(Deserialize)]
struct VeniceImageResponse {
    #[serde(default)]
    images: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::VeniceImageGenerator;
    use crate::http;
    use clovy_config::UpstreamConfig;
    use clovy_domain::{
        DomainError, ImageGenerationRequest, ImageGenerator, ModelId, ProviderCredentials,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, header, method, path},
    };

    fn generator(server: &MockServer) -> VeniceImageGenerator {
        VeniceImageGenerator::from_config(
            http::default_client(),
            &UpstreamConfig {
                api_key: "venice_key".to_string(),
                base_url: server.uri(),
                byok_base_url: Some(server.uri()),
            },
            std::time::Duration::from_secs(5),
        )
    }

    fn request(prompt: &str) -> ImageGenerationRequest {
        ImageGenerationRequest {
            prompt: prompt.to_string(),
            model: ModelId("venice-sd35".to_string()),
            width: Some(1024),
            height: Some(1024),
            safe_mode: None,
            provider_credentials: ProviderCredentials::default(),
        }
    }

    #[tokio::test]
    async fn sends_prompt_and_maps_first_image() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/image/generate"))
            .and(header("authorization", "Bearer venice_key"))
            .and(body_string_contains("\"model\":\"venice-sd35\""))
            .and(body_string_contains("\"prompt\":\"a red bicycle\""))
            .and(body_string_contains("\"format\":\"png\""))
            .and(body_string_contains("\"width\":1024"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "generate-image-1",
                "images": ["aGVsbG8=", "second"],
            })))
            .mount(&server)
            .await;

        let generated = generator(&server)
            .generate(request("a red bicycle"))
            .await
            .expect("generation should succeed");

        assert_eq!(generated.image_base64, "aGVsbG8=");
        assert_eq!(generated.mime_type, "image/png");
        assert_eq!(generated.model, "venice-sd35");
        assert_eq!(generated.provider, "venice");
    }

    #[tokio::test]
    async fn image_generation_prefers_request_venice_api_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/image/generate"))
            .and(header("authorization", "Bearer user_venice_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "images": ["aGVsbG8="],
            })))
            .mount(&server)
            .await;

        let mut request = request("a red bicycle");
        request.provider_credentials = ProviderCredentials {
            venice_api_key: Some("user_venice_key".to_string()),
        };
        let generated = generator(&server)
            .generate(request)
            .await
            .expect("generation should succeed");

        assert_eq!(generated.image_base64, "aGVsbG8=");
    }

    #[tokio::test]
    async fn empty_images_array_fails_closed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/image/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "images": [] })))
            .mount(&server)
            .await;

        let error = generator(&server)
            .generate(request("anything"))
            .await
            .expect_err("an empty images array should error");

        assert_eq!(error, DomainError::UpstreamProvider);
    }

    #[tokio::test]
    async fn rejected_prompt_maps_to_invalid_input() {
        let server = MockServer::start().await;
        // Venice rejects an unknown model or a refused prompt with a 400.
        Mock::given(method("POST"))
            .and(path("/image/generate"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "model not found"
            })))
            .mount(&server)
            .await;

        let error = generator(&server)
            .generate(request("anything"))
            .await
            .expect_err("a 400 should surface as invalid input");

        assert_eq!(
            error,
            DomainError::InvalidInput {
                reason: "image_generation_rejected".to_string()
            }
        );
    }

    #[tokio::test]
    async fn retries_transient_503_then_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/image/generate"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/image/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "images": ["cmVjb3ZlcmVk"]
            })))
            .mount(&server)
            .await;

        let generated = generator(&server)
            .generate(request("anything"))
            .await
            .expect("a transient 503 should be retried then succeed");

        assert_eq!(generated.image_base64, "cmVjb3ZlcmVk");
    }

    #[tokio::test]
    async fn auth_failure_stays_upstream_not_invalid_input() {
        let server = MockServer::start().await;
        // A bad Venice credential (401) is our problem, not the user's input;
        // it must not be reported as a fixable bad request.
        Mock::given(method("POST"))
            .and(path("/image/generate"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "Invalid API key"
            })))
            .mount(&server)
            .await;

        let error = generator(&server)
            .generate(request("anything"))
            .await
            .expect_err("an auth failure should error");

        assert_eq!(error, DomainError::UpstreamProvider);
    }
}
