use super::*;
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::approval_broker::testing::{Scripted, TestBroker};
use crate::llm::cli::CliKind;
use crate::mcp_server::tests::data;
use crate::mcp_server::tools::{call, ActivityAccess};

#[test]
fn project_arguments_require_explicit_deletion_and_bound_fields() {
    assert!(validate("delete_project", &json!({"projectId":"p"})).is_err());
    assert!(validate("update_project", &json!({"projectId":"p"})).is_err());
    assert!(validate(
        "update_project",
        &json!({"projectId":"p","memoryDisabled":"false"})
    )
    .is_err());
    assert!(validate("create_project", &json!({"name":"p","profile":"other"})).is_err());
    assert!(validate(
        "create_project",
        &json!({"name":"p","instructions":"x".repeat(4001)})
    )
    .is_err());
    assert!(validate(
        "update_project",
        &json!({"projectId":"p","instructions":null,"description":null,"memoryDisabled":true})
    )
    .is_ok());
}

#[tokio::test]
async fn approved_projects_preserve_plural_associations_and_delete_only_the_project() {
    let broker = TestBroker::new((0..10).map(|_| Scripted::Approve)).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = crate::db::repositories::Repositories::new(pool.clone());
    let note = repo
        .create_note_checked("default", None, None)
        .await
        .unwrap();
    let conversation = AgentRepository::new(pool.clone())
        .create_session_in_profile("Filed", "test", AgentSafetyMode::Sandboxed, None, "default")
        .await
        .unwrap();
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let p1 = call(&ctx, "create_project", &json!({"name":"First"}))
        .await
        .unwrap();
    let p2 = call(&ctx, "create_project", &json!({"name":"Second"}))
        .await
        .unwrap();
    let a = p1["id"].as_str().unwrap();
    let b = p2["id"].as_str().unwrap();
    repo.create_memory("default", Some(a), "Remember", "user")
        .await
        .unwrap();
    let updated=call(&ctx,"update_project",&json!({"projectId":a,"name":"Updated","instructions":"Keep context","memoryDisabled":true,"description":"Description"})).await.unwrap();
    assert_eq!(updated["name"], "Updated");
    assert_eq!(updated["memoryDisabled"], true);
    for project in [a, b] {
        call(
            &ctx,
            "assign_note_project",
            &json!({"projectId":project,"noteId":note.id}),
        )
        .await
        .unwrap();
        call(
            &ctx,
            "assign_conversation_project",
            &json!({"projectId":project,"conversationId":conversation.id}),
        )
        .await
        .unwrap();
    }
    let count: i64 =
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM note_folders WHERE note_id=?")
            .bind(&note.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 2);
    call(
        &ctx,
        "remove_note_project",
        &json!({"projectId":a,"noteId":note.id}),
    )
    .await
    .unwrap();
    call(
        &ctx,
        "remove_conversation_project",
        &json!({"projectId":a,"conversationId":conversation.id}),
    )
    .await
    .unwrap();
    call(
        &ctx,
        "delete_project",
        &json!({"projectId":a,"deleteNotes":false}),
    )
    .await
    .unwrap();
    assert!(repo.get_note(&note.id).await.is_ok());
    assert!(AgentRepository::new(pool.clone())
        .get_session(&conversation.id)
        .await
        .is_ok());
    let remaining: Vec<String> =
        sqlx::query_scalar::query_scalar("SELECT folder_id FROM note_folders WHERE note_id=?")
            .bind(&note.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(remaining, vec![b]);
    let sessions: Vec<String> = sqlx::query_scalar::query_scalar(
        "SELECT folder_id FROM session_folders WHERE session_id=?",
    )
    .bind(&conversation.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(sessions, vec![b]);
    let deleted_memories: i64 =
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM memory_tombstones")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(deleted_memories, 1);
}

#[tokio::test]
async fn project_write_boundary_rejects_cross_partition_or_missing_objects_and_stopped_callers() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = crate::db::repositories::Repositories::new(pool.clone());
    let project = repo.create_folder("default", "Owned", None).await.unwrap();
    let other = repo.create_folder("other", "Other", None).await.unwrap();
    let note = repo.create_note_checked("other", None, None).await.unwrap();
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    assert_eq!(
        apply(
            project_turn(&ctx).unwrap(),
            "update_project",
            &json!({"name":"Changed"}),
            &other.id,
            None
        )
        .await
        .unwrap_err()
        .code,
        "folder_not_found"
    );
    assert_eq!(
        apply(
            project_turn(&ctx).unwrap(),
            "assign_note_project",
            &json!({"noteId":note.id}),
            &project.id,
            None
        )
        .await
        .unwrap_err()
        .code,
        "project_object_not_found"
    );
    assert_eq!(
        apply(
            project_turn(&ctx).unwrap(),
            "assign_conversation_project",
            &json!({"conversationId":"nonexistent"}),
            &project.id,
            None
        )
        .await
        .unwrap_err()
        .code,
        "project_object_not_found"
    );
    query("UPDATE agent_runs SET status='cancelled' WHERE id=?")
        .bind(&binding.run_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        apply(
            project_turn(&ctx).unwrap(),
            "update_project",
            &json!({"name":"Changed"}),
            &project.id,
            None
        )
        .await
        .unwrap_err()
        .code,
        "turn_expired"
    );
    assert_eq!(repo.list_folders("default").await.unwrap()[0].name, "Owned");
}

#[tokio::test]
async fn project_delete_notes_snapshot_and_recording_guards_are_atomic() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = crate::db::repositories::Repositories::new(pool.clone());
    let project = repo.create_folder("default", "Owned", None).await.unwrap();
    let note = repo
        .create_note_checked("default", Some(&project.id), None)
        .await
        .unwrap();
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    assert_eq!(
        apply(
            project_turn(&ctx).unwrap(),
            "delete_project",
            &json!({"deleteNotes":true}),
            &project.id,
            Some(&[])
        )
        .await
        .unwrap_err()
        .code,
        "project_conflict"
    );
    let snapshot = vec![(note.id.clone(), note.title.clone())];
    query("INSERT INTO recording_sessions (id,note_id,status,started_at,permission_state,partial_path,final_path) VALUES ('recording',?,'paused','2026-01-01','granted','partial.wav','final.wav')").bind(&note.id).execute(&pool).await.unwrap();
    assert_eq!(
        apply(
            project_turn(&ctx).unwrap(),
            "delete_project",
            &json!({"deleteNotes":true}),
            &project.id,
            Some(&snapshot)
        )
        .await
        .unwrap_err()
        .code,
        "note_recording_active"
    );
    assert!(repo.get_note(&note.id).await.is_ok());
    query("UPDATE recording_sessions SET status='stopped' WHERE id='recording'")
        .execute(&pool)
        .await
        .unwrap();
    let (deleted, files) = apply(
        project_turn(&ctx).unwrap(),
        "delete_project",
        &json!({"deleteNotes":true}),
        &project.id,
        Some(&snapshot),
    )
    .await
    .unwrap();
    assert_eq!(deleted["deletedNoteIds"], json!([note.id]));
    assert!(files.contains(&"partial.wav".to_string()));
    assert!(files.contains(&"final.wav".to_string()));
    assert!(repo.get_note(&note.id).await.is_err());
    for table in ["recording_sessions", "note_folders", "session_folders"] {
        let count: i64 = sqlx::query_scalar::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

#[tokio::test]
async fn project_approval_cannot_survive_bound_partition_reassignment() {
    use crate::approval_broker::Decision;
    let broker = TestBroker::new([Scripted::Leave]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = crate::db::repositories::Repositories::new(pool.clone());
    let project = repo.create_folder("default", "Before", None).await.unwrap();
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let args = json!({"projectId":project.id,"name":"After"});
    let mutate = call(&ctx, "update_project", &args);
    let reassign_and_approve = async {
        let request = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(id) = broker.broker().waiting_ids().first() {
                    break id.clone();
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        query("INSERT INTO session_profiles (session_id,profile,assigned_at) VALUES (?,'other','now') ON CONFLICT(session_id) DO UPDATE SET profile='other'")
            .bind(&binding.session_id).execute(&pool).await.unwrap();
        broker
            .broker()
            .decide(&pool, &request, Decision::Approve)
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(mutate, reassign_and_approve);
    assert_eq!(result.unwrap_err().code, "turn_expired");
    assert_eq!(
        repo.list_folders("default").await.unwrap()[0].name,
        "Before"
    );
}

#[tokio::test]
async fn denied_project_change_has_no_effect() {
    let broker = TestBroker::new([Scripted::Deny]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    assert_eq!(
        call(&ctx, "create_project", &json!({"name":"Denied"}))
            .await
            .unwrap_err()
            .code,
        "user_denied"
    );
    let count: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM folders")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
}

#[tokio::test]
async fn project_reads_page_only_bound_projects_and_return_association_counts() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let pool = broker.pool().clone();
    let repo = crate::db::repositories::Repositories::new(pool.clone());
    let first = repo.create_folder("default", "First", None).await.unwrap();
    let second = repo.create_folder("default", "Second", None).await.unwrap();
    let foreign = repo.create_folder("other", "Foreign", None).await.unwrap();
    repo.create_note_checked("default", Some(&first.id), None)
        .await
        .unwrap();
    let data = data(pool.clone(), ActivityAccess::Off);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let page = call(&ctx, "list_projects", &json!({"limit":1}))
        .await
        .unwrap();
    assert_eq!(page["items"].as_array().unwrap().len(), 1);
    let next = call(
        &ctx,
        "list_projects",
        &json!({"limit":1,"offset":page["nextOffset"]}),
    )
    .await
    .unwrap();
    assert!(next["nextOffset"].is_null());
    let mut ids = vec![
        page["items"][0]["id"].as_str().unwrap().to_string(),
        next["items"][0]["id"].as_str().unwrap().to_string(),
    ];
    ids.sort();
    let mut expected = vec![first.id.clone(), second.id];
    expected.sort();
    assert_eq!(ids, expected);
    let detail = call(&ctx, "get_project", &json!({"projectId":first.id}))
        .await
        .unwrap();
    assert_eq!(detail["noteCount"], 1);
    assert_eq!(detail["conversationCount"], 0);
    assert_eq!(
        call(&ctx, "get_project", &json!({"projectId":foreign.id}))
            .await
            .unwrap_err()
            .code,
        "folder_not_found"
    );
    assert!(call(&ctx, "list_projects", &json!({"profile":"other"}))
        .await
        .is_err());
}
