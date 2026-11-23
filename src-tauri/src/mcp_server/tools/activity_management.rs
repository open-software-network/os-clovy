//! Installation-wide activity configuration and on-demand day summaries.
//! Names are fixed by melhorias-agente-audio-automacao design D3.
use super::{ToolCtx, ToolGroup};
#[cfg(test)]
use crate::activity::settings::ActivitySettings;
use crate::activity::ActivitySettingsPatch;
use crate::approval_broker::{external::ExternalCall, ApprovalKind, ApprovalRequest, ApprovalTurn};
use crate::domain::types::AppError;
use futures_util::future::BoxFuture;
use serde_json::{json, Value};

pub struct ActivityManagementTools;
const TOOLS: [&str; 5] = [
    "get_activity_settings",
    "update_activity_settings",
    "pause_activity",
    "resume_activity",
    "generate_day_summary",
];

impl ToolGroup for ActivityManagementTools {
    fn owns(&self, name: &str) -> bool {
        TOOLS.contains(&name)
    }
    fn descriptors(&self, _: &ToolCtx<'_>) -> Vec<Value> {
        TOOLS.iter().map(|name| {
            let (description, properties, required) = match *name {
                "get_activity_settings" => ("Read installation-wide activity settings and capture state, including when capture is off.", json!({}), json!([])),
                "update_activity_settings" => ("Update activity settings for this installation. Protected MCP and defaults fields cannot be changed. Capture need not be enabled.", json!({"settings":settings_schema()}), json!(["settings"])),
                "pause_activity" => ("Pause activity capture for this installation until resumed.", json!({}), json!([])),
                "resume_activity" => ("Clear the manual activity pause. Other capture settings and OS permissions still apply.", json!({}), json!([])),
                _ => ("Generate the day's summary using the configured activity provider. May incur provider charges; reports missing provider, closed database, empty day or pending hours distinctly.", json!({"day":{"type":"string","pattern":"^[0-9]{4}-[0-9]{2}-[0-9]{2}$"}}), json!(["day"])),
            };
            json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":*name == "get_activity_settings","openWorldHint":*name == "generate_day_summary","destructiveHint":false,"idempotentHint":matches!(*name,"get_activity_settings"|"pause_activity"|"resume_activity")}})
        }).collect()
    }
    fn call<'a>(
        &'a self,
        ctx: &'a ToolCtx<'a>,
        name: &'a str,
        args: &'a Value,
    ) -> BoxFuture<'a, Result<Value, AppError>> {
        Box::pin(async move {
            ctx.revalidate().await?;
            let app = ctx.app.ok_or_else(|| {
                AppError::new(
                    "activity_unavailable",
                    "The activity service is unavailable.",
                )
            })?;
            if name == "get_activity_settings" {
                return value(crate::activity::tool_status(app));
            }
            // Validate before the approval; read current settings again afterwards so
            // unrelated changes made while the card is pending survive the patch.
            if name == "update_activity_settings" {
                parse_patch(args)?;
            }
            let day = if name == "generate_day_summary" {
                let day = args
                    .get("day")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("day is required."))?;
                chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
                    .map_err(|_| invalid("day must be a valid YYYY-MM-DD date."))?;
                Some(day.to_string())
            } else {
                None
            };
            let binding = ctx
                .binding
                .ok_or_else(|| AppError::new("turn_required", "A bound turn is required."))?;
            ctx.broker()?.request(&ApprovalTurn::from(binding), ApprovalRequest {
                tool: name.into(), call_id: Some(uuid::Uuid::new_v4().to_string()),
                kind: ApprovalKind::Approval { title: "Manage activity".into(), description: if name == "generate_day_summary" { "Generate a day summary using your configured provider, which may incur a charge.".into() } else { "Change installation-wide activity capture settings.".into() }, arguments: args.clone(), preview: None },
            }).await.map_err(|e| AppError::new(e.code(), e.message()))?;
            ctx.revalidate().await?;
            match name {
                "update_activity_settings" => {
                    let result = value(
                        crate::activity::tool_update_settings(
                            app,
                            parse_patch(args)?,
                            binding,
                            &ctx.data.notes,
                            &ctx.data.profile,
                        )
                        .await?,
                    )?;
                    if let Err(mut error) = ctx.revalidate().await {
                        error.details = Some(
                            json!({"operation":"update_activity_settings","effect":"completed","result":result,"retrySafe":false}),
                        );
                        return Err(error);
                    }
                    Ok(result)
                }
                "pause_activity" | "resume_activity" => value(crate::activity::tool_set_paused(
                    app,
                    name == "pause_activity",
                )?),
                "generate_day_summary" => {
                    let requested_day = day.ok_or_else(|| invalid("day is required."))?;
                    let dispatched = std::sync::atomic::AtomicBool::new(false);
                    let operation = crate::day_intelligence::tool_generate(
                        app,
                        requested_day.clone(),
                        binding,
                        &ctx.data.profile,
                        &dispatched,
                    );
                    tokio::pin!(operation);
                    let result = loop {
                        tokio::select! {
                            biased;
                            result=&mut operation=>break result?,
                            _=tokio::time::sleep(std::time::Duration::from_millis(100))=>{
                                if let Err(mut error)=ctx.revalidate().await {
                                    if dispatched.load(std::sync::atomic::Ordering::SeqCst){error.details=Some(json!({"day":requested_day,"operation":"generate_day_summary","delivery":"unknown","effect":"unknown","retrySafe":false}));}
                                    return Err(error);
                                }
                            },
                        }
                    };
                    let result = value(result)?;
                    if let Err(mut error) = ctx.revalidate().await {
                        error.details = Some(
                            json!({"day":requested_day,"operation":"generate_day_summary","delivery":"completed","effect":"completed","result":result,"retrySafe":false}),
                        );
                        return Err(error);
                    }
                    Ok(result)
                }
                _ => Err(AppError::new(
                    "mcp_tool_unknown",
                    "Unknown activity management tool.",
                )),
            }
        })
    }
}
pub(super) async fn call_external(
    ctx: &ToolCtx<'_>,
    scope: &ExternalCall<'_>,
    name: &str,
    args: &Value,
) -> Result<Value, AppError> {
    let app = ctx.app.ok_or_else(|| {
        AppError::new(
            "activity_unavailable",
            "The activity service is unavailable.",
        )
    })?;
    if name == "get_activity_settings" {
        return value(crate::activity::tool_status(app));
    }
    let patch = (name == "update_activity_settings")
        .then(|| parse_patch(args))
        .transpose()?;
    let title = match name {
        "pause_activity" => "Pause Activity capture",
        "resume_activity" => "Resume Activity capture",
        "update_activity_settings" => "Change Activity settings",
        _ => return Err(AppError::new("mcp_tool_unknown", "Unknown activity tool.")),
    };
    let approval = scope
        .approve(
            name,
            title,
            "Apply this change to Activity capture on this installation.",
            args,
            None,
        )
        .await?;
    if let Some(patch) = patch {
        // The shared port invokes the guard after taking the settings lock.
        value(crate::activity::tool_update_settings_checked(
            app,
            patch,
            || approval.check(&ctx.data.profile),
        )?)
    } else {
        approval.check(&ctx.data.profile)?;
        value(crate::activity::tool_set_paused(
            app,
            name == "pause_activity",
        )?)
    }
}

fn invalid(message: &str) -> AppError {
    AppError::new("agent_tool_arguments_invalid", message)
}
fn value(value: impl serde::Serialize) -> Result<Value, AppError> {
    serde_json::to_value(value)
        .map_err(|e| AppError::new("agent_tool_response_invalid", e.to_string()))
}
fn parse_patch(args: &Value) -> Result<ActivitySettingsPatch, AppError> {
    let fields = args
        .get("settings")
        .and_then(Value::as_object)
        .filter(|fields| !fields.is_empty())
        .ok_or_else(|| invalid("settings must be a nonempty object."))?;
    fn meaningful(value: &Value) -> bool {
        match value {
            Value::Null => false,
            Value::Object(fields) => !fields.is_empty() && fields.values().all(meaningful),
            _ => true,
        }
    }
    if !fields.values().all(meaningful) {
        return Err(invalid(
            "Settings values must be non-null and nested updates must be nonempty.",
        ));
    }
    serde_json::from_value(Value::Object(fields.clone()))
        .map_err(|error| invalid(&format!("Invalid activity settings: {error}")))
}
fn object(properties: Value) -> Value {
    json!({"type":"object","properties":properties,"additionalProperties":false,"minProperties":1})
}
fn settings_schema() -> Value {
    let boolean = json!({"type":"boolean"});
    let time = json!({"type":"string","pattern":"^([01][0-9]|2[0-3]):[0-5][0-9]$"});
    object(json!({
        "enabled":boolean,"secondaryMonitors":boolean,"inputEvents":boolean,"pauseOnProtectedVideo":boolean,
        "ignoredApps":{"type":"array","maxItems":200,"items":{"type":"string"}},"ignoredDomains":{"type":"array","maxItems":200,"items":{"type":"string"}},
        "retentionDays":{"type":"integer","minimum":1,"maximum":365},
        "workHours":object(json!({"enabled":boolean,"days":{"type":"array","items":{"type":"integer","minimum":1,"maximum":7},"maxItems":7},"start":time,"end":time})),
        "daySummary":object(json!({"time":time})),
        "notifications":object(json!({"enabled":boolean,"quietHours":object(json!({"enabled":boolean,"start":time,"end":time}))})),
        "codingAgents":object(json!({"claudeCode":boolean,"codex":boolean,"copilotCli":boolean,"copilotVscode":boolean,"cursor":boolean,"cursorCli":boolean,"antigravity":boolean}))
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn partial_update_preserves_protected_and_unrelated_settings_when_capture_is_off() {
        let current = ActivitySettings {
            enabled: false,
            mcp_server: false,
            defaults_version: 17,
            ignored_apps: vec!["private.app".into()],
            ..ActivitySettings::default()
        };
        let args = json!({"settings":{"retentionDays":14}});
        let patch = parse_patch(&args).unwrap();
        let mut next = current;
        patch.apply(&mut next);
        assert!(!next.enabled);
        assert!(!next.mcp_server);
        assert_eq!(next.defaults_version, 17);
        assert_eq!(next.ignored_apps, vec!["private.app"]);
        assert_eq!(next.retention_days, 14);
        assert!(parse_patch(&json!({"settings":{"mcpServer":true}})).is_err());
        assert!(parse_patch(&json!({"settings":{"defaultsVersion":9}})).is_err());
        assert!(parse_patch(&json!({"settings":{"enabled":"yes"}})).is_err());
        assert!(parse_patch(&json!({"settings":{"workHours":{"unknown":true}}})).is_err());
        assert!(parse_patch(
            &json!({"settings":{"notifications":{"quietHours":{"unknown":true}}}})
        )
        .is_err());
        let existing_time = next.notifications.quiet_hours.start.clone();
        parse_patch(&json!({"settings":{"notifications":{"quietHours":{"enabled":false}}}}))
            .unwrap()
            .apply(&mut next);
        assert_eq!(next.notifications.quiet_hours.start, existing_time);
    }
}
