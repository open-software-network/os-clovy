use super::{call, descriptors, ActivityAccess, McpData, ToolCtx};
use crate::approval_broker::testing::{Scripted, TestBroker};
use crate::llm::cli::CliKind;
use serde_json::json;

#[tokio::test]
async fn routines_crud_approved_and_persisted_for_both_clis() {
    for cli in [CliKind::Claude, CliKind::Codex] {
        let broker = TestBroker::new(std::iter::repeat_n(Scripted::Approve, 5)).await;
        let binding = broker.turn(cli, true).await;
        let data = McpData {
            notes: broker.pool().clone(),
            profile: "default".into(),
            memory_enabled: true,
            activity: ActivityAccess::Off,
            now: chrono::Utc::now(),
        };
        let ctx = ToolCtx {
            data: &data,
            app: None,
            binding: Some(&binding),
            broker: Some(broker.broker()),
        };
        let created = call(&ctx, "create_routine", &json!({"prompt":"Summarize yesterday", "schedule":"every 1d", "safetyMode":"sandboxed"})).await.unwrap();
        let id = created["id"].as_str().unwrap();
        let args = json!({"routineId":id});
        call(
            &ctx,
            "update_routine",
            &json!({"routineId":id,"name":"Daily recap"}),
        )
        .await
        .unwrap();
        assert_eq!(
            call(&ctx, "pause_routine", &args).await.unwrap()["state"],
            "paused"
        );
        assert_eq!(
            call(&ctx, "resume_routine", &args).await.unwrap()["state"],
            "scheduled"
        );
        let listed = call(&ctx, "list_routines", &json!({})).await.unwrap();
        assert_eq!(listed[0]["name"], "Daily recap");
        assert_eq!(listed[0]["id"], id);
        call(&ctx, "delete_routine", &args).await.unwrap();
        assert_eq!(
            call(&ctx, "list_routines", &json!({})).await.unwrap(),
            json!([])
        );
        assert_eq!(broker.requests().len(), 5);
    }
}

#[tokio::test]
async fn routines_denied_unattended_unbound_and_unavailable_leave_database_unchanged() {
    let broker = TestBroker::new(std::iter::repeat_n(Scripted::Deny, 5)).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let routine = crate::routines::create(
        broker.pool(),
        serde_json::from_value(json!({"prompt":"Read notes", "schedule":"every 1d"})).unwrap(),
    )
    .await
    .unwrap();
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let create_args = json!({"name":"Changed", "prompt":"Changed", "schedule":"every 2d","safetyMode":"sandboxed"});
    let unattended = broker.turn(CliKind::Codex, false).await;
    for name in [
        "create_routine",
        "update_routine",
        "pause_routine",
        "resume_routine",
        "delete_routine",
    ] {
        let args = if name == "create_routine" {
            create_args.clone()
        } else if name == "update_routine" {
            json!({"routineId":routine.id,"name":"Changed","prompt":"Changed","schedule":"every 2d","safetyMode":"sandboxed"})
        } else {
            json!({"routineId":routine.id})
        };
        assert_eq!(
            call(&ctx, name, &args).await.unwrap_err().code,
            "user_denied"
        );
        assert_eq!(
            call(
                &ToolCtx {
                    binding: Some(&unattended),
                    ..ctx
                },
                name,
                &args
            )
            .await
            .unwrap_err()
            .code,
            "routine_tool_not_enabled"
        );
        assert_eq!(
            call(&ToolCtx::unbound(&data), name, &args)
                .await
                .unwrap_err()
                .code,
            "turn_required"
        );
        assert_eq!(
            call(
                &ToolCtx {
                    broker: None,
                    ..ctx
                },
                name,
                &args
            )
            .await
            .unwrap_err()
            .code,
            "approval_unavailable"
        );
    }
    assert_eq!(
        crate::routines::get(broker.pool(), &routine.id)
            .await
            .unwrap()
            .name,
        routine.name
    );
    assert_eq!(crate::routines::list(broker.pool()).await.unwrap().len(), 1);
    assert_eq!(broker.requests().len(), 5);
    for name in [
        "list_routines",
        "create_routine",
        "update_routine",
        "pause_routine",
        "resume_routine",
        "delete_routine",
    ] {
        assert!(!descriptors(&ToolCtx::unbound(&data))
            .await
            .iter()
            .any(|tool| tool["name"] == name));
        let tools = descriptors(&ctx).await;
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        assert_eq!(tool["annotations"]["readOnlyHint"], name == "list_routines");
    }
}

#[tokio::test]
async fn routine_history_pages_and_claim_reports_busy_or_blocked() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let routine = crate::routines::create(broker.pool(), serde_json::from_value(json!({"prompt":"Read notes","schedule":"every 1d","timezone":"America/Sao_Paulo","enabledToolsets":["skills"]})).unwrap()).await.unwrap();
    let claim = crate::routines::claim(broker.pool(), &routine.id, "manual", false)
        .await
        .unwrap()
        .unwrap();
    assert!(
        crate::routines::claim(broker.pool(), &routine.id, "manual", false)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        crate::routines::trigger_blocked_error(broker.pool(), &routine.id)
            .await
            .unwrap()
            .code,
        "routine_busy"
    );
    let data = McpData {
        notes: broker.pool().clone(),
        profile: "default".into(),
        memory_enabled: true,
        activity: ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = ToolCtx {
        binding: Some(&binding),
        ..ToolCtx::unbound(&data)
    };
    let page = call(
        &ctx,
        "list_routine_runs",
        &json!({"routineId":routine.id,"limit":1}),
    )
    .await
    .unwrap();
    assert_eq!(page["items"][0]["id"], claim.routine_run_id);
    assert_eq!(page["hasMore"], false);
    let page = call(
        &ctx,
        "list_routine_runs",
        &json!({"routineId":routine.id,"offset":1}),
    )
    .await
    .unwrap();
    assert_eq!(page["items"], json!([]));
    let paused = crate::routines::create(
        broker.pool(),
        serde_json::from_value(json!({"prompt":"Paused","schedule":"every 1d","state":"paused"}))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        crate::routines::trigger_blocked_error(broker.pool(), &paused.id)
            .await
            .unwrap()
            .code,
        "routine_blocked"
    );
    assert_eq!(crate::routines::create(broker.pool(),serde_json::from_value(json!({"prompt":"Paused invalid timezone","schedule":"every 1d","state":"paused","timezone":"invalid"})).unwrap()).await.unwrap_err().code,"routine_timezone_invalid");
}

#[tokio::test]
async fn stopped_turn_cannot_mutate_routines_or_admit_a_claim() {
    let broker = TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let routine = crate::routines::create(
        broker.pool(),
        serde_json::from_value(json!({"prompt":"Keep","schedule":"every 1d"})).unwrap(),
    )
    .await
    .unwrap();
    sqlx::query::query("UPDATE agent_runs SET status='cancelled' WHERE id=?")
        .bind(&binding.run_id)
        .execute(broker.pool())
        .await
        .unwrap();
    let turn = Some((
        "default",
        binding.session_id.as_str(),
        binding.run_id.as_str(),
        &binding.authority,
    ));
    assert_eq!(
        crate::routines::create_for_turn(
            broker.pool(),
            serde_json::from_value(json!({"prompt":"Denied","schedule":"every 1d"})).unwrap(),
            turn
        )
        .await
        .unwrap_err()
        .code,
        "turn_expired"
    );
    assert_eq!(
        crate::routines::update_for_turn(
            broker.pool(),
            serde_json::from_value(json!({"routineId":routine.id,"name":"Denied"})).unwrap(),
            turn
        )
        .await
        .unwrap_err()
        .code,
        "turn_expired"
    );
    assert_eq!(
        crate::routines::pause_for_turn(broker.pool(), &routine.id, turn)
            .await
            .unwrap_err()
            .code,
        "turn_expired"
    );
    assert_eq!(
        crate::routines::resume_for_turn(broker.pool(), &routine.id, turn)
            .await
            .unwrap_err()
            .code,
        "turn_expired"
    );
    assert_eq!(
        crate::routines::delete_for_turn(broker.pool(), &routine.id, turn)
            .await
            .unwrap_err()
            .code,
        "turn_expired"
    );
    assert_eq!(
        crate::routines::list(broker.pool()).await.unwrap(),
        vec![routine]
    );
}
