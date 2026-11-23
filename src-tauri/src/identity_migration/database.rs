//! Databases travel as single-file snapshots, and every path into the old
//! profile is rewritten to the new one in the staged copy.

use std::path::{Path, PathBuf};

use sqlx::connection::{ConnectOptions, Connection};
use sqlx::row::Row;
use sqlx_sqlite::{SqliteConnectOptions, SqliteConnection};

use super::{fail, FileCopier, MigrationError};

const SQLITE_HEADER: &[u8; 16] = b"SQLite format 3\0";
/// The one encrypted database; its key comes from the migrated credentials.
pub(super) const ENCRYPTED_DATABASE: &str = "activity.sqlite3";
const COMPANION_SUFFIXES: [&str; 3] = ["-wal", "-shm", "-journal"];

pub(super) fn is_plain_sqlite(path: &Path) -> bool {
    use std::io::Read;
    let mut header = [0_u8; 16];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .is_ok_and(|()| &header == SQLITE_HEADER)
}

/// `notes.sqlite3-wal` -> `notes.sqlite3`.
pub(super) fn companion_of(name: &str) -> Option<&str> {
    COMPANION_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
}

/// Runs database work on its own thread and runtime: the migration runs
/// before Tauri's runtime exists, and tests call it from inside one.
fn block_on<T: Send + 'static>(
    work: impl std::future::Future<Output = Result<T, MigrationError>> + Send + 'static,
) -> Result<T, MigrationError> {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| fail("starting the database runtime", e))?
            .block_on(work)
    })
    .join()
    .map_err(|_| MigrationError::new("the database snapshot thread panicked"))?
}

fn options(path: &Path, key: Option<&str>) -> SqliteConnectOptions {
    let options = SqliteConnectOptions::new().filename(path);
    match key {
        // The raw-key form ActivityStore uses.
        Some(key) => options.pragma("key", format!("\"x'{key}'\"")),
        None => options,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Snapshot {
    Written,
    /// The key does not open it: copied byte for byte like any other file.
    NotADatabase,
}

/// Writes a consistent single-file snapshot of `from` (and its WAL) to `to`.
///
/// The source is never opened: SQLite would create or checkpoint its WAL and
/// change the old profile. The database and its companions are copied to a
/// private scratch folder, the copy is checked against a source that did not
/// change meanwhile, and `VACUUM INTO` writes the snapshot from there.
pub(super) fn snapshot(
    from: &Path,
    to: &Path,
    key: Option<&str>,
    scratch: &Path,
    copier: &mut dyn FileCopier,
) -> Result<Snapshot, MigrationError> {
    let name = from
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let members: Vec<(PathBuf, PathBuf)> = std::iter::once(name.clone())
        .chain(
            COMPANION_SUFFIXES
                .iter()
                .map(|suffix| format!("{name}{suffix}")),
        )
        .map(|member| (from.with_file_name(&member), scratch.join(&member)))
        .filter(|(source, _)| source.is_file())
        .collect();
    std::fs::create_dir_all(scratch)
        .map_err(|e| fail(format!("creating {}", scratch.display()), e))?;
    let before = stamps(&members)?;
    for (source, copy) in &members {
        copier
            .copy_file(source, copy)
            .map_err(|e| fail(format!("copying {}", source.display()), e))?;
    }
    if stamps(&members)? != before {
        return Err(MigrationError::new(format!(
            "{} changed while it was being copied; quit every app that uses it and reopen Clovy",
            from.display()
        )));
    }
    let scratch_db = scratch.join(&name);
    let target = to.to_path_buf();
    let key = key.map(str::to_owned);
    let written = block_on(async move {
        let mut connection = match options(&scratch_db, key.as_deref()).connect().await {
            Ok(connection) => connection,
            Err(error) if key.is_some() && not_a_database(&error) => {
                return Ok(Snapshot::NotADatabase)
            }
            Err(error) => return Err(fail(format!("opening {}", scratch_db.display()), error)),
        };
        if let Err(error) = sqlx::query::query("SELECT count(*) FROM sqlite_master")
            .fetch_one(&mut connection)
            .await
        {
            if key.is_some() && not_a_database(&error) {
                return Ok(Snapshot::NotADatabase);
            }
            return Err(fail(format!("reading {}", scratch_db.display()), error));
        }
        sqlx::query::query("VACUUM INTO ?")
            .bind(target.to_string_lossy().into_owned())
            .execute(&mut connection)
            .await
            .map_err(|e| fail(format!("snapshotting into {}", target.display()), e))?;
        connection
            .close()
            .await
            .map_err(|e| fail(format!("closing {}", scratch_db.display()), e))?;
        // The snapshot must be whole, and an encrypted one must stay so.
        if key.is_some() && is_plain_sqlite(&target) {
            return Err(MigrationError::new(format!(
                "the snapshot of {} was written without encryption",
                target.display()
            )));
        }
        let mut copy = options(&target, key.as_deref())
            .connect()
            .await
            .map_err(|e| fail(format!("opening {}", target.display()), e))?;
        let check: String = sqlx::query::query("PRAGMA quick_check")
            .fetch_one(&mut copy)
            .await
            .map_err(|e| fail(format!("checking {}", target.display()), e))?
            .get(0);
        copy.close()
            .await
            .map_err(|e| fail(format!("closing {}", target.display()), e))?;
        if check != "ok" {
            return Err(MigrationError::new(format!(
                "the snapshot of {} failed its integrity check: {check}",
                target.display()
            )));
        }
        Ok(Snapshot::Written)
    });
    let _ = std::fs::remove_dir_all(scratch);
    let written = written?;
    if written == Snapshot::NotADatabase {
        let _ = std::fs::remove_file(to);
    }
    Ok(written)
}

fn not_a_database(error: &sqlx::Error) -> bool {
    error.to_string().contains("file is not a database")
}

type Stamp = (PathBuf, u64, Option<std::time::SystemTime>);

fn stamps(members: &[(PathBuf, PathBuf)]) -> Result<Vec<Stamp>, MigrationError> {
    members
        .iter()
        .map(|(source, _)| {
            let metadata = std::fs::metadata(source)
                .map_err(|e| fail(format!("reading {}", source.display()), e))?;
            Ok((source.clone(), metadata.len(), metadata.modified().ok()))
        })
        .collect()
}

/// Where the app stores absolute paths into its own profile, and only there:
/// notes, transcripts, chat messages, captured activity and any other text
/// that merely mentions the old folder is the user's content and stays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    /// The whole value is a path.
    Path,
    /// JSON whose path-valued keys (`PATH_KEYS`) hold paths.
    PathsInJson,
    /// A JSON array of command arguments, any of which may be a path.
    Arguments,
}

/// `(database file, table, column, kind)`.
const PATH_FIELDS: &[(&str, &str, &str, Field)] = &[
    (
        "notes.sqlite3",
        "recording_sessions",
        "partial_path",
        Field::Path,
    ),
    (
        "notes.sqlite3",
        "recording_sessions",
        "final_path",
        Field::Path,
    ),
    ("notes.sqlite3", "audio_artifacts", "path", Field::Path),
    (
        "notes.sqlite3",
        "audio_artifacts",
        "partial_path",
        Field::Path,
    ),
    (
        "notes.sqlite3",
        "agent_sessions",
        "workspace_path",
        Field::Path,
    ),
    ("notes.sqlite3", "agent_artifacts", "path", Field::Path),
    (
        "notes.sqlite3",
        "agent_artifacts",
        "original_path",
        Field::Path,
    ),
    (
        "notes.sqlite3",
        "agent_migration_manifests",
        "source_path",
        Field::Path,
    ),
    ("notes.sqlite3", "agent_mcp_servers", "command", Field::Path),
    (
        "notes.sqlite3",
        "agent_mcp_servers",
        "args_json",
        Field::Arguments,
    ),
    (
        "notes.sqlite3",
        "agent_items",
        "payload_json",
        Field::PathsInJson,
    ),
    (
        "notes.sqlite3",
        "agent_runs",
        "run_config_json",
        Field::PathsInJson,
    ),
    (
        "notes.sqlite3",
        "routines",
        "metadata_json",
        Field::PathsInJson,
    ),
    (
        ENCRYPTED_DATABASE,
        "coding_agent_blocks",
        "cwd",
        Field::Path,
    ),
];

/// JSON keys whose string values (or string array items) are paths: file
/// attachments, artifacts, workspaces, captures, legacy routine scripts.
const PATH_KEYS: &[&str] = &[
    "path",
    "partialPath",
    "sourcePath",
    "originalPath",
    "workspace",
    "workspacePath",
    "capture_path",
    "capturePath",
    "cwd",
    "attachments",
    "legacyScriptStoredPath",
];

/// WebKit localStorage keys whose JSON value holds profile paths.
const LOCAL_STORAGE_KEYS: &[&str] = &["clovy.agent.queuedFollowUps"];

/// How a profile file holds its paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathFile {
    /// The whole content is a path.
    Whole,
    /// A JSON object whose listed top-level keys are paths.
    JsonKeys(&'static [&'static str]),
    /// The legacy Hermes routines: each job's `script` and `workdir`.
    HermesJobs,
    /// The legacy Hermes MCP servers: each server's command and arguments.
    HermesMcpServers,
}

/// Profile files that hold paths into the profile, by their place in it.
pub(super) const PATH_FILES: &[(&str, PathFile)] = &[
    ("extension-host-path", PathFile::Whole),
    ("obsidian.json", PathFile::JsonKeys(&["vaultPath"])),
    ("hermes/cron/jobs.json", PathFile::HermesJobs),
    ("hermes/config.yaml", PathFile::HermesMcpServers),
];

const NOTES_DATABASE: &str = "notes.sqlite3";
/// Why a paused run stopped: its serialized SDK state nests the old paths
/// where they cannot be rewritten, so no run stays resumable after the
/// migration.
const RETIRED_RUN_MESSAGE: &str = "Clovy moved to its new identity before this run finished.";

/// The old profile roots and their new equivalents.
#[derive(Debug, Clone, Default)]
pub(super) struct PathRewrite {
    /// `(old root, new root)`, longest first.
    roots: Vec<(String, String)>,
}

impl PathRewrite {
    pub(super) fn new(folders: &[(PathBuf, PathBuf)]) -> Self {
        let mut roots = Vec::new();
        for (old, new) in folders {
            roots.push((
                old.to_string_lossy().into_owned(),
                new.to_string_lossy().into_owned(),
            ));
            // The app stores canonical paths (`/private/var` for `/var`).
            if let (Ok(old_canonical), Some(parent), Some(name)) =
                (old.canonicalize(), new.parent(), new.file_name())
            {
                if let Ok(parent) = parent.canonicalize() {
                    roots.push((
                        old_canonical.to_string_lossy().into_owned(),
                        parent.join(name).to_string_lossy().into_owned(),
                    ));
                }
            }
        }
        roots.sort_by_key(|(old, _): &(String, String)| std::cmp::Reverse(old.len()));
        roots.dedup();
        Self { roots }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }

    /// A path value into the old profile, moved to the new one: the root
    /// itself, or the root followed by a directory separator. Anything else,
    /// a sibling like `<root> (backup)` included, is not inside it.
    pub(super) fn path(&self, value: &str) -> Option<String> {
        self.roots.iter().find_map(|(old, new)| {
            let rest = value.strip_prefix(old.as_str())?;
            let inside = rest.is_empty() || rest.starts_with('/');
            inside.then(|| format!("{new}{rest}"))
        })
    }

    fn field(&self, field: Field, value: &str) -> Option<String> {
        match field {
            Field::Path => self.path(value),
            Field::PathsInJson | Field::Arguments => {
                let mut json = serde_json::from_str::<serde_json::Value>(value).ok()?;
                let changed = if field == Field::Arguments {
                    match &mut json {
                        serde_json::Value::Array(items) => self.rewrite_strings(items),
                        _ => false,
                    }
                } else {
                    self.rewrite_path_keys(&mut json)
                };
                changed.then(|| json.to_string())
            }
        }
    }

    fn rewrite_strings(&self, items: &mut [serde_json::Value]) -> bool {
        let mut changed = false;
        for item in items {
            match item {
                serde_json::Value::String(text) => {
                    if let Some(rewritten) = self.path(text) {
                        *text = rewritten;
                        changed = true;
                    }
                }
                other => changed |= self.rewrite_path_keys(other),
            }
        }
        changed
    }

    fn rewrite_path_keys(&self, value: &mut serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(fields) => {
                let mut changed = false;
                for (key, item) in fields.iter_mut() {
                    let is_path_key = PATH_KEYS.contains(&key.as_str());
                    changed |= match item {
                        serde_json::Value::String(text) if is_path_key => match self.path(text) {
                            Some(rewritten) => {
                                *text = rewritten;
                                true
                            }
                            None => false,
                        },
                        serde_json::Value::Array(items) if is_path_key => {
                            self.rewrite_strings(items)
                        }
                        other => self.rewrite_path_keys(other),
                    };
                }
                changed
            }
            serde_json::Value::Array(items) => items.iter_mut().fold(false, |changed, item| {
                self.rewrite_path_keys(item) | changed
            }),
            _ => false,
        }
    }

    /// Rewrites the listed path fields of one staged database, by its file
    /// name; any other database is left as it is.
    pub(super) fn apply_to_database(
        &self,
        path: &Path,
        key: Option<&str>,
    ) -> Result<u64, MigrationError> {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let fields: Vec<(&'static str, &'static str, Field)> = PATH_FIELDS
            .iter()
            .filter(|(database, ..)| *database == name)
            .map(|(_, table, column, field)| (*table, *column, *field))
            .collect();
        let local_storage = name == "localstorage.sqlite3";
        let notes = name == NOTES_DATABASE;
        if fields.is_empty() && !local_storage {
            return Ok(0);
        }
        let rewrite = self.clone();
        let path = path.to_path_buf();
        let key = key.map(str::to_owned);
        block_on(async move {
            let mut connection = options(&path, key.as_deref())
                .connect()
                .await
                .map_err(|e| fail(format!("opening {}", path.display()), e))?;
            let changed = async {
                let mut changed = rewrite.rewrite_fields(&mut connection, &fields).await?;
                if local_storage {
                    changed += rewrite.rewrite_local_storage(&mut connection).await?;
                }
                if notes {
                    changed += retire_paused_runs(&mut connection).await?;
                }
                Ok::<u64, sqlx::Error>(changed)
            }
            .await
            .map_err(|e| fail(format!("rewriting paths in {}", path.display()), e))?;
            connection
                .close()
                .await
                .map_err(|e| fail(format!("closing {}", path.display()), e))?;
            Ok(changed)
        })
    }

    async fn has_column(
        connection: &mut SqliteConnection,
        table: &str,
        column: &str,
    ) -> Result<bool, sqlx::Error> {
        Ok(
            sqlx::query::query("SELECT 1 FROM pragma_table_info(?) WHERE name = ?")
                .bind(table)
                .bind(column)
                .fetch_optional(&mut *connection)
                .await?
                .is_some(),
        )
    }

    async fn rewrite_fields(
        &self,
        connection: &mut SqliteConnection,
        fields: &[(&str, &str, Field)],
    ) -> Result<u64, sqlx::Error> {
        let mut changed = 0;
        for (table, column, field) in fields {
            if !Self::has_column(connection, table, column).await? {
                continue;
            }
            let (table, column) = (quote(table), quote(column));
            let filter = self
                .roots
                .iter()
                .map(|_| format!("instr({column}, ?) > 0"))
                .collect::<Vec<_>>()
                .join(" OR ");
            let sql = format!(
                "SELECT DISTINCT {column} FROM {table} WHERE typeof({column}) = 'text' AND ({filter})"
            );
            let mut select = sqlx::query::query(&sql);
            for (old, _) in &self.roots {
                select = select.bind(old.clone());
            }
            let values: Vec<String> = select
                .fetch_all(&mut *connection)
                .await?
                .iter()
                .map(|row| row.get(0))
                .collect();
            for value in values {
                if let Some(rewritten) = self.field(*field, &value) {
                    changed += sqlx::query::query(&format!(
                        "UPDATE {table} SET {column} = ? WHERE {column} = ?"
                    ))
                    .bind(rewritten)
                    .bind(value)
                    .execute(&mut *connection)
                    .await?
                    .rows_affected();
                }
            }
        }
        Ok(changed)
    }

    /// WebKit keeps localStorage values as UTF-16 little-endian blobs.
    async fn rewrite_local_storage(
        &self,
        connection: &mut SqliteConnection,
    ) -> Result<u64, sqlx::Error> {
        if !Self::has_column(connection, "ItemTable", "value").await? {
            return Ok(0);
        }
        let mut changed = 0;
        for key in LOCAL_STORAGE_KEYS {
            let Some(row) = sqlx::query::query(
                "SELECT value FROM ItemTable WHERE key = ? AND typeof(value) = 'blob'",
            )
            .bind(*key)
            .fetch_optional(&mut *connection)
            .await?
            else {
                continue;
            };
            let value: Vec<u8> = row.get(0);
            if value.len() % 2 != 0 {
                continue;
            }
            let units: Vec<u16> = value
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            let Ok(text) = String::from_utf16(&units) else {
                continue;
            };
            if let Some(rewritten) = self.field(Field::PathsInJson, &text) {
                let encoded: Vec<u8> = rewritten
                    .encode_utf16()
                    .flat_map(u16::to_le_bytes)
                    .collect();
                changed += sqlx::query::query("UPDATE ItemTable SET value = ? WHERE key = ?")
                    .bind(encoded)
                    .bind(*key)
                    .execute(&mut *connection)
                    .await?
                    .rows_affected();
            }
        }
        Ok(changed)
    }

    /// Rewrites one of `PATH_FILES`. A file its reader could not parse
    /// either is left as it is.
    pub(super) fn apply_to_path_file(
        &self,
        path: &Path,
        kind: PathFile,
    ) -> Result<bool, MigrationError> {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Ok(false);
        };
        let rewritten = match kind {
            PathFile::Whole => {
                let trimmed = text.trim_end();
                self.path(trimmed)
                    .map(|rewritten| format!("{rewritten}{}", &text[trimmed.len()..]))
            }
            PathFile::JsonKeys(keys) => self.json_file(&text, |json| self.json_keys(json, keys)),
            PathFile::HermesJobs => self.json_file(&text, |json| {
                let jobs = match json {
                    serde_json::Value::Object(root) => root
                        .get_mut("jobs")
                        .and_then(serde_json::Value::as_array_mut),
                    serde_json::Value::Array(jobs) => Some(jobs),
                    _ => None,
                };
                jobs.into_iter().flatten().fold(false, |changed, job| {
                    self.json_keys(job, &["script", "workdir"]) | changed
                })
            }),
            PathFile::HermesMcpServers => self.hermes_mcp_servers(&text),
        };
        let Some(rewritten) = rewritten else {
            return Ok(false);
        };
        std::fs::write(path, rewritten)
            .map_err(|e| fail(format!("writing {}", path.display()), e))?;
        Ok(true)
    }

    fn json_file(
        &self,
        text: &str,
        rewrite: impl FnOnce(&mut serde_json::Value) -> bool,
    ) -> Option<String> {
        let mut json = serde_json::from_str::<serde_json::Value>(text).ok()?;
        if !rewrite(&mut json) {
            return None;
        }
        serde_json::to_string_pretty(&json)
            .ok()
            .map(|json| format!("{json}\n"))
    }

    /// The string values of `keys` in a JSON object.
    fn json_keys(&self, value: &mut serde_json::Value, keys: &[&str]) -> bool {
        let Some(fields) = value.as_object_mut() else {
            return false;
        };
        let mut changed = false;
        for key in keys {
            if let Some(serde_json::Value::String(text)) = fields.get_mut(*key) {
                if let Some(rewritten) = self.path(text) {
                    *text = rewritten;
                    changed = true;
                }
            }
        }
        changed
    }

    /// `mcp_servers` (or `mcpServers`) of the legacy Hermes config, as the
    /// legacy import reads it: `command` or `cmd`, and each of `args`.
    fn hermes_mcp_servers(&self, text: &str) -> Option<String> {
        use serde_yaml::Value;
        let mut root = serde_yaml::from_str::<Value>(text).ok()?;
        let key = if root.get("mcp_servers").is_some() {
            "mcp_servers"
        } else {
            "mcpServers"
        };
        let servers = root.get_mut(key).and_then(Value::as_mapping_mut)?;
        let mut changed = false;
        let mut rewrite = |value: &mut Value| {
            if let Value::String(text) = value {
                if let Some(rewritten) = self.path(text) {
                    *text = rewritten;
                    changed = true;
                }
            }
        };
        for server in servers.values_mut().filter_map(Value::as_mapping_mut) {
            for key in ["command", "cmd"] {
                if let Some(value) = server.get_mut(key) {
                    rewrite(value);
                }
            }
            if let Some(Value::Sequence(args)) = server.get_mut("args") {
                args.iter_mut().for_each(&mut rewrite);
            }
        }
        if !changed {
            return None;
        }
        serde_yaml::to_string(&root).ok()
    }
}

/// Retires every paused agent run of the staged notes database, the way a
/// cancelled pause is retired: its pending cards expire, the run and its
/// session stop waiting, and no serialized SDK state is left to resume. The
/// routine projection follows the run at the next startup's reconciliation.
async fn retire_paused_runs(connection: &mut SqliteConnection) -> Result<u64, sqlx::Error> {
    for (table, column) in [
        ("agent_runs", "interrupted_state_json"),
        ("agent_items", "payload_json"),
        ("agent_sessions", "status"),
    ] {
        if !PathRewrite::has_column(connection, table, column).await? {
            return Ok(0);
        }
    }
    let now = crate::db::repositories::timestamp();
    let mut changed = sqlx::query::query(
        "UPDATE agent_items
         SET payload_json = json_set(payload_json, '$.status', 'expired', '$.resolvedAt', ?)
         WHERE kind = 'interruption'
           AND CASE WHEN json_valid(payload_json)
                    THEN json_extract(payload_json, '$.status') = 'pending'
                    ELSE 0 END",
    )
    .bind(&now)
    .execute(&mut *connection)
    .await?
    .rows_affected();
    changed += sqlx::query::query(
        "UPDATE agent_sessions SET status = 'interrupted', updated_at = ?, last_error = ?
         WHERE status = 'waiting_for_user'",
    )
    .bind(&now)
    .bind(RETIRED_RUN_MESSAGE)
    .execute(&mut *connection)
    .await?
    .rows_affected();
    changed += sqlx::query::query(
        "UPDATE agent_runs
         SET status = 'interrupted', updated_at = ?, completed_at = COALESCE(completed_at, ?),
             error_code = 'identity_migrated', error_message = ?
         WHERE status = 'waiting_for_user'",
    )
    .bind(&now)
    .bind(&now)
    .bind(RETIRED_RUN_MESSAGE)
    .execute(&mut *connection)
    .await?
    .rows_affected();
    changed += sqlx::query::query(
        "UPDATE agent_runs SET interrupted_state_json = NULL
         WHERE interrupted_state_json IS NOT NULL",
    )
    .execute(&mut *connection)
    .await?
    .rows_affected();
    Ok(changed)
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}
