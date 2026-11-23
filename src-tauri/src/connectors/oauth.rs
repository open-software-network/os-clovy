//! Google native-app OAuth for private connectors, plus the provider-neutral
//! loopback primitive ([`loopback_authorize`]) every connector's OAuth flow
//! is built on.
//!
//! PKCE (S256) + loopback redirect on an ephemeral 127.0.0.1 port, for BOTH
//! debug and release builds: Google desktop-app clients use the loopback
//! flow, not a custom URI scheme. Google requires the Desktop client's
//! `client_secret` at its token endpoint even though an installed application
//! cannot keep that credential confidential; PKCE remains the protection for
//! an intercepted authorization code.
//! [`loopback_authorize`] owns the PKCE/CSRF minting, the
//! listener, and the browser handoff; [`authorize`] is Google's thin wrapper
//! around it and owns Google's own auth-URL shape, token exchange, and email
//! resolution. `linear::authorize` is the same kind of thin wrapper; neither
//! touches the shared listener code.
//!
//! NEVER log, print, or serialize tokens (or authorization codes) into
//! errors. Error messages carry stable codes and short human text only.

use crate::domain::types::AppError;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    sync::{LazyLock, OnceLock},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use zeroize::{Zeroize, ZeroizeOnDrop};

const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";
const USERINFO_ENDPOINT: &str = "https://openidconnect.googleapis.com/v1/userinfo";
/// How long the whole connect handoff (browser consent + callback) may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(300);
const SOCKET_READ_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// Total refresh attempts (1 initial + retries) on transient upstream
/// failures; definitive rejections (invalid_grant) never retry.
pub(crate) const REFRESH_MAX_ATTEMPTS: usize = 3;
pub(crate) const REFRESH_RETRY_BACKOFF: Duration = Duration::from_millis(300);

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub(crate) fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        crate::http_client::build_client(
            crate::http_client::base_client_builder().timeout(HTTP_TIMEOUT),
            "connectors",
        )
    })
}

#[derive(Default)]
struct ConnectFlowState {
    active: bool,
    canceled: bool,
    completed: bool,
    cancel_sender: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Operation-wide cancellation state for an in-flight connect.
///
/// Cancellation stays latched from command start through token exchange,
/// identity resolution, and persistence. The sender only wakes whichever
/// browser/device waiter is active; clearing it must not clear the latch.
#[derive(Default)]
pub struct ConnectFlow {
    state: std::sync::Mutex<ConnectFlowState>,
}

pub struct ConnectOperation<'a> {
    flow: &'a ConnectFlow,
}

impl ConnectFlow {
    pub(crate) fn begin_operation(&self) -> Result<ConnectOperation<'_>, AppError> {
        if let Ok(mut state) = self.state.lock() {
            if state.active {
                return Err(AppError::new(
                    "connector_connect_in_progress",
                    "Another connector is already waiting for authorization.",
                ));
            }
            state.active = true;
            state.canceled = false;
            state.completed = false;
            state.cancel_sender = None;
        }
        Ok(ConnectOperation { flow: self })
    }

    pub fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            if !state.active || state.completed {
                return;
            }
            state.canceled = true;
            if let Some(sender) = state.cancel_sender.take() {
                let _ = sender.send(());
            }
        }
    }

    pub(crate) fn ensure_not_canceled(&self, provider_label: &str) -> Result<(), AppError> {
        let canceled = self
            .state
            .lock()
            .map(|state| state.canceled)
            .unwrap_or(false);
        if canceled {
            return Err(AppError::new(
                "connector_connect_canceled",
                format!("Connecting to {provider_label} was canceled."),
            ));
        }
        Ok(())
    }

    /// Linearize successful finalization against Cancel. Once this succeeds,
    /// a later cancel is a no-op because persistence has already completed;
    /// when cancellation won the race, the caller still owns the active
    /// operation and can restore its snapshots before returning.
    pub(crate) fn complete_operation(&self, provider_label: &str) -> Result<(), AppError> {
        let mut state = self.state.lock().map_err(|_| {
            AppError::new(
                "connector_connect_state_failed",
                "Could not finalize the connector authorization state.",
            )
        })?;
        if state.canceled {
            return Err(AppError::new(
                "connector_connect_canceled",
                format!("Connecting to {provider_label} was canceled."),
            ));
        }
        state.completed = true;
        state.cancel_sender = None;
        Ok(())
    }

    /// Register a cancel sender for an in-flight device-flow poll. Sibling
    /// modules (e.g. `github`) call this so the shared cancel signal works
    /// without accessing the private `cancel` field directly.
    pub(super) fn register_cancel_sender(&self, tx: tokio::sync::oneshot::Sender<()>) {
        if let Ok(mut state) = self.state.lock() {
            if state.canceled {
                let _ = tx.send(());
            } else {
                state.cancel_sender = Some(tx);
            }
        }
    }

    /// Clear only the waiter after a device-flow poll completes or is aborted.
    /// The operation-wide cancellation latch remains set until the command
    /// finishes, so later token and persistence stages still observe it.
    pub(super) fn clear_cancel_sender(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.cancel_sender = None;
        }
    }
}

impl Drop for ConnectOperation<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.flow.state.lock() {
            state.active = false;
            state.completed = false;
            state.cancel_sender = None;
        }
    }
}

/// Token endpoint response. Secret fields zeroize on drop.
#[derive(Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct GoogleTokenResponse {
    pub access_token: String,
    /// Absent on refresh (unless Google rotates) and on scope escalation for
    /// an already-connected account; callers keep the existing one then.
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[zeroize(skip)]
    pub expires_in: i64,
    /// Space-joined scope set actually granted.
    #[serde(default)]
    #[zeroize(skip)]
    pub scope: Option<String>,
    #[serde(default)]
    pub id_token: Option<String>,
}

/// Outcome of the full browser handoff: granted tokens plus the account
/// email that keys the keychain entry and the DB index row.
pub struct AuthorizedGrant {
    pub tokens: GoogleTokenResponse,
    pub email: String,
}

#[derive(Deserialize)]
struct TokenErrorBody {
    #[serde(default)]
    error: Option<String>,
}

pub enum RefreshOutcome {
    Refreshed(GoogleTokenResponse),
    /// Definitive: the grant was revoked or expired. The account must be
    /// reconnected; retrying cannot help.
    InvalidGrant,
    /// Upstream wobble (5xx, 429, network error): worth a bounded retry.
    Transient,
}

/// Outcome of the provider-neutral PKCE + loopback handoff: the
/// authorization code, its PKCE verifier, and the exact `redirect_uri` the
/// code was issued for (the token endpoint requires it echoed back
/// byte-identical). The caller still owns the token exchange and identity
/// resolution, which differ per provider (Google's OIDC id_token/userinfo;
/// Linear's own token endpoint + viewer query).
pub(crate) struct LoopbackAuthorization {
    pub code: String,
    pub verifier: String,
    pub redirect_uri: String,
}

/// How the loopback listener picks its port. Google ignores the loopback
/// port when matching redirect URIs (RFC 8252 native-app behavior), so an
/// OS-assigned ephemeral port works. Linear matches the registered callback
/// URL exactly, port included, so its listener must bind one of the fixed
/// ports whose callback URLs are registered on the OAuth application.
pub(crate) enum LoopbackPort {
    Ephemeral,
    Candidates(Vec<u16>),
}

/// Bind the loopback listener per the port strategy. For candidates, the
/// first free port wins; every candidate being taken is reported with the
/// full list so the user can see which local ports the connect needs.
pub(crate) async fn bind_loopback(port: &LoopbackPort) -> Result<TcpListener, AppError> {
    let bind_failed = |detail: String| {
        AppError::new(
            "connector_loopback_bind_failed",
            format!("Could not start the local connect listener: {detail}"),
        )
    };
    match port {
        LoopbackPort::Ephemeral => TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| bind_failed(e.to_string())),
        LoopbackPort::Candidates(ports) => {
            for &candidate in ports {
                if let Ok(listener) = TcpListener::bind(("127.0.0.1", candidate)).await {
                    return Ok(listener);
                }
            }
            let list = ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            Err(bind_failed(format!(
                "ports {list} are all in use on this Mac"
            )))
        }
    }
}

/// Provider-neutral half of a native-app OAuth connect: mint PKCE + CSRF
/// state, bind a 127.0.0.1 listener per the port strategy, ask
/// `build_auth_url` to assemble the provider's consent URL from the
/// resulting `(redirect_uri, code_challenge, state)`, open it in the system
/// browser, and race the loopback callback against the connect timeout and
/// the flow's cancel signal. `provider_label` names the provider in the
/// timeout/cancel/denial copy shown to the user (e.g. "Google"), so each
/// provider's wrapper keeps producing its own exact error text.
pub(crate) async fn loopback_authorize(
    flow: &ConnectFlow,
    provider_label: &str,
    port: LoopbackPort,
    build_auth_url: impl FnOnce(&str, &str, &str) -> String,
) -> Result<LoopbackAuthorization, AppError> {
    let (verifier, challenge) = pkce();
    let csrf = random_b64url(24);

    let listener = bind_loopback(&port).await?;
    let port = listener
        .local_addr()
        .map_err(|e| AppError::new("connector_loopback_bind_failed", e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let auth_url = build_auth_url(&redirect_uri, &challenge, &csrf);

    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel::<()>();
    flow.register_cancel_sender(cancel_tx);
    if let Err(error) = flow.ensure_not_canceled(provider_label) {
        flow.clear_cancel_sender();
        return Err(error);
    }
    if let Err(error) = crate::system_browser::open_in_browser(&auth_url) {
        flow.clear_cancel_sender();
        return Err(error);
    }
    let outcome = tokio::select! {
        result = tokio::time::timeout(CONNECT_TIMEOUT, await_callback(&listener, &csrf, provider_label)) => {
            result.unwrap_or_else(|_| {
                Err(AppError::new(
                    "connector_connect_timed_out",
                    format!("Connecting to {provider_label} timed out. Please try again."),
                ))
            })
        }
        _ = cancel_rx => Err(AppError::new(
            "connector_connect_canceled",
            format!("Connecting to {provider_label} was canceled."),
        )),
    };
    flow.clear_cancel_sender();
    let code = outcome?;
    flow.ensure_not_canceled(provider_label)?;

    Ok(LoopbackAuthorization {
        code,
        verifier,
        redirect_uri,
    })
}

/// Run the full Google authorization handoff: open the consent screen in the
/// default browser, wait on a loopback listener for the redirect, exchange
/// the code, and resolve the account email. A thin Google-specific wrapper
/// over [`loopback_authorize`]: it supplies Google's auth URL and owns the
/// token exchange + email resolution the shared primitive knows nothing
/// about.
pub async fn authorize(
    flow: &ConnectFlow,
    client_id: &str,
    client_secret: &str,
    scopes: &[&str],
    login_hint: Option<&str>,
) -> Result<AuthorizedGrant, AppError> {
    let authorization = loopback_authorize(
        flow,
        "Google",
        LoopbackPort::Ephemeral,
        |redirect_uri, code_challenge, state| {
            build_auth_url(
                client_id,
                redirect_uri,
                scopes,
                code_challenge,
                state,
                login_hint,
            )
        },
    )
    .await?;

    let tokens = exchange_code(
        client_id,
        client_secret,
        &authorization.code,
        &authorization.verifier,
        &authorization.redirect_uri,
    )
    .await?;
    let email = resolve_email(&tokens).await?;
    Ok(AuthorizedGrant { tokens, email })
}

fn build_auth_url(
    client_id: &str,
    redirect_uri: &str,
    scopes: &[&str],
    code_challenge: &str,
    state: &str,
    login_hint: Option<&str>,
) -> String {
    let mut url = format!(
        "{AUTH_ENDPOINT}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &code_challenge={}&code_challenge_method=S256&state={}\
         &access_type=offline&prompt=consent&include_granted_scopes=true",
        urlencoding::encode(client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(&scopes.join(" ")),
        urlencoding::encode(code_challenge),
        urlencoding::encode(state),
    );
    // Incremental scope escalation on an existing account: pre-select the
    // account so the user consents for the right identity.
    if let Some(hint) = login_hint.map(str::trim).filter(|hint| !hint.is_empty()) {
        url.push_str("&login_hint=");
        url.push_str(&urlencoding::encode(hint));
    }
    url
}

// Branded loopback success page, shared by every loopback flow (the connectors
// here). Self-contained: the loopback
// origin cannot reach the app's bundled assets, so the two text faces the page
// uses are embedded as data: URIs — the listener answers exactly one request,
// so a follow-up fetch for a font file would find the port already closed —
// and the design tokens from src/styles/tokens.css are baked at their sage
// defaults (the page can't read the runtime accent, and sage is the fixed
// brand identity the app icon and marks use). Mirrors the in-app welcome /
// sign-in surface: warm gradient field, the Clovy mark glyph, ok-tone
// status pill, serif display title. Follows the system light/dark preference,
// since the page can't read the app's theme setting either.
const DIATYPE_REGULAR_WOFF2: &[u8] = include_bytes!("../../../public/ABCDiatype-Regular.woff2");
const MARTINA_PLANTIJN_WOFF2: &[u8] =
    include_bytes!("../../../public/martina-plantijn-light.woff2");

const SUCCESS_TEMPLATE: &str = r##"<!doctype html>
<html lang=en>
<meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1">
<title>Clovy</title>
<style>
  @font-face{font-family:"ABC Diatype";src:url(data:font/woff2;base64,%%DIATYPE%%) format("woff2");font-weight:400;font-style:normal}
  @font-face{font-family:"Martina Plantijn";src:url(data:font/woff2;base64,%%MARTINA%%) format("woff2");font-weight:300 400;font-style:normal}
  :root{--brand:#3f812f;--brand-wash:#5a7c56;--clovy-lime-top:#eefe92;--clovy-lime:#c4f979;--clovy-pine:#183d2f;--clovy-tile-top:#10271d;--clovy-tile-bottom:#020b08;--background:color-mix(in oklch,oklch(95.13% 0.0015 84.59),var(--brand-wash) 3%);--foreground:oklch(27.24% 0.0015 84.59);--card:color-mix(in oklch,oklch(100% 0 none),var(--brand-wash) 2%);--muted-foreground:oklch(55.8% 0.0015 84.59);--success:oklch(48% 0.12 150);--warm-soft:color-mix(in oklch,var(--brand) 16%,var(--card))}
  @media (prefers-color-scheme:dark){:root{--background:color-mix(in oklch,oklch(16.5% 0.0015 84.59),var(--brand-wash) 6%);--foreground:oklch(98.5% 0.0015 84.59);--card:color-mix(in oklch,oklch(19.5% 0.0015 84.59),var(--brand-wash) 6%);--muted-foreground:oklch(70.9% 0.0015 84.59);--success:oklch(72% 0.14 150);--warm-soft:color-mix(in oklch,var(--brand) 30%,var(--card))}}
  *{box-sizing:border-box}
  html,body{height:100%}
  body{margin:0;display:grid;place-items:center;padding:24px;background:linear-gradient(180deg,color-mix(in oklch,var(--background) 78%,var(--warm-soft)) 0%,var(--background) 52%,color-mix(in oklch,var(--background) 92%,var(--warm-soft)) 100%);color:var(--foreground);font-family:"ABC Diatype",ui-sans-serif,system-ui,-apple-system,BlinkMacSystemFont,"Segoe UI",sans-serif;font-size:13px;-webkit-font-smoothing:antialiased;text-rendering:optimizeLegibility}
  .stack{display:flex;flex-direction:column;align-items:center;gap:10px;width:100%;max-width:340px;text-align:center}
  .mark{display:grid;place-items:center;width:64px;height:64px;margin-bottom:6px;border-radius:14px;background:linear-gradient(180deg,var(--clovy-tile-top),var(--clovy-tile-bottom));box-shadow:inset 0 0 0 1px color-mix(in oklch,var(--clovy-lime-top) 12%,transparent),0 8px 24px color-mix(in oklch,var(--clovy-tile-bottom) 20%,transparent)}
  .mark svg{display:block;width:62%;height:auto;filter:drop-shadow(0 1px 2px color-mix(in oklch,var(--clovy-tile-bottom) 72%,transparent))}
  .status-pill{display:inline-flex;align-items:center;padding:1px 6px;border-radius:6px;background:color-mix(in oklch,var(--success) 14%,transparent);color:var(--success);font-size:11px;font-weight:400;line-height:1.4}
  .title{margin:0;font-family:"Martina Plantijn","Iowan Old Style",Georgia,serif;font-size:30px;font-weight:400;line-height:1.2;letter-spacing:0;text-wrap:balance}
  .sub{margin:0;font-size:14px;line-height:1.5;color:var(--muted-foreground)}
</style>
<body>
  <main class=stack>
    <span class=mark aria-hidden=true><svg viewBox="0 0 257 264" fill=none><defs><linearGradient id=clovy-gradient x1=128.5 x2=128.5 y1=0 y2=264 gradientUnits=userSpaceOnUse><stop stop-color="var(--clovy-lime-top)"/><stop offset=1 stop-color="var(--clovy-lime)"/></linearGradient></defs><path fill-rule=evenodd clip-rule=evenodd fill="url(#clovy-gradient)" d="M99.8104 0.0262613C138.071 -1.09911 151.338 34.147 150.906 66.788C167.086 45.9839 192.628 28.1589 220.16 38.9501C241.444 47.2931 249.633 72.6964 240.35 92.9462C233.01 108.96 220.258 118.226 204.12 124.622C227.262 127.441 256.1 138.358 256.872 166.01C257.135 174.337 253.911 182.401 247.976 188.255C239.398 196.79 229.401 198.063 218.1 198.198C218.809 203.806 219.436 208.075 218.173 213.732C216.633 220.792 212.285 226.916 206.129 230.7C199.246 234.9 190.188 235.879 182.506 233.804C160.618 227.878 149.874 210.258 139.249 192.639C141.827 211.271 139.744 230.327 130.786 247.389C124.994 257.838 113.127 266.907 102.004 262.545C99.7278 261.652 98.0027 260.037 97.3554 257.294C95.4185 249.082 101.913 246.2 106.112 241.234C118.643 226.414 122.536 210.503 122.279 193.751C117.643 206.759 112.835 215.471 101.8 227.467C78.3712 252.945 34.7107 245.56 34.2548 206.465C34.1017 193.319 42.7063 179.94 52.4725 170.682C30.5596 169.39 9.31687 162.366 1.8827 139.435C-1.39788 128.962 -0.361542 117.612 4.75966 107.904C9.57541 98.6542 17.9531 91.7675 27.9599 88.8339C45.4751 83.5195 62.0831 89.9152 77.4052 98.0311C58.417 71.6194 44.7798 31.7735 77.2987 7.72646C83.6764 3.01084 91.9867 0.92448 99.8104 0.0262613ZM107.627 109C100.101 109 93.9999 119.074 93.9999 131.5C93.9999 137.007 95.1986 142.053 97.1884 145.964C98.1705 147.893 100.792 147.568 102.395 146.113C103.87 144.775 105.621 144 107.5 144C109.446 144 111.256 144.832 112.763 146.259C114.334 147.747 116.946 148.106 117.948 146.188C120.008 142.247 121.254 137.114 121.254 131.5C121.254 119.074 115.153 109 107.627 109ZM147.627 109C140.101 109 134 119.074 134 131.5C134 137.007 135.199 142.053 137.188 145.964C138.17 147.893 140.792 147.568 142.395 146.113C143.87 144.775 145.621 144 147.5 144C149.446 144 151.256 144.832 152.763 146.259C154.334 147.747 156.946 148.106 157.948 146.188C160.008 142.247 161.254 137.114 161.254 131.5C161.254 119.074 155.153 109 147.627 109Z"/></svg></span>
    <span class=status-pill>%%PILL%%</span>
    <h1 class=title>%%TITLE%%</h1>
    <p class=sub>%%SUB%%</p>
  </main>
</body>
</html>"##;

/// Render the branded loopback success page with flow-specific copy. The
/// arguments are trusted string literals from this crate, not user input —
/// they land in the HTML unescaped.
pub(crate) fn success_page(pill: &str, title: &str, sub: &str) -> String {
    SUCCESS_TEMPLATE
        .replace("%%DIATYPE%%", &STANDARD.encode(DIATYPE_REGULAR_WOFF2))
        .replace("%%MARTINA%%", &STANDARD.encode(MARTINA_PLANTIJN_WOFF2))
        .replace("%%PILL%%", pill)
        .replace("%%TITLE%%", title)
        .replace("%%SUB%%", sub)
}

pub(crate) static SUCCESS_BODY: LazyLock<String> = LazyLock::new(|| {
    success_page(
        "Connected",
        "You're connected",
        "You can close this tab and return to Clovy.",
    )
});

/// Accept connections until one hits `/callback` with a matching state.
/// Every per-socket read is bounded so a slow client on the loopback port
/// cannot stall the listener for the full connect timeout. `provider_label`
/// names the provider in the denial/missing-code error copy.
pub(crate) async fn await_callback(
    listener: &TcpListener,
    expected_state: &str,
    provider_label: &str,
) -> Result<String, AppError> {
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|e| AppError::new("connector_loopback_accept_failed", e.to_string()))?;

        let mut buf = [0u8; 4096];
        let n = match tokio::time::timeout(SOCKET_READ_TIMEOUT, stream.read(&mut buf)).await {
            Ok(Ok(n)) => n,
            Ok(Err(_)) | Err(_) => continue,
        };
        let request = String::from_utf8_lossy(&buf[..n]);
        let path = request
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap_or("");

        if !is_loopback_callback_path(path) {
            write_http(&mut stream, "404 Not Found", "Not found").await;
            continue;
        }

        match validate_callback(path, expected_state) {
            CallbackOutcome::Ignore => {
                write_http(&mut stream, "400 Bad Request", "Invalid connect callback").await;
                continue;
            }
            CallbackOutcome::Rejected(error_code) => {
                write_http(&mut stream, "200 OK", "You can close this tab.").await;
                let mut error = if error_code == "access_denied" {
                    AppError::new(
                        "connector_connect_denied",
                        format!("{provider_label} access was declined."),
                    )
                } else {
                    AppError::new(
                        "connector_authorization_rejected",
                        format!(
                            "{provider_label} could not complete authorization ({error_code})."
                        ),
                    )
                };
                error.details = Some(serde_json::json!({"oauthError":error_code}));
                return Err(error);
            }
            CallbackOutcome::MissingCode => {
                write_http(&mut stream, "400 Bad Request", "Missing authorization code").await;
                return Err(AppError::new(
                    "connector_missing_code",
                    format!("{provider_label}'s response was missing an authorization code."),
                ));
            }
            CallbackOutcome::Code(code) => {
                write_http(&mut stream, "200 OK", &SUCCESS_BODY).await;
                return Ok(code);
            }
        }
    }
}

fn is_loopback_callback_path(path: &str) -> bool {
    path.split_once('?').map_or(path, |(path, _query)| path) == "/callback"
}

async fn write_http(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.flush().await;
}

enum CallbackOutcome {
    /// Wrong or missing state: not our callback, keep waiting.
    Ignore,
    /// A provider error with valid state; only a known error word survives.
    Rejected(&'static str),
    MissingCode,
    Code(String),
}

fn parse_callback_query(path: &str) -> (Option<String>, Option<String>, Option<String>) {
    let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        let decoded = urlencoding::decode(value)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| value.to_string());
        match key {
            "code" => code = Some(decoded),
            "state" => state = Some(decoded),
            "error" => error = Some(decoded),
            _ => {}
        }
    }
    (code, state, error)
}

fn validate_callback(path: &str, expected_state: &str) -> CallbackOutcome {
    let (code, state, error) = parse_callback_query(path);
    if state.as_deref() != Some(expected_state) {
        return CallbackOutcome::Ignore;
    }
    if let Some(error) = error {
        return CallbackOutcome::Rejected(classify_callback_error(&error));
    }
    code.map_or(CallbackOutcome::MissingCode, CallbackOutcome::Code)
}

fn classify_callback_error(error: &str) -> &'static str {
    match error {
        "access_denied" => "access_denied",
        "unauthorized_client" => "unauthorized_client",
        "invalid_client" => "invalid_client",
        "invalid_request" => "invalid_request",
        "unsupported_response_type" => "unsupported_response_type",
        "invalid_scope" => "invalid_scope",
        "server_error" => "server_error",
        "temporarily_unavailable" => "temporarily_unavailable",
        _ => "unknown_oauth_error",
    }
}

/// The shared loopback primitive's safe provider-code contract. Callers keep
/// AppError as their return type; free-form query values never cross it.
pub(crate) fn oauth_callback_error_code(error: &AppError) -> Option<&'static str> {
    match error.code.as_str() {
        "connector_connect_denied" => Some("access_denied"),
        "connector_authorization_rejected" => Some(classify_callback_error(
            error
                .details
                .as_ref()
                .and_then(|details| details.get("oauthError"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
        )),
        _ => None,
    }
}

/// Exchange the authorization code for tokens. Google requires the Desktop
/// credential's `client_secret`; PKCE independently proves possession of the
/// authorization request's verifier.
async fn exchange_code(
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<GoogleTokenResponse, AppError> {
    let response = http_client()
        .post(TOKEN_ENDPOINT)
        .form(&authorization_code_form(
            client_id,
            client_secret,
            code,
            verifier,
            redirect_uri,
        ))
        .send()
        .await
        .map_err(|_| exchange_failed(None))?;
    let status = response.status().as_u16();
    let body = response.text().await.map_err(|_| exchange_failed(None))?;
    if let Ok(tokens) = serde_json::from_str::<GoogleTokenResponse>(&body) {
        if !tokens.access_token.is_empty() {
            return Ok(tokens);
        }
    }
    // Never echo the body: it could carry partial token material. Surface
    // the OAuth error code word only.
    let error_code = serde_json::from_str::<TokenErrorBody>(&body)
        .ok()
        .and_then(|body| body.error);
    tracing::warn!(status, error_code = ?error_code, "google token exchange failed");
    Err(exchange_failed(error_code))
}

fn authorization_code_form<'a>(
    client_id: &'a str,
    client_secret: &'a str,
    code: &'a str,
    verifier: &'a str,
    redirect_uri: &'a str,
) -> [(&'static str, &'a str); 6] {
    [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("redirect_uri", redirect_uri),
    ]
}

fn exchange_failed(error_code: Option<String>) -> AppError {
    let message = match error_code {
        Some(code) => format!("Could not complete the Google connection ({code})."),
        None => "Could not complete the Google connection.".to_string(),
    };
    AppError::new("connector_token_exchange_failed", message)
}

/// One refresh attempt. Classifies invalid_grant (definitive, the account
/// must be reconnected) apart from transient upstream wobble.
pub async fn refresh(client_id: &str, client_secret: &str, refresh_token: &str) -> RefreshOutcome {
    let response = match http_client()
        .post(TOKEN_ENDPOINT)
        .form(&refresh_form(client_id, client_secret, refresh_token))
        .send()
        .await
    {
        Ok(response) => response,
        // No response at all: DNS, connection reset, timeout. Always transient.
        Err(_) => return RefreshOutcome::Transient,
    };
    let status = response.status().as_u16();
    let body = match response.text().await {
        Ok(body) => body,
        Err(_) => return RefreshOutcome::Transient,
    };
    if let Ok(tokens) = serde_json::from_str::<GoogleTokenResponse>(&body) {
        if !tokens.access_token.is_empty() {
            return RefreshOutcome::Refreshed(tokens);
        }
    }
    let error_code = serde_json::from_str::<TokenErrorBody>(&body)
        .ok()
        .and_then(|body| body.error);
    classify_refresh_failure(status, error_code.as_deref())
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

/// `invalid_grant` is the definitive "grant revoked/expired" signal; 5xx and
/// 429 are upstream wobble. Any other parsed OAuth error (e.g.
/// invalid_client) will not heal by retrying either, but is not a revocation:
/// treat it as transient so a config hiccup never flips an account into the
/// reconnect state.
fn classify_refresh_failure(status: u16, error_code: Option<&str>) -> RefreshOutcome {
    // Log status + error code word only; never the body or tokens.
    tracing::warn!(status, error_code = ?error_code, "google token refresh failed");
    match error_code {
        Some("invalid_grant") => RefreshOutcome::InvalidGrant,
        _ => RefreshOutcome::Transient,
    }
}

/// Best-effort revocation of the grant at Google (used by
/// `disconnect(revoke_grant = true)`). Failures are swallowed after logging
/// the HTTP status: local custody removal is the real disconnect.
pub async fn revoke(token: &str) -> bool {
    match http_client()
        .post(REVOKE_ENDPOINT)
        .form(&[("token", token)])
        .send()
        .await
    {
        Ok(response) => {
            let ok = response.status().is_success();
            if !ok {
                tracing::warn!(status = response.status().as_u16(), "google revoke failed");
            }
            ok
        }
        Err(_) => {
            tracing::warn!("google revoke request failed");
            false
        }
    }
}

/// Resolve the account email that keys custody and the DB index. Prefer the
/// id_token email claim (scopes always include `openid email`; decoded
/// without verification — the token came
/// straight from Google over TLS); fall back to the userinfo endpoint.
async fn resolve_email(tokens: &GoogleTokenResponse) -> Result<String, AppError> {
    if let Some(email) = tokens
        .id_token
        .as_deref()
        .and_then(id_token_email)
        .filter(|email| !email.is_empty())
    {
        return Ok(email);
    }
    fetch_userinfo_email(&tokens.access_token).await
}

fn id_token_email(jwt: &str) -> Option<String> {
    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims = serde_json::from_slice::<serde_json::Value>(&decoded).ok()?;
    claims
        .get("email")
        .and_then(serde_json::Value::as_str)
        .map(|email| email.trim().to_ascii_lowercase())
}

#[derive(Deserialize)]
struct UserinfoWire {
    #[serde(default)]
    email: Option<String>,
}

async fn fetch_userinfo_email(access_token: &str) -> Result<String, AppError> {
    let identity_failed = || {
        AppError::new(
            "connector_identity_failed",
            "Could not determine the Google account email.",
        )
    };
    let response = http_client()
        .get(USERINFO_ENDPOINT)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|_| identity_failed())?;
    if !response.status().is_success() {
        return Err(identity_failed());
    }
    let info: UserinfoWire = response.json().await.map_err(|_| identity_failed())?;
    info.email
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| !email.is_empty())
        .ok_or_else(identity_failed)
}

pub(crate) fn pkce() -> (String, String) {
    let verifier = random_b64url(32);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

pub(crate) fn random_b64url(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(&buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn callback_request(port: u16, path: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        stream
            .write_all(format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8(response).unwrap()
    }

    #[tokio::test]
    async fn loopback_ignores_wrong_state_and_accepts_only_matching_callback() {
        let listener = bind_loopback(&LoopbackPort::Ephemeral).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = async move {
            let rejected = callback_request(port, "/callback?code=wrong&state=other").await;
            assert!(rejected.starts_with("HTTP/1.1 400"));
            let accepted = callback_request(port, "/callback?code=fixture&state=expected").await;
            assert!(accepted.starts_with("HTTP/1.1 200"));
        };
        let (code, ()) = tokio::join!(await_callback(&listener, "expected", "Vercel"), client);
        assert_eq!(code.unwrap(), "fixture");
    }

    #[tokio::test]
    async fn loopback_reports_denial_only_for_matching_state() {
        let listener = bind_loopback(&LoopbackPort::Ephemeral).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = async move {
            let rejected =
                callback_request(port, "/callback?error=access_denied&state=other").await;
            assert!(rejected.starts_with("HTTP/1.1 400"));
            let denied =
                callback_request(port, "/callback?error=access_denied&state=expected").await;
            assert!(denied.starts_with("HTTP/1.1 200"));
        };
        let (result, ()) = tokio::join!(await_callback(&listener, "expected", "Vercel"), client);
        assert_eq!(result.unwrap_err().code, "connector_connect_denied");
    }

    #[tokio::test]
    async fn loopback_preserves_safe_provider_error_codes_after_state_and_path_checks() {
        for (provider_error, expected) in [
            ("access_denied", "access_denied"),
            ("unauthorized_client", "unauthorized_client"),
            ("server_error", "server_error"),
            ("temporarily_unavailable", "temporarily_unavailable"),
            ("fixture-private-error-word", "unknown_oauth_error"),
        ] {
            let listener = bind_loopback(&LoopbackPort::Ephemeral).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let client = async move {
                let wrong_path = callback_request(
                    port,
                    "/callback-extra?error=unauthorized_client&state=expected",
                )
                .await;
                assert!(wrong_path.starts_with("HTTP/1.1 404"));
                let wrong_state =
                    callback_request(port, "/callback?error=unauthorized_client&state=other").await;
                assert!(wrong_state.starts_with("HTTP/1.1 400"));
                let response = callback_request(port, &format!(
                    "/callback?error={provider_error}&state=expected&error_description=fixture-private-description&code=fixture-private-code",
                )).await;
                assert!(response.starts_with("HTTP/1.1 200"));
            };
            let (result, ()) =
                tokio::join!(await_callback(&listener, "expected", "Vercel"), client);
            let error = result.unwrap_err();
            assert_eq!(oauth_callback_error_code(&error), Some(expected));
            assert_eq!(error.details.as_ref().unwrap()["oauthError"], expected);
            assert_eq!(
                error.code,
                if expected == "access_denied" {
                    "connector_connect_denied"
                } else {
                    "connector_authorization_rejected"
                }
            );
            let encoded = serde_json::to_string(&error).unwrap();
            for private in [
                "fixture-private-error-word",
                "fixture-private-description",
                "fixture-private-code",
            ] {
                assert!(!encoded.contains(private), "{encoded}");
            }
            if expected != "access_denied" {
                assert!(!error.message.contains("declined"));
            }
        }
    }

    #[test]
    fn success_page_carries_the_clovy_app_tile() {
        let page = success_page("Signed in", "Signed in to Clovy", "Return to the app.");

        assert!(page.contains("<title>Clovy</title>"));
        assert!(page.contains("class=mark"));
        assert!(page.contains("id=clovy-gradient"));
        assert!(page.contains("Signed in to Clovy"));
        assert!(!page.contains("%%TITLE%%"));
    }

    #[test]
    fn cancellation_before_waiter_registration_stays_latched() {
        let flow = ConnectFlow::default();
        let _operation = flow.begin_operation().expect("operation starts");

        flow.cancel();
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
        flow.register_cancel_sender(cancel_tx);
        flow.clear_cancel_sender();

        assert!(cancel_rx.try_recv().is_ok());
        let error = flow
            .ensure_not_canceled("Linear")
            .expect_err("clearing the waiter must not clear cancellation");
        assert_eq!(error.code, "connector_connect_canceled");
    }

    #[test]
    fn completed_operation_resets_cancellation_for_the_next_connect() {
        let flow = ConnectFlow::default();
        {
            let _operation = flow.begin_operation().expect("first operation starts");
            flow.cancel();
            assert!(flow.ensure_not_canceled("Linear").is_err());
            assert!(flow.begin_operation().is_err());
        }

        let _operation = flow.begin_operation().expect("next operation starts");
        assert!(flow.ensure_not_canceled("Linear").is_ok());
    }

    #[test]
    fn completed_operation_wins_over_a_late_cancel() {
        let flow = ConnectFlow::default();
        let _operation = flow.begin_operation().expect("operation starts");

        flow.complete_operation("Linear")
            .expect("finalization succeeds");
        flow.cancel();

        assert!(flow.ensure_not_canceled("Linear").is_ok());
    }

    #[test]
    fn auth_url_carries_native_app_flow_params() {
        let url = build_auth_url(
            "client-123",
            "http://127.0.0.1:49152/callback",
            &[
                "openid",
                "email",
                "https://www.googleapis.com/auth/gmail.readonly",
            ],
            "challenge",
            "csrf-state",
            None,
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        assert!(url.contains("client_id=client-123"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A49152%2Fcallback"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains(
            "scope=openid%20email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.readonly"
        ));
        assert!(url.contains("code_challenge=challenge"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=csrf-state"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("include_granted_scopes=true"));
        assert!(!url.contains("login_hint"));
    }

    #[test]
    fn auth_url_includes_login_hint_for_escalation() {
        let url = build_auth_url(
            "client-123",
            "http://127.0.0.1:49152/callback",
            &["openid", "email"],
            "challenge",
            "csrf-state",
            Some("user@example.com"),
        );
        assert!(url.contains("login_hint=user%40example.com"));
    }

    #[test]
    fn callback_validation_ignores_wrong_state() {
        assert!(matches!(
            validate_callback("/callback?code=bad&state=wrong", "expected"),
            CallbackOutcome::Ignore
        ));
    }

    #[test]
    fn callback_validation_accepts_matching_state() {
        assert!(matches!(
            validate_callback("/callback?code=good&state=expected", "expected"),
            CallbackOutcome::Code(code) if code == "good"
        ));
    }

    #[test]
    fn callback_validation_surfaces_consent_denial() {
        assert!(matches!(
            validate_callback("/callback?error=access_denied&state=expected", "expected"),
            CallbackOutcome::Rejected("access_denied")
        ));
    }

    #[test]
    fn callback_path_rejects_prefix_matches() {
        assert!(is_loopback_callback_path("/callback?code=x&state=y"));
        assert!(!is_loopback_callback_path("/callback-extra?code=x&state=y"));
    }

    #[test]
    fn id_token_email_decodes_payload_claim() {
        let payload = URL_SAFE_NO_PAD
            .encode(r#"{"sub":"123","email":"User@Example.COM","email_verified":true}"#);
        let jwt = format!("header.{payload}.signature");
        assert_eq!(id_token_email(&jwt), Some("user@example.com".to_string()));
        assert_eq!(id_token_email("not-a-jwt"), None);
    }

    #[test]
    fn refresh_failure_classification() {
        assert!(matches!(
            classify_refresh_failure(400, Some("invalid_grant")),
            RefreshOutcome::InvalidGrant
        ));
        assert!(matches!(
            classify_refresh_failure(500, None),
            RefreshOutcome::Transient
        ));
        assert!(matches!(
            classify_refresh_failure(429, Some("rate_limited")),
            RefreshOutcome::Transient
        ));
        assert!(matches!(
            classify_refresh_failure(400, Some("invalid_client")),
            RefreshOutcome::Transient
        ));
    }

    #[test]
    fn token_forms_include_the_google_desktop_client_credential() {
        let exchange = authorization_code_form(
            "desktop-id",
            "desktop-secret",
            "authorization-code",
            "pkce-verifier",
            "http://127.0.0.1:49152/callback",
        );
        assert!(exchange.contains(&("client_id", "desktop-id")));
        assert!(exchange.contains(&("client_secret", "desktop-secret")));
        assert!(exchange.contains(&("code_verifier", "pkce-verifier")));

        let refresh = refresh_form("desktop-id", "desktop-secret", "refresh-token");
        assert!(refresh.contains(&("client_id", "desktop-id")));
        assert!(refresh.contains(&("client_secret", "desktop-secret")));
        assert!(refresh.contains(&("refresh_token", "refresh-token")));
    }

    #[test]
    fn pkce_challenge_is_s256_of_verifier() {
        let (verifier, challenge) = pkce();
        assert_eq!(
            challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
        );
    }
}
