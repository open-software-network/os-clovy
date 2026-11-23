use clovy_lib::{
    db::{migrations::run_migrations, repositories::Repositories},
    domain::types::{ProcessingStatus, RecordingSourceMode},
};
use sqlx::query::query;
use sqlx::row::Row;
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

#[tokio::test]
async fn updates_title_body_and_active_tab() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    let updated = repos
        .update_note(
            &note.id,
            Some("Edited title".to_string()),
            Some("Edited body".to_string()),
            Some("transcription".to_string()),
        )
        .await
        .expect("update");

    assert_eq!(updated.title, "Edited title");
    assert_eq!(updated.edited_content.as_deref(), Some("Edited body"));
    assert_eq!(updated.active_tab.as_deref(), Some("transcription"));
}

#[tokio::test]
async fn update_note_returns_its_patch_without_hydrating_related_note_data() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    // A fully hydrated NoteDto reads transcripts after the note row. Removing
    // the relation makes this a regression guard that update_note executes its
    // UPDATE ... RETURNING statement without a pre/post hydration round trip.
    query("DROP TABLE transcripts")
        .execute(&repos.pool)
        .await
        .expect("drop transcript relation");

    let updated = repos
        .update_note(
            &note.id,
            Some("Single statement".to_string()),
            Some("Returned patch".to_string()),
            None,
        )
        .await
        .expect("single-query update");

    assert_eq!(updated.id, note.id);
    assert_eq!(updated.title, "Single statement");
    assert_eq!(updated.preview, "Returned patch");
    assert_eq!(updated.edited_content.as_deref(), Some("Returned patch"));
    assert_eq!(updated.active_tab.as_deref(), Some("notes"));
    let persisted = query("SELECT title, edited_content, active_tab FROM notes WHERE id = ?")
        .bind(&note.id)
        .fetch_one(&repos.pool)
        .await
        .expect("persisted note row");
    assert_eq!(persisted.get::<String, _>("title"), "Single statement");
    assert_eq!(
        persisted
            .get::<Option<String>, _>("edited_content")
            .as_deref(),
        Some("Returned patch")
    );
    assert_eq!(
        persisted.get::<Option<String>, _>("active_tab").as_deref(),
        Some("notes")
    );
}

#[tokio::test]
async fn update_note_returns_null_content_and_normalizes_a_null_active_tab() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    query("UPDATE notes SET edited_content = NULL, active_tab = NULL WHERE id = ?")
        .bind(&note.id)
        .execute(&repos.pool)
        .await
        .expect("nullable note row");

    let updated = repos
        .update_note(&note.id, Some("Title only".to_string()), None, None)
        .await
        .expect("patch nullable row");

    assert_eq!(updated.title, "Title only");
    assert_eq!(updated.edited_content, None);
    assert_eq!(updated.active_tab.as_deref(), Some("notes"));
}

#[tokio::test]
async fn autosave_omits_title_after_generation_commits_a_newer_title() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let autosave_snapshot = repos.get_note(&note.id).await.expect("autosave snapshot");
    assert_eq!(autosave_snapshot.title, "");

    let generated = repos
        .set_generated_note(
            &note.id,
            Some("Freshly generated title".to_string()),
            "Generated content".to_string(),
        )
        .await
        .expect("generated note");
    assert_eq!(generated.title, "Freshly generated title");

    // The debounced autosave is still based on the pre-generation editor
    // snapshot. Fail deterministically if its content-only patch touches the
    // title column at all; doing so can rewrite the stale snapshot over the
    // title that generation committed in the meantime.
    query(
        "CREATE TRIGGER reject_stale_autosave_title_write
         BEFORE UPDATE OF title ON notes
         BEGIN
             SELECT RAISE(ABORT, 'content autosave must not write title');
         END",
    )
    .execute(&repos.pool)
    .await
    .expect("install title-write guard");

    let updated = repos
        .update_note(
            &note.id,
            None,
            Some("Content saved from the older editor snapshot".to_string()),
            None,
        )
        .await
        .expect("content autosave");

    assert_eq!(updated.title, "Freshly generated title");
    assert_eq!(
        updated.edited_content.as_deref(),
        Some("Content saved from the older editor snapshot")
    );
}

#[tokio::test]
async fn deleting_note_removes_it_from_all_note_lists() {
    let repos = repos().await;
    let folder = repos
        .create_folder("default", "Work", None)
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
        .expect("all notes");
    assert!(all_notes.items.is_empty());

    let folder_notes = repos
        .list_notes("default", Some(folder.id), 50, None)
        .await
        .expect("folder notes");
    assert!(folder_notes.items.is_empty());
}

#[tokio::test]
async fn deleting_notes_removes_multiple_notes_from_all_note_lists() {
    let repos = repos().await;
    let folder = repos
        .create_folder("default", "Work", None)
        .await
        .expect("folder");
    let first = repos
        .create_note("default", Some(folder.id.clone()))
        .await
        .expect("first note");
    let second = repos
        .create_note("default", Some(folder.id.clone()))
        .await
        .expect("second note");
    let retained = repos
        .create_note("default", Some(folder.id.clone()))
        .await
        .expect("retained note");

    repos
        .delete_notes(&[first.id.clone(), second.id.clone()])
        .await
        .expect("delete notes");

    let all_notes = repos
        .list_notes("default", None, 50, None)
        .await
        .expect("all notes");
    assert_eq!(all_notes.items.len(), 1);
    assert_eq!(all_notes.items[0].id, retained.id);

    let folder_notes = repos
        .list_notes("default", Some(folder.id), 50, None)
        .await
        .expect("folder notes");
    assert_eq!(folder_notes.items.len(), 1);
    assert_eq!(folder_notes.items[0].id, retained.id);
}

#[tokio::test]
async fn generated_note_returns_to_notes_tab() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .update_note(&note.id, None, None, Some("transcription".to_string()))
        .await
        .expect("tab update");

    let generated = repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "Generated content".to_string(),
        )
        .await
        .expect("generated note");

    assert_eq!(generated.active_tab.as_deref(), Some("notes"));
    assert_eq!(
        generated.generated_content.as_deref(),
        Some("Generated content")
    );
}

#[tokio::test]
async fn generated_note_derives_title_when_provider_returns_placeholder() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    let generated = repos
        .set_generated_note(
            &note.id,
            Some("Untitled note".to_string()),
            "- Conversation about rate limits for upcoming launch\n- Sabrina can help with higher limits"
                .to_string(),
        )
        .await
        .expect("generated note");

    assert_eq!(
        generated.title,
        "Conversation about rate limits for upcoming launch"
    );
}

#[tokio::test]
async fn generated_note_derives_title_from_all_section_headings() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    let generated = repos
        .set_generated_note(
            &note.id,
            Some("Untitled note".to_string()),
            "# Budget review\n\n- Costs are rising\n\n# Hiring plan\n\n- Open roles are approved\n\n# Product launch\n\n- The team is keeping the launch date"
                .to_string(),
        )
        .await
        .expect("generated note");

    assert_eq!(
        generated.title,
        "Budget review, Hiring plan, and Product launch"
    );
}

#[tokio::test]
async fn generated_note_title_uses_subjects_not_fixed_sections() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    let generated = repos
        .set_generated_note(
            &note.id,
            Some("New note".to_string()),
            "# Resumo\n\nReunião gravada em 28 de outubro de 2026.\n\n# Discussão\n\n## Integrações DHL e Azul\n\n- Falta a credencial.\n\n## Site para a loja de carros\n\n- Vitrine sem venda online.\n\n# Tarefas\n\n- **Cliente:** enviar o logo"
                .to_string(),
        )
        .await
        .expect("generated note");

    assert_eq!(
        generated.title,
        "Integrações DHL e Azul and Site para a loja de carros"
    );
}

#[tokio::test]
async fn generated_note_title_uses_continuation_subjects() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    let generated = repos
        .set_generated_note(
            &note.id,
            Some("New note".to_string()),
            "# Continuação — 30 de outubro de 2026\n\nNesta parte.\n\n## Discussão\n\n### Credencial da Azul\n\n- Encontrada.\n\n## Tarefas\n\n- **Dev:** deploy"
                .to_string(),
        )
        .await
        .expect("generated note");

    assert_eq!(generated.title, "Credencial da Azul");
}

#[tokio::test]
async fn generated_note_appends_to_existing_generated_content() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "First recording".to_string(),
        )
        .await
        .expect("first generated note");

    let updated = repos
        .set_generated_note(&note.id, None, "Second recording".to_string())
        .await
        .expect("second generated note");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some("First recording\n\nSecond recording")
    );
    assert_eq!(updated.edited_content, None);
}

#[tokio::test]
async fn generated_note_does_not_duplicate_full_regenerated_content() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "First recording".to_string(),
        )
        .await
        .expect("first generated note");

    let updated = repos
        .set_generated_note(
            &note.id,
            None,
            "First recording\n\nSecond recording".to_string(),
        )
        .await
        .expect("full regenerated note");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some("First recording\n\nSecond recording")
    );
}

#[tokio::test]
async fn generated_note_strips_placeholder_heading_when_appending() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "First recording".to_string(),
        )
        .await
        .expect("first generated note");

    let updated = repos
        .set_generated_note(&note.id, None, "# New note\n\nSecond recording".to_string())
        .await
        .expect("second generated note");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some("First recording\n\nSecond recording")
    );
}

#[tokio::test]
async fn generated_note_strips_existing_note_prefix_when_appending() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "Hola hola\n\nSegundo test".to_string(),
        )
        .await
        .expect("first generated note");

    let updated = repos
        .set_generated_note(
            &note.id,
            None,
            "Hola hola\n\nSegundo test\n\nTres, tres, dos, uno.".to_string(),
        )
        .await
        .expect("second generated note");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some("Hola hola\n\nSegundo test\n\nTres, tres, dos, uno.")
    );
}

#[tokio::test]
async fn generated_note_strips_manual_note_echo_when_appending() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"".to_string(),
        )
        .await
        .expect("first generated note");
    repos
        .update_note(
            &note.id,
            None,
            Some(
                "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"\n\nTest 2".to_string(),
            ),
            Some("notes".to_string()),
        )
        .await
        .expect("manual note");

    let updated = repos
        .set_generated_note(
            &note.id,
            None,
            "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"\n\nTest 2:\n\n## Test 2\n\n- Transcript: \"Tres, dos, uno, hola hola.\""
                .to_string(),
        )
        .await
        .expect("second generated note");

    assert_eq!(
        updated.edited_content.as_deref(),
        Some(
            "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"\n\nTest 2\n\n- Transcript: \"Tres, dos, uno, hola hola.\""
        )
    );
}

#[tokio::test]
async fn generated_note_replaces_existing_session_block() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            Some("Generated title".to_string()),
            "First transcript result".to_string(),
        )
        .await
        .expect("first generated block");

    let updated = repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            None,
            "Retried transcript result".to_string(),
        )
        .await
        .expect("retried generated block");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some("Retried transcript result")
    );
}

#[tokio::test]
async fn generated_note_composes_distinct_session_blocks_in_order() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            Some("Generated title".to_string()),
            "First transcript result".to_string(),
        )
        .await
        .expect("first generated block");

    let updated = repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-2"),
            None,
            None,
            "First transcript result\n\nSecond transcript result".to_string(),
        )
        .await
        .expect("second generated block");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some("First transcript result\n\nSecond transcript result")
    );
}

#[tokio::test]
async fn generated_note_preserves_user_edit_written_after_its_read() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");

    // Generation inserts its session block after reading the note and before
    // updating it. Use that exact boundary to deterministically model an
    // autosave that commits while generation is still working from its stale
    // `edited_content = NULL` snapshot.
    query(
        "CREATE TRIGGER simulate_concurrent_note_autosave
         AFTER INSERT ON note_generation_blocks
         BEGIN
             UPDATE notes
             SET title = 'User title persisted during generation',
                 edited_content = 'User edit persisted during generation'
             WHERE id = NEW.note_id;
         END",
    )
    .execute(&repos.pool)
    .await
    .expect("install concurrent autosave trigger");

    let updated = repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            Some("Generated title".to_string()),
            "Generated content".to_string(),
        )
        .await
        .expect("generated note");

    assert_eq!(updated.title, "User title persisted during generation");
    assert_eq!(
        updated.edited_content.as_deref(),
        Some("User edit persisted during generation")
    );
    assert_eq!(
        updated.generated_content.as_deref(),
        Some("Generated content")
    );
}

#[tokio::test]
async fn generated_note_preserves_newer_non_null_user_edit_written_after_its_read() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .update_note(&note.id, None, Some("Initial user edit".to_string()), None)
        .await
        .expect("initial user edit");

    // The trigger fires after generation has read the existing edit and
    // changes it before generation's final compare-and-set update.
    query(
        "CREATE TRIGGER simulate_concurrent_non_null_note_autosave
         AFTER INSERT ON note_generation_blocks
         BEGIN
             UPDATE notes
             SET edited_content = 'Newer user edit persisted during generation'
             WHERE id = NEW.note_id;
         END",
    )
    .execute(&repos.pool)
    .await
    .expect("install concurrent autosave trigger");

    let updated = repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            Some("Generated title".to_string()),
            "Generated content".to_string(),
        )
        .await
        .expect("generated note");

    assert_eq!(
        updated.edited_content.as_deref(),
        Some("Newer user edit persisted during generation")
    );
    assert_eq!(
        updated.generated_content.as_deref(),
        Some("Generated content")
    );
}

#[tokio::test]
async fn generated_session_block_strips_manual_note_echo_before_composing() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            Some("Generated title".to_string()),
            "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"".to_string(),
        )
        .await
        .expect("first generated block");
    repos
        .update_note(
            &note.id,
            None,
            Some(
                "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"\n\nTest 2".to_string(),
            ),
            Some("notes".to_string()),
        )
        .await
        .expect("manual note");

    let updated = repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-2"),
            None,
            None,
            "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"\n\nTest 2:\n\n## Test 2\n\n- Transcript: \"Tres, dos, uno, hola hola.\""
                .to_string(),
        )
        .await
        .expect("second generated block");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some(
            "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"\n\n- Transcript: \"Tres, dos, uno, hola hola.\""
        )
    );
    assert_eq!(
        updated.edited_content.as_deref(),
        Some(
            "## Transcript (verbatim)\n\n- \"Un, dos, tres, hola hola.\"\n\nTest 2\n\n- Transcript: \"Tres, dos, uno, hola hola.\""
        )
    );
}

#[tokio::test]
async fn first_generated_session_block_strips_initial_manual_note_echo() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .update_note(
            &note.id,
            None,
            Some("Test 1:".to_string()),
            Some("notes".to_string()),
        )
        .await
        .expect("manual note");

    let updated = repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            Some("Generated title".to_string()),
            "Test 1:\n\n- Test 1\n- \"Test uno, probando probando, un dos tres.\"".to_string(),
        )
        .await
        .expect("first generated block");

    assert_eq!(
        updated.generated_content.as_deref(),
        Some("- \"Test uno, probando probando, un dos tres.\"")
    );
    assert_eq!(
        updated.edited_content.as_deref(),
        Some("Test 1:\n\n- \"Test uno, probando probando, un dos tres.\"")
    );
}

#[tokio::test]
async fn generated_session_block_strips_generic_note_heading_after_manual_echo() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-1"),
            None,
            Some("Generated title".to_string()),
            "- \"Test uno, probando probando, un dos tres.\"".to_string(),
        )
        .await
        .expect("first generated block");
    repos
        .update_note(
            &note.id,
            None,
            Some("- \"Test uno, probando probando, un dos tres.\"\n\nTest 2:".to_string()),
            Some("notes".to_string()),
        )
        .await
        .expect("manual note");

    let updated = repos
        .set_generated_note_for_session(
            &note.id,
            Some("session-2"),
            None,
            None,
            "Test 2:\n\n## Note\n\n- Test: \"probando, probando, tres, dos, uno\"".to_string(),
        )
        .await
        .expect("second generated block");

    assert_eq!(
        updated.edited_content.as_deref(),
        Some(
            "- \"Test uno, probando probando, un dos tres.\"\n\nTest 2:\n\n- Test: \"probando, probando, tres, dos, uno\""
        )
    );
}

#[tokio::test]
async fn generated_note_appends_to_existing_edited_content() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "First recording".to_string(),
        )
        .await
        .expect("first generated note");
    repos
        .update_note(
            &note.id,
            None,
            Some("User edited first recording".to_string()),
            Some("notes".to_string()),
        )
        .await
        .expect("edit note");

    let updated = repos
        .set_generated_note(&note.id, None, "Second recording".to_string())
        .await
        .expect("second generated note");

    assert_eq!(
        updated.edited_content.as_deref(),
        Some("User edited first recording\n\nSecond recording")
    );
    assert_eq!(
        updated.generated_content.as_deref(),
        Some("First recording\n\nSecond recording")
    );
}

#[tokio::test]
async fn generated_note_does_not_duplicate_full_regenerated_edited_content() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    repos
        .set_generated_note(
            &note.id,
            Some("Generated title".to_string()),
            "First recording".to_string(),
        )
        .await
        .expect("first generated note");
    repos
        .update_note(
            &note.id,
            None,
            Some("First recording".to_string()),
            Some("notes".to_string()),
        )
        .await
        .expect("edited note");

    let updated = repos
        .set_generated_note(
            &note.id,
            None,
            "First recording\n\nSecond recording".to_string(),
        )
        .await
        .expect("full regenerated note");

    assert_eq!(
        updated.edited_content.as_deref(),
        Some("First recording\n\nSecond recording")
    );
    assert_eq!(
        updated.generated_content.as_deref(),
        Some("First recording\n\nSecond recording")
    );
}

#[tokio::test]
async fn get_note_returns_transcript_and_audio_metadata() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let session_id = "session-1";
    repos
        .create_recording_session(
            &note.id,
            session_id,
            RecordingSourceMode::MicrophoneOnly,
            "/tmp/partial.wav",
            "/tmp/final.wav",
            None,
        )
        .await
        .expect("session");
    let audio = repos
        .create_audio_artifact(&note.id, session_id, "/tmp/final.wav", 1200, 2048, "abc")
        .await
        .expect("artifact");
    repos
        .create_transcript(
            &note.id,
            &audio.id,
            "Raw transcript text",
            Some("en".into()),
            "groq",
        )
        .await
        .expect("transcript");

    let loaded = repos.get_note(&note.id).await.expect("loaded note");

    assert_eq!(loaded.audio.expect("audio").id, audio.id);
    assert_eq!(
        loaded.transcript.expect("transcript").text,
        "Raw transcript text"
    );
    assert_eq!(loaded.source_transcripts.len(), 1);
    assert_eq!(loaded.source_transcripts[0].text, "Raw transcript text");
}

#[tokio::test]
async fn discarded_recording_no_longer_exposes_retryable_audio() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let session_id = "session-1";
    repos
        .create_recording_session(
            &note.id,
            session_id,
            RecordingSourceMode::MicrophoneOnly,
            "/tmp/partial.wav",
            "/tmp/final.wav",
            None,
        )
        .await
        .expect("session");
    repos
        .create_audio_artifact(&note.id, session_id, "/tmp/final.wav", 1200, 2048, "abc")
        .await
        .expect("artifact");

    let discarded = repos
        .mark_recording_discarded(session_id, &note.id)
        .await
        .expect("discarded note");

    assert_eq!(discarded.processing_status, ProcessingStatus::Failed);
    assert_eq!(discarded.last_error.as_deref(), Some("Recording discarded"));
    assert!(discarded.audio.is_none());
    assert!(discarded.audio_sources.is_empty());
}

#[tokio::test]
async fn get_note_preserves_legacy_transcripts_alongside_timed_turns() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let session_id = "session-1";
    repos
        .create_recording_session(
            &note.id,
            session_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "/tmp/microphone.partial.wav",
            "/tmp/microphone.wav",
            None,
        )
        .await
        .expect("session");
    let audio = repos
        .create_audio_artifact(
            &note.id,
            session_id,
            "/tmp/microphone.wav",
            1200,
            2048,
            "abc",
        )
        .await
        .expect("artifact");
    let legacy = repos
        .create_transcript(
            &note.id,
            &audio.id,
            "Legacy transcript without a recording session",
            Some("en".into()),
            "groq",
        )
        .await
        .expect("standalone transcript");
    let timed = repos
        .create_source_transcript(
            &note.id,
            session_id,
            &audio.id,
            RecordingSourceMode::MicrophonePlusSystem,
            "microphone",
            "Timed source transcript",
            Some("en".into()),
            "groq",
            Some(1_000),
            Some(2_000),
            Some(0),
        )
        .await
        .expect("source transcript");

    let loaded = repos.get_note(&note.id).await.expect("loaded note");

    assert_eq!(loaded.source_transcripts.len(), 2);
    assert!(loaded.source_transcripts.iter().any(|row| {
        row.id == legacy.id
            && row.text == legacy.text
            && row.recording_session_id.is_none()
            && row.start_ms.is_none()
            && row.end_ms.is_none()
            && row.turn_index.is_none()
    }));
    assert!(loaded.source_transcripts.iter().any(|row| {
        row.id == timed.id
            && row.text == timed.text
            && row.recording_session_id.as_deref() == Some(session_id)
            && row.source.as_deref() == Some("microphone")
            && row.start_ms == Some(1_000)
            && row.end_ms == Some(2_000)
            && row.turn_index == Some(0)
    }));
}

#[tokio::test]
async fn get_note_returns_failed_source_transcript_reason() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let session_id = "session-1";
    repos
        .create_recording_session(
            &note.id,
            session_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "/tmp/system.partial.wav",
            "/tmp/system.wav",
            None,
        )
        .await
        .expect("session");
    let audio = repos
        .create_audio_artifact(&note.id, session_id, "/tmp/system.wav", 1200, 2048, "abc")
        .await
        .expect("artifact");

    repos
        .create_failed_source_transcript(
            &note.id,
            session_id,
            &audio.id,
            RecordingSourceMode::MicrophonePlusSystem,
            "system",
            "groq",
            "System source was silent.",
            Some(1_000),
            Some(2_000),
            Some(0),
        )
        .await
        .expect("failed source transcript");

    let loaded = repos.get_note(&note.id).await.expect("loaded note");

    assert_eq!(loaded.source_transcripts.len(), 1);
    assert_eq!(
        loaded.source_transcripts[0].source.as_deref(),
        Some("system")
    );
    assert_eq!(loaded.source_transcripts[0].status, "failed");
    assert_eq!(
        loaded.source_transcripts[0].last_error.as_deref(),
        Some("System source was silent.")
    );
}

#[tokio::test]
async fn get_note_orders_source_transcripts_by_session_then_turn() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let first_audio_id = create_session_audio(&repos, &note.id, "session-1").await;
    let second_audio_id = create_session_audio(&repos, &note.id, "session-2").await;

    repos
        .create_source_transcript(
            &note.id,
            "session-1",
            &first_audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "microphone",
            "First session second turn",
            Some("en".into()),
            "groq",
            Some(2_000),
            Some(3_000),
            Some(1),
        )
        .await
        .expect("first session second turn");
    repos
        .create_source_transcript(
            &note.id,
            "session-2",
            &second_audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "system",
            "Second session first turn",
            Some("en".into()),
            "groq",
            Some(1_000),
            Some(2_000),
            Some(0),
        )
        .await
        .expect("second session first turn");
    repos
        .create_source_transcript(
            &note.id,
            "session-1",
            &first_audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "system",
            "First session first turn",
            Some("en".into()),
            "groq",
            Some(500),
            Some(1_500),
            Some(0),
        )
        .await
        .expect("first session first turn");

    let loaded = repos.get_note(&note.id).await.expect("loaded note");

    assert_eq!(
        loaded
            .source_transcripts
            .iter()
            .map(|transcript| transcript.text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "First session first turn",
            "First session second turn",
            "Second session first turn",
        ]
    );
}

#[tokio::test]
async fn successful_source_turns_for_session_exclude_failed_and_empty_rows() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let audio_id = create_session_audio(&repos, &note.id, "session-1").await;

    repos
        .create_source_transcript(
            &note.id,
            "session-1",
            &audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "microphone",
            "Valid microphone turn",
            Some("en".into()),
            "groq",
            Some(1_000),
            Some(2_000),
            Some(0),
        )
        .await
        .expect("valid turn");
    repos
        .create_source_transcript(
            &note.id,
            "session-1",
            &audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "system",
            "   ",
            Some("en".into()),
            "groq",
            Some(2_500),
            Some(3_000),
            Some(1),
        )
        .await
        .expect("empty turn");
    repos
        .create_failed_source_transcript(
            &note.id,
            "session-1",
            &audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "system",
            "groq",
            "No speech detected.",
            Some(3_500),
            Some(4_000),
            Some(2),
        )
        .await
        .expect("failed turn");

    let transcripts = repos
        .successful_source_turn_transcripts_for_session("session-1")
        .await
        .expect("successful turns");

    assert_eq!(transcripts.len(), 1);
    assert_eq!(transcripts[0].text, "Valid microphone turn");
    assert_eq!(transcripts[0].turn_index, Some(0));
}

#[tokio::test]
async fn source_turn_upsert_replaces_failed_retry_with_success() {
    let repos = repos().await;
    let note = repos.create_note("default", None).await.expect("note");
    let audio_id = create_session_audio(&repos, &note.id, "session-1").await;

    let failed = repos
        .upsert_failed_source_turn_transcript(
            &note.id,
            "session-1",
            &audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "system",
            "groq",
            "No speech detected.",
            1_000,
            2_000,
            0,
        )
        .await
        .expect("failed upsert");
    let succeeded = repos
        .upsert_successful_source_turn_transcript(
            &note.id,
            "session-1",
            &audio_id,
            RecordingSourceMode::MicrophonePlusSystem,
            "system",
            "Recovered system turn",
            Some("en".into()),
            "groq",
            1_000,
            2_000,
            0,
        )
        .await
        .expect("success upsert");

    let loaded = repos.get_note(&note.id).await.expect("loaded note");

    assert_eq!(succeeded.id, failed.id);
    assert_eq!(loaded.source_transcripts.len(), 1);
    assert_eq!(loaded.source_transcripts[0].status, "succeeded");
    assert_eq!(loaded.source_transcripts[0].text, "Recovered system turn");
    assert_eq!(loaded.source_transcripts[0].last_error, None);
}

async fn create_session_audio(repos: &Repositories, note_id: &str, session_id: &str) -> String {
    let final_path = format!("/tmp/{session_id}.wav");
    repos
        .create_recording_session(
            note_id,
            session_id,
            RecordingSourceMode::MicrophonePlusSystem,
            &format!("/tmp/{session_id}.partial.wav"),
            &final_path,
            None,
        )
        .await
        .expect("session");
    repos
        .create_audio_artifact(note_id, session_id, &final_path, 1200, 2048, session_id)
        .await
        .expect("artifact")
        .id
}
