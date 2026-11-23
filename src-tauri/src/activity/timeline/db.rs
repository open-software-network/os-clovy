//! The timeline's tables in `activity.sqlite3` (migration 2,
//! `activity_timeline`): persisting builder events and reading days, session
//! details, and search results. Every batch is written in one transaction
//! together with the ETL state and the processing cursor, so a restart never
//! processes a frame twice or skips one.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::query::query;
use sqlx::row::Row;
use sqlx_sqlite::{SqliteConnection, SqliteRow};

use super::builder::{BuilderState, GapKind, GapRecord, OpenSession, TimelineEvent, WindowSeen};
use super::categorize::{categorize, Category, SessionEvidence, WindowSample};
use super::context::ContextKind;
use crate::activity::store::{
    advance_cursor_on, timestamp, ActivityStore, PauseReason, StoreError, TIMELINE_CONSUMER,
};

/// A search document covers one window (title + URL) for at most this long,
/// so a result points within 5 minutes of when the text was on screen.
pub const DOC_SPAN: Duration = Duration::seconds(300);
/// Distinct lines kept per search document.
const DOC_BODY_LIMIT: usize = 32_000;
/// Captured text the categorizer reads per session.
const EVIDENCE_TEXT_LIMIT: usize = 20_000;
const EVIDENCE_WINDOWS: i64 = 50;
const EVIDENCE_DOCS: i64 = 40;
const DETAIL_WINDOWS: i64 = 20;
const EXCERPT_CHARS: usize = 1500;

/// A recording interval (meeting audio) from the main database.
pub type Interval = (DateTime<Utc>, DateTime<Utc>);

/// The search document currently receiving text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenDoc {
    pub rowid: i64,
    pub session_row_id: i64,
    pub window_title: String,
    pub browser_url: String,
    pub started_at: DateTime<Utc>,
}

/// Everything the ETL needs to resume exactly where it stopped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EtlState {
    pub builder: BuilderState,
    /// `timeline_sessions.id` of `builder.open`.
    pub open_row_id: Option<i64>,
    pub doc: Option<OpenDoc>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSessionDto {
    pub id: i64,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub context_kind: Option<ContextKind>,
    pub context: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub duration_ms: i64,
    pub active: bool,
    pub category: Category,
    pub confidence: f64,
    /// The most-seen window title.
    pub window_title: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineGapDto {
    pub id: i64,
    pub started_at: String,
    pub ended_at: String,
    pub duration_ms: i64,
    pub kind: GapKind,
    pub pause_reason: Option<PauseReason>,
    /// Still going on at read time (synthesized up to "now", not stored yet;
    /// `id` is 0). Stored gaps are always `false`.
    pub ongoing: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineWindowDto {
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub frame_count: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSessionDetailDto {
    pub session: TimelineSessionDto,
    pub windows: Vec<TimelineWindowDto>,
    pub text_excerpt: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSearchResultDto {
    pub session_id: i64,
    pub app_name: String,
    #[serde(skip)]
    pub bundle_id: Option<String>,
    /// The session's persisted browser domain, for exclusion filtering: a
    /// document written before the URL was known has no URL of its own.
    #[serde(skip)]
    pub session_domain: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub seen_at: String,
    pub snippet: String,
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn pause_reason_from_db(value: Option<&str>) -> Option<PauseReason> {
    match value? {
        "manual" => Some(PauseReason::Manual),
        "work_hours" => Some(PauseReason::WorkHours),
        "low_disk" => Some(PauseReason::LowDisk),
        "protected_video" => Some(PauseReason::ProtectedVideo),
        _ => None,
    }
}

/// `existing` plus the lines of `text` it does not have yet, within the
/// document limit; `None` when nothing is new.
fn merged_body(existing: &str, text: &str) -> Option<String> {
    let seen: HashSet<&str> = existing.lines().map(str::trim).collect();
    let mut added_lines: HashSet<&str> = HashSet::new();
    let mut added = String::new();
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        if seen.contains(line) || !added_lines.insert(line) {
            continue;
        }
        if existing.len() + added.len() + line.len() + 1 > DOC_BODY_LIMIT {
            break;
        }
        added.push_str(line);
        added.push('\n');
    }
    (!added.is_empty()).then(|| format!("{existing}{added}"))
}

pub async fn load_state(store: &ActivityStore) -> Result<EtlState, StoreError> {
    let row = query("SELECT state_json FROM timeline_state WHERE id = 1")
        .fetch_optional(store.pool())
        .await?;
    Ok(match row {
        Some(row) => serde_json::from_str(row.get::<&str, _>(0)).unwrap_or_else(|error| {
            tracing::warn!(%error, "activity timeline: unreadable ETL state, starting over");
            EtlState::default()
        }),
        None => EtlState::default(),
    })
}

async fn save_state(conn: &mut SqliteConnection, state: &EtlState) -> Result<(), StoreError> {
    let json = serde_json::to_string(state)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    query(
        "INSERT INTO timeline_state (id, state_json, updated_at) VALUES (1, ?, ?)
         ON CONFLICT(id) DO UPDATE SET state_json = excluded.state_json,
            updated_at = excluded.updated_at",
    )
    .bind(json)
    .bind(timestamp(Utc::now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// The body of the search document receiving text, kept while a batch
/// writes to it: read once, merged in memory, and written once when the
/// document stops receiving text or the batch ends (an FTS5 update
/// re-indexes the whole body, so one per frame would re-tokenize up to
/// 32k chars every capture tick).
struct PendingBody {
    rowid: i64,
    body: String,
    changed: bool,
}

async fn flush_body(
    conn: &mut SqliteConnection,
    pending: &mut Option<PendingBody>,
) -> Result<(), StoreError> {
    if let Some(body) = pending.take().filter(|body| body.changed) {
        query("UPDATE timeline_search SET body = ? WHERE rowid = ?")
            .bind(body.body)
            .bind(body.rowid)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Writes `events`, the active session, the state, and (for frame batches)
/// the cursor in one transaction. On error nothing is written and the caller
/// must reload the state.
pub async fn persist_batch(
    store: &ActivityStore,
    state: &mut EtlState,
    events: &[TimelineEvent],
    meetings: &[Interval],
    cursor: Option<(i64, DateTime<Utc>)>,
) -> Result<(), StoreError> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await?;
    let mut pending = None;
    for event in events {
        apply(&mut tx, state, event, meetings, &mut pending).await?;
    }
    flush_body(&mut tx, &mut pending).await?;
    if let (Some(open), Some(row_id)) = (state.builder.open.clone(), state.open_row_id) {
        update_session(&mut tx, row_id, &open, "active", meetings).await?;
    }
    save_state(&mut tx, state).await?;
    if let Some((last_frame_id, last_frame_at)) = cursor {
        advance_cursor_on(&mut tx, TIMELINE_CONSUMER, last_frame_id, last_frame_at).await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn apply(
    conn: &mut SqliteConnection,
    state: &mut EtlState,
    event: &TimelineEvent,
    meetings: &[Interval],
    pending: &mut Option<PendingBody>,
) -> Result<(), StoreError> {
    if !matches!(event, TimelineEvent::Window(_)) {
        // The session's evidence (read when it closes) includes the body.
        flush_body(conn, pending).await?;
    }
    match event {
        TimelineEvent::Opened(session) => {
            // Defensive: the builder closes before it opens, but a lost state
            // must never leave two active rows.
            query("UPDATE timeline_sessions SET status = 'closed' WHERE status = 'active'")
                .execute(&mut *conn)
                .await?;
            let id = query(
                "INSERT INTO timeline_sessions
                    (app_name, bundle_id, context_kind, context, started_at, ended_at, duration_ms,
                     first_frame_id, last_frame_id, frame_count, idle_frame_count, status, category)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'active', 'idle_personal')",
            )
            .bind(&session.app_name)
            .bind(&session.bundle_id)
            .bind(
                session
                    .context
                    .as_ref()
                    .map(|context| context.kind.as_str()),
            )
            .bind(
                session
                    .context
                    .as_ref()
                    .map(|context| context.value.as_str()),
            )
            .bind(timestamp(session.started_at))
            .bind(timestamp(session.ended_at))
            .bind(session.duration().num_milliseconds())
            .bind(session.first_frame_id)
            .bind(session.last_frame_id)
            .bind(i64::from(session.frame_count))
            .bind(i64::from(session.idle_frame_count))
            .execute(&mut *conn)
            .await?
            .last_insert_rowid();
            state.open_row_id = Some(id);
            state.doc = None;
        }
        TimelineEvent::Window(window) => {
            if let Some(session_row_id) = state.open_row_id {
                record_window(conn, state, session_row_id, window, pending).await?;
            }
        }
        TimelineEvent::Closed(session) => {
            if let Some(row_id) = state.open_row_id.take() {
                update_session(conn, row_id, session, "closed", meetings).await?;
            }
            state.doc = None;
        }
        TimelineEvent::Gap(gap) => insert_gap(conn, gap).await?,
    }
    Ok(())
}

async fn insert_gap(conn: &mut SqliteConnection, gap: &GapRecord) -> Result<(), StoreError> {
    query(
        "INSERT INTO timeline_gaps (started_at, ended_at, duration_ms, kind, pause_reason)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(timestamp(gap.started_at))
    .bind(timestamp(gap.ended_at))
    .bind((gap.ended_at - gap.started_at).num_milliseconds())
    .bind(gap.kind.as_db())
    .bind(gap.pause_reason.map(PauseReason::as_str))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn record_window(
    conn: &mut SqliteConnection,
    state: &mut EtlState,
    session_row_id: i64,
    window: &WindowSeen,
    pending: &mut Option<PendingBody>,
) -> Result<(), StoreError> {
    let title = window.window_title.clone().unwrap_or_default();
    let url = window.browser_url.clone().unwrap_or_default();
    let at = timestamp(window.at);
    query(
        "INSERT INTO timeline_session_windows
            (session_id, window_title, browser_url, first_seen_at, last_seen_at, frame_count)
         VALUES (?, ?, ?, ?, ?, 1)
         ON CONFLICT(session_id, window_title, browser_url) DO UPDATE SET
            last_seen_at = excluded.last_seen_at,
            frame_count = timeline_session_windows.frame_count + 1",
    )
    .bind(session_row_id)
    .bind(&title)
    .bind(&url)
    .bind(&at)
    .bind(&at)
    .execute(&mut *conn)
    .await?;

    let current = state
        .doc
        .as_ref()
        .filter(|doc| {
            doc.session_row_id == session_row_id
                && doc.window_title == title
                && doc.browser_url == url
                && window.at - doc.started_at < DOC_SPAN
        })
        .map(|doc| doc.rowid);
    if let Some(rowid) = current {
        if let Some(text) = &window.text {
            if pending.as_ref().map(|body| body.rowid) != Some(rowid) {
                flush_body(conn, pending).await?;
                let body: String = query("SELECT body FROM timeline_search WHERE rowid = ?")
                    .bind(rowid)
                    .fetch_optional(&mut *conn)
                    .await?
                    .map(|row| row.get(0))
                    .unwrap_or_default();
                *pending = Some(PendingBody {
                    rowid,
                    body,
                    changed: false,
                });
            }
            if let Some(open) = pending.as_mut() {
                if let Some(body) = merged_body(&open.body, text) {
                    open.body = body;
                    open.changed = true;
                }
            }
        }
        return Ok(());
    }

    flush_body(conn, pending).await?;
    let body = window
        .text
        .as_deref()
        .and_then(|text| merged_body("", text))
        .unwrap_or_default();
    let rowid = query(
        "INSERT INTO timeline_search (window_title, browser_url, body, session_id, seen_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&title)
    .bind(&url)
    .bind(&body)
    .bind(session_row_id)
    .bind(&at)
    .execute(&mut *conn)
    .await?
    .last_insert_rowid();
    *pending = Some(PendingBody {
        rowid,
        body,
        changed: false,
    });
    query(
        "UPDATE timeline_sessions
         SET search_rowid_first = COALESCE(search_rowid_first, ?), search_rowid_last = ?
         WHERE id = ?",
    )
    .bind(rowid)
    .bind(rowid)
    .bind(session_row_id)
    .execute(&mut *conn)
    .await?;
    state.doc = Some(OpenDoc {
        rowid,
        session_row_id,
        window_title: title,
        browser_url: url,
        started_at: window.at,
    });
    Ok(())
}

async fn session_evidence_rows(
    conn: &mut SqliteConnection,
    row_id: i64,
) -> Result<(Vec<WindowSample>, String), StoreError> {
    let windows = query(
        "SELECT window_title, browser_url, frame_count FROM timeline_session_windows
         WHERE session_id = ? ORDER BY frame_count DESC LIMIT ?",
    )
    .bind(row_id)
    .bind(EVIDENCE_WINDOWS)
    .fetch_all(&mut *conn)
    .await?
    .iter()
    .map(|row| WindowSample {
        title: row.get(0),
        url: row.get(1),
        frames: u32::try_from(row.get::<i64, _>(2)).unwrap_or(u32::MAX),
    })
    .collect();
    let bodies = query(
        "SELECT body FROM timeline_search
         WHERE rowid BETWEEN
            (SELECT search_rowid_first FROM timeline_sessions WHERE id = ?1)
            AND (SELECT search_rowid_last FROM timeline_sessions WHERE id = ?1)
           AND session_id = ?1
         ORDER BY rowid LIMIT ?2",
    )
    .bind(row_id)
    .bind(EVIDENCE_DOCS)
    .fetch_all(&mut *conn)
    .await?;
    let mut text = String::new();
    for row in &bodies {
        let body: &str = row.get(0);
        if text.len() + body.len() > EVIDENCE_TEXT_LIMIT {
            break;
        }
        text.push_str(body);
    }
    Ok((windows, text))
}

async fn update_session(
    conn: &mut SqliteConnection,
    row_id: i64,
    session: &OpenSession,
    status: &str,
    meetings: &[Interval],
) -> Result<(), StoreError> {
    let (windows, text) = session_evidence_rows(conn, row_id).await?;
    let meeting_audio = meetings
        .iter()
        .any(|(start, end)| *start < session.ended_at && *end > session.started_at);
    let (category, confidence) = categorize(&SessionEvidence {
        app_name: &session.app_name,
        bundle_id: session.bundle_id.as_deref(),
        windows: &windows,
        text: &text,
        meeting_audio,
    });
    query(
        "UPDATE timeline_sessions SET
            context_kind = ?, context = ?, ended_at = ?, duration_ms = ?, last_frame_id = ?,
            frame_count = ?, idle_frame_count = ?, status = ?, category = ?, confidence = ?,
            meeting_audio = ?
         WHERE id = ?",
    )
    .bind(
        session
            .context
            .as_ref()
            .map(|context| context.kind.as_str()),
    )
    .bind(
        session
            .context
            .as_ref()
            .map(|context| context.value.as_str()),
    )
    .bind(timestamp(session.ended_at))
    .bind(session.duration().num_milliseconds())
    .bind(session.last_frame_id)
    .bind(i64::from(session.frame_count))
    .bind(i64::from(session.idle_frame_count))
    .bind(status)
    .bind(category.as_db())
    .bind(f64::from(confidence))
    .bind(meeting_audio)
    .bind(row_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

const SESSION_COLUMNS: &str = "s.id, s.app_name, s.bundle_id, s.context_kind, s.context,
    s.started_at, s.ended_at, s.duration_ms, s.status, s.category, s.confidence,
    (SELECT w.window_title FROM timeline_session_windows w
      WHERE w.session_id = s.id AND w.window_title <> ''
      ORDER BY w.frame_count DESC LIMIT 1)";

fn session_dto(row: &SqliteRow) -> TimelineSessionDto {
    TimelineSessionDto {
        id: row.get(0),
        app_name: row.get(1),
        bundle_id: row.get(2),
        context_kind: row.get::<Option<&str>, _>(3).and_then(ContextKind::parse),
        context: row.get(4),
        started_at: row.get(5),
        ended_at: row.get(6),
        duration_ms: row.get(7),
        active: row.get::<&str, _>(8) == "active",
        category: Category::from_db(row.get::<&str, _>(9)),
        confidence: row.get(10),
        window_title: row.get(11),
    }
}

fn gap_dto(row: &SqliteRow) -> TimelineGapDto {
    TimelineGapDto {
        id: row.get(0),
        started_at: row.get(1),
        ended_at: row.get(2),
        duration_ms: row.get(3),
        kind: GapKind::from_db(row.get::<&str, _>(4)),
        pause_reason: pause_reason_from_db(row.get::<Option<&str>, _>(5)),
        ongoing: false,
    }
}

/// Sessions overlapping `[from, to)`, oldest first.
pub async fn sessions_between(
    store: &ActivityStore,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<TimelineSessionDto>, StoreError> {
    let rows = query(&format!(
        "SELECT {SESSION_COLUMNS} FROM timeline_sessions s
         WHERE s.started_at < ? AND s.ended_at > ? ORDER BY s.started_at, s.id"
    ))
    .bind(timestamp(to))
    .bind(timestamp(from))
    .fetch_all(store.pool())
    .await?;
    Ok(rows.iter().map(session_dto).collect())
}

/// Gaps overlapping `[from, to)`, oldest first.
pub async fn gaps_between(
    store: &ActivityStore,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<TimelineGapDto>, StoreError> {
    let rows = query(
        "SELECT id, started_at, ended_at, duration_ms, kind, pause_reason FROM timeline_gaps
         WHERE started_at < ? AND ended_at > ? ORDER BY started_at, id",
    )
    .bind(timestamp(to))
    .bind(timestamp(from))
    .fetch_all(store.pool())
    .await?;
    Ok(rows.iter().map(gap_dto).collect())
}

pub async fn session_detail(
    store: &ActivityStore,
    id: i64,
) -> Result<Option<TimelineSessionDetailDto>, StoreError> {
    let Some(row) = query(&format!(
        "SELECT {SESSION_COLUMNS} FROM timeline_sessions s WHERE s.id = ?"
    ))
    .bind(id)
    .fetch_optional(store.pool())
    .await?
    else {
        return Ok(None);
    };
    let session = session_dto(&row);
    let windows = query(
        "SELECT window_title, browser_url, first_seen_at, last_seen_at, frame_count
         FROM timeline_session_windows WHERE session_id = ?
         ORDER BY frame_count DESC, first_seen_at LIMIT ?",
    )
    .bind(id)
    .bind(DETAIL_WINDOWS)
    .fetch_all(store.pool())
    .await?
    .iter()
    .map(|row| TimelineWindowDto {
        window_title: non_empty(row.get(0)),
        browser_url: non_empty(row.get(1)),
        first_seen_at: row.get(2),
        last_seen_at: row.get(3),
        frame_count: row.get(4),
    })
    .collect();
    let excerpt: Option<String> = query(
        "SELECT body FROM timeline_search
         WHERE rowid BETWEEN
            (SELECT search_rowid_first FROM timeline_sessions WHERE id = ?1)
            AND (SELECT search_rowid_last FROM timeline_sessions WHERE id = ?1)
           AND session_id = ?1 AND body <> ''
         ORDER BY length(body) DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(store.pool())
    .await?
    .map(|row| row.get::<String, _>(0))
    .map(|body| body.chars().take(EXCERPT_CHARS).collect::<String>())
    .and_then(non_empty);
    Ok(Some(TimelineSessionDetailDto {
        session,
        windows,
        text_excerpt: excerpt,
    }))
}

/// The user's words as an FTS5 query: every word must appear (any column),
/// the last one as a prefix. FTS5 syntax in the input is neutralized.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .map(|term| term.replace('"', ""))
        .filter(|term| term.chars().any(char::is_alphanumeric))
        .map(|term| format!("\"{term}\""))
        .collect();
    let last = terms.len().checked_sub(1)?;
    Some(
        terms
            .iter()
            .enumerate()
            .map(|(index, term)| {
                if index == last {
                    format!("{term}*")
                } else {
                    term.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// Best matches first among documents seen in `[from, to)`; `offset` pages
/// through the ranked results.
pub async fn search(
    store: &ActivityStore,
    input: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: u32,
    offset: u32,
) -> Result<Vec<TimelineSearchResultDto>, StoreError> {
    let Some(expression) = fts_query(input) else {
        return Ok(Vec::new());
    };
    let rows = query(
        "SELECT timeline_search.session_id, timeline_search.seen_at,
                timeline_search.window_title, timeline_search.browser_url,
                snippet(timeline_search, -1, '', '', '\u{2026}', 16),
                s.app_name, s.bundle_id,
                CASE WHEN s.context_kind = 'domain' THEN s.context END
         FROM timeline_search JOIN timeline_sessions s ON s.id = timeline_search.session_id
         WHERE timeline_search MATCH ?
           AND timeline_search.seen_at >= ? AND timeline_search.seen_at < ?
         ORDER BY bm25(timeline_search), timeline_search.seen_at DESC
         LIMIT ? OFFSET ?",
    )
    .bind(expression)
    .bind(timestamp(from))
    .bind(timestamp(to))
    .bind(i64::from(limit))
    .bind(i64::from(offset))
    .fetch_all(store.pool())
    .await?;
    Ok(rows
        .iter()
        .map(|row| TimelineSearchResultDto {
            session_id: row.get(0),
            seen_at: row.get(1),
            window_title: non_empty(row.get(2)),
            browser_url: non_empty(row.get(3)),
            snippet: row.get(4),
            app_name: row.get(5),
            bundle_id: row.get(6),
            session_domain: row.get(7),
        })
        .collect())
}

/// Debug export sections: the timeline tables, newest rows first.
pub async fn debug_sections(
    store: &ActivityStore,
    limit: u32,
) -> Result<serde_json::Map<String, serde_json::Value>, StoreError> {
    let sessions: Vec<TimelineSessionDto> = query(&format!(
        "SELECT {SESSION_COLUMNS} FROM timeline_sessions s ORDER BY s.id DESC LIMIT ?"
    ))
    .bind(i64::from(limit))
    .fetch_all(store.pool())
    .await?
    .iter()
    .map(session_dto)
    .collect();
    let gaps: Vec<TimelineGapDto> = query(
        "SELECT id, started_at, ended_at, duration_ms, kind, pause_reason FROM timeline_gaps
         ORDER BY id DESC LIMIT ?",
    )
    .bind(i64::from(limit))
    .fetch_all(store.pool())
    .await?
    .iter()
    .map(gap_dto)
    .collect();
    let documents: i64 = query("SELECT count(*) FROM timeline_search")
        .fetch_one(store.pool())
        .await?
        .get(0);
    let state = load_state(store).await?;
    let mut sections = serde_json::Map::new();
    sections.insert("timelineSessions".into(), json(&sessions));
    sections.insert("timelineGaps".into(), json(&gaps));
    sections.insert("timelineSearchDocuments".into(), documents.into());
    sections.insert("timelineState".into(), json(&state));
    Ok(sections)
}

fn json<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merged_body_adds_only_new_lines() {
        let body = merged_body("", "alpha\nbeta\n\nalpha").unwrap();
        assert_eq!(body, "alpha\nbeta\n");
        assert_eq!(merged_body(&body, "beta\nalpha"), None);
        assert_eq!(
            merged_body(&body, "gamma\nbeta").unwrap(),
            "alpha\nbeta\ngamma\n"
        );
    }

    #[test]
    fn fts_query_quotes_terms_and_prefixes_the_last() {
        assert_eq!(
            fts_query("timeline sqlx").as_deref(),
            Some("\"timeline\" \"sqlx\"*")
        );
        assert_eq!(
            fts_query("a\" OR b NEAR(").as_deref(),
            Some("\"a\" \"OR\" \"b\" \"NEAR(\"*")
        );
        assert_eq!(fts_query("  -- "), None);
    }
}
