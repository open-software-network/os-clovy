//! Day intelligence tables in the encrypted `activity.sqlite3` (migration 4,
//! `day_intelligence`): hour reports, workstreams and their hours, day
//! summaries, and the scheduler's run records. This is also the read API
//! other slices use (the MCP `get_day_summary` tool): `summary_of_day`,
//! `workstreams_of_day`, `hour_reports_of_day`.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sqlx::query::query;
use sqlx::query_scalar::query_scalar;
use sqlx::row::Row;
use sqlx_sqlite::SqliteRow;

use super::hour::{ActivityPeriod, HourActivity};
use super::schedule::{next_attempt, RunOutcome, RunRecord};
use super::summary::{DaySummaryDto, HourReportDto, Insight, Standup, SummaryTrigger};
use super::workstreams::{FoldPlan, WorkstreamDto, WorkstreamHourDto};
use crate::activity::store::{timestamp, ActivityStore, StoreError};

/// A finished hour report to store.
#[derive(Clone, Debug, PartialEq)]
pub struct NewHourReport {
    pub hour: String,
    pub day: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub active_minutes: i64,
    pub summary: String,
    pub activities: Vec<HourActivity>,
    pub distilled: String,
    pub distill_stats: Value,
    pub locale: String,
    pub provider: String,
    pub generated_at: DateTime<Utc>,
}

const REPORT_COLUMNS: &str =
    "hour, started_at, ended_at, active_minutes, summary, activities_json, provider, generated_at";

fn report_dto(row: &SqliteRow) -> HourReportDto {
    HourReportDto {
        hour: row.get(0),
        started_at: row.get(1),
        ended_at: row.get(2),
        active_minutes: row.get(3),
        summary: row.get(4),
        activities: serde_json::from_str(row.get::<&str, _>(5)).unwrap_or_default(),
        provider: row.get(6),
        generated_at: row.get(7),
    }
}

pub async fn insert_hour_report(
    store: &ActivityStore,
    report: &NewHourReport,
) -> Result<(), StoreError> {
    query(
        "INSERT INTO day_hour_reports (hour, day, started_at, ended_at, active_minutes, summary,
            activities_json, distilled, distill_json, locale, provider, generated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(hour) DO NOTHING",
    )
    .bind(&report.hour)
    .bind(&report.day)
    .bind(timestamp(report.started_at))
    .bind(timestamp(report.ended_at))
    .bind(report.active_minutes)
    .bind(&report.summary)
    .bind(serde_json::to_string(&report.activities).unwrap_or_else(|_| "[]".into()))
    .bind(&report.distilled)
    .bind(report.distill_stats.to_string())
    .bind(&report.locale)
    .bind(&report.provider)
    .bind(timestamp(report.generated_at))
    .execute(store.pool())
    .await?;
    Ok(())
}

pub async fn hour_report_exists(store: &ActivityStore, hour: &str) -> Result<bool, StoreError> {
    Ok(query("SELECT 1 FROM day_hour_reports WHERE hour = ?")
        .bind(hour)
        .fetch_optional(store.pool())
        .await?
        .is_some())
}

/// The day's hour reports, in hour order.
pub async fn hour_reports_of_day(
    store: &ActivityStore,
    day: &str,
) -> Result<Vec<HourReportDto>, StoreError> {
    Ok(query(&format!(
        "SELECT {REPORT_COLUMNS} FROM day_hour_reports WHERE day = ? ORDER BY hour"
    ))
    .bind(day)
    .fetch_all(store.pool())
    .await?
    .iter()
    .map(report_dto)
    .collect())
}

/// Reports of `day` not folded into workstreams yet, in hour order.
pub async fn unfolded_reports(
    store: &ActivityStore,
    day: &str,
) -> Result<Vec<HourReportDto>, StoreError> {
    Ok(query(&format!(
        "SELECT {REPORT_COLUMNS} FROM day_hour_reports
         WHERE day = ? AND folded_at IS NULL ORDER BY hour"
    ))
    .bind(day)
    .fetch_all(store.pool())
    .await?
    .iter()
    .map(report_dto)
    .collect())
}

/// `day_workstream_hours.periods_json`: where an hour's work was located,
/// its seconds and located_seconds, and how many minutes that covers. NULL: only
/// the hour is known.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredLocation {
    #[serde(default)]
    pub seconds: Option<i64>,
    #[serde(default)]
    pub located_seconds: Option<i64>,
    #[serde(default)]
    pub located_minutes: Option<i64>,
    #[serde(default)]
    pub periods: Option<Vec<ActivityPeriod>>,
}

fn location_json(
    seconds: i64,
    located_seconds: i64,
    located_minutes: i64,
    periods: &Option<Vec<ActivityPeriod>>,
) -> Option<String> {
    serde_json::to_string(&StoredLocation {
        seconds: Some(seconds),
        located_seconds: Some(located_seconds),
        located_minutes: Some(located_minutes),
        periods: periods.clone(),
    })
    .ok()
}

/// The day's workstreams (oldest first) with their hours; seconds and minutes are the
/// sum of the hours' measured active times. An hour stored without periods (or
/// with an unreadable location) reads as `periods: None`, known by hour only.
pub async fn workstreams_of_day(
    store: &ActivityStore,
    day: &str,
) -> Result<Vec<WorkstreamDto>, StoreError> {
    let rows = query(
        "SELECT w.id, w.title, w.summary, h.hour, h.minutes, h.note, h.periods_json
         FROM day_workstreams w
         LEFT JOIN day_workstream_hours h ON h.workstream_id = w.id
         WHERE w.day = ? ORDER BY w.id, h.hour",
    )
    .bind(day)
    .fetch_all(store.pool())
    .await?;
    let mut workstreams: Vec<WorkstreamDto> = Vec::new();
    for row in &rows {
        let id: i64 = row.get(0);
        if workstreams.last().map(|workstream| workstream.id) != Some(id) {
            workstreams.push(WorkstreamDto {
                id,
                title: row.get(1),
                summary: row.get(2),
                minutes: 0,
                seconds: 0,
                hours: Vec::new(),
            });
        }
        let (Some(hour), Some(workstream)) =
            (row.get::<Option<String>, _>(3), workstreams.last_mut())
        else {
            continue;
        };
        let location: Option<StoredLocation> = row
            .get::<Option<&str>, _>(6)
            .and_then(|json| serde_json::from_str(json).ok());
        let minutes: i64 = row.get(4);
        let seconds: i64 = location
            .as_ref()
            .and_then(|found| found.seconds)
            .unwrap_or(minutes * 60);
        let located_minutes: i64 = location
            .as_ref()
            .and_then(|found| found.located_minutes)
            .unwrap_or(0);
        let located_seconds: i64 = location
            .as_ref()
            .and_then(|found| found.located_seconds)
            .unwrap_or(located_minutes * 60);
        workstream.minutes += minutes;
        workstream.seconds += seconds;
        workstream.hours.push(WorkstreamHourDto {
            hour,
            minutes,
            seconds,
            note: row.get(5),
            located_minutes,
            located_seconds,
            periods: location.and_then(|found| found.periods),
        });
    }
    Ok(workstreams)
}

/// Applies an hour's fold in one transaction and marks the report folded.
/// Returns false (and writes nothing) when the hour was folded already.
pub async fn apply_fold(
    store: &ActivityStore,
    day: &str,
    hour: &str,
    plan: &FoldPlan,
    now: DateTime<Utc>,
) -> Result<bool, StoreError> {
    let at = timestamp(now);
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await?;
    let marked =
        query("UPDATE day_hour_reports SET folded_at = ? WHERE hour = ? AND folded_at IS NULL")
            .bind(&at)
            .bind(hour)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    if marked == 0 {
        tx.rollback().await?;
        return Ok(false);
    }
    for append in &plan.appends {
        let inserted = query(
            "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note, periods_json)
             SELECT id, ?, ?, ?, ? FROM day_workstreams WHERE id = ? AND day = ?
             ON CONFLICT(workstream_id, hour) DO NOTHING",
        )
        .bind(hour)
        .bind(append.minutes)
        .bind(&append.note)
        .bind(location_json(
            append.seconds,
            append.located_seconds,
            append.located_minutes,
            &append.periods,
        ))
        .bind(append.workstream_id)
        .bind(day)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if inserted == 0 {
            continue;
        }
        query(
            "UPDATE day_workstreams
             SET summary = COALESCE(?, summary), last_hour = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(append.summary.as_deref())
        .bind(hour)
        .bind(&at)
        .bind(append.workstream_id)
        .execute(&mut *tx)
        .await?;
    }
    for create in &plan.creates {
        let id = query(
            "INSERT INTO day_workstreams (day, title, summary, first_hour, last_hour, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(day)
        .bind(&create.title)
        .bind(&create.summary)
        .bind(hour)
        .bind(hour)
        .bind(&at)
        .bind(&at)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
        query(
            "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note, periods_json)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(hour)
        .bind(create.minutes)
        .bind(&create.note)
        .bind(location_json(
            create.seconds,
            create.located_seconds,
            create.located_minutes,
            &create.periods,
        ))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

pub async fn save_summary(
    store: &ActivityStore,
    summary: &DaySummaryDto,
) -> Result<(), StoreError> {
    let standup = json!({
        "done": summary.standup.done,
        "in_progress": summary.standup.in_progress,
        "blockers": summary.standup.blockers,
    });
    query(
        "INSERT INTO day_summaries (day, headline, narrative, insights_json, standup_json,
            hours_covered, locale, provider, trigger, generated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(day) DO UPDATE SET
            headline = excluded.headline, narrative = excluded.narrative,
            insights_json = excluded.insights_json, standup_json = excluded.standup_json,
            hours_covered = excluded.hours_covered, locale = excluded.locale,
            provider = excluded.provider, trigger = excluded.trigger,
            generated_at = excluded.generated_at",
    )
    .bind(&summary.day)
    .bind(&summary.headline)
    .bind(&summary.narrative)
    .bind(serde_json::to_string(&summary.insights).unwrap_or_else(|_| "[]".into()))
    .bind(standup.to_string())
    .bind(summary.hours_covered)
    .bind(&summary.locale)
    .bind(&summary.provider)
    .bind(summary.trigger.as_str())
    .bind(&summary.generated_at)
    .execute(store.pool())
    .await?;
    Ok(())
}

fn string_list(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The stored summary of a local day ("YYYY-MM-DD").
pub async fn summary_of_day(
    store: &ActivityStore,
    day: &str,
) -> Result<Option<DaySummaryDto>, StoreError> {
    let Some(row) = query(
        "SELECT day, headline, narrative, insights_json, standup_json, hours_covered, locale,
                provider, trigger, generated_at
         FROM day_summaries WHERE day = ?",
    )
    .bind(day)
    .fetch_optional(store.pool())
    .await?
    else {
        return Ok(None);
    };
    let standup: Value = serde_json::from_str(row.get::<&str, _>(4)).unwrap_or(Value::Null);
    let insights: Vec<Insight> = serde_json::from_str(row.get::<&str, _>(3)).unwrap_or_default();
    Ok(Some(DaySummaryDto {
        day: row.get(0),
        headline: row.get(1),
        narrative: row.get(2),
        insights,
        standup: Standup {
            done: string_list(&standup, "done"),
            in_progress: string_list(&standup, "in_progress"),
            blockers: string_list(&standup, "blockers"),
        },
        hours_covered: row.get(5),
        locale: row.get(6),
        provider: row.get(7),
        trigger: SummaryTrigger::parse(row.get::<&str, _>(8)),
        generated_at: row.get(9),
    }))
}

pub async fn run_record(store: &ActivityStore, key: &str) -> Result<Option<RunRecord>, StoreError> {
    Ok(
        query("SELECT attempts, outcome, next_attempt_at FROM day_intelligence_runs WHERE key = ?")
            .bind(key)
            .fetch_optional(store.pool())
            .await?
            .map(|row| RunRecord {
                attempts: row.get(0),
                outcome: RunOutcome::parse(row.get::<&str, _>(1)),
                next_attempt_at: row
                    .get::<Option<&str>, _>(2)
                    .and_then(|at| DateTime::parse_from_rfc3339(at).ok())
                    .map(|at| at.with_timezone(&Utc)),
            }),
    )
}

/// Records an outcome; a failure counts an attempt and schedules the retry.
pub async fn record_run(
    store: &ActivityStore,
    key: &str,
    outcome: RunOutcome,
    error: Option<&str>,
    now: DateTime<Utc>,
) -> Result<RunRecord, StoreError> {
    let previous = run_record(store, key)
        .await?
        .map_or(0, |record| record.attempts);
    let attempts = previous + 1;
    let next = match outcome {
        RunOutcome::Failed => next_attempt(now, attempts),
        RunOutcome::Ok | RunOutcome::Empty => None,
    };
    query(
        "INSERT INTO day_intelligence_runs (key, attempts, outcome, last_error, next_attempt_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET attempts = excluded.attempts, outcome = excluded.outcome,
            last_error = excluded.last_error, next_attempt_at = excluded.next_attempt_at,
            updated_at = excluded.updated_at",
    )
    .bind(key)
    .bind(attempts)
    .bind(outcome.as_str())
    .bind(error)
    .bind(next.map(timestamp))
    .bind(timestamp(now))
    .execute(store.pool())
    .await?;
    Ok(RunRecord {
        attempts,
        outcome,
        next_attempt_at: next,
    })
}

/// Sends `day`'s text written in another language than `locale` back for
/// generation (see `pipeline::relocalize_day`). True when anything went.
pub async fn relocalize_day(
    store: &ActivityStore,
    day: &str,
    locale: &str,
) -> Result<bool, StoreError> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await?;
    let stale_reports: i64 =
        query_scalar("SELECT COUNT(*) FROM day_hour_reports WHERE day = ? AND locale <> ?")
            .bind(day)
            .bind(locale)
            .fetch_one(&mut *tx)
            .await?;
    let stale_summary: i64 =
        query_scalar("SELECT COUNT(*) FROM day_summaries WHERE day = ? AND locale <> ?")
            .bind(day)
            .bind(locale)
            .fetch_one(&mut *tx)
            .await?;
    if stale_reports == 0 && stale_summary == 0 {
        tx.commit().await?;
        return Ok(false);
    }
    let hour_keys = format!("{day}T%");
    if stale_reports > 0 {
        query("DELETE FROM day_hour_reports WHERE day = ? AND locale <> ?")
            .bind(day)
            .bind(locale)
            .execute(&mut *tx)
            .await?;
        // Workstream titles are in the old language too: the day folds again
        // from the reports that stay.
        query(
            "DELETE FROM day_workstream_hours WHERE workstream_id IN
                (SELECT id FROM day_workstreams WHERE day = ?)",
        )
        .bind(day)
        .execute(&mut *tx)
        .await?;
        query("DELETE FROM day_workstreams WHERE day = ?")
            .bind(day)
            .execute(&mut *tx)
            .await?;
        query("UPDATE day_hour_reports SET folded_at = NULL WHERE day = ?")
            .bind(day)
            .execute(&mut *tx)
            .await?;
        query(
            "DELETE FROM day_intelligence_runs
             WHERE key LIKE 'hour:' || ? OR key LIKE 'fold:' || ?",
        )
        .bind(&hour_keys)
        .bind(&hour_keys)
        .execute(&mut *tx)
        .await?;
    }
    // A summary in another language, or one written from the reports that
    // just went, is written again as soon as the reports are, with the
    // trigger it had (`pipeline::finish_pending_resummary`). The scheduled
    // run keeps its record, so it does not write a second one.
    let trigger: Option<String> = query_scalar("SELECT trigger FROM day_summaries WHERE day = ?")
        .bind(day)
        .fetch_optional(&mut *tx)
        .await?;
    if let Some(trigger) = trigger {
        query("DELETE FROM day_summaries WHERE day = ?")
            .bind(day)
            .execute(&mut *tx)
            .await?;
        query("DELETE FROM day_intelligence_runs WHERE key LIKE 'resummary:' || ? || ':%'")
            .bind(day)
            .execute(&mut *tx)
            .await?;
        query(
            "INSERT INTO day_intelligence_runs (key, attempts, outcome, last_error, next_attempt_at, updated_at)
             VALUES (?, 0, 'ok', NULL, NULL, ?)",
        )
        .bind(format!("resummary:{day}:{}", SummaryTrigger::parse(&trigger).as_str()))
        .bind(timestamp(Utc::now()))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// The summary of `day` waiting to be written again after a language
/// change, by the trigger it had.
pub async fn pending_resummary(
    store: &ActivityStore,
    day: &str,
) -> Result<Option<SummaryTrigger>, StoreError> {
    let key: Option<String> = query_scalar(
        "SELECT key FROM day_intelligence_runs WHERE key LIKE 'resummary:' || ? || ':%'",
    )
    .bind(day)
    .fetch_optional(store.pool())
    .await?;
    Ok(key.and_then(|key| key.rsplit(':').next().map(SummaryTrigger::parse)))
}

pub async fn clear_resummary(store: &ActivityStore, day: &str) -> Result<(), StoreError> {
    query("DELETE FROM day_intelligence_runs WHERE key LIKE 'resummary:' || ? || ':%'")
        .bind(day)
        .execute(store.pool())
        .await?;
    Ok(())
}

/// Retention: removes everything of days before `cutoff_day`.
pub async fn prune(store: &ActivityStore, cutoff_day: &str) -> Result<u64, StoreError> {
    let mut tx = store.pool().begin_with("BEGIN IMMEDIATE").await?;
    let mut removed = 0;
    for statement in [
        "DELETE FROM day_hour_reports WHERE day < ?",
        "DELETE FROM day_workstreams WHERE day < ?",
        "DELETE FROM day_summaries WHERE day < ?",
    ] {
        removed += query(statement)
            .bind(cutoff_day)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    }
    // Run keys end with the hour or day they are about.
    query(
        "DELETE FROM day_intelligence_runs
         WHERE substr(key, instr(key, ':') + 1, 10) < ?",
    )
    .bind(cutoff_day)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(removed)
}

/// One window segment of a session: when it was seen, its URL, its text.
pub type SessionDocument = (String, Option<String>, String);

/// Window segments (local text) seen in `[from, to)` of each of the
/// timeline sessions, in one query.
pub async fn session_documents(
    store: &ActivityStore,
    session_ids: &[i64],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<HashMap<i64, Vec<SessionDocument>>, StoreError> {
    let mut documents: HashMap<i64, Vec<SessionDocument>> = HashMap::new();
    if session_ids.is_empty() {
        return Ok(documents);
    }
    let rows = query(
        "SELECT s.id, ts.seen_at, ts.browser_url, ts.body
         FROM timeline_sessions s JOIN timeline_search ts
           ON ts.rowid BETWEEN s.search_rowid_first AND s.search_rowid_last
          AND ts.session_id = s.id
         WHERE s.id IN (SELECT value FROM json_each(?))
           AND ts.seen_at >= ? AND ts.seen_at < ? AND ts.body <> ''
         ORDER BY ts.rowid",
    )
    .bind(json!(session_ids).to_string())
    .bind(timestamp(from))
    .bind(timestamp(to))
    .fetch_all(store.pool())
    .await?;
    for row in &rows {
        let url: String = row.get(2);
        documents.entry(row.get(0)).or_default().push((
            row.get(1),
            (!url.is_empty()).then_some(url),
            row.get(3),
        ));
    }
    Ok(documents)
}

/// Debug export sections (development builds): the newest days.
pub async fn debug_sections(store: &ActivityStore) -> Result<Map<String, Value>, StoreError> {
    let days: Vec<String> =
        query("SELECT DISTINCT day FROM day_hour_reports ORDER BY day DESC LIMIT 3")
            .fetch_all(store.pool())
            .await?
            .iter()
            .map(|row| row.get(0))
            .collect();
    let mut reports = Vec::new();
    let mut workstreams = Vec::new();
    let mut summaries = Vec::new();
    for day in &days {
        for report in hour_reports_of_day(store, day).await? {
            reports.push(serde_json::to_value(report).unwrap_or(Value::Null));
        }
        workstreams.push(json!({
            "day": day,
            "workstreams": workstreams_of_day(store, day).await?,
        }));
    }
    for row in query("SELECT day FROM day_summaries ORDER BY day DESC LIMIT 3")
        .fetch_all(store.pool())
        .await?
    {
        if let Some(summary) = summary_of_day(store, row.get::<&str, _>(0)).await? {
            summaries.push(serde_json::to_value(summary).unwrap_or(Value::Null));
        }
    }
    let runs: Vec<Value> = query(
        "SELECT key, attempts, outcome, last_error, next_attempt_at, updated_at
         FROM day_intelligence_runs ORDER BY updated_at DESC LIMIT 100",
    )
    .fetch_all(store.pool())
    .await?
    .iter()
    .map(|row| {
        json!({
            "key": row.get::<String, _>(0),
            "attempts": row.get::<i64, _>(1),
            "outcome": row.get::<String, _>(2),
            "lastError": row.get::<Option<String>, _>(3),
            "nextAttemptAt": row.get::<Option<String>, _>(4),
            "updatedAt": row.get::<String, _>(5),
        })
    })
    .collect();
    let mut sections = Map::new();
    sections.insert("dayHourReports".into(), Value::Array(reports));
    sections.insert("dayWorkstreams".into(), Value::Array(workstreams));
    sections.insert("daySummaries".into(), Value::Array(summaries));
    sections.insert("dayIntelligenceRuns".into(), Value::Array(runs));
    Ok(sections)
}
