use super::*;

async fn item(pool: &SqlitePool, session: &str, run: &str, kind: &str, payload: Value) -> String {
    let id = Uuid::new_v4().to_string();
    query("INSERT INTO agent_items(id,session_id,run_id,sequence,kind,payload_json,created_at) SELECT ?,?,?,COALESCE(MAX(sequence),-1)+1,?,?,'now' FROM agent_items WHERE session_id=?")
        .bind(&id).bind(session).bind(run).bind(kind).bind(payload.to_string()).bind(session)
        .execute(pool).await.unwrap();
    id
}

fn running_call(call: &str) -> Value {
    json!({"toolName":"clovy.create_note","toolCallId":call,"arguments":{"title":"Synthetic"},"result":null,"status":"running"})
}

async fn payload(pool: &SqlitePool, id: &str) -> Value {
    let row = query("SELECT payload_json FROM agent_items WHERE id=?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
    serde_json::from_str(&row.get::<String, _>("payload_json")).unwrap()
}

#[tokio::test]
async fn stop_settles_outer_call_without_matching_broker_approval_id_or_fabricating_a_result() {
    let (file, pool) = fixture().await;
    let call = item(&pool, "session", "run", "tool_call", running_call("item_1")).await;
    let receipt = json!({"toolCallId":"8434465c-37fc-4eb0-844a-3bc0e4c192af","status":"cancelled","retiredReason":"cancelled"});
    let approval = item(&pool, "session", "run", "interruption", receipt.clone()).await;
    // The production UI Stop already owns cancellation before host/fallback publication.
    let (changed, owned) = crate::agent_sessions::api::claim_ui_run_cancellation_with_signal(
        &pool,
        "session",
        "run",
        || false,
    )
    .await
    .unwrap();
    assert!(changed && !owned);
    let event = persist_terminal_event(
        &pool,
        &frame("run.completed"),
        super::super::events::sanitize_log,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(event.method, "run.cancelled");
    assert_eq!(event.data["interruptedCallIds"], json!(["item_1"]));
    let mut expected = running_call("item_1");
    expected["status"] = json!("interrupted");
    assert_eq!(payload(&pool, &call).await, expected);
    assert_eq!(payload(&pool, &approval).await, receipt);
    assert_eq!(
        query("SELECT COUNT(*) AS n FROM agent_items WHERE kind='tool_result'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<i64, _>("n"),
        0
    );
    assert!(persist_terminal_event(
        &pool,
        &frame("run.cancelled"),
        super::super::events::sanitize_log
    )
    .await
    .unwrap()
    .is_none());
    assert_eq!(payload(&pool, &call).await, expected);
    cleanup(file, pool).await;
}

#[tokio::test]
async fn every_terminal_winner_settles_only_unresolved_calls_in_its_exact_run() {
    for winner in ["cancelled", "failed", "interrupted", "completed"] {
        let (file, pool) = fixture().await;
        query("INSERT INTO agent_sessions(id,title,model,status,created_at,updated_at) VALUES ('session-b','Other','test','running','now','now')").execute(&pool).await.unwrap();
        query("INSERT INTO agent_runs(id,session_id,model,status,started_at,updated_at) VALUES ('newer','session','test','running','now','now'),('run-b','session-b','test','completed','now','now')").execute(&pool).await.unwrap();
        let unresolved = item(&pool, "session", "run", "tool_call", running_call("item_1")).await;
        let newer = item(
            &pool,
            "session",
            "newer",
            "tool_call",
            running_call("item_1"),
        )
        .await;
        let other = item(
            &pool,
            "session-b",
            "run-b",
            "tool_call",
            running_call("item_1"),
        )
        .await;
        // Same id in another run/session must not suppress this run's settlement.
        item(
            &pool,
            "session",
            "newer",
            "tool_result",
            json!({"toolCallId":"item_1","result":"committed","status":"complete"}),
        )
        .await;
        item(
            &pool,
            "session-b",
            "run-b",
            "tool_result",
            json!({"toolCallId":"item_1","result":"committed","status":"complete"}),
        )
        .await;
        let success_call = item(
            &pool,
            "session",
            "run",
            "tool_call",
            running_call("success"),
        )
        .await;
        let success = json!({"toolCallId":"success","result":{"noteId":"committed-note"},"status":"complete"});
        let success_result = item(&pool, "session", "run", "tool_result", success.clone()).await;
        let error_call = item(&pool, "session", "run", "tool_call", running_call("error")).await;
        let error = json!({"toolCallId":"error","result":"observed error","status":"failed"});
        let error_result = item(&pool, "session", "run", "tool_result", error.clone()).await;
        let observed =
            json!({"toolCallId":"inline","result":{"committed":true},"status":"running"});
        let inline = item(&pool, "session", "run", "tool_call", observed.clone()).await;
        query("UPDATE agent_runs SET status=? WHERE id='run'")
            .bind(winner)
            .execute(&pool)
            .await
            .unwrap();
        let event = persist_terminal_event(
            &pool,
            &frame("run.completed"),
            super::super::events::sanitize_log,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            event.data["interruptedCallIds"],
            json!(["item_1"]),
            "{winner}"
        );
        assert_eq!(payload(&pool, &unresolved).await["status"], "interrupted");
        for (id, expected) in [
            (newer, running_call("item_1")),
            (other, running_call("item_1")),
            (success_call, running_call("success")),
            (error_call, running_call("error")),
            (success_result, success),
            (error_result, error),
            (inline, observed),
        ] {
            assert_eq!(payload(&pool, &id).await, expected, "{winner}");
        }
        assert_eq!(
            query("SELECT status FROM agent_runs WHERE id='newer'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<String, _>("status"),
            "running"
        );
        cleanup(file, pool).await;
    }
}

#[tokio::test]
async fn claimed_terminal_publication_still_repairs_unresolved_persistence_without_republishing() {
    let (file, pool) = fixture().await;
    let first = persist_terminal_event(
        &pool,
        &frame("run.cancelled"),
        super::super::events::sanitize_log,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(first.data.get("interruptedCallIds").is_none());
    // Historical data from a previously published terminal run must reconcile too.
    let call = item(&pool, "session", "run", "tool_call", running_call("item_1")).await;
    assert!(persist_terminal_event(
        &pool,
        &frame("run.completed"),
        super::super::events::sanitize_log
    )
    .await
    .unwrap()
    .is_none());
    assert_eq!(payload(&pool, &call).await["status"], "interrupted");
    cleanup(file, pool).await;
}

#[tokio::test]
async fn terminal_call_admission_rejects_late_starts_but_keeps_real_late_results() {
    use crate::agent_sessions::{AgentItemPayload, AgentRepository, ToolPayload};
    let (file, pool) = fixture().await;
    let call = item(&pool, "session", "run", "tool_call", running_call("item_1")).await;
    persist_terminal_event(
        &pool,
        &frame("run.cancelled"),
        super::super::events::sanitize_log,
    )
    .await
    .unwrap();
    let repo = AgentRepository::new(pool.clone());
    let tool = ToolPayload {
        error: None,
        tool_name: Some("clovy.create_note".into()),
        tool_call_id: Some("item_1".into()),
        arguments: Some(json!({"title":"Synthetic"})),
        result: None,
        status: Some("running".into()),
    };
    assert!(repo
        .append_item(
            "session",
            Some("run"),
            100,
            &AgentItemPayload::ToolCall(tool.clone()),
            Some("late-start")
        )
        .await
        .unwrap()
        .is_none());
    let late = ToolPayload {
        error: None,
        result: Some(json!({"noteId":"committed-before-stop"})),
        status: Some("complete".into()),
        ..tool
    };
    assert!(repo
        .append_item(
            "session",
            Some("run"),
            101,
            &AgentItemPayload::ToolResult(late.clone()),
            Some("late-result")
        )
        .await
        .unwrap()
        .is_some());
    let items = repo.items("session").await.unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].payload, AgentItemPayload::ToolResult(late));
    assert_eq!(items[1].sequence, 1); // conversation ordering, never replay cursor 101
    assert_eq!(payload(&pool, &call).await["status"], "interrupted");
    cleanup(file, pool).await;
}

#[tokio::test]
async fn terminal_event_never_forwards_untrusted_settlement_ids() {
    let (file, pool) = fixture().await;
    let mut proposal = frame("run.completed");
    proposal.params = Some(json!({"interruptedCallIds":["another-runs-call"]}));
    let event = persist_terminal_event(&pool, &proposal, super::super::events::sanitize_log)
        .await
        .unwrap()
        .unwrap();
    assert!(event.data.get("interruptedCallIds").is_none());
    cleanup(file, pool).await;
}

#[tokio::test]
async fn interruption_does_not_claim_an_unobserved_effect_was_rolled_back() {
    let (file, pool) = fixture().await;
    let call = item(&pool, "session", "run", "tool_call", running_call("item_1")).await;
    // The CLI can stop after an effect committed but before reporting output.
    query("INSERT INTO notes(id,title,created_at,updated_at) VALUES ('committed-note','Synthetic committed effect','now','now')").execute(&pool).await.unwrap();
    persist_terminal_event(
        &pool,
        &frame("run.cancelled"),
        super::super::events::sanitize_log,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(payload(&pool, &call).await["status"], "interrupted");
    assert!(payload(&pool, &call).await["result"].is_null());
    assert_eq!(
        query("SELECT title FROM notes WHERE id='committed-note'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<String, _>("title"),
        "Synthetic committed effect"
    );
    assert_eq!(
        query("SELECT COUNT(*) AS n FROM agent_items WHERE kind='tool_result'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<i64, _>("n"),
        0
    );
    cleanup(file, pool).await;
}

#[tokio::test]
async fn unresolved_calls_without_a_valid_outer_id_are_settled_without_publishing_a_false_identity()
{
    let (file, pool) = fixture().await;
    let mut calls = Vec::new();
    for id in [Value::Null, json!(""), json!(42)] {
        let mut call = running_call("unused");
        call["toolCallId"] = id;
        calls.push(item(&pool, "session", "run", "tool_call", call).await);
    }
    let event = persist_terminal_event(
        &pool,
        &frame("run.cancelled"),
        super::super::events::sanitize_log,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(event.data.get("interruptedCallIds").is_none());
    for call in calls {
        assert_eq!(payload(&pool, &call).await["status"], "interrupted");
    }
    cleanup(file, pool).await;
}

#[tokio::test]
async fn append_item_cannot_claim_another_sessions_run() {
    let (file, pool) = fixture().await;
    query("INSERT INTO agent_sessions(id,title,model,status,created_at,updated_at) VALUES ('session-b','Other','test','running','now','now')").execute(&pool).await.unwrap();
    let repo = crate::agent_sessions::AgentRepository::new(pool.clone());
    let tool = crate::agent_sessions::ToolPayload {
        error: None,
        tool_name: None,
        tool_call_id: Some("item_1".into()),
        arguments: None,
        result: None,
        status: Some("running".into()),
    };
    assert!(repo
        .append_item(
            "session-b",
            Some("run"),
            100,
            &crate::agent_sessions::AgentItemPayload::ToolCall(tool),
            None
        )
        .await
        .unwrap()
        .is_none());
    assert!(repo.items("session-b").await.unwrap().is_empty());
    assert_eq!(
        query("SELECT last_sequence FROM agent_runs WHERE id='run'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<i64, _>("last_sequence"),
        -1
    );
    cleanup(file, pool).await;
}

#[tokio::test]
async fn upgrade_backfills_historical_terminal_calls_without_touching_live_or_observed_evidence() {
    let (file, pool) = fixture().await;
    // Recreate an installed schema-55 database with historical terminal data.
    query("DELETE FROM schema_migrations WHERE version > 55")
        .execute(&pool)
        .await
        .unwrap();
    query("INSERT INTO agent_runs(id,session_id,model,status,started_at,updated_at) VALUES ('live','session','test','running','now','now')").execute(&pool).await.unwrap();
    let live = item(
        &pool,
        "session",
        "live",
        "tool_call",
        running_call("item_1"),
    )
    .await;
    let unresolved = item(&pool, "session", "run", "tool_call", running_call("item_1")).await;
    let mut terminal_calls = vec![unresolved];
    for status in ["failed", "completed", "interrupted"] {
        query("INSERT INTO agent_runs(id,session_id,model,status,started_at,updated_at) VALUES (?,'session','test',?,'now','now')").bind(status).bind(status).execute(&pool).await.unwrap();
        terminal_calls.push(
            item(
                &pool,
                "session",
                status,
                "tool_call",
                running_call("item_1"),
            )
            .await,
        );
    }
    let observed = item(
        &pool,
        "session",
        "run",
        "tool_call",
        running_call("success"),
    )
    .await;
    item(
        &pool,
        "session",
        "run",
        "tool_result",
        json!({"toolCallId":"success","status":"complete","result":{"noteId":"preserved"}}),
    )
    .await;
    query("UPDATE agent_runs SET status='cancelled' WHERE id='run'")
        .execute(&pool)
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    for call in terminal_calls {
        assert_eq!(payload(&pool, &call).await["status"], "interrupted");
    }
    assert_eq!(payload(&pool, &live).await, running_call("item_1"));
    assert_eq!(payload(&pool, &observed).await, running_call("success"));
    let after = crate::agent_sessions::AgentRepository::new(pool.clone())
        .items("session")
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    assert_eq!(
        crate::agent_sessions::AgentRepository::new(pool.clone())
            .items("session")
            .await
            .unwrap(),
        after
    );
    cleanup(file, pool).await;
}

async fn fixture() -> (std::path::PathBuf, SqlitePool) {
    let file = std::env::temp_dir().join(format!("clovy-terminal-{}.sqlite3", Uuid::new_v4()));
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(
            sqlx_sqlite::SqliteConnectOptions::new()
                .filename(&file)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    query("INSERT INTO agent_sessions(id,title,model,status,created_at,updated_at) VALUES ('session','Synthetic','test','running','now','now')").execute(&pool).await.unwrap();
    query("INSERT INTO agent_runs(id,session_id,model,status,started_at,updated_at,run_config_json) VALUES ('run','session','test','running','now','now','{\"engine\":\"claude\"}')").execute(&pool).await.unwrap();
    (file, pool)
}

fn frame(method: &str) -> RpcFrame {
    RpcFrame {
        jsonrpc: "2.0".into(),
        protocol_version: 1,
        session_id: "session".into(),
        run_id: "run".into(),
        sequence: 12,
        id: None,
        event_id: Some(Uuid::new_v4().to_string()),
        method: Some(method.into()),
        params: Some(
            json!({"error":"Loser failure","code":"agent_fixture","category":"tool","retryable":false}),
        ),
        result: None,
        error: None,
    }
}

async fn cleanup(file: std::path::PathBuf, pool: SqlitePool) {
    pool.close().await;
    std::fs::remove_file(file).unwrap();
}

#[tokio::test]
async fn stop_winner_replaces_late_success_or_failure_with_one_correlated_cancel() {
    for proposed in ["run.completed", "run.failed"] {
        let (file, pool) = fixture().await;
        let (changed, owned) = crate::agent_sessions::api::claim_ui_run_cancellation_with_signal(
            &pool,
            "session",
            "run",
            || true,
        )
        .await
        .unwrap();
        assert!(changed && owned);
        let proposal = frame(proposed);
        let event = persist_terminal_event(&pool, &proposal, super::super::events::sanitize_log)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.method, "run.cancelled");
        assert_eq!(event.event_id, proposal.event_id.unwrap());
        assert_eq!(event.sequence, 12);
        assert!(event.data.get("message").is_none());
        assert!(persist_terminal_event(
            &pool,
            &frame("run.cancelled"),
            super::super::events::sanitize_log
        )
        .await
        .unwrap()
        .is_none());
        let row = query("SELECT status,json_extract(run_config_json,'$.engine') AS engine FROM agent_runs WHERE id='run'").fetch_one(&pool).await.unwrap();
        assert_eq!(row.get::<String, _>("status"), "cancelled");
        assert_eq!(row.get::<String, _>("engine"), "claude");
        assert_eq!(
            query("SELECT COUNT(*) AS n FROM agent_items WHERE kind='error'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<i64, _>("n"),
            0
        );
        cleanup(file, pool).await;
    }
}

#[tokio::test]
async fn completion_winner_survives_late_stop_and_duplicate_publishers() {
    let (file, pool) = fixture().await;
    let completion = frame("run.completed");
    let event = persist_terminal_event(&pool, &completion, super::super::events::sanitize_log)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.method, "run.completed");
    let (changed, owned) = crate::agent_sessions::api::claim_ui_run_cancellation_with_signal(
        &pool,
        "session",
        "run",
        || panic!("late Stop must not signal completed run"),
    )
    .await
    .unwrap();
    assert!(!changed && !owned);
    assert!(persist_terminal_event(
        &pool,
        &frame("run.cancelled"),
        super::super::events::sanitize_log
    )
    .await
    .unwrap()
    .is_none());
    assert_eq!(
        query("SELECT status FROM agent_runs WHERE id='run'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<String, _>("status"),
        "completed"
    );
    cleanup(file, pool).await;
}

#[tokio::test]
async fn retry_copied_config_does_not_copy_the_previous_runs_publication_claim() {
    let (file, pool) = fixture().await;
    // Retry deliberately copies the previous run config. Its private terminal
    // claim belongs to that previous run and cannot suppress this run's event.
    query("UPDATE agent_runs SET run_config_json=json_set(run_config_json,'$.clovyTerminalEvent',json('{\"runId\":\"previous-run\",\"eventId\":\"previous-event\",\"method\":\"run.completed\",\"sequence\":99}')) WHERE id='run'")
        .execute(&pool).await.unwrap();
    let proposal = frame("run.completed");
    let event = persist_terminal_event(&pool, &proposal, super::super::events::sanitize_log)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.method, "run.completed");
    assert_eq!(event.event_id, proposal.event_id.unwrap());
    assert!(persist_terminal_event(
        &pool,
        &frame("run.completed"),
        super::super::events::sanitize_log
    )
    .await
    .unwrap()
    .is_none());
    assert_eq!(query("SELECT json_extract(run_config_json,'$.clovyTerminalEvent.runId') AS owner FROM agent_runs WHERE id='run'")
        .fetch_one(&pool).await.unwrap().get::<String,_>("owner"), "run");
    cleanup(file, pool).await;
}

#[tokio::test]
async fn terminal_publishers_share_one_writer_claim_and_preserve_another_live_run() {
    let (file, pool) = fixture().await;
    let call = item(&pool, "session", "run", "tool_call", running_call("item_1")).await;
    query("INSERT INTO agent_runs(id,session_id,model,status,started_at,updated_at) VALUES ('other','session','test','running','now','now')").execute(&pool).await.unwrap();
    let a = frame("run.completed");
    let b = frame("run.cancelled");
    let (a, b) = tokio::join!(
        persist_terminal_event(&pool, &a, super::super::events::sanitize_log),
        persist_terminal_event(&pool, &b, super::super::events::sanitize_log),
    );
    let events = [a.unwrap(), b.unwrap()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["interruptedCallIds"], json!(["item_1"]));
    assert_eq!(payload(&pool, &call).await["status"], "interrupted");
    assert_eq!(
        query("SELECT status FROM agent_sessions WHERE id='session'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<String, _>("status"),
        "running"
    );
    assert_eq!(
        query("SELECT status FROM agent_runs WHERE id='other'")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get::<String, _>("status"),
        "running"
    );
    cleanup(file, pool).await;
}

#[tokio::test]
async fn persisted_failure_winner_keeps_its_error_and_single_durable_card() {
    let (file, pool) = fixture().await;
    query("UPDATE agent_runs SET status='failed', completed_at='earlier',error_code='agent_original',error_message='Original password=synthetic' WHERE id='run'").execute(&pool).await.unwrap();
    let event = persist_terminal_event(
        &pool,
        &frame("run.completed"),
        super::super::events::sanitize_log,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(event.method, "run.failed");
    assert_eq!(event.data["code"], "agent_original");
    assert_eq!(event.data["message"], "Original password=[redacted]");
    assert_eq!(event.data["completedAt"], "earlier");
    assert!(persist_terminal_event(
        &pool,
        &frame("run.failed"),
        super::super::events::sanitize_log
    )
    .await
    .unwrap()
    .is_none());
    let rows =
        query("SELECT sequence,payload_json,external_id FROM agent_items WHERE kind='error'")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        event.data["itemSequence"],
        rows[0].get::<i64, _>("sequence")
    );
    assert_ne!(event.sequence, rows[0].get::<i64, _>("sequence"));
    let mut persisted_data = event.data.clone();
    persisted_data
        .as_object_mut()
        .unwrap()
        .remove("itemSequence");
    assert_eq!(
        serde_json::from_str::<Value>(&rows[0].get::<String, _>("payload_json")).unwrap(),
        persisted_data
    );
    assert_eq!(rows[0].get::<String, _>("external_id"), event.event_id);
    cleanup(file, pool).await;
}
