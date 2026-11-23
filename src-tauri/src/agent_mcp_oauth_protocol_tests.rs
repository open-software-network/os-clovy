//! These fixtures call the same discovery, registration, exchange, rotation and
//! revocation helpers used by the desktop. All credentials are synthetic.
use super::*;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn vercel_metadata() -> OAuthAuthorizationServerMetadata {
    serde_json::from_value(json!({
        "issuer":"https://vercel.com",
        "authorization_endpoint":"https://vercel.com/oauth/authorize",
        "token_endpoint":"https://vercel.com/api/login/oauth/token",
        "registration_endpoint":"https://vercel.com/api/login/oauth/register",
        "revocation_endpoint":"https://vercel.com/api/login/oauth/token/revoke",
        "response_types_supported":["code"],
        "grant_types_supported":["authorization_code","refresh_token"],
        "code_challenge_methods_supported":["S256"],
        "token_endpoint_auth_methods_supported":["none"]
    }))
    .unwrap()
}

#[test]
fn exact_official_resource_selects_mcp_metadata_without_issuer_fallback() {
    let issuer = secure_oauth_url("https://vercel.com").unwrap();
    for endpoint in ["https://mcp.vercel.com", "https://mcp.vercel.com/"] {
        let resource = secure_oauth_url(endpoint).unwrap();
        let candidates = oauth_metadata_urls(&resource, &issuer).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].as_str(),
            "https://mcp.vercel.com/.well-known/oauth-authorization-server"
        );
        validate_oauth_metadata(&resource, &issuer, &vercel_metadata()).unwrap();
    }
    for endpoint in [
        "https://mcp.vercel.com/other",
        "https://mcp.vercel.com/?next=1",
        "https://mcp.vercel.com.evil.example/",
        "https://mcp.vercel.com:444/",
    ] {
        let candidates =
            oauth_metadata_urls(&secure_oauth_url(endpoint).unwrap(), &issuer).unwrap();
        assert!(candidates
            .iter()
            .all(|url| url.host_str() == Some("vercel.com")));
    }
    assert!(oauth_metadata_urls(
        &secure_oauth_url("https://mcp.vercel.com").unwrap(),
        &secure_oauth_url("https://other.example").unwrap()
    )
    .is_err());
}

#[test]
fn vercel_rejects_general_login_contract_and_untrusted_endpoints() {
    let resource = secure_oauth_url("https://mcp.vercel.com").unwrap();
    let issuer = secure_oauth_url("https://vercel.com").unwrap();
    let mut auth = vercel_metadata();
    auth.response_modes_supported = Some(vec!["web_message.opener".into()]);
    assert!(validate_oauth_metadata(&resource, &issuer, &auth).is_err());
    auth.response_modes_supported = Some(vec!["query".into()]);
    validate_oauth_metadata(&resource, &issuer, &auth).unwrap();
    auth.token_endpoint = "https://api.vercel.com/login/oauth/token".into();
    assert!(validate_oauth_metadata(&resource, &issuer, &auth).is_err());
    auth = vercel_metadata();
    auth.registration_endpoint = Some("https://attacker.example/register".into());
    assert!(validate_oauth_metadata(&resource, &issuer, &auth).is_err());
    auth = vercel_metadata();
    auth.issuer = "https://other.example".into();
    assert!(validate_oauth_metadata(&resource, &issuer, &auth).is_err());
    auth = vercel_metadata();
    auth.code_challenge_methods_supported = vec!["plain".into()];
    assert!(validate_oauth_metadata(&resource, &issuer, &auth).is_err());
    auth = vercel_metadata();
    auth.token_endpoint_auth_methods_supported = Some(vec!["client_secret_basic".into()]);
    assert!(validate_oauth_metadata(&resource, &issuer, &auth).is_err());
    auth = vercel_metadata();
    auth.token_endpoint_auth_methods_supported = None;
    assert!(validate_oauth_metadata(&resource, &issuer, &auth).is_err());
}

#[test]
fn official_resource_rejects_changed_audience_path_query_or_origin() {
    let requested = secure_oauth_url("https://mcp.vercel.com").unwrap();
    for resource in ["https://mcp.vercel.com", "https://mcp.vercel.com/"] {
        let metadata = OAuthResourceMetadata {
            resource: resource.into(),
            authorization_servers: vec!["https://vercel.com".into()],
            scopes_supported: vec![],
        };
        validate_oauth_resource(&requested, &metadata).unwrap();
    }
    for resource in [
        "https://mcp.vercel.com/other",
        "https://mcp.vercel.com/?audience=other",
        "https://other.example/",
    ] {
        let metadata = OAuthResourceMetadata {
            resource: resource.into(),
            authorization_servers: vec!["https://vercel.com".into()],
            scopes_supported: vec![],
        };
        assert!(validate_oauth_resource(&requested, &metadata).is_err());
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, String) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        assert!(bytes.len() <= 64 * 1024);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let len = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + len {
                let request = String::from_utf8(bytes).unwrap();
                let (headers, body) = request.split_once("\r\n\r\n").unwrap();
                return (headers.to_owned(), body.to_owned());
            }
        }
    }
}

async fn reply(stream: &mut tokio::net::TcpStream, status: &str, body: &Value) {
    let body = body.to_string();
    stream
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
}

async fn listener() -> (tokio::net::TcpListener, String) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    (listener, origin)
}

#[tokio::test]
async fn fixture_discovers_registers_and_exchanges_code_without_cli_identity() {
    let (listener, origin) = listener().await;
    let fixture_origin = origin.clone();
    let fixture = tokio::spawn(async move {
        let mut seen = Vec::new();
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (headers, body) = read_request(&mut stream).await;
            let path = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let response = match path {
                "/.well-known/oauth-protected-resource" => {
                    json!({"resource":format!("{fixture_origin}/"),"authorization_servers":[fixture_origin],"scopes_supported":["openid"]})
                }
                "/.well-known/oauth-authorization-server" => {
                    json!({"issuer":fixture_origin,"authorization_endpoint":format!("{fixture_origin}/authorize"),"token_endpoint":format!("{fixture_origin}/token"),"registration_endpoint":format!("{fixture_origin}/register"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"],"response_modes_supported":["query"],"token_endpoint_auth_methods_supported":["none"]})
                }
                "/register" => {
                    let registration: Value = serde_json::from_str(&body).unwrap();
                    assert_eq!(registration["client_name"], "Clovy");
                    assert_eq!(registration["token_endpoint_auth_method"], "none");
                    assert_eq!(
                        registration["redirect_uris"],
                        json!(["http://127.0.0.1:45678/callback"])
                    );
                    json!({"client_id":"synthetic-clovy-client"})
                }
                "/token" => {
                    let form: BTreeMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect();
                    assert_eq!(form["grant_type"], "authorization_code");
                    assert_eq!(form["code_verifier"], "synthetic-verifier");
                    assert_eq!(form["client_id"], "synthetic-clovy-client");
                    assert_eq!(form["redirect_uri"], "http://127.0.0.1:45678/callback");
                    assert_eq!(form["resource"], format!("{fixture_origin}/"));
                    json!({"access_token":"synthetic-access","refresh_token":"synthetic-refresh","token_type":"Bearer","expires_in":3600})
                }
                _ => panic!("unexpected fixture route"),
            };
            seen.push(path.to_owned());
            reply(&mut stream, "200 OK", &response).await;
        }
        seen
    });
    let (resource, auth) = discover_oauth_metadata(&origin, json!({})).await.unwrap();
    let registration = register_oauth_client(&auth, "http://127.0.0.1:45678/callback")
        .await
        .unwrap();
    let authorization = oauth_authorization_url(
        &auth,
        &registration.client_id,
        "http://127.0.0.1:45678/callback",
        "synthetic-challenge",
        "synthetic-state",
        &resource.resource,
        "openid",
    )
    .unwrap();
    let query: BTreeMap<_, _> = reqwest::Url::parse(&authorization)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["code_challenge"], "synthetic-challenge");
    assert_eq!(query["state"], "synthetic-state");
    assert_eq!(query["scope"], "openid");
    assert_eq!(query["response_mode"], "query");
    let tokens = exchange_oauth_code(
        &auth,
        &registration.client_id,
        None,
        "synthetic-code",
        "synthetic-verifier",
        "http://127.0.0.1:45678/callback",
        &resource.resource,
    )
    .await
    .unwrap();
    assert_eq!(tokens.access_token, "synthetic-access");
    assert_eq!(fixture.await.unwrap().len(), 4);
}

#[tokio::test]
async fn discovery_rejects_web_message_only_before_registration_or_browser_handoff() {
    let (listener, origin) = listener().await;
    let fixture_origin = origin.clone();
    let fixture = tokio::spawn(async move {
        for index in 0..2 {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            let body = if index == 0 {
                json!({"resource":format!("{fixture_origin}/"),"authorization_servers":[fixture_origin]})
            } else {
                json!({"issuer":fixture_origin,"authorization_endpoint":format!("{fixture_origin}/authorize"),"token_endpoint":format!("{fixture_origin}/token"),"registration_endpoint":format!("{fixture_origin}/register"),"response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"response_modes_supported":["web_message.opener"]})
            };
            reply(&mut stream, "200 OK", &body).await;
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    assert!(matches!(
        discover_oauth_metadata(&origin, json!({})).await,
        Err(AgentMcpError::Protocol)
    ));
    fixture.await.unwrap();
}

fn token_map(origin: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("client_id".into(), "synthetic-clovy-client".into()),
        ("token_endpoint".into(), format!("{origin}/token")),
        ("access_token".into(), "synthetic-old-access".into()),
        ("refresh_token".into(), "synthetic-old-refresh".into()),
        ("expires_at_unix".into(), "1".into()),
    ])
}

#[tokio::test]
async fn fixture_rotates_granted_refresh_and_preserves_it_when_omitted() {
    let (listener, origin) = listener().await;
    let fixture = tokio::spawn(async move {
        for rotation in [true, false] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let (_, body) = read_request(&mut stream).await;
            let form: BTreeMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
                .into_owned()
                .collect();
            assert_eq!(form["grant_type"], "refresh_token");
            assert_eq!(
                form["refresh_token"],
                if rotation {
                    "synthetic-old-refresh"
                } else {
                    "synthetic-rotated-refresh"
                }
            );
            let response = if rotation {
                json!({"access_token":"synthetic-new-access","refresh_token":"synthetic-rotated-refresh","expires_in":3600})
            } else {
                json!({"access_token":"synthetic-newer-access","expires_in":i64::MAX})
            };
            reply(&mut stream, "200 OK", &response).await;
        }
    });
    let mut oauth = token_map(&origin);
    assert!(refresh_oauth_bundle(&mut oauth, false).await.unwrap());
    assert_eq!(oauth["access_token"], "synthetic-new-access");
    assert_eq!(oauth["refresh_token"], "synthetic-rotated-refresh");
    assert!(oauth["expires_at_unix"].parse::<i64>().unwrap() > oauth_now_unix());
    assert!(refresh_oauth_bundle(&mut oauth, true).await.unwrap());
    assert_eq!(oauth["refresh_token"], "synthetic-rotated-refresh");
    assert_eq!(oauth["expires_at_unix"], i64::MAX.to_string());
    fixture.await.unwrap();
}

#[tokio::test]
async fn fixture_invalid_grant_does_not_retry_or_replace_committed_tokens() {
    let (listener, origin) = listener().await;
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        reply(&mut stream, "400 Bad Request", &json!({"error":"invalid_grant","error_description":"synthetic-secret-must-not-escape"})).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err()
        );
    });
    let mut oauth = token_map(&origin);
    let before = oauth.clone();
    let error = refresh_oauth_bundle(&mut oauth, true).await.unwrap_err();
    assert!(matches!(error, AgentMcpError::OauthReconnectRequired));
    assert!(!error.to_string().contains("synthetic-secret"));
    assert_eq!(oauth, before);
    fixture.await.unwrap();
}

#[tokio::test]
async fn absent_refresh_requires_reconnect_only_when_expired_or_forced() {
    let mut oauth = BTreeMap::from([("access_token".into(), "synthetic-access".into())]);
    assert!(!refresh_oauth_bundle(&mut oauth, false).await.unwrap());
    assert!(matches!(
        refresh_oauth_bundle(&mut oauth, true).await,
        Err(AgentMcpError::OauthReconnectRequired)
    ));
    oauth.insert("expires_at_unix".into(), "1".into());
    assert!(matches!(
        refresh_oauth_bundle(&mut oauth, false).await,
        Err(AgentMcpError::OauthReconnectRequired)
    ));
}

#[tokio::test]
async fn credential_endpoints_do_not_follow_redirects() {
    let (sink, sink_origin) = listener().await;
    let (listener, origin) = listener().await;
    let fixture = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            stream.write_all(format!("HTTP/1.1 302 Found\r\nLocation: {sink_origin}/capture\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(100), sink.accept())
                .await
                .is_err()
        );
    });
    let mut auth = vercel_metadata();
    auth.registration_endpoint = Some(format!("{origin}/register"));
    auth.token_endpoint = format!("{origin}/token");
    assert!(
        register_oauth_client(&auth, "http://127.0.0.1:45678/callback")
            .await
            .is_err()
    );
    assert!(exchange_oauth_code(
        &auth,
        "synthetic-client",
        None,
        "synthetic-code",
        "synthetic-verifier",
        "http://127.0.0.1:45678/callback",
        "synthetic-resource"
    )
    .await
    .is_err());
    let mut oauth = token_map(&origin);
    assert!(refresh_oauth_bundle(&mut oauth, true).await.is_err());
    fixture.await.unwrap();
}

#[tokio::test]
async fn provider_registration_rejection_preserves_safe_actionable_code() {
    let (listener, origin) = listener().await;
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        reply(
            &mut stream,
            "403 Forbidden",
            &json!({"error":"unauthorized_client","error_description":"synthetic-secret"}),
        )
        .await;
    });
    let mut auth = vercel_metadata();
    auth.registration_endpoint = Some(format!("{origin}/register"));
    let result = register_oauth_client(&auth, "http://127.0.0.1:45678/callback").await;
    let error = match result {
        Ok(_) => panic!("registration should fail"),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        AgentMcpError::OauthProviderRejected("unauthorized_client")
    ));
    assert!(!error.to_string().contains("synthetic-secret"));
    fixture.await.unwrap();
}

#[tokio::test]
async fn failed_remote_revocation_is_distinct_from_unavailable_endpoint() {
    assert_eq!(revoke_oauth_bundle(&BTreeMap::new()).await, None);
    let (listener, origin) = listener().await;
    let fixture = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let (_, body) = read_request(&mut stream).await;
        let form: BTreeMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(form["token"], "synthetic-old-refresh");
        reply(&mut stream, "503 Service Unavailable", &json!({})).await;
    });
    let mut oauth = token_map(&origin);
    oauth.insert("revocation_endpoint".into(), format!("{origin}/revoke"));
    assert_eq!(revoke_oauth_bundle(&oauth).await, Some(false));
    fixture.await.unwrap();
}

#[test]
fn token_type_and_url_validation_reject_unsupported_contracts() {
    for invalid_modes in [Value::Null, json!("query"), json!(["query", null])] {
        let metadata = json!({"issuer":"https://identity.example","authorization_endpoint":"https://identity.example/authorize","token_endpoint":"https://identity.example/token","response_types_supported":["code"],"code_challenge_methods_supported":["S256"],"response_modes_supported":invalid_modes});
        assert!(serde_json::from_value::<OAuthAuthorizationServerMetadata>(metadata).is_err());
    }
    for raw in [
        "http://remote.example/token",
        "https://user:secret@identity.example/token",
        "https://identity.example/token#fragment",
    ] {
        assert!(secure_oauth_url(raw).is_err());
    }
    for response in [
        json!({"access_token":""}),
        json!({"access_token":"synthetic","token_type":"MAC"}),
        json!({"access_token":"synthetic","refresh_token":""}),
    ] {
        let tokens = serde_json::from_value::<OAuthTokenResponse>(response).unwrap();
        assert!(validate_tokens(&tokens).is_err());
    }
}
