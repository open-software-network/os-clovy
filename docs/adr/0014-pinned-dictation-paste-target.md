---
status: accepted
date: 2026-07-09
---

# The dictation paste target is pinned when the recording stops

The **dictation helper** delivers a transcript by writing it to the clipboard
and posting a synthetic Cmd+V. It chooses the app that receives that keystroke
by **pinning** the frontmost app at the instant the recording stops
(`FocusTargetController.pinCurrentTarget()`, called from `stopActiveRecording()`
before the recorder is torn down), and it pastes into that pinned app and no
other. The pin must precede teardown: the selected-microphone recorder stops
asynchronously, so pinning after it would reopen the drift window.

If the pinned app has quit by the time the transcript comes back, June does not
paste at all: it leaves the transcript on the clipboard and tells the user to
paste it themselves, the same way it behaves when Accessibility trust is
missing.

The helper also waits (bounded) for the pinned app to actually become
frontmost, and posts Cmd+V **only** if it is frontmost at that moment. If the
app never comes forward, the paste is abandoned to the clipboard rather than
typed somewhere else. The clipboard-restore delay is measured from the
keystroke rather than from the start of the paste.

## Why

Dictation is not a single instant. The user releases push-to-talk, and only
then does June upload the audio, run dictation transcription upstream, and
optionally run a cleanup pass over the text. That round trip is fast for a
short phrase and slow for a long one: **the wait scales with how long the user
spoke.**

The helper used to resolve the paste destination at paste time, reading
`lastExternalApp` — a field that a global `didActivateApplicationNotification`
observer rewrites on *every* app activation. Those two facts compose into a
time-of-check/time-of-use bug:

- Short dictation: the gap is under a second, nothing else activates, the
  paste lands where the user was typing. It looks like it always works.
- Long dictation: the gap is many seconds. Anything that activates an app in
  that window silently repoints the target, and the transcript is typed into
  whatever app now happens to be frontmost.

Because the transcript is written to dictation history regardless of where the
paste went, the failure presents as "dictation didn't work, but the text is in
the dictation tab" (JUN-226) rather than as an error.

Pinning removes the drift window entirely. It also makes the target a single
source of truth: Rust already snapshots `targetBundleIdentifier` at
`recording_ready` to decide cleanup context, and the helper now pastes into
that same app instead of into a value that has since moved.

Two smaller assumptions on the same path were wrong for the same reason:
`NSRunningApplication.activate(options:)` is asynchronous, so posting Cmd+V a
fixed 180 ms later could deliver the keystroke to the app that was *still*
frontmost; and scheduling the clipboard restore from the start of the paste
left the target only ~520 ms to read a clipboard that was about to be yanked
back.

## Alternatives considered

- **Insert into June's own composer directly, and keep clipboard+Cmd+V only for
  external apps.** Rejected for now: it splits dictation delivery into two
  code paths with different failure modes, and it fixes the bug only when the
  target is June. Dictation into any other app would keep drifting. Pinning
  fixes every target with one mechanism.
- **Pass the pinned target from Rust in the `paste_text` command.** Rust holds
  the bundle id already, and this would make the choice unit-testable in Rust.
  Rejected because a bundle id cannot distinguish two running instances, while
  the helper can hold the exact `NSRunningApplication`. Correctness beat
  testability; the helper ships in the same binary, so there is no wire
  contract to keep compatible.
- **Fall back to the frontmost app when the pinned app is gone.** Rejected: it
  reintroduces the failure this ADR exists to prevent, and typing a long
  transcript into an unintended app is worse than asking the user to press
  Cmd+V.
- **Pin when the recording starts rather than when it stops.** Equivalent in
  practice (the frontmost app rarely changes while the user is holding
  push-to-talk and speaking), and stop-time pinning matches the instant Rust
  already snapshots the bundle id.

## Consequences

- If the user deliberately switches apps while a long dictation is
  transcribing, the text lands in the app they dictated from, not the one they
  moved to. This is intended, and is what "paste into the chat I had active"
  means.
- A new `paste_target_unavailable` error can reach the HUD. It is not in the
  silent-error allowlist, so it renders as a real error rather than being
  swallowed as "nothing recorded".
- The helper's paste path is now the only place that decides the target.
  `activateLastExternalApp()` is gone; `lastExternalApp` remains only as the
  live source that `pinCurrentTarget()` samples.
- A pinned app that refuses to come forward (a modal, another full-screen
  space, a refused cooperative activation) now yields `paste_target_unavailable`
  instead of a paste. That is the intended trade: activation is a request, not
  a command, and typing a private transcript into the wrong window is worse
  than asking for a manual Cmd+V.
- The guarantee is "frontmost when we posted the keystroke", not "frontmost
  when the keystroke was delivered". The remaining window is the time to post a
  CGEvent, versus the multi-second window this ADR removes. It cannot be closed
  from outside the window server.
- `paste_text` carries no state guard, so if the helper crashes and respawns
  while Rust is still transcribing, the replacement process has no pin and
  reports `paste_target_unavailable`. The transcript is still on the clipboard
  and in dictation history, so nothing is lost.

## Windows addendum (2026-07-22)

Real Windows QA proved that exact top-level `HWND` activation plus the 180 ms
settle period is not sufficient to focus a WebView2 DOM editor. The helper can
successfully submit `Ctrl+V` to June's window while the ProseMirror composer
receives nothing. External applications still use the exact pinned-window
activation and `SendInput` path above without change.

Windows therefore has one exact-request exception. A Dictate-button request may
carry a bounded random request id. Rust validates it and injects June's process
id; the frontend is never trusted to identify its process. The helper binds
that metadata only when the command actually starts recording, captures the
start target, and offers direct composer delivery only when the exact `HWND`
and process also match the stop target. Toggle commands that stop an existing
recording cannot retarget it.

For an eligible request, `paste_text` must echo the id and the helper must
revalidate the exact window and process. It writes the transcript to the
clipboard, emits a correlated `agent_composer` transcript, and waits up to two
seconds for June to acknowledge insertion. A matching success permits the
usual delayed restoration of a previous clipboard backup. Rejection, timeout,
or any missing, stale, malformed, or mismatched identity leaves the transcript
on the clipboard with manual-paste guidance. Direct delivery never falls back
to synthetic `Ctrl+V`; this avoids duplicate or misdirected text.

### Correction (2026-07-23)

Direct delivery revalidates the initiating `HWND` and process identity but does
not require that window to remain foreground when recording stops. The stop
foreground is the destination only for synthetic paste. For direct delivery,
the random request id and captured draft key pin the composer, and the frontend
rejects a changed session or missing editor. Requiring start and stop foreground
handles to match incorrectly rejected valid flows stopped through the separate
non-focusable dictation HUD without adding routing safety.

### Focused global shortcuts (2026-07-23)

The same direct-delivery path may serve a global shortcut only when the session
composer pre-registers a one-shot request while its editor owns focus. Tauri
stores that request with the exact main-window `HWND`, consumes it only for a
shortcut command that starts recording while that same `HWND` is foreground,
and injects both the trusted process id and exact window handle into the helper
command. The helper independently requires its captured start target to match
both values. A shortcut started in another application, another June window,
or another field in June therefore retains native synthetic paste.

Registration and removal commands are ordered and request-id conditional so a
late removal cannot clear a newer registration. Session changes, editor blur,
workspace teardown, helper failure, and rejected starts retire stale requests;
a still-focused composer re-arms after terminal outcomes. Toggle start versus
stop decisions follow helper-confirmed listening state, so only actual starts
can consume a registration. Direct delivery remains insertion-only and never
auto-submits the composer.

## Destination indicator addendum (2026-10-12)

The macOS dictation HUD now presents the helper's current candidate while
listening and the pinned app during processing, paste and any visible error
for that take. This does not pin at start or transfer destination authority
to the renderer: the exact `NSRunningApplication` is still pinned before
recorder teardown. A closed pinned app keeps its displayed identity, but
paste still fails to clipboard recovery instead of selecting a replacement.

The additive `payload.target` contains `app`, `bundleIdentifier` and
`processIdentifier`, or explicit `null` when unavailable. `listening_started`
carries the initial candidate; take-correlated `focus_target` updates it only
while capturing. `finalizing_transcript` publishes the stop-time pin, and
`paste_target` carries the same identity. Metadata never commands activation.

The HUD resolves icons through the existing local `activity_app_icon` command
and its bundle-id cache. Bundle identity is sufficient for icon presentation;
it never replaces the helper's exact process instance for paste. Missing
metadata or icons use the central-icons window fallback, with the known name
when available or an unidentified-destination description otherwise. Icon
resolution cannot delay capture, cancel or paste. The indicator remains
separate from the waveform/spinner and ignores late resolutions from older
targets or takes. Older helpers and Windows retain the fallback path.

## Bound agent dictation addendum (2026-11-01)

The D3 management contract for `melhorias-agente-audio-automacao` adds a
typed, explicitly requested dictation lifecycle for a bound Clovy chat turn.
Its destination is the calling conversation's composer on the main window,
fixed by the host before approval with take, composer-request, conversation,
run and data-partition identities. Changing the visible conversation or data
partition cannot redirect delivery. This extends the exact-request exception
above to agent-owned takes on both desktop platforms; ordinary shortcut paste
keeps its pinned native destination.

Start and status reflect helper-confirmed state. Stop finalizes the same take,
persists history in its bound partition and sends the result only to that
composer. The main window must claim delivery through the native host, which
revalidates the live run, take and destination, then acknowledge insertion.
The draft store deduplicates by take/request and inserts without submission.
No model-provided target app, helper command passthrough or wake action can
retarget the take or start another conversation.

Cancel discards the take before delivery is claimed and refuses late results.
Once the main window has claimed insertion, cancellation reports
`dictation_delivery_in_progress` instead of falsely promising a discard; the
matching insertion completes its acknowledgement. Failure of this explicit
composer route is reported with its correlation and delivery state, without
an OS-paste fallback that could duplicate or misdirect the transcript.

## macOS start-time destination addendum (2026-11-06)

For macOS automatic dictation paste, this addendum supersedes the stop-time
pin above. Real QA showed that another window can become frontmost while
the shortcut is held: both the indicator and stop-time pin then followed it.
Capturing at stop protects transcription latency but leaves the capture
itself exposed to destination drift.

The helper now captures the exact app when it accepts a take's start, where
`startPending` becomes true and the take id is assigned, before microphone
permission or recorder setup. With Accessibility it also captures the focused
AX window and that app's focused field, accepting a field only from that
app's PID and, when captured, that window. AX messages use at most 20 ms each
within a shared 50 ms capture budget. Missing permission or unreadable AX
metadata keeps the start-time app-only fallback; it never requests another
permission. Native latency still requires measurement on the QA app.

Field lookup is scoped to the captured app rather than the system-wide AX
object. Setting a messaging timeout on the system-wide object changes the
helper's global default and could affect the independent Control+V path.
App-scoped lookup keeps the timeout local; PID and window checks still refuse
an element that belongs to another destination.

The take-owned destination is passed explicitly through the start reset.
Live candidate changes remain available for the next take but cannot change
this take's metadata, indicator or cleanup app. Stop does not pin again.
Cancel, rejection, no-speech discard and delivery clear the destination.
An asynchronous paste holds its own snapshot after the recorder state resets.

Before automatic Command+V, the helper activates the captured app, raises
the original window and makes it main, then tries to focus the original
field. A captured window must be restored and read back as focused; failure
or expiry leaves the text on the clipboard with `paste_target_unavailable`.
Field focus is best effort as specified in the approved design: a field
that rejects focus does not authorize another window, and its failure is
recorded as `dictation_focus_restore`. The existing activation/poll/settle
constants and 700 ms clipboard restoration remain; window work uses the
same activation deadline rather than extending it.

The final app/window/deadline check follows event construction and potentially
blocking AX queries. Check and submission are still not atomic with the
WindowServer and do not confirm consumption by a field. Control+V retains
its independent current-focus Unicode path. Windows and bound composer
delivery keep their existing contracts.
