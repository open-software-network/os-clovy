use super::{
    finish_recording_session_with_timing,
    note_transcription_benchmark::{
        benchmark_repositories, configure_fake_note_provider_keys, spawn_fake_note_providers,
        BenchmarkClock, RequestEvents,
    },
};
use crate::{
    audio::capture::{FinishedRecording, FinishedSource},
    domain::{
        processing::{NoteProcessingProgressReporter, ProcessingTiming},
        types::{
            AudioLevelDto, ProcessingStatus, RecordingSessionDto, RecordingSource,
            RecordingSourceMode, RecordingState,
        },
    },
};
use sqlx::row::Row;
use std::{
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const TIMING_TEST_CHILD_ENV: &str = "CLOVY_NOTE_TRANSCRIPTION_TIMING_TEST_CHILD";
const TIMING_TEST_NAME: &str = "commands::note_transcription_timing_tests::done_origin_checkpoints_are_monotonic_and_single_shot";
const TIMING_TEST_COMPLETED_SENTINEL: &str = "CLOVY_NOTE_TRANSCRIPTION_TIMING_TEST_COMPLETED";

fn assert_timing_test_child_succeeded(success: bool, stdout: &[u8], stderr: &[u8]) {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    assert!(
        success,
        "isolated timing test failed: stdout={stdout} stderr={stderr}"
    );
    assert!(
        stdout.contains(TIMING_TEST_COMPLETED_SENTINEL),
        "isolated timing test did not report completion: stdout={stdout} stderr={stderr}"
    );
}

fn write_one_second_timing_wav(path: &Path) {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).expect("timing WAV");
    for frame in 0..48_000_u32 {
        let phase = frame as f32 * 311.0 * std::f32::consts::TAU / 48_000.0;
        let sample = (phase.sin() * 8_000.0) as i16;
        writer.write_sample(sample).expect("left timing sample");
        writer.write_sample(sample).expect("right timing sample");
    }
    writer.finalize().expect("finalize timing WAV");
}

fn timing_finished_recording(
    note_id: &str,
    recording_session_id: &str,
    path: PathBuf,
) -> FinishedRecording {
    FinishedRecording {
        session_id: recording_session_id.to_string(),
        note_id: note_id.to_string(),
        source_mode: RecordingSourceMode::MicrophoneOnly,
        final_path: path.clone(),
        sources: vec![FinishedSource {
            source: RecordingSource::Microphone,
            final_path: path,
            elapsed_ms: 1_000,
            dropped_samples: 0,
            capture_issue: None,
            failure: None,
        }],
        elapsed_ms: 1_000,
        recording: RecordingSessionDto {
            id: recording_session_id.to_string(),
            note_id: note_id.to_string(),
            source_mode: RecordingSourceMode::MicrophoneOnly,
            state: RecordingState::Ready,
            started_at: "2026-07-15T00:00:00.000Z".to_string(),
            elapsed_ms: 1_000,
            device_label: Some("Timing fixture".to_string()),
            level: AudioLevelDto::default(),
            live_preview_enabled: false,
            sources: Vec::new(),
            warnings: Vec::new(),
        },
    }
}

async fn assert_done_origin_checkpoints_are_monotonic_and_single_shot() {
    configure_fake_note_provider_keys();
    let dir = tempfile::tempdir().expect("timing tempdir");
    let repos = benchmark_repositories(&dir).await;
    let note = repos
        .create_note("default", None)
        .await
        .expect("timing note");
    let recording_session_id = format!("timing-{}", uuid::Uuid::new_v4());
    let audio_path = dir.path().join("timing-microphone.wav");
    write_one_second_timing_wav(&audio_path);
    let partial_path = audio_path.with_extension("partial.wav");
    repos
        .create_recording_session(
            &note.id,
            &recording_session_id,
            RecordingSourceMode::MicrophoneOnly,
            &partial_path.to_string_lossy(),
            &audio_path.to_string_lossy(),
            Some("Timing fixture".to_string()),
        )
        .await
        .expect("timing recording session");
    repos
        .create_pending_source_artifact(
            &note.id,
            &recording_session_id,
            RecordingSource::Microphone.as_db(),
            &partial_path.to_string_lossy(),
            &audio_path.to_string_lossy(),
        )
        .await
        .expect("timing microphone artifact");

    let timing = ProcessingTiming::from_done(Instant::now());
    let response = finish_recording_session_with_timing(
        &repos,
        crate::app_paths::AppPaths::from_data_dir(dir.path().join("video-data"))
            .expect("owned timing paths"),
        timing_finished_recording(&note.id, &recording_session_id, audio_path),
        Instant::now(),
        timing,
        NoteProcessingProgressReporter::default(),
    )
    .await
    .expect("finish timing recording");
    assert!(response.processing_started);

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let status: String =
            sqlx::query_scalar::query_scalar("SELECT processing_status FROM notes WHERE id = ?")
                .bind(&note.id)
                .fetch_one(&repos.pool)
                .await
                .expect("timing note status");
        let processing_complete_count: i64 = sqlx::query_scalar::query_scalar(
            "SELECT COUNT(*)
             FROM recording_checkpoints
             WHERE recording_session_id = ? AND kind = 'processing_complete'",
        )
        .bind(&recording_session_id)
        .fetch_one(&repos.pool)
        .await
        .expect("processing-complete count");
        assert_ne!(
            status,
            ProcessingStatus::Failed.as_db(),
            "timing processing reached Failed",
        );
        if status == ProcessingStatus::Ready.as_db() && processing_complete_count >= 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timing processing did not finish"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let rows = sqlx::query::query(
        "SELECT kind, details
         FROM recording_checkpoints
         WHERE recording_session_id = ?
           AND kind IN (
             'audio_validation',
             'processing_dequeued',
             'first_note_transcription_request',
             'first_transcript_persisted',
             'note_transcription_complete',
             'note_generation',
             'processing_complete'
           )
         ORDER BY rowid ASC",
    )
    .bind(&recording_session_id)
    .fetch_all(&repos.pool)
    .await
    .expect("timing checkpoints");

    for first_event_kind in [
        "first_note_transcription_request",
        "first_transcript_persisted",
    ] {
        assert_eq!(
            rows.iter()
                .filter(|row| row.get::<String, _>("kind") == first_event_kind)
                .count(),
            1,
            "checkpoint count for {first_event_kind}",
        );
    }

    let ordered_kinds = [
        "audio_validation",
        "processing_dequeued",
        "first_note_transcription_request",
        "first_transcript_persisted",
        "note_transcription_complete",
        "note_generation",
        "processing_complete",
    ];
    let durations = ordered_kinds
        .iter()
        .map(|expected_kind| {
            let matching = rows
                .iter()
                .filter(|row| row.get::<String, _>("kind") == *expected_kind)
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "checkpoint count for {expected_kind}");
            let details = matching[0]
                .get::<Option<String>, _>("details")
                .expect("timing checkpoint details");
            let details: serde_json::Value =
                serde_json::from_str(&details).expect("timing checkpoint JSON");
            details["doneToDurationMs"]
                .as_i64()
                .unwrap_or_else(|| panic!("missing Done duration for {expected_kind}"))
        })
        .collect::<Vec<_>>();
    assert!(
        durations.windows(2).all(|pair| pair[0] <= pair[1]),
        "Done-relative checkpoints must be monotonic: {durations:?}",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn done_origin_checkpoints_are_monotonic_and_single_shot() {
    if std::env::var_os(TIMING_TEST_CHILD_ENV).is_some() {
        assert_done_origin_checkpoints_are_monotonic_and_single_shot().await;
        println!("{TIMING_TEST_COMPLETED_SENTINEL}");
        return;
    }

    let clock = BenchmarkClock::default();
    clock.start();
    let events = RequestEvents::new(clock);
    let (address, api_handle) = spawn_fake_note_providers(events).await;
    let executable = std::env::current_exe().expect("timing test executable");
    let output = tokio::task::spawn_blocking(move || {
        Command::new(executable)
            .args([
                "--exact",
                TIMING_TEST_NAME,
                "--nocapture",
                "--test-threads=1",
            ])
            .env(TIMING_TEST_CHILD_ENV, "1")
            .env(
                "CLOVY_TEST_NOTE_PROVIDER_ORIGIN",
                format!("http://{address}"),
            )
            .output()
            .expect("run isolated timing test child")
    })
    .await
    .expect("join isolated timing test child");

    api_handle.abort();
    let _ = api_handle.await;
    assert_timing_test_child_succeeded(output.status.success(), &output.stdout, &output.stderr);
}

#[test]
#[should_panic(expected = "isolated timing test did not report completion")]
fn isolated_timing_test_rejects_a_zero_test_child() {
    assert_timing_test_child_succeeded(true, b"running 0 tests", b"");
}
