use super::domain::{AgentMigrationManifestDto, MigrationCounts};
use crate::agent_mcp::{
    parse_legacy_mcp_config, KeychainMcpSecretStore, LegacyMcpImport, McpSecretBundle,
    McpSecretStore,
};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{query::query, row::Row};
use sqlx_sqlite::{
    SqliteConnectOptions, SqliteConnection, SqlitePool, SqlitePoolOptions, SqliteRow,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    time::Duration,
};
use thiserror::Error;

// Persisted migration keys are append-only wire values. Retaining the shipped
// key prevents an upgraded Clovy build (or a rollback) from importing the same
// Hermes state a second time.
const MIGRATION_KEY: &str = "hermes-to-june-agent-runtime-v2";
const HERMES_GATEWAY_LAUNCHD_LABEL: &str = "ai.hermes.gateway";
const LEGACY_GATEWAY_STOP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_LEGACY_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
const LEGACY_ROUTINE_REVIEW_ERROR: &str =
    "Legacy script or autonomous execution settings were preserved but require review before Clovy can run them.";

#[derive(Debug, Clone)]
pub struct LegacyImportOptions {
    pub hermes_state_db: PathBuf,
    pub hermes_home: Option<PathBuf>,
    pub artifact_root: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum LegacyImportError {
    #[error("legacy state database could not be read: {0}")]
    Source(#[source] sqlx::Error),
    #[error("Clovy agent database migration failed: {0}")]
    Destination(#[source] sqlx::Error),
    #[error("legacy state metadata could not be read: {0}")]
    Metadata(#[source] std::io::Error),
    #[error("legacy MCP credentials could not be moved to secure storage")]
    SecureStorage,
}

#[derive(Debug, Error)]
enum LegacyImportCommitError {
    #[error(transparent)]
    Destination(#[from] sqlx::Error),
    #[error("legacy MCP credentials could not be moved to secure storage")]
    SecureStorage,
}

/// Stops only gateway processes that are recorded inside Clovy's retired Hermes
/// home and whose live command line still identifies the Hermes gateway. Stale
/// state files and unrelated Hermes installations are ignored.
pub async fn stop_legacy_hermes_runtime(hermes_home: &Path, user_home: Option<&Path>) {
    for pid in legacy_gateway_pids(hermes_home) {
        if pid == std::process::id() || !live_gateway_matches(pid).await {
            continue;
        }
        stop_legacy_gateway_process(pid).await;
    }
    #[cfg(target_os = "macos")]
    if let Some(user_home) = user_home {
        stop_legacy_launch_agent(hermes_home, user_home).await;
    }
}

fn legacy_gateway_pids(hermes_home: &Path) -> BTreeSet<u32> {
    let mut state_paths = vec![hermes_home.join("gateway_state.json")];
    if let Ok(profiles) = fs::read_dir(hermes_home.join("profiles")) {
        state_paths.extend(
            profiles
                .flatten()
                .map(|entry| entry.path().join("gateway_state.json")),
        );
    }
    state_paths
        .into_iter()
        .filter_map(|path| fs::read(path).ok())
        .filter_map(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .filter_map(|state| legacy_gateway_pid_from_state(&state))
        .collect()
}

fn legacy_gateway_pid_from_state(state: &Value) -> Option<u32> {
    if state.get("kind").and_then(Value::as_str) != Some("hermes-gateway") {
        return None;
    }
    let argv = state.get("argv").and_then(Value::as_array)?;
    let command = argv
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if !legacy_gateway_command_matches(&command) {
        return None;
    }
    state
        .get("pid")
        .and_then(Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .filter(|pid| *pid > 1)
}

fn legacy_gateway_command_matches(command: &str) -> bool {
    let normalized = command.replace('\\', "/").to_ascii_lowercase();
    normalized.contains("resources/native/hermes/")
        && normalized.contains("hermes_cli")
        && normalized.contains("gateway run")
}

#[cfg(target_os = "macos")]
async fn live_gateway_matches(pid: u32) -> bool {
    crate::hide_console::tokio_command("/bin/ps")
        .args(["-ww", "-p", &pid.to_string(), "-o", "command="])
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|command| legacy_gateway_command_matches(&command))
}

#[cfg(target_os = "windows")]
async fn live_gateway_matches(pid: u32) -> bool {
    let script = format!("(Get-CimInstance Win32_Process -Filter 'ProcessId = {pid}').CommandLine");
    crate::hide_console::tokio_command("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .await
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|command| legacy_gateway_command_matches(&command))
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn live_gateway_matches(_pid: u32) -> bool {
    false
}

#[cfg(target_os = "macos")]
async fn stop_legacy_gateway_process(pid: u32) {
    let pid = pid as libc::pid_t;
    // SAFETY: the live command line and Clovy-owned state file were validated
    // immediately above. Signals target that exact positive pid only.
    let _ = unsafe { libc::kill(pid, libc::SIGTERM) };
    let exited = tokio::time::timeout(LEGACY_GATEWAY_STOP_TIMEOUT, async {
        loop {
            // SAFETY: signal 0 only probes whether the validated pid exists.
            if unsafe { libc::kill(pid, 0) } != 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .is_ok();
    if !exited {
        // SAFETY: this is the same validated pid after a bounded graceful stop.
        let _ = unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

#[cfg(target_os = "windows")]
async fn stop_legacy_gateway_process(pid: u32) {
    let _ = tokio::time::timeout(
        LEGACY_GATEWAY_STOP_TIMEOUT,
        crate::hide_console::tokio_command("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .status(),
    )
    .await;
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
async fn stop_legacy_gateway_process(_pid: u32) {}

#[cfg(target_os = "macos")]
async fn stop_legacy_launch_agent(hermes_home: &Path, user_home: &Path) {
    let plist = user_home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{HERMES_GATEWAY_LAUNCHD_LABEL}.plist"));
    let Ok(contents) = fs::read_to_string(&plist) else {
        return;
    };
    if !contents.contains(HERMES_GATEWAY_LAUNCHD_LABEL)
        || !contents.contains(hermes_home.to_string_lossy().as_ref())
    {
        return;
    }
    // SAFETY: getuid reads the current process's immutable real-user id.
    let uid = unsafe { libc::getuid() };
    for target in [
        format!("gui/{uid}/{HERMES_GATEWAY_LAUNCHD_LABEL}"),
        format!("user/{uid}/{HERMES_GATEWAY_LAUNCHD_LABEL}"),
    ] {
        let _ = tokio::time::timeout(
            LEGACY_GATEWAY_STOP_TIMEOUT,
            crate::hide_console::tokio_command("/bin/launchctl")
                .args(["bootout", &target])
                .status(),
        )
        .await;
    }
    let _ = fs::remove_file(plist);
}

#[derive(Debug)]
struct LegacySession {
    id: String,
    source: String,
    model: String,
    title: String,
    started_at: f64,
    ended_at: Option<f64>,
    end_reason: Option<String>,
}

fn read_legacy_jobs(path: &Path, errors: &mut Vec<String>) -> Vec<Value> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    match serde_json::from_str::<Value>(&raw) {
        Ok(Value::Object(root)) => root
            .get("jobs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        Ok(Value::Array(jobs)) => jobs,
        Ok(_) | Err(_) => {
            errors
                .push("Legacy routine definitions were unreadable and were left untouched.".into());
            Vec::new()
        }
    }
}

fn read_legacy_mcp(path: &Path, errors: &mut Vec<String>) -> LegacyMcpImport {
    let Ok(raw) = fs::read_to_string(path) else {
        return LegacyMcpImport {
            definitions: Vec::new(),
            secrets: Vec::new(),
        };
    };
    match parse_legacy_mcp_config(&raw) {
        Ok(mut imported) => {
            let retained_ids: BTreeSet<String> = imported
                .definitions
                .iter()
                .filter(|definition| !is_legacy_june_owned_mcp_server(&definition.name))
                .map(|definition| definition.id.clone())
                .collect();
            imported
                .definitions
                .retain(|definition| retained_ids.contains(&definition.id));
            imported.secrets.retain(|(secret_ref, _)| {
                retained_ids
                    .iter()
                    .any(|id| secret_ref == &format!("legacy-{id}"))
            });
            imported
        }
        Err(_) => {
            errors.push(
                "Legacy custom MCP definitions were unreadable and were left untouched.".into(),
            );
            LegacyMcpImport {
                definitions: Vec::new(),
                secrets: Vec::new(),
            }
        }
    }
}

fn is_legacy_june_owned_mcp_server(name: &str) -> bool {
    name.starts_with("june_")
        || matches!(
            name,
            "computer_use" | "june-computer-use" | "june_context" | "june_web"
        )
}

fn restore_staged_secrets(
    store: &impl McpSecretStore,
    staged: &[(String, Option<McpSecretBundle>)],
) {
    for (secret_ref, previous) in staged.iter().rev() {
        if let Some(previous) = previous {
            let _ = store.put(secret_ref, previous);
        } else {
            let _ = store.delete(secret_ref);
        }
    }
}

async fn insert_legacy_routine(
    transaction: &mut SqliteConnection,
    job: &Value,
    routine_storage_root: Option<&Path>,
    staged_artifacts: &mut Vec<PathBuf>,
    errors: &mut Vec<String>,
) -> Result<bool, sqlx::Error> {
    let Some(job) = job.as_object() else {
        return Ok(false);
    };
    let Some(id) = job
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    else {
        return Ok(false);
    };
    let prompt = job
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let name = job
        .get("name")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Imported routine");
    let Some(schedule) = legacy_schedule(job.get("schedule")) else {
        return Ok(false);
    };
    let timezone = job
        .get("timezone")
        .and_then(Value::as_str)
        .or_else(|| {
            job.get("schedule")
                .and_then(|schedule| schedule.get("timezone"))
                .and_then(Value::as_str)
        })
        .filter(|timezone| !timezone.trim().is_empty())
        .unwrap_or("UTC");
    let requested_enabled = job.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let requested_state = match job
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("scheduled")
    {
        "paused" => "paused",
        "completed" => "completed",
        "scheduled" => "scheduled",
        _ => "needs_review",
    };
    let created_at = job
        .get("created_at")
        .and_then(Value::as_str)
        .unwrap_or("1970-01-01T00:00:00.000Z");
    let updated_at = job
        .get("last_run_at")
        .and_then(Value::as_str)
        .unwrap_or(created_at);
    let toolsets = job
        .get("enabled_toolsets")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // Scripts were historically executed without the agent loop. Clovy's
    // unattended Routine harness deliberately has no shell tool or approval
    // UI, so scheduling one would silently run only its prompt. Preserve the
    // inline source in SQLite but fail closed until a user explicitly reviews
    // and recreates it against the Rust-owned shell/approval boundary.
    let legacy_script = job
        .get("script")
        .and_then(Value::as_str)
        .filter(|script| !script.trim().is_empty());
    let copied_script = if let (Some(root), Some(script)) = (routine_storage_root, legacy_script) {
        match copy_legacy_routine_script(root, id, script, job.get("workdir")).await {
            Ok(copied) => {
                if copied.as_ref().is_some_and(|copied| copied.created) {
                    staged_artifacts.push(
                        copied
                            .as_ref()
                            .expect("checked copied routine script")
                            .path
                            .clone(),
                    );
                }
                copied
            }
            Err(error) => {
                errors.push(format!(
                    "routine {id} script could not be copied into Clovy storage: {error}"
                ));
                None
            }
        }
    } else {
        None
    };
    let legacy_no_agent = job
        .get("no_agent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let legacy_unsupported_toolset = toolsets
        .iter()
        .filter_map(Value::as_str)
        .any(|toolset| matches!(toolset, "terminal" | "file" | "code_execution" | "skills"));
    let needs_execution_review =
        legacy_script.is_some() || legacy_no_agent || legacy_unsupported_toolset;
    let (state, enabled, next_run_at, safety_mode, last_error) = if needs_execution_review {
        // Do not preserve `unrestricted` as an active policy. It becomes a
        // reviewable legacy fact in metadata, while the stored Routine remains
        // disabled and cannot be claimed by the scheduler.
        (
            "needs_review",
            false,
            None,
            "sandboxed",
            Some(LEGACY_ROUTINE_REVIEW_ERROR),
        )
    } else {
        (
            requested_state,
            requested_enabled,
            job.get("next_run_at").and_then(Value::as_str),
            "sandboxed",
            job.get("last_error").and_then(Value::as_str),
        )
    };
    let repeat = job
        .get("repeat")
        .and_then(|value| {
            value
                .get("times")
                .and_then(Value::as_i64)
                .map(|times| times.to_string())
        })
        .unwrap_or_else(|| "forever".into());
    let script_storage = copied_script.as_ref().map_or_else(
        || {
            legacy_script.map_or(
                Value::Null,
                |script| json!({ "kind": "inline", "content": script }),
            )
        },
        |copied| json!({ "kind": "june_owned_file", "path": copied.path.to_string_lossy() }),
    );
    let metadata = json!({
        "importedFrom": "legacy_hermes",
        "enabledToolsets": toolsets,
        "legacyScriptPresent": legacy_script.is_some(),
        // The original inline program is copied into Clovy's SQLite row inside
        // the same transaction as the Routine. Nothing needs to read the
        // retired home after import to display or recover it.
        "legacyScript": legacy_script,
        "legacyScriptExecution": legacy_script.is_some().then_some("needs_review"),
        "legacyScriptStorage": script_storage,
        "legacyScriptStoredPath": copied_script
            .as_ref()
            .map(|copied| copied.path.to_string_lossy().into_owned()),
        "legacyNoAgent": legacy_no_agent,
        "legacyExecutionNeedsReview": needs_execution_review,
        "legacyRequestedState": requested_state,
        "legacyRequestedEnabled": requested_enabled,
        "legacySafetyMode": if needs_execution_review { "unrestricted" } else { "sandboxed" },
        "skills": job.get("skills").cloned().unwrap_or(Value::Null),
        "contextFrom": job.get("context_from").cloned().unwrap_or(Value::Null),
        "workdir": job.get("workdir").cloned().unwrap_or(Value::Null),
        "legacyTimezone": timezone,
        "origin": job.get("origin").cloned().unwrap_or(Value::Null),
        "repeatState": job.get("repeat").cloned().unwrap_or(Value::Null),
        "legacySchedule": job.get("schedule").cloned().unwrap_or(Value::Null),
        "legacyLastError": job.get("last_error").cloned().unwrap_or(Value::Null),
    });
    let inserted = query(
        "INSERT INTO routines
         (id, legacy_job_id, name, prompt, schedule, timezone, repeat, deliver, model,
          safety_mode, state, enabled, created_at, updated_at, next_run_at, last_run_at,
          last_status, last_error, last_delivery_error, metadata_json, tool_catalog_version)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1)
         ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(id)
    .bind(name)
    .bind(prompt)
    .bind(schedule)
    .bind(timezone)
    .bind(repeat)
    .bind(
        job.get("deliver")
            .and_then(Value::as_str)
            .unwrap_or("local"),
    )
    .bind(job.get("model").and_then(Value::as_str).unwrap_or("auto"))
    .bind(safety_mode)
    .bind(state)
    .bind(i64::from(enabled))
    .bind(created_at)
    .bind(updated_at)
    .bind(next_run_at)
    .bind(job.get("last_run_at").and_then(Value::as_str))
    .bind(job.get("last_status").and_then(Value::as_str))
    .bind(last_error)
    .bind(job.get("last_delivery_error").and_then(Value::as_str))
    .bind(metadata.to_string())
    .execute(&mut *transaction)
    .await?;
    Ok(inserted.rows_affected() == 1)
}

fn legacy_schedule(schedule: Option<&Value>) -> Option<String> {
    match schedule? {
        Value::String(schedule) => Some(schedule.clone()),
        Value::Object(schedule) => match schedule.get("kind").and_then(Value::as_str)? {
            "cron" => schedule.get("expr")?.as_str().map(str::to_string),
            "interval" => schedule
                .get("minutes")?
                .as_i64()
                .map(|minutes| format!("every {minutes}m")),
            "once" => schedule.get("run_at")?.as_str().map(str::to_string),
            _ => None,
        },
        _ => None,
    }
}

/// Copy a legacy script only when its command value identifies a regular file.
/// Inline shell programs remain transactionally stored in `metadata_json`.
/// Neither representation is executable until review: the copy is a recovery
/// artifact, not a shell input.
async fn copy_legacy_routine_script(
    root: &Path,
    routine_id: &str,
    script: &str,
    workdir: Option<&Value>,
) -> Result<Option<CopiedArtifact>, std::io::Error> {
    let direct = PathBuf::from(script);
    let source = if direct.is_absolute() {
        direct
    } else if !script.contains(char::is_whitespace) {
        workdir
            .and_then(Value::as_str)
            .map(PathBuf::from)
            .filter(|workdir| workdir.is_absolute())
            .map(|workdir| workdir.join(script))
            .unwrap_or(direct)
    } else {
        return Ok(None);
    };
    if !source.is_absolute() {
        return Ok(None);
    }
    let metadata = match tokio::fs::symlink_metadata(&source).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_LEGACY_ARTIFACT_BYTES
    {
        return Ok(None);
    }
    let source_key = deterministic_id("routine-script", source.to_string_lossy().as_ref());
    let destination_dir = root
        .join("routine-scripts")
        .join(safe_component(routine_id))
        .join(source_key);
    tokio::fs::create_dir_all(&destination_dir).await?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .map(safe_component)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| deterministic_id("script", source.to_string_lossy().as_ref()));
    let destination = destination_dir.join(name);
    let mut created = false;
    if !destination.exists() {
        tokio::fs::copy(&source, &destination).await?;
        created = true;
    }
    Ok(Some(CopiedArtifact {
        path: destination,
        created,
    }))
}

/// Imports the retired runtime's database without mutating it. All Clovy
/// conversation writes and the success manifest commit atomically. A failed
/// import rolls those writes back and records a separate failed manifest for
/// diagnostics; the source database and its WAL files are never changed.
pub async fn import_legacy_agent_state(
    destination: &SqlitePool,
    options: &LegacyImportOptions,
) -> Result<AgentMigrationManifestDto, LegacyImportError> {
    let source_path = options.hermes_state_db.as_path();
    // A completed import is a hard cutover boundary. Check Clovy's own
    // database before touching any path under the retired home, so future app
    // launches neither inspect nor depend on the old runtime data.
    if let Some(existing) = completed_manifest(destination).await? {
        return Ok(existing);
    }
    let hermes_home = options
        .hermes_home
        .clone()
        .or_else(|| source_path.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let jobs_path = hermes_home.join("cron").join("jobs.json");
    let config_path = hermes_home.join("config.yaml");
    if !source_path.exists() && !jobs_path.exists() && !config_path.exists() {
        return Ok(empty_manifest(source_path, "source_missing"));
    }
    let fingerprint = source_fingerprint(&[source_path, &jobs_path, &config_path])
        .map_err(LegacyImportError::Metadata)?;
    let source = if source_path.exists() {
        read_only_pool(source_path).await?
    } else {
        empty_legacy_source_pool().await?
    };
    let mut errors = Vec::new();
    let legacy_jobs = read_legacy_jobs(&jobs_path, &mut errors);
    let legacy_mcp = read_legacy_mcp(&config_path, &mut errors);
    let mut source_counts = source_counts(&source).await?;
    source_counts.routines = legacy_jobs.len() as u64;
    source_counts.mcp_servers = legacy_mcp.definitions.len() as u64;
    let started_at = crate::db::repositories::timestamp();
    let mut imported = MigrationCounts::default();
    let mut skipped_count = 0_u64;
    let completed_at = crate::db::repositories::timestamp();
    let secret_store = KeychainMcpSecretStore;
    let mut staged_secrets = Vec::new();
    let mut staged_artifacts = Vec::new();

    let result: Result<(), LegacyImportCommitError> = async {
        let sessions = load_sessions(&source).await?;
        let has_active = table_has_column(&source, "messages", "active").await?;
        let mut transaction = destination.begin().await?;

        for session in sessions {
            let session_created_at = timestamp(session.started_at);
            let session_completed_at = session.ended_at.map(timestamp);
            let session_updated_at = session_completed_at
                .clone()
                .unwrap_or_else(|| session_created_at.clone());
            let imported_source = if session.source == "cron" {
                "legacy_routine"
            } else {
                "legacy_hermes"
            };
            query(
                "INSERT INTO agent_sessions
                 (id, title, status, model, safety_mode, source, created_at, updated_at,
                  completed_at, last_error)
                 VALUES (?, ?, ?, ?, 'sandboxed', ?, ?, ?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                   title = excluded.title, status = excluded.status, model = excluded.model,
                   approval_mode = 'approvals',
                   source = excluded.source, created_at = excluded.created_at,
                   updated_at = excluded.updated_at, completed_at = excluded.completed_at,
                   last_error = excluded.last_error",
            )
            .bind(&session.id)
            .bind(&session.title)
            .bind(if session.ended_at.is_some() {
                "completed"
            } else {
                "idle"
            })
            .bind(&session.model)
            .bind(imported_source)
            .bind(&session_created_at)
            .bind(&session_updated_at)
            .bind(&session_completed_at)
            .bind(&session.end_reason)
            .execute(&mut *transaction)
            .await?;
            imported.sessions += 1;

            let rows = load_messages(&source, &session.id, has_active).await?;
            let mut sequence = next_sequence(&mut transaction, &session.id).await?;
            for row in rows {
                let message_id = row.try_get::<i64, _>("id").unwrap_or_default();
                let created_at = timestamp(row.try_get::<f64, _>("timestamp").unwrap_or_default());
                let role = row
                    .try_get::<String, _>("role")
                    .unwrap_or_else(|_| "user".to_string());
                let content = row.try_get::<Option<String>, _>("content").ok().flatten();
                let reasoning = first_nonempty(&row, &["reasoning", "reasoning_content"]);
                let tool_name = row.try_get::<Option<String>, _>("tool_name").ok().flatten();
                let tool_call_id = row
                    .try_get::<Option<String>, _>("tool_call_id")
                    .ok()
                    .flatten();
                let tool_calls = row
                    .try_get::<Option<String>, _>("tool_calls")
                    .ok()
                    .flatten();

                if let Some(text) = reasoning.filter(|value| !value.trim().is_empty()) {
                    if insert_item(
                        &mut transaction,
                        &session.id,
                        sequence,
                        "reasoning",
                        json!({ "text": text }),
                        &format!("hermes:{}:{}:reasoning", session.id, message_id),
                        &created_at,
                    )
                    .await?
                    {
                        imported.reasoning_items += 1;
                        sequence += 1;
                    } else {
                        skipped_count += 1;
                    }
                }

                if let Some(raw_calls) = tool_calls.filter(|value| !value.trim().is_empty()) {
                    let calls = parse_json_or_string(&raw_calls);
                    if insert_item(
                        &mut transaction,
                        &session.id,
                        sequence,
                        "tool_call",
                        json!({
                            "toolName": tool_name,
                            "toolCallId": tool_call_id,
                            "arguments": calls,
                            "result": null,
                            "status": "completed"
                        }),
                        &format!("hermes:{}:{}:tool-call", session.id, message_id),
                        &created_at,
                    )
                    .await?
                    {
                        imported.messages += 1;
                        sequence += 1;
                    } else {
                        skipped_count += 1;
                    }
                }

                if let Some(text) = content.filter(|value| !value.trim().is_empty()) {
                    let is_tool_result = role == "tool" || tool_name.is_some();
                    let (kind, payload) = if is_tool_result {
                        (
                            "tool_result",
                            json!({
                                "toolName": tool_name,
                                "toolCallId": tool_call_id,
                                "arguments": null,
                                "result": parse_json_or_string(&text),
                                "status": "completed"
                            }),
                        )
                    } else {
                        let kind = match role.as_str() {
                            "assistant" => "assistant_message",
                            "system" => "system_message",
                            _ => "user_message",
                        };
                        (kind, json!({ "role": role, "content": text }))
                    };
                    if insert_item(
                        &mut transaction,
                        &session.id,
                        sequence,
                        kind,
                        payload.clone(),
                        &format!("hermes:{}:{}:content", session.id, message_id),
                        &created_at,
                    )
                    .await?
                    {
                        imported.messages += 1;
                        sequence += 1;
                        if let Some(root) = options.artifact_root.as_deref() {
                            let candidates = artifact_candidates(&payload);
                            for candidate in candidates {
                                match copy_artifact(root, &session.id, &candidate).await {
                                    Ok(copied) => {
                                        if copied.as_ref().is_some_and(|copied| copied.created) {
                                            staged_artifacts.push(
                                                copied
                                                    .as_ref()
                                                    .expect("checked copied artifact")
                                                    .path
                                                    .clone(),
                                            );
                                        }
                                        insert_artifact(
                                            &mut transaction,
                                            &session.id,
                                            message_id,
                                            &candidate,
                                            copied.as_ref().map(|copied| copied.path.as_path()),
                                            &created_at,
                                        )
                                        .await?;
                                        imported.artifacts += 1;
                                    }
                                    Err(error) => errors.push(format!(
                                        "session {} artifact {}: {}",
                                        session.id,
                                        candidate.display(),
                                        error
                                    )),
                                }
                            }
                        }
                    } else {
                        skipped_count += 1;
                    }
                }
            }
        }

        for job in &legacy_jobs {
            match insert_legacy_routine(
                &mut transaction,
                job,
                options.artifact_root.as_deref(),
                &mut staged_artifacts,
                &mut errors,
            )
            .await?
            {
                true => imported.routines += 1,
                false => skipped_count += 1,
            }
        }

        for definition in &legacy_mcp.definitions {
            let inserted = query(
                "INSERT INTO agent_mcp_servers
                 (id, name, enabled, transport, command, args_json, url, secret_ref,
                  metadata_json, tool_visibility_json, safety_json, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(name) DO NOTHING",
            )
            .bind(&definition.id)
            .bind(&definition.name)
            .bind(i64::from(definition.enabled))
            .bind(match definition.transport {
                crate::agent_mcp::McpTransport::Stdio => "stdio",
                crate::agent_mcp::McpTransport::StreamableHttp => "streamable_http",
            })
            .bind(&definition.command)
            .bind(serde_json::to_string(&definition.args).unwrap_or_else(|_| "[]".into()))
            .bind(&definition.url)
            .bind(&definition.secret_ref)
            .bind(serde_json::to_string(&definition.metadata).unwrap_or_else(|_| "{}".into()))
            .bind(
                serde_json::to_string(&definition.tool_visibility).unwrap_or_else(|_| "{}".into()),
            )
            .bind(serde_json::to_string(&definition.safety).unwrap_or_else(|_| "{}".into()))
            .bind(&started_at)
            .bind(&started_at)
            .execute(&mut *transaction)
            .await?;
            if inserted.rows_affected() == 1 {
                if let Some(secret_ref) = definition.secret_ref.as_deref() {
                    if let Some((_, bundle)) = legacy_mcp
                        .secrets
                        .iter()
                        .find(|(candidate, _)| candidate == secret_ref)
                    {
                        let previous = secret_store
                            .get(secret_ref)
                            .map_err(|_| LegacyImportCommitError::SecureStorage)?;
                        staged_secrets.push((secret_ref.to_string(), previous));
                        secret_store
                            .put(secret_ref, bundle)
                            .map_err(|_| LegacyImportCommitError::SecureStorage)?;
                    }
                }
                imported.mcp_servers += 1;
            } else {
                skipped_count += 1;
            }
        }

        query(
            "INSERT INTO agent_migration_manifests
             (migration_key, source_path, source_fingerprint, status, source_counts_json,
              imported_counts_json, skipped_count, errors_json, started_at, completed_at)
             VALUES (?, ?, ?, 'completed', ?, ?, ?, ?, ?, ?)
             ON CONFLICT(migration_key) DO UPDATE SET
               source_path = excluded.source_path,
               source_fingerprint = excluded.source_fingerprint,
               status = excluded.status,
               source_counts_json = excluded.source_counts_json,
               imported_counts_json = excluded.imported_counts_json,
               skipped_count = excluded.skipped_count,
               errors_json = excluded.errors_json,
               started_at = excluded.started_at,
               completed_at = excluded.completed_at",
        )
        .bind(MIGRATION_KEY)
        .bind(source_path.to_string_lossy().as_ref())
        .bind(&fingerprint)
        .bind(serde_json::to_string(&source_counts).unwrap_or_else(|_| "{}".to_string()))
        .bind(serde_json::to_string(&imported).unwrap_or_else(|_| "{}".to_string()))
        .bind(skipped_count as i64)
        .bind(serde_json::to_string(&errors).unwrap_or_else(|_| "[]".to_string()))
        .bind(&started_at)
        .bind(&completed_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(())
    }
    .await;

    source.close().await;
    if let Err(error) = result {
        restore_staged_secrets(&secret_store, &staged_secrets);
        remove_staged_artifacts(&staged_artifacts).await;
        let diagnostic = vec![error.to_string()];
        record_failed_manifest(
            destination,
            source_path,
            &fingerprint,
            &source_counts,
            &diagnostic,
            &started_at,
        )
        .await?;
        return Err(match error {
            LegacyImportCommitError::Destination(error) => LegacyImportError::Destination(error),
            LegacyImportCommitError::SecureStorage => LegacyImportError::SecureStorage,
        });
    }

    Ok(AgentMigrationManifestDto {
        migration_key: MIGRATION_KEY.to_string(),
        source_path: source_path.to_string_lossy().into_owned(),
        source_fingerprint: Some(fingerprint),
        status: "completed".to_string(),
        source_counts,
        imported_counts: imported,
        skipped_count,
        errors,
        started_at,
        completed_at: Some(completed_at),
    })
}

/// Whether the one-time legacy import has committed in Clovy's database. This
/// check intentionally touches no legacy path, allowing app startup to avoid
/// even inspecting the retired home after the cutover succeeds.
pub async fn legacy_import_completed(destination: &SqlitePool) -> Result<bool, LegacyImportError> {
    Ok(completed_manifest(destination).await?.is_some())
}

async fn read_only_pool(path: &Path) -> Result<SqlitePool, LegacyImportError> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .map_err(LegacyImportError::Source)?
        .create_if_missing(false)
        .read_only(true);
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .map_err(LegacyImportError::Source)
}

async fn empty_legacy_source_pool() -> Result<SqlitePool, LegacyImportError> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .map_err(LegacyImportError::Source)?;
    query(
        "CREATE TABLE sessions (
           id TEXT PRIMARY KEY, source TEXT, model TEXT, title TEXT,
           started_at REAL, ended_at REAL, end_reason TEXT, parent_session_id TEXT
         )",
    )
    .execute(&pool)
    .await
    .map_err(LegacyImportError::Source)?;
    query(
        "CREATE TABLE messages (
           id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, content TEXT,
           timestamp REAL
         )",
    )
    .execute(&pool)
    .await
    .map_err(LegacyImportError::Source)?;
    Ok(pool)
}

async fn source_counts(pool: &SqlitePool) -> Result<MigrationCounts, LegacyImportError> {
    let sessions = query(
        "SELECT COUNT(*) AS count FROM sessions
         WHERE parent_session_id IS NULL
           AND lower(source) NOT IN ('subagent', 'delegate', 'delegated')",
    )
    .fetch_one(pool)
    .await
    .map_err(LegacyImportError::Source)?
    .get::<i64, _>("count") as u64;
    let messages = query(
        "SELECT COUNT(*) AS count FROM messages m
         JOIN sessions s ON s.id = m.session_id
         WHERE s.parent_session_id IS NULL
           AND lower(s.source) NOT IN ('subagent', 'delegate', 'delegated')",
    )
    .fetch_one(pool)
    .await
    .map_err(LegacyImportError::Source)?
    .get::<i64, _>("count") as u64;
    Ok(MigrationCounts {
        sessions,
        messages,
        reasoning_items: 0,
        artifacts: 0,
        routines: 0,
        mcp_servers: 0,
    })
}

async fn load_sessions(pool: &SqlitePool) -> Result<Vec<LegacySession>, sqlx::Error> {
    let rows = query(
        "SELECT id, source, model, title, started_at, ended_at, end_reason
         FROM sessions
         WHERE parent_session_id IS NULL
           AND lower(source) NOT IN ('subagent', 'delegate', 'delegated')
         ORDER BY started_at ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| LegacySession {
            id: row.get("id"),
            source: row.get::<String, _>("source").to_lowercase(),
            model: row
                .try_get::<Option<String>, _>("model")
                .ok()
                .flatten()
                .unwrap_or_else(|| "auto".to_string()),
            title: row
                .try_get::<Option<String>, _>("title")
                .ok()
                .flatten()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "Imported session".to_string()),
            started_at: row.get("started_at"),
            ended_at: row.try_get("ended_at").ok().flatten(),
            end_reason: row.try_get("end_reason").ok().flatten(),
        })
        .collect())
}

async fn load_messages(
    pool: &SqlitePool,
    session_id: &str,
    has_active: bool,
) -> Result<Vec<SqliteRow>, sqlx::Error> {
    let sql = if has_active {
        "SELECT id, role, content, tool_call_id, tool_calls, tool_name, timestamp,
                reasoning, reasoning_content
         FROM messages WHERE session_id = ? AND active = 1 ORDER BY timestamp ASC, id ASC"
    } else {
        "SELECT id, role, content, tool_call_id, tool_calls, tool_name, timestamp,
                reasoning, reasoning_content
         FROM messages WHERE session_id = ? ORDER BY timestamp ASC, id ASC"
    };
    query(sql).bind(session_id).fetch_all(pool).await
}

async fn table_has_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
) -> Result<bool, sqlx::Error> {
    let rows = query(&format!("PRAGMA table_info({table})"))
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .any(|row| row.get::<String, _>("name") == column))
}

async fn next_sequence(
    transaction: &mut SqliteConnection,
    session_id: &str,
) -> Result<i64, sqlx::Error> {
    query("SELECT COALESCE(MAX(sequence), -1) + 1 AS next FROM agent_items WHERE session_id = ?")
        .bind(session_id)
        .fetch_one(&mut *transaction)
        .await
        .map(|row| row.get("next"))
}

async fn insert_item(
    transaction: &mut SqliteConnection,
    session_id: &str,
    sequence: i64,
    kind: &str,
    payload: Value,
    external_id: &str,
    created_at: &str,
) -> Result<bool, sqlx::Error> {
    let id = deterministic_id("item", external_id);
    query(
        "INSERT OR IGNORE INTO agent_items
         (id, session_id, sequence, kind, payload_json, external_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(session_id)
    .bind(sequence)
    .bind(kind)
    .bind(payload.to_string())
    .bind(external_id)
    .bind(created_at)
    .execute(&mut *transaction)
    .await
    .map(|result| result.rows_affected() == 1)
}

async fn insert_artifact(
    transaction: &mut SqliteConnection,
    session_id: &str,
    message_id: i64,
    original: &Path,
    copied: Option<&Path>,
    created_at: &str,
) -> Result<(), sqlx::Error> {
    let original_string = original.to_string_lossy();
    let id = deterministic_id("artifact", &format!("{session_id}:{original_string}"));
    let path = copied.unwrap_or(original).to_string_lossy();
    let size = copied
        .and_then(|path| std::fs::metadata(path).ok())
        .map(|metadata| metadata.len() as i64);
    query(
        "INSERT OR IGNORE INTO agent_artifacts
         (id, session_id, provenance, action, path, original_path, size_bytes, available, created_at)
         VALUES (?, ?, ?, 'created', ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(session_id)
    .bind(format!("legacy_hermes_message:{message_id}"))
    .bind(path.as_ref())
    .bind(original_string.as_ref())
    .bind(size)
    .bind(i64::from(copied.is_some()))
    .bind(created_at)
    .execute(&mut *transaction)
    .await?;
    Ok(())
}

async fn copy_artifact(
    root: &Path,
    session_id: &str,
    source: &Path,
) -> Result<Option<CopiedArtifact>, std::io::Error> {
    if !source.is_absolute() {
        return Ok(None);
    }
    let metadata = match tokio::fs::symlink_metadata(source).await {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_LEGACY_ARTIFACT_BYTES
    {
        return Ok(None);
    }
    let source_key = deterministic_id("source", source.to_string_lossy().as_ref());
    let destination_dir = root.join(safe_component(session_id)).join(source_key);
    tokio::fs::create_dir_all(&destination_dir).await?;
    let name = source
        .file_name()
        .and_then(|name| name.to_str())
        .map(safe_component)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| deterministic_id("file", source.to_string_lossy().as_ref()));
    let destination = destination_dir.join(name);
    let mut created = false;
    if !destination.exists() {
        tokio::fs::copy(source, &destination).await?;
        created = true;
    }
    Ok(Some(CopiedArtifact {
        path: destination,
        created,
    }))
}

#[derive(Debug)]
struct CopiedArtifact {
    path: PathBuf,
    created: bool,
}

async fn remove_staged_artifacts(paths: &[PathBuf]) {
    for path in paths.iter().rev() {
        let _ = tokio::fs::remove_file(path).await;
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::remove_dir(parent).await;
        }
    }
}

fn artifact_candidates(value: &Value) -> BTreeSet<PathBuf> {
    fn visit(value: &Value, key: Option<&str>, found: &mut BTreeSet<PathBuf>) {
        match value {
            Value::Object(map) => {
                for (child_key, child) in map {
                    visit(child, Some(child_key), found);
                }
            }
            Value::Array(items) => {
                for item in items {
                    visit(item, key, found);
                }
            }
            Value::String(path)
                if key.is_some_and(|key| {
                    matches!(
                        key,
                        "path"
                            | "filePath"
                            | "file_path"
                            | "artifactPath"
                            | "artifact_path"
                            | "outputPath"
                            | "output_path"
                    )
                }) =>
            {
                found.insert(PathBuf::from(path));
            }
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    visit(value, None, &mut found);
    found
}

async fn completed_manifest(
    pool: &SqlitePool,
) -> Result<Option<AgentMigrationManifestDto>, LegacyImportError> {
    let row = query(
        "SELECT migration_key, source_path, source_fingerprint, status, source_counts_json,
                imported_counts_json, skipped_count, errors_json, started_at, completed_at
         FROM agent_migration_manifests
         WHERE migration_key = ? AND status = 'completed'",
    )
    .bind(MIGRATION_KEY)
    .fetch_optional(pool)
    .await
    .map_err(LegacyImportError::Destination)?;
    Ok(row.map(manifest_from_row))
}

async fn record_failed_manifest(
    pool: &SqlitePool,
    source_path: &Path,
    fingerprint: &str,
    source_counts: &MigrationCounts,
    errors: &[String],
    started_at: &str,
) -> Result<(), LegacyImportError> {
    query(
        "INSERT INTO agent_migration_manifests
         (migration_key, source_path, source_fingerprint, status, source_counts_json,
          imported_counts_json, skipped_count, errors_json, started_at, completed_at)
         VALUES (?, ?, ?, 'failed', ?, '{}', 0, ?, ?, ?)
         ON CONFLICT(migration_key) DO UPDATE SET
           source_path = excluded.source_path, source_fingerprint = excluded.source_fingerprint,
           status = excluded.status, source_counts_json = excluded.source_counts_json,
           imported_counts_json = excluded.imported_counts_json,
           skipped_count = excluded.skipped_count, errors_json = excluded.errors_json,
           started_at = excluded.started_at, completed_at = excluded.completed_at",
    )
    .bind(MIGRATION_KEY)
    .bind(source_path.to_string_lossy().as_ref())
    .bind(fingerprint)
    .bind(serde_json::to_string(source_counts).unwrap_or_else(|_| "{}".to_string()))
    .bind(serde_json::to_string(errors).unwrap_or_else(|_| "[]".to_string()))
    .bind(started_at)
    .bind(crate::db::repositories::timestamp())
    .execute(pool)
    .await
    .map_err(LegacyImportError::Destination)?;
    Ok(())
}

fn manifest_from_row(row: SqliteRow) -> AgentMigrationManifestDto {
    AgentMigrationManifestDto {
        migration_key: row.get("migration_key"),
        source_path: row.get("source_path"),
        source_fingerprint: row.get("source_fingerprint"),
        status: row.get("status"),
        source_counts: serde_json::from_str(&row.get::<String, _>("source_counts_json"))
            .unwrap_or_default(),
        imported_counts: serde_json::from_str(&row.get::<String, _>("imported_counts_json"))
            .unwrap_or_default(),
        skipped_count: row.get::<i64, _>("skipped_count") as u64,
        errors: serde_json::from_str(&row.get::<String, _>("errors_json")).unwrap_or_default(),
        started_at: row.get("started_at"),
        completed_at: row.get("completed_at"),
    }
}

fn empty_manifest(path: &Path, status: &str) -> AgentMigrationManifestDto {
    AgentMigrationManifestDto {
        migration_key: MIGRATION_KEY.to_string(),
        source_path: path.to_string_lossy().into_owned(),
        source_fingerprint: None,
        status: status.to_string(),
        source_counts: MigrationCounts::default(),
        imported_counts: MigrationCounts::default(),
        skipped_count: 0,
        errors: Vec::new(),
        started_at: crate::db::repositories::timestamp(),
        completed_at: Some(crate::db::repositories::timestamp()),
    }
}

fn source_fingerprint(paths: &[&Path]) -> Result<String, std::io::Error> {
    let mut hash = Sha256::new();
    for path in paths {
        hash.update(path.to_string_lossy().as_bytes());
        if let Ok(metadata) = std::fs::metadata(path) {
            let modified = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            hash.update(metadata.len().to_le_bytes());
            hash.update(modified.to_le_bytes());
        } else {
            hash.update(b"missing");
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn first_nonempty(row: &SqliteRow, columns: &[&str]) -> Option<String> {
    columns.iter().find_map(|column| {
        row.try_get::<Option<String>, _>(*column)
            .ok()
            .flatten()
            .filter(|value| !value.trim().is_empty())
    })
}

fn parse_json_or_string(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::String(value.to_string()))
}

fn deterministic_id(namespace: &str, value: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(namespace.as_bytes());
    hash.update(b":");
    hash.update(value.as_bytes());
    format!("{namespace}-{:x}", hash.finalize())
}

fn safe_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn timestamp(value: f64) -> String {
    let seconds = value.trunc() as i64;
    let nanos = ((value.fract().abs() * 1_000_000_000.0).round() as u32).min(999_999_999);
    Utc.timestamp_opt(seconds, nanos)
        .single()
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::{
        copy_artifact, legacy_gateway_command_matches, legacy_gateway_pid_from_state,
        stop_legacy_hermes_runtime,
    };
    use serde_json::json;
    use std::{fs, time::Duration};

    #[test]
    fn accepts_june_hermes_gateway_state() {
        let state = json!({
            "pid": 42,
            "kind": "hermes-gateway",
            "argv": [
                "/Applications/Clovy.app/Contents/Resources/native/hermes/bin/python3",
                "-m",
                "hermes_cli.main",
                "--profile",
                "profile-2",
                "gateway",
                "run",
                "--replace"
            ]
        });

        assert_eq!(legacy_gateway_pid_from_state(&state), Some(42));
    }

    #[test]
    fn rejects_unrelated_or_incomplete_process_state() {
        assert_eq!(
            legacy_gateway_pid_from_state(&json!({
                "pid": 42,
                "kind": "other",
                "argv": ["hermes", "gateway", "run"]
            })),
            None
        );
        assert_eq!(
            legacy_gateway_pid_from_state(&json!({
                "pid": 42,
                "kind": "hermes-gateway",
                "argv": ["hermes", "status"]
            })),
            None
        );
        assert!(!legacy_gateway_command_matches(
            "/usr/bin/python unrelated_agent.py"
        ));
    }

    #[tokio::test]
    async fn copies_distinct_regular_artifacts_without_following_symlinks() {
        let root = tempfile::tempdir().expect("temporary migration root");
        let first_dir = root.path().join("one");
        let second_dir = root.path().join("two");
        fs::create_dir_all(&first_dir).expect("first source directory");
        fs::create_dir_all(&second_dir).expect("second source directory");
        let first = first_dir.join("report.txt");
        let second = second_dir.join("report.txt");
        fs::write(&first, "first").expect("first artifact");
        fs::write(&second, "second").expect("second artifact");
        let destination = root.path().join("imported");

        let copied_first = copy_artifact(&destination, "session", &first)
            .await
            .expect("copy first")
            .expect("first artifact exists");
        let copied_second = copy_artifact(&destination, "session", &second)
            .await
            .expect("copy second")
            .expect("second artifact exists");
        assert_ne!(copied_first.path, copied_second.path);
        assert_eq!(fs::read_to_string(copied_first.path).unwrap(), "first");
        assert_eq!(fs::read_to_string(copied_second.path).unwrap(), "second");

        #[cfg(unix)]
        {
            let linked = root.path().join("linked.txt");
            std::os::unix::fs::symlink(&first, &linked).expect("artifact symlink");
            assert!(copy_artifact(&destination, "session", &linked)
                .await
                .expect("reject symlink")
                .is_none());
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn stops_only_a_live_gateway_recorded_in_junes_legacy_home() {
        let root = tempfile::tempdir().expect("temporary Hermes home");
        let mut child = tokio::process::Command::new("/bin/bash")
            .args([
                "-c",
                "exec -a '/tmp/Clovy.app/Contents/Resources/native/hermes/hermes_cli gateway run' sleep 30",
            ])
            .spawn()
            .expect("spawn fake legacy gateway");
        let pid = child.id().expect("child pid");
        fs::write(
            root.path().join("gateway_state.json"),
            serde_json::to_vec(&json!({
                "pid": pid,
                "kind": "hermes-gateway",
                "argv": [
                    "/tmp/Clovy.app/Contents/Resources/native/hermes/hermes_cli/main.py",
                    "gateway",
                    "run",
                    "--replace"
                ]
            }))
            .expect("serialize gateway state"),
        )
        .expect("write gateway state");

        stop_legacy_hermes_runtime(root.path(), None).await;

        tokio::time::timeout(Duration::from_secs(3), child.wait())
            .await
            .expect("gateway should stop before the timeout")
            .expect("wait for fake gateway");
    }
}
