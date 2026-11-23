//! CDP transport restricted to inherited anonymous pipes. No discovery or listeners.
use crate::native::cdp::types::{CdpEvent, CdpMessage};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Read, Write},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    sync::{Arc, Mutex as StdMutex},
};
use tokio::sync::{broadcast, mpsc, oneshot};
const LIMIT: usize = 16 * 1024 * 1024;

fn nonblocking(file: &File, for_write: bool) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = for_write;
        use std::os::fd::AsRawFd;
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    #[cfg(windows)]
    {
        // CreatePipe's read handle lacks FILE_WRITE_ATTRIBUTES, so it cannot
        // change mode. The sole reader uses PeekNamedPipe before ReadFile.
        if !for_write {
            return Ok(());
        }
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Pipes::{
            SetNamedPipeHandleState, PIPE_NOWAIT, PIPE_READMODE_BYTE,
        };
        let mode = PIPE_NOWAIT | PIPE_READMODE_BYTE;
        if unsafe {
            SetNamedPipeHandleState(
                file.as_raw_handle() as _,
                &mode,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

fn retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
    ) || cfg!(windows) && error.raw_os_error() == Some(232)
}

fn read_frame(reader: &mut File, buffered: &mut Vec<u8>, closed: &AtomicBool) -> Option<Vec<u8>> {
    let mut scanned = 0;
    loop {
        if closed.load(Ordering::SeqCst) {
            return None;
        }
        if let Some(relative) = buffered[scanned..].iter().position(|byte| *byte == 0) {
            let end = scanned + relative;
            if end > LIMIT {
                return None;
            }
            let mut frame = buffered.drain(..=end).collect::<Vec<_>>();
            frame.pop();
            return Some(frame);
        }
        scanned = buffered.len();
        if buffered.len() > LIMIT {
            return None;
        }
        let mut chunk = [0u8; 8192];
        #[cfg(windows)]
        let read_limit = {
            use std::os::windows::io::AsRawHandle;
            let mut available = 0u32;
            let ok = unsafe {
                windows_sys::Win32::System::Pipes::PeekNamedPipe(
                    reader.as_raw_handle() as _,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null_mut(),
                    &mut available,
                    std::ptr::null_mut(),
                )
            };
            if ok == 0 {
                return None;
            }
            if available == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            (available as usize).min(chunk.len())
        };
        #[cfg(not(windows))]
        let read_limit = chunk.len();
        match reader.read(&mut chunk[..read_limit]) {
            Ok(0) => return None,
            Ok(n) => buffered.extend_from_slice(&chunk[..n]),
            Err(error) if retryable(&error) => {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            Err(_) => return None,
        }
    }
}
fn valid_observation_breakpoint(id: &str, marker: &str) -> bool {
    let Some(nonce) = marker.strip_prefix("clovy-private-observation:") else {
        return false;
    };
    if nonce.len() != 32
        || !nonce
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return false;
    }
    id == format!("br:1:0:0:{marker}")
}
fn observation_event(method: &str, params: &Value, owned: &HashSet<String>) -> Value {
    if method == "Debugger.resumed" {
        return Value::Null;
    }
    let Some(hits) = params["hitBreakpoints"].as_array() else {
        return json!({"ownHits":[],"foreignPaused":true});
    };
    let mut own_hits = Vec::new();
    let mut foreign = hits.is_empty() || hits.len() > 16;
    for hit in hits.iter().take(16) {
        match hit.as_str() {
            Some(id) if owned.contains(id) => own_hits.push(id),
            _ => foreign = true,
        }
    }
    json!({"ownHits":own_hits,"foreignPaused":foreign})
}

type Pending = Arc<StdMutex<HashMap<u64, oneshot::Sender<CdpMessage>>>>;
struct PendingCommand {
    id: u64,
    pending: Pending,
    cancelled: Arc<AtomicBool>,
}
impl Drop for PendingCommand {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.pending.lock().unwrap().remove(&self.id);
    }
}
type Taps = Arc<StdMutex<Vec<(&'static [&'static str], mpsc::UnboundedSender<CdpEvent>)>>>;
#[derive(Debug, Clone)]
pub struct RawCdpMessage {
    pub text: String,
    pub session_id: Option<String>,
}
pub struct CdpClient {
    writer: Arc<StdMutex<Option<File>>>,
    observation_breakpoints: Arc<StdMutex<HashSet<String>>>,
    redacted_nodes: StdMutex<HashSet<i64>>,
    redacted_values: StdMutex<Vec<String>>,
    pending: Pending,
    events: broadcast::Sender<CdpEvent>,
    closed: Arc<AtomicBool>,
    next_id: AtomicU64,
    taps: Taps,
    sessions: Arc<StdMutex<HashMap<String, mpsc::Sender<CdpEvent>>>>,
}
impl Drop for CdpClient {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut writer) = self.writer.try_lock() {
            writer.take();
        }
    }
}
impl CdpClient {
    pub fn register_observation_breakpoint(&self, id: &str, marker: &str) -> Result<(), String> {
        if !valid_observation_breakpoint(id, marker) {
            return Err("redaction_failed".into());
        }
        let mut owned = self.observation_breakpoints.lock().unwrap();
        if owned.len() >= 4 {
            return Err("redaction_failed".into());
        }
        owned.insert(id.to_owned());
        Ok(())
    }
    pub fn unregister_observation_breakpoint(&self, id: &str) {
        self.observation_breakpoints.lock().unwrap().remove(id);
    }
    pub fn set_redacted_nodes(&self, nodes: HashSet<i64>, values: Vec<String>) {
        *self.redacted_values.lock().unwrap() = values;
        *self.redacted_nodes.lock().unwrap() = nodes;
    }
    pub fn from_pipe(read: File, write: File) -> Result<Self, String> {
        nonblocking(&read, false).map_err(|_| "Cannot configure private CDP pipe")?;
        nonblocking(&write, true).map_err(|_| "Cannot configure private CDP pipe")?;
        let observation_breakpoints = Arc::new(StdMutex::new(HashSet::new()));
        let reader_breakpoints = observation_breakpoints.clone();
        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(128);
        let closed = Arc::new(AtomicBool::new(false));
        let taps: Taps = Arc::new(StdMutex::new(Vec::new()));
        let sessions: Arc<StdMutex<HashMap<String, mpsc::Sender<CdpEvent>>>> =
            Arc::new(StdMutex::new(HashMap::new()));
        let (p, e, c, t, s) = (
            pending.clone(),
            events.clone(),
            closed.clone(),
            taps.clone(),
            sessions.clone(),
        );
        std::thread::spawn(move || {
            let mut reader = read;
            let mut buffered = Vec::new();
            loop {
                let Some(bytes) = read_frame(&mut reader, &mut buffered, &c) else {
                    break;
                };
                let Ok(msg) = serde_json::from_slice::<CdpMessage>(&bytes) else {
                    break;
                };
                if let Some(id) = msg.id {
                    if let Some(tx) = p.lock().unwrap().remove(&id) {
                        let _ = tx.send(msg);
                    }
                } else if let Some(method) = msg.method {
                    // No console, response-body, credential, plugin or streaming event buffer.
                    if !matches!(
                        method.as_str(),
                        "Page.domContentEventFired"
                            | "Page.loadEventFired"
                            | "Page.lifecycleEvent"
                            | "Debugger.paused"
                            | "Debugger.resumed"
                    ) {
                        continue;
                    }
                    let ev = CdpEvent {
                        method: method.clone(),
                        params: if method.starts_with("Debugger.") {
                            observation_event(
                                &method,
                                &msg.params.unwrap_or(Value::Null),
                                &reader_breakpoints.lock().unwrap(),
                            )
                        } else {
                            msg.params.unwrap_or(Value::Null)
                        },
                        session_id: msg.session_id,
                    };
                    if let Some(tx) = ev
                        .session_id
                        .as_ref()
                        .and_then(|id| s.lock().unwrap().get(id).cloned())
                    {
                        let _ = tx.try_send(ev.clone());
                    }
                    t.lock().unwrap().retain(|(methods, tx)| {
                        !methods.contains(&ev.method.as_str()) || tx.send(ev.clone()).is_ok()
                    });
                    let _ = e.send(ev);
                }
            }
            c.store(true, Ordering::SeqCst);
            p.lock().unwrap().clear();
        });
        Ok(Self {
            writer: Arc::new(StdMutex::new(Some(write))),
            observation_breakpoints,
            redacted_nodes: StdMutex::new(HashSet::new()),
            redacted_values: StdMutex::new(Vec::new()),
            pending,
            events,
            closed,
            next_id: AtomicU64::new(1),
            taps,
            sessions,
        })
    }
    pub async fn connect(_: &str) -> Result<Self, String> {
        Err("Private host forbids CDP endpoints".into())
    }
    pub async fn connect_with_headers(
        _: &str,
        _: Option<Vec<(String, String)>>,
    ) -> Result<Self, String> {
        Self::connect("").await
    }
    pub async fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut writer) = self.writer.try_lock() {
            writer.take();
        }
        self.pending.lock().unwrap().clear();
    }
    pub async fn send_command(
        &self,
        method: &str,
        params: Option<Value>,
        sid: Option<&str>,
    ) -> Result<Value, String> {
        if self.closed.load(Ordering::SeqCst) {
            return Err("CDP pipe closed".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut v = json!({"id":id,"method":method});
        if let Some(p) = params {
            v["params"] = p;
        }
        if let Some(s) = sid.filter(|s| !s.is_empty()) {
            v["sessionId"] = json!(s);
        }
        let mut bytes = serde_json::to_vec(&v).map_err(|_| "CDP encoding")?;
        if bytes.len() > LIMIT {
            return Err("CDP request too large".into());
        }
        bytes.push(0);
        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if self.closed.load(Ordering::SeqCst) {
                return Err("CDP pipe closed".into());
            }
            pending.insert(id, tx);
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let _pending = PendingCommand {
            id,
            pending: self.pending.clone(),
            cancelled: cancelled.clone(),
        };
        let writer = self.writer.clone();
        let closed = self.closed.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let write = tokio::task::spawn_blocking(move || {
            let mut w = writer.lock().unwrap();
            let mut offset = 0;
            while offset < bytes.len() {
                if closed.load(Ordering::SeqCst)
                    || cancelled.load(Ordering::SeqCst)
                    || std::time::Instant::now() >= deadline
                {
                    if offset > 0
                        || closed.load(Ordering::SeqCst)
                        || std::time::Instant::now() >= deadline
                    {
                        closed.store(true, Ordering::SeqCst);
                        w.take();
                    }
                    return Err("CDP pipe closed");
                }
                let pipe = w.as_mut().ok_or("CDP pipe closed")?;
                match pipe.write(&bytes[offset..]) {
                    Ok(0) if cfg!(windows) => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Ok(0) => {
                        closed.store(true, Ordering::SeqCst);
                        w.take();
                        return Err("CDP pipe write failed");
                    }
                    Ok(n) => offset += n,
                    Err(error) if retryable(&error) => {
                        std::thread::sleep(std::time::Duration::from_millis(5))
                    }
                    Err(_) => {
                        closed.store(true, Ordering::SeqCst);
                        w.take();
                        return Err("CDP pipe write failed");
                    }
                }
            }
            Ok(())
        });
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            write.await.map_err(|_| "CDP writer lost")??;
            let msg = rx.await.map_err(|_| "CDP pipe closed")?;
            if msg.error.is_some() {
                Err("CDP command rejected")
            } else {
                redact_response(
                    method,
                    msg.result.unwrap_or(Value::Null),
                    &self.redacted_nodes.lock().unwrap(),
                    &self.redacted_values.lock().unwrap(),
                )
            }
        })
        .await;
        outcome
            .map_err(|_| "CDP command timed out".to_string())?
            .map_err(String::from)
    }
    pub async fn send_command_typed<P: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        m: &str,
        p: &P,
        s: Option<&str>,
    ) -> Result<R, String> {
        let v = self
            .send_command(
                m,
                Some(serde_json::to_value(p).map_err(|_| "CDP encoding")?),
                s,
            )
            .await?;
        serde_json::from_value(v).map_err(|_| "Invalid CDP response".into())
    }
    pub async fn send_command_no_params(&self, m: &str, s: Option<&str>) -> Result<Value, String> {
        self.send_command(m, None, s).await
    }
    pub async fn send_command_no_wait(
        &self,
        m: &str,
        p: Option<Value>,
        s: Option<&str>,
    ) -> Result<(), String> {
        self.send_command(m, p, s).await.map(|_| ())
    }
    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }
    /// Count the actual in-flight command map for upstream cancellation tests.
    #[cfg(test)]
    pub(crate) async fn pending_len(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
    pub fn subscribe_methods(
        &self,
        methods: &'static [&'static str],
    ) -> mpsc::UnboundedReceiver<CdpEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.taps.lock().unwrap().push((methods, tx));
        rx
    }
    pub fn subscribe_session(&self, id: &str) -> mpsc::Receiver<CdpEvent> {
        let (tx, rx) = mpsc::channel(16);
        self.sessions.lock().unwrap().insert(id.into(), tx);
        rx
    }
    pub fn unsubscribe_session(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
    }
    pub fn subscribe_raw(&self) -> broadcast::Receiver<RawCdpMessage> {
        let (_, rx) = broadcast::channel(1);
        rx
    }
    pub fn inspect_handle(&self) -> InspectProxyHandle {
        InspectProxyHandle
    }
    pub async fn send_raw(&self, _: String) -> Result<(), String> {
        Err("Private host forbids raw CDP".into())
    }
}
pub struct InspectProxyHandle;
impl InspectProxyHandle {
    pub async fn send_raw(&self, _: String) -> Result<(), String> {
        Err("Private host forbids inspect proxy".into())
    }
    pub fn subscribe_raw(&self) -> broadcast::Receiver<RawCdpMessage> {
        let (_, rx) = broadcast::channel(1);
        rx
    }
}

// Sanitize one complete AX response before any upstream snapshot caller sees it.
// Raw values and editable descendants come from this response, never only a prior DOM inventory.
fn redact_response(
    method: &str,
    mut v: Value,
    redacted: &HashSet<i64>,
    inventory: &[String],
) -> Result<Value, &'static str> {
    if !method.starts_with("Accessibility.")
        || matches!(method, "Accessibility.enable" | "Accessibility.disable")
    {
        return Ok(v);
    }
    let nodes = v
        .get_mut("nodes")
        .and_then(Value::as_array_mut)
        .ok_or("redaction_failed")?;
    let mut values = inventory.to_vec();
    let mut private_children = HashSet::<String>::new();
    for node in nodes.iter() {
        if let Some(value) = node["value"]["value"].as_str().filter(|v| !v.is_empty()) {
            values.push(value.to_owned());
            let trimmed = value.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
            if !trimmed.is_empty() && trimmed != value {
                values.push(trimmed.to_owned());
            }
            let normalized = value.split_whitespace().collect::<Vec<_>>().join(" ");
            if !normalized.is_empty() && normalized != value {
                values.push(normalized);
            }
        }
        let editable = node["properties"].as_array().is_some_and(|props| {
            props.iter().any(|p| {
                p["name"] == "editable"
                    && (p["value"]["value"].as_bool() == Some(true)
                        || matches!(p["value"]["value"].as_str(), Some("plaintext" | "richtext")))
            })
        });
        let field = matches!(
            node["role"]["value"].as_str(),
            Some("textbox" | "searchbox" | "combobox" | "spinbutton")
        );
        if editable || field {
            for child in node["childIds"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                private_children.insert(child.to_owned());
            }
        }
    }
    values.retain(|value| !value.is_empty());
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    values.dedup();
    if values.len() > 4000 || values.iter().map(String::len).sum::<usize>() > 8 * 1024 * 1024 {
        return Err("redaction_failed");
    }
    let mut descendants: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut edges = 0usize;
    for node in nodes.iter() {
        let Some(id) = node["nodeId"].as_str() else {
            continue;
        };
        for child in node["childIds"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            edges = edges
                .checked_add(1)
                .filter(|n| *n <= LIMIT)
                .ok_or("redaction_failed")?;
            descendants.entry(id).or_default().push(child);
        }
    }
    let mut pending: Vec<String> = private_children.iter().cloned().collect();
    while let Some(id) = pending.pop() {
        if let Some(children) = descendants.get(id.as_str()) {
            for child in children {
                if private_children.insert((*child).to_owned()) {
                    pending.push((*child).to_owned());
                }
            }
        }
    }
    drop(descendants);
    let mut buckets: [Vec<&str>; 256] = std::array::from_fn(|_| Vec::new());
    for value in &values {
        buckets[value.as_bytes()[0] as usize].push(value);
    }
    let mut budget = LIMIT;
    let mut output_budget = LIMIT;
    for node in nodes {
        let private = node["backendDOMNodeId"]
            .as_i64()
            .is_some_and(|id| redacted.contains(&id))
            || node["nodeId"]
                .as_str()
                .is_some_and(|id| private_children.contains(id));
        for key in ["name", "description"] {
            if let Some(text) = node[key]["value"].as_str() {
                let safe = if private {
                    output_budget = output_budget.checked_sub(10).ok_or("redaction_failed")?;
                    "[redacted]".to_owned()
                } else {
                    redact_original(text, &buckets, &mut budget, &mut output_budget)?
                };
                // Drop AX computed-name sources too: they can retain raw field content.
                node[key] = json!({"type":"computedString", "value":safe});
            } else if let Some(object) = node.as_object_mut() {
                object.remove(key);
            }
        }
        if let Some(object) = node.as_object_mut() {
            object.remove("value");
        }
    }
    Ok(v)
}

fn redact_original(
    text: &str,
    buckets: &[Vec<&str>; 256],
    budget: &mut usize,
    output_budget: &mut usize,
) -> Result<String, &'static str> {
    let mut safe = String::new();
    let mut covered_until = 0;
    let mut masking = false;
    for (offset, character) in text.char_indices() {
        let remaining = &text[offset..];
        *budget = budget
            .checked_sub(character.len_utf8())
            .ok_or("redaction_failed")?;
        for value in &buckets[remaining.as_bytes()[0] as usize] {
            *budget = budget.checked_sub(value.len()).ok_or("redaction_failed")?;
            if remaining.starts_with(value) {
                if !masking {
                    *output_budget = output_budget.checked_sub(10).ok_or("redaction_failed")?;
                    safe.push_str("[redacted]");
                    masking = true;
                }
                covered_until = covered_until.max(offset + value.len());
                break;
            }
        }
        if offset >= covered_until {
            masking = false;
            *output_budget = output_budget
                .checked_sub(character.len_utf8())
                .ok_or("redaction_failed")?;
            safe.push(character);
        }
    }
    Ok(safe)
}

#[cfg(test)]
mod private_redaction_tests {
    use super::*;
    #[cfg(unix)]
    fn wait_for_pipe_data(file: &File) {
        use std::os::fd::AsRawFd;
        let mut pollfd = libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert_eq!(
            unsafe { libc::poll(&mut pollfd, 1, 2000) },
            1,
            "writer did not reach the pipe"
        );
        assert_ne!(pollfd.revents & libc::POLLIN, 0);
    }
    #[test]
    fn overlapping_private_values_mask_their_entire_union() {
        for (original, values, expected) in [
            (
                "public 1234567 end",
                vec!["12", "234567"],
                "public [redacted] end",
            ),
            ("前猫犬鳥後", vec!["猫犬", "犬鳥"], "前[redacted]後"),
            ("前😀🐱犬後", vec!["😀🐱", "🐱犬"], "前[redacted]後"),
            ("xabcdefz", vec!["abc", "cde", "ef"], "x[redacted]z"),
            ("xabcdy", vec!["ab", "cd"], "x[redacted]y"),
            ("xabcdy", vec!["a", "abc"], "x[redacted]dy"),
        ] {
            let raw = json!({"nodes":[{"nodeId":"1","name":{"value":original}}]});
            let values: Vec<String> = values.into_iter().map(String::from).collect();
            let safe =
                redact_response("Accessibility.getFullAXTree", raw, &HashSet::new(), &values)
                    .unwrap();
            assert_eq!(safe["nodes"][0]["name"]["value"], expected);
        }
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn close_does_not_wait_for_a_blocked_pipe_writer() {
        use std::os::fd::FromRawFd;
        fn pipe() -> (File, File) {
            let mut fds = [0; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) }
        }
        let (host_read, peer_write) = pipe();
        let (peer_read, host_write) = pipe();
        let client = Arc::new(CdpClient::from_pipe(host_read, host_write).unwrap());
        let writer = client.clone();
        let request = tokio::spawn(async move {
            writer
                .send_command(
                    "fixture",
                    Some(json!({"payload":"x".repeat(1024 * 1024)})),
                    None,
                )
                .await
        });
        wait_for_pipe_data(&peer_read);
        assert_eq!(client.pending_len().await, 1);
        let close_client = client.clone();
        let close = tokio::spawn(async move { close_client.close().await });
        tokio::time::timeout(std::time::Duration::from_millis(500), close)
            .await
            .expect("close timed out")
            .expect("close task failed");
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), request)
            .await
            .expect("writer timed out")
            .expect("writer task failed");
        assert!(result.is_err());
        assert_eq!(client.pending_len().await, 0);
        drop(peer_read);
        drop(peer_write);
    }
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_command_releases_pending_entry() {
        use std::os::fd::FromRawFd;
        fn pipe() -> (File, File) {
            let mut fds = [0; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) }
        }
        let (host_read, _peer_write) = pipe();
        let (peer_read, host_write) = pipe();
        let client = Arc::new(CdpClient::from_pipe(host_read, host_write).unwrap());
        let worker = client.clone();
        let request = tokio::spawn(async move {
            worker
                .send_command(
                    "fixture",
                    Some(json!({"payload":"x".repeat(1024 * 1024)})),
                    None,
                )
                .await
        });
        wait_for_pipe_data(&peer_read);
        assert_eq!(client.pending_len().await, 1);
        request.abort();
        assert!(request.await.unwrap_err().is_cancelled());
        assert_eq!(client.pending_len().await, 0);
        client.close().await;
    }
    #[cfg(unix)]
    #[test]
    fn nonblocking_reader_keeps_split_and_coalesced_frames() {
        use std::os::fd::FromRawFd;
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        let (mut read, mut write) =
            unsafe { (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1])) };
        nonblocking(&read, false).unwrap();
        write.write_all(b"-part\0second\0").unwrap();
        let mut buffered = b"first".to_vec();
        let closed = AtomicBool::new(false);
        assert_eq!(
            read_frame(&mut read, &mut buffered, &closed).unwrap(),
            b"first-part"
        );
        assert_eq!(
            read_frame(&mut read, &mut buffered, &closed).unwrap(),
            b"second"
        );
    }
    #[test]
    fn repeated_short_values_redact_original_text_without_expanding_markers() {
        let raw =
            json!({"nodes":[{"nodeId":"1","role":{"value":"button"},"name":{"value":"a e"}}]});
        let values = vec!["a".into(), "e".into(), "a".into(), "e".into()];
        let safe =
            redact_response("Accessibility.getFullAXTree", raw, &HashSet::new(), &values).unwrap();
        assert_eq!(safe["nodes"][0]["name"]["value"], "[redacted] [redacted]");
        let raw = json!({"nodes":[{"nodeId":"1","name":{"value":"a".repeat(10_000)}}]});
        let safe =
            redact_response("Accessibility.getFullAXTree", raw, &HashSet::new(), &values).unwrap();
        assert_eq!(safe["nodes"][0]["name"]["value"], "[redacted]");
        let mut buckets: [Vec<&str>; 256] = std::array::from_fn(|_| Vec::new());
        buckets[b'a' as usize].push("a");
        assert!(redact_original("aaaa", &buckets, &mut 3, &mut 100).is_err());
    }
    #[test]
    fn debugger_notifications_never_retain_foreign_urls_or_stack_values() {
        let marker = "clovy-private-observation:aaaaaaaaaaaa4aaa8aaaaaaaaaaaaaaa";
        let own = format!("br:1:0:0:{marker}");
        assert!(valid_observation_breakpoint(&own, marker));
        let foreign = "br:1:0:0:https://private.invalid/?otp=123456";
        assert!(!valid_observation_breakpoint(foreign, marker));
        let raw = json!({"hitBreakpoints":[own,foreign],"callFrames":[{"url":"secret-url","scopeChain":[{"value":"secret-local"}]}]});
        let safe = observation_event("Debugger.paused", &raw, &HashSet::from([own.clone()]));
        assert_eq!(safe["ownHits"], json!([own]));
        assert_eq!(safe["foreignPaused"], true);
        let encoded = safe.to_string();
        assert!(
            !encoded.contains("private.invalid")
                && !encoded.contains("123456")
                && !encoded.contains("secret-url")
                && !encoded.contains("secret-local")
        );
        assert_eq!(
            observation_event(
                "Debugger.paused",
                &json!({"hitBreakpoints":[]}),
                &HashSet::new()
            )["foreignPaused"],
            true
        );
    }

    #[test]
    fn ax_current_and_new_field_values_redact_names_without_stale_inventory() {
        let raw = json!({"nodes":[
            {"nodeId":"1","role":{"value":"textbox"},"name":{"value":"private-B","sources":[{"value":"private-B"}]},"value":{"value":"private-B"}},
            {"nodeId":"2","role":{"value":"textbox"},"name":{"value":"Fresh field"},"value":{"value":"new-C"},"childIds":["3"]},
            {"nodeId":"3","role":{"value":"StaticText"},"name":{"value":"new-C"}},
            {"nodeId":"4","role":{"value":"button"},"name":{"value":"Query"}},
            {"nodeId":"5","role":{"value":"button"},"name":{"value":"Copied new-C"}}
        ]});
        let safe = redact_response(
            "Accessibility.getFullAXTree",
            raw,
            &HashSet::new(),
            &["old-A".into()],
        )
        .unwrap();
        let encoded = safe.to_string();
        assert!(!encoded.contains("private-B") && !encoded.contains("new-C"));
        assert_eq!(safe["nodes"][1]["name"]["value"], "Fresh field");
        assert_eq!(safe["nodes"][3]["name"]["value"], "Query");
        assert_eq!(safe["nodes"][4]["name"]["value"], "Copied [redacted]");
    }
}
