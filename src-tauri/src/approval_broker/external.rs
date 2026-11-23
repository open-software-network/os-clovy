//! Approval of one operation from an authenticated external MCP connection.
//! External requests have no session, run, tool grant, or bypass policy.
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tokio::sync::oneshot;
use tokio::time::Instant;
use uuid::Uuid;

use crate::domain::types::AppError;
use crate::mcp_server::channel::ExternalConnection;

pub const CHANGED_EVENT: &str = "mcp-external-approvals-changed";
const PREVIEW_LIMIT: usize = 16 * 1024;
const PROFILE_RECHECK: Duration = Duration::from_millis(100);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalApprovalRequest {
    id: String,
    tool: String,
    title: String,
    profile: String,
    preview: String,
}

struct Pending {
    request: ExternalApprovalRequest,
    connection: ExternalConnection,
    deadline: Instant,
    generation: u64,
    decision: oneshot::Sender<bool>,
}

type Profile = dyn Fn() -> Option<String> + Send + Sync;
type Notify = dyn Fn() + Send + Sync;

/// A short-lived queue of one-shot external requests and their decision
/// channels. No untrusted arguments reach a renderer event or a log.
pub struct ExternalApprovalBroker {
    pending: Mutex<HashMap<String, Pending>>,
    current_profile: Arc<Profile>,
    notify: Box<Notify>,
    timeout: Duration,
    generation: AtomicU64,
}

impl ExternalApprovalBroker {
    pub fn new(
        current_profile: impl Fn() -> Option<String> + Send + Sync + 'static,
        notify: impl Fn() + Send + Sync + 'static,
        timeout: Duration,
    ) -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
            current_profile: Arc::new(current_profile),
            notify: Box::new(notify),
            timeout,
            generation: AtomicU64::new(0),
        }
    }

    pub fn pending(&self) -> Vec<ExternalApprovalRequest> {
        let profile = (self.current_profile)();
        let now = Instant::now();
        let requests = self.lock();
        let mut pending = requests
            .values()
            .filter(|entry| {
                entry.live(
                    profile.as_deref(),
                    now,
                    self.generation.load(Ordering::Acquire),
                )
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|entry| entry.deadline);
        pending
            .into_iter()
            .map(|entry| entry.request.clone())
            .collect()
    }

    pub fn decide(&self, id: &str, approve: bool) -> Result<(), AppError> {
        let profile = (self.current_profile)();
        let mut requests = self.lock();
        let pending = requests.remove(id).ok_or_else(stale)?;
        let decision = if pending.live(
            profile.as_deref(),
            Instant::now(),
            self.generation.load(Ordering::Acquire),
        ) {
            pending.decision.send(approve).map_err(|_| stale())
        } else {
            Err(stale())
        };
        drop(requests);
        (self.notify)();
        decision
    }

    /// Called by a host transition that revokes external authority. Even
    /// switching away and back cannot revive operations on the old scope.
    pub fn invalidate(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.lock().clear();
        (self.notify)();
    }

    pub(crate) fn scope<'a>(
        &'a self,
        connection: &'a ExternalConnection,
        profile: &'a str,
        recording_data: Option<&'a Path>,
    ) -> ExternalCall<'a> {
        ExternalCall {
            broker: self,
            connection,
            profile,
            recording_data,
            deadline: Instant::now() + self.timeout,
            generation: self.generation.load(Ordering::Acquire),
        }
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Pending {
    fn live(&self, profile: Option<&str>, now: Instant, generation: u64) -> bool {
        self.connection.is_live()
            && profile == Some(self.request.profile.as_str())
            && now < self.deadline
            && generation == self.generation
            && !self.decision.is_closed()
    }
}

pub(crate) struct ExternalCall<'a> {
    broker: &'a ExternalApprovalBroker,
    connection: &'a ExternalConnection,
    profile: &'a str,
    recording_data: Option<&'a Path>,
    deadline: Instant,
    generation: u64,
}

impl ExternalCall<'_> {
    pub(crate) fn recording_data(&self) -> Option<&Path> {
        self.recording_data
    }

    pub(crate) fn check_read_scope(&self) -> Result<(), AppError> {
        if self.broker.generation.load(Ordering::Acquire) != self.generation
            || (self.broker.current_profile)().as_deref() != Some(self.profile)
        {
            return Err(AppError::new(
                "mcp_external_scope_changed",
                "The active data partition or MCP access changed.",
            ));
        }
        if Instant::now() >= self.deadline {
            return Err(stale());
        }
        Ok(())
    }

    pub(crate) fn check_live(&self) -> Result<(), AppError> {
        if !self.connection.is_live() {
            return Err(AppError::new(
                "mcp_external_cancelled",
                "The external MCP connection or request closed.",
            ));
        }
        self.check_read_scope()
    }

    pub(crate) async fn approve<'a>(
        &'a self,
        tool: &str,
        title: &str,
        description: &str,
        arguments: &Value,
        target: Option<&'a Value>,
    ) -> Result<ExternalApproval<'a>, AppError> {
        self.check_live()?;
        let preview = preview(description, arguments, target)?;
        let id = Uuid::new_v4().to_string();
        let (decision, mut answer) = oneshot::channel();
        let _waiting = Waiting {
            broker: self.broker,
            id: &id,
        };
        self.broker.lock().insert(
            id.clone(),
            Pending {
                request: ExternalApprovalRequest {
                    id: id.clone(),
                    tool: tool.to_string(),
                    title: title.to_string(),
                    profile: self.profile.to_string(),
                    preview,
                },
                connection: self.connection.clone(),
                deadline: self.deadline,
                generation: self.generation,
                decision,
            },
        );
        (self.broker.notify)();
        let result = loop {
            if let Err(error) = self.check_live() {
                break Err(error);
            }
            tokio::select! {
                biased;
                _ = self.connection.closed() => break Err(self.check_live().err().unwrap_or_else(stale)),
                _ = tokio::time::sleep_until(self.deadline) => break Err(stale()),
                decision = &mut answer => {
                    break match decision {
                        Ok(true) => self.check_live().map(|()| ExternalApproval { call: self, target }),
                        Ok(false) => Err(AppError::new("user_denied", "The user denied this MCP action.")),
                        Err(_) => self.check_live().and_then(|()| Err(stale())),
                    };
                },
                _ = tokio::time::sleep(PROFILE_RECHECK) => {}
            }
        };
        tracing::info!(
            request_id = %id,
            connection_id = %self.connection.id(),
            tool,
            outcome = result.as_ref().map_or_else(|error| error.code.as_str(), |_| "approved"),
            "External MCP approval settled"
        );
        result
    }
}

/// An approval can only be produced by the matching pending request. The
/// persistence boundary rechecks it after obtaining its write lock.
#[derive(Clone, Copy)]
pub(crate) struct ExternalApproval<'a> {
    call: &'a ExternalCall<'a>,
    target: Option<&'a Value>,
}

impl<'a> ExternalApproval<'a> {
    pub(crate) fn check(&self, profile: &str) -> Result<(), AppError> {
        if self.call.profile != profile {
            return Err(AppError::new(
                "mcp_external_scope_changed",
                "The operation's data partition changed.",
            ));
        }
        self.call.check_live()
    }

    pub(crate) fn target(&self) -> Option<&'a Value> {
        self.target
    }
}

struct Waiting<'a> {
    broker: &'a ExternalApprovalBroker,
    id: &'a str,
}

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let removed = self.broker.lock().remove(self.id).is_some();
        if removed {
            (self.broker.notify)();
        }
    }
}

fn stale() -> AppError {
    AppError::new(
        "mcp_external_approval_expired",
        "This external MCP approval is no longer pending.",
    )
}

fn preview(
    description: &str,
    arguments: &Value,
    target: Option<&Value>,
) -> Result<String, AppError> {
    let data = match target {
        Some(target) => json!({ "arguments": arguments, "target": target }),
        None => json!({ "arguments": arguments }),
    };
    let raw = format!(
        "{description}\n\n{}",
        serde_json::to_string_pretty(&data).unwrap_or_default()
    );
    let mut display = String::with_capacity(raw.len());
    for char in raw.chars() {
        // JSON already escapes ASCII control bytes. Isolate Unicode display
        // controls so an untrusted target cannot reorder the visible preview.
        if matches!(char, '\u{061c}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')
        {
            use std::fmt::Write;
            let _ = write!(display, "\\u{:04x}", char as u32);
        } else {
            display.push(char);
        }
        if display.len() > PREVIEW_LIMIT {
            return Err(AppError::new(
                "mcp_external_preview_too_large",
                "Clovy could not preview this entire MCP action for approval.",
            ));
        }
    }
    Ok(display)
}

pub(super) fn setup(app: &tauri::App) {
    let scope_app = app.handle().clone();
    let notify_app = app.handle().clone();
    app.manage(ExternalApprovalBroker::new(
        move || {
            crate::activity::mcp_server_enabled(&scope_app)
                .then(|| crate::commands::active_profile(&scope_app))
        },
        move || {
            if let Some(main) = notify_app.get_webview_window("main") {
                let _ = main.emit(CHANGED_EVENT, ());
            }
        },
        super::APPROVAL_TIMEOUT,
    ));
}

fn app_broker<'a>(
    app: &'a AppHandle,
    window: &WebviewWindow,
) -> Result<tauri::State<'a, ExternalApprovalBroker>, AppError> {
    if window.label() != "main" {
        return Err(AppError::new(
            "mcp_external_approval_unavailable",
            "Open Clovy's main window to review MCP approvals.",
        ));
    }
    app.try_state().ok_or_else(stale)
}

#[tauri::command]
pub async fn mcp_external_approvals(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<Vec<ExternalApprovalRequest>, AppError> {
    Ok(app_broker(&app, &window)?.pending())
}

#[tauri::command]
pub async fn mcp_external_approval_decide(
    app: AppHandle,
    window: WebviewWindow,
    id: String,
    approve: bool,
) -> Result<(), AppError> {
    app_broker(&app, &window)?.decide(&id, approve)
}
