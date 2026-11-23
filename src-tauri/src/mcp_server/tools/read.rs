//! The read group: read-only views of the current data partition's notes,
//! dictations, and memories, and (only while capture is on) of the
//! installation's activity timeline and coding-agent blocks (activity has no
//! partition, ADR-0059). Offered to every connection, bound to a turn or
//! not. Names are fixed in the `clovy-mcp-server` spec of the OpenSpec change
//! `add-activity-intelligence`; `docs/mcp-server.md` lists them with their
//! arguments and limits.

use chrono::{DateTime, Duration, Local, NaiveDate, Utc};
use serde_json::{json, Value};
use sqlx::{query::query, row::Row};
use sqlx_sqlite::SqlitePool;

use crate::activity::settings::ActivitySettings;
use crate::activity::store::ActivityStore;
use crate::activity::timeline::agent_tools::{
    self, gap_json, local, local_at, minutes, range, session_json, stats_json,
};
use crate::activity::timeline::stats::TimelineStatsDto;
use crate::activity::timeline::{
    retention_floor, stats, timeline_view, visible_session_detail, TimelineView,
};
use crate::coding_agents::settings::CodingAgentSources;
use crate::day_intelligence::pipeline::{self, ReadContext};
use crate::day_intelligence::schedule::{LocalZone, SystemZone};
use crate::day_intelligence::sources::{
    coding_blocks_between, meetings_between, CodingBlock, DaySources, MeetingNote,
};
use crate::db::repositories::{
    dictation_history_cutoff_timestamp, Repositories, DICTATION_HISTORY_RETENTION_DAYS,
};
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;

use super::{ActivityAccess, McpData, ToolCtx, ToolGroup};

pub const SEARCH_NOTES: &str = "search_notes";
pub const GET_NOTE: &str = "get_note";
pub const LIST_NOTE_TRANSCRIPTS: &str = "list_note_transcripts";
pub const LIST_DICTATIONS: &str = "list_dictations";
pub const LIST_MEMORIES: &str = "list_memories";
pub const GET_ACTIVITY_STATS: &str = "get_activity_stats";
pub const GET_ACTIVE_SESSION: &str = "get_active_session";
pub const LIST_APP_USAGE: &str = "list_app_usage";
pub const GET_SESSION_DETAIL: &str = "get_session_detail";
pub const LIST_CODING_AGENT_SESSIONS: &str = "list_coding_agent_sessions";
pub const GET_DAY_SUMMARY: &str = "get_day_summary";
pub const GET_DAY_PANELS: &str = "get_day_panels";
pub const GET_DAY_SCHEDULE: &str = "get_day_schedule";

/// Activity tools, in catalog order.
pub const ACTIVITY_TOOLS: [&str; 10] = [
    agent_tools::GET_ACTIVITY_TIMELINE,
    agent_tools::SEARCH_ACTIVITY,
    GET_ACTIVITY_STATS,
    GET_ACTIVE_SESSION,
    LIST_APP_USAGE,
    GET_SESSION_DETAIL,
    LIST_CODING_AGENT_SESSIONS,
    GET_DAY_SUMMARY,
    GET_DAY_PANELS,
    GET_DAY_SCHEDULE,
];
const DATA_TOOLS: [&str; 5] = [
    SEARCH_NOTES,
    GET_NOTE,
    LIST_NOTE_TRANSCRIPTS,
    LIST_DICTATIONS,
    LIST_MEMORIES,
];

const SNIPPET_RADIUS_CHARS: usize = 160;
const NOTE_CONTENT_MAX_CHARS: usize = 20_000;
const TRANSCRIPT_PAGE_CHARS: usize = 40_000;
const DICTATION_MAX_CHARS: usize = 4_000;
const MEMORY_MAX_CHARS: usize = 4_000;
const DEFAULT_APP_USAGE: u64 = 20;
const MAX_APP_USAGE: u64 = 100;
const DEFAULT_CODING_AGENT_BLOCKS: u64 = 20;
const MAX_CODING_AGENT_BLOCKS: u64 = 50;
const MAX_DAY_WORKSTREAMS: usize = 20;
const MAX_DAY_HOUR_REPORTS: usize = 24;
const MAX_WORKSTREAM_HOURS: usize = 24;
const MAX_HOUR_ACTIVITIES: usize = 20;
const DAY_SUMMARY_TEXT_MAX_CHARS: usize = 4_000;
const DAY_SUMMARY_NARRATIVE_MAX_CHARS: usize = 20_000;
/// How far back `get_active_session` looks for the session going on now.
const ACTIVE_LOOKBACK_HOURS: i64 = 12;

pub fn is_activity_tool(name: &str) -> bool {
    ACTIVITY_TOOLS.contains(&name)
}

pub(crate) fn is_read_tool(name: &str) -> bool {
    DATA_TOOLS.contains(&name) || is_activity_tool(name)
}

/// The tools of this file as a [`ToolGroup`].
pub struct ReadTools;

impl ToolGroup for ReadTools {
    fn needs_turn(&self) -> bool {
        false
    }

    fn owns(&self, name: &str) -> bool {
        is_read_tool(name)
    }

    fn descriptors(&self, ctx: &ToolCtx<'_>) -> Vec<Value> {
        descriptors(&ctx.data.activity)
    }

    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let mut catalog = Vec::new();
            for descriptor in self.descriptors(ctx) {
                let Some(name) = descriptor["name"].as_str() else {
                    continue;
                };
                match routine_read_allowed(ctx, name).await {
                    Ok(true) => catalog.push(descriptor),
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(code = %error.code, "Read tool permissions unavailable");
                    }
                }
            }
            catalog
        })
    }

    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            // Catalogs are hints, not grants: recheck before arguments or
            // data access in case the owner revoked the routine's toolset.
            if !routine_read_allowed(ctx, name).await? {
                return Err(AppError::new(
                    "routine_tool_not_enabled",
                    "This tool is not enabled for the routine.",
                ));
            }
            call(ctx.data, name, arguments).await
        })
    }
}

async fn routine_read_allowed(ctx: &ToolCtx<'_>, name: &str) -> Result<bool, AppError> {
    let Some(binding) = ctx.binding else {
        return Ok(true);
    };
    super::routine_tool_allowed(&ctx.data.notes, binding, name).await
}

fn read_only(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": { "readOnlyHint": true, "openWorldHint": false, "idempotentHint": true },
    })
}

fn range_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "from": { "type": "string", "description": "Start, RFC 3339 with offset (default: local midnight today)." },
            "to": { "type": "string", "description": "End, RFC 3339 with offset (default: now)." }
        },
        "additionalProperties": false
    })
}

/// The `tools/list` catalog: data tools always, activity tools only while
/// capture is on.
fn descriptors(activity: &ActivityAccess) -> Vec<Value> {
    let mut tools = vec![
        read_only(
            SEARCH_NOTES,
            "Search the user's Clovy meeting notes (titles, note text, and transcripts) for a word or phrase. Returns each matching note's id, title, a snippet around the match, and dates, newest first. Use get_note with the id for the full note.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Word or phrase to find (case-insensitive)." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 10 }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        ),
        read_only(
            GET_NOTE,
            "Read one Clovy note by id, including its authoritative transcripts. Long text comes in character pages.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string" },
                    "contentOffset": { "type": "integer", "minimum": 0, "default": 0, "description": "Character offset into the note content." },
                    "transcriptOffset": { "type": "integer", "minimum": 0, "default": 0, "description": "Character offset into the transcript." }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        read_only(LIST_NOTE_TRANSCRIPTS, "List authoritative transcript records for a note, in recording order. Page long record text with textOffset.", json!({"type":"object","properties":{"id":{"type":"string"},"offset":{"type":"integer","minimum":0},"textOffset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":50}},"required":["id"],"additionalProperties":false})),
        read_only(
            LIST_DICTATIONS,
            "List the user's recent Clovy dictations (kept for 7 days), newest first, optionally only those containing `query`.",
            json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 20 },
                    "offset": { "type": "integer", "minimum": 0, "default": 0 }
                },
                "additionalProperties": false
            }),
        ),
        read_only(
            LIST_MEMORIES,
            "List what Clovy remembers about the user (global memories and, with `projectId`, that project's), newest first.",
            json!({
                "type": "object",
                "properties": {
                    "projectId": { "type": "string" },
                    "includeGlobal": { "type": "boolean", "default": true },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 20, "default": 8 },
                    "offset": { "type": "integer", "minimum": 0, "default": 0 }
                },
                "additionalProperties": false
            }),
        ),
    ];
    if !activity.offered() {
        return tools;
    }
    for descriptor in agent_tools::descriptors(true) {
        tools.push(read_only(
            descriptor["name"].as_str().unwrap_or_default(),
            descriptor["description"].as_str().unwrap_or_default(),
            descriptor["parameters"].clone(),
        ));
    }
    tools.extend([
        read_only(
            GET_ACTIVITY_STATS,
            "Totals for a period of the user's activity: focused, idle, and away minutes, the top apps, and time per category. Omit both bounds for today so far.",
            range_schema(),
        ),
        read_only(
            GET_ACTIVE_SESSION,
            "What the user is doing right now: the app session in progress (app, browser domain or editor workspace, window, category, minutes) or the idle/away gap going on, and the last session when nothing is active.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
        read_only(
            LIST_APP_USAGE,
            "Time per app for a period, most used first, with the number of sessions. Omit both bounds for today so far.",
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Start, RFC 3339 with offset (default: local midnight today)." },
                    "to": { "type": "string", "description": "End, RFC 3339 with offset (default: now)." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_APP_USAGE, "default": DEFAULT_APP_USAGE }
                },
                "additionalProperties": false
            }),
        ),
        read_only(
            GET_SESSION_DETAIL,
            "One activity session in detail: the windows and URLs seen in it and an excerpt of its captured text. Session ids come from get_activity_timeline and search_activity.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "integer" } },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        read_only(
            LIST_CODING_AGENT_SESSIONS,
            "Work the user did with coding agents (Claude Code, Codex, Copilot, Cursor, Antigravity) in a period, newest first. Each entry is one block of a session (a session splits into blocks after an hour of silence or every hour of work): agent, project, title, times, prompt and reply counts, active minutes, and Clovy's short summary when it has one (else the first prompt). Only agents the user reads in Clovy Settings, Activity. Omit both bounds for today so far.",
            json!({
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Start, RFC 3339 with offset (default: local midnight today)." },
                    "to": { "type": "string", "description": "End, RFC 3339 with offset (default: now)." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_CODING_AGENT_BLOCKS, "default": DEFAULT_CODING_AGENT_BLOCKS }
                },
                "additionalProperties": false
            }),
        ),
        read_only(
            GET_DAY_SUMMARY,
            "The day's summary: headline, narrative, insights, standup (done, in-progress, blockers), workstreams, and hour reports. Omit `date` for today.",
            json!({
                "type": "object",
                "properties": {
                    "date": {
                        "type": "string",
                        "description": "Local date in YYYY-MM-DD format (default: today)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        read_only(
            GET_DAY_PANELS,
            "The numbers next to the day's summary, read from the activity now: focused, idle, and away minutes, minutes per category, top apps, focused minutes per hour, minutes per workstream, recorded meetings (count and minutes), and coding-agent blocks with their active minutes inside the day. Omit `date` for today.",
            json!({
                "type": "object",
                "properties": {
                    "date": {
                        "type": "string",
                        "description": "Local date in YYYY-MM-DD format (default: today)."
                    }
                },
                "additionalProperties": false
            }),
        ),
        read_only(
            GET_DAY_SCHEDULE,
            "When Clovy writes the day summary on its own (local HH:MM) and how it notifies: whether activity notifications are on and the quiet hours during which they wait.",
            json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        ),
    ]);
    tools
}

/// Runs a known tool. Errors become `isError` tool results.
async fn call(data: &McpData, name: &str, arguments: &Value) -> Result<Value, AppError> {
    match name {
        SEARCH_NOTES => search_notes(data, arguments).await,
        GET_NOTE => get_note(data, arguments).await,
        LIST_NOTE_TRANSCRIPTS => list_note_transcripts(data, arguments).await,
        LIST_DICTATIONS => list_dictations(data, arguments).await,
        LIST_MEMORIES => list_memories(data, arguments).await,
        name if is_activity_tool(name) => {
            let (store, settings) = match &data.activity {
                ActivityAccess::Off => return Err(AppError::new(
                    "activity_capture_off",
                    "Activity capture is off. The user can turn it on in Clovy Settings, Activity.",
                )),
                ActivityAccess::On => {
                    return Err(AppError::new(
                        "activity_database_closed",
                        "The activity history is not available right now.",
                    ))
                }
                ActivityAccess::Unavailable(error) => return Err(error.clone()),
                ActivityAccess::Open { store, settings } => (store, settings),
            };
            activity_tool(data, store, settings, name, arguments).await
        }
        _ => Err(AppError::new(
            "mcp_tool_unknown",
            format!("Unknown tool `{name}`."),
        )),
    }
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("mcp_invalid_arguments", message.into())
}

fn required_text<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, AppError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid(format!("`{key}` is required")))
}

fn optional_text<'a>(arguments: &'a Value, key: &str) -> Option<&'a str> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn bounded(arguments: &Value, key: &str, default: u64, min: u64, max: u64) -> u64 {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(min, max)
}

fn like_pattern(text: &str) -> String {
    format!(
        "%{}%",
        text.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

/// The first `max` characters, and whether anything was cut.
fn clip(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((end, _)) => (text[..end].to_string(), true),
        None => (text.to_string(), false),
    }
}

/// About `SNIPPET_RADIUS_CHARS` characters on each side of the first
/// case-insensitive match of `needle`; `None` when it does not occur.
fn snippet(text: &str, needle: &str) -> Option<String> {
    let haystack: Vec<char> = text.chars().collect();
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    let lower = |c: char| c.to_lowercase().next().unwrap_or(c);
    let start = (0..=haystack.len() - needle.len()).find(|&start| {
        needle
            .iter()
            .enumerate()
            .all(|(offset, expected)| lower(haystack[start + offset]) == *expected)
    })?;
    let from = start.saturating_sub(SNIPPET_RADIUS_CHARS);
    let to = (start + needle.len() + SNIPPET_RADIUS_CHARS).min(haystack.len());
    let mut excerpt: String = haystack[from..to].iter().collect();
    excerpt = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
    if from > 0 {
        excerpt.insert_str(0, "...");
    }
    if to < haystack.len() {
        excerpt.push_str("...");
    }
    Some(excerpt)
}

async fn search_notes(data: &McpData, arguments: &Value) -> Result<Value, AppError> {
    let needle = required_text(arguments, "query")?;
    let limit = bounded(arguments, "limit", 10, 1, 20);
    let pattern = like_pattern(needle);
    let rows = query(&format!(
        "SELECT n.id, n.title, COALESCE(n.edited_content, n.generated_content, '') AS note,
                COALESCE(n.generated_content, '') AS generated,
                COALESCE(hit.text, '') AS transcript, hit.id AS transcript_id, n.created_at, n.updated_at
         FROM notes n LEFT JOIN transcripts hit ON hit.id = (SELECT t.id FROM transcripts t WHERE t.note_id = n.id AND t.text LIKE ? ESCAPE '\\' AND {} ORDER BY t.created_at, t.id LIMIT 1)
         WHERE n.profile = ? AND (
            n.title LIKE ? ESCAPE '\\' OR n.generated_content LIKE ? ESCAPE '\\'
            OR n.edited_content LIKE ? ESCAPE '\\'
            OR EXISTS (SELECT 1 FROM transcripts t WHERE t.note_id = n.id AND t.text LIKE ? ESCAPE '\\' AND {})
         )
         ORDER BY n.updated_at DESC, n.id DESC LIMIT ?", super::notes::AUTHORITATIVE_TRANSCRIPT, super::notes::AUTHORITATIVE_TRANSCRIPT
    ))
    .bind(&pattern)
    .bind(&data.profile)
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit as i64)
    .fetch_all(&data.notes)
    .await?;
    let notes: Vec<Value> = rows
        .into_iter()
        .map(|row| {
            let title: String = row.get("title");
            let note: String = row.get("note");
            let generated: String = row.get("generated");
            let transcript: String = row.get("transcript");
            let (matched_in, excerpt) = if let Some(excerpt) = snippet(&note, needle) {
                ("note", excerpt)
            } else if let Some(excerpt) = snippet(&generated, needle) {
                ("note", excerpt)
            } else if let Some(excerpt) = snippet(&transcript, needle) {
                ("transcript", excerpt)
            } else if let Some(excerpt) = snippet(&title, needle) {
                ("title", excerpt)
            } else {
                ("transcript", clip(&note, SNIPPET_RADIUS_CHARS * 2).0)
            };
            json!({
                "id": row.get::<String, _>("id"),
                "title": title,
                "snippet": excerpt,
                "matchedIn": matched_in,
                "transcriptId": if matched_in == "transcript" { row.get::<Option<String>, _>("transcript_id") } else { None },
                "createdAt": row.get::<String, _>("created_at"),
                "updatedAt": row.get::<String, _>("updated_at"),
            })
        })
        .collect();
    Ok(json!({ "count": notes.len(), "notes": notes }))
}

async fn get_note(data: &McpData, arguments: &Value) -> Result<Value, AppError> {
    let id = required_text(arguments, "id")?;
    let offset = super::notes::character_offset(arguments, "transcriptOffset")?;
    let row = super::notes::read_note(&data.notes, &data.profile, id).await?;
    let content_offset = super::notes::character_offset(arguments, "contentOffset")?;
    let note: String = row.get("note");
    let (content, content_total, next_content_offset) =
        super::notes::character_page(&note, content_offset, NOTE_CONTENT_MAX_CHARS);
    let content_truncated = content.chars().count() < content_total;
    let transcript = super::notes::authoritative_transcripts(&data.notes, id, &data.profile, 0, -1)
        .await?
        .into_iter()
        .map(|row| row.get::<String, _>("text"))
        .collect::<Vec<_>>()
        .join("\n");
    let total = transcript.chars().count();
    let page: String = transcript
        .chars()
        .skip(offset)
        .take(TRANSCRIPT_PAGE_CHARS)
        .collect();
    let end = offset.saturating_add(TRANSCRIPT_PAGE_CHARS);
    Ok(json!({
        "id": row.get::<String, _>("id"),
        "title": row.get::<String, _>("title"),
        "createdAt": row.get::<String, _>("created_at"),
        "updatedAt": row.get::<String, _>("updated_at"),
        "revision": row.get::<i64, _>("revision"),
        "content": content,
        "contentTruncated": content_truncated,
        "contentOffset": content_offset,
        "contentTotalChars": content_total,
        "nextContentOffset": next_content_offset,
        "transcript": page,
        "transcriptOffset": offset,
        "transcriptTotalChars": total,
        "nextTranscriptOffset": (end < total).then_some(end),
    }))
}

async fn list_note_transcripts(data: &McpData, arguments: &Value) -> Result<Value, AppError> {
    let id = required_text(arguments, "id")?;
    super::notes::read_note(&data.notes, &data.profile, id).await?;
    let offset = super::notes::character_offset(arguments, "offset")?;
    let text_offset = super::notes::character_offset(arguments, "textOffset")?;
    let limit = bounded(arguments, "limit", 20, 1, 50) as usize;
    let sql_offset = i64::try_from(offset).unwrap_or(i64::MAX);
    let rows = super::notes::authoritative_transcripts(
        &data.notes,
        id,
        &data.profile,
        sql_offset,
        (limit + 1) as i64,
    )
    .await?;
    let mut items = Vec::new();
    let mut next_offset = None;
    let mut next_text_offset = None;
    for (index, row) in rows.iter().enumerate().take(limit) {
        let start = if index == 0 { text_offset } else { 0 };
        let (text, total, next) = super::notes::character_page(
            &row.get::<String, _>("text"),
            start,
            TRANSCRIPT_PAGE_CHARS,
        );
        items.push(json!({"id":row.get::<String,_>("id"),"recordingSessionId":row.get::<Option<String>,_>("recording_session_id"),"source":row.get::<Option<String>,_>("source"),"startMs":row.get::<Option<i64>,_>("start_ms"),"endMs":row.get::<Option<i64>,_>("end_ms"),"turnIndex":row.get::<Option<i64>,_>("turn_index"),"status":row.get::<String,_>("status"),"lastError":row.get::<Option<String>,_>("last_error"),"createdAt":row.get::<String,_>("created_at"),"text":text,"textOffset":start,"textTotalChars":total}));
        if let Some(next) = next {
            next_offset = Some(offset.saturating_add(index));
            next_text_offset = Some(next);
            break;
        }
        next_offset = (index + 1 < rows.len()).then_some(offset.saturating_add(index + 1));
    }
    Ok(json!({"items":items,"nextOffset":next_offset,"nextTextOffset":next_text_offset}))
}

async fn list_dictations(data: &McpData, arguments: &Value) -> Result<Value, AppError> {
    let limit = bounded(arguments, "limit", 20, 1, 50);
    let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0);
    let needle = optional_text(arguments, "query");
    let rows = query(
        "SELECT id, text, language, created_at FROM dictation_history
         WHERE profile = ? AND created_at >= ? AND (? IS NULL OR text LIKE ? ESCAPE '\\')
         ORDER BY created_at DESC, rowid DESC LIMIT ? OFFSET ?",
    )
    .bind(&data.profile)
    .bind(dictation_history_cutoff_timestamp())
    .bind(needle)
    .bind(needle.map(like_pattern))
    .bind((limit + 1) as i64)
    .bind(i64::try_from(offset).unwrap_or(i64::MAX))
    .fetch_all(&data.notes)
    .await?;
    let has_more = rows.len() as u64 > limit;
    let items: Vec<Value> = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| {
            let (text, truncated) = clip(&row.get::<String, _>("text"), DICTATION_MAX_CHARS);
            json!({
                "id": row.get::<String, _>("id"),
                "text": text,
                "truncated": truncated,
                "language": row.get::<Option<String>, _>("language"),
                "createdAt": row.get::<String, _>("created_at"),
            })
        })
        .collect();
    Ok(json!({
        "count": items.len(),
        "items": items,
        "offset": offset,
        "hasMore": has_more,
        "nextOffset": has_more.then_some(offset + limit),
        "retentionDays": DICTATION_HISTORY_RETENTION_DAYS,
    }))
}

async fn list_memories(data: &McpData, arguments: &Value) -> Result<Value, AppError> {
    if !data.memory_enabled {
        return Err(AppError::new(
            "memory_disabled",
            "Memory is turned off in Clovy Settings.",
        ));
    }
    let project_id = optional_text(arguments, "projectId");
    if let Some(project_id) = project_id {
        let row = query(
            "SELECT memory_disabled FROM folders WHERE id = ? AND profile = ? AND deleted_at IS NULL",
        )
        .bind(project_id)
        .bind(&data.profile)
        .fetch_optional(&data.notes)
        .await?
        .ok_or_else(|| AppError::new("folder_not_found", "Project was not found."))?;
        if row.get::<i64, _>("memory_disabled") != 0 {
            return Err(AppError::new(
                "memory_disabled",
                "Memory is turned off for this project.",
            ));
        }
    }
    let include_global = arguments
        .get("includeGlobal")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let limit = bounded(arguments, "limit", 8, 1, 20) as usize;
    let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let mut memories = Repositories::new(data.notes.clone())
        .list_memories(&data.profile, project_id, include_global)
        .await?;
    let enabled_projects: Vec<String> = sqlx::query_scalar::query_scalar(
        "SELECT id FROM folders WHERE profile = ? AND deleted_at IS NULL AND memory_disabled = 0",
    )
    .bind(&data.profile)
    .fetch_all(&data.notes)
    .await?;
    // Filter before counting or paging; a disabled project's memory is not
    // disclosed by a global search and must not consume a page slot.
    memories.retain(|memory| {
        memory
            .folder_id
            .as_ref()
            .map_or(true, |id| enabled_projects.contains(id))
    });
    let has_more = memories.len() > offset.saturating_add(limit);
    let items: Vec<Value> = memories
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|memory| {
            json!({
                "id": memory.id,
                "content": clip(&memory.content, MEMORY_MAX_CHARS).0,
                "createdAt": memory.created_at,
                "updatedAt": memory.updated_at,
                "source": memory.source,
                "projectId": memory.folder_id,
                "scope": if memory.folder_id.is_some() { "project" } else { "global" },
            })
        })
        .collect();
    Ok(json!({
        "count": items.len(),
        "items": items,
        "offset": offset,
        "hasMore": has_more,
        "nextOffset": has_more.then_some(offset + items.len()),
    }))
}

async fn activity_tool(
    data: &McpData,
    store: &ActivityStore,
    settings: &ActivitySettings,
    name: &str,
    arguments: &Value,
) -> Result<Value, AppError> {
    let now = data.now;
    if let Some(result) = agent_tools::dispatch(store, settings, name, arguments, now).await {
        return result;
    }
    match name {
        GET_ACTIVITY_STATS => {
            let (from, to) = range(arguments, &SystemZone, now)?;
            let view = view(store, settings, from, to, now).await?;
            let mut value = stats_json(&view.stats);
            value["from"] = json!(local_at(from));
            value["to"] = json!(local_at(to));
            Ok(value)
        }
        GET_ACTIVE_SESSION => active_session(store, settings, now).await,
        LIST_APP_USAGE => app_usage(store, settings, arguments, now).await,
        GET_SESSION_DETAIL => session_detail(store, settings, arguments, now).await,
        LIST_CODING_AGENT_SESSIONS => coding_agent_sessions(store, settings, arguments, now).await,
        GET_DAY_SUMMARY => day_summary(store, settings, arguments, now).await,
        GET_DAY_PANELS => day_panels(data, store, settings, arguments).await,
        GET_DAY_SCHEDULE => Ok(day_schedule(settings)),
        _ => Err(AppError::new(
            "mcp_tool_unknown",
            format!("Unknown activity tool `{name}`."),
        )),
    }
}

async fn view(
    store: &ActivityStore,
    settings: &ActivitySettings,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<TimelineView, AppError> {
    timeline_view(store, settings, from, to, now)
        .await
        .map_err(|error| AppError::new("activity_timeline_failed", error.to_string()))
}

async fn active_session(
    store: &ActivityStore,
    settings: &ActivitySettings,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let view = view(
        store,
        settings,
        now - Duration::hours(ACTIVE_LOOKBACK_HOURS),
        now + Duration::minutes(1),
        now,
    )
    .await?;
    let active = view.sessions.iter().rev().find(|session| session.active);
    let ongoing_gap = view.gaps.iter().rev().find(|gap| gap.ongoing);
    Ok(json!({
        "now": local_at(now),
        "session": active.map(session_json),
        "ongoingGap": ongoing_gap.map(gap_json),
        "lastSession": if active.is_none() { view.sessions.last().map(session_json) } else { None },
    }))
}

async fn app_usage(
    store: &ActivityStore,
    settings: &ActivitySettings,
    arguments: &Value,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let (from, to) = range(arguments, &SystemZone, now)?;
    let limit = bounded(arguments, "limit", DEFAULT_APP_USAGE, 1, MAX_APP_USAGE) as usize;
    let view = view(store, settings, from, to, now).await?;
    let mut apps: Vec<(String, i64, u32)> = Vec::new();
    for session in &view.sessions {
        let ms = stats::clipped_ms(&session.started_at, &session.ended_at, from, to);
        if ms == 0 {
            continue;
        }
        match apps.iter_mut().find(|(app, _, _)| *app == session.app_name) {
            Some((_, total, sessions)) => {
                *total += ms;
                *sessions += 1;
            }
            None => apps.push((session.app_name.clone(), ms, 1)),
        }
    }
    apps.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let total_apps = apps.len();
    Ok(json!({
        "from": local_at(from),
        "to": local_at(to),
        "apps": apps.iter().take(limit).map(|(app, ms, sessions)| json!({
            "app": app,
            "minutes": minutes(*ms),
            "sessions": sessions,
        })).collect::<Vec<_>>(),
        "totalApps": total_apps,
        "truncated": total_apps > limit,
    }))
}

async fn session_detail(
    store: &ActivityStore,
    settings: &ActivitySettings,
    arguments: &Value,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let id = arguments
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("`id` must be a session id"))?;
    let missing = || {
        AppError::new(
            "activity_session_not_found",
            "This session is not in the activity history.",
        )
    };
    let detail = visible_session_detail(store, settings, id, now)
        .await
        .map_err(|error| AppError::new("activity_session_failed", error.to_string()))?
        .ok_or_else(missing)?;
    let windows: Vec<Value> = detail
        .windows
        .iter()
        .map(|window| {
            json!({
                "windowTitle": window.window_title,
                "url": window.browser_url,
                "firstSeenAt": local(&window.first_seen_at),
                "lastSeenAt": local(&window.last_seen_at),
                "frames": window.frame_count,
            })
        })
        .collect();
    Ok(json!({
        "session": session_json(&detail.session),
        "windows": windows,
        "textExcerpt": detail.text_excerpt,
    }))
}

/// Coding-agent blocks overlapping the period, newest first, from the
/// sources still turned on and within the retention period.
async fn coding_agent_sessions(
    store: &ActivityStore,
    settings: &ActivitySettings,
    arguments: &Value,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let (from, to) = range(arguments, &SystemZone, now)?;
    let limit = bounded(
        arguments,
        "limit",
        DEFAULT_CODING_AGENT_BLOCKS,
        1,
        MAX_CODING_AGENT_BLOCKS,
    ) as usize;
    let floor = retention_floor(settings, now);
    let mut blocks = store
        .enabled_coding_agent_blocks_between(&settings.coding_agents, from.max(floor), to)
        .await
        .map_err(|error| AppError::new("coding_agents_read_failed", error.to_string()))?;
    blocks.reverse();
    let total = blocks.len();
    Ok(json!({
        "from": local_at(from),
        "to": local_at(to),
        "blocks": blocks.iter().take(limit).map(|block| json!({
            "id": block.id,
            "agent": block.source.display_name(),
            "source": block.source.as_str(),
            "sessionId": block.session_id,
            "project": block.project,
            "title": block.title,
            "startedAt": local(&block.started_at),
            "endedAt": local(&block.ended_at),
            "activeMinutes": minutes(block.active_seconds * 1000),
            "prompts": block.prompt_count,
            "replies": block.reply_count,
            "state": block.state.as_str(),
            "summary": block.summary,
            "firstPrompt": if block.summary.is_none() { block.first_prompt.as_deref() } else { None },
        })).collect::<Vec<_>>(),
        "totalBlocks": total,
        "truncated": total > limit,
    }))
}

fn parse_day_argument(arguments: &Value, now: DateTime<Utc>) -> Result<NaiveDate, AppError> {
    match arguments.get("date") {
        None | Some(Value::Null) => Ok(now.with_timezone(&Local).date_naive()),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Ok(now.with_timezone(&Local).date_naive())
            } else {
                NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                    .map_err(|_| invalid("`date` must be a valid date in YYYY-MM-DD format"))
            }
        }
        Some(_) => Err(invalid("`date` must be a string in YYYY-MM-DD format")),
    }
}

/// Whether `day` ends before the retention floor: its activity is gone.
fn past_retention(settings: &ActivitySettings, day: NaiveDate, now: DateTime<Utc>) -> bool {
    day < retention_floor(settings, now)
        .with_timezone(&Local)
        .date_naive()
}

async fn day_summary(
    store: &ActivityStore,
    settings: &ActivitySettings,
    arguments: &Value,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let target_date = parse_day_argument(arguments, now)?;
    let day_key = target_date.format("%Y-%m-%d").to_string();

    if past_retention(settings, target_date, now) {
        return Ok(json!({
            "date": day_key,
            "state": "past_retention",
            "reason": "The requested date is past the activity retention period.",
            "summary": Value::Null,
            "workstreams": [],
            "hourReports": [],
            "totalWorkstreams": 0,
            "totalHourReports": 0,
            "truncated": false,
        }));
    }

    let summary = crate::day_intelligence::db::summary_of_day(store, &day_key)
        .await
        .map_err(|error| AppError::new("activity_day_summary_failed", error.to_string()))?;
    let workstreams = crate::day_intelligence::db::workstreams_of_day(store, &day_key)
        .await
        .map_err(|error| AppError::new("activity_day_summary_failed", error.to_string()))?;
    let hour_reports = crate::day_intelligence::db::hour_reports_of_day(store, &day_key)
        .await
        .map_err(|error| AppError::new("activity_day_summary_failed", error.to_string()))?;

    let (summary_value, summary_truncated) = match summary {
        Some(s) => {
            let (narrative, narr_trunc) = clip(&s.narrative, DAY_SUMMARY_NARRATIVE_MAX_CHARS);
            let (headline, head_trunc) = clip(&s.headline, 500);
            let mut val = serde_json::to_value(&s).unwrap_or_default();
            if narr_trunc {
                val["narrative"] = json!(narrative);
            }
            if head_trunc {
                val["headline"] = json!(headline);
            }
            let is_trunc = narr_trunc || head_trunc;
            val["truncated"] = json!(is_trunc);
            (val, is_trunc)
        }
        None => (Value::Null, false),
    };

    let total_workstreams = workstreams.len();
    let mut workstreams_truncated = total_workstreams > MAX_DAY_WORKSTREAMS;
    let workstreams_json: Vec<Value> = workstreams
        .into_iter()
        .take(MAX_DAY_WORKSTREAMS)
        .map(|ws| {
            let (title, title_trunc) = clip(&ws.title, 500);
            let (summary, summary_trunc) = clip(&ws.summary, DAY_SUMMARY_TEXT_MAX_CHARS);
            let total_hours = ws.hours.len();
            let hours_truncated = total_hours > MAX_WORKSTREAM_HOURS;
            let mut any_hour_note_trunc = false;
            let hours_json: Vec<Value> = ws
                .hours
                .into_iter()
                .take(MAX_WORKSTREAM_HOURS)
                .map(|h| {
                    let (note, note_trunc) = clip(&h.note, 500);
                    if note_trunc {
                        any_hour_note_trunc = true;
                    }
                    json!({
                        "hour": h.hour,
                        "minutes": h.minutes,
                        "seconds": h.seconds,
                        "note": note,
                        // Bounds of the measured sessions the hour's work was
                        // located in (where, not how long); null when only the
                        // hour is known. `locatedSeconds` of `seconds` are
                        // covered by them; rounded minutes remain available.
                        "periods": h.periods,
                        "locatedMinutes": h.located_minutes,
                        "locatedSeconds": h.located_seconds,
                        "truncated": note_trunc,
                    })
                })
                .collect();
            let item_trunc = title_trunc || summary_trunc || hours_truncated || any_hour_note_trunc;
            if item_trunc {
                workstreams_truncated = true;
            }
            json!({
                "id": ws.id,
                "title": title,
                "summary": summary,
                "minutes": ws.minutes,
                "seconds": ws.seconds,
                "hours": hours_json,
                "truncated": item_trunc,
            })
        })
        .collect();

    let total_reports = hour_reports.len();
    let mut reports_truncated = total_reports > MAX_DAY_HOUR_REPORTS;
    let reports_json: Vec<Value> = hour_reports
        .into_iter()
        .take(MAX_DAY_HOUR_REPORTS)
        .map(|hr| {
            let (summary, summary_trunc) = clip(&hr.summary, DAY_SUMMARY_TEXT_MAX_CHARS);
            let total_activities = hr.activities.len();
            let activities_truncated = total_activities > MAX_HOUR_ACTIVITIES;
            let mut any_desc_trunc = false;
            let activities_json: Vec<Value> = hr
                .activities
                .into_iter()
                .take(MAX_HOUR_ACTIVITIES)
                .map(|a| {
                    let (desc, desc_trunc) = clip(&a.description, 500);
                    if desc_trunc {
                        any_desc_trunc = true;
                    }
                    json!({
                        "description": desc,
                        "minutes": a.minutes,
                        "seconds": a.seconds.unwrap_or_else(|| a.minutes * 60),
                        // Bounds of the measured sessions the activity
                        // happened in, or null when only the hour is known.
                        "periods": a.periods,
                        "truncated": desc_trunc,
                    })
                })
                .collect();
            let item_trunc = summary_trunc || activities_truncated || any_desc_trunc;
            if item_trunc {
                reports_truncated = true;
            }
            json!({
                "hour": hr.hour,
                "startedAt": hr.started_at,
                "endedAt": hr.ended_at,
                "activeMinutes": hr.active_minutes,
                "summary": summary,
                "activities": activities_json,
                "provider": hr.provider,
                "generatedAt": hr.generated_at,
                "truncated": item_trunc,
            })
        })
        .collect();

    let has_summary = !summary_value.is_null();
    let any_truncated = summary_truncated || workstreams_truncated || reports_truncated;
    let mut result = json!({
        "date": day_key,
        "state": if has_summary { "ready" } else { "not_generated" },
        "summary": summary_value,
        "workstreams": workstreams_json,
        "hourReports": reports_json,
        "totalWorkstreams": total_workstreams,
        "totalHourReports": total_reports,
        "truncated": any_truncated,
    });
    if !has_summary {
        result["reason"] = json!("No summary has been generated for this date yet.");
    }
    Ok(result)
}

/// The day view's sources for one MCP message: meetings from the open data
/// partition (ADR-0059), coding-agent blocks of the sources turned on, none
/// before the retention floor.
struct McpDaySources<'a> {
    notes: &'a SqlitePool,
    profile: &'a str,
    floor: DateTime<Utc>,
    now: DateTime<Utc>,
}

impl DaySources for McpDaySources<'_> {
    fn meetings<'a>(
        &'a self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<MeetingNote>> {
        Box::pin(meetings_between(
            self.notes,
            self.profile,
            from,
            to,
            zone,
            self.now,
        ))
    }

    fn coding_blocks<'a>(
        &'a self,
        store: &'a ActivityStore,
        enabled: &'a CodingAgentSources,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<CodingBlock>> {
        Box::pin(coding_blocks_between(
            store,
            enabled,
            from.max(self.floor),
            to,
            zone,
        ))
    }

    /// Only generation waits for ingestion; the panels never ask.
    fn coding_agents_read_until(&self, _until: DateTime<Utc>) -> bool {
        true
    }
}

/// The summary's panels (`pipeline::day_view`), as the "Today" view draws
/// them, in minutes.
async fn day_panels(
    data: &McpData,
    store: &ActivityStore,
    settings: &ActivitySettings,
    arguments: &Value,
) -> Result<Value, AppError> {
    let day = parse_day_argument(arguments, data.now)?;
    let day_key = day.format("%Y-%m-%d").to_string();
    if past_retention(settings, day, data.now) {
        return Ok(json!({
            "date": day_key,
            "state": "past_retention",
            "reason": "The requested date is past the activity retention period.",
            "panels": Value::Null,
        }));
    }
    let sources = McpDaySources {
        notes: &data.notes,
        profile: &data.profile,
        floor: retention_floor(settings, data.now),
        now: data.now,
    };
    let context = ReadContext {
        store,
        settings,
        sources: &sources,
        zone: &SystemZone,
        now: data.now,
    };
    let (_, _, _, panels) = pipeline::day_view(&context, day)
        .await
        .map_err(|error| AppError::new("activity_day_panels_failed", error.to_string()))?;
    let mut totals = stats_json(&TimelineStatsDto {
        focused_ms: panels.focused_ms,
        idle_ms: panels.idle_ms,
        away_ms: panels.away_ms,
        top_apps: panels.top_apps,
        categories: panels.categories,
    });
    let total_workstreams = panels.workstreams.len();
    totals["hours"] = json!(panels
        .hours
        .iter()
        .map(|hour| json!({ "hour": hour.hour, "focusedMinutes": minutes(hour.focused_ms) }))
        .collect::<Vec<_>>());
    totals["workstreams"] = json!(panels
        .workstreams
        .iter()
        .take(MAX_DAY_WORKSTREAMS)
        .map(|workstream| json!({
            "id": workstream.id,
            "title": clip(&workstream.title, 500).0,
            "minutes": workstream.minutes,
            "seconds": workstream.seconds,
        }))
        .collect::<Vec<_>>());
    totals["totalWorkstreams"] = json!(total_workstreams);
    totals["meetings"] = json!({
        "count": panels.meeting_count,
        "minutes": minutes(panels.meeting_ms),
    });
    totals["codingAgents"] = json!({
        "blocks": panels.coding_agent_blocks,
        "activeMinutes": minutes(panels.coding_agent_active_seconds * 1000),
    });
    Ok(json!({
        "date": day_key,
        "state": "ready",
        "panels": totals,
        "truncated": total_workstreams > MAX_DAY_WORKSTREAMS,
    }))
}

/// When the day summary is written and how activity notifications behave
/// (Settings, Activity).
fn day_schedule(settings: &ActivitySettings) -> Value {
    let quiet = &settings.notifications.quiet_hours;
    json!({
        "summaryTime": settings.day_summary.time,
        "notifications": {
            "enabled": settings.notifications.enabled,
            "quietHours": {
                "enabled": quiet.enabled,
                "start": quiet.start,
                "end": quiet.end,
            },
        },
    })
}
