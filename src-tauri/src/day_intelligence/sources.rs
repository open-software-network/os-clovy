//! What the day summary reads besides the timeline: meetings and notes the
//! user recorded in Clovy (main database) and coding-agent session blocks.

use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde::Serialize;
use sqlx::query::query;
use sqlx::row::Row;
use sqlx_sqlite::SqlitePool;
use tauri::AppHandle;

use super::schedule::LocalZone;
use crate::activity::store::{parse_time, ActivityStore};
use crate::coding_agents::settings::CodingAgentSources;
use crate::coding_agents::store::CodingAgentBlock;
use crate::coding_agents::CodingAgentsStatusDto;

const EXCERPT_CHARS: usize = 400;

/// One row of the notes query: note id, title, content, recording start and
/// end (none without a recording), and creation time.
pub type NoteRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    String,
);

/// A note recorded in Clovy that day: a meeting when it has a recording.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingNote {
    pub note_id: String,
    pub title: String,
    pub started_at: String,
    pub ended_at: String,
    /// Local "HH:MM".
    pub start_local: String,
    pub end_local: String,
    pub duration_ms: i64,
    pub recorded: bool,
    #[serde(skip)]
    pub excerpt: Option<String>,
}

impl MeetingNote {
    pub fn prompt_line(&self) -> String {
        let kind = if self.recorded { "meeting" } else { "note" };
        let title = if self.title.trim().is_empty() {
            "Untitled"
        } else {
            self.title.trim()
        };
        let mut line = format!(
            "{}-{} · {kind} \"{title}\" ({} min)",
            self.start_local,
            self.end_local,
            whole_minutes(self.duration_ms)
        );
        if let Some(excerpt) = &self.excerpt {
            line.push_str(": ");
            line.push_str(excerpt);
        }
        line
    }
}

/// A coding-agent session block (coding-agent ingestion), as the day
/// summary uses it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingBlock {
    pub source: String,
    pub project: Option<String>,
    pub title: Option<String>,
    pub started_at: String,
    pub ended_at: String,
    pub start_local: String,
    pub end_local: String,
    pub active_seconds: i64,
    #[serde(skip)]
    pub summary: Option<String>,
}

impl CodingBlock {
    pub fn prompt_line(&self) -> String {
        let mut line = format!(
            "{}-{} · {} · {} min",
            self.start_local,
            self.end_local,
            self.source,
            whole_minutes(self.active_seconds * 1000)
        );
        if let Some(project) = &self.project {
            line.push_str(&format!(" · project {project}"));
        }
        if let Some(title) = &self.title {
            line.push_str(&format!(" · \"{title}\""));
        }
        if let Some(summary) = &self.summary {
            line.push_str(": ");
            line.push_str(summary);
        }
        line
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DictationNote {
    pub local_time: String,
    pub app_bundle_id: Option<String>,
    pub app_name: Option<String>,
    pub text: String,
}

impl DictationNote {
    pub fn prompt_line(&self, limit: usize) -> String {
        let app = self.app_name.as_deref().or(self.app_bundle_id.as_deref());
        let text: String = self
            .text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(limit)
            .collect();
        match app {
            Some(app) => format!("{} · {}: {}", self.local_time, app, text),
            None => format!("{}: {}", self.local_time, text),
        }
    }
}

pub fn append_dictations(data: &mut String, notes: &[DictationNote], count: usize, chars: usize) {
    for note in notes.iter().take(count) {
        data.push_str(&format!("- {}\n", note.prompt_line(chars)));
    }
    if notes.len() > count {
        data.push_str(&format!("(+{} more)\n", notes.len() - count));
    }
}

pub trait DaySources: Send + Sync {
    fn dictations<'a>(
        &'a self,
        _from: DateTime<Utc>,
        _to: DateTime<Utc>,
        _zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<DictationNote>> {
        Box::pin(async { Vec::new() })
    }

    /// Notes recorded or created in `[from, to)`, oldest first.
    fn meetings<'a>(
        &'a self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<MeetingNote>>;

    /// Coding-agent blocks overlapping `[from, to)` from the sources in
    /// `enabled`, oldest first.
    fn coding_blocks<'a>(
        &'a self,
        store: &'a ActivityStore,
        enabled: &'a CodingAgentSources,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<CodingBlock>>;

    /// Whether coding-agent ingestion has read every enabled agent's
    /// transcripts up to `until` (true with every source off).
    fn coding_agents_read_until(&self, until: DateTime<Utc>) -> bool;
}

/// From the ingestion status: read past `until` when no source is on, when
/// the last scan finished at or after it, or when scanning fails (its error
/// shows in Settings; a broken source must not hold the summary forever).
pub fn ingestion_read_until(status: &CodingAgentsStatusDto, until: DateTime<Utc>) -> bool {
    if !status.supported || !status.sources.iter().any(|source| source.enabled) {
        return true;
    }
    if status.last_error.is_some() {
        return true;
    }
    status
        .last_scan_at
        .as_deref()
        .and_then(parse_time)
        .is_some_and(|at| at >= until)
}

pub fn excerpt(text: &str) -> Option<String> {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    (!collapsed.is_empty()).then(|| collapsed.chars().take(EXCERPT_CHARS).collect())
}

/// Whole minutes of a duration, rounded to the nearest minute: the one
/// rounding every prompt and measure of day intelligence uses.
pub fn whole_minutes(ms: i64) -> i64 {
    (ms + 30_000).div_euclid(60_000)
}

pub fn hhmm(zone: &dyn LocalZone, at: DateTime<Utc>) -> String {
    zone.local(at).format("%H:%M").to_string()
}

/// Groups note rows (one per recording session, or one without a recording)
/// into notes: a meeting spans its first recording start to its last end; a
/// note without a recording sits at its creation time. A recording counts
/// only its part inside `[from, to)`, so a meeting across midnight splits its
/// duration between the two days.
pub fn meetings_from_rows(
    rows: Vec<NoteRow>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    zone: &dyn LocalZone,
    now: DateTime<Utc>,
) -> Vec<MeetingNote> {
    let mut notes: Vec<MeetingNote> = Vec::new();
    for (note_id, title, content, started, ended, created) in rows {
        let (start, end, recorded) = match started.as_deref().and_then(parse_time) {
            Some(start) => {
                let end = ended
                    .as_deref()
                    .and_then(parse_time)
                    .unwrap_or(now)
                    .max(start);
                (start, end, true)
            }
            None => match parse_time(&created) {
                Some(at) => (at, at, false),
                None => continue,
            },
        };
        let overlaps = if recorded {
            start < to && (end > from || start >= from)
        } else {
            start >= from && start < to
        };
        let inside_ms = if recorded {
            (end.min(to) - start.max(from)).num_milliseconds().max(0)
        } else {
            0
        };
        if !overlaps {
            continue;
        }
        if let Some(existing) = notes.iter_mut().find(|note| note.note_id == note_id) {
            let existing_start = parse_time(&existing.started_at).unwrap_or(start);
            let existing_end = parse_time(&existing.ended_at).unwrap_or(end);
            if recorded {
                existing.duration_ms += inside_ms;
                existing.recorded = true;
            }
            let start = start.min(existing_start);
            let end = end.max(existing_end);
            existing.started_at = start.to_rfc3339();
            existing.ended_at = end.to_rfc3339();
            existing.start_local = hhmm(zone, start);
            existing.end_local = hhmm(zone, end);
            continue;
        }
        notes.push(MeetingNote {
            note_id,
            title,
            started_at: start.to_rfc3339(),
            ended_at: end.to_rfc3339(),
            start_local: hhmm(zone, start),
            end_local: hhmm(zone, end),
            duration_ms: inside_ms,
            recorded,
            excerpt: excerpt(&content),
        });
    }
    notes.sort_by(|a, b| a.started_at.cmp(&b.started_at));
    notes
}

/// The app's sources: the main database and the activity database.
pub struct AppSources {
    pub app: AppHandle,
    /// Host-bound note partition. Activity and coding-agent stores stay installation-wide.
    pub profile: Option<String>,
}
pub(crate) fn profile_for_notes(pinned: Option<&str>, current: impl FnOnce() -> String) -> String {
    pinned.map(str::to_owned).unwrap_or_else(current)
}

impl DaySources for AppSources {
    fn dictations<'a>(
        &'a self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<DictationNote>> {
        Box::pin(async move {
            let repositories = match crate::commands::repositories(&self.app).await {
                Ok(repositories) => repositories,
                Err(error) => {
                    tracing::warn!(code = %error.code, "day summary: dictations repository unavailable");
                    return Vec::new();
                }
            };
            let profile = profile_for_notes(self.profile.as_deref(), || {
                crate::commands::active_profile(&self.app)
            });
            let cutoff = crate::db::repositories::dictation_history_cutoff_timestamp();
            dictations_between(&repositories.pool, &profile, from, to, zone, &cutoff).await
        })
    }

    fn meetings<'a>(
        &'a self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<MeetingNote>> {
        Box::pin(async move {
            let Ok(repositories) = crate::commands::repositories(&self.app).await else {
                return Vec::new();
            };
            let profile = profile_for_notes(self.profile.as_deref(), || {
                crate::commands::active_profile(&self.app)
            });
            meetings_between(&repositories.pool, &profile, from, to, zone, Utc::now()).await
        })
    }

    fn coding_blocks<'a>(
        &'a self,
        store: &'a ActivityStore,
        enabled: &'a CodingAgentSources,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        zone: &'a dyn LocalZone,
    ) -> BoxFuture<'a, Vec<CodingBlock>> {
        Box::pin(coding_blocks_between(store, enabled, from, to, zone))
    }

    fn coding_agents_read_until(&self, until: DateTime<Utc>) -> bool {
        use tauri::Manager;
        self.app
            .try_state::<crate::coding_agents::CodingAgentsState>()
            .map_or(true, |state| {
                ingestion_read_until(&crate::coding_agents::coding_agents_status(state), until)
            })
    }
}

/// Notes of the data partition `profile` recorded or created in
/// `[from, to)` (`DaySources::meetings`), from the main database `pool`;
/// a recording still going on ends at `now`.
pub(crate) async fn meetings_between(
    pool: &SqlitePool,
    profile: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    zone: &dyn LocalZone,
    now: DateTime<Utc>,
) -> Vec<MeetingNote> {
    let from_text = from.to_rfc3339();
    let to_text = to.to_rfc3339();
    let rows = query(
        "SELECT n.id, n.title, COALESCE(n.edited_content, n.generated_content, ''),
                    r.started_at, r.ended_at, n.created_at
             FROM notes n LEFT JOIN recording_sessions r ON r.note_id = n.id
             WHERE n.profile = ? AND (
                (r.started_at IS NOT NULL
                  AND julianday(r.started_at) < julianday(?)
                  AND (r.ended_at IS NULL OR julianday(r.ended_at) > julianday(?)
                    OR julianday(r.started_at) >= julianday(?)))
                OR (r.id IS NULL
                  AND julianday(n.created_at) >= julianday(?)
                  AND julianday(n.created_at) < julianday(?)))
             ORDER BY COALESCE(r.started_at, n.created_at)",
    )
    .bind(profile)
    .bind(&to_text)
    .bind(&from_text)
    .bind(&from_text)
    .bind(&from_text)
    .bind(&to_text)
    .fetch_all(pool)
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(%error, "day summary: meetings could not be read");
        Vec::new()
    });
    let rows = rows
        .iter()
        .map(|row| {
            (
                row.get::<String, _>(0),
                row.get::<String, _>(1),
                row.get::<String, _>(2),
                row.get::<Option<String>, _>(3),
                row.get::<Option<String>, _>(4),
                row.get::<String, _>(5),
            )
        })
        .collect();
    meetings_from_rows(rows, from, to, zone, now)
}

pub(crate) async fn dictations_between(
    pool: &SqlitePool,
    profile: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    zone: &dyn LocalZone,
    cutoff: &str,
) -> Vec<DictationNote> {
    let rows = query("SELECT created_at, app_bundle_id, text FROM dictation_history WHERE profile = ? AND julianday(created_at) >= julianday(?) AND julianday(created_at) < julianday(?) AND julianday(created_at) >= julianday(?) ORDER BY julianday(created_at), rowid")
        .bind(profile).bind(from.to_rfc3339()).bind(to.to_rfc3339()).bind(cutoff).fetch_all(pool).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "day summary: dictations could not be read"); Vec::new()
        });
    rows.into_iter()
        .filter_map(|row| {
            let created_at = row.get::<String, _>(0);
            let Some(at) = parse_time(&created_at) else {
                tracing::warn!("day intelligence: dictation timestamp is invalid");
                return None;
            };
            Some(DictationNote {
                local_time: hhmm(zone, at),
                app_bundle_id: row.get(1),
                app_name: None,
                text: row.get(2),
            })
        })
        .collect()
}

/// Coding-agent blocks from the activity database
/// (`ActivityStore::enabled_coding_agent_blocks_between`, docs/coding-agent-sessions.md)
/// of the sources still turned on: turning a source off removes its blocks
/// from reports and summaries, as from the MCP server.
pub(crate) async fn coding_blocks_between(
    store: &ActivityStore,
    enabled: &CodingAgentSources,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    zone: &dyn LocalZone,
) -> Vec<CodingBlock> {
    match store
        .enabled_coding_agent_blocks_between(enabled, from, to)
        .await
    {
        Ok(blocks) => blocks_in_window(blocks, from, to, zone),
        Err(error) => {
            tracing::warn!(%error, "day intelligence: coding-agent blocks could not be read");
            Vec::new()
        }
    }
}

/// Stored blocks as the hour report and the day summary use them. A block
/// counts only inside `[from, to)` (one ending exactly at `from` is not in
/// it), and its active time is split by the share of the block inside, so a
/// block across two hours or two days is not counted twice. Text: the block's
/// summary once summarized, else its first prompt.
pub fn blocks_in_window(
    blocks: Vec<CodingAgentBlock>,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    zone: &dyn LocalZone,
) -> Vec<CodingBlock> {
    blocks
        .into_iter()
        .filter_map(|block| {
            let start = parse_time(&block.started_at)?;
            let end = parse_time(&block.ended_at)?.max(start);
            if start >= to || (end <= from && start < from) {
                return None;
            }
            let span_ms = (end - start).num_milliseconds();
            let inside_ms = (end.min(to) - start.max(from)).num_milliseconds().max(0);
            let active_seconds = if span_ms > 0 {
                ((block.active_seconds as i128 * inside_ms as i128 + span_ms as i128 / 2)
                    / span_ms as i128) as i64
            } else {
                block.active_seconds
            };
            Some(CodingBlock {
                source: block.source.display_name().to_string(),
                project: block.project,
                title: block.title,
                start_local: hhmm(zone, start),
                end_local: hhmm(zone, end),
                started_at: block.started_at,
                ended_at: block.ended_at,
                active_seconds,
                summary: block.summary.or(block.first_prompt),
            })
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::schedule::FixedZone;
    use super::*;

    #[tokio::test]
    async fn dictations_query_is_partitioned_ordered_and_half_open() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        query("CREATE TABLE dictation_history (profile TEXT, created_at TEXT, app_bundle_id TEXT, text TEXT)").execute(&pool).await.unwrap();
        for (profile, at, text) in [
            ("a", "2026-01-01T14:30:00Z", "second"),
            ("b", "2026-01-01T14:15:00Z", "other"),
            ("a", "2026-01-01T15:00:00Z", "outside"),
            ("a", "2026-01-01T14:00:00Z", "first"),
        ] {
            query("INSERT INTO dictation_history VALUES (?, ?, ?, ?)")
                .bind(profile)
                .bind(at)
                .bind("org.example.app")
                .bind(text)
                .execute(&pool)
                .await
                .unwrap();
        }
        let from = parse_time("2026-01-01T14:00:00Z").unwrap();
        let to = parse_time("2026-01-01T15:00:00Z").unwrap();
        let zone = FixedZone(chrono::FixedOffset::east_opt(0).unwrap());
        let notes = dictations_between(&pool, "a", from, to, &zone, "2025-12-25T00:00:00Z").await;
        assert_eq!(
            notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(notes[0].local_time, "14:00");
        assert_eq!(notes[0].app_bundle_id.as_deref(), Some("org.example.app"));
    }

    #[tokio::test]
    async fn dictations_exclude_expired_rows_without_deleting_them() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        query("CREATE TABLE dictation_history (profile TEXT, created_at TEXT, app_bundle_id TEXT, text TEXT)").execute(&pool).await.unwrap();
        for (profile, at, text) in [
            ("a", "2026-10-24T11:59:59.999Z", "expired"),
            ("a", "2026-10-24T12:00:00.000Z", "at cutoff"),
            ("a", "2026-10-25T09:30:00-03:00", "recent"),
            ("b", "2026-10-25T12:30:00Z", "other profile"),
            ("a", "2026-10-26T00:00:00Z", "outside"),
        ] {
            query("INSERT INTO dictation_history VALUES (?, ?, NULL, ?)")
                .bind(profile)
                .bind(at)
                .bind(text)
                .execute(&pool)
                .await
                .unwrap();
        }
        let from = parse_time("2026-10-24T00:00:00Z").unwrap();
        let to = parse_time("2026-10-26T00:00:00Z").unwrap();
        let zone = FixedZone(chrono::FixedOffset::east_opt(0).unwrap());
        let notes =
            dictations_between(&pool, "a", from, to, &zone, "2026-10-24T12:00:00.000Z").await;
        assert_eq!(
            notes.iter().map(|n| n.text.as_str()).collect::<Vec<_>>(),
            ["at cutoff", "recent"]
        );
        assert_eq!(notes[1].local_time, "12:30");
        let count: i64 = query("SELECT COUNT(*) FROM dictation_history WHERE text = 'expired'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get(0);
        assert_eq!(count, 1, "expired row remains physically present");
    }

    /// Fixed meetings and blocks, filtered by overlap like the real sources.
    /// `ingestion_scanned_at`: the last coding-agent scan (`None`: up to date).
    #[derive(Default)]
    pub(crate) struct FixedSources {
        pub meetings: Vec<MeetingNote>,
        pub dictations: Vec<(DateTime<Utc>, DictationNote)>,
        pub blocks: Vec<CodingBlock>,
        pub ingestion_scanned_at: std::sync::Mutex<Option<DateTime<Utc>>>,
    }

    /// Sessions, meetings, and coding blocks in one prompt round their
    /// minutes the same way (2.5 min is 3, not 2).
    #[test]
    fn prompt_lines_round_minutes_like_the_measured_sessions() {
        assert_eq!(whole_minutes(150_000), 3);
        assert_eq!(whole_minutes(89_000), 1);
        let block = CodingBlock {
            source: "Codex".into(),
            project: None,
            title: None,
            started_at: "2026-10-04T14:00:00Z".into(),
            ended_at: "2026-10-04T14:03:00Z".into(),
            start_local: "11:00".into(),
            end_local: "11:03".into(),
            active_seconds: 150,
            summary: None,
        };
        assert_eq!(block.prompt_line(), "11:00-11:03 · Codex · 3 min");
        let meeting = MeetingNote {
            note_id: "n".into(),
            title: "Standup".into(),
            started_at: "2026-10-04T14:00:00Z".into(),
            ended_at: "2026-10-04T14:03:00Z".into(),
            start_local: "11:00".into(),
            end_local: "11:03".into(),
            duration_ms: 150_000,
            recorded: true,
            excerpt: None,
        };
        assert_eq!(
            meeting.prompt_line(),
            "11:00-11:03 · meeting \"Standup\" (3 min)"
        );
    }

    fn overlaps(start: &str, end: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
        let start = parse_time(start).unwrap();
        let end = parse_time(end).unwrap();
        start < to && (end > from || start >= from)
    }

    impl DaySources for FixedSources {
        fn dictations<'a>(
            &'a self,
            from: DateTime<Utc>,
            to: DateTime<Utc>,
            _zone: &'a dyn LocalZone,
        ) -> BoxFuture<'a, Vec<DictationNote>> {
            let found = self
                .dictations
                .iter()
                .filter(|(at, _)| *at >= from && *at < to)
                .map(|(_, note)| note.clone())
                .collect();
            Box::pin(async move { found })
        }
        fn meetings<'a>(
            &'a self,
            from: DateTime<Utc>,
            to: DateTime<Utc>,
            _zone: &'a dyn LocalZone,
        ) -> BoxFuture<'a, Vec<MeetingNote>> {
            let found = self
                .meetings
                .iter()
                .filter(|note| overlaps(&note.started_at, &note.ended_at, from, to))
                .cloned()
                .collect();
            Box::pin(async move { found })
        }

        fn coding_blocks<'a>(
            &'a self,
            _store: &'a ActivityStore,
            _enabled: &'a CodingAgentSources,
            from: DateTime<Utc>,
            to: DateTime<Utc>,
            _zone: &'a dyn LocalZone,
        ) -> BoxFuture<'a, Vec<CodingBlock>> {
            let found = self
                .blocks
                .iter()
                .filter(|block| overlaps(&block.started_at, &block.ended_at, from, to))
                .cloned()
                .collect();
            Box::pin(async move { found })
        }

        fn coding_agents_read_until(&self, until: DateTime<Utc>) -> bool {
            crate::day_intelligence::lock(&self.ingestion_scanned_at)
                .map_or(true, |scanned| scanned >= until)
        }
    }

    #[test]
    fn recordings_of_one_note_merge_and_plain_notes_count_by_creation() {
        let zone = FixedZone(chrono::FixedOffset::west_opt(3 * 3600).unwrap());
        let from = parse_time("2026-10-04T03:00:00Z").unwrap();
        let to = parse_time("2026-10-05T03:00:00Z").unwrap();
        let now = parse_time("2026-10-04T23:00:00Z").unwrap();
        let rows = vec![
            (
                "n1".into(),
                "Weekly sync".into(),
                "## Decisions\n\nShip   the beta".into(),
                Some("2026-10-04T18:00:00.000Z".into()),
                Some("2026-10-04T18:20:00.000Z".into()),
                "2026-10-04T18:00:00.000Z".into(),
            ),
            (
                "n1".into(),
                "Weekly sync".into(),
                String::new(),
                Some("2026-10-04T18:25:00.000Z".into()),
                Some("2026-10-04T18:35:00.000Z".into()),
                "2026-10-04T18:00:00.000Z".into(),
            ),
            (
                "n2".into(),
                "Ideas".into(),
                "draft".into(),
                None,
                None,
                "2026-10-04T12:00:00Z".into(),
            ),
            (
                "n3".into(),
                "Yesterday".into(),
                String::new(),
                None,
                None,
                "2026-10-03T12:00:00Z".into(),
            ),
        ];
        let notes = meetings_from_rows(rows, from, to, &zone, now);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].title, "Ideas");
        assert!(!notes[0].recorded);
        let meeting = &notes[1];
        assert_eq!(
            (meeting.start_local.as_str(), meeting.end_local.as_str()),
            ("15:00", "15:35")
        );
        assert_eq!(meeting.duration_ms, 30 * 60_000);
        assert_eq!(
            meeting.excerpt.as_deref(),
            Some("## Decisions Ship the beta")
        );
        assert!(meeting
            .prompt_line()
            .contains("meeting \"Weekly sync\" (30 min)"));
    }

    #[test]
    fn a_meeting_across_midnight_splits_its_minutes_between_the_days() {
        let zone = FixedZone(chrono::FixedOffset::west_opt(3 * 3600).unwrap());
        let midnight = parse_time("2026-10-05T03:00:00Z").unwrap();
        let day = chrono::Duration::days(1);
        let now = parse_time("2026-10-05T12:00:00Z").unwrap();
        let row = |id: &str, start: &str, end: &str| {
            (
                id.to_string(),
                "Late call".to_string(),
                String::new(),
                Some(start.to_string()),
                Some(end.to_string()),
                start.to_string(),
            )
        };
        let rows = || {
            vec![
                row("n1", "2026-10-05T02:50:00Z", "2026-10-05T03:10:00Z"),
                row("n2", "2026-10-05T02:30:00Z", "2026-10-05T03:00:00Z"),
            ]
        };

        let before = meetings_from_rows(rows(), midnight - day, midnight, &zone, now);
        let after = meetings_from_rows(rows(), midnight, midnight + day, &zone, now);

        let minutes = |notes: &[MeetingNote]| {
            notes
                .iter()
                .map(|note| (note.note_id.clone(), note.duration_ms / 60_000))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            minutes(&before),
            vec![("n2".to_string(), 30), ("n1".to_string(), 10)]
        );
        // n2 ended exactly at midnight: it is not part of the next day.
        assert_eq!(minutes(&after), vec![("n1".to_string(), 10)]);
    }

    #[test]
    fn ingestion_holds_the_summary_until_a_scan_passes_the_last_hour() {
        use crate::coding_agents::{SourceId, SourceStatusDto};
        let until = parse_time("2026-10-04T21:00:00Z").unwrap();
        let status =
            |enabled: bool, scanned: Option<&str>, error: Option<&str>| CodingAgentsStatusDto {
                supported: true,
                sources: vec![SourceStatusDto {
                    id: SourceId::Codex,
                    name: "Codex",
                    enabled,
                    present: true,
                }],
                database_ready: true,
                last_scan_at: scanned.map(str::to_string),
                last_error: error.map(str::to_string),
            };
        assert!(ingestion_read_until(&status(false, None, None), until));
        assert!(!ingestion_read_until(&status(true, None, None), until));
        assert!(!ingestion_read_until(
            &status(true, Some("2026-10-04T20:59:00.000000Z"), None),
            until
        ));
        assert!(ingestion_read_until(
            &status(true, Some("2026-10-04T21:00:30.000000Z"), None),
            until
        ));
        assert!(ingestion_read_until(
            &status(
                true,
                Some("2026-10-04T20:00:00.000000Z"),
                Some("permission denied")
            ),
            until
        ));
    }
}

#[cfg(test)]
mod bound_note_profile_tests {
    use super::*;
    #[tokio::test]
    async fn bound_note_context_ignores_later_ui_partition_selection() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        query("CREATE TABLE notes(id TEXT PRIMARY KEY,title TEXT,edited_content TEXT,generated_content TEXT,created_at TEXT,profile TEXT)").execute(&pool).await.unwrap();
        query("CREATE TABLE recording_sessions(id TEXT PRIMARY KEY,note_id TEXT,started_at TEXT,ended_at TEXT)").execute(&pool).await.unwrap();
        query("INSERT INTO notes VALUES('bound','Bound note','Bound transcript',NULL,'2026-11-01T12:00:00Z','bound-profile'),('foreign','Foreign note','Foreign transcript',NULL,'2026-11-01T12:00:00Z','foreign-profile')").execute(&pool).await.unwrap();
        let profile = profile_for_notes(Some("bound-profile"), || "foreign-profile".into());
        let from = DateTime::parse_from_rfc3339("2026-11-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let to = from + chrono::Duration::days(1);
        let notes = meetings_between(
            &pool,
            &profile,
            from,
            to,
            &crate::day_intelligence::schedule::SystemZone,
            to,
        )
        .await;
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].note_id, "bound");
        assert_eq!(
            profile_for_notes(None, || "foreign-profile".into()),
            "foreign-profile"
        );
    }
    #[test]
    fn a_pinned_partition_does_not_consult_mutable_ui_selection() {
        let selected = profile_for_notes(Some("bound-profile"), || {
            panic!("bound read consulted UI selection")
        });
        assert_eq!(selected, "bound-profile");
    }
}
