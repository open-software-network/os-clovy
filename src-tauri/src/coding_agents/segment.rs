//! Cuts one session into blocks while its records stream in. A block always
//! starts at a user prompt; a prompt starts a new block when:
//!
//! - more than [`IDLE_GAP`] passed since the previous turn;
//! - the open block is at least [`TIME_BOX`] long;
//! - the agent exited after the open block's last turn (a resumed session is
//!   new work).
//!
//! Replies and tool output, however late, join the open block, so delayed
//! agent output never becomes a block of its own. Bookkeeping records and
//! exit markers never move a block's time. Records before the first prompt
//! belong to no block. Cutting depends only
//! on earlier records, so appending to a transcript never moves an existing
//! block's start, which is the block's identity in the store.
//!
//! Memory stays bounded however long the transcript is: closed blocks that
//! ended before the window are dropped as soon as they close, and each kept
//! block holds at most [`TRANSCRIPT_HEAD_CHARS`] + [`TRANSCRIPT_TAIL_CHARS`]
//! of rendered transcript (first part kept, last part rolling).

use std::collections::VecDeque;

use chrono::{DateTime, Duration, SecondsFormat, Utc};

use super::record::{cap, is_clovy_prompt, Record, RecordKind, SessionInfo};
use super::store::NewBlock;
use super::SourceId;

pub const IDLE_GAP: Duration = Duration::hours(1);
pub const TIME_BOX: Duration = Duration::hours(1);
/// Idle time between two records counted as active, at most.
pub const ACTIVE_GAP_CAP: Duration = Duration::minutes(2);
/// Rendered transcript kept per block: the first characters…
pub const TRANSCRIPT_HEAD_CHARS: usize = 70_000;
/// …and a rolling window of the last ones.
pub const TRANSCRIPT_TAIL_CHARS: usize = 30_000;
const FIRST_PROMPT_CAP: usize = 200;
const SEPARATOR: &str = "\n\n";

/// Head plus rolling tail of the block's rendered turns.
#[derive(Debug, Default)]
struct BoundedText {
    head: String,
    head_chars: usize,
    tail: VecDeque<(String, usize)>,
    tail_chars: usize,
    omitted: usize,
}

impl BoundedText {
    fn push(&mut self, piece: String) {
        let chars = piece.chars().count();
        if self.tail.is_empty() && self.head_chars + chars <= TRANSCRIPT_HEAD_CHARS {
            if !self.head.is_empty() {
                self.head.push_str(SEPARATOR);
            }
            self.head.push_str(&piece);
            self.head_chars += chars;
            return;
        }
        self.tail.push_back((piece, chars));
        self.tail_chars += chars;
        while self.tail_chars > TRANSCRIPT_TAIL_CHARS {
            let Some((_, dropped)) = self.tail.pop_front() else {
                break;
            };
            self.tail_chars -= dropped;
            self.omitted += dropped;
        }
    }

    fn render(self) -> String {
        let mut text = self.head;
        if self.omitted > 0 {
            text.push_str(&format!(
                "{SEPARATOR}…[{} characters omitted]…",
                self.omitted
            ));
        }
        for (piece, _) in self.tail {
            if !text.is_empty() {
                text.push_str(SEPARATOR);
            }
            text.push_str(&piece);
        }
        text
    }
}

#[derive(Debug)]
struct OpenBlock {
    started_at: DateTime<Utc>,
    ended_at: DateTime<Utc>,
    cwd: Option<String>,
    first_prompt: Option<String>,
    prompt_count: i64,
    reply_count: i64,
    active_seconds: i64,
    /// The agent exited after the block's last turn.
    exited: bool,
    transcript: BoundedText,
}

impl OpenBlock {
    fn add_turn(&mut self, record: &Record, agent_label: &str) {
        match record.kind {
            RecordKind::Prompt => {
                self.prompt_count += 1;
                if self.first_prompt.is_none() {
                    self.first_prompt = Some(cap(&record.body, FIRST_PROMPT_CAP));
                }
            }
            RecordKind::Reply => self.reply_count += 1,
            _ => {}
        }
        let role = match record.kind {
            RecordKind::Prompt => "user",
            RecordKind::Tool => "tool",
            _ => agent_label,
        };
        self.transcript.push(match record.at {
            Some(at) => format!(
                "[{}] {role}: {}",
                at.to_rfc3339_opts(SecondsFormat::Secs, true),
                record.body
            ),
            None => format!("{role}: {}", record.body),
        });
        self.exited = false;
    }
}

/// Builds the blocks of one session from its streamed records.
#[derive(Debug)]
pub struct BlockBuilder {
    source: SourceId,
    now: DateTime<Utc>,
    window_start: DateTime<Utc>,
    cwd: Option<String>,
    previous_at: Option<DateTime<Utc>>,
    open: Option<OpenBlock>,
    closed: Vec<OpenBlock>,
    first_prompt_seen: bool,
    clovy_call: bool,
}

impl BlockBuilder {
    /// Blocks that ended before `window_start` are not kept.
    pub fn new(source: SourceId, now: DateTime<Utc>, window_start: DateTime<Utc>) -> Self {
        Self {
            source,
            now,
            window_start,
            cwd: None,
            previous_at: None,
            open: None,
            closed: Vec::new(),
            first_prompt_seen: false,
            clovy_call: false,
        }
    }

    /// The session's first prompt was one of Clovy's own CLI calls.
    pub fn is_clovy_call(&self) -> bool {
        self.clovy_call
    }

    /// Feeds the next record. Returns `false` once the session turns out to be
    /// one of Clovy's own calls: the reader stops there.
    pub fn push(&mut self, record: Record) -> bool {
        if let Some(cwd) = &record.cwd {
            self.cwd = Some(cwd.clone());
        }
        if record.kind == RecordKind::Prompt && !self.first_prompt_seen {
            self.first_prompt_seen = true;
            if is_clovy_prompt(&record.body) {
                self.clovy_call = true;
                self.open = None;
                self.closed.clear();
                return false;
            }
        }
        // Bookkeeping (metadata, token counts, an exit marker) never moves a
        // block's time: a session reopened days later must not stretch the
        // old block over those days.
        match record.kind {
            RecordKind::Event => return true,
            RecordKind::End => {
                if let Some(open) = self.open.as_mut() {
                    open.exited = true;
                }
                return true;
            }
            _ => {}
        }
        let label = self.source.display_name();
        let Some(at) = record.at else {
            if let Some(open) = self.open.as_mut() {
                open.add_turn(&record, label);
            }
            return true;
        };
        let starts_block = record.kind == RecordKind::Prompt
            && match (&self.open, self.previous_at) {
                (Some(open), Some(previous_at)) => {
                    open.exited || at - previous_at > IDLE_GAP || at - open.started_at >= TIME_BOX
                }
                _ => true,
            };
        if starts_block {
            self.close_open();
            self.open = Some(OpenBlock {
                started_at: at,
                ended_at: at,
                cwd: self.cwd.clone(),
                first_prompt: None,
                prompt_count: 0,
                reply_count: 0,
                active_seconds: 0,
                exited: false,
                transcript: BoundedText::default(),
            });
        } else if let (Some(open), Some(previous_at)) = (self.open.as_mut(), self.previous_at) {
            let gap = (at - previous_at).clamp(Duration::zero(), ACTIVE_GAP_CAP);
            open.active_seconds += gap.num_seconds();
        }
        self.previous_at = Some(at);
        let Some(open) = self.open.as_mut() else {
            return true;
        };
        open.ended_at = open.ended_at.max(at);
        open.cwd = self.cwd.clone();
        open.add_turn(&record, label);
        true
    }

    fn close_open(&mut self) {
        if let Some(block) = self.open.take() {
            if block.ended_at >= self.window_start {
                self.closed.push(block);
            }
        }
    }

    /// The blocks that ended inside the window. Every block but the last is
    /// sealed; the last stays live until the agent exits or the session has
    /// been idle for longer than [`IDLE_GAP`].
    pub fn finish(mut self, info: &SessionInfo) -> Vec<NewBlock> {
        if self.clovy_call {
            return Vec::new();
        }
        let last = self
            .open
            .take()
            .filter(|last| last.ended_at >= self.window_start);
        let now = self.now;
        let closed = self.closed.into_iter().map(|block| (block, true));
        let last = last.map(|block| {
            let sealed = block.exited || now - block.ended_at > IDLE_GAP;
            (block, sealed)
        });
        closed
            .chain(last)
            .map(|(block, sealed)| NewBlock {
                source: self.source,
                session_id: info.id.clone(),
                started_at: block.started_at,
                ended_at: block.ended_at,
                project: block.cwd.as_deref().and_then(project_name),
                cwd: block.cwd,
                title: info.title.clone(),
                first_prompt: block.first_prompt,
                prompt_count: block.prompt_count,
                reply_count: block.reply_count,
                active_seconds: block.active_seconds,
                transcript: block.transcript.render(),
                sealed,
            })
            .collect()
    }
}

/// The last path component of the working directory.
fn project_name(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd.trim_end_matches('/'))
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(minute: i64) -> Option<DateTime<Utc>> {
        Some(Utc.with_ymd_and_hms(2026, 10, 4, 10, 0, 0).unwrap() + Duration::minutes(minute))
    }

    fn blocks_at(records: Vec<Record>, now_minute: i64, window_minute: i64) -> Vec<NewBlock> {
        let mut builder = BlockBuilder::new(
            SourceId::ClaudeCode,
            at(now_minute).unwrap(),
            at(window_minute).unwrap(),
        );
        for record in records {
            if !builder.push(record) {
                break;
            }
        }
        builder.finish(&SessionInfo {
            id: "s".into(),
            title: Some("Tests".into()),
        })
    }

    fn blocks(records: Vec<Record>) -> Vec<NewBlock> {
        blocks_at(records, 600, -600)
    }

    fn prompt(minute: i64, body: &str) -> Record {
        Record::new(at(minute), RecordKind::Prompt, body).with_cwd(Some("/Users/me/clovy/".into()))
    }

    #[test]
    fn delayed_agent_output_after_an_idle_hour_joins_the_open_block() {
        let blocks = blocks(vec![
            prompt(0, "a"),
            Record::new(at(5), RecordKind::Reply, "b"),
            Record::new(at(66), RecordKind::Reply, "late reply"),
            Record::new(at(70), RecordKind::Tool, "late tool"),
        ]);
        assert_eq!(blocks.len(), 1, "no block without a prompt");
        assert_eq!(blocks[0].ended_at, at(70).unwrap());
        assert_eq!(blocks[0].reply_count, 2);
        assert!(blocks[0].transcript.contains("late reply"));
    }

    #[test]
    fn a_prompt_after_more_than_an_idle_hour_starts_a_block() {
        let blocks = blocks(vec![
            prompt(0, "a"),
            Record::new(at(5), RecordKind::Reply, "b"),
            prompt(66, "back"),
        ]);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].started_at, at(66).unwrap());
        assert_eq!(blocks[0].active_seconds, 120);
        assert_eq!(blocks[0].project.as_deref(), Some("clovy"));
    }

    #[test]
    fn time_box_waits_for_a_prompt() {
        let mut records = vec![prompt(0, "start")];
        records.extend((1..=9).map(|step| Record::new(at(step * 10), RecordKind::Tool, "work")));
        assert_eq!(
            blocks(records).len(),
            1,
            "an autonomous stretch past an hour stays in its block"
        );
    }

    #[test]
    fn exit_closes_the_block_and_a_resume_opens_another() {
        let blocks = blocks(vec![
            prompt(0, "a"),
            Record::new(at(1), RecordKind::Reply, "b"),
            Record::new(at(2), RecordKind::End, ""),
            prompt(10, "resumed"),
        ]);
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].sealed);
    }

    #[test]
    fn bookkeeping_days_later_does_not_stretch_the_block() {
        let records = vec![
            prompt(0, "a"),
            Record::new(at(5), RecordKind::Reply, "b"),
            Record::event(at(3 * 24 * 60)),
            Record::new(at(3 * 24 * 60 + 1), RecordKind::End, ""),
        ];
        // Seen right after: the block still ends at its last turn, and the
        // exit marker seals it without moving its end.
        let blocks = blocks_at(records, 10, -600);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].ended_at, at(5).unwrap());
        assert!(
            blocks[0].sealed,
            "the exit seals it although it is not idle"
        );
    }

    #[test]
    fn records_before_the_first_prompt_belong_to_no_block() {
        let blocks = blocks(vec![
            Record::event(at(0)),
            Record::new(at(1), RecordKind::Reply, "orphan"),
            prompt(2, "a"),
            Record::new(None, RecordKind::Reply, "untimed"),
        ]);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].started_at, at(2).unwrap());
        assert!(!blocks[0].transcript.contains("orphan"));
        assert!(blocks[0].transcript.contains("untimed"));
    }

    #[test]
    fn last_block_stays_live_until_idle_or_exit() {
        let records = vec![
            prompt(0, "a"),
            Record::new(at(5), RecordKind::Reply, "b"),
            prompt(120, "c"),
            Record::new(at(125), RecordKind::Reply, "d"),
        ];
        let live = blocks_at(records.clone(), 150, -600);
        assert!(live[0].sealed);
        assert!(!live[1].sealed, "the last block is still growing");
        assert_eq!(live[1].first_prompt.as_deref(), Some("c"));
        let idle = blocks_at(records, 186, -600);
        assert!(idle[1].sealed, "idle for more than an hour seals it");
    }

    #[test]
    fn blocks_that_ended_before_the_window_are_dropped() {
        let blocks = blocks_at(vec![prompt(0, "old"), prompt(300, "new")], 310, 200);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].first_prompt.as_deref(), Some("new"));
    }

    #[test]
    fn transcript_keeps_its_head_and_a_rolling_tail() {
        let mut records = vec![prompt(0, "first")];
        for step in 0..200 {
            records.push(Record::new(
                at(1),
                RecordKind::Reply,
                &format!("step {step} {}", "y".repeat(3_000)),
            ));
        }
        let blocks = blocks(records);
        let transcript = &blocks[0].transcript;
        let chars = transcript.chars().count();
        assert!(
            chars <= TRANSCRIPT_HEAD_CHARS + TRANSCRIPT_TAIL_CHARS + 200,
            "{chars} characters kept"
        );
        assert!(transcript.contains("user: first"));
        assert!(transcript.contains("step 199"));
        assert!(transcript.contains("characters omitted"));
        assert!(!transcript.contains("step 100 "));
    }

    #[test]
    fn a_clovy_call_stops_reading_and_yields_nothing() {
        let mut builder =
            BlockBuilder::new(SourceId::ClaudeCode, at(600).unwrap(), at(-600).unwrap());
        let marker = format!("{}\nSummarize.", crate::llm::CLOVY_AUTHORSHIP_MARKER);
        assert!(!builder.push(Record::new(at(0), RecordKind::Prompt, &marker)));
        assert!(builder.is_clovy_call());
        assert!(builder.finish(&SessionInfo::default()).is_empty());

        let mut builder =
            BlockBuilder::new(SourceId::ClaudeCode, at(600).unwrap(), at(-600).unwrap());
        let mention = format!("what is {}?", crate::llm::CLOVY_AUTHORSHIP_MARKER);
        assert!(builder.push(Record::new(at(0), RecordKind::Prompt, &mention)));
        assert_eq!(builder.finish(&SessionInfo::default()).len(), 1);
    }
}
