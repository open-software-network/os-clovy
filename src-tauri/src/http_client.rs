//! Shared construction for Clovy's long-lived outbound HTTP clients.

use std::time::Duration;

pub const USER_AGENT: &str = concat!("clovy/", env!("CARGO_PKG_VERSION"));

/// Direct connections only (ambient proxy variables could redirect
/// credentials), pooled keep-alive, and Clovy's user agent. Callers add their
/// own timeouts, headers, and redirect policy.
pub fn base_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_keepalive(Some(Duration::from_secs(30)))
        .user_agent(USER_AGENT)
}

/// Builds `builder`. A failure is logged and falls back to a plain client
/// that still ignores proxies, never to one that would honor them.
pub fn build_client(builder: reqwest::ClientBuilder, purpose: &str) -> reqwest::Client {
    builder.build().unwrap_or_else(|error| {
        tracing::error!(%error, purpose, "HTTP client configuration failed; using defaults");
        reqwest::Client::builder()
            .no_proxy()
            .user_agent(USER_AGENT)
            .build()
            .expect("the HTTP client backend could not be initialized")
    })
}

/// The human-readable message of a JSON error body: `error.message`, a bare
/// string `error`, or a top-level `message`, cut to `max_chars` characters.
/// Never echoes the rest of the body.
pub fn json_error_message(body: &serde_json::Value, max_chars: usize) -> Option<String> {
    body.pointer("/error/message")
        .or_else(|| body.get("error"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| body.get("message").and_then(serde_json::Value::as_str))
        .filter(|message| !message.is_empty())
        .map(|message| message.chars().take(max_chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_error_message_reads_each_error_shape_in_order() {
        let nested = json!({ "error": { "message": "nested" }, "message": "top" });
        assert_eq!(json_error_message(&nested, 100).as_deref(), Some("nested"));
        let bare = json!({ "error": "bare", "message": "top" });
        assert_eq!(json_error_message(&bare, 100).as_deref(), Some("bare"));
        let envelope = json!({ "error_code": 2001, "message": "top" });
        assert_eq!(json_error_message(&envelope, 100).as_deref(), Some("top"));
        assert_eq!(
            json_error_message(&json!({ "error": { "code": 1 } }), 100),
            None
        );
        assert_eq!(json_error_message(&json!({ "message": "" }), 100), None);
    }

    #[test]
    fn json_error_message_is_cut_on_characters() {
        let body = json!({ "message": "ééé" });
        assert_eq!(json_error_message(&body, 2).as_deref(), Some("éé"));
    }
}
