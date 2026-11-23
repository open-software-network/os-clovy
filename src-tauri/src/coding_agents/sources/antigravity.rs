//! Antigravity (CLI `agy` and the editor): each conversation keeps a JSONL
//! transcript at `~/.gemini/antigravity-cli/brain/<conversation>/
//! .system_generated/logs/transcript.jsonl` (the editor uses
//! `~/.gemini/antigravity/brain/`). Steps carry `created_at` (ISO), `source`
//! (`USER_EXPLICIT`, `MODEL`, `SYSTEM`), `type`, and `content`. A user step's
//! text is wrapped in `<USER_REQUEST>`, followed by `<ADDITIONAL_METADATA>`.
//! The transcript keeps no working directory.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    for_each_json_line, is_real_dir, is_regular_file, list_subdirs, source_root, str_at,
    SourceRoots,
};
use crate::activity::store::parse_time;
use crate::coding_agents::record::{
    cap, inner_tag, Record, RecordKind, RecordSink, SessionInfo, TOOL_TEXT_CAP,
};

pub fn cli_root(roots: &SourceRoots) -> PathBuf {
    roots.home.join(".gemini").join("antigravity-cli")
}

pub fn ide_root(roots: &SourceRoots) -> PathBuf {
    roots.home.join(".gemini").join("antigravity")
}

/// Each install location is its own root.
pub fn discover(roots: &SourceRoots) -> Vec<(PathBuf, PathBuf)> {
    let mut found = Vec::new();
    for location in [cli_root(roots), ide_root(roots)] {
        let Some(root) = source_root(&location) else {
            continue;
        };
        let brain = root.join("brain");
        if !is_real_dir(&brain) {
            continue;
        }
        for conversation in list_subdirs(&brain) {
            let generated = conversation.join(".system_generated");
            let logs = generated.join("logs");
            if !is_real_dir(&generated) || !is_real_dir(&logs) {
                continue;
            }
            let file = logs.join("transcript.jsonl");
            if is_regular_file(&file) {
                found.push((root.clone(), file));
            }
        }
    }
    found
}

pub fn read(path: &Path, sink: &mut RecordSink<'_>) -> io::Result<SessionInfo> {
    for_each_json_line(path, |value| sink(record(value)))?;
    Ok(SessionInfo {
        id: path
            .ancestors()
            .nth(3)
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        title: None,
    })
}

fn record(value: &Value) -> Record {
    let at = str_at(value, &["created_at"]).and_then(parse_time);
    let content = str_at(value, &["content"]).unwrap_or_default().trim();
    match (str_at(value, &["source"]), str_at(value, &["type"])) {
        (Some("USER_EXPLICIT"), Some("USER_INPUT")) => {
            let prompt = inner_tag(content, "USER_REQUEST").unwrap_or(content);
            if prompt.is_empty() {
                Record::event(at)
            } else {
                Record::new(at, RecordKind::Prompt, prompt)
            }
        }
        (Some("MODEL"), Some("PLANNER_RESPONSE")) => {
            let mut parts: Vec<String> = value
                .get("tool_calls")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|call| str_at(call, &["name"]))
                .map(|name| format!("[tool_use: {name}]"))
                .collect();
            if !content.is_empty() {
                parts.push(content.to_string());
            }
            if parts.is_empty() {
                Record::event(at)
            } else {
                Record::new(at, RecordKind::Reply, &parts.join("\n"))
            }
        }
        (Some("MODEL"), Some(step)) if !content.is_empty() => Record::new(
            at,
            RecordKind::Tool,
            &format!("[{step}: {}]", cap(content, TOOL_TEXT_CAP)),
        ),
        _ => Record::event(at),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{collect, write_lines};
    use super::*;
    use serde_json::json;

    #[test]
    fn unwraps_user_requests_and_classifies_steps() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lines(
            dir.path(),
            "brain/conv/.system_generated/logs/transcript.jsonl",
            &[
                json!({"step_index": 0, "source": "USER_EXPLICIT", "type": "USER_INPUT",
                       "created_at": "2026-10-04T12:27:18Z",
                       "content": "<USER_REQUEST>\nAdd a health check\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\nlocal time\n</ADDITIONAL_METADATA>"}),
                json!({"step_index": 1, "source": "SYSTEM", "type": "CONVERSATION_HISTORY"}),
                json!({"step_index": 2, "source": "MODEL", "type": "PLANNER_RESPONSE",
                       "created_at": "2026-10-04T12:27:20Z", "tool_calls": [{"name": "run_command"}]}),
                json!({"step_index": 3, "source": "MODEL", "type": "RUN_COMMAND",
                       "created_at": "2026-10-04T12:27:25Z", "content": "ok"}),
                json!({"step_index": 4, "source": "MODEL", "type": "PLANNER_RESPONSE",
                       "created_at": "2026-10-04T12:27:30Z", "content": "Added /health."}),
            ],
        );
        let (info, records) = collect(|sink| read(&path, sink));
        assert_eq!(info.id, "conv");
        let turns: Vec<(RecordKind, &str)> =
            records.iter().map(|r| (r.kind, r.body.as_str())).collect();
        assert_eq!(
            turns,
            vec![
                (RecordKind::Prompt, "Add a health check"),
                (RecordKind::Event, ""),
                (RecordKind::Reply, "[tool_use: run_command]"),
                (RecordKind::Tool, "[RUN_COMMAND: ok]"),
                (RecordKind::Reply, "Added /health."),
            ]
        );
    }
}
