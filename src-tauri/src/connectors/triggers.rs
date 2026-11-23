//! Connector trigger daemon.
//!
//! A background tokio task that polls Google for the events routines subscribe
//! to and wakes the matching routine through the Clovy-owned scheduler.
//! The event is a wake-up only: the routine re-reads state through its tools.
//!
//! - `email_received`: Gmail `history.list` from a stored cursor (seeded from
//!   the profile's history id on the first run). New INBOX messages fire the
//!   routine; the cursor advances.
//! - `event_upcoming`: a Calendar window scan; an event with external attendees
//!   starting within the configured lead time fires the routine, tracked per
//!   event id in a per-job cursor so it never fires twice for the same event.
//!
//! Cadence adapts: ~90s for mail, ~5min for calendar, backing off to ~5min for
//! everything when the machine is on battery below 20% and discharging, or when
//! no triggers are configured. A single account's error never takes the daemon
//! down, and nothing here logs a token or any mail or event content.

use crate::connectors::google::{self, GoogleApiError, ListEventsParams};
use crate::db::repositories::{ConnectorTriggerRecord, Repositories};
use crate::domain::types::AppError;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

const STARTUP_DELAY: Duration = Duration::from_secs(30);
const GMAIL_PERIOD: Duration = Duration::from_secs(90);
const CALENDAR_PERIOD: Duration = Duration::from_secs(300);
/// Cadence when the machine is on battery below the threshold and discharging,
/// or when there is nothing to poll.
const BACKOFF_PERIOD: Duration = Duration::from_secs(300);
const MIN_SLEEP: Duration = Duration::from_secs(5);
const BATTERY_LOW_PERCENT: u32 = 20;
const DEFAULT_LEAD_MINUTES: i64 = 15;
const MAX_CALENDAR_ACCOUNT_CONCURRENCY: usize = 4;
const MAX_EVENT_PAGES: usize = 20;
const EMAIL_KIND: &str = "email_received";
const EVENT_KIND: &str = "event_upcoming";

/// Spawn the trigger daemon. Called once from app setup after the bridge init.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(STARTUP_DELAY).await;
        run(app).await;
    });
}

async fn run(app: AppHandle) {
    let mut next_gmail = Instant::now();
    let mut next_calendar = Instant::now();
    // Last-known "is anything subscribed" per kind, refreshed each time that
    // kind is polled. A kind not due this iteration keeps its prior value, so
    // the idle backoff below reflects triggers across BOTH kinds, not just the
    // one polled this loop. Without this, an install with only Gmail triggers
    // would be forced onto the 5-minute idle cadence on every iteration where
    // the empty calendar poll came due before the next mail poll.
    let mut gmail_has_triggers = false;
    let mut calendar_has_triggers = false;
    loop {
        let backoff = battery_low_and_discharging();
        let now = Instant::now();

        if now >= next_gmail {
            gmail_has_triggers = poll_kind(&app, EMAIL_KIND).await;
            let period = if backoff {
                BACKOFF_PERIOD
            } else {
                GMAIL_PERIOD
            };
            next_gmail = Instant::now() + period;
        }
        if now >= next_calendar {
            calendar_has_triggers = poll_kind(&app, EVENT_KIND).await;
            next_calendar = Instant::now() + CALENDAR_PERIOD;
        }

        // Idle backoff: with nothing subscribed of either kind, wait a full
        // backoff period before looking again instead of spinning on the short
        // mail cadence. As long as some trigger exists, honor the normal
        // per-kind cadence so due mail polls are not delayed to the idle rate.
        let soonest = next_gmail.min(next_calendar);
        let mut sleep = soonest.saturating_duration_since(Instant::now());
        if !gmail_has_triggers && !calendar_has_triggers {
            sleep = sleep.max(BACKOFF_PERIOD);
        }
        tokio::time::sleep(sleep.max(MIN_SLEEP)).await;
    }
}

/// Poll every trigger of one kind, grouped so each account is queried once.
/// Returns whether any triggers of this kind exist (for idle backoff).
async fn poll_kind(app: &AppHandle, kind: &str) -> bool {
    let Ok(repos) = crate::commands::repositories(app).await else {
        return false;
    };
    let triggers = match repos.list_connector_triggers(None).await {
        Ok(triggers) => triggers,
        Err(error) => {
            tracing::warn!(error_code = %AppError::from(error).code, kind, "connector triggers query failed");
            return false;
        }
    };
    let relevant: Vec<ConnectorTriggerRecord> =
        triggers.into_iter().filter(|t| t.kind == kind).collect();
    if relevant.is_empty() {
        return false;
    }

    if kind == EMAIL_KIND {
        // One history.list per account, firing every job subscribed to it.
        let mut by_account: HashMap<String, Vec<ConnectorTriggerRecord>> = HashMap::new();
        for trigger in relevant {
            by_account
                .entry(trigger.account_id.clone())
                .or_default()
                .push(trigger);
        }
        for (account_id, triggers) in by_account {
            if let Err(error) = poll_email_account(app, &repos, &account_id, &triggers).await {
                tracing::warn!(error_code = %error.code, kind, "connector email trigger poll failed");
            }
        }
    } else {
        poll_calendar_accounts(app, &repos, relevant).await;
    }
    true
}

// --- Email --------------------------------------------------------------------

async fn poll_email_account(
    app: &AppHandle,
    repos: &Repositories,
    account_id: &str,
    triggers: &[ConnectorTriggerRecord],
) -> Result<(), AppError> {
    let token = crate::connectors::google_access_token(app, account_id).await?;
    let cursor = repos.trigger_cursor(account_id, EMAIL_KIND).await?;

    let Some(cursor) = cursor else {
        // First run establishes the baseline; nothing fires until the next
        // poll sees mail that arrived after this point.
        let profile = call_gmail_profile(app, account_id, &token).await?;
        if let Some(history_id) = profile.history_id {
            repos
                .set_trigger_cursor(account_id, EMAIL_KIND, &history_id)
                .await?;
            for trigger in triggers {
                repos
                    .set_email_trigger_cursor_if_active(
                        &trigger.id,
                        &trigger.job_id,
                        account_id,
                        &history_id,
                    )
                    .await?;
            }
        }
        return Ok(());
    };

    // Each routine owns an acknowledgement cursor. Jobs at the same cursor
    // share one Gmail request, while a failed job can retry independently
    // without replaying the delta into subscribers that already succeeded.
    let mut jobs_by_cursor: HashMap<String, Vec<&ConnectorTriggerRecord>> = HashMap::new();
    for trigger in triggers {
        let kind = email_cursor_kind(&trigger.job_id);
        let job_cursor = match repos.trigger_cursor(account_id, &kind).await? {
            Some(cursor) => cursor,
            None => cursor.clone(),
        };
        jobs_by_cursor.entry(job_cursor).or_default().push(trigger);
    }

    let mut latest_account_cursor = cursor;
    for (job_cursor, cursor_jobs) in jobs_by_cursor {
        let delta = match call_gmail_history(app, account_id, &token, &job_cursor).await {
            Ok(delta) => delta,
            Err(error) if error.code == "connector_history_cursor_expired" => {
                // Reseed only the affected subscribers. Their cursor may lag
                // because an earlier dispatch failed while peers advanced.
                let profile = call_gmail_profile(app, account_id, &token).await?;
                if let Some(history_id) = profile.history_id {
                    latest_account_cursor =
                        newer_history_id(latest_account_cursor, history_id.clone());
                    for trigger in cursor_jobs {
                        repos
                            .set_email_trigger_cursor_if_active(
                                &trigger.id,
                                &trigger.job_id,
                                account_id,
                                &history_id,
                            )
                            .await?;
                    }
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let Some(history_id) = delta.history_id else {
            continue;
        };
        latest_account_cursor = newer_history_id(latest_account_cursor, history_id.clone());
        for trigger in cursor_jobs {
            let acknowledged =
                delta.added.is_empty() || email_outcome_acknowledged(fire(app, trigger).await);
            if acknowledged {
                repos
                    .set_email_trigger_cursor_if_active(
                        &trigger.id,
                        &trigger.job_id,
                        account_id,
                        &history_id,
                    )
                    .await?;
            }
        }
    }
    repos
        .set_trigger_cursor(account_id, EMAIL_KIND, &latest_account_cursor)
        .await?;
    Ok(())
}

fn email_cursor_kind(job_id: &str) -> String {
    format!("{EMAIL_KIND}:{job_id}")
}

fn newer_history_id(current: String, candidate: String) -> String {
    match (current.parse::<u128>(), candidate.parse::<u128>()) {
        (Ok(current_id), Ok(candidate_id)) if current_id > candidate_id => current,
        _ => candidate,
    }
}

async fn call_gmail_profile(
    app: &AppHandle,
    account_id: &str,
    token: &str,
) -> Result<google::GmailProfile, AppError> {
    match google::get_profile(token).await {
        Ok(profile) => Ok(profile),
        Err(GoogleApiError::Unauthorized) => {
            let token =
                crate::connectors::force_refresh_google_access_token(app, account_id).await?;
            google::get_profile(&token).await.map_err(Into::into)
        }
        Err(error) => Err(error.into()),
    }
}

async fn call_gmail_history(
    app: &AppHandle,
    account_id: &str,
    token: &str,
    cursor: &str,
) -> Result<google::HistoryDelta, AppError> {
    // history.list is paginated; a busy inbox spreads new mail across several
    // pages. Drain every page before the caller advances the cursor, otherwise
    // mail beyond the first page is silently skipped and its trigger never fires.
    let mut token = token.to_string();
    let mut page_token: Option<String> = None;
    let mut added = Vec::new();
    let mut history_id: Option<String> = None;
    loop {
        let delta = match google::history_list(&token, cursor, page_token.as_deref()).await {
            Ok(delta) => delta,
            Err(GoogleApiError::Unauthorized) => {
                token =
                    crate::connectors::force_refresh_google_access_token(app, account_id).await?;
                google::history_list(&token, cursor, page_token.as_deref()).await?
            }
            Err(error) => return Err(error.into()),
        };
        added.extend(delta.added);
        // The top-level historyId is the mailbox's latest and is stable across
        // pages; keep the newest one seen as the cursor to persist.
        if delta.history_id.is_some() {
            history_id = delta.history_id;
        }
        match delta.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }
    Ok(google::HistoryDelta {
        added,
        next_page_token: None,
        history_id,
    })
}

// --- Calendar -----------------------------------------------------------------

#[derive(Debug)]
struct CalendarTriggerPlan {
    trigger: ConnectorTriggerRecord,
    lead_minutes: i64,
    external_only: bool,
}

#[derive(Debug)]
struct CalendarAccountPlan {
    account_id: String,
    max_lead_minutes: i64,
    triggers: Vec<CalendarTriggerPlan>,
}

fn calendar_account_plans(triggers: Vec<ConnectorTriggerRecord>) -> Vec<CalendarAccountPlan> {
    let mut by_account: HashMap<String, Vec<CalendarTriggerPlan>> = HashMap::new();
    for trigger in triggers {
        let account_id = trigger.account_id.clone();
        by_account
            .entry(account_id)
            .or_default()
            .push(CalendarTriggerPlan {
                lead_minutes: lead_minutes_from_config(&trigger.config),
                external_only: external_only_from_config(&trigger.config),
                trigger,
            });
    }

    let mut plans: Vec<CalendarAccountPlan> = by_account
        .into_iter()
        .map(|(account_id, triggers)| CalendarAccountPlan {
            max_lead_minutes: triggers
                .iter()
                .map(|trigger| trigger.lead_minutes)
                .max()
                .unwrap_or(DEFAULT_LEAD_MINUTES),
            account_id,
            triggers,
        })
        .collect();
    plans.sort_by(|left, right| left.account_id.cmp(&right.account_id));
    plans
}

async fn poll_calendar_accounts(
    app: &AppHandle,
    repos: &Repositories,
    triggers: Vec<ConnectorTriggerRecord>,
) {
    let plans = calendar_account_plans(triggers);
    let app = app.clone();
    let repos = repos.clone();
    let results =
        run_calendar_account_polls(plans, MAX_CALENDAR_ACCOUNT_CONCURRENCY, move |plan| {
            let app = app.clone();
            let repos = repos.clone();
            async move { poll_calendar_account(&app, &repos, plan).await }
        })
        .await;

    for result in results {
        if let Err(error) = result {
            tracing::warn!(
                error_code = %error.code,
                kind = EVENT_KIND,
                "connector calendar account poll failed"
            );
        }
    }
}

async fn run_calendar_account_polls<F, Fut>(
    plans: Vec<CalendarAccountPlan>,
    max_concurrency: usize,
    poll_account: F,
) -> Vec<Result<(), AppError>>
where
    F: Fn(CalendarAccountPlan) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<(), AppError>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(max_concurrency.max(1)));
    let mut tasks = JoinSet::new();
    for plan in plans {
        let semaphore = semaphore.clone();
        let poll_account = poll_account.clone();
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await.map_err(|error| {
                AppError::new(
                    "connector_calendar_poll_task_failed",
                    format!("Calendar poll concurrency gate closed: {error}"),
                )
            })?;
            poll_account(plan).await
        });
    }

    let mut results = Vec::with_capacity(tasks.len());
    while let Some(result) = tasks.join_next().await {
        results.push(result.unwrap_or_else(|error| {
            Err(AppError::new(
                "connector_calendar_poll_task_failed",
                format!("Calendar account poll task failed: {error}"),
            ))
        }));
    }
    results
}

async fn poll_calendar_account(
    app: &AppHandle,
    repos: &Repositories,
    plan: CalendarAccountPlan,
) -> Result<(), AppError> {
    let account_id = &plan.account_id;
    let token = crate::connectors::google_access_token(app, account_id).await?;
    let now = Utc::now();
    let items =
        match fetch_calendar_events(app, account_id, token, &now, plan.max_lead_minutes).await {
            Ok(items) => items,
            Err(error) if error.code == "connector_sync_token_expired" => {
                // Window mode never sends a sync token, but preserve the existing
                // defensive reset for every routine consuming this account.
                for trigger in &plan.triggers {
                    let cursor_kind = format!("{EVENT_KIND}:{}", trigger.trigger.job_id);
                    let _ = repos
                        .set_trigger_cursor(account_id, &cursor_kind, "{}")
                        .await;
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };

    // Every trigger keeps its own audience rule, lead time, fired cursor, and
    // wake result. Only the provider scan above is shared.
    for trigger in &plan.triggers {
        if let Err(error) =
            apply_calendar_events_to_trigger(app, repos, trigger, &now, &items).await
        {
            tracing::warn!(
                error_code = %error.code,
                kind = EVENT_KIND,
                "connector event trigger poll failed"
            );
        }
    }
    Ok(())
}

async fn fetch_calendar_events(
    app: &AppHandle,
    account_id: &str,
    mut token: String,
    now: &DateTime<Utc>,
    max_lead_minutes: i64,
) -> Result<Vec<google::EventSummary>, AppError> {
    // Scan a little past the lead window so an event is seen before it starts.
    let time_max = *now + ChronoDuration::minutes(max_lead_minutes + 5);
    let base_params = ListEventsParams {
        time_min: Some(now.to_rfc3339()),
        time_max: Some(time_max.to_rfc3339()),
        max_results: Some(25),
        ..ListEventsParams::default()
    };

    // Drain every page of the window. A busy calendar can hold more than one
    // 25-event page, and a matching event on a later page must not be missed,
    // or the routine silently skips its trigger. Bounded by a page cap so a
    // pathological calendar can never spin the poll forever.
    let mut items: Vec<google::EventSummary> = Vec::new();
    let mut page_token: Option<String> = None;
    for _ in 0..MAX_EVENT_PAGES {
        let params = ListEventsParams {
            page_token: page_token.take(),
            ..base_params.clone()
        };
        let page = match google::list_events(&token, &params).await {
            Ok(page) => page,
            Err(GoogleApiError::Unauthorized) => {
                token =
                    crate::connectors::force_refresh_google_access_token(app, account_id).await?;
                google::list_events(&token, &params).await?
            }
            Err(error) => return Err(error.into()),
        };
        items.extend(page.items);
        match page.next_page_token {
            Some(next) => page_token = Some(next),
            None => break,
        }
    }
    Ok(items)
}

async fn apply_calendar_events_to_trigger(
    app: &AppHandle,
    repos: &Repositories,
    plan: &CalendarTriggerPlan,
    now: &DateTime<Utc>,
    items: &[google::EventSummary],
) -> Result<(), AppError> {
    let trigger = &plan.trigger;
    let account_id = &trigger.account_id;
    let job_id = &trigger.job_id;
    let cursor_kind = format!("{EVENT_KIND}:{job_id}");
    let mut fired = load_fired(repos, account_id, &cursor_kind).await;
    let to_fire = calendar_events_to_fire(plan, now, items, &fired);

    // Prune events whose start time is in the past so the cursor stays small.
    let before = fired.len();
    fired.retain(|_, start_ts| *start_ts >= now.timestamp());
    let mut changed = fired.len() != before;

    // Wake the routine once for the batch, and record the events as fired only
    // when the wake actually queued. A failed or temporarily ineligible wake
    // leaves them unrecorded so this routine can retry independently.
    if !to_fire.is_empty() && fire(app, trigger).await == FireOutcome::Queued {
        for (event_id, start_ts) in to_fire {
            fired.insert(event_id, start_ts);
        }
        changed = true;
    }

    if changed {
        let json = serde_json::to_string(&fired).unwrap_or_else(|_| "{}".to_string());
        repos
            .set_trigger_cursor(account_id, &cursor_kind, &json)
            .await?;
    }
    Ok(())
}

fn calendar_events_to_fire(
    plan: &CalendarTriggerPlan,
    now: &DateTime<Utc>,
    items: &[google::EventSummary],
    fired: &HashMap<String, i64>,
) -> Vec<(String, i64)> {
    let account_domain = email_domain(&plan.trigger.account_id);
    let lead_cutoff = *now + ChronoDuration::minutes(plan.lead_minutes);
    let mut to_fire = Vec::new();
    for event in items {
        if fired.contains_key(&event.id) {
            continue;
        }
        let Some(start) = event.start.as_deref().and_then(parse_event_start) else {
            continue;
        };
        let starts_soon = start >= *now && start <= lead_cutoff;
        let has_external = event
            .attendees
            .iter()
            .any(|attendee| attendee_is_external(attendee, account_domain));
        // When the routine opts out of external-only, internal and solo events
        // fire too; otherwise a meeting needs at least one outside-domain guest.
        let passes_audience = !plan.external_only || has_external;
        if starts_soon && passes_audience {
            to_fire.push((event.id.clone(), start.timestamp()));
        }
    }
    to_fire
}

async fn load_fired(
    repos: &Repositories,
    account_id: &str,
    cursor_kind: &str,
) -> HashMap<String, i64> {
    repos
        .trigger_cursor(account_id, cursor_kind)
        .await
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn lead_minutes_from_config(config: &str) -> i64 {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(config) else {
        return DEFAULT_LEAD_MINUTES;
    };
    value
        .get("leadMinutes")
        .or_else(|| value.get("lead_minutes"))
        .and_then(serde_json::Value::as_i64)
        .filter(|minutes| *minutes > 0)
        .unwrap_or(DEFAULT_LEAD_MINUTES)
}

fn external_only_from_config(config: &str) -> bool {
    // Defaults to true so a legacy config missing the field keeps the safer
    // external-only behavior; the picker sends `externalOnly: false` explicitly
    // when the user opts internal meetings in.
    let Ok(value) = serde_json::from_str::<serde_json::Value>(config) else {
        return true;
    };
    value
        .get("externalOnly")
        .or_else(|| value.get("external_only"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

fn email_domain(email: &str) -> Option<&str> {
    email
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .filter(|domain| !domain.is_empty())
}

/// An attendee counts as external when their email domain differs from the
/// connected account's domain. A self attendee is never external, and an
/// attendee with no email is not evidence of an outside guest. When either
/// domain cannot be parsed, fall back to treating any addressed non-self
/// attendee as external so an external-only routine errs toward firing rather
/// than silently skipping a genuine outside guest.
fn attendee_is_external(attendee: &google::EventAttendee, account_domain: Option<&str>) -> bool {
    if attendee.is_self {
        return false;
    }
    let Some(email) = attendee.email.as_deref() else {
        return false;
    };
    match (email_domain(email), account_domain) {
        (Some(attendee_domain), Some(account_domain)) => {
            !attendee_domain.eq_ignore_ascii_case(account_domain)
        }
        _ => true,
    }
}

fn parse_event_start(start: &str) -> Option<DateTime<Utc>> {
    // Timed events carry an RFC 3339 dateTime; all-day events are a bare date
    // with no meeting time, so they are skipped for lead-time firing.
    DateTime::parse_from_rfc3339(start)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

// --- Firing & battery ---------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FireOutcome {
    Queued,
    NotQueued,
    Failed,
}

fn email_outcome_acknowledged(outcome: FireOutcome) -> bool {
    outcome != FireOutcome::Failed
}

/// Wake a routine and distinguish an ineligible subscription from a dispatch
/// failure. Email delivery acknowledges ineligible subscriptions, while
/// calendar delivery keeps its per-routine event pending.
async fn fire(app: &AppHandle, trigger: &ConnectorTriggerRecord) -> FireOutcome {
    match crate::routines::trigger_from_connector(
        app,
        &trigger.job_id,
        &trigger.id,
        &trigger.kind,
        &trigger.account_id,
    )
    .await
    {
        Ok(true) => FireOutcome::Queued,
        Ok(false) => FireOutcome::NotQueued,
        Err(error) => {
            tracing::warn!(
                error_code = %error.code,
                routine_id = %trigger.job_id,
                "connector routine trigger failed"
            );
            FireOutcome::Failed
        }
    }
}

/// True when the machine is on battery below the low threshold and
/// discharging, read from `pmset -g batt`. Any parse or spawn failure (for
/// example a desktop with no battery, or a non-macOS host) reports false, so
/// the daemon keeps its normal cadence.
fn battery_low_and_discharging() -> bool {
    let Ok(output) = crate::hide_console::command("pmset")
        .args(["-g", "batt"])
        .output()
    else {
        return false;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let discharging = text.contains("discharging");
    let low = parse_battery_percent(&text).is_some_and(|percent| percent < BATTERY_LOW_PERCENT);
    discharging && low
}

fn parse_battery_percent(text: &str) -> Option<u32> {
    let idx = text.find('%')?;
    let digits: String = text[..idx]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use tokio::sync::Notify;

    #[test]
    fn parses_battery_percent_from_pmset_output() {
        let sample = "Now drawing from 'Battery Power'\n \
             -InternalBattery-0 (id=1234)\t18%; discharging; 1:42 remaining present: true";
        assert_eq!(parse_battery_percent(sample), Some(18));
    }

    #[test]
    fn missing_percent_is_none() {
        assert_eq!(parse_battery_percent("no battery here"), None);
    }

    #[test]
    fn email_cursor_acknowledges_ineligible_routines_but_retries_dispatch_failures() {
        assert!(email_outcome_acknowledged(FireOutcome::Queued));
        assert!(email_outcome_acknowledged(FireOutcome::NotQueued));
        assert!(!email_outcome_acknowledged(FireOutcome::Failed));
    }

    #[test]
    fn email_subscribers_have_independent_monotonic_cursors() {
        assert_eq!(email_cursor_kind("routine-a"), "email_received:routine-a");
        assert_eq!(email_cursor_kind("routine-b"), "email_received:routine-b");
        assert_eq!(newer_history_id("105".into(), "103".into()), "105");
        assert_eq!(newer_history_id("105".into(), "108".into()), "108");
    }

    #[test]
    fn lead_minutes_reads_camel_and_snake_case() {
        assert_eq!(lead_minutes_from_config(r#"{"leadMinutes": 30}"#), 30);
        assert_eq!(lead_minutes_from_config(r#"{"lead_minutes": 45}"#), 45);
        assert_eq!(lead_minutes_from_config("{}"), DEFAULT_LEAD_MINUTES);
        assert_eq!(lead_minutes_from_config("not json"), DEFAULT_LEAD_MINUTES);
        assert_eq!(
            lead_minutes_from_config(r#"{"leadMinutes": 0}"#),
            DEFAULT_LEAD_MINUTES
        );
    }

    #[test]
    fn parses_timed_event_start_but_not_all_day() {
        assert!(parse_event_start("2026-07-10T09:00:00-07:00").is_some());
        assert!(parse_event_start("2026-07-11").is_none());
    }

    fn attendee(email: Option<&str>, is_self: bool) -> google::EventAttendee {
        google::EventAttendee {
            email: email.map(str::to_string),
            response_status: None,
            is_self,
        }
    }

    fn calendar_trigger(
        job_id: &str,
        account_id: &str,
        lead_minutes: i64,
        external_only: bool,
    ) -> ConnectorTriggerRecord {
        ConnectorTriggerRecord {
            id: format!("trigger-{job_id}"),
            job_id: job_id.to_string(),
            kind: EVENT_KIND.to_string(),
            account_id: account_id.to_string(),
            config: serde_json::json!({
                "leadMinutes": lead_minutes,
                "externalOnly": external_only,
            })
            .to_string(),
            created_at: "2026-07-24T08:00:00Z".to_string(),
        }
    }

    fn calendar_event(
        id: &str,
        now: &DateTime<Utc>,
        starts_in_minutes: i64,
        attendee_email: Option<&str>,
    ) -> google::EventSummary {
        google::EventSummary {
            id: id.to_string(),
            summary: None,
            start: Some((*now + ChronoDuration::minutes(starts_in_minutes)).to_rfc3339()),
            end: None,
            attendees: attendee_email
                .map(|email| vec![attendee(Some(email), false)])
                .unwrap_or_default(),
            location: None,
            organizer: None,
            html_link: None,
            status: None,
        }
    }

    fn fixed_calendar_now() -> DateTime<Utc> {
        parse_event_start("2026-07-24T10:00:00Z").expect("fixed test time")
    }

    fn sorted_event_ids(events: Vec<(String, i64)>) -> Vec<String> {
        let mut ids: Vec<String> = events.into_iter().map(|(id, _)| id).collect();
        ids.sort();
        ids
    }

    #[tokio::test]
    async fn ten_routines_share_one_calendar_fetch_without_changing_trigger_rules() {
        let account_id = "alex@acme.com";
        let triggers: Vec<ConnectorTriggerRecord> = (0..10)
            .map(|index| match index % 4 {
                0 => calendar_trigger(&format!("job-{index}"), account_id, 10, true),
                1 => calendar_trigger(&format!("job-{index}"), account_id, 10, false),
                2 => calendar_trigger(&format!("job-{index}"), account_id, 30, true),
                _ => calendar_trigger(&format!("job-{index}"), account_id, 30, false),
            })
            .collect();
        let plans = calendar_account_plans(triggers);
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].account_id, account_id);
        assert_eq!(plans[0].max_lead_minutes, 30);
        assert_eq!(plans[0].triggers.len(), 10);

        let now = Arc::new(fixed_calendar_now());
        // The shared maximum window can return events beyond a particular
        // routine's lead time. Local fanout must still apply that routine's
        // original cutoff and audience rule.
        let events = Arc::new(vec![
            calendar_event("internal-soon", &now, 5, Some("dana@acme.com")),
            calendar_event("external-soon", &now, 5, Some("guest@partner.com")),
            calendar_event("external-later", &now, 20, Some("guest@partner.com")),
            calendar_event("outside-lead", &now, 34, Some("guest@partner.com")),
        ]);
        let fetches = Arc::new(AtomicUsize::new(0));
        let observed = Arc::new(Mutex::new(HashMap::<String, Vec<String>>::new()));

        let results = run_calendar_account_polls(plans, 4, {
            let now = now.clone();
            let events = events.clone();
            let fetches = fetches.clone();
            let observed = observed.clone();
            move |plan| {
                let now = now.clone();
                let events = events.clone();
                let fetches = fetches.clone();
                let observed = observed.clone();
                async move {
                    fetches.fetch_add(1, Ordering::SeqCst);
                    for trigger in &plan.triggers {
                        assert_eq!(trigger.trigger.account_id, plan.account_id);
                        let mut fired = HashMap::new();
                        // Only job-0 has seen this event. The same-config job-4
                        // must still fire, proving fired state stays per routine.
                        if trigger.trigger.job_id == "job-0" {
                            fired.insert(
                                "external-soon".to_string(),
                                (*now + ChronoDuration::minutes(5)).timestamp(),
                            );
                        }
                        let ids = sorted_event_ids(calendar_events_to_fire(
                            trigger, &now, &events, &fired,
                        ));
                        observed
                            .lock()
                            .expect("observed trigger results")
                            .insert(trigger.trigger.job_id.clone(), ids);
                    }
                    Ok(())
                }
            }
        })
        .await;

        assert!(results.iter().all(Result::is_ok));
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        let observed = observed.lock().expect("observed trigger results");
        for index in 0..10 {
            let expected: Vec<&str> = match index % 4 {
                0 if index == 0 => vec![],
                0 => vec!["external-soon"],
                1 => vec!["external-soon", "internal-soon"],
                2 => vec!["external-later", "external-soon"],
                _ => vec!["external-later", "external-soon", "internal-soon"],
            };
            assert_eq!(
                observed.get(&format!("job-{index}")),
                Some(&expected.into_iter().map(str::to_string).collect())
            );
        }
    }

    #[tokio::test]
    async fn calendar_accounts_keep_fetches_and_trigger_results_isolated() {
        let plans = calendar_account_plans(vec![
            calendar_trigger("job-acme", "alex@acme.com", 15, true),
            calendar_trigger("job-beta", "bea@beta.com", 30, true),
        ]);
        assert_eq!(plans.len(), 2);

        let now = Arc::new(fixed_calendar_now());
        let events = Arc::new(HashMap::from([
            (
                "alex@acme.com".to_string(),
                vec![calendar_event(
                    "acme-external",
                    &now,
                    5,
                    Some("guest@partner.com"),
                )],
            ),
            (
                "bea@beta.com".to_string(),
                vec![calendar_event(
                    "beta-internal",
                    &now,
                    5,
                    Some("coworker@beta.com"),
                )],
            ),
        ]));
        let fetches = Arc::new(Mutex::new(HashMap::<String, usize>::new()));
        let observed = Arc::new(Mutex::new(HashMap::<String, Vec<String>>::new()));

        let results = run_calendar_account_polls(plans, 2, {
            let now = now.clone();
            let events = events.clone();
            let fetches = fetches.clone();
            let observed = observed.clone();
            move |plan| {
                let now = now.clone();
                let events = events.clone();
                let fetches = fetches.clone();
                let observed = observed.clone();
                async move {
                    *fetches
                        .lock()
                        .expect("fetch counts")
                        .entry(plan.account_id.clone())
                        .or_default() += 1;
                    let account_events = events.get(&plan.account_id).expect("events for account");
                    for trigger in &plan.triggers {
                        assert_eq!(trigger.trigger.account_id, plan.account_id);
                        let ids = sorted_event_ids(calendar_events_to_fire(
                            trigger,
                            &now,
                            account_events,
                            &HashMap::new(),
                        ));
                        observed
                            .lock()
                            .expect("observed trigger results")
                            .insert(trigger.trigger.job_id.clone(), ids);
                    }
                    Ok(())
                }
            }
        })
        .await;

        assert!(results.iter().all(Result::is_ok));
        assert_eq!(
            *fetches.lock().expect("fetch counts"),
            HashMap::from([
                ("alex@acme.com".to_string(), 1),
                ("bea@beta.com".to_string(), 1),
            ])
        );
        assert_eq!(
            *observed.lock().expect("observed trigger results"),
            HashMap::from([
                ("job-acme".to_string(), vec!["acme-external".to_string()]),
                ("job-beta".to_string(), vec![]),
            ])
        );
    }

    #[tokio::test]
    async fn calendar_account_poll_concurrency_is_bounded() {
        let account_count = MAX_CALENDAR_ACCOUNT_CONCURRENCY + 3;
        let plans = calendar_account_plans(
            (0..account_count)
                .map(|index| {
                    let account_id = format!("user-{index}@example.com");
                    calendar_trigger(&format!("job-{index}"), &account_id, 15, true)
                })
                .collect(),
        );
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));

        let results = run_calendar_account_polls(plans, MAX_CALENDAR_ACCOUNT_CONCURRENCY, {
            let active = active.clone();
            let peak = peak.clone();
            move |_| {
                let active = active.clone();
                let peak = peak.clone();
                async move {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                }
            }
        })
        .await;

        assert!(results.iter().all(Result::is_ok));
        assert_eq!(
            peak.load(Ordering::SeqCst),
            MAX_CALENDAR_ACCOUNT_CONCURRENCY
        );
    }

    #[tokio::test]
    async fn slow_calendar_account_does_not_starve_another_account() {
        let plans = calendar_account_plans(vec![
            calendar_trigger("job-slow", "a-slow@example.com", 15, true),
            calendar_trigger("job-fast", "b-fast@example.com", 15, true),
        ]);
        let slow_started = Arc::new(Notify::new());
        let release_slow = Arc::new(Notify::new());
        let fast_finished = Arc::new(Notify::new());
        let polling = tokio::spawn(run_calendar_account_polls(plans, 2, {
            let slow_started = slow_started.clone();
            let release_slow = release_slow.clone();
            let fast_finished = fast_finished.clone();
            move |plan| {
                let slow_started = slow_started.clone();
                let release_slow = release_slow.clone();
                let fast_finished = fast_finished.clone();
                async move {
                    if plan.account_id.starts_with("a-slow") {
                        slow_started.notify_one();
                        release_slow.notified().await;
                    } else {
                        fast_finished.notify_one();
                    }
                    Ok(())
                }
            }
        }));

        tokio::time::timeout(Duration::from_secs(1), slow_started.notified())
            .await
            .expect("slow account started");
        tokio::time::timeout(Duration::from_secs(1), fast_finished.notified())
            .await
            .expect("fast account finished while the slow account was blocked");
        assert!(!polling.is_finished());
        release_slow.notify_one();
        let results = polling.await.expect("calendar polling task");
        assert!(results.iter().all(Result::is_ok));
    }

    #[tokio::test]
    #[ignore = "JUN-426 diagnostic benchmark"]
    async fn benchmark_calendar_account_poll_consolidation() {
        let now = fixed_calendar_now();
        let events = Arc::new(vec![calendar_event(
            "benchmark-event",
            &now,
            5,
            Some("guest@partner.com"),
        )]);
        for routine_count in [1_usize, 10, 100] {
            let plans = calendar_account_plans(
                (0..routine_count)
                    .map(|index| {
                        calendar_trigger(
                            &format!("benchmark-job-{index}"),
                            "benchmark@acme.com",
                            15,
                            true,
                        )
                    })
                    .collect(),
            );
            let credential_loads = Arc::new(AtomicUsize::new(0));
            let google_requests = Arc::new(AtomicUsize::new(0));
            let trigger_evaluations = Arc::new(AtomicUsize::new(0));
            let active = Arc::new(AtomicUsize::new(0));
            let peak = Arc::new(AtomicUsize::new(0));
            let started = Instant::now();

            let results = run_calendar_account_polls(plans, 4, {
                let events = events.clone();
                let credential_loads = credential_loads.clone();
                let google_requests = google_requests.clone();
                let trigger_evaluations = trigger_evaluations.clone();
                let active = active.clone();
                let peak = peak.clone();
                move |plan| {
                    let events = events.clone();
                    let credential_loads = credential_loads.clone();
                    let google_requests = google_requests.clone();
                    let trigger_evaluations = trigger_evaluations.clone();
                    let active = active.clone();
                    let peak = peak.clone();
                    async move {
                        credential_loads.fetch_add(1, Ordering::SeqCst);
                        google_requests.fetch_add(1, Ordering::SeqCst);
                        let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(current, Ordering::SeqCst);
                        for trigger in &plan.triggers {
                            let _ = calendar_events_to_fire(
                                trigger,
                                &fixed_calendar_now(),
                                &events,
                                &HashMap::new(),
                            );
                            trigger_evaluations.fetch_add(1, Ordering::SeqCst);
                        }
                        active.fetch_sub(1, Ordering::SeqCst);
                        Ok(())
                    }
                }
            })
            .await;
            assert!(results.iter().all(Result::is_ok));

            println!(
                "JUN426_BENCHMARK {}",
                serde_json::json!({
                    "routines": routine_count,
                    "accounts": 1,
                    "credentialLoads": credential_loads.load(Ordering::SeqCst),
                    "googleRequests": google_requests.load(Ordering::SeqCst),
                    "triggerEvaluations": trigger_evaluations.load(Ordering::SeqCst),
                    "wallTimeMicros": started.elapsed().as_micros(),
                    "peakConcurrency": peak.load(Ordering::SeqCst),
                })
            );
        }
    }

    #[test]
    fn attendee_is_external_compares_domains_against_the_account() {
        let domain = email_domain("alex@acme.com");
        assert_eq!(domain, Some("acme.com"));

        // Same domain coworker: internal.
        assert!(!attendee_is_external(
            &attendee(Some("dana@acme.com"), false),
            domain
        ));
        // Different domain guest: external.
        assert!(attendee_is_external(
            &attendee(Some("guest@partner.com"), false),
            domain
        ));
        // Case-insensitive domain match stays internal.
        assert!(!attendee_is_external(
            &attendee(Some("sam@ACME.com"), false),
            domain
        ));
        // Self is never external; a missing email is not an outside guest.
        assert!(!attendee_is_external(
            &attendee(Some("alex@acme.com"), true),
            domain
        ));
        assert!(!attendee_is_external(&attendee(None, false), domain));
        // Unknown account domain falls back to "any addressed non-self guest".
        assert!(attendee_is_external(
            &attendee(Some("dana@acme.com"), false),
            None
        ));
    }

    #[test]
    fn external_only_defaults_true_but_honors_an_explicit_false() {
        // Missing field or unparseable config keeps the safer external-only gate.
        assert!(external_only_from_config("{}"));
        assert!(external_only_from_config("not json"));
        assert!(external_only_from_config(r#"{"leadMinutes": 30}"#));
        // The picker opts internal meetings in by sending an explicit false.
        assert!(!external_only_from_config(r#"{"externalOnly": false}"#));
        assert!(!external_only_from_config(r#"{"external_only": false}"#));
        assert!(external_only_from_config(r#"{"externalOnly": true}"#));
    }
}
