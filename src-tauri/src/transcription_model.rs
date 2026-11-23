//! User-installed, pinned speech model. Transcription itself never downloads files.
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Mutex};

#[cfg(test)]
tokio::task_local! {
    static TEST_BEFORE_FLUSH: (std::sync::Arc<tokio::sync::Notify>, std::sync::Arc<tokio::sync::Notify>);
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelState {
    Absent,
    Downloading { received: u64, total: u64 },
    Ready { bytes: u64 },
    Failed { reason: String },
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub repository: String,
    pub revision: String,
    pub total_bytes: u64,
    pub files: Vec<ModelFile>,
}

// A wakeable cancellation token backed by Tokio watch; no additional crate.
struct CancellationToken(tokio::sync::watch::Sender<bool>);
impl CancellationToken {
    fn new() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }
    fn cancel(&self) {
        self.0.send_replace(true);
    }
    fn is_cancelled(&self) -> bool {
        *self.0.borrow()
    }
    async fn cancelled(&self) {
        let mut receiver = self.0.subscribe();
        let _ = receiver.wait_for(|cancelled| *cancelled).await;
    }
}

pub struct TranscriptionModel {
    dir: PathBuf,
    manifest: Manifest,
    state: Mutex<ModelState>,
    cancellation: Mutex<Option<std::sync::Arc<CancellationToken>>>,
    operation: tokio::sync::Mutex<()>,
    removals: std::sync::atomic::AtomicUsize,
}

pub struct RemovalGuard<'a>(&'a std::sync::atomic::AtomicUsize);
impl Drop for RemovalGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl TranscriptionModel {
    pub fn new(dir: PathBuf, manifest: Manifest) -> Self {
        // The receipt check is metadata-only, so persisted Local selection is usable
        // before the startup worker runs. Missing/stale receipts still need its hash check.
        let state = if marker_matches(&dir, &manifest) {
            ModelState::Ready {
                bytes: manifest.total_bytes,
            }
        } else {
            ModelState::Absent
        };
        Self {
            dir,
            manifest,
            state: Mutex::new(state),
            cancellation: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            removals: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    /// Startup registers immediately; disk verification runs away from the window thread.
    pub async fn refresh_from_disk(
        &self,
        progress: impl Fn(ModelState) + Send + Sync,
        on_ready: impl FnOnce() + Send,
    ) {
        let _operation = self.operation.lock().await;
        if self.removals.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            return;
        }
        let dir = self.dir.clone();
        let manifest = self.manifest.clone();
        let ready = tokio::task::spawn_blocking(move || installed(&dir, &manifest))
            .await
            .unwrap_or(false);
        if self.removals.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            return;
        }
        self.publish(
            if ready {
                ModelState::Ready {
                    bytes: self.manifest.total_bytes,
                }
            } else {
                ModelState::Absent
            },
            &progress,
        );
        if ready {
            on_ready();
        }
    }
    pub fn directory(&self) -> &std::path::Path {
        &self.dir
    }
    pub fn state(&self) -> ModelState {
        if self.removals.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            return ModelState::Absent;
        }
        lock(&self.state).clone()
    }
    pub fn begin_removal(&self) -> RemovalGuard<'_> {
        self.removals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.cancel();
        RemovalGuard(&self.removals)
    }
    pub fn cancel(&self) {
        if let Some(token) = lock(&self.cancellation).as_ref() {
            token.cancel();
        }
    }
    fn publish(&self, state: ModelState, progress: &impl Fn(ModelState)) {
        *lock(&self.state) = state.clone();
        progress(state);
    }
    pub async fn install(
        &self,
        base: &str,
        progress: impl Fn(ModelState) + Send + Sync,
    ) -> Result<(), String> {
        let _operation = self
            .operation
            .try_lock()
            .map_err(|_| "local_model_busy".to_string())?;
        let token = std::sync::Arc::new(CancellationToken::new());
        *lock(&self.cancellation) = Some(token.clone());
        if self.removals.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            token.cancel();
        }
        self.publish(
            ModelState::Downloading {
                received: 0,
                total: self.manifest.total_bytes,
            },
            &progress,
        );
        // Only network waits are interrupted. Dropping Tokio filesystem operations
        // can leave blocking writes running after the operation lock is released.
        let result = self.download(base, &progress, &token).await;
        let result = if token.is_cancelled() {
            Ok(false)
        } else {
            result.map(|()| true)
        };
        let final_state = match &result {
            Ok(true) if !token.is_cancelled() => ModelState::Ready {
                bytes: self.manifest.total_bytes,
            },
            Ok(_) => ModelState::Absent,
            Err(reason) => ModelState::Failed {
                reason: reason.clone(),
            },
        };
        if !matches!(final_state, ModelState::Ready { .. }) {
            let _ = tokio::fs::remove_file(self.dir.join("installed.json")).await;
        }
        *lock(&self.cancellation) = None;
        self.publish(final_state, &progress);
        result.map(|_| ())
    }
    async fn download(
        &self,
        base: &str,
        progress: &impl Fn(ModelState),
        token: &CancellationToken,
    ) -> Result<(), String> {
        use futures_util::StreamExt;
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncWriteExt;
        tokio::fs::create_dir_all(&self.dir)
            .await
            .map_err(|e| e.to_string())?;
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(20))
            .read_timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        let mut completed = 0;
        for file in &self.manifest.files {
            if token.is_cancelled() {
                return Ok(());
            }
            let destination = self.dir.join(&file.path);
            if verified_during_install(&destination, file, token).await? {
                completed += file.bytes;
                self.publish(
                    ModelState::Downloading {
                        received: completed,
                        total: self.manifest.total_bytes,
                    },
                    progress,
                );
                continue;
            }
            let parent = destination.parent().ok_or("invalid model path")?;
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
            let part = self.dir.join(format!("{}.part", file.path));
            let mut offset = tokio::fs::metadata(&part)
                .await
                .map(|m| m.len())
                .unwrap_or(0);
            // Cancellation can arrive after the final durable progress event,
            // before checksum verification/rename. Reuse only a verified part.
            if offset == file.bytes && verified_during_install(&part, file, token).await? {
                tokio::fs::rename(&part, &destination)
                    .await
                    .map_err(|e| e.to_string())?;
                completed += file.bytes;
                self.publish(
                    ModelState::Downloading {
                        received: completed,
                        total: self.manifest.total_bytes,
                    },
                    progress,
                );
                continue;
            }
            if offset >= file.bytes {
                offset = 0;
            }
            let mut request = client
                .get(format!("{}/{}", base.trim_end_matches('/'), file.path))
                .header(reqwest::header::ACCEPT_ENCODING, "identity");
            if offset > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
            }
            let response = tokio::select! {
                biased;
                _ = token.cancelled() => return Ok(()),
                response = request.send() => response,
            }
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| format!("network: {}", file.path))?;
            if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
                let expected = format!("bytes {offset}-{}/{}", file.bytes - 1, file.bytes);
                if response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    != Some(expected.as_str())
                {
                    return Err(format!("range: {}", file.path));
                }
            } else {
                offset = 0;
            }
            let mut hash = Sha256::new();
            if offset > 0 {
                hash = hash_resume_prefix(&part, token).await?;
            }
            let mut output = tokio::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(offset == 0)
                .append(offset > 0)
                .open(&part)
                .await
                .map_err(|e| e.to_string())?;
            let mut received = offset;
            let mut last_progress = None;
            let mut stream = response.bytes_stream();
            loop {
                let chunk = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Ok(()),
                    chunk = stream.next() => chunk,
                };
                let Some(chunk) = chunk else { break };
                let chunk = chunk.map_err(|_| format!("network: {}", file.path))?;
                received += chunk.len() as u64;
                if received > file.bytes {
                    return Err(format!("size: {}", file.path));
                }
                output.write_all(&chunk).await.map_err(|e| e.to_string())?;
                #[cfg(test)]
                if let Ok((entered, resume)) = TEST_BEFORE_FLUSH.try_with(Clone::clone) {
                    entered.notify_one();
                    resume.notified().await;
                }
                // Make progress a durable watermark: cancel/restart resumes these bytes.
                output.flush().await.map_err(|e| e.to_string())?;
                hash.update(&chunk);
                // Bound IPC/render work independently of the server's chunk size.
                // Always publish the first durable bytes and the completed file.
                let now = std::time::Instant::now();
                if received == file.bytes
                    || last_progress.map_or(true, |last| {
                        now.duration_since(last) >= std::time::Duration::from_millis(100)
                    })
                {
                    self.publish(
                        ModelState::Downloading {
                            received: completed + received,
                            total: self.manifest.total_bytes,
                        },
                        progress,
                    );
                    last_progress = Some(now);
                }
                if token.is_cancelled() {
                    return Ok(());
                }
            }
            output.sync_all().await.map_err(|e| e.to_string())?;
            drop(output);
            if received != file.bytes || format!("{:x}", hash.finalize()) != file.sha256 {
                let _ = tokio::fs::remove_file(&part).await;
                return Err(format!("checksum: {}", file.path));
            }
            tokio::fs::rename(&part, &destination)
                .await
                .map_err(|e| e.to_string())?;
            completed += file.bytes;
        }
        write_marker(&self.dir, &self.manifest).map_err(|e| e.to_string())
    }
    pub async fn remove(&self) -> Result<(), String> {
        let _removal = self.begin_removal();
        let _operation = self.operation.lock().await;
        // Disable selection before removing any files.
        *lock(&self.state) = ModelState::Absent;
        match tokio::fs::remove_dir_all(&self.dir).await {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                *lock(&self.state) = ModelState::Failed {
                    reason: error.to_string(),
                };
                Err(error.to_string())
            }
        }
    }
}

async fn hash_resume_prefix(
    path: &std::path::Path,
    token: &CancellationToken,
) -> Result<sha2::Sha256, String> {
    use sha2::Digest;
    use tokio::io::AsyncReadExt;
    if token.is_cancelled() {
        return Err("cancelled".into());
    }
    let mut input = tokio::fs::File::open(path)
        .await
        .map_err(|error| error.to_string())?;
    let mut hash = sha2::Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = tokio::select! {
            biased;
            _ = token.cancelled() => return Err("cancelled".into()),
            read = input.read(&mut buffer) => read.map_err(|error| error.to_string())?,
        };
        if count == 0 {
            return Ok(hash);
        }
        hash.update(&buffer[..count]);
    }
}

async fn verified_during_install(
    path: &std::path::Path,
    file: &ModelFile,
    token: &CancellationToken,
) -> Result<bool, String> {
    use sha2::Digest;
    if !tokio::fs::symlink_metadata(path)
        .await
        .is_ok_and(|m| m.is_file() && m.len() == file.bytes)
    {
        return Ok(false);
    }
    let hash = hash_resume_prefix(path, token).await?;
    Ok(format!("{:x}", hash.finalize()) == file.sha256)
}

fn hash_prefix(path: &std::path::Path, hash: &mut sha2::Sha256) -> std::io::Result<()> {
    use sha2::Digest;
    use std::io::Read;
    let mut input = std::fs::File::open(path)?;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            return Ok(());
        }
        hash.update(&buffer[..count]);
    }
}
fn verified(path: &std::path::Path, file: &ModelFile) -> bool {
    use sha2::Digest;
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == file.bytes) {
        return false;
    }
    let mut hash = sha2::Sha256::new();
    hash_prefix(path, &mut hash).is_ok() && format!("{:x}", hash.finalize()) == file.sha256
}
fn write_marker(dir: &std::path::Path, manifest: &Manifest) -> std::io::Result<()> {
    use std::io::Write;
    let mut marker = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut marker, manifest)?;
    marker.flush()?;
    marker.as_file().sync_all()?;
    marker
        .persist(dir.join("installed.json"))
        .map_err(|e| e.error)?;
    Ok(())
}
fn marker_matches(dir: &std::path::Path, manifest: &Manifest) -> bool {
    // D5 intentionally treats this marker as the receipt for verified installation bytes.
    // A matching pinned manifest and file sizes avoid rehashing 632 MB at each startup.
    // Missing receipts or size changes rehash below; same-size external edits are outside
    // this readiness check (see ADR-0074 and the change's design.md, D5).
    let marker = std::fs::read(dir.join("installed.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<Manifest>(&b).ok());
    marker.as_ref() == Some(manifest)
        && manifest.files.iter().all(|file| {
            std::fs::symlink_metadata(dir.join(&file.path))
                .is_ok_and(|m| m.is_file() && m.len() == file.bytes)
        })
}
fn installed(dir: &std::path::Path, manifest: &Manifest) -> bool {
    if marker_matches(dir, manifest) {
        return true;
    }
    if manifest
        .files
        .iter()
        .all(|file| verified(&dir.join(&file.path), file))
    {
        return write_marker(dir, manifest).is_ok();
    }
    false
}

pub const MODEL_STATE_EVENT: &str = "local-transcription-model-state";
static MODEL: std::sync::OnceLock<std::sync::Arc<TranscriptionModel>> = std::sync::OnceLock::new();

pub fn setup(app: &mut tauri::App) -> Result<(), tauri::Error> {
    use tauri::{Emitter, Manager};
    if !crate::providers::local_transcription_supported() {
        return Ok(());
    }
    let dir = crate::app_paths::app_data_dir(app.handle())?.join("models/parakeet-ultra-coreml");
    let manifest = serde_json::from_str(include_str!("../resources/local-transcriber-model.json"))
        .expect("pinned model manifest");
    let model = MODEL.get_or_init(|| std::sync::Arc::new(TranscriptionModel::new(dir, manifest)));
    app.manage(model.clone());
    let model = model.clone();
    let app = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        model
            .refresh_from_disk(
                |state| {
                    let _ = app.emit(MODEL_STATE_EVENT, state);
                },
                crate::providers::local_transcriber::selection_changed,
            )
            .await;
    });
    Ok(())
}
pub fn runtime(
) -> Result<&'static std::sync::Arc<TranscriptionModel>, crate::domain::types::AppError> {
    MODEL.get().ok_or_else(|| {
        crate::domain::types::AppError::new(
            "local_model_not_installed",
            "Install the local model in Settings > Models.",
        )
    })
}
pub fn ready_directory() -> Option<PathBuf> {
    let model = runtime().ok()?;
    matches!(model.state(), ModelState::Ready { .. }).then(|| model.dir.clone())
}
pub fn download_base() -> String {
    let manifest: Manifest =
        serde_json::from_str(include_str!("../resources/local-transcriber-model.json"))
            .expect("pinned model manifest");
    format!(
        "https://huggingface.co/{}/resolve/{}",
        manifest.repository, manifest.revision
    )
}

#[cfg(test)]
#[path = "transcription_model_tests.rs"]
mod tests;
