//! Tauri commands for connectors, routine trust modes, and event triggers.
//! Registered in lib.rs. Request/response
//! payloads are camelCase, matching the existing command structs.

use crate::db::repositories::{ConnectorGrant, Repositories, RoutineTrustRecord};
use crate::domain::types::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};

use super::{
    begin_connect, begin_connect_github, begin_connect_linear, disconnect, emit_connectors_changed,
    linear, list_accounts_resilient, list_google_accounts, notion,
    policy::{self, ScopeBundle},
    ConnectFlow, ConnectorAccount, ConnectorAccountStatus, ConnectorProvider, NotionConnectFlow,
    SelectedTeamDto,
};

/// Presentation-free connector policy. Additive command: older frontend/native
/// pairs keep their existing command shapes, while current React reads all
/// authorization-relevant catalog data from this projection.
#[tauri::command]
pub fn connectors_policy() -> policy::ConnectorPolicyCatalog {
    policy::catalog()
}

#[tauri::command]
pub async fn connectors_list(app: tauri::AppHandle) -> Result<Vec<ConnectorAccount>, AppError> {
    list_accounts_resilient(&app).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorsConnectRequest {
    /// Bundle names: "gmail_read" | "gmail_draft" | "gmail_send" |
    /// "calendar_events" | "linear_read" | "linear_write". Every bundle must
    /// belong to the requested provider.
    pub scopes: Vec<String>,
    /// Existing account identity for a reconnect or incremental scope
    /// escalation: the account EMAIL for Google, the ACCOUNT ID (workspace
    /// id) for Linear.
    #[serde(default)]
    pub login_hint: Option<String>,
    /// "google" | "linear". Absent means "google": the field predates
    /// multi-provider support and older frontends never send it.
    #[serde(default)]
    pub provider: Option<String>,
}

#[tauri::command]
pub async fn connectors_connect(
    app: tauri::AppHandle,
    flow: tauri::State<'_, ConnectFlow>,
    request: ConnectorsConnectRequest,
) -> Result<ConnectorAccount, AppError> {
    let provider = parse_provider(request.provider.as_deref())?;
    let bundles = parse_bundles(&request.scopes)?;
    validate_bundle_providers(&bundles, provider)?;
    let _operation = flow.begin_operation()?;
    let account = match provider {
        ConnectorProvider::Google => {
            begin_connect(&app, &flow, &bundles, request.login_hint.as_deref()).await?
        }
        ConnectorProvider::Linear => {
            begin_connect_linear(&app, &flow, &bundles, request.login_hint.as_deref()).await?
        }
        ConnectorProvider::Github => {
            begin_connect_github(&app, &flow, &bundles, request.login_hint.as_deref()).await?
        }
        ConnectorProvider::Notion => {
            return Err(AppError::new(
                "connector_provider_invalid",
                "Notion uses its dedicated connect flow.",
            ));
        }
    };
    if provider != ConnectorProvider::Google {
        return Ok(account);
    }

    // Re-mint autonomous grants for routines that still declare autonomous
    // trust. A prior disconnect deletes an account's grants but keeps the
    // routine_trust rows and the jobs' auto toolsets, so reconnecting the same
    // account must restore the grants or those routines stay autonomous in name
    // only (their action servers never render). Single-account mode makes "the
    // connected account" unambiguous. Best-effort and per routine: a re-mint
    // failure must not fail the connect, and each grant is recreated with the
    // token carried over when the tool set is unchanged, so this is a no-op for
    // routines that already hold valid grants (a plain scope escalation).
    // Google-only: the grants are Gmail/Calendar autonomy, so a Linear connect
    // never touches them.
    let repos = crate::commands::repositories(&app).await?;
    match repos.list_routine_trust_by_mode("autonomous").await {
        Ok(records) => {
            for record in records {
                if let Err(error) = mint_autonomy_grants(&app, &repos, &record).await {
                    tracing::warn!(error_code = %error.code, "re-mint autonomous grants on connect failed");
                }
            }
        }
        Err(error) => {
            tracing::warn!(error_code = %AppError::from(error).code, "autonomous routine lookup on connect failed");
        }
    }

    Ok(account)
}

/// Connect "GitHub (via gh)" on its own when the GitHub CLI is signed in and
/// no GitHub account exists. Runs when Plugins opens; never errors the page.
#[tauri::command]
pub async fn connectors_github_cli_detect(
    app: tauri::AppHandle,
) -> Result<Option<ConnectorAccount>, AppError> {
    match super::connect_github_cli(&app, super::github_cli::ConnectMode::Automatic).await {
        Ok(account) => Ok(account),
        Err(error) => {
            tracing::warn!(error_code = %error.code, "github cli detection failed");
            Ok(None)
        }
    }
}

/// The "Connect with GitHub CLI" button: connects through gh and turns the
/// automatic connection back on.
#[tauri::command]
pub async fn connectors_github_cli_connect(
    app: tauri::AppHandle,
) -> Result<Option<ConnectorAccount>, AppError> {
    super::connect_github_cli(&app, super::github_cli::ConnectMode::Explicit).await
}

#[tauri::command]
pub fn connectors_cancel_connect(flow: tauri::State<'_, ConnectFlow>) -> Result<(), AppError> {
    super::cancel_connect(&flow);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorsDisconnectRequest {
    pub account_id: String,
    /// Also ask the provider to revoke the grant after removing local custody.
    #[serde(default)]
    pub revoke: bool,
}

#[tauri::command]
pub async fn connectors_disconnect(
    app: tauri::AppHandle,
    request: ConnectorsDisconnectRequest,
) -> Result<super::DisconnectOutcome, AppError> {
    disconnect(&app, &request.account_id, request.revoke).await
}

#[tauri::command]
pub async fn notion_connector_status(
    app: tauri::AppHandle,
) -> Result<notion::NotionConnectionStatus, AppError> {
    notion::status(&app).await
}

#[tauri::command]
pub async fn notion_connector_connect(
    app: tauri::AppHandle,
    flow: tauri::State<'_, NotionConnectFlow>,
) -> Result<notion::NotionConnection, AppError> {
    let connection = notion::connect(&app, &flow).await?;
    emit_connectors_changed(&app);
    Ok(connection)
}

#[tauri::command]
pub fn notion_connector_cancel_connect(
    flow: tauri::State<'_, NotionConnectFlow>,
) -> Result<(), AppError> {
    flow.cancel();
    Ok(())
}

#[tauri::command]
pub async fn notion_connector_disconnect(app: tauri::AppHandle) -> Result<(), AppError> {
    notion::disconnect(&app).await?;
    emit_connectors_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn notion_connector_list_tools(
    app: tauri::AppHandle,
) -> Result<notion::NotionToolInventory, AppError> {
    notion::list_tools(&app).await
}

// --- Linear teams --------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorsLinearTeamsRequest {
    /// Linear account id (the workspace id).
    pub account_id: String,
}

/// The workspace's teams for the selection UI, live from Linear. A rejected
/// access token is force-refreshed once and the listing retried, matching
/// the Google retry-once convention.
#[tauri::command]
pub async fn connectors_linear_teams(
    app: tauri::AppHandle,
    request: ConnectorsLinearTeamsRequest,
) -> Result<LinearTeamsDto, AppError> {
    let repos = crate::commands::repositories(&app).await?;
    require_linear_account(&repos, &request.account_id).await?;
    let token = super::linear_access_token(&app, &request.account_id).await?;
    let listing = match linear::list_teams(&token).await {
        Ok(listing) => listing,
        Err(linear::LinearApiError::Unauthorized) => {
            let token = super::force_refresh_linear_access_token(&app, &request.account_id).await?;
            linear::list_teams(&token).await.map_err(AppError::from)?
        }
        Err(error) => return Err(error.into()),
    };
    Ok(LinearTeamsDto {
        teams: listing
            .teams
            .into_iter()
            .map(|team| SelectedTeamDto {
                id: team.id,
                key: team.key,
                name: team.name,
            })
            .collect(),
        truncated: listing.truncated,
    })
}

/// The live team listing for the selection dialog. `truncated` means the
/// pagination cap cut the listing short, so the UI must not present it as
/// the complete team inventory.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearTeamsDto {
    pub teams: Vec<SelectedTeamDto>,
    pub truncated: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorsSelectedTeamsSetRequest {
    /// Linear account id (the workspace id).
    pub account_id: String,
    /// The full desired team set; replaces the stored selection wholesale.
    pub teams: Vec<SelectedTeamDto>,
}

/// Replace the account's selected-team set. The selection is Clovy's own
/// authorization boundary (Linear has no teams-scoped grant), so it must
/// never be empty: an empty set would make every team query answer nothing
/// while looking connected.
#[tauri::command]
pub async fn connectors_selected_teams_set(
    app: tauri::AppHandle,
    request: ConnectorsSelectedTeamsSetRequest,
) -> Result<ConnectorAccount, AppError> {
    let repos = crate::commands::repositories(&app).await?;
    require_linear_account(&repos, &request.account_id).await?;
    let teams = validate_selected_teams(&request.teams)?;
    let records: Vec<crate::db::repositories::SelectedTeamRecord> = teams
        .into_iter()
        .map(|team| crate::db::repositories::SelectedTeamRecord {
            team_id: team.id,
            team_key: team.key,
            team_name: team.name,
        })
        .collect();
    repos
        .set_selected_teams(&request.account_id, &records)
        .await?;
    super::emit_connectors_changed(&app);
    let record = repos
        .get_connector_account(&request.account_id)
        .await?
        .ok_or_else(linear_account_not_found)?;
    super::account_dto(&repos, record).await
}

fn linear_account_not_found() -> AppError {
    AppError::new(
        "connector_account_not_found",
        "That Linear workspace is not connected.",
    )
}

/// The account must exist AND be a Linear row: a Google email passed as the
/// account id must not pass the existence check and then hit Linear APIs.
async fn require_linear_account(repos: &Repositories, account_id: &str) -> Result<(), AppError> {
    let record = repos.get_connector_account(account_id).await?;
    match record {
        Some(record) if record.provider == ConnectorProvider::Linear.as_str() => Ok(()),
        _ => Err(linear_account_not_found()),
    }
}

// --- Routine trust modes -----------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineTrustDto {
    pub trust_mode: String,
    pub approval_run_count: i64,
    pub autonomous_tools: Vec<String>,
    /// Names of the per-job auto MCP servers minted for the current
    /// autonomous grants (`june_<provider>_auto_<jobid8>`), sorted. Empty
    /// unless the routine is autonomous with granted mutating tools.
    pub autonomous_servers: Vec<String>,
}

#[tauri::command]
pub async fn routine_trust_get(
    app: tauri::AppHandle,
    job_id: String,
) -> Result<Option<RoutineTrustDto>, AppError> {
    let repos = crate::commands::repositories(&app).await?;
    let Some(record) = repos.routine_trust_get(&job_id).await? else {
        return Ok(None);
    };
    // Reflect the persisted grants so the UI shows the right toolset
    // composition on load.
    let servers = grant_server_names(&repos, &job_id).await?;
    Ok(Some(trust_dto(record, servers)))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineTrustSetRequest {
    pub job_id: String,
    /// "read_only" | "approval" | "autonomous".
    pub trust_mode: String,
    /// Per-tool autonomy grants; only meaningful for "autonomous". Omitted
    /// means keep the existing grants.
    #[serde(default)]
    pub autonomous_tools: Option<Vec<String>>,
}

#[tauri::command]
pub async fn routine_trust_set(
    app: tauri::AppHandle,
    request: RoutineTrustSetRequest,
) -> Result<RoutineTrustDto, AppError> {
    validate_trust_mode(&request.trust_mode)?;
    let repos = crate::commands::repositories(&app).await?;
    let existing = repos.routine_trust_get(&request.job_id).await?;
    if request.trust_mode == "autonomous" {
        let run_count = existing
            .as_ref()
            .map(|record| record.approval_run_count)
            .unwrap_or(0);
        if !autonomy_earned(run_count) {
            return Err(AppError::new(
                "routine_trust_not_earned",
                format!(
                    "This routine needs {} completed approval runs before it can act on its own.",
                    policy::EARNED_AUTONOMY_MIN_APPROVAL_RUNS
                ),
            ));
        }
    }
    let autonomous_tools = request.autonomous_tools.unwrap_or_else(|| {
        existing
            .map(|record| record.autonomous_tools)
            .unwrap_or_default()
    });
    let record = repos
        .routine_trust_set(&request.job_id, &request.trust_mode, &autonomous_tools)
        .await?;
    // Autonomy grant minting is the choke point for session-blind
    // attribution: an autonomous routine gets a per-provider grant (token +
    // tool names) that the bridge carries into a per-job auto MCP server.
    let servers = if record.trust_mode == "autonomous" {
        mint_autonomy_grants(&app, &repos, &record).await?
    } else {
        repos.delete_connector_grants(&record.job_id).await?;
        Vec::new()
    };
    Ok(trust_dto(record, servers))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineRunRecordRequest {
    pub job_id: String,
    /// The completed run's session id, used to credit each run exactly once.
    pub run_id: String,
    /// When the run finished (RFC 3339). Runs that finished before the routine
    /// entered approval mode are ignored.
    pub run_ended_at: String,
}

/// Called when a routine run completes; credits it toward the earned-autonomy
/// counter exactly once. Returns `None` (a no-op) when the run is not
/// approval-mode, predates the approval window, or was already counted, so the
/// caller can fire this for every finished run without bookkeeping of its own.
#[tauri::command]
pub async fn routine_trust_record_run(
    app: tauri::AppHandle,
    request: RoutineRunRecordRequest,
) -> Result<Option<RoutineTrustDto>, AppError> {
    let repos = crate::commands::repositories(&app).await?;
    let Some(record) = repos
        .record_approval_run(&request.job_id, &request.run_id, &request.run_ended_at)
        .await?
    else {
        return Ok(None);
    };
    let servers = grant_server_names(&repos, &request.job_id).await?;
    Ok(Some(trust_dto(record, servers)))
}

/// Build the trust DTO from a persisted record plus its current auto-server
/// names.
fn trust_dto(record: RoutineTrustRecord, autonomous_servers: Vec<String>) -> RoutineTrustDto {
    RoutineTrustDto {
        trust_mode: record.trust_mode,
        approval_run_count: record.approval_run_count,
        autonomous_tools: record.autonomous_tools,
        autonomous_servers,
    }
}

/// Sorted names of the auto MCP servers backing a job's current grants.
async fn grant_server_names(repos: &Repositories, job_id: &str) -> Result<Vec<String>, AppError> {
    let mut names: Vec<String> = repos
        .connector_grants_for_job(job_id)
        .await?
        .into_iter()
        .map(|grant| grant.server_name)
        .collect();
    names.sort();
    Ok(names)
}

/// The provider whose auto server owns a granted tool, or None for tools
/// that never need a grant.
///
/// Linear, Notion, and GitHub actions never earn autonomy in v1. The native
/// policy excludes every tool from those servers: returning None means the
/// grant-minting path ignores it and no auto server can be created.
fn provider_for_tool(tool: &str) -> Option<&'static str> {
    policy::autonomy_provider_for_tool(tool)
}

/// A collision-free, deterministic suffix for a job's auto MCP server name.
/// A short sanitized prefix keeps the name recognizable, and a hash of the full
/// job id disambiguates ids that share a prefix or sanitize alike, so two
/// autonomous routines can never mint the same `june_<provider>_auto_<...>`
/// server name and clobber each other's grant (and token) in `config.yaml`.
/// The bridge reads the stored `server_name`, so this authoring is the single
/// source of truth; no other layer re-derives it.
fn job_server_suffix(job_id: &str) -> String {
    let readable: String = job_id
        .chars()
        .map(|ch| ch.to_ascii_lowercase())
        .filter(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        .take(8)
        .collect();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    job_id.hash(&mut hasher);
    let hash = format!("{:016x}", hasher.finish());
    if readable.is_empty() {
        hash
    } else {
        format!("{readable}_{hash}")
    }
}

fn tools_match(existing: &[String], wanted: &[String]) -> bool {
    let mut existing = existing.to_vec();
    let mut wanted = wanted.to_vec();
    existing.sort();
    wanted.sort();
    existing == wanted
}

/// Mint (or preserve) the per-provider autonomy grants for a routine that is
/// now autonomous.
///
/// Re-mint policy: the grant token is reused for a provider whose granted
/// tool set is unchanged, so an unrelated edit does not churn the token and
/// force a runtime restart; the token is re-minted only when that provider's
/// tools actually change. Providers no longer granted are dropped. Returns
/// the minted server names, sorted.
///
/// Account resolution is best-effort: the first connected Google account's
/// email. When none is connected the grant is still written with an empty
/// `account_id` (the bridge simply will not spawn a usable server); this is not
/// an error, so setting autonomy before connecting an account still succeeds.
async fn mint_autonomy_grants(
    app: &tauri::AppHandle,
    repos: &Repositories,
    record: &RoutineTrustRecord,
) -> Result<Vec<String>, AppError> {
    let account_id = first_connected_google_account_email(app).await;
    let job_suffix = job_server_suffix(&record.job_id);

    // Granted mutating tools grouped by provider (deduped, sorted, ordered).
    let mut by_provider: BTreeMap<&'static str, Vec<String>> = BTreeMap::new();
    for tool in &record.autonomous_tools {
        if let Some(provider) = provider_for_tool(tool) {
            let entry = by_provider.entry(provider).or_default();
            if !entry.iter().any(|existing| existing == tool) {
                entry.push(tool.clone());
            }
        }
    }
    for tools in by_provider.values_mut() {
        tools.sort();
    }

    // Existing grants (pre-change), so an unchanged provider keeps its token.
    let existing: BTreeMap<String, ConnectorGrant> = repos
        .connector_grants_for_job(&record.job_id)
        .await?
        .into_iter()
        .map(|grant| (grant.provider.clone(), grant))
        .collect();

    // Recreate the job's grants for exactly the current provider set. This
    // drops providers no longer granted; tokens are carried over where the
    // tool set matched.
    repos.delete_connector_grants(&record.job_id).await?;
    let created_at = crate::db::repositories::timestamp();
    let mut server_names = Vec::with_capacity(by_provider.len());
    for (provider, tools) in &by_provider {
        let server_name = format!("june_{provider}_auto_{job_suffix}");
        let token = match existing.get(*provider) {
            Some(previous) if tools_match(&previous.tools, tools) => previous.token.clone(),
            _ => super::random_b64url(32),
        };
        let grant = ConnectorGrant {
            job_id: record.job_id.clone(),
            provider: (*provider).to_string(),
            server_name: server_name.clone(),
            token,
            tools: tools.clone(),
            account_id: account_id.clone(),
        };
        repos.set_connector_grant(&grant, &created_at).await?;
        server_names.push(server_name);
    }
    server_names.sort();
    Ok(server_names)
}

async fn first_connected_google_account_email(app: &tauri::AppHandle) -> String {
    match list_google_accounts(app).await {
        Ok(accounts) => first_connected_google_account_email_from(accounts),
        // Never fail the trust change on an account-enumeration hiccup; the
        // grant is still written and can be re-minted once an account exists.
        Err(_) => String::new(),
    }
}

fn first_connected_google_account_email_from(accounts: Vec<ConnectorAccount>) -> String {
    accounts
        .into_iter()
        .find(|account| {
            account.provider == ConnectorProvider::Google
                && account.status == ConnectorAccountStatus::Connected
        })
        .map(|account| account.email)
        .unwrap_or_default()
}

// --- Connector triggers --------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorTriggerDto {
    pub id: String,
    pub job_id: String,
    pub kind: String,
    pub account_id: String,
    pub config: serde_json::Value,
    pub created_at: String,
}

#[tauri::command]
pub async fn connector_triggers_list(
    app: tauri::AppHandle,
    job_id: Option<String>,
) -> Result<Vec<ConnectorTriggerDto>, AppError> {
    let repos = crate::commands::repositories(&app).await?;
    let records = repos.list_connector_triggers(job_id.as_deref()).await?;
    Ok(records
        .into_iter()
        .map(|record| ConnectorTriggerDto {
            config: serde_json::from_str(&record.config)
                .unwrap_or(serde_json::Value::Object(Default::default())),
            id: record.id,
            job_id: record.job_id,
            kind: record.kind,
            account_id: record.account_id,
            created_at: record.created_at,
        })
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorTriggerSetRequest {
    pub job_id: String,
    /// "email_received" | "event_upcoming".
    pub kind: String,
    pub account_id: String,
    /// Trigger-specific settings (e.g. a Gmail query, a lead-time window).
    #[serde(default)]
    pub config: Option<serde_json::Value>,
}

#[tauri::command]
pub async fn connector_trigger_set(
    app: tauri::AppHandle,
    request: ConnectorTriggerSetRequest,
) -> Result<ConnectorTriggerDto, AppError> {
    validate_trigger_kind(&request.kind)?;
    let repos = crate::commands::repositories(&app).await?;
    // Triggers poll Gmail history and Calendar events, so the subscribed
    // account must be a Google one: a Linear workspace id would pass a bare
    // existence check and leave the routine silently never firing.
    let is_google_account = repos
        .get_connector_account(&request.account_id)
        .await?
        .is_some_and(|record| {
            super::ConnectorProvider::from_db(&record.provider) == super::ConnectorProvider::Google
        });
    if !is_google_account {
        return Err(AppError::new(
            "connector_account_not_found",
            "That Google account is not connected.",
        ));
    }
    let config = request
        .config
        .unwrap_or(serde_json::Value::Object(Default::default()));
    let config_json = serde_json::to_string(&config)
        .map_err(|e| AppError::new("connector_trigger_invalid_config", e.to_string()))?;

    let record = repos
        .replace_connector_trigger(
            &request.job_id,
            &request.kind,
            &request.account_id,
            &config_json,
        )
        .await?;

    Ok(ConnectorTriggerDto {
        config,
        id: record.id,
        job_id: record.job_id,
        kind: record.kind,
        account_id: record.account_id,
        created_at: record.created_at,
    })
}

#[tauri::command]
pub async fn connector_trigger_delete(app: tauri::AppHandle, id: String) -> Result<(), AppError> {
    let repos = crate::commands::repositories(&app).await?;
    // Idempotent: deleting an already-deleted trigger is not an error.
    repos.delete_connector_trigger(&id).await?;
    Ok(())
}

// --- Validation helpers -----------------------------------------------------------

fn parse_bundles(names: &[String]) -> Result<Vec<ScopeBundle>, AppError> {
    if names.is_empty() {
        return Err(AppError::new(
            "connector_unknown_scope_bundle",
            "At least one scope bundle is required.",
        ));
    }
    names
        .iter()
        .map(|name| {
            ScopeBundle::from_name(name).ok_or_else(|| {
                AppError::new(
                    "connector_unknown_scope_bundle",
                    format!("Unknown scope bundle \"{name}\"."),
                )
            })
        })
        .collect()
}

/// Parse the connect request's provider field. Absent means Google (the
/// field predates multi-provider support); anything else must name a known
/// provider exactly.
fn parse_provider(value: Option<&str>) -> Result<ConnectorProvider, AppError> {
    match value {
        None => Ok(ConnectorProvider::Google),
        Some(value) => match value.trim() {
            "google" => Ok(ConnectorProvider::Google),
            "linear" => Ok(ConnectorProvider::Linear),
            "github" => Ok(ConnectorProvider::Github),
            _ => Err(AppError::new(
                "connector_unknown_provider",
                format!("Unknown connector provider \"{value}\"."),
            )),
        },
    }
}

/// Every requested bundle must belong to the provider being connected: a
/// mixed request would silently drop the foreign bundles' scopes from the
/// consent screen while the caller believes they were granted.
fn validate_bundle_providers(
    bundles: &[ScopeBundle],
    provider: ConnectorProvider,
) -> Result<(), AppError> {
    for bundle in bundles {
        if bundle.provider() != provider {
            return Err(AppError::new(
                "connector_scope_provider_mismatch",
                format!(
                    "Scope bundle \"{}\" does not belong to the {} connector.",
                    bundle.name(),
                    provider.as_str()
                ),
            ));
        }
    }
    Ok(())
}

/// Validate a selected-teams payload: non-empty, every field non-blank
/// (trimmed), deduplicated by team id preserving first-seen order. Returns
/// the cleaned (trimmed, deduped) set to persist.
fn validate_selected_teams(teams: &[SelectedTeamDto]) -> Result<Vec<SelectedTeamDto>, AppError> {
    if teams.is_empty() {
        return Err(AppError::new(
            "connector_teams_required",
            "Select at least one team.",
        ));
    }
    let mut cleaned: Vec<SelectedTeamDto> = Vec::with_capacity(teams.len());
    for team in teams {
        let id = team.id.trim();
        let key = team.key.trim();
        let name = team.name.trim();
        if id.is_empty() || key.is_empty() || name.is_empty() {
            return Err(AppError::new(
                "connector_teams_invalid",
                "Team entries need an id, key, and name.",
            ));
        }
        if cleaned.iter().any(|existing| existing.id == id) {
            continue;
        }
        cleaned.push(SelectedTeamDto {
            id: id.to_string(),
            key: key.to_string(),
            name: name.to_string(),
        });
    }
    Ok(cleaned)
}

fn validate_trust_mode(mode: &str) -> Result<(), AppError> {
    if policy::TRUST_MODES.contains(&mode) {
        Ok(())
    } else {
        Err(AppError::new(
            "routine_trust_invalid_mode",
            format!("Unknown trust mode \"{mode}\"."),
        ))
    }
}

fn validate_trigger_kind(kind: &str) -> Result<(), AppError> {
    if policy::is_trigger_kind(kind) {
        Ok(())
    } else {
        Err(AppError::new(
            "connector_trigger_invalid_kind",
            format!("Unknown trigger kind \"{kind}\"."),
        ))
    }
}

fn autonomy_earned(approval_run_count: i64) -> bool {
    policy::autonomy_earned(approval_run_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle(name: &str) -> ScopeBundle {
        ScopeBundle::from_name(name).expect("known bundle")
    }

    #[test]
    fn parses_known_bundles_and_rejects_unknown() {
        let bundles = parse_bundles(&["gmail_read".to_string(), "calendar_events".to_string()])
            .expect("known bundles");
        assert_eq!(
            bundles,
            vec![bundle("gmail_read"), bundle("calendar_events")]
        );

        let error = parse_bundles(&["gmail_everything".to_string()]).unwrap_err();
        assert_eq!(error.code, "connector_unknown_scope_bundle");
        let error = parse_bundles(&[]).unwrap_err();
        assert_eq!(error.code, "connector_unknown_scope_bundle");
    }

    #[test]
    fn provider_parsing_defaults_to_google_and_rejects_unknown() {
        // Absent field: requests that predate multi-provider are Google.
        assert_eq!(parse_provider(None).unwrap(), ConnectorProvider::Google);
        assert_eq!(
            parse_provider(Some("google")).unwrap(),
            ConnectorProvider::Google
        );
        assert_eq!(
            parse_provider(Some("linear")).unwrap(),
            ConnectorProvider::Linear
        );
        assert_eq!(
            parse_provider(Some("github")).unwrap(),
            ConnectorProvider::Github
        );
        let error = parse_provider(Some("jira")).unwrap_err();
        assert_eq!(error.code, "connector_unknown_provider");
        assert!(error.message.contains("\"jira\""));
        assert_eq!(
            parse_provider(Some("")).unwrap_err().code,
            "connector_unknown_provider"
        );
    }

    #[test]
    fn bundle_provider_mismatch_is_rejected_both_ways() {
        // A Google bundle cannot ride a Linear connect...
        let error = validate_bundle_providers(
            &[bundle("linear_read"), bundle("gmail_read")],
            ConnectorProvider::Linear,
        )
        .unwrap_err();
        assert_eq!(error.code, "connector_scope_provider_mismatch");
        assert!(error.message.contains("\"gmail_read\""));
        assert!(error.message.contains("linear"));
        // ...nor a Linear bundle a Google connect.
        let error = validate_bundle_providers(&[bundle("linear_write")], ConnectorProvider::Google)
            .unwrap_err();
        assert_eq!(error.code, "connector_scope_provider_mismatch");
        // Matching sets pass for both providers.
        assert!(validate_bundle_providers(
            &[bundle("gmail_read"), bundle("calendar_events")],
            ConnectorProvider::Google
        )
        .is_ok());
        assert!(validate_bundle_providers(
            &[bundle("linear_read"), bundle("linear_write")],
            ConnectorProvider::Linear
        )
        .is_ok());
        assert!(validate_bundle_providers(
            &[bundle("github_read"), bundle("github_write")],
            ConnectorProvider::Github
        )
        .is_ok());
        // A GitHub bundle cannot ride a non-GitHub connect.
        let error = validate_bundle_providers(&[bundle("github_read")], ConnectorProvider::Google)
            .unwrap_err();
        assert_eq!(error.code, "connector_scope_provider_mismatch");
    }

    fn team(id: &str, key: &str, name: &str) -> SelectedTeamDto {
        SelectedTeamDto {
            id: id.to_string(),
            key: key.to_string(),
            name: name.to_string(),
        }
    }

    #[test]
    fn selected_teams_payload_validation() {
        // Empty set: the selection is Clovy's authorization boundary and must
        // never be cleared to nothing.
        assert_eq!(
            validate_selected_teams(&[]).unwrap_err().code,
            "connector_teams_required"
        );
        // Blank fields (after trimming) are rejected.
        for invalid in [
            team(" ", "ENG", "Engineering"),
            team("team-1", "", "Engineering"),
            team("team-1", "ENG", "  "),
        ] {
            assert_eq!(
                validate_selected_teams(&[invalid]).unwrap_err().code,
                "connector_teams_invalid"
            );
        }
        // Dedupe by id keeps the first entry and its order; fields come back
        // trimmed.
        let cleaned = validate_selected_teams(&[
            team(" team-2 ", " DES ", " Design "),
            team("team-1", "ENG", "Engineering"),
            team("team-2", "DES2", "Design again"),
        ])
        .expect("valid payload");
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned[0].id, "team-2");
        assert_eq!(cleaned[0].key, "DES");
        assert_eq!(cleaned[0].name, "Design");
        assert_eq!(cleaned[1].id, "team-1");
    }

    #[test]
    fn trust_mode_validation() {
        assert!(validate_trust_mode("read_only").is_ok());
        assert!(validate_trust_mode("approval").is_ok());
        assert!(validate_trust_mode("autonomous").is_ok());
        assert_eq!(
            validate_trust_mode("yolo").unwrap_err().code,
            "routine_trust_invalid_mode"
        );
    }

    #[test]
    fn trigger_kind_validation() {
        assert!(validate_trigger_kind("email_received").is_ok());
        assert!(validate_trigger_kind("event_upcoming").is_ok());
        assert_eq!(
            validate_trigger_kind("webhook").unwrap_err().code,
            "connector_trigger_invalid_kind"
        );
    }

    #[test]
    fn autonomy_needs_three_approval_runs() {
        assert!(!autonomy_earned(0));
        assert!(!autonomy_earned(2));
        assert!(autonomy_earned(3));
        assert!(autonomy_earned(10));
    }

    #[test]
    fn maps_tools_to_providers() {
        assert_eq!(provider_for_tool("create_draft"), Some("gmail"));
        assert_eq!(provider_for_tool("send_email"), Some("gmail"));
        assert_eq!(provider_for_tool("modify_labels"), Some("gmail"));
        assert_eq!(provider_for_tool("archive"), Some("gmail"));
        assert_eq!(provider_for_tool("create_event"), Some("gcal"));
        assert_eq!(provider_for_tool("respond_to_invite"), Some("gcal"));
        // Read-only or unknown tools need no grant.
        assert_eq!(provider_for_tool("read_thread"), None);
        assert_eq!(provider_for_tool("list_events"), None);
        // GitHub actions never earn autonomy in v1 (ADR-0036).
        assert_eq!(provider_for_tool("create_issue"), None);
        assert_eq!(provider_for_tool("update_issue"), None);
        assert_eq!(provider_for_tool("add_comment"), None);
        assert_eq!(provider_for_tool("list_repositories"), None);
        assert_eq!(provider_for_tool("search_issues"), None);
    }

    #[test]
    fn google_identity_selection_ignores_synthetic_notion_account() {
        let notion = ConnectorAccount {
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
        let reconnecting_google = ConnectorAccount {
            account_id: "stale@example.com".to_string(),
            provider: ConnectorProvider::Google,
            email: "stale@example.com".to_string(),
            scopes: Vec::new(),
            status: ConnectorAccountStatus::ReconnectRequired,
            via_gh: false,
            workspace_name: None,
            workspace_url_key: None,
            selected_teams: Vec::new(),
        };
        let connected_google = ConnectorAccount {
            account_id: "user@example.com".to_string(),
            provider: ConnectorProvider::Google,
            email: "user@example.com".to_string(),
            scopes: Vec::new(),
            status: ConnectorAccountStatus::Connected,
            via_gh: false,
            workspace_name: None,
            workspace_url_key: None,
            selected_teams: Vec::new(),
        };

        assert_eq!(
            first_connected_google_account_email_from(vec![notion.clone()]),
            ""
        );
        assert_eq!(
            first_connected_google_account_email_from(vec![
                notion.clone(),
                reconnecting_google,
                connected_google,
            ]),
            "user@example.com"
        );
    }

    #[test]
    fn job_server_suffix_is_deterministic_and_collision_free() {
        // Deterministic for a given id.
        assert_eq!(job_server_suffix("job-1"), job_server_suffix("job-1"));
        // Distinct ids that share the first 8 sanitized chars still differ,
        // because the hash of the full id disambiguates them.
        let a = job_server_suffix("routine-abcdefgh-1111");
        let b = job_server_suffix("routine-abcdefgh-2222");
        assert_ne!(a, b);
        // And ids that sanitize to the same readable prefix differ too.
        assert_ne!(job_server_suffix("job-1"), job_server_suffix("job_1x"));
    }

    #[test]
    fn server_name_stays_a_safe_mcp_key() {
        for id in ["job-1", "Routine ABC 12345678 xyz", "!!!", "cron_job_42"] {
            let name = format!("june_gmail_auto_{}", job_server_suffix(id));
            assert!(name.starts_with("june_gmail_auto_"));
            assert!(name.contains("_auto_"));
            assert!(name.len() <= 64);
            assert!(name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.'));
        }
    }

    #[test]
    fn tools_match_is_order_independent() {
        assert!(tools_match(
            &["send_email".to_string(), "create_draft".to_string()],
            &["create_draft".to_string(), "send_email".to_string()],
        ));
        assert!(!tools_match(
            &["create_draft".to_string()],
            &["create_draft".to_string(), "send_email".to_string()],
        ));
    }
}
