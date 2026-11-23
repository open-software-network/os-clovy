//! Fixtures for the day intelligence tests: a real encrypted activity store
//! with frames turned into timeline sessions, a fake activity provider that
//! answers by prompt kind and checks each answer against the request schema,
//! and a fixed time zone (UTC-3).

use std::sync::Mutex;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use futures_util::future::BoxFuture;
use serde_json::Value;

use super::embedder::NoEmbedder;
use super::pipeline::{Deps, Generator};
use super::prompts;
use super::schedule::FixedZone;
use super::sources::tests::FixedSources;
use super::sources::{CodingBlock, MeetingNote};
use crate::activity::key::MemoryKeyStore;
use crate::activity::settings::ActivitySettings;
use crate::activity::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};
use crate::activity::timeline::etl::run_pass;
use crate::interface_locale::UiLocale;
use crate::llm::{GenerateOutput, GenerateRequest, LlmError};

pub fn zone() -> FixedZone {
    FixedZone(chrono::FixedOffset::west_opt(3 * 3600).unwrap())
}

pub fn day() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 10, 4).unwrap()
}

/// A local time of 2026-10-04 (UTC-3) as an instant.
pub fn local(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 4, hour + 3, minute, 0)
        .unwrap()
}

pub fn settings() -> ActivitySettings {
    ActivitySettings {
        enabled: true,
        retention_days: 365,
        ..ActivitySettings::default()
    }
}

pub async fn store() -> (tempfile::TempDir, ActivityStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ActivityStore::open(
        &dir.path().join(ACTIVITY_DB_FILE),
        &MemoryKeyStore::default(),
    )
    .await
    .unwrap();
    (dir, store)
}

/// One frame per minute from `start` through `start + minutes` of `app`,
/// each with new text (so every frame is useful): a session of `minutes`.
pub async fn work(
    store: &ActivityStore,
    start: DateTime<Utc>,
    minutes: i64,
    app: &str,
    title: &str,
    lines: &[&str],
) {
    for minute in 0..=minutes {
        let mut text =
            format!("Step {minute} of the work in this window, carried on from before\n");
        text.push_str(&lines.join("\n"));
        store
            .insert_frame(&NewFrame {
                captured_at: start + Duration::minutes(minute),
                app_name: app.into(),
                bundle_id: None,
                window_title: Some(title.into()),
                browser_url: None,
                text_source: TextSource::Accessibility,
                text: Some(text),
                input_idle_ms: None,
            })
            .await
            .unwrap();
    }
}

pub async fn build_timeline(store: &ActivityStore, now: DateTime<Utc>) {
    run_pass(store, now, &[]).await.unwrap();
}

type Answer = Box<dyn Fn(&GenerateRequest) -> Result<Value, LlmError> + Send + Sync>;

/// Answers each call with `answer(request)`; records every request.
pub struct FakeProvider {
    pub calls: Mutex<Vec<GenerateRequest>>,
    answer: Answer,
}

impl FakeProvider {
    pub fn new(
        answer: impl Fn(&GenerateRequest) -> Result<Value, LlmError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            answer: Box::new(answer),
        }
    }

    pub fn calls(&self) -> Vec<GenerateRequest> {
        super::lock(&self.calls).clone()
    }

    pub fn calls_of(&self, base: &str) -> Vec<GenerateRequest> {
        self.calls()
            .into_iter()
            .filter(|call| {
                call.system
                    .as_deref()
                    .is_some_and(|system| system.starts_with(base))
            })
            .collect()
    }
}

impl Generator for FakeProvider {
    fn generate(
        &self,
        request: GenerateRequest,
    ) -> BoxFuture<'_, Result<GenerateOutput, LlmError>> {
        let result = (self.answer)(&request);
        if let (Ok(answer), Some(schema)) = (&result, &request.schema) {
            assert!(
                crate::llm::matches_schema(answer, schema),
                "fake answer off schema: {answer}"
            );
        }
        super::lock(&self.calls).push(request);
        Box::pin(async move {
            result.map(|json| GenerateOutput {
                text: json.to_string(),
                json: Some(json),
                provider: "cli:claude".into(),
                latency: StdDuration::ZERO,
            })
        })
    }
}

pub fn kind(request: &GenerateRequest) -> &'static str {
    let system = request.system.as_deref().unwrap_or_default();
    if system.starts_with(prompts::HOUR_REPORT) {
        "hour"
    } else if system.starts_with(prompts::WORKSTREAM_FOLD) {
        "fold"
    } else {
        "summary"
    }
}

pub fn meeting() -> MeetingNote {
    MeetingNote {
        note_id: "note-1".into(),
        title: "Weekly sync".into(),
        started_at: local(15, 0).to_rfc3339(),
        ended_at: local(15, 30).to_rfc3339(),
        start_local: "15:00".into(),
        end_local: "15:30".into(),
        duration_ms: 30 * 60_000,
        recorded: true,
        excerpt: Some("Decided to ship the beta on Friday".into()),
    }
}

pub fn coding_block() -> CodingBlock {
    CodingBlock {
        source: "claude_code".into(),
        project: Some("os-clovy".into()),
        title: Some("Fix login redirect".into()),
        started_at: local(14, 5).to_rfc3339(),
        ended_at: local(14, 35).to_rfc3339(),
        start_local: "14:05".into(),
        end_local: "14:35".into(),
        active_seconds: 1500,
        summary: Some("Patched the redirect loop and added a test".into()),
    }
}

pub fn sources() -> FixedSources {
    FixedSources {
        meetings: vec![meeting()],
        blocks: vec![coding_block()],
        ..FixedSources::default()
    }
}

pub fn deps<'a>(
    store: &'a ActivityStore,
    settings: &'a ActivitySettings,
    generator: &'a dyn Generator,
    sources: &'a FixedSources,
    zone: &'a FixedZone,
    locale: UiLocale,
    now: DateTime<Utc>,
) -> Deps<'a> {
    Deps {
        store,
        settings,
        generator,
        embedder: &NoEmbedder,
        sources,
        zone,
        locale,
        now,
    }
}
