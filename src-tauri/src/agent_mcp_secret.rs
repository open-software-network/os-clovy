//! Zeroized secret material for MCP configuration and OAuth custody.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use zeroize::Zeroize;

/// Values persist only in the credential store. Failed writes may retain a
/// zeroized-on-drop in-memory candidate for explicit cleanup. These values are
/// never part of a server definition or runtime tool descriptor.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpSecretBundle {
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub oauth: BTreeMap<String, String>,
}
impl std::fmt::Debug for McpSecretBundle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpSecretBundle")
            .field("env_keys", &self.env.keys().collect::<Vec<_>>())
            .field("header_keys", &self.headers.keys().collect::<Vec<_>>())
            .field("oauth_keys", &self.oauth.keys().collect::<Vec<_>>())
            .finish()
    }
}
impl Drop for McpSecretBundle {
    fn drop(&mut self) {
        for value in self.env.values_mut() {
            value.zeroize();
        }
        for value in self.headers.values_mut() {
            value.zeroize();
        }
        for value in self.oauth.values_mut() {
            value.zeroize();
        }
    }
}
