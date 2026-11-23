//! The `coding-agent-ingest` spec scenarios, end to end over a temporary
//! home directory and a real (encrypted) activity database.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::ingest::Scanner;
use super::settings::CodingAgentSources;
use super::sources::SourceRoots;
use super::store::{BlockState, CodingAgentBlock, PendingSummary};
use super::SourceId;
use crate::activity::key::MemoryKeyStore;
use crate::activity::store::{timestamp, ActivityStore, ACTIVITY_DB_FILE};
use crate::interface_locale::UiLocale;
use crate::llm::cli::{compose_prompt, CliKind};

fn at(minute: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 4, 10, 0, 0).unwrap() + Duration::minutes(minute)
}

fn window() -> DateTime<Utc> {
    at(-24 * 60)
}

fn now() -> DateTime<Utc> {
    at(5 * 60)
}

struct Home {
    dir: tempfile::TempDir,
    store: ActivityStore,
    _db: tempfile::TempDir,
}

impl Home {
    async fn new() -> Self {
        let db = tempfile::tempdir().unwrap();
        let store = ActivityStore::open(
            &db.path().join(ACTIVITY_DB_FILE),
            &MemoryKeyStore::default(),
        )
        .await
        .unwrap();
        Self {
            dir: tempfile::tempdir().unwrap(),
            store,
            _db: db,
        }
    }

    fn roots(&self) -> SourceRoots {
        SourceRoots::from_home(self.dir.path())
    }

    fn write_jsonl(&self, relative: &str, lines: &[serde_json::Value]) -> PathBuf {
        let path = self.dir.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(&path, body).unwrap();
        path
    }

    async fn blocks(&self) -> Vec<CodingAgentBlock> {
        self.store
            .coding_agent_blocks_between(window(), now() + Duration::days(1))
            .await
            .unwrap()
    }
}

fn claude_prompt(minute: i64, text: &str) -> serde_json::Value {
    json!({"type": "user", "timestamp": timestamp(at(minute)), "cwd": "/Users/me/clovy",
           "message": {"role": "user", "content": text}})
}

fn claude_reply(minute: i64, text: &str) -> serde_json::Value {
    json!({"type": "assistant", "timestamp": timestamp(at(minute)),
           "message": {"role": "assistant", "content": [{"type": "text", "text": text}]}})
}

fn codex_session(minute: i64) -> Vec<serde_json::Value> {
    vec![
        json!({"timestamp": timestamp(at(minute)), "type": "session_meta",
               "payload": {"id": "codex-1", "cwd": "/Users/me/api"}}),
        json!({"timestamp": timestamp(at(minute + 1)), "type": "event_msg",
               "payload": {"type": "item_completed",
                           "item": {"type": "UserMessage", "content": [{"type": "text", "text": "add a health check"}]}}}),
        json!({"timestamp": timestamp(at(minute + 2)), "type": "event_msg",
               "payload": {"type": "item_completed",
                           "item": {"type": "AgentMessage", "content": [{"type": "Text", "text": "Added /health."}]}}}),
    ]
}

fn only(sources: &[SourceId]) -> CodingAgentSources {
    let mut enabled = CodingAgentSources::default();
    for source in sources {
        match source {
            SourceId::ClaudeCode => enabled.claude_code = true,
            SourceId::Codex => enabled.codex = true,
            _ => unreachable!("not used here"),
        }
    }
    enabled
}

fn fingerprint(path: &Path) -> (String, std::time::SystemTime, u64) {
    let bytes = fs::read(path).unwrap();
    let metadata = fs::metadata(path).unwrap();
    (
        format!("{:x}", Sha256::digest(&bytes)),
        metadata.modified().unwrap(),
        metadata.len(),
    )
}

fn listing(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = walk(dir);
    entries.sort();
    entries
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .flat_map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                let mut nested = walk(&path);
                nested.push(path);
                nested
            } else {
                vec![path]
            }
        })
        .collect()
}

/// Scenario "Fonte desligada": with Codex off, nothing under
/// `~/.codex/sessions` is read and no Codex block exists.
#[tokio::test]
async fn a_disabled_source_is_never_read() {
    let home = Home::new().await;
    home.write_jsonl(
        ".codex/sessions/2026/10/04/rollout-2026-10-04T10-00-00-codex-1.jsonl",
        &codex_session(0),
    );
    home.write_jsonl(
        ".claude/projects/-Users-me-clovy/claude-1.jsonl",
        &[
            claude_prompt(0, "fix the login bug"),
            claude_reply(1, "Fixed."),
        ],
    );

    let mut scanner = Scanner::new(home.roots());
    let report = scanner
        .scan(&home.store, &only(&[SourceId::ClaudeCode]), now(), window())
        .await
        .unwrap();

    assert_eq!(report.files_read.get(&SourceId::Codex), None);
    assert_eq!(report.files_read.get(&SourceId::ClaudeCode), Some(&1));
    let blocks = home.blocks().await;
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].source, SourceId::ClaudeCode);

    let report = scanner
        .scan(
            &home.store,
            &only(&[SourceId::ClaudeCode, SourceId::Codex]),
            now(),
            window(),
        )
        .await
        .unwrap();
    assert_eq!(report.files_read.get(&SourceId::Codex), Some(&1));
    assert_eq!(
        report.files_read.get(&SourceId::ClaudeCode),
        None,
        "an unchanged transcript is not read again"
    );
    let codex: Vec<_> = home
        .blocks()
        .await
        .into_iter()
        .filter(|block| block.source == SourceId::Codex)
        .collect();
    assert_eq!(codex.len(), 1);
    assert_eq!(codex[0].project.as_deref(), Some("api"));
    assert_eq!(codex[0].first_prompt.as_deref(), Some("add a health check"));
}

/// Scenario "Arquivos intactos": ingestion leaves the transcript's content
/// and modification time unchanged and creates nothing next to it.
#[tokio::test]
async fn ingestion_leaves_the_transcript_untouched() {
    let home = Home::new().await;
    let path = home.write_jsonl(
        ".claude/projects/-Users-me-clovy/claude-1.jsonl",
        &[
            claude_prompt(0, "fix the login bug"),
            claude_reply(1, "Fixed."),
        ],
    );
    let before = fingerprint(&path);
    let files_before = listing(home.dir.path());

    let report = Scanner::new(home.roots())
        .scan(&home.store, &only(&[SourceId::ClaudeCode]), now(), window())
        .await
        .unwrap();

    assert_eq!(report.blocks_written, 1);
    assert_eq!(fingerprint(&path), before);
    assert_eq!(listing(home.dir.path()), files_before);
}

/// Scenario "Conversa longa": 2 h 30 min without pauses becomes three
/// blocks, each cut at a user prompt.
#[tokio::test]
async fn a_long_conversation_becomes_three_blocks_cut_at_prompts() {
    let home = Home::new().await;
    let mut lines = Vec::new();
    for minute in (0..150).step_by(10) {
        lines.push(claude_prompt(minute, &format!("step at {minute}")));
        lines.push(claude_reply(minute + 4, "done"));
        lines.push(json!({"type": "user", "timestamp": timestamp(at(minute + 6)),
                          "message": {"role": "user", "content": [{"type": "tool_result", "content": "ok"}]}}));
    }
    lines.push(claude_reply(150, "all done"));
    home.write_jsonl(".claude/projects/-Users-me-clovy/long.jsonl", &lines);

    Scanner::new(home.roots())
        .scan(&home.store, &only(&[SourceId::ClaudeCode]), now(), window())
        .await
        .unwrap();

    let blocks = home.blocks().await;
    let starts: Vec<String> = blocks
        .iter()
        .map(|block| block.started_at.clone())
        .collect();
    assert_eq!(
        starts,
        vec![
            crate::activity::store::timestamp(at(0)),
            crate::activity::store::timestamp(at(60)),
            crate::activity::store::timestamp(at(120)),
        ]
    );
    let first_prompts: Vec<Option<&str>> = blocks
        .iter()
        .map(|block| block.first_prompt.as_deref())
        .collect();
    assert_eq!(
        first_prompts,
        vec![Some("step at 0"), Some("step at 60"), Some("step at 120")]
    );
    assert_eq!(
        blocks[2].ended_at,
        crate::activity::store::timestamp(at(150))
    );
    assert!(blocks.iter().all(|block| block.state == BlockState::Sealed));
}

/// Scenario "Chamada do Clovy ignorada": the conversation Claude Code
/// records for Clovy's own summary call never becomes a block.
#[tokio::test]
async fn clovys_own_cli_conversation_is_ignored() {
    let home = Home::new().await;
    let pending = PendingSummary {
        block: CodingAgentBlock {
            id: 1,
            source: SourceId::ClaudeCode,
            session_id: "user-session".into(),
            started_at: crate::activity::store::timestamp(at(0)),
            ended_at: crate::activity::store::timestamp(at(30)),
            cwd: Some("/Users/me/clovy".into()),
            project: Some("clovy".into()),
            title: None,
            first_prompt: Some("fix the login bug".into()),
            prompt_count: 1,
            reply_count: 1,
            active_seconds: 60,
            state: BlockState::Sealed,
            sealed_at: None,
            summary: None,
            summary_source: None,
            summary_attempts: 0,
            summary_error: None,
        },
        transcript: "user: fix the login bug".into(),
    };
    let sent = compose_prompt(
        CliKind::Claude,
        &super::summarize::request(&pending, UiLocale::En),
    );
    home.write_jsonl(
        ".claude/projects/-private-tmp-clovy-llm-x/summary-call.jsonl",
        &[
            claude_prompt(40, &sent),
            claude_reply(41, "Fixed the login bug."),
        ],
    );
    home.write_jsonl(
        ".claude/projects/-Users-me-clovy/user-session.jsonl",
        &[
            claude_prompt(0, "fix the login bug"),
            claude_reply(1, "Fixed."),
        ],
    );

    let report = Scanner::new(home.roots())
        .scan(&home.store, &only(&[SourceId::ClaudeCode]), now(), window())
        .await
        .unwrap();

    assert_eq!(report.clovy_calls_skipped, 1);
    let sessions: Vec<String> = home
        .blocks()
        .await
        .into_iter()
        .map(|block| block.session_id)
        .collect();
    assert_eq!(sessions, vec!["user-session".to_string()]);
}
