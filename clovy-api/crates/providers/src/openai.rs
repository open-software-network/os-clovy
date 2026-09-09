use crate::retry::{self, UpstreamAttemptError};
use crate::transcription::TranscriptionWireResponse;
use async_trait::async_trait;
use clovy_config::UpstreamConfig;
use clovy_domain::{DomainError, Transcriber, Transcript, TranscriptionRequest};
use reqwest::multipart::{Form, Part};

pub const PROVIDER_NAME: &str = "openai";

pub struct OpenAiTranscriber {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenAiTranscriber {
    pub fn from_config(http: reqwest::Client, config: &UpstreamConfig) -> Self {
        Self {
            http,
            api_key: config.api_key.clone(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait]
impl Transcriber for OpenAiTranscriber {
    async fn transcribe(&self, request: TranscriptionRequest) -> Result<Transcript, DomainError> {
        let url = format!("{}/audio/transcriptions", self.base_url);
        // Bounded retry on transient failures (connection reset, 429, 5xx).
        // Safe to replay: the metering charge only settles after this call
        // succeeds, so a retried attempt can never double-charge.
        for attempt in 0..retry::UPSTREAM_ATTEMPTS {
            let error = match self.transcribe_once(&url, &request).await {
                Ok(transcript) => return Ok(transcript),
                Err(error) => error,
            };
            if attempt + 1 < retry::UPSTREAM_ATTEMPTS
                && let Some(delay) = error.retry_delay
            {
                tracing::warn!(
                    %url,
                    model = %request.model.0,
                    attempt,
                    "openai: transient upstream failure, retrying"
                );
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(error.error);
        }
        Err(DomainError::UpstreamProvider)
    }
}

impl OpenAiTranscriber {
    async fn transcribe_once(
        &self,
        url: &str,
        request: &TranscriptionRequest,
    ) -> Result<Transcript, UpstreamAttemptError> {
        let model_id = &request.model.0;
        // Canonical part name and mime from the container format only — the
        // user's own file name never reaches the provider.
        let audio_part = Part::bytes(request.audio.clone())
            .file_name(request.format.upstream_filename())
            .mime_str(request.format.mime())
            .map_err(|error| {
                tracing::error!(%error, %url, model = %model_id, "openai: audio mime build failed");
                UpstreamAttemptError::fatal(DomainError::UpstreamProvider)
            })?;
        let mut form = Form::new()
            .text("model", model_id.clone())
            .text("response_format", "json")
            .part("file", audio_part);
        if let Some(prompt) = request
            .context
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            form = form.text("prompt", prompt.to_string());
        }
        if let Some(language) = request
            .language
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            form = form.text("language", language.to_string());
        }
        let response = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .multipart(form)
            .send()
            .await
            .map_err(|error| {
                let retryable = retry::is_retryable_transport_error(&error);
                tracing::error!(%error, %url, model = %model_id, retryable, "openai: transport error");
                UpstreamAttemptError {
                    error: DomainError::UpstreamProvider,
                    retry_delay: retryable.then_some(retry::UPSTREAM_RETRY_BACKOFF),
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            let retry_delay = retry::response_retry_delay(status, response.headers());
            let retryable = retry::is_retryable_status(status);
            let body = response.text().await.unwrap_or_default();
            tracing::error!(%status, %url, model = %model_id, body_bytes = body.len(), retryable, retry_allowed = retry_delay.is_some(), "openai: non-success response");
            return Err(UpstreamAttemptError {
                error: DomainError::UpstreamProvider,
                retry_delay,
            });
        }
        let parsed = response
            .json::<TranscriptionWireResponse>()
            .await
            .map_err(|error| {
                tracing::error!(%error, %url, model = %model_id, "openai: response JSON parse failed");
                UpstreamAttemptError::fatal(DomainError::UpstreamProvider)
            })?;
        let text = parsed.text.trim().to_string();
        if text.is_empty() {
            // No speech detected is an input condition, not an upstream fault —
            // surface it as a 400 so the client can stay silent ("nothing
            // captured") instead of flashing a backend error.
            tracing::info!(%url, model = %model_id, "openai: no speech in audio");
            return Err(UpstreamAttemptError::fatal(DomainError::InvalidInput {
                reason: "no_speech".to_string(),
            }));
        }
        Ok(Transcript {
            text,
            language: parsed.language,
            provider: PROVIDER_NAME.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::OpenAiTranscriber;
    use crate::http;
    use clovy_config::UpstreamConfig;
    use clovy_domain::{
        AudioFormat, DomainError, ModelId, ProviderCredentials, Transcriber, TranscriptionRequest,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn transcriber(server: &MockServer) -> OpenAiTranscriber {
        OpenAiTranscriber::from_config(
            http::default_client(),
            &UpstreamConfig {
                api_key: "openai_key".to_string(),
                base_url: server.uri(),
                byok_base_url: None,
            },
        )
    }

    fn request() -> TranscriptionRequest {
        TranscriptionRequest {
            audio: b"fake wav".to_vec(),
            format: AudioFormat::Wav,
            context: None,
            language: None,
            model: ModelId("gpt-4o-mini-transcribe".to_string()),
            provider_credentials: ProviderCredentials::default(),
        }
    }

    #[tokio::test]
    async fn upstream_request_carries_only_the_canonical_part_name() {
        // The privacy guarantee for opt-in third-party ASR: the multipart
        // body names the audio "audio.wav", whatever the user's file was
        // called, and contains nothing identifying beyond the audio itself.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "Hello" })))
            .expect(1)
            .mount(&server)
            .await;

        let result = transcriber(&server).transcribe(request()).await;
        assert_eq!(result.map(|value| value.text), Ok("Hello".to_string()));

        let received = server.received_requests().await.unwrap_or_default();
        assert_eq!(received.len(), 1);
        let body = String::from_utf8_lossy(&received[0].body);
        assert!(body.contains("filename=\"audio.wav\""), "body: {body}");
        assert!(body.contains("Content-Type: audio/wav"), "body: {body}");
    }

    #[tokio::test]
    async fn retries_transient_5xx_then_succeeds() {
        // Regression: a single momentary 503 used to surface straight to the
        // user as upstream_provider_failed with no retry.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "Hello" })))
            .mount(&server)
            .await;

        let transcript = transcriber(&server).transcribe(request()).await;

        assert_eq!(transcript.map(|value| value.text), Ok("Hello".to_string()));
    }

    #[tokio::test]
    async fn does_not_retry_deterministic_client_errors() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(400))
            .expect(1)
            .mount(&server)
            .await;

        let result = transcriber(&server).transcribe(request()).await;

        assert_eq!(result, Err(DomainError::UpstreamProvider));
    }

    #[tokio::test]
    async fn malformed_success_response_maps_to_upstream_failure() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let result = transcriber(&server).transcribe(request()).await;

        assert_eq!(result, Err(DomainError::UpstreamProvider));
    }

    #[tokio::test]
    async fn empty_transcript_maps_to_no_speech_invalid_input() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/audio/transcriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "text": "   " })))
            .mount(&server)
            .await;

        let result = transcriber(&server).transcribe(request()).await;

        assert_eq!(
            result,
            Err(DomainError::InvalidInput {
                reason: "no_speech".to_string()
            })
        );
    }
}
