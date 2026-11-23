//! Local byte-mode overlapped named pipes. Tokio/Mio owns native pending
//! OVERLAPPED operations and buffers until completion/cancellation; no blocking
//! ReadFile hidden behind a timeout, and no FlushFileBuffers on close.
use super::common::*;
use super::*;
use crate::mcp_server::turns::TurnRegistry;
use ::windows::Win32::Foundation::HANDLE;
use ::windows::Win32::System::Pipes::{
    GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, WaitNamedPipeW,
};
use ::windows::Win32::System::IO::CancelIoEx;
use std::io::{self, Read, Write};
use std::os::windows::io::AsRawHandle;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use tokio::sync::{watch, Semaphore};
#[path = "windows_security.rs"]
pub(crate) mod security;

/// Mio intentionally leaves writes pending on ordinary pipe Drop. Explicitly
/// cancel *all* I/O first. Mio owns an Arc for each completion, retaining its
/// OVERLAPPED and buffer even after this wrapper and the Tokio stream drop.
struct Pipe<T: AsRawHandle>(T);
impl<T: AsRawHandle> Drop for Pipe<T> {
    fn drop(&mut self) {
        unsafe {
            let _ = CancelIoEx(HANDLE(self.0.as_raw_handle()), None);
        }
    }
}
impl<T: AsRawHandle + tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Pipe<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        std::pin::Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
    }
}
impl<T: AsRawHandle + tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Pipe<T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        std::pin::Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        // Tokio's named-pipe flush is a no-op. Mio accepts a new write only
        // after the previous native write completes. A zero-byte write is a
        // completion barrier without FlushFileBuffers or a blocking syscall.
        std::pin::Pin::new(&mut self.get_mut().0)
            .poll_write(cx, &[])
            .map_ok(|_| ())
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        self.poll_flush(cx)
    }
}

// One bounded I/O driver for the relay process, shared by reconnects. It also
// outlives the last cancelled native completion, which may arrive after the
// synchronous read/write facade has returned. No runtime is torn down while
// Mio's kernel-owned buffers are awaiting a completion packet.
fn runtime() -> io::Result<&'static tokio::runtime::Runtime> {
    static DRIVER: std::sync::OnceLock<io::Result<tokio::runtime::Runtime>> =
        std::sync::OnceLock::new();
    DRIVER
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
        })
        .as_ref()
        .map_err(|error| io::Error::new(error.kind(), "MCP I/O driver could not start"))
}

fn endpoint(dir: &Path) -> io::Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::os::windows::ffi::OsStrExt;
    let _held = security::directory(dir, false)?;
    let canonical = dunce::canonicalize(dir)?;
    let mut digest = Sha256::new();
    for unit in canonical.as_os_str().encode_wide() {
        digest.update(unit.to_le_bytes());
    }
    Ok(PathBuf::from(format!(
        r"\\.\pipe\clovy-mcp-{:x}",
        digest.finalize()
    )))
}
pub fn endpoint_available(dir: &Path) -> bool {
    let Ok(endpoint) = endpoint(dir).and_then(|p| security::wide(p.as_os_str())) else {
        return false;
    };
    // WaitNamedPipe checks availability without connecting or taking a slot.
    if unsafe { WaitNamedPipeW(::windows::core::PCWSTR(endpoint.as_ptr()), 1) }.as_bool() {
        return true;
    }
    matches!(io::Error::last_os_error().raw_os_error(), Some(121 | 231))
}
fn instance(name: &Path, first: bool) -> io::Result<NamedPipeServer> {
    let security = security::Security::new()?;
    unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(
                name,
                (&security.attributes as *const ::windows::Win32::Security::SECURITY_ATTRIBUTES)
                    .cast_mut()
                    .cast(),
            )
    }
}
pub struct Listener {
    socket: PathBuf,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}
impl Listener {
    pub async fn start(dir: &Path, handler: impl Into<Service>) -> io::Result<Self> {
        Self::start_with_turns(dir, handler, TurnRegistry::default()).await
    }
    pub async fn start_with_turns(
        dir: &Path,
        handler: impl Into<Service>,
        turns: TurnRegistry,
    ) -> io::Result<Self> {
        let service = handler.into();
        let secret: Arc<str> = security::secret(dir, true)?.into();
        let socket = endpoint(dir)?;
        // Cancelled IOCP operations can briefly retain the previous instance
        // after stop_and_wait has joined its tasks. Every attempt still uses
        // FIRST_PIPE_INSTANCE; an occupied/squatted name never gets adopted.
        let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
        let mut listener = loop {
            match instance(&socket, true) {
                Ok(listener) => break listener,
                Err(error)
                    if error.raw_os_error() == Some(5)
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error),
            }
        };
        let name = socket.clone();
        let (shutdown, mut stopped) = watch::channel(false);
        let connections = shutdown.subscribe();
        let failed = shutdown.clone();
        let handshakes = Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES));
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let task = tokio::spawn(async move {
            let mut served = tokio::task::JoinSet::new();
            loop {
                while served.try_join_next().is_some() {}
                tokio::select! {
                    biased;
                    _ = stopped.changed() => break,
                    accepted = listener.connect() => {
                        if let Err(error) = accepted {
                            tracing::warn!(%error, "mcp server: pipe accept failed");
                            tokio::select! { _ = stopped.changed() => break, _ = tokio::time::sleep(ACCEPT_ERROR_BACKOFF) => {} }
                            continue;
                        }
                        // Keep the old instance alive while creating the next:
                        // there is never an unowned name between clients.
                        let next = match instance(&name, false) {
                            Ok(next) => next,
                            Err(error) => {
                                tracing::warn!(%error, "mcp server: pipe renewal failed");
                                break;
                            }
                        };
                        let stream = std::mem::replace(&mut listener, next);
                        let Ok(handshake) = handshakes.clone().try_acquire_owned() else { continue; };
                        let peer = unsafe {
                            let mut pid = 0;
                            GetNamedPipeClientProcessId(HANDLE(stream.as_raw_handle()), &mut pid)
                                .map_err(security::win).and_then(|()| security::same_user(pid))
                        };
                        // Unknown/denied peers fail before reading credentials.
                        if peer.is_err() { continue; }
                        drop(peer);
                        served.spawn(serve(Pipe(stream), Authentication { same_user: true, secret: secret.clone() }, handshake, slots.clone(), turns.clone(), service.clone(), connections.clone()));
                    }
                }
            }
            // An accept/renewal failure must close all existing connections.
            let _ = failed.send(true);
            drop(listener);
            while served.join_next().await.is_some() {}
        });
        Ok(Self {
            socket,
            shutdown,
            task,
        })
    }
    pub fn socket(&self) -> &Path {
        &self.socket
    }
    pub fn stop(self) {
        let _ = self.shutdown.send(true);
    }
    pub async fn stop_and_wait(self) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
    }
    pub fn is_running(&self) -> bool {
        !*self.shutdown.borrow() && !self.task.is_finished()
    }
}

struct Io {
    runtime: tokio::runtime::Handle,
    handle: usize,
    read: tokio::sync::Mutex<Option<ReadHalf<Pipe<NamedPipeClient>>>>,
    write: tokio::sync::Mutex<Option<WriteHalf<Pipe<NamedPipeClient>>>>,
    closed: watch::Sender<bool>,
}
impl Drop for Io {
    fn drop(&mut self) {
        self.closed.send_replace(true);
        // Drop both halves while the I/O driver still lives. Mio retains
        // pending OVERLAPPED storage until its completion packets are handled.
        drop(self.read.get_mut().take());
        drop(self.write.get_mut().take());
    }
}
/// Small synchronous facade used by the shared stdio relay. The dedicated
/// process runtime owns the driver throughout both reader and writer lifetimes.
pub struct SyncStream {
    io: Arc<Io>,
    read_timeout: Mutex<Option<Duration>>,
    write_timeout: Mutex<Option<Duration>>,
}
impl SyncStream {
    pub(super) fn open_raw(dir: &Path) -> io::Result<Self> {
        let name = endpoint(dir)?;
        if !name.to_string_lossy().starts_with(r"\\.\pipe\clovy-mcp-") {
            return Err(security::denied());
        }
        let runtime = runtime()?;
        let (handle, read, write) = runtime.block_on(async {
            // Identification QOS prevents a pipe squatter impersonating a CLI.
            let pipe = ClientOptions::new().open(&name)?;
            let mut pid = 0;
            unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }
                .map_err(security::win)?;
            let _peer = security::same_user(pid)?;
            let handle = pipe.as_raw_handle() as usize;
            let (read, write) = tokio::io::split(Pipe(pipe));
            Ok::<_, io::Error>((handle, read, write))
        })?;
        Ok(Self {
            io: Arc::new(Io {
                runtime: runtime.handle().clone(),
                handle,
                read: tokio::sync::Mutex::new(Some(read)),
                write: tokio::sync::Mutex::new(Some(write)),
                closed: watch::channel(false).0,
            }),
            read_timeout: Mutex::new(None),
            write_timeout: Mutex::new(None),
        })
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            io: self.io.clone(),
            read_timeout: Mutex::new(*self.read_timeout.lock().unwrap()),
            write_timeout: Mutex::new(*self.write_timeout.lock().unwrap()),
        })
    }
    pub fn set_read_timeout(&self, duration: Option<Duration>) -> io::Result<()> {
        *self.read_timeout.lock().unwrap() = duration;
        Ok(())
    }
    pub fn set_write_timeout(&self, duration: Option<Duration>) -> io::Result<()> {
        *self.write_timeout.lock().unwrap() = duration;
        Ok(())
    }
    pub fn shutdown(&self, _how: std::net::Shutdown) -> io::Result<()> {
        self.io.closed.send_replace(true);
        // Join access to both halves after waking their cancellation branches.
        // No operation can register new native I/O after this cancellation.
        self.io.runtime.block_on(async {
            let _read = self.io.read.lock().await;
            let _write = self.io.write.lock().await;
            // SAFETY: this Arc retains both pipe halves and their handle.
            // Mio retains each native buffer/OVERLAPPED until its IOCP packet.
            unsafe {
                let _ = CancelIoEx(HANDLE(self.io.handle as *mut _), None);
            }
        });
        Ok(())
    }
    fn blocking<T>(
        &self,
        deadline: Option<Duration>,
        operation: impl std::future::Future<Output = io::Result<T>>,
    ) -> io::Result<T> {
        let mut stopped = self.io.closed.subscribe();
        self.io.runtime.block_on(async {
            if *stopped.borrow() { return Err(io::Error::from(io::ErrorKind::BrokenPipe)); }
            let timed = async {
                match deadline {
                    Some(duration) => tokio::time::timeout(duration, operation).await.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?,
                    None => operation.await,
                }
            };
            tokio::select! { biased; _ = stopped.changed() => Err(io::Error::from(io::ErrorKind::BrokenPipe)), result = timed => result }
        })
    }
}
impl Read for SyncStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.blocking(*self.read_timeout.lock().unwrap(), async {
            self.io
                .read
                .lock()
                .await
                .as_mut()
                .expect("live stream")
                .read(buffer)
                .await
        })
    }
}
impl Write for SyncStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.blocking(*self.write_timeout.lock().unwrap(), async {
            self.io
                .write
                .lock()
                .await
                .as_mut()
                .expect("live stream")
                .write(buffer)
                .await
        })
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn connect(dir: &Path) -> Result<SyncStream, ClientError> {
    connect_turn(dir, None)
}
pub fn connect_turn(dir: &Path, turn: Option<&str>) -> Result<SyncStream, ClientError> {
    let secret = security::secret(dir, false).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            ClientError::NeverEnabled
        } else {
            ClientError::Other(error)
        }
    })?;
    let stream = SyncStream::open_raw(dir).map_err(|error| match error.raw_os_error() {
        Some(2 | 3) => ClientError::NotListening,
        Some(231) => ClientError::Busy,
        _ => ClientError::Other(error),
    })?;
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(ClientError::Other)?;
    stream
        .set_write_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(ClientError::Other)?;
    // One deadline for the entire exchange, including a peer dripping bytes.
    let reply = stream
        .io
        .runtime
        .block_on(async {
            tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
                let mut writer = stream.io.write.lock().await;
                writer
                    .as_mut()
                    .expect("live stream")
                    .write_all(format!("{}\n", turn_handshake(&secret, turn)).as_bytes())
                    .await?;
                drop(writer);
                let mut read = stream.io.read.lock().await;
                let reader = tokio::io::BufReader::new(read.as_mut().expect("live stream"));
                let mut reply = String::new();
                tokio::io::AsyncBufReadExt::read_line(&mut reader.take(4096), &mut reply).await?;
                Ok::<_, io::Error>(reply)
            })
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
        })
        .map_err(ClientError::Other)?;
    if reply.is_empty() {
        return Err(ClientError::Busy);
    }
    let reply: Value = serde_json::from_str(&reply).unwrap_or(Value::Null);
    if reply.get("clovyMcp").and_then(Value::as_u64) != Some(HANDSHAKE_VERSION)
        || reply.get("ok").and_then(Value::as_bool) != Some(true)
    {
        return Err(match reply.get("error").and_then(Value::as_str) {
            Some("busy") => ClientError::Busy,
            Some("turn_expired") => ClientError::TurnExpired,
            _ => ClientError::Refused,
        });
    }
    stream.set_read_timeout(None).map_err(ClientError::Other)?;
    stream.set_write_timeout(None).map_err(ClientError::Other)?;
    Ok(stream)
}
pub(super) fn read_secret(dir: &Path) -> io::Result<String> {
    security::secret(dir, false)
}
#[cfg(test)]
pub(super) fn test_stream(dir: &Path) -> NamedPipeClient {
    ClientOptions::new().open(endpoint(dir).unwrap()).unwrap()
}
#[cfg(test)]
pub(super) fn security_test_secret(dir: &Path) -> io::Result<String> {
    security::secret(dir, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn handshake_deadline_is_total_even_when_server_drips_bytes() {
        let root = tempfile::tempdir().unwrap();
        let dir = channel_dir(root.path());
        security::secret(&dir, true).unwrap();
        let server = instance(&endpoint(&dir).unwrap(), true).unwrap();
        security::test_pipe_acl(HANDLE(server.as_raw_handle())).unwrap();
        let drip = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut server = Pipe(server);
            let mut hello = [0; 1024];
            assert!(server.read(&mut hello).await.unwrap() > 0);
            for _ in 0..14 {
                if server.write_all(b" ").await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(450)).await;
            }
        });
        tokio::task::spawn_blocking(move || {
            let start = std::time::Instant::now();
            assert!(matches!(connect(&dir), Err(ClientError::Other(error)) if error.kind() == io::ErrorKind::TimedOut));
            assert!(start.elapsed() < Duration::from_secs(6));
        }).await.unwrap();
        drip.abort();
        let _ = drip.await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn partial_native_write_is_unknown_and_never_reconnects_to_replay() {
        use crate::mcp_server::channel::transport_tests::{Input, Output};
        let root = tempfile::tempdir().unwrap();
        let dir = channel_dir(root.path());
        security::secret(&dir, true).unwrap();
        let name = endpoint(&dir).unwrap();
        let server = instance(&name, true).unwrap();
        let fault = tokio::spawn(async move {
            server.connect().await.unwrap();
            let mut stream = tokio::io::BufReader::new(Pipe(server));
            let mut hello = String::new();
            tokio::io::AsyncBufReadExt::read_line(&mut stream, &mut hello)
                .await
                .unwrap();
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
            let next = instance(&name, false).unwrap();
            drop(stream);
            assert!(
                tokio::time::timeout(Duration::from_millis(400), next.connect())
                    .await
                    .is_err(),
                "tool request was replayed"
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
            crate::mcp_server::stdio::relay(
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
}
