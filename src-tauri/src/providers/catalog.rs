//! Direct OpenRouter text catalog and the curated audio/media model lists.

use super::{BuiltinProvider, ModelMode, ProviderModelDto};
use crate::domain::types::AppError;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        LazyLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const CATALOG_TIMEOUT: Duration = Duration::from_secs(10);
const CATALOG_TTL: Duration = Duration::from_secs(600);
const MAX_CATALOG_BYTES: usize = 8 * 1024 * 1024;
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
static TEXT_CATALOG: LazyLock<TextCatalog> = LazyLock::new(|| {
    TextCatalog::new(
        OPENROUTER_MODELS_URL.to_string(),
        CATALOG_TIMEOUT,
        CATALOG_TTL,
    )
});

struct CachedModels {
    models: Vec<ProviderModelDto>,
    loaded_at: Instant,
    key_fingerprint: Option<[u8; 32]>,
    revision: u64,
}

struct TextCatalog {
    client: reqwest::Client,
    url: String,
    ttl: Duration,
    timeout: Duration,
    revision: AtomicU64,
    cache: Mutex<Option<CachedModels>>,
}

fn catalog_error() -> AppError {
    AppError::new(
        "openrouter_catalog_failed",
        "The OpenRouter model list could not be loaded. Try again.",
    )
}

impl TextCatalog {
    fn new(url: String, timeout: Duration, ttl: Duration) -> Self {
        Self {
            client: crate::http_client::build_client(
                crate::http_client::base_client_builder()
                    .timeout(timeout)
                    .redirect(reqwest::redirect::Policy::none()),
                "openrouter_catalog",
            ),
            url,
            ttl,
            timeout,
            revision: AtomicU64::new(0),
            cache: Mutex::new(None),
        }
    }

    fn invalidate(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    async fn load(&self, key: Option<&str>) -> Result<Vec<ProviderModelDto>, AppError> {
        tokio::time::timeout(self.timeout, self.load_with_cache(key))
            .await
            .map_err(|_| catalog_error())?
    }

    async fn load_with_cache(&self, key: Option<&str>) -> Result<Vec<ProviderModelDto>, AppError> {
        let key = key.map(str::trim).filter(|key| !key.is_empty());
        let key_fingerprint = key.map(|key| Sha256::digest(key.as_bytes()).into());
        // Serialize simultaneous picker requests so one refresh fills the cache.
        let mut cache = self.cache.lock().await;
        let revision = self.revision.load(Ordering::Acquire);
        if let Some(cached) = cache.as_ref() {
            if cached.revision == revision
                && cached.key_fingerprint == key_fingerprint
                && cached.loaded_at.elapsed() < self.ttl
            {
                return Ok(cached.models.clone());
            }
        }
        let mut request = self.client.get(&self.url);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let mut response = request.send().await.map_err(|_| catalog_error())?;
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|size| size > MAX_CATALOG_BYTES as u64)
        {
            return Err(catalog_error());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| catalog_error())? {
            if chunk.len() > MAX_CATALOG_BYTES.saturating_sub(body.len()) {
                return Err(catalog_error());
            }
            body.extend_from_slice(&chunk);
        }
        let models = map_text_models(&body)?;
        // A credential write during this fetch must not repopulate its old cache.
        if self.revision.load(Ordering::Acquire) == revision {
            *cache = Some(CachedModels {
                models: models.clone(),
                loaded_at: Instant::now(),
                key_fingerprint,
                revision,
            });
        }
        Ok(models)
    }
}

fn text_values(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}

fn price_per_million(value: &Value) -> Option<f64> {
    let price = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())?
        * 1_000_000.0;
    (price.is_finite() && price >= 0.0).then_some(price)
}

fn map_text_models(body: &[u8]) -> Result<Vec<ProviderModelDto>, AppError> {
    let document: Value = serde_json::from_slice(body).map_err(|_| catalog_error())?;
    let rows = document["data"].as_array().ok_or_else(catalog_error)?;
    Ok(rows
        .iter()
        .filter_map(|row| {
            if !text_values(&row["architecture"]["output_modalities"])
                .iter()
                .any(|value| value == "text")
            {
                return None;
            }
            let id = row["id"].as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            let name = row["name"]
                .as_str()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(id);
            let mut model = model_row("openrouter", id, name, "text");
            model.description = row["description"].as_str().map(str::to_string);
            model.context_tokens = row["context_length"].as_i64().filter(|value| *value > 0);
            model.capabilities = text_values(&row["supported_parameters"]);
            if model.capabilities.iter().any(|value| value == "tools") {
                model
                    .capabilities
                    .push("supportsFunctionCalling".to_string());
            }
            if text_values(&row["architecture"]["input_modalities"])
                .iter()
                .any(|value| value == "image")
            {
                model.capabilities.push("supportsVision".to_string());
            }
            let mut pricing = row["pricing"].as_object().cloned().unwrap_or_default();
            pricing.insert("unit".to_string(), json!("per_million_tokens"));
            if let Some(price) = price_per_million(&row["pricing"]["prompt"]) {
                pricing.insert("input".to_string(), json!({"usd": price}));
            }
            if let Some(price) = price_per_million(&row["pricing"]["completion"]) {
                pricing.insert("output".to_string(), json!({"usd": price}));
            }
            model.pricing = Some(Value::Object(pricing));
            model.price_unit = "tokens".to_string();
            Some(model)
        })
        .collect())
}

fn model_row(provider: &str, id: &str, name: &str, model_type: &str) -> ProviderModelDto {
    ProviderModelDto {
        available: None,
        provider: provider.to_string(),
        id: id.to_string(),
        name: name.to_string(),
        model_type: model_type.to_string(),
        description: None,
        privacy: None,
        pricing: None,
        context_tokens: None,
        traits: vec![],
        capabilities: vec![],
        price_unit: model_type.to_string(),
        price_description: String::new(),
    }
}

pub(super) async fn models(mode: ModelMode) -> Result<Vec<ProviderModelDto>, AppError> {
    let (provider, ids) = match mode {
        ModelMode::Generation => {
            let key = super::provider_api_key(BuiltinProvider::OpenRouter)?;
            return TEXT_CATALOG.load(key.as_deref()).await;
        }
        ModelMode::Transcription => ("groq", super::KNOWN_TRANSCRIPTION_MODELS),
        ModelMode::Image => ("fal", super::KNOWN_IMAGE_MODELS),
        ModelMode::Video => ("fal", super::KNOWN_VIDEO_MODELS),
    };
    let mut rows: Vec<_> = ids
        .iter()
        .map(|id| {
            let name = match *id {
                "whisper-large-v3-turbo" => "Whisper large v3 turbo",
                "whisper-large-v3" => "Whisper large v3",
                "fal-ai/flux/dev" => "FLUX.1 [dev]",
                "fal-ai/flux/schnell" => "FLUX.1 [schnell]",
                "fal-ai/qwen-image" => "Qwen Image",
                "fal-ai/wan/v2.2-a14b/text-to-video" => "Wan 2.2 A14B",
                "alibaba/wan-3.0/text-to-video" => "Wan 3.0",
                value => value,
            };
            model_row(provider, id, name, mode.api_type())
        })
        .collect();
    if mode == ModelMode::Transcription && super::local_transcription_supported() {
        let mut local = model_row(
            "local",
            super::transcription::LOCAL_MODEL,
            "Parakeet Ultra",
            mode.api_type(),
        );
        local.available = Some(super::transcription::ready_directory().is_some());
        local.privacy = Some("local".into());
        rows.push(local);
    }
    Ok(rows)
}

pub(super) fn invalidate() {
    TEXT_CATALOG.invalidate();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    struct Server {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn server(status: u16, body: serde_json::Value, delay: Duration) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api/v1/models", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let task = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut buffer = [0; 1024];
                    let count = stream.read(&mut buffer).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 16 * 1024);
                }
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8(request).unwrap());
                tokio::time::sleep(delay).await;
                let payload = body.to_string();
                let response = format!("HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len());
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        Server {
            url,
            requests,
            task,
        }
    }

    fn fixture() -> serde_json::Value {
        json!({"data": [
            {"id":"vendor/plain","name":"Plain text","description":"Text without tools",
             "context_length":32768,"pricing":{"prompt":"0.0000003","completion":"0.0000025"},
             "supported_parameters":["temperature"],
             "architecture":{"input_modalities":["text"],"output_modalities":["text"]}},
            {"id":"vendor/vision","name":"Vision and tools","context_length":128000,
             "pricing":{"prompt":"0","completion":"0"},"supported_parameters":["tools"],
             "architecture":{"input_modalities":["text","image"],"output_modalities":["text"]}},
            {"id":"vendor/mixed","name":"Mixed output","supported_parameters":[],
             "architecture":{"input_modalities":["text"],"output_modalities":["image","text"]}},
            {"id":"vendor/image","name":"Image only",
             "architecture":{"input_modalities":["text"],"output_modalities":["image"]}}
        ]})
    }

    #[tokio::test]
    async fn text_catalog_maps_every_text_output_model_without_requiring_tools() {
        let server = server(200, fixture(), Duration::ZERO).await;
        let catalog = TextCatalog::new(server.url.clone(), CATALOG_TIMEOUT, CATALOG_TTL);
        let models = catalog.load(Some("fixture-openrouter-key")).await.unwrap();
        assert_eq!(models.len(), 3);
        let plain = models
            .iter()
            .find(|model| model.id == "vendor/plain")
            .unwrap();
        assert_eq!(plain.name, "Plain text");
        assert_eq!(plain.context_tokens, Some(32768));
        assert_eq!(plain.provider, "openrouter");
        assert_eq!(plain.model_type, "text");
        assert!(plain.capabilities.contains(&"temperature".to_string()));
        assert!(!plain
            .capabilities
            .contains(&"supportsFunctionCalling".to_string()));
        let pricing = plain.pricing.as_ref().unwrap();
        assert_eq!(pricing["unit"], "per_million_tokens");
        assert!((pricing["input"]["usd"].as_f64().unwrap() - 0.3).abs() < 0.000001);
        assert_eq!(pricing["output"]["usd"], 2.5);
        assert_eq!(plain.privacy, None);
        let vision = models
            .iter()
            .find(|model| model.id == "vendor/vision")
            .unwrap();
        assert!(vision.capabilities.contains(&"supportsVision".to_string()));
        assert!(vision
            .capabilities
            .contains(&"supportsFunctionCalling".to_string()));
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with("GET /api/v1/models HTTP/1.1"));
        assert!(requests[0]
            .to_ascii_lowercase()
            .contains("authorization: bearer fixture-openrouter-key\r\n"));
        assert!(!requests[0].contains("x-openrouter-api-key"));
    }

    #[tokio::test]
    async fn text_catalog_cache_reloads_after_key_change_save_clear_and_expiry() {
        let server = server(200, fixture(), Duration::ZERO).await;
        let catalog = TextCatalog::new(server.url.clone(), CATALOG_TIMEOUT, CATALOG_TTL);
        catalog.load(None).await.unwrap();
        catalog.load(None).await.unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), 1);
        catalog.load(Some("first-fixture-key")).await.unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), 2);
        catalog.invalidate();
        catalog.load(Some("first-fixture-key")).await.unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), 3);
        catalog.load(None).await.unwrap();
        assert_eq!(server.requests.lock().unwrap().len(), 4);
        let expired_catalog = TextCatalog::new(server.url.clone(), CATALOG_TIMEOUT, Duration::ZERO);
        expired_catalog.load(None).await.unwrap();
        expired_catalog.load(None).await.unwrap();
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 6);
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
        assert!(!requests[3].to_ascii_lowercase().contains("authorization:"));
    }

    #[tokio::test]
    async fn text_catalog_failure_is_actionable_and_not_cached() {
        let server = server(
            500,
            json!({"error":"private upstream detail"}),
            Duration::ZERO,
        )
        .await;
        let catalog = TextCatalog::new(server.url.clone(), CATALOG_TIMEOUT, CATALOG_TTL);
        for _ in 0..2 {
            let error = catalog.load(None).await.unwrap_err();
            assert_eq!(error.code, "openrouter_catalog_failed");
            assert!(!error.message.contains("private upstream detail"));
        }
        assert_eq!(server.requests.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn text_catalog_has_a_bounded_timeout() {
        assert_eq!(CATALOG_TIMEOUT, Duration::from_secs(10));
        assert_eq!(CATALOG_TTL, Duration::from_secs(600));
        let server = server(200, fixture(), Duration::from_secs(1)).await;
        let catalog = TextCatalog::new(server.url.clone(), Duration::from_millis(30), CATALOG_TTL);
        let error = tokio::time::timeout(Duration::from_millis(500), catalog.load(None))
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "openrouter_catalog_failed");
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn text_catalog_deadline_includes_waiting_for_another_refresh() {
        let server = server(200, fixture(), Duration::from_secs(1)).await;
        let catalog = TextCatalog::new(server.url.clone(), Duration::from_millis(30), CATALOG_TTL);
        // A concurrent refresh owns this lock while its upstream is stalled.
        let _refresh = catalog.cache.lock().await;
        let error = tokio::time::timeout(
            Duration::from_millis(300),
            catalog.load(Some("new-fixture-key")),
        )
        .await
        .expect("the catalog deadline must include waiting for the old refresh")
        .unwrap_err();
        assert_eq!(error.code, "openrouter_catalog_failed");
        assert!(server.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn local_transcription_catalog_includes_unavailable_model_only_on_supported_platforms() {
        let catalog = models(ModelMode::Transcription).await.unwrap();
        let local = catalog.iter().find(|model| model.id == "parakeet-ultra");
        if super::super::local_transcription_supported() {
            let local = local.expect("local choice remains visible before installation");
            assert_eq!(local.provider, "local");
            assert_eq!(serde_json::to_value(local).unwrap()["available"], false);
        } else {
            assert!(local.is_none());
        }
    }

    #[tokio::test]
    async fn curated_catalogs_are_local_and_have_no_service_availability() {
        for (mode, expected) in [
            (
                ModelMode::Transcription,
                super::super::KNOWN_TRANSCRIPTION_MODELS,
            ),
            (ModelMode::Image, super::super::KNOWN_IMAGE_MODELS),
            (ModelMode::Video, super::super::KNOWN_VIDEO_MODELS),
        ] {
            let models: Vec<_> = models(mode)
                .await
                .unwrap()
                .into_iter()
                .filter(|row| row.provider != "local")
                .collect();
            assert_eq!(
                models
                    .iter()
                    .map(|model| model.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(models.iter().all(|model| {
                let value = serde_json::to_value(model).unwrap();
                value.get("serviceAvailable").is_none()
                    && value.get("inputCreditsPerMillionTokens").is_none()
            }));
        }
    }
}
