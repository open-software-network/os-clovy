//! Capture and the Clovy MCP server start without a visit to Settings: new
//! and partial profiles, and profiles saved before this behavior (turned on
//! once). A later opt-out of either survives restarts and saves made by the
//! other switch. Kept in its own file so the ticket's RED/GREEN receipts
//! hash fixed test text.

use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Map, Value};

use super::engine::tests::{FakePlatform, Harness};
use super::engine::ActivityShared;
use super::platform::{ActivityPermission, PermissionState};
use super::schedule::CaptureState;
use super::settings::{self, ActivitySettings};
use super::{save_user_settings, store_mcp_server_switch, SaveActivitySettingsRequest};
use crate::mcp_server::tests::{by_id, data, handler_for, notes_pool, run_relay};
use crate::mcp_server::tools::ActivityAccess;
use crate::mcp_server::{channel, stdio, McpServerState};

/// Saved before this behavior: both switches off, plus choices the upgrade
/// must keep.
const LEGACY_OFF: &str = r#"{
  "enabled": false,
  "secondaryMonitors": false,
  "inputEvents": true,
  "pauseOnProtectedVideo": true,
  "ignoredApps": ["Slack"],
  "ignoredDomains": ["youtube.com"],
  "workHours": {"enabled": true, "days": [1, 2, 3], "start": "08:00", "end": "16:00"},
  "retentionDays": 14,
  "mcpServer": false
}"#;

/// Saved before this behavior with capture on and the MCP server off.
const LEGACY_CAPTURE_ONLY: &str = r#"{"enabled": true, "retentionDays": 14, "mcpServer": false}"#;

/// Fields of `ActivitySettingsDto` (`src/lib/activity-capture.ts`): what
/// Settings, Activity sends back.
const UI_FIELDS: [&str; 11] = [
    "enabled",
    "secondaryMonitors",
    "inputEvents",
    "pauseOnProtectedVideo",
    "ignoredApps",
    "ignoredDomains",
    "workHours",
    "retentionDays",
    "daySummary",
    "notifications",
    "codingAgents",
];

/// One app start: what `activity::start` loads from the config dir.
fn boot(config: &Path) -> ActivitySettings {
    settings::load_at_startup(&settings::settings_path(config))
}

fn write_profile(config: &Path, json: &str) {
    std::fs::write(settings::settings_path(config), json).unwrap();
}

/// The `activity_save_settings` request Settings, Activity builds from
/// `current`, after `change`.
fn ui_request(
    current: &ActivitySettings,
    change: impl FnOnce(&mut Map<String, Value>),
) -> SaveActivitySettingsRequest {
    let Value::Object(all) = serde_json::to_value(current).unwrap() else {
        unreachable!("settings serialize to an object")
    };
    let mut dto: Map<String, Value> = all
        .into_iter()
        .filter(|(key, _)| UI_FIELDS.contains(&key.as_str()))
        .collect();
    change(&mut dto);
    serde_json::from_value(json!({ "settings": dto })).unwrap()
}

/// What the app hands the MCP handler for these settings: activity tools
/// exist only while capture is on.
fn activity_access(settings: &ActivitySettings) -> ActivityAccess {
    if settings.enabled {
        ActivityAccess::On
    } else {
        ActivityAccess::Off
    }
}

/// Starts or stops the listener under `root` from `settings`, as the app's
/// reconcile does at launch.
async fn reconcile(state: &McpServerState, root: &Path, settings: &ActivitySettings) {
    let handler = handler_for(data(notes_pool().await, activity_access(settings)));
    state
        .apply_switch(
            settings.mcp_server,
            || Ok(channel::channel_dir(root)),
            || handler,
        )
        .await;
}

async fn running(state: &McpServerState) -> bool {
    state.listener_status().await.0
}

/// `tools/list` through the real `clovy-mcp` relay: the tool names, or the
/// error the client gets.
async fn list_tools(root: &Path) -> Result<Vec<String>, Value> {
    let output = run_relay(
        &channel::channel_dir(root),
        &[
            json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } } }),
            json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
            json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }),
        ],
    )
    .await;
    let messages = output.messages(2);
    let answer = by_id(&messages, 2);
    match answer["result"]["tools"].as_array() {
        Some(tools) => Ok(tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect()),
        None => Err(answer["error"].clone()),
    }
}

fn missing_permissions() -> FakePlatform {
    let mut platform = FakePlatform::granted();
    platform.permissions.accessibility = PermissionState::Denied;
    platform.permissions.screen_recording = PermissionState::NotDetermined;
    platform
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_profile_starts_capture_and_the_mcp_server() {
    let config = tempfile::tempdir().unwrap();
    let settings = boot(config.path());
    assert!(settings.enabled, "capture is off on a new profile");
    assert!(settings.mcp_server, "MCP server is off on a new profile");

    let mut harness = Harness::new(FakePlatform::granted(), settings.clone());
    harness.engine.tick().await;
    assert_eq!(harness.shared.state(), CaptureState::Active);
    assert_eq!(harness.frames().await.len(), 1);

    // Without permissions nothing is recorded and the switch stays on.
    let mut blocked = Harness::new(missing_permissions(), settings.clone());
    blocked.engine.tick().await;
    assert_eq!(
        blocked.shared.state(),
        CaptureState::NeedsPermissions {
            missing: vec![
                ActivityPermission::Accessibility,
                ActivityPermission::ScreenRecording
            ]
        }
    );
    assert!(blocked.frames().await.is_empty());
    assert!(blocked.shared.settings().enabled);

    let root = tempfile::tempdir().unwrap();
    let state = McpServerState::default();
    reconcile(&state, root.path(), &settings).await;
    assert!(running(&state).await);
    let tools = list_tools(root.path()).await.expect("tools/list answered");
    assert!(tools.contains(&"search_notes".to_string()));
    assert!(tools.contains(&"get_activity_timeline".to_string()));
}

#[test]
fn a_partial_profile_turns_on_what_it_does_not_mention() {
    let config = tempfile::tempdir().unwrap();
    write_profile(config.path(), r#"{"retentionDays": 14}"#);
    let settings = boot(config.path());
    assert!(settings.enabled && settings.mcp_server);
    assert_eq!(settings.retention_days, 14);
}

#[test]
fn legacy_profiles_are_turned_on_once_keeping_their_other_choices() {
    for legacy in [LEGACY_OFF, LEGACY_CAPTURE_ONLY] {
        let config = tempfile::tempdir().unwrap();
        write_profile(config.path(), legacy);
        for start in 1..=2 {
            let settings = boot(config.path());
            assert!(settings.enabled, "capture off at start {start}: {legacy}");
            assert!(settings.mcp_server, "MCP off at start {start}: {legacy}");
            assert_eq!(settings.retention_days, 14);
        }
    }
    let config = tempfile::tempdir().unwrap();
    write_profile(config.path(), LEGACY_OFF);
    let settings = boot(config.path());
    assert_eq!(settings.ignored_apps, vec!["Slack"]);
    assert_eq!(settings.ignored_domains, vec!["youtube.com"]);
    assert!(settings.work_hours.enabled);
    assert_eq!(settings.work_hours.days, vec![1, 2, 3]);
    assert_eq!(settings.work_hours.start, "08:00");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_opt_out_survives_restarts_and_mcp_switch_saves() {
    let config = tempfile::tempdir().unwrap();
    let path = settings::settings_path(config.path());
    write_profile(config.path(), LEGACY_OFF);

    let first = ActivityShared::new(boot(config.path()));
    let request = ui_request(&first.settings(), |dto| {
        dto.insert("enabled".into(), json!(false));
    });
    save_user_settings(&first, &path, request.settings).unwrap();

    let second = ActivityShared::new(boot(config.path()));
    assert!(!second.settings().enabled, "capture opt-out undone");
    assert!(second.settings().mcp_server);
    store_mcp_server_switch(&second, &path, false).unwrap();
    store_mcp_server_switch(&second, &path, true).unwrap();

    for start in 3..=4 {
        let settings = boot(config.path());
        assert!(!settings.enabled, "capture opt-out undone at start {start}");
        assert!(settings.mcp_server);
    }
    let settings = boot(config.path());
    let mut harness = Harness::new(FakePlatform::granted(), settings.clone());
    harness.engine.tick().await;
    assert_eq!(harness.shared.state(), CaptureState::Off);
    assert!(harness.shared.store().is_none(), "store opened while off");

    // The MCP server keeps serving notes, without activity tools.
    let root = tempfile::tempdir().unwrap();
    let state = McpServerState::default();
    reconcile(&state, root.path(), &settings).await;
    let tools = list_tools(root.path()).await.expect("tools/list answered");
    for name in ["search_notes", "get_note"] {
        assert!(tools.iter().any(|tool| tool == name), "missing {name}");
    }
    assert!(tools
        .iter()
        .all(|name| !crate::mcp_server::tools::is_activity_tool(name)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_opt_out_survives_restarts_and_activity_saves() {
    let config = tempfile::tempdir().unwrap();
    let path = settings::settings_path(config.path());
    write_profile(config.path(), LEGACY_OFF);
    let root = tempfile::tempdir().unwrap();

    let first = Arc::new(ActivityShared::new(boot(config.path())));
    let state = McpServerState::default();
    reconcile(&state, root.path(), &first.settings()).await;
    assert!(
        running(&state).await,
        "MCP server not started after upgrade"
    );
    store_mcp_server_switch(&first, &path, false).unwrap();
    reconcile(&state, root.path(), &first.settings()).await;
    assert!(!running(&state).await);

    // A stale Activity tab sends `mcpServer: true` with its own change.
    let second = ActivityShared::new(boot(config.path()));
    assert!(!second.settings().mcp_server, "MCP opt-out undone");
    let request = ui_request(&second.settings(), |dto| {
        dto.insert("retentionDays".into(), json!(7));
        dto.insert("mcpServer".into(), json!(true));
    });
    save_user_settings(&second, &path, request.settings).unwrap();

    for start in 3..=4 {
        let settings = boot(config.path());
        assert!(!settings.mcp_server, "MCP opt-out undone at start {start}");
        assert!(settings.enabled);
        assert_eq!(settings.retention_days, 7);
    }
    let settings = boot(config.path());
    let state = McpServerState::default();
    reconcile(&state, root.path(), &settings).await;
    assert!(!running(&state).await);
    let error = list_tools(root.path()).await.expect_err("server answered");
    assert_eq!(error["message"], json!(stdio::OFF_MESSAGE));

    // Capture keeps running.
    let mut harness = Harness::new(FakePlatform::granted(), settings);
    harness.engine.tick().await;
    assert_eq!(harness.shared.state(), CaptureState::Active);
    assert_eq!(harness.frames().await.len(), 1);
}

fn set_writable(config: &Path, writable: bool) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if writable { 0o755 } else { 0o555 };
    std::fs::set_permissions(config, std::fs::Permissions::from_mode(mode)).unwrap();
}

/// Starts with `config` read-only, so the upgrade cannot be saved; returns
/// the running settings with `config` writable again.
fn boot_unsaved(config: &Path) -> ActivityShared {
    set_writable(config, false);
    let shared = ActivityShared::new(boot(config));
    set_writable(config, true);
    let stored = std::fs::read_to_string(settings::settings_path(config)).unwrap();
    assert_eq!(stored, LEGACY_OFF, "the upgrade was written after all");
    shared
}

#[test]
fn an_unsaved_upgrade_runs_on_and_refuses_choices_it_cannot_store() {
    let config = tempfile::tempdir().unwrap();
    let path = settings::settings_path(config.path());
    write_profile(config.path(), LEGACY_OFF);
    let first = boot_unsaved(config.path());
    assert!(first.settings().enabled && first.settings().mcp_server);

    // Opt-outs that cannot be written fail and change nothing.
    set_writable(config.path(), false);
    let request = ui_request(&first.settings(), |dto| {
        dto.insert("enabled".into(), json!(false));
    });
    assert!(save_user_settings(&first, &path, request.settings).is_err());
    assert!(store_mcp_server_switch(&first, &path, false).is_err());
    set_writable(config.path(), true);
    assert!(first.settings().enabled && first.settings().mcp_server);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), LEGACY_OFF);

    // Nothing was saved, so the next start upgrades again.
    let settings = boot(config.path());
    assert!(settings.enabled && settings.mcp_server);
}

#[test]
fn after_an_unsaved_upgrade_either_switch_saves_its_opt_out_for_good() {
    for capture_switch in [true, false] {
        let config = tempfile::tempdir().unwrap();
        let path = settings::settings_path(config.path());
        write_profile(config.path(), LEGACY_OFF);
        let first = boot_unsaved(config.path());
        assert!(first.settings().enabled && first.settings().mcp_server);

        if capture_switch {
            let request = ui_request(&first.settings(), |dto| {
                dto.insert("enabled".into(), json!(false));
            });
            save_user_settings(&first, &path, request.settings).unwrap();
        } else {
            store_mcp_server_switch(&first, &path, false).unwrap();
        }
        for start in 2..=3 {
            let settings = boot(config.path());
            assert_eq!(
                (settings.enabled, settings.mcp_server),
                (!capture_switch, capture_switch),
                "opt-out undone at start {start} (capture switch: {capture_switch})"
            );
        }
    }
}
