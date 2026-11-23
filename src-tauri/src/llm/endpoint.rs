//! One-shot generation against an OpenAI-compatible `/chat/completions`
//! endpoint, its structured-output probe, and per-endpoint RPM pacing.

use super::{
    cli::{schema_instruction, strictify},
    matches_schema, parse_json_value, GenerateRequest, LlmError, StructuredOutputLevel,
};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const ERROR_DETAIL_CHARS: usize = 200;

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    crate::http_client::build_client(crate::http_client::base_client_builder(), "llm_endpoint")
});

/// Even-spacing reservations: the next free slot per endpoint id.
static NEXT_SLOT: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Everything needed to call one endpoint, key included. Never serialized.
#[derive(Clone, PartialEq, Eq)]
pub struct EndpointConnection {
    pub id: String,
    pub base_url: String,
    pub model_id: String,
    pub api_key: Option<String>,
    /// Requests per minute; 0 means unmetered.
    pub rpm_limit: u32,
}

impl std::fmt::Debug for EndpointConnection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EndpointConnection")
            .field("id", &self.id)
            .field("base_url", &self.base_url)
            .field("model_id", &self.model_id)
            .field("has_api_key", &self.api_key.is_some())
            .field("rpm_limit", &self.rpm_limit)
            .finish()
    }
}

/// Runs one completion. `level` decides how a requested schema is enforced:
/// `response_format` for json_schema/strict/json_object, a prompt contract
/// otherwise.
pub async fn complete(
    connection: &EndpointConnection,
    request: &GenerateRequest,
    level: StructuredOutputLevel,
) -> Result<String, LlmError> {
    wait_for_slot(&connection.id, connection.rpm_limit).await;
    let body = request_body(connection, request, level);
    let mut http = HTTP_CLIENT
        .post(format!(
            "{}/chat/completions",
            connection.base_url.trim_end_matches('/')
        ))
        .timeout(request.timeout.unwrap_or(DEFAULT_TIMEOUT))
        .json(&body);
    if let Some(key) = connection.api_key.as_deref().filter(|key| !key.is_empty()) {
        http = http.bearer_auth(key);
    }
    let response = http.send().await.map_err(|error| {
        if error.is_timeout() {
            LlmError::TimedOut
        } else {
            LlmError::EndpointFailed(
                "Could not reach the endpoint. Check the URL and that the server is running."
                    .to_string(),
            )
        }
    })?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|_| {
        LlmError::EndpointFailed("Could not read the endpoint response.".to_string())
    })?;
    let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    if !status.is_success() {
        if matches!(status.as_u16(), 401 | 403) {
            return Err(LlmError::EndpointFailed(
                "The endpoint rejected the API key.".to_string(),
            ));
        }
        let detail = crate::http_client::json_error_message(&value, ERROR_DETAIL_CHARS)
            .map(|message| format!(": {message}"))
            .unwrap_or_default();
        return Err(LlmError::EndpointFailed(format!(
            "The endpoint returned status {}{detail}",
            status.as_u16()
        )));
    }
    let text = completion_text(&value)
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or_else(|| LlmError::InvalidOutput("The endpoint returned no text.".to_string()))?;
    if let Some(schema) = &request.schema {
        if !parse_json_value(&text).is_some_and(|value| matches_schema(&value, schema)) {
            return Err(LlmError::InvalidOutput(
                "The endpoint did not return JSON matching the schema.".to_string(),
            ));
        }
    }
    Ok(text)
}

fn request_body(
    connection: &EndpointConnection,
    request: &GenerateRequest,
    level: StructuredOutputLevel,
) -> Value {
    let mut user = request.prompt.trim().to_string();
    let mut response_format = None;
    if let Some(schema) = &request.schema {
        match level {
            StructuredOutputLevel::Strict | StructuredOutputLevel::JsonSchema => {
                response_format = Some(json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "answer",
                        "strict": level == StructuredOutputLevel::Strict,
                        "schema": strictify(schema),
                    }
                }));
            }
            StructuredOutputLevel::JsonObject => {
                response_format = Some(json!({ "type": "json_object" }));
                user = format!("{user}\n\n{}", schema_instruction(schema));
            }
            StructuredOutputLevel::Prompt | StructuredOutputLevel::None => {
                user = format!("{user}\n\n{}", schema_instruction(schema));
            }
        }
    }
    let mut messages = Vec::new();
    if let Some(system) = request
        .system
        .as_deref()
        .map(str::trim)
        .filter(|system| !system.is_empty())
    {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.push(json!({ "role": "user", "content": user }));
    let mut body = json!({
        "model": connection.model_id,
        "messages": messages,
        "stream": false,
    });
    if let Some(format) = response_format {
        body["response_format"] = format;
    }
    body
}

fn completion_text(value: &Value) -> Option<String> {
    match value.pointer("/choices/0/message/content")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(parts) => Some(
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    }
}

/// Measures latency with a minimal prompt, then walks the structured-output
/// ladder (strict, json_schema, json_object, prompt) and returns the first
/// level whose answer validates against the probe schema. All rungs failing
/// yields `none`.
///
/// The schema-enforcing rungs use a prompt that asks for a different object
/// than the schema allows ([`probe_prompt`]), so an endpoint that silently
/// ignores `response_format` follows the prompt, fails validation, and is not
/// credited with schema support.
pub async fn probe(
    connection: &EndpointConnection,
    timeout: Duration,
) -> Result<(Duration, StructuredOutputLevel), LlmError> {
    let started = Instant::now();
    complete(
        connection,
        &GenerateRequest {
            system: None,
            prompt: "Reply with the single word: ok".to_string(),
            schema: None,
            timeout: Some(timeout),
        },
        StructuredOutputLevel::Prompt,
    )
    .await?;
    let latency = started.elapsed();
    for level in [
        StructuredOutputLevel::Strict,
        StructuredOutputLevel::JsonSchema,
        StructuredOutputLevel::JsonObject,
        StructuredOutputLevel::Prompt,
    ] {
        let request = GenerateRequest {
            system: None,
            prompt: probe_prompt(level).to_string(),
            schema: Some(probe_schema()),
            timeout: Some(timeout),
        };
        if let Ok(text) = complete(connection, &request, level).await {
            if probe_answer_ok(&text) {
                return Ok((latency, level));
            }
        }
    }
    Ok((latency, StructuredOutputLevel::None))
}

/// Only `{"answer": "schema"}` satisfies this schema.
pub fn probe_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "answer": { "type": "string", "const": "schema" } },
        "required": ["answer"],
        "additionalProperties": false
    })
}

/// Prompt for one probe rung. Where the level claims to enforce the schema,
/// the prompt deliberately asks for an object the schema forbids; only real
/// enforcement yields a valid answer. Prompt-level rungs carry the schema in
/// the prompt, so they are asked to follow it.
pub fn probe_prompt(level: StructuredOutputLevel) -> &'static str {
    if level.supports_json_schema() {
        "Reply with a JSON object whose \"color\" field is \"blue\"."
    } else {
        "Reply with the JSON object the schema describes."
    }
}

pub fn probe_answer_ok(text: &str) -> bool {
    parse_json_value(text).is_some_and(|value| matches_schema(&value, &probe_schema()))
}

async fn wait_for_slot(endpoint_id: &str, rpm: u32) {
    if rpm == 0 {
        return;
    }
    let interval = Duration::from_secs_f64(60.0 / f64::from(rpm));
    let wait = {
        let Ok(mut slots) = NEXT_SLOT.lock() else {
            return;
        };
        let now = Instant::now();
        let slot = slots
            .get(endpoint_id)
            .copied()
            .map_or(now, |next| next.max(now));
        slots.insert(endpoint_id.to_string(), slot + interval);
        slot - now
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

#[cfg(test)]
pub(crate) mod test_server {
    //! A tiny OpenAI-compatible server for tests: answers each request with
    //! the handler's (status, body) and records the parsed request bodies and
    //! `Authorization` headers.

    use serde_json::Value;
    use std::sync::{Arc, Mutex};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    pub type Handler = Arc<dyn Fn(&Value) -> (u16, Value) + Send + Sync>;
    /// A handler that also names the response headers, for rate-limit replies.
    pub type HeaderHandler =
        Arc<dyn Fn(&Value) -> (u16, Value, Vec<(String, String)>) + Send + Sync>;

    pub struct FakeServer {
        pub base_url: String,
        pub requests: Arc<Mutex<Vec<Value>>>,
        pub authorizations: Arc<Mutex<Vec<Option<String>>>>,
        /// Each request's line and headers, as received.
        pub heads: Arc<Mutex<Vec<String>>>,
        /// Each request body as text (multipart bodies are not JSON).
        pub raw_bodies: Arc<Mutex<Vec<String>>>,
    }

    pub async fn start(handler: Handler) -> FakeServer {
        start_with_headers(Arc::new(move |request: &Value| {
            let (status, body) = handler(request);
            (status, body, Vec::new())
        }))
        .await
    }

    pub async fn start_with_headers(handler: HeaderHandler) -> FakeServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let authorizations = Arc::new(Mutex::new(Vec::new()));
        let recorded_auth = authorizations.clone();
        let heads = Arc::new(Mutex::new(Vec::new()));
        let recorded_heads = heads.clone();
        let raw_bodies = Arc::new(Mutex::new(Vec::new()));
        let recorded_raw_bodies = raw_bodies.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                let recorded = recorded.clone();
                let recorded_auth = recorded_auth.clone();
                let recorded_heads = recorded_heads.clone();
                let recorded_raw_bodies = recorded_raw_bodies.clone();
                tokio::spawn(async move {
                    let mut buffer = Vec::new();
                    let mut chunk = [0_u8; 8192];
                    let (head, body) = loop {
                        let read = socket.read(&mut chunk).await.unwrap_or(0);
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                        let text = String::from_utf8_lossy(&buffer).to_string();
                        if let Some(split) = text.find("\r\n\r\n") {
                            let length = text[..split]
                                .lines()
                                .find_map(|line| {
                                    let (name, value) = line.split_once(':')?;
                                    name.eq_ignore_ascii_case("content-length")
                                        .then(|| value.trim().parse::<usize>().ok())?
                                })
                                .unwrap_or(0);
                            if buffer.len() >= split + 4 + length {
                                break (
                                    text[..split].to_string(),
                                    buffer[split + 4..split + 4 + length].to_vec(),
                                );
                            }
                        }
                    };
                    let authorization = head.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("authorization")
                            .then(|| value.trim().to_string())
                    });
                    recorded_auth.lock().unwrap().push(authorization);
                    recorded_heads.lock().unwrap().push(head.clone());
                    recorded_raw_bodies
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&body).to_string());
                    let request: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let (status, response, headers) = handler(&request);
                    recorded.lock().unwrap().push(request);
                    let payload = response.to_string();
                    let extra = headers
                        .iter()
                        .map(|(name, value)| format!("{name}: {value}\r\n"))
                        .collect::<String>();
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\n{extra}content-length: {}\r\nconnection: close\r\n\r\n{payload}",
                        payload.len()
                    );
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        FakeServer {
            base_url: format!("http://{address}/v1"),
            requests,
            authorizations,
            heads,
            raw_bodies,
        }
    }

    pub fn chat_reply(text: &str) -> Value {
        serde_json::json!({ "choices": [{ "message": { "role": "assistant", "content": text } }] })
    }
}

#[cfg(test)]
mod tests {
    use super::{test_server::*, *};
    use std::sync::Arc;

    fn connection(base_url: &str) -> EndpointConnection {
        EndpointConnection {
            id: "test".to_string(),
            base_url: base_url.to_string(),
            model_id: "model-x".to_string(),
            api_key: None,
            rpm_limit: 0,
        }
    }

    #[tokio::test]
    async fn llm_endpoint_probe_reports_latency_and_strict_level() {
        // A server that enforces the schema: whatever the prompt asks, the
        // answer satisfies `response_format`.
        let server = start(Arc::new(|request: &Value| {
            if request.get("response_format").is_some() {
                (200, chat_reply(r#"{"answer": "schema"}"#))
            } else {
                (200, chat_reply("ok"))
            }
        }))
        .await;
        let (latency, level) = probe(&connection(&server.base_url), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(level, StructuredOutputLevel::Strict);
        assert!(latency > Duration::ZERO);
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests[0]["model"], "model-x");
        assert_eq!(
            requests[1]["response_format"]["json_schema"]["strict"],
            true
        );
    }

    /// Answers like a model that ignores `response_format` and only follows
    /// the prompt text.
    fn prompt_follower(request: &Value) -> (u16, Value) {
        let prompt = request["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .and_then(|message| message["content"].as_str())
            .unwrap_or_default()
            .to_string();
        if prompt.contains("\"color\"") {
            (200, chat_reply(r#"{"color": "blue"}"#))
        } else if prompt.contains("JSON schema") {
            (200, chat_reply(r#"{"answer": "schema"}"#))
        } else {
            (200, chat_reply("ok"))
        }
    }

    #[tokio::test]
    async fn llm_endpoint_ignoring_response_format_is_not_credited_with_schema_support() {
        let server = start(Arc::new(prompt_follower)).await;
        let (_, level) = probe(&connection(&server.base_url), Duration::from_secs(5))
            .await
            .unwrap();
        assert!(
            !level.supports_json_schema(),
            "a prompt-following endpoint was credited with {level:?}"
        );
        assert_eq!(level, StructuredOutputLevel::JsonObject);
    }

    #[tokio::test]
    async fn llm_endpoint_answer_outside_the_schema_is_rejected() {
        let server = start(Arc::new(prompt_follower)).await;
        let error = complete(
            &connection(&server.base_url),
            &GenerateRequest {
                system: None,
                prompt: probe_prompt(StructuredOutputLevel::Strict).to_string(),
                schema: Some(probe_schema()),
                timeout: Some(Duration::from_secs(5)),
            },
            StructuredOutputLevel::Strict,
        )
        .await
        .unwrap_err();
        assert!(matches!(error, LlmError::InvalidOutput(_)), "{error:?}");
    }

    #[tokio::test]
    async fn llm_endpoint_probe_falls_back_to_prompt_level() {
        // A server that rejects every response_format but follows the prompt.
        let server = start(Arc::new(|request: &Value| {
            if request.get("response_format").is_some() {
                (
                    400,
                    serde_json::json!({ "error": { "message": "response_format unsupported" } }),
                )
            } else {
                (200, chat_reply("```json\n{\"answer\": \"schema\"}\n```"))
            }
        }))
        .await;
        let (_, level) = probe(&connection(&server.base_url), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(level, StructuredOutputLevel::Prompt);
    }

    #[tokio::test]
    async fn llm_endpoint_failure_surfaces_status_without_the_key() {
        let server = start(Arc::new(|_: &Value| {
            (
                500,
                serde_json::json!({ "error": { "message": "model not loaded" } }),
            )
        }))
        .await;
        let mut connection = connection(&server.base_url);
        connection.api_key = Some("sk-hidden".to_string());
        let error = probe(&connection, Duration::from_secs(5))
            .await
            .unwrap_err();
        let message = crate::domain::types::AppError::from(error).message;
        assert!(
            message.contains("500") && message.contains("model not loaded"),
            "{message}"
        );
        assert!(!message.contains("sk-hidden"));
        assert!(!format!("{connection:?}").contains("sk-hidden"));
    }

    #[tokio::test]
    async fn llm_endpoint_rpm_limit_spaces_requests() {
        let id = format!("rpm-{}", uuid::Uuid::new_v4());
        let started = Instant::now();
        wait_for_slot(&id, 600).await;
        wait_for_slot(&id, 600).await;
        wait_for_slot(&id, 600).await;
        // 600 rpm = one slot per 100 ms; the third call waits ~200 ms.
        assert!(started.elapsed() >= Duration::from_millis(190));
    }
}
