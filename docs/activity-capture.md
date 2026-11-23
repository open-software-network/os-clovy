# Activity capture

On by default with a persistent opt-out (OpenSpec change
`default-capture-and-session-mcp`). Supported on macOS and Windows 10 (1903+) /
Windows 11 (x64) via dedicated platform adapters (`MacActivityPlatform` and
`WinActivityPlatform`); Linux remains an unsupported state where the capture
runtime is disabled and the Activity settings tab is hidden. Text-only recording
of what the user sees and does, kept in an encrypted database (`activity.sqlite3`)
using SQLCipher on both supported platforms. Windows builds require Visual Studio
C++ Build Tools, Strawberry Perl, and NASM on `PATH` for the vendored
OpenSSL/SQLCipher compilation. This doc is the contract for the slices that build
on it (timeline, coding-agent ingestion, day summaries). Terms are in
[CONTEXT.md](../CONTEXT.md#activity-capture); the encryption decision is
[ADR-0057](adr/0057-activity-database-uses-sqlcipher-linked-app-wide.md).

## Where the code lives

| Path | Role |
|---|---|
| `src-tauri/src/activity/mod.rs` | Tauri commands, managed state, `setup` (starts capture and timeline on macOS and Windows), `publish` (frontend event + menu bar) |
| `src-tauri/src/activity/engine.rs` | `CaptureEngine::tick` (every 2 s), `ActivityShared` (state shared with commands and the tray) |
| `src-tauri/src/activity/platform.rs` | `ActivityPlatform` trait: the OS seam (window descriptors, AX/UIA/OCR text, input events, system idle duration, clipboard, disk) |
| `src-tauri/src/activity/macos/` | `MacActivityPlatform`: Accessibility (AX), ScreenCaptureKit + Vision OCR, event tap, system idle duration, pasteboard, TCC permissions |
| `src-tauri/src/activity/windows/` | `WinActivityPlatform`: UI Automation (UIA) tree inspection and text, Windows Graphics Capture (WGC) + `Windows.Media.Ocr`, content-free RawInput, fail-closed clipboard concealment, monitor enumeration, `GetDiskFreeSpaceExW` |
| `src-tauri/src/activity/store.rs` | `ActivityStore`: the only code that opens `activity.sqlite3`; schema, migrations, read/write API, retention, debug export |
| `src-tauri/src/activity/key.rs` | Database key (`ActivityKeyStore` via `keyring`: macOS Keychain or Windows Credential Manager) |
| `src-tauri/src/activity/{filter,schedule,input,redact,settings}.rs` | Exclusions and work hours, tick plan and state machine, input normalizer, clipboard redactor, `activity-settings.json` |
| `src/components/settings/ActivitySettingsSection.tsx`, `src/lib/activity-capture.ts` | Settings → Activity tab and its typed bindings |
| `src-tauri/src/menu_bar.rs` | Menu bar status line and "Pause capture" / "Resume capture" |
| `scripts/build-windows.ps1` | Windows build script verifying Strawberry Perl and NASM on `PATH` for SQLCipher compilation |

Tests: `cargo test --manifest-path src-tauri/Cargo.toml activity` (every Rust
test of the module is under `activity::`), and
`pnpm exec vitest run --configLoader=runner src/test/activity-settings`.

## Lifecycle

`activity::setup` (called from `lib.rs` after the tray exists) loads settings,
starts the timeline loop (`timeline::start`), and starts the
`clovy-activity-capture` thread on macOS and Windows; on Linux and unsupported
platforms it manages an unsupported state (`ActivityState(None)`) where capture
is disabled and the settings tab is hidden. The thread ticks every 2 s whether or
not capture is on and is woken early by any command. While off, it checks
permissions, discards input and clipboard changes, and reaps pending native
operations; it does not extract window text or persist frames. Each tick:

1. Reads permissions. On macOS, queries live TCC permissions (Accessibility,
   Screen Recording, Input Monitoring). On Windows, standard desktop user
   permissions apply (`permissions()` reports `Granted` for accessibility,
   screen recording, and input monitoring without interactive TCC prompts).
2. Opens the database on first need (only once capture or a coding-agent
   source is enabled; a key is minted only then).
3. Evaluates the capture state (`schedule::evaluate`), in this precedence:
   `off` → `keyMissing` / `error` → `needsPermissions` (Accessibility and
   Screen Recording are required on macOS; on Windows permissions are `Granted`)
   → `paused` (`manual` → `workHours` → `lowDisk` → `protectedVideo`) → `active`.
4. Opens or closes a pause record on every pause-reason transition.
5. While active, captures the focused window, every 5th tick the secondary
   displays (when `secondaryMonitors` is enabled, default false), input events
   and the clipboard; every 30th tick checks free disk space; on the first tick
   and every 1800th runs retention.
State changes are emitted as `clovy://activity-state` (payload: the status DTO
below) and mirrored to the menu bar.

## Privacy rules (enforced before anything is read or written)

- **Window identity:**
  - On macOS, `bundle_id` or `NSRunningApplication.localizedName`.
  - On Windows, `resolve_identity` (`src-tauri/src/activity/windows/window.rs`)
    resolves `(app_name, bundle_id)` from the executable path, `FileDescription`
    version info (fallback to file stem without extension for `app_name`), and
    optional AppUserModelID (AUMID). The `bundle_id` is the AUMID for packaged
    apps (UWP/MSIX), or the lowercase normalized executable name (e.g. `chrome.exe`)
    otherwise. Executable paths are cached in a bounded in-process cache (up to
    256 entries) with fallbacks to `App Paths` registry keys (HKLM/HKCU) and
    `SearchPathW`, without ever executing files.
- **Text only.** Accessibility text first:
  - On macOS, `AXManualAccessibility`/`AXEnhancedUserInterface` is set once per
    process for Chromium/Electron apps. Window chrome (toolbars, menus, buttons,
    images, scroll bars) is not counted as text.
  - On Windows, UI Automation (UIA) tree extraction runs in a dedicated
    multithreaded COM apartment (`COINIT_MULTITHREADED` / MTA) using lazy
    `IUIAutomationTreeWalker` navigation. Window chrome (toolbars, menus,
    buttons, scroll bars) is skipped. Password elements (`is_password()`) and
    their entire subtrees are skipped unconditionally during traversal.
    Offscreen nodes and nodes whose visibility cannot be determined are also
    pruned before reading text or visiting children, including hidden browser
    documents, so an allowed document cannot carry text from another tab.
  - When the content has fewer than 20 non-whitespace characters, Clovy falls
    back to local in-memory OCR on exactly that window:
    - On macOS, Apple Vision OCR on an in-memory ScreenCaptureKit image
      (`ocr.rs` calls ScreenCaptureKit and Vision through objc2).
    - On Windows, Windows Graphics Capture (WGC) item (`IGraphicsCaptureItemInterop`
      from HWND) and local on-device `Windows.Media.Ocr` (`OcrEngine`). Captures
      at native size; if dimensions exceed `OcrEngine::MaxImageDimension`
      (typically 2600 px), proportionally downscales the BGRA `SoftwareBitmap`
      in CPU RAM (`compute_scaled_dimensions` / `resize_software_bitmap` in
      `src-tauri/src/activity/windows/ocr.rs`) before recognition, preserving
      the entire aspect ratio without intermediate image encoding or disk I/O.
  - No image is encoded or written anywhere on either platform.
  - On Windows, native in-flight operations (`PendingCapture::Copy` surface
    copy lease or `PendingCapture::Recognition` OCR) are bounded to at most one
    per platform adapter until cancellation completes. A timed-out native
    operation is canceled and retained until terminal state (`Completed`,
    `Canceled`, or `Error`), reaped by subsequent ticks so uncompleted GPU
    surface copies are never dropped unsafely.
- **Bounded ticks.**
  - On macOS, every AX element gets a 0.25 s messaging timeout (the system
    default is 6 s); the private-badge walk is capped at 150 ms, the URL walk
    at 250 ms, the text walk at 300 ms / 3,000 nodes / 20k characters, and each
    ScreenCaptureKit completion at 2 s. A window that is still building its
    tree cannot stall capture.
  - On Windows, the UIA description walk (URL and private detection) checks a
    250 ms deadline, 200-node budget, and depth 40. A URL must be an explicit
    HTTP(S) URI from one visible Document's LegacyIAccessible metadata with
    document role. Editable address bars are not identity; missing, ambiguous,
    or incompletely inspected document metadata leaves the URL unknown.
    Document names, text, and descendants are skipped during description to
    prevent in-page URL/private spoofing. UIA text
    extraction checks a 250 ms deadline, 3,000 nodes, depth 40, and 20,000
    characters. Password subtrees are skipped unconditionally. UIA provider
    calls have configured 250 ms timeouts. WGC frame capture and OCR share a
    2 s work deadline; an unfinished native operation is canceled and retained
    until terminal state. These budgets are not a hard latency guarantee for
    synchronous native API calls.
- **Exclusions** (`filter::skip_reason`): Clovy's own windows (own pid or a
  `dev.guilhermexp.clovy*` bundle id), ignored apps
  (exact, case-insensitive name or bundle
  id/executable), private/incognito windows (no OCR fallback either), URLs
  whose host is an ignored domain or one of its subdomains, and, while any domain
  is ignored, browser windows whose tab URL is not known yet (Chromium builds its
  accessibility tree a moment after Clovy first asks). An excluded window has its
  title and URL read to decide, but no window text is extracted, no frame is
  stored, and the tick's input and clipboard are discarded.
- **Private windows** are recognized by title markers (Chrome "Incognito" /
  pt-BR "Modo anônimo", Firefox "Private Browsing" / pt-BR "Navegação
  privativa", Safari "Private Browsing", Edge "InPrivate", and es/fr/de
  variants) or by an incognito/private badge in the first levels of the
  window's accessibility / UIA tree. The check runs for every app the one
  browser predicate (`filter::is_browser`: known bundle ids or a browser name
  as a whole word) accepts. False positives only drop frames.
- **Revalidation.** For browser windows the descriptor is read again after the
  text is extracted; the sample is dropped unless the same window still shows
  the same URL and is still capturable (a navigation to an ignored domain, a
  tab turning private, or an unknown re-read all discard it). Secondary frames
  follow the same rule.
- **Secondary displays** (setting `secondaryMonitors`, off by default):
  when enabled, captured every 5th tick (~10 s).
  - On macOS, `CGDirectDisplayCopyWindowList`.
  - On Windows, `secondary_windows` (`src-tauri/src/activity/windows/window.rs`)
    enumerates physical non-primary display monitors via `EnumDisplayMonitors`
    (filtering out `is_primary`) and topmost top-level visible non-iconic
    windows in Z-order (`EnumWindows`). Windows must not be tool windows
    (`WS_EX_TOOLWINDOW`), must have valid non-empty geometry, must belong to
    non-Clovy pids, and the currently focused window (`focused_hwnd` from
    `GetForegroundWindow`) is explicitly excluded so it is never captured twice.
- **Protected video** (setting, on by default): a known DRM streaming app or
  site focused pauses capture with reason `protected_video`; such windows on
  secondary displays are skipped.
- **Work hours** (setting): outside the configured local days/times capture
  pauses with reason `work_hours`. An end at or before the start spans midnight
  and belongs to the start day.
- **Low disk**: below 1 GiB free capture pauses (`low_disk`) and resumes above
  2 GiB.
  - On macOS, checked via `statvfs`.
  - On Windows, checked via `GetDiskFreeSpaceExW` in
    `src-tauri/src/activity/windows/disk.rs` on the volume holding the database
    path (falling back to parent directory).
- **Input** carries no content:
  - On macOS, the event tap reads only event types (click, key-down); the
    normalizer drops any characters reported.
  - On Windows, Raw Input (`RegisterRawInputDevices` with `RIDEV_INPUTSINK` in
    `src-tauri/src/activity/windows/input.rs`) monitors keyboard and mouse events
    passively via a dedicated message-only window (`HWND_MESSAGE`). Checks flags
    only: keyboard events check only `(flags & RI_KEY_BREAK) == 0` for key-down,
    never inspecting virtual keys, scancodes, or characters (`characters: None`);
    mouse events check only `RI_MOUSE_*_BUTTON_DOWN` button flags, never
    inspecting cursor coordinates or motion deltas. Events are buffered in a
    bounded queue (up to 10,000 events) and drained on tick. Normalizer drops
    any characters reported.
  - Clicks, keys, and clipboard changes are buffered between ticks without their
    source window, so they are stored only when the same capturable window and
    URL were focused at both ends of the 2 s interval; any switch in between
    discards that interval's input (the app switch itself is still recorded).
- **Clipboard** text is redacted (`redact.rs`: private keys, `password=` /
  `password = value` / `"token": value` assignments, bearer values, provider
  token prefixes, JWTs, card numbers, high-entropy strings) and truncated to
  1000 characters.
  - Concealment markers are checked before plaintext read (fail-closed):
    - On macOS, pasteboards marked concealed (`org.nspasteboard.ConcealedType`),
      transient (`org.nspasteboard.TransientType`), or auto-generated are skipped.
    - On Windows, checks registered clipboard formats
      `ExcludeClipboardContentFromMonitorProcessing` and
      `CanIncludeInClipboardHistory` *before* requesting `CF_UNICODETEXT`
      (`read_clipboard_content_with` in `src-tauri/src/activity/windows/clipboard.rs`).
      If exclusion format is present or history format is present with value 0,
      clipboard text reading is skipped entirely (fail-closed, text reader is
      never called), preventing passwords and tokens from ever entering memory.
      Sequence numbers (`GetClipboardSequenceNumber`) track changes across ticks.
      A single worker, attached to the capture caller's desktop, owns
      `OpenClipboard`, native reads and `CloseClipboard`. The capture thread
      waits at most 50 ms per call; an unfinished read remains single-flight
      and quarantined until it terminates. Expired reads never deliver text
      later. A replacement noticed before any read remains eligible for a
      fresh epoch; a snapshot already read by an expired request is suppressed.
      Dropping the adapter does not join a blocked native read. A busy
      clipboard is retried only on a later tick.
      Native delayed-render regressions run in child processes with private
      window stations/desktops and never use the console clipboard.
- **Tempo desde a última entrada do sistema.** Cada quadro da janela em foco
  registra `input_idle_ms`, uma duração em milissegundos amostrada no tick,
  junto da janela. No macOS, `ActivityPlatform::system_idle_ms()` consulta
  `CGEventSourceSecondsSinceLastEventType(kCGEventSourceStateHIDSystemState,
  kCGAnyInputEventType)`. No Windows, consulta `GetLastInputInfo` e
  `GetTickCount`, subtraindo os contadores de 32 bits com `wrapping_sub` antes
  de converter para `u64`, inclusive quando o relógio circula após 49,7 dias.
  A consulta não instala um tap, não pede permissão e independe do switch
  `inputEvents`, do tap de cliques/teclas ou do Raw Input. Não registra tipo,
  posição nem conteúdo da entrada. Se a plataforma não informar o dado, o
  quadro guarda `NULL`. Quadros de monitores secundários continuam sendo
  apenas contexto e não recebem esse sinal.
- **Pause control.** "Pause capture" / "Resume capture" (settings and menu
  bar) act on the manual pause only; an automatic pause (work hours, low disk,
  protected video) shows "Pause capture" and resumes on its own.

## Database

`activity.sqlite3` in the app data dir. Debug builds use the compiled app
identifier with a `-dev` suffix (normally `dev.guilhermexp.clovy-dev`; proof
builds use a run-owned identifier). SQLCipher uses a random 32-byte raw key
in the system credential vault:
- On macOS, Keychain service `dev.guilhermexp.clovy.activity-db`
  (`dev.guilhermexp.clovy-dev.activity-db` in debug builds), account `sqlcipher-key`.
- On Windows, Windows Credential Manager via `keyring` with identical service
  names (`dev.guilhermexp.clovy.activity-db` canonical,
  `dev.guilhermexp.clovy-dev.activity-db` dev) and user `sqlcipher-key`.

WAL mode, `auto_vacuum = INCREMENTAL`. A file without its key is unreadable
("file is not a database"). Windows builds require Strawberry Perl and NASM on
`PATH` for compiling vendored OpenSSL and SQLCipher (`libsqlite3-sys` feature
`bundled-sqlcipher-vendored-openssl`).

Lost key: if the file exists but the key is missing or wrong, the store
returns `KeyMissing`/`KeyRejected`, capture shows `keyMissing`, and nothing is
deleted or rekeyed until the user confirms "Recreate database"
(`activity_recreate_database`), which deletes the file and its WAL and starts
empty with a new key.

Timestamps are RFC 3339 UTC with microseconds and `Z`
(`2026-10-04T17:43:10.123456Z`), so SQL string comparison is chronological.

### Schema (migration 1, `activity_capture`)

Migrations are an append-only catalog in `store.rs` (`MIGRATIONS`), recorded in
`schema_migrations(version, name, applied_at)`; a database from a newer build
refuses to open (`NewerSchema`). Add a version; never edit one.

| Table | Columns | Notes |
|---|---|---|
| `frame_texts` | `id`, `hash` (sha256 hex, unique), `text` | Deduplicated text bodies; consecutive ticks of an unchanged window share a row |
| `frames` | `id` (AUTOINCREMENT, never reused), `captured_at`, `app_name`, `bundle_id`, `window_title`, `browser_url`, `text_source` (`accessibility`/`ocr`/`none`), `text_id` → `frame_texts`, `input_idle_ms` (migration 6, nullable) | One row per 2 s tick of the focused window; the timeline's input |
| `secondary_frames` | `id`, `captured_at`, `display_id`, `app_name`, `bundle_id`, `window_title`, `browser_url`, `text_source`, `text_id` | ~10 s context samples of other displays; never part of a session |
| `input_events` | `id`, `occurred_at`, `kind` (`click`/`key`/`app_switch`/`window_focus`/`clipboard`), `app_name`, `count`, `clipboard_text` | Clicks/keys coalesced per tick (`count`); `CHECK (kind = 'clipboard' OR clipboard_text IS NULL)` |
| `pauses` | `id`, `started_at`, `ended_at` (NULL while open), `reason` (`manual`/`work_hours`/`low_disk`/`protected_video`) | Pauses left open by a crash are closed at the next start |
| `processing_cursor` | `consumer` (PK), `last_frame_id`, `last_frame_at`, `updated_at` | Per-consumer high-water mark; `timeline` is the one retention honors |

Migration 2 (`activity_timeline`) adds the timeline's tables; see
[activity-timeline.md](activity-timeline.md#schema-migration-2-activity_timeline).
Migration 3 (`coding_agent_blocks`) belongs to coding-agent ingestion; see
[coding-agent-sessions.md](coding-agent-sessions.md#database-migration-3-coding_agent_blocks).
Migration 4 (`day_intelligence`) adds hour reports, workstreams, and day
summaries; see [day-intelligence.md](day-intelligence.md#schema-migration-4-day_intelligence).

A migração 6 (`frame_input_idle`) acrescenta `frames.input_idle_ms INTEGER
NULL`, sem backfill. Quadros antigos permanecem sem o sinal, preservando a
classificação anterior ao reconstruir a linha do tempo. `frames_after` e o
export de depuração incluem o valor como `inputIdleMs`.

### Rust API (`crate::activity::store::ActivityStore`)

Get the open store from the managed `ActivityState` runtime
(`ActivityShared::store()`); it is `None` while capture was never enabled or the
key is missing. All methods are async and safe to call concurrently with
capture (a four-connection WAL pool).

Closing uses `db::close_sqlite_pool` to drain until the live connection count
is zero, including asynchronous returns and native worker shutdown. SQLx 0.8's
semaphore close can finish before those resources have gone away. This matters
on Windows, where an open SQLite handle prevents a confirmed database reset;
the barrier waits for resources rather than retrying file deletion.

| Method | Use |
|---|---|
| `frames_after(after_id, limit)` | Timeline ETL read path, oldest first, text joined |
| `processing_cursor(consumer)` / `advance_processing_cursor(consumer, last_frame_id, last_frame_at)` | Read and move the cursor; never moves backwards. Use `TIMELINE_CONSUMER` for the timeline |
| `secondary_frames_between(from, to)`, `input_events_between(from, to)`, `pauses_between(from, to)` | Context, signals, and pause records for a time range (`pauses_between` includes a still-open pause) |
| `latest_frame_at()` | Newest frame time |
| `prune(now, retention_days)` | Retention (the engine calls it hourly) |
| `debug_export(dir, now, limit)` | Readable JSON for verification (debug builds only, via the command) |

Writes (`insert_frame`, `insert_secondary_frame`, `insert_input_events`,
`open_pause`, `close_pause`) belong to the engine.

### Retention

Default 30 days, configurable 1 to 365. A frame is deleted only when it is older
than the period **and** `id <= processing_cursor('timeline').last_frame_id`.
Secondary frames, input events, and closed pauses are deleted when older than
both the period and the cursor frame's `last_frame_at`. Unreferenced
`frame_texts` go with them. Each sweep that deleted anything then runs
`PRAGMA incremental_vacuum(500)`, so space returns gradually without a full
`VACUUM` blocking capture. Until the timeline advances the cursor, nothing is
deleted. The same sweep removes timeline sessions, gaps, and search documents
older than the period.

## Settings (`activity-settings.json`, app config dir)

`enabled` (true), `secondaryMonitors` (false), `inputEvents` (true),
`pauseOnProtectedVideo` (true), `ignoredApps`, `ignoredDomains` (bare hosts,
normalized on save), `workHours` `{enabled: false, days: [1..5] (ISO), start:
"09:00", end: "18:00"}`, `retentionDays` (30), `codingAgents` (every source
off; see [coding-agent-sessions.md](coding-agent-sessions.md)), for day
intelligence `daySummary` `{time: "18:00"}` and `notifications` `{enabled:
true, quietHours: {enabled: false, start: "22:00", end: "08:00"}}`, and
`mcpServer` (true; the Clovy MCP server switch, changed only by
`mcp_server_set_enabled` from Settings → MCP; `activity_save_settings` keeps
the stored value, see [mcp-server.md](mcp-server.md)), and `defaultsVersion`
(internal; `activity_save_settings` keeps the stored value). Fields missing
from the file load their defaults, so a new profile starts with capture and
the MCP server on; malformed files load defaults. Manual pause is in memory
only: a restart resumes capture.

Defaults upgrade: a file without `defaultsVersion` was saved before capture
and the MCP server became default-on, when `false` could not be told apart
from an explicit opt-out. `settings::load_at_startup` turns both on once for
such a file (owner decision, 2026-10-07), keeps every other choice, sets
`defaultsVersion` to 1, and saves. From then on `false` is the user's choice
and no start changes it. If that save fails (reported on the process's
stderr), the app runs with the upgraded settings in memory while the file
keeps the old content, so the upgrade is repeated at the next start until a
save succeeds. The first successful save from either switch (Settings,
Activity or Settings → MCP) writes the marker together with the user's
choice, so a saved opt-out is never undone by the upgrade. A save that fails
returns `activity_settings_save_failed` and changes nothing, in memory or on
disk. Missing macOS permissions block recording (`needsPermissions`), whereas
on Windows standard desktop user permissions report `granted`. On Linux and
unsupported systems capture is disabled.

## Commands and event (frontend contract)

| Command | Request | Returns |
|---|---|---|
| `activity_status` | none | status DTO |
| `activity_save_settings` | `{ request: { settings } }` | status DTO |
| `activity_set_paused` | `{ request: { paused } }` | status DTO |
| `activity_request_permission` | `{ request: { permission: "accessibility" \| "screenRecording" \| "inputMonitoring" } }` | status DTO; prompts TCC on macOS, no-op returning `granted` on Windows |
| `activity_recreate_database` | none | status DTO |
| `activity_debug_export` | none | `{ path, frames, secondaryFrames, inputEvents, pauses, codingAgentBlocks }`; errors in release builds |
| `open_privacy_settings` | pane `"inputMonitoring"` added (`Privacy_ListenEvent`) | void; opens macOS Privacy & Security, no-op on Windows |

Status DTO: `{ supported, settings, state: { kind: "off" | "active" |
"paused" (reason) | "needsPermissions" (missing) | "keyMissing" | "error"
(message) }, permissions: { accessibility, screenRecording, inputMonitoring:
"granted" | "denied" | "notDetermined" | "unsupported" }, manualPause,
lastFrameAt, debugExportAvailable }`. Types live in `src/lib/activity-capture.ts`.

## Debug export

`activity_debug_export` (development builds only; the Activity tab shows the
button only when `debugExportAvailable`) writes
`<data dir>/activity-debug-exports/activity-debug-export-<UTC>.json` (mode 0600)
with the newest 500 frames, all secondary frames, input events, pauses,
coding-agent blocks (without their transcripts), the cursor, and the schema
version. The key is never written; a test asserts it is
absent from the output.
