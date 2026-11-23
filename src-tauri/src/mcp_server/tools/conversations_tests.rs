use super::{call, ActivityAccess, ToolCtx};
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::approval_broker::testing::TestBroker;
use crate::llm::cli::CliKind;
use crate::mcp_server::tests::data;
use serde_json::json;
use sqlx::query::query;

#[tokio::test]
async fn routine_artifact_read_rechecks_policy_at_publication_after_io() {
    use super::ToolGroup;
    for change in ["keep", "remove_toolset", "delete_policy"] {
        let broker = TestBroker::new([]).await;
        let binding = broker.turn(CliKind::Claude, false).await;
        let pool = broker.pool().clone();
        query("INSERT INTO routines (id,name,prompt,schedule,created_at,updated_at,metadata_json,tool_catalog_version) VALUES ('read-policy','Read','Read','every 1d','now','now','{\"enabledToolsets\":[\"session_search\"]}',1)").execute(&pool).await.unwrap();
        query("INSERT INTO routine_runs (id,routine_id,agent_session_id,claim_token,trigger_kind,status,model,safety_mode,created_at,updated_at) VALUES ('read-run','read-policy',?,'claim','schedule','running','claude','sandboxed','now','now')").bind(&binding.session_id).execute(&pool).await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("fixture.txt");
        std::fs::write(&file, "Authorized file content").unwrap();
        query("INSERT INTO agent_artifacts (id,session_id,run_id,provenance,action,path,available,created_at) VALUES ('policy-artifact',?,?,'user','attachment',?,1,'now')").bind(&binding.session_id).bind(&binding.run_id).bind(file.to_string_lossy().as_ref()).execute(&pool).await.unwrap();
        let data = data(pool.clone(), ActivityAccess::Off);
        let ctx = ToolCtx {
            data: &data,
            app: None,
            binding: Some(&binding),
            broker: None,
        };
        assert!(
            super::routine_tool_allowed(&pool, &binding, "read_conversation_artifact")
                .await
                .unwrap()
        );
        let (read_done, observed_read) = tokio::sync::oneshot::channel();
        let (release, released) = tokio::sync::oneshot::channel();
        let read = async {
            let value = super::conversations::ConversationTools
                .call(
                    &ctx,
                    "read_conversation_artifact",
                    &json!({"conversationId":binding.session_id,"artifactId":"policy-artifact"}),
                )
                .await?;
            read_done.send(()).unwrap();
            released.await.unwrap();
            Ok(value)
        };
        let mutate = async {
            observed_read.await.unwrap();
            match change {
                "remove_toolset" => {
                    query("UPDATE routines SET metadata_json='{\"enabledToolsets\":[]}' WHERE id='read-policy'").execute(&pool).await.unwrap();
                }
                "delete_policy" => {
                    query("DELETE FROM routines WHERE id='read-policy'")
                        .execute(&pool)
                        .await
                        .unwrap();
                }
                _ => {}
            }
            release.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(
            super::authorized_native_read(&ctx, "read_conversation_artifact", read),
            mutate
        );
        if change == "keep" {
            assert_eq!(result.unwrap()["text"], "Authorized file content");
        } else {
            assert_eq!(result.unwrap_err().code, "routine_tool_not_enabled");
        }
    }
}

#[tokio::test]
async fn conversation_pages_hide_other_partitions_and_tool_payloads() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = AgentRepository::new(pool.clone());
    let other = repo
        .create_session_in_profile("Other", "test", AgentSafetyMode::Sandboxed, None, "other")
        .await
        .unwrap();
    let run2 = repo
        .create_run(&binding.session_id, "test", None)
        .await
        .unwrap();
    query("UPDATE agent_runs SET started_at='2026-01-01' WHERE id=?")
        .bind(&binding.run_id)
        .execute(&pool)
        .await
        .unwrap();
    query("UPDATE agent_runs SET started_at='2026-01-02' WHERE id=?")
        .bind(&run2.id)
        .execute(&pool)
        .await
        .unwrap();
    for (id, run, sequence, kind, text) in [
        ("m1", binding.run_id.as_str(), 0, "user_message", "first"),
        (
            "tool",
            binding.run_id.as_str(),
            1,
            "tool_result",
            "secret tool payload",
        ),
        ("m2", run2.id.as_str(), 2, "assistant_message", "second"),
    ] {
        query("INSERT INTO agent_items (id,session_id,run_id,sequence,kind,payload_json,created_at) VALUES (?,?,?,?,?,?, '2026-01-03')")
            .bind(id).bind(&binding.session_id).bind(run).bind(sequence).bind(kind).bind(json!({"content":text}).to_string()).execute(&pool).await.unwrap();
    }
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    assert_eq!(
        call(
            &ctx,
            "get_conversation",
            &json!({"conversationId":other.id})
        )
        .await
        .unwrap_err()
        .code,
        "conversation_not_found"
    );
    let first = call(
        &ctx,
        "list_conversation_messages",
        &json!({"conversationId":binding.session_id,"limit":1}),
    )
    .await
    .unwrap();
    assert_eq!(first["items"][0]["text"], "first");
    let second = call(
        &ctx,
        "list_conversation_messages",
        &json!({"conversationId":binding.session_id,"limit":1,"cursor":first["nextCursor"]}),
    )
    .await
    .unwrap();
    assert_eq!(second["items"][0]["text"], "second");
    assert!(second["nextCursor"].is_null());
    let search = call(
        &ctx,
        "search_conversation_messages",
        &json!({"conversationId":binding.session_id,"query":"secret tool"}),
    )
    .await
    .unwrap();
    assert!(search["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn artifact_reads_are_id_scoped_and_page_large_utf8_files() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let folder = tempfile::tempdir().unwrap();
    let file = folder.path().join("large.txt");
    let content = "ação🙂".repeat(200_000);
    std::fs::write(&file, &content).unwrap();
    query("INSERT INTO agent_artifacts (id,session_id,run_id,provenance,action,path,available,created_at) VALUES ('artifact',?,?,'user','attachment',?,1,'2026-01-01')").bind(&binding.session_id).bind(&binding.run_id).bind(file.to_string_lossy().as_ref()).execute(&pool).await.unwrap();
    let data = data(pool, ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let mut offset = 0_u64;
    let mut joined = String::new();
    loop {
        let page = call(
            &ctx,
            "read_conversation_artifact",
            &json!({"conversationId":binding.session_id,"artifactId":"artifact","offset":offset}),
        )
        .await
        .unwrap();
        joined.push_str(page["text"].as_str().unwrap());
        match page["nextOffset"].as_u64() {
            Some(next) => {
                assert!(next > offset);
                offset = next
            }
            None => break,
        }
    }
    assert_eq!(joined, content);
    assert_eq!(
        call(
            &ctx,
            "read_conversation_artifact",
            &json!({"conversationId":binding.session_id,"artifactId":"missing"})
        )
        .await
        .unwrap_err()
        .code,
        "agent_artifact_unavailable"
    );
}

#[tokio::test]
async fn long_message_continuation_does_not_skip_the_next_message() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let content = "ação🙂".repeat(12_000);
    for (id, sequence, text) in [("long", 0, content.as_str()), ("next", 1, "after")] {
        query("INSERT INTO agent_items (id,session_id,run_id,sequence,kind,payload_json,created_at) VALUES (?,?,?,?,'user_message',?,'2026-01-03')")
            .bind(id).bind(&binding.session_id).bind(&binding.run_id).bind(sequence).bind(json!({"content":text}).to_string()).execute(&pool).await.unwrap();
    }
    let data = data(pool, ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let first = call(
        &ctx,
        "list_conversation_messages",
        &json!({"conversationId":binding.session_id,"limit":1}),
    )
    .await
    .unwrap();
    let text_offset = first["nextTextOffset"].as_u64().unwrap();
    let second = call(&ctx, "list_conversation_messages", &json!({"conversationId":binding.session_id,"limit":1,"textOffset":text_offset,"cursor":first["nextCursor"]})).await.unwrap();
    let joined = format!(
        "{}{}",
        first["items"][0]["text"].as_str().unwrap(),
        second["items"][0]["text"].as_str().unwrap()
    );
    assert_eq!(joined, content);
    let third = call(
        &ctx,
        "list_conversation_messages",
        &json!({"conversationId":binding.session_id,"limit":1,"cursor":second["nextCursor"]}),
    )
    .await
    .unwrap();
    assert_eq!(third["items"][0]["text"], "after");
}

#[tokio::test]
async fn approved_conversation_lifecycle_preserves_bound_partition_and_guards_self() {
    use crate::approval_broker::testing::Scripted;
    let broker = TestBroker::new([
        Scripted::Approve,
        Scripted::Approve,
        Scripted::Approve,
        Scripted::Approve,
        Scripted::Approve,
    ])
    .await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = data(broker.pool().clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    assert_eq!(
        call(
            &ctx,
            "delete_conversation",
            &json!({"conversationId":binding.session_id})
        )
        .await
        .unwrap_err()
        .code,
        "conversation_self_delete"
    );
    let created = call(&ctx, "create_conversation", &json!({"title":"First"}))
        .await
        .unwrap();
    let id = created["id"].as_str().unwrap();
    call(
        &ctx,
        "rename_conversation",
        &json!({"conversationId":id,"title":"Renamed"}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(&ctx, "get_conversation", &json!({"conversationId":id}))
            .await
            .unwrap()["title"],
        "Renamed"
    );
    call(&ctx, "archive_conversation", &json!({"conversationId":id}))
        .await
        .unwrap();
    assert!(
        !call(&ctx, "get_conversation", &json!({"conversationId":id}))
            .await
            .unwrap()["completedAt"]
            .is_null()
    );
    call(&ctx, "restore_conversation", &json!({"conversationId":id}))
        .await
        .unwrap();
    assert!(
        call(&ctx, "get_conversation", &json!({"conversationId":id}))
            .await
            .unwrap()["completedAt"]
            .is_null()
    );
    call(&ctx, "delete_conversation", &json!({"conversationId":id}))
        .await
        .unwrap();
    assert_eq!(
        call(&ctx, "get_conversation", &json!({"conversationId":id}))
            .await
            .unwrap_err()
            .code,
        "conversation_not_found"
    );
    let orphan: i64 = sqlx::query_scalar::query_scalar(
        "SELECT COUNT(*) FROM session_profiles WHERE session_id=?",
    )
    .bind(id)
    .fetch_one(&data.notes)
    .await
    .unwrap();
    assert_eq!(orphan, 0);
}

#[tokio::test]
async fn conversation_run_cancellation_targets_the_exact_session_and_rejects_self_and_foreign_profiles(
) {
    use crate::agent_sessions::api::claim_conversation_run_cancellation;
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = AgentRepository::new(pool.clone());
    let target = repo
        .create_session_in_profile(
            "Target",
            "test",
            AgentSafetyMode::Sandboxed,
            None,
            "default",
        )
        .await
        .unwrap();
    let run = repo.create_run(&target.id, "test", None).await.unwrap();
    let foreign = repo
        .create_session_in_profile("Foreign", "test", AgentSafetyMode::Sandboxed, None, "other")
        .await
        .unwrap();
    let foreign_run = repo.create_run(&foreign.id, "test", None).await.unwrap();
    assert_eq!(
        claim_conversation_run_cancellation(
            &pool,
            "default",
            &binding.session_id,
            &binding.run_id,
            &binding.session_id,
            &binding.run_id,
            &binding.authority,
        )
        .await
        .unwrap_err()
        .code,
        "conversation_self_cancel"
    );
    assert_eq!(
        claim_conversation_run_cancellation(
            &pool,
            "default",
            &binding.session_id,
            &binding.run_id,
            &binding.session_id,
            &run.id,
            &binding.authority,
        )
        .await
        .unwrap_err()
        .code,
        "run_not_found"
    );
    assert_eq!(
        claim_conversation_run_cancellation(
            &pool,
            "default",
            &binding.session_id,
            &binding.run_id,
            &foreign.id,
            &foreign_run.id,
            &binding.authority,
        )
        .await
        .unwrap_err()
        .code,
        "run_not_found"
    );
    assert_ne!(repo.get_run(&run.id).await.unwrap().status, "cancelled");
    query("UPDATE agent_runs SET status='waiting_for_user' WHERE id=?")
        .bind(&run.id)
        .execute(&pool)
        .await
        .unwrap();
    query("INSERT INTO agent_items (id,session_id,run_id,sequence,kind,payload_json,created_at) VALUES ('pending-target',?,?,1,'interruption','{\"status\":\"pending\"}','2026-01-01')").bind(&target.id).bind(&run.id).execute(&pool).await.unwrap();
    assert!(claim_conversation_run_cancellation(
        &pool,
        "default",
        &binding.session_id,
        &binding.run_id,
        &target.id,
        &run.id,
        &binding.authority,
    )
    .await
    .unwrap());
    assert_eq!(repo.get_run(&run.id).await.unwrap().status, "cancelled");
    assert_eq!(repo.get_session(&target.id).await.unwrap().status, "idle");
    assert_ne!(
        repo.get_run(&binding.run_id).await.unwrap().status,
        "cancelled"
    );
    crate::agent_sessions::api::retire_conversation_run_interruptions(&pool, &target.id, &run.id)
        .await
        .unwrap();
    let status: String = sqlx::query_scalar::query_scalar(
        "SELECT json_extract(payload_json,'$.status') FROM agent_items WHERE id='pending-target'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "expired");
    assert!(!claim_conversation_run_cancellation(
        &pool,
        "default",
        &binding.session_id,
        &binding.run_id,
        &target.id,
        &run.id,
        &binding.authority,
    )
    .await
    .unwrap());
    query("UPDATE agent_runs SET status='cancelled' WHERE id=?")
        .bind(&binding.run_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        claim_conversation_run_cancellation(
            &pool,
            "default",
            &binding.session_id,
            &binding.run_id,
            &foreign.id,
            &foreign_run.id,
            &binding.authority,
        )
        .await
        .unwrap_err()
        .code,
        "turn_expired"
    );
}

#[tokio::test]
async fn binary_artifacts_and_non_character_offsets_are_explicit_errors() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let folder = tempfile::tempdir().unwrap();
    for (id, bytes) in [
        ("binary", vec![0, 1, 2, 3]),
        ("utf8", "ação🙂".as_bytes().to_vec()),
        ("pdf", b"%PDF-1.7 ascii body".to_vec()),
        ("truncated", vec![b'a', 0xe2]),
    ] {
        let path = folder.path().join(id);
        std::fs::write(&path, bytes).unwrap();
        query("INSERT INTO agent_artifacts (id,session_id,run_id,provenance,action,path,available,created_at) VALUES (?,?,?,'user','attachment',?,1,'2026-01-01')").bind(id).bind(&binding.session_id).bind(&binding.run_id).bind(path.to_string_lossy().as_ref()).execute(&pool).await.unwrap();
    }
    query("UPDATE agent_artifacts SET mime_type='application/pdf' WHERE id='pdf'")
        .execute(&pool)
        .await
        .unwrap();
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    for (artifact, offset) in [("binary", 0), ("utf8", 2), ("pdf", 0), ("truncated", 0)] {
        assert_eq!(
            call(
                &ctx,
                "read_conversation_artifact",
                &json!({"conversationId":binding.session_id,"artifactId":artifact,"offset":offset})
            )
            .await
            .unwrap_err()
            .code,
            "agent_artifact_binary"
        );
    }
}

#[tokio::test]
async fn ui_stop_is_durable_before_host_teardown_and_blocks_a_stale_bound_write() {
    use crate::agent_sessions::api::claim_ui_run_cancellation;
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    query("INSERT INTO agent_items (id,session_id,run_id,sequence,kind,payload_json,created_at) VALUES ('approval-before-stop',?,?,0,'interruption','{\"status\":\"pending\"}','now')")
        .bind(&binding.session_id).bind(&binding.run_id).execute(&pool).await.unwrap();
    assert!(
        claim_ui_run_cancellation(&pool, &binding.session_id, &binding.run_id)
            .await
            .unwrap()
    );
    let status: String =
        sqlx::query_scalar::query_scalar("SELECT status FROM agent_runs WHERE id=?")
            .bind(&binding.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "cancelled");
    let pending:String=sqlx::query_scalar::query_scalar("SELECT json_extract(payload_json,'$.status') FROM agent_items WHERE id='approval-before-stop'").fetch_one(&pool).await.unwrap();
    assert_eq!(
        pending, "pending",
        "The live broker must settle its own cards after the durable Stop claim"
    );
    let repo = crate::db::repositories::Repositories::new(pool.clone());
    assert_eq!(
        repo.create_note_checked(
            "default",
            None,
            Some(crate::db::repositories::McpMutation::Turn(
                &binding.session_id,
                &binding.run_id,
                &binding.authority
            ))
        )
        .await
        .unwrap_err()
        .code,
        "turn_expired"
    );
    assert!(
        !claim_ui_run_cancellation(&pool, &binding.session_id, &binding.run_id)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn ui_session_delete_checks_actual_runs_and_cleans_metadata_atomically() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = AgentRepository::new(pool.clone());
    let target = repo
        .create_session_in_profile(
            "Target",
            "test",
            AgentSafetyMode::Sandboxed,
            None,
            "default",
        )
        .await
        .unwrap();
    let run = repo.create_run(&target.id, "test", None).await.unwrap();
    // The UI's session status can be stale; active agent_runs is authoritative.
    query("UPDATE agent_sessions SET status='idle' WHERE id=?")
        .bind(&target.id)
        .execute(&pool)
        .await
        .unwrap();
    for status in ["queued", "running", "waiting_for_user"] {
        query("UPDATE agent_runs SET status=? WHERE id=?")
            .bind(status)
            .bind(&run.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            repo.delete_session(&target.id).await.unwrap_err().code,
            "agent_run_active"
        );
        assert!(repo.get_session(&target.id).await.is_ok());
    }
    query("UPDATE agent_runs SET status='completed' WHERE id=?")
        .bind(&run.id)
        .execute(&pool)
        .await
        .unwrap();
    query("INSERT INTO completed_sessions(session_id,completed_at) VALUES (?,'now')")
        .bind(&target.id)
        .execute(&pool)
        .await
        .unwrap();
    repo.delete_session(&target.id).await.unwrap();
    for table in ["agent_sessions", "session_profiles", "completed_sessions"] {
        let key = if table == "agent_sessions" {
            "id"
        } else {
            "session_id"
        };
        let count: i64 = sqlx::query_scalar::query_scalar(&format!(
            "SELECT COUNT(*) FROM {table} WHERE {key}=?"
        ))
        .bind(&target.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0);
    }
    assert_eq!(
        repo.delete_session(&target.id).await.unwrap_err().code,
        "conversation_not_found"
    );
    assert_eq!(
        repo.delete_session_for_turn(
            &binding.session_id,
            Some((
                "default",
                &binding.session_id,
                &binding.run_id,
                &binding.authority
            ))
        )
        .await
        .unwrap_err()
        .code,
        "conversation_self_delete"
    );
    let foreign = repo
        .create_session_in_profile("Foreign", "test", AgentSafetyMode::Sandboxed, None, "other")
        .await
        .unwrap();
    assert_eq!(
        repo.delete_session_for_turn(
            &foreign.id,
            Some((
                "default",
                &binding.session_id,
                &binding.run_id,
                &binding.authority
            ))
        )
        .await
        .unwrap_err()
        .code,
        "conversation_not_found"
    );
    assert!(repo.get_session(&foreign.id).await.is_ok());
}

#[tokio::test]
async fn concurrent_real_run_admission_and_ui_deletion_have_one_serial_order() {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let repo = AgentRepository::new(pool.clone());
    for _ in 0..8 {
        let target = repo
            .create_session_in_profile("Race", "test", AgentSafetyMode::Sandboxed, None, "default")
            .await
            .unwrap();
        let (run, deleted) = tokio::join!(
            repo.create_run(&target.id, "test", None),
            repo.delete_session(&target.id)
        );
        match (run, deleted) {
            (Ok(run), Err(error)) => {
                assert_eq!(error.code, "agent_run_active");
                assert_eq!(repo.get_run(&run.id).await.unwrap().session_id, target.id);
                assert!(repo.get_session(&target.id).await.is_ok());
            }
            (Err(_), Ok(())) => {
                assert!(repo.get_session(&target.id).await.is_err());
                let orphan: i64 = sqlx::query_scalar::query_scalar(
                    "SELECT COUNT(*) FROM agent_runs WHERE session_id=?",
                )
                .bind(&target.id)
                .fetch_one(&pool)
                .await
                .unwrap();
                assert_eq!(orphan, 0);
            }
            outcome => panic!("Run admission and deletion did not serialize: {outcome:?}"),
        }
    }
    assert!(query("PRAGMA foreign_key_check")
        .fetch_all(&pool)
        .await
        .unwrap()
        .is_empty());
}
