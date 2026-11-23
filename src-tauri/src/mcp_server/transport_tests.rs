//! Portable behavioral tests over the actual channel and relay. No CLI login,
//! app profile, shell fixture or desktop permission is involved.
use super::*;
use crate::mcp_server::{protocol, stdio};
use std::io::{BufRead, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn raw(dir: &std::path::Path) -> SyncStream {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(socket_path(dir)).unwrap()
    }
    #[cfg(windows)]
    {
        super::windows::SyncStream::open_raw(dir).unwrap()
    }
}
fn hello(dir: &std::path::Path, offered: Value) -> (SyncStream, Value) {
    let mut stream = raw(dir);
    stream
        .set_read_timeout(Some(Duration::from_secs(7)))
        .unwrap();
    stream.write_all(format!("{offered}\n").as_bytes()).unwrap();
    let mut reply = String::new();
    std::io::BufReader::new(&mut stream)
        .read_line(&mut reply)
        .unwrap();
    (stream, serde_json::from_str(&reply).unwrap())
}
fn binding() -> crate::mcp_server::turns::TurnBinding {
    crate::mcp_server::turns::TurnBinding {
        authority: crate::domain::turn_authority::TurnAuthority::new(),
        wire_names: None,
        session_id: "synthetic-session".into(),
        run_id: "synthetic-run".into(),
        data_partition: "frozen-host-partition".into(),
        cli: crate::llm::cli::CliKind::Claude,
        safety_mode: crate::agent_sessions::domain::AgentSafetyMode::Sandboxed,
        attended: true,
        surface: "chat".into(),
    }
}

#[derive(Clone, Default)]
pub(super) struct Output(Arc<Mutex<Vec<u8>>>);
impl Write for Output {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl Output {
    pub(super) async fn response(&self) -> Value {
        self.responses(1).await.remove(0)
    }
    async fn responses(&self, count: usize) -> Vec<Value> {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let bytes = self.0.lock().unwrap().clone();
                let messages: Vec<Value> = bytes
                    .split(|b| *b == b'\n')
                    .filter_map(|line| serde_json::from_slice(line).ok())
                    .collect();
                if messages.len() >= count {
                    return messages;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("relay must answer")
    }
}

// Keep stdin open until explicitly released, like a real MCP client.
pub(super) struct Input {
    pub bytes: std::io::Cursor<Vec<u8>>,
    pub end: std::sync::mpsc::Receiver<()>,
}
impl Read for Input {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let b = self.fill_buf()?;
        let n = b.len().min(out.len());
        out[..n].copy_from_slice(&b[..n]);
        self.consume(n);
        Ok(n)
    }
}
impl BufRead for Input {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        if self.bytes.position() as usize == self.bytes.get_ref().len() {
            let _ = self.end.recv();
        }
        self.bytes.fill_buf()
    }
    fn consume(&mut self, n: usize) {
        self.bytes.consume(n);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lost_receipt_after_real_mutation_is_unknown_and_never_replayed() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let target = root.path().join("effects");
    let observed = target.clone();
    let admitted = Arc::new(tokio::sync::Notify::new());
    let signal = admitted.clone();
    let handler: Handler = Arc::new(move |_| {
        let target = target.clone();
        let signal = signal.clone();
        Box::pin(async move {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(target)
                .unwrap()
                .write_all(b"effect\n")
                .unwrap();
            signal.notify_one();
            std::future::pending().await
        })
    });
    let listener = Listener::start(&dir, handler).await.unwrap();
    let (end, wait) = std::sync::mpsc::channel();
    let output = Output::default();
    let sink = output.clone();
    let relay = tokio::task::spawn_blocking(move || {
        stdio::relay(
            &dir,
            Input {
                bytes: std::io::Cursor::new(
                    b"{\"jsonrpc\":\"2.0\",\"id\":73,\"method\":\"tools/call\",\"params\":{}}\n"
                        .to_vec(),
                ),
                end: wait,
            },
            Box::new(sink),
        )
    });
    tokio::time::timeout(Duration::from_secs(5), admitted.notified())
        .await
        .unwrap();
    listener.stop();
    let reply = output.response().await;
    assert_eq!(reply["id"], 73);
    let details = &reply["result"]["structuredContent"]["error"]["details"];
    assert_eq!(details["delivery"], "unknown", "{reply}");
    assert_eq!(details["effect"], "unknown");
    assert_eq!(details["retrySafe"], false);
    assert_eq!(details["callId"], 73);
    assert_eq!(std::fs::read_to_string(observed).unwrap(), "effect\n");
    drop(end);
    relay.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_installations_keep_distinct_secrets_and_real_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let a = channel_dir(&root.path().join("a"));
    let b = channel_dir(&root.path().join("b"));
    std::fs::create_dir_all(a.parent().unwrap()).unwrap();
    std::fs::create_dir_all(b.parent().unwrap()).unwrap();
    let handler: Handler = Arc::new(|message| {
        Box::pin(async move { Some(protocol::result_response(message["id"].clone(), json!({}))) })
    });
    let first = Listener::start(&a, handler.clone()).await.unwrap();
    let second = Listener::start(&b, handler).await.unwrap();
    assert_ne!(first.socket(), second.socket());
    assert_ne!(read_secret(&a).unwrap(), read_secret(&b).unwrap());
    tokio::task::spawn_blocking(move || {
        for dir in [a, b] {
            let mut stream = connect(&dir).unwrap();
            stream
                .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
                .unwrap();
            let mut reply = String::new();
            std::io::BufReader::new(&mut stream)
                .read_line(&mut reply)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&reply).unwrap()["result"],
                json!({})
            );
        }
    })
    .await
    .unwrap();
    first.stop();
    second.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_closes_a_silent_handshake_without_waiting_five_seconds() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let handler: Handler = Arc::new(|_| Box::pin(async { None }));
    let listener = Listener::start(&dir, handler).await.unwrap();
    let clone = dir.clone();
    let pending = tokio::task::spawn_blocking(move || {
        let mut pending = Vec::new();
        for _ in 0..MAX_PENDING_HANDSHAKES {
            let stream = raw(&clone);
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            pending.push(stream);
        }
        // The excess peer closes only after accept has reserved every
        // handshake slot. Keep those silent peers open through stop.
        let mut excess = raw(&clone);
        excess
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let result = excess.read(&mut [0; 1]);
        assert!(
            matches!(&result, Ok(0))
                || matches!(&result, Err(e) if matches!(
                    e.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                )),
            "handshake slots must actually be occupied: {result:?}"
        );
        pending
    })
    .await
    .unwrap();
    let start = Instant::now();
    listener.stop();
    tokio::task::spawn_blocking(move || {
        for mut stream in pending {
            let result = stream.read(&mut [0; 256]);
            assert!(
                matches!(&result, Ok(0))
                    || matches!(&result, Err(e) if matches!(
                        e.kind(),
                        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                    )),
                "listener must actually close: {result:?}"
            );
        }
        assert!(start.elapsed() < Duration::from_secs(2));
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn peer_secret_version_token_order_and_eight_connection_slots() {
    use crate::mcp_server::turns::TurnRegistry;
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let turns = TurnRegistry::default();
    let token = turns.issue(binding());
    let handler: Handler = Arc::new(|_| Box::pin(async { None }));
    let listener = Listener::start_with_turns(&dir, handler, turns.clone())
        .await
        .unwrap();
    tokio::task::spawn_blocking(move || {
        let secret = read_secret(&dir).unwrap();
        for offered in [
            json!({"clovyMcp":2,"secret":secret}),
            json!({"clovyMcp":1,"secret":"wrong","turn":token}),
        ] {
            assert_eq!(hello(&dir, offered).1["error"], "unauthorized");
        }
        assert_eq!(
            hello(&dir, turn_handshake(&secret, Some("unknown"))).1["error"],
            "turn_expired"
        );
        let bound = connect_turn(&dir, Some(&token)).unwrap();
        let mut clients = vec![bound];
        for _ in 1..MAX_CONNECTIONS {
            clients.push(connect(&dir).unwrap());
        }
        assert!(matches!(connect(&dir), Err(ClientError::Busy)));
        turns.revoke(&token);
        let mut bound = clients.remove(0);
        let mut b = [0; 1];
        assert_eq!(bound.read(&mut b).unwrap(), 0);
        assert!(matches!(
            connect_turn(&dir, Some(&token)),
            Err(ClientError::TurnExpired)
        ));
        // Unbound connections survived the other token's revocation.
        for client in &clients {
            client
                .set_read_timeout(Some(Duration::from_millis(50)))
                .unwrap();
        }
        assert!(clients[0].read(&mut b).is_err());
    })
    .await
    .unwrap();
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn four_pending_handshakes_and_timeout_recover_capacity() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let handler: Handler = Arc::new(|_| Box::pin(async { None }));
    let listener = Listener::start(&dir, handler).await.unwrap();
    tokio::task::spawn_blocking(move || {
        let mut pending = Vec::new();
        for _ in 0..MAX_PENDING_HANDSHAKES {
            pending.push(raw(&dir));
            std::thread::sleep(Duration::from_millis(30));
        }
        let mut excess = raw(&dir);
        excess
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut byte = [0; 1];
        let closed = excess.read(&mut byte);
        assert!(
            matches!(closed, Ok(0))
                || closed.is_err_and(|e| matches!(
                    e.kind(),
                    std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                ))
        );
        let start = Instant::now();
        for stream in &mut pending {
            stream
                .set_read_timeout(Some(Duration::from_secs(7)))
                .unwrap();
            let mut line = String::new();
            std::io::BufReader::new(stream)
                .read_line(&mut line)
                .unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&line).unwrap()["error"],
                "unauthorized"
            );
        }
        assert!(start.elapsed() >= Duration::from_secs(4));
        assert!(connect(&dir).is_ok());
    })
    .await
    .unwrap();
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn four_inflight_out_of_order_framing_and_revocation_receipt() {
    use crate::mcp_server::turns::TurnRegistry;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let admitted = Arc::new(AtomicUsize::new(0));
    let count = admitted.clone();
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let gate = release.clone();
    let turns = TurnRegistry::default();
    let token = turns.issue(binding());
    let handler: TurnHandler = Arc::new(move |message, binding| {
        let count = count.clone();
        let gate = gate.clone();
        Box::pin(async move {
            assert_eq!(
                binding.as_ref().unwrap().data_partition,
                "frozen-host-partition"
            );
            count.fetch_add(1, Ordering::SeqCst);
            let permit = gate.acquire().await.unwrap();
            permit.forget();
            if message["id"] == 1 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Some(protocol::result_response(
                message["id"].clone(),
                json!({"isError":true,"structuredContent":{"error":{"code":"receipt","details":{"delivery":"delivered","effect":"unknown","retrySafe":false,"callId":message["id"]}}}}),
            ))
        })
    });
    let listener = Listener::start_with_turns(&dir, handler, turns.clone())
        .await
        .unwrap();
    let (sent, ready) = tokio::sync::oneshot::channel();
    let client = tokio::task::spawn_blocking(move || {
        let mut stream = connect_turn(&dir, Some(&token)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        for id in 1..=5 {
            stream
                .write_all(
                    format!(
                        "{}\n",
                        json!({"jsonrpc":"2.0","id":id,"method":"tools/call"})
                    )
                    .as_bytes(),
                )
                .unwrap();
        }
        sent.send(()).unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let mut answers = Vec::new();
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            answers.push(serde_json::from_str::<Value>(&line).unwrap());
        }
        answers
    });
    ready.await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while admitted.load(Ordering::SeqCst) != 4 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    turns.revoke_all();
    release.add_permits(4);
    let replies = client.await.unwrap();
    assert_eq!(admitted.load(Ordering::SeqCst), 4);
    assert_eq!(replies.len(), 4);
    assert_ne!(replies[0]["id"], 1);
    for reply in replies {
        assert_eq!(
            reply["result"]["structuredContent"]["error"]["code"],
            "receipt"
        );
    }
    listener.stop();
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_timeout_shutdown_first_instance_and_private_storage() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let handler: Handler = Arc::new(|_| Box::pin(async { None }));
    let listener = Listener::start(&dir, handler.clone()).await.unwrap();
    assert!(Listener::start(&dir, handler).await.is_err());
    tokio::task::spawn_blocking(move || {
        let mut stream = raw(&dir);
        stream
            .set_read_timeout(Some(Duration::from_millis(70)))
            .unwrap();
        let mut b = [0; 1];
        assert_eq!(
            stream.read(&mut b).unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
        // A cancelled async read neither loses its driver nor frees native
        // pending buffers early. The very same connection can authenticate.
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(format!("{}\n", handshake(&read_secret(&dir).unwrap())).as_bytes())
            .unwrap();
        let mut reply = String::new();
        std::io::BufReader::new(&mut stream)
            .read_line(&mut reply)
            .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&reply).unwrap()["ok"], true);
        let mut reader = stream.try_clone().unwrap();
        reader.set_read_timeout(None).unwrap();
        let read = std::thread::spawn(move || reader.read(&mut [0; 1]));
        std::thread::sleep(Duration::from_millis(50));
        stream.shutdown(std::net::Shutdown::Both).unwrap();
        assert_eq!(
            read.join().unwrap().unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        // Closure remains recorded even between operations (no watch waiter).
        let mut closed = raw(&dir);
        closed.shutdown(std::net::Shutdown::Both).unwrap();
        assert_eq!(
            closed.read(&mut b).unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        let original = read_secret(&dir).unwrap();
        let alias = root.path().join("secret-alias");
        std::fs::hard_link(secret_path(&dir), &alias).unwrap();
        assert!(read_secret(&dir).is_err());
        std::fs::remove_file(alias).unwrap();
        assert_eq!(read_secret(&dir).unwrap(), original);
    })
    .await
    .unwrap();
    listener.stop();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_joins_nonreading_peer_and_immediate_restart_uses_same_secret() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let signal = Arc::new(tokio::sync::Notify::new());
    let admitted = signal.clone();
    let handler: Handler = Arc::new(move |message| {
        let admitted = admitted.clone();
        Box::pin(async move {
            admitted.notify_one();
            Some(protocol::result_response(
                message["id"].clone(),
                json!({"data":"z".repeat(3 * 1024 * 1024)}),
            ))
        })
    });
    let listener = Listener::start(&dir, handler.clone()).await.unwrap();
    let original = read_secret(&dir).unwrap();
    let clone = dir.clone();
    let stream = tokio::task::spawn_blocking(move || {
        let mut stream = connect(&clone).unwrap();
        stream
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
            .unwrap();
        stream
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(3), signal.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), listener.stop_and_wait())
        .await
        .expect("shutdown joins a blocked writer");
    let restarted = Listener::start(&dir, handler).await.unwrap();
    assert_eq!(read_secret(&dir).unwrap(), original);
    let check = dir.clone();
    tokio::task::spawn_blocking(move || {
        assert!(connect(&check).is_ok());
        drop(stream);
    })
    .await
    .unwrap();
    restarted.stop_and_wait().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_large_receipt_is_completed_before_graceful_close() {
    use crate::mcp_server::turns::TurnRegistry;
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let turns = TurnRegistry::default();
    let token = turns.issue(binding());
    let admitted = Arc::new(tokio::sync::Notify::new());
    let signal = admitted.clone();
    let release = Arc::new(tokio::sync::Notify::new());
    let gate = release.clone();
    let handler: TurnHandler = Arc::new(move |message, _| {
        let signal = signal.clone();
        let gate = gate.clone();
        Box::pin(async move {
            signal.notify_one();
            gate.notified().await;
            Some(protocol::result_response(
                message["id"].clone(),
                json!({"data":"r".repeat(3 * 1024 * 1024),"receipt":{"delivery":"delivered","effect":"unknown","retrySafe":false}}),
            ))
        })
    });
    let listener = Listener::start_with_turns(&dir, handler, turns.clone())
        .await
        .unwrap();
    let client = tokio::task::spawn_blocking(move || {
        let mut stream = connect_turn(&dir, Some(&token)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":22,\"method\":\"tools/call\"}\n")
            .unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(
            reply["result"]["data"].as_str().unwrap().len(),
            3 * 1024 * 1024
        );
        assert_eq!(reply["result"]["receipt"]["delivery"], "delivered");
        line.clear();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
    });
    tokio::time::timeout(Duration::from_secs(3), admitted.notified())
        .await
        .unwrap();
    turns.revoke_all();
    release.notify_one();
    client.await.unwrap();
    listener.stop_and_wait().await;
}

#[cfg(windows)]
#[test]
fn windows_refuses_ambiguous_storage_and_preserves_malformed_secret() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    // A default-inherited directory cannot silently become trusted storage.
    std::fs::create_dir(&dir).unwrap();
    assert!(super::windows::read_secret(&dir).is_err());
    std::fs::remove_dir(&dir).unwrap();
    let original = super::windows::security_test_secret(&dir).unwrap();
    assert_eq!(read_secret(&dir).unwrap(), original);
    std::fs::write(secret_path(&dir), "malformed").unwrap();
    assert!(super::windows::security_test_secret(&dir).is_err());
    assert_eq!(
        std::fs::read_to_string(secret_path(&dir)).unwrap(),
        "malformed"
    );
}

#[test]
fn existing_defaults_and_persisted_mcp_opt_out_are_preserved() {
    use crate::activity::settings::{self, ActivitySettings};
    let root = tempfile::tempdir().unwrap();
    let path = settings::settings_path(root.path());
    let mut stored = settings::load_at_startup(&path);
    assert!(stored.mcp_server);
    // A current-version opt-out must survive saves and startup unchanged.
    stored.enabled = false;
    stored.mcp_server = false;
    settings::save(&path, &stored).unwrap();
    assert_eq!(settings::load_at_startup(&path), stored);
    stored.mcp_server = true;
    settings::save(&path, &stored).unwrap();
    let after = settings::load_at_startup(&path);
    assert!(after.mcp_server);
    assert!(!after.enabled);
    assert_eq!(
        after.coding_agents,
        ActivitySettings::default().coding_agents
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_partial_write_is_not_replayed_after_transport_failure() {
    // Fault injection at the actual socket, after a valid handshake. The peer
    // drops a request prefix, retaining the listener to detect a replay. It
    // neither echoes requests nor supplies a tool result.
    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let handler: Handler = Arc::new(|_| Box::pin(async { None }));
    let listener = Listener::start(&dir, handler).await.unwrap();
    listener.stop_and_wait().await;
    let socket = tokio::net::UnixListener::bind(socket_path(&dir)).unwrap();
    let fault = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
        let (stream, _) = socket.accept().await.unwrap();
        let mut stream = tokio::io::BufReader::new(stream);
        let mut hello = String::new();
        stream.read_line(&mut hello).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&hello).unwrap()["clovyMcp"],
            1
        );
        stream
            .get_mut()
            .write_all(b"{\"clovyMcp\":1,\"ok\":true}\n")
            .await
            .unwrap();
        let mut prefix = [0; 128];
        stream.read_exact(&mut prefix).await.unwrap();
        assert!(!prefix.contains(&b'\n'));
        drop(stream);
        assert!(
            tokio::time::timeout(Duration::from_millis(400), socket.accept())
                .await
                .is_err(),
            "request was replayed over a new connection"
        );
    });
    let output = Output::default();
    let sink = output.clone();
    let (end, wait) = std::sync::mpsc::channel();
    let input = format!(
        "{}\n",
        json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"fixture_write","arguments":{"body":"z".repeat(3 * 1024 * 1024)}}})
    );
    let relay = tokio::task::spawn_blocking(move || {
        stdio::relay(
            &dir,
            Input {
                bytes: std::io::Cursor::new(input.into_bytes()),
                end: wait,
            },
            Box::new(sink),
        )
    });
    let reply = output.response().await;
    assert_eq!(
        reply["result"]["structuredContent"]["error"]["details"],
        json!({"delivery":"unknown","effect":"unknown","retrySafe":false,"callId":99})
    );
    fault.await.unwrap();
    drop(end);
    relay.await.unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn independent_request_reconnects_after_partial_write_before_reader_drains() {
    // EOF cannot mark the first connection dead until this test releases its
    // local gate. The next stdin request therefore exposes the write path's
    // own decision, independently of reader-thread scheduling.
    struct AtEnd {
        input: Input,
        attempted: Option<tokio::sync::oneshot::Sender<()>>,
        peer_done: Arc<tokio::sync::Notify>,
    }
    impl Read for AtEnd {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(bytes)
        }
    }
    impl BufRead for AtEnd {
        fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
            if self.input.bytes.position() as usize == self.input.bytes.get_ref().len() {
                if let Some(attempted) = self.attempted.take() {
                    self.peer_done.notify_one();
                    let _ = attempted.send(());
                }
            }
            self.input.fill_buf()
        }
        fn consume(&mut self, count: usize) {
            self.input.consume(count);
        }
    }

    let root = tempfile::tempdir().unwrap();
    let dir = channel_dir(root.path());
    let handler: Handler = Arc::new(|_| Box::pin(async { None }));
    let listener = Listener::start(&dir, handler).await.unwrap();
    listener.stop_and_wait().await;
    let socket = tokio::net::UnixListener::bind(socket_path(&dir)).unwrap();
    let effects = root.path().join("independent-effects");
    std::fs::write(&effects, b"").unwrap();
    let target = effects.clone();
    let peer_done = Arc::new(tokio::sync::Notify::new());
    let done = peer_done.clone();
    let peer = tokio::spawn(async move {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
        let (stream, _) = socket.accept().await.unwrap();
        let mut first = tokio::io::BufReader::new(stream);
        let mut hello = String::new();
        first.read_line(&mut hello).await.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&hello).unwrap()["clovyMcp"],
            1
        );
        first
            .get_mut()
            .write_all(b"{\"clovyMcp\":1,\"ok\":true}\n")
            .await
            .unwrap();
        let mut prefix = [0; 128];
        first.read_exact(&mut prefix).await.unwrap();
        assert!(!prefix.contains(&b'\n'));
        drop(first);

        let mut applied = Vec::new();
        let accepted = tokio::select! {
            biased;
            accepted = socket.accept() => Some(accepted.unwrap().0),
            _ = done.notified() => None,
        };
        if let Some(stream) = accepted {
            let mut next = tokio::io::BufReader::new(stream);
            hello.clear();
            next.read_line(&mut hello).await.unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(&hello).unwrap()["clovyMcp"],
                1
            );
            next.get_mut()
                .write_all(b"{\"clovyMcp\":1,\"ok\":true}\n")
                .await
                .unwrap();
            loop {
                let mut line = String::new();
                if next.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                let message: Value = serde_json::from_str(&line).unwrap();
                let id = message["id"].clone();
                // A real append and its receipt, rather than an echo peer.
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(&target)
                    .unwrap()
                    .write_all(format!("{id}\n").as_bytes())
                    .unwrap();
                applied.push(id.clone());
                let response = protocol::result_response(
                    id.clone(),
                    json!({"isError":false,"receipt":{"callId":id,"delivery":"delivered","effect":"applied","retrySafe":false}}),
                );
                next.get_mut()
                    .write_all(format!("{response}\n").as_bytes())
                    .await
                    .unwrap();
            }
        }
        applied
    });
    let output = Output::default();
    let sink = output.clone();
    let (end, wait) = std::sync::mpsc::channel();
    let (attempted, attempts_done) = tokio::sync::oneshot::channel();
    let (pause, resume) = stdio::reader_exit_pause();
    let input = format!(
        "{}\n{}\n",
        json!({"jsonrpc":"2.0","id":99,"method":"tools/call","params":{"name":"fixture_write","arguments":{"body":"z".repeat(3 * 1024 * 1024)}}}),
        json!({"jsonrpc":"2.0","id":100,"method":"tools/call","params":{"name":"fixture_write","arguments":{"body":"independent"}}})
    );
    let relay = tokio::task::spawn_blocking(move || {
        stdio::relay_with_reader_pause(
            &dir,
            AtEnd {
                input: Input {
                    bytes: std::io::Cursor::new(input.into_bytes()),
                    end: wait,
                },
                attempted: Some(attempted),
                peer_done,
            },
            Box::new(sink),
            pause,
        );
    });
    tokio::time::timeout(Duration::from_secs(8), attempts_done)
        .await
        .unwrap()
        .unwrap();
    // In GREEN, Drop already released the first connection before reconnect.
    // In RED, both writes have finished on the old connection. Release before
    // collecting results/joining so neither branch can wait on its own hook.
    let _ = resume.send(());
    let _ = output.responses(2).await;
    drop(end);
    relay.await.unwrap();
    let applied = peer.await.unwrap();
    let replies = output.responses(2).await;
    assert_eq!(replies.len(), 2, "one result per independent request");
    let original = replies.iter().find(|reply| reply["id"] == 99).unwrap();
    assert_eq!(
        original["result"]["structuredContent"]["error"]["code"],
        "mcp_delivery_unknown"
    );
    assert_eq!(
        original["result"]["structuredContent"]["error"]["details"],
        json!({"delivery":"unknown","effect":"unknown","retrySafe":false,"callId":99})
    );
    let independent = replies.iter().find(|reply| reply["id"] == 100).unwrap();
    assert_eq!(
        independent["result"]["receipt"],
        json!({"callId":100,"delivery":"delivered","effect":"applied","retrySafe":false}),
        "independent request did not reconnect: {independent}"
    );
    assert_eq!(applied, vec![json!(100)], "the original must never replay");
    assert_eq!(std::fs::read_to_string(effects).unwrap(), "100\n");
}
