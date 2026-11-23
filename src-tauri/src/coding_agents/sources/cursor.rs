//! Cursor (the editor): every conversation lives in
//! `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`,
//! table `cursorDiskKV`. `composerData:<composer>` holds the conversation
//! (`fullConversationHeadersOnly` is the authoritative turn order; key order
//! is not), and `bubbleId:<composer>:<bubble>` each message (`type` 1 = user,
//! 2 = assistant, `createdAt` ISO time). The store keeps no working directory.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::query::query;

use super::sqlite::{text_column, value_for_key, Snapshot};
use super::{is_regular_file, source_root, str_at, SourceRoots};
use crate::activity::store::parse_time;
use crate::coding_agents::record::{
    cap, from_epoch_millis, Record, RecordKind, SessionInfo, TOOL_INPUT_CAP, TOOL_TEXT_CAP,
};
use crate::coding_agents::segment::BlockBuilder;

const VALUE_SQL: &str = "SELECT value FROM cursorDiskKV WHERE key = ?";

pub fn root(roots: &SourceRoots) -> PathBuf {
    roots
        .app_support("Cursor")
        .join("User")
        .join("globalStorage")
}

pub fn discover(roots: &SourceRoots) -> Vec<(PathBuf, PathBuf)> {
    source_root(&root(roots))
        .map(|root| (root.clone(), root.join("state.vscdb")))
        .filter(|(_, database)| is_regular_file(database))
        .into_iter()
        .collect()
}

/// Conversations updated at or after `since`, each streamed into a builder.
pub async fn load(
    path: &Path,
    since: DateTime<Utc>,
    new_builder: impl Fn() -> BlockBuilder,
) -> Result<Vec<(SessionInfo, BlockBuilder)>, String> {
    let snapshot = Snapshot::open(path).await?;
    let result = read(&snapshot, since, new_builder).await;
    snapshot.close().await;
    result
}

async fn read(
    snapshot: &Snapshot,
    since: DateTime<Utc>,
    new_builder: impl Fn() -> BlockBuilder,
) -> Result<Vec<(SessionInfo, BlockBuilder)>, String> {
    let pool = snapshot.pool();
    // Keys only: conversations are loaded one at a time.
    let keys: Vec<String> =
        query("SELECT key FROM cursorDiskKV WHERE key LIKE 'composerData:%' ORDER BY key")
            .fetch_all(pool)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .filter_map(|row| text_column(row, 0))
            .collect();
    let mut sessions = Vec::new();
    for key in keys {
        let Some(composer) = value_for_key(pool, VALUE_SQL, &key)
            .await
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        else {
            continue;
        };
        let updated = ["lastUpdatedAt", "createdAt"]
            .iter()
            .find_map(|field| composer.get(field).and_then(Value::as_f64))
            .and_then(from_epoch_millis);
        if !updated.is_some_and(|updated| updated >= since) {
            continue;
        }
        let id = key.trim_start_matches("composerData:").to_string();
        let mut builder = new_builder();
        let mut any = false;
        for header in composer
            .get("fullConversationHeadersOnly")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(bubble_id) = str_at(header, &["bubbleId"]) else {
                continue;
            };
            let bubble = value_for_key(pool, VALUE_SQL, &format!("bubbleId:{id}:{bubble_id}"))
                .await
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
            if let Some(bubble) = bubble {
                any = true;
                if !builder.push(record(&bubble)) {
                    break;
                }
            }
        }
        if any {
            let title = str_at(&composer, &["name"])
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string);
            sessions.push((SessionInfo { id, title }, builder));
        }
    }
    Ok(sessions)
}

fn record(bubble: &Value) -> Record {
    let at = str_at(bubble, &["createdAt"]).and_then(parse_time);
    let text = str_at(bubble, &["text"]).unwrap_or_default().trim();
    match bubble.get("type").and_then(Value::as_i64) {
        Some(1) if !text.is_empty() => Record::new(at, RecordKind::Prompt, text),
        Some(2) => {
            let mut parts = Vec::new();
            if let Some(tool) = bubble.get("toolFormerData") {
                let name = str_at(tool, &["name"]).unwrap_or("?");
                let args = str_at(tool, &["rawArgs"]).unwrap_or_default();
                parts.push(format!("[tool_use: {name} {}]", cap(args, TOOL_INPUT_CAP)));
                if let Some(result) = str_at(tool, &["result"]).filter(|r| !r.trim().is_empty()) {
                    parts.push(format!("[tool_result: {}]", cap(result, TOOL_TEXT_CAP)));
                }
            }
            if !text.is_empty() {
                parts.push(text.to_string());
            }
            if parts.is_empty() {
                Record::event(at)
            } else {
                Record::new(at, RecordKind::Reply, &parts.join("\n"))
            }
        }
        _ => Record::event(at),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx_sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn fixture(path: &Path, rows: &[(&str, &str)]) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        query("CREATE TABLE cursorDiskKV (key TEXT UNIQUE ON CONFLICT REPLACE, value BLOB)")
            .execute(&pool)
            .await
            .unwrap();
        for (key, value) in rows {
            query("INSERT INTO cursorDiskKV (key, value) VALUES (?, ?)")
                .bind(*key)
                .bind(value.as_bytes())
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;
    }

    #[tokio::test]
    async fn reads_conversations_in_header_order_and_skips_old_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.vscdb");
        fixture(
            &path,
            &[
                ("composerData:c1", r#"{"name":"fix the bug","createdAt":1791100800000,"lastUpdatedAt":1791100830000,"fullConversationHeadersOnly":[{"bubbleId":"b1","type":1},{"bubbleId":"b2","type":2}]}"#),
                ("bubbleId:c1:b2", r#"{"type":2,"createdAt":"2026-10-04T08:00:10Z","text":"Fixed.","toolFormerData":{"name":"read_file","rawArgs":"{\"path\":\"auth.ts\"}","result":"{\"lines\":42}"}}"#),
                ("bubbleId:c1:b1", r#"{"type":1,"createdAt":"2026-10-04T08:00:00Z","text":"fix the login bug"}"#),
                ("composerData:old", r#"{"createdAt":1000,"fullConversationHeadersOnly":[{"bubbleId":"x","type":1}]}"#),
                ("bubbleId:old:x", r#"{"type":1,"createdAt":"1970-01-01T00:00:01Z","text":"old"}"#),
            ],
        )
        .await;
        let since = parse_time("2026-10-01T00:00:00Z").unwrap();
        let now = parse_time("2026-10-04T12:00:00Z").unwrap();
        let sessions = load(&path, since, || {
            BlockBuilder::new(crate::coding_agents::SourceId::Cursor, now, since)
        })
        .await
        .unwrap();
        assert_eq!(sessions.len(), 1);
        let (info, builder) = sessions.into_iter().next().unwrap();
        assert_eq!(info.id, "c1");
        assert_eq!(info.title.as_deref(), Some("fix the bug"));
        let blocks = builder.finish(&info);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].first_prompt.as_deref(), Some("fix the login bug"));
        assert!(blocks[0].transcript.contains(
            "[tool_use: read_file {\"path\":\"auth.ts\"}]\n[tool_result: {\"lines\":42}]\nFixed."
        ));
        assert_eq!(
            blocks[0].ended_at,
            parse_time("2026-10-04T08:00:10Z").unwrap()
        );
    }
}
