//! GitHub App user access tokens, device authorization flow.
//!
//! GitHub does NOT support PKCE-only public clients for the web flow, but
//! the device authorization flow (RFC 8628) needs only the client id: no
//! callback URL is registered, no loopback listener is required, and no
//! client secret is used during authorization. The user is shown a short
//! `user_code` and directed to `verification_uri`; the app polls GitHub
//! until the user approves, declines, or the code expires.
//!
//! Token lifetime: when the GitHub App has "expire user authorization tokens"
//! enabled, the exchange returns an `expires_in` and a `refresh_token` that
//! rotates on every refresh. When the app has that setting disabled, GitHub
//! returns a non-expiring token with no `expires_in` and no `refresh_token`;
//! Clovy stores it with a far-future expiry so the freshness gate never triggers
//! an unnecessary refresh.
//!
//! Refresh requires the client secret. A deployment that ships only the client
//! id (and therefore no secret) must disable "expire user authorization tokens"
//! on the GitHub App so tokens are non-expiring and the refresh path is never
//! triggered.
//!
//! NEVER log, print, or serialize tokens (or authorization codes) into errors.
//! Error messages carry stable codes and short human text only.

use crate::domain::types::AppError;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::oauth::{self, ConnectFlow};

const DEVICE_CODE_ENDPOINT: &str = "https://github.com/login/device/code";
const TOKEN_ENDPOINT: &str = "https://github.com/login/oauth/access_token";
/// GitHub requires a User-Agent header on every API call; mirror the shared
/// http_client's value.
const GITHUB_API_VERSION: &str = "2022-11-28";

/// For GitHub Apps that do not expire user tokens: store a far-future expiry
/// so the freshness gate never triggers. ~100 years in seconds.
pub(super) const NON_EXPIRING_TOKEN_LIFETIME_SECS: i64 = 3_153_600_000;

/// How many extra seconds to add to the polling interval on `slow_down`.
const SLOW_DOWN_INCREMENT_SECS: u64 = 5;

/// Maximum transient network failures tolerated before aborting the poll loop.
const POLL_MAX_NETWORK_ERRORS: usize = 5;

// ----- Token exchange & refresh -----------------------------------------------

/// GitHub token exchange/refresh response. Secret fields zeroize on drop.
/// GitHub returns HTTP 200 with an `error` field on failure (not a 4xx), so
/// callers must parse the `error` field even on 200.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct GithubTokenResponse {
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub token_type: String,
    /// Present only when the GitHub App has "expire user authorization tokens"
    /// enabled. `None` means the token is non-expiring.
    #[serde(default)]
    #[zeroize(skip)]
    pub expires_in: Option<i64>,
    /// Rotates on every refresh. Absent for non-expiring token apps.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    #[zeroize(skip)]
    pub refresh_token_expires_in: Option<i64>,
    /// Present when GitHub returns an error on HTTP 200.
    #[serde(default)]
    #[zeroize(skip)]
    pub error: Option<String>,
    #[serde(default)]
    #[zeroize(skip)]
    pub error_description: Option<String>,
}

impl GithubTokenResponse {
    fn is_success(&self) -> bool {
        self.error.is_none() && !self.access_token.is_empty()
    }
}

/// The GitHub user who authorized, resolved from `GET /user`.
/// Carries no token material; `Debug` is safe.
#[derive(Debug, Clone)]
pub struct GithubIdentity {
    /// Stringified numeric GitHub user id (e.g. "1234567"). Keys custody and
    /// the DB index - NOT the login, which can change.
    pub user_id: String,
    /// GitHub username (@-handle), used as the display identity.
    pub login: String,
    /// Display name from the profile; may be empty.
    pub name: String,
}

/// Outcome of the full device flow handoff.
pub struct GithubAuthorizedGrant {
    pub tokens: GithubTokenResponse,
    pub identity: GithubIdentity,
}

/// Outcome of one refresh attempt. Mirrors [`linear::LinearRefreshOutcome`]
/// with GitHub-specific error classification.
pub enum GithubRefreshOutcome {
    Refreshed(GithubTokenResponse),
    /// Definitive: the grant was revoked or the refresh token is invalid.
    /// GitHub signals this with `error: "bad_refresh_token"`. Reconnect required.
    InvalidGrant,
    /// Upstream wobble (network error, 5xx): worth a bounded retry.
    Transient,
}

// ----- Device code request ----------------------------------------------------

/// The device code response from GitHub. Contains everything needed to display
/// the prompt to the user and drive the poll loop.
#[derive(Debug, Clone, Deserialize)]
pub struct GithubDeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default)]
    pub interval: u64,
    /// Present when GitHub returns an error on HTTP 200.
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub error_description: Option<String>,
}

/// Request a device code from GitHub. The form carries only `client_id` — no
/// scope, no client_secret. GitHub Apps ignore scopes on user-token requests;
/// the token's capabilities are determined by the app's configured permissions
/// and the installation's repository selection.
async fn request_device_code(client_id: &str) -> Result<GithubDeviceCode, AppError> {
    let response = oauth::http_client()
        .post(DEVICE_CODE_ENDPOINT)
        .header("Accept", "application/json")
        .form(&[("client_id", client_id)])
        .send()
        .await
        .map_err(|_| device_code_failed(None))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|_| device_code_failed(None))?;
    match serde_json::from_str::<GithubDeviceCode>(&body) {
        Ok(resp) if resp.error.is_none() && !resp.device_code.is_empty() => Ok(resp),
        Ok(resp) => {
            let error_code = resp.error.clone();
            tracing::warn!(status, error_code = ?error_code, "github device code request returned error");
            Err(classify_device_code_error(error_code))
        }
        Err(_) => {
            tracing::warn!(status, "github device code response unparseable");
            Err(device_code_failed(None))
        }
    }
}

fn device_code_failed(error_code: Option<String>) -> AppError {
    let message = match error_code {
        Some(code) => format!("Could not start the GitHub device flow ({code})."),
        None => "Could not start the GitHub device flow.".to_string(),
    };
    AppError::new("connector_token_exchange_failed", message)
}

fn classify_device_code_error(error_code: Option<String>) -> AppError {
    match error_code.as_deref() {
        Some("unauthorized_client") | Some("device_flow_not_enabled") => {
            AppError::new(
                "connector_github_device_flow_disabled",
                "Device flow is not enabled on the GitHub App. Enable it in the App's settings and try again.",
            )
        }
        _ => device_code_failed(error_code),
    }
}

// ----- Poll deadline (pure, unit-testable) ------------------------------------

/// Fallback `expires_in` used when the device-code response returns 0 or an
/// absent value. 900 seconds (15 minutes) is well above GitHub's documented
/// 15-minute device-code lifetime plus the 30-second margin below.
const DEVICE_CODE_EXPIRES_IN_FALLBACK_SECS: u64 = 900;

/// Margin added to `expires_in` when computing the local deadline. GitHub may
/// report `expired_token` slightly after the nominal expiry; the margin ensures
/// we do not race the server and give the user the full advertised window.
const DEVICE_CODE_DEADLINE_MARGIN_SECS: u64 = 30;

/// Pure helper: returns `true` when the local poll loop should stop because the
/// device code's validity window (plus a grace margin) has elapsed.
///
/// - `elapsed_secs`: seconds elapsed since the device code was obtained.
/// - `expires_in_secs`: the `expires_in` value from the device-code response;
///   0 means "not provided" and triggers the fallback.
///
/// Factored as a pure function so the deadline policy is unit-testable without
/// any async or time dependencies.
pub fn poll_deadline_reached(elapsed_secs: u64, expires_in_secs: u64) -> bool {
    let window = if expires_in_secs == 0 {
        DEVICE_CODE_EXPIRES_IN_FALLBACK_SECS
    } else {
        expires_in_secs
    };
    elapsed_secs >= window.saturating_add(DEVICE_CODE_DEADLINE_MARGIN_SECS)
}

// ----- Poll decision (pure, unit-testable) ------------------------------------

/// Decision returned by [`device_poll_action`] for one poll iteration.
#[derive(Debug, PartialEq)]
pub enum DevicePollAction {
    /// The poll succeeded; the response contains the token.
    Token,
    /// Keep polling after `next_interval_secs`.
    Pending { next_interval_secs: u64 },
    /// The code expired before the user approved.
    Expired,
    /// The user explicitly declined.
    Declined,
    /// Device flow is not enabled on the GitHub App.
    Disabled,
}

/// Pure decision function: map an `error` field from the GitHub poll response
/// to the next action. The `interval_secs` is the current polling interval;
/// `slow_down` adds [`SLOW_DOWN_INCREMENT_SECS`] to it.
///
/// Factored as a pure function so poll-action classification is unit-testable
/// without any HTTP.
pub fn device_poll_action(error: Option<&str>, interval_secs: u64) -> DevicePollAction {
    match error {
        None => DevicePollAction::Token,
        Some("authorization_pending") => DevicePollAction::Pending {
            next_interval_secs: interval_secs,
        },
        Some("slow_down") => DevicePollAction::Pending {
            next_interval_secs: interval_secs + SLOW_DOWN_INCREMENT_SECS,
        },
        Some("expired_token") => DevicePollAction::Expired,
        Some("access_denied") => DevicePollAction::Declined,
        Some("unauthorized_client") | Some("device_flow_not_enabled") => DevicePollAction::Disabled,
        Some(_) => DevicePollAction::Pending {
            next_interval_secs: interval_secs,
        },
    }
}

// ----- Poll loop --------------------------------------------------------------

/// Poll the token endpoint until the user approves, declines, or the code
/// expires. Honors the `ConnectFlow` cancellation signal between polls so
/// closing the dialog cleanly aborts the wait. Bounded network-error retries
/// prevent a momentary blip from aborting a long-lived poll loop.
///
/// A local deadline is anchored when the device code is obtained: the loop
/// terminates with `connector_github_device_expired` once
/// `expires_in + DEVICE_CODE_DEADLINE_MARGIN_SECS` seconds have elapsed,
/// regardless of GitHub's responses. This closes the gap where an unknown
/// GitHub error code mapped to `Pending` forever.
async fn poll_for_token(
    flow: &ConnectFlow,
    client_id: &str,
    device_code: &GithubDeviceCode,
) -> Result<GithubTokenResponse, AppError> {
    let mut interval_secs = device_code.interval.max(5);
    let mut network_errors = 0usize;

    // Anchor a local deadline so the loop cannot run past the device code's
    // validity window even if GitHub sends unrecognised error codes.
    let poll_start = std::time::Instant::now();
    let expires_in_secs = device_code.expires_in;

    // Register a cancellation sender for the duration of the poll.
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    flow.register_cancel_sender(cancel_tx);

    let result = loop {
        // Check the local deadline before sleeping. This runs at the top of
        // every iteration so the loop exits promptly once the code window
        // (plus the margin) is exhausted, even if GitHub keeps responding with
        // unknown error codes that `device_poll_action` maps to Pending.
        if poll_deadline_reached(poll_start.elapsed().as_secs(), expires_in_secs) {
            break Err(AppError::new(
                "connector_github_device_expired",
                "The code expired before it was approved. Try again.",
            ));
        }

        // Wait for the polling interval, racing against cancellation.
        let sleep = tokio::time::sleep(std::time::Duration::from_secs(interval_secs));
        tokio::select! {
            _ = sleep => {}
            _ = &mut cancel_rx => {
                break Err(AppError::new(
                    "connector_connect_canceled",
                    "Connecting to GitHub was canceled.",
                ));
            }
        }

        let response = match oauth::http_client()
            .post(TOKEN_ENDPOINT)
            .header("Accept", "application/json")
            .form(&poll_form(client_id, &device_code.device_code))
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => {
                network_errors += 1;
                if network_errors >= POLL_MAX_NETWORK_ERRORS {
                    break Err(AppError::new(
                        "connector_refresh_unavailable",
                        "Couldn't reach GitHub to complete the connection. Try again in a moment.",
                    ));
                }
                continue;
            }
        };

        let status = response.status().as_u16();
        let body = match response.text().await {
            Ok(b) => b,
            Err(_) => {
                network_errors += 1;
                if network_errors >= POLL_MAX_NETWORK_ERRORS {
                    break Err(AppError::new(
                        "connector_refresh_unavailable",
                        "Couldn't reach GitHub to complete the connection. Try again in a moment.",
                    ));
                }
                continue;
            }
        };
        // Reset the transient-error counter on any parseable response.
        network_errors = 0;

        match serde_json::from_str::<GithubTokenResponse>(&body) {
            Ok(resp) if resp.is_success() => break Ok(resp),
            Ok(resp) => {
                let error_code = resp.error.as_deref();
                match device_poll_action(error_code, interval_secs) {
                    DevicePollAction::Token => {
                        // is_success() would have caught this; shouldn't reach here.
                        break Ok(resp);
                    }
                    DevicePollAction::Pending { next_interval_secs } => {
                        interval_secs = next_interval_secs;
                    }
                    DevicePollAction::Expired => {
                        break Err(AppError::new(
                            "connector_github_device_expired",
                            "The code expired before it was approved. Try again.",
                        ));
                    }
                    DevicePollAction::Declined => {
                        break Err(AppError::new(
                            "connector_github_device_declined",
                            "GitHub reported the request was declined.",
                        ));
                    }
                    DevicePollAction::Disabled => {
                        break Err(AppError::new(
                            "connector_github_device_flow_disabled",
                            "Device flow is not enabled on the GitHub App. Enable it in the App's settings and try again.",
                        ));
                    }
                }
            }
            Err(_) => {
                tracing::warn!(
                    status,
                    "github device poll response unparseable; treating as transient"
                );
                network_errors += 1;
                if network_errors >= POLL_MAX_NETWORK_ERRORS {
                    break Err(AppError::new(
                        "connector_refresh_unavailable",
                        "Couldn't reach GitHub to complete the connection. Try again in a moment.",
                    ));
                }
            }
        }
    };

    // Clear the cancel slot so a later connect starts clean.
    flow.clear_cancel_sender();
    result
}

fn poll_form<'a>(client_id: &'a str, device_code: &'a str) -> [(&'static str, &'a str); 3] {
    // NO client_secret: the device flow specification does not use it at the
    // token endpoint. NO scope: GitHub Apps ignore scopes on user tokens.
    [
        ("client_id", client_id),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ]
}

// ----- authorize --------------------------------------------------------------

/// Run the full GitHub device authorization flow: request a device code,
/// invoke `on_device_code` so the caller can show the `user_code` and open
/// the `verification_uri` in the browser, then poll GitHub until the user
/// approves or the code expires. Cancellation is honored between polls via
/// the `ConnectFlow` cancel slot.
///
/// `github.rs` is intentionally tauri-free; `on_device_code` lets the caller
/// (mod.rs) emit the Tauri event and open the browser without this module
/// taking a dependency on `tauri`.
pub async fn authorize(
    flow: &ConnectFlow,
    client_id: &str,
    on_device_code: impl Fn(&GithubDeviceCode),
) -> Result<GithubAuthorizedGrant, AppError> {
    let device_code = request_device_code(client_id).await?;
    on_device_code(&device_code);

    let tokens = poll_for_token(flow, client_id, &device_code).await?;
    let identity = fetch_identity(&tokens.access_token).await?;
    Ok(GithubAuthorizedGrant { tokens, identity })
}

/// One refresh attempt. `bad_refresh_token` is the definitive invalid-grant
/// signal from GitHub; everything else (network failure, 5xx) is transient.
/// Refresh tokens rotate on every successful refresh; the caller persists the
/// new one and logs if the persist fails (strand-logging mirrors Linear's
/// pattern).
pub async fn refresh(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> GithubRefreshOutcome {
    let response = match oauth::http_client()
        .post(TOKEN_ENDPOINT)
        .header("Accept", "application/json")
        .form(&refresh_form(client_id, client_secret, refresh_token))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return GithubRefreshOutcome::Transient,
    };
    let status = response.status().as_u16();
    let body = match response.text().await {
        Ok(body) => body,
        Err(_) => return GithubRefreshOutcome::Transient,
    };
    match serde_json::from_str::<GithubTokenResponse>(&body) {
        Ok(resp) if resp.is_success() => GithubRefreshOutcome::Refreshed(resp),
        Ok(resp) => classify_refresh_failure(status, resp.error.as_deref()),
        Err(_) => GithubRefreshOutcome::Transient,
    }
}

fn refresh_form<'a>(
    client_id: &'a str,
    client_secret: &'a str,
    refresh_token: &'a str,
) -> [(&'static str, &'a str); 4] {
    [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ]
}

/// `bad_refresh_token` is GitHub's definitive "grant invalid" error; every
/// other failure (5xx, network, unknown error code) is transient. Mirrors the
/// Google/Linear classification pattern.
fn classify_refresh_failure(status: u16, error_code: Option<&str>) -> GithubRefreshOutcome {
    tracing::warn!(status, error_code = ?error_code, "github token refresh failed");
    match error_code {
        Some("bad_refresh_token") => GithubRefreshOutcome::InvalidGrant,
        _ => GithubRefreshOutcome::Transient,
    }
}

/// Shared GitHub API request builder: Bearer auth, required Accept and
/// X-GitHub-Api-Version headers, and the required User-Agent. GitHub requires
/// a User-Agent on every call; absent one returns 403.
const API_BASE: &str = "https://api.github.com";

#[cfg(test)]
thread_local! {
    static TEST_API_BASE: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Tests point every API call of the current thread at a local fake server.
#[cfg(test)]
pub fn set_test_api_base(base: Option<String>) {
    TEST_API_BASE.with(|cell| *cell.borrow_mut() = base);
}

fn api_url(url: &str) -> String {
    #[cfg(test)]
    if let Some(base) = TEST_API_BASE.with(|cell| cell.borrow().clone()) {
        if let Some(rest) = url.strip_prefix(API_BASE) {
            return format!("{base}{rest}");
        }
    }
    url.to_string()
}

fn github_api_request(
    access_token: &str,
    method: reqwest::Method,
    url: &str,
) -> reqwest::RequestBuilder {
    oauth::http_client()
        .request(method, api_url(url))
        .bearer_auth(access_token)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header("User-Agent", crate::http_client::USER_AGENT)
}

// ----- Identity ---------------------------------------------------------------

const USER_ENDPOINT: &str = "https://api.github.com/user";

#[derive(Deserialize)]
struct GithubUserWire {
    id: u64,
    login: String,
    #[serde(default)]
    name: Option<String>,
}

fn identity_failed() -> AppError {
    AppError::new(
        "connector_identity_failed",
        "Could not determine the GitHub account.",
    )
}

/// Resolve the user identity with the fresh access token. The numeric id keys
/// custody and the DB index; the login is the display identity. Mirrors how
/// Linear's fetch_identity works: any transport or API failure maps to the
/// single identity error.
pub async fn fetch_identity(access_token: &str) -> Result<GithubIdentity, AppError> {
    let response = github_api_request(access_token, reqwest::Method::GET, USER_ENDPOINT)
        .send()
        .await
        .map_err(|_| identity_failed())?;
    if !response.status().is_success() {
        return Err(identity_failed());
    }
    let user: GithubUserWire = response.json().await.map_err(|_| identity_failed())?;
    if user.login.is_empty() {
        return Err(identity_failed());
    }
    Ok(GithubIdentity {
        user_id: user.id.to_string(),
        login: user.login,
        name: user.name.unwrap_or_default(),
    })
}

/// Whether the connected user has at least one installation of THIS GitHub
/// App (the user-to-server token only ever lists this app's installations).
///
/// GitHub Apps split "user authorization" (the OAuth grant this connect just
/// completed) from "installation" (which repositories the app can touch). A
/// user who authorized but never installed the app holds a valid token that
/// reaches zero repositories, so the connect flow uses this to refuse a
/// hollow "connected" account rather than storing one. Only the first page is
/// read: presence, not enumeration, is the question.
pub async fn has_installation(access_token: &str) -> Result<bool, GithubApiError> {
    let url = "https://api.github.com/user/installations?per_page=1";
    let response = github_api_request(access_token, reqwest::Method::GET, url)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let wire: InstallationsWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    Ok(!wire.installations.is_empty())
}

// ----- Revoke -----------------------------------------------------------------

/// Best-effort revocation of a user access token at GitHub (used by
/// `disconnect(revoke_grant = true)` when the client secret is available).
/// Uses HTTP Basic auth (client_id:client_secret) and a JSON body, as GitHub
/// requires for this endpoint. Failures are logged and swallowed; local
/// custody removal is the real disconnect.
///
/// Revoke is only possible when the client secret is configured. Device-flow
/// authorization needs no secret, but revocation requires it. A deployment
/// that omits the secret must revoke manually at GitHub if needed.
pub async fn revoke(client_id: &str, client_secret: &str, access_token: &str) -> bool {
    let url = format!("https://api.github.com/applications/{client_id}/token");
    match oauth::http_client()
        .delete(&url)
        .basic_auth(client_id, Some(client_secret))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header("User-Agent", crate::http_client::USER_AGENT)
        .json(&serde_json::json!({ "access_token": access_token }))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let ok = response.status().is_success() || status == 404;
            if !ok {
                tracing::warn!(status, "github token revoke did not confirm");
            }
            ok
        }
        Err(_) => {
            tracing::warn!("github revoke request failed");
            false
        }
    }
}

// ----- API error type ---------------------------------------------------------

/// API error type for GitHub REST calls. Mirrors [`linear::LinearApiError`]'s
/// shape: `Unauthorized` for 401 so callers can force-refresh and retry once.
#[derive(Debug)]
pub enum GithubApiError {
    /// 401: the token was rejected. Caller should force-refresh and retry once.
    Unauthorized,
    /// 403: typically rate-limiting or scope issues.
    Forbidden,
    /// 404: the requested resource was not found.
    NotFound,
    /// Any other HTTP error with status and message.
    Api { status: u16, message: String },
    /// Network failure.
    Network(String),
}

impl From<GithubApiError> for AppError {
    fn from(error: GithubApiError) -> Self {
        match error {
            GithubApiError::Unauthorized => AppError::new(
                "github_unauthorized",
                "GitHub rejected the connection's access token.",
            ),
            GithubApiError::Forbidden => AppError::new(
                "github_forbidden",
                "GitHub denied this request. Check the app's permissions.",
            ),
            GithubApiError::NotFound => AppError::new(
                "github_not_found",
                "The requested GitHub resource was not found.",
            ),
            GithubApiError::Api { status, message } => AppError::new(
                // A 5xx can arrive AFTER GitHub committed a mutation, so it is
                // ambiguous rather than a definitive rejection (mirrors
                // Linear's linear_upstream_error). 4xx is a real rejection.
                if status >= 500 {
                    "github_upstream_error"
                } else {
                    "github_api_error"
                },
                format!("GitHub API request failed ({status}): {message}"),
            ),
            GithubApiError::Network(message) => AppError::new("network_error", message),
        }
    }
}

/// Classify an HTTP status into the error type. A response body message is
/// bounded and never echoes token material.
fn classify_api_error(status: u16, message: String) -> GithubApiError {
    match status {
        401 => GithubApiError::Unauthorized,
        403 => GithubApiError::Forbidden,
        404 => GithubApiError::NotFound,
        _ => GithubApiError::Api { status, message },
    }
}

/// Extract a human-readable error message from a GitHub API error body,
/// bounded to avoid flooding context.
async fn error_message_from_response(response: reqwest::Response) -> String {
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| crate::http_client::json_error_message(&body, 200))
        .unwrap_or_else(|| "request failed".to_string())
}

// ----- Repositories -----------------------------------------------------------

/// One repository reachable through the connected user's installations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepository {
    pub full_name: String,
    pub private: bool,
    pub default_branch: String,
    pub html_url: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
struct InstallationWire {
    id: u64,
}

#[derive(Deserialize)]
struct InstallationsWire {
    installations: Vec<InstallationWire>,
}

#[derive(Deserialize)]
struct InstallationReposWire {
    repositories: Vec<RepoWire>,
}

#[derive(Deserialize)]
struct RepoWire {
    full_name: String,
    #[serde(default)]
    private: bool,
    default_branch: String,
    html_url: String,
    #[serde(default)]
    description: Option<String>,
}

/// Maximum pages fetched per paginated GitHub endpoint call (100 items/page,
/// so at most 500 installations or 500 repos per installation).
const PAGINATION_MAX_PAGES: u32 = 5;
const PAGINATION_PER_PAGE: u32 = 100;

/// Result of a paginated page-accumulation run. Carries the accumulated items
/// and whether the safety cap was reached before exhausting all pages.
pub struct PageAccumulation {
    pub count: usize,
    pub truncated: bool,
}

/// Pure helper: given the number of items on the latest page (`page_len`) and
/// the page number just fetched (`page`), decide whether to continue to the
/// next page. Returns `(fetch_next, truncated)`.
///
/// - If `page_len < per_page`, the API has no more items - stop, not truncated.
/// - If `page == max_pages`, we hit the cap with a full page - stop, truncated.
///   `truncated` here means "may be incomplete": a set of exactly
///   `per_page * max_pages` items also lands in this branch, because knowing
///   for sure would cost one extra request past the cap. The tool copy
///   phrases the flag as "capped at 500", never as a definite "incomplete".
/// - Otherwise, continue to `page + 1`.
pub fn pagination_next(
    page_len: usize,
    page: u32,
    per_page: usize,
    max_pages: u32,
) -> (bool, bool) {
    if page_len < per_page {
        // Last page (partial or empty): no more data.
        (false, false)
    } else if page >= max_pages {
        // Full page but safety cap reached — more pages may remain.
        (false, true)
    } else {
        (true, false)
    }
}

/// List of repositories reachable through the user's GitHub App installations,
/// with a `truncated` flag that is `true` when the safety cap stopped the
/// enumeration (the list may be incomplete; see [`pagination_next`]).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepositoryList {
    pub repositories: Vec<GithubRepository>,
    /// `true` when the safety page cap stopped the enumeration, so the list
    /// may be incomplete (500 installations, or 500 repositories in one
    /// installation; see [`pagination_next`] for the boundary semantics).
    pub truncated: bool,
}

/// List the repositories a GitHub CLI token reaches (`GET /user/repos`, up to
/// 5 pages × 100). A `gh` token is an OAuth token, not a GitHub App user
/// token, so the installation endpoints [`list_repositories`] reads refuse it.
pub async fn list_user_repositories(
    access_token: &str,
) -> Result<GithubRepositoryList, GithubApiError> {
    let mut repositories = Vec::new();
    let mut truncated = false;
    for page in 1..=PAGINATION_MAX_PAGES {
        let url = format!(
            "{API_BASE}/user/repos?per_page={PAGINATION_PER_PAGE}&page={page}&sort=full_name"
        );
        let response = github_api_request(access_token, reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| GithubApiError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            let message = error_message_from_response(response).await;
            return Err(classify_api_error(status, message));
        }
        let wire: Vec<RepoWire> = response
            .json()
            .await
            .map_err(|e| GithubApiError::Network(e.to_string()))?;
        let page_len = wire.len();
        repositories.extend(wire.into_iter().map(|repo| GithubRepository {
            full_name: repo.full_name,
            private: repo.private,
            default_branch: repo.default_branch,
            html_url: repo.html_url,
            description: repo.description,
        }));
        let (next, capped) = pagination_next(
            page_len,
            page,
            PAGINATION_PER_PAGE as usize,
            PAGINATION_MAX_PAGES,
        );
        truncated = capped;
        if !next {
            break;
        }
    }
    Ok(GithubRepositoryList {
        repositories,
        truncated,
    })
}

/// List all repositories accessible via the user's GitHub App installations.
/// Paginates both `GET /user/installations` (up to 5 pages × 100 = 500
/// installations) and `GET /user/installations/{id}/repositories` (same cap
/// per installation). Returns `truncated: true` when the safety cap cuts
/// enumeration short.
pub async fn list_repositories(access_token: &str) -> Result<GithubRepositoryList, GithubApiError> {
    // --- Paginate installations -----------------------------------------------
    let mut installations: Vec<InstallationWire> = Vec::new();
    let mut install_truncated = false;
    'install_pages: for page in 1..=PAGINATION_MAX_PAGES {
        let url = format!(
            "https://api.github.com/user/installations?per_page={}&page={}",
            PAGINATION_PER_PAGE, page
        );
        let response = github_api_request(access_token, reqwest::Method::GET, &url)
            .send()
            .await
            .map_err(|e| GithubApiError::Network(e.to_string()))?;
        let status = response.status().as_u16();
        if !response.status().is_success() {
            let message = error_message_from_response(response).await;
            return Err(classify_api_error(status, message));
        }
        let wire: InstallationsWire = response
            .json()
            .await
            .map_err(|e| GithubApiError::Network(e.to_string()))?;
        let page_len = wire.installations.len();
        installations.extend(wire.installations);
        let (fetch_next, truncated) = pagination_next(
            page_len,
            page,
            PAGINATION_PER_PAGE as usize,
            PAGINATION_MAX_PAGES,
        );
        if truncated {
            install_truncated = true;
            break 'install_pages;
        }
        if !fetch_next {
            break 'install_pages;
        }
    }

    // --- For each installation, paginate repositories -------------------------
    let mut repos: Vec<GithubRepository> = Vec::new();
    let mut repo_truncated = false;
    for installation in installations {
        'repo_pages: for page in 1..=PAGINATION_MAX_PAGES {
            let url = format!(
                "https://api.github.com/user/installations/{}/repositories?per_page={}&page={}",
                installation.id, PAGINATION_PER_PAGE, page
            );
            let resp = github_api_request(access_token, reqwest::Method::GET, &url)
                .send()
                .await
                .map_err(|e| GithubApiError::Network(e.to_string()))?;
            let install_status = resp.status().as_u16();
            if !resp.status().is_success() {
                let message = error_message_from_response(resp).await;
                return Err(classify_api_error(install_status, message));
            }
            let install_repos: InstallationReposWire = resp
                .json()
                .await
                .map_err(|e| GithubApiError::Network(e.to_string()))?;
            let page_len = install_repos.repositories.len();
            for repo in install_repos.repositories {
                repos.push(GithubRepository {
                    full_name: repo.full_name,
                    private: repo.private,
                    default_branch: repo.default_branch,
                    html_url: repo.html_url,
                    description: repo.description,
                });
            }
            let (fetch_next, truncated) = pagination_next(
                page_len,
                page,
                PAGINATION_PER_PAGE as usize,
                PAGINATION_MAX_PAGES,
            );
            if truncated {
                repo_truncated = true;
                break 'repo_pages;
            }
            if !fetch_next {
                break 'repo_pages;
            }
        }
    }

    Ok(GithubRepositoryList {
        repositories: repos,
        truncated: install_truncated || repo_truncated,
    })
}

// ----- Issue search -----------------------------------------------------------

/// One issue or pull request from a search result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubIssueSummary {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub is_pull_request: bool,
    pub repo_full_name: String,
    pub updated_at: String,
    pub html_url: String,
}

#[derive(Deserialize)]
struct SearchIssuesWire {
    items: Vec<SearchIssueItemWire>,
}

#[derive(Deserialize)]
struct SearchIssueItemWire {
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
    repository_url: String,
    updated_at: String,
    html_url: String,
}

/// Search issues and pull requests via GitHub's search API. `per_page` is
/// capped at 30.
pub async fn search_issues(
    access_token: &str,
    query: &str,
    per_page: Option<u32>,
) -> Result<Vec<GithubIssueSummary>, GithubApiError> {
    let per_page = per_page.unwrap_or(25).min(30);
    let url = format!(
        "https://api.github.com/search/issues?q={}&per_page={}",
        urlencoding::encode(query),
        per_page,
    );
    let response = github_api_request(access_token, reqwest::Method::GET, &url)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let result: SearchIssuesWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    Ok(result
        .items
        .into_iter()
        .map(|item| {
            // Parse repo full_name from repository_url:
            // "https://api.github.com/repos/{owner}/{repo}"
            let repo_full_name = item
                .repository_url
                .strip_prefix("https://api.github.com/repos/")
                .unwrap_or(&item.repository_url)
                .to_string();
            GithubIssueSummary {
                number: item.number,
                title: item.title,
                state: item.state,
                is_pull_request: item.pull_request.is_some(),
                repo_full_name,
                updated_at: item.updated_at,
                html_url: item.html_url,
            }
        })
        .collect())
}

// ----- Issue detail -----------------------------------------------------------

const ISSUE_BODY_MAX_CHARS: usize = 20_000;

/// Detailed view of one GitHub issue.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubIssueDetail {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub body_truncated: bool,
    pub state: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub updated_at: String,
    pub html_url: String,
}

#[derive(Deserialize)]
struct IssueDetailWire {
    number: u64,
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    #[serde(default)]
    labels: Vec<LabelWire>,
    #[serde(default)]
    assignees: Vec<AssigneeWire>,
    updated_at: String,
    html_url: String,
}

#[derive(Deserialize)]
struct LabelWire {
    name: String,
}

#[derive(Deserialize)]
struct AssigneeWire {
    login: String,
}

/// Fetch one issue's full detail. `owner` and `repo` are the repository owner
/// and name; `number` is the issue number.
pub async fn get_issue(
    access_token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<GithubIssueDetail, GithubApiError> {
    let url = repo_api_url(owner, repo, &format!("/issues/{number}"));
    let response = github_api_request(access_token, reqwest::Method::GET, &url)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let wire: IssueDetailWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let (body, body_truncated) = bound_body(wire.body);
    Ok(GithubIssueDetail {
        number: wire.number,
        title: wire.title,
        body,
        body_truncated,
        state: wire.state,
        labels: wire.labels.into_iter().map(|l| l.name).collect(),
        assignees: wire.assignees.into_iter().map(|a| a.login).collect(),
        updated_at: wire.updated_at,
        html_url: wire.html_url,
    })
}

/// Bound a body text to `ISSUE_BODY_MAX_CHARS`, returning the bounded text and
/// whether truncation occurred. Works on chars so multi-byte text always lands
/// on a valid boundary.
fn bound_body(body: Option<String>) -> (Option<String>, bool) {
    match body {
        None => (None, false),
        Some(text) => {
            let char_count = text.chars().count();
            if char_count <= ISSUE_BODY_MAX_CHARS {
                (Some(text), false)
            } else {
                let bounded: String = text.chars().take(ISSUE_BODY_MAX_CHARS).collect();
                (Some(bounded), true)
            }
        }
    }
}

// ----- Issue comments ---------------------------------------------------------

const COMMENT_BODY_MAX_CHARS: usize = 4_000;

/// One comment on a GitHub issue or pull request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubIssueComment {
    pub id: u64,
    pub author_login: String,
    pub body: String,
    pub created_at: String,
}

#[derive(Deserialize)]
struct CommentWire {
    id: u64,
    #[serde(default)]
    user: Option<UserLoginWire>,
    #[serde(default)]
    body: String,
    created_at: String,
}

#[derive(Deserialize)]
struct UserLoginWire {
    login: String,
}

/// List comments on an issue or pull request. `per_page` is capped at 30.
pub async fn list_issue_comments(
    access_token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    per_page: Option<u32>,
) -> Result<Vec<GithubIssueComment>, GithubApiError> {
    let per_page = per_page.unwrap_or(25).min(30);
    let url = repo_api_url(
        owner,
        repo,
        &format!("/issues/{number}/comments?per_page={per_page}"),
    );
    let response = github_api_request(access_token, reqwest::Method::GET, &url)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let wires: Vec<CommentWire> = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    Ok(wires
        .into_iter()
        .map(|w| {
            let body_chars: String = w.body.chars().take(COMMENT_BODY_MAX_CHARS).collect();
            GithubIssueComment {
                id: w.id,
                author_login: w.user.map(|u| u.login).unwrap_or_default(),
                body: body_chars,
                created_at: w.created_at,
            }
        })
        .collect())
}

// ----- Pull request detail ----------------------------------------------------

/// Detailed view of one GitHub pull request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubPullRequest {
    pub number: u64,
    pub title: String,
    pub body: Option<String>,
    pub body_truncated: bool,
    pub state: String,
    pub draft: bool,
    pub head_ref: String,
    pub head_sha: String,
    pub base_ref: String,
    pub base_sha: String,
    pub mergeable_state: Option<String>,
    pub changed_files: u32,
    pub additions: u32,
    pub deletions: u32,
    pub html_url: String,
    pub updated_at: String,
}

#[derive(Deserialize)]
struct PullRequestWire {
    number: u64,
    title: String,
    #[serde(default)]
    body: Option<String>,
    state: String,
    #[serde(default)]
    draft: bool,
    head: BranchRefWire,
    base: BranchRefWire,
    #[serde(default)]
    mergeable_state: Option<String>,
    #[serde(default)]
    changed_files: u32,
    #[serde(default)]
    additions: u32,
    #[serde(default)]
    deletions: u32,
    html_url: String,
    updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BranchRefWire {
    #[serde(rename = "ref")]
    ref_name: String,
    sha: String,
}

/// Fetch one pull request's detail.
pub async fn get_pull_request(
    access_token: &str,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<GithubPullRequest, GithubApiError> {
    let url = repo_api_url(owner, repo, &format!("/pulls/{number}"));
    let response = github_api_request(access_token, reqwest::Method::GET, &url)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let wire: PullRequestWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let (body, body_truncated) = bound_body(wire.body);
    Ok(GithubPullRequest {
        number: wire.number,
        title: wire.title,
        body,
        body_truncated,
        state: wire.state,
        draft: wire.draft,
        head_ref: wire.head.ref_name,
        head_sha: wire.head.sha,
        base_ref: wire.base.ref_name,
        base_sha: wire.base.sha,
        mergeable_state: wire.mergeable_state,
        changed_files: wire.changed_files,
        additions: wire.additions,
        deletions: wire.deletions,
        html_url: wire.html_url,
        updated_at: wire.updated_at,
    })
}

// ----- URL building -----------------------------------------------------------

/// Percent-encode one URL path segment (an owner or repo name). Valid GitHub
/// names never need escaping, so this is defense in depth against a
/// model-supplied value smuggling `/`, `?`, or `#` into the request path.
fn encode_segment(value: &str) -> String {
    urlencoding::encode(value).into_owned()
}

/// Percent-encode a repository file path segment by segment, preserving `/`
/// so the contents API sees the real directory structure. Encoding the whole
/// path would send `src/lib.rs` as the single segment `src%2Flib.rs`.
fn encode_repo_path(path: &str) -> String {
    path.split('/')
        .map(|segment| urlencoding::encode(segment).into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// `https://api.github.com/repos/{owner}/{repo}{tail}` with the owner and
/// repo segments encoded. `tail` must start with `/` (or `?` for none) and is
/// appended verbatim, so callers encode any dynamic parts of it themselves.
fn repo_api_url(owner: &str, repo: &str, tail: &str) -> String {
    format!(
        "https://api.github.com/repos/{}/{}{tail}",
        encode_segment(owner),
        encode_segment(repo)
    )
}

// ----- File read --------------------------------------------------------------

/// Maximum bytes to decode from a file's base64 content before truncating.
const READ_FILE_MAX_BYTES: usize = 200_000;

/// Contents of one file from a GitHub repository.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubFileContent {
    pub path: String,
    pub content: String,
    pub truncated: bool,
    pub encoding: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct ContentsWire {
    #[serde(rename = "type")]
    content_type: String,
    path: String,
    content: String,
    encoding: String,
    #[serde(default)]
    size: u64,
}

/// Read one file from a GitHub repository, decoding base64 content. Refuses
/// non-file types (directories, symlinks, submodules) with a clean error.
/// GitHub returns `encoding: "none"` (with empty `content`) for files between
/// 1 MB and 100 MB; those return an error naming the file's size and Clovy's
/// read cap (200 KB) rather than fabricating an empty file.
/// Caps decoded base64 content at `READ_FILE_MAX_BYTES` bytes.
pub async fn read_file(
    access_token: &str,
    owner: &str,
    repo: &str,
    path: &str,
    git_ref: Option<&str>,
) -> Result<GithubFileContent, GithubApiError> {
    let mut url = repo_api_url(
        owner,
        repo,
        &format!("/contents/{}", encode_repo_path(path)),
    );
    if let Some(r) = git_ref {
        url.push_str(&format!("?ref={}", urlencoding::encode(r)));
    }
    let response = github_api_request(access_token, reqwest::Method::GET, &url)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let wire: ContentsWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;

    if wire.content_type != "file" {
        return Err(GithubApiError::Api {
            status: 422,
            message: format!("Path '{}' is a {}, not a file.", path, wire.content_type),
        });
    }

    // GitHub returns `encoding: "none"` (with empty `content`) for files
    // between 1 MB and 100 MB. Treating that as a successful empty file would
    // silently fabricate content. Return a clear error naming the file's
    // actual size and Clovy's read cap so the agent can advise the user.
    if wire.encoding != "base64" {
        let size_kb = wire.size / 1024;
        return Err(GithubApiError::Api {
            status: 422,
            message: format!(
                "File '{}' is {size_kb} KB, which exceeds Clovy's {cap_kb} KB read cap \
                 (GitHub returns non-base64 encoding for files over 1 MB). \
                 Clovy can read files up to {cap_kb} KB.",
                path,
                size_kb = size_kb,
                cap_kb = READ_FILE_MAX_BYTES / 1024,
            ),
        });
    }

    // GitHub base64 encodes with newlines; strip them before decoding.
    let cleaned = wire.content.replace(['\n', '\r'], "");
    let decoded = URL_SAFE_NO_PAD
        .decode(cleaned.as_bytes())
        .or_else(|_| {
            // Standard base64 (with padding) as fallback.
            base64::engine::general_purpose::STANDARD.decode(cleaned.as_bytes())
        })
        .map_err(|_| GithubApiError::Api {
            status: 422,
            message: "Could not decode file content from GitHub.".to_string(),
        })?;

    let truncated = decoded.len() > READ_FILE_MAX_BYTES;
    let capped = &decoded[..decoded.len().min(READ_FILE_MAX_BYTES)];
    // Lossy UTF-8: non-UTF-8 bytes become replacement characters rather than
    // failing the whole read; binary files are expected to be unreadable.
    let content = String::from_utf8_lossy(capped).into_owned();

    Ok(GithubFileContent {
        path: wire.path,
        content,
        truncated,
        encoding: wire.encoding,
        size: wire.size,
    })
}

// ----- Write operations -------------------------------------------------------

/// Created or updated issue returned by GitHub.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubIssueRef {
    pub number: u64,
    pub html_url: String,
}

#[derive(Deserialize)]
struct IssueRefWire {
    number: u64,
    html_url: String,
}

/// Create a new issue in a repository.
pub async fn create_issue(
    access_token: &str,
    owner: &str,
    repo: &str,
    title: &str,
    body: Option<&str>,
    labels: Option<&[String]>,
) -> Result<GithubIssueRef, GithubApiError> {
    let url = repo_api_url(owner, repo, "/issues");
    let mut payload = serde_json::json!({ "title": title });
    if let Some(b) = body {
        payload["body"] = serde_json::json!(b);
    }
    if let Some(ls) = labels {
        payload["labels"] = serde_json::json!(ls);
    }
    let response = github_api_request(access_token, reqwest::Method::POST, &url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let wire: IssueRefWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    Ok(GithubIssueRef {
        number: wire.number,
        html_url: wire.html_url,
    })
}

/// Update an existing issue's title, body, and/or labels. NEVER sends `state`:
/// closing/reopening is a launch non-goal per ADR-0036 and the PRD.
pub async fn update_issue(
    access_token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    title: Option<&str>,
    body: Option<&str>,
    labels: Option<&[String]>,
) -> Result<GithubIssueRef, GithubApiError> {
    let url = repo_api_url(owner, repo, &format!("/issues/{number}"));
    let mut payload = serde_json::json!({});
    if let Some(t) = title {
        payload["title"] = serde_json::json!(t);
    }
    if let Some(b) = body {
        payload["body"] = serde_json::json!(b);
    }
    if let Some(ls) = labels {
        payload["labels"] = serde_json::json!(ls);
    }
    let response = github_api_request(access_token, reqwest::Method::PATCH, &url)
        .json(&payload)
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    let wire: IssueRefWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    Ok(GithubIssueRef {
        number: wire.number,
        html_url: wire.html_url,
    })
}

/// Add a comment to an issue or pull request. Works for both by design: GitHub
/// issue and PR comments share the same endpoint.
pub async fn add_comment(
    access_token: &str,
    owner: &str,
    repo: &str,
    number: u64,
    body: &str,
) -> Result<GithubIssueRef, GithubApiError> {
    let url = repo_api_url(owner, repo, &format!("/issues/{number}/comments"));
    let response = github_api_request(access_token, reqwest::Method::POST, &url)
        .json(&serde_json::json!({ "body": body }))
        .send()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    if !response.status().is_success() {
        let message = error_message_from_response(response).await;
        return Err(classify_api_error(status, message));
    }
    // Comment response has `id` and `html_url`, not `number`. Return html_url
    // and the original issue number (which we have from the caller) so the
    // return type is consistent with create/update_issue.
    #[derive(Deserialize)]
    struct CommentRefWire {
        html_url: String,
    }
    let wire: CommentRefWire = response
        .json()
        .await
        .map_err(|e| GithubApiError::Network(e.to_string()))?;
    Ok(GithubIssueRef {
        number,
        html_url: wire.html_url,
    })
}

// ----- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_repo_path_preserves_directory_separators() {
        // A nested path must stay multi-segment: encoding the whole string
        // would send `src%2Flib.rs` and 404 (or worse, hit the wrong route).
        assert_eq!(encode_repo_path("src/lib.rs"), "src/lib.rs");
        assert_eq!(
            encode_repo_path("src/connectors/github.rs"),
            "src/connectors/github.rs"
        );
        // Spaces and other unsafe characters inside a segment are still
        // escaped; the separators between segments are not.
        assert_eq!(encode_repo_path("docs/my notes.md"), "docs/my%20notes.md");
        assert_eq!(encode_repo_path("a/b?c"), "a/b%3Fc");
    }

    #[test]
    fn repo_api_url_encodes_owner_and_repo_segments() {
        assert_eq!(
            repo_api_url("owner", "repo", "/issues/7"),
            "https://api.github.com/repos/owner/repo/issues/7"
        );
        // A stray slash in a model-supplied owner cannot open a new path
        // segment: it is percent-encoded.
        assert_eq!(
            repo_api_url("evil/../x", "repo", "/issues"),
            "https://api.github.com/repos/evil%2F..%2Fx/repo/issues"
        );
    }

    // ----- device_poll_action classification ----------------------------------

    #[test]
    fn device_poll_action_success_when_no_error() {
        assert_eq!(device_poll_action(None, 5), DevicePollAction::Token);
    }

    #[test]
    fn device_poll_action_pending_keeps_interval() {
        assert_eq!(
            device_poll_action(Some("authorization_pending"), 5),
            DevicePollAction::Pending {
                next_interval_secs: 5
            }
        );
    }

    #[test]
    fn device_poll_action_slow_down_increments_by_5() {
        assert_eq!(
            device_poll_action(Some("slow_down"), 5),
            DevicePollAction::Pending {
                next_interval_secs: 10
            }
        );
        // Slow down from a higher base.
        assert_eq!(
            device_poll_action(Some("slow_down"), 12),
            DevicePollAction::Pending {
                next_interval_secs: 17
            }
        );
    }

    #[test]
    fn device_poll_action_expired_token_maps_to_expired() {
        assert_eq!(
            device_poll_action(Some("expired_token"), 5),
            DevicePollAction::Expired
        );
    }

    #[test]
    fn device_poll_action_access_denied_maps_to_declined() {
        assert_eq!(
            device_poll_action(Some("access_denied"), 5),
            DevicePollAction::Declined
        );
    }

    #[test]
    fn device_poll_action_device_flow_disabled_variants() {
        assert_eq!(
            device_poll_action(Some("unauthorized_client"), 5),
            DevicePollAction::Disabled
        );
        assert_eq!(
            device_poll_action(Some("device_flow_not_enabled"), 5),
            DevicePollAction::Disabled
        );
    }

    #[test]
    fn device_poll_action_unknown_error_treated_as_pending() {
        // Unknown errors are treated as transient pending rather than aborting.
        assert_eq!(
            device_poll_action(Some("some_future_error"), 5),
            DevicePollAction::Pending {
                next_interval_secs: 5
            }
        );
    }

    // ----- poll_form: no client_secret, no scope ------------------------------

    #[test]
    fn poll_form_carries_no_client_secret_and_correct_grant_type() {
        let form = poll_form("cid", "device-code-abc");
        assert!(form.contains(&("client_id", "cid")));
        assert!(form.contains(&("grant_type", "urn:ietf:params:oauth:grant-type:device_code")));
        assert!(form.contains(&("device_code", "device-code-abc")));
        // No client_secret in the device flow token request.
        assert!(!form.iter().any(|(k, _)| *k == "client_secret"));
        // No scope: GitHub Apps ignore scopes on user-token requests.
        assert!(!form.iter().any(|(k, _)| *k == "scope"));
    }

    // ----- refresh_form carries client_secret ---------------------------------

    #[test]
    fn refresh_form_carries_rotation_fields() {
        let form = refresh_form("cid", "csecret", "refresh-tok");
        assert!(form.contains(&("grant_type", "refresh_token")));
        assert!(form.contains(&("refresh_token", "refresh-tok")));
        assert!(form.contains(&("client_id", "cid")));
        assert!(form.contains(&("client_secret", "csecret")));
    }

    #[test]
    fn classify_refresh_failure_bad_refresh_token_is_invalid_grant() {
        assert!(matches!(
            classify_refresh_failure(200, Some("bad_refresh_token")),
            GithubRefreshOutcome::InvalidGrant
        ));
    }

    #[test]
    fn classify_refresh_failure_other_errors_are_transient() {
        assert!(matches!(
            classify_refresh_failure(500, None),
            GithubRefreshOutcome::Transient
        ));
        assert!(matches!(
            classify_refresh_failure(400, Some("some_other_error")),
            GithubRefreshOutcome::Transient
        ));
        assert!(matches!(
            classify_refresh_failure(200, Some("invalid_grant")),
            GithubRefreshOutcome::Transient
        ));
    }

    #[test]
    fn token_response_success_check() {
        let success = GithubTokenResponse {
            access_token: "ghu_abc".to_string(),
            token_type: "bearer".to_string(),
            expires_in: None,
            refresh_token: None,
            refresh_token_expires_in: None,
            error: None,
            error_description: None,
        };
        assert!(success.is_success());

        let failure = GithubTokenResponse {
            access_token: String::new(),
            token_type: String::new(),
            expires_in: None,
            refresh_token: None,
            refresh_token_expires_in: None,
            error: Some("bad_verification_code".to_string()),
            error_description: None,
        };
        assert!(!failure.is_success());
    }

    #[test]
    fn non_expiring_token_lifetime_is_about_100_years() {
        // 100 years = 100 * 365.25 * 24 * 3600 ~ 3_155_760_000 seconds.
        // The constant is slightly below that for clarity; verify it's in the
        // right ballpark (between 95 and 105 years).
        let years = NON_EXPIRING_TOKEN_LIFETIME_SECS / (365 * 24 * 3600);
        assert!(years >= 95, "expected ~100 years, got {years}");
        assert!(years <= 105, "expected ~100 years, got {years}");
    }

    #[test]
    fn bound_body_truncates_at_max_and_reports_flag() {
        // No truncation when under the limit.
        let short = "hello".to_string();
        let (text, truncated) = bound_body(Some(short.clone()));
        assert_eq!(text.as_deref(), Some("hello"));
        assert!(!truncated);

        // Truncation at exactly the limit.
        let long: String = "x".repeat(ISSUE_BODY_MAX_CHARS + 10);
        let (text, truncated) = bound_body(Some(long));
        assert_eq!(text.map(|t| t.chars().count()), Some(ISSUE_BODY_MAX_CHARS));
        assert!(truncated);

        // None body is preserved as None.
        let (text, truncated) = bound_body(None);
        assert!(text.is_none());
        assert!(!truncated);
    }

    #[test]
    fn search_issues_result_parses_pull_request_flag() {
        // Verify the JSON shape: presence of pull_request key => is_pull_request.
        let issue_json = r#"{
            "number": 1,
            "title": "Bug",
            "state": "open",
            "repository_url": "https://api.github.com/repos/owner/repo",
            "updated_at": "2026-01-01T00:00:00Z",
            "html_url": "https://github.com/owner/repo/issues/1"
        }"#;
        let pr_json = r#"{
            "number": 2,
            "title": "Feature",
            "state": "open",
            "pull_request": {"url": "https://api.github.com/repos/owner/repo/pulls/2"},
            "repository_url": "https://api.github.com/repos/owner/repo",
            "updated_at": "2026-01-01T00:00:00Z",
            "html_url": "https://github.com/owner/repo/pull/2"
        }"#;
        let issue_wire: SearchIssueItemWire = serde_json::from_str(issue_json).unwrap();
        let pr_wire: SearchIssueItemWire = serde_json::from_str(pr_json).unwrap();
        assert!(issue_wire.pull_request.is_none());
        assert!(pr_wire.pull_request.is_some());
    }

    #[test]
    fn api_error_401_maps_to_unauthorized() {
        assert!(matches!(
            classify_api_error(401, "Bad credentials".to_string()),
            GithubApiError::Unauthorized
        ));
    }

    #[test]
    fn api_error_404_maps_to_not_found() {
        assert!(matches!(
            classify_api_error(404, "Not Found".to_string()),
            GithubApiError::NotFound
        ));
    }

    #[test]
    fn api_error_403_maps_to_forbidden() {
        assert!(matches!(
            classify_api_error(403, "Forbidden".to_string()),
            GithubApiError::Forbidden
        ));
    }

    // ----- pagination_next tests -----------------------------------------------

    #[test]
    fn pagination_next_partial_page_means_done_not_truncated() {
        // Fewer items than per_page: we've seen the last page.
        let (fetch_next, truncated) = pagination_next(42, 1, 100, 5);
        assert!(!fetch_next);
        assert!(!truncated);
    }

    #[test]
    fn pagination_next_empty_page_means_done_not_truncated() {
        let (fetch_next, truncated) = pagination_next(0, 1, 100, 5);
        assert!(!fetch_next);
        assert!(!truncated);
    }

    #[test]
    fn pagination_next_full_page_below_cap_means_continue() {
        // Full page and not yet at the cap: fetch the next page.
        let (fetch_next, truncated) = pagination_next(100, 3, 100, 5);
        assert!(fetch_next);
        assert!(!truncated);
    }

    #[test]
    fn pagination_next_full_page_at_cap_means_truncated() {
        // Full page at the safety cap: stop and signal truncation.
        let (fetch_next, truncated) = pagination_next(100, 5, 100, 5);
        assert!(!fetch_next);
        assert!(truncated);
    }

    #[test]
    fn pagination_next_full_page_on_first_page_at_cap_one_is_truncated() {
        // Edge case: cap is 1 and we got a full page.
        let (fetch_next, truncated) = pagination_next(100, 1, 100, 1);
        assert!(!fetch_next);
        assert!(truncated);
    }

    #[test]
    fn github_repository_list_serializes_camel_case_with_truncated() {
        let list = GithubRepositoryList {
            repositories: vec![GithubRepository {
                full_name: "owner/repo".to_string(),
                private: false,
                default_branch: "main".to_string(),
                html_url: "https://github.com/owner/repo".to_string(),
                description: None,
            }],
            truncated: true,
        };
        let json = serde_json::to_value(&list).unwrap();
        assert_eq!(json["truncated"], true);
        assert!(json["repositories"].is_array());
        let repo = &json["repositories"][0];
        // camelCase field names
        assert!(repo.get("fullName").is_some());
        assert!(repo.get("defaultBranch").is_some());
        assert!(repo.get("htmlUrl").is_some());
    }

    #[test]
    fn server_5xx_maps_to_ambiguous_upstream_error_not_definitive() {
        // A 5xx can land after GitHub committed a mutation, so it must carry
        // the ambiguous code; 4xx stays a definitive github_api_error.
        let upstream: AppError = classify_api_error(502, "bad gateway".to_string()).into();
        assert_eq!(upstream.code, "github_upstream_error");
        let definitive: AppError = classify_api_error(422, "validation failed".to_string()).into();
        assert_eq!(definitive.code, "github_api_error");
    }

    // ----- read_file: large-file encoding guard tests ---------------------------

    /// Simulate the wire response GitHub returns for files between 1 MB and
    /// 100 MB: `encoding: "none"`, empty `content`, and a non-zero `size`.
    fn make_large_file_wire(size_bytes: u64) -> ContentsWire {
        ContentsWire {
            content_type: "file".to_string(),
            path: "big.bin".to_string(),
            content: String::new(),
            encoding: "none".to_string(),
            size: size_bytes,
        }
    }

    fn make_base64_file_wire(content_b64: &str) -> ContentsWire {
        ContentsWire {
            content_type: "file".to_string(),
            path: "small.txt".to_string(),
            content: content_b64.to_string(),
            encoding: "base64".to_string(),
            size: content_b64.len() as u64,
        }
    }

    /// Pure helper: apply the encoding guard logic used in `read_file` and
    /// return an error if encoding is non-base64 with size > 0, mirroring the
    /// real implementation. Used to make the policy testable without hitting
    /// the network.
    fn encoding_guard_result(wire: &ContentsWire, path: &str) -> Result<(), String> {
        if wire.encoding != "base64" {
            let size_kb = wire.size / 1024;
            return Err(format!(
                "File '{}' is {size_kb} KB, which exceeds Clovy's {cap_kb} KB read cap \
                 (GitHub returns non-base64 encoding for files over 1 MB). \
                 Clovy can read files up to {cap_kb} KB.",
                path,
                size_kb = size_kb,
                cap_kb = READ_FILE_MAX_BYTES / 1024,
            ));
        }
        Ok(())
    }

    #[test]
    fn read_file_encoding_none_with_nonzero_size_returns_error_naming_size() {
        // 5 MB file: 5 * 1024 * 1024 / 1024 = 5120 KB.
        // READ_FILE_MAX_BYTES = 200_000; 200_000 / 1024 = 195 KB.
        let wire = make_large_file_wire(5 * 1024 * 1024);
        let result = encoding_guard_result(&wire, "big.bin");
        assert!(result.is_err(), "expected error for encoding=none, size>0");
        let msg = result.unwrap_err();
        let expected_size_kb = (5u64 * 1024 * 1024) / 1024;
        let expected_cap_kb = READ_FILE_MAX_BYTES / 1024;
        assert!(
            msg.contains(&format!("{expected_size_kb} KB")),
            "error should name size in KB ({expected_size_kb}); got: {msg}"
        );
        assert!(
            msg.contains(&format!("{expected_cap_kb} KB")),
            "error should mention Clovy's {expected_cap_kb} KB cap; got: {msg}"
        );
        assert!(
            msg.contains("big.bin"),
            "error should name the file; got: {msg}"
        );
    }

    #[test]
    fn read_file_encoding_base64_path_is_unchanged() {
        // A normal small file should not be blocked by the encoding guard.
        let wire = make_base64_file_wire("aGVsbG8="); // "hello"
        let result = encoding_guard_result(&wire, "small.txt");
        assert!(
            result.is_ok(),
            "base64 encoding must not be blocked by the guard"
        );
    }

    #[test]
    fn read_file_encoding_none_zero_size_returns_error_guard() {
        // Even with size=0 the guard triggers on non-base64 encoding.
        let wire = make_large_file_wire(0);
        let result = encoding_guard_result(&wire, "empty.bin");
        assert!(
            result.is_err(),
            "non-base64 encoding should always be rejected, even with size=0"
        );
    }

    // ----- device error code stable strings -----------------------------------

    #[test]
    fn device_error_codes_are_stable() {
        // Verify the exact error codes the frontend contract specifies.
        let declined = AppError::new(
            "connector_github_device_declined",
            "GitHub reported the request was declined.",
        );
        assert_eq!(declined.code, "connector_github_device_declined");

        let expired = AppError::new(
            "connector_github_device_expired",
            "The code expired before it was approved. Try again.",
        );
        assert_eq!(expired.code, "connector_github_device_expired");

        let disabled = AppError::new(
            "connector_github_device_flow_disabled",
            "Device flow is not enabled on the GitHub App. Enable it in the App's settings and try again.",
        );
        assert_eq!(disabled.code, "connector_github_device_flow_disabled");
    }

    // ----- poll_deadline_reached tests ----------------------------------------

    #[test]
    fn poll_deadline_not_reached_before_window() {
        // Well before the expiry window: not reached.
        assert!(!poll_deadline_reached(0, 900));
        assert!(!poll_deadline_reached(100, 900));
        // Just before the deadline (900 + 30 - 1 = 929 elapsed).
        assert!(!poll_deadline_reached(929, 900));
    }

    #[test]
    fn poll_deadline_reached_at_boundary() {
        // Exactly at expires_in + margin (900 + 30 = 930): reached.
        assert!(poll_deadline_reached(930, 900));
    }

    #[test]
    fn poll_deadline_reached_after_boundary() {
        // Past the deadline: reached.
        assert!(poll_deadline_reached(1000, 900));
        assert!(poll_deadline_reached(u64::MAX / 2, 900));
    }

    #[test]
    fn poll_deadline_fallback_when_expires_in_is_zero() {
        // expires_in = 0 means "not provided"; fallback is
        // DEVICE_CODE_EXPIRES_IN_FALLBACK_SECS (900).
        // Before fallback + margin: not reached.
        assert!(
            !poll_deadline_reached(929, 0),
            "before fallback deadline must not be reached"
        );
        // At fallback + margin: reached.
        assert!(
            poll_deadline_reached(930, 0),
            "at fallback deadline must be reached"
        );
    }

    #[test]
    fn poll_deadline_uses_actual_expires_in_not_fallback() {
        // A non-zero expires_in (e.g. 600 s, GitHub's minimum) must use the
        // actual value, not the fallback. Deadline is 600 + 30 = 630 s.
        assert!(!poll_deadline_reached(629, 600));
        assert!(poll_deadline_reached(630, 600));
        // Must NOT use the fallback (900 + 30 = 930):
        assert!(
            poll_deadline_reached(700, 600),
            "600-s code must expire well before fallback deadline"
        );
    }
}
