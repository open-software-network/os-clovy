use clovy_lib::agent_sessions::{
    import_legacy_agent_state, legacy_import_completed, AgentApprovalMode, AgentItemPayload,
    AgentRepository, LegacyImportOptions, MessagePayload, ToolPayload,
};
use clovy_lib::db::migrations::run_migrations;
use sqlx::{query::query, row::Row};
use sqlx_sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use std::str::FromStr;

async fn memory_database() -> SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory database");
    run_migrations(&pool).await.expect("migrations");
    pool
}

#[tokio::test]
async fn user_sessions_are_created_in_the_active_data_partition_atomically() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool.clone());
    let session = repository
        .create_session_in_profile(
            "Private chat",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
            "private",
        )
        .await
        .expect("session and partition mapping");

    let profile: String = query("SELECT profile FROM session_profiles WHERE session_id = ?")
        .bind(&session.id)
        .fetch_one(&pool)
        .await
        .expect("partition mapping")
        .get("profile");
    assert_eq!(profile, "private");
}

#[tokio::test]
async fn run_configuration_and_streamed_reasoning_survive_resume_and_hydration() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool.clone());
    let session = repository
        .create_session(
            "Durable run",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Unrestricted,
            None,
        )
        .await
        .expect("session");
    let run = repository
        .create_run(&session.id, "private-auto", Some("medium"))
        .await
        .expect("run");
    query("UPDATE agent_sessions SET safety_mode = 'sandboxed' WHERE id = ?")
        .bind(&session.id)
        .execute(&pool)
        .await
        .expect("change next run safety mode");
    let config = serde_json::json!({
        "model": "private-auto",
        "instructions": "Unattended routine instructions",
        "tools": [{ "name": "read_file" }],
        "skills": []
    });
    repository
        .set_run_config(&run.id, &config)
        .await
        .expect("persist run config");
    repository
        .set_run_config(
            &run.id,
            &serde_json::json!({ "instructions": "Changed after interruption", "approvalMode": "approvals" }),
        )
        .await
        .expect("ignore later run config mutation");
    repository
        .append_reasoning_delta(&session.id, &run.id, 1, "First ", "reasoning:run")
        .await
        .expect("first reasoning delta");
    repository
        .append_reasoning_delta(&session.id, &run.id, 2, "second", "reasoning:run")
        .await
        .expect("second reasoning delta");

    let stored = repository
        .run_config(&run.id)
        .await
        .expect("run config")
        .expect("stored config");
    for (key, value) in config.as_object().unwrap() {
        assert_eq!(&stored[key], value, "{key}");
    }
    assert_eq!(
        repository.get_run(&run.id).await.unwrap().approval_mode,
        AgentApprovalMode::Bypass
    );
    let next = repository
        .create_run(&session.id, "private-auto", Some("medium"))
        .await
        .expect("next run");
    assert_eq!(next.approval_mode, AgentApprovalMode::Approvals);
    let reasoning = repository.items(&session.id).await.expect("items");
    assert_eq!(reasoning.len(), 1);
    assert!(matches!(
        &reasoning[0].payload,
        AgentItemPayload::Reasoning(text) if text.text == "First second"
    ));
    assert_eq!(repository.get_run(&run.id).await.unwrap().last_sequence, 2);
}

#[tokio::test]
async fn streamed_assistant_text_survives_mid_run_hydration_without_duplicates() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool);
    let session = repository
        .create_session(
            "Visible active run",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .expect("session");
    let run = repository
        .create_run(&session.id, "private-auto", Some("medium"))
        .await
        .expect("run");
    let stream_id = format!("assistant:{}", run.id);

    repository
        .append_assistant_message_delta(&session.id, &run.id, 1, "What was ", &stream_id)
        .await
        .expect("first message delta");
    repository
        .append_assistant_message_delta(&session.id, &run.id, 2, "already said", &stream_id)
        .await
        .expect("second message delta");

    let partial = repository.items(&session.id).await.expect("partial items");
    assert_eq!(partial.len(), 1);
    assert_eq!(partial[0].external_id.as_deref(), Some(stream_id.as_str()));
    assert!(matches!(
        &partial[0].payload,
        AgentItemPayload::AssistantMessage(message)
            if message.content == "What was already said"
    ));

    repository
        .complete_assistant_message(
            &session.id,
            &run.id,
            3,
            "What was already said, plus the ending.",
            &stream_id,
        )
        .await
        .expect("completed message");

    let completed = repository
        .items(&session.id)
        .await
        .expect("completed items");
    assert_eq!(completed.len(), 1);
    assert_eq!(completed[0].id, partial[0].id);
    assert_eq!(completed[0].external_id, None);
    assert!(matches!(
        &completed[0].payload,
        AgentItemPayload::AssistantMessage(message)
            if message.content == "What was already said, plus the ending."
    ));
    assert_eq!(repository.get_run(&run.id).await.unwrap().last_sequence, 3);
}

#[tokio::test]
async fn completed_assistant_text_moves_behind_the_tools_that_produced_it() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool);
    let session = repository
        .create_session(
            "Tool ordering",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .expect("session");
    let run = repository
        .create_run(&session.id, "private-auto", Some("medium"))
        .await
        .expect("run");
    let stream_id = format!("assistant:{}", run.id);

    // Stream the start of the assistant response — this allocates the first
    // display sequence (0) and a low created_at.
    repository
        .append_assistant_message_delta(&session.id, &run.id, 1, "Here is ", &stream_id)
        .await
        .expect("streamed delta");

    let streamed = repository
        .items(&session.id)
        .await
        .expect("items after delta");
    assert_eq!(streamed.len(), 1);
    let assistant_row_id = streamed[0].id.clone();

    // Simulate back-dating: the delta row was created early, but tool call and
    // tool result arrive afterward and get higher display sequences.
    repository
        .append_item(
            &session.id,
            Some(&run.id),
            2,
            &AgentItemPayload::ToolCall(ToolPayload {
                error: None,
                tool_name: Some("read_file".into()),
                tool_call_id: Some("call-1".into()),
                arguments: Some(serde_json::json!({ "path": "notes.md" })),
                result: None,
                status: None,
            }),
            None,
        )
        .await
        .expect("tool call item")
        .expect("tool call inserted");

    repository
        .append_item(
            &session.id,
            Some(&run.id),
            3,
            &AgentItemPayload::ToolResult(ToolPayload {
                error: None,
                tool_name: Some("read_file".into()),
                tool_call_id: Some("call-1".into()),
                arguments: None,
                result: Some(serde_json::json!("file contents")),
                status: Some("ok".into()),
            }),
            None,
        )
        .await
        .expect("tool result item")
        .expect("tool result inserted");

    // Complete the assistant message — the fix moves the coalesced row to the
    // session-global tail so hydration places it after the tool rows.
    repository
        .complete_assistant_message(
            &session.id,
            &run.id,
            4,
            "Here is the final answer based on the file.",
            &stream_id,
        )
        .await
        .expect("completed message");

    let hydrated = repository.items(&session.id).await.expect("hydrated items");
    assert_eq!(
        hydrated.len(),
        3,
        "should have exactly one assistant row (reused) plus two tool rows"
    );

    // The assistant row must reuse the original id, not create a duplicate.
    assert_eq!(hydrated[2].id, assistant_row_id);

    // Order must be: tool call, tool result, final assistant response.
    assert!(matches!(
        &hydrated[0].payload,
        AgentItemPayload::ToolCall(tool) if tool.tool_name.as_deref() == Some("read_file")
    ));
    assert!(matches!(
        &hydrated[1].payload,
        AgentItemPayload::ToolResult(tool) if tool.tool_name.as_deref() == Some("read_file")
    ));
    assert!(matches!(
        &hydrated[2].payload,
        AgentItemPayload::AssistantMessage(msg)
            if msg.content == "Here is the final answer based on the file."
    ));
    assert_eq!(hydrated[2].external_id, None);
}

#[tokio::test]
async fn compaction_replaces_old_items_with_one_ordered_visible_summary() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool.clone());
    let session = repository
        .create_session(
            "Compaction",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .expect("session");
    let run = repository
        .create_run(&session.id, "private-auto", Some("medium"))
        .await
        .expect("run");
    let first = repository
        .append_item(
            &session.id,
            None,
            0,
            &AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: "Old question".into(),
                attachments: vec![],
            }),
            None,
        )
        .await
        .expect("first item")
        .expect("first inserted");
    let second = repository
        .append_item(
            &session.id,
            None,
            0,
            &AgentItemPayload::AssistantMessage(MessagePayload {
                role: "assistant".into(),
                content: "Old answer".into(),
                attachments: vec![],
            }),
            None,
        )
        .await
        .expect("second item")
        .expect("second inserted");
    let recent = repository
        .append_item(
            &session.id,
            None,
            0,
            &AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: "Recent question".into(),
                attachments: vec![],
            }),
            None,
        )
        .await
        .expect("recent item")
        .expect("recent inserted");

    let summary = repository
        .replace_items_with_context_summary(
            &session.id,
            &run.id,
            "Earlier conversation context",
            None,
            &[first.id.clone(), second.id.clone()],
        )
        .await
        .expect("compaction")
        .expect("summary");

    let items = repository.items(&session.id).await.expect("items");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, summary.id);
    assert_eq!(items[0].sequence, first.sequence);
    assert_eq!(items[1].id, recent.id);
    assert!(matches!(
        &items[0].payload,
        AgentItemPayload::ContextSummary(text) if text.text == "Earlier conversation context"
    ));
    assert!(repository
        .replace_items_with_context_summary(
            &session.id,
            &run.id,
            "Duplicate",
            None,
            &[first.id, second.id],
        )
        .await
        .expect("idempotent replay")
        .is_none());
}

#[tokio::test]
async fn run_skills_are_deduplicated_and_persisted_for_retry_and_resume() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool);
    let session = repository
        .create_session(
            "Skills",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .expect("session");
    let run = repository
        .create_run(&session.id, "private-auto", Some("medium"))
        .await
        .expect("run");

    repository
        .set_run_enabled_skills(
            &run.id,
            &[
                "notes".to_string(),
                "notes".to_string(),
                "research".to_string(),
            ],
        )
        .await
        .expect("persist skills");

    assert_eq!(
        repository
            .run_enabled_skills(&run.id)
            .await
            .expect("load skills"),
        vec!["notes".to_string(), "research".to_string()]
    );
}

#[tokio::test]
async fn obsidian_run_skill_ids_are_stored_under_the_clovy_id() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool.clone());
    let session = repository
        .create_session(
            "Obsidian skills",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .expect("session");
    let run = repository
        .create_run(&session.id, "private-auto", None)
        .await
        .expect("run");

    repository
        .set_run_enabled_skills(&run.id, &["clovy-obsidian".to_string()])
        .await
        .expect("persist canonical skill");

    let stored: String = query("SELECT enabled_skills_json FROM agent_runs WHERE id = ?")
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .expect("stored skills")
        .get("enabled_skills_json");
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&stored).expect("stored skill ids"),
        vec!["clovy-obsidian".to_string()]
    );
    assert_eq!(
        repository
            .run_enabled_skills(&run.id)
            .await
            .expect("current skill ids"),
        vec!["clovy-obsidian".to_string()]
    );

    query("UPDATE agent_runs SET enabled_skills_json = ? WHERE id = ?")
        .bind(r#"["june-obsidian"]"#)
        .bind(&run.id)
        .execute(&pool)
        .await
        .expect("simulate legacy run");
    assert_eq!(
        repository
            .run_enabled_skills(&run.id)
            .await
            .expect("migrate legacy run"),
        vec!["clovy-obsidian".to_string()]
    );
    let migrated: String = query("SELECT enabled_skills_json FROM agent_runs WHERE id = ?")
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .expect("migrated skills")
        .get("enabled_skills_json");
    assert_eq!(
        serde_json::from_str::<Vec<String>>(&migrated).expect("migrated skill ids"),
        vec!["clovy-obsidian".to_string()]
    );
}

#[tokio::test]
async fn a_june_era_obsidian_setting_is_read_once_into_the_clovy_id() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool.clone());
    query("INSERT INTO agent_skill_settings(skill_id, enabled, managed, updated_at) VALUES (?, 0, 1, 'legacy-time')")
        .bind("june-obsidian")
        .execute(&pool)
        .await
        .expect("June-era setting");

    let settings = repository.skills().await.expect("reconciled settings");
    let clovy = settings
        .iter()
        .find(|setting| setting.id == "clovy-obsidian")
        .expect("Clovy setting");
    assert!(!clovy.enabled, "the June-era disabled state carries over");

    repository
        .set_skill_enabled("clovy-obsidian", true, true)
        .await
        .expect("enable Clovy skill");
    let legacy_enabled: i64 =
        query("SELECT enabled FROM agent_skill_settings WHERE skill_id = 'june-obsidian'")
            .fetch_one(&pool)
            .await
            .expect("June-era row")
            .get("enabled");
    assert_eq!(legacy_enabled, 0, "the June-era row is never written");
}

#[tokio::test]
async fn resumed_run_rebases_process_local_event_sequence() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool);
    let session = repository
        .create_session(
            "Resume",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let run = repository
        .create_run(&session.id, "private-auto", None)
        .await
        .unwrap();
    repository
        .append_item(
            &session.id,
            Some(&run.id),
            42,
            &AgentItemPayload::AssistantMessage(MessagePayload {
                role: "assistant".into(),
                content: "Before pause".into(),
                attachments: vec![],
            }),
            Some("old-process-event"),
        )
        .await
        .unwrap()
        .unwrap();

    repository
        .reset_run_sequence_for_resume(&run.id)
        .await
        .unwrap();
    let resumed = repository
        .append_item(
            &session.id,
            Some(&run.id),
            1,
            &AgentItemPayload::AssistantMessage(MessagePayload {
                role: "assistant".into(),
                content: "After resume".into(),
                attachments: vec![],
            }),
            Some("new-process-event"),
        )
        .await
        .unwrap();

    assert!(resumed.is_some());
    assert_eq!(repository.get_run(&run.id).await.unwrap().last_sequence, 1);
}

#[tokio::test]
async fn restart_interrupts_ordinary_active_runs_but_preserves_waiting_state() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool);
    let running_session = repository
        .create_session(
            "Running",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let running = repository
        .create_run(&running_session.id, "private-auto", None)
        .await
        .unwrap();
    let waiting_session = repository
        .create_session(
            "Waiting",
            "private-auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let waiting = repository
        .create_run(&waiting_session.id, "private-auto", None)
        .await
        .unwrap();
    repository
        .update_run_status(
            &waiting.id,
            "waiting_for_user",
            None,
            Some(&serde_json::json!("serialized")),
            None,
        )
        .await
        .unwrap();

    assert_eq!(
        repository
            .reconcile_non_routine_runs_after_restart()
            .await
            .unwrap(),
        1
    );

    assert_eq!(
        repository.get_run(&running.id).await.unwrap().status,
        "interrupted"
    );
    assert_eq!(
        repository
            .get_session(&running_session.id)
            .await
            .unwrap()
            .status,
        "interrupted"
    );
    let waiting = repository.get_run(&waiting.id).await.unwrap();
    assert_eq!(waiting.status, "waiting_for_user");
    assert_eq!(
        waiting.interrupted_state,
        Some(serde_json::json!("serialized"))
    );
    assert_eq!(
        repository
            .get_session(&waiting_session.id)
            .await
            .unwrap()
            .status,
        "waiting_for_user"
    );
    let reconciled_session = repository.get_session(&running_session.id).await.unwrap();
    assert_eq!(
        repository
            .reconcile_non_routine_runs_after_restart()
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        repository
            .get_session(&running_session.id)
            .await
            .unwrap()
            .updated_at,
        reconciled_session.updated_at
    );
}

#[tokio::test]
async fn runtime_schema_replaces_legacy_tables_and_keeps_folder_assignments() {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("memory database");
    run_migrations(&pool).await.expect("current schema");
    for statement in [
        "DROP TABLE routine_runs",
        "DROP TABLE routines",
        "DROP TABLE agent_run_mcp_policies",
        "DROP TABLE agent_mcp_servers",
        "DROP TABLE session_folders",
        "DROP TABLE agent_artifacts",
        "DROP TABLE agent_items",
        "DROP TABLE agent_runs",
        "DROP TABLE agent_skill_settings",
        "DROP TABLE agent_migration_manifests",
        "DROP TABLE agent_sessions",
        "DELETE FROM schema_migrations WHERE version >= 32",
    ] {
        query(statement)
            .execute(&pool)
            .await
            .expect("restore pre-runtime schema");
    }
    for migration in [
        include_str!("../migrations/007_agent.sql"),
        include_str!("../migrations/009_session_folders.sql"),
    ] {
        for statement in migration
            .split(';')
            .map(str::trim)
            .filter(|sql| !sql.is_empty())
        {
            query(statement)
                .execute(&pool)
                .await
                .expect("legacy schema");
        }
    }
    query("ALTER TABLE agent_tasks ADD COLUMN hermes_session_id TEXT")
        .execute(&pool)
        .await
        .expect("legacy Hermes identity column");
    query("ALTER TABLE agent_messages ADD COLUMN external_id TEXT")
        .execute(&pool)
        .await
        .expect("legacy external identity column");
    query(
        "INSERT INTO folders (id, name, created_at, updated_at) VALUES ('folder-1', 'Work', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("folder");
    query(
        "INSERT INTO session_folders (session_id, folder_id, assigned_at) VALUES ('hermes-1', 'folder-1', '2026-01-02T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("folder assignment");
    query(
        "INSERT INTO agent_tasks
         (id, title, prompt, status, safety_profile, created_at, updated_at, hermes_session_id)
         VALUES ('task-1', 'Task title', 'Prompt', 'completed', 'autonomous_private',
                 '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', 'hermes-1')",
    )
    .execute(&pool)
    .await
    .expect("legacy task");
    query(
        "INSERT INTO agent_messages (id, task_id, role, content, created_at)
         VALUES ('message-1', 'task-1', 'user', 'Hello', '2026-01-01T00:00:00Z')",
    )
    .execute(&pool)
    .await
    .expect("legacy message");

    run_migrations(&pool).await.expect("runtime migration");

    let assignment = query("SELECT session_id FROM session_folders")
        .fetch_one(&pool)
        .await
        .expect("preserved assignment");
    assert_eq!(assignment.get::<String, _>("session_id"), "hermes-1");
    let session = query("SELECT title, source FROM agent_sessions WHERE id = 'hermes-1'")
        .fetch_one(&pool)
        .await
        .expect("imported session");
    assert_eq!(session.get::<String, _>("title"), "Task title");
    assert_eq!(session.get::<String, _>("source"), "legacy_agent_task");
    let content: String =
        query("SELECT payload_json FROM agent_items WHERE session_id = 'hermes-1'")
            .fetch_one(&pool)
            .await
            .expect("imported message")
            .get("payload_json");
    assert!(content.contains("Hello"));
    let legacy_count: i64 = query(
        "SELECT COUNT(*) AS count FROM sqlite_master
         WHERE type = 'table' AND name IN ('agent_tasks', 'agent_messages', 'agent_tool_events')",
    )
    .fetch_one(&pool)
    .await
    .expect("legacy table check")
    .get("count");
    assert_eq!(legacy_count, 0);
}

#[tokio::test]
async fn legacy_import_is_read_only_idempotent_and_filters_delegated_sessions() {
    let destination = memory_database().await;
    let directory = tempfile::tempdir().expect("temp directory");
    let source_path = directory.path().join("state.db");
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", source_path.display()))
        .expect("source options")
        .create_if_missing(true);
    let source = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .expect("source database");
    query(
        "CREATE TABLE sessions (
           id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT, title TEXT,
           parent_session_id TEXT, started_at REAL NOT NULL, ended_at REAL, end_reason TEXT
         )",
    )
    .execute(&source)
    .await
    .expect("sessions");
    query(
        "CREATE TABLE messages (
           id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL,
           content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT,
           timestamp REAL NOT NULL, reasoning TEXT, reasoning_content TEXT, active INTEGER
         )",
    )
    .execute(&source)
    .await
    .expect("messages");
    for statement in [
        "INSERT INTO sessions VALUES ('user-1', 'cli', 'model-a', 'User session', NULL, 1000, 1002, NULL)",
        "INSERT INTO sessions VALUES ('daily-brief', 'cron', 'model-b', 'Daily run', NULL, 2000, 2002, NULL)",
        "INSERT INTO sessions VALUES ('child-1', 'subagent', 'model-a', 'Delegate', NULL, 3000, 3002, NULL)",
        "INSERT INTO sessions VALUES ('split-1', 'cli', 'model-a', 'Compressed child', 'user-1', 4000, 4002, NULL)",
        "INSERT INTO messages VALUES (1, 'user-1', 'user', 'Question', NULL, NULL, NULL, 1000, NULL, NULL, 1)",
        "INSERT INTO messages VALUES (2, 'user-1', 'assistant', 'Answer', NULL, NULL, NULL, 1001, 'Thought', NULL, 1)",
        "INSERT INTO messages VALUES (3, 'daily-brief', 'assistant', 'Routine result', NULL, NULL, NULL, 2001, NULL, NULL, 1)",
        "INSERT INTO messages VALUES (4, 'child-1', 'assistant', 'Child result', NULL, NULL, NULL, 3001, NULL, NULL, 1)",
        "INSERT INTO messages VALUES (5, 'split-1', 'assistant', 'Split result', NULL, NULL, NULL, 4001, NULL, NULL, 1)",
    ] {
        query(statement).execute(&source).await.expect("fixture row");
    }
    source.close().await;
    std::fs::create_dir_all(directory.path().join("cron")).expect("cron directory");
    std::fs::write(
        directory.path().join("cron/jobs.json"),
        r#"{
          "jobs": [{
            "id": "daily-brief",
            "name": "Daily brief",
            "prompt": "Summarize my recent notes.",
            "schedule": {"kind":"cron","expr":"0 9 * * *","display":"0 9 * * *"},
            "repeat": {"times":null,"completed":4},
            "enabled": true,
            "state": "scheduled",
            "created_at": "2026-01-01T00:00:00Z",
            "next_run_at": "2026-07-25T13:00:00Z",
            "last_run_at": "2026-07-24T13:00:00Z",
            "last_status": "ok",
            "deliver": "local",
            "enabled_toolsets": ["web"],
            "script": "echo preserved-routine-output",
            "no_agent": true
          }]
        }"#,
    )
    .expect("legacy routines");
    std::fs::write(
        directory.path().join("config.yaml"),
        r#"mcp_servers:
  june_context:
    command: python
    args: [managed.py]
  todo:
    enabled: true
    command: node
    args: [server.js]
    tools:
      include: [list_tasks]
"#,
    )
    .expect("legacy MCP config");
    let source_bytes_before = std::fs::read(&source_path).expect("source bytes");

    let options = LegacyImportOptions {
        hermes_state_db: source_path.clone(),
        hermes_home: Some(directory.path().to_path_buf()),
        artifact_root: Some(directory.path().join("artifacts")),
    };
    let first = import_legacy_agent_state(&destination, &options)
        .await
        .expect("first import");
    let second = import_legacy_agent_state(&destination, &options)
        .await
        .expect("idempotent import");

    assert_eq!(first.imported_counts.sessions, 2);
    assert_eq!(first.imported_counts.routines, 1);
    assert_eq!(first.imported_counts.mcp_servers, 1);
    assert_eq!(second, first);
    let sessions = AgentRepository::new(destination.clone())
        .list_sessions()
        .await
        .expect("sessions");
    assert!(sessions.iter().any(|session| session.id == "user-1"));
    assert!(sessions
        .iter()
        .any(|session| session.id == "daily-brief" && session.source == "legacy_routine"));
    assert!(!sessions.iter().any(|session| session.id == "child-1"));
    assert!(!sessions.iter().any(|session| session.id == "split-1"));
    let items = AgentRepository::new(destination.clone())
        .items("user-1")
        .await
        .expect("items");
    assert!(items.iter().any(|item| matches!(
        &item.payload,
        AgentItemPayload::AssistantMessage(MessagePayload { content, .. })
        if content == "Answer"
    )));
    assert!(items
        .iter()
        .any(|item| matches!(&item.payload, AgentItemPayload::Reasoning(_))));
    let routine_items = AgentRepository::new(destination.clone())
        .items("daily-brief")
        .await
        .expect("routine history");
    assert!(routine_items.iter().any(|item| matches!(
        &item.payload,
        AgentItemPayload::AssistantMessage(MessagePayload { content, .. }) if content == "Routine result"
    )));
    let routine = query(
        "SELECT state, enabled, next_run_at, metadata_json FROM routines WHERE id = 'daily-brief'",
    )
    .fetch_one(&destination)
    .await
    .expect("imported routine");
    assert_eq!(routine.get::<String, _>("state"), "needs_review");
    assert_eq!(routine.get::<i64, _>("enabled"), 0);
    assert!(routine.get::<Option<String>, _>("next_run_at").is_none());
    let routine_metadata: serde_json::Value =
        serde_json::from_str(&routine.get::<String, _>("metadata_json")).expect("routine metadata");
    assert_eq!(
        routine_metadata["legacyScript"],
        "echo preserved-routine-output"
    );
    assert_eq!(routine_metadata["legacyScriptExecution"], "needs_review");
    let mcp_count: i64 =
        query("SELECT COUNT(*) AS count FROM agent_mcp_servers WHERE name = 'todo'")
            .fetch_one(&destination)
            .await
            .expect("imported MCP server")
            .get("count");
    assert_eq!(mcp_count, 1);
    let managed_mcp_count: i64 =
        query("SELECT COUNT(*) AS count FROM agent_mcp_servers WHERE name = 'june_context'")
            .fetch_one(&destination)
            .await
            .expect("managed MCP server")
            .get("count");
    assert_eq!(managed_mcp_count, 0);
    assert_eq!(
        std::fs::read(&source_path).expect("source bytes after"),
        source_bytes_before
    );
}

#[tokio::test]
async fn legacy_import_recovers_routines_and_mcp_when_state_database_is_missing() {
    let destination = memory_database().await;
    let directory = tempfile::tempdir().expect("temp directory");
    std::fs::create_dir_all(directory.path().join("cron")).expect("cron directory");
    std::fs::write(
        directory.path().join("cron/jobs.json"),
        r#"[{
          "id": "companion-routine",
          "name": "Companion routine",
          "prompt": "Summarize notes.",
          "schedule": {"kind":"cron","expr":"0 9 * * *","timezone":"America/New_York"},
          "enabled": false,
          "state": "paused"
        }]"#,
    )
    .expect("routine companion");
    std::fs::write(
        directory.path().join("config.yaml"),
        "mcp_servers:\n  docs:\n    command: node\n    args: [server.js]\n",
    )
    .expect("MCP companion");
    let source_path = directory.path().join("missing-state.db");
    let options = LegacyImportOptions {
        hermes_state_db: source_path.clone(),
        hermes_home: Some(directory.path().to_path_buf()),
        artifact_root: None,
    };

    let first = import_legacy_agent_state(&destination, &options)
        .await
        .expect("companion import");
    let second = import_legacy_agent_state(&destination, &options)
        .await
        .expect("idempotent companion import");

    assert_eq!(first.status, "completed");
    assert_eq!(first.imported_counts.sessions, 0);
    assert_eq!(first.imported_counts.routines, 1);
    assert_eq!(first.imported_counts.mcp_servers, 1);
    assert_eq!(second, first);
    let timezone: String = query("SELECT timezone FROM routines WHERE id = 'companion-routine'")
        .fetch_one(&destination)
        .await
        .expect("imported routine")
        .get("timezone");
    assert_eq!(timezone, "America/New_York");
    assert!(!source_path.exists());
}

#[tokio::test]
async fn legacy_import_does_not_touch_secrets_for_a_duplicate_mcp_server() {
    let destination = memory_database().await;
    query(
        "INSERT INTO agent_mcp_servers
         (id, name, enabled, transport, command, args_json, secret_ref,
          metadata_json, tool_visibility_json, safety_json, created_at, updated_at)
         VALUES ('existing-id', 'docs', 1, 'stdio', 'existing-command', '[]',
                 'existing-secret', '{}', '{}', '{}', '2026-01-01', '2026-01-01')",
    )
    .execute(&destination)
    .await
    .expect("existing MCP server");
    let directory = tempfile::tempdir().expect("temporary migration directory");
    std::fs::write(
        directory.path().join("config.yaml"),
        "mcp_servers:\n  docs:\n    command: replacement-command\n    env:\n      TOKEN: must-not-be-staged\n",
    )
    .expect("legacy MCP config");
    let options = LegacyImportOptions {
        hermes_state_db: directory.path().join("missing-state.db"),
        hermes_home: Some(directory.path().to_path_buf()),
        artifact_root: None,
    };

    let manifest = import_legacy_agent_state(&destination, &options)
        .await
        .expect("duplicate definition is skipped without secure storage access");

    assert_eq!(manifest.imported_counts.mcp_servers, 0);
    assert_eq!(manifest.skipped_count, 1);
    let existing = query("SELECT command, secret_ref FROM agent_mcp_servers WHERE name = 'docs'")
        .fetch_one(&destination)
        .await
        .expect("existing MCP server remains");
    assert_eq!(existing.get::<String, _>("command"), "existing-command");
    assert_eq!(
        existing.get::<Option<String>, _>("secret_ref").as_deref(),
        Some("existing-secret")
    );
}

#[tokio::test]
async fn legacy_script_routine_is_copied_and_disabled_until_review() {
    let destination = memory_database().await;
    let directory = tempfile::tempdir().expect("temporary migration directory");
    let legacy_home = directory.path().join("legacy-home");
    let script_path = legacy_home.join("scripts").join("nightly.sh");
    std::fs::create_dir_all(script_path.parent().expect("script parent"))
        .expect("legacy script directory");
    std::fs::write(&script_path, "#!/bin/sh\necho preserved-output\n").expect("legacy script");
    std::fs::create_dir_all(legacy_home.join("cron")).expect("legacy cron directory");
    let jobs = serde_json::json!([{
        "id": "scripted-routine",
        "name": "Nightly cleanup",
        "prompt": "Run cleanup.",
        "schedule": {"kind": "cron", "expr": "0 2 * * *", "timezone": "America/New_York"},
        "enabled": true,
        "state": "scheduled",
        "script": script_path.to_string_lossy(),
        "no_agent": true,
        "last_error": "legacy failure detail"
    }]);
    std::fs::write(legacy_home.join("cron").join("jobs.json"), jobs.to_string())
        .expect("legacy routine");
    let storage_root = directory.path().join("june-owned-storage");
    let options = LegacyImportOptions {
        hermes_state_db: legacy_home.join("state.db"),
        hermes_home: Some(legacy_home.clone()),
        artifact_root: Some(storage_root.clone()),
    };

    let first = import_legacy_agent_state(&destination, &options)
        .await
        .expect("script routine import");
    assert_eq!(first.imported_counts.routines, 1);
    assert!(legacy_import_completed(&destination)
        .await
        .expect("completed manifest"));
    let row = query(
        "SELECT state, enabled, next_run_at, safety_mode, timezone, last_error, metadata_json
         FROM routines WHERE id = 'scripted-routine'",
    )
    .fetch_one(&destination)
    .await
    .expect("imported script routine");
    assert_eq!(row.get::<String, _>("state"), "needs_review");
    assert_eq!(row.get::<i64, _>("enabled"), 0);
    assert!(row.get::<Option<String>, _>("next_run_at").is_none());
    assert_eq!(row.get::<String, _>("safety_mode"), "sandboxed");
    assert_eq!(row.get::<String, _>("timezone"), "America/New_York");
    assert!(row
        .get::<String, _>("last_error")
        .contains("require review"));
    let metadata: serde_json::Value =
        serde_json::from_str(&row.get::<String, _>("metadata_json")).expect("routine metadata");
    assert_eq!(
        metadata["legacyScript"],
        serde_json::Value::String(script_path.to_string_lossy().into_owned())
    );
    assert_eq!(metadata["legacyScriptExecution"], "needs_review");
    assert_eq!(metadata["legacyNoAgent"], true);
    assert_eq!(metadata["legacyLastError"], "legacy failure detail");
    let copied_path = metadata["legacyScriptStoredPath"]
        .as_str()
        .map(std::path::PathBuf::from)
        .expect("June-owned script copy");
    assert!(copied_path.starts_with(&storage_root));
    assert_eq!(
        std::fs::read_to_string(&copied_path).expect("copied script contents"),
        "#!/bin/sh\necho preserved-output\n"
    );

    // A completed manifest is checked before any legacy-home filesystem work.
    // Removing the old home therefore cannot erase the imported source or make
    // a later app launch depend on the retired runtime.
    std::fs::remove_dir_all(&legacy_home).expect("remove retired home");
    let second = import_legacy_agent_state(&destination, &options)
        .await
        .expect("completed import uses June manifest only");
    assert_eq!(second, first);
    assert!(copied_path.is_file());
}

#[tokio::test]
async fn native_waiting_runs_are_cancelled_without_losing_history_or_cli_runs() {
    let pool = memory_database().await;
    let repository = AgentRepository::new(pool.clone());
    let native = repository
        .create_session(
            "Old conversation",
            "open-software/auto",
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let run = repository
        .create_run(&native.id, "open-software/auto", None)
        .await
        .unwrap();
    repository
        .append_item(
            &native.id,
            Some(&run.id),
            1,
            &AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: "Preserved question".into(),
                attachments: vec![],
            }),
            Some("old-question"),
        )
        .await
        .unwrap();
    repository.append_item(&native.id, Some(&run.id), 2, &AgentItemPayload::Interruption(serde_json::json!({"id":"old-request", "status":"pending", "secretRef":"opaque-reference"})), Some("old-request")).await.unwrap();
    repository
        .update_run_status(
            &run.id,
            "waiting_for_user",
            None,
            Some(&serde_json::json!({"legacy":true})),
            None,
        )
        .await
        .unwrap();
    let cli_model = clovy_lib::chat_engine::engine_model_id(clovy_lib::llm::cli::CliKind::Claude);
    let cli = repository
        .create_session(
            "CLI conversation",
            &cli_model,
            clovy_lib::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let cli_run = repository
        .create_run(&cli.id, &cli_model, None)
        .await
        .unwrap();
    let items_before = repository.items(&native.id).await.unwrap();
    // Rewind the synthetic stamp to a contiguous pre-retirement prefix.
    // Later migrations must be replayed too, rather than leaving a gap at 52.
    query("DELETE FROM schema_migrations WHERE version >= 52")
        .execute(&pool)
        .await
        .unwrap();
    run_migrations(&pool).await.unwrap();
    let retired = repository.get_run(&run.id).await.unwrap();
    assert_eq!(retired.status, "cancelled");
    assert!(retired.interrupted_state.is_none());
    assert_eq!(
        repository.get_session(&native.id).await.unwrap().status,
        "idle"
    );
    assert_eq!(
        repository.get_run(&cli_run.id).await.unwrap().status,
        "running"
    );
    let items = repository.items(&native.id).await.unwrap();
    assert_eq!(items.len(), items_before.len());
    assert_eq!(items[0].id, items_before[0].id);
    assert!(
        matches!(&items[0].payload, AgentItemPayload::UserMessage(message) if message.content == "Preserved question")
    );
    assert!(
        matches!(&items[1].payload, AgentItemPayload::Interruption(value) if value["status"] == "cancelled")
    );
    let secret: String = query("SELECT secret_ref FROM agent_secret_retirements")
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("secret_ref");
    assert_eq!(secret, "opaque-reference");
    run_migrations(&pool).await.unwrap();
    assert_eq!(
        repository.get_run(&run.id).await.unwrap().completed_at,
        retired.completed_at
    );
}
