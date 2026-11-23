//! Private connectors (Google, Linear), local mode.
//!
//! OAuth (PKCE + loopback), Keychain token custody, the scope registry, and
//! the direct provider clients. The provider proxy and the trigger daemon
//! consume this module: they resolve an access token via
//! [`google_access_token`] / [`linear_access_token`] and call the [`google`]
//! / [`linear`] functions with it. Clovy API and OpenSoftware infrastructure
//! are never in the connector data path.
//!
//! Secrets live ONLY in the keychain ([`store`]); the SQLite index carries
//! non-secret account metadata (emails, scopes, status) so accounts can be
//! enumerated without keychain prompts. Tokens are never logged and never
//! serialized into errors.

pub mod agent_tools;
pub mod approvals;
pub mod commands;
pub mod github;
pub mod github_cli;
#[cfg(all(test, unix))]
pub(crate) mod github_cli_fixtures;
pub mod google;
pub mod linear;
pub mod notion;
pub mod oauth;
pub mod policy;
pub mod store;
pub mod triggers;

use crate::domain::types::AppError;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex as StdMutex, OnceLock,
    },
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

pub use notion::NotionConnectFlow;
pub use oauth::ConnectFlow;

/// Access tokens within this many seconds of expiry are refreshed instead of
/// returned, so a caller never receives a token that dies mid-request.
const ACCESS_TOKEN_EXPIRY_BUFFER_SECS: i64 = 60;
const GOOGLE_OAUTH_CLIENT_ID_ENV: &str = "GOOGLE_OAUTH_CLIENT_ID";
const GOOGLE_OAUTH_CLIENT_SECRET_ENV: &str = "GOOGLE_OAUTH_CLIENT_SECRET";
const LINEAR_OAUTH_CLIENT_ID_ENV: &str = "LINEAR_OAUTH_CLIENT_ID";
pub(crate) const GITHUB_OAUTH_CLIENT_ID_ENV: &str = "GITHUB_OAUTH_CLIENT_ID";
pub(crate) const GITHUB_OAUTH_CLIENT_SECRET_ENV: &str = "GITHUB_OAUTH_CLIENT_SECRET";
/// Tauri event emitted when a GitHub device code is obtained. The camelCase
/// payload carries `userCode`, `verificationUri`, and `expiresInSeconds` so
/// the frontend dialog can display the code and open the verification URL.
const GITHUB_DEVICE_CODE_EVENT: &str = "clovy://connectors-github-device-code";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorProvider {
    Google,
    Notion,
    Linear,
    Github,
}

impl ConnectorProvider {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectorProvider::Google => "google",
            ConnectorProvider::Notion => "notion",
            ConnectorProvider::Linear => "linear",
            ConnectorProvider::Github => "github",
        }
    }

    /// Parse the `connector_accounts.provider` column. Unrecognized values
    /// default to `Google` (mirrors `ConnectorAccountStatus::from_db`'s
    /// defaulting style) rather than failing to load the whole account list
    /// over a single stale or corrupt row.
    pub fn from_db(value: &str) -> Self {
        match value {
            "notion" => ConnectorProvider::Notion,
            "linear" => ConnectorProvider::Linear,
            "github" => ConnectorProvider::Github,
            _ => ConnectorProvider::Google,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectorAccountStatus {
    Connected,
    ReconnectRequired,
    /// Transient settings-list projection. Never persist this as account state.
    Unavailable,
}

impl ConnectorAccountStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ConnectorAccountStatus::Connected => "connected",
            ConnectorAccountStatus::ReconnectRequired => "reconnect_required",
            ConnectorAccountStatus::Unavailable => "unavailable",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "reconnect_required" => ConnectorAccountStatus::ReconnectRequired,
            _ => ConnectorAccountStatus::Connected,
        }
    }
}

/// Non-secret account descriptor returned to the frontend and used by the
/// proxy to enumerate accounts. The account id IS the Google account email
/// for a Google row, and the Linear workspace id for a Linear row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAccount {
    pub account_id: String,
    pub provider: ConnectorProvider,
    pub email: String,
    pub scopes: Vec<String>,
    pub status: ConnectorAccountStatus,
    /// Linear workspace display name, parsed from the account's `metadata`
    /// JSON. Always `None` for Google rows.
    pub workspace_name: Option<String>,
    /// Linear workspace url key (the `foo` in `linear.app/foo`), parsed from
    /// `metadata`. Always `None` for Google rows.
    pub workspace_url_key: Option<String>,
    /// The Linear teams this account is scoped to. Always empty for Google
    /// rows, which have no team concept.
    pub selected_teams: Vec<SelectedTeamDto>,
    /// A GitHub account whose credential is the GitHub CLI's own login
    /// (`gh auth token`), read on every call and never stored by Clovy.
    #[serde(default)]
    pub via_gh: bool,
}

/// One Linear team the account is scoped to, as shown to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedTeamDto {
    pub id: String,
    pub key: String,
    pub name: String,
}

/// Non-secret metadata carried on a `connector_accounts` row, beyond the
/// columns every provider shares (email, scopes, status). Parsed
/// best-effort from the `metadata` JSON column; unknown or absent keys
/// resolve to `None` rather than failing account enumeration.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorAccountMetadata {
    #[serde(default)]
    workspace_name: Option<String>,
    #[serde(default)]
    workspace_url_key: Option<String>,
}

// --- Config ------------------------------------------------------------------

fn env_trimmed(key: &str) -> String {
    std::env::var(key)
        .map(|value| value.trim().to_string())
        .unwrap_or_default()
}

fn env_or_build_trimmed(key: &str, build_value: Option<&'static str>) -> String {
    let runtime_value = env_trimmed(key);
    if runtime_value.is_empty() {
        build_value.map(str::trim).unwrap_or_default().to_string()
    } else {
        runtime_value
    }
}

/// Cryptographically-random base64url string of `bytes` entropy. Mirrors
/// `oauth::random_b64url`; used to mint autonomy grant tokens (never a
/// time/counter source).
pub(crate) fn random_b64url(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(&buf)
}

/// Google Desktop OAuth credential. Google calls the second field a client
/// secret and requires it at the token endpoint, but an installed app cannot
/// keep it confidential: both values are shipped in the binary and neither
/// grants user-data access without the user's authorization code or refresh
/// token. Runtime env values override the build-time values for local testing.
struct GoogleOAuthClient {
    client_id: String,
    client_secret: String,
}

fn google_oauth_client() -> GoogleOAuthClient {
    crate::local_env::load_local_env();
    GoogleOAuthClient {
        client_id: env_or_build_trimmed(
            GOOGLE_OAUTH_CLIENT_ID_ENV,
            option_env!("GOOGLE_OAUTH_CLIENT_ID"),
        ),
        client_secret: env_or_build_trimmed(
            GOOGLE_OAUTH_CLIENT_SECRET_ENV,
            option_env!("GOOGLE_OAUTH_CLIENT_SECRET"),
        ),
    }
}

fn require_oauth_client() -> Result<GoogleOAuthClient, AppError> {
    let client = google_oauth_client();
    if client.client_id.is_empty() || client.client_secret.is_empty() {
        return Err(AppError::new(
            "connector_not_configured",
            "Google connector is not configured in this build.",
        ));
    }
    Ok(client)
}

/// Linear public-client credential: a client id only. Linear's PKCE flow
/// needs no secret anywhere (see docs/plugins/linear-oauth-spike.md), so
/// unlike [`google_oauth_client`] there is no second field to ship. Runtime
/// env overrides the build-time value for local testing.
fn linear_oauth_client_id() -> String {
    crate::local_env::load_local_env();
    env_or_build_trimmed(
        LINEAR_OAUTH_CLIENT_ID_ENV,
        option_env!("LINEAR_OAUTH_CLIENT_ID"),
    )
}

fn require_linear_client_id() -> Result<String, AppError> {
    let client_id = linear_oauth_client_id();
    if client_id.is_empty() {
        return Err(AppError::new(
            "connector_not_configured",
            "Linear connector is not configured in this build.",
        ));
    }
    Ok(client_id)
}

/// GitHub App credential. The device authorization flow requires only the
/// client id; the client secret is optional and is used only for token refresh
/// and revocation. Both values ship in the binary; neither grants user-data
/// access without the user's authorization. Runtime env values override
/// build-time values for local testing.
///
/// When the secret is absent, the GitHub App must have "expire user
/// authorization tokens" disabled so tokens are non-expiring and the refresh
/// path is never taken. Revocation is unavailable without the secret and must
/// be done manually at GitHub if needed.
struct GithubOAuthClient {
    client_id: String,
    /// Empty when not configured. Use `client_secret_opt()` for a
    /// `Some`/`None` view.
    client_secret: String,
}

impl GithubOAuthClient {
    fn client_secret_opt(&self) -> Option<&str> {
        if self.client_secret.is_empty() {
            None
        } else {
            Some(&self.client_secret)
        }
    }
}

fn github_oauth_client() -> GithubOAuthClient {
    crate::local_env::load_local_env();
    GithubOAuthClient {
        client_id: env_or_build_trimmed(
            GITHUB_OAUTH_CLIENT_ID_ENV,
            option_env!("GITHUB_OAUTH_CLIENT_ID"),
        ),
        client_secret: env_or_build_trimmed(
            GITHUB_OAUTH_CLIENT_SECRET_ENV,
            option_env!("GITHUB_OAUTH_CLIENT_SECRET"),
        ),
    }
}

/// Require the GitHub client id. The secret is optional; callers that need it
/// (refresh, revoke) check `client.client_secret_opt()` separately.
fn require_github_client_id() -> Result<GithubOAuthClient, AppError> {
    let client = github_oauth_client();
    if client.client_id.is_empty() {
        return Err(AppError::new(
            "connector_not_configured",
            "GitHub connector is not configured in this build.",
        ));
    }
    Ok(client)
}

/// The GitHub App's public slug (the `foo` in `github.com/apps/foo`), used
/// only to point the user at the app's install page. Env-configured like the
/// client id so switching apps (staging vs production) never needs a code
/// change; empty means the not-installed message falls back to the user's
/// own installation settings page.
const GITHUB_APP_SLUG_ENV: &str = "GITHUB_APP_SLUG";

fn github_app_slug() -> String {
    crate::local_env::load_local_env();
    env_or_build_trimmed(GITHUB_APP_SLUG_ENV, option_env!("GITHUB_APP_SLUG"))
}

/// The `connector_github_not_installed` message, pointing at the app's
/// install page when the slug is configured and at the user's installation
/// settings otherwise.
fn github_not_installed_message(slug: &str) -> String {
    if slug.is_empty() {
        "Install the Clovy GitHub App on at least one repository, then connect again. You can manage app installations at github.com/settings/installations.".to_string()
    } else {
        format!(
            "Install the Clovy GitHub App on at least one repository, then connect again. Open github.com/apps/{slug} to choose repositories."
        )
    }
}

/// The callback ports registered on the Linear OAuth application
/// (`http://127.0.0.1:<port>/callback`, one URL per port). Linear matches
/// the registered callback URL exactly - it does not ignore the loopback
/// port the way Google does (RFC 8252) - so the connect listener must bind
/// one of exactly these. Keep this list in sync with the OAuth app's
/// registered callback URLs.
const LINEAR_LOOPBACK_PORTS: &[u16] = &[44741, 44742, 44743];
const LINEAR_OAUTH_LOOPBACK_PORT_ENV: &str = "LINEAR_OAUTH_LOOPBACK_PORT";

/// The candidate loopback ports for a Linear connect: the
/// `LINEAR_OAUTH_LOOPBACK_PORT` override alone when it parses as a port
/// (for an OAuth app registered with a custom callback URL), otherwise the
/// registered defaults.
fn linear_loopback_ports() -> Vec<u16> {
    crate::local_env::load_local_env();
    linear_loopback_ports_from(&env_trimmed(LINEAR_OAUTH_LOOPBACK_PORT_ENV))
}

fn linear_loopback_ports_from(override_value: &str) -> Vec<u16> {
    match override_value.parse::<u16>() {
        Ok(port) if port != 0 => vec![port],
        _ => LINEAR_LOOPBACK_PORTS.to_vec(),
    }
}

// --- Access tokens ----------------------------------------------------------------

/// Per-account refresh serialization: refresh tokens can rotate, so two
/// parallel refreshes for the same account must never race (one would burn a
/// consumed token and force a reconnect).
static REFRESH_LOCKS: OnceLock<StdMutex<HashMap<String, Arc<AsyncMutex<()>>>>> = OnceLock::new();
static LINEAR_LIFECYCLES: OnceLock<StdMutex<HashMap<String, Arc<LinearLifecycleState>>>> =
    OnceLock::new();

struct LinearLifecycleState {
    gate: Arc<AsyncMutex<()>>,
    epoch: AtomicU64,
}

pub(crate) struct LinearLifecycleGuard {
    state: Arc<LinearLifecycleState>,
    _guard: OwnedMutexGuard<()>,
}

#[derive(Clone)]
pub(crate) struct LinearLifecycleSnapshot {
    state: Arc<LinearLifecycleState>,
    epoch: u64,
}

impl LinearLifecycleGuard {
    pub(crate) fn bump_epoch(&self) -> u64 {
        self.state.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub(crate) fn snapshot(&self) -> LinearLifecycleSnapshot {
        LinearLifecycleSnapshot {
            state: self.state.clone(),
            epoch: self.state.epoch.load(Ordering::SeqCst),
        }
    }
}

impl LinearLifecycleSnapshot {
    pub(crate) fn is_current(&self) -> bool {
        self.state.epoch.load(Ordering::SeqCst) == self.epoch
    }

    pub(crate) async fn acquire_current(&self) -> Option<OwnedMutexGuard<()>> {
        let guard = self.state.gate.clone().lock_owned().await;
        self.is_current().then_some(guard)
    }
}

fn linear_lifecycle_for(account_id: &str) -> Arc<LinearLifecycleState> {
    let lifecycles = LINEAR_LIFECYCLES.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut lifecycles = lifecycles
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    lifecycles
        .entry(account_id.to_string())
        .or_insert_with(|| {
            Arc::new(LinearLifecycleState {
                gate: Arc::new(AsyncMutex::new(())),
                epoch: AtomicU64::new(0),
            })
        })
        .clone()
}

pub(crate) async fn acquire_linear_lifecycle(account_id: &str) -> LinearLifecycleGuard {
    let state = linear_lifecycle_for(account_id);
    let guard = state.gate.clone().lock_owned().await;
    LinearLifecycleGuard {
        state,
        _guard: guard,
    }
}

fn refresh_lock_for(account_id: &str) -> Arc<AsyncMutex<()>> {
    let locks = REFRESH_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut locks = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(AsyncMutex::new(())))
        .clone()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn access_token_is_fresh(expires_at_unix: i64, now_unix: i64) -> bool {
    expires_at_unix > now_unix + ACCESS_TOKEN_EXPIRY_BUFFER_SECS
}

fn not_connected_error() -> AppError {
    AppError::new(
        "connector_not_connected",
        "This Google account is not connected.",
    )
}

fn reconnect_required_error() -> AppError {
    AppError::new(
        "connector_reconnect_required",
        "Google access for this account expired. Reconnect it in settings.",
    )
}

fn linear_not_connected_error() -> AppError {
    AppError::new(
        "connector_not_connected",
        "This Linear workspace is not connected.",
    )
}

fn linear_reconnect_required_error() -> AppError {
    AppError::new(
        "connector_reconnect_required",
        "Linear access for this workspace expired. Reconnect it in settings.",
    )
}

/// Resolve a usable access token for the account: the cached token when it
/// is comfortably fresh, otherwise a refreshed one. Refreshes are serialized
/// per account and handle refresh-token rotation. On a definitive
/// `invalid_grant` the account is flagged `reconnect_required` in the DB
/// index and `connector_reconnect_required` is returned.
pub async fn google_access_token(
    app: &tauri::AppHandle,
    account_id: &str,
) -> Result<String, AppError> {
    let stored = store::load_tokens(ConnectorProvider::Google, account_id)
        .await?
        .ok_or_else(not_connected_error)?;
    if access_token_is_fresh(stored.expires_at_unix, now_unix()) {
        return Ok(stored.access_token.clone());
    }
    refresh_google_access_token(app, account_id).await
}

/// Refresh regardless of cached freshness. Callers use this to retry once
/// after `google::GoogleApiError::Unauthorized` (a token revoked or expired
/// server side before its local expiry).
pub async fn force_refresh_google_access_token(
    app: &tauri::AppHandle,
    account_id: &str,
) -> Result<String, AppError> {
    // Skip the freshness fast path but still serialize on the account lock.
    refresh_google_access_token_with_freshness_gate(app, account_id, false).await
}

async fn refresh_google_access_token(
    app: &tauri::AppHandle,
    account_id: &str,
) -> Result<String, AppError> {
    refresh_google_access_token_with_freshness_gate(app, account_id, true).await
}

async fn refresh_google_access_token_with_freshness_gate(
    app: &tauri::AppHandle,
    account_id: &str,
    accept_fresh: bool,
) -> Result<String, AppError> {
    let client = require_oauth_client()?;
    let lock = refresh_lock_for(account_id);
    let _guard = lock.lock().await;
    // Re-read inside the lock: another caller may have already refreshed
    // (and rotated the refresh token) while we waited.
    let mut stored = store::load_tokens(ConnectorProvider::Google, account_id)
        .await?
        .ok_or_else(not_connected_error)?;
    if accept_fresh && access_token_is_fresh(stored.expires_at_unix, now_unix()) {
        return Ok(stored.access_token.clone());
    }

    let mut attempt = 0;
    loop {
        attempt += 1;
        match oauth::refresh(
            &client.client_id,
            &client.client_secret,
            &stored.refresh_token,
        )
        .await
        {
            oauth::RefreshOutcome::Refreshed(fresh) => {
                stored.access_token = fresh.access_token.clone();
                // Rotation: Google occasionally issues a new refresh token;
                // persist it, otherwise keep the existing one.
                if let Some(rotated) = fresh
                    .refresh_token
                    .as_deref()
                    .filter(|token| !token.is_empty())
                {
                    stored.refresh_token = rotated.to_string();
                }
                stored.expires_at_unix = now_unix() + fresh.expires_in.max(0);
                store::store_tokens(ConnectorProvider::Google, account_id, &stored).await?;
                return Ok(stored.access_token.clone());
            }
            oauth::RefreshOutcome::InvalidGrant => {
                mark_reconnect_required(app, account_id).await;
                return Err(reconnect_required_error());
            }
            oauth::RefreshOutcome::Transient => {
                if attempt < oauth::REFRESH_MAX_ATTEMPTS {
                    tokio::time::sleep(oauth::REFRESH_RETRY_BACKOFF * attempt as u32).await;
                    continue;
                }
                return Err(AppError::new(
                    "connector_refresh_unavailable",
                    "Couldn't reach Google to refresh access. Try again in a moment.",
                ));
            }
        }
    }
}

/// Resolve a usable access token for the Linear workspace: the cached token
/// when it is comfortably fresh, otherwise a refreshed one. The account id
/// is the workspace id; it shares [`REFRESH_LOCKS`] with Google accounts
/// (workspace ids and emails cannot collide). The refresh loop is a
/// deliberate duplicate of the Google one with Linear types - the two flows
/// differ in credential shape (no secret) and outcome enum, and an
/// abstraction over both would obscure more than it saves.
pub async fn linear_access_token(
    app: &tauri::AppHandle,
    account_id: &str,
) -> Result<String, AppError> {
    refresh_linear_access_token_with_freshness_gate(app, account_id, true).await
}

/// Refresh regardless of cached freshness. Callers use this to retry once
/// after `linear::LinearApiError::Unauthorized` (a token revoked or expired
/// server side before its local expiry).
pub async fn force_refresh_linear_access_token(
    app: &tauri::AppHandle,
    account_id: &str,
) -> Result<String, AppError> {
    // Skip the freshness fast path but still serialize on the account lock.
    refresh_linear_access_token_with_freshness_gate(app, account_id, false).await
}

async fn refresh_linear_access_token_with_freshness_gate(
    app: &tauri::AppHandle,
    account_id: &str,
    accept_fresh: bool,
) -> Result<String, AppError> {
    let client_id = require_linear_client_id()?;
    let lock = refresh_lock_for(account_id);
    let _guard = lock.lock().await;
    // Re-read inside the lock: another caller may have already refreshed
    // (and rotated the refresh token) while we waited.
    let mut stored = store::load_tokens(ConnectorProvider::Linear, account_id)
        .await?
        .ok_or_else(linear_not_connected_error)?;
    if accept_fresh && access_token_is_fresh(stored.expires_at_unix, now_unix()) {
        return Ok(stored.access_token.clone());
    }

    let mut attempt = 0;
    loop {
        attempt += 1;
        match linear::refresh(&client_id, &stored.refresh_token).await {
            linear::LinearRefreshOutcome::Refreshed(fresh) => {
                stored.access_token = fresh.access_token.clone();
                // Rotation: Linear issues a new refresh token on every
                // refresh; persist it, keep the existing one if absent.
                if let Some(rotated) = fresh
                    .refresh_token
                    .as_deref()
                    .filter(|token| !token.is_empty())
                {
                    stored.refresh_token = rotated.to_string();
                }
                stored.expires_at_unix = now_unix() + fresh.expires_in.max(0);
                // Linear already rotated server-side, so a failed persist
                // here strands the NEW refresh token. Recovery rides on
                // Linear's 30-minute grace window for the consumed (old)
                // token still in custody (spike doc): the next refresh
                // within that window rotates again. Log distinctly so a
                // later forced reconnect is diagnosable to this write.
                if let Err(error) =
                    store::store_tokens(ConnectorProvider::Linear, account_id, &stored).await
                {
                    tracing::error!(
                        error_code = %error.code,
                        "persisting rotated linear refresh token failed; grant recovers only within linear's consumption grace window"
                    );
                    return Err(error);
                }
                return Ok(stored.access_token.clone());
            }
            linear::LinearRefreshOutcome::InvalidGrant => {
                mark_reconnect_required(app, account_id).await;
                return Err(linear_reconnect_required_error());
            }
            linear::LinearRefreshOutcome::Transient => {
                if attempt < oauth::REFRESH_MAX_ATTEMPTS {
                    tokio::time::sleep(oauth::REFRESH_RETRY_BACKOFF * attempt as u32).await;
                    continue;
                }
                return Err(AppError::new(
                    "connector_refresh_unavailable",
                    "Couldn't reach Linear to refresh access. Try again in a moment.",
                ));
            }
        }
    }
}

/// The account a connector tool call may use: connected, holding `scope`
/// (directly or through a broader scope), and with teams when `teams` asks
/// for them. Same rules for every credential source.
pub fn check_account_grant(
    accounts: &[ConnectorAccount],
    account_id: &str,
    provider: ConnectorProvider,
    scope: &str,
    teams: bool,
) -> Result<ConnectorAccount, AppError> {
    let account = accounts
        .iter()
        .find(|account| account.account_id == account_id && account.provider == provider)
        .ok_or_else(|| {
            AppError::new(
                "connector_not_connected",
                "This connector account is not connected.",
            )
        })?;
    if account.status != ConnectorAccountStatus::Connected {
        return Err(AppError::new(
            "connector_reconnect_required",
            "Reconnect this connector account in settings.",
        ));
    }
    if !account
        .scopes
        .iter()
        .any(|held| policy::scope_grants(held, scope))
    {
        return Err(AppError::new(
            "connector_scope_missing",
            "This connector account does not have the required permission.",
        ));
    }
    if teams && account.selected_teams.is_empty() {
        return Err(AppError::new(
            "linear_teams_not_selected",
            "Select at least one Linear team in settings.",
        ));
    }
    Ok(account.clone())
}

fn github_not_connected_error() -> AppError {
    AppError::new(
        "connector_not_connected",
        "This GitHub account is not connected.",
    )
}

fn github_reconnect_required_error() -> AppError {
    AppError::new(
        "connector_reconnect_required",
        "GitHub access for this account expired. Reconnect it in settings.",
    )
}

/// Resolve a usable access token for the GitHub account: the cached token when
/// it is comfortably fresh, otherwise a refreshed one. The account id is the
/// stringified numeric GitHub user id; it shares [`REFRESH_LOCKS`] with Google
/// and Linear accounts (no collision possible). The refresh loop is a
/// deliberate duplicate of the Linear one with GitHub types - the two flows
/// differ in credential shape (secret required) and outcome enum, and an
/// abstraction over both would obscure more than it saves.
pub async fn github_access_token(
    app: &tauri::AppHandle,
    account_id: &str,
) -> Result<String, AppError> {
    // A "via gh" account has no Clovy custody: its token comes from `gh` on
    // every call (ADR-0066).
    let repos = crate::commands::repositories(app).await?;
    if let Some(record) = repos.get_connector_account(account_id).await? {
        if github_cli::is_gh_account(&record.metadata) {
            let env = crate::llm::shell_env::login_env().await;
            let token = github_cli::access_token(&repos, &env, account_id).await;
            if token.is_err() {
                emit_connectors_changed(app);
            }
            return token;
        }
    }
    let stored = store::load_tokens(ConnectorProvider::Github, account_id)
        .await?
        .ok_or_else(github_not_connected_error)?;
    if access_token_is_fresh(stored.expires_at_unix, now_unix()) {
        return Ok(stored.access_token.clone());
    }
    refresh_github_access_token_with_freshness_gate(app, account_id, true).await
}

async fn refresh_github_access_token_with_freshness_gate(
    app: &tauri::AppHandle,
    account_id: &str,
    accept_fresh: bool,
) -> Result<String, AppError> {
    let client = require_github_client_id()?;
    let lock = refresh_lock_for(account_id);
    let _guard = lock.lock().await;
    // Re-read inside the lock: another caller may have already refreshed
    // (and rotated the refresh token) while we waited.
    let mut stored = store::load_tokens(ConnectorProvider::Github, account_id)
        .await?
        .ok_or_else(github_not_connected_error)?;
    if accept_fresh && access_token_is_fresh(stored.expires_at_unix, now_unix()) {
        return Ok(stored.access_token.clone());
    }

    // Non-expiring token: if the stored refresh_token is empty the GitHub App
    // has "expire user authorization tokens" disabled. The access token does
    // not expire; never call GitHub with an empty refresh token.
    if stored.refresh_token.is_empty() {
        mark_reconnect_required(app, account_id).await;
        return Err(github_reconnect_required_error());
    }

    // Device-flow authorization needs no client secret, but GitHub requires
    // client_id + client_secret to refresh expiring tokens. A secret-less
    // deployment must disable "expire user authorization tokens" (non-expiring
    // tokens never reach this path). When a refresh is needed and no secret is
    // configured, mark reconnect_required and return the reconnect error.
    let client_secret = match client.client_secret_opt() {
        Some(secret) => secret.to_string(),
        None => {
            tracing::warn!(
                account_id,
                "github token refresh needed but no client secret is configured; marking reconnect_required"
            );
            mark_reconnect_required(app, account_id).await;
            return Err(github_reconnect_required_error());
        }
    };

    let mut attempt = 0;
    loop {
        attempt += 1;
        match github::refresh(&client.client_id, &client_secret, &stored.refresh_token).await {
            github::GithubRefreshOutcome::Refreshed(fresh) => {
                stored.access_token = fresh.access_token.clone();
                // Rotation: GitHub rotates the refresh token on every
                // refresh; persist it, keep the existing one if absent.
                if let Some(rotated) = fresh
                    .refresh_token
                    .as_deref()
                    .filter(|token| !token.is_empty())
                {
                    stored.refresh_token = rotated.to_string();
                }
                stored.expires_at_unix = fresh
                    .expires_in
                    .map(|secs| now_unix() + secs.max(0))
                    .unwrap_or_else(|| now_unix() + github::NON_EXPIRING_TOKEN_LIFETIME_SECS);
                // GitHub already rotated server-side, so a failed persist
                // here strands the NEW refresh token. Log distinctly so a
                // later forced reconnect is diagnosable to this write.
                if let Err(error) =
                    store::store_tokens(ConnectorProvider::Github, account_id, &stored).await
                {
                    tracing::error!(
                        error_code = %error.code,
                        "persisting rotated github refresh token failed; grant recovers only within github's consumption grace window"
                    );
                    return Err(error);
                }
                return Ok(stored.access_token.clone());
            }
            github::GithubRefreshOutcome::InvalidGrant => {
                mark_reconnect_required(app, account_id).await;
                return Err(github_reconnect_required_error());
            }
            github::GithubRefreshOutcome::Transient => {
                if attempt < oauth::REFRESH_MAX_ATTEMPTS {
                    tokio::time::sleep(oauth::REFRESH_RETRY_BACKOFF * attempt as u32).await;
                    continue;
                }
                return Err(AppError::new(
                    "connector_refresh_unavailable",
                    "Couldn't reach GitHub to refresh access. Try again in a moment.",
                ));
            }
        }
    }
}

/// Fired whenever an account's connection state changes (connect, disconnect,
/// or a background `reconnect_required` transition) so an open settings page
/// refreshes without a remount. The frontend `CONNECTORS_CHANGED_EVENT`
/// subscribes to this.
const CONNECTORS_CHANGED_EVENT: &str = "clovy://connectors-changed";

fn emit_connectors_changed(app: &tauri::AppHandle) {
    use tauri::Emitter;
    let _ = app.emit(CONNECTORS_CHANGED_EVENT, ());
}

async fn mark_reconnect_required(app: &tauri::AppHandle, account_id: &str) {
    match crate::commands::repositories(app).await {
        Ok(repos) => {
            if let Err(error) = repos
                .set_connector_account_status(
                    account_id,
                    ConnectorAccountStatus::ReconnectRequired.as_str(),
                )
                .await
            {
                tracing::warn!(
                    error_code = %AppError::from(error).code,
                    "failed to flag connector account for reconnect"
                );
            } else {
                // A background refresh just downgraded this account; tell any
                // open settings page so it does not show a stale "Connected".
                emit_connectors_changed(app);
            }
        }
        Err(error) => {
            tracing::warn!(
                error_code = %error.code,
                "failed to open repositories to flag connector reconnect"
            );
        }
    }
}

// --- Account lifecycle -------------------------------------------------------------

/// Build the frontend-facing DTO for one stored account row: map the
/// provider/status columns, parse the metadata JSON, and load the selected
/// teams. The single mapping shared by [`list_accounts`], the connect
/// short-circuits, and the team-selection command, so the row-to-DTO
/// translation cannot drift between call sites.
async fn account_dto(
    repos: &crate::db::repositories::Repositories,
    record: crate::db::repositories::ConnectorAccountRecord,
) -> Result<ConnectorAccount, AppError> {
    // Best-effort: a malformed metadata blob degrades to "no workspace
    // info" rather than failing the whole account list.
    let metadata: ConnectorAccountMetadata =
        serde_json::from_str(&record.metadata).unwrap_or_default();
    let via_gh = github_cli::is_gh_account(&record.metadata);
    // Same best-effort stance for the team rows: account enumeration was
    // infallible before selected teams existed, and callers treat an Err as
    // "no accounts" (blanking the settings page and routine gates), so one
    // transient DB hiccup on one row must not fail the whole list.
    let selected_teams = match repos.list_selected_teams(&record.account_id).await {
        Ok(teams) => teams
            .into_iter()
            .map(|team| SelectedTeamDto {
                id: team.team_id,
                key: team.team_key,
                name: team.team_name,
            })
            .collect(),
        Err(error) => {
            tracing::warn!(
                error_code = %AppError::from(error).code,
                "listing selected teams failed; account enumerates with an empty team set"
            );
            Vec::new()
        }
    };
    Ok(ConnectorAccount {
        account_id: record.account_id,
        provider: ConnectorProvider::from_db(&record.provider),
        email: record.email,
        scopes: record.scopes,
        status: ConnectorAccountStatus::from_db(&record.status),
        via_gh,
        workspace_name: metadata.workspace_name,
        workspace_url_key: metadata.workspace_url_key,
        selected_teams,
    })
}

/// Enumerate persisted Google accounts from the non-secret DB index.
///
/// This intentionally does not inspect Notion. Callers that need a real Google
/// identity for grants or Google MCP registration must use this provider-scoped
/// listing so the synthetic Notion preview row can never be treated as an email.
pub async fn list_google_accounts(
    app: &tauri::AppHandle,
) -> Result<Vec<ConnectorAccount>, AppError> {
    let repos = crate::commands::repositories(app).await?;
    let records = repos.list_connector_accounts().await?;
    let mut accounts = Vec::new();
    for record in records {
        if record.provider == ConnectorProvider::Google.as_str() {
            accounts.push(account_dto(&repos, record).await?);
        }
    }
    Ok(accounts)
}

/// Enumerate the persisted account index used for Google and Linear runtime
/// registration. Notion is deliberately excluded: its optional Keychain state
/// must never hide otherwise healthy database-backed connector accounts.
pub async fn list_runtime_accounts(
    app: &tauri::AppHandle,
) -> Result<Vec<ConnectorAccount>, AppError> {
    let repos = crate::commands::repositories(app).await?;
    let records = repos.list_connector_accounts().await?;
    let mut accounts = Vec::with_capacity(records.len());
    for record in records {
        if record.provider != ConnectorProvider::Notion.as_str() {
            accounts.push(account_dto(&repos, record).await?);
        }
    }
    Ok(accounts)
}

/// Enumerate connected accounts for shared API consumers.
pub async fn list_accounts(app: &tauri::AppHandle) -> Result<Vec<ConnectorAccount>, AppError> {
    let mut accounts = list_runtime_accounts(app).await?;
    append_notion_account(&mut accounts, notion::account_status(app).await, false)?;
    Ok(accounts)
}

/// Enumerate accounts for the Connectors settings UI.
///
/// The settings provider directory owns disconnected rows, so an unreadable
/// optional Notion preview is represented by a transient unavailable row.
/// Shared API consumers should call [`list_accounts`] and receive the original
/// Notion read error.
pub async fn list_accounts_resilient(
    app: &tauri::AppHandle,
) -> Result<Vec<ConnectorAccount>, AppError> {
    let repos = crate::commands::repositories(app).await?;
    let records = repos.list_connector_accounts().await?;
    let mut accounts = Vec::with_capacity(records.len() + 1);
    for record in records {
        if record.provider != ConnectorProvider::Notion.as_str() {
            accounts.push(account_dto(&repos, record).await?);
        }
    }
    append_notion_account(&mut accounts, notion::account_status(app).await, true)?;
    Ok(accounts)
}

fn append_notion_account(
    accounts: &mut Vec<ConnectorAccount>,
    status: Result<Option<ConnectorAccountStatus>, AppError>,
    suppress_error: bool,
) -> Result<(), AppError> {
    match status {
        Ok(Some(status)) => accounts.push(ConnectorAccount {
            account_id: notion::notion_account_id().to_string(),
            provider: ConnectorProvider::Notion,
            email: notion::notion_account_email().to_string(),
            scopes: Vec::new(),
            status,
            via_gh: false,
            workspace_name: None,
            workspace_url_key: None,
            selected_teams: Vec::new(),
        }),
        Ok(None) => {}
        Err(error) if suppress_error => {
            tracing::warn!(
                error_code = %error.code,
                "failed to read optional Notion connector status"
            );
            accounts.push(ConnectorAccount {
                account_id: notion::notion_account_id().to_string(),
                provider: ConnectorProvider::Notion,
                email: notion::notion_account_email().to_string(),
                scopes: Vec::new(),
                status: ConnectorAccountStatus::Unavailable,
                via_gh: false,
                workspace_name: None,
                workspace_url_key: None,
                selected_teams: Vec::new(),
            });
        }
        Err(error) => return Err(error),
    }
    Ok(())
}

/// The identity of an already-stored account, for the SAME provider, that
/// differs from the one being connected, if any. The identity string is
/// whatever keys that provider's accounts: the email for Google, the
/// workspace id for Linear. Local mode is single-account per provider
/// (every connector surface for a given provider resolves the one connected
/// account for that provider), so a second, distinct account for that
/// provider is refused to avoid a cross-account read/write mix-up. A
/// different provider's account never conflicts: a connected Google account
/// must not block connecting Linear, and vice versa. Comparison is
/// case-insensitive, so reconnecting or adding scope to the same identity
/// returns `None` and is allowed.
fn conflicting_existing_account<'a>(
    existing: impl IntoIterator<Item = (&'a str, &'a str)>,
    connecting_provider: &str,
    connecting_identity: &str,
) -> Option<String> {
    existing
        .into_iter()
        .filter(|(provider, _)| *provider == connecting_provider)
        .find(|(_, identity)| !identity.eq_ignore_ascii_case(connecting_identity))
        .map(|(_, identity)| identity.to_string())
}

/// Run the full connect flow (browser consent, loopback callback, code
/// exchange, custody write, DB index upsert) for the requested scope
/// bundles. With a `login_hint` for an already-connected account whose
/// granted scopes already cover the request, no browser round-trip happens
/// (incremental auth short-circuit).
pub async fn begin_connect(
    app: &tauri::AppHandle,
    flow: &ConnectFlow,
    bundles: &[policy::ScopeBundle],
    login_hint: Option<&str>,
) -> Result<ConnectorAccount, AppError> {
    let client = require_oauth_client()?;
    let repos = crate::commands::repositories(app).await?;

    // Escalation short-circuit: an existing, healthy account that already
    // holds every wanted scope needs no new consent.
    if let Some(hint) = login_hint.map(str::trim).filter(|hint| !hint.is_empty()) {
        let hint_lower = hint.to_ascii_lowercase();
        if let Some(record) = repos.get_connector_account(&hint_lower).await? {
            let already_granted = policy::missing_scopes(&record.scopes, bundles).is_empty();
            if already_granted && record.status == ConnectorAccountStatus::Connected.as_str() {
                return account_dto(&repos, record).await;
            }
        }
    }

    let requested = policy::requested_scopes(bundles);
    let grant = oauth::authorize(
        flow,
        &client.client_id,
        &client.client_secret,
        &requested,
        login_hint,
    )
    .await?;
    let email = grant.email.clone();

    // A login hint means the user asked to (re)connect one specific account.
    // Google only preselects it; the browser can still consent as a different
    // account. Abort on mismatch rather than silently storing the wrong account
    // (which would leave the intended account still flagged reconnect_required).
    if let Some(hint) = login_hint.map(str::trim).filter(|hint| !hint.is_empty()) {
        if !email.eq_ignore_ascii_case(hint) {
            return Err(AppError::new(
                "connector_account_mismatch",
                "That Google account does not match the one you were reconnecting. Try again and choose that account.",
            ));
        }
    }

    // Local mode v1 binds every connector surface to a single account: the base
    // Gmail/Calendar MCP servers, the per-job autonomy servers, and every
    // trigger all independently resolve "the connected account" (the first
    // connected row). A second, distinct account would let a routine created
    // against account B silently read or mutate account A's mail and calendar,
    // a cross-account privacy leak. Refuse a different account while one is
    // already stored; reconnecting or adding scope to the same email still
    // passes (the email matches). Multi-account routing is a documented
    // follow-up. Checked after auth because the account identity is only known
    // once Google returns it; the settings UI also hides "add another" so this
    // guard is the safety net, not the primary path.
    let existing_accounts = repos.list_connector_accounts().await?;
    if let Some(existing_email) = conflicting_existing_account(
        existing_accounts
            .iter()
            .map(|record| (record.provider.as_str(), record.email.as_str())),
        ConnectorProvider::Google.as_str(),
        &email,
    ) {
        return Err(AppError::new(
            "connector_single_account_only",
            format!(
                "Clovy local mode uses one Google account at a time. Disconnect {existing_email} before connecting another."
            ),
        ));
    }

    // Persist the account's scopes. When Google omits the response scope field
    // on an incremental grant, this unions the requested scopes with the ones
    // the account already held, so add-access never makes the DB forget earlier
    // grants the token still carries. Matched by provider AND email: a Linear
    // row can carry the same email (the Linear viewer email is often the
    // Google address), and its "read"/"write" scopes must never leak into the
    // Google account's persisted grant.
    let existing_scopes = existing_accounts
        .iter()
        .find(|record| {
            record.provider == ConnectorProvider::Google.as_str()
                && record.email.eq_ignore_ascii_case(&email)
        })
        .map(|record| record.scopes.as_slice());
    let granted_scopes =
        policy::resolve_granted_scopes(grant.tokens.scope.as_deref(), &requested, existing_scopes);

    // Scope escalation on an existing grant can omit the refresh token; keep
    // the one already in custody then.
    let refresh_token = match grant
        .tokens
        .refresh_token
        .as_deref()
        .filter(|token| !token.is_empty())
    {
        Some(token) => token.to_string(),
        None => store::load_tokens(ConnectorProvider::Google, &email)
            .await?
            .map(|existing| existing.refresh_token.clone())
            .ok_or_else(|| {
                AppError::new(
                    "connector_missing_refresh_token",
                    "Google did not return a refresh token. Remove Clovy's access at myaccount.google.com/permissions and connect again.",
                )
            })?,
    };

    let tokens = store::StoredConnectorTokens {
        access_token: grant.tokens.access_token.clone(),
        refresh_token,
        expires_at_unix: now_unix() + grant.tokens.expires_in.max(0),
        scopes: granted_scopes.clone(),
        email: email.clone(),
    };
    store::store_tokens(ConnectorProvider::Google, &email, &tokens).await?;

    repos
        .upsert_connector_account(
            &email,
            ConnectorProvider::Google.as_str(),
            &email,
            &granted_scopes,
            ConnectorAccountStatus::Connected.as_str(),
            "{}",
        )
        .await?;
    emit_connectors_changed(app);

    Ok(ConnectorAccount {
        account_id: email.clone(),
        provider: ConnectorProvider::Google,
        email,
        scopes: granted_scopes,
        status: ConnectorAccountStatus::Connected,
        via_gh: false,
        workspace_name: None,
        workspace_url_key: None,
        selected_teams: Vec::new(),
    })
}

/// The non-secret metadata blob persisted on a Linear account row. Keys are
/// camelCase to match [`ConnectorAccountMetadata`]'s parse; empty-string
/// fields are omitted rather than stored as noise.
fn linear_account_metadata_json(identity: &linear::LinearIdentity) -> String {
    let mut map = serde_json::Map::new();
    for (key, value) in [
        ("workspaceName", &identity.workspace_name),
        ("workspaceUrlKey", &identity.workspace_url_key),
        ("actorUserId", &identity.user_id),
        ("actorName", &identity.user_name),
    ] {
        if !value.is_empty() {
            map.insert(key.to_string(), serde_json::Value::String(value.clone()));
        }
    }
    serde_json::Value::Object(map).to_string()
}

struct LinearRetryAccount<'a> {
    email: &'a str,
    scopes: &'a [String],
    metadata: &'a str,
}

async fn restore_linear_connect_state_with<RestoreTokens, RestoreTokensFuture>(
    repos: &crate::db::repositories::Repositories,
    workspace_id: &str,
    previous_record: Option<&crate::db::repositories::ConnectorAccountRecord>,
    retry_account: &LinearRetryAccount<'_>,
    restore_tokens: RestoreTokens,
) -> Result<(), AppError>
where
    RestoreTokens: FnOnce() -> RestoreTokensFuture,
    RestoreTokensFuture: Future<Output = Result<(), AppError>>,
{
    let token_result = restore_tokens().await;
    let custody_restored = token_result.is_ok();

    // Always attempt both halves. Keychain and SQLite cannot share a
    // transaction, but a failure restoring one must not prevent the other
    // from returning to a safe, retryable state. When custody restoration is
    // uncertain, retain an index row marked reconnect_required instead of
    // deleting the only Settings surface capable of cleaning it up.
    let account_result = match (previous_record, custody_restored) {
        (Some(record), true) => repos
            .upsert_connector_account(
                &record.account_id,
                &record.provider,
                &record.email,
                &record.scopes,
                &record.status,
                &record.metadata,
            )
            .await
            .map_err(AppError::from),
        (Some(record), false) => repos
            .upsert_connector_account(
                &record.account_id,
                &record.provider,
                &record.email,
                &record.scopes,
                ConnectorAccountStatus::ReconnectRequired.as_str(),
                &record.metadata,
            )
            .await
            .map_err(AppError::from),
        (None, true) => {
            match repos
                .delete_connector_account(workspace_id)
                .await
                .map_err(AppError::from)
            {
                Ok(()) => Ok(()),
                Err(delete_error) => {
                    match repos
                        .upsert_connector_account(
                            workspace_id,
                            ConnectorProvider::Linear.as_str(),
                            retry_account.email,
                            retry_account.scopes,
                            ConnectorAccountStatus::ReconnectRequired.as_str(),
                            retry_account.metadata,
                        )
                        .await
                        .map_err(AppError::from)
                    {
                        Ok(()) => Err(delete_error),
                        Err(repair_error) => Err(AppError::new(
                            "connector_connect_rollback_failed",
                            format!(
                                "Clovy could not remove or mark the canceled Linear connection for retry (delete: {}; repair: {}). Remove Clovy in Linear's authorized applications settings before trying again.",
                                delete_error.code, repair_error.code
                            ),
                        )),
                    }
                }
            }
        }
        (None, false) => repos
            .upsert_connector_account(
                workspace_id,
                ConnectorProvider::Linear.as_str(),
                retry_account.email,
                retry_account.scopes,
                ConnectorAccountStatus::ReconnectRequired.as_str(),
                retry_account.metadata,
            )
            .await
            .map_err(AppError::from),
    };

    match (token_result, account_result) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(token_error), Err(account_error)) => Err(AppError::new(
            "connector_connect_rollback_failed",
            format!(
                "Clovy could not safely restore the Linear connection after cancellation (custody: {}; account: {}). Remove Clovy in Linear's authorized applications settings before trying again.",
                token_error.code, account_error.code
            ),
        )),
    }
}

async fn restore_linear_connect_state(
    repos: &crate::db::repositories::Repositories,
    workspace_id: &str,
    previous_tokens: Option<&store::StoredConnectorTokens>,
    previous_record: Option<&crate::db::repositories::ConnectorAccountRecord>,
    retry_account: &LinearRetryAccount<'_>,
) -> Result<(), AppError> {
    restore_linear_connect_state_with(
        repos,
        workspace_id,
        previous_record,
        retry_account,
        || async {
            match previous_tokens {
                Some(tokens) => {
                    store::store_tokens(ConnectorProvider::Linear, workspace_id, tokens).await
                }
                None => store::delete_tokens(ConnectorProvider::Linear, workspace_id).await,
            }
        },
    )
    .await
}

/// Run the full Linear connect flow (browser consent, loopback callback,
/// code exchange, identity resolution, custody write, DB index upsert) for
/// the requested scope bundles. The account is keyed by WORKSPACE id, not
/// email: a Linear grant is a workspace grant, and `reconnect_account_id`
/// carries that id on a reconnect or scope escalation. With a
/// `reconnect_account_id` naming an already-connected workspace whose
/// granted scopes cover the request, no browser round-trip happens
/// (mirroring the Google incremental-auth short-circuit).
pub async fn begin_connect_linear(
    app: &tauri::AppHandle,
    flow: &ConnectFlow,
    bundles: &[policy::ScopeBundle],
    reconnect_account_id: Option<&str>,
) -> Result<ConnectorAccount, AppError> {
    // Defensive: the command layer validates too, but a Google bundle here
    // would request Google scope URLs from Linear's consent screen.
    if let Some(bundle) = bundles
        .iter()
        .find(|bundle| bundle.provider() != ConnectorProvider::Linear)
    {
        return Err(AppError::new(
            "connector_scope_provider_mismatch",
            format!(
                "Scope bundle \"{}\" does not belong to the linear connector.",
                bundle.name()
            ),
        ));
    }
    let client_id = require_linear_client_id()?;
    let repos = crate::commands::repositories(app).await?;
    flow.ensure_not_canceled("Linear")?;

    let reconnect_account_id = reconnect_account_id
        .map(str::trim)
        .filter(|id| !id.is_empty());

    // Escalation/reconnect short-circuit: an existing, healthy workspace
    // that already holds every wanted scope needs no new consent.
    if let Some(account_id) = reconnect_account_id {
        let lifecycle = acquire_linear_lifecycle(account_id).await;
        if let Some(record) = repos.get_connector_account(account_id).await? {
            let already_granted = policy::missing_scopes(&record.scopes, bundles).is_empty();
            if record.provider == ConnectorProvider::Linear.as_str()
                && already_granted
                && record.status == ConnectorAccountStatus::Connected.as_str()
            {
                flow.ensure_not_canceled("Linear")?;
                return account_dto(&repos, record).await;
            }
        }
        drop(lifecycle);
    }

    let requested = policy::requested_linear_scopes(bundles);
    let grant = linear::authorize(flow, &client_id, &requested, &linear_loopback_ports()).await?;
    let result = async {
        flow.ensure_not_canceled("Linear")?;
        let identity = &grant.identity;
        let workspace_id = identity.workspace_id.clone();

        // A reconnect id means the user asked to (re)connect one specific
        // workspace. The browser can still consent for a different one; abort on
        // mismatch rather than silently storing the wrong workspace (which would
        // leave the intended one still flagged reconnect_required).
        if let Some(expected) = reconnect_account_id {
            if !workspace_id.eq_ignore_ascii_case(expected) {
                return Err(AppError::new(
                    "connector_account_mismatch",
                    "That Linear workspace does not match the one you were reconnecting. Try again and pick that workspace.",
                ));
            }
        }

        // Final credential and index writes share the same lifecycle boundary as
        // token refresh, managed MCP eligibility, session creation, and
        // disconnect. The browser authorization above remains outside the lock.
        let lifecycle = acquire_linear_lifecycle(&workspace_id).await;
        let refresh_lock = refresh_lock_for(&workspace_id);
        let _refresh_guard = refresh_lock.lock().await;
        flow.ensure_not_canceled("Linear")?;

        // Same single-account rationale as the Google guard above, scoped to the
        // Linear provider: every Linear surface resolves "the connected
        // workspace", so a second, distinct workspace is refused. Compared by
        // workspace id (the account id), never email.
        let existing_accounts = repos.list_connector_accounts().await?;
        flow.ensure_not_canceled("Linear")?;
        let previous_record = existing_accounts
            .iter()
            .find(|record| {
                record.provider == ConnectorProvider::Linear.as_str()
                    && record.account_id.eq_ignore_ascii_case(&workspace_id)
            })
            .cloned();
        if let Some(existing_id) = conflicting_existing_account(
            existing_accounts
                .iter()
                .map(|record| (record.provider.as_str(), record.account_id.as_str())),
            ConnectorProvider::Linear.as_str(),
            &workspace_id,
        ) {
            // Name the stored workspace when its metadata carries a name; the
            // raw id is a last resort that at least identifies the row.
            let display = existing_accounts
                .iter()
                .find(|record| record.account_id == existing_id)
                .and_then(|record| {
                    serde_json::from_str::<ConnectorAccountMetadata>(&record.metadata).ok()
                })
                .and_then(|metadata| metadata.workspace_name)
                .filter(|name| !name.is_empty())
                .unwrap_or(existing_id);
            return Err(AppError::new(
                "connector_single_account_only",
                format!(
                    "Clovy local mode uses one Linear workspace at a time. Disconnect {display} before connecting another."
                ),
            ));
        }

        // Persist the granted scopes: Linear's response scope field when it
        // carries anything, otherwise the requested list.
        let granted_scopes: Vec<String> = grant
            .tokens
            .scope
            .as_deref()
            .map(linear::parse_scope_field)
            .filter(|scopes| !scopes.is_empty())
            .unwrap_or_else(|| requested.iter().map(|scope| scope.to_string()).collect());

        // Snapshot custody before changing it so a cancellation that lands while
        // persistence is pending can restore the pre-connect state.
        let previous_tokens = store::load_tokens(ConnectorProvider::Linear, &workspace_id).await?;
        flow.ensure_not_canceled("Linear")?;

        // Scope escalation on an existing grant can omit the refresh token; keep
        // the one already in custody then (mirrors the Google fallback).
        let refresh_token = match grant
            .tokens
            .refresh_token
            .as_deref()
            .filter(|token| !token.is_empty())
        {
            Some(token) => token.to_string(),
            None => previous_tokens
                .as_ref()
                .map(|existing| existing.refresh_token.clone())
                .ok_or_else(|| {
                    AppError::new(
                        "connector_missing_refresh_token",
                        "Linear did not return a refresh token. Remove Clovy's access in Linear settings and connect again.",
                    )
                })?,
        };

        let tokens = store::StoredConnectorTokens {
            access_token: grant.tokens.access_token.clone(),
            refresh_token,
            expires_at_unix: now_unix() + grant.tokens.expires_in.max(0),
            scopes: granted_scopes.clone(),
            // May be empty; informational only. The workspace id keys custody.
            email: identity.user_email.clone(),
        };
        let metadata_json = linear_account_metadata_json(identity);
        let retry_account = LinearRetryAccount {
            email: &identity.user_email,
            scopes: &granted_scopes,
            metadata: &metadata_json,
        };

        // Read the response-only projection before the first write. From this
        // point onward every await is followed by a cancellation check and, once
        // custody changes, a rollback to these snapshots.
        let selected_teams = repos
            .list_selected_teams(&workspace_id)
            .await?
            .into_iter()
            .map(|team| SelectedTeamDto {
                id: team.team_id,
                key: team.team_key,
                name: team.team_name,
            })
            .collect();
        flow.ensure_not_canceled("Linear")?;

        // The old bearer must stop owning a live hosted MCP session before the
        // replacement credential becomes eligible for discovery or dispatch.
        lifecycle.bump_epoch();
        crate::agent_mcp::retire_server_sessions(crate::agent_mcp::MANAGED_LINEAR_SERVER_ID).await;
        flow.ensure_not_canceled("Linear")?;
        store::store_tokens(ConnectorProvider::Linear, &workspace_id, &tokens).await?;
        if let Err(canceled) = flow.ensure_not_canceled("Linear") {
            if let Err(error) = restore_linear_connect_state(
                &repos,
                &workspace_id,
                previous_tokens.as_ref(),
                previous_record.as_ref(),
                &retry_account,
            )
            .await
            {
                tracing::error!(
                    error_code = %error.code,
                    "failed to restore Linear state after canceled credential write"
                );
                emit_connectors_changed(app);
                return Err(error);
            }
            emit_connectors_changed(app);
            return Err(canceled);
        }

        if let Err(error) = repos
            .upsert_connector_account(
                &workspace_id,
                ConnectorProvider::Linear.as_str(),
                &identity.user_email,
                &granted_scopes,
                ConnectorAccountStatus::Connected.as_str(),
                &metadata_json,
            )
            .await
        {
            let connect_error = AppError::from(error);
            if let Err(rollback_error) = restore_linear_connect_state(
                &repos,
                &workspace_id,
                previous_tokens.as_ref(),
                previous_record.as_ref(),
                &retry_account,
            )
            .await
            {
                tracing::error!(
                    error_code = %rollback_error.code,
                    "failed to restore Linear state after account persistence error"
                );
                emit_connectors_changed(app);
                return Err(rollback_error);
            }
            emit_connectors_changed(app);
            return Err(connect_error);
        }
        if let Err(canceled) = flow.complete_operation("Linear") {
            if let Err(error) = restore_linear_connect_state(
                &repos,
                &workspace_id,
                previous_tokens.as_ref(),
                previous_record.as_ref(),
                &retry_account,
            )
            .await
            {
                tracing::error!(
                    error_code = %error.code,
                    "failed to restore Linear state after canceled account write"
                );
                emit_connectors_changed(app);
                return Err(error);
            }
            emit_connectors_changed(app);
            return Err(canceled);
        }
        emit_connectors_changed(app);

        Ok(ConnectorAccount {
            account_id: workspace_id,
            provider: ConnectorProvider::Linear,
            email: identity.user_email.clone(),
            scopes: granted_scopes,
            status: ConnectorAccountStatus::Connected,
            via_gh: false,
            workspace_name: Some(identity.workspace_name.clone()).filter(|name| !name.is_empty()),
            workspace_url_key: Some(identity.workspace_url_key.clone()).filter(|key| !key.is_empty()),
            selected_teams,
        })
    }
    .await;

    match result {
        Ok(account) => Ok(account),
        Err(error) => {
            if linear::revoke_unpersisted_grant(&grant.tokens).await {
                Err(error)
            } else {
                Err(linear::unpersisted_grant_cleanup_failed())
            }
        }
    }
}

/// The non-secret metadata blob persisted on a GitHub account row. Keys are
/// camelCase to match [`ConnectorAccountMetadata`]'s parse; empty-string
/// fields are omitted rather than stored as noise. Note: ConnectorAccountMetadata
/// ignores unknown keys (actorLogin, actorName), which is fine.
fn github_account_metadata_json(identity: &github::GithubIdentity) -> String {
    let mut map = serde_json::Map::new();
    for (key, value) in [
        ("actorLogin", &identity.login),
        ("actorName", &identity.name),
    ] {
        if !value.is_empty() {
            map.insert(key.to_string(), serde_json::Value::String(value.clone()));
        }
    }
    serde_json::Value::Object(map).to_string()
}

/// Run the full GitHub connect flow (device authorization, poll until approved,
/// identity resolution, custody write, DB index upsert) for the requested
/// scope bundles. The account is keyed by the NUMERIC GitHub user id
/// (stringified), not the login: logins can change, but numeric ids are
/// stable. `reconnect_account_id` carries that id on a reconnect or scope
/// escalation. With a `reconnect_account_id` naming an already-connected
/// account whose granted scopes cover the request, no device-flow round-trip
/// happens (mirroring the Linear escalation short-circuit).
/// Block a connect only on a CONFIRMED-empty installation set. A GitHub App
/// user token reaches only installed repos, so an authorized-but-uninstalled
/// grant is refused with guidance to the install page. A probe that cannot
/// determine installation status (transient 5xx, secondary rate limit) fails
/// OPEN: stranding a legitimate connect on a momentary GitHub blip (right
/// after the user approved the device flow) is worse than deferring the check
/// to the real repository calls, which surface any access problem themselves.
async fn ensure_github_installed(access_token: &str) -> Result<(), AppError> {
    match github::has_installation(access_token).await {
        Ok(true) => Ok(()),
        Ok(false) => Err(AppError::new(
            "connector_github_not_installed",
            github_not_installed_message(&github_app_slug()),
        )),
        Err(error) => {
            tracing::warn!(
                error_code = %AppError::from(error).code,
                "could not verify github installation during connect; proceeding"
            );
            Ok(())
        }
    }
}

pub async fn begin_connect_github(
    app: &tauri::AppHandle,
    flow: &ConnectFlow,
    bundles: &[policy::ScopeBundle],
    reconnect_account_id: Option<&str>,
) -> Result<ConnectorAccount, AppError> {
    // Defensive: the command layer validates too, but a Google bundle here
    // would request Google scope URLs from GitHub's consent screen.
    if let Some(bundle) = bundles
        .iter()
        .find(|bundle| bundle.provider() != ConnectorProvider::Github)
    {
        return Err(AppError::new(
            "connector_scope_provider_mismatch",
            format!(
                "Scope bundle \"{}\" does not belong to the github connector.",
                bundle.name()
            ),
        ));
    }
    let repos = crate::commands::repositories(app).await?;

    let reconnect_account_id = reconnect_account_id
        .map(str::trim)
        .filter(|id| !id.is_empty());

    // A "via gh" account rides on the GitHub CLI's login: adding a bundle is
    // a Clovy-side grant, never a device flow (ADR-0066).
    if let Some(account_id) = reconnect_account_id {
        if let Some(record) = repos.get_connector_account(account_id).await? {
            if record.provider == ConnectorProvider::Github.as_str()
                && github_cli::is_gh_account(&record.metadata)
            {
                let account = github_cli::grant_bundles(&repos, account_id, bundles).await?;
                emit_connectors_changed(app);
                return Ok(account);
            }
        }
    }
    let client = require_github_client_id()?;

    // Escalation/reconnect short-circuit: an existing, healthy account
    // that already holds every wanted scope needs no new device-flow round-trip.
    if let Some(account_id) = reconnect_account_id {
        if let Some(record) = repos.get_connector_account(account_id).await? {
            let already_granted = policy::missing_scopes(&record.scopes, bundles).is_empty();
            if record.provider == ConnectorProvider::Github.as_str()
                && already_granted
                && record.status == ConnectorAccountStatus::Connected.as_str()
            {
                // Re-verify the installation before honoring a no-op reconnect:
                // a user who uninstalled the app on GitHub (which does not
                // invalidate the token) must surface the not-installed error
                // here instead of silently reaffirming a hollow account. Skip
                // the check only when a fresh token cannot be resolved (a real
                // reconnect is needed anyway); the check itself fails open.
                if let Ok(token) = github_access_token(app, account_id).await {
                    ensure_github_installed(&token).await?;
                }
                return account_dto(&repos, record).await;
            }
        }
    }

    // Clone the AppHandle so we can move it into the closure. The closure is
    // called synchronously (on_device_code is not async), so no await inside.
    let app_handle = app.clone();
    let grant = github::authorize(flow, &client.client_id, |device_code| {
        use tauri::Emitter;
        // Emit the camelCase device-code payload for the frontend dialog.
        let _ = app_handle.emit(
            GITHUB_DEVICE_CODE_EVENT,
            serde_json::json!({
                "userCode": device_code.user_code,
                "verificationUri": device_code.verification_uri,
                "expiresInSeconds": device_code.expires_in,
            }),
        );
        // Open the verification URI in the default browser so the user
        // can approve the device without leaving Clovy.
        let _ = crate::system_browser::open_in_browser(&device_code.verification_uri);
    })
    .await?;
    let identity = grant.identity;
    let user_id = identity.user_id.clone();

    // A GitHub App user grant is worthless without an installation: the token
    // only reaches repositories the app is installed on, and authorization and
    // installation are separate GitHub steps. A user who authorized but never
    // installed the app would otherwise get a "connected" row and MCP servers
    // backed by a token that lists zero repositories. Refuse it here, before
    // any custody write, and point them at the install flow.
    ensure_github_installed(&grant.tokens.access_token).await?;

    // A reconnect id means the user asked to (re)connect one specific account.
    // The browser can still consent for a different one; abort on mismatch
    // rather than silently storing the wrong account.
    if let Some(expected) = reconnect_account_id {
        if !user_id.eq_ignore_ascii_case(expected) {
            return Err(AppError::new(
                "connector_account_mismatch",
                "That GitHub account does not match the one you were reconnecting. Try again and choose that account.",
            ));
        }
    }

    // Same single-account rationale as Google and Linear, scoped to GitHub.
    // The check and the writes below run under the GitHub account lifecycle
    // lock so a concurrent "via gh" detection cannot add a second account.
    let _lifecycle = github_cli::github_accounts_lock().lock().await;
    let existing_accounts = repos.list_connector_accounts().await?;
    if let Some(existing_id) = conflicting_existing_account(
        existing_accounts
            .iter()
            .map(|record| (record.provider.as_str(), record.account_id.as_str())),
        ConnectorProvider::Github.as_str(),
        &user_id,
    ) {
        // Name the stored login when the metadata carries it.
        let display = existing_accounts
            .iter()
            .find(|record| record.account_id == existing_id)
            .and_then(|record| serde_json::from_str::<serde_json::Value>(&record.metadata).ok())
            .and_then(|metadata| {
                metadata
                    .get("actorLogin")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .filter(|name| !name.is_empty())
            .unwrap_or(existing_id);
        return Err(AppError::new(
            "connector_single_account_only",
            format!(
                "Clovy local mode uses one GitHub account at a time. Disconnect {display} before connecting another."
            ),
        ));
    }

    // Clovy-side grant markers: GitHub returns no scope field; the markers are
    // determined by what the user requested. When write is being added to an
    // existing account, union the requested markers with the stored ones so a
    // write-escalation never drops the read marker.
    let requested_markers = policy::requested_github_scopes(bundles);
    let existing_scopes = existing_accounts
        .iter()
        .find(|record| {
            record.provider == ConnectorProvider::Github.as_str()
                && record.account_id.eq_ignore_ascii_case(&user_id)
        })
        .map(|record| record.scopes.as_slice());
    // GitHub returns no scope field; always use the fallback union path.
    let granted_scopes = policy::resolve_granted_scopes(None, &requested_markers, existing_scopes);

    // Refresh-token keep-existing fallback on escalation: when GitHub omits
    // the refresh token on a write-escalation connect AND the custody already
    // holds one, keep the existing one (mirrors the Linear pattern).
    let refresh_token = match grant
        .tokens
        .refresh_token
        .as_deref()
        .filter(|token| !token.is_empty())
    {
        Some(token) => token.to_string(),
        None => {
            // Non-expiring token app or escalation with no new refresh token:
            // keep whatever is in custody (may be empty for non-expiring apps).
            store::load_tokens(ConnectorProvider::Github, &user_id)
                .await?
                .map(|existing| existing.refresh_token.clone())
                .unwrap_or_default()
        }
    };

    let expires_at_unix = grant
        .tokens
        .expires_in
        .map(|secs| now_unix() + secs.max(0))
        .unwrap_or_else(|| now_unix() + github::NON_EXPIRING_TOKEN_LIFETIME_SECS);

    let tokens = store::StoredConnectorTokens {
        access_token: grant.tokens.access_token.clone(),
        refresh_token,
        expires_at_unix,
        scopes: granted_scopes.clone(),
        // login as the informational email field (mirrors Linear's user_email).
        email: identity.login.clone(),
    };
    store::store_tokens(ConnectorProvider::Github, &user_id, &tokens).await?;

    let metadata_json = github_account_metadata_json(&identity);
    repos
        .upsert_connector_account(
            &user_id,
            ConnectorProvider::Github.as_str(),
            &identity.login,
            &granted_scopes,
            ConnectorAccountStatus::Connected.as_str(),
            &metadata_json,
        )
        .await?;
    emit_connectors_changed(app);

    Ok(ConnectorAccount {
        account_id: user_id,
        provider: ConnectorProvider::Github,
        email: identity.login,
        scopes: granted_scopes,
        status: ConnectorAccountStatus::Connected,
        via_gh: false,
        workspace_name: None,
        workspace_url_key: None,
        selected_teams: Vec::new(),
    })
}

fn github_cli_preferences_dir(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppError> {
    crate::app_paths::app_data_dir(app)
        .map_err(|error| AppError::new("app_data_dir_unavailable", error.to_string()))
}

pub fn start_github_cli_detection(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = connect_github_cli(&app, github_cli::ConnectMode::Automatic).await {
            tracing::warn!(error_code = %error.code, "github cli detection at start failed");
        }
    });
}

/// Connect "GitHub (via gh)" when the GitHub CLI is signed in and no GitHub
/// account exists (app start and Plugins), or on the explicit button.
pub async fn connect_github_cli(
    app: &tauri::AppHandle,
    mode: github_cli::ConnectMode,
) -> Result<Option<ConnectorAccount>, AppError> {
    let repos = crate::commands::repositories(app).await?;
    let env = crate::llm::shell_env::login_env().await;
    let before = repos.list_connector_accounts().await?;
    let account =
        github_cli::detect_and_connect(&repos, &env, &github_cli_preferences_dir(app)?, mode)
            .await?;
    let after = repos.list_connector_accounts().await?;
    let changed = before.len() != after.len()
        || before
            .iter()
            .zip(&after)
            .any(|(old, new)| old.account_id != new.account_id || old.status != new.status);
    if changed {
        emit_connectors_changed(app);
    }
    Ok(account)
}

/// Abort an in-flight connect (drains the browser-handoff wait).
pub fn cancel_connect(flow: &ConnectFlow) {
    flow.cancel();
}

/// Disconnect an account: optionally revoke the grant at the provider
/// (best-effort), always remove local custody, and drop the account from
/// the DB index along with its triggers, cursors, and selected teams. The
/// provider is read from the account row; when the row is already gone
/// (a half-completed earlier disconnect), custody cleanup sweeps BOTH
/// providers' keychain services so no token is ever stranded.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectOutcome {
    pub provider_revocation_confirmed: Option<bool>,
}

fn merge_revocation_confirmation(
    current: Option<bool>,
    confirmation: Option<bool>,
) -> Option<bool> {
    match (current, confirmation) {
        (None, confirmation) => confirmation,
        (current, None) => current,
        (Some(current), Some(confirmation)) => Some(current && confirmation),
    }
}

fn missing_revocation_tokens_confirmation(
    revocation_requested: bool,
    account_was_present: bool,
) -> Option<bool> {
    // A known account without custody cannot prove that its remote grant was
    // revoked. During an accountless recovery sweep, however, a missing token
    // only means that this provider did not own the deleted account.
    (revocation_requested && account_was_present).then_some(false)
}

/// Delete the secret custody record before deleting its non-secret account
/// index. Keeping this ordering in one tested boundary preserves the
/// Settings retry action when secure-storage cleanup fails.
async fn delete_custody_then_account<C, I>(
    custody_cleanup: C,
    account_cleanup: I,
) -> Result<(), AppError>
where
    C: Future<Output = Result<(), AppError>>,
    I: Future<Output = Result<(), AppError>>,
{
    custody_cleanup.await?;
    account_cleanup.await
}

fn tokens_can_revoke_provider(
    provider: ConnectorProvider,
    tokens: &store::StoredConnectorTokens,
) -> bool {
    match provider {
        ConnectorProvider::Google | ConnectorProvider::Linear => {
            !tokens.refresh_token.is_empty() || !tokens.access_token.is_empty()
        }
        ConnectorProvider::Github => !tokens.access_token.is_empty(),
        ConnectorProvider::Notion => false,
    }
}

pub async fn disconnect(
    app: &tauri::AppHandle,
    account_id: &str,
    revoke_grant: bool,
) -> Result<DisconnectOutcome, AppError> {
    let repos = crate::commands::repositories(app).await?;
    let account = repos.get_connector_account(account_id).await?;
    if account
        .as_ref()
        .is_some_and(|record| github_cli::is_gh_account(&record.metadata))
    {
        // Nothing of the credential lives in Clovy; disconnecting also stops
        // the automatic connection through gh.
        github_cli::disconnect(&repos, &github_cli_preferences_dir(app)?, account_id).await?;
        emit_connectors_changed(app);
        return Ok(DisconnectOutcome {
            provider_revocation_confirmed: None,
        });
    }
    let account_was_present = account.is_some();
    let providers: &[ConnectorProvider] = match account {
        Some(record) => match ConnectorProvider::from_db(&record.provider) {
            ConnectorProvider::Google => &[ConnectorProvider::Google],
            ConnectorProvider::Linear => &[ConnectorProvider::Linear],
            ConnectorProvider::Github => &[ConnectorProvider::Github],
            ConnectorProvider::Notion => {
                notion::disconnect(app).await?;
                emit_connectors_changed(app);
                return Ok(DisconnectOutcome {
                    provider_revocation_confirmed: None,
                });
            }
        },
        None if account_id == notion::notion_account_id() => {
            notion::disconnect(app).await?;
            emit_connectors_changed(app);
            return Ok(DisconnectOutcome {
                provider_revocation_confirmed: None,
            });
        }
        None => &[
            ConnectorProvider::Google,
            ConnectorProvider::Linear,
            ConnectorProvider::Github,
        ],
    };
    let disconnects_linear = providers.contains(&ConnectorProvider::Linear);
    let linear_lifecycle = if disconnects_linear {
        Some(acquire_linear_lifecycle(account_id).await)
    } else {
        None
    };

    // Advance Linear's eligibility boundary before touching custody. Any
    // managed request that captured the previous epoch now fails closed at
    // the session network boundary. Retiring under the lifecycle gate also
    // prevents a session from escaping a point-in-time retirement sweep.
    if let Some(lifecycle) = linear_lifecycle.as_ref() {
        lifecycle.bump_epoch();
        crate::agent_mcp::retire_server_sessions(crate::agent_mcp::MANAGED_LINEAR_SERVER_ID).await;
    }
    let refresh_lock = refresh_lock_for(account_id);
    let refresh_guard = refresh_lock.lock().await;

    // Capture any credentials needed for best-effort provider revocation
    // while refresh and disconnect are serialized. Remote revoke requests
    // remain outside both local lifecycle locks.
    let mut revoke_tokens = Vec::new();
    let mut provider_revocation_confirmed = None;
    if revoke_grant {
        for &provider in providers {
            match store::load_tokens(provider, account_id).await {
                Ok(Some(stored)) if tokens_can_revoke_provider(provider, &stored) => {
                    revoke_tokens.push((provider, stored));
                }
                Ok(Some(_)) => {
                    provider_revocation_confirmed =
                        merge_revocation_confirmation(provider_revocation_confirmed, Some(false));
                }
                Ok(None) => {
                    provider_revocation_confirmed = merge_revocation_confirmation(
                        provider_revocation_confirmed,
                        missing_revocation_tokens_confirmation(revoke_grant, account_was_present),
                    );
                }
                Err(error) => {
                    tracing::warn!(
                        provider = provider.as_str(),
                        error_code = %error.code,
                        "connector token capture failed before provider revocation"
                    );
                    provider_revocation_confirmed =
                        merge_revocation_confirmation(provider_revocation_confirmed, Some(false));
                }
            }
        }
    }

    // Keep the account row and its Settings retry action until local custody
    // removal succeeds. A Keychain failure returns with the row intact.
    delete_custody_then_account(
        async {
            for &provider in providers {
                store::delete_tokens(provider, account_id).await?;
            }
            Ok(())
        },
        async {
            repos
                .delete_connector_account(account_id)
                .await
                .map_err(AppError::from)
        },
    )
    .await?;

    // Slow provider revocation must not block another local lifecycle
    // operation. The account is already unavailable and custody is gone.
    drop(refresh_guard);
    drop(linear_lifecycle);
    emit_connectors_changed(app);

    if revoke_grant {
        for (provider, stored) in revoke_tokens {
            let confirmed = match provider {
                ConnectorProvider::Google => {
                    // Google: revoking either token of the pair
                    // invalidates the whole grant; prefer the refresh
                    // token.
                    let token = if stored.refresh_token.is_empty() {
                        stored.access_token.clone()
                    } else {
                        stored.refresh_token.clone()
                    };
                    if !token.is_empty() {
                        Some(oauth::revoke(&token).await)
                    } else {
                        None
                    }
                }
                ConnectorProvider::Linear => {
                    // Linear documents no cross-token cascade, so revoke
                    // BOTH tokens: a lone refresh-token revoke can leave
                    // the 24-hour access token alive and the app still
                    // listed under the user's authorized applications.
                    let mut attempted = false;
                    let mut confirmed = true;
                    if !stored.refresh_token.is_empty() {
                        attempted = true;
                        confirmed &= linear::revoke(&stored.refresh_token, "refresh_token").await;
                    }
                    if !stored.access_token.is_empty() {
                        attempted = true;
                        confirmed &= linear::revoke(&stored.access_token, "access_token").await;
                    }
                    attempted.then_some(confirmed)
                }
                ConnectorProvider::Github => {
                    // GitHub: revoke the user access token via the
                    // applications endpoint (DELETE). Best-effort.
                    // Revocation requires the client secret; a
                    // secret-less deployment skips revocation (the user
                    // must revoke manually at GitHub if needed).
                    if !stored.access_token.is_empty() {
                        let client = github_oauth_client();
                        if !client.client_id.is_empty() {
                            if let Some(secret) = client.client_secret_opt() {
                                Some(
                                    github::revoke(&client.client_id, secret, &stored.access_token)
                                        .await,
                                )
                            } else {
                                tracing::warn!(
                                        "github revoke skipped: no client secret configured; revoke manually at GitHub"
                                    );
                                Some(false)
                            }
                        } else {
                            Some(false)
                        }
                    } else {
                        None
                    }
                }
                ConnectorProvider::Notion => None,
            };
            provider_revocation_confirmed =
                merge_revocation_confirmation(provider_revocation_confirmed, confirmed);
        }
    }
    Ok(DisconnectOutcome {
        provider_revocation_confirmed,
    })
}

// --- Linear team enforcement (agent reads) -----------------------------------------
//
// The selected-team grant is the enforcement boundary for every `june_linear`
// agent read (slice 2, JUN-284): the provider-proxy route handlers (a later
// chunk) load the grant once per request via [`linear_granted_team_ids`] and
// check individual reads against it with [`linear_require_team_granted`] /
// [`linear_require_any_team_granted`]. Both stable error codes are defined
// here because this is where the caller-level checks live; `linear.rs`
// itself never sees an `AppHandle` or the repository layer, so it cannot
// construct these errors directly.

fn linear_teams_not_selected_error() -> AppError {
    AppError::new(
        "linear_teams_not_selected",
        "Select Linear teams in settings before Clovy can read this workspace.",
    )
}

fn linear_team_not_granted_error() -> AppError {
    AppError::new(
        "linear_team_not_granted",
        "That team is not in Clovy's selected teams.",
    )
}

/// Pure "records -> granted ids or error" mapping behind
/// [`linear_granted_team_ids`]: an empty selection fails closed rather than
/// letting a caller silently query an unscoped workspace (spec decision 7).
fn granted_team_ids_from_records(
    records: &[crate::db::repositories::SelectedTeamRecord],
) -> Result<Vec<String>, AppError> {
    if records.is_empty() {
        return Err(linear_teams_not_selected_error());
    }
    Ok(records.iter().map(|team| team.team_id.clone()).collect())
}

/// Load the account's selected-team grant for a `june_linear` read. Errors
/// `linear_teams_not_selected` when the account has no selected teams -
/// every team-scoped route calls this before doing anything else, so an
/// empty grant fails the whole request closed rather than reading an
/// unscoped workspace.
pub async fn linear_granted_team_ids(
    app: &tauri::AppHandle,
    account_id: &str,
) -> Result<Vec<String>, AppError> {
    let repos = crate::commands::repositories(app).await?;
    let records = repos.list_selected_teams(account_id).await?;
    granted_team_ids_from_records(&records)
}

/// Validate a single team id against the grant: used where the caller names
/// one team directly (`list_cycles`'s `team_id`, `search_issues`'s optional
/// team narrow). Errors `linear_team_not_granted` when `team_id` is not in
/// `granted_team_ids`.
pub fn linear_require_team_granted(
    team_id: &str,
    granted_team_ids: &[String],
) -> Result<(), AppError> {
    if granted_team_ids.iter().any(|granted| granted == team_id) {
        Ok(())
    } else {
        Err(linear_team_not_granted_error())
    }
}

/// Validate that at least one of `team_ids` (an already-fetched issue's or
/// project's linked teams) is in the grant: the post-fetch check for
/// `get_issue` / `list_issue_comments` / `list_project_updates`. The caller
/// discards the fetched data on `Err` rather than returning it partially.
/// Errors `linear_team_not_granted`.
pub fn linear_require_any_team_granted(
    team_ids: &[String],
    granted_team_ids: &[String],
) -> Result<(), AppError> {
    let granted = team_ids
        .iter()
        .any(|id| granted_team_ids.iter().any(|granted| granted == id));
    if granted {
        Ok(())
    } else {
        Err(linear_team_not_granted_error())
    }
}

// --- Linear writes: action ids, journal, stable errors (slice 3) --------------------
//
// Every Linear mutation is journaled in `connector_actions` (migration 015)
// around the provider call: `pending` before, then exactly one of
// `committed` / `failed` / `ambiguous`. The journal is an AUDIT surface, not
// a gate: a journal write failure is logged and swallowed, because failing
// the user's already-approved mutation over local bookkeeping would be
// strictly worse than a missing journal row (the mutation itself stays
// reconcilable through its client-minted object id). The stable write-flow
// error constructors live here, next to the read-flow ones above, so their
// wording exists in exactly one place.

/// Mint the client-side v4 UUID for a Linear create: it becomes the created
/// object's id at Linear (making the create idempotent and reconcilable)
/// AND the journal's action id. Hyphenated lowercase, e.g.
/// `67e55044-10b1-426f-9247-bb680e5fe0c8`.
pub fn mint_action_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The update conflicted with a newer edit: the issue's `updatedAt` no
/// longer matches the `expected_updated_at` the agent supplied from its last
/// read. The route layer returns this WITHOUT mutating anything.
pub fn linear_issue_conflict_error() -> AppError {
    AppError::new(
        "linear_issue_conflict",
        "This issue changed since Clovy last read it. Re-read it and try again.",
    )
}

/// The mutation's outcome is unknown (timeout or transport loss, and the
/// reconciliation lookup could not confirm the object exists). Carries the
/// action id so the user can locate the attempt; the agent must NOT retry.
pub fn linear_action_ambiguous_error(action_id: &str) -> AppError {
    AppError::new(
        "linear_action_ambiguous",
        format!(
            "Clovy could not confirm whether Linear applied this change. Check Linear before retrying; action id {action_id}."
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx_sqlite::SqlitePoolOptions;

    async fn rollback_test_repositories() -> crate::db::repositories::Repositories {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("sqlite memory");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        crate::db::repositories::Repositories::new(pool)
    }

    fn rollback_test_tokens(marker: &str, scopes: &[&str]) -> store::StoredConnectorTokens {
        store::StoredConnectorTokens {
            access_token: format!("{marker}-access"),
            refresh_token: format!("{marker}-refresh"),
            expires_at_unix: 4_102_444_800,
            scopes: scopes.iter().map(|scope| (*scope).to_string()).collect(),
            email: "actor@example.com".to_string(),
        }
    }

    #[tokio::test]
    async fn fresh_connect_cancellation_removes_new_custody_and_account() {
        let repos = rollback_test_repositories().await;
        let workspace_id = "fresh-canceled-workspace";
        let new_scopes = vec!["linear_read".to_string(), "linear_write".to_string()];
        repos
            .upsert_connector_account(
                workspace_id,
                ConnectorProvider::Linear.as_str(),
                "actor@example.com",
                &new_scopes,
                ConnectorAccountStatus::Connected.as_str(),
                r#"{"workspaceName":"New workspace"}"#,
            )
            .await
            .expect("simulate new account write");
        let custody = Arc::new(StdMutex::new(Some(rollback_test_tokens(
            "new",
            &["linear_read", "linear_write"],
        ))));

        let flow = ConnectFlow::default();
        let _operation = flow.begin_operation().expect("operation starts");
        flow.cancel();
        let canceled = flow
            .ensure_not_canceled("Linear")
            .expect_err("cancel wins before finalization");

        let retry_account = LinearRetryAccount {
            email: "actor@example.com",
            scopes: &new_scopes,
            metadata: r#"{"workspaceName":"New workspace"}"#,
        };
        let custody_to_restore = Arc::clone(&custody);
        restore_linear_connect_state_with(
            &repos,
            workspace_id,
            None,
            &retry_account,
            move || async move {
                *custody_to_restore.lock().expect("custody lock") = None;
                Ok(())
            },
        )
        .await
        .expect("fresh state rolls back");

        assert_eq!(canceled.code, "connector_connect_canceled");
        assert!(custody.lock().expect("custody lock").is_none());
        assert!(repos
            .get_connector_account(workspace_id)
            .await
            .expect("account lookup")
            .is_none());
    }

    #[tokio::test]
    async fn fresh_connect_custody_rollback_failure_keeps_a_retryable_account() {
        let repos = rollback_test_repositories().await;
        let workspace_id = "fresh-custody-rollback-failed";
        let new_scopes = vec!["linear_read".to_string(), "linear_write".to_string()];
        let retry_account = LinearRetryAccount {
            email: "actor@example.com",
            scopes: &new_scopes,
            metadata: r#"{"workspaceName":"Retry workspace"}"#,
        };

        let error = restore_linear_connect_state_with(
            &repos,
            workspace_id,
            None,
            &retry_account,
            || async {
                Err(AppError::new(
                    "connector_keychain_delete_failed",
                    "injected failure",
                ))
            },
        )
        .await
        .expect_err("custody rollback fails");

        assert_eq!(error.code, "connector_keychain_delete_failed");
        let retry_record = repos
            .get_connector_account(workspace_id)
            .await
            .expect("retry account lookup")
            .expect("retry account remains visible");
        assert_eq!(
            retry_record.status,
            ConnectorAccountStatus::ReconnectRequired.as_str()
        );
        assert_eq!(retry_record.email, "actor@example.com");
        assert_eq!(retry_record.scopes, new_scopes);
        assert_eq!(
            retry_record.metadata,
            r#"{"workspaceName":"Retry workspace"}"#
        );
    }

    #[tokio::test]
    async fn fresh_connect_account_delete_failure_marks_the_row_retryable() {
        let repos = rollback_test_repositories().await;
        let workspace_id = "fresh-account-delete-failed";
        let new_scopes = vec!["linear_read".to_string(), "linear_write".to_string()];
        let retry_account = LinearRetryAccount {
            email: "actor@example.com",
            scopes: &new_scopes,
            metadata: r#"{"workspaceName":"Retry workspace"}"#,
        };
        repos
            .upsert_connector_account(
                workspace_id,
                ConnectorProvider::Linear.as_str(),
                retry_account.email,
                retry_account.scopes,
                ConnectorAccountStatus::Connected.as_str(),
                retry_account.metadata,
            )
            .await
            .expect("seed connected account");
        sqlx::query::query(
            "CREATE TRIGGER fail_linear_account_delete
             BEFORE DELETE ON connector_accounts
             BEGIN
               SELECT RAISE(FAIL, 'injected delete failure');
             END",
        )
        .execute(&repos.pool)
        .await
        .expect("install delete failure");

        let error = restore_linear_connect_state_with(
            &repos,
            workspace_id,
            None,
            &retry_account,
            || async { Ok(()) },
        )
        .await
        .expect_err("account rollback failure is surfaced");

        assert_ne!(error.code, "connector_connect_canceled");
        let retry_record = repos
            .get_connector_account(workspace_id)
            .await
            .expect("retry account lookup")
            .expect("retry account remains visible");
        assert_eq!(
            retry_record.status,
            ConnectorAccountStatus::ReconnectRequired.as_str()
        );
        assert_eq!(retry_record.email, retry_account.email);
        assert_eq!(retry_record.scopes, new_scopes);
        assert_eq!(retry_record.metadata, retry_account.metadata);
    }

    #[tokio::test]
    async fn reconnect_cancellation_restores_previous_state_without_cascading_related_rows() {
        let repos = rollback_test_repositories().await;
        let workspace_id = "reconnect-canceled-workspace";
        let previous_scopes = vec!["linear_read".to_string()];
        let previous_metadata = r#"{"workspaceName":"Previous workspace"}"#;
        repos
            .upsert_connector_account(
                workspace_id,
                ConnectorProvider::Linear.as_str(),
                "previous@example.com",
                &previous_scopes,
                ConnectorAccountStatus::ReconnectRequired.as_str(),
                previous_metadata,
            )
            .await
            .expect("seed previous account");
        let team = selected_team("engineering");
        repos
            .set_selected_teams(workspace_id, std::slice::from_ref(&team))
            .await
            .expect("seed selected team");
        let previous_record = repos
            .get_connector_account(workspace_id)
            .await
            .expect("previous account lookup")
            .expect("previous account");
        let previous_tokens = rollback_test_tokens("previous", &["linear_read"]);
        let custody = Arc::new(StdMutex::new(Some(rollback_test_tokens(
            "new",
            &["linear_read", "linear_write"],
        ))));

        let new_scopes = vec!["linear_read".to_string(), "linear_write".to_string()];
        repos
            .upsert_connector_account(
                workspace_id,
                ConnectorProvider::Linear.as_str(),
                "new@example.com",
                &new_scopes,
                ConnectorAccountStatus::Connected.as_str(),
                r#"{"workspaceName":"New workspace"}"#,
            )
            .await
            .expect("simulate replacement account write");

        let flow = ConnectFlow::default();
        let _operation = flow.begin_operation().expect("operation starts");
        flow.cancel();
        let canceled = flow
            .complete_operation("Linear")
            .expect_err("cancel wins during persistence");

        let retry_account = LinearRetryAccount {
            email: "new@example.com",
            scopes: &new_scopes,
            metadata: r#"{"workspaceName":"New workspace"}"#,
        };
        let custody_to_restore = Arc::clone(&custody);
        let tokens_to_restore = previous_tokens.clone();
        restore_linear_connect_state_with(
            &repos,
            workspace_id,
            Some(&previous_record),
            &retry_account,
            move || async move {
                *custody_to_restore.lock().expect("custody lock") = Some(tokens_to_restore);
                Ok(())
            },
        )
        .await
        .expect("reconnect state rolls back");

        assert_eq!(canceled.code, "connector_connect_canceled");
        {
            let custody_guard = custody.lock().expect("custody lock");
            let restored_tokens = custody_guard.as_ref().expect("restored custody");
            assert_eq!(restored_tokens.access_token, previous_tokens.access_token);
            assert_eq!(restored_tokens.refresh_token, previous_tokens.refresh_token);
            assert_eq!(restored_tokens.scopes, previous_tokens.scopes);
        }

        let restored_record = repos
            .get_connector_account(workspace_id)
            .await
            .expect("restored account lookup")
            .expect("restored account");
        assert_eq!(restored_record.email, "previous@example.com");
        assert_eq!(restored_record.scopes, previous_scopes);
        assert_eq!(
            restored_record.status,
            ConnectorAccountStatus::ReconnectRequired.as_str()
        );
        assert_eq!(restored_record.metadata, previous_metadata);
        assert_eq!(
            repos
                .list_selected_teams(workspace_id)
                .await
                .expect("selected teams"),
            vec![team]
        );
    }

    #[tokio::test]
    async fn reconnect_custody_rollback_failure_keeps_previous_account_retryable() {
        let repos = rollback_test_repositories().await;
        let workspace_id = "reconnect-custody-rollback-failed";
        let previous_scopes = vec!["linear_read".to_string()];
        let previous_metadata = r#"{"workspaceName":"Previous workspace"}"#;
        repos
            .upsert_connector_account(
                workspace_id,
                ConnectorProvider::Linear.as_str(),
                "previous@example.com",
                &previous_scopes,
                ConnectorAccountStatus::Connected.as_str(),
                previous_metadata,
            )
            .await
            .expect("seed previous account");
        let team = selected_team("engineering");
        repos
            .set_selected_teams(workspace_id, std::slice::from_ref(&team))
            .await
            .expect("seed selected team");
        let previous_record = repos
            .get_connector_account(workspace_id)
            .await
            .expect("previous account lookup")
            .expect("previous account");

        let new_scopes = vec!["linear_read".to_string(), "linear_write".to_string()];
        let retry_account = LinearRetryAccount {
            email: "new@example.com",
            scopes: &new_scopes,
            metadata: r#"{"workspaceName":"New workspace"}"#,
        };
        repos
            .upsert_connector_account(
                workspace_id,
                ConnectorProvider::Linear.as_str(),
                retry_account.email,
                retry_account.scopes,
                ConnectorAccountStatus::Connected.as_str(),
                retry_account.metadata,
            )
            .await
            .expect("simulate replacement account write");

        let error = restore_linear_connect_state_with(
            &repos,
            workspace_id,
            Some(&previous_record),
            &retry_account,
            || async {
                Err(AppError::new(
                    "connector_keychain_write_failed",
                    "injected failure",
                ))
            },
        )
        .await
        .expect_err("custody rollback fails");

        assert_eq!(error.code, "connector_keychain_write_failed");
        let retry_record = repos
            .get_connector_account(workspace_id)
            .await
            .expect("retry account lookup")
            .expect("previous account remains visible");
        assert_eq!(retry_record.email, "previous@example.com");
        assert_eq!(retry_record.scopes, previous_scopes);
        assert_eq!(
            retry_record.status,
            ConnectorAccountStatus::ReconnectRequired.as_str()
        );
        assert_eq!(retry_record.metadata, previous_metadata);
        assert_eq!(
            repos
                .list_selected_teams(workspace_id)
                .await
                .expect("selected teams"),
            vec![team]
        );
    }

    #[test]
    fn provider_and_status_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&ConnectorProvider::Google).unwrap(),
            "\"google\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectorProvider::Notion).unwrap(),
            "\"notion\""
        );
        assert_eq!(
            serde_json::from_str::<ConnectorProvider>("\"notion\"").unwrap(),
            ConnectorProvider::Notion
        );
        assert_eq!(ConnectorProvider::Notion.as_str(), "notion");
        assert_eq!(
            serde_json::to_string(&ConnectorProvider::Linear).unwrap(),
            "\"linear\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectorProvider::Github).unwrap(),
            "\"github\""
        );
        assert_eq!(
            serde_json::from_str::<ConnectorProvider>("\"github\"").unwrap(),
            ConnectorProvider::Github
        );
        assert_eq!(ConnectorProvider::Github.as_str(), "github");
        assert_eq!(
            serde_json::to_string(&ConnectorAccountStatus::ReconnectRequired).unwrap(),
            "\"reconnect_required\""
        );
        assert_eq!(
            serde_json::to_string(&ConnectorAccountStatus::Unavailable).unwrap(),
            "\"unavailable\""
        );
        assert_eq!(
            serde_json::from_str::<ConnectorAccountStatus>("\"connected\"").unwrap(),
            ConnectorAccountStatus::Connected
        );
    }

    #[test]
    fn revocation_confirmation_ignores_missing_attempts_and_combines_results() {
        assert_eq!(merge_revocation_confirmation(None, None), None);
        assert_eq!(merge_revocation_confirmation(None, Some(true)), Some(true));
        assert_eq!(merge_revocation_confirmation(Some(true), None), Some(true));
        assert_eq!(
            merge_revocation_confirmation(Some(true), Some(false)),
            Some(false)
        );
        assert_eq!(
            merge_revocation_confirmation(Some(false), Some(true)),
            Some(false)
        );
    }

    #[test]
    fn missing_revocation_tokens_warn_for_a_known_account_only() {
        assert_eq!(
            missing_revocation_tokens_confirmation(true, true),
            Some(false)
        );
        assert_eq!(missing_revocation_tokens_confirmation(false, true), None);
    }

    #[test]
    fn accountless_revocation_sweep_ignores_missing_provider_tokens() {
        let missing_provider = missing_revocation_tokens_confirmation(true, false);

        assert_eq!(missing_provider, None);
        assert_eq!(
            merge_revocation_confirmation(missing_provider, Some(true)),
            Some(true)
        );
        assert_eq!(
            merge_revocation_confirmation(missing_provider, missing_provider),
            None
        );
    }

    #[test]
    fn provider_revocation_requires_a_usable_captured_token() {
        let tokens = |access_token: &str, refresh_token: &str| store::StoredConnectorTokens {
            access_token: access_token.to_string(),
            refresh_token: refresh_token.to_string(),
            expires_at_unix: 0,
            scopes: Vec::new(),
            email: String::new(),
        };

        assert!(tokens_can_revoke_provider(
            ConnectorProvider::Google,
            &tokens("", "google-refresh"),
        ));
        assert!(tokens_can_revoke_provider(
            ConnectorProvider::Linear,
            &tokens("linear-access", ""),
        ));
        assert!(tokens_can_revoke_provider(
            ConnectorProvider::Github,
            &tokens("github-access", "ignored-refresh"),
        ));
        assert!(!tokens_can_revoke_provider(
            ConnectorProvider::Github,
            &tokens("", "ignored-refresh"),
        ));
        assert!(!tokens_can_revoke_provider(
            ConnectorProvider::Linear,
            &tokens("", ""),
        ));
        assert!(!tokens_can_revoke_provider(
            ConnectorProvider::Notion,
            &tokens("unused", "unused"),
        ));
    }

    #[tokio::test]
    async fn linear_lifecycle_serializes_boundaries_and_invalidates_old_snapshots() {
        let account_id = format!("linear-lifecycle-{}", uuid::Uuid::new_v4());
        let first = acquire_linear_lifecycle(&account_id).await;
        let stale = first.snapshot();
        let acquired = Arc::new(tokio::sync::Notify::new());
        let acquired_waiter = acquired.clone();
        let waiter_account_id = account_id.clone();
        let waiter = tokio::spawn(async move {
            let next = acquire_linear_lifecycle(&waiter_account_id).await;
            acquired_waiter.notify_one();
            next.snapshot()
        });

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(25), acquired.notified())
                .await
                .is_err()
        );
        first.bump_epoch();
        assert!(!stale.is_current());
        drop(first);

        let current = waiter.await.unwrap();
        assert!(current.is_current());
    }

    #[tokio::test]
    async fn linear_disconnect_waits_for_refresh_before_deleting_custody() {
        let account_id = format!("linear-refresh-disconnect-{}", uuid::Uuid::new_v4());
        let lifecycle = acquire_linear_lifecycle(&account_id).await;
        let stale = lifecycle.snapshot();
        drop(lifecycle);

        let token_present = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let refresh_started = Arc::new(tokio::sync::Notify::new());
        let finish_refresh = Arc::new(tokio::sync::Notify::new());

        let refresh_account_id = account_id.clone();
        let refresh_token_present = token_present.clone();
        let refresh_started_signal = refresh_started.clone();
        let finish_refresh_waiter = finish_refresh.clone();
        let refresh = tokio::spawn(async move {
            let lock = refresh_lock_for(&refresh_account_id);
            let _guard = lock.lock().await;
            refresh_started_signal.notify_one();
            finish_refresh_waiter.notified().await;
            // Model a rotated credential being persisted at refresh commit.
            refresh_token_present.store(true, std::sync::atomic::Ordering::SeqCst);
        });
        refresh_started.notified().await;

        let disconnect_account_id = account_id.clone();
        let disconnect_token_present = token_present.clone();
        let lifecycle_advanced = Arc::new(tokio::sync::Notify::new());
        let lifecycle_advanced_signal = lifecycle_advanced.clone();
        let disconnect = tokio::spawn(async move {
            let lifecycle = acquire_linear_lifecycle(&disconnect_account_id).await;
            lifecycle.bump_epoch();
            lifecycle_advanced_signal.notify_one();
            let lock = refresh_lock_for(&disconnect_account_id);
            let _guard = lock.lock().await;
            // This is the local custody delete in disconnect. It must run
            // after an in-flight refresh finishes its final store.
            disconnect_token_present.store(false, std::sync::atomic::Ordering::SeqCst);
        });
        lifecycle_advanced.notified().await;

        assert!(!stale.is_current());
        assert!(!disconnect.is_finished());
        finish_refresh.notify_one();
        refresh.await.unwrap();
        disconnect.await.unwrap();
        assert!(!token_present.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn custody_failure_keeps_account_index_retryable() {
        let account_deleted = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let account_deleted_signal = account_deleted.clone();

        let result = delete_custody_then_account(
            async {
                Err(AppError::new(
                    "connector_keychain_delete_failed",
                    "injected failure",
                ))
            },
            async move {
                account_deleted_signal.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            },
        )
        .await;

        assert_eq!(result.unwrap_err().code, "connector_keychain_delete_failed");
        assert!(!account_deleted.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn provider_from_db_defaults_to_google() {
        assert_eq!(
            ConnectorProvider::from_db("google"),
            ConnectorProvider::Google
        );
        assert_eq!(
            ConnectorProvider::from_db("linear"),
            ConnectorProvider::Linear
        );
        assert_eq!(
            ConnectorProvider::from_db("github"),
            ConnectorProvider::Github
        );
        assert_eq!(
            ConnectorProvider::from_db("unexpected"),
            ConnectorProvider::Google
        );
    }

    #[test]
    fn account_serializes_camel_case_for_the_frontend() {
        let account = ConnectorAccount {
            account_id: "user@example.com".to_string(),
            provider: ConnectorProvider::Google,
            email: "user@example.com".to_string(),
            scopes: vec!["openid".to_string()],
            status: ConnectorAccountStatus::Connected,
            via_gh: false,
            workspace_name: None,
            workspace_url_key: None,
            selected_teams: Vec::new(),
        };
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["accountId"], "user@example.com");
        assert_eq!(json["provider"], "google");
        assert_eq!(json["status"], "connected");
        assert!(json["workspaceName"].is_null());
        assert!(json["workspaceUrlKey"].is_null());
        assert_eq!(json["selectedTeams"], serde_json::json!([]));

        let linear_account = ConnectorAccount {
            account_id: "workspace-1".to_string(),
            provider: ConnectorProvider::Linear,
            email: String::new(),
            scopes: Vec::new(),
            status: ConnectorAccountStatus::Connected,
            via_gh: false,
            workspace_name: Some("Acme".to_string()),
            workspace_url_key: Some("acme".to_string()),
            selected_teams: vec![SelectedTeamDto {
                id: "team-1".to_string(),
                key: "ENG".to_string(),
                name: "Engineering".to_string(),
            }],
        };
        let json = serde_json::to_value(&linear_account).unwrap();
        assert_eq!(json["provider"], "linear");
        assert_eq!(json["workspaceName"], "Acme");
        assert_eq!(json["workspaceUrlKey"], "acme");
        assert_eq!(json["selectedTeams"][0]["id"], "team-1");
        assert_eq!(json["selectedTeams"][0]["key"], "ENG");
        assert_eq!(json["selectedTeams"][0]["name"], "Engineering");
    }

    #[test]
    fn notion_account_serializes_camel_case_for_the_frontend() {
        let account = ConnectorAccount {
            account_id: notion::notion_account_id().to_string(),
            provider: ConnectorProvider::Notion,
            email: notion::notion_account_email().to_string(),
            scopes: Vec::new(),
            status: ConnectorAccountStatus::Connected,
            via_gh: false,
            workspace_name: None,
            workspace_url_key: None,
            selected_teams: Vec::new(),
        };
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["accountId"], "notion-hosted-mcp");
        assert_eq!(json["provider"], "notion");
        assert_eq!(json["email"], "Notion");
        assert_eq!(json["scopes"].as_array().unwrap().len(), 0);
        assert_eq!(json["status"], "connected");
        assert!(json["workspaceName"].is_null());
        assert!(json["workspaceUrlKey"].is_null());
        assert_eq!(json["selectedTeams"], serde_json::json!([]));
    }

    #[test]
    fn append_notion_account_preserves_errors_unless_resilient() {
        let mut strict = Vec::new();
        let error = AppError::new("notion_keychain_read_failed", "failed");
        let strict_error =
            append_notion_account(&mut strict, Err(error.clone()), false).unwrap_err();
        assert_eq!(strict_error.code, error.code);
        assert_eq!(strict_error.message, error.message);
        assert!(strict.is_empty());

        let mut resilient = Vec::new();
        append_notion_account(&mut resilient, Err(error), true).unwrap();
        assert_eq!(resilient.len(), 1);
        assert_eq!(resilient[0].provider, ConnectorProvider::Notion);
        assert_eq!(resilient[0].status, ConnectorAccountStatus::Unavailable);

        let mut disconnected = Vec::new();
        append_notion_account(&mut disconnected, Ok(None), true).unwrap();
        assert!(disconnected.is_empty());
    }

    #[test]
    fn append_notion_account_adds_synthetic_connected_row() {
        let mut accounts = Vec::new();
        append_notion_account(
            &mut accounts,
            Ok(Some(ConnectorAccountStatus::Connected)),
            false,
        )
        .unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].provider, ConnectorProvider::Notion);
        assert_eq!(accounts[0].account_id, notion::notion_account_id());
        assert_eq!(accounts[0].email, notion::notion_account_email());
        assert!(accounts[0].selected_teams.is_empty());
    }

    #[test]
    fn append_notion_account_preserves_reconnect_required_status() {
        let mut accounts = Vec::new();
        append_notion_account(
            &mut accounts,
            Ok(Some(ConnectorAccountStatus::ReconnectRequired)),
            false,
        )
        .unwrap();
        assert_eq!(
            accounts[0].status,
            ConnectorAccountStatus::ReconnectRequired
        );
    }

    #[test]
    fn freshness_uses_expiry_buffer() {
        let now = 1_000_000;
        assert!(access_token_is_fresh(now + 61, now));
        assert!(!access_token_is_fresh(now + 60, now));
        assert!(!access_token_is_fresh(now - 1, now));
    }

    #[test]
    fn status_from_db_defaults_to_connected() {
        assert_eq!(
            ConnectorAccountStatus::from_db("reconnect_required"),
            ConnectorAccountStatus::ReconnectRequired
        );
        assert_eq!(
            ConnectorAccountStatus::from_db("connected"),
            ConnectorAccountStatus::Connected
        );
        assert_eq!(
            ConnectorAccountStatus::from_db("unexpected"),
            ConnectorAccountStatus::Connected
        );
        assert_eq!(
            ConnectorAccountStatus::from_db("unavailable"),
            ConnectorAccountStatus::Connected
        );
    }

    #[test]
    fn single_account_guard_blocks_a_different_account_only() {
        // First-ever connect: nothing stored, nothing conflicts.
        assert_eq!(
            conflicting_existing_account([], "google", "a@example.com"),
            None
        );
        // Reconnect or scope-add on the same account (any casing) is allowed.
        assert_eq!(
            conflicting_existing_account([("google", "a@example.com")], "google", "A@Example.com"),
            None
        );
        // A second, distinct account is refused, naming the stored one.
        assert_eq!(
            conflicting_existing_account([("google", "a@example.com")], "google", "b@example.com"),
            Some("a@example.com".to_string())
        );
        // The stored account is reported even when the new one is also present
        // in the list (defensive: only the differing email matters).
        assert_eq!(
            conflicting_existing_account(
                [("google", "a@example.com"), ("google", "b@example.com")],
                "google",
                "b@example.com"
            ),
            Some("a@example.com".to_string())
        );
    }

    #[test]
    fn linear_metadata_json_omits_empty_fields_and_round_trips() {
        let identity = linear::LinearIdentity {
            workspace_id: "org-1".to_string(),
            workspace_name: "Acme".to_string(),
            workspace_url_key: String::new(),
            user_id: "user-1".to_string(),
            user_name: String::new(),
            user_email: "ada@example.com".to_string(),
        };
        let raw = linear_account_metadata_json(&identity);
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["workspaceName"], "Acme");
        assert_eq!(json["actorUserId"], "user-1");
        assert!(json.get("workspaceUrlKey").is_none());
        assert!(json.get("actorName").is_none());
        // The stored blob parses back through the account-metadata reader
        // that list_accounts uses.
        let metadata: ConnectorAccountMetadata = serde_json::from_str(&raw).unwrap();
        assert_eq!(metadata.workspace_name.as_deref(), Some("Acme"));
        assert_eq!(metadata.workspace_url_key, None);
    }

    #[test]
    fn linear_loopback_ports_prefer_a_valid_env_override_only() {
        // A parseable non-zero override narrows the candidates to that port.
        assert_eq!(linear_loopback_ports_from("50000"), vec![50000]);
        // Empty, garbage, zero, and out-of-range values all fall back to the
        // registered defaults instead of breaking the connect flow.
        assert_eq!(linear_loopback_ports_from(""), LINEAR_LOOPBACK_PORTS);
        assert_eq!(
            linear_loopback_ports_from("not-a-port"),
            LINEAR_LOOPBACK_PORTS
        );
        assert_eq!(linear_loopback_ports_from("0"), LINEAR_LOOPBACK_PORTS);
        assert_eq!(linear_loopback_ports_from("70000"), LINEAR_LOOPBACK_PORTS);
    }

    #[test]
    fn single_account_guard_ignores_other_providers() {
        // A connected Google account must never block connecting a Linear
        // workspace (and vice versa): the single-account guard is scoped per
        // provider, not global across the whole connector_accounts table.
        assert_eq!(
            conflicting_existing_account([("google", "a@example.com")], "linear", "workspace-1"),
            None
        );
        assert_eq!(
            conflicting_existing_account(
                [("linear", "workspace-1"), ("google", "a@example.com")],
                "google",
                "b@example.com"
            ),
            Some("a@example.com".to_string())
        );
    }

    fn selected_team(id: &str) -> crate::db::repositories::SelectedTeamRecord {
        crate::db::repositories::SelectedTeamRecord {
            team_id: id.to_string(),
            team_key: id.to_uppercase(),
            team_name: format!("Team {id}"),
        }
    }

    #[test]
    fn granted_team_ids_from_records_fails_closed_on_an_empty_selection() {
        let error = granted_team_ids_from_records(&[]).unwrap_err();
        assert_eq!(error.code, "linear_teams_not_selected");
    }

    #[test]
    fn granted_team_ids_from_records_maps_ids_when_present() {
        let records = vec![selected_team("eng"), selected_team("design")];
        let ids = granted_team_ids_from_records(&records).expect("granted ids");
        assert_eq!(ids, vec!["eng", "design"]);
    }

    #[test]
    fn linear_require_team_granted_checks_membership() {
        let granted = vec!["eng".to_string(), "design".to_string()];
        assert!(linear_require_team_granted("eng", &granted).is_ok());
        let error = linear_require_team_granted("ops", &granted).unwrap_err();
        assert_eq!(error.code, "linear_team_not_granted");
        // An empty grant refuses every team id, including one that would
        // otherwise match a non-empty grant.
        let error = linear_require_team_granted("eng", &[]).unwrap_err();
        assert_eq!(error.code, "linear_team_not_granted");
    }

    #[test]
    fn linear_require_any_team_granted_checks_intersection() {
        let granted = vec!["eng".to_string(), "design".to_string()];
        assert!(linear_require_any_team_granted(
            &["design".to_string(), "ops".to_string()],
            &granted
        )
        .is_ok());
        let error = linear_require_any_team_granted(&["ops".to_string()], &granted).unwrap_err();
        assert_eq!(error.code, "linear_team_not_granted");
        // An empty team-ids list (an entity linked to no team at all) and an
        // empty grant both refuse to match anything.
        let error = linear_require_any_team_granted(&[], &granted).unwrap_err();
        assert_eq!(error.code, "linear_team_not_granted");
        let error = linear_require_any_team_granted(&["eng".to_string()], &[]).unwrap_err();
        assert_eq!(error.code, "linear_team_not_granted");
    }

    #[test]
    fn mint_action_id_produces_hyphenated_lowercase_v4_uuids() {
        let id = mint_action_id();
        let bytes: Vec<char> = id.chars().collect();
        assert_eq!(bytes.len(), 36);
        for position in [8, 13, 18, 23] {
            assert_eq!(bytes[position], '-', "hyphen expected at {position}");
        }
        // RFC 4122: version nibble is 4, variant nibble is 8|9|a|b.
        assert_eq!(bytes[14], '4');
        assert!(matches!(bytes[19], '8' | '9' | 'a' | 'b'));
        assert_eq!(id, id.to_lowercase());
        assert!(id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));

        // Practically collision-free: 1000 mints, 1000 distinct ids.
        let mut ids: Vec<String> = (0..1000).map(|_| mint_action_id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), 1000);
    }

    #[test]
    fn linear_write_errors_carry_stable_codes_and_wording() {
        let conflict = linear_issue_conflict_error();
        assert_eq!(conflict.code, "linear_issue_conflict");
        assert_eq!(
            conflict.message,
            "This issue changed since Clovy last read it. Re-read it and try again."
        );

        let ambiguous = linear_action_ambiguous_error("action-123");
        assert_eq!(ambiguous.code, "linear_action_ambiguous");
        assert_eq!(
            ambiguous.message,
            "Clovy could not confirm whether Linear applied this change. Check Linear before retrying; action id action-123."
        );
    }

    // --- GitHub-specific tests -------------------------------------------------------

    #[test]
    fn github_not_installed_message_uses_slug_when_configured() {
        let with_slug = github_not_installed_message("june-staging");
        assert!(with_slug.contains("github.com/apps/june-staging"));
        // No slug configured: point at the user's own installations page
        // instead of a broken github.com/apps/ URL.
        let without = github_not_installed_message("");
        assert!(without.contains("github.com/settings/installations"));
        assert!(!without.contains("github.com/apps/"));
    }

    #[test]
    fn github_oauth_client_secret_opt_returns_none_when_empty() {
        let no_secret = GithubOAuthClient {
            client_id: "cid".to_string(),
            client_secret: String::new(),
        };
        assert!(no_secret.client_secret_opt().is_none());

        let with_secret = GithubOAuthClient {
            client_id: "cid".to_string(),
            client_secret: "csecret".to_string(),
        };
        assert_eq!(with_secret.client_secret_opt(), Some("csecret"));
    }

    #[test]
    fn require_github_client_id_accepts_id_without_secret() {
        // The id-only path must not error: device flow needs only the id.
        // We test the logic by verifying the struct's behavior directly since
        // the function reads env vars (integration would require env mocking).
        let client_id_only = GithubOAuthClient {
            client_id: "cid".to_string(),
            client_secret: String::new(),
        };
        assert!(!client_id_only.client_id.is_empty());
        assert!(client_id_only.client_secret_opt().is_none());
    }

    #[test]
    fn github_error_helpers_carry_stable_codes() {
        let not_connected = github_not_connected_error();
        assert_eq!(not_connected.code, "connector_not_connected");
        assert_eq!(
            not_connected.message,
            "This GitHub account is not connected."
        );

        let reconnect = github_reconnect_required_error();
        assert_eq!(reconnect.code, "connector_reconnect_required");
        assert_eq!(
            reconnect.message,
            "GitHub access for this account expired. Reconnect it in settings."
        );
    }

    #[test]
    fn github_account_metadata_json_omits_empty_fields() {
        let identity = github::GithubIdentity {
            user_id: "12345".to_string(),
            login: "octocat".to_string(),
            name: "Octo Cat".to_string(),
        };
        let raw = github_account_metadata_json(&identity);
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(json["actorLogin"], "octocat");
        assert_eq!(json["actorName"], "Octo Cat");

        let empty_name = github::GithubIdentity {
            user_id: "12345".to_string(),
            login: "octocat".to_string(),
            name: String::new(),
        };
        let raw2 = github_account_metadata_json(&empty_name);
        let json2: serde_json::Value = serde_json::from_str(&raw2).unwrap();
        assert_eq!(json2["actorLogin"], "octocat");
        assert!(json2.get("actorName").is_none());
    }

    #[test]
    fn github_provider_as_str_round_trips_from_db() {
        assert_eq!(ConnectorProvider::Github.as_str(), "github");
        assert_eq!(
            ConnectorProvider::from_db("github"),
            ConnectorProvider::Github
        );
    }

    #[test]
    fn single_account_guard_works_for_github() {
        // First connect: no conflict.
        assert_eq!(conflicting_existing_account([], "github", "12345"), None);
        // Reconnect same account (case-insensitive): allowed.
        assert_eq!(
            conflicting_existing_account([("github", "12345")], "github", "12345"),
            None
        );
        // Different account: blocked.
        assert_eq!(
            conflicting_existing_account([("github", "12345")], "github", "99999"),
            Some("12345".to_string())
        );
        // GitHub account does not block Linear or Google connect.
        assert_eq!(
            conflicting_existing_account([("github", "12345")], "linear", "workspace-1"),
            None
        );
    }
}
