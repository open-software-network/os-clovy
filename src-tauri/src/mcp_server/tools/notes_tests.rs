use super::{call, ActivityAccess, ToolCtx};
use crate::approval_broker::testing::{Scripted, TestBroker};
use crate::llm::cli::CliKind;
use crate::mcp_server::tests::{data, notes_pool};
use serde_json::json;
use sqlx::query::query;

async fn insert_bound_turn(pool: &sqlx_sqlite::SqlitePool, binding: &super::TurnBinding) {
    query("INSERT INTO agent_sessions (id,title,model,status,created_at,updated_at) VALUES (?, 'Bound fixture', 'test', 'running', 'now','now')")
        .bind(&binding.session_id).execute(pool).await.unwrap();
    query("INSERT INTO agent_runs (id,session_id,model,status,started_at,updated_at,run_config_json) VALUES (?,?,'test','running','now','now','{\"dataPartition\":\"default\"}')")
        .bind(&binding.run_id).bind(&binding.session_id).execute(pool).await.unwrap();
}

#[tokio::test]
async fn registry_revocation_while_writer_is_held_refuses_note_write_even_if_db_is_running() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let registry = crate::mcp_server::turns::TurnRegistry::default();
    let token = registry.issue(binding.clone());
    let directory = tempfile::tempdir().unwrap();
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(
            sqlx_sqlite::SqliteConnectOptions::new()
                .filename(directory.path().join("fixture.sqlite3"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    insert_bound_turn(&pool, &binding).await;
    let repos = crate::db::repositories::Repositories::new(pool.clone());
    let note = repos.create_note("default", None).await.unwrap();
    let writer = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let write = repos.update_note_if_revision(
        "default",
        &note.id,
        note.revision as i64,
        Some("Must not commit"),
        None,
        Some(crate::db::repositories::McpMutation::Turn(
            &binding.session_id,
            &binding.run_id,
            &binding.authority,
        )),
    );
    tokio::pin!(write);
    assert!(futures_util::poll!(write.as_mut()).is_pending());
    assert!(registry.revoke(&token));
    writer.rollback().await.unwrap();
    assert_eq!(write.await.unwrap_err().code, "turn_expired");
    let status: String =
        sqlx::query_scalar::query_scalar("SELECT status FROM agent_runs WHERE id=?")
            .bind(&binding.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        status, "running",
        "Natural teardown has not persisted its terminal row yet"
    );
    let saved = repos.get_note(&note.id).await.unwrap();
    assert_eq!(saved.title, note.title);
    assert_eq!(saved.revision, note.revision);
}

#[tokio::test]
async fn bound_notes_use_approval_and_compare_revision_in_real_repository() {
    let broker = TestBroker::new([Scripted::Approve, Scripted::Approve, Scripted::Approve]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(broker.pool().clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let created = call(&ctx, "create_note", &json!({})).await.unwrap();
    let id = created["id"].as_str().unwrap();
    let revision = created["revision"].as_u64().unwrap();
    let changed = call(
        &ctx,
        "update_note",
        &json!({"id":id,"expectedRevision":revision,"content":"Saved fact"}),
    )
    .await
    .unwrap();
    assert_eq!(changed["editedContent"], "Saved fact");
    assert_eq!(
        call(
            &ctx,
            "update_note",
            &json!({"id":id,"expectedRevision":revision,"title":"stale"})
        )
        .await
        .unwrap_err()
        .code,
        "note_conflict"
    );
    assert_eq!(
        call(&ToolCtx::unbound(&data), "create_note", &json!({}))
            .await
            .unwrap_err()
            .code,
        "turn_required"
    );
    assert_eq!(broker.requests().len(), 3);
}

#[tokio::test]
async fn note_transaction_rejects_cancelled_or_reassigned_bound_turn() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let repos = crate::db::repositories::Repositories::new(broker.pool().clone());
    let note = repos.create_note("default", None).await.unwrap();
    query("UPDATE agent_runs SET status = 'cancelled' WHERE id = ?")
        .bind(&binding.run_id)
        .execute(broker.pool())
        .await
        .unwrap();
    let error = repos
        .update_note_if_revision(
            "default",
            &note.id,
            note.revision as i64,
            Some("unsafe"),
            None,
            Some(crate::db::repositories::McpMutation::Turn(
                &binding.session_id,
                &binding.run_id,
                &binding.authority,
            )),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "turn_expired");
    assert_eq!(repos.get_note(&note.id).await.unwrap().title, "");
    query("UPDATE agent_runs SET status = 'running' WHERE id = ?")
        .bind(&binding.run_id)
        .execute(broker.pool())
        .await
        .unwrap();
    query("INSERT INTO session_profiles (session_id, profile, assigned_at) VALUES (?, 'other', 'now')").bind(&binding.session_id).execute(broker.pool()).await.unwrap();
    let error = repos
        .delete_note_in_profile(
            "default",
            &note.id,
            Some(crate::db::repositories::McpMutation::Turn(
                &binding.session_id,
                &binding.run_id,
                &binding.authority,
            )),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "turn_expired");
    assert!(repos.get_note(&note.id).await.is_ok());
}

#[tokio::test]
async fn note_update_rechecks_bound_partition_after_approval_wait() {
    let broker = TestBroker::new([Scripted::Leave]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(broker.pool().clone(), ActivityAccess::Off);
    let repos = crate::db::repositories::Repositories::new(data.notes.clone());
    let note = repos.create_note("default", None).await.unwrap();
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let args = json!({"id":note.id,"expectedRevision":note.revision,"title":"must not persist"});
    let operation = call(&ctx, "update_note", &args);
    let change = async {
        while broker.broker().waiting_ids().is_empty() {
            tokio::task::yield_now().await;
        }
        query("INSERT INTO session_profiles (session_id,profile,assigned_at) VALUES (?, 'other', 'now')").bind(&binding.session_id).execute(broker.pool()).await.unwrap();
        let id = broker.broker().waiting_ids().pop().unwrap();
        broker
            .broker()
            .decide(
                broker.pool(),
                &id,
                crate::approval_broker::Decision::Approve,
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(operation, change);
    assert_eq!(result.unwrap_err().code, "turn_expired");
    assert_eq!(repos.get_note(&note.id).await.unwrap().title, "");
}

#[tokio::test]
async fn transcripts_search_and_pages_keep_legacy_and_hide_replaced_aggregate() {
    let pool = notes_pool().await;
    query("INSERT INTO recording_sessions (id, note_id, status, started_at) VALUES ('session-a', 'note-roadmap', 'completed', '2026-01-01'), ('session-b', 'note-roadmap', 'completed', '2026-01-02')").execute(&pool).await.unwrap();
    for (id, session, source, turn, text) in [
        (
            "aggregate",
            "session-a",
            "microphone",
            None,
            "obsolete searchable phrase",
        ),
        (
            "turn-one",
            "session-a",
            "microphone",
            Some(0),
            "first unique phrase",
        ),
        ("turn-two", "session-a", "microphone", Some(1), "repeated"),
        ("turn-three", "session-b", "system", Some(0), "repeated"),
    ] {
        query("INSERT INTO transcripts (id,note_id,audio_artifact_id,recording_session_id,source,turn_index,text,provider,status,created_at,updated_at) VALUES (?, 'note-roadmap','audio',?,?,?,?, 'test','completed','2026-01-03','2026-01-03')")
            .bind(id).bind(session).bind(source).bind(turn).bind(text).execute(&pool).await.unwrap();
    }
    let data = data(pool, ActivityAccess::Off);
    let ctx = ToolCtx::unbound(&data);
    let first = call(
        &ctx,
        "list_note_transcripts",
        &json!({"id":"note-roadmap","limit":1}),
    )
    .await
    .unwrap();
    let mut ids = vec![first["items"][0]["id"].as_str().unwrap().to_string()];
    let mut next = first["nextOffset"].as_u64();
    while let Some(offset) = next {
        let page = call(
            &ctx,
            "list_note_transcripts",
            &json!({"id":"note-roadmap","offset":offset,"limit":1}),
        )
        .await
        .unwrap();
        ids.push(page["items"][0]["id"].as_str().unwrap().to_string());
        next = page["nextOffset"].as_u64();
    }
    assert_eq!(ids.len(), 4);
    assert!(!ids.iter().any(|id| id == "aggregate"));
    assert!(ids.iter().any(|id| id == "transcript-note-roadmap"));
    let found = call(&ctx, "search_notes", &json!({"query":"first unique"}))
        .await
        .unwrap();
    assert!(found["notes"][0]["snippet"]
        .as_str()
        .unwrap()
        .contains("first unique"));
    let missing = call(
        &ctx,
        "search_notes",
        &json!({"query":"obsolete searchable"}),
    )
    .await
    .unwrap();
    assert_eq!(missing["count"], 0);
    assert_eq!(
        call(
            &ctx,
            "list_note_transcripts",
            &json!({"id":"note-other-profile"})
        )
        .await
        .unwrap_err()
        .code,
        "note_not_found"
    );
}

#[tokio::test]
async fn revision_compare_and_swap_rejects_stale_and_other_partition() {
    let pool = notes_pool().await;
    let repos = crate::db::repositories::Repositories::new(pool);
    let before = repos.get_note("note-roadmap").await.unwrap();
    let first = repos
        .update_note_if_revision(
            "default",
            &before.id,
            before.revision as i64,
            Some("New title"),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(first.revision, before.revision + 1);
    assert_eq!(
        repos
            .update_note_if_revision(
                "default",
                &before.id,
                before.revision as i64,
                None,
                Some("stale"),
                None,
            )
            .await
            .unwrap_err()
            .code,
        "note_conflict"
    );
    assert_eq!(
        repos
            .update_note_if_revision(
                "work",
                &before.id,
                first.revision as i64,
                None,
                Some("foreign"),
                None,
            )
            .await
            .unwrap_err()
            .code,
        "note_not_found"
    );
    assert_eq!(repos.get_note(&before.id).await.unwrap().title, "New title");
}

#[tokio::test]
async fn concurrent_revision_updates_have_exactly_one_winner() {
    let repos = crate::db::repositories::Repositories::new(notes_pool().await);
    let note = repos.get_note("note-roadmap").await.unwrap();
    let (a, b) = tokio::join!(
        repos.update_note_if_revision(
            "default",
            &note.id,
            note.revision as i64,
            Some("one"),
            None,
            None
        ),
        repos.update_note_if_revision(
            "default",
            &note.id,
            note.revision as i64,
            Some("two"),
            None,
            None
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(a.err().or_else(|| b.err()).unwrap().code, "note_conflict");
    assert_eq!(
        repos.get_note(&note.id).await.unwrap().revision,
        note.revision + 1
    );
}

#[tokio::test]
async fn partition_scoped_delete_refuses_active_recording_and_removes_note_records() {
    let pool = notes_pool().await;
    let repos = crate::db::repositories::Repositories::new(pool.clone());
    assert_eq!(
        repos
            .delete_note_in_profile("work", "note-roadmap", None)
            .await
            .unwrap_err()
            .code,
        "note_not_found"
    );
    query("INSERT INTO recording_sessions (id,note_id,status,started_at) VALUES ('live', 'note-roadmap', 'recording', '2026-01-01')").execute(&pool).await.unwrap();
    assert_eq!(
        repos
            .delete_note_in_profile("default", "note-roadmap", None)
            .await
            .unwrap_err()
            .code,
        "note_recording_active"
    );
    query("UPDATE recording_sessions SET status = 'recoverable', partial_path = 'owned.partial.wav', final_path = 'owned.wav' WHERE id = 'live'")
        .execute(&pool)
        .await
        .unwrap();
    let paths = repos
        .delete_note_in_profile("default", "note-roadmap", None)
        .await
        .unwrap();
    assert!(paths.contains(&"owned.partial.wav".to_string()));
    assert!(paths.contains(&"owned.wav".to_string()));
    assert!(repos.get_note("note-roadmap").await.is_err());
}

#[tokio::test]
async fn note_content_pages_round_trip_multibyte_and_preserve_transcript_contract() {
    let pool = notes_pool().await;
    let text = "ação🙂漢字".repeat(12000);
    query("UPDATE notes SET edited_content = ? WHERE id = 'note-roadmap'")
        .bind(&text)
        .execute(&pool)
        .await
        .unwrap();
    let data = data(pool, ActivityAccess::Off);
    let ctx = ToolCtx::unbound(&data);
    let mut offset = 0;
    let mut joined = String::new();
    loop {
        let page = call(
            &ctx,
            "get_note",
            &json!({"id":"note-roadmap", "contentOffset":offset}),
        )
        .await
        .unwrap();
        assert_eq!(page["contentOffset"], offset);
        assert_eq!(page["contentTotalChars"], text.chars().count());
        let content = page["content"].as_str().unwrap();
        assert!(content.chars().count() <= 20000);
        joined.push_str(content);
        let Some(next) = page["nextContentOffset"].as_u64() else {
            break;
        };
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(joined, text);
    let empty = call(
        &ctx,
        "get_note",
        &json!({"id":"note-roadmap","contentOffset":u64::MAX,"transcriptOffset":0}),
    )
    .await
    .unwrap();
    assert_eq!(empty["content"], "");
    assert!(empty["nextContentOffset"].is_null());
    assert!(empty["transcriptTotalChars"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn dictation_pages_round_trip_and_enforce_retention_and_partition() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let pool = notes_pool().await;
    insert_bound_turn(&pool, &binding).await;
    let text = "ação🙂漢字".repeat(12000);
    query("UPDATE dictation_history SET text = ? WHERE id = 'dictation-1'")
        .bind(&text)
        .execute(&pool)
        .await
        .unwrap();
    query("UPDATE dictation_history SET created_at = ? WHERE id = 'dictation-2'")
        .bind((chrono::Utc::now() - chrono::Duration::days(8)).to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
    let data = data(pool, ActivityAccess::Off);
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let mut offset = 0;
    let mut joined = String::new();
    loop {
        let page = call(
            &ctx,
            "get_dictation",
            &json!({"id":"dictation-1","textOffset":offset}),
        )
        .await
        .unwrap();
        assert_eq!(page["textOffset"], offset);
        assert_eq!(page["textTotalChars"], text.chars().count());
        joined.push_str(page["text"].as_str().unwrap());
        let Some(next) = page["nextTextOffset"].as_u64() else {
            break;
        };
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(joined, text);
    for id in ["dictation-2", "dictation-3", "missing"] {
        assert_eq!(
            call(&ctx, "get_dictation", &json!({"id":id}))
                .await
                .unwrap_err()
                .code,
            "dictation_not_found"
        );
    }
    assert_eq!(
        call(
            &ToolCtx::unbound(&data),
            "get_dictation",
            &json!({"id":"dictation-1"})
        )
        .await
        .unwrap_err()
        .code,
        "turn_required"
    );
    assert!(broker.requests().is_empty());
}

async fn routine_dictation_fixture() -> (super::McpData, crate::mcp_server::turns::TurnBinding) {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, false).await;
    let pool = notes_pool().await;
    insert_bound_turn(&pool, &binding).await;
    let now = "2026-10-28T00:00:00Z";
    query("INSERT INTO routines (id, name, prompt, schedule, created_at, updated_at, metadata_json, tool_catalog_version) VALUES ('dictation-routine', 'Daily', 'Summarise', 'every 1d', ?, ?, '{\"enabledToolsets\":[]}', 1)")
        .bind(now).bind(now).execute(&pool).await.unwrap();
    query("INSERT INTO routine_runs (id, routine_id, agent_session_id, claim_token, trigger_kind, status, model, safety_mode, created_at, updated_at) VALUES ('dictation-routine-run', 'dictation-routine', ?, 'claim', 'schedule', 'running', 'claude', 'sandboxed', ?, ?)")
        .bind(&binding.session_id).bind(now).bind(now).execute(&pool).await.unwrap();
    (data(pool, ActivityAccess::Off), binding)
}

#[tokio::test]
async fn routine_dictation_catalog_requires_an_enabled_context_toolset() {
    let (data, binding) = routine_dictation_fixture().await;
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    assert!(!super::descriptors(&ctx)
        .await
        .iter()
        .any(|tool| tool["name"] == "get_dictation"));
    query("UPDATE routines SET metadata_json = '{\"enabledToolsets\":[\"context_engine\"]}' WHERE id = 'dictation-routine'")
        .execute(&data.notes).await.unwrap();
    assert!(super::descriptors(&ctx)
        .await
        .iter()
        .any(|tool| tool["name"] == "get_dictation"));
}

#[tokio::test]
async fn routine_dictation_dispatch_checks_permission_and_rechecks_after_revocation() {
    let (data, binding) = routine_dictation_fixture().await;
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let args = json!({"id":"dictation-1"});
    assert_eq!(
        call(&ctx, "get_dictation", &args).await.unwrap_err().code,
        "routine_tool_not_enabled"
    );
    query("UPDATE routines SET metadata_json = '{\"enabledToolsets\":[\"context_engine\"]}' WHERE id = 'dictation-routine'")
        .execute(&data.notes).await.unwrap();
    assert_eq!(
        call(&ctx, "get_dictation", &args).await.unwrap()["id"],
        "dictation-1"
    );
    query("UPDATE routines SET metadata_json = '{\"enabledToolsets\":[]}' WHERE id = 'dictation-routine'")
        .execute(&data.notes).await.unwrap();
    assert_eq!(
        call(&ctx, "get_dictation", &args).await.unwrap_err().code,
        "routine_tool_not_enabled"
    );
    assert!(!super::descriptors(&ctx)
        .await
        .iter()
        .any(|tool| tool["name"] == "get_dictation"));
}
