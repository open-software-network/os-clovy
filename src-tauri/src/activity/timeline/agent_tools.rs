//! Agent host tools over the timeline, named in the `activity-timeline` spec
//! (`openspec/changes/add-activity-intelligence`): `search_activity` and
//! `get_activity_timeline`. They are offered only while capture is enabled
//! and answer through the same filtered queries as the "Today" view, so
//! exclusions and retention apply to the agent too.

use chrono::{DateTime, Local, Utc};
use serde_json::{json, Value};

use super::db::{TimelineGapDto, TimelineSessionDto};
use super::stats::TimelineStatsDto;
use super::{search_view, timeline_view};
use crate::activity::settings::ActivitySettings;
use crate::activity::store::ActivityStore;
use crate::day_intelligence::schedule::{self, LocalZone, SystemZone};
use crate::domain::types::AppError;

pub const SEARCH_ACTIVITY: &str = "search_activity";
pub const GET_ACTIVITY_TIMELINE: &str = "get_activity_timeline";
const MAX_SESSIONS: usize = 300;
const MAX_GAPS: usize = 300;
const DEFAULT_RESULTS: u32 = 20;
const MAX_RESULTS: u32 = 50;

pub fn is_activity_tool(name: &str) -> bool {
    name == SEARCH_ACTIVITY || name == GET_ACTIVITY_TIMELINE
}

/// Tool descriptors for the agent catalog; empty unless capture is enabled.
pub fn descriptors(capture_enabled: bool) -> Vec<Value> {
    if !capture_enabled {
        return Vec::new();
    }
    vec![
        json!({
            "name": GET_ACTIVITY_TIMELINE,
            "description": "Read the user's activity timeline (app sessions with browser domain or editor workspace, category, and gaps for idle, sleep, or paused capture) and its totals for a period. Sessions and gaps are complete records overlapping [from, to); totals count only time within the requested period. Active state comes from the materialized timeline, updated by the activity ETL approximately every 30 seconds. Times are RFC 3339; omit both bounds for today so far. Long periods are truncated (see `truncated`, `totalSessions`, `totalGaps`); ask for a shorter period to see everything.",
            "parameters": {
                "type": "object",
                "properties": {
                    "from": { "type": "string", "description": "Start, RFC 3339 with offset (default: local midnight today)." },
                    "to": { "type": "string", "description": "End, RFC 3339 with offset (default: now)." }
                },
                "required": [],
                "additionalProperties": false
            }
        }),
        json!({
            "name": SEARCH_ACTIVITY,
            "description": "Search what was on the user's screen (window titles, URLs, captured text) in their activity history, optionally within a period. Returns the moment, app, window, and a text snippet.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" },
                    "from": { "type": "string", "description": "Optional start, RFC 3339 with offset." },
                    "to": { "type": "string", "description": "Optional end, RFC 3339 with offset." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_RESULTS, "default": DEFAULT_RESULTS }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }),
    ]
}

fn invalid(message: impl Into<String>) -> AppError {
    AppError::new("activity_tool_invalid_arguments", message.into())
}

pub(crate) fn time_argument(
    arguments: &Value,
    key: &str,
) -> Result<Option<DateTime<Utc>>, AppError> {
    match arguments.get(key).and_then(Value::as_str) {
        None => Ok(None),
        Some(value) if value.trim().is_empty() => Ok(None),
        Some(value) => DateTime::parse_from_rfc3339(value.trim())
            .map(|at| Some(at.with_timezone(&Utc)))
            .map_err(|_| invalid(format!("`{key}` must be an RFC 3339 timestamp"))),
    }
}

pub(crate) fn local(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|at| at.with_timezone(&Local).to_rfc3339())
        .unwrap_or_else(|_| value.to_string())
}

pub(crate) fn minutes(ms: i64) -> f64 {
    (ms as f64 / 60_000.0 * 10.0).round() / 10.0
}

pub(crate) fn local_at(at: DateTime<Utc>) -> String {
    at.with_timezone(&Local).to_rfc3339()
}

/// The `from` / `to` arguments of a period: local midnight today (in `zone`)
/// and now by default; `to` must be after `from`.
pub(crate) fn range(
    arguments: &Value,
    zone: &dyn LocalZone,
    now: DateTime<Utc>,
) -> Result<(DateTime<Utc>, DateTime<Utc>), AppError> {
    let from =
        time_argument(arguments, "from")?.unwrap_or_else(|| schedule::local_midnight(zone, now, 0));
    let to = time_argument(arguments, "to")?.unwrap_or(now);
    if to <= from {
        return Err(invalid("`to` must be after `from`"));
    }
    Ok((from, to))
}

pub(crate) fn session_json(session: &TimelineSessionDto) -> Value {
    json!({
        "id": session.id,
        "app": session.app_name,
        "context": session.context,
        "contextKind": session.context_kind,
        "category": session.category,
        "startedAt": local(&session.started_at),
        "endedAt": local(&session.ended_at),
        "minutes": minutes(session.duration_ms),
        "active": session.active,
        "windowTitle": session.window_title,
    })
}

pub(crate) fn gap_json(gap: &TimelineGapDto) -> Value {
    json!({
        "kind": gap.kind,
        "pauseReason": gap.pause_reason,
        "startedAt": local(&gap.started_at),
        "endedAt": local(&gap.ended_at),
        "minutes": minutes(gap.duration_ms),
        "ongoing": gap.ongoing,
    })
}

pub(crate) fn stats_json(stats: &TimelineStatsDto) -> Value {
    json!({
        "focusedMinutes": minutes(stats.focused_ms),
        "idleMinutes": minutes(stats.idle_ms),
        "awayMinutes": minutes(stats.away_ms),
        "topApps": stats.top_apps.iter().map(|app| json!({ "app": app.app_name, "bundleId": app.bundle_id, "minutes": minutes(app.duration_ms) })).collect::<Vec<_>>(),
        "categories": stats.categories.iter().map(|entry| json!({ "category": entry.category, "minutes": minutes(entry.duration_ms) })).collect::<Vec<_>>(),
    })
}

/// Runs an activity tool against an open store. `None` when `name` is not
/// one of them.
pub async fn dispatch(
    store: &ActivityStore,
    settings: &ActivitySettings,
    name: &str,
    arguments: &Value,
    now: DateTime<Utc>,
) -> Option<Result<Value, AppError>> {
    match name {
        GET_ACTIVITY_TIMELINE => Some(get_timeline(store, settings, arguments, now).await),
        SEARCH_ACTIVITY => Some(search(store, settings, arguments, now).await),
        _ => None,
    }
}

async fn get_timeline(
    store: &ActivityStore,
    settings: &ActivitySettings,
    arguments: &Value,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let (from, to) = range(arguments, &SystemZone, now)?;
    let view = timeline_view(store, settings, from, to, now)
        .await
        .map_err(|error| AppError::new("activity_timeline_failed", error.to_string()))?;
    let truncated = view.sessions.len() > MAX_SESSIONS || view.gaps.len() > MAX_GAPS;
    Ok(json!({
        "from": local_at(from),
        "to": local_at(to),
        "sessions": view.sessions.iter().take(MAX_SESSIONS).map(session_json).collect::<Vec<_>>(),
        "gaps": view.gaps.iter().take(MAX_GAPS).map(gap_json).collect::<Vec<_>>(),
        "totalSessions": view.sessions.len(),
        "totalGaps": view.gaps.len(),
        "stats": stats_json(&view.stats),
        "truncated": truncated,
    }))
}

async fn search(
    store: &ActivityStore,
    settings: &ActivitySettings,
    arguments: &Value,
    now: DateTime<Utc>,
) -> Result<Value, AppError> {
    let query = arguments
        .get("query")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| invalid("`query` is required"))?;
    let limit = arguments
        .get("limit")
        .and_then(Value::as_u64)
        .map_or(DEFAULT_RESULTS, |limit| {
            u32::try_from(limit)
                .unwrap_or(MAX_RESULTS)
                .clamp(1, MAX_RESULTS)
        });
    let from = time_argument(arguments, "from")?;
    let to = time_argument(arguments, "to")?;
    let results = search_view(store, settings, query, from, to, limit, now)
        .await
        .map_err(|error| AppError::new("activity_search_failed", error.to_string()))?;
    Ok(json!({
        "results": results.iter().map(|result| json!({
            "sessionId": result.session_id,
            "app": result.app_name,
            "windowTitle": result.window_title,
            "url": result.browser_url,
            "seenAt": local(&result.seen_at),
            "snippet": result.snippet,
        })).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::key::MemoryKeyStore;
    use crate::activity::store::{NewFrame, TextSource, ACTIVITY_DB_FILE};
    use crate::activity::timeline::etl::run_pass;
    use chrono::{Duration, TimeZone};

    fn at(hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, hour, minute, 0).unwrap()
    }

    async fn store_with_morning() -> (tempfile::TempDir, ActivityStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ActivityStore::open(
            &dir.path().join(ACTIVITY_DB_FILE),
            &MemoryKeyStore::default(),
        )
        .await
        .unwrap();
        let runs = [
            (
                at(9, 0),
                150,
                "Code",
                "etl.rs \u{2014} os-clovy",
                None,
                "fn run_pass",
            ),
            (
                at(9, 5),
                60,
                "Firefox",
                "Rust FTS",
                Some("https://docs.rs/zephyrine"),
                "zephyrine crate docs",
            ),
            (at(9, 7), 30, "1Password", "Vault", None, "secret"),
        ];
        for (start, count, app, title, url, text) in runs {
            for index in 0..count {
                store
                    .insert_frame(&NewFrame {
                        captured_at: start + Duration::seconds(index * 2),
                        app_name: app.into(),
                        bundle_id: None,
                        window_title: Some(title.into()),
                        browser_url: url.map(str::to_string),
                        text_source: TextSource::Accessibility,
                        text: Some(format!("{text} {index}")),
                        input_idle_ms: None,
                    })
                    .await
                    .unwrap();
            }
        }
        run_pass(&store, at(9, 8), &[]).await.unwrap();
        (dir, store)
    }

    /// Santiago skips midnight on 2026-09-06: a period without `from` starts
    /// at 01:00 that day, not 24 hours before now.
    #[test]
    fn a_default_period_starts_at_the_first_minute_of_a_day_without_midnight() {
        use crate::day_intelligence::schedule::NamedZone;
        use chrono_tz::America::Santiago;
        let now = Santiago
            .with_ymd_and_hms(2026, 9, 6, 15, 0, 0)
            .unwrap()
            .with_timezone(&Utc);
        let (from, to) = range(&json!({}), &NamedZone(Santiago), now).unwrap();
        assert_eq!(
            from,
            Santiago.with_ymd_and_hms(2026, 9, 6, 1, 0, 0).unwrap()
        );
        assert_eq!(to, now);
        assert!(range(
            &json!({ "to": "2026-09-06T00:00:00Z" }),
            &NamedZone(Santiago),
            now
        )
        .is_err());
    }

    #[test]
    fn descriptors_exist_only_while_capture_is_enabled() {
        assert!(descriptors(false).is_empty());
        let names: Vec<String> = descriptors(true)
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec![GET_ACTIVITY_TIMELINE, SEARCH_ACTIVITY]);
    }

    #[tokio::test]
    async fn get_activity_timeline_returns_the_morning_sessions() {
        let (_dir, store) = store_with_morning().await;
        let settings = ActivitySettings {
            enabled: true,
            ..ActivitySettings::default()
        };
        let arguments = json!({ "from": "2026-10-01T08:00:00Z", "to": "2026-10-01T12:00:00Z" });
        let value = dispatch(
            &store,
            &settings,
            GET_ACTIVITY_TIMELINE,
            &arguments,
            at(9, 8),
        )
        .await
        .expect("routed")
        .expect("ok");
        let apps: Vec<&str> = value["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|session| session["app"].as_str().unwrap())
            .collect();
        assert_eq!(apps, vec!["Code", "Firefox", "1Password"]);
        assert_eq!(value["sessions"][0]["minutes"], json!(5.0));
        assert_eq!(value["sessions"][0]["category"], json!("coding"));
        assert_eq!(value["sessions"][1]["context"], json!("docs.rs"));
        assert_eq!(value["stats"]["focusedMinutes"], json!(8.0));
    }

    #[tokio::test]
    async fn current_exclusions_hide_sessions_and_search_results() {
        let (_dir, store) = store_with_morning().await;
        let settings = ActivitySettings {
            enabled: true,
            ignored_apps: vec!["1password".into()],
            ignored_domains: vec!["docs.rs".into()],
            ..ActivitySettings::default()
        };
        let value = dispatch(
            &store,
            &settings,
            GET_ACTIVITY_TIMELINE,
            &json!({ "from": "2026-10-01T08:00:00Z", "to": "2026-10-01T12:00:00Z" }),
            at(9, 8),
        )
        .await
        .unwrap()
        .unwrap();
        let apps: Vec<&str> = value["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|session| session["app"].as_str().unwrap())
            .collect();
        assert_eq!(apps, vec!["Code"]);
        let found = dispatch(
            &store,
            &settings,
            SEARCH_ACTIVITY,
            &json!({ "query": "zephyrine" }),
            at(9, 8),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(found["results"], json!([]));
    }

    #[tokio::test]
    async fn get_activity_timeline_bounds_gaps_and_flags_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let store = ActivityStore::open(
            &dir.path().join(ACTIVITY_DB_FILE),
            &MemoryKeyStore::default(),
        )
        .await
        .unwrap();
        // One frame every 6 minutes: every interval is a gap.
        let start = at(0, 0);
        for index in 0..310 {
            store
                .insert_frame(&NewFrame {
                    captured_at: start + Duration::minutes(index * 6),
                    app_name: "Zed".into(),
                    bundle_id: None,
                    window_title: Some("p \u{2014} a.rs".into()),
                    browser_url: None,
                    text_source: TextSource::Accessibility,
                    text: Some(format!("fn f{index}")),
                    input_idle_ms: None,
                })
                .await
                .unwrap();
        }
        let now = start + Duration::minutes(309 * 6) + Duration::seconds(30);
        run_pass(&store, now, &[]).await.unwrap();
        let settings = ActivitySettings {
            enabled: true,
            ..ActivitySettings::default()
        };
        let arguments = json!({ "from": "2026-10-01T00:00:00Z", "to": "2026-10-03T00:00:00Z" });
        let value = dispatch(&store, &settings, GET_ACTIVITY_TIMELINE, &arguments, now)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(value["gaps"].as_array().unwrap().len(), MAX_GAPS);
        assert_eq!(value["sessions"].as_array().unwrap().len(), MAX_SESSIONS);
        assert_eq!(value["totalGaps"], json!(309));
        assert_eq!(value["totalSessions"], json!(310));
        assert_eq!(value["truncated"], json!(true));
    }

    #[tokio::test]
    async fn search_activity_finds_page_text_with_its_moment() {
        let (_dir, store) = store_with_morning().await;
        let settings = ActivitySettings {
            enabled: true,
            ..ActivitySettings::default()
        };
        let value = dispatch(
            &store,
            &settings,
            SEARCH_ACTIVITY,
            &json!({ "query": "zephyrine", "from": "2026-10-01T00:00:00Z" }),
            at(9, 8),
        )
        .await
        .unwrap()
        .unwrap();
        let results = value["results"].as_array().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["app"], json!("Firefox"));
        assert_eq!(results[0]["url"], json!("https://docs.rs/zephyrine"));
        let missing = dispatch(&store, &settings, SEARCH_ACTIVITY, &json!({}), at(9, 8))
            .await
            .unwrap();
        assert!(missing.is_err());
        assert!(
            dispatch(&store, &settings, "search_june", &json!({}), at(9, 8))
                .await
                .is_none()
        );
    }
}
