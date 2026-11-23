//! GitHub through the GitHub CLI's own login (ADR-0066).
//!
//! When the user is signed in to `gh` on `github.com` and has no GitHub
//! account in Clovy, Clovy connects one marked "via gh". Its credential is
//! whatever `gh auth token --hostname github.com` prints at the moment of each
//! call: Clovy never copies it into the Keychain, SQLite, a file or a log, and
//! never hands it to an agent CLI. The account row only carries
//! `credentialSource: "gh"` next to the usual non-secret identity. Clovy's
//! `github_read`/`github_write` bundles and write approvals still bound what
//! the tools may do, whatever scopes the `gh` token holds.
//!
//! Disconnecting such an account writes a local preference that stops the
//! automatic connection until the user connects through `gh` again.

use super::{github, policy, ConnectorAccount, ConnectorAccountStatus, ConnectorProvider};
use crate::db::repositories::{ConnectorAccountRecord, Repositories};
use crate::domain::types::AppError;
use crate::llm::shell_env::LoginEnv;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use zeroize::{Zeroize, Zeroizing};

/// Each `gh` invocation is a short local process; detection must never hold
/// up app start or the Plugins page.
const GH_TIMEOUT: Duration = Duration::from_secs(2);
const GH_HOSTNAME: &str = "github.com";
const CREDENTIAL_SOURCE_KEY: &str = "credentialSource";
const CREDENTIAL_SOURCE_GH: &str = "gh";
const PREFERENCES_FILE: &str = "github-cli-connector.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectMode {
    /// App start and opening Plugins: honors the disconnect preference and
    /// stays silent when `gh` is missing or signed out.
    Automatic,
    /// The "Connect with GitHub CLI" button: turns the preference back on
    /// and explains what is missing.
    Explicit,
}

// ----- gh process ---------------------------------------------------------------

/// The `gh` binary on the login shell's PATH, run with that shell's
/// environment so it reads the same config the user's terminal does.
struct Gh {
    path: PathBuf,
    vars: BTreeMap<String, String>,
}

impl Gh {
    fn find(env: &LoginEnv) -> Option<Self> {
        Some(Self {
            path: env.which("gh")?,
            vars: env.vars().clone(),
        })
    }

    /// Stdout of one `gh` call when it exits successfully. Stderr is
    /// discarded so no `gh` diagnostic can carry a credential into an error.
    async fn run(&self, args: &[&str]) -> Option<Zeroizing<Vec<u8>>> {
        let mut command = crate::hide_console::tokio_command(&self.path);
        command
            .args(args)
            .env_clear()
            .envs(&self.vars)
            .env("GH_PROMPT_DISABLED", "1")
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(GH_TIMEOUT, command.output())
            .await
            .ok()?
            .ok()?;
        let stdout = Zeroizing::new(output.stdout);
        output.status.success().then_some(stdout)
    }

    async fn signed_in(&self) -> bool {
        self.run(&["auth", "status", "--hostname", GH_HOSTNAME])
            .await
            .is_some()
    }

    async fn token(&self) -> Option<Zeroizing<String>> {
        let stdout = self
            .run(&["auth", "token", "--hostname", GH_HOSTNAME])
            .await?;
        let mut text = String::from_utf8(stdout.to_vec()).ok()?;
        let token = Zeroizing::new(text.trim().to_string());
        text.zeroize();
        (!token.is_empty()).then_some(token)
    }
}

// ----- preference -----------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Preferences {
    github_cli_auto_connect: bool,
}

/// `githubCliAutoConnect`, on unless the user disconnected a "via gh"
/// account. An unreadable file reads as the default.
pub fn auto_connect_enabled(prefs_dir: &Path) -> bool {
    std::fs::read(prefs_dir.join(PREFERENCES_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Preferences>(&bytes).ok())
        .map(|prefs| prefs.github_cli_auto_connect)
        .unwrap_or(true)
}

fn set_auto_connect(prefs_dir: &Path, enabled: bool) -> Result<(), AppError> {
    let failed = |error: std::io::Error| {
        AppError::new("connector_preferences_write_failed", error.to_string())
    };
    std::fs::create_dir_all(prefs_dir).map_err(failed)?;
    let json = serde_json::to_vec(&Preferences {
        github_cli_auto_connect: enabled,
    })
    .map_err(|error| AppError::new("connector_preferences_write_failed", error.to_string()))?;
    std::fs::write(prefs_dir.join(PREFERENCES_FILE), json).map_err(failed)
}

// ----- accounts ---------------------------------------------------------------------

pub fn is_gh_account(metadata: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(metadata)
        .ok()
        .and_then(|value| {
            value
                .get(CREDENTIAL_SOURCE_KEY)
                .and_then(serde_json::Value::as_str)
                .map(|source| source == CREDENTIAL_SOURCE_GH)
        })
        .unwrap_or(false)
}

fn metadata_json(identity: &github::GithubIdentity) -> String {
    let mut map = serde_json::Map::new();
    for (key, value) in [
        ("actorLogin", &identity.login),
        ("actorName", &identity.name),
    ] {
        if !value.is_empty() {
            map.insert(key.to_string(), serde_json::Value::String(value.clone()));
        }
    }
    map.insert(
        CREDENTIAL_SOURCE_KEY.to_string(),
        serde_json::Value::String(CREDENTIAL_SOURCE_GH.to_string()),
    );
    serde_json::Value::Object(map).to_string()
}

pub fn account_from_record(record: ConnectorAccountRecord) -> ConnectorAccount {
    ConnectorAccount {
        via_gh: is_gh_account(&record.metadata),
        provider: ConnectorProvider::from_db(&record.provider),
        status: ConnectorAccountStatus::from_db(&record.status),
        account_id: record.account_id,
        email: record.email,
        scopes: record.scopes,
        workspace_name: None,
        workspace_url_key: None,
        selected_teams: Vec::new(),
    }
}

fn cli_missing_error() -> AppError {
    AppError::new(
        "connector_github_cli_missing",
        "GitHub CLI (gh) was not found on this computer. Install it, run gh auth login, then try again.",
    )
}

fn cli_signed_out_error() -> AppError {
    AppError::new(
        "connector_github_cli_logged_out",
        "GitHub CLI is not signed in to github.com. Run gh auth login, then try again.",
    )
}

fn reconnect_required_error() -> AppError {
    AppError::new(
        "connector_reconnect_required",
        "GitHub CLI is no longer signed in. Run gh auth login, then reconnect GitHub in settings.",
    )
}

async fn stored(repos: &Repositories, account_id: &str) -> Result<ConnectorAccount, AppError> {
    repos
        .get_connector_account(account_id)
        .await?
        .map(account_from_record)
        .ok_or_else(|| {
            AppError::new(
                "connector_not_connected",
                "This GitHub account is not connected.",
            )
        })
}

/// Serializes every write to GitHub account rows: the detection commit,
/// disconnect, bundle grants and the GitHub App connect. Holders reread the
/// accounts and the preference inside it, so a slow probe never decides on
/// a stale snapshot.
pub(crate) fn github_accounts_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    &LOCK
}

/// What `gh` and `GET /user` said, gathered outside the account lock.
pub(crate) enum GhProbe {
    Missing,
    SignedOut,
    Failed(AppError),
    Identity(github::GithubIdentity),
}

pub(crate) async fn probe_gh(env: &LoginEnv) -> GhProbe {
    let Some(gh) = Gh::find(env) else {
        return GhProbe::Missing;
    };
    let token = if gh.signed_in().await {
        gh.token().await
    } else {
        None
    };
    let Some(token) = token else {
        return GhProbe::SignedOut;
    };
    match github::fetch_identity(&token).await {
        Ok(identity) => {
            remember_owner(&token, &identity.user_id);
            GhProbe::Identity(identity)
        }
        Err(error) => GhProbe::Failed(error),
    }
}

async fn github_record(repos: &Repositories) -> Result<Option<ConnectorAccountRecord>, AppError> {
    Ok(repos
        .list_connector_accounts()
        .await?
        .into_iter()
        .find(|record| record.provider == ConnectorProvider::Github.as_str()))
}

fn app_account_present(
    explicit: bool,
    record: &ConnectorAccountRecord,
) -> Result<Option<ConnectorAccount>, AppError> {
    if explicit {
        return Err(AppError::new(
            "connector_single_account_only",
            format!(
                "Clovy local mode uses one GitHub account at a time. Disconnect {} before connecting another.",
                record.email
            ),
        ));
    }
    Ok(None)
}

fn account_mismatch_error(gh_login: &str, account_login: &str) -> AppError {
    AppError::new(
        "connector_account_mismatch",
        format!(
            "GitHub CLI is signed in as {gh_login}, not {account_login}. Switch back with gh auth switch, or disconnect GitHub in Clovy first."
        ),
    )
}

/// Connect "GitHub (via gh)" when there is no GitHub account yet, and heal a
/// "via gh" account that went `reconnect_required` once `gh` is signed in
/// again as the same user. An account connected through the GitHub App is
/// left alone. Only the read bundle is granted.
pub async fn detect_and_connect(
    repos: &Repositories,
    env: &LoginEnv,
    prefs_dir: &Path,
    mode: ConnectMode,
) -> Result<Option<ConnectorAccount>, AppError> {
    let explicit = mode == ConnectMode::Explicit;
    // Cheap early exits before spawning gh; `commit` checks again under the lock.
    if !explicit && !auto_connect_enabled(prefs_dir) {
        return Ok(None);
    }
    if let Some(record) = github_record(repos).await? {
        if !is_gh_account(&record.metadata) {
            return app_account_present(explicit, &record);
        }
    }
    let probe = probe_gh(env).await;
    commit(repos, prefs_dir, mode, probe).await
}

/// Decide and write on the accounts as they are now, under the lifecycle lock.
pub(crate) async fn commit(
    repos: &Repositories,
    prefs_dir: &Path,
    mode: ConnectMode,
    probe: GhProbe,
) -> Result<Option<ConnectorAccount>, AppError> {
    let explicit = mode == ConnectMode::Explicit;
    let _lifecycle = github_accounts_lock().lock().await;
    if !explicit && !auto_connect_enabled(prefs_dir) {
        return Ok(None);
    }
    let existing = github_record(repos).await?;
    if let Some(record) = existing.as_ref() {
        if !is_gh_account(&record.metadata) {
            return app_account_present(explicit, record);
        }
    }
    let identity = match probe {
        GhProbe::Identity(identity) => identity,
        GhProbe::Missing if explicit => return Err(cli_missing_error()),
        GhProbe::SignedOut if explicit => return Err(cli_signed_out_error()),
        GhProbe::Failed(error) if explicit => return Err(error),
        GhProbe::Failed(error) => {
            tracing::warn!(error_code = %error.code, "github cli identity check failed");
            return Ok(existing.map(account_from_record));
        }
        GhProbe::Missing | GhProbe::SignedOut => return Ok(existing.map(account_from_record)),
    };

    if let Some(record) = existing {
        // Heal only the same GitHub user; a different gh login needs the
        // user to switch gh back or disconnect first, like any account switch.
        if !record.account_id.eq_ignore_ascii_case(&identity.user_id) {
            if explicit {
                return Err(account_mismatch_error(&identity.login, &record.email));
            }
            return Ok(Some(account_from_record(record)));
        }
        if record.status != ConnectorAccountStatus::Connected.as_str() {
            repos
                .set_connector_account_status(
                    &record.account_id,
                    ConnectorAccountStatus::Connected.as_str(),
                )
                .await?;
        }
    } else {
        let scopes = vec![policy::GITHUB_READ.to_string()];
        repos
            .upsert_connector_account(
                &identity.user_id,
                ConnectorProvider::Github.as_str(),
                &identity.login,
                &scopes,
                ConnectorAccountStatus::Connected.as_str(),
                &metadata_json(&identity),
            )
            .await?;
    }
    if explicit {
        set_auto_connect(prefs_dir, true)?;
    }
    stored(repos, &identity.user_id).await.map(Some)
}

// ----- token owner -----------------------------------------------------------------

/// How long a confirmed "token → GitHub user" pairing is trusted.
const OWNER_CONFIRMATION_TTL: Duration = Duration::from_secs(60);

type TokenDigest = [u8; 32];

/// In memory only, and only the SHA-256 of the token: never the token itself,
/// never on disk.
fn owner_confirmations() -> &'static std::sync::Mutex<HashMap<TokenDigest, (String, Instant)>> {
    static CONFIRMED: OnceLock<std::sync::Mutex<HashMap<TokenDigest, (String, Instant)>>> =
        OnceLock::new();
    CONFIRMED.get_or_init(Default::default)
}

fn token_digest(token: &str) -> TokenDigest {
    Sha256::digest(token.as_bytes()).into()
}

fn remember_owner(token: &str, user_id: &str) {
    if let Ok(mut confirmed) = owner_confirmations().lock() {
        confirmed.retain(|_, (_, at)| at.elapsed() < OWNER_CONFIRMATION_TTL);
        confirmed.insert(token_digest(token), (user_id.to_string(), Instant::now()));
    }
}

fn confirmed_owner(token: &str) -> Option<String> {
    let confirmed = owner_confirmations().lock().ok()?;
    confirmed
        .get(&token_digest(token))
        .filter(|(_, at)| at.elapsed() < OWNER_CONFIRMATION_TTL)
        .map(|(user_id, _)| user_id.clone())
}

/// The `gh` token for one call, only when it belongs to `account_id`. When
/// `gh` no longer yields one, or yields another user's (after
/// `gh auth switch`), the account turns `reconnect_required` and the call
/// fails closed.
pub async fn access_token(
    repos: &Repositories,
    env: &LoginEnv,
    account_id: &str,
) -> Result<String, AppError> {
    let token = match Gh::find(env) {
        Some(gh) => gh.token().await,
        None => None,
    };
    let Some(token) = token else {
        mark_reconnect_required(repos, account_id).await?;
        return Err(reconnect_required_error());
    };
    let owner = match confirmed_owner(&token) {
        Some(owner) => owner,
        None => {
            let identity = github::fetch_identity(&token).await?;
            remember_owner(&token, &identity.user_id);
            identity.user_id
        }
    };
    if !owner.eq_ignore_ascii_case(account_id) {
        mark_reconnect_required(repos, account_id).await?;
        let account_login = repos
            .get_connector_account(account_id)
            .await?
            .map(|record| record.email)
            .unwrap_or_else(|| account_id.to_string());
        return Err(account_mismatch_error(
            "another GitHub user",
            &account_login,
        ));
    }
    Ok(token.as_str().to_string())
}

async fn mark_reconnect_required(repos: &Repositories, account_id: &str) -> Result<(), AppError> {
    repos
        .set_connector_account_status(
            account_id,
            ConnectorAccountStatus::ReconnectRequired.as_str(),
        )
        .await?;
    Ok(())
}

/// Disconnect a "via gh" account: drop the row (there is no Clovy-held
/// credential to revoke or delete) and stop the automatic connection.
pub async fn disconnect(
    repos: &Repositories,
    prefs_dir: &Path,
    account_id: &str,
) -> Result<(), AppError> {
    let _lifecycle = github_accounts_lock().lock().await;
    set_auto_connect(prefs_dir, false)?;
    repos.delete_connector_account(account_id).await?;
    Ok(())
}

/// "Add access" for a "via gh" account: the `gh` token already reaches
/// GitHub, so adding a bundle is a Clovy-side grant with no device flow.
pub async fn grant_bundles(
    repos: &Repositories,
    account_id: &str,
    bundles: &[policy::ScopeBundle],
) -> Result<ConnectorAccount, AppError> {
    let _lifecycle = github_accounts_lock().lock().await;
    let record = repos
        .get_connector_account(account_id)
        .await?
        .filter(|record| is_gh_account(&record.metadata))
        .ok_or_else(|| {
            AppError::new(
                "connector_not_connected",
                "This GitHub account is not connected.",
            )
        })?;
    let requested = policy::requested_github_scopes(bundles);
    let scopes = policy::resolve_granted_scopes(None, &requested, Some(&record.scopes));
    repos
        .upsert_connector_account(
            &record.account_id,
            &record.provider,
            &record.email,
            &scopes,
            &record.status,
            &record.metadata,
        )
        .await?;
    stored(repos, account_id).await
}

#[cfg(all(test, unix))]
#[path = "github_cli_tests.rs"]
mod tests;
