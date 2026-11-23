//! Claude Code: `~/.claude/projects/<project>/<session-uuid>.jsonl`, one JSON
//! record per line. Tool results are logged as `type: "user"` records whose
//! content has only `tool_result` blocks; they are not prompts. Sidechain
//! (subagent) and meta records carry only their timestamp.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    file_stem, for_each_json_line, is_regular_file, list_subdirs, source_root, str_at, under,
    SourceRoots,
};
use crate::activity::store::parse_time;
use crate::coding_agents::record::{
    cap, Record, RecordKind, RecordSink, SessionInfo, TOOL_INPUT_CAP, TOOL_TEXT_CAP,
};

pub fn root(roots: &SourceRoots) -> PathBuf {
    roots.home.join(".claude").join("projects")
}

/// Top-level session files of every project (subagent files live one level
/// deeper and are part of their parent session's work).
pub fn discover(roots: &SourceRoots) -> Vec<(PathBuf, PathBuf)> {
    let Some(root) = source_root(&root(roots)) else {
        return Vec::new();
    };
    let files = list_subdirs(&root)
        .into_iter()
        .flat_map(|project| super::list_dir(&project))
        .filter(|file| file.extension().is_some_and(|ext| ext == "jsonl") && is_regular_file(file))
        .collect();
    under(&root, files)
}

/// Titles seen so far; the last of each kind wins.
#[derive(Default)]
struct Titles {
    ai: Option<String>,
    custom: Option<String>,
    summary: Option<String>,
}

pub fn read(path: &Path, sink: &mut RecordSink<'_>) -> io::Result<SessionInfo> {
    let mut titles = Titles::default();
    for_each_json_line(path, |value| {
        let text = |field| {
            str_at(value, &[field])
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        };
        match str_at(value, &["type"]) {
            Some("ai-title") => titles.ai = text("aiTitle").or(titles.ai.take()),
            Some("custom-title") => titles.custom = text("customTitle").or(titles.custom.take()),
            Some("summary") => titles.summary = text("summary").or(titles.summary.take()),
            _ => {}
        }
        sink(record(value))
    })?;
    Ok(SessionInfo {
        id: file_stem(path),
        title: titles.ai.or(titles.custom).or(titles.summary),
    })
}

fn record(value: &Value) -> Record {
    let at = str_at(value, &["timestamp"]).and_then(parse_time);
    let cwd = str_at(value, &["cwd"]).map(str::to_string);
    let kind = str_at(value, &["type"]);
    let flag = |name: &str| value.get(name).and_then(Value::as_bool).unwrap_or(false);
    if flag("isSidechain") || flag("isMeta") || !matches!(kind, Some("user" | "assistant")) {
        return Record::event(at).with_cwd(cwd);
    }
    let content = value
        .get("message")
        .and_then(|message| message.get("content"))
        .unwrap_or(&Value::Null);
    let body = render_content(content);
    let kind = match kind {
        Some("user") if has_prompt_text(content) => RecordKind::Prompt,
        Some("user") => RecordKind::Tool,
        _ => RecordKind::Reply,
    };
    if body.trim().is_empty() {
        return Record::event(at).with_cwd(cwd);
    }
    Record::new(at, kind, &body).with_cwd(cwd)
}

fn has_prompt_text(content: &Value) -> bool {
    match content {
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(blocks) => blocks.iter().any(|block| {
            str_at(block, &["type"]) == Some("text")
                && str_at(block, &["text"]).is_some_and(|text| !text.trim().is_empty())
        }),
        _ => false,
    }
}

fn render_content(content: &Value) -> String {
    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(render_block)
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn render_block(block: &Value) -> Option<String> {
    match str_at(block, &["type"])? {
        "text" => str_at(block, &["text"])
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string),
        "tool_use" => {
            let name = str_at(block, &["name"]).unwrap_or("?");
            let input = block
                .get("input")
                .map(|input| cap(&input.to_string(), TOOL_INPUT_CAP))
                .unwrap_or_default();
            Some(format!("[tool_use: {name} {input}]"))
        }
        "tool_result" => {
            let text = match block.get("content") {
                Some(Value::String(text)) => cap(text, TOOL_TEXT_CAP),
                Some(Value::Array(parts)) => cap(
                    &parts
                        .iter()
                        .filter_map(|part| str_at(part, &["text"]))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    TOOL_TEXT_CAP,
                ),
                _ => String::new(),
            };
            Some(format!("[tool_result: {text}]"))
        }
        // Thinking and images are not part of the work log.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{collect, write_lines};
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_results_logged_as_user_are_not_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_lines(
            dir.path(),
            "p/s1.jsonl",
            &[
                json!({"type": "user", "timestamp": "2026-10-04T10:00:00Z", "cwd": "/repo",
                       "message": {"role": "user", "content": "fix the login bug"}}),
                json!({"type": "assistant", "timestamp": "2026-10-04T10:00:05Z",
                       "message": {"role": "assistant", "content": [
                           {"type": "thinking", "thinking": "hmm"},
                           {"type": "text", "text": "Reading the file."},
                           {"type": "tool_use", "name": "Read", "input": {"path": "a.rs"}}]}}),
                json!({"type": "user", "timestamp": "2026-10-04T10:00:06Z",
                       "message": {"role": "user", "content": [
                           {"type": "tool_result", "content": "fn main() {}"}]}}),
                json!({"type": "user", "isSidechain": true, "timestamp": "2026-10-04T10:00:07Z",
                       "message": {"role": "user", "content": "subagent prompt"}}),
                json!({"type": "custom-title", "customTitle": "Renamed"}),
                json!({"type": "ai-title", "aiTitle": "Fix login"}),
            ],
        );
        let (info, records) = collect(|sink| read(&path, sink));
        let kinds: Vec<RecordKind> = records.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            vec![
                RecordKind::Prompt,
                RecordKind::Reply,
                RecordKind::Tool,
                RecordKind::Event,
                RecordKind::Event,
                RecordKind::Event
            ]
        );
        assert_eq!(
            records[1].body,
            "Reading the file.\n[tool_use: Read {\"path\":\"a.rs\"}]"
        );
        assert_eq!(records[2].body, "[tool_result: fn main() {}]");
        assert_eq!(records[0].cwd.as_deref(), Some("/repo"));
        assert_eq!(info.id, "s1");
        assert_eq!(info.title.as_deref(), Some("Fix login"));
    }
}
