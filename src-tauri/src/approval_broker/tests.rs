//! Broker tests against an in-memory agent database with the app's
//! migrations, a recorder for emitted events, and a manual clock.

use super::*;
use crate::agent_sessions::AgentRepository;
use std::sync::atomic::AtomicUsize;
use std::sync::Mutex as StdMutex;
use tokio::sync::watch;

#[derive(Clone)]
struct TestIo {
    pool: SqlitePool,
    events: Arc<StdMutex<Vec<(String, Value)>>>,
    sequences: Arc<StdMutex<Vec<i64>>>,
}

impl BrokerIo for TestIo {
    fn pool(&self) -> BoxFuture<'_, Result<SqlitePool, AppError>> {
        Box::pin(async move { Ok(self.pool.clone()) })
    }

    fn emit(&self, _session_id: &str, _run_id: &str, sequence: i64, method: &str, data: Value) {
        self.sequences.lock().unwrap().push(sequence);
        self.events.lock().unwrap().push((method.to_string(), data));
    }
}

/// Time that only moves when the test advances it.
#[derive(Clone)]
struct ManualClock {
    now: Arc<watch::Sender<Duration>>,
}

impl ManualClock {
    fn new() -> Self {
        Self {
            now: Arc::new(watch::channel(Duration::ZERO).0),
        }
    }

    fn advance(&self, by: Duration) {
        self.now.send_modify(|now| *now += by);
    }
}

impl Clock for ManualClock {
    fn sleep(&self, duration: Duration) -> BoxFuture<'static, ()> {
        let mut now = self.now.subscribe();
        let deadline = *now.borrow() + duration;
        Box::pin(async move {
            let _ = now.wait_for(|now| *now >= deadline).await;
        })
    }
}

struct Harness {
    broker: ApprovalBroker,
    io: TestIo,
    clock: ManualClock,
    repository: AgentRepository,
    session_id: String,
    run_id: String,
}

#[tokio::test]
async fn bypass_uses_run_snapshot_and_records_a_receipt() {
    let harness = Harness::new().await;
    sqlx::query::query("UPDATE agent_sessions SET safety_mode = 'unrestricted' WHERE id = ?")
        .bind(&harness.session_id)
        .execute(&harness.io.pool)
        .await
        .unwrap();
    assert_eq!(
        harness
            .broker
            .effective_policy(&harness.turn())
            .await
            .unwrap()
            .mode,
        AgentApprovalMode::Approvals
    );
    let run = harness
        .repository
        .create_run(&harness.session_id, "__clovy_cli_engine__:claude", None)
        .await
        .unwrap();
    let mut turn = harness.turn();
    turn.safety_mode = AgentSafetyMode::Unrestricted;
    turn.run_id = run.id.clone();
    sqlx::query::query("UPDATE agent_sessions SET safety_mode = 'sandboxed' WHERE id = ?")
        .bind(&harness.session_id)
        .execute(&harness.io.pool)
        .await
        .unwrap();
    assert_eq!(
        harness.broker.effective_policy(&turn).await.unwrap().mode,
        AgentApprovalMode::Bypass
    );
    assert_eq!(
        harness
            .broker
            .request(&turn, approval("save_memory", json!({})))
            .await,
        Ok(ApprovalOutcome::Approved)
    );
    let receipt = harness.interruptions().await.pop().unwrap();
    assert_eq!(receipt["authorizationOrigin"], "session_bypass");
    assert_eq!(receipt["authorizationPolicyRunId"], run.id);
    assert_eq!(receipt["resolvedBy"], "policy");
    assert!(receipt["toolCallId"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    turn.attended = false;
    assert_eq!(
        harness.broker.effective_policy(&turn).await.unwrap().mode,
        AgentApprovalMode::Approvals
    );
    assert_eq!(
        harness
            .broker
            .request(&turn, approval("save_memory", json!({})))
            .await,
        Err(ApprovalDenial::UnattendedRequiresApproval)
    );
    turn.session_id = "other".into();
    assert_eq!(
        harness.broker.effective_policy(&turn).await,
        Err(ApprovalDenial::RunCancelled)
    );
}

#[tokio::test]
async fn bypass_does_not_authorize_when_receipt_cannot_be_written() {
    let harness = Harness::new().await;
    sqlx::query::query("UPDATE agent_sessions SET safety_mode = 'unrestricted' WHERE id = ?")
        .bind(&harness.session_id)
        .execute(&harness.io.pool)
        .await
        .unwrap();
    let run = harness
        .repository
        .create_run(&harness.session_id, "__clovy_cli_engine__:claude", None)
        .await
        .unwrap();
    sqlx::query::query("CREATE TRIGGER deny_receipt BEFORE INSERT ON agent_items BEGIN SELECT RAISE(FAIL, 'receipt unavailable'); END")
        .execute(&harness.io.pool).await.unwrap();
    let mut turn = harness.turn();
    turn.safety_mode = AgentSafetyMode::Unrestricted;
    turn.run_id = run.id;
    assert!(matches!(
        harness
            .broker
            .request(&turn, approval("save_memory", json!({})))
            .await,
        Err(ApprovalDenial::Unavailable(_))
    ));
    assert!(harness.interruptions().await.is_empty());
}

#[tokio::test]
async fn bypass_still_waits_for_clarification() {
    let harness = Harness::new().await;
    sqlx::query::query("UPDATE agent_runs SET run_config_json = json_set(run_config_json, '$.approvalMode', 'bypass') WHERE id = ?")
        .bind(&harness.run_id).execute(&harness.io.pool).await.unwrap();
    let (task, id) = harness
        .ask(ApprovalRequest {
            tool: "request_clarification".into(),
            call_id: None,
            kind: ApprovalKind::Clarification {
                question: "Which day?".into(),
                choices: vec![],
            },
        })
        .await;
    assert_eq!(harness.item(&id).await["status"], "pending");
    harness
        .resolve(&id, json!({"kind":"clarification", "answer":"Tuesday"}))
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap(),
        Ok(ApprovalOutcome::Answered("Tuesday".into()))
    );
}

#[tokio::test]
async fn legacy_or_cancelled_run_never_inherits_bypass() {
    let harness = Harness::new().await;
    sqlx::query::query("UPDATE agent_runs SET run_config_json = NULL WHERE id = ?")
        .bind(&harness.run_id)
        .execute(&harness.io.pool)
        .await
        .unwrap();
    assert_eq!(
        harness
            .broker
            .effective_policy(&harness.turn())
            .await
            .unwrap()
            .mode,
        AgentApprovalMode::Approvals
    );
    harness
        .repository
        .update_run_status(&harness.run_id, "cancelled", None, None, None)
        .await
        .unwrap();
    assert_eq!(
        harness.broker.effective_policy(&harness.turn()).await,
        Err(ApprovalDenial::RunCancelled)
    );
}

#[tokio::test]
async fn changing_safety_mode_does_not_settle_a_pending_request() {
    let harness = Harness::new().await;
    let (task, id) = harness.ask(approval("save_memory", json!({}))).await;
    sqlx::query::query("UPDATE agent_sessions SET safety_mode = 'unrestricted' WHERE id = ?")
        .bind(&harness.session_id)
        .execute(&harness.io.pool)
        .await
        .unwrap();
    assert_eq!(harness.item(&id).await["status"], "pending");
    assert!(!task.is_finished());
    harness
        .resolve(&id, json!({"kind":"approval", "choice":"deny"}))
        .await
        .unwrap();
    assert_eq!(task.await.unwrap(), Err(ApprovalDenial::UserDenied));
    let next = harness
        .repository
        .create_run(&harness.session_id, "__clovy_cli_engine__:claude", None)
        .await
        .unwrap();
    let mut turn = harness.turn();
    turn.safety_mode = AgentSafetyMode::Unrestricted;
    turn.run_id = next.id;
    assert_eq!(
        harness.broker.effective_policy(&turn).await.unwrap().mode,
        AgentApprovalMode::Bypass
    );
}

impl Harness {
    async fn new() -> Self {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory database");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let repository = AgentRepository::new(pool.clone());
        let session = repository
            .create_session(
                "CLI chat",
                "__clovy_cli_engine__:claude",
                AgentSafetyMode::Sandboxed,
                Some("/tmp"),
            )
            .await
            .unwrap();
        let run = repository
            .create_run(&session.id, "__clovy_cli_engine__:claude", None)
            .await
            .unwrap();
        repository
            .update_run_status(&run.id, "running", None, None, None)
            .await
            .unwrap();
        let io = TestIo {
            pool,
            events: Arc::default(),
            sequences: Arc::default(),
        };
        let clock = ManualClock::new();
        Self {
            broker: ApprovalBroker::new(io.clone(), clock.clone(), APPROVAL_TIMEOUT),
            io,
            clock,
            repository,
            session_id: session.id,
            run_id: run.id,
        }
    }

    fn turn(&self) -> ApprovalTurn {
        ApprovalTurn {
            session_id: self.session_id.clone(),
            run_id: self.run_id.clone(),
            cli: "claude".into(),
            safety_mode: AgentSafetyMode::Sandboxed,
            attended: true,
        }
    }

    /// Starts a request in the background and returns once its card exists.
    async fn ask(
        &self,
        request: ApprovalRequest,
    ) -> (
        tokio::task::JoinHandle<Result<ApprovalOutcome, ApprovalDenial>>,
        String,
    ) {
        let before = self.broker.waiting_ids();
        let broker = self.broker.clone();
        let turn = self.turn();
        let task = tokio::spawn(async move { broker.request(&turn, request).await });
        for _ in 0..200 {
            let pending = self.pending_items().await;
            if let Some(id) = pending
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str))
                .find(|id| !before.iter().any(|known| known == id))
            {
                return (task, id.to_string());
            }
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the request never recorded its card");
    }

    async fn interruptions(&self) -> Vec<Value> {
        self.repository
            .items(&self.session_id)
            .await
            .unwrap()
            .into_iter()
            .filter(|item| item.payload.kind() == "interruption")
            .map(|item| item.payload.value().unwrap())
            .collect()
    }

    async fn pending_items(&self) -> Vec<Value> {
        self.interruptions()
            .await
            .into_iter()
            .filter(|item| item["status"] == "pending")
            .collect()
    }

    async fn item(&self, id: &str) -> Value {
        self.interruptions()
            .await
            .into_iter()
            .find(|item| item["id"] == id)
            .expect("item exists")
    }

    async fn run_status(&self) -> String {
        self.repository.get_run(&self.run_id).await.unwrap().status
    }

    async fn resolve(&self, id: &str, resolution: Value) -> Result<bool, AppError> {
        commands::resolve(&self.broker, &self.io.pool, &self.run_id, id, &resolution).await
    }

    fn methods(&self) -> Vec<String> {
        self.io
            .events
            .lock()
            .unwrap()
            .iter()
            .map(|(method, _)| method.clone())
            .collect()
    }
}

fn approval(tool: &str, arguments: Value) -> ApprovalRequest {
    ApprovalRequest {
        tool: tool.into(),
        call_id: Some("call-1".into()),
        kind: ApprovalKind::Approval {
            title: "Create a routine".into(),
            description: "Clovy wants to create the routine \"Daily digest\".".into(),
            arguments,
            preview: None,
        },
    }
}

fn clarification() -> ApprovalRequest {
    ApprovalRequest {
        tool: "request_clarification".into(),
        call_id: None,
        kind: ApprovalKind::Clarification {
            question: "Which calendar?".into(),
            choices: vec!["Work".into(), "Personal".into()],
        },
    }
}

async fn f16_history() -> Harness {
    use crate::agent_sessions::{AgentItemPayload, MessagePayload};
    let mut harness = Harness::new().await;
    for sequence in 1..=18 {
        harness
            .repository
            .append_item(
                &harness.session_id,
                Some(&harness.run_id),
                sequence,
                &AgentItemPayload::AssistantMessage(MessagePayload {
                    role: "assistant".into(),
                    content: format!("Earlier run answer {sequence}"),
                    attachments: vec![],
                }),
                None,
            )
            .await
            .unwrap()
            .unwrap();
    }
    harness
        .repository
        .update_run_status(&harness.run_id, "completed", None, None, None)
        .await
        .unwrap();
    harness.run_id = harness
        .repository
        .create_run(&harness.session_id, "__clovy_cli_engine__:claude", None)
        .await
        .unwrap()
        .id;
    harness
        .repository
        .append_item(
            &harness.session_id,
            Some(&harness.run_id),
            1,
            &AgentItemPayload::UserMessage(MessagePayload {
                role: "user".into(),
                content: "Current request".into(),
                attachments: vec![],
            }),
            None,
        )
        .await
        .unwrap()
        .unwrap();
    harness
}

async fn f16_events(harness: &Harness, count: usize) -> Vec<(String, Value)> {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let events = harness.io.events.lock().unwrap().clone();
            if events.len() >= count {
                return events;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("broker public events")
}

async fn f16_following_item(harness: &Harness) -> crate::agent_sessions::AgentItemDto {
    use crate::agent_sessions::{AgentItemPayload, MessagePayload};
    harness
        .repository
        .append_item(
            &harness.session_id,
            Some(&harness.run_id),
            2,
            &AgentItemPayload::AssistantMessage(MessagePayload {
                role: "assistant".into(),
                content: "CLI continued after the pending card".into(),
                attachments: vec![],
            }),
            None,
        )
        .await
        .unwrap()
        .unwrap()
}

fn f16_visible_order(
    history: &[crate::agent_sessions::AgentItemDto],
    card: &Value,
    following: &crate::agent_sessions::AgentItemDto,
) -> Vec<String> {
    // Compare the public item's coordinate with real history, using the same
    // session-wide order consumed by the frontend, never timestamps or append.
    let mut order: Vec<_> = history
        .iter()
        .chain(std::iter::once(following))
        .map(|item| {
            (
                item.sequence,
                item.external_id.clone().unwrap_or_else(|| item.id.clone()),
            )
        })
        .collect();
    order.push((
        card["itemSequence"].as_i64().unwrap(),
        card["itemId"].as_str().unwrap().to_string(),
    ));
    order.sort_by_key(|(sequence, _)| *sequence);
    order.into_iter().map(|(_, id)| id).collect()
}

#[tokio::test]
async fn f16_pending_card_public_order_matches_real_history_before_hydration() {
    let harness = f16_history().await;
    let history = harness.repository.items(&harness.session_id).await.unwrap();
    let user = history.last().unwrap();
    let cursor = harness
        .repository
        .get_run(&harness.run_id)
        .await
        .unwrap()
        .last_sequence;
    assert!(
        user.sequence > cursor + 10,
        "fixture must separate item order from the new run cursor"
    );
    let (task, id) = harness
        .ask(approval("update_note", json!({"id":"synthetic-note"})))
        .await;
    let events = f16_events(&harness, 1).await;
    assert_eq!(
        harness
            .repository
            .get_run(&harness.run_id)
            .await
            .unwrap()
            .last_sequence,
        cursor
    );
    let following = f16_following_item(&harness).await;
    let public = &events[0].1;
    let persisted = harness.repository.items(&harness.session_id).await.unwrap();
    let expected: Vec<_> = persisted
        .iter()
        .map(|item| item.external_id.clone().unwrap_or_else(|| item.id.clone()))
        .collect();
    assert_eq!(
        f16_visible_order(&history, public, &following),
        expected,
        "live pending must have its canonical position before any hydration"
    );
    assert!(public["itemSequence"].as_i64().unwrap() > user.sequence);
    assert!(public["itemSequence"].as_i64().unwrap() < following.sequence);
    assert_eq!(*harness.io.sequences.lock().unwrap(), vec![cursor]);
    harness
        .resolve(&id, json!({"kind":"approval","choice":"deny"}))
        .await
        .unwrap();
    assert_eq!(task.await.unwrap(), Err(ApprovalDenial::UserDenied));
}

#[tokio::test]
async fn f16_updated_card_keeps_original_position_and_one_decision_effect() {
    for outcome in ["approve", "deny", "clarify", "retire"] {
        let harness = f16_history().await;
        sqlx::query::query("INSERT INTO notes(id,title,edited_content,processing_status,profile,created_at,updated_at,revision) VALUES ('f16-note','Synthetic','Before approval','draft','default','now','now',6)").execute(&harness.io.pool).await.unwrap();
        let history = harness.repository.items(&harness.session_id).await.unwrap();
        let (task, id) = harness
            .ask(if outcome == "clarify" {
                clarification()
            } else {
                approval("update_note", json!({"id":"f16-note"}))
            })
            .await;
        f16_events(&harness, 1).await;
        let pending = harness
            .repository
            .items(&harness.session_id)
            .await
            .unwrap()
            .pop()
            .unwrap();
        let following = f16_following_item(&harness).await;
        let revision = || async {
            use sqlx::row::Row;
            sqlx::query::query("SELECT revision FROM notes WHERE id='f16-note'")
                .fetch_one(&harness.io.pool)
                .await
                .unwrap()
                .get::<i64, _>("revision")
        };
        assert_eq!(revision().await, 6, "no effect before the decision");
        let pool = harness.io.pool.clone();
        let effect = tokio::spawn(async move {
            let result = task.await.unwrap();
            if matches!(result, Ok(ApprovalOutcome::Approved)) {
                sqlx::query::query("UPDATE notes SET edited_content='After approval',revision=revision+1 WHERE id='f16-note'").execute(&pool).await.unwrap();
            }
            result
        });
        match outcome {
            "approve" => {
                harness
                    .resolve(&id, json!({"kind":"approval","choice":"once"}))
                    .await
                    .unwrap();
            }
            "deny" => {
                harness
                    .resolve(&id, json!({"kind":"approval","choice":"deny"}))
                    .await
                    .unwrap();
            }
            "clarify" => {
                harness
                    .resolve(&id, json!({"kind":"clarification","answer":"Work"}))
                    .await
                    .unwrap();
            }
            _ => {
                assert_eq!(harness.broker.cancel_run(&harness.run_id).await, 1);
            }
        }
        let result = effect.await.unwrap();
        match outcome {
            "approve" => assert_eq!(result, Ok(ApprovalOutcome::Approved)),
            "deny" => assert_eq!(result, Err(ApprovalDenial::UserDenied)),
            "clarify" => assert_eq!(result, Ok(ApprovalOutcome::Answered("Work".into()))),
            _ => assert_eq!(result, Err(ApprovalDenial::RunCancelled)),
        }
        let events = f16_events(&harness, 2).await;
        assert_eq!(events[1].0, "interruption.updated");
        let persisted = harness.repository.items(&harness.session_id).await.unwrap();
        let updated = persisted.iter().find(|item| item.id == pending.id).unwrap();
        assert_eq!(
            (updated.sequence, &updated.external_id, &updated.created_at),
            (pending.sequence, &pending.external_id, &pending.created_at)
        );
        let expected: Vec<_> = persisted
            .iter()
            .map(|item| item.external_id.clone().unwrap_or_else(|| item.id.clone()))
            .collect();
        assert_eq!(
            f16_visible_order(&history, &events[1].1, &following),
            expected,
            "updated card must remain at its original position: {outcome}"
        );
        assert_eq!(events[0].1["itemId"], events[1].1["itemId"]);
        assert_eq!(
            harness
                .repository
                .get_run(&harness.run_id)
                .await
                .unwrap()
                .last_sequence,
            2
        );
        assert_eq!(*harness.io.sequences.lock().unwrap(), vec![1, 2]);
        let again = if outcome == "clarify" {
            json!({"kind":"clarification","answer":"Personal"})
        } else {
            json!({"kind":"approval","choice":"once"})
        };
        assert_eq!(
            harness.resolve(&id, again).await.unwrap_err().code,
            "agent_interruption_expired"
        );
        assert_eq!(revision().await, if outcome == "approve" { 7 } else { 6 });
        assert_eq!(
            f16_events(&harness, 2).await.len(),
            2,
            "one requested and one update only"
        );
    }
}

#[tokio::test]
async fn pending_request_records_the_card_the_native_engine_records() {
    let harness = Harness::new().await;
    let sequence_before = harness
        .repository
        .get_run(&harness.run_id)
        .await
        .unwrap()
        .last_sequence;
    let (task, id) = harness
        .ask(approval(
            "create_routine",
            json!({ "name": "Daily digest", "apiKey": "sk-live-abcdefghijklmnopqrstuvwxyz" }),
        ))
        .await;

    let item = harness.item(&id).await;
    assert_eq!(item["kind"], "approval");
    assert_eq!(item["engine"], "cli");
    assert_eq!(item["status"], "pending");
    assert_eq!(item["toolName"], "create_routine");
    assert_eq!(item["toolCallId"], "call-1");
    assert_eq!(item["sessionId"], harness.session_id.as_str());
    assert_eq!(item["runId"], harness.run_id.as_str());
    assert_eq!(item["allowAlways"], false);
    let command = item["command"].as_str().unwrap();
    assert!(command.starts_with("create_routine "), "{command}");
    assert!(command.contains("Daily digest"), "{command}");
    assert!(
        !command.contains("abcdefghijklmnopqrstuvwxyz"),
        "the preview is sanitized: {command}"
    );
    assert_eq!(harness.run_status().await, "waiting_for_user");
    let session = harness
        .repository
        .get_session(&harness.session_id)
        .await
        .unwrap();
    assert_eq!(session.status, "waiting_for_user");
    let events = harness.io.events.lock().unwrap().clone();
    assert_eq!(events[0].0, "interruption.requested");
    assert_eq!(
        events[0].1["itemId"],
        format!("interruption:{}:{id}", harness.run_id)
    );
    assert_eq!(events[0].1["interruption"]["id"], id.as_str());
    // The broker never moves the turn's event counter.
    assert_eq!(
        harness
            .repository
            .get_run(&harness.run_id)
            .await
            .unwrap()
            .last_sequence,
        sequence_before
    );

    harness
        .resolve(&id, json!({ "kind": "approval", "choice": "once" }))
        .await
        .unwrap();
    assert_eq!(task.await.unwrap(), Ok(ApprovalOutcome::Approved));
    let item = harness.item(&id).await;
    assert_eq!(item["status"], "resolved");
    assert_eq!(item["resolution"], "once");
    assert_eq!(item["approved"], true);
    assert_eq!(item["authorizationOrigin"], "user_once");
    assert_eq!(item["authorizationPolicyRunId"], harness.run_id);
    assert_eq!(item["resolvedBy"], "desktop");
    assert_eq!(harness.run_status().await, "running");
    assert_eq!(
        harness.methods(),
        ["interruption.requested", "interruption.updated"]
    );
}

#[tokio::test]
async fn deny_returns_user_denied() {
    let harness = Harness::new().await;
    let (task, id) = harness.ask(approval("save_memory", json!({}))).await;
    harness
        .resolve(&id, json!({ "kind": "approval", "choice": "deny" }))
        .await
        .unwrap();
    let denial = task.await.unwrap().unwrap_err();
    assert_eq!(denial.code(), "user_denied");
    let item = harness.item(&id).await;
    assert_eq!(item["status"], "resolved");
    assert_eq!(item["resolution"], "deny");
    assert_eq!(item["approved"], false);
}

#[tokio::test]
async fn session_and_always_are_rejected_without_settling() {
    for choice in ["session", "always"] {
        let harness = Harness::new().await;
        let (task, id) = harness.ask(approval("save_memory", json!({}))).await;
        let error = harness
            .resolve(&id, json!({ "kind": "approval", "choice": choice }))
            .await
            .unwrap_err();
        assert_eq!(error.code, "agent_interruption_resolution_invalid");
        assert_eq!(harness.item(&id).await["status"], "pending");
        harness
            .resolve(&id, json!({ "kind": "approval", "choice": "deny" }))
            .await
            .unwrap();
        task.await.unwrap().unwrap_err();
    }
}

/// Scenario "Ninguém responde" (unit, controlled clock).
#[tokio::test]
async fn unanswered_request_times_out_and_the_card_expires() {
    let harness = Harness::new().await;
    let (task, id) = harness.ask(approval("create_routine", json!({}))).await;

    harness
        .clock
        .advance(APPROVAL_TIMEOUT - Duration::from_secs(1));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!task.is_finished(), "still waiting before the timeout");
    assert_eq!(harness.item(&id).await["status"], "pending");

    harness.clock.advance(Duration::from_secs(1));
    let denial = task.await.unwrap().unwrap_err();
    assert_eq!(denial, ApprovalDenial::TimedOut);
    assert_eq!(denial.code(), "approval_timed_out");
    let item = harness.item(&id).await;
    assert_eq!(item["status"], "expired");
    assert_eq!(harness.run_status().await, "running");
    assert_eq!(
        harness.methods(),
        ["interruption.requested", "interruption.updated"]
    );

    // A late answer is refused without effect.
    let late = harness
        .resolve(&id, json!({ "kind": "approval", "choice": "once" }))
        .await
        .unwrap_err();
    assert_eq!(late.code, "agent_interruption_expired");
    assert_eq!(harness.item(&id).await["status"], "expired");
}

#[test]
fn approval_timeout_stays_below_the_cli_tool_timeout() {
    // design.md §3 passes 15 minutes to the CLIs (MCP_TOOL_TIMEOUT /
    // tool_timeout_sec = 900).
    assert!(APPROVAL_TIMEOUT < Duration::from_secs(900));
}

/// Scenario "Duas respostas simultâneas": the tool runs at most once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_resolutions_only_the_first_counts() {
    let harness = Arc::new(Harness::new().await);
    let executions = Arc::new(AtomicUsize::new(0));
    let (task, id) = harness.ask(approval("create_routine", json!({}))).await;
    let tool = {
        let executions = executions.clone();
        tokio::spawn(async move {
            if task.await.unwrap().is_ok() {
                executions.fetch_add(1, Ordering::SeqCst);
            }
        })
    };

    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let answers = ["once", "deny"].map(|choice| {
        let harness = harness.clone();
        let barrier = barrier.clone();
        let id = id.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            harness
                .resolve(&id, json!({ "kind": "approval", "choice": choice }))
                .await
        })
    });
    let mut results = Vec::new();
    for answer in answers {
        results.push(answer.await.unwrap());
    }
    tool.await.unwrap();

    let accepted = results.iter().filter(|result| result.is_ok()).count();
    assert_eq!(accepted, 1, "{results:?}");
    let refused = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert_eq!(refused.code, "agent_interruption_expired");
    let item = harness.item(&id).await;
    assert_eq!(item["status"], "resolved");
    let approved = item["approved"] == true;
    assert_eq!(executions.load(Ordering::SeqCst), usize::from(approved));
    assert!(executions.load(Ordering::SeqCst) <= 1);
    // One requested card and one update: the loser changed nothing.
    assert_eq!(
        harness.methods(),
        ["interruption.requested", "interruption.updated"]
    );
}

/// Scenario "Cancelar durante a espera" at the broker boundary.
#[tokio::test]
async fn cancelling_the_run_answers_run_cancelled() {
    let harness = Harness::new().await;
    let executions = AtomicUsize::new(0);
    let (task, id) = harness.ask(approval("create_routine", json!({}))).await;

    assert_eq!(harness.broker.cancel_run(&harness.run_id).await, 1);
    let result = task.await.unwrap();
    if result.is_ok() {
        executions.fetch_add(1, Ordering::SeqCst);
    }
    assert_eq!(result.unwrap_err().code(), "run_cancelled");
    assert_eq!(executions.load(Ordering::SeqCst), 0);
    assert_eq!(harness.item(&id).await["status"], "cancelled");
    let late = harness
        .resolve(&id, json!({ "kind": "approval", "choice": "once" }))
        .await
        .unwrap_err();
    assert_eq!(late.code, "agent_interruption_expired");
    // Another run's requests are untouched.
    assert_eq!(harness.broker.cancel_run("another-run").await, 0);
}

#[tokio::test]
async fn closing_the_app_cancels_waiting_requests_and_refuses_new_ones() {
    let harness = Harness::new().await;
    let (task, id) = harness.ask(approval("create_routine", json!({}))).await;
    harness.broker.close();
    assert_eq!(task.await.unwrap().unwrap_err().code(), "run_cancelled");
    for _ in 0..100 {
        if harness.item(&id).await["status"] == "cancelled" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(harness.item(&id).await["status"], "cancelled");
    let refused = harness
        .broker
        .request(&harness.turn(), approval("create_routine", json!({})))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), "run_cancelled");
}

#[tokio::test]
async fn dropping_the_waiting_tool_call_cancels_its_card() {
    let harness = Harness::new().await;
    let (task, id) = harness.ask(approval("create_routine", json!({}))).await;
    task.abort();
    let _ = task.await;
    for _ in 0..100 {
        if harness.item(&id).await["status"] == "cancelled" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(harness.item(&id).await["status"], "cancelled");
    assert!(!harness.broker.is_waiting(&id));
}

/// Reopening the app cancels CLI requests left pending by the previous
/// process and ends the run they held.
#[tokio::test]
async fn reopening_cancels_stale_cli_requests() {
    let harness = Harness::new().await;
    let (task, stale_id) = harness.ask(approval("create_routine", json!({}))).await;
    // The previous process dies: its waiter vanishes without settling.
    harness.broker.lock_waiters().clear();
    task.abort();
    let _ = task.await;
    assert_eq!(harness.item(&stale_id).await["status"], "pending");

    // A native interruption pending in another session is not the broker's.
    let native_session = harness
        .repository
        .create_session(
            "Native",
            "openrouter/auto",
            AgentSafetyMode::Sandboxed,
            Some("/tmp"),
        )
        .await
        .unwrap();
    let native_run = harness
        .repository
        .create_run(&native_session.id, "openrouter/auto", None)
        .await
        .unwrap();
    harness
        .repository
        .append_item(
            &native_session.id,
            Some(&native_run.id),
            1,
            &crate::agent_sessions::AgentItemPayload::Interruption(json!({
                "id": "native-1", "kind": "approval", "status": "pending",
                "sessionId": native_session.id, "runId": native_run.id,
            })),
            Some("interruption:native"),
        )
        .await
        .unwrap();

    let reopened = ApprovalBroker::new(harness.io.clone(), harness.clock.clone(), APPROVAL_TIMEOUT);
    let changed = store::cancel_stale(&harness.io.pool, &reopened.waiting_ids())
        .await
        .unwrap();
    assert_eq!(changed, 1);
    let item = harness.item(&stale_id).await;
    assert_eq!(item["status"], "cancelled");
    assert_eq!(item["retiredReason"], "restarted");
    assert_eq!(harness.run_status().await, "cancelled");
    let session = harness
        .repository
        .get_session(&harness.session_id)
        .await
        .unwrap();
    assert_eq!(session.status, "idle");
    let native = harness
        .repository
        .items(&native_session.id)
        .await
        .unwrap()
        .remove(0)
        .payload
        .value()
        .unwrap();
    assert_eq!(native["status"], "pending");
    let late = commands::resolve(
        &reopened,
        &harness.io.pool,
        &harness.run_id,
        &stale_id,
        &json!({ "kind": "approval", "choice": "once" }),
    )
    .await
    .unwrap_err();
    assert_eq!(late.code, "agent_interruption_expired");
}

#[tokio::test]
async fn startup_cleanup_keeps_requests_of_the_current_process() {
    let harness = Harness::new().await;
    let (task, id) = harness.ask(approval("create_routine", json!({}))).await;
    let changed = store::cancel_stale(&harness.io.pool, &harness.broker.waiting_ids())
        .await
        .unwrap();
    assert_eq!(changed, 0);
    assert_eq!(harness.item(&id).await["status"], "pending");
    harness
        .resolve(&id, json!({ "kind": "approval", "choice": "once" }))
        .await
        .unwrap();
    task.await.unwrap().unwrap();
}

/// Scenario "Rotina tenta escrever": no card, no waiting.
#[tokio::test]
async fn unattended_turn_is_denied_without_a_card() {
    let harness = Harness::new().await;
    let mut turn = harness.turn();
    turn.attended = false;
    let denial = harness
        .broker
        .request(&turn, approval("save_memory", json!({ "text": "x" })))
        .await
        .unwrap_err();
    assert_eq!(denial.code(), "unattended_requires_approval");
    assert!(harness.interruptions().await.is_empty());
    assert!(harness.methods().is_empty());
    assert_eq!(harness.run_status().await, "running");
}

/// Scenario "Pergunta respondida".
#[tokio::test]
async fn clarification_returns_exactly_the_chosen_answer() {
    let harness = Harness::new().await;
    let (task, id) = harness.ask(clarification()).await;
    let item = harness.item(&id).await;
    assert_eq!(item["kind"], "clarification");
    assert_eq!(item["engine"], "cli");
    assert_eq!(item["question"], "Which calendar?");
    assert_eq!(item["choices"], json!(["Work", "Personal"]));

    // An approval answer does not fit a question.
    let wrong = harness
        .resolve(&id, json!({ "kind": "approval", "choice": "once" }))
        .await
        .unwrap_err();
    assert_eq!(wrong.code, "agent_interruption_resolution_invalid");
    assert_eq!(harness.item(&id).await["status"], "pending");

    harness
        .resolve(
            &id,
            json!({ "kind": "clarification", "answer": "Personal" }),
        )
        .await
        .unwrap();
    assert_eq!(
        task.await.unwrap(),
        Ok(ApprovalOutcome::Answered("Personal".into()))
    );
    let item = harness.item(&id).await;
    assert_eq!(item["status"], "resolved");
    assert_eq!(item["answer"], "Personal");
}

#[tokio::test]
async fn finished_run_cannot_ask() {
    let harness = Harness::new().await;
    harness
        .repository
        .update_run_status(&harness.run_id, "cancelled", None, None, None)
        .await
        .unwrap();
    let denial = harness
        .broker
        .request(&harness.turn(), approval("save_memory", json!({})))
        .await
        .unwrap_err();
    assert_eq!(denial.code(), "run_cancelled");
    assert!(harness.interruptions().await.is_empty());
    assert!(!harness.broker.waiting_ids().iter().any(|_| true));
}

#[tokio::test]
async fn native_items_are_left_to_the_native_path() {
    let harness = Harness::new().await;
    harness
        .repository
        .append_item(
            &harness.session_id,
            Some(&harness.run_id),
            1,
            &crate::agent_sessions::AgentItemPayload::Interruption(json!({
                "id": "native-1", "kind": "approval", "status": "pending",
            })),
            Some("interruption:native"),
        )
        .await
        .unwrap();
    let handled = harness
        .resolve("native-1", json!({ "kind": "approval", "choice": "once" }))
        .await
        .unwrap();
    assert!(!handled);
}

#[test]
fn terminal_events_name_their_run() {
    assert_eq!(
        terminal_run_id(r#"{"method":"run.cancelled","runId":"r1"}"#).as_deref(),
        Some("r1")
    );
    assert_eq!(
        terminal_run_id(r#"{"method":"run.failed","runId":"r2"}"#).as_deref(),
        Some("r2")
    );
    assert_eq!(
        terminal_run_id(r#"{"method":"tool.completed","runId":"r1"}"#),
        None
    );
}
