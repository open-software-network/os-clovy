//! The interaction group: `request_clarification` (asks the user through a
//! clarification card of the approval broker; the answer is the tool result)
//! and `approve_action`, the tool a Claude Code turn sends its own permission
//! requests to (`--permission-prompt-tool mcp__clovy__approve_action`, spike
//! S1 of `cli-substitui-agente-nativo`). Bound to a turn; `approve_action` is
//! listed only to Claude turns, and the turn's policy keeps it from the model
//! (`--disallowedTools`, `chat_engine::policy`).

use futures_util::future::BoxFuture;
use serde_json::{json, Value};

use super::{ToolCtx, ToolGroup, TurnBinding};
use crate::approval_broker::{ApprovalKind, ApprovalOutcome, ApprovalRequest, ApprovalTurn};
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;
use crate::mcp_server::turns::TURN_REQUIRED;

pub const REQUEST_CLARIFICATION: &str = "request_clarification";
pub const APPROVE_ACTION: &str = "approve_action";
const INTERACTION_TOOLS: [&str; 2] = [REQUEST_CLARIFICATION, APPROVE_ACTION];

/// Longest Claude Code tool name shown on an approval card.
const MAX_TOOL_NAME_CHARS: usize = 64;

/// The tools of this file as a [`ToolGroup`].
pub struct InteractionTools;

impl ToolGroup for InteractionTools {
    fn owns(&self, name: &str) -> bool {
        INTERACTION_TOOLS.contains(&name)
    }

    fn descriptors(&self, ctx: &ToolCtx<'_>) -> Vec<Value> {
        let mut tools = vec![json!({
            "name": REQUEST_CLARIFICATION,
            "description": "Pause and ask the user a question when their answer is required to continue. Clovy shows the question with the choices; the user's answer is the result.",
            "inputSchema": { "type": "object", "properties": { "question": { "type": "string" }, "choices": { "type": "array", "items": { "type": "string" } } }, "required": ["question", "choices"], "additionalProperties": false },
            "annotations": { "readOnlyHint": true, "openWorldHint": false, "idempotentHint": false },
        })];
        if ctx.binding.is_some_and(is_claude) {
            tools.push(json!({
                "name": APPROVE_ACTION,
                "description": "Claude Code's permission prompt for this Clovy turn: asks the user in Clovy and answers allow or deny. Not for direct use.",
                "inputSchema": { "type": "object", "properties": { "tool_name": { "type": "string" }, "input": { "type": "object" }, "tool_use_id": { "type": "string" } }, "required": ["tool_name", "input"] },
                "annotations": { "readOnlyHint": true, "openWorldHint": false, "idempotentHint": false },
            }));
        }
        tools
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
                    "Clovy can ask the user only inside a Clovy chat turn.",
                )
            })?;
            match name {
                REQUEST_CLARIFICATION => request_clarification(ctx, binding, arguments).await,
                APPROVE_ACTION if is_claude(binding) => {
                    Ok(approve_action(ctx, binding, arguments).await)
                }
                APPROVE_ACTION => Err(AppError::new(
                    "approve_action_claude_only",
                    "approve_action answers Claude Code's permission prompts only.",
                )),
                other => Err(AppError::new(
                    "mcp_tool_unknown",
                    format!("Unknown tool `{other}`."),
                )),
            }
        })
    }
}

fn is_claude(binding: &TurnBinding) -> bool {
    binding.cli == CliKind::Claude
}

/// Asks the question on a clarification card and answers `{answer}` (the
/// native tool's result).
async fn request_clarification(
    ctx: &ToolCtx<'_>,
    binding: &TurnBinding,
    arguments: &Value,
) -> Result<Value, AppError> {
    let question = arguments
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|question| !question.is_empty())
        .ok_or_else(|| AppError::new("agent_tool_arguments_invalid", "question is required."))?;
    let choices = match arguments.get("choices") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(choices)) => choices
            .iter()
            .map(|choice| {
                choice
                    .as_str()
                    .map(|choice| choice.trim().to_string())
                    .ok_or_else(|| {
                        AppError::new(
                            "agent_tool_arguments_invalid",
                            "choices must be a list of strings.",
                        )
                    })
            })
            .filter(|choice| choice.as_ref().map_or(true, |choice| !choice.is_empty()))
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => {
            return Err(AppError::new(
                "agent_tool_arguments_invalid",
                "choices must be a list of strings.",
            ))
        }
    };
    let request = ApprovalRequest {
        tool: REQUEST_CLARIFICATION.to_string(),
        call_id: None,
        kind: ApprovalKind::Clarification {
            question: question.to_string(),
            choices,
        },
    };
    match ctx
        .broker()?
        .request(&ApprovalTurn::from(binding), request)
        .await
        .map_err(|denial| AppError::new(denial.code(), denial.message()))?
    {
        ApprovalOutcome::Answered(answer) => Ok(json!({ "answer": answer })),
        ApprovalOutcome::Approved => Err(AppError::new(
            "agent_clarification_unanswered",
            "The clarification has not been answered.",
        )),
    }
}

/// Claude Code's permission request (`{tool_name, input, tool_use_id}`)
/// asked on an approval card. The answer is always a normal result in the
/// format claude reads (S1): `{"behavior":"allow","updatedInput":<input>}`
/// once the user approves, `{"behavior":"deny","message":…}` otherwise,
/// including when nobody can be asked.
async fn approve_action(ctx: &ToolCtx<'_>, binding: &TurnBinding, arguments: &Value) -> Value {
    match ask_permission(ctx, binding, arguments).await {
        Ok(input) => json!({ "behavior": "allow", "updatedInput": input }),
        Err(error) => json!({
            "behavior": "deny",
            "message": format!("{} ({})", error.message, error.code),
        }),
    }
}

async fn ask_permission(
    ctx: &ToolCtx<'_>,
    binding: &TurnBinding,
    arguments: &Value,
) -> Result<Value, AppError> {
    let tool_name = arguments
        .get("tool_name")
        .and_then(Value::as_str)
        .map(card_tool_name)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| AppError::new("agent_tool_arguments_invalid", "tool_name is required."))?;
    let input = match arguments.get("input") {
        Some(input @ Value::Object(_)) => input.clone(),
        None | Some(Value::Null) => json!({}),
        Some(_) => {
            return Err(AppError::new(
                "agent_tool_arguments_invalid",
                "input must be an object.",
            ))
        }
    };
    let request = ApprovalRequest {
        tool: tool_name.clone(),
        call_id: None,
        kind: ApprovalKind::Approval {
            title: format!("Allow Claude Code to use {tool_name}"),
            description: format!(
                "Claude Code asks to use its {tool_name} tool in this chat. Nothing runs until you approve."
            ),
            arguments: input.clone(),
            preview: permission_preview(&input),
        },
    };
    ctx.broker()?
        .request(&ApprovalTurn::from(binding), request)
        .await
        .map_err(|denial| AppError::new(denial.code(), denial.message()))?;
    ctx.revalidate().await?;
    Ok(input)
}

/// The tool name as shown on a card: claude's own names and MCP names keep
/// letters, digits, `_`, `-`, and `.`; anything else is dropped.
fn card_tool_name(name: &str) -> String {
    name.chars()
        .filter(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        })
        .take(MAX_TOOL_NAME_CHARS)
        .collect()
}

/// The one line that says what the request does: the command of a shell
/// call, the file of an edit, the address of a fetch; `None` lets the broker
/// summarise the input.
fn permission_preview(input: &Value) -> Option<String> {
    ["command", "file_path", "notebook_path", "url", "path"]
        .into_iter()
        .find_map(|key| input.get(key).and_then(Value::as_str))
        .map(str::to_string)
}
