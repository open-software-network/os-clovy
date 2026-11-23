//! Cursor CLI (`cursor-agent`): `~/.cursor/chats/<workspace-md5>/<chat>/
//! store.db`, a small SQLite store per chat. `meta` key `'0'` is hex-encoded
//! JSON (`createdAt` epoch ms, `latestRootBlobId`, `name`); the root blob is
//! protobuf whose repeated field 1 lists the 32-byte ids of the message blobs
//! in order; each message blob is JSON `{role, content}`.
//!
//! Messages carry no timestamps: every record gets the chat's `createdAt`
//! except the last, which gets the store's modification time. The user text
//! is the `<user_query>` part of a user message; messages that only carry
//! `<user_info>` (environment details) are scaffolding.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::query::query;
use sqlx::row::Row;

use super::sqlite::{text_column, value_for_key, Snapshot};
use super::{is_regular_file, list_subdirs, source_root, str_at, under, SourceRoots};
use crate::coding_agents::record::{from_epoch_millis, inner_tag, Record, RecordKind, SessionInfo};
use crate::coding_agents::segment::BlockBuilder;

pub fn root(roots: &SourceRoots) -> PathBuf {
    roots.home.join(".cursor").join("chats")
}

pub fn discover(roots: &SourceRoots) -> Vec<(PathBuf, PathBuf)> {
    let Some(root) = source_root(&root(roots)) else {
        return Vec::new();
    };
    let stores = list_subdirs(&root)
        .into_iter()
        .flat_map(|workspace| list_subdirs(&workspace))
        .map(|chat| chat.join("store.db"))
        .filter(|store| is_regular_file(store))
        .collect();
    under(&root, stores)
}

pub async fn load(
    path: &Path,
    modified: SystemTime,
    new_builder: impl Fn() -> BlockBuilder,
) -> Result<Vec<(SessionInfo, BlockBuilder)>, String> {
    let Some(id) = path
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
    else {
        return Ok(Vec::new());
    };
    let snapshot = Snapshot::open(path).await?;
    let mut builder = new_builder();
    let read = read(&snapshot, modified.into(), &mut builder).await;
    snapshot.close().await;
    Ok(read?
        .map(|title| (SessionInfo { id, title }, builder))
        .into_iter()
        .collect())
}

/// Streams the chat's turns into `builder`; returns the chat's title, or
/// `None` when the store holds no chat.
async fn read(
    snapshot: &Snapshot,
    modified: DateTime<Utc>,
    builder: &mut BlockBuilder,
) -> Result<Option<Option<String>>, String> {
    let pool = snapshot.pool();
    let Some(raw_meta) = value_for_key(pool, "SELECT value FROM meta WHERE key = ?", "0").await
    else {
        return Ok(None);
    };
    let Some(meta) = decode_meta(&raw_meta) else {
        return Ok(None);
    };
    let Some(root_id) = str_at(&meta, &["latestRootBlobId"]) else {
        return Ok(None);
    };
    let root = query("SELECT data FROM blobs WHERE id = ?")
        .bind(root_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| error.to_string())?
        .and_then(|row| row.try_get::<Vec<u8>, _>(0).ok());
    let Some(root) = root else {
        return Ok(None);
    };
    let created = meta
        .get("createdAt")
        .and_then(Value::as_f64)
        .and_then(from_epoch_millis);
    // Every turn is stamped `created` except the last, so each turn is held
    // back until the next one shows it was not the last.
    let mut pending: Option<(RecordKind, String)> = None;
    for id in message_ids(&root) {
        let blob = query("SELECT data FROM blobs WHERE id = ?")
            .bind(&id)
            .fetch_optional(pool)
            .await
            .map_err(|error| error.to_string())?;
        let Some(turn) = blob
            .and_then(|row| text_column(&row, 0))
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|message| turn(&message))
        else {
            continue;
        };
        if let Some((kind, body)) = pending.replace(turn) {
            if !builder.push(Record::new(created, kind, &body)) {
                return Ok(Some(None));
            }
        }
    }
    if let Some((kind, body)) = pending {
        builder.push(Record::new(Some(modified), kind, &body));
    }
    Ok(Some(
        str_at(&meta, &["name"])
            .map(str::trim)
            .filter(|name| !name.is_empty() && *name != "New Agent")
            .map(str::to_string),
    ))
}

/// Hex-encoded JSON, or plain JSON.
pub fn decode_meta(raw: &str) -> Option<Value> {
    let raw = raw.trim();
    let bytes = decode_hex(raw);
    let text = bytes
        .as_deref()
        .map(String::from_utf8_lossy)
        .map(|text| text.into_owned())
        .unwrap_or_else(|| raw.to_string());
    serde_json::from_str(&text).ok().filter(Value::is_object)
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).ok())
        .collect()
}

/// Repeated protobuf field 1 (tag `0x0A`, length-delimited) holding 32-byte
/// ids, read until the first other tag.
pub fn message_ids(root: &[u8]) -> Vec<String> {
    let mut ids = Vec::new();
    let mut index = 0;
    while index < root.len() && root[index] == 0x0A {
        index += 1;
        let Some((length, used)) = varint(&root[index..]) else {
            break;
        };
        index += used;
        let Some(end) = index.checked_add(length).filter(|end| *end <= root.len()) else {
            break;
        };
        if length == 32 {
            ids.push(
                root[index..end]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            );
        }
        index = end;
    }
    ids
}

fn varint(bytes: &[u8]) -> Option<(usize, usize)> {
    let mut value: usize = 0;
    for (index, byte) in bytes.iter().enumerate().take(10) {
        value |= usize::from(byte & 0x7f) << (7 * index);
        if byte & 0x80 == 0 {
            return Some((value, index + 1));
        }
    }
    None
}

fn message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| str_at(part, &["text"]))
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

/// A user or assistant message as a turn; scaffolding yields `None`.
fn turn(message: &Value) -> Option<(RecordKind, String)> {
    let text = message_text(message);
    match str_at(message, &["role"]) {
        Some("user") => {
            let prompt = match inner_tag(&text, "user_query") {
                Some(query) => query.to_string(),
                None if text.contains("<user_info>") => return None,
                None => text,
            };
            (!prompt.is_empty()).then_some((RecordKind::Prompt, prompt))
        }
        Some("assistant") if !text.is_empty() => Some((RecordKind::Reply, text)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn hex(text: &str) -> String {
        text.bytes().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn meta_decodes_from_hex_or_plain_json() {
        let meta = json!({"createdAt": 1, "name": "x"}).to_string();
        assert_eq!(decode_meta(&hex(&meta)).unwrap()["name"], "x");
        assert_eq!(decode_meta(&meta).unwrap()["name"], "x");
    }

    #[test]
    fn root_blob_lists_message_ids_in_order() {
        let mut root = vec![0x0A, 32];
        root.extend([1u8; 32]);
        root.extend([0x0A, 32]);
        root.extend([2u8; 32]);
        root.extend([0x12, 1, 9]);
        let ids = message_ids(&root);
        assert_eq!(ids, vec!["01".repeat(32), "02".repeat(32)]);
    }

    #[tokio::test]
    async fn loads_a_chat_store() {
        use sqlx_sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        let dir = tempfile::tempdir().unwrap();
        let chat = dir.path().join("ws").join("chat-1");
        std::fs::create_dir_all(&chat).unwrap();
        let path = chat.join("store.db");
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(true),
            )
            .await
            .unwrap();
        for sql in [
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT)",
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB)",
        ] {
            query(sql).execute(&pool).await.unwrap();
        }
        let root_id = "aa".repeat(32);
        let meta = json!({"latestRootBlobId": root_id, "name": "Inspect bug", "createdAt": 1_791_100_800_000_i64});
        query("INSERT INTO meta VALUES ('0', ?)")
            .bind(hex(&meta.to_string()))
            .execute(&pool)
            .await
            .unwrap();
        let mut root = Vec::new();
        for byte in [1u8, 2, 3] {
            root.extend([0x0A, 32]);
            root.extend([byte; 32]);
        }
        let blobs = [
            (root_id.clone(), root),
            ("01".repeat(32), json!({"role": "user", "content": [{"type": "text", "text": "<user_info>OS: darwin</user_info>"}]}).to_string().into_bytes()),
            ("02".repeat(32), json!({"role": "user", "content": [{"type": "text", "text": "<user_query>\nwhat is a WAL file?\n</user_query>"}]}).to_string().into_bytes()),
            ("03".repeat(32), json!({"role": "assistant", "content": "A write-ahead log."}).to_string().into_bytes()),
        ];
        for (id, data) in blobs {
            query("INSERT INTO blobs VALUES (?, ?)")
                .bind(id)
                .bind(data)
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;

        let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_791_101_000);
        let created = from_epoch_millis(1_791_100_800_000.0).unwrap();
        let sessions = load(&path, modified, || {
            BlockBuilder::new(
                crate::coding_agents::SourceId::CursorCli,
                DateTime::<Utc>::from(modified),
                created - chrono::Duration::days(1),
            )
        })
        .await
        .unwrap();
        let (info, builder) = sessions.into_iter().next().unwrap();
        assert_eq!(info.id, "chat-1");
        assert_eq!(info.title.as_deref(), Some("Inspect bug"));
        let blocks = builder.finish(&info);
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].first_prompt.as_deref(),
            Some("what is a WAL file?")
        );
        assert_eq!(blocks[0].prompt_count, 1);
        assert_eq!(blocks[0].reply_count, 1);
        assert!(
            !blocks[0].transcript.contains("darwin"),
            "scaffolding is dropped"
        );
        assert_eq!(blocks[0].started_at, created);
        assert_eq!(blocks[0].ended_at, DateTime::<Utc>::from(modified));
    }
}
