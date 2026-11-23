---
status: accepted
date: 2026-10-28
---

# Meeting window video and scoped Claude analysis

An explicit, default-off option records the detected meeting window on macOS.
The recording session remains audio-first: a missing permission, closed window,
failed helper or failed video analysis must not stop audio capture or prevent
transcript-only note generation.

## Capture and storage

Use a separate signed `Clovy Video Capture.app`, following ADR-0004's file
status and signal contract. ScreenCaptureKit captures a
`desktopIndependentWindow`, never the display. AVFoundation writes silent H.264
fragmented MP4 at no more than 5 frames per second, with the long side capped
at 1920 pixels and ten-second fragments. Paused frames are omitted from the
video timeline. The artifact records its start offset relative to recorded
audio time.
The writer uses the segmented MPEG-4 Apple HLS profile and concatenates its
initialization and media segments into one MP4. A serialized delegate writes
and synchronizes each segment. The older `movieFragmentInterval` setting
alone produced a flat final file in an actual AVAssetWriter fixture, so it
does not establish the required recovery contract. No playlist is needed.
Pause and resume publish sequenced host-clock control records through a
nonblocking, owner-only FIFO before the audio state changes. The FIFO is
created after audio starts and retains controls while the helper is launching.
The helper drains it before appending frames; signals only wake processing
or request finalization. A delayed worker or coalesced signal cannot postpone
the authoritative pause boundary. Full or broken control channels fail only
the video and discard its unsafe bytes, without waiting on disk or a helper
acknowledgement in the audio control path.

A clean FIFO end after confirmed termination of the original host process
finalizes the recoverable recording. The helper records the parent's kernel
birth time, so a reused PID does not count as that original process. It
applies complete queued controls and admits no more frames after EOF. EOF
with a live or unverified parent, malformed controls or a truncated record
remains a capture failure; recovery never promotes that failure to success.
Shutdown arms an independent four-second deadline before status writes or
awaiting ScreenCaptureKit. A known failure is published before that await;
a stalled shutdown terminates immediately without cleanup I/O, preserving
already synchronized fragments and metadata for recovery.

The FIFO preserves the LaunchServices launch of the helper bundle, which
owns its TCC identity; a pipe inherited by a directly launched executable
would change that attribution. The helper verifies the FIFO's type, owner
and permissions without following symlinks. Pause clears cached frames,
and resume rejects samples captured before its boundary.
Resume requests a fresh screenshot through the same exact-window filter,
so an unchanged slide does not depend on a new stream frame. Its result is
accepted only for the same active control generation and cannot replace a
newer stream frame.

The session owns `window.partial.mp4` and `window.mp4` next to its WAV files.
A separate `video_artifacts` table carries capture status, paths, window
metadata, dimensions, duration, offset and failure reason. Audio artifact
validation remains specific to WAV. Recovery preserves completed fragments;
it cannot recover a fragment the writer had not committed. Deleting the note
or recording deletes the video through the existing recording cleanup.
Deleting a project with its notes uses the same cleanup. A per-run lease
prevents a delayed helper launch from recreating deleted artifacts; an
independent ownership check stops a helper whose database row disappeared.
Normal completion keeps the lease until the writer has settled.
Recovery and analysis validate the artifact against the current AppPaths and
its exact session directory before reading or repairing it. Absolute paths
retained by a copied database cannot authorize access to another profile.
Process birth time and executable identity distinguish a live writer from
an unrelated process reusing its PID. Unknown identity and pending helper
launches remain protected from repair.
The capture binding records the host process birth identity before the startup
lease is created. A confirmed dead host allows recovery to revoke an orphaned
startup lease, then recheck helper registration. Live or unknown helper identity
still prevents repair, and delayed launches cannot use the revoked lease.
Signals require a verified identity; unknown identity never authorizes
signaling a PID. The worker uses kernel calls rather than spawning a process
for each liveness check, including checks before the first video frame.
Before that first frame is appended, the helper durably writes a session-bound
timing checkpoint. Recovery restores its audio offset instead of assuming
zero; missing or invalid timing produces a transcript-only fallback.
The host durably binds the capture attempt before launch. Recovery and worker
signals require matching attempt metadata; an older terminal status cannot
override a newer live helper. Recovery checks database ownership after file
promotion and removes the owned files if note deletion already committed.

The helper owns its Screen Recording permission. A new helper identity may
need the owner's grant in macOS Privacy & Security. Window enumeration runs
in the desktop process and needs its separate permission; if macOS refuses
enumeration, no candidate or video switch is available until the host grant
is configured. Detection selects a
meeting app or a matching browser window, preferring the microphone owner
and then frontmost order. A Meet tab in the background is not detectable from
the browser window title. There is no window picker or full-display fallback.
Native Zoom and Teams candidates require meeting or webinar title indicators.
Settings, preferences, chat, calendar, contacts and sign-in windows are excluded.
The exclusion matches whole utility titles after removing known app labels;
those words inside an explicit meeting subject do not disqualify the meeting.
A generic "Meeting", "Webinar" or "Reunião" title also requires microphone
ownership by that app family. Ambiguous custom titles are not selected.
This tightens the initial design's app-only heuristic following the owner's
review instruction. The helper repeats the same policy for the exact selected
window, and the native HUD queue retains the authoritative opt-in and window ID;
a renderer retry cannot enable video or substitute a different window.

A one-shot frame timer rearms at the next eligible host-clock deadline.
An early timer notification is deferred to that deadline instead of dropping
an entire frame interval. Successful frames remain at least 200 ms apart;
backpressure waits before retrying, without a catch-up burst.
Delayed controls also defer overlapping presentation timestamps until they
are at least 200 ms after the last successful write, preserving the original
pause boundaries and preventing the writer from receiving backwards time.

## One-shot tool exception

Only note generation with a recording session's video on the `claude` CLI may
use tools. Other CLIs refuse a video tool grant. This does not change the
separately accepted least-privilege policy for ordinary Codex or agy calls.

Resolve `python3`, `ffmpeg` and `ffprobe` from the login-shell PATH. Read
`~/.agents/skills/video-analysis/SKILL.md` into the system prompt and invoke
`~/.agents/skills/video-analysis/scripts/video_evidence.py` with the session's
video, `--no-transcript` and an empty temporary evidence directory passed as
`--out`. None of these prerequisites is bundled.

The installed `claude --help` is the source of the supported flags. Preserve
`--strict-mcp-config`, `--setting-sources ""`, `--disable-slash-commands`,
`--no-session-persistence` and the `[clovy-internal-ai-call]` authorship marker.
The grant names only `Bash(python3 <script> <video>:*)`, `Read(<video>)` and
`Read(<evidence>/**)`, with `--tools Bash,Read`, `--allowedTools`,
`--permission-mode dontAsk` and `--add-dir <evidence>`. The timeout is twenty
minutes and termination covers the process tree. Evidence is removed after
the call, including failure and timeout.

Those allow rules alone are insufficient: Claude may automatically allow
reads and read-only shell commands. A Clovy-owned per-call `PreToolUse` hook
therefore rejects all other commands and file reads, including paths that
escape through symlinks. It accepts the exact evidence-script invocation and
only the canonical video/evidence paths. User hooks and MCP configuration
remain excluded. This is a CLI permission boundary, not an OS sandbox or a
claim that the Claude executable itself cannot access the machine.

The guard sets `onFailure: "block"`, so a failed or timed-out guard does not
restore default tool permissions. This needs Claude Code 2.1.295 or newer;
older or unrecognized versions fall back to transcript-only generation.
The implementation was checked against installed version 2.1.312 and the
[official hook failure contract](https://code.claude.com/docs/en/hooks#block-the-action-when-a-hook-fails).

A Clovy-owned `PostToolUse` hook records successful evidence reads with a
per-call token in private scratch space outside the model's evidence grant.
Analysis requires a complete matching summary, every contact sheet image,
and at least one listed full-resolution frame image. Extracting files without
reading them, failed reads or forged evidence-local receipts produce
`analysis_failed`. The private key and receipt ledger are removed with the
CLI scratch directory. These receipts establish tool reads; they do not
replace the native proof of correct visual interpretation.

The system prompt treats speech and screen contents as untrusted data and
asks Claude to cross-check visual evidence with the transcript using the
offset, citing timestamps. Screen content and the transcript are sent to the
user's configured Claude service for inference. Only local evidence extraction
uses Python and FFmpeg.

## Fallback and observability

Other Notes engines, unsupported CLI versions, missing prerequisites, capture failures, analysis failures
and timeouts produce a transcript-only note. `generation_results` persists
the analysis outcome and prompt version; the note exposes a localized
"Video not analyzed: <reason>" notice, or indicates successful analysis.
No fallback sends the video to an endpoint or to Clovy API.
Crash recovery can finalize complete fragments from an interrupted `recording`
artifact. An explicitly failed capture keeps its failure outcome and uses the
transcript, even if some fragments remain readable; readable bytes alone do
not establish that the requested meeting capture completed.

## Alternatives and consequences

In-process ScreenCaptureKit would couple its lifecycle and failure modes to
audio. FFmpeg capture would introduce another capture dependency and risks
capturing a display instead of the selected window. The separate helper has
an extra permission surface but isolates these failures.

Frames and a local skill keep the integration narrow, at the cost of requiring
user-installed tools and limiting video analysis to Claude. Video takes more
disk space than audio; long meetings also increase inference time and usage.
An unavailable analysis never silently claims to have inspected the video.

## 2026-10-29 integration with direct providers

ADR-0072 and ADR-0073 removed the native chat harness and Clovy API after
this feature was proposed. Video note routing now lives in
`providers/notes.rs`; the existing Groq transcription, OpenRouter and endpoint
fallback routes stay direct. The CLI-only chat engine and Clovy MCP tools
retain their existing boundaries. No retired module or service is restored.

The video schema is catalog version 54 (`038_meeting_video.sql`), after
main's immutable retirement versions 52 and 53. The earlier unpublished
video version 52 belongs only to the isolated pre-integration proof; that
profile must not be reopened with this build. A new proof requires a fresh
run-owned identifier and profile. The migration regression upgrades a
synthetic version-53 database and verifies existing stamps and history survive.

## 2026-11-06 integration after main schema version 56

Main has since shipped catalog versions 54 to 56, including
`038_interrupt_unresolved_tool_calls.sql`. These migrations remain immutable.
Meeting video now uses catalog version 57 and `039_meeting_video.sql`.
The migration regression upgrades a synthetic version-56 database and verifies
all previously applied stamps and history survive without replay. Earlier
video proof profiles retain their old schema identity and must not be reopened
with this build; native verification still requires a fresh run-owned profile.
