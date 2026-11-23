//! The computer use group: `computer_use` for an attended CLI turn (change
//! `cli-substitui-agente-nativo`, slice 2.6). Each call follows the turn’s effective approval policy:
//!
//! 1. the turn binding opens the run's Computer use lease
//!    (`crate::computer_use::begin_turn_lease`), in place of the chat surface;
//! 2. the approval broker records the decision; when a card is shown, the first answer
//!    settles it and any later one is refused;
//! 3. only then the existing Computer use broker runs the action, and the
//!    per-app authorization of the Computer use broker still applies
//!    (`computer_use.rs`).
//!
//! Unattended turns are refused with `unattended_requires_approval` before
//! anything happens. The tool result carries the driver's content as MCP
//! blocks (`protocol::content_value`): the capture as an `image` block, the
//! text and structured content as `text` blocks.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use tauri::AppHandle;
use uuid::Uuid;

use super::{ToolCtx, ToolGroup};
use crate::approval_broker::{ApprovalDenial, ApprovalKind, ApprovalRequest, ApprovalTurn};
use crate::domain::types::AppError;
use crate::mcp_server::protocol;
use crate::mcp_server::turns::{TurnBinding, TURN_REQUIRED};

pub const COMPUTER_USE_TOOL: &str = "computer_use";

const DESCRIPTION: &str =
    "Operate the attended computer-use session through Clovy's permission and approval broker.";

fn parameters() -> Value {
    let coordinate = json!({ "type": "array", "items": { "type": "integer", "minimum": 0, "maximum": 100000 }, "minItems": 2, "maxItems": 2 });
    json!({
        "type": "object",
        "properties": {
            "action": { "type": "string", "enum": ["capture", "list_apps", "wait", "open_app", "focus_app", "click", "double_click", "right_click", "drag", "scroll", "type", "key", "set_value"] },
            "app": { "type": "string", "minLength": 1, "maxLength": 200 },
            "window_id": { "type": "integer", "minimum": 1 },
            "mode": { "type": "string", "enum": ["ax", "vision", "som"] },
            "max_elements": { "type": "integer", "minimum": 1, "maximum": 1000 },
            "element": { "type": "integer", "minimum": 0 },
            "coordinate": coordinate,
            "from_coordinate": coordinate,
            "to_coordinate": coordinate,
            "button": { "type": "string", "enum": ["left", "right"] },
            "modifiers": { "type": "array", "items": { "type": "string", "enum": ["cmd", "ctrl", "shift", "option", "fn"] }, "uniqueItems": true },
            "text": { "type": "string", "minLength": 1, "maxLength": 10000 },
            "value": { "type": "string", "maxLength": 10000 },
            "keys": { "type": "string", "maxLength": 64 },
            "direction": { "type": "string", "enum": ["up", "down", "left", "right"] },
            "amount": { "type": "integer", "minimum": 1, "maximum": 50 },
            "seconds": { "type": "number", "minimum": 0, "maximum": 30 },
            "raise_window": { "type": "boolean" },
            "capture_after": { "type": "boolean" }
        },
        "required": ["action"],
        "additionalProperties": false
    })
}

/// What the group needs from the app: the lease and the Computer use broker.
/// Tests replace it with a fake driver.
pub(crate) trait ComputerUseHost: Send + Sync {
    /// Opens (or keeps) the turn's Computer use lease.
    fn begin_turn(&self, binding: &TurnBinding) -> Result<(), AppError>;

    /// Runs the approved action. Returns the broker's MCP-shaped result.
    fn execute(&self, run_id: String, arguments: Value) -> BoxFuture<'_, Value>;
}

/// The app's Computer use broker.
struct AppHost<'a> {
    app: &'a AppHandle,
}

impl ComputerUseHost for AppHost<'_> {
    fn begin_turn(&self, binding: &TurnBinding) -> Result<(), AppError> {
        crate::computer_use::begin_turn_lease(self.app, binding)
    }

    fn execute(&self, run_id: String, arguments: Value) -> BoxFuture<'_, Value> {
        Box::pin(crate::computer_use::handle_cli_action_for_call(
            self.app, run_id, arguments,
        ))
    }
}

/// The computer use group. The registered instance acts through the app;
/// tests build one over a fake host.
pub struct ComputerUseTools {
    host: Option<Arc<dyn ComputerUseHost>>,
}

/// The instance registered in [`super::groups`].
pub static COMPUTER_USE: ComputerUseTools = ComputerUseTools { host: None };

impl ComputerUseTools {
    #[cfg(test)]
    pub(crate) fn with_host(host: Arc<dyn ComputerUseHost>) -> Self {
        Self { host: Some(host) }
    }

    async fn run(
        &self,
        host: &dyn ComputerUseHost,
        ctx: &ToolCtx<'_>,
        binding: &TurnBinding,
        arguments: &Value,
    ) -> Result<Value, AppError> {
        let action = crate::computer_use::normalized_action(arguments)?;
        let broker = ctx.broker()?;
        host.begin_turn(binding)?;
        let call_id = Uuid::new_v4().to_string();
        let decision = broker
            .request(
                &ApprovalTurn::from(binding),
                ApprovalRequest {
                    tool: COMPUTER_USE_TOOL.to_string(),
                    call_id: Some(call_id.clone()),
                    kind: ApprovalKind::Approval {
                        title: "Use this computer".into(),
                        description: format!(
                            "Clovy runs one Computer use action ({action}) on this computer."
                        ),
                        arguments: arguments.clone(),
                        preview: None,
                    },
                },
            )
            .await;
        decision.map_err(|denial| AppError::new(denial.code(), denial.message()))?;
        ctx.revalidate().await?;
        let result = host
            .execute(binding.run_id.clone(), arguments.clone())
            .await;
        if is_error(&result) {
            return Err(driver_error(&result));
        }
        Ok(content_for_cli(&result))
    }
}

impl ToolGroup for ComputerUseTools {
    fn owns(&self, name: &str) -> bool {
        name == COMPUTER_USE_TOOL
    }

    fn descriptors(&self, ctx: &ToolCtx<'_>) -> Vec<Value> {
        if !ctx.binding.is_some_and(|binding| binding.attended) {
            return Vec::new();
        }
        vec![json!({
            "name": COMPUTER_USE_TOOL,
            "description": format!(
                "{DESCRIPTION} Your effective approval policy and exact app/window grants apply to every call. Start with list_apps, then capture an exact window_id. Captures return an image and numbered elements. Prefer semantic element actions in the background; an operation that requires focus or pointer input can use visible mode in that authorized window. Inspect effective_modality and disposition in the result. Never repeat delivered or uncertain input; observe the window first. `type` inserts text; `set_value` replaces it. Passwords, one-time codes and payment fields require user takeover. All arguments go beside action."
            ),
            "inputSchema": parameters(),
            "annotations": { "readOnlyHint": false, "destructiveHint": true, "openWorldHint": true },
        })]
    }

    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let binding = ctx.binding.ok_or_else(|| {
                AppError::new(
                    TURN_REQUIRED,
                    format!("`{name}` is available only inside a Clovy chat turn."),
                )
            })?;
            if !binding.attended {
                let denial = ApprovalDenial::UnattendedRequiresApproval;
                return Err(AppError::new(denial.code(), denial.message()));
            }
            match (&self.host, ctx.app) {
                (Some(host), _) => self.run(host.as_ref(), ctx, binding, arguments).await,
                (None, Some(app)) => self.run(&AppHost { app }, ctx, binding, arguments).await,
                (None, None) => Err(AppError::new(
                    "computer_use_unavailable",
                    "Computer use is unavailable in this Clovy process.",
                )),
            }
        })
    }
}

fn is_error(result: &Value) -> bool {
    result.get("isError").and_then(Value::as_bool) == Some(true)
}

fn driver_error(result: &Value) -> AppError {
    result
        .get("structuredContent")
        .and_then(|value| value.get("error"))
        .and_then(|error| serde_json::from_value(error.clone()).ok())
        .unwrap_or_else(|| AppError::new("computer_use_failed", error_text(result)))
}

/// The error text of an MCP-shaped failure (`{"error": "..."}` in text).
fn error_text(result: &Value) -> String {
    let text = result["content"]
        .as_array()
        .and_then(|parts| parts.iter().find_map(|part| part["text"].as_str()))
        .unwrap_or("Computer use failed.");
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| text.to_string())
}

/// Images larger than this (base64 bytes) stay in Clovy: the relay and
/// the CLI read a response as one line.
pub(crate) const MAX_IMAGE_BASE64: usize = 3 * 1024 * 1024;

/// The driver's MCP-shaped result as MCP content blocks for the CLI: its
/// text and image blocks, then its structured content as JSON text.
fn content_for_cli(result: &Value) -> Value {
    let mut blocks = Vec::new();
    for part in result["content"].as_array().into_iter().flatten() {
        match part["type"].as_str() {
            Some("text") => {
                if let Some(text) = part["text"].as_str() {
                    blocks.push(protocol::text_block(text));
                }
            }
            Some("image") => match (part["data"].as_str(), part["mimeType"].as_str()) {
                (Some(data), Some(mime))
                    if mime.starts_with("image/") && data.len() <= MAX_IMAGE_BASE64 =>
                {
                    blocks.push(protocol::image_block(data, mime));
                }
                _ => blocks.push(protocol::text_block(
                    "The capture image stays in Clovy (too large or unreadable); use the element list.",
                )),
            },
            _ => {}
        }
    }
    let structured = result
        .get("structuredContent")
        .or_else(|| result.get("structured_content"));
    if let Some(structured) = structured.filter(|value| !value.is_null()) {
        blocks.push(protocol::text_block(structured.to_string()));
    }
    if blocks.is_empty() {
        blocks.push(protocol::text_block("{}"));
    }
    protocol::content_value(blocks)
}

#[cfg(test)]
mod result_tests {
    use super::*;

    #[test]
    fn cli_keeps_the_delivered_action_and_complete_follow_up_error() {
        let action_receipt = json!({
            "action_id": "action-one",
            "disposition": { "delivery": "delivered_unverified", "retry": "unsafe" },
            "effective_modality": "visible",
            "engine_evidence": { "steps": [{ "mechanism": "native", "outcome": "succeeded" }] }
        });
        let action = json!({
            "content": [{ "type": "text", "text": "Original action observation." }],
            "structuredContent": action_receipt
        });
        let error = AppError {
            code: "computer_use_desktop_unavailable".into(),
            message: "The native desktop became unavailable.".into(),
            details: Some(json!({
                "engineReceipt": {
                    "error": { "code": "capture_failed", "details": { "clovy_code": "computer_use_desktop_unavailable", "cause": "native desktop changed" } },
                    "disposition": { "delivery": "not_delivered", "retry": "safe" }
                },
                "disposition": { "delivery": "not_delivered", "retry": "safe" },
                "effective_modality": "background"
            })),
        };
        let expected_error = serde_json::to_value(&error).unwrap();
        let combined = crate::computer_use::combine_action_and_capture(action, Err(error));
        // Exercise the final CLI consumer and MCP envelope, not just the helper.
        let mcp = protocol::tool_result(&content_for_cli(&combined));
        assert_eq!(mcp["isError"], false);
        let blocks = mcp["content"].as_array().unwrap();
        assert_eq!(blocks[0]["text"], "Original action observation.");
        let warning: Value = serde_json::from_str(blocks[1]["text"].as_str().unwrap()).unwrap();
        assert_eq!(warning["capture_after_error"], expected_error);
        let delivered: Value = serde_json::from_str(blocks[2]["text"].as_str().unwrap()).unwrap();
        assert_eq!(delivered, action_receipt);
    }
}
