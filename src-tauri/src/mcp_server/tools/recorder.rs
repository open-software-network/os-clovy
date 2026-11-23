//! Recorder operations use the visible main-window bridge and fixed note/session targets.
use super::{ToolCtx, ToolGroup};
use crate::agent_recorder::{
    AgentRecorderAction, AgentRecorderBroker, AgentRecorderTarget, RecordingRecoveryAction,
};
use crate::approval_broker::{ApprovalKind, ApprovalRequest, ApprovalTurn};
use crate::db::repositories::Repositories;
use crate::domain::types::{AppError, RecordingSourceMode};
use crate::mcp_server::turns::TURN_REQUIRED;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use sqlx::{query::query, row::Row};
use tauri::{AppHandle, Emitter, Manager};
pub const START_RECORDING: &str = "start_recording";
pub const STOP_RECORDING: &str = "stop_recording";
pub const RECORDING_STATUS: &str = "recording_status";
const TOOLS: [&str; 8] = [
    START_RECORDING,
    STOP_RECORDING,
    RECORDING_STATUS,
    "pause_recording",
    "resume_recording",
    "recover_recording",
    "retry_note_processing",
    "list_recording_artifacts",
];
pub struct RecorderTools;
impl ToolGroup for RecorderTools {
    fn owns(&self, name: &str) -> bool {
        TOOLS.contains(&name)
    }
    fn descriptors(&self, _: &ToolCtx<'_>) -> Vec<Value> {
        TOOLS.iter().map(|name| {
            let (description, properties, required) = match *name {
                START_RECORDING => ("Start a visible Clovy recording only when explicitly requested. Creates its note in the bound data partition. Uses meeting source selection, independent of dictation microphone selection.", json!({"sourceMode":{"type":"string","enum":["microphoneOnly","microphonePlusSystem"]}}),json!([])),
                STOP_RECORDING => ("Stop the exact active recording after approval; its note continues through transcription and generation, which may incur configured provider charges.",json!({"sessionId":{"type":"string"}}),json!([])),
                RECORDING_STATUS => ("Read active recording state, source status and elapsed duration when its note belongs to this partition.",json!({}),json!([])),
                "pause_recording" | "resume_recording" => ("Pause or resume the specified visible recording, preserving its note, session, sources and elapsed duration.",json!({"sessionId":{"type":"string","minLength":1}}),json!(["sessionId"])),
                "recover_recording" => ("Validate saved audio for a recoverable session, or explicitly discard that take. Validation can start transcription and generation using configured providers.",json!({"sessionId":{"type":"string","minLength":1},"action":{"type":"string","enum":["validate","discard"]}}),json!(["sessionId","action"])),
                "retry_note_processing" => ("Retry a failed note using durable saved audio for the specified recording session. Transcription and generation may incur configured provider charges.",json!({"noteId":{"type":"string","minLength":1},"sessionId":{"type":"string","minLength":1}}),json!(["noteId","sessionId"])),
                _ => ("List saved recording artifact IDs, sources, durations and validation state for a bound session. Does not expose file paths or grant arbitrary file access.",json!({"sessionId":{"type":"string","minLength":1}}),json!(["sessionId"])),
            };
            json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":matches!(*name,RECORDING_STATUS|"list_recording_artifacts"),"destructiveHint":*name=="recover_recording","openWorldHint":matches!(*name,STOP_RECORDING|"recover_recording"|"retry_note_processing")}})
        }).collect()
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            let recorder = ctx.app.map(|app| AppRecorder { app });
            call(
                ctx,
                recorder.as_ref().map(|r| r as &dyn RecorderPort),
                name,
                args,
            )
            .await
        })
    }
}
pub(crate) trait RecorderPort: Send + Sync {
    fn request(
        &self,
        action: AgentRecorderAction,
        source_mode: Option<RecordingSourceMode>,
        target: AgentRecorderTarget,
    ) -> BoxFuture<'_, Result<Value, AppError>>;
}
struct AppRecorder<'a> {
    app: &'a AppHandle,
}
impl RecorderPort for AppRecorder<'_> {
    fn request(
        &self,
        action: AgentRecorderAction,
        source_mode: Option<RecordingSourceMode>,
        target: AgentRecorderTarget,
    ) -> BoxFuture<'_, Result<Value, AppError>> {
        Box::pin(async move {
            self.app
                .state::<AgentRecorderBroker>()
                .request(self.app, action, source_mode, target)
                .await
        })
    }
}
pub(super) async fn recording_status_snapshot(
    ctx: &ToolCtx<'_>,
    status: Option<crate::domain::types::RecordingStatusDto>,
) -> Result<Value, AppError> {
    if let Some(status) = status.as_ref() {
        session_target(ctx, &status.session_id).await?;
    }
    serde_json::to_value(status)
        .map_err(|e| AppError::new("agent_tool_response_invalid", e.to_string()))
}
/// The recording session and note are qualified together. An id alone is never a grant.
async fn session_target(ctx: &ToolCtx<'_>, id: &str) -> Result<AgentRecorderTarget, AppError> {
    let row=query("SELECT r.note_id FROM recording_sessions r JOIN notes n ON n.id=r.note_id WHERE r.id=? AND n.profile=?").bind(id).bind(&ctx.data.profile).fetch_optional(&ctx.data.notes).await?.ok_or_else(||AppError::new("recording_not_found","The recording was not found in the bound data partition."))?;
    Ok(AgentRecorderTarget {
        profile: Some(ctx.data.profile.clone()),
        note_id: Some(row.get("note_id")),
        session_id: Some(id.into()),
        recovery_action: None,
        binding: None,
    })
}
pub(crate) async fn call(
    ctx: &ToolCtx<'_>,
    recorder: Option<&dyn RecorderPort>,
    name: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let binding = ctx
        .binding
        .ok_or_else(|| AppError::new(TURN_REQUIRED, "Recorder tools require a Clovy turn."))?;
    ctx.revalidate().await?;
    if name == RECORDING_STATUS {
        return recording_status_snapshot(ctx, crate::audio::capture::current_status()).await;
    }
    let (action, source_mode, title, description, mut target) = match name {
        START_RECORDING => {
            let source = match args.get("sourceMode").and_then(Value::as_str) {
                None | Some("microphoneOnly") => RecordingSourceMode::MicrophoneOnly,
                Some("microphonePlusSystem") => RecordingSourceMode::MicrophonePlusSystem,
                _ => return Err(invalid("Invalid sourceMode.")),
            };
            (AgentRecorderAction::Start,Some(source),"Start a recording","Clovy starts a visible recording of the selected meeting audio sources and opens its note.",AgentRecorderTarget{profile:Some(ctx.data.profile.clone()),..Default::default()})
        }
        STOP_RECORDING | "pause_recording" | "resume_recording" => {
            let active = crate::audio::capture::current_status().ok_or_else(|| {
                AppError::new("recording_not_found", "No recording is currently running.")
            })?;
            let id = args
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or(&active.session_id);
            if id != active.session_id {
                return Err(AppError::new(
                    "recording_target_mismatch",
                    "The requested session is not the active recording.",
                ));
            }
            let action = match name {
                STOP_RECORDING => AgentRecorderAction::Stop,
                "pause_recording" => AgentRecorderAction::Pause,
                _ => AgentRecorderAction::Resume,
            };
            (
                action,
                None,
                "Control recording",
                "Change the specified visible recording.",
                session_target(ctx, id).await?,
            )
        }
        "recover_recording" | "retry_note_processing" | "list_recording_artifacts" => {
            let id = required(args, "sessionId")?;
            let mut target = session_target(ctx, id).await?;
            if name == "list_recording_artifacts" {
                let rows = query("SELECT id,source,format,duration_ms,size_bytes,status,expected_duration_ms,created_at FROM audio_artifacts WHERE recording_session_id=? AND note_id=? ORDER BY CASE source WHEN 'microphone' THEN 0 WHEN 'system' THEN 1 ELSE 2 END,id").bind(id).bind(target.note_id.as_deref()).fetch_all(&ctx.data.notes).await?;
                let artifacts:Vec<Value>=rows.into_iter().map(|row|json!({"id":row.get::<String,_>("id"),"source":row.get::<String,_>("source"),"format":row.get::<String,_>("format"),"durationMs":row.get::<i64,_>("duration_ms"),"sizeBytes":row.get::<i64,_>("size_bytes"),"status":row.get::<String,_>("status"),"expectedDurationMs":row.get::<i64,_>("expected_duration_ms"),"createdAt":row.get::<String,_>("created_at")})).collect();
                let row=query("SELECT status,source_mode,expected_elapsed_ms,duration_ms,validation_summary,last_error FROM recording_sessions WHERE id=?").bind(id).fetch_one(&ctx.data.notes).await?;
                return Ok(
                    json!({"sessionId":id,"noteId":target.note_id,"status":row.get::<String,_>("status"),"sourceMode":RecordingSourceMode::from(row.get::<String,_>("source_mode").as_str()),"elapsedMs":row.get::<i64,_>("expected_elapsed_ms"),"durationMs":row.get::<Option<i64>,_>("duration_ms"),"validationSummary":row.get::<Option<String>,_>("validation_summary"),"lastError":row.get::<Option<String>,_>("last_error"),"artifacts":artifacts}),
                );
            }
            if name == "recover_recording" {
                target.recovery_action = Some(match required(args, "action")? {
                    "validate" => RecordingRecoveryAction::Validate,
                    "discard" => RecordingRecoveryAction::Discard,
                    _ => return Err(invalid("action must be validate or discard.")),
                });
            } else if target.note_id.as_deref() != Some(required(args, "noteId")?) {
                return Err(AppError::new(
                    "recording_target_mismatch",
                    "The recording session does not belong to this note.",
                ));
            }
            validate_saved_operation(ctx, name, &target).await?;
            (if name=="recover_recording"{AgentRecorderAction::Recover}else{AgentRecorderAction::Retry},None,"Process saved recording","Validate or discard the specified recoverable take, or retry its saved audio. Processing can incur provider charges.",target)
        }
        _ => return Err(AppError::new("mcp_tool_unknown", "Unknown recorder tool.")),
    };
    let recorder = recorder.ok_or_else(|| {
        AppError::new(
            "agent_recorder_unavailable",
            "The recorder request broker is unavailable.",
        )
    })?;
    ctx.broker()?
        .request(
            &ApprovalTurn::from(binding),
            ApprovalRequest {
                tool: name.into(),
                call_id: Some(uuid::Uuid::new_v4().to_string()),
                kind: ApprovalKind::Approval {
                    title: title.into(),
                    description: description.into(),
                    arguments: args.clone(),
                    preview: Some(serde_json::to_string(&target).unwrap_or_default()),
                },
            },
        )
        .await
        .map_err(|e| AppError::new(e.code(), e.message()))?;
    ctx.revalidate().await?;
    if let Some(id) = target.session_id.as_deref() {
        let checked = session_target(ctx, id).await?;
        if checked.note_id != target.note_id {
            return Err(AppError::new(
                "recording_target_mismatch",
                "Recording ownership changed.",
            ));
        }
    }
    if matches!(name, "recover_recording" | "retry_note_processing") {
        validate_saved_operation(ctx, name, &target).await?;
    }
    if name == START_RECORDING {
        let note = Repositories::new(ctx.data.notes.clone())
            .create_note_checked(
                &ctx.data.profile,
                None,
                Some(crate::db::repositories::McpMutation::Turn(
                    &binding.session_id,
                    &binding.run_id,
                    &binding.authority,
                )),
            )
            .await?;
        if let Some(app) = ctx.app {
            let _ = app.emit(
                "clovy-data-changed",
                json!({"profile":ctx.data.profile,"kind":"notes","id":note.id}),
            );
        }
        target.note_id = Some(note.id);
    }
    let effect_target = serde_json::to_value(&target).unwrap_or_default();
    target.binding = Some(binding.clone());
    let request = recorder.request(action, source_mode, target);
    tokio::pin!(request);
    let result = loop {
        tokio::select! {
            result = &mut request => {
                match result {
                    Ok(result)=>break result,
                    Err(mut error)=>{
                        if error.details.is_none(){error.details=Some(json!({"target":effect_target,"delivery":if matches!(error.code.as_str(),"agent_recorder_timeout"|"agent_recorder_cancelled"){"unknown"}else{"answered"},"effect":if matches!(error.code.as_str(),"agent_recorder_timeout"|"agent_recorder_cancelled"){"unknown"}else{"failed"},"retrySafe":false}));}
                        return Err(error);
                    }
                }
            },
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                if let Err(mut error)=ctx.revalidate().await {error.details=Some(json!({"target":effect_target,"delivery":"unknown","effect":"unknown","retrySafe":false}));return Err(error);}
            },
        }
    };
    if let Err(mut error) = ctx.revalidate().await {
        error.details = Some(
            json!({"recordingResult":result,"effect":"completed_before_turn_revalidation","retrySafe":false}),
        );
        return Err(error);
    }
    Ok(result)
}
async fn validate_saved_operation(
    ctx: &ToolCtx<'_>,
    name: &str,
    target: &AgentRecorderTarget,
) -> Result<(), AppError> {
    let row=query("SELECT r.status,n.processing_status FROM recording_sessions r JOIN notes n ON n.id=r.note_id WHERE r.id=? AND n.id=? AND n.profile=?").bind(target.session_id.as_deref()).bind(target.note_id.as_deref()).bind(&ctx.data.profile).fetch_optional(&ctx.data.notes).await?.ok_or_else(||AppError::new("recording_not_found","The saved recording target no longer exists."))?;
    if name == "recover_recording" && row.get::<String, _>("status") != "recoverable" {
        return Err(AppError::new(
            "recording_not_recoverable",
            "Only an interrupted recoverable take can be validated or discarded.",
        ));
    }
    if name == "retry_note_processing" && row.get::<String, _>("processing_status") != "failed" {
        return Err(AppError::new(
            "note_not_failed",
            "Only failed note processing can be retried.",
        ));
    }
    Ok(())
}
fn invalid(message: &str) -> AppError {
    AppError::new("agent_tool_arguments_invalid", message)
}
fn required<'a>(args: &'a Value, key: &str) -> Result<&'a str, AppError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty() && v.len() <= 128)
        .ok_or_else(|| invalid(&format!("{key} is required.")))
}
