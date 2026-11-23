#![recursion_limit = "256"]

pub mod activity;
pub mod agent_hud;
pub mod agent_mcp;
pub mod agent_recorder;
pub mod agent_sessions;
pub mod app_identity;
pub mod app_paths;
pub mod app_preferences;
pub mod approval_broker;
pub mod audio;
mod background_image;
pub mod browser;
mod browser_broker;
pub mod chat_engine;
pub mod claude_projects;
pub mod coding_agents;
pub mod commands;
pub mod computer_use;
#[allow(dead_code)] // Shared by the native helper and desktop ownership paths.
mod computer_use_cancel;
mod computer_use_cursor;
#[cfg(target_os = "macos")]
#[allow(dead_code)] // Permission entry points are used by the helper binary.
mod computer_use_macos;
mod computer_use_owned_runtime;
mod computer_use_permission_drag;
mod computer_use_policy;
#[cfg(target_os = "windows")]
#[allow(dead_code)] // Native server and client share the same audited module.
mod computer_use_windows;
pub mod connectors;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod credentials;
pub mod day_intelligence;
pub mod db;
pub mod dictation;
pub mod domain;
pub mod experimental_settings;
pub mod feature_flags;
mod filesystem;
pub(crate) mod hide_console;
mod http_client;
#[cfg(all(target_os = "macos", debug_assertions))]
pub mod identity_migration;
pub mod image_safety;
pub mod interface_locale;
pub mod llm;
mod local_env;
pub mod macos_menu_icons;
pub mod main_window;
pub mod mcp_server;
mod meeting_assist;
pub mod meeting_calendar_context;
pub mod meeting_detection;
pub mod meeting_hud;
pub mod menu_bar;
mod note_audio_export;
mod note_save_flush;
pub mod notifications;
pub mod obsidian;
mod process_group;
pub mod providers;
pub mod routines;
mod shutdown;
mod system_browser;
pub mod theme_icon;
#[cfg(target_os = "macos")]
pub mod transcription_model;
pub mod update_gate;
pub mod updates;
pub mod video_download_url;

use serde::Deserialize;
#[cfg(target_os = "macos")]
use std::sync::OnceLock;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Listener, Manager};

const CLOSE_TAB_MENU_ID: &str = "close_tab";
const CLOSE_TAB_EVENT: &str = "clovy://close-tab";
const CLOSE_WINDOW_MENU_ID: &str = "close_window_main";
const OPEN_SETTINGS_MENU_ID: &str = "open_settings";
const OPEN_SETTINGS_EVENT: &str = "clovy://open-settings";

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecordingPresenceBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl RecordingPresenceBounds {
    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }

    fn is_valid(self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetRecordingPresenceBoundsRequest {
    owner_id: String,
    bounds: Option<RecordingPresenceBounds>,
}

#[derive(Debug, Clone)]
struct RegisteredRecordingPresenceBounds {
    owner_id: String,
    bounds: RecordingPresenceBounds,
}

#[derive(Default)]
struct RecordingPresenceBoundsState(Mutex<Option<RegisteredRecordingPresenceBounds>>);

#[cfg(target_os = "macos")]
static MAIN_WINDOW_APP: OnceLock<tauri::AppHandle> = OnceLock::new();
#[cfg(target_os = "macos")]
static MAIN_WINDOW_NS_WINDOW: Mutex<Option<usize>> = Mutex::new(None);
#[cfg(target_os = "macos")]
static MAIN_WINDOW_SEND_EVENT_ORIGINAL: OnceLock<MainWindowSendEventImp> = OnceLock::new();
#[cfg(target_os = "macos")]
type MainWindowSendEventImp = unsafe extern "C-unwind" fn(
    &objc2::runtime::AnyObject,
    objc2::runtime::Sel,
    *mut objc2::runtime::AnyObject,
);

pub fn run() {
    providers::load_local_env();
    // Before Tauri creates a window (and its WebView store) or opens a
    // database: the first launch copies the June-era identity (ADR-0068).
    // Only the macOS development build has a June-era profile to copy; a
    // release and Windows start clean.
    #[cfg(all(target_os = "macos", debug_assertions))]
    identity_migration::run_at_startup();
    let context = tauri::generate_context!();
    let mut builder = tauri::Builder::default();

    // Single-instance MUST register before the deep-link plugin so it owns
    // the second-launch handoff: when a deep link fires while the app is
    // already running, the OS launches a new instance, single-instance
    // forwards argv (which includes the URL on macOS) to the live instance,
    // and tauri-plugin-single-instance's `deep-link` feature wires that into
    // the deep-link plugin's `on_open_url` event so we hear one signal in
    // both cold-launch and warm-launch cases.
    #[cfg(desktop)]
    {
        if should_register_single_instance_plugin() {
            builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
                main_window::show(app);
            }));
        }
    }

    builder
        // deep-link registers immediately after single-instance to keep the
        // single-instance -> deep-link handoff (documented above) adjacent and
        // obvious; the updater is order-independent so it follows.
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Endpoint and public key come from `plugins.updater` in
        // tauri.conf.json; only a release build checks (see `updates`).
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Launch-at-login. LaunchAgent (not AppleScript) so the entry
        // survives macOS upgrades and shows up under System Settings ->
        // General -> Login Items. Enabling/disabling stays a user action in
        // Settings and onboarding; registering the plugin alone changes
        // nothing.
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Clovy")
                .build(),
        )
        .on_menu_event(|app, event| {
            if event.id().as_ref() == CLOSE_TAB_MENU_ID {
                emit_close_tab_if_main_window_focused(app);
                return;
            }
            if event.id().as_ref() == CLOSE_WINDOW_MENU_ID {
                close_main_window(app);
                return;
            }
            if event.id().as_ref() == OPEN_SETTINGS_MENU_ID {
                main_window::show(app);
                let _ = app.emit(OPEN_SETTINGS_EVENT, ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            background_image::choose_background_image,
            background_image::read_background_image,
            background_image::remove_background_image,
            theme_icon::set_dock_icon,
            print_current_webview,
            commands::bootstrap_app,
            meeting_assist::list_recording_marks,
            meeting_assist::add_recording_mark,
            meeting_assist::suggest_recording_question,
            meeting_assist::get_recording_assistance_status,
            agent_sessions::api::list_agent_sessions,
            agent_sessions::api::get_agent_session,
            agent_sessions::api::get_latest_agent_run,
            agent_sessions::api::create_agent_session,
            agent_sessions::api::rename_agent_session,
            agent_sessions::api::branch_agent_session,
            agent_sessions::api::delete_agent_session,
            agent_sessions::api::list_agent_items,
            agent_sessions::api::import_legacy_home_turns,
            agent_sessions::api::stage_agent_attachment_bytes,
            agent_sessions::api::discard_staged_agent_attachments,
            agent_sessions::api::prune_staged_agent_attachments,
            agent_sessions::api::start_agent_run,
            agent_sessions::api::cancel_agent_run,
            agent_sessions::api::retry_agent_run,
            agent_sessions::api::resolve_agent_interruption,
            agent_sessions::api::list_agent_artifacts,
            agent_sessions::api::read_agent_artifact_preview,
            agent_sessions::api::read_agent_artifact_text,
            agent_sessions::api::download_agent_artifact,
            agent_sessions::api::list_agent_skills,
            agent_sessions::api::read_agent_skill,
            agent_sessions::api::update_agent_skill,
            agent_sessions::api::set_agent_skill_enabled,
            agent_sessions::persona::clovy_persona,
            agent_sessions::persona::set_clovy_persona,
            agent_recorder::resolve_agent_recorder_request,
            agent_recorder::validate_agent_recorder_request,
            dictation::claim_agent_dictation_delivery,
            browser_broker::browser_access_get,
            browser_broker::browser_access_set,
            browser_broker::browser_sessions,
            browser_broker::browser_human_takeover,
            browser_broker::browser_resume_control,
            browser_broker::browser_stop_session,
            agent_mcp::list_agent_mcp_servers,
            agent_mcp::create_agent_mcp_server,
            agent_mcp::update_agent_mcp_server,
            agent_mcp::delete_agent_mcp_server,
            agent_mcp::test_agent_mcp_server,
            agent_mcp::connect_agent_mcp_oauth,
            agent_mcp::cancel_agent_mcp_oauth,
            agent_mcp::disconnect_agent_mcp_oauth,
            routines::list_agent_routines,
            routines::create_agent_routine,
            routines::update_agent_routine,
            routines::pause_agent_routine,
            routines::resume_agent_routine,
            routines::trigger_agent_routine,
            routines::delete_agent_routine,
            routines::list_agent_routine_runs,
            routines::routine_browser_access_get,
            routines::routine_browser_access_set,
            commands::app_preferences_get,
            commands::app_preferences_set,
            commands::experimental_flags_get,
            commands::experimental_flags_set,
            commands::create_note,
            commands::list_notes,
            commands::get_note,
            commands::download_note_audio,
            commands::update_note,
            note_save_flush::complete_note_save_flush,
            commands::delete_note,
            commands::delete_notes,
            commands::create_folder,
            commands::discover_claude_projects,
            commands::import_claude_projects,
            commands::list_folders,
            commands::delete_folder,
            commands::rename_folder,
            commands::assign_note_to_folder,
            commands::remove_note_from_folder,
            commands::list_session_folders,
            commands::assign_session_to_folder,
            commands::list_session_profiles,
            commands::assign_session_to_profile,
            commands::sticky_active_profile,
            commands::profile_data_summary,
            commands::move_profile_data_to_default,
            commands::delete_profile_data,
            commands::remove_session_from_folder,
            commands::list_completed_sessions,
            commands::set_session_completed,
            commands::list_dictionary_entries,
            commands::create_dictionary_entry,
            commands::update_dictionary_entry,
            commands::delete_dictionary_entry,
            commands::list_memories,
            commands::create_memory,
            commands::update_memory,
            commands::delete_memory,
            commands::set_folder_instructions,
            commands::set_folder_memory_disabled,
            commands::memory_settings,
            commands::set_memory_enabled,
            commands::get_microphone_permission_state,
            commands::check_recording_source_readiness,
            commands::open_privacy_settings,
            commands::reveal_path,
            commands::clovy_open_external_url,
            commands::june_open_external_url,
            commands::start_meeting_recording,
            commands::detect_meeting_window,
            commands::start_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::get_recording_status,
            set_recording_presence_bounds,
            commands::finish_recording,
            commands::retry_processing,
            commands::recover_recording,
            dictation::dictation_capabilities,
            dictation::dictation_settings,
            dictation::list_dictation_history,
            dictation::delete_dictation_history_item,
            dictation::set_dictation_shortcut,
            dictation::set_dictation_microphone,
            dictation::set_dictation_style,
            dictation::set_dictation_language,
            dictation::set_dictation_cleanup,
            dictation::set_dictation_hud_placement,
            dictation::dictation_hud_start_drag,
            dictation::dictation_helper_command,
            dictation::dictation_hud_set_stop_bounds,
            dictation::dictation_hud_set_cancel_bounds,
            dictation::dictation_hud_set_dismiss_bounds,
            dictation::dictation_hud_set_record_bounds,
            dictation::dictation_hud_preferred_error_placement,
            dictation::dictation_hud_set_tooltip_visible,
            dictation::dictation_hud_set_size,
            dictation::dictation_hud_set_alpha,
            dictation::dictation_hud_show,
            dictation::dictation_hud_exit,
            dictation::dictation_hud_set_chrome,
            dictation::dictation_hud_shake,
            dictation::dictation_hotkey_status,
            dictation::latest_dictation_event,
            agent_hud::agent_hud_show,
            agent_hud::agent_hud_hide,
            agent_hud::agent_hud_set_layout,
            agent_hud::agent_hud_open_agent,
            agent_hud::agent_hud_main_focused,
            notifications::send_app_notification,
            notifications::agent_open_ready,
            notifications::routine_open_ready,
            notifications::today_open_ready,
            meeting_detection::queue_meeting_start_request,
            meeting_detection::pending_meeting_start_request,
            meeting_detection::acknowledge_meeting_start_request,
            meeting_detection::pending_meeting_end_status,
            meeting_detection::pending_meeting_end_finish_request,
            meeting_detection::queue_meeting_end_finish_request,
            meeting_detection::keep_meeting_recording,
            meeting_detection::acknowledge_meeting_end_finish_request,
            meeting_detection::debug_force_meeting_end_countdown,
            meeting_hud::meeting_hud_latest_status,
            meeting_hud::meeting_hud_reopen,
            meeting_hud::meeting_hud_set_end_prompt_expanded,
            providers::provider_model_settings,
            providers::profile_model_overrides,
            providers::set_profile_model_overrides,
            providers::delete_profile_model_overrides,
            providers::list_provider_models,
            providers::set_provider_model,
            commands::local_transcription_model_state,
            commands::install_local_transcription_model,
            commands::cancel_local_transcription_model_install,
            commands::remove_local_transcription_model,
            providers::set_provider_api_key,
            providers::clear_provider_api_key,
            providers::set_image_safe_mode,
            providers::set_live_transcription,
            providers::set_image_safe_mode_prompt_dismissed,
            image_safety::image_prompt_may_be_explicit,
            providers::generate_image,
            providers::edit_image,
            providers::video_generate,
            providers::video_status,
            providers::generated_video_dir,
            providers::probe_local_generation_endpoint,
            llm::commands::llm_providers,
            llm::commands::llm_detect_clis,
            llm::commands::llm_open_agy_login,
            llm::commands::llm_save_endpoint,
            llm::commands::llm_delete_endpoint,
            llm::commands::llm_set_usage,
            llm::commands::llm_test_provider,
            connectors::commands::connectors_policy,
            connectors::commands::connectors_list,
            connectors::commands::connectors_connect,
            connectors::commands::connectors_cancel_connect,
            connectors::commands::connectors_github_cli_detect,
            connectors::commands::connectors_github_cli_connect,
            connectors::commands::connectors_disconnect,
            connectors::commands::connectors_linear_teams,
            connectors::commands::connectors_selected_teams_set,
            obsidian::obsidian_status,
            obsidian::obsidian_configure,
            obsidian::obsidian_disconnect,
            connectors::commands::notion_connector_status,
            connectors::commands::notion_connector_connect,
            connectors::commands::notion_connector_cancel_connect,
            connectors::commands::notion_connector_disconnect,
            connectors::commands::notion_connector_list_tools,
            connectors::commands::routine_trust_get,
            connectors::commands::routine_trust_set,
            connectors::commands::routine_trust_record_run,
            connectors::commands::connector_triggers_list,
            connectors::commands::connector_trigger_set,
            connectors::commands::connector_trigger_delete,
            connectors::approvals::connector_approvals_pending,
            connectors::approvals::connector_approval_respond,
            connectors::approvals::connector_approvals_respond_all,
            computer_use::computer_use_status,
            computer_use::set_computer_use_grant,
            computer_use::computer_use_request_permissions,
            computer_use_permission_drag::set_computer_use_permission_drag_bounds,
            computer_use::computer_use_stop,
            computer_use::computer_use_begin_run,
            computer_use::computer_use_end_run,
            computer_use::computer_use_approvals_pending,
            computer_use::computer_use_approval_respond,
            updates::updates_enabled,
            updates::fetch_update,
            updates::install_update,
            updates::postpone_update_install,
            updates::relaunch_for_update,
            activity::activity_status,
            activity::activity_save_settings,
            activity::activity_set_paused,
            activity::activity_request_permission,
            activity::activity_recreate_database,
            activity::activity_debug_export,
            coding_agents::coding_agents_status,
            coding_agents::coding_agents_blocks,
            activity::timeline::activity_timeline,
            activity::timeline::activity_timeline_session,
            activity::timeline::activity_timeline_search,
            activity::app_icon::activity_app_icon,
            day_intelligence::day_intelligence_day,
            day_intelligence::day_intelligence_generate,
            mcp_server::mcp_server_status,
            mcp_server::mcp_server_set_enabled,
            approval_broker::external::mcp_external_approvals,
            approval_broker::external::mcp_external_approval_decide,
            chat_engine::chat_engine_catalog,
            chat_engine::chat_default_engine,
        ])
        .manage(RecordingPresenceBoundsState::default())
        .manage(note_save_flush::NoteSaveFlushState::default())
        .manage(chat_engine::ChatEngineHost::default())
        .manage(agent_recorder::AgentRecorderBroker::default())
        .manage(Arc::new(browser_broker::BrowserBroker::default()))
        .manage(computer_use::ComputerUseState::default())
        .manage(shutdown::ShutdownCoordinator::default())
        .manage(connectors::ConnectFlow::default())
        .manage(connectors::NotionConnectFlow::default())
        .setup(|app| {
            agent_sessions::skills::seed_bundled_skills(app.handle());
            // Before any CLI turn can ask: cancels requests a previous
            // process left pending.
            approval_broker::setup(app);
            // Before the menus and the activity loops: both read it.
            restore_interface_locale(app);
            setup_app_menu(app)?;
            menu_bar::setup(app)?;
            // After the tray exists: the capture thread publishes its state
            // to the menu bar from its first tick.
            activity::setup(app);
            // Reads the switch from the activity settings registered above.
            mcp_server::setup(app);
            // Shares the activity database and settings.
            coding_agents::setup(app);
            day_intelligence::setup(app);
            setup_interface_locale_listener(app);
            experimental_settings::setup(app)?;
            app_preferences::setup(app)?;
            providers::setup(app);
            #[cfg(target_os = "macos")]
            transcription_model::setup(app)?;
            providers::local_transcriber::setup(app);
            setup_video_asset_scope(app);
            setup_computer_use_asset_scope(app);
            updates::setup(app);
            dictation::setup(app);
            notifications::setup(app);
            meeting_detection::setup(app);
            browser::setup(app.handle()).map_err(|error| std::io::Error::other(error.message))?;
            routines::start_scheduler(app.handle());
            // Poll Google for the events routines subscribe to (email arrivals
            // and upcoming meetings) and wake the matching durable routine.
            connectors::triggers::start(app.handle());
            // Connect GitHub through a signed-in GitHub CLI when there is no
            // GitHub account yet (ADR-0066); off the startup path.
            connectors::start_github_cli_detection(app.handle());
            meeting_hud::setup(app);
            main_window::setup(app);
            Ok(())
        })
        .build(context)
        .expect("failed to build Clovy")
        .run(|app, event| match event {
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                approval_broker::close(app);
                shutdown::handle_exit_requested(app, code, &api);
            }
            // Tao emits only Exit for macOS logout/application termination.
            // This is a bounded synchronous backstop when ExitRequested never
            // had a chance to latch the coordinator.
            tauri::RunEvent::Exit => shutdown::handle_exit(app),
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => {
                main_window::show(app);
            }
            _ => {}
        });
}

/// Registers the generated-video directory in the asset-protocol scope so the
/// inline `<video>` player can load it. Registering the resolved directory
/// matches the real write path in both development and production.
fn setup_video_asset_scope(app: &mut tauri::App) {
    let videos_dir = match crate::app_paths::app_data_dir(app.handle()) {
        Ok(data_dir) => data_dir
            .join("agent-workspaces")
            .join("generated-media")
            .join("videos"),
        Err(error) => {
            tracing::warn!(%error, "video asset scope: could not resolve app data dir");
            return;
        }
    };
    if let Err(error) = app
        .asset_protocol_scope()
        .allow_directory(&videos_dir, false)
    {
        tracing::warn!(%error, path = %videos_dir.display(), "video asset scope: allow_directory failed");
    }
}

fn setup_computer_use_asset_scope(app: &mut tauri::App) {
    let captures_dir = match crate::app_paths::app_data_dir(app.handle()) {
        Ok(data_dir) => data_dir
            .join("agent-workspaces")
            .join("computer-use")
            .join("captures"),
        Err(error) => {
            tracing::warn!(%error, "computer use asset scope: could not resolve app data dir");
            return;
        }
    };
    if let Err(error) = app
        .asset_protocol_scope()
        .allow_directory(&captures_dir, false)
    {
        tracing::warn!(%error, path = %captures_dir.display(), "computer use asset scope: allow_directory failed");
    }
}

#[cfg(desktop)]
fn should_register_single_instance_plugin() -> bool {
    single_instance_enabled_for_build(
        cfg!(debug_assertions),
        std::env::var_os("OS_CLOVY_ENABLE_DEV_SINGLE_INSTANCE").is_some(),
    )
}

#[cfg(desktop)]
fn single_instance_enabled_for_build(debug_assertions: bool, force_dev: bool) -> bool {
    !debug_assertions || force_dev
}

#[cfg(all(test, desktop))]
mod tests {
    use super::{should_emit_close_tab_event, single_instance_enabled_for_build};

    #[test]
    fn single_instance_is_disabled_for_dev_builds_by_default() {
        assert!(!single_instance_enabled_for_build(true, false));
    }

    #[test]
    fn single_instance_can_be_forced_on_for_dev_builds() {
        assert!(single_instance_enabled_for_build(true, true));
    }

    #[test]
    fn single_instance_remains_enabled_for_release_builds() {
        assert!(single_instance_enabled_for_build(false, false));
    }

    #[test]
    fn close_tab_menu_emits_only_when_main_window_focus_is_known_true() {
        assert!(should_emit_close_tab_event(Some(true)));
        assert!(!should_emit_close_tab_event(Some(false)));
        assert!(!should_emit_close_tab_event(None));
    }
}

fn setup_app_menu(app: &tauri::App) -> tauri::Result<()> {
    install_app_menu(app.handle())
}

/// Mirrors the webview's interface language into the native menus. The
/// frontend emits the tag on startup and whenever the user changes it; menus
/// are rebuilt on the main thread only when the language actually changed.
/// The language saved by the last run, before the background loops that
/// write activity text start.
fn restore_interface_locale(app: &tauri::App) {
    if let Ok(dir) = app_paths::app_config_dir(app.handle()) {
        interface_locale::restore(&dir);
    }
}

fn setup_interface_locale_listener(app: &tauri::App) {
    let handle = app.handle().clone();
    app.listen_any(interface_locale::INTERFACE_LOCALE_EVENT, move |event| {
        let locale = interface_locale::UiLocale::from_payload(event.payload());
        let unsaved = app_paths::app_config_dir(&handle)
            .ok()
            .filter(|dir| interface_locale::read_saved(dir) != Some(locale));
        if let Some(dir) = unsaved {
            if let Err(error) = interface_locale::write_saved(&dir, locale) {
                tracing::warn!(%error, "could not save the interface language");
            }
        }
        if !interface_locale::set_current(locale) {
            return;
        }
        let main_thread_handle = handle.clone();
        let _ = handle.run_on_main_thread(move || {
            let _ = install_app_menu(&main_thread_handle);
            menu_bar::relocalize(&main_thread_handle);
        });
    });
    // The main window may have broadcast before this listener existed, and a
    // single request can be lost the same way: ask until the language is
    // known (at once when the saved file already said it).
    let asker = app.handle().clone();
    std::thread::spawn(move || {
        interface_locale::ask_until_known(
            || {
                let _ = asker.emit(interface_locale::INTERFACE_LOCALE_REQUEST_EVENT, ());
            },
            || interface_locale::known().is_some(),
            std::thread::sleep,
        );
    });
}

fn install_app_menu(handle: &tauri::AppHandle) -> tauri::Result<()> {
    use interface_locale::PredefinedLabel as Label;
    use tauri::menu::{
        AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID,
        WINDOW_SUBMENU_ID,
    };

    let text = interface_locale::strings(interface_locale::current());
    let pkg_info = handle.package_info();
    let config = handle.config();
    let about_metadata = AboutMetadata {
        name: Some("Clovy".into()),
        version: Some(pkg_info.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config
            .bundle
            .publisher
            .clone()
            .map(|publisher| vec![publisher]),
        ..Default::default()
    };

    let app_menu = Submenu::with_items(
        handle,
        pkg_info.name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(handle, text.predefined_about(), Some(about_metadata))?,
            &PredefinedMenuItem::separator(handle)?,
            &MenuItem::with_id(
                handle,
                OPEN_SETTINGS_MENU_ID,
                text.settings(),
                true,
                Some("CmdOrCtrl+,"),
            )?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::services(handle, text.predefined(Label::Services))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::hide(handle, text.predefined(Label::Hide))?,
            &PredefinedMenuItem::hide_others(handle, text.predefined(Label::HideOthers))?,
            &PredefinedMenuItem::show_all(handle, text.predefined(Label::ShowAll))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::quit(handle, text.predefined(Label::Quit))?,
        ],
    )?;

    let file_menu = Submenu::with_items(
        handle,
        text.file(),
        true,
        &[
            &MenuItem::with_id(
                handle,
                CLOSE_TAB_MENU_ID,
                text.close_tab(),
                true,
                Some("CmdOrCtrl+W"),
            )?,
            &MenuItem::with_id(
                handle,
                CLOSE_WINDOW_MENU_ID,
                text.close_window(),
                true,
                Some("CmdOrCtrl+Shift+W"),
            )?,
        ],
    )?;
    let edit_menu = Submenu::with_items(
        handle,
        text.edit(),
        true,
        &[
            &PredefinedMenuItem::undo(handle, text.predefined(Label::Undo))?,
            &PredefinedMenuItem::redo(handle, text.predefined(Label::Redo))?,
            &PredefinedMenuItem::separator(handle)?,
            &PredefinedMenuItem::cut(handle, text.predefined(Label::Cut))?,
            &PredefinedMenuItem::copy(handle, text.predefined(Label::Copy))?,
            &PredefinedMenuItem::paste(handle, text.predefined(Label::Paste))?,
            &PredefinedMenuItem::select_all(handle, text.predefined(Label::SelectAll))?,
        ],
    )?;
    let view_menu = Submenu::with_items(
        handle,
        text.view(),
        true,
        &[&PredefinedMenuItem::fullscreen(
            handle,
            text.predefined(Label::Fullscreen),
        )?],
    )?;
    let window_menu = Submenu::with_id_and_items(
        handle,
        WINDOW_SUBMENU_ID,
        text.window(),
        true,
        &[
            &PredefinedMenuItem::minimize(handle, text.predefined(Label::Minimize))?,
            &PredefinedMenuItem::maximize(handle, text.predefined(Label::Maximize))?,
        ],
    )?;
    let help_menu = Submenu::with_id_and_items(handle, HELP_SUBMENU_ID, text.help(), true, &[])?;

    let menu = Menu::with_items(
        handle,
        &[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &window_menu,
            &help_menu,
        ],
    )?;
    handle.set_menu(menu)?;
    macos_menu_icons::install_settings_symbol_on_app_menu(text.settings());
    Ok(())
}

fn close_main_window(app: &tauri::AppHandle) {
    main_window::hide(app);
}

fn emit_close_tab_if_main_window_focused(app: &tauri::AppHandle) {
    if should_emit_close_tab_event(main_window_focus_state(app)) {
        let _ = app.emit_to("main", CLOSE_TAB_EVENT, ());
    }
}

fn main_window_focus_state(app: &tauri::AppHandle) -> Option<bool> {
    app.get_webview_window("main")
        .and_then(|window| window.is_focused().ok())
}

fn should_emit_close_tab_event(main_window_focused: Option<bool>) -> bool {
    main_window_focused == Some(true)
}

#[tauri::command]
fn print_current_webview(window: tauri::WebviewWindow) -> Result<(), String> {
    window.print().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_recording_presence_bounds(
    state: tauri::State<'_, RecordingPresenceBoundsState>,
    request: SetRecordingPresenceBoundsRequest,
) {
    if let Ok(mut current) = state.0.lock() {
        if let Some(bounds) = request.bounds.filter(|bounds| bounds.is_valid()) {
            *current = Some(RegisteredRecordingPresenceBounds {
                owner_id: request.owner_id,
                bounds,
            });
        } else if current
            .as_ref()
            .is_some_and(|registered| registered.owner_id == request.owner_id)
        {
            *current = None;
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn install_main_window_first_mouse_bridge(
    app: &tauri::AppHandle,
    window: &tauri::WebviewWindow,
) {
    use objc2::runtime::AnyObject;
    use objc2::sel;

    let _ = MAIN_WINDOW_APP.get_or_init(|| app.clone());
    let Ok(handle) = window.ns_window() else {
        return;
    };
    if handle.is_null() {
        return;
    }
    unsafe {
        let window = handle as *mut AnyObject;
        if let Ok(mut current_window) = MAIN_WINDOW_NS_WINDOW.lock() {
            *current_window = Some(window as usize);
        }

        if MAIN_WINDOW_SEND_EVENT_ORIGINAL.get().is_some() {
            return;
        }

        let class = objc2::ffi::object_getClass(window);
        if class.is_null() {
            return;
        }
        let class = &*class;
        let Some(method) = class.instance_method(sel!(sendEvent:)) else {
            return;
        };
        let replacement: objc2::runtime::Imp =
            std::mem::transmute(main_window_send_event as MainWindowSendEventImp);
        let original = method.set_implementation(replacement);
        let original: MainWindowSendEventImp = std::mem::transmute(original);
        let _ = MAIN_WINDOW_SEND_EVENT_ORIGINAL.set(original);
    }
}

#[cfg(target_os = "macos")]
extern "C-unwind" fn main_window_send_event(
    this: &objc2::runtime::AnyObject,
    _sel: objc2::runtime::Sel,
    event: *mut objc2::runtime::AnyObject,
) {
    use objc2::msg_send;

    const NS_EVENT_TYPE_LEFT_MOUSE_DOWN: i64 = 1;
    const NS_EVENT_TYPE_LEFT_MOUSE_DRAGGED: i64 = 6;

    unsafe {
        let is_registered_main_window = MAIN_WINDOW_NS_WINDOW
            .lock()
            .ok()
            .and_then(|window| *window)
            .is_some_and(|window| {
                std::ptr::addr_eq(this, window as *const objc2::runtime::AnyObject)
            });
        if is_registered_main_window && !event.is_null() {
            let event_type: i64 = msg_send![event, type];
            if event_type == NS_EVENT_TYPE_LEFT_MOUSE_DOWN
                && main_app_inactive()
                && main_first_mouse_hits_recording_presence(this, event)
            {
                emit_recording_presence_reopen();
            }
            if event_type == NS_EVENT_TYPE_LEFT_MOUSE_DRAGGED
                && MAIN_WINDOW_APP
                    .get()
                    .is_some_and(|app| computer_use::begin_permission_drag(app, this, event))
            {
                return;
            }
        }

        if let Some(original) = MAIN_WINDOW_SEND_EVENT_ORIGINAL.get().copied() {
            original(this, _sel, event);
        }
    }
}

#[cfg(target_os = "macos")]
fn main_app_inactive() -> bool {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};

    let Some(app_class) = AnyClass::get(c"NSApplication") else {
        return false;
    };
    unsafe {
        let app: *mut AnyObject = msg_send![app_class, sharedApplication];
        if app.is_null() {
            return false;
        }
        let active: bool = msg_send![app, isActive];
        !active
    }
}

#[cfg(target_os = "macos")]
unsafe fn main_first_mouse_hits_recording_presence(
    window: &objc2::runtime::AnyObject,
    event: *mut objc2::runtime::AnyObject,
) -> bool {
    use objc2::msg_send;
    use objc2_foundation::{NSPoint, NSRect};

    let Some(app) = MAIN_WINDOW_APP.get() else {
        return false;
    };
    let Some(registered) = app
        .try_state::<RecordingPresenceBoundsState>()
        .and_then(|state| state.0.lock().ok().and_then(|bounds| bounds.clone()))
    else {
        return false;
    };

    let content: *mut objc2::runtime::AnyObject = msg_send![window, contentView];
    if content.is_null() {
        return false;
    }
    let frame: NSRect = msg_send![content, frame];
    let point: NSPoint = msg_send![event, locationInWindow];
    let x = point.x;
    let y = frame.size.height - point.y;
    registered.bounds.contains(x, y)
}

#[cfg(target_os = "macos")]
fn emit_recording_presence_reopen() {
    let Some(app) = MAIN_WINDOW_APP.get() else {
        return;
    };
    let note_id = audio::capture::current_status().and_then(|status| status.note_id);
    let _ = app.emit_to(
        "main",
        "meeting-hud-action",
        serde_json::json!({ "action": "reopen", "noteId": note_id }),
    );
}
