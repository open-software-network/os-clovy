//! Safe, host-owned context for MCP failures. Provider response bodies and
//! credential material never become diagnostics.
use super::AgentMcpError;
use crate::domain::types::AppError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMcpDiagnostic {
    pub origin: String,
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub phase: String,
    pub code: String,
    pub state: String,
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_code: Option<i64>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_safe: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<Box<AgentMcpDiagnostic>>,
}

impl AgentMcpDiagnostic {
    pub(crate) fn into_app_error(self) -> AppError {
        let remote_code = self
            .remote_code
            .map_or_else(String::new, |code| format!("; remote code: {code}"));
        let context = format!(
            "MCP origin: {}; server: {}; tool: {}; phase: {}; code: {}{}; state: {}; action: {}. {}",
            self.origin,
            self.server,
            self.tool.as_deref().unwrap_or("none"),
            self.phase,
            self.code,
            remote_code,
            self.state,
            self.action,
            self.message
        );
        let mut error = AppError::new(self.code.clone(), context);
        error.details = serde_json::to_value(self).ok();
        error
    }
}

impl AgentMcpError {
    pub(crate) fn diagnostic(
        &self,
        server: &str,
        tool: Option<&str>,
        phase: &str,
    ) -> AgentMcpDiagnostic {
        if let Self::AfterDelivery { cause } = self {
            let mut diagnostic = cause.diagnostic(server, tool, phase);
            diagnostic.action = "inspect_server_before_retry".into();
            diagnostic.delivery = Some("unknown".into());
            diagnostic.retry_safe = Some(false);
            diagnostic.message.push_str(
                " The original MCP request may have been applied. Inspect the server before retrying.",
            );
            return diagnostic;
        }
        if let Self::Phase { phase, cause } = self {
            return cause.diagnostic(server, tool, phase);
        }
        let (state, action) = match self {
            Self::OauthReconnectRequired => ("reconnect_required", "reconnect"),
            Self::OauthProviderRejected("access_denied") => {
                ("unavailable", "review_provider_consent")
            }
            Self::OauthProviderRejected("invalid_client" | "unauthorized_client") => {
                ("unavailable", "check_provider_client_approval")
            }
            Self::OauthProviderRejected(_) => ("unavailable", "configure"),
            Self::OauthCleanupPending => ("unavailable", "retry_local_cleanup"),
            Self::OauthRevokeFailed => ("unavailable", "revoke_at_provider"),
            Self::OauthRevokeUnconfirmed => ("unavailable", "revoke_at_provider"),
            Self::OauthCancelled => ("unavailable", "connect"),
            Self::Unauthorized => ("unavailable", "check_permission_or_connection"),
            Self::DeliveryUncertain(_) => ("unavailable", "inspect_server_before_retry"),
            Self::ToolReportedError => ("unavailable", "inspect_server_before_retry"),
            Self::HttpStatus(403) => ("unavailable", "check_permission_or_connection"),
            Self::HttpStatus(_) => ("unavailable", "inspect_server"),
            Self::SecureStorage | Self::SecureStorageUnavailable | Self::Storage => {
                ("unavailable", "check_local_storage")
            }
            Self::TimedOut | Self::Transport => ("unavailable", "check_connection"),
            Self::Remote(_) => ("unavailable", "inspect_server"),
            Self::ElicitationRequired(_) => ("unavailable", "answer_in_chat"),
            _ => ("unavailable", "configure"),
        };
        let message = match self {
            // These variants can contain a provider's free-form text. Keep
            // the cause typed without copying that payload into UI or logs.
            Self::InvalidDefinition(_) => "The MCP server configuration is invalid.".into(),
            Self::ElicitationRequired(_) => "The MCP server requires an answer in chat.".into(),
            _ => self.to_string(),
        };
        AgentMcpDiagnostic {
            origin: "clovy_managed".into(),
            server: server.into(),
            tool: tool.map(str::to_owned),
            phase: phase.into(),
            code: self.code().into(),
            state: state.into(),
            action: action.into(),
            remote_code: match self {
                Self::Remote(code) => Some(*code),
                Self::Unauthorized => Some(401),
                Self::HttpStatus(code) => Some(i64::from(*code)),
                Self::DeliveryUncertain(code) => *code,
                _ => None,
            },
            message,
            delivery: matches!(self, Self::DeliveryUncertain(_)).then(|| "unknown".into()),
            retry_safe: matches!(self, Self::DeliveryUncertain(_)).then_some(false),
            cause: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_failure_keeps_cause_and_cannot_be_retried() {
        let initial = AgentMcpError::Unauthorized.diagnostic("Linear", Some("send"), "call");
        assert_eq!(initial.retry_safe, None);
        let resumed = AgentMcpError::Unauthorized.after_elicitation().diagnostic(
            "Linear",
            Some("send"),
            "call",
        );
        assert_eq!(resumed.code, initial.code);
        assert_eq!(resumed.remote_code, initial.remote_code);
        assert_eq!(resumed.delivery.as_deref(), Some("unknown"));
        assert_eq!(resumed.retry_safe, Some(false));
        assert_eq!(resumed.action, "inspect_server_before_retry");
        assert!(matches!(
            AgentMcpError::DeliveryUncertain(Some(401)).after_elicitation(),
            AgentMcpError::DeliveryUncertain(Some(401))
        ));
    }

    #[test]
    fn post_delivery_diagnostic_preserves_cause_and_forbids_retry() {
        let error = AgentMcpError::AfterDelivery {
            cause: Box::new(AgentMcpError::Remote(-32601)),
        }
        .diagnostic("synthetic-server", Some("send"), "call")
        .into_app_error();
        let details = error.details.unwrap();
        assert_eq!(error.code, "agent_mcp_remote_error");
        assert_eq!(details["remoteCode"], -32601);
        assert_eq!(details["action"], "inspect_server_before_retry");
        assert_eq!(details["delivery"], "unknown");
        assert_eq!(details["retrySafe"], false);
    }

    #[test]
    fn auth_transport_and_remote_failures_keep_distinct_recovery_actions() {
        let reconnect = AgentMcpError::OauthReconnectRequired.diagnostic("Vercel", None, "refresh");
        assert_eq!(reconnect.state, "reconnect_required");
        assert_eq!(reconnect.action, "reconnect");
        let forbidden = AgentMcpError::Unauthorized.diagnostic("private", None, "discover");
        assert_eq!(forbidden.state, "unavailable");
        assert_ne!(forbidden.action, "reconnect");
        let remote = AgentMcpError::Remote(-32601)
            .diagnostic("private", Some("lookup"), "call")
            .into_app_error();
        assert_eq!(remote.code, "agent_mcp_remote_error");
        assert_eq!(remote.details.as_ref().unwrap()["remoteCode"], -32601);
        assert!(remote.message.contains("phase: call"));
        assert!(remote.message.contains("server: private"));
        assert_eq!(
            AgentMcpError::DeliveryUncertain(None)
                .diagnostic("private", Some("send"), "call")
                .action,
            "inspect_server_before_retry"
        );
        assert_eq!(
            AgentMcpError::OauthProviderRejected("access_denied")
                .diagnostic("Vercel", None, "connect")
                .action,
            "review_provider_consent"
        );
        assert_eq!(
            AgentMcpError::OauthProviderRejected("invalid_scope")
                .diagnostic("Vercel", None, "connect")
                .action,
            "configure"
        );
    }

    #[test]
    fn diagnostic_does_not_copy_untrusted_error_payloads() {
        for error in [
            AgentMcpError::InvalidDefinition("fixture-private-payload".into()),
            AgentMcpError::ElicitationRequired("fixture-private-payload".into()),
            AgentMcpError::AfterDelivery {
                cause: Box::new(AgentMcpError::InvalidDefinition(
                    "fixture-private-payload".into(),
                )),
            },
        ] {
            let diagnostic = error.diagnostic("private", None, "discover");
            let encoded = serde_json::to_string(&diagnostic).unwrap();
            assert!(!encoded.contains("fixture-private-payload"));
            let app_error = diagnostic.into_app_error();
            assert!(!app_error.message.contains("fixture-private-payload"));
        }
    }

    #[test]
    fn nested_refresh_failure_preserves_actual_phase_code_and_remote_cause() {
        let error = AgentMcpError::Phase {
            phase: "refresh",
            cause: Box::new(AgentMcpError::Remote(-32001)),
        };
        let diagnostic = error.diagnostic("Vercel", Some("mcp_vercel_lookup"), "call");
        assert_eq!(diagnostic.phase, "refresh");
        assert_eq!(diagnostic.remote_code, Some(-32001));
        assert_eq!(diagnostic.code, error.code());
        assert_eq!(diagnostic.code, "agent_mcp_remote_error");
        assert_eq!(diagnostic.message, "MCP server returned error code -32001");
    }
}
