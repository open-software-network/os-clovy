//! Copilot CLI: `~/.copilot/session-state/<session-uuid>/events.jsonl`, an
//! append-only event log. `user.message` `data.content` is what the user
//! typed (`transformedContent` adds Copilot's own framing and is ignored);
//! `session.shutdown` / `session.end` mark an exit.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    for_each_json_line, is_regular_file, list_subdirs, source_root, str_at, under, SourceRoots,
};
use crate::activity::store::parse_time;
use crate::coding_agents::record::{
    cap, Record, RecordKind, RecordSink, SessionInfo, TOOL_INPUT_CAP,
};

pub fn root(roots: &SourceRoots) -> PathBuf {
    roots.home.join(".copilot").join("session-state")
}

pub fn discover(roots: &SourceRoots) -> Vec<(PathBuf, PathBuf)> {
    let Some(root) = source_root(&root(roots)) else {
        return Vec::new();
    };
    let files = list_subdirs(&root)
        .into_iter()
        .map(|dir| dir.join("events.jsonl"))
        .filter(|file| is_regular_file(file))
        .collect();
    under(&root, files)
}

pub fn read(path: &Path, sink: &mut RecordSink<'_>) -> io::Result<SessionInfo> {
    for_each_json_line(path, |value| sink(record(value)))?;
    Ok(SessionInfo {
        id: path
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        title: None,
    })
}

fn record(value: &Value) -> Record {
    let at = str_at(value, &["timestamp"]).and_then(parse_time);
    let text = str_at(value, &["data", "content"])
        .unwrap_or_default()
        .trim();
    match str_at(value, &["type"]) {
        Some("session.start") => Record::event(at)
            .with_cwd(str_at(value, &["data", "context", "cwd"]).map(str::to_string)),
        Some("user.message") if !text.is_empty() => Record::new(at, RecordKind::Prompt, text),
        Some("assistant.message") if !text.is_empty() => Record::new(at, RecordKind::Reply, text),
        Some("tool.execution_start") => {
            let name = str_at(value, &["data", "toolName"]).unwrap_or("?");
            let arguments = value["data"]
                .get("arguments")
                .map(|args| cap(&args.to_string(), TOOL_INPUT_CAP))
                .unwrap_or_default();
            Record::new(
                at,
                RecordKind::Tool,
                &format!("[tool_use: {name} {arguments}]"),
            )
        }
        Some("session.shutdown" | "session.end") => Record::new(at, RecordKind::End, ""),
        _ => Record::event(at),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{collect, write_lines};
    use super::*;
    use serde_json::json;

    #[test]
    fn uses_typed_content_and_marks_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lines(
            dir.path(),
            "u1/events.jsonl",
            &[
                json!({"type": "session.start", "timestamp": "2026-10-04T06:14:06.353Z",
                       "data": {"sessionId": "u1", "context": {"cwd": "/Users/dev/repo"}}}),
                json!({"type": "user.message", "timestamp": "2026-10-04T06:14:15.323Z",
                       "data": {"content": "what is an editorial?",
                                "transformedContent": "<current_datetime>x</current_datetime> what is an editorial?"}}),
                json!({"type": "assistant.turn_start", "timestamp": "2026-10-04T06:14:15.500Z", "data": {}}),
                json!({"type": "assistant.message", "timestamp": "2026-10-04T06:14:20Z",
                       "data": {"content": "An opinion piece."}}),
                json!({"type": "session.shutdown", "timestamp": "2026-10-04T06:15:00Z", "data": {}}),
            ],
        );
        let (info, records) = collect(|sink| read(&path, sink));
        assert_eq!(info.id, "u1");
        assert_eq!(records[0].cwd.as_deref(), Some("/Users/dev/repo"));
        assert_eq!(records[1].kind, RecordKind::Prompt);
        assert_eq!(records[1].body, "what is an editorial?");
        assert_eq!(records[3].kind, RecordKind::Reply);
        assert_eq!(records[4].kind, RecordKind::End);
    }
}
