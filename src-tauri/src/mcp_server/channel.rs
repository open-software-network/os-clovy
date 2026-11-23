//! The local channel between `clovy-mcp` (spawned by an MCP client) and the
//! running app: a Unix socket (0700 directory, 0600 socket/secret), or a local
//! Windows named pipe with protected user/SYSTEM DACL and a private secret.
//! The binary never opens Clovy's databases; it relays JSON-RPC lines here.
//!
//! Wire format: newline-delimited JSON. The client's first line is the
//! handshake `{"clovyMcp": 1, "secret": "<hex>"}`, plus `"turn": "<token>"`
//! when a Clovy CLI turn spawned the relay (additive: the version stays 1,
//! and a handshake without `turn` is an unbound connection). The app answers
//! `{"clovyMcp": 1, "ok": true}`, or `{"clovyMcp": 1, "ok": false, "error":
//! "unauthorized"}` (or `"busy"` when every connection slot is taken, or
//! `"turn_expired"` when the turn token is unknown or revoked) and closes. A
//! bound connection is closed when its turn is revoked (`super::turns`),
//! after the calls it was serving answer (at most 2 s). After `ok`, each line is one MCP JSON-RPC message in either
//! direction; responses may arrive out of order.
//!
//! Limits (every process of the same user can reach the socket): at most
//! `MAX_PENDING_HANDSHAKES` connections waiting for their handshake (more are
//! closed as soon as they are accepted), `MAX_CONNECTIONS` authenticated
//! connections, and `MAX_IN_FLIGHT_PER_CONNECTION` requests being answered
//! per connection (the app stops reading that connection until one finishes).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

pub const DIR_NAME: &str = "mcp";
pub const SOCKET_FILE: &str = "clovy.sock";
pub const SECRET_FILE: &str = "secret";
pub const HANDSHAKE_VERSION: u64 = 1;
/// A relay whose stdin closes can revoke its external authority while it
/// drains responses on transports without a write-only shutdown.
pub(super) const RELAY_EOF: &str = "notifications/clovy/relayEof";
/// Longest accepted line, in bytes (a JSON-RPC message).
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;
/// Accepted connections still in the handshake at once.
pub const MAX_PENDING_HANDSHAKES: usize = 4;
/// Authenticated connections at once (each MCP client holds one).
pub const MAX_CONNECTIONS: usize = 8;
/// Requests of one connection being answered at once.
pub const MAX_IN_FLIGHT_PER_CONNECTION: usize = 4;
const SECRET_BYTES: usize = 32;

/// `<app data dir>/mcp`, the directory the configuration snippets name.
pub fn channel_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(DIR_NAME)
}

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join(SOCKET_FILE)
}

pub fn secret_path(dir: &Path) -> PathBuf {
    dir.join(SECRET_FILE)
}

pub fn handshake(secret: &str) -> Value {
    json!({ "clovyMcp": HANDSHAKE_VERSION, "secret": secret })
}

/// The handshake of a relay spawned for a CLI turn (`turn: None` is
/// [`handshake`]).
pub fn turn_handshake(secret: &str, turn: Option<&str>) -> Value {
    let mut hello = handshake(secret);
    if let Some(turn) = turn {
        hello["turn"] = json!(turn);
    }
    hello
}

fn valid_secret(secret: &str) -> bool {
    secret.len() == SECRET_BYTES * 2 && secret.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Equal-time comparison, so a wrong guess learns nothing from timing.
fn secrets_match(expected: &str, offered: &str) -> bool {
    expected.len() == offered.len()
        && expected
            .bytes()
            .zip(offered.bytes())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// Reads the installation secret (the relay side).
pub fn read_secret(dir: &Path) -> std::io::Result<String> {
    #[cfg(windows)]
    return windows::read_secret(dir);
    #[cfg(not(windows))]
    {
        let secret = std::fs::read_to_string(secret_path(dir))?
            .trim()
            .to_string();
        if valid_secret(&secret) {
            Ok(secret)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "the Clovy MCP secret file is malformed",
            ))
        }
    }
}

#[cfg(any(unix, windows))]
#[path = "channel/common.rs"]
mod common;
#[cfg(any(unix, windows))]
pub use common::{
    AuthenticatedHandler, ClientError, Connection, ExternalConnection, Handler, Service,
    TurnHandler,
};
#[cfg(unix)]
#[path = "channel/unix.rs"]
mod unix;
#[cfg(unix)]
pub use unix::{connect, connect_turn, endpoint_available, Listener, SyncStream};
#[cfg(windows)]
#[path = "channel/windows.rs"]
pub(crate) mod windows;
#[cfg(windows)]
pub use windows::{connect, connect_turn, endpoint_available, Listener, SyncStream};
#[cfg(test)]
#[path = "transport_tests.rs"]
mod transport_tests;

#[cfg(all(test, unix))]
pub(crate) type TestStream = tokio::net::UnixStream;
#[cfg(all(test, windows))]
pub(crate) type TestStream = tokio::net::windows::named_pipe::NamedPipeClient;
#[cfg(test)]
pub(crate) async fn test_stream(dir: &Path) -> TestStream {
    #[cfg(unix)]
    {
        tokio::net::UnixStream::connect(socket_path(dir))
            .await
            .unwrap()
    }
    #[cfg(windows)]
    {
        windows::test_stream(dir)
    }
}
