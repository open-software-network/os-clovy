#![allow(dead_code)]
mod ca_bundle;
mod chat;
mod color;
mod commands;
mod connection;
mod doctor;
mod flags;
mod install;
#[allow(unused_imports)]
mod native;
mod output;
mod plugins;
mod read;
mod session_files;
mod skills;
#[cfg(test)]
mod test_utils;
mod tls;
mod upgrade;
mod validation;

mod clovy_browser_grant;
mod clovy_browser_process_identity;
#[allow(dead_code)] // Shared recovery implementation; host uses directory flush.
mod clovy_browser_recovery;
mod clovy_private_launch;
use native::{
    browser::{BrowserManager, WaitUntil},
    cdp::client::CdpClient,
    element::RefMap,
    snapshot::{take_snapshot, SnapshotOptions},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, Write},
};
const LIMIT: usize = 16 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    token: String,
    protocol: u32,
    grant: clovy_browser_grant::Grant,
    #[serde(rename = "jobName")]
    job_name: Option<String>,
}
#[derive(Deserialize)]
struct Request {
    id: u64,
    #[serde(flatten)]
    op: Operation,
}
#[derive(Deserialize)]
#[serde(tag = "action", deny_unknown_fields)]
enum Operation {
    #[serde(rename = "start")]
    Start {
        #[serde(rename = "chromePath")]
        chrome_path: String,
        #[serde(rename = "profilePath")]
        profile_path: String,
        #[serde(rename = "proxyPort")]
        proxy_port: u16,
        persistent: bool,
        headed: bool,
    },
    #[serde(rename = "navigate")]
    Navigate { tab_id: u32, url: String },
    #[serde(rename = "back")]
    Back { tab_id: u32 },
    #[serde(rename = "tab_open")]
    TabOpen { url: Option<String> },
    #[serde(rename = "tab_list")]
    TabList,
    #[serde(rename = "tab_switch")]
    TabSwitch { tab_id: u32 },
    #[serde(rename = "tab_close")]
    TabClose { tab_id: u32 },
    #[serde(rename = "snapshot")]
    Snapshot { tab_id: u32 },
    #[serde(rename = "screenshot")]
    Screenshot { tab_id: u32 },
    #[serde(rename = "inspect_reference")]
    Inspect {
        tab_id: u32,
        #[serde(rename = "ref")]
        reference: String,
    },
    #[serde(rename = "guarded_action")]
    Guarded {
        tab_id: u32,
        #[serde(rename = "ref")]
        reference: String,
        operation: String,
        text: Option<String>,
        key: Option<String>,
        expected: Value,
    },
    #[serde(rename = "suspend")]
    Suspend,
    #[serde(rename = "resume")]
    Resume,
    #[serde(rename = "takeover")]
    Takeover,
    #[serde(rename = "shutdown")]
    Shutdown,
}
#[derive(Clone)]
struct PrivateReference {
    tab: u32,
    node: i64,
    root: i64,
    document: String,
    // Never emitted: original metadata keeps exact guards even after value redaction.
    metadata: Option<(Value, Value)>,
}
struct GuardTarget {
    entry: PrivateReference,
    client: std::sync::Arc<CdpClient>,
    session: String,
    object: String,
    root: String,
    object_group: String,
}
impl GuardTarget {
    async fn release(&self) {
        release_guard_objects(&self.client, &self.session, &self.object_group).await;
    }
}
async fn release_guard_objects(client: &CdpClient, session: &str, group: &str) {
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        client.send_command(
            "Runtime.releaseObjectGroup",
            Some(json!({"objectGroup":group})),
            Some(session),
        ),
    )
    .await;
}
struct ObservationBarrier {
    client: std::sync::Arc<CdpClient>,
    session: String,
    events: tokio::sync::broadcast::Receiver<native::cdp::types::CdpEvent>,
    enabled: bool,
    breakpoint: Option<String>,
    nudge: Option<tokio::task::JoinHandle<Result<Value, String>>>,
    owned_pause: bool,
    context: Option<i64>,
    external_pause: bool,
    unexpectedly_resumed: bool,
}

impl ObservationBarrier {
    fn new(client: std::sync::Arc<CdpClient>, session: String) -> Self {
        let events = client.subscribe(); // Before enabling or requesting any pause.
        Self {
            client,
            session,
            events,
            enabled: false,
            breakpoint: None,
            nudge: None,
            owned_pause: false,
            context: None,
            external_pause: false,
            unexpectedly_resumed: false,
        }
    }

    fn notification(&mut self, event: native::cdp::types::CdpEvent) -> Result<(), String> {
        if event.session_id.as_deref() != Some(self.session.as_str()) {
            return Ok(());
        }
        match event.method.as_str() {
            "Debugger.paused" => {
                let Some(hits) = event.params["ownHits"].as_array() else {
                    self.external_pause = true;
                    return Err("effect_unknown".into());
                };
                if event.params["foreignPaused"] != false {
                    self.external_pause = true;
                    return Err("effect_unknown".into());
                }
                let owned = self
                    .breakpoint
                    .as_ref()
                    .is_some_and(|id| hits.len() == 1 && hits[0].as_str() == Some(id.as_str()));
                if !owned {
                    self.external_pause = true;
                    return Err("effect_unknown".into());
                }
                self.owned_pause = true;
            }
            "Debugger.resumed" => {
                if self.owned_pause {
                    self.unexpectedly_resumed = true;
                }
                self.owned_pause = false;
            }
            _ => {}
        }
        Ok(())
    }

    fn drain(&mut self) -> Result<(), String> {
        loop {
            match self.events.try_recv() {
                Ok(event) => self.notification(event)?,
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => return Ok(()),
                Err(_) => {
                    self.external_pause = true;
                    return Err("effect_unknown".into());
                }
            }
        }
    }

    async fn start(&mut self) -> Result<(), String> {
        self.enabled = true; // Cleanup must also handle a lost enable acknowledgement.
        self.client
            .send_command(
                "Debugger.enable",
                Some(json!({"maxScriptsCacheSize":0})),
                Some(&self.session),
            )
            .await?;
        // Exact V8 enableImpl reports a pre-existing pause before its reply.
        self.drain()?;
        if self.external_pause {
            return Err("effect_unknown".into());
        }
        let frame = self
            .client
            .send_command_no_params("Page.getFrameTree", Some(&self.session))
            .await?;
        let frame = frame["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or("redaction_failed")?;
        let world=self.client.send_command("Page.createIsolatedWorld",Some(json!({"frameId":frame,"worldName":"clovy-private-guard","grantUniveralAccess":false})),Some(&self.session)).await?;
        let context = world["executionContextId"]
            .as_i64()
            .ok_or("redaction_failed")?;
        self.context = Some(context);
        let marker = format!(
            "clovy-private-observation:{}",
            uuid::Uuid::new_v4().simple()
        );
        let breakpoint = self
            .client
            .send_command(
                "Debugger.setBreakpointByUrl",
                Some(json!({"lineNumber":0,"columnNumber":0,"url":marker})),
                Some(&self.session),
            )
            .await?;
        self.breakpoint = Some(
            breakpoint["breakpointId"]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 1024)
                .ok_or("redaction_failed")?
                .to_owned(),
        );
        self.client.register_observation_breakpoint(
            self.breakpoint.as_deref().ok_or("redaction_failed")?,
            &marker,
        )?;
        self.drain()?;
        let client = self.client.clone();
        let session = self.session.clone();
        self.nudge = Some(tokio::spawn(async move {
            client.send_command("Runtime.evaluate",Some(json!({"expression":format!("void 0;\n//# sourceURL={marker}"),"contextId":context,"returnByValue":true,"silent":true})),Some(&session)).await
        }));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !self.owned_pause {
                let event = match self.events.recv().await {
                    Ok(event) => event,
                    Err(_) => {
                        self.external_pause = true;
                        return Err("effect_unknown".to_owned());
                    }
                };
                self.notification(event)?;
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| "redaction_failed".to_owned())??;
        Ok(())
    }

    async fn visible(&self) -> Result<bool, String> {
        // Fixed synchronous isolated-world read of the native Document getter.
        // Never await page promises, emulate visibility, or activate a target.
        let context = self.context.ok_or("redaction_failed")?;
        let result = self.client.send_command(
            "Runtime.evaluate",
            Some(json!({"expression":"document.visibilityState","contextId":context,"returnByValue":true,"silent":true})),
            Some(&self.session),
        ).await?;
        if result.get("exceptionDetails").is_some() {
            return Err("redaction_failed".into());
        }
        match result["result"]["value"].as_str() {
            Some("visible") => Ok(true),
            Some("hidden") => Ok(false),
            _ => Err("redaction_failed".into()),
        }
    }

    async fn finish(&mut self) -> Result<(), String> {
        self.drain()?;
        if self.external_pause || self.unexpectedly_resumed {
            // Never release another debugger's pause. The host closes this task.
            return Err("effect_unknown".into());
        }
        let breakpoint = self.breakpoint.take();
        let removed = if let Some(id) = breakpoint.as_ref() {
            self.client
                .send_command(
                    "Debugger.removeBreakpoint",
                    Some(json!({"breakpointId":id})),
                    Some(&self.session),
                )
                .await
        } else {
            Ok(Value::Null)
        };
        self.drain()?;
        if self.external_pause || self.unexpectedly_resumed {
            return Err("effect_unknown".into());
        }
        let disabled = if self.enabled {
            self.client
                .send_command_no_params("Debugger.disable", Some(&self.session))
                .await
        } else {
            Ok(Value::Null)
        };
        // V8 releases this agent's pause only if no other accepting debugger
        // retains it. No global Debugger.resume command is ever issued.
        self.enabled = false;
        if let Some(id) = breakpoint.as_deref() {
            self.client.unregister_observation_breakpoint(id);
        }
        let completed = if let Some(nudge) = self.nudge.take() {
            matches!(tokio::time::timeout(std::time::Duration::from_secs(3),nudge).await,
                Ok(Ok(Ok(ref value))) if value.get("exceptionDetails").is_none())
        } else {
            true
        };
        self.owned_pause = false;
        if removed.is_err() || disabled.is_err() || !completed {
            return Err("effect_unknown".into());
        }
        Ok(())
    }
}

struct Host {
    grant: clovy_browser_grant::Grant,
    browser: Option<BrowserManager>,
    chrome: Option<clovy_private_launch::Chrome>,
    launch: Option<(String, String, u16, bool, bool)>,
    suspended: bool,
    tabs: HashMap<u32, (u32, String)>,
    next_tab: u32,
    refs: HashMap<String, PrivateReference>,
    owned: Option<clovy_browser_grant::OwnedLease>,
}
impl Host {
    fn new(
        grant: clovy_browser_grant::Grant,
        owned: Option<clovy_browser_grant::OwnedLease>,
    ) -> Self {
        Self {
            grant,
            browser: None,
            chrome: None,
            launch: None,
            suspended: false,
            tabs: HashMap::new(),
            next_tab: 1,
            refs: HashMap::new(),
            owned,
        }
    }
    fn valid_grant(&self) -> bool {
        match (&self.owned, self.grant.persistent) {
            (Some(owned), false) => owned.validate(&self.grant).is_ok(),
            (None, true) => self.grant.validate().is_ok(),
            _ => false,
        }
    }
    fn browser(&mut self) -> Result<&mut BrowserManager, String> {
        self.browser.as_mut().ok_or_else(|| "not_started".into())
    }
    fn register(&mut self) -> Result<Value, String> {
        let b = self.browser()?;
        let own = b.active_tab_id().ok_or("tab_gone")?;
        let target = b.active_target_id()?.to_owned();
        let id = self.next_tab;
        self.next_tab = self.next_tab.checked_add(1).ok_or("tab_limit")?;
        self.tabs.insert(id, (own, target.clone()));
        Ok(json!({"tabId":id,"targetId":target,"url":"about:blank"}))
    }
    async fn select(&mut self, tab: u32) -> Result<(), String> {
        let (own, target) = self.tabs.get(&tab).cloned().ok_or("tab_gone")?;
        let b = self.browser()?;
        let targets = b
            .client
            .send_command_no_params("Target.getTargets", None)
            .await?;
        if !targets["targetInfos"]
            .as_array()
            .is_some_and(|ts| ts.iter().any(|t| t["targetId"] == target))
        {
            self.tabs.remove(&tab);
            self.refs.retain(|_, entry| entry.tab != tab);
            return Err("tab_gone".into());
        }
        // Never use ensure_page / revive-and-replace paths on a private operation.
        b.select_private_tab(own)?;
        if b.active_target_id()? != target {
            return Err("tab_gone".into());
        }
        Ok(())
    }
    async fn close(&mut self) -> bool {
        self.refs.clear();
        self.tabs.clear();
        if let Some(mut b) = self.browser.take() {
            let _ = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                b.client.send_command_no_params("Browser.close", None),
            )
            .await;
            let _ = b.close().await;
        }
        if let Some(mut chrome) = self.chrome.take() {
            return chrome.wait_for_exit(std::time::Duration::from_millis(100));
        }
        true
    }
    async fn launch(&mut self) -> Result<Value, String> {
        let (c, p, port, persistent, headed) = self.launch.clone().ok_or("not_started")?;
        if self.grant.persistent != persistent
            || self.grant.profile.as_os_str() != std::ffi::OsStr::new(&p)
            || !self.valid_grant()
        {
            return Err("grant_mismatch".into());
        }
        let (chrome, r, w) = clovy_private_launch::launch(&c, &p, port, persistent, headed)?;
        self.chrome = Some(chrome);
        self.browser = Some(BrowserManager::from_private_pipe(CdpClient::from_pipe(r, w)?).await?);
        self.register()
    }
    async fn document(&mut self) -> Result<(i64, String), String> {
        let browser = self.browser()?;
        document_identity(&browser.client, browser.active_session_id()?).await
    }
    async fn guard_target(&mut self, tab: u32, reference: &str) -> Result<GuardTarget, String> {
        let entry = self
            .refs
            .get(reference.trim_start_matches('@'))
            .cloned()
            .ok_or("stale_reference")?;
        let (t, node, doc, document_id) =
            (entry.tab, entry.node, entry.root, entry.document.clone());
        if t != tab {
            return Err("stale_reference".into());
        }
        self.select(tab).await?;
        if self.document().await? != (doc, document_id.clone()) {
            return Err("stale_reference".into());
        }
        let b = self.browser()?;
        let sid = b.active_session_id()?;
        let tree = b
            .client
            .send_command_no_params("Page.getFrameTree", Some(sid))
            .await?;
        let frame = tree["frameTree"]["frame"]["id"]
            .as_str()
            .ok_or("document_unavailable")?;
        let world=b.client.send_command("Page.createIsolatedWorld",Some(json!({"frameId":frame,"worldName":"clovy-private-guard","grantUniveralAccess":false})),Some(sid)).await?;
        let context = world["executionContextId"]
            .as_i64()
            .ok_or("document_unavailable")?;
        let group = format!("clovy-guard-{}", uuid::Uuid::new_v4().simple());
        let result = async {
        let obj = b
            .client
            .send_command(
                "DOM.resolveNode",
                Some(json!({"backendNodeId":node,"executionContextId":context,"objectGroup":group})),
                Some(sid),
            )
            .await
            .map_err(|_| "stale_reference")?;
        let root = b
            .client
            .send_command(
                "DOM.resolveNode",
                Some(json!({"backendNodeId":doc,"executionContextId":context,"objectGroup":group})),
                Some(sid),
            )
            .await?;
        let object = obj["object"]["objectId"]
            .as_str()
            .ok_or("stale_reference")?;
        let root = root["object"]["objectId"]
            .as_str()
            .ok_or("stale_reference")?;
        // Recheck after resolving objects as well: a renderer can change between CDP calls.
        if document_identity(&b.client, sid).await? != (doc, document_id.clone()) {
            return Err("stale_reference".into());
        }
        Ok(GuardTarget {
            entry,
            client: b.client.clone(),
            session: sid.to_owned(),
            object: object.to_owned(),
            root: root.to_owned(),
            object_group: group.clone(),
        })
        }.await;
        if result.is_err() {
            release_guard_objects(&b.client, sid, &group).await;
        }
        result
    }
    async fn inspect_guard(
        &mut self,
        tab: u32,
        reference: &str,
        expected: Option<Value>,
        operation: Option<&str>,
        text: Option<&str>,
        key: Option<&str>,
        remember: bool,
    ) -> Result<Value, String> {
        self.select(tab).await?;
        let b = self.browser()?;
        let client = b.client.clone();
        let sid = b.active_session_id()?.to_owned();
        let mut observation = ObservationBarrier::new(client.clone(), sid.clone());
        let prepared = observation.start().await;
        let result = if prepared.is_ok() {
            self.inspect_observed(tab, reference, expected.as_ref(), remember)
                .await
        } else {
            Err("redaction_failed".to_owned())
        };
        let restored = observation.finish().await;
        if restored.is_err() {
            self.close().await;
            return Err("effect_unknown".into());
        }
        let (inspection, values) = result?;
        let Some(operation) = operation else {
            return Ok(inspection);
        };
        // Restore ordinary page/network scheduling before the one atomic guard+effect.
        // No metadata is emitted from the effect; exact raw metadata stays privately bound.
        let target = self.guard_target(tab, reference).await?;
        let result = async {
        let (saved, saved_raw) = target.entry.metadata.as_ref().ok_or("stale_reference")?;
        if expected.as_ref() != Some(saved) {
            return Err("stale_reference".into());
        }
        let r=target.client.send_command("Runtime.callFunctionOn",Some(json!({"objectId":target.object,"functionDeclaration":include_str!("clovy_private_guard.js"),"arguments":[{"objectId":target.root},{"value":target.entry.document},{"value":target.entry.node},{"value":saved},{"value":operation},{"value":text},{"value":key},{"value":saved_raw},{"value":values},{"value":saved["metadataId"]}],"returnByValue":true})),Some(&target.session)).await?;
        if r.get("exceptionDetails").is_some() {
            return Err("guard_execution_failed".into());
        }
        let value = &r["result"]["value"];
        if let Some(error) = value["error"].as_str() {
            return Err(error.into());
        }
        Ok(value["inspection"].clone())
        }.await;
        target.release().await;
        result
    }
    async fn inspect_observed(
        &mut self,
        tab: u32,
        reference: &str,
        expected: Option<&Value>,
        remember: bool,
    ) -> Result<(Value, Vec<String>), String> {
        let target = self.guard_target(tab, reference).await?;
        let result = async {
        let values = privacy_inventory(&target.client, &target.session, false)
            .await?
            .values;
        let probe=target.client.send_command("Runtime.callFunctionOn",Some(json!({"objectId":target.object,"functionDeclaration":include_str!("clovy_private_guard.js"),"arguments":[{"objectId":target.root},{"value":target.entry.document},{"value":target.entry.node},{"value":null},{"value":null},{"value":null},{"value":null},{"value":null},{"value":values},{"value":null}],"returnByValue":true})),Some(&target.session)).await?;
        if probe.get("exceptionDetails").is_some() {
            return Err("guard_execution_failed".into());
        }
        let observed = &probe["result"]["value"];
        if let Some(error) = observed["error"].as_str() {
            return Err(error.into());
        }
        let mut inspection = observed
            .get("inspection")
            .cloned()
            .ok_or("guard_execution_failed")?;
        let raw = observed
            .get("rawElement")
            .cloned()
            .ok_or("guard_execution_failed")?;
        // An opaque revision does not expose a digest oracle for low-entropy field metadata.
        // Reuse only unchanged raw facts and inspection core; another inspection of new
        // facts changes the revision and invalidates an older pending approval.
        let revision = target
            .entry
            .metadata
            .as_ref()
            .filter(|(prior, prior_raw)| {
                let mut core = prior.clone();
                core["metadataId"] = Value::Null;
                prior_raw == &raw && core == inspection
            })
            .and_then(|(prior, _)| prior["metadataId"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string());
        inspection["metadataId"] = json!(revision);
        if remember {
            self.refs
                .get_mut(reference.trim_start_matches('@'))
                .ok_or("stale_reference")?
                .metadata = Some((inspection.clone(), raw));
        } else {
            let (saved, saved_raw) = target.entry.metadata.as_ref().ok_or("stale_reference")?;
            if saved != &inspection
                || saved_raw != &raw
                || expected.is_some_and(|expected| expected != saved)
            {
                return Err("stale_reference".into());
            }
        }
        Ok((inspection, values))
        }.await;
        target.release().await;
        result
    }
    async fn execute(&mut self, op: Operation, attempted: &mut bool) -> Result<Value, String> {
        match op {
            Operation::Start {
                chrome_path,
                profile_path,
                proxy_port,
                persistent,
                headed,
            } => {
                if self.grant.persistent != persistent
                    || self.grant.profile.as_os_str() != std::ffi::OsStr::new(&profile_path)
                    || !self.valid_grant()
                {
                    return Err("grant_mismatch".into());
                }
                if self.launch.is_some() {
                    return Err("already_started".into());
                }
                self.launch = Some((chrome_path, profile_path, proxy_port, persistent, headed));
                *attempted = true;
                self.launch().await
            }
            Operation::Suspend => {
                self.suspended = true;
                self.refs.clear();
                Ok(json!({"suspended":true}))
            }
            Operation::Resume => {
                self.refs.clear();
                self.suspended = false;
                Ok(json!({"suspended":false}))
            }
            Operation::Takeover => {
                self.suspended = true;
                self.refs.clear();
                self.launch.as_ref().ok_or("not_started")?;
                *attempted = true;
                if !self.close().await {
                    return Err("close_incomplete".into());
                }
                self.launch.as_mut().ok_or("not_started")?.4 = true;
                let mut r = self.launch().await?;
                r["mode"] = json!("visible");
                r["suspended"] = json!(true);
                Ok(r)
            }
            Operation::Shutdown => {
                *attempted = true;
                self.close().await;
                Ok(json!({"closed":true}))
            }
            op => {
                if self.suspended {
                    return Err("human_takeover".into());
                }
                match op {
                    Operation::TabList => {
                        let ids: Vec<_> = self.tabs.keys().copied().collect();
                        let mut result = Vec::new();
                        for id in ids {
                            match self.select(id).await {
                                Err(e) if e == "tab_gone" => continue,
                                Err(e) => return Err(e),
                                Ok(()) => {}
                            }
                            {
                                let target = self.tabs[&id].1.clone();
                                let url = self.browser()?.get_url().await?;
                                result.push(json!({"tabId":id,"targetId":target,"url":url}));
                            }
                        }
                        Ok(json!({"tabs":result}))
                    }
                    Operation::TabOpen { url } => {
                        if let Some(ref u) = url {
                            validate_url(u)?;
                        }
                        *attempted = true;
                        self.browser()?.tab_new(None, None).await?;
                        let mut result = self.register()?;
                        if let Some(u) = url {
                            self.browser()?
                                .navigate(&u, WaitUntil::DomContentLoaded)
                                .await?;
                            result["url"] = json!(self.browser()?.get_url().await?);
                        }
                        Ok(result)
                    }
                    Operation::TabSwitch { tab_id } => {
                        self.select(tab_id).await?;
                        Ok(json!({"tabId":tab_id,"targetId":self.tabs[&tab_id].1}))
                    }
                    Operation::TabClose { tab_id } => {
                        self.select(tab_id).await?;
                        let target = self.tabs[&tab_id].1.clone();
                        *attempted = true;
                        let b = self.browser()?;
                        let result = b
                            .client
                            .send_command(
                                "Target.closeTarget",
                                Some(json!({"targetId":target})),
                                None,
                            )
                            .await?;
                        if result["success"] != true {
                            return Err("tab_gone".into());
                        }
                        b.remove_page_by_target_id(&target);
                        self.tabs.remove(&tab_id);
                        self.refs.retain(|_, entry| entry.tab != tab_id);
                        Ok(json!({"closed":true,"tabId":tab_id}))
                    }
                    Operation::Navigate { tab_id, url } => {
                        validate_url(&url)?;
                        self.select(tab_id).await?;
                        self.refs.retain(|_, entry| entry.tab != tab_id);
                        *attempted = true;
                        self.browser()?
                            .navigate(&url, WaitUntil::DomContentLoaded)
                            .await?;
                        Ok(json!({"url":self.browser()?.get_url().await?,"tabId":tab_id}))
                    }
                    Operation::Back { tab_id } => {
                        self.select(tab_id).await?;
                        let b = self.browser()?;
                        let sid = b.active_session_id()?.to_owned();
                        let h = b
                            .client
                            .send_command_no_params("Page.getNavigationHistory", Some(&sid))
                            .await?;
                        let i = h["currentIndex"].as_i64().ok_or("history_unavailable")?;
                        if i <= 0 {
                            return Err("no_history".into());
                        }
                        let destination = h["entries"][i as usize - 1]["url"]
                            .as_str()
                            .ok_or("history_unavailable")?;
                        if destination != "about:blank" {
                            validate_url(destination)?;
                        }
                        let entry = h["entries"][i as usize - 1]["id"]
                            .as_i64()
                            .ok_or("history_unavailable")?;
                        self.refs.retain(|_, entry| entry.tab != tab_id);
                        let b = self.browser()?;
                        let mut lifecycle = b.client.subscribe();
                        *attempted = true;
                        b.client
                            .send_command(
                                "Page.navigateToHistoryEntry",
                                Some(json!({"entryId":entry})),
                                Some(&sid),
                            )
                            .await?;
                        wait_history(b, &sid, entry, &mut lifecycle).await?;
                        Ok(json!({"url":b.get_url().await?,"tabId":tab_id}))
                    }
                    Operation::Snapshot { tab_id } => self.snapshot(tab_id).await,
                    Operation::Screenshot { tab_id } => {
                        self.select(tab_id).await?;
                        let document = self.document().await?;
                        let url = self.browser()?.get_url().await?;
                        if url != "about:blank" {
                            validate_url(&url)?;
                        }
                        let mut captured = self.screenshot(&document, &url).await?;
                        if self.document().await? != document
                            || self.browser()?.get_url().await? != url
                        {
                            return Err("stale_reference".into());
                        }
                        captured["url"] = json!(url);
                        captured["tabId"] = json!(tab_id);
                        Ok(captured)
                    }
                    Operation::Inspect { tab_id, reference } => {
                        self.inspect_guard(tab_id, &reference, None, None, None, None, true)
                            .await
                    }
                    Operation::Guarded {
                        tab_id,
                        reference,
                        operation,
                        text,
                        key,
                        expected,
                    } => {
                        if !matches!(operation.as_str(), "click" | "fill" | "press") {
                            return Err("unsupported_operation".into());
                        }
                        if text.as_ref().is_some_and(|s| s.len() > 1024 * 1024) {
                            return Err("text_too_large".into());
                        }
                        // Preflight rejects stale metadata without claiming an effect was attempted.
                        let observed = self
                            .inspect_guard(tab_id, &reference, None, None, None, None, false)
                            .await?;
                        if observed != expected {
                            return Err("stale_reference".into());
                        }
                        if operation == "fill" && text.is_none() {
                            return Err("text_required".into());
                        }
                        if operation == "press"
                            && !matches!(
                                key.as_deref(),
                                Some("Enter" | "Space" | "Escape" | "Backspace" | "Delete")
                            )
                        {
                            return Err("unsupported_key".into());
                        }
                        *attempted = true;
                        match self
                            .inspect_guard(
                                tab_id,
                                &reference,
                                Some(expected),
                                Some(&operation),
                                text.as_deref(),
                                key.as_deref(),
                                false,
                            )
                            .await
                        {
                            Ok(_) => {
                                Ok(json!({"tabId":tab_id,"url":self.browser()?.get_url().await?}))
                            }
                            Err(e)
                                if matches!(
                                    e.as_str(),
                                    "stale_reference"
                                        | "sensitive_field"
                                        | "unsupported_operation"
                                        | "unsupported_key"
                                ) =>
                            {
                                *attempted = false;
                                Err(e)
                            }
                            Err(e) => Err(e),
                        }
                    }
                    _ => Err("invalid_operation".into()),
                }
            }
        }
    }
    async fn snapshot(&mut self, tab_id: u32) -> Result<Value, String> {
        self.select(tab_id).await?;
        self.refs.retain(|_, entry| entry.tab != tab_id);
        let b = self.browser()?;
        let client = b.client.clone();
        let sid = b.active_session_id()?.to_owned();
        let mut observation = ObservationBarrier::new(client.clone(), sid.clone());
        let prepared = observation.start().await;
        let result = if prepared.is_ok() {
            self.snapshot_observed(tab_id).await
        } else {
            Err("redaction_failed".to_owned())
        };
        let restored = observation.finish().await;
        if restored.is_err() {
            self.close().await;
            return Err("effect_unknown".into());
        }
        result
    }
    async fn snapshot_observed(&mut self, tab_id: u32) -> Result<Value, String> {
        let (doc, document_id) = self.document().await?;
        let b = self.browser()?;
        let sid = b.active_session_id()?.to_owned();
        let (redactions, values) = redaction_nodes(&b.client, &sid).await?;
        b.client.set_redacted_nodes(redactions, values);
        let mut refs = RefMap::new();
        let mut snapshot = take_snapshot(
            &b.client,
            &sid,
            &SnapshotOptions {
                selector: None,
                interactive: false,
                compact: false,
                depth: Some(30),
                urls: false,
            },
            &mut refs,
            None,
            &HashMap::new(),
        )
        .await?;
        let url = observed_url(&b.client, &sid).await?;
        if self.document().await? != (doc, document_id.clone()) {
            return Err("stale_reference".into());
        }
        let mut substitutions = Vec::new();
        for (old, entry) in refs.entries_sorted() {
            let reference = uuid::Uuid::new_v4().simple().to_string();
            if entry.frame_id.is_none() {
                if let Some(node) = entry.backend_node_id {
                    self.refs.insert(
                        reference.clone(),
                        PrivateReference {
                            tab: tab_id,
                            node,
                            root: doc,
                            document: document_id.clone(),
                            metadata: None,
                        },
                    );
                }
            }
            substitutions.push((old, reference));
        }
        substitutions.sort_by_key(|(old, _)| std::cmp::Reverse(old.len()));
        for (old, new) in substitutions {
            snapshot = snapshot.replace(&format!("ref={old}]"), &format!("ref={new}]"));
        }
        Ok(json!({"snapshot":snapshot,"url":url,"tabId":tab_id}))
    }
    async fn screenshot(
        &mut self,
        expected_document: &(i64, String),
        expected_url: &str,
    ) -> Result<Value, String> {
        use base64::Engine;
        let b = self.browser()?;
        let client = b.client.clone();
        let sid = b.active_session_id()?.to_owned();
        let mut observation = ObservationBarrier::new(client.clone(), sid.clone());
        let prepared = observation.start().await;
        let mut original_playback_rate = None;
        let capture=async {
            if prepared.is_err(){return Err("redaction_failed".into());}
            if &document_identity(&client,&sid).await?!=expected_document||observed_url(&client,&sid).await?!=expected_url{return Err("stale_reference".into());}
            if !observation.visible().await? {return Err("screenshot_hidden".into());}
            client.send_command_no_params("Animation.enable",Some(&sid)).await?;
            let rate=client.send_command_no_params("Animation.getPlaybackRate",Some(&sid)).await?;
            original_playback_rate=Some(rate["playbackRate"].as_f64().filter(|v|v.is_finite()).ok_or("redaction_failed")?);
            client.send_command("Animation.setPlaybackRate",Some(json!({"playbackRate":0})),Some(&sid)).await?;
            let boxes=redaction_boxes(&b.client,&sid).await?;
            let metrics=b.client.send_command_no_params("Page.getLayoutMetrics",Some(&sid)).await?;
            let viewport=&metrics["cssVisualViewport"];
            if viewport["scale"].as_f64().is_some_and(|v|(v-1.0).abs()>0.001)||viewport["offsetX"].as_f64().is_some_and(|v|v.abs()>0.01)||viewport["offsetY"].as_f64().is_some_and(|v|v.abs()>0.01){return Err("redaction_failed".into());}
            let width=viewport["clientWidth"].as_f64().filter(|v|*v>0.0).ok_or("redaction_failed")?;
            let height=viewport["clientHeight"].as_f64().filter(|v|*v>0.0).ok_or("redaction_failed")?;
            let r=b.client.send_command("Page.captureScreenshot",Some(json!({"format":"png","captureBeyondViewport":false})),Some(&sid)).await?;
            let captured_boxes=redaction_boxes(&b.client,&sid).await?;
            let captured_metrics=b.client.send_command_no_params("Page.getLayoutMetrics",Some(&sid)).await?;
            if captured_boxes!=boxes || &captured_metrics["cssVisualViewport"]!=viewport{return Err("redaction_failed".into());}
            let data=r["data"].as_str().ok_or("screenshot_unavailable")?;let bytes=base64::engine::general_purpose::STANDARD.decode(data).map_err(|_|"screenshot_invalid")?;
            let mut img=image::load_from_memory(&bytes).map_err(|_|"screenshot_invalid")?.to_rgba8();
            let sx=img.width() as f64/width;let sy=img.height() as f64/height;
            if let Some(boxes)=boxes.as_array(){for rect in boxes {let x=((rect[0].as_f64().unwrap_or(0.0)-32.0)*sx).floor().max(0.0) as u32;let y=((rect[1].as_f64().unwrap_or(0.0)-32.0)*sy).floor().max(0.0) as u32;let right=((rect[0].as_f64().unwrap_or(0.0)+rect[2].as_f64().unwrap_or(0.0)+32.0)*sx).ceil().max(0.0) as u32;let bottom=((rect[1].as_f64().unwrap_or(0.0)+rect[3].as_f64().unwrap_or(0.0)+32.0)*sy).ceil().max(0.0) as u32;for yy in y..bottom.min(img.height()){for xx in x..right.min(img.width()){img.put_pixel(xx,yy,image::Rgba([24,24,24,255]));}}}}
            let mut output=std::io::Cursor::new(Vec::new());img.write_to(&mut output,image::ImageFormat::Png).map_err(|_|"screenshot_encoding")?;Ok::<_,String>(json!({"base64":base64::engine::general_purpose::STANDARD.encode(output.into_inner())}))
        }.await;
        // The owned debugger pause holds page tasks. Virtual time is never
        // enabled: advancing it would not restore the native wall clock.
        // Restore the observed animation rate even after a lost setup reply.
        let animation_restored = if let Some(rate) = original_playback_rate {
            client
                .send_command(
                    "Animation.setPlaybackRate",
                    Some(json!({"playbackRate":rate})),
                    Some(&sid),
                )
                .await
                .map(|_| ())
        } else {
            Ok(())
        };
        let active_restored = observation.finish().await;
        if animation_restored.is_err() || active_restored.is_err() {
            self.close().await;
            return Err("effect_unknown".into());
        }
        capture
    }
}
async fn observed_url(client: &CdpClient, sid: &str) -> Result<String, String> {
    // A promise-awaiting Runtime.evaluate cannot finish while our debugger
    // pause holds page tasks. The native frame URL needs no page evaluation.
    let tree = client
        .send_command_no_params("Page.getFrameTree", Some(sid))
        .await?;
    let frame = &tree["frameTree"]["frame"];
    let mut url = frame["url"]
        .as_str()
        .filter(|url| !url.is_empty())
        .ok_or("document_unavailable")?
        .to_owned();
    if let Some(fragment) = frame.get("urlFragment") {
        let fragment = fragment
            .as_str()
            .filter(|fragment| fragment.starts_with('#'))
            .ok_or("document_unavailable")?;
        url.push_str(fragment);
    }
    Ok(url)
}
async fn document_identity(client: &CdpClient, sid: &str) -> Result<(i64, String), String> {
    let document = client
        .send_command("DOM.getDocument", Some(json!({"depth":0})), Some(sid))
        .await?;
    let root = document["root"]["backendNodeId"]
        .as_i64()
        .ok_or("document_unavailable")?;
    let tree = client
        .send_command_no_params("Page.getFrameTree", Some(sid))
        .await?;
    let frame = &tree["frameTree"]["frame"];
    let frame_id = frame["id"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("document_unavailable")?;
    let loader_id = frame["loaderId"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("document_unavailable")?;
    Ok((root, json!([frame_id, loader_id, root]).to_string()))
}
async fn wait_history(
    browser: &BrowserManager,
    sid: &str,
    entry: i64,
    events: &mut tokio::sync::broadcast::Receiver<native::cdp::types::CdpEvent>,
) -> Result<(), String> {
    // Subscribe before dispatch, and confirm the actual committed history entry.
    // A cached or same-document back may have no future DOMContentLoaded event.
    tokio::time::timeout(std::time::Duration::from_secs(15),async {
        loop {
            let h=browser.client.send_command_no_params("Page.getNavigationHistory",Some(sid)).await?;
            if let Some(index)=h["currentIndex"].as_u64(){if h["entries"][index as usize]["id"].as_i64()==Some(entry){return browser.wait_for_lifecycle_external(WaitUntil::DomContentLoaded,sid,15000).await;}}
            tokio::select!{_=events.recv()=>{},_=tokio::time::sleep(std::time::Duration::from_millis(20))=>{}}
        }
    }).await.map_err(|_|"history_timeout".to_string())?
}
struct PrivacyInventory {
    nodes: std::collections::HashSet<i64>,
    values: Vec<String>,
    boxes: Value,
}

// Native capture flattens DOM/shadow/pseudo nodes and obtains layout/text bounds in
// one renderer observation. No page-overridable geometry or stale field getter is used.
async fn privacy_inventory(
    client: &CdpClient,
    sid: &str,
    geometry: bool,
) -> Result<PrivacyInventory, String> {
    let snapshot = client
        .send_command(
            "DOMSnapshot.captureSnapshot",
            Some(json!({
                "computedStyles":["-webkit-user-modify","text-shadow","filter","backdrop-filter","-webkit-box-reflect","transform","translate","rotate","scale","-webkit-text-stroke-width","text-emphasis-style"],
                "includeDOMRects":true
            })),
            Some(sid),
        )
        .await?;
    let tree = client
        .send_command_no_params("Page.getFrameTree", Some(sid))
        .await?;
    let main_frame = tree["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or("redaction_failed")?;
    parse_privacy_snapshot(&snapshot, main_frame, geometry)
}

fn native_snapshot_string<'a>(
    strings: &'a [Value],
    index: &Value,
    allow_empty: bool,
) -> Result<&'a str, String> {
    let index = index.as_i64().ok_or("redaction_failed")?;
    if index == -1 && allow_empty {
        return Ok("");
    }
    if index < 0 {
        return Err("redaction_failed".into());
    }
    let index = usize::try_from(index).map_err(|_| "redaction_failed")?;
    strings
        .get(index)
        .and_then(Value::as_str)
        .filter(|value| allow_empty || !value.is_empty())
        .ok_or_else(|| "redaction_failed".into())
}
fn parse_privacy_snapshot(
    snapshot: &Value,
    main_frame: &str,
    geometry: bool,
) -> Result<PrivacyInventory, String> {
    use std::collections::{HashMap, HashSet};
    let strings = snapshot["strings"].as_array().ok_or("redaction_failed")?;
    let string = |value: &Value| native_snapshot_string(strings, value, false);
    let text = |value: &Value| native_snapshot_string(strings, value, true);
    let documents = snapshot["documents"].as_array().ok_or("redaction_failed")?;
    let mut private = HashSet::new();
    let mut values = Vec::new();
    let mut boxes = Vec::new();
    let mut total_nodes = 0usize;
    let mut value_bytes = 0usize;
    let mut add_value = |value: &str| -> Result<(), String> {
        if value.is_empty() {
            return Ok(());
        }
        value_bytes = value_bytes.saturating_add(value.len());
        if value.len() > 1024 * 1024 || value_bytes > 8 * 1024 * 1024 || values.len() >= 4000 {
            return Err("redaction_failed".into());
        }
        values.push(value.to_owned());
        let trimmed = value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
        if !trimmed.is_empty() && trimmed != value {
            values.push(trimmed.to_owned());
        }
        let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
        if normalized != value && !normalized.is_empty() {
            values.push(normalized);
        }
        Ok(())
    };
    let mut found_main = false;
    for document in documents {
        let is_main = string(&document["frameId"])? == main_frame;
        found_main |= is_main;
        let nodes = &document["nodes"];
        let names = nodes["nodeName"].as_array().ok_or("redaction_failed")?;
        let n = names.len();
        total_nodes = total_nodes.saturating_add(n);
        if total_nodes > 50_000 {
            return Err("redaction_failed".into());
        }
        let parents = nodes["parentIndex"]
            .as_array()
            .filter(|v| v.len() == n)
            .ok_or("redaction_failed")?;
        let backend = nodes["backendNodeId"]
            .as_array()
            .filter(|v| v.len() == n)
            .ok_or("redaction_failed")?;
        let attrs = nodes["attributes"]
            .as_array()
            .filter(|v| v.len() == n)
            .ok_or("redaction_failed")?;
        let node_values = nodes["nodeValue"]
            .as_array()
            .filter(|v| v.len() == n)
            .ok_or("redaction_failed")?;
        let node_types = nodes["nodeType"]
            .as_array()
            .filter(|v| v.len() == n)
            .ok_or("redaction_failed")?;
        let layout = &document["layout"];
        let indexes = layout["nodeIndex"].as_array().ok_or("redaction_failed")?;
        let bounds = layout["bounds"]
            .as_array()
            .filter(|v| v.len() == indexes.len())
            .ok_or("redaction_failed")?;
        let styles = layout["styles"]
            .as_array()
            .filter(|v| v.len() == indexes.len())
            .ok_or("redaction_failed")?;
        let layout_text = layout["text"]
            .as_array()
            .filter(|v| v.len() == indexes.len())
            .ok_or("redaction_failed")?;
        let mut editable = vec![false; n];
        let mut control = vec![false; n];
        let mut control_root = vec![false; n];
        let mut value_field = vec![false; n];
        let mut iframe = vec![false; n];
        let mut owner = vec![None; n];
        let mut paint_effect = vec![false; n];
        for i in 0..n {
            let tag = string(&names[i])?.to_ascii_lowercase();
            control_root[i] = matches!(tag.as_str(), "input" | "textarea" | "select");
            control[i] = control_root[i];
            iframe[i] = matches!(tag.as_str(), "iframe" | "frame");
            let attributes = attrs[i].as_array().ok_or("redaction_failed")?;
            if attributes.len() % 2 != 0 {
                return Err("redaction_failed".into());
            }
            let input_type = attributes
                .chunks_exact(2)
                .find_map(|pair| {
                    if string(&pair[0])
                        .ok()
                        .is_some_and(|name| name.eq_ignore_ascii_case("type"))
                    {
                        text(&pair[1]).ok()
                    } else {
                        None
                    }
                })
                .unwrap_or("text")
                .to_ascii_lowercase();
            value_field[i] = tag == "textarea"
                || (tag == "input"
                    && !matches!(
                        input_type.as_str(),
                        "button" | "submit" | "reset" | "checkbox" | "radio" | "image"
                    ));
            for pair in attributes.chunks_exact(2) {
                if string(&pair[0])?.eq_ignore_ascii_case("contenteditable")
                    && !text(&pair[1])?.eq_ignore_ascii_case("false")
                {
                    editable[i] = true;
                }
            }
        }
        for (li, index) in indexes.iter().enumerate() {
            let i = index
                .as_u64()
                .filter(|i| (*i as usize) < n)
                .ok_or("redaction_failed")? as usize;
            let css = styles[li].as_array().ok_or("redaction_failed")?;
            if css.is_empty() && node_types[i].as_u64() == Some(9) {
                continue;
            }
            if css.len() != 11 {
                return Err("redaction_failed".into());
            }
            paint_effect[i] =
                css.iter()
                    .enumerate()
                    .skip(1)
                    .any(|(index, value)| match string(value) {
                        Ok(value) => {
                            if index == 9 {
                                !value.is_empty()
                                    && value.trim_end_matches("px").parse::<f64>().ok() != Some(0.0)
                            } else {
                                value != "none" && !value.is_empty()
                            }
                        }
                        Err(_) => true,
                    });
            // Effective native editability covers inherited/design-mode and shadow layout.
            let modify = string(&css[0])?;
            if !matches!(modify, "read-only" | "none" | "") && !control_root[i] {
                editable[i] = true;
            }
        }
        for i in 0..n {
            let parent = parents[i].as_i64().ok_or("redaction_failed")?;
            if parent >= 0 {
                let parent = parent as usize;
                if parent >= i {
                    return Err("redaction_failed".into());
                }
                editable[i] = editable[i] || editable[parent];
                control[i] = control[i] || control[parent];
                owner[i] = owner[parent];
                paint_effect[i] = paint_effect[i] || paint_effect[parent];
            }
            if editable[i] && owner[i].is_none() {
                owner[i] = Some(i);
            }
            if editable[i] || (control[i] && !control_root[i]) {
                private.insert(backend[i].as_i64().ok_or("redaction_failed")?);
            }
        }
        for property in ["inputValue", "textValue"] {
            if nodes.get(property).is_none() {
                continue;
            }
            let indices = nodes[property]["index"]
                .as_array()
                .ok_or("redaction_failed")?;
            let content = nodes[property]["value"]
                .as_array()
                .filter(|v| v.len() == indices.len())
                .ok_or("redaction_failed")?;
            for (index, content) in indices.iter().zip(content) {
                let i = index
                    .as_u64()
                    .filter(|i| (*i as usize) < n)
                    .ok_or("redaction_failed")? as usize;
                if value_field[i] {
                    add_value(text(content)?)?;
                }
            }
        }
        for (li, index) in indexes.iter().enumerate() {
            let index = index.as_u64().ok_or("redaction_failed")? as usize;
            if editable[index] || (control[index] && !control_root[index]) {
                let index = layout_text[li]
                    .as_i64()
                    .filter(|i| *i >= -1)
                    .ok_or("redaction_failed")?;
                if index >= 0 {
                    add_value(text(&layout_text[li])?)?;
                }
            }
        }
        let mut selected = HashSet::new();
        if let Some(option) = nodes.get("optionSelected") {
            for index in option["index"].as_array().ok_or("redaction_failed")? {
                let index = index
                    .as_u64()
                    .filter(|i| (*i as usize) < n)
                    .ok_or("redaction_failed")? as usize;
                selected.insert(index);
                let attributes = attrs[index].as_array().ok_or("redaction_failed")?;
                for pair in attributes.chunks_exact(2) {
                    if string(&pair[0])?.eq_ignore_ascii_case("value") {
                        add_value(text(&pair[1])?)?;
                    }
                }
            }
        }
        let mut selected_owner = vec![None; n];
        let mut selected_text = HashMap::<usize, String>::new();
        for i in 0..n {
            if let Some(parent) = parents[i].as_i64().filter(|p| *p >= 0) {
                selected_owner[i] = selected_owner[parent as usize];
            }
            if selected.contains(&i) {
                selected_owner[i] = Some(i);
            }
            if matches!(node_types[i].as_u64(), Some(3 | 4)) {
                if let Some(root) = selected_owner[i] {
                    let text = text(&node_values[i])?;
                    let joined = selected_text.entry(root).or_default();
                    if joined.len().saturating_add(text.len()) > 1024 * 1024 {
                        return Err("redaction_failed".into());
                    }
                    joined.push_str(text);
                }
            }
        }
        for text in selected_text.values() {
            add_value(text)?;
        }
        let mut editable_text = HashMap::<usize, String>::new();
        for i in 0..n {
            if matches!(node_types[i].as_u64(), Some(3 | 4)) {
                if let Some(root) = owner[i] {
                    let text = text(&node_values[i])?;
                    add_value(text)?;
                    let joined = editable_text.entry(root).or_default();
                    if joined.len().saturating_add(text.len()) > 1024 * 1024 {
                        return Err("redaction_failed".into());
                    }
                    joined.push_str(text);
                }
            }
        }
        for text in editable_text.values() {
            add_value(text)?;
        }
        if !geometry || !is_main {
            continue;
        }
        let scroll_x = document["scrollOffsetX"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("redaction_failed")?;
        let scroll_y = document["scrollOffsetY"]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("redaction_failed")?;
        let mut protected_layout = vec![false; indexes.len()];
        let mut add_box = |rect: &Value| -> Result<(), String> {
            let rect = rect
                .as_array()
                .filter(|r| r.len() == 4)
                .ok_or("redaction_failed")?;
            let r = rect
                .iter()
                .map(|v| {
                    v.as_f64()
                        .filter(|v| v.is_finite())
                        .ok_or_else(|| "redaction_failed".to_owned())
                })
                .collect::<Result<Vec<_>, _>>()?;
            if r[2] < 0.0 || r[3] < 0.0 {
                return Err("redaction_failed".into());
            }
            if r[2] > 0.0 && r[3] > 0.0 {
                boxes.push(json!([
                    r[0] - scroll_x - 3.0,
                    r[1] - scroll_y - 3.0,
                    r[2] + 6.0,
                    r[3] + 6.0
                ]));
            }
            Ok(())
        };
        for (li, index) in indexes.iter().enumerate() {
            let i = index.as_u64().ok_or("redaction_failed")? as usize;
            protected_layout[li] = editable[i] || control[i] || iframe[i];
            if !protected_layout[li] {
                continue;
            }
            // Paint effects can extend glyphs beyond native layout bounds; do not guess.
            if paint_effect[i] {
                return Err("redaction_failed".into());
            }
            add_box(&bounds[li])?;
        }
        let text_boxes = &document["textBoxes"];
        let text_layout = text_boxes["layoutIndex"]
            .as_array()
            .ok_or("redaction_failed")?;
        let text_bounds = text_boxes["bounds"]
            .as_array()
            .filter(|v| v.len() == text_layout.len())
            .ok_or("redaction_failed")?;
        for (index, bounds) in text_layout.iter().zip(text_bounds) {
            let i = index
                .as_u64()
                .filter(|i| (*i as usize) < protected_layout.len())
                .ok_or("redaction_failed")? as usize;
            if protected_layout[i] {
                add_box(bounds)?;
            }
        }
    }
    if !found_main {
        return Err("redaction_failed".into());
    }
    drop(add_value);
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    values.dedup();
    if boxes.len() > 20_000 {
        return Err("redaction_failed".into());
    }
    Ok(PrivacyInventory {
        nodes: private,
        values,
        boxes: json!(boxes),
    })
}
async fn redaction_boxes(client: &CdpClient, sid: &str) -> Result<Value, String> {
    Ok(privacy_inventory(client, sid, true).await?.boxes)
}
async fn redaction_nodes(
    client: &CdpClient,
    sid: &str,
) -> Result<(std::collections::HashSet<i64>, Vec<String>), String> {
    let inventory = privacy_inventory(client, sid, false).await?;
    Ok((inventory.nodes, inventory.values))
}
fn host_error_code(code: &str) -> &'static str {
    // Upstream diagnostics may contain URLs and page text. They never cross the private wire.
    const CODES: &[&str] = &[
        "tab_gone",
        "stale_reference",
        "not_started",
        "already_started",
        "tab_limit",
        "document_unavailable",
        "human_takeover",
        "unsupported_operation",
        "unsupported_key",
        "sensitive_field",
        "text_too_large",
        "text_required",
        "invalid_operation",
        "invalid_url",
        "no_history",
        "history_unavailable",
        "history_timeout",
        "guard_execution_failed",
        "redaction_failed",
        "screenshot_unavailable",
        "screenshot_hidden",
        "screenshot_invalid",
        "screenshot_encoding",
        "effect_unknown",
        "close_incomplete",
    ];
    CODES
        .iter()
        .copied()
        .find(|known| *known == code)
        .unwrap_or("browser_operation_failed")
}
fn validate_url(u: &str) -> Result<(), String> {
    let u = url::Url::parse(u).map_err(|_| "invalid_url")?;
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some()
    {
        return Err("invalid_url".into());
    }
    Ok(())
}
fn line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let n = std::io::Read::take(reader, (LIMIT + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "wire_closed")?;
    if n == 0 {
        return Ok(None);
    }
    if bytes.len() > LIMIT || bytes.last() != Some(&b'\n') {
        return Err("wire_limit".into());
    }
    Ok(Some(bytes))
}
fn output(value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "wire_encoding")?;
    if bytes.len() > LIMIT {
        return Err("wire_limit".into());
    }
    let out = std::io::stdout();
    let mut out = out.lock();
    out.write_all(&bytes)
        .and_then(|_| out.write_all(b"\n"))
        .and_then(|_| out.flush())
        .map_err(|_| "wire_closed".into())
}
#[tokio::main]
async fn main() {
    if std::env::args_os().len() != 1 {
        std::process::exit(2);
    }
    if run().await.is_err() {
        std::process::exit(1);
    }
}
async fn run() -> Result<(), String> {
    clovy_browser_process_identity::process_birth(std::process::id())
        .map_err(|_| "process_identity_unavailable")?;
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    std::thread::spawn(move || {
        let input = std::io::stdin();
        let mut reader = input.lock();
        loop {
            let r = line(&mut reader);
            let stop = !matches!(&r, Ok(Some(_)));
            if tx.blocking_send(r).is_err() || stop {
                break;
            }
        }
    });
    let bytes = rx
        .recv()
        .await
        .ok_or("wire_closed")??
        .ok_or("wire_closed")?;
    let hello: Hello = serde_json::from_slice(&bytes).map_err(|_| "invalid_handshake")?;
    if hello.protocol != 1
        || hello.token.len() != 64
        || !hello
            .token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err("invalid_handshake".into());
    }
    hello.grant.validate().map_err(|_| "invalid_grant")?;
    let owned = if hello.grant.persistent {
        if hello.job_name.is_some() {
            return Err("invalid_grant".into());
        }
        None
    } else {
        Some(
            hello
                .grant
                .acquire_owned(hello.job_name.as_deref())
                .map_err(|_| "invalid_grant")?,
        )
    };
    if let Some(owned) = &owned {
        output(&json!({"protocol":1,"token":hello.token,"owned":owned.identity}))?;
    } else {
        output(&json!({"protocol":1,"token":hello.token}))?;
    }
    let mut host = Host::new(hello.grant, owned);
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|_| "signal_unavailable")?;
    loop {
        #[cfg(unix)]
        let next =
            tokio::select! {v=rx.recv()=>v,_=term.recv()=>{host.close().await;return Ok(());}};
        #[cfg(not(unix))]
        let next = rx.recv().await;
        let Some(Ok(Some(bytes))) = next else {
            host.close().await;
            return Ok(());
        };
        let request: Request = match serde_json::from_slice(&bytes) {
            Ok(r) => r,
            Err(_) => {
                output(
                    &json!({"id":0,"delivery":"not_attempted","error":{"code":"invalid_request","message":"Invalid private host request"}}),
                )?;
                continue;
            }
        };
        let shutdown = matches!(request.op, Operation::Shutdown);
        let mut attempted = false;
        #[cfg(unix)]
        let result = tokio::select! {result=host.execute(request.op,&mut attempted)=>result,_=term.recv()=>{host.close().await;return Ok(());}};
        #[cfg(not(unix))]
        let result = host.execute(request.op, &mut attempted).await;
        // A failed observation restoration poisons this browser task even on a read.
        // The supervisor must reap the slot and invalidate every advertised handle.
        if result
            .as_ref()
            .err()
            .is_some_and(|code| code == "effect_unknown")
        {
            attempted = true;
        }
        if result.is_err() && attempted {
            host.refs.clear();
        }
        let response = match result {
            Ok(data) => {
                json!({"id":request.id,"delivery":if attempted{"delivered"}else{"not_attempted"},"data":data})
            }
            Err(code) => {
                json!({"id":request.id,"delivery":if attempted{"unknown"}else{"not_attempted"},"error":{"code":host_error_code(&code),"message":"Browser operation failed"}})
            }
        };
        if output(&response).is_err() {
            host.close().await;
            return Err("wire_closed".into());
        }
        if shutdown {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod private_inventory_tests {
    use super::*;

    fn empty_fields() -> Value {
        // Native156 AddString("") emits -1. There is intentionally NO empty
        // string in this table: the producer's signed sentinel is exercised.
        json!({"strings":["frame","#document","INPUT","TEXTAREA","read-only","none","0px"],"documents":[{
            "frameId":0,"scrollOffsetX":0,"scrollOffsetY":0,
            "nodes":{"parentIndex":[-1,0,0],"nodeType":[9,1,1],"nodeName":[1,2,3],"nodeValue":[-1,-1,-1],"backendNodeId":[1,2,3],"attributes":[[],[],[]],"inputValue":{"index":[1],"value":[-1]},"textValue":{"index":[2],"value":[-1]}},
            "layout":{"nodeIndex":[0,1,2],"bounds":[[0,0,800,600],[10,10,100,20],[10,40,100,40]],"styles":[[],[4,5,5,5,5,5,5,5,5,6,5],[4,5,5,5,5,5,5,5,5,6,5]],"text":[-1,-1,-1]},
            "textBoxes":{"layoutIndex":[],"bounds":[]}
        }]})
    }

    #[test]
    fn actual_parser_accepts_native_empty_input_and_textarea_sentinels() {
        let raw = empty_fields();
        let inventory = parse_privacy_snapshot(&raw, "frame", true).unwrap();
        assert!(inventory.values.is_empty());
        assert_eq!(inventory.boxes.as_array().unwrap().len(), 2);
    }

    #[test]
    fn actual_parser_rejects_malformed_signed_string_indices() {
        for property in ["inputValue", "textValue"] {
            for invalid in [
                json!(-2),
                json!(1000),
                json!("0"),
                json!(true),
                Value::Null,
                json!(0.5),
            ] {
                let mut raw = empty_fields();
                raw["documents"][0]["nodes"][property]["value"][0] = invalid;
                assert!(parse_privacy_snapshot(&raw, "frame", false).is_err());
            }
        }
        for field in ["frameId", "nodeName", "style"] {
            let mut raw = empty_fields();
            match field {
                "frameId" => raw["documents"][0]["frameId"] = json!(-1),
                "nodeName" => raw["documents"][0]["nodes"]["nodeName"][1] = json!(-1),
                _ => raw["documents"][0]["layout"]["styles"][1][0] = json!(-1),
            }
            assert!(parse_privacy_snapshot(&raw, "frame", false).is_err());
        }
    }

    #[test]
    fn empty_attribute_content_and_text_do_not_weaken_required_name_validation() {
        let mut raw = empty_fields();
        raw["strings"]
            .as_array_mut()
            .unwrap()
            .extend([json!("contenteditable"), json!("#text")]);
        raw["documents"][0]["nodes"]["attributes"][1] = json!([7, -1]);
        raw["documents"][0]["nodes"]["parentIndex"][2] = json!(1);
        raw["documents"][0]["nodes"]["nodeType"][2] = json!(3);
        raw["documents"][0]["nodes"]["nodeName"][2] = json!(8);
        assert!(parse_privacy_snapshot(&raw, "frame", false).is_ok());
        raw["documents"][0]["nodes"]["attributes"][1][0] = json!(-1);
        assert!(parse_privacy_snapshot(&raw, "frame", false).is_err());
    }
}
