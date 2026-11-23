//! Activity capture: on by default with a persistent opt-out, text-only
//! recording of what the user sees and does on the Mac, stored in the
//! encrypted `activity.sqlite3`. See `docs/activity-capture.md` for the
//! schema, the settings upgrade, and the API later slices use.
//!
//! Layout: `engine` (2 s tick and privacy decisions), `platform` (OS seam,
//! `macos` implements it), `store` + `key` (SQLCipher database and its
//! Keychain key), `settings`, `filter`, `schedule`, `input`, `redact`,
//! `timeline` (sessions, gaps, categories, search; `docs/activity-timeline.md`),
//! and `app_icon` (local app icons by bundle id for "Top apps").

pub mod app_icon;
#[cfg(all(test, unix))]
#[path = "default_services_tests.rs"]
mod default_services_tests;
pub mod engine;
pub mod filter;
pub mod input;
pub mod key;
#[cfg(target_os = "macos")]
pub mod macos;
pub mod platform;
#[cfg(test)]
#[path = "red_tests.rs"]
mod red_tests;
pub mod redact;
pub mod schedule;
pub mod settings;
pub mod store;
pub mod timeline;
#[cfg(target_os = "windows")]
pub mod windows;

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::domain::types::AppError;
use engine::ActivityShared;
use key::ActivityKeyStore;
use platform::{ActivityPermission, PermissionSnapshot};
use schedule::{CaptureState, StoreStatus};
use settings::ActivitySettings;
use store::{ActivityStore, ACTIVITY_DB_FILE};

pub const ACTIVITY_STATE_EVENT: &str = "clovy://activity-state";
/// Rows written by the debug export.
const DEBUG_EXPORT_LIMIT: u32 = 500;

/// Everything the commands need once capture is supported on this platform.
pub struct ActivityRuntime {
    shared: Arc<ActivityShared>,
    keys: Arc<dyn ActivityKeyStore>,
    db_path: PathBuf,
    settings_path: PathBuf,
    data_dir: PathBuf,
}

/// Tauri-managed handle; `None` where capture is unsupported (Windows/Linux).
#[derive(Default)]
pub struct ActivityState(Option<ActivityRuntime>);

impl ActivityState {
    /// The state shared with the capture thread, for slices that use the
    /// same database (`crate::coding_agents`).
    pub fn shared(&self) -> Option<Arc<ActivityShared>> {
        self.0.as_ref().map(|runtime| Arc::clone(&runtime.shared))
    }

    /// For the slices that build on capture (`crate::day_intelligence`).
    pub fn runtime(&self) -> Option<&ActivityRuntime> {
        self.0.as_ref()
    }
}

impl ActivityRuntime {
    pub fn shared(&self) -> &Arc<ActivityShared> {
        &self.shared
    }

    pub fn data_dir(&self) -> &std::path::Path {
        &self.data_dir
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStatusDto {
    pub supported: bool,
    pub settings: ActivitySettings,
    pub state: CaptureState,
    pub permissions: PermissionSnapshot,
    pub manual_pause: bool,
    pub last_frame_at: Option<String>,
    pub debug_export_available: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDebugExportDto {
    pub path: String,
    pub frames: usize,
    pub secondary_frames: usize,
    pub input_events: usize,
    pub pauses: usize,
    pub coding_agent_blocks: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveActivitySettingsRequest {
    pub settings: ActivitySettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetActivityPausedRequest {
    pub paused: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestActivityPermissionRequest {
    pub permission: ActivityPermission,
}

fn status_of(runtime: Option<&ActivityRuntime>) -> ActivityStatusDto {
    match runtime {
        Some(runtime) => ActivityStatusDto {
            supported: true,
            settings: runtime.shared.settings(),
            state: runtime.shared.state(),
            permissions: runtime.shared.permissions(),
            manual_pause: runtime.shared.manual_pause(),
            last_frame_at: runtime.shared.last_frame_at(),
            debug_export_available: cfg!(debug_assertions),
        },
        None => ActivityStatusDto {
            supported: false,
            settings: ActivitySettings::default(),
            state: CaptureState::Off,
            permissions: PermissionSnapshot::UNSUPPORTED,
            manual_pause: false,
            last_frame_at: None,
            debug_export_available: false,
        },
    }
}

fn runtime<'a>(state: &'a State<'_, ActivityState>) -> Result<&'a ActivityRuntime, AppError> {
    state.0.as_ref().ok_or_else(|| {
        AppError::new(
            "activity_unsupported",
            "Activity settings are unavailable. Restart Clovy and try again.",
        )
    })
}

/// Pushes the current status to the frontend and the menu bar, and tells day
/// intelligence about capture failures (it notifies the user).
pub fn publish(app: &AppHandle) {
    let state = app.state::<ActivityState>();
    let status = status_of(state.0.as_ref());
    crate::menu_bar::set_activity_state(app, &status.state, status.manual_pause);
    crate::day_intelligence::on_capture_state(app, &status.state);
    let _ = app.emit(ACTIVITY_STATE_EVENT, status);
}

/// Pause or resume from the menu bar (same path as the settings button).
pub fn set_manual_pause(app: &AppHandle, paused: bool) {
    if let Some(runtime) = app.state::<ActivityState>().0.as_ref() {
        runtime.shared.set_manual_pause(paused);
    }
    publish(app);
}

/// Installation-wide settings port shared with bound MCP tools. Capture may be off.
pub(crate) fn tool_status(app: &AppHandle) -> ActivityStatusDto {
    status_of(app.state::<ActivityState>().0.as_ref())
}

/// Partial, typed user settings update. Unknown and internal fields are rejected.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ActivitySettingsPatch {
    enabled: Option<bool>,
    secondary_monitors: Option<bool>,
    input_events: Option<bool>,
    pause_on_protected_video: Option<bool>,
    ignored_apps: Option<Vec<String>>,
    ignored_domains: Option<Vec<String>>,
    retention_days: Option<u32>,
    work_hours: Option<WorkHoursPatch>,
    day_summary: Option<DaySummaryPatch>,
    notifications: Option<NotificationsPatch>,
    coding_agents: Option<CodingAgentsPatch>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkHoursPatch {
    enabled: Option<bool>,
    days: Option<Vec<u8>>,
    start: Option<String>,
    end: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DaySummaryPatch {
    time: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QuietHoursPatch {
    enabled: Option<bool>,
    start: Option<String>,
    end: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NotificationsPatch {
    enabled: Option<bool>,
    quiet_hours: Option<QuietHoursPatch>,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodingAgentsPatch {
    claude_code: Option<bool>,
    codex: Option<bool>,
    copilot_cli: Option<bool>,
    copilot_vscode: Option<bool>,
    cursor: Option<bool>,
    cursor_cli: Option<bool>,
    antigravity: Option<bool>,
}
macro_rules! apply_activity_fields {
    ($target:expr, $patch:expr, $($field:ident),+ $(,)?) => { $(if let Some(value) = $patch.$field { $target.$field = value; })+ };
}
impl ActivitySettingsPatch {
    pub(crate) fn apply(self, current: &mut ActivitySettings) {
        apply_activity_fields!(
            current,
            self,
            enabled,
            secondary_monitors,
            input_events,
            pause_on_protected_video,
            ignored_apps,
            ignored_domains,
            retention_days
        );
        if let Some(patch) = self.work_hours {
            apply_activity_fields!(current.work_hours, patch, enabled, days, start, end);
        }
        if let Some(patch) = self.day_summary {
            apply_activity_fields!(current.day_summary, patch, time);
        }
        if let Some(patch) = self.notifications {
            apply_activity_fields!(current.notifications, patch, enabled);
            if let Some(patch) = patch.quiet_hours {
                apply_activity_fields!(
                    current.notifications.quiet_hours,
                    patch,
                    enabled,
                    start,
                    end
                );
            }
        }
        if let Some(patch) = self.coding_agents {
            apply_activity_fields!(
                current.coding_agents,
                patch,
                claude_code,
                codex,
                copilot_cli,
                copilot_vscode,
                cursor,
                cursor_cli,
                antigravity
            );
        }
        *current = current.clone().normalized();
    }
}
pub(crate) async fn tool_update_settings(
    app: &AppHandle,
    patch: ActivitySettingsPatch,
    binding: &crate::mcp_server::turns::TurnBinding,
    pool: &sqlx_sqlite::SqlitePool,
    profile: &str,
) -> Result<ActivityStatusDto, AppError> {
    let app = app.clone();
    let binding = binding.clone();
    let pool = pool.clone();
    let profile = profile.to_owned();
    tokio::task::spawn_blocking(move || {
        // Notes authority and the activity settings lock have no reverse lock
        // dependency. Await the actual host/DB guard on this blocking worker.
        tool_update_settings_checked(&app, patch, || {
            tauri::async_runtime::block_on(crate::mcp_server::tools::revalidate_binding(
                Some(&app),
                &pool,
                &binding,
                &profile,
            ))
        })
    })
    .await
    .map_err(|error| AppError::new("activity_settings_save_failed", error.to_string()))?
}

pub(crate) fn tool_update_settings_checked(
    app: &AppHandle,
    patch: ActivitySettingsPatch,
    check: impl FnOnce() -> Result<(), AppError>,
) -> Result<ActivityStatusDto, AppError> {
    let state = app.state::<ActivityState>();
    let runtime = runtime(&state)?;
    let settings =
        runtime
            .shared
            .update_settings_checked(&runtime.settings_path, check, |current| {
                patch.apply(current)
            })?;
    if !settings.enabled {
        runtime.shared.set_manual_pause(false);
    }
    crate::coding_agents::wake(app);
    publish(app);
    Ok(status_of(Some(runtime)))
}

pub(crate) fn tool_set_paused(
    app: &AppHandle,
    paused: bool,
) -> Result<ActivityStatusDto, AppError> {
    activity_set_paused(
        app.clone(),
        app.state::<ActivityState>(),
        SetActivityPausedRequest { paused },
    )
}

#[tauri::command]
pub fn activity_status(state: State<'_, ActivityState>) -> ActivityStatusDto {
    status_of(state.0.as_ref())
}

#[tauri::command]
pub fn activity_save_settings(
    app: AppHandle,
    state: State<'_, ActivityState>,
    request: SaveActivitySettingsRequest,
) -> Result<ActivityStatusDto, AppError> {
    let runtime = runtime(&state)?;
    let settings = save_user_settings(&runtime.shared, &runtime.settings_path, request.settings)
        .map_err(|error| AppError::new("activity_settings_save_failed", error.to_string()))?;
    if !settings.enabled {
        runtime.shared.set_manual_pause(false);
    }
    crate::coding_agents::wake(&app);
    publish(&app);
    Ok(status_of(Some(runtime)))
}

/// Stores what Settings, Activity sent. The MCP server switch lives in
/// Settings, Agent; a stale copy from the Activity tab must not flip it, and
/// the request does not carry `defaults_version`. The update reads the latest
/// values under the same lock [`store_mcp_server_switch`] takes.
pub(crate) fn save_user_settings(
    shared: &ActivityShared,
    path: &std::path::Path,
    requested: ActivitySettings,
) -> std::io::Result<ActivitySettings> {
    let requested = requested.normalized();
    shared.update_settings(path, |current| {
        *current = ActivitySettings {
            mcp_server: current.mcp_server,
            defaults_version: current.defaults_version,
            ..requested
        };
    })
}

/// Stores the Clovy MCP server switch, keeping every other setting.
pub(crate) fn store_mcp_server_switch(
    shared: &ActivityShared,
    path: &std::path::Path,
    enabled: bool,
) -> std::io::Result<ActivitySettings> {
    shared.update_settings(path, |settings| settings.mcp_server = enabled)
}

/// Whether this platform has activity settings (and so the MCP switch).
pub(crate) fn activity_settings_supported(app: &AppHandle) -> bool {
    app.state::<ActivityState>().0.is_some()
}

/// The stored Clovy MCP server switch; `false` where activity settings are
/// unsupported.
pub(crate) fn mcp_server_enabled(app: &AppHandle) -> bool {
    app.state::<ActivityState>()
        .0
        .as_ref()
        .is_some_and(|runtime| runtime.shared.settings().mcp_server)
}

/// Persists the Clovy MCP server switch (`crate::mcp_server` owns the
/// listener it controls).
pub(crate) fn set_mcp_server_enabled(app: &AppHandle, enabled: bool) -> Result<(), AppError> {
    let state = app.state::<ActivityState>();
    let runtime = runtime(&state)?;
    store_mcp_server_switch(&runtime.shared, &runtime.settings_path, enabled)
        .map_err(|error| AppError::new("activity_settings_save_failed", error.to_string()))?;
    publish(app);
    Ok(())
}

#[tauri::command]
pub fn activity_set_paused(
    app: AppHandle,
    state: State<'_, ActivityState>,
    request: SetActivityPausedRequest,
) -> Result<ActivityStatusDto, AppError> {
    let runtime = runtime(&state)?;
    runtime.shared.set_manual_pause(request.paused);
    publish(&app);
    Ok(status_of(Some(runtime)))
}

#[tauri::command]
pub fn activity_request_permission(
    state: State<'_, ActivityState>,
    request: RequestActivityPermissionRequest,
) -> Result<ActivityStatusDto, AppError> {
    let runtime = runtime(&state)?;
    runtime.shared.request_permission(request.permission);
    Ok(status_of(Some(runtime)))
}

/// User-confirmed: deletes the unreadable database and starts a new one with
/// a new key. The old data cannot be recovered without the old key.
#[tauri::command]
pub async fn activity_recreate_database(
    app: AppHandle,
    state: State<'_, ActivityState>,
) -> Result<ActivityStatusDto, AppError> {
    let runtime = runtime(&state)?;
    if let Some(store) = runtime.shared.take_store() {
        store.close().await;
    }
    match ActivityStore::recreate(&runtime.db_path, runtime.keys.as_ref()).await {
        Ok(store) => runtime
            .shared
            .replace_store(Some(store), StoreStatus::Ready),
        Err(error) => {
            runtime
                .shared
                .replace_store(None, StoreStatus::Failed(error.to_string()));
            return Err(AppError::new("activity_recreate_failed", error.to_string()));
        }
    }
    publish(&app);
    Ok(status_of(Some(runtime)))
}

/// Development builds only: writes the newest activity rows to a readable
/// JSON file in the (dev) data directory for verification. The key is never
/// part of the output.
#[tauri::command]
pub async fn activity_debug_export(
    state: State<'_, ActivityState>,
) -> Result<ActivityDebugExportDto, AppError> {
    if !cfg!(debug_assertions) {
        return Err(AppError::new(
            "activity_debug_export_unavailable",
            "The activity debug export exists only in development builds.",
        ));
    }
    let runtime = runtime(&state)?;
    let store = runtime.shared.store().ok_or_else(|| {
        AppError::new(
            "activity_database_closed",
            "The activity database is not open. Turn capture on first.",
        )
    })?;
    let mut extra = timeline::db::debug_sections(&store, DEBUG_EXPORT_LIMIT)
        .await
        .map_err(|error| AppError::new("activity_debug_export_failed", error.to_string()))?;
    extra.extend(
        crate::day_intelligence::db::debug_sections(&store)
            .await
            .map_err(|error| AppError::new("activity_debug_export_failed", error.to_string()))?,
    );
    let export = store
        .debug_export(
            &runtime.data_dir.join("activity-debug-exports"),
            chrono::Utc::now(),
            DEBUG_EXPORT_LIMIT,
            extra,
        )
        .await
        .map_err(|error| AppError::new("activity_debug_export_failed", error.to_string()))?;
    Ok(ActivityDebugExportDto {
        path: export.path.display().to_string(),
        frames: export.frames,
        secondary_frames: export.secondary_frames,
        input_events: export.input_events,
        pauses: export.pauses,
        coding_agent_blocks: export.coding_agent_blocks,
    })
}

/// Registers the managed state and, on macOS, starts the capture thread. The
/// thread ticks while capture is off too (only permissions are read then), so
/// the Activity tab shows live permission state.
pub fn setup(app: &mut tauri::App) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let state = setup_state(|| start(app.handle()));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let state = ActivityState::default();
    app.manage(state);
}

#[cfg(any(test, target_os = "macos", target_os = "windows"))]
fn setup_state<E: std::fmt::Display>(
    start: impl FnOnce() -> Result<ActivityRuntime, E>,
) -> ActivityState {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    match start() {
        Ok(runtime) => return ActivityState(Some(runtime)),
        Err(error) => tracing::warn!(%error, "activity capture unavailable"),
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = start;
    ActivityState::default()
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn start(app: &AppHandle) -> Result<ActivityRuntime, tauri::Error> {
    let data_dir = crate::app_paths::app_data_dir(app)?;
    let config_dir = crate::app_paths::app_config_dir(app)?;
    let settings_path = settings::settings_path(&config_dir);
    let db_path = data_dir.join(ACTIVITY_DB_FILE);
    let shared = Arc::new(ActivityShared::new(settings::load_at_startup(
        &settings_path,
    )));
    let keys: Arc<dyn ActivityKeyStore> = Arc::new(key::KeychainKeyStore::for_current_build());

    let thread_shared = Arc::clone(&shared);
    let thread_keys = Arc::clone(&keys);
    let thread_db = db_path.clone();
    let handle = app.clone();
    timeline::start(app.clone(), Arc::clone(&shared), data_dir.clone());
    std::thread::Builder::new()
        .name("clovy-activity-capture".into())
        .spawn(move || {
            #[cfg(target_os = "macos")]
            let platform = macos::MacActivityPlatform::new();
            #[cfg(target_os = "windows")]
            let platform = windows::WinActivityPlatform::new();
            let engine = engine::CaptureEngine::new(
                platform,
                thread_shared,
                thread_keys,
                thread_db,
                Box::new(engine::SystemClock),
                std::process::id() as i32,
            );
            engine::run_capture_loop(engine, schedule::TICK, |report| {
                if report.state_changed || report.permissions_changed {
                    publish(&handle);
                }
            });
        })?;

    Ok(ActivityRuntime {
        shared,
        keys,
        db_path,
        settings_path,
        data_dir,
    })
}

#[cfg(all(test, target_os = "windows"))]
mod windows_setup_tests {
    use super::*;
    use crate::activity::engine::{tests::FakePlatform, CaptureEngine, SystemClock};
    use crate::activity::key::MemoryKeyStore;

    #[tokio::test]
    async fn windows_setup_keeps_a_runtime_that_captures_with_an_injected_platform_and_vault() {
        let dir = tempfile::tempdir().unwrap();
        let keys = Arc::new(MemoryKeyStore::default());
        let state = setup_state(|| {
            Ok::<_, std::convert::Infallible>(ActivityRuntime {
                shared: Arc::new(ActivityShared::new(ActivitySettings {
                    enabled: true,
                    ..ActivitySettings::default()
                })),
                keys: keys.clone(),
                db_path: dir.path().join(ACTIVITY_DB_FILE),
                settings_path: dir.path().join("activity-settings.json"),
                data_dir: dir.path().to_path_buf(),
            })
        });
        let runtime = state
            .0
            .as_ref()
            .expect("Windows setup must keep the runtime");
        let mut engine = CaptureEngine::new(
            FakePlatform::granted(),
            runtime.shared.clone(),
            runtime.keys.clone(),
            runtime.db_path.clone(),
            Box::new(SystemClock),
            1,
        );
        engine.tick().await;
        assert!(status_of(state.0.as_ref()).supported);
        assert_eq!(runtime.shared.state(), CaptureState::Active);
        let store = runtime.shared.store().expect("capture must open its store");
        let frames = store.frames_after(0, 10).await.unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].app_name, "Zed");
        assert!(
            keys.current().is_some(),
            "the injected vault must receive a key"
        );
        store.close().await;
    }
}
