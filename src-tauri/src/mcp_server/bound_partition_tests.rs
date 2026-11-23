use super::{bound_profile, protocol, tools, turns};
use crate::agent_sessions::{AgentRepository, AgentSafetyMode};
use crate::approval_broker::testing::TestBroker;
use crate::db::repositories::Repositories;
use crate::llm::cli::CliKind;
use serde_json::json;

#[tokio::test]
async fn issued_token_rejects_reassignment_before_listing_reading_or_context_publication() {
    let broker = TestBroker::new([]).await;
    let pool = broker.pool().clone();
    let repo = AgentRepository::new(pool.clone());
    let a = repo
        .create_session_in_profile("A visible", "test", AgentSafetyMode::Sandboxed, None, "A")
        .await
        .unwrap();
    let b = repo
        .create_session_in_profile("B sentinel", "test", AgentSafetyMode::Sandboxed, None, "B")
        .await
        .unwrap();
    let run = repo.create_run(&a.id, "test", None).await.unwrap();
    let binding = turns::TurnBinding {
        authority: crate::domain::turn_authority::TurnAuthority::new(),
        wire_names: None,
        session_id: a.id.clone(),
        run_id: run.id.clone(),
        data_partition: crate::agent_sessions::run_partition::resolve(&pool, &a.id, &run.id)
            .await
            .unwrap(),
        cli: CliKind::Claude,
        safety_mode: AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "home".into(),
    };
    let registry = turns::TurnRegistry::default();
    let token = registry.issue(binding);
    let binding = registry.binding(&token).unwrap();
    let mut data = super::tests::data(pool.clone(), tools::ActivityAccess::Off);
    data.profile = bound_profile(&pool, &binding).await.unwrap();
    let ctx = tools::ToolCtx {
        data: &data,
        app: None,
        binding: Some(&binding),
        broker: Some(broker.broker()),
    };
    let before = tools::call(&ctx, "list_conversations", &json!({}))
        .await
        .unwrap();
    assert!(before.to_string().contains("A visible"));
    assert!(!before.to_string().contains("B sentinel"));
    Repositories::new(pool.clone())
        .assign_session_to_profile(&a.id, "B")
        .await
        .unwrap();
    assert_eq!(
        bound_profile(&pool, &binding).await.unwrap_err().code,
        "turn_expired"
    );
    for (tool, args) in [
        ("list_conversations", json!({})),
        ("get_conversation", json!({"conversationId":b.id})),
    ] {
        assert_eq!(
            tools::call(&ctx, tool, &args).await.unwrap_err().code,
            "turn_expired"
        );
    }
    let context = protocol::handle_in(&ctx, &json!({"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":"clovy://context"}})).await.unwrap();
    assert!(context.get("error").is_some(), "{context}");
    assert!(!context.to_string().contains("B sentinel"));
    assert_eq!(repo.get_session(&b.id).await.unwrap().title, "B sentinel");
    registry.revoke(&token);
    assert_eq!(
        bound_profile(&pool, &binding).await.unwrap_err().code,
        "turn_expired"
    );
}
