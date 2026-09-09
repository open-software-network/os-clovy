//! Venice image editing: proxies the `/image/edit` endpoint.
//!
//! Unlike `/image/generate` (which returns base64 strings in an `images`
//! array), the edit endpoint responds with the RAW edited image bytes, so this
//! provider reads the binary body and base64-encodes it into the shared
//! `GeneratedImage` shape. Editing uses a SEPARATE Venice model catalog from
//! generation (default `firered-image-edit`). Metering (authorize -> edit ->
//! charge) lives in the service; this provider does inference only.

use crate::retry::{self, UpstreamAttemptError};
use crate::venice::{PROVIDER_NAME, user_venice_key_auth_error};
use async_trait::async_trait;
use base64::Engine as _;
use clovy_config::UpstreamConfig;
use clovy_domain::{
    DomainError, GeneratedImage, ImageEditRequest, ImageEditor, ProviderCredentials,
};
use reqwest::StatusCode;
use serde::Serialize;

/// We pin `png` output so the mime the frontend wraps into a data URL is
/// deterministic, mirroring the generator.
const OUTPUT_FORMAT: &str = "png";
const IMAGE_MIME: &str = "image/png";

pub struct VeniceImageEditor {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    byok_base_url: String,
    /// End-to-end budget for the whole edit leg INCLUDING retries (see
    /// `VeniceImageGenerator::leg_budget`).
    leg_budget: std::time::Duration,
}

impl VeniceImageEditor {
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

    async fn edit_once(
        &self,
        url: &str,
        body: &VeniceImageEditBody<'_>,
        provider_credentials: &ProviderCredentials,
    ) -> Result<GeneratedImage, UpstreamAttemptError> {
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
                tracing::error!(%error, %url, model = %body.model, retryable, "venice image edit: transport error");
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
            tracing::error!(%status, %url, model = %body.model, body_bytes = body_text.len(), retryable, retry_allowed = retry_delay.is_some(), "venice image edit: non-success response");
            // A 400 means Venice rejected this specific input (unknown model,
            // unsupported image, or a prompt its safety layer refused). Surface a
            // usable bad request; auth/billing/5xx stay provider failures.
            if status == StatusCode::BAD_REQUEST {
                return Err(UpstreamAttemptError::fatal(DomainError::InvalidInput {
                    reason: "image_edit_rejected".to_string(),
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
        // Success is the RAW edited image bytes (not the base64 envelope the
        // generate endpoint returns), so read the body and base64-encode it into
        // the shared GeneratedImage shape.
        let bytes = response.bytes().await.map_err(|error| {
            tracing::error!(%error, %url, model = %body.model, "venice image edit: reading image bytes failed");
            UpstreamAttemptError::fatal(DomainError::UpstreamProvider)
        })?;
        if bytes.is_empty() {
            return Err(UpstreamAttemptError::fatal(DomainError::UpstreamProvider));
        }
        Ok(GeneratedImage {
            image_base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
            mime_type: IMAGE_MIME.to_string(),
            model: body.model.to_string(),
            provider: PROVIDER_NAME.to_string(),
        })
    }
}

#[async_trait]
impl ImageEditor for VeniceImageEditor {
    async fn edit(&self, request: ImageEditRequest) -> Result<GeneratedImage, DomainError> {
        let prompt = request.prompt.trim();
        if prompt.is_empty() {
            return Err(DomainError::InvalidInput {
                reason: "image_edit_prompt_empty".to_string(),
            });
        }
        let image = request.image_base64.trim();
        if image.is_empty() {
            return Err(DomainError::InvalidInput {
                reason: "image_edit_source_empty".to_string(),
            });
        }
        let body = VeniceImageEditBody {
            model: &request.model.0,
            prompt,
            image,
            output_format: OUTPUT_FORMAT,
            safe_mode: request.safe_mode,
        };
        let base_url = crate::venice::request_base_url(
            &self.base_url,
            &self.byok_base_url,
            &request.provider_credentials,
        );
        let url = format!("{base_url}/image/edit");
        // Bounded retry on transient failures, same as the generator. The
        // service charges once AFTER a successful edit, so a retry never
        // double-charges; at most one extra upstream edit if a completed
        // response was lost, bounded by UPSTREAM_ATTEMPTS.
        let attempts = async {
            for attempt in 0..retry::UPSTREAM_ATTEMPTS {
                let error = match self
                    .edit_once(&url, &body, &request.provider_credentials)
                    .await
                {
                    Ok(image) => return Ok(image),
                    Err(error) => error,
                };
                if attempt + 1 < retry::UPSTREAM_ATTEMPTS
                    && let Some(delay) = error.retry_delay
                {
                    tracing::warn!(%url, model = %request.model.0, attempt, "venice image edit: transient failure, retrying");
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
                tracing::error!(%url, model = %request.model.0, "venice image edit: edit leg budget exhausted");
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

#[derive(Serialize)]
struct VeniceImageEditBody<'a> {
    model: &'a str,
    prompt: &'a str,
    /// The source image as raw base64 (no `data:` prefix).
    image: &'a str,
    output_format: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    safe_mode: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::VeniceImageEditor;
    use crate::http;
    use base64::Engine as _;
    use clovy_config::UpstreamConfig;
    use clovy_domain::{DomainError, ImageEditRequest, ImageEditor, ModelId, ProviderCredentials};
    use pretty_assertions::assert_eq;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn editor(base_url: &str) -> VeniceImageEditor {
        VeniceImageEditor::from_config(
            http::default_client(),
            &UpstreamConfig {
                api_key: "test-key".to_string(),
                base_url: base_url.to_string(),
                byok_base_url: None,
            },
            std::time::Duration::from_secs(5),
        )
    }

    fn request() -> ImageEditRequest {
        ImageEditRequest {
            image_base64: "aGVsbG8=".to_string(),
            mime_type: "image/png".to_string(),
            prompt: "make it fluffier".to_string(),
            model: ModelId("firered-image-edit".to_string()),
            safe_mode: Some(false),
            provider_credentials: ProviderCredentials::default(),
        }
    }

    #[tokio::test]
    async fn base64_encodes_the_raw_binary_response() {
        let server = MockServer::start().await;
        // The edit endpoint returns raw image bytes, not a base64 envelope.
        Mock::given(method("POST"))
            .and(path("/image/edit"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(vec![1_u8, 2, 3, 4], "image/png"))
            .mount(&server)
            .await;

        let image = editor(&server.uri())
            .edit(request())
            .await
            .expect("edit succeeds");

        assert_eq!(
            image.image_base64,
            base64::engine::general_purpose::STANDARD.encode([1_u8, 2, 3, 4])
        );
        assert_eq!(image.mime_type, "image/png");
        assert_eq!(image.model, "firered-image-edit");
        assert_eq!(image.provider, "venice");
    }

    #[tokio::test]
    async fn maps_a_400_to_a_usable_rejection() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/image/edit"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad model"))
            .mount(&server)
            .await;

        let result = editor(&server.uri()).edit(request()).await;

        assert!(matches!(
            result,
            Err(DomainError::InvalidInput { reason }) if reason == "image_edit_rejected"
        ));
    }

    #[tokio::test]
    async fn empty_prompt_is_rejected_before_calling_venice() {
        let server = MockServer::start().await;
        let result = editor(&server.uri())
            .edit(ImageEditRequest {
                prompt: "   ".to_string(),
                ..request()
            })
            .await;

        assert!(matches!(
            result,
            Err(DomainError::InvalidInput { reason }) if reason == "image_edit_prompt_empty"
        ));
    }
}
