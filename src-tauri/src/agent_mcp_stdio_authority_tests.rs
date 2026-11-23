#![cfg(unix)]
//! Real child transport tests. Control files only schedule the synthetic MCP.
use super::*;
use sqlx_sqlite::SqlitePoolOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

struct NoSecrets;
impl McpSecretStore for NoSecrets {
    fn put(&self, _: &str, _: &McpSecretBundle) -> Result<(), AgentMcpError> {
        panic!("public fixture has no credentials")
    }
    fn get(&self, _: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        panic!("public fixture has no credentials")
    }
    fn delete(&self, _: &str) -> Result<(), AgentMcpError> {
        panic!("public fixture has no credentials")
    }
}

struct StdioFixture {
    _directory: tempfile::TempDir,
    initialized: PathBuf,
    release: PathBuf,
    calls: PathBuf,
    answers: PathBuf,
    server: McpServerDefinition,
    tool: RegisteredMcpTool,
    repository: AgentMcpRepository,
}

impl StdioFixture {
    async fn new(mode: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("mcp.py");
        let initialized = directory.path().join("initialize-received");
        let release = directory.path().join("release-initialize");
        let calls = directory.path().join("tools-call-received");
        let answers = directory.path().join("answer-received");
        std::fs::write(&script, r#"import sys, json, time
from pathlib import Path
mode, initialized, release, calls, answers = sys.argv[1:]
def emit(value):
    print(json.dumps(value), flush=True)
for line in sys.stdin:
    frame=json.loads(line)
    method=frame.get('method')
    if method=='initialize':
        Path(initialized).touch()
        while mode=='hold_initialize' and not Path(release).exists():
            time.sleep(0.005)
        emit({'jsonrpc':'2.0','id':frame['id'],'result':{'protocolVersion':'2025-06-18','capabilities':{},'serverInfo':{'name':'synthetic','version':'1'}}})
    elif method=='tools/call':
        previous=int(Path(calls).read_text()) if Path(calls).exists() else 0
        Path(calls).write_text(str(previous+1))
        if mode=='elicitation':
            emit({'jsonrpc':'2.0','id':'question','method':'elicitation/create','params':{'message':'Synthetic choice','requestedSchema':{'type':'object'}}})
            answer=sys.stdin.readline()
            if not answer:
                break
            json.loads(answer)
            Path(answers).touch()
        emit({'jsonrpc':'2.0','id':frame['id'],'result':{'content':[{'type':'text','text':'synthetic result'}]}})
"#).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let repository = AgentMcpRepository::new(pool);
        repository.ensure_schema().await.unwrap();
        let mut server = McpServerDefinition::new(
            format!("stdio-{}", Uuid::new_v4().simple()),
            McpTransport::Stdio,
        );
        server.command = Some("python3".into());
        server.args = vec![
            "-u".into(),
            script.to_string_lossy().into_owned(),
            mode.into(),
            initialized.to_string_lossy().into_owned(),
            release.to_string_lossy().into_owned(),
            calls.to_string_lossy().into_owned(),
            answers.to_string_lossy().into_owned(),
        ];
        server.safety.timeout_ms = 10_000;
        repository.create(&server).await.unwrap();
        let mut registry = McpToolRegistry::default();
        registry
            .register(
                &server,
                vec![McpDiscoveredTool {
                    name: "mutate".into(),
                    description: "Synthetic effect".into(),
                    input_schema: json!({"type":"object"}),
                    annotations: McpToolAnnotations::default(),
                }],
            )
            .unwrap();
        let tool = registry
            .resolve(&runtime_tool_name(&server.name, "mutate").unwrap())
            .unwrap()
            .clone();
        Self {
            _directory: directory,
            initialized,
            release,
            calls,
            answers,
            server,
            tool,
            repository,
        }
    }
}

async fn wait_file(path: &Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn custom_stdio_does_not_send_a_tool_after_initialize_was_invalidated() {
    let fixture = StdioFixture::new("hold_initialize").await;
    let repository = fixture.repository.clone();
    let tool = fixture.tool.clone();
    let call = tokio::spawn(async move {
        invoke_custom_mcp(&repository, &NoSecrets, &tool, json!({}), false, None, None).await
    });
    wait_file(&fixture.initialized).await;
    let repository = fixture.repository.clone();
    let id = fixture.server.id.clone();
    let disconnect =
        tokio::spawn(async move { disconnect_oauth(&repository, &NoSecrets, &id).await });
    wait_oauth_invalidation(
        &oauth_lifecycle(&fixture.server.id),
        fixture.tool.lifecycle_epoch,
    )
    .await;
    std::fs::write(&fixture.release, "ready").unwrap();
    let result = call.await.unwrap();
    let outcome = disconnect.await.unwrap().unwrap();
    retire_server_sessions(&fixture.server.id).await;
    assert!(
        !fixture.calls.exists(),
        "no mutating frame may leave after initialization was cancelled"
    );
    assert!(matches!(result, Err(AgentMcpError::ConnectionChanged)));
    assert!(outcome.local_cleared);
}

#[tokio::test]
async fn custom_stdio_elicitation_cannot_write_an_answer_after_waiting_on_a_stale_slot() {
    let fixture = StdioFixture::new("elicitation").await;
    let result = invoke_custom_mcp(
        &fixture.repository,
        &NoSecrets,
        &fixture.tool,
        json!({}),
        false,
        None,
        None,
    )
    .await;
    assert!(matches!(result, Err(AgentMcpError::ElicitationRequired(_))));
    let key = session_key(&fixture.server, None);
    let shared = MCP_SESSIONS
        .get()
        .unwrap()
        .lock()
        .await
        .get(&key)
        .unwrap()
        .clone();
    let slot = shared.lock().await;
    let repository = fixture.repository.clone();
    let tool = fixture.tool.clone();
    let resume = tokio::spawn(async move {
        invoke_custom_mcp(
            &repository,
            &NoSecrets,
            &tool,
            json!({}),
            false,
            None,
            Some("choice"),
        )
        .await
    });
    let lifecycle = oauth_lifecycle(&fixture.server.id);
    tokio::time::timeout(Duration::from_secs(5), async {
        while lifecycle.gate.try_lock().is_ok() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    invalidate_oauth(&lifecycle);
    drop(slot);
    let result = resume.await.unwrap();
    retire_server_sessions(&fixture.server.id).await;
    assert_eq!(std::fs::read_to_string(&fixture.calls).unwrap(), "1");
    assert!(
        !fixture.answers.exists(),
        "an expired capability cannot resume an already delivered effect"
    );
    assert!(matches!(result, Err(AgentMcpError::DeliveryUncertain(_))));
}

#[derive(Default)]
struct ResumeSecrets {
    values: Mutex<BTreeMap<String, McpSecretBundle>>,
    unreadable: AtomicBool,
}
impl McpSecretStore for ResumeSecrets {
    fn put(&self, key: &str, bundle: &McpSecretBundle) -> Result<(), AgentMcpError> {
        self.values
            .lock()
            .unwrap()
            .insert(key.into(), bundle.clone());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<McpSecretBundle>, AgentMcpError> {
        if self.unreadable.load(Ordering::SeqCst) {
            return Err(AgentMcpError::SecureStorage);
        }
        Ok(self.values.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<(), AgentMcpError> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[tokio::test]
async fn custom_stdio_unreadable_vault_on_resume_preserves_cause_and_prior_delivery_uncertainty() {
    let mut fixture = StdioFixture::new("elicitation").await;
    let store = ResumeSecrets::default();
    let reference = format!("resume-fixture-{}", Uuid::new_v4());
    fixture.server.secret_ref = Some(reference.clone());
    fixture.repository.replace(&fixture.server).await.unwrap();
    let mut bundle = McpSecretBundle::default();
    bundle.env.insert(
        "MCP_RESUME_FIXTURE_KEY".into(),
        "synthetic-resume-key".into(),
    );
    store.put(&reference, &bundle).unwrap();
    let mut registry = McpToolRegistry::default();
    registry
        .register(
            &fixture.server,
            vec![McpDiscoveredTool {
                name: "mutate".into(),
                description: "Synthetic effect".into(),
                input_schema: json!({"type":"object"}),
                annotations: McpToolAnnotations::default(),
            }],
        )
        .unwrap();
    fixture.tool = registry
        .resolve(&runtime_tool_name(&fixture.server.name, "mutate").unwrap())
        .unwrap()
        .clone();
    let first = invoke_custom_mcp(
        &fixture.repository,
        &store,
        &fixture.tool,
        json!({}),
        false,
        None,
        None,
    )
    .await;
    assert!(matches!(first, Err(AgentMcpError::ElicitationRequired(_))));
    store.unreadable.store(true, Ordering::SeqCst);
    let error = invoke_custom_mcp(
        &fixture.repository,
        &store,
        &fixture.tool,
        json!({}),
        false,
        None,
        Some("choice"),
    )
    .await
    .unwrap_err();
    let diagnostic = error.diagnostic(&fixture.server.name, Some("mutate"), "call");
    assert_eq!(diagnostic.code, "agent_mcp_secure_storage_failed");
    assert_eq!(diagnostic.phase, "refresh");
    assert_eq!(diagnostic.action, "inspect_server_before_retry");
    assert!(diagnostic.message.contains("may have been applied"));
    assert!(diagnostic
        .message
        .contains("secure storage operation failed"));
    let final_error = diagnostic.into_app_error();
    assert!(final_error.message.contains("may have been applied"));
    assert!(final_error
        .message
        .contains("agent_mcp_secure_storage_failed"));
    assert_eq!(std::fs::read_to_string(&fixture.calls).unwrap(), "1");
    assert!(!fixture.answers.exists());
    retire_server_sessions(&fixture.server.id).await;
}

#[tokio::test]
async fn custom_cli_lost_elicitation_session_keeps_delivery_uncertain_without_replay() {
    let fixture = StdioFixture::new("elicitation").await;
    in_cli_turn("synthetic-lost-elicitation", async {
        let first = invoke_custom_mcp(
            &fixture.repository,
            &NoSecrets,
            &fixture.tool,
            json!({}),
            false,
            None,
            None,
        )
        .await;
        assert!(matches!(first, Err(AgentMcpError::ElicitationRequired(_))));
        retire_server_sessions(&fixture.server.id).await;
        let resumed = invoke_custom_mcp(
            &fixture.repository,
            &NoSecrets,
            &fixture.tool,
            json!({}),
            false,
            None,
            Some("choice"),
        )
        .await;
        assert!(matches!(resumed, Err(AgentMcpError::DeliveryUncertain(_))));
        assert_eq!(std::fs::read_to_string(&fixture.calls).unwrap(), "1");
        assert!(!fixture.answers.exists());
    })
    .await;
}
