//! Note generation through the user's own CLI providers (`crate::llm`),
//! exercised through `providers::notes::generate_note_from_transcript`. Kept in its
//! own file so the routing code in `providers/notes.rs` can change without touching
//! the test source.

use super::*;
#[cfg(unix)]
use crate::db::repositories::Repositories;
#[cfg(unix)]
use crate::llm::shell_env::LoginEnv;
use crate::llm::{cli::CliKind, registry::ProviderRef, shell_env::set_login_env_for_tests};
#[cfg(unix)]
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};

/// Restores the process-wide settings and login environment on drop.
struct Restore;

impl Drop for Restore {
    fn drop(&mut self) {
        crate::providers::replace_current_settings_for_tests(
            crate::providers::default_settings_for_tests(),
        );
        set_login_env_for_tests(None);
    }
}

#[tokio::test]
async fn recording_question_routes_only_to_the_selected_tool_free_notes_engine() {
    use crate::llm::{endpoint::test_server, registry::LlmEndpointRecord, GenerateRequest};

    let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
    let _restore = Restore;
    let endpoint = test_server::start(std::sync::Arc::new(|_| {
        (200, test_server::chat_reply("Qual é o prazo?"))
    }))
    .await;
    let mut settings = crate::providers::default_settings_for_tests();
    let endpoint_id = "recording-question-endpoint";
    let provider = ProviderRef::Endpoint {
        id: endpoint_id.into(),
    };
    settings.llm_endpoints.push(LlmEndpointRecord {
        id: endpoint_id.into(),
        name: "Notes endpoint".into(),
        base_url: endpoint.base_url.clone(),
        model_id: "notes-test-model".into(),
        has_api_key: false,
        structured_output: None,
        rpm_limit: None,
    });
    settings.llm_usage.chat = provider.clone();
    let request = GenerateRequest {
        system: Some("Return one grounded question.".into()),
        prompt: r#"{"transcript":[{"source":"microphone","text":"Ainda falta decidir o prazo."}]}"#
            .into(),
        ..Default::default()
    };

    for (selected, expected_error) in [
        (ProviderRef::None, "recording_suggestion_provider_missing"),
        (
            ProviderRef::Cli { id: CliKind::Codex },
            "recording_suggestion_tools_unavailable",
        ),
        (
            ProviderRef::Cli { id: CliKind::Agy },
            "recording_suggestion_tools_unavailable",
        ),
    ] {
        settings.llm_usage.notes = selected;
        crate::providers::replace_current_settings_for_tests(settings.clone());
        let error = generate_recording_question(request.clone())
            .await
            .unwrap_err();
        assert_eq!(error.code, expected_error);
        assert!(endpoint.requests.lock().unwrap().is_empty());
    }

    settings.llm_usage.notes = provider;
    settings.llm_usage.chat = ProviderRef::None;
    crate::providers::replace_current_settings_for_tests(settings);
    assert_eq!(
        generate_recording_question(request.clone()).await.unwrap(),
        "Qual é o prazo?"
    );
    let requests = endpoint.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let body = &requests[0];
    assert_eq!(body["model"], "notes-test-model");
    assert_eq!(body["messages"][0]["content"], request.system.unwrap());
    assert_eq!(body["messages"][1]["content"], request.prompt);
    assert!(body.get("tools").is_none());
    assert!(body.get("tool_choice").is_none());
}

#[tokio::test]
#[cfg(unix)]
async fn llm_note_generated_by_fake_cli_is_persisted_like_a_clovy_note() {
    let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
    let _restore = Restore;
    let bin = tempfile::tempdir().unwrap();
    let stdin_log = bin.path().join("stdin.txt");
    let claude = bin.path().join("claude");
    // A fake `claude -p` that records its prompt and answers with a note.
    std::fs::write(
        &claude,
        format!(
            "#!/bin/sh\ncat > '{}'\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"## Summary\\\\n- Ship the updater fix\"}}'\n",
            stdin_log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    set_login_env_for_tests(Some(LoginEnv::from_vars(BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.path().display()),
    )]))));
    let mut settings = crate::providers::default_settings_for_tests();
    settings.llm_usage.notes = ProviderRef::Cli {
        id: CliKind::Claude,
    };
    crate::providers::replace_current_settings_for_tests(settings);

    let generated = generate_note_from_transcript(GenerationRequest {
        video: None,
        provider: crate::providers::generation_provider(),
        operation_id: Some("note-1".to_string()),
        title: "Weekly sync".to_string(),
        existing_generated_note: None,
        transcript: "We will ship the updater fix this week.".to_string(),
        transcript_source_labels: false,
        manual_notes: None,
        language: Some("en".to_string()),
        recorded_on: None,
    })
    .await
    .unwrap();
    assert_eq!(generated.content, "## Summary\n- Ship the updater fix");
    assert_eq!(generated.provider, "cli:claude");
    let prompt = std::fs::read_to_string(&stdin_log).unwrap();
    assert!(prompt.starts_with(crate::llm::CLOVY_AUTHORSHIP_MARKER));
    assert!(prompt.contains("We will ship the updater fix this week."));

    // Persist exactly as the processing pipeline does and read it back.
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let repos = Repositories::new(pool);
    let note = repos.create_note("default", None).await.unwrap();
    repos
        .create_recording_session(
            &note.id,
            "session-1",
            crate::domain::types::RecordingSourceMode::MicrophoneOnly,
            "microphone.partial.wav",
            "microphone.wav",
            None,
        )
        .await
        .unwrap();
    let artifact = repos
        .create_audio_artifact(&note.id, "session-1", "microphone.wav", 1_000, 10, "sum")
        .await
        .unwrap();
    let transcript = repos
        .create_transcript(
            &note.id,
            &artifact.id,
            "We will ship the updater fix this week.",
            Some("en".to_string()),
            crate::providers::PROVIDER_GROQ,
        )
        .await
        .unwrap();
    let result_id = repos
        .create_generation_result(
            &note.id,
            &transcript.id,
            &generated.content,
            generated.title_suggestion.clone(),
            &generated.provider,
            &generated.prompt_version,
        )
        .await
        .unwrap();
    repos
        .set_generated_note_for_session(
            &note.id,
            None,
            Some(&result_id),
            generated.title_suggestion,
            generated.content,
        )
        .await
        .unwrap();
    let stored = repos.get_note(&note.id).await.unwrap();
    assert_eq!(
        stored.generated_content.as_deref(),
        Some("## Summary\n- Ship the updater fix")
    );
}

#[tokio::test]
#[cfg(unix)]
async fn note_transcript_correction_runs_on_the_notes_cli_not_clovy_api() {
    // ditado-rapido R2: dictation cleanup left the registry, so the note
    // transcript correction follows the engine that writes the note.
    let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
    let _restore = Restore;
    let bin = tempfile::tempdir().unwrap();
    let stdin_log = bin.path().join("stdin.txt");
    let claude = bin.path().join("claude");
    std::fs::write(
        &claude,
        format!(
            "#!/bin/sh\ncat > '{}'\necho '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"Ship the Clovy updater fix.\"}}'\n",
            stdin_log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
    set_login_env_for_tests(Some(LoginEnv::from_vars(BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.path().display()),
    )]))));
    let mut settings = crate::providers::default_settings_for_tests();
    settings.llm_usage.notes = ProviderRef::Cli {
        id: CliKind::Claude,
    };
    crate::providers::replace_current_settings_for_tests(settings);

    // Only the synthetic CLI is configured here, so an answer can only
    // come from the CLI.
    let corrected = cleanup_text(DictateCleanupRequestParams {
        text: "ship the clovie updater fix".to_string(),
        dictionary_context: Some("Clovy".to_string()),
        app_context: None,
        style: NOTE_TRANSCRIPT_CLEANUP_STYLE.to_string(),
        session_id: "note_transcript".to_string(),
        utterance_id: "turn-1".to_string(),
    })
    .await
    .unwrap();
    assert_eq!(corrected, "Ship the Clovy updater fix.");
    let prompt = std::fs::read_to_string(&stdin_log).unwrap();
    assert!(prompt.contains("ship the clovie updater fix"));
}

/// Generates a note with the real `claude` CLI found through the real
/// login shell (no fakes). Needs `claude` installed and signed in.
#[tokio::test]
#[ignore = "requires an installed, signed-in claude CLI"]
async fn live_claude_cli_generates_a_note() {
    let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
    let _restore = Restore;
    set_login_env_for_tests(None);
    let mut settings = crate::providers::default_settings_for_tests();
    settings.llm_usage.notes = ProviderRef::Cli {
        id: CliKind::Claude,
    };
    crate::providers::replace_current_settings_for_tests(settings);
    let generated = generate_note_from_transcript(GenerationRequest {
        video: None,
        provider: crate::providers::generation_provider(),
        operation_id: Some("live-claude-note".to_string()),
        title: "Weekly sync".to_string(),
        existing_generated_note: None,
        transcript: "Weekly sync. We agreed to ship the updater fix on Thursday. Ana will review the onboarding metrics next Monday.".to_string(),
        transcript_source_labels: false,
        manual_notes: None,
        language: Some("en".to_string()),
        recorded_on: None,
    })
    .await
    .expect("claude should generate the note");
    assert_eq!(generated.provider, "cli:claude");
    assert!(generated.content.to_lowercase().contains("updater"));
    eprintln!("live claude note:\n{}\n", generated.content);
}

#[tokio::test]
#[cfg(unix)]
async fn llm_video_note_uses_skill_and_falls_back_with_stable_reasons() {
    const CHILD: &str = "CLOVY_VIDEO_NOTE_TEST_CHILD";
    if std::env::var(CHILD).is_err() {
        let output = tokio::task::spawn_blocking(|| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "providers::notes::llm_note_tests::llm_video_note_uses_skill_and_falls_back_with_stable_reasons",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap()
        })
        .await
        .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _lock = crate::providers::GLOBAL_SETTINGS_TEST_LOCK.lock().await;
    let _restore = Restore;
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let skill = home.join(".agents/skills/video-analysis");
    std::fs::create_dir_all(skill.join("scripts")).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "VIDEO_SKILL_SENTINEL: inspect contact sheets",
    )
    .unwrap();
    std::fs::write(skill.join("scripts/video_evidence.py"), "# test script").unwrap();
    let video = home.join("window.mp4");
    std::fs::write(&video, "fake video").unwrap();
    for name in ["ffmpeg", "ffprobe"] {
        let path = home.join(name);
        std::fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let log = home.join("calls");
    let cli = home.join("claude");
    let read_receipts = r#"import json, pathlib, subprocess, sys
root, mode = pathlib.Path(sys.argv[1]), sys.argv[2]
if mode == "noreads":
    sys.exit(0)
if mode == "forged":
    (root / "video_read_receipts.jsonl").write_text('{"token":"forged","kind":"image"}')
    sys.exit(0)
summary_path = root / "summary.json"
summary_text = summary_path.read_text()
summary = json.loads(summary_text)
def post(path, response):
    event = {"hook_event_name": "PostToolUseFailure" if mode == "failedreads" else "PostToolUse",
             "tool_name": "Read", "tool_input": {"file_path": str(path)}, "tool_response": response}
    subprocess.run(["/usr/bin/python3", "video_guard.py", "post"], input=json.dumps(event), text=True,
                   stdout=subprocess.DEVNULL, check=True)
post(summary_path, {"type":"text", "file":{"filePath":str(summary_path), "content":summary_text,
     "startLine":1, "numLines":1, "totalLines":1}})
for path in summary["contact_sheets"] + summary["frames"][:1]:
    post(path, {"type":"text" if mode == "textreads" else "image",
         "file":{"base64":"aW1hZ2U=", "type":"image/png", "originalSize":5}})
"#;
    std::fs::write(
        &cli,
        format!(
            r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  if [ "$(cat '{}/mode')" = old ]; then echo '2.1.294 (Claude Code)'; else echo '2.1.312 (Claude Code)'; fi
  exit 0
fi
case " $* " in *" --init-only "*)
  if [ "$(cat '{}/mode')" != hooks_off ]; then printf ready > video_hook_ready; fi
  exit 0;;
esac
cat >> '{}'
printf '%s\n' "$@" >> '{}'
printf 'PRIVATE_SCRATCH=%s\n' "$PWD" >> '{}'
case " $* " in
  *" Bash,Read "*)
    case "$(cat '{}/mode')" in fail) exit 1;; timeout) sleep 10;; empty) echo '{{"type":"result","is_error":false,"result":"Unsupported visual claim"}}'; exit 0;; esac
    while [ "$#" -gt 0 ]; do
      if [ "$1" = "--add-dir" ]; then shift; evidence="$1"; break; fi
      shift
    done
    printf 'frame' > "$evidence/frame.jpg"
    printf 'contact' > "$evidence/contact.jpg"
    printf '{{"frames":["%s/frame.jpg"],"contact_sheets":["%s/contact.jpg"]}}' "$evidence" "$evidence" > "$evidence/summary.json"
    /usr/bin/python3 - "$evidence" "$(cat '{}/mode')" <<'PYREAD'
{read_receipts}
PYREAD
    echo '{{"type":"result","is_error":false,"result":"Visual note at 00:10"}}';;
  *) echo '{{"type":"result","is_error":false,"result":"Transcript note"}}';;
esac
"#,
            home.display(),
            home.display(),
            log.display(),
            log.display(),
            log.display(),
            home.display(),
            home.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    set_login_env_for_tests(Some(LoginEnv::from_vars(BTreeMap::from([
        (
            "PATH".to_string(),
            format!("{}:/usr/bin:/bin", home.display()),
        ),
        ("HOME".to_string(), home.display().to_string()),
    ]))));
    let mut settings = crate::providers::default_settings_for_tests();
    settings.llm_usage.notes = ProviderRef::Cli {
        id: CliKind::Claude,
    };
    crate::providers::replace_current_settings_for_tests(settings);
    let request = || GenerationRequest {
        provider: "openrouter".into(),
        operation_id: None,
        title: "Review".into(),
        existing_generated_note: None,
        transcript: "We approved the slide.".into(),
        transcript_source_labels: false,
        manual_notes: None,
        language: None,
        recorded_on: None,
        video: Some(NoteVideoInput {
            path: video.display().to_string(),
            offset_ms: 1234,
            app: "Browser".into(),
            title: "Meet".into(),
            failure_reason: None,
        }),
    };
    for (mode, reason, text) in [
        ("ok", "analyzed", "Visual note at 00:10"),
        ("noreads", "analysis_failed", "Transcript note"),
        ("textreads", "analysis_failed", "Transcript note"),
        ("failedreads", "analysis_failed", "Transcript note"),
        ("forged", "analysis_failed", "Transcript note"),
        ("fail", "analysis_failed", "Transcript note"),
        ("empty", "analysis_failed", "Transcript note"),
        ("timeout", "analysis_timeout", "Transcript note"),
        ("old", "cli_unsupported", "Transcript note"),
        ("hooks_off", "cli_unsupported", "Transcript note"),
    ] {
        std::fs::write(home.join("mode"), mode).unwrap();
        let generated = generate_note_from_transcript_with_video_timeout(
            request(),
            if mode == "timeout" {
                std::time::Duration::from_millis(300)
            } else {
                std::time::Duration::from_secs(5)
            },
        )
        .await
        .unwrap();
        assert_eq!(generated.video_analysis.as_deref(), Some(reason));
        assert_eq!(generated.content, text);
        if reason == "analyzed" {
            assert_eq!(generated.prompt_version, "notes-video-v1");
        }
    }
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(calls.contains("VIDEO_SKILL_SENTINEL"));
    assert!(calls.contains("1234"));
    assert!(calls.contains("--no-transcript"));
    // All evidence directories disappear after success, failure, and timeout.
    let lines: Vec<_> = calls.lines().collect();
    for line in &lines {
        if let Some(path) = line.strip_prefix("PRIVATE_SCRATCH=") {
            assert!(
                !std::path::Path::new(path).exists(),
                "private guard/key/receipts must be removed"
            );
        }
    }
    for pair in lines.windows(2) {
        if pair[0] == "--add-dir" {
            assert!(!std::path::Path::new(pair[1]).exists());
        }
    }
    std::fs::remove_file(home.join("ffmpeg")).unwrap();
    let generated = generate_note_from_transcript(request()).await.unwrap();
    assert_eq!(generated.video_analysis.as_deref(), Some("tools_missing"));
    let mut denied = request();
    denied.video.as_mut().unwrap().failure_reason = Some("permission_denied".into());
    let generated = generate_note_from_transcript(denied).await.unwrap();
    assert_eq!(
        generated.video_analysis.as_deref(),
        Some("permission_denied")
    );
    let mut failed_capture = request();
    failed_capture.video.as_mut().unwrap().failure_reason = Some("capture_failed".into());
    let generated = generate_note_from_transcript(failed_capture).await.unwrap();
    assert_eq!(generated.video_analysis.as_deref(), Some("capture_failed"));
    assert_eq!(generated.content, "Transcript note");
    let pi = home.join("pi");
    std::fs::write(
        &pi,
        r#"#!/bin/sh
cat >/dev/null
echo '{"type":"message_end","message":{"role":"assistant","content":"Transcript note"}}'
echo '{"type":"agent_end"}'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut settings = crate::providers::default_settings_for_tests();
    settings.llm_usage.notes = ProviderRef::Cli { id: CliKind::Pi };
    crate::providers::replace_current_settings_for_tests(settings);
    let generated = generate_note_from_transcript(request()).await.unwrap();
    assert_eq!(
        generated.video_analysis.as_deref(),
        Some("engine_unsupported")
    );
    assert_eq!(generated.content, "Transcript note");
    let mut settings = crate::providers::default_settings_for_tests();
    settings.llm_usage.notes = ProviderRef::Cli {
        id: CliKind::Claude,
    };
    crate::providers::replace_current_settings_for_tests(settings);
    std::fs::remove_file(skill.join("SKILL.md")).unwrap();
    let generated = generate_note_from_transcript(request()).await.unwrap();
    assert_eq!(generated.video_analysis.as_deref(), Some("skill_missing"));
    assert_eq!(generated.content, "Transcript note");
    std::fs::remove_file(&video).unwrap();
    let generated = generate_note_from_transcript(request()).await.unwrap();
    assert_eq!(generated.video_analysis.as_deref(), Some("video_missing"));
}

#[test]
#[cfg(unix)]
fn llm_video_evidence_must_exist_inside_the_current_run() {
    let root = tempfile::tempdir().unwrap();
    assert!(!video_evidence_exists(root.path()));
    let frame = root.path().join("frame.jpg");
    let contact = root.path().join("contact.jpg");
    std::fs::write(&frame, b"frame").unwrap();
    std::fs::write(&contact, b"contact").unwrap();
    let summary = serde_json::json!({ "frames": [frame], "contact_sheets": [contact] });
    std::fs::write(root.path().join("summary.json"), summary.to_string()).unwrap();
    assert!(video_evidence_exists(root.path()));
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::remove_file(&contact).unwrap();
    std::os::unix::fs::symlink(outside.path(), &contact).unwrap();
    assert!(!video_evidence_exists(root.path()));
}
