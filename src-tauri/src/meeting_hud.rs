//! Floating meeting-recording HUD.
//!
//! A small always-on-top pill — sibling to the dictation HUD — that surfaces a
//! "still recording" signal (Clovy mark + live waveform) whenever the main
//! window is closed or minimized mid-recording. It's a presence indicator, not
//! a control surface: clicking it reopens the app on the meeting being recorded
//! (see [`meeting_hud_reopen`]); all recording controls stay in-app.
//!
//! Unlike the dictation HUD (driven by helper-process events), note recording
//! is owned by the native capture layer. One background supervisor samples that
//! layer, emits the shared narrow telemetry stream, and drives HUD visibility.
//! The main renderer and HUD subscribe to the same event instead of polling
//! capture independently.
//!
//! The window normally stays square around the pill in either orientation. The
//! end-of-meeting grace period temporarily grows it into a card. The frosted
//! surface is a real NSVisualEffectView sized to the visible pill or card (CSS
//! `backdrop-filter` can't sample other apps' pixels), and depth comes from the
//! native NSWindow shadow.
//!
//! The pill is orientation-aware: parked in the left or right third of the
//! screen it stands upright (mark above the waveform); in the middle third it
//! lies horizontal. The supervisor watches the drag position and, once the
//! window settles, turns the *native contentView layer* a quarter turn — frost,
//! tint, mark, and waveform rotate as one unit under a single Core Animation
//! ease, so nothing can clip or drift out of sync mid-turn. The one exception:
//! the webview counter-rotates the bars (CSS, same duration/curve) so the
//! waveform itself still reads left-to-right when the pill stands upright.

use crate::audio::capture;
use crate::domain::types::{
    AudioLevelDto, RecordingState, RecordingStatusDto, RecordingTelemetryDto,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::Duration,
};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow,
    WindowEvent,
};

#[cfg(target_os = "macos")]
use objc2::runtime::{AnyClass, AnyObject};

const WINDOW_LABEL: &str = "meeting-hud";
pub const RECORDING_TELEMETRY_EVENT: &str = "recording-telemetry";

/// One native sample feeds both renderer surfaces. Twenty hertz matches the
/// waveform's peak window and ballistics while remaining below half the old
/// combined 45 Hz status-read rate.
const ACTIVE_TICK: Duration = Duration::from_millis(50);
const IDLE_TICK: Duration = Duration::from_millis(220);

/// Logical pill size — must agree with `.mhud` in meeting-hud.css.
const PILL_SIZE: LogicalSize<f64> = LogicalSize::new(76.0, 32.0);
/// Upright the pill carries less (4 bars instead of 7), so it runs shorter.
/// Must agree with `.mhud[data-orient="vertical"]` in meeting-hud.css.
const VERTICAL_PILL_LENGTH: f64 = 62.0;
/// The window is a fixed square the length of the pill (tauri.conf.json), so a
/// quarter turn always fits without resizing the native frame — the transparent
/// gutters above/below (or beside) the pill are part of the window.
const WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(76.0, 76.0);
/// The end-of-meeting card replaces the compact presence pill during its
/// countdown. The window grows around a 380x48 CSS card with a small transparent
/// gutter for the native shadow.
const END_PROMPT_WINDOW_SIZE: LogicalSize<f64> = LogicalSize::new(400.0, 68.0);

/// How long the quarter turn takes. The easing matches the app's `--ease-out`
/// token (cubic-bezier(0.22, 1, 0.36, 1)) so the HUD moves like the rest of
/// the UI even though this animation runs in Core Animation, not CSS.
const TURN_SECS: f64 = 0.32;

/// How many supervisor ticks the window must hold still before an orientation
/// flip is applied — resizing mid-native-drag is what we're avoiding.
const SETTLE_TICKS: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Left,
    Center,
    Right,
}

impl Zone {
    fn is_vertical(self) -> bool {
        matches!(self, Zone::Left | Zone::Right)
    }
}

pub struct MeetingHudPosition {
    /// Drag position remembered between appearances in the same process. Like
    /// the dictation HUD this is intentionally not persisted to disk — every
    /// launch resets the pill to its default anchor so it can never get
    /// stranded on a monitor that's no longer connected.
    inner: Mutex<Option<(i32, i32)>>,
}

pub struct MeetingHudState {
    /// Freshest status, so a HUD webview that loads mid-recording can paint
    /// immediately instead of waiting for the next pump.
    latest_status: Mutex<Option<RecordingStatusDto>>,
    /// Which screen third the pill currently occupies; the webview mirrors this
    /// as its layout orientation.
    zone: Mutex<Zone>,
    /// The webview expands into an interactive end-of-meeting card while the
    /// native detector's countdown is active.
    end_prompt_expanded: Mutex<bool>,
}

pub fn setup(app: &mut tauri::App) {
    app.manage(MeetingHudPosition {
        inner: Mutex::new(None),
    });
    app.manage(MeetingHudState {
        latest_status: Mutex::new(None),
        zone: Mutex::new(Zone::Center),
        end_prompt_expanded: Mutex::new(false),
    });
    if let Err(error) = configure_window(app.handle()) {
        tracing::warn!(%error, "failed to configure meeting HUD");
    }
    spawn_supervisor(app.handle().clone());
}

#[tauri::command]
pub fn meeting_hud_latest_status(state: State<'_, MeetingHudState>) -> Option<RecordingStatusDto> {
    state.latest_status.lock().ok().and_then(|g| g.clone())
}

/// Clicking the HUD reopens the app on the meeting being recorded. Activation is
/// done here in Rust because clicking a non-activating panel can't pull a
/// backgrounded app forward on its own; React then hears the emitted action and
/// navigates to the recording note.
#[tauri::command]
pub fn meeting_hud_reopen(app: AppHandle) {
    let note_id = capture::current_status().and_then(|status| status.note_id);
    if crate::main_window::show(&app) {
        let _ = app.emit_to(
            "main",
            "meeting-hud-action",
            serde_json::json!({ "action": "reopen", "noteId": note_id }),
        );
        return;
    }
    let _ = app.emit(
        "meeting-hud-action",
        serde_json::json!({ "action": "reopen", "noteId": note_id }),
    );
}

#[tauri::command]
pub fn meeting_hud_set_end_prompt_expanded(
    app: AppHandle,
    state: State<'_, MeetingHudState>,
    expanded: bool,
) -> Result<(), String> {
    let mut guard = state
        .end_prompt_expanded
        .lock()
        .map_err(|_| "meeting HUD state is unavailable".to_string())?;
    if *guard == expanded {
        return Ok(());
    }
    *guard = expanded;
    drop(guard);

    let Some(hud) = app.get_webview_window(WINDOW_LABEL) else {
        return Ok(());
    };
    if expanded {
        set_orientation(&hud, false, false);
        // The stored zone must track the forced-horizontal card so that
        // `apply_zone_now` re-derives the side zone on collapse and re-applies
        // its quarter turn (Center is the horizontal zone).
        if let Ok(mut guard) = state.zone.lock() {
            *guard = Zone::Center;
        }
        hud.set_size(END_PROMPT_WINDOW_SIZE)
            .map_err(|error| error.to_string())?;
        set_end_prompt_surface(&hud, true);
        position_end_prompt(&hud);
    } else {
        hud.set_size(WINDOW_SIZE)
            .map_err(|error| error.to_string())?;
        set_end_prompt_surface(&hud, false);
        position_window(&app, &hud);
        apply_zone_now(&hud, &state);
    }
    Ok(())
}

fn is_live(state: RecordingState) -> bool {
    matches!(
        state,
        RecordingState::Recording | RecordingState::Paused | RecordingState::Starting
    )
}

/// The HUD should take over when the main window is no longer a visible
/// recording surface: it has been hidden (close button -> hide), minimized, or
/// Clovy is no longer the active app because another app took focus.
fn main_window_dismissed(app: &AppHandle) -> bool {
    let Some(main) = app.get_webview_window("main") else {
        return false;
    };
    let hidden = !main.is_visible().unwrap_or(true);
    let minimized = main.is_minimized().unwrap_or(false);
    hidden || minimized || app_inactive()
}

#[cfg(target_os = "macos")]
fn app_inactive() -> bool {
    use objc2::msg_send;

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

/// Windows has no app-level activation: Clovy counts as active only while one
/// of its own windows is the foreground window. The HUD is non-focusable
/// (`WS_EX_NOACTIVATE`), so clicking it never makes Clovy active and hides it.
#[cfg(target_os = "windows")]
fn app_inactive() -> bool {
    clovy_inactive_for_foreground(
        crate::activity::windows::window::foreground_window_pid(),
        std::process::id(),
    )
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn app_inactive() -> bool {
    false
}

/// Clovy is inactive on Windows unless the foreground window belongs to this
/// process (the main window or the HUD). No foreground window (lock screen,
/// focus in transition) also counts as inactive, so the countdown shows.
#[cfg(any(target_os = "windows", test))]
fn clovy_inactive_for_foreground(foreground_pid: Option<u32>, own_pid: u32) -> bool {
    foreground_pid != Some(own_pid)
}

/// Per-thread drag-settle tracker for the orientation flip.
struct ZoneTracker {
    last_position: Option<PhysicalPosition<i32>>,
    stable_ticks: u32,
    /// Settled while the button was still held — flip on release instead.
    pending_release: bool,
    /// Last emitted telemetry. Paused recordings stop advancing, so equality
    /// suppresses redundant events until state or signal changes again.
    last_telemetry: Option<RecordingTelemetryDto>,
    /// Position + zone were applied last tick; show on this one. The native
    /// rotation is queued on the main thread and lands before show either
    /// way, but the `meeting-hud-zone` event rides WKWebView's async message
    /// queue — without this beat, the bars' CSS counter-rotation could miss
    /// the first visible frame and paint sideways for an instant.
    show_armed: bool,
}

/// Edge-tracks the menu-bar recording dot so the supervisor's 20 Hz poll only
/// notifies the tray when capture actually starts or stops, not every tick.
static RECORDING_INDICATOR_ACTIVE: AtomicBool = AtomicBool::new(false);

fn sync_recording_indicator(app: &AppHandle, active: bool) {
    if RECORDING_INDICATOR_ACTIVE.swap(active, Ordering::SeqCst) == active {
        return;
    }
    crate::menu_bar::set_meeting_recording_active(app, active);
}

fn spawn_supervisor(app: AppHandle) {
    thread::spawn(move || {
        let mut tracker = ZoneTracker {
            last_position: None,
            stable_ticks: 0,
            pending_release: false,
            last_telemetry: None,
            show_armed: false,
        };
        loop {
            let tick = supervise(&app, &mut tracker);
            thread::sleep(tick);
        }
    });
}

fn supervise(app: &AppHandle, tracker: &mut ZoneTracker) -> Duration {
    let status = capture::current_status();

    // Drive the native menu-bar recording dot from the same poll. It lights only
    // while capture is actively running — a paused take stops capturing, so the
    // dot goes dark until it resumes (matching what a recording dot implies).
    // Independent of the HUD window and of dictation, both of which can be
    // active at the same time.
    let recording_active = status
        .as_ref()
        .map(|status| matches!(status.state, RecordingState::Recording))
        .unwrap_or(false);
    sync_recording_indicator(app, recording_active);

    let Some(state) = app.try_state::<MeetingHudState>() else {
        return IDLE_TICK;
    };

    let live = status.filter(|status| is_live(status.state));

    let Some(status) = live else {
        // Tell renderer subscribers once that the native session disappeared.
        // The session id makes the clear safe if a newer recording already won
        // the UI race.
        if let Some(previous) = tracker.last_telemetry.take() {
            let ended = RecordingTelemetryDto {
                session_id: previous.session_id,
                state: RecordingState::Idle,
                elapsed_ms: previous.elapsed_ms,
                level: AudioLevelDto::default(),
                silence_warning: false,
                sources: Vec::new(),
                warnings: Vec::new(),
            };
            let _ = app.emit(RECORDING_TELEMETRY_EVENT, ended);
        }
        if let Ok(mut guard) = state.latest_status.lock() {
            *guard = None;
        }
        // Nothing recording: make sure the pill is gone and forget the snapshot.
        if let Some(hud) = app.get_webview_window(WINDOW_LABEL) {
            if hud.is_visible().unwrap_or(false) {
                let _ = hud.hide();
            }
        }
        tracker.last_position = None;
        tracker.show_armed = false;
        return IDLE_TICK;
    };

    let telemetry = RecordingTelemetryDto::from(&status);
    if tracker.last_telemetry.as_ref() != Some(&telemetry) {
        let _ = app.emit(RECORDING_TELEMETRY_EVENT, &telemetry);
        tracker.last_telemetry = Some(telemetry);
    }

    if let Ok(mut guard) = state.latest_status.lock() {
        *guard = Some(status.clone());
    }

    let Some(hud) = app.get_webview_window(WINDOW_LABEL) else {
        return ACTIVE_TICK;
    };

    let end_prompt_expanded = state
        .end_prompt_expanded
        .lock()
        .map(|guard| *guard)
        .unwrap_or(false);
    // The end card follows the pill's visibility rule: only while Clovy is not
    // active. When the main window is frontmost the record dock's inline
    // notice owns the countdown; the floating card would be a duplicate.
    let should_show = main_window_dismissed(app);
    let visible = hud.is_visible().unwrap_or(false);
    if should_show && !visible {
        if end_prompt_expanded {
            tracker.show_armed = false;
            position_end_prompt(&hud);
            let _ = hud.show();
        } else if tracker.show_armed {
            // Position + zone went out last tick; the webview has had a full
            // tick to apply data-orient before the first visible frame.
            tracker.show_armed = false;
            let _ = hud.show();
        } else {
            position_window(app, &hud);
            // Apply the zone for wherever the pill landed before it appears,
            // so it never flashes the wrong orientation.
            apply_zone_now(&hud, &state);
            tracker.show_armed = true;
        }
    } else if !should_show {
        tracker.show_armed = false;
        if visible {
            let _ = hud.hide();
        }
    }

    if hud.is_visible().unwrap_or(false) && !end_prompt_expanded {
        track_zone(&hud, &state, tracker);
    }
    ACTIVE_TICK
}

/// Which third of the monitor's work area the pill center sits in.
fn zone_for(hud: &WebviewWindow) -> Option<Zone> {
    let position = hud.outer_position().ok()?;
    let size = hud.outer_size().ok()?;
    let center_x = position.x + size.width as i32 / 2;
    let center_y = position.y + size.height as i32 / 2;
    let monitor = hud
        .monitor_from_point(center_x as f64, center_y as f64)
        .ok()
        .flatten()
        .or_else(|| hud.current_monitor().ok().flatten())?;
    let work = monitor.work_area();
    let third = work.size.width as i32 / 3;
    let offset = center_x - work.position.x;
    Some(if offset < third {
        Zone::Left
    } else if offset > 2 * third {
        Zone::Right
    } else {
        Zone::Center
    })
}

/// Watch the drag position; once the window has held still for a few ticks
/// AND the mouse button is up, flip orientation if it crossed into a different
/// third. Both guards exist so we never resize mid-drag — the resize would
/// shift the pill out from under the cursor's grab point.
fn track_zone(hud: &WebviewWindow, state: &MeetingHudState, tracker: &mut ZoneTracker) {
    let Ok(position) = hud.outer_position() else {
        return;
    };
    if tracker.last_position == Some(position) {
        tracker.stable_ticks = tracker.stable_ticks.saturating_add(1);
    } else {
        tracker.last_position = Some(position);
        tracker.stable_ticks = 0;
        return;
    }
    if tracker.stable_ticks >= SETTLE_TICKS && !left_mouse_button_down() {
        // Apply once per rest: bump past the threshold so we don't re-run
        // zone math every tick while parked.
        if tracker.stable_ticks == SETTLE_TICKS || tracker.pending_release {
            tracker.pending_release = false;
            apply_zone_now(hud, state);
        }
    } else if tracker.stable_ticks >= SETTLE_TICKS {
        // Settled but still held — remember to flip on release.
        tracker.pending_release = true;
    }
}

/// Whether the left mouse button is currently held, via the window server's
/// combined session state — works regardless of which window has the cursor.
#[cfg(target_os = "macos")]
fn left_mouse_button_down() -> bool {
    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        // CGEventSourceButtonState(kCGEventSourceStateCombinedSessionState = 0,
        //                          kCGMouseButtonLeft = 0)
        fn CGEventSourceButtonState(state_id: i32, button: u32) -> bool;
    }
    unsafe { CGEventSourceButtonState(0, 0) }
}

#[cfg(not(target_os = "macos"))]
fn left_mouse_button_down() -> bool {
    false
}

/// Recompute the zone and, if it changed, turn the pill to the new orientation.
/// The whole turn is one Core Animation transform on the native contentView —
/// frost, tint, mark, and waveform rotate together, so nothing clips against the
/// frame or drifts out of sync (the window itself never resizes). The webview
/// hears `meeting-hud-zone` at the same moment and counter-rotates the bars so
/// the waveform keeps reading left-to-right (meeting-hud.css).
fn apply_zone_now(hud: &WebviewWindow, state: &MeetingHudState) {
    let Some(zone) = zone_for(hud) else {
        return;
    };
    let previous = state
        .zone
        .lock()
        .map(|mut guard| std::mem::replace(&mut *guard, zone))
        .unwrap_or(Zone::Center);
    if previous == zone || previous.is_vertical() == zone.is_vertical() {
        return;
    }
    let animate = hud.is_visible().unwrap_or(false);
    let _ = hud.emit_to(
        WINDOW_LABEL,
        "meeting-hud-zone",
        serde_json::json!({ "vertical": zone.is_vertical(), "animate": animate }),
    );
    set_orientation(hud, zone.is_vertical(), animate);
}

/// Rotate the window's contentView (webview + frost) a quarter turn via Core
/// Animation. Runs the AppKit calls on the main thread; the native window
/// shadow is recomputed once the turn lands.
#[cfg(target_os = "macos")]
fn set_orientation(hud: &WebviewWindow, vertical: bool, animate: bool) {
    let window = hud.clone();
    let _ = hud.run_on_main_thread(move || unsafe {
        rotate_content(&window, vertical, animate);
    });
    if animate {
        // The shadow shape is derived from the rendered content; refresh it
        // after the turn settles, off-thread so the status pump never stalls.
        let window = hud.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs_f64(TURN_SECS + 0.06));
            let handle = window.clone();
            let _ = window.run_on_main_thread(move || unsafe {
                invalidate_shadow(&handle);
            });
        });
    }
}

#[cfg(not(target_os = "macos"))]
fn set_orientation(_hud: &WebviewWindow, _vertical: bool, _animate: bool) {}

#[cfg(target_os = "macos")]
fn set_end_prompt_surface(hud: &WebviewWindow, expanded: bool) {
    let window = hud.clone();
    let _ = hud.run_on_main_thread(move || unsafe {
        resize_frost_for_end_prompt(&window, expanded);
        invalidate_shadow(&window);
    });
}

#[cfg(not(target_os = "macos"))]
fn set_end_prompt_surface(_hud: &WebviewWindow, _expanded: bool) {}

fn configure_window(app: &AppHandle) -> Result<(), String> {
    if let Some(hud) = app.get_webview_window(WINDOW_LABEL) {
        hud.set_always_on_top(true)
            .map_err(|error| error.to_string())?;
        hud.set_visible_on_all_workspaces(true)
            .map_err(|error| error.to_string())?;
        hud.set_focusable(false)
            .map_err(|error| error.to_string())?;
        hud.set_skip_taskbar(true)
            .map_err(|error| error.to_string())?;
        // The window is exactly the pill, so depth comes from the native
        // window shadow instead of a CSS one painted into a gutter.
        hud.set_shadow(true).map_err(|error| error.to_string())?;

        #[cfg(target_os = "macos")]
        {
            make_nonactivating(&hud);
            // Real behind-window blur, sized to the pill (not the window — the
            // square window's gutters stay fully transparent). Done by hand
            // rather than via window_vibrancy, which always fills the window.
            unsafe { install_frost(&hud) };
        }

        let app_for_events = app.clone();
        hud.on_window_event(move |event| {
            if let WindowEvent::Moved(position) = event {
                let expanded = app_for_events
                    .try_state::<MeetingHudState>()
                    .and_then(|state| state.end_prompt_expanded.lock().ok().map(|guard| *guard))
                    .unwrap_or(false);
                if expanded {
                    return;
                }
                if let Some(state) = app_for_events.try_state::<MeetingHudPosition>() {
                    if let Ok(mut guard) = state.inner.lock() {
                        *guard = Some((position.x, position.y));
                    }
                }
            }
        });
    }
    Ok(())
}

fn position_window(app: &AppHandle, hud: &WebviewWindow) {
    let Ok(window_size) = hud.outer_size() else {
        return;
    };

    if let Some(state) = app.try_state::<MeetingHudPosition>() {
        let saved = state.inner.lock().ok().and_then(|guard| *guard);
        if let Some((x, y)) = saved {
            if position_is_visible(hud, x, y, window_size) {
                let _ = hud.set_position(PhysicalPosition::new(x, y));
                return;
            }
        }
    }

    if let Some((x, y)) = default_position(hud, window_size) {
        let _ = hud.set_position(PhysicalPosition::new(x, y));
    }
}

/// Top-center of the work area, notification-style — the card only appears
/// while Clovy is inactive, so it should land like a transient alert. Centered,
/// not top-right: the agent HUD's default anchor is the top-right corner
/// (agent_hud.rs), and the card must never cover a running session HUD.
fn position_end_prompt(hud: &WebviewWindow) {
    const MARGIN: i32 = 16;
    let scale = hud.scale_factor().unwrap_or(1.0);
    let width = (END_PROMPT_WINDOW_SIZE.width * scale).round() as i32;
    let monitor = hud
        .cursor_position()
        .ok()
        .and_then(|cursor| hud.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| hud.current_monitor().ok().flatten())
        .or_else(|| hud.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else {
        return;
    };
    let work = monitor.work_area();
    let x = work.position.x + (work.size.width as i32 - width) / 2;
    let y = work.position.y + MARGIN;
    let _ = hud.set_position(PhysicalPosition::new(x, y));
}

fn default_position(hud: &WebviewWindow, window_size: PhysicalSize<u32>) -> Option<(i32, i32)> {
    const BOTTOM_MARGIN: i32 = 16;

    let monitor = hud
        .cursor_position()
        .ok()
        .and_then(|cursor| hud.monitor_from_point(cursor.x, cursor.y).ok().flatten())
        .or_else(|| hud.current_monitor().ok().flatten())
        .or_else(|| hud.primary_monitor().ok().flatten())?;

    let work_area = monitor.work_area();
    let work_bottom = work_area.position.y + work_area.size.height as i32;
    let work_center_x = work_area.position.x + work_area.size.width as i32 / 2;

    // The visible pill sits centered in the square window; discount the
    // transparent gutter below it so the margin is measured from the pill.
    let scale = hud.scale_factor().unwrap_or(1.0);
    let gutter = ((WINDOW_SIZE.height - PILL_SIZE.height) / 2.0 * scale).round() as i32;

    let x = work_center_x - window_size.width as i32 / 2;
    let y = work_bottom - BOTTOM_MARGIN - window_size.height as i32 + gutter;
    Some((x, y))
}

fn position_is_visible(
    hud: &WebviewWindow,
    x: i32,
    y: i32,
    window_size: PhysicalSize<u32>,
) -> bool {
    const MIN_OVERLAP_PX: i32 = 24;
    let Ok(monitors) = hud.available_monitors() else {
        return false;
    };
    let pill_w = window_size.width as i32;
    let pill_h = window_size.height as i32;
    monitors.iter().any(|monitor| {
        let work = monitor.work_area();
        let mx = work.position.x;
        let my = work.position.y;
        let mw = work.size.width as i32;
        let mh = work.size.height as i32;
        let overlap_x = (x + pill_w).min(mx + mw) - x.max(mx);
        let overlap_y = (y + pill_h).min(my + mh) - y.max(my);
        overlap_x >= MIN_OVERLAP_PX && overlap_y >= MIN_OVERLAP_PX
    })
}

/// The NSWindow's contentView as a raw object, or null.
#[cfg(target_os = "macos")]
unsafe fn content_view(hud: &WebviewWindow) -> (*mut AnyObject, *mut AnyObject) {
    use objc2::msg_send;

    let Ok(handle) = hud.ns_window() else {
        return (std::ptr::null_mut(), std::ptr::null_mut());
    };
    if handle.is_null() {
        return (std::ptr::null_mut(), std::ptr::null_mut());
    }
    let window = handle as *mut AnyObject;
    let content: *mut AnyObject = msg_send![window, contentView];
    (window, content)
}

/// macOS: add an NSVisualEffectView behind the webview, framed to the pill
/// rather than the window, so the square window's gutters stay fully
/// transparent. Hand-rolled instead of `window_vibrancy::apply_vibrancy`
/// because that always fills the window.
#[cfg(target_os = "macos")]
unsafe fn install_frost(hud: &WebviewWindow) {
    use objc2::msg_send;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    /// Must match the pill's CSS border-radius (--r-lg, 10px) so tint and
    /// frost trace the same curve.
    const FROST_RADIUS: f64 = 10.0;

    let (_, content) = content_view(hud);
    if content.is_null() {
        return;
    }
    // The contentView hosts the orientation turn (`rotate_content`); give it a
    // layer up front so the first turn doesn't switch rendering paths mid-use.
    let _: () = msg_send![content, setWantsLayer: true];

    let Some(effect_class) = AnyClass::get(c"NSVisualEffectView") else {
        return;
    };
    let frame = NSRect::new(
        NSPoint::new(
            (WINDOW_SIZE.width - PILL_SIZE.width) / 2.0,
            (WINDOW_SIZE.height - PILL_SIZE.height) / 2.0,
        ),
        NSSize::new(PILL_SIZE.width, PILL_SIZE.height),
    );
    let frost: *mut AnyObject = msg_send![effect_class, alloc];
    let frost: *mut AnyObject = msg_send![frost, initWithFrame: frame];
    if frost.is_null() {
        return;
    }
    // HUDWindow material, behind-window blending, always active — the same
    // look the dictation HUD gets from window_vibrancy.
    let _: () = msg_send![frost, setMaterial: 13isize];
    let _: () = msg_send![frost, setBlendingMode: 0isize];
    let _: () = msg_send![frost, setState: 1isize];
    let _: () = msg_send![frost, setWantsLayer: true];
    let layer: *mut AnyObject = msg_send![frost, layer];
    if !layer.is_null() {
        let _: () = msg_send![layer, setCornerRadius: FROST_RADIUS];
        let _: () = msg_send![layer, setMasksToBounds: true];
    }
    // Below the webview (NSWindowBelow = -1) so the CSS tint paints over it.
    let _: () = msg_send![content, addSubview: frost, positioned: -1isize, relativeTo: std::ptr::null_mut::<AnyObject>()];
}

/// The `--ease-out` token from tokens.css as a CAMediaTimingFunction.
/// `functionWithControlPoints::::` has bare colons `msg_send!` can't spell,
/// hence `send_message`.
#[cfg(target_os = "macos")]
unsafe fn ease_out_timing() -> *mut AnyObject {
    use objc2::runtime::MessageReceiver;
    use objc2::sel;

    let Some(timing_class) = AnyClass::get(c"CAMediaTimingFunction") else {
        return std::ptr::null_mut();
    };
    (timing_class as *const AnyClass as *mut AnyObject).send_message(
        sel!(functionWithControlPoints::::),
        (0.22f32, 1.0f32, 0.36f32, 1.0f32),
    )
}

/// The frost NSVisualEffectView installed by `install_frost`, or null.
#[cfg(target_os = "macos")]
unsafe fn frost_view(content: *mut AnyObject) -> *mut AnyObject {
    use objc2::msg_send;

    let Some(effect_class) = AnyClass::get(c"NSVisualEffectView") else {
        return std::ptr::null_mut();
    };
    let subviews: *mut AnyObject = msg_send![content, subviews];
    if subviews.is_null() {
        return std::ptr::null_mut();
    }
    let count: usize = msg_send![subviews, count];
    for index in 0..count {
        let view: *mut AnyObject = msg_send![subviews, objectAtIndex: index];
        let is_frost: bool = msg_send![view, isKindOfClass: effect_class];
        if is_frost {
            return view;
        }
    }
    std::ptr::null_mut()
}

/// Resize the native vibrancy surface to the same visible bounds as the CSS
/// pill or end prompt. The webview window itself includes a transparent gutter
/// for the shadow in both modes.
#[cfg(target_os = "macos")]
unsafe fn resize_frost_for_end_prompt(hud: &WebviewWindow, expanded: bool) {
    use objc2::msg_send;
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let (_, content) = content_view(hud);
    if content.is_null() {
        return;
    }
    let frost = frost_view(content);
    if frost.is_null() {
        return;
    }
    let (window, surface, radius) = if expanded {
        (END_PROMPT_WINDOW_SIZE, LogicalSize::new(380.0, 48.0), 14.0)
    } else {
        (WINDOW_SIZE, PILL_SIZE, 10.0)
    };
    let frame = NSRect::new(
        NSPoint::new(
            (window.width - surface.width) / 2.0,
            (window.height - surface.height) / 2.0,
        ),
        NSSize::new(surface.width, surface.height),
    );
    let _: () = msg_send![frost, setFrame: frame];
    let layer: *mut AnyObject = msg_send![frost, layer];
    if !layer.is_null() {
        let _: () = msg_send![layer, setCornerRadius: radius];
    }
}

/// Turn the contentView's layer between flat (0°) and upright (90°). Because
/// the frost view and the webview are both subviews, one transform carries the
/// blur, tint, mark, and waveform together — they can't desync or clip. The
/// frost's length follows the pill's (CSS shrinks the upright pill; the two
/// animations share the duration and curve).
#[cfg(target_os = "macos")]
unsafe fn rotate_content(hud: &WebviewWindow, vertical: bool, animate: bool) {
    use objc2::msg_send;
    use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};

    let (window, content) = content_view(hud);
    if content.is_null() {
        return;
    }
    let layer: *mut AnyObject = msg_send![content, layer];
    if layer.is_null() {
        return;
    }

    // Rotate about the window's center. AppKit anchors view layers at their
    // corner, so re-center the anchor (re-asserted every turn — layout passes
    // can reset layer geometry).
    let frame: NSRect = msg_send![content, frame];
    let _: () = msg_send![layer, setAnchorPoint: NSPoint::new(0.5, 0.5)];
    let _: () = msg_send![layer, setPosition: NSPoint::new(
        frame.origin.x + frame.size.width / 2.0,
        frame.origin.y + frame.size.height / 2.0,
    )];

    // CA's +z spins counterclockwise (y-up), so -90° swings the pill's left
    // end (the Clovy mark) to the top, matching the old CSS rotate(90deg).
    let angle = if vertical {
        -std::f64::consts::FRAC_PI_2
    } else {
        0.0
    };
    let key = NSString::from_str("transform.rotation.z");
    let target = NSNumber::new_f64(angle);

    if animate {
        if let Some(animation_class) = AnyClass::get(c"CABasicAnimation") {
            let animation: *mut AnyObject = msg_send![animation_class, animationWithKeyPath: &*key];
            if !animation.is_null() {
                // Start from wherever the layer visibly is right now, so a
                // turn reversed mid-flight doubles back smoothly.
                let presentation: *mut AnyObject = msg_send![layer, presentationLayer];
                let source = if presentation.is_null() {
                    layer
                } else {
                    presentation
                };
                let from: *mut AnyObject = msg_send![source, valueForKeyPath: &*key];
                let _: () = msg_send![animation, setFromValue: from];
                let _: () = msg_send![animation, setToValue: &*target];
                let _: () = msg_send![animation, setDuration: TURN_SECS];
                let timing = ease_out_timing();
                if !timing.is_null() {
                    let _: () = msg_send![animation, setTimingFunction: timing];
                }
                let _: () = msg_send![layer, addAnimation: animation, forKey: &*key];
            }
        }
    }

    // The upright pill runs shorter (fewer bars); keep the frost hugging it.
    // Same duration and curve as the turn, on the same Core Animation clock.
    let frost = frost_view(content);
    if !frost.is_null() {
        let length = if vertical {
            VERTICAL_PILL_LENGTH
        } else {
            PILL_SIZE.width
        };
        let frost_frame = NSRect::new(
            NSPoint::new(
                (WINDOW_SIZE.width - length) / 2.0,
                (WINDOW_SIZE.height - PILL_SIZE.height) / 2.0,
            ),
            NSSize::new(length, PILL_SIZE.height),
        );
        if animate {
            if let Some(context_class) = AnyClass::get(c"NSAnimationContext") {
                let _: () = msg_send![context_class, beginGrouping];
                let context: *mut AnyObject = msg_send![context_class, currentContext];
                if !context.is_null() {
                    let _: () = msg_send![context, setDuration: TURN_SECS];
                    let timing = ease_out_timing();
                    if !timing.is_null() {
                        let _: () = msg_send![context, setTimingFunction: timing];
                    }
                }
                let animator: *mut AnyObject = msg_send![frost, animator];
                if !animator.is_null() {
                    let _: () = msg_send![animator, setFrame: frost_frame];
                }
                let _: () = msg_send![context_class, endGrouping];
            }
        } else {
            let _: () = msg_send![frost, setFrame: frost_frame];
        }
    }

    // Commit the model value with implicit actions off — the explicit
    // animation above (when any) owns the visible motion.
    if let Some(transaction) = AnyClass::get(c"CATransaction") {
        let _: () = msg_send![transaction, begin];
        let _: () = msg_send![transaction, setDisableActions: true];
        let _: () = msg_send![layer, setValue: &*target, forKeyPath: &*key];
        let _: () = msg_send![transaction, commit];
    } else {
        let _: () = msg_send![layer, setValue: &*target, forKeyPath: &*key];
    }
    if !animate && !window.is_null() {
        let _: () = msg_send![window, invalidateShadow];
    }
}

/// Ask AppKit to recompute the window shadow from the rendered content — the
/// shape changed when the pill turned.
#[cfg(target_os = "macos")]
unsafe fn invalidate_shadow(hud: &WebviewWindow) {
    use objc2::msg_send;

    let (window, _) = content_view(hud);
    if !window.is_null() {
        let _: () = msg_send![window, invalidateShadow];
    }
}

/// macOS: reclass the HUD's NSWindow to a non-activating NSPanel so clicking the
/// pill or its buttons never steals focus from the meeting app underneath.
/// Mirrors `dictation.rs::make_hud_nonactivating`.
#[cfg(target_os = "macos")]
fn make_nonactivating(hud: &WebviewWindow) {
    use objc2::msg_send;

    let Ok(handle) = hud.ns_window() else {
        return;
    };
    if handle.is_null() {
        return;
    }
    let Some(panel_class) = AnyClass::get(c"NSPanel") else {
        return;
    };

    unsafe {
        let window = handle as *mut AnyObject;
        objc2::ffi::object_setClass(window, panel_class as *const _ as *const _);

        // NSWindowStyleMaskNonactivatingPanel = 1 << 7.
        const NON_ACTIVATING: usize = 1 << 7;
        let current_mask: usize = msg_send![window, styleMask];
        let _: () = msg_send![window, setStyleMask: current_mask | NON_ACTIVATING];

        let _: () = msg_send![window, setAcceptsMouseMovedEvents: true];
        let _: () = msg_send![window, setBecomesKeyOnlyIfNeeded: true];
    }
}

#[cfg(test)]
mod tests {
    use super::{
        clovy_inactive_for_foreground, ACTIVE_TICK, END_PROMPT_WINDOW_SIZE, PILL_SIZE,
        VERTICAL_PILL_LENGTH, WINDOW_SIZE,
    };
    use crate::domain::types::{
        AudioLevelDto, RecordingSource, RecordingSourceMode, RecordingState, RecordingStatusDto,
        RecordingTelemetryDto, SourceState, SourceStatusDto,
    };
    use std::time::Duration;

    #[test]
    fn active_telemetry_cadence_keeps_waveform_responsive() {
        assert_eq!(
            ACTIVE_TICK,
            Duration::from_millis(50),
            "the waveform peak window and ballistics are tuned for 20 Hz level targets"
        );
    }

    /// Windows has no app-level activation, so the HUD (and the end-of-meeting
    /// countdown it carries) must take over whenever another process owns the
    /// foreground window, even with Clovy's main window visible behind it.
    #[test]
    fn windows_clovy_is_inactive_when_another_process_owns_the_foreground() {
        assert!(clovy_inactive_for_foreground(Some(4242), 1000));
    }

    #[test]
    fn windows_clovy_is_inactive_without_a_foreground_window() {
        assert!(clovy_inactive_for_foreground(None, 1000));
    }

    #[test]
    fn windows_clovy_is_active_when_it_owns_the_foreground() {
        assert!(!clovy_inactive_for_foreground(Some(1000), 1000));
    }

    /// First `prop: <n>px` declaration inside the rule whose selector line
    /// contains `selector`. Good enough for the flat declarations this test
    /// reads — it is a tripwire, not a CSS parser.
    fn css_px(css: &str, selector: &str, prop: &str) -> f64 {
        let start = css
            .lines()
            .position(|line| line.contains(selector) && line.contains('{'))
            .unwrap_or_else(|| panic!("selector {selector:?} not found"));
        let mut depth = 0usize;
        for line in css.lines().skip(start) {
            if let Some(value) = line.trim().strip_prefix(&format!("{prop}: ")) {
                if depth == 1 {
                    return value
                        .trim_end_matches(';')
                        .trim_end_matches("px")
                        .parse()
                        .unwrap_or_else(|_| panic!("unparsable {prop} in {selector:?}"));
                }
            }
            depth += line.matches('{').count();
            depth = depth.saturating_sub(line.matches('}').count());
            if depth == 0 && line.contains('}') {
                break;
            }
        }
        panic!("{prop} not found inside {selector:?}");
    }

    /// The frost NSVisualEffectView is framed from these Rust constants while
    /// the pill itself is painted from meeting-hud.css and the window from
    /// tauri.conf.json — nothing aligns them at runtime, so drift means a
    /// frost halo sticking out of (or hiding under) the pill. This test is
    /// the shared source of truth the comments promise.
    #[test]
    fn pill_dimensions_agree_with_css_and_window_config() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let css = std::fs::read_to_string(manifest.join("../src/styles/meeting-hud.css"))
            .expect("meeting-hud.css should be readable");

        assert_eq!(css_px(&css, ".mhud", "width"), PILL_SIZE.width);
        assert_eq!(css_px(&css, ".mhud", "height"), PILL_SIZE.height);
        assert_eq!(
            css_px(&css, ".mhud[data-orient=\"vertical\"]", "width"),
            VERTICAL_PILL_LENGTH,
        );
        assert_eq!(
            css_px(&css, ".mhud[data-mode=\"meeting-end\"]", "width") + 20.0,
            END_PROMPT_WINDOW_SIZE.width,
        );
        assert_eq!(
            css_px(&css, ".mhud[data-mode=\"meeting-end\"]", "height") + 20.0,
            END_PROMPT_WINDOW_SIZE.height,
        );

        let conf: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(manifest.join("tauri.conf.json"))
                .expect("tauri.conf.json should be readable"),
        )
        .expect("tauri.conf.json should parse");
        let window = conf["app"]["windows"]
            .as_array()
            .expect("windows array")
            .iter()
            .find(|window| window["label"] == "meeting-hud")
            .expect("meeting-hud window entry");
        assert_eq!(window["width"].as_f64(), Some(WINDOW_SIZE.width));
        assert_eq!(window["height"].as_f64(), Some(WINDOW_SIZE.height));
    }

    #[test]
    fn recording_telemetry_omits_stable_command_fields() {
        let level = AudioLevelDto {
            peak: 0.5,
            rms: 0.25,
            recent_peaks: vec![0.2, 0.5],
        };
        let status = RecordingStatusDto {
            session_id: "session-1".to_string(),
            note_id: Some("note-1".to_string()),
            source_mode: RecordingSourceMode::MicrophoneOnly,
            state: RecordingState::Recording,
            elapsed_ms: 1_250,
            level: level.clone(),
            silence_warning: false,
            bytes_written: 8_192,
            live_preview_enabled: true,
            sources: vec![SourceStatusDto {
                source: RecordingSource::Microphone,
                state: SourceState::Recording,
                elapsed_ms: 1_250,
                bytes_written: 8_192,
                dropped_samples: 0,
                level,
                silence_warning: false,
                path_finalized: false,
                last_error: None,
            }],
            warnings: Vec::new(),
        };

        let value = serde_json::to_value(RecordingTelemetryDto::from(&status))
            .expect("telemetry should serialize");
        assert_eq!(value["sessionId"], "session-1");
        assert_eq!(value["elapsedMs"], 1_250);
        assert!(value.get("noteId").is_none());
        assert!(value.get("sourceMode").is_none());
        assert!(value.get("bytesWritten").is_none());
        assert!(value.get("livePreviewEnabled").is_none());
        assert!(value["sources"][0].get("bytesWritten").is_none());
        assert!(value["sources"][0].get("pathFinalized").is_none());
    }
}
