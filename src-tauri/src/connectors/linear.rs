//! Linear native-app OAuth and the fixed GraphQL documents Clovy needs.
//!
//! Linear is a PKCE-only PUBLIC client: unlike Google's Desktop credential,
//! no client secret exists anywhere in this flow - the token endpoint marks
//! it optional for PKCE and Clovy never sends one (see
//! docs/plugins/linear-oauth-spike.md). The browser handoff itself is the
//! shared [`oauth::loopback_authorize`] primitive; this module owns Linear's
//! auth-URL shape (COMMA-joined scopes, explicit `actor=user` - v1
//! deliberately uses the user actor so grants inherit the authorizing user's
//! team visibility, never the app actor's all-teams view), the secretless
//! token exchange and refresh, revocation, and the fixed GraphQL documents:
//! slice 1's viewer/organization identity and Teams listing, plus slice 2's
//! nine read operations (users, projects, cycles, initiatives, issue search,
//! issue detail, issue comments, project updates) that back the `june_linear`
//! agent surface. Every team-scoped read operation here takes the caller's
//! granted team ids as a plain `&[String]` parameter (or, for single-entity
//! fetches, returns the entity's team id(s) for the caller to check) - this
//! module never touches `AppHandle` or the repository layer, so it stays
//! unit-testable without a database. The actual grant load and the
//! grant-check helpers live in `connectors::mod` (see
//! [`crate::connectors::linear_granted_team_ids`],
//! [`crate::connectors::linear_require_team_granted`],
//! [`crate::connectors::linear_require_any_team_granted`]).
//!
//! Refresh responses rotate the refresh token on every success; callers
//! persist the rotated token when present and keep the old one otherwise
//! (same logic as Google). Rate limiting arrives as HTTP 400 with a
//! `RATELIMITED` error code in the GraphQL errors array, not as HTTP 429.
//!
//! NEVER log, print, or serialize tokens (or authorization codes) into
//! errors. Error messages carry stable codes and short human text only.

use crate::domain::types::AppError;
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use super::oauth::{self, ConnectFlow};

const AUTH_ENDPOINT: &str = "https://linear.app/oauth/authorize";
const TOKEN_ENDPOINT: &str = "https://api.linear.app/oauth/token";
const REVOKE_ENDPOINT: &str = "https://api.linear.app/oauth/revoke";
const GRAPHQL_ENDPOINT: &str = "https://api.linear.app/graphql";
const API_ERROR_MESSAGE_MAX_LEN: usize = 200;
/// Teams pagination hard cap: 5 pages of 100 covers 500 teams, far beyond
/// any workspace Clovy plausibly serves; the cap bounds a pathological or
/// adversarial cursor loop.
const TEAMS_PAGE_SIZE: u32 = 100;
const TEAMS_MAX_PAGES: usize = 5;

/// Token endpoint response. Secret fields zeroize on drop.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct LinearTokenResponse {
    pub access_token: String,
    /// Present on the initial exchange and (rotated) on every refresh; may
    /// be absent on a scope escalation for an already-connected workspace,
    /// in which case callers keep the one in custody.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[zeroize(skip)]
    pub expires_in: i64,
    /// The granted scope set. Linear's separator is not contractual, so
    /// callers parse it with [`parse_scope_field`] (commas or whitespace).
    #[serde(default, deserialize_with = "deserialize_scope_response")]
    #[zeroize(skip)]
    pub scope: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LinearScopeResponse {
    String(String),
    Array(Vec<String>),
}

/// Normalize both token-response shapes Linear documents: current apps
/// receive one string, while apps created before 2023-12-01 receive an array.
/// Keeping the public field as a string preserves the existing scope parser
/// and every downstream caller.
fn deserialize_scope_response<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(
        Option::<LinearScopeResponse>::deserialize(deserializer)?.map(|scope| match scope {
            LinearScopeResponse::String(value) => value,
            LinearScopeResponse::Array(values) => values.join(" "),
        }),
    )
}

/// Split a token response `scope` field into individual scopes. Accepts
/// comma- and whitespace-separated forms (and mixtures) because Linear's
/// exact separator is not contractual; empty fragments are dropped.
pub fn parse_scope_field(raw: &str) -> Vec<String> {
    raw.split(|c: char| c.is_whitespace() || c == ',')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

/// Who authorized, and for which workspace. The workspace id keys the
/// keychain entry and the DB index row (NOT the email: a Linear account is a
/// workspace grant, and the email may legitimately be empty). Carries no
/// token material, so deriving `Debug` is safe.
#[derive(Debug, Clone)]
pub struct LinearIdentity {
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_url_key: String,
    pub user_id: String,
    pub user_name: String,
    /// Lowercased and trimmed; may be empty (kept as-is, never substituted).
    pub user_email: String,
}

/// Outcome of the full browser handoff: granted tokens plus the identity
/// resolved with the fresh access token.
pub struct LinearAuthorizedGrant {
    pub tokens: LinearTokenResponse,
    pub identity: LinearIdentity,
}

#[derive(Deserialize)]
struct TokenErrorBody {
    #[serde(default)]
    error: Option<String>,
}

/// Mirrors `oauth::RefreshOutcome`, which hard-codes the Google token type;
/// a Linear-local enum beats contorting the Google one.
pub enum LinearRefreshOutcome {
    Refreshed(LinearTokenResponse),
    /// Definitive: the grant was revoked or expired. The workspace must be
    /// reconnected; retrying cannot help.
    InvalidGrant,
    /// Upstream wobble (5xx, 429, network error): worth a bounded retry.
    Transient,
}

/// Run the full Linear authorization handoff: open the consent screen in the
/// default browser, wait on the loopback listener for the redirect, exchange
/// the code (no client secret), and resolve the workspace identity with the
/// fresh access token. A thin Linear-specific wrapper over
/// [`oauth::loopback_authorize`], parallel to the Google `oauth::authorize`.
pub async fn authorize(
    flow: &ConnectFlow,
    client_id: &str,
    scopes: &[&str],
    loopback_ports: &[u16],
) -> Result<LinearAuthorizedGrant, AppError> {
    // Fixed candidate ports, not an ephemeral one: Linear matches the
    // registered callback URL exactly (port included), so the redirect URI
    // must be one whose URL is registered on the OAuth application.
    let authorization = oauth::loopback_authorize(
        flow,
        "Linear",
        oauth::LoopbackPort::Candidates(loopback_ports.to_vec()),
        |redirect_uri, code_challenge, state| {
            build_auth_url(client_id, redirect_uri, scopes, code_challenge, state)
        },
    )
    .await?;
    flow.ensure_not_canceled("Linear")?;

    let tokens = exchange_code(
        client_id,
        &authorization.code,
        &authorization.verifier,
        &authorization.redirect_uri,
    )
    .await?;
    if let Err(canceled) = flow.ensure_not_canceled("Linear") {
        return if revoke_unpersisted_grant(&tokens).await {
            Err(canceled)
        } else {
            Err(unpersisted_grant_cleanup_failed())
        };
    }
    let identity = match fetch_identity(&tokens.access_token).await {
        Ok(identity) => identity,
        Err(error) => {
            return if revoke_unpersisted_grant(&tokens).await {
                Err(error)
            } else {
                Err(unpersisted_grant_cleanup_failed())
            };
        }
    };
    if let Err(canceled) = flow.ensure_not_canceled("Linear") {
        return if revoke_unpersisted_grant(&tokens).await {
            Err(canceled)
        } else {
            Err(unpersisted_grant_cleanup_failed())
        };
    }
    Ok(LinearAuthorizedGrant { tokens, identity })
}

pub(super) async fn revoke_unpersisted_grant(tokens: &LinearTokenResponse) -> bool {
    let mut confirmed = true;
    for (token, token_type_hint) in unpersisted_revocation_targets(tokens) {
        confirmed &= revoke(token, token_type_hint).await;
    }
    confirmed
}

pub(super) fn unpersisted_grant_cleanup_failed() -> AppError {
    AppError::new(
        "connector_connect_cleanup_unconfirmed",
        "Clovy stopped connecting, but could not confirm that Linear removed the new authorization. Remove Clovy in Linear's authorized applications settings before trying again.",
    )
}

fn unpersisted_revocation_targets(tokens: &LinearTokenResponse) -> Vec<(&str, &'static str)> {
    let mut targets = Vec::with_capacity(2);
    if let Some(refresh_token) = tokens
        .refresh_token
        .as_deref()
        .filter(|token| !token.is_empty())
    {
        targets.push((refresh_token, "refresh_token"));
    }
    if !tokens.access_token.is_empty() {
        targets.push((tokens.access_token.as_str(), "access_token"));
    }
    targets
}

fn build_auth_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    code_challenge: &str,
    state: &str,
) -> String {
    // Scopes are COMMA-joined (Linear's documented form, unlike Google's
    // spaces). `actor=user` is the default but stated explicitly: v1
    // deliberately authorizes as the user, never the app actor (see the
    // spike doc - app actor needs a secret and sees every public team).
    // `prompt=consent` forces the workspace picker on every connect: after
    // a non-revoking disconnect the old grant still exists at Linear, and
    // without the prompt a fresh connect would silently re-authorize the
    // previous workspace with no way to pick a different one.
    format!(
        "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &state={}&code_challenge={}&code_challenge_method=S256&actor=user&prompt=consent",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(&scopes.join(",")),
        urlencoding::encode(state),
        urlencoding::encode(code_challenge),
    )
}

/// Exchange the authorization code for tokens. PKCE only: the form carries
/// NO client_secret (public client), the verifier is the whole proof.
async fn exchange_code(
    client_id: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<LinearTokenResponse, AppError> {
    let response = oauth::http_client()
        .post(TOKEN_ENDPOINT)
        .form(&authorization_code_form(
            client_id,
            code,
            verifier,
            redirect_uri,
        ))
        .send()
        .await
        .map_err(|_| exchange_failed(None))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|_| exchange_failed(None))?;
    if let Ok(tokens) = serde_json::from_str::<LinearTokenResponse>(&body) {
        if !tokens.access_token.is_empty() {
            return Ok(tokens);
        }
    }
    // Never echo the body: it could carry partial token material. Surface
    // the OAuth error code word only.
    let error_code = serde_json::from_str::<TokenErrorBody>(&body)
        .ok()
        .and_then(|body| body.error);
    tracing::warn!(status, error_code = ?error_code, "linear token exchange failed");
    Err(exchange_failed(error_code))
}

fn authorization_code_form<'a>(
    client_id: &'a str,
    code: &'a str,
    verifier: &'a str,
    redirect_uri: &'a str,
) -> [(&'static str, &'a str); 5] {
    [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", client_id),
        ("code_verifier", verifier),
    ]
}

fn exchange_failed(error_code: Option<String>) -> AppError {
    let message = match error_code {
        Some(code) => format!("Could not complete the Linear connection ({code})."),
        None => "Could not complete the Linear connection.".to_string(),
    };
    AppError::new("connector_token_exchange_failed", message)
}

/// One refresh attempt. Classifies invalid_grant (definitive, the workspace
/// must be reconnected) apart from transient upstream wobble. On success the
/// response ALWAYS carries a rotated refresh token; the caller persists it.
pub async fn refresh(client_id: &str, refresh_token: &str) -> LinearRefreshOutcome {
    let response = match oauth::http_client()
        .post(TOKEN_ENDPOINT)
        .form(&refresh_form(client_id, refresh_token))
        .send()
        .await
    {
        Ok(response) => response,
        // No response at all: DNS, connection reset, timeout. Always transient.
        Err(_) => return LinearRefreshOutcome::Transient,
    };
    let status = response.status().as_u16();
    let body = match response.text().await {
        Ok(body) => body,
        Err(_) => return LinearRefreshOutcome::Transient,
    };
    if let Ok(tokens) = serde_json::from_str::<LinearTokenResponse>(&body) {
        if !tokens.access_token.is_empty() {
            return LinearRefreshOutcome::Refreshed(tokens);
        }
    }
    let error_code = serde_json::from_str::<TokenErrorBody>(&body)
        .ok()
        .and_then(|body| body.error);
    classify_refresh_failure(status, error_code.as_deref())
}

fn refresh_form<'a>(client_id: &'a str, refresh_token: &'a str) -> [(&'static str, &'a str); 3] {
    [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ]
}

/// `invalid_grant` is the definitive "grant revoked/expired" signal; anything
/// else (5xx, 429, even invalid_client) is treated as transient so a config
/// hiccup never flips a workspace into the reconnect state. Mirrors the
/// Google classification in oauth.rs.
fn classify_refresh_failure(status: u16, error_code: Option<&str>) -> LinearRefreshOutcome {
    // Log status + error code word only; never the body or tokens.
    tracing::warn!(status, error_code = ?error_code, "linear token refresh failed");
    match error_code {
        Some("invalid_grant") => LinearRefreshOutcome::InvalidGrant,
        _ => LinearRefreshOutcome::Transient,
    }
}

/// Best-effort revocation of the grant at Linear (used by
/// `disconnect(revoke_grant = true)`). Linear answers 200 on revocation and
/// 400 when the token was already revoked; both leave the grant dead, so
/// both count as success. Other failures are swallowed after logging the
/// HTTP status: local custody removal is the real disconnect.
pub async fn revoke(token: &str, token_type_hint: &str) -> bool {
    match oauth::http_client()
        .post(REVOKE_ENDPOINT)
        .form(&[("token", token), ("token_type_hint", token_type_hint)])
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let ok = response.status().is_success();
            if !ok {
                // 400 can mean "already revoked" but ALSO "could not revoke"
                // (e.g. an unidentifiable token), so it is never silently
                // success: log the OAuth error word (never the body) so a
                // grant surviving a revoking disconnect is diagnosable.
                let error_code = match response.text().await {
                    Ok(body) => serde_json::from_str::<TokenErrorBody>(&body)
                        .ok()
                        .and_then(|body| body.error),
                    Err(_) => None,
                };
                tracing::warn!(status, token_type_hint, error_code = ?error_code, "linear revoke did not confirm");
            }
            ok
        }
        Err(_) => {
            tracing::warn!(token_type_hint, "linear revoke request failed");
            false
        }
    }
}

// --- GraphQL -------------------------------------------------------------------

#[derive(Debug)]
pub enum LinearApiError {
    /// The access token was rejected. The caller should refresh and retry
    /// once before surfacing an error.
    Unauthorized,
    /// Linear's request/complexity budget is exhausted (HTTP 400 with the
    /// `RATELIMITED` error code, per the rate-limiting docs).
    RateLimited,
    Api {
        status: u16,
        message: String,
    },
    /// The response could not be interpreted at all (body did not parse, or
    /// a 2xx carried neither data nor errors). Distinct from [`Self::Api`]
    /// because it says nothing about whether a mutation applied: a mangled
    /// success is indistinguishable from a mangled failure, so the action
    /// dispatch treats it as AMBIGUOUS and reconciles by object UUID. A
    /// parsed provider rejection (GraphQL errors array, `success: false`
    /// payload) is never this variant - those are definitive.
    UnusableResponse {
        status: u16,
    },
    /// A successful HTTP response carried both mutation data and GraphQL
    /// errors. At least part of the mutation may have committed, so callers
    /// must reconcile by the client-minted object UUID before retrying.
    AmbiguousMutationResponse {
        status: u16,
    },
    Network(String),
}

impl From<LinearApiError> for AppError {
    fn from(error: LinearApiError) -> Self {
        match error {
            LinearApiError::Unauthorized => AppError::new(
                "linear_unauthorized",
                "Linear rejected the connection's access token.",
            ),
            LinearApiError::RateLimited => AppError::new(
                "linear_rate_limited",
                "Linear is rate limiting requests. Try again in a few minutes.",
            ),
            // A received 5xx can arrive after the backend committed (gateway
            // failure), so it maps to the ambiguous code and the action
            // dispatch reconciles by object UUID. Every other Api error is a
            // PARSED provider rejection (GraphQL errors array or a
            // success:false payload, which arrive at HTTP 200) - definitive:
            // the mutation never applied, and the message carries the
            // actionable reason.
            LinearApiError::Api { status, message } if status >= 500 => AppError::new(
                "linear_upstream_error",
                format!("Linear had a server error ({status}): {message}"),
            ),
            LinearApiError::Api { status, message } => AppError::new(
                "linear_api_error",
                format!("Linear API request failed ({status}): {message}"),
            ),
            // Unusable body: the response says nothing about the mutation's
            // fate, so it shares the ambiguous code with 5xx.
            LinearApiError::UnusableResponse { status } => AppError::new(
                "linear_upstream_error",
                format!("Linear returned an unusable response ({status})."),
            ),
            LinearApiError::AmbiguousMutationResponse { status } => AppError::new(
                "linear_upstream_error",
                format!("Linear returned a partial mutation response ({status})."),
            ),
            LinearApiError::Network(message) => AppError::new("network_error", message),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GraphqlOperation {
    Query,
    Mutation,
}

#[derive(Deserialize)]
struct GraphqlResponseWire<T> {
    // Missing `data` reads as None (serde treats Option fields as optional);
    // no `default` attribute, which would wrongly demand `T: Default`.
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphqlErrorWire>,
}

#[derive(Deserialize)]
struct GraphqlErrorWire {
    #[serde(default)]
    message: String,
    #[serde(default)]
    extensions: Option<GraphqlErrorExtensionsWire>,
}

#[derive(Deserialize)]
struct GraphqlErrorExtensionsWire {
    #[serde(default)]
    code: Option<String>,
    /// Linear also tags errors with a human category here (e.g.
    /// "authentication error"); carried because auth failures are not
    /// guaranteed to arrive as HTTP 401.
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// Shared GraphQL POST for every fixed document in this module. HTTP 401 maps
/// to `Unauthorized`; a
/// `RATELIMITED` code anywhere in the errors array maps to `RateLimited`;
/// any other GraphQL error surfaces the first message (bounded). Never logs
/// or echoes the access token.
async fn graphql<T: DeserializeOwned>(
    access_token: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<T, LinearApiError> {
    graphql_with_operation(access_token, query, variables, GraphqlOperation::Query).await
}

/// Mutation variant of [`graphql`]. A 2xx response containing both data and
/// errors is ambiguous because some mutation work may already have committed.
async fn graphql_mutation<T: DeserializeOwned>(
    access_token: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<T, LinearApiError> {
    graphql_with_operation(access_token, query, variables, GraphqlOperation::Mutation).await
}

async fn graphql_with_operation<T: DeserializeOwned>(
    access_token: &str,
    query: &str,
    variables: serde_json::Value,
    operation: GraphqlOperation,
) -> Result<T, LinearApiError> {
    let response = oauth::http_client()
        .post(GRAPHQL_ENDPOINT)
        .bearer_auth(access_token)
        .json(&serde_json::json!({ "query": query, "variables": variables }))
        .send()
        .await
        .map_err(|e| LinearApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| LinearApiError::Network(e.to_string()))?;
    if status == 401 {
        return Err(LinearApiError::Unauthorized);
    }
    let parsed: GraphqlResponseWire<T> =
        serde_json::from_str(&body).map_err(|_| LinearApiError::UnusableResponse { status })?;
    graphql_data(parsed, status, operation)
}

fn graphql_data<T>(
    parsed: GraphqlResponseWire<T>,
    status: u16,
    operation: GraphqlOperation,
) -> Result<T, LinearApiError> {
    if operation == GraphqlOperation::Mutation
        && (200..300).contains(&status)
        && parsed.data.is_some()
        && !parsed.errors.is_empty()
    {
        return Err(LinearApiError::AmbiguousMutationResponse { status });
    }
    check_graphql_errors(&parsed.errors, status)?;
    if !(200..300).contains(&status) {
        return Err(LinearApiError::Api {
            status,
            message: "request failed".to_string(),
        });
    }
    parsed
        .data
        .ok_or(LinearApiError::UnusableResponse { status })
}

/// Classify a GraphQL errors array: `RATELIMITED` (checked in both
/// `extensions.code` and the message, since the docs only guarantee the code
/// word appears) beats a generic API failure; the first error's message is
/// surfaced, bounded, never the raw body.
fn check_graphql_errors(errors: &[GraphqlErrorWire], status: u16) -> Result<(), LinearApiError> {
    if errors.is_empty() {
        return Ok(());
    }
    // Linear signals failures through GraphQL error codes rather than
    // semantic HTTP statuses (rate limiting arrives as HTTP 400), so a
    // revoked or invalid token cannot be assumed to arrive as HTTP 401
    // either: classify an authentication-flavored code or type as
    // Unauthorized so the callers' refresh-and-retry and the
    // reconnect_required transition still fire.
    let unauthorized = errors.iter().any(|error| {
        error.extensions.as_ref().is_some_and(|extensions| {
            extensions
                .code
                .as_deref()
                .is_some_and(|code| code.to_ascii_uppercase().contains("AUTHENTICATION"))
                || extensions
                    .kind
                    .as_deref()
                    .is_some_and(|kind| kind.to_ascii_lowercase().contains("authentication"))
        })
    });
    if unauthorized {
        return Err(LinearApiError::Unauthorized);
    }
    let rate_limited = errors.iter().any(|error| {
        error
            .extensions
            .as_ref()
            .and_then(|extensions| extensions.code.as_deref())
            == Some("RATELIMITED")
            || error.message.contains("RATELIMITED")
    });
    if rate_limited {
        return Err(LinearApiError::RateLimited);
    }
    let message = errors
        .first()
        .map(|error| {
            error
                .message
                .chars()
                .take(API_ERROR_MESSAGE_MAX_LEN)
                .collect()
        })
        .unwrap_or_else(|| "request failed".to_string());
    Err(LinearApiError::Api { status, message })
}

// --- Identity --------------------------------------------------------------------

const IDENTITY_QUERY: &str = "{ viewer { id name email } organization { id name urlKey } }";

#[derive(Deserialize)]
struct IdentityDataWire {
    #[serde(default)]
    viewer: Option<ViewerWire>,
    #[serde(default)]
    organization: Option<OrganizationWire>,
}

#[derive(Deserialize)]
struct ViewerWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    email: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrganizationWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    url_key: String,
}

fn identity_failed() -> AppError {
    AppError::new(
        "connector_identity_failed",
        "Could not determine the Linear workspace.",
    )
}

/// Resolve the workspace + user identity that keys custody and the DB index.
/// Any transport or API failure maps to the identity error (mirroring how
/// Google's email resolution reports): during a connect there is nothing
/// more actionable to say.
async fn fetch_identity(access_token: &str) -> Result<LinearIdentity, AppError> {
    let data: IdentityDataWire = graphql(access_token, IDENTITY_QUERY, serde_json::json!({}))
        .await
        .map_err(|_| identity_failed())?;
    identity_from_data(data)
}

/// Pure mapping from the identity document's data to [`LinearIdentity`].
/// Workspace and viewer ids are load-bearing (custody key, actor
/// attribution) and must be present; name/urlKey/email may be empty and are
/// carried as-is - the email is NOT substituted with the user name, because
/// the account is keyed by workspace id, not email.
fn identity_from_data(data: IdentityDataWire) -> Result<LinearIdentity, AppError> {
    let viewer = data.viewer.ok_or_else(identity_failed)?;
    let organization = data.organization.ok_or_else(identity_failed)?;
    if viewer.id.is_empty() || organization.id.is_empty() {
        return Err(identity_failed());
    }
    Ok(LinearIdentity {
        workspace_id: organization.id,
        workspace_name: organization.name,
        workspace_url_key: organization.url_key,
        user_id: viewer.id,
        user_name: viewer.name,
        user_email: viewer.email.trim().to_ascii_lowercase(),
    })
}

// --- Teams -----------------------------------------------------------------------

/// One team as offered to the selection UI. The GraphQL node shape (id, key,
/// name) deserializes directly.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct LinearTeam {
    pub id: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: String,
}

// Page size rides in as a variable so [`TEAMS_PAGE_SIZE`] is the single
// source of truth; a literal here would silently diverge from the constant
// (and its truncation-warn log) if the cap were ever tuned.
const TEAMS_QUERY: &str = "query Teams($after: String, $first: Int!) \
     { teams(first: $first, after: $after) \
     { nodes { id key name } pageInfo { hasNextPage endCursor } } }";

#[derive(Deserialize)]
struct TeamsDataWire {
    teams: TeamsPageWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamsPageWire {
    #[serde(default)]
    nodes: Vec<LinearTeam>,
    page_info: PageInfoWire,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfoWire {
    #[serde(default)]
    has_next_page: bool,
    #[serde(default)]
    end_cursor: Option<String>,
}

/// Pure page-fold for the Teams query, so the merge, the page cap, and the
/// final ordering are unit-testable without HTTP.
struct TeamsPager {
    teams: Vec<LinearTeam>,
    pages: usize,
    truncated: bool,
}

impl TeamsPager {
    fn new() -> Self {
        TeamsPager {
            teams: Vec::new(),
            pages: 0,
            truncated: false,
        }
    }

    /// Fold one page in. Returns the `after` cursor for the next request, or
    /// `None` when pagination is complete - either no further page exists or
    /// the hard cap was reached (recorded as truncation).
    fn absorb(&mut self, page: TeamsPageWire) -> Option<String> {
        self.pages += 1;
        self.teams.extend(page.nodes);
        if !page.page_info.has_next_page {
            return None;
        }
        if self.pages >= TEAMS_MAX_PAGES {
            self.truncated = true;
            return None;
        }
        page.page_info
            .end_cursor
            .filter(|cursor| !cursor.is_empty())
    }

    /// The accumulated teams sorted by name, case-insensitively (stable, so
    /// equal names keep their arrival order), plus whether the cap truncated
    /// the listing.
    fn finish(self) -> (Vec<LinearTeam>, bool) {
        let mut teams = self.teams;
        teams.sort_by_key(|team| team.name.to_lowercase());
        (teams, self.truncated)
    }
}

/// A teams listing plus whether the pagination cap cut it short. The flag
/// travels to the UI: a silently incomplete list would present itself as
/// the complete team inventory in a very large workspace.
pub struct LinearTeamsListing {
    pub teams: Vec<LinearTeam>,
    pub truncated: bool,
}

/// List the workspace's teams for the selection UI: pages of 100, hard cap
/// [`TEAMS_MAX_PAGES`] pages (a warn is logged and the listing is flagged if
/// the cap truncates), sorted by name case-insensitively.
pub async fn list_teams(access_token: &str) -> Result<LinearTeamsListing, LinearApiError> {
    let mut pager = TeamsPager::new();
    let mut after: Option<String> = None;
    loop {
        let data: TeamsDataWire = graphql(
            access_token,
            TEAMS_QUERY,
            serde_json::json!({ "after": after, "first": TEAMS_PAGE_SIZE }),
        )
        .await?;
        after = pager.absorb(data.teams);
        if after.is_none() {
            break;
        }
    }
    let (teams, truncated) = pager.finish();
    if truncated {
        tracing::warn!(
            page_cap = TEAMS_MAX_PAGES,
            page_size = TEAMS_PAGE_SIZE,
            "linear teams listing truncated at the pagination cap"
        );
    }
    Ok(LinearTeamsListing { teams, truncated })
}

// --- Slice 2: read operations -----------------------------------------------------

// Page sizes below that are NOT exposed as caller-configurable parameters
// (nested collections such as "teams a project belongs to", or "labels on
// one issue") are embedded directly as literals in their query constant
// rather than routed through a named constant + GraphQL variable the way
// [`TEAMS_PAGE_SIZE`] is above: each such literal appears in exactly one
// query string, so there is nothing for it to drift from.
/// Per-team member page cap for [`list_users`]. Applied to EACH selected
/// team's `members` connection, then the unioned set is deduped by id, so a
/// larger workspace is bounded per team rather than globally.
const USERS_PAGE_DEFAULT: u32 = 100;
const USERS_PAGE_MAX: u32 = 250;
/// A single bounded page, server-filtered to the granted teams (see
/// [`list_projects`]). Not caller-configurable - directory data stays a
/// fixed shape, matching [`list_teams`].
const PROJECTS_PAGE_SIZE: u32 = 100;
/// One team's cycles: fixed page, not caller-configurable.
const CYCLES_PAGE_SIZE: u32 = 25;
/// All workspace initiatives: fixed page, not caller-configurable.
const INITIATIVES_PAGE_SIZE: u32 = 50;
const SEARCH_ISSUES_DEFAULT: u32 = 25;
const SEARCH_ISSUES_MAX: u32 = 50;
const COMMENTS_DEFAULT: u32 = 25;
const COMMENTS_MAX: u32 = 50;
const PROJECT_UPDATES_DEFAULT: u32 = 10;
const PROJECT_UPDATES_MAX: u32 = 25;

/// Character bounds for free-text fields entering agent context, so a single
/// issue/comment/update can never dominate the context window.
const ISSUE_DESCRIPTION_MAX_CHARS: usize = 4000;
const COMMENT_BODY_MAX_CHARS: usize = 2000;
const PROJECT_UPDATE_BODY_MAX_CHARS: usize = 2000;

/// Clamp a caller-supplied page size to `[min, max]`, defaulting when absent.
/// Every bounded read op below runs its `first` argument through this so a
/// caller can never request an unbounded (or negative-size) page.
fn clamp_first(value: Option<u32>, default: u32, min: u32, max: u32) -> u32 {
    value.unwrap_or(default).clamp(min, max)
}

const TRUNCATION_SUFFIX: &str = " ... [truncated]";

/// Bound `text` to at most `max` chars, appending [`TRUNCATION_SUFFIX`] only
/// when truncation actually removes content. Operates on chars, not bytes,
/// so the cut always lands on a char boundary - safe for the multi-byte text
/// (emoji, non-Latin scripts, hostile Unicode) that Linear issue/comment
/// content may carry.
fn truncate_bounded(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max).collect();
    format!("{truncated}{TRUNCATION_SUFFIX}")
}

/// Linear's numeric fields used here (`priority`, cycle `number`) are
/// GraphQL Floats that are always whole-valued in practice; round rather
/// than truncate so a hypothetical off-by-epsilon float never quietly
/// reports the wrong value.
fn round_to_i64(value: f64) -> i64 {
    value.round() as i64
}

/// True when at least one id in `team_ids` is present in `granted_team_ids`.
/// The pure predicate behind [`list_initiatives`]'s client-side project-list
/// narrowing (and reusable for any other "does this multi-team entity touch
/// the grant" check).
fn any_team_granted(team_ids: &[String], granted_team_ids: &[String]) -> bool {
    team_ids
        .iter()
        .any(|id| granted_team_ids.iter().any(|granted| granted == id))
}

// Shared minimal wire shapes reused across several read operations below.

#[derive(Deserialize)]
struct NameOnlyWire {
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct IdOnlyWire {
    #[serde(default)]
    id: String,
}

#[derive(Deserialize)]
struct KeyOnlyWire {
    #[serde(default)]
    key: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IdListPageWire {
    #[serde(default)]
    nodes: Vec<IdOnlyWire>,
    #[serde(default)]
    page_info: PageInfoWire,
}

fn ids_from_page(page: Option<IdListPageWire>) -> Vec<String> {
    page.map(|page| page.nodes.into_iter().map(|node| node.id).collect())
        .unwrap_or_default()
}

// --- Users -------------------------------------------------------------------------

/// One member of the account's selected teams, as surfaced to the agent.
/// Scoped to the selected-team boundary (not the workspace directory), and
/// deliberately carries NO email field - the agent has no need for teammate
/// emails and they are personal data.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearUser {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearUsersListing {
    pub users: Vec<LinearUser>,
    pub truncated: bool,
}

// `UserFilter` has no team-membership field, so users are scoped by walking
// the selected teams' `members` connections instead of filtering the
// workspace-wide `users` root query. `Query.teams(filter: { id: { in } })`
// selects exactly the granted teams; each `Team.members(first:)` yields that
// team's members; the results are unioned and deduped by id.
// The outer `teams` connection carries an explicit `$teamLimit` (the number
// of granted teams) so it never relies on Linear's implicit page default: an
// account that selected more teams than that default would otherwise silently
// get members from only the first page of granted teams. `$first` caps each
// team's member page.
const LIST_USERS_QUERY: &str =
    "query TeamMembers($teamIds: [ID!]!, $teamLimit: Int!, $first: Int!) \
     { teams(filter: { id: { in: $teamIds } }, first: $teamLimit) \
     { nodes { members(first: $first) { nodes { id name displayName active } \
     pageInfo { hasNextPage } } } pageInfo { hasNextPage } } }";

#[derive(Deserialize)]
struct TeamMembersDataWire {
    teams: TeamMembersTeamsWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamMembersTeamsWire {
    #[serde(default)]
    nodes: Vec<TeamMembersNodeWire>,
    #[serde(default)]
    page_info: PageInfoWire,
}

#[derive(Deserialize)]
struct TeamMembersNodeWire {
    #[serde(default)]
    members: Option<UsersPageWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UsersPageWire {
    #[serde(default)]
    nodes: Vec<UserWire>,
    #[serde(default)]
    page_info: PageInfoWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    display_name: String,
    #[serde(default)]
    active: bool,
}

/// Union the members across the selected teams, deduping by id (a user on
/// two selected teams appears once) and sorting by name case-insensitively.
/// The pure fold behind [`list_users`], so the union/dedupe/sort is testable
/// without HTTP.
fn union_team_members(teams: Vec<TeamMembersNodeWire>) -> Vec<LinearUser> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut users: Vec<LinearUser> = Vec::new();
    for team in teams {
        let Some(members) = team.members else {
            continue;
        };
        for user in members.nodes {
            if seen.insert(user.id.clone()) {
                users.push(LinearUser {
                    id: user.id,
                    name: user.name,
                    display_name: user.display_name,
                    active: user.active,
                });
            }
        }
    }
    users.sort_by_key(|user| user.name.to_lowercase());
    users
}

fn team_members_page_truncated(teams: &TeamMembersTeamsWire) -> bool {
    teams.page_info.has_next_page
        || teams.nodes.iter().any(|team| {
            team.members
                .as_ref()
                .is_some_and(|page| page.page_info.has_next_page)
        })
}

/// Members of the account's SELECTED teams (not the workspace directory):
/// one bounded page of members per granted team, unioned and deduped by id.
/// `first` caps each team's member page (1..=[`USERS_PAGE_MAX`], default
/// [`USERS_PAGE_DEFAULT`]). An empty grant returns an empty list without a
/// request - the same defensive empty-in-list guard as [`list_projects`],
/// since `id: { in: [] }` could otherwise be read as "no constraint".
pub async fn list_users(
    access_token: &str,
    granted_team_ids: &[String],
    first: Option<u32>,
) -> Result<LinearUsersListing, LinearApiError> {
    if granted_team_ids.is_empty() {
        return Ok(LinearUsersListing {
            users: Vec::new(),
            truncated: false,
        });
    }
    let limit = clamp_first(first, USERS_PAGE_DEFAULT, 1, USERS_PAGE_MAX);
    // The outer connection must return every granted team, so its page size is
    // the grant size itself - never a fixed default that could drop teams.
    let team_limit = granted_team_ids.len() as u32;
    let data: TeamMembersDataWire = graphql(
        access_token,
        LIST_USERS_QUERY,
        serde_json::json!({
            "teamIds": granted_team_ids,
            "teamLimit": team_limit,
            "first": limit,
        }),
    )
    .await?;
    let truncated = team_members_page_truncated(&data.teams);
    Ok(LinearUsersListing {
        users: union_team_members(data.teams.nodes),
        truncated,
    })
}

// --- Projects ------------------------------------------------------------------------

/// One project as surfaced to the agent, already scoped to the grant: only
/// projects linked to at least one selected team are ever constructed (spec
/// decision 3). `state` is `Project.status.name` - the schema's own
/// `Project.state` field is deprecated in favor of `status`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearProject {
    pub id: String,
    pub name: String,
    pub state: String,
    pub target_date: Option<String>,
    pub team_ids: Vec<String>,
    pub url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearProjectsListing {
    pub projects: Vec<LinearProject>,
    pub truncated: bool,
}

const LIST_PROJECTS_QUERY: &str = "query Projects($filter: ProjectFilter, $first: Int!) \
     { projects(filter: $filter, first: $first) \
     { nodes { id name status { name } targetDate teams(first: 10) \
     { nodes { id } pageInfo { hasNextPage } } url } pageInfo { hasNextPage } } }";

#[derive(Deserialize)]
struct ProjectsDataWire {
    projects: ProjectsPageWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectsPageWire {
    #[serde(default)]
    nodes: Vec<ProjectWire>,
    #[serde(default)]
    page_info: PageInfoWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    status: Option<NameOnlyWire>,
    #[serde(default)]
    target_date: Option<String>,
    #[serde(default)]
    teams: Option<IdListPageWire>,
    #[serde(default)]
    url: String,
}

fn project_from_wire(wire: ProjectWire) -> LinearProject {
    LinearProject {
        id: wire.id,
        name: wire.name,
        state: wire.status.map(|status| status.name).unwrap_or_default(),
        target_date: wire.target_date,
        team_ids: ids_from_page(wire.teams),
        url: wire.url,
    }
}

fn projects_page_truncated(projects: &ProjectsPageWire) -> bool {
    projects.page_info.has_next_page
        || projects.nodes.iter().any(|project| {
            project
                .teams
                .as_ref()
                .is_some_and(|page| page.page_info.has_next_page)
        })
}

/// Projects linked to at least one granted team, one bounded page. Schema
/// note: `ProjectFilter.accessibleTeams` (a `TeamCollectionFilter`) supports
/// exactly this via `some: { id: { in: $teamIds } } }`, so the grant is
/// enforced SERVER SIDE here rather than via the client-side page-and-filter
/// fallback the chunk spec allowed for - simpler, and it cannot leak an
/// unfiltered page if a pagination loop were ever cut short.
pub async fn list_projects(
    access_token: &str,
    granted_team_ids: &[String],
) -> Result<LinearProjectsListing, LinearApiError> {
    // `granted_team_ids` backs a GraphQL `id: { in: [...] } }` comparator. An
    // empty `in` list must never reach the server: some GraphQL backends
    // treat an empty in-list as "no constraint" rather than "matches
    // nothing", which here would silently return every team's projects -
    // exactly the leak the selected-team grant exists to prevent. Callers
    // are contracted to never call this with an empty grant (loading the
    // grant fails closed first, in `connectors::linear_granted_team_ids`);
    // this is the defense-in-depth backstop.
    if granted_team_ids.is_empty() {
        return Ok(LinearProjectsListing {
            projects: Vec::new(),
            truncated: false,
        });
    }
    let filter = serde_json::json!({
        "accessibleTeams": { "some": { "id": { "in": granted_team_ids } } }
    });
    let data: ProjectsDataWire = graphql(
        access_token,
        LIST_PROJECTS_QUERY,
        serde_json::json!({ "filter": filter, "first": PROJECTS_PAGE_SIZE }),
    )
    .await?;
    let truncated = projects_page_truncated(&data.projects);
    Ok(LinearProjectsListing {
        projects: data
            .projects
            .nodes
            .into_iter()
            .map(project_from_wire)
            .collect(),
        truncated,
    })
}

// --- Cycles --------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearCycle {
    pub id: String,
    pub number: i64,
    pub name: Option<String>,
    pub starts_at: String,
    pub ends_at: String,
    pub completed_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearCyclesListing {
    pub cycles: Vec<LinearCycle>,
    pub truncated: bool,
}

const LIST_CYCLES_QUERY: &str = "query TeamCycles($teamId: String!, $first: Int!) \
     { team(id: $teamId) { cycles(first: $first) \
     { nodes { id number name startsAt endsAt completedAt } \
     pageInfo { hasNextPage } } } }";

#[derive(Deserialize)]
struct TeamCyclesDataWire {
    #[serde(default)]
    team: Option<TeamCyclesWire>,
}

#[derive(Deserialize)]
struct TeamCyclesWire {
    cycles: CyclesPageWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CyclesPageWire {
    #[serde(default)]
    nodes: Vec<CycleWire>,
    #[serde(default)]
    page_info: PageInfoWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CycleWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    number: f64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    starts_at: String,
    #[serde(default)]
    ends_at: String,
    #[serde(default)]
    completed_at: Option<String>,
}

fn team_not_found() -> LinearApiError {
    LinearApiError::Api {
        status: 200,
        message: "team not found".to_string(),
    }
}

/// A team's cycles, one bounded page. Grant validation ("is `team_id` one of
/// the account's selected teams") happens in the CALLER
/// ([`crate::connectors::linear_require_team_granted`]) - this function only
/// fetches, and accepts any team id the access token can see, so it stays
/// pure and unit-testable without a grant to construct.
pub async fn list_cycles(
    access_token: &str,
    team_id: &str,
) -> Result<LinearCyclesListing, LinearApiError> {
    let data: TeamCyclesDataWire = graphql(
        access_token,
        LIST_CYCLES_QUERY,
        serde_json::json!({ "teamId": team_id, "first": CYCLES_PAGE_SIZE }),
    )
    .await?;
    let team = data.team.ok_or_else(team_not_found)?;
    Ok(LinearCyclesListing {
        truncated: team.cycles.page_info.has_next_page,
        cycles: team
            .cycles
            .nodes
            .into_iter()
            .map(|cycle| LinearCycle {
                id: cycle.id,
                number: round_to_i64(cycle.number),
                name: cycle.name,
                starts_at: cycle.starts_at,
                ends_at: cycle.ends_at,
                completed_at: cycle.completed_at,
            })
            .collect(),
    })
}

// --- Initiatives ---------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitiativeProjectRef {
    pub id: String,
    pub name: String,
}

/// An initiative that touches the selected-team boundary. Each initiative's
/// PROJECT list is narrowed to the grant client side, and an initiative
/// whose narrowed list is empty (no project on any selected team) is dropped
/// entirely: it lies wholly outside the boundary. Reverses the earlier
/// workspace-level-directory treatment (slice-2 decisions 3-4).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearInitiative {
    pub id: String,
    pub name: String,
    pub target_date: Option<String>,
    pub status: Option<String>,
    pub projects: Vec<InitiativeProjectRef>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearInitiativesListing {
    pub initiatives: Vec<LinearInitiative>,
    pub truncated: bool,
}

const LIST_INITIATIVES_QUERY: &str = "query Initiatives($first: Int!) \
     { initiatives(first: $first) { nodes { id name targetDate status \
     projects(first: 25) { nodes { id name teams(first: 10) \
     { nodes { id } pageInfo { hasNextPage } } } pageInfo { hasNextPage } } } \
     pageInfo { hasNextPage } } }";

#[derive(Deserialize)]
struct InitiativesDataWire {
    initiatives: InitiativesPageWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitiativesPageWire {
    #[serde(default)]
    nodes: Vec<InitiativeWire>,
    #[serde(default)]
    page_info: PageInfoWire,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitiativeWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    target_date: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    projects: Option<InitiativeProjectsPageWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InitiativeProjectsPageWire {
    #[serde(default)]
    nodes: Vec<InitiativeProjectWire>,
    #[serde(default)]
    page_info: PageInfoWire,
}

#[derive(Deserialize)]
struct InitiativeProjectWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    teams: Option<IdListPageWire>,
}

/// Narrow one initiative's project list to the grant, returning `None` when
/// nothing survives - an initiative with no project on any selected team is
/// wholly outside the boundary and is dropped, not returned empty.
fn initiative_from_wire(
    wire: InitiativeWire,
    granted_team_ids: &[String],
) -> Option<LinearInitiative> {
    let projects: Vec<InitiativeProjectRef> = wire
        .projects
        .map(|page| page.nodes)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|project| {
            let team_ids = ids_from_page(project.teams);
            any_team_granted(&team_ids, granted_team_ids).then_some(InitiativeProjectRef {
                id: project.id,
                name: project.name,
            })
        })
        .collect();
    if projects.is_empty() {
        return None;
    }
    Some(LinearInitiative {
        id: wire.id,
        name: wire.name,
        target_date: wire.target_date,
        status: wire.status,
        projects,
    })
}

fn initiatives_page_truncated(initiatives: &InitiativesPageWire) -> bool {
    initiatives.page_info.has_next_page
        || initiatives.nodes.iter().any(|initiative| {
            initiative.projects.as_ref().is_some_and(|projects| {
                projects.page_info.has_next_page
                    || projects.nodes.iter().any(|project| {
                        project
                            .teams
                            .as_ref()
                            .is_some_and(|teams| teams.page_info.has_next_page)
                    })
            })
        })
}

/// Initiatives that touch the selected-team boundary, one bounded page. Each
/// initiative's project list is narrowed to `granted_team_ids` client side,
/// and an initiative with no granted project is dropped entirely - only
/// initiatives that retain at least one granted project are returned, still
/// showing just those projects (reverses slice-2 decision 3).
pub async fn list_initiatives(
    access_token: &str,
    granted_team_ids: &[String],
) -> Result<LinearInitiativesListing, LinearApiError> {
    let data: InitiativesDataWire = graphql(
        access_token,
        LIST_INITIATIVES_QUERY,
        serde_json::json!({ "first": INITIATIVES_PAGE_SIZE }),
    )
    .await?;
    let truncated = initiatives_page_truncated(&data.initiatives);
    Ok(LinearInitiativesListing {
        initiatives: data
            .initiatives
            .nodes
            .into_iter()
            .filter_map(|initiative| initiative_from_wire(initiative, granted_team_ids))
            .collect(),
        truncated,
    })
}

// --- Issue search --------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueSearchParams {
    #[serde(default)]
    pub query: Option<String>,
    /// Team ids to constrain the search to. ALREADY narrowed to the
    /// account's grant by the caller - never empty by contract (see
    /// [`search_issues`]'s defensive guard, which refuses an empty list
    /// rather than trusting the contract blindly).
    pub team_ids: Vec<String>,
    #[serde(default)]
    pub state_type: Option<String>,
    #[serde(default)]
    pub assignee_id: Option<String>,
    #[serde(default)]
    pub first: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueSummary {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub state_name: String,
    pub state_type: String,
    pub priority: i64,
    pub assignee_name: Option<String>,
    pub team_key: String,
    pub updated_at: String,
    pub url: String,
}

const SEARCH_ISSUES_QUERY: &str = "query SearchIssues($filter: IssueFilter, $first: Int!) \
     { issues(filter: $filter, first: $first, orderBy: updatedAt) \
     { nodes { id identifier title state { name type } priority assignee { name } \
     team { key } updatedAt url } } }";

#[derive(Deserialize)]
struct IssuesDataWire {
    issues: IssuesPageWire,
}

#[derive(Deserialize)]
struct IssuesPageWire {
    #[serde(default)]
    nodes: Vec<IssueSummaryWire>,
}

#[derive(Deserialize)]
struct StateWire {
    #[serde(default)]
    name: String,
    #[serde(default, rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueSummaryWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    identifier: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    state: Option<StateWire>,
    #[serde(default)]
    priority: f64,
    #[serde(default)]
    assignee: Option<NameOnlyWire>,
    #[serde(default)]
    team: Option<KeyOnlyWire>,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    url: String,
}

fn issue_summary_from_wire(wire: IssueSummaryWire) -> LinearIssueSummary {
    LinearIssueSummary {
        id: wire.id,
        identifier: wire.identifier,
        title: wire.title,
        state_name: wire
            .state
            .as_ref()
            .map(|state| state.name.clone())
            .unwrap_or_default(),
        state_type: wire.state.map(|state| state.kind).unwrap_or_default(),
        priority: round_to_i64(wire.priority),
        assignee_name: wire
            .assignee
            .map(|assignee| assignee.name)
            .filter(|name| !name.is_empty()),
        team_key: wire.team.map(|team| team.key).unwrap_or_default(),
        updated_at: wire.updated_at,
        url: wire.url,
    }
}

/// Build the `IssueFilter` JSON for [`search_issues`]: the team scope is
/// ALWAYS present (`team.id.in`); state/assignee/text narrow it further when
/// supplied. Text search note: `IssueFilter` exposes no dedicated
/// full-text-search field reachable from this bounded, non-`searchIssues`
/// query path (`searchableContent` exists but is marked `[Internal]` in the
/// schema - not something to build a stable public tool contract on), so the
/// text query matches issue TITLE only, via `title.containsIgnoreCase`.
/// Callers should not expect the query to match descriptions or comments.
fn issue_search_filter(params: &IssueSearchParams) -> serde_json::Value {
    let mut filter = serde_json::json!({ "team": { "id": { "in": params.team_ids } } });
    if let Some(state_type) = params.state_type.as_deref().filter(|v| !v.is_empty()) {
        filter["state"] = serde_json::json!({ "type": { "eq": state_type } });
    }
    if let Some(assignee_id) = params.assignee_id.as_deref().filter(|v| !v.is_empty()) {
        filter["assignee"] = serde_json::json!({ "id": { "eq": assignee_id } });
    }
    if let Some(query) = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        filter["title"] = serde_json::json!({ "containsIgnoreCase": query });
    }
    filter
}

/// Search issues within the granted team scope. `params.team_ids` is a
/// security boundary (see [`IssueSearchParams::team_ids`]); an empty list is
/// refused rather than sent to Linear as `id: { in: [] }`, because some
/// GraphQL backends treat an empty in-list as "no constraint" - the same
/// defense-in-depth backstop as [`list_projects`].
pub async fn search_issues(
    access_token: &str,
    params: &IssueSearchParams,
) -> Result<Vec<LinearIssueSummary>, LinearApiError> {
    if params.team_ids.is_empty() {
        return Err(LinearApiError::Api {
            status: 400,
            message: "search_issues requires at least one granted team id".to_string(),
        });
    }
    let limit = clamp_first(params.first, SEARCH_ISSUES_DEFAULT, 1, SEARCH_ISSUES_MAX);
    let filter = issue_search_filter(params);
    let data: IssuesDataWire = graphql(
        access_token,
        SEARCH_ISSUES_QUERY,
        serde_json::json!({ "filter": filter, "first": limit }),
    )
    .await?;
    Ok(data
        .issues
        .nodes
        .into_iter()
        .map(issue_summary_from_wire)
        .collect())
}

// --- Issue detail --------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueDetail {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: Option<String>,
    pub state_name: String,
    pub state_type: String,
    pub priority: i64,
    pub assignee_name: Option<String>,
    pub team_id: String,
    pub team_key: String,
    pub label_names: Vec<String>,
    pub url: String,
    pub created_at: String,
    pub updated_at: String,
}

// `issue(id:)` is documented in the schema only as "looked up by its unique
// identifier" - the description does not spell out whether that accepts the
// human-readable identifier (e.g. "ENG-123") alongside the UUID. Linear's
// public API docs describe both forms as accepted; this passes whatever
// string the caller supplies through unchanged rather than second-guessing
// its shape. If that assumption is ever wrong, only UUID lookups succeed:
// Linear returns a GraphQL "not found" error for an unresolvable id, which
// surfaces as `LinearApiError::Api` (never a silent wrong-issue result).
const GET_ISSUE_QUERY: &str = "query GetIssue($id: String!) \
     { issue(id: $id) { id identifier title description state { name type } priority \
     assignee { name } team { id key } labels(first: 20) { nodes { name } } \
     url createdAt updatedAt } }";

#[derive(Deserialize)]
struct GetIssueDataWire {
    #[serde(default)]
    issue: Option<IssueDetailWire>,
}

#[derive(Deserialize)]
struct TeamIdKeyWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    key: String,
}

#[derive(Deserialize)]
struct LabelsPageWire {
    #[serde(default)]
    nodes: Vec<NameOnlyWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct IssueDetailWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    identifier: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    state: Option<StateWire>,
    #[serde(default)]
    priority: f64,
    #[serde(default)]
    assignee: Option<NameOnlyWire>,
    #[serde(default)]
    team: Option<TeamIdKeyWire>,
    #[serde(default)]
    labels: Option<LabelsPageWire>,
    #[serde(default)]
    url: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
}

fn issue_not_found() -> LinearApiError {
    LinearApiError::Api {
        status: 200,
        message: "issue not found".to_string(),
    }
}

fn issue_detail_from_wire(wire: IssueDetailWire) -> LinearIssueDetail {
    LinearIssueDetail {
        id: wire.id,
        identifier: wire.identifier,
        title: wire.title,
        description: wire
            .description
            .map(|text| truncate_bounded(&text, ISSUE_DESCRIPTION_MAX_CHARS)),
        state_name: wire
            .state
            .as_ref()
            .map(|state| state.name.clone())
            .unwrap_or_default(),
        state_type: wire.state.map(|state| state.kind).unwrap_or_default(),
        priority: round_to_i64(wire.priority),
        assignee_name: wire
            .assignee
            .map(|assignee| assignee.name)
            .filter(|name| !name.is_empty()),
        team_id: wire
            .team
            .as_ref()
            .map(|team| team.id.clone())
            .unwrap_or_default(),
        team_key: wire.team.map(|team| team.key).unwrap_or_default(),
        label_names: wire
            .labels
            .map(|page| page.nodes.into_iter().map(|node| node.name).collect())
            .unwrap_or_default(),
        url: wire.url,
        created_at: wire.created_at,
        updated_at: wire.updated_at,
    }
}

/// Fetch one issue's full detail, including its team id. Grant enforcement
/// ("is this issue's team one the account selected") happens AFTER this
/// call, in the caller: it discards the returned value entirely rather than
/// surfacing it when [`crate::connectors::linear_require_any_team_granted`]
/// rejects `team_id` - never a partial return.
pub async fn get_issue(
    access_token: &str,
    issue_id: &str,
) -> Result<LinearIssueDetail, LinearApiError> {
    let data: GetIssueDataWire = graphql(
        access_token,
        GET_ISSUE_QUERY,
        serde_json::json!({ "id": issue_id }),
    )
    .await?;
    let issue = data.issue.ok_or_else(issue_not_found)?;
    Ok(issue_detail_from_wire(issue))
}

// --- Issue comments ------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearComment {
    pub id: String,
    pub author_name: Option<String>,
    pub body: String,
    pub created_at: String,
    pub url: String,
}

const LIST_ISSUE_COMMENTS_QUERY: &str = "query IssueComments($id: String!, $first: Int!) \
     { issue(id: $id) { team { id } comments(first: $first) \
     { nodes { id user { name } botActor { name } body createdAt url } } } }";

#[derive(Deserialize)]
struct IssueCommentsDataWire {
    #[serde(default)]
    issue: Option<IssueCommentsIssueWire>,
}

#[derive(Deserialize)]
struct IssueCommentsIssueWire {
    #[serde(default)]
    team: Option<IdOnlyWire>,
    #[serde(default)]
    comments: Option<CommentsPageWire>,
}

#[derive(Deserialize)]
struct CommentsPageWire {
    #[serde(default)]
    nodes: Vec<CommentWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    user: Option<NameOnlyWire>,
    #[serde(default)]
    bot_actor: Option<NameOnlyWire>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    url: String,
}

/// A comment's author: the human user when present, otherwise the
/// integration/automation bot actor (Linear leaves `user` null for
/// integration-authored comments - see `Comment.user`/`Comment.botActor` in
/// the schema). `None` only when neither is populated.
fn comment_author_name(wire: &CommentWire) -> Option<String> {
    wire.user
        .as_ref()
        .map(|user| user.name.as_str())
        .or_else(|| wire.bot_actor.as_ref().map(|bot| bot.name.as_str()))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
}

fn comment_from_wire(wire: CommentWire) -> LinearComment {
    let author_name = comment_author_name(&wire);
    LinearComment {
        id: wire.id,
        author_name,
        body: truncate_bounded(&wire.body, COMMENT_BODY_MAX_CHARS),
        created_at: wire.created_at,
        url: wire.url,
    }
}

/// Fetch one issue's comments plus the issue's team id, one bounded page.
/// Grant enforcement happens AFTER this call, in the caller, exactly like
/// [`get_issue`] - the returned team id is what the caller checks, and the
/// whole result is discarded on a mismatch.
pub async fn list_issue_comments(
    access_token: &str,
    issue_id: &str,
    first: Option<u32>,
) -> Result<(String, Vec<LinearComment>), LinearApiError> {
    let limit = clamp_first(first, COMMENTS_DEFAULT, 1, COMMENTS_MAX);
    let data: IssueCommentsDataWire = graphql(
        access_token,
        LIST_ISSUE_COMMENTS_QUERY,
        serde_json::json!({ "id": issue_id, "first": limit }),
    )
    .await?;
    let issue = data.issue.ok_or_else(issue_not_found)?;
    let team_id = issue.team.map(|team| team.id).unwrap_or_default();
    let comments = issue
        .comments
        .map(|page| page.nodes)
        .unwrap_or_default()
        .into_iter()
        .map(comment_from_wire)
        .collect();
    Ok((team_id, comments))
}

// --- Project updates -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearProjectUpdate {
    pub id: String,
    pub author_name: Option<String>,
    pub body: String,
    pub health: Option<String>,
    pub created_at: String,
    pub url: String,
}

// The project's teams gate the post-fetch grant check, so this cap is a
// grant boundary, not a display limit: it must be high enough that a granted
// team never sits past it and gets a project wrongly DENIED. 50 covers any
// realistic project (fail-closed direction, so the residual risk is only a
// wrongly-denied read of a project spanning 50+ teams, never a leak).
const LIST_PROJECT_UPDATES_QUERY: &str = "query ProjectUpdates($id: String!, $first: Int!) \
     { project(id: $id) { teams(first: 50) { nodes { id } } \
     projectUpdates(first: $first) { nodes { id user { name } body health createdAt url } } } }";

#[derive(Deserialize)]
struct ProjectUpdatesDataWire {
    #[serde(default)]
    project: Option<ProjectUpdatesProjectWire>,
}

#[derive(Deserialize)]
struct ProjectUpdatesProjectWire {
    #[serde(default)]
    teams: Option<IdListPageWire>,
    #[serde(default, rename = "projectUpdates")]
    project_updates: Option<ProjectUpdatesPageWire>,
}

#[derive(Deserialize)]
struct ProjectUpdatesPageWire {
    #[serde(default)]
    nodes: Vec<ProjectUpdateWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProjectUpdateWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    user: Option<NameOnlyWire>,
    #[serde(default)]
    body: String,
    #[serde(default)]
    health: Option<String>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    url: String,
}

fn project_not_found() -> LinearApiError {
    LinearApiError::Api {
        status: 200,
        message: "project not found".to_string(),
    }
}

fn project_update_from_wire(wire: ProjectUpdateWire) -> LinearProjectUpdate {
    LinearProjectUpdate {
        id: wire.id,
        author_name: wire
            .user
            .map(|user| user.name)
            .filter(|name| !name.is_empty()),
        body: truncate_bounded(&wire.body, PROJECT_UPDATE_BODY_MAX_CHARS),
        health: wire.health,
        created_at: wire.created_at,
        url: wire.url,
    }
}

/// Fetch one project's updates plus the project's linked team ids, one
/// bounded page. Grant enforcement happens AFTER this call, in the caller
/// ([`crate::connectors::linear_require_any_team_granted`] against the
/// returned team ids) - exactly like [`get_issue`]/[`list_issue_comments`].
pub async fn list_project_updates(
    access_token: &str,
    project_id: &str,
    first: Option<u32>,
) -> Result<(Vec<String>, Vec<LinearProjectUpdate>), LinearApiError> {
    let limit = clamp_first(first, PROJECT_UPDATES_DEFAULT, 1, PROJECT_UPDATES_MAX);
    let data: ProjectUpdatesDataWire = graphql(
        access_token,
        LIST_PROJECT_UPDATES_QUERY,
        serde_json::json!({ "id": project_id, "first": limit }),
    )
    .await?;
    let project = data.project.ok_or_else(project_not_found)?;
    let team_ids = ids_from_page(project.teams);
    let updates = project
        .project_updates
        .map(|page| page.nodes)
        .unwrap_or_default()
        .into_iter()
        .map(project_update_from_wire)
        .collect();
    Ok((team_ids, updates))
}

// --- Slice 3: write operations ---------------------------------------------------
//
// Four fixed mutation documents - issue create/update, comment create,
// project update create - and NOTHING else (no delete/archive/admin
// mutations exist by design). They ride the same [`graphql`] POST as the
// reads: a mutation document is just a query string plus variables to that
// endpoint, so error classification (auth, RATELIMITED, bounded messages)
// is identical. Grant enforcement, the `expected_updated_at` conflict
// check, approval parking, and the action journal all live in the CALLER
// (the provider-proxy route layer): these functions are plain mutations so
// they stay unit-testable without an app handle.
//
// Creates are retry-safe by construction: the caller mints a v4 UUID and
// passes it as the input's `id`, which Linear adopts as the created
// object's id (spike doc). Replaying the same input after an ambiguous
// outcome can then be reconciled by querying that id instead of guessing.

/// Issue priority is an Int 0-4 at Linear (0 = none, 1 = urgent .. 4 = low);
/// out-of-range values are clamped rather than rejected so an off-by-one
/// caller degrades to the nearest real priority.
const ISSUE_PRIORITY_MAX: i64 = 4;

fn clamp_priority(priority: Option<i64>) -> Option<i64> {
    priority.map(|value| value.clamp(0, ISSUE_PRIORITY_MAX))
}

/// The mutation payload said `success: false` without an accompanying
/// GraphQL error (an error array would already have surfaced through
/// [`check_graphql_errors`]). Nothing was applied.
fn mutation_not_applied() -> LinearApiError {
    LinearApiError::Api {
        status: 200,
        message: "Linear reported the change was not applied".to_string(),
    }
}

/// Fields for `issueCreate`. Serializes directly as the GraphQL
/// `IssueCreateInput` variable: camelCase keys, absent optionals OMITTED
/// from the JSON (never serialized as null).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueCreate {
    /// Client-minted v4 UUID; Linear adopts it as the issue id, making the
    /// create idempotent and reconcilable (this same value is the journal's
    /// action id).
    pub id: String,
    pub team_id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

/// Fields for `issueUpdate`. Every field is optional and absent fields are
/// OMITTED from the serialized input (`skip_serializing_if`), so an update
/// can never null a field the caller did not name. The allowed field set is
/// deliberately narrow (no labels, relations, or lifecycle fields).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearIssueUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
}

/// Fields for `commentCreate`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearCommentCreate {
    /// Client-minted v4 UUID (see [`LinearIssueCreate::id`]).
    pub id: String,
    pub issue_id: String,
    pub body: String,
}

/// Fields for `projectUpdateCreate`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearProjectUpdateCreate {
    /// Client-minted v4 UUID (see [`LinearIssueCreate::id`]).
    pub id: String,
    pub project_id: String,
    pub body: String,
    /// One of [`PROJECT_UPDATE_HEALTH_VALUES`]; validated before the request
    /// because the wire value is a GraphQL enum (`ProjectUpdateHealthType`)
    /// and a typo would otherwise surface as an opaque provider error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
}

/// The `ProjectUpdateHealthType` enum values, verbatim from the schema.
pub const PROJECT_UPDATE_HEALTH_VALUES: [&str; 3] = ["onTrack", "atRisk", "offTrack"];

fn validate_project_update_health(health: Option<&str>) -> Result<(), LinearApiError> {
    match health {
        None => Ok(()),
        Some(value) if PROJECT_UPDATE_HEALTH_VALUES.contains(&value) => Ok(()),
        Some(_) => Err(LinearApiError::Api {
            status: 400,
            message: format!(
                "health must be one of: {}",
                PROJECT_UPDATE_HEALTH_VALUES.join(", ")
            ),
        }),
    }
}

// The mutation selections reuse the read summary shape so the tool's reply
// after a write matches what search_issues/get_issue return for the same
// issue.
const CREATE_ISSUE_MUTATION: &str = "mutation CreateIssue($input: IssueCreateInput!) \
     { issueCreate(input: $input) { success issue { id identifier title \
     state { name type } priority assignee { name } team { key } updatedAt url } } }";

const UPDATE_ISSUE_MUTATION: &str =
    "mutation UpdateIssue($id: String!, $input: IssueUpdateInput!) \
     { issueUpdate(id: $id, input: $input) { success issue { id identifier title \
     state { name type } priority assignee { name } team { key } updatedAt url } } }";

const ADD_COMMENT_MUTATION: &str = "mutation AddComment($input: CommentCreateInput!) \
     { commentCreate(input: $input) { success comment { id url } } }";

const CREATE_PROJECT_UPDATE_MUTATION: &str =
    "mutation CreateProjectUpdate($input: ProjectUpdateCreateInput!) \
     { projectUpdateCreate(input: $input) { success projectUpdate { id url } } }";

#[derive(Deserialize)]
struct IssueCreateDataWire {
    #[serde(rename = "issueCreate")]
    issue_create: IssuePayloadWire,
}

#[derive(Deserialize)]
struct IssueUpdateDataWire {
    #[serde(rename = "issueUpdate")]
    issue_update: IssuePayloadWire,
}

#[derive(Deserialize)]
struct IssuePayloadWire {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    issue: Option<IssueSummaryWire>,
}

/// Map an `IssuePayload` to the returned summary: `success: false` (with no
/// GraphQL error array, or that would have surfaced already) and a missing
/// issue both mean the mutation did not take effect.
fn issue_from_payload(payload: IssuePayloadWire) -> Result<LinearIssueSummary, LinearApiError> {
    if !payload.success {
        return Err(mutation_not_applied());
    }
    payload
        .issue
        .map(issue_summary_from_wire)
        .ok_or_else(mutation_not_applied)
}

#[derive(Deserialize)]
struct CommentCreateDataWire {
    #[serde(rename = "commentCreate")]
    comment_create: CommentPayloadWire,
}

#[derive(Deserialize)]
struct CommentPayloadWire {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    comment: Option<IdUrlWire>,
}

#[derive(Deserialize)]
struct IdUrlWire {
    #[serde(default)]
    id: String,
    #[serde(default)]
    url: String,
}

/// The created comment, as returned to the agent: the id (the UUID Clovy
/// minted) plus the canonical Linear URL.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearCommentRef {
    pub id: String,
    pub url: String,
}

fn comment_ref_from_payload(
    payload: CommentPayloadWire,
) -> Result<LinearCommentRef, LinearApiError> {
    if !payload.success {
        return Err(mutation_not_applied());
    }
    payload
        .comment
        .map(|comment| LinearCommentRef {
            id: comment.id,
            url: comment.url,
        })
        .ok_or_else(mutation_not_applied)
}

#[derive(Deserialize)]
struct ProjectUpdateCreateDataWire {
    #[serde(rename = "projectUpdateCreate")]
    project_update_create: ProjectUpdatePayloadWire,
}

#[derive(Deserialize)]
struct ProjectUpdatePayloadWire {
    #[serde(default)]
    success: bool,
    #[serde(default, rename = "projectUpdate")]
    project_update: Option<IdUrlWire>,
}

/// The created project update: the id (the UUID Clovy minted) plus its URL.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinearProjectUpdateRef {
    pub id: String,
    pub url: String,
}

fn project_update_ref_from_payload(
    payload: ProjectUpdatePayloadWire,
) -> Result<LinearProjectUpdateRef, LinearApiError> {
    if !payload.success {
        return Err(mutation_not_applied());
    }
    payload
        .project_update
        .map(|update| LinearProjectUpdateRef {
            id: update.id,
            url: update.url,
        })
        .ok_or_else(mutation_not_applied)
}

/// Create an issue. The caller validates `input.team_id` against the
/// selected-team grant BEFORE calling (this fn is a plain mutation) and has
/// already minted `input.id`; the priority is clamped to Linear's 0-4 range
/// here rather than rejected.
pub async fn create_issue(
    access_token: &str,
    input: LinearIssueCreate,
) -> Result<LinearIssueSummary, LinearApiError> {
    let input = LinearIssueCreate {
        priority: clamp_priority(input.priority),
        ..input
    };
    let data: IssueCreateDataWire = graphql_mutation(
        access_token,
        CREATE_ISSUE_MUTATION,
        serde_json::json!({ "input": input }),
    )
    .await?;
    issue_from_payload(data.issue_create)
}

/// Update an issue's narrow allowed field set. A PLAIN mutation: the
/// `expected_updated_at` conflict check is the CALLER's job - the route
/// layer pre-reads the issue (which also grant-checks its team), compares
/// `updatedAt` against the agent-supplied value, and only calls this when
/// they match. Absent fields are omitted from the input, never nulled.
pub async fn update_issue(
    access_token: &str,
    issue_id: &str,
    input: LinearIssueUpdate,
) -> Result<LinearIssueSummary, LinearApiError> {
    let input = LinearIssueUpdate {
        priority: clamp_priority(input.priority),
        ..input
    };
    let data: IssueUpdateDataWire = graphql_mutation(
        access_token,
        UPDATE_ISSUE_MUTATION,
        serde_json::json!({ "id": issue_id, "input": input }),
    )
    .await?;
    issue_from_payload(data.issue_update)
}

/// Add a comment to an issue. The caller grant-checks the issue's team via
/// a pre-flight read and has already minted `input.id`.
pub async fn add_comment(
    access_token: &str,
    input: LinearCommentCreate,
) -> Result<LinearCommentRef, LinearApiError> {
    let data: CommentCreateDataWire = graphql_mutation(
        access_token,
        ADD_COMMENT_MUTATION,
        serde_json::json!({ "input": input }),
    )
    .await?;
    comment_ref_from_payload(data.comment_create)
}

/// Create a project status update. The caller grant-checks the project's
/// teams via a pre-flight read and has already minted `input.id`; the
/// health value is validated against [`PROJECT_UPDATE_HEALTH_VALUES`] here.
pub async fn create_project_update(
    access_token: &str,
    input: LinearProjectUpdateCreate,
) -> Result<LinearProjectUpdateRef, LinearApiError> {
    validate_project_update_health(input.health.as_deref())?;
    let data: ProjectUpdateCreateDataWire = graphql_mutation(
        access_token,
        CREATE_PROJECT_UPDATE_MUTATION,
        serde_json::json!({ "input": input }),
    )
    .await?;
    project_update_ref_from_payload(data.project_update_create)
}

// --- Reconciliation lookups --------------------------------------------------------
//
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpersisted_grant_cleanup_revokes_every_issued_token() {
        let tokens = LinearTokenResponse {
            access_token: "issued-access".to_string(),
            refresh_token: Some("issued-refresh".to_string()),
            expires_in: 3_600,
            scope: Some("read,write".to_string()),
        };

        assert_eq!(
            unpersisted_revocation_targets(&tokens),
            vec![
                ("issued-refresh", "refresh_token"),
                ("issued-access", "access_token"),
            ]
        );
    }

    #[test]
    fn auth_url_carries_pkce_public_client_params() {
        let url = build_auth_url(
            "client-123",
            "http://127.0.0.1:49152/callback",
            &["read", "write"],
            "challenge",
            "csrf-state",
        );
        assert!(url.starts_with("https://linear.app/oauth/authorize?"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcallback"));
        assert!(url.contains("response_type=code"));
        // Scopes are comma-joined (encoded comma), never space-joined.
        assert!(url.contains("scope=read%2Cwrite"));
        assert!(url.contains("state=csrf-state"));
        assert!(url.contains("code_challenge=challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("actor=user"));
        // Forces the workspace picker so a lingering grant after a
        // non-revoking disconnect cannot silently re-authorize.
        assert!(url.contains("prompt=consent"));
        // Public client: no secret in the authorization request.
        assert!(!url.contains("client_secret"));
    }

    #[test]
    fn token_forms_carry_no_client_secret() {
        let exchange = authorization_code_form(
            "public-id",
            "authorization-code",
            "pkce-verifier",
            "http://127.0.0.1:49152/callback",
        );
        assert!(exchange.contains(&("client_id", "public-id")));
        assert!(exchange.contains(&("code_verifier", "pkce-verifier")));
        assert!(exchange.iter().all(|(key, _)| *key != "client_secret"));

        let refresh = refresh_form("public-id", "refresh-token");
        assert!(refresh.contains(&("client_id", "public-id")));
        assert!(refresh.contains(&("refresh_token", "refresh-token")));
        assert!(refresh.iter().all(|(key, _)| *key != "client_secret"));
    }

    #[test]
    fn parse_scope_field_accepts_commas_spaces_and_mixtures() {
        assert_eq!(parse_scope_field("read,write"), vec!["read", "write"]);
        assert_eq!(parse_scope_field("read write"), vec!["read", "write"]);
        assert_eq!(
            parse_scope_field(" read, write ,,  issues:create "),
            vec!["read", "write", "issues:create"]
        );
        assert!(parse_scope_field("").is_empty());
        assert!(parse_scope_field(" , ,").is_empty());
    }

    #[test]
    fn token_response_accepts_string_and_array_scope_shapes() {
        let string_scope: LinearTokenResponse = serde_json::from_str(
            r#"{
                "access_token": "access",
                "refresh_token": "refresh",
                "expires_in": 86399,
                "scope": "read write"
            }"#,
        )
        .expect("modern string-valued scope");
        let array_scope: LinearTokenResponse = serde_json::from_str(
            r#"{
                "access_token": "access",
                "refresh_token": "refresh",
                "expires_in": 86399,
                "scope": ["read", "write"]
            }"#,
        )
        .expect("legacy array-valued scope");

        assert_eq!(string_scope.scope.as_deref(), Some("read write"));
        assert_eq!(array_scope.scope.as_deref(), Some("read write"));
    }

    #[test]
    fn graphql_auth_errors_classify_as_unauthorized_at_any_status() {
        // Linear signals failures through GraphQL error codes rather than
        // semantic HTTP statuses, so an auth failure must classify as
        // Unauthorized even when it arrives as HTTP 400/200.
        let errors: Vec<GraphqlErrorWire> = serde_json::from_str(
            r#"[{"message":"Authentication required","extensions":{"code":"AUTHENTICATION_ERROR"}}]"#,
        )
        .expect("errors parse");
        assert!(matches!(
            check_graphql_errors(&errors, 400),
            Err(LinearApiError::Unauthorized)
        ));
        let errors: Vec<GraphqlErrorWire> = serde_json::from_str(
            r#"[{"message":"boom","extensions":{"type":"authentication error"}}]"#,
        )
        .expect("errors parse");
        assert!(matches!(
            check_graphql_errors(&errors, 200),
            Err(LinearApiError::Unauthorized)
        ));
        // A non-auth error at HTTP 400 still classifies as a plain API error.
        let errors: Vec<GraphqlErrorWire> =
            serde_json::from_str(r#"[{"message":"boom","extensions":{"code":"INTERNAL_ERROR"}}]"#)
                .expect("errors parse");
        assert!(matches!(
            check_graphql_errors(&errors, 400),
            Err(LinearApiError::Api { status: 400, .. })
        ));
    }

    #[test]
    fn refresh_failure_classification() {
        assert!(matches!(
            classify_refresh_failure(400, Some("invalid_grant")),
            LinearRefreshOutcome::InvalidGrant
        ));
        assert!(matches!(
            classify_refresh_failure(500, None),
            LinearRefreshOutcome::Transient
        ));
        assert!(matches!(
            classify_refresh_failure(429, Some("rate_limited")),
            LinearRefreshOutcome::Transient
        ));
        assert!(matches!(
            classify_refresh_failure(400, Some("invalid_client")),
            LinearRefreshOutcome::Transient
        ));
    }

    #[test]
    fn identity_parses_and_normalizes_the_email() {
        let data: IdentityDataWire = serde_json::from_str(
            r#"{
                "viewer": { "id": "user-1", "name": "Ada", "email": " Ada@Example.COM " },
                "organization": { "id": "org-1", "name": "Acme", "urlKey": "acme" }
            }"#,
        )
        .expect("fixture");
        let identity = identity_from_data(data).expect("identity");
        assert_eq!(identity.workspace_id, "org-1");
        assert_eq!(identity.workspace_name, "Acme");
        assert_eq!(identity.workspace_url_key, "acme");
        assert_eq!(identity.user_id, "user-1");
        assert_eq!(identity.user_name, "Ada");
        assert_eq!(identity.user_email, "ada@example.com");
    }

    #[test]
    fn identity_requires_workspace_and_viewer_ids_but_not_email() {
        // Missing organization: no workspace to key custody by.
        let data: IdentityDataWire =
            serde_json::from_str(r#"{ "viewer": { "id": "user-1" } }"#).expect("fixture");
        let error = identity_from_data(data).unwrap_err();
        assert_eq!(error.code, "connector_identity_failed");

        // Empty viewer id is as unusable as a missing viewer.
        let data: IdentityDataWire = serde_json::from_str(
            r#"{ "viewer": { "id": "" }, "organization": { "id": "org-1" } }"#,
        )
        .expect("fixture");
        assert_eq!(
            identity_from_data(data).unwrap_err().code,
            "connector_identity_failed"
        );

        // An empty email is fine: the account is keyed by workspace id.
        let data: IdentityDataWire = serde_json::from_str(
            r#"{ "viewer": { "id": "user-1" }, "organization": { "id": "org-1" } }"#,
        )
        .expect("fixture");
        let identity = identity_from_data(data).expect("identity");
        assert_eq!(identity.user_email, "");
    }

    fn team(name: &str) -> LinearTeam {
        LinearTeam {
            id: format!("id-{name}"),
            key: name.to_uppercase(),
            name: name.to_string(),
        }
    }

    fn page(names: &[&str], has_next: bool, cursor: Option<&str>) -> TeamsPageWire {
        TeamsPageWire {
            nodes: names.iter().map(|name| team(name)).collect(),
            page_info: PageInfoWire {
                has_next_page: has_next,
                end_cursor: cursor.map(str::to_string),
            },
        }
    }

    #[test]
    fn teams_pager_merges_pages_and_sorts_case_insensitively() {
        let mut pager = TeamsPager::new();
        assert_eq!(
            pager.absorb(page(&["zeta", "Alpha"], true, Some("c1"))),
            Some("c1".to_string())
        );
        assert_eq!(pager.absorb(page(&["beta"], false, None)), None);
        let (teams, truncated) = pager.finish();
        assert!(!truncated);
        let names: Vec<&str> = teams.iter().map(|team| team.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "beta", "zeta"]);
    }

    #[test]
    fn teams_pager_stops_at_the_page_cap_and_reports_truncation() {
        let mut pager = TeamsPager::new();
        for index in 0..TEAMS_MAX_PAGES - 1 {
            let cursor = format!("c{index}");
            assert_eq!(
                pager.absorb(page(&["team"], true, Some(&cursor))),
                Some(cursor)
            );
        }
        // The capping page still claims more data, but the pager refuses.
        assert_eq!(pager.absorb(page(&["team"], true, Some("more"))), None);
        let (teams, truncated) = pager.finish();
        assert!(truncated);
        assert_eq!(teams.len(), TEAMS_MAX_PAGES);
    }

    #[test]
    fn teams_pager_treats_a_missing_cursor_as_the_end() {
        // hasNextPage with no cursor cannot be followed; stop rather than
        // loop on a request that would replay the same page.
        let mut pager = TeamsPager::new();
        assert_eq!(pager.absorb(page(&["team"], true, None)), None);
        let (_, truncated) = pager.finish();
        assert!(!truncated);
    }

    #[test]
    fn graphql_error_classification() {
        // Empty errors array: not an error.
        assert!(check_graphql_errors(&[], 200).is_ok());

        // RATELIMITED via extensions.code.
        let errors: Vec<GraphqlErrorWire> = serde_json::from_str(
            r#"[{ "message": "Rate limit exceeded", "extensions": { "code": "RATELIMITED" } }]"#,
        )
        .expect("fixture");
        assert!(matches!(
            check_graphql_errors(&errors, 400),
            Err(LinearApiError::RateLimited)
        ));

        // RATELIMITED mentioned only in the message still classifies.
        let errors: Vec<GraphqlErrorWire> =
            serde_json::from_str(r#"[{ "message": "RATELIMITED: slow down" }]"#).expect("fixture");
        assert!(matches!(
            check_graphql_errors(&errors, 400),
            Err(LinearApiError::RateLimited)
        ));

        // Anything else surfaces the first message, bounded to the cap.
        let long_message = "x".repeat(API_ERROR_MESSAGE_MAX_LEN + 50);
        let errors = vec![GraphqlErrorWire {
            message: long_message,
            extensions: None,
        }];
        match check_graphql_errors(&errors, 400) {
            Err(LinearApiError::Api { status, message }) => {
                assert_eq!(status, 400);
                assert_eq!(message.len(), API_ERROR_MESSAGE_MAX_LEN);
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[test]
    fn partial_mutation_response_is_ambiguous_but_partial_query_is_definitive() {
        let mutation_response: GraphqlResponseWire<serde_json::Value> = serde_json::from_str(
            r#"{
                "data": { "issueCreate": { "success": true } },
                "errors": [{ "message": "A sibling field failed" }]
            }"#,
        )
        .expect("partial mutation fixture");
        assert!(matches!(
            graphql_data(mutation_response, 200, GraphqlOperation::Mutation),
            Err(LinearApiError::AmbiguousMutationResponse { status: 200 })
        ));

        let query_response: GraphqlResponseWire<serde_json::Value> = serde_json::from_str(
            r#"{
                "data": { "viewer": { "id": "viewer-1" } },
                "errors": [{ "message": "A sibling field failed" }]
            }"#,
        )
        .expect("partial query fixture");
        assert!(matches!(
            graphql_data(query_response, 200, GraphqlOperation::Query),
            Err(LinearApiError::Api { status: 200, .. })
        ));
    }

    #[test]
    fn api_errors_map_to_stable_app_error_codes() {
        assert_eq!(
            AppError::from(LinearApiError::Unauthorized).code,
            "linear_unauthorized"
        );
        assert_eq!(
            AppError::from(LinearApiError::RateLimited).code,
            "linear_rate_limited"
        );
        assert_eq!(
            AppError::from(LinearApiError::Api {
                status: 400,
                message: "bad".to_string()
            })
            .code,
            "linear_api_error"
        );
        // Ambiguity is structural, never inferred from a 2xx status: a
        // PARSED provider rejection stays definitive even at HTTP 200 (the
        // GraphQL-over-HTTP norm for validation errors and success:false
        // payloads), while 5xx and unusable bodies share the ambiguous code
        // so the action dispatch reconciles by UUID.
        for status in [200, 204, 302, 400, 404, 422, 499] {
            assert_eq!(
                AppError::from(LinearApiError::Api {
                    status,
                    message: "rejected".to_string()
                })
                .code,
                "linear_api_error",
                "parsed rejection at status {status} must stay definitive"
            );
        }
        for status in [500, 502, 503, 504] {
            assert_eq!(
                AppError::from(LinearApiError::Api {
                    status,
                    message: "gateway".to_string()
                })
                .code,
                "linear_upstream_error",
                "status {status} must classify as upstream error"
            );
        }
        for status in [200, 500] {
            assert_eq!(
                AppError::from(LinearApiError::UnusableResponse { status }).code,
                "linear_upstream_error",
                "unusable body at status {status} must classify as ambiguous"
            );
        }
        assert_eq!(
            AppError::from(LinearApiError::AmbiguousMutationResponse { status: 200 }).code,
            "linear_upstream_error",
            "partial mutation data must enter the reconciliation path"
        );
        assert_eq!(
            AppError::from(LinearApiError::Network("down".to_string())).code,
            "network_error"
        );
    }

    // --- Slice 2: shared helpers ---------------------------------------------------

    #[test]
    fn clamp_first_defaults_and_bounds() {
        assert_eq!(clamp_first(None, 25, 1, 50), 25);
        assert_eq!(clamp_first(Some(0), 25, 1, 50), 1);
        assert_eq!(clamp_first(Some(1), 25, 1, 50), 1);
        assert_eq!(clamp_first(Some(50), 25, 1, 50), 50);
        assert_eq!(clamp_first(Some(999), 25, 1, 50), 50);
    }

    #[test]
    fn truncate_bounded_leaves_short_and_exact_text_untouched() {
        assert_eq!(truncate_bounded("hello", 10), "hello");
        assert_eq!(truncate_bounded("hello", 5), "hello");
        assert!(!truncate_bounded("hello", 5).contains("truncated"));
    }

    #[test]
    fn truncate_bounded_cuts_and_suffixes_when_over_limit() {
        let truncated = truncate_bounded("hello world", 5);
        assert_eq!(truncated, "hello ... [truncated]");
    }

    #[test]
    fn truncate_bounded_is_char_boundary_safe_on_multi_byte_text() {
        // Multi-byte emoji and CJK characters must never be split mid-codepoint.
        let text = "héllo 世界 🎉🎉🎉 more text past the cut";
        let truncated = truncate_bounded(text, 9);
        assert!(truncated.starts_with("héllo 世界"));
        assert!(truncated.ends_with(TRUNCATION_SUFFIX));
        // Also exercise a cut that lands exactly between two 4-byte emoji.
        let emoji_only = "🎉🎉🎉🎉🎉";
        let truncated_emoji = truncate_bounded(emoji_only, 2);
        assert_eq!(truncated_emoji, format!("🎉🎉{TRUNCATION_SUFFIX}"));
    }

    #[test]
    fn round_to_i64_rounds_rather_than_truncates() {
        assert_eq!(round_to_i64(2.0), 2);
        assert_eq!(round_to_i64(1.9999), 2);
        assert_eq!(round_to_i64(0.0), 0);
    }

    #[test]
    fn any_team_granted_checks_intersection() {
        let granted = vec!["a".to_string(), "b".to_string()];
        assert!(any_team_granted(&["b".to_string()], &granted));
        assert!(!any_team_granted(&["c".to_string()], &granted));
        assert!(!any_team_granted(&["a".to_string()], &[]));
        assert!(!any_team_granted(&[], &granted));
    }

    #[test]
    fn ids_from_page_defaults_a_missing_page_to_empty() {
        assert_eq!(ids_from_page(None), Vec::<String>::new());
        let page = IdListPageWire {
            nodes: vec![
                IdOnlyWire {
                    id: "a".to_string(),
                },
                IdOnlyWire {
                    id: "b".to_string(),
                },
            ],
            page_info: PageInfoWire::default(),
        };
        assert_eq!(ids_from_page(Some(page)), vec!["a", "b"]);
    }

    // --- Slice 2 (+ team-boundary revision): users -----------------------------------

    #[test]
    fn team_members_fixture_parses_active_and_inactive_members() {
        let data: TeamMembersDataWire = serde_json::from_str(
            r#"{
                "teams": { "nodes": [
                    { "members": { "nodes": [
                        { "id": "u1", "name": "Ada Lovelace", "displayName": "ada", "active": true },
                        { "id": "u2", "name": "Bea", "displayName": "bea", "active": false }
                    ] } }
                ] }
            }"#,
        )
        .expect("fixture");
        let users = union_team_members(data.teams.nodes);
        assert_eq!(users.len(), 2);
        // Sorted by name case-insensitively: "Ada Lovelace" before "Bea".
        assert_eq!(users[0].id, "u1");
        assert_eq!(users[0].display_name, "ada");
        assert!(users[0].active);
        assert!(!users[1].active);
    }

    #[test]
    fn union_team_members_dedupes_across_teams_and_sorts_by_name() {
        // A member on two selected teams (u-shared) must appear exactly once,
        // and the union is sorted by name case-insensitively regardless of
        // which team it arrived from.
        let data: TeamMembersDataWire = serde_json::from_str(
            r#"{
                "teams": { "nodes": [
                    { "members": { "nodes": [
                        { "id": "u-shared", "name": "carol", "displayName": "carol", "active": true },
                        { "id": "u-zeta", "name": "Zeta", "displayName": "zeta", "active": true }
                    ] } },
                    { "members": { "nodes": [
                        { "id": "u-shared", "name": "carol", "displayName": "carol", "active": true },
                        { "id": "u-alpha", "name": "Alpha", "displayName": "alpha", "active": true }
                    ] } }
                ] }
            }"#,
        )
        .expect("fixture");
        let users = union_team_members(data.teams.nodes);
        let ids: Vec<&str> = users.iter().map(|u| u.id.as_str()).collect();
        // Deduped (u-shared once) and sorted: Alpha, carol, Zeta.
        assert_eq!(ids, vec!["u-alpha", "u-shared", "u-zeta"]);
    }

    #[test]
    fn union_team_members_tolerates_a_team_with_no_members_block() {
        // A team whose `members` came back null contributes nothing rather
        // than panicking.
        let data: TeamMembersDataWire = serde_json::from_str(
            r#"{ "teams": { "nodes": [ { "members": null }, { "members": { "nodes": [
                { "id": "u1", "name": "Ada", "displayName": "ada", "active": true }
            ] } } ] } }"#,
        )
        .expect("fixture");
        let users = union_team_members(data.teams.nodes);
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].id, "u1");
    }

    #[tokio::test]
    async fn list_users_empty_grant_returns_empty_without_a_request() {
        // The empty-in-list guard: an empty grant returns Ok(vec![]) before
        // any network call, so `id: { in: [] }` never reaches Linear. The
        // dummy token is never used because the guard short-circuits first.
        let users = list_users("unused-token", &[], None)
            .await
            .expect("empty grant is not an error");
        assert!(users.users.is_empty());
        assert!(!users.truncated);
    }

    #[test]
    fn bounded_list_queries_request_truncation_metadata() {
        for (name, query) in [
            ("users", LIST_USERS_QUERY),
            ("projects", LIST_PROJECTS_QUERY),
            ("cycles", LIST_CYCLES_QUERY),
            ("initiatives", LIST_INITIATIVES_QUERY),
        ] {
            assert!(
                query.contains("pageInfo { hasNextPage }"),
                "{name} query must expose whether its bounded page was truncated"
            );
        }
    }

    #[test]
    fn bounded_listings_include_nested_connection_truncation() {
        let users: TeamMembersDataWire = serde_json::from_str(
            r#"{
                "teams": {
                    "nodes": [{
                        "members": {
                            "nodes": [],
                            "pageInfo": { "hasNextPage": true }
                        }
                    }],
                    "pageInfo": { "hasNextPage": false }
                }
            }"#,
        )
        .expect("users fixture");
        assert!(team_members_page_truncated(&users.teams));

        let projects: ProjectsDataWire = serde_json::from_str(
            r#"{
                "projects": {
                    "nodes": [{
                        "id": "project-1",
                        "teams": {
                            "nodes": [],
                            "pageInfo": { "hasNextPage": true }
                        }
                    }],
                    "pageInfo": { "hasNextPage": false }
                }
            }"#,
        )
        .expect("projects fixture");
        assert!(projects_page_truncated(&projects.projects));

        let initiatives: InitiativesDataWire = serde_json::from_str(
            r#"{
                "initiatives": {
                    "nodes": [{
                        "id": "initiative-1",
                        "projects": {
                            "nodes": [],
                            "pageInfo": { "hasNextPage": true }
                        }
                    }],
                    "pageInfo": { "hasNextPage": false }
                }
            }"#,
        )
        .expect("initiatives fixture");
        assert!(initiatives_page_truncated(&initiatives.initiatives));
    }

    // --- Slice 2: projects -------------------------------------------------------------

    #[test]
    fn project_from_wire_handles_missing_status_and_teams() {
        let wire: ProjectWire = serde_json::from_str(
            r#"{ "id": "p1", "name": "Roadmap", "url": "https://linear.app/x/project/p1" }"#,
        )
        .expect("fixture");
        let project = project_from_wire(wire);
        assert_eq!(project.id, "p1");
        assert_eq!(project.state, "");
        assert_eq!(project.target_date, None);
        assert!(project.team_ids.is_empty());
    }

    #[test]
    fn project_from_wire_maps_status_name_and_team_ids() {
        let wire: ProjectWire = serde_json::from_str(
            r#"{
                "id": "p1",
                "name": "Roadmap",
                "status": { "name": "In Progress" },
                "targetDate": "2026-09-01",
                "teams": { "nodes": [ { "id": "team-1" }, { "id": "team-2" } ] },
                "url": "https://linear.app/x/project/p1"
            }"#,
        )
        .expect("fixture");
        let project = project_from_wire(wire);
        assert_eq!(project.state, "In Progress");
        assert_eq!(project.target_date.as_deref(), Some("2026-09-01"));
        assert_eq!(project.team_ids, vec!["team-1", "team-2"]);
    }

    // --- Slice 2: cycles -----------------------------------------------------------

    #[test]
    fn team_cycles_fixture_parses_and_handles_null_name_and_completed_at() {
        let data: TeamCyclesDataWire = serde_json::from_str(
            r#"{
                "team": { "cycles": { "nodes": [
                    { "id": "c1", "number": 4, "name": null, "startsAt": "2026-07-01T00:00:00Z",
                      "endsAt": "2026-07-15T00:00:00Z", "completedAt": null }
                ] } }
            }"#,
        )
        .expect("fixture");
        let team = data.team.expect("team present");
        let cycle = &team.cycles.nodes[0];
        assert_eq!(round_to_i64(cycle.number), 4);
        assert_eq!(cycle.name, None);
        assert_eq!(cycle.completed_at, None);
        assert_eq!(cycle.starts_at, "2026-07-01T00:00:00Z");
    }

    #[test]
    fn team_cycles_fixture_missing_team_parses_as_none() {
        let data: TeamCyclesDataWire = serde_json::from_str(r#"{}"#).expect("fixture");
        assert!(data.team.is_none());
    }

    // --- Slice 2 (+ team-boundary revision): initiatives -----------------------------

    #[test]
    fn initiative_from_wire_narrows_projects_and_keeps_a_partially_granted_initiative() {
        let wire: InitiativeWire = serde_json::from_str(
            r#"{
                "id": "i1",
                "name": "Q3 push",
                "targetDate": "2026-09-30",
                "status": "planned",
                "projects": { "nodes": [
                    { "id": "p1", "name": "Granted", "teams": { "nodes": [ { "id": "team-1" } ] } },
                    { "id": "p2", "name": "Not granted", "teams": { "nodes": [ { "id": "team-9" } ] } }
                ] }
            }"#,
        )
        .expect("fixture");
        let granted = vec!["team-1".to_string()];
        let initiative =
            initiative_from_wire(wire, &granted).expect("partially-granted initiative kept");
        assert_eq!(initiative.id, "i1");
        // Only the granted project survives the narrowing.
        assert_eq!(initiative.projects.len(), 1);
        assert_eq!(initiative.projects[0].id, "p1");
    }

    #[test]
    fn initiative_from_wire_drops_a_fully_out_of_grant_initiative() {
        // An initiative whose every project is outside the grant is entirely
        // outside the boundary and is dropped (reverses the earlier
        // keep-even-when-empty behavior).
        let wire: InitiativeWire = serde_json::from_str(
            r#"{
                "id": "i2",
                "name": "No access",
                "projects": { "nodes": [
                    { "id": "p3", "name": "Blocked", "teams": { "nodes": [ { "id": "team-9" } ] } }
                ] }
            }"#,
        )
        .expect("fixture");
        assert!(initiative_from_wire(wire, &["team-1".to_string()]).is_none());

        // An empty grant drops every initiative.
        let wire: InitiativeWire = serde_json::from_str(
            r#"{
                "id": "i3",
                "name": "Anything",
                "projects": { "nodes": [
                    { "id": "p4", "name": "P", "teams": { "nodes": [ { "id": "team-1" } ] } }
                ] }
            }"#,
        )
        .expect("fixture");
        assert!(initiative_from_wire(wire, &[]).is_none());
    }

    // --- Slice 2: issue search -------------------------------------------------------

    #[test]
    fn issue_search_filter_always_carries_team_scope() {
        let params = IssueSearchParams {
            query: None,
            team_ids: vec!["team-1".to_string(), "team-2".to_string()],
            state_type: None,
            assignee_id: None,
            first: None,
        };
        let filter = issue_search_filter(&params);
        assert_eq!(
            filter,
            serde_json::json!({ "team": { "id": { "in": ["team-1", "team-2"] } } })
        );
    }

    #[test]
    fn issue_search_filter_composes_optional_narrows() {
        let params = IssueSearchParams {
            query: Some("  onboarding  ".to_string()),
            team_ids: vec!["team-1".to_string()],
            state_type: Some("started".to_string()),
            assignee_id: Some("user-1".to_string()),
            first: Some(10),
        };
        let filter = issue_search_filter(&params);
        assert_eq!(
            filter,
            serde_json::json!({
                "team": { "id": { "in": ["team-1"] } },
                "state": { "type": { "eq": "started" } },
                "assignee": { "id": { "eq": "user-1" } },
                "title": { "containsIgnoreCase": "onboarding" },
            })
        );
    }

    #[test]
    fn issue_search_filter_omits_blank_optional_narrows() {
        let params = IssueSearchParams {
            query: Some("   ".to_string()),
            team_ids: vec!["team-1".to_string()],
            state_type: Some("".to_string()),
            assignee_id: None,
            first: None,
        };
        let filter = issue_search_filter(&params);
        assert_eq!(
            filter,
            serde_json::json!({ "team": { "id": { "in": ["team-1"] } } })
        );
    }

    #[test]
    fn issue_summary_from_wire_handles_null_assignee_and_team() {
        let wire: IssueSummaryWire = serde_json::from_str(
            r#"{
                "id": "issue-1",
                "identifier": "ENG-1",
                "title": "Fix the thing",
                "state": { "name": "In Review", "type": "started" },
                "priority": 2,
                "assignee": null,
                "team": null,
                "updatedAt": "2026-07-01T00:00:00Z",
                "url": "https://linear.app/x/issue/ENG-1"
            }"#,
        )
        .expect("fixture");
        let summary = issue_summary_from_wire(wire);
        assert_eq!(summary.assignee_name, None);
        assert_eq!(summary.team_key, "");
        assert_eq!(summary.state_name, "In Review");
        assert_eq!(summary.state_type, "started");
        assert_eq!(summary.priority, 2);
    }

    // --- Slice 2: issue detail -------------------------------------------------------

    #[test]
    fn issue_detail_from_wire_truncates_a_long_description_and_handles_nulls() {
        let long_description = "d".repeat(ISSUE_DESCRIPTION_MAX_CHARS + 500);
        let wire = IssueDetailWire {
            id: "issue-1".to_string(),
            identifier: "ENG-1".to_string(),
            title: "Title".to_string(),
            description: Some(long_description.clone()),
            state: None,
            priority: 0.0,
            assignee: None,
            team: None,
            labels: None,
            url: "https://linear.app/x/issue/ENG-1".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            updated_at: "2026-07-02T00:00:00Z".to_string(),
        };
        let detail = issue_detail_from_wire(wire);
        let description = detail.description.expect("description present");
        assert!(description.len() < long_description.len());
        assert!(description.ends_with(TRUNCATION_SUFFIX));
        assert_eq!(detail.assignee_name, None);
        assert_eq!(detail.team_id, "");
        assert_eq!(detail.team_key, "");
        assert!(detail.label_names.is_empty());
    }

    #[test]
    fn issue_detail_from_wire_maps_team_and_labels() {
        let data: GetIssueDataWire = serde_json::from_str(
            r#"{
                "issue": {
                    "id": "issue-1",
                    "identifier": "ENG-1",
                    "title": "Title",
                    "description": "short",
                    "state": { "name": "Todo", "type": "unstarted" },
                    "priority": 3,
                    "assignee": { "name": "Ada" },
                    "team": { "id": "team-1", "key": "ENG" },
                    "labels": { "nodes": [ { "name": "bug" }, { "name": "urgent" } ] },
                    "url": "https://linear.app/x/issue/ENG-1",
                    "createdAt": "2026-06-01T00:00:00Z",
                    "updatedAt": "2026-07-01T00:00:00Z"
                }
            }"#,
        )
        .expect("fixture");
        let detail = issue_detail_from_wire(data.issue.expect("issue present"));
        assert_eq!(detail.description.as_deref(), Some("short"));
        assert_eq!(detail.team_id, "team-1");
        assert_eq!(detail.team_key, "ENG");
        assert_eq!(detail.assignee_name.as_deref(), Some("Ada"));
        assert_eq!(detail.label_names, vec!["bug", "urgent"]);
        assert_eq!(detail.priority, 3);
    }

    // --- Slice 2: issue comments -----------------------------------------------------

    #[test]
    fn comment_author_name_prefers_user_then_falls_back_to_bot_actor() {
        let with_user = CommentWire {
            id: "c1".to_string(),
            user: Some(NameOnlyWire {
                name: "Ada".to_string(),
            }),
            bot_actor: Some(NameOnlyWire {
                name: "GitHub".to_string(),
            }),
            body: String::new(),
            created_at: String::new(),
            url: String::new(),
        };
        assert_eq!(comment_author_name(&with_user).as_deref(), Some("Ada"));

        let bot_only = CommentWire {
            id: "c2".to_string(),
            user: None,
            bot_actor: Some(NameOnlyWire {
                name: "GitHub".to_string(),
            }),
            body: String::new(),
            created_at: String::new(),
            url: String::new(),
        };
        assert_eq!(comment_author_name(&bot_only).as_deref(), Some("GitHub"));

        let neither = CommentWire {
            id: "c3".to_string(),
            user: None,
            bot_actor: None,
            body: String::new(),
            created_at: String::new(),
            url: String::new(),
        };
        assert_eq!(comment_author_name(&neither), None);
    }

    #[test]
    fn comment_from_wire_truncates_a_long_body() {
        let long_body = "b".repeat(COMMENT_BODY_MAX_CHARS + 200);
        let wire = CommentWire {
            id: "c1".to_string(),
            user: Some(NameOnlyWire {
                name: "Ada".to_string(),
            }),
            bot_actor: None,
            body: long_body.clone(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            url: "https://linear.app/x/issue/ENG-1#comment-c1".to_string(),
        };
        let comment = comment_from_wire(wire);
        assert!(comment.body.len() < long_body.len());
        assert!(comment.body.ends_with(TRUNCATION_SUFFIX));
        assert_eq!(comment.author_name.as_deref(), Some("Ada"));
    }

    #[test]
    fn issue_comments_fixture_returns_team_id_alongside_comments() {
        let data: IssueCommentsDataWire = serde_json::from_str(
            r#"{
                "issue": {
                    "team": { "id": "team-1" },
                    "comments": { "nodes": [
                        { "id": "c1", "user": { "name": "Ada" }, "botActor": null,
                          "body": "hello", "createdAt": "2026-07-01T00:00:00Z",
                          "url": "https://linear.app/x/issue/ENG-1#comment-c1" }
                    ] }
                }
            }"#,
        )
        .expect("fixture");
        let issue = data.issue.expect("issue present");
        assert_eq!(issue.team.expect("team present").id, "team-1");
        assert_eq!(issue.comments.expect("comments present").nodes.len(), 1);
    }

    // --- Slice 2: project updates ----------------------------------------------------

    #[test]
    fn project_update_from_wire_handles_null_author_and_health_and_truncates_body() {
        let long_body = "b".repeat(PROJECT_UPDATE_BODY_MAX_CHARS + 200);
        let wire = ProjectUpdateWire {
            id: "pu1".to_string(),
            user: None,
            body: long_body.clone(),
            health: None,
            created_at: "2026-07-01T00:00:00Z".to_string(),
            url: "https://linear.app/x/project/p1#update-pu1".to_string(),
        };
        let update = project_update_from_wire(wire);
        assert_eq!(update.author_name, None);
        assert_eq!(update.health, None);
        assert!(update.body.len() < long_body.len());
        assert!(update.body.ends_with(TRUNCATION_SUFFIX));
    }

    #[test]
    fn project_updates_fixture_returns_team_ids_alongside_updates() {
        let data: ProjectUpdatesDataWire = serde_json::from_str(
            r#"{
                "project": {
                    "teams": { "nodes": [ { "id": "team-1" }, { "id": "team-2" } ] },
                    "projectUpdates": { "nodes": [
                        { "id": "pu1", "user": { "name": "Ada" }, "body": "on track",
                          "health": "onTrack", "createdAt": "2026-07-01T00:00:00Z",
                          "url": "https://linear.app/x/project/p1#update-pu1" }
                    ] }
                }
            }"#,
        )
        .expect("fixture");
        let project = data.project.expect("project present");
        assert_eq!(ids_from_page(project.teams), vec!["team-1", "team-2"]);
        assert_eq!(
            project
                .project_updates
                .expect("updates present")
                .nodes
                .len(),
            1
        );
    }

    #[test]
    fn project_updates_fixture_missing_project_parses_as_none() {
        let data: ProjectUpdatesDataWire = serde_json::from_str(r#"{}"#).expect("fixture");
        assert!(data.project.is_none());
    }

    // --- Slice 3: write operations -------------------------------------------------

    #[test]
    fn issue_create_input_serializes_camel_case_and_omits_absent_optionals() {
        let input = LinearIssueCreate {
            id: "0d1f2e3a-0000-4000-8000-000000000001".to_string(),
            team_id: "team-1".to_string(),
            title: "Fix the flaky test".to_string(),
            description: None,
            priority: None,
            assignee_id: None,
            project_id: None,
        };
        let json = serde_json::to_value(&input).expect("serialize");
        // The client-minted UUID is present; absent optionals are OMITTED,
        // never serialized as null.
        assert_eq!(
            json,
            serde_json::json!({
                "id": "0d1f2e3a-0000-4000-8000-000000000001",
                "teamId": "team-1",
                "title": "Fix the flaky test",
            })
        );

        let input = LinearIssueCreate {
            id: "0d1f2e3a-0000-4000-8000-000000000002".to_string(),
            team_id: "team-1".to_string(),
            title: "Title".to_string(),
            description: Some("Body".to_string()),
            priority: Some(2),
            assignee_id: Some("user-1".to_string()),
            project_id: Some("project-1".to_string()),
        };
        let json = serde_json::to_value(&input).expect("serialize");
        assert_eq!(json["description"], "Body");
        assert_eq!(json["priority"], 2);
        assert_eq!(json["assigneeId"], "user-1");
        assert_eq!(json["projectId"], "project-1");
    }

    #[test]
    fn issue_update_input_omits_every_absent_field() {
        // Only the named field serializes: an omitted field can never null
        // out its current value at Linear.
        let input = LinearIssueUpdate {
            title: Some("New title".to_string()),
            ..LinearIssueUpdate::default()
        };
        let json = serde_json::to_value(&input).expect("serialize");
        assert_eq!(json, serde_json::json!({ "title": "New title" }));

        let input = LinearIssueUpdate {
            state_id: Some("state-1".to_string()),
            cycle_id: Some("cycle-1".to_string()),
            ..LinearIssueUpdate::default()
        };
        let json = serde_json::to_value(&input).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({ "stateId": "state-1", "cycleId": "cycle-1" })
        );

        // A fully empty update serializes to an empty object.
        let json = serde_json::to_value(LinearIssueUpdate::default()).expect("serialize");
        assert_eq!(json, serde_json::json!({}));
    }

    #[test]
    fn priority_clamps_to_linears_zero_to_four_range() {
        assert_eq!(clamp_priority(None), None);
        assert_eq!(clamp_priority(Some(0)), Some(0));
        assert_eq!(clamp_priority(Some(4)), Some(4));
        assert_eq!(clamp_priority(Some(9)), Some(4));
        assert_eq!(clamp_priority(Some(-1)), Some(0));
    }

    #[test]
    fn comment_and_project_update_inputs_carry_the_client_minted_id() {
        let comment = LinearCommentCreate {
            id: "0d1f2e3a-0000-4000-8000-000000000003".to_string(),
            issue_id: "issue-1".to_string(),
            body: "Looks good".to_string(),
        };
        let json = serde_json::to_value(&comment).expect("serialize");
        assert_eq!(
            json,
            serde_json::json!({
                "id": "0d1f2e3a-0000-4000-8000-000000000003",
                "issueId": "issue-1",
                "body": "Looks good",
            })
        );

        let update = LinearProjectUpdateCreate {
            id: "0d1f2e3a-0000-4000-8000-000000000004".to_string(),
            project_id: "project-1".to_string(),
            body: "On track for the release".to_string(),
            health: Some("onTrack".to_string()),
        };
        let json = serde_json::to_value(&update).expect("serialize");
        assert_eq!(json["id"], "0d1f2e3a-0000-4000-8000-000000000004");
        assert_eq!(json["projectId"], "project-1");
        assert_eq!(json["health"], "onTrack");
        // Absent health is omitted, not null.
        let update = LinearProjectUpdateCreate {
            id: "x".to_string(),
            project_id: "project-1".to_string(),
            body: "b".to_string(),
            health: None,
        };
        let json = serde_json::to_value(&update).expect("serialize");
        assert!(json.get("health").is_none());
    }

    #[test]
    fn project_update_health_validates_against_the_schema_enum() {
        assert!(validate_project_update_health(None).is_ok());
        for value in PROJECT_UPDATE_HEALTH_VALUES {
            assert!(validate_project_update_health(Some(value)).is_ok());
        }
        // Wrong casing and arbitrary strings are rejected before the request.
        for value in ["ontrack", "OnTrack", "green", ""] {
            let error = validate_project_update_health(Some(value)).unwrap_err();
            match error {
                LinearApiError::Api { status, message } => {
                    assert_eq!(status, 400);
                    assert!(message.contains("onTrack"));
                }
                other => panic!("expected Api error, got {other:?}"),
            }
        }
    }

    #[test]
    fn issue_mutation_payload_maps_success_and_rejection() {
        // Success with an issue: the read-summary shape comes back.
        let data: IssueCreateDataWire = serde_json::from_str(
            r#"{
                "issueCreate": {
                    "success": true,
                    "issue": {
                        "id": "issue-1",
                        "identifier": "ENG-42",
                        "title": "Created",
                        "state": { "name": "Todo", "type": "unstarted" },
                        "priority": 2,
                        "assignee": null,
                        "team": { "key": "ENG" },
                        "updatedAt": "2026-07-15T00:00:00Z",
                        "url": "https://linear.app/x/issue/ENG-42"
                    }
                }
            }"#,
        )
        .expect("fixture");
        let summary = issue_from_payload(data.issue_create).expect("summary");
        assert_eq!(summary.identifier, "ENG-42");
        assert_eq!(summary.team_key, "ENG");

        // success=false with no error array: the mutation did not apply.
        let data: IssueUpdateDataWire =
            serde_json::from_str(r#"{ "issueUpdate": { "success": false, "issue": null } }"#)
                .expect("fixture");
        assert!(matches!(
            issue_from_payload(data.issue_update),
            Err(LinearApiError::Api { status: 200, .. })
        ));

        // success=true but no issue node is equally unusable.
        let payload: IssuePayloadWire =
            serde_json::from_str(r#"{ "success": true }"#).expect("fixture");
        assert!(issue_from_payload(payload).is_err());
    }

    #[test]
    fn comment_and_project_update_payloads_map_success_and_rejection() {
        let data: CommentCreateDataWire = serde_json::from_str(
            r#"{
                "commentCreate": {
                    "success": true,
                    "comment": { "id": "comment-1", "url": "https://linear.app/x/c/1" }
                }
            }"#,
        )
        .expect("fixture");
        let comment = comment_ref_from_payload(data.comment_create).expect("comment");
        assert_eq!(comment.id, "comment-1");
        assert_eq!(comment.url, "https://linear.app/x/c/1");

        let payload: CommentPayloadWire =
            serde_json::from_str(r#"{ "success": false }"#).expect("fixture");
        assert!(comment_ref_from_payload(payload).is_err());

        let data: ProjectUpdateCreateDataWire = serde_json::from_str(
            r#"{
                "projectUpdateCreate": {
                    "success": true,
                    "projectUpdate": { "id": "update-1", "url": "https://linear.app/x/pu/1" }
                }
            }"#,
        )
        .expect("fixture");
        let update = project_update_ref_from_payload(data.project_update_create).expect("update");
        assert_eq!(update.id, "update-1");

        let payload: ProjectUpdatePayloadWire =
            serde_json::from_str(r#"{ "success": false }"#).expect("fixture");
        assert!(project_update_ref_from_payload(payload).is_err());
    }
}
