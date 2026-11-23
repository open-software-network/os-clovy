//! The encrypted activity database, `activity.sqlite3`.
//!
//! A SQLite file separate from `notes.sqlite3`, encrypted with SQLCipher
//! under a random 32-byte key kept in the Keychain (`key.rs`). This module is
//! the only code that opens it; later slices (timeline, ingestion, summaries)
//! read and advance it through the API below. Schema, cursor, and retention
//! rules are documented in `docs/activity-capture.md`.
//!
//! Timestamps are RFC 3339 UTC with microseconds and a `Z` suffix, so string
//! comparison in SQL matches chronological order.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::query::query;
use sqlx::row::Row;
use sqlx_sqlite::{
    SqliteAutoVacuum, SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePool,
    SqlitePoolOptions, SqliteRow,
};
use zeroize::Zeroizing;

use super::key::{generate_key_hex, is_valid_key_hex, ActivityKeyStore, KeyStoreError};

pub const ACTIVITY_DB_FILE: &str = "activity.sqlite3";
/// The processing-cursor consumer the retention rule honors: the timeline ETL.
pub const TIMELINE_CONSUMER: &str = "timeline";
/// Pages reclaimed per retention sweep. Small enough that the sweep never
/// stalls a capture write for long.
const INCREMENTAL_VACUUM_PAGES: i64 = 500;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
/// Capture writes, the timeline ETL transaction, and timeline/agent reads can
/// overlap; SQLite still serializes the writers (WAL, busy timeout).
const MAX_CONNECTIONS: u32 = 4;
/// SQLite's primary result code for "file is not a database", which is how
/// SQLCipher reports a wrong key.
const SQLITE_NOTADB: &str = "26";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("the activity database exists but its key is missing from the Keychain")]
    KeyMissing,
    #[error("the activity database could not be decrypted with the stored key")]
    KeyRejected,
    #[error("this build has no SQLCipher; activity is never stored unencrypted")]
    CipherUnavailable,
    #[error("the activity database was written by a newer Clovy (schema {0})")]
    NewerSchema(i64),
    #[error("the activity database schema history does not match this build: {0}")]
    SchemaMismatch(String),
    #[error(transparent)]
    Keychain(#[from] KeyStoreError),
    #[error("activity database: {0}")]
    Sql(#[from] sqlx::Error),
    #[error("activity database file: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TextSource {
    Accessibility,
    Ocr,
    /// Neither path produced text (an empty window); app and title still count.
    None,
}

impl TextSource {
    pub fn as_str(self) -> &'static str {
        match self {
            TextSource::Accessibility => "accessibility",
            TextSource::Ocr => "ocr",
            TextSource::None => "none",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "accessibility" => TextSource::Accessibility,
            "ocr" => TextSource::Ocr,
            _ => TextSource::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputEventKind {
    Click,
    Key,
    AppSwitch,
    WindowFocus,
    Clipboard,
}

impl InputEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            InputEventKind::Click => "click",
            InputEventKind::Key => "key",
            InputEventKind::AppSwitch => "app_switch",
            InputEventKind::WindowFocus => "window_focus",
            InputEventKind::Clipboard => "clipboard",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "click" => InputEventKind::Click,
            "key" => InputEventKind::Key,
            "app_switch" => InputEventKind::AppSwitch,
            "window_focus" => InputEventKind::WindowFocus,
            "clipboard" => InputEventKind::Clipboard,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    Manual,
    WorkHours,
    LowDisk,
    ProtectedVideo,
}

impl PauseReason {
    pub fn as_str(self) -> &'static str {
        match self {
            PauseReason::Manual => "manual",
            PauseReason::WorkHours => "work_hours",
            PauseReason::LowDisk => "low_disk",
            PauseReason::ProtectedVideo => "protected_video",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "manual" => PauseReason::Manual,
            "work_hours" => PauseReason::WorkHours,
            "low_disk" => PauseReason::LowDisk,
            "protected_video" => PauseReason::ProtectedVideo,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewFrame {
    pub captured_at: DateTime<Utc>,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub text_source: TextSource,
    pub text: Option<String>,
    pub input_idle_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSecondaryFrame {
    pub captured_at: DateTime<Utc>,
    pub display_id: Option<u32>,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub text_source: TextSource,
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewInputEvent {
    pub occurred_at: DateTime<Utc>,
    pub kind: InputEventKind,
    pub app_name: Option<String>,
    /// Events of one kind coalesced within a capture tick.
    pub count: u32,
    /// Redacted clipboard text; `None` for every other kind (schema-enforced).
    pub clipboard_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredFrame {
    pub id: i64,
    pub captured_at: String,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub text_source: TextSource,
    pub text_id: Option<i64>,
    pub text: Option<String>,
    pub input_idle_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredSecondaryFrame {
    pub id: i64,
    pub captured_at: String,
    pub display_id: Option<i64>,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub text_source: TextSource,
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredInputEvent {
    pub id: i64,
    pub occurred_at: String,
    pub kind: InputEventKind,
    pub app_name: Option<String>,
    pub count: i64,
    pub clipboard_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredPause {
    pub id: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub reason: PauseReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingCursor {
    pub consumer: String,
    /// Highest `frames.id` the consumer has fully processed (0 = none).
    pub last_frame_id: i64,
    pub last_frame_at: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PruneReport {
    pub frames: u64,
    pub secondary_frames: u64,
    pub input_events: u64,
    pub pauses: u64,
    pub texts: u64,
    pub timeline_sessions: u64,
    pub timeline_gaps: u64,
    pub search_documents: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugExport {
    pub path: PathBuf,
    pub frames: usize,
    pub secondary_frames: usize,
    pub input_events: usize,
    pub pauses: usize,
    pub coding_agent_blocks: usize,
}

struct ActivityMigration {
    version: i64,
    name: &'static str,
    statements: &'static [&'static str],
}

/// Append-only, like the main catalog: never edit or reorder an entry; add a
/// new version instead.
const MIGRATIONS: &[ActivityMigration] = &[
    ActivityMigration {
    version: 1,
    name: "activity_capture",
    statements: &[
        // Deduplicated text bodies; frames point at them. Consecutive ticks of
        // an unchanged window share one row.
        "CREATE TABLE frame_texts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            hash TEXT NOT NULL UNIQUE,
            text TEXT NOT NULL
        )",
        // One row per 2 s tick of the focused window. AUTOINCREMENT so ids are
        // never reused after retention: the processing cursor relies on it.
        "CREATE TABLE frames (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at TEXT NOT NULL,
            app_name TEXT NOT NULL,
            bundle_id TEXT,
            window_title TEXT,
            browser_url TEXT,
            text_source TEXT NOT NULL CHECK (text_source IN ('accessibility', 'ocr', 'none')),
            text_id INTEGER REFERENCES frame_texts(id)
        )",
        "CREATE INDEX idx_frames_captured_at ON frames(captured_at)",
        "CREATE INDEX idx_frames_text_id ON frames(text_id)",
        // Context samples of other displays (~10 s). Never part of a session.
        "CREATE TABLE secondary_frames (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            captured_at TEXT NOT NULL,
            display_id INTEGER,
            app_name TEXT NOT NULL,
            bundle_id TEXT,
            window_title TEXT,
            browser_url TEXT,
            text_source TEXT NOT NULL CHECK (text_source IN ('accessibility', 'ocr', 'none')),
            text_id INTEGER REFERENCES frame_texts(id)
        )",
        "CREATE INDEX idx_secondary_frames_captured_at ON secondary_frames(captured_at)",
        "CREATE INDEX idx_secondary_frames_text_id ON secondary_frames(text_id)",
        // Input without content: only clipboard rows may carry (redacted) text.
        "CREATE TABLE input_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            occurred_at TEXT NOT NULL,
            kind TEXT NOT NULL CHECK (kind IN ('click', 'key', 'app_switch', 'window_focus', 'clipboard')),
            app_name TEXT,
            count INTEGER NOT NULL DEFAULT 1 CHECK (count >= 1),
            clipboard_text TEXT,
            CHECK (kind = 'clipboard' OR clipboard_text IS NULL)
        )",
        "CREATE INDEX idx_input_events_occurred_at ON input_events(occurred_at)",
        // Capture pauses with their reason, for timeline gap classification.
        "CREATE TABLE pauses (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            reason TEXT NOT NULL CHECK (reason IN ('manual', 'work_hours', 'low_disk', 'protected_video'))
        )",
        "CREATE INDEX idx_pauses_started_at ON pauses(started_at)",
        // How far each consumer (the timeline ETL) has processed frames.
        // Retention never deletes past it.
        "CREATE TABLE processing_cursor (
            consumer TEXT PRIMARY KEY,
            last_frame_id INTEGER NOT NULL DEFAULT 0,
            last_frame_at TEXT,
            updated_at TEXT NOT NULL
        )",
    ],
    },
    ActivityMigration {
        version: 2,
        name: "activity_timeline",
        statements: &[
            // Sessions the timeline ETL derives from frames (`activity::timeline`).
            // At most one row is `active`: the session still being extended.
            "CREATE TABLE timeline_sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                app_name TEXT NOT NULL,
                bundle_id TEXT,
                context_kind TEXT CHECK (context_kind IN ('domain', 'workspace')),
                context TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                duration_ms INTEGER NOT NULL CHECK (duration_ms >= 0),
                first_frame_id INTEGER NOT NULL,
                last_frame_id INTEGER NOT NULL,
                frame_count INTEGER NOT NULL,
                idle_frame_count INTEGER NOT NULL DEFAULT 0,
                status TEXT NOT NULL CHECK (status IN ('active', 'closed')),
                category TEXT NOT NULL CHECK (category IN ('coding', 'code_review', 'meeting',
                    'communication', 'design', 'documentation', 'planning', 'deployment_devops',
                    'research', 'idle_personal')),
                confidence REAL NOT NULL DEFAULT 0,
                meeting_audio INTEGER NOT NULL DEFAULT 0,
                -- rowid range of the session's documents in timeline_search
                -- (inserted while it is open, so contiguous).
                search_rowid_first INTEGER,
                search_rowid_last INTEGER,
                CHECK ((context_kind IS NULL) = (context IS NULL))
            )",
            "CREATE INDEX idx_timeline_sessions_started_at ON timeline_sessions(started_at)",
            "CREATE INDEX idx_timeline_sessions_ended_at ON timeline_sessions(ended_at)",
            "CREATE UNIQUE INDEX idx_timeline_sessions_one_active ON timeline_sessions(status)
                WHERE status = 'active'",
            // Window titles and URLs seen in a session, for its detail view.
            "CREATE TABLE timeline_session_windows (
                session_id INTEGER NOT NULL REFERENCES timeline_sessions(id) ON DELETE CASCADE,
                window_title TEXT NOT NULL DEFAULT '',
                browser_url TEXT NOT NULL DEFAULT '',
                first_seen_at TEXT NOT NULL,
                last_seen_at TEXT NOT NULL,
                frame_count INTEGER NOT NULL,
                PRIMARY KEY (session_id, window_title, browser_url)
            )",
            // Intervals over 5 minutes without useful frames; never inside a session.
            "CREATE TABLE timeline_gaps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                duration_ms INTEGER NOT NULL CHECK (duration_ms > 0),
                kind TEXT NOT NULL CHECK (kind IN ('idle', 'sleep', 'paused')),
                pause_reason TEXT CHECK (pause_reason IN ('manual', 'work_hours', 'low_disk',
                    'protected_video')),
                CHECK ((kind = 'paused') = (pause_reason IS NOT NULL))
            )",
            "CREATE INDEX idx_timeline_gaps_started_at ON timeline_gaps(started_at)",
            "CREATE INDEX idx_timeline_gaps_ended_at ON timeline_gaps(ended_at)",
            // The ETL's resumable state (open session, last frame, open search
            // document), written in the same transaction as the cursor.
            "CREATE TABLE timeline_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                state_json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            // Full-text search over what was on screen: one document per window
            // segment (same title and URL, at most 5 minutes) of a session.
            "CREATE VIRTUAL TABLE timeline_search USING fts5(
                window_title,
                browser_url,
                body,
                session_id UNINDEXED,
                seen_at UNINDEXED,
                tokenize = 'unicode61 remove_diacritics 2'
            )",
        ],
    },
    // Coding-agent blocks (`crate::coding_agents`): one row per block of a
    // session read from a local agent's transcript, keyed by its start.
    ActivityMigration {
        version: 3,
        name: "coding_agent_blocks",
        statements: &[
            "CREATE TABLE coding_agent_blocks (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL CHECK (source IN ('claude_code', 'codex', 'copilot_cli', 'copilot_vscode', 'cursor', 'cursor_cli', 'antigravity')),
                session_id TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                cwd TEXT,
                project TEXT,
                title TEXT,
                first_prompt TEXT,
                prompt_count INTEGER NOT NULL DEFAULT 0,
                reply_count INTEGER NOT NULL DEFAULT 0,
                active_seconds INTEGER NOT NULL DEFAULT 0,
                transcript TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('live', 'sealed', 'summarized')),
                sealed_at TEXT,
                summary TEXT,
                summary_source TEXT,
                summarized_at TEXT,
                summary_attempts INTEGER NOT NULL DEFAULT 0,
                summary_error TEXT,
                next_attempt_at TEXT,
                updated_at TEXT NOT NULL,
                UNIQUE (source, session_id, started_at),
                CHECK (state = 'live' OR sealed_at IS NOT NULL),
                CHECK (state <> 'summarized' OR summary IS NOT NULL)
            )",
            "CREATE INDEX idx_coding_agent_blocks_started_at ON coding_agent_blocks(started_at)",
            "CREATE INDEX idx_coding_agent_blocks_ended_at ON coding_agent_blocks(ended_at)",
            "CREATE INDEX idx_coding_agent_blocks_state ON coding_agent_blocks(state)",
        ],
    },
    // Day intelligence (`crate::day_intelligence`, docs/day-intelligence.md):
    // hour reports, the day's workstreams, day summaries, and the scheduler's
    // attempt bookkeeping. Days and hours are local ("YYYY-MM-DD",
    // "YYYY-MM-DDTHH"); `started_at`/`ended_at` are the UTC bounds.
    ActivityMigration {
        version: 4,
        name: "day_intelligence",
        statements: &[
            "CREATE TABLE day_hour_reports (
                hour TEXT PRIMARY KEY,
                day TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT NOT NULL,
                active_minutes INTEGER NOT NULL CHECK (active_minutes >= 0),
                summary TEXT NOT NULL,
                activities_json TEXT NOT NULL,
                distilled TEXT NOT NULL,
                distill_json TEXT NOT NULL,
                locale TEXT NOT NULL,
                provider TEXT NOT NULL,
                generated_at TEXT NOT NULL,
                -- set once the hour is part of the day's workstreams
                folded_at TEXT
            )",
            "CREATE INDEX idx_day_hour_reports_day ON day_hour_reports(day)",
            // Workstreams only grow: a later hour appends a row to
            // day_workstream_hours (and may refresh the summary of the one
            // workstream it joins); titles and other workstreams never change.
            "CREATE TABLE day_workstreams (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                day TEXT NOT NULL,
                title TEXT NOT NULL,
                summary TEXT NOT NULL,
                first_hour TEXT NOT NULL,
                last_hour TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
            "CREATE INDEX idx_day_workstreams_day ON day_workstreams(day)",
            "CREATE TABLE day_workstream_hours (
                workstream_id INTEGER NOT NULL REFERENCES day_workstreams(id) ON DELETE CASCADE,
                hour TEXT NOT NULL,
                minutes INTEGER NOT NULL CHECK (minutes >= 0),
                note TEXT NOT NULL,
                PRIMARY KEY (workstream_id, hour)
            )",
            "CREATE TABLE day_summaries (
                day TEXT PRIMARY KEY,
                headline TEXT NOT NULL,
                narrative TEXT NOT NULL,
                insights_json TEXT NOT NULL,
                standup_json TEXT NOT NULL,
                hours_covered INTEGER NOT NULL,
                locale TEXT NOT NULL,
                provider TEXT NOT NULL,
                trigger TEXT NOT NULL CHECK (trigger IN ('scheduled', 'manual')),
                generated_at TEXT NOT NULL
            )",
            // One row per unit of scheduled work ('hour:<hour>', 'fold:<hour>',
            // 'auto:<day>'): retries back off and stop after a few failures.
            "CREATE TABLE day_intelligence_runs (
                key TEXT PRIMARY KEY,
                attempts INTEGER NOT NULL,
                outcome TEXT NOT NULL CHECK (outcome IN ('ok', 'failed', 'empty')),
                last_error TEXT,
                next_attempt_at TEXT,
                updated_at TEXT NOT NULL
            )",
        ],
    },
    // Where a workstream's hour was located: `{periods: [{startedAt, endedAt}],
    // locatedMinutes}` (UTC bounds of the measured sessions its activities
    // named, and the minutes those activities cover). NULL: only the hour is
    // known (rows written before this migration, or no activity named one).
    ActivityMigration {
        version: 5,
        name: "day_workstream_periods",
        statements: &["ALTER TABLE day_workstream_hours ADD COLUMN periods_json TEXT"],
    },
    ActivityMigration {
        version: 6,
        name: "frame_input_idle",
        statements: &["ALTER TABLE frames ADD COLUMN input_idle_ms INTEGER NULL"],
    },
];

pub fn timestamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Micros, true)
}

/// Reads an RFC 3339 timestamp (any offset, surrounding whitespace ignored)
/// as UTC: the stored [`timestamp`] format and the ones agents write.
pub fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

fn text_hash(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_not_a_database(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(database) => {
            database.code().as_deref() == Some(SQLITE_NOTADB)
                || database.message().contains("file is not a database")
        }
        _ => false,
    }
}

fn classify(error: sqlx::Error) -> StoreError {
    if is_not_a_database(&error) {
        StoreError::KeyRejected
    } else {
        StoreError::Sql(error)
    }
}

/// Paths SQLite may leave next to the database (WAL mode).
fn database_files(path: &Path) -> [PathBuf; 3] {
    let wal = PathBuf::from(format!("{}-wal", path.display()));
    let shm = PathBuf::from(format!("{}-shm", path.display()));
    [path.to_path_buf(), wal, shm]
}

/// Fails unless the linked SQLite is SQLCipher. Checked on an in-memory
/// connection before the file is touched, so a build without SQLCipher can
/// never create a plaintext activity file.
async fn ensure_cipher_available() -> Result<(), StoreError> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")?;
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    let version = query("PRAGMA cipher_version")
        .fetch_optional(&pool)
        .await?
        .map(|row| row.get::<String, _>(0));
    crate::db::close_sqlite_pool(&pool).await;
    match version {
        Some(version) if !version.trim().is_empty() => Ok(()),
        _ => Err(StoreError::CipherUnavailable),
    }
}

#[derive(Clone)]
pub struct ActivityStore {
    pool: SqlitePool,
}

impl ActivityStore {
    /// Opens (or creates) the database. A missing key with an existing file
    /// is `KeyMissing`: nothing is deleted and no new key is minted, so the
    /// user decides through `recreate`.
    pub async fn open(path: &Path, keys: &dyn ActivityKeyStore) -> Result<Self, StoreError> {
        ensure_cipher_available().await?;
        let key = match keys.load()? {
            Some(key) if is_valid_key_hex(&key) => key,
            Some(_) => return Err(StoreError::KeyRejected),
            None if path.exists() => return Err(StoreError::KeyMissing),
            None => {
                let key = generate_key_hex();
                keys.store(&key)?;
                key
            }
        };
        Self::connect(path, &key).await
    }

    /// User-confirmed reset after a lost key: deletes the unreadable file and
    /// its WAL, drops the old key, and starts an empty database.
    pub async fn recreate(path: &Path, keys: &dyn ActivityKeyStore) -> Result<Self, StoreError> {
        ensure_cipher_available().await?;
        for file in database_files(path) {
            match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        keys.delete()?;
        Self::open(path, keys).await
    }

    async fn connect(path: &Path, key_hex: &str) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Raw-key form: SQLCipher uses the 32 bytes directly, skipping KDF.
        let key_pragma = Zeroizing::new(format!("\"x'{key_hex}'\""));
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .pragma("key", key_pragma.as_str().to_owned())
            .auto_vacuum(SqliteAutoVacuum::Incremental)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(BUSY_TIMEOUT);
        let pool = SqlitePoolOptions::new()
            .max_connections(MAX_CONNECTIONS)
            .connect_with(options)
            .await
            .map_err(classify)?;
        if let Err(error) = migrate(&pool).await {
            crate::db::close_sqlite_pool(&pool).await;
            return Err(error);
        }
        Ok(Self { pool })
    }

    pub async fn close(&self) {
        crate::db::close_sqlite_pool(&self.pool).await;
    }

    /// For the slices that keep their own tables in this database: the
    /// timeline (`timeline::db`), coding agents (`crate::coding_agents::store`),
    /// and day intelligence (`crate::day_intelligence::db`).
    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn insert_frame(&self, frame: &NewFrame) -> Result<i64, StoreError> {
        let mut tx = self.pool.begin().await?;
        let text_id = upsert_text(&mut tx, frame.text.as_deref()).await?;
        let id = query(
            "INSERT INTO frames
                (captured_at, app_name, bundle_id, window_title, browser_url, text_source, text_id, input_idle_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(timestamp(frame.captured_at))
        .bind(&frame.app_name)
        .bind(&frame.bundle_id)
        .bind(&frame.window_title)
        .bind(&frame.browser_url)
        .bind(frame.text_source.as_str())
        .bind(text_id)
        .bind(frame.input_idle_ms.and_then(|idle| i64::try_from(idle).ok()))
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
        tx.commit().await?;
        Ok(id)
    }

    pub async fn insert_secondary_frame(
        &self,
        frame: &NewSecondaryFrame,
    ) -> Result<i64, StoreError> {
        let mut tx = self.pool.begin().await?;
        let text_id = upsert_text(&mut tx, frame.text.as_deref()).await?;
        let id = query(
            "INSERT INTO secondary_frames
                (captured_at, display_id, app_name, bundle_id, window_title, browser_url, text_source, text_id)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(timestamp(frame.captured_at))
        .bind(frame.display_id.map(i64::from))
        .bind(&frame.app_name)
        .bind(&frame.bundle_id)
        .bind(&frame.window_title)
        .bind(&frame.browser_url)
        .bind(frame.text_source.as_str())
        .bind(text_id)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid();
        tx.commit().await?;
        Ok(id)
    }

    pub async fn insert_input_events(&self, events: &[NewInputEvent]) -> Result<(), StoreError> {
        if events.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for event in events {
            query(
                "INSERT INTO input_events (occurred_at, kind, app_name, count, clipboard_text)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(timestamp(event.occurred_at))
            .bind(event.kind.as_str())
            .bind(&event.app_name)
            .bind(i64::from(event.count.max(1)))
            .bind(&event.clipboard_text)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn open_pause(
        &self,
        reason: PauseReason,
        at: DateTime<Utc>,
    ) -> Result<i64, StoreError> {
        Ok(
            query("INSERT INTO pauses (started_at, reason) VALUES (?, ?)")
                .bind(timestamp(at))
                .bind(reason.as_str())
                .execute(&self.pool)
                .await?
                .last_insert_rowid(),
        )
    }

    pub async fn close_pause(&self, id: i64, at: DateTime<Utc>) -> Result<(), StoreError> {
        query("UPDATE pauses SET ended_at = ? WHERE id = ? AND ended_at IS NULL")
            .bind(timestamp(at))
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Closes pauses a previous process left open (crash, quit while paused)
    /// at the time this process starts capturing again.
    pub async fn close_open_pauses(&self, at: DateTime<Utc>) -> Result<(), StoreError> {
        query("UPDATE pauses SET ended_at = ? WHERE ended_at IS NULL")
            .bind(timestamp(at))
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn latest_frame_at(&self) -> Result<Option<String>, StoreError> {
        Ok(
            query("SELECT captured_at FROM frames ORDER BY id DESC LIMIT 1")
                .fetch_optional(&self.pool)
                .await?
                .map(|row| row.get(0)),
        )
    }

    /// Frames with `id > after_id`, oldest first: the timeline's read path.
    pub async fn frames_after(
        &self,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<StoredFrame>, StoreError> {
        let rows = query(
            "SELECT f.id, f.captured_at, f.app_name, f.bundle_id, f.window_title, f.browser_url,
                    f.text_source, f.text_id, t.text, f.input_idle_ms
             FROM frames f LEFT JOIN frame_texts t ON t.id = f.text_id
             WHERE f.id > ? ORDER BY f.id LIMIT ?",
        )
        .bind(after_id)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(stored_frame).collect())
    }

    pub async fn secondary_frames_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoredSecondaryFrame>, StoreError> {
        let rows = query(
            "SELECT s.id, s.captured_at, s.display_id, s.app_name, s.bundle_id, s.window_title,
                    s.browser_url, s.text_source, t.text
             FROM secondary_frames s LEFT JOIN frame_texts t ON t.id = s.text_id
             WHERE s.captured_at >= ? AND s.captured_at < ? ORDER BY s.id",
        )
        .bind(timestamp(from))
        .bind(timestamp(to))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().map(stored_secondary_frame).collect())
    }

    pub async fn input_events_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoredInputEvent>, StoreError> {
        let rows = query(
            "SELECT id, occurred_at, kind, app_name, count, clipboard_text FROM input_events
             WHERE occurred_at >= ? AND occurred_at < ? ORDER BY id",
        )
        .bind(timestamp(from))
        .bind(timestamp(to))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().filter_map(stored_input_event).collect())
    }

    /// Pauses overlapping `[from, to)`, including one still open.
    pub async fn pauses_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<StoredPause>, StoreError> {
        let rows = query(
            "SELECT id, started_at, ended_at, reason FROM pauses
             WHERE started_at < ? AND (ended_at IS NULL OR ended_at >= ?) ORDER BY id",
        )
        .bind(timestamp(to))
        .bind(timestamp(from))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.iter().filter_map(stored_pause).collect())
    }

    pub async fn processing_cursor(&self, consumer: &str) -> Result<ProcessingCursor, StoreError> {
        let row =
            query("SELECT last_frame_id, last_frame_at FROM processing_cursor WHERE consumer = ?")
                .bind(consumer)
                .fetch_optional(&self.pool)
                .await?;
        Ok(ProcessingCursor {
            consumer: consumer.to_string(),
            last_frame_id: row.as_ref().map_or(0, |row| row.get(0)),
            last_frame_at: row.and_then(|row| row.get(1)),
        })
    }

    /// Records that `consumer` processed every frame up to `last_frame_id`
    /// (captured at `last_frame_at`). Never moves backwards.
    pub async fn advance_processing_cursor(
        &self,
        consumer: &str,
        last_frame_id: i64,
        last_frame_at: DateTime<Utc>,
    ) -> Result<ProcessingCursor, StoreError> {
        let mut conn = self.pool.acquire().await?;
        advance_cursor_on(&mut conn, consumer, last_frame_id, last_frame_at).await?;
        drop(conn);
        self.processing_cursor(consumer).await
    }

    /// Deletes rows older than `retention_days` that the timeline already
    /// processed, then reclaims a bounded number of free pages. Frames are
    /// bounded by the cursor id; other tables by the cursor frame's time.
    pub async fn prune(
        &self,
        now: DateTime<Utc>,
        retention_days: u32,
    ) -> Result<PruneReport, StoreError> {
        let cursor = self.processing_cursor(TIMELINE_CONSUMER).await?;
        let Some(cursor_at) = cursor.last_frame_at.filter(|_| cursor.last_frame_id > 0) else {
            return Ok(PruneReport::default());
        };
        let cutoff = timestamp(now - chrono::Duration::days(i64::from(retention_days)));
        let bound = std::cmp::min(cutoff.clone(), cursor_at);

        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let frames = query("DELETE FROM frames WHERE id <= ? AND captured_at < ?")
            .bind(cursor.last_frame_id)
            .bind(&cutoff)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let secondary_frames = query("DELETE FROM secondary_frames WHERE captured_at < ?")
            .bind(&bound)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let input_events = query("DELETE FROM input_events WHERE occurred_at < ?")
            .bind(&bound)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let pauses = query("DELETE FROM pauses WHERE ended_at IS NOT NULL AND ended_at < ?")
            .bind(&bound)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let texts = query(
            "DELETE FROM frame_texts
             WHERE NOT EXISTS (SELECT 1 FROM frames WHERE frames.text_id = frame_texts.id)
               AND NOT EXISTS (SELECT 1 FROM secondary_frames s WHERE s.text_id = frame_texts.id)",
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        // Derived timeline rows follow the same period (search included).
        let timeline_sessions =
            query("DELETE FROM timeline_sessions WHERE status = 'closed' AND ended_at < ?")
                .bind(&cutoff)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        let timeline_gaps = query("DELETE FROM timeline_gaps WHERE ended_at < ?")
            .bind(&cutoff)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        let search_documents = query("DELETE FROM timeline_search WHERE seen_at < ?")
            .bind(&cutoff)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        tx.commit().await?;

        let report = PruneReport {
            frames,
            secondary_frames,
            input_events,
            pauses,
            texts,
            timeline_sessions,
            timeline_gaps,
            search_documents,
        };
        if report != PruneReport::default() {
            query(&format!(
                "PRAGMA incremental_vacuum({INCREMENTAL_VACUUM_PAGES})"
            ))
            .execute(&self.pool)
            .await?;
        }
        Ok(report)
    }

    /// Debug builds only (the caller enforces it): writes the newest rows to a
    /// readable JSON file in `dir`, plus the `extra` sections (the timeline's
    /// tables). The key is never part of the output.
    pub async fn debug_export(
        &self,
        dir: &Path,
        now: DateTime<Utc>,
        limit: u32,
        extra: serde_json::Map<String, serde_json::Value>,
    ) -> Result<DebugExport, StoreError> {
        let frames = {
            let mut rows = query(
                "SELECT f.id, f.captured_at, f.app_name, f.bundle_id, f.window_title, f.browser_url,
                        f.text_source, f.text_id, t.text, f.input_idle_ms
                 FROM frames f LEFT JOIN frame_texts t ON t.id = f.text_id
                 ORDER BY f.id DESC LIMIT ?",
            )
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await?
            .iter()
            .map(stored_frame)
            .collect::<Vec<_>>();
            rows.reverse();
            rows
        };
        let epoch = DateTime::<Utc>::UNIX_EPOCH;
        let far = now + chrono::Duration::days(1);
        let secondary_frames = self.secondary_frames_between(epoch, far).await?;
        let input_events = self.input_events_between(epoch, far).await?;
        let pauses = self.pauses_between(epoch, far).await?;
        let coding_agent_blocks = self.coding_agent_blocks_between(epoch, far).await?;
        let cursor = self.processing_cursor(TIMELINE_CONSUMER).await?;
        let schema_version: i64 = query("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_one(&self.pool)
            .await?
            .get(0);

        let mut body = serde_json::json!({
            "exportedAt": timestamp(now),
            "database": ACTIVITY_DB_FILE,
            "schemaVersion": schema_version,
            "processingCursor": cursor,
            "frames": frames,
            "secondaryFrames": secondary_frames,
            "inputEvents": input_events,
            "pauses": pauses,
            "codingAgentBlocks": coding_agent_blocks,
        });
        if let Some(object) = body.as_object_mut() {
            object.extend(extra);
        }
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!(
            "activity-debug-export-{}.json",
            now.format("%Y%m%dT%H%M%S%.3fZ")
        ));
        let serialized = serde_json::to_vec_pretty(&body)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(&path, serialized)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(DebugExport {
            path,
            frames: frames.len(),
            secondary_frames: secondary_frames.len(),
            input_events: input_events.len(),
            pauses: pauses.len(),
            coding_agent_blocks: coding_agent_blocks.len(),
        })
    }

    #[cfg(test)]
    async fn text_row_count(&self) -> Result<i64, StoreError> {
        Ok(query("SELECT count(*) FROM frame_texts")
            .fetch_one(&self.pool)
            .await?
            .get(0))
    }
}

async fn upsert_text(
    conn: &mut SqliteConnection,
    text: Option<&str>,
) -> Result<Option<i64>, StoreError> {
    let Some(text) = text.filter(|text| !text.trim().is_empty()) else {
        return Ok(None);
    };
    let hash = text_hash(text);
    query("INSERT INTO frame_texts (hash, text) VALUES (?, ?) ON CONFLICT(hash) DO NOTHING")
        .bind(&hash)
        .bind(text)
        .execute(&mut *conn)
        .await?;
    let id: i64 = query("SELECT id FROM frame_texts WHERE hash = ?")
        .bind(&hash)
        .fetch_one(&mut *conn)
        .await?
        .get(0);
    Ok(Some(id))
}

/// The cursor upsert, shared with the timeline ETL so it can advance the
/// cursor inside the transaction that writes the sessions. Never moves back.
pub(in crate::activity) async fn advance_cursor_on(
    conn: &mut SqliteConnection,
    consumer: &str,
    last_frame_id: i64,
    last_frame_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    query(
        "INSERT INTO processing_cursor (consumer, last_frame_id, last_frame_at, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(consumer) DO UPDATE SET
            last_frame_id = excluded.last_frame_id,
            last_frame_at = excluded.last_frame_at,
            updated_at = excluded.updated_at
         WHERE excluded.last_frame_id > processing_cursor.last_frame_id",
    )
    .bind(consumer)
    .bind(last_frame_id)
    .bind(timestamp(last_frame_at))
    .bind(timestamp(Utc::now()))
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn migrate(pool: &SqlitePool) -> Result<(), StoreError> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.map_err(classify)?;
    query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            name TEXT NOT NULL,
            applied_at TEXT NOT NULL
        )",
    )
    .execute(&mut *tx)
    .await
    .map_err(classify)?;
    let applied: Vec<(i64, String)> =
        query("SELECT version, name FROM schema_migrations ORDER BY version")
            .fetch_all(&mut *tx)
            .await?
            .iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect();
    for (index, (version, name)) in applied.iter().enumerate() {
        match MIGRATIONS.get(index) {
            Some(known) if known.version == *version && known.name == name => {}
            Some(known) => {
                return Err(StoreError::SchemaMismatch(format!(
                    "expected {} {}, found {version} {name}",
                    known.version, known.name
                )))
            }
            None => return Err(StoreError::NewerSchema(*version)),
        }
    }
    for migration in &MIGRATIONS[applied.len()..] {
        for statement in migration.statements {
            query(statement).execute(&mut *tx).await?;
        }
        query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)")
            .bind(migration.version)
            .bind(migration.name)
            .bind(timestamp(Utc::now()))
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}

fn stored_frame(row: &SqliteRow) -> StoredFrame {
    StoredFrame {
        id: row.get(0),
        captured_at: row.get(1),
        app_name: row.get(2),
        bundle_id: row.get(3),
        window_title: row.get(4),
        browser_url: row.get(5),
        text_source: TextSource::parse(row.get::<&str, _>(6)),
        text_id: row.get(7),
        text: row.get(8),
        input_idle_ms: row
            .get::<Option<i64>, _>(9)
            .and_then(|idle| u64::try_from(idle).ok()),
    }
}

fn stored_secondary_frame(row: &SqliteRow) -> StoredSecondaryFrame {
    StoredSecondaryFrame {
        id: row.get(0),
        captured_at: row.get(1),
        display_id: row.get(2),
        app_name: row.get(3),
        bundle_id: row.get(4),
        window_title: row.get(5),
        browser_url: row.get(6),
        text_source: TextSource::parse(row.get::<&str, _>(7)),
        text: row.get(8),
    }
}

fn stored_input_event(row: &SqliteRow) -> Option<StoredInputEvent> {
    Some(StoredInputEvent {
        id: row.get(0),
        occurred_at: row.get(1),
        kind: InputEventKind::parse(row.get::<&str, _>(2))?,
        app_name: row.get(3),
        count: row.get(4),
        clipboard_text: row.get(5),
    })
}

fn stored_pause(row: &SqliteRow) -> Option<StoredPause> {
    Some(StoredPause {
        id: row.get(0),
        started_at: row.get(1),
        ended_at: row.get(2),
        reason: PauseReason::parse(row.get::<&str, _>(3))?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::key::{ActivityKeyStore, MemoryKeyStore};
    use chrono::{Duration as ChronoDuration, TimeZone};

    fn temp_db() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(ACTIVITY_DB_FILE);
        (dir, path)
    }

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour, 0, 0).unwrap()
    }

    fn frame(at: DateTime<Utc>, text: &str) -> NewFrame {
        NewFrame {
            captured_at: at,
            app_name: "Zed".into(),
            bundle_id: Some("dev.zed.Zed".into()),
            window_title: Some("main.rs".into()),
            browser_url: None,
            text_source: TextSource::Accessibility,
            text: Some(text.into()),
            input_idle_ms: None,
        }
    }

    #[tokio::test]
    async fn migration_6_adds_nullable_idle_time_and_preserves_old_frames() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        store.insert_frame(&frame(at(1, 10), "kept")).await.unwrap();
        // Reconstruct the released v5 shape without rewriting catalog entries.
        let columns = query("PRAGMA table_info(frames)")
            .fetch_all(store.pool())
            .await
            .unwrap();
        if columns
            .iter()
            .any(|row| row.get::<&str, _>("name") == "input_idle_ms")
        {
            query("ALTER TABLE frames DROP COLUMN input_idle_ms")
                .execute(store.pool())
                .await
                .unwrap();
        }
        query("DELETE FROM schema_migrations WHERE version >= 6")
            .execute(store.pool())
            .await
            .unwrap();
        store.close().await;
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        let idle: Option<i64> = query("SELECT input_idle_ms FROM frames")
            .fetch_one(store.pool())
            .await
            .expect("migration 6 adds the idle column")
            .get(0);
        assert_eq!(idle, None, "old frames have no invented presence signal");
        let versions: Vec<i64> = query("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(store.pool())
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(versions, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(
            store.frames_after(0, 10).await.unwrap()[0].text.as_deref(),
            Some("kept")
        );
    }

    #[tokio::test]
    async fn missing_key_with_existing_file_refuses_to_open_and_keeps_the_file() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        ActivityStore::open(&path, &keys)
            .await
            .unwrap()
            .close()
            .await;
        keys.delete().unwrap();

        let error = ActivityStore::open(&path, &keys)
            .await
            .err()
            .expect("must fail");
        assert!(matches!(error, StoreError::KeyMissing));
        assert!(path.exists(), "nothing may be deleted automatically");
        assert!(
            keys.current().is_none(),
            "no new key may be minted over old data"
        );
    }

    #[tokio::test]
    async fn recreate_replaces_the_unreadable_database_with_a_fresh_key() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        store.insert_frame(&frame(at(1, 10), "old")).await.unwrap();
        store.close().await;
        keys.delete().unwrap();

        let store = ActivityStore::recreate(&path, &keys)
            .await
            .expect("recreate");
        assert!(keys.current().is_some());
        assert!(store.frames_after(0, 10).await.unwrap().is_empty());
    }

    #[cfg(target_os = "windows")]
    #[tokio::test]
    async fn windows_recreate_waits_for_a_connection_returning_during_close() {
        use std::future::{poll_fn, Future};
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        use std::task::Poll;
        use tokio::sync::Notify;

        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let key = generate_key_hex();
        keys.store(&key).unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let returning = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .pragma("key", format!("\"x'{}'\"", key.as_str()));
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .after_release({
                let armed = armed.clone();
                let returning = returning.clone();
                let resume = resume.clone();
                move |_, _| {
                    let armed = armed.clone();
                    let returning = returning.clone();
                    let resume = resume.clone();
                    Box::pin(async move {
                        if armed.swap(false, Ordering::AcqRel) {
                            returning.notify_one();
                            resume.notified().await;
                        }
                        Ok(true)
                    })
                }
            })
            .connect_with(options)
            .await
            .unwrap();
        migrate(&pool).await.unwrap();
        let store = ActivityStore { pool };
        store.insert_frame(&frame(at(1, 10), "old")).await.unwrap();
        let mut connection = store.pool.acquire().await.unwrap();
        let mut idle = store.pool.acquire().await.unwrap();
        idle.return_to_pool().await;
        drop(idle);
        armed.store(true, Ordering::Release);
        let return_task = tokio::spawn(connection.return_to_pool());
        drop(connection);
        tokio::time::timeout(Duration::from_secs(5), returning.notified())
            .await
            .expect("connection must enter its asynchronous return");
        // Drain the idle connection while another is paused in its return.
        // SQLx 0.8 can finish its close future before that return completes.
        {
            let draining = store.pool.close();
            tokio::pin!(draining);
            tokio::time::timeout(
                Duration::from_secs(5),
                poll_fn(|cx| {
                    let _ = draining.as_mut().poll(cx);
                    if store.pool.size() == 1 {
                        Poll::Ready(())
                    } else {
                        Poll::Pending
                    }
                }),
            )
            .await
            .expect("idle connection must finish closing");
        }
        let closing = store.close();
        tokio::pin!(closing);
        let closed_early = poll_fn(|cx| Poll::Ready(closing.as_mut().poll(cx).is_ready())).await;
        resume.notify_one();
        if !closed_early {
            tokio::time::timeout(Duration::from_secs(5), closing)
                .await
                .expect("close must finish after its returning connection");
        }
        return_task.await.unwrap();
        // Clean up even on RED, without retrying recreation.
        store.pool.close().await;
        assert!(
            !closed_early,
            "close returned while a SQLite connection was still returning"
        );
        keys.delete().unwrap();

        let recreated = ActivityStore::recreate(&path, &keys).await;
        let recreated = recreated.expect("closed database must be replaceable on Windows");
        assert!(recreated.frames_after(0, 10).await.unwrap().is_empty());
        recreated.close().await;
    }

    #[tokio::test]
    async fn timeline_database_upgrades_with_coding_agents_table_as_migration_3() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        // A database written by the timeline build: migrations 1 and 2 only.
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        store.insert_frame(&frame(at(1, 10), "kept")).await.unwrap();
        for statement in [
            "DROP TABLE coding_agent_blocks",
            "DROP TABLE day_workstream_hours",
            "DROP TABLE day_workstreams",
            "DROP TABLE day_hour_reports",
            "DROP TABLE day_summaries",
            "DROP TABLE day_intelligence_runs",
            "ALTER TABLE frames DROP COLUMN input_idle_ms",
            "DELETE FROM schema_migrations WHERE version >= 3",
        ] {
            query(statement).execute(&store.pool).await.unwrap();
        }
        store.close().await;

        let store = ActivityStore::open(&path, &keys).await.expect("upgrade");
        let history: Vec<(i64, String)> =
            query("SELECT version, name FROM schema_migrations ORDER BY version")
                .fetch_all(&store.pool)
                .await
                .unwrap()
                .iter()
                .map(|row| (row.get(0), row.get(1)))
                .collect();
        assert_eq!(
            history,
            vec![
                (1, "activity_capture".to_string()),
                (2, "activity_timeline".to_string()),
                (3, "coding_agent_blocks".to_string()),
                (4, "day_intelligence".to_string()),
                (5, "day_workstream_periods".to_string()),
                (6, "frame_input_idle".to_string()),
            ]
        );
        assert!(store
            .coding_agent_blocks_between(at(1, 0), at(2, 0))
            .await
            .unwrap()
            .is_empty());
        assert_eq!(store.frames_after(0, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn coding_agents_database_upgrades_with_day_intelligence_as_migration_4() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        // A database written by the coding-agents build: migrations 1 to 3.
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        store.insert_frame(&frame(at(1, 10), "kept")).await.unwrap();
        for statement in [
            "DROP TABLE day_workstream_hours",
            "DROP TABLE day_workstreams",
            "DROP TABLE day_hour_reports",
            "DROP TABLE day_summaries",
            "DROP TABLE day_intelligence_runs",
            "ALTER TABLE frames DROP COLUMN input_idle_ms",
            "DELETE FROM schema_migrations WHERE version >= 4",
        ] {
            query(statement).execute(&store.pool).await.unwrap();
        }
        store.close().await;

        let store = ActivityStore::open(&path, &keys).await.expect("upgrade");
        let versions: Vec<i64> = query("SELECT version FROM schema_migrations ORDER BY version")
            .fetch_all(&store.pool)
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        assert_eq!(versions, vec![1, 2, 3, 4, 5, 6]);
        assert!(
            crate::day_intelligence::db::summary_of_day(&store, "2026-10-01")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(store.frames_after(0, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn day_intelligence_database_keeps_its_workstreams_known_by_hour_after_migration_5() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        // A database written by the day-intelligence build: migrations 1 to 4,
        // with a workstream hour that has only its minutes.
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        for statement in [
            "ALTER TABLE day_workstream_hours DROP COLUMN periods_json",
            "ALTER TABLE frames DROP COLUMN input_idle_ms",
            "DELETE FROM schema_migrations WHERE version >= 5",
            "INSERT INTO day_workstreams (id, day, title, summary, first_hour, last_hour, created_at, updated_at)
             VALUES (1, '2026-10-01', 'Login fix', 's', '2026-10-01T09', '2026-10-01T09', 'x', 'x')",
            "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note)
             VALUES (1, '2026-10-01T09', 35, 'Fixed the redirect')",
        ] {
            query(statement).execute(&store.pool).await.unwrap();
        }
        store.close().await;

        let store = ActivityStore::open(&path, &keys).await.expect("upgrade");
        let workstreams = crate::day_intelligence::db::workstreams_of_day(&store, "2026-10-01")
            .await
            .unwrap();
        assert_eq!(workstreams.len(), 1);
        assert_eq!(workstreams[0].minutes, 35);
        assert_eq!(workstreams[0].hours[0].periods, None);
        assert_eq!(workstreams[0].hours[0].located_minutes, 0);
    }

    #[tokio::test]
    async fn wrong_key_is_reported_not_overwritten() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        ActivityStore::open(&path, &keys)
            .await
            .unwrap()
            .close()
            .await;
        keys.store(&crate::activity::key::generate_key_hex())
            .unwrap();

        let error = ActivityStore::open(&path, &keys)
            .await
            .err()
            .expect("must fail");
        assert!(matches!(error, StoreError::KeyRejected));
    }

    #[tokio::test]
    async fn identical_text_is_stored_once() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        store.insert_frame(&frame(at(1, 10), "same")).await.unwrap();
        store.insert_frame(&frame(at(1, 11), "same")).await.unwrap();
        let frames = store.frames_after(0, 10).await.unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].text.as_deref(), Some("same"));
        assert_eq!(frames[0].text_id, frames[1].text_id);
    }

    #[tokio::test]
    async fn retention_keeps_old_frames_the_timeline_has_not_processed() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        let old_a = store.insert_frame(&frame(at(1, 9), "a")).await.unwrap();
        let old_b = store.insert_frame(&frame(at(1, 10), "b")).await.unwrap();
        let recent = store.insert_frame(&frame(at(30, 10), "c")).await.unwrap();
        let now = at(30, 12);

        // Cursor at zero: nothing processed, nothing deleted even past 30 days.
        let report = store.prune(now, 7).await.unwrap();
        assert_eq!(report.frames, 0);
        assert_eq!(store.frames_after(0, 10).await.unwrap().len(), 3);

        // The timeline processed only the first old frame.
        store
            .advance_processing_cursor(TIMELINE_CONSUMER, old_a, at(1, 9))
            .await
            .unwrap();
        let report = store.prune(now, 7).await.unwrap();
        assert_eq!(report.frames, 1);
        let ids: Vec<i64> = store
            .frames_after(0, 10)
            .await
            .unwrap()
            .into_iter()
            .map(|frame| frame.id)
            .collect();
        assert_eq!(ids, vec![old_b, recent]);
        // The orphaned text row went with it; shared text is kept.
        assert_eq!(store.text_row_count().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn processing_cursor_only_moves_forward() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        let initial = store.processing_cursor(TIMELINE_CONSUMER).await.unwrap();
        assert_eq!(initial.last_frame_id, 0);
        store
            .advance_processing_cursor(TIMELINE_CONSUMER, 5, at(2, 10))
            .await
            .unwrap();
        let after = store
            .advance_processing_cursor(TIMELINE_CONSUMER, 3, at(2, 9))
            .await
            .unwrap();
        assert_eq!(after.last_frame_id, 5);
    }

    #[tokio::test]
    async fn pauses_open_and_close_with_reason() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        let id = store
            .open_pause(PauseReason::Manual, at(3, 10))
            .await
            .unwrap();
        store
            .close_pause(id, at(3, 10) + ChronoDuration::minutes(5))
            .await
            .unwrap();
        let pauses = store.pauses_between(at(3, 0), at(4, 0)).await.unwrap();
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].reason, PauseReason::Manual);
        assert!(pauses[0].ended_at.is_some());
    }

    #[tokio::test]
    async fn debug_export_contains_rows_but_never_the_key() {
        let (dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        store
            .insert_frame(&frame(at(5, 10), "exported text"))
            .await
            .unwrap();
        store
            .insert_input_events(&[NewInputEvent {
                occurred_at: at(5, 10),
                kind: InputEventKind::Clipboard,
                app_name: Some("Zed".into()),
                count: 1,
                clipboard_text: Some("[redacted]".into()),
            }])
            .await
            .unwrap();
        let export = store
            .debug_export(dir.path(), at(5, 11), 500, serde_json::Map::new())
            .await
            .expect("export");
        assert_eq!(export.frames, 1);
        assert_eq!(export.input_events, 1);
        let body = std::fs::read_to_string(&export.path).unwrap();
        assert!(body.contains("exported text"));
        let key = keys.current().unwrap();
        assert!(!body.contains(&key));
        assert!(!body.to_lowercase().contains(&key.to_lowercase()));
    }

    #[tokio::test]
    async fn non_clipboard_events_cannot_carry_text() {
        let (_dir, path) = temp_db();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&path, &keys).await.unwrap();
        let result = store
            .insert_input_events(&[NewInputEvent {
                occurred_at: at(5, 10),
                kind: InputEventKind::Key,
                app_name: Some("Zed".into()),
                count: 1,
                clipboard_text: Some("typed".into()),
            }])
            .await;
        assert!(result.is_err(), "schema must reject content on key events");
    }
}
