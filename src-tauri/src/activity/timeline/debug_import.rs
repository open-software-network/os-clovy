//! Development builds only: seeds the activity database from a JSON fixture
//! named by `CLOVY_ACTIVITY_DEBUG_IMPORT` when the timeline starts, so the
//! "Today" view can be verified with test data (for example a previous day).
//! Rows go through the store's normal write API; a marker per fixture digest
//! in the data dir prevents importing the same file twice.
//!
//! Fixture: `{ "runs": [{ "start", "count", "everySeconds"?, "appName",
//! "bundleId"?, "windowTitle"?, "browserUrl"?, "text"? }], "pauses":
//! [{ "startedAt", "endedAt", "reason" }] }`. Each run writes `count` frames
//! from `start`; frame text is `text` plus a per-frame line so every frame is
//! useful.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::activity::store::{ActivityStore, NewFrame, PauseReason, TextSource};

pub const DEBUG_IMPORT_ENV: &str = "CLOVY_ACTIVITY_DEBUG_IMPORT";
const MARKER_DIR: &str = "activity-debug-imports";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    #[serde(default)]
    runs: Vec<FixtureRun>,
    #[serde(default)]
    pauses: Vec<FixturePause>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureRun {
    start: DateTime<Utc>,
    count: u32,
    every_seconds: Option<u32>,
    app_name: String,
    bundle_id: Option<String>,
    window_title: Option<String>,
    browser_url: Option<String>,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixturePause {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    reason: PauseReason,
}

/// Imports `fixture` unless already imported; returns the frames written.
pub async fn import_file(
    store: &ActivityStore,
    fixture: &Path,
    data_dir: &Path,
) -> Result<usize, String> {
    let bytes = std::fs::read(fixture).map_err(|error| error.to_string())?;
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let marker = data_dir.join(MARKER_DIR).join(format!("{digest}.done"));
    if marker.exists() {
        return Ok(0);
    }
    let fixture: Fixture = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    let mut written = 0;
    for run in &fixture.runs {
        let step = Duration::seconds(i64::from(run.every_seconds.unwrap_or(2).max(1)));
        for index in 0..run.count {
            let base = run.text.as_deref().unwrap_or_default();
            store
                .insert_frame(&NewFrame {
                    captured_at: run.start + step * i32::try_from(index).unwrap_or(i32::MAX),
                    app_name: run.app_name.clone(),
                    bundle_id: run.bundle_id.clone(),
                    window_title: run.window_title.clone(),
                    browser_url: run.browser_url.clone(),
                    text_source: TextSource::Accessibility,
                    text: Some(format!("{base}\nframe {index}")),
                    input_idle_ms: None,
                })
                .await
                .map_err(|error| error.to_string())?;
            written += 1;
        }
    }
    for pause in &fixture.pauses {
        let id = store
            .open_pause(pause.reason, pause.started_at)
            .await
            .map_err(|error| error.to_string())?;
        store
            .close_pause(id, pause.ended_at)
            .await
            .map_err(|error| error.to_string())?;
    }
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&marker, b"").map_err(|error| error.to_string())?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::key::MemoryKeyStore;
    use crate::activity::store::ACTIVITY_DB_FILE;

    #[tokio::test]
    async fn a_fixture_is_imported_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = ActivityStore::open(
            &dir.path().join(ACTIVITY_DB_FILE),
            &MemoryKeyStore::default(),
        )
        .await
        .unwrap();
        let fixture = dir.path().join("fixture.json");
        std::fs::write(
            &fixture,
            r#"{ "runs": [{ "start": "2026-10-03T12:00:00Z", "count": 3, "appName": "Zed", "windowTitle": "os-clovy \u2014 a.rs", "text": "fn main" }],
                 "pauses": [{ "startedAt": "2026-10-03T13:00:00Z", "endedAt": "2026-10-03T13:20:00Z", "reason": "manual" }] }"#,
        )
        .unwrap();
        assert_eq!(import_file(&store, &fixture, dir.path()).await.unwrap(), 3);
        assert_eq!(import_file(&store, &fixture, dir.path()).await.unwrap(), 0);
        let frames = store.frames_after(0, 10).await.unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[2].captured_at, "2026-10-03T12:00:04.000000Z");
        let start = "2026-10-03T00:00:00Z".parse().unwrap();
        let end = "2026-10-04T00:00:00Z".parse().unwrap();
        assert_eq!(store.pauses_between(start, end).await.unwrap().len(), 1);
    }
}
