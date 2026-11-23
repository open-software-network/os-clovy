use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentApprovalMode {
    Approvals,
    Bypass,
}

impl AgentApprovalMode {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Approvals => "approvals",
            Self::Bypass => "bypass",
        }
    }
}

impl From<&str> for AgentApprovalMode {
    fn from(value: &str) -> Self {
        match value {
            "bypass" => Self::Bypass,
            _ => Self::Approvals,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentSafetyMode {
    Sandboxed,
    Unrestricted,
}

impl AgentSafetyMode {
    pub fn as_db(self) -> &'static str {
        match self {
            Self::Sandboxed => "sandboxed",
            Self::Unrestricted => "unrestricted",
        }
    }
}

impl From<&str> for AgentSafetyMode {
    fn from(value: &str) -> Self {
        match value {
            "unrestricted" => Self::Unrestricted,
            _ => Self::Sandboxed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSessionDto {
    pub id: String,
    pub title: String,
    pub status: String,
    pub model: String,
    pub safety_mode: AgentSafetyMode,
    pub workspace_path: Option<String>,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRunDto {
    pub id: String,
    pub session_id: String,
    pub status: String,
    pub model: String,
    pub approval_mode: AgentApprovalMode,
    pub reasoning_effort: Option<String>,
    pub started_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
    pub usage: Option<Value>,
    pub interrupted_state: Option<Value>,
    pub last_sequence: i64,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum AgentItemPayload {
    UserMessage(MessagePayload),
    AssistantMessage(MessagePayload),
    SystemMessage(MessagePayload),
    Reasoning(TextPayload),
    Steering(TextPayload),
    ContextSummary(TextPayload),
    ToolCall(ToolPayload),
    ToolResult(ToolPayload),
    Interruption(Value),
    Error(Value),
}

impl AgentItemPayload {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::UserMessage(_) => "user_message",
            Self::AssistantMessage(_) => "assistant_message",
            Self::SystemMessage(_) => "system_message",
            Self::Reasoning(_) => "reasoning",
            Self::Steering(_) => "steering",
            Self::ContextSummary(_) => "context_summary",
            Self::ToolCall(_) => "tool_call",
            Self::ToolResult(_) => "tool_result",
            Self::Interruption(_) => "interruption",
            Self::Error(_) => "error",
        }
    }

    pub fn value(&self) -> Result<Value, serde_json::Error> {
        match self {
            Self::UserMessage(value)
            | Self::AssistantMessage(value)
            | Self::SystemMessage(value) => serde_json::to_value(value),
            Self::Reasoning(value) | Self::Steering(value) | Self::ContextSummary(value) => {
                serde_json::to_value(value)
            }
            Self::ToolCall(value) | Self::ToolResult(value) => serde_json::to_value(value),
            Self::Interruption(value) | Self::Error(value) => Ok(value.clone()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessagePayload {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<MessageAttachmentPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachmentPayload {
    pub id: String,
    pub name: String,
    pub path: String,
    pub mime_type: Option<String>,
    pub size_bytes: i64,
    pub available: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TextPayload {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPayload {
    pub tool_name: Option<String>,
    pub tool_call_id: Option<String>,
    pub arguments: Option<Value>,
    pub result: Option<Value>,
    /// Host-normalized failure, separate from the raw observed result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentItemDto {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub sequence: i64,
    pub payload: AgentItemPayload,
    pub external_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentArtifactDto {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub item_id: Option<String>,
    pub provenance: String,
    pub action: String,
    pub path: String,
    pub original_path: Option<String>,
    pub display_name: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub available: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkillDto {
    pub id: String,
    pub enabled: bool,
    pub managed: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AgentInterruptionDto {
    pub id: String,
    pub session_id: String,
    pub run_id: String,
    pub interruption_type: String,
    pub payload: Value,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MigrationCounts {
    pub sessions: u64,
    pub messages: u64,
    pub reasoning_items: u64,
    pub artifacts: u64,
    #[serde(default)]
    pub routines: u64,
    #[serde(default)]
    pub mcp_servers: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentMigrationManifestDto {
    pub migration_key: String,
    pub source_path: String,
    pub source_fingerprint: Option<String>,
    pub status: String,
    pub source_counts: MigrationCounts,
    pub imported_counts: MigrationCounts,
    pub skipped_count: u64,
    pub errors: Vec<String>,
    pub started_at: String,
    pub completed_at: Option<String>,
}
