use crate::domain::types::{AppError, RecordingSourceMode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use tokio::sync::oneshot;

const REQUEST_EVENT: &str = "clovy://agent-recorder-request";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(420);
const COMPLETED_CAP: usize = 32;

#[derive(Default)]
pub struct AgentRecorderBroker {
    pending: Mutex<HashMap<String, PendingRecorderRequest>>,
    completed: Mutex<VecDeque<String>>,
}

struct PendingRecorderRequest {
    sender: oneshot::Sender<ResolveAgentRecorderRequest>,
    target: AgentRecorderTarget,
    action: AgentRecorderAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentRecorderAction {
    Start,
    Stop,
    Pause,
    Resume,
    Recover,
    Retry,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRecorderTarget {
    #[serde(skip)]
    pub(crate) binding: Option<crate::mcp_server::turns::TurnBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_action: Option<RecordingRecoveryAction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingRecoveryAction {
    Validate,
    Discard,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentRecorderRequestPayload {
    request_id: String,
    action: AgentRecorderAction,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_mode: Option<RecordingSourceMode>,
    #[serde(flatten)]
    target: AgentRecorderTarget,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveAgentRecorderRequest {
    pub request_id: String,
    pub ok: bool,
    #[serde(default)]
    pub note_id: Option<String>,
    #[serde(default)]
    pub note_title: Option<String>,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub error_details: Option<serde_json::Value>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub status: Option<crate::domain::types::RecordingStatusDto>,
}

struct PendingRecorderLease<'a> {
    broker: &'a AgentRecorderBroker,
    request_id: String,
}
impl Drop for PendingRecorderLease<'_> {
    fn drop(&mut self) {
        self.broker.remove_pending(&self.request_id);
    }
}

impl AgentRecorderBroker {
    pub async fn request(
        &self,
        app: &AppHandle,
        action: AgentRecorderAction,
        source_mode: Option<RecordingSourceMode>,
        target: AgentRecorderTarget,
    ) -> Result<serde_json::Value, AppError> {
        self.request_with(action, source_mode, target, |payload| {
            app.get_webview_window("main")
                .ok_or_else(|| {
                    AppError::new(
                        "agent_recorder_window_unavailable",
                        "The Clovy window is unavailable.",
                    )
                })
                .and_then(|window| {
                    window.emit(REQUEST_EVENT, payload).map_err(|error| {
                        AppError::new("agent_recorder_emit_failed", error.to_string())
                    })
                })
        })
        .await
    }

    /// [`Self::request`] with the delivery to the frontend given by the
    /// caller (`emit` receives the `clovy://agent-recorder-request` payload),
    /// so tests can answer as the frontend does through [`Self::resolve`].
    pub(crate) async fn request_with(
        &self,
        action: AgentRecorderAction,
        source_mode: Option<RecordingSourceMode>,
        target: AgentRecorderTarget,
        emit: impl FnOnce(serde_json::Value) -> Result<(), AppError>,
    ) -> Result<serde_json::Value, AppError> {
        let request_id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().map_err(|_| unavailable())?.insert(
            request_id.clone(),
            PendingRecorderRequest {
                sender,
                target: target.clone(),
                action,
            },
        );
        let _lease = PendingRecorderLease {
            broker: self,
            request_id: request_id.clone(),
        };
        let payload = AgentRecorderRequestPayload {
            request_id: request_id.clone(),
            action,
            source_mode,
            target: target.clone(),
        };
        let payload = serde_json::to_value(payload)
            .map_err(|error| AppError::new("agent_recorder_emit_failed", error.to_string()))?;
        if let Err(error) = emit(payload) {
            self.remove_pending(&request_id);
            return Err(error);
        }
        let resolution = match tokio::time::timeout(REQUEST_TIMEOUT, receiver).await {
            Ok(Ok(resolution)) => resolution,
            Ok(Err(_)) => {
                let mut error=AppError::new("agent_recorder_cancelled","The recorder request ended after delivery and before Clovy answered. Inspect its target before retrying.");
                error.details = Some(
                    serde_json::json!({"target":target,"delivery":"unknown","effect":"unknown","retrySafe":false}),
                );
                return Err(error);
            }
            Err(_) => {
                self.remove_pending(&request_id);
                let mut error=AppError::new("agent_recorder_timeout","The recorder request timed out after delivery. Inspect its target before retrying.");
                error.details = Some(
                    serde_json::json!({"target":target,"delivery":"unknown","effect":"unknown","retrySafe":false}),
                );
                return Err(error);
            }
        };
        if !resolution.ok {
            let mut error = AppError::new(
                resolution
                    .error_code
                    .unwrap_or_else(|| "agent_recorder_failed".into()),
                resolution
                    .error_message
                    .unwrap_or_else(|| "The recorder request failed.".into()),
            );
            error.details = resolution.error_details;
            return Err(error);
        }
        if target
            .note_id
            .as_ref()
            .is_some_and(|id| resolution.note_id.as_ref() != Some(id))
            || target
                .session_id
                .as_ref()
                .is_some_and(|id| resolution.session_id.as_ref() != Some(id))
        {
            return Err(uncertain_response(
                &target,
                "agent_recorder_target_mismatch",
                "The recorder response does not match the authorized note and recording session.",
            ));
        }
        if let Some(status) = resolution.status.as_ref() {
            if resolution.session_id.as_deref() != Some(status.session_id.as_str())
                || resolution.note_id != status.note_id
            {
                return Err(uncertain_response(
                    &target,
                    "agent_recorder_target_mismatch",
                    "The recorder metadata does not match its response target.",
                ));
            }
        }
        if matches!(
            action,
            AgentRecorderAction::Start | AgentRecorderAction::Pause | AgentRecorderAction::Resume
        ) && resolution.status.is_none()
        {
            return Err(uncertain_response(
                &target,
                "agent_recorder_metadata_missing",
                "The recorder did not confirm its session, source and duration metadata.",
            ));
        }
        Ok(serde_json::json!({
            "success": true,
            "noteId": resolution.note_id,
            "noteTitle": resolution.note_title,
            "sessionId": resolution.session_id,
            "status": resolution.status,
        }))
    }

    pub(crate) fn validate_target(
        &self,
        request_id: &str,
        profile: &str,
        note_id: &str,
    ) -> Result<(), AppError> {
        let pending = self.pending.lock().map_err(|_| unavailable())?;
        let entry = pending.get(request_id).ok_or_else(|| {
            AppError::new(
                "agent_recorder_request_not_found",
                "The recorder request expired or its chat turn was cancelled.",
            )
        })?;
        if entry.action == AgentRecorderAction::Start
            && entry.target.profile.as_deref() == Some(profile)
            && entry.target.note_id.as_deref() == Some(note_id)
        {
            return Ok(());
        }
        Err(AppError::new(
            "agent_recorder_target_mismatch",
            "The recording target differs from the authorized data partition and note.",
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn saved_authorization(
        &self,
        request_id: &str,
        profile: &str,
        note_id: &str,
        session_id: &str,
        action: AgentRecorderAction,
        recovery_action: Option<RecordingRecoveryAction>,
    ) -> Result<crate::mcp_server::turns::TurnBinding, AppError> {
        let pending = self.pending.lock().map_err(|_| unavailable())?;
        let entry = pending.get(request_id).ok_or_else(|| {
            AppError::new(
                "agent_recorder_request_not_found",
                "The recorder request expired or its chat turn was cancelled.",
            )
        })?;
        if entry.action != action
            || entry.target.profile.as_deref() != Some(profile)
            || entry.target.note_id.as_deref() != Some(note_id)
            || entry.target.session_id.as_deref() != Some(session_id)
            || entry.target.recovery_action != recovery_action
        {
            return Err(AppError::new(
                "agent_recorder_target_mismatch",
                "The saved recording target or action differs from the approved request.",
            ));
        }
        entry.target.binding.clone().ok_or_else(|| {
            AppError::new(
                "turn_expired",
                "The recorder request has no controlling chat turn.",
            )
        })
    }

    pub(crate) fn request_authorization(
        &self,
        request_id: &str,
    ) -> Result<(crate::mcp_server::turns::TurnBinding, String), AppError> {
        let pending = self.pending.lock().map_err(|_| unavailable())?;
        let entry = pending.get(request_id).ok_or_else(|| {
            AppError::new(
                "agent_recorder_request_not_found",
                "The recorder request expired or its chat turn was cancelled.",
            )
        })?;
        let binding = entry.target.binding.clone().ok_or_else(|| {
            AppError::new(
                "turn_expired",
                "The recorder request has no controlling turn.",
            )
        })?;
        let profile = entry.target.profile.clone().ok_or_else(|| {
            AppError::new(
                "agent_recorder_target_invalid",
                "The recorder request has no bound data partition.",
            )
        })?;
        Ok((binding, profile))
    }

    pub(crate) fn authorization_for_start(
        &self,
        request_id: &str,
        profile: &str,
        note_id: &str,
    ) -> Result<crate::mcp_server::turns::TurnBinding, AppError> {
        self.validate_target(request_id, profile, note_id)?;
        self.request_authorization(request_id)
            .map(|(binding, _)| binding)
    }

    pub(crate) fn validate_request(&self, request_id: &str) -> Result<(), AppError> {
        if self
            .pending
            .lock()
            .map_err(|_| unavailable())?
            .contains_key(request_id)
        {
            return Ok(());
        }
        Err(AppError::new(
            "agent_recorder_request_not_found",
            "The recorder request expired or its chat turn was cancelled.",
        ))
    }

    fn remove_pending(&self, request_id: &str) {
        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(request_id);
        }
    }

    pub(crate) fn resolve(&self, request: ResolveAgentRecorderRequest) -> Result<(), AppError> {
        let request_id = request.request_id.clone();
        let sender = self
            .pending
            .lock()
            .map_err(|_| unavailable())?
            .remove(&request_id);
        let Some(sender) = sender else {
            if self
                .completed
                .lock()
                .map(|completed| completed.iter().any(|id| id == &request_id))
                .unwrap_or(false)
            {
                return Ok(());
            }
            return Err(AppError::new(
                "agent_recorder_request_not_found",
                "The recorder request was not found or has already timed out.",
            ));
        };
        sender.sender.send(request).map_err(|_| {
            AppError::new(
                "agent_recorder_request_not_found",
                "The recorder request was not found or has already timed out.",
            )
        })?;
        if let Ok(mut completed) = self.completed.lock() {
            completed.push_back(request_id);
            while completed.len() > COMPLETED_CAP {
                completed.pop_front();
            }
        }
        Ok(())
    }
}

fn uncertain_response(target: &AgentRecorderTarget, code: &str, message: &str) -> AppError {
    let mut error = AppError::new(code, message);
    error.details = Some(
        serde_json::json!({"target":target,"delivery":"answered","effect":"unknown","retrySafe":false}),
    );
    error
}

fn unavailable() -> AppError {
    AppError::new(
        "agent_recorder_unavailable",
        "The recorder request broker is unavailable.",
    )
}

#[tauri::command]
pub fn resolve_agent_recorder_request(
    window: WebviewWindow,
    broker: State<'_, AgentRecorderBroker>,
    request: ResolveAgentRecorderRequest,
) -> Result<(), AppError> {
    if window.label() != "main" {
        return Err(AppError::new(
            "agent_recorder_window_forbidden",
            "Only the main window may answer recorder requests.",
        ));
    }
    broker.resolve(request)
}

#[tauri::command]
pub async fn validate_agent_recorder_request(
    app: AppHandle,
    window: WebviewWindow,
    broker: State<'_, AgentRecorderBroker>,
    request_id: String,
) -> Result<(), AppError> {
    if window.label() != "main" {
        return Err(AppError::new(
            "agent_recorder_window_forbidden",
            "Only the main window may control recorder requests.",
        ));
    }
    let (binding, profile) = broker.request_authorization(&request_id)?;
    let repos = crate::commands::repositories(&app).await?;
    crate::mcp_server::tools::revalidate_binding(Some(&app), &repos.pool, &binding, &profile)
        .await?;
    broker.validate_request(&request_id)
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;

    fn resolution(id: String, note: &str, session: &str) -> ResolveAgentRecorderRequest {
        ResolveAgentRecorderRequest {
            request_id: id,
            ok: true,
            note_id: Some(note.into()),
            note_title: None,
            error_code: None,
            error_message: None,
            error_details: None,
            session_id: Some(session.into()),
            status: None,
        }
    }
    fn target() -> AgentRecorderTarget {
        AgentRecorderTarget {
            binding: Some(crate::mcp_server::turns::TurnBinding {
                authority: crate::domain::turn_authority::TurnAuthority::new(),
                wire_names: None,
                session_id: "conversation-a".into(),
                run_id: "run-a".into(),
                data_partition: "partition-a".into(),
                cli: crate::llm::cli::CliKind::Claude,
                safety_mode: crate::agent_sessions::domain::AgentSafetyMode::Sandboxed,
                attended: true,
                surface: "chat".into(),
            }),
            profile: Some("partition-a".into()),
            note_id: Some("note-a".into()),
            session_id: Some("take-a".into()),
            recovery_action: Some(RecordingRecoveryAction::Discard),
        }
    }
    #[tokio::test]
    async fn cancelled_request_revokes_capture_permission_and_rejects_late_ui_response() {
        let broker = Arc::new(AgentRecorderBroker::default());
        let (tx, rx) = oneshot::channel();
        let worker = {
            let broker = broker.clone();
            tokio::spawn(async move {
                broker
                    .request_with(AgentRecorderAction::Recover, None, target(), |payload| {
                        let _ = tx.send(payload);
                        Ok(())
                    })
                    .await
            })
        };
        let payload = rx.await.unwrap();
        let id = payload["requestId"].as_str().unwrap();
        assert_eq!(payload["recoveryAction"], "discard");
        broker
            .saved_authorization(
                id,
                "partition-a",
                "note-a",
                "take-a",
                AgentRecorderAction::Recover,
                Some(RecordingRecoveryAction::Discard),
            )
            .unwrap();
        assert!(broker
            .saved_authorization(
                id,
                "partition-a",
                "note-a",
                "take-a",
                AgentRecorderAction::Recover,
                Some(RecordingRecoveryAction::Validate)
            )
            .is_err());
        assert!(broker
            .saved_authorization(
                id,
                "partition-a",
                "note-a",
                "take-b",
                AgentRecorderAction::Recover,
                Some(RecordingRecoveryAction::Discard)
            )
            .is_err());
        assert!(broker
            .saved_authorization(
                id,
                "partition-a",
                "note-a",
                "take-a",
                AgentRecorderAction::Retry,
                None
            )
            .is_err());

        assert!(broker.validate_target(id, "partition-a", "note-a").is_err());
        assert!(broker
            .saved_authorization(
                id,
                "partition-b",
                "note-a",
                "take-a",
                AgentRecorderAction::Recover,
                Some(RecordingRecoveryAction::Discard)
            )
            .is_err());
        assert!(broker
            .saved_authorization(
                id,
                "partition-a",
                "note-b",
                "take-a",
                AgentRecorderAction::Recover,
                Some(RecordingRecoveryAction::Discard)
            )
            .is_err());

        assert!(broker.validate_target(id, "partition-b", "note-a").is_err());
        assert!(broker.validate_target(id, "partition-a", "note-b").is_err());
        worker.abort();
        let _ = worker.await;
        assert_eq!(
            broker.validate_request(id).unwrap_err().code,
            "agent_recorder_request_not_found"
        );
        assert_eq!(
            broker
                .resolve(resolution(id.into(), "note-a", "take-a"))
                .unwrap_err()
                .code,
            "agent_recorder_request_not_found"
        );
    }
    #[tokio::test]
    async fn response_cannot_substitute_another_take_or_note() {
        let broker = Arc::new(AgentRecorderBroker::default());
        for (note, session) in [("note-b", "take-a"), ("note-a", "take-b")] {
            let (tx, rx) = oneshot::channel();
            let worker = {
                let broker = broker.clone();
                tokio::spawn(async move {
                    broker
                        .request_with(AgentRecorderAction::Pause, None, target(), |payload| {
                            let _ = tx.send(payload);
                            Ok(())
                        })
                        .await
                })
            };
            let payload = rx.await.unwrap();
            let id = payload["requestId"].as_str().unwrap();
            broker
                .resolve(resolution(id.into(), note, session))
                .unwrap();
            let error = worker.await.unwrap().unwrap_err();
            assert_eq!(error.code, "agent_recorder_target_mismatch");
            let details = error.details.unwrap();
            assert_eq!(details["target"]["noteId"], "note-a");
            assert_eq!(details["effect"], "unknown");
            assert_eq!(details["retrySafe"], false);
        }
    }
    #[tokio::test]
    async fn valid_response_preserves_session_and_duplicate_resolution_is_idempotent() {
        let broker = AgentRecorderBroker::default();
        let result = broker
            .request_with(AgentRecorderAction::Recover, None, target(), |payload| {
                let answer = resolution(
                    payload["requestId"].as_str().unwrap().into(),
                    "note-a",
                    "take-a",
                );
                broker.resolve(answer.clone())?;
                broker.resolve(answer)
            })
            .await
            .unwrap();
        assert_eq!(result["sessionId"], "take-a");
        assert_eq!(result["noteId"], "note-a");
        assert_eq!(result["success"], json!(true));
    }
}
