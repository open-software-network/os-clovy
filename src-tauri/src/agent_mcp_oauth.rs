//! OAuth protocol helpers shared by the desktop registry and isolated HTTP fixtures.
//! No keychain, Tauri, CLI identity, browser profile, or account state lives here.
use super::AgentMcpError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::BTreeMap, time::Duration};
use zeroize::{Zeroize, ZeroizeOnDrop};

pub(super) const OAUTH_HTTP_TIMEOUT: Duration = Duration::from_secs(20);
pub(super) const OAUTH_EXPIRY_BUFFER_SECS: i64 = 60;

#[derive(Debug, Deserialize)]
pub(super) struct OAuthResourceMetadata {
    pub(super) resource: String,
    #[serde(default)]
    pub(super) authorization_servers: Vec<String>,
    #[serde(default)]
    pub(super) scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OAuthAuthorizationServerMetadata {
    pub(super) issuer: String,
    pub(super) authorization_endpoint: String,
    pub(super) token_endpoint: String,
    #[serde(default)]
    pub(super) registration_endpoint: Option<String>,
    #[serde(default)]
    pub(super) revocation_endpoint: Option<String>,
    #[serde(default)]
    pub(super) response_types_supported: Vec<String>,
    #[serde(default)]
    pub(super) grant_types_supported: Vec<String>,
    #[serde(default)]
    pub(super) code_challenge_methods_supported: Vec<String>,
    #[serde(default, deserialize_with = "oauth_optional_string_list")]
    pub(super) response_modes_supported: Option<Vec<String>>,
    #[serde(default, deserialize_with = "oauth_optional_string_list")]
    pub(super) token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

// Omission permits the standard default; a present null or malformed value is
// not an omitted metadata member and must not silently select that default.
fn oauth_optional_string_list<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error> {
    Vec::<String>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Serialize)]
pub(super) struct OAuthRegistrationRequest<'a> {
    client_name: &'a str,
    redirect_uris: Vec<&'a str>,
    grant_types: Vec<&'a str>,
    response_types: Vec<&'a str>,
    token_endpoint_auth_method: &'a str,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
pub(super) struct OAuthRegistrationResponse {
    pub(super) client_id: String,
    #[serde(default)]
    pub(super) client_secret: Option<String>,
}

#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
pub(super) struct OAuthTokenResponse {
    pub(super) access_token: String,
    #[serde(default)]
    pub(super) refresh_token: Option<String>,
    #[zeroize(skip)]
    #[serde(default)]
    pub(super) expires_in: Option<i64>,
    #[zeroize(skip)]
    #[serde(default)]
    pub(super) scope: Option<String>,
    #[serde(default)]
    pub(super) token_type: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct OAuthTokenError {
    #[serde(default)]
    error: Option<String>,
}

pub(super) fn oauth_now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub(super) fn secure_oauth_url(raw: &str) -> Result<reqwest::Url, AgentMcpError> {
    let parsed = reqwest::Url::parse(raw).map_err(|_| AgentMcpError::Protocol)?;
    let secure = parsed.scheme() == "https"
        || (parsed.scheme() == "http"
            && parsed
                .host_str()
                .is_some_and(|host| matches!(host, "localhost" | "127.0.0.1" | "::1")));
    if !secure
        || parsed.username() != ""
        || parsed.password().is_some()
        || parsed.host_str().is_none()
        || parsed.fragment().is_some()
    {
        return Err(AgentMcpError::Protocol);
    }
    Ok(parsed)
}

fn oauth_http_client() -> Result<reqwest::Client, AgentMcpError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AgentMcpError::Transport)
}

fn oauth_transport_error(error: reqwest::Error) -> AgentMcpError {
    if error.is_timeout() {
        AgentMcpError::TimedOut
    } else {
        AgentMcpError::Transport
    }
}

/// This override belongs to the exact official MCP resource, never to model
/// input or an arbitrary metadata field. Both issuer and endpoints are checked.
fn is_vercel_mcp(resource: &reqwest::Url) -> bool {
    resource.as_str() == "https://mcp.vercel.com/"
}

fn validate_oauth_resource(
    requested: &reqwest::Url,
    metadata: &OAuthResourceMetadata,
) -> Result<(), AgentMcpError> {
    let declared = secure_oauth_url(&metadata.resource)?;
    if declared.origin() != requested.origin()
        || (is_vercel_mcp(requested) && declared != *requested)
    {
        return Err(AgentMcpError::Protocol);
    }
    Ok(())
}

pub(super) fn oauth_metadata_urls(
    resource: &reqwest::Url,
    issuer: &reqwest::Url,
) -> Result<Vec<reqwest::Url>, AgentMcpError> {
    if is_vercel_mcp(resource) {
        if issuer.as_str() != "https://vercel.com/" {
            return Err(AgentMcpError::Protocol);
        }
        return Ok(vec![secure_oauth_url(
            "https://mcp.vercel.com/.well-known/oauth-authorization-server",
        )?]);
    }
    authorization_server_metadata_urls(issuer)
}

pub(super) fn validate_oauth_metadata(
    resource: &reqwest::Url,
    issuer: &reqwest::Url,
    auth: &OAuthAuthorizationServerMetadata,
) -> Result<(), AgentMcpError> {
    if secure_oauth_url(&auth.issuer)? != *issuer
        || !auth
            .response_types_supported
            .iter()
            .any(|value| value == "code")
        || (!auth.grant_types_supported.is_empty()
            && !auth
                .grant_types_supported
                .iter()
                .any(|value| value == "authorization_code"))
        || !auth
            .code_challenge_methods_supported
            .iter()
            .any(|value| value == "S256")
        || auth
            .response_modes_supported
            .as_ref()
            .is_some_and(|modes| !modes.iter().any(|mode| mode == "query"))
        || auth
            .token_endpoint_auth_methods_supported
            .as_ref()
            .is_some_and(|methods| !methods.iter().any(|method| method == "none"))
    {
        return Err(AgentMcpError::Protocol);
    }
    secure_oauth_url(&auth.authorization_endpoint)?;
    secure_oauth_url(&auth.token_endpoint)?;
    for endpoint in [&auth.registration_endpoint, &auth.revocation_endpoint]
        .into_iter()
        .flatten()
    {
        secure_oauth_url(endpoint)?;
    }
    if is_vercel_mcp(resource)
        && (issuer.as_str() != "https://vercel.com/"
            || auth.token_endpoint_auth_methods_supported.is_none()
            || auth.authorization_endpoint != "https://vercel.com/oauth/authorize"
            || auth.token_endpoint != "https://vercel.com/api/login/oauth/token"
            || auth.registration_endpoint.as_deref()
                != Some("https://vercel.com/api/login/oauth/register")
            || auth.revocation_endpoint.as_deref()
                != Some("https://vercel.com/api/login/oauth/token/revoke"))
    {
        return Err(AgentMcpError::Protocol);
    }
    Ok(())
}

fn validate_tokens(tokens: &OAuthTokenResponse) -> Result<(), AgentMcpError> {
    if tokens.access_token.trim().is_empty()
        || tokens
            .token_type
            .as_ref()
            .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
        || tokens
            .refresh_token
            .as_ref()
            .is_some_and(|token| token.trim().is_empty())
    {
        return Err(AgentMcpError::Protocol);
    }
    Ok(())
}

async fn provider_error(response: reqwest::Response) -> AgentMcpError {
    let error = response
        .json::<OAuthTokenError>()
        .await
        .ok()
        .and_then(|body| body.error);
    let code = match error.as_deref() {
        Some("invalid_client") => "invalid_client",
        Some("invalid_client_metadata") => "invalid_client_metadata",
        Some("invalid_redirect_uri") => "invalid_redirect_uri",
        Some("invalid_software_statement") => "invalid_software_statement",
        Some("unapproved_software_statement") => "unapproved_software_statement",
        Some("invalid_request") => "invalid_request",
        Some("unauthorized_client") => "unauthorized_client",
        Some("access_denied") => "access_denied",
        Some("unsupported_grant_type") => "unsupported_grant_type",
        Some("invalid_scope") => "invalid_scope",
        Some("invalid_grant") => "invalid_grant",
        Some("server_error") => "server_error",
        Some("temporarily_unavailable") => "temporarily_unavailable",
        _ => "provider_error",
    };
    AgentMcpError::OauthProviderRejected(code)
}

pub(super) fn protected_resource_metadata_urls(
    resource: &reqwest::Url,
) -> Result<Vec<reqwest::Url>, AgentMcpError> {
    let origin = format!(
        "{}://{}{}",
        resource.scheme(),
        resource.host_str().ok_or(AgentMcpError::Protocol)?,
        resource
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    );
    let mut paths = Vec::new();
    let resource_path = resource.path().trim_start_matches('/');
    if !resource_path.is_empty() {
        paths.push(format!(
            "{origin}/.well-known/oauth-protected-resource/{resource_path}"
        ));
    }
    paths.push(format!("{origin}/.well-known/oauth-protected-resource"));
    paths
        .into_iter()
        .map(|value| reqwest::Url::parse(&value).map_err(|_| AgentMcpError::Protocol))
        .collect()
}

pub(super) fn authorization_server_metadata_urls(
    issuer: &reqwest::Url,
) -> Result<Vec<reqwest::Url>, AgentMcpError> {
    let origin = format!(
        "{}://{}{}",
        issuer.scheme(),
        issuer.host_str().ok_or(AgentMcpError::Protocol)?,
        issuer
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default()
    );
    let issuer_path = issuer.path().trim_matches('/');
    let suffix = if issuer_path.is_empty() {
        String::new()
    } else {
        format!("/{issuer_path}")
    };
    [
        format!("{origin}/.well-known/oauth-authorization-server{suffix}"),
        format!("{origin}/.well-known/openid-configuration{suffix}"),
    ]
    .into_iter()
    .map(|value| reqwest::Url::parse(&value).map_err(|_| AgentMcpError::Protocol))
    .collect()
}

pub(super) async fn get_first_oauth_metadata<T: for<'de> Deserialize<'de>>(
    client: &reqwest::Client,
    urls: Vec<reqwest::Url>,
) -> Result<T, AgentMcpError> {
    for url in urls {
        let Ok(response) = client.get(url).timeout(OAUTH_HTTP_TIMEOUT).send().await else {
            continue;
        };
        if response.status().is_success() {
            if let Ok(metadata) = response.json::<T>().await {
                return Ok(metadata);
            }
        }
    }
    Err(AgentMcpError::Protocol)
}

pub(super) fn resource_metadata_from_challenge(value: &str) -> Option<&str> {
    let marker = "resource_metadata=";
    let start = value.find(marker)? + marker.len();
    let remainder = value[start..].trim_start();
    if let Some(quoted) = remainder.strip_prefix('"') {
        return quoted.split_once('"').map(|(url, _)| url);
    }
    Some(
        remainder
            .split([',', ' '])
            .next()
            .unwrap_or_default()
            .trim(),
    )
    .filter(|value| !value.is_empty())
}

pub(super) async fn challenge_resource_metadata_url(
    client: &reqwest::Client,
    resource_url: &reqwest::Url,
    initialize_params: &serde_json::Value,
) -> Result<reqwest::Url, AgentMcpError> {
    let response = client
        .post(resource_url.clone())
        .timeout(OAUTH_HTTP_TIMEOUT)
        .header("accept", "application/json, text/event-stream")
        .header("content-type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": initialize_params
        }))
        .send()
        .await
        .map_err(oauth_transport_error)?;
    let advertised = response
        .headers()
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find_map(resource_metadata_from_challenge)
        .ok_or(AgentMcpError::Protocol)
        .and_then(secure_oauth_url)?;
    if advertised.origin() != resource_url.origin() {
        return Err(AgentMcpError::Protocol);
    }
    Ok(advertised)
}

pub(super) async fn discover_oauth_metadata(
    endpoint: &str,
    initialize_params: serde_json::Value,
) -> Result<(OAuthResourceMetadata, OAuthAuthorizationServerMetadata), AgentMcpError> {
    let resource_url = secure_oauth_url(endpoint)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| AgentMcpError::Transport)?;
    let resource = match get_first_oauth_metadata::<OAuthResourceMetadata>(
        &client,
        protected_resource_metadata_urls(&resource_url)?,
    )
    .await
    {
        Ok(metadata) => metadata,
        Err(_) => {
            let advertised =
                challenge_resource_metadata_url(&client, &resource_url, &initialize_params).await?;
            get_first_oauth_metadata::<OAuthResourceMetadata>(&client, vec![advertised]).await?
        }
    };
    validate_oauth_resource(&resource_url, &resource)?;
    let issuer = resource
        .authorization_servers
        .first()
        .ok_or(AgentMcpError::Protocol)
        .and_then(|value| secure_oauth_url(value))?;
    let auth = get_first_oauth_metadata::<OAuthAuthorizationServerMetadata>(
        &client,
        oauth_metadata_urls(&resource_url, &issuer)?,
    )
    .await?;
    validate_oauth_metadata(&resource_url, &issuer, &auth)?;
    Ok((resource, auth))
}

pub(super) async fn register_oauth_client(
    auth: &OAuthAuthorizationServerMetadata,
    redirect_uri: &str,
) -> Result<OAuthRegistrationResponse, AgentMcpError> {
    let endpoint = auth
        .registration_endpoint
        .as_deref()
        .ok_or(AgentMcpError::Protocol)?;
    let response = oauth_http_client()?
        .post(secure_oauth_url(endpoint)?)
        .timeout(OAUTH_HTTP_TIMEOUT)
        .json(&OAuthRegistrationRequest {
            client_name: "Clovy",
            redirect_uris: vec![redirect_uri],
            grant_types: if auth
                .grant_types_supported
                .iter()
                .any(|v| v == "refresh_token")
            {
                vec!["authorization_code", "refresh_token"]
            } else {
                vec!["authorization_code"]
            },
            response_types: vec!["code"],
            token_endpoint_auth_method: "none",
        })
        .send()
        .await
        .map_err(oauth_transport_error)?;
    if !response.status().is_success() {
        return Err(provider_error(response).await);
    }
    let registration = response
        .json::<OAuthRegistrationResponse>()
        .await
        .map_err(|_| AgentMcpError::Protocol)?;
    if registration.client_id.trim().is_empty() {
        return Err(AgentMcpError::Protocol);
    }
    Ok(registration)
}

pub(super) fn oauth_authorization_url(
    auth: &OAuthAuthorizationServerMetadata,
    client_id: &str,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
    resource: &str,
    scope: &str,
) -> Result<String, AgentMcpError> {
    let mut url = secure_oauth_url(&auth.authorization_endpoint)?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("client_id", client_id)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("response_mode", "query")
            .append_pair("code_challenge", challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state)
            .append_pair("resource", resource);
        if !scope.is_empty() {
            query.append_pair("scope", scope);
        }
    }
    Ok(url.to_string())
}

pub(super) async fn exchange_oauth_code(
    auth: &OAuthAuthorizationServerMetadata,
    client_id: &str,
    client_secret: Option<&str>,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
    resource: &str,
) -> Result<OAuthTokenResponse, AgentMcpError> {
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("resource", resource),
    ];
    if let Some(secret) = client_secret.filter(|value| !value.is_empty()) {
        form.push(("client_secret", secret));
    }
    let request = oauth_http_client()?
        .post(secure_oauth_url(&auth.token_endpoint)?)
        .timeout(OAUTH_HTTP_TIMEOUT)
        .form(&form);
    super::prepare_oauth_code_exchange().await?;
    super::admit_oauth_code_exchange()?;
    let response = request.send().await.map_err(oauth_transport_error)?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        let error = provider_error(response).await;
        super::record_oauth_exchange_refusal(status, &error);
        return Err(error);
    }
    let tokens = response
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|_| AgentMcpError::Protocol)?;
    validate_tokens(&tokens)?;
    Ok(tokens)
}

pub(super) fn oauth_refresh_due(oauth: &BTreeMap<String, String>, force: bool) -> bool {
    force
        || oauth
            .get("expires_at_unix")
            .and_then(|value| value.parse::<i64>().ok())
            .is_some_and(|expires| expires <= oauth_now_unix() + OAUTH_EXPIRY_BUFFER_SECS)
}

pub(super) async fn refresh_oauth_bundle(
    oauth: &mut BTreeMap<String, String>,
    force: bool,
) -> Result<bool, AgentMcpError> {
    let Some(refresh_token) = oauth
        .get("refresh_token")
        .cloned()
        .map(zeroize::Zeroizing::new)
    else {
        return if oauth_refresh_due(oauth, force) {
            Err(AgentMcpError::OauthReconnectRequired)
        } else {
            Ok(false)
        };
    };
    if !oauth_refresh_due(oauth, force) {
        return Ok(false);
    }
    let token_endpoint = oauth
        .get("token_endpoint")
        .ok_or(AgentMcpError::Protocol)
        .and_then(|value| secure_oauth_url(value))?;
    let client_id = oauth
        .get("client_id")
        .filter(|value| !value.trim().is_empty())
        .ok_or(AgentMcpError::Protocol)?;
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", client_id.as_str()),
    ];
    if let Some(resource) = oauth.get("resource") {
        form.push(("resource", resource.as_str()));
    }
    if let Some(client_secret) = oauth.get("client_secret") {
        form.push(("client_secret", client_secret.as_str()));
    }
    let response = oauth_http_client()?
        .post(token_endpoint)
        .timeout(OAUTH_HTTP_TIMEOUT)
        .form(&form)
        .send()
        .await
        .map_err(oauth_transport_error)?;
    if !response.status().is_success() {
        let error = provider_error(response).await;
        return if matches!(error, AgentMcpError::OauthProviderRejected("invalid_grant")) {
            Err(AgentMcpError::OauthReconnectRequired)
        } else {
            Err(error)
        };
    }
    let tokens = response
        .json::<OAuthTokenResponse>()
        .await
        .map_err(|_| AgentMcpError::Protocol)?;
    validate_tokens(&tokens)?;
    if let Some(mut previous) = oauth.insert("access_token".into(), tokens.access_token.clone()) {
        previous.zeroize();
    }
    if let Some(rotated) = tokens.refresh_token.as_ref() {
        if let Some(mut previous) = oauth.insert("refresh_token".into(), rotated.clone()) {
            previous.zeroize();
        }
    }
    if let Some(expires_in) = tokens.expires_in {
        oauth.insert(
            "expires_at_unix".into(),
            oauth_now_unix()
                .saturating_add(expires_in.max(0))
                .to_string(),
        );
    } else {
        oauth.remove("expires_at_unix");
    }
    if let Some(scope) = tokens.scope.as_ref() {
        oauth.insert("scope".into(), scope.clone());
    }
    Ok(true)
}

pub(super) async fn revoke_oauth_bundle(oauth: &BTreeMap<String, String>) -> Option<bool> {
    let endpoint = oauth.get("revocation_endpoint")?;
    let token = oauth
        .get("refresh_token")
        .or_else(|| oauth.get("access_token"))?;
    let endpoint = match secure_oauth_url(endpoint) {
        Ok(url) => url,
        Err(_) => return Some(false),
    };
    let mut form = vec![("token", token.as_str())];
    if let Some(client_id) = oauth.get("client_id") {
        form.push(("client_id", client_id));
    }
    if let Some(secret) = oauth.get("client_secret") {
        form.push(("client_secret", secret));
    }
    let client = match reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(client) => client,
        Err(_) => return Some(false),
    };
    Some(
        client
            .post(endpoint)
            .timeout(OAUTH_HTTP_TIMEOUT)
            .form(&form)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success()),
    )
}

#[cfg(test)]
#[path = "agent_mcp_oauth_protocol_tests.rs"]
mod protocol_tests;
