//! The agent-blind shape every source normalizes its transcripts into. Only
//! a `Prompt` (a real human message) can open a block; tool results the
//! agent logs as user messages are `Tool`.
//!
//! Readers stream records one at a time into a sink (the block builder), so
//! no reader ever holds a whole transcript; every record body is capped here.

use chrono::{DateTime, TimeZone, Utc};

/// Tool results and tool inputs are capped so one file dump cannot dominate
/// a block's transcript.
pub const TOOL_TEXT_CAP: usize = 800;
pub const TOOL_INPUT_CAP: usize = 400;
/// Any record body (a pasted file in a prompt, a long answer) is capped.
pub const TURN_TEXT_CAP: usize = 4_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordKind {
    /// A message the user typed (non-empty text).
    Prompt,
    /// The agent's answer text (and the tools it decided to call).
    Reply,
    /// A tool call result or tool activity.
    Tool,
    /// Bookkeeping with only a timestamp (metadata, token counts).
    Event,
    /// An explicit end of the session (the agent exited).
    End,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub at: Option<DateTime<Utc>>,
    pub cwd: Option<String>,
    pub kind: RecordKind,
    pub body: String,
}

impl Record {
    /// A record whose body is trimmed and capped at [`TURN_TEXT_CAP`].
    pub fn new(at: Option<DateTime<Utc>>, kind: RecordKind, body: &str) -> Self {
        Self {
            at,
            cwd: None,
            kind,
            body: cap(body, TURN_TEXT_CAP),
        }
    }

    pub fn event(at: Option<DateTime<Utc>>) -> Self {
        Self::new(at, RecordKind::Event, "")
    }

    pub fn with_cwd(mut self, cwd: Option<String>) -> Self {
        self.cwd = cwd.filter(|cwd| !cwd.trim().is_empty());
        self
    }

    pub fn is_turn(&self) -> bool {
        matches!(
            self.kind,
            RecordKind::Prompt | RecordKind::Reply | RecordKind::Tool
        )
    }
}

/// Receives the records of one session in transcript order. Returns `false`
/// to stop reading (the session is one of Clovy's own calls).
pub type RecordSink<'a> = dyn FnMut(Record) -> bool + 'a;

/// What a reader learns about a session besides its records.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub title: Option<String>,
}

/// True when a first prompt is one of Clovy's own CLI calls: the producer
/// (`llm::cli::compose_prompt`) puts the authorship marker alone on the first
/// line. A user prompt that merely mentions the marker is user work.
pub fn is_clovy_prompt(body: &str) -> bool {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|line| line == crate::llm::CLOVY_AUTHORSHIP_MARKER)
}

pub fn from_epoch_millis(millis: f64) -> Option<DateTime<Utc>> {
    if !millis.is_finite() || millis <= 0.0 {
        return None;
    }
    Utc.timestamp_millis_opt(millis as i64).single()
}

/// First `max` characters, with an ellipsis marker when cut.
pub fn cap(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    match trimmed.char_indices().nth(max) {
        Some((end, _)) => format!("{}…[truncated]", &trimmed[..end]),
        None => trimmed.to_string(),
    }
}

/// Text inside `<tag>…</tag>`, if the tag is present.
pub fn inner_tag<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = text.find(&open)? + open.len();
    let end = text[start..]
        .find(&close)
        .map_or(text.len(), |end| start + end);
    Some(text[start..end].trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::CLOVY_AUTHORSHIP_MARKER;

    #[test]
    fn cap_marks_truncation_on_char_boundaries() {
        assert_eq!(cap("ação", 10), "ação");
        assert_eq!(cap("ação ok", 3), "açã…[truncated]");
    }

    #[test]
    fn record_bodies_are_capped() {
        let body = "x".repeat(TURN_TEXT_CAP * 3);
        let record = Record::new(None, RecordKind::Prompt, &body);
        assert_eq!(
            record.body.chars().count(),
            TURN_TEXT_CAP + "…[truncated]".chars().count()
        );
    }

    #[test]
    fn inner_tag_extracts_wrapped_text() {
        assert_eq!(
            inner_tag("<USER_REQUEST>\nfix it\n</USER_REQUEST>", "USER_REQUEST"),
            Some("fix it")
        );
        assert_eq!(inner_tag("plain", "USER_REQUEST"), None);
    }

    #[test]
    fn only_a_marker_alone_on_the_first_line_is_a_clovy_call() {
        assert!(is_clovy_prompt(&format!(
            "{CLOVY_AUTHORSHIP_MARKER}\nSummarize this."
        )));
        assert!(is_clovy_prompt(&format!(
            "\n  {CLOVY_AUTHORSHIP_MARKER}  \nSummarize this."
        )));
        assert!(!is_clovy_prompt(&format!(
            "why does ingestion skip prompts with {CLOVY_AUTHORSHIP_MARKER}?"
        )));
        assert!(!is_clovy_prompt(&format!(
            "grep the repo\n{CLOVY_AUTHORSHIP_MARKER}"
        )));
        assert!(!is_clovy_prompt(&format!(
            "{CLOVY_AUTHORSHIP_MARKER} is the marker, explain it"
        )));
    }
}
