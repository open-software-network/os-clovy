//! Integration tests of the CLI chat engine with fake CLIs (shell scripts in
//! a temporary directory that reproduce each CLI's measured stream format),
//! an in-memory agent database, and the shared persistence path.

use super::cli::{self, EngineEvent, StreamTranslator};
use super::turn::{execute_turn, TurnContext};
use super::*;
use crate::agent_sessions::{AgentItemPayload, AgentRepository, AgentSafetyMode, MessagePayload};
use crate::mcp_server::McpLaunch;
use serde_json::{json, Value};
use sqlx_sqlite::SqlitePoolOptions;
use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex as StdMutex;

/// Persists like the app does and records what would be emitted.
pub(super) struct TestSink {
    pub repository: AgentRepository,
    pub emitted: StdMutex<Vec<(String, Value)>>,
}

impl FrameSink for TestSink {
    async fn publish(&self, frame: RpcFrame) {
        let persisted =
            crate::agent_sessions::events::persist_runtime_event(&self.repository, &frame)
                .await
                .expect("event persists");
        if let Some(event) = persisted {
            self.emitted
                .lock()
                .unwrap()
                .push((event.method, event.data));
        }
    }
}

pub(super) struct Harness {
    pub repository: AgentRepository,
    pub session_id: String,
    pub workspace: tempfile::TempDir,
    pub bin: tempfile::TempDir,
    pub log: tempfile::TempDir,
    pub app_data: tempfile::TempDir,
}

impl Harness {
    pub async fn new(kind: CliKind) -> Self {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory database");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repository = AgentRepository::new(pool);
        let workspace = tempfile::tempdir().unwrap();
        let session = repository
            .create_session(
                "CLI chat",
                &engine_model_id(kind),
                AgentSafetyMode::Sandboxed,
                workspace.path().to_str(),
            )
            .await
            .unwrap();
        Self {
            repository,
            session_id: session.id,
            workspace,
            bin: tempfile::tempdir().unwrap(),
            log: tempfile::tempdir().unwrap(),
            app_data: {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    tempfile::Builder::new()
                        .permissions(std::fs::Permissions::from_mode(0o700))
                        .tempdir()
                        .unwrap()
                }
                #[cfg(windows)]
                {
                    tempfile::tempdir().unwrap()
                }
            },
        }
    }

    /// Writes an executable fake CLI. The prelude numbers each call and logs
    /// its argv (one per line) and stdin to `<log>/<n>.args` / `<n>.stdin`.
    #[cfg(unix)]
    pub fn fake_cli(&self, kind: CliKind, body: &str) -> PathBuf {
        let path = self.bin.path().join(kind.id());
        let script = format!(
            "#!/bin/sh\nLOG='{log}'\nn=$(( $(cat \"$LOG/count\" 2>/dev/null || echo 0) + 1 ))\necho $n > \"$LOG/count\"\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done > \"$LOG/$n.args\"\ncat > \"$LOG/$n.stdin\"\n{body}\n",
            log = self.log.path().display()
        );
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    pub fn args(&self, call: usize) -> Vec<String> {
        std::fs::read_to_string(self.log.path().join(format!("{call}.args")))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    pub fn stdin(&self, call: usize) -> String {
        std::fs::read_to_string(self.log.path().join(format!("{call}.stdin"))).unwrap_or_default()
    }

    pub fn context(
        &self,
        kind: CliKind,
        program: &Path,
        run_id: &str,
        input: &str,
        mcp: Option<McpLaunch>,
    ) -> TurnContext {
        TurnContext {
            session_id: self.session_id.clone(),
            run_id: run_id.to_string(),
            data_partition: "default".into(),
            previous_run_id: None,
            kind,
            program: program.to_path_buf(),
            env: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            agy: if kind == CliKind::Agy {
                Some(
                    crate::llm::agy::HostContext::new(
                        self.app_data.path().canonicalize().unwrap(),
                        &BTreeMap::from([
                            (
                                "HOME".into(),
                                self.app_data.path().join("host-home").display().to_string(),
                            ),
                            (
                                "APPDATA".into(),
                                self.app_data
                                    .path()
                                    .join("host-appdata")
                                    .display()
                                    .to_string(),
                            ),
                        ]),
                    )
                    .unwrap(),
                )
            } else {
                None
            },
            workspace: self.workspace.path().to_path_buf(),
            input: input.to_string(),
            mcp,
            turns: None,
            safety_mode: AgentSafetyMode::Sandboxed,
            attended: true,
            surface: "chat".into(),
            routine_tools: None,
            images: Vec::new(),
        }
    }

    /// What `start_agent_run` does before the turn: the run and the user
    /// message.
    pub async fn new_run(&self, kind: CliKind, input: &str) -> String {
        let run = self
            .repository
            .create_run(&self.session_id, &engine_model_id(kind), None)
            .await
            .unwrap();
        self.repository
            .append_item(
                &self.session_id,
                Some(&run.id),
                0,
                &AgentItemPayload::UserMessage(MessagePayload {
                    role: "user".into(),
                    content: input.to_string(),
                    attachments: Vec::new(),
                }),
                Some(&format!("user:{}", run.id)),
            )
            .await
            .unwrap();
        run.id
    }

    /// Runs one complete turn and returns its run id and emitted events.
    pub async fn turn(
        &self,
        kind: CliKind,
        program: &Path,
        input: &str,
        mcp: Option<McpLaunch>,
    ) -> (String, Vec<(String, Value)>) {
        let run_id = self.new_run(kind, input).await;
        let sink = TestSink {
            repository: self.repository.clone(),
            emitted: StdMutex::new(Vec::new()),
        };
        let (_cancel, receiver) = watch::channel(false);
        execute_turn(
            &self.repository,
            &sink,
            self.context(kind, program, &run_id, input, mcp),
            receiver,
        )
        .await;
        let emitted = sink.emitted.into_inner().unwrap();
        (run_id, emitted)
    }

    /// The session transcript as (kind, text) pairs, in display order.
    pub async fn transcript(&self) -> Vec<(String, String)> {
        self.repository
            .items(&self.session_id)
            .await
            .unwrap()
            .into_iter()
            .map(|item| {
                let kind = item.payload.kind().to_string();
                let text = match item.payload {
                    AgentItemPayload::UserMessage(message)
                    | AgentItemPayload::AssistantMessage(message) => message.content,
                    AgentItemPayload::Reasoning(text) => text.text,
                    AgentItemPayload::ToolCall(tool) | AgentItemPayload::ToolResult(tool) => {
                        format!(
                            "{}:{}",
                            tool.tool_name.unwrap_or_default(),
                            tool.status.unwrap_or_default()
                        )
                    }
                    AgentItemPayload::Error(value) => {
                        value["message"].as_str().unwrap_or_default().to_string()
                    }
                    other => format!("{other:?}"),
                };
                (kind, text)
            })
            .collect()
    }
}

/// A fake claude that requires `--resume claude-conv-1` after its first call
/// and answers "Answer <n>" with reasoning and one tool call.
#[cfg(unix)]
pub(super) const FAKE_CLAUDE: &str = r#"
if [ "$n" -gt 1 ] && ! grep -qx -- 'claude-conv-1' "$LOG/$n.args"; then echo 'No conversation found to resume' >&2; exit 3; fi
echo '{"type":"system","subtype":"init","session_id":"claude-conv-1"}'
echo '{"type":"stream_event","event":{"type":"message_start"}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"thinking it over"}}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Answer "}}}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"'"$n"'"}}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"Answer '"$n"'"}]}}'
echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tool-'"$n"'","name":"Bash","input":{"command":"ls"}}]}}'
echo '{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool-'"$n"'","content":"notes.txt","is_error":false}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Answer '"$n"'","session_id":"claude-conv-1"}'
"#;

fn argument_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

/// Fake stream for each CLI: answers "Answer <n>" and enforces the resume
/// argument from the second call on; `check` is a shell test on the args.
#[cfg(unix)]
fn fake_two_turn_script(kind: CliKind) -> (&'static str, String) {
    match kind {
        CliKind::Claude => ("claude-conv-1", FAKE_CLAUDE.to_string()),
        CliKind::Codex => (
            "codex-thread-1",
            r#"
if [ "$n" -gt 1 ]; then grep -qx resume "$LOG/$n.args" && grep -qx codex-thread-1 "$LOG/$n.args" || exit 3; fi
echo '{"type":"thread.started","thread_id":"codex-thread-1"}'
echo '{"type":"turn.started"}'
echo '{"type":"item.completed","item":{"id":"item_0","type":"reasoning","text":"thinking it over"}}'
echo '{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"ls","aggregated_output":"","exit_code":null,"status":"in_progress"}}'
echo '{"type":"item.completed","item":{"id":"item_1","type":"command_execution","command":"ls","aggregated_output":"notes.txt","exit_code":0,"status":"completed"}}'
echo '{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"Answer '"$n"'"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}'
"#
            .to_string(),
        ),
        CliKind::Pi => (
            "",
            r#"
id=$(sed -n '/^--session-id$/{n;p;}' "$LOG/$n.args")
[ -n "$id" ] || exit 3
if [ "$n" -gt 1 ]; then [ "$id" = "$(cat "$LOG/pi.id")" ] || exit 4; fi
echo "$id" > "$LOG/pi.id"
echo '{"type":"session","version":3,"id":"'"$id"'"}'
echo '{"type":"agent_start"}'
echo '{"type":"message_start","message":{"role":"assistant","content":[]}}'
echo '{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","contentIndex":0,"delta":"thinking it over"}}'
echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Answer '"$n"'"}}'
echo '{"type":"message_end","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"text","text":"Answer '"$n"'"}]}}'
echo '{"type":"tool_execution_start","toolCallId":"call-'"$n"'","toolName":"bash","args":{"command":"ls"}}'
echo '{"type":"tool_execution_end","toolCallId":"call-'"$n"'","toolName":"bash","result":{"content":[{"type":"text","text":"notes.txt"}]},"isError":false}'
echo '{"type":"message_start","message":{"role":"assistant","content":[]}}'
echo '{"type":"message_end","message":{"role":"assistant","stopReason":"stop","content":[]}}'
echo '{"type":"agent_end","messages":[],"willRetry":false}'
echo '{"type":"agent_settled"}'
"#
            .to_string(),
        ),
        CliKind::Agy => (
            "agy-conv-1",
            r#"
if [ "$n" -gt 1 ] && ! grep -qx agy-conv-1 "$LOG/$n.args"; then exit 3; fi
echo '{"event":"init","conversation_id":"agy-conv-1","init":{"cwd":"."}}'
echo '{"event":"step_update","step_update":{"conversation_id":"agy-conv-1","step_index":0,"state":"DONE","step_type":"user_input"}}'
echo '{"event":"step_update","step_update":{"step_index":1,"state":"ACTIVE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls"}}}}'
echo '{"event":"step_update","step_update":{"step_index":1,"state":"DONE","step_type":"tool","tool_name":"run_command","tool_info":{"name":"run_command","parameters":{"CommandLine":"ls"},"result":"notes.txt"}}}'
echo '{"event":"step_update","step_update":{"step_index":2,"state":"ACTIVE","step_type":"agent_response","thinking_delta":"thinking it over","text_delta":"Answer '"$n"'"}}'
echo '{"event":"result","result":{"conversation_id":"agy-conv-1","status":"SUCCESS","response":"Answer '"$n"'"}}'
"#
            .to_string(),
        ),
        CliKind::CursorAgent => (
            "cursor-chat-1",
            r#"
if [ "$n" -gt 1 ] && ! grep -qx cursor-chat-1 "$LOG/$n.args"; then exit 3; fi
echo '{"type":"system","subtype":"init","session_id":"cursor-chat-1"}'
echo '{"type":"thinking","subtype":"delta","text":"thinking it over","timestamp_ms":1}'
echo '{"type":"tool_call","subtype":"started","call_id":"call-'"$n"'","tool_call":{"shellToolCall":{"args":{"command":"ls"}}}}'
echo '{"type":"tool_call","subtype":"completed","call_id":"call-'"$n"'","tool_call":{"shellToolCall":{"result":{"success":{"stdout":"notes.txt"}}}}}'
echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Answer "}]},"timestamp_ms":2}'
echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"'"$n"'"}]},"timestamp_ms":3}'
echo '{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Answer '"$n"'"}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Answer '"$n"'","session_id":"cursor-chat-1"}'
"#
            .to_string(),
        ),
        CliKind::Copilot => (
            "",
            r#"
id=$(sed -n '/^--session-id$/{n;p;}' "$LOG/$n.args")
[ -n "$id" ] || exit 3
if [ "$n" -gt 1 ]; then [ "$id" = "$(cat "$LOG/copilot.id")" ] || exit 4; fi
echo "$id" > "$LOG/copilot.id"
echo '{"type":"assistant.turn_start","data":{"turnId":"0"}}'
echo '{"type":"assistant.reasoning_delta","data":{"reasoningId":"r1","deltaContent":"thinking it over"}}'
echo '{"type":"tool.execution_start","data":{"toolCallId":"call-'"$n"'","toolName":"bash","arguments":{"command":"ls"}}}'
echo '{"type":"tool.execution_complete","data":{"toolCallId":"call-'"$n"'","success":true,"result":{"content":"notes.txt"}}}'
echo '{"type":"assistant.message_delta","data":{"messageId":"m'"$n"'","deltaContent":"Answer '"$n"'"}}'
echo '{"type":"assistant.message","data":{"messageId":"m'"$n"'","content":"Answer '"$n"'"}}'
echo '{"type":"assistant.turn_end","data":{"turnId":"0"}}'
echo '{"type":"result","exitCode":0}'
"#
            .to_string(),
        ),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_two_turns_resume_the_cli_conversation_for_every_cli() {
    for kind in CliKind::ALL {
        let harness = Harness::new(kind).await;
        let (conversation, body) = fake_two_turn_script(kind);
        let program = harness.fake_cli(kind, &body);

        let (first_run, first) = harness.turn(kind, &program, "First question", None).await;
        let (second_run, second) = harness.turn(kind, &program, "Second question", None).await;

        let methods = |events: &[(String, Value)]| {
            events
                .iter()
                .map(|(method, _)| method.clone())
                .collect::<Vec<_>>()
        };
        for events in [&first, &second] {
            let methods = methods(events);
            assert_eq!(
                methods.first().map(String::as_str),
                Some("run.started"),
                "{kind:?}"
            );
            assert_eq!(
                methods.last().map(String::as_str),
                Some("run.completed"),
                "{kind:?}: {events:?}"
            );
            for expected in [
                "reasoning.delta",
                "message.delta",
                "tool.started",
                "tool.completed",
                "message.completed",
            ] {
                assert!(
                    methods.iter().any(|method| method == expected),
                    "{kind:?} missing {expected}: {methods:?}"
                );
            }
        }
        // The second call resumed the first call's conversation.
        let second_args = harness.args(2);
        if conversation.is_empty() {
            let first_id = argument_after(&harness.args(1), "--session-id").map(str::to_string);
            assert!(first_id.is_some(), "{kind:?} chose a conversation id");
            assert_eq!(
                argument_after(&second_args, "--session-id").map(str::to_string),
                first_id,
                "{kind:?}"
            );
        } else {
            assert!(
                second_args.iter().any(|arg| arg == conversation),
                "{kind:?}: {second_args:?}"
            );
            assert!(
                !harness.args(1).iter().any(|arg| arg == conversation),
                "{kind:?} first turn starts fresh"
            );
        }
        // A resumed turn gets only the new message.
        let prompt_of = |call: usize| {
            let stdin = harness.stdin(call);
            if stdin.is_empty() {
                harness.args(call).join("\n")
            } else {
                stdin
            }
        };
        assert!(prompt_of(2).contains("Second question"), "{kind:?}");
        assert!(
            !prompt_of(2).contains("First question"),
            "{kind:?} resent history"
        );

        // Both turns are persisted, in order, as durable session items.
        let transcript = harness.transcript().await;
        let messages = transcript
            .iter()
            .filter(|(kind, _)| kind == "user_message" || kind == "assistant_message")
            .map(|(_, text)| text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            ["First question", "Answer 1", "Second question", "Answer 2"],
            "{kind:?}: {transcript:?}"
        );
        assert!(
            transcript
                .iter()
                .any(|(kind, text)| kind == "reasoning" && text.contains("thinking it over")),
            "{kind:?}"
        );
        assert_eq!(
            transcript
                .iter()
                .filter(|(kind, text)| kind == "tool_result" && text.ends_with(":complete"))
                .count(),
            2,
            "{kind:?}: {transcript:?}"
        );
        for run in [&first_run, &second_run] {
            assert_eq!(
                harness.repository.get_run(run).await.unwrap().status,
                "completed",
                "{kind:?}"
            );
        }
    }
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_cancel_stops_the_cli_process_group_without_orphans() {
    struct PartialAnswerSink<'a> {
        inner: &'a TestSink,
        received: tokio::sync::Notify,
    }
    impl FrameSink for PartialAnswerSink<'_> {
        async fn publish(&self, frame: RpcFrame) {
            let partial_answer = frame.method.as_deref() == Some("message.delta");
            self.inner.publish(frame).await;
            if partial_answer {
                self.received.notify_one();
            }
        }
    }
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(
        kind,
        r#"
sleep 300 &
echo $! > "$LOG/helper.pid"
echo $$ > "$LOG/cli.pid"
echo '{"type":"system","subtype":"init","session_id":"claude-conv-1"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Working"}}}'
sleep 300
"#,
    );
    let run_id = harness.new_run(kind, "Long task").await;
    let sink = TestSink {
        repository: harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    };
    let observed = PartialAnswerSink {
        inner: &sink,
        received: tokio::sync::Notify::new(),
    };
    let (cancel, receiver) = watch::channel(false);
    let context = harness.context(kind, &program, &run_id, "Long task", None);
    let pid_file = harness.log.path().join("cli.pid");
    let helper_file = harness.log.path().join("helper.pid");
    let canceller = async {
        // PID files prove child progress, not that the parent received text.
        // Keep the previous five-second readiness budget; always cancel so a
        // missing delta still cleans up the CLI and its helper before failing.
        let received = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            observed.received.notified(),
        )
        .await
        .is_ok();
        cancel.send(true).unwrap();
        received
    };
    let started = std::time::Instant::now();
    let (_, received_partial) = tokio::join!(
        execute_turn(&harness.repository, &observed, context, receiver),
        canceller
    );
    assert!(
        received_partial,
        "the parent received the partial answer before cancel"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(10));

    let read_pid = |path: &Path| {
        std::fs::read_to_string(path)
            .unwrap()
            .trim()
            .parse::<i32>()
            .unwrap()
    };
    for pid in [read_pid(&pid_file), read_pid(&helper_file)] {
        let mut alive = true;
        for _ in 0..100 {
            // SAFETY: signal 0 only checks that the process exists.
            alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(!alive, "process {pid} survived the cancel");
    }
    let emitted = sink.emitted.into_inner().unwrap();
    assert_eq!(
        emitted.last().map(|(method, _)| method.as_str()),
        Some("run.cancelled")
    );
    let run = harness.repository.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, "cancelled");
    // The partial answer stays in the session.
    let transcript = harness.transcript().await;
    println!(
        "CANCEL_OBSERVATION={}",
        json!({
            "received_partial_before_cancel": received_partial,
            "cli_pid": read_pid(&pid_file),
            "helper_pid": read_pid(&helper_file),
            // SAFETY: signal 0 only checks these run-owned process ids.
            "cli_alive_after_cancel": unsafe { libc::kill(read_pid(&pid_file), 0) } == 0,
            "helper_alive_after_cancel": unsafe { libc::kill(read_pid(&helper_file), 0) } == 0,
            "elapsed_ms": started.elapsed().as_millis(),
            "status": run.status,
            "events": emitted,
            "transcript": transcript,
        })
    );
    assert!(transcript.contains(&("assistant_message".to_string(), "Working".to_string())));
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_message_sent_during_a_run_becomes_the_next_turn() {
    // While a CLI turn runs, the composer's message is not steered into it
    // (`steer_agent_run` answers `cli_engine`); it is sent when the turn
    // ends, as the next turn of the same CLI conversation.
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(kind, FAKE_CLAUDE);
    let gate = harness.log.path().join("release");
    let slow = harness.fake_cli(
        kind,
        &format!(
            "while [ ! -f '{}' ]; do sleep 0.05; done\n{FAKE_CLAUDE}",
            gate.display()
        ),
    );
    assert_eq!(slow, program);
    let run_id = harness.new_run(kind, "First question").await;
    let sink = TestSink {
        repository: harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    };
    let (_cancel, receiver) = watch::channel(false);
    let first = execute_turn(
        &harness.repository,
        &sink,
        harness.context(kind, &program, &run_id, "First question", None),
        receiver,
    );
    let queued_while_running = async {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        let run = harness.repository.get_run(&run_id).await.unwrap();
        assert_eq!(run.status, "running");
        assert!(
            cli_for_model(&run.model).is_some(),
            "the run refuses live steering"
        );
        std::fs::write(&gate, "").unwrap();
        "Follow-up sent during the run"
    };
    let ((), follow_up) = tokio::join!(first, queued_while_running);
    let (next_run, _) = harness.turn(kind, &program, follow_up, None).await;

    assert!(harness.args(2).iter().any(|arg| arg == "claude-conv-1"));
    assert_eq!(harness.stdin(2), follow_up);
    assert_eq!(
        harness.repository.get_run(&next_run).await.unwrap().status,
        "completed"
    );
    let users = harness
        .transcript()
        .await
        .into_iter()
        .filter(|(kind, _)| kind == "user_message")
        .map(|(_, text)| text)
        .collect::<Vec<_>>();
    assert_eq!(users, ["First question", follow_up]);
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_hands_clovy_mcp_server_only_to_clis_that_accept_it() {
    let launch = McpLaunch {
        command: PathBuf::from("/Applications/Clovy.app/Contents/Resources/native/bin/clovy-mcp"),
        args: vec!["--dir".into(), "/tmp/clovy data/mcp".into()],
    };
    // Each fake CLI reads the configuration it was given and reports it.
    let readers = [
        (
            CliKind::Claude,
            r#"cfg=$(sed -n '/^--mcp-config$/{n;p;}' "$LOG/$n.args"); cp "$cfg" "$LOG/mcp.json" 2>/dev/null
echo '{"type":"system","subtype":"init","session_id":"c"}'
echo '{"type":"result","subtype":"success","is_error":false,"result":""}'"#,
        ),
        (
            CliKind::Copilot,
            r#"cfg=$(sed -n 's/^--additional-mcp-config=@//p' "$LOG/$n.args"); cp "$cfg" "$LOG/mcp.json" 2>/dev/null
echo '{"type":"result","exitCode":0}'"#,
        ),
        (
            CliKind::Codex,
            r#"echo '{"type":"thread.started","thread_id":"t"}'
echo '{"type":"turn.completed"}'"#,
        ),
        (CliKind::Pi, r#"echo '{"type":"agent_end","messages":[]}'"#),
        (
            CliKind::Agy,
            r#"cp .agents/mcp_config.json "$LOG/mcp.json"
echo '{"event":"result","result":{"status":"SUCCESS","response":"Configured"}}'"#,
        ),
        (
            CliKind::CursorAgent,
            r#"echo '{"type":"result","subtype":"success","is_error":false,"result":""}'"#,
        ),
    ];
    for (kind, body) in readers {
        let harness = Harness::new(kind).await;
        let program = harness.fake_cli(kind, body);
        let (run_id, _) = harness
            .turn(kind, &program, "Use Clovy's tools", Some(launch.clone()))
            .await;
        assert_eq!(
            harness.repository.get_run(&run_id).await.unwrap().status,
            "completed",
            "{kind:?}"
        );
        let args = harness.args(1);
        let config = std::fs::read_to_string(harness.log.path().join("mcp.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok());
        match kind {
            CliKind::Claude | CliKind::Copilot => {
                let config =
                    config.unwrap_or_else(|| panic!("{kind:?} received no MCP configuration"));
                let server = &config["mcpServers"]["clovy"];
                assert_eq!(
                    server["command"],
                    json!(launch.command.display().to_string()),
                    "{kind:?}"
                );
                assert_eq!(server["args"], json!(launch.args), "{kind:?}");
            }
            CliKind::Agy => {
                let config = config.expect("Agy received workspace MCP configuration");
                let servers = config["mcpServers"].as_object().unwrap();
                assert_eq!(servers.len(), 1);
                let server = servers.values().next().unwrap();
                assert_eq!(server["command"], json!(launch.command));
                assert_eq!(server["args"], json!(launch.args));
                assert_eq!(server["timeoutSeconds"], 900);
                assert!(!harness
                    .workspace
                    .path()
                    .join(".agents/mcp_config.json")
                    .exists());
            }
            CliKind::Codex => {
                assert!(args.contains(&format!(
                    "mcp_servers.clovy.command=\"{}\"",
                    launch.command.display()
                )));
                assert!(args.contains(
                    &"mcp_servers.clovy.args=[\"--dir\",\"/tmp/clovy data/mcp\"]".to_string()
                ));
            }
            _ => {
                assert!(config.is_none(), "{kind:?}");
                assert!(
                    !args.iter().any(|arg| arg.contains("mcp")),
                    "{kind:?}: {args:?}"
                );
            }
        }
    }
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_shows_actions_the_cli_refused_for_lack_of_approval() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    // Measured from claude 2.1 in a non-interactive turn.
    let program = harness.fake_cli(
        kind,
        r#"
echo '{"type":"system","subtype":"init","session_id":"c"}'
echo '{"type":"assistant","message":{"content":[{"type":"tool_use","id":"w1","name":"Write","input":{"file_path":"/tmp/x"}}]}}'
echo '{"type":"user","message":{"content":[{"type":"tool_result","content":"Claude requested permissions to write to /tmp/x, but you haven'"'"'t granted it yet.","is_error":true,"tool_use_id":"w1"}]}}'
echo '{"type":"assistant","message":{"content":[{"type":"text","text":"The write was blocked."}]}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"The write was blocked.","permission_denials":[{"tool_name":"Write","tool_use_id":"w1"}]}'
"#,
    );
    let (_, emitted) = harness.turn(kind, &program, "Write a file", None).await;
    let failed = emitted
        .iter()
        .find(|(method, _)| method == "tool.failed")
        .map(|(_, data)| data.clone())
        .expect("the refused tool is shown as failed");
    assert_eq!(failed["needsApproval"], json!(true));
    assert!(failed["error"]
        .as_str()
        .unwrap()
        .contains("needs your approval"));
    assert_eq!(emitted.last().unwrap().0, "run.completed");
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_failures_end_the_run_with_the_cli_error() {
    let kind = CliKind::Codex;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(
        kind,
        r#"
echo '{"type":"thread.started","thread_id":"t1"}'
echo '{"type":"error","message":"stream disconnected"}'
echo 'Not logged in. Run codex login.' >&2
exit 1
"#,
    );
    let (run_id, emitted) = harness.turn(kind, &program, "Hi", None).await;
    let (method, data) = emitted.last().unwrap();
    assert_eq!(method, "run.failed");
    assert_eq!(data["message"], json!("stream disconnected"));
    let run = harness.repository.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, "failed");
    // The conversation id was saved, so a retry resumes it.
    let next = harness.fake_cli(
        kind,
        r#"grep -qx t1 "$LOG/$n.args" || exit 3
echo '{"type":"item.completed","item":{"id":"a","type":"agent_message","text":"Back"}}'
echo '{"type":"turn.completed"}'"#,
    );
    let (retry, _) = harness.turn(kind, &next, "Hi", None).await;
    assert_eq!(
        harness.repository.get_run(&retry).await.unwrap().status,
        "completed"
    );
}

/// What to configure when Codex reports that its Windows sandbox is not set
/// up or its setup failed (texts from codex-rs `core/src/exec.rs`,
/// `windows-sandbox-rs/src/setup_error.rs` and the TUI).
fn assert_sandbox_hint(text: &str, detail: &str) {
    assert!(
        text.contains("codex sandbox setup --elevated --current-user"),
        "{text}"
    );
    assert!(text.contains("administrator"), "{text}");
    assert!(text.contains(detail), "keeps what Codex said: {text}");
}

/// Codex's Windows sandbox failures, as codex-rs writes them: the exec
/// prefixes and the setup codes of `windows-sandbox-rs/src/setup_error.rs`.
#[test]
fn chat_engine_recognizes_only_codex_windows_sandbox_failures() {
    use super::cli::{codex_sandbox_hint, codex_windows_sandbox_failure};
    for text in [
        "windows sandbox: failed to resolve codex_home: not found",
        "windows sandbox join error: task panicked",
        "invalid Windows sandbox workspace roots: C:\\x",
        "Windows sandbox setup failed: orchestrator_elevation_required: run elevated",
        "Windows sandbox setup failed.",
        "orchestrator_helper_launch_canceled: the UAC prompt was canceled",
        "helper_firewall_rule_create_or_add_failed: access denied",
        "error: helper_unknown_error",
    ] {
        assert!(codex_windows_sandbox_failure(text, true), "{text}");
        assert_sandbox_hint(&codex_sandbox_hint(text, true), text);
        // Never outside Windows.
        assert!(!codex_windows_sandbox_failure(text, false), "{text}");
        assert_eq!(codex_sandbox_hint(text, false), text);
    }
    for text in [
        "error: orchestrator_config.yaml not found",
        "make: *** [sandbox setup] Error 2",
        "test my_orchestrator_test ... FAILED",
        "helper_scripts/run.sh: permission denied",
        "Sandbox setup complete",
        "You have hit your usage limit.",
    ] {
        assert!(!codex_windows_sandbox_failure(text, true), "{text}");
        assert_eq!(codex_sandbox_hint(text, true), text);
    }
}

/// Outside Windows a Codex sandbox text keeps Codex's own words: no hint to
/// set up a Windows sandbox.
#[tokio::test]
#[cfg(unix)]
async fn chat_engine_codex_windows_sandbox_failure_gets_no_hint_outside_windows() {
    let kind = CliKind::Codex;
    let harness = Harness::new(kind).await;
    let detail =
        "windows sandbox: orchestrator_helper_launch_failed: failed to launch setup helper";
    let program = harness.fake_cli(
        kind,
        &format!(
            r#"
echo '{{"type":"thread.started","thread_id":"t1"}}'
echo '{{"type":"turn.failed","error":{{"message":"{detail}"}}}}'
exit 1
"#
        ),
    );
    let (_, emitted) = harness.turn(kind, &program, "Hi", None).await;
    let (method, data) = emitted.last().unwrap();
    assert_eq!(method, "run.failed");
    assert_no_sandbox_hint(data["message"].as_str().unwrap());
    assert!(data["message"].as_str().unwrap().contains(detail));

    // Only on stderr, the way an older codex exits.
    let stderr =
        "Windows sandbox setup failed: helper_firewall_rule_create_or_add_failed: access denied";
    let program = harness.fake_cli(
        kind,
        &format!(
            r#"
echo '{{"type":"thread.started","thread_id":"t2"}}'
echo '{stderr}' >&2
exit 1
"#
        ),
    );
    let (_, emitted) = harness.turn(kind, &program, "Hi", None).await;
    let (method, data) = emitted.last().unwrap();
    assert_eq!(method, "run.failed");
    assert_no_sandbox_hint(data["message"].as_str().unwrap());
}

#[cfg(unix)]
fn assert_no_sandbox_hint(text: &str) {
    assert!(!text.contains("codex sandbox setup"), "{text}");
}

/// A command that could not start in the sandbox fails the tool step with
/// Codex's words (no Windows hint outside Windows), and the turn still ends
/// normally.
#[tokio::test]
#[cfg(unix)]
async fn chat_engine_codex_command_blocked_by_the_sandbox_fails_the_step_only() {
    let kind = CliKind::Codex;
    let harness = Harness::new(kind).await;
    let detail = "windows sandbox: helper_sandbox_lock_failed: SetFileAttributesW failed";
    let program = harness.fake_cli(
        kind,
        &format!(
            r#"
echo '{{"type":"thread.started","thread_id":"t1"}}'
echo '{{"type":"item.started","item":{{"id":"c1","type":"command_execution","command":"dir","aggregated_output":"","exit_code":null,"status":"in_progress"}}}}'
echo '{{"type":"item.completed","item":{{"id":"c1","type":"command_execution","command":"dir","aggregated_output":"{detail}","exit_code":null,"status":"failed"}}}}'
echo '{{"type":"item.completed","item":{{"id":"a","type":"agent_message","text":"I could not run it."}}}}'
echo '{{"type":"turn.completed"}}'
"#
        ),
    );
    let (_, emitted) = harness.turn(kind, &program, "List files", None).await;
    let failed = emitted
        .iter()
        .find(|(method, _)| method == "tool.failed")
        .map(|(_, data)| data.clone())
        .expect("the blocked command is shown as failed");
    assert_eq!(failed["error"], json!(detail));
    assert_eq!(emitted.last().unwrap().0, "run.completed");
}

/// Other failures keep Codex's own words.
#[tokio::test]
#[cfg(unix)]
async fn chat_engine_codex_other_failures_get_no_sandbox_hint() {
    let kind = CliKind::Codex;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(
        kind,
        r#"
echo '{"type":"thread.started","thread_id":"t1"}'
echo '{"type":"turn.failed","error":{"message":"You have hit your usage limit."}}'
exit 1
"#,
    );
    let (_, emitted) = harness.turn(kind, &program, "Hi", None).await;
    let (_, data) = emitted.last().unwrap();
    assert_eq!(data["message"], json!("You have hit your usage limit."));
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_new_cli_conversation_gets_earlier_messages_of_the_session() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    // An earlier turn on the Clovy engine.
    let earlier = harness
        .repository
        .create_run(&harness.session_id, "open-software/auto", None)
        .await
        .unwrap();
    for (sequence, payload) in [
        (
            0,
            AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: "My dog is Rex".into(),
                attachments: Vec::new(),
            }),
        ),
        (
            1,
            AgentItemPayload::AssistantMessage(MessagePayload {
                role: "assistant".into(),
                content: "Noted, Rex.".into(),
                attachments: Vec::new(),
            }),
        ),
        (
            2,
            AgentItemPayload::Steering(crate::agent_sessions::TextPayload {
                text: "Correction: my dog is actually called Max.".into(),
                metadata: None,
            }),
        ),
    ] {
        harness
            .repository
            .append_item(
                &harness.session_id,
                Some(&earlier.id),
                sequence,
                &payload,
                None,
            )
            .await
            .unwrap();
    }
    let program = harness.fake_cli(kind, FAKE_CLAUDE);
    harness
        .turn(kind, &program, "What is my dog called?", None)
        .await;
    let prompt = harness.stdin(1);
    assert!(prompt.contains("User: My dog is Rex"), "{prompt}");
    assert!(prompt.contains("Assistant: Noted, Rex."), "{prompt}");
    assert!(
        prompt.contains("User: Correction: my dog is actually called Max."),
        "{prompt}"
    );
    assert!(prompt.ends_with("What is my dog called?"), "{prompt}");
    assert!(!harness.args(1).iter().any(|arg| arg == "--resume"));
}

#[tokio::test]
#[cfg(unix)]
async fn chat_history_replays_available_attachments_even_without_text() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let earlier = harness
        .repository
        .create_run(&harness.session_id, "open-software/auto", None)
        .await
        .unwrap();
    let attachment = |name: &str, available| crate::agent_sessions::MessageAttachmentPayload {
        id: name.into(),
        name: name.into(),
        path: harness.workspace.path().join(name).to_string_lossy().into(),
        mime_type: Some("text/plain".into()),
        size_bytes: 4,
        available,
        created_at: "now".into(),
    };
    let present = attachment("historical-brief.txt", true);
    std::fs::write(&present.path, "plan").unwrap();
    harness
        .repository
        .append_item(
            &harness.session_id,
            Some(&earlier.id),
            0,
            &AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: "old context ".repeat(4000),
                attachments: vec![],
            }),
            None,
        )
        .await
        .unwrap();
    harness
        .repository
        .append_item(
            &harness.session_id,
            Some(&earlier.id),
            1,
            &AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: String::new(),
                attachments: vec![present.clone(), attachment("unavailable.txt", false)],
            }),
            None,
        )
        .await
        .unwrap();
    let program = harness.fake_cli(kind, FAKE_CLAUDE);
    harness
        .turn(kind, &program, "Use the earlier brief", None)
        .await;
    let prompt = harness.stdin(1);
    assert!(
        prompt.contains("[Clovy attachment manifest v1]"),
        "missing attachment manifest"
    );
    assert!(
        prompt.contains(&present.path),
        "missing available historical path"
    );
    assert!(!prompt.contains("unavailable.txt"));
    assert!(prompt.ends_with("Use the earlier brief"));
    assert!(
        prompt.chars().count() < 24_200,
        "history must remain bounded"
    );
}

#[tokio::test]
#[cfg(unix)]
async fn chat_retry_starts_fresh_and_excludes_rejected_attempts() {
    for kind in [CliKind::Claude, CliKind::Codex] {
        for prior_model in ["open-software/auto".to_string(), engine_model_id(kind)] {
            let harness = Harness::new(kind).await;
            let accepted = harness
                .repository
                .create_run(&harness.session_id, "open-software/auto", None)
                .await
                .unwrap();
            harness
                .repository
                .append_item(
                    &harness.session_id,
                    Some(&accepted.id),
                    0,
                    &AgentItemPayload::UserMessage(MessagePayload {
                        role: "user".into(),
                        content: "Keep accepted context".into(),
                        attachments: vec![],
                    }),
                    None,
                )
                .await
                .unwrap();
            let rejected = harness
                .repository
                .create_run(&harness.session_id, &prior_model, None)
                .await
                .unwrap();
            for (sequence, payload) in [
                (
                    0,
                    AgentItemPayload::UserMessage(MessagePayload {
                        role: "user".into(),
                        content: "Retry this request".into(),
                        attachments: vec![],
                    }),
                ),
                (
                    1,
                    AgentItemPayload::AssistantMessage(MessagePayload {
                        role: "assistant".into(),
                        content: "Rejected partial answer".into(),
                        attachments: vec![],
                    }),
                ),
            ] {
                harness
                    .repository
                    .append_item(
                        &harness.session_id,
                        Some(&rejected.id),
                        sequence,
                        &payload,
                        None,
                    )
                    .await
                    .unwrap();
            }
            harness
            .repository
            .set_run_config(
                &rejected.id,
                &json!({"engine":"cli", "cli":kind.id(), "conversationId":"rejected-conversation"}),
            )
            .await
            .unwrap();
            let successful = match kind {
                CliKind::Claude => {
                    r#"echo '{"type":"system","subtype":"init","session_id":"fresh-retry"}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Accepted replacement","session_id":"fresh-retry"}'"#
                }
                CliKind::Codex => {
                    r#"echo '{"type":"thread.started","thread_id":"fresh-retry"}'
echo '{"type":"item.completed","item":{"id":"answer","type":"agent_message","text":"Accepted replacement"}}'
echo '{"type":"turn.completed","usage":{"input_tokens":1,"output_tokens":1}}'"#
                }
                _ => unreachable!(),
            };
            // First retry exits before reporting a conversation id. A second retry
            // still needs the original rejection excluded from its local history.
            let program = harness.fake_cli(kind, &format!("if [ \"$n\" = 1 ]; then echo 'synthetic launch failure' >&2; exit 1; fi\n{successful}"));
            let mut previous = rejected.id;
            // A retry of a retry must also exclude the original rejected attempt.
            for call in 1..=2 {
                let run_id = harness.new_run(kind, "Retry this request").await;
                let sink = TestSink {
                    repository: harness.repository.clone(),
                    emitted: StdMutex::new(Vec::new()),
                };
                let mut context =
                    harness.context(kind, &program, &run_id, "Retry this request", None);
                context.previous_run_id = Some(previous);
                let (_cancel, receiver) = watch::channel(false);
                execute_turn(&harness.repository, &sink, context, receiver).await;
                assert_eq!(
                    harness.repository.get_run(&run_id).await.unwrap().status,
                    if call == 1 { "failed" } else { "completed" }
                );
                let prompt = harness.stdin(call);
                assert!(
                    !harness
                        .args(call)
                        .iter()
                        .any(|arg| arg == "--resume" || arg == "resume"),
                    "retry resumed a rejected conversation"
                );
                assert!(prompt.contains("Keep accepted context"));
                assert!(!prompt.contains("Rejected partial answer"));
                assert_eq!(
                    prompt.matches("Retry this request").count(),
                    1,
                    "request replayed twice"
                );
                previous = run_id;
            }
        }
    }
}

#[tokio::test]
#[cfg(unix)]
async fn chat_history_excludes_persisted_retry_parents_after_failure_or_engine_switch() {
    for switched in [false, true] {
        let kind = CliKind::Claude;
        let harness = Harness::new(kind).await;
        let original = harness.new_run(kind, "Original rejected request").await;
        harness
            .repository
            .append_item(
                &harness.session_id,
                Some(&original),
                1,
                &AgentItemPayload::AssistantMessage(MessagePayload {
                    role: "assistant".into(),
                    content: "Rejected partial answer".into(),
                    attachments: vec![],
                }),
                None,
            )
            .await
            .unwrap();
        harness
            .repository
            .set_run_config(
                &original,
                &json!({"engine":"cli", "cli":"claude", "conversationId":"rejected-conversation"}),
            )
            .await
            .unwrap();
        let prior_kind = if switched { CliKind::Codex } else { kind };
        let retried = harness.new_run(prior_kind, "Accepted retry request").await;
        let config = if switched {
            json!({"engine":"cli", "cli":"codex", "conversationId":"clean-retry", "retryOfRunId":original})
        } else {
            // A failed attempt never reported a conversation id.
            json!({"retryOfRunId":original})
        };
        harness
            .repository
            .set_run_config(&retried, &config)
            .await
            .unwrap();
        // Foreign-session ancestry must not suppress this session's messages.
        let foreign = harness
            .repository
            .create_session(
                "Other session",
                &engine_model_id(kind),
                AgentSafetyMode::Sandboxed,
                None,
            )
            .await
            .unwrap();
        let foreign_run = harness
            .repository
            .create_run(&foreign.id, &engine_model_id(kind), None)
            .await
            .unwrap();
        harness
            .repository
            .set_run_config(&foreign_run.id, &json!({"retryOfRunId":retried}))
            .await
            .unwrap();
        let program = harness.fake_cli(kind, FAKE_CLAUDE);
        harness
            .turn(kind, &program, "Continue after the retry", None)
            .await;
        let prompt = harness.stdin(1);
        assert!(
            !harness.args(1).iter().any(|arg| arg == "--resume"),
            "crossed a failed retry boundary"
        );
        assert!(!prompt.contains("Rejected partial answer"));
        assert!(!prompt.contains("Original rejected request"));
        assert!(prompt.contains("Accepted retry request"));
        assert!(prompt.ends_with("Continue after the retry"));
    }
}

#[test]
fn chat_engine_model_ids_round_trip() {
    for kind in CliKind::ALL {
        assert_eq!(cli_for_model(&engine_model_id(kind)), Some(kind));
    }
    assert_eq!(cli_for_model("__june_local_generation__:llama3"), None);
    assert_eq!(cli_for_model("__clovy_cli_engine__:unknown"), None);
    assert_eq!(cli_for_model("open-software/auto"), None);
}

const ATTENDED: policy::TurnPermissions = policy::TurnPermissions {
    safety_mode: AgentSafetyMode::Sandboxed,
    attended: true,
    routine_tools: None,
};
const UNATTENDED: policy::TurnPermissions = policy::TurnPermissions {
    safety_mode: AgentSafetyMode::Sandboxed,
    attended: false,
    routine_tools: Some(policy::RoutineBuiltinTools {
        file: true,
        web: true,
    }),
};

#[tokio::test]
#[cfg(unix)]
async fn policy_fake_codex_receives_an_existing_workspace_image() {
    let kind = CliKind::Codex;
    let harness = Harness::new(kind).await;
    let attachments = harness.workspace.path().join("attachments");
    std::fs::create_dir(&attachments).unwrap();
    let image_path = attachments.join("synthetic.png");
    image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]))
        .save(&image_path)
        .unwrap();
    let program = harness.fake_cli(kind, r#"
previous=''
for argument in "$@"; do
  if [ "$previous" = '--image' ]; then
    test -f "$argument" || exit 8
    printf '%s' "$argument" > "$LOG/image-read"
  fi
  previous="$argument"
done
test -s "$LOG/image-read" || exit 9
echo '{"type":"thread.started","thread_id":"image-fixture"}'
echo '{"type":"item.completed","item":{"id":"answer","type":"agent_message","text":"Image file received."}}'
echo '{"type":"turn.completed"}'
"#);
    let manifest = crate::agent_sessions::api::cli_message_with_attachments(
        "Inspect the image.",
        &[crate::agent_sessions::MessageAttachmentPayload {
            id: "synthetic".into(),
            name: "synthetic.png".into(),
            path: image_path.to_string_lossy().into(),
            mime_type: Some("image/png".into()),
            size_bytes: i64::try_from(std::fs::metadata(&image_path).unwrap().len()).unwrap(),
            available: true,
            created_at: "2026-10-28T00:00:00Z".into(),
        }],
    );
    let run_id = harness.new_run(kind, &manifest).await;
    let mut context = harness.context(kind, &program, &run_id, &manifest, None);
    context.images = vec![image_path.clone()];
    let sink = TestSink {
        repository: harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    };
    let (_cancel, receiver) = tokio::sync::watch::channel(false);
    execute_turn(&harness.repository, &sink, context, receiver).await;
    assert_eq!(
        harness.repository.get_run(&run_id).await.unwrap().status,
        "completed"
    );
    let argv = harness.args(1);
    let image_arg = argv.windows(2).find(|pair| pair[0] == "--image").unwrap();
    assert_eq!(Path::new(&image_arg[1]), image_path);
    assert!(Path::new(&image_arg[1]).is_file());
    assert_eq!(
        std::fs::read_to_string(harness.log.path().join("image-read")).unwrap(),
        image_path.to_string_lossy()
    );
    assert!(harness
        .stdin(1)
        .contains(&image_path.to_string_lossy().to_string()));
}

#[test]
fn chat_engine_unattended_claude_turn_has_only_read_tools_clovy_mcp_and_web() {
    let launch = McpLaunch {
        command: PathBuf::from("/bin/clovy-mcp"),
        args: vec![],
    };
    let scratch = tempfile::tempdir().unwrap();
    let invocation = cli::turn_invocation(
        CliKind::Claude,
        "hello",
        None,
        Some(&launch),
        scratch.path(),
        UNATTENDED,
        &[],
    );
    let value_after = |flag: &str| {
        let at = invocation
            .args
            .iter()
            .position(|arg| arg == flag)
            .unwrap_or_else(|| panic!("{flag} missing: {:?}", invocation.args));
        invocation.args[at + 1].clone()
    };
    // Bash, Edit, Write and every other built-in do not exist in the turn.
    let tools = value_after("--tools");
    let tools = tools.split(',').collect::<Vec<_>>();
    assert_eq!(tools, ["Read", "Grep", "Glob", "WebSearch", "WebFetch"]);
    let allowed = value_after("--allowedTools");
    let allowed = allowed.split(',').collect::<Vec<_>>();
    assert!(allowed.contains(&"mcp__clovy__*"), "{allowed:?}");
    assert!(
        !allowed
            .iter()
            .any(|tool| ["Bash", "Edit", "Write", "MultiEdit", "NotebookEdit"].contains(tool)),
        "{allowed:?}"
    );
    assert_eq!(value_after("--permission-mode"), "default");
    // Only Clovy's MCP server, never the user's others.
    assert!(invocation.args.contains(&"--strict-mcp-config".to_string()));
    assert!(invocation.args.contains(&"--mcp-config".to_string()));
    // Exactly one --allowedTools: the attended `mcp__clovy__*` grant is not
    // added on top of the read-only set.
    assert_eq!(
        invocation
            .args
            .iter()
            .filter(|arg| *arg == "--allowedTools")
            .count(),
        1
    );
    // A chat turn loads the user's own MCP servers too (D7).
    let chat = cli::turn_invocation(
        CliKind::Claude,
        "hello",
        None,
        Some(&launch),
        scratch.path(),
        ATTENDED,
        &[],
    );
    for flag in ["--tools", "--strict-mcp-config"] {
        assert!(!chat.args.contains(&flag.to_string()), "chat passes {flag}");
    }
}

#[test]
fn chat_engine_unattended_codex_turn_is_read_only_and_never_waits_for_approval() {
    let scratch = tempfile::tempdir().unwrap();
    for conversation in [None, Some("thread-1")] {
        let invocation = cli::turn_invocation(
            CliKind::Codex,
            "hello",
            conversation,
            None,
            scratch.path(),
            UNATTENDED,
            &[],
        );
        assert!(invocation
            .args
            .contains(&"sandbox_mode=\"read-only\"".to_string()));
        assert!(!invocation
            .args
            .contains(&"sandbox_mode=\"workspace-write\"".to_string()));
        assert!(invocation
            .args
            .contains(&"approval_policy=\"never\"".to_string()));
    }
}

#[test]
fn chat_engine_refuses_unattended_turns_on_clis_it_cannot_confine() {
    for kind in CliKind::ALL {
        let checked = check_policy(kind, false);
        if matches!(kind, CliKind::Claude | CliKind::Codex) {
            assert!(checked.is_ok(), "{kind:?}");
        } else {
            assert_eq!(
                checked.unwrap_err().code,
                "agent_cli_unattended_unsupported",
                "{kind:?}"
            );
        }
        assert!(check_policy(kind, true).is_ok());
    }
}

#[test]
fn chat_engine_tells_the_cli_how_to_read_referenced_notes() {
    // The note chat panel's first message: its note reference, then the
    // question.
    let message =
        r#"@note:7f3c-a_1 ("Weekly sync") What did we decide? See @note:7f3c-a_1 and @note:b2."#;
    assert_eq!(note_references(message), ["7f3c-a_1", "b2"]);
    let input = with_note_reference_context(message);
    assert!(input.starts_with(message), "{input}");
    assert!(input.contains("`get_note`"), "{input}");
    assert!(input.contains("`7f3c-a_1`, `b2`"), "{input}");
    // A message without note references reaches the CLI unchanged.
    assert_eq!(with_note_reference_context("hello"), "hello");
    assert!(note_references("mail me@note:").is_empty());
}

fn cursor_text(lines: &[&str]) -> (String, Vec<EngineEvent>) {
    let mut translator = StreamTranslator::new(CliKind::CursorAgent);
    let events = lines
        .iter()
        .flat_map(|line| translator.translate_line(line))
        .collect::<Vec<_>>();
    let text = events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::TextDelta(text) | EngineEvent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    (text, events)
}

#[test]
fn chat_engine_cursor_drops_the_whole_copy_of_a_streamed_segment() {
    // Shapes as captured from cursor-agent 2026.09 (`cursor-t2.jsonl`): the
    // whole copy before a tool call carries `model_call_id`, the one at the
    // end has no `timestamp_ms`.
    let (text, events) = cursor_text(&[
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"The"}]},"timestamp_ms":1}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":" plan."}]},"timestamp_ms":2}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"The plan."}]},"model_call_id":"m-0","timestamp_ms":3}"#,
        r#"{"type":"tool_call","subtype":"started","call_id":"c1","tool_call":{"shellToolCall":{"args":{"command":"touch x"}}}}"#,
        r#"{"type":"tool_call","subtype":"completed","call_id":"c1","tool_call":{"shellToolCall":{"result":{"rejected":{"command":"touch x","reason":""}}}}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]},"timestamp_ms":4}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done"}]}}"#,
    ]);
    assert_eq!(text, "The plan.Done");
    assert!(events.iter().any(|event| matches!(
        event,
        EngineEvent::ToolFinished { needs_approval: true, name, .. } if name == "shell"
    )));
}

#[test]
fn chat_engine_cursor_one_delta_segment_before_a_tool_call_is_not_doubled() {
    let (text, _) = cursor_text(&[
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"OK"}]},"timestamp_ms":1}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"OK"}]},"model_call_id":"m-0","timestamp_ms":2}"#,
        r#"{"type":"tool_call","subtype":"started","call_id":"c1","tool_call":{"readToolCall":{"args":{"path":"a"}}}}"#,
        r#"{"type":"tool_call","subtype":"completed","call_id":"c1","tool_call":{"readToolCall":{"result":{"success":{"content":"a"}}}}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"."}]},"timestamp_ms":3}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"."}]}}"#,
    ]);
    assert_eq!(text, "OK.");
    // Without streamed deltas the whole copies are the answer.
    let (text, _) = cursor_text(&[
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Plain answer"}]}}"#,
    ]);
    assert_eq!(text, "Plain answer");
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_app_shutdown_stops_and_awaits_running_cli_turns() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(
        kind,
        r#"
echo '{"type":"system","subtype":"init","session_id":"claude-conv-1"}'
sleep 300 &
echo $! > "$LOG/helper.pid"
echo $$ > "$LOG/cli.pid"
sleep 300
"#,
    );
    let host = std::sync::Arc::new(ChatEngineHost::default());
    let run_id = harness.new_run(kind, "Long task").await;
    let ticket = host.register(&run_id).unwrap();
    let sink = std::sync::Arc::new(TestSink {
        repository: harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    });
    let task = {
        let repository = harness.repository.clone();
        let sink = sink.clone();
        let context = harness.context(kind, &program, &run_id, "Long task", None);
        tokio::spawn(async move {
            let TurnTicket {
                cancel,
                _done: done,
            } = ticket;
            execute_turn(&repository, sink.as_ref(), context, cancel).await;
            drop(done);
        })
    };
    let pid_file = harness.log.path().join("cli.pid");
    let helper_file = harness.log.path().join("helper.pid");
    for _ in 0..200 {
        if pid_file.exists() && helper_file.exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }

    host.shutdown().await;

    // Shutdown returned only after the turn ended and settled the run.
    assert_eq!(
        sink.emitted
            .lock()
            .unwrap()
            .last()
            .map(|(method, _)| method.clone()),
        Some("run.cancelled".to_string())
    );
    assert_eq!(
        harness.repository.get_run(&run_id).await.unwrap().status,
        "cancelled"
    );
    for path in [&pid_file, &helper_file] {
        assert_process_gone(read_pid(path)).await;
    }
    task.await.unwrap();
    // No new turn starts while the app quits.
    assert!(host.register("late-run").is_err());
}

#[cfg(unix)]
fn read_pid(path: &Path) -> i32 {
    std::fs::read_to_string(path)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

#[cfg(unix)]
async fn assert_process_gone(pid: i32) {
    for _ in 0..100 {
        // SAFETY: signal 0 only checks that the process exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("process {pid} is still running");
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_cli_exit_ends_the_turn_even_if_a_helper_keeps_stdout_open() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    // The helper inherits stdout and outlives the CLI.
    let program = harness.fake_cli(
        kind,
        r#"
echo '{"type":"system","subtype":"init","session_id":"claude-conv-1"}'
sleep 300 &
echo $! > "$LOG/helper.pid"
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"All done"}}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"All done"}'
exit 0
"#,
    );
    let started = std::time::Instant::now();
    let finished = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        harness.turn(kind, &program, "Quick task", None),
    )
    .await;
    let (run_id, emitted) = finished.expect("the turn ends when the CLI exits");
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    assert_eq!(
        emitted.last().map(|(method, _)| method.as_str()),
        Some("run.completed")
    );
    assert_eq!(
        harness.repository.get_run(&run_id).await.unwrap().status,
        "completed"
    );
    assert!(harness
        .transcript()
        .await
        .contains(&("assistant_message".to_string(), "All done".to_string())));
    assert_process_gone(read_pid(&harness.log.path().join("helper.pid"))).await;
}

#[tokio::test]
#[cfg(unix)]
async fn chat_engine_cli_environment_keeps_the_profile_but_not_clovy_credentials() {
    let kind = CliKind::Pi;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(
        kind,
        r#"env > "$LOG/env"
echo '{"type":"agent_end","messages":[]}'"#,
    );
    let run_id = harness.new_run(kind, "Hi").await;
    let sink = TestSink {
        repository: harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    };
    let mut context = harness.context(kind, &program, &run_id, "Hi", None);
    for (name, value) in [
        ("HOME", "/Users/someone"),
        ("PI_CODING_AGENT_DIR", "/Users/someone/.pi/agent"),
        ("OPENROUTER_API_KEY", "user-own-key"),
        ("CLOVY_TEST_PRIVATE_TOKEN", "clovy-dev-token"),
        ("CLOVY_TEST_LEGACY_TOKEN", "june-dev-token"),
        ("OS_ACCOUNTS_CLIENT_ID", "clovy-client"),
        (
            "CLOVY_TEST_RETIRED_PROVIDER_ORIGIN",
            "http://127.0.0.1:8787",
        ),
        ("GOOGLE_OAUTH_CLIENT_SECRET", "clovy-oauth-secret"),
    ] {
        context.env.insert(name.to_string(), value.to_string());
    }
    let (_cancel, receiver) = watch::channel(false);
    execute_turn(&harness.repository, &sink, context, receiver).await;

    let env = std::fs::read_to_string(harness.log.path().join("env")).unwrap();
    for kept in [
        "HOME=/Users/someone",
        "PI_CODING_AGENT_DIR=/Users/someone/.pi/agent",
        "PATH=/usr/bin:/bin",
        "OPENROUTER_API_KEY=user-own-key",
    ] {
        assert!(env.lines().any(|line| line == kept), "missing {kept}");
    }
    for secret in [
        "clovy-dev-token",
        "june-dev-token",
        "clovy-client",
        "127.0.0.1:8787",
        "clovy-oauth-secret",
    ] {
        assert!(!env.contains(secret), "{secret} reached the CLI");
    }
}

#[tokio::test]
async fn chat_engine_cancel_during_launch_preparation_spawns_nothing() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    // Any attempt to start this program fails, which would end the run as
    // `failed` ("Could not start …") instead of `cancelled`.
    let program = harness.bin.path().join("claude-must-not-start");
    let host = ChatEngineHost::default();

    // The cancel arrives after the run is registered but before the turn
    // reaches its launch (while the message is saved or the CLI resolved).
    let run_id = harness.new_run(kind, "Never run").await;
    let ticket = host.register(&run_id).unwrap();
    assert!(host.cancel(&run_id), "the registered run is cancellable");
    let sink = TestSink {
        repository: harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    };
    let TurnTicket {
        cancel,
        _done: _completion,
    } = ticket;
    execute_turn(
        &harness.repository,
        &sink,
        harness.context(kind, &program, &run_id, "Never run", None),
        cancel,
    )
    .await;
    let emitted = sink.emitted.into_inner().unwrap();
    assert!(
        !emitted.iter().any(|(method, _)| method == "run.failed"),
        "the turn tried to start the CLI after the cancel: {emitted:?}"
    );
    assert_eq!(
        emitted.last().map(|(method, _)| method.clone()),
        Some("run.cancelled".to_string())
    );
    assert_eq!(
        harness.repository.get_run(&run_id).await.unwrap().status,
        "cancelled"
    );

    // A run already settled by someone else is not started either.
    let settled = harness.new_run(kind, "Settled").await;
    harness
        .repository
        .update_run_status(&settled, "cancelled", None, None, None)
        .await
        .unwrap();
    let (_cancel, receiver) = watch::channel(false);
    execute_turn(
        &harness.repository,
        &TestSink {
            repository: harness.repository.clone(),
            emitted: StdMutex::new(Vec::new()),
        },
        harness.context(kind, &program, &settled, "Settled", None),
        receiver,
    )
    .await;
    // Nothing was attempted: the settled run keeps its status.
    assert_eq!(
        harness.repository.get_run(&settled).await.unwrap().status,
        "cancelled"
    );
}

#[tokio::test]
async fn remote_cancellation_after_started_during_preparation_publishes_only_target_terminal() {
    struct PreparingSink {
        inner: TestSink,
        started: tokio::sync::Notify,
        resume: tokio::sync::Notify,
        session_id: String,
        run_id: String,
    }
    impl FrameSink for PreparingSink {
        async fn publish(&self, frame: RpcFrame) {
            assert_eq!(frame.session_id, self.session_id);
            assert_eq!(frame.run_id, self.run_id);
            let preparing = frame.method.as_deref() == Some("run.started");
            self.inner.publish(frame).await;
            if preparing {
                self.started.notify_one();
                self.resume.notified().await;
            }
        }
    }
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let caller = harness
        .repository
        .create_session("Caller", "test", AgentSafetyMode::Sandboxed, None)
        .await
        .unwrap();
    let caller_run = harness
        .repository
        .create_run(&caller.id, "test", None)
        .await
        .unwrap();
    let run_id = harness.new_run(kind, "Do not spawn").await;
    let host = ChatEngineHost::default();
    let TurnTicket {
        cancel,
        _done: _completion,
    } = host.register(&run_id).unwrap();
    let sink = PreparingSink {
        inner: TestSink {
            repository: harness.repository.clone(),
            emitted: StdMutex::new(Vec::new()),
        },
        started: tokio::sync::Notify::new(),
        resume: tokio::sync::Notify::new(),
        session_id: harness.session_id.clone(),
        run_id: run_id.clone(),
    };
    let program = harness.bin.path().join("must-not-spawn");
    let context = harness.context(kind, &program, &run_id, "Do not spawn", None);
    let terminal_owned = std::sync::atomic::AtomicBool::new(false);
    let remote_cancel = async {
        sink.started.notified().await;
        let (changed, host_owned_terminal) =
            crate::agent_sessions::api::claim_conversation_run_cancellation_with_signal(
                &harness.repository.pool,
                "default",
                &caller.id,
                &caller_run.id,
                &harness.session_id,
                &run_id,
                &crate::domain::turn_authority::TurnAuthority::new(),
                || {
                    // The only connection is still the claim's writer. Release
                    // real preparation now, before commit can expose cancelled.
                    assert!(harness.repository.pool.try_acquire().is_none());
                    let owned = host.cancel(&run_id);
                    sink.resume.notify_one();
                    owned
                },
            )
            .await
            .unwrap();
        assert!(changed);
        assert!(host_owned_terminal);
        terminal_owned.store(host_owned_terminal, std::sync::atomic::Ordering::SeqCst);
    };
    tokio::join!(
        execute_turn(&harness.repository, &sink, context, cancel),
        remote_cancel
    );
    // Cleanup can run after the task has already emitted and dropped its
    // receiver. Its false result must not steal the original event ownership.
    assert!(!host.cancel(&run_id));
    assert!(terminal_owned.load(std::sync::atomic::Ordering::SeqCst));
    let emitted = sink.inner.emitted.into_inner().unwrap();
    assert_eq!(
        emitted
            .iter()
            .filter(|(method, _)| method == "run.cancelled")
            .count(),
        1
    );
    assert!(!emitted.iter().any(|(method, _)| method == "run.failed"));
    assert_eq!(
        emitted.last().map(|(method, _)| method.as_str()),
        Some("run.cancelled")
    );
    assert_eq!(
        harness.repository.get_run(&run_id).await.unwrap().status,
        "cancelled"
    );
    assert_eq!(
        harness
            .repository
            .get_run(&caller_run.id)
            .await
            .unwrap()
            .status,
        "running"
    );
}

/// Real CLIs, two turns each in a temporary directory. Run apart with
/// `cargo test chat_engine_real -- --ignored --nocapture`; they use the
/// user's own CLI login and quota.
mod real {
    use super::*;

    async fn two_turns_on_real_cli(kind: CliKind) {
        let env = crate::llm::shell_env::login_env().await;
        let Some(program) = env.which(kind.id()) else {
            panic!("{} is not installed", kind.id());
        };
        let harness = Harness::new(kind).await;
        let turn = |input: &'static str| {
            let harness = &harness;
            let program = program.clone();
            let env = env.vars().clone();
            async move {
                let run_id = harness.new_run(kind, input).await;
                let sink = TestSink {
                    repository: harness.repository.clone(),
                    emitted: StdMutex::new(Vec::new()),
                };
                let (_cancel, receiver) = watch::channel(false);
                let mut context = harness.context(kind, &program, &run_id, input, None);
                context.env = env;
                execute_turn(&harness.repository, &sink, context, receiver).await;
                let emitted = sink.emitted.into_inner().unwrap();
                println!("{kind:?} turn {input:?}: {:?}", emitted.last());
                run_id
            }
        };
        let first = turn("Remember the code word PELICAN-42. Reply with only: stored").await;
        let second = turn("What was the code word? Reply with only the code word.").await;
        let transcript = harness.transcript().await;
        for (kind, text) in &transcript {
            println!("  {kind}: {}", text.chars().take(200).collect::<String>());
        }
        for run in [first, second] {
            assert_eq!(
                harness.repository.get_run(&run).await.unwrap().status,
                "completed"
            );
        }
        let answer = transcript
            .iter()
            .rev()
            .find(|(kind, _)| kind == "assistant_message")
            .map(|(_, text)| text.clone())
            .unwrap_or_default();
        assert!(
            answer.contains("PELICAN-42"),
            "{kind:?} lost the conversation: {answer}"
        );
    }

    #[tokio::test]
    #[ignore = "runs the installed claude CLI"]
    async fn chat_engine_real_claude_two_turns() {
        two_turns_on_real_cli(CliKind::Claude).await;
    }

    #[tokio::test]
    #[ignore = "runs the installed codex CLI"]
    async fn chat_engine_real_codex_two_turns() {
        two_turns_on_real_cli(CliKind::Codex).await;
    }

    #[tokio::test]
    #[ignore = "runs the installed pi CLI"]
    async fn chat_engine_real_pi_two_turns() {
        two_turns_on_real_cli(CliKind::Pi).await;
    }
}

/// Home (ADR-0064): an old Home session (`open-software/auto`, with the turns
/// imported from local storage) answers on the default CLI. The first turn
/// gets the imported conversation as earlier messages, the second resumes the
/// CLI conversation, and Clovy API is never called.
#[tokio::test]
#[cfg(unix)]
async fn chat_engine_home_session_runs_two_turns_on_the_cli_without_clovy_api() {
    let api = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api_address = api.local_addr().unwrap();
    let api_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = api_calls.clone();
    let api_server = tokio::spawn(async move {
        while let Ok((_stream, _)) = api.accept().await {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    // Inject the historical origin at its original environment boundary. A
    // reintroduced server resolver would hit this listener; current CLI code
    // never reads it. Assemble the retired name only inside this regression.
    let retired_origin_env = ["CLOVY", "API", "URL"].join("_");
    let previous_origin = std::env::var_os(&retired_origin_env);
    std::env::set_var(&retired_origin_env, format!("http://{api_address}"));

    let kind = CliKind::Claude;
    let mut harness = Harness::new(kind).await;
    let home = harness
        .repository
        .create_session(
            "Home",
            "open-software/auto",
            AgentSafetyMode::Sandboxed,
            harness.workspace.path().to_str(),
        )
        .await
        .unwrap();
    harness.session_id = home.id.clone();
    let imported = harness
        .repository
        .import_legacy_home_turns(
            &home.id,
            &[
                json!({
                    "id": "home:direct:user:1",
                    "role": "user",
                    "createdAt": "2026-10-01T09:00:00.000Z",
                    "parts": [{ "type": "text", "text": "My name is Gui" }],
                }),
                json!({
                    "id": "home:direct:assistant:1",
                    "role": "assistant",
                    "createdAt": "2026-10-01T09:00:02.000Z",
                    "parts": [{ "type": "text", "text": "Nice to meet you, Gui." }],
                }),
            ],
        )
        .await
        .unwrap();
    assert_eq!(imported.imported, 2);
    // What start_agent_run does with the Home turn's model.
    sqlx::query::query("UPDATE agent_sessions SET model = ? WHERE id = ?")
        .bind(engine_model_id(kind))
        .bind(&home.id)
        .execute(&harness.repository.pool)
        .await
        .unwrap();
    let program = harness.fake_cli(kind, FAKE_CLAUDE);

    let (_, first) = harness.turn(kind, &program, "eae", None).await;
    let (_, second) = harness.turn(kind, &program, "What is my name?", None).await;

    for events in [&first, &second] {
        assert_eq!(
            events.last().map(|(method, _)| method.as_str()),
            Some("run.completed"),
            "{events:?}"
        );
    }
    let first_prompt = harness.stdin(1);
    assert!(
        first_prompt.contains("User: My name is Gui"),
        "{first_prompt}"
    );
    assert!(
        first_prompt.contains("Assistant: Nice to meet you, Gui."),
        "{first_prompt}"
    );
    assert!(first_prompt.ends_with("eae"), "{first_prompt}");
    assert!(!harness.args(1).iter().any(|arg| arg == "--resume"));
    assert_eq!(
        argument_after(&harness.args(2), "--resume"),
        Some("claude-conv-1")
    );
    assert_eq!(harness.stdin(2).trim(), "What is my name?");
    let transcript = harness.transcript().await;
    let messages = transcript
        .iter()
        .filter(|(kind, _)| kind == "user_message" || kind == "assistant_message")
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        vec![
            "My name is Gui",
            "Nice to meet you, Gui.",
            "eae",
            "Answer 1",
            "What is my name?",
            "Answer 2",
        ]
    );
    assert_eq!(api_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    api_server.abort();
    match previous_origin {
        Some(value) => std::env::set_var(retired_origin_env, value),
        None => std::env::remove_var(retired_origin_env),
    }
}

#[tokio::test]
#[cfg(unix)]
async fn remote_stop_after_cli_exit_publishes_only_the_persisted_terminal_winner() {
    struct MessageCompletionSink {
        inner: TestSink,
        reached: tokio::sync::Notify,
        resume: tokio::sync::Notify,
        session_id: String,
        run_id: String,
    }
    impl FrameSink for MessageCompletionSink {
        async fn publish(&self, frame: RpcFrame) {
            assert_eq!(frame.session_id, self.session_id);
            assert_eq!(frame.run_id, self.run_id);
            if frame.method.as_deref() == Some("message.completed") {
                self.reached.notify_one();
                self.resume.notified().await;
            }
            self.inner.publish(frame).await;
        }
    }

    // Separate controls prove both the ordinary completion and the already
    // persisted completion that wins before a later remote Stop claim.
    for (stop_before_terminal, stop_after_terminal) in
        [(true, false), (false, false), (false, true)]
    {
        let kind = CliKind::Claude;
        let harness = Harness::new(kind).await;
        let program = harness.fake_cli(
            kind,
            r#"
echo $$ > "$LOG/cli.pid"
echo '{"type":"system","subtype":"init","session_id":"terminal-winner-fixture"}'
echo '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Finished locally"}}}'
echo '{"type":"result","subtype":"success","is_error":false,"result":"Finished locally"}'
exit 0
"#,
        );
        let run_id = harness.new_run(kind, "Finish before settlement").await;
        let caller_session = harness
            .repository
            .create_session("Caller", "test", AgentSafetyMode::Sandboxed, None)
            .await
            .unwrap();
        let caller_run = harness
            .repository
            .create_run(&caller_session.id, "test", None)
            .await
            .unwrap();
        let mut caller_context =
            harness.context(kind, &program, &caller_run.id, "Remote Stop", None);
        caller_context.session_id = caller_session.id.clone();
        let turns = crate::mcp_server::turns::TurnRegistry::default();
        let caller_token = turns.issue(caller_context.binding());
        let caller_binding = turns.binding(&caller_token).unwrap();
        let host = ChatEngineHost::default();
        let TurnTicket {
            cancel,
            _done: _completion,
        } = host.register(&run_id).unwrap();
        let sink = MessageCompletionSink {
            inner: TestSink {
                repository: harness.repository.clone(),
                emitted: StdMutex::new(Vec::new()),
            },
            reached: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
            session_id: harness.session_id.clone(),
            run_id: run_id.clone(),
        };
        let context = harness.context(kind, &program, &run_id, "Finish before settlement", None);
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            let coordinate_stop = async {
                sink.reached.notified().await;
                // This is the real post-exit message-publication boundary,
                // not a timer or a simulated terminal-frame-only test.
                assert_process_gone(read_pid(&harness.log.path().join("cli.pid"))).await;
                if stop_before_terminal {
                    let (changed, host_owned_terminal) =
                        crate::agent_sessions::api::claim_conversation_run_cancellation_for_host(
                            &harness.repository.pool,
                            "default",
                            &caller_binding.session_id,
                            &caller_binding.run_id,
                            &harness.session_id,
                            &run_id,
                            &caller_binding.authority,
                            &host,
                            Some(&turns),
                        )
                        .await
                        .unwrap();
                    assert!(changed);
                    assert!(host_owned_terminal);
                    assert_eq!(
                        harness.repository.get_run(&run_id).await.unwrap().status,
                        "cancelled"
                    );
                }
                sink.resume.notify_one();
            };
            tokio::join!(
                execute_turn(&harness.repository, &sink, context, cancel),
                coordinate_stop
            );
            if stop_after_terminal {
                let (changed, host_owned_terminal) =
                    crate::agent_sessions::api::claim_conversation_run_cancellation_for_host(
                        &harness.repository.pool,
                        "default",
                        &caller_binding.session_id,
                        &caller_binding.run_id,
                        &harness.session_id,
                        &run_id,
                        &caller_binding.authority,
                        &host,
                        Some(&turns),
                    )
                    .await
                    .unwrap();
                assert!(!changed, "persisted completion already won");
                assert!(!host_owned_terminal);
            }
            let emitted = sink.inner.emitted.lock().unwrap().clone();
            let terminal = emitted
                .iter()
                .filter(|(method, _)| {
                    matches!(
                        method.as_str(),
                        "run.cancelled" | "run.completed" | "run.failed"
                    )
                })
                .collect::<Vec<_>>();
            let expected_status = if stop_before_terminal {
                "cancelled"
            } else {
                "completed"
            };
            let expected_event = if stop_before_terminal {
                "run.cancelled"
            } else {
                "run.completed"
            };
            assert_eq!(terminal.len(), 1, "terminal events: {terminal:?}");
            assert_eq!(terminal[0].0, expected_event);
            assert_eq!(
                emitted.last().map(|(method, _)| method.as_str()),
                Some(expected_event)
            );
            assert_eq!(
                harness.repository.get_run(&run_id).await.unwrap().status,
                expected_status
            );
            assert!(turns.contains_binding(&caller_binding));
            caller_binding.authority.check_live().unwrap();
            assert_ne!(
                harness
                    .repository
                    .get_run(&caller_run.id)
                    .await
                    .unwrap()
                    .status,
                "cancelled",
                "remote Stop must leave the caller run intact"
            );
        })
        .await
        .expect("post-exit terminal race and controls finish within their whole-scenario timeout");
    }
}

#[tokio::test]
#[cfg(unix)]
async fn reassignment_during_prompt_preparation_refuses_cli_launch() {
    struct ReassigningSink(TestSink);
    impl FrameSink for ReassigningSink {
        async fn publish(&self, frame: RpcFrame) {
            let started = frame.method.as_deref() == Some("run.started");
            let session_id = frame.session_id.clone();
            self.0.publish(frame).await;
            if started {
                crate::db::repositories::Repositories::new(self.0.repository.pool.clone())
                    .assign_session_to_profile(&session_id, "B")
                    .await
                    .unwrap();
            }
        }
    }
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let run_id = harness.new_run(kind, "Synthetic message").await;
    let program = harness.fake_cli(kind, "echo must-not-launch");
    let sink = ReassigningSink(TestSink {
        repository: harness.repository.clone(),
        emitted: StdMutex::new(Vec::new()),
    });
    let (_cancel, receiver) = watch::channel(false);
    execute_turn(
        &harness.repository,
        &sink,
        harness.context(kind, &program, &run_id, "Synthetic message", None),
        receiver,
    )
    .await;
    assert!(!harness.log.path().join("count").exists());
    let events = sink.0.emitted.into_inner().unwrap();
    assert!(events.iter().any(|(method, value)| method == "run.failed"
        && value.to_string().contains("data partition changed")));
}
