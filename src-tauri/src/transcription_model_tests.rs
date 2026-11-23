use super::*;
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn manifest() -> Manifest {
    let files = [
        ("Encoder.mlmodelc/model.mil", b"encoder".as_slice()),
        ("parakeet_vocab.json", b"vocabulary".as_slice()),
    ]
    .into_iter()
    .map(|(path, data)| ModelFile {
        path: path.into(),
        bytes: data.len() as u64,
        sha256: format!("{:x}", Sha256::digest(data)),
    })
    .collect();
    Manifest {
        repository: "fixture/model".into(),
        revision: "fixture".into(),
        total_bytes: 17,
        files,
    }
}
struct Server {
    url: String,
    seen: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn server(corrupt: bool, delay: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let requests = seen.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut data = Vec::new();
            while !data.ends_with(b"\r\n\r\n") {
                let mut b = [0; 1];
                if stream.read(&mut b).await.unwrap() == 0 {
                    break;
                }
                data.extend_from_slice(&b);
            }
            let request = String::from_utf8(data).unwrap();
            requests.lock().unwrap().push(request.clone());
            let body: &[u8] = if request.contains("model.mil") {
                b"encoder"
            } else if corrupt {
                b"corruption"
            } else {
                b"vocabulary"
            };
            let start = request
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(|v| v.trim_end_matches('-').parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let headers = if start > 0 {
                format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\n",
                    body.len() - 1,
                    body.len()
                )
            } else {
                "HTTP/1.1 200 OK\r\n".into()
            };
            if stream
                .write_all(
                    format!(
                        "{headers}Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len() - start
                    )
                    .as_bytes(),
                )
                .await
                .is_err()
            {
                continue;
            }
            for byte in &body[start..] {
                if delay {
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                if stream.write_all(&[*byte]).await.is_err() {
                    break;
                }
            }
        }
    });
    Server { url, seen, task }
}
#[tokio::test]
async fn install_verifies_bytes_and_reopens_ready_without_network() {
    let temp = tempfile::tempdir().unwrap();
    let server = server(false, false).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    let progress = Mutex::new(Vec::new());
    model
        .install(&server.url, |s| progress.lock().unwrap().push(s))
        .await
        .unwrap();
    assert_eq!(model.state(), ModelState::Ready { bytes: 17 });
    assert!(progress.lock().unwrap().iter().any(|s| matches!(
        s,
        ModelState::Downloading {
            received: 17,
            total: 17
        }
    )));
    let reopened = TranscriptionModel::new(model.dir.clone(), manifest());
    reopened.refresh_from_disk(|_| {}, || {}).await;
    assert_eq!(reopened.state(), ModelState::Ready { bytes: 17 });
    reopened.install(&server.url, |_| {}).await.unwrap();
    assert_eq!(server.seen.lock().unwrap().len(), 2);
    reopened.remove().await.unwrap();
    assert_eq!(reopened.state(), ModelState::Absent);
    assert!(!model.dir.exists());
}
#[tokio::test]
async fn corrupt_file_preserves_verified_files_and_retry_fetches_only_missing() {
    let temp = tempfile::tempdir().unwrap();
    let bad = server(true, false).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    let error = model.install(&bad.url, |_| {}).await.unwrap_err();
    assert!(error.contains("parakeet_vocab.json"));
    assert!(matches!(model.state(), ModelState::Failed { .. }));
    assert_eq!(
        std::fs::read(model.dir.join("Encoder.mlmodelc/model.mil")).unwrap(),
        b"encoder"
    );
    let good = server(false, false).await;
    model.install(&good.url, |_| {}).await.unwrap();
    assert_eq!(good.seen.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn cancel_returns_absent_and_resume_uses_range() {
    let temp = tempfile::tempdir().unwrap();
    let server = server(false, true).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    let watermark = Mutex::new(0);
    model
        .install(&server.url, |s| {
            if let ModelState::Downloading { received, .. } = s {
                if received >= 3 {
                    *lock(&watermark) = received;
                    model.cancel();
                }
            }
        })
        .await
        .unwrap();
    assert_eq!(model.state(), ModelState::Absent);
    assert!(!model.dir.join("installed.json").exists());
    let reopened = TranscriptionModel::new(model.dir.clone(), manifest());
    reopened.refresh_from_disk(|_| {}, || {}).await;
    assert_eq!(reopened.state(), ModelState::Absent);
    reopened.install(&server.url, |_| {}).await.unwrap();
    assert_eq!(reopened.state(), ModelState::Ready { bytes: 17 });
    assert!(server.seen.lock().unwrap().iter().any(|s| s
        .to_ascii_lowercase()
        .contains(&format!("range: bytes={}-", *lock(&watermark)))));
}
#[tokio::test]
async fn parakeet_review_complete_partial_is_verified_before_retry_downloads() {
    let temp = tempfile::tempdir().unwrap();
    let server = server(false, true).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    model
        .install(&server.url, |state| {
            if matches!(state, ModelState::Downloading { received: 7, .. }) {
                model.cancel();
            }
        })
        .await
        .unwrap();
    assert_eq!(model.state(), ModelState::Absent);
    assert_eq!(
        std::fs::read(model.dir.join("Encoder.mlmodelc/model.mil.part")).unwrap(),
        b"encoder"
    );
    model.install(&server.url, |_| {}).await.unwrap();
    assert_eq!(model.state(), ModelState::Ready { bytes: 17 });
    assert_eq!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.contains("model.mil"))
            .count(),
        1
    );
}

#[tokio::test]
async fn parakeet_review_corrupt_complete_partial_is_downloaded_again() {
    let temp = tempfile::tempdir().unwrap();
    let server = server(false, false).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    std::fs::create_dir_all(model.dir.join("Encoder.mlmodelc")).unwrap();
    std::fs::write(
        model.dir.join("Encoder.mlmodelc/model.mil.part"),
        b"invalid",
    )
    .unwrap();
    model.install(&server.url, |_| {}).await.unwrap();
    assert_eq!(model.state(), ModelState::Ready { bytes: 17 });
    assert_eq!(
        std::fs::read(model.dir.join("Encoder.mlmodelc/model.mil")).unwrap(),
        b"encoder"
    );
    let requests = server.seen.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests
        .iter()
        .all(|r| !r.to_ascii_lowercase().contains("range:")));
}

#[tokio::test]
async fn disk_without_marker_is_verified_and_size_changes_invalidate_marker() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("model");
    std::fs::create_dir_all(dir.join("Encoder.mlmodelc")).unwrap();
    std::fs::write(dir.join("Encoder.mlmodelc/model.mil"), b"encoder").unwrap();
    assert_eq!(reopened_state(dir.clone()).await, ModelState::Absent);
    std::fs::write(dir.join("parakeet_vocab.json"), b"vocabulary").unwrap();
    assert_eq!(
        reopened_state(dir.clone()).await,
        ModelState::Ready { bytes: 17 }
    );
    std::fs::write(dir.join("parakeet_vocab.json"), b"bad").unwrap();
    assert_eq!(reopened_state(dir).await, ModelState::Absent);
}

#[tokio::test]
async fn removal_disables_selection_before_waiting_for_download_lock() {
    let temp = tempfile::tempdir().unwrap();
    let server = server(false, false).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    model.install(&server.url, |_| {}).await.unwrap();
    let guard = model.operation.lock().await;
    let removal = model.remove();
    tokio::pin!(removal);
    tokio::select! { biased; _ = &mut removal => panic!("operation lock must block filesystem removal"), _ = tokio::task::yield_now() => {} }
    assert!(
        !matches!(model.state(), ModelState::Ready { .. }),
        "removing model must not remain selectable"
    );
    drop(guard);
    removal.await.unwrap();
}

#[tokio::test]
async fn startup_registration_does_not_hash_files_or_write_the_install_marker() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("model");
    std::fs::create_dir_all(dir.join("Encoder.mlmodelc")).unwrap();
    std::fs::write(dir.join("Encoder.mlmodelc/model.mil"), b"encoder").unwrap();
    std::fs::write(dir.join("parakeet_vocab.json"), b"vocabulary").unwrap();
    let model = TranscriptionModel::new(dir.clone(), manifest());
    assert_eq!(model.state(), ModelState::Absent);
    assert!(!dir.join("installed.json").exists());
    let emitted = Mutex::new(Vec::new());
    // A removal queued during startup must not revive readiness.
    let removing = model.begin_removal();
    model
        .refresh_from_disk(|state| lock(&emitted).push(state), || {})
        .await;
    assert!(lock(&emitted).is_empty());
    assert!(!dir.join("installed.json").exists());
    drop(removing);
    model
        .refresh_from_disk(|state| lock(&emitted).push(state), || {})
        .await;
    assert_eq!(model.state(), ModelState::Ready { bytes: 17 });
    assert_eq!(*lock(&emitted), vec![ModelState::Ready { bytes: 17 }]);
    assert!(dir.join("installed.json").exists());
}

#[test]
fn parakeet_review_verified_receipt_is_ready_before_startup_worker_runs() {
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("model");
    std::fs::create_dir_all(dir.join("Encoder.mlmodelc")).unwrap();
    std::fs::write(dir.join("Encoder.mlmodelc/model.mil"), b"encoder").unwrap();
    std::fs::write(dir.join("parakeet_vocab.json"), b"vocabulary").unwrap();
    write_marker(&dir, &manifest()).unwrap();

    // No async refresh has run: a previously verified installation is immediately usable.
    let model = TranscriptionModel::new(dir.clone(), manifest());
    assert_eq!(model.state(), ModelState::Ready { bytes: 17 });
    let removing = model.begin_removal();
    assert_eq!(model.state(), ModelState::Absent);
    drop(removing);

    // A stale receipt cannot admit changed or missing files before worker verification.
    std::fs::write(dir.join("parakeet_vocab.json"), b"short").unwrap();
    assert_eq!(
        TranscriptionModel::new(dir.clone(), manifest()).state(),
        ModelState::Absent
    );
    std::fs::remove_file(dir.join("Encoder.mlmodelc/model.mil")).unwrap();
    assert_eq!(
        TranscriptionModel::new(dir, manifest()).state(),
        ModelState::Absent
    );
}

async fn reopened_state(dir: PathBuf) -> ModelState {
    let model = TranscriptionModel::new(dir, manifest());
    model.refresh_from_disk(|_| {}, || {}).await;
    model.state()
}

#[tokio::test]
async fn parakeet_review_cancelled_resume_does_not_hash_the_prefix() {
    let temp = tempfile::tempdir().unwrap();
    let part = temp.path().join("encoder.part");
    std::fs::write(&part, b"partial encoder").unwrap();
    let token = CancellationToken::new();
    token.cancel();
    assert!(hash_resume_prefix(&part, &token).await.is_err());
    assert_eq!(std::fs::read(part).unwrap(), b"partial encoder");
}

#[tokio::test]
async fn parakeet_review_startup_warms_only_after_ready_is_published() {
    let temp = tempfile::tempdir().unwrap();
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    let warmed = Mutex::new(0);
    model
        .refresh_from_disk(|_| {}, || *lock(&warmed) += 1)
        .await;
    assert_eq!(*lock(&warmed), 0);
    let server = server(false, false).await;
    model.install(&server.url, |_| {}).await.unwrap();
    let reopened = TranscriptionModel::new(model.dir.clone(), manifest());
    reopened
        .refresh_from_disk(
            |_| {},
            || {
                assert_eq!(reopened.state(), ModelState::Ready { bytes: 17 });
                *lock(&warmed) += 1;
            },
        )
        .await;
    assert_eq!(*lock(&warmed), 1);
}

#[tokio::test]
async fn parakeet_review_download_progress_is_throttled_but_finishes_at_total() {
    let temp = tempfile::tempdir().unwrap();
    let server = server(false, true).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    let events = Mutex::new(Vec::new());
    model
        .install(&server.url, |state| lock(&events).push(state))
        .await
        .unwrap();
    let events = lock(&events);
    let downloading: Vec<_> = events
        .iter()
        .filter_map(|state| match state {
            ModelState::Downloading { received, .. } => Some(*received),
            _ => None,
        })
        .collect();
    assert!(
        downloading.len() <= 10,
        "one event per network byte: {downloading:?}"
    );
    assert_eq!(downloading.last(), Some(&17));
    assert_eq!(events.last(), Some(&ModelState::Ready { bytes: 17 }));
}

#[tokio::test]
async fn cancellation_holds_operation_until_pending_output_is_flushed() {
    let temp = tempfile::tempdir().unwrap();
    let server = server(false, false).await;
    let model = TranscriptionModel::new(temp.path().join("model"), manifest());
    let entered = Arc::new(tokio::sync::Notify::new());
    let resume = Arc::new(tokio::sync::Notify::new());
    TEST_BEFORE_FLUSH
        .scope((entered.clone(), resume.clone()), async {
            let mut install = Box::pin(model.install(&server.url, |_| {}));
            tokio::select! {
                _ = entered.notified() => {},
                result = &mut install => panic!("install ended before flush: {result:?}"),
            }
            model.cancel();
            assert!(
                tokio::time::timeout(Duration::from_millis(20), &mut install)
                    .await
                    .is_err(),
                "cancellation must settle the pending output before returning"
            );
            assert!(model.operation.try_lock().is_err());
            resume.notify_one();
            install.await.unwrap();
        })
        .await;
    assert_eq!(model.state(), ModelState::Absent);
    assert!(!model.dir.join("installed.json").exists());
    model.install(&server.url, |_| {}).await.unwrap();
    assert_eq!(model.state(), ModelState::Ready { bytes: 17 });
    assert_eq!(
        std::fs::read(model.dir.join("Encoder.mlmodelc/model.mil")).unwrap(),
        b"encoder"
    );
}

#[tokio::test]
async fn cancellation_interrupts_stalled_response_headers_and_body() {
    for send_headers in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let connected = Arc::new(tokio::sync::Notify::new());
        let signal = connected.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            if send_headers {
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\n")
                    .await
                    .unwrap();
            }
            signal.notify_one();
            std::future::pending::<()>().await;
            drop(socket);
        });
        let temp = tempfile::tempdir().unwrap();
        let model = TranscriptionModel::new(temp.path().join("model"), manifest());
        let mut install = Box::pin(model.install(&url, |_| {}));
        tokio::select! {
            _ = connected.notified() => {},
            result = &mut install => panic!("unexpected install result: {result:?}"),
        }
        model.cancel();
        let result = tokio::time::timeout(Duration::from_secs(1), &mut install).await;
        server.abort();
        result.expect("network cancellation timed out").unwrap();
        assert_eq!(model.state(), ModelState::Absent);
    }
}
