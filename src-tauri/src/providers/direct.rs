//! Direct-provider HTTP transport shared by transcription, dictation and notes.

use super::groq::AUDIO_TOO_LARGE_MESSAGE;
use crate::{domain::types::AppError, providers::BuiltinProvider};
use std::{sync::OnceLock, time::Duration};

// Notes retain the existing ten-minute request deadline.
pub(super) const HTTP_TIMEOUT: Duration = Duration::from_secs(600);

// Dictation with the user's own key calls these hosts directly (ADR-0065).
pub(super) const GROQ_API_BASE: &str = "https://api.groq.com/openai/v1";

pub(super) const OPENROUTER_API_BASE: &str = "https://openrouter.ai/api/v1";

pub(super) const DIRECT_DICTATION_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) static DIRECT_DICTATION_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub(super) static DIRECT_NOTES_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub(super) const REQUEST_FAILED_CODE: &str = "clovy_request_failed";

/// The original request plus one retry for transient provider failures.
pub(super) const DIRECT_PROVIDER_ATTEMPTS: usize = 2;

pub(super) const DIRECT_PROVIDER_RETRY_BACKOFF: Duration = Duration::from_millis(300);

/// The longest `retry-after` a direct call waits out itself. A longer wait
/// goes back to the caller as `details.retryAfterMs`: retrying earlier only
/// spends another request (the note pacer waits instead).
pub(super) const DIRECT_PROVIDER_MAX_INLINE_RATE_LIMIT_WAIT_MS: u64 = 2_000;

/// Direct provider transport shared by dictation and notes. Each request carries
/// only its provider's user key. Tests inject local fake-provider origins.
pub(crate) struct DirectTransport<'a> {
    /// Provider requests carry only the user's key for that provider.
    pub(crate) direct_client: &'a reqwest::Client,
    pub(crate) groq_base: String,
    pub(crate) openrouter_base: String,
    /// Historical URL injection remains test-only so retired-server sentinels
    /// exercise the same transport fixture used before the cut.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) retired_api_base: String,
}

/// Notes use a 600-second deadline because each audio part can be up to 25 MB.
pub(super) fn production_notes_transport() -> DirectTransport<'static> {
    DirectTransport {
        direct_client: direct_notes_client(),
        ..production_dictation_transport()
    }
}

pub(super) fn direct_notes_client() -> &'static reqwest::Client {
    DIRECT_NOTES_CLIENT.get_or_init(|| {
        crate::http_client::build_client(
            crate::http_client::base_client_builder()
                .timeout(HTTP_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none()),
            "notes_direct",
        )
    })
}

#[cfg(test)]
tokio::task_local! { pub(crate) static TEST_ORIGIN: String; }

pub(super) fn production_dictation_transport() -> DirectTransport<'static> {
    // Only test binaries can redirect these provider origins. Timing fixtures
    // run in an isolated child process with ephemeral loopback providers.
    #[cfg(test)]
    let fake_origin = TEST_ORIGIN
        .try_with(Clone::clone)
        .ok()
        .or_else(|| std::env::var("CLOVY_TEST_NOTE_PROVIDER_ORIGIN").ok());
    #[cfg(test)]
    if let Some(origin) = fake_origin {
        assert!(origin.starts_with("http://127.0.0.1:"));
        return DirectTransport {
            direct_client: direct_dictation_client(),
            groq_base: format!("{origin}/openai/v1"),
            openrouter_base: format!("{origin}/api/v1"),
            retired_api_base: String::new(),
        };
    }
    DirectTransport {
        direct_client: direct_dictation_client(),
        groq_base: GROQ_API_BASE.to_string(),
        openrouter_base: OPENROUTER_API_BASE.to_string(),
        #[cfg(test)]
        retired_api_base: String::new(),
    }
}

pub(super) fn direct_dictation_client() -> &'static reqwest::Client {
    DIRECT_DICTATION_CLIENT.get_or_init(|| {
        crate::http_client::build_client(
            crate::http_client::base_client_builder()
                .timeout(DIRECT_DICTATION_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none()),
            "dictation_direct",
        )
    })
}

/// The user's own key for `provider`, and only that provider's. No cache
/// here: the Keychain store already caches per account and is updated when a
/// key is saved or removed, so a take never reads a stale key and one
/// provider's unreadable key never blocks another.
pub(super) fn dictation_user_key(provider: BuiltinProvider) -> Result<Option<String>, AppError> {
    crate::providers::provider_api_key(provider)
}

/// Sends one request to the user's own provider, retrying a transient
/// failure once: 408, 429,
/// 5xx, and a transport error that is not a timeout. A 429 waits for the
/// provider's `retry-after` before that retry, and is returned at once with
/// the wait when it is longer than two seconds.
pub(super) async fn send_direct<F>(
    provider: BuiltinProvider,
    task: &str,
    build: F,
) -> Result<reqwest::Response, AppError>
where
    F: Fn() -> Result<reqwest::RequestBuilder, AppError>,
{
    let mut attempt = 1;
    loop {
        let retry = attempt < DIRECT_PROVIDER_ATTEMPTS;
        let mut delay = DIRECT_PROVIDER_RETRY_BACKOFF;
        match build()?.send().await {
            Ok(response) if response.status().is_success() => return Ok(response),
            Ok(response) => {
                let status = response.status();
                let rate_limit_wait_ms = (status == reqwest::StatusCode::TOO_MANY_REQUESTS)
                    .then(|| rate_limit_wait_ms(response.headers()))
                    .flatten();
                let wait_too_long = rate_limit_wait_ms
                    .is_some_and(|wait_ms| wait_ms > DIRECT_PROVIDER_MAX_INLINE_RATE_LIMIT_WAIT_MS);
                if !(retry && direct_status_is_transient(status)) || wait_too_long {
                    tracing::warn!(
                        provider = provider.id(),
                        status = status.as_u16(),
                        retry_after_ms = rate_limit_wait_ms,
                        "direct provider call failed"
                    );
                    let mut error = direct_note_provider_error(provider, status, task);
                    if let Some(wait_ms) = rate_limit_wait_ms {
                        error.details = Some(serde_json::json!({ "retryAfterMs": wait_ms }));
                    }
                    return Err(error);
                }
                if let Some(wait_ms) = rate_limit_wait_ms {
                    delay = Duration::from_millis(wait_ms);
                }
            }
            Err(error) => {
                if !(retry && !error.is_timeout() && !error.is_builder()) {
                    return Err(network_error(error));
                }
            }
        }
        tracing::warn!(
            provider = provider.id(),
            attempt,
            delay_ms = delay.as_millis() as u64,
            "direct provider call failed transiently, retrying"
        );
        tokio::time::sleep(delay).await;
        attempt += 1;
    }
}

/// The wait a 429 asks for: `retry-after` in seconds, else the request
/// window's reset when no requests remain (Groq's `2m59.56s` format).
pub(super) fn rate_limit_wait_ms(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
    };
    if let Some(seconds) = header("retry-after")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
    {
        return Some((seconds * 1_000.0).ceil() as u64);
    }
    (header("x-ratelimit-remaining-requests") == Some("0"))
        .then(|| header("x-ratelimit-reset-requests").and_then(parse_groq_reset_ms))
        .flatten()
}

/// Parses a reset duration such as `7.66s`, `2m59.56s`, `1h2m3s` or `12ms`.
pub(super) fn parse_groq_reset_ms(value: &str) -> Option<u64> {
    let mut rest = value.trim();
    if rest.is_empty() {
        return None;
    }
    let mut total_ms = 0.0_f64;
    while !rest.is_empty() {
        let number_end = rest
            .find(|character: char| !(character.is_ascii_digit() || character == '.'))
            .unwrap_or(rest.len());
        let number = rest[..number_end].parse::<f64>().ok()?;
        rest = &rest[number_end..];
        let (unit_ms, unit_len) = if rest.starts_with("ms") {
            (1.0, 2)
        } else if rest.starts_with('h') {
            (3_600_000.0, 1)
        } else if rest.starts_with('m') {
            (60_000.0, 1)
        } else if rest.starts_with('s') {
            (1_000.0, 1)
        } else {
            return None;
        };
        total_ms += number * unit_ms;
        rest = &rest[unit_len..];
    }
    Some(total_ms.round() as u64)
}

pub(super) fn direct_status_is_transient(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::REQUEST_TIMEOUT
        || status == reqwest::StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

pub(super) fn direct_note_provider_error(
    provider: BuiltinProvider,
    status: reqwest::StatusCode,
    task: &str,
) -> AppError {
    if status == reqwest::StatusCode::PAYLOAD_TOO_LARGE {
        return AppError::new("audio_too_large", AUDIO_TOO_LARGE_MESSAGE);
    }
    direct_provider_error_for(provider, status, task)
}

pub(super) const INVALID_DIRECT_RESPONSE: &str = "The provider returned an invalid response.";

/// The error a non-success status from the user's own provider becomes,
/// naming the provider and the action needed to recover.
pub(super) fn direct_provider_error(
    provider: BuiltinProvider,
    status: reqwest::StatusCode,
) -> AppError {
    direct_provider_error_for(provider, status, "this dictation")
}

pub(super) fn direct_provider_error_for(
    provider: BuiltinProvider,
    status: reqwest::StatusCode,
    task: &str,
) -> AppError {
    let kind = match status.as_u16() {
        401 | 403 => "api_key_rejected",
        402 => "usage_limit_exceeded",
        404 => "model_unavailable",
        429 => "usage_limit_exceeded",
        _ => {
            return AppError::new(
                REQUEST_FAILED_CODE,
                format!(
                    "{} could not finish {task}. Try again.",
                    provider.display_name()
                ),
            )
        }
    };
    provider_error(&format!("{}_{kind}", provider.id()))
        .unwrap_or_else(|| AppError::new(REQUEST_FAILED_CODE, INVALID_DIRECT_RESPONSE))
}

/// Final assistant text from a chat completion, normalized for reasoning
/// models: inline `<think>` blocks are stripped, and when generation stopped
/// at the token cap (`finish_reason: "length"`) the text is cut back to its
/// last complete sentence so the UI never shows a mid-word fragment. Returns
/// `None` when nothing presentable survives (e.g. the cap landed inside an
/// unterminated think block), which callers surface as a generation error.
pub(super) fn extract_chat_completion_text(value: &serde_json::Value) -> Option<String> {
    let choice = value.get("choices")?.as_array()?.first()?;
    let content = choice.get("message")?.get("content")?.as_str()?;
    let text = strip_think_blocks(content);
    let truncated = choice
        .get("finish_reason")
        .and_then(serde_json::Value::as_str)
        == Some("length");
    let text = if truncated {
        trim_to_sentence_boundary(&text)
    } else {
        text.trim().to_string()
    };
    (!text.is_empty()).then_some(text)
}

/// Removes `<think>...</think>` spans some reasoning models inline into
/// `content`. An unterminated `<think>` means the token cap landed inside the
/// reasoning itself, so nothing after it is an answer; drop it all.
pub(super) fn strip_think_blocks(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => return out.trim().to_string(),
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

/// Cuts capped output back to its last sentence terminator. A fragment with
/// no terminator has no complete sentence to keep, so it yields an empty
/// string rather than a mid-word cutoff.
pub(super) fn trim_to_sentence_boundary(text: &str) -> String {
    let trimmed = text.trim_end();
    match trimmed.rfind(['.', '!', '?']) {
        Some(index) => trimmed[..=index].trim().to_string(),
        None => String::new(),
    }
}

pub(super) fn network_error(error: reqwest::Error) -> AppError {
    AppError::new(REQUEST_FAILED_CODE, error.to_string())
}

fn provider_error(message: &str) -> Option<AppError> {
    let (provider, kind) = [BuiltinProvider::Groq, BuiltinProvider::OpenRouter]
        .into_iter()
        .find_map(|provider| {
            message
                .strip_prefix(provider.id())
                .and_then(|rest| rest.strip_prefix('_'))
                .map(|kind| (provider, kind))
        })?;
    let name = provider.display_name();
    let text = match kind {
        "api_key_rejected" => format!("{name} rejected your saved API key. Open Settings and update it."),
        "model_unavailable" => format!("{name} could not serve the selected model right now. Choose another model or try again later."),
        "usage_limit_exceeded" => format!("{name} reports that its usage limit was reached. Try again later, or check the plan behind your saved {name} key."),
        _ => return None,
    };
    Some(AppError::new(message, text))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn direct_provider_clients_never_follow_a_redirect() {
        use crate::llm::endpoint::test_server;
        use std::sync::Arc;
        for (name, client) in [
            ("notes", direct_notes_client()),
            ("dictation", direct_dictation_client()),
            ("key check", &*super::super::PROVIDER_KEY_CHECK_HTTP_CLIENT),
        ] {
            let target = test_server::start(Arc::new(|_| (200, serde_json::json!({})))).await;
            let location = target.base_url.clone();
            let origin = test_server::start_with_headers(Arc::new(move |_| {
                (
                    302,
                    serde_json::json!({}),
                    vec![("location".to_string(), location.clone())],
                )
            }))
            .await;
            let response = client
                .get(&origin.base_url)
                .bearer_auth("synthetic-key")
                .send()
                .await
                .unwrap();
            assert!(
                target.requests.lock().unwrap().is_empty(),
                "{name} client followed a provider redirect"
            );
            assert_eq!(response.status(), reqwest::StatusCode::FOUND);
            assert_eq!(origin.requests.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn direct_provider_payment_refusal_has_no_retired_credit_action() {
        let error = direct_provider_error(
            BuiltinProvider::OpenRouter,
            reqwest::StatusCode::PAYMENT_REQUIRED,
        );
        assert_eq!(error.code, "openrouter_usage_limit_exceeded");
        assert!(!error.message.to_lowercase().contains("credit"));
        assert!(!error.message.to_lowercase().contains("remove the key"));
        assert!(error.message.contains("OpenRouter"));
    }
}
