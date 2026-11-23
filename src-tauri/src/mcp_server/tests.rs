//! MCP calls against test databases: the protocol handler directly, and the
//! real relay (`stdio::relay`) through the authenticated socket.

use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use serde_json::{json, Value};
use sqlx::query::query;
use sqlx_sqlite::SqlitePool;

use super::tools::{ActivityAccess, McpData};
use super::*;
use crate::activity::key::MemoryKeyStore;
use crate::activity::settings::ActivitySettings;
use crate::activity::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};
use crate::activity::timeline::etl::run_pass;
use crate::db::repositories::Repositories;

pub(super) fn at(hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, hour, minute, 0).unwrap()
}

/// Notes, dictations, and memories in the default profile, and a decoy of
/// each in another profile.
pub(crate) async fn notes_pool() -> SqlitePool {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    // Transcripts reference audio artifacts the fixture does not need.
    query("PRAGMA foreign_keys = OFF")
        .execute(&pool)
        .await
        .unwrap();
    let repositories = Repositories::new(pool.clone());
    for (id, profile, title, content, transcript) in [
        (
            "note-roadmap",
            "default",
            "Roadmap sync",
            "Decisions: ship the zephyr importer on Friday after QA signs off.",
            "Alice: the zephyr importer is blocked by QA until Thursday.",
        ),
        (
            "note-other-profile",
            "work",
            "Zephyr budget",
            "Other profile: zephyr budget is confidential.",
            "",
        ),
        (
            "note-unrelated",
            "default",
            "Lunch",
            "Pizza on Tuesday.",
            "",
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
        if !transcript.is_empty() {
            query("INSERT INTO transcripts (id, note_id, audio_artifact_id, text, provider, status, created_at, updated_at) VALUES (?, ?, 'audio', ?, 'test', 'completed', ?, ?)")
                .bind(format!("transcript-{id}"))
                .bind(id)
                .bind(transcript)
                .bind(Utc::now().to_rfc3339())
                .bind(Utc::now().to_rfc3339())
                .execute(&pool)
                .await
                .unwrap();
        }
    }
    for (id, profile, text) in [
        (
            "dictation-1",
            "default",
            "Remember to call Bruno about zephyr.",
        ),
        ("dictation-2", "default", "Buy coffee."),
        ("dictation-3", "work", "Zephyr work dictation."),
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
    repositories
        .create_memory("default", None, "Prefers short summaries.", "user")
        .await
        .unwrap();
    repositories
        .create_memory("work", None, "Work profile memory.", "user")
        .await
        .unwrap();
    pool
}

/// The morning of `at(9, 0)`: Code, Firefox on docs.rs, 1Password.
pub(super) async fn activity_store() -> (tempfile::TempDir, ActivityStore) {
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
            "etl.rs - os-clovy",
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

pub(crate) fn data(notes: SqlitePool, activity: ActivityAccess) -> McpData {
    McpData {
        notes,
        profile: "default".into(),
        memory_enabled: true,
        activity,
        now: at(9, 8),
    }
}

fn capture_on() -> ActivitySettings {
    ActivitySettings {
        enabled: true,
        ..ActivitySettings::default()
    }
}

async fn request(data: &McpData, method: &str, params: Value) -> Value {
    protocol::handle(
        data,
        &json!({ "jsonrpc": "2.0", "id": 7, "method": method, "params": params }),
    )
    .await
    .expect("a request is answered")
}

/// A `tools/call` result's JSON payload, or the error text.
async fn call(data: &McpData, name: &str, arguments: Value) -> Result<Value, String> {
    let response = request(
        data,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await;
    let result = &response["result"];
    let text = result["content"][0]["text"].as_str().unwrap_or_default();
    if result["isError"] == json!(true) {
        Err(text.to_string())
    } else {
        Ok(serde_json::from_str(text).expect("tool text is JSON"))
    }
}

fn tool_names(response: &Value) -> Vec<String> {
    response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn activity_tools_are_listed_only_while_capture_is_on() {
    let pool = notes_pool().await;
    let off = tool_names(
        &request(
            &data(pool.clone(), ActivityAccess::Off),
            "tools/list",
            json!({}),
        )
        .await,
    );
    assert_eq!(
        off,
        vec![
            "search_notes",
            "get_note",
            "list_note_transcripts",
            "list_dictations",
            "list_memories"
        ]
    );
    let on = tool_names(&request(&data(pool, ActivityAccess::On), "tools/list", json!({})).await);
    assert_eq!(
        on,
        vec![
            "search_notes",
            "get_note",
            "list_note_transcripts",
            "list_dictations",
            "list_memories",
            "get_activity_timeline",
            "search_activity",
            "get_activity_stats",
            "get_active_session",
            "list_app_usage",
            "get_session_detail",
            "list_coding_agent_sessions",
            "get_day_summary",
            "get_day_panels",
            "get_day_schedule",
        ]
    );
}

#[tokio::test]
async fn activity_tool_calls_refuse_while_capture_is_off() {
    let data = data(notes_pool().await, ActivityAccess::Off);
    let error = call(&data, "get_activity_timeline", json!({}))
        .await
        .unwrap_err();
    assert!(error.contains("Activity capture is off"), "{error}");
}

#[tokio::test]
async fn search_notes_returns_title_and_snippet_from_the_active_profile_only() {
    let data = data(notes_pool().await, ActivityAccess::Off);
    let found = call(&data, "search_notes", json!({ "query": "Zephyr" }))
        .await
        .unwrap();
    let notes = found["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{found}");
    assert_eq!(notes[0]["id"], json!("note-roadmap"));
    assert_eq!(notes[0]["title"], json!("Roadmap sync"));
    assert_eq!(notes[0]["matchedIn"], json!("note"));
    assert!(notes[0]["snippet"]
        .as_str()
        .unwrap()
        .contains("ship the zephyr importer"));

    let by_transcript = call(&data, "search_notes", json!({ "query": "Alice" }))
        .await
        .unwrap();
    assert_eq!(by_transcript["notes"][0]["matchedIn"], json!("transcript"));
}

#[tokio::test]
async fn get_note_reads_the_note_and_transcript_but_not_another_profile() {
    let data = data(notes_pool().await, ActivityAccess::Off);
    let note = call(&data, "get_note", json!({ "id": "note-roadmap" }))
        .await
        .unwrap();
    assert_eq!(note["title"], json!("Roadmap sync"));
    assert!(note["content"].as_str().unwrap().contains("Friday"));
    assert!(note["transcript"].as_str().unwrap().contains("Alice"));
    assert_eq!(note["nextTranscriptOffset"], Value::Null);

    let error = call(&data, "get_note", json!({ "id": "note-other-profile" }))
        .await
        .unwrap_err();
    assert!(error.contains("note_not_found"), "{error}");
}

#[tokio::test]
async fn dictations_and_memories_stay_in_the_active_profile() {
    let pool = notes_pool().await;
    let data_on = data(pool.clone(), ActivityAccess::Off);
    let dictations = call(&data_on, "list_dictations", json!({})).await.unwrap();
    let texts: Vec<&str> = dictations["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts.len(), 2, "{dictations}");
    assert!(!texts.iter().any(|text| text.contains("work")));
    let filtered = call(&data_on, "list_dictations", json!({ "query": "zephyr" }))
        .await
        .unwrap();
    assert_eq!(filtered["count"], json!(1));

    let memories = call(&data_on, "list_memories", json!({})).await.unwrap();
    assert_eq!(memories["count"], json!(1));
    assert_eq!(
        memories["items"][0]["content"],
        json!("Prefers short summaries.")
    );

    let mut memory_off = data(pool, ActivityAccess::Off);
    memory_off.memory_enabled = false;
    let error = call(&memory_off, "list_memories", json!({}))
        .await
        .unwrap_err();
    assert!(error.contains("memory_disabled"), "{error}");
}

#[tokio::test]
async fn activity_tools_answer_with_exclusions_and_retention() {
    let (_dir, store) = activity_store().await;
    let pool = notes_pool().await;
    let open = |settings: ActivitySettings| ActivityAccess::Open {
        store: store.clone(),
        settings,
    };
    let data_all = data(pool.clone(), open(capture_on()));
    let range = json!({ "from": "2026-10-01T08:00:00Z", "to": "2026-10-01T12:00:00Z" });

    let timeline = call(&data_all, "get_activity_timeline", range.clone())
        .await
        .unwrap();
    let apps: Vec<&str> = timeline["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| session["app"].as_str().unwrap())
        .collect();
    assert_eq!(apps, vec!["Code", "Firefox", "1Password"]);
    let vault_id = timeline["sessions"][2]["id"].as_i64().unwrap();

    let usage = call(&data_all, "list_app_usage", range.clone())
        .await
        .unwrap();
    assert_eq!(
        usage["apps"][0],
        json!({ "app": "Code", "minutes": 5.0, "sessions": 1 })
    );
    let stats = call(&data_all, "get_activity_stats", range.clone())
        .await
        .unwrap();
    assert_eq!(stats["focusedMinutes"], json!(8.0));

    let detail = call(&data_all, "get_session_detail", json!({ "id": vault_id }))
        .await
        .unwrap();
    assert_eq!(detail["session"]["app"], json!("1Password"));
    assert_eq!(detail["windows"][0]["windowTitle"], json!("Vault"));

    let now = call(&data_all, "get_active_session", json!({}))
        .await
        .unwrap();
    let current = if now["session"].is_null() {
        &now["lastSession"]
    } else {
        &now["session"]
    };
    assert_eq!(current["app"], json!("1Password"), "{now}");

    let excluding = data(
        pool.clone(),
        open(ActivitySettings {
            ignored_apps: vec!["1Password".into()],
            ignored_domains: vec!["docs.rs".into()],
            ..capture_on()
        }),
    );
    let usage = call(&excluding, "list_app_usage", range.clone())
        .await
        .unwrap();
    let apps: Vec<&str> = usage["apps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|app| app["app"].as_str().unwrap())
        .collect();
    assert_eq!(apps, vec!["Code"]);
    let hidden = call(&excluding, "get_session_detail", json!({ "id": vault_id }))
        .await
        .unwrap_err();
    assert!(hidden.contains("activity_session_not_found"), "{hidden}");
    let searched = call(
        &excluding,
        "search_activity",
        json!({ "query": "zephyrine" }),
    )
    .await
    .unwrap();
    assert_eq!(searched["results"], json!([]));

    let mut expired = data(pool, open(capture_on()));
    expired.now = at(9, 8) + Duration::days(31);
    let gone = call(&expired, "get_session_detail", json!({ "id": vault_id }))
        .await
        .unwrap_err();
    assert!(gone.contains("activity_session_not_found"), "{gone}");
}

#[tokio::test]
async fn resources_give_the_current_context_and_the_guide() {
    let data = data(notes_pool().await, ActivityAccess::Off);
    let listed = request(&data, "resources/list", json!({})).await;
    let uris: Vec<&str> = listed["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|resource| resource["uri"].as_str().unwrap())
        .collect();
    assert_eq!(uris, vec!["clovy://context", "clovy://guide"]);

    let guide = request(&data, "resources/read", json!({ "uri": "clovy://guide" })).await;
    assert!(guide["result"]["contents"][0]["text"]
        .as_str()
        .unwrap()
        .contains("search_notes"));
    let missing = request(&data, "resources/read", json!({ "uri": "clovy://nope" })).await;
    assert_eq!(missing["error"]["code"], json!(-32002));

    let context = request(&data, "resources/read", json!({ "uri": "clovy://context" })).await;
    let context: Value =
        serde_json::from_str(context["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    let local = at(9, 8).with_timezone(&Local);
    assert_eq!(context["date"], json!(local.format("%Y-%m-%d").to_string()));
    assert_eq!(context["time"], json!(local.format("%H:%M").to_string()));
    assert!(context["timeZone"]
        .as_str()
        .is_some_and(|zone| !zone.is_empty()));
}

#[tokio::test]
async fn context_uses_bound_session_and_hides_it_from_external_clients() {
    use super::tools::ToolCtx;
    use crate::approval_broker::testing::TestBroker;
    use crate::llm::cli::CliKind;
    let broker = TestBroker::new([]).await;
    let mut binding = broker.turn(CliKind::Claude, true).await;
    binding.surface = "home".into();
    // A retained legacy preference is not the current configured policy.
    sqlx::query::query("UPDATE agent_sessions SET approval_mode = 'bypass' WHERE id = ?")
        .bind(&binding.session_id)
        .execute(broker.pool())
        .await
        .unwrap();
    let data = data(broker.pool().clone(), ActivityAccess::Off);
    let message = json!({"jsonrpc":"2.0", "id":1, "method":"resources/read", "params":{"uri":"clovy://context"}});
    let external = protocol::handle(&data, &message).await.unwrap();
    let external: Value =
        serde_json::from_str(external["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert!(external.get("session").is_none());
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let bound = protocol::handle_in(&ctx, &message).await.unwrap();
    let bound: Value =
        serde_json::from_str(bound["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(bound["session"]["id"], binding.session_id);
    assert_eq!(bound["session"]["runId"], binding.run_id);
    assert_eq!(bound["session"]["surface"], "home");
    assert_eq!(bound["session"]["approvalMode"], "approvals");
    assert_eq!(bound["session"]["effectiveApprovalMode"], "approvals");

    // Configuration changes apply to the next run, not this run's snapshot.
    sqlx::query::query("UPDATE agent_sessions SET safety_mode = 'unrestricted' WHERE id = ?")
        .bind(&binding.session_id)
        .execute(broker.pool())
        .await
        .unwrap();
    let changed = protocol::handle_in(&ctx, &message).await.unwrap();
    let changed: Value =
        serde_json::from_str(changed["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        changed["session"]["configuredExecutionPolicy"],
        "unrestricted"
    );
    assert_eq!(changed["session"]["approvalMode"], "bypass");
    assert_eq!(changed["session"]["effectiveApprovalMode"], "approvals");
}

#[test]
fn tool_failure_preserves_delivery_details_for_structured_and_text_clients() {
    let mut error = crate::domain::types::AppError::new("turn_expired", "The turn expired.");
    error.details = Some(json!({"effect":"completed_before_turn_revalidation","retrySafe":false}));
    let result = protocol::tool_error_for(&error);
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["content"][0]["text"],
        "The turn expired. (turn_expired)"
    );
    assert_eq!(
        result["structuredContent"]["error"]["details"]["retrySafe"],
        false
    );
    let text: Value = serde_json::from_str(result["content"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(
        text["details"]["effect"],
        "completed_before_turn_revalidation"
    );
}

#[test]
fn context_reports_date_time_and_zone() {
    let now = chrono::FixedOffset::west_opt(3 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 10, 4, 14, 5, 0)
        .unwrap()
        .with_timezone(&Local);
    let context = protocol::context_json(now, Some("America/Sao_Paulo".into()));
    assert_eq!(context["timeZone"], json!("America/Sao_Paulo"));
    assert_eq!(context["now"], json!(now.to_rfc3339()));
    assert_eq!(context["date"], json!(now.format("%Y-%m-%d").to_string()));
}

#[test]
fn configuration_snippets_point_at_the_bundled_relay() {
    let launch = launch(
        Path::new("/Applications/Clovy.app/Contents/Resources/native/bin/clovy-mcp"),
        Path::new("/Users/me/Library/Application Support/dev.guilhermexp.clovy/mcp"),
    );
    let claude = claude_code_config(&launch);
    assert_eq!(
        claude,
        json!({ "mcpServers": { "clovy": {
            "type": "stdio",
            "command": "/Applications/Clovy.app/Contents/Resources/native/bin/clovy-mcp",
            "args": ["--dir", "/Users/me/Library/Application Support/dev.guilhermexp.clovy/mcp"],
        }}})
    );
    assert_eq!(
        cursor_config(&launch)["mcpServers"]["clovy"]["command"],
        claude["mcpServers"]["clovy"]["command"]
    );
    assert_eq!(
        cursor_config(&launch)["mcpServers"]["clovy"].get("type"),
        None
    );
    #[cfg(not(windows))]
    assert_eq!(
        claude_code_command(&launch),
        "claude mcp add --scope user clovy -- /Applications/Clovy.app/Contents/Resources/native/bin/clovy-mcp --dir '/Users/me/Library/Application Support/dev.guilhermexp.clovy/mcp'"
    );
    #[cfg(windows)]
    assert_eq!(
        claude_code_command(&launch),
        "claude mcp add --scope user clovy -- '/Applications/Clovy.app/Contents/Resources/native/bin/clovy-mcp' '--dir' '/Users/me/Library/Application Support/dev.guilhermexp.clovy/mcp'"
    );
    assert_eq!(
        stdio::parse_dir(&["--dir".into(), "/tmp/x".into()]).unwrap(),
        Path::new("/tmp/x")
    );
    assert!(stdio::parse_dir(&[]).is_err());
}

#[cfg(windows)]
#[test]
fn windows_launch_uses_exe_and_powershell_literals_while_json_stays_structured() {
    let resources = Path::new(r"C:\Clovy's folder\resources");
    let exe = Path::new(r"C:\Clovy's folder\clovy.exe");
    let candidates = binary_candidates(Some(resources), Some(exe));
    assert_eq!(candidates[0], resources.join("native/bin/clovy-mcp.exe"));
    assert_eq!(candidates[1], exe.parent().unwrap().join("clovy-mcp.exe"));
    let launch = launch(&candidates[0], Path::new(r"C:\Data ' literal\mcp"));
    assert!(claude_code_command(&launch).contains("Clovy''s folder"));
    assert!(claude_code_command(&launch).ends_with("'C:\\Data '' literal\\mcp'"));
    let config = claude_code_config(&launch);
    assert_eq!(
        config["mcpServers"][SERVER_KEY]["args"][1],
        r"C:\Data ' literal\mcp"
    );
    assert_eq!(
        config["mcpServers"][SERVER_KEY]["command"],
        launch.command.display().to_string()
    );
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Collects what the relay writes to stdout.
#[derive(Clone, Default)]
pub(crate) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        lock(&self.0).extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Captured {
    /// Waits for `count` complete newline-delimited messages from the relay.
    /// The app serializes each message across writes on another thread.
    pub(crate) fn messages(&self, count: usize) -> Vec<Value> {
        for _ in 0..200 {
            let messages: Vec<Value> = lock(&self.0)
                .split_inclusive(|byte| *byte == b'\n')
                .take_while(|line| line.ends_with(b"\n"))
                .map(|line| serde_json::from_slice(line).unwrap())
                .collect();
            if messages.len() >= count {
                return messages;
            }
            std::thread::sleep(StdDuration::from_millis(25));
        }
        panic!("relay wrote fewer than {count} messages");
    }
}

pub(crate) fn by_id(messages: &[Value], id: i64) -> &Value {
    messages
        .iter()
        .find(|message| message["id"] == json!(id))
        .unwrap_or_else(|| panic!("no answer for id {id}: {messages:?}"))
}

#[cfg(any(unix, windows))]
pub(crate) fn handler_for(data: McpData) -> channel::Handler {
    let data = Arc::new(data);
    Arc::new(move |message: Value| {
        let data = Arc::clone(&data);
        Box::pin(async move { protocol::handle(&data, &message).await })
    })
}

pub(crate) async fn run_relay(dir: &Path, lines: &[Value]) -> Captured {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    let output = Captured::default();
    let (dir, sink) = (dir.to_path_buf(), output.clone());
    tokio::task::spawn_blocking(move || {
        stdio::relay(&dir, std::io::Cursor::new(input), Box::new(sink));
    })
    .await
    .unwrap();
    output
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(unix)]
async fn the_relay_answers_mcp_calls_through_the_authenticated_socket() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let listener = channel::Listener::start(
        &dir,
        handler_for(data(notes_pool().await, ActivityAccess::Off)),
    )
    .await
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&channel::socket_path(&dir)), 0o600);
    assert_eq!(mode(&channel::secret_path(&dir)), 0o600);

    let output = run_relay(
        &dir,
        &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
            json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "search_notes", "arguments": { "query": "zephyr" } } }),
        ],
    )
    .await;
    let messages = output.messages(3);
    assert_eq!(
        by_id(&messages, 1)["result"]["serverInfo"]["name"],
        json!("clovy")
    );
    assert!(by_id(&messages, 2)["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == tools::SEARCH_NOTES));
    let found: Value = serde_json::from_str(
        by_id(&messages, 3)["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(found["notes"][0]["title"], json!("Roadmap sync"));
    listener.stop();
    assert!(!channel::socket_path(&dir).exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(any(unix, windows))]
async fn the_relay_reports_a_server_that_is_off_or_never_enabled() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let lines = [
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18" } }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "search_notes", "arguments": { "query": "x" } } }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }),
    ];
    let never = run_relay(&dir, &lines).await.messages(3);
    assert_eq!(
        by_id(&never, 2)["result"]["content"][0]["text"],
        json!(stdio::NEVER_ENABLED_MESSAGE)
    );

    // Turned on once, then off: the secret stays, the socket is gone.
    let listener = channel::Listener::start(
        &dir,
        handler_for(data(notes_pool().await, ActivityAccess::Off)),
    )
    .await
    .unwrap();
    listener.stop_and_wait().await;
    let off = run_relay(&dir, &lines).await.messages(3);
    assert!(by_id(&off, 1)["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains(stdio::OFF_MESSAGE));
    assert_eq!(by_id(&off, 2)["result"]["isError"], json!(true));
    assert_eq!(
        by_id(&off, 2)["result"]["content"][0]["text"],
        json!(stdio::OFF_MESSAGE)
    );
    assert_eq!(
        by_id(&off, 3)["error"]["message"],
        json!(stdio::OFF_MESSAGE)
    );
}

/// `channel::connect`, retried while the app frees slots.
#[cfg(unix)]
async fn connect_eventually(dir: &Path) -> std::os::unix::net::UnixStream {
    let dir = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        for _ in 0..60 {
            if let Ok(stream) = channel::connect(&dir) {
                return stream;
            }
            std::thread::sleep(StdDuration::from_millis(50));
        }
        panic!("no authenticated connection within 3 s");
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(unix)]
async fn peers_that_never_authenticate_are_capped_and_closed_at_once() {
    use tokio::io::AsyncReadExt;
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let listener = channel::Listener::start(
        &dir,
        handler_for(data(notes_pool().await, ActivityAccess::Off)),
    )
    .await
    .unwrap();
    let socket = channel::socket_path(&dir);
    let mut idle = Vec::new();
    for _ in 0..channel::MAX_PENDING_HANDSHAKES {
        idle.push(tokio::net::UnixStream::connect(&socket).await.unwrap());
    }
    // Over the cap: closed right away, not held for the 5 s handshake.
    let mut extra = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let mut byte = [0u8; 1];
    let read = tokio::time::timeout(StdDuration::from_secs(2), extra.read(&mut byte))
        .await
        .expect("the extra peer is closed before the handshake timeout");
    assert!(matches!(read, Ok(0) | Err(_)), "{read:?}");

    // Once the silent peers go away, a real client gets in.
    drop(idle);
    drop(connect_eventually(&dir).await);
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(unix)]
async fn authenticated_connections_beyond_the_cap_are_turned_away_as_busy() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let listener = channel::Listener::start(
        &dir,
        handler_for(data(notes_pool().await, ActivityAccess::Off)),
    )
    .await
    .unwrap();
    let held_dir = dir.clone();
    let held = tokio::task::spawn_blocking(move || {
        (0..channel::MAX_CONNECTIONS)
            .map(|_| channel::connect(&held_dir).expect("within the cap"))
            .collect::<Vec<_>>()
    })
    .await
    .unwrap();
    let extra_dir = dir.clone();
    let extra = tokio::task::spawn_blocking(move || channel::connect(&extra_dir))
        .await
        .unwrap();
    assert!(
        matches!(extra, Err(channel::ClientError::Busy)),
        "{extra:?}"
    );
    drop(held);
    drop(connect_eventually(&dir).await);
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(unix)]
async fn a_connection_gets_only_a_few_requests_answered_at_once() {
    use std::io::{BufRead, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let started = Arc::new(AtomicUsize::new(0));
    let handler: channel::Handler = {
        let (gate, started) = (Arc::clone(&gate), Arc::clone(&started));
        Arc::new(move |message: Value| {
            let (gate, started) = (Arc::clone(&gate), Arc::clone(&started));
            Box::pin(async move {
                started.fetch_add(1, Ordering::SeqCst);
                gate.acquire().await.unwrap().forget();
                Some(protocol::result_response(message["id"].clone(), json!({})))
            })
        })
    };
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let listener = channel::Listener::start(&dir, handler).await.unwrap();
    let stream = connect_eventually(&dir).await;
    let requests = 40;
    let mut writer = stream.try_clone().unwrap();
    tokio::task::spawn_blocking(move || {
        for id in 0..requests {
            writeln!(
                writer,
                "{}",
                json!({ "jsonrpc": "2.0", "id": id, "method": "ping" })
            )
            .unwrap();
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(StdDuration::from_millis(300)).await;
    // The app stopped reading at the cap instead of spawning 40 handlers.
    assert_eq!(
        started.load(Ordering::SeqCst),
        channel::MAX_IN_FLIGHT_PER_CONNECTION
    );

    gate.add_permits(requests);
    let answered = tokio::task::spawn_blocking(move || {
        stream
            .set_read_timeout(Some(StdDuration::from_secs(5)))
            .unwrap();
        let mut ids: Vec<i64> = std::io::BufReader::new(stream)
            .lines()
            .take(requests)
            .map(|line| {
                serde_json::from_str::<Value>(&line.unwrap()).unwrap()["id"]
                    .as_i64()
                    .unwrap()
            })
            .collect();
        ids.sort_unstable();
        ids
    })
    .await
    .unwrap();
    assert_eq!(answered, (0..requests as i64).collect::<Vec<_>>());
    listener.stop();
}

#[test]
fn context_uses_the_local_calendar_across_daylight_saving_changes() {
    use chrono_tz::America::{New_York, Santiago};
    // New York springs forward on 2026-03-08 at 02:00: that day has 23 hours.
    let after = New_York.with_ymd_and_hms(2026, 3, 9, 0, 30, 0).unwrap();
    let context = protocol::context_json(after, Some("America/New_York".into()));
    assert_eq!(context["date"], json!("2026-03-09"));
    assert_eq!(context["yesterday"], json!("2026-03-08"));
    assert_eq!(context["startOfToday"], json!("2026-03-09T00:00:00-04:00"));

    let during = New_York.with_ymd_and_hms(2026, 3, 8, 12, 0, 0).unwrap();
    let context = protocol::context_json(during, None);
    assert_eq!(context["utcOffset"], json!("-04:00"));
    assert_eq!(context["startOfToday"], json!("2026-03-08T00:00:00-05:00"));

    // Santiago skips midnight on 2026-09-06: the day starts at 01:00.
    let skipped = Santiago.with_ymd_and_hms(2026, 9, 6, 12, 0, 0).unwrap();
    let context = protocol::context_json(skipped, None);
    assert_eq!(context["startOfToday"], json!("2026-09-06T01:00:00-03:00"));
}

#[tokio::test]
async fn malformed_json_rpc_objects_get_invalid_request_errors() {
    let data = data(notes_pool().await, ActivityAccess::Off);
    for (message, id) in [
        (json!({ "jsonrpc": "2.0", "id": 1 }), json!(1)),
        (json!({ "jsonrpc": "2.0", "id": 2, "method": 7 }), json!(2)),
        (json!({ "id": 3, "method": "ping" }), json!(3)),
        (
            json!({ "jsonrpc": "2.0", "id": { "nested": 1 }, "method": "ping" }),
            Value::Null,
        ),
        (
            json!([{ "jsonrpc": "2.0", "id": 4, "method": "ping" }]),
            Value::Null,
        ),
    ] {
        let response = protocol::handle(&data, &message)
            .await
            .unwrap_or_else(|| panic!("no answer to {message}"));
        assert_eq!(
            response["error"]["code"],
            json!(protocol::INVALID_REQUEST),
            "{message}"
        );
        assert_eq!(response["id"], id, "{message}");
    }
    let notification = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert!(protocol::handle(&data, &notification).await.is_none());
    let client_response = json!({ "jsonrpc": "2.0", "id": 5, "result": {} });
    assert!(protocol::handle(&data, &client_response).await.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_relay_skips_an_oversized_line_and_keeps_answering() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let oversized = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\",\"pad\":\"{}\"}}",
        "a".repeat(channel::MAX_LINE_BYTES)
    );
    let input = format!(
        "{oversized}\n{}\n{}\n",
        json!({ "jsonrpc": "2.0", "id": 2, "method": "ping" }),
        json!({ "jsonrpc": "2.0", "id": 3 }),
    );
    let output = Captured::default();
    let (relay_dir, sink) = (dir.clone(), output.clone());
    tokio::task::spawn_blocking(move || {
        stdio::relay(&relay_dir, std::io::Cursor::new(input), Box::new(sink));
    })
    .await
    .unwrap();
    let messages = output.messages(3);
    assert_eq!(messages[0]["id"], Value::Null);
    assert_eq!(
        messages[0]["error"]["code"],
        json!(protocol::INVALID_REQUEST)
    );
    assert_eq!(
        messages[0]["error"]["message"],
        json!(stdio::TOO_LONG_MESSAGE)
    );
    assert_eq!(by_id(&messages, 2)["result"], json!({}));
    assert_eq!(
        by_id(&messages, 3)["error"]["code"],
        json!(protocol::INVALID_REQUEST)
    );
}

#[test]
fn mcp_switch_and_activity_saves_never_undo_each_other() {
    use crate::activity::engine::ActivityShared;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("activity-settings.json");
    let shared = Arc::new(ActivityShared::new(ActivitySettings::default()));
    let toggles = {
        let (shared, path) = (Arc::clone(&shared), path.clone());
        std::thread::spawn(move || {
            for index in 0..200 {
                shared
                    .update_settings(&path, |settings| settings.mcp_server = index % 2 == 1)
                    .unwrap();
            }
        })
    };
    let saves = {
        let (shared, path) = (Arc::clone(&shared), path.clone());
        std::thread::spawn(move || {
            for index in 0..200u32 {
                // What `activity_save_settings` does with a stale request.
                shared
                    .update_settings(&path, |current| {
                        *current = ActivitySettings {
                            mcp_server: current.mcp_server,
                            retention_days: index % 30 + 1,
                            ..ActivitySettings::default()
                        };
                    })
                    .unwrap();
            }
        })
    };
    toggles.join().unwrap();
    saves.join().unwrap();
    let stored = crate::activity::settings::load(&path);
    assert!(shared.settings().mcp_server && stored.mcp_server);
    assert_eq!(shared.settings().retention_days, 20);
    assert_eq!(stored.retention_days, 20);
}

#[tokio::test]
async fn activity_belongs_to_the_installation_while_notes_follow_the_partition() {
    let (_dir, store) = activity_store().await;
    let pool = notes_pool().await;
    let range = json!({ "from": "2026-10-01T08:00:00Z", "to": "2026-10-01T12:00:00Z" });
    let open = || ActivityAccess::Open {
        store: store.clone(),
        settings: capture_on(),
    };
    let default = data(pool.clone(), open());
    let mut work = data(pool, open());
    work.profile = "work".into();
    assert_eq!(
        call(&default, "get_activity_timeline", range.clone()).await,
        call(&work, "get_activity_timeline", range).await
    );
    let notes = call(&work, "search_notes", json!({ "query": "zephyr" }))
        .await
        .unwrap();
    assert_eq!(notes["notes"][0]["title"], json!("Zephyr budget"));
    assert_eq!(notes["count"], json!(1));
}

#[tokio::test]
async fn coding_agent_sessions_list_enabled_sources_newest_first_within_retention() {
    use crate::coding_agents::settings::CodingAgentSources;
    use crate::coding_agents::store::NewBlock;
    use crate::coding_agents::SourceId;
    let (_dir, store) = activity_store().await;
    let block = |source, session: &str, start: DateTime<Utc>, minutes_long: i64, sealed| NewBlock {
        source,
        session_id: session.into(),
        started_at: start,
        ended_at: start + Duration::minutes(minutes_long),
        cwd: Some("/Users/me/os-clovy".into()),
        project: Some("os-clovy".into()),
        title: Some(format!("{session} title")),
        first_prompt: Some(format!("{session} first prompt")),
        prompt_count: 3,
        reply_count: 4,
        active_seconds: 600,
        transcript: "SECRET TRANSCRIPT".into(),
        sealed,
    };
    for new in [
        block(SourceId::ClaudeCode, "claude-1", at(7, 0), 40, true),
        block(SourceId::Codex, "codex-1", at(8, 30), 30, false),
        block(SourceId::Cursor, "cursor-1", at(8, 0), 20, true),
        block(
            SourceId::ClaudeCode,
            "claude-old",
            at(7, 0) - Duration::days(40),
            30,
            true,
        ),
    ] {
        store
            .upsert_coding_agent_block(&new, at(9, 0))
            .await
            .unwrap();
    }
    let claude = store
        .coding_agent_blocks_between(at(6, 0), at(8, 0))
        .await
        .unwrap()
        .into_iter()
        .find(|block| block.session_id == "claude-1")
        .unwrap();
    store
        .record_coding_agent_summary(claude.id, "Fixed the relay.", "cli:claude", at(9, 0))
        .await
        .unwrap();
    let settings = ActivitySettings {
        coding_agents: CodingAgentSources {
            claude_code: true,
            codex: true,
            ..CodingAgentSources::default()
        },
        ..capture_on()
    };
    let data = data(
        notes_pool().await,
        ActivityAccess::Open {
            store: store.clone(),
            settings,
        },
    );

    let listed = call(
        &data,
        "list_coding_agent_sessions",
        json!({ "from": "2026-08-01T00:00:00Z", "to": "2026-10-01T12:00:00Z" }),
    )
    .await
    .unwrap();
    let sessions: Vec<&str> = listed["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|block| block["sessionId"].as_str().unwrap())
        .collect();
    // Cursor is turned off and the 40-day-old block is past retention.
    assert_eq!(sessions, vec!["codex-1", "claude-1"], "{listed}");
    assert_eq!(listed["blocks"][0]["agent"], json!("Codex"));
    assert_eq!(listed["blocks"][0]["state"], json!("live"));
    assert_eq!(
        listed["blocks"][0]["firstPrompt"],
        json!("codex-1 first prompt")
    );
    assert_eq!(listed["blocks"][1]["summary"], json!("Fixed the relay."));
    assert_eq!(listed["blocks"][1]["firstPrompt"], Value::Null);
    assert_eq!(listed["blocks"][1]["activeMinutes"], json!(10.0));
    assert!(!listed.to_string().contains("SECRET TRANSCRIPT"));

    let limited = call(&data, "list_coding_agent_sessions", json!({ "limit": 1 }))
        .await
        .unwrap();
    assert_eq!(limited["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(limited["totalBlocks"], json!(2));
    assert_eq!(limited["truncated"], json!(true));
}

#[tokio::test]
async fn day_summary_returns_stored_summary_workstreams_and_reports_or_empty_state() {
    use crate::day_intelligence::db as day_db;
    use crate::day_intelligence::db::NewHourReport;
    use crate::day_intelligence::hour::{ActivityPeriod, HourActivity};
    use crate::day_intelligence::summary::{DaySummaryDto, Insight, Standup, SummaryTrigger};
    use crate::day_intelligence::workstreams::{Create, FoldPlan};

    let (_dir, store) = activity_store().await;
    let pool = notes_pool().await;
    let day = "2026-10-01";

    // Seed hour report
    day_db::insert_hour_report(
        &store,
        &NewHourReport {
            hour: "2026-10-01T09".into(),
            day: day.into(),
            started_at: at(9, 0),
            ended_at: at(10, 0),
            active_minutes: 45,
            summary: "Worked on Zephyr importer".into(),
            activities: vec![
                HourActivity {
                    description: "Coding Zephyr".into(),
                    minutes: 40,
                    seconds: Some(2400),
                    periods: Some(vec![ActivityPeriod {
                        started_at: "2026-10-01T09:05:00.000000Z".into(),
                        ended_at: "2026-10-01T09:45:00.000000Z".into(),
                    }]),
                },
                HourActivity {
                    description: "Reading mail".into(),
                    minutes: 5,
                    seconds: Some(300),
                    periods: None,
                },
            ],
            distilled: "distilled text".into(),
            distill_stats: json!({}),
            locale: "en".into(),
            provider: "test-provider".into(),
            generated_at: at(10, 0),
        },
    )
    .await
    .unwrap();

    // Seed workstream
    day_db::apply_fold(
        &store,
        day,
        "2026-10-01T09",
        &FoldPlan {
            creates: vec![Create {
                title: "Zephyr importer".into(),
                summary: "Scaffolding importer".into(),
                minutes: 45,
                seconds: 2700,
                note: "Initial scaffold".into(),
                periods: Some(vec![ActivityPeriod {
                    started_at: "2026-10-01T09:05:00.000000Z".into(),
                    ended_at: "2026-10-01T09:50:00.000000Z".into(),
                }]),
                located_minutes: 30,
                located_seconds: 1800,
            }],
            appends: vec![],
        },
        at(10, 0),
    )
    .await
    .unwrap();

    // Seed day summary
    let summary_dto = DaySummaryDto {
        day: day.into(),
        headline: "Shipped Zephyr importer and reviewed PRs".into(),
        narrative: "Spent the morning developing the importer and the afternoon testing.".into(),
        insights: vec![Insight {
            title: "Deep focus".into(),
            text: "Long uninterrupted blocks in Code.".into(),
        }],
        standup: Standup {
            done: vec!["Zephyr importer".into()],
            in_progress: vec!["Testing".into()],
            blockers: vec![],
        },
        hours_covered: 1,
        locale: "en".into(),
        provider: "test-provider".into(),
        trigger: SummaryTrigger::Scheduled,
        generated_at: "2026-10-01T18:00:00Z".into(),
    };
    day_db::save_summary(&store, &summary_dto).await.unwrap();

    let open_data = McpData {
        notes: pool.clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Open {
            store: store.clone(),
            settings: ActivitySettings {
                enabled: true,
                retention_days: 30,
                ..ActivitySettings::default()
            },
        },
        now: at(19, 0),
    };

    // 1. REAL tools/call for get_day_summary: asserting the summary fields come back
    let result = call(&open_data, "get_day_summary", json!({ "date": day }))
        .await
        .unwrap();
    assert_eq!(result["date"], json!(day));
    assert_eq!(result["state"], json!("ready"));
    let summary = &result["summary"];
    assert_eq!(
        summary["headline"],
        json!("Shipped Zephyr importer and reviewed PRs")
    );
    assert_eq!(
        summary["narrative"],
        json!("Spent the morning developing the importer and the afternoon testing.")
    );
    assert_eq!(summary["standup"]["done"], json!(["Zephyr importer"]));
    assert_eq!(summary["standup"]["inProgress"], json!(["Testing"]));
    assert_eq!(summary["insights"][0]["title"], json!("Deep focus"));
    assert_eq!(summary["hoursCovered"], json!(1));
    assert_eq!(summary["locale"], json!("en"));
    assert_eq!(summary["provider"], json!("test-provider"));
    assert_eq!(summary["trigger"], json!("scheduled"));

    let workstreams = result["workstreams"].as_array().unwrap();
    assert_eq!(workstreams.len(), 1);
    assert_eq!(workstreams[0]["title"], json!("Zephyr importer"));
    assert_eq!(workstreams[0]["minutes"], json!(45));
    assert_eq!(
        workstreams[0]["hours"][0]["periods"],
        json!([{"startedAt": "2026-10-01T09:05:00.000000Z", "endedAt": "2026-10-01T09:50:00.000000Z"}])
    );
    assert_eq!(workstreams[0]["hours"][0]["locatedMinutes"], json!(30));

    let reports = result["hourReports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["hour"], json!("2026-10-01T09"));
    assert_eq!(reports[0]["activeMinutes"], json!(45));
    // Each activity carries where it happened, or null when only the hour
    // is known.
    assert_eq!(
        reports[0]["activities"][0]["periods"],
        json!([{"startedAt": "2026-10-01T09:05:00.000000Z", "endedAt": "2026-10-01T09:45:00.000000Z"}])
    );
    assert_eq!(reports[0]["activities"][1]["periods"], Value::Null);

    // Never includes raw distilled text
    assert!(!result.to_string().contains("distilled text"));

    // 2. Default date (omitted date defaults to today)
    let default_result = call(&open_data, "get_day_summary", json!({}))
        .await
        .unwrap();
    let today_str = open_data
        .now
        .with_timezone(&Local)
        .format("%Y-%m-%d")
        .to_string();
    assert_eq!(default_result["date"], json!(today_str));

    // 3. No summary -> clear empty state
    let empty_result = call(
        &open_data,
        "get_day_summary",
        json!({ "date": "2026-09-30" }),
    )
    .await
    .unwrap();
    assert_eq!(empty_result["date"], json!("2026-09-30"));
    assert_eq!(empty_result["state"], json!("not_generated"));
    assert!(empty_result["summary"].is_null());
    assert!(empty_result["reason"].is_string());

    // 4. Capture off -> activity_capture_off
    let off_data = McpData {
        notes: pool.clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: at(19, 0),
    };
    let off_err = call(&off_data, "get_day_summary", json!({ "date": day }))
        .await
        .unwrap_err();
    assert!(off_err.contains("activity_capture_off"), "{off_err}");

    // 5. Invalid date -> isError
    let invalid_err = call(
        &open_data,
        "get_day_summary",
        json!({ "date": "not-a-date" }),
    )
    .await
    .unwrap_err();
    assert!(
        invalid_err.contains("mcp_invalid_arguments"),
        "{invalid_err}"
    );

    // 6. Respects retention (past retention returns empty state with past_retention)
    let past_result = call(
        &open_data,
        "get_day_summary",
        json!({ "date": "2026-08-01" }),
    )
    .await
    .unwrap();
    assert_eq!(past_result["state"], json!("past_retention"));
    assert!(past_result["summary"].is_null());
    assert!(past_result["workstreams"].as_array().unwrap().is_empty());
    assert!(past_result["hourReports"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn day_summary_preserves_exact_seconds_and_legacy_duration_fallbacks() {
    use crate::day_intelligence::db::{apply_fold, insert_hour_report, NewHourReport};
    use crate::day_intelligence::hour::{ActivityPeriod, HourActivity};
    use crate::day_intelligence::workstreams::{plan_fold, Placement};

    let (_dir, store) = activity_store().await;
    let day = "2026-10-01";
    let period = ActivityPeriod {
        started_at: "2026-10-01T09:00:00.000000Z".into(),
        ended_at: "2026-10-01T09:00:40.000000Z".into(),
    };
    let activities: Vec<HourActivity> = (1..=3)
        .map(|index| HourActivity {
            description: format!("Short task {index}"),
            minutes: 1,
            seconds: Some(40),
            periods: (index == 1).then(|| vec![period.clone()]),
        })
        .collect();
    for hour in [8, 9] {
        insert_hour_report(
            &store,
            &NewHourReport {
                hour: format!("{day}T{hour:02}"),
                day: day.into(),
                started_at: at(hour, 0),
                ended_at: at(hour + 1, 0),
                active_minutes: 2,
                summary: "Short tasks".into(),
                activities: if hour == 9 {
                    activities.clone()
                } else {
                    Vec::new()
                },
                distilled: String::new(),
                distill_stats: json!({}),
                locale: "en".into(),
                provider: "test-provider".into(),
                generated_at: at(10, 0),
            },
        )
        .await
        .unwrap();
    }
    // A released report has no seconds field. An explicit zero in a newer
    // report must not fall back to its rounded minutes.
    query("UPDATE day_hour_reports SET activities_json = ? WHERE hour = ?")
        .bind(
            json!([
                {"description": "Legacy task", "minutes": 2},
                {"description": "Explicit zero", "minutes": 1, "seconds": 0}
            ])
            .to_string(),
        )
        .bind(format!("{day}T08"))
        .execute(store.pool())
        .await
        .unwrap();
    let placements: Vec<Placement> = (1..=3)
        .map(|index| Placement {
            workstream_id: 0,
            title: format!("Short task {index}"),
            summary: String::new(),
            activities: vec![index],
        })
        .collect();
    apply_fold(
        &store,
        day,
        &format!("{day}T09"),
        &plan_fold(&[], &activities, &placements),
        at(10, 0),
    )
    .await
    .unwrap();
    let legacy_id = query(
        "INSERT INTO day_workstreams (day, title, summary, first_hour, last_hour, created_at, updated_at)
         VALUES (?, 'Legacy work', 'Legacy work', '2026-10-01T08', '2026-10-01T08', 'x', 'x')",
    )
    .bind(day)
    .execute(store.pool())
    .await
    .unwrap()
    .last_insert_rowid();
    query(
        "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note, periods_json)
         VALUES (?, '2026-10-01T08', 2, 'Legacy task', ?)",
    )
    .bind(legacy_id)
    .bind(json!({"periods": [period], "locatedMinutes": 1}).to_string())
    .execute(store.pool())
    .await
    .unwrap();

    let open = data(
        notes_pool().await,
        ActivityAccess::Open {
            store,
            settings: capture_on(),
        },
    );
    let summary = call(&open, "get_day_summary", json!({"date": day}))
        .await
        .unwrap();
    let reports = summary["hourReports"].as_array().unwrap();
    let modern = &reports[1];
    assert_eq!(modern["activeMinutes"], json!(2));
    let durations: Vec<_> = modern["activities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|activity| (activity["minutes"].clone(), activity["seconds"].clone()))
        .collect();
    assert_eq!(durations, vec![(json!(1), json!(40)); 3]);
    let legacy_activities = &reports[0]["activities"];
    assert_eq!(legacy_activities[0]["minutes"], json!(2));
    assert_eq!(legacy_activities[0]["seconds"], json!(120));
    assert_eq!(legacy_activities[1]["minutes"], json!(1));
    assert_eq!(legacy_activities[1]["seconds"], json!(0));

    let workstreams = summary["workstreams"].as_array().unwrap();
    let durations: Vec<_> = workstreams
        .iter()
        .map(|workstream| (workstream["minutes"].clone(), workstream["seconds"].clone()))
        .collect();
    assert_eq!(
        durations,
        vec![
            (json!(1), json!(40)),
            (json!(1), json!(40)),
            (json!(1), json!(40)),
            (json!(2), json!(120)),
        ]
    );
    let hours: Vec<_> = workstreams
        .iter()
        .map(|workstream| {
            let hour = &workstream["hours"][0];
            (
                hour["minutes"].clone(),
                hour["seconds"].clone(),
                hour["locatedMinutes"].clone(),
                hour["locatedSeconds"].clone(),
            )
        })
        .collect();
    assert_eq!(
        hours,
        vec![
            (json!(1), json!(40), json!(1), json!(40)),
            (json!(1), json!(40), json!(0), json!(0)),
            (json!(1), json!(40), json!(0), json!(0)),
            (json!(2), json!(120), json!(1), json!(60)),
        ]
    );

    let panels = call(&open, "get_day_panels", json!({"date": day}))
        .await
        .unwrap();
    let durations: Vec<_> = panels["panels"]["workstreams"]
        .as_array()
        .unwrap()
        .iter()
        .map(|workstream| (workstream["minutes"].clone(), workstream["seconds"].clone()))
        .collect();
    assert_eq!(
        durations,
        vec![
            (json!(1), json!(40)),
            (json!(1), json!(40)),
            (json!(1), json!(40)),
            (json!(2), json!(120)),
        ]
    );
}

#[tokio::test]
async fn day_panels_draw_the_day_from_filtered_activity_meetings_and_coding_agents() {
    use crate::coding_agents::settings::CodingAgentSources;
    use crate::coding_agents::store::NewBlock;
    use crate::coding_agents::SourceId;
    use crate::day_intelligence::db as day_db;
    use crate::day_intelligence::db::NewHourReport;
    use crate::day_intelligence::workstreams::{Create, FoldPlan};

    let (_dir, store) = activity_store().await;
    let pool = notes_pool().await;
    let local_day = at(9, 0).with_timezone(&Local).date_naive();
    let day = local_day.format("%Y-%m-%d").to_string();
    let nine = at(9, 0)
        .with_timezone(&Local)
        .format("%Y-%m-%dT%H")
        .to_string();

    // A 30-minute meeting in the open partition and one in another.
    for (id, note) in [
        ("rec-default", "note-roadmap"),
        ("rec-work", "note-other-profile"),
    ] {
        query("INSERT INTO recording_sessions (id, note_id, status, started_at, ended_at) VALUES (?, ?, 'completed', ?, ?)")
            .bind(id)
            .bind(note)
            .bind(at(8, 0).to_rfc3339())
            .bind(at(8, 30).to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
    }
    let block = |source, session: &str| NewBlock {
        source,
        session_id: session.into(),
        started_at: at(8, 40),
        ended_at: at(9, 0),
        cwd: None,
        project: Some("os-clovy".into()),
        title: None,
        first_prompt: None,
        prompt_count: 1,
        reply_count: 1,
        active_seconds: 600,
        transcript: String::new(),
        sealed: true,
    };
    for new in [
        block(SourceId::ClaudeCode, "claude-1"),
        block(SourceId::Cursor, "cursor-1"),
    ] {
        store
            .upsert_coding_agent_block(&new, at(9, 0))
            .await
            .unwrap();
    }
    // A workstream comes from folding an hour report.
    day_db::insert_hour_report(
        &store,
        &NewHourReport {
            hour: nine.clone(),
            day: day.clone(),
            started_at: at(9, 0),
            ended_at: at(10, 0),
            active_minutes: 7,
            summary: "Worked on Zephyr importer".into(),
            activities: vec![],
            distilled: String::new(),
            distill_stats: json!({}),
            locale: "en".into(),
            provider: "test-provider".into(),
            generated_at: at(9, 8),
        },
    )
    .await
    .unwrap();
    day_db::apply_fold(
        &store,
        &day,
        &nine,
        &FoldPlan {
            creates: vec![Create {
                title: "Zephyr importer".into(),
                summary: "Scaffolding importer".into(),
                minutes: 1,
                seconds: 40,
                note: "Initial scaffold".into(),
                periods: None,
                located_minutes: 0,
                located_seconds: 0,
            }],
            appends: vec![],
        },
        at(9, 8),
    )
    .await
    .unwrap();

    let settings = ActivitySettings {
        ignored_apps: vec!["1Password".into()],
        coding_agents: CodingAgentSources {
            claude_code: true,
            ..CodingAgentSources::default()
        },
        ..capture_on()
    };
    let open = |settings: ActivitySettings| ActivityAccess::Open {
        store: store.clone(),
        settings,
    };
    let data_open = data(pool.clone(), open(settings.clone()));

    let result = call(&data_open, "get_day_panels", json!({ "date": day }))
        .await
        .unwrap();
    assert_eq!(result["date"], json!(day));
    assert_eq!(result["state"], json!("ready"));
    let panels = &result["panels"];
    // 1Password is excluded: Code (5 min) and Firefox (2 min) remain.
    assert_eq!(panels["focusedMinutes"], json!(7.0), "{result}");
    let apps: Vec<&str> = panels["topApps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|app| app["app"].as_str().unwrap())
        .collect();
    assert_eq!(apps, vec!["Code", "Firefox"]);
    assert!(!panels["categories"].as_array().unwrap().is_empty());
    let hours = panels["hours"].as_array().unwrap();
    assert!(hours.len() >= 23, "{result}");
    let at_nine = hours
        .iter()
        .find(|hour| hour["hour"] == json!(nine))
        .unwrap();
    assert_eq!(at_nine["focusedMinutes"], json!(7.0));
    assert_eq!(
        panels["workstreams"],
        json!([{ "id": panels["workstreams"][0]["id"], "title": "Zephyr importer", "minutes": 1, "seconds": 40 }])
    );
    // Only the open partition's meeting; only the source turned on.
    assert_eq!(panels["meetings"], json!({ "count": 1, "minutes": 30.0 }));
    assert_eq!(
        panels["codingAgents"],
        json!({ "blocks": 1, "activeMinutes": 10.0 })
    );

    // Today by default.
    let today = call(&data_open, "get_day_panels", json!({})).await.unwrap();
    assert_eq!(today["date"], json!(day));

    // The retention floor cuts the day: activity and coding-agent blocks
    // before it are gone; a day wholly before it is past retention.
    let mut later = data(pool.clone(), open(settings.clone()));
    later.now = at(9, 8) + Duration::days(i64::from(settings.retention_days));
    let floored = call(&later, "get_day_panels", json!({ "date": day }))
        .await
        .unwrap();
    assert_eq!(floored["state"], json!("ready"));
    assert_eq!(floored["panels"]["focusedMinutes"], json!(0.0), "{floored}");
    assert_eq!(floored["panels"]["codingAgents"]["blocks"], json!(0));
    later.now += Duration::days(2);
    let past = call(&later, "get_day_panels", json!({ "date": day }))
        .await
        .unwrap();
    assert_eq!(past["state"], json!("past_retention"));
    assert_eq!(past["panels"], Value::Null);

    let invalid = call(
        &data_open,
        "get_day_panels",
        json!({ "date": "01/10/2026" }),
    )
    .await
    .unwrap_err();
    assert!(invalid.contains("mcp_invalid_arguments"), "{invalid}");
    let off = call(
        &data(pool, ActivityAccess::Off),
        "get_day_panels",
        json!({}),
    )
    .await
    .unwrap_err();
    assert!(off.contains("activity_capture_off"), "{off}");
}

#[tokio::test]
async fn day_schedule_reports_the_summary_time_and_notification_settings() {
    use crate::activity::settings::{ActivityNotificationSettings, DaySummarySettings, QuietHours};
    let (_dir, store) = activity_store().await;
    let pool = notes_pool().await;
    let settings = ActivitySettings {
        day_summary: DaySummarySettings {
            time: "07:30".into(),
        },
        notifications: ActivityNotificationSettings {
            enabled: false,
            quiet_hours: QuietHours {
                enabled: true,
                start: "23:00".into(),
                end: "07:00".into(),
            },
        },
        ..capture_on()
    };
    let open = data(
        pool.clone(),
        ActivityAccess::Open {
            store: store.clone(),
            settings,
        },
    );
    assert_eq!(
        call(&open, "get_day_schedule", json!({})).await.unwrap(),
        json!({
            "summaryTime": "07:30",
            "notifications": {
                "enabled": false,
                "quietHours": { "enabled": true, "start": "23:00", "end": "07:00" },
            },
        })
    );
    let off = call(
        &data(pool, ActivityAccess::Off),
        "get_day_schedule",
        json!({}),
    )
    .await
    .unwrap_err();
    assert!(off.contains("activity_capture_off"), "{off}");
}
