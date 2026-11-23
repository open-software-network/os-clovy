//! RED for WT-20261004-clovy-s6-servidor-mcp: a process that connects to the
//! Clovy MCP socket without the installation secret is refused and never
//! reaches the data.

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use super::tests::{data, handler_for, notes_pool};
use super::tools::ActivityAccess;
use super::*;

async fn exchange(dir: &std::path::Path, first_line: Value) -> (Value, Option<String>) {
    let stream = channel::test_stream(dir).await;
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    write
        .write_all(format!("{first_line}\n").as_bytes())
        .await
        .unwrap();
    let mut reply = String::new();
    reader.read_line(&mut reply).await.unwrap();
    let call = json!({ "jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": { "name": "search_notes", "arguments": { "query": "zephyr" } } });
    // The app may already have closed the connection; a failed write is fine.
    let _ = write.write_all(format!("{call}\n").as_bytes()).await;
    let mut answer = String::new();
    let answered = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut answer),
    )
    .await
    .ok()
    .and_then(Result::ok)
    .filter(|read| *read > 0)
    .map(|_| answer);
    (
        serde_json::from_str(&reply).unwrap_or(Value::Null),
        answered,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_connection_without_the_installation_secret_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let listener = channel::Listener::start(
        &dir,
        handler_for(data(notes_pool().await, ActivityAccess::Off)),
    )
    .await
    .unwrap();

    let wrong = "0".repeat(64);
    let (reply, answered) = exchange(&dir, channel::handshake(&wrong)).await;
    assert_eq!(reply["ok"], json!(false), "wrong secret accepted: {reply}");
    assert_eq!(answered, None, "refused connection still answered");

    let (reply, answered) = exchange(
        &dir,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    )
    .await;
    assert_eq!(reply["ok"], json!(false), "no handshake accepted: {reply}");
    assert_eq!(answered, None, "connection without handshake answered");

    let secret = channel::read_secret(&dir).unwrap();
    let (reply, answered) = exchange(&dir, channel::handshake(&secret)).await;
    assert_eq!(reply["ok"], json!(true));
    assert!(answered.is_some_and(|answer| answer.contains("Roadmap sync")));
    listener.stop();
}
