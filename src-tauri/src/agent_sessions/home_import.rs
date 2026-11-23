//! One-time import of the Home conversation that older builds kept only in
//! the webview's local storage (`clovy:home:direct-turns:v1`) into the Home
//! session's items (ADR-0064). The local storage copy is never touched.
//!
//! Each imported item carries the external id
//! `home-import:<stored session id>:<turn id>`, and the presence of any such
//! item is the marker that the session already received its import, so a
//! second open (or a webview that lost and recovered its storage) imports
//! nothing.

use super::domain::{AgentItemPayload, MessagePayload};
use super::repository::AgentRepository;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use sqlx::{query::query, row::Row};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyHomeImport {
    pub imported: usize,
    /// Entries that could not be read (wrong shape, role, or date).
    pub skipped: usize,
    pub already_imported: bool,
}

struct LegacyTurn {
    id: String,
    payload: AgentItemPayload,
    created_at: String,
}

fn import_external_id_prefix(session_id: &str) -> String {
    format!("home-import:{session_id}:")
}

/// A turn as the old Home stored it: `{ id, role, createdAt, parts }`, its
/// text being the `text` parts. `Ok(None)` is a readable turn with nothing to
/// import (a task card's tool part, an attachment-only message).
fn legacy_turn(value: &Value) -> Result<Option<LegacyTurn>, ()> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or(())?;
    let role = value.get("role").and_then(Value::as_str).ok_or(())?;
    let created_at = value
        .get("createdAt")
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .ok_or(())?
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::Millis, true);
    let parts = value.get("parts").and_then(Value::as_array).ok_or(())?;
    let text = parts
        .iter()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    let message = |role: &str| MessagePayload {
        role: role.into(),
        content: text.clone(),
        attachments: Vec::new(),
    };
    let payload = match role {
        "user" => AgentItemPayload::UserMessage(message("user")),
        "assistant" => AgentItemPayload::AssistantMessage(message("assistant")),
        _ => return Err(()),
    };
    if text.is_empty() {
        return Ok(None);
    }
    Ok(Some(LegacyTurn {
        id: id.to_string(),
        payload,
        created_at,
    }))
}

impl AgentRepository {
    /// Imports the old Home turns into `session_id`, once. The turns are
    /// placed among the session's existing items by their original time, and
    /// the items keep that time.
    pub async fn import_legacy_home_turns(
        &self,
        session_id: &str,
        turns: &[Value],
    ) -> Result<LegacyHomeImport, sqlx::Error> {
        let prefix = import_external_id_prefix(session_id);
        let mut result = LegacyHomeImport::default();
        let mut seen = HashSet::new();
        let mut legacy = Vec::new();
        for value in turns {
            match legacy_turn(value) {
                Ok(Some(turn)) if seen.insert(turn.id.clone()) => legacy.push(turn),
                Ok(_) => {}
                Err(()) => result.skipped += 1,
            }
        }
        if result.skipped > 0 {
            tracing::warn!(
                stored_session_id = %session_id,
                skipped = result.skipped,
                "skipped unreadable legacy Home turns"
            );
        }
        // Stable: equal times keep the order they were stored in.
        legacy.sort_by(|left, right| left.created_at.cmp(&right.created_at));

        let mut transaction = self.pool.begin().await?;
        let marker = query(
            "SELECT 1 FROM agent_items WHERE session_id = ? AND substr(external_id, 1, ?) = ? LIMIT 1",
        )
        .bind(session_id)
        .bind(prefix.chars().count() as i64)
        .bind(&prefix)
        .fetch_optional(&mut *transaction)
        .await?;
        if marker.is_some() {
            result.already_imported = true;
            return Ok(result);
        }
        if legacy.is_empty() {
            return Ok(result);
        }
        let existing = query(
            "SELECT id, created_at FROM agent_items WHERE session_id = ? ORDER BY sequence ASC",
        )
        .bind(session_id)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(|row| {
            (
                row.get::<String, _>("id"),
                row.get::<String, _>("created_at"),
            )
        })
        .collect::<Vec<_>>();

        // Merge by time, keeping the existing items in their own order.
        enum Slot<'a> {
            Existing(&'a str),
            Imported(&'a LegacyTurn),
        }
        let mut order = Vec::with_capacity(existing.len() + legacy.len());
        let mut imported = legacy.iter().peekable();
        for (id, created_at) in &existing {
            while let Some(turn) = imported.next_if(|turn| turn.created_at < *created_at) {
                order.push(Slot::Imported(turn));
            }
            order.push(Slot::Existing(id));
        }
        order.extend(imported.map(Slot::Imported));

        // Move every sequence out of the way first: (session, sequence) is
        // unique.
        query("UPDATE agent_items SET sequence = -sequence - 1 WHERE session_id = ?")
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        for (sequence, slot) in order.iter().enumerate() {
            match slot {
                Slot::Existing(id) => {
                    query("UPDATE agent_items SET sequence = ? WHERE id = ?")
                        .bind(sequence as i64)
                        .bind(*id)
                        .execute(&mut *transaction)
                        .await?;
                }
                Slot::Imported(turn) => {
                    let payload_json = turn
                        .payload
                        .value()
                        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?
                        .to_string();
                    query(
                        "INSERT INTO agent_items
                         (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at)
                         VALUES (?, ?, NULL, ?, ?, ?, ?, ?)",
                    )
                    .bind(Uuid::new_v4().to_string())
                    .bind(session_id)
                    .bind(sequence as i64)
                    .bind(turn.payload.kind())
                    .bind(payload_json)
                    .bind(format!("{prefix}{}", turn.id))
                    .bind(&turn.created_at)
                    .execute(&mut *transaction)
                    .await?;
                    result.imported += 1;
                }
            }
        }
        transaction.commit().await?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_sessions::AgentSafetyMode;
    use serde_json::json;
    use sqlx_sqlite::SqlitePoolOptions;

    async fn repository_with_session() -> (AgentRepository, String) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory database");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repository = AgentRepository::new(pool);
        let session = repository
            .create_session(
                "Home",
                "open-software/auto",
                AgentSafetyMode::Sandboxed,
                None,
            )
            .await
            .unwrap();
        (repository, session.id)
    }

    fn turn(id: &str, role: &str, created_at: &str, text: &str) -> Value {
        json!({
            "id": id,
            "role": role,
            "createdAt": created_at,
            "status": "complete",
            "parts": [{ "type": "text", "text": text, "status": "complete" }],
        })
    }

    async fn transcript(repository: &AgentRepository, session_id: &str) -> Vec<(String, String)> {
        repository
            .items(session_id)
            .await
            .unwrap()
            .into_iter()
            .map(|item| match item.payload {
                AgentItemPayload::UserMessage(message) => ("user".into(), message.content),
                AgentItemPayload::AssistantMessage(message) => {
                    ("assistant".into(), message.content)
                }
                other => (other.kind().into(), String::new()),
            })
            .collect()
    }

    #[tokio::test]
    async fn home_import_adds_old_turns_in_order_with_their_time() {
        let (repository, session_id) = repository_with_session().await;
        let turns = [
            turn(
                "home:direct:assistant:2",
                "assistant",
                "2026-10-01T09:00:05.000Z",
                "Hi Gui",
            ),
            turn(
                "home:direct:user:1",
                "user",
                "2026-10-01T09:00:00.000Z",
                "My name is Gui",
            ),
        ];

        let result = repository
            .import_legacy_home_turns(&session_id, &turns)
            .await
            .unwrap();

        assert_eq!(
            result,
            LegacyHomeImport {
                imported: 2,
                skipped: 0,
                already_imported: false
            }
        );
        assert_eq!(
            transcript(&repository, &session_id).await,
            vec![
                ("user".to_string(), "My name is Gui".to_string()),
                ("assistant".to_string(), "Hi Gui".to_string()),
            ]
        );
        let items = repository.items(&session_id).await.unwrap();
        assert_eq!(items[0].created_at, "2026-10-01T09:00:00.000Z");
        assert_eq!(
            items[0].external_id.as_deref(),
            Some(format!("home-import:{session_id}:home:direct:user:1").as_str())
        );
        assert!(items.iter().all(|item| item.run_id.is_none()));
    }

    #[tokio::test]
    async fn home_import_runs_once() {
        let (repository, session_id) = repository_with_session().await;
        let turns = [
            turn("u1", "user", "2026-10-01T09:00:00Z", "One"),
            turn("a1", "assistant", "2026-10-01T09:00:01Z", "Two"),
        ];
        repository
            .import_legacy_home_turns(&session_id, &turns)
            .await
            .unwrap();
        // A later turn of the session, then another open with more old turns.
        let more = [
            turn("u1", "user", "2026-10-01T09:00:00Z", "One"),
            turn("u2", "user", "2026-10-02T09:00:00Z", "Three"),
        ];

        let again = repository
            .import_legacy_home_turns(&session_id, &more)
            .await
            .unwrap();

        assert!(again.already_imported);
        assert_eq!(again.imported, 0);
        assert_eq!(repository.items(&session_id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn home_import_skips_unreadable_entries_and_turns_without_text() {
        let (repository, session_id) = repository_with_session().await;
        let turns = [
            json!("not a turn"),
            json!({ "id": "x", "role": "system", "createdAt": "2026-10-01T09:00:00Z", "parts": [] }),
            json!({ "id": "y", "role": "user", "createdAt": "yesterday", "parts": [] }),
            json!({ "role": "user", "createdAt": "2026-10-01T09:00:00Z", "parts": [] }),
            // A task card's assistant turn: readable, nothing to import.
            json!({
                "id": "card",
                "role": "assistant",
                "createdAt": "2026-10-01T09:00:02Z",
                "parts": [{ "type": "tool", "id": "direct:1", "name": "clovy_home_start_task" }],
            }),
            turn("u1", "user", "2026-10-01T09:00:01Z", "Kept"),
        ];

        let result = repository
            .import_legacy_home_turns(&session_id, &turns)
            .await
            .unwrap();

        assert_eq!(result.imported, 1);
        assert_eq!(result.skipped, 4);
        assert_eq!(
            transcript(&repository, &session_id).await,
            vec![("user".to_string(), "Kept".to_string())]
        );
    }

    #[tokio::test]
    async fn home_import_places_old_turns_among_existing_items_by_time() {
        let (repository, session_id) = repository_with_session().await;
        repository
            .append_item(
                &session_id,
                None,
                0,
                &AgentItemPayload::UserMessage(MessagePayload {
                    role: "user".into(),
                    content: "Existing".into(),
                    attachments: Vec::new(),
                }),
                None,
            )
            .await
            .unwrap();
        let existing_at = repository.items(&session_id).await.unwrap()[0]
            .created_at
            .clone();
        let before = "2000-01-01T00:00:00Z";
        let after = "2999-01-01T00:00:00Z";
        let turns = [
            turn("late", "user", after, "Late"),
            turn("early", "user", before, "Early"),
        ];

        repository
            .import_legacy_home_turns(&session_id, &turns)
            .await
            .unwrap();

        let items = repository.items(&session_id).await.unwrap();
        assert_eq!(
            transcript(&repository, &session_id).await,
            vec![
                ("user".to_string(), "Early".to_string()),
                ("user".to_string(), "Existing".to_string()),
                ("user".to_string(), "Late".to_string()),
            ]
        );
        assert_eq!(
            items.iter().map(|item| item.sequence).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(items[1].created_at, existing_at);
        // The next item of a turn lands after all of them.
        let next = repository
            .append_item(
                &session_id,
                None,
                0,
                &AgentItemPayload::UserMessage(MessagePayload {
                    role: "user".into(),
                    content: "Next".into(),
                    attachments: Vec::new(),
                }),
                None,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.sequence, 3);
    }
}
