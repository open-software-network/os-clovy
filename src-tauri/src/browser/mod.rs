//! Clovy's private, pinned agent-browser and public-web connection policy.
pub mod engine;
pub mod policy;
pub mod proxy;
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

use crate::{
    browser_broker::{
        BrowserBroker, BrowserTransport, BrowserTransportKind, TransportArtifact, TransportFuture,
        TransportResponse,
    },
    domain::types::AppError,
};
use base64::Engine as _;
use engine::{AgentBrowserTransport, EngineConfig, EngineError};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Manager;
fn map_error(e: EngineError) -> AppError {
    AppError::new(e.code, e.message)
}
struct Transport(Arc<AgentBrowserTransport>);

#[cfg(test)]
pub(crate) fn test_transport(config: EngineConfig) -> Arc<dyn BrowserTransport> {
    Arc::new(Transport(AgentBrowserTransport::new(config)))
}

impl BrowserTransport for Transport {
    fn reserve_session(&self, id: &str) -> Result<(), AppError> {
        self.0.reserve(id).map_err(map_error)
    }
    fn execute<'a>(&'a self, tool: &'a str, arguments: Value) -> TransportFuture<'a> {
        Box::pin(async move {
            let id = arguments["session_id"]
                .as_str()
                .ok_or_else(|| AppError::new("invalid_arguments", "session_id is required."))?;
            let mut data = self
                .0
                .execute(id, tool, arguments.clone())
                .await
                .map_err(map_error)?;
            if tool == "screenshot" {
                let encoded = data["base64"].as_str().ok_or_else(|| {
                    AppError::new("browser_artifact_invalid", "The browser returned no image.")
                })?;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| {
                        AppError::new(
                            "browser_artifact_invalid",
                            "The browser returned an invalid image.",
                        )
                    })?;
                if let Some(object) = data.as_object_mut() {
                    object.remove("base64");
                }
                Ok(TransportResponse::artifact(
                    data,
                    TransportArtifact {
                        kind: "screenshot".into(),
                        mime_type: "image/png".into(),
                        bytes,
                    },
                ))
            } else {
                Ok(TransportResponse::data(data))
            }
        })
    }
    fn terminate_session(&self, id: &str) {
        self.0.terminate(id);
    }
    fn terminate_session_for_shutdown(
        &self,
        id: &str,
        deadline: crate::shutdown::ShutdownDeadline,
    ) {
        self.0.terminate_with_grace(id, deadline.remaining());
    }
    fn session_state(&self, id: &str) -> Result<Value, AppError> {
        self.0.state(id).map_err(map_error)
    }
    fn human_takeover<'a>(&'a self, id: &'a str) -> TransportFuture<'a> {
        Box::pin(async move {
            self.0
                .takeover(id)
                .await
                .map(TransportResponse::data)
                .map_err(map_error)
        })
    }
    fn resume_control<'a>(&'a self, id: &'a str) -> TransportFuture<'a> {
        Box::pin(async move {
            self.0.resume(id).await.map_err(map_error)?;
            Ok(TransportResponse::data(json!({"resumed":true})))
        })
    }
}

/// Initialization completes before publication to the MCP dispatch path;
/// missing resources fail at startup of a session, never select another engine.
pub(crate) fn setup(app: &tauri::AppHandle) -> Result<(), AppError> {
    let data = crate::app_paths::app_data_dir(app).map_err(|_| {
        AppError::new(
            "browser_unavailable",
            "Browser data storage is unavailable.",
        )
    })?;
    let bundled = app
        .path()
        .resource_dir()
        .map_err(|_| AppError::new("browser_unavailable", "Browser resources are unavailable."))?
        .join("native/agent-browser");
    let resources = if cfg!(debug_assertions) && !bundled.is_dir() {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../resources/agent-browser")
    } else {
        bundled
    };
    let broker = Arc::clone(app.state::<Arc<BrowserBroker>>().inner());
    broker.set_feature_enabled(crate::experimental_settings::browser_use_enabled(app));
    let weak = Arc::downgrade(&broker);
    app.state::<crate::mcp_server::McpServerState>()
        .turns()
        .add_revoke_hook(Arc::new(move |run| {
            if let Some(broker) = weak.upgrade() {
                broker.terminate_run(run);
            }
        }));
    broker.set_access_flag_path(data.join("browser-access"));
    let screenshot_root = data.join("browser-artifacts/images");
    let artifact_root = data.join("browser-artifacts/snapshots");
    for (kind, persistent) in [
        (BrowserTransportKind::Attended, true),
        (BrowserTransportKind::Managed, false),
    ] {
        let config = EngineConfig::production(resources.clone(), data.clone(), persistent);
        if kind == BrowserTransportKind::Attended {
            let namespace = config.chrome_namespace().ok_or_else(|| {
                AppError::new("browser_unavailable", "Browser resources are unavailable.")
            })?;
            app.state::<crate::computer_use::ComputerUseState>()
                .set_browser_runtime(
                    crate::computer_use_owned_runtime::OwnedBrowserRuntime::new(
                        namespace,
                        config.chrome.clone(),
                    ),
                )?;
        }
        let engine = tauri::async_runtime::block_on(async { AgentBrowserTransport::new(config) });
        broker.configure_transport(
            kind,
            Arc::new(Transport(engine)),
            screenshot_root.clone(),
            artifact_root.clone(),
        );
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        match crate::commands::repositories(&handle).await {
            Ok(repositories) => {
                if let Ok(grants) = repositories.list_routine_browser_grants().await {
                    for grant in grants {
                        broker.set_routine_grant(crate::browser_broker::RoutineBrowserGrant {
                            job_id: grant.job_id,
                            enabled: grant.enabled,
                        });
                    }
                }
                broker.configure_outcome_repository(repositories);
            }
            Err(error) => {
                tracing::error!(code=%error.code,"browser outcome repository initialization failed")
            }
        }
    });
    Ok(())
}
