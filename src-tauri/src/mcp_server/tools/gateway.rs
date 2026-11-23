//! Host-owned MCP gateway. Only descriptors and tool results cross the relay;
//! secrets stay in AgentMcpSubsystem's Keychain-backed transport. A cached
//! menu never grants authority: the native run-policy snapshot is checked
//! before dispatch and again after an approval.
use super::connectors::{approve, bound, ensure_active, TurnCache};
use super::{ToolCtx, ToolGroup};
use crate::agent_mcp::{
    self, AgentMcpError, AgentMcpRepository, AgentMcpSubsystem, KeychainMcpSecretStore,
    McpSecretStore, McpToolPolicy, RuntimeToolDescriptorJson,
};
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::approval_broker::{ApprovalKind, ApprovalOutcome, ApprovalRequest, ApprovalTurn};
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Mutex;

pub(crate) trait GatewayDriver: Send + Sync {
    type Secrets: McpSecretStore + 'static;
    fn secrets(&self) -> Self::Secrets;
}
pub(crate) struct NativeGateway;
impl GatewayDriver for NativeGateway {
    type Secrets = KeychainMcpSecretStore;
    fn secrets(&self) -> Self::Secrets {
        KeychainMcpSecretStore
    }
}
struct Catalog<S: McpSecretStore> {
    subsystem: AgentMcpSubsystem<S>,
    descriptors: Vec<RuntimeToolDescriptorJson>,
    workspace: PathBuf,
}
type CallLocks = Mutex<HashMap<String, Arc<Mutex<()>>>>;
pub(crate) struct GatewayTools<D: GatewayDriver> {
    driver: D,
    cache: TurnCache<Arc<Catalog<D::Secrets>>>,
    calls: OnceLock<CallLocks>,
}
impl<D: GatewayDriver> GatewayTools<D> {
    pub(crate) const fn new(driver: D) -> Self {
        Self {
            driver,
            cache: TurnCache::new(),
            calls: OnceLock::new(),
        }
    }
    async fn discover(&self, ctx: &ToolCtx<'_>) -> Result<Arc<Catalog<D::Secrets>>, AppError> {
        let binding = bound(ctx)?;
        let session = AgentRepository::new(ctx.data.notes.clone())
            .get_session(&binding.session_id)
            .await?;
        let workspace = PathBuf::from(session.workspace_path.ok_or_else(|| {
            AppError::new("agent_workspace_missing", "This turn has no workspace.")
        })?);
        let subsystem = AgentMcpSubsystem::new(
            AgentMcpRepository::new(ctx.data.notes.clone()),
            self.driver.secrets(),
        );
        let sandboxed = binding.safety_mode == AgentSafetyMode::Sandboxed;
        let descriptors = agent_mcp::in_cli_turn(&binding.run_id, async {
            match ctx.app {
                Some(app) => {
                    subsystem
                        .refresh_registry_for_workspace_with_managed_linear(
                            app,
                            sandboxed,
                            Some(&workspace),
                        )
                        .await
                }
                None => {
                    subsystem
                        .refresh_registry_for_workspace(sandboxed, Some(&workspace))
                        .await
                }
            }
        })
        .await
        .map_err(mcp_discovery_error)?;
        for diagnostic in subsystem
            .discovery_diagnostics()
            .map_err(mcp_discovery_error)?
        {
            tracing::warn!(
                origin = %diagnostic.origin,
                server = %diagnostic.server,
                phase = %diagnostic.phase,
                code = %diagnostic.code,
                state = %diagnostic.state,
                action = %diagnostic.action,
                remote_code = ?diagnostic.remote_code,
                "MCP server omitted from the turn catalog"
            );
        }
        agent_mcp::snapshot_run_policies(&ctx.data.notes, &binding.run_id, &descriptors)
            .await
            .map_err(mcp_discovery_error)?;
        Ok(Arc::new(Catalog {
            subsystem,
            descriptors,
            workspace,
        }))
    }
    async fn check(
        &self,
        ctx: &ToolCtx<'_>,
        catalog: &Catalog<D::Secrets>,
        name: &str,
    ) -> Result<McpToolPolicy, AppError> {
        ensure_active(ctx).await?;
        let binding = bound(ctx)?;
        let policy = catalog
            .subsystem
            .policy_for_tool(name)
            .map_err(|error| mcp_call_error(error, "gateway", name))?;
        let policy = match policy {
            Some(policy) => policy,
            None => {
                if let Some(mut diagnostic) = catalog
                    .subsystem
                    .discovery_failure_for_tool(name)
                    .map_err(|error| mcp_call_error(error, "gateway", name))?
                {
                    diagnostic.tool = Some(name.to_owned());
                    return Err(diagnostic.into_app_error());
                }
                return Err(mcp_call_error(
                    AgentMcpError::ToolUnavailable,
                    "gateway",
                    name,
                ));
            }
        };
        let server_name = catalog
            .subsystem
            .server_name_for_tool(name)
            .map_err(|error| mcp_call_error(error, "gateway", name))?
            .ok_or_else(|| mcp_call_error(AgentMcpError::ToolUnavailable, "gateway", name))?;
        if crate::routines::routine_mcp_server_allowed_for_session(
            &ctx.data.notes,
            &binding.session_id,
            &policy.server_id,
            &server_name,
            policy.requires_approval,
        )
        .await?
            == Some(false)
        {
            return Err(AppError::new(
                "routine_tool_not_enabled",
                "This MCP server is not enabled for the routine.",
            ));
        }
        if !agent_mcp::run_policy_matches(&ctx.data.notes, &binding.run_id, name, &policy)
            .await
            .map_err(|error| mcp_call_error(error, &server_name, name))?
        {
            return Err(AppError::new("agent_mcp_policy_changed","This MCP server changed during the turn. Retry the turn to use its current policy."));
        }
        Ok(policy)
    }
    async fn invoke(
        &self,
        ctx: &ToolCtx<'_>,
        catalog: &Catalog<D::Secrets>,
        name: &str,
        args: &Value,
        policy: &McpToolPolicy,
    ) -> Result<Value, AppError> {
        let binding = bound(ctx)?;
        let sandboxed = binding.safety_mode == AgentSafetyMode::Sandboxed;
        let server_name = catalog
            .subsystem
            .server_name_for_tool(name)
            .map_err(|error| mcp_call_error(error, "gateway", name))?
            .ok_or_else(|| mcp_call_error(AgentMcpError::ToolUnavailable, "gateway", name))?;
        let mut answer = None::<String>;
        let mut dispatched = false;
        for _ in 0..16 {
            if let Err(error) = self.check(ctx, catalog, name).await {
                if dispatched {
                    agent_mcp::retire_server_sessions(&policy.server_id).await;
                    return Err(after_dispatch(error, &server_name, name));
                }
                return Err(error);
            }
            let operation = async {
                match ctx.app {
                    Some(app) => {
                        catalog
                            .subsystem
                            .invoke_in_workspace_with_elicitation_and_managed_linear(
                                app,
                                name,
                                args.clone(),
                                sandboxed,
                                Some(&catalog.workspace),
                                answer.as_deref(),
                            )
                            .await
                    }
                    None => {
                        catalog
                            .subsystem
                            .invoke_in_workspace_with_elicitation(
                                name,
                                args.clone(),
                                sandboxed,
                                Some(&catalog.workspace),
                                answer.as_deref(),
                            )
                            .await
                    }
                }
            };
            dispatched = true;
            let result = tokio::select! {
                result=operation=>result,
                _=wait_cancelled(ctx)=> {
                    agent_mcp::retire_server_sessions(&policy.server_id).await;
                    return Err(after_dispatch(AppError::new("run_cancelled","This turn was cancelled."), &server_name, name));
                }
            };
            match result {
                Ok(value) => return Ok(value),
                Err(AgentMcpError::ElicitationRequired(message)) => {
                    let broker = match ctx.broker() {
                        Ok(broker) => broker,
                        Err(error) => {
                            agent_mcp::retire_server_sessions(&policy.server_id).await;
                            return Err(after_dispatch(error, &server_name, name));
                        }
                    };
                    let clarified = broker
                        .request(
                            &ApprovalTurn::from(binding),
                            ApprovalRequest {
                                tool: name.into(),
                                call_id: None,
                                kind: ApprovalKind::Clarification {
                                    question: format!("MCP server asks: {message}"),
                                    choices: Vec::new(),
                                },
                            },
                        )
                        .await;
                    match clarified {
                        Ok(ApprovalOutcome::Answered(value)) => answer = Some(value),
                        other => {
                            agent_mcp::retire_server_sessions(&policy.server_id).await;
                            return Err(after_dispatch(
                                match other {
                                    Err(denial) => AppError::new(denial.code(), denial.message()),
                                    _ => AppError::new(
                                        "approval_unavailable",
                                        "The clarification was not answered.",
                                    ),
                                },
                                &server_name,
                                name,
                            ));
                        }
                    }
                    // Same subsystem, arguments and parked session. Its
                    // transport resumes the original request, never submits
                    // the potentially mutating tools/call a second time.
                }
                Err(error) => {
                    if answer.is_some() {
                        agent_mcp::retire_server_sessions(&policy.server_id).await;
                        return Err(mcp_call_error(
                            error.after_elicitation(),
                            &server_name,
                            name,
                        ));
                    }
                    return Err(mcp_call_error(error, &server_name, name));
                }
            }
        }
        agent_mcp::retire_server_sessions(&policy.server_id).await;
        Err(after_dispatch(
            AppError::new(
                "agent_mcp_tool_failed",
                "The MCP server asked too many clarification questions.",
            ),
            &server_name,
            name,
        ))
    }
}
fn after_dispatch(error: AppError, server: &str, tool: &str) -> AppError {
    let mut diagnostic =
        AgentMcpError::DeliveryUncertain(None).diagnostic(server, Some(tool), "call");
    diagnostic.cause = error
        .details
        .as_ref()
        .and_then(|details| {
            serde_json::from_value::<agent_mcp::AgentMcpDiagnostic>(details.clone()).ok()
        })
        .map(Box::new);
    diagnostic.code = error.code;
    diagnostic.message = format!(
        "{} The original MCP request may have been applied. Inspect the server before retrying.",
        error.message
    );
    diagnostic.into_app_error()
}
async fn wait_cancelled(ctx: &ToolCtx<'_>) {
    loop {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if ensure_active(ctx).await.is_err() {
            return;
        }
    }
}
fn mcp_discovery_error(error: AgentMcpError) -> AppError {
    error
        .diagnostic("gateway", None, "discover")
        .into_app_error()
}
fn mcp_call_error(error: AgentMcpError, server: &str, tool: &str) -> AppError {
    error
        .diagnostic(server, Some(tool), "call")
        .into_app_error()
}
fn descriptor(tool: &RuntimeToolDescriptorJson) -> Value {
    // Custom MCP policy may waive approval for a mutating tool. Do not
    // mislabel that as read-only: only the managed Linear classifier proves
    // a read from the provider annotations. Unknown hints stay conservative.
    let remote_annotations = serde_json::to_value(&tool.annotations).unwrap_or_else(|_| json!({}));
    let mut annotations = remote_annotations.as_object().cloned().unwrap_or_default();
    annotations.insert(
        "readOnlyHint".into(),
        json!(
            tool.approval_provider.as_deref() == Some("Linear")
                && tool.requires_approval != Some(true)
        ),
    );
    json!({"name":tool.name,"description":tool.description,"inputSchema":tool.parameters,"annotations":annotations,"_meta":{"clovy":{"origin":"clovy_managed","id":tool.id,"remoteAnnotations":remote_annotations}}})
}
pub(crate) static GATEWAY: GatewayTools<NativeGateway> = GatewayTools::new(NativeGateway);
impl<D: GatewayDriver> ToolGroup for GatewayTools<D> {
    fn owns(&self, name: &str) -> bool {
        name.starts_with("mcp_")
    }
    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        Vec::new()
    }
    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            if ctx.binding.is_none() {
                return Vec::new();
            }
            let catalog = match self.cache.get_or_load(ctx, self.discover(ctx)).await {
                Ok(catalog) => catalog,
                Err(error) => {
                    tracing::warn!(code=%error.code,"MCP gateway discovery unavailable");
                    return Vec::new();
                }
            };
            let mut tools = Vec::new();
            for tool in &catalog.descriptors {
                if self.check(ctx, &catalog, &tool.name).await.is_ok() {
                    tools.push(descriptor(tool));
                }
            }
            tools
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
            agent_mcp::in_cli_turn(&binding.run_id, async {
                let mut catalog = self.discover(ctx).await?;
                let policy = self.check(ctx, &catalog, name).await?;
                // Serialize each server through elicitation too. A second turn
                // must not answer an earlier turn's parked request.
                let lock = {
                    let mut locks = self
                        .calls
                        .get_or_init(|| Mutex::new(HashMap::new()))
                        .lock()
                        .await;
                    locks.retain(|_, lock| Arc::strong_count(lock) > 1);
                    locks
                        .entry(policy.server_id.clone())
                        .or_insert_with(|| Arc::new(Mutex::new(())))
                        .clone()
                };
                let _guard = lock.lock().await;
                if policy.requires_approval {
                    approve(
                        ctx,
                        name,
                        args,
                        "Approve MCP action".into(),
                        "Clovy will call this connected MCP tool with the displayed arguments."
                            .into(),
                        None,
                    )
                    .await?;
                    if policy.server_id == agent_mcp::MANAGED_LINEAR_SERVER_ID {
                        catalog = self.discover(ctx).await?;
                    }
                }
                let current = self.check(ctx, &catalog, name).await?;
                let result = self.invoke(ctx, &catalog, name, args, &current).await;
                if result.is_err() {
                    agent_mcp::retire_server_sessions(&current.server_id).await;
                }
                result
            })
            .await
        })
    }
}
