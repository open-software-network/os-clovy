use super::{ActivityAccess, McpData, ToolCtx};
use crate::approval_broker::testing::{Scripted, TestBroker};
use crate::db::repositories::Repositories;
use crate::llm::cli::CliKind;
use serde_json::json;

#[tokio::test]
async fn memory_approved_save_and_forget_persist_with_tombstone() {
    for cli in [CliKind::Claude, CliKind::Codex] {
        let broker = TestBroker::new([Scripted::Approve, Scripted::Approve]).await;
        let binding = broker.turn(cli, true).await;
        let data = McpData {
            notes: broker.pool().clone(),
            profile: "default".into(),
            memory_enabled: true,
            activity: ActivityAccess::Off,
            now: chrono::Utc::now(),
        };
        let ctx = ToolCtx {
            data: &data,
            app: None,
            binding: Some(&binding),
            broker: Some(broker.broker()),
        };
        let settings = tempfile::tempdir().unwrap();
        let path = settings.path().join("memory.json");
        std::fs::write(&path, r#"{"enabled":true}"#).unwrap();
        let saved = super::memory::call_with_settings(
            &ctx,
            "save_memory",
            &json!({"content":"  Prefer concise notes  "}),
            &path,
        )
        .await
        .unwrap();
        let id = saved["id"].as_str().unwrap();
        assert_eq!(
            Repositories::new(data.notes.clone())
                .list_memories("default", None, false)
                .await
                .unwrap()[0]
                .content,
            "Prefer concise notes"
        );
        super::memory::call_with_settings(&ctx, "forget_memory", &json!({"id":id}), &path)
            .await
            .unwrap();
        assert!(Repositories::new(data.notes.clone())
            .list_memories("default", None, false)
            .await
            .unwrap()
            .is_empty());
        let count: i64 =
            sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM memory_tombstones WHERE id = ?")
                .bind(id)
                .fetch_one(&data.notes)
                .await
                .unwrap();
        assert_eq!(count, 1);
        assert_eq!(broker.requests().len(), 2);
    }
}

#[tokio::test]
async fn memory_denied_unattended_and_disabled_scopes_do_not_write() {
    let broker = TestBroker::new([Scripted::Deny, Scripted::Deny]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let unattended = broker.turn(CliKind::Codex, false).await;
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let settings = tempfile::tempdir().unwrap();
    let path = settings.path().join("memory.json");
    std::fs::write(&path, r#"{"enabled":true}"#).unwrap();
    let repos = Repositories::new(data.notes.clone());
    let existing = repos
        .create_memory("default", None, "Keep", "user")
        .await
        .unwrap();
    let folder = repos
        .create_folder("default", "Project", None)
        .await
        .unwrap();
    let scoped = repos
        .create_memory("default", Some(&folder.id), "Keep project", "user")
        .await
        .unwrap();
    repos
        .set_folder_memory_disabled(&folder.id, true)
        .await
        .unwrap();
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    for (name, args) in [
        ("save_memory", json!({"content":"Denied"})),
        ("forget_memory", json!({"id":existing.id})),
    ] {
        assert_eq!(
            super::memory::call_with_settings(&ctx, name, &args, &path)
                .await
                .unwrap_err()
                .code,
            "user_denied"
        );
        assert_eq!(
            super::memory::call_with_settings(
                &ToolCtx {
                    binding: Some(&unattended),
                    ..ctx
                },
                name,
                &args,
                &path
            )
            .await
            .unwrap_err()
            .code,
            "unattended_requires_approval"
        );
        assert_eq!(
            super::memory::call_with_settings(&ToolCtx::unbound(&data), name, &args, &path)
                .await
                .unwrap_err()
                .code,
            "turn_required"
        );
        assert_eq!(
            super::memory::call_with_settings(
                &ToolCtx {
                    broker: None,
                    ..ctx
                },
                name,
                &args,
                &path
            )
            .await
            .unwrap_err()
            .code,
            "approval_unavailable"
        );
    }
    for (name, args) in [
        (
            "save_memory",
            json!({"content":"Disabled","projectId":folder.id}),
        ),
        ("forget_memory", json!({"id":scoped.id})),
    ] {
        assert_eq!(
            super::memory::call_with_settings(&ctx, name, &args, &path)
                .await
                .unwrap_err()
                .code,
            "memory_disabled"
        );
    }
    std::fs::write(&path, r#"{"enabled":false}"#).unwrap();
    assert_eq!(
        super::memory::call_with_settings(
            &ctx,
            "save_memory",
            &json!({"content":"Disabled"}),
            &path
        )
        .await
        .unwrap_err()
        .code,
        "memory_disabled"
    );
    let disabled_data = McpData {
        memory_enabled: false,
        notes: data.notes.clone(),
        profile: data.profile.clone(),
        activity: ActivityAccess::Off,
        now: data.now,
    };
    assert_eq!(
        super::memory::call_with_settings(
            &ToolCtx {
                data: &disabled_data,
                ..ctx
            },
            "forget_memory",
            &json!({"id":existing.id}),
            &path
        )
        .await
        .unwrap_err()
        .code,
        "memory_disabled"
    );
    assert_eq!(
        repos
            .list_memories("default", None, true)
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(broker.requests().len(), 2);
}

struct DeletePause {
    reached: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

fn paused_memory_repositories(
    pool: sqlx_sqlite::SqlitePool,
    boundary: crate::db::repositories::MemoryWriteBoundary,
) -> (Repositories, std::sync::Arc<DeletePause>) {
    let pause = std::sync::Arc::new(DeletePause {
        reached: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
    });
    let observer = pause.clone();
    let repos = Repositories::new(pool).observe_memory_writes(move |observed| {
        let pause = observer.clone();
        Box::pin(async move {
            if observed == boundary {
                pause.reached.notify_one();
                pause.resume.notified().await;
            }
        })
    });
    (repos, pause)
}

#[tokio::test]
async fn forget_memory_serializes_global_toggle_until_delete_commit() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("memory.json");
    std::fs::write(&settings, r#"{"enabled":true}"#).unwrap();
    let (repos, pause) = paused_memory_repositories(
        broker.pool().clone(),
        crate::db::repositories::MemoryWriteBoundary::BeforeDeleteCommit,
    );
    let memory = repos
        .create_memory("default", None, "Keep until admitted", "user")
        .await
        .unwrap();
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let arguments = json!({"id": memory.id});
    let delete =
        super::memory::call_with_repositories(&ctx, "forget_memory", &arguments, &settings, &repos);
    let toggle = async {
        pause.reached.notified().await;
        // The semantic probe is inside the delete transaction, immediately
        // before commit; the settings lock must still span that boundary.
        let mut toggle = Box::pin(crate::commands::set_memory_enabled_with_settings(
            &settings, false,
        ));
        let blocked = futures_util::poll!(toggle.as_mut()).is_pending();
        let enabled_before_commit = crate::commands::load_memory_settings(&settings).enabled;
        pause.resume.notify_one();
        toggle.await.unwrap();
        (blocked, enabled_before_commit)
    };
    let (result, (blocked, enabled_before_commit)) =
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(delete, toggle)
        })
        .await
        .expect("concurrent operations finish");
    assert_eq!(result.unwrap()["forgotten"], true);
    assert_eq!(broker.requests().len(), 1);
    assert!(blocked, "settings toggle waits for delete commit");
    assert!(enabled_before_commit);
    assert!(!crate::commands::load_memory_settings(&settings).enabled);
    assert!(repos
        .list_memories("default", None, false)
        .await
        .unwrap()
        .is_empty());
    let count: i64 =
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM memory_tombstones WHERE id = ?")
            .bind(&memory.id)
            .fetch_one(broker.pool())
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn forget_memory_revalidates_concurrent_project_toggle_in_delete_transaction() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let binding = broker.turn(CliKind::Codex, true).await;
    let dir = tempfile::tempdir().unwrap();
    let settings = dir.path().join("memory.json");
    std::fs::write(&settings, r#"{"enabled":true}"#).unwrap();
    let (repos, pause) = paused_memory_repositories(
        broker.pool().clone(),
        crate::db::repositories::MemoryWriteBoundary::BeforeDeleteTransaction,
    );
    let project = repos
        .create_folder("default", "Project", None)
        .await
        .unwrap();
    let memory = repos
        .create_memory("default", Some(&project.id), "Preserve this fact", "user")
        .await
        .unwrap();
    let writer = Repositories::new(broker.pool().clone());
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let arguments = json!({"id": memory.id});
    let delete =
        super::memory::call_with_repositories(&ctx, "forget_memory", &arguments, &settings, &repos);
    let toggle = async {
        pause.reached.notified().await;
        // Settings admission has happened, but transaction/project validation
        // has not. Commit the project disable before releasing that boundary.
        writer
            .set_folder_memory_disabled(&project.id, true)
            .await
            .unwrap();
        pause.resume.notify_one();
    };
    let (result, ()) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(delete, toggle)
    })
    .await
    .expect("concurrent operations finish");
    assert_eq!(result.unwrap_err().code, "memory_disabled");
    assert_eq!(broker.requests().len(), 1);
    assert_eq!(
        repos
            .list_memories("default", Some(&project.id), false)
            .await
            .unwrap()[0]
            .id,
        memory.id
    );
    let count: i64 =
        sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM memory_tombstones WHERE id = ?")
            .bind(&memory.id)
            .fetch_one(broker.pool())
            .await
            .unwrap();
    assert_eq!(count, 0);
    writer.delete_memory("default", &memory.id).await.unwrap();
    assert!(repos
        .list_memories("default", Some(&project.id), false)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn update_preserves_memory_identity_and_search_filters_before_paging() {
    let broker = TestBroker::new([Scripted::Approve]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        binding: Some(&binding),
        broker: Some(broker.broker()),
        ..ToolCtx::unbound(&data)
    };
    let root = tempfile::tempdir().unwrap();
    let settings = root.path().join("memory.json");
    let repos = Repositories::new(data.notes.clone());
    let folder = repos
        .create_folder("default", "Enabled", None)
        .await
        .unwrap();
    let hidden = repos
        .create_folder("default", "Disabled", None)
        .await
        .unwrap();
    let original = repos
        .create_memory("default", Some(&folder.id), "Before", "user")
        .await
        .unwrap();
    repos
        .create_memory("foreign", None, "needle foreign", "user")
        .await
        .unwrap();
    repos
        .create_memory("default", Some(&hidden.id), "needle disabled", "user")
        .await
        .unwrap();
    repos
        .set_folder_memory_disabled(&hidden.id, true)
        .await
        .unwrap();
    let edited = super::memory::call_with_settings(
        &ctx,
        "update_memory",
        &json!({"id":original.id,"content":"needle changed"}),
        &settings,
    )
    .await
    .unwrap();
    assert_eq!(edited["id"], original.id);
    assert_eq!(edited["folderId"], folder.id);
    assert_eq!(edited["createdAt"], original.created_at);
    assert_eq!(edited["source"], "user");
    let found = super::memory::call_with_settings(
        &ctx,
        "search_memories",
        &json!({"query":"needle","limit":1}),
        &settings,
    )
    .await
    .unwrap();
    assert_eq!(found["items"][0]["id"], original.id);
    assert_eq!(found["hasMore"], false);
    assert_eq!(broker.requests().len(), 1);
    repos
        .set_folder_memory_disabled(&folder.id, true)
        .await
        .unwrap();
    let found = super::memory::call_with_settings(
        &ctx,
        "search_memories",
        &json!({"query":"needle","limit":1}),
        &settings,
    )
    .await
    .unwrap();
    assert_eq!(found["count"], 0);
    let error = super::memory::call_with_settings(
        &ctx,
        "update_memory",
        &json!({"id":original.id,"content":"Denied"}),
        &settings,
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "memory_disabled");
}
