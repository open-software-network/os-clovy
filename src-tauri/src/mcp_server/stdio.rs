//! `clovy-mcp`: the stdio MCP server an MCP client spawns. It relays each
//! JSON-RPC line to the running app over the authenticated channel
//! (`super::channel`) and copies the app's answers to stdout. While the app
//! cannot be reached (closed, server turned off, wrong secret) it answers by
//! itself: `initialize` and `ping` succeed so the client can show the reason,
//! tool calls return an error result saying why, and other requests fail with
//! the same message.
//!
//! Usage: `clovy-mcp --dir "<app data dir>/mcp"` (the configuration snippets
//! in Settings, MCP carry the right directory). A Clovy CLI chat turn adds
//! `--turn <token>`: the connection is then bound to that turn
//! (`super::turns`), and once the turn ends every call answers that it has.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

use super::protocol;

pub const OFF_MESSAGE: &str = "The Clovy MCP server is off or Clovy is not open. Open Clovy and turn on the MCP server in Settings, MCP.";
pub const NEVER_ENABLED_MESSAGE: &str =
    "The Clovy MCP server has not been turned on yet. Open Clovy and turn it on in Settings, MCP.";
pub const REFUSED_MESSAGE: &str = "Clovy refused the connection because the installation secret did not match. Copy the configuration again from Clovy Settings, MCP.";
pub const BUSY_MESSAGE: &str = "Clovy is already serving as many MCP connections as it allows. Close another MCP client, then try again.";
pub const DROPPED_MESSAGE: &str =
    "Clovy closed the connection before answering (the MCP server was turned off or Clovy quit).";
pub const PRODUCTION_DATA_MESSAGE: &str = "This development build of clovy-mcp does not connect to the installed Clovy's data. Copy the configuration from the development app, or set OS_CLOVY_USE_PROD_DATA_DIR=1 to allow it.";
pub const TOO_LONG_MESSAGE: &str = "The message is longer than 4 MiB.";
pub const TURN_EXPIRED_MESSAGE: &str =
    "This Clovy chat turn has ended, so its Clovy tools no longer answer (turn_expired).";

/// The `--dir` argument.
pub fn parse_dir(args: &[String]) -> Result<PathBuf, String> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--dir=") {
            return Ok(PathBuf::from(value));
        }
        if arg == "--dir" {
            return args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--dir needs a directory".to_string());
        }
    }
    Err("usage: clovy-mcp --dir <Clovy data directory>/mcp (copy the configuration from Clovy Settings, MCP)".to_string())
}

/// The `--turn` argument, when given.
pub fn parse_turn(args: &[String]) -> Result<Option<String>, String> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = match arg.strip_prefix("--turn=") {
            Some(value) => Some(value.to_string()),
            None if arg == "--turn" => args.next().cloned(),
            None => continue,
        };
        return value
            .filter(|token| !token.is_empty())
            .map(Some)
            .ok_or_else(|| "--turn needs a token".to_string());
    }
    Ok(None)
}

/// The local answer to `message` while the app is unreachable for `reason`.
pub fn fallback(message: &Value, reason: &str) -> Option<Value> {
    let id = match protocol::classify(message) {
        protocol::Incoming::Request { id, .. } => id,
        protocol::Incoming::Invalid { id, reason } => {
            return Some(protocol::error_response(
                id,
                protocol::INVALID_REQUEST,
                reason,
            ))
        }
        protocol::Incoming::Notification | protocol::Incoming::Response => return None,
    };
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    Some(match protocol::method(message).unwrap_or_default() {
        "initialize" => protocol::result_response(
            id,
            protocol::initialize_result(&params, &format!("Clovy is not reachable: {reason}")),
        ),
        "ping" => protocol::result_response(id, json!({})),
        "tools/call" => protocol::result_response(id, protocol::tool_error(reason)),
        _ => protocol::error_response(id, protocol::SERVER_UNAVAILABLE, reason),
    })
}

/// Debug builds keep away from the installed app's data, like the debug app
/// itself (`crate::app_paths`): a channel directory under the production
/// data directory is refused unless `OS_CLOVY_USE_PROD_DATA_DIR` is set.
pub fn production_data_refusal(dir: &std::path::Path) -> Option<&'static str> {
    if !cfg!(debug_assertions) || crate::app_paths::use_prod_data_dir() {
        return None;
    }
    let Some(production) = crate::app_paths::production_app_data_dir() else {
        return cfg!(windows).then_some(PRODUCTION_DATA_MESSAGE);
    };
    production_directory_refusal(dir, &production)
}

fn production_directory_refusal(
    dir: &std::path::Path,
    production: &std::path::Path,
) -> Option<&'static str> {
    let resolve =
        |path: &std::path::Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    resolve(dir)
        .starts_with(resolve(production))
        .then_some(PRODUCTION_DATA_MESSAGE)
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn synthetic_release_root_is_refused_and_sibling_dev_root_is_allowed() {
        let root = tempfile::tempdir().unwrap();
        let production = root.path().join("synthetic-installation");
        let channel = production.join("mcp");
        let dev = root.path().join("synthetic-installation-dev/mcp");
        std::fs::create_dir_all(&channel).unwrap();
        std::fs::create_dir_all(&dev).unwrap();
        assert_eq!(
            production_directory_refusal(&channel, &production),
            Some(PRODUCTION_DATA_MESSAGE)
        );
        assert_eq!(production_directory_refusal(&dev, &production), None);
        #[cfg(unix)]
        {
            let alias = root.path().join("alias");
            std::os::unix::fs::symlink(&production, &alias).unwrap();
            assert_eq!(
                production_directory_refusal(&alias.join("mcp"), &production),
                Some(PRODUCTION_DATA_MESSAGE)
            );
        }
    }
}

/// One input line.
pub enum InputLine {
    Line(Vec<u8>),
    /// Longer than the limit: skipped to its end without being kept.
    TooLong,
}

/// Reads one line of at most `max` bytes (newline excluded). A longer line
/// is consumed to its newline while keeping nothing, so a client cannot make
/// the relay buffer an unbounded line.
pub fn read_bounded_line(
    input: &mut impl BufRead,
    max: usize,
) -> std::io::Result<Option<InputLine>> {
    let mut line = Vec::new();
    let mut too_long = false;
    let mut read_any = false;
    loop {
        let available = match input.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            break;
        }
        read_any = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let content = newline.unwrap_or(available.len());
        if !too_long {
            if line.len() + content > max {
                too_long = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(&available[..content]);
            }
        }
        input.consume(newline.map_or(content, |index| index + 1));
        if newline.is_some() {
            break;
        }
    }
    let read = if too_long {
        InputLine::TooLong
    } else {
        InputLine::Line(line)
    };
    Ok(read_any.then_some(read))
}

/// Entry point for the `clovy-mcp` binary.
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dir, turn) = match parse_dir(&args).and_then(|dir| Ok((dir, parse_turn(&args)?))) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("clovy-mcp: {message}");
            return 2;
        }
    };
    relay_turn(
        &dir,
        turn.as_deref(),
        std::io::stdin().lock(),
        Box::new(std::io::stdout()),
    );
    0
}

/// Relays `input` lines until end of input, for no turn.
pub fn relay(dir: &std::path::Path, input: impl BufRead, output: Box<dyn Write + Send>) {
    relay_turn(dir, None, input, output);
}

#[cfg(not(any(unix, windows)))]
pub fn relay_turn(
    _dir: &std::path::Path,
    _turn: Option<&str>,
    input: impl BufRead,
    output: Box<dyn Write + Send>,
) {
    let mut output = output;
    for line in input.lines().map_while(Result::ok) {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if let Some(response) =
            fallback(&message, "The Clovy MCP server requires macOS or Windows.")
        {
            let _ = writeln!(output, "{response}");
            let _ = output.flush();
        }
    }
}

#[cfg(any(unix, windows))]
pub use local::relay_turn;
#[cfg(all(test, unix))]
pub(crate) use local::{reader_exit_pause, relay_with_reader_pause};

#[cfg(any(unix, windows))]
mod local {
    use std::collections::HashMap;
    use std::io::{BufRead, BufReader, Write};
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use serde_json::Value;

    use super::*;
    use crate::mcp_server::channel::{self, ClientError};

    type Output = Arc<Mutex<Box<dyn Write + Send>>>;

    /// A first connection's EOF pause, owned by one test relay invocation.
    /// Production and Windows builds have no hook or shared test state.
    #[cfg(all(test, unix))]
    pub(crate) struct ReaderExitPause {
        wait: std::sync::mpsc::Receiver<()>,
        release: std::sync::mpsc::Sender<()>,
    }

    #[cfg(all(test, unix))]
    pub(crate) fn reader_exit_pause() -> (ReaderExitPause, std::sync::mpsc::Sender<()>) {
        let (release, wait) = std::sync::mpsc::channel();
        (
            ReaderExitPause {
                wait,
                release: release.clone(),
            },
            release,
        )
    }

    fn emit(output: &Output, message: &Value) {
        let mut output = output.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writeln!(output, "{message}");
        let _ = output.flush();
    }

    fn dropped_reason(dir: &Path) -> &'static str {
        if channel::endpoint_available(dir) {
            DROPPED_MESSAGE
        } else {
            OFF_MESSAGE
        }
    }

    // Once writing began, EOF/errors cannot prove zero delivery. Preserve a
    // real receipt when present; otherwise retain correlation and uncertainty.
    fn uncertain(message: &Value, reason: &str) -> Option<Value> {
        if protocol::method(message) != Some("tools/call") {
            return fallback(message, reason);
        }
        let id = protocol::request_id(message)?;
        let mut error = crate::domain::types::AppError::new("mcp_delivery_unknown", reason);
        error.details = Some(
            json!({"delivery":"unknown","effect":"unknown","retrySafe":false,"callId":id.clone()}),
        );
        Some(protocol::result_response(
            id,
            protocol::tool_error_for(&error),
        ))
    }

    /// An authenticated connection and the requests still waiting on it.
    struct Connection {
        stream: channel::SyncStream,
        external: bool,
        reader: Option<std::thread::JoinHandle<()>>,
        alive: Arc<AtomicBool>,
        pending: Arc<Mutex<HashMap<String, Value>>>,
        #[cfg(all(test, unix))]
        reader_release: Option<std::sync::mpsc::Sender<()>>,
    }

    impl Connection {
        fn open(
            dir: &Path,
            turn: Option<&str>,
            output: &Output,
            #[cfg(all(test, unix))] pause: Option<ReaderExitPause>,
        ) -> Result<Self, ClientError> {
            let stream = channel::connect_turn(dir, turn)?;
            let reader = stream.try_clone().map_err(ClientError::Other)?;
            let alive = Arc::new(AtomicBool::new(true));
            let pending: Arc<Mutex<HashMap<String, Value>>> = Arc::default();
            let (thread_alive, thread_pending, thread_output) =
                (Arc::clone(&alive), Arc::clone(&pending), Arc::clone(output));
            let probe_dir = dir.to_path_buf();
            #[cfg(all(test, unix))]
            let (reader_wait, reader_release) = match pause {
                Some(pause) => (Some(pause.wait), Some(pause.release)),
                None => (None, None),
            };
            let reader = std::thread::spawn(move || {
                let mut reader = BufReader::new(reader);
                while let Ok(Some(InputLine::Line(line))) =
                    read_bounded_line(&mut reader, channel::MAX_LINE_BYTES)
                {
                    let Ok(message) = serde_json::from_slice::<Value>(&line) else {
                        continue;
                    };
                    if let Some(id) = message.get("id") {
                        thread_pending
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .remove(&id.to_string());
                    }
                    emit(&thread_output, &message);
                }
                #[cfg(all(test, unix))]
                if let Some(wait) = reader_wait {
                    let _ = wait.recv();
                }
                let orphans: Vec<Value> = {
                    let mut pending = thread_pending
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    thread_alive.store(false, Ordering::SeqCst);
                    pending.drain().map(|(_, message)| message).collect()
                };
                if !orphans.is_empty() {
                    let reason = dropped_reason(&probe_dir);
                    for message in orphans {
                        if let Some(response) = uncertain(&message, reason) {
                            emit(&thread_output, &response);
                        }
                    }
                }
            });
            Ok(Self {
                stream,
                external: turn.is_none(),
                reader: Some(reader),
                alive,
                pending,
                #[cfg(all(test, unix))]
                reader_release,
            })
        }

        fn send(&mut self, line: &str, message: &Value) -> bool {
            // Register while holding the same lock used for EOF draining.
            // No request may be added after the reader has drained the map.
            {
                let mut pending = self.pending.lock().unwrap_or_else(PoisonError::into_inner);
                if !self.alive.load(Ordering::SeqCst) {
                    drop(pending);
                    // The caller only reconnects before starting a write.
                    return false;
                }
                if protocol::method(message) == Some("notifications/cancelled")
                    && matches!(
                        protocol::classify(message),
                        protocol::Incoming::Notification
                    )
                {
                    if let Some(id) = message.pointer("/params/requestId") {
                        pending.remove(&id.to_string());
                    }
                }
                if let Some(id) = protocol::request_id(message) {
                    pending.insert(id.to_string(), message.clone());
                }
            }
            if self
                .stream
                .write_all(line.as_bytes())
                .and_then(|()| self.stream.write_all(b"\n"))
                .is_err()
            {
                // Never replay after write_all: even its error may mean a
                // prefix reached Clovy. The reader emits unknown for orphans.
                // Only a later independent request may open a new connection,
                // even if the reader has not processed EOF yet.
                self.alive.store(false, Ordering::SeqCst);
                let _ = self.stream.shutdown(std::net::Shutdown::Both);
            }
            true
        }

        fn finish(&mut self) {
            if self.external {
                let _ = self
                    .stream
                    .set_write_timeout(Some(std::time::Duration::from_millis(250)));
                let message = json!({"jsonrpc": "2.0", "method": channel::RELAY_EOF});
                let _ = writeln!(self.stream, "{message}");
            }
            #[cfg(unix)]
            let _ = self.stream.shutdown(std::net::Shutdown::Write);
            // stdin EOF allows a bounded drain of receipts before closing IO.
            let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while self
                .reader
                .as_ref()
                .is_some_and(|reader| !reader.is_finished())
                && std::time::Instant::now() < until
            {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
    impl Drop for Connection {
        fn drop(&mut self) {
            // A reconnect drops this connection before opening the next one.
            // Release its test pause before join, never wait on our own hook.
            #[cfg(all(test, unix))]
            if let Some(release) = self.reader_release.take() {
                let _ = release.send(());
            }
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }

    fn reason(error: &ClientError) -> String {
        match error {
            ClientError::NeverEnabled => NEVER_ENABLED_MESSAGE.to_string(),
            ClientError::NotListening => OFF_MESSAGE.to_string(),
            ClientError::Refused => REFUSED_MESSAGE.to_string(),
            ClientError::Busy => BUSY_MESSAGE.to_string(),
            ClientError::TurnExpired => TURN_EXPIRED_MESSAGE.to_string(),
            ClientError::Other(error) => format!("Could not reach Clovy: {error}"),
        }
    }

    /// Relays `input` lines until end of input, bound to `turn` when given.
    pub fn relay_turn(
        dir: &Path,
        turn: Option<&str>,
        input: impl BufRead,
        output: Box<dyn Write + Send>,
    ) {
        relay_inner(
            dir,
            turn,
            input,
            output,
            #[cfg(all(test, unix))]
            None,
        );
    }

    #[cfg(all(test, unix))]
    pub(crate) fn relay_with_reader_pause(
        dir: &Path,
        input: impl BufRead,
        output: Box<dyn Write + Send>,
        pause: ReaderExitPause,
    ) {
        relay_inner(dir, None, input, output, Some(pause));
    }

    fn relay_inner(
        dir: &Path,
        turn: Option<&str>,
        mut input: impl BufRead,
        output: Box<dyn Write + Send>,
        #[cfg(all(test, unix))] mut reader_pause: Option<ReaderExitPause>,
    ) {
        let output: Output = Arc::new(Mutex::new(output));
        let refused = production_data_refusal(dir);
        let mut connection: Option<Connection> = None;
        loop {
            let bytes = match read_bounded_line(&mut input, channel::MAX_LINE_BYTES) {
                Ok(Some(InputLine::Line(bytes))) => bytes,
                Ok(Some(InputLine::TooLong)) => {
                    emit(
                        &output,
                        &protocol::error_response(
                            Value::Null,
                            protocol::INVALID_REQUEST,
                            TOO_LONG_MESSAGE,
                        ),
                    );
                    continue;
                }
                Ok(None) | Err(_) => break,
            };
            let parsed = String::from_utf8(bytes)
                .map_err(|error| error.to_string())
                .and_then(|text| {
                    let line = text.trim().to_string();
                    if line.is_empty() {
                        return Ok(None);
                    }
                    serde_json::from_str::<Value>(&line)
                        .map(|message| Some((line, message)))
                        .map_err(|error| error.to_string())
                });
            let (line, message) = match parsed {
                Ok(Some(parsed)) => parsed,
                Ok(None) => continue,
                Err(error) => {
                    emit(
                        &output,
                        &protocol::error_response(Value::Null, protocol::PARSE_ERROR, error),
                    );
                    continue;
                }
            };
            // Invalid requests are answered here; they never reach the app.
            if let protocol::Incoming::Invalid { id, reason } = protocol::classify(&message) {
                emit(
                    &output,
                    &protocol::error_response(id, protocol::INVALID_REQUEST, reason),
                );
                continue;
            }
            if let Some(reason) = refused {
                if let Some(response) = fallback(&message, reason) {
                    emit(&output, &response);
                }
                continue;
            }
            let line = line.as_str();
            if connection
                .as_ref()
                .is_some_and(|connection| !connection.alive.load(Ordering::SeqCst))
            {
                connection = None;
            }
            if connection.is_none() {
                match Connection::open(
                    dir,
                    turn,
                    &output,
                    #[cfg(all(test, unix))]
                    reader_pause.take(),
                ) {
                    Ok(opened) => connection = Some(opened),
                    Err(error) => {
                        if let Some(response) = fallback(&message, &reason(&error)) {
                            emit(&output, &response);
                        }
                        continue;
                    }
                }
            }
            let connection = connection.as_mut().expect("opened above");
            // If the reader won the race before registration, no bytes were
            // written. Answer conservatively without replaying this call.
            if !connection.send(line, &message) {
                if let Some(response) = uncertain(&message, DROPPED_MESSAGE) {
                    emit(&output, &response);
                }
            }
        }
        if let Some(connection) = connection.as_mut() {
            connection.finish();
        }
    }
}
