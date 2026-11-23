//! Secret-free MCP error contract shared by native orchestration and portable OAuth.
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AgentMcpError {
    #[error("The original MCP request may have been applied. Inspect the server before retrying.")]
    AfterDelivery {
        #[source]
        cause: Box<AgentMcpError>,
    },
    #[error("{cause}")]
    Phase {
        phase: &'static str,
        #[source]
        cause: Box<AgentMcpError>,
    },
    #[error("MCP server definition is invalid: {0}")]
    InvalidDefinition(String),
    #[error("MCP server already exists")]
    DuplicateServer,
    #[error("The MCP connection changed. Reload Settings and retry the operation.")]
    ConnectionChanged,
    #[error("MCP server was not found")]
    NotFound,
    #[error("MCP tool is not available for this server")]
    ToolUnavailable,
    #[error("The MCP tool reported a failure. Check the server before retrying.")]
    ToolReportedError,
    #[error("MCP response exceeded Clovy's safety limit")]
    OutputTooLarge,
    #[error("MCP operation timed out")]
    TimedOut,
    #[error("MCP server returned an invalid protocol response")]
    Protocol,
    #[error("MCP secure storage is unavailable on this platform")]
    SecureStorageUnavailable,
    #[error("MCP secure storage operation failed")]
    SecureStorage,
    #[error("MCP storage operation failed")]
    Storage,
    #[error("MCP transport operation failed")]
    Transport,
    #[error("MCP server returned error code {0}")]
    Remote(i64),
    #[error("MCP authorization was rejected")]
    Unauthorized,
    #[error("MCP server returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("MCP authorization has expired. Reconnect this server in Settings.")]
    OauthReconnectRequired,
    #[error("OAuth connection was cancelled.")]
    OauthCancelled,
    #[error("OAuth credential cleanup is pending. Disconnect to retry cleanup.")]
    OauthCleanupPending,
    #[error("Local OAuth credentials were removed, but remote revocation failed. Revoke the grant at the provider.")]
    OauthRevokeFailed,
    #[error("Revocation of a rotated OAuth grant could not be confirmed. Revoke the grant at the provider.")]
    OauthRevokeUnconfirmed,
    #[error(
        "OAuth provider rejected this request ({0}). Review the client registration or reconnect."
    )]
    OauthProviderRejected(&'static str),
    #[error("The MCP request may have been applied. Check the server before retrying.")]
    DeliveryUncertain(Option<i64>),
    #[error("MCP server requested user input: {0}")]
    ElicitationRequired(String),
}

impl AgentMcpError {
    pub(crate) fn after_elicitation(self) -> Self {
        match self {
            error @ (Self::ElicitationRequired(_)
            | Self::DeliveryUncertain(_)
            | Self::AfterDelivery { .. }) => error,
            cause => Self::AfterDelivery {
                cause: Box::new(cause),
            },
        }
    }

    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::AfterDelivery { cause } => cause.code(),
            Self::Phase { cause, .. } => cause.code(),
            Self::InvalidDefinition(_) => "agent_mcp_invalid_definition",
            Self::DuplicateServer => "agent_mcp_duplicate_server",
            Self::ConnectionChanged => "agent_mcp_connection_changed",
            Self::NotFound => "agent_mcp_not_found",
            Self::ToolUnavailable => "agent_mcp_tool_unavailable",
            Self::ToolReportedError => "agent_mcp_tool_failed",
            Self::OutputTooLarge => "agent_mcp_output_too_large",
            Self::TimedOut => "agent_mcp_timeout",
            Self::Protocol => "agent_mcp_protocol_error",
            Self::SecureStorageUnavailable => "agent_mcp_secure_storage_unavailable",
            Self::SecureStorage => "agent_mcp_secure_storage_failed",
            Self::Storage => "agent_mcp_storage_failed",
            Self::Transport => "agent_mcp_transport_failed",
            Self::Remote(_) => "agent_mcp_remote_error",
            Self::Unauthorized => "agent_mcp_unauthorized",
            Self::HttpStatus(_) => "agent_mcp_http_error",
            Self::OauthReconnectRequired => "agent_mcp_reconnect_required",
            Self::OauthCancelled => "agent_mcp_oauth_cancelled",
            Self::OauthCleanupPending => "agent_mcp_oauth_cleanup_pending",
            Self::OauthRevokeFailed => "agent_mcp_oauth_revoke_failed",
            Self::OauthRevokeUnconfirmed => "agent_mcp_oauth_revoke_unconfirmed",
            Self::OauthProviderRejected(_) => "agent_mcp_oauth_provider_rejected",
            Self::DeliveryUncertain(_) => "agent_mcp_delivery_uncertain",
            Self::ElicitationRequired(_) => "agent_mcp_elicitation_required",
        }
    }
}
