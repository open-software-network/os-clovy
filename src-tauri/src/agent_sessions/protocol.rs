use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RpcFrame {
    pub jsonrpc: String,
    pub protocol_version: u32,
    pub session_id: String,
    pub run_id: String,
    pub sequence: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl RpcFrame {
    pub fn request(
        id: String,
        method: &str,
        session_id: &str,
        run_id: &str,
        sequence: i64,
        params: Value,
    ) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            protocol_version: PROTOCOL_VERSION,
            session_id: session_id.into(),
            run_id: run_id.into(),
            sequence,
            id: Some(id),
            event_id: None,
            method: Some(method.into()),
            params: Some(params),
            result: None,
            error: None,
        }
    }

    pub fn success(request: &Self, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            protocol_version: PROTOCOL_VERSION,
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            sequence: request.sequence,
            id: request.id.clone(),
            event_id: None,
            method: None,
            params: None,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(request: &Self, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            protocol_version: PROTOCOL_VERSION,
            session_id: request.session_id.clone(),
            run_id: request.run_id.clone(),
            sequence: request.sequence,
            id: request.id.clone(),
            event_id: None,
            method: None,
            params: None,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.jsonrpc != "2.0" {
            return Err("unsupported JSON-RPC version".into());
        }
        if self.protocol_version != PROTOCOL_VERSION {
            return Err(format!(
                "unsupported agent protocol version {}",
                self.protocol_version
            ));
        }
        if self.sequence < 0 {
            return Err("sequence must be non-negative".into());
        }
        if self.session_id.is_empty() || self.run_id.is_empty() {
            return Err("sessionId and runId are required".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_protocol_versions() {
        let mut frame = RpcFrame::request(
            "1".into(),
            "session.inspect",
            "session",
            "run",
            0,
            serde_json::json!({}),
        );
        frame.protocol_version = 2;
        assert!(frame
            .validate()
            .unwrap_err()
            .contains("unsupported agent protocol"));
    }

    #[test]
    fn frame_uses_camel_case_wire_fields() {
        let value = serde_json::to_value(RpcFrame::request(
            "1".into(),
            "run.cancel",
            "s",
            "r",
            3,
            serde_json::json!({}),
        ))
        .unwrap();
        assert_eq!(value["protocolVersion"], 1);
        assert_eq!(value["sessionId"], "s");
    }
}
