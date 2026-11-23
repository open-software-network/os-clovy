//! The one path that shows Clovy's main window, and what closing it does.
//!
//! Closing the main window hides it on every system; the app keeps running
//! in the tray (Windows) or menu bar (macOS). Everything that brings the
//! window back (tray icon and "Open Clovy", a second launch, the Dock,
//! notifications, HUDs, menus) calls [`show`], which rebuilds the window
//! from the app configuration if it no longer exists. On macOS the Dock
//! presence follows the preference while the window is hidden.

use tauri::{AppHandle, Manager, WebviewWindow};

pub const MAIN_WINDOW_LABEL: &str = "main";

/// What closing the main window does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseAction {
    /// Hide it; the app keeps running in the tray / menu bar.
    Hide,
    /// Let the window be destroyed.
    Close,
}

/// What a close request on the main window does: hide, on every system.
pub fn close_action() -> CloseAction {
    CloseAction::Hide
}

#[path = "dock_policy.rs"]
mod dock_policy;
pub use dock_policy::{desired_policy, DockPolicy};

#[cfg(target_os = "macos")]
fn apply_policy(app: &AppHandle, visible: bool) {
    let policy = match desired_policy(crate::app_preferences::hide_from_dock(app), visible) {
        DockPolicy::Regular => tauri::ActivationPolicy::Regular,
        DockPolicy::Accessory => tauri::ActivationPolicy::Accessory,
    };
    if let Err(error) = app.set_activation_policy(policy) {
        tracing::warn!(%error, "could not update Dock presence");
    }
}

#[cfg(not(target_os = "macos"))]
fn apply_policy(_app: &AppHandle, _visible: bool) {}

pub fn hide(app: &AppHandle) {
    if app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .is_some_and(|window| window.hide().is_ok())
    {
        apply_policy(app, false);
    }
}

/// Where the main window lives; tests substitute it.
pub trait MainWindowSlot {
    type Window;
    fn existing(&self) -> Option<Self::Window>;
    /// Builds the window again from the app configuration.
    fn create(&self) -> Option<Self::Window>;
    /// Shows, restores, and focuses it.
    fn reveal(&self, window: &Self::Window);
}

/// Shows the main window, rebuilding it when it no longer exists. Returns
/// whether a window was revealed.
pub fn show_with<S: MainWindowSlot>(slot: &S) -> bool {
    match slot.existing().or_else(|| slot.create()) {
        Some(window) => {
            slot.reveal(&window);
            true
        }
        None => false,
    }
}

/// Shows, restores, and focuses the main window, rebuilding it if needed.
pub fn show(app: &AppHandle) -> bool {
    show_with(&AppSlot(app))
}

/// Registers the lifecycle of the main window built from the configuration
/// at startup.
pub fn setup(app: &tauri::App) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        register_lifecycle(app.handle(), &window);
    }
}

/// Close hides (every system); on macOS also the first-click bridge and the
/// Computer use driver prewarm on focus.
pub fn register_lifecycle(app: &AppHandle, window: &WebviewWindow) {
    #[cfg(target_os = "macos")]
    crate::install_main_window_first_mouse_bridge(app, window);
    let lifecycle_app = app.clone();
    window.on_window_event(move |event| match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            if close_action() == CloseAction::Hide {
                api.prevent_close();
                hide(&lifecycle_app);
            }
        }
        #[cfg(target_os = "macos")]
        tauri::WindowEvent::Focused(true) => {
            crate::computer_use::schedule_driver_prewarm(&lifecycle_app);
        }
        _ => {}
    });
}

struct AppSlot<'a>(&'a AppHandle);

impl MainWindowSlot for AppSlot<'_> {
    type Window = WebviewWindow;

    fn existing(&self) -> Option<WebviewWindow> {
        self.0.get_webview_window(MAIN_WINDOW_LABEL)
    }

    fn create(&self) -> Option<WebviewWindow> {
        let config = self
            .0
            .config()
            .app
            .windows
            .iter()
            .find(|window| window.label == MAIN_WINDOW_LABEL)?;
        let window = tauri::WebviewWindowBuilder::from_config(self.0, config)
            .and_then(|builder| builder.build())
            .map_err(|error| tracing::warn!(%error, "could not rebuild the main window"))
            .ok()?;
        register_lifecycle(self.0, &window);
        Some(window)
    }

    fn reveal(&self, window: &WebviewWindow) {
        apply_policy(self.0, true);
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeSlot {
        window: RefCell<Option<u32>>,
        created: RefCell<u32>,
        revealed: RefCell<Vec<u32>>,
    }

    impl MainWindowSlot for FakeSlot {
        type Window = u32;
        fn existing(&self) -> Option<u32> {
            *self.window.borrow()
        }
        fn create(&self) -> Option<u32> {
            *self.created.borrow_mut() += 1;
            *self.window.borrow_mut() = Some(7);
            Some(7)
        }
        fn reveal(&self, window: &u32) {
            self.revealed.borrow_mut().push(*window);
        }
    }

    #[test]
    fn an_existing_window_is_revealed_not_rebuilt() {
        let slot = FakeSlot {
            window: RefCell::new(Some(3)),
            ..FakeSlot::default()
        };
        assert!(show_with(&slot));
        assert_eq!(*slot.created.borrow(), 0);
        assert_eq!(*slot.revealed.borrow(), [3]);
    }

    /// The tray, "Open Clovy", a second launch, notifications, and HUDs all
    /// end here; a window that no longer exists comes back.
    #[test]
    fn a_destroyed_window_is_rebuilt_and_revealed() {
        let slot = FakeSlot::default();
        assert!(show_with(&slot));
        assert_eq!(*slot.created.borrow(), 1);
        assert_eq!(*slot.revealed.borrow(), [7]);
    }

    #[test]
    fn closing_hides_the_window_on_every_system() {
        assert_eq!(close_action(), CloseAction::Hide);
    }

    /// Every caller goes through `main_window::show`; no module keeps its own
    /// copy that silently does nothing when the window is gone.
    #[test]
    fn no_other_module_shows_the_main_window_on_its_own() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|ext| ext == "rs")
                    && !path.ends_with("main_window.rs")
                {
                    let source = std::fs::read_to_string(&path).unwrap();
                    if source.contains("fn show_main_window(") {
                        offenders.push(path.strip_prefix(&root).unwrap().display().to_string());
                    }
                }
            }
        }
        offenders.sort();
        assert!(offenders.is_empty(), "use main_window::show: {offenders:?}");
    }
}
