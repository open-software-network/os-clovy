use std::collections::BTreeMap;
use zeroize::Zeroize;

pub(super) fn oauth_mode(metadata: &serde_json::Value) -> bool {
    metadata.get("auth").and_then(serde_json::Value::as_str) == Some("oauth")
        || metadata
            .get("legacyAuth")
            .and_then(serde_json::Value::as_str)
            == Some("oauth")
}

pub(super) fn remove_authorization(headers: &mut BTreeMap<String, String>) {
    headers.retain(|name, value| {
        if name.eq_ignore_ascii_case("authorization") {
            value.zeroize();
            false
        } else {
            true
        }
    });
}

pub(super) fn apply(
    mut request: reqwest::RequestBuilder,
    headers: &BTreeMap<String, String>,
    oauth: &BTreeMap<String, String>,
    has_oauth: bool,
) -> reqwest::RequestBuilder {
    for (key, value) in headers {
        if has_oauth && key.eq_ignore_ascii_case("authorization") {
            continue;
        }
        request = request.header(key, value);
    }
    if has_oauth
        || !headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("authorization"))
    {
        if let Some(token) = oauth
            .get("access_token")
            .filter(|value| !value.trim().is_empty())
        {
            request = request.bearer_auth(token);
        }
    }
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn oauth_transport_sends_new_identity_to_a_real_local_receiver() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let receiver = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut data = Vec::new();
            while !data.windows(4).any(|part| part == b"\r\n\r\n") {
                let mut chunk = [0u8; 1024];
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&chunk[..count]);
            }
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            String::from_utf8(data).unwrap()
        });
        let headers = BTreeMap::from([
            ("aUtHoRiZaTiOn".into(), "Bearer synthetic-account-a".into()),
            ("x-fixture".into(), "retained".into()),
        ]);
        let oauth = BTreeMap::from([("access_token".into(), "synthetic-account-b".into())]);
        let response = apply(
            reqwest::Client::new().get(format!("http://{address}/mcp")),
            &headers,
            &oauth,
            oauth_mode(&serde_json::json!({"legacyAuth":"oauth"})),
        )
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        let received = receiver.join().unwrap().to_ascii_lowercase();
        assert!(received.contains("authorization: bearer synthetic-account-b\r\n"));
        assert!(!received.contains("synthetic-account-a"));
        assert!(received.contains("x-fixture: retained\r\n"));
    }

    #[test]
    fn oauth_uses_new_identity_and_preserves_non_auth_headers() {
        for key in ["Authorization", "aUtHoRiZaTiOn"] {
            let mut headers = BTreeMap::from([
                (key.into(), "Bearer synthetic-account-a".into()),
                ("x-fixture".into(), "retained".into()),
            ]);
            let original = headers.clone();
            let oauth = BTreeMap::from([("access_token".into(), "synthetic-account-b".into())]);
            let request = apply(
                reqwest::Client::new().get("https://example.test"),
                &headers,
                &oauth,
                true,
            )
            .build()
            .unwrap();
            assert_eq!(
                request.headers()["authorization"],
                "Bearer synthetic-account-b"
            );
            assert_eq!(request.headers()["x-fixture"], "retained");
            remove_authorization(&mut headers);
            assert!(!headers
                .keys()
                .any(|name| name.eq_ignore_ascii_case("authorization")));
            assert_eq!(headers["x-fixture"], "retained");
            assert!(original.contains_key(key));
        }
    }

    #[test]
    fn manual_mode_keeps_its_authorization() {
        let headers = BTreeMap::from([("Authorization".into(), "Bearer synthetic-manual".into())]);
        let residual = BTreeMap::from([("scope".into(), "legacy-fixture".into())]);
        let request = apply(
            reqwest::Client::new().get("https://example.test"),
            &headers,
            &residual,
            false,
        )
        .build()
        .unwrap();
        assert_eq!(
            request.headers()["authorization"],
            "Bearer synthetic-manual"
        );
    }

    #[test]
    fn incomplete_oauth_never_falls_back_to_the_old_manual_identity() {
        let headers =
            BTreeMap::from([("authorization".into(), "Bearer synthetic-account-a".into())]);
        let oauth = BTreeMap::from([("client_id".into(), "synthetic-client".into())]);
        let request = apply(
            reqwest::Client::new().get("https://example.test"),
            &headers,
            &oauth,
            true,
        )
        .build()
        .unwrap();
        assert!(!request.headers().contains_key("authorization"));
    }
}
