use clovy_lib::domain::{
    processing::manual_notes_for_generation,
    types::{NoteDto, ProcessingStatus},
};

const NOW: &str = "2026-05-21T10:00:00Z";

fn note(overrides: impl FnOnce(&mut NoteDto)) -> NoteDto {
    let mut note = NoteDto {
        revision: 1,
        id: "note-1".to_string(),
        title: "Note".to_string(),
        preview: String::new(),
        processing_status: ProcessingStatus::Ready,
        folder_ids: Vec::new(),
        created_at: NOW.to_string(),
        updated_at: NOW.to_string(),
        duration_ms: None,
        calendar_event: None,
        generated_content: None,
        edited_content: None,
        transcript: None,
        transcript_coverage: None,
        video_analysis: None,
        source_transcripts: Vec::new(),
        recording: None,
        audio: None,
        audio_sources: Vec::new(),
        active_tab: Some("notes".to_string()),
        last_error: None,
        queued_recordings: 0,
        retry_recording_session_id: None,
    };
    overrides(&mut note);
    note
}

#[test]
fn manual_notes_for_generation_uses_only_new_text_after_generated_note() {
    let note = note(|note| {
        note.generated_content = Some("First generated note".to_string());
        note.edited_content =
            Some("First generated note\n\nManual note for the next recording".to_string());
    });

    assert_eq!(
        manual_notes_for_generation(&note).as_deref(),
        Some("Manual note for the next recording")
    );
}

#[test]
fn manual_notes_for_generation_ignores_existing_edited_note_body() {
    let note = note(|note| {
        note.generated_content = Some("First generated note".to_string());
        note.edited_content = Some("Edited version of the first generated note".to_string());
    });

    assert_eq!(manual_notes_for_generation(&note), None);
}

#[test]
fn manual_notes_for_generation_uses_new_tail_after_manual_preface() {
    let note = note(|note| {
        note.generated_content = Some("- Generated transcript note".to_string());
        note.edited_content = Some("Test 1:\n\n- Generated transcript note\n\nTest 2".to_string());
    });

    assert_eq!(
        manual_notes_for_generation(&note).as_deref(),
        Some("Test 2")
    );
}

#[test]
fn manual_notes_for_generation_uses_preface_when_generated_note_was_appended_below_it() {
    let note = note(|note| {
        note.generated_content = Some("Generated note body".to_string());
        note.edited_content =
            Some("Manual notes from recording\n\nGenerated note body".to_string());
    });

    assert_eq!(
        manual_notes_for_generation(&note).as_deref(),
        Some("Manual notes from recording")
    );
}

#[tokio::test]
async fn generation_rejects_empty_transcript() {
    let err = clovy_lib::providers::notes::generate_note_from_transcript(
        clovy_lib::providers::notes::GenerationRequest {
            provider: "openrouter".to_string(),
            operation_id: None,
            title: "Empty".to_string(),
            existing_generated_note: None,
            transcript: "   ".to_string(),
            transcript_source_labels: false,
            manual_notes: None,
            language: None,
            recorded_on: None,
            video: None,
        },
    )
    .await
    .expect_err("empty transcript should fail");

    assert_eq!(err.code, "transcription_empty");
}
