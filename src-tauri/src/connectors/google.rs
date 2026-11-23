//! Direct Google REST client for the private connectors.
//!
//! This is the ONLY place Google APIs are called: the provider proxy and the
//! trigger daemon call these functions with an access token resolved through
//! `connectors::google_access_token`. Calls go straight from the device to
//! Google; Clovy API is never in the connector data path.
//!
//! Responses are parsed into compact summary structs (unknown fields
//! tolerated) so agent context stays small. On a 401 this module returns the
//! typed `GoogleApiError::Unauthorized` so the caller can refresh the access
//! token (`connectors::force_refresh_google_access_token`) and retry once.
//!
//! Never log or embed tokens in errors; error text carries HTTP status and
//! Google's error message field only.

use crate::domain::types::AppError;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use super::oauth::http_client;

const GMAIL_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const CALENDAR_BASE: &str = "https://www.googleapis.com/calendar/v3";
const API_ERROR_MESSAGE_MAX_LEN: usize = 200;
/// Unread triage expands each message to a metadata summary (one call each), so
/// the count is bounded to keep the tool responsive on a large unread backlog.
const UNREAD_SUMMARY_DEFAULT: u32 = 15;
const UNREAD_SUMMARY_MAX: u32 = 40;

// --- Errors ------------------------------------------------------------------

#[derive(Debug)]
pub enum GoogleApiError {
    /// The access token was rejected. The caller should refresh and retry
    /// once before surfacing an error.
    Unauthorized,
    /// Calendar returned 410 GONE for a sync token: the incremental cursor
    /// is dead and a full resync is required.
    SyncTokenExpired,
    /// Gmail returned 404 for a `startHistoryId`: the history cursor fell out
    /// of the mailbox's retention window and must be reseeded from the profile.
    HistoryCursorExpired,
    /// The caller supplied invalid input (e.g. a header with line breaks).
    InvalidInput(String),
    Api {
        status: u16,
        message: String,
    },
    Network(String),
}

impl From<GoogleApiError> for AppError {
    fn from(error: GoogleApiError) -> Self {
        match error {
            GoogleApiError::Unauthorized => AppError::new(
                "connector_unauthorized",
                "Google rejected the connector credentials.",
            ),
            GoogleApiError::SyncTokenExpired => AppError::new(
                "connector_sync_token_expired",
                "The calendar sync token expired. A full resync is required.",
            ),
            GoogleApiError::HistoryCursorExpired => AppError::new(
                "connector_history_cursor_expired",
                "The Gmail history cursor expired. It must be reseeded from the profile.",
            ),
            GoogleApiError::InvalidInput(message) => {
                AppError::new("connector_invalid_input", message)
            }
            GoogleApiError::Api { status, message } => AppError::new(
                "connector_google_api_error",
                format!("Google API request failed ({status}): {message}"),
            ),
            GoogleApiError::Network(message) => AppError::new("network_error", message),
        }
    }
}

// --- Shared request plumbing ---------------------------------------------------

async fn send_request<T: DeserializeOwned>(
    builder: reqwest::RequestBuilder,
) -> Result<T, GoogleApiError> {
    let response = builder
        .send()
        .await
        .map_err(|e| GoogleApiError::Network(e.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .text()
        .await
        .map_err(|e| GoogleApiError::Network(e.to_string()))?;
    if status == 401 {
        return Err(GoogleApiError::Unauthorized);
    }
    if status == 410 {
        return Err(GoogleApiError::SyncTokenExpired);
    }
    if !(200..300).contains(&status) {
        return Err(GoogleApiError::Api {
            status,
            message: api_error_message(&body),
        });
    }
    serde_json::from_str(&body).map_err(|e| GoogleApiError::Api {
        status,
        message: format!("unexpected response shape: {e}"),
    })
}

/// Extract Google's error message only; never echo the raw body.
fn api_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| crate::http_client::json_error_message(&value, API_ERROR_MESSAGE_MAX_LEN))
        .unwrap_or_else(|| "request failed".to_string())
}

fn gmail_get(access_token: &str, path: &str, query: &[(&str, String)]) -> reqwest::RequestBuilder {
    http_client()
        .get(format!("{GMAIL_BASE}{path}"))
        .bearer_auth(access_token)
        .query(query)
}

// --- Gmail wire structs -------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageListWire {
    #[serde(default)]
    messages: Vec<MessageRefWire>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadListWire {
    #[serde(default)]
    threads: Vec<ThreadRefWire>,
    #[serde(default)]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageRefWire {
    id: String,
    #[serde(default)]
    thread_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadRefWire {
    id: String,
    #[serde(default)]
    snippet: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageWire {
    id: String,
    #[serde(default)]
    thread_id: String,
    #[serde(default)]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    #[serde(default)]
    payload: Option<PayloadWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PayloadWire {
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    filename: Option<String>,
    #[serde(default)]
    headers: Vec<HeaderWire>,
    #[serde(default)]
    body: Option<BodyWire>,
    #[serde(default)]
    parts: Vec<PayloadWire>,
}

#[derive(Deserialize)]
struct HeaderWire {
    name: String,
    value: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BodyWire {
    #[serde(default)]
    data: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadWire {
    id: String,
    #[serde(default)]
    messages: Vec<MessageWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DraftWire {
    id: String,
    #[serde(default)]
    message: Option<MessageRefWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileWire {
    email_address: String,
    #[serde(default)]
    history_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryListWire {
    #[serde(default)]
    history: Vec<HistoryEntryWire>,
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    history_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HistoryEntryWire {
    #[serde(default)]
    messages_added: Vec<MessageAddedWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MessageAddedWire {
    #[serde(default)]
    message: Option<MessageWire>,
}

// --- Gmail public structs -------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageRef {
    pub id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagePage {
    pub messages: Vec<MessageRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

/// Unread inbox triage: compact per-message summaries, not bare ids.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnreadPage {
    pub messages: Vec<EmailSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRef {
    pub id: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadPage {
    pub threads: Vec<ThreadRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

/// Compact per-message summary: enough for triage without pulling bodies
/// into agent context.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailSummary {
    pub id: String,
    pub thread_id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub snippet: String,
    pub date: Option<String>,
    /// The RFC 2822 `Message-ID` header, for use as `in_reply_to` when replying
    /// so the reply threads for recipients (Gmail's `thread_id` alone does not).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rfc_message_id: Option<String>,
    pub label_ids: Vec<String>,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadDetail {
    pub id: String,
    pub messages: Vec<ThreadMessage>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessage {
    pub id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub date: Option<String>,
    /// The RFC 2822 `Message-ID` header. Pass the latest message's value as
    /// `in_reply_to` when replying so recipients' clients thread the reply.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rfc_message_id: Option<String>,
    pub snippet: String,
    /// Plain-text body; only populated by `read_thread` (format=full).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_text: Option<String>,
    pub label_ids: Vec<String>,
    pub unread: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRef {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModifiedMessage {
    pub id: String,
    pub thread_id: String,
    pub label_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailProfile {
    pub email: String,
    /// Starting point for `history_list` deltas.
    pub history_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryDelta {
    pub added: Vec<EmailSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
    /// New cursor to persist in `trigger_cursors`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history_id: Option<String>,
}

/// Outgoing mail for `create_draft` / `send_email`. `in_reply_to` and
/// `references` thread a reply; `thread_id` keeps Gmail's thread grouping.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutgoingEmail {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    pub body_text: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub references: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
}

// --- Gmail functions ------------------------------------------------------------

/// users.messages.list with an optional Gmail search query.
pub async fn list_messages(
    access_token: &str,
    q: Option<&str>,
    max_results: Option<u32>,
    page_token: Option<&str>,
) -> Result<MessagePage, GoogleApiError> {
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(q) = q {
        query.push(("q", q.to_string()));
    }
    if let Some(max) = max_results {
        query.push(("maxResults", max.to_string()));
    }
    if let Some(token) = page_token {
        query.push(("pageToken", token.to_string()));
    }
    let wire: MessageListWire = send_request(gmail_get(access_token, "/messages", &query)).await?;
    Ok(MessagePage {
        messages: wire
            .messages
            .into_iter()
            .map(|m| MessageRef {
                id: m.id,
                thread_id: m.thread_id,
            })
            .collect(),
        next_page_token: wire.next_page_token,
    })
}

/// users.threads.list with an optional Gmail search query.
pub async fn list_threads(
    access_token: &str,
    q: Option<&str>,
    max_results: Option<u32>,
    page_token: Option<&str>,
) -> Result<ThreadPage, GoogleApiError> {
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(q) = q {
        query.push(("q", q.to_string()));
    }
    if let Some(max) = max_results {
        query.push(("maxResults", max.to_string()));
    }
    if let Some(token) = page_token {
        query.push(("pageToken", token.to_string()));
    }
    let wire: ThreadListWire = send_request(gmail_get(access_token, "/threads", &query)).await?;
    Ok(ThreadPage {
        threads: wire
            .threads
            .into_iter()
            .map(|t| ThreadRef {
                id: t.id,
                snippet: t.snippet,
            })
            .collect(),
        next_page_token: wire.next_page_token,
    })
}

/// Unread inbox messages as compact summaries (sender, subject, snippet, and
/// the RFC Message-ID) for triage. Scoped to `in:inbox` so archived-but-unread
/// mail is never surfaced as inbox triage, and expanded to metadata summaries
/// (bounded) rather than the bare ids `list_messages` returns.
pub async fn list_unread(
    access_token: &str,
    max_results: Option<u32>,
    page_token: Option<&str>,
) -> Result<UnreadPage, GoogleApiError> {
    let limit = max_results
        .unwrap_or(UNREAD_SUMMARY_DEFAULT)
        .clamp(1, UNREAD_SUMMARY_MAX);
    let page = list_messages(
        access_token,
        Some("is:unread in:inbox"),
        Some(limit),
        page_token,
    )
    .await?;
    let mut messages = Vec::with_capacity(page.messages.len());
    for message in &page.messages {
        messages.push(get_message(access_token, &message.id).await?);
    }
    Ok(UnreadPage {
        messages,
        next_page_token: page.next_page_token,
    })
}

/// users.messages.get format=metadata: compact summary, headers only.
pub async fn get_message(
    access_token: &str,
    message_id: &str,
) -> Result<EmailSummary, GoogleApiError> {
    let query = metadata_headers_query();
    let wire: MessageWire = send_request(gmail_get(
        access_token,
        &format!("/messages/{message_id}"),
        &query,
    ))
    .await?;
    Ok(email_summary_from_message(wire))
}

/// users.threads.get format=full: summaries plus extracted plain-text
/// bodies. Full bodies enter agent context only through this explicit call.
pub async fn read_thread(
    access_token: &str,
    thread_id: &str,
) -> Result<ThreadDetail, GoogleApiError> {
    let query = vec![("format", "full".to_string())];
    let wire: ThreadWire = send_request(gmail_get(
        access_token,
        &format!("/threads/{thread_id}"),
        &query,
    ))
    .await?;
    Ok(thread_detail_from_wire(wire, true))
}

/// users.drafts.create with an RFC 2822 raw message.
pub async fn create_draft(
    access_token: &str,
    email: &OutgoingEmail,
) -> Result<DraftRef, GoogleApiError> {
    let raw = encode_raw_message(email)?;
    let mut message = serde_json::json!({ "raw": raw });
    if let Some(thread_id) = &email.thread_id {
        message["threadId"] = serde_json::json!(thread_id);
    }
    let wire: DraftWire = send_request(
        http_client()
            .post(format!("{GMAIL_BASE}/drafts"))
            .bearer_auth(access_token)
            .json(&serde_json::json!({ "message": message })),
    )
    .await?;
    Ok(DraftRef {
        id: wire.id,
        message_id: wire.message.map(|m| m.id),
    })
}

/// users.messages.send with an RFC 2822 raw message.
pub async fn send_email(
    access_token: &str,
    email: &OutgoingEmail,
) -> Result<MessageRef, GoogleApiError> {
    let raw = encode_raw_message(email)?;
    let mut body = serde_json::json!({ "raw": raw });
    if let Some(thread_id) = &email.thread_id {
        body["threadId"] = serde_json::json!(thread_id);
    }
    let wire: MessageRefWire = send_request(
        http_client()
            .post(format!("{GMAIL_BASE}/messages/send"))
            .bearer_auth(access_token)
            .json(&body),
    )
    .await?;
    Ok(MessageRef {
        id: wire.id,
        thread_id: wire.thread_id,
    })
}

/// users.messages.modify. Archiving is `remove_label_ids = ["INBOX"]`;
/// marking read is `remove_label_ids = ["UNREAD"]`.
pub async fn modify_labels(
    access_token: &str,
    message_id: &str,
    add_label_ids: &[String],
    remove_label_ids: &[String],
) -> Result<ModifiedMessage, GoogleApiError> {
    let wire: MessageWire = send_request(
        http_client()
            .post(format!("{GMAIL_BASE}/messages/{message_id}/modify"))
            .bearer_auth(access_token)
            .json(&serde_json::json!({
                "addLabelIds": add_label_ids,
                "removeLabelIds": remove_label_ids,
            })),
    )
    .await?;
    Ok(ModifiedMessage {
        id: wire.id,
        thread_id: wire.thread_id,
        label_ids: wire.label_ids,
    })
}

/// users.getProfile: the account email plus the initial historyId cursor.
pub async fn get_profile(access_token: &str) -> Result<GmailProfile, GoogleApiError> {
    let wire: ProfileWire = send_request(gmail_get(access_token, "/profile", &[])).await?;
    Ok(GmailProfile {
        email: wire.email_address,
        history_id: wire.history_id,
    })
}

/// users.history.list deltas since `start_history_id`
/// (historyTypes=messageAdded, labelId=INBOX): the polling primitive for the
/// email_received trigger.
pub async fn history_list(
    access_token: &str,
    start_history_id: &str,
    page_token: Option<&str>,
) -> Result<HistoryDelta, GoogleApiError> {
    let mut query: Vec<(&str, String)> = vec![
        ("startHistoryId", start_history_id.to_string()),
        ("historyTypes", "messageAdded".to_string()),
        ("labelId", "INBOX".to_string()),
    ];
    if let Some(token) = page_token {
        query.push(("pageToken", token.to_string()));
    }
    let wire: HistoryListWire =
        match send_request(gmail_get(access_token, "/history", &query)).await {
            Ok(wire) => wire,
            // A 404 here means the startHistoryId is too old; surface it distinctly
            // so the caller reseeds the cursor instead of retrying the dead one.
            Err(GoogleApiError::Api { status: 404, .. }) => {
                return Err(GoogleApiError::HistoryCursorExpired)
            }
            Err(other) => return Err(other),
        };
    let added = wire
        .history
        .into_iter()
        .flat_map(|entry| entry.messages_added)
        .filter_map(|added| added.message)
        .map(email_summary_from_message)
        .collect();
    Ok(HistoryDelta {
        added,
        next_page_token: wire.next_page_token,
        history_id: wire.history_id,
    })
}

fn metadata_headers_query() -> Vec<(&'static str, String)> {
    vec![
        ("format", "metadata".to_string()),
        ("metadataHeaders", "From".to_string()),
        ("metadataHeaders", "To".to_string()),
        ("metadataHeaders", "Subject".to_string()),
        ("metadataHeaders", "Date".to_string()),
        ("metadataHeaders", "Message-ID".to_string()),
    ]
}

// --- Gmail parsing helpers --------------------------------------------------------

fn header_value(payload: Option<&PayloadWire>, name: &str) -> Option<String> {
    payload?
        .headers
        .iter()
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.clone())
}

fn email_summary_from_message(wire: MessageWire) -> EmailSummary {
    let payload = wire.payload.as_ref();
    EmailSummary {
        from: header_value(payload, "From"),
        to: header_value(payload, "To"),
        subject: header_value(payload, "Subject"),
        date: header_value(payload, "Date"),
        rfc_message_id: header_value(payload, "Message-ID"),
        unread: wire.label_ids.iter().any(|label| label == "UNREAD"),
        id: wire.id,
        thread_id: wire.thread_id,
        snippet: wire.snippet,
        label_ids: wire.label_ids,
    }
}

fn thread_detail_from_wire(wire: ThreadWire, include_bodies: bool) -> ThreadDetail {
    let messages = wire
        .messages
        .into_iter()
        .map(|message| {
            let payload = message.payload.as_ref();
            ThreadMessage {
                from: header_value(payload, "From"),
                to: header_value(payload, "To"),
                subject: header_value(payload, "Subject"),
                date: header_value(payload, "Date"),
                rfc_message_id: header_value(payload, "Message-ID"),
                body_text: if include_bodies {
                    payload.and_then(extract_body_text)
                } else {
                    None
                },
                unread: message.label_ids.iter().any(|label| label == "UNREAD"),
                id: message.id,
                snippet: message.snippet,
                label_ids: message.label_ids,
            }
        })
        .collect();
    ThreadDetail {
        id: wire.id,
        messages,
    }
}

/// Plain-text body extraction: walk payload parts depth first, prefer
/// text/plain; fall back to text/html with tags stripped.
fn extract_body_text(payload: &PayloadWire) -> Option<String> {
    if let Some(text) = find_part_text(payload, "text/plain") {
        return Some(text);
    }
    find_part_text(payload, "text/html").map(|html| strip_html_tags(&html))
}

fn find_part_text(payload: &PayloadWire, mime_type: &str) -> Option<String> {
    let is_match = payload
        .mime_type
        .as_deref()
        .map(|value| value.eq_ignore_ascii_case(mime_type))
        .unwrap_or(false);
    // Attachments can also be text/plain; skip anything with a filename.
    let is_attachment = payload
        .filename
        .as_deref()
        .map(|name| !name.is_empty())
        .unwrap_or(false);
    if is_match && !is_attachment {
        if let Some(data) = payload.body.as_ref().and_then(|body| body.data.as_deref()) {
            if let Ok(bytes) = URL_SAFE_NO_PAD.decode(data.trim_end_matches('=')) {
                return Some(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
    payload
        .parts
        .iter()
        .find_map(|part| find_part_text(part, mime_type))
}

/// Minimal HTML-to-text: drop tags, decode a few common entities, collapse
/// blank runs. Good enough for summaries; not a sanitizer.
fn strip_html_tags(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    let decoded = text
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

// --- MIME builder ----------------------------------------------------------------

/// Build the RFC 2822 message and base64url-encode it for the Gmail `raw`
/// field.
fn encode_raw_message(email: &OutgoingEmail) -> Result<String, GoogleApiError> {
    Ok(URL_SAFE_NO_PAD.encode(build_mime(email)?))
}

/// Assemble an RFC 2822 message. Header injection resistant: recipient and
/// threading header values are rejected if they carry line breaks or other
/// control characters, and the subject is RFC 2047 encoded whenever it is
/// not printable ASCII, so no user-supplied value can smuggle an extra
/// header line. The body is base64 encoded (no line-length pitfalls).
fn build_mime(email: &OutgoingEmail) -> Result<String, GoogleApiError> {
    if email.to.is_empty() {
        return Err(GoogleApiError::InvalidInput(
            "At least one recipient is required.".to_string(),
        ));
    }
    let to = join_addresses("To", &email.to)?;
    let cc = if email.cc.is_empty() {
        None
    } else {
        Some(join_addresses("Cc", &email.cc)?)
    };
    let subject = encode_subject(&email.subject)?;

    let mut mime = String::new();
    mime.push_str(&format!("To: {to}\r\n"));
    if let Some(cc) = cc {
        mime.push_str(&format!("Cc: {cc}\r\n"));
    }
    mime.push_str(&format!("Subject: {subject}\r\n"));
    if let Some(in_reply_to) = email.in_reply_to.as_deref().filter(|v| !v.is_empty()) {
        validate_header_value("In-Reply-To", in_reply_to)?;
        mime.push_str(&format!("In-Reply-To: {in_reply_to}\r\n"));
    }
    if let Some(references) = email.references.as_deref().filter(|v| !v.is_empty()) {
        validate_header_value("References", references)?;
        mime.push_str(&format!("References: {references}\r\n"));
    }
    mime.push_str("MIME-Version: 1.0\r\n");
    mime.push_str("Content-Type: text/plain; charset=UTF-8\r\n");
    mime.push_str("Content-Transfer-Encoding: base64\r\n");
    mime.push_str("\r\n");
    mime.push_str(&wrap_base64(&STANDARD.encode(email.body_text.as_bytes())));
    Ok(mime)
}

fn join_addresses(header: &str, addresses: &[String]) -> Result<String, GoogleApiError> {
    let mut cleaned = Vec::with_capacity(addresses.len());
    for address in addresses {
        let address = address.trim();
        if address.is_empty() {
            continue;
        }
        validate_header_value(header, address)?;
        cleaned.push(address.to_string());
    }
    if cleaned.is_empty() {
        return Err(GoogleApiError::InvalidInput(format!(
            "The {header} field needs at least one address."
        )));
    }
    Ok(cleaned.join(", "))
}

fn validate_header_value(header: &str, value: &str) -> Result<(), GoogleApiError> {
    if value.chars().any(|ch| ch.is_control()) {
        return Err(GoogleApiError::InvalidInput(format!(
            "The {header} field must not contain line breaks or control characters."
        )));
    }
    Ok(())
}

/// Reject raw line breaks outright; encode anything non printable-ASCII as
/// an RFC 2047 encoded word so it is inert as a header value.
fn encode_subject(subject: &str) -> Result<String, GoogleApiError> {
    if subject.contains('\r') || subject.contains('\n') {
        return Err(GoogleApiError::InvalidInput(
            "The subject must not contain line breaks.".to_string(),
        ));
    }
    let printable_ascii = subject
        .chars()
        .all(|ch| ch.is_ascii() && !ch.is_ascii_control());
    if printable_ascii {
        Ok(subject.to_string())
    } else {
        Ok(format!(
            "=?UTF-8?B?{}?=",
            STANDARD.encode(subject.as_bytes())
        ))
    }
}

fn wrap_base64(encoded: &str) -> String {
    let mut wrapped = String::with_capacity(encoded.len() + encoded.len() / 76 * 2 + 2);
    let bytes = encoded.as_bytes();
    for chunk in bytes.chunks(76) {
        // base64 output is always ASCII, so byte chunking is char safe.
        wrapped.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        wrapped.push_str("\r\n");
    }
    wrapped
}

// --- Calendar wire structs ---------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsListWire {
    #[serde(default)]
    items: Vec<EventWire>,
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    next_sync_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventWire {
    id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    html_link: Option<String>,
    #[serde(default)]
    location: Option<String>,
    #[serde(default)]
    start: Option<EventTimeWire>,
    #[serde(default)]
    end: Option<EventTimeWire>,
    #[serde(default)]
    attendees: Vec<AttendeeWire>,
    #[serde(default)]
    organizer: Option<OrganizerWire>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventTimeWire {
    #[serde(default)]
    date_time: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttendeeWire {
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    response_status: Option<String>,
    #[serde(rename = "self", default)]
    is_self: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrganizerWire {
    #[serde(default)]
    email: Option<String>,
}

#[derive(Deserialize)]
struct FreeBusyWire {
    #[serde(default)]
    calendars: std::collections::HashMap<String, FreeBusyCalendarWire>,
}

#[derive(Deserialize)]
struct FreeBusyCalendarWire {
    #[serde(default)]
    busy: Vec<FreeBusyIntervalWire>,
}

#[derive(Deserialize)]
struct FreeBusyIntervalWire {
    start: String,
    end: String,
}

// --- Calendar public structs ---------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSummary {
    pub id: String,
    pub summary: Option<String>,
    /// RFC 3339 dateTime for timed events, YYYY-MM-DD for all-day events.
    pub start: Option<String>,
    pub end: Option<String>,
    pub attendees: Vec<EventAttendee>,
    pub location: Option<String>,
    pub organizer: Option<String>,
    pub html_link: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventAttendee {
    pub email: Option<String>,
    pub response_status: Option<String>,
    #[serde(rename = "self")]
    pub is_self: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventsPage {
    pub items: Vec<EventSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
    /// Cursor for incremental sync; persist in `trigger_cursors`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_sync_token: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ListEventsParams {
    /// Defaults to "primary".
    pub calendar_id: Option<String>,
    pub time_min: Option<String>,
    pub time_max: Option<String>,
    pub max_results: Option<u32>,
    pub page_token: Option<String>,
    /// Incremental sync cursor. When set, the window and ordering params are
    /// omitted (the Calendar API rejects them alongside syncToken). A
    /// `GoogleApiError::SyncTokenExpired` (410) means: drop the cursor and
    /// do a full resync.
    pub sync_token: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BusyInterval {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FreeSlot {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Working-hours parameters for `find_free_slots`. Hours are in the
/// caller's local time, expressed via `utc_offset_minutes`.
#[derive(Debug, Clone, Copy)]
pub struct FreeSlotParams {
    pub time_min: DateTime<Utc>,
    pub time_max: DateTime<Utc>,
    /// Start of the working day, 0-23.
    pub working_start_hour: u32,
    /// End of the working day, 1-24.
    pub working_end_hour: u32,
    pub utc_offset_minutes: i32,
    pub min_slot_minutes: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEvent {
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    /// RFC 3339 with offset, e.g. "2026-07-10T09:00:00-07:00".
    pub start_rfc3339: String,
    pub end_rfc3339: String,
    #[serde(default)]
    pub attendee_emails: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteResponse {
    Accepted,
    Declined,
    Tentative,
}

impl InviteResponse {
    fn as_str(&self) -> &'static str {
        match self {
            InviteResponse::Accepted => "accepted",
            InviteResponse::Declined => "declined",
            InviteResponse::Tentative => "tentative",
        }
    }
}

// --- Calendar functions ---------------------------------------------------------------

fn calendar_id_or_primary(calendar_id: Option<&str>) -> String {
    let id = calendar_id.map(str::trim).unwrap_or("");
    if id.is_empty() {
        "primary".to_string()
    } else {
        urlencoding::encode(id).into_owned()
    }
}

/// events.list. Window mode (timeMin/timeMax, singleEvents, ordered by start
/// time) or incremental sync mode when `sync_token` is set.
pub async fn list_events(
    access_token: &str,
    params: &ListEventsParams,
) -> Result<EventsPage, GoogleApiError> {
    let calendar = calendar_id_or_primary(params.calendar_id.as_deref());
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(sync_token) = &params.sync_token {
        query.push(("syncToken", sync_token.clone()));
    } else {
        query.push(("singleEvents", "true".to_string()));
        query.push(("orderBy", "startTime".to_string()));
        if let Some(time_min) = &params.time_min {
            query.push(("timeMin", time_min.clone()));
        }
        if let Some(time_max) = &params.time_max {
            query.push(("timeMax", time_max.clone()));
        }
    }
    if let Some(max) = params.max_results {
        query.push(("maxResults", max.to_string()));
    }
    if let Some(page_token) = &params.page_token {
        query.push(("pageToken", page_token.clone()));
    }
    let wire: EventsListWire = send_request(
        http_client()
            .get(format!("{CALENDAR_BASE}/calendars/{calendar}/events"))
            .bearer_auth(access_token)
            .query(&query),
    )
    .await?;
    Ok(EventsPage {
        items: wire
            .items
            .into_iter()
            .map(event_summary_from_wire)
            .collect(),
        next_page_token: wire.next_page_token,
        next_sync_token: wire.next_sync_token,
    })
}

pub async fn get_event(
    access_token: &str,
    calendar_id: Option<&str>,
    event_id: &str,
) -> Result<EventSummary, GoogleApiError> {
    let calendar = calendar_id_or_primary(calendar_id);
    let wire: EventWire = send_request(
        http_client()
            .get(format!(
                "{CALENDAR_BASE}/calendars/{calendar}/events/{event_id}"
            ))
            .bearer_auth(access_token),
    )
    .await?;
    Ok(event_summary_from_wire(wire))
}

/// freebusy.query for the primary calendar (or the given calendar ids),
/// merged into one busy list. Feed the result to `find_free_slots`.
pub async fn freebusy(
    access_token: &str,
    time_min: &str,
    time_max: &str,
    calendar_ids: &[String],
) -> Result<Vec<BusyInterval>, GoogleApiError> {
    let items: Vec<serde_json::Value> = if calendar_ids.is_empty() {
        vec![serde_json::json!({ "id": "primary" })]
    } else {
        calendar_ids
            .iter()
            .map(|id| serde_json::json!({ "id": id }))
            .collect()
    };
    let wire: FreeBusyWire = send_request(
        http_client()
            .post(format!("{CALENDAR_BASE}/freeBusy"))
            .bearer_auth(access_token)
            .json(&serde_json::json!({
                "timeMin": time_min,
                "timeMax": time_max,
                "items": items,
            })),
    )
    .await?;
    let mut busy: Vec<BusyInterval> = wire
        .calendars
        .into_values()
        .flat_map(|calendar| calendar.busy)
        .filter_map(|interval| {
            let start = DateTime::parse_from_rfc3339(&interval.start).ok()?;
            let end = DateTime::parse_from_rfc3339(&interval.end).ok()?;
            Some(BusyInterval {
                start: start.with_timezone(&Utc),
                end: end.with_timezone(&Utc),
            })
        })
        .collect();
    busy.sort_by_key(|interval| interval.start);
    Ok(busy)
}

/// Compute free gaps between busy intervals inside the working-hours windows
/// of each day in the range. Pure Rust; the network half is `freebusy`.
pub fn find_free_slots(params: &FreeSlotParams, busy: &[BusyInterval]) -> Vec<FreeSlot> {
    let start_hour = params.working_start_hour.min(23);
    let end_hour = params.working_end_hour.clamp(1, 24);
    if params.time_max <= params.time_min || end_hour <= start_hour {
        return Vec::new();
    }
    let min_slot = ChronoDuration::minutes(params.min_slot_minutes.max(1));
    let merged = merge_busy(busy);
    let offset = ChronoDuration::minutes(params.utc_offset_minutes as i64);

    // Work in shifted-local naive time, then subtract the offset to get UTC.
    let local_min = (params.time_min + offset).naive_utc();
    let local_max = (params.time_max + offset).naive_utc();
    let mut slots = Vec::new();
    let mut day = local_min.date();
    while day <= local_max.date() {
        let Some(window_start_local) = day.and_hms_opt(start_hour, 0, 0) else {
            break;
        };
        let window_end_local = if end_hour == 24 {
            day.succ_opt().and_then(|next| next.and_hms_opt(0, 0, 0))
        } else {
            day.and_hms_opt(end_hour, 0, 0)
        };
        let Some(window_end_local) = window_end_local else {
            break;
        };
        let window_start =
            DateTime::<Utc>::from_naive_utc_and_offset(window_start_local, Utc) - offset;
        let window_end = DateTime::<Utc>::from_naive_utc_and_offset(window_end_local, Utc) - offset;
        let window_start = window_start.max(params.time_min);
        let window_end = window_end.min(params.time_max);
        if window_start < window_end {
            collect_gaps(window_start, window_end, &merged, min_slot, &mut slots);
        }
        let Some(next) = day.succ_opt() else {
            break;
        };
        day = next;
    }
    slots
}

fn merge_busy(busy: &[BusyInterval]) -> Vec<BusyInterval> {
    let mut sorted: Vec<BusyInterval> = busy
        .iter()
        .copied()
        .filter(|interval| interval.end > interval.start)
        .collect();
    sorted.sort_by_key(|interval| interval.start);
    let mut merged: Vec<BusyInterval> = Vec::with_capacity(sorted.len());
    for interval in sorted {
        match merged.last_mut() {
            Some(last) if interval.start <= last.end => {
                last.end = last.end.max(interval.end);
            }
            _ => merged.push(interval),
        }
    }
    merged
}

fn collect_gaps(
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    merged_busy: &[BusyInterval],
    min_slot: ChronoDuration,
    into: &mut Vec<FreeSlot>,
) {
    let mut cursor = window_start;
    for interval in merged_busy {
        if interval.end <= cursor {
            continue;
        }
        if interval.start >= window_end {
            break;
        }
        if interval.start > cursor {
            let gap_end = interval.start.min(window_end);
            if gap_end - cursor >= min_slot {
                into.push(FreeSlot {
                    start: cursor,
                    end: gap_end,
                });
            }
        }
        cursor = cursor.max(interval.end);
        if cursor >= window_end {
            return;
        }
    }
    if window_end - cursor >= min_slot {
        into.push(FreeSlot {
            start: cursor,
            end: window_end,
        });
    }
}

/// events.insert on the given (or primary) calendar.
pub async fn insert_event(
    access_token: &str,
    calendar_id: Option<&str>,
    event: &NewEvent,
) -> Result<EventSummary, GoogleApiError> {
    let calendar = calendar_id_or_primary(calendar_id);
    let attendees: Vec<serde_json::Value> = event
        .attendee_emails
        .iter()
        .map(|email| serde_json::json!({ "email": email }))
        .collect();
    let mut body = serde_json::json!({
        "summary": event.summary,
        "start": { "dateTime": event.start_rfc3339 },
        "end": { "dateTime": event.end_rfc3339 },
    });
    if let Some(description) = &event.description {
        body["description"] = serde_json::json!(description);
    }
    if let Some(location) = &event.location {
        body["location"] = serde_json::json!(location);
    }
    if !attendees.is_empty() {
        body["attendees"] = serde_json::json!(attendees);
    }
    let wire: EventWire = send_request(
        http_client()
            .post(format!("{CALENDAR_BASE}/calendars/{calendar}/events"))
            .bearer_auth(access_token)
            .query(&[("sendUpdates", "all")])
            .json(&body),
    )
    .await?;
    Ok(event_summary_from_wire(wire))
}

/// Build a participant-only RSVP patch. Google Calendar replaces arrays in a
/// PATCH, so a GET followed by re-sending the full attendee list can overwrite
/// another guest's concurrent response. `attendeesOmitted: true` gives this
/// request the documented participant-only semantics: only the authenticated
/// account's attendee row is updated.
fn rsvp_patch_body(attendee_email: &str, response: InviteResponse) -> serde_json::Value {
    serde_json::json!({
        "attendeesOmitted": true,
        "attendees": [{
            "email": attendee_email,
            "responseStatus": response.as_str(),
        }],
    })
}

pub async fn respond_to_invite(
    access_token: &str,
    calendar_id: Option<&str>,
    event_id: &str,
    attendee_email: &str,
    response: InviteResponse,
) -> Result<EventSummary, GoogleApiError> {
    if attendee_email.trim().is_empty() {
        return Err(GoogleApiError::InvalidInput(
            "The connected account email is required to respond to an invite.".to_string(),
        ));
    }
    let calendar = calendar_id_or_primary(calendar_id);
    let wire: EventWire = send_request(
        http_client()
            .patch(format!(
                "{CALENDAR_BASE}/calendars/{calendar}/events/{event_id}"
            ))
            .bearer_auth(access_token)
            .json(&rsvp_patch_body(attendee_email, response)),
    )
    .await?;
    Ok(event_summary_from_wire(wire))
}

fn event_summary_from_wire(wire: EventWire) -> EventSummary {
    EventSummary {
        id: wire.id,
        summary: wire.summary,
        start: wire.start.and_then(|time| time.date_time.or(time.date)),
        end: wire.end.and_then(|time| time.date_time.or(time.date)),
        attendees: wire
            .attendees
            .into_iter()
            .map(|attendee| EventAttendee {
                email: attendee.email,
                response_status: attendee.response_status,
                is_self: attendee.is_self,
            })
            .collect(),
        location: wire.location,
        organizer: wire.organizer.and_then(|organizer| organizer.email),
        html_link: wire.html_link,
        status: wire.status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn b64url(text: &str) -> String {
        URL_SAFE_NO_PAD.encode(text.as_bytes())
    }

    #[test]
    fn message_metadata_parses_into_email_summary() {
        let fixture = format!(
            r#"{{
              "id": "18f0a",
              "threadId": "18f0t",
              "labelIds": ["UNREAD", "INBOX", "IMPORTANT"],
              "snippet": "Quarterly numbers attached",
              "sizeEstimate": 5321,
              "payload": {{
                "mimeType": "multipart/mixed",
                "headers": [
                  {{"name": "From", "value": "Ada Lovelace <ada@example.com>"}},
                  {{"name": "To", "value": "june@example.com"}},
                  {{"name": "Subject", "value": "Q2 numbers"}},
                  {{"name": "Date", "value": "Wed, 08 Jul 2026 10:00:00 -0700"}},
                  {{"name": "Message-Id", "value": "<q2-numbers@mail.example.com>"}}
                ],
                "parts": [
                  {{"mimeType": "text/plain", "filename": "", "body": {{"size": 42, "data": "{}"}}}}
                ]
              }},
              "unknownField": true
            }}"#,
            b64url("hello")
        );
        let wire: MessageWire = serde_json::from_str(&fixture).expect("parse message");
        let summary = email_summary_from_message(wire);
        assert_eq!(summary.id, "18f0a");
        assert_eq!(summary.thread_id, "18f0t");
        assert_eq!(
            summary.from.as_deref(),
            Some("Ada Lovelace <ada@example.com>")
        );
        assert_eq!(summary.to.as_deref(), Some("june@example.com"));
        assert_eq!(summary.subject.as_deref(), Some("Q2 numbers"));
        assert_eq!(summary.snippet, "Quarterly numbers attached");
        // The RFC Message-ID is exposed (case-insensitively) for reply threading.
        assert_eq!(
            summary.rfc_message_id.as_deref(),
            Some("<q2-numbers@mail.example.com>")
        );
        assert!(summary.unread);
        assert_eq!(summary.label_ids.len(), 3);
    }

    #[test]
    fn thread_body_extraction_prefers_text_plain() {
        let fixture = format!(
            r#"{{
              "id": "t1",
              "messages": [
                {{
                  "id": "m1",
                  "threadId": "t1",
                  "labelIds": ["INBOX"],
                  "snippet": "snippet text",
                  "payload": {{
                    "mimeType": "multipart/alternative",
                    "headers": [
                      {{"name": "From", "value": "ada@example.com"}},
                      {{"name": "Subject", "value": "Hi"}}
                    ],
                    "parts": [
                      {{"mimeType": "text/html", "filename": "", "body": {{"size": 60, "data": "{}"}}}},
                      {{"mimeType": "text/plain", "filename": "", "body": {{"size": 20, "data": "{}"}}}},
                      {{"mimeType": "text/plain", "filename": "notes.txt", "body": {{"size": 9, "data": "{}"}}}}
                    ]
                  }}
                }}
              ]
            }}"#,
            b64url("<p>Hello <b>June</b></p>"),
            b64url("Hello June, plain body."),
            b64url("attached")
        );
        let wire: ThreadWire = serde_json::from_str(&fixture).expect("parse thread");
        let detail = thread_detail_from_wire(wire, true);
        assert_eq!(detail.id, "t1");
        assert_eq!(detail.messages.len(), 1);
        assert_eq!(
            detail.messages[0].body_text.as_deref(),
            Some("Hello June, plain body.")
        );
        assert!(!detail.messages[0].unread);
    }

    #[test]
    fn thread_body_extraction_falls_back_to_stripped_html() {
        let fixture = format!(
            r#"{{
              "id": "t2",
              "messages": [
                {{
                  "id": "m2",
                  "threadId": "t2",
                  "snippet": "",
                  "payload": {{
                    "mimeType": "text/html",
                    "body": {{"size": 60, "data": "{}"}}
                  }}
                }}
              ]
            }}"#,
            b64url("<div>Hello&nbsp;<b>June</b> &amp; team</div>")
        );
        let wire: ThreadWire = serde_json::from_str(&fixture).expect("parse thread");
        let detail = thread_detail_from_wire(wire, true);
        assert_eq!(
            detail.messages[0].body_text.as_deref(),
            Some("Hello June & team")
        );
    }

    #[test]
    fn metadata_thread_carries_no_bodies() {
        let fixture = format!(
            r#"{{
              "id": "t3",
              "messages": [
                {{
                  "id": "m3",
                  "threadId": "t3",
                  "snippet": "s",
                  "payload": {{"mimeType": "text/plain", "body": {{"size": 5, "data": "{}"}}}}
                }}
              ]
            }}"#,
            b64url("body!")
        );
        let wire: ThreadWire = serde_json::from_str(&fixture).expect("parse thread");
        let detail = thread_detail_from_wire(wire, false);
        assert!(detail.messages[0].body_text.is_none());
    }

    #[test]
    fn events_list_parses_into_summaries() {
        let fixture = r#"{
          "kind": "calendar#events",
          "nextSyncToken": "sync-123",
          "items": [
            {
              "id": "evt1",
              "status": "confirmed",
              "htmlLink": "https://calendar.google.com/event?eid=abc",
              "summary": "Design review",
              "location": "Conference room 2",
              "organizer": {"email": "boss@example.com", "displayName": "Boss"},
              "start": {"dateTime": "2026-07-10T09:00:00-07:00"},
              "end": {"dateTime": "2026-07-10T09:30:00-07:00"},
              "attendees": [
                {"email": "boss@example.com", "responseStatus": "accepted"},
                {"email": "me@example.com", "responseStatus": "needsAction", "self": true}
              ]
            },
            {
              "id": "evt2",
              "summary": "Company holiday",
              "start": {"date": "2026-07-11"},
              "end": {"date": "2026-07-12"}
            }
          ]
        }"#;
        let wire: EventsListWire = serde_json::from_str(fixture).expect("parse events");
        assert_eq!(wire.next_sync_token.as_deref(), Some("sync-123"));
        let items: Vec<EventSummary> = wire
            .items
            .into_iter()
            .map(event_summary_from_wire)
            .collect();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].summary.as_deref(), Some("Design review"));
        assert_eq!(items[0].start.as_deref(), Some("2026-07-10T09:00:00-07:00"));
        assert_eq!(items[0].organizer.as_deref(), Some("boss@example.com"));
        assert_eq!(items[0].attendees.len(), 2);
        assert!(items[0].attendees[1].is_self);
        assert_eq!(
            items[0].attendees[1].response_status.as_deref(),
            Some("needsAction")
        );
        assert_eq!(items[1].start.as_deref(), Some("2026-07-11"));
    }

    #[test]
    fn history_list_flattens_added_messages() {
        let fixture = r#"{
          "historyId": "500",
          "history": [
            {
              "id": "498",
              "messagesAdded": [
                {"message": {"id": "m1", "threadId": "t1", "labelIds": ["INBOX", "UNREAD"]}}
              ]
            },
            {"id": "499"},
            {
              "id": "500",
              "messagesAdded": [
                {"message": {"id": "m2", "threadId": "t2", "labelIds": ["INBOX"]}}
              ]
            }
          ]
        }"#;
        let wire: HistoryListWire = serde_json::from_str(fixture).expect("parse history");
        assert_eq!(wire.history_id.as_deref(), Some("500"));
        let added: Vec<EmailSummary> = wire
            .history
            .into_iter()
            .flat_map(|entry| entry.messages_added)
            .filter_map(|added| added.message)
            .map(email_summary_from_message)
            .collect();
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].id, "m1");
        assert!(added[0].unread);
        assert!(!added[1].unread);
    }

    #[test]
    fn history_cursor_expired_maps_to_a_stable_code() {
        // history_list turns a 404 into this variant; the trigger daemon keys
        // its reseed on this exact code.
        let error: AppError = GoogleApiError::HistoryCursorExpired.into();
        assert_eq!(error.code, "connector_history_cursor_expired");
    }

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn free_slot_computation_respects_busy_and_working_hours() {
        let params = FreeSlotParams {
            time_min: utc(2026, 7, 10, 0, 0),
            time_max: utc(2026, 7, 11, 0, 0),
            working_start_hour: 9,
            working_end_hour: 17,
            utc_offset_minutes: 0,
            min_slot_minutes: 30,
        };
        let busy = vec![
            // Overlapping meetings 10:00-11:00 and 10:30-11:30 merge.
            BusyInterval {
                start: utc(2026, 7, 10, 10, 0),
                end: utc(2026, 7, 10, 11, 0),
            },
            BusyInterval {
                start: utc(2026, 7, 10, 10, 30),
                end: utc(2026, 7, 10, 11, 30),
            },
            // 14:00-14:15 leaves a sub-minimum gap before 14:30.
            BusyInterval {
                start: utc(2026, 7, 10, 14, 0),
                end: utc(2026, 7, 10, 14, 15),
            },
            BusyInterval {
                start: utc(2026, 7, 10, 14, 30),
                end: utc(2026, 7, 10, 16, 0),
            },
            // Busy outside working hours is irrelevant.
            BusyInterval {
                start: utc(2026, 7, 10, 20, 0),
                end: utc(2026, 7, 10, 21, 0),
            },
        ];
        let slots = find_free_slots(&params, &busy);
        assert_eq!(
            slots,
            vec![
                FreeSlot {
                    start: utc(2026, 7, 10, 9, 0),
                    end: utc(2026, 7, 10, 10, 0),
                },
                FreeSlot {
                    start: utc(2026, 7, 10, 11, 30),
                    end: utc(2026, 7, 10, 14, 0),
                },
                FreeSlot {
                    start: utc(2026, 7, 10, 16, 0),
                    end: utc(2026, 7, 10, 17, 0),
                },
            ]
        );
    }

    #[test]
    fn free_slot_computation_applies_utc_offset() {
        // 9:00-17:00 at UTC-7 is 16:00-24:00 UTC.
        let params = FreeSlotParams {
            time_min: utc(2026, 7, 10, 0, 0),
            time_max: utc(2026, 7, 11, 4, 0),
            working_start_hour: 9,
            working_end_hour: 17,
            utc_offset_minutes: -420,
            min_slot_minutes: 60,
        };
        let slots = find_free_slots(&params, &[]);
        assert!(slots.contains(&FreeSlot {
            start: utc(2026, 7, 10, 16, 0),
            end: utc(2026, 7, 11, 0, 0),
        }));
    }

    #[test]
    fn free_slot_computation_empty_on_inverted_window() {
        let params = FreeSlotParams {
            time_min: utc(2026, 7, 11, 0, 0),
            time_max: utc(2026, 7, 10, 0, 0),
            working_start_hour: 9,
            working_end_hour: 17,
            utc_offset_minutes: 0,
            min_slot_minutes: 30,
        };
        assert!(find_free_slots(&params, &[]).is_empty());
    }

    fn outgoing() -> OutgoingEmail {
        OutgoingEmail {
            to: vec!["ada@example.com".to_string()],
            cc: vec![],
            subject: "Meeting notes".to_string(),
            body_text: "Hi Ada,\n\nNotes attached below.\n".to_string(),
            in_reply_to: Some("<abc@mail.example.com>".to_string()),
            references: Some("<abc@mail.example.com>".to_string()),
            thread_id: None,
        }
    }

    #[test]
    fn mime_builder_round_trips_headers_and_body() {
        let email = outgoing();
        let mime = build_mime(&email).expect("build mime");
        let (headers, body) = mime.split_once("\r\n\r\n").expect("header/body split");
        assert!(headers.contains("To: ada@example.com"));
        assert!(headers.contains("Subject: Meeting notes"));
        assert!(headers.contains("In-Reply-To: <abc@mail.example.com>"));
        assert!(headers.contains("References: <abc@mail.example.com>"));
        assert!(headers.contains("Content-Type: text/plain; charset=UTF-8"));
        assert!(headers.contains("Content-Transfer-Encoding: base64"));
        let encoded: String = body
            .chars()
            .filter(|ch| !ch.is_ascii_whitespace())
            .collect();
        let decoded = STANDARD.decode(encoded).expect("decode body");
        assert_eq!(String::from_utf8(decoded).unwrap(), email.body_text);
    }

    #[test]
    fn mime_builder_encodes_unicode_subject() {
        let mut email = outgoing();
        email.subject = "Réunion à 9h".to_string();
        let mime = build_mime(&email).expect("build mime");
        let subject_line = mime
            .lines()
            .find(|line| line.starts_with("Subject: "))
            .expect("subject header");
        let encoded = subject_line
            .strip_prefix("Subject: =?UTF-8?B?")
            .and_then(|rest| rest.strip_suffix("?="))
            .expect("rfc 2047 encoded word");
        assert_eq!(
            String::from_utf8(STANDARD.decode(encoded).unwrap()).unwrap(),
            "Réunion à 9h"
        );
    }

    #[test]
    fn mime_builder_rejects_header_injection() {
        let mut email = outgoing();
        email.subject = "Hello\r\nBcc: attacker@example.com".to_string();
        assert!(matches!(
            build_mime(&email),
            Err(GoogleApiError::InvalidInput(_))
        ));

        let mut email = outgoing();
        email.to = vec!["ada@example.com\r\nBcc: attacker@example.com".to_string()];
        assert!(matches!(
            build_mime(&email),
            Err(GoogleApiError::InvalidInput(_))
        ));

        let mut email = outgoing();
        email.in_reply_to = Some("<x>\nInjected: yes".to_string());
        assert!(matches!(
            build_mime(&email),
            Err(GoogleApiError::InvalidInput(_))
        ));

        let mut email = outgoing();
        email.to = vec![];
        assert!(matches!(
            build_mime(&email),
            Err(GoogleApiError::InvalidInput(_))
        ));
    }

    #[test]
    fn raw_message_is_base64url() {
        let raw = encode_raw_message(&outgoing()).expect("encode");
        assert!(URL_SAFE_NO_PAD.decode(&raw).is_ok());
        assert!(!raw.contains('+') && !raw.contains('/'));
    }

    #[test]
    fn api_error_message_extracts_google_error_field() {
        let body = r#"{"error": {"code": 403, "message": "Rate limit exceeded", "status": "RESOURCE_EXHAUSTED"}}"#;
        assert_eq!(api_error_message(body), "Rate limit exceeded");
        assert_eq!(api_error_message("not json"), "request failed");
    }

    #[test]
    fn rsvp_patch_updates_only_the_connected_participant() {
        let body = rsvp_patch_body("me@example.com", InviteResponse::Declined);
        assert_eq!(body["attendeesOmitted"], true);
        assert_eq!(body["attendees"].as_array().map(Vec::len), Some(1));
        assert_eq!(body["attendees"][0]["email"], "me@example.com");
        assert_eq!(body["attendees"][0]["responseStatus"], "declined");
    }
}
