# Day intelligence

Turns the [activity timeline](activity-timeline.md) into hour reports, the
day's workstreams, and a day summary with a standup, written by the user's
**activity provider** ([llm-providers.md](llm-providers.md)), delivered on
schedule and announced by notifications. Terms are in
[CONTEXT.md](../CONTEXT.md#day-intelligence). Everything lives in the
encrypted `activity.sqlite3`.

## Where the code lives

| Path | Role |
|---|---|
| `src-tauri/src/day_intelligence/distill.rs` | Local distillation of an hour's text (no network) |
| `src-tauri/src/day_intelligence/embedder.rs` | On-device BGE small embeddings (candle), one-time pinned download |
| `src-tauri/src/day_intelligence/hour.rs` | Prompt de relatório da hora, análise da resposta, repartição em segundos com parte mínima |
| `src-tauri/src/day_intelligence/workstreams.rs` | Anchored incremental fold of an hour into the day's workstreams |
| `src-tauri/src/day_intelligence/summary.rs` | Day summary prompt and answer, panels, DTOs |
| `src-tauri/src/day_intelligence/prompts.rs` | System prompts, language directive, JSON schemas |
| `src-tauri/src/day_intelligence/schedule.rs` | Pure time decisions: local hours, completed hours, daily run, retries, quiet hours |
| `src-tauri/src/day_intelligence/notify.rs` | Notice queue, quiet hours, copy |
| `src-tauri/src/day_intelligence/sources.rs` | Meetings and notes (main database), coding-agent blocks (integration point) |
| `src-tauri/src/day_intelligence/db.rs` | Tables and the read API |
| `src-tauri/src/day_intelligence/pipeline.rs` | Orchestration over injected dependencies |
| `src-tauri/src/day_intelligence/mod.rs` | Tauri state, background loop, commands |
| `src-tauri/src/notifications.rs` | Activity notifications (click opens "Today", snooze button on macOS; standard notifications elsewhere) |
| `src/components/activity-timeline/DaySummaryPanel.tsx`, `src/lib/day-intelligence.ts` | The "Day summary" section of the "Today" view and its bindings |
| `src/components/settings/DaySummarySettingsSection.tsx` | Settings, Activity: summary time, notifications, quiet hours |

Tests: `cargo test --manifest-path src-tauri/Cargo.toml day_intelligence`
(real store, fake provider) and
`pnpm exec vitest run --configLoader=runner src/test/day-intelligence`.

### BGE embedder configuration and verification

- **Platform backend (`Cargo.toml`)**:
  - Both macOS and Windows use `Device::Cpu`. On macOS, `candle-core`,
    `candle-nn`, and `candle-transformers` enable `features = ["accelerate"]`
    for Apple Accelerate CPU operations, not Metal.
  - On Windows (`[target.'cfg(target_os = "windows")'.dependencies]`), Candle
    runs CPU-only (`candle-core = "0.10"`, `candle-nn = "0.10"`,
    `candle-transformers = "0.10"`, without `accelerate`), using `Device::Cpu`,
    and `tokenizers` uses `default-features = false, features = ["fancy-regex"]`
    without C dependencies.
  - Model download, directory structure (`<app data dir>/models/bge-small-en-v1.5`),
    and SHA-256 caching checks are identical across platforms. If the model is
    absent, downloading, or fails, distillation transparently falls back to
    existing lexical deduplication (`lexical_dedup`). No additional models or
    providers are used.
- **Real-model test invocation**:
  - The test `day_intelligence::embedder::tests::real_model_separates_near_duplicates_from_unrelated_text`
    is ignored by default because it requires the downloaded model files.
  - It is invoked by passing the directory containing `bge-small-en-v1.5` via the
    `CLOVY_TEST_EMBEDDER_MODELS` environment variable:
    ```bash
    CLOVY_TEST_EMBEDDER_MODELS=<dir containing bge-small-en-v1.5> cargo test --manifest-path src-tauri/Cargo.toml --lib day_intelligence::embedder::tests::real_model_separates_near_duplicates_from_unrelated_text -- --ignored --nocapture
    ```
  - Acceptance: verifies `embedder.ensure_ready()` transitions to `Ready`,
    generates 384-dimensional embeddings, and asserts `near > 0.86`
    (deduplicating near-duplicates) and `far < 0.86` (retaining distinct topics).
- **Synthetic 1-hour benchmark (Windows CPU)**:
  - An explicit measurement on a debug x64 build running on an Intel Core
    i5-10300H CPU:
    - Input: 1,800 samples / 2 s across 4 synthetic sessions (VS Code, Edge,
      Terminal, Notes), producing 847,650 raw characters.
    - Filtering stages: 12,800 gated spans cut down through prose, junk, and
      document frequency gates to 135 lexical candidates, which are embedded to
      yield 24 selected semantic spans.
    - Timings: model verification/readiness took 10.13 s separately and synthetic
      fixture generation took 1.13 ms; a single
      batch embed call for the 135 candidate texts took 55.9095 s wall clock
      (encompassing tokenization, model initialization, and CPU inference);
      total distillation time was 56.7775 s wall clock.
    - Evidence boundary: This is an explicit debug synthetic measurement under
      unoptimized CPU execution to verify functionality and throughput; it is
      not a production comparative performance claim, not an LLM-generated
      report, and not a native 1-hour live capture E2E test.

## Gating and privacy

- Inactive without an activity provider with JSON schema support (`provider`
  `missing` / `insufficient` in the DTO): no call, no model download. Every
  call goes through `llm::generate_for_activity` (activity selection only,
  never a fallback inference service, one call at a time).
- Inactive while capture is off. Current exclusions apply: excluded sessions
  and documents with an ignored domain never reach a prompt.
- Distillation is local. The only network access of this module is the
  one-time download of the embedding model into `<app data dir>/models/`
  (`BAAI/bge-small-en-v1.5` at revision `5c38ec7c405ec4b44b94cc5a9bb96e735b38267a`,
  every file verified by SHA-256). Cached files are trusted only after their
  SHA-256 matches again, once per launch (`LocalEmbedder::ensure_ready`, at the
  first tick with a provider); a mismatch is deleted and downloaded again.
  Until the files are verified, or if that fails, the distiller runs its
  lexical stages only.
- Captured text is untrusted. Every prompt puts it, and everything written
  from it earlier (hour summaries, activities, workstreams, coding-agent
  summaries, note excerpts and titles, window titles), between
  `<activity_data>` and `</activity_data>`; a closing tag inside (any case) is
  escaped (`prompts::fence`), and every system prompt says fenced content is
  quoted data whose instructions are never followed. Only Clovy's own numbers
  and labels (hour, measured minutes, categories) sit outside the fence.
- Notes and meetings of the day (title, time, a 400-character excerpt of the
  note, the user's edited version when there is one) go to the activity
  provider with the day summary. A recording across midnight counts only its
  part inside each day.
- Dictation history from the current data partition and period also goes to the
  activity provider inside the same untrusted-data fence. Hour reports include
  at most 20 excerpts of 400 characters; day summaries include at most 40 of
  240 characters. The read excludes dictations older than the seven-day
  history cutoff even when their rows have not yet been pruned. App names come
  from the already loaded timeline when available, otherwise the bundle id is
  used. Dictations alone do not trigger a report or summary, and existing
  reports are not regenerated.

## Pipeline

The background loop (`day_intelligence::setup`, after `activity::setup`)
wakes every minute, on a capture failure, and when a notice is due:

1. **Completed hours.** For today and yesterday, every local hour that ended
   and that the timeline has built (`built_until`: now when the ETL caught up
   with capture, else the cursor's last frame) and has no report gets one.
   Hours without session time are recorded `empty` and skipped.
2. **Distillation** (`distill.rs`, Meridian's pipeline): segment lines over 140
   characters, drop junk (under 18 characters, spinners, low letter ratio) and
   lines without function words (English and Portuguese stop words; code
   passes), except lines naming an entity, which skip both gates; cut lines seen in at least max(3, 25%) of the hour's sessions unless
   they name an entity, dedupe by normalized 80-character prefix, then
   semantically (cosine above 0.86, except an entity line in another session),
   pick 3 to 14 diverse lines per session (facility location), and rescue up to
   4 dropped lines per session whose entities (ticket keys, PR numbers, file
   paths, commit hashes) are not covered. Sessions under 15 s are flicker.
3. **Hour report**: uma chamada com as sessões medidas (numeradas `S1`,
   `S2`, ...), blocos de assistentes de código, reuniões e o texto destilado.
   Os minutos da resposta funcionam como pesos: o tempo ativo medido da hora
   (`active_seconds = focused_ms / 1000`) é repartido em segundos por maior
   resto (`normalize_seconds`). Toda atividade listada recebe pelo menos 1 s
   (quando `active_seconds >= n`); pesos ≤ 0, não finitos ou ausentes assumem
   o menor peso positivo da lista (ou 1 se nenhum for positivo). A soma dos
   segundos bate exatamente com o tempo ativo medido. `HourActivity` grava
   `seconds` e mantém `minutes` arredondados para compatibilidade legada e
   para o prompt da dobra. Tempos de 1 a 59 s são exibidos como "< 1 min"
   (nunca "0 min" para atividades reais). Cada atividade também nomeia as
   sessões em que ocorreu (`sessions: [3]`, opcional); seus `periods` são os
   limites medidos dessas sessões recortados na hora e unidos onde se tocam.
   O gate anterior é preservado: se `whole_minutes(focused_ms) == 0`, a hora
   fica vazia sem chamar o provedor. A precisão em segundos só muda a
   repartição das horas que já passariam por esse gate. Quando houver menos
   segundos que atividades, a soma permanece exata com partes não negativas;
   não é possível garantir 1 s para cada uma nesse caso.
4. **Workstream fold**: one call with the day's workstreams as anchors (id,
   title, summary) and the hour's numbered activities; the answer places this
   hour only. A placement on an existing id appends the hour and may refresh
   that workstream's summary; id 0 opens one; activities left out open one
   workstream named after the first of them. Titles and untouched workstreams
   never change. Hours fold in order; a fold that failed its 4 attempts lands
   as one new workstream without a call. A hora da frente de trabalho mantém a
   união dos `periods` das suas atividades (nulo quando nenhuma tiver períodos),
   `locatedSeconds` (e `locatedMinutes`), que são os segundos das atividades com
   períodos; o restante dos segundos fica conhecido apenas pela hora. Os segundos
   da frente são a soma exata dos segundos das suas horas.
5. **Day summary**: at the configured time (default 18:00), once per day: due
   when the local clock is past the time and no automatic run of the day
   finished, so a device asleep at 18:00 generates it at the first tick after
   waking. A device that slept through the time and woke after midnight first
   generates the previous day's summary (once; one day per tick). The summary
   is written only once the day's inputs are settled
   (`pipeline::incomplete`): the timeline built through the last hour that
   ended, each such hour reported (or empty, or given up after its last
   attempt), every report folded, and coding-agent ingestion scanned past that
   hour (or every source off, or scanning failing). Until then the scheduled
   run waits without recording anything and tries again at the next tick.
   On demand ("Generate summary") it reports and folds the day's pending hours
   now (retrying failed ones); if one still fails or an input is behind, the
   command fails with `day_summary_incomplete` instead of writing a partial
   summary. Text follows the interface
   language (`interface_locale::known()`, saved in `interface-locale.json` in
   the config dir and restored at startup); the background loop writes
   nothing while it is unknown, and today's reports and summary in another
   language go back for generation with the day's workstreams
   (`pipeline::relocalize_day`); a summary removed that way stays pending
   (`resummary:<day>:<trigger>`) and is written again, once, as soon as the
   reports are (`pipeline::finish_pending_resummary`). JSON keys stay English. A day with
   no reports, meetings, or blocks has no summary and no call.

Failures are recorded per unit (`hour:<hour>`, `fold:<hour>`, `auto:<day>`)
and retried after 5, 10, 20 minutes; 4 attempts in total (a manual run retries
anyway).

## Notifications

`notify.rs`: "Your day summary is ready" (body: the headline), "The day
summary could not be generated" (first failure of the scheduled run), and
"Activity capture stopped" (capture entered `error` or `keyMissing`, once per
distinct failure). The Settings, Activity switch drops them all; inside quiet
hours they wait for the end (22:00 to 08:00 and ready at 23:00 → shown at
08:00). Pending notices are in memory, one per kind and day.

Delivery (`notifications::send_today_notification`):
- **macOS**: uses Clovy's native `NSUserNotification` path where `userInfo`
  carries `clovyTodayDay` and `clovyTodayKind`. A click focuses the window and
  emits `clovy:today:open` (`{ day }`), queued until the webview calls
  `today_open_ready` (same handshake as agent notifications); the app switches
  to "Today", selects the day, and opens the "Day summary" section. The
  notification's action button "Snooze 1h" posts it again an hour later; macOS
  shows it only when Clovy's alert style is "Alerts" (with "Banners" only the
  click works).
- **Windows / elsewhere**: uses `tauri-plugin-notification` (`send_via_plugin`)
  with group `clovy-today-<kind>`, posting standard native system toasts
  without action buttons or click-through handlers (unbundled dev runs fall back
  to this path as well).

## Schema (migration 4, `day_intelligence`)

Days are local `YYYY-MM-DD`, hours local `YYYY-MM-DDTHH`; `started_at` /
`ended_at` are UTC (RFC 3339, microseconds).

| Table | Columns | Notes |
|---|---|---|
| `day_hour_reports` | `hour` (PK), `day`, `started_at`, `ended_at`, `active_minutes`, `summary`, `activities_json` (`[{description, minutes, seconds, periods}]`; `seconds` presente nos relatórios novos, com fallback em leitores legados), `distilled`, `distill_json` (stats), `locale`, `provider`, `generated_at`, `folded_at` | One per completed hour with activity |
| `day_workstreams` | `id`, `day`, `title`, `summary`, `first_hour`, `last_hour`, `created_at`, `updated_at` | Title never changes |
| `day_workstream_hours` | `workstream_id` → workstreams (cascade), `hour`, `minutes`, `note`, `periods_json` (envelope `{seconds, locatedSeconds, locatedMinutes, periods}` com bounds de sessão em UTC; nulo em registros antigos anteriores à mudança, com fallback `minutes * 60` quando `seconds` estiver ausente) | PK `(workstream_id, hour)`; minutes and seconds are the sum |
| `day_summaries` | `day` (PK), `headline`, `narrative`, `insights_json`, `standup_json` (`{done, in_progress, blockers}`), `hours_covered`, `locale`, `provider`, `trigger` (`scheduled`/`manual`), `generated_at` | Regenerating replaces the row |
| `day_intelligence_runs` | `key` (PK), `attempts`, `outcome` (`ok`/`failed`/`empty`), `last_error`, `next_attempt_at`, `updated_at` | Scheduler bookkeeping |

Migration 4 (`day_intelligence`) and migration 5 (`day_workstream_periods`)
define the complete schema; no idle columns (`system_idle_ms` / `input_idle_ms`)
or migration 6 exist (task 4.2 remains open).

Retention follows capture's period: rows of days before it are deleted hourly,
whenever the activity database is open, also while capture is off.
The debug export adds `dayHourReports`, `dayWorkstreams`, `daySummaries`, and
`dayIntelligenceRuns` (newest three days).

## Read API (Rust, for the MCP server and the chat engine)

In `crate::day_intelligence::db`, over an open `ActivityStore`:

| Function | Returns |
|---|---|
| `summary_of_day(store, "YYYY-MM-DD")` | `Option<DaySummaryDto>`: headline, narrative, insights, standup `{done, inProgress, blockers}`, `hoursCovered`, `locale`, `provider`, `trigger`, `generatedAt` |
| `workstreams_of_day(store, day)` | `Vec<WorkstreamDto>`: id, title, summary, minutes, seconds, hours `[{hour, minutes, seconds, note, periods, locatedMinutes, locatedSeconds}]` (`periods`: `[{startedAt, endedAt}]` ou null; fallback `minutes * 60` se ausente) |
| `hour_reports_of_day(store, day)` | `Vec<HourReportDto>`: hour, bounds, active minutes, summary, activities with rounded minutes, optional `seconds` and `periods` (legacy reports without seconds fall back to `minutes * 60`; no history is reprocessed) |

`pipeline::day_view(&ReadContext, day)` returns those plus `DayPanelsDto`
(numbers read now from the timeline, workstreams, meetings, and blocks). The
MCP server's `get_day_summary` tool ([mcp-server.md](mcp-server.md#tools-and-resources))
uses this read API over `ActivityStore` to return the stored day summary,
workstreams, and hour reports; `get_day_panels` calls `day_view` with its own
`DaySources` (meetings of the open data partition through
`sources::meetings_between`, the query `AppSources` runs too, and coding-agent
blocks from the retention floor on) and returns the panels in minutes, with
exact `seconds` alongside `minutes` for workstreams. `get_day_summary` likewise
preserves seconds for activities, workstreams and their hours, including
`locatedSeconds`; legacy data falls back to `minutes * 60`.
`get_day_schedule` returns the settings below.

## Commands and event (frontend contract)

| Command | Request | Returns |
|---|---|---|
| `day_intelligence_day` | `{ request: { day } }` | `{ availability, day, provider: "ready" \| "missing" \| "insufficient", embedder: "absent" \| "downloading" \| "ready" \| "failed", running, summaryTime, summary, workstreams, hourReports, panels }` |
| `day_intelligence_generate` | `{ request: { day } }` | Same DTO; errors `llm_activity_provider_missing`, `llm_structured_output_insufficient`, `activity_database_closed`, `day_summary_no_activity`, `day_summary_incomplete` (an hour or fold still failing, or an input behind; nothing written), provider errors (`llm_timeout`, ...) |
| `today_open_ready` | none | Day of a notification clicked before the webview listened, or null |

`panels`: `focusedMs`, `idleMs`, `awayMs`, `categories`, `topApps` (the day's
timeline stats, `{appName, bundleId, durationMs}`), `hours` (24 local hours with focused ms; they add up to
`focusedMs`), `workstreams` (`{id, title, minutes, seconds}`), `meetingCount`,
`meetingMs`, `codingAgentBlocks`, `codingAgentActiveSeconds`.

Event `clovy://day-intelligence` (`{ day, running }`) follows a run's start and
end and new reports. Types live in `src/lib/day-intelligence.ts`.

## Settings (`activity-settings.json`)

`daySummary.time` ("18:00"), `notifications.enabled` (true),
`notifications.quietHours` `{enabled: false, start: "22:00", end: "08:00"}`.
Invalid times load the defaults.

## Coding-agent blocks

`sources::coding_blocks_between` reads
`ActivityStore::enabled_coding_agent_blocks_between`
([coding-agent-sessions.md](coding-agent-sessions.md#rust-api-for-later-slices-crateactivitystoreactivitystore))
for each hour report and for the day summary and its panels, keeping only
the sources still turned on in Settings, Activity (like the MCP server's
`list_coding_agent_sessions`). Each block
carries the agent's name, project, title, local times, active minutes, and
its summary once ingestion wrote one, else its first prompt (never the
transcript). A block counts only inside the window (one ending exactly at
its start is out), and its active time is split by the share of the block
inside, so a block across two hours or two days is not counted twice.
