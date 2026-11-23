use super::{call, ActivityAccess, McpData, ReadTools, ToolCtx, ToolGroup, TurnBinding};
use crate::approval_broker::testing::TestBroker;
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;
use crate::mcp_server::tests::{activity_store, data, notes_pool};
use serde_json::{json, Value};
use sqlx::query::query;

async fn fixture() -> (McpData, TurnBinding) {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, false).await;
    let pool = notes_pool().await;
    query("INSERT INTO agent_sessions (id,title,model,status,created_at,updated_at) VALUES (?, 'Routine read fixture', 'test', 'running', 'now','now')")
        .bind(&binding.session_id).execute(&pool).await.unwrap();
    query("INSERT INTO agent_runs (id,session_id,model,status,started_at,updated_at,run_config_json) VALUES (?,?,'test','running','now','now','{\"dataPartition\":\"default\"}')")
        .bind(&binding.run_id).bind(&binding.session_id).execute(&pool).await.unwrap();
    let now = "2026-10-28T00:00:00Z";
    query("INSERT INTO routines (id, name, prompt, schedule, created_at, updated_at, metadata_json, tool_catalog_version) VALUES ('read-routine', 'Daily', 'Summarise', 'every 1d', ?, ?, '{\"enabledToolsets\":[]}', 1)")
        .bind(now).bind(now).execute(&pool).await.unwrap();
    query("INSERT INTO routine_runs (id, routine_id, agent_session_id, claim_token, trigger_kind, status, model, safety_mode, created_at, updated_at) VALUES ('read-routine-run', 'read-routine', ?, 'claim', 'schedule', 'running', 'claude', 'sandboxed', ?, ?)")
        .bind(&binding.session_id).bind(now).bind(now).execute(&pool).await.unwrap();
    (data(pool, ActivityAccess::On), binding)
}

async fn set_toolsets(data: &McpData, toolsets: &[&str]) {
    query("UPDATE routines SET metadata_json = ? WHERE id = 'read-routine'")
        .bind(json!({"enabledToolsets": toolsets}).to_string())
        .execute(&data.notes)
        .await
        .unwrap();
}

fn names(ctx: &ToolCtx<'_>) -> Vec<String> {
    ReadTools
        .descriptors(ctx)
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn routine_read_catalog_obeys_each_context_toolset_and_rechecks_revocation() {
    let (data, binding) = fixture().await;
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let expected = names(&ctx);
    assert_eq!(expected.len(), 15);
    assert!(ReadTools.catalog(&ctx).await.is_empty());
    set_toolsets(&data, &["web", "file"]).await;
    assert!(ReadTools.catalog(&ctx).await.is_empty());
    for toolset in ["context_engine", "memory", "session_search"] {
        set_toolsets(&data, &[toolset]).await;
        let catalog = ReadTools.catalog(&ctx).await;
        assert_eq!(catalog.len(), expected.len(), "{toolset}");
        for name in &expected {
            assert_eq!(
                crate::routines::routine_tool_allowed_for_session(
                    &data.notes,
                    &binding.session_id,
                    name
                )
                .await
                .unwrap(),
                Some(true),
                "{name}: {toolset}"
            );
        }
    }
    set_toolsets(&data, &[]).await;
    assert!(ReadTools.catalog(&ctx).await.is_empty());
}

#[tokio::test]
async fn routine_read_dispatch_denies_before_arguments_or_data_and_after_revocation() {
    let (mut data, binding) = fixture().await;
    data.memory_enabled = false;
    data.activity = ActivityAccess::Unavailable(AppError::new(
        "read_data_touched",
        "Synthetic unavailable store.",
    ));
    for table in ["notes", "dictation_history", "memories"] {
        query(&format!("DROP TABLE {table}"))
            .execute(&data.notes)
            .await
            .unwrap();
    }
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let tools = names(&ctx);
    for after_catalog in [false, true] {
        if after_catalog {
            set_toolsets(&data, &["context_engine"]).await;
            assert_eq!(ReadTools.catalog(&ctx).await.len(), 15);
            set_toolsets(&data, &[]).await;
        }
        for name in &tools {
            assert_eq!(
                call(&ctx, name, &Value::Null).await.unwrap_err().code,
                "routine_tool_not_enabled",
                "{name}, after_catalog={after_catalog}"
            );
        }
    }
}

#[tokio::test]
async fn deleted_routine_loses_read_catalog_and_dispatch_before_arguments() {
    let (data, binding) = fixture().await;
    set_toolsets(&data, &["context_engine", "skills"]).await;
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    assert_eq!(ReadTools.catalog(&ctx).await.len(), 15);
    query("DELETE FROM routines WHERE id = 'read-routine'")
        .execute(&data.notes)
        .await
        .unwrap();
    assert!(ReadTools.catalog(&ctx).await.is_empty());
    let mut tools = names(&ctx);
    tools.extend(["get_dictation", "list_skills", "load_skill"].map(str::to_owned));
    for name in tools {
        assert_eq!(
            call(&ctx, &name, &Value::Null).await.unwrap_err().code,
            "routine_tool_not_enabled",
            "{name}"
        );
    }
}

#[tokio::test]
async fn routine_read_enabled_context_permits_data_reads() {
    let (mut data, binding) = fixture().await;
    let (_directory, store) = activity_store().await;
    data.activity = ActivityAccess::Open {
        store,
        settings: Default::default(),
    };
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    for toolset in ["context_engine", "memory", "session_search"] {
        set_toolsets(&data, &[toolset]).await;
        for (name, args) in [
            ("search_notes", json!({"query":"zephyr"})),
            ("get_note", json!({"id":"note-roadmap"})),
            ("list_dictations", json!({})),
            ("list_memories", json!({})),
            ("get_day_schedule", json!({})),
        ] {
            call(&ctx, name, &args)
                .await
                .unwrap_or_else(|error| panic!("{name}: {}", error.code));
        }
    }
}

#[tokio::test]
async fn routine_read_policy_preserves_unbound_and_attended_chat_reads() {
    let (data, _) = fixture().await;
    let repo = crate::agent_sessions::AgentRepository::new(data.notes.clone());
    let session = repo
        .create_session_in_profile(
            "Attended read fixture",
            "test",
            crate::agent_sessions::AgentSafetyMode::Sandboxed,
            None,
            &data.profile,
        )
        .await
        .unwrap();
    let run = repo.create_run(&session.id, "test", None).await.unwrap();
    let chat = TurnBinding {
        authority: crate::domain::turn_authority::TurnAuthority::new(),
        wire_names: None,
        data_partition: crate::agent_sessions::run_partition::resolve(
            &data.notes,
            &session.id,
            &run.id,
        )
        .await
        .unwrap(),
        session_id: session.id,
        run_id: run.id,
        cli: CliKind::Claude,
        safety_mode: crate::agent_sessions::AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "chat".into(),
    };
    for binding in [None, Some(&chat)] {
        let ctx = ToolCtx {
            binding,
            ..ToolCtx::unbound(&data)
        };
        assert!(ReadTools
            .catalog(&ctx)
            .await
            .iter()
            .any(|tool| tool["name"] == "get_note"));
        assert_eq!(
            call(&ctx, "get_note", &json!({"id":"note-roadmap"}))
                .await
                .unwrap()["id"],
            "note-roadmap"
        );
    }
}
