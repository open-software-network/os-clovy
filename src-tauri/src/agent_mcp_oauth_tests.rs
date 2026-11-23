//! Isolated loopback HTTP fixture exercising the production OAuth helpers.
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn fixture_discovers_registers_and_exchanges_s256_authorization() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
    let fixture_origin = origin.clone();
    let fixture = tokio::spawn(async move {
        let mut seen = Vec::new();
        for _ in 0..4 {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&chunk[..n]);
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
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            let path = request
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let body = request.split_once("\r\n\r\n").unwrap().1.to_owned();
            let response = match path {
                "/.well-known/oauth-protected-resource" => {
                    json!({"resource":format!("{fixture_origin}/"),"authorization_servers":[fixture_origin],"scopes_supported":["openid"]})
                }
                "/.well-known/oauth-authorization-server" => {
                    json!({"issuer":fixture_origin,"authorization_endpoint":format!("{fixture_origin}/authorize"),"token_endpoint":format!("{fixture_origin}/token"),"registration_endpoint":format!("{fixture_origin}/register"),"response_types_supported":["code"],"grant_types_supported":["authorization_code","refresh_token"],"code_challenge_methods_supported":["S256"]})
                }
                "/register" => json!({"client_id":"fixture-client"}),
                "/token" => {
                    json!({"access_token":"fixture-access","refresh_token":"fixture-refresh","expires_in":3600})
                }
                _ => panic!("unexpected fixture route"),
            };
            seen.push((path.to_string(), body));
            let body = response.to_string();
            let wire = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            stream.write_all(wire.as_bytes()).await.unwrap();
        }
        seen
    });
    let (resource, metadata) = discover_oauth_metadata(&format!("{origin}/"), initialize_params())
        .await
        .unwrap();
    let redirect = "http://127.0.0.1:45678/callback";
    let registration = register_oauth_client(&metadata, redirect).await.unwrap();
    let (verifier, challenge) = crate::connectors::oauth::pkce();
    let authorization = oauth_authorization_url(
        &metadata,
        &registration.client_id,
        redirect,
        &challenge,
        "fixture-state",
        &resource.resource,
        "openid",
    )
    .unwrap();
    let parsed = reqwest::Url::parse(&authorization).unwrap();
    let query: std::collections::HashMap<_, _> = parsed.query_pairs().into_owned().collect();
    assert_eq!(
        query.get("state").map(String::as_str),
        Some("fixture-state")
    );
    assert_eq!(
        query.get("code_challenge_method").map(String::as_str),
        Some("S256")
    );
    assert_eq!(query.get("code_challenge"), Some(&challenge));
    let tokens = exchange_oauth_code(
        &metadata,
        &registration.client_id,
        None,
        "fixture-code",
        &verifier,
        redirect,
        &resource.resource,
    )
    .await
    .unwrap();
    assert_eq!(tokens.access_token, "fixture-access");
    let seen = fixture.await.unwrap();
    let registration: Value = serde_json::from_str(&seen[2].1).unwrap();
    assert_eq!(registration["client_name"], "Clovy");
    assert_eq!(registration["token_endpoint_auth_method"], "none");
    let form: std::collections::HashMap<_, _> = url::form_urlencoded::parse(seen[3].1.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(form.get("code_verifier"), Some(&verifier));
    assert_eq!(form.get("redirect_uri").map(String::as_str), Some(redirect));
    assert_eq!(form.get("code").map(String::as_str), Some("fixture-code"));
}
