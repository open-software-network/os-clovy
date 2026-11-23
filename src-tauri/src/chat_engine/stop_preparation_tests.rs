//! Real host, SQLite terminal arbiter, executor and OS child sentinel.
use super::{
    turn::{execute_turn, TurnContext},
    ChatEngineHost, FrameSink,
};
use crate::{
    agent_sessions::{protocol::RpcFrame, AgentRepository, AgentSafetyMode},
    llm::cli::CliKind,
};
use sqlx::{query::query, row::Row};
use std::{
    collections::BTreeMap,
    sync::{atomic::Ordering, Mutex},
};

struct TerminalSink {
    pool: sqlx_sqlite::SqlitePool,
    events: Mutex<Vec<String>>,
}
impl FrameSink for TerminalSink {
    async fn publish(&self, frame: RpcFrame) {
        if matches!(
            frame.method.as_deref(),
            Some("run.completed" | "run.failed" | "run.cancelled")
        ) {
            if let Some(event) = crate::agent_sessions::terminal::persist_terminal_event(
                &self.pool,
                &frame,
                crate::agent_sessions::events::sanitize_log,
            )
            .await
            .unwrap()
            {
                self.events.lock().unwrap().push(event.method);
            }
        }
    }
}
async fn fixture() -> (AgentRepository, String, String) {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(2)
        .test_before_acquire(false)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let repo = AgentRepository::new(pool);
    let session = repo
        .create_session(
            "Synthetic",
            "__clovy_cli_engine__:claude",
            AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let run = repo
        .create_run(&session.id, "__clovy_cli_engine__:claude", None)
        .await
        .unwrap();
    (repo, session.id, run.id)
}
fn context(session: &str, run: &str, dir: &std::path::Path) -> TurnContext {
    use std::os::unix::fs::PermissionsExt;
    let program = dir.join("synthetic-cli");
    std::fs::write(&program,"#!/bin/sh\nprintf launched > launch\ncat > prompt\nprintf '%s\\n' '{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"Synthetic answer\"}'\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    TurnContext {
        session_id: session.into(),
        run_id: run.into(),
        data_partition: "default".into(),
        previous_run_id: None,
        kind: CliKind::Claude,
        program,
        env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        agy: None,
        workspace: dir.into(),
        input: "Synthetic private prompt".into(),
        mcp: None,
        turns: None,
        safety_mode: AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "home".into(),
        routine_tools: None,
        images: vec![],
    }
}
async fn claim_stop(repo: &AgentRepository, host: &ChatEngineHost, session: &str, run: &str) {
    let (changed, owned) = crate::agent_sessions::api::claim_ui_run_cancellation_for_host(
        &repo.pool, session, run, host, None,
    )
    .await
    .unwrap();
    assert!(
        changed && owned,
        "the registered host must own terminal publication"
    );
}
async fn assert_cancelled_once(repo: &AgentRepository, run: &str, sink: &TerminalSink) {
    assert_eq!(*sink.events.lock().unwrap(), ["run.cancelled"]);
    let row=query("SELECT status,json_extract(run_config_json,'$.clovyTerminalEvent.method') AS method FROM agent_runs WHERE id=?").bind(run).fetch_one(&repo.pool).await.unwrap();
    assert_eq!(row.get::<String, _>("status"), "cancelled");
    assert_eq!(row.get::<String, _>("method"), "run.cancelled");
    let errors: i64 = sqlx::query_scalar::query_scalar(
        "SELECT COUNT(*) FROM agent_items WHERE run_id=? AND kind='error'",
    )
    .bind(run)
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(errors, 0);
}

#[tokio::test]
async fn stop_after_final_partition_read_prevents_real_child_and_prompt() {
    let (repo, session, run) = fixture().await;
    let host = ChatEngineHost::default();
    let ticket = host.register(&run).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let context = context(&session, &run, dir.path());
    let sink = TerminalSink {
        pool: repo.pool.clone(),
        events: Mutex::default(),
    };
    let count = super::turn::testing::watch_spawns(&run);
    let (read, resume) = crate::agent_sessions::run_partition::testing::pause_after_read(&run);
    let stop = async {
        read.await.unwrap();
        claim_stop(&repo, &host, &session, &run).await;
        resume.send(()).unwrap();
    };
    tokio::join!(execute_turn(&repo, &sink, context, ticket.cancel), stop);
    assert_cancelled_once(&repo, &run, &sink).await;
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "ProcessTree::spawn ran after Stop"
    );
    assert!(!dir.path().join("launch").exists());
    assert!(!dir.path().join("prompt").exists());
    super::turn::testing::clear_spawns(&run);
    host.release(&run);
}

#[tokio::test]
async fn stopped_preparation_failure_publishes_the_winner_before_releasing_owner() {
    for invalid_snapshot in [false, true] {
        let (repo, session, run) = fixture().await;
        let host = ChatEngineHost::default();
        let ticket = host.register(&run).unwrap();
        let sink = TerminalSink {
            pool: repo.pool.clone(),
            events: Mutex::default(),
        };
        let dir = tempfile::tempdir().unwrap();
        let count = super::turn::testing::watch_spawns(&run);
        let resolution = if invalid_snapshot {
            query("UPDATE agent_runs SET run_config_json='{}' WHERE id=?")
                .bind(&run)
                .execute(&repo.pool)
                .await
                .unwrap();
            let (read, resume) =
                crate::agent_sessions::run_partition::testing::pause_after_read(&run);
            let stop = async {
                read.await.unwrap();
                claim_stop(&repo, &host, &session, &run).await;
                resume.send(()).unwrap();
            };
            let (result, ()) = tokio::join!(
                crate::agent_sessions::run_partition::resolve(&repo.pool, &session, &run),
                stop
            );
            result
        } else {
            let (preparing, prepared) = tokio::sync::oneshot::channel();
            let (resume, resumed) = tokio::sync::oneshot::channel();
            let prepare = async {
                preparing.send(()).unwrap();
                resumed.await.unwrap();
                crate::agent_sessions::run_partition::resolve(&repo.pool, &session, &run).await
            };
            let stop = async {
                prepared.await.unwrap();
                claim_stop(&repo, &host, &session, &run).await;
                resume.send(()).unwrap();
            };
            let (result, ()) = tokio::join!(prepare, stop);
            result
        };
        let error = resolution.unwrap_err();
        assert_eq!(error.code, "turn_expired");
        drop(ticket);
        let result = super::finish_dispatch(&host, &sink, &session, &run, Err(error)).await;
        assert_eq!(result.unwrap_err().code, "turn_expired");
        assert_cancelled_once(&repo, &run, &sink).await;
        // A fallback/duplicate publisher must use the same arbiter.
        let duplicate = crate::agent_sessions::protocol::RpcFrame {
            jsonrpc: "2.0".into(),
            protocol_version: 1,
            session_id: session.clone(),
            run_id: run.clone(),
            sequence: 0,
            id: None,
            event_id: None,
            method: Some("run.cancelled".into()),
            params: Some(serde_json::json!({})),
            result: None,
            error: None,
        };
        sink.publish(duplicate).await;
        assert_cancelled_once(&repo, &run, &sink).await;
        assert!(!host.cancel(&run));
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert!(!dir.path().join("launch").exists());
        super::turn::testing::clear_spawns(&run);
    }
}

#[tokio::test]
async fn stop_wins_a_dispatch_error_waiting_on_the_real_terminal_publisher() {
    struct PublishingSink {
        inner: TerminalSink,
        ready: tokio::sync::Notify,
        resume: tokio::sync::Notify,
    }
    impl FrameSink for PublishingSink {
        async fn publish(&self, frame: RpcFrame) {
            self.ready.notify_one();
            self.resume.notified().await;
            self.inner.publish(frame).await;
        }
    }
    let (repo, session, run) = fixture().await;
    let host = ChatEngineHost::default();
    let ticket = host.register(&run).unwrap();
    let sink = PublishingSink {
        inner: TerminalSink {
            pool: repo.pool.clone(),
            events: Mutex::default(),
        },
        ready: Default::default(),
        resume: Default::default(),
    };
    let finish = super::finish_dispatch(
        &host,
        &sink,
        &session,
        &run,
        Err(crate::domain::types::AppError::new(
            "turn_expired",
            "Synthetic resolution failure",
        )),
    );
    let stop = async {
        sink.ready.notified().await;
        claim_stop(&repo, &host, &session, &run).await;
        sink.resume.notify_one();
    };
    let (result, ()) = tokio::join!(finish, stop);
    assert_eq!(result.unwrap_err().code, "turn_expired");
    assert_cancelled_once(&repo, &run, &sink.inner).await;
    drop(ticket);
    assert!(!host.cancel(&run));
}

#[tokio::test]
async fn final_partition_error_after_stop_remains_cancelled_without_child() {
    let (repo, session, run) = fixture().await;
    let host = ChatEngineHost::default();
    let ticket = host.register(&run).unwrap();
    // Valid JSON with missing authority passes get_run but the real resolver
    // refuses it after its read barrier. Stop must take priority over that Err.
    query("UPDATE agent_runs SET run_config_json='{}' WHERE id=?")
        .bind(&run)
        .execute(&repo.pool)
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let context = context(&session, &run, dir.path());
    let sink = TerminalSink {
        pool: repo.pool.clone(),
        events: Mutex::default(),
    };
    let count = super::turn::testing::watch_spawns(&run);
    let (read, resume) = crate::agent_sessions::run_partition::testing::pause_after_read(&run);
    let stop = async {
        read.await.unwrap();
        claim_stop(&repo, &host, &session, &run).await;
        resume.send(()).unwrap();
    };
    tokio::join!(execute_turn(&repo, &sink, context, ticket.cancel), stop);
    assert_cancelled_once(&repo, &run, &sink).await;
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(!dir.path().join("prompt").exists());
    super::turn::testing::clear_spawns(&run);
    host.release(&run);
}

#[tokio::test]
async fn no_stop_still_runs_the_real_child_and_partition_change_still_refuses_it() {
    for moved in [false, true] {
        let (repo, session, run) = fixture().await;
        let host = ChatEngineHost::default();
        let ticket = host.register(&run).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let turn = context(&session, &run, dir.path());
        if moved {
            crate::db::repositories::Repositories::new(repo.pool.clone())
                .assign_session_to_profile(&session, "B")
                .await
                .unwrap();
        }
        let sink = TerminalSink {
            pool: repo.pool.clone(),
            events: Mutex::default(),
        };
        let count = super::turn::testing::watch_spawns(&run);
        execute_turn(&repo, &sink, turn, ticket.cancel).await;
        if moved {
            assert_eq!(*sink.events.lock().unwrap(), ["run.failed"]);
            assert_eq!(count.load(Ordering::SeqCst), 0);
            assert!(!dir.path().join("prompt").exists());
            let next = repo
                .create_run(&session, "__clovy_cli_engine__:claude", None)
                .await
                .unwrap();
            let mut fresh = context(&session, &next.id, dir.path());
            fresh.data_partition =
                crate::agent_sessions::run_partition::resolve(&repo.pool, &session, &next.id)
                    .await
                    .unwrap();
            let next_host = ChatEngineHost::default();
            let ticket = next_host.register(&next.id).unwrap();
            let next_sink = TerminalSink {
                pool: repo.pool.clone(),
                events: Mutex::default(),
            };
            execute_turn(&repo, &next_sink, fresh, ticket.cancel).await;
            assert_eq!(*next_sink.events.lock().unwrap(), ["run.completed"]);
            next_host.release(&next.id);
        } else {
            assert_eq!(*sink.events.lock().unwrap(), ["run.completed"]);
            assert_eq!(count.load(Ordering::SeqCst), 1);
            assert_eq!(
                std::fs::read_to_string(dir.path().join("launch")).unwrap(),
                "launched"
            );
            assert!(std::fs::read_to_string(dir.path().join("prompt"))
                .unwrap()
                .contains("Synthetic private prompt"));
        }
        super::turn::testing::clear_spawns(&run);
        host.release(&run);
    }
}

#[tokio::test]
async fn preparation_error_without_stop_publishes_one_failure_and_releases_host() {
    let (repo, session, run) = fixture().await;
    let host = ChatEngineHost::default();
    let ticket = host.register(&run).unwrap();
    crate::db::repositories::Repositories::new(repo.pool.clone())
        .assign_session_to_profile(&session, "B")
        .await
        .unwrap();
    let error = crate::agent_sessions::run_partition::resolve(&repo.pool, &session, &run)
        .await
        .unwrap_err();
    let sink = TerminalSink {
        pool: repo.pool.clone(),
        events: Mutex::default(),
    };
    drop(ticket);
    assert_eq!(
        super::finish_dispatch(&host, &sink, &session, &run, Err(error))
            .await
            .unwrap_err()
            .code,
        "turn_expired"
    );
    assert_eq!(*sink.events.lock().unwrap(), ["run.failed"]);
    assert!(!host.cancel(&run));
    let errors: i64 = sqlx::query_scalar::query_scalar(
        "SELECT COUNT(*) FROM agent_items WHERE run_id=? AND kind='error'",
    )
    .bind(&run)
    .fetch_one(&repo.pool)
    .await
    .unwrap();
    assert_eq!(errors, 1);
}
