//! Transport-independent JSON-RPC envelopes and MCP content.

use serde_json::{json, Value};

pub const SERVER_NAME: &str = "clovy";
/// Newest first; an unknown client version gets the newest.
const PROTOCOL_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
/// Server-defined: Clovy (or its MCP server) is unreachable or failed.
pub const SERVER_UNAVAILABLE: i64 = -32000;

pub fn result_response(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

pub fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

/// What one incoming JSON-RPC message is.
#[derive(Debug, PartialEq)]
pub enum Incoming<'a> {
    Request {
        id: Value,
        method: &'a str,
    },
    Notification,
    /// A response the client sent back (Clovy sends it no requests).
    Response,
    /// Not a valid JSON-RPC 2.0 message: answered with `INVALID_REQUEST`
    /// and `id` (`null` when the id itself is unusable).
    Invalid {
        id: Value,
        reason: &'static str,
    },
}

pub fn classify(message: &Value) -> Incoming<'_> {
    let Some(object) = message.as_object() else {
        return Incoming::Invalid {
            id: Value::Null,
            reason: "Expected one JSON-RPC message object.",
        };
    };
    let id = object.get("id");
    let usable_id = id.filter(|id| id.is_string() || id.is_number()).cloned();
    let reply_id = usable_id.clone().unwrap_or(Value::Null);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Incoming::Invalid {
            id: reply_id,
            reason: "`jsonrpc` must be \"2.0\".",
        };
    }
    match object.get("method") {
        Some(Value::String(method)) => match (id, usable_id) {
            (None, _) => Incoming::Notification,
            (Some(_), Some(id)) => Incoming::Request { id, method },
            (Some(_), None) => Incoming::Invalid {
                id: Value::Null,
                reason: "`id` must be a string or a number.",
            },
        },
        Some(_) => Incoming::Invalid {
            id: reply_id,
            reason: "`method` must be a string.",
        },
        None if id.is_some() && (object.contains_key("result") || object.contains_key("error")) => {
            Incoming::Response
        }
        None => Incoming::Invalid {
            id: reply_id,
            reason: "A request needs a `method`.",
        },
    }
}

/// The `id` of a valid request; `None` for anything else.
pub fn request_id(message: &Value) -> Option<Value> {
    match classify(message) {
        Incoming::Request { id, .. } => Some(id),
        _ => None,
    }
}

pub fn method(message: &Value) -> Option<&str> {
    message.get("method").and_then(Value::as_str)
}

pub fn initialize_result(params: &Value, instructions: &str) -> Value {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    let version = PROTOCOL_VERSIONS
        .iter()
        .find(|version| Some(**version) == requested)
        .copied()
        .unwrap_or(PROTOCOL_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false }
        },
        "serverInfo": { "name": SERVER_NAME, "title": "Clovy", "version": env!("CARGO_PKG_VERSION") },
        "instructions": instructions,
    })
}

/// A tool result whose single text block is `value` as JSON, or, when the
/// tool returned [`content_value`], those MCP content blocks as they are.
pub fn tool_result(value: &Value) -> Value {
    if let Some(blocks) = content_blocks(value) {
        return json!({ "content": blocks, "isError": false });
    }
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "isError": false,
    })
}

/// The reserved key of [`content_value`]. Tools that return plain JSON never
/// use it, so their results keep the single text block.
pub const CONTENT_KEY: &str = "$mcpContent";

/// A tool return value carrying MCP content blocks instead of one JSON text
/// block (addendum to contract C4, 2026-10-15): `text` and `image` blocks
/// only, built with [`text_block`] and [`image_block`].
pub fn content_value(blocks: Vec<Value>) -> Value {
    json!({ CONTENT_KEY: blocks })
}

pub fn text_block(text: impl Into<String>) -> Value {
    json!({ "type": "text", "text": text.into() })
}

/// An image block: `data` is base64, `mime_type` an `image/*` type.
pub fn image_block(data: impl Into<String>, mime_type: impl Into<String>) -> Value {
    json!({ "type": "image", "data": data.into(), "mimeType": mime_type.into() })
}

/// The blocks of a [`content_value`], when every one is a well-formed text
/// or image block; anything else is serialized as JSON text like before.
fn content_blocks(value: &Value) -> Option<&Vec<Value>> {
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let blocks = object.get(CONTENT_KEY)?.as_array()?;
    let valid = |block: &Value| match block.get("type").and_then(Value::as_str) {
        Some("text") => block.get("text").is_some_and(Value::is_string),
        Some("image") => {
            block.get("data").is_some_and(Value::is_string)
                && block
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .is_some_and(|mime| mime.starts_with("image/"))
        }
        _ => false,
    };
    (!blocks.is_empty() && blocks.iter().all(valid)).then_some(blocks)
}

/// A failed tool call the model can read (MCP reports tool failures as
/// results, not protocol errors).
pub fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true,
    })
}

pub(crate) fn tool_error_for(error: &crate::domain::types::AppError) -> Value {
    let mut result = tool_error(&format!("{} ({})", error.message, error.code));
    result["structuredContent"] = json!({"error": {
        "code": error.code, "message": error.message, "details": error.details,
    }});
    // Keep the familiar first text block while making delivery and retry
    // disposition visible to clients that consume text only.
    if let Some(details) = error.details.as_ref() {
        if let Some(content) = result["content"].as_array_mut() {
            content.push(text_block(
                json!({"code":error.code,"details":details}).to_string(),
            ));
        }
    }
    result
}
