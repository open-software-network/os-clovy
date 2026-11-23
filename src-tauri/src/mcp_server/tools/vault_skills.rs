//! Vault discovery and skill loading for bound CLI turns. Skills follow global
//! enablement; routine skill calls also recheck the saved toolset policy.
use super::{ToolCtx, ToolGroup};
use crate::agent_sessions::skill_identity::canonical_skill_id;
use crate::agent_sessions::AgentRepository;
use crate::approval_broker::{external::ExternalCall, ApprovalKind, ApprovalRequest, ApprovalTurn};
use crate::domain::types::AppError;
use crate::mcp_server::turns::TURN_REQUIRED;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};
use std::{collections::HashMap, path::PathBuf};
use tauri::AppHandle;
use tauri::Emitter;

pub struct VaultSkillTools;
impl ToolGroup for VaultSkillTools {
    fn owns(&self, name: &str) -> bool {
        matches!(
            name,
            "get_obsidian_vault"
                | "list_skills"
                | "load_skill"
                | "list_skill_settings"
                | "read_skill"
                | "update_skill"
                | "enable_skill"
                | "disable_skill"
        )
    }
    fn descriptors(&self, _ctx: &ToolCtx<'_>) -> Vec<Value> {
        [
            ("get_obsidian_vault","Discover the currently selected Obsidian vault for each distinct task. Discovery does not authorize writes; never infer an unavailable vault's path.",json!({}),vec![]),
            ("list_skills","List globally enabled Clovy skills.",json!({}),vec![]),
            ("list_skill_settings","List active and disabled skills with source and editability.",json!({}),vec![]),
            ("read_skill","Read skill instructions without executing them, including disabled skills.",json!({"name":{"type":"string"}}),vec!["name"]),
            ("update_skill","Replace instructions of an existing managed skill. Global skills are read-only.",json!({"name":{"type":"string"},"content":{"type":"string","maxLength":524288}}),vec!["name","content"]),
            ("enable_skill","Enable an existing managed skill for future discovery.",json!({"name":{"type":"string"}}),vec!["name"]),
            ("disable_skill","Disable an existing managed skill for future discovery.",json!({"name":{"type":"string"}}),vec!["name"]),
            ("load_skill","Load instructions for one globally enabled Clovy skill.",json!({"name":{"type":"string"}}),vec!["name"]),
        ].into_iter().map(|(name,description,properties,required)| json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":!is_mutation(name),"openWorldHint":false,"idempotentHint":true}})).collect()
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        arguments: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(dispatch(ctx, None, name, arguments))
    }

    fn catalog<'a>(&'a self, ctx: &'a ToolCtx<'a>) -> BoxFuture<'a, Vec<Value>> {
        Box::pin(async move {
            let mut catalog = Vec::new();
            for descriptor in self.descriptors(ctx) {
                let name = descriptor["name"].as_str().unwrap_or_default();
                if require_tool(ctx, name).await.is_ok() {
                    catalog.push(descriptor);
                }
            }
            catalog
        })
    }
}

pub(super) async fn dispatch(
    ctx: &ToolCtx<'_>,
    external: Option<&ExternalCall<'_>>,
    name: &str,
    arguments: &Value,
) -> Result<Value, AppError> {
    if external.is_none() {
        require_tool(ctx, name).await?;
    }
    let app = ctx.app.ok_or_else(|| {
        AppError::new(
            "mcp_app_unavailable",
            "Clovy's vault and skill directories are unavailable.",
        )
    })?;
    if name == "get_obsidian_vault" {
        return vault_result(crate::obsidian::discovery_for_app(app));
    }
    let roots = crate::agent_sessions::skills::skill_roots(app);
    // Dropping the managed root after a path-resolution failure must
    // never promote the remaining global root to editable storage.
    if roots.first().map_or(true, |(_, managed)| !managed) {
        return Err(AppError::new(
            "agent_skill_write_failed",
            "Managed skill storage is unavailable.",
        ));
    }
    let mut value = call_scoped(ctx, external, name, arguments, &skill_roots(app)).await?;
    if external.is_some() {
        fn remove_path(object: &mut Value) {
            if let Some(fields) = object.as_object_mut() {
                fields.remove("path");
                fields.remove("localPath");
            }
        }
        remove_path(&mut value);
        if let Some(skills) = value.get_mut("skills").and_then(Value::as_array_mut) {
            skills.iter_mut().for_each(remove_path);
        }
    }
    Ok(value)
}

pub(crate) fn vault_result(
    discovery: Result<crate::obsidian::ObsidianDiscovery, AppError>,
) -> Result<Value, AppError> {
    serde_json::to_value(discovery?)
        .map_err(|error| AppError::new("agent_tool_response_invalid", error.to_string()))
}

#[cfg(test)]
pub(super) async fn call_with_roots(
    ctx: &ToolCtx<'_>,
    name: &str,
    arguments: &Value,
    roots: &[PathBuf],
) -> Result<Value, AppError> {
    call_scoped(ctx, None, name, arguments, roots).await
}

async fn call_scoped(
    ctx: &ToolCtx<'_>,
    external: Option<&ExternalCall<'_>>,
    name: &str,
    arguments: &Value,
    roots: &[PathBuf],
) -> Result<Value, AppError> {
    if external.is_none() {
        require_tool(ctx, name).await?;
    } else if !matches!(name, "list_skills" | "load_skill") {
        return Err(AppError::new(
            TURN_REQUIRED,
            "This skill operation requires a Clovy turn.",
        ));
    }
    ctx.revalidate_local(external).await?;
    let requested = if !matches!(name, "list_skills" | "list_skill_settings") {
        let requested = arguments
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| AppError::new("agent_tool_arguments_invalid", "name is required."))?;
        Some(validate_skill_name(requested)?)
    } else {
        None
    };
    let enabled = crate::agent_sessions::skill_identity::skill_enabled_overrides(
        AgentRepository::new(ctx.data.notes.clone())
            .skills()
            .await?
            .into_iter()
            .map(|skill| (skill.id, skill.enabled)),
    );
    let typed_roots = roots
        .iter()
        .enumerate()
        .map(|(index, root)| (root.clone(), index == 0))
        .collect::<Vec<_>>();
    let catalog =
        crate::agent_sessions::skills::catalog_from_roots(typed_roots.clone(), &enabled).await?;
    match name {
        "list_skill_settings" => Ok(json!({"skills":catalog})),
        "list_skills" => list_skills_from_roots(roots, &enabled, true).await,
        "read_skill" => {
            crate::agent_sessions::skills::read_from_roots(
                &typed_roots,
                requested.unwrap_or_default(),
            )
            .await
        }
        "load_skill" => {
            let allowed = catalog
                .iter()
                .filter(|skill| skill["enabled"] == true)
                .filter_map(|skill| skill["id"].as_str())
                .map(|id| (id.to_string(), true))
                .collect();
            let id = requested.unwrap_or_default();
            if catalog.iter().any(|skill| {
                skill["id"] == id
                    && skill["configuredEnabled"] == true
                    && skill["readable"] == false
            }) {
                return crate::agent_sessions::skills::read_from_roots(&typed_roots, id).await;
            }
            load_skill_from_roots(roots, &allowed, id).await
        }
        "update_skill" | "enable_skill" | "disable_skill" => {
            let id = requested.unwrap_or_default();
            let editable = |catalog: &[Value]| {
                catalog
                    .iter()
                    .find(|skill| skill["id"] == id)
                    .is_some_and(|skill| skill["editable"] == true)
            };
            if !editable(&catalog) {
                return Err(AppError::new(
                    "agent_skill_read_only",
                    "Only existing managed skills can be changed.",
                ));
            }
            let binding = ctx.binding.ok_or_else(|| {
                AppError::new(TURN_REQUIRED, "Skill changes require a Clovy turn.")
            })?;
            ctx.broker()?
                .request(
                    &ApprovalTurn::from(binding),
                    ApprovalRequest {
                        tool: name.into(),
                        call_id: Some(uuid::Uuid::new_v4().to_string()),
                        kind: ApprovalKind::Approval {
                            title: "Change a skill".into(),
                            description: "Change the selected managed skill in Clovy.".into(),
                            arguments: arguments.clone(),
                            preview: None,
                        },
                    },
                )
                .await
                .map_err(|denial| AppError::new(denial.code(), denial.message()))?;
            ctx.revalidate().await?;
            require_tool(ctx, name).await?;
            // Resolve again after approval; neither an old catalog nor an id is
            // a grant to write a replaced global file or an escaped symlink.
            let catalog =
                crate::agent_sessions::skills::catalog_from_roots(typed_roots.clone(), &enabled)
                    .await?;
            if !editable(&catalog) {
                return Err(AppError::new(
                    "agent_skill_read_only",
                    "Only existing managed skills can be changed.",
                ));
            }
            if name == "update_skill" {
                let content = arguments
                    .get("content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| AppError::new("invalid_arguments", "content is required."))?;
                crate::agent_sessions::skills::update_for_turn(
                    &ctx.data.notes,
                    &typed_roots,
                    id,
                    content,
                    (
                        &ctx.data.profile,
                        &binding.session_id,
                        &binding.run_id,
                        &binding.authority,
                    ),
                )
                .await?;
            } else {
                crate::agent_sessions::skills::set_enabled_from_roots(
                    &ctx.data.notes,
                    &typed_roots,
                    id,
                    name == "enable_skill",
                    Some((
                        &ctx.data.profile,
                        &binding.session_id,
                        &binding.run_id,
                        &binding.authority,
                    )),
                )
                .await?;
            }
            if let Some(app) = ctx.app {
                let _ = app.emit(
                    "clovy-data-changed",
                    json!({"profile":ctx.data.profile,"kind":"skills","id":id}),
                );
            }
            let overrides = crate::agent_sessions::skill_identity::skill_enabled_overrides(
                AgentRepository::new(ctx.data.notes.clone())
                    .skills()
                    .await?
                    .into_iter()
                    .map(|skill| (skill.id, skill.enabled)),
            );
            crate::agent_sessions::skills::catalog_from_roots(typed_roots, &overrides)
                .await?
                .into_iter()
                .find(|skill| skill["id"] == id)
                .ok_or_else(|| {
                    AppError::new(
                        "agent_skill_not_found",
                        "Skill was not found after the change.",
                    )
                })
        }
        _ => Err(AppError::new(
            "mcp_tool_unknown",
            "Unknown vault or skill tool.",
        )),
    }
}

fn is_mutation(name: &str) -> bool {
    matches!(name, "update_skill" | "enable_skill" | "disable_skill")
}

async fn require_tool(ctx: &ToolCtx<'_>, name: &str) -> Result<(), AppError> {
    let binding = ctx.binding.ok_or_else(|| {
        AppError::new(TURN_REQUIRED, "Vault and skill reads require a Clovy turn.")
    })?;
    if !binding.attended
        && matches!(
            name,
            "list_skill_settings"
                | "read_skill"
                | "update_skill"
                | "enable_skill"
                | "disable_skill"
        )
    {
        return Err(AppError::new(
            "routine_tool_not_enabled",
            "Skill administration requires an attended Clovy turn.",
        ));
    }
    // Vault discovery keeps its existing bound-turn contract. The routine
    // skill toolset controls instructions, not discovery of the chosen vault.
    if name != "get_obsidian_vault"
        && !super::routine_tool_allowed(&ctx.data.notes, binding, name).await?
    {
        return Err(AppError::new(
            "routine_tool_not_enabled",
            "This tool is not enabled for the routine.",
        ));
    }
    Ok(())
}

pub(crate) fn skill_roots(app: &AppHandle) -> Vec<PathBuf> {
    crate::agent_sessions::skills::skill_roots(app)
        .into_iter()
        .map(|(root, _)| root)
        .collect()
}

pub(crate) async fn list_skills_from_roots(
    roots: &[PathBuf],
    enabled: &HashMap<String, bool>,
    default_enabled: bool,
) -> Result<Value, AppError> {
    let typed_roots = roots
        .iter()
        .enumerate()
        .map(|(index, root)| (root.clone(), index == 0))
        .collect::<Vec<_>>();
    let catalog = crate::agent_sessions::skills::catalog_from_roots(typed_roots, enabled).await?;
    let skills = catalog
        .into_iter()
        .filter(|skill| {
            skill["readable"] == true
                && skill["id"]
                    .as_str()
                    .is_some_and(|id| enabled.get(id).copied().unwrap_or(default_enabled))
        })
        .collect::<Vec<_>>();
    Ok(json!({"skills":skills}))
}

pub(crate) fn validate_skill_name(requested: &str) -> Result<&str, AppError> {
    if !crate::agent_sessions::skill_identity::valid_skill_id(requested) {
        return Err(AppError::new(
            "agent_skill_invalid",
            "Skill name is invalid.",
        ));
    }
    Ok(canonical_skill_id(requested))
}

pub(crate) async fn load_skill_from_roots(
    roots: &[PathBuf],
    enabled: &HashMap<String, bool>,
    requested: &str,
) -> Result<Value, AppError> {
    let name = validate_skill_name(requested)?;
    if !enabled.get(name).copied().unwrap_or(false) {
        return Err(AppError::new(
            "agent_skill_disabled",
            "This skill is not enabled for the current run.",
        ));
    }
    let typed_roots = roots
        .iter()
        .enumerate()
        .map(|(index, root)| (root.clone(), index == 0))
        .collect::<Vec<_>>();
    crate::agent_sessions::skills::read_from_roots(&typed_roots, name).await
}
