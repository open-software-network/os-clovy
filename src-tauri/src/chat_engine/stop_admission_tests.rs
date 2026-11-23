//! Production Stop claims and broker cancellation against real SQLite/IPC.
//! No direct authority.revoke call is used. No hardware or provider is opened;
//! observable counters exercise the production effect gates and helper IPC is real.

use super::{cancel_cli_run, ChatEngineHost};
use crate::approval_broker::{
    ApprovalBroker, ApprovalDenial, ApprovalKind, ApprovalRequest, ApprovalTurn, BrokerIo,
    TokioClock, APPROVAL_TIMEOUT,
};
use crate::domain::turn_authority::TurnAuthority;
use crate::domain::types::AppError;
use crate::mcp_server::turns::{TurnBinding, TurnRegistry};
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::query::query;
use sqlx_sqlite::SqlitePool;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Clone)]
struct SettlementIo {
    pool: SqlitePool,
    requested: Arc<Notify>,
    entered_settlement: Arc<Notify>,
    release_settlement: Arc<Notify>,
    pause_settlement: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<String>>>,
}
impl BrokerIo for SettlementIo {
    fn pool(&self) -> BoxFuture<'_, Result<SqlitePool, AppError>> {
        Box::pin(async move {
            // A semantic I/O boundary, not a count of incidental queries.
            if self.pause_settlement.load(Ordering::Acquire) {
                self.entered_settlement.notify_one();
                self.release_settlement.notified().await;
            }
            Ok(self.pool.clone())
        })
    }
    fn emit(&self, _: &str, _: &str, _: i64, method: &str, _: Value) {
        self.events.lock().unwrap().push(method.into());
        if method == "interruption.requested" {
            self.requested.notify_one();
        }
    }
}

struct HeldLock {
    release: Option<mpsc::Sender<()>>,
    holder: Option<std::thread::JoinHandle<()>>,
}
impl HeldLock {
    fn new<T: Send + 'static>(mutex: Arc<Mutex<T>>) -> Self {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _held = mutex.lock().unwrap();
            entered_tx.send(()).unwrap();
            let _ = wait.recv();
        });
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        Self {
            release: Some(release),
            holder: Some(holder),
        }
    }
    fn release(mut self) {
        self.release.take().unwrap().send(()).unwrap();
        self.holder.take().unwrap().join().unwrap();
    }
}
impl Drop for HeldLock {
    fn drop(&mut self) {
        self.release.take();
        if let Some(holder) = self.holder.take() {
            holder.join().unwrap();
        }
    }
}

fn blocked_capture(
    binding: &TurnBinding,
    system: bool,
) -> (
    HeldLock,
    std::thread::JoinHandle<Result<(), AppError>>,
    Arc<AtomicUsize>,
) {
    let readiness = Arc::new(Mutex::new(()));
    let held = HeldLock::new(readiness.clone());
    let authority = binding.authority.clone();
    let gate: crate::audio::capture::CaptureStartAuthorization =
        Arc::new(move || authority.check_live());
    let effects = Arc::new(AtomicUsize::new(0));
    let count = effects.clone();
    let worker = std::thread::spawn(move || {
        let _prepared = readiness.lock().unwrap();
        let effect = || {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        if system {
            crate::audio::system_audio::admit_system_audio_start(Some(&gate), effect)
        } else {
            let mut state = crate::audio::capture::CaptureStartState::Pending;
            crate::audio::capture::admit_capture_start(&mut state, false, Some(&gate), effect)
        }
    });
    (held, worker, effects)
}

fn blocked_helper(
    binding: &TurnBinding,
    generation_wait: bool,
) -> (
    HeldLock,
    std::thread::JoinHandle<Result<(), AppError>>,
    UnixStream,
) {
    let (writer, reader) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let process = Arc::new(Mutex::new(writer));
    let generation = Arc::new(Mutex::new(()));
    let held = if generation_wait {
        HeldLock::new(generation.clone())
    } else {
        HeldLock::new(process.clone())
    };
    let authority = binding.authority.clone();
    let worker = std::thread::spawn(move || {
        let _generation = generation.lock().unwrap();
        let mut process = process.lock().unwrap();
        crate::dictation::write_helper_command(
            &mut *process,
            &json!({"type":"start_listening","takeId":"stop-production"}),
            Some(&authority),
        )
    });
    (held, worker, reader)
}

async fn fixture() -> (
    SqlitePool,
    SettlementIo,
    ApprovalBroker,
    TurnBinding,
    TurnBinding,
    TurnRegistry,
    Arc<ChatEngineHost>,
) {
    assert!(std::env::var_os("OS_CLOVY_USE_PROD_DATA_DIR").is_none());
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .test_before_acquire(false)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    for id in ["caller", "target"] {
        query("INSERT INTO agent_sessions(id,title,model,status,created_at,updated_at) VALUES (?,?,'test','running','now','now')").bind(id).bind(id).execute(&pool).await.unwrap();
        query("INSERT INTO agent_runs(id,session_id,model,status,started_at,updated_at,run_config_json) VALUES (?,?,'test','running','now','now','{\"dataPartition\":\"default\"}')").bind(format!("{id}-run")).bind(id).execute(&pool).await.unwrap();
    }
    let binding = |id: &str| TurnBinding {
        authority: TurnAuthority::new(),
        wire_names: None,
        session_id: id.into(),
        run_id: format!("{id}-run"),
        data_partition: "default".into(),
        cli: crate::llm::cli::CliKind::Claude,
        safety_mode: crate::agent_sessions::domain::AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "chat".into(),
    };
    let caller = binding("caller");
    let target = binding("target");
    let turns = TurnRegistry::default();
    turns.issue(caller.clone());
    turns.issue(target.clone());
    let io = SettlementIo {
        pool: pool.clone(),
        requested: Arc::default(),
        entered_settlement: Arc::default(),
        release_settlement: Arc::default(),
        pause_settlement: Arc::new(AtomicBool::new(false)),
        events: Arc::default(),
    };
    let broker = ApprovalBroker::new(io.clone(), TokioClock, APPROVAL_TIMEOUT);
    (
        pool,
        io,
        broker,
        caller,
        target,
        turns,
        Arc::new(ChatEngineHost::default()),
    )
}

#[derive(Clone, Copy)]
enum StopPath {
    Ui,
    Remote,
    Direct,
}

async fn stopped_while_broker_settlement_waits(path: StopPath) {
    let (pool, io, broker, caller, target, turns, host) = fixture().await;
    let ticket = host.register(&target.run_id).unwrap();
    let request = {
        let broker = broker.clone();
        let turn = ApprovalTurn::from(&target);
        tokio::spawn(async move {
            broker
                .request(
                    &turn,
                    ApprovalRequest {
                        tool: "ask_user".into(),
                        call_id: None,
                        kind: ApprovalKind::Clarification {
                            question: "Synthetic pending clarification".into(),
                            choices: vec![],
                        },
                    },
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(5), io.requested.notified())
        .await
        .unwrap();
    let mut probes = Vec::new();
    for system in [false, true] {
        probes.push(blocked_capture(&target, system));
    }
    let mut helpers = Vec::new();
    for generation in [false, true] {
        helpers.push(blocked_helper(&target, generation));
    }
    // An already admitted capture is a recorder-owned operation: Stop must
    // prevent new admission without undoing this publication (U4).
    let mut published = crate::audio::capture::CaptureStartState::Pending;
    let mut admitted = 0;
    let admitted_authority = target.authority.clone();
    let admitted_gate: crate::audio::capture::CaptureStartAuthorization =
        Arc::new(move || admitted_authority.check_live());
    crate::audio::capture::admit_capture_start(&mut published, false, Some(&admitted_gate), || {
        admitted += 1;
        Ok(())
    })
    .unwrap();
    io.pause_settlement.store(true, Ordering::Release);
    match path {
        StopPath::Ui => assert_eq!(
            crate::agent_sessions::api::claim_ui_run_cancellation_for_host(
                &pool,
                &target.session_id,
                &target.run_id,
                &host,
                Some(&turns)
            )
            .await
            .unwrap(),
            (true, true)
        ),
        StopPath::Remote => assert_eq!(
            crate::agent_sessions::api::claim_conversation_run_cancellation_for_host(
                &pool,
                "default",
                &caller.session_id,
                &caller.run_id,
                &target.session_id,
                &target.run_id,
                &caller.authority,
                &host,
                Some(&turns)
            )
            .await
            .unwrap(),
            (true, true)
        ),
        StopPath::Direct => {}
    }
    let cleanup = {
        let host = host.clone();
        let broker = broker.clone();
        let turns = turns.clone();
        let run = target.run_id.clone();
        tokio::spawn(async move { cancel_cli_run(&host, Some(&broker), Some(&turns), &run).await })
    };
    tokio::time::timeout(Duration::from_secs(5), io.entered_settlement.notified())
        .await
        .unwrap();
    assert!(!cleanup.is_finished());
    assert!(!request.is_finished());
    let durable: String =
        sqlx::query_scalar::query_scalar("SELECT status FROM agent_runs WHERE id=?")
            .bind(&target.run_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        durable,
        if matches!(path, StopPath::Direct) {
            "waiting_for_user"
        } else {
            "cancelled"
        }
    );
    for (held, worker, mut reader) in helpers {
        held.release();
        let result = worker.join().unwrap();
        let mut bytes = [0; 1024];
        match reader.read(&mut bytes) {
            Ok(0) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Ok(count) => panic!(
                "a helper start escaped production Stop: {}",
                String::from_utf8_lossy(&bytes[..count])
            ),
            Err(error) => {
                panic!("failed to inspect helper commands after production Stop: {error}")
            }
        }
        assert_eq!(result.unwrap_err().code, "turn_expired");
    }
    for (held, worker, effects) in probes {
        held.release();
        assert_eq!(worker.join().unwrap().unwrap_err().code, "turn_expired");
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }
    assert!(*ticket.cancel.borrow());
    assert!(!turns.contains_binding(&target));
    assert!(turns.contains_binding(&caller));
    assert!(caller.authority.check_live().is_ok());
    assert_eq!(
        published,
        crate::audio::capture::CaptureStartState::Published
    );
    assert_eq!(admitted, 1);
    // Receipt settlement is still blocked after native gates have refused.
    assert!(!cleanup.is_finished());
    assert!(!request.is_finished());
    let pending:i64=sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_items WHERE run_id=? AND json_extract(payload_json,'$.status')='pending'").bind(&target.run_id).fetch_one(&pool).await.unwrap();
    assert_eq!(pending, 1);
    io.release_settlement.notify_one();
    assert!(tokio::time::timeout(Duration::from_secs(5), cleanup)
        .await
        .unwrap()
        .unwrap());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), request)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err(),
        ApprovalDenial::RunCancelled
    );
    let cancelled:i64=sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_items WHERE run_id=? AND json_extract(payload_json,'$.status')='cancelled'").bind(&target.run_id).fetch_one(&pool).await.unwrap();
    assert_eq!(cancelled, 1);
    assert_eq!(
        io.events
            .lock()
            .unwrap()
            .iter()
            .filter(|name| name.as_str() == "interruption.updated")
            .count(),
        1
    );
}

#[tokio::test]
async fn ui_stop_revokes_before_broker_settlement_and_blocks_capture_and_helper_start() {
    stopped_while_broker_settlement_waits(StopPath::Ui).await;
}
#[tokio::test]
async fn remote_stop_revokes_before_broker_settlement_and_preserves_caller() {
    stopped_while_broker_settlement_waits(StopPath::Remote).await;
}
#[tokio::test]
async fn direct_cancel_revokes_before_broker_settlement() {
    stopped_while_broker_settlement_waits(StopPath::Direct).await;
}

#[tokio::test]
async fn live_production_binding_releases_one_capture_and_one_helper_command() {
    let (pool, _, broker, _, target, turns, host) = fixture().await;
    let _ticket = host.register(&target.run_id).unwrap();
    for system in [false, true] {
        let (held, worker, count) = blocked_capture(&target, system);
        held.release();
        worker.join().unwrap().unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }
    for generation in [false, true] {
        let (held, worker, mut reader) = blocked_helper(&target, generation);
        held.release();
        worker.join().unwrap().unwrap();
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        let command: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(command["type"], "start_listening");
        assert_eq!(command["takeId"], "stop-production");
        assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    }
    assert_eq!(
        crate::agent_sessions::api::claim_ui_run_cancellation_for_host(
            &pool,
            &target.session_id,
            &target.run_id,
            &host,
            Some(&turns)
        )
        .await
        .unwrap(),
        (true, true)
    );
    assert!(cancel_cli_run(&host, Some(&broker), Some(&turns), &target.run_id).await);
}

#[tokio::test]
async fn a_stop_that_loses_to_completion_does_not_signal_or_revoke() {
    let (pool, _, _, _, target, turns, host) = fixture().await;
    let ticket = host.register(&target.run_id).unwrap();
    query("UPDATE agent_runs SET status='completed' WHERE id=?")
        .bind(&target.run_id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        crate::agent_sessions::api::claim_ui_run_cancellation_for_host(
            &pool,
            &target.session_id,
            &target.run_id,
            &host,
            Some(&turns)
        )
        .await
        .unwrap(),
        (false, false)
    );
    assert!(turns.contains_binding(&target));
    assert!(!*ticket.cancel.borrow());
}
