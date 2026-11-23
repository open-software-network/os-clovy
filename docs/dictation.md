# Dictation

How one **dictation take** goes from the shortcut to pasted text, and what
keeps it quick. Names follow [CONTEXT.md](../CONTEXT.md); the decisions are
[ADR-0014](adr/0014-pinned-dictation-paste-target.md) (pinned paste target),
[ADR-0049](adr/0049-cancelled-dictation-takes-never-deliver.md) (cancelled
takes never deliver), and
[ADR-0065](adr/0065-dictation-calls-the-users-own-provider-directly.md)
(direct calls with the user's own key). The latency work is the openspec change
`ditado-rapido`.

## Path of a take

1. **Shortcut.** The dictation helper (`src-tauri/native/mac-dictation-helper/main.swift`
   on macOS) reports `shortcut_key_down`; `ShortcutActivationController` in
   `src-tauri/src/dictation.rs` answers `start_listening` with a take id. The
   same key-down also rewarms the network route when it has been idle for a
   minute (`warm_dictation_route_if_idle` in the Groq provider). On macOS,
   accepting that start fixes the take's paste target before microphone
   permission or recorder setup: the exact app plus its focused AX window
   and field when available. Capture uses a shared 50 ms AX query budget,
   with at most 20 ms per message, and otherwise keeps the app alone.
2. **Capture.** The helper starts recording at once: microphone permission is
   only requested when it is not already granted, and the recorder for the
   next take is prepared after each take and at launch. The start cue plays in
   parallel and is cut from the recording before it is handed over (see
   [Start cue](#start-cue)). `listening_started` follows the key-down.
3. **Stop.** On release the helper stops the recorder and emits
   `recording_ready` with the `.m4a` path and peak meter level. macOS retains
   the start-time target throughout capture and finalization; stop never
   selects it again. Candidate app changes remain available for the next
   take, while this take's indicator and cleanup app remain fixed.
   No speech analysis runs before the upload.
4. **Transcription** (`dictate_transcribe` in the Groq provider). With the user's own
   Groq key it posts straight to `api.groq.com/openai/v1/audio/transcriptions`
   (model, `response_format=verbose_json`, the short dictionary hint as `prompt`, and
   `language`), carrying only that key. Without one it returns
   `groq_api_key_missing` without a request. The HUD identifies the missing Groq key and its location in Settings,
   in English or pt-BR.
5. **No speech.** A take under 0.5 s of audio (`audioDurationSeconds` in
   `recording_ready`) is not uploaded. On the direct Groq route the request
   asks for `verbose_json` and segments with `no_speech_prob` of 0.6 or more
   are dropped, since Whisper invents text over silence. A transcript with no
   letters or digits left (empty or only punctuation) ends the take with
   `no_speech` and nothing is pasted. The meter level is only logged.
6. **Cleanup** (optional, off by default). Off, the pasted text is Whisper's
   transcript only trimmed: no filler removal and no style. With **Clean up dictation text** on
   in Settings > Style, the transcript goes to the chosen OpenRouter model in
   one chat completion (`dictate_cleanup` in the OpenRouter provider): straight to
   `openrouter.ai` with the user's own key. A missing key returns
   `openrouter_api_key_missing` without a request. Transcripts above 800 bytes are split at word
   boundaries and up to four chunks are cleaned at once, then rejoined in
   order. Any failure or timeout pastes the whole raw transcript, and the HUD
   says "Pasting into {app} without cleanup". Cleanup never runs on an agent
   CLI or endpoint. The output style (standard, casual, formal) applies only
   while cleanup is on.
7. **Paste.** The helper writes the text to the clipboard and posts Cmd+V into
   the pinned target. On macOS it activates that app, raises the captured
   window and makes it main, then tries to focus the captured field. App
   activation still polls every 20 ms for up to 1 s with the existing 180 ms
   settle delay. Window restoration and verification use that same deadline,
   without a new activation budget. After preparing the key events, the
   helper checks the original AX window, frontmost app and deadline again.
   A closed or unrestorable window means `paste_target_unavailable`, with
   no Cmd+V and the text left on the clipboard. Field focus is best effort:
   if the original window is valid but its field refuses focus, paste stays
   in that window and `dictation_focus_restore` records the failure. Without
   a captured AX window, delivery keeps the start-time app-only fallback.

The macOS destination is owned by the take id and passed explicitly through
the recorder's start reset. Cancel, rejected start, no-speech discard and
delivery clear it. An asynchronous paste retains its own snapshot, so a reset
cannot redirect it to the next take. No part of this changes Control+V, which
continues to use the current focus. Clipboard restoration still runs 700 ms
after the automatic shortcut. The final check and WindowServer delivery are
not atomic; event submission does not prove which field consumed the text.

The seven-day dictation history stores the target bundle identifier captured at
`recording_ready`, when available, with both ordinary and low-speech recovery
entries. Older entries and platforms without a target identifier keep it null.
History displays the installed app icon when available and the microphone
otherwise. Dictation excerpts can also be sent to the activity provider as
bounded, fenced context; see [day intelligence](day-intelligence.md#gating-and-privacy).

## Reinsert the last dictation on macOS

**Control+V** reinserts the last eligible dictation into the currently focused
field using Unicode keyboard events. It never reads or writes the clipboard,
activates the old paste target, starts a take, runs transcription or cleanup,
adds history, or invokes a voice command. A remembered voice command is
inserted as literal text. **Command+V** and automatic paste, including its
700 ms clipboard restoration, keep their existing behavior.

Rust retains one result in memory, qualified by the data partition that owned
processing. A valid final result replaces it only after winning the delivery
claim, before helper I/O, so an automatic paste failure does not lose it.
Empty, rejected, quarantined, cancelled and stale results do not replace it.
Another data partition receives no text; a new eligible result replaces the
single slot. Restarting the app clears the memory. Nothing is loaded from
history. A helper restart alone does not clear the app's in-memory slot.

The macOS helper registers Control+V through the existing Carbon monitor,
suspends it during shortcut capture, and suppresses held-key repeats and use
during recording/finalization. Configured recording shortcuts take priority
over Control+V; a conflict or failed Carbon registration reports
`shortcut_register_failed`. Reinsertion uses the existing Accessibility grant
and never requests Input Monitoring. Missing Accessibility reports
`accessibility_permission_missing` without copying text.

A keyboard press creates a one-second request bound to the current app and
focused Accessibility element. Only its matching internal Rust reply can
start insertion. Duplicate replies, expired requests, app activation changes,
and a changed focused element are refused. Modifiers must be released within
that same deadline. Text is emitted without modifier flags in blocks of at
most 20 UTF-16 units, keeping surrogate pairs intact. Streaming long text has
no time-based truncation; focus, permission, capture, new recording and
modifier changes can interrupt delivery between blocks. An interrupted stream
may already have inserted a prefix. There is no clipboard fallback.

Focus queries through Accessibility are synchronous and can block. Each block
is prepared before the final query; after that query returns, the helper checks
Accessibility, modifiers, target/element identity, the frontmost process and
request authority/deadline again before posting. The initial deadline still
does not expire a stream already started. The final check and event submission
are not atomic with WindowServer focus changes, so a residual check-to-post
race remains; submission does not guarantee which field consumed the events.

Some apps do not expose a focused Accessibility element or accept Unicode
keyboard events. In that case reinsertion cannot guarantee delivery. Event
submission is not evidence that a native or web field consumed the text;
both require native QA on macOS. This shortcut is macOS-only.

## Settings

`dictation-settings.json` holds `cleanupEnabled` (default `false`),
`cleanupModel` (an OpenRouter catalog id, default
`nvidia/nemotron-3-nano-30b-a3b`), and `cleanupMigrated`. A file saved before
optional cleanup has no `cleanupMigrated`; on the first launch it is migrated
once to cleanup off, whatever engine cleanup used before. The obsolete
`dictationCleanup` engine slot is removed from provider settings; cleanup
always uses OpenRouter.

The same file stores `hudPlacement`: `bottom` by default, including files
without the field, or `top`. Settings > Dictation > Style exposes **Indicator
position**, independently of cleanup. The frame is horizontally centered in
the cursor monitor's work area with a 12 logical-pixel margin above the Dock
or below the menu bar. Resizing preserves the nearest vertical edge so a
bottom pill grows upward; error details and the native Esc hint use the
available side (`above` at the bottom, `below` at the top).

Only the destination of an explicit user drag is remembered for the current
process. AppKit returns from starting a drag immediately and may consume
`mouseUp`; the existing 33 ms native poll samples the frame and left button
on the main thread, committing the destination after release. Programmatic
show, resize, shake and exit cancel any active capture and never overwrite
the manual anchor. Restoration clamps the current size to the work area
without changing that anchor.

Preference changes apply on the next dictation show, not immediately to
shared-window content. Meeting-detection prompts keep their top anchor;
the separate recording HUD is unchanged.

Both settings and helper-ownership records use `app_paths::app_config_dir`,
the same dev/release isolation as provider settings. Debug builds never
import release settings or scan release helper records.

## Warm-up

At launch and on a key-down after a minute idle, Clovy reads the user's Groq
and OpenRouter keys, each on its own, into the Keychain store's per-account
cache (the same cache a key save or removal updates; there is no second one),
and opens the transcription connection without sending audio: `GET /models`
on Groq with the user's key. Without a Groq key no warm-up request is made. The helper preloads the cues
and prepares the next recorder after every take. No audio is recorded outside
a take.

## Start cue

The start cue used to play before recording began, which kept Clovy's sound
out of the transcript but delayed capture by the cue's length (336 ms). Now
recording starts on the key-down and the cue plays at the same time. The
cue's span is measured on the file's own clock: the default recorder's
`currentTime` when the cue starts, or, for a pinned microphone (whose file
starts at its first sample), what is left of the cue after that sample. When
the recording stops, the helper cuts that span from the start of the file
with a passthrough export (no re-encoding) before `recording_ready`. If the
export fails, the uncut file is used. See the design of `ditado-rapido` (D3)
for why this technique was chosen.

## Latency targets

From the `dictation-events.log` of the app being measured:

| Interval | Target |
| --- | --- |
| `shortcut_key_down` to `listening_started` | 150 ms (300 ms on the first press after launch) |
| `shortcut_key_up` to `paste_completed`, take up to 10 s, cleanup off, own Groq key | 1.5 s |
| `paste_target` to `paste_completed`, target already frontmost | 50 ms |

## Tests

- `cargo test --lib dictation::tests::last_dictation` - volatile memory,
  replacement, paste failure recovery, cancellation, stale preclaimed results,
  data partition isolation, Unicode payloads and private response authority.
- Compile the real macOS helper with the frameworks and targets in
  `src-tauri/build.rs`, then run it with `--self-test-last-dictation` for
  28 policy and UTF-16 checks without posting input events. These checks do
  not qualify Carbon registration, Accessibility or actual field insertion.

- `cargo test dictation` - settings migration, the pipeline with a fake
  transcriber and cleaner (cleanup off, on, failed, timed out, parallel
  chunks), and `no_speech` on an empty transcript.
- `cargo test dictation_route_tests` - fake Groq and OpenRouter hosts plus an
  isolated retired-origin listener: direct requests, provider-key separation,
  warm-up and zero requests to the retired origin.
- `cargo test llm::registry` - persisted provider migrations preserve CLI and
  endpoint note selections and remove obsolete cleanup selection.
- `cargo test dictation_hud_` - top/bottom origin, Dock/no Dock, 1x/2x,
  offset monitors, resize bounds, and settings save/reload compatibility.
- `src/test/dictation-cleanup-settings.test.tsx` - the cleanup switch,
  OpenRouter-only model picker, and indicator-position persistence errors.
