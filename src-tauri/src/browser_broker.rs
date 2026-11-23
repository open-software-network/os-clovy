//! Host-bound browser authority and receipts. Engine handles never confer
//! authority across conversations, turns, tabs or human takeover.
use crate::{
    approval_broker::{
        ApprovalBroker, ApprovalKind, ApprovalOutcome, ApprovalRequest, ApprovalTurn,
    },
    browser::policy::{interaction_route, InteractionClass, InteractionRoute, InteractiveElement},
    db::repositories::Repositories,
    domain::types::AppError,
    mcp_server::turns::TurnBinding,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, Mutex},
};
pub(crate) type TransportFuture<'a> =
    Pin<Box<dyn Future<Output = Result<TransportResponse, AppError>> + Send + 'a>>;
pub(crate) trait BrowserTransport: Send + Sync {
    fn reserve_session(&self, id: &str) -> Result<(), AppError>;
    fn execute<'a>(&'a self, tool: &'a str, arguments: Value) -> TransportFuture<'a>;
    fn terminate_session(&self, id: &str);
    fn terminate_session_for_shutdown(
        &self,
        id: &str,
        _deadline: crate::shutdown::ShutdownDeadline,
    ) {
        self.terminate_session(id);
    }
    fn session_state(&self, id: &str) -> Result<Value, AppError>;
    fn human_takeover<'a>(&'a self, id: &'a str) -> TransportFuture<'a>;
    fn resume_control<'a>(&'a self, id: &'a str) -> TransportFuture<'a>;
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum BrowserTransportKind {
    Attended,
    Managed,
}
impl BrowserTransportKind {
    fn name(self) -> &'static str {
        match self {
            Self::Attended => "attended",
            Self::Managed => "managed",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RoutineBrowserGrant {
    pub job_id: String,
    pub enabled: bool,
}
pub(crate) struct TransportResponse {
    pub(crate) data: Value,
    pub(crate) artifact: Option<TransportArtifact>,
}
impl TransportResponse {
    pub(crate) fn data(data: Value) -> Self {
        Self {
            data,
            artifact: None,
        }
    }
    pub(crate) fn artifact(data: Value, artifact: TransportArtifact) -> Self {
        Self {
            data,
            artifact: Some(artifact),
        }
    }
}
pub(crate) struct TransportArtifact {
    pub kind: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Owner {
    conversation: String,
    run: String,
    routine: Option<String>,
}
impl Owner {
    fn kind(&self) -> BrowserTransportKind {
        if self.routine.is_some() {
            BrowserTransportKind::Managed
        } else {
            BrowserTransportKind::Attended
        }
    }
}
struct Session {
    owner: Owner,
    tabs: HashSet<i64>,
    operation: Arc<tokio::sync::Mutex<()>>,
}
#[derive(Default)]
struct State {
    flag: Option<PathBuf>,
    blocked: bool,
    feature_blocked: bool,
    closing: bool,
    sessions: HashMap<String, Session>,
    ended: HashSet<String>,
    transports: HashMap<BrowserTransportKind, Arc<dyn BrowserTransport>>,
    screenshots: Option<PathBuf>,
    artifacts: Option<PathBuf>,
    repositories: Option<Repositories>,
    grants: Vec<RoutineBrowserGrant>,
}
#[derive(Default)]
pub(crate) struct BrowserBroker {
    state: Mutex<State>,
    transition: tokio::sync::Mutex<()>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct Inspection {
    url: String,
    origin: String,
    document_id: String,
    metadata_id: String,
    interaction_class: InteractionClass,
    backend_node_id: i64,
    element: InteractiveElement,
}
fn err(code: &str, message: &str) -> AppError {
    AppError::new(code, message)
}
fn missing() -> AppError {
    err(
        "browser_session_not_found",
        "This browser session does not belong to this conversation and run, or has ended.",
    )
}
fn unavailable() -> AppError {
    err(
        "browser_unavailable",
        "Clovy's dedicated browser is not ready. Prepare or reinstall its bundled resources.",
    )
}
impl BrowserBroker {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    pub(crate) fn configure_transport(
        &self,
        kind: BrowserTransportKind,
        transport: Arc<dyn BrowserTransport>,
        screenshots: PathBuf,
        artifacts: PathBuf,
    ) {
        let mut s = self.lock();
        s.transports.insert(kind, transport);
        s.screenshots = Some(screenshots);
        s.artifacts = Some(artifacts);
    }
    pub(crate) fn configure_outcome_repository(&self, repositories: Repositories) {
        self.lock().repositories = Some(repositories);
    }
    pub(crate) fn set_access_flag_path(&self, path: PathBuf) {
        self.lock().flag = Some(path);
    }
    pub(crate) fn set_routine_grant(&self, grant: RoutineBrowserGrant) {
        let mut s = self.lock();
        s.grants.retain(|g| g.job_id != grant.job_id);
        s.grants.push(grant);
    }
    pub(crate) fn remove_routine_grant(&self, id: &str) -> Option<RoutineBrowserGrant> {
        let mut s = self.lock();
        let index = s.grants.iter().position(|g| g.job_id == id)?;
        Some(s.grants.remove(index))
    }
    pub(crate) async fn revoke_routine_sessions(&self, id: &str) {
        self.terminate_matching(|s| s.owner.routine.as_deref() == Some(id));
    }
    pub(crate) fn is_enabled(&self) -> bool {
        let s = self.lock();
        !s.closing
            && !s.blocked
            && !s.feature_blocked
            && s.flag.as_ref().is_some_and(|p| p.is_file())
    }
    pub(crate) fn set_feature_enabled(&self, enabled: bool) {
        self.lock().feature_blocked = !enabled;
        if !enabled {
            self.terminate_matching(|_| true);
        }
    }
    pub(crate) async fn set_enabled(&self, enabled: bool) -> Result<(), AppError> {
        self.lock().blocked = !enabled;
        if !enabled {
            self.terminate_matching(|s| s.owner.routine.is_none());
        }
        Ok(())
    }
    fn terminate_matching(&self, predicate: impl Fn(&Session) -> bool) {
        let pending = {
            let mut s = self.lock();
            let ids: Vec<_> = s
                .sessions
                .iter()
                .filter(|(_, session)| predicate(session))
                .map(|(id, _)| id.clone())
                .collect();
            ids.into_iter()
                .filter_map(|id| {
                    let session = s.sessions.remove(&id)?;
                    Some((id, s.transports.get(&session.owner.kind())?.clone()))
                })
                .collect::<Vec<_>>()
        };
        for (id, t) in pending {
            t.terminate_session(&id);
        }
    }
    pub(crate) fn terminate_run(&self, id: &str) {
        self.lock().ended.insert(id.into());
        self.terminate_matching(|s| s.owner.run == id);
    }
    pub(crate) fn terminate_sessions(&self, deadline: crate::shutdown::ShutdownDeadline) {
        let Some(mut state) = deadline.try_lock(&self.state) else {
            return;
        };
        state.feature_blocked = true;
        state.closing = true;
        let sessions: Vec<_> = state
            .sessions
            .drain()
            .map(|(id, s)| (id, s.owner.kind()))
            .collect();
        let transports = state.transports.clone();
        drop(state);
        for (id, kind) in sessions {
            if let Some(transport) = transports.get(&kind) {
                transport.terminate_session_for_shutdown(&id, deadline);
            }
        }
    }
    fn require_access(&self, owner: &Owner) -> Result<(), AppError> {
        let s = self.lock();
        if s.closing {
            return Err(err(
                "browser_session_closed",
                "Browser control is shutting down.",
            ));
        }
        if s.feature_blocked {
            return Err(err(
                "browser_use_disabled",
                "Enable Browser use in experimental settings first.",
            ));
        }
        if s.ended.contains(&owner.run) {
            return Err(missing());
        }
        if let Some(job) = &owner.routine {
            if s.grants.iter().any(|g| g.job_id == *job && g.enabled) {
                return Ok(());
            }
            return Err(err(
                "routine_browser_access_disabled",
                "This routine has no current browser grant.",
            ));
        }
        if !s.blocked && s.flag.as_ref().is_some_and(|p| p.is_file()) {
            Ok(())
        } else {
            Err(err(
                "browser_access_disabled",
                "Enable Browser use before starting a browser task.",
            ))
        }
    }
    fn require_session(
        &self,
        owner: &Owner,
        id: &str,
    ) -> Result<Arc<tokio::sync::Mutex<()>>, AppError> {
        self.require_access(owner)?;
        self.lock()
            .sessions
            .get(id)
            .filter(|s| s.owner == *owner)
            .map(|s| s.operation.clone())
            .ok_or_else(missing)
    }
    fn transport(&self, kind: BrowserTransportKind) -> Result<Arc<dyn BrowserTransport>, AppError> {
        self.lock()
            .transports
            .get(&kind)
            .cloned()
            .ok_or_else(unavailable)
    }
    async fn revalidate(&self, owner: &Owner, id: Option<&str>) -> Result<(), AppError> {
        self.require_access(owner)?;
        if let Some(job) = &owner.routine {
            let repos = self.lock().repositories.clone().ok_or_else(unavailable)?;
            if !repos
                .routine_browser_grant(job)
                .await?
                .is_some_and(|g| g.enabled)
            {
                self.terminate_matching(|s| s.owner.routine.as_ref() == Some(job));
                return Err(err(
                    "routine_browser_access_disabled",
                    "The routine's browser grant has been revoked.",
                ));
            }
        }
        if let Some(id) = id {
            self.require_session(owner, id)?;
        }
        Ok(())
    }
    /// Routine identity is resolved by the MCP group from routine_runs; no
    /// caller-supplied profile/workspace/routine identifier is accepted.
    pub(crate) async fn execute_bound(
        &self,
        binding: &TurnBinding,
        approvals: &ApprovalBroker,
        routine: Option<&str>,
        tool: &str,
        mut arguments: Value,
    ) -> Result<Value, AppError> {
        if (!binding.attended && routine.is_none()) || (binding.attended && routine.is_some()) {
            return Err(err(
                "browser_owner_required",
                "Browser authority requires a trusted chat or routine binding.",
            ));
        }
        let owner = Owner {
            conversation: binding.session_id.clone(),
            run: binding.run_id.clone(),
            routine: routine.map(str::to_owned),
        };
        self.revalidate(&owner, None).await?;
        let transport = self.transport(owner.kind())?;
        if tool == "start_session" {
            let id = uuid::Uuid::new_v4().to_string();
            {
                let mut s = self.lock();
                if s.ended.contains(&owner.run) {
                    return Err(missing());
                }
                transport.reserve_session(&id)?;
                s.sessions.insert(
                    id.clone(),
                    Session {
                        owner: owner.clone(),
                        tabs: HashSet::new(),
                        operation: Arc::new(tokio::sync::Mutex::new(())),
                    },
                );
            }
            let started = self
                .dispatch(
                    &owner,
                    &id,
                    tool,
                    json!({"session_id":id}),
                    transport.clone(),
                )
                .await;
            match started {
                Ok(data) => {
                    if let Err(e) = self.revalidate(&owner, Some(&id)).await {
                        transport.terminate_session(&id);
                        return Err(e);
                    }
                    let Some(tab) = data["tabId"].as_i64().filter(|tab| *tab > 0) else {
                        self.lock().sessions.remove(&id);
                        transport.terminate_session(&id);
                        return Err(err(
                            "browser_outcome_unknown",
                            "The browser startup receipt was invalid. Its task has been stopped.",
                        ));
                    };
                    self.lock()
                        .sessions
                        .get_mut(&id)
                        .ok_or_else(missing)?
                        .tabs
                        .insert(tab);
                    return Ok(json!({"sessionId":id,"tabId":tab,"mode":"background"}));
                }
                Err(e) => {
                    self.lock().sessions.remove(&id);
                    transport.terminate_session(&id);
                    return Err(e);
                }
            }
        }
        let id = string(&arguments, "session_id")?.to_owned();
        let operation = self.require_session(&owner, &id)?;
        // Stop never waits for a call/approval/CDP mutex.
        if tool == "close_session" {
            self.stop_session(&id)?;
            return Ok(json!({"closed":true}));
        }
        let mut operation_guard = Some(operation.lock().await);
        self.revalidate(&owner, Some(&id)).await?;
        if !matches!(
            tool,
            "open_tab"
                | "list_tabs"
                | "navigate"
                | "back"
                | "snapshot"
                | "screenshot"
                | "click"
                | "fill"
                | "press"
                | "switch_tab"
                | "close_tab"
        ) {
            return Err(err("unknown_browser_tool", "Unknown browser operation."));
        }
        if !matches!(tool, "open_tab" | "list_tabs") {
            let tab = integer(&arguments, "tab_id")?;
            if !self
                .lock()
                .sessions
                .get(&id)
                .is_some_and(|s| s.tabs.contains(&tab))
            {
                return Err(err(
                    "tab_not_owned",
                    "This tab is not owned by this browser task.",
                ));
            }
        }
        if matches!(tool, "click" | "fill" | "press") {
            let inspected = transport
                .execute(
                    "inspect_reference",
                    json!({
                        "session_id": id,
                        "tab_id": integer(&arguments, "tab_id")?,
                        "ref": string(&arguments, "ref")?,
                    }),
                )
                .await?;
            let expected: Inspection =
                serde_json::from_value(inspected.data).map_err(|_| unavailable())?;
            match tool {
                "fill" => {
                    if !arguments["text"].is_string() {
                        return Err(err("invalid_arguments", "text is required."));
                    }
                }
                "press" => {
                    string(&arguments, "key")?;
                }
                _ => {}
            }
            // Redacted metadata cannot safely be classified again here. The
            // required host enum was computed from original facts and is bound
            // into metadataId/expected for the final guard.
            match interaction_route(expected.interaction_class, binding.attended) {
                InteractionRoute::HumanOnly => {
                    return Err(err(
                        "browser_human_takeover_required",
                        if binding.attended {
                            "First call request_clarification and keep it pending while the user takes over the dedicated browser, completes sensitive input and resumes control. Then list tabs and inspect a fresh snapshot; never enter credentials or repeat this input."
                        } else {
                            "This routine uses an anonymous browser and cannot enter sensitive fields or use a login profile."
                        },
                    ))
                }
                InteractionRoute::UnattendedDenied => {
                    return Err(err(
                        "unattended_requires_approval",
                        "This browser action needs a person to approve it.",
                    ));
                }
                InteractionRoute::CentralApproval => {
                    // A human takeover/Stop must remain available while the
                    // central approval card waits. The exact inspection is
                    // retained and guarded again after reacquiring authority.
                    drop(operation_guard.take());
                    let decision = approvals.request(
                        &ApprovalTurn::from(binding),
                        ApprovalRequest {
                            tool: tool.into(),
                            call_id: Some(uuid::Uuid::new_v4().to_string()),
                            kind: ApprovalKind::Approval {
                                title: "Use the browser".into(),
                                description: "Clovy will interact with the inspected page. The page can attach effects to any click, field change or key.".into(),
                                arguments: json!({
                                    "session_id": id,
                                    "tab_id": integer(&arguments, "tab_id")?,
                                    "operation": tool,
                                    "origin": expected.origin,
                                }),
                                preview: Some(browser_action_preview(tool, &expected)),
                            },
                        },
                    ).await.map_err(|d| AppError::new(d.code(), d.message()))?;
                    if decision != ApprovalOutcome::Approved {
                        return Err(err(
                            "browser_action_declined",
                            "The browser action was not approved.",
                        ));
                    }
                    operation_guard = Some(operation.lock().await);
                    self.revalidate(&owner, Some(&id)).await?;
                }
            }
            arguments["expected"] = serde_json::to_value(expected).map_err(|_| unavailable())?;
        }
        self.revalidate(&owner, Some(&id)).await?;
        if !matches!(tool, "open_tab" | "list_tabs")
            && !self.lock().sessions.get(&id).is_some_and(|session| {
                session
                    .tabs
                    .contains(&arguments["tab_id"].as_i64().unwrap_or_default())
            })
        {
            return Err(err(
                "tab_not_owned",
                "The inspected tab is no longer owned by this task.",
            ));
        }
        let mut data = self
            .dispatch(&owner, &id, tool, arguments.clone(), transport.clone())
            .await?;
        if let Err(error) = self.revalidate(&owner, Some(&id)).await {
            if !matches!(tool, "snapshot" | "screenshot" | "list_tabs") {
                self.lock().sessions.remove(&id);
                transport.terminate_session(&id);
                return Err(err("browser_outcome_unknown","Browser authority ended after dispatch. The operation may have happened; do not repeat it."));
            }
            return Err(error);
        }
        if tool == "list_tabs" {
            let owned = self
                .lock()
                .sessions
                .get(&id)
                .ok_or_else(missing)?
                .tabs
                .clone();
            data["tabs"]
                .as_array_mut()
                .ok_or_else(unavailable)?
                .retain(|tab| tab["tabId"].as_i64().is_some_and(|id| owned.contains(&id)));
        }
        if tool == "open_tab" {
            let tab = data["tabId"].as_i64().ok_or_else(unavailable)?;
            self.lock()
                .sessions
                .get_mut(&id)
                .ok_or_else(missing)?
                .tabs
                .insert(tab);
        }
        if tool == "close_tab" {
            if let Some(s) = self.lock().sessions.get_mut(&id) {
                s.tabs.remove(&integer(&arguments, "tab_id")?);
            }
        }
        drop(operation_guard);
        Ok(data)
    }
    async fn dispatch(
        &self,
        owner: &Owner,
        id: &str,
        tool: &str,
        arguments: Value,
        transport: Arc<dyn BrowserTransport>,
    ) -> Result<Value, AppError> {
        let repos = Some(self.lock().repositories.clone().ok_or_else(|| {
            err(
                "browser_ledger_unavailable",
                "Browser storage is still initializing. Try again after startup.",
            )
        })?);
        let action = uuid::Uuid::new_v4().to_string();
        let class = match tool {
            "navigate" | "back" => Some("target_state"),
            "snapshot" | "screenshot" => Some("artifact"),
            "click" | "fill" | "press" => Some("action_receipt"),
            _ => None,
        };
        if let Some(repos) = &repos {
            repos
                .declare_browser_action(&action, tool, owner.kind().name(), id, class)
                .await
                .map_err(|_| {
                    err(
                        "browser_ledger_unavailable",
                        "The browser action could not be recorded before dispatch.",
                    )
                })?;
        }
        self.revalidate(owner, Some(id)).await?;
        let result = transport
            .execute(tool, arguments)
            .await
            .and_then(|response| self.finish_response(response));
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.code == "browser_outcome_unknown")
        {
            self.lock().sessions.remove(id);
            transport.terminate_session(id);
        }
        if let Some(repos) = repos {
            let (kind, code) = match &result {
                Ok(_) => ("executed", None),
                Err(e) if e.code == "browser_outcome_unknown" => {
                    ("transport_error", Some(e.code.as_str()))
                }
                Err(e) => ("refused", Some(e.code.as_str())),
            };
            // Delivery is not proof of a website's business outcome.
            if repos
                .evaluate_browser_action(&action, kind, code, false)
                .await
                .is_err()
                && result.is_ok()
            {
                // Delivery happened. Destroy this task's handles before
                // reporting an uncertain receipt so a repeated tool call
                // cannot replay the same mutation.
                self.lock().sessions.remove(id);
                transport.terminate_session(id);
                return Err(err(
                    "browser_outcome_unknown",
                    "The browser acknowledged the operation, but its receipt could not be saved. Do not repeat it.",
                ));
            }
        }
        result
    }
    fn finish_response(&self, mut response: TransportResponse) -> Result<Value, AppError> {
        if let Some(artifact) = response.artifact {
            let root = self.lock().screenshots.clone().ok_or_else(unavailable)?;
            if artifact.kind != "screenshot"
                || artifact.mime_type != "image/png"
                || !artifact.bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            {
                return Err(err(
                    "browser_artifact_invalid",
                    "The browser returned an invalid image.",
                ));
            }
            let filename = format!("browser-screenshot-{}.png", uuid::Uuid::new_v4().simple());
            atomic_write(&root, &filename, &artifact.bytes)?;
            response.data["fileReference"] = json!(filename);
            response.data["media"] = json!(format!("MEDIA:{filename}"));
        }
        if response
            .data
            .get("snapshot")
            .and_then(Value::as_str)
            .is_some_and(|s| s.len() > 200 * 1024)
        {
            let root = self.lock().artifacts.clone().ok_or_else(unavailable)?;
            let filename = format!("browser-snapshot-{}.json", uuid::Uuid::new_v4().simple());
            atomic_write(&root, &filename, response.data.to_string().as_bytes())?;
            response.data = json!({"fileReference":root.join(filename),"kind":"snapshot"});
        }
        Ok(response.data)
    }
    pub(crate) fn screenshot_file(&self, reference: &str) -> Option<PathBuf> {
        if !reference.starts_with("browser-screenshot-")
            || !reference.ends_with(".png")
            || reference.contains(['/', '\\'])
        {
            return None;
        }
        let path = self.lock().screenshots.as_ref()?.join(reference);
        path.is_file().then_some(path)
    }
    pub(crate) fn stop_session(&self, id: &str) -> Result<(), AppError> {
        let session = self.lock().sessions.remove(id).ok_or_else(missing)?;
        self.transport(session.owner.kind())?.terminate_session(id);
        Ok(())
    }
    fn session_states(&self) -> Vec<Value> {
        let sessions: Vec<_> = {
            let s = self.lock();
            s.sessions
                .iter()
                .filter(|(_, session)| session.owner.kind() == BrowserTransportKind::Attended)
                .filter_map(|(id, session)| {
                    s.transports
                        .get(&session.owner.kind())
                        .map(|t| (id.clone(), t.clone()))
                })
                .collect()
        };
        sessions
            .into_iter()
            .filter_map(|(id, t)| t.session_state(&id).ok())
            .collect()
    }
    async fn takeover(&self, id: &str) -> Result<(), AppError> {
        let (t, operation) = {
            let s = self.lock();
            let session = s.sessions.get(id).ok_or_else(missing)?;
            if session.owner.kind() != BrowserTransportKind::Attended {
                return Err(err(
                    "browser_takeover_unavailable",
                    "Routine browsers cannot use the login profile.",
                ));
            }
            (
                s.transports
                    .get(&session.owner.kind())
                    .cloned()
                    .ok_or_else(unavailable)?,
                session.operation.clone(),
            )
        };
        let _operation = operation.lock().await;
        let data = t.human_takeover(id).await?.data;
        let tab = data["tabId"].as_i64().ok_or_else(unavailable)?;
        let mut s = self.lock();
        let session = s.sessions.get_mut(id).ok_or_else(missing)?;
        session.tabs.clear();
        session.tabs.insert(tab);
        Ok(())
    }
    async fn resume(&self, id: &str) -> Result<(), AppError> {
        let (t, operation) = {
            let s = self.lock();
            let session = s.sessions.get(id).ok_or_else(missing)?;
            (
                s.transports
                    .get(&session.owner.kind())
                    .cloned()
                    .ok_or_else(unavailable)?,
                session.operation.clone(),
            )
        };
        let _operation = operation.lock().await;
        t.resume_control(id).await?;
        Ok(())
    }
}

/// Only inspected, already value-redacted element facts may enter a card.
/// Page text remains explicitly untrusted and quoted, never an instruction
/// authored by Clovy. Input text, values, name/id and full URLs are excluded.
fn browser_action_preview(tool: &str, expected: &Inspection) -> String {
    fn text(value: &str, limit: usize) -> String {
        value
            .chars()
            .filter(|c| {
                !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            })
            .take(limit)
            .collect()
    }
    format!(
        "Untrusted page element label: {}\nElement tag: {}; role: {}; operation: {tool}\nOrigin: {}",
        json!(text(&expected.element.label, 128)),
        json!(text(&expected.element.tag, 32)),
        json!(text(&expected.element.role, 32)),
        text(&expected.origin, 256),
    )
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, AppError> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| err("invalid_arguments", &format!("{key} is required.")))
}
fn integer(v: &Value, key: &str) -> Result<i64, AppError> {
    v.get(key)
        .and_then(Value::as_i64)
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            err(
                "invalid_arguments",
                &format!("{key} must be a positive integer."),
            )
        })
}
fn atomic_write(root: &Path, name: &str, bytes: &[u8]) -> Result<(), AppError> {
    std::fs::create_dir_all(root).map_err(|_| unavailable())?;
    let temporary = root.join(format!(".{name}.tmp"));
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|_| unavailable())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| unavailable())?;
    }
    std::fs::rename(temporary, root.join(name)).map_err(|_| unavailable())
}
#[tauri::command]
pub(crate) fn browser_access_get(broker: tauri::State<'_, Arc<BrowserBroker>>) -> bool {
    broker.is_enabled()
}
#[tauri::command]
pub(crate) async fn browser_access_set(
    app: tauri::AppHandle,
    broker: tauri::State<'_, Arc<BrowserBroker>>,
    enabled: bool,
) -> Result<bool, AppError> {
    let _transition = broker.transition.lock().await;
    if broker.lock().closing {
        return Err(err(
            "browser_session_closed",
            "Browser control is shutting down.",
        ));
    }
    if enabled && !crate::experimental_settings::browser_use_enabled(&app) {
        return Err(err(
            "browser_use_disabled",
            "Enable Browser use in experimental settings first.",
        ));
    }
    let path = broker.lock().flag.clone().ok_or_else(unavailable)?;
    if enabled {
        atomic_write(
            path.parent().ok_or_else(unavailable)?,
            path.file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(unavailable)?,
            b"1",
        )?;
    } else {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(unavailable()),
        }
    }
    broker.set_enabled(enabled).await?;
    Ok(enabled)
}
#[tauri::command]
pub(crate) fn browser_sessions(broker: tauri::State<'_, Arc<BrowserBroker>>) -> Vec<Value> {
    broker.session_states()
}
#[tauri::command]
pub(crate) async fn browser_human_takeover(
    broker: tauri::State<'_, Arc<BrowserBroker>>,
    session_id: String,
) -> Result<(), AppError> {
    broker.takeover(&session_id).await
}
#[tauri::command]
pub(crate) async fn browser_resume_control(
    broker: tauri::State<'_, Arc<BrowserBroker>>,
    session_id: String,
) -> Result<(), AppError> {
    broker.resume(&session_id).await
}
#[tauri::command]
pub(crate) fn browser_stop_session(
    broker: tauri::State<'_, Arc<BrowserBroker>>,
    session_id: String,
) -> Result<(), AppError> {
    broker.stop_session(&session_id)
}
