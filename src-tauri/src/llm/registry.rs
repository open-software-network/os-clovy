//! Persistent provider registry: named OpenAI-compatible endpoints, the
//! provider chosen for each use, and measured structured-output levels. It
//! lives inside `provider-settings.json` (see `crate::providers`); endpoint API
//! keys live only in the Keychain ([`super::secrets`]).
//!
//! Chat accepts only CLI engines. The text model is independent and serves
//! note generation when the selected note engine is OpenRouter.

use super::{
    cli::CliKind, endpoint::EndpointConnection, secrets::SecretStore, LlmError,
    StructuredOutputLevel,
};
use crate::{
    domain::types::AppError,
    providers::{LocalGenerationSettings, ProviderModelSettings, PROVIDER_LOCAL},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Id given to the endpoint migrated from the legacy single local endpoint.
pub const LEGACY_LOCAL_ENDPOINT_ID: &str = "local";
const LEGACY_LOCAL_ENDPOINT_NAME: &str = "Local model";
const MAX_ENDPOINT_NAME_CHARS: usize = 80;
const MAX_API_KEY_CHARS: usize = 4_096;

/// Which provider serves a use. OpenRouter uses the user's own key for notes;
/// `None` means off and is only allowed for activity.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ProviderRef {
    #[serde(rename = "openrouter", alias = "clovy")]
    OpenRouter,
    None,
    Endpoint {
        id: String,
    },
    Cli {
        id: CliKind,
    },
}

impl ProviderRef {
    /// Stable key for logs and the stored `provider` of generated notes.
    pub fn key(&self) -> String {
        match self {
            ProviderRef::OpenRouter => "openrouter".to_string(),
            ProviderRef::None => "none".to_string(),
            ProviderRef::Endpoint { id } => format!("endpoint:{id}"),
            ProviderRef::Cli { id } => format!("cli:{}", id.id()),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LlmUsage {
    Chat,
    Notes,
    DictationCleanup,
    Activity,
}

impl LlmUsage {
    fn default_provider(self) -> ProviderRef {
        match self {
            LlmUsage::Activity => ProviderRef::None,
            LlmUsage::Chat => claude(),
            _ => ProviderRef::OpenRouter,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LlmUsageSelection {
    #[serde(default = "claude")]
    pub chat: ProviderRef,
    #[serde(default = "openrouter")]
    pub notes: ProviderRef,
    // Keep the internal usage API stable; cleanup is fixed and never persisted.
    #[serde(skip, default = "openrouter")]
    pub dictation_cleanup: ProviderRef,
    #[serde(default = "none")]
    pub activity: ProviderRef,
}

fn openrouter() -> ProviderRef {
    ProviderRef::OpenRouter
}

fn claude() -> ProviderRef {
    ProviderRef::Cli {
        id: CliKind::Claude,
    }
}

fn none() -> ProviderRef {
    ProviderRef::None
}

impl Default for LlmUsageSelection {
    fn default() -> Self {
        Self {
            chat: claude(),
            notes: ProviderRef::OpenRouter,
            dictation_cleanup: ProviderRef::OpenRouter,
            activity: ProviderRef::None,
        }
    }
}

impl LlmUsageSelection {
    pub fn get(&self, usage: LlmUsage) -> &ProviderRef {
        match usage {
            LlmUsage::Chat => &self.chat,
            LlmUsage::Notes => &self.notes,
            LlmUsage::DictationCleanup => &self.dictation_cleanup,
            LlmUsage::Activity => &self.activity,
        }
    }

    fn slot(&mut self, usage: LlmUsage) -> &mut ProviderRef {
        match usage {
            LlmUsage::Chat => &mut self.chat,
            LlmUsage::Notes => &mut self.notes,
            LlmUsage::DictationCleanup => &mut self.dictation_cleanup,
            LlmUsage::Activity => &mut self.activity,
        }
    }
}

/// A named endpoint as persisted. The API key is not here: `has_api_key`
/// only records that the Keychain holds one under the endpoint id.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LlmEndpointRecord {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    #[serde(default)]
    pub has_api_key: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<StructuredOutputLevel>,
    /// Requests per minute for one-shot calls; absent or 0 means unmetered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpm_limit: Option<u32>,
}

/// The client view of an endpoint: never carries the key.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LlmEndpointDto {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    pub has_api_key: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<StructuredOutputLevel>,
}

impl From<&LlmEndpointRecord> for LlmEndpointDto {
    fn from(record: &LlmEndpointRecord) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            base_url: record.base_url.clone(),
            model_id: record.model_id.clone(),
            has_api_key: record.has_api_key,
            structured_output: record.structured_output,
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LlmProvidersDto {
    pub endpoints: Vec<LlmEndpointDto>,
    pub usage: LlmUsageSelection,
    pub cli_levels: BTreeMap<CliKind, StructuredOutputLevel>,
    /// Chat still runs on the legacy Clovy agent only because neither
    /// Claude Code nor Codex was found ([`apply_cli_default`] retries on the
    /// next launch).
    pub cli_default_pending: bool,
}

impl From<&ProviderModelSettings> for LlmProvidersDto {
    fn from(settings: &ProviderModelSettings) -> Self {
        Self {
            endpoints: settings
                .llm_endpoints
                .iter()
                .map(LlmEndpointDto::from)
                .collect(),
            usage: settings.llm_usage.clone(),
            cli_levels: settings.llm_cli_levels.clone(),
            cli_default_pending: cli_default_pending(settings),
        }
    }
}

/// Read-only snapshot used to route one generation call.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LlmRegistry {
    pub endpoints: Vec<LlmEndpointRecord>,
    pub usage: LlmUsageSelection,
    pub cli_levels: BTreeMap<CliKind, StructuredOutputLevel>,
    /// The legacy endpoint key, kept on disk only while the Keychain refused
    /// it during migration.
    pub legacy_local_api_key: Option<String>,
}

impl From<&ProviderModelSettings> for LlmRegistry {
    fn from(settings: &ProviderModelSettings) -> Self {
        let legacy_key = settings.local_generation.api_key.trim();
        Self {
            endpoints: settings.llm_endpoints.clone(),
            usage: settings.llm_usage.clone(),
            cli_levels: settings.llm_cli_levels.clone(),
            legacy_local_api_key: (!legacy_key.is_empty()).then(|| legacy_key.to_string()),
        }
    }
}

impl LlmRegistry {
    pub fn endpoint(&self, id: &str) -> Option<&LlmEndpointRecord> {
        self.endpoints.iter().find(|endpoint| endpoint.id == id)
    }

    /// Last measured level; `None` when never tested.
    pub fn level_of(&self, provider: &ProviderRef) -> Option<StructuredOutputLevel> {
        match provider {
            ProviderRef::Endpoint { id } => self.endpoint(id)?.structured_output,
            ProviderRef::Cli { id } => self.cli_levels.get(id).copied(),
            ProviderRef::OpenRouter | ProviderRef::None => None,
        }
    }

    pub fn connection(
        &self,
        id: &str,
        store: &dyn SecretStore,
    ) -> Result<EndpointConnection, LlmError> {
        let endpoint = self
            .endpoint(id)
            .ok_or_else(|| LlmError::EndpointNotFound(id.to_string()))?;
        let api_key = if endpoint.has_api_key {
            match store.get(&endpoint.id) {
                Ok(Some(key)) => Some(key),
                Ok(None) | Err(_) if endpoint.id == LEGACY_LOCAL_ENDPOINT_ID => {
                    self.legacy_local_api_key.clone()
                }
                Ok(None) => None,
                Err(error) => {
                    tracing::warn!(endpoint = %endpoint.id, error = %error, "endpoint key unavailable");
                    None
                }
            }
        } else {
            None
        };
        Ok(EndpointConnection {
            id: endpoint.id.clone(),
            base_url: endpoint.base_url.clone(),
            model_id: endpoint.model_id.clone(),
            api_key,
            rpm_limit: endpoint.rpm_limit.unwrap_or(0),
        })
    }

    /// The endpoint an agent request tagged with `model_id` should use: the
    /// chat endpoint when it serves that model, otherwise the first endpoint
    /// that does. Sessions keep working after the user adds endpoints.
    pub fn endpoint_for_model(&self, model_id: &str) -> Option<&LlmEndpointRecord> {
        let model_id = model_id.trim();
        if model_id.is_empty() {
            return None;
        }
        let chat = match &self.usage.chat {
            ProviderRef::Endpoint { id } => self.endpoint(id),
            _ => None,
        };
        chat.filter(|endpoint| endpoint.model_id == model_id)
            .or_else(|| {
                self.endpoints
                    .iter()
                    .find(|endpoint| endpoint.model_id == model_id)
            })
    }
}

/// Converts a connection into the legacy shape the agent and note routes use.
pub fn legacy_settings(connection: Option<EndpointConnection>) -> LocalGenerationSettings {
    connection
        .map(|connection| LocalGenerationSettings {
            base_url: connection.base_url,
            model_id: connection.model_id,
            api_key: connection.api_key.unwrap_or_default(),
        })
        .unwrap_or_default()
}

/// One-time move of the legacy single `localGeneration` endpoint into the
/// registry. Idempotent: it never duplicates the endpoint, and when the
/// Keychain refuses the key the legacy key stays on disk (still usable via
/// [`LlmRegistry::connection`]) and the move is retried on the next launch.
pub fn migrate_legacy_local_generation(
    mut settings: ProviderModelSettings,
    store: &dyn SecretStore,
) -> ProviderModelSettings {
    if settings.llm_registry_migrated {
        return settings;
    }
    let legacy = settings.local_generation.clone();
    let base_url = crate::providers::normalize_local_base_url(&legacy.base_url).ok();
    let model_id = legacy.model_id.trim().to_string();
    let api_key = legacy.api_key.trim().to_string();
    let (Some(base_url), false) = (base_url, model_id.is_empty()) else {
        settings.local_generation = LocalGenerationSettings::default();
        settings.llm_registry_migrated = true;
        return settings;
    };
    if settings.endpoint(LEGACY_LOCAL_ENDPOINT_ID).is_none() {
        settings.llm_endpoints.insert(
            0,
            LlmEndpointRecord {
                id: LEGACY_LOCAL_ENDPOINT_ID.to_string(),
                name: LEGACY_LOCAL_ENDPOINT_NAME.to_string(),
                base_url,
                model_id,
                has_api_key: !api_key.is_empty(),
                structured_output: None,
                rpm_limit: None,
            },
        );
        if settings.generation_provider == PROVIDER_LOCAL {
            // The legacy toggle drove both the agent and note generation.
            let local = ProviderRef::Endpoint {
                id: LEGACY_LOCAL_ENDPOINT_ID.to_string(),
            };
            settings.llm_usage.chat = local.clone();
            settings.llm_usage.notes = local;
        }
    }
    let key_moved = api_key.is_empty() || store.set(LEGACY_LOCAL_ENDPOINT_ID, &api_key).is_ok();
    if key_moved {
        settings.local_generation = LocalGenerationSettings::default();
        settings.llm_registry_migrated = true;
    } else {
        tracing::warn!("keychain refused the local endpoint key; migration retries next launch");
    }
    settings
}

/// Existing unattended eligibility, kept separate from automatic chat choice.
/// Do not admit another engine until its unattended policy is qualified.
pub const UNATTENDED_CLIS: [CliKind; 2] = [CliKind::Claude, CliKind::Codex];

/// Resolve unattended work independently of chat and Notes. Explicit choices
/// must be both eligible and installed; they never fall back to another engine.
pub fn resolve_unattended_cli(
    model: &str,
    installed: impl Fn(CliKind) -> bool,
) -> Result<CliKind, AppError> {
    if matches!(model, "" | "auto" | "open-software/auto") {
        return UNATTENDED_CLIS
            .into_iter()
            .find(|kind| installed(*kind))
            .ok_or_else(|| {
                AppError::new(
                    "routine_engine_unavailable",
                    "Install an eligible CLI engine before starting this routine.",
                )
            });
    }
    let kind = crate::chat_engine::cli_for_model(model)
        .filter(|kind| UNATTENDED_CLIS.contains(kind))
        .ok_or_else(|| {
            AppError::new(
                "routine_engine_not_allowed",
                "This engine is not eligible for unattended routines.",
            )
        })?;
    if !installed(kind) {
        return Err(LlmError::CliNotInstalled(kind).into());
    }
    Ok(kind)
}

/// Automatic chat preference follows catalog order: Claude Code, Codex, then
/// Antigravity. This selection never starts a CLI or changes an explicit choice.
pub fn default_chat_cli(installed: impl Fn(CliKind) -> bool) -> Option<CliKind> {
    CliKind::ALL
        .into_iter()
        .filter(|kind| matches!(kind, CliKind::Claude | CliKind::Codex | CliKind::Agy))
        .find(|kind| installed(*kind))
}

/// Retire native/endpoint chat selections even if the earlier CLI-default
/// migration was recorded. A CLI choice survives. When no eligible CLI is installed,
/// select Claude Code so its normal setup error is shown without another route.
pub fn apply_cli_default(
    settings: &mut ProviderModelSettings,
    installed: impl Fn(CliKind) -> bool,
) -> Option<CliKind> {
    if let ProviderRef::Cli { id } = settings.llm_usage.chat {
        if settings.llm_cli_default_applied || id != CliKind::Claude {
            settings.llm_cli_default_applied = true;
            return None;
        }
    }
    let kind = default_chat_cli(installed).unwrap_or(CliKind::Claude);
    settings.llm_usage.chat = ProviderRef::Cli { id: kind };
    settings.llm_cli_default_applied = true;
    normalize(settings);
    Some(kind)
}

/// The saved default has not yet been resolved against installed CLIs.
pub fn cli_default_pending(settings: &ProviderModelSettings) -> bool {
    !settings.llm_cli_default_applied
}

/// Drops selections that point at missing endpoints and invalid kinds, then
/// retires native chat without changing the independent text-model selection.
pub fn normalize(settings: &mut ProviderModelSettings) {
    if !matches!(settings.llm_usage.chat, ProviderRef::Cli { .. }) {
        settings.llm_usage.chat = ProviderRef::Cli {
            id: CliKind::Claude,
        };
        settings.llm_cli_default_applied = false;
    }
    for usage in [
        LlmUsage::Notes,
        LlmUsage::DictationCleanup,
        LlmUsage::Activity,
    ] {
        let current = settings.llm_usage.get(usage).clone();
        let valid = match &current {
            ProviderRef::OpenRouter => usage != LlmUsage::Activity,
            // Dictation cleanup always uses OpenRouter directly.
            _ if usage == LlmUsage::DictationCleanup => false,
            ProviderRef::Endpoint { id } => settings.endpoint(id).is_some(),
            ProviderRef::None => usage == LlmUsage::Activity,
            ProviderRef::Cli { .. } => true,
        };
        if !valid {
            *settings.llm_usage.slot(usage) = usage.default_provider();
        }
    }
}

/// Validates and applies a usage choice. `installed` reports whether a CLI
/// is present on this machine.
pub fn set_usage(
    settings: &mut ProviderModelSettings,
    usage: LlmUsage,
    provider: ProviderRef,
    installed: impl Fn(CliKind) -> bool,
) -> Result<(), AppError> {
    if usage == LlmUsage::Chat && !matches!(provider, ProviderRef::Cli { .. }) {
        return Err(AppError::new(
            "llm_provider_not_allowed",
            "Chat uses a CLI engine. Choose Claude Code or another installed CLI.",
        ));
    }
    let registry = LlmRegistry::from(&*settings);
    match &provider {
        // Dictation cleanup is a single OpenRouter chat completion (the model
        // is chosen in the dictation settings), never a CLI or an endpoint.
        ProviderRef::Endpoint { .. } | ProviderRef::Cli { .. } | ProviderRef::None
            if usage == LlmUsage::DictationCleanup =>
        {
            return Err(AppError::new(
                "llm_provider_not_allowed",
                "Dictation cleanup runs only on an OpenRouter model. Choose it in the dictation settings.",
            ));
        }
        ProviderRef::OpenRouter if usage == LlmUsage::Activity => {
            return Err(AppError::new(
                "llm_provider_not_allowed",
                "Activity features use a CLI or registered endpoint. Choose one of those providers or none.",
            ));
        }
        ProviderRef::None if usage != LlmUsage::Activity => {
            return Err(AppError::new(
                "llm_provider_not_allowed",
                "Choose a provider for this use.",
            ));
        }
        ProviderRef::Endpoint { id } if registry.endpoint(id).is_none() => {
            return Err(LlmError::EndpointNotFound(id.clone()).into());
        }
        ProviderRef::Cli { id } if !installed(*id) => {
            return Err(LlmError::CliNotInstalled(*id).into());
        }
        // One-shot uses send content (transcripts, notes) to the CLI; chat
        // goes through the CLI chat engine, which has its own contract.
        ProviderRef::Cli { id } if usage != LlmUsage::Chat && !id.one_shot_allowed() => {
            return Err(LlmError::CliToolsNotDisabled(*id).into());
        }
        _ => {}
    }
    if usage == LlmUsage::Activity && provider != ProviderRef::None {
        require_json_schema(&registry, &provider)?;
    }
    *settings.llm_usage.slot(usage) = provider;
    if usage == LlmUsage::Chat {
        // An explicit CLI choice is preserved across launches.
        settings.llm_cli_default_applied = true;
    }
    normalize(settings);
    Ok(())
}

/// Features that need structured JSON refuse providers below `json_schema`.
pub fn require_json_schema(registry: &LlmRegistry, provider: &ProviderRef) -> Result<(), AppError> {
    if registry
        .level_of(provider)
        .is_some_and(StructuredOutputLevel::supports_json_schema)
    {
        Ok(())
    } else {
        Err(LlmError::StructuredOutputInsufficient.into())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveEndpointRequest {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub model_id: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub clear_api_key: bool,
}

/// Creates or updates an endpoint. The key is written to `store` before the
/// settings change, so a Keychain failure leaves everything untouched.
pub fn save_endpoint(
    settings: &mut ProviderModelSettings,
    request: SaveEndpointRequest,
    store: &dyn SecretStore,
) -> Result<String, AppError> {
    let name = request.name.trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_ENDPOINT_NAME_CHARS {
        return Err(AppError::new(
            "llm_endpoint_name_invalid",
            "Enter a name of up to 80 characters.",
        ));
    }
    let base_url = crate::providers::normalize_local_base_url(&request.base_url)?;
    let model_id = request.model_id.trim().to_string();
    if model_id.is_empty() {
        return Err(AppError::new(
            "llm_endpoint_model_required",
            "Enter a model ID.",
        ));
    }
    let api_key = request
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if api_key.is_some_and(|key| {
        key.chars().count() > MAX_API_KEY_CHARS || key.chars().any(char::is_control)
    }) {
        return Err(AppError::new(
            "llm_endpoint_key_invalid",
            "Enter a valid API key.",
        ));
    }
    let existing = match request
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        Some(id) => Some(
            settings
                .endpoint(id)
                .cloned()
                .ok_or_else(|| AppError::from(LlmError::EndpointNotFound(id.to_string())))?,
        ),
        None => None,
    };
    let id = existing
        .as_ref()
        .map(|endpoint| endpoint.id.clone())
        .unwrap_or_else(|| format!("ep-{}", uuid::Uuid::new_v4().simple()));
    let has_api_key = if let Some(key) = api_key {
        store.set(&id, key).map_err(|_| keychain_error())?;
        if id == LEGACY_LOCAL_ENDPOINT_ID {
            // The new key is in the Keychain; a legacy key still waiting
            // for migration must not be moved over it on the next launch.
            settings.local_generation = LocalGenerationSettings::default();
            settings.llm_registry_migrated = true;
        }
        true
    } else if request.clear_api_key {
        store.delete(&id).map_err(|_| keychain_error())?;
        if id == LEGACY_LOCAL_ENDPOINT_ID {
            settings.local_generation.api_key.clear();
        }
        false
    } else {
        existing
            .as_ref()
            .is_some_and(|endpoint| endpoint.has_api_key)
    };
    // A different URL or model invalidates the measured level.
    let structured_output = existing
        .as_ref()
        .filter(|endpoint| endpoint.base_url == base_url && endpoint.model_id == model_id)
        .and_then(|endpoint| endpoint.structured_output);
    let record = LlmEndpointRecord {
        id: id.clone(),
        name,
        base_url,
        model_id,
        has_api_key,
        structured_output,
        rpm_limit: existing.as_ref().and_then(|endpoint| endpoint.rpm_limit),
    };
    match settings
        .llm_endpoints
        .iter_mut()
        .find(|endpoint| endpoint.id == id)
    {
        Some(slot) => *slot = record,
        None => settings.llm_endpoints.push(record),
    }
    normalize(settings);
    Ok(id)
}

pub fn delete_endpoint(
    settings: &mut ProviderModelSettings,
    id: &str,
    store: &dyn SecretStore,
) -> Result<(), AppError> {
    if settings.endpoint(id).is_none() {
        return Err(LlmError::EndpointNotFound(id.to_string()).into());
    }
    store.delete(id).map_err(|_| keychain_error())?;
    settings.llm_endpoints.retain(|endpoint| endpoint.id != id);
    if id == LEGACY_LOCAL_ENDPOINT_ID {
        settings.local_generation = LocalGenerationSettings::default();
    }
    normalize(settings);
    Ok(())
}

/// The endpoint configuration a connection test ran against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestedEndpoint {
    pub base_url: String,
    pub model_id: String,
}

/// Stores a measured level for an endpoint or CLI. An endpoint level is
/// stored only while the endpoint still has the URL and model that were
/// tested, so editing it during a test never attaches a stale level to the
/// new configuration.
pub fn record_level(
    settings: &mut ProviderModelSettings,
    provider: &ProviderRef,
    level: StructuredOutputLevel,
    tested: Option<&TestedEndpoint>,
) {
    match provider {
        ProviderRef::Endpoint { id } => {
            let Some(tested) = tested else {
                return;
            };
            if let Some(endpoint) = settings.llm_endpoints.iter_mut().find(|e| &e.id == id) {
                if endpoint.base_url == tested.base_url && endpoint.model_id == tested.model_id {
                    endpoint.structured_output = Some(level);
                }
            }
        }
        ProviderRef::Cli { id } => {
            settings.llm_cli_levels.insert(*id, level);
        }
        ProviderRef::OpenRouter | ProviderRef::None => {}
    }
}

/// The key to send when listing an endpoint's models: the key typed in the
/// form, else the key saved for the endpoint being edited.
pub fn listing_key(
    registry: &LlmRegistry,
    endpoint_id: Option<&str>,
    typed: &str,
    store: &dyn SecretStore,
) -> Option<String> {
    let typed = typed.trim();
    if !typed.is_empty() {
        return Some(typed.to_string());
    }
    registry.connection(endpoint_id?, store).ok()?.api_key
}

fn keychain_error() -> AppError {
    AppError::new(
        "llm_keychain_unavailable",
        "Clovy could not reach the system keychain. Try again.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::secrets::MemorySecretStore;
    use crate::providers::PROVIDER_OPENROUTER;

    fn legacy_settings_json(provider: &str) -> String {
        format!(
            r#"{{
              "transcriptionProvider": "venice",
              "generationProvider": "{provider}",
              "transcriptionModel": "nvidia/parakeet-tdt-0.6b-v3",
              "generationModel": "llama3.1:8b",
              "remoteGenerationModel": "zai-org-glm-5-2",
              "localGeneration": {{
                "baseUrl": "http://localhost:11434/v1/",
                "modelId": "llama3.1:8b",
                "apiKey": "sk-legacy"
              }}
            }}"#
        )
    }

    fn load(json: &str, store: &dyn SecretStore) -> ProviderModelSettings {
        crate::providers::settings_from_json_for_tests(json, store).unwrap()
    }

    #[test]
    fn llm_legacy_local_endpoint_migrates_with_url_model_and_key() {
        let store = MemorySecretStore::default();
        let settings = load(&legacy_settings_json("local"), &store);

        let endpoint = settings.endpoint(LEGACY_LOCAL_ENDPOINT_ID).unwrap();
        assert_eq!(endpoint.base_url, "http://localhost:11434/v1");
        assert_eq!(endpoint.model_id, "llama3.1:8b");
        assert!(endpoint.has_api_key);
        assert_eq!(store.get("local").unwrap().as_deref(), Some("sk-legacy"));
        // Notes retain the endpoint; historical chat moves to a CLI.
        let local = ProviderRef::Endpoint {
            id: "local".to_string(),
        };
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
        assert_eq!(settings.llm_usage.notes, local);
        assert_eq!(settings.generation_provider, PROVIDER_OPENROUTER);
        assert_eq!(settings.generation_model, settings.remote_generation_model);
        assert_eq!(settings.llm_usage.activity, ProviderRef::None);
        // The key no longer sits in the settings file.
        assert!(settings.llm_registry_migrated);
        let written = serde_json::to_string(&settings).unwrap();
        assert!(!written.contains("sk-legacy"), "{written}");
        assert!(!written.contains("localGeneration"), "{written}");
        let registry = LlmRegistry::from(&settings);
        let connection = registry.connection("local", &store).unwrap();
        assert_eq!(connection.api_key.as_deref(), Some("sk-legacy"));
    }

    #[test]
    fn llm_legacy_disabled_local_endpoint_migrates_without_taking_over_chat() {
        let store = MemorySecretStore::default();
        let settings = load(&legacy_settings_json("venice"), &store);
        assert!(settings.endpoint("local").is_some());
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
        assert_eq!(settings.llm_usage.notes, ProviderRef::OpenRouter);
        assert_eq!(settings.generation_provider, PROVIDER_OPENROUTER);
    }

    #[test]
    fn llm_migration_keeps_the_legacy_key_when_the_keychain_refuses_it() {
        let store = MemorySecretStore::failing();
        let settings = load(&legacy_settings_json("local"), &store);
        assert!(!settings.llm_registry_migrated);
        assert_eq!(settings.local_generation.api_key, "sk-legacy");
        let registry = LlmRegistry::from(&settings);
        assert_eq!(
            registry
                .connection("local", &store)
                .unwrap()
                .api_key
                .as_deref(),
            Some("sk-legacy")
        );
        // A second load does not duplicate the endpoint.
        let again = migrate_legacy_local_generation(settings, &store);
        assert_eq!(again.llm_endpoints.len(), 1);
    }

    #[test]
    fn llm_new_local_key_is_not_overwritten_by_a_pending_legacy_migration() {
        // Launch 1: the Keychain refuses the legacy key, which stays on disk.
        let refusing = MemorySecretStore::failing();
        let mut settings = load(&legacy_settings_json("local"), &refusing);
        assert!(!settings.llm_registry_migrated);
        // Later the Keychain works and the user saves a new key for `local`.
        let store = MemorySecretStore::default();
        save_endpoint(
            &mut settings,
            SaveEndpointRequest {
                id: Some(LEGACY_LOCAL_ENDPOINT_ID.to_string()),
                name: "Local model".to_string(),
                base_url: "http://localhost:11434/v1".to_string(),
                model_id: "llama3.1:8b".to_string(),
                api_key: Some("sk-new".to_string()),
                clear_api_key: false,
            },
            &store,
        )
        .unwrap();
        assert!(settings.llm_registry_migrated);
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains("sk-legacy"));
        // Launch 2: migration runs again on the saved file.
        let saved = serde_json::to_string(&settings).unwrap();
        let reloaded = load(&saved, &store);
        assert_eq!(store.get("local").unwrap().as_deref(), Some("sk-new"));
        assert_eq!(
            LlmRegistry::from(&reloaded)
                .connection("local", &store)
                .unwrap()
                .api_key
                .as_deref(),
            Some("sk-new")
        );
    }

    #[test]
    fn llm_test_level_is_not_recorded_on_an_endpoint_edited_during_the_test() {
        let mut settings = settings_with_endpoint(None);
        let endpoint = ProviderRef::Endpoint {
            id: "ep".to_string(),
        };
        let tested = TestedEndpoint {
            base_url: "http://localhost:1/v1".to_string(),
            model_id: "m".to_string(),
        };
        // The user switched the model while the test ran.
        settings.llm_endpoints[0].model_id = "other-model".to_string();
        record_level(
            &mut settings,
            &endpoint,
            StructuredOutputLevel::Strict,
            Some(&tested),
        );
        assert_eq!(settings.endpoint("ep").unwrap().structured_output, None);
        // Unchanged configuration: the level is stored.
        settings.llm_endpoints[0].model_id = "m".to_string();
        record_level(
            &mut settings,
            &endpoint,
            StructuredOutputLevel::Strict,
            Some(&tested),
        );
        assert_eq!(
            settings.endpoint("ep").unwrap().structured_output,
            Some(StructuredOutputLevel::Strict)
        );
    }

    #[test]
    fn llm_model_listing_uses_the_saved_key_unless_one_is_typed() {
        let store = MemorySecretStore::default();
        let mut settings = settings_with_endpoint(None);
        settings.llm_endpoints[0].has_api_key = true;
        store.set("ep", "sk-saved").unwrap();
        let registry = LlmRegistry::from(&settings);
        assert_eq!(
            listing_key(&registry, Some("ep"), "  ", &store).as_deref(),
            Some("sk-saved")
        );
        assert_eq!(
            listing_key(&registry, Some("ep"), "sk-typed", &store).as_deref(),
            Some("sk-typed")
        );
        assert_eq!(listing_key(&registry, None, "", &store), None);
    }

    #[test]
    fn llm_endpoint_dto_never_serializes_the_key() {
        let store = MemorySecretStore::default();
        let mut settings = crate::providers::default_settings_for_tests();
        save_endpoint(
            &mut settings,
            SaveEndpointRequest {
                id: None,
                name: "Gateway".to_string(),
                base_url: "https://gateway.example/v1".to_string(),
                model_id: "gpt-x".to_string(),
                api_key: Some("sk-secret-value".to_string()),
                clear_api_key: false,
            },
            &store,
        )
        .unwrap();
        let dto = serde_json::to_value(LlmProvidersDto::from(&settings)).unwrap();
        let endpoint = &dto["endpoints"][0];
        assert_eq!(endpoint["hasApiKey"], serde_json::json!(true));
        assert!(endpoint.get("apiKey").is_none());
        assert!(!dto.to_string().contains("sk-secret-value"));
        assert!(!serde_json::to_string(&settings)
            .unwrap()
            .contains("sk-secret-value"));
    }

    #[test]
    fn llm_update_without_key_keeps_it_and_clear_removes_it() {
        let store = MemorySecretStore::default();
        let mut settings = crate::providers::default_settings_for_tests();
        let request = |id: Option<String>, key: Option<&str>, clear| SaveEndpointRequest {
            id,
            name: "Gateway".to_string(),
            base_url: "https://gateway.example/v1".to_string(),
            model_id: "gpt-x".to_string(),
            api_key: key.map(str::to_string),
            clear_api_key: clear,
        };
        let id = save_endpoint(&mut settings, request(None, Some("sk-1"), false), &store).unwrap();
        save_endpoint(
            &mut settings,
            request(Some(id.clone()), None, false),
            &store,
        )
        .unwrap();
        assert!(settings.endpoint(&id).unwrap().has_api_key);
        assert_eq!(store.get(&id).unwrap().as_deref(), Some("sk-1"));
        save_endpoint(&mut settings, request(Some(id.clone()), None, true), &store).unwrap();
        assert!(!settings.endpoint(&id).unwrap().has_api_key);
        assert_eq!(store.get(&id).unwrap(), None);
    }

    fn settings_with_endpoint(level: Option<StructuredOutputLevel>) -> ProviderModelSettings {
        let mut settings = crate::providers::default_settings_for_tests();
        settings.llm_endpoints.push(LlmEndpointRecord {
            id: "ep".to_string(),
            name: "Endpoint".to_string(),
            base_url: "http://localhost:1/v1".to_string(),
            model_id: "m".to_string(),
            has_api_key: false,
            structured_output: level,
            rpm_limit: None,
        });
        settings
    }

    #[test]
    fn llm_activity_refuses_providers_below_json_schema() {
        let endpoint = ProviderRef::Endpoint {
            id: "ep".to_string(),
        };
        for level in [
            None,
            Some(StructuredOutputLevel::Prompt),
            Some(StructuredOutputLevel::JsonObject),
        ] {
            let mut settings = settings_with_endpoint(level);
            let error = set_usage(&mut settings, LlmUsage::Activity, endpoint.clone(), |_| {
                true
            })
            .unwrap_err();
            assert_eq!(error.code, "llm_structured_output_insufficient");
            assert_eq!(settings.llm_usage.activity, ProviderRef::None);
        }
        let mut settings = settings_with_endpoint(Some(StructuredOutputLevel::JsonSchema));
        set_usage(&mut settings, LlmUsage::Activity, endpoint.clone(), |_| {
            true
        })
        .unwrap();
        assert_eq!(settings.llm_usage.activity, endpoint);
    }

    #[test]
    fn llm_activity_never_uses_clovy_and_defaults_to_none() {
        let mut settings = crate::providers::default_settings_for_tests();
        assert_eq!(settings.llm_usage.activity, ProviderRef::None);
        let error = set_usage(
            &mut settings,
            LlmUsage::Activity,
            ProviderRef::OpenRouter,
            |_| true,
        )
        .unwrap_err();
        assert_eq!(error.code, "llm_provider_not_allowed");
    }

    #[test]
    fn llm_missing_cli_cannot_be_selected() {
        let mut settings = crate::providers::default_settings_for_tests();
        let error = set_usage(
            &mut settings,
            LlmUsage::Notes,
            ProviderRef::Cli {
                id: CliKind::CursorAgent,
            },
            |_| false,
        )
        .unwrap_err();
        assert_eq!(error.code, "llm_cli_not_installed");
        assert_eq!(settings.llm_usage.notes, ProviderRef::OpenRouter);
    }

    #[test]
    fn llm_cli_that_keeps_tools_cannot_take_content_uses() {
        let mut settings = crate::providers::default_settings_for_tests();
        for id in [CliKind::CursorAgent, CliKind::Copilot] {
            for usage in [LlmUsage::Notes, LlmUsage::Activity] {
                let error =
                    set_usage(&mut settings, usage, ProviderRef::Cli { id }, |_| true).unwrap_err();
                assert_eq!(error.code, "llm_cli_tools_not_disabled", "{id:?} {usage:?}");
            }
        }
        assert_eq!(settings.llm_usage.notes, ProviderRef::OpenRouter);
        assert_eq!(settings.llm_usage.activity, ProviderRef::None);
    }

    /// The owner's decision: codex and agy run notes and activity in their
    /// least-privileged one-shot mode.
    #[test]
    fn llm_codex_and_agy_take_notes_and_activity() {
        for id in [CliKind::Codex, CliKind::Agy] {
            let mut settings = crate::providers::default_settings_for_tests();
            // Activity also needs the level the connection test measured.
            settings.llm_cli_levels.insert(id, id.structured_output());
            for usage in [LlmUsage::Notes, LlmUsage::Activity] {
                set_usage(&mut settings, usage, ProviderRef::Cli { id }, |_| true)
                    .unwrap_or_else(|error| panic!("{id:?} {usage:?}: {error:?}"));
            }
            assert_eq!(settings.llm_usage.notes, ProviderRef::Cli { id });
            assert_eq!(settings.llm_usage.activity, ProviderRef::Cli { id });
        }
    }

    #[test]
    fn llm_chat_accepts_only_clis_and_leaves_the_text_model_unchanged() {
        let mut settings = settings_with_endpoint(None);
        let model = settings.generation_model.clone();
        for provider in [
            ProviderRef::OpenRouter,
            ProviderRef::Endpoint { id: "ep".into() },
        ] {
            assert_eq!(
                set_usage(&mut settings, LlmUsage::Chat, provider, |_| true)
                    .unwrap_err()
                    .code,
                "llm_provider_not_allowed"
            );
        }
        set_usage(
            &mut settings,
            LlmUsage::Chat,
            ProviderRef::Cli {
                id: CliKind::Claude,
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
        assert_eq!(settings.generation_model, model);
    }

    #[test]
    fn llm_deleting_an_endpoint_resets_its_usages() {
        let store = MemorySecretStore::default();
        let mut settings = settings_with_endpoint(Some(StructuredOutputLevel::Strict));
        let endpoint = ProviderRef::Endpoint {
            id: "ep".to_string(),
        };
        set_usage(&mut settings, LlmUsage::Notes, endpoint.clone(), |_| true).unwrap();
        set_usage(&mut settings, LlmUsage::Activity, endpoint, |_| true).unwrap();
        delete_endpoint(&mut settings, "ep", &store).unwrap();
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
        assert_eq!(settings.llm_usage.notes, ProviderRef::OpenRouter);
        assert_eq!(settings.llm_usage.activity, ProviderRef::None);
        assert_eq!(settings.generation_provider, PROVIDER_OPENROUTER);
    }

    #[test]
    fn llm_agent_model_resolves_to_the_endpoint_serving_it() {
        let mut settings = settings_with_endpoint(None);
        settings.llm_endpoints.push(LlmEndpointRecord {
            id: "other".to_string(),
            name: "Other".to_string(),
            base_url: "http://lan:8000/v1".to_string(),
            model_id: "qwen".to_string(),
            has_api_key: false,
            structured_output: None,
            rpm_limit: None,
        });
        let registry = LlmRegistry::from(&settings);
        assert_eq!(registry.endpoint_for_model("qwen").unwrap().id, "other");
        assert_eq!(registry.endpoint_for_model("m").unwrap().id, "ep");
        assert!(registry.endpoint_for_model("unknown").is_none());
    }

    fn chat_on_clovy() -> ProviderModelSettings {
        let mut settings = crate::providers::default_settings_for_tests();
        settings.llm_usage.chat = ProviderRef::OpenRouter;
        settings.llm_cli_default_applied = false;
        settings
    }

    #[test]
    fn retired_chat_choices_migrate_despite_the_old_default_marker() {
        for chat in [
            ProviderRef::OpenRouter,
            ProviderRef::Endpoint { id: "ep".into() },
        ] {
            let mut settings = settings_with_endpoint(None);
            settings.llm_usage.chat = chat;
            settings.llm_usage.notes = ProviderRef::Endpoint { id: "ep".into() };
            settings.llm_cli_default_applied = true;
            assert_eq!(
                apply_cli_default(&mut settings, |_| true),
                Some(CliKind::Claude)
            );
            assert_eq!(
                settings.llm_usage.chat,
                ProviderRef::Cli {
                    id: CliKind::Claude
                }
            );
            assert_eq!(
                settings.llm_usage.notes,
                ProviderRef::Endpoint { id: "ep".into() }
            );
            let saved = serde_json::to_string(&settings).unwrap();
            let mut reloaded: ProviderModelSettings = serde_json::from_str(&saved).unwrap();
            assert_eq!(apply_cli_default(&mut reloaded, |_| false), None);
            assert_eq!(reloaded, settings);
        }
    }

    #[test]
    fn selection_single_installed_cli_becomes_the_persisted_chat_engine() {
        for only in [CliKind::Claude, CliKind::Codex, CliKind::Agy] {
            let mut settings = crate::providers::default_settings_for_tests();
            let notes = settings.llm_usage.notes.clone();
            let text_model = settings.generation_model.clone();
            apply_cli_default(&mut settings, |kind| kind == only);
            assert_eq!(
                crate::chat_engine::default_engine_of(&settings).model_id,
                crate::chat_engine::engine_model_id(only),
                "only {only:?} is installed"
            );
            let saved = serde_json::to_string(&settings).unwrap();
            let mut reloaded: ProviderModelSettings = serde_json::from_str(&saved).unwrap();
            assert_eq!(apply_cli_default(&mut reloaded, |_| false), None);
            assert_eq!(reloaded.llm_usage.chat, ProviderRef::Cli { id: only });
            assert_eq!(reloaded.llm_usage.notes, notes);
            assert_eq!(reloaded.generation_model, text_model);
        }
    }

    #[test]
    fn selection_explicit_switch_and_unavailability_never_select_another_cli() {
        let mut settings = crate::providers::default_settings_for_tests();
        apply_cli_default(&mut settings, |_| true);
        for chosen in [CliKind::Agy, CliKind::Codex, CliKind::Claude] {
            set_usage(
                &mut settings,
                LlmUsage::Chat,
                ProviderRef::Cli { id: chosen },
                |_| true,
            )
            .unwrap();
            let before = settings.clone();
            // A later launch with no CLI available preserves the explicit choice.
            assert_eq!(apply_cli_default(&mut settings, |_| false), None);
            assert_eq!(settings, before);
            let unavailable = if chosen == CliKind::Claude {
                CliKind::Agy
            } else {
                CliKind::Claude
            };
            let error = set_usage(
                &mut settings,
                LlmUsage::Chat,
                ProviderRef::Cli { id: unavailable },
                |kind| kind == chosen,
            )
            .unwrap_err();
            assert_eq!(error.code, "llm_cli_not_installed");
            assert_eq!(settings, before);
            assert_eq!(crate::chat_engine::default_engine_of(&settings).cli, chosen);
        }
    }

    #[test]
    fn selection_note_provider_is_independent_of_chat_for_each_installed_cli() {
        for only in [CliKind::Claude, CliKind::Codex, CliKind::Agy] {
            let mut settings = crate::providers::default_settings_for_tests();
            let chat = settings.llm_usage.chat.clone();
            let text_model = settings.generation_model.clone();
            set_usage(
                &mut settings,
                LlmUsage::Notes,
                ProviderRef::Cli { id: only },
                |kind| kind == only,
            )
            .unwrap();
            assert_eq!(
                LlmRegistry::from(&settings).usage.get(LlmUsage::Notes),
                &ProviderRef::Cli { id: only }
            );
            assert_eq!(settings.llm_usage.chat, chat);
            assert_eq!(settings.generation_model, text_model);
            let before = settings.clone();
            let missing = if only == CliKind::Claude {
                CliKind::Codex
            } else {
                CliKind::Claude
            };
            assert_eq!(
                set_usage(
                    &mut settings,
                    LlmUsage::Notes,
                    ProviderRef::Cli { id: missing },
                    |kind| kind == only
                )
                .unwrap_err()
                .code,
                "llm_cli_not_installed"
            );
            assert_eq!(settings, before);
        }
    }

    #[test]
    fn llm_chat_moves_once_to_claude_code_when_it_is_installed() {
        // An existing install whose chat was on the Clovy agent (the old
        // default), and a new install, which starts there too.
        let mut settings = chat_on_clovy();
        let moved = apply_cli_default(&mut settings, |_| true);
        assert_eq!(moved, Some(CliKind::Claude));
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
        assert!(settings.llm_cli_default_applied);
        assert!(!cli_default_pending(&settings));
        // A recorded CLI choice survives future launches.
        settings.llm_usage.chat = ProviderRef::Cli { id: CliKind::Codex };
        let before = settings.clone();
        assert_eq!(apply_cli_default(&mut settings, |_| true), None);
        assert_eq!(settings, before);
    }

    #[test]
    fn llm_chat_moves_to_codex_when_claude_code_is_missing() {
        let mut settings = chat_on_clovy();
        let moved = apply_cli_default(&mut settings, |kind| kind == CliKind::Codex);
        assert_eq!(moved, Some(CliKind::Codex));
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli { id: CliKind::Codex }
        );
        // Other CLIs never become the default.
        let mut settings = chat_on_clovy();
        assert_eq!(
            apply_cli_default(&mut settings, |kind| kind == CliKind::Pi),
            Some(CliKind::Claude)
        );
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
    }

    #[test]
    fn llm_chat_without_an_installed_cli_still_selects_claude() {
        let mut settings = chat_on_clovy();
        assert_eq!(
            apply_cli_default(&mut settings, |_| false),
            Some(CliKind::Claude)
        );
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
        assert!(settings.llm_cli_default_applied);
        assert!(!cli_default_pending(&settings));
    }

    #[test]
    fn llm_a_chosen_cli_is_never_moved_to_the_default() {
        let mut settings = chat_on_clovy();
        settings.llm_usage.chat = ProviderRef::Cli { id: CliKind::Pi };
        assert_eq!(apply_cli_default(&mut settings, |_| true), None);
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli { id: CliKind::Pi }
        );
        assert!(settings.llm_cli_default_applied);
        assert_eq!(
            set_usage(
                &mut settings,
                LlmUsage::Chat,
                ProviderRef::OpenRouter,
                |_| false
            )
            .unwrap_err()
            .code,
            "llm_provider_not_allowed"
        );
    }

    #[test]
    fn llm_dictation_cleanup_refuses_clis_and_endpoints() {
        // Dictation cleanup runs only on OpenRouter directly (ditado-rapido
        // D1): never a CLI process nor a local endpoint, even with tools off.
        let mut settings = settings_with_endpoint(Some(StructuredOutputLevel::Strict));
        for provider in [
            ProviderRef::Cli {
                id: CliKind::Claude,
            },
            ProviderRef::Cli { id: CliKind::Codex },
            ProviderRef::Endpoint {
                id: "ep".to_string(),
            },
            ProviderRef::None,
        ] {
            let error = set_usage(
                &mut settings,
                LlmUsage::DictationCleanup,
                provider.clone(),
                |_| true,
            )
            .unwrap_err();
            assert_eq!(error.code, "llm_provider_not_allowed", "{provider:?}");
        }
        assert_eq!(
            settings.llm_usage.dictation_cleanup,
            ProviderRef::OpenRouter
        );
        set_usage(
            &mut settings,
            LlmUsage::DictationCleanup,
            ProviderRef::OpenRouter,
            |_| true,
        )
        .unwrap();
    }

    #[test]
    fn llm_normalize_fixes_dictation_cleanup_on_openrouter() {
        for stored in [
            ProviderRef::Cli {
                id: CliKind::Claude,
            },
            ProviderRef::Endpoint {
                id: "ep".to_string(),
            },
        ] {
            let mut settings = settings_with_endpoint(None);
            settings.llm_usage.dictation_cleanup = stored.clone();
            normalize(&mut settings);
            assert_eq!(
                settings.llm_usage.dictation_cleanup,
                ProviderRef::OpenRouter,
                "{stored:?}"
            );
        }
    }

    #[test]
    fn llm_provider_settings_with_cli_dictation_cleanup_load_on_openrouter() {
        // The owner's profile before ditado-rapido: cleanup on Claude Code.
        let store = MemorySecretStore::default();
        let raw = r#"{
          "transcriptionProvider": "groq",
          "generationProvider": "openrouter",
          "llmRegistryMigrated": true,
          "llmCliDefaultApplied": true,
          "llmUsage": {
            "chat": { "kind": "cli", "id": "claude" },
            "notes": { "kind": "cli", "id": "claude" },
            "dictationCleanup": { "kind": "cli", "id": "claude" },
            "activity": { "kind": "none" }
          }
        }"#;
        let settings = load(raw, &store);
        assert_eq!(
            settings.llm_usage.dictation_cleanup,
            ProviderRef::OpenRouter
        );
        // Notes keep their CLI: only dictation cleanup changed.
        assert_eq!(
            settings.llm_usage.notes,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
    }

    #[test]
    fn llm_settings_saved_before_the_cli_default_are_moved_on_load() {
        // A settings file from before ADR-0062 has no flag: chat on Clovy is
        // the old default and is pending the move.
        let store = MemorySecretStore::default();
        let settings = load(&legacy_settings_json("venice"), &store);
        assert_eq!(
            settings.llm_usage.chat,
            ProviderRef::Cli {
                id: CliKind::Claude
            }
        );
        assert!(!settings.llm_cli_default_applied);
        assert!(cli_default_pending(&settings));
        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(written["llmCliDefaultApplied"], false);
    }
}
