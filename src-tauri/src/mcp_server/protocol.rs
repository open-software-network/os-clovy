//! MCP (JSON-RPC 2.0) as the app answers it: lifecycle, `tools/*`, and
//! `resources/*`. Every message is answered on its own (no session state),
//! so any authenticated connection can send any request, and the stdio relay
//! can answer `initialize` itself while the app is unreachable.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, TimeZone, Utc};
use serde_json::{json, Value};

use super::channel::ExternalConnection;
use super::tools::{self, McpData, ToolCtx};
use crate::approval_broker::external::{ExternalApprovalBroker, ExternalCall};
use std::path::Path;

#[path = "protocol/wire.rs"]
mod wire;
pub(crate) use wire::tool_error_for;
pub use wire::*;

pub const CONTEXT_URI: &str = "clovy://context";
pub const GUIDE_URI: &str = "clovy://guide";
const RESOURCE_NOT_FOUND: i64 = -32002;

const INSTRUCTIONS: &str = "Read-only access to the user's Clovy notes, dictations, and memories (from the data set open in Clovy) and, when activity capture is on, their activity timeline, day summaries, and coding-agent work on this computer. Read clovy://context for the current date, time, and time zone before resolving words like \"today\"; clovy://guide explains which tool answers which question.";
const EXTERNAL_INSTRUCTIONS: &str = "Read the user's Clovy notes, dictations, memories, and available Activity. Use only the tools listed for this connection. Protected changes require a one-time decision in Clovy's MCP settings, using the active data partition at the start of the request. Read clovy://context for dates and clovy://guide for tool details.";
const BOUND_INSTRUCTIONS: &str = "Clovy tools for this host-bound chat turn. Notes, conversations, projects, dictations and memories use the conversation's data partition even if the app view changes. Activity belongs to the installation. Read clovy://context for dates and host context and clovy://guide for operations, pagination, effects and costs. Use only the tools actually listed for this turn. Mutations use the host's approval broker and revalidate authority after waits; tool configuration never grants extra access. Media tools can incur direct provider charges without a separate broker card. Status and reading a skill do not activate permissions or execute instructions.";

/// `tools/call` for one of the activity tools: the app opens the activity
/// store only for these.
pub fn calls_activity_tool(message: &Value) -> bool {
    method(message) == Some("tools/call")
        && message
            .pointer("/params/name")
            .and_then(Value::as_str)
            .is_some_and(tools::is_activity_tool)
}

/// MCP error context is host-owned. Do not forward arbitrary provider error
/// bodies or other AppError details through the relay.
fn app_tool_error(error: &crate::domain::types::AppError) -> Value {
    let mut result = tool_error(&format!("{} ({})", error.message, error.code));
    if let Some(diagnostic) = error
        .details
        .as_ref()
        .and_then(|details| {
            serde_json::from_value::<crate::agent_mcp::AgentMcpDiagnostic>(details.clone()).ok()
        })
        .filter(|diagnostic| {
            diagnostic.origin == "clovy_managed"
                && diagnostic.code == error.code
                && matches!(
                    diagnostic.phase.as_str(),
                    "connect" | "discover" | "refresh" | "call" | "disconnect"
                )
        })
    {
        result["_meta"] = json!({"clovy": {"diagnostic": diagnostic}});
    }
    result
}

fn call_tool_error(name: &str, error: &crate::domain::types::AppError) -> Value {
    if tools::is_gateway_tool(name) {
        app_tool_error(error)
    } else {
        tool_error_for(error)
    }
}

/// Answers one JSON-RPC message for a client with neither app nor turn;
/// `None` for notifications and responses.
pub async fn handle(data: &McpData, message: &Value) -> Option<Value> {
    handle_in(&ToolCtx::unbound(data), message).await
}

/// Answers one JSON-RPC message in `ctx` (the connection's turn decides
/// which tool groups exist); `None` for notifications and responses.
pub async fn handle_in(ctx: &ToolCtx<'_>, message: &Value) -> Option<Value> {
    handle_with_access(ctx, message, None).await
}

/// Handles MCP for a local peer authenticated and identified by the relay.
/// The active partition comes from the host's per-request data snapshot.
pub async fn handle_external(
    ctx: &ToolCtx<'_>,
    connection: &ExternalConnection,
    broker: &ExternalApprovalBroker,
    memory_settings: Option<&Path>,
    recording_data: Option<&Path>,
    message: &Value,
) -> Option<Value> {
    let scope = broker.scope(connection, &ctx.data.profile, recording_data);
    // An external message always has an unbound ToolCtx, even if a future
    // caller were to supply a context with the internal fields populated.
    let ctx = ToolCtx {
        app: ctx.app,
        ..ToolCtx::unbound(ctx.data)
    };
    handle_with_access(&ctx, message, Some((&scope, memory_settings))).await
}

async fn handle_with_access(
    ctx: &ToolCtx<'_>,
    message: &Value,
    external: Option<(&ExternalCall<'_>, Option<&Path>)>,
) -> Option<Value> {
    let (id, method) = match classify(message) {
        Incoming::Request { id, method } => (id, method),
        Incoming::Notification | Incoming::Response => return None,
        Incoming::Invalid { id, reason } => {
            return Some(error_response(id, INVALID_REQUEST, reason))
        }
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let outcome = match method {
        "initialize" => Ok(initialize_result(
            &params,
            if ctx.binding.is_some() {
                BOUND_INSTRUCTIONS
            } else if external.is_some() {
                EXTERNAL_INSTRUCTIONS
            } else {
                INSTRUCTIONS
            },
        )),
        "ping" => Ok(json!({})),
        "tools/list" => {
            let mut descriptors = match external {
                Some((scope, settings)) if scope.check_read_scope().is_ok() => {
                    tools::external_descriptors(ctx, scope, settings).await
                }
                _ => tools::descriptors(ctx).await,
            };
            match ctx.binding.and_then(|binding| binding.wire_names.as_ref()) {
                Some(names) => names
                    .publish(&mut descriptors)
                    .map(|()| json!({ "tools": descriptors }))
                    .map_err(|error| (SERVER_UNAVAILABLE, error.to_string())),
                None => Ok(json!({ "tools": descriptors })),
            }
        }
        "tools/call" => call_tool(ctx, &params, external).await,
        "resources/list" => Ok(json!({ "resources": resources() })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": [] })),
        "resources/read" => read_resource(ctx, &params, ctx.data.now).await,
        other => Err((METHOD_NOT_FOUND, format!("Method not found: {other}"))),
    };
    Some(match outcome {
        Ok(result) => result_response(id, result),
        Err((code, message)) => error_response(id, code, message),
    })
}

async fn call_tool(
    ctx: &ToolCtx<'_>,
    params: &Value,
    external: Option<(&ExternalCall<'_>, Option<&Path>)>,
) -> Result<Value, (i64, String)> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "`name` is required".to_string()))?;
    let resolved = match ctx.binding.and_then(|binding| binding.wire_names.as_ref()) {
        Some(names) => Some(
            names
                .resolve(name)
                .ok_or_else(|| (INVALID_PARAMS, format!("Unknown tool: {name}")))?,
        ),
        None => None,
    };
    let name = resolved.as_ref().map_or(name, AsRef::as_ref);
    if !tools::is_known_tool(name) {
        return Err((INVALID_PARAMS, format!("Unknown tool: {name}")));
    }
    let arguments = params
        .get("arguments")
        .filter(|arguments| !arguments.is_null())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let result = match external {
        Some((scope, settings)) => {
            tools::call_external(ctx, scope, settings, name, &arguments).await
        }
        None => tools::call(ctx, name, &arguments).await,
    };
    Ok(match result {
        Ok(value) => tool_result(&value),
        Err(error) => call_tool_error(name, &error),
    })
}

fn resources() -> Value {
    json!([
        {
            "uri": CONTEXT_URI,
            "name": "context",
            "title": "Current context",
            "description": "Current date, time, and time zone of the user's computer.",
            "mimeType": "application/json"
        },
        {
            "uri": GUIDE_URI,
            "name": "guide",
            "title": "Usage guide",
            "description": "Which Clovy tool answers which question, and how dates, limits, and privacy work.",
            "mimeType": "text/markdown"
        }
    ])
}

async fn read_resource(
    ctx: &ToolCtx<'_>,
    params: &Value,
    now: DateTime<Utc>,
) -> Result<Value, (i64, String)> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "`uri` is required".to_string()))?;
    let (mime_type, text) = match uri {
        CONTEXT_URI => {
            let mut context = context_json(now.with_timezone(&Local), local_time_zone_name());
            if let Some(binding) = ctx.binding {
                use sqlx::row::Row;
                let policy = ctx
                    .broker()
                    .map_err(|_| (SERVER_UNAVAILABLE, "Approval policy unavailable".into()))?
                    .effective_policy(&crate::approval_broker::ApprovalTurn::from(binding))
                    .await
                    .map_err(|e| {
                        (
                            SERVER_UNAVAILABLE,
                            format!("{} ({})", e.message(), e.code()),
                        )
                    })?;
                ctx.revalidate()
                    .await
                    .map_err(|e| (SERVER_UNAVAILABLE, e.message))?;
                let row = sqlx::query::query("SELECT COALESCE(json_extract(r.run_config_json, '$.workspace'), s.workspace_path) AS workspace_path, s.safety_mode, CASE WHEN s.safety_mode = 'unrestricted' THEN 'bypass' ELSE 'approvals' END AS approval_mode FROM agent_sessions s JOIN agent_runs r ON r.session_id = s.id WHERE s.id = ? AND r.id = ?")
                    .bind(&binding.session_id).bind(&binding.run_id).fetch_optional(&ctx.data.notes).await
                    .map_err(|_| (SERVER_UNAVAILABLE, "Session context unavailable".into()))?
                    .ok_or((SERVER_UNAVAILABLE, "Session context unavailable".into()))?;
                let folders = sqlx::query::query(
                    "SELECT folder_id FROM session_folders WHERE session_id = ? ORDER BY folder_id",
                )
                .bind(&binding.session_id)
                .fetch_all(&ctx.data.notes)
                .await
                .map_err(|_| (SERVER_UNAVAILABLE, "Project context unavailable".into()))?
                .into_iter()
                .map(|r| r.get::<String, _>("folder_id"))
                .collect::<Vec<_>>();
                ctx.revalidate()
                    .await
                    .map_err(|e| (SERVER_UNAVAILABLE, e.message))?;
                context["session"] = json!({"id": binding.session_id, "runId": binding.run_id, "engine": binding.cli.id(), "surface": binding.surface, "partition": binding.data_partition, "projectIds": folders, "workspace": row.get::<Option<String>, _>("workspace_path"), "executionPolicy": binding.safety_mode, "configuredExecutionPolicy": row.get::<String, _>("safety_mode"), "approvalMode": row.get::<String, _>("approval_mode"), "effectiveApprovalMode": policy.mode});
            }
            ("application/json", context.to_string())
        }
        GUIDE_URI => (
            "text/markdown",
            if ctx.binding.is_some() {
                format!("{GUIDE}\n{BOUND_GUIDE}")
            } else {
                GUIDE.to_string()
            },
        ),
        _ => return Err((RESOURCE_NOT_FOUND, format!("Resource not found: {uri}"))),
    };
    Ok(json!({ "contents": [{ "uri": uri, "mimeType": mime_type, "text": text }] }))
}

/// `clovy://context`: the moment in the Mac's local time. Calendar values
/// come from the local calendar (`yesterday` is the previous date, and
/// `startOfToday` is local midnight with the offset in force then), so they
/// stay right across daylight-saving changes.
pub fn context_json<Tz: TimeZone>(now: DateTime<Tz>, time_zone: Option<String>) -> Value
where
    Tz::Offset: std::fmt::Display,
{
    let offset = now.format("%:z").to_string();
    let today = now.date_naive();
    json!({
        "now": now.to_rfc3339(),
        "date": today.format("%Y-%m-%d").to_string(),
        "time": now.format("%H:%M").to_string(),
        "weekday": now.format("%A").to_string(),
        "timeZone": time_zone.unwrap_or_else(|| format!("UTC{offset}")),
        "utcOffset": offset,
        "yesterday": today.pred_opt().map(|day| day.format("%Y-%m-%d").to_string()),
        "startOfToday": start_of_day(&now.timezone(), today).map(|start| start.to_rfc3339()),
    })
}

/// The first local moment of `day`: midnight, or the first valid minute
/// after it where a daylight-saving jump skips midnight.
fn start_of_day<Tz: TimeZone>(zone: &Tz, day: NaiveDate) -> Option<DateTime<Tz>> {
    (0..=24 * 60).find_map(|minute| {
        let time = NaiveTime::from_num_seconds_from_midnight_opt(minute * 60, 0)?;
        zone.from_local_datetime(&day.and_time(time)).earliest()
    })
}

/// The IANA name of the Mac's time zone (`TZ`, else `/etc/localtime`).
fn local_time_zone_name() -> Option<String> {
    if let Some(name) = std::env::var("TZ")
        .ok()
        .map(|tz| tz.trim_start_matches(':').to_string())
        .filter(|tz| !tz.is_empty() && !tz.starts_with('/'))
    {
        return Some(name);
    }
    let target = std::fs::read_link("/etc/localtime").ok()?;
    let target = target.to_string_lossy();
    target
        .split_once("zoneinfo/")
        .map(|(_, name)| name.to_string())
        .filter(|name| !name.is_empty())
}

const GUIDE: &str = r#"# Clovy MCP guide

Clovy records meetings and dictations, turns them into notes, keeps memories, and (when activity capture is on) a text-only timeline, day summaries, and coding-agent work. External clients receive the read tools in this section, from the data partition active in Clovy. On the local relay, they can also use applicable tools from the authenticated connection’s catalog. Some tools require a host-owned turn or workspace and are unavailable. Protected changes require a one-time decision in Clovy’s MCP settings. Bound chat turns read their conversation's host-owned data partition. Activity belongs to the installation, whichever data set is open.

## Which tool

| Question | Tool |
| --- | --- |
| "What did we decide about X in the meeting?" | `search_notes` with `query`, then `get_note` with the id |
| "Show me the notes of that call" | `get_note` (long transcripts come in pages: `nextTranscriptOffset`) |
| "What did I dictate yesterday?" | `list_dictations` (kept for 7 days; `query` filters) |
| "What do you know about me / this project?" | `list_memories` (`projectId` for a project) |
| "What did I do this morning?" | `get_activity_timeline` with `from`/`to` |
| "How much did I focus today?" | `get_activity_stats` |
| "Which apps did I use most this week?" | `list_app_usage` |
| "What am I doing right now?" | `get_active_session` |
| "When did I see X on screen?" | `search_activity` with `query` |
| "Details of that session" | `get_session_detail` with the session id |
| "What did I build with Claude Code today?" | `list_coding_agent_sessions` |
| "Summarize my day" / "What did I do yesterday?" | `get_day_summary` (`date`: YYYY-MM-DD, default today) |
| "How was my day in numbers?" (focus per hour, categories, meetings, coding agents) | `get_day_panels` (`date`: YYYY-MM-DD, default today) |
| "When does Clovy write my day summary?" / "Are notifications on?" | `get_day_schedule` |

## Tips

- Read `clovy://context` first: it has today's date, the time, and the time zone. Activity tools take RFC 3339 times with an offset (`2026-10-04T09:00:00-03:00`); omit `from`/`to` for today so far. `get_day_summary` and `get_day_panels` take a date (`YYYY-MM-DD`), default today; a date before the retention period answers `state: "past_retention"`.
- Activity tools exist only while activity capture is on. Apps and sites the user excluded never appear, and nothing older than the retention period is returned.
- Timeline sessions and gaps retain their full original bounds when they overlap [from,to). Totals count only the overlap. Active state comes from the materialized ETL, updated approximately every 30 seconds; it is not an instantaneous capture watermark.
- Long results are capped: look at `truncated`, `hasMore`/`nextOffset`, or `nextTranscriptOffset`, and ask for a shorter period or the next page.
- Clovy must be open with the MCP server turned on (Settings, MCP). Otherwise every tool answers with an error saying so.
"#;

const BOUND_GUIDE: &str = r#"## Management in this bound turn

The catalog is authoritative for this connection. Routine toolsets restrict discovery and dispatch and do not create grants. A project organizes objects within a partition; knowing an ID does not grant access. Stop revokes the current run. After an approval wait, the host checks the live run, partition and target again. The approval broker decides the effective policy and records its receipt before an effect.

- Notes: list_notes/create_note/update_note/delete_note. Read get_note.revision before update_note and supply expectedRevision; note_conflict requires rereading, never blind retry. list_note_transcripts includes all sessions and legacy records, with replaced aggregates omitted by identity, not text. Continue both nextOffset and nextTextOffset. search_notes returns a matching transcript ID and snippet.
- Conversations: list_conversations/get_conversation; list_conversation_messages/search_conversation_messages page human/assistant messages in total run/sequence/ID order, excluding tool payloads. Continue nextCursor and nextTextOffset for long messages. list_conversation_artifacts/read_conversation_artifact require conversationId plus artifactId; UTF-8 pages count bytes and binary content is an explicit error. get_conversation_run/cancel_conversation_run target an exact conversation/run pair. A turn cannot cancel itself or delete its conversation. Creating/managing conversations does not start another model turn.
- Projects: list_projects/get_project/create_project/update_project/delete_project and assign/remove note/conversation project operations. Associations are plural. update_project applies the compound change atomically. delete_project requires explicit deleteNotes and previews affected data; a changed preview needs another approval.
- Routines: CRUD/pause/resume plus run_routine/list_routine_runs. Timing includes an IANA timezone, resolved CLI model and enabledToolsets. Running can return started, busy or blocked; only started means a run was actually admitted. History is paginated. Configuration never creates connector, browser or execution grants. Running a routine can invoke the configured CLI/provider and incur its normal usage.
- Memory: save_memory/update_memory/forget_memory/search_memories/list_memories. Updates preserve identity, source, creation and scope. Disabled or deleted projects are filtered before paging. Changing scope is not supported.
- Activity: get_activity_settings/update_activity_settings/pause_activity/resume_activity/generate_day_summary. Settings work while capture is off and cannot alter internal MCP/default fields. Generation requires local day (YYYY-MM-DD), may call the configured activity provider and preserves provider, empty-data, database and pending-hour errors. These settings belong to the installation. No manual category editing or database rebuild is offered.
- Dictation: list/get/delete_dictation and start_dictation/get_dictation_status/stop_dictation/cancel_dictation. Start pins a take, destination composer and window; status confirms actual helper state. Stop delivers to that same destination; cancel discards. No arbitrary target app or helper command passthrough is accepted. Audio may use the user's configured transcription provider; cleanup may incur OpenRouter usage.
- Recorder: start_recording/recording_status/stop_recording/pause_recording/resume_recording/recover_recording/retry_note_processing/list_recording_artifacts. IDs, source mode and duration refer to the actual session. Recovery is explicitly validate or discard. Finalizing/retrying can use configured transcription and note providers; listing metadata does not grant paths. Recording controls use the real main-window bridge and may need native permissions.
- Skills: list_skills/load_skill describe active skills; list_skill_settings/read_skill also inspect disabled settings for administration. update_skill/enable_skill/disable_skill use approval. Only managed skills are editable; global roots are read-only. Reading presence or content does not execute instructions. Hidden entries and objective editor backups are excluded; legitimate foo-backup names remain valid.

Reads above are local and do not activate capture, login, permissions or a feature. Native mutations and browser input use the approval broker's effective policy; clarification always requires an answer. Browser ownership, network and target guards also apply. Media generation/editing is the existing exception: direct provider usage can be charged without another broker card. Status is not authorization. Inspect error codes and delivery state; never resend an effect whose delivery is uncertain.
"#;

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    use crate::{agent_mcp::AgentMcpError, domain::types::AppError};

    #[test]
    fn relay_retains_host_diagnostic_but_discards_arbitrary_details() {
        let error = AgentMcpError::Remote(-32601)
            .diagnostic("private", Some("lookup"), "call")
            .into_app_error();
        let result = app_tool_error(&error);
        assert_eq!(result["isError"], true);
        let diagnostic = &result["_meta"]["clovy"]["diagnostic"];
        assert_eq!(diagnostic["server"], "private");
        assert_eq!(diagnostic["remoteCode"], -32601);
        assert_eq!(diagnostic["phase"], "call");
        let mut unrelated = AppError::new("provider_failure", "Provider unavailable");
        unrelated.details = Some(json!({"authorization": "fixture-private-payload"}));
        let result = app_tool_error(&unrelated);
        assert!(result.get("_meta").is_none());
        assert!(!result.to_string().contains("fixture-private-payload"));
        let result = call_tool_error("mcp_example_read", &unrelated);
        assert!(!result.to_string().contains("fixture-private-payload"));

        let mut native = AppError::new("turn_expired", "Turn expired");
        native.details = Some(json!({"retrySafe": false}));
        let result = call_tool_error("computer_use", &native);
        assert_eq!(
            result["structuredContent"]["error"]["details"]["retrySafe"],
            false
        );
        let result = call_tool_error("mcp_example_read", &error);
        assert_eq!(result["_meta"]["clovy"]["diagnostic"]["remoteCode"], -32601);
    }
}
