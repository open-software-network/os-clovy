//! Orchestration over injected dependencies (store, provider, embedder,
//! sources, zone, clock), so the whole path runs in tests with a fake
//! provider: completed hours → distillation → hour report → workstream fold,
//! then the day summary; and the read model of the "Today" view.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use futures_util::future::BoxFuture;
use serde::Serialize;

use super::db::{self, NewHourReport};
use super::distill::{self, SessionText};
use super::embedder::{Embedder, EmbedderStatus};
use super::hour::{self, HourInput, SessionLine};
use super::prompts;
use super::schedule::{self, day_key, HourWindow, LocalZone, RunOutcome};
use super::sources::{hhmm, whole_minutes, DaySources, DictationNote};
use super::summary::{
    self, DayInput, DayPanelsDto, DaySummaryDto, HourFocusDto, HourReportDto, SummaryTrigger,
};
use super::workstreams::{self, WorkstreamDto};
use crate::activity::settings::ActivitySettings;
use crate::activity::store::{ActivityStore, StoreError, TIMELINE_CONSUMER};
use crate::activity::timeline::{self, db::TimelineSessionDto, stats::day_stats};
use crate::interface_locale::UiLocale;
use crate::llm::{GenerateOutput, GenerateRequest, LlmError};

/// One activity call; production goes through `llm::generate_for_activity`.
pub trait Generator: Send + Sync {
    fn generate(&self, request: GenerateRequest)
        -> BoxFuture<'_, Result<GenerateOutput, LlmError>>;
}

pub struct ActivityProvider;

impl Generator for ActivityProvider {
    fn generate(
        &self,
        request: GenerateRequest,
    ) -> BoxFuture<'_, Result<GenerateOutput, LlmError>> {
        Box::pin(crate::llm::generate_for_activity(request))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error(transparent)]
    Llm(#[from] LlmErrorWrap),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the provider answer was not usable: {0}")]
    Unusable(&'static str),
}

/// `LlmError` has no `Error` impl; this carries it with a message.
#[derive(Debug)]
pub struct LlmErrorWrap(pub LlmError);

impl std::fmt::Display for LlmErrorWrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let error: crate::domain::types::AppError = self.0.clone().into();
        write!(f, "{}", error.message)
    }
}

impl std::error::Error for LlmErrorWrap {}

impl From<LlmError> for PipelineError {
    fn from(error: LlmError) -> Self {
        PipelineError::Llm(LlmErrorWrap(error))
    }
}

pub struct Deps<'a> {
    pub store: &'a ActivityStore,
    pub settings: &'a ActivitySettings,
    pub generator: &'a dyn Generator,
    pub embedder: &'a dyn Embedder,
    pub sources: &'a dyn DaySources,
    pub zone: &'a dyn LocalZone,
    pub locale: UiLocale,
    pub now: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HourOutcome {
    Reported,
    /// No session time in the hour.
    Empty,
}

async fn call(
    deps: &Deps<'_>,
    system: &str,
    prompt: String,
    schema: serde_json::Value,
) -> Result<GenerateOutput, PipelineError> {
    Ok(deps
        .generator
        .generate(GenerateRequest {
            system: Some(prompts::system_prompt(system, deps.locale)),
            prompt,
            schema: Some(schema),
            timeout: None,
        })
        .await?)
}

fn session_line(
    session: &TimelineSessionDto,
    window: &HourWindow,
    zone: &dyn LocalZone,
) -> SessionLine {
    let start = crate::activity::store::parse_time(&session.started_at)
        .unwrap_or(window.start)
        .max(window.start);
    let end = crate::activity::store::parse_time(&session.ended_at)
        .unwrap_or(window.end)
        .min(window.end);
    SessionLine {
        start: hhmm(zone, start),
        end: hhmm(zone, end),
        started_at: start,
        ended_at: end,
        app: session.app_name.clone(),
        context: session.context.clone(),
        category: session.category.as_db().to_string(),
        minutes: whole_minutes((end - start).num_milliseconds().max(0)),
        window: session.window_title.clone(),
    }
}

/// Enrich dictations from the timeline already read for the same period.
fn resolve_dictation_apps(dictations: &mut [DictationNote], sessions: &[TimelineSessionDto]) {
    for note in dictations {
        if let Some(session) = note.app_bundle_id.as_ref().and_then(|id| {
            sessions
                .iter()
                .find(|session| session.bundle_id.as_ref() == Some(id))
        }) {
            note.app_name = Some(session.app_name.clone());
        }
    }
}

/// Generates the report of one completed hour and stores it.
pub async fn report_hour(
    deps: &Deps<'_>,
    window: &HourWindow,
) -> Result<HourOutcome, PipelineError> {
    let view = timeline::timeline_view(
        deps.store,
        deps.settings,
        window.start,
        window.end,
        deps.now,
    )
    .await?;
    let active_minutes = whole_minutes(view.stats.focused_ms);
    if active_minutes == 0 {
        return Ok(HourOutcome::Empty);
    }
    let active_seconds = view.stats.focused_ms / 1000;
    let mut texts = Vec::new();
    let mut lines = Vec::new();
    let ids: Vec<i64> = view.sessions.iter().map(|session| session.id).collect();
    let mut by_session = db::session_documents(deps.store, &ids, window.start, window.end).await?;
    for session in &view.sessions {
        let line = session_line(session, window, deps.zone);
        let start = crate::activity::store::parse_time(&session.started_at).unwrap_or(window.start);
        let end = crate::activity::store::parse_time(&session.ended_at).unwrap_or(window.end);
        let seconds = (end.min(window.end) - start.max(window.start)).num_seconds();
        let documents = by_session
            .remove(&session.id)
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, url, _)| {
                !url.as_deref().is_some_and(|url| {
                    timeline::session_excluded("", None, None, Some(url), deps.settings)
                })
            })
            .map(|(seen_at, _, body)| {
                let at = crate::activity::store::parse_time(&seen_at).unwrap_or(window.start);
                (hhmm(deps.zone, at), body)
            })
            .collect();
        texts.push(SessionText {
            session_id: session.id,
            app: session.app_name.clone(),
            window: session.window_title.clone().unwrap_or_default(),
            time: line.start.clone(),
            seconds,
            documents,
        });
        lines.push(line);
    }
    let label = format!("{} {}:00", window.day, &window.hour[11..13]);
    let header = format!(
        "=== HOUR {label} · {} sessions · {active_minutes} min active ===",
        texts.len()
    );
    let distilled = distill::distill(&texts, &header, deps.embedder).await;
    let blocks = deps
        .sources
        .coding_blocks(
            deps.store,
            &deps.settings.coding_agents,
            window.start,
            window.end,
            deps.zone,
        )
        .await;
    let meetings = deps
        .sources
        .meetings(window.start, window.end, deps.zone)
        .await;
    let mut dictations = deps
        .sources
        .dictations(window.start, window.end, deps.zone)
        .await;
    resolve_dictation_apps(&mut dictations, &view.sessions);
    let prompt = hour::prompt(&HourInput {
        label: &label,
        active_minutes,
        sessions: &lines,
        coding_blocks: &blocks,
        meetings: &meetings,
        dictations: &dictations,
        distilled: &distilled.body,
    });
    let output = call(
        deps,
        prompts::HOUR_REPORT,
        prompt,
        prompts::hour_report_schema(),
    )
    .await?;
    let answer = output
        .json
        .as_ref()
        .and_then(hour::parse_answer)
        .ok_or(PipelineError::Unusable("hour report without activities"))?;
    let activities = hour::assemble(&answer, active_seconds, &lines);
    db::insert_hour_report(
        deps.store,
        &NewHourReport {
            hour: window.hour.clone(),
            day: window.day.clone(),
            started_at: window.start,
            ended_at: window.end,
            active_minutes,
            summary: answer.summary,
            activities,
            distilled: distilled.body,
            distill_stats: serde_json::to_value(&distilled.stats).unwrap_or_default(),
            locale: prompts::locale_tag(deps.locale).to_string(),
            provider: output.provider,
            generated_at: deps.now,
        },
    )
    .await?;
    Ok(HourOutcome::Reported)
}

/// Folds one stored hour report into the day's workstreams. When the
/// provider keeps failing (`give_up`), the hour lands as one new workstream
/// without a call, so no hour is ever left out.
pub async fn fold_hour(
    deps: &Deps<'_>,
    report: &HourReportDto,
    give_up: bool,
) -> Result<bool, PipelineError> {
    let day = &report.hour[..10];
    let prior = db::workstreams_of_day(deps.store, day).await?;
    let placements = if give_up {
        Vec::new()
    } else {
        let label = format!("{}:00", &report.hour[11..13]);
        let output = call(
            deps,
            prompts::WORKSTREAM_FOLD,
            workstreams::prompt(&prior, &label, &report.activities),
            prompts::workstream_schema(),
        )
        .await?;
        output
            .json
            .as_ref()
            .and_then(workstreams::parse_placements)
            .ok_or(PipelineError::Unusable(
                "workstream answer without placements",
            ))?
    };
    let plan = workstreams::plan_fold(&prior, &report.activities, &placements);
    Ok(db::apply_fold(deps.store, day, &report.hour, &plan, deps.now).await?)
}

/// How far the timeline has built sessions: now when caught up with
/// capture, else the last processed frame.
pub async fn built_until(
    store: &ActivityStore,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, StoreError> {
    let cursor = store.processing_cursor(TIMELINE_CONSUMER).await?;
    if store.latest_frame_at().await? == cursor.last_frame_at {
        return Ok(now);
    }
    Ok(cursor
        .last_frame_at
        .as_deref()
        .and_then(crate::activity::store::parse_time)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CatchUp {
    pub reported: usize,
    pub folded: usize,
    pub failed: usize,
}

/// Reports every completed hour of `day` that has none yet (respecting retry
/// backoff unless `force`), then folds every unfolded report in hour order.
pub async fn catch_up_day(
    deps: &Deps<'_>,
    day: NaiveDate,
    force: bool,
) -> Result<CatchUp, StoreError> {
    let mut result = CatchUp::default();
    let built = built_until(deps.store, deps.now).await?;
    for window in schedule::completed_hours(deps.zone, day, deps.now, built) {
        if db::hour_report_exists(deps.store, &window.hour).await? {
            continue;
        }
        let key = format!("hour:{}", window.hour);
        let record = db::run_record(deps.store, &key).await?;
        if !schedule::may_run(record.as_ref(), deps.now, force) {
            continue;
        }
        match report_hour(deps, &window).await {
            Ok(HourOutcome::Reported) => {
                db::record_run(deps.store, &key, RunOutcome::Ok, None, deps.now).await?;
                result.reported += 1;
            }
            Ok(HourOutcome::Empty) => {
                db::record_run(deps.store, &key, RunOutcome::Empty, None, deps.now).await?;
            }
            Err(PipelineError::Store(error)) => return Err(error),
            Err(error) => {
                tracing::warn!(hour = %window.hour, %error, "hour report failed");
                db::record_run(
                    deps.store,
                    &key,
                    RunOutcome::Failed,
                    Some(&error.to_string()),
                    deps.now,
                )
                .await?;
                result.failed += 1;
            }
        }
    }
    for report in db::unfolded_reports(deps.store, &day_key(day)).await? {
        let key = format!("fold:{}", report.hour);
        let record = db::run_record(deps.store, &key).await?;
        let used_up = record
            .as_ref()
            .is_some_and(|record| record.attempts >= schedule::MAX_ATTEMPTS);
        if !used_up && !schedule::may_run(record.as_ref(), deps.now, force) {
            // Later hours fold after this one; wait for its retry.
            break;
        }
        match fold_hour(deps, &report, used_up).await {
            Ok(_) => {
                db::record_run(deps.store, &key, RunOutcome::Ok, None, deps.now).await?;
                result.folded += 1;
            }
            Err(PipelineError::Store(error)) => return Err(error),
            Err(error) => {
                tracing::warn!(hour = %report.hour, %error, "workstream fold failed");
                db::record_run(
                    deps.store,
                    &key,
                    RunOutcome::Failed,
                    Some(&error.to_string()),
                    deps.now,
                )
                .await?;
                result.failed += 1;
                break;
            }
        }
    }
    Ok(result)
}

/// What a summary of the day would still leave out right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Incomplete {
    /// The timeline has not built every hour that ended yet.
    TimelineBehind,
    /// Hours that ended without a report and may still get one (never tried,
    /// or failed with retries left).
    Hours(Vec<String>),
    /// Hour reports not yet part of the day's workstreams.
    Folds(usize),
    /// Coding-agent ingestion has not read past the day's last ended hour.
    CodingAgents,
    /// Hour reports or folds that failed in an on-demand run.
    Failed(usize),
}

impl std::fmt::Display for Incomplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Incomplete::TimelineBehind => write!(f, "the timeline is still being built"),
            Incomplete::Hours(hours) => write!(f, "hours without a report: {}", hours.join(", ")),
            Incomplete::Folds(count) => write!(f, "{count} hour reports not in workstreams yet"),
            Incomplete::CodingAgents => write!(f, "coding-agent sessions are still being read"),
            Incomplete::Failed(count) => write!(f, "{count} hour reports or folds failed"),
        }
    }
}

/// Sends `day`'s hour reports and summary written in another language than
/// `deps.locale` back for generation: those reports, every workstream of
/// the day (their titles are in that language too, so the remaining reports
/// fold again) and the summary go, with their run records. True when
/// anything went.
pub async fn relocalize_day(deps: &Deps<'_>, day: NaiveDate) -> Result<bool, StoreError> {
    db::relocalize_day(deps.store, &day_key(day), prompts::locale_tag(deps.locale)).await
}

/// Writes again, in `deps.locale`, the summary of `day` that
/// [`relocalize_day`] removed, as soon as its inputs are settled, with the
/// trigger it had. Runs once: a summary written, an empty day or a failure
/// all clear the pending mark; only unsettled inputs keep it. `None` when
/// nothing was pending.
pub async fn finish_pending_resummary(
    deps: &Deps<'_>,
    day: NaiveDate,
) -> Result<Option<SummaryRun>, PipelineError> {
    let key = day_key(day);
    let Some(trigger) = db::pending_resummary(deps.store, &key).await? else {
        return Ok(None);
    };
    // Written meanwhile in this language ("Generate summary"): done.
    if db::summary_of_day(deps.store, &key)
        .await?
        .is_some_and(|summary| summary.locale == prompts::locale_tag(deps.locale))
    {
        db::clear_resummary(deps.store, &key).await?;
        return Ok(None);
    }
    if let Some(reason) = incomplete(deps, day).await? {
        return Ok(Some(SummaryRun::Incomplete(reason)));
    }
    let generated = generate_summary(deps, day, trigger).await;
    // Once, whatever happened: a failure is not retried in a loop.
    db::clear_resummary(deps.store, &key).await?;
    Ok(Some(match generated? {
        Some(summary) => SummaryRun::Generated(Box::new(summary)),
        None => SummaryRun::Empty,
    }))
}

/// [`finish_pending_resummary`] for the days the background loop catches
/// up (yesterday and `today`): a mark left yesterday must not outlive
/// midnight, when the scheduled run of that day is already recorded.
pub async fn finish_pending_resummaries(
    deps: &Deps<'_>,
    today: NaiveDate,
) -> Vec<(NaiveDate, Result<Option<SummaryRun>, PipelineError>)> {
    let mut results = Vec::with_capacity(2);
    for day in [today - Duration::days(1), today] {
        results.push((day, finish_pending_resummary(deps, day).await));
    }
    results
}

/// Whether every input of `day`'s summary is settled: the timeline built
/// through the last hour that ended, each ended hour reported (or empty, or
/// given up after its last attempt), every report folded, and coding-agent
/// ingestion past that hour.
pub async fn incomplete(deps: &Deps<'_>, day: NaiveDate) -> Result<Option<Incomplete>, StoreError> {
    let Some(last_end) = schedule::hours_of_day(deps.zone, day)
        .iter()
        .rev()
        .find(|window| window.end <= deps.now)
        .map(|window| window.end)
    else {
        return Ok(None);
    };
    if built_until(deps.store, deps.now).await? < last_end {
        return Ok(Some(Incomplete::TimelineBehind));
    }
    let mut pending = Vec::new();
    for window in schedule::completed_hours(deps.zone, day, deps.now, last_end) {
        if db::hour_report_exists(deps.store, &window.hour).await? {
            continue;
        }
        let record = db::run_record(deps.store, &format!("hour:{}", window.hour)).await?;
        let settled = record.is_some_and(|record| match record.outcome {
            RunOutcome::Ok | RunOutcome::Empty => true,
            RunOutcome::Failed => record.next_attempt_at.is_none(),
        });
        if !settled {
            pending.push(window.hour);
        }
    }
    if !pending.is_empty() {
        return Ok(Some(Incomplete::Hours(pending)));
    }
    let unfolded = db::unfolded_reports(deps.store, &day_key(day)).await?.len();
    if unfolded > 0 {
        return Ok(Some(Incomplete::Folds(unfolded)));
    }
    if !deps.sources.coding_agents_read_until(last_end) {
        return Ok(Some(Incomplete::CodingAgents));
    }
    Ok(None)
}

/// A summary run over a day whose inputs may not be settled yet.
#[derive(Clone, Debug, PartialEq)]
pub enum SummaryRun {
    /// Nothing was generated or saved: the summary would leave this out.
    Incomplete(Incomplete),
    /// Nothing to summarize (no reports, meetings, or blocks).
    Empty,
    Generated(Box<DaySummaryDto>),
}

/// Generates the summary of `day` only once its inputs are settled
/// ([`incomplete`]), so a stored summary is never written from partial rows.
/// `started` runs right before the provider call.
pub async fn summarize_settled_day(
    deps: &Deps<'_>,
    day: NaiveDate,
    trigger: SummaryTrigger,
    started: &(dyn Fn() + Send + Sync),
) -> Result<SummaryRun, PipelineError> {
    if let Some(reason) = incomplete(deps, day).await? {
        return Ok(SummaryRun::Incomplete(reason));
    }
    started();
    Ok(match generate_summary(deps, day, trigger).await? {
        Some(summary) => SummaryRun::Generated(Box::new(summary)),
        None => SummaryRun::Empty,
    })
}

/// "Generate summary": reports and folds the day's pending hours now (failed
/// ones retried), then writes the summary. An hour or fold that still fails,
/// or an input still behind, comes back as [`SummaryRun::Incomplete`] instead
/// of a summary written from partial rows.
pub async fn summarize_on_demand(
    deps: &Deps<'_>,
    day: NaiveDate,
) -> Result<SummaryRun, PipelineError> {
    let caught_up = catch_up_day(deps, day, true).await?;
    if caught_up.failed > 0 {
        return Ok(SummaryRun::Incomplete(Incomplete::Failed(caught_up.failed)));
    }
    summarize_settled_day(deps, day, SummaryTrigger::Manual, &|| {}).await
}

/// The outcome of the daily automatic summary of one day.
#[derive(Clone, Debug, PartialEq)]
pub enum ScheduledRun {
    /// Inputs not settled yet: nothing generated or recorded; the next tick
    /// tries again.
    Waiting(Incomplete),
    Generated(Box<DaySummaryDto>),
    Empty,
    /// `first`: the first failure of the day's run (worth a notification).
    Failed {
        message: String,
        first: bool,
    },
}

/// The automatic summary of `day`, recorded under `auto:<day>` (so it runs
/// once) only when it was generated, had nothing to say, or failed.
pub async fn run_scheduled_summary(
    deps: &Deps<'_>,
    day: NaiveDate,
    started: &(dyn Fn() + Send + Sync),
) -> Result<ScheduledRun, StoreError> {
    let key = format!("auto:{}", day_key(day));
    match summarize_settled_day(deps, day, SummaryTrigger::Scheduled, started).await {
        Ok(SummaryRun::Incomplete(reason)) => Ok(ScheduledRun::Waiting(reason)),
        Ok(SummaryRun::Generated(summary)) => {
            db::record_run(deps.store, &key, RunOutcome::Ok, None, deps.now).await?;
            Ok(ScheduledRun::Generated(summary))
        }
        Ok(SummaryRun::Empty) => {
            db::record_run(deps.store, &key, RunOutcome::Empty, None, deps.now).await?;
            Ok(ScheduledRun::Empty)
        }
        Err(error) => {
            let message = error.to_string();
            let record = db::record_run(
                deps.store,
                &key,
                RunOutcome::Failed,
                Some(&message),
                deps.now,
            )
            .await?;
            Ok(ScheduledRun::Failed {
                message,
                first: record.attempts == 1,
            })
        }
    }
}

#[derive(Default)]
struct DayMeasures {
    stats: timeline::stats::TimelineStatsDto,
    hours: Vec<HourFocusDto>,
    sessions: Vec<TimelineSessionDto>,
}

/// The day's measured statistics and per-hour focus from the timeline.
async fn day_measures(
    deps_store: &ActivityStore,
    settings: &ActivitySettings,
    zone: &dyn LocalZone,
    day: NaiveDate,
    now: DateTime<Utc>,
) -> Result<DayMeasures, StoreError> {
    let Some((from, to)) = schedule::day_bounds(zone, day) else {
        return Ok(Default::default());
    };
    let view = timeline::timeline_view(deps_store, settings, from, to, now).await?;
    let hours = schedule::hours_of_day(zone, day)
        .into_iter()
        .map(|window| HourFocusDto {
            focused_ms: day_stats(&view.sessions, &view.gaps, window.start, window.end).focused_ms,
            hour: window.hour,
        })
        .collect();
    Ok(DayMeasures {
        stats: view.stats,
        hours,
        sessions: view.sessions,
    })
}

/// Generates (or regenerates) the summary of `day` from what is stored.
pub async fn generate_summary(
    deps: &Deps<'_>,
    day: NaiveDate,
    trigger: SummaryTrigger,
) -> Result<Option<DaySummaryDto>, PipelineError> {
    let Some((from, to)) = schedule::day_bounds(deps.zone, day) else {
        return Ok(None);
    };
    let key = day_key(day);
    let DayMeasures {
        stats, sessions, ..
    } = day_measures(deps.store, deps.settings, deps.zone, day, deps.now).await?;
    let workstreams = db::workstreams_of_day(deps.store, &key).await?;
    let reports = db::hour_reports_of_day(deps.store, &key).await?;
    let meetings = deps.sources.meetings(from, to, deps.zone).await;
    let blocks = deps
        .sources
        .coding_blocks(
            deps.store,
            &deps.settings.coding_agents,
            from,
            to,
            deps.zone,
        )
        .await;
    if reports.is_empty() && meetings.is_empty() && blocks.is_empty() {
        return Ok(None);
    }
    let mut dictations = deps.sources.dictations(from, to, deps.zone).await;
    resolve_dictation_apps(&mut dictations, &sessions);
    let prompt = summary::prompt(&DayInput {
        day: &key,
        stats: &stats,
        workstreams: &workstreams,
        hour_reports: &reports,
        meetings: &meetings,
        coding_blocks: &blocks,
        dictations: &dictations,
    });
    let output = call(
        deps,
        prompts::DAY_SUMMARY,
        prompt,
        prompts::day_summary_schema(),
    )
    .await?;
    let answer = output
        .json
        .as_ref()
        .and_then(summary::parse_answer)
        .ok_or(PipelineError::Unusable("day summary without text"))?;
    let dto = DaySummaryDto {
        day: key,
        headline: answer.headline,
        narrative: answer.narrative,
        insights: answer.insights,
        standup: answer.standup,
        hours_covered: reports.len() as i64,
        locale: prompts::locale_tag(deps.locale).to_string(),
        provider: output.provider,
        trigger,
        generated_at: crate::activity::store::timestamp(deps.now),
    };
    db::save_summary(deps.store, &dto).await?;
    Ok(Some(dto))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderState {
    Ready,
    /// No activity provider chosen ("none"): nothing is generated.
    Missing,
    /// The chosen provider has no JSON schema support (or was never tested).
    Insufficient,
}

/// Everything the "Today" view shows for a day's intelligence.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayIntelligenceDto {
    pub availability: timeline::Availability,
    pub day: String,
    pub provider: ProviderState,
    pub embedder: EmbedderStatus,
    pub running: bool,
    pub summary_time: String,
    pub summary: Option<DaySummaryDto>,
    pub workstreams: Vec<WorkstreamDto>,
    pub hour_reports: Vec<HourReportDto>,
    pub panels: DayPanelsDto,
}

pub struct ReadContext<'a> {
    pub store: &'a ActivityStore,
    pub settings: &'a ActivitySettings,
    pub sources: &'a dyn DaySources,
    pub zone: &'a dyn LocalZone,
    pub now: DateTime<Utc>,
}

/// The read model: stored texts plus panels computed from the data now.
pub async fn day_view(
    context: &ReadContext<'_>,
    day: NaiveDate,
) -> Result<
    (
        Option<DaySummaryDto>,
        Vec<WorkstreamDto>,
        Vec<HourReportDto>,
        DayPanelsDto,
    ),
    StoreError,
> {
    let key = day_key(day);
    let DayMeasures { stats, hours, .. } = day_measures(
        context.store,
        context.settings,
        context.zone,
        day,
        context.now,
    )
    .await?;
    let workstreams = db::workstreams_of_day(context.store, &key).await?;
    let reports = db::hour_reports_of_day(context.store, &key).await?;
    let (meetings, blocks) = match schedule::day_bounds(context.zone, day) {
        Some((from, to)) => (
            context.sources.meetings(from, to, context.zone).await,
            context
                .sources
                .coding_blocks(
                    context.store,
                    &context.settings.coding_agents,
                    from,
                    to,
                    context.zone,
                )
                .await,
        ),
        None => (Vec::new(), Vec::new()),
    };
    let panels = summary::panels(&stats, hours, &workstreams, &meetings, &blocks);
    let stored = db::summary_of_day(context.store, &key).await?;
    Ok((stored, workstreams, reports, panels))
}

/// Retention for the day intelligence tables (same period as capture).
pub async fn prune(
    store: &ActivityStore,
    settings: &ActivitySettings,
    zone: &dyn LocalZone,
    now: DateTime<Utc>,
) -> Result<u64, StoreError> {
    let cutoff = now - Duration::days(i64::from(settings.retention_days));
    db::prune(store, &day_key(schedule::local_day(zone, cutoff))).await
}
