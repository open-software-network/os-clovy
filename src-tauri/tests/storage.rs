use chrono::{SecondsFormat, Utc};
use clovy_lib::db::{migrations::run_migrations, repositories::Repositories};
use sqlx::query::query;
use sqlx::row::Row;
use sqlx_sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;
use tempfile::tempdir;

async fn test_repositories() -> Repositories {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("in-memory sqlite should open");
    run_migrations(&pool).await.expect("migrations should run");
    Repositories::new(pool)
}

async fn count_rows(repos: &Repositories, statement: &str) -> i64 {
    query(statement)
        .fetch_one(&repos.pool)
        .await
        .expect("count query should run")
        .get("count")
}

fn legacy_note_preview(
    title: &str,
    generated_content: Option<&str>,
    edited_content: Option<&str>,
) -> String {
    let content = edited_content.or(generated_content).unwrap_or_default();
    let source = if content.trim().is_empty() {
        title
    } else {
        content
    };
    source.chars().take(140).collect()
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
async fn migrations_create_empty_store() {
    let repos = test_repositories().await;

    let folders = repos
        .list_folders("default")
        .await
        .expect("folders list should load");
    let notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("notes list should load");

    assert!(folders.is_empty());
    assert!(notes.items.is_empty());
}

#[tokio::test]
async fn migrations_create_note_hydration_indexes() {
    let repos = test_repositories().await;
    let rows = query(
        "SELECT name
         FROM sqlite_master
         WHERE type = 'index'
           AND name IN (
             'idx_audio_artifacts_note_status_created_at',
             'idx_transcripts_note_created_at',
             'idx_recording_checkpoints_session_kind_created_at'
           )
         ORDER BY name",
    )
    .fetch_all(&repos.pool)
    .await
    .expect("note hydration indexes should be queryable");
    let names = rows
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "idx_audio_artifacts_note_status_created_at",
            "idx_recording_checkpoints_session_kind_created_at",
            "idx_transcripts_note_created_at",
        ]
    );
}

#[tokio::test]
async fn session_profiles_list_and_upsert_by_session_id() {
    let repos = test_repositories().await;

    repos
        .assign_session_to_profile("session-a", "a")
        .await
        .expect("session a profile should save");
    repos
        .assign_session_to_profile("session-b", "b")
        .await
        .expect("session b profile should save");

    let mut profiles = repos
        .list_session_profiles()
        .await
        .expect("session profiles should list");
    profiles.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].session_id, "session-a");
    assert_eq!(profiles[0].profile, "a");
    assert_eq!(profiles[1].session_id, "session-b");
    assert_eq!(profiles[1].profile, "b");

    repos
        .assign_session_to_profile("session-a", "b")
        .await
        .expect("session a profile should upsert");

    let mut profiles = repos
        .list_session_profiles()
        .await
        .expect("session profiles should list after upsert");
    profiles.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].session_id, "session-a");
    assert_eq!(profiles[0].profile, "b");
    assert_eq!(profiles[1].session_id, "session-b");
    assert_eq!(profiles[1].profile, "b");
}

#[tokio::test]
async fn migrations_tolerate_concurrent_startup() {
    let dir = tempdir().expect("tempdir");
    let database_path = dir.path().join("notes.sqlite3");
    let url = format!("sqlite://{}", database_path.display());
    let mut handles = Vec::new();

    for _ in 0..8 {
        let url = url.clone();
        handles.push(tokio::spawn(async move {
            let options = SqliteConnectOptions::from_str(&url)
                .expect("sqlite options")
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(2)
                .connect_with(options)
                .await
                .expect("sqlite file should open");
            run_migrations(&pool).await
        }));
    }

    for handle in handles {
        handle
            .await
            .expect("migration task should finish")
            .expect("concurrent migrations should be idempotent");
    }
}

#[tokio::test]
async fn notes_and_folders_are_partitioned_by_profile() {
    let repos = test_repositories().await;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    let folder_a = repos
        .create_folder("a", "Profile A folder", None)
        .await
        .expect("profile a folder");
    let folder_b = repos
        .create_folder("b", "Profile B folder", None)
        .await
        .expect("profile b folder");
    let note_a = repos
        .create_note("a", Some(folder_a.id.clone()))
        .await
        .expect("profile a note");
    let note_b = repos
        .create_note("b", Some(folder_b.id.clone()))
        .await
        .expect("profile b note");

    query(
        "INSERT INTO folders (id, name, created_at, updated_at)
         VALUES ('legacy-folder', 'Legacy folder', ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("legacy folder insert without profile");
    query(
        "INSERT INTO notes (id, title, processing_status, created_at, updated_at)
         VALUES ('legacy-note', 'Legacy note', 'draft', ?, ?)",
    )
    .bind(&now)
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("legacy note insert without profile");

    let folders_a = repos.list_folders("a").await.expect("profile a folders");
    let folders_b = repos.list_folders("b").await.expect("profile b folders");
    let default_folders = repos
        .list_folders("default")
        .await
        .expect("default folders");
    assert_eq!(
        folders_a
            .iter()
            .map(|folder| &folder.id)
            .collect::<Vec<_>>(),
        vec![&folder_a.id]
    );
    assert_eq!(
        folders_b
            .iter()
            .map(|folder| &folder.id)
            .collect::<Vec<_>>(),
        vec![&folder_b.id]
    );
    assert_eq!(
        default_folders
            .iter()
            .map(|folder| folder.id.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy-folder"]
    );

    let notes_a = repos
        .list_notes("a", None, 50, None)
        .await
        .expect("profile a notes");
    let notes_b = repos
        .list_notes("b", None, 50, None)
        .await
        .expect("profile b notes");
    let default_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("default notes");
    assert_eq!(
        notes_a
            .items
            .iter()
            .map(|note| &note.id)
            .collect::<Vec<_>>(),
        vec![&note_a.id]
    );
    assert_eq!(
        notes_b
            .items
            .iter()
            .map(|note| &note.id)
            .collect::<Vec<_>>(),
        vec![&note_b.id]
    );
    assert_eq!(
        default_notes
            .items
            .iter()
            .map(|note| note.id.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy-note"]
    );

    let folder_notes_a = repos
        .list_notes("a", Some(folder_a.id.clone()), 50, None)
        .await
        .expect("profile a folder notes");
    let folder_notes_b = repos
        .list_notes("b", Some(folder_a.id), 50, None)
        .await
        .expect("profile b folder notes");
    assert_eq!(
        folder_notes_a
            .items
            .iter()
            .map(|note| &note.id)
            .collect::<Vec<_>>(),
        vec![&note_a.id]
    );
    assert!(folder_notes_b.items.is_empty());
}

#[tokio::test]
async fn dictation_history_is_partitioned_by_profile() {
    let repos = test_repositories().await;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);

    let item_a = repos
        .create_dictation_history_item("a", "Profile A dictation.", None, "openai")
        .await
        .expect("profile a history create")
        .expect("profile a history item");
    let item_b = repos
        .create_dictation_history_item("b", "Profile B dictation.", None, "openai")
        .await
        .expect("profile b history create")
        .expect("profile b history item");
    query(
        "INSERT INTO dictation_history (id, text, language, provider, created_at)
         VALUES ('legacy-dictation', 'Legacy dictation.', NULL, 'openai', ?)",
    )
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("legacy dictation insert without profile");

    let history_a = repos
        .list_dictation_history("a", 50)
        .await
        .expect("profile a history");
    let history_b = repos
        .list_dictation_history("b", 50)
        .await
        .expect("profile b history");
    let default_history = repos
        .list_dictation_history("default", 50)
        .await
        .expect("default history");

    assert_eq!(
        history_a
            .items
            .iter()
            .map(|item| &item.id)
            .collect::<Vec<_>>(),
        vec![&item_a.id]
    );
    assert_eq!(
        history_b
            .items
            .iter()
            .map(|item| &item.id)
            .collect::<Vec<_>>(),
        vec![&item_b.id]
    );
    assert_eq!(
        default_history
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["legacy-dictation"]
    );
}

#[tokio::test]
async fn profile_data_summary_counts_owned_profile_rows() {
    let repos = test_repositories().await;

    repos
        .create_folder("x", "Profile X folder", None)
        .await
        .expect("profile x folder");
    repos.create_note("x", None).await.expect("profile x note");
    repos
        .create_dictation_history_item("x", "Profile X dictation.", None, "openai")
        .await
        .expect("profile x dictation");
    repos
        .assign_session_to_profile("session-x", "x")
        .await
        .expect("profile x session");
    repos
        .create_memory("x", None, "Profile X memory", "user")
        .await
        .expect("profile x memory");

    repos
        .create_folder("other", "Other folder", None)
        .await
        .expect("other folder");
    repos.create_note("other", None).await.expect("other note");
    repos
        .create_dictation_history_item("other", "Other dictation.", None, "openai")
        .await
        .expect("other dictation");
    repos
        .assign_session_to_profile("session-other", "other")
        .await
        .expect("other session");
    repos
        .create_memory("other", None, "Other memory", "user")
        .await
        .expect("other memory");

    let summary = repos
        .profile_data_summary("x")
        .await
        .expect("profile data summary");

    assert_eq!(summary.notes, 1);
    assert_eq!(summary.dictation, 1);
    assert_eq!(summary.folders, 1);
    assert_eq!(summary.sessions, 1);
    assert_eq!(summary.memories, 1);
}

#[tokio::test]
async fn move_profile_data_to_default_retags_owned_rows_only() {
    let repos = test_repositories().await;

    let folder_x = repos
        .create_folder("x", "Profile X folder", None)
        .await
        .expect("profile x folder");
    let note_x = repos
        .create_note("x", Some(folder_x.id.clone()))
        .await
        .expect("profile x note");
    let dictation_x = repos
        .create_dictation_history_item("x", "Profile X dictation.", None, "openai")
        .await
        .expect("profile x dictation")
        .expect("profile x dictation item");
    repos
        .assign_session_to_profile("session-x", "x")
        .await
        .expect("profile x session");
    let memory_x = repos
        .create_memory("x", Some(&folder_x.id), "Profile X memory", "user")
        .await
        .expect("profile x memory");

    let folder_default = repos
        .create_folder("default", "Default folder", None)
        .await
        .expect("default folder");
    let note_default = repos
        .create_note("default", Some(folder_default.id.clone()))
        .await
        .expect("default note");
    let dictation_default = repos
        .create_dictation_history_item("default", "Default dictation.", None, "openai")
        .await
        .expect("default dictation")
        .expect("default dictation item");
    repos
        .assign_session_to_profile("session-default", "default")
        .await
        .expect("default session");
    let memory_default = repos
        .create_memory("default", None, "Default memory", "user")
        .await
        .expect("default memory");

    let folder_other = repos
        .create_folder("other", "Other folder", None)
        .await
        .expect("other folder");
    let note_other = repos
        .create_note("other", Some(folder_other.id.clone()))
        .await
        .expect("other note");
    let dictation_other = repos
        .create_dictation_history_item("other", "Other dictation.", None, "openai")
        .await
        .expect("other dictation")
        .expect("other dictation item");
    repos
        .assign_session_to_profile("session-other", "other")
        .await
        .expect("other session");
    let memory_other = repos
        .create_memory("other", None, "Other memory", "user")
        .await
        .expect("other memory");

    repos
        .move_profile_data_to_default("x", None)
        .await
        .expect("move profile data");

    let summary_x = repos
        .profile_data_summary("x")
        .await
        .expect("profile x summary after move");
    assert_eq!(summary_x.notes, 0);
    assert_eq!(summary_x.dictation, 0);
    assert_eq!(summary_x.folders, 0);
    assert_eq!(summary_x.sessions, 0);
    assert_eq!(summary_x.memories, 0);

    let default_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("default notes");
    let default_note_ids = default_notes
        .items
        .iter()
        .map(|note| note.id.as_str())
        .collect::<Vec<_>>();
    assert!(default_note_ids.contains(&note_x.id.as_str()));
    assert!(default_note_ids.contains(&note_default.id.as_str()));

    let other_notes = repos
        .list_notes("other", None, 50, None)
        .await
        .expect("other notes");
    assert_eq!(
        other_notes
            .items
            .iter()
            .map(|note| note.id.as_str())
            .collect::<Vec<_>>(),
        vec![note_other.id.as_str()]
    );

    let default_folders = repos
        .list_folders("default")
        .await
        .expect("default folders");
    let default_folder_ids = default_folders
        .iter()
        .map(|folder| folder.id.as_str())
        .collect::<Vec<_>>();
    assert!(default_folder_ids.contains(&folder_x.id.as_str()));
    assert!(default_folder_ids.contains(&folder_default.id.as_str()));

    let other_folders = repos.list_folders("other").await.expect("other folders");
    assert_eq!(
        other_folders
            .iter()
            .map(|folder| folder.id.as_str())
            .collect::<Vec<_>>(),
        vec![folder_other.id.as_str()]
    );

    let default_history = repos
        .list_dictation_history("default", 50)
        .await
        .expect("default dictation");
    let default_history_ids = default_history
        .items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<Vec<_>>();
    assert!(default_history_ids.contains(&dictation_x.id.as_str()));
    assert!(default_history_ids.contains(&dictation_default.id.as_str()));

    let other_history = repos
        .list_dictation_history("other", 50)
        .await
        .expect("other dictation");
    assert_eq!(
        other_history
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![dictation_other.id.as_str()]
    );

    let default_memories = repos
        .list_memories("default", None, true)
        .await
        .expect("default memories");
    let default_memory_ids = default_memories
        .iter()
        .map(|memory| memory.id.as_str())
        .collect::<Vec<_>>();
    assert!(default_memory_ids.contains(&memory_x.id.as_str()));
    assert!(default_memory_ids.contains(&memory_default.id.as_str()));
    assert_eq!(
        repos
            .list_memories("other", None, true)
            .await
            .expect("other memories"),
        vec![memory_other]
    );

    let mut session_profiles = repos
        .list_session_profiles()
        .await
        .expect("session profiles after move");
    session_profiles.sort_by(|left, right| left.session_id.cmp(&right.session_id));
    assert_eq!(
        session_profiles
            .iter()
            .map(|entry| (entry.session_id.as_str(), entry.profile.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("session-default", "default"),
            ("session-other", "other"),
            ("session-x", "default"),
        ]
    );
}

#[tokio::test]
async fn delete_profile_data_removes_owned_rows_and_cascades_satellites() {
    let repos = test_repositories().await;
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    create_agent_session(&repos, "session-x").await;
    create_agent_session(&repos, "session-default").await;

    let folder_x = repos
        .create_folder("x", "Profile X folder", None)
        .await
        .expect("profile x folder");
    let note_x = repos
        .create_note("x", Some(folder_x.id.clone()))
        .await
        .expect("profile x note");
    repos
        .create_dictation_history_item("x", "Profile X dictation.", None, "openai")
        .await
        .expect("profile x dictation");
    repos
        .assign_session_to_profile("session-x", "x")
        .await
        .expect("profile x session profile");
    repos
        .assign_session_to_folder("x", "session-x", &folder_x.id)
        .await
        .expect("profile x session folder");
    let memory_x = repos
        .create_memory("x", Some(&folder_x.id), "Profile X memory", "agent")
        .await
        .expect("profile x memory");

    let folder_default = repos
        .create_folder("default", "Default folder", None)
        .await
        .expect("default folder");
    let note_default = repos
        .create_note("default", Some(folder_default.id))
        .await
        .expect("default note");
    repos
        .create_dictation_history_item("default", "Default dictation.", None, "openai")
        .await
        .expect("default dictation");
    repos
        .assign_session_to_profile("session-default", "default")
        .await
        .expect("default session profile");
    let memory_default = repos
        .create_memory("default", None, "Default memory", "user")
        .await
        .expect("default memory");

    query(
        "INSERT INTO recording_sessions
           (id, note_id, status, started_at, partial_path, final_path)
         VALUES (
           'recording-x', ?, 'completed', ?,
           '/tmp/audio-x.partial.wav', '/tmp/audio-x.wav'
         )",
    )
    .bind(&note_x.id)
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("recording session");
    query(
        "INSERT INTO recording_checkpoints (id, recording_session_id, kind, created_at)
         VALUES ('checkpoint-x', 'recording-x', 'validated', ?)",
    )
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("recording checkpoint");
    query(
        "INSERT INTO audio_artifacts
           (id, note_id, recording_session_id, partial_path, path, format, duration_ms, size_bytes, checksum, created_at)
         VALUES (
           'audio-x', ?, 'recording-x',
           '/tmp/audio-x.partial.wav', '/tmp/audio-x.wav',
           'wav', 1000, 10, 'checksum', ?
         )",
    )
    .bind(&note_x.id)
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("audio artifact");
    query(
        "INSERT INTO transcripts
           (id, note_id, audio_artifact_id, text, provider, status, created_at, updated_at)
         VALUES ('transcript-x', ?, 'audio-x', 'hello', 'openai', 'succeeded', ?, ?)",
    )
    .bind(&note_x.id)
    .bind(&now)
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("transcript");
    query(
        "INSERT INTO generation_results
           (id, note_id, transcript_id, provider, prompt_version, status, created_at, updated_at)
         VALUES ('generation-x', ?, 'transcript-x', 'openrouter', 'v1', 'succeeded', ?, ?)",
    )
    .bind(&note_x.id)
    .bind(&now)
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("generation result");
    query(
        "INSERT INTO note_generation_blocks
           (id, note_id, recording_session_id, generation_result_id, content, sort_order, created_at, updated_at)
         VALUES ('block-x', ?, 'recording-x', 'generation-x', 'generated', 0, ?, ?)",
    )
    .bind(&note_x.id)
    .bind(&now)
    .bind(&now)
    .execute(&repos.pool)
    .await
    .expect("generation block");

    let mut profile_audio_paths = repos
        .audio_artifact_paths_for_profile("x")
        .await
        .expect("profile audio paths");
    profile_audio_paths.sort();
    assert_eq!(
        profile_audio_paths,
        vec![
            "/tmp/audio-x.partial.wav".to_string(),
            "/tmp/audio-x.wav".to_string(),
        ]
    );

    repos
        .delete_profile_data("x")
        .await
        .expect("delete profile data");

    let summary_x = repos
        .profile_data_summary("x")
        .await
        .expect("profile x summary after delete");
    assert_eq!(summary_x.notes, 0);
    assert_eq!(summary_x.dictation, 0);
    assert_eq!(summary_x.folders, 0);
    assert_eq!(summary_x.sessions, 0);
    assert_eq!(summary_x.memories, 0);

    assert_eq!(
        count_rows(&repos, "SELECT COUNT(*) AS count FROM note_folders").await,
        1
    );
    assert_eq!(
        count_rows(&repos, "SELECT COUNT(*) AS count FROM session_folders").await,
        0
    );
    assert_eq!(
        count_rows(&repos, "SELECT COUNT(*) AS count FROM recording_sessions").await,
        0
    );
    assert_eq!(
        count_rows(
            &repos,
            "SELECT COUNT(*) AS count FROM recording_checkpoints"
        )
        .await,
        0
    );
    assert_eq!(
        count_rows(&repos, "SELECT COUNT(*) AS count FROM audio_artifacts").await,
        0
    );
    assert_eq!(
        count_rows(&repos, "SELECT COUNT(*) AS count FROM transcripts").await,
        0
    );
    assert_eq!(
        count_rows(&repos, "SELECT COUNT(*) AS count FROM generation_results").await,
        0
    );
    assert_eq!(
        count_rows(
            &repos,
            "SELECT COUNT(*) AS count FROM note_generation_blocks"
        )
        .await,
        0
    );

    assert_eq!(
        repos
            .list_memories("default", None, true)
            .await
            .expect("default memories"),
        vec![memory_default]
    );
    let tombstone_ids = query("SELECT id FROM memory_tombstones")
        .fetch_all(&repos.pool)
        .await
        .expect("memory tombstones")
        .into_iter()
        .map(|row| row.get::<String, _>("id"))
        .collect::<Vec<_>>();
    assert_eq!(tombstone_ids, vec![memory_x.id]);

    let default_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("default notes");
    assert_eq!(
        default_notes
            .items
            .iter()
            .map(|note| note.id.as_str())
            .collect::<Vec<_>>(),
        vec![note_default.id.as_str()]
    );
    assert_eq!(
        repos
            .list_dictation_history("default", 50)
            .await
            .expect("default dictation")
            .items
            .len(),
        1
    );
    assert_eq!(
        repos
            .list_session_profiles()
            .await
            .expect("session profiles")
            .iter()
            .map(|entry| (entry.session_id.as_str(), entry.profile.as_str()))
            .collect::<Vec<_>>(),
        vec![("session-default", "default")]
    );

    repos
        .delete_profile_data("default")
        .await
        .expect("delete default profile data is a no-op");
    assert_eq!(
        repos
            .profile_data_summary("default")
            .await
            .expect("default summary")
            .notes,
        1
    );
}

#[tokio::test]
async fn creates_notes_in_reverse_chronological_order() {
    let repos = test_repositories().await;

    let first = repos
        .create_note("default", None)
        .await
        .expect("first note");
    let second = repos
        .create_note("default", None)
        .await
        .expect("second note");

    let notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("notes list should load");

    assert_eq!(notes.items.len(), 2);
    assert_eq!(notes.items[0].id, second.id);
    assert_eq!(notes.items[1].id, first.id);
}

#[tokio::test]
async fn list_notes_sql_previews_match_the_legacy_derivation() {
    let repos = test_repositories().await;
    let unicode_content = format!("{}Zażółć gęślą jaźń 你好", "🦀".repeat(145));
    let cases = vec![
        (
            "Markdown title",
            Some("# Heading\n\n- first\n- **second**".to_string()),
            None,
        ),
        (
            "Edited content is intentionally empty",
            Some("Generated content must not win".to_string()),
            Some(String::new()),
        ),
        ("Unicode title", Some(unicode_content), None),
        (
            "Unicode whitespace falls back to the title",
            Some("\u{2003}\u{00a0}\u{3000}".to_string()),
            None,
        ),
        (
            "Long leading whitespace is still content",
            Some(format!("{}body", " ".repeat(145))),
            None,
        ),
    ];

    let mut expected = Vec::new();
    for (title, generated_content, edited_content) in cases {
        let note = repos
            .create_note("default", None)
            .await
            .expect("create preview note");
        query(
            "UPDATE notes
             SET title = ?, generated_content = ?, edited_content = ?
             WHERE id = ?",
        )
        .bind(title)
        .bind(generated_content.as_deref())
        .bind(edited_content.as_deref())
        .bind(&note.id)
        .execute(&repos.pool)
        .await
        .expect("seed preview content");
        expected.push((
            note.id,
            legacy_note_preview(
                title,
                generated_content.as_deref(),
                edited_content.as_deref(),
            ),
        ));
    }

    let listed = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("list notes with SQL previews");
    for (note_id, expected_preview) in expected {
        let item = listed
            .items
            .iter()
            .find(|item| item.id == note_id)
            .expect("preview note should be listed");
        assert_eq!(item.preview, expected_preview);
        assert!(item.preview.chars().count() <= 140);
    }
}

#[tokio::test]
async fn list_notes_cursor_paginates_across_timestamp_and_rowid_boundaries() {
    let repos = test_repositories().await;
    let oldest = repos
        .create_note("default", None)
        .await
        .expect("oldest note");
    let tied_earlier = repos
        .create_note("default", None)
        .await
        .expect("earlier tied note");
    let tied_later = repos
        .create_note("default", None)
        .await
        .expect("later tied note");
    let newest = repos
        .create_note("default", None)
        .await
        .expect("newest note");
    let final_note = repos
        .create_note("default", None)
        .await
        .expect("final note");

    for (note_id, created_at) in [
        (&oldest.id, "2026-07-23T10:01:00.000Z"),
        (&tied_earlier.id, "2026-07-23T10:02:00.000Z"),
        (&tied_later.id, "2026-07-23T10:02:00.000Z"),
        (&newest.id, "2026-07-23T10:03:00.000Z"),
        (&final_note.id, "2026-07-23T10:00:00.000Z"),
    ] {
        query("UPDATE notes SET created_at = ? WHERE id = ?")
            .bind(created_at)
            .bind(note_id)
            .execute(&repos.pool)
            .await
            .expect("set pagination timestamp");
    }

    let first_page = repos
        .list_notes("default", None, 2, None)
        .await
        .expect("first page");
    assert_eq!(
        first_page
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![newest.id.as_str(), tied_later.id.as_str()]
    );
    let first_cursor = first_page.next_cursor.expect("first page cursor");

    let second_page = repos
        .list_notes("default", None, 2, Some(first_cursor))
        .await
        .expect("second page");
    assert_eq!(
        second_page
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![tied_earlier.id.as_str(), oldest.id.as_str()]
    );
    let second_cursor = second_page.next_cursor.expect("second page cursor");

    let third_page = repos
        .list_notes("default", None, 2, Some(second_cursor))
        .await
        .expect("third page");
    assert_eq!(
        third_page
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec![final_note.id.as_str()]
    );
    assert!(third_page.next_cursor.is_none());
}

#[tokio::test]
async fn list_notes_groups_all_folder_assignments_in_the_page_query() {
    let repos = test_repositories().await;
    let alpha = repos
        .create_folder("default", "Alpha", None)
        .await
        .expect("alpha folder");
    let beta = repos
        .create_folder("default", "Beta", None)
        .await
        .expect("beta folder");
    let gamma = repos
        .create_folder("default", "Gamma", None)
        .await
        .expect("gamma folder");
    let archived = repos
        .create_folder("default", "Archived", None)
        .await
        .expect("archived folder");
    let first = repos
        .create_note("default", None)
        .await
        .expect("first note");
    let second = repos
        .create_note("default", None)
        .await
        .expect("second note");

    for (note_id, folder_id, assigned_at) in [
        (&first.id, &alpha.id, "2026-07-23T10:00:00.000Z"),
        (&first.id, &archived.id, "2026-07-23T10:00:30.000Z"),
        (&first.id, &beta.id, "2026-07-23T10:01:00.000Z"),
        (&second.id, &gamma.id, "2026-07-23T10:00:00.000Z"),
        (&second.id, &alpha.id, "2026-07-23T10:01:00.000Z"),
    ] {
        query(
            "INSERT INTO note_folders (note_id, folder_id, assigned_at)
             VALUES (?, ?, ?)",
        )
        .bind(note_id)
        .bind(folder_id)
        .bind(assigned_at)
        .execute(&repos.pool)
        .await
        .expect("assign note folder");
    }
    query("UPDATE folders SET deleted_at = ? WHERE id = ?")
        .bind("2026-07-23T10:02:00.000Z")
        .bind(&archived.id)
        .execute(&repos.pool)
        .await
        .expect("soft-delete archived folder");

    let all_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("list all notes");
    let first_item = all_notes
        .items
        .iter()
        .find(|item| item.id == first.id)
        .expect("first note should be listed");
    assert_eq!(first_item.folder_ids, vec![alpha.id.clone(), beta.id]);
    let second_item = all_notes
        .items
        .iter()
        .find(|item| item.id == second.id)
        .expect("second note should be listed");
    assert_eq!(second_item.folder_ids, vec![gamma.id, alpha.id.clone()]);

    let alpha_notes = repos
        .list_notes("default", Some(alpha.id.clone()), 50, None)
        .await
        .expect("list alpha notes");
    assert_eq!(alpha_notes.items.len(), 2);
    assert_eq!(
        alpha_notes
            .items
            .iter()
            .find(|item| item.id == first.id)
            .expect("first alpha note")
            .folder_ids,
        first_item.folder_ids
    );
    assert_eq!(
        alpha_notes
            .items
            .iter()
            .find(|item| item.id == second.id)
            .expect("second alpha note")
            .folder_ids,
        second_item.folder_ids
    );

    let first_alpha_page = repos
        .list_notes("default", Some(alpha.id.clone()), 1, None)
        .await
        .expect("list first alpha page");
    let alpha_cursor = first_alpha_page
        .next_cursor
        .expect("first alpha page cursor");
    let second_alpha_page = repos
        .list_notes("default", Some(alpha.id), 1, Some(alpha_cursor))
        .await
        .expect("list second alpha page");
    assert_eq!(first_alpha_page.items.len(), 1);
    assert_eq!(second_alpha_page.items.len(), 1);
    assert_ne!(first_alpha_page.items[0].id, second_alpha_page.items[0].id);
    assert!(second_alpha_page.next_cursor.is_none());
}

#[tokio::test]
async fn creates_folders_and_assigns_notes_without_removing_all_notes_visibility() {
    let repos = test_repositories().await;
    let folder = repos
        .create_folder("default", "Field Notes", None)
        .await
        .expect("folder should be created");
    let note = repos
        .create_note("default", Some(folder.id.clone()))
        .await
        .expect("note should be created");

    let all_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("all notes should load");
    let folder_notes = repos
        .list_notes("default", Some(folder.id.clone()), 50, None)
        .await
        .expect("folder notes should load");

    assert_eq!(
        all_notes
            .items
            .iter()
            .map(|item| &item.id)
            .collect::<Vec<_>>(),
        vec![&note.id]
    );
    assert_eq!(
        folder_notes
            .items
            .iter()
            .map(|item| &item.id)
            .collect::<Vec<_>>(),
        vec![&note.id]
    );
    assert_eq!(folder_notes.items[0].folder_ids, vec![folder.id]);
}

#[tokio::test]
async fn deletes_note_and_removes_folder_assignment() {
    let repos = test_repositories().await;
    let folder = repos
        .create_folder("default", "Calls", None)
        .await
        .expect("folder");
    let note = repos
        .create_note("default", Some(folder.id.clone()))
        .await
        .expect("note");

    repos.delete_note(&note.id).await.expect("delete note");

    let all_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("all notes should load");
    let folder_notes = repos
        .list_notes("default", Some(folder.id), 50, None)
        .await
        .expect("folder notes should load");

    assert!(all_notes.items.is_empty());
    assert!(folder_notes.items.is_empty());
}
