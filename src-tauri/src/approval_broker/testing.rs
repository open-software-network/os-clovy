//! A broker for tests of write tools (the parity slices of phase 2): an
//! in-memory agent database with the app's migrations, a session and run per
//! turn, and decisions given in advance, so a tool group runs its approval
//! path without a Tauri runtime.
//!
//! ```ignore
//! let broker = TestBroker::new([Scripted::Approve, Scripted::Deny]).await;
//! let binding = broker.turn(CliKind::Claude, true).await;
//! let ctx = ToolCtx { data: &data, app: None, binding: Some(&binding), broker: Some(broker.broker()) };
//! group.call(&ctx, "save_memory", &args).await;  // first card approved
//! group.call(&ctx, "save_memory", &args).await;  // second card denied: Err(user_denied)
//! assert_eq!(broker.requests().len(), 2);
//! ```
//!
//! Each card the broker raises takes the next scripted decision (cards raised
//! one after another get them in order; simultaneous cards in no set order);
//! with the script exhausted (or [`Scripted::Leave`]) the card stays pending
//! until the run is cancelled ([`ApprovalBroker::cancel_run`]) or the
//! broker's timeout ([`TestBroker::with_timeout`]) passes.

use super::{ApprovalBroker, BrokerIo, Decision, TokioClock, APPROVAL_TIMEOUT};
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::domain::types::AppError;
use crate::llm::cli::CliKind;
use crate::mcp_server::turns::TurnBinding;
use futures_util::future::BoxFuture;
use serde_json::Value;
use sqlx_sqlite::SqlitePool;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The decision the next card gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scripted {
    Approve,
    Deny,
    /// The answer to a clarification.
    Answer(String),
    /// Nobody answers this card.
    Leave,
}

#[derive(Clone)]
struct RecordingIo {
    pool: SqlitePool,
    events: Arc<Mutex<Vec<(String, Value)>>>,
}

impl BrokerIo for RecordingIo {
    fn pool(&self) -> BoxFuture<'_, Result<SqlitePool, AppError>> {
        Box::pin(async move { Ok(self.pool.clone()) })
    }

    fn emit(&self, _session_id: &str, _run_id: &str, _sequence: i64, method: &str, data: Value) {
        self.events.lock().unwrap().push((method.to_string(), data));
    }
}

pub(crate) struct TestBroker {
    broker: ApprovalBroker,
    pool: SqlitePool,
    events: Arc<Mutex<Vec<(String, Value)>>>,
    driver: tokio::task::JoinHandle<()>,
}

impl TestBroker {
    pub async fn new(script: impl IntoIterator<Item = Scripted>) -> Self {
        Self::with_timeout(script, APPROVAL_TIMEOUT).await
    }

    pub async fn with_timeout(
        script: impl IntoIterator<Item = Scripted>,
        timeout: Duration,
    ) -> Self {
        let pool = sqlx_sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            // A cancelled acquire must not drop the sole in-memory database.
            // SQLx recommends disabling this await point for size-one fixtures.
            .test_before_acquire(false)
            .connect("sqlite::memory:")
            .await
            .expect("memory database");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        let events = Arc::default();
        let broker = ApprovalBroker::new(
            RecordingIo {
                pool: pool.clone(),
                events: Arc::clone(&events),
            },
            TokioClock,
            timeout,
        );
        let driver = tokio::spawn(drive(
            broker.clone(),
            pool.clone(),
            script.into_iter().collect(),
        ));
        Self {
            broker,
            pool,
            events,
            driver,
        }
    }

    pub fn broker(&self) -> &ApprovalBroker {
        &self.broker
    }

    /// The broker's database (the cards are `agent_items` rows there).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// A new session and running run, bound as a turn of `cli`.
    pub async fn turn(&self, cli: CliKind, attended: bool) -> TurnBinding {
        let repository = AgentRepository::new(self.pool.clone());
        let model = crate::chat_engine::engine_model_id(cli);
        let session = repository
            .create_session(
                "Test turn",
                &model,
                AgentSafetyMode::Sandboxed,
                Some("/tmp"),
            )
            .await
            .expect("session");
        let run = repository
            .create_run(&session.id, &model, None)
            .await
            .expect("run");
        repository
            .update_run_status(&run.id, "running", None, None, None)
            .await
            .expect("running run");
        TurnBinding {
            authority: crate::domain::turn_authority::TurnAuthority::new(),
            wire_names: matches!(cli, CliKind::Claude | CliKind::Codex | CliKind::Agy)
                .then(crate::mcp_server::wire_names::WireNames::default),
            session_id: session.id,
            run_id: run.id,
            data_partition: "default".into(),
            cli,
            safety_mode: AgentSafetyMode::Sandboxed,
            attended,
            surface: if attended { "chat" } else { "routine" }.into(),
        }
    }

    /// The cards raised so far (`interruption.requested` payloads), in order.
    pub fn requests(&self) -> Vec<Value> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == "interruption.requested")
            .map(|(_, data)| data["interruption"].clone())
            .collect()
    }

    /// Every event the broker emitted, as `(method, data)`.
    pub fn events(&self) -> Vec<(String, Value)> {
        self.events.lock().unwrap().clone()
    }
}

impl Drop for TestBroker {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

/// Answers each new card with the next scripted decision.
async fn drive(broker: ApprovalBroker, pool: SqlitePool, mut script: VecDeque<Scripted>) {
    let mut seen = HashSet::new();
    loop {
        let mut waiting = broker.waiting_ids();
        waiting.sort();
        for id in waiting {
            if !seen.insert(id.clone()) {
                continue;
            }
            let decision = match script.pop_front() {
                Some(Scripted::Approve) => Decision::Approve,
                Some(Scripted::Deny) => Decision::Deny,
                Some(Scripted::Answer(answer)) => Decision::Answer(answer),
                Some(Scripted::Leave) | None => continue,
            };
            let _ = broker.decide(&pool, &id, decision).await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn test_broker_answers_cards_in_script_order_and_records_them() {
    use super::{ApprovalKind, ApprovalOutcome, ApprovalRequest, ApprovalTurn};
    let broker = TestBroker::new([Scripted::Answer("Tuesday".into()), Scripted::Deny]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let turn = ApprovalTurn::from(&binding);
    let ask = |kind| ApprovalRequest {
        tool: "probe".into(),
        call_id: None,
        kind,
    };
    let answer = broker
        .broker()
        .request(
            &turn,
            ask(ApprovalKind::Clarification {
                question: "Which day?".into(),
                choices: vec!["Monday".into(), "Tuesday".into()],
            }),
        )
        .await;
    assert_eq!(answer, Ok(ApprovalOutcome::Answered("Tuesday".into())));
    let denied = broker
        .broker()
        .request(
            &turn,
            ask(ApprovalKind::Approval {
                title: "Write".into(),
                description: "Writes.".into(),
                arguments: serde_json::json!({}),
                preview: None,
            }),
        )
        .await;
    assert_eq!(denied.unwrap_err().code(), "user_denied");
    let methods = broker
        .events()
        .into_iter()
        .map(|(method, _)| method)
        .collect::<Vec<_>>();
    assert_eq!(
        methods,
        [
            "interruption.requested",
            "interruption.updated",
            "interruption.requested",
            "interruption.updated"
        ]
    );
    // The cards are rows of the broker's database, settled.
    let pending: i64 = {
        use sqlx::row::Row;
        sqlx::query::query(
            "SELECT COUNT(*) FROM agent_items WHERE kind = 'interruption' AND json_extract(payload_json, '$.status') = 'pending'",
        )
        .fetch_one(broker.pool())
        .await
        .unwrap()
        .get(0)
    };
    assert_eq!(pending, 0);
    assert_eq!(
        AgentRepository::new(broker.pool().clone())
            .get_run(&binding.run_id)
            .await
            .unwrap()
            .status,
        "running"
    );
}
