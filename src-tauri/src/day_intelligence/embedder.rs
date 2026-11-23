//! On-device sentence embeddings for the distiller's semantic dedup and
//! diversity pick: BGE small (BERT, 384 dimensions) run with candle on the
//! CPU (Accelerate), CLS-pooled and L2-normalized so cosine is a dot product.
//!
//! The weights are downloaded once, from a pinned revision with pinned
//! SHA-256 digests, into `<app data dir>/models/bge-small-en-v1.5/`; this is
//! the only network access of the distillation. Cached files are trusted only
//! after their digests match again ([`LocalEmbedder::ensure_ready`], once per
//! launch); a mismatch is deleted and downloaded again. Until then (or when
//! loading fails), [`Embedder::embed`] answers `None` and the distiller
//! degrades to lexical stages. The model is loaded for one batch and dropped
//! right after, so ~130 MB of weights do not stay resident between hours.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures_util::future::BoxFuture;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub trait Embedder: Send + Sync {
    /// One L2-normalized vector per text, same order; `None` when embeddings
    /// are unavailable.
    fn embed(&self, texts: Vec<String>) -> BoxFuture<'_, Option<Vec<Vec<f32>>>>;
}

/// No embeddings: the distiller's lexical-only path.
pub struct NoEmbedder;

impl Embedder for NoEmbedder {
    fn embed(&self, _texts: Vec<String>) -> BoxFuture<'_, Option<Vec<Vec<f32>>>> {
        Box::pin(async { None })
    }
}

pub const MODEL_REPO: &str = "BAAI/bge-small-en-v1.5";
pub const MODEL_REVISION: &str = "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a";
pub const MODEL_DIR_NAME: &str = "bge-small-en-v1.5";

struct ModelFile<'d> {
    name: &'static str,
    size: u64,
    sha256: &'d str,
}

const MODEL_FILES: [ModelFile<'static>; 3] = [
    ModelFile {
        name: "config.json",
        size: 743,
        sha256: "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
    },
    ModelFile {
        name: "tokenizer.json",
        size: 711_396,
        sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    },
    ModelFile {
        name: "model.safetensors",
        size: 133_466_304,
        sha256: "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EmbedderStatus {
    /// Not verified or downloaded yet (nothing asked for it: no activity
    /// provider).
    Absent,
    /// Verifying the cached files or downloading the missing ones.
    Downloading,
    Ready,
    /// The download or the model failed; lexical distillation continues.
    Failed,
}

/// The BGE embedder over the files in `dir`.
pub struct LocalEmbedder {
    dir: PathBuf,
    status: Mutex<EmbedderStatus>,
    /// Serializes model use (one forward pass at a time).
    run: Arc<Mutex<()>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl LocalEmbedder {
    /// `models_dir` is `<app data dir>/models`. Nothing is trusted yet: the
    /// first [`Self::ensure_ready`] verifies what is on disk.
    pub fn new(models_dir: &Path) -> Self {
        Self {
            dir: models_dir.join(MODEL_DIR_NAME),
            status: Mutex::new(EmbedderStatus::Absent),
            run: Arc::new(Mutex::new(())),
        }
    }

    pub fn status(&self) -> EmbedderStatus {
        *lock(&self.status)
    }

    /// Verifies the cached files against their pinned digests, deletes the
    /// ones that do not match, and downloads what is missing (pinned
    /// revision, verified digests). A failure is not retried until the next
    /// launch.
    pub async fn ensure_ready(&self) {
        {
            let mut status = lock(&self.status);
            if *status != EmbedderStatus::Absent {
                return;
            }
            *status = EmbedderStatus::Downloading;
        }
        let result = prepare(&self.dir).await;
        let mut status = lock(&self.status);
        *status = match result {
            Ok(()) => EmbedderStatus::Ready,
            Err(error) => {
                tracing::warn!(%error, "embedding model download failed");
                EmbedderStatus::Failed
            }
        };
    }
}

impl Embedder for LocalEmbedder {
    fn embed(&self, texts: Vec<String>) -> BoxFuture<'_, Option<Vec<Vec<f32>>>> {
        Box::pin(async move {
            if self.status() != EmbedderStatus::Ready || texts.is_empty() {
                return None;
            }
            let dir = self.dir.clone();
            let run = Arc::clone(&self.run);
            let result = tokio::task::spawn_blocking(move || {
                let _guard = lock(&run);
                bert::embed(&dir, &texts)
            })
            .await;
            match result {
                Ok(Ok(vectors)) => Some(vectors),
                Ok(Err(error)) => {
                    tracing::warn!(%error, "local embeddings failed; lexical distillation only");
                    None
                }
                Err(error) => {
                    tracing::warn!(%error, "local embedding task failed");
                    None
                }
            }
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Whether `path` holds exactly `file`: its size, then its SHA-256.
fn verified(path: &Path, file: &ModelFile<'_>) -> bool {
    use std::io::Read;

    if !std::fs::metadata(path).is_ok_and(|meta| meta.len() == file.size) {
        return false;
    }
    let Ok(mut reader) = std::fs::File::open(path) else {
        return false;
    };
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 16];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => hasher.update(&buffer[..read]),
            Err(_) => return false,
        }
    }
    hex(&hasher.finalize()) == file.sha256
}

/// The files of `files` that `dir` does not hold verified; cached files whose
/// digest does not match are deleted so they are downloaded again.
fn unverified<'a, 'd>(dir: &Path, files: &'a [ModelFile<'d>]) -> Vec<&'a ModelFile<'d>> {
    files
        .iter()
        .filter(|file| {
            let path = dir.join(file.name);
            if verified(&path, file) {
                return false;
            }
            if path.exists() {
                tracing::warn!(
                    file = file.name,
                    "embedding model file does not match its digest"
                );
                let _ = std::fs::remove_file(&path);
            }
            true
        })
        .collect()
}

/// Verifies the cache, then downloads the files that are missing.
async fn prepare(dir: &Path) -> Result<(), String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|error| error.to_string())?;
    let cache = dir.to_path_buf();
    let missing = tokio::task::spawn_blocking(move || {
        unverified(&cache, &MODEL_FILES)
            .into_iter()
            .map(|file| file.name)
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|error| error.to_string())?;
    if missing.is_empty() {
        return Ok(());
    }
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| error.to_string())?;
    for file in MODEL_FILES
        .iter()
        .filter(|file| missing.contains(&file.name))
    {
        let dest = dir.join(file.name);
        let url = format!(
            "https://huggingface.co/{MODEL_REPO}/resolve/{MODEL_REVISION}/{}",
            file.name
        );
        let response = client
            .get(&url)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| format!("{}: {error}", file.name))?;
        let part = dir.join(format!("{}.part", file.name));
        let mut out = tokio::fs::File::create(&part)
            .await
            .map_err(|error| error.to_string())?;
        let mut hasher = Sha256::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| format!("{}: {error}", file.name))?;
            hasher.update(&chunk);
            out.write_all(&chunk)
                .await
                .map_err(|error| error.to_string())?;
        }
        out.flush().await.map_err(|error| error.to_string())?;
        drop(out);
        let digest = hex(&hasher.finalize());
        if digest != file.sha256 {
            let _ = tokio::fs::remove_file(&part).await;
            return Err(format!("{}: digest mismatch", file.name));
        }
        tokio::fs::rename(&part, &dest)
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod bert {
    use std::path::Path;

    use candle_core::{DType, Device, IndexOp, Tensor};
    use candle_nn::VarBuilder;
    use candle_transformers::models::bert::{BertModel, Config, DTYPE};
    use tokenizers::{PaddingParams, Tokenizer, TruncationParams};

    const MAX_TOKENS: usize = 512;
    /// Small batches keep padding (and memory) low on long lines.
    const BATCH: usize = 4;

    pub fn embed(dir: &Path, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let error = |error: &dyn std::fmt::Display| error.to_string();
        let device = Device::Cpu;
        let config: Config =
            serde_json::from_slice(&std::fs::read(dir.join("config.json")).map_err(|e| error(&e))?)
                .map_err(|e| error(&e))?;
        let mut tokenizer =
            Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| error(&e))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..TruncationParams::default()
            }))
            .map_err(|e| error(&e))?;
        tokenizer.with_padding(Some(PaddingParams::default()));
        // SAFETY: the weights file is our own download, verified by digest;
        // it is not modified while mapped.
        let weights = unsafe {
            VarBuilder::from_mmaped_safetensors(&[dir.join("model.safetensors")], DTYPE, &device)
                .map_err(|e| error(&e))?
        };
        let model = BertModel::load(weights, &config).map_err(|e| error(&e))?;

        let mut out = Vec::with_capacity(texts.len());
        for chunk in texts.chunks(BATCH) {
            let encodings = tokenizer
                .encode_batch(chunk.to_vec(), true)
                .map_err(|e| error(&e))?;
            let width = encodings
                .first()
                .map_or(0, |encoding| encoding.get_ids().len());
            let mut ids = Vec::with_capacity(chunk.len() * width);
            let mut mask = Vec::with_capacity(chunk.len() * width);
            for encoding in &encodings {
                ids.extend_from_slice(encoding.get_ids());
                mask.extend(encoding.get_attention_mask().iter().map(|&m| m as f32));
            }
            let input_ids = Tensor::new(ids.as_slice(), &device)
                .and_then(|tensor| tensor.reshape((chunk.len(), width)))
                .map_err(|e| error(&e))?;
            let token_types = input_ids.zeros_like().map_err(|e| error(&e))?;
            let attention = Tensor::new(mask.as_slice(), &device)
                .and_then(|tensor| tensor.reshape((chunk.len(), width)))
                .map_err(|e| error(&e))?;
            let hidden = model
                .forward(&input_ids, &token_types, Some(&attention))
                .map_err(|e| error(&e))?;
            for row in 0..chunk.len() {
                let cls = hidden
                    .i((row, 0))
                    .and_then(|tensor| tensor.to_dtype(DType::F32))
                    .and_then(|tensor| tensor.to_vec1::<f32>())
                    .map_err(|e| error(&e))?;
                let norm = cls.iter().map(|x| x * x).sum::<f32>().sqrt();
                let inverse = if norm > 0.0 { 1.0 / norm } else { 0.0 };
                out.push(cls.into_iter().map(|x| x * inverse).collect());
            }
        }
        Ok(out)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod bert {
    pub fn embed(_dir: &std::path::Path, _texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        Err("local embeddings are available on macOS only".into())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Fixed vectors, in input order.
    pub(crate) struct FixedEmbedder(pub Vec<Vec<f32>>);

    impl Embedder for FixedEmbedder {
        fn embed(&self, texts: Vec<String>) -> BoxFuture<'_, Option<Vec<Vec<f32>>>> {
            let vectors = self.0.iter().take(texts.len()).cloned().collect();
            Box::pin(async move { Some(vectors) })
        }
    }

    #[tokio::test]
    async fn a_cache_with_the_pinned_sizes_is_not_trusted_before_its_digests_match() {
        let dir = tempfile::tempdir().unwrap();
        let model_dir = dir.path().join(MODEL_DIR_NAME);
        std::fs::create_dir_all(&model_dir).unwrap();
        for file in &MODEL_FILES {
            // Right size, wrong bytes (sparse, so the test stays cheap).
            std::fs::File::create(model_dir.join(file.name))
                .unwrap()
                .set_len(file.size)
                .unwrap();
        }
        let embedder = LocalEmbedder::new(dir.path());
        assert_eq!(embedder.status(), EmbedderStatus::Absent);
        assert!(embedder.embed(vec!["text".into()]).await.is_none());
    }

    #[test]
    fn cached_files_whose_digest_does_not_match_are_deleted_for_download() {
        let dir = tempfile::tempdir().unwrap();
        let good = br#"{"hidden_size": 384}"#;
        let digest = |bytes: &[u8]| hex(&Sha256::digest(bytes));
        let (good_digest, expected_digest, none_digest) =
            (digest(good), digest(b"expected"), digest(b"none"));
        let files = [
            ModelFile {
                name: "config.json",
                size: good.len() as u64,
                sha256: &good_digest,
            },
            // Same size as the pinned bytes, different content.
            ModelFile {
                name: "tokenizer.json",
                size: 8,
                sha256: &expected_digest,
            },
            ModelFile {
                name: "model.safetensors",
                size: 4,
                sha256: &none_digest,
            },
        ];
        std::fs::write(dir.path().join("config.json"), good).unwrap();
        std::fs::write(dir.path().join("tokenizer.json"), b"replaced").unwrap();

        let names: Vec<&str> = unverified(dir.path(), &files)
            .into_iter()
            .map(|file| file.name)
            .collect();

        assert_eq!(names, vec!["tokenizer.json", "model.safetensors"]);
        assert!(dir.path().join("config.json").exists());
        assert!(!dir.path().join("tokenizer.json").exists());
        std::fs::write(dir.path().join("tokenizer.json"), b"expected").unwrap();
        assert!(verified(&dir.path().join("tokenizer.json"), &files[1]));
    }

    /// The real model: near-duplicates score above the dedup threshold and
    /// unrelated text below it. Needs the downloaded files; run with
    /// `CLOVY_TEST_EMBEDDER_MODELS=<dir containing bge-small-en-v1.5>`
    /// `cargo test day_intelligence::embedder -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn real_model_separates_near_duplicates_from_unrelated_text() {
        let models = std::env::var("CLOVY_TEST_EMBEDDER_MODELS").expect("models dir");
        let embedder = LocalEmbedder::new(Path::new(&models));
        embedder.ensure_ready().await;
        assert_eq!(embedder.status(), EmbedderStatus::Ready);
        let vectors = embedder
            .embed(vec![
                "Implemented the Rust session distiller today".into(),
                "Wrote the new session distiller in Rust".into(),
                "Cooked pasta and watched a film in the evening".into(),
            ])
            .await
            .expect("vectors");
        let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        assert_eq!(vectors[0].len(), 384);
        let near = dot(&vectors[0], &vectors[1]);
        let far = dot(&vectors[0], &vectors[2]);
        println!("near {near:.3} far {far:.3}");
        let threshold = crate::day_intelligence::distill::SEM_DEDUP_THRESHOLD;
        assert!(near > threshold, "near-duplicates must dedupe");
        assert!(far < threshold, "unrelated text must stay");
    }
}
