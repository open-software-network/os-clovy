//! Codex CLI and app: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`. The
//! working directory comes from `session_meta` / `turn_context`. Turns are
//! read from either generation of the rollout format:
//!
//! - current: `event_msg` `item_completed` with `item.type` `UserMessage` /
//!   `AgentMessage` (tool items such as `CommandExecution` become tool lines);
//! - earlier: `event_msg` `user_message` / `agent_message`.
//!
//! `response_item` messages are skipped: they include the injected
//! instructions and environment context, not what the user typed. A first
//! cheap pass (raw bytes, no JSON parsing) tells whether the file has
//! current-format turns; if it does, earlier-format turns are ignored so a
//! transitional file never counts a prompt twice.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    file_stem, for_each_json_line, for_each_line, is_regular_file, list_dir, list_subdirs,
    source_root, str_at, under, SourceRoots,
};
use crate::activity::store::parse_time;
use crate::coding_agents::record::{
    cap, Record, RecordKind, RecordSink, SessionInfo, TOOL_INPUT_CAP,
};

pub fn root(roots: &SourceRoots) -> PathBuf {
    roots.home.join(".codex").join("sessions")
}

pub fn discover(roots: &SourceRoots) -> Vec<(PathBuf, PathBuf)> {
    let Some(root) = source_root(&root(roots)) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for year in list_subdirs(&root) {
        for month in list_subdirs(&year) {
            for day in list_subdirs(&month) {
                files.extend(list_dir(&day).into_iter().filter(|file| {
                    file.extension().is_some_and(|ext| ext == "jsonl") && is_regular_file(file)
                }));
            }
        }
    }
    under(&root, files)
}

fn has_current_format(path: &Path) -> io::Result<bool> {
    let contains = |haystack: &[u8], needle: &[u8]| {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    };
    let mut found = false;
    for_each_line(path, |line| {
        found = contains(line, b"\"item_completed\"")
            && (contains(line, b"\"UserMessage\"") || contains(line, b"\"AgentMessage\""));
        !found
    })?;
    Ok(found)
}

pub fn read(path: &Path, sink: &mut RecordSink<'_>) -> io::Result<SessionInfo> {
    let current_format = has_current_format(path)?;
    let mut id = None;
    for_each_json_line(path, |value| {
        if id.is_none() && str_at(value, &["type"]) == Some("session_meta") {
            id = str_at(value, &["payload", "id"]).map(str::to_string);
        }
        sink(record(value, current_format))
    })?;
    Ok(SessionInfo {
        id: id.unwrap_or_else(|| file_stem(path)),
        title: None,
    })
}

fn record(value: &Value, current_format: bool) -> Record {
    let at = str_at(value, &["timestamp"]).and_then(parse_time);
    let cwd = match str_at(value, &["type"]) {
        Some("session_meta" | "turn_context") => {
            str_at(value, &["payload", "cwd"]).map(str::to_string)
        }
        _ => None,
    };
    let event = || Record::event(at).with_cwd(cwd.clone());
    if str_at(value, &["type"]) != Some("event_msg") {
        return event();
    }
    let payload = &value["payload"];
    let (kind, body) = match str_at(payload, &["type"]) {
        Some("item_completed") => match item(&payload["item"]) {
            Some(turn) => turn,
            None => return event(),
        },
        Some("user_message") if !current_format => {
            (RecordKind::Prompt, message_text(&payload["message"]))
        }
        Some("agent_message") if !current_format => {
            (RecordKind::Reply, message_text(&payload["message"]))
        }
        _ => return event(),
    };
    if body.trim().is_empty() {
        return event();
    }
    Record::new(at, kind, &body).with_cwd(cwd)
}

fn item(item: &Value) -> Option<(RecordKind, String)> {
    match str_at(item, &["type"])? {
        "UserMessage" => Some((RecordKind::Prompt, content_text(&item["content"]))),
        "AgentMessage" => Some((RecordKind::Reply, content_text(&item["content"]))),
        "Reasoning" => None,
        other => {
            let name = ["tool", "name", "command", "query"]
                .iter()
                .find_map(|key| match item.get(key) {
                    Some(Value::String(text)) => Some(text.clone()),
                    Some(Value::Array(parts)) => Some(
                        parts
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" "),
                    ),
                    _ => None,
                })
                .unwrap_or_default();
            Some((
                RecordKind::Tool,
                format!("[tool: {other} {}]", cap(&name, TOOL_INPUT_CAP)),
            ))
        }
    }
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| str_at(part, &["text"]))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn message_text(message: &Value) -> String {
    match message {
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| match part {
                Value::String(text) => Some(text.as_str()),
                other => str_at(other, &["text"]).or_else(|| str_at(other, &["content"])),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        other => content_text(other),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{collect, write_lines};
    use super::*;
    use serde_json::json;

    /// A resumed session keeps writing to the rollout under the day it
    /// started: day directories before the window are still listed, and the
    /// file's mtime decides.
    #[test]
    fn a_resumed_session_under_an_old_day_directory_is_discovered() {
        use std::time::{Duration, SystemTime};
        let home = tempfile::tempdir().unwrap();
        let day = home.path().join(".codex/sessions/2026/10/05");
        std::fs::create_dir_all(&day).unwrap();
        let path = write_lines(
            &day,
            "rollout-old.jsonl",
            &[json!({"type": "session_meta"})],
        );
        let roots = SourceRoots::from_home(home.path());
        let since = SystemTime::now() - Duration::from_secs(3600);
        let found = super::super::discover(crate::coding_agents::SourceId::Codex, &roots, since);
        assert_eq!(found.len(), 1);
        assert!(found[0].path.ends_with("2026/10/05/rollout-old.jsonl"));
        assert!(path.exists());
    }

    #[test]
    fn current_format_turns_and_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lines(
            dir.path(),
            "rollout-x.jsonl",
            &[
                json!({"timestamp": "2026-10-04T10:00:00Z", "type": "session_meta",
                       "payload": {"id": "abc", "cwd": "/repo"}}),
                json!({"timestamp": "2026-10-04T10:00:01Z", "type": "response_item",
                       "payload": {"type": "message", "role": "user",
                                   "content": [{"type": "input_text", "text": "<environment_context>"}]}}),
                json!({"timestamp": "2026-10-04T10:00:01Z", "type": "event_msg",
                       "payload": {"type": "user_message", "message": "duplicate"}}),
                json!({"timestamp": "2026-10-04T10:00:02Z", "type": "event_msg",
                       "payload": {"type": "item_completed",
                                   "item": {"type": "UserMessage", "content": [{"type": "text", "text": "add tests"}]}}}),
                json!({"timestamp": "2026-10-04T10:00:03Z", "type": "event_msg",
                       "payload": {"type": "item_completed",
                                   "item": {"type": "CommandExecution", "command": "cargo test"}}}),
                json!({"timestamp": "2026-10-04T10:00:04Z", "type": "event_msg",
                       "payload": {"type": "item_completed",
                                   "item": {"type": "AgentMessage", "content": [{"type": "Text", "text": "Done."}]}}}),
            ],
        );
        let (info, records) = collect(|sink| read(&path, sink));
        assert_eq!(info.id, "abc");
        let turns: Vec<(RecordKind, &str)> = records
            .iter()
            .filter(|r| r.is_turn())
            .map(|r| (r.kind, r.body.as_str()))
            .collect();
        assert_eq!(
            turns,
            vec![
                (RecordKind::Prompt, "add tests"),
                (RecordKind::Tool, "[tool: CommandExecution cargo test]"),
                (RecordKind::Reply, "Done."),
            ],
            "the earlier-format duplicate before the first item is ignored too"
        );
        assert_eq!(records[0].cwd.as_deref(), Some("/repo"));
    }

    #[test]
    fn earlier_format_turns() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lines(
            dir.path(),
            "rollout-y.jsonl",
            &[
                json!({"timestamp": "2026-10-04T10:00:00Z", "type": "turn_context",
                       "payload": {"cwd": "/repo"}}),
                json!({"timestamp": "2026-10-04T10:00:02Z", "type": "event_msg",
                       "payload": {"type": "user_message", "message": "explain"}}),
                json!({"timestamp": "2026-10-04T10:00:04Z", "type": "event_msg",
                       "payload": {"type": "agent_message", "message": "It works."}}),
            ],
        );
        let (info, records) = collect(|sink| read(&path, sink));
        assert_eq!(info.id, "rollout-y");
        let kinds: Vec<RecordKind> = records.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![RecordKind::Event, RecordKind::Prompt, RecordKind::Reply]
        );
    }
}
