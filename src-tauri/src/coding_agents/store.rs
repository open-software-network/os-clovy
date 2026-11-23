//! `coding_agent_blocks` in the encrypted activity database (migration 3 in
//! `activity::store`). Lifecycle: `live` (re-written on every scan while the
//! block grows) → `sealed` (immutable; the upsert only touches live rows) →
//! `summarized` (summary written, terminal).

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::query::query;
use sqlx::row::Row;
use sqlx_sqlite::SqliteRow;

use super::settings::CodingAgentSources;
use super::SourceId;
use crate::activity::store::{timestamp, ActivityStore, StoreError};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BlockState {
    Live,
    Sealed,
    Summarized,
}

impl BlockState {
    pub fn as_str(self) -> &'static str {
        match self {
            BlockState::Live => "live",
            BlockState::Sealed => "sealed",
            BlockState::Summarized => "summarized",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "live" => BlockState::Live,
            "sealed" => BlockState::Sealed,
            "summarized" => BlockState::Summarized,
            _ => return None,
        })
    }
}

/// A block as the scanner produces it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewBlock {
    pub source: SourceId,
    pub session_id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub prompt_count: i64,
    pub reply_count: i64,
    pub active_seconds: i64,
    pub transcript: String,
    pub sealed: bool,
}

/// A stored block without its transcript: what the timeline, the day
/// summary, and the MCP tools read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentBlock {
    pub id: i64,
    pub source: SourceId,
    pub session_id: String,
    pub started_at: String,
    pub ended_at: String,
    pub cwd: Option<String>,
    pub project: Option<String>,
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub prompt_count: i64,
    pub reply_count: i64,
    pub active_seconds: i64,
    pub state: BlockState,
    pub sealed_at: Option<String>,
    pub summary: Option<String>,
    /// Provider key that wrote the summary (`cli:claude`, `endpoint:<id>`).
    pub summary_source: Option<String>,
    pub summary_attempts: i64,
    pub summary_error: Option<String>,
}

/// A sealed block waiting for its summary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingSummary {
    pub block: CodingAgentBlock,
    pub transcript: String,
}

const BLOCK_COLUMNS: &str = "id, source, session_id, started_at, ended_at, cwd, project, title,
    first_prompt, prompt_count, reply_count, active_seconds, state, sealed_at, summary,
    summary_source, summary_attempts, summary_error";

fn block_row(row: &SqliteRow) -> Option<CodingAgentBlock> {
    Some(CodingAgentBlock {
        id: row.get(0),
        source: SourceId::parse(row.get::<&str, _>(1))?,
        session_id: row.get(2),
        started_at: row.get(3),
        ended_at: row.get(4),
        cwd: row.get(5),
        project: row.get(6),
        title: row.get(7),
        first_prompt: row.get(8),
        prompt_count: row.get(9),
        reply_count: row.get(10),
        active_seconds: row.get(11),
        state: BlockState::parse(row.get::<&str, _>(12))?,
        sealed_at: row.get(13),
        summary: row.get(14),
        summary_source: row.get(15),
        summary_attempts: row.get(16),
        summary_error: row.get(17),
    })
}

impl ActivityStore {
    /// Inserts a block or refreshes it while it is still live. Sealed and
    /// summarized blocks never change. Returns whether a row was written.
    pub async fn upsert_coding_agent_block(
        &self,
        block: &NewBlock,
        now: DateTime<Utc>,
    ) -> Result<bool, StoreError> {
        let state = if block.sealed {
            BlockState::Sealed
        } else {
            BlockState::Live
        };
        let written = query(
            "INSERT INTO coding_agent_blocks
                (source, session_id, started_at, ended_at, cwd, project, title, first_prompt,
                 prompt_count, reply_count, active_seconds, transcript, state, sealed_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(source, session_id, started_at) DO UPDATE SET
                ended_at = excluded.ended_at,
                cwd = excluded.cwd,
                project = excluded.project,
                title = excluded.title,
                first_prompt = excluded.first_prompt,
                prompt_count = excluded.prompt_count,
                reply_count = excluded.reply_count,
                active_seconds = excluded.active_seconds,
                transcript = excluded.transcript,
                state = excluded.state,
                sealed_at = excluded.sealed_at,
                updated_at = excluded.updated_at
             WHERE coding_agent_blocks.state = 'live'",
        )
        .bind(block.source.as_str())
        .bind(&block.session_id)
        .bind(timestamp(block.started_at))
        .bind(timestamp(block.ended_at))
        .bind(&block.cwd)
        .bind(&block.project)
        .bind(&block.title)
        .bind(&block.first_prompt)
        .bind(block.prompt_count)
        .bind(block.reply_count)
        .bind(block.active_seconds)
        .bind(&block.transcript)
        .bind(state.as_str())
        .bind(block.sealed.then(|| timestamp(now)))
        .bind(timestamp(now))
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(written > 0)
    }

    /// Seals live blocks whose last record is older than `idle_before`: the
    /// backstop for sessions that stopped without an exit record (closed
    /// terminal, sleep, crash).
    pub async fn seal_idle_coding_agent_blocks(
        &self,
        idle_before: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<u64, StoreError> {
        Ok(query(
            "UPDATE coding_agent_blocks SET state = 'sealed', sealed_at = ?, updated_at = ?
             WHERE state = 'live' AND ended_at < ?",
        )
        .bind(timestamp(now))
        .bind(timestamp(now))
        .bind(timestamp(idle_before))
        .execute(self.pool())
        .await?
        .rows_affected())
    }

    /// Blocks overlapping `[from, to)`, oldest first.
    pub async fn coding_agent_blocks_between(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CodingAgentBlock>, StoreError> {
        let rows = query(&format!(
            "SELECT {BLOCK_COLUMNS} FROM coding_agent_blocks
             WHERE started_at < ? AND ended_at >= ? ORDER BY started_at, id"
        ))
        .bind(timestamp(to))
        .bind(timestamp(from))
        .fetch_all(self.pool())
        .await?;
        Ok(rows.iter().filter_map(block_row).collect())
    }

    /// Blocks overlapping `[from, to)` of the sources still turned on in
    /// `enabled`, oldest first: turning a source off hides its blocks from
    /// every reader (the app, reports and summaries, the MCP server).
    pub async fn enabled_coding_agent_blocks_between(
        &self,
        enabled: &CodingAgentSources,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CodingAgentBlock>, StoreError> {
        let mut blocks = self.coding_agent_blocks_between(from, to).await?;
        blocks.retain(|block| enabled.is_enabled(block.source));
        Ok(blocks)
    }

    /// Sealed blocks of `sources` that ended at or after `since` (the same
    /// boundary ingestion keeps blocks by, so a block crossing it is still
    /// summarized) and are due for a summary attempt, newest first.
    pub async fn coding_agent_blocks_to_summarize(
        &self,
        sources: &[SourceId],
        since: DateTime<Utc>,
        now: DateTime<Utc>,
        max_attempts: i64,
        limit: u32,
    ) -> Result<Vec<PendingSummary>, StoreError> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        // Fixed identifiers from the enum, never user input.
        let source_list = sources
            .iter()
            .map(|source| format!("'{}'", source.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        let rows = query(&format!(
            "SELECT {BLOCK_COLUMNS}, transcript FROM coding_agent_blocks
             WHERE state = 'sealed' AND source IN ({source_list}) AND ended_at >= ?
               AND summary_attempts < ? AND (next_attempt_at IS NULL OR next_attempt_at <= ?)
             ORDER BY ended_at DESC LIMIT ?"
        ))
        .bind(timestamp(since))
        .bind(max_attempts)
        .bind(timestamp(now))
        .bind(i64::from(limit))
        .fetch_all(self.pool())
        .await?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                Some(PendingSummary {
                    block: block_row(row)?,
                    transcript: row.get(18),
                })
            })
            .collect())
    }

    pub async fn record_coding_agent_summary(
        &self,
        id: i64,
        summary: &str,
        provider: &str,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        query(
            "UPDATE coding_agent_blocks
             SET state = 'summarized', summary = ?, summary_source = ?, summarized_at = ?,
                 summary_error = NULL, next_attempt_at = NULL, updated_at = ?
             WHERE id = ? AND state = 'sealed'",
        )
        .bind(summary)
        .bind(provider)
        .bind(timestamp(now))
        .bind(timestamp(now))
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn record_coding_agent_summary_failure(
        &self,
        id: i64,
        error: &str,
        retry_at: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> Result<(), StoreError> {
        query(
            "UPDATE coding_agent_blocks
             SET summary_attempts = summary_attempts + 1, summary_error = ?, next_attempt_at = ?,
                 updated_at = ?
             WHERE id = ? AND state = 'sealed'",
        )
        .bind(error)
        .bind(timestamp(retry_at))
        .bind(timestamp(now))
        .bind(id)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    /// Retention: blocks that ended before `before`.
    pub async fn prune_coding_agent_blocks(
        &self,
        before: DateTime<Utc>,
    ) -> Result<u64, StoreError> {
        Ok(
            query("DELETE FROM coding_agent_blocks WHERE ended_at < ? AND state <> 'live'")
                .bind(timestamp(before))
                .execute(self.pool())
                .await?
                .rows_affected(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::key::MemoryKeyStore;
    use crate::activity::store::ACTIVITY_DB_FILE;
    use chrono::{Duration, TimeZone};

    fn at(minute: i64) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 4, 10, 0, 0).unwrap() + Duration::minutes(minute)
    }

    fn block(ended: i64, sealed: bool, prompts: i64) -> NewBlock {
        NewBlock {
            source: SourceId::Codex,
            session_id: "s1".into(),
            started_at: at(0),
            ended_at: at(ended),
            cwd: Some("/repo".into()),
            project: Some("repo".into()),
            title: None,
            first_prompt: Some("add tests".into()),
            prompt_count: prompts,
            reply_count: 1,
            active_seconds: 60,
            transcript: "user: add tests".into(),
            sealed,
        }
    }

    #[tokio::test]
    async fn sealed_blocks_are_immutable_and_flow_to_summarized() {
        let dir = tempfile::tempdir().unwrap();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&dir.path().join(ACTIVITY_DB_FILE), &keys)
            .await
            .unwrap();

        assert!(store
            .upsert_coding_agent_block(&block(10, false, 1), at(10))
            .await
            .unwrap());
        assert!(store
            .upsert_coding_agent_block(&block(20, false, 2), at(20))
            .await
            .unwrap());
        assert!(
            store
                .coding_agent_blocks_to_summarize(&SourceId::ALL, at(-60), at(30), 3, 10)
                .await
                .unwrap()
                .is_empty(),
            "live blocks are not summarized"
        );
        assert_eq!(
            store
                .seal_idle_coding_agent_blocks(at(19), at(90))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .seal_idle_coding_agent_blocks(at(21), at(90))
                .await
                .unwrap(),
            1
        );
        assert!(
            !store
                .upsert_coding_agent_block(&block(40, false, 5), at(95))
                .await
                .unwrap(),
            "a sealed block never changes"
        );

        let pending = store
            .coding_agent_blocks_to_summarize(&SourceId::ALL, at(-60), at(100), 3, 10)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].block.prompt_count, 2);
        assert_eq!(pending[0].transcript, "user: add tests");

        let id = pending[0].block.id;
        store
            .record_coding_agent_summary_failure(id, "rate limited", at(130), at(100))
            .await
            .unwrap();
        assert!(store
            .coding_agent_blocks_to_summarize(&SourceId::ALL, at(-60), at(110), 3, 10)
            .await
            .unwrap()
            .is_empty());
        store
            .record_coding_agent_summary(id, "Added tests.", "cli:codex", at(131))
            .await
            .unwrap();

        let stored = store
            .coding_agent_blocks_between(at(5), at(6))
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].state, BlockState::Summarized);
        assert_eq!(stored[0].summary.as_deref(), Some("Added tests."));
        assert_eq!(stored[0].summary_source.as_deref(), Some("cli:codex"));
        assert_eq!(stored[0].summary_attempts, 1);
        assert_eq!(stored[0].summary_error, None);
        assert!(store
            .coding_agent_blocks_between(at(21), at(30))
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_block_crossing_the_window_start_is_still_summarized() {
        let dir = tempfile::tempdir().unwrap();
        let keys = MemoryKeyStore::default();
        let store = ActivityStore::open(&dir.path().join(ACTIVITY_DB_FILE), &keys)
            .await
            .unwrap();
        // Starts at 10:00, ends 10:40; the window starts at 10:30.
        store
            .upsert_coding_agent_block(&block(40, true, 1), at(41))
            .await
            .unwrap();
        let before = NewBlock {
            session_id: "s2".into(),
            ..block(20, true, 1)
        };
        store
            .upsert_coding_agent_block(&before, at(41))
            .await
            .unwrap();
        let pending = store
            .coding_agent_blocks_to_summarize(&SourceId::ALL, at(30), at(50), 3, 10)
            .await
            .unwrap();
        let sessions: Vec<&str> = pending
            .iter()
            .map(|item| item.block.session_id.as_str())
            .collect();
        assert_eq!(
            sessions,
            vec!["s1"],
            "only the block that ended inside the window"
        );
    }
}
