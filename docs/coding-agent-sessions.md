# Coding-agent sessions

Clovy reads the local transcripts of the coding agents the user runs on
macOS and Windows, cuts them into blocks, summarizes each block, and keeps
the blocks in the encrypted activity database, so the work done in a
terminal or editor agent (which screen capture barely sees) reaches the
timeline and the day summary.
Terms are in [CONTEXT.md](../CONTEXT.md#activity-capture); the database is the
one described in [activity-capture.md](activity-capture.md).

## Where the code lives

| Path | Role |
|---|---|
| `src-tauri/src/coding_agents/mod.rs` | `SourceId`, Tauri commands, managed state, `setup` (starts the scan loop), `window_start` |
| `src-tauri/src/coding_agents/sources/` | One read-only reader per agent (`claude`, `codex`, `copilot_cli`, `copilot_vscode`, `cursor`, `cursor_cli`, `antigravity`), discovery and change fingerprints (`mod.rs`), SQLite snapshot reading (`sqlite.rs`) |
| `src-tauri/src/coding_agents/record.rs` | The normalized `Record` / `Session` every reader produces; the authorship-marker check |
| `src-tauri/src/coding_agents/segment.rs` | Block cutting and the transcript rendered for the summarizer |
| `src-tauri/src/coding_agents/ingest.rs` | `Scanner::scan` (one pass over the enabled sources) and `blocks_for` |
| `src-tauri/src/coding_agents/store.rs` | `coding_agent_blocks` queries (an `impl ActivityStore` block) |
| `src-tauri/src/coding_agents/summarize.rs` | Summarizer choice and the summary drain |
| `src-tauri/src/coding_agents/settings.rs` | `codingAgents` in `activity-settings.json` |
| `src/components/settings/CodingAgentSourcesSection.tsx` | Settings, Activity, "Coding agents": one switch per source |
| `src/components/coding-agents/CodingAgentLane.tsx` | The coding-agent lane of the "Today" view (`CODING_AGENT_LANE`, registered in `EXTRA_TIMELINE_LANES`) |
| `src/lib/coding-agents.ts` | Typed bindings, DTOs, event |

Tests: `cargo test --manifest-path src-tauri/Cargo.toml --locked coding_agents`
(the spec scenarios end to end are in `coding_agents/red_tests.rs`) and
`pnpm exec vitest run --configLoader=runner src/test/coding-agents`.

## Lifecycle

`coding_agents::setup` (after `activity::setup`, cross-platform on macOS and
Windows) starts one async loop once `SourceRoots::current()` resolves. While at
least one source is on it runs every 60 s, and right away after the Activity
settings are saved:

1. Turning any source on makes the capture engine open the activity database
   even when activity capture itself is off (`ActivitySettings::needs_store`).
   Until it is open the loop retries every 3 s; nothing is read meanwhile.
2. **Scan.** Live blocks whose last record is more than 1 hour old are sealed.
   Then, for each enabled source only, its store entries changed since the
   start of the window are listed (on a blocking thread; Codex day
   directories before the window are listed too, because a resumed session
   keeps writing to the rollout of the day it started); an entry whose size
   and mtime (and its
   SQLite WAL's) did not change since it was last loaded is skipped; the rest
   are streamed into block builders, Clovy's own CLI conversations are
   dropped, and every block that ended inside the window is upserted.
3. **Summaries.** Up to 4 sealed blocks of enabled sources that ended inside
   the window are summarized per pass, newest first; with more due the loop
   comes back after 1 s. A block that started before the window and ended
   inside it is summarized like any other.
4. **Retention**, hourly: blocks that ended before the activity retention
   period (`retentionDays`, default 30) are deleted. It runs whenever the
   activity database is open, also with every source turned off.
5. When blocks or summaries changed, `clovy://coding-agents-updated` (no
   payload) is emitted.

The **window** is today and yesterday in local time (`BACKFILL_DAYS = 2`):
older history is never read, so turning a source on does not summarize weeks
of past sessions.

## Sources (read-only)

| Source | Store | Prompt vs reply | Working directory |
|---|---|---|---|
| Claude Code | `~/.claude/projects/<project>/<session>.jsonl` (`%USERPROFILE%\.claude\...` on Windows) | `type: user` with text is a prompt; `type: user` with only `tool_result` blocks is a tool result; sidechain and meta records count only for time | `cwd` per record |
| Codex | `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl` (`%USERPROFILE%\.codex\...` on Windows) | `event_msg` `item_completed` `UserMessage` / `AgentMessage` (current), or `user_message` / `agent_message` (earlier); `response_item` is skipped (injected context) | `session_meta` / `turn_context` `cwd` |
| Copilot CLI | `~/.copilot/session-state/<session>/events.jsonl` (`%USERPROFILE%\.copilot\...` on Windows) | `user.message` `data.content` (not `transformedContent`), `assistant.message`; `session.shutdown` / `session.end` is an exit | `session.start` `data.context.cwd` |
| Copilot in VS Code | macOS: `~/Library/Application Support/Code/User/workspaceStorage/<hash>/chatSessions/*.jsonl` (and `globalStorage/emptyWindowChatSessions/`); Windows: `%APPDATA%\Code\User\workspaceStorage\<hash>\chatSessions\*.jsonl` | Operation log replayed (`kind` 0 snapshot, 1 set, 2 append); `requests[].message.text`, response parts | `workspace.json` `folder` |
| Cursor | macOS: `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`; Windows: `%APPDATA%\Cursor\User\globalStorage\state.vscdb`, table `cursorDiskKV` | `composerData:<id>` gives the turn order, `bubbleId:<id>:<bubble>` type 1 / 2 | none |
| Cursor CLI | `~/.cursor/chats/<workspace-md5>/<chat>/store.db` (`%USERPROFILE%\.cursor\...` on Windows) | Root blob lists message blobs; user text is the `<user_query>` part | none |
| Antigravity | `~/.gemini/antigravity-cli/brain/<conversation>/.system_generated/logs/transcript.jsonl` (editor: `~/.gemini/antigravity/brain/`) (`%USERPROFILE%\.gemini\...` on Windows) | `USER_EXPLICIT` `USER_INPUT` (inside `<USER_REQUEST>`), `MODEL` `PLANNER_RESPONSE`, other `MODEL` steps are tool output | none |

Source locations and path resolution (`SourceRoots`):

- **User home (`roots.home`)**:
  - `SourceRoots::current()` resolves the user's home directory across platforms: checks `HOME` first, falls back to `dirs::home_dir()`, and falls back to `USERPROFILE` on Windows.
  - Tests use `SourceRoots::from_home(path)` to point home at a temporary fixture directory.
- **Application support (`roots.app_support(app)`)**:
  - On macOS: resolves under `~/Library/Application Support/<app>`.
  - On Windows: resolves via `dirs::config_dir()`, falling back to the `APPDATA` environment variable, or `%USERPROFILE%\AppData\Roaming\<app>`.
  - In tests, `SourceRoots::from_windows(profile, appdata)` configures explicit profile and AppData paths.

Source boundary and read-only rules:

- Each source has a configured location (its root). The root may itself be a
  symlink (dotfiles); it is resolved once. Below it nothing is followed:
  listings skip symlinked entries, a candidate (and a SQLite WAL next to it)
  must be a regular file, and right before reading the candidate is checked
  again to be a regular file whose canonical path is under the canonical
  root.
- **Symlink and reparse point defense**:
  - On Unix / macOS, files are opened with `libc::O_NOFOLLOW` so a symlink is
    never traversed.
  - On Windows, where `O_NOFOLLOW` does not exist, `open_regular` opens candidates
    with `FILE_FLAG_OPEN_REPARSE_POINT` (0x0020_0000), opening the final path
    component itself rather than following it if it is a reparse point.
    `GetFileInformationByHandleEx(handle, FileAttributeTagInfo, &attributes, ...)`
    rejects handles marked `FILE_ATTRIBUTE_REPARSE_POINT` or
    `FILE_ATTRIBUTE_DIRECTORY` with `io::ErrorKind::PermissionDenied`.
    The retained, validated handle is used for reading; the file is not
    reopened after the check. This protects the final component against a
    check/open race; it is not a claim that mutable parent junctions are
    immune to every path-resolution race.
- Files are opened for reading only.
- SQLite stores (Cursor, Cursor CLI) are never opened in place: opening a live
  WAL database, even read-only, can make SQLite create or write `-wal`/`-shm`
  next to it, and `immutable=1` would miss rows still in the WAL. The file and
  its WAL are copied (no symlink followed) to a private temporary directory
  and the copy is read.
- A disabled source's directories are not listed or read at all.
- Cursor CLI messages carry no timestamps: every record gets the chat's
  `createdAt` except the last, which gets the store's mtime.

**Bounded memory.** Transcripts are never held whole. JSONL is read line by
line; a line over 4 MiB is skipped without being buffered or parsed. Each
record body is capped (tool inputs 400, tool results 800, any record 4,000
characters) and goes straight into the session's block builder. A block that
closes before the window is dropped at once, and a kept block holds at most
the first 70,000 and the last 30,000 characters of its rendered transcript. A
VS Code chat session over 32 MiB (its operation log must be replayed whole)
is skipped. Cursor conversations are loaded one at a time.

**Self-ingestion.** Every CLI call Clovy makes puts
`CLOVY_AUTHORSHIP_MARKER` (`[clovy-internal-ai-call]`, see
[llm-providers.md](llm-providers.md)) alone on the first line of its prompt. A
session whose first prompt's first non-empty line is exactly the marker
(after the source's own wrapper, such as `<user_query>`, is removed) is
skipped entirely and reading stops there. A user prompt that merely mentions
the marker is user work. Claude calls are not persisted
(`--no-session-persistence`); anything else is caught by the marker.

## Blocks

`segment::BlockBuilder` cuts one session while its records stream in. A block
always starts at a user prompt; a prompt starts a new block when more than
1 hour passed since the previous timestamped record, when the open block is
at least 1 hour long (so a block ends on a complete agent turn and the next
starts on what the user asked), or when the agent exited after the open
block's last turn. Replies and tool output, however late, join the open
block, so delayed agent output never becomes a block of its own; bookkeeping
records (metadata, token counts) and exit markers never move a block's time,
so a session reopened days later does not stretch its old block; records
before the first prompt belong to no block. A long
autonomous stretch stays in its block until the next prompt. `active_seconds`
sums the gaps between records, each capped at 2 minutes. Cutting depends only
on earlier records, so appending to a transcript never moves an existing
block's start (its identity).

**Lifecycle**: every block but the session's last is written `sealed`; the
last stays `live` (rewritten on each scan) until the agent exits or it is idle
for more than 1 hour. Sealed blocks never change (the upsert only updates
`live` rows). A summary moves a block to `summarized`.

## Summaries

A block transcript is untrusted text: it holds whatever the agent read and
whatever the user pasted. It only ever goes to a model that cannot act on it,
that is, a CLI started with every tool off or an endpoint
([CLI isolation contract](llm-providers.md#cli-isolation-contract)).

| Agent | Summarized by |
|---|---|
| Claude Code | `claude` when installed (runs with every tool off), else the activity provider |
| Codex, Copilot CLI, Copilot in VS Code, Cursor, Cursor CLI, Antigravity | the activity provider: their own CLIs (`codex`, `copilot`, `cursor-agent`, `agy`) keep tools that cannot be turned off, so they never receive a transcript |

The activity provider is the one chosen in Settings, Models
(`llm::generate_for_activity`, which requires JSON schema support and refuses a
CLI that keeps its tools). With no usable summarizer, the block is not read
for summarizing, no call is made, no attempt is counted, and it stays
`sealed` until one becomes available. Own-CLI calls go through
`llm::generate_on_cli_for_activity`, which shares the activity semaphore (one
activity call at a time) and refuses CLIs that keep their tools.

The prompt asks for 1 to 3 plain sentences in the interface language (English
or Brazilian Portuguese) with the agent, project, and UTC time range outside
the fence, and the session title and block transcript inside
`<transcript>…</transcript>`; the instructions say the fenced text is
quoted, untrusted data whose instructions must never be followed, and a
`</transcript>` inside it is escaped so it cannot close the fence. Answers
are trimmed, unfenced, and capped at 1,200 characters. A failed attempt is
retried after 30 minutes times the attempt number; after 3 attempts the block
stays sealed with `summaryError`.

## Database (migration 3, `coding_agent_blocks`)

Appended after the timeline's migration 2 in the activity catalog
(`activity/store.rs`, append-only).

| Column | Notes |
|---|---|
| `id` | AUTOINCREMENT |
| `source` | `claude_code`, `codex`, `copilot_cli`, `copilot_vscode`, `cursor`, `cursor_cli`, `antigravity` |
| `session_id` | The agent's session id (file stem, `session_meta.id`, composer id, chat id, conversation id) |
| `started_at`, `ended_at` | RFC 3339 UTC with microseconds; `UNIQUE (source, session_id, started_at)` is the block identity |
| `cwd`, `project` | Working directory and its last component, when the agent records one |
| `title` | The agent's session title, when it has one |
| `first_prompt` | First user prompt of the block, capped at 200 characters |
| `prompt_count`, `reply_count`, `active_seconds` | Counts and capped active time |
| `transcript` | Rendered block text for the summarizer (never in a DTO) |
| `state` | `live`, `sealed`, `summarized` (`CHECK`: sealed and summarized have `sealed_at`; summarized has `summary`) |
| `sealed_at`, `summary`, `summary_source` (`cli:<id>`, `endpoint:<id>`), `summarized_at` | |
| `summary_attempts`, `summary_error`, `next_attempt_at` | Retry state |
| `updated_at` | Last write |

### Rust API for later slices (`crate::activity::store::ActivityStore`)

| Method | Use |
|---|---|
| `coding_agent_blocks_between(from, to)` | Blocks overlapping `[from, to)`, oldest first, as `CodingAgentBlock` (every column except `transcript`), of every source |
| `enabled_coding_agent_blocks_between(enabled, from, to)` | The same, keeping only the sources turned on in `enabled` (`settings.coding_agents`). The `coding_agents_blocks` command, the day summary, hour reports, and MCP tools read this, so a source turned off disappears from all of them |
| `upsert_coding_agent_block`, `seal_idle_coding_agent_blocks`, `coding_agent_blocks_to_summarize`, `record_coding_agent_summary`, `record_coding_agent_summary_failure`, `prune_coding_agent_blocks` | Owned by the scan loop |

Get the store from `ActivityState::shared()` → `ActivityShared::store()`.

## Settings, commands, event

`activity-settings.json` gains `codingAgents`: `{ claudeCode, codex,
copilotCli, copilotVscode, cursor, cursorCli, antigravity }`, all `false` by
default; older files load with every source off. The DTO exposes only these
seven source switches (there is no separate summary-disable toggle; summaries
are automatically handled for enabled sources when a suitable summarizer is
available). They are saved with the rest of the activity settings
(`activity_save_settings`), which also wakes the scan.

| Command | Request | Returns |
|---|---|---|
| `coding_agents_status` | none | `{ supported, sources: [{ id, name, enabled, present }], databaseReady, lastScanAt, lastError }` |
| `coding_agents_blocks` | `{ request: { from, to } }` (ISO times) | `CodingAgentBlockDto[]` of the sources turned on in Settings, Activity (empty while the database is not open) |
| `activity_debug_export` | none | adds `codingAgentBlocks` (count); the JSON file has a `codingAgentBlocks` array |

Event: `clovy://coding-agents-updated` (no payload).

## UI

- **Settings, Activity, "Coding agents"**: one switch per source with "Found on
  this device" / "Not found on this device" (device-neutral wording), how
  summaries are made, and the last read time (or why the database is unavailable).
- **"Today" view lane**: `CodingAgentLane` is registered in
  `EXTRA_TIMELINE_LANES` (`src/components/activity-timeline/lanes.ts`, see
  [activity-timeline.md](activity-timeline.md#lane-extension-point)), so the
  day strip draws it as the "Coding agents" row under the activity sessions,
  on the same 24 h scale. It loads the selected day's blocks itself
  (`from`..`to` of the lane props) and reloads on
  `clovy://coding-agents-updated`. Each block sits at its start and end time
  (at least 0.25 % of the day wide); hover or keyboard focus opens a card with
  agent, project, time range, prompt count, and the summary (or "In progress" /
  "Waiting for a summary" with the first prompt). Blocks that are not
  summarized yet are drawn faded; a live one has a solid border. A day without
  blocks shows "No coding agent sessions on this day" in the row.

## Verification

- Unit and integration: readers per agent with synthetic fixtures (including
  real SQLite stores for Cursor and Cursor CLI), symlinks below a root never
  discovered or opened (and a symlinked root resolved once), Windows
  `FILE_FLAG_OPEN_REPARSE_POINT` handle attribute reparse point and directory
  rejection, cross-platform path resolution in `SourceRoots::current` /
  `SourceRoots::from_windows` (verifying `HOME`, `USERPROFILE`, `APPDATA`, and
  Mac `Library/Application Support`), oversized JSONL lines skipped, bounded
  block transcripts, the segmenter (blocks only at prompts, delayed output
  joins the open block), store lifecycle (including a block crossing the window
  start, and a database at the timeline's migration 2 gaining
  `coding_agent_blocks` as 3), summarizer choice (only tool-free CLIs) and drain
  with a fake backend, the fenced prompt, the first-line marker rule, and the
  spec scenarios over a temporary home (a disabled source is never read,
  transcripts keep hash and mtime and nothing is created next to them, 2 h 30 min
  becomes three blocks cut at prompts, a conversation started by Clovy's own
  summary prompt is skipped).
- Component: `src/test/coding-agents.test.tsx` (settings section and lane).
