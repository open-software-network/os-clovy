//! Workstreams: the threads of work a day was made of, built by an anchored
//! incremental fold. Each completed hour hands the model the current
//! workstreams as fixed anchors plus the hour's numbered activities; the
//! model answers only this hour's placements. Code applies them: a placement
//! on an existing workstream appends the hour (and may refresh that one
//! workstream's summary), a new one opens a workstream. Titles never change
//! and workstreams that receive nothing are untouched, so a bad answer can
//! only fail to place, never rewrite earlier hours. Minutes come from the
//! hour report's measured minutes, never from the model, and an hour's
//! periods are the union of its placed activities' measured periods.

use std::collections::HashSet;

use serde::Serialize;
use serde_json::{json, Value};

use super::hour::{merge_periods, ActivityPeriod, HourActivity};
use super::prompts;

const TITLE_CHARS: usize = 80;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamHourDto {
    pub hour: String,
    pub minutes: i64,
    pub seconds: i64,
    pub note: String,
    /// Bounds of the measured sessions the hour's work was located in; `None`
    /// when only the hour is known (written before periods existed, or no
    /// activity named a session). Where, not how long: active time is
    /// `seconds`.
    pub periods: Option<Vec<ActivityPeriod>>,
    /// Of `minutes`, those from activities located in `periods`; the rest
    /// happened somewhere in the hour. 0 when `periods` is `None`.
    pub located_minutes: i64,
    pub located_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkstreamDto {
    pub id: i64,
    pub title: String,
    pub summary: String,
    pub minutes: i64,
    pub seconds: i64,
    pub hours: Vec<WorkstreamHourDto>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    pub workstream_id: i64,
    pub title: String,
    pub summary: String,
    /// 1-based activity numbers.
    pub activities: Vec<i64>,
}

/// The hour's changes to the day's workstreams.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FoldPlan {
    pub appends: Vec<Append>,
    pub creates: Vec<Create>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Append {
    pub workstream_id: i64,
    pub minutes: i64,
    pub seconds: i64,
    pub note: String,
    /// The refreshed story; `None` keeps the stored summary.
    pub summary: Option<String>,
    pub periods: Option<Vec<ActivityPeriod>>,
    pub located_minutes: i64,
    pub located_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Create {
    pub title: String,
    pub summary: String,
    pub minutes: i64,
    pub seconds: i64,
    pub note: String,
    pub periods: Option<Vec<ActivityPeriod>>,
    pub located_minutes: i64,
    pub located_seconds: i64,
}

pub fn parse_placements(value: &Value) -> Option<Vec<Placement>> {
    let placements = value.get("placements")?.as_array()?;
    Some(
        placements
            .iter()
            .filter_map(|placement| {
                Some(Placement {
                    workstream_id: placement.get("workstream_id")?.as_i64()?,
                    title: placement
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                    summary: placement
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                    activities: placement
                        .get("activities")?
                        .as_array()?
                        .iter()
                        .filter_map(Value::as_i64)
                        .collect(),
                })
            })
            .collect(),
    )
}

fn short_title(text: &str) -> String {
    let title: String = text.trim().chars().take(TITLE_CHARS).collect();
    title.trim_end_matches(['.', ' ']).to_string()
}

/// Applies the model's placements onto `prior`: each activity lands at most
/// once (first placement wins), unknown ids open a new workstream, and
/// activities the model left out become one new workstream named after the
/// first of them. With no usable answer (`placements` empty), the whole hour
/// is that one new workstream.
pub fn plan_fold(
    prior: &[WorkstreamDto],
    activities: &[HourActivity],
    placements: &[Placement],
) -> FoldPlan {
    let mut plan = FoldPlan::default();
    let mut assigned: HashSet<usize> = HashSet::new();
    let activity_seconds =
        |activity: &HourActivity| -> i64 { activity.seconds.unwrap_or(activity.minutes * 60) };
    let located_sec = |indices: &[usize]| -> i64 {
        indices
            .iter()
            .filter(|&&index| activities[index].periods.is_some())
            .map(|&index| activity_seconds(&activities[index]))
            .sum()
    };
    for placement in placements {
        let picked: Vec<usize> = placement
            .activities
            .iter()
            .filter_map(|&number| usize::try_from(number).ok())
            .filter(|&number| number >= 1 && number <= activities.len())
            .map(|number| number - 1)
            .filter(|&index| assigned.insert(index))
            .collect();
        if picked.is_empty() {
            continue;
        }
        let seconds: i64 = picked
            .iter()
            .map(|&index| activity_seconds(&activities[index]))
            .sum();
        let minutes: i64 = (seconds + 30).div_euclid(60);
        let periods = merge_periods(picked.iter().map(|&index| &activities[index].periods));
        let located_seconds = located_sec(&picked);
        let located_minutes = (located_seconds + 30).div_euclid(60);
        let note = picked
            .iter()
            .map(|&index| activities[index].description.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let summary = (!placement.summary.is_empty()).then(|| placement.summary.clone());
        if prior
            .iter()
            .any(|workstream| workstream.id == placement.workstream_id)
        {
            match plan
                .appends
                .iter_mut()
                .find(|append| append.workstream_id == placement.workstream_id)
            {
                Some(append) => {
                    append.seconds += seconds;
                    append.minutes = (append.seconds + 30).div_euclid(60);
                    append.note.push(' ');
                    append.note.push_str(&note);
                    if summary.is_some() {
                        append.summary = summary;
                    }
                    append.periods = merge_periods([&append.periods, &periods]);
                    append.located_seconds += located_seconds;
                    append.located_minutes = (append.located_seconds + 30).div_euclid(60);
                }
                None => plan.appends.push(Append {
                    workstream_id: placement.workstream_id,
                    minutes,
                    seconds,
                    note,
                    summary,
                    periods,
                    located_minutes,
                    located_seconds,
                }),
            }
        } else {
            let title = if placement.title.is_empty() {
                short_title(&activities[picked[0]].description)
            } else {
                short_title(&placement.title)
            };
            plan.creates.push(Create {
                title,
                summary: summary.unwrap_or_else(|| note.clone()),
                minutes,
                seconds,
                note,
                periods,
                located_minutes,
                located_seconds,
            });
        }
    }
    let left: Vec<usize> = (0..activities.len())
        .filter(|index| !assigned.contains(index))
        .collect();
    if let Some(&first) = left.first() {
        let note = left
            .iter()
            .map(|&index| activities[index].description.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        let left_seconds: i64 = left
            .iter()
            .map(|&index| activity_seconds(&activities[index]))
            .sum();
        let left_located_seconds = located_sec(&left);
        plan.creates.push(Create {
            title: short_title(&activities[first].description),
            summary: note.clone(),
            minutes: (left_seconds + 30).div_euclid(60),
            seconds: left_seconds,
            note,
            periods: merge_periods(left.iter().map(|&index| &activities[index].periods)),
            located_minutes: (left_located_seconds + 30).div_euclid(60),
            located_seconds: left_located_seconds,
        });
    }
    plan
}

/// The fold prompt: the day's workstreams and this hour's numbered
/// activities, both written earlier from captured text, inside the
/// untrusted-data fence.
pub fn prompt(prior: &[WorkstreamDto], hour_label: &str, activities: &[HourActivity]) -> String {
    let anchors: Vec<Value> = prior
        .iter()
        .map(|workstream| {
            json!({
                "id": workstream.id,
                "title": workstream.title,
                "summary": workstream.summary,
            })
        })
        .collect();
    let anchors = if anchors.is_empty() {
        "(none yet)".to_string()
    } else {
        serde_json::to_string_pretty(&anchors).unwrap_or_default()
    };
    let numbered: Vec<String> = activities
        .iter()
        .enumerate()
        .map(|(index, activity)| {
            format!(
                "{}. ({} min) {}",
                index + 1,
                activity.minutes,
                activity.description
            )
        })
        .collect();
    format!(
        "=== CURRENT WORKSTREAMS (anchors; keep them as they are) ===\n{}\n\n=== NEW HOUR {hour_label}: place these activities only ===\n{}\n",
        prompts::fence(&anchors),
        prompts::fence(&numbered.join("\n"))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activity(description: &str, minutes: i64) -> HourActivity {
        HourActivity {
            description: description.into(),
            minutes,
            seconds: None,
            periods: None,
        }
    }

    fn period(from: &str, to: &str) -> ActivityPeriod {
        ActivityPeriod {
            started_at: format!("2026-10-04T{from}:00.000000Z"),
            ended_at: format!("2026-10-04T{to}:00.000000Z"),
        }
    }

    fn workstream(id: i64, title: &str) -> WorkstreamDto {
        WorkstreamDto {
            id,
            title: title.into(),
            summary: format!("{title} so far."),
            minutes: 40,
            seconds: 40 * 60,
            hours: Vec::new(),
        }
    }

    #[test]
    fn placements_parse_from_the_schema_shape() {
        let placements = parse_placements(&json!({"placements": [
            {"workstream_id": 2, "title": "Fix login", "summary": "s", "activities": [1, 2]},
            {"workstream_id": "x", "title": "", "summary": "", "activities": [3]}
        ]}))
        .unwrap();
        assert_eq!(placements.len(), 1);
        assert_eq!(placements[0].activities, vec![1, 2]);
        assert!(parse_placements(&json!({"nope": []})).is_none());
    }

    #[test]
    fn duplicate_and_out_of_range_activities_are_ignored_and_leftovers_open_one_workstream() {
        let prior = vec![workstream(1, "Corrigir bug de login")];
        let activities = vec![
            activity("Fixed the redirect", 20),
            activity("Read the news", 10),
            activity("Answered email", 5),
        ];
        let plan = plan_fold(
            &prior,
            &activities,
            &[
                Placement {
                    workstream_id: 1,
                    title: "ignored".into(),
                    summary: String::new(),
                    activities: vec![1, 1, 9],
                },
                Placement {
                    workstream_id: 0,
                    title: "News".into(),
                    summary: "Read the news.".into(),
                    activities: vec![1, 2],
                },
            ],
        );
        assert_eq!(
            plan.appends,
            vec![Append {
                workstream_id: 1,
                minutes: 20,
                seconds: 20 * 60,
                note: "Fixed the redirect".into(),
                summary: None,
                periods: None,
                located_minutes: 0,
                located_seconds: 0,
            }]
        );
        assert_eq!(plan.creates.len(), 2);
        assert_eq!(
            (
                plan.creates[0].title.as_str(),
                plan.creates[0].minutes,
                plan.creates[0].seconds
            ),
            ("News", 10, 10 * 60)
        );
        assert_eq!(
            (
                plan.creates[1].title.as_str(),
                plan.creates[1].minutes,
                plan.creates[1].seconds
            ),
            ("Answered email", 5, 5 * 60)
        );
    }

    #[test]
    fn without_placements_the_hour_becomes_one_workstream() {
        let plan = plan_fold(
            &[],
            &[activity("Wrote the spec.", 30), activity("Lunch", 12)],
            &[],
        );
        assert!(plan.appends.is_empty());
        assert_eq!(plan.creates.len(), 1);
        assert_eq!(plan.creates[0].title, "Wrote the spec");
        assert_eq!(plan.creates[0].minutes, 42);
        assert_eq!(plan.creates[0].seconds, 42 * 60);
    }

    #[test]
    fn a_placement_keeps_the_measured_periods_of_its_activities() {
        let prior = vec![workstream(1, "Login fix")];
        let activities = vec![
            HourActivity {
                periods: Some(vec![period("14:00", "14:20")]),
                ..activity("Fixed the redirect", 20)
            },
            activity("Thought about tests", 5),
            HourActivity {
                periods: Some(vec![period("14:15", "14:30"), period("14:45", "14:50")]),
                ..activity("Wrote the test", 15)
            },
            activity("Lunch", 10),
        ];
        let placement = |workstream_id, numbers: Vec<i64>| Placement {
            workstream_id,
            title: "Email".into(),
            summary: String::new(),
            activities: numbers,
        };
        let plan = plan_fold(
            &prior,
            &activities,
            &[placement(1, vec![1, 2]), placement(1, vec![3])],
        );
        // Two placements on one workstream: overlapping periods join, the
        // later stretch stays apart (a resumption). The 5 minutes of the
        // activity without periods stay unlocated: the union does not claim
        // the whole hour's work happened inside the periods.
        assert_eq!(plan.appends.len(), 1);
        assert_eq!(plan.appends[0].minutes, 40);
        assert_eq!(plan.appends[0].seconds, 40 * 60);
        assert_eq!(plan.appends[0].located_minutes, 35);
        assert_eq!(plan.appends[0].located_seconds, 35 * 60);
        assert_eq!(
            plan.appends[0].periods,
            Some(vec![period("14:00", "14:30"), period("14:45", "14:50")])
        );
        // Leftovers without any period stay known by hour only.
        assert_eq!(plan.creates.len(), 1);
        assert_eq!(plan.creates[0].periods, None);
        assert_eq!(plan.creates[0].located_minutes, 0);
        assert_eq!(plan.creates[0].located_seconds, 0);
    }
}
