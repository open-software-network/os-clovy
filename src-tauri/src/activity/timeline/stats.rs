//! Day statistics from recorded sessions and gaps, clipped to the requested
//! range: focused time is session time, idle time is idle gaps, away time is
//! sleep and pause gaps. Category totals always add up to focused time.

use chrono::{DateTime, Utc};
use serde::Serialize;

use super::builder::GapKind;
use super::categorize::Category;
use super::db::{TimelineGapDto, TimelineSessionDto};
use crate::activity::store::parse_time;

const TOP_APPS: usize = 5;

/// Time in one app. Apps are told apart by bundle id, so two apps with the
/// same name stay two rows; the name groups only sessions without an id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppTime {
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub duration_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryTime {
    pub category: Category,
    pub duration_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineStatsDto {
    pub focused_ms: i64,
    pub idle_ms: i64,
    pub away_ms: i64,
    pub top_apps: Vec<AppTime>,
    pub categories: Vec<CategoryTime>,
}

pub(crate) fn clipped_ms(start: &str, end: &str, from: DateTime<Utc>, to: DateTime<Utc>) -> i64 {
    match (parse_time(start), parse_time(end)) {
        (Some(start), Some(end)) => (end.min(to) - start.max(from)).num_milliseconds().max(0),
        _ => 0,
    }
}

fn add_to<K: PartialEq>(totals: &mut Vec<(K, i64)>, key: K, ms: i64) {
    match totals.iter_mut().find(|(existing, _)| *existing == key) {
        Some((_, total)) => *total += ms,
        None => totals.push((key, ms)),
    }
}

pub fn day_stats(
    sessions: &[TimelineSessionDto],
    gaps: &[TimelineGapDto],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> TimelineStatsDto {
    let mut stats = TimelineStatsDto::default();
    let mut apps: Vec<AppTime> = Vec::new();
    let mut categories: Vec<(Category, i64)> = Vec::new();
    for session in sessions {
        let ms = clipped_ms(&session.started_at, &session.ended_at, from, to);
        if ms == 0 {
            continue;
        }
        stats.focused_ms += ms;
        let same_app = |app: &&mut AppTime| match (&app.bundle_id, &session.bundle_id) {
            (Some(known), Some(bundle)) => known == bundle,
            (None, None) => app.app_name == session.app_name,
            _ => false,
        };
        match apps.iter_mut().find(same_app) {
            Some(app) => app.duration_ms += ms,
            None => apps.push(AppTime {
                app_name: session.app_name.clone(),
                bundle_id: session.bundle_id.clone(),
                duration_ms: ms,
            }),
        }
        add_to(&mut categories, session.category, ms);
    }
    for gap in gaps {
        let ms = clipped_ms(&gap.started_at, &gap.ended_at, from, to);
        match gap.kind {
            GapKind::Idle => stats.idle_ms += ms,
            GapKind::Sleep | GapKind::Paused => stats.away_ms += ms,
        }
    }
    apps.sort_by(|a, b| {
        b.duration_ms
            .cmp(&a.duration_ms)
            .then_with(|| a.app_name.cmp(&b.app_name))
            .then_with(|| a.bundle_id.cmp(&b.bundle_id))
    });
    apps.truncate(TOP_APPS);
    stats.top_apps = apps;
    categories.sort_by_key(|entry| std::cmp::Reverse(entry.1));
    stats.categories = categories
        .into_iter()
        .map(|(category, duration_ms)| CategoryTime {
            category,
            duration_ms,
        })
        .collect();
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::store::timestamp;
    use chrono::{Duration, TimeZone};

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 2, hour, 0, 0).unwrap()
    }

    fn session(
        id: i64,
        app: &str,
        category: Category,
        from: DateTime<Utc>,
        hours: i64,
    ) -> TimelineSessionDto {
        let to = from + Duration::hours(hours);
        TimelineSessionDto {
            id,
            app_name: app.into(),
            bundle_id: None,
            context_kind: None,
            context: None,
            started_at: timestamp(from),
            ended_at: timestamp(to),
            duration_ms: (to - from).num_milliseconds(),
            active: false,
            category,
            confidence: 0.9,
            window_title: None,
        }
    }

    fn gap(id: i64, kind: GapKind, from: DateTime<Utc>, hours: i64) -> TimelineGapDto {
        let to = from + Duration::hours(hours);
        TimelineGapDto {
            id,
            started_at: timestamp(from),
            ended_at: timestamp(to),
            duration_ms: (to - from).num_milliseconds(),
            kind,
            pause_reason: None,
            ongoing: false,
        }
    }

    #[test]
    fn six_hours_of_sessions_and_one_idle_hour_add_up() {
        let sessions = [
            session(1, "Cursor", Category::Coding, at(8), 3),
            session(2, "Slack", Category::Communication, at(11), 1),
            session(3, "Cursor", Category::Coding, at(13), 2),
        ];
        let gaps = [
            gap(1, GapKind::Idle, at(12), 1),
            gap(2, GapKind::Sleep, at(0), 8),
        ];
        let stats = day_stats(&sessions, &gaps, at(0), at(0) + Duration::days(1));
        assert_eq!(stats.focused_ms, Duration::hours(6).num_milliseconds());
        assert_eq!(stats.idle_ms, Duration::hours(1).num_milliseconds());
        assert_eq!(stats.away_ms, Duration::hours(8).num_milliseconds());
        let by_category: i64 = stats.categories.iter().map(|entry| entry.duration_ms).sum();
        assert_eq!(by_category, stats.focused_ms);
        assert_eq!(stats.categories[0].category, Category::Coding);
        assert_eq!(stats.top_apps[0].app_name, "Cursor");
        assert_eq!(
            stats.top_apps[0].duration_ms,
            Duration::hours(5).num_milliseconds()
        );
    }

    #[test]
    fn sessions_across_midnight_count_only_inside_the_day() {
        let sessions = [session(1, "Zed", Category::Coding, at(23), 2)];
        let day_end = at(0) + Duration::days(1);
        let stats = day_stats(&sessions, &[], at(0), day_end);
        assert_eq!(stats.focused_ms, Duration::hours(1).num_milliseconds());
    }

    #[test]
    fn top_apps_tell_apps_apart_by_bundle_id_not_by_name() {
        let with_bundle = |id, name: &str, bundle: Option<&str>, from| TimelineSessionDto {
            bundle_id: bundle.map(str::to_string),
            ..session(id, name, Category::Coding, from, 1)
        };
        let sessions = [
            // Two different apps that happen to share a name.
            with_bundle(1, "Notes", Some("com.apple.Notes"), at(8)),
            with_bundle(2, "Notes", Some("com.example.notes"), at(9)),
            // One app renamed by an update keeps its single row.
            with_bundle(3, "Code", Some("com.microsoft.VSCode"), at(10)),
            with_bundle(
                4,
                "Visual Studio Code",
                Some("com.microsoft.VSCode"),
                at(11),
            ),
            // Without an id, the name is all there is.
            with_bundle(5, "Legacy", None, at(12)),
        ];
        let stats = day_stats(&sessions, &[], at(0), at(0) + Duration::days(1));
        let rows: Vec<(&str, Option<&str>, i64)> = stats
            .top_apps
            .iter()
            .map(|app| {
                (
                    app.app_name.as_str(),
                    app.bundle_id.as_deref(),
                    app.duration_ms / 3_600_000,
                )
            })
            .collect();
        assert_eq!(
            rows,
            vec![
                ("Code", Some("com.microsoft.VSCode"), 2),
                ("Legacy", None, 1),
                ("Notes", Some("com.apple.Notes"), 1),
                ("Notes", Some("com.example.notes"), 1),
            ]
        );
    }
}
