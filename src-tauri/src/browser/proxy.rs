//! Pinning forward proxy for Clovy's private browser.
//!
//! The private transport launches its pinned Chrome with
//! `--proxy-server=http://127.0.0.1:<port>` pointed here, so every browser
//! connection - first navigation, every redirect hop, every subresource, and
//! the CONNECT for https/wss - flows through this proxy. For each connection
//! the proxy resolves the hostname ITSELF and validates every resolved address
//! ([`super::policy::resolve_validated`]) before connecting, and connects only
//! to a validated address. Resolution and connection are one atomic act per
//! connection: that is the DNS-rebinding pin. Because every forwarded HTTP
//! request carries `Connection: close`, each redirect hop opens a fresh
//! connection and re-enters validation.
//!
//! ## Privacy
//!
//! This module never logs, prints, or traces anything about a request: no
//! URLs, hosts, paths, or bodies, on any path including errors. Blocked hosts
//! are kept in a small in-memory ring only, so the navigate tool can map a
//! connection-time refusal to a clear error; they never leave the process.

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::policy::{resolve_validated, PolicyConfig, Resolver};

/// Maximum bytes of a request head we will buffer before giving up (protects
/// against a client that never sends the header terminator).
const MAX_HEAD_BYTES: usize = 32 * 1024;
/// How long we wait for the full request head.
const HEAD_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// How long we wait for a single upstream connect attempt.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How many recently blocked hosts to retain for error mapping.
const BLOCK_RING_CAP: usize = 32;

type BlockRing = Arc<Mutex<VecDeque<(Instant, String)>>>;

/// A loopback forward proxy that pins every browser connection to an address it
/// resolved and validated itself. Dropping it (or calling [`shutdown`]) stops
/// the accept loop; in-flight tunnels then die with their sockets.
///
/// [`shutdown`]: PinningProxy::shutdown
pub struct PinningProxy {
    port: u16,
    blocks: BlockRing,
    accept_task: JoinHandle<()>,
}

impl PinningProxy {
    /// Bind `127.0.0.1:0`, start accepting, and return the running proxy. The
    /// `config` is normally [`PolicyConfig::default`]; the E2E harness passes
    /// `allow_loopback` so it can pin to loopback fixtures.
    pub async fn start(
        resolver: Arc<dyn Resolver>,
        config: PolicyConfig,
    ) -> std::io::Result<PinningProxy> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let blocks: BlockRing = Arc::new(Mutex::new(VecDeque::new()));

        let accept_blocks = blocks.clone();
        let accept_task = tokio::spawn(async move {
            // Own every CONNECT tunnel as well as the listener. Revocation
            // must also close an already established connection.
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let accepted = tokio::select! {
                    accepted = listener.accept() => accepted,
                    _ = connections.join_next(), if !connections.is_empty() => continue,
                };
                let stream = match accepted {
                    Ok((stream, _peer)) => stream,
                    // A broken loopback listener will not recover; stop cleanly.
                    Err(_) => break,
                };
                let resolver = resolver.clone();
                let config = config.clone();
                let blocks = accept_blocks.clone();
                connections.spawn(async move {
                    // Errors are per-connection and carry no reportable content;
                    // the socket simply closes.
                    let _ = handle_connection(stream, resolver, config, blocks).await;
                });
            }
        });

        Ok(PinningProxy {
            port,
            blocks,
            accept_task,
        })
    }

    /// The loopback port the browser must proxy through.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The most recently blocked host recorded after `instant`, if any. Lets
    /// the navigate tool turn a connection-time refusal into a clear error. The
    /// host stays in memory and is never logged.
    pub fn blocked_since(&self, instant: Instant) -> Option<String> {
        let ring = self.blocks.lock().expect("block ring poisoned");
        ring.iter()
            .rev()
            .find(|(at, _)| *at >= instant)
            .map(|(_, host)| host.clone())
    }

    /// Stop accepting new connections. Idempotent; Drop is the backstop.
    pub fn shutdown(&self) {
        self.accept_task.abort();
    }
}

impl Drop for PinningProxy {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    struct FixtureResolver(SocketAddr);
    impl Resolver for FixtureResolver {
        fn resolve<'a>(
            &'a self,
            _host: &'a str,
            _port: u16,
        ) -> crate::browser::BoxFuture<'a, std::io::Result<Vec<SocketAddr>>> {
            Box::pin(async move { Ok(vec![self.0]) })
        }
    }
    #[tokio::test]
    async fn shutdown_closes_an_already_established_connect_tunnel() {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy = PinningProxy::start(
            Arc::new(FixtureResolver(upstream.local_addr().unwrap())),
            PolicyConfig {
                allow_loopback: true,
            },
        )
        .await
        .unwrap();
        let mut client = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        client
            .write_all(b"CONNECT fixture.invalid:443 HTTP/1.1\r\nHost: fixture.invalid:443\r\n\r\n")
            .await
            .unwrap();
        let (_peer, _) = upstream.accept().await.unwrap();
        let mut response = [0; 256];
        let count = client.read(&mut response).await.unwrap();
        assert!(String::from_utf8_lossy(&response[..count]).contains("200"));
        proxy.shutdown();
        let count = timeout(Duration::from_secs(1), client.read(&mut response))
            .await
            .expect("an established tunnel survived revocation")
            .unwrap();
        assert_eq!(count, 0);
    }
}

fn record_block(blocks: &BlockRing, host: String) {
    let mut ring = blocks.lock().expect("block ring poisoned");
    if ring.len() >= BLOCK_RING_CAP {
        ring.pop_front();
    }
    ring.push_back((Instant::now(), host));
}

/// The parsed first line of a proxied request head.
enum Request {
    /// `CONNECT host:port HTTP/1.1` (https/wss tunnels).
    Connect { host: String, port: u16 },
    /// Absolute-form plain HTTP: `GET http://host/path HTTP/1.1`.
    AbsoluteHttp { method: String, version: String },
    /// Origin-form or otherwise unsupported at a proxy.
    Unsupported,
}

async fn handle_connection(
    mut client: TcpStream,
    resolver: Arc<dyn Resolver>,
    config: PolicyConfig,
    blocks: BlockRing,
) -> std::io::Result<()> {
    let head = match read_head(&mut client).await {
        Ok(Some(head)) => head,
        // Malformed, oversized, or truncated head: refuse without detail.
        Ok(None) | Err(_) => {
            let _ = write_status(&mut client, "400 Bad Request").await;
            return Ok(());
        }
    };

    let request_line = match head.request_line() {
        Some(line) => line,
        None => {
            let _ = write_status(&mut client, "400 Bad Request").await;
            return Ok(());
        }
    };

    match parse_request_line(request_line) {
        Request::Connect { host, port } => {
            handle_connect(client, &host, port, &resolver, &config, &blocks).await
        }
        Request::AbsoluteHttp { method, version } => {
            handle_absolute_http(
                client,
                &head,
                request_line,
                &method,
                &version,
                &resolver,
                &config,
                &blocks,
            )
            .await
        }
        Request::Unsupported => {
            let _ = write_status(&mut client, "400 Bad Request").await;
            Ok(())
        }
    }
}

async fn handle_connect(
    mut client: TcpStream,
    host: &str,
    port: u16,
    resolver: &Arc<dyn Resolver>,
    config: &PolicyConfig,
    blocks: &BlockRing,
) -> std::io::Result<()> {
    let addrs = match resolve_validated(host, port, resolver.as_ref(), config).await {
        Ok(addrs) => addrs,
        Err(_) => {
            record_block(blocks, host.to_string());
            let _ = write_status(&mut client, "403 Forbidden").await;
            return Ok(());
        }
    };

    let mut upstream = match connect_first(&addrs).await {
        Some(upstream) => upstream,
        None => {
            let _ = write_status(&mut client, "502 Bad Gateway").await;
            return Ok(());
        }
    };

    write_status(&mut client, "200 Connection Established").await?;
    // The tunnel is opaque from here: copy bytes both ways until either side
    // closes. We never inspect the ciphertext.
    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_absolute_http(
    mut client: TcpStream,
    head: &RequestHead,
    request_line: &str,
    method: &str,
    version: &str,
    resolver: &Arc<dyn Resolver>,
    config: &PolicyConfig,
    blocks: &BlockRing,
) -> std::io::Result<()> {
    // The absolute target is the second token of the request line.
    let target = match request_line.split_whitespace().nth(1) {
        Some(target) => target,
        None => {
            let _ = write_status(&mut client, "400 Bad Request").await;
            return Ok(());
        }
    };

    let validated = match super::policy::validate_public_http_url(target) {
        Ok(validated) => validated,
        Err(_) => {
            // Record the host if we can name it; the URL is not logged.
            if let Ok(url) = url::Url::parse(target) {
                if let Some(host) = url.host_str() {
                    record_block(blocks, host.to_string());
                }
            }
            let _ = write_status(&mut client, "403 Forbidden").await;
            return Ok(());
        }
    };

    let host = validated.host().to_string();
    let port = validated.port();

    let addrs = match resolve_validated(&host, port, resolver.as_ref(), config).await {
        Ok(addrs) => addrs,
        Err(_) => {
            record_block(blocks, host);
            let _ = write_status(&mut client, "403 Forbidden").await;
            return Ok(());
        }
    };

    let mut upstream = match connect_first(&addrs).await {
        Some(upstream) => upstream,
        None => {
            let _ = write_status(&mut client, "502 Bad Gateway").await;
            return Ok(());
        }
    };

    // Rewrite the request line to origin-form and force one request per
    // connection, so the next redirect hop re-enters validation.
    let mut origin_target = validated.url().path().to_string();
    if let Some(query) = validated.url().query() {
        origin_target.push('?');
        origin_target.push_str(query);
    }
    let rewritten = rewrite_head(head, method, &origin_target, version);

    upstream.write_all(rewritten.as_bytes()).await?;
    // Forward any request body bytes already read past the head.
    let body = head.body_bytes();
    if !body.is_empty() {
        upstream.write_all(body).await?;
    }

    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    Ok(())
}

/// The buffered request head plus the offset where the body begins.
struct RequestHead {
    buf: Vec<u8>,
    head_end: usize,
}

impl RequestHead {
    /// The head as text (up to and including the terminating blank line), or
    /// `None` if it is not valid UTF-8.
    fn head_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.buf[..self.head_end]).ok()
    }

    /// The request line (first line), trimmed of its CRLF.
    fn request_line(&self) -> Option<&str> {
        self.head_str()?.split("\r\n").next()
    }

    /// Bytes already read past the head (a request body, usually empty).
    fn body_bytes(&self) -> &[u8] {
        &self.buf[self.head_end..]
    }
}

/// Read until the CRLFCRLF head terminator, bounded in size and time. Returns
/// `None` for an oversized or truncated head (mapped to 400 by the caller).
async fn read_head(client: &mut TcpStream) -> std::io::Result<Option<RequestHead>> {
    let read = timeout(HEAD_READ_TIMEOUT, async {
        let mut buf = Vec::with_capacity(1024);
        let mut chunk = [0u8; 4096];
        loop {
            let n = client.read(&mut chunk).await?;
            if n == 0 {
                // Connection closed before the head completed.
                return Ok::<Option<RequestHead>, std::io::Error>(None);
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = find_head_end(&buf) {
                return Ok(Some(RequestHead { head_end: pos, buf }));
            }
            if buf.len() > MAX_HEAD_BYTES {
                return Ok(None);
            }
        }
    })
    .await;

    match read {
        Ok(result) => result,
        // Timed out waiting for the head.
        Err(_) => Ok(None),
    }
}

/// The index just past the first `\r\n\r\n`, if present.
fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn parse_request_line(line: &str) -> Request {
    let mut parts = line.split_whitespace();
    let method = match parts.next() {
        Some(method) => method,
        None => return Request::Unsupported,
    };
    let target = match parts.next() {
        Some(target) => target,
        None => return Request::Unsupported,
    };
    let version = parts.next().unwrap_or("HTTP/1.1");

    if method.eq_ignore_ascii_case("CONNECT") {
        // authority-form host:port; rsplit keeps IPv6 bracket literals intact.
        return match target.rsplit_once(':') {
            Some((host, port)) => match port.parse::<u16>() {
                Ok(port) if !host.is_empty() => Request::Connect {
                    host: host.to_string(),
                    port,
                },
                _ => Request::Unsupported,
            },
            None => Request::Unsupported,
        };
    }

    let lower = target.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Request::AbsoluteHttp {
            method: method.to_string(),
            version: version.to_string(),
        };
    }

    Request::Unsupported
}

/// Rebuild the head in origin-form: a new request line, the original headers
/// minus `Proxy-Connection` and any `Connection`, and a forced
/// `Connection: close`.
fn rewrite_head(head: &RequestHead, method: &str, origin_target: &str, version: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!("{method} {origin_target} {version}\r\n"));

    // head_str is Some here: the caller already read the request line from it.
    if let Some(text) = head.head_str() {
        let mut lines = text.split("\r\n");
        let _request_line = lines.next();
        for line in lines {
            if line.is_empty() {
                break;
            }
            let lower = line.to_ascii_lowercase();
            if lower.starts_with("proxy-connection:") || lower.starts_with("connection:") {
                continue;
            }
            out.push_str(line);
            out.push_str("\r\n");
        }
    }

    out.push_str("Connection: close\r\n\r\n");
    out
}

async fn connect_first(addrs: &[SocketAddr]) -> Option<TcpStream> {
    for addr in addrs {
        if let Ok(Ok(stream)) = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            return Some(stream);
        }
    }
    None
}

async fn write_status(client: &mut TcpStream, status: &str) -> std::io::Result<()> {
    client
        .write_all(format!("HTTP/1.1 {status}\r\n\r\n").as_bytes())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A resolver backed by a fixed hostname to address map, so tests pin the
    /// proxy to sockets they control.
    struct MapResolver(HashMap<String, Vec<SocketAddr>>);

    impl Resolver for MapResolver {
        fn resolve<'a>(
            &'a self,
            host: &'a str,
            _port: u16,
        ) -> super::super::BoxFuture<'a, std::io::Result<Vec<SocketAddr>>> {
            let addrs = self.0.get(host).cloned().unwrap_or_default();
            Box::pin(async move { Ok(addrs) })
        }
    }

    fn resolver(pairs: &[(&str, SocketAddr)]) -> Arc<dyn Resolver> {
        let mut map: HashMap<String, Vec<SocketAddr>> = HashMap::new();
        for (host, addr) in pairs {
            map.entry((*host).to_string()).or_default().push(*addr);
        }
        Arc::new(MapResolver(map))
    }

    /// A TCP echo server; returns its address.
    async fn spawn_echo() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if sock.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
        addr
    }

    /// A one-request HTTP responder that captures the request it saw and
    /// replies with a fixed response, then closes.
    async fn spawn_responder(
        response: &'static str,
        captured: Arc<Mutex<Option<String>>>,
    ) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let captured = captured.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 1024];
                    loop {
                        match sock.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                buf.extend_from_slice(&chunk[..n]);
                                if find_head_end(&buf).is_some() {
                                    break;
                                }
                            }
                        }
                    }
                    *captured.lock().unwrap() = Some(String::from_utf8_lossy(&buf).to_string());
                    let _ = sock.write_all(response.as_bytes()).await;
                });
            }
        });
        addr
    }

    /// Read a single chunk (enough for our small status responses).
    async fn read_once(stream: &mut TcpStream) -> String {
        let mut buf = [0u8; 4096];
        let n = timeout(Duration::from_secs(5), stream.read(&mut buf))
            .await
            .expect("read timed out")
            .expect("read failed");
        String::from_utf8_lossy(&buf[..n]).to_string()
    }

    /// Read until the peer closes the connection.
    async fn read_to_eof(stream: &mut TcpStream) -> String {
        let mut out = Vec::new();
        let mut chunk = [0u8; 1024];
        loop {
            let n = timeout(Duration::from_secs(5), stream.read(&mut chunk))
                .await
                .expect("read timed out")
                .expect("read failed");
            if n == 0 {
                break;
            }
            out.extend_from_slice(&chunk[..n]);
        }
        String::from_utf8_lossy(&out).to_string()
    }

    async fn connect(proxy: &PinningProxy) -> TcpStream {
        TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn connect_to_private_address_is_blocked_and_recorded() {
        let resolver = resolver(&[("blocked.test", "10.0.0.1:443".parse().unwrap())]);
        let proxy = PinningProxy::start(resolver, PolicyConfig::default())
            .await
            .unwrap();

        let since = Instant::now();
        let mut client = connect(&proxy).await;
        client
            .write_all(b"CONNECT blocked.test:443 HTTP/1.1\r\nHost: blocked.test:443\r\n\r\n")
            .await
            .unwrap();

        let response = read_to_eof(&mut client).await;
        assert!(response.contains("403"), "expected 403, got: {response}");
        assert_eq!(proxy.blocked_since(since).as_deref(), Some("blocked.test"));
    }

    #[tokio::test]
    async fn connect_tunnel_pins_to_resolved_address_and_round_trips() {
        let echo = spawn_echo().await;
        // The resolver, not the client, decides the address: proof of pinning.
        let resolver = resolver(&[("ok.test", echo)]);
        let proxy = PinningProxy::start(
            resolver,
            PolicyConfig {
                allow_loopback: true,
            },
        )
        .await
        .unwrap();

        let mut client = connect(&proxy).await;
        client
            .write_all(b"CONNECT ok.test:1234 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();

        let status = read_once(&mut client).await;
        assert!(status.contains("200"), "expected 200, got: {status}");

        client.write_all(b"ping-through-tunnel").await.unwrap();
        let mut buf = [0u8; 32];
        let n = timeout(Duration::from_secs(5), client.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf[..n], b"ping-through-tunnel");
    }

    #[tokio::test]
    async fn plain_http_is_forwarded_in_origin_form_with_connection_close() {
        let captured = Arc::new(Mutex::new(None));
        let responder = spawn_responder(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi",
            captured.clone(),
        )
        .await;
        let resolver = resolver(&[("web.test", responder)]);
        let proxy = PinningProxy::start(
            resolver,
            PolicyConfig {
                allow_loopback: true,
            },
        )
        .await
        .unwrap();

        let mut client = connect(&proxy).await;
        client
            .write_all(
                b"GET http://web.test/path?x=1 HTTP/1.1\r\nHost: web.test\r\nProxy-Connection: keep-alive\r\n\r\n",
            )
            .await
            .unwrap();

        let response = read_to_eof(&mut client).await;
        assert!(response.contains("200 OK"), "got: {response}");
        assert!(response.ends_with("hi"), "got: {response}");

        let seen = captured
            .lock()
            .unwrap()
            .clone()
            .expect("responder saw a request");
        let request_line = seen.split("\r\n").next().unwrap();
        assert_eq!(request_line, "GET /path?x=1 HTTP/1.1");
        let lower = seen.to_ascii_lowercase();
        assert!(lower.contains("connection: close"), "got: {seen}");
        assert!(!lower.contains("proxy-connection"), "got: {seen}");
    }

    #[tokio::test]
    async fn blocked_destinations_refuse_after_a_redirect() {
        // Responder A lives on loopback (allowed by config) and 302s to a host
        // that resolves private. Following the redirect must re-validate and
        // refuse - this is the acceptance evidence for redirect re-check.
        let captured = Arc::new(Mutex::new(None));
        let responder = spawn_responder(
            "HTTP/1.1 302 Found\r\nLocation: http://blocked.test/x\r\nConnection: close\r\n\r\n",
            captured,
        )
        .await;
        let resolver = resolver(&[
            ("allowed.test", responder),
            ("blocked.test", "10.0.0.1:80".parse().unwrap()),
        ]);
        let proxy = PinningProxy::start(
            resolver,
            PolicyConfig {
                allow_loopback: true,
            },
        )
        .await
        .unwrap();

        let since = Instant::now();

        // First hop: the allowed origin answers 302.
        let mut first = connect(&proxy).await;
        first
            .write_all(b"GET http://allowed.test/ HTTP/1.1\r\nHost: allowed.test\r\n\r\n")
            .await
            .unwrap();
        let first_response = read_to_eof(&mut first).await;
        assert!(first_response.contains("302"), "got: {first_response}");
        assert!(
            first_response.contains("Location: http://blocked.test/x"),
            "got: {first_response}"
        );

        // Second hop: the browser follows the redirect through the proxy again,
        // and the now-private destination is refused.
        let mut second = connect(&proxy).await;
        second
            .write_all(b"GET http://blocked.test/x HTTP/1.1\r\nHost: blocked.test\r\n\r\n")
            .await
            .unwrap();
        let second_response = read_to_eof(&mut second).await;
        assert!(second_response.contains("403"), "got: {second_response}");
        assert_eq!(proxy.blocked_since(since).as_deref(), Some("blocked.test"));
    }

    #[tokio::test]
    async fn origin_form_request_is_rejected() {
        let resolver = resolver(&[]);
        let proxy = PinningProxy::start(resolver, PolicyConfig::default())
            .await
            .unwrap();

        let mut client = connect(&proxy).await;
        client
            .write_all(b"GET /just/a/path HTTP/1.1\r\nHost: web.test\r\n\r\n")
            .await
            .unwrap();

        let response = read_to_eof(&mut client).await;
        assert!(response.contains("400"), "expected 400, got: {response}");
    }
}
