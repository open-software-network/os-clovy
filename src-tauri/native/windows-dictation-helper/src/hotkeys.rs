use crate::protocol::{ShortcutCommand, ShortcutKind, ShortcutModifiers};
use crate::solo_key::{
    self, AltReleaseMask, CaptureTracker, Diagnostic, Gesture, GestureEvent, GestureStep, Input,
    KeySnapshot, SoloCapture,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{mpsc, Arc},
    thread,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::GetLastError,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, RegisterHotKey, SendInput, UnregisterHotKey, INPUT, INPUT_0,
            INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
            MOD_SHIFT, MOD_WIN, VK_0, VK_A, VK_BACK, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_F1, VK_F12,
            VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_OEM_1, VK_OEM_2,
            VK_OEM_3, VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS,
            VK_OEM_PERIOD, VK_OEM_PLUS, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT,
            VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB, VK_UP,
        },
        WindowsAndMessaging::{
            CallNextHookEx, DispatchMessageW, GetMessageW, KillTimer, PeekMessageW,
            PostThreadMessageW, SetTimer, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
            KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE,
            WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN,
            WM_MBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_SYSKEYDOWN,
            WM_SYSKEYUP, WM_TIMER, WM_XBUTTONDOWN,
        },
    },
};

const HOTKEY_PUSH_TO_TALK: i32 = 1;
const HOTKEY_TOGGLE: i32 = 2;
const HOTKEY_RECONFIGURE: u32 = WM_APP + 1;
const HOTKEY_KEYBOARD_EVENT: u32 = WM_APP + 2;
/// A mouse button press or wheel turn, for lone-modifier chords
/// (Ctrl + click, Ctrl + wheel); lParam is 1 when injected.
const HOTKEY_MOUSE_EVENT: u32 = WM_APP + 3;
/// How often a pending lone-modifier press is checked against its hold
/// threshold.
const GESTURE_TICK_MS: u32 = 20;
/// An unassigned virtual key: tapped while a lone Alt dictates so its
/// release does not open the focused app's menu bar.
const VK_MENU_MASK: u16 = 0xE8;
const CAPTURE_TIMEOUT_MS: u32 = 12_000;
const PUSH_RELEASE_POLL_MS: u32 = 50;
const ALTGR_FAKE_CTRL_SCAN_CODE: u32 = 0x21D;

type EventSink = Arc<dyn Fn(HotkeyEvent) + Send + Sync + 'static>;

#[derive(Clone, Debug)]
pub enum HotkeyEvent {
    KeyDown {
        kind: ShortcutKind,
        shortcut: String,
    },
    KeyUp {
        kind: ShortcutKind,
        shortcut: String,
    },
    /// Another key joined a held lone-key push-to-talk (Right Ctrl + C):
    /// drop the take instead of pasting it.
    KeyCancel {
        kind: ShortcutKind,
        shortcut: String,
    },
    Ready {
        push_to_talk_shortcut: String,
        toggle_shortcut: String,
    },
    RegistrationFailed {
        kind: ShortcutKind,
        shortcut: String,
        code: String,
        message: String,
    },
    CaptureStarted {
        kind: ShortcutKind,
    },
    CaptureCancelled {
        kind: ShortcutKind,
    },
    CaptureError {
        kind: ShortcutKind,
        code: String,
        message: String,
    },
    Captured {
        kind: ShortcutKind,
        shortcut: ShortcutCommand,
    },
    /// What the hook saw for a lone-modifier shortcut and what it did, for
    /// the dictation event log.
    Diagnostic {
        kind: ShortcutKind,
        diagnostic: Diagnostic,
        action: &'static str,
    },
}

/// The hook's view of one key edge, packed into the posted message's lParam.
const KEY_DOWN_BIT: isize = 1;
const KEY_INJECTED_BIT: isize = 2;
const KEY_ALTGR_BIT: isize = 4;
/// A down whose key was already down: auto-repeat. Read in the hook, which
/// runs before Windows updates the key's async state.
const KEY_REPEAT_BIT: isize = 8;
/// A down the hook saw while another key was held ([`KeySnapshot`]): the
/// one answer the gestures and the Alt mask share.
const KEY_NOT_ALONE_BIT: isize = 16;

#[derive(Clone, Debug)]
pub struct HotkeyManager {
    sender: mpsc::Sender<HotkeyMessage>,
    thread_id: u32,
}

#[derive(Clone, Debug)]
enum HotkeyMessage {
    Set(ShortcutCommand),
    StartCapture(ShortcutKind),
    CancelCapture,
    Shutdown,
}

impl HotkeyManager {
    pub fn start(event_sink: Box<dyn Fn(HotkeyEvent) + Send + Sync + 'static>) -> Option<Self> {
        let (message_tx, message_rx) = mpsc::channel::<HotkeyMessage>();
        let (ready_tx, ready_rx) = mpsc::channel::<u32>();
        let event_sink = Arc::from(event_sink);
        thread::spawn(move || hotkey_thread(message_rx, ready_tx, event_sink));
        let thread_id = ready_rx.recv().ok()?;
        Some(Self {
            sender: message_tx,
            thread_id,
        })
    }

    pub fn set_shortcut(&self, shortcut: ShortcutCommand) {
        self.send(HotkeyMessage::Set(shortcut), HOTKEY_RECONFIGURE);
    }

    pub fn start_capture(&self, kind: ShortcutKind) {
        self.send(HotkeyMessage::StartCapture(kind), HOTKEY_RECONFIGURE);
    }

    pub fn cancel_capture(&self) {
        self.send(HotkeyMessage::CancelCapture, HOTKEY_RECONFIGURE);
    }

    pub fn shutdown(&self) {
        self.send(HotkeyMessage::Shutdown, WM_QUIT);
    }

    fn send(&self, message: HotkeyMessage, wake_message: u32) {
        let _ = self.sender.send(message);
        unsafe { PostThreadMessageW(self.thread_id, wake_message, 0, 0) };
    }
}

#[derive(Clone, Copy, Debug)]
struct CaptureState {
    kind: ShortcutKind,
}

#[derive(Default)]
struct HotkeyThreadState {
    shortcuts: HashMap<ShortcutKind, ShortcutCommand>,
    /// Lone-modifier shortcuts, one gesture per key (push-to-talk and a
    /// double-tap toggle may share it), watched through the hooks because
    /// `RegisterHotKey` cannot take a modifier alone.
    gestures: Vec<Gesture>,
    gesture_timer_id: Option<usize>,
    capture_keys: CaptureTracker,
    registered: HashSet<i32>,
    pressed_keys: HashSet<u32>,
    active_push_to_talk: bool,
    capture: Option<CaptureState>,
    capture_timer_id: Option<usize>,
    push_release_timer_id: Option<usize>,
}

fn hotkey_thread(
    message_rx: mpsc::Receiver<HotkeyMessage>,
    ready_tx: mpsc::Sender<u32>,
    event_sink: EventSink,
) {
    let thread_id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    let mut initial_msg = MSG::default();
    unsafe {
        PeekMessageW(&mut initial_msg, std::ptr::null_mut(), 0, 0, PM_NOREMOVE);
    }
    let hook = unsafe {
        SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(low_level_keyboard_proc),
            std::ptr::null_mut(),
            0,
        )
    };
    let mouse_hook = unsafe {
        SetWindowsHookExW(
            WH_MOUSE_LL,
            Some(low_level_mouse_proc),
            std::ptr::null_mut(),
            0,
        )
    };
    let _ = ready_tx.send(thread_id);

    let started = Instant::now();
    let mut state = HotkeyThreadState::default();
    if hook.is_null() {
        event_sink(HotkeyEvent::RegistrationFailed {
            kind: ShortcutKind::PushToTalk,
            shortcut: String::new(),
            code: "keyboard_hook_unavailable".to_string(),
            message: "Windows could not start dictation shortcut monitoring.".to_string(),
        });
    }

    loop {
        if drain_messages(&message_rx, &mut state, &event_sink) {
            break;
        }
        let mut msg = MSG::default();
        let result = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
        if result <= 0 {
            break;
        }
        match msg.message {
            HOTKEY_RECONFIGURE => {}
            HOTKEY_KEYBOARD_EVENT => handle_hook_message(
                &mut state,
                msg.wParam as u32,
                msg.lParam,
                started.elapsed().as_millis() as u64,
                &event_sink,
            ),
            HOTKEY_MOUSE_EVENT => handle_mouse_event(
                &mut state,
                msg.lParam != 0,
                started.elapsed().as_millis() as u64,
                &event_sink,
            ),
            WM_TIMER => {
                if state.capture_timer_id == Some(msg.wParam) {
                    end_capture_on_timeout(&mut state, &event_sink);
                } else if state.push_release_timer_id == Some(msg.wParam) {
                    poll_active_push_to_talk(&mut state, &event_sink);
                } else if state.gesture_timer_id == Some(msg.wParam) {
                    tick_gestures(
                        &mut state,
                        started.elapsed().as_millis() as u64,
                        &event_sink,
                    );
                }
            }
            WM_HOTKEY if state.capture.is_none() => match msg.wParam as i32 {
                HOTKEY_PUSH_TO_TALK if !state.active_push_to_talk => {
                    if let Some(shortcut) = state.shortcuts.get(&ShortcutKind::PushToTalk) {
                        state.active_push_to_talk = true;
                        state.push_release_timer_id = nonzero_timer_id(unsafe {
                            SetTimer(std::ptr::null_mut(), 0, PUSH_RELEASE_POLL_MS, None)
                        });
                        event_sink(HotkeyEvent::KeyDown {
                            kind: ShortcutKind::PushToTalk,
                            shortcut: shortcut.label.clone(),
                        });
                    }
                }
                HOTKEY_TOGGLE => {
                    if let Some(shortcut) = state.shortcuts.get(&ShortcutKind::Toggle) {
                        event_sink(HotkeyEvent::KeyDown {
                            kind: ShortcutKind::Toggle,
                            shortcut: shortcut.label.clone(),
                        });
                    }
                }
                _ => {}
            },
            _ => unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            },
        }
    }

    release_active_push_to_talk(&mut state, &event_sink);
    unregister_all(&mut state);
    kill_timer(&mut state.gesture_timer_id);
    if !hook.is_null() {
        let _ = unsafe { UnhookWindowsHookEx(hook) };
    }
    if !mouse_hook.is_null() {
        let _ = unsafe { UnhookWindowsHookEx(mouse_hook) };
    }
}

fn drain_messages(
    message_rx: &mpsc::Receiver<HotkeyMessage>,
    state: &mut HotkeyThreadState,
    event_sink: &EventSink,
) -> bool {
    let mut shortcuts_changed = false;
    while let Ok(message) = message_rx.try_recv() {
        match message {
            HotkeyMessage::Set(shortcut) => {
                release_active_push_to_talk(state, event_sink);
                state.shortcuts.insert(shortcut.kind, shortcut);
                rebuild_gestures(state);
                shortcuts_changed = true;
            }
            HotkeyMessage::StartCapture(kind) => {
                release_active_push_to_talk(state, event_sink);
                unregister_all(state);
                state.capture = Some(CaptureState { kind });
                state.capture_keys = CaptureTracker::default();
                state.pressed_keys.clear();
                kill_timer(&mut state.capture_timer_id);
                state.capture_timer_id = nonzero_timer_id(unsafe {
                    SetTimer(std::ptr::null_mut(), 0, CAPTURE_TIMEOUT_MS, None)
                });
                event_sink(HotkeyEvent::CaptureStarted { kind });
            }
            HotkeyMessage::CancelCapture => cancel_capture(state, event_sink),
            HotkeyMessage::Shutdown => return true,
        }
    }
    if shortcuts_changed && state.capture.is_none() {
        register_shortcuts(state, event_sink);
    }
    false
}

/// One gesture per lone-modifier key among the shortcuts: push-to-talk on
/// it, and the toggle when it uses the same key (a tap or a double tap).
fn rebuild_gestures(state: &mut HotkeyThreadState) {
    let lone = |kind| {
        state
            .shortcuts
            .get(&kind)
            .and_then(|shortcut: &ShortcutCommand| {
                solo_key::is_solo_modifier_code(&shortcut.code)
                    .then(|| solo_key::solo_vk(&shortcut.code))
                    .flatten()
                    .map(|vk| (vk, u32::from(shortcut.press_count)))
            })
    };
    let push = lone(ShortcutKind::PushToTalk).map(|(vk, _)| vk);
    let toggle = lone(ShortcutKind::Toggle);
    let mut gestures = Vec::new();
    if let Some(vk) = push {
        let presses = toggle
            .filter(|(key, _)| *key == vk)
            .map(|(_, presses)| presses);
        gestures.push(Gesture::new(vk, true, presses));
    }
    if let Some((vk, presses)) = toggle.filter(|(key, _)| Some(*key) != push) {
        gestures.push(Gesture::new(vk, false, Some(presses)));
    }
    // The hook masks releases of the toggle's lone Alt by itself.
    let toggle_alt = gestures
        .iter()
        .find(|gesture| {
            gesture.toggles()
                && (gesture.target() == VK_LMENU as u32 || gesture.target() == VK_RMENU as u32)
        })
        .map(Gesture::target);
    HOOK_VIEW.with(|view| view.borrow_mut().mask.set_target(toggle_alt));
    state.gestures = gestures;
    kill_timer(&mut state.gesture_timer_id);
}

/// Which shortcut a gesture event or diagnostic belongs to.
fn gesture_kind(gesture: &Gesture, event: Option<GestureEvent>) -> ShortcutKind {
    if event == Some(GestureEvent::Toggle) || !gesture.pushes() {
        ShortcutKind::Toggle
    } else {
        ShortcutKind::PushToTalk
    }
}

fn handle_mouse_event(
    state: &mut HotkeyThreadState,
    injected: bool,
    now_ms: u64,
    event_sink: &EventSink,
) {
    if state.capture.is_some() {
        return;
    }
    feed_gestures(state, Input::Mouse { injected }, now_ms, event_sink);
}

fn feed_gestures(state: &mut HotkeyThreadState, input: Input, now_ms: u64, event_sink: &EventSink) {
    let steps: Vec<(usize, GestureStep)> = state
        .gestures
        .iter_mut()
        .map(|gesture| gesture.on_input(input, now_ms))
        .enumerate()
        .collect();
    for (index, step) in steps {
        apply_gesture_step(state, index, step, event_sink);
    }
    sync_gesture_timer(state);
}

fn tick_gestures(state: &mut HotkeyThreadState, now_ms: u64, event_sink: &EventSink) {
    let steps: Vec<(usize, GestureStep)> = state
        .gestures
        .iter_mut()
        .map(|gesture| gesture.on_tick(now_ms))
        .enumerate()
        .collect();
    for (index, step) in steps {
        apply_gesture_step(state, index, step, event_sink);
    }
    sync_gesture_timer(state);
}

/// A timer runs only while a press waits for its hold threshold.
fn sync_gesture_timer(state: &mut HotkeyThreadState) {
    let waiting = state
        .gestures
        .iter()
        .any(|gesture| gesture.deadline().is_some());
    if waiting && state.gesture_timer_id.is_none() {
        state.gesture_timer_id =
            nonzero_timer_id(unsafe { SetTimer(std::ptr::null_mut(), 0, GESTURE_TICK_MS, None) });
    } else if !waiting {
        kill_timer(&mut state.gesture_timer_id);
    }
}

/// Turns a gesture's decision into the shortcut edge dictation expects, and
/// logs it.
fn apply_gesture_step(
    state: &mut HotkeyThreadState,
    index: usize,
    step: GestureStep,
    event_sink: &EventSink,
) {
    let Some(gesture) = state.gestures.get(index) else {
        return;
    };
    let target = gesture.target();
    let kind = gesture_kind(gesture, step.event);
    let label = state
        .shortcuts
        .get(&kind)
        .map(|shortcut| shortcut.label.clone());
    let action = match (label, step.event) {
        (_, None) => "none",
        (None, Some(_)) => "no_shortcut",
        (Some(label), Some(GestureEvent::PushDown)) if !state.active_push_to_talk => {
            state.active_push_to_talk = true;
            if target == VK_LMENU as u32 || target == VK_RMENU as u32 {
                mask_alt_menu();
            }
            event_sink(HotkeyEvent::KeyDown {
                kind,
                shortcut: label,
            });
            "push_start"
        }
        (Some(label), Some(GestureEvent::PushUp)) if state.active_push_to_talk => {
            state.active_push_to_talk = false;
            event_sink(HotkeyEvent::KeyUp {
                kind,
                shortcut: label,
            });
            "push_end"
        }
        (Some(label), Some(GestureEvent::PushCancel)) if state.active_push_to_talk => {
            state.active_push_to_talk = false;
            event_sink(HotkeyEvent::KeyCancel {
                kind,
                shortcut: label,
            });
            "push_cancel"
        }
        (Some(label), Some(GestureEvent::Toggle)) => {
            event_sink(HotkeyEvent::KeyDown {
                kind,
                shortcut: label,
            });
            "toggle"
        }
        (Some(_), Some(_)) => "already_handled",
    };
    if let Some(diagnostic) = step.diagnostic {
        event_sink(HotkeyEvent::Diagnostic {
            kind,
            diagnostic,
            action,
        });
    }
}

/// What the keyboard hook procedure decides by itself, at the moment of a
/// key edge: whether this is a lone toggle-Alt release to mask.
/// One key edge as the keyboard hook sees it.
#[derive(Clone, Copy, Debug)]
struct HookEdge {
    vk: u32,
    down: bool,
    injected: bool,
    repeat: bool,
    altgr: bool,
}

/// The synchronous part of the keyboard hook: the flags of the message it
/// posts, and whether to mask this release from the app's menu bar now.
/// `still_down` asks Windows which keys are down at this moment. Whether a
/// down was alone is decided here once ([`KeySnapshot`]): the mask uses it
/// now and the message carries it to the gestures (`KEY_NOT_ALONE_BIT`).
fn hook_edge(edge: HookEdge, still_down: &dyn Fn(u32) -> bool) -> (isize, bool) {
    let mut flags = 0;
    if edge.down {
        flags |= KEY_DOWN_BIT;
    }
    if edge.injected {
        flags |= KEY_INJECTED_BIT;
    }
    if edge.repeat {
        flags |= KEY_REPEAT_BIT;
    }
    if edge.altgr {
        flags |= KEY_ALTGR_BIT;
        return (flags, false);
    }
    HOOK_VIEW.with(|view| {
        let view = &mut *view.borrow_mut();
        let alone = edge.injected || view.keys.on_key(edge.vk, edge.down, still_down);
        if edge.down && !alone {
            flags |= KEY_NOT_ALONE_BIT;
        }
        let mask = view
            .mask
            .on_key(edge.vk, edge.down, edge.injected, edge.repeat, alone);
        (flags, mask)
    })
}

/// What the hook procedures keep for themselves.
#[derive(Default)]
struct HookView {
    keys: KeySnapshot,
    mask: AltReleaseMask,
}

thread_local! {
    /// The hooks run on the hotkey thread, called by Windows in its message
    /// loop before posted messages are handled: this is their own view of
    /// the keys held and of the toggle's lone Alt, fed by every hook event as
    /// it happens.
    static HOOK_VIEW: std::cell::RefCell<HookView> =
        std::cell::RefCell::new(HookView::default());
}

/// Taps an unassigned key while Alt is held, so releasing a lone Alt that
/// dictated does not focus the app's menu bar. The tap is injected, so the
/// gestures ignore it.
fn mask_alt_menu() {
    let key = |flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_MENU_MASK,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [key(0), key(KEYEVENTF_KEYUP)];
    unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
}

fn handle_hook_message(
    state: &mut HotkeyThreadState,
    vk: u32,
    flags: isize,
    now_ms: u64,
    event_sink: &EventSink,
) {
    let is_down = flags & KEY_DOWN_BIT != 0;
    let injected = flags & KEY_INJECTED_BIT != 0;
    let repeat = flags & KEY_REPEAT_BIT != 0;
    if flags & KEY_ALTGR_BIT != 0 {
        // AltGr layouts send a synthetic Left Ctrl with Right Alt; it is not a
        // key the user pressed, but a Left Ctrl shortcut should say why it
        // did not count.
        if state.capture.is_none() {
            for gesture in &state.gestures {
                if gesture.target() == vk {
                    event_sink(HotkeyEvent::Diagnostic {
                        kind: gesture_kind(gesture, None),
                        diagnostic: Diagnostic {
                            key: solo_key::key_name(vk),
                            edge: if is_down { "down" } else { "up" },
                            injected,
                            alone: false,
                            held_ms: None,
                            decision: "ignored",
                            reason: "altgr",
                        },
                        action: "none",
                    });
                }
            }
        }
        return;
    }
    // The hook's own answer, decided at the edge: never looked up again.
    let alone = flags & KEY_NOT_ALONE_BIT == 0;
    handle_keyboard_event(
        state, vk, is_down, injected, repeat, alone, now_ms, event_sink,
    );
}

#[allow(clippy::too_many_arguments)]
fn handle_keyboard_event(
    state: &mut HotkeyThreadState,
    vk: u32,
    is_down: bool,
    injected: bool,
    repeat: bool,
    alone: bool,
    now_ms: u64,
    event_sink: &EventSink,
) {
    // `pressed_keys` alone would read a fresh press of a key whose release
    // was missed as auto-repeat forever; the hook's own repeat flag decides.
    let repeated = if is_down {
        state.pressed_keys.insert(vk);
        repeat
    } else {
        !state.pressed_keys.remove(&vk)
    };

    if let Some(capture) = state.capture {
        if !repeated {
            handle_capture_key(state, capture, vk, is_down, event_sink);
        }
        return;
    }

    let input = Input::Key {
        vk,
        down: is_down,
        injected,
        repeat,
        alone,
    };
    feed_gestures(state, input, now_ms, event_sink);
    if repeated {
        return;
    }

    if !is_down && state.active_push_to_talk {
        let should_release =
            state
                .shortcuts
                .get(&ShortcutKind::PushToTalk)
                .is_some_and(|shortcut| {
                    virtual_key_for_code(&shortcut.code) == Some(vk)
                        || modifier_virtual_keys(shortcut.modifiers).contains(&vk)
                });
        if should_release {
            release_active_push_to_talk(state, event_sink);
        }
    }
}

fn handle_capture_key(
    state: &mut HotkeyThreadState,
    capture: CaptureState,
    vk: u32,
    is_down: bool,
    event_sink: &EventSink,
) {
    if is_down && vk == VK_ESCAPE as u32 {
        cancel_capture(state, event_sink);
        return;
    }
    match state.capture_keys.on_key(vk, is_down, is_modifier_vk(vk)) {
        Some(SoloCapture::Modifier(vk)) => {
            if let Some((code, label)) = solo_key::solo_code(vk) {
                // A lone modifier toggles with a double tap, as Fn does on
                // the Mac: a single tap of Ctrl happens all the time.
                let (label, press_count) = match capture.kind {
                    ShortcutKind::Toggle => (format!("{label}+{label}"), 2),
                    ShortcutKind::PushToTalk => (label.to_string(), 1),
                };
                finish_capture_with_presses(
                    state,
                    capture,
                    vk,
                    code,
                    label,
                    ShortcutModifiers::default(),
                    press_count,
                    event_sink,
                );
            }
            return;
        }
        Some(SoloCapture::Unsupported(_)) => {
            fail_capture(
                state,
                capture,
                "modifier_required",
                "Shortcut must include Ctrl, Alt, Shift, or Win.",
                event_sink,
            );
            return;
        }
        None => {}
    }
    if !is_down || is_modifier_vk(vk) {
        return;
    }

    let modifiers = current_modifiers(&state.pressed_keys);
    if !has_standard_modifier(modifiers) {
        if let Some((code, label)) = solo_key::solo_code(vk) {
            finish_capture(
                state,
                capture,
                vk,
                code,
                label.to_string(),
                modifiers,
                event_sink,
            );
            return;
        }
        fail_capture(
            state,
            capture,
            "modifier_required",
            "Shortcut must include Ctrl, Alt, Shift, or Win.",
            event_sink,
        );
        return;
    }
    let Some(code) = code_for_virtual_key(vk) else {
        fail_capture(
            state,
            capture,
            "unsupported_key",
            "Choose a supported letter, number, punctuation, navigation, or function key.",
            event_sink,
        );
        return;
    };
    let label = shortcut_label(modifiers, code);
    finish_capture(state, capture, vk, code, label, modifiers, event_sink);
}

fn finish_capture(
    state: &mut HotkeyThreadState,
    capture: CaptureState,
    vk: u32,
    code: &str,
    label: String,
    modifiers: ShortcutModifiers,
    event_sink: &EventSink,
) {
    finish_capture_with_presses(state, capture, vk, code, label, modifiers, 1, event_sink);
}

#[allow(clippy::too_many_arguments)]
fn finish_capture_with_presses(
    state: &mut HotkeyThreadState,
    capture: CaptureState,
    vk: u32,
    code: &str,
    label: String,
    modifiers: ShortcutModifiers,
    press_count: u8,
    event_sink: &EventSink,
) {
    let shortcut = ShortcutCommand {
        key_code: vk,
        code: code.to_string(),
        label,
        kind: capture.kind,
        press_count,
        modifiers,
    };
    state.capture = None;
    kill_timer(&mut state.capture_timer_id);
    state.pressed_keys.clear();
    event_sink(HotkeyEvent::Captured {
        kind: capture.kind,
        shortcut,
    });
    register_shortcuts(state, event_sink);
}

fn cancel_capture(state: &mut HotkeyThreadState, event_sink: &EventSink) {
    let Some(capture) = state.capture.take() else {
        return;
    };
    kill_timer(&mut state.capture_timer_id);
    state.pressed_keys.clear();
    event_sink(HotkeyEvent::CaptureCancelled { kind: capture.kind });
    register_shortcuts(state, event_sink);
}

/// The capture timed out: nothing pressed at all means the key never reached
/// Windows (Fn on most keyboards); otherwise it simply ended.
fn end_capture_on_timeout(state: &mut HotkeyThreadState, event_sink: &EventSink) {
    let Some(capture) = state.capture else {
        return;
    };
    if state.capture_keys.saw_any_key() {
        cancel_capture(state, event_sink);
    } else {
        fail_capture(
            state,
            capture,
            "no_key_detected",
            "No key reached Windows. Keys such as Fn are handled by the keyboard itself.",
            event_sink,
        );
    }
}

fn fail_capture(
    state: &mut HotkeyThreadState,
    capture: CaptureState,
    code: &str,
    message: &str,
    event_sink: &EventSink,
) {
    fail_capture_with(
        state,
        capture,
        code,
        message,
        event_sink,
        register_shortcuts,
    );
}

fn fail_capture_with(
    state: &mut HotkeyThreadState,
    capture: CaptureState,
    code: &str,
    message: &str,
    event_sink: &EventSink,
    mut register: impl FnMut(&mut HotkeyThreadState, &EventSink),
) {
    state.capture = None;
    kill_timer(&mut state.capture_timer_id);
    state.pressed_keys.clear();
    event_sink(HotkeyEvent::CaptureError {
        kind: capture.kind,
        code: code.to_string(),
        message: message.to_string(),
    });
    register(state, event_sink);
}

fn register_shortcuts(state: &mut HotkeyThreadState, event_sink: &EventSink) {
    // The hook was not fed during a capture; start the lone-key rules over.
    rebuild_gestures(state);
    register_shortcuts_with(state, event_sink, |id, modifiers, vk| {
        if unsafe { RegisterHotKey(std::ptr::null_mut(), id, modifiers, vk) } != 0 {
            Ok(())
        } else {
            Err(unsafe { GetLastError() })
        }
    });
}

fn register_shortcuts_with(
    state: &mut HotkeyThreadState,
    event_sink: &EventSink,
    mut register: impl FnMut(i32, u32, u32) -> Result<(), u32>,
) {
    unregister_all(state);
    let mut all_registered = state.shortcuts.len() == 2;
    for kind in [ShortcutKind::PushToTalk, ShortcutKind::Toggle] {
        let Some(shortcut) = state.shortcuts.get(&kind) else {
            all_registered = false;
            continue;
        };
        let Some(vk) = virtual_key_for_code(&shortcut.code) else {
            all_registered = false;
            event_sink(HotkeyEvent::RegistrationFailed {
                kind,
                shortcut: shortcut.label.clone(),
                code: "unsupported_key".to_string(),
                message: format!(
                    "{} is not supported as a Windows dictation shortcut.",
                    shortcut.label
                ),
            });
            continue;
        };
        let solo = solo_key::solo_vk(&shortcut.code).is_some();
        // A double tap exists only for lone modifiers (the hook counts taps).
        let presses_ok = shortcut.press_count == 1
            || (shortcut.press_count == 2 && solo_key::is_solo_modifier_code(&shortcut.code));
        if !presses_ok
            || shortcut.modifiers.function
            || (!solo && !has_standard_modifier(shortcut.modifiers))
        {
            all_registered = false;
            event_sink(HotkeyEvent::RegistrationFailed {
                kind,
                shortcut: shortcut.label.clone(),
                code: "unsupported_shortcut".to_string(),
                message: format!(
                    "{} is not supported as a Windows dictation shortcut.",
                    shortcut.label
                ),
            });
            continue;
        }
        if solo_key::is_solo_modifier_code(&shortcut.code) {
            // Watched by the keyboard hook (`handle_solo_modifiers`).
            continue;
        }
        let id = hotkey_id(kind);
        let modifiers = modifiers_to_win32(shortcut.modifiers) | MOD_NOREPEAT;
        match register(id, modifiers, vk) {
            Ok(()) => {
                state.registered.insert(id);
            }
            Err(error) => {
                all_registered = false;
                event_sink(HotkeyEvent::RegistrationFailed {
                    kind,
                    shortcut: shortcut.label.clone(),
                    code: format!("register_hotkey_{}", error),
                    message: format!(
                        "Windows could not register {}. It may already be used by another app.",
                        shortcut.label
                    ),
                });
            }
        }
    }

    if !all_registered {
        return;
    }

    let push_to_talk_shortcut = state
        .shortcuts
        .get(&ShortcutKind::PushToTalk)
        .map(|shortcut| shortcut.label.clone())
        .unwrap_or_default();
    let toggle_shortcut = state
        .shortcuts
        .get(&ShortcutKind::Toggle)
        .map(|shortcut| shortcut.label.clone())
        .unwrap_or_default();
    event_sink(HotkeyEvent::Ready {
        push_to_talk_shortcut,
        toggle_shortcut,
    });
}

fn nonzero_timer_id(timer_id: usize) -> Option<usize> {
    (timer_id != 0).then_some(timer_id)
}

fn kill_timer(timer_id: &mut Option<usize>) {
    if let Some(timer_id) = timer_id.take() {
        unsafe { KillTimer(std::ptr::null_mut(), timer_id) };
    }
}

fn poll_active_push_to_talk(state: &mut HotkeyThreadState, event_sink: &EventSink) {
    if !state.active_push_to_talk {
        kill_timer(&mut state.push_release_timer_id);
        return;
    }
    let still_held = state
        .shortcuts
        .get(&ShortcutKind::PushToTalk)
        .and_then(|shortcut| {
            let key = virtual_key_for_code(&shortcut.code)?;
            Some(key_is_down(key) && required_modifiers_are_down(shortcut.modifiers))
        })
        .unwrap_or(false);
    if !still_held {
        release_active_push_to_talk(state, event_sink);
    }
}

fn required_modifiers_are_down(modifiers: ShortcutModifiers) -> bool {
    (!modifiers.command || key_is_down(VK_LWIN as u32) || key_is_down(VK_RWIN as u32))
        && (!modifiers.control
            || key_is_down(VK_CONTROL as u32)
            || key_is_down(VK_LCONTROL as u32)
            || key_is_down(VK_RCONTROL as u32))
        && (!modifiers.option
            || key_is_down(VK_MENU as u32)
            || key_is_down(VK_LMENU as u32)
            || key_is_down(VK_RMENU as u32))
        && (!modifiers.shift
            || key_is_down(VK_SHIFT as u32)
            || key_is_down(VK_LSHIFT as u32)
            || key_is_down(VK_RSHIFT as u32))
}

fn key_is_down(vk: u32) -> bool {
    (unsafe { GetAsyncKeyState(vk as i32) }) < 0
}

fn release_active_push_to_talk(state: &mut HotkeyThreadState, event_sink: &EventSink) {
    if !state.active_push_to_talk {
        return;
    }
    state.active_push_to_talk = false;
    kill_timer(&mut state.push_release_timer_id);
    if let Some(shortcut) = state.shortcuts.get(&ShortcutKind::PushToTalk) {
        event_sink(HotkeyEvent::KeyUp {
            kind: ShortcutKind::PushToTalk,
            shortcut: shortcut.label.clone(),
        });
    }
}

fn unregister_all(state: &mut HotkeyThreadState) {
    for id in state.registered.drain() {
        unsafe { UnregisterHotKey(std::ptr::null_mut(), id) };
    }
}

fn hotkey_id(kind: ShortcutKind) -> i32 {
    match kind {
        ShortcutKind::PushToTalk => HOTKEY_PUSH_TO_TALK,
        ShortcutKind::Toggle => HOTKEY_TOGGLE,
    }
}

fn modifiers_to_win32(modifiers: ShortcutModifiers) -> u32 {
    let mut value = 0;
    if modifiers.control {
        value |= MOD_CONTROL;
    }
    if modifiers.option {
        value |= MOD_ALT;
    }
    if modifiers.shift {
        value |= MOD_SHIFT;
    }
    if modifiers.command {
        value |= MOD_WIN;
    }
    value
}

fn current_modifiers(pressed_keys: &HashSet<u32>) -> ShortcutModifiers {
    ShortcutModifiers {
        command: key_is_down(VK_LWIN as u32)
            || key_is_down(VK_RWIN as u32)
            || pressed_keys.contains(&(VK_LWIN as u32))
            || pressed_keys.contains(&(VK_RWIN as u32)),
        control: key_is_down(VK_CONTROL as u32)
            || key_is_down(VK_LCONTROL as u32)
            || key_is_down(VK_RCONTROL as u32)
            || pressed_keys.contains(&(VK_CONTROL as u32))
            || pressed_keys.contains(&(VK_LCONTROL as u32))
            || pressed_keys.contains(&(VK_RCONTROL as u32)),
        option: key_is_down(VK_MENU as u32)
            || key_is_down(VK_LMENU as u32)
            || key_is_down(VK_RMENU as u32)
            || pressed_keys.contains(&(VK_MENU as u32))
            || pressed_keys.contains(&(VK_LMENU as u32))
            || pressed_keys.contains(&(VK_RMENU as u32)),
        shift: key_is_down(VK_SHIFT as u32)
            || key_is_down(VK_LSHIFT as u32)
            || key_is_down(VK_RSHIFT as u32)
            || pressed_keys.contains(&(VK_SHIFT as u32))
            || pressed_keys.contains(&(VK_LSHIFT as u32))
            || pressed_keys.contains(&(VK_RSHIFT as u32)),
        function: false,
    }
}

fn modifier_virtual_keys(modifiers: ShortcutModifiers) -> Vec<u32> {
    let mut keys = Vec::new();
    if modifiers.command {
        keys.extend([VK_LWIN as u32, VK_RWIN as u32]);
    }
    if modifiers.control {
        keys.extend([VK_CONTROL as u32, VK_LCONTROL as u32, VK_RCONTROL as u32]);
    }
    if modifiers.option {
        keys.extend([VK_MENU as u32, VK_LMENU as u32, VK_RMENU as u32]);
    }
    if modifiers.shift {
        keys.extend([VK_SHIFT as u32, VK_LSHIFT as u32, VK_RSHIFT as u32]);
    }
    keys
}

fn has_standard_modifier(modifiers: ShortcutModifiers) -> bool {
    modifiers.command || modifiers.control || modifiers.option || modifiers.shift
}

fn is_modifier_vk(vk: u32) -> bool {
    matches!(
        vk,
        value if value == VK_CONTROL as u32
            || value == VK_LCONTROL as u32
            || value == VK_RCONTROL as u32
            || value == VK_MENU as u32
            || value == VK_LMENU as u32
            || value == VK_RMENU as u32
            || value == VK_SHIFT as u32
            || value == VK_LSHIFT as u32
            || value == VK_RSHIFT as u32
            || value == VK_LWIN as u32
            || value == VK_RWIN as u32
    )
}

fn shortcut_label(modifiers: ShortcutModifiers, code: &str) -> String {
    [
        modifiers.control.then_some("Ctrl"),
        modifiers.option.then_some("Alt"),
        modifiers.shift.then_some("Shift"),
        modifiers.command.then_some("Win"),
        Some(key_label(code)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("+")
}

fn key_label(code: &str) -> &str {
    code.strip_prefix("Key")
        .or_else(|| code.strip_prefix("Digit"))
        .unwrap_or(code)
}

pub(crate) fn virtual_key_for_code(code: &str) -> Option<u32> {
    if code.len() == 4 && code.starts_with("Key") {
        let letter = code.as_bytes()[3];
        if letter.is_ascii_uppercase() {
            return Some(VK_A as u32 + u32::from(letter - b'A'));
        }
    }
    if code.len() == 6 && code.starts_with("Digit") {
        let digit = code.as_bytes()[5];
        if digit.is_ascii_digit() {
            return Some(VK_0 as u32 + u32::from(digit - b'0'));
        }
    }
    if let Some(number) = code
        .strip_prefix('F')
        .and_then(|value| value.parse::<u32>().ok())
    {
        if (1..=12).contains(&number) {
            return Some(VK_F1 as u32 + number - 1);
        }
    }
    Some(match code {
        "Space" => VK_SPACE as u32,
        "Tab" => VK_TAB as u32,
        "Enter" => VK_RETURN as u32,
        "Backspace" => VK_BACK as u32,
        "Escape" => VK_ESCAPE as u32,
        "ArrowLeft" => VK_LEFT as u32,
        "ArrowUp" => VK_UP as u32,
        "ArrowRight" => VK_RIGHT as u32,
        "ArrowDown" => VK_DOWN as u32,
        "Semicolon" => VK_OEM_1 as u32,
        "Equal" => VK_OEM_PLUS as u32,
        "Comma" => VK_OEM_COMMA as u32,
        "Minus" => VK_OEM_MINUS as u32,
        "Period" => VK_OEM_PERIOD as u32,
        "Slash" => VK_OEM_2 as u32,
        "Backquote" => VK_OEM_3 as u32,
        "BracketLeft" => VK_OEM_4 as u32,
        "Backslash" => VK_OEM_5 as u32,
        "BracketRight" => VK_OEM_6 as u32,
        "Quote" => VK_OEM_7 as u32,
        other => return solo_key::solo_vk(other),
    })
}

fn code_for_virtual_key(vk: u32) -> Option<&'static str> {
    const LETTER_CODES: [&str; 26] = [
        "KeyA", "KeyB", "KeyC", "KeyD", "KeyE", "KeyF", "KeyG", "KeyH", "KeyI", "KeyJ", "KeyK",
        "KeyL", "KeyM", "KeyN", "KeyO", "KeyP", "KeyQ", "KeyR", "KeyS", "KeyT", "KeyU", "KeyV",
        "KeyW", "KeyX", "KeyY", "KeyZ",
    ];
    const DIGIT_CODES: [&str; 10] = [
        "Digit0", "Digit1", "Digit2", "Digit3", "Digit4", "Digit5", "Digit6", "Digit7", "Digit8",
        "Digit9",
    ];
    const FUNCTION_CODES: [&str; 12] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    ];
    if (VK_A as u32..VK_A as u32 + 26).contains(&vk) {
        return LETTER_CODES.get((vk - VK_A as u32) as usize).copied();
    }
    if (VK_0 as u32..VK_0 as u32 + 10).contains(&vk) {
        return DIGIT_CODES.get((vk - VK_0 as u32) as usize).copied();
    }
    if (VK_F1 as u32..=VK_F12 as u32).contains(&vk) {
        return FUNCTION_CODES.get((vk - VK_F1 as u32) as usize).copied();
    }
    match vk {
        value if value == VK_SPACE as u32 => Some("Space"),
        value if value == VK_TAB as u32 => Some("Tab"),
        value if value == VK_RETURN as u32 => Some("Enter"),
        value if value == VK_BACK as u32 => Some("Backspace"),
        value if value == VK_ESCAPE as u32 => Some("Escape"),
        value if value == VK_LEFT as u32 => Some("ArrowLeft"),
        value if value == VK_UP as u32 => Some("ArrowUp"),
        value if value == VK_RIGHT as u32 => Some("ArrowRight"),
        value if value == VK_DOWN as u32 => Some("ArrowDown"),
        value if value == VK_OEM_1 as u32 => Some("Semicolon"),
        value if value == VK_OEM_PLUS as u32 => Some("Equal"),
        value if value == VK_OEM_COMMA as u32 => Some("Comma"),
        value if value == VK_OEM_MINUS as u32 => Some("Minus"),
        value if value == VK_OEM_PERIOD as u32 => Some("Period"),
        value if value == VK_OEM_2 as u32 => Some("Slash"),
        value if value == VK_OEM_3 as u32 => Some("Backquote"),
        value if value == VK_OEM_4 as u32 => Some("BracketLeft"),
        value if value == VK_OEM_5 as u32 => Some("Backslash"),
        value if value == VK_OEM_6 as u32 => Some("BracketRight"),
        value if value == VK_OEM_7 as u32 => Some("Quote"),
        other if !is_modifier_vk(other) => solo_key::solo_code(other).map(|(code, _)| code),
        _ => None,
    }
}

unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if code >= 0 && lparam != 0 {
        let message = wparam as u32;
        let is_down = matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN);
        let is_up = matches!(message, WM_KEYUP | WM_SYSKEYUP);
        if is_down || is_up {
            let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            let edge = HookEdge {
                vk: event.vkCode,
                down: is_down,
                injected: event.flags & LLKHF_INJECTED != 0,
                // The hook runs before Windows updates the async key state,
                // so this is whether the key was already down: auto-repeat.
                repeat: is_down && unsafe { GetAsyncKeyState(event.vkCode as i32) } < 0,
                // AltGr layouts send a synthetic Left Ctrl (scan code 0x21D)
                // with Right Alt; it is not a key the user pressed.
                altgr: event.vkCode == VK_LCONTROL as u32
                    && event.scanCode == ALTGR_FAKE_CTRL_SCAN_CODE,
            };
            let (flags, mask) = hook_edge(edge, &|vk| key_is_down(vk));
            // A lone tap of the toggle's Alt: tap the mask key now, while Alt
            // is still down for the app, so its release does not open the
            // menu bar.
            if mask {
                mask_alt_menu();
            }
            let thread_id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
            unsafe {
                PostThreadMessageW(
                    thread_id,
                    HOTKEY_KEYBOARD_EVENT,
                    event.vkCode as usize,
                    flags,
                )
            };
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

/// Mouse buttons and the wheel, only so a lone-modifier shortcut can tell a
/// Ctrl + click or Ctrl + wheel from a lone press. Moves are passed straight
/// on; nothing about the pointer is kept.
unsafe extern "system" fn low_level_mouse_proc(code: i32, wparam: usize, lparam: isize) -> isize {
    if code >= 0 && lparam != 0 {
        let message = wparam as u32;
        if matches!(
            message,
            WM_LBUTTONDOWN
                | WM_RBUTTONDOWN
                | WM_MBUTTONDOWN
                | WM_XBUTTONDOWN
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
        ) {
            let event = unsafe { &*(lparam as *const MSLLHOOKSTRUCT) };
            let injected = (event.flags & LLMHF_INJECTED != 0) as isize;
            HOOK_VIEW.with(|view| view.borrow_mut().mask.on_mouse(injected != 0));
            let thread_id = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
            unsafe { PostThreadMessageW(thread_id, HOTKEY_MOUSE_EVENT, 0, injected) };
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_the_supported_windows_key_allowlist() {
        for code in [
            "KeyA",
            "KeyZ",
            "Digit0",
            "Digit9",
            "Space",
            "Tab",
            "Enter",
            "Backspace",
            "Escape",
            "ArrowLeft",
            "ArrowUp",
            "ArrowRight",
            "ArrowDown",
            "Semicolon",
            "Equal",
            "Comma",
            "Minus",
            "Period",
            "Slash",
            "Backquote",
            "BracketLeft",
            "Backslash",
            "BracketRight",
            "Quote",
            "F1",
            "F12",
        ] {
            assert!(
                virtual_key_for_code(code).is_some(),
                "missing mapping for {code}"
            );
        }
        // F13-F24, Pause, and the right-hand modifiers work as lone keys.
        assert_eq!(virtual_key_for_code("F13"), Some(0x7C));
        assert_eq!(
            virtual_key_for_code("ControlRight"),
            Some(VK_RCONTROL as u32)
        );
        assert_eq!(virtual_key_for_code("NumpadEnter"), None);
        assert_eq!(virtual_key_for_code("KeyAA"), None);
    }

    #[test]
    fn captured_key_round_trips_through_the_allowlist() {
        for code in ["KeyD", "Digit4", "Space", "ArrowDown", "Quote", "F8"] {
            let vk = virtual_key_for_code(code).expect("supported key");
            assert_eq!(code_for_virtual_key(vk), Some(code));
        }
    }

    #[test]
    fn partial_registration_keeps_the_successful_shortcut_armed() {
        let mut state = HotkeyThreadState::default();
        state.shortcuts.insert(
            ShortcutKind::PushToTalk,
            shortcut(ShortcutKind::PushToTalk, "KeyD", "Ctrl+Alt+D"),
        );
        state.shortcuts.insert(
            ShortcutKind::Toggle,
            shortcut(ShortcutKind::Toggle, "KeyT", "Ctrl+Alt+T"),
        );
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sink: EventSink = Arc::new(move |event| {
            captured_events.lock().expect("event lock").push(event);
        });

        register_shortcuts_with(&mut state, &event_sink, |id, _, _| {
            if id == HOTKEY_PUSH_TO_TALK {
                Ok(())
            } else {
                Err(1409)
            }
        });

        assert_eq!(state.registered, HashSet::from([HOTKEY_PUSH_TO_TALK]));
        let events = events.lock().expect("event lock");
        assert!(events.iter().any(|event| matches!(
            event,
            HotkeyEvent::RegistrationFailed {
                kind: ShortcutKind::Toggle,
                ..
            }
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, HotkeyEvent::Ready { .. })));
    }

    #[test]
    fn failed_capture_clears_state_and_re_registers_shortcuts() {
        let mut state = HotkeyThreadState {
            capture: Some(CaptureState {
                kind: ShortcutKind::PushToTalk,
            }),
            capture_timer_id: Some(42),
            pressed_keys: HashSet::from([VK_A as u32]),
            ..HotkeyThreadState::default()
        };
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sink: EventSink = Arc::new(move |event| {
            captured_events.lock().expect("event lock").push(event);
        });
        let mut registered = false;

        fail_capture_with(
            &mut state,
            CaptureState {
                kind: ShortcutKind::PushToTalk,
            },
            "modifier_required",
            "Shortcut must include Ctrl, Alt, Shift, or Win.",
            &event_sink,
            |state, _| {
                registered = true;
                assert!(state.capture.is_none());
            },
        );

        assert!(registered);
        assert!(state.capture.is_none());
        assert!(state.capture_timer_id.is_none());
        assert!(state.pressed_keys.is_empty());
        assert!(events
            .lock()
            .expect("event lock")
            .iter()
            .any(|event| matches!(
                event,
                HotkeyEvent::CaptureError {
                    kind: ShortcutKind::PushToTalk,
                    code,
                    ..
                } if code == "modifier_required"
            )));
    }

    fn recorder() -> (Arc<std::sync::Mutex<Vec<HotkeyEvent>>>, EventSink) {
        let events = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured_events = Arc::clone(&events);
        let event_sink: EventSink = Arc::new(move |event| {
            captured_events.lock().expect("event lock").push(event);
        });
        (events, event_sink)
    }

    fn lone(kind: ShortcutKind, code: &str, label: &str) -> ShortcutCommand {
        ShortcutCommand {
            modifiers: ShortcutModifiers::default(),
            ..shortcut(kind, code, label)
        }
    }

    /// Right Ctrl goes through the hook, F13 is a hotkey with no modifier;
    /// both count as registered.
    #[test]
    fn lone_keys_register_without_modifiers() {
        let mut state = HotkeyThreadState::default();
        state.shortcuts.insert(
            ShortcutKind::PushToTalk,
            lone(ShortcutKind::PushToTalk, "ControlRight", "Right Ctrl"),
        );
        state.shortcuts.insert(
            ShortcutKind::Toggle,
            lone(ShortcutKind::Toggle, "F13", "F13"),
        );
        let (events, event_sink) = recorder();
        let mut calls = Vec::new();
        register_shortcuts_with(&mut state, &event_sink, |id, modifiers, vk| {
            calls.push((id, modifiers, vk));
            Ok(())
        });
        assert_eq!(calls, [(HOTKEY_TOGGLE, MOD_NOREPEAT, 0x7C)]);
        assert!(events
            .lock()
            .expect("event lock")
            .iter()
            .any(|event| matches!(event, HotkeyEvent::Ready { .. })));
    }

    /// A chord on an extended key (Ctrl+Alt+F13, Ctrl+Alt+Pause) is a hotkey
    /// with its modifiers, like any other chord.
    #[test]
    fn chords_on_extended_keys_register_with_their_modifiers() {
        let mut state = HotkeyThreadState::default();
        state.shortcuts.insert(
            ShortcutKind::PushToTalk,
            shortcut(ShortcutKind::PushToTalk, "Pause", "Ctrl+Alt+Pause"),
        );
        state.shortcuts.insert(
            ShortcutKind::Toggle,
            shortcut(ShortcutKind::Toggle, "F13", "Ctrl+Alt+F13"),
        );
        let (_, event_sink) = recorder();
        let mut calls = Vec::new();
        register_shortcuts_with(&mut state, &event_sink, |id, modifiers, vk| {
            calls.push((id, modifiers, vk));
            Ok(())
        });
        let modifiers = MOD_CONTROL | MOD_ALT | MOD_NOREPEAT;
        assert_eq!(
            calls,
            [
                (HOTKEY_PUSH_TO_TALK, modifiers, 0x13),
                (HOTKEY_TOGGLE, modifiers, 0x7C)
            ]
        );
    }

    #[test]
    fn lone_right_ctrl_push_to_talk_starts_stops_and_cancels() {
        let mut state = HotkeyThreadState::default();
        drain_set(
            &mut state,
            lone(ShortcutKind::PushToTalk, "ControlRight", "Right Ctrl"),
        );
        let (events, event_sink) = recorder();
        let ctrl = VK_RCONTROL as u32;
        let down = KEY_DOWN_BIT;
        handle_hook_message(&mut state, ctrl, down, 0, &event_sink);
        tick_gestures(&mut state, solo_key::HOLD_MS, &event_sink);
        handle_hook_message(&mut state, ctrl, 0, 300, &event_sink);
        handle_hook_message(&mut state, ctrl, down, 400, &event_sink);
        tick_gestures(&mut state, 400 + solo_key::HOLD_MS, &event_sink);
        handle_hook_message(&mut state, 0x43, down, 650, &event_sink); // C
        handle_hook_message(&mut state, 0x43, 0, 660, &event_sink);
        handle_hook_message(&mut state, ctrl, 0, 700, &event_sink);
        let names: Vec<&str> = events
            .lock()
            .expect("event lock")
            .iter()
            .map(|event| match event {
                HotkeyEvent::KeyDown { .. } => "down",
                HotkeyEvent::KeyUp { .. } => "up",
                HotkeyEvent::KeyCancel { .. } => "cancel",
                _ => "other",
            })
            .filter(|name| *name != "other")
            .collect();
        assert_eq!(names, ["down", "up", "down", "cancel"]);
    }

    #[test]
    fn each_hook_decision_is_reported_with_what_it_did() {
        let mut state = HotkeyThreadState::default();
        drain_set(
            &mut state,
            lone(ShortcutKind::PushToTalk, "ControlRight", "Right Ctrl"),
        );
        let (events, event_sink) = recorder();
        let ctrl = VK_RCONTROL as u32;
        handle_hook_message(&mut state, ctrl, KEY_DOWN_BIT, 0, &event_sink);
        tick_gestures(&mut state, solo_key::HOLD_MS, &event_sink);
        handle_hook_message(&mut state, ctrl, 0, 300, &event_sink);
        handle_hook_message(
            &mut state,
            VK_LCONTROL as u32,
            KEY_DOWN_BIT | KEY_ALTGR_BIT,
            310,
            &event_sink,
        );
        // An injected press is logged and does nothing.
        handle_hook_message(
            &mut state,
            ctrl,
            KEY_DOWN_BIT | KEY_INJECTED_BIT,
            400,
            &event_sink,
        );
        let lines: Vec<(&'static str, &'static str, Option<u64>)> = events
            .lock()
            .expect("event lock")
            .iter()
            .filter_map(|event| match event {
                HotkeyEvent::Diagnostic {
                    diagnostic, action, ..
                } => Some((diagnostic.decision, *action, diagnostic.held_ms)),
                _ => None,
            })
            .collect();
        assert_eq!(
            lines,
            [
                ("pending", "none", None),
                ("push_start", "push_start", Some(solo_key::HOLD_MS)),
                ("push_end", "push_end", Some(300)),
                ("ignored", "none", None),
            ]
        );
    }

    fn captured(events: &[HotkeyEvent]) -> Option<ShortcutCommand> {
        events.iter().find_map(|event| match event {
            HotkeyEvent::Captured { shortcut, .. } => Some(shortcut.clone()),
            _ => None,
        })
    }

    #[test]
    fn capture_takes_left_modifiers_alone_and_a_toggle_as_a_double_tap() {
        for (kind, presses, label) in [
            (ShortcutKind::PushToTalk, 1, "Left Ctrl"),
            (ShortcutKind::Toggle, 2, "Left Ctrl+Left Ctrl"),
        ] {
            let mut state = HotkeyThreadState {
                capture: Some(CaptureState { kind }),
                ..HotkeyThreadState::default()
            };
            let (events, event_sink) = recorder();
            let ctrl = VK_LCONTROL as u32;
            handle_hook_message(&mut state, ctrl, KEY_DOWN_BIT, 0, &event_sink);
            handle_hook_message(&mut state, ctrl, 0, 90, &event_sink);
            let shortcut = captured(&events.lock().expect("event lock")).expect("captured");
            assert_eq!(shortcut.code, "ControlLeft");
            assert_eq!(shortcut.press_count, presses);
            assert_eq!(shortcut.label, label);
        }
    }

    #[test]
    fn a_shared_left_ctrl_registers_without_a_system_hotkey() {
        let mut state = HotkeyThreadState::default();
        state.shortcuts.insert(
            ShortcutKind::PushToTalk,
            lone(ShortcutKind::PushToTalk, "ControlLeft", "Left Ctrl"),
        );
        state.shortcuts.insert(
            ShortcutKind::Toggle,
            ShortcutCommand {
                press_count: 2,
                ..lone(ShortcutKind::Toggle, "ControlLeft", "Left Ctrl+Left Ctrl")
            },
        );
        let (events, event_sink) = recorder();
        let mut calls = 0;
        register_shortcuts_with(&mut state, &event_sink, |_, _, _| {
            calls += 1;
            Ok(())
        });
        assert_eq!(calls, 0);
        assert!(events
            .lock()
            .expect("event lock")
            .iter()
            .any(|event| matches!(event, HotkeyEvent::Ready { .. })));
    }

    #[test]
    fn left_ctrl_c_never_starts_a_dictation() {
        let mut state = HotkeyThreadState::default();
        drain_set(
            &mut state,
            lone(ShortcutKind::PushToTalk, "ControlLeft", "Left Ctrl"),
        );
        let (events, event_sink) = recorder();
        let ctrl = VK_LCONTROL as u32;
        handle_hook_message(&mut state, ctrl, KEY_DOWN_BIT, 0, &event_sink);
        handle_hook_message(&mut state, 0x43, KEY_DOWN_BIT, 40, &event_sink);
        handle_hook_message(&mut state, 0x43, 0, 60, &event_sink);
        handle_hook_message(&mut state, ctrl, 0, 400, &event_sink);
        assert!(!events
            .lock()
            .expect("event lock")
            .iter()
            .any(|event| matches!(
                event,
                HotkeyEvent::KeyDown { .. }
                    | HotkeyEvent::KeyUp { .. }
                    | HotkeyEvent::KeyCancel { .. }
            )));
    }

    /// The hook's down and up of a quick Alt tap can both run before the
    /// thread consumes the first posted message: the hook decides alone.
    #[test]
    fn the_hook_masks_an_alt_tap_before_any_posted_message_is_handled() {
        let mut state = HotkeyThreadState::default();
        drain_set(
            &mut state,
            ShortcutCommand {
                press_count: 2,
                ..lone(ShortcutKind::Toggle, "AltLeft", "Left Alt+Left Alt")
            },
        );
        let alt = VK_LMENU as u32;
        let mask = |vk, down| {
            let edge = HookEdge {
                vk,
                down,
                injected: false,
                repeat: false,
                altgr: false,
            };
            hook_edge(edge, &|_| false).1
        };
        assert!(!mask(alt, true));
        assert!(mask(alt, false), "first tap");
        assert!(!mask(alt, true));
        assert!(mask(alt, false), "second tap");
        assert!(!mask(alt, true));
        assert!(!mask(VK_TAB as u32, true));
        assert!(!mask(VK_TAB as u32, false));
        assert!(!mask(alt, false), "Alt+Tab");
    }

    /// The review's order: C held at the Alt-down, C released before the
    /// Alt-down message is handled. The gesture and the mask must decide
    /// "alone" from the same hook snapshot: no tap counted, no mask.
    #[test]
    fn the_gesture_and_the_mask_share_the_hooks_snapshot() {
        let mut state = HotkeyThreadState::default();
        drain_set(
            &mut state,
            ShortcutCommand {
                press_count: 2,
                ..lone(ShortcutKind::Toggle, "AltLeft", "Left Alt+Left Alt")
            },
        );
        let (events, event_sink) = recorder();
        let (alt, c) = (VK_LMENU as u32, 0x43);
        let c_held = std::cell::Cell::new(true);
        let still_down = |vk: u32| vk == c && c_held.get();
        let edge = |vk, down| HookEdge {
            vk,
            down,
            injected: false,
            repeat: false,
            altgr: false,
        };
        let (c_down, _) = hook_edge(edge(c, true), &still_down);
        let (alt_down, _) = hook_edge(edge(alt, true), &still_down);
        c_held.set(false);
        let (c_up, _) = hook_edge(edge(c, false), &still_down);
        // Only now does the thread handle the posted messages.
        handle_hook_message(&mut state, c, c_down, 0, &event_sink);
        handle_hook_message(&mut state, alt, alt_down, 10, &event_sink);
        handle_hook_message(&mut state, c, c_up, 20, &event_sink);
        let (alt_up, masked) = hook_edge(edge(alt, false), &still_down);
        handle_hook_message(&mut state, alt, alt_up, 80, &event_sink);
        assert!(!masked, "not a lone tap: no mask");
        let decisions: Vec<&'static str> = events
            .lock()
            .expect("event lock")
            .iter()
            .filter_map(|event| match event {
                HotkeyEvent::Diagnostic { diagnostic, .. } if diagnostic.key == "AltLeft" => {
                    Some(diagnostic.decision)
                }
                _ => None,
            })
            .collect();
        assert_eq!(decisions, ["ignored", "released"], "not a tap either");

        // The next quick tap is a first tap: masked, and no toggle yet.
        let (down, _) = hook_edge(edge(alt, true), &still_down);
        handle_hook_message(&mut state, alt, down, 200, &event_sink);
        let (up, masked) = hook_edge(edge(alt, false), &still_down);
        handle_hook_message(&mut state, alt, up, 260, &event_sink);
        assert!(masked);
        assert!(!events
            .lock()
            .expect("event lock")
            .iter()
            .any(|event| matches!(event, HotkeyEvent::KeyDown { .. })));
    }

    /// A quick Ctrl tap, then Ctrl+C: copying, so no toggle and no indicator.
    #[test]
    fn a_tap_then_ctrl_c_never_toggles_dictation() {
        let mut state = HotkeyThreadState::default();
        drain_set(
            &mut state,
            lone(ShortcutKind::PushToTalk, "ControlLeft", "Left Ctrl"),
        );
        drain_set(
            &mut state,
            ShortcutCommand {
                press_count: 2,
                ..lone(ShortcutKind::Toggle, "ControlLeft", "Left Ctrl+Left Ctrl")
            },
        );
        let (events, event_sink) = recorder();
        let ctrl = VK_LCONTROL as u32;
        handle_hook_message(&mut state, ctrl, KEY_DOWN_BIT, 0, &event_sink);
        handle_hook_message(&mut state, ctrl, 0, 80, &event_sink);
        handle_hook_message(&mut state, ctrl, KEY_DOWN_BIT, 200, &event_sink);
        handle_hook_message(&mut state, 0x43, KEY_DOWN_BIT, 230, &event_sink);
        handle_hook_message(&mut state, 0x43, 0, 260, &event_sink);
        handle_hook_message(&mut state, ctrl, 0, 300, &event_sink);
        assert!(!events
            .lock()
            .expect("event lock")
            .iter()
            .any(|event| matches!(
                event,
                HotkeyEvent::KeyDown { .. }
                    | HotkeyEvent::KeyUp { .. }
                    | HotkeyEvent::KeyCancel { .. }
            )));
    }

    fn drain_set(state: &mut HotkeyThreadState, shortcut: ShortcutCommand) {
        let (tx, rx) = mpsc::channel();
        tx.send(HotkeyMessage::Set(shortcut)).unwrap();
        let (_, event_sink) = recorder();
        // Registration itself would call RegisterHotKey; a lone modifier
        // never does.
        drain_messages(&rx, state, &event_sink);
    }

    fn shortcut(kind: ShortcutKind, code: &str, label: &str) -> ShortcutCommand {
        ShortcutCommand {
            key_code: virtual_key_for_code(code).expect("supported shortcut"),
            code: code.to_string(),
            label: label.to_string(),
            kind,
            press_count: 1,
            modifiers: ShortcutModifiers {
                control: true,
                option: true,
                ..ShortcutModifiers::default()
            },
        }
    }
}
