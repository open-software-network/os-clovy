use clovy_domain::DomainError;
use reqwest::{
    StatusCode,
    header::{HeaderMap, RETRY_AFTER},
};
use std::time::{Duration, SystemTime};

/// Total attempts per upstream call: the original request plus one retry.
/// Mirrors the bounded charge retry in `os_accounts.rs` — enough to absorb a
/// connection reset or a momentary 429/5xx without blowing the desktop
/// client's request budget.
pub(crate) const UPSTREAM_ATTEMPTS: u32 = 2;
pub(crate) const UPSTREAM_RETRY_BACKOFF: Duration = Duration::from_millis(300);
// Keep the existing retry time allowance: these calls may run under a short
// wallet hold. A longer upstream cooldown means no retry, never an early retry
// or a longer wait that could outlive the reservation.
const MAX_RETRY_AFTER: Duration = UPSTREAM_RETRY_BACKOFF;

/// A single failed upstream attempt, classified so the caller knows whether
/// another attempt is worth making.
pub(crate) struct UpstreamAttemptError {
    pub(crate) error: DomainError,
    pub(crate) retry_delay: Option<Duration>,
}

impl UpstreamAttemptError {
    pub(crate) fn fatal(error: DomainError) -> Self {
        Self {
            error,
            retry_delay: None,
        }
    }
}

pub(crate) fn response_retry_delay(status: StatusCode, headers: &HeaderMap) -> Option<Duration> {
    if !is_retryable_status(status) {
        return None;
    }
    let requested = headers
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| parse_retry_after(value, SystemTime::now()))
        .unwrap_or(UPSTREAM_RETRY_BACKOFF);
    (requested <= MAX_RETRY_AFTER).then_some(requested.max(UPSTREAM_RETRY_BACKOFF))
}

fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) {
        // An overflowing delta is still a long cooldown, not a malformed
        // header that should fall back to an immediate retry.
        return Some(Duration::from_secs(value.parse().unwrap_or(u64::MAX)));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or_default())
}

/// Transient HTTP statuses worth one more attempt: request timeout, rate
/// limit, and any 5xx. Everything else (4xx) is deterministic and must not
/// be replayed.
pub(crate) fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

/// Transport errors worth retrying (connection refused/reset, broken pipe).
/// Timeouts are excluded on purpose: the per-attempt timeout already consumes
/// most of the caller's budget, so a second attempt would land after the
/// client has given up. Builder errors are deterministic and excluded too.
pub(crate) fn is_retryable_transport_error(error: &reqwest::Error) -> bool {
    !error.is_timeout() && !error.is_builder()
}

#[cfg(test)]
mod tests {
    use super::{
        UPSTREAM_RETRY_BACKOFF, is_retryable_status, parse_retry_after, response_retry_delay,
    };
    use crate::{OpenAiTranscriber, VeniceTranscriber, http};
    use clovy_config::UpstreamConfig;
    use clovy_domain::{
        AudioFormat, DomainError, ModelId, ProviderCredentials, Transcriber, TranscriptionRequest,
    };
    use reqwest::{
        StatusCode,
        header::{HeaderMap, HeaderValue, RETRY_AFTER},
    };
    use std::time::{Duration, Instant, SystemTime};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    fn transcriber(provider: &str, server: &MockServer) -> Box<dyn Transcriber> {
        let config = UpstreamConfig {
            api_key: "test-key".to_string(),
            base_url: server.uri(),
            byok_base_url: None,
        };
        match provider {
            "venice" => Box::new(VeniceTranscriber::from_config(
                http::default_client(),
                &config,
            )),
            _ => Box::new(OpenAiTranscriber::from_config(
                http::default_client(),
                &config,
            )),
        }
    }

    fn request() -> TranscriptionRequest {
        TranscriptionRequest {
            audio: b"test audio".to_vec(),
            format: AudioFormat::Wav,
            context: None,
            language: None,
            model: ModelId("test-asr".to_string()),
            provider_credentials: ProviderCredentials::default(),
        }
    }

    #[tokio::test]
    async fn transcribers_retry_when_retry_after_fits_existing_allowance() {
        for provider in ["venice", "openai"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/audio/transcriptions"))
                .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "0"))
                .up_to_n_times(1)
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/audio/transcriptions"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({"text": "Recovered"})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let started = Instant::now();
            let result = transcriber(provider, &server).transcribe(request()).await;
            assert_eq!(result.map(|value| value.text), Ok("Recovered".to_string()));
            assert!(
                started.elapsed() >= UPSTREAM_RETRY_BACKOFF,
                "{provider} retried before the rate limit reset"
            );
        }
    }

    #[tokio::test]
    async fn transcribers_do_not_retry_early_when_requested_delay_exceeds_budget() {
        for (provider, delay) in [
            ("venice", "1"),
            ("openai", "1"),
            ("venice", "60"),
            ("openai", "61"),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/audio/transcriptions"))
                .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", delay))
                .expect(1)
                .mount(&server)
                .await;
            let result = transcriber(provider, &server).transcribe(request()).await;
            assert_eq!(result, Err(DomainError::UpstreamProvider));
        }
    }

    #[test]
    fn retry_after_accepts_delta_seconds_and_http_dates() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(
            parse_retry_after(" 60 ", now),
            Some(Duration::from_secs(60))
        );
        let date = httpdate::fmt_http_date(now + Duration::from_secs(30));
        assert_eq!(parse_retry_after(&date, now), Some(Duration::from_secs(30)));
        let past = httpdate::fmt_http_date(now - Duration::from_secs(30));
        assert_eq!(parse_retry_after(&past, now), Some(Duration::ZERO));
    }

    #[test]
    fn retry_after_waits_are_bounded_without_shortening_long_cooldowns() {
        for (value, expected) in [
            ("1", None),
            ("60", None),
            ("61", None),
            ("18446744073709551616", None),
            ("0", Some(UPSTREAM_RETRY_BACKOFF)),
            ("invalid", Some(UPSTREAM_RETRY_BACKOFF)),
            ("-1", Some(UPSTREAM_RETRY_BACKOFF)),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(RETRY_AFTER, HeaderValue::from_str(value).unwrap());
            assert_eq!(
                response_retry_delay(StatusCode::TOO_MANY_REQUESTS, &headers),
                expected,
                "{value}"
            );
            assert_eq!(
                response_retry_delay(StatusCode::UNAUTHORIZED, &headers),
                None
            );
        }
        assert_eq!(
            response_retry_delay(StatusCode::SERVICE_UNAVAILABLE, &HeaderMap::new()),
            Some(UPSTREAM_RETRY_BACKOFF)
        );
        let mut headers = HeaderMap::new();
        let date = httpdate::fmt_http_date(SystemTime::now() + Duration::from_secs(120));
        headers.insert(RETRY_AFTER, HeaderValue::from_str(&date).unwrap());
        assert_eq!(
            response_retry_delay(StatusCode::TOO_MANY_REQUESTS, &headers),
            None
        );
    }

    #[test]
    fn server_errors_and_rate_limits_are_retryable() {
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::REQUEST_TIMEOUT));
    }

    #[test]
    fn deterministic_client_errors_are_not_retryable() {
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::PAYLOAD_TOO_LARGE));
        assert!(!is_retryable_status(StatusCode::UNPROCESSABLE_ENTITY));
    }
}
