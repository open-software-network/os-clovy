//! Real relay executable and real local channel, with a synthetic effect file.
//! Portable to Windows; does not require the desktop, a CLI or credentials.
#![cfg(any(unix, windows))]
use clovy_lib::mcp_server::{channel, protocol};
use serde_json::{json, Value};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_binary_carries_large_receipts_and_reports_lost_mutation_without_replay() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let effects = root.path().join("effect-count");
    let target = effects.clone();
    let admitted = Arc::new(tokio::sync::Notify::new());
    let signal = admitted.clone();
    let handler: channel::Handler = Arc::new(move |message| {
        let target = target.clone();
        let signal = signal.clone();
        Box::pin(async move {
            if message["id"] == 2 {
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(target)
                    .unwrap()
                    .write_all(b"effect\n")
                    .unwrap();
                signal.notify_one();
                return std::future::pending().await;
            }
            Some(protocol::result_response(
                message["id"].clone(),
                json!({"payload":"p".repeat(1024 * 1024)}),
            ))
        })
    });
    let listener = channel::Listener::start(&dir, handler).await.unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_clovy-mcp"))
        .arg("--dir")
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(10), output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        response["result"]["payload"].as_str().unwrap().len(),
        1024 * 1024
    );
    input
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\"}\n")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), admitted.notified())
        .await
        .unwrap();
    listener.stop();
    line.clear();
    tokio::time::timeout(Duration::from_secs(5), output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(&line).unwrap();
    let error = &response["result"]["structuredContent"]["error"];
    assert_eq!(error["code"], "mcp_delivery_unknown");
    assert_eq!(
        error["details"],
        json!({"delivery":"unknown","effect":"unknown","retrySafe":false,"callId":2})
    );
    assert_eq!(std::fs::read_to_string(effects).unwrap(), "effect\n");
    drop(input);
    assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_binary_bounds_input_and_recovers_after_invalid_messages() {
    let root = tempfile::tempdir().unwrap();
    let dir = channel::channel_dir(root.path());
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_clovy-mcp"))
        .arg("--dir")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    input
        .write_all(&vec![b'x'; channel::MAX_LINE_BYTES + 1])
        .await
        .unwrap();
    input
        .write_all(b"\n{broken}\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n")
        .await
        .unwrap();
    let mut responses = Vec::new();
    for _ in 0..3 {
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(5), output.read_line(&mut line))
            .await
            .unwrap()
            .unwrap();
        responses.push(serde_json::from_str::<Value>(&line).unwrap());
    }
    assert_eq!(responses[0]["error"]["code"], protocol::INVALID_REQUEST);
    assert_eq!(responses[1]["error"]["code"], protocol::PARSE_ERROR);
    assert_eq!(responses[2], protocol::result_response(json!(9), json!({})));
    drop(input);
    assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .unwrap()
        .unwrap()
        .success());
}
