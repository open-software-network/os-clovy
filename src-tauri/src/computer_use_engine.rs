//! The private Clovy/agent-desktop translation boundary. The model never sees
//! argv, namespaces, executable paths, or upstream's unrestricted command set.
use std::collections::HashMap;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::{json, Value};

use crate::computer_use_host::CommandHost;

#[cfg(target_os = "macos")]
#[path = "computer_use_engine/stage.rs"]
mod stage;
#[path = "computer_use_engine/vision.rs"]
mod vision;

const MAX_ELEMENTS: usize = 2_000;
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;

#[derive(Clone, Debug)]
struct Window {
    public_id: u64,
    raw: Value,
}

impl Window {
    fn id(&self) -> &str {
        self.raw["id"].as_str().unwrap_or("")
    }
    fn pid(&self) -> i64 {
        self.raw["pid"].as_i64().unwrap_or(0)
    }
    fn geometry(&self) -> Option<[f64; 4]> {
        geometry(&self.raw["bounds"])
    }
    fn same_identity(&self, other: &Self) -> bool {
        self.id() == other.id()
            && self.pid() == other.pid()
            && self.raw["process_instance"] == other.raw["process_instance"]
            && self.raw["native_owner"] == other.raw["native_owner"]
            && self.raw["process_instance"]
                .as_str()
                .is_some_and(|s| !s.is_empty())
    }
    fn same_capture(&self, other: &Self) -> bool {
        self.same_identity(other)
            && self.geometry().is_some()
            && self.geometry() == other.geometry()
            && self.raw["title"] == other.raw["title"]
    }
    fn public(&self, z_index: usize) -> Value {
        json!({"pid":self.pid(),"window_id":self.public_id,"app_name":self.raw["app_name"],
            "title":self.raw["title"],"bounds":self.raw["bounds"],"z_index":z_index,
            "is_on_screen":self.raw["visible"].as_bool().unwrap_or(false),
            "process_instance":self.raw["process_instance"],"native_owner":self.raw["native_owner"],"accessible":self.raw["accessible"],
            "is_focused":self.raw["is_focused"]})
    }
}

#[derive(Clone, Debug)]
struct Element {
    reference: String,
    node: Value,
    native: Value,
}

#[derive(Clone)]
struct Capture {
    window: Window,
    elements: HashMap<u32, Element>,
    width: u32,
    height: u32,
    mode: String,
    complete: bool,
}

pub(crate) struct EngineAdapter {
    host: CommandHost,
    windows: HashMap<u64, Window>,
    captures: HashMap<(i64, u64), Capture>,
    next_public_element: u32,
}

impl EngineAdapter {
    pub(crate) fn new(host: CommandHost) -> Self {
        Self {
            host,
            windows: HashMap::new(),
            captures: HashMap::new(),
            next_public_element: 1,
        }
    }

    pub(crate) async fn invoke(&mut self, name: &str, args: Value) -> Value {
        if self.host.cancel_handle().is_cancelled()
            || crate::computer_use_cancel::current_run_cancelled()
        {
            return failure(
                "computer_use_stopped",
                "The computer-use run was stopped.",
                false,
            );
        }
        #[cfg(target_os = "windows")]
        if crate::computer_use_windows::readiness().is_err() {
            return failure("computer_use_desktop_unavailable","The interactive Windows desktop is unavailable. Unlock it and start a new computer-use run.",false);
        }
        let result = self.invoke_inner(name, &args).await;
        match result {
            Ok(value) => value,
            Err(value) => value,
        }
    }

    async fn engine(&mut self, argv: Vec<String>, mutation: bool) -> Result<Value, Value> {
        match self.host.execute(argv).await {
            Ok(response) if response["version"] == "2.5" && response["ok"].is_boolean() => Ok(response),
            Ok(_) => Err(failure("computer_use_engine_protocol", "The Computer use engine returned an incompatible response.", mutation)),
            // A lost reply can follow delivery. Never retry or replace it with
            // an apparently safe pre-dispatch failure.
            Err(_) => Err(failure("computer_use_engine_transport", "The Computer use engine stopped before its result was received. Inspect the target before continuing.", mutation)),
        }
    }

    async fn inventory(&mut self) -> Result<Vec<Window>, Value> {
        let result = self.engine(vec!["list-windows".into()], false).await?;
        if result["ok"] != true {
            return Err(normalize(result, "background"));
        }
        let entries = result["data"].as_array().ok_or_else(|| {
            failure(
                "computer_use_engine_protocol",
                "The engine window inventory is invalid.",
                false,
            )
        })?;
        let mut windows = Vec::new();
        for raw in entries {
            let Some(id) = raw["id"].as_str().filter(|s| !s.is_empty()) else {
                continue;
            };
            let Some(public_id) = public_window_id(id) else {
                continue;
            };
            if raw["pid"].as_i64().map_or(true, |p| p <= 0)
                || geometry(&raw["bounds"]).is_none()
                || raw["process_instance"].as_str().map_or(true, str::is_empty)
                || (cfg!(target_os = "windows")
                    && (raw["native_owner"]["pid"]
                        .as_u64()
                        .map_or(true, |p| p == 0 || p > u32::MAX as u64)
                        || raw["native_owner"]["instance"]
                            .as_str()
                            .map_or(true, str::is_empty)))
            {
                continue;
            }
            windows.push(Window {
                public_id,
                raw: raw.clone(),
            });
        }
        self.windows = windows.iter().map(|w| (w.public_id, w.clone())).collect();
        Ok(windows)
    }

    async fn exact_window(&mut self, args: &Value) -> Result<Window, Value> {
        let pid = args["pid"]
            .as_i64()
            .filter(|n| *n > 0)
            .ok_or_else(invalid_target)?;
        let id = args["window_id"]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(invalid_target)?;
        let prior = self.windows.get(&id).cloned();
        let windows = self.inventory().await?;
        let live = windows
            .into_iter()
            .find(|w| w.pid() == pid && w.public_id == id)
            .ok_or_else(stale_target)?;
        if prior.as_ref().is_some_and(|w| !w.same_identity(&live)) {
            return Err(stale_target());
        }
        Ok(live)
    }

    async fn invoke_inner(&mut self, name: &str, args: &Value) -> Result<Value, Value> {
        match name {
            "check_permissions" => {
                let mut argv = vec!["permissions".into()];
                if args["prompt"] == true {
                    argv.push("--request".into());
                }
                let result = self.engine(argv, false).await?;
                if result["ok"] != true {
                    return Ok(normalize(result, "background"));
                }
                let mut data = result["data"].clone();
                for key in ["accessibility", "screen_recording"] {
                    data[key] = json!(result["data"][key]["state"] == "granted");
                }
                data["engine_evidence"] = result;
                Ok(success(data, vec![], "background"))
            }
            "list_windows" => {
                let windows = self.inventory().await?;
                let selected: Vec<Value> = windows
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| args["pid"].as_i64().map_or(true, |p| p == w.pid()))
                    .filter(|(_, w)| args["on_screen_only"] != true || w.raw["visible"] == true)
                    .map(|(z, w)| w.public(z))
                    .collect();
                Ok(success(json!({"windows":selected}), vec![], "background"))
            }
            "launch_app" => {
                let modality = if cfg!(target_os = "macos") {
                    "background"
                } else {
                    "visible"
                };
                let app = args["name"]
                    .as_str()
                    .filter(|s| {
                        !s.trim().is_empty() && !s.starts_with('-') && s.chars().count() <= 200
                    })
                    .ok_or_else(invalid_arguments)?;
                #[cfg(target_os = "windows")]
                let result = crate::computer_use_windows::launch_app(
                    app,
                    self.host.browser_runtime().clone(),
                )
                .await
                .map_err(|error| launch_failure(error.as_ref(), modality))?;
                #[cfg(not(target_os = "windows"))]
                let result = self
                    .engine(vec!["launch".into(), app.into()], true)
                    .await
                    .map_err(|failure| remode_failure(failure, modality))?;
                Ok(normalize(result, modality))
            }
            "join_current_stage" => {
                let window = self.exact_window(args).await?;
                #[cfg(target_os = "macos")]
                {
                    Ok(stage::join(
                        i32::try_from(window.pid()).map_err(|_| invalid_target())?,
                        u32::try_from(window.public_id).map_err(|_| invalid_target())?,
                        window.raw["process_instance"]
                            .as_str()
                            .ok_or_else(stale_target)?,
                    ))
                }
                #[cfg(target_os = "windows")]
                {
                    let mut scoped = args.clone();
                    scoped["expected_process_instance"] = window.raw["process_instance"].clone();
                    scoped["expected_native_owner"] = window.raw["native_owner"].clone();
                    let mut result = crate::computer_use_windows::focus_window_result(
                        &scoped,
                        self.host.browser_runtime(),
                    );
                    if let Some(data) = result.get_mut("structuredContent") {
                        data["effective_modality"] = json!("visible");
                    }
                    if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
                        content
                            .push(json!({"type":"text","text":"The effective mode was visible."}));
                    }
                    Ok(result)
                }
                #[cfg(not(any(target_os = "macos", target_os = "windows")))]
                {
                    Ok(normalize(
                        self.engine(
                            vec![
                                "focus-window".into(),
                                "--window-id".into(),
                                window.id().into(),
                            ],
                            true,
                        )
                        .await?,
                        "visible",
                    ))
                }
            }
            "get_window_state" => self.capture(args).await,
            "click" | "double_click" | "right_click" | "drag" | "scroll" | "type_text"
            | "press_key" | "hotkey" | "set_value" => self.action(name, args).await,
            _ => Err(failure(
                "computer_use_tool_not_allowed",
                "This Computer use operation is unavailable.",
                false,
            )),
        }
    }

    async fn capture(&mut self, args: &Value) -> Result<Value, Value> {
        let window = self.exact_window(args).await?;
        let previous = self.captures.remove(&(window.pid(), window.public_id));
        let revalidate = args["revalidate"] == true;
        if revalidate && previous.is_none() {
            return Err(stale_target());
        }
        let mode = if revalidate {
            previous
                .as_ref()
                .map(|capture| capture.mode.as_str())
                .unwrap_or("som")
        } else {
            args["capture_mode"].as_str().unwrap_or("som")
        };
        if !matches!(mode, "ax" | "vision" | "som") {
            return Err(invalid_arguments());
        }
        let mut snapshot = self
            .engine(
                vec![
                    "snapshot".into(),
                    "--window-id".into(),
                    window.id().into(),
                    "--include-bounds".into(),
                    "--max-depth".into(),
                    "50".into(),
                    "--timeout-ms".into(),
                    "15000".into(),
                ],
                false,
            )
            .await?;
        // A visual capture remains useful for a canvas or inaccessible app. Its
        // lack of semantic refs is explicit; it never invents AX elements.
        let snapshot_data = if snapshot["ok"] == true {
            snapshot["data"].clone()
        } else {
            Value::Null
        };
        let screenshot = self
            .engine(
                vec![
                    "screenshot".into(),
                    "--window-id".into(),
                    window.id().into(),
                    "--clovy-target-json".into(),
                    window.raw.to_string(),
                ],
                true,
            )
            .await?;
        if screenshot["ok"] != true {
            return Ok(normalize(screenshot, "background"));
        }
        let png = image_bytes(&screenshot["data"])?;
        let width = screenshot["data"]["width"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(invalid_image)?;
        let height = screenshot["data"]["height"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or_else(invalid_image)?;
        if width as u64 * height as u64 > 64_000_000 {
            return Err(invalid_image());
        }
        let mut elements = HashMap::new();
        let mut tree_markdown = String::new();
        flatten_tree(&snapshot_data["tree"], 0, &mut elements, &mut tree_markdown);
        for element in elements.values_mut() {
            let key = element
                .reference
                .rsplit_once(':')
                .map(|(_, part)| format!("@{part}"))
                .ok_or_else(stale_target)?;
            element.native = snapshot_data
                .pointer("/_clovy_refmap/inner")
                .and_then(|entries| entries.get(&key))
                .cloned()
                .ok_or_else(|| {
                    failure(
                        "computer_use_engine_protocol",
                        "The private snapshot omitted exact element identity.",
                        false,
                    )
                })?;
        }
        if revalidate
            && previous.as_ref().is_some_and(|capture| {
                !capture.window.same_capture(&window)
                    || capture.width != width
                    || capture.height != height
            })
        {
            return Err(stale_target());
        }
        elements = bind_public_elements(
            elements,
            previous.as_ref(),
            revalidate,
            &mut self.next_public_element,
        )?;
        tree_markdown = render_elements(&elements);
        let mut public_tree = snapshot_data["tree"].clone();
        rewrite_public_tree(&mut public_tree, &elements);
        if let Some(body) = snapshot.get_mut("data").and_then(Value::as_object_mut) {
            body.remove("_clovy_refmap");
            body.insert("tree".into(), public_tree.clone());
        }

        let observations = if mode != "ax" {
            vision::recognize(png.clone()).await
        } else {
            Ok(vec![])
        };
        let (ocr, vision_error) = match observations {
            Ok(items) => (items, None),
            Err(reason) => (vec![], Some(reason)),
        };
        // Re-read identity and geometry after both observation operations; a
        // capture spanning a resize or PID reuse cannot authorize later input.
        let live = self.exact_window(args).await?;
        if !window.same_capture(&live) {
            return Err(stale_target());
        }
        let pixels = if mode == "som" {
            draw_marks(&png, &window, &elements, width, height)?
        } else {
            png
        };
        if pixels.len() > MAX_IMAGE_BYTES {
            return Err(invalid_image());
        }
        let capture = Capture {
            window: window.clone(),
            elements,
            width,
            height,
            mode: mode.to_owned(),
            complete: snapshot_data["complete"] == true,
        };
        self.captures
            .insert((window.pid(), window.public_id), capture);
        let mut data = json!({"pid":window.pid(),"window_id":window.public_id,
            "tree_markdown":tree_markdown,"tree":public_tree,"capture_mode":mode,
            "screenshot_width":width,"screenshot_height":height,
            "scale_factor":screenshot["data"]["scale_factor"],"window_bounds":window.raw["bounds"],
            "snapshot_id":snapshot_data["snapshot_id"],"complete":snapshot_data["complete"],
            "ocr":ocr,"engine_evidence":{"snapshot":snapshot,"screenshot":{"version":screenshot["version"],"command":screenshot["command"],"ok":true,"width":width,"height":height}}});
        if let Some(reason) = vision_error {
            data["vision_error"] = json!(reason);
        }
        remove_external_guidance(&mut data);
        Ok(success(
            data,
            vec![json!({"type":"image","mimeType":"image/png","data":BASE64.encode(pixels)})],
            "background",
        ))
    }

    async fn action(&mut self, name: &str, args: &Value) -> Result<Value, Value> {
        let window = self.exact_window(args).await?;
        let capture = self
            .captures
            .get(&(window.pid(), window.public_id))
            .cloned()
            .ok_or_else(stale_target)?;
        if !capture.window.same_capture(&window) {
            return Err(stale_target());
        }
        let element = args
            .get("element_index")
            .map(|v| {
                let index = v
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(invalid_arguments)?;
                capture
                    .elements
                    .get(&index)
                    .cloned()
                    .ok_or_else(stale_target)
            })
            .transpose()?;
        if element.as_ref().is_some_and(|e| sensitive(&e.node)) {
            return Err(sensitive_field());
        }
        if matches!(name, "type_text" | "set_value") {
            validate_value_action(name, args, element.as_ref().ok_or_else(invalid_arguments)?)?;
        }
        // Window-only keys can otherwise type into a newly focused secret
        // field. Reject them while any captured sensitive field owns focus.
        if matches!(name, "press_key" | "hotkey")
            && element.is_none()
            && capture
                .elements
                .values()
                .any(|e| sensitive(&e.node) && focused(&e.node))
        {
            return Err(sensitive_field());
        }
        let physical = matches!(name, "drag" | "double_click")
            // Linux is the executable qualification surface only; its Scroll
            // adapter uses physical pointer synthesis, requiring headed policy.
            || (cfg!(target_os = "linux") && name == "scroll")
            || semantic_capability_absent(&capture, element.as_ref(), name)
            || args.get("x").is_some()
            || has_modifiers(args)
            || (matches!(name, "press_key" | "hotkey") && element.is_some())
            || (name == "scroll"
                && element.is_none()
                && !capture.elements.values().any(|e| {
                    e.node["role"]
                        .as_str()
                        .is_some_and(|s| s.contains("scroll"))
                }))
            || (cfg!(target_os = "windows")
                && matches!(name, "type_text" | "press_key" | "hotkey"));
        let mut argv = action_argv(name, args, &capture, element.as_ref(), physical)?;
        // Modality is selected before dispatch for operations with no semantic
        // equivalent. A semantic action may escalate only after the engine
        // definitively reports unsupported + not_delivered + retry safe.
        // Expire refs before dispatch, including a transport error that has
        // no reply. The local clone remains the sole action capability.
        self.captures.remove(&(window.pid(), window.public_id));
        let mut modality = action_modality(name, physical);
        let first = self
            .engine(argv.clone(), true)
            .await
            .map_err(|failure| remode_failure(failure, modality))?;
        let mut result = first.clone();
        if !physical && may_escalate(&first) && name != "set_value" {
            let live = self.exact_window(args).await?;
            if !capture.window.same_capture(&live) {
                return Err(stale_target());
            }
            argv = action_argv(name, args, &capture, element.as_ref(), true)?;
            result = self.engine(argv, true).await.map_err(|mut failure| {
                failure = remode_failure(failure, "visible");
                failure["structuredContent"]["background_attempt"] = first.clone();
                failure
            })?;
            modality = "visible";
            if let Some(body) = result.get_mut("data").and_then(Value::as_object_mut) {
                body.insert("background_attempt".into(), first);
            } else if let Some(body) = result.get_mut("error").and_then(Value::as_object_mut) {
                body.insert("background_attempt".into(), first);
            }
        }
        // Any mutation attempt expires numbered refs, including an uncertain
        // outcome. A fresh observation is required before another input.
        self.captures.remove(&(window.pid(), window.public_id));
        Ok(normalize(result, modality))
    }
}

fn action_modality(name: &str, physical: bool) -> &'static str {
    // The Linux qualification backend sends window keys only to the already
    // active exact window. Label that visible without changing policy/argv.
    if physical || (cfg!(target_os = "linux") && matches!(name, "press_key" | "hotkey")) {
        "visible"
    } else {
        "background"
    }
}

fn public_window_id(raw: &str) -> Option<u64> {
    if let Some(id) = raw
        .strip_prefix("w-")
        .and_then(|n| n.parse::<u64>().ok())
        .filter(|n| *n > 0)
    {
        return Some(id);
    }
    // Linux is an engine qualification surface only. AT-SPI ids are strings;
    // their stable hash supplies the old numeric Clovy envelope to the harness.
    #[cfg(target_os = "linux")]
    {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        raw.hash(&mut h);
        Some(h.finish().max(1))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

fn geometry(value: &Value) -> Option<[f64; 4]> {
    let g = [
        value["x"].as_f64()?,
        value["y"].as_f64()?,
        value["width"].as_f64()?,
        value["height"].as_f64()?,
    ];
    (g.iter().all(|n| n.is_finite() && n.abs() <= 10_000_000.0) && g[2] > 0.0 && g[3] > 0.0)
        .then_some(g)
}

fn image_bytes(data: &Value) -> Result<Vec<u8>, Value> {
    let raw = data["data"]
        .as_str()
        .filter(|s| s.len() <= MAX_IMAGE_BYTES.div_ceil(3) * 4)
        .ok_or_else(invalid_image)?;
    let png = BASE64.decode(raw).map_err(|_| invalid_image())?;
    if png.len() > MAX_IMAGE_BYTES || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(invalid_image());
    }
    Ok(png)
}

fn flatten_tree(
    node: &Value,
    depth: usize,
    elements: &mut HashMap<u32, Element>,
    text: &mut String,
) {
    if depth > 64 || elements.len() >= MAX_ELEMENTS {
        return;
    }
    if let Some(reference) = node["ref_id"]
        .as_str()
        .filter(|r| r.starts_with('@') && r.contains(":e"))
    {
        let index = elements.len() as u32 + 1;
        let mut metadata = node.clone();
        if let Some(o) = metadata.as_object_mut() {
            o.remove("children");
            o.remove("ref_id");
        }
        let label = serde_json::to_string(node["name"].as_str().unwrap_or("")).unwrap_or_default();
        text.push_str(&format!(
            "{}[element_index {}] {} {} metadata={}\n",
            "  ".repeat(depth.min(16)),
            index,
            node["role"].as_str().unwrap_or("unknown"),
            label,
            metadata
        ));
        elements.insert(
            index,
            Element {
                reference: reference.into(),
                node: metadata,
                native: Value::Null,
            },
        );
    }
    if let Some(children) = node["children"].as_array() {
        for child in children {
            flatten_tree(child, depth + 1, elements, text);
        }
    }
}

fn bind_public_elements(
    mut fresh: HashMap<u32, Element>,
    previous: Option<&Capture>,
    revalidate: bool,
    next: &mut u32,
) -> Result<HashMap<u32, Element>, Value> {
    let mut keys: Vec<u32> = fresh.keys().copied().collect();
    keys.sort_unstable();
    let mut result = HashMap::new();
    if revalidate {
        let previous = previous.ok_or_else(stale_target)?;
        let mut old: Vec<u32> = previous.elements.keys().copied().collect();
        old.sort_unstable();
        if old.len() != keys.len() {
            return Err(stale_target());
        }
        for (public, fresh_key) in old.into_iter().zip(keys) {
            let before = &previous.elements[&public];
            let mut live = fresh.remove(&fresh_key).ok_or_else(stale_target)?;
            if before.node != live.node || before.native != live.native {
                return Err(stale_target());
            }
            // Inspect fresh evidence, but never retarget a pending public ID
            // to a fresh native handle. Upstream resolves the original ref.
            live.reference = before.reference.clone();
            result.insert(public, live);
        }
    } else {
        for key in keys {
            let index = *next;
            *next = next.checked_add(1).ok_or_else(stale_target)?;
            result.insert(index, fresh.remove(&key).ok_or_else(stale_target)?);
        }
    }
    Ok(result)
}
fn render_elements(elements: &HashMap<u32, Element>) -> String {
    let mut sorted: Vec<_> = elements.iter().collect();
    sorted.sort_by_key(|(index, _)| **index);
    let mut text = String::new();
    for (index, element) in sorted {
        let label =
            serde_json::to_string(element.node["name"].as_str().unwrap_or("")).unwrap_or_default();
        text.push_str(&format!(
            "[element_index {}] {} {} metadata={}\n",
            index,
            element.node["role"].as_str().unwrap_or("unknown"),
            label,
            element.node
        ));
    }
    text
}
fn rewrite_public_tree(node: &mut Value, elements: &HashMap<u32, Element>) {
    let mut ids: Vec<u32> = elements.keys().copied().collect();
    ids.sort_unstable();
    fn rewrite(node: &mut Value, ids: &[u32], cursor: &mut usize) {
        if let Some(fields) = node.as_object_mut() {
            if fields.remove("ref_id").is_some() {
                if let Some(index) = ids.get(*cursor) {
                    fields.insert("element_index".into(), json!(index));
                }
                *cursor += 1;
            }
            if let Some(children) = fields.get_mut("children").and_then(Value::as_array_mut) {
                for child in children {
                    rewrite(child, ids, cursor);
                }
            }
        }
    }
    rewrite(node, &ids, &mut 0);
}

fn sensitive(node: &Value) -> bool {
    if node["states"].as_array().is_some_and(|states| {
        states
            .iter()
            .any(|state| matches!(state.as_str(), Some("secure" | "protected" | "password")))
    }) {
        return true;
    }
    let s = node.to_string().to_lowercase();
    [
        "password",
        "passcode",
        "one-time",
        "one time",
        "verification code",
        "security code",
        "auth code",
        "otp",
        "credit card",
        "card number",
        "payment",
        "billing",
        "expiration",
        "expiry",
        "cardholder",
        "bank account",
        "account number",
        "routing number",
        "iban",
        "cvv",
        "cvc",
        "social security",
        "tax identification",
        "date of birth",
        "securetextfield",
        "passwordfield",
        "secret",
        "private key",
        "api key",
        "access token",
        "seed phrase",
        "recovery phrase",
        "cc-number",
        "cc-exp",
        "cc-csc",
        "routing number",
        "sort code",
        "swift code",
        "mm/yy",
        "terminal",
        "shell prompt",
    ]
    .iter()
    .any(|v| s.contains(v))
}
// Select visible input before dispatch only from an explicit, complete
// native capability list. Missing/partial metadata never means unsupported.
fn semantic_capability_absent(capture: &Capture, element: Option<&Element>, name: &str) -> bool {
    let capability = match name {
        "right_click" => "RightClick",
        "type_text" => "TypeText",
        _ => return false,
    };
    let Some(element) = element else {
        return false;
    };
    let Some(actions) = element.node["available_actions"].as_array() else {
        return false;
    };
    capture.complete
        && element.node["subtree_truncated"] != true
        && !actions.is_empty()
        && actions.iter().all(Value::is_string)
        && element.node["available_actions"] == element.native["available_actions"]
        && !actions.iter().any(|action| action == capability)
}

fn validate_value_action(name: &str, args: &Value, element: &Element) -> Result<(), Value> {
    if name == "type_text" && !editable(&element.node) {
        return Err(failure(
            "computer_use_text_target_invalid",
            "Computer use can enter text only into a captured editable field.",
            false,
        ));
    }
    let field = if name == "type_text" { "text" } else { "value" };
    if args[field]
        .as_str()
        .map_or(true, |text| text.chars().count() > 10_000)
    {
        return Err(invalid_arguments());
    }
    Ok(())
}

fn editable(node: &Value) -> bool {
    let role = node["role"].as_str().unwrap_or("").to_ascii_lowercase();
    [
        "textfield",
        "textarea",
        "searchfield",
        "combobox",
        "editabletext",
    ]
    .iter()
    .any(|s| role.contains(s))
}
fn focused(node: &Value) -> bool {
    node["states"]
        .as_array()
        .is_some_and(|a| a.iter().any(|s| s == "focused"))
}

fn action_argv(
    name: &str,
    args: &Value,
    capture: &Capture,
    element: Option<&Element>,
    physical: bool,
) -> Result<Vec<String>, Value> {
    let reference = || {
        element
            .map(|e| e.reference.clone())
            .ok_or_else(invalid_arguments)
    };
    let mut argv = Vec::new();
    if physical {
        argv.push("--headed".into());
    }
    let mut command = match name {
        "click" => vec!["click".into(), reference().unwrap_or_default()],
        "double_click" => vec!["double-click".into(), reference().unwrap_or_default()],
        "right_click" => vec!["right-click".into(), reference().unwrap_or_default()],
        "type_text" => vec![
            "type".into(),
            "--timeout-ms".into(),
            "10000".into(),
            reference()?,
            "--".into(),
            args["text"].as_str().ok_or_else(invalid_arguments)?.into(),
        ],
        "set_value" => vec![
            "set-value".into(),
            "--timeout-ms".into(),
            "5000".into(),
            reference()?,
            "--".into(),
            args["value"].as_str().ok_or_else(invalid_arguments)?.into(),
        ],
        "scroll" => {
            let target = element.or_else(|| {
                capture.elements.values().find(|e| {
                    e.node["role"]
                        .as_str()
                        .is_some_and(|s| s.contains("scroll"))
                })
            });
            if let Some(target) = target {
                vec![
                    "scroll".into(),
                    target.reference.clone(),
                    "--direction".into(),
                    direction(args)?.into(),
                    "--amount".into(),
                    amount(args)?.to_string(),
                ]
            } else if !physical {
                return Err(failure("computer_use_background_unsupported","This window has no semantic scroll target. Capture and select a scroll element or use visible input.",false));
            } else {
                wheel_args(args, capture)?
            }
        }
        "press_key" | "hotkey" => {
            let mut command = vec![
                "press".into(),
                key_combo(name, args)?,
                "--window-id".into(),
                capture.window.id().into(),
            ];
            if let Some(element) = element {
                command.extend(["--clovy-ref".into(), element.reference.clone()]);
            } else {
                command.extend(["--clovy-target-json".into(), capture.window.raw.to_string()]);
            }
            command
        }
        "drag" => {
            let from = local_point(args, "from_x", "from_y", capture)?;
            let to = local_point(args, "to_x", "to_y", capture)?;
            let mut command = vec![
                "drag".into(),
                format!("--from-xy={},{}", from.0, from.1),
                format!("--to-xy={},{}", to.0, to.1),
                "--timeout-ms".into(),
                "5000".into(),
                "--clovy-target-json".into(),
                capture.window.raw.to_string(),
            ];
            let mut parsed = Vec::new();
            modifiers(args, &mut parsed)?;
            for pair in parsed.chunks_exact(2) {
                command.extend(["--clovy-modifier".into(), pair[1].clone()]);
            }
            command
        }
        _ => return Err(invalid_arguments()),
    };
    if matches!(name, "click" | "double_click" | "right_click") {
        if args.get("x").is_some() || has_modifiers(args) {
            if !physical {
                return Err(invalid_arguments());
            }
            let point = if args.get("x").is_some() {
                local_point(args, "x", "y", capture)?
            } else {
                element_point(element.ok_or_else(invalid_arguments)?, capture)?
            };
            if capture
                .elements
                .values()
                .any(|e| sensitive(&e.node) && node_contains(&e.node, point))
            {
                return Err(sensitive_field());
            }
            let count = if name == "double_click" {
                2
            } else {
                args["count"].as_u64().unwrap_or(1)
            };
            if !(1..=3).contains(&count) {
                return Err(invalid_arguments());
            }
            command = vec![
                "mouse-click".into(),
                format!("--xy={},{}", point.0, point.1),
                "--button".into(),
                if name == "right_click" {
                    "right"
                } else {
                    "left"
                }
                .into(),
                "--count".into(),
                count.to_string(),
                "--clovy-target-json".into(),
                capture.window.raw.to_string(),
            ];
            modifiers(args, &mut command)?;
        } else {
            reference()?;
        }
    }
    if name == "scroll" && physical && command[0] == "scroll" { /* qualified refs keep their exact window */
    }
    argv.extend(command);
    Ok(argv)
}
fn has_modifiers(args: &Value) -> bool {
    args.get("modifier")
        .is_some_and(|value| value.as_array().map_or(true, |items| !items.is_empty()))
}
fn modifiers(args: &Value, command: &mut Vec<String>) -> Result<(), Value> {
    let Some(value) = args.get("modifier") else {
        return Ok(());
    };
    let values: Vec<&str> = if let Some(s) = value.as_str() {
        vec![s]
    } else {
        value
            .as_array()
            .ok_or_else(invalid_arguments)?
            .iter()
            .map(|s| s.as_str().ok_or_else(invalid_arguments))
            .collect::<Result<_, _>>()?
    };
    for value in values {
        if !matches!(
            value,
            "shift" | "meta" | "cmd" | "ctrl" | "control" | "alt" | "option"
        ) {
            return Err(invalid_arguments());
        }
        command.extend(["--modifiers".into(), value.into()]);
    }
    Ok(())
}
fn direction(args: &Value) -> Result<&str, Value> {
    let d = args["direction"].as_str().unwrap_or("down");
    if matches!(d, "up" | "down" | "left" | "right") {
        Ok(d)
    } else {
        Err(invalid_arguments())
    }
}
fn amount(args: &Value) -> Result<u64, Value> {
    let n = args["amount"].as_u64().unwrap_or(3);
    (1..=50)
        .contains(&n)
        .then_some(n)
        .ok_or_else(invalid_arguments)
}
fn wheel_args(args: &Value, capture: &Capture) -> Result<Vec<String>, Value> {
    let g = capture.window.geometry().ok_or_else(stale_target)?;
    let n = amount(args)? as f64;
    let (dx, dy) = match direction(args)? {
        "up" => (0.0, n),
        "down" => (0.0, -n),
        "left" => (-n, 0.0),
        _ => (n, 0.0),
    };
    Ok(vec![
        "mouse-wheel".into(),
        format!("--x={}", g[0] + g[2] / 2.0),
        format!("--y={}", g[1] + g[3] / 2.0),
        format!("--dx={dx}"),
        format!("--dy={dy}"),
        "--clovy-target-json".into(),
        capture.window.raw.to_string(),
    ])
}
fn local_point(args: &Value, x: &str, y: &str, capture: &Capture) -> Result<(f64, f64), Value> {
    let x = args[x].as_f64().ok_or_else(invalid_arguments)?;
    let y = args[y].as_f64().ok_or_else(invalid_arguments)?;
    if !x.is_finite()
        || !y.is_finite()
        || x < 0.0
        || y < 0.0
        || x >= capture.width as f64
        || y >= capture.height as f64
    {
        return Err(invalid_arguments());
    }
    let g = capture.window.geometry().ok_or_else(stale_target)?;
    Ok((
        g[0] + x * g[2] / capture.width as f64,
        g[1] + y * g[3] / capture.height as f64,
    ))
}
fn element_point(element: &Element, capture: &Capture) -> Result<(f64, f64), Value> {
    let e = geometry(&element.node["bounds"]).ok_or_else(stale_target)?;
    let g = capture.window.geometry().ok_or_else(stale_target)?;
    let point = (e[0] + e[2] / 2.0, e[1] + e[3] / 2.0);
    if point.0 < g[0] || point.1 < g[1] || point.0 >= g[0] + g[2] || point.1 >= g[1] + g[3] {
        return Err(stale_target());
    }
    Ok(point)
}
fn node_contains(node: &Value, p: (f64, f64)) -> bool {
    geometry(&node["bounds"])
        .is_some_and(|g| p.0 >= g[0] && p.1 >= g[1] && p.0 < g[0] + g[2] && p.1 < g[1] + g[3])
}
fn key_combo(name: &str, args: &Value) -> Result<String, Value> {
    let mut values = Vec::new();
    if name == "hotkey" {
        for value in args["keys"].as_array().ok_or_else(invalid_arguments)? {
            values.push(value.as_str().ok_or_else(invalid_arguments)?.to_string());
        }
    } else {
        if let Some(modifiers) = args["modifiers"].as_array() {
            for m in modifiers {
                values.push(m.as_str().ok_or_else(invalid_arguments)?.into());
            }
        }
        values.push(args["key"].as_str().ok_or_else(invalid_arguments)?.into());
    }
    if values.is_empty()
        || values.len() > 5
        || values
            .iter()
            .any(|s| s.is_empty() || s.len() > 32 || s.starts_with('-') || s.contains(['\n', '\r']))
    {
        return Err(invalid_arguments());
    }
    let combo = values.join("+");
    let lower = combo.to_ascii_lowercase();
    if lower.contains("+v")
        || lower.contains("+c")
        || lower.contains("+x")
        || lower.contains("+q")
        || lower.contains("+w")
        || lower.contains("+delete")
        || lower.contains("+backspace")
    {
        return Err(failure(
            "computer_use_key_blocked",
            "Computer use blocked a clipboard, destructive, or system shortcut.",
            false,
        ));
    }
    Ok(combo)
}
fn may_escalate(value: &Value) -> bool {
    value["ok"] == false
        && value["error"]["code"] == "ACTION_NOT_SUPPORTED"
        && value["error"]["disposition"]["delivery"] == "not_delivered"
        && value["error"]["disposition"]["retry"] == "safe"
}

// Keep native evidence and dispositions, while discarding upstream's CLI
// tutorials. Those commands are outside the private Clovy allowlist.
fn remove_external_guidance(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            fields.remove("suggestion");
            fields.remove("recovery");
            for child in fields.values_mut() {
                remove_external_guidance(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                remove_external_guidance(child);
            }
        }
        _ => {}
    }
}

#[cfg(any(target_os = "windows", test))]
fn launch_failure(error: &(dyn std::error::Error + 'static), modality: &str) -> Value {
    let (code, message, uncertain) = if error
        .is::<crate::computer_use_policy::PrivateBrowserLaunchDenied>()
    {
        (
            "computer_use_private_browser_blocked",
            "Use Browser use or human takeover for the private browser.",
            false,
        )
    } else {
        ("computer_use_launch_unverified", "The application launch could not be verified. Inspect the window list before continuing.", true)
    };
    remode_failure(failure(code, message, uncertain), modality)
}

fn remode_failure(failure: Value, modality: &str) -> Value {
    normalize(
        failure["structuredContent"]["engine_evidence"].clone(),
        modality,
    )
}
fn normalize(mut response: Value, modality: &str) -> Value {
    remove_external_guidance(&mut response);
    let ok = response["ok"] == true;
    let mut data = if ok {
        response["data"].clone()
    } else {
        json!({"error":response["error"]})
    };
    if !data.is_object() {
        data = json!({"result":data});
    }
    data["engine_evidence"] = response;
    if !ok {
        data["code"] = data["error"]["code"].clone();
        data["disposition"] = data["error"]["disposition"].clone();
    }
    let text = if ok {
        format!("Computer use operation completed in {modality} mode.")
    } else {
        let cause = data["error"]["message"]
            .as_str()
            .unwrap_or("Computer use operation failed.");
        if data["error"]["code"] == "POLICY_DENIED"
            || data["error"]["code"] == "computer_use_sensitive_field_blocked"
        {
            format!("{cause} Take over to continue with this target. The effective mode was {modality}.")
        } else {
            format!("{cause} The effective mode was {modality}.")
        }
    };
    data["effective_modality"] = json!(modality);
    json!({"content":[{"type":"text","text":text}],"structuredContent":data,"isError":!ok})
}
fn success(mut data: Value, mut content: Vec<Value>, modality: &str) -> Value {
    data["effective_modality"] = json!(modality);
    content.insert(
        0,
        json!({"type":"text","text":format!("Computer use observation completed in {modality} mode.")}),
    );
    json!({"content":content,"structuredContent":data,"isError":false})
}
fn failure(code: &str, message: &str, uncertain: bool) -> Value {
    let disposition = if uncertain {
        json!({"delivery":"delivery_uncertain","retry":"unsafe"})
    } else {
        json!({"delivery":"not_delivered","retry":"safe"})
    };
    normalize(
        json!({"version":"2.5","ok":false,"error":{"code":code,"message":message,"disposition":disposition}}),
        "background",
    )
}
fn invalid_arguments() -> Value {
    failure(
        "computer_use_arguments_invalid",
        "The Computer use arguments are invalid.",
        false,
    )
}
fn invalid_target() -> Value {
    failure(
        "computer_use_target_required",
        "An exact process and window are required.",
        false,
    )
}
fn stale_target() -> Value {
    failure(
        "computer_use_target_stale",
        "The target window or captured elements changed. Capture the target again.",
        false,
    )
}
fn invalid_image() -> Value {
    failure(
        "computer_use_capture_invalid",
        "The Computer use screenshot is invalid or exceeds its size limit.",
        false,
    )
}
fn sensitive_field() -> Value {
    failure("computer_use_sensitive_field_blocked","Computer use cannot enter credentials, one-time codes, payment data, or secrets. Take over to complete this field.",false)
}

fn draw_marks(
    png: &[u8],
    window: &Window,
    elements: &HashMap<u32, Element>,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Value> {
    use image::{ImageEncoder, Rgba};
    let mut pixels = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|_| invalid_image())?
        .to_rgba8();
    if pixels.width() != width || pixels.height() != height {
        return Err(invalid_image());
    }
    let g = window.geometry().ok_or_else(stale_target)?;
    let mut ordered: Vec<_> = elements.iter().collect();
    ordered.sort_by_key(|(index, _)| **index);
    for (index, element) in ordered {
        let Some(b) = geometry(&element.node["bounds"]) else {
            continue;
        };
        let x = ((b[0] - g[0]) * width as f64 / g[2]).round().max(0.0) as u32;
        let y = ((b[1] - g[1]) * height as f64 / g[3]).round().max(0.0) as u32;
        if x >= width || y >= height {
            continue;
        }
        let label = index.to_string();
        let label_width = (label.len() as u32 * 8 + 4).min(width - x);
        for py in y..(y + 13).min(height) {
            for px in x..x + label_width {
                pixels.put_pixel(px, py, Rgba([27, 85, 220, 255]));
            }
        }
        for (i, digit) in label.bytes().enumerate() {
            let rows = DIGITS[(digit - b'0') as usize];
            for (ry, row) in rows.iter().enumerate() {
                for rx in 0..5 {
                    if row & (1 << (4 - rx)) != 0 {
                        let px = x + 3 + i as u32 * 8 + rx;
                        let py = y + 3 + ry as u32;
                        if px < width && py < height {
                            pixels.put_pixel(px, py, Rgba([255, 255, 255, 255]));
                        }
                    }
                }
            }
        }
    }
    let mut output = Vec::new();
    image::codecs::png::PngEncoder::new(&mut output)
        .write_image(
            pixels.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|_| invalid_image())?;
    Ok(output)
}
const DIGITS: [[u8; 7]; 10] = [
    [14, 17, 19, 21, 25, 17, 14],
    [4, 12, 4, 4, 4, 4, 14],
    [14, 17, 1, 2, 4, 8, 31],
    [30, 1, 1, 14, 1, 1, 30],
    [2, 6, 10, 18, 31, 2, 2],
    [31, 16, 16, 30, 1, 1, 30],
    [14, 16, 16, 30, 17, 17, 14],
    [31, 1, 2, 4, 8, 8, 8],
    [14, 17, 17, 14, 17, 17, 14],
    [14, 17, 17, 15, 1, 1, 14],
];

#[cfg(test)]
#[path = "computer_use_engine/tests.rs"]
mod tests;
