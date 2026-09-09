//! Venice "augment" web tools: standalone web search and single-URL scrape.
//!
//! Venice exposes both behind the same inference boundary the rest of the app
//! already routes through, so the agent gets web context without adding a
//! third-party processor. The endpoints are flagged experimental upstream
//! ("request and response format may change without notice"), so the wire
//! shapes are confined to this one file: a schema change here is a one-file fix
//! and never leaks into the domain types or the agent.

use crate::{
    retry::{self, UpstreamAttemptError},
    venice::{PROVIDER_NAME, user_venice_key_auth_error},
};
use async_trait::async_trait;
use clovy_config::UpstreamConfig;
use clovy_domain::{
    DomainError, ProviderCredentials, WebFetchRequest, WebFetchResult, WebFetcher,
    WebSearchProvider, WebSearchRequest, WebSearchResult, WebSearchResults, WebSearcher,
};
use reqwest::StatusCode;
use serde::{Serialize, de::DeserializeOwned};

/// Read-only client for Venice's `/augment/search` and `/augment/scrape`
/// endpoints. One struct backs both the `WebSearcher` and `WebFetcher` traits
/// because they share the base URL and credential.
pub struct VeniceAugment {
    http: reqwest::Client,
    api_key: String,
    base_url: String,
    byok_base_url: String,
}

impl VeniceAugment {
    pub fn from_config(http: reqwest::Client, config: &UpstreamConfig) -> Self {
        Self {
            http,
            api_key: config.api_key.clone(),
            base_url: config.base_url.trim_end_matches('/').to_string(),
            byok_base_url: crate::venice::byok_base_url(config),
        }
    }

    /// POSTs `body` to an augment endpoint and parses the response.
    ///
    /// A `400` is the upstream rejecting this specific input (a query it won't
    /// run, or a site that blocks automated access); it is surfaced as
    /// `InvalidInput` with `client_error_reason` so the caller can answer the
    /// agent with a usable 400. Every other non-2xx (including auth/billing
    /// 401/403/402) is our problem, not the user's, so it surfaces as a
    /// provider failure; transient ones (timeout, 429, 5xx) get one bounded
    /// retry, matching the chat path.
    async fn post_augment<B, T>(
        &self,
        endpoint: AugmentEndpoint,
        body: &B,
        provider_credentials: &ProviderCredentials,
    ) -> Result<T, DomainError>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let base_url = crate::venice::request_base_url(
            &self.base_url,
            &self.byok_base_url,
            provider_credentials,
        );
        let endpoint = ResolvedAugmentEndpoint {
            url: format!("{}{}", base_url, endpoint.path),
            client_error_reason: endpoint.client_error_reason,
        };
        for attempt in 0..retry::UPSTREAM_ATTEMPTS {
            let error = match self
                .post_augment_once(&endpoint, body, provider_credentials)
                .await
            {
                Ok(parsed) => return Ok(parsed),
                Err(error) => error,
            };
            if attempt + 1 < retry::UPSTREAM_ATTEMPTS
                && let Some(delay) = error.retry_delay
            {
                tracing::warn!(url = %endpoint.url, attempt, "venice augment: transient failure, retrying");
                tokio::time::sleep(delay).await;
                continue;
            }
            return Err(error.error);
        }
        Err(DomainError::UpstreamProvider)
    }

    async fn post_augment_once<B, T>(
        &self,
        endpoint: &ResolvedAugmentEndpoint,
        body: &B,
        provider_credentials: &ProviderCredentials,
    ) -> Result<T, UpstreamAttemptError>
    where
        B: Serialize,
        T: DeserializeOwned,
    {
        let api_key = venice_api_key(&self.api_key, provider_credentials);
        let response = self
            .http
            .post(&endpoint.url)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await
            .map_err(|error| {
                let retryable = retry::is_retryable_transport_error(&error);
                tracing::error!(%error, url = %endpoint.url, retryable, "venice augment: transport error");
                UpstreamAttemptError {
                    error: DomainError::UpstreamProvider,
                    retry_delay: retryable.then_some(retry::UPSTREAM_RETRY_BACKOFF),
                }
            })?;
        let status = response.status();
        if !status.is_success() {
            let retry_delay = retry::response_retry_delay(status, response.headers());
            let retryable = retry::is_retryable_status(status);
            let body_text = response.text().await.unwrap_or_default();
            tracing::error!(%status, url = %endpoint.url, body_bytes = body_text.len(), retryable, retry_allowed = retry_delay.is_some(), "venice augment: non-success response");
            // Only a 400 means the upstream rejected this specific input.
            // Auth/billing/config failures (401, 403, 402, ...) are ours, not
            // the user's, and must stay provider failures so they are not
            // reported to the agent as a fixable bad request.
            if status == StatusCode::BAD_REQUEST {
                return Err(UpstreamAttemptError::fatal(DomainError::InvalidInput {
                    reason: endpoint.client_error_reason.to_string(),
                }));
            }
            if let Some(error) = user_venice_key_auth_error(status, provider_credentials) {
                return Err(UpstreamAttemptError::fatal(error));
            }
            return Err(UpstreamAttemptError {
                error: DomainError::UpstreamProvider,
                retry_delay,
            });
        }
        response.json::<T>().await.map_err(|error| {
            // A body we can't parse means the experimental schema drifted (or
            // an upstream error leaked through as 200). Fail closed rather than
            // hand the agent half-parsed data.
            tracing::error!(%error, url = %endpoint.url, "venice augment: response JSON parse failed");
            UpstreamAttemptError::fatal(DomainError::UpstreamProvider)
        })
    }
}

#[async_trait]
impl WebSearcher for VeniceAugment {
    async fn search(&self, request: WebSearchRequest) -> Result<WebSearchResults, DomainError> {
        let provider = request.provider;
        let wire: AugmentSearchResponse = self
            .post_augment(
                AugmentEndpoint {
                    path: "/augment/search",
                    client_error_reason: "web_search_rejected",
                },
                &AugmentSearchRequest {
                    query: &request.query,
                    limit: request.limit,
                    search_provider: search_provider_param(provider),
                },
                &request.provider_credentials,
            )
            .await?;
        let results = wire
            .results
            .into_iter()
            // A result with no URL is unusable as a citation; drop it rather
            // than surface a dead row.
            .filter(|result| !result.url.trim().is_empty())
            .map(|result| WebSearchResult {
                title: non_empty(result.title).unwrap_or_else(|| result.url.clone()),
                url: result.url,
                snippet: non_empty(result.content.unwrap_or_default()),
                published_at: non_empty(result.date.unwrap_or_default()),
            })
            .collect();
        Ok(WebSearchResults {
            query: request.query,
            provider: search_provider_param(provider).to_string(),
            results,
        })
    }
}

#[async_trait]
impl WebFetcher for VeniceAugment {
    async fn fetch(&self, request: WebFetchRequest) -> Result<WebFetchResult, DomainError> {
        let wire: AugmentScrapeResponse = self
            .post_augment(
                AugmentEndpoint {
                    path: "/augment/scrape",
                    client_error_reason: "web_fetch_unsupported_url",
                },
                &AugmentScrapeRequest { url: &request.url },
                &request.provider_credentials,
            )
            .await?;
        Ok(WebFetchResult {
            url: non_empty(wire.url).unwrap_or(request.url),
            content: wire.content,
            format: non_empty(wire.format.unwrap_or_default())
                .unwrap_or_else(|| "markdown".to_string()),
            provider: PROVIDER_NAME.to_string(),
        })
    }
}

fn search_provider_param(provider: WebSearchProvider) -> &'static str {
    match provider {
        WebSearchProvider::Brave => "brave",
        WebSearchProvider::Google => "google",
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn venice_api_key<'a>(configured: &'a str, credentials: &'a ProviderCredentials) -> &'a str {
    credentials
        .venice_api_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(configured)
}

#[derive(Clone, Copy)]
struct AugmentEndpoint {
    path: &'static str,
    client_error_reason: &'static str,
}

struct ResolvedAugmentEndpoint {
    url: String,
    client_error_reason: &'static str,
}

// --- Venice augment wire shapes (experimental; keep confined to this file) ---

#[derive(Serialize)]
struct AugmentSearchRequest<'a> {
    query: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u32>,
    search_provider: &'a str,
}

#[derive(serde::Deserialize)]
struct AugmentSearchResponse {
    #[serde(default)]
    results: Vec<AugmentSearchResultWire>,
}

#[derive(serde::Deserialize)]
struct AugmentSearchResultWire {
    #[serde(default)]
    title: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Serialize)]
struct AugmentScrapeRequest<'a> {
    url: &'a str,
}

#[derive(serde::Deserialize)]
struct AugmentScrapeResponse {
    #[serde(default)]
    url: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    format: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::VeniceAugment;
    use crate::http;
    use clovy_config::UpstreamConfig;
    use clovy_domain::{
        DomainError, ProviderCredentials, WebFetchRequest, WebFetcher, WebSearchProvider,
        WebSearchRequest, WebSearcher,
    };
    use pretty_assertions::assert_eq;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_string_contains, header, method, path},
    };

    fn augment(server: &MockServer) -> VeniceAugment {
        VeniceAugment::from_config(
            http::default_client(),
            &UpstreamConfig {
                api_key: "venice_key".to_string(),
                base_url: server.uri(),
                byok_base_url: None,
            },
        )
    }

    #[tokio::test]
    async fn byok_search_routes_direct_to_byok_base_url() {
        // A request carrying a user's own Venice key must go to the BYOK base;
        // the configured base_url may be a Clovy-managed gateway that rejects
        // any bearer other than Clovy's service key.
        let byok_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/augment/search"))
            .and(header("authorization", "Bearer user_venice_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "query": "rust async",
                "results": []
            })))
            .expect(1)
            .mount(&byok_server)
            .await;
        let augment = VeniceAugment::from_config(
            http::default_client(),
            &UpstreamConfig {
                api_key: "venice_key".to_string(),
                base_url: "http://127.0.0.1:9".to_string(),
                byok_base_url: Some(byok_server.uri()),
            },
        );

        let results = augment
            .search(WebSearchRequest {
                query: "rust async".to_string(),
                limit: Some(5),
                provider: WebSearchProvider::Brave,
                provider_credentials: ProviderCredentials {
                    venice_api_key: Some("user_venice_key".to_string()),
                },
            })
            .await
            .expect("search should succeed");

        assert_eq!(results.results.len(), 0);
    }

    #[tokio::test]
    async fn search_sends_query_and_parses_results() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/augment/search"))
            .and(header("authorization", "Bearer venice_key"))
            .and(body_string_contains("\"query\":\"rust async\""))
            .and(body_string_contains("\"search_provider\":\"brave\""))
            .and(body_string_contains("\"limit\":5"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "query": "rust async",
                "results": [
                    {
                        "title": "Async Rust",
                        "url": "https://example.com/async",
                        "content": "A guide to async.",
                        "date": "2026-01-02"
                    },
                    {
                        "title": "",
                        "url": "https://example.com/no-title",
                        "content": null
                    },
                    {
                        "title": "Dropped",
                        "url": "   "
                    }
                ]
            })))
            .mount(&server)
            .await;

        let results = augment(&server)
            .search(WebSearchRequest {
                query: "rust async".to_string(),
                limit: Some(5),
                provider: WebSearchProvider::Brave,
                provider_credentials: ProviderCredentials::default(),
            })
            .await
            .expect("search should succeed");

        assert_eq!(results.provider, "brave");
        // The blank-URL row is dropped; the blank-title row falls back to URL.
        assert_eq!(results.results.len(), 2);
        assert_eq!(results.results[0].title, "Async Rust");
        assert_eq!(
            results.results[0].snippet.as_deref(),
            Some("A guide to async.")
        );
        assert_eq!(
            results.results[0].published_at.as_deref(),
            Some("2026-01-02")
        );
        assert_eq!(results.results[1].title, "https://example.com/no-title");
        assert_eq!(results.results[1].snippet, None);
    }

    #[tokio::test]
    async fn search_defaults_provider_param_to_google_when_selected() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/augment/search"))
            .and(body_string_contains("\"search_provider\":\"google\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": [] })))
            .mount(&server)
            .await;

        let results = augment(&server)
            .search(WebSearchRequest {
                query: "anything".to_string(),
                limit: None,
                provider: WebSearchProvider::Google,
                provider_credentials: ProviderCredentials::default(),
            })
            .await
            .expect("search should succeed");

        assert_eq!(results.provider, "google");
        assert!(results.results.is_empty());
    }

    #[tokio::test]
    async fn fetch_returns_markdown() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/augment/scrape"))
            .and(header("authorization", "Bearer venice_key"))
            .and(body_string_contains("\"url\":\"https://example.com/post\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "url": "https://example.com/post",
                "content": "# Heading\n\nBody text.",
                "format": "markdown"
            })))
            .mount(&server)
            .await;

        let fetched = augment(&server)
            .fetch(WebFetchRequest {
                url: "https://example.com/post".to_string(),
                provider_credentials: ProviderCredentials::default(),
            })
            .await
            .expect("fetch should succeed");

        assert_eq!(fetched.url, "https://example.com/post");
        assert_eq!(fetched.content, "# Heading\n\nBody text.");
        assert_eq!(fetched.format, "markdown");
        assert_eq!(fetched.provider, "venice");
    }

    #[tokio::test]
    async fn fetch_blocked_site_maps_to_invalid_input() {
        let server = MockServer::start().await;
        // Venice rejects bot-walled sites (X/Twitter, Reddit, ...) with a 400.
        Mock::given(method("POST"))
            .and(path("/augment/scrape"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "This site blocks automated access."
            })))
            .mount(&server)
            .await;

        let error = augment(&server)
            .fetch(WebFetchRequest {
                url: "https://x.com/some/post".to_string(),
                provider_credentials: ProviderCredentials::default(),
            })
            .await
            .expect_err("blocked site should error");

        assert_eq!(
            error,
            DomainError::InvalidInput {
                reason: "web_fetch_unsupported_url".to_string()
            }
        );
    }

    #[tokio::test]
    async fn fetch_auth_failure_maps_to_upstream_not_invalid_input() {
        let server = MockServer::start().await;
        // A bad or expired Venice credential (401/403) is our problem, not the
        // user's input; it must not be reported to the agent as a fixable 400.
        Mock::given(method("POST"))
            .and(path("/augment/scrape"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "Invalid API key"
            })))
            .mount(&server)
            .await;

        let error = augment(&server)
            .fetch(WebFetchRequest {
                url: "https://example.com/post".to_string(),
                provider_credentials: ProviderCredentials::default(),
            })
            .await
            .expect_err("auth failure should error");

        assert_eq!(error, DomainError::UpstreamProvider);
    }

    #[tokio::test]
    async fn search_malformed_schema_fails_closed() {
        let server = MockServer::start().await;
        // `results` arriving as a string is the kind of drift the experimental
        // flag warns about; we fail closed rather than hand back junk.
        Mock::given(method("POST"))
            .and(path("/augment/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "results": "nope" })))
            .mount(&server)
            .await;

        let error = augment(&server)
            .search(WebSearchRequest {
                query: "anything".to_string(),
                limit: None,
                provider: WebSearchProvider::Brave,
                provider_credentials: ProviderCredentials::default(),
            })
            .await
            .expect_err("malformed schema should error");

        assert_eq!(error, DomainError::UpstreamProvider);
    }

    #[tokio::test]
    async fn search_retries_then_fails_on_upstream_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/augment/search"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let error = augment(&server)
            .search(WebSearchRequest {
                query: "anything".to_string(),
                limit: None,
                provider: WebSearchProvider::Brave,
                provider_credentials: ProviderCredentials::default(),
            })
            .await
            .expect_err("a 5xx should surface as an upstream failure");

        assert_eq!(error, DomainError::UpstreamProvider);
    }
}
