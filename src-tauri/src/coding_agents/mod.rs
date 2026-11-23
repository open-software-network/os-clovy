//! Coding-agent session ingestion: brings the work done with local coding
//! agents (what screen capture cannot see) into the encrypted activity
//! database as summarized blocks. See `docs/coding-agent-sessions.md`.
//!
//! Pipeline, per scan (every minute, or right after the settings change):
//! `sources` read each enabled agent's transcripts read-only → `record`
//! normalizes them → `ingest` drops Clovy's own CLI calls and `segment` cuts
//! blocks (idle over 1 h, or 1 h long cut at a prompt) → `store` upserts them
//! (`live` → `sealed`) → `summarize` writes summaries with the agent's own CLI
//! or the activity provider (`sealed` → `summarized`).

pub mod ingest;
pub mod record;
#[cfg(test)]
#[path = "red_tests.rs"]
mod red_tests;
pub mod segment;
pub mod settings;
pub mod sources;
pub mod store;
pub mod summarize;

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Notify;

use crate::activity::engine::ActivityShared;
use crate::day_intelligence::schedule::{self, LocalZone, SystemZone};
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;
use sources::SourceRoots;
use store::CodingAgentBlock;

/// Emitted (no payload) when blocks or summaries changed.
pub const CODING_AGENTS_UPDATED_EVENT: &str = "clovy://coding-agents-updated";
const SCAN_INTERVAL: Duration = Duration::from_secs(60);
/// While a source is on but the activity database is still opening.
const STORE_RETRY: Duration = Duration::from_secs(3);
/// Next drain right away when more summaries are due.
const DRAIN_AGAIN: Duration = Duration::from_secs(1);
const PRUNE_EVERY: Duration = Duration::from_secs(60 * 60);
/// History read on first sight: today and yesterday (local days).
pub const BACKFILL_DAYS: u64 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceId {
    ClaudeCode,
    Codex,
    CopilotCli,
    CopilotVscode,
    Cursor,
    CursorCli,
    Antigravity,
}

impl SourceId {
    pub const ALL: [SourceId; 7] = [
        SourceId::ClaudeCode,
        SourceId::Codex,
        SourceId::CopilotCli,
        SourceId::CopilotVscode,
        SourceId::Cursor,
        SourceId::CursorCli,
        SourceId::Antigravity,
    ];

    /// Stored identifier (the `source` column).
    pub fn as_str(self) -> &'static str {
        match self {
            SourceId::ClaudeCode => "claude_code",
            SourceId::Codex => "codex",
            SourceId::CopilotCli => "copilot_cli",
            SourceId::CopilotVscode => "copilot_vscode",
            SourceId::Cursor => "cursor",
            SourceId::CursorCli => "cursor_cli",
            SourceId::Antigravity => "antigravity",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|source| source.as_str() == value)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            SourceId::ClaudeCode => "Claude Code",
            SourceId::Codex => "Codex",
            SourceId::CopilotCli => "Copilot CLI",
            SourceId::CopilotVscode => "Copilot in VS Code",
            SourceId::Cursor => "Cursor",
            SourceId::CursorCli => "Cursor CLI",
            SourceId::Antigravity => "Antigravity",
        }
    }

    /// The CLI that summarizes this agent's blocks.
    pub fn own_cli(self) -> CliKind {
        match self {
            SourceId::ClaudeCode => CliKind::Claude,
            SourceId::Codex => CliKind::Codex,
            SourceId::CopilotCli | SourceId::CopilotVscode => CliKind::Copilot,
            SourceId::Cursor | SourceId::CursorCli => CliKind::CursorAgent,
            SourceId::Antigravity => CliKind::Agy,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ScanStatus {
    last_scan_at: Option<String>,
    last_error: Option<String>,
}

/// Tauri-managed handle. Without activity support (non-macOS) or a home
/// directory, the loop never starts and the commands report `supported: false`.
#[derive(Default)]
pub struct CodingAgentsState {
    runtime: Option<Runtime>,
}

struct Runtime {
    activity: Arc<ActivityShared>,
    roots: SourceRoots,
    wake: Arc<Notify>,
    status: Arc<Mutex<ScanStatus>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceStatusDto {
    pub id: SourceId,
    pub name: &'static str,
    pub enabled: bool,
    /// The agent's data directory exists on this Mac.
    pub present: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentsStatusDto {
    pub supported: bool,
    pub sources: Vec<SourceStatusDto>,
    /// The activity database is open (blocks can be stored and read).
    pub database_ready: bool,
    pub last_scan_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentBlocksRequest {
    pub from: DateTime<Utc>,
    pub to: DateTime<Utc>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[tauri::command]
pub fn coding_agents_status(state: State<'_, CodingAgentsState>) -> CodingAgentsStatusDto {
    let Some(runtime) = state.runtime.as_ref() else {
        return CodingAgentsStatusDto {
            supported: false,
            sources: Vec::new(),
            database_ready: false,
            last_scan_at: None,
            last_error: None,
        };
    };
    let enabled = runtime.activity.settings().coding_agents;
    let status = lock(&runtime.status).clone();
    CodingAgentsStatusDto {
        supported: true,
        sources: SourceId::ALL
            .into_iter()
            .map(|source| SourceStatusDto {
                id: source,
                name: source.display_name(),
                enabled: enabled.is_enabled(source),
                present: runtime.roots.is_present(source),
            })
            .collect(),
        database_ready: runtime.activity.store().is_some(),
        last_scan_at: status.last_scan_at,
        last_error: status.last_error,
    }
}

/// Blocks overlapping `[from, to)` of the sources still turned on, oldest
/// first. Empty while the activity database is not open.
#[tauri::command]
pub async fn coding_agents_blocks(
    state: State<'_, CodingAgentsState>,
    request: CodingAgentBlocksRequest,
) -> Result<Vec<CodingAgentBlock>, AppError> {
    match state.runtime.as_ref() {
        Some(runtime) => blocks_of_enabled_sources(&runtime.activity, &request).await,
        None => Ok(Vec::new()),
    }
}

async fn blocks_of_enabled_sources(
    activity: &ActivityShared,
    request: &CodingAgentBlocksRequest,
) -> Result<Vec<CodingAgentBlock>, AppError> {
    let Some(store) = activity.store() else {
        return Ok(Vec::new());
    };
    store
        .enabled_coding_agent_blocks_between(
            &activity.settings().coding_agents,
            request.from,
            request.to,
        )
        .await
        .map_err(|error| AppError::new("coding_agents_read_failed", error.to_string()))
}

/// Runs a scan now (after the sources were changed in settings).
pub fn wake(app: &AppHandle) {
    if let Some(runtime) = app.state::<CodingAgentsState>().runtime.as_ref() {
        runtime.wake.notify_one();
    }
}

/// Registers the managed state and starts the scan loop. Must run after
/// `activity::setup`, whose database this module shares.
pub fn setup(app: &mut tauri::App) {
    let activity = app.state::<crate::activity::ActivityState>().shared();
    let runtime = activity
        .zip(SourceRoots::current())
        .map(|(activity, roots)| Runtime {
            activity,
            roots,
            wake: Arc::new(Notify::new()),
            status: Arc::new(Mutex::new(ScanStatus::default())),
        });
    if let Some(runtime) = &runtime {
        tauri::async_runtime::spawn(run(
            app.handle().clone(),
            Arc::clone(&runtime.activity),
            runtime.roots.clone(),
            Arc::clone(&runtime.wake),
            Arc::clone(&runtime.status),
        ));
    }
    app.manage(CodingAgentsState { runtime });
}

/// Local midnight `BACKFILL_DAYS - 1` days before `now`.
pub fn window_start(zone: &dyn LocalZone, now: DateTime<Utc>) -> DateTime<Utc> {
    schedule::local_midnight(zone, now, BACKFILL_DAYS - 1)
}

async fn run(
    app: AppHandle,
    activity: Arc<ActivityShared>,
    roots: SourceRoots,
    wake: Arc<Notify>,
    status: Arc<Mutex<ScanStatus>>,
) {
    let mut scanner = ingest::Scanner::new(roots);
    let mut last_prune: Option<Instant> = None;
    loop {
        let settings = activity.settings();
        let mut wait = SCAN_INTERVAL;
        if settings.coding_agents.any_enabled() {
            match activity.store() {
                None => wait = STORE_RETRY,
                Some(store) => {
                    let now = Utc::now();
                    let window = window_start(&SystemZone, now);
                    let mut changed = false;
                    match scanner
                        .scan(&store, &settings.coding_agents, now, window)
                        .await
                    {
                        Ok(report) => {
                            changed |= report.changed();
                            *lock(&status) = ScanStatus {
                                last_scan_at: Some(crate::activity::store::timestamp(now)),
                                last_error: None,
                            };
                        }
                        Err(error) => {
                            tracing::warn!(%error, "coding agents: scan failed");
                            lock(&status).last_error = Some(error.to_string());
                        }
                    }
                    // Summaries are written in the interface language: none
                    // before it is known (saved file or webview).
                    if let Some(locale) = crate::interface_locale::known() {
                        match summarize::drain(
                            &store,
                            &summarize::LlmBackend,
                            &settings.coding_agents,
                            locale,
                            window,
                            Utc::now,
                        )
                        .await
                        {
                            Ok(report) => {
                                changed |= report.summarized + report.failed > 0;
                                if report.more {
                                    wait = DRAIN_AGAIN;
                                }
                            }
                            Err(error) => {
                                tracing::warn!(%error, "coding agents: summaries failed")
                            }
                        }
                    }
                    if changed {
                        let _ = app.emit(CODING_AGENTS_UPDATED_EVENT, ());
                    }
                }
            }
        }
        // Stored transcripts follow the retention period whenever the
        // database is open, also with every source turned off.
        if let Some(store) = activity.store() {
            if !last_prune.is_some_and(|at| at.elapsed() < PRUNE_EVERY) {
                let before =
                    Utc::now() - chrono::Duration::days(i64::from(settings.retention_days));
                if let Err(error) = store.prune_coding_agent_blocks(before).await {
                    tracing::warn!(%error, "coding agents: retention failed");
                }
                last_prune = Some(Instant::now());
            }
        }
        tokio::select! {
            _ = wake.notified() => {}
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::day_intelligence::schedule::NamedZone;
    use chrono::TimeZone;
    use chrono_tz::America::{Santiago, Sao_Paulo};

    #[test]
    fn window_starts_at_local_midnight_of_yesterday() {
        let now = Sao_Paulo.with_ymd_and_hms(2026, 10, 4, 15, 30, 0).unwrap();
        let start = window_start(&NamedZone(Sao_Paulo), now.with_timezone(&Utc));
        assert_eq!(
            start,
            Sao_Paulo.with_ymd_and_hms(2026, 10, 3, 0, 0, 0).unwrap()
        );
    }

    /// Santiago skips midnight on 2026-09-06: the window starting that day
    /// starts at 01:00, not at "now".
    #[test]
    fn a_skipped_midnight_starts_the_window_at_the_first_minute_that_exists() {
        let now = Santiago.with_ymd_and_hms(2026, 9, 7, 15, 0, 0).unwrap();
        let start = window_start(&NamedZone(Santiago), now.with_timezone(&Utc));
        assert_eq!(
            start,
            Santiago.with_ymd_and_hms(2026, 9, 6, 1, 0, 0).unwrap()
        );
    }

    /// The Tauri command shows only the sources still turned on, like the
    /// MCP server and the day reports.
    #[tokio::test]
    async fn blocks_command_hides_sources_turned_off() {
        use crate::activity::key::MemoryKeyStore;
        use crate::activity::schedule::StoreStatus;
        use crate::activity::settings::ActivitySettings;
        use crate::activity::store::{ActivityStore, ACTIVITY_DB_FILE};
        use settings::CodingAgentSources;
        use store::NewBlock;

        let dir = tempfile::tempdir().unwrap();
        let store = ActivityStore::open(
            &dir.path().join(ACTIVITY_DB_FILE),
            &MemoryKeyStore::default(),
        )
        .await
        .unwrap();
        let start = Utc.with_ymd_and_hms(2026, 10, 4, 10, 0, 0).unwrap();
        for source in [SourceId::ClaudeCode, SourceId::Codex] {
            let block = NewBlock {
                source,
                session_id: source.as_str().into(),
                started_at: start,
                ended_at: start + chrono::Duration::minutes(30),
                cwd: None,
                project: None,
                title: None,
                first_prompt: Some("fix it".into()),
                prompt_count: 1,
                reply_count: 1,
                active_seconds: 60,
                transcript: "user: fix it".into(),
                sealed: true,
            };
            store
                .upsert_coding_agent_block(&block, start)
                .await
                .unwrap();
        }
        let activity = ActivityShared::new(ActivitySettings {
            coding_agents: CodingAgentSources {
                codex: true,
                ..CodingAgentSources::default()
            },
            ..ActivitySettings::default()
        });
        activity.replace_store(Some(store), StoreStatus::Ready);

        let request = CodingAgentBlocksRequest {
            from: start - chrono::Duration::hours(1),
            to: start + chrono::Duration::hours(1),
        };
        let blocks = blocks_of_enabled_sources(&activity, &request)
            .await
            .unwrap();
        let sources: Vec<SourceId> = blocks.iter().map(|block| block.source).collect();
        assert_eq!(sources, vec![SourceId::Codex]);
    }

    #[test]
    fn source_ids_round_trip() {
        for source in SourceId::ALL {
            assert_eq!(SourceId::parse(source.as_str()), Some(source));
            assert_eq!(
                serde_json::to_value(source).unwrap(),
                serde_json::Value::String(source.as_str().to_string())
            );
        }
    }
}
