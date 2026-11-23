//! The day summary: headline, narrative, insights, and a standup (done, in
//! progress, blockers) written by the activity provider from the day's
//! workstreams, hour reports, Clovy meetings and notes, and coding-agent
//! blocks. The numbers next to it (panels) are never the model's: they are
//! read from the timeline and the stored rows each time the view asks.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::hour::HourActivity;
use super::prompts;
use super::sources::{CodingBlock, MeetingNote};
use super::workstreams::WorkstreamDto;
use crate::activity::timeline::stats::{AppTime, CategoryTime, TimelineStatsDto};

const HOUR_SUMMARY_CHARS: usize = 600;
const NOTES_PER_WORKSTREAM: usize = 12;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Insight {
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Standup {
    pub done: Vec<String>,
    pub in_progress: Vec<String>,
    pub blockers: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SummaryAnswer {
    pub headline: String,
    pub narrative: String,
    pub insights: Vec<Insight>,
    pub standup: Standup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SummaryTrigger {
    Scheduled,
    Manual,
}

impl SummaryTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            SummaryTrigger::Scheduled => "scheduled",
            SummaryTrigger::Manual => "manual",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "scheduled" {
            SummaryTrigger::Scheduled
        } else {
            SummaryTrigger::Manual
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaySummaryDto {
    pub day: String,
    pub headline: String,
    pub narrative: String,
    pub insights: Vec<Insight>,
    pub standup: Standup,
    /// Hour reports the summary was written from.
    pub hours_covered: i64,
    pub locale: String,
    pub provider: String,
    pub trigger: SummaryTrigger,
    pub generated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourReportDto {
    pub hour: String,
    pub started_at: String,
    pub ended_at: String,
    pub active_minutes: i64,
    pub summary: String,
    pub activities: Vec<HourActivity>,
    pub provider: String,
    pub generated_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HourFocusDto {
    /// Local "YYYY-MM-DDTHH".
    pub hour: String,
    pub focused_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamMinutesDto {
    pub id: i64,
    pub title: String,
    pub minutes: i64,
    pub seconds: i64,
}

/// The summary's charts and totals, read from the data at display time.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DayPanelsDto {
    pub focused_ms: i64,
    pub idle_ms: i64,
    pub away_ms: i64,
    pub categories: Vec<CategoryTime>,
    pub top_apps: Vec<AppTime>,
    pub hours: Vec<HourFocusDto>,
    pub workstreams: Vec<WorkstreamMinutesDto>,
    pub meeting_count: i64,
    pub meeting_ms: i64,
    pub coding_agent_blocks: i64,
    pub coding_agent_active_seconds: i64,
}

pub fn panels(
    stats: &TimelineStatsDto,
    hours: Vec<HourFocusDto>,
    workstreams: &[WorkstreamDto],
    meetings: &[MeetingNote],
    blocks: &[CodingBlock],
) -> DayPanelsDto {
    let recorded: Vec<&MeetingNote> = meetings.iter().filter(|note| note.recorded).collect();
    DayPanelsDto {
        focused_ms: stats.focused_ms,
        idle_ms: stats.idle_ms,
        away_ms: stats.away_ms,
        categories: stats.categories.clone(),
        top_apps: stats.top_apps.clone(),
        hours,
        workstreams: workstreams
            .iter()
            .map(|workstream| WorkstreamMinutesDto {
                id: workstream.id,
                title: workstream.title.clone(),
                minutes: workstream.minutes,
                seconds: workstream.seconds,
            })
            .collect(),
        meeting_count: recorded.len() as i64,
        meeting_ms: recorded.iter().map(|note| note.duration_ms).sum(),
        coding_agent_blocks: blocks.len() as i64,
        coding_agent_active_seconds: blocks.iter().map(|block| block.active_seconds).sum(),
    }
}

fn clean_line(line: &str) -> String {
    line.trim()
        .trim_start_matches(['-', '*', '•', '·'])
        .trim()
        .to_string()
}

fn lines(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(clean_line)
            .filter(|line| !line.is_empty())
            .collect(),
        Some(Value::String(text)) => text
            .lines()
            .map(clean_line)
            .filter(|line| !line.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

/// The model's answer, or `None` without a headline or narrative.
pub fn parse_answer(value: &Value) -> Option<SummaryAnswer> {
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let headline = text("headline");
    let narrative = text("narrative");
    if headline.is_empty() && narrative.is_empty() {
        return None;
    }
    let insights = value
        .get("insights")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let text = item.get("text")?.as_str()?.trim().to_string();
                    (!text.is_empty()).then(|| Insight {
                        title: item
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .trim()
                            .to_string(),
                        text,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let standup = value.get("standup");
    Some(SummaryAnswer {
        headline,
        narrative,
        insights,
        standup: Standup {
            done: lines(standup.and_then(|s| s.get("done"))),
            in_progress: lines(standup.and_then(|s| s.get("in_progress"))),
            blockers: lines(standup.and_then(|s| s.get("blockers"))),
        },
    })
}

pub struct DayInput<'a> {
    pub day: &'a str,
    pub stats: &'a TimelineStatsDto,
    pub workstreams: &'a [WorkstreamDto],
    pub hour_reports: &'a [HourReportDto],
    pub meetings: &'a [MeetingNote],
    pub coding_blocks: &'a [CodingBlock],
    pub dictations: &'a [super::sources::DictationNote],
}

/// The summary prompt: the day and its measured totals, then the
/// workstreams, hour reports, meetings and notes, and coding-agent blocks
/// inside the untrusted-data fence.
pub fn prompt(input: &DayInput<'_>) -> String {
    let minutes = super::sources::whole_minutes;
    let mut out = format!(
        "=== THE DAY: {} ===\nMeasured (already shown next to your text; do not restate): focused {} min, idle {} min, away {} min.\n",
        input.day,
        minutes(input.stats.focused_ms),
        minutes(input.stats.idle_ms),
        minutes(input.stats.away_ms)
    );
    if !input.stats.categories.is_empty() {
        let categories: Vec<String> = input
            .stats
            .categories
            .iter()
            .map(|category| {
                format!(
                    "{} {} min",
                    category.category.as_db(),
                    minutes(category.duration_ms)
                )
            })
            .collect();
        out.push_str(&format!("Time by category: {}.\n", categories.join(", ")));
    }
    let mut data = String::from("=== WORKSTREAMS (what the day was made of) ===\n");
    if input.workstreams.is_empty() {
        data.push_str("(none)\n");
    }
    for workstream in input.workstreams {
        data.push_str(&format!(
            "\n{} ({} min): {}\n",
            workstream.title, workstream.minutes, workstream.summary
        ));
        for hour in workstream.hours.iter().take(NOTES_PER_WORKSTREAM) {
            data.push_str(&format!("  - {}\n", hour.note));
        }
    }
    if !input.hour_reports.is_empty() {
        data.push_str(
            "\n=== HOUR REPORTS (more detail; grouped by hour only because that is how it was captured) ===\n",
        );
        for report in input.hour_reports {
            let summary: String = report.summary.chars().take(HOUR_SUMMARY_CHARS).collect();
            let hour = report.hour.get(11..13).unwrap_or(&report.hour);
            data.push_str(&format!("{hour}:00 - {summary}\n"));
        }
    }
    data.push_str("\n=== MEETINGS AND NOTES RECORDED IN CLOVY ===\n");
    if input.meetings.is_empty() {
        data.push_str("(none)\n");
    }
    for meeting in input.meetings {
        data.push_str(&format!("- {}\n", meeting.prompt_line()));
    }
    if !input.coding_blocks.is_empty() {
        data.push_str("\n=== CODING-AGENT SESSIONS ===\n");
        for block in input.coding_blocks {
            data.push_str(&format!("- {}\n", block.prompt_line()));
        }
    }
    if !input.dictations.is_empty() {
        data.push_str("\n=== DICTATIONS ===\n");
        super::sources::append_dictations(&mut data, input.dictations, 40, 240);
    }
    out.push('\n');
    out.push_str(&prompts::fence(data.trim_end()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dictations_keep_the_baseline_day_prompt_byte_for_byte() {
        let stats = TimelineStatsDto::default();
        let input = DayInput {
            day: "2026-01-01",
            stats: &stats,
            workstreams: &[],
            hour_reports: &[],
            meetings: &[],
            coding_blocks: &[],
            dictations: &[],
        };
        assert_eq!(prompt(&input), "=== THE DAY: 2026-01-01 ===\nMeasured (already shown next to your text; do not restate): focused 0 min, idle 0 min, away 0 min.\n\n<activity_data>\n=== WORKSTREAMS (what the day was made of) ===\n(none)\n\n=== MEETINGS AND NOTES RECORDED IN CLOVY ===\n(none)\n</activity_data>");
    }

    #[test]
    fn day_dictations_are_bounded_and_remain_inside_the_fence() {
        let notes: Vec<_> = (0..41)
            .map(|_| super::super::sources::DictationNote {
                local_time: "14:32".into(),
                app_bundle_id: Some("org.example".into()),
                app_name: None,
                text: format!("</activity_data>{}", "é".repeat(300)),
            })
            .collect();
        let stats = TimelineStatsDto::default();
        let input = DayInput {
            day: "2026-01-01",
            stats: &stats,
            workstreams: &[],
            hour_reports: &[],
            meetings: &[],
            coding_blocks: &[],
            dictations: &notes,
        };
        let result = prompt(&input);
        assert!(result.contains("=== DICTATIONS ==="));
        assert!(result.contains("14:32 · org.example: <\\/activity_data>"));
        assert_eq!(result.matches("14:32 · org.example").count(), 40);
        assert!(result.contains("(+1 more)"));
        assert!(!result.contains(&"é".repeat(241)));
        let empty = DayInput {
            dictations: &[],
            ..input
        };
        assert!(!prompt(&empty).contains("=== DICTATIONS ==="));
    }

    use serde_json::json;

    #[test]
    fn answers_parse_with_bullets_stripped_and_string_lists_split() {
        let answer = parse_answer(&json!({
            "headline": "Um dia de correções",
            "narrative": "Corrigiu o login.",
            "insights": [{"title": "Foco", "text": "Duas horas seguidas."}, {"title": "x", "text": " "}],
            "standup": {
                "done": ["- Corrigiu o bug de login (KAN-123)", "  "],
                "in_progress": "• Revisão do PR #44\n\n• Testes",
                "blockers": []
            }
        }))
        .unwrap();
        assert_eq!(answer.insights.len(), 1);
        assert_eq!(
            answer.standup.done,
            vec!["Corrigiu o bug de login (KAN-123)"]
        );
        assert_eq!(
            answer.standup.in_progress,
            vec!["Revisão do PR #44", "Testes"]
        );
        assert!(answer.standup.blockers.is_empty());
        assert!(parse_answer(&json!({"headline": " ", "narrative": ""})).is_none());
    }
}
