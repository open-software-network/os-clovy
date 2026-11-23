use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::common::*;
use rand::RngCore;
use serde_json::Value;
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, Semaphore};

use super::*;
use crate::mcp_server::turns::{TurnRegistry, TURN_EXPIRED};

fn ensure_private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

/// The installation secret, created on first use (file mode 0600).
pub(crate) fn load_or_create_secret(dir: &Path) -> std::io::Result<String> {
    ensure_private_dir(dir)?;
    match read_secret(dir) {
        Ok(secret) => return Ok(secret),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => std::fs::remove_file(secret_path(dir))?,
    }
    let mut bytes = [0u8; SECRET_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let secret: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(secret_path(dir))?;
    file.write_all(secret.as_bytes())?;
    Ok(secret)
}

/// A running socket listener; dropping it without `stop` leaves the
/// accept task running.
pub struct Listener {
    socket: PathBuf,
    shutdown: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
}

impl Listener {
    /// Binds the socket and serves connections until `stop`, with no
    /// live turns (a handshake offering one gets `turn_expired`). Must
    /// run inside the Tokio runtime.
    pub async fn start(dir: &Path, handler: impl Into<Service>) -> std::io::Result<Self> {
        Self::start_with_turns(dir, handler, TurnRegistry::default()).await
    }

    /// [`Listener::start`], binding connections to the live turns of
    /// `turns`.
    pub async fn start_with_turns(
        dir: &Path,
        handler: impl Into<Service>,
        turns: TurnRegistry,
    ) -> std::io::Result<Self> {
        let service = handler.into();
        let secret: Arc<str> = load_or_create_secret(dir)?.into();
        let socket = socket_path(dir);
        if let Ok(metadata) = std::fs::symlink_metadata(&socket) {
            use std::os::unix::fs::FileTypeExt;
            if !metadata.file_type().is_socket() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", socket.display()),
                ));
            }
            // Left behind by a previous run that did not stop cleanly.
            std::fs::remove_file(&socket)?;
        }
        let listener = UnixListener::bind(&socket).map_err(|error| {
            std::io::Error::new(
                error.kind(),
                format!("could not listen on {}: {error}", socket.display()),
            )
        })?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        let (shutdown, mut stopped) = watch::channel(false);
        let connections = shutdown.subscribe();
        let handshakes = Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES));
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        let task = tokio::spawn(async move {
            let mut served = tokio::task::JoinSet::new();
            loop {
                while served.try_join_next().is_some() {}
                tokio::select! {
                    accepted = listener.accept() => {
                        let stream = match accepted {
                            Ok((stream, _)) => stream,
                            Err(error) => {
                                // Out of descriptors (EMFILE) and the like
                                // persist: do not spin on them.
                                tracing::warn!(%error, "mcp server: accept failed");
                                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                                continue;
                            }
                        };
                        // Too many peers that have not authenticated yet:
                        // close this one now instead of holding it open.
                        let Ok(handshake) = Arc::clone(&handshakes).try_acquire_owned() else {
                            continue;
                        };
                        let authorized = same_user(&stream);
                        served.spawn(serve(
                            stream,
                            Authentication { same_user: authorized, secret: Arc::clone(&secret) },
                            handshake,
                            Arc::clone(&slots),
                            turns.clone(),
                            service.clone(),
                            connections.clone(),
                        ));
                    }
                    _ = stopped.changed() => break,
                }
            }
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

    /// Removes the socket file, then stops accepting and closes every open
    /// connection. Removing first means a relay whose connection drops can
    /// already tell that the server is off.
    pub fn stop(self) {
        let _ = std::fs::remove_file(&self.socket);
        let _ = self.shutdown.send(true);
    }
    pub async fn stop_and_wait(self) {
        let _ = std::fs::remove_file(&self.socket);
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
    }
    pub fn is_running(&self) -> bool {
        !*self.shutdown.borrow() && !self.task.is_finished()
    }
}

fn same_user(stream: &UnixStream) -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail.
    let own = unsafe { libc::geteuid() };
    stream.peer_cred().is_ok_and(|peer| peer.uid() == own)
}

/// Connects and authenticates (the relay side, blocking).
pub fn connect(dir: &Path) -> Result<std::os::unix::net::UnixStream, ClientError> {
    connect_turn(dir, None)
}

/// [`connect`], bound to the CLI turn of `turn` when given.
pub fn connect_turn(
    dir: &Path,
    turn: Option<&str>,
) -> Result<std::os::unix::net::UnixStream, ClientError> {
    let secret = match read_secret(dir) {
        Ok(secret) => secret,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(ClientError::NeverEnabled)
        }
        Err(error) => return Err(ClientError::Other(error)),
    };
    let stream = std::os::unix::net::UnixStream::connect(socket_path(dir)).map_err(|error| {
        match error.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                ClientError::NotListening
            }
            _ => ClientError::Other(error),
        }
    })?;
    stream
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(ClientError::Other)?;
    // The app closes a peer it has no handshake slot for.
    let closed = |error: std::io::Error| match error.kind() {
        std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset => ClientError::Busy,
        _ => ClientError::Other(error),
    };
    (&stream)
        .write_all(format!("{}\n", turn_handshake(&secret, turn)).as_bytes())
        .map_err(closed)?;
    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .map_err(closed)?;
    if reply.is_empty() {
        return Err(ClientError::Busy);
    }
    let reply = serde_json::from_str::<Value>(&reply).unwrap_or(Value::Null);
    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(match reply.get("error").and_then(Value::as_str) {
            Some("busy") => ClientError::Busy,
            Some(TURN_EXPIRED) => ClientError::TurnExpired,
            _ => ClientError::Refused,
        });
    }
    stream.set_read_timeout(None).map_err(ClientError::Other)?;
    Ok(stream)
}
pub type SyncStream = std::os::unix::net::UnixStream;

pub fn endpoint_available(dir: &Path) -> bool {
    super::socket_path(dir).exists()
}
