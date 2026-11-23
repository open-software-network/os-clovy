//! RED for WT-20261005-clovy-s1b-clis-e-endpoints-como-motor-do-chat: the
//! second message of a chat session on a CLI engine resumes the CLI's own
//! conversation (`--resume <id>` for claude) and both answers are persisted
//! in order.

#![cfg(unix)]

use super::tests::{Harness, FAKE_CLAUDE};
use crate::llm::cli::CliKind;

#[tokio::test]
async fn chat_engine_red_second_message_resumes_the_cli_conversation() {
    let kind = CliKind::Claude;
    let harness = Harness::new(kind).await;
    let program = harness.fake_cli(kind, FAKE_CLAUDE);

    let (_, _) = harness.turn(kind, &program, "First question", None).await;
    let (second_run, _) = harness.turn(kind, &program, "Second question", None).await;

    let second_args = harness.args(2);
    let resume = second_args
        .iter()
        .position(|arg| arg == "--resume")
        .and_then(|index| second_args.get(index + 1));
    assert_eq!(resume.map(String::as_str), Some("claude-conv-1"));
    assert_eq!(
        harness
            .repository
            .get_run(&second_run)
            .await
            .unwrap()
            .status,
        "completed"
    );
    let messages = harness
        .transcript()
        .await
        .into_iter()
        .filter(|(kind, _)| kind == "user_message" || kind == "assistant_message")
        .map(|(_, text)| text)
        .collect::<Vec<_>>();
    assert_eq!(
        messages,
        ["First question", "Answer 1", "Second question", "Answer 2"]
    );
}
