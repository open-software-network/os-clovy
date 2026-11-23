//! Connector tool descriptors and direct provider dispatch for the Clovy MCP server.
//! Mutations reach dispatch only after the MCP approval broker authorizes them.

use crate::{
    connectors::{
        self,
        policy::{
            self, CALENDAR_EVENTS, CALENDAR_READONLY, GITHUB_READ, GITHUB_WRITE, GMAIL_COMPOSE,
            GMAIL_MODIFY, GMAIL_READONLY, GMAIL_SEND,
        },
        ConnectorAccount, ConnectorAccountStatus, ConnectorProvider,
    },
    domain::types::AppError,
};
use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use tauri::AppHandle;

const PROVIDER_GITHUB: &str = "github";

#[derive(Clone, Copy)]
struct Capability {
    name: &'static str,
    description: &'static str,
    provider: ConnectorProvider,
    required_scope: &'static str,
    mutation: bool,
    needs_selected_teams: bool,
}

const CAPABILITIES: &[Capability] = &[
    Capability {
        name: "search_threads",
        description: "Search connected Gmail threads and return compact summaries.",
        provider: ConnectorProvider::Google,
        required_scope: GMAIL_READONLY,
        mutation: false,
        needs_selected_teams: false,
    },
    Capability {
        name: "read_thread",
        description: "Read the messages and plain-text bodies in one connected Gmail thread.",
        provider: ConnectorProvider::Google,
        required_scope: GMAIL_READONLY,
        mutation: false,
        needs_selected_teams: false,
    },
    Capability {
        name: "list_unread",
        description: "List unread messages from the connected Gmail inbox.",
        provider: ConnectorProvider::Google,
        required_scope: GMAIL_READONLY,
        mutation: false,
        needs_selected_teams: false,
    },
    Capability {
        name: "create_draft",
        description: "Create a Gmail draft. The user must approve this action.",
        provider: ConnectorProvider::Google,
        required_scope: GMAIL_COMPOSE,
        mutation: true,
        needs_selected_teams: false,
    },
    Capability {
        name: "send_email",
        description: "Send an email through Gmail. The user must approve this action.",
        provider: ConnectorProvider::Google,
        required_scope: GMAIL_SEND,
        mutation: true,
        needs_selected_teams: false,
    },
    Capability {
        name: "modify_labels",
        description: "Add or remove Gmail labels from a message. The user must approve this action.",
        provider: ConnectorProvider::Google,
        required_scope: GMAIL_MODIFY,
        mutation: true,
        needs_selected_teams: false,
    },
    Capability {
        name: "archive",
        description: "Archive a Gmail message. The user must approve this action.",
        provider: ConnectorProvider::Google,
        required_scope: GMAIL_MODIFY,
        mutation: true,
        needs_selected_teams: false,
    },
    Capability {
        name: "list_events",
        description: "List events from a connected Google calendar.",
        provider: ConnectorProvider::Google,
        required_scope: CALENDAR_READONLY,
        mutation: false,
        needs_selected_teams: false,
    },
    Capability {
        name: "get_event",
        description: "Read one event from a connected Google calendar.",
        provider: ConnectorProvider::Google,
        required_scope: CALENDAR_READONLY,
        mutation: false,
        needs_selected_teams: false,
    },
    Capability {
        name: "find_free_slots",
        description: "Find free time in a connected Google calendar.",
        provider: ConnectorProvider::Google,
        required_scope: CALENDAR_READONLY,
        mutation: false,
        needs_selected_teams: false,
    },
    Capability {
        name: "create_event",
        description: "Create a Google calendar event. The user must approve this action.",
        provider: ConnectorProvider::Google,
        required_scope: CALENDAR_EVENTS,
        mutation: true,
        needs_selected_teams: false,
    },
    Capability {
        name: "respond_to_invite",
        description: "Accept, decline, or tentatively accept a Google calendar invite. The user must approve this action.",
        provider: ConnectorProvider::Google,
        required_scope: CALENDAR_EVENTS,
        mutation: true,
        needs_selected_teams: false,
    },
    Capability {
        name: "list_repositories",
        description: "List repositories available through the connected GitHub App.",
        provider: ConnectorProvider::Github,
        required_scope: GITHUB_READ,
        mutation: false,
        needs_selected_teams: false,
    },
    Capability {
        name: "get_pull_request",
        description: "Read one pull request from the connected GitHub account.",
        provider: ConnectorProvider::Github,
        required_scope: GITHUB_READ,
        mutation: false,
        needs_selected_teams: false,
    },
];

/// Build a fresh catalog from the non-secret account index. It is invoked for
/// every start, retry, and resume so reconnects, scope changes, and selected
/// team changes cannot leave an old tool available in a later run.
pub async fn descriptors(app: &AppHandle) -> Result<Vec<Value>, AppError> {
    let accounts = connectors::list_runtime_accounts(app).await?;
    let mut descriptors = descriptors_from_accounts(&accounts);
    match crate::connectors::notion::runtime_descriptors(app).await {
        Ok(notion_descriptors) => descriptors.extend(notion_descriptors),
        Err(error) => tracing::warn!(
            code = %error.code,
            "notion tool discovery failed; continuing without notion tools"
        ),
    }
    Ok(descriptors)
}

pub fn routine_tool_allowed(
    name: &str,
    enabled_toolsets: &[String],
    autonomous_tools: &[String],
    trust_mode: Option<&str>,
) -> bool {
    use crate::connectors::policy::{
        JUNE_GCAL_ACTIONS_SERVER, JUNE_GCAL_SERVER, JUNE_GITHUB_ACTIONS_SERVER, JUNE_GITHUB_SERVER,
        JUNE_GMAIL_ACTIONS_SERVER, JUNE_GMAIL_SERVER,
    };
    let has = |server: &str| enabled_toolsets.iter().any(|value| value == server);
    let has_prefix = |prefix: &str| {
        enabled_toolsets
            .iter()
            .any(|value| value.starts_with(prefix))
    };
    match name {
        "notion-search"
        | "notion-fetch"
        | "notion-query-data-sources"
        | "notion-query-database-view"
        | "notion-get-comments"
        | "notion-get-enhanced-markdown-specification"
        | "notion-get-view-configuration-dsl" => has("june_notion"),
        "notion-create-pages" | "notion-update-page" => {
            has("june_notion_actions") && trust_mode == Some("approval")
        }
        "search_threads" | "read_thread" | "list_unread" => has(JUNE_GMAIL_SERVER),
        "create_draft" | "send_email" | "modify_labels" | "archive" => {
            has(JUNE_GMAIL_ACTIONS_SERVER)
                || (has_prefix("june_gmail_auto_")
                    && autonomous_tools.iter().any(|tool| tool == name))
        }
        "list_events" | "get_event" | "find_free_slots" => has(JUNE_GCAL_SERVER),
        "create_event" | "respond_to_invite" => {
            has(JUNE_GCAL_ACTIONS_SERVER)
                || (has_prefix("june_gcal_auto_")
                    && autonomous_tools.iter().any(|tool| tool == name))
        }
        "list_repositories" | "get_pull_request" => has(JUNE_GITHUB_SERVER),
        "search_issues" | "get_issue" | "list_issue_comments" => has(JUNE_GITHUB_SERVER),
        "create_issue" | "update_issue" | "add_comment" => has(JUNE_GITHUB_ACTIONS_SERVER),
        _ => false,
    }
}

fn descriptors_from_accounts(accounts: &[ConnectorAccount]) -> Vec<Value> {
    let mut descriptors = CAPABILITIES
        .iter()
        .filter_map(|capability| {
            let ids = eligible_account_ids(
                accounts,
                capability.provider,
                capability.required_scope,
                capability.needs_selected_teams,
            );
            (!ids.is_empty()).then(|| account_descriptor(*capability, ids))
        })
        .collect::<Vec<_>>();

    let github_read = eligible_account_ids(accounts, ConnectorProvider::Github, GITHUB_READ, false);
    if !github_read.is_empty() {
        let providers = provider_choices(true);
        descriptors.extend([
            provider_descriptor(
                "search_issues",
                "Search issues in connected GitHub repositories.",
                &providers,
                github_read.clone(),
                false,
            ),
            provider_descriptor(
                "get_issue",
                "Read one GitHub issue.",
                &providers,
                github_read.clone(),
                false,
            ),
            provider_descriptor(
                "list_issue_comments",
                "List comments on one GitHub issue.",
                &providers,
                github_read,
                false,
            ),
        ]);
    }
    let github_write =
        eligible_account_ids(accounts, ConnectorProvider::Github, GITHUB_WRITE, false);
    if !github_write.is_empty() {
        let providers = provider_choices(true);
        descriptors.extend([
            provider_descriptor(
                "create_issue",
                "Create a GitHub issue. The user must approve this action.",
                &providers,
                github_write.clone(),
                true,
            ),
            provider_descriptor(
                "update_issue",
                "Update a GitHub issue. The user must approve this action.",
                &providers,
                github_write.clone(),
                true,
            ),
            provider_descriptor(
                "add_comment",
                "Add a GitHub issue comment. The user must approve this action.",
                &providers,
                github_write,
                true,
            ),
        ]);
    }
    descriptors
}

fn provider_choices(github: bool) -> Vec<&'static str> {
    [github.then_some(PROVIDER_GITHUB)]
        .into_iter()
        .flatten()
        .collect()
}

fn eligible_account_ids(
    accounts: &[ConnectorAccount],
    provider: ConnectorProvider,
    scope: &str,
    needs_selected_teams: bool,
) -> Vec<String> {
    accounts
        .iter()
        .filter(|account| {
            account.provider == provider
                && account.status == ConnectorAccountStatus::Connected
                && account
                    .scopes
                    .iter()
                    .any(|held| policy::scope_grants(held, scope))
                && (!needs_selected_teams || !account.selected_teams.is_empty())
        })
        .map(|account| account.account_id.clone())
        .collect()
}

fn account_descriptor(capability: Capability, account_ids: Vec<String>) -> Value {
    let (mut properties, mut required) = input_schema(capability.name);
    properties.insert(
        "accountId".to_string(),
        json!({ "type": "string", "enum": account_ids }),
    );
    required.push("accountId");
    descriptor(
        capability.name,
        capability.description,
        properties,
        required,
        capability.mutation,
    )
}

fn provider_descriptor(
    name: &str,
    description: &str,
    providers: &[&str],
    account_ids: Vec<String>,
    mutation: bool,
) -> Value {
    let (mut properties, mut required) = input_schema(name);
    properties.insert(
        "provider".to_string(),
        json!({ "type": "string", "enum": providers }),
    );
    properties.insert(
        "accountId".to_string(),
        json!({ "type": "string", "enum": account_ids }),
    );
    required.extend(["provider", "accountId"]);
    descriptor(name, description, properties, required, mutation)
}

fn input_schema(name: &str) -> (Map<String, Value>, Vec<&'static str>) {
    let mut properties = Map::new();
    let mut required = Vec::new();
    let string = || json!({ "type": "string" });
    let integer = || json!({ "type": "integer" });
    let strings = || json!({ "type": "array", "items": { "type": "string" } });
    match name {
        "search_threads" | "list_unread" => {
            properties.insert("query".into(), string());
            properties.insert("maxResults".into(), integer());
            properties.insert("pageToken".into(), string());
        }
        "read_thread" => {
            properties.insert("threadId".into(), string());
            required.push("threadId");
        }
        "create_draft" | "send_email" => {
            properties.insert("to".into(), strings());
            properties.insert("cc".into(), strings());
            properties.insert("subject".into(), string());
            properties.insert("bodyText".into(), string());
            properties.insert("inReplyTo".into(), string());
            properties.insert("references".into(), string());
            properties.insert("threadId".into(), string());
            required.extend(["to", "subject", "bodyText"]);
        }
        "modify_labels" => {
            properties.insert("messageId".into(), string());
            properties.insert("add".into(), strings());
            properties.insert("remove".into(), strings());
            required.push("messageId");
        }
        "archive" => {
            properties.insert("messageId".into(), string());
            required.push("messageId");
        }
        "list_events" => {
            properties.insert("calendarId".into(), string());
            properties.insert("timeMin".into(), string());
            properties.insert("timeMax".into(), string());
            properties.insert("maxResults".into(), integer());
            properties.insert("pageToken".into(), string());
        }
        "get_event" => {
            properties.insert("calendarId".into(), string());
            properties.insert("eventId".into(), string());
            required.push("eventId");
        }
        "find_free_slots" => {
            properties.insert("timeMin".into(), string());
            properties.insert("timeMax".into(), string());
            properties.insert("calendarIds".into(), strings());
            properties.insert("workingStartHour".into(), integer());
            properties.insert("workingEndHour".into(), integer());
            properties.insert("utcOffsetMinutes".into(), integer());
            properties.insert("minSlotMinutes".into(), integer());
            required.extend(["timeMin", "timeMax"]);
        }
        "create_event" => {
            properties.insert("calendarId".into(), string());
            properties.insert("summary".into(), string());
            properties.insert("description".into(), string());
            properties.insert("location".into(), string());
            properties.insert("startRfc3339".into(), string());
            properties.insert("endRfc3339".into(), string());
            properties.insert("attendeeEmails".into(), strings());
            required.extend(["summary", "startRfc3339", "endRfc3339"]);
        }
        "respond_to_invite" => {
            properties.insert("calendarId".into(), string());
            properties.insert("eventId".into(), string());
            properties.insert(
                "response".into(),
                json!({ "type": "string", "enum": ["accepted", "declined", "tentative"] }),
            );
            required.extend(["eventId", "response"]);
        }
        "get_pull_request" => {
            properties.insert("owner".into(), string());
            properties.insert("repo".into(), string());
            properties.insert("number".into(), integer());
            required.extend(["owner", "repo", "number"]);
        }
        "search_issues" => {
            properties.insert("query".into(), string());
            properties.insert("stateType".into(), string());
            properties.insert("assigneeId".into(), string());
            properties.insert("first".into(), integer());
        }
        "get_issue" | "list_issue_comments" => {
            properties.insert("issueId".into(), string());
            properties.insert("owner".into(), string());
            properties.insert("repo".into(), string());
            properties.insert("number".into(), integer());
            properties.insert("first".into(), integer());
        }
        "create_issue" => {
            properties.insert("teamId".into(), string());
            properties.insert("owner".into(), string());
            properties.insert("repo".into(), string());
            properties.insert("title".into(), string());
            properties.insert("description".into(), string());
            properties.insert("body".into(), string());
            properties.insert("priority".into(), integer());
            properties.insert("assigneeId".into(), string());
            properties.insert("projectId".into(), string());
            properties.insert("labels".into(), strings());
            required.push("title");
        }
        "update_issue" => {
            properties.insert("issueId".into(), string());
            properties.insert("owner".into(), string());
            properties.insert("repo".into(), string());
            properties.insert("number".into(), integer());
            properties.insert("title".into(), string());
            properties.insert("description".into(), string());
            properties.insert("body".into(), string());
            properties.insert("stateId".into(), string());
            properties.insert("priority".into(), integer());
            properties.insert("assigneeId".into(), string());
            properties.insert("projectId".into(), string());
            properties.insert("cycleId".into(), string());
            properties.insert("labels".into(), strings());
        }
        "add_comment" => {
            properties.insert("issueId".into(), string());
            properties.insert("owner".into(), string());
            properties.insert("repo".into(), string());
            properties.insert("number".into(), integer());
            properties.insert("body".into(), string());
            required.push("body");
        }
        "create_project_update" => {
            properties.insert("projectId".into(), string());
            properties.insert("body".into(), string());
            properties.insert(
                "health".into(),
                json!({ "type": "string", "enum": ["onTrack", "atRisk", "offTrack"] }),
            );
            required.extend(["projectId", "body"]);
        }
        _ => {}
    }
    (properties, required)
}

fn descriptor(
    name: &str,
    description: &str,
    properties: Map<String, Value>,
    required: Vec<&str>,
    requires_approval: bool,
) -> Value {
    json!({
        "name": name,
        "description": description,
        "parameters": { "type": "object", "properties": properties, "required": required, "additionalProperties": false },
        "requiresApproval": requires_approval,
    })
}

/// Where the GitHub tools get accounts and tokens: the app (Keychain or `gh`
/// per account, ADR-0066) or a test double. Authorization is the same
/// [`connectors::check_account_grant`] either way.
pub(crate) trait GithubAccess: Send + Sync {
    fn accounts(&self) -> BoxFuture<'_, Result<Vec<ConnectorAccount>, AppError>>;
    fn token<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, Result<String, AppError>>;
}

struct AppGithub<'a>(&'a AppHandle);

impl GithubAccess for AppGithub<'_> {
    fn accounts(&self) -> BoxFuture<'_, Result<Vec<ConnectorAccount>, AppError>> {
        Box::pin(connectors::list_runtime_accounts(self.0))
    }
    fn token<'a>(&'a self, account_id: &'a str) -> BoxFuture<'a, Result<String, AppError>> {
        Box::pin(connectors::github_access_token(self.0, account_id))
    }
}

/// The GitHub tools; `None` for any other name.
pub(crate) async fn dispatch_github(
    access: &dyn GithubAccess,
    name: &str,
    arguments: Value,
) -> Option<Result<Value, AppError>> {
    Some(match name {
        "list_repositories" => github_list_repositories(access, arguments).await,
        "get_pull_request" => github_get_pull_request(access, arguments).await,
        "search_issues" => issues_search(access, arguments).await,
        "get_issue" => issues_get(access, arguments).await,
        "list_issue_comments" => issues_comments(access, arguments).await,
        "create_issue" => issues_create(access, arguments).await,
        "update_issue" => issues_update(access, arguments).await,
        "add_comment" => issues_add_comment(access, arguments).await,
        _ => return None,
    })
}

/// Returns `Ok(None)` for a non-connector tool. A matching connector tool is
/// always re-authorized from live account state rather than trusting the
/// descriptor emitted at run start.
pub async fn dispatch(
    app: &AppHandle,
    name: &str,
    arguments: Value,
) -> Result<Option<Value>, AppError> {
    let result = match name {
        "search_threads" => gmail_search_threads(app, arguments).await,
        "read_thread" => gmail_read_thread(app, arguments).await,
        "list_unread" => gmail_list_unread(app, arguments).await,
        "create_draft" => gmail_create_draft(app, arguments).await,
        "send_email" => gmail_send_email(app, arguments).await,
        "modify_labels" => gmail_modify_labels(app, arguments).await,
        "archive" => gmail_archive(app, arguments).await,
        "list_events" => calendar_list_events(app, arguments).await,
        "get_event" => calendar_get_event(app, arguments).await,
        "find_free_slots" => calendar_find_free_slots(app, arguments).await,
        "create_event" => calendar_create_event(app, arguments).await,
        "respond_to_invite" => calendar_respond_to_invite(app, arguments).await,
        _ => {
            return dispatch_github(&AppGithub(app), name, arguments)
                .await
                .transpose()
        }
    }?;
    Ok(Some(result))
}

async fn authorized_account(
    app: &AppHandle,
    account_id: &str,
    provider: ConnectorProvider,
    scope: &str,
    teams: bool,
) -> Result<ConnectorAccount, AppError> {
    let accounts = connectors::list_runtime_accounts(app).await?;
    connectors::check_account_grant(&accounts, account_id, provider, scope, teams)
}

fn json_error(error: serde_json::Error) -> AppError {
    AppError::new("agent_connector_invalid_response", error.to_string())
}

/// Provider clients define their own `From<ProviderError> for AppError`
/// conversions. Keep every provider error on that reviewed, sanitized path;
/// JSON handling above is intentionally separate because it is local input or
/// serialization failure, not an upstream response.
fn app_error<E>(error: E) -> AppError
where
    AppError: From<E>,
{
    AppError::from(error)
}
fn required(arguments: &Value, field: &str) -> Result<String, AppError> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| AppError::new("invalid_arguments", format!("{field} is required.")))
}
fn optional(arguments: &Value, field: &str) -> Option<String> {
    arguments
        .get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
fn number(arguments: &Value, field: &str) -> Option<u32> {
    arguments
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
}
fn account_id(arguments: &Value) -> Result<String, AppError> {
    required(arguments, "accountId")
}
fn as_value(value: impl serde::Serialize) -> Result<Value, AppError> {
    serde_json::to_value(value).map_err(json_error)
}

async fn google_token(app: &AppHandle, arguments: &Value, scope: &str) -> Result<String, AppError> {
    let account = authorized_account(
        app,
        &account_id(arguments)?,
        ConnectorProvider::Google,
        scope,
        false,
    )
    .await?;
    connectors::google_access_token(app, &account.account_id).await
}

async fn gmail_search_threads(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, GMAIL_READONLY).await?;
    as_value(
        crate::connectors::google::list_threads(
            &token,
            optional(&arguments, "query").as_deref(),
            number(&arguments, "maxResults"),
            optional(&arguments, "pageToken").as_deref(),
        )
        .await
        .map_err(app_error)?,
    )
}
async fn gmail_read_thread(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, GMAIL_READONLY).await?;
    as_value(
        crate::connectors::google::read_thread(&token, &required(&arguments, "threadId")?)
            .await
            .map_err(app_error)?,
    )
}
async fn gmail_list_unread(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, GMAIL_READONLY).await?;
    as_value(
        crate::connectors::google::list_unread(
            &token,
            number(&arguments, "maxResults"),
            optional(&arguments, "pageToken").as_deref(),
        )
        .await
        .map_err(app_error)?,
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EmailArgs {
    to: Vec<String>,
    #[serde(default)]
    cc: Vec<String>,
    subject: String,
    body_text: String,
    #[serde(default)]
    in_reply_to: Option<String>,
    #[serde(default)]
    references: Option<String>,
    #[serde(default)]
    thread_id: Option<String>,
}
fn email(args: EmailArgs) -> crate::connectors::google::OutgoingEmail {
    crate::connectors::google::OutgoingEmail {
        to: args.to,
        cc: args.cc,
        subject: args.subject,
        body_text: args.body_text,
        in_reply_to: args.in_reply_to,
        references: args.references,
        thread_id: args.thread_id,
    }
}
async fn gmail_create_draft(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let email_args: EmailArgs = serde_json::from_value(arguments.clone()).map_err(json_error)?;
    let token = google_token(app, &arguments, GMAIL_COMPOSE).await?;
    as_value(
        crate::connectors::google::create_draft(&token, &email(email_args))
            .await
            .map_err(app_error)?,
    )
}
async fn gmail_send_email(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let email_args: EmailArgs = serde_json::from_value(arguments.clone()).map_err(json_error)?;
    let token = google_token(app, &arguments, GMAIL_SEND).await?;
    as_value(
        crate::connectors::google::send_email(&token, &email(email_args))
            .await
            .map_err(app_error)?,
    )
}

async fn gmail_modify_labels(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, GMAIL_MODIFY).await?;
    let add = string_list(&arguments, "add").unwrap_or_default();
    let remove = string_list(&arguments, "remove").unwrap_or_default();
    as_value(
        crate::connectors::google::modify_labels(
            &token,
            &required(&arguments, "messageId")?,
            &add,
            &remove,
        )
        .await
        .map_err(app_error)?,
    )
}

async fn gmail_archive(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, GMAIL_MODIFY).await?;
    as_value(
        crate::connectors::google::modify_labels(
            &token,
            &required(&arguments, "messageId")?,
            &[],
            &["INBOX".to_string()],
        )
        .await
        .map_err(app_error)?,
    )
}

async fn calendar_list_events(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, CALENDAR_READONLY).await?;
    let params = crate::connectors::google::ListEventsParams {
        calendar_id: optional(&arguments, "calendarId"),
        time_min: optional(&arguments, "timeMin"),
        time_max: optional(&arguments, "timeMax"),
        max_results: number(&arguments, "maxResults"),
        page_token: optional(&arguments, "pageToken"),
        sync_token: None,
    };
    as_value(
        crate::connectors::google::list_events(&token, &params)
            .await
            .map_err(app_error)?,
    )
}
async fn calendar_get_event(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, CALENDAR_READONLY).await?;
    as_value(
        crate::connectors::google::get_event(
            &token,
            optional(&arguments, "calendarId").as_deref(),
            &required(&arguments, "eventId")?,
        )
        .await
        .map_err(app_error)?,
    )
}
async fn calendar_find_free_slots(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let token = google_token(app, &arguments, CALENDAR_READONLY).await?;
    let time_min = parse_time(&required(&arguments, "timeMin")?)?;
    let time_max = parse_time(&required(&arguments, "timeMax")?)?;
    let calendar_ids = arguments
        .get("calendarIds")
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    let busy = crate::connectors::google::freebusy(
        &token,
        &time_min.to_rfc3339(),
        &time_max.to_rfc3339(),
        &calendar_ids,
    )
    .await
    .map_err(app_error)?;
    let params = crate::connectors::google::FreeSlotParams {
        time_min,
        time_max,
        working_start_hour: number(&arguments, "workingStartHour").unwrap_or(9),
        working_end_hour: number(&arguments, "workingEndHour").unwrap_or(17),
        utc_offset_minutes: arguments
            .get("utcOffsetMinutes")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
        min_slot_minutes: arguments
            .get("minSlotMinutes")
            .and_then(Value::as_i64)
            .unwrap_or(30),
    };
    as_value(crate::connectors::google::find_free_slots(&params, &busy))
}
fn parse_time(value: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(value)
        .map(|time| time.with_timezone(&Utc))
        .map_err(|_| {
            AppError::new(
                "invalid_arguments",
                "Calendar times must be RFC 3339 timestamps.",
            )
        })
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventArgs {
    #[serde(default)]
    calendar_id: Option<String>,
    summary: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    start_rfc3339: String,
    end_rfc3339: String,
    #[serde(default)]
    attendee_emails: Vec<String>,
}
async fn calendar_create_event(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let event: EventArgs = serde_json::from_value(arguments.clone()).map_err(json_error)?;
    let token = google_token(app, &arguments, CALENDAR_EVENTS).await?;
    let payload = crate::connectors::google::NewEvent {
        summary: event.summary,
        description: event.description,
        location: event.location,
        start_rfc3339: event.start_rfc3339,
        end_rfc3339: event.end_rfc3339,
        attendee_emails: event.attendee_emails,
    };
    as_value(
        crate::connectors::google::insert_event(&token, event.calendar_id.as_deref(), &payload)
            .await
            .map_err(app_error)?,
    )
}

async fn calendar_respond_to_invite(app: &AppHandle, arguments: Value) -> Result<Value, AppError> {
    let account = authorized_account(
        app,
        &account_id(&arguments)?,
        ConnectorProvider::Google,
        CALENDAR_EVENTS,
        false,
    )
    .await?;
    let token = connectors::google_access_token(app, &account.account_id).await?;
    let response = match required(&arguments, "response")?.as_str() {
        "accepted" => crate::connectors::google::InviteResponse::Accepted,
        "declined" => crate::connectors::google::InviteResponse::Declined,
        "tentative" => crate::connectors::google::InviteResponse::Tentative,
        _ => {
            return Err(AppError::new(
                "invalid_arguments",
                "response must be accepted, declined, or tentative.",
            ));
        }
    };
    as_value(
        crate::connectors::google::respond_to_invite(
            &token,
            optional(&arguments, "calendarId").as_deref(),
            &required(&arguments, "eventId")?,
            &account.email,
            response,
        )
        .await
        .map_err(app_error)?,
    )
}

async fn github_account(
    access: &dyn GithubAccess,
    arguments: &Value,
    scope: &str,
) -> Result<(ConnectorAccount, String), AppError> {
    let accounts = access.accounts().await?;
    let account = connectors::check_account_grant(
        &accounts,
        &account_id(arguments)?,
        ConnectorProvider::Github,
        scope,
        false,
    )?;
    let token = access.token(&account.account_id).await?;
    Ok((account, token))
}
async fn github_token(
    access: &dyn GithubAccess,
    arguments: &Value,
    scope: &str,
) -> Result<String, AppError> {
    Ok(github_account(access, arguments, scope).await?.1)
}
async fn github_list_repositories(
    access: &dyn GithubAccess,
    arguments: Value,
) -> Result<Value, AppError> {
    let (account, token) = github_account(access, &arguments, GITHUB_READ).await?;
    // A gh token is an OAuth token; the App installation endpoints refuse it.
    let list = if account.via_gh {
        crate::connectors::github::list_user_repositories(&token).await
    } else {
        crate::connectors::github::list_repositories(&token).await
    };
    as_value(list.map_err(app_error)?)
}
async fn github_get_pull_request(
    access: &dyn GithubAccess,
    arguments: Value,
) -> Result<Value, AppError> {
    let token = github_token(access, &arguments, GITHUB_READ).await?;
    as_value(
        crate::connectors::github::get_pull_request(
            &token,
            &required(&arguments, "owner")?,
            &required(&arguments, "repo")?,
            parse_number(&arguments)?,
        )
        .await
        .map_err(app_error)?,
    )
}

fn require_github_provider(arguments: &Value) -> Result<(), AppError> {
    if required(arguments, "provider")? == PROVIDER_GITHUB {
        Ok(())
    } else {
        Err(AppError::new(
            "invalid_arguments",
            "provider must be github.",
        ))
    }
}
async fn issues_search(access: &dyn GithubAccess, arguments: Value) -> Result<Value, AppError> {
    require_github_provider(&arguments)?;
    let token = github_token(access, &arguments, GITHUB_READ).await?;
    as_value(
        crate::connectors::github::search_issues(
            &token,
            &required(&arguments, "query")?,
            number(&arguments, "first"),
        )
        .await
        .map_err(app_error)?,
    )
}
async fn issues_get(access: &dyn GithubAccess, arguments: Value) -> Result<Value, AppError> {
    require_github_provider(&arguments)?;
    let token = github_token(access, &arguments, GITHUB_READ).await?;
    as_value(
        crate::connectors::github::get_issue(
            &token,
            &required(&arguments, "owner")?,
            &required(&arguments, "repo")?,
            parse_number(&arguments)?,
        )
        .await
        .map_err(app_error)?,
    )
}
async fn issues_comments(access: &dyn GithubAccess, arguments: Value) -> Result<Value, AppError> {
    require_github_provider(&arguments)?;
    let token = github_token(access, &arguments, GITHUB_READ).await?;
    as_value(
        crate::connectors::github::list_issue_comments(
            &token,
            &required(&arguments, "owner")?,
            &required(&arguments, "repo")?,
            parse_number(&arguments)?,
            number(&arguments, "first"),
        )
        .await
        .map_err(app_error)?,
    )
}
fn parse_number(arguments: &Value) -> Result<u64, AppError> {
    arguments
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| AppError::new("invalid_arguments", "number must be an integer."))
}

async fn issues_create(access: &dyn GithubAccess, arguments: Value) -> Result<Value, AppError> {
    require_github_provider(&arguments)?;
    let token = github_token(access, &arguments, GITHUB_WRITE).await?;
    let labels = string_list(&arguments, "labels");
    as_value(
        crate::connectors::github::create_issue(
            &token,
            &required(&arguments, "owner")?,
            &required(&arguments, "repo")?,
            &required(&arguments, "title")?,
            optional(&arguments, "body").as_deref(),
            labels.as_deref(),
        )
        .await
        .map_err(app_error)?,
    )
}
async fn issues_update(access: &dyn GithubAccess, arguments: Value) -> Result<Value, AppError> {
    require_github_provider(&arguments)?;
    let token = github_token(access, &arguments, GITHUB_WRITE).await?;
    let labels = string_list(&arguments, "labels");
    as_value(
        crate::connectors::github::update_issue(
            &token,
            &required(&arguments, "owner")?,
            &required(&arguments, "repo")?,
            parse_number(&arguments)?,
            optional(&arguments, "title").as_deref(),
            optional(&arguments, "body").as_deref(),
            labels.as_deref(),
        )
        .await
        .map_err(app_error)?,
    )
}
async fn issues_add_comment(
    access: &dyn GithubAccess,
    arguments: Value,
) -> Result<Value, AppError> {
    require_github_provider(&arguments)?;
    let token = github_token(access, &arguments, GITHUB_WRITE).await?;
    as_value(
        crate::connectors::github::add_comment(
            &token,
            &required(&arguments, "owner")?,
            &required(&arguments, "repo")?,
            parse_number(&arguments)?,
            &required(&arguments, "body")?,
        )
        .await
        .map_err(app_error)?,
    )
}
fn string_list(arguments: &Value, field: &str) -> Option<Vec<String>> {
    arguments
        .get(field)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(provider: ConnectorProvider, scopes: &[&str], teams: usize) -> ConnectorAccount {
        ConnectorAccount {
            account_id: format!("{provider:?}-account"),
            provider,
            email: "account@example.test".into(),
            scopes: scopes.iter().map(|scope| (*scope).into()).collect(),
            status: ConnectorAccountStatus::Connected,
            via_gh: false,
            workspace_name: None,
            workspace_url_key: None,
            selected_teams: (0..teams)
                .map(|index| crate::connectors::SelectedTeamDto {
                    id: format!("team-{index}"),
                    key: "ENG".into(),
                    name: "Engineering".into(),
                })
                .collect(),
        }
    }
    fn named(catalog: &[Value], name: &str) -> Value {
        catalog
            .iter()
            .find(|tool| tool["name"] == name)
            .cloned()
            .expect("tool present")
    }
    #[test]
    fn catalog_only_exposes_granted_connected_provider_tools() {
        let catalog = descriptors_from_accounts(&[
            account(
                ConnectorProvider::Google,
                &[GMAIL_READONLY, CALENDAR_READONLY],
                0,
            ),
            account(ConnectorProvider::Linear, &[policy::LINEAR_READ], 0),
            account(ConnectorProvider::Github, &[GITHUB_READ], 0),
        ]);
        assert!(catalog.iter().any(|tool| tool["name"] == "search_threads"));
        assert!(catalog.iter().any(|tool| tool["name"] == "list_events"));
        assert!(catalog
            .iter()
            .any(|tool| tool["name"] == "list_repositories"));
        assert!(!catalog.iter().any(|tool| tool["name"] == "list_projects"));
        assert!(!catalog.iter().any(|tool| tool["name"] == "create_event"));
    }

    #[test]
    fn notion_routine_toolset_matrix_never_grants_autonomy() {
        let reads = vec!["june_notion".to_string()];
        let actions = vec!["june_notion_actions".to_string()];
        let autonomous = vec!["notion-create-pages".to_string()];
        assert!(routine_tool_allowed("notion-search", &reads, &[], None));
        assert!(!routine_tool_allowed(
            "notion-create-pages",
            &reads,
            &autonomous,
            Some("approval")
        ));
        assert!(routine_tool_allowed(
            "notion-create-pages",
            &actions,
            &[],
            Some("approval")
        ));
        assert!(!routine_tool_allowed(
            "notion-create-pages",
            &actions,
            &[],
            Some("autonomous")
        ));
        assert!(!routine_tool_allowed(
            "notion-create-pages",
            &actions,
            &[],
            Some("read_only")
        ));
        assert!(!routine_tool_allowed(
            "notion-create-pages",
            &actions,
            &[],
            None
        ));
        assert!(!routine_tool_allowed(
            "notion-search",
            &actions,
            &[],
            Some("approval")
        ));
        assert!(!routine_tool_allowed(
            "notion-update-page",
            &[],
            &autonomous,
            Some("approval")
        ));
    }
    #[test]
    fn mutation_descriptors_require_broker_approval_and_keep_provider_choices_bounded() {
        let catalog = descriptors_from_accounts(&[
            account(ConnectorProvider::Linear, &[policy::LINEAR_WRITE], 1),
            account(ConnectorProvider::Github, &[GITHUB_WRITE], 0),
        ]);
        let create = named(&catalog, "create_issue");
        assert_eq!(create["requiresApproval"], true);
        assert_eq!(
            create["parameters"]["properties"]["provider"]["enum"],
            json!(["github"])
        );
        assert_eq!(
            create["parameters"]["properties"]["accountId"]["enum"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    #[test]
    fn connector_catalog_never_advertises_linear_accounts() {
        let mut stale = account(
            ConnectorProvider::Linear,
            &[policy::LINEAR_READ, policy::LINEAR_WRITE],
            1,
        );
        stale.status = ConnectorAccountStatus::ReconnectRequired;
        let unselected = account(
            ConnectorProvider::Linear,
            &[policy::LINEAR_READ, policy::LINEAR_WRITE],
            0,
        );
        let connected = account(
            ConnectorProvider::Linear,
            &[policy::LINEAR_READ, policy::LINEAR_WRITE],
            1,
        );
        let catalog = descriptors_from_accounts(&[stale, unselected, connected]);
        assert!(!catalog.iter().any(|tool| tool["name"] == "search_issues"));
        assert!(!catalog.iter().any(|tool| tool["name"] == "create_issue"));
        assert!(!catalog.iter().any(|tool| tool["name"] == "list_projects"));
        assert!(!catalog
            .iter()
            .any(|tool| tool["name"] == "create_project_update"));
    }
}
