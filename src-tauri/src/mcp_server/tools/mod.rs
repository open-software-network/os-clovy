//! The Clovy MCP server's tools, registered as groups ([`ToolGroup`]): one
//! file per group, so parallel work on different groups never shares a file
//! beyond the one-line registration in [`groups`]. A group whose tools act on
//! a CLI turn (anything that writes, or that needs the turn's session) is
//! offered only to a connection bound to a turn ([`ToolCtx::binding`]);
//! unbound clients (Cursor, `claude mcp add`) see the read group alone, as
//! ADR-0058 describes, and calling a bound-only tool answers `turn_required`.

mod activity_management;
mod argument_validation;
pub(crate) mod browser;
#[cfg(test)]
mod browser_tests;
pub(crate) mod computer_use;
#[cfg(test)]
mod computer_use_tests;
mod connectors;
#[cfg(test)]
mod connectors_tests;
mod conversation_mutations;
mod conversations;
#[cfg(test)]
mod conversations_tests;
mod dictation;
mod external;
mod gateway;
#[cfg(test)]
mod gateway_tests;
mod interaction;
#[cfg(test)]
mod interaction_tests;
pub(crate) mod media;
#[cfg(test)]
mod media_tests;
mod memory;
#[cfg(test)]
mod memory_tests;
pub(crate) mod notes;
#[cfg(test)]
mod notes_tests;
mod projects;
mod read;
#[cfg(test)]
mod read_tests;
pub(crate) mod recorder;
#[cfg(test)]
mod recorder_tests;
mod routines;
#[cfg(test)]
mod routines_tests;
pub(crate) mod vault_skills;
#[cfg(test)]
mod vault_skills_tests;

use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde_json::Value;
use sqlx_sqlite::SqlitePool;
use tauri::AppHandle;

use crate::activity::settings::ActivitySettings;
use crate::activity::store::ActivityStore;
use crate::approval_broker::external::ExternalCall;
use crate::approval_broker::ApprovalBroker;
use crate::db::repositories::McpMutation;
use crate::domain::types::AppError;

pub use super::turns::TurnBinding;
use super::turns::TURN_REQUIRED;
pub(crate) use conversations::is_conversation_read_tool;
pub(crate) use read::is_read_tool;
pub use read::{
    is_activity_tool, ReadTools, ACTIVITY_TOOLS, GET_ACTIVE_SESSION, GET_ACTIVITY_STATS,
    GET_DAY_PANELS, GET_DAY_SCHEDULE, GET_DAY_SUMMARY, GET_NOTE, GET_SESSION_DETAIL,
    LIST_APP_USAGE, LIST_CODING_AGENT_SESSIONS, LIST_DICTATIONS, LIST_MEMORIES, SEARCH_NOTES,
};

/// A routine binding remains a routine after its persisted authorization is
/// deleted. Missing policy must not turn it into an unrestricted chat turn.
async fn routine_tool_allowed(
    pool: &SqlitePool,
    binding: &TurnBinding,
    name: &str,
) -> Result<bool, AppError> {
    // Connected MCP servers have their own persisted per-server policy,
    // checked by gateway at catalog and dispatch (including after waits).
    if gateway::GATEWAY.owns(name) {
        let routine: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM routine_runs rr JOIN routines r ON r.id = rr.routine_id WHERE rr.agent_session_id = ?")
            .bind(&binding.session_id).fetch_one(pool).await?;
        return Ok(binding.surface != "routine" || routine > 0);
    }
    Ok(
        crate::routines::routine_tool_allowed_for_session(pool, &binding.session_id, name)
            .await?
            .unwrap_or(binding.surface != "routine"),
    )
}

/// What one MCP request may read, resolved by the app per message
/// (`super::app_data`) or built directly by tests.
pub struct McpData {
    /// The main database (`notes.sqlite3`).
    pub notes: SqlitePool,
    /// The authorized data partition: bound requests use the conversation's
    /// frozen run snapshot; external clients use the current app view. Notes,
    /// dictations and memories are limited to it; activity is not partitioned.
    pub profile: String,
    /// Settings, Memory: `list_memories` refuses while memory is off.
    pub memory_enabled: bool,
    pub activity: ActivityAccess,
    pub now: DateTime<Utc>,
}

pub enum ActivityAccess {
    /// Capture is off: activity tools are not offered and refuse calls.
    Off,
    /// Capture is on; the store was not opened for this message.
    On,
    /// Capture is on and the store is open for an activity tool call.
    Open {
        store: ActivityStore,
        settings: ActivitySettings,
    },
    /// Capture is on but the history cannot be read right now.
    Unavailable(AppError),
}

impl ActivityAccess {
    pub fn offered(&self) -> bool {
        !matches!(self, ActivityAccess::Off)
    }
}

/// What one tool call runs with: the data the message may read, the app (for
/// groups that act through it; `None` where no Tauri runtime exists, as in
/// tests), the CLI turn the connection is bound to (`None` for every client
/// outside a Clovy CLI turn), and the approval broker write tools ask before
/// acting (the app's, or a test broker:
/// `crate::approval_broker::testing::TestBroker`).
#[derive(Clone, Copy)]
pub struct ToolCtx<'a> {
    pub data: &'a McpData,
    pub app: Option<&'a AppHandle>,
    pub binding: Option<&'a TurnBinding>,
    pub broker: Option<&'a ApprovalBroker>,
}

impl<'a> ToolCtx<'a> {
    pub(crate) fn recording_data(
        &self,
        external: Option<&ExternalCall<'_>>,
    ) -> Result<std::path::PathBuf, AppError> {
        if let Some(external) = external {
            return external
                .recording_data()
                .map(std::path::Path::to_path_buf)
                .ok_or_else(|| {
                    AppError::new("storage_unavailable", "Recording storage is unavailable.")
                });
        }
        let app = self.app.ok_or_else(|| {
            AppError::new("storage_unavailable", "Recording storage is unavailable.")
        })?;
        crate::app_paths::app_data_dir(app)
            .map_err(|error| AppError::new("storage_unavailable", error.to_string()))
    }

    pub(crate) async fn revalidate_local(
        &self,
        external: Option<&ExternalCall<'_>>,
    ) -> Result<(), AppError> {
        match external {
            Some(call) => call.check_live(),
            None => self.revalidate().await,
        }
    }

    pub(crate) async fn approve_local<'b>(
        &'b self,
        external: Option<&'b ExternalCall<'b>>,
        request: crate::approval_broker::ApprovalRequest,
        target: Option<&'b Value>,
    ) -> Result<McpMutation<'b>, AppError> {
        use crate::approval_broker::{ApprovalKind, ApprovalTurn};
        if let Some(external) = external {
            let ApprovalKind::Approval {
                title,
                description,
                arguments,
                ..
            } = &request.kind
            else {
                return Err(AppError::new(
                    TURN_REQUIRED,
                    "This operation requires a Clovy turn.",
                ));
            };
            return Ok(McpMutation::External(
                external
                    .approve(&request.tool, title, description, arguments, target)
                    .await?,
            ));
        }
        let binding = self
            .binding
            .ok_or_else(|| AppError::new(TURN_REQUIRED, "A bound Clovy turn is required."))?;
        self.broker()?
            .request(&ApprovalTurn::from(binding), request)
            .await
            .map_err(|denial| AppError::new(denial.code(), denial.message()))?;
        Ok(McpMutation::Turn(
            &binding.session_id,
            &binding.run_id,
            &binding.authority,
        ))
    }

    /// Revalidate host ownership immediately before an effect, including
    /// after approval. UI partition changes cannot redirect this context.
    pub(crate) async fn revalidate(&self) -> Result<(), AppError> {
        let Some(binding) = self.binding else {
            return Err(AppError::new(
                TURN_REQUIRED,
                "A bound Clovy turn is required.",
            ));
        };
        revalidate_binding(self.app, &self.data.notes, binding, &self.data.profile).await
    }

    /// A context with neither app, turn, nor broker (what an unbound client
    /// gets apart from the app handle).
    pub fn unbound(data: &'a McpData) -> Self {
        Self {
            data,
            app: None,
            binding: None,
            broker: None,
        }
    }

    /// The approval broker; `approval_unavailable` when the context has none
    /// (the tool must not act without asking).
    pub fn broker(&self) -> Result<&'a ApprovalBroker, AppError> {
        self.broker.ok_or_else(|| {
            AppError::new(
                "approval_unavailable",
                "Clovy could not ask for approval: the approval broker is unavailable.",
            )
        })
    }
}

pub(super) use external::{call_external, external_descriptors};

/// Domain ports use this after their own settings/single-flight locks, before
/// provider dispatch. The binding is supplied by the host, never model fields.
pub(crate) async fn revalidate_binding(
    app: Option<&AppHandle>,
    pool: &SqlitePool,
    binding: &TurnBinding,
    profile: &str,
) -> Result<(), AppError> {
    binding.authority.check_live()?;
    let check_registry = || -> Result<(), AppError> {
        if let Some(app) = app {
            use tauri::Manager;
            let state = app.try_state::<super::McpServerState>().ok_or_else(|| {
                AppError::new("turn_expired", "The turn registry is unavailable.")
            })?;
            if !state.turns().contains_binding(binding) {
                return Err(AppError::new("turn_expired", "The bound turn was revoked."));
            }
        }
        Ok(())
    };
    check_registry()?;
    let actual_profile = super::bound_profile(pool, binding).await?;
    if actual_profile != profile {
        return Err(AppError::new(
            "turn_expired",
            "The bound run or data partition changed.",
        ));
    }
    binding.authority.check_live()?;
    check_registry()
}

/// A set of tools served together. Registered once in [`groups`].
pub trait ToolGroup: Send + Sync {
    /// Whether the group is offered only to connections bound to a turn.
    /// Unbound connections never see its tools, and calling one answers
    /// `turn_required` before the group runs. Defaults to `true`: only the
    /// read group is open to every client.
    fn needs_turn(&self) -> bool {
        true
    }

    /// Whether `name` is one of the group's tools, whatever the context
    /// (decides `Unknown tool` against `turn_required`).
    fn owns(&self, name: &str) -> bool;

    /// The group's `tools/list` entries for this context. Called only when
    /// the context may see the group ([`ToolGroup::needs_turn`]); a group
    /// may still narrow by `ctx.binding` (for example by CLI).
    fn descriptors(&self, ctx: &ToolCtx<'_>) -> Vec<Value>;

    /// The group's `tools/list` entries when building them needs I/O
    /// (connector accounts, a hosted server's inventory, user MCP
    /// discovery). `tools/list` awaits this; the default is
    /// [`ToolGroup::descriptors`], so static groups implement only that.
    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move { self.descriptors(ctx) })
    }

    /// Runs one of the group's tools (`owns(name)` holds). Errors become
    /// `isError` tool results.
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>>;
}

/// Every group, in catalog order. Registering a group is one line here.
pub fn groups() -> Vec<&'static dyn ToolGroup> {
    #[allow(unused_mut)]
    let mut groups: Vec<&'static dyn ToolGroup> = vec![&ReadTools];
    groups.push(&routines::RoutineTools);
    groups.push(&memory::MemoryTools);
    groups.push(&notes::NoteTools);
    groups.push(&conversations::ConversationTools);
    groups.push(&conversation_mutations::ConversationMutations);
    groups.push(&projects::ProjectTools);
    groups.push(&activity_management::ActivityManagementTools);
    groups.push(&dictation::DictationTools);
    groups.push(&vault_skills::VaultSkillTools);
    groups.push(&connectors::CONNECTORS);
    groups.push(&gateway::GATEWAY);
    groups.push(&browser::BROWSER);
    groups.push(&computer_use::COMPUTER_USE);
    groups.push(&media::MediaTools);
    groups.push(&recorder::RecorderTools);
    groups.push(&interaction::InteractionTools);
    #[cfg(all(test, any(unix, windows)))]
    groups.push(&super::turn_tests::TurnProbe);
    groups
}

fn visible(group: &dyn ToolGroup, ctx: &ToolCtx<'_>) -> bool {
    ctx.binding.is_some() || !group.needs_turn()
}

/// The `tools/list` catalog for this context, in group order.
pub async fn descriptors(ctx: &ToolCtx<'_>) -> Vec<Value> {
    let mut tools = Vec::new();
    for group in groups().into_iter().filter(|group| visible(*group, ctx)) {
        for tool in group.catalog(ctx).await {
            if let Some(binding) = ctx.binding {
                let Some(name) = tool["name"].as_str() else {
                    continue;
                };
                if !routine_tool_allowed(&ctx.data.notes, binding, name)
                    .await
                    .unwrap_or(false)
                {
                    continue;
                }
            }
            tools.push(tool);
        }
    }
    tools
}

/// Whether some group owns `name` (bound-only tools included).
pub fn is_known_tool(name: &str) -> bool {
    groups().into_iter().any(|group| group.owns(name))
}

pub(crate) fn is_gateway_tool(name: &str) -> bool {
    gateway::GATEWAY.owns(name)
}

/// Runs a tool. A bound-only tool on an unbound connection answers
/// `turn_required` without running.
pub async fn call(ctx: &ToolCtx<'_>, name: &str, arguments: &Value) -> Result<Value, AppError> {
    let Some(group) = groups().into_iter().find(|group| group.owns(name)) else {
        return Err(AppError::new(
            "mcp_tool_unknown",
            format!("Unknown tool `{name}`."),
        ));
    };
    if !visible(group, ctx) {
        return Err(AppError::new(
            TURN_REQUIRED,
            format!("`{name}` is available only inside a Clovy chat turn."),
        ));
    }
    if let Some(binding) = ctx.binding {
        if !routine_tool_allowed(&ctx.data.notes, binding, name).await? {
            return Err(AppError::new(
                "routine_tool_not_enabled",
                "This tool is not enabled for the routine.",
            ));
        }
        ctx.revalidate().await?;
    }
    // Dynamic connectors and engine-specific protocols validate their own
    // contracts. Native management has closed schemas, including nested DTOs.
    let native = [
        &ReadTools as &dyn ToolGroup,
        &notes::NoteTools,
        &conversations::ConversationTools,
        &conversation_mutations::ConversationMutations,
        &projects::ProjectTools,
        &routines::RoutineTools,
        &memory::MemoryTools,
        &vault_skills::VaultSkillTools,
        &activity_management::ActivityManagementTools,
        &dictation::DictationTools,
        &recorder::RecorderTools,
    ]
    .iter()
    .any(|native| native.owns(name));
    let mut read_only = false;
    if native {
        if let Some(descriptor) = group
            .descriptors(ctx)
            .iter()
            .find(|tool| tool["name"] == name)
        {
            read_only = descriptor["annotations"]["readOnlyHint"] == true;
            argument_validation::validate(&descriptor["inputSchema"], arguments)
                .map_err(|error| AppError::new("agent_tool_arguments_invalid", error))?;
        }
    }
    // Reads have no committed effect to preserve. A revoked run must not
    // receive a snapshot fetched while it was waiting on storage or IO.
    if read_only {
        return authorized_native_read(ctx, name, group.call(ctx, name, arguments)).await;
    }
    group.call(ctx, name, arguments).await
}

/// Publication boundary, after the real read/IO future has completed. A
/// previously allowed routine configuration is not authority for this result.
async fn authorized_native_read(
    ctx: &ToolCtx<'_>,
    name: &str,
    read: impl std::future::Future<Output = Result<Value, AppError>>,
) -> Result<Value, AppError> {
    let result = read.await?;
    if let Some(binding) = ctx.binding {
        ctx.revalidate().await?;
        if !routine_tool_allowed(&ctx.data.notes, binding, name).await? {
            return Err(AppError::new(
                "routine_tool_not_enabled",
                "This tool is not enabled for the routine.",
            ));
        }
        binding.authority.check_live()?;
    }
    Ok(result)
}
