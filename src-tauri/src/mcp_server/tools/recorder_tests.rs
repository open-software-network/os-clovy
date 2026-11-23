//! Real persistence and broker correlation tests. No audio capture is started.
use super::recorder::{self, RecorderTools};
use super::{ActivityAccess, ToolCtx, ToolGroup};
use crate::approval_broker::testing::TestBroker;
use crate::db::repositories::Repositories;
use crate::domain::types::RecordingSourceMode;
use crate::llm::cli::CliKind;
use serde_json::json;

#[tokio::test]
async fn artifact_inventory_retains_note_session_sources_and_duration_without_paths() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = crate::mcp_server::tests::data(broker.pool().clone(), ActivityAccess::Off);
    let repos = Repositories::new(data.notes.clone());
    let note = repos.create_note(&data.profile, None).await.unwrap();
    repos
        .create_recording_session(
            &note.id,
            "take-one",
            RecordingSourceMode::MicrophonePlusSystem,
            "/private/synthetic-mic.part",
            "/private/synthetic-mic.wav",
            None,
        )
        .await
        .unwrap();
    repos
        .create_pending_source_artifact(
            &note.id,
            "take-one",
            "microphone",
            "/private/synthetic-mic.part",
            "/private/synthetic-mic.wav",
        )
        .await
        .unwrap();
    repos
        .create_pending_source_artifact(
            &note.id,
            "take-one",
            "system",
            "/private/synthetic-system.part",
            "/private/synthetic-system.wav",
        )
        .await
        .unwrap();
    sqlx::query::query("UPDATE audio_artifacts SET duration_ms=1234,size_bytes=990,status='valid' WHERE recording_session_id='take-one'").execute(&data.notes).await.unwrap();
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let result = RecorderTools
        .call(
            &ctx,
            "list_recording_artifacts",
            &json!({"sessionId":"take-one"}),
        )
        .await
        .unwrap();
    assert_eq!(result["sessionId"], "take-one");
    assert_eq!(result["sourceMode"], "microphonePlusSystem");
    assert_eq!(result["noteId"], note.id);
    assert_eq!(result["artifacts"][0]["source"], "microphone");
    assert_eq!(result["artifacts"][1]["source"], "system");
    assert_eq!(result["artifacts"][0]["durationMs"], 1234);
    assert!(!result.to_string().contains("/private/"));
    sqlx::query::query("UPDATE notes SET profile='other-partition' WHERE id=?")
        .bind(&note.id)
        .execute(&data.notes)
        .await
        .unwrap();
    assert_eq!(
        RecorderTools
            .call(
                &ctx,
                "list_recording_artifacts",
                &json!({"sessionId":"take-one"})
            )
            .await
            .unwrap_err()
            .code,
        "recording_not_found"
    );
    assert!(broker.requests().is_empty());
}

#[tokio::test]
async fn recovery_rejects_invalid_action_or_other_note_before_approval() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = crate::mcp_server::tests::data(broker.pool().clone(), ActivityAccess::Off);
    let repos = Repositories::new(data.notes.clone());
    let note = repos.create_note(&data.profile, None).await.unwrap();
    repos
        .create_recording_session(
            &note.id,
            "take-recover",
            RecordingSourceMode::MicrophoneOnly,
            "fixture.part",
            "fixture.wav",
            None,
        )
        .await
        .unwrap();
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    assert_eq!(
        recorder::call(
            &ctx,
            None,
            "recover_recording",
            &json!({"sessionId":"take-recover","action":"erase_everything"})
        )
        .await
        .unwrap_err()
        .code,
        "agent_tool_arguments_invalid"
    );
    assert_eq!(
        recorder::call(
            &ctx,
            None,
            "retry_note_processing",
            &json!({"sessionId":"take-recover","noteId":"other-note"})
        )
        .await
        .unwrap_err()
        .code,
        "recording_target_mismatch"
    );
    assert!(broker.requests().is_empty());
}

#[tokio::test]
async fn status_serializes_the_exact_authorized_snapshot_after_database_wait() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = crate::mcp_server::tests::data(broker.pool().clone(), ActivityAccess::Off);
    let repos = Repositories::new(data.notes.clone());
    let note = repos.create_note(&data.profile, None).await.unwrap();
    repos
        .create_recording_session(
            &note.id,
            "snapshot-take",
            RecordingSourceMode::MicrophoneOnly,
            "fixture.part",
            "fixture.wav",
            None,
        )
        .await
        .unwrap();
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let status=serde_json::from_value(json!({"sessionId":"snapshot-take","noteId":note.id,"sourceMode":"microphoneOnly","state":"paused","elapsedMs":4123,"level":{"peak":0,"rms":0,"recentPeaks":[]},"silenceWarning":false,"bytesWritten":100})).unwrap();
    let result = recorder::recording_status_snapshot(&ctx, Some(status))
        .await
        .unwrap();
    assert_eq!(result["sessionId"], "snapshot-take");
    assert_eq!(result["elapsedMs"], 4123);
    assert_eq!(result["state"], "paused");
}
