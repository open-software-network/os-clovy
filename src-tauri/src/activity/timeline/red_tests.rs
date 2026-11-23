//! Acceptance test for the timeline slice (WT-20261004 S3): app switch and a
//! system-sleep gap through the real ETL, store, and day statistics.

use chrono::{DateTime, Duration, TimeZone, Utc};

use super::builder::GapKind;
use super::etl::run_pass;
use super::timeline_view;
use crate::activity::key::MemoryKeyStore;
use crate::activity::settings::ActivitySettings;
use crate::activity::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, hour, minute, 0).unwrap()
}

async fn frames(
    store: &ActivityStore,
    start: DateTime<Utc>,
    count: i64,
    app: &str,
    title: &str,
    url: Option<&str>,
) {
    for index in 0..count {
        store
            .insert_frame(&NewFrame {
                captured_at: start + Duration::seconds(index * 2),
                app_name: app.into(),
                bundle_id: None,
                window_title: Some(title.into()),
                browser_url: url.map(str::to_string),
                text_source: TextSource::Accessibility,
                text: Some(format!("{title} {index}")),
                input_idle_ms: None,
            })
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn activity_timeline_switch_and_sleep_gap_are_excluded_from_session_time() {
    let dir = tempfile::tempdir().unwrap();
    let store = ActivityStore::open(
        &dir.path().join(ACTIVITY_DB_FILE),
        &MemoryKeyStore::default(),
    )
    .await
    .unwrap();
    // Editor 10 min, browser 5 min, the Mac sleeps 9:15 to 10:30, editor again.
    frames(
        &store,
        at(9, 0),
        300,
        "Code",
        "main.rs \u{2014} os-clovy",
        None,
    )
    .await;
    frames(
        &store,
        at(9, 10),
        150,
        "Google Chrome",
        "Docs",
        Some("https://docs.rs/x"),
    )
    .await;
    frames(
        &store,
        at(10, 30),
        30,
        "Code",
        "main.rs \u{2014} os-clovy",
        None,
    )
    .await;
    run_pass(&store, at(10, 31), &[]).await.unwrap();

    let settings = ActivitySettings {
        enabled: true,
        ..ActivitySettings::default()
    };
    let view = timeline_view(&store, &settings, at(0, 0), at(23, 59), at(10, 31))
        .await
        .unwrap();
    let sessions: Vec<(&str, i64, bool)> = view
        .sessions
        .iter()
        .map(|session| {
            (
                session.app_name.as_str(),
                session.duration_ms / 1000,
                session.active,
            )
        })
        .collect();
    assert_eq!(
        sessions,
        vec![
            ("Code", 600, false),
            ("Google Chrome", 300, false),
            ("Code", 60, true)
        ]
    );
    assert_eq!(view.gaps.len(), 1);
    assert_eq!(view.gaps[0].kind, GapKind::Sleep);
    assert_eq!(view.gaps[0].duration_ms, 75 * 60_000);
    assert_eq!(view.stats.focused_ms, 16 * 60_000);
    assert_eq!(view.stats.away_ms, 75 * 60_000);
    let by_category: i64 = view
        .stats
        .categories
        .iter()
        .map(|entry| entry.duration_ms)
        .sum();
    assert_eq!(by_category, view.stats.focused_ms);
}
