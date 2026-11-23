//! Bind transport/catalog fixtures through the host's constructor, keeping
//! authority and the frozen data partition identical to a real CLI turn.
use clovy_lib::agent_sessions::domain::AgentSafetyMode;
use clovy_lib::chat_engine::turn::TurnContext;
use clovy_lib::llm::cli::CliKind;
use clovy_lib::mcp_server::turns::TurnBinding;

pub fn binding(cli: CliKind, attended: bool, session: &str, run: &str) -> TurnBinding {
    TurnContext {
        session_id: session.into(),
        run_id: run.into(),
        data_partition: "default".into(),
        previous_run_id: None,
        kind: cli,
        program: std::path::PathBuf::new(),
        env: Default::default(),
        agy: None,
        workspace: std::path::PathBuf::new(),
        input: String::new(),
        mcp: None,
        turns: None,
        safety_mode: AgentSafetyMode::Sandboxed,
        attended,
        surface: if attended { "chat" } else { "routine" }.into(),
        routine_tools: None,
        images: Vec::new(),
    }
    .binding()
}
