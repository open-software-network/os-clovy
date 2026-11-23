---
status: accepted
date: 2026-07-10
---

# Windows dictation uses a hybrid global hotkey and keyboard hook

Windows dictation shortcuts use `RegisterHotKey` for shortcut reservation and
initial key-down activation, plus a narrowly scoped `WH_KEYBOARD_LL` hook for
push-to-talk release detection and interactive shortcut capture.

The Windows v1 shortcut contract supports a modifier plus one non-modifier
physical key. Supported modifier labels are `Ctrl`, `Alt`, `Shift`, and `Win`.
The helper rejects bare keys, modifier-only chords, `Fn`, and `pressCount`
values other than `1`. Windows shortcut identity is based on the DOM `code`
and corresponding virtual-key mapping, not a localized character.

The helper emits both `shortcut_key_down` and `shortcut_key_up` for a
push-to-talk shortcut. Toggle shortcuts activate from `shortcut_key_down` only.
The hook does not replace `RegisterHotKey`: reservation remains authoritative
so conflicts with shortcuts owned by Windows or another application can be
reported instead of silently accepting an unusable setting.

## Why

The Rust dictation activation controller models push-to-talk as a held chord.
It starts recording on key-down and stops or discards on key-up. Windows
`RegisterHotKey` only posts `WM_HOTKEY` on activation and does not report the
release edge, so using it alone can start a push-to-talk recording that never
stops.

Changing Windows to toggle-only behavior would avoid the native hook, but it
would remove the default push-to-talk interaction and require a different
capability and settings contract from macOS. A low-level keyboard hook supplies
the missing release edge while preserving the existing platform-neutral Rust
state machine.

Using a hook for every activation was also rejected. `RegisterHotKey` provides
operating-system conflict detection and reserves configured shortcuts without
requiring June to inspect the full keyboard stream. The hybrid design limits
the hook's responsibility and the key state it retains.

The same narrow hook can capture the next supported chord when Settings asks
the helper to change a shortcut. Keeping capture in the helper ensures that
capture validation, registration, and activation share one Windows allowlist.

## Alternatives considered

- **Support toggle only on Windows.** Rejected because it removes
  push-to-talk, diverges from the shared interaction model, and leaves the
  existing default shortcut unsafe unless product capabilities and settings
  are also changed.
- **Use only `WH_KEYBOARD_LL`.** Rejected because June would have to reproduce
  shortcut reservation and conflict semantics while observing more keyboard
  activity than necessary.
- **Poll key state after `WM_HOTKEY`.** Rejected because polling introduces a
  timing loop, can miss short releases, and provides no clean mechanism for
  shortcut capture.
- **Support modifier-only or `Fn` shortcuts in v1.** Rejected because these
  cannot use the same `RegisterHotKey` reservation path. They would require
  hook-only activation with weaker conflict behavior and platform-specific
  semantics.

## Consequences

- The helper installs the keyboard hook only on its native hotkey thread and
  keeps minimal state for the configured push-to-talk chord and active capture.
- The helper must not log raw keys or dictated text, and must not swallow normal
  keyboard input. It may suppress a supported chord only while shortcut
  capture is active.
- Key repeat must not produce duplicate shortcut edges.
- Reconfiguration and shutdown must release an active push-to-talk chord so
  Rust cannot remain in a held state after helper state is torn down.
- Authenticode signing and Windows QA are important because low-level keyboard
  hooks can affect antivirus and reputation checks.
- A shortcut is not ready merely because it was saved. The helper emits
  readiness only after registration succeeds and emits a recoverable
  registration failure when Windows refuses the chord.

## Addendum 2026-10-23: lone keys, Right Ctrl default, helper-only capture

Supersedes the v1 rejection of bare keys and modifier-only shortcuts for a
narrow set, by the owner's decision after v0.1.0 on Windows (Fn does not reach
Windows on most keyboards, so the Mac habit of a lone dictation key needs a
key Windows does see). `Fn` stays refused.

- **Lone right-hand modifiers** (`ControlRight`, `AltRight`, `ShiftRight`) are
  watched by the existing hook, not `RegisterHotKey`, which cannot reserve a
  modifier alone. They count only when pressed and released with no other key
  in between (`solo_key::SoloModifier`): Right Ctrl + C keeps copying.
  Push-to-talk starts on the lone press, stops on its release, and a key that
  joins while it is held sends `shortcut_key_cancel`, which discards the take
  instead of pasting it. Toggle fires on the clean release. AltGr's synthetic
  Left Ctrl (scan code 0x21D) is ignored. Left-hand modifiers alone stay
  refused: they are in constant use. These shortcuts get no system conflict
  detection, the trade-off this ADR named; the chord rule keeps them out of the
  way of normal typing.
- **Lone keys no app uses on their own** (`F13` to `F24`, `Pause`,
  `ScrollLock`, `Insert`, `PrintScreen`, `ContextMenu`) keep the reservation
  path: `RegisterHotKey` with no modifier, so conflicts are still reported.
- **Default:** Windows push-to-talk is Right Ctrl. A shortcut already saved
  is kept; only installs without one get the new default.
- **Capture has one source, the helper's hook.** Settings no longer reads the
  same keys from the DOM on Windows: the DOM cannot tell the right modifier
  from the left or see AltGr reliably, and the two sources raced (the DOM
  path cancelled the helper's capture, and the cancel cleared the row's
  error). A capture that times out with no key event at all reports
  `no_key_detected`, shown as "that key doesn't reach Windows" with Right Ctrl
  suggested. Capture and registration errors stay on the row until the user's
  next action.

## Addendum 2026-10-23 (v0.1.2): either side, the Mac Fn gesture, a mouse hook

Supersedes the lone-modifier rules of the addendum above, by the owner's
decision after v0.1.1: only Right Ctrl could be recorded, and even it did not
start dictation reliably (a tap shorter than the app's 160 ms graze filter
was discarded, and a key whose release the hook missed made every later press
look like a chord).

- **Ctrl, Alt and Shift of either side** work alone (`ControlLeft`,
  `AltLeft`, `ShiftLeft` join the right-hand keys). The Windows key alone
  stays refused: it opens Start.
- **The gesture is Fn's on the Mac helper** (`solo_key::Gesture`): held alone
  for 160 ms (`HOLD_MS`, the Mac `holdThreshold`) push-to-talk starts and runs
  until release; a double tap within 340 ms (`DOUBLE_TAP_MS`, the Mac
  `doublePressWindow`) toggles when the second tap is released clean (the
  Mac fires on the press; on Windows a key, click or wheel turn during the
  second press, as in a quick Ctrl tap and then Ctrl + C, must leave it
  ordinary). Push-to-talk no longer starts on the down
  edge: the short wait is what keeps Ctrl + C, Ctrl + click and Ctrl + wheel
  from opening the microphone or showing the indicator. A chord after the
  start sends `shortcut_key_cancel` and the take is discarded.
- **A lone modifier on the toggle is a double tap** (`pressCount: 2`,
  labelled `Left Ctrl+Left Ctrl`), so push-to-talk and the toggle can share
  one key as Fn and Fn+Fn do on the Mac. Double taps exist on Windows only for
  these keys; every other shortcut keeps `pressCount: 1`.
- **A low-level mouse hook** (`WH_MOUSE_LL`) reports button presses and wheel
  turns, nothing else (no position), so a click or a wheel turn spends the
  press like a key. Its procedure only posts a message, as the keyboard hook
  does, so it adds no work to pointer moves.
- **Injected input never counts** (`LLKHF_INJECTED`, `LLMHF_INJECTED`): not
  as a press, and not as a chord (Clovy's own paste is injected).
- **Missed releases do not wedge the key**: auto-repeat is read in the hook,
  where the async key state still holds the previous state, instead of
  inferred from a set of pressed keys; other keys are checked against
  `GetAsyncKeyState` on each new press; a fresh press while push-to-talk was
  still running cancels that take and starts over.
- **A lone Alt that dictated** gets an unassigned key (VK 0xE8) tapped while
  held, so its release does not open the focused app's menu bar. So does
  every Alt release the toggle gesture counts (each tap of a double tap, a
  single-tap toggle): the keyboard hook procedure taps the mask key itself,
  before passing the release on, from a target the hotkey thread publishes
  (`MENU_MASK_ON_RELEASE`). A chord (Alt+Tab) spends the gesture and clears
  the target; a plain Alt tap with no toggle on Alt still opens the menu.
- **Every decision is logged**: the hook sends `shortcut_trigger_diagnostic`
  (key as a modifier code or `other`, edge, injected, alone, held time,
  decision, reason) and the app logs what it did with each edge
  (`shortcut_edge`), both in `dictation-events.log` only, never the HUD.

## Addendum 2026-10-23 (v0.1.3): the hook decides the Alt mask itself

Supersedes how the v0.1.2 addendum above masks toggle-Alt releases. The
hotkey thread published `MENU_MASK_ON_RELEASE` only after handling the
posted message of the key-down; Windows calls a low-level hook before the
thread's posted messages, so a quick tap could run the hook's down and up
first and reach the app unmasked. The hook procedures now keep their own
view (`solo_key::AltReleaseMask`, thread-local on the hotkey thread), fed by
every keyboard and mouse hook event as it happens: a release of the toggle's
Alt is masked when that press went down alone (keys held before it are
checked with `GetAsyncKeyState`) and no key, button or wheel turn joined it.
Injected input is ignored; a press long enough to start push-to-talk is
masked as well, which changes nothing since push-to-talk already masked it.

Update the same day: the gestures read "alone" from that hook view too. The
keyboard hook keeps the keys held (`solo_key::KeySnapshot`, keys whose
release never came dropped with `GetAsyncKeyState` at the edge), decides
whether each down went down alone, masks with that answer and sends it in
the posted message (`KEY_NOT_ALONE_BIT`). `Gesture` no longer looks at live
key state, so the mask and the gesture cannot disagree when a key is
released between the hook call and the handling of its message.
