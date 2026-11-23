//! The day-intelligence spec scenarios that are unit-level, as written in
//! the OpenSpec change `add-activity-intelligence` (spec `day-intelligence`).

use super::db;
use super::distill::{distill, SessionText};
use super::embedder::NoEmbedder;
use super::hour::{
    assemble, normalize_seconds, parse_answer, AnswerActivity, HourActivity, HourAnswer,
};
use super::notify::{NoticeKind, NoticeQueue};
use super::schedule::{auto_summary_due, FixedZone, LocalZone, RunOutcome, RunRecord};
use super::test_support;
use super::workstreams::{plan_fold, Append, Placement, WorkstreamDto, WorkstreamHourDto};
use crate::activity::settings::{ActivityNotificationSettings, QuietHours};
use chrono::{Duration, NaiveDate, NaiveDateTime, Utc};
use serde_json::{json, Value};
use sqlx::query::query;

fn zone() -> FixedZone {
    FixedZone(chrono::FixedOffset::west_opt(3 * 3600).unwrap())
}

fn local(day: u32, hour: u32, minute: u32) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 10, day)
        .unwrap()
        .and_hms_opt(hour, minute, 0)
        .unwrap()
}

/// Scenario "Minutos normalizados": 42 active minutes, the model estimates
/// 30 and 30 → the report shows 21 and 21.
#[test]
fn reported_minutes_are_the_measured_minutes_split_by_the_model_weights() {
    assert_eq!(
        normalize_seconds(42 * 60, &[30.0, 30.0]),
        vec![21 * 60, 21 * 60]
    );
    assert_eq!(normalize_seconds(120, &[10.0, 1.0, 1.0]), vec![98, 11, 11]);
    assert_eq!(normalize_seconds(0, &[5.0]), vec![0]);
    assert_eq!(normalize_seconds(-10, &[5.0, 5.0]), vec![0, 0]);
    assert_eq!(normalize_seconds(9, &[0.0, -3.0, f64::NAN]), vec![3, 3, 3]);
    assert_eq!(normalize_seconds(10, &[1e308, 1e308]), vec![5, 5]);
    assert_eq!(normalize_seconds(2, &[1.0, 1.0, 1.0]), vec![1, 1, 0]);
    let answer = HourAnswer {
        summary: "s".into(),
        activities: ["Fixed the login bug", "Reviewed a PR"]
            .into_iter()
            .map(|description| AnswerActivity {
                description: description.into(),
                minutes: 30.0,
                sessions: Vec::new(),
            })
            .collect(),
    };
    let activities = assemble(&answer, 42 * 60, &[]);
    let minutes: Vec<i64> = activities.iter().map(|activity| activity.minutes).collect();
    assert_eq!(minutes, vec![21, 21]);
    let seconds: Vec<i64> = activities
        .iter()
        .map(|activity| activity.seconds.unwrap_or(0))
        .collect();
    assert_eq!(seconds, vec![1260, 1260]);
}

/// Scenario "Entidades preservadas": repeated interface lines go, the
/// `KAN-123` line and the file path stay.
#[tokio::test]
async fn distillation_drops_repeated_interface_lines_and_keeps_entities() {
    let chrome = "You are signed in as gui and you have no new notifications";
    let sessions: Vec<SessionText> = (1..=4)
        .map(|id| SessionText {
            session_id: id,
            app: "Arc".into(),
            window: "GitHub".into(),
            time: format!("14:0{id}"),
            seconds: 300,
            documents: vec![(
                format!("14:0{id}"),
                if id == 1 {
                    format!("{chrome}\nMoved KAN-123 to review after editing src/auth/login.rs")
                } else {
                    format!("{chrome}\nRead the discussion about release number {id} with the team")
                },
            )],
        })
        .collect();
    let out = distill(&sessions, "=== HOUR ===", &NoEmbedder).await;
    assert!(!out.body.contains("no new notifications"), "{}", out.body);
    assert!(out.body.contains("KAN-123"));
    assert!(out.body.contains("src/auth/login.rs"));
}

/// Scenario "Hora atribuída a frente existente": the hour is appended to
/// "Corrigir bug de login" and the other workstreams do not change.
#[test]
fn an_hour_continuing_a_workstream_is_appended_to_it_only() {
    let prior = vec![
        WorkstreamDto {
            id: 7,
            title: "Corrigir bug de login".into(),
            summary: "Investigou o redirecionamento.".into(),
            minutes: 35,
            seconds: 35 * 60,
            hours: vec![WorkstreamHourDto {
                hour: "2026-10-04T13".into(),
                minutes: 35,
                seconds: 35 * 60,
                note: "Investigou".into(),
                periods: None,
                located_minutes: 0,
                located_seconds: 0,
            }],
        },
        WorkstreamDto {
            id: 8,
            title: "Planejamento".into(),
            summary: "Revisou o quadro.".into(),
            minutes: 10,
            seconds: 10 * 60,
            hours: Vec::new(),
        },
    ];
    let activities = assemble(
        &HourAnswer {
            summary: "s".into(),
            activities: vec![AnswerActivity {
                description: "Corrigiu o redirecionamento".into(),
                minutes: 1.0,
                sessions: Vec::new(),
            }],
        },
        40 * 60,
        &[],
    );
    let plan = plan_fold(
        &prior,
        &activities,
        &[Placement {
            workstream_id: 7,
            title: "Corrigir bug de login".into(),
            summary: "Investigou e corrigiu o redirecionamento.".into(),
            activities: vec![1],
        }],
    );
    assert!(plan.creates.is_empty());
    assert_eq!(
        plan.appends,
        vec![Append {
            workstream_id: 7,
            minutes: 40,
            seconds: 40 * 60,
            note: "Corrigiu o redirecionamento".into(),
            summary: Some("Investigou e corrigiu o redirecionamento.".into()),
            periods: None,
            located_minutes: 0,
            located_seconds: 0,
        }]
    );
}

/// Scenario "Mac dormindo às 18:00": asleep from 17:30 to 19:00, the summary
/// is generated right after waking, and only once that day.
#[test]
fn the_daily_summary_runs_once_right_after_waking() {
    let zone = zone();
    let mut record: Option<RunRecord> = None;
    let mut generated = Vec::new();
    // The scheduler ticks every minute while awake; no ticks while asleep.
    let awake = (0..30)
        .map(|minute| local(4, 17, minute))
        .chain((0..120).map(|minute| local(4, 19, 0) + Duration::minutes(minute)));
    for tick in awake {
        let now = zone.instant(tick).unwrap();
        if auto_summary_due(tick, "18:00", record.as_ref(), now) {
            generated.push(tick);
            record = Some(RunRecord {
                attempts: 1,
                outcome: RunOutcome::Ok,
                next_attempt_at: None,
            });
        }
    }
    assert_eq!(generated, vec![local(4, 19, 0)]);
}

/// Scenario "Horário silencioso": quiet 22:00-08:00, ready at 23:00 →
/// delivered only at 08:00.
#[test]
fn a_summary_ready_in_quiet_hours_is_delivered_when_they_end() {
    let zone = zone();
    let settings = ActivityNotificationSettings {
        enabled: true,
        quiet_hours: QuietHours {
            enabled: true,
            start: "22:00".into(),
            end: "08:00".into(),
        },
    };
    let mut queue = NoticeQueue::default();
    let ready = zone.instant(local(4, 23, 0)).unwrap();
    queue.raise(
        NoticeKind::SummaryReady,
        "2026-10-04".into(),
        None,
        ready,
        &zone,
        &settings,
    );
    let mut delivered_at = None;
    let mut now = ready;
    while now < ready + Duration::hours(12) {
        if !queue.take_due(now, &zone, &settings).is_empty() {
            delivered_at = Some(now);
            break;
        }
        now += Duration::minutes(1);
    }
    assert_eq!(delivered_at.map(|at| zone.local(at)), Some(local(5, 8, 0)));
}

/// Scenario "Horas e atividades expõem segundos via JSON":
/// HourActivity deve serializar `seconds` e tolerar ausência em JSON legado.
#[test]
fn hour_activity_serde_preserves_seconds_and_handles_legacy_without_seconds() {
    let legacy =
        serde_json::from_str::<HourActivity>(r#"{"description": "Old", "minutes": 42}"#).unwrap();
    assert_eq!(legacy.minutes, 42);
    let legacy_val = serde_json::to_value(&legacy).unwrap();
    assert!(
        legacy_val.get("seconds").is_none() || legacy_val["seconds"].is_null(),
        "Relatório legado sem segundos não deve inventar campo inválido"
    );

    let modern = serde_json::from_str::<HourActivity>(
        r#"{"description": "New", "minutes": 21, "seconds": 1260}"#,
    )
    .unwrap();
    let modern_val = serde_json::to_value(&modern).unwrap();
    assert_eq!(
        modern_val.get("seconds"),
        Some(&json!(1260)),
        "HourActivity deve preservar seconds após desserialização"
    );
}

/// Scenario "Assemble expõe segundos nas atividades":
/// Em 120s reais (2 min), atividades recebem partes em segundos estritamente positivas somando 120.
#[test]
fn assemble_activities_expose_seconds_via_json() {
    let answer = HourAnswer {
        summary: "s".into(),
        activities: ["Task 1", "Task 2", "Task 3"]
            .into_iter()
            .zip([10.0, 1.0, 1.0])
            .map(|(desc, minutes)| AnswerActivity {
                description: desc.into(),
                minutes,
                sessions: Vec::new(),
            })
            .collect(),
    };
    let activities = assemble(&answer, 120, &[]);
    let act_json = serde_json::to_value(&activities).unwrap();
    assert!(
        act_json[0].get("seconds").is_some() && !act_json[0]["seconds"].is_null(),
        "HourActivity deve serializar seconds: {}",
        act_json[0]
    );
    let s1 = act_json[0]
        .get("seconds")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let s2 = act_json[1]
        .get("seconds")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let s3 = act_json[2]
        .get("seconds")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    assert!(
        s1 > 0 && s2 > 0 && s3 > 0,
        "Todas as partes devem ser > 0: {s1}, {s2}, {s3}"
    );
    assert_eq!(s1 + s2 + s3, 120);
}

/// Scenario "Assemble distribui segundos com pesos inválidos e ausentes":
/// Pesos zero, negativos, NaN e infinito devem virar o menor positivo e receber parte > 0.
#[test]
fn assemble_handles_invalid_weights_with_positive_seconds() {
    let answer = HourAnswer {
        summary: "s".into(),
        activities: ["Task Pos", "Task Zero", "Task Neg", "Task NaN", "Task Inf"]
            .into_iter()
            .zip([10.0, 0.0, -1.0, f64::NAN, f64::INFINITY])
            .map(|(desc, minutes)| AnswerActivity {
                description: desc.into(),
                minutes,
                sessions: Vec::new(),
            })
            .collect(),
    };
    let activities = assemble(&answer, 300, &[]);
    let act_json = serde_json::to_value(&activities).unwrap();
    for (i, act) in act_json.as_array().unwrap().iter().enumerate() {
        let sec = act.get("seconds").and_then(Value::as_i64).unwrap_or(0);
        assert!(
            sec > 0,
            "Activity {i} should receive positive seconds, got {sec}: {act}"
        );
    }
    let total: i64 = act_json
        .as_array()
        .unwrap()
        .iter()
        .map(|act| act.get("seconds").and_then(Value::as_i64).unwrap_or(0))
        .sum();
    assert_eq!(total, 300);
}

/// Scenario "Assemble quando active_seconds < n ou active_seconds == 0":
#[test]
fn assemble_handles_active_time_smaller_than_activity_count() {
    let answer = HourAnswer {
        summary: "s".into(),
        activities: ["Task 1", "Task 2", "Task 3"]
            .into_iter()
            .map(|desc| AnswerActivity {
                description: desc.into(),
                minutes: 1.0,
                sessions: Vec::new(),
            })
            .collect(),
    };
    let zero_acts = assemble(&answer, 0, &[]);
    let zero_json = serde_json::to_value(&zero_acts).unwrap();
    for act in zero_json.as_array().unwrap() {
        let sec = act.get("seconds").and_then(Value::as_i64).unwrap_or(-1);
        assert_eq!(sec, 0, "When active time is 0, seconds must be 0: {act}");
    }

    let partial_acts = assemble(&answer, 2, &[]);
    let partial_json = serde_json::to_value(&partial_acts).unwrap();
    let seconds: Vec<i64> = partial_json
        .as_array()
        .unwrap()
        .iter()
        .map(|act| act.get("seconds").and_then(Value::as_i64).unwrap_or(-1))
        .collect();
    for &sec in &seconds {
        assert!(sec >= 0, "Seconds must be non-negative: {sec}");
    }
    let sum: i64 = seconds.iter().sum();
    assert_eq!(sum, 2, "Sum of seconds must equal active total (2s)");
}

/// Scenario "parse_answer trata minutos ausentes como 0.0 e assemble distribui segundos positivos":
#[test]
fn parse_answer_and_assemble_handle_missing_minutes() {
    let parsed = parse_answer(&json!({
        "summary": "s",
        "activities": [
            {"description": "Task Pos", "minutes": 10.0},
            {"description": "Task Missing Minutes"}
        ]
    }))
    .unwrap();
    assert_eq!(parsed.activities.len(), 2);
    assert_eq!(parsed.activities[0].minutes, 10.0);
    assert_eq!(parsed.activities[1].minutes, 0.0);

    let activities = assemble(&parsed, 120, &[]);
    let act_json = serde_json::to_value(&activities).unwrap();
    let s1 = act_json[0]
        .get("seconds")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let s2 = act_json[1]
        .get("seconds")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    assert!(
        s1 > 0 && s2 > 0,
        "Missing minutes activity must get positive seconds: s1={s1}, s2={s2}"
    );
    assert_eq!(s1 + s2, 120);
}

/// Scenario "Leitura de frentes expõe seconds e locatedSeconds com fallback para legado e autoridade sobre minutes":
#[tokio::test]
async fn workstreams_read_exposes_seconds_with_legacy_fallback() {
    let (_dir, store) = test_support::store().await;
    let day = "2026-10-04";
    query(
        "INSERT INTO day_workstreams (id, day, title, summary, first_hour, last_hour, created_at, updated_at)
         VALUES (1, ?, 'Legacy Workstream', 'Summary', '2026-10-04T10', '2026-10-04T10', '2026-10-04T10:00:00Z', '2026-10-04T10:00:00Z')",
    )
    .bind(day)
    .execute(store.pool())
    .await
    .unwrap();

    // Legacy row: periods_json tem formato legado com apenas locatedMinutes
    query(
        "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note, periods_json)
         VALUES (1, '2026-10-04T10', 25, 'Legacy note', '{\"periods\":[],\"locatedMinutes\":15}')",
    )
    .execute(store.pool())
    .await
    .unwrap();

    // Modern row: periods_json contém envelope com seconds: 40 e locatedSeconds: 40, mas minutes: 1.
    // 1 * 60 = 60, mas seconds é 40! Isso prova a autoridade de seconds sobre minutes.
    query(
        "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note, periods_json)
         VALUES (1, '2026-10-04T11', 1, 'Modern note', '{\"seconds\":40,\"locatedSeconds\":40,\"locatedMinutes\":1,\"periods\":[]}')",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let workstreams = db::workstreams_of_day(&store, day).await.unwrap();
    assert_eq!(workstreams.len(), 1);
    let ws_val = serde_json::to_value(&workstreams[0]).unwrap();

    // Total seconds for workstream:
    // Legacy: 25 min -> 25 * 60 = 1500 s
    // Modern: 40 s
    // Total = 1540 s (se houvesse bug de fallback, seria 1500 + 1 * 60 = 1560)
    assert_eq!(
        ws_val.get("seconds"),
        Some(&json!(1540)),
        "WorkstreamDto deve somar seconds reais com autoridade (1500s legado + 40s novo)"
    );

    let hours = ws_val["hours"].as_array().unwrap();
    assert_eq!(
        hours[0].get("seconds"),
        Some(&json!(1500)),
        "Hora legada deve ter fallback de seconds para minutes * 60"
    );
    assert_eq!(
        hours[0].get("locatedSeconds"),
        Some(&json!(900)),
        "Hora legada deve ter fallback de locatedSeconds para locatedMinutes * 60"
    );

    assert_eq!(
        hours[1].get("seconds"),
        Some(&json!(40)),
        "Hora moderna deve ler seconds (40) em vez de minutes * 60 (60)"
    );
    assert_eq!(
        hours[1].get("locatedSeconds"),
        Some(&json!(40)),
        "Hora moderna deve ler locatedSeconds (40) em vez de locatedMinutes * 60 (60)"
    );
}

/// Scenario "Leitura de frentes preserva zero explícito sem fallback":
#[tokio::test]
async fn workstreams_read_preserves_explicit_zero_seconds_without_fallback() {
    let (_dir, store) = test_support::store().await;
    let day = "2026-10-04";
    query(
        "INSERT INTO day_workstreams (id, day, title, summary, first_hour, last_hour, created_at, updated_at)
         VALUES (2, ?, 'Zero Workstream', 'Summary', '2026-10-04T12', '2026-10-04T12', '2026-10-04T12:00:00Z', '2026-10-04T12:00:00Z')",
    )
    .bind(day)
    .execute(store.pool())
    .await
    .unwrap();

    query(
        "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note, periods_json)
         VALUES (2, '2026-10-04T12', 5, 'Zero seconds note', '{\"seconds\":0,\"locatedSeconds\":0}')",
    )
    .execute(store.pool())
    .await
    .unwrap();

    let workstreams = db::workstreams_of_day(&store, day).await.unwrap();
    let ws_val = serde_json::to_value(&workstreams[0]).unwrap();
    assert_eq!(
        ws_val["hours"][0].get("seconds"),
        Some(&json!(0)),
        "Zero explícito não deve sofrer fallback para minutes * 60"
    );
    assert_eq!(
        ws_val["hours"][0].get("locatedSeconds"),
        Some(&json!(0)),
        "Zero explícito de locatedSeconds não deve sofrer fallback"
    );
}

/// Scenario "apply_fold grava envelope com seconds mesmo sem periods e totaliza literal":
#[tokio::test]
async fn apply_fold_stores_seconds_in_envelope_even_without_periods_and_workstreams_read_totals_literal(
) {
    let (_dir, store) = test_support::store().await;
    let day = "2026-10-04";
    let hour = "2026-10-04T14";

    query(
        "INSERT INTO day_workstreams (id, day, title, summary, first_hour, last_hour, created_at, updated_at)
         VALUES (3, ?, 'Frente Teste', 'Summary', '2026-10-04T13', '2026-10-04T13', '2026-10-04T13:00:00Z', '2026-10-04T13:00:00Z')",
    )
    .bind(day)
    .execute(store.pool())
    .await
    .unwrap();

    query(
        "INSERT INTO day_workstream_hours (workstream_id, hour, minutes, note, periods_json)
         VALUES (3, '2026-10-04T13', 10, 'Legado 10min', NULL)",
    )
    .execute(store.pool())
    .await
    .unwrap();

    query(
        "INSERT INTO day_hour_reports (hour, day, started_at, ended_at, active_minutes, summary, activities_json, distilled, distill_json, locale, provider, generated_at)
         VALUES (?, ?, '2026-10-04T14:00:00Z', '2026-10-04T15:00:00Z', 1, 'Summary', '[]', '', '{}', 'en', 'test', '2026-10-04T15:00:00Z')",
    )
    .bind(hour)
    .bind(day)
    .execute(store.pool())
    .await
    .unwrap();

    let activity_json = json!({
        "description": "Atividade de 40s",
        "minutes": 1,
        "seconds": 40
    });
    let act: HourActivity = serde_json::from_value(activity_json).unwrap();
    let prior = db::workstreams_of_day(&store, day).await.unwrap();
    let plan = plan_fold(
        &prior,
        &[act],
        &[Placement {
            workstream_id: 3,
            title: "Frente Teste".into(),
            summary: "Update".into(),
            activities: vec![1],
        }],
    );

    let folded = db::apply_fold(&store, day, hour, &plan, Utc::now())
        .await
        .unwrap();
    assert!(folded);

    let workstreams = db::workstreams_of_day(&store, day).await.unwrap();
    let ws = workstreams.iter().find(|w| w.id == 3).unwrap();
    let ws_val = serde_json::to_value(ws).unwrap();

    assert_eq!(
        ws_val.get("seconds"),
        Some(&json!(640)),
        "Totalização literal da frente deve ser 640s (600s legado + 40s da nova hora sem periods)"
    );

    let hour14 = ws_val["hours"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["hour"] == hour)
        .unwrap();
    assert_eq!(
        hour14.get("seconds"),
        Some(&json!(40)),
        "Nova hora dobrada sem periods deve ter exatamente 40s"
    );
    assert_eq!(
        hour14.get("locatedSeconds"),
        Some(&json!(0)),
        "Sem periods, locatedSeconds deve ser 0"
    );
}
