//! The activity timeline: an incremental ETL from captured frames to app
//! sessions (with browser domain / editor workspace context), classified gaps,
//! deterministic categories, day statistics, and FTS5 search, all in the
//! encrypted activity database. Read by the "Today" view and the agent tools.
//! Contract and schema: `docs/activity-timeline.md`.
//!
//! Layout: `builder` (pure frames → events state machine), `context`,
//! `categorize`, `stats` (pure), `db` (tables, persistence, queries), `etl`
//! (one pass), `agent_tools` (`search_activity`, `get_activity_timeline`).

pub mod agent_tools;
pub mod builder;
pub mod categorize;
pub mod context;
pub mod db;
pub mod debug_import;
pub mod etl;
#[cfg(test)]
#[path = "red_tests.rs"]
mod red_tests;
pub mod stats;

use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager, State};

use super::engine::ActivityShared;
use super::filter::{app_is_ignored, url_matches_domains};
use super::schedule::{CaptureState, StoreStatus};
use super::settings::ActivitySettings;
use super::store::{parse_time, timestamp, ActivityStore, StoreError, TIMELINE_CONSUMER};
use super::{ActivityRuntime, ActivityState};
use crate::domain::types::AppError;
use context::ContextKind;
use db::{
    Interval, TimelineGapDto, TimelineSearchResultDto, TimelineSessionDetailDto, TimelineSessionDto,
};
use stats::TimelineStatsDto;

/// Emitted after an ETL pass wrote sessions or gaps.
pub const TIMELINE_EVENT: &str = "clovy://activity-timeline";
/// How often the background pass runs once caught up.
const PASS_INTERVAL: StdDuration = StdDuration::from_secs(30);
/// Frames a read (view, search, agent tool) may process before answering;
/// the rest of a backlog is left to the background loop.
const FOREGROUND_FRAMES: usize = 2_000;
/// Frames per lock hold in the background loop, which then yields so reads
/// can take a turn while a historical backfill runs.
const BACKGROUND_FRAMES: usize = 10_000;
const DEFAULT_SEARCH_LIMIT: u32 = 50;
const MAX_SEARCH_LIMIT: u32 = 200;
/// Smallest page of ranked search rows fetched while skipping exclusions.
const SEARCH_PAGE_MIN: u32 = 50;

/// Serializes passes from the background loop, commands, and agent tools.
static PASS_LOCK: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(|| tokio::sync::Mutex::new(()));

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    Ready,
    NeverEnabled,
    Unsupported,
    KeyMissing,
    Error,
}

/// Sessions, gaps, and stats of a range after exclusions and retention.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineView {
    pub sessions: Vec<TimelineSessionDto>,
    pub gaps: Vec<TimelineGapDto>,
    pub stats: TimelineStatsDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityTimelineDto {
    pub availability: Availability,
    pub message: Option<String>,
    pub capture_enabled: bool,
    pub sessions: Vec<TimelineSessionDto>,
    pub gaps: Vec<TimelineGapDto>,
    pub stats: TimelineStatsDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityTimelineSearchDto {
    pub availability: Availability,
    pub results: Vec<TimelineSearchResultDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineRangeRequest {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSessionRequest {
    pub id: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineSearchRequest {
    pub query: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
}

/// A session is hidden while the user's current exclusions match its app or
/// its domain: removing something from capture also removes its history
/// from the view and the agent.
pub(crate) fn session_excluded(
    app_name: &str,
    bundle_id: Option<&str>,
    domain: Option<&str>,
    url: Option<&str>,
    settings: &ActivitySettings,
) -> bool {
    app_is_ignored(app_name, bundle_id, &settings.ignored_apps)
        || domain.is_some_and(|domain| {
            url_matches_domains(&format!("https://{domain}/"), &settings.ignored_domains)
        })
        || url.is_some_and(|url| url_matches_domains(url, &settings.ignored_domains))
}

pub(crate) fn retention_floor(settings: &ActivitySettings, now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::days(i64::from(settings.retention_days))
}

/// A session's detail as the "Today" view, the agent, and the MCP server may
/// show it: `None` when it is gone, ended before the retention period, or
/// excluded (app or domain). Windows on excluded URLs are dropped, and the
/// text excerpt with them, since it can come from any window of the session.
pub(crate) async fn visible_session_detail(
    store: &ActivityStore,
    settings: &ActivitySettings,
    id: i64,
    now: DateTime<Utc>,
) -> Result<Option<TimelineSessionDetailDto>, StoreError> {
    let Some(mut detail) = db::session_detail(store, id).await? else {
        return Ok(None);
    };
    let session = &detail.session;
    let domain = (session.context_kind == Some(ContextKind::Domain))
        .then_some(session.context.as_deref())
        .flatten();
    let expired =
        parse_time(&session.ended_at).map_or(true, |ended| ended < retention_floor(settings, now));
    if expired
        || session_excluded(
            &session.app_name,
            session.bundle_id.as_deref(),
            domain,
            None,
            settings,
        )
    {
        return Ok(None);
    }
    let app_name = session.app_name.clone();
    let shown = detail.windows.len();
    detail.windows.retain(|window| {
        !window
            .browser_url
            .as_deref()
            .is_some_and(|url| session_excluded(&app_name, None, None, Some(url), settings))
    });
    if detail.windows.len() != shown {
        detail.text_excerpt = None;
    }
    Ok(Some(detail))
}

pub async fn timeline_view(
    store: &ActivityStore,
    settings: &ActivitySettings,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<TimelineView, StoreError> {
    let from = from.max(retention_floor(settings, now));
    if to <= from {
        return Ok(TimelineView::default());
    }
    let sessions: Vec<TimelineSessionDto> = db::sessions_between(store, from, to)
        .await?
        .into_iter()
        .filter(|session| {
            let domain = (session.context_kind == Some(ContextKind::Domain))
                .then_some(session.context.as_deref())
                .flatten();
            !session_excluded(
                &session.app_name,
                session.bundle_id.as_deref(),
                domain,
                None,
                settings,
            )
        })
        .collect();
    let mut gaps = db::gaps_between(store, from, to).await?;
    if let Some(gap) = ongoing_gap(store, now).await? {
        if gap.started_at < timestamp(to) && gap.ended_at > timestamp(from) {
            gaps.push(gap);
        }
    }
    let stats = stats::day_stats(&sessions, &gaps, from, to);
    Ok(TimelineView {
        sessions,
        gaps,
        stats,
    })
}

/// The gap going on now, when the timeline is caught up with capture (with a
/// backlog, the last useful frame is not the latest activity).
async fn ongoing_gap(
    store: &ActivityStore,
    now: DateTime<Utc>,
) -> Result<Option<TimelineGapDto>, StoreError> {
    let cursor = store.processing_cursor(TIMELINE_CONSUMER).await?;
    if store.latest_frame_at().await? != cursor.last_frame_at {
        return Ok(None);
    }
    let state = db::load_state(store).await?;
    let Some(last_useful) = state.builder.last_useful_at else {
        return Ok(None);
    };
    let pauses = etl::pause_spans(store, last_useful, now).await?;
    Ok(
        builder::ongoing_gap(&state.builder, now, &pauses).map(|gap| TimelineGapDto {
            id: 0,
            started_at: timestamp(gap.started_at),
            ended_at: timestamp(gap.ended_at),
            duration_ms: (gap.ended_at - gap.started_at).num_milliseconds(),
            kind: gap.kind,
            pause_reason: gap.pause_reason,
            ongoing: true,
        }),
    )
}

/// Ranked FTS rows are paged through until `limit` rows pass the current
/// exclusions (app, the session's domain, the document's URL) or the matches
/// run out, so newly excluded top matches never starve the allowed ones.
pub async fn search_view(
    store: &ActivityStore,
    settings: &ActivitySettings,
    query: &str,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    limit: u32,
    now: DateTime<Utc>,
) -> Result<Vec<TimelineSearchResultDto>, StoreError> {
    let floor = retention_floor(settings, now);
    let from = from.map_or(floor, |from| from.max(floor));
    let to = to.unwrap_or(now + Duration::days(1));
    if to <= from || limit == 0 {
        return Ok(Vec::new());
    }
    let page = limit.saturating_mul(3).max(SEARCH_PAGE_MIN);
    let mut results = Vec::new();
    let mut offset = 0u32;
    loop {
        let rows = db::search(store, query, from, to, page, offset).await?;
        let fetched = rows.len();
        for row in rows {
            let excluded = session_excluded(
                &row.app_name,
                row.bundle_id.as_deref(),
                row.session_domain.as_deref(),
                row.browser_url.as_deref(),
                settings,
            );
            if !excluded {
                results.push(row);
                if results.len() == limit as usize {
                    return Ok(results);
                }
            }
        }
        if fetched < page as usize {
            return Ok(results);
        }
        offset = offset.saturating_add(page);
    }
}

fn parse_request_time(value: &str, field: &str) -> Result<DateTime<Utc>, AppError> {
    parse_time(value).ok_or_else(|| {
        AppError::new(
            "activity_timeline_invalid_range",
            format!("`{field}` must be an RFC 3339 timestamp"),
        )
    })
}

/// How the timeline can be read right now.
pub(crate) enum StoreAccess {
    Open(ActivityStore),
    /// Capture is on but its database is not open yet: an empty, ready view.
    Empty,
    Unavailable(Availability, Option<String>),
}

pub(crate) async fn store_access(runtime: &ActivityRuntime) -> StoreAccess {
    if let Some(store) = runtime.shared.store() {
        return StoreAccess::Open(store);
    }
    let settings = runtime.shared.settings();
    if !runtime.db_path.exists() {
        return if settings.enabled {
            StoreAccess::Empty
        } else {
            StoreAccess::Unavailable(Availability::NeverEnabled, None)
        };
    }
    if settings.enabled {
        // The capture thread owns opening while capture is on.
        return match runtime.shared.state() {
            CaptureState::KeyMissing => StoreAccess::Unavailable(Availability::KeyMissing, None),
            CaptureState::Error { message } => {
                StoreAccess::Unavailable(Availability::Error, Some(message))
            }
            _ => StoreAccess::Empty,
        };
    }
    // Capture is off but history exists: open it read-mostly for the view.
    match ActivityStore::open(&runtime.db_path, runtime.keys.as_ref()).await {
        Ok(store) => {
            runtime
                .shared
                .replace_store(Some(store.clone()), StoreStatus::Ready);
            StoreAccess::Open(store)
        }
        Err(StoreError::KeyMissing | StoreError::KeyRejected) => {
            StoreAccess::Unavailable(Availability::KeyMissing, None)
        }
        Err(error) => StoreAccess::Unavailable(Availability::Error, Some(error.to_string())),
    }
}

/// Recording intervals from the main database that end at or after `since`:
/// the categorizer's evidence of meeting audio.
async fn meeting_intervals(app: &AppHandle, since: DateTime<Utc>) -> Vec<Interval> {
    let Ok(repositories) = crate::commands::repositories(app).await else {
        return Vec::new();
    };
    // `recording_sessions` stores RFC 3339 UTC with milliseconds.
    let since_text = since.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let rows = sqlx::query::query(
        "SELECT started_at, ended_at FROM recording_sessions
         WHERE ended_at IS NULL OR ended_at >= ?",
    )
    .bind(since_text)
    .fetch_all(&repositories.pool)
    .await
    .unwrap_or_default();
    let now = Utc::now();
    rows.iter()
        .filter_map(|row| {
            use sqlx::row::Row;
            let start = parse_time(row.try_get::<&str, _>(0).ok()?)?;
            let end = row
                .try_get::<Option<&str>, _>(1)
                .ok()
                .flatten()
                .and_then(parse_time)
                .unwrap_or(now);
            (end >= since).then_some((start, end))
        })
        .collect()
}

/// The earliest moment a pass can touch: the open session's start or the
/// first unprocessed frame, whichever is older (a first build after the
/// migration starts at the oldest retained frame). Recordings ending before
/// it cannot overlap any session the pass writes.
async fn meeting_window_start(
    store: &ActivityStore,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, StoreError> {
    let open_started = db::load_state(store)
        .await?
        .builder
        .open
        .map(|open| open.started_at);
    let cursor = store.processing_cursor(TIMELINE_CONSUMER).await?;
    let first_unprocessed = store
        .frames_after(cursor.last_frame_id, 1)
        .await?
        .first()
        .and_then(|frame| parse_time(&frame.captured_at));
    Ok([open_started, first_unprocessed]
        .into_iter()
        .flatten()
        .fold(now, DateTime::min))
}

/// One pass of at most `max_frames`, with the caller holding `PASS_LOCK`;
/// tells the frontend when it changed something.
async fn pass_locked(
    app: &AppHandle,
    store: &ActivityStore,
    max_frames: usize,
) -> Result<etl::PassReport, StoreError> {
    let now = Utc::now();
    let meetings = meeting_intervals(app, meeting_window_start(store, now).await?).await;
    let report = etl::run_pass_bounded(store, now, &meetings, max_frames).await?;
    if report.changed {
        let _ = app.emit(TIMELINE_EVENT, serde_json::json!({ "changed": true }));
    }
    Ok(report)
}

/// Before a read: a bounded pass, skipped when another pass is running (the
/// read then answers from what is already built instead of waiting).
pub async fn refresh_before_read(app: &AppHandle, store: &ActivityStore) {
    let Ok(_guard) = PASS_LOCK.try_lock() else {
        return;
    };
    if let Err(error) = pass_locked(app, store, FOREGROUND_FRAMES).await {
        tracing::warn!(%error, "activity timeline pass failed before a read");
    }
}

/// Background loop: while a backlog remains, passes of `BACKGROUND_FRAMES`
/// back to back (releasing the lock between them); once caught up, a pass
/// every 30 s while the store is open, so the cursor (and with it retention)
/// keeps moving without the view open. In development builds, a fixture named
/// by `CLOVY_ACTIVITY_DEBUG_IMPORT` is imported once the store is first open
/// (`debug_import`).
pub fn start(app: AppHandle, shared: Arc<ActivityShared>, data_dir: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let mut fixture = cfg!(debug_assertions)
            .then(|| std::env::var_os(debug_import::DEBUG_IMPORT_ENV))
            .flatten()
            .map(PathBuf::from);
        loop {
            let mut backlog = false;
            if let Some(store) = shared.store() {
                if let Some(path) = fixture.take() {
                    match debug_import::import_file(&store, &path, &data_dir).await {
                        Ok(frames) => tracing::info!(frames, "activity debug fixture imported"),
                        Err(error) => tracing::warn!(%error, "activity debug fixture failed"),
                    }
                }
                let result = {
                    let _guard = PASS_LOCK.lock().await;
                    pass_locked(&app, &store, BACKGROUND_FRAMES).await
                };
                match result {
                    Ok(report) => backlog = !report.caught_up,
                    Err(error) => tracing::warn!(%error, "activity timeline pass failed"),
                }
            }
            if backlog {
                tokio::time::sleep(StdDuration::from_millis(100)).await;
            } else {
                tokio::time::sleep(PASS_INTERVAL).await;
            }
        }
    });
}

fn runtime_of<'a>(state: &'a State<'_, ActivityState>) -> Option<&'a ActivityRuntime> {
    state.0.as_ref()
}

fn empty_timeline(
    availability: Availability,
    message: Option<String>,
    capture_enabled: bool,
) -> ActivityTimelineDto {
    ActivityTimelineDto {
        availability,
        message,
        capture_enabled,
        sessions: Vec::new(),
        gaps: Vec::new(),
        stats: TimelineStatsDto::default(),
    }
}

/// The "Today" view's read path: sessions, gaps, and stats overlapping
/// `[from, to)`, after a fresh pass.
#[tauri::command]
pub async fn activity_timeline(
    app: AppHandle,
    state: State<'_, ActivityState>,
    request: TimelineRangeRequest,
) -> Result<ActivityTimelineDto, AppError> {
    let from = parse_request_time(&request.from, "from")?;
    let to = parse_request_time(&request.to, "to")?;
    let Some(runtime) = runtime_of(&state) else {
        return Ok(empty_timeline(Availability::Unsupported, None, false));
    };
    let settings = runtime.shared.settings();
    let store = match store_access(runtime).await {
        StoreAccess::Open(store) => store,
        StoreAccess::Empty => {
            return Ok(empty_timeline(Availability::Ready, None, settings.enabled))
        }
        StoreAccess::Unavailable(availability, message) => {
            return Ok(empty_timeline(availability, message, settings.enabled))
        }
    };
    refresh_before_read(&app, &store).await;
    match timeline_view(&store, &settings, from, to, Utc::now()).await {
        Ok(view) => Ok(ActivityTimelineDto {
            availability: Availability::Ready,
            message: None,
            capture_enabled: settings.enabled,
            sessions: view.sessions,
            gaps: view.gaps,
            stats: view.stats,
        }),
        Err(error) => Ok(empty_timeline(
            Availability::Error,
            Some(error.to_string()),
            settings.enabled,
        )),
    }
}

#[tauri::command]
pub async fn activity_timeline_session(
    state: State<'_, ActivityState>,
    request: TimelineSessionRequest,
) -> Result<TimelineSessionDetailDto, AppError> {
    let missing = || {
        AppError::new(
            "activity_session_not_found",
            "This session is no longer in the activity history.",
        )
    };
    let runtime = runtime_of(&state).ok_or_else(missing)?;
    let StoreAccess::Open(store) = store_access(runtime).await else {
        return Err(missing());
    };
    visible_session_detail(&store, &runtime.shared.settings(), request.id, Utc::now())
        .await
        .map_err(|error| AppError::new("activity_session_failed", error.to_string()))?
        .ok_or_else(missing)
}

#[tauri::command]
pub async fn activity_timeline_search(
    app: AppHandle,
    state: State<'_, ActivityState>,
    request: TimelineSearchRequest,
) -> Result<ActivityTimelineSearchDto, AppError> {
    let from = request
        .from
        .as_deref()
        .map(|value| parse_request_time(value, "from"))
        .transpose()?;
    let to = request
        .to
        .as_deref()
        .map(|value| parse_request_time(value, "to"))
        .transpose()?;
    let limit = request
        .limit
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);
    let Some(runtime) = runtime_of(&state) else {
        return Ok(ActivityTimelineSearchDto {
            availability: Availability::Unsupported,
            results: Vec::new(),
        });
    };
    let store = match store_access(runtime).await {
        StoreAccess::Open(store) => store,
        StoreAccess::Empty => {
            return Ok(ActivityTimelineSearchDto {
                availability: Availability::Ready,
                results: Vec::new(),
            })
        }
        StoreAccess::Unavailable(availability, _) => {
            return Ok(ActivityTimelineSearchDto {
                availability,
                results: Vec::new(),
            })
        }
    };
    refresh_before_read(&app, &store).await;
    let settings = runtime.shared.settings();
    let results = search_view(
        &store,
        &settings,
        &request.query,
        from,
        to,
        limit,
        Utc::now(),
    )
    .await
    .map_err(|error| AppError::new("activity_search_failed", error.to_string()))?;
    Ok(ActivityTimelineSearchDto {
        availability: Availability::Ready,
        results,
    })
}

/// Whether capture is on: activity tools (agent and MCP server) exist only
/// then.
pub(crate) fn capture_enabled(app: &AppHandle) -> bool {
    app.state::<ActivityState>()
        .0
        .as_ref()
        .is_some_and(|runtime| runtime.shared.settings().enabled)
}

/// Agent catalog entries: present only while capture is enabled.
pub fn agent_tool_descriptors(app: &AppHandle) -> Vec<Value> {
    agent_tools::descriptors(capture_enabled(app))
}

/// The open store and the current settings for a tool call (agent or MCP
/// server), after a bounded refresh. Refuses while capture is off.
pub(crate) async fn store_for_tools(
    app: &AppHandle,
) -> Result<(ActivityStore, ActivitySettings), AppError> {
    let state = app.state::<ActivityState>();
    let runtime = state
        .0
        .as_ref()
        .filter(|runtime| runtime.shared.settings().enabled)
        .ok_or_else(|| {
            AppError::new(
                "activity_capture_off",
                "Activity capture is off. The user can turn it on in Settings, Activity.",
            )
        })?;
    let StoreAccess::Open(store) = store_access(runtime).await else {
        return Err(AppError::new(
            "activity_database_closed",
            "The activity history is not available right now.",
        ));
    };
    refresh_before_read(app, &store).await;
    Ok((store, runtime.shared.settings()))
}

/// Agent dispatch for `search_activity` / `get_activity_timeline`.
pub async fn dispatch_agent_tool(
    app: &AppHandle,
    name: &str,
    arguments: &Value,
) -> Result<Value, AppError> {
    let (store, settings) = store_for_tools(app).await?;
    agent_tools::dispatch(&store, &settings, name, arguments, Utc::now())
        .await
        .unwrap_or_else(|| {
            Err(AppError::new(
                "agent_tool_unknown",
                format!("Unknown activity tool `{name}`."),
            ))
        })
}

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::activity::key::MemoryKeyStore;
    use crate::activity::store::{NewFrame, PauseReason, TextSource, ACTIVITY_DB_FILE};
    use builder::GapKind;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, hour, minute, second)
            .unwrap()
    }

    async fn open_store() -> (tempfile::TempDir, ActivityStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ActivityStore::open(
            &dir.path().join(ACTIVITY_DB_FILE),
            &MemoryKeyStore::default(),
        )
        .await
        .unwrap();
        (dir, store)
    }

    async fn frame(
        store: &ActivityStore,
        when: DateTime<Utc>,
        app: &str,
        title: &str,
        url: Option<&str>,
        text: &str,
    ) {
        store
            .insert_frame(&NewFrame {
                captured_at: when,
                app_name: app.into(),
                bundle_id: None,
                window_title: Some(title.into()),
                browser_url: url.map(str::to_string),
                text_source: TextSource::Accessibility,
                text: Some(text.into()),
                input_idle_ms: None,
            })
            .await
            .unwrap();
    }

    fn settings() -> ActivitySettings {
        ActivitySettings {
            enabled: true,
            ..ActivitySettings::default()
        }
    }

    /// The one detail path (the "Today" view's command and the MCP server):
    /// past retention or excluded it is gone, and a window on an excluded
    /// URL takes the text excerpt with it.
    #[tokio::test]
    async fn session_detail_applies_retention_and_url_exclusions() {
        let (_dir, store) = open_store().await;
        for second in 0..5 {
            frame(
                &store,
                at(9, 0, second * 2),
                "Notion",
                "Plan",
                None,
                "roadmap draft",
            )
            .await;
        }
        for second in 5..10 {
            let url = Some("https://bank.example/statement");
            frame(
                &store,
                at(9, 0, second * 2),
                "Notion",
                "Bank",
                url,
                "balance 42",
            )
            .await;
        }
        etl::run_pass(&store, at(9, 1, 0), &[]).await.unwrap();
        let id = timeline_view(&store, &settings(), at(0, 0, 0), at(23, 0, 0), at(9, 1, 0))
            .await
            .unwrap()
            .sessions[0]
            .id;
        let now = at(9, 1, 0);
        let shown = visible_session_detail(&store, &settings(), id, now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(shown.windows.len(), 2);
        assert!(shown.text_excerpt.is_some());

        let excluding = ActivitySettings {
            ignored_domains: vec!["bank.example".into()],
            ..settings()
        };
        let filtered = visible_session_detail(&store, &excluding, id, now)
            .await
            .unwrap()
            .unwrap();
        let titles: Vec<_> = filtered
            .windows
            .iter()
            .map(|window| window.window_title.as_deref())
            .collect();
        assert_eq!(titles, vec![Some("Plan")]);
        assert_eq!(filtered.text_excerpt, None);

        let ignoring_app = ActivitySettings {
            ignored_apps: vec!["Notion".into()],
            ..settings()
        };
        assert!(visible_session_detail(&store, &ignoring_app, id, now)
            .await
            .unwrap()
            .is_none());
        let later = now + Duration::days(i64::from(settings().retention_days) + 1);
        assert!(visible_session_detail(&store, &settings(), id, later)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn search_hides_text_of_a_session_whose_domain_is_now_ignored() {
        let (_dir, store) = open_store().await;
        // The tab's URL is unknown on the first frames (Chromium still building
        // its tree), then the session learns its domain without splitting.
        for second in 0..5 {
            frame(
                &store,
                at(9, 0, second * 2),
                "Firefox",
                "Bank",
                None,
                &format!("wombat balance {second}"),
            )
            .await;
        }
        for second in 5..10 {
            frame(
                &store,
                at(9, 0, second * 2),
                "Firefox",
                "Bank",
                Some("https://bank.example/"),
                &format!("statement {second}"),
            )
            .await;
        }
        etl::run_pass(&store, at(9, 1, 0), &[]).await.unwrap();
        let mut excluding = settings();
        assert_eq!(
            search_view(&store, &excluding, "wombat", None, None, 10, at(9, 1, 0))
                .await
                .unwrap()
                .len(),
            1,
            "found while the domain is allowed"
        );
        excluding.ignored_domains = vec!["bank.example".into()];
        let view = timeline_view(&store, &excluding, at(0, 0, 0), at(23, 0, 0), at(9, 1, 0))
            .await
            .unwrap();
        assert!(view.sessions.is_empty());
        let hits = search_view(&store, &excluding, "wombat", None, None, 10, at(9, 1, 0))
            .await
            .unwrap();
        assert!(
            hits.is_empty(),
            "the URL-less first document leaked: {hits:?}"
        );
    }

    #[tokio::test]
    async fn search_keeps_paging_past_excluded_top_matches() {
        let (_dir, store) = open_store().await;
        // 120 short, dense matches in an app that becomes ignored rank above
        // the single allowed match in a long document.
        for index in 0..120 {
            frame(
                &store,
                at(10, 0, 0) + Duration::seconds(index * 2),
                "Vault",
                &format!("entry {index}"),
                None,
                "wombat wombat wombat",
            )
            .await;
        }
        let filler = "lorem ipsum dolor sit amet ".repeat(40);
        frame(
            &store,
            at(10, 5, 0),
            "Notes",
            "Draft",
            None,
            &format!("{filler} wombat {filler}"),
        )
        .await;
        etl::run_pass(&store, at(10, 5, 10), &[]).await.unwrap();
        let mut excluding = settings();
        excluding.ignored_apps = vec!["Vault".into()];
        let hits = search_view(&store, &excluding, "wombat", None, None, 1, at(10, 6, 0))
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].app_name, "Notes");
    }

    #[tokio::test]
    async fn an_unchanged_screen_shows_an_ongoing_idle_gap_before_activity_resumes() {
        let (_dir, store) = open_store().await;
        for index in 0..30 {
            frame(
                &store,
                at(9, 0, 0) + Duration::seconds(index * 2),
                "Preview",
                "doc.pdf",
                None,
                &format!("page {index}"),
            )
            .await;
        }
        // Ten more minutes of the same window and text, no input.
        for index in 1..=300 {
            frame(
                &store,
                at(9, 1, 0) + Duration::seconds(index * 2),
                "Preview",
                "doc.pdf",
                None,
                "page 29",
            )
            .await;
        }
        let now = at(9, 11, 5);
        etl::run_pass(&store, now, &[]).await.unwrap();
        let view = timeline_view(&store, &settings(), at(0, 0, 0), at(23, 0, 0), now)
            .await
            .unwrap();
        assert_eq!(view.gaps.len(), 1);
        let gap = &view.gaps[0];
        assert!(gap.ongoing);
        assert_eq!(gap.kind, GapKind::Idle);
        assert_eq!(gap.started_at, timestamp(at(9, 1, 0)));
        assert_eq!(gap.ended_at, timestamp(now));
        assert_eq!(view.stats.idle_ms, (now - at(9, 1, 0)).num_milliseconds());
        assert_eq!(view.stats.focused_ms, 60_000);

        // Activity resumes: the stored gap replaces the synthesized one.
        frame(&store, at(9, 12, 0), "Preview", "doc.pdf", None, "page 30").await;
        etl::run_pass(&store, at(9, 12, 1), &[]).await.unwrap();
        let view = timeline_view(&store, &settings(), at(0, 0, 0), at(23, 0, 0), at(9, 12, 1))
            .await
            .unwrap();
        assert_eq!(view.gaps.len(), 1);
        assert!(!view.gaps[0].ongoing);
        assert_eq!(view.gaps[0].ended_at, timestamp(at(9, 12, 0)));
    }

    #[tokio::test]
    async fn an_open_manual_pause_counts_as_away_while_it_lasts() {
        let (_dir, store) = open_store().await;
        for index in 0..10 {
            frame(
                &store,
                at(14, 0, 0) + Duration::seconds(index * 2),
                "Zed",
                "p \u{2014} a.rs",
                None,
                &format!("fn {index}"),
            )
            .await;
        }
        store
            .open_pause(PauseReason::Manual, at(14, 0, 25))
            .await
            .unwrap();
        let now = at(14, 30, 0);
        etl::run_pass(&store, now, &[]).await.unwrap();
        let view = timeline_view(&store, &settings(), at(0, 0, 0), at(23, 0, 0), now)
            .await
            .unwrap();
        let gap = view.gaps.last().expect("ongoing gap");
        assert!(gap.ongoing);
        assert_eq!(gap.kind, GapKind::Paused);
        assert_eq!(gap.pause_reason, Some(PauseReason::Manual));
        assert_eq!(view.stats.away_ms, (now - at(14, 0, 20)).num_milliseconds());
    }

    #[tokio::test]
    async fn meeting_evidence_window_starts_at_the_oldest_unprocessed_frame() {
        let (_dir, store) = open_store().await;
        let now = at(12, 0, 0);
        let old = now - Duration::days(20);
        frame(&store, old, "Slack", "huddle", None, "huddle").await;
        frame(
            &store,
            now - Duration::hours(1),
            "Slack",
            "general",
            None,
            "hi",
        )
        .await;
        assert_eq!(meeting_window_start(&store, now).await.unwrap(), old);

        etl::run_pass(&store, now, &[]).await.unwrap();
        frame(
            &store,
            now + Duration::minutes(1),
            "Slack",
            "general",
            None,
            "hello",
        )
        .await;
        // Processed history no longer widens the window: it starts at the
        // only unprocessed frame (the earlier session was closed by expiry).
        assert_eq!(
            meeting_window_start(&store, now + Duration::minutes(2))
                .await
                .unwrap(),
            now + Duration::minutes(1)
        );
    }
}
