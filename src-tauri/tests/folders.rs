use clovy_lib::db::{migrations::run_migrations, repositories::Repositories};
use sqlx::query::query;
use sqlx_sqlite::SqlitePoolOptions;

async fn repos() -> Repositories {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("sqlite memory");
    run_migrations(&pool).await.expect("migrations");
    Repositories::new(pool)
}

async fn create_agent_session(repos: &Repositories, id: &str) {
    query(
        "INSERT INTO agent_sessions
         (id, title, status, model, safety_mode, source, created_at, updated_at)
         VALUES (?, 'Test session', 'idle', 'auto', 'sandboxed', 'user',
                 '2026-05-19T10:00:00Z', '2026-05-19T10:00:00Z')",
    )
    .bind(id)
    .execute(&repos.pool)
    .await
    .expect("agent session fixture");
}

#[tokio::test]
async fn allows_duplicate_folder_names_with_distinct_ids() {
    let repos = repos().await;
    let first = repos
        .create_folder("default", "Ideas", None)
        .await
        .expect("first folder");
    let second = repos
        .create_folder("default", "Ideas", None)
        .await
        .expect("second folder allowed");

    assert_ne!(first.id, second.id);
    let listed = repos.list_folders("default").await.expect("list");
    let ideas: Vec<_> = listed
        .into_iter()
        .filter(|folder| folder.name == "Ideas")
        .collect();
    assert_eq!(ideas.len(), 2);
}

#[tokio::test]
async fn assigning_and_removing_folder_keeps_note_in_all_notes() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let folder = repos
        .create_folder("default", "Work", None)
        .await
        .expect("folder");

    let assigned = repos
        .assign_note_to_folder("default", &note.id, &folder.id)
        .await
        .expect("assign");
    assert_eq!(assigned.folder_ids, vec![folder.id.clone()]);

    let removed = repos
        .remove_note_from_folder(&note.id, &folder.id)
        .await
        .expect("remove");
    assert!(removed.folder_ids.is_empty());

    let all_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("all notes");
    assert_eq!(all_notes.items.len(), 1);
    assert_eq!(all_notes.items[0].id, note.id);
}

#[tokio::test]
async fn assigning_note_to_folder_requires_both_to_match_the_active_profile() {
    let repos = repos().await;
    let note_a = repos
        .create_note("profile-a", None)
        .await
        .expect("profile A note");
    let folder_a = repos
        .create_folder("profile-a", "Profile A project", None)
        .await
        .expect("profile A project");
    let folder_b = repos
        .create_folder("profile-b", "Profile B project", None)
        .await
        .expect("profile B project");

    let note_mismatch = repos
        .assign_note_to_folder("profile-b", &note_a.id, &folder_b.id)
        .await
        .expect_err("profile A note must not be filed while profile B is active");
    assert_eq!(note_mismatch.code, "note_folder_profile_mismatch");

    let folder_mismatch = repos
        .assign_note_to_folder("profile-a", &note_a.id, &folder_b.id)
        .await
        .expect_err("profile B project must not accept a profile A note");
    assert_eq!(folder_mismatch.code, "note_folder_profile_mismatch");

    let unchanged = repos.get_note(&note_a.id).await.expect("unchanged note");
    assert!(unchanged.folder_ids.is_empty());

    let assigned = repos
        .assign_note_to_folder("profile-a", &note_a.id, &folder_a.id)
        .await
        .expect("matching profile assignment");
    assert_eq!(assigned.folder_ids, vec![folder_a.id]);
}

#[tokio::test]
async fn deleting_folder_without_notes_keeps_associated_notes() {
    let repos = repos().await;
    let folder = repos
        .create_folder("default", "Work", None)
        .await
        .expect("folder");
    let note = repos
        .create_note("default", Some(folder.id.clone()))
        .await
        .expect("note");

    repos
        .delete_folder(&folder.id, false)
        .await
        .expect("delete folder");

    let folders = repos.list_folders("default").await.expect("folders");
    assert!(folders.is_empty());

    let all_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("all notes");
    assert_eq!(all_notes.items.len(), 1);
    assert_eq!(all_notes.items[0].id, note.id);
    assert!(all_notes.items[0].folder_ids.is_empty());
}

#[tokio::test]
async fn deleting_folder_with_notes_removes_associated_notes() {
    let repos = repos().await;
    let folder = repos
        .create_folder("default", "Work", None)
        .await
        .expect("folder");
    repos
        .create_note("default", Some(folder.id.clone()))
        .await
        .expect("note");

    repos
        .delete_folder(&folder.id, true)
        .await
        .expect("delete folder and notes");

    let folders = repos.list_folders("default").await.expect("folders");
    assert!(folders.is_empty());

    let all_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("all notes");
    assert!(all_notes.items.is_empty());
}

#[tokio::test]
async fn renaming_missing_folder_returns_descriptive_error() {
    let repos = repos().await;
    let error = repos
        .rename_folder("missing-folder", "Renamed", None)
        .await
        .expect_err("missing folder should fail");

    assert_eq!(error.code, "folder_not_found");
    assert_eq!(
        error.message,
        "Folder was not found or has already been deleted."
    );
}

#[tokio::test]
async fn assigning_and_removing_session_folders_round_trips() {
    let repos = repos().await;
    create_agent_session(&repos, "agent-session-1").await;
    let folder = repos
        .create_folder("default", "Launch", None)
        .await
        .expect("folder");

    repos
        .assign_session_to_folder("default", "agent-session-1", &folder.id)
        .await
        .expect("assign session");
    // Re-assigning is a no-op rather than an error.
    repos
        .assign_session_to_folder("default", "agent-session-1", &folder.id)
        .await
        .expect("assign session twice");

    let assignments = repos.list_session_folders().await.expect("list");
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].session_id, "agent-session-1");
    assert_eq!(assignments[0].folder_id, folder.id);

    repos
        .remove_session_from_folder("agent-session-1", &folder.id)
        .await
        .expect("remove session");
    assert!(repos
        .list_session_folders()
        .await
        .expect("list after remove")
        .is_empty());
}

#[tokio::test]
async fn assigning_session_to_folder_requires_both_to_match_the_active_profile() {
    let repos = repos().await;
    create_agent_session(&repos, "session-a").await;
    let folder_a = repos
        .create_folder("profile-a", "Profile A project", None)
        .await
        .expect("profile A project");
    let folder_b = repos
        .create_folder("profile-b", "Profile B project", None)
        .await
        .expect("profile B project");
    repos
        .assign_session_to_profile("session-a", "profile-a")
        .await
        .expect("profile A session");

    let session_mismatch = repos
        .assign_session_to_folder("profile-b", "session-a", &folder_b.id)
        .await
        .expect_err("profile A session must not be filed while profile B is active");
    assert_eq!(session_mismatch.code, "session_folder_profile_mismatch");

    let folder_mismatch = repos
        .assign_session_to_folder("profile-a", "session-a", &folder_b.id)
        .await
        .expect_err("profile B project must not accept a profile A session");
    assert_eq!(folder_mismatch.code, "session_folder_profile_mismatch");
    assert!(repos
        .list_session_folders()
        .await
        .expect("unchanged assignments")
        .is_empty());

    repos
        .assign_session_to_folder("profile-a", "session-a", &folder_a.id)
        .await
        .expect("matching profile assignment");
    let assignments = repos.list_session_folders().await.expect("assignments");
    assert_eq!(assignments.len(), 1);
    assert_eq!(assignments[0].session_id, "session-a");
    assert_eq!(assignments[0].folder_id, folder_a.id);
}

#[tokio::test]
async fn assigning_a_missing_session_is_rejected_even_in_default_partition() {
    let repos = repos().await;
    let folder = repos
        .create_folder("default", "Project", None)
        .await
        .unwrap();
    let error = repos
        .assign_session_to_folder("default", "missing-session", &folder.id)
        .await
        .unwrap_err();
    assert_eq!(error.code, "session_folder_profile_mismatch");
    assert!(repos.list_session_folders().await.unwrap().is_empty());
}

#[tokio::test]
async fn deleting_folder_drops_its_session_assignments() {
    let repos = repos().await;
    create_agent_session(&repos, "agent-session-1").await;
    let folder = repos
        .create_folder("default", "Launch", None)
        .await
        .expect("folder");
    repos
        .assign_session_to_folder("default", "agent-session-1", &folder.id)
        .await
        .expect("assign session");

    repos
        .delete_folder(&folder.id, false)
        .await
        .expect("delete folder");

    assert!(repos
        .list_session_folders()
        .await
        .expect("list after folder delete")
        .is_empty());
}
