# LLM providers (own providers)

Notes can use OpenRouter directly with the user's key, an installed CLI, or
an explicit OpenAI-compatible endpoint. Chat uses CLI engines only. The
one-shot API remains in `src-tauri/src/llm/`; direct provider transports and
note orchestration live under `src-tauri/src/providers/`. Settings are in
`src/components/settings/LlmProvidersSection.tsx`. Transcription uses Groq
with the user's own key; CLIs and endpoints do not transcribe audio.

## Pieces

| Module | Role |
|---|---|
| `llm/mod.rs` | Public API: `generate`, `generate_for_activity`, `generate_on_cli_for_activity`, `provider_for`, `test_provider`, `run_cli`, `parse_json_value`, `StructuredOutputLevel`, `LlmError`, `CLOVY_AUTHORSHIP_MARKER` |
| `llm/cli.rs` | Catalog of the six CLIs (`claude`, `codex`, `pi`, `agy`, `cursor-agent`, `copilot`), which of them can run with every tool off, one-shot argv/stdin/env for those, output parsing, schema strictifier |
| `llm/shell_env.rs` | Login-shell environment (`$SHELL -l -c 'env -0'`, captured once, merged over the app environment, plus fallback bin dirs); on Windows the app environment plus the installers' directories, with `PATHEXT` resolution |
| `llm/detect.rs` | Installed / path / `--version` per CLI, without claiming authentication |
| `llm/agy.rs`, `llm/agy_private.rs` | App-owned Antigravity profile, private execution directories and disposable MCP transport |
| `llm/agy_login.rs` | Owned OS terminal for native Antigravity authentication in that same profile |
| `llm/process.rs` | Child runner: own process group, timeout kills the whole group, group swept after exit, 8 MiB output cap |
| `llm/endpoint.rs` | OpenAI-compatible `/chat/completions` one-shot calls, structured-output ladder probe, RPM pacing |
| `llm/registry.rs` | Persisted registry (endpoints, per-use selection, measured levels), legacy migration, validation |
| `llm/secrets.rs` | Keychain store for endpoint keys (`dev.guilhermexp.clovy.llm-providers`, `dev.guilhermexp.clovy-dev.llm-providers` in debug); in-memory under `cargo test` |
| `llm/commands.rs` | Tauri commands (below) |

## Generation API for feature code

```rust
use crate::llm::{self, registry::{LlmUsage, ProviderRef}, GenerateRequest};

// One call on an explicit provider (endpoint or CLI). OpenRouter/None -> LlmError::NotLocal.
let out = llm::generate(&provider, GenerateRequest {
    system: Some(instructions),
    prompt: user_text,
    schema: Some(json_schema),      // None for plain text
    timeout: None,                  // CLI default 180 s, endpoint default 120 s
}).await?;
out.text;      // raw answer
out.json;      // Some(object) when a schema was given; answers that do not
               // validate against the schema fail with LlmError::InvalidOutput
out.provider;  // "endpoint:<id>" or "cli:<id>"

// Background activity work (reports, day summary, agent-session summaries).
llm::generate_for_activity(request).await?;
```

`generate_for_activity` is the only entry point activity features may use:

- activity provider "none" (the default) returns `LlmError::ActivityProviderMissing`
  without any call, so no activity text leaves the machine and nothing falls
  back to another provider;
- a provider whose measured level is below `json_schema` (or never tested)
  returns `LlmError::StructuredOutputInsufficient`;
- calls are serialized by a process-wide semaphore (one at a time).

`generate_on_cli_for_activity(kind, request)` is the one exception: a
background call on a named CLI regardless of the activity selection, used to
summarize a coding-agent block with that agent's own CLI
([coding-agent-sessions.md](coding-agent-sessions.md)). It takes the same
semaphore and follows the CLI isolation contract below.

`llm::provider_for(LlmUsage::X)` returns the chat, notes or activity selection.
Dictation cleanup is fixed to OpenRouter and has no selectable engine.

## Structured output levels

`none < prompt < json_object < json_schema < strict`. Endpoints are measured by
the connection test: a plain prompt for latency, then the ladder strict
(`response_format.json_schema.strict = true`), json_schema (`strict = false`),
json_object, prompt; the first rung whose answer validates against the probe
schema (only `{"answer": "schema"}` is valid) wins. The strict and json_schema
rungs ask the prompt for a different object (`"color": "blue"`), so an
endpoint that ignores `response_format` follows the prompt, fails validation,
and is not credited with schema support. CLIs have a catalog level (codex
`strict` via `--output-schema`; claude and agy `json_schema` via
`--json-schema`; pi, cursor-agent, copilot `prompt`); for the CLIs Clovy may
call (below) the test confirms it with the same discriminating probe (`none`
when the answer does not validate).
A level is stored only if the endpoint still has the URL and model that were
tested. Features that need JSON must refuse levels below `json_schema`.

When selecting an installed, one-shot-capable CLI for Activity without a measured
level, Settings runs the provider test automatically and shows its testing state.
A successful test persists the measured level before the selection is applied;
only `json_schema` or `strict` qualifies. Authentication failures point to
"Authenticate in Clovy" for Antigravity's private profile; quota and timeout
failures retain their own messages. A measured insufficient level uses the JSON
schema error. This does not change the CLI catalog level, schema flags, or the
backend's validation, and it never authenticates automatically.

## CLI isolation contract

A one-shot prompt carries untrusted text (meeting transcripts, notes,
dictation, coding-agent transcripts). Text in a prompt must never be able to
make the CLI read files, run commands, or reach the network, so **a one-shot
call only runs on a CLI that can be started with every tool off**
(`CliKind::tools_disabled`), **or, by the owner's decision (2026-10-23), on
codex and agy in their least-privileged mode** (`CliKind::one_shot_allowed`):

| CLI | One-shot calls | Why |
|---|---|---|
| claude | Yes | `--tools ""` turns off every built-in tool; `--strict-mcp-config` without `--mcp-config` and `--setting-sources ""` load no MCP server, plugin, or hook |
| pi | Yes | `--no-tools` (built-in and extension tools) and `--no-extensions` |
| codex | Yes, least privilege | `exec --sandbox read-only --ephemeral --skip-git-repo-check -c approval_policy="never" -c web_search="disabled" -c sandbox_workspace_write.network_access=false -c features.plugins=false -c features.apps=false -c features.hooks=false -c notify=[] --output-last-message last_message.txt [--output-schema schema.json] -c mcp_servers.<name>.enabled=false… -`, prompt on stdin: no writes, no network for commands, no approval prompt, no saved session; the answer is read from the last-message file in the scratch directory, under the 8 MiB capture cap (a larger answer is refused). Its built-in tools stay on (it can still read files), which the owner accepted. MCP servers, plugins, hooks and `notify` run outside its sandbox, so all are off: before the call, `codex -c features.plugins=false -c features.apps=false mcp list --json` lists the servers codex resolves from every config layer, and each is turned off by name. `--ignore-user-config` is not used because it also drops the user's `[windows] sandbox` and model settings. A list that fails or a name `-c` cannot address (outside `[A-Za-z0-9_-]`) fails the call before codex sees any content |
| agy | Yes, least privilege, in Clovy's private profile | `--print <prompt> --output-format json --sandbox --disable-slash-commands [--json-schema <schema>]`, plus transient settings with native tools and MCP denied. The app-owned profile replaces personal HOME/XDG discovery; personal MCP/plugins do not block a call. Each call has its own disposable working directory and a process-tree timeout. Native OAuth state persists in the common profile; API-key billing variables are excluded. The CLI still saves its native conversation. Windows measures the full quoted command line in UTF-16 and refuses more than 32,766 units before spawn. These restrictions do not establish an OS sandbox or unattended acceptance. |
| cursor-agent | Refused | print mode "has access to all tools"; `--mode ask` still reads |
| copilot | Refused | MCP servers from the user's configuration and plugins cannot all be excluded |

Native Mac backend qualification also exercised the unchanged production
`llm::test_provider` path (the core of Models, Test): Antigravity measured
`JsonSchema` in its authenticated private profile. Synthetic calls through
`providers::generate_note_from_transcript` and `llm::generate_for_activity`
returned a generated note and validated activity JSON with `provider: cli:agy`.
These headless checks changed settings only in their isolated test process;
they do not qualify the recording/ASR UI, day-summary persistence, sole-CLI
installation or unattended routines.

cursor-agent and copilot stay refused. A refused CLI fails with `LlmError::CliToolsNotDisabled` (`llm_cli_tools_not_disabled`)
before anything is spawned, in `generate`, `generate_for_activity`,
`generate_on_cli_for_activity`, and the connection test. `llm_set_usage`
refuses it for notes and activity (chat goes through the
[CLI chat engine](#cli-chat-engine), which keeps every CLI), and the Models tab
disables it in those menus and hides its test button. A selection saved before
this rule fails at use time with the same error; nothing is rerouted silently.

Every allowed one-shot call also:

- resolves the executable on the login-shell PATH (a GUI app does not inherit
  the shell PATH) and runs with the login-shell environment, so `HOME`,
  `PI_CODING_AGENT_DIR`, and other profile variables reach the CLI unchanged
  except for Antigravity's final private environment described above;
  Clovy's own variables (`OS_CLOVY_*`, `OS_JUNE_*`, `OS_ACCOUNTS_*`, `CLOVY_*`,
  `JUNE_*`, `GOOGLE_OAUTH_CLIENT_*`) are removed by `llm::shell_env::cli_environment`, the one filter
  for one-shot calls, `--version` detection, the login-shell capture, and the
  CLI chat engine; Antigravity then applies its private environment allowlist.
  Clovy opens native login only on explicit request and owns only its private
  transport configuration; on
  Windows see [Windows resolution](#windows-resolution);
- starts with `CLOVY_AUTHORSHIP_MARKER` (`[clovy-internal-ai-call]`) alone on
  the first prompt line and sets `CLOVY_ONESHOT=1`; session ingestion skips
  any CLI session whose first prompt's first line is exactly the marker;
- runs in an empty scratch directory (Antigravity: an app-owned exclusive
  execution directory with durable native authentication). Where supported, sessions, context files, skills,
  and slash commands are off (claude `--no-session-persistence --tools ""
  --setting-sources "" --strict-mcp-config --disable-slash-commands`; pi
  `--no-session --no-tools --no-extensions --no-context-files --no-skills
  --no-prompt-templates --no-approve`);
- strips `ANTHROPIC_API_KEY` for claude so a stray key never switches the user
  to metered billing;
- is killed with its whole process tree on timeout (`LlmError::TimedOut`), and
  the tree is swept right after the CLI exits, before output is collected, so
  a lingering helper cannot hold stdout open (Unix: the process group;
  Windows: a kill-on-close Job Object, `process_group::ProcessTree`); stdout
  and stderr are each capped at 8 MiB;
- on Windows starts without a console window (`hide_console`), like every
  one-shot process Clovy starts. Explicit native login creates a visible
  owned console instead.

## Settings and routing

### Meeting video exception

An opt-in recording session can include a silent video of its detected meeting
window on macOS. Only the `claude` Notes engine may analyze that video.
See [ADR-0071](adr/0071-meeting-window-video-and-scoped-claude-analysis.md).
This exception does not change ordinary one-shot calls or grant tools to
Codex, agy, other CLIs or endpoints. A video tool grant on a different engine
is refused before invocation.

Prerequisites are `~/.agents/skills/video-analysis/SKILL.md`, its
`scripts/video_evidence.py`, and `python3`, `ffmpeg` and `ffprobe` on the
login-shell PATH. The note system prompt embeds the skill text, treats screen
contents and speech as data, and asks for visual findings with timestamps
adjusted by the recorded video/audio offset. Evidence extraction runs with
`--no-transcript --out <empty temporary directory>`; the existing transcript
remains the speech source.

The installed Claude flags retain `--no-session-persistence`,
`--strict-mcp-config`, `--setting-sources ""` and
`--disable-slash-commands`, plus the Clovy authorship marker. Only
`--tools Bash,Read` is enabled. `--allowedTools` grants
`Bash(python3 <script> <session video>:*)`, `Read(<session video>)` and
`Read(<evidence>/**)`, with `--permission-mode dontAsk` and
`--add-dir <evidence>`. A Clovy-owned per-call `PreToolUse` guard also denies
every other Bash command and canonical Read path: the allow rules alone do
not exclude Claude's automatically permitted reads and read-only commands.
No user MCP server or hook is loaded. These are CLI permissions, not an OS
sandbox around Claude.

The explicit guard uses `onFailure: "block"`; its failure, timeout or invalid
output blocks the tool rather than restoring default permissions. Claude Code
2.1.295 or newer is required for that setting. Older or unrecognized versions
produce a notice asking for a CLI update and use the transcript-only route.
See the [Claude hook contract](https://code.claude.com/docs/en/hooks#block-the-action-when-a-hook-fails).

Extraction alone does not count as analysis. A Clovy-owned `PostToolUse` hook
records successful `Read` results in private per-call scratch space outside
the evidence grant. Completion requires the full matching `summary.json`,
every contact sheet and at least one listed full-resolution frame. Failed,
non-image or missing reads produce `analysis_failed`; files placed in the
evidence directory cannot forge the private receipts. The receipt key and
ledger are removed with the scratch directory.

Video analysis has a twenty-minute process-tree timeout. Its temporary
evidence directory is removed on completion or error. A missing prerequisite,
different Notes engine, capture error, analysis failure or timeout falls back
to the ordinary transcript-only route. `generation_results.video_analysis`
persists `analyzed` or the failure reason, and the note displays the localized
outcome. Video paths and frames never enter an OpenRouter or endpoint
generation request.

### Provider settings

The registry is part of `provider-settings.json`: `llmEndpoints` (id, name,
`baseUrl`, `modelId`, `hasApiKey`, measured `structuredOutput`, optional
`rpmLimit`), `llmUsage` (`chat`, `notes`, `activity`, each
`{ "kind": "openrouter" | "none" | "endpoint" | "cli", "id"? }`), `llmCliLevels`, and
`llmRegistryMigrated`. Endpoint keys are never in the file or any DTO.

- **Migration.** A legacy `localGeneration` endpoint becomes endpoint `local`
  ("Local model") with the same URL, model, and key (key moved to the
  Keychain). If the legacy local model was enabled, notes select it; chat is
  separately normalized to a CLI.
  If the Keychain refuses the key, the legacy key stays in the file, keeps
  working, and the move is retried on the next launch. Saving a new key for
  `local` ends a pending migration and drops the legacy key, so the retry can
  never overwrite the new key.
- **Default chat engine** ([ADR-0072](adr/0072-cli-chat-replaces-the-native-agent.md)).
  Claude Code is the default, with Codex as the existing installed fallback.
  A one-time settings migration moves old Clovy or endpoint chat selections
  to that default. No native or endpoint engine remains in chat. If no eligible
  CLI is installed, chat reports the installation/connection requirement.
- **Chat.** New sessions use the chosen CLI engine. Changing the generation
  model affects notes, never `llmUsage.chat`. Old session messages, runs,
  interruptions, and artifacts remain readable. Sending or retrying an old
  native session uses the CLI default; stale native runs waiting for a user
  are cancelled by migration rather than resumed. Legacy endpoint settings
  remain available to notes and activity through `llm::generate`.
- **Notes.** The selected Notes engine generates and cleans the transcript:
  OpenRouter directly with the user's key and saved model, or the existing
  endpoint/CLI route through `llm::generate`. Prompts live in
  `src-tauri/src/prompts/`. Missing OpenRouter keys return
  `openrouter_api_key_missing` before network access. There is no Auto or
  fallback route. A one-time migration rewrites the old server selection to
  OpenRouter and the old Auto model to the default. CLI/endpoint choices stay.
- **Note transcription.** `transcribe_saved_audio` in the provider layer (final turns, the
  per-source fallback, live preview) posts straight to Groq with the user's
  own Groq key, with the transcription parameters (`response_format=json`,
  canonical `audio.wav`/`audio.m4a` name, dictionary prompt, language, 25 MB
  cap, one retry of a 408/5xx or transport failure; an empty transcript is
  `no_speech`). Without a key it returns `groq_api_key_missing` before a request.
  A Groq 429 carries its `retry-after` (else `x-ratelimit-reset-requests`) as
  `details.retryAfterMs`; `send_direct` waits it out only up to 2 s. Final
  transcription reads one route per request (the saved key, the run's
  model, that key's pacer) and its kept chunks, pacer and transport all use
  it; the key is never read again for that request. A saved key the credential store cannot return
  fails the request with `provider_api_key_unavailable` (the note says to
  save it again) and the live preview sends nothing. It runs behind the
  pacer of that route (`domain/processing.rs` `RequestPacer`): one per Groq account, keyed by the
  first 8 bytes of the key's SHA-256 (never the key), with 18 requests a
  minute and 7,200 s of billed audio an hour (each request billed at least
  10 s) across both Sources, every note and the live preview. There is no
  server-route pacer. Pacer
  times are wall-clock milliseconds, so a pause ends on time across sleep,
  and a clock set back never stretches it. A 429 holds every caller of that
  pacer and repeats the same request; a wait over 90 s pauses the note with
  `transcription_rate_limited` and `details.resumeAtMs`, the note says when
  it continues, and `commands.rs` resumes it then while Clovy is open: the
  resume claims the pause in one update, so a Retry that got there first
  leaves it nothing, and every retry checks under the note's lock how many
  runs finished since it was queued: a ready note is never processed again.
  A manual retry shows itself at once only by turning a still-failed note
  into transcribing (a compare-and-set), and a retry that did so always
  runs, so no skipped run leaves the note transcribing.
  Every finished chunk of a split job (a full-Source job, an over-long turn)
  is kept in `note_transcription_chunks`, keyed with its model and language,
  so a resume sends only the chunks still missing. A job's durable identity
  includes the transcription model (the default adds nothing), so a model
  switched between pause and resume transcribes the note again. With the key, consecutive speech
  turns of one Source go as turn packs of up to 600 s through
  `transcribe_saved_audio_timed` (`verbose_json`, segment times) and come
  back per turn. The live preview takes a request only when one is free and
  within 3,600 s of the hour's audio, otherwise it drops the chunk (ADR-0070
  addendum).
  The live preview runs only when the Groq key is available. Missing keys
  are configuration errors, not HTTP errors or permission failures.
- **Dictation cleanup.** Optional and off by default. When enabled it calls
  OpenRouter directly with the user's key and the dictation cleanup model;
  it never runs through the Notes engine, a CLI or an endpoint. No key means
  no cleanup request, and the raw transcript is delivered with a HUD notice.
- **Note transcript cleanup.** Uses the Notes engine, the local
  `note_transcript_cleanup.md` prompt, the full dictionary, and earlier
  same-source turns. The Groq spelling hint stays short; see
  [configuration.md](configuration.md#transcription-context).
- **Activity.** Defaults to none; remote OpenRouter selection is not allowed.

## Text-model catalog

The text picker reads OpenRouter's `/models` catalog and includes all
text-output models, without a tools requirement. The cache lasts ten minutes;
requests have a ten-second total deadline. Key changes invalidate the cache
and reload the picker; load failures provide retry. Selecting a model updates
only the remote generation model, never `llmUsage` or the chat engine. The
picker description names the active Notes engine. Audio and media lists are
curated locally; missing keys are checked when an operation is requested.

## CLI chat engine

> Referência do motor de execução e do formato persistido. A interface de chat,
> incluindo Início e o chat de notas, foi retirada. Rotinas continuam usando o
> motor CLI e os registros de sessão. Veja [ADR-0075](adr/0075-desktop-opens-today-and-approves-external-mcp-actions.md).

A chat session's **chat engine** is an installed CLI, encoded as
`__clovy_cli_engine__:<cli>` (`crate::chat_engine`). `chat_default_engine`
resolves the default locally: Claude Code, otherwise the existing Codex
fallback. New sessions, Home turns
([home-assistant.md](home-assistant.md), ADR-0064), and note chat use it.
The composer's engine picker lists CLI engines only; endpoints remain in the
note/activity provider settings. Picking a text model never changes chat.

Session and run persistence lives in `src-tauri/src/agent_sessions/`, separate
from process orchestration. Historical native or endpoint sessions open with
their existing records; their next message or retry runs on the CLI default.
Clovy does not ship a second model-and-tool loop or fall back to a remote
chat route. No installed eligible CLI produces a clear setup requirement.

One message on a CLI engine is one CLI process in the session workspace:

| CLI | Turn | Resume id | Clovy's tools |
|---|---|---|---|
| claude | `-p --output-format stream-json --verbose --include-partial-messages`, prompt on stdin | `session_id` of `system/init`; `--resume <id>` | `--mcp-config <temporary file>` |
| codex | `exec [resume] --json --skip-git-repo-check [--image <path>]... -c <policy>... [<id>] -`, prompt on stdin | `thread_id` of `thread.started` | `-c mcp_servers.clovy.command=… -c mcp_servers.clovy.args=…` |
| pi | `--print --mode json --session-id <id>`, prompt on stdin | a UUID Clovy chooses on the first turn | none (pi has no MCP) |
| agy | `--print=<prompt> --output-format stream-json` | `conversation_id` of `init`; `--conversation <id>` | workspace `.agents/mcp_config.json`, with a unique server and a turn-scoped lease |
| cursor-agent | `-p --output-format stream-json --stream-partial-output --trust -- <prompt>` | `session_id` of `system/init`; `--resume <id>` | none (project MCP servers need approval inside Cursor) |
| copilot | `--prompt=<prompt> --output-format json --session-id <id>` | a UUID Clovy chooses on the first turn | `--additional-mcp-config=@<temporary file>` |

On Windows a CLI resolved as a `.cmd` or `.bat` launcher (npm installs, the
Cursor agent installer) runs through `cmd.exe`, which refuses an argument
with a line break and a command line over 8191 characters. For such a
launcher cursor-agent gets its prompt on stdin instead of `-- <prompt>` (it
reads stdin when `-p` has no prompt), and copilot gets it on stdin instead of
`--prompt=<prompt>` (`chat_engine::cli::prompt_on_stdin`). agy keeps the
argument: its installer ships `agy.exe`.

- **Permissions.** The flags come from one table, `chat_engine::policy`, by
  CLI, the session's safety mode (`sandboxed` or `unrestricted`), and whether
  a person watches the turn (`attended`: chat `true`, routines `false`); the
  turn builder only places them. The rule is: never a flag that skips or
  pre-grants approvals (`--dangerously-skip-permissions`,
  `--allow-dangerously-skip-permissions`, `--permission-mode bypassPermissions`,
  `--dangerously-bypass-approvals-and-sandbox`,
  `--approve-for-me`, `--yolo`, `--full-auto`, `--always-approve`,
  `--allow-all*`, `--approve-mcps`; `policy::FORBIDDEN`, swept over every
  combination by a test). The sole full-access exception is Codex with
  `unrestricted`: it uses `danger-full-access` and `approval_policy="never"`,
  including a routine explicitly configured with that mode. In an attended turn `--allowedTools` only for
  `mcp__clovy__*`:

  | Turn | claude | codex |
  |---|---|---|
  | attended, `sandboxed` | `--permission-mode default`; with Clovy's server `--allowedTools mcp__clovy__*` | `sandbox_mode="workspace-write"`, `sandbox_workspace_write.network_access=false`, `approval_policy="never"` |
  | attended, `unrestricted` | `--permission-mode acceptEdits` (edits in the workspace without asking; shell and the rest still ask), same `--allowedTools` | `sandbox_mode="danger-full-access"`, `approval_policy="never"` |
  | unattended (routines) | see below | see below |

  Every codex turn also gets `web_search="live"` and, with Clovy's server,
  `mcp_servers.clovy.default_tools_approval_mode="approve"` (codex `exec`
  fails an MCP tool without `readOnlyHint` otherwise; the approval broker
  inside each Clovy write tool is the authority, for that server only) and
  `mcp_servers.clovy.tool_timeout_sec=900`; claude gets `MCP_TOOL_TIMEOUT=900000`.
  Both outlast the broker's 10-minute wait (`approval_broker::APPROVAL_TIMEOUT`,
  checked at compile time). Image attachments reach codex as one `--image`
  per file, placed before the `-c` flags because `exec --image` takes every
  value that follows (a path with a comma stays in the manifest only); claude
  reads them from the manifest. The permission mode is always explicit, so a
  `defaultMode` in the user's own claude settings never decides it. Attended
  turns never get `--strict-mcp-config`: the MCP servers and claude.ai
  connectors the user configured in the CLI also load, and a server of the
  user's named `clovy` is replaced by the turn's. These are the CLI's own
  connections. MCP servers configured in Clovy use the Clovy gateway;
  their stored credentials are not copied into the CLI configuration
  ([ADR-0063](adr/0063-cli-turns-get-write-tools-through-an-approval-broker.md)).
  Clovy tool approval is derived from the runtime mode at run creation:
  `sandboxed` requires approval and `unrestricted` bypasses it for attended runs.
  There is one mode control, offered only before a new focused conversation.
  Home remains sandboxed. The legacy session `approval_mode` column has no
  authority; retries also derive a fresh run snapshot. Clarifications still
  require a person, and unattended tools never inherit bypass.
  Approval events keep the run's event cursor in the envelope and the persisted
  session-wide item position in `data.itemSequence`. Requested and updated cards
  use that same item position, so live approval does not move into older turns
  or wait for hydration to appear in the correct order.
  With Clovy's server, claude
  also gets `--permission-prompt-tool mcp__clovy__approve_action
  --disallowedTools mcp__clovy__approve_action AskUserQuestion`
  (`policy::CLAUDE_INTERACTION_TOOLS`): its own permission requests (shell,
  edits, reads outside the workspace) go to Clovy's `approve_action` tool and
  become Clovy approval cards through the approval broker, the model cannot
  call that tool itself, and it asks questions through Clovy's
  `request_clarification` instead of `AskUserQuestion`
  ([mcp-server.md](mcp-server.md#approval-broker)). Codex has no such hook
  in `exec`: its own actions use the selected sandbox policy. On codex, and on claude
  without Clovy's server, an action the
  CLI refuses for lack of approval becomes a failed tool item whose error
  says so (`needsApproval: true` in the event). Measured: claude answers the
  tool with "Claude requested permissions to …, but you haven't granted it
  yet"; codex marks the command `declined` or the sandbox blocks it
  ("operation not permitted"); agy fails the step with "permission check
  failed … user denied permission"; cursor-agent completes the call as
  `rejected`; copilot fails the tool with a permission error. pi, agy,
  cursor-agent, and copilot get no policy flags (their default
  non-interactive mode). `--trust` (cursor-agent) and `--skip-git-repo-check`
  (codex) only accept the session workspace, a directory Clovy created, as a
  workspace.
- **Unattended turns** (`attended: false`, routines only). Nobody can
  approve anything, so the tools are confined instead: claude gets
  `--permission-mode default`, `--tools Read,Grep,Glob,WebSearch,WebFetch` (no
  other built-in exists in the turn),
  `--allowedTools Read,Grep,Glob,WebSearch,WebFetch,mcp__clovy__*` (the only
  `--allowedTools` beyond Clovy's tools: the read-only set itself), and
  `--strict-mcp-config` (Clovy's server only), plus, with Clovy's server,
  `--disallowedTools mcp__clovy__approve_action` (no permission prompt to
  answer, so the model never sees that tool); codex gets
  `sandbox_mode="read-only"` for `sandboxed`, or `sandbox_mode="danger-full-access"`
  for `unrestricted`, with `approval_policy="never"`, and the same web
  search and server settings as an attended turn. Clovy's write tools reach
  the broker, which refuses them at once (`unattended_requires_approval`).
  `chat_engine::check_policy` refuses an unattended turn on any other CLI
  (`agent_cli_unattended_unsupported`).
- **Turn binding.** When Clovy's server is in the turn, the turn issues a
  token in the server's turn registry (`mcp_server::turns::TurnRegistry`,
  binding: session, run, CLI, safety mode, attended) and the CLI's server
  arguments carry `--turn <token>` (claude and copilot in the temporary
  configuration file, codex in `mcp_servers.clovy.args`, and agy in the
  workspace configuration); the token never
  goes into the prompt or the environment. Connections opened with it are
  bound to the run and see the tools that need a turn. It is revoked when
  the turn ends, fails, or is cancelled (`chat_engine::turn::TurnGrant`), and
  on cancel before the process has stopped; a new handshake with it is then
  refused with `turn_expired`. A revoked connection reads no new request but
  still delivers the answers of the calls it was serving (at most 2 s) before
  it closes; a call still unanswered then gets `turn_expired` from the relay.
  App shutdown revokes every token after the turns stop
  (`shutdown::run_cleanup`). Each run gets a new token.
- **Native wire names.** Bound Claude Code, Codex and Antigravity turns share
  one passive name codec per MCP grant, attached to the binding and stream
  translator. The existing canonical catalog, dispatcher, history, snapshots
  and approval broker remain the single pool and authority. At `tools/list`,
  ASCII names of at most 36 bytes using letters, digits, `_` and `-` pass
  unchanged unless they start with the reserved `_c_` prefix. Other names use
  `_c_` plus unpadded URL-safe base64 of the first 24 SHA-256 digest bytes
  (35 bytes total). Only advertised encoded names resolve, exactly once;
  raw names requiring encoding and unknown aliases are refused. A collision
  fails catalog publication atomically. Descriptions retain the full canonical
  name and original description; schemas and annotations stay intact.
  No canonical name is truncated, filtered or replaced in policy keys. The
  map lives only for this grant and adds no permissions. Public/unbound MCP
  and other CLI kinds retain their existing canonical behavior. Native events
  resolve wire names only after the engine's managed-server guard; foreign
  origins and unresolved names cannot claim a managed diagnostic.
- **Antigravity execution context.** `llm::agy::HostContext` is captured from
  `app_paths::app_data_dir` while the host environment is intact. The SDK's
  common HOME is `<app_data>/runtime-profiles/agy/home`; XDG directories live
  there too. Windows also redirects USERPROFILE/APPDATA/LOCALAPPDATA. LoginEnv
  and the app's global environment remain host-owned. Chat executes in a
  stable app-owned directory keyed by the stored session; each one-shot call
  gets an exclusive disposable directory. A requested personal workspace stays
  in the host gateway's authorization context and is never the SDK's cwd or
  an additional discovery root. Personal coding-agent ingestion is unchanged.

  A retained OS lease excludes concurrent turns of the same session; separate
  sessions have separate execution directories and grants. Directory/file
  creation refuses symlinks, unexpected ownership and unsafe permissions or
  ACLs before publishing authority. Unix creates children relative to held
  parent descriptors. Windows creates private current-user/SYSTEM objects
  with held ancestor handles. Existing objects are never repaired in place.
  There is no shared user configuration replacement or preservation algorithm.

  Each execution directory also owns a valid empty Git root, created directly
  through the private filesystem primitives without running Git. This stops
  SDK customization discovery at that directory instead of an ancestor's
  repository and hooks. A private receipt binds the minimal metadata to its
  filesystem identities; unknown, altered or unreceipted Git roots are refused
  before authority. Host `GIT_*` routing/configuration variables remain excluded.
  New boundaries are built entirely in exclusive private staging, with their
  complete ownership receipt inside the unit, then published atomically as
  `.git` without replacement. Interrupted staging remains private and retained
  without blocking another attempt in the same session. Existing complete
  boundaries with an outer receipt remain valid without rewriting their proof.
  One-shot cleanup removes only identified files and empty directories, retaining
  unknown data. Chat/login boundaries remain durable; native authentication and
  the HOME-based conversation origin are unchanged.

  Only the owned execution directory receives `.agents/mcp_config.json`: one
  fresh server named `c` plus the unpadded URL-safe base64 encoding of a full
  128-bit UUID nonce (23 characters), the existing absolute `McpLaunch` program
  and `--dir`, a turn token and `timeoutSeconds: 900`. The supported server
  `env` entry supplies the host HOME (and Windows profile paths) to the stdio
  relay. The SDK retains its private HOME; the relay keeps the existing host
  identity and debug production-data refusal. It remains a proxy and opens no
  database. No alternate relay bypass or credential export is introduced.

  Native Antigravity CLI 1.4.12 measurements show that the SDK qualifies names
  as `mcp_<server>_<tool>` and enforces `^[a-zA-Z0-9_-]{1,64}$`. The compact
  server leaves 36 bytes for tool names. The common wire codec above makes
  every admitted canonical tool representable, including the gateway's
  unchanged `agent_mcp::runtime_tool_name` identity of up to 128 bytes. This
  identifier is not authority; only the exact active server and a resolved
  wire name normalize as Clovy. Previous server namespaces remain foreign,
  with no raw-name or old-server fallback. Actual native SDK/UI qualification
  of the codec for all three engines remains pending; pure codec tests do not
  establish native acceptance.
  Real Tauri protocol/broker checks additionally cover the 36/37/128-byte
  boundaries, approved effects, deny/revoke without effects, canonical policy
  keys and all three event adapters. They do not substitute for a native model
  calling the wire-named tool through the signed app.

  After MCP installation, chat supplies the exact active server name as
  transient transport context, including when resuming a native conversation.
  Canonical input/history is unchanged; previous messages and native results
  do not define the current namespace. This context grants no authority:
  approvals remain with the app, and unowned servers are never rerouted or
  normalized as Clovy. Native SDK acceptance of this context remains separate.

  A private receipt records the exact owned transport and filesystem identity.
  Cleanup removes only matching transport, preserves altered/foreign content
  with an explicit diagnostic, and never removes native authentication state.
  The next lease can recover a crashed transport. Revocation precedes cleanup
  and terminal publication, including preparation failure and Stop. Persistent
  cleanup diagnostics remain canonical even if Stop already published.
  MCP config and receipt are published from complete private candidates with
  atomic creation that refuses an existing destination. Recovery recognizes
  the receipted candidate before publication and the same unit after publication.
  New transport receipts use version 3 with strict compact-name validation.
  Versions 1 and 2 remain cleanup-only recovery inputs under their existing
  long-name grammar and profile/config/filesystem identity checks; they are
  never issued or adopted as the current live server. A version 1 receipt can
  also identify a partial write of its exact config bytes.
  Unreceipted staging files are retained without blocking a retry; malformed
  receipts and foreign published units remain refused. No cleanup relies on a
  staging filename alone as proof of ownership.

  Transient settings use `request-review`, allow only this turn's MCP server,
  and deny native command/file/URL actions. A one-shot call denies MCP too.
  `useG1Credits: false`, an empty `modelProvider`, and the environment allowlist
  preserve native OAuth rather than silently selecting API-key billing.
  Settings are not a claim of an OS sandbox. The parent's native evidence
  verifies server env.HOME and some denied native actions; browser, delegation,
  app Stop and unattended/platform acceptance still require their own proofs.
- **Note references.** A message that references notes (`@note:<id>`, which
  the note chat panel always puts first) reaches the CLI with a line asking it
  to read them with Clovy's `get_note` tool
  (`chat_engine::with_note_reference_context`); the stored user message is
  unchanged.
- **Environment.** Except for the private Antigravity context above, the login-shell environment, as for one-shot calls (PATH,
  `HOME`, `PI_CODING_AGENT_DIR`, and the user's other profile variables
  unchanged), minus Clovy's own variables (`OS_CLOVY_*`, `OS_JUNE_*`,
  `OS_ACCOUNTS_*`, `CLOVY_*`, `JUNE_*`, `GOOGLE_OAUTH_CLIENT_*`; for example the
  local development bearer token loaded from `.env`), which would otherwise
  reach the CLI, its tools, and its MCP servers (`llm::shell_env::cli_environment`,
  the same filter as one-shot calls).
  `ANTHROPIC_API_KEY` is removed for claude. No authorship marker: these are
  the user's own conversations.
- **Continuity.** The CLI's conversation id is saved in the run's config
  (`{"engine": "cli", "cli", "conversationId"}`) as soon as it is known, and
  the next message resumes it while the session's previous turns stayed on the
  same CLI. Antigravity also persists `cliProfileId` in this existing JSON;
  a missing or different profile origin starts a new native conversation with
  canonical replay, never passes the legacy ID to `--conversation`, and never
  copies SDK brain/auth data. The next turn in the same private profile resumes
  normally. A CLI that starts a new conversation in a session with earlier
  messages (another engine answered before, or a branch) gets the canonical
  persisted messages, steering, summaries and completed Clovy tool observations,
  bounded to 24,000 characters, ahead of the new message. Tool observations keep
  call identity, arguments, results and references as context, with secrets
  scrubbed; they grant no approval and never execute a tool. Replaced retry
  ancestry remains excluded, including on later switches, and the stored
  history is unchanged. Returning A to B to A starts fresh on A rather than
  resurrecting its older internal conversation.
- **Stream.** Text, reasoning, and tool activity become stable session events
  (`message.delta`, `reasoning.delta`, `tool.started`,
  `tool.completed`/`tool.failed`, `message.completed`, `run.completed`/
  `run.failed`/`run.cancelled`). The neutral `agent_sessions` module persists
  them so the UI and stored history share the same event/item contract.
  Claude's successful Clovy MCP results keep their original content blocks.
  A structured JSON block after a long Computer use tree therefore keeps its
  action/application references in replay. Existing flattened history stays
  untouched; arbitrary JSON is never searched for inside untrusted prose.
  Antigravity preserves the measured step indices, MCP arguments, output and
  tool errors, deduplicates terminal tool updates, and consumes `result.response`
  when no answer was streamed (or appends its unstreamed suffix). A missing
  terminal verdict or a successful verdict without an answer fails the run.
  A headless `SUCCESS` with an empty response and `denied_actions` reports the
  observed CLI permission denial and action names; it is not a human decision
  in Clovy. No successful tool result is inferred, and CLI stderr suggestions
  to bypass permissions are not followed or forwarded as recovery advice.
  Only `call_mcp_tool` events naming this turn's exact leased server normalize
  to `clovy.<tool>`; inherited/previous servers retain an external MCP origin,
  including one that calls itself `clovy`. A DONE wrapper containing an MCP
  error stays a failed tool; no result is manufactured to settle a spinner.
- **Cancel.** The run's cancellation is registered (`ChatEngineHost::register`)
  before anything is awaited for it, and the turn checks it and the run's
  status again right before spawning, so a cancel during launch preparation
  starts nothing. A winning UI or remote Stop claim synchronously revokes
  the run's turn tokens and every cloned live authority while holding its
  SQLite writer, then signals the host before committing. Broker settlement
  and process teardown cannot delay that admission boundary. The shared
  `chat_engine::cancel_cli_run` also revokes and signals before awaiting the
  run's pending approval receipts (the waiting tool answers `run_cancelled`).
  The executor sends SIGTERM to the CLI's process group, then SIGKILL after
  2 s. This prevents a prepared capture or serialized dictation start after
  Stop; a recording already admitted to the recorder remains active.
  Terminal publication uses `persist_terminal_event` under `BEGIN IMMEDIATE`
  to select the immutable database winner and claim its one publication in
  `run_config_json.clovyTerminalEvent`. Host and fallback publishers share
  this claim; emission uses the accepted sequence and winning terminal method.
  A registered preparation that fails before its task is spawned also owes
  that publication: the chat and routine callers pass its error through
  `finish_cli_dispatch` before releasing the host registration. The ordinary
  event sink and terminal arbiter publish the persisted winner, so a Stop
  that already won stays cancelled and fallback publishers cannot duplicate it.
  The routine's dispatch-failure fallback also preserves terminal history and
  its routine summary when cancellation or completion already won.
  The executor checks cancellation after the final partition authorization
  await, before either spawning the CLI or writing its prompt, on both success
  and error from that query.
  No user configuration is required.
- **Tool terminality.** The same terminal transaction settles unresolved
  `running` tool calls as `interrupted` for the exact session and run, even
  when Stop already claimed the terminal status. A genuine `tool_result`
  paired by that session, run, and outer CLI call id takes precedence; its
  output or error is preserved. No result or effect outcome is synthesized.
  Terminal events include `interruptedCallIds` only when nonempty, using
  those outer call ids rather than the approval broker's separate UUIDs.
  The UI applies them to the matching run's calls, including historical
  calls, without ending a newer run. Persisted hydration and live events
  show the same localized interrupted state in single and folded tool cards.
  A late start cannot recreate running activity in a terminal run; genuine
  late results and approval receipts remain admissible. Interrupted means
  that no final result was observed, not that an effect failed or was undone.
  Database migration 56 applies the same exact pairing and evidence rules to
  historical terminal runs once on upgrade, without a read-time heuristic.
- **End of the CLI.** The CLI's exit is watched alongside its stdout: when it
  exits, its process group is swept (MCP servers and helpers it started,
  including one still holding stdout), and output still in the pipe is read
  for at most 1 s.
- **App shutdown.** The shutdown coordinator (`shutdown::run_cleanup`, also
  used for restart and update) calls `ChatEngineHost::shutdown`: new turns are
  refused, every turn is cancelled, and it waits up to 5 s for each to stop its
  process group and publish `run.cancelled`; each turn's token is revoked as
  its turn ends.
- **Messages during a run.** `steer_agent_run` answers
  `{ accepted: false, reason: "cli_engine" }`; the composer's follow-up queue
  sends the message as the next turn when the run ends.
- **Clovy's tools.** Given to claude, codex, agy, and copilot while the Clovy MCP
  server is on ([mcp-server.md](mcp-server.md)); `chat_engine_catalog` reports
  `clovyTools` (`available`, `server_off`, `unsupported`) and the session says
  when it has none. A bound turn gets the read tools plus the turn-bound
  tools (notes, memory, routines, vault and skills, media, recorder,
  connectors, the MCP gateway, interaction, and, in attended turns, Browser
  use and Computer use); every Clovy write asks the approval broker first and
  an unattended turn's writes are refused (`unattended_requires_approval`),
  see [mcp-server.md](mcp-server.md#tools-for-turn-bound-connections). The
  agent's web search and fetch come from the CLI itself (claude's
  `WebSearch`/`WebFetch`, codex `web_search="live"`). Compaction is refused (`agent_compaction_unsupported`):
  the CLI manages its own context.

## Routines on a CLI engine

Rotinas reúne a criação, a edição e a execução manual. O histórico abre uma
visualização de leitura com o estado e o resultado ou erro da execução. Ela usa
os itens persistidos da execução, incluindo sessões importadas de versões anteriores.

Routines run on an installed Claude Code or Codex engine selected by
`providers::default_unattended_cli`; an unavailable engine produces a clear
setup error without fallback. The claim snapshots `__clovy_cli_engine__:<cli>`.
`routines::start_claim` persists the session, run, prompt and claim mapping,
then calls `chat_engine::start_cli_turn` unattended with the routine's safety
mode. Events use the same session persistence as chat turns and project the
terminal state into routine history. The turn retains unattended tool limits;
old explicit remote-model selections do not revive the removed native loop.

| Command | Args | Returns |
|---|---|---|
| `chat_default_engine` | none | `{ modelId: string, cli: string }` |
| `chat_engine_catalog` | none | `{ clis: [{ id, name, installed, reason?, modelId, clovyTools }] }` |

## Tauri commands

| Command | Args | Returns |
|---|---|---|
| `llm_providers` | none | `{ endpoints, usage, cliLevels, cliDefaultPending }` |
| `llm_detect_clis` | none | six `{ id, name, installed, path?, version?, reason?, structuredOutput, toolsDisabled }` |
| `llm_save_endpoint` | `request: { id?, name, baseUrl, modelId, apiKey?, clearApiKey? }` | registry DTO |
| `llm_delete_endpoint` | `id` | registry DTO (usages pointing at it reset) |
| `llm_set_usage` | `usage`, `provider` | registry DTO, or `llm_structured_output_insufficient`, `llm_provider_not_allowed`, `llm_cli_not_installed`, `llm_cli_tools_not_disabled`, `llm_endpoint_not_found` |
| `llm_test_provider` | `provider` | `{ latencyMs, structuredOutput }` (level persisted) |

`probe_local_generation_endpoint` lists an endpoint's `/models` for the
endpoint form; with no typed key and an `endpointId`, the backend uses the
Keychain key saved for that endpoint (the key never reaches the webview).

## Windows resolution

Windows has no login shell. `llm::shell_env` uses the app's own environment
(an app started from Explorer inherits the user's and the machine's `Path`),
captured once per run like the macOS login environment:

- the search path is the existing variable that matches `PATH` ignoring case
  (normally `Path`); fallback directories are written back to that same
  variable, so a child never sees two;
- `which` tries, in each directory, the name plus each `PATHEXT` extension in
  order (default `.COM;.EXE;.BAT;.CMD`), keeping only `.com`, `.exe`, `.bat`
  and `.cmd`, and never the bare name: npm writes a POSIX `codex` script
  beside `codex.cmd`;
- the fallback directories are where the official installers put each CLI:
  `%APPDATA%\npm` (npm global installs), `%USERPROFILE%\.local\bin` (claude),
  `%LOCALAPPDATA%\Programs\OpenAI\Codex\bin` and `%USERPROFILE%\.codex\bin`
  (codex standalone, current and earlier layout), `%LOCALAPPDATA%\agy\bin`
  (agy), `%LOCALAPPDATA%\cursor-agent` (cursor-agent),
  `%LOCALAPPDATA%\Microsoft\WinGet\Links` (copilot through winget),
  `%USERPROFILE%\.bun\bin` and `%LOCALAPPDATA%\Volta\bin`.

A CLI installed while the app is open appears after a restart, as on macOS.
One-shot calls always send the prompt on stdin, which `.cmd` launchers pass
through byte for byte; for chat turns see [CLI chat engine](#cli-chat-engine).

## Verification

- `cargo test --manifest-path src-tauri/Cargo.toml --locked llm` covers the
  catalog, isolation flags and marker, login-shell resolution with fake shells
  and CLIs, timeouts without orphans, the endpoint probe against a fake HTTP
  server, migration, DTO secrecy, usage validation, activity gating and
  serialization, and a fake-CLI note persisted like a Clovy note. On Windows
  it also covers the `Path` key, `PATHEXT`, the installers' directories, and
  a one-shot prompt through an npm `.cmd` shim.
- `cargo test --manifest-path src-tauri/Cargo.toml --locked chat_engine` covers
  the CLI chat engine with a fake CLI per CLI (two turns with the resume id,
  stream to events and items, cancel without orphans, a message sent during a
  run, the MCP configuration each CLI receives, approval refusals, failures;
  on Windows, multi-line and over-limit prompts reaching cursor-agent and
  copilot `.cmd` launchers intact),
  the permission table and the forbidden-flag sweep (`chat_engine::policy`),
  the turn token (present in each CLI's server arguments, bound to the run,
  revoked at the end) and a write tool approved, denied, and cancelled through
  the broker over a real socket and the relay code (`chat_engine::binding_tests`),
  and historical session routing to the CLI default. The ignored
  `chat_engine_real` tests exercise installed engines with authorized usage:
  `cargo test chat_engine_real -- --ignored --nocapture` runs two
  turns on the installed claude, codex, and pi, and the policy's argv on the
  installed claude and codex with the built `clovy-mcp` and an approval card
  (`chat_engine_real_policy_smoke`, after `cargo build --bin clovy-mcp`).
- The `windows-rust` CI job runs the `llm` and `chat_engine` filters on
  Windows, where the `Path`, `PATHEXT`, and `.cmd` tests compile.
- `pnpm exec vitest run src/test/llm-providers src/test/chat-engine` covers the
  settings section and the engine picker; `src/test/note-chat-default-engine`
  and the default-engine cases in
  `src/test/agent-workspace-runtime` and `src/test/app-shortcut` cover the
  entry points that start on the default engine.
- `cargo test --locked llm::registry` covers the one-time move to the default
  CLI; `cargo test --locked routines` covers a routine on a fake claude under
  the unattended policy, recorded in the routine history.

## Antigravity authentication in Clovy

For an installed Antigravity CLI, Settings offers **Authenticate in Clovy**
(**Autenticar no Clovy**). `openAntigravityLogin(): Promise<void>` invokes
`llm_open_agy_login` without a token payload. The command resolves the installed
program from the real login environment, then opens a new owned OS terminal in
that same app-owned profile. macOS uses Terminal via `/usr/bin/open`; Windows
creates the CLI in a new native console with structured argv and an explicit
child environment. A POSIX launcher is private, contains no inherited provider keys,
and removes itself after the interactive CLI exits.

The SDK conducts OAuth and consent. Clovy supplies no task prompt, consent
bypass, API-key route or other CLI. Opening a terminal is not authentication
success: the button reports opening/errors, and the existing Test action checks
the provider afterward. First authentication is independent of personal SDK
login. Closing a chat, Stop, one-shot failure and transport recovery retain the
SDK's native state. No credential/profile contents belong in review artifacts.
Native terminal/OAuth and Windows runtime acceptance are performed separately
by the parent; portable source tests do not qualify them.
