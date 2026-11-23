# Activity timeline

Turns [activity capture](activity-capture.md) frames into a day timeline:
app sessions with their context, classified gaps, deterministic categories,
day statistics, and full-text search, all inside the encrypted
`activity.sqlite3`. Supported on macOS and Windows 10 (1903+) / Windows 11 (x64)
with dedicated capture adapters; Linux remains unsupported. The "Today" view
and the agent read it; later slices (day summaries, the MCP server, coding-agent
lanes) build on the query API below. Terms are in
[CONTEXT.md](../CONTEXT.md#activity-capture).

## Where the code lives

| Path | Role |
|---|---|
| `src-tauri/src/activity/timeline/builder.rs` | Pure state machine: frames → `Opened` / `Window` / `Closed` / `Gap` events; gap classification |
| `src-tauri/src/activity/timeline/context.rs` | Session context: browser domain, editor workspace from the window title |
| `src-tauri/src/activity/timeline/categorize.rs` | The ten-class categorizer and its confidence floor |
| `src-tauri/src/activity/timeline/stats.rs` | Day statistics (focused, idle, away, top apps, categories) |
| `src-tauri/src/activity/timeline/db.rs` | Timeline tables: batch persistence, day/detail/search queries, debug export sections |
| `src-tauri/src/activity/timeline/etl.rs` | One incremental pass over unprocessed frames |
| `src-tauri/src/activity/timeline/mod.rs` | Tauri commands, background loop, exclusion/retention filtering, agent tool wiring |
| `src-tauri/src/activity/timeline/agent_tools.rs` | `search_activity`, `get_activity_timeline` |
| `src-tauri/src/activity/timeline/debug_import.rs` | Development-only fixture import (`CLOVY_ACTIVITY_DEBUG_IMPORT`) |
| `src-tauri/src/activity/app_icon.rs` | `activity_app_icon`: an installed app's icon resolved by bundle id through Launch Services on macOS; executable icons via `SHGetFileInfoW` and packaged AUMIDs via AppsFolder `IShellItemImageFactory` on Windows. WIC draws a 64x64 PNG (≤ 64 KB). Successful Windows results are cached; misses remain retryable without observing a window first. |
| `src/components/activity-timeline/`, `src/lib/activity-timeline.ts` | The "Today" view and its typed bindings; the workstream timeline is `WorkstreamTimeline.tsx` / `WorkstreamDetail.tsx` with its pure layout in `workstream-timeline-layout.ts` |

Tests: `cargo test --manifest-path src-tauri/Cargo.toml activity` (timeline
tests are under `activity::timeline::`) and
`pnpm exec vitest run --configLoader=runner src/test/activity-timeline src/test/day-timeline-grid src/test/workstream-timeline`.

## ETL

`timeline::start` (from `activity::setup` on macOS and Windows) runs the background loop:
while a backlog remains it runs passes of at most 10,000 frames back to back,
releasing the pass lock between them; once caught up, one pass every 30 s while
the activity store is open. `activity_timeline`, `activity_timeline_search`, and
the agent tools call `refresh_before_read` first: a pass of at most 2,000
frames, skipped (not awaited) when another pass holds the lock, so a read never
waits for a historical backfill and answers from what is already built.

A pass reads frames after `processing_cursor('timeline')` in batches of up to
500 (`frames_after`), with that batch's input events (`input_events_between`)
and pause records (`pauses_between`), feeds them to the builder, and writes the
resulting rows, the builder state (`timeline_state`), and the cursor
(`advance_cursor_on`) in **one transaction**. A crash rolls the whole batch
back, so a restart never processes a frame twice or skips one. Retention (in
`ActivityStore::prune`) only deletes frames the cursor has passed. Only a pass
that caught up closes a session left open past the threshold (with a backlog,
later frames may still continue it). Meeting evidence (Clovy recordings) is
loaded from the older of the open session's start and the first unprocessed
frame, so a first build over weeks of retained frames still sees old meetings.

Rules (`builder.rs`):

- **Quadro útil**: o app, título da janela, URL ou texto mudou desde o quadro
  anterior; houve entrada gravada (clique, tecla, troca ou clipboard) de 3 s
  antes até 1 s depois do quadro; ou `input_idle_ms <= FRAME_SPAN + 1 s`
  (3.000 ms, inclusive). Esse último sinal vem do sistema a cada tick e inclui
  mouse, rolagem e trackpad, sem ampliar o tap nem gravar eventos sintéticos.
  Sem o valor (`NULL`), vale a regra anterior. Os demais quadros são ociosos;
  `loginwindow`, `ScreenSaverEngine`, `LockApp.exe` e `LogonUI.exe` nunca são
  úteis, mesmo com entrada.
  No Windows, quadros de `LockApp.exe` e `LogonUI.exe` também não contam como
  evidência de uma área de trabalho ociosa: o intervalo fica em **Ausente**,
  tanto enquanto o bloqueio continua quanto depois de retomar o trabalho.
  A contagem preexistente dos apps de bloqueio do macOS é preservada.
  O ETL recebe a duração em `StoredFrame` por `frames_after`; o limiar de 5 min
  e a divisão entre sono, ociosidade e pausa não mudam. Ler parado por mais de
  5 min sem nenhuma entrada continua podendo produzir uma lacuna ociosa.
- **Sessions**: consecutive useful frames of the same app (name and bundle id /
  executable name / AUMID) and context. A new context splits a session only when
  both are known: the browser's domain (`filter::url_host`, no `www.`) or the
  editor's workspace (VS Code family: last title segment; Zed, Xcode, JetBrains:
  first segment).
  A session lasts from its first frame to the next session's first frame, or to
  its last useful frame + 2 s when a gap follows. At most one session is
  active; it is categorized provisionally on every pass and finally on close.
- **Gaps**: more than 5 minutes between the end of a useful frame and the next
  one. Classified as `paused` (a pause record of one reason covers at least
  half; the reason is kept: `manual`, `work_hours`, `low_disk`, or
  `protected_video`), else `idle` (idle frames cover at least half), else
  `sleep` (no frames: the computer slept, capture was off, or an excluded app was in
  front). A gap closes the session before it, so its time is never session
  time. An active session left more than 5 minutes after its last useful frame
  is closed by the next pass that caught up.
- **Ongoing gap**: while the gap has not ended yet (an unchanged screen, an open
  pause, the machine asleep), it is not stored; reads that are caught up with
  capture add it from the last useful frame to now (`ongoing: true`, `id` 0),
  classified the same way, so the view, stats, and agent count idle and away
  time as it happens. The next useful frame stores the real gap.
- **Time going backwards** (clock change, imported fixture): the open session
  closes and the builder restarts without a gap; the first frame past the time
  reached before the rewind restarts it again, so no gap spans time already
  built.

## Categories

`categorize.rs`, no model: scores per category from the app (a fixed table of
known apps; terminals get a small coding prior, browsers none), window titles
and URLs weighted by each window's share of the session's frames (pull
requests, devops consoles and files, planning, design, docs, communication,
meeting URLs, research sites, entertainment; unknown titles nudge research),
captured text tokens (devops commands, code keywords, meeting controls, diffs),
and meeting audio (a Clovy recording in `recording_sessions` overlapping the
session: strong with a meeting app or meeting URL, weak otherwise). Confidence
is the winner's share of the total; below `CONFIDENCE_FLOOR` (0.35) the session
is `idle_personal`.

| Stored (`category`) | DTO | UI |
|---|---|---|
| `coding` | `coding` | Coding / Programação |
| `code_review` | `codeReview` | Code review / Revisão de código |
| `meeting` | `meeting` | Meeting / Reunião |
| `communication` | `communication` | Communication / Comunicação |
| `design` | `design` | Design |
| `documentation` | `documentation` | Documentation / Documentação |
| `planning` | `planning` | Planning / Planejamento |
| `deployment_devops` | `deploymentDevops` | Deploy and DevOps / Deploy e DevOps |
| `research` | `research` | Research / Pesquisa |
| `idle_personal` | `idlePersonal` | Idle or personal / Ocioso ou pessoal |

## Schema (migration 2, `activity_timeline`)

Appended to the activity catalog in `store.rs` (append-only; never edit).

| Table | Columns | Notes |
|---|---|---|
| `timeline_sessions` | `id`, `app_name`, `bundle_id`, `context_kind` (`domain`/`workspace`), `context`, `started_at`, `ended_at`, `duration_ms`, `first_frame_id`, `last_frame_id`, `frame_count`, `idle_frame_count`, `status` (`active`/`closed`), `category`, `confidence`, `meeting_audio`, `search_rowid_first`, `search_rowid_last` | Unique partial index: one `active` row at most |
| `timeline_session_windows` | `session_id` → sessions (cascade), `window_title`, `browser_url`, `first_seen_at`, `last_seen_at`, `frame_count` | PK `(session_id, window_title, browser_url)`; the detail view and categorizer evidence |
| `timeline_gaps` | `id`, `started_at`, `ended_at`, `duration_ms`, `kind` (`idle`/`sleep`/`paused`), `pause_reason` | `pause_reason` set iff `kind = 'paused'` |
| `timeline_state` | `id` (=1), `state_json`, `updated_at` | Builder state, open session row id, open search document |
| `timeline_search` | FTS5 (`unicode61 remove_diacritics 2`): `window_title`, `browser_url`, `body`, `session_id` (unindexed), `seen_at` (unindexed) | One document per window segment of a session (same title and URL, at most 5 minutes); `body` is the distinct lines seen (≤ 32k chars) |

Retention: each sweep also deletes closed sessions, gaps, and search documents
older than the retention period, and every query clamps its range to the
period, so search and the agent never return expired data.

## Query API (Rust, for later slices)

All in `crate::activity::timeline` over an open `ActivityStore`:

| Function | Use |
|---|---|
| `timeline_view(store, settings, from, to, now)` | Sessions and gaps overlapping `[from, to)` plus `TimelineStatsDto`, after current exclusions and retention |
| `search_view(store, settings, query, from, to, limit, now)` | FTS5 results (best first) with session id, app, title, URL, `seen_at`, snippet |
| `visible_session_detail(store, settings, id, now)` | The session detail the view and the MCP server show: `None` past retention or when excluded; windows on excluded URLs are dropped, and the text excerpt with them |
| `db::session_detail(store, id)` | Session, its windows (most seen first, ≤ 20), and a text excerpt (≤ 1500 chars), unfiltered |
| `db::sessions_between`, `db::gaps_between` | Raw rows without filtering |
| `refresh_before_read(app, store)` | Bounded catch-up before a read (at most 2,000 frames; skipped while another pass runs) |

Exclusions: a session is hidden while the app is in `ignoredApps` or its domain
matches `ignoredDomains`, so excluding something later also removes its history
from the view and the agent. Search applies the same rule with the session's
persisted domain and the document's own URL (a document written before the tab
URL was known has none), and pages through ranked FTS results until `limit`
allowed rows are found or the matches run out.

## Commands and event (frontend contract)

| Command | Request | Returns |
|---|---|---|
| `activity_timeline` | `{ request: { from, to } }` (RFC 3339) | `{ availability, message, captureEnabled, sessions, gaps, stats }`; a gap with `ongoing: true` (`id` 0) runs to the time of the read |
| `activity_timeline_session` | `{ request: { id } }` | `{ session, windows, textExcerpt }` through `visible_session_detail`; error `activity_session_not_found` (also past retention or excluded) |
| `activity_timeline_search` | `{ request: { query, from, to, limit } }` (`from`/`to`/`limit` nullable; limit 1 to 200, default 50) | `{ availability, results }` |
| `activity_app_icon` | `{ request: { bundleId } }` | The installed app's icon as a 64x64 PNG data URL (at most 64 KB), or `null` when the app is gone, the id is malformed, the PNG exceeds 64 KB, or capture is unsupported (Linux) |

`stats.topApps` (`{ appName, bundleId, durationMs }`, at most 5) tells apps
apart by bundle id: two apps sharing a name stay two rows, and an app renamed
by an update stays one; sessions without a bundle id group by name.
`activity_app_icon` (`activity/app_icon.rs`) resolves app icons locally without
network calls. On macOS it resolves by bundle id through Launch Services
(`NSWorkspace`, no scan of `/Applications`) and draws the icon into a bitmap.
On Windows it resolves the executable path for the bundle id / AUMID (via cache,
`App Paths`, or `SearchPathW`), extracts the `HICON` via Shell (`SHGetFileInfoW`),
and scales it into a 64x64 PNG using Windows Imaging Component
(`CLSID_WICImagingFactory` and `IWICBitmapScaler`). Encoded PNGs exceeding 64 KB
are rejected. Answers (including `None` for missing apps) are cached per
identity for the life of the process (up to 256 entries before cycling); the
frontend keeps one in-flight request per id (`activityAppIcon`).

`availability`: `ready`, `neverEnabled` (no activity database and capture
off), `unsupported` (Linux / unsupported platform), `keyMissing`, `error`. When
capture is off but history exists, the view opens the database to show it. Event
`clovy://activity-timeline` (`{ changed: true }`) follows a pass that wrote
sessions or gaps. Types live in `src/lib/activity-timeline.ts`.

## Agent tools

Named in the `activity-timeline` spec of the OpenSpec change
`add-activity-intelligence`. CLI turns read activity through the Clovy MCP
server, using the same bounded store/query API as the Today view.
`store_for_tools` opens the store with current settings and a bounded refresh;
capture must be enabled for tool access (`activity_capture_off` otherwise).
The server exposes these two tools plus four other activity tools; see
[mcp-server.md](mcp-server.md).

| Tool | Arguments | Result |
|---|---|---|
| `get_activity_timeline` | `from`, `to` (RFC 3339, optional; default local midnight to now) | Sessions (app, context, category, local start/end, minutes, active, window title; ≤ 300), gaps (kind, pause reason, ongoing; ≤ 300), `totalSessions`, `totalGaps`, `truncated` when either list was cut, stats in minutes |
| `search_activity` | `query`, optional `from`, `to`, `limit` (1 to 50, default 20) | Results with session id, app, window title, URL, local `seenAt`, snippet |

## "Today" view

First-level sidebar view (visible on macOS and Windows; hidden where capture is
unsupported on Linux via `isMacLikePlatform() || isWindowsPlatform()`). Day
navigation, search with a period filter whose results open the day at that
session, and two sections: this timeline and the "Day summary"
([day-intelligence.md](day-intelligence.md)). On a narrow window the header's
groups (day navigation, section tabs, search with its period filter) wrap whole
onto a second row; labels never break a line and only the search field shrinks
(to 140 px).

View states in `ActivityTimelineView.tsx`:
- **`unsupported`**: renders the empty state card (`timeline-unsupported-title`,
  `activity.unsupported.title`, `activity.unsupported.description`) with **no
  capture CTA button**.
- **`neverEnabled`**: renders the empty state card with a primary action button
  "Turn on in Settings" (`activity.emptyState.turnOn`) navigating to the Activity
  settings tab (`onNavigateToSettings("activity")`).
- **`keyMissing` / `error`**: renders top alert banner notices with recovery
  actions (e.g. "Open Settings" to recreate the database).
- **`ready`**: renders day navigation, search with period filter (`today`, `7d`,
  `30d`, `all`), section tabs ("Timeline" and "Day summary"), and the
  "Workstreams" / "Sessions" switch.
The timeline shows the day's workstreams by default ("Workstreams" /
"Sessions" switch):

- **Timeline** (`WorkstreamTimeline.tsx`, layout in
  `workstream-timeline-layout.ts`): a column of cards on a rail of the day's
  real local hours, in one brand color.
  - **One card per stretch.** A workstream gathers work from the whole day,
    so its sittings are split where the pause between the end of one and the
    start of the next exceeds 45 minutes (`CARD_SPLIT_GAP_MS`): the work
    after it starts a new card of the same workstream at its real time.
    Pauses up to 45 minutes stay gaps between the bars of one card. Hour-only
    parts follow the same rule by their hour window. Each card shows the
    workstream's title, and its range and "N sittings" count that stretch
    only. Its active time comes from the data, split per stretch so the
    cards always add up to the workstream's total:
    - an hour's located minutes go to the stretches its measured periods
      fall in, in proportion to those periods, in whole minutes (largest
      remainder), even when one hour has periods on both sides of a pause;
    - the minutes no period located (`minutes - locatedMinutes`, or a whole
      legacy hour) go to the stretch holding that hour's by-hour part, which
      is drawn approximate (dashed bar, "By hour"). That part is one segment
      of the cut, so it never straddles two cards and these minutes are
      never split.

    An hour's note goes to the stretch with most of its time. The header
    counts workstreams and their active minutes once.
  - **Sittings and honesty.** A card spans its first to last sitting.
    Sittings are the bounds of the measured sessions its hours' work was
    located in (`periods`,
    [day-intelligence.md](day-intelligence.md#schema-migration-4-day_intelligence)),
    joined when under a minute apart. They say where the work happened, not
    how long it took (one session can hold several workstreams), so the card
    prints the workstream's active minutes, never derived from its height or
    span. Each sitting is a solid 4 px bar on the card's left edge at its real
    time (inset 10 px from the card's ends, or a quarter of a short card;
    at least 4 px tall; always inside the card). Work known only by hour (a
    legacy hour without periods, or `minutes - locatedMinutes`) spans its
    whole local hour (on a fall-back day the repeated hour's two-hour window),
    draws a dashed bar, adds the "By hour" badge to the meta line, uses
    whole-hour times and does not count as a sitting.
  - **Window and scale.** From 20 minutes before the first sitting to 20
    minutes after the last (and to now, today), at least 150 minutes centered
    on the work. An hour with any work is drawn whole; a run of hours without
    work takes at most the height of one hour. The base is 1 px per minute; a
    shorter day grows by one factor for active and idle hours alike, up to 9
    px per minute, until it fills the scroller's usable height
    (`use-timeline-canvas.ts` observes the scroller, never the canvas). Longer
    days scroll. Positions are whole pixels; DST days keep 23 or 25 hours.
  - **Rail.** Labels ("9 AM" / "9h") only for hours with work and the hour
    right after one; today none after the current hour, which shows a solid
    brand dot and a brand label, plus the now line. Geometry from the
    canvas's left edge: labels end at 46 px, the spine and dots center on 54
    px, hour lines and cards start at 74 px.
  - **Columns.** Cards linked by overlap, even indirectly, share the width by
    the group's widest column count; a card takes the leftmost column free at
    its start. Back-to-back work stays in one column, touching. A card is at
    least 22 px (`--control-xs` or one title line); a card stretched to that
    minimum also keeps `--sp-2` below it, so two short cards stack only when
    their starts are 28 px apart at the current scale.
  - **Card by drawn height** (thresholds grow with the text-size
    preference): under 46 px a one-line pill; from 46 px a dot and up to two
    title lines; from 62 px the meta line (active time, range, "· N
    sittings" from 2); from 104 px the notes that fit below the measured
    header (17 px per slot after 8 px), with "+N more" when some do not,
    which the notes give way to so it is never clipped. Width does not
    change density.
  - **Interaction.** Hover lifts the card 1 px; selecting any card selects
    its whole workstream: all of its cards stay highlighted, the others dim
    to 0.38, and the details show the whole day of the workstream; clicking it again, clicking the
    empty column, or Escape clears the selection.

  Regressions: `workstream-timeline.test.tsx` drives the view through the
  spec's measured scenarios (scale, labels, columns, short cards, density,
  bars, selection) and holds the RED/GREEN witness;
  `workstream-timeline-split.test.tsx` covers the 45-minute split, per-card
  minutes and notes, whole-workstream selection and a 12-workstream day
  spread from 10:35 to 23:54; `workstream-timeline-minutes.test.tsx` the
  active minutes per card (a pause inside one hour, unlocated minutes in a
  by-hour stretch, whole minutes adding up); `workstream-timeline-now.test.tsx` the short
  window today; `workstream-timeline-layout.test.ts` covers DST, hour-only work, the scale
  limits, today and the note slots; `day-timeline-grid.test.tsx` covers
  navigation, fallback, sittings, legacy and the side panel.
- **Side panel**: without a selection, the day overview (focused, idle, away,
  categories, top apps with their real icons). Selecting a card (click or
  keyboard) keeps the grid, highlights the card, and shows its active time,
  its sittings (session times, with a note that they are not its duration),
  hour-only parts with their minutes, breaks between sittings only when all of
  its work was located, and what was done per hour; "Back to the day
  overview", clicking the card again, or Escape returns. A session from search
  shows the session detail (windows, URLs, excerpt). Changing the day clears
  any selection.
- **No workstreams yet** (no activity provider, or no completed hour folded):
  a notice says why (with a shortcut to the provider settings) and the day's
  captured sessions stay listed below it.
- **Sessions**: the 24 h strip (one block per session colored by category,
  distinct gap segments: idle, system sleep, paused for manual, work hours,
  low disk, protected video, plus the extra lanes below) and the chronological
  list with the live active session. Hour ticks sit at the real local times of
  the selected day (`hourTicks`).

Data refreshes on `clovy://activity-timeline`, every 30 s for today, and on
`clovy://day-intelligence` for the day shown (`use-day-intelligence.ts`).

### Lane extension point

`src/components/activity-timeline/lanes.ts` is the hook for extra lanes.
Register a lane by appending to `EXTRA_TIMELINE_LANES`; the coding-agent lane
(`CODING_AGENT_LANE`, see
[coding-agent-sessions.md](coding-agent-sessions.md#ui)) is the first one:

```ts
type TimelineLane = {
  id: string;
  label: MessageKey; // catalog key, en and pt-BR
  render: (props: TimelineLaneProps) => ReactNode;
};
type TimelineLaneProps = {
  from: string; to: string;         // selected local day, RFC 3339
  dayStartMs: number; dayEndMs: number;
  xForTime(ms: number): number;     // 0..1 fraction of the day
};
```

`TimelineDayStrip` renders the sessions lane, then each registered lane in
order, each in its own row on the same 24 h scale. A lane fetches its own data
for `from`..`to` (and may listen to `clovy://activity-timeline` or its own
event); positions are `xForTime(ms) * 100%` of the track width.

## Debug data

Development builds only. `activity_debug_export` adds `timelineSessions`,
`timelineGaps`, `timelineSearchDocuments` (count), and `timelineState` to the
export. To verify with test data, launch the debug app with
`CLOVY_ACTIVITY_DEBUG_IMPORT=/path/fixture.json`; once the activity store is
open, the fixture is written through the store's write API (format in
`debug_import.rs`) and a marker in `<data dir>/activity-debug-imports/` keeps
it from being imported twice. Import with capture off, so live frames do not
interleave with the fixture's.
