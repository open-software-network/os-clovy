//! One incremental ETL pass: read frames after the timeline cursor in batches
//! of up to 500, feed them to the builder with that batch's input and pause
//! records, and persist each batch atomically with the cursor. A pass stops
//! after its frame budget, so a historical backfill never holds a reader for
//! long; only a pass that caught up closes a session left open past the gap
//! threshold (with a backlog, later frames may still continue it).

use chrono::{DateTime, Duration, Utc};

use super::builder::{self, BatchSignals, PauseSpan, TimelineEvent};
use super::db::{self, Interval};
use crate::activity::store::parse_time;
use crate::activity::store::{ActivityStore, StoreError, TIMELINE_CONSUMER};

pub const BATCH_SIZE: u32 = 500;
/// Input events are read slightly around the batch so the first and last
/// frames see the input that happened during their tick.
const INPUT_MARGIN: Duration = Duration::seconds(5);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassReport {
    pub frames: usize,
    /// Sessions or gaps were written.
    pub changed: bool,
    /// No unprocessed frame was left when the pass ended.
    pub caught_up: bool,
}

/// Pause records overlapping `[from, to)`, an open one running to `to`.
pub(super) async fn pause_spans(
    store: &ActivityStore,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<PauseSpan>, StoreError> {
    Ok(store
        .pauses_between(from, to)
        .await?
        .iter()
        .filter_map(|pause| {
            Some(PauseSpan {
                started_at: parse_time(&pause.started_at)?,
                ended_at: pause.ended_at.as_deref().and_then(parse_time),
                reason: pause.reason,
            })
        })
        .collect())
}

/// Processes every unprocessed frame (tests and one-off catch-up).
pub async fn run_pass(
    store: &ActivityStore,
    now: DateTime<Utc>,
    meetings: &[Interval],
) -> Result<PassReport, StoreError> {
    run_pass_bounded(store, now, meetings, usize::MAX).await
}

/// Processes at most `max_frames` unprocessed frames. `meetings` are
/// recording intervals (meeting audio evidence for the categorizer).
pub async fn run_pass_bounded(
    store: &ActivityStore,
    now: DateTime<Utc>,
    meetings: &[Interval],
    max_frames: usize,
) -> Result<PassReport, StoreError> {
    let mut report = PassReport::default();
    let mut state = db::load_state(store).await?;
    loop {
        let budget = max_frames.saturating_sub(report.frames);
        if budget == 0 {
            break;
        }
        let limit = u32::try_from(budget).unwrap_or(BATCH_SIZE).min(BATCH_SIZE);
        let cursor = store.processing_cursor(TIMELINE_CONSUMER).await?;
        let frames = store.frames_after(cursor.last_frame_id, limit).await?;
        let (Some(first), Some(last)) = (frames.first(), frames.last()) else {
            report.caught_up = true;
            break;
        };
        let first_at = parse_time(&first.captured_at).unwrap_or(now);
        let last_at = parse_time(&last.captured_at).unwrap_or(now);
        let (batch_from, batch_to) = (first_at.min(last_at), first_at.max(last_at));

        let mut input_times: Vec<DateTime<Utc>> = store
            .input_events_between(batch_from - INPUT_MARGIN, batch_to + INPUT_MARGIN)
            .await?
            .iter()
            .filter_map(|event| parse_time(&event.occurred_at))
            .collect();
        input_times.sort_unstable();
        let pauses_from = state
            .builder
            .last_useful_at
            .map_or(batch_from, |at| at.min(batch_from));
        let pauses = pause_spans(store, pauses_from, batch_to + Duration::seconds(1)).await?;
        let signals = BatchSignals {
            input_times,
            pauses,
        };

        let mut events: Vec<TimelineEvent> = Vec::new();
        for frame in &frames {
            builder::push_frame(&mut state.builder, frame, &signals, &mut events);
        }
        db::persist_batch(
            store,
            &mut state,
            &events,
            meetings,
            Some((last.id, last_at)),
        )
        .await?;
        report.frames += frames.len();
        report.changed = true;
        if frames.len() < limit as usize {
            report.caught_up = true;
            break;
        }
    }

    if !report.caught_up {
        return Ok(report);
    }
    let mut events = Vec::new();
    builder::expire(&mut state.builder, now, &mut events);
    if !events.is_empty() {
        db::persist_batch(store, &mut state, &events, meetings, None).await?;
        report.changed = true;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::key::MemoryKeyStore;
    use crate::activity::store::{
        InputEventKind, NewFrame, NewInputEvent, PauseReason, TextSource, ACTIVITY_DB_FILE,
    };
    use crate::activity::timeline::builder::GapKind;
    use crate::activity::timeline::categorize::Category;
    use chrono::TimeZone;

    fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, hour, minute, second)
            .unwrap()
    }

    fn new_frame(
        when: DateTime<Utc>,
        app: &str,
        title: &str,
        url: Option<&str>,
        text: &str,
    ) -> NewFrame {
        NewFrame {
            captured_at: when,
            app_name: app.into(),
            bundle_id: None,
            window_title: Some(title.into()),
            browser_url: url.map(str::to_string),
            text_source: TextSource::Accessibility,
            text: Some(text.into()),
            input_idle_ms: None,
        }
    }

    async fn insert_run(
        store: &ActivityStore,
        start: DateTime<Utc>,
        count: i64,
        app: &str,
        title: &str,
        url: Option<&str>,
        text: &str,
    ) {
        for index in 0..count {
            store
                .insert_frame(&new_frame(
                    start + Duration::seconds(index * 2),
                    app,
                    title,
                    url,
                    &format!("{text}\nline {index}"),
                ))
                .await
                .unwrap();
        }
    }

    async fn open_store(dir: &tempfile::TempDir, keys: &MemoryKeyStore) -> ActivityStore {
        ActivityStore::open(&dir.path().join(ACTIVITY_DB_FILE), keys)
            .await
            .unwrap()
    }

    /// The day as the view sees it, reduced to comparable facts.
    async fn day(
        store: &ActivityStore,
    ) -> (Vec<(String, i64, i64, bool, Category)>, Vec<(GapKind, i64)>) {
        let sessions = db::sessions_between(store, at(0, 0, 0), at(23, 59, 59))
            .await
            .unwrap()
            .into_iter()
            .map(|session| {
                (
                    session.app_name,
                    session.duration_ms,
                    session.id,
                    session.active,
                    session.category,
                )
            })
            .collect();
        let gaps = db::gaps_between(store, at(0, 0, 0), at(23, 59, 59))
            .await
            .unwrap()
            .into_iter()
            .map(|gap| (gap.kind, gap.duration_ms))
            .collect();
        (sessions, gaps)
    }

    async fn frame_total(store: &ActivityStore) -> (i64, i64) {
        let rows = sqlx::query::query(
            "SELECT COALESCE(SUM(frame_count), 0), COALESCE(SUM(last_frame_id - first_frame_id + 1), 0)
             FROM timeline_sessions",
        )
        .fetch_one(store.pool())
        .await
        .unwrap();
        use sqlx::row::Row;
        (rows.get(0), rows.get(1))
    }

    /// A window's document gathers every new line, frames of one pass and
    /// of the next alike (the body is merged in memory within a batch).
    #[tokio::test]
    async fn a_document_keeps_the_new_lines_of_every_frame_across_passes() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &MemoryKeyStore::default()).await;
        insert_run(&store, at(9, 0, 0), 20, "Zed", "notes.md", None, "header").await;
        run_pass(&store, at(9, 0, 40), &[]).await.unwrap();
        insert_run(&store, at(9, 0, 40), 10, "Zed", "notes.md", None, "later").await;
        run_pass(&store, at(9, 1, 0), &[]).await.unwrap();
        let bodies: Vec<String> = sqlx::query::query("SELECT body FROM timeline_search")
            .fetch_all(store.pool())
            .await
            .unwrap()
            .iter()
            .map(|row| sqlx::row::Row::get(row, 0))
            .collect();
        assert_eq!(bodies.len(), 1, "{bodies:?}");
        let lines: Vec<&str> = bodies[0].lines().collect();
        let mut expected = vec!["header"];
        let numbered: Vec<String> = (0..20).map(|index| format!("line {index}")).collect();
        expected.extend(numbered.iter().map(String::as_str));
        expected.push("later");
        assert_eq!(lines, expected);
    }

    #[tokio::test]
    async fn persisted_system_idle_keeps_scrolling_focused_across_passes() {
        let dir = tempfile::tempdir().unwrap();
        let store = open_store(&dir, &MemoryKeyStore::default()).await;
        for index in 0..=300 {
            store
                .insert_frame(&NewFrame {
                    input_idle_ms: Some(1000),
                    ..new_frame(
                        at(10, 0, 0) + Duration::seconds(index * 2),
                        "Preview",
                        "doc.pdf",
                        None,
                        "unchanged document",
                    )
                })
                .await
                .unwrap();
        }
        let first = run_pass_bounded(&store, at(10, 10, 2), &[], 150)
            .await
            .unwrap();
        assert!(!first.caught_up);
        run_pass(&store, at(10, 10, 2), &[]).await.unwrap();
        let (sessions, gaps) = day(&store).await;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].1, 602_000);
        assert!(gaps.is_empty());
        assert_eq!(
            store
                .processing_cursor(TIMELINE_CONSUMER)
                .await
                .unwrap()
                .last_frame_id,
            301
        );
    }

    async fn seed_day(store: &ActivityStore) {
        // 10 min editor, 5 min browser, 20 min manual pause, then Slack.
        insert_run(
            store,
            at(9, 0, 0),
            300,
            "Code",
            "store.rs \u{2014} os-clovy",
            None,
            "fn main() {}",
        )
        .await;
        insert_run(
            store,
            at(9, 10, 0),
            150,
            "Google Chrome",
            "SQLite FTS5 docs",
            Some("https://www.sqlite.org/fts5.html"),
            "The quasar extension module",
        )
        .await;
        let pause = store
            .open_pause(PauseReason::Manual, at(9, 15, 0))
            .await
            .unwrap();
        store.close_pause(pause, at(9, 35, 0)).await.unwrap();
        insert_run(
            store,
            at(9, 35, 0),
            30,
            "Slack",
            "general",
            None,
            "standup notes",
        )
        .await;
    }

    #[tokio::test]
    async fn a_day_becomes_sessions_a_paused_gap_and_searchable_text() {
        let dir = tempfile::tempdir().unwrap();
        let keys = MemoryKeyStore::default();
        let store = open_store(&dir, &keys).await;
        seed_day(&store).await;

        let report = run_pass(&store, at(9, 36, 30), &[]).await.unwrap();
        assert_eq!(report.frames, 480);
        let (sessions, gaps) = day(&store).await;
        let summary: Vec<(&str, i64, bool)> = sessions
            .iter()
            .map(|(app, ms, _, active, _)| (app.as_str(), *ms, *active))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Code", 600_000, false),
                ("Google Chrome", 300_000, false),
                ("Slack", 60_000, true),
            ]
        );
        assert_eq!(sessions[0].4, Category::Coding);
        assert_eq!(gaps, vec![(GapKind::Paused, 20 * 60_000)]);
        assert_eq!(frame_total(&store).await, (480, 480));

        let cursor = store.processing_cursor(TIMELINE_CONSUMER).await.unwrap();
        assert_eq!(cursor.last_frame_id, 480);

        let hits = db::search(&store, "quasar", at(0, 0, 0), at(23, 0, 0), 10, 0)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].session_id, sessions[1].2);
        assert_eq!(
            hits[0].seen_at,
            crate::activity::store::timestamp(at(9, 10, 0))
        );
        assert!(hits[0].snippet.contains("quasar"));
        // The period filter excludes it.
        assert!(
            db::search(&store, "quasar", at(10, 0, 0), at(23, 0, 0), 10, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn interrupted_processing_resumes_without_double_or_skipped_frames() {
        // Reference: everything in one pass.
        let reference_dir = tempfile::tempdir().unwrap();
        let reference_keys = MemoryKeyStore::default();
        let reference = open_store(&reference_dir, &reference_keys).await;
        seed_day(&reference).await;
        run_pass(&reference, at(9, 36, 30), &[]).await.unwrap();
        let expected = day(&reference).await;

        // Same frames, but the app "restarts" mid-way: a pass over the first
        // part, the store closed and reopened, more frames, another pass.
        let dir = tempfile::tempdir().unwrap();
        let keys = MemoryKeyStore::default();
        let store = open_store(&dir, &keys).await;
        insert_run(
            &store,
            at(9, 0, 0),
            300,
            "Code",
            "store.rs \u{2014} os-clovy",
            None,
            "fn main() {}",
        )
        .await;
        insert_run(
            &store,
            at(9, 10, 0),
            77,
            "Google Chrome",
            "SQLite FTS5 docs",
            Some("https://www.sqlite.org/fts5.html"),
            "The quasar extension module",
        )
        .await;
        let first = run_pass(&store, at(9, 12, 34), &[]).await.unwrap();
        assert_eq!(first.frames, 377);
        store.close().await;

        let store = open_store(&dir, &keys).await;
        for index in 77..150 {
            store
                .insert_frame(&new_frame(
                    at(9, 10, 0) + Duration::seconds(index * 2),
                    "Google Chrome",
                    "SQLite FTS5 docs",
                    Some("https://www.sqlite.org/fts5.html"),
                    &format!("The quasar extension module\nline {index}"),
                ))
                .await
                .unwrap();
        }
        let pause = store
            .open_pause(PauseReason::Manual, at(9, 15, 0))
            .await
            .unwrap();
        store.close_pause(pause, at(9, 35, 0)).await.unwrap();
        insert_run(
            &store,
            at(9, 35, 0),
            30,
            "Slack",
            "general",
            None,
            "standup notes",
        )
        .await;
        let second = run_pass(&store, at(9, 36, 30), &[]).await.unwrap();
        assert_eq!(second.frames, 103);
        // A pass with nothing new changes nothing.
        let third = run_pass(&store, at(9, 36, 31), &[]).await.unwrap();
        assert_eq!(
            third,
            PassReport {
                caught_up: true,
                ..PassReport::default()
            }
        );

        assert_eq!(day(&store).await, expected);
        assert_eq!(frame_total(&store).await, (480, 480));
    }

    #[tokio::test]
    async fn retention_removes_old_sessions_gaps_and_search_documents() {
        let dir = tempfile::tempdir().unwrap();
        let keys = MemoryKeyStore::default();
        let store = open_store(&dir, &keys).await;
        seed_day(&store).await;
        run_pass(&store, at(9, 36, 30), &[]).await.unwrap();
        // Close the active Slack session too.
        run_pass(&store, at(10, 0, 0), &[]).await.unwrap();

        let report = store
            .prune(at(9, 0, 0) + Duration::days(31), 30)
            .await
            .unwrap();
        assert_eq!(report.timeline_sessions, 3);
        assert_eq!(report.timeline_gaps, 1);
        assert!(report.search_documents >= 3);
        let (sessions, gaps) = day(&store).await;
        assert!(sessions.is_empty() && gaps.is_empty());
        assert!(
            db::search(&store, "quasar", at(0, 0, 0), at(23, 0, 0), 10, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_budgeted_pass_stops_early_and_never_expires_with_a_backlog() {
        let dir = tempfile::tempdir().unwrap();
        let keys = MemoryKeyStore::default();
        let store = open_store(&dir, &keys).await;
        // Four minutes of editor frames; "now" is long after them.
        insert_run(
            &store,
            at(9, 0, 0),
            120,
            "Code",
            "a.rs \u{2014} p",
            None,
            "fn a()",
        )
        .await;

        let first = run_pass_bounded(&store, at(10, 0, 0), &[], 60)
            .await
            .unwrap();
        assert_eq!(first.frames, 60);
        assert!(!first.caught_up);
        let (sessions, _) = day(&store).await;
        assert_eq!(sessions.len(), 1);
        assert!(
            sessions[0].3,
            "with frames left, the session must not be expired at its 2-minute mark"
        );

        let rest = run_pass_bounded(&store, at(10, 0, 0), &[], 1_000)
            .await
            .unwrap();
        assert_eq!(rest.frames, 60);
        assert!(rest.caught_up);
        let (sessions, _) = day(&store).await;
        assert_eq!(
            sessions.len(),
            1,
            "one continuous session, not split at the budget"
        );
        assert_eq!(sessions[0].1, 4 * 60_000);
        assert!(!sessions[0].3, "caught up: closed by expiry");
        assert_eq!(frame_total(&store).await, (120, 120));
    }

    #[tokio::test]
    async fn meeting_audio_makes_a_zoom_session_a_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let keys = MemoryKeyStore::default();
        let store = open_store(&dir, &keys).await;
        insert_run(
            &store,
            at(15, 0, 0),
            60,
            "zoom.us",
            "Zoom Meeting",
            None,
            "Participants",
        )
        .await;
        insert_run(
            &store,
            at(15, 2, 0),
            10,
            "Finder",
            "Downloads",
            None,
            "files",
        )
        .await;
        store
            .insert_input_events(&[NewInputEvent {
                occurred_at: at(15, 0, 1),
                kind: InputEventKind::Click,
                app_name: Some("zoom.us".into()),
                count: 1,
                clipboard_text: None,
            }])
            .await
            .unwrap();
        run_pass(&store, at(15, 3, 0), &[(at(14, 59, 0), at(15, 2, 30))])
            .await
            .unwrap();
        let (sessions, _) = day(&store).await;
        assert_eq!(sessions[0].0, "zoom.us");
        assert_eq!(sessions[0].4, Category::Meeting);
    }
}
