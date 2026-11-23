//! Test fixtures for GitHub via the GitHub CLI: a fake `gh` on a login PATH
//! and a fake HTTP server in place of `api.github.com`. Nothing here touches
//! the real `gh`, the network, or the token store.

use crate::connectors::github;
use crate::llm::shell_env::LoginEnv;
use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub(crate) const TOKEN: &str = "gho_fake_cli_token_0123456789";
pub(crate) const USER_ID: &str = "4242";
pub(crate) const LOGIN: &str = "octo";
/// A second GitHub user the fake `gh` can switch to.
pub(crate) const OTHER_TOKEN: &str = "gho_fake_cli_token_other_987654";
pub(crate) const OTHER_USER_ID: &str = "5151";
pub(crate) const OTHER_LOGIN: &str = "hubot";

/// A fake `gh`: signed in while `<dir>/token` exists (its content is the
/// active account's token), and every invocation is appended to `<dir>/calls`.
pub(crate) struct FakeGh {
    dir: tempfile::TempDir,
}

impl FakeGh {
    pub(crate) fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("gh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
dir="${0%/*}"
echo "$*" >> "$dir/calls"
case "$1 $2 $3 $4" in
  "auth status --hostname github.com")
    if [ -f "$dir/token" ]; then echo "Logged in to github.com" >&2; exit 0; fi
    echo "You are not logged into any GitHub hosts." >&2; exit 1 ;;
  "auth token --hostname github.com")
    if [ -f "$dir/token" ]; then IFS= read -r token < "$dir/token"; printf '%s\n' "$token"; exit 0; fi
    echo "no oauth token found for github.com" >&2; exit 1 ;;
esac
exit 2
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self { dir }
    }

    pub(crate) fn log_in(&self) {
        self.log_in_with(TOKEN);
    }

    /// `gh auth switch` to the account owning `token`.
    pub(crate) fn log_in_with(&self, token: &str) {
        std::fs::write(self.dir.path().join("token"), token).unwrap();
    }

    pub(crate) fn log_out(&self) {
        let _ = std::fs::remove_file(self.dir.path().join("token"));
    }

    pub(crate) fn calls(&self, subcommand: &str) -> usize {
        std::fs::read_to_string(self.dir.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .filter(|line| line.starts_with(subcommand))
            .count()
    }

    pub(crate) fn env(&self) -> LoginEnv {
        login_env_with_path(self.dir.path())
    }
}

/// The login PATH is only `dir`, so the box's own `gh` is never found; the
/// fake script uses shell builtins alone.
pub(crate) fn login_env_with_path(dir: &Path) -> LoginEnv {
    let mut vars = BTreeMap::new();
    vars.insert("PATH".to_string(), dir.display().to_string());
    vars.insert("HOME".to_string(), dir.display().to_string());
    LoginEnv::from_vars(vars)
}

/// A fake `api.github.com` for this test's thread: `/user`, `/user/repos` and
/// `POST /repos/{owner}/{repo}/issues`, answered only for the two fake
/// tokens. Records `METHOD path` of every request.
pub(crate) struct FakeGithub {
    requests: Arc<Mutex<Vec<String>>>,
    _server: tokio::task::JoinHandle<()>,
}

fn user_for(token: &str) -> Option<(&'static str, &'static str)> {
    match token {
        TOKEN => Some((USER_ID, LOGIN)),
        OTHER_TOKEN => Some((OTHER_USER_ID, OTHER_LOGIN)),
        _ => None,
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = stream.read(&mut buffer).await.unwrap_or(0);
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        let text = String::from_utf8_lossy(&bytes);
        if let Some(end) = text.find("\r\n\r\n") {
            let length = text[..end]
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

impl FakeGithub {
    pub(crate) async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        github::set_test_api_base(Some(base));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = requests.clone();
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let recorded = recorded.clone();
                tokio::spawn(async move {
                    let request = read_request(&mut stream).await;
                    let mut first = request
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .split_whitespace();
                    let method = first.next().unwrap_or_default().to_string();
                    let path = first.next().unwrap_or_default().to_string();
                    let user = request.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        if !name.eq_ignore_ascii_case("authorization") {
                            return None;
                        }
                        user_for(value.trim().strip_prefix("Bearer ")?)
                    });
                    recorded.lock().unwrap().push(format!("{method} {path}"));
                    let (status, body) = match (user, method.as_str()) {
                        (None, _) => ("401 Unauthorized", r#"{"message":"Bad credentials"}"#.to_string()),
                        (Some((id, login)), "GET") if path == "/user" => (
                            "200 OK",
                            format!(r#"{{"id":{id},"login":"{login}","name":"Fake {login}"}}"#),
                        ),
                        (Some(_), "GET") if path.starts_with("/user/repos") => (
                            "200 OK",
                            r#"[{"id":1,"name":"hello","full_name":"octo/hello","private":false,"html_url":"https://github.com/octo/hello","description":null,"default_branch":"main","owner":{"login":"octo"}}]"#
                                .to_string(),
                        ),
                        (Some(_), "POST") if path == "/repos/octo/hello/issues" => (
                            "201 Created",
                            r#"{"number":7,"html_url":"https://github.com/octo/hello/issues/7"}"#.to_string(),
                        ),
                        _ => ("404 Not Found", r#"{"message":"Not Found"}"#.to_string()),
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self {
            requests,
            _server: server,
        }
    }

    /// `METHOD path` of every request, in order.
    pub(crate) fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }

    /// Paths only, for GET-only assertions.
    pub(crate) fn paths(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| {
                request
                    .split_once(' ')
                    .map(|(_, path)| path.to_string())
                    .unwrap_or(request)
            })
            .collect()
    }
}

impl Drop for FakeGithub {
    fn drop(&mut self) {
        github::set_test_api_base(None);
    }
}

pub(crate) async fn repositories() -> crate::db::repositories::Repositories {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("sqlite memory");
    crate::db::migrations::run_migrations(&pool)
        .await
        .expect("migrations");
    crate::db::repositories::Repositories::new(pool)
}
