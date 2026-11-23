//! Copilot chat in VS Code: `~/Library/Application Support/Code/User/
//! workspaceStorage/<hash>/chatSessions/<session>.jsonl` (and
//! `globalStorage/emptyWindowChatSessions/` for windows without a folder).
//!
//! The `.jsonl` file is an operation log replayed in order: `kind 0` is a full
//! snapshot, `kind 1` sets the value at key path `k`, `kind 2` appends to the
//! array at `k`. Older builds wrote the snapshot alone as `<session>.json`.
//! The workspace folder comes from the sibling `workspace.json`. Replaying
//! needs the whole session in memory, so files over [`MAX_SESSION_BYTES`]
//! are skipped.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    file_stem, for_each_json_line, inside_root, is_real_dir, is_regular_file, list_dir,
    list_subdirs, open_regular, path_from_file_uri, source_root, str_at, under, SourceRoots,
};
use crate::coding_agents::record::{
    cap, from_epoch_millis, Record, RecordKind, RecordSink, SessionInfo, TOOL_INPUT_CAP,
};

/// A chat session larger than this is not replayed.
pub const MAX_SESSION_BYTES: u64 = 32 * 1024 * 1024;
const MAX_WORKSPACE_JSON_BYTES: u64 = 64 * 1024;

pub fn user_dir(roots: &SourceRoots) -> PathBuf {
    roots.app_support("Code").join("User")
}

pub fn discover(roots: &SourceRoots) -> Vec<(PathBuf, PathBuf)> {
    let Some(root) = source_root(&user_dir(roots)) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = Vec::new();
    let storage = root.join("workspaceStorage");
    if is_real_dir(&storage) {
        dirs.extend(
            list_subdirs(&storage)
                .into_iter()
                .map(|workspace| workspace.join("chatSessions")),
        );
    }
    let global = root.join("globalStorage");
    if is_real_dir(&global) {
        dirs.push(global.join("emptyWindowChatSessions"));
    }
    let files = dirs
        .iter()
        .filter(|dir| is_real_dir(dir))
        .flat_map(|dir| list_dir(dir))
        .filter(|file| {
            file.extension()
                .is_some_and(|ext| ext == "jsonl" || ext == "json")
                && is_regular_file(file)
        })
        .collect();
    under(&root, files)
}

fn read_capped(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    open_regular(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

pub fn read(root: &Path, path: &Path, sink: &mut RecordSink<'_>) -> io::Result<SessionInfo> {
    let mut info = SessionInfo {
        id: file_stem(path),
        title: None,
    };
    if std::fs::symlink_metadata(path)?.len() > MAX_SESSION_BYTES {
        tracing::debug!(path = %path.display(), "coding agents: chat session too large to replay");
        return Ok(info);
    }
    let state = if path.extension().is_some_and(|ext| ext == "json") {
        read_capped(path, MAX_SESSION_BYTES)?
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null)
    } else {
        let mut state = Value::Null;
        for_each_json_line(path, |operation| {
            apply(&mut state, operation);
            true
        })?;
        state
    };
    let cwd = path
        .parent()
        .and_then(Path::parent)
        .map(|workspace| workspace.join("workspace.json"))
        .filter(|file| inside_root(root, file).is_some())
        .and_then(|file| read_capped(&file, MAX_WORKSPACE_JSON_BYTES).ok().flatten())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|workspace| str_at(&workspace, &["folder"]).and_then(path_from_file_uri));
    info.title = str_at(&state, &["customTitle"])
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string);
    emit(&state, cwd, sink);
    Ok(info)
}

/// Applies one operation of the log.
fn apply(state: &mut Value, operation: &Value) {
    let value = operation.get("v").cloned().unwrap_or(Value::Null);
    match operation.get("kind").and_then(Value::as_i64) {
        Some(0) => *state = value,
        Some(1) => set(state, operation.get("k"), value),
        Some(2) => {
            let path = operation.get("k").and_then(Value::as_array);
            if let Some(Value::Array(items)) = path.and_then(|path| walk(state, path)) {
                match value {
                    Value::Array(more) => items.extend(more),
                    other => items.push(other),
                }
            }
        }
        _ => {}
    }
}

/// The value at `path`; `None` (the operation is dropped) when any step is
/// missing, so a malformed log never invents structure.
fn walk<'a>(state: &'a mut Value, path: &[Value]) -> Option<&'a mut Value> {
    let mut current = state;
    for step in path {
        current = match step {
            Value::String(key) => current.get_mut(key.as_str())?,
            Value::Number(index) => current.get_mut(usize::try_from(index.as_u64()?).ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Sets the value at `path`. The parent must exist; a new property may be
/// added to an existing object (VS Code sets `modelState` on completion).
fn set(state: &mut Value, path: Option<&Value>, value: Value) {
    let Some((last, parent)) = path
        .and_then(Value::as_array)
        .and_then(|path| path.split_last())
    else {
        return;
    };
    match (walk(state, parent), last) {
        (Some(Value::Object(object)), Value::String(key)) => {
            object.insert(key.clone(), value);
        }
        (Some(Value::Array(items)), Value::Number(index)) => {
            if let Some(slot) = index
                .as_u64()
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| items.get_mut(index))
            {
                *slot = value;
            }
        }
        _ => {}
    }
}

fn emit(state: &Value, cwd: Option<String>, sink: &mut RecordSink<'_>) {
    // The workspace folder applies to the whole session.
    if !sink(Record::event(None).with_cwd(cwd)) {
        return;
    }
    for request in state
        .get("requests")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let asked_at = request
            .get("timestamp")
            .and_then(Value::as_f64)
            .and_then(from_epoch_millis);
        let prompt = str_at(request, &["message", "text"])
            .unwrap_or_default()
            .trim();
        if !prompt.is_empty() && !sink(Record::new(asked_at, RecordKind::Prompt, prompt)) {
            return;
        }
        let answered_at = request
            .get("modelState")
            .and_then(|state| state.get("completedAt"))
            .and_then(Value::as_f64)
            .and_then(from_epoch_millis)
            .or(asked_at);
        let reply = response_text(request.get("response"));
        if !reply.is_empty() && !sink(Record::new(answered_at, RecordKind::Reply, &reply)) {
            return;
        }
    }
}

fn response_text(parts: Option<&Value>) -> String {
    parts
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| match str_at(part, &["kind"]) {
            None => str_at(part, &["value"])
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string),
            Some("toolInvocationSerialized") => {
                let tool = str_at(part, &["toolId"]).unwrap_or("?");
                let message = str_at(part, &["invocationMessage"])
                    .or_else(|| str_at(part, &["invocationMessage", "value"]))
                    .unwrap_or_default();
                Some(format!(
                    "[tool_use: {tool} {}]",
                    cap(message, TOOL_INPUT_CAP)
                ))
            }
            Some("textEditGroup") => {
                str_at(part, &["uri", "fsPath"]).map(|path| format!("[edit: {path}]"))
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{collect, write_lines};
    use super::*;
    use serde_json::json;

    #[test]
    fn replays_the_operation_log_with_the_workspace_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("workspaceStorage/h1")).unwrap();
        std::fs::write(
            root.join("workspaceStorage/h1/workspace.json"),
            r#"{"folder":"file:///Users/me/My%20Repo"}"#,
        )
        .unwrap();
        let path = write_lines(
            &root,
            "workspaceStorage/h1/chatSessions/s1.jsonl",
            &[
                json!({"kind": 0, "v": {"customTitle": "fix the bug", "requests": [
                    {"timestamp": 1_791_100_800_000_i64, "message": {"text": "fix the login bug"}, "response": []}]}}),
                json!({"kind": 2, "k": ["requests", 0, "response"], "v": [
                    {"kind": "thinking", "value": "null check"},
                    {"kind": "toolInvocationSerialized", "toolId": "copilot_readFile", "invocationMessage": "Reading auth.ts"}]}),
                json!({"kind": 2, "k": ["requests", 0, "response"], "v": [
                    {"kind": "textEditGroup", "uri": {"fsPath": "/repo/auth.ts"}},
                    {"value": "Added a null check."}]}),
                json!({"kind": 1, "k": ["requests", 0, "modelState"], "v": {"completedAt": 1_791_100_830_000_i64}}),
                json!({"kind": 1, "k": ["missing", 3], "v": 1}),
            ],
        );
        let (info, records) = collect(|sink| read(&root, &path, sink));
        assert_eq!(info.id, "s1");
        assert_eq!(info.title.as_deref(), Some("fix the bug"));
        assert_eq!(records[0].cwd.as_deref(), Some("/Users/me/My Repo"));
        let turns: Vec<_> = records.iter().filter(|r| r.is_turn()).collect();
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].kind, RecordKind::Prompt);
        assert_eq!(
            turns[1].body,
            "[tool_use: copilot_readFile Reading auth.ts]\n[edit: /repo/auth.ts]\nAdded a null check."
        );
        let gap = turns[1].at.unwrap() - turns[0].at.unwrap();
        assert_eq!(gap.num_seconds(), 30);
    }
}
