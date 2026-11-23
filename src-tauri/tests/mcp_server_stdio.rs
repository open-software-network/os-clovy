//! The `clovy-mcp` binary driven the way an MCP client drives it (spawned,
//! JSON-RPC over stdio), with the app side of the local channel (Unix socket
//! + installation secret) served in this process against test databases.
#![cfg(any(unix, windows))]

use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, TimeZone, Utc};
use clovy_lib::activity::key::{ActivityKeyStore, KeyStoreError};
use clovy_lib::activity::settings::ActivitySettings;
use clovy_lib::activity::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};
use clovy_lib::activity::timeline::etl::run_pass;
use clovy_lib::agent_sessions::{AgentRepository, AgentSafetyMode};
use clovy_lib::db::repositories::Repositories;
use clovy_lib::llm::cli::CliKind;
use clovy_lib::mcp_server::tools::{ActivityAccess, McpData, ReadTools, ToolCtx, ToolGroup};
use clovy_lib::mcp_server::turns::{TurnBinding, TurnRegistry};
use clovy_lib::mcp_server::{channel, protocol, stdio};
use serde_json::{json, Value};
use sqlx::query::query;
use sqlx_sqlite::SqlitePool;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use zeroize::Zeroizing;

const RANGE_FROM: &str = "2026-10-01T08:00:00Z";
const RANGE_TO: &str = "2026-10-01T12:00:00Z";

#[path = "support/mcp_binding.rs"]
mod host_binding;

fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, hour, minute, 0).unwrap()
}

/// The activity key held in memory: these tests never touch the Keychain.
#[derive(Default)]
struct MemoryKey(Mutex<Option<String>>);

impl MemoryKey {
    fn slot(&self) -> std::sync::MutexGuard<'_, Option<String>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ActivityKeyStore for MemoryKey {
    fn load(&self) -> Result<Option<Zeroizing<String>>, KeyStoreError> {
        Ok(self.slot().clone().map(Zeroizing::new))
    }
    fn store(&self, key_hex: &str) -> Result<(), KeyStoreError> {
        *self.slot() = Some(key_hex.to_string());
        Ok(())
    }
    fn delete(&self) -> Result<(), KeyStoreError> {
        *self.slot() = None;
        Ok(())
    }
}

/// A note and a dictation in the current data partition (`default`) and
/// decoys in another partition.
async fn notes_pool() -> SqlitePool {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    clovy_lib::db::migrations::run_migrations(&pool)
        .await
        .unwrap();
    let repositories = Repositories::new(pool.clone());
    for (id, profile, title, content) in [
        (
            "note-roadmap",
            "default",
            "Roadmap sync",
            "Decisions: ship the zephyr importer on Friday.",
        ),
        (
            "note-other-profile",
            "work",
            "Zephyr budget",
            "Other profile: zephyr budget is confidential.",
        ),
    ] {
        repositories
            .create_note_with_id(profile, None, id)
            .await
            .unwrap();
        query("UPDATE notes SET title = ?, edited_content = ? WHERE id = ?")
            .bind(title)
            .bind(content)
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
    }
    for (id, profile, text) in [
        ("dictation-1", "default", "Call Bruno about zephyr."),
        ("dictation-2", "work", "Zephyr work dictation."),
    ] {
        query("INSERT INTO dictation_history (id, text, language, provider, created_at, profile) VALUES (?, ?, 'en', 'test', ?, ?)")
            .bind(id)
            .bind(text)
            .bind(Utc::now().to_rfc3339())
            .bind(profile)
            .execute(&pool)
            .await
            .unwrap();
    }
    pool
}

/// Five minutes in Code, then two in Firefox, on the morning of `at(9, 0)`.
async fn activity_store(dir: &Path) -> ActivityStore {
    let store = ActivityStore::open(&dir.join(ACTIVITY_DB_FILE), &MemoryKey::default())
        .await
        .unwrap();
    for (start, count, app, url) in [
        (at(9, 0), 150, "Code", None),
        (at(9, 5), 60, "Firefox", Some("https://docs.rs/zephyrine")),
    ] {
        for index in 0..count {
            store
                .insert_frame(&NewFrame {
                    captured_at: start + Duration::seconds(index * 2),
                    app_name: app.into(),
                    bundle_id: None,
                    window_title: Some(format!("{app} window")),
                    browser_url: url.map(str::to_string),
                    text_source: TextSource::Accessibility,
                    text: Some(format!("{app} text {index}")),
                    input_idle_ms: None,
                })
                .await
                .unwrap();
        }
    }
    run_pass(&store, at(9, 8), &[]).await.unwrap();
    store
}

/// The app side: each message is answered from the test databases, activity
/// capture on, in the `default` profile.
fn handler(notes: SqlitePool, store: ActivityStore) -> channel::Handler {
    Arc::new(move |message: Value| {
        let data = McpData {
            notes: notes.clone(),
            profile: "default".into(),
            memory_enabled: true,
            activity: ActivityAccess::Open {
                store: store.clone(),
                settings: ActivitySettings {
                    enabled: true,
                    ..ActivitySettings::default()
                },
            },
            now: at(9, 8),
        };
        Box::pin(async move { protocol::handle(&data, &message).await })
    })
}

/// `clovy-mcp --dir <dir>` as an MCP client spawns it.
struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: i64,
}

impl Client {
    async fn spawn(dir: &Path) -> Self {
        Self::spawn_with_env(dir, &[]).await
    }

    /// Spawns with `env` added; the production-data opt-in is always
    /// cleared first so the host environment cannot change the outcome.
    async fn spawn_with_env(dir: &Path, env: &[(&str, &std::ffi::OsStr)]) -> Self {
        Self::spawn_with(dir, &[], env).await
    }

    /// `clovy-mcp --dir <dir> --turn <token>`, as a Clovy CLI turn spawns it.
    async fn spawn_for_turn(dir: &Path, token: &str) -> Self {
        Self::spawn_with(dir, &["--turn", token], &[]).await
    }

    async fn spawn_with(dir: &Path, args: &[&str], env: &[(&str, &std::ffi::OsStr)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_clovy-mcp"));
        command
            .arg("--dir")
            .arg(dir)
            .args(args)
            .env_remove("OS_CLOVY_USE_PROD_DATA_DIR")
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().expect("clovy-mcp starts");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 0,
        };
        let initialized = client
            .request(
                "initialize",
                json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "integration-test", "version": "1" } }),
            )
            .await;
        assert_eq!(
            initialized["result"]["serverInfo"]["name"],
            json!("clovy"),
            "{initialized}"
        );
        client
            .send(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .await;
        client
    }

    async fn send(&mut self, message: Value) {
        self.stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        self.stdin.flush().await.unwrap();
    }

    async fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await;
        loop {
            let line = tokio::time::timeout(StdDuration::from_secs(10), self.stdout.next_line())
                .await
                .unwrap_or_else(|_| panic!("no answer to {method} within 10 s"))
                .unwrap()
                .expect("clovy-mcp closed stdout");
            let message: Value = serde_json::from_str(&line).expect("stdout carries JSON-RPC");
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// A `tools/call` result: `Ok(payload)` or `Err(text)` for `isError`.
    async fn call(&mut self, name: &str, arguments: Value) -> Result<Value, String> {
        let response = self
            .request(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await;
        let result = &response["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("tools/call answered without text: {response}"));
        if result["isError"] == json!(true) {
            Err(text.to_string())
        } else {
            Ok(serde_json::from_str(text).expect("tool text is JSON"))
        }
    }

    async fn close(mut self) {
        drop(self.stdin);
        let status = tokio::time::timeout(StdDuration::from_secs(10), self.child.wait())
            .await
            .expect("clovy-mcp exits when stdin closes")
            .unwrap();
        assert!(status.success(), "{status}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_binary_answers_a_stdio_client_from_the_active_profile() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let notes = notes_pool().await;
    let store = activity_store(root.path()).await;
    let listener = channel::Listener::start(&dir, handler(notes.clone(), store.clone()))
        .await
        .unwrap();
    let mut client = Client::spawn(&dir).await;

    let listed = client.request("tools/list", json!({})).await;
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    // ReadTools::owns delegates to the crate's is_read_tool predicate.
    assert!(names.iter().all(|name| ReadTools.owns(name)), "{listed}");
    for name in [
        "search_notes",
        "get_note",
        "list_dictations",
        "get_activity_timeline",
        "get_day_panels",
        "get_day_schedule",
    ] {
        assert!(names.contains(&name), "missing {name}: {listed}");
    }

    let found = client
        .call("search_notes", json!({ "query": "zephyr" }))
        .await
        .unwrap();
    let titles: Vec<&str> = found["notes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|note| note["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["Roadmap sync"], "{found}");
    let other = client
        .call("get_note", json!({ "id": "note-other-profile" }))
        .await;
    assert!(other.is_err(), "another profile's note leaked: {other:?}");
    let dictations = client.call("list_dictations", json!({})).await.unwrap();
    assert_eq!(
        dictations["items"].as_array().unwrap().len(),
        1,
        "{dictations}"
    );
    assert_eq!(
        dictations["items"][0]["text"],
        json!("Call Bruno about zephyr.")
    );

    let timeline = client
        .call(
            "get_activity_timeline",
            json!({ "from": RANGE_FROM, "to": RANGE_TO }),
        )
        .await
        .unwrap();
    let apps: Vec<&str> = timeline["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["app"].as_str().unwrap())
        .collect();
    assert_eq!(apps, vec!["Code", "Firefox"], "{timeline}");

    let day = at(9, 0)
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d")
        .to_string();
    let panels = client
        .call("get_day_panels", json!({ "date": day }))
        .await
        .unwrap();
    assert_eq!(panels["state"], json!("ready"), "{panels}");
    assert_eq!(panels["panels"]["focusedMinutes"], json!(7.0), "{panels}");
    assert_eq!(panels["panels"]["meetings"]["count"], json!(0));
    let schedule = client.call("get_day_schedule", json!({})).await.unwrap();
    assert_eq!(schedule["summaryTime"], json!("18:00"), "{schedule}");

    let guide = client
        .request("resources/read", json!({ "uri": "clovy://guide" }))
        .await;
    assert!(guide["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("search_notes"));
    let context = client
        .request("resources/read", json!({ "uri": "clovy://context" }))
        .await;
    let context: Value =
        serde_json::from_str(context["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert!(context["timeZone"]
        .as_str()
        .is_some_and(|zone| !zone.is_empty()));

    // Turned off in Settings: the same client process gets the reason, and
    // answers again once the server is back on.
    listener.stop_and_wait().await;
    assert_eq!(
        client
            .call("search_notes", json!({ "query": "zephyr" }))
            .await,
        Err(stdio::OFF_MESSAGE.to_string())
    );
    let listener = channel::Listener::start(&dir, handler(notes, store))
        .await
        .unwrap();
    assert!(client
        .call("search_notes", json!({ "query": "zephyr" }))
        .await
        .is_ok());

    client.close().await;
    listener.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_binary_reports_never_enabled_and_a_refused_secret() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());

    let mut never = Client::spawn(&dir).await;
    assert_eq!(
        never
            .call("search_notes", json!({ "query": "zephyr" }))
            .await,
        Err(stdio::NEVER_ENABLED_MESSAGE.to_string())
    );
    never.close().await;

    let store = activity_store(root.path()).await;
    let listener = channel::Listener::start(&dir, handler(notes_pool().await, store))
        .await
        .unwrap();
    // A secret that is not the installation's: the app refuses before any
    // tool runs.
    std::fs::write(channel::secret_path(&dir), "ab".repeat(32)).unwrap();
    let mut refused = Client::spawn(&dir).await;
    assert_eq!(
        refused
            .call("search_notes", json!({ "query": "zephyr" }))
            .await,
        Err(stdio::REFUSED_MESSAGE.to_string())
    );
    let listed = refused.request("tools/list", json!({})).await;
    assert_eq!(listed["error"]["message"], json!(stdio::REFUSED_MESSAGE));
    refused.close().await;
    listener.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(target_os = "macos")]
async fn mcp_server_debug_binary_refuses_the_installed_app_data() {
    // A throwaway HOME (short path: Unix socket paths are capped at 104
    // bytes) whose production data directory has a live channel.
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let production = home
        .path()
        .join("Library/Application Support/dev.guilhermexp.clovy");
    let dir = channel::channel_dir(&production);
    let store = activity_store(home.path()).await;
    let listener = channel::Listener::start(&dir, handler(notes_pool().await, store))
        .await
        .unwrap();
    let home_env = home.path().as_os_str();

    let mut refused = Client::spawn_with_env(&dir, &[("HOME", home_env)]).await;
    assert_eq!(
        refused
            .call("search_notes", json!({ "query": "zephyr" }))
            .await,
        Err(stdio::PRODUCTION_DATA_MESSAGE.to_string())
    );
    refused.close().await;

    // The explicit production-data opt-in, as for the debug app.
    let mut allowed = Client::spawn_with_env(
        &dir,
        &[
            ("HOME", home_env),
            ("OS_CLOVY_USE_PROD_DATA_DIR", std::ffi::OsStr::new("1")),
        ],
    )
    .await;
    assert!(allowed
        .call("search_notes", json!({ "query": "zephyr" }))
        .await
        .is_ok());
    allowed.close().await;
    listener.stop_and_wait().await;
}

/// The app side with turn bindings, as the app serves CLI turns.
fn turn_handler(notes: SqlitePool) -> channel::TurnHandler {
    Arc::new(move |message: Value, binding: Option<Arc<TurnBinding>>| {
        let data = McpData {
            notes: notes.clone(),
            profile: "default".into(),
            memory_enabled: true,
            activity: ActivityAccess::Off,
            now: at(9, 8),
        };
        Box::pin(async move {
            let ctx = ToolCtx {
                data: &data,
                app: None,
                binding: binding.as_deref(),
                broker: None,
            };
            protocol::handle_in(&ctx, &message).await
        })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_binary_binds_a_turn_until_it_is_revoked() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    let notes = notes_pool().await;
    let repo = AgentRepository::new(notes.clone());
    let session = repo
        .create_session_in_profile(
            "Bound stdio fixture",
            "test",
            AgentSafetyMode::Sandboxed,
            None,
            "default",
        )
        .await
        .unwrap();
    let run = repo.create_run(&session.id, "test", None).await.unwrap();
    let mut binding = host_binding::binding(CliKind::Codex, true, &session.id, &run.id);
    // Read the host-created snapshot through the public repository API.
    // Bound dispatch revalidates it via the internal run_partition::resolve.
    binding.data_partition = repo.run_config(&run.id).await.unwrap().unwrap()["dataPartition"]
        .as_str()
        .unwrap()
        .to_string();
    let token = turns.issue(binding);
    let listener = channel::Listener::start_with_turns(&dir, turn_handler(notes), turns.clone())
        .await
        .unwrap();

    // A live turn: the compiled relay is accepted and serves the catalog.
    let mut bound = Client::spawn_for_turn(&dir, &token).await;
    let listed = bound.request("tools/list", json!({})).await;
    assert!(listed["result"]["tools"].as_array().is_some(), "{listed}");
    let found = bound
        .call("search_notes", json!({ "query": "zephyr" }))
        .await
        .unwrap();
    let found_notes = found["notes"].as_array().unwrap();
    let roadmap = found_notes
        .iter()
        .find(|note| note["id"] == "note-roadmap")
        .unwrap_or_else(|| panic!("default partition's note missing: {found}"));
    assert_eq!(roadmap["title"], json!("Roadmap sync"), "{found}");
    assert!(
        found_notes
            .iter()
            .all(|note| note["id"] != "note-other-profile"),
        "another partition's note leaked: {found}"
    );

    // The run ends: the open relay and a new one with the same token both
    // answer that the turn ended, and no tool runs.
    assert_eq!(turns.revoke_run(&run.id), 1);
    for _ in 0..50 {
        if bound
            .call("search_notes", json!({ "query": "zephyr" }))
            .await
            == Err(stdio::TURN_EXPIRED_MESSAGE.to_string())
        {
            break;
        }
        tokio::time::sleep(StdDuration::from_millis(20)).await;
    }
    assert_eq!(
        bound
            .call("search_notes", json!({ "query": "zephyr" }))
            .await,
        Err(stdio::TURN_EXPIRED_MESSAGE.to_string())
    );
    bound.close().await;
    let mut reused = Client::spawn_for_turn(&dir, &token).await;
    assert_eq!(
        reused
            .call("search_notes", json!({ "query": "zephyr" }))
            .await,
        Err(stdio::TURN_EXPIRED_MESSAGE.to_string())
    );
    reused.close().await;

    // An empty `--turn` is a usage error, not an unbound connection.
    let status = Command::new(env!("CARGO_BIN_EXE_clovy-mcp"))
        .arg("--dir")
        .arg(&dir)
        .arg("--turn")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .unwrap();
    assert_eq!(status.code(), Some(2));
    listener.stop_and_wait().await;
}

/// The app side of a turn whose `computer_use` answers with a capture as MCP
/// content blocks (`protocol::content_value`), as the computer use group
/// does after an approved action.
fn image_handler(notes: SqlitePool, image: String) -> channel::TurnHandler {
    let inner = turn_handler(notes);
    Arc::new(move |message: Value, binding: Option<Arc<TurnBinding>>| {
        let is_capture = binding.is_some()
            && protocol::method(&message) == Some("tools/call")
            && message.pointer("/params/name") == Some(&json!("computer_use"));
        if !is_capture {
            return inner(message, binding);
        }
        let image = image.clone();
        Box::pin(async move {
            let id = protocol::request_id(&message)?;
            Some(protocol::result_response(
                id,
                protocol::tool_result(&protocol::content_value(vec![
                    protocol::text_block("[1] button \"Save\""),
                    protocol::image_block(image, "image/png"),
                ])),
            ))
        })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_server_binary_carries_image_content_blocks_to_the_cli() {
    use base64::Engine as _;
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let turns = TurnRegistry::default();
    // About 1 MiB of base64, the size of a real window capture.
    let image = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; 768 * 1024]);
    let listener = channel::Listener::start_with_turns(
        &dir,
        image_handler(notes_pool().await, image.clone()),
        turns.clone(),
    )
    .await
    .unwrap();
    let token = turns.issue(host_binding::binding(
        CliKind::Claude,
        true,
        "session-1",
        "run-1",
    ));
    let mut client = Client::spawn_for_turn(&dir, &token).await;
    let response = client
        .request(
            "tools/call",
            json!({ "name": "computer_use", "arguments": { "action": "capture", "app": "TextEdit" } }),
        )
        .await;
    let result = &response["result"];
    assert_eq!(result["isError"], json!(false), "{response}");
    assert_eq!(
        result["content"][0],
        json!({ "type": "text", "text": "[1] button \"Save\"" })
    );
    assert_eq!(result["content"][1]["type"], json!("image"));
    assert_eq!(result["content"][1]["mimeType"], json!("image/png"));
    assert_eq!(result["content"][1]["data"], json!(image));

    // A plain tool keeps its single JSON text block.
    let plain = client
        .request(
            "tools/call",
            json!({ "name": "search_notes", "arguments": { "query": "zephyr" } }),
        )
        .await;
    let blocks = plain["result"]["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], json!("text"));
    client.close().await;
    listener.stop_and_wait().await;
}

struct ExternalFixture {
    // Keep the socket directory alive for the listener's lifetime.
    listener: channel::Listener,
    root: tempfile::TempDir,
    notes: SqlitePool,
    approvals: Arc<clovy_lib::approval_broker::external::ExternalApprovalBroker>,
    profile: Arc<Mutex<String>>,
}

impl ExternalFixture {
    async fn new(timeout: StdDuration) -> Self {
        let root = tempfile::tempdir().unwrap();
        let notes = notes_pool().await;
        let profile = Arc::new(Mutex::new("default".to_string()));
        let active_profile = Arc::clone(&profile);
        let approvals = Arc::new(
            clovy_lib::approval_broker::external::ExternalApprovalBroker::new(
                move || Some(active_profile.lock().unwrap().clone()),
                || {},
                timeout,
            ),
        );
        let settings = root.path().join("memory-settings.json");
        std::fs::write(&settings, br#"{"enabled":true}"#).unwrap();
        let fixture_recordings = root.path().to_path_buf();
        let fixture_notes = notes.clone();
        let fixture_profile = Arc::clone(&profile);
        let fixture_approvals = Arc::clone(&approvals);
        let handler: channel::AuthenticatedHandler = Arc::new(move |message, connection| {
            let notes = fixture_notes.clone();
            let profile = fixture_profile.lock().unwrap().clone();
            let approvals = Arc::clone(&fixture_approvals);
            let settings = settings.clone();
            let recordings = fixture_recordings.clone();
            Box::pin(async move {
                let channel::Connection::External(connection) = connection else {
                    panic!("fixture issued no turn grants");
                };
                let data = McpData {
                    notes,
                    profile,
                    memory_enabled: true,
                    activity: ActivityAccess::Off,
                    now: Utc::now(),
                };
                protocol::handle_external(
                    &ToolCtx::unbound(&data),
                    &connection,
                    &approvals,
                    Some(&settings),
                    Some(&recordings),
                    &message,
                )
                .await
            })
        });
        let listener = channel::Listener::start(&channel::channel_dir(root.path()), handler)
            .await
            .unwrap();
        Self {
            listener,
            root,
            notes,
            approvals,
            profile,
        }
    }

    async fn connect(&self) -> Client {
        Client::spawn(&channel::channel_dir(self.root.path())).await
    }

    async fn pending(&self, expected: usize) -> Vec<Value> {
        tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                let rows = self.approvals.pending();
                if rows.len() == expected {
                    return rows
                        .into_iter()
                        .map(|row| serde_json::to_value(row).unwrap())
                        .collect();
                }
                tokio::time::sleep(StdDuration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("expected {expected} pending approvals"))
    }

    async fn memory_count(&self) -> i64 {
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM memories WHERE source = 'agent'")
            .fetch_one(&self.notes)
            .await
            .unwrap()
    }

    fn decide(
        &self,
        request: &Value,
        approve: bool,
    ) -> Result<(), clovy_lib::domain::types::AppError> {
        self.approvals
            .decide(request["id"].as_str().unwrap(), approve)
    }

    fn select_profile(&self, profile: &str) {
        *self.profile.lock().unwrap() = profile.into();
        self.approvals.invalidate();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_external_relay_discovers_protected_tools_without_a_turn() {
    let fixture = ExternalFixture::new(StdDuration::from_secs(30)).await;
    let mut client = fixture.connect().await;
    let listed = client.request("tools/list", json!({})).await;
    let names = listed["result"]["tools"].as_array().unwrap();
    assert!(names.iter().any(|tool| tool["name"] == "get_note"));
    assert!(names.iter().any(|tool| tool["name"] == "save_memory"));
    assert!(!names.iter().any(|tool| tool["name"] == "get_conversation"));
    assert_eq!(
        client
            .call("get_note", json!({"id":"note-roadmap"}))
            .await
            .unwrap()["id"],
        "note-roadmap"
    );
    assert!(client
        .call("get_note", json!({"id":"note-other-profile"}))
        .await
        .unwrap_err()
        .contains("note_not_found"));
    let missing_turn = client
        .call(
            "get_conversation",
            json!({"conversationId":"internal-session-fixture"}),
        )
        .await
        .unwrap_err();
    assert!(missing_turn.contains("turn_required"), "{missing_turn}");
    let invalid = client
        .call(
            "save_memory",
            json!({"content":"Fixture","workspace":"/invalid","runId":"internal-run-fixture"}),
        )
        .await
        .unwrap_err();
    assert!(
        invalid.contains("agent_tool_arguments_invalid"),
        "{invalid}"
    );
    assert!(fixture.approvals.pending().is_empty());
    assert_eq!(fixture.memory_count().await, 0);
    client.close().await;
    fixture.listener.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_external_relay_applies_one_decision_to_one_connection_and_target() {
    let fixture = ExternalFixture::new(StdDuration::from_secs(30)).await;
    let mut first = fixture.connect().await;
    let mut second = fixture.connect().await;
    let approve = tokio::spawn(async move {
        let result = first
            .call("save_memory", json!({"content":"Keep the local fixture"}))
            .await;
        (first, result)
    });
    let deny = tokio::spawn(async move {
        let result = second
            .call("save_memory", json!({"content":"Deny the local fixture"}))
            .await;
        (second, result)
    });
    let requests = fixture.pending(2).await;
    let allowed = requests
        .iter()
        .find(|value| {
            value["preview"]
                .as_str()
                .unwrap()
                .contains("Keep the local fixture")
        })
        .unwrap();
    let blocked = requests
        .iter()
        .find(|value| value["id"] != allowed["id"])
        .unwrap();
    assert_eq!(allowed["profile"], "default");
    assert_eq!(allowed["tool"], "save_memory");
    assert_eq!(fixture.memory_count().await, 0);
    fixture.decide(blocked, false).unwrap();
    fixture.decide(allowed, true).unwrap();
    assert_eq!(
        fixture.decide(allowed, true).unwrap_err().code,
        "mcp_external_approval_expired"
    );
    let (mut first, written) = approve.await.unwrap();
    let id = written.unwrap()["id"].as_str().unwrap().to_string();
    let (second, denied) = deny.await.unwrap();
    assert!(denied.unwrap_err().contains("user_denied"));
    assert_eq!(fixture.memory_count().await, 1);
    let delete = tokio::spawn(async move {
        let result = first.call("forget_memory", json!({"id":id})).await;
        (first, result)
    });
    let pending_delete = fixture.pending(1).await.remove(0);
    assert_eq!(pending_delete["tool"], "forget_memory");
    assert!(pending_delete["preview"]
        .as_str()
        .unwrap()
        .contains("Permanently delete"));
    assert!(pending_delete["preview"]
        .as_str()
        .unwrap()
        .contains("Keep the local fixture"));
    assert_eq!(fixture.memory_count().await, 1);
    fixture.decide(&pending_delete, true).unwrap();
    let (first, result) = delete.await.unwrap();
    assert_eq!(result.unwrap()["forgotten"], true);
    assert_eq!(fixture.memory_count().await, 0);
    let tombstones: i64 =
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM memory_tombstones")
            .fetch_one(&fixture.notes)
            .await
            .unwrap();
    assert_eq!(tombstones, 1);
    let session_count: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_runs")
        .fetch_one(&fixture.notes)
        .await
        .unwrap();
    assert_eq!(session_count, 0);
    first.close().await;
    second.close().await;
    fixture.listener.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_external_relay_expiry_disconnect_and_cancellation_revoke_pending_calls() {
    let fixture = ExternalFixture::new(StdDuration::from_millis(1500)).await;
    let mut client = fixture.connect().await;
    let expires = tokio::spawn(async move {
        let expired = client
            .call("save_memory", json!({"content":"Expires"}))
            .await;
        (client, expired)
    });
    let request = fixture.pending(1).await.remove(0);
    let (mut client, expired) = expires.await.unwrap();
    assert!(expired
        .unwrap_err()
        .contains("mcp_external_approval_expired"));
    assert!(fixture.decide(&request, true).is_err());

    client.send(json!({"jsonrpc":"2.0","id":300,"method":"tools/call","params":{"name":"save_memory","arguments":{"content":"User cancelled"}}})).await;
    let cancel = fixture.pending(1).await.remove(0);
    client
        .send(
            json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":300}}),
        )
        .await;
    fixture.pending(0).await;
    assert!(fixture.decide(&cancel, true).is_err());

    for id in 310..314 {
        client.send(json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"save_memory","arguments":{"content":"Disconnected"}}})).await;
    }
    let disconnected = fixture.pending(4).await;
    client.close().await;
    fixture.pending(0).await;
    for request in disconnected {
        assert!(fixture.decide(&request, true).is_err());
    }
    assert_eq!(fixture.memory_count().await, 0);
    fixture.listener.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_external_relay_revalidates_approved_write_after_profile_switch_and_writer_wait() {
    let fixture = ExternalFixture::new(StdDuration::from_secs(30)).await;
    let mut client = fixture.connect().await;
    let changed = tokio::spawn(async move {
        let result = client
            .call("save_memory", json!({"content":"Old data partition"}))
            .await;
        (client, result)
    });
    let request = fixture.pending(1).await.remove(0);
    let writer = fixture.notes.begin_with("BEGIN IMMEDIATE").await.unwrap();
    fixture.decide(&request, true).unwrap();
    fixture.select_profile("work");
    fixture.select_profile("default");
    writer.rollback().await.unwrap();
    let (mut client, result) = changed.await.unwrap();
    assert!(result.unwrap_err().contains("mcp_external_scope_changed"));
    assert_eq!(fixture.memory_count().await, 0);

    let pending = tokio::spawn(async move {
        let result = client
            .call("save_memory", json!({"content":"Still pending"}))
            .await;
        (client, result)
    });
    let request = fixture.pending(1).await.remove(0);
    fixture.select_profile("work");
    fixture.pending(0).await;
    assert!(fixture.decide(&request, true).is_err());
    let (mut client, result) = pending.await.unwrap();
    assert!(result.unwrap_err().contains("mcp_external_scope_changed"));
    assert_eq!(fixture.memory_count().await, 0);
    assert_eq!(
        client
            .call("get_note", json!({"id":"note-other-profile"}))
            .await
            .unwrap()["id"],
        "note-other-profile"
    );
    client.close().await;
    fixture.listener.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_external_relay_rejects_a_second_approved_write_against_a_changed_target() {
    let fixture = ExternalFixture::new(StdDuration::from_secs(30)).await;
    let mut first = fixture.connect().await;
    let mut second = fixture.connect().await;
    let memory = Repositories::new(fixture.notes.clone())
        .create_memory("default", None, "Unchanged fixture", "user")
        .await
        .unwrap();
    let target = memory.id.clone();
    let update_first = tokio::spawn(async move {
        let result = first
            .call("update_memory", json!({"id":target,"content":"Edit one"}))
            .await;
        (first, result)
    });
    let target = memory.id.clone();
    let update_second = tokio::spawn(async move {
        let result = second
            .call("update_memory", json!({"id":target,"content":"Edit two"}))
            .await;
        (second, result)
    });
    let approvals = fixture.pending(2).await;
    assert!(approvals.iter().all(|request| request["preview"]
        .as_str()
        .unwrap()
        .contains("Unchanged fixture")));
    for request in approvals {
        fixture.decide(&request, true).unwrap();
    }
    let (first, a) = update_first.await.unwrap();
    let (second, b) = update_second.await.unwrap();
    assert_ne!(a.is_ok(), b.is_ok());
    let (result, conflict) = match (a, b) {
        (Ok(result), Err(conflict)) | (Err(conflict), Ok(result)) => (result, conflict),
        _ => unreachable!("only one approved mutation can apply"),
    };
    assert!(conflict.contains("memory_conflict"), "{conflict}");
    let stored: String =
        sqlx::query_scalar::query_scalar("SELECT content FROM memories WHERE id = ?")
            .bind(memory.id)
            .fetch_one(&fixture.notes)
            .await
            .unwrap();
    assert_eq!(result["content"], stored);
    first.close().await;
    second.close().await;
    fixture.listener.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_external_relay_refuses_project_cascade_when_only_an_artifact_changed() {
    use clovy_lib::domain::types::RecordingSourceMode;
    let fixture = ExternalFixture::new(StdDuration::from_secs(30)).await;
    let mut client = fixture.connect().await;
    let repos = Repositories::new(fixture.notes.clone());
    let project = repos
        .create_folder("default", "Project fixture", None)
        .await
        .unwrap();
    let note = repos
        .create_note("default", Some(project.id.clone()))
        .await
        .unwrap();
    let recording = "recording-fixture";
    repos
        .create_recording_session(
            &note.id,
            recording,
            RecordingSourceMode::MicrophoneOnly,
            "",
            "",
            None,
        )
        .await
        .unwrap();
    query("UPDATE recording_sessions SET status = 'stopped' WHERE id = ?")
        .bind(recording)
        .execute(&fixture.notes)
        .await
        .unwrap();
    let paths =
        clovy_lib::app_paths::AppPaths::from_data_dir(fixture.root.path().to_owned()).unwrap();
    let directory = paths.recording_session_dir(&note.id, recording).unwrap();
    std::fs::create_dir_all(&directory).unwrap();

    let project_id = project.id.clone();
    let cascade = tokio::spawn(async move {
        let result = client
            .call(
                "delete_project",
                json!({"projectId":project_id,"deleteNotes":true}),
            )
            .await;
        (client, result)
    });
    let approval = fixture.pending(1).await.remove(0);
    assert!(approval["preview"]
        .as_str()
        .unwrap()
        .contains("Project fixture"));
    assert_eq!(
        repos.get_note(&note.id).await.unwrap().revision,
        note.revision
    );
    let file = directory.join("synthetic-fixture.wav");
    std::fs::write(&file, b"fixture only").unwrap();
    repos
        .create_audio_artifact(
            &note.id,
            recording,
            file.to_str().unwrap(),
            1,
            12,
            "fixture",
        )
        .await
        .unwrap();
    assert_eq!(
        repos.get_note(&note.id).await.unwrap().revision,
        note.revision
    );
    fixture.decide(&approval, true).unwrap();
    let (client, result) = cascade.await.unwrap();
    assert!(result.unwrap_err().contains("project_conflict"));
    assert!(file.is_file());
    assert!(repos.get_note(&note.id).await.is_ok());
    assert_eq!(
        repos.list_folders("default").await.unwrap()[0].name,
        "Project fixture"
    );
    client.close().await;
    fixture.listener.stop_and_wait().await;
}
