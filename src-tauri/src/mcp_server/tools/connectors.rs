//! Connector parity through the native catalog and dispatch. The short-lived
//! catalog is a menu, never an authorization grant: dispatch resolves live
//! account scopes after the broker approves each mutation.
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::{query::query, row::Row};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

use super::{ToolCtx, ToolGroup, TurnBinding};
use crate::approval_broker::{ApprovalKind, ApprovalOutcome, ApprovalRequest, ApprovalTurn};
use crate::connectors::notion::{self, BrokerAction};
use crate::domain::types::AppError;
use crate::mcp_server::turns::TURN_REQUIRED;

const READS: &[&str] = &[
    "search_threads",
    "read_thread",
    "list_unread",
    "list_events",
    "get_event",
    "find_free_slots",
    "list_repositories",
    "get_pull_request",
    "search_issues",
    "get_issue",
    "list_issue_comments",
];
const WRITES: &[&str] = &[
    "create_draft",
    "send_email",
    "modify_labels",
    "archive",
    "create_event",
    "respond_to_invite",
    "create_issue",
    "update_issue",
    "add_comment",
];
const CATALOG_TTL: Duration = Duration::from_secs(15);

/// Bounded, per-binding cache. Holding the async mutex coalesces simultaneous
/// list requests; neither credentials nor authorization outcomes are cached.
pub(crate) struct TurnCache<T> {
    values: Mutex<Vec<(TurnBinding, String, Instant, T)>>,
}
impl<T: Clone + Send> TurnCache<T> {
    pub(crate) const fn new() -> Self {
        Self {
            values: Mutex::const_new(Vec::new()),
        }
    }
    pub(crate) async fn get_or_load<F>(&self, ctx: &ToolCtx<'_>, load: F) -> Result<T, AppError>
    where
        F: std::future::Future<Output = Result<T, AppError>> + Send,
    {
        let binding = bound(ctx)?;
        let mut values = self.values.lock().await;
        values.retain(|(_, _, at, _)| at.elapsed() < CATALOG_TTL);
        if let Some((_, _, _, value)) = values
            .iter()
            .find(|(turn, profile, _, _)| turn == binding && profile == &ctx.data.profile)
        {
            return Ok(value.clone());
        }
        let value = load.await?;
        if values.len() >= 64 {
            values.remove(0);
        }
        values.push((
            binding.clone(),
            ctx.data.profile.clone(),
            Instant::now(),
            value.clone(),
        ));
        Ok(value)
    }
}

pub(crate) fn bound<'a>(ctx: &ToolCtx<'a>) -> Result<&'a TurnBinding, AppError> {
    ctx.binding
        .ok_or_else(|| AppError::new(TURN_REQUIRED, "This tool requires a Clovy turn."))
}
pub(crate) async fn ensure_active(ctx: &ToolCtx<'_>) -> Result<(), AppError> {
    ctx.revalidate().await?;
    let binding = bound(ctx)?;
    let row = query("SELECT status FROM agent_runs WHERE id = ? AND session_id = ?")
        .bind(&binding.run_id)
        .bind(&binding.session_id)
        .fetch_optional(&ctx.data.notes)
        .await?;
    if row.is_some_and(|row| {
        matches!(
            row.get::<String, _>("status").as_str(),
            "running" | "waiting_for_user"
        )
    }) {
        Ok(())
    } else {
        Err(AppError::new(
            "run_cancelled",
            "This turn is no longer active.",
        ))
    }
}

/// Correlate a host-generated call id with the broker-generated approval id.
/// The id is never accepted from model arguments; the broker remains C2's
/// authority, and Notion claims the durable item once before sending.
pub(crate) async fn approve(
    ctx: &ToolCtx<'_>,
    name: &str,
    arguments: &Value,
    title: String,
    description: String,
    preview: Option<String>,
) -> Result<String, AppError> {
    let binding = bound(ctx)?;
    let call_id = uuid::Uuid::new_v4().to_string();
    let outcome = ctx
        .broker()?
        .request(
            &ApprovalTurn::from(binding),
            ApprovalRequest {
                tool: name.into(),
                call_id: Some(call_id.clone()),
                kind: ApprovalKind::Approval {
                    title,
                    description,
                    arguments: arguments.clone(),
                    preview,
                },
            },
        )
        .await
        .map_err(|d| AppError::new(d.code(), d.message()))?;
    if outcome != ApprovalOutcome::Approved {
        return Err(AppError::new(
            "approval_unavailable",
            "The broker did not approve this action.",
        ));
    }
    ensure_active(ctx).await?;
    let row = query("SELECT json_extract(payload_json, '$.id') AS approval_id FROM agent_items WHERE session_id = ? AND run_id = ? AND kind = 'interruption' AND json_extract(payload_json, '$.engine') = 'cli' AND json_extract(payload_json, '$.toolName') = ? AND json_extract(payload_json, '$.toolCallId') = ? AND json_extract(payload_json, '$.approved') = 1 AND json_extract(payload_json, '$.status') = 'resolved'")
        .bind(&binding.session_id).bind(&binding.run_id).bind(name).bind(call_id).fetch_optional(&ctx.data.notes).await?
        .ok_or_else(|| AppError::new("approval_unavailable","The approval receipt is unavailable."))?;
    Ok(row.get("approval_id"))
}

pub(crate) fn descriptor(runtime: &Value) -> Value {
    json!({"name":runtime["name"],"description":runtime["description"],"inputSchema":runtime["parameters"],"annotations":{"readOnlyHint":runtime["requiresApproval"] != true}})
}

pub(crate) struct PreparedCall {
    pub name: String,
    pub arguments: Value,
    pub notion: Option<BrokerAction>,
}
pub(crate) trait ConnectorDriver: Send + Sync {
    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Result<Vec<Value>, AppError>>;
    fn invoke<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        call: PreparedCall,
        approval_id: Option<String>,
    ) -> BoxFuture<'a, Result<Value, AppError>>;
}
pub(crate) struct DirectConnectors;
impl ConnectorDriver for DirectConnectors {
    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Result<Vec<Value>, AppError>> {
        Box::pin(async move {
            let app = ctx.app.ok_or_else(|| {
                AppError::new(
                    "connector_unavailable",
                    "The connector host is unavailable.",
                )
            })?;
            crate::connectors::agent_tools::descriptors(app).await
        })
    }
    fn invoke<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        call: PreparedCall,
        approval_id: Option<String>,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let app = ctx.app.ok_or_else(|| {
                AppError::new(
                    "connector_unavailable",
                    "The connector host is unavailable.",
                )
            })?;
            if let Some(action) = call.notion {
                let id = approval_id.ok_or_else(|| {
                    AppError::new(
                        "notion_action_binding_missing",
                        "The approval receipt is missing.",
                    )
                })?;
                return serde_json::to_value(
                    action
                        .execute(app, &ctx.data.notes, bound(ctx)?, &id)
                        .await?,
                )
                .map_err(|e| AppError::new("agent_connector_response_invalid", e.to_string()));
            }
            if call.name.starts_with("notion-") {
                return serde_json::to_value(
                    notion::call_hosted_tool(
                        app,
                        notion::NotionHostedToolCallRequest {
                            tool_name: call.name,
                            arguments: call.arguments,
                            deadline_unix_ms: None,
                        },
                    )
                    .await?,
                )
                .map_err(|e| AppError::new("agent_connector_response_invalid", e.to_string()));
            }
            crate::connectors::agent_tools::dispatch(app, &call.name, call.arguments)
                .await?
                .ok_or_else(|| AppError::new("mcp_tool_unknown", "Unknown connector tool."))
        })
    }
}

pub(crate) struct ConnectorTools<D> {
    pub(crate) driver: D,
    cache: TurnCache<Vec<Value>>,
}
impl<D> ConnectorTools<D> {
    pub(crate) const fn new(driver: D) -> Self {
        Self {
            driver,
            cache: TurnCache::new(),
        }
    }
}
pub(crate) static CONNECTORS: ConnectorTools<DirectConnectors> =
    ConnectorTools::new(DirectConnectors);
impl<D: ConnectorDriver> ToolGroup for ConnectorTools<D> {
    fn owns(&self, name: &str) -> bool {
        READS.contains(&name) || WRITES.contains(&name) || name.starts_with("notion-")
    }
    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        Vec::new()
    }
    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            if ctx.binding.is_none() {
                return Vec::new();
            }
            self.cache
                .get_or_load(ctx, async {
                    let catalog = self.driver.catalog(ctx).await?;
                    let mut visible = Vec::new();
                    for tool in catalog {
                        let Some(name) = tool["name"].as_str() else {
                            continue;
                        };
                        if super::routine_tool_allowed(&ctx.data.notes, bound(ctx)?, name).await? {
                            visible.push(descriptor(&tool));
                        }
                    }
                    Ok(visible)
                })
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(code=%error.code,"Connector discovery unavailable");
                    Vec::new()
                })
        })
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let binding = bound(ctx)?;
            let notion = if name.starts_with("notion-") {
                match notion::runtime_name_to_provider(name) {
                    Some((_, true)) => Some(BrokerAction::prepare(name, args)?),
                    Some((_, false)) => None,
                    None => {
                        return Err(AppError::new(
                            "notion_tool_not_allowed",
                            "This Notion tool is not enabled.",
                        ))
                    }
                }
            } else {
                None
            };
            let mutation = WRITES.contains(&name) || notion.is_some();
            // Ask the broker first for unattended mutations too, so D6 has
            // the stable denial code and never opens a pending card.
            let approval_id = if mutation {
                let (title, description, preview) = notion
                    .as_ref()
                    .map(|action| action.presentation())
                    .unwrap_or_else(|| {
                        (
                            format!("Approve {}", name.replace('_', " ")),
                            "Clovy will perform this action in the connected account.".into(),
                            None,
                        )
                    });
                Some(approve(ctx, name, args, title, description, preview).await?)
            } else {
                None
            };
            if !super::routine_tool_allowed(&ctx.data.notes, binding, name).await? {
                return Err(AppError::new(
                    "routine_tool_not_enabled",
                    "This tool is not enabled for the routine.",
                ));
            }
            ensure_active(ctx).await?;
            self.driver
                .invoke(
                    ctx,
                    PreparedCall {
                        name: name.into(),
                        arguments: args.clone(),
                        notion,
                    },
                    approval_id,
                )
                .await
        })
    }
}
