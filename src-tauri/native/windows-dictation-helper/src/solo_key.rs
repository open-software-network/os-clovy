//! Lone-key shortcuts on Windows, as pure state machines over the low-level
//! keyboard hook's key events (no Win32 here, so the rules are testable
//! anywhere with `rustc --test`).
//!
//! A lone modifier (Ctrl, Alt or Shift of either side) is not a hotkey
//! Windows can register, and it must not get in the way of normal use:
//! Ctrl + C still copies, Ctrl + click still selects. It works like Fn on the
//! Mac helper: held alone past `HOLD_MS` it dictates until released; a double
//! tap toggles. Any other key, mouse button or wheel before that keeps it an
//! ordinary modifier, and input injected by software never counts.

use std::collections::HashSet;

/// Virtual-key codes the rules need.
pub const VK_LSHIFT: u32 = 0xA0;
pub const VK_RSHIFT: u32 = 0xA1;
pub const VK_LCONTROL: u32 = 0xA2;
pub const VK_RCONTROL: u32 = 0xA3;
pub const VK_LMENU: u32 = 0xA4;
pub const VK_RMENU: u32 = 0xA5;

/// One decision of the hook about a lone-modifier shortcut, for the
/// dictation event log. Never names a key other than a modifier: anything
/// else is `"other"`, so the log holds no typed content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// The modifier's DOM `code`, `"other"` or `"mouse"`.
    pub key: &'static str,
    /// `"down"`, `"up"` or `"click"`.
    pub edge: &'static str,
    pub injected: bool,
    /// The press went down with nothing else held.
    pub alone: bool,
    /// How long the shortcut key was held, on its release.
    pub held_ms: Option<u64>,
    pub decision: &'static str,
    pub reason: &'static str,
}

/// The DOM `code` of a modifier virtual key, `"other"` for any other key.
pub fn key_name(vk: u32) -> &'static str {
    match vk {
        VK_LSHIFT => "ShiftLeft",
        VK_RSHIFT => "ShiftRight",
        VK_LCONTROL => "ControlLeft",
        VK_RCONTROL => "ControlRight",
        VK_LMENU => "AltLeft",
        VK_RMENU => "AltRight",
        0x5B => "MetaLeft",
        0x5C => "MetaRight",
        _ => "other",
    }
}

/// How long a lone modifier must be held alone before push-to-talk opens the
/// microphone (the Mac helper's `holdThreshold`).
pub const HOLD_MS: u64 = 160;
/// Most time between a tap's release and the next press for a double tap
/// (the Mac helper's `doublePressWindow`).
pub const DOUBLE_TAP_MS: u64 = 340;

/// One input the hooks saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    /// A key edge; `repeat` is a down whose key was already down before it.
    /// `alone`: on a down, no other key was held at that moment, as the hook
    /// saw it (the one snapshot the gesture and the Alt mask both use).
    Key {
        vk: u32,
        down: bool,
        injected: bool,
        repeat: bool,
        alone: bool,
    },
    /// A mouse button press or a wheel turn.
    Mouse { injected: bool },
}

/// What a lone-modifier gesture asks of dictation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureEvent {
    PushDown,
    PushUp,
    /// The press became a chord after push-to-talk started: drop the take.
    PushCancel,
    Toggle,
}

/// What one input meant for a gesture, and why.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GestureStep {
    pub event: Option<GestureEvent>,
    pub diagnostic: Option<Diagnostic>,
}

/// Where the current press of the gesture's key stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The key is up.
    Idle,
    /// Down alone since `down_at`, not yet held long enough to dictate.
    Pending,
    /// Push-to-talk started and runs until the key goes up.
    Holding,
    /// The second press of a double tap: it toggles when released clean;
    /// a key, a click or the wheel before that keeps it ordinary.
    SecondTap,
    /// The press is spent (a chord, a key already held): nothing more
    /// happens until the key goes up.
    Spent,
}

/// The Mac Fn gesture on one Windows modifier.
#[derive(Clone, Debug)]
pub struct Gesture {
    target: u32,
    push: bool,
    toggle_presses: Option<u32>,
    phase: Phase,
    down_at: u64,
    /// The release of the last clean tap, for a double tap.
    last_tap_at: Option<u64>,
}

impl Gesture {
    /// `toggle_presses` is the toggle shortcut's press count when it uses the
    /// same key (1: a tap, 2: a double tap).
    pub fn new(target: u32, push: bool, toggle_presses: Option<u32>) -> Self {
        Self {
            target,
            push,
            toggle_presses,
            phase: Phase::Idle,
            down_at: 0,
            last_tap_at: None,
        }
    }

    pub fn target(&self) -> u32 {
        self.target
    }

    /// Whether holding the key is push-to-talk.
    pub fn pushes(&self) -> bool {
        self.push
    }

    /// Whether taps of the key toggle dictation.
    pub fn toggles(&self) -> bool {
        self.toggle_presses.is_some()
    }

    /// Feeds one input. Whether a press went down alone is the hook's
    /// snapshot (`Input::Key::alone`), never read again here: the Alt mask
    /// decided from that same snapshot at the moment of the edge.
    pub fn on_input(&mut self, input: Input, now_ms: u64) -> GestureStep {
        match input {
            Input::Key {
                vk,
                down,
                injected,
                repeat,
                alone,
            } if vk == self.target => {
                if injected {
                    let edge = if down { "down" } else { "up" };
                    return self.step(None, vk, edge, true, None, "ignored", "injected");
                }
                if down {
                    self.target_down(repeat, alone, now_ms)
                } else {
                    self.target_up(now_ms)
                }
            }
            Input::Key { injected: true, .. } | Input::Mouse { injected: true } => {
                GestureStep::default()
            }
            Input::Key {
                vk, down, repeat, ..
            } => {
                if !down || repeat {
                    return GestureStep::default();
                }
                self.chord(vk, "down", "other_key", now_ms)
            }
            Input::Mouse { .. } => self.chord(0, "click", "mouse", now_ms),
        }
    }

    /// Starts push-to-talk once the key has been held alone long enough.
    pub fn on_tick(&mut self, now_ms: u64) -> GestureStep {
        match self.deadline() {
            Some(deadline) if now_ms >= deadline => {
                self.phase = Phase::Holding;
                self.last_tap_at = None;
                let held = Some(now_ms.saturating_sub(self.down_at));
                self.step(
                    Some(GestureEvent::PushDown),
                    self.target,
                    "down",
                    false,
                    held,
                    "push_start",
                    "held",
                )
            }
            _ => GestureStep::default(),
        }
    }

    /// When `on_tick` has something to decide.
    pub fn deadline(&self) -> Option<u64> {
        (self.push && self.phase == Phase::Pending).then_some(self.down_at + HOLD_MS)
    }

    fn target_down(&mut self, repeat: bool, alone: bool, now_ms: u64) -> GestureStep {
        if repeat && self.phase != Phase::Idle {
            return GestureStep::default();
        }
        // A fresh press while the last one never saw its release: the
        // release was missed (a secure desktop, a lock screen). Start over.
        let missed_release = self.phase == Phase::Holding;
        self.down_at = now_ms;
        if missed_release {
            self.phase = if alone { Phase::Pending } else { Phase::Spent };
            return self.step(
                Some(GestureEvent::PushCancel),
                self.target,
                "down",
                true,
                None,
                "push_cancel",
                "missed_release",
            );
        }
        if !alone {
            self.phase = Phase::Spent;
            self.last_tap_at = None;
            return self.step(
                None,
                self.target,
                "down",
                false,
                None,
                "ignored",
                "other_key_held",
            );
        }
        let double_tap = self.toggle_presses == Some(2)
            && self
                .last_tap_at
                .is_some_and(|at| now_ms.saturating_sub(at) <= DOUBLE_TAP_MS);
        if double_tap {
            self.phase = Phase::SecondTap;
            self.last_tap_at = None;
            return self.step(
                None,
                self.target,
                "down",
                true,
                None,
                "second_tap",
                "double_tap",
            );
        }
        self.phase = Phase::Pending;
        self.step(None, self.target, "down", true, None, "pending", "alone")
    }

    fn target_up(&mut self, now_ms: u64) -> GestureStep {
        let phase = std::mem::replace(&mut self.phase, Phase::Idle);
        let held = Some(now_ms.saturating_sub(self.down_at));
        let line = |this: &Self, event, decision, reason| {
            this.step(event, this.target, "up", true, held, decision, reason)
        };
        match phase {
            Phase::Idle => self.step(
                None,
                self.target,
                "up",
                false,
                None,
                "ignored",
                "up_without_down",
            ),
            Phase::Holding => line(self, Some(GestureEvent::PushUp), "push_end", "released"),
            Phase::Spent => line(self, None, "released", "spent"),
            Phase::SecondTap => line(self, Some(GestureEvent::Toggle), "toggle", "double_tap"),
            Phase::Pending => match self.toggle_presses {
                Some(1) => line(self, Some(GestureEvent::Toggle), "toggle", "tap"),
                Some(2) => {
                    self.last_tap_at = Some(now_ms);
                    line(self, None, "tap", "first_of_double_tap")
                }
                _ => line(self, None, "tap", "short_hold"),
            },
        }
    }

    /// Another key, a mouse button or the wheel while the key is down.
    fn chord(
        &mut self,
        vk: u32,
        edge: &'static str,
        reason: &'static str,
        now_ms: u64,
    ) -> GestureStep {
        self.last_tap_at = None;
        let held = Some(now_ms.saturating_sub(self.down_at));
        let key = if reason == "mouse" {
            "mouse"
        } else {
            key_name(vk)
        };
        let (event, decision) = match self.phase {
            // Before the hold threshold: nothing was shown, nothing to undo.
            Phase::Pending | Phase::SecondTap => (None, "chord"),
            Phase::Holding => (Some(GestureEvent::PushCancel), "push_cancel"),
            Phase::Idle | Phase::Spent => return GestureStep::default(),
        };
        self.phase = Phase::Spent;
        GestureStep {
            event,
            diagnostic: Some(Diagnostic {
                key,
                edge,
                injected: false,
                alone: true,
                held_ms: held,
                decision,
                reason,
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step(
        &self,
        event: Option<GestureEvent>,
        vk: u32,
        edge: &'static str,
        alone: bool,
        held_ms: Option<u64>,
        decision: &'static str,
        reason: &'static str,
    ) -> GestureStep {
        let injected = reason == "injected";
        GestureStep {
            event,
            diagnostic: Some(Diagnostic {
                key: key_name(vk),
                edge,
                injected,
                alone: alone && !injected,
                held_ms,
                decision,
                reason,
            }),
        }
    }
}

/// The keys the user holds, kept by the keyboard hook edge by edge: the one
/// place that decides whether a press went down alone. The hook sends the
/// answer with each posted message, so the gestures (which hear of the edge
/// later) and the Alt mask (decided at once) agree.
#[derive(Clone, Debug, Default)]
pub struct KeySnapshot {
    down: HashSet<u32>,
}

impl KeySnapshot {
    /// Feeds one key edge the user made (not injected, not AltGr's synthetic
    /// Left Ctrl). On a down, whether no other key is held at this moment;
    /// keys whose release never reached the hook are dropped first, by
    /// asking Windows (`still_down`), so they cannot block every later press.
    pub fn on_key(&mut self, vk: u32, down: bool, still_down: &dyn Fn(u32) -> bool) -> bool {
        if !down {
            self.down.remove(&vk);
            return false;
        }
        self.down.retain(|key| *key == vk || still_down(*key));
        let alone = self.down.iter().all(|key| *key == vk);
        self.down.insert(vk);
        alone
    }
}

/// Decides, inside the keyboard hook, whether a release of the toggle's lone
/// Alt must be masked from the app's menu bar: the press went down alone (the
/// hook's [`KeySnapshot`]) and nothing (key, button, wheel) joined it. The
/// hotkey thread learns of the same events later, from posted messages, so
/// the hook cannot wait for it.
#[derive(Clone, Debug, Default)]
pub struct AltReleaseMask {
    target: Option<u32>,
    held: bool,
    /// The current press went down with nothing else held.
    alone: bool,
    /// A key, button or wheel turn joined the current press.
    chorded: bool,
}

impl AltReleaseMask {
    /// The Alt key the toggle gesture uses alone, if any.
    pub fn set_target(&mut self, target: Option<u32>) {
        *self = Self {
            target,
            ..Self::default()
        };
    }

    /// Feeds one key edge with the snapshot's `alone` for it; true when it
    /// is a release to mask.
    pub fn on_key(
        &mut self,
        vk: u32,
        down: bool,
        injected: bool,
        repeat: bool,
        alone: bool,
    ) -> bool {
        let Some(target) = self.target else {
            return false;
        };
        if injected {
            return false;
        }
        if vk == target {
            if down {
                if !(repeat && self.held) {
                    self.held = true;
                    self.alone = alone;
                    self.chorded = false;
                }
                return false;
            }
            let was_held = std::mem::replace(&mut self.held, false);
            return was_held && self.alone && !self.chorded;
        }
        if down && self.held && !repeat {
            self.chorded = true;
        }
        false
    }

    /// A mouse button press or wheel turn.
    pub fn on_mouse(&mut self, injected: bool) {
        if !injected && self.held {
            self.chorded = true;
        }
    }
}

/// What a finished capture that never formed a chord recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoloCapture {
    /// Ctrl, Alt or Shift (either side) pressed and released alone.
    Modifier(u32),
    /// The Windows key pressed and released alone: refused, it opens Start.
    Unsupported(u32),
}

/// Watches a shortcut capture for a lone modifier and for whether any key
/// reached Windows at all.
#[derive(Clone, Debug, Default)]
pub struct CaptureTracker {
    /// The modifier pressed first while nothing else was down.
    candidate: Option<u32>,
    /// Keys currently down.
    down: HashSet<u32>,
    saw_key: bool,
}

impl CaptureTracker {
    /// Feeds one key edge. Returns a lone-modifier capture when the first
    /// modifier pressed alone is released with no other key in between.
    pub fn on_key(&mut self, vk: u32, is_down: bool, is_modifier: bool) -> Option<SoloCapture> {
        self.saw_key = true;
        if is_down {
            let first = self.down.insert(vk);
            if !first {
                return None; // auto-repeat
            }
            if is_modifier && self.down.len() == 1 {
                self.candidate = Some(vk);
            } else {
                self.candidate = None;
            }
            return None;
        }
        self.down.remove(&vk);
        if self.candidate != Some(vk) {
            return None;
        }
        self.candidate = None;
        Some(if is_lone_modifier(vk) {
            SoloCapture::Modifier(vk)
        } else {
            SoloCapture::Unsupported(vk)
        })
    }

    /// Whether any key event reached the hook during the capture.
    pub fn saw_any_key(&self) -> bool {
        self.saw_key
    }
}

fn is_lone_modifier(vk: u32) -> bool {
    matches!(
        vk,
        VK_LCONTROL | VK_RCONTROL | VK_LMENU | VK_RMENU | VK_LSHIFT | VK_RSHIFT
    )
}

/// Lone keys: virtual key, DOM `code`, label.
const SOLO_KEYS: [(u32, &str, &str); 23] = [
    (VK_LCONTROL, "ControlLeft", "Left Ctrl"),
    (VK_RCONTROL, "ControlRight", "Right Ctrl"),
    (VK_LMENU, "AltLeft", "Left Alt"),
    (VK_RMENU, "AltRight", "Right Alt"),
    (VK_LSHIFT, "ShiftLeft", "Left Shift"),
    (VK_RSHIFT, "ShiftRight", "Right Shift"),
    (0x7C, "F13", "F13"),
    (0x7D, "F14", "F14"),
    (0x7E, "F15", "F15"),
    (0x7F, "F16", "F16"),
    (0x80, "F17", "F17"),
    (0x81, "F18", "F18"),
    (0x82, "F19", "F19"),
    (0x83, "F20", "F20"),
    (0x84, "F21", "F21"),
    (0x85, "F22", "F22"),
    (0x86, "F23", "F23"),
    (0x87, "F24", "F24"),
    (0x13, "Pause", "Pause"),
    (0x91, "ScrollLock", "Scroll Lock"),
    (0x2D, "Insert", "Insert"),
    (0x2C, "PrintScreen", "Print Screen"),
    (0x5D, "ContextMenu", "Menu"),
];

/// The DOM `code` of a key that works as a shortcut on its own, and a label.
pub fn solo_code(vk: u32) -> Option<(&'static str, &'static str)> {
    SOLO_KEYS
        .iter()
        .find(|(key, _, _)| *key == vk)
        .map(|(_, code, label)| (*code, *label))
}

/// The virtual key of a lone-key `code`.
pub fn solo_vk(code: &str) -> Option<u32> {
    SOLO_KEYS
        .iter()
        .find(|(_, known, _)| *known == code)
        .map(|(vk, _, _)| *vk)
}

/// Whether `code` names a lone modifier (handled by the hook, not
/// `RegisterHotKey`).
pub fn is_solo_modifier_code(code: &str) -> bool {
    solo_vk(code).is_some_and(is_lone_modifier)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_C: u32 = 0x43;

    #[test]
    fn a_key_already_held_keeps_the_press_ordinary() {
        let mut gesture = Gesture::new(VK_RCONTROL, true, None);
        let mut events = play(
            &mut gesture,
            &[
                (0, Some(key(VK_LSHIFT, true))),
                (10, Some(key_not_alone(VK_RCONTROL))),
            ],
        );
        events.extend(play(
            &mut gesture,
            &[
                (10 + HOLD_MS, None),
                (400, Some(key(VK_RCONTROL, false))),
                (410, Some(key(VK_LSHIFT, false))),
            ],
        ));
        assert_eq!(events, []);
        // Released everything: a later lone press counts again.
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (1_000, Some(key(VK_RCONTROL, true))),
                    (1_000 + HOLD_MS, None)
                ]
            ),
            [GestureEvent::PushDown]
        );
    }

    #[test]
    fn other_keys_alone_mean_nothing() {
        let mut gesture = Gesture::new(VK_RMENU, true, Some(2));
        let steps = [
            gesture.on_input(key(KEY_C, true), 0),
            gesture.on_input(key(KEY_C, false), 10),
            gesture.on_input(Input::Mouse { injected: false }, 20),
        ];
        assert!(steps.iter().all(|step| *step == GestureStep::default()));
    }

    #[test]
    fn capture_records_a_lone_right_modifier() {
        let mut capture = CaptureTracker::default();
        assert!(!capture.saw_any_key());
        assert_eq!(capture.on_key(VK_RCONTROL, true, true), None);
        assert_eq!(
            capture.on_key(VK_RCONTROL, false, true),
            Some(SoloCapture::Modifier(VK_RCONTROL))
        );
        assert!(capture.saw_any_key());
    }

    #[test]
    fn capture_records_ctrl_alt_and_shift_of_either_side() {
        for vk in [
            VK_LCONTROL,
            VK_RCONTROL,
            VK_LMENU,
            VK_RMENU,
            VK_LSHIFT,
            VK_RSHIFT,
        ] {
            let mut capture = CaptureTracker::default();
            capture.on_key(vk, true, true);
            assert_eq!(
                capture.on_key(vk, false, true),
                Some(SoloCapture::Modifier(vk)),
                "{vk:#x}"
            );
        }
    }

    #[test]
    fn capture_does_not_take_the_windows_key_alone() {
        let mut capture = CaptureTracker::default();
        capture.on_key(VK_LWIN, true, true);
        assert_ne!(
            capture.on_key(VK_LWIN, false, true),
            Some(SoloCapture::Modifier(VK_LWIN))
        );
    }

    const VK_LWIN: u32 = 0x5B;
    const NOTHING_HELD: fn(u32) -> bool = |_| false;

    fn key(vk: u32, down: bool) -> Input {
        Input::Key {
            vk,
            down,
            injected: false,
            repeat: false,
            alone: true,
        }
    }

    /// A down the hook saw while another key was held.
    fn key_not_alone(vk: u32) -> Input {
        Input::Key {
            vk,
            down: true,
            injected: false,
            repeat: false,
            alone: false,
        }
    }

    fn injected(vk: u32, down: bool) -> Input {
        Input::Key {
            vk,
            down,
            injected: true,
            repeat: false,
            alone: true,
        }
    }

    /// The hook's snapshot decides "alone", not a later look at which keys
    /// are down: C held at the Alt-down, released before the gesture hears
    /// of that down, still leaves the press ordinary.
    #[test]
    fn the_press_is_alone_only_if_the_hook_saw_it_alone() {
        let mut gesture = Gesture::new(VK_LMENU, true, Some(2));
        gesture.on_input(key(KEY_C, true), 0);
        let down = gesture
            .on_input(key_not_alone(VK_LMENU), 10)
            .diagnostic
            .expect("logged");
        assert_eq!(down.reason, "other_key_held");
        gesture.on_input(key(KEY_C, false), 20);
        let mut events = play(
            &mut gesture,
            &[(10 + HOLD_MS, None), (100, Some(key(VK_LMENU, false)))],
        );
        // And the next quick tap is a first tap, not a second.
        events.extend(play(
            &mut gesture,
            &[
                (200, Some(key(VK_LMENU, true))),
                (260, Some(key(VK_LMENU, false))),
            ],
        ));
        assert_eq!(events, []);
    }

    /// Feeds timed inputs (`None` is a timer tick) and collects the events.
    fn play(gesture: &mut Gesture, script: &[(u64, Option<Input>)]) -> Vec<GestureEvent> {
        script
            .iter()
            .filter_map(|(at, input)| match input {
                Some(input) => gesture.on_input(*input, *at).event,
                None => gesture.on_tick(*at).event,
            })
            .collect()
    }

    #[test]
    fn holding_ctrl_alone_dictates_while_held() {
        for target in [VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RSHIFT] {
            let mut gesture = Gesture::new(target, true, None);
            assert_eq!(play(&mut gesture, &[(0, Some(key(target, true)))]), []);
            assert_eq!(gesture.deadline(), Some(HOLD_MS));
            assert_eq!(play(&mut gesture, &[(HOLD_MS - 1, None)]), []);
            assert_eq!(
                play(
                    &mut gesture,
                    &[(HOLD_MS, None), (2_000, Some(key(target, false)))]
                ),
                [GestureEvent::PushDown, GestureEvent::PushUp],
                "{target:#x}"
            );
        }
    }

    #[test]
    fn ctrl_c_click_and_wheel_never_open_the_microphone() {
        let chords: [&[(u64, Option<Input>)]; 3] = [
            &[(40, Some(key(KEY_C, true))), (60, Some(key(KEY_C, false)))],
            &[(40, Some(Input::Mouse { injected: false }))],
            &[(90, Some(Input::Mouse { injected: false }))],
        ];
        for chord in chords {
            let mut gesture = Gesture::new(VK_LCONTROL, true, Some(2));
            play(&mut gesture, &[(0, Some(key(VK_LCONTROL, true)))]);
            let mut events = play(&mut gesture, chord);
            events.extend(play(
                &mut gesture,
                &[
                    (HOLD_MS, None),
                    (400, None),
                    (500, Some(key(VK_LCONTROL, false))),
                ],
            ));
            assert_eq!(events, [], "{chord:?}");
            assert_eq!(gesture.deadline(), None);
        }
    }

    #[test]
    fn a_chord_after_the_start_drops_the_take() {
        let mut gesture = Gesture::new(VK_LCONTROL, true, None);
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (0, Some(key(VK_LCONTROL, true))),
                    (HOLD_MS, None),
                    (300, Some(key(KEY_C, true))),
                    (320, Some(key(KEY_C, false))),
                    (400, Some(key(VK_LCONTROL, false))),
                ]
            ),
            [GestureEvent::PushDown, GestureEvent::PushCancel]
        );
    }

    #[test]
    fn injected_input_is_not_the_users_gesture() {
        let mut gesture = Gesture::new(VK_LCONTROL, true, Some(2));
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (0, Some(injected(VK_LCONTROL, true))),
                    (HOLD_MS, None),
                    (300, Some(injected(VK_LCONTROL, false))),
                ]
            ),
            []
        );
        let line = gesture
            .on_input(injected(VK_LCONTROL, true), 400)
            .diagnostic
            .expect("an injected press is logged");
        assert!(line.injected);
        assert_eq!(line.reason, "injected");
        gesture.on_input(injected(VK_LCONTROL, false), 410);

        // An injected key (a paste) while the user holds Ctrl is no chord.
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (1_000, Some(key(VK_LCONTROL, true))),
                    (1_000 + HOLD_MS, None),
                    (1_300, Some(injected(0x56, true))),
                    (1_310, Some(injected(0x56, false))),
                    (1_320, Some(Input::Mouse { injected: true })),
                    (2_000, Some(key(VK_LCONTROL, false))),
                ]
            ),
            [GestureEvent::PushDown, GestureEvent::PushUp]
        );
    }

    #[test]
    fn a_key_whose_release_was_missed_does_not_block_the_next_press() {
        let mut gesture = Gesture::new(VK_LCONTROL, true, None);
        // C went down and its release never reached the hook; the hook's
        // snapshot dropped it (see `the_snapshot_forgets_keys_windows_no_longer_holds`).
        play(&mut gesture, &[(0, Some(key(KEY_C, true)))]);
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (1_000, Some(key(VK_LCONTROL, true))),
                    (1_000 + HOLD_MS, None),
                    (2_000, Some(key(VK_LCONTROL, false))),
                ],
            ),
            [GestureEvent::PushDown, GestureEvent::PushUp]
        );

        // Ctrl's own release was missed: a fresh press starts over.
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (3_000, Some(key(VK_LCONTROL, true))),
                    (3_000 + HOLD_MS, None)
                ]
            ),
            [GestureEvent::PushDown]
        );
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (9_000, Some(key(VK_LCONTROL, true))),
                    (9_000 + HOLD_MS, None)
                ]
            ),
            [GestureEvent::PushCancel, GestureEvent::PushDown]
        );
    }

    #[test]
    fn auto_repeat_is_not_a_new_press() {
        let mut gesture = Gesture::new(VK_LCONTROL, true, None);
        let repeat = Input::Key {
            vk: VK_LCONTROL,
            down: true,
            injected: false,
            repeat: true,
            alone: true,
        };
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (0, Some(key(VK_LCONTROL, true))),
                    (HOLD_MS, None),
                    (500, Some(repeat)),
                    (530, Some(repeat)),
                    (900, Some(key(VK_LCONTROL, false))),
                ]
            ),
            [GestureEvent::PushDown, GestureEvent::PushUp]
        );
    }

    #[test]
    fn a_double_tap_toggles_like_fn_on_the_mac() {
        let mut gesture = Gesture::new(VK_LCONTROL, true, Some(2));
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (0, Some(key(VK_LCONTROL, true))),
                    (90, Some(key(VK_LCONTROL, false))),
                    (300, Some(key(VK_LCONTROL, true))),
                    (300 + HOLD_MS, None),
                    (600, Some(key(VK_LCONTROL, false))),
                ]
            ),
            [GestureEvent::Toggle]
        );
        // Too slow: the second press is a fresh hold.
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (5_000, Some(key(VK_LCONTROL, true))),
                    (5_090, Some(key(VK_LCONTROL, false))),
                    (5_090 + DOUBLE_TAP_MS + 1, Some(key(VK_LCONTROL, true))),
                    (5_090 + DOUBLE_TAP_MS + 1 + HOLD_MS, None),
                ]
            ),
            [GestureEvent::PushDown]
        );
    }

    /// The toggle fires when the second tap is released clean, never on its
    /// press: a quick Ctrl tap followed by Ctrl+C is copying, not dictating.
    #[test]
    fn a_double_tap_toggles_only_on_a_clean_second_release() {
        let mut gesture = Gesture::new(VK_LCONTROL, true, Some(2));
        let steps: Vec<Option<GestureEvent>> = [
            (0, key(VK_LCONTROL, true)),
            (90, key(VK_LCONTROL, false)),
            (300, key(VK_LCONTROL, true)),
            (390, key(VK_LCONTROL, false)),
        ]
        .into_iter()
        .map(|(at, input)| gesture.on_input(input, at).event)
        .collect();
        assert_eq!(steps, [None, None, None, Some(GestureEvent::Toggle)]);

        let chords: [Input; 3] = [
            key(KEY_C, true),
            Input::Mouse { injected: false },
            Input::Mouse { injected: false },
        ];
        for (offset, chord) in chords.into_iter().enumerate() {
            let start = 10_000 * (offset as u64 + 1);
            let mut events = play(
                &mut gesture,
                &[
                    (start, Some(key(VK_LCONTROL, true))),
                    (start + 80, Some(key(VK_LCONTROL, false))),
                    (start + 200, Some(key(VK_LCONTROL, true))),
                    (start + 230, Some(chord)),
                    (start + 200 + HOLD_MS, None),
                    (start + 400, Some(key(VK_LCONTROL, false))),
                ],
            );
            if chord == key(KEY_C, true) {
                events.extend(play(
                    &mut gesture,
                    &[(start + 410, Some(key(KEY_C, false)))],
                ));
            }
            assert_eq!(events, [], "{chord:?}");
        }
    }

    fn alt_edges(mask: &mut AltReleaseMask, edges: &[(u32, bool)]) -> Vec<bool> {
        edges
            .iter()
            .map(|(vk, down)| mask.on_key(*vk, *down, false, false, true))
            .collect()
    }

    #[test]
    fn the_snapshot_says_whether_a_press_went_down_alone() {
        let mut keys = KeySnapshot::default();
        let held = |vk: u32| vk == KEY_C;
        assert!(keys.on_key(VK_LMENU, true, &held));
        keys.on_key(VK_LMENU, false, &held);
        assert!(keys.on_key(KEY_C, true, &held));
        assert!(!keys.on_key(VK_LMENU, true, &held), "C is held");
        keys.on_key(KEY_C, false, &held);
        keys.on_key(VK_LMENU, false, &held);
        assert!(keys.on_key(VK_LMENU, true, &held), "C was released");
        // Auto-repeat of the same key is not another key.
        assert!(keys.on_key(VK_LMENU, true, &held));
    }

    #[test]
    fn the_snapshot_forgets_keys_windows_no_longer_holds() {
        let mut keys = KeySnapshot::default();
        keys.on_key(KEY_C, true, &NOTHING_HELD); // its release never came
        assert!(keys.on_key(VK_LCONTROL, true, &NOTHING_HELD));
    }

    #[test]
    fn the_hook_masks_a_lone_alt_tap_by_itself() {
        let mut mask = AltReleaseMask::default();
        mask.set_target(Some(VK_LMENU));
        assert_eq!(
            alt_edges(&mut mask, &[(VK_LMENU, true), (VK_LMENU, false)]),
            [false, true]
        );
        // Again, for the second tap of a double tap.
        assert_eq!(
            alt_edges(&mut mask, &[(VK_LMENU, true), (VK_LMENU, false)]),
            [false, true]
        );
        // Auto-repeat while held changes nothing.
        assert!(!mask.on_key(VK_LMENU, true, false, false, true));
        assert!(!mask.on_key(VK_LMENU, true, false, true, false));
        assert!(mask.on_key(VK_LMENU, false, false, false, false));
    }

    #[test]
    fn alt_chords_reach_the_app_unmasked() {
        let mut mask = AltReleaseMask::default();
        mask.set_target(Some(VK_LMENU));
        // Alt+Tab.
        assert_eq!(
            alt_edges(
                &mut mask,
                &[
                    (VK_LMENU, true),
                    (0x09, true),
                    (0x09, false),
                    (VK_LMENU, false)
                ]
            ),
            [false, false, false, false]
        );
        // Alt + click.
        mask.on_key(VK_LMENU, true, false, false, true);
        mask.on_mouse(false);
        assert!(!mask.on_key(VK_LMENU, false, false, false, false));
        // A key already held when Alt went down (the snapshot said so).
        mask.on_key(VK_LMENU, true, false, false, false);
        assert!(!mask.on_key(VK_LMENU, false, false, false, false));
    }

    #[test]
    fn injected_input_and_stale_keys_do_not_decide_the_mask() {
        let mut mask = AltReleaseMask::default();
        mask.set_target(Some(VK_RMENU));
        // An injected Alt (or the mask key itself) is never masked.
        assert!(!mask.on_key(VK_RMENU, true, true, false, true));
        assert!(!mask.on_key(VK_RMENU, false, true, false, false));
        // Injected keys and clicks during a real press are no chord.
        mask.on_key(VK_RMENU, true, false, false, true);
        mask.on_key(0xE8, true, true, false, true);
        mask.on_key(0xE8, false, true, false, false);
        mask.on_mouse(true);
        assert!(mask.on_key(VK_RMENU, false, false, false, false));
        // No target: nothing is masked.
        mask.set_target(None);
        assert_eq!(
            alt_edges(&mut mask, &[(VK_RMENU, true), (VK_RMENU, false)]),
            [false, false]
        );
    }

    #[test]
    fn a_single_tap_toggle_fires_on_a_clean_release() {
        let mut gesture = Gesture::new(VK_RSHIFT, false, Some(1));
        assert_eq!(
            play(
                &mut gesture,
                &[
                    (0, Some(key(VK_RSHIFT, true))),
                    (80, Some(key(VK_RSHIFT, false))),
                    (500, Some(key(VK_RSHIFT, true))),
                    (520, Some(key(KEY_C, true))),
                    (600, Some(key(VK_RSHIFT, false))),
                ]
            ),
            [GestureEvent::Toggle]
        );
    }

    #[test]
    fn every_gesture_decision_is_logged_without_typed_keys() {
        let mut gesture = Gesture::new(VK_LCONTROL, true, None);
        let mut lines = Vec::new();
        for (at, input) in [
            (0, Some(key(VK_LCONTROL, true))),
            (HOLD_MS, None),
            (300, Some(key(KEY_C, true))),
            (400, Some(key(VK_LCONTROL, false))),
        ] {
            let step = match input {
                Some(input) => gesture.on_input(input, at),
                None => gesture.on_tick(at),
            };
            lines.extend(step.diagnostic);
        }
        let decisions: Vec<_> = lines.iter().map(|line| line.decision).collect();
        assert_eq!(
            decisions,
            ["pending", "push_start", "push_cancel", "released"]
        );
        assert_eq!(lines[2].key, "other");
        assert_eq!(lines[3].held_ms, Some(400));
    }

    #[test]
    fn capture_of_a_chord_is_not_a_lone_modifier() {
        let mut capture = CaptureTracker::default();
        capture.on_key(VK_RCONTROL, true, true);
        capture.on_key(KEY_C, true, false);
        capture.on_key(KEY_C, false, false);
        assert_eq!(capture.on_key(VK_RCONTROL, false, true), None);
    }

    #[test]
    fn a_press_with_a_key_held_and_a_short_tap_say_why() {
        let mut gesture = Gesture::new(VK_RCONTROL, true, None);
        gesture.on_input(key(VK_LSHIFT, true), 0);
        let held = gesture
            .on_input(key_not_alone(VK_RCONTROL), 10)
            .diagnostic
            .expect("logged");
        assert!(!held.alone);
        assert_eq!(held.reason, "other_key_held");
        gesture.on_input(key(VK_RCONTROL, false), 20);
        gesture.on_input(key(VK_LSHIFT, false), 30);

        gesture.on_input(key(VK_RCONTROL, true), 1_000);
        let tap = gesture
            .on_input(key(VK_RCONTROL, false), 1_090)
            .diagnostic
            .expect("logged");
        assert_eq!((tap.decision, tap.reason), ("tap", "short_hold"));
        assert_eq!(tap.held_ms, Some(90));
    }

    #[test]
    fn modifier_names_are_codes_and_other_keys_are_anonymous() {
        assert_eq!(key_name(VK_LCONTROL), "ControlLeft");
        assert_eq!(key_name(VK_RMENU), "AltRight");
        assert_eq!(key_name(KEY_C), "other");
    }

    #[test]
    fn lone_keys_round_trip() {
        for (vk, code) in [
            (VK_RCONTROL, "ControlRight"),
            (VK_RMENU, "AltRight"),
            (VK_RSHIFT, "ShiftRight"),
            (0x7C, "F13"),
            (0x87, "F24"),
            (0x13, "Pause"),
            (0x91, "ScrollLock"),
            (0x2D, "Insert"),
        ] {
            assert_eq!(solo_code(vk).map(|(code, _)| code), Some(code));
            assert_eq!(solo_vk(code), Some(vk), "{code}");
        }
        assert_eq!(
            solo_code(VK_RCONTROL).map(|(_, label)| label),
            Some("Right Ctrl")
        );
        assert_eq!(solo_code(0x44), None, "D needs a modifier");
        assert_eq!(solo_code(VK_LCONTROL), Some(("ControlLeft", "Left Ctrl")));
        assert_eq!(solo_code(VK_LMENU), Some(("AltLeft", "Left Alt")));
        assert_eq!(solo_code(VK_LSHIFT), Some(("ShiftLeft", "Left Shift")));
        assert_eq!(solo_code(VK_LWIN), None);
        assert!(is_solo_modifier_code("AltRight"));
        assert!(is_solo_modifier_code("ControlLeft"));
        assert!(!is_solo_modifier_code("F13"));
    }
}
