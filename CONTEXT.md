# Clovy

Clovy is a Tauri desktop app that records meetings/dictation, transcribes
the audio, turns the transcript into structured notes, and hosts an AI agent
you can chat with over those notes. It calls the user-configured providers
directly, with no sign-in, billing service, local bearer token, telemetry or
companion app. Chat runs on installed **CLI chat engines**, with
Clovy tools exposed through the **Clovy MCP server** and local approval broker.

This document is a glossary, not a spec. Terms are canonical; the `_Avoid_`
lines are binding. Implementation, endpoints, and code shape live under
[docs/](docs/index.md).

## Language

### Platform

**Clovy (the app)**:
The user-facing Tauri desktop product — `Clovy.app`, executable `clovy`,
bundle identifier `dev.guilhermexp.clovy`. See
[ADR-0068](docs/adr/0068-clovy-takes-its-own-identity-with-a-one-time-migration.md).
_Avoid_: notetaker, OS Notetaker (legacy names — fully removed from code as of
the earlier bundle rename; don't reintroduce), June as the current product
name.

**Identity migration**:
The one-time copy, on the first launch under `dev.guilhermexp.clovy`, of the
June-era data and config directories, WebView store and credential items into
the new names. All or nothing; the source is never changed.
_Avoid_: move, rename (the old profile stays), upgrade.

**Direct provider**:
An external service called directly by the desktop with the user's key:
**Groq** for audio, **OpenRouter** for remote text, and **fal** for media.
There is no Clovy inference server or service-managed credential fallback.
_Avoid_: proxy, shared service key, automatic fallback.

**Local account**:
The account-free desktop state. It has no server identity or local bearer
token. Operating-system profile isolation and per-provider credentials still
apply.
_Avoid_: signed-in user, local token, authenticated server session.

**Upstream provider**:
The external provider that performs an inference operation. For desktop
operations this is a **direct provider**; a CLI or configured endpoint can
choose its own hosting route.
_Avoid_: the LLM (unqualified).

**BYOK**:
The user's own provider credential, saved in the operating system credential
store and sent only to that provider. Usage is charged by that provider to
that account. No key is substituted when it is missing or rejected.
_Avoid_: service key, credit balance.

**Own provider**:
An installed **agent CLI**, using its own login, or a named
**OpenAI-compatible endpoint** (base URL, model, optional credential-store
key). Notes and activity use these through `llm::generate`; chat accepts only
CLI engines. They never transcribe audio. See
[docs/llm-providers.md](docs/llm-providers.md).
_Avoid_: local model (unless the endpoint actually runs on this machine).

**Provider use (usage selection)**:
The engine assigned to **chat**, **notes**, or **activity**. Chat uses a CLI;
notes use OpenRouter, CLI or endpoint. Activity defaults to **none** and
requires measured `json_schema` output support. Dictation cleanup is fixed to
OpenRouter and has no engine selection.
_Avoid_: default model, active provider (unqualified).

**Text model catalog**:
OpenRouter's models with text output, including models without tools. The
picker selection changes the remote note model, never the chat or Notes
engine. Its description names the active Notes engine.
_Avoid_: chat model picker, tool-capable models (as a text-mode requirement).

**Dictation cleanup**:
The optional pass that punctuates a **dictation** transcript and removes
filler before pasting. Off by default; when enabled, it calls OpenRouter with
the user's key and selected cleanup model. A failure preserves the raw
transcript and is disclosed in the HUD. Never a CLI or endpoint.
_Avoid_: dictation formatting, rewrite (it keeps the speaker's words).

**One-shot generation**:
A single isolated prompt-to-answer call on an own provider (text, or JSON
constrained by a schema), through `crate::llm`. CLI one-shot calls carry the
**authorship marker** `[clovy-internal-ai-call]` so session ingestion skips
Clovy's own calls. Distinct from an **agent run**.
_Avoid_: completion (unqualified), query.

**Chat engine**:
The installed **agent CLI** that answers a chat session, one process per message,
resuming its own conversation. Clovy tools reach a bound turn through the
**Clovy MCP server** and the local approval broker. The session stores
`__clovy_cli_engine__:<cli>` as its engine id. Registered endpoints and the
text model used for notes are separate from chat. See
[docs/llm-providers.md](docs/llm-providers.md#cli-chat-engine).
_Avoid_: backend, model (an engine is not a model), provider (unqualified).

**Default chat engine**:
Claude Code when installed, otherwise the existing Codex fallback. Without
an eligible installed CLI, chat asks for installation or connection; it never
starts a bundled agent or an endpoint. Routines use the CLI default as an
**unattended turn** with their existing tool policy. See
[ADR-0072](docs/adr/0072-cli-chat-replaces-the-native-agent.md).
_Avoid_: native agent, legacy engine (neither is a current chat engine).

**Structured-output level**:
How strongly a provider can be held to a JSON schema: `none`, `prompt`,
`json_object`, `json_schema`, `strict`. Measured by the provider's connection
test (endpoints) or given by the CLI catalog and confirmed by the test.

### Notes

**Note**:
The central Clovy artifact — a persistent, user-editable markdown document,
listed in the sidebar and optionally organized into folders. Created blank or
filled by **note generation**, and owns any manual notes the user types. A
**recording session** is note-backed: the recording attaches to a note, never
the reverse, and a note need not have a recording at all.
_Avoid_: meeting (Clovy has no meeting entity), document, summary.

**Project (folder)**:
The user-facing organizational unit — "Projects" in the UI, the `folders`
entity in code — that groups notes and agent sessions and, since JUN-256,
carries **project instructions** and scopes **memory entries**. One entity,
two names: say "project" in UI copy, keep `folder` in code and schema.
_Avoid_: workspace (overloaded), profile (a different, reverted concept).

**Project instructions**:
The user-written, per-project text (max 4000 chars, on the folder row) that
Clovy follows in sessions filed in that project. Delivered by injecting a
delimited project-context block into the session's prompt text at run
boundaries — never via the global SOUL.md (see
[ADR-0027](docs/adr/0027-june-owned-project-memory-store.md)).
_Avoid_: system prompt (that is SOUL territory), folder description (a
separate, filing-help field).

**Data partition**:
The local isolation boundary used when Clovy reads or writes notes, projects,
agent sessions, memories, and related assignments. `default` is the fallback
partition for data created before partitioning and for stored sessions without
an explicit mapping. The **current data partition** is the partition selected
for the active app view.
_Avoid_: profile, active profile, tenant, account.

**Memory entry**:
One durable fact Clovy remembers, stored in Clovy's own SQLite `memories`
table — global or scoped to one project — written by the agent through
`june_context` memory tools (or by the user in Settings), inspectable and
editable in the "Memory" settings tab and the project detail view. Deletion
is a hard DELETE plus a tombstone row (future-proofing for any later
multi-device sync; nothing reads tombstones today).
_Avoid_: skill-owned `memories/` files, Biography (the connector-built user
profile).

### Audio & recording

**Recording session**:
One note-backed capture lifecycle (a UUID) that owns its source mode,
artifacts, elapsed time, and status; the unit of recovery and retry.
_Avoid_: meeting object (Clovy deliberately has no separate "meeting" entity).

**Source mode**:
The capture scope chosen before recording starts: `MicrophoneOnly` or
`MicrophonePlusSystem` (meeting mode).
_Avoid_: recording type.

**Meeting video (vídeo da reunião)**:
An optional, silent recording of the detected meeting window, owned by a
**recording session** alongside its audio artifacts. It follows the session's
pause and resume and records an offset against audio time. Only the Claude
Notes engine can analyze it through the local `video-analysis` skill; other
engines produce a transcript-only note with a persisted notice.
_Avoid_: screen recording (it captures one window, not the display), video
Source (a Source is an audio lane), meeting object.

**Source**:
A single audio lane — `Microphone` or `System` — each captured to its own
file and transcribed independently.
_Avoid_: channel (that is the WAV interleave sense), track.

**Turn**:
A detected active interval on one source (`source`, `start_ms`, `end_ms`,
`turn_index`) used to order the transcript as a back-and-forth conversation.
Detection is energy-based (RMS windows + noise floor), never diarization.
_Avoid_: segment (that is a live-preview chunk), utterance, speaker (no
speaker identity is inferred).

**Transcription engine** (OpenSpec: _motor de transcrição_):
The selected implementation that converts audio to text for dictation, note
Turns, fixed chunks, Turn packs, and Live transcript preview. Groq uses the
user's key; Local (Parakeet Ultra) runs on-device on macOS 14+ Apple Silicon.
It is independent of the Notes engine and Dictation cleanup. A Local error
never selects Groq automatically.
_Avoid_: chat engine, Notes engine, offline app (other features may use a network).

**Local transcription model** (OpenSpec: _modelo local_):
The pinned Parakeet Ultra Core ML files installed explicitly through Settings.
Their installation state is absent, downloading, ready, or failed; readiness
is required to select Local. The model id is `parakeet-ultra`.
_Avoid_: downloaded provider, local Groq, embedded model (it is optional).

**Turn pack** (OpenSpec: _pacote de turnos_):
Consecutive Turns of one Source sent to Groq as one final-transcription
request of up to 600 s, 0.5 s apart, and split back per Turn by the
provider's segment times. Each Turn keeps its own durable job; only the
Groq route with the user's key supports packs. Local sends one request per Turn
to avoid assigning text across imprecise token boundaries (ADR-0074 addendum).
_Avoid_: batch, chunk (a chunk is a fixed split of one long audio file).

**Speaker bleed (echo)**:
System audio re-captured by the microphone after playing through the
loudspeakers — "speaker" is the device, never a person; no speaker identity
is inferred. Echo rejection trims bleed spans out of Microphone turns on
signal evidence (lag-aligned similarity, cancellation depth, level
dominance); the speech stays attributed to the System source, and no
downstream step may reintroduce trimmed audio.
_Avoid_: crosstalk, feedback (that is the amplification loop), AEC as the
concept name (it names the canceller mechanism, one evidence tier).

**Coalescing**:
Merging adjacent same-source turns before transcription when the gap is short
and no other source intervenes. Distinct from `merge_close_turns` (intra-turn
gap fill).
_Avoid_: merging (unqualified).

**Normalization**:
Preparing a source WAV for transcription: downmix to mono, resample to 16 kHz,
apply bounded gain toward a target peak.
_Avoid_: conversion, resampling (that is one step of it).

**Live transcript preview**:
Optional, ephemeral chunked transcription shown while recording. Revisable,
never written to `transcripts`, never the note's source of truth (see
[ADR-0002](docs/adr/0002-live-transcript-preview-strategy.md)). The Settings
control for it is labeled **Live transcription** (default on); its disclosure
copy discloses extra provider usage with Groq and on-device processing with Local, and
turning it off stops the preview lanes (gated at capture start and re-checked
per chunk, so it also takes effect mid-recording) (JUN-375, ADR-0002
addendum).
_Avoid_: realtime transcription, live captions, streaming.

**ASR hint**:
The bounded dictionary spelling guidance supplied to speech recognition,
distinct from the complete context used to correct recognized text.
_Avoid_: full correction context, instructions, previous transcript.

**Recording mark**:
A user-created timestamp and optional label on a note-backed recording session.
The timestamp uses the recorder's active elapsed time, excluding pauses; the
mark stays attached to its original note and session after final transcription.
_Avoid_: transcript turn (a mark is not detected speech), meeting object.

**Recording assistance**:
An explicitly requested question grounded in recent live transcript preview
from the current recording session. Uses the selected Notes engine in a
tool-free one-shot call; it does not authorize an agent run or a new provider.
_Avoid_: automatic tool trigger, decision (a suggestion is not an agreement).

**Correction context**:
The authorized dictionary and prior same-source transcript used by the
notes text engine to correct a note transcript; dictation cleanup
retains its own complete dictionary context. It is not an ASR hint and does
not include another audio source's transcript.
_Avoid_: ASR hint, recognition prompt, cross-source transcript.

**Transcript coverage**:
How much of a recording's detected speech ended up in persisted, successful
note-transcription turns (`transcribed_ms` vs `detected_speech_ms`). Always
measured against detected speech spans, never wall-clock recording duration —
silence is not lost audio. Persisted per processing pass as a
`transcript_coverage` checkpoint; surfaced on the note (non-blocking) when
materially incomplete.
_Avoid_: transcript completeness, duration coverage (wall-clock framing).

**Note transcription job**:
A durable unit of saved-audio processing for one Source and exact time range.
Its stable Source-span identity is separate from `turn_index`, which is only
presentation order; its versioned input fingerprint decides whether succeeded
text may be reused. Jobs are workflow state, while transcript rows are the
current user-visible output projection.
_Avoid_: preview segment (ephemeral live-preview input), provider request (one
job may make transient retries or bounded chunk requests), transcript row (the
committed result, not the work).

**System audio helper**:
The out-of-process macOS `.app` (`june-system-audio-recorder`) that captures
system audio via CoreAudio process taps and reports over a `status.json` file
(see [ADR-0004](docs/adr/0004-out-of-process-system-audio-helper.md)).
_Avoid_: system driver, in-process capture.

**Dictation helper**:
The platform-native helper (`mac-dictation-helper` on macOS,
`june-dictation-helper.exe` on Windows) that owns push-to-talk **dictation**
capture and text insertion into the **paste target**. It is the authoritative
source for helper-owned microphone state, destination selection, and platform
paste readiness. On Windows, an exact Dictate-button request or a global
shortcut started while Clovy's composer owns focus may be delivered through
acknowledged in-app insertion, but only when the helper verifies the initiating
Clovy window's exact handle and original process identity. The foreground window
at stop is irrelevant to this logical in-app destination and may be the
non-focusable dictation HUD.
On macOS it is also authoritative for Accessibility permission state.
_Avoid_: dictation app, keyboard helper.

**Dictation take**:
One helper-owned dictation capture from `listening_started` until its text is
delivered or the capture is discarded. Clovy gives each take an opaque
identity before asking the helper to start; a helper paired with an older
coordinator mints the identity itself. The identity lets Clovy cancel pending
dictation transcription, cleanup, history, and delivery without affecting a
newer take. Cancellation is terminal; a late result never regains delivery
authority.
_Avoid_: recording session (that is note-backed), utterance (that identifies a
provider request, not the user-visible capture lifecycle).

**Paste target**:
The destination where a finished dictation transcript is delivered. For native
paste, it is the exact app window pinned when recording stops and never
re-resolved afterwards (see
[ADR-0014](docs/adr/0014-pinned-dictation-paste-target.md)). Pinning matters
because the dictation round trip (capture, then dictation transcription, then
cleanup) can outlast the user's attention: the frontmost app at paste time is
often no longer the one they dictated into.
On Windows, the helper may select acknowledged in-app composer insertion for a
correlated Dictate-button request or a global shortcut pre-registered while the
composer owns focus. The helper requires the exact initiating window handle and
original process identity. The request id and captured draft key pin the in-app
destination; all other targets retain exact-window activation and synthetic
paste.
_Avoid_: foreground app, frontmost app (both name a live value, not the pin);
focus target.

### Agent sessions

**Home conversation**:
The retired conversation surface of the desktop app. Its backing session,
items, artifacts, and legacy local storage are historical data. Today is the
desktop entry point, including when a legacy chat destination is opened.
_Avoid_: dashboard, default agent session, Hermes Home.

**Agent sessions**:
The engine-neutral Rust module that owns session and run records, ordered
items, persisted events, and historical imports. CLI orchestration belongs to
`chat_engine`; tool authority belongs to the MCP server and approval broker.
_Avoid_: agent harness, model loop (neither describes session persistence).

**Runtime mode**:
The per-session execution policy: `sandboxed` or `unrestricted`. CLI launch
flags enforce the engine's supported policy; Clovy tools independently enforce
local grants and approvals through the broker. See the permission table in
[docs/llm-providers.md](docs/llm-providers.md#cli-chat-engine).
_Avoid_: permission, profile.

### Legacy agent data

**Legacy agent home**:
The preserved on-disk home from versions released under the June name that
used Hermes. The migration reads its state database once, transactionally,
then retains the directory only as recovery data. The current app never starts
or reads the retired runtime after a successful import.
_Avoid_: runtime home, active profile.

**Browser use**:
The consent-gated capability that lets the agent operate Clovy's own pinned
browser through a private agent-browser host. Attended tasks use an exclusive
persistent profile with logins separate from the user's usual browser.
Routines use anonymous ephemeral profiles and their own public-web grants.
Every tab and reference belongs to one conversation/run; Stop ends control
and processes while retaining the attended profile. Clicks, fills and key
presses use the central approval broker because page metadata cannot prove an
input event harmless. Background is the default; human takeover opens
the dedicated browser visibly and suspends agent capture/input until resume.
_Avoid_: web browsing (that is `june_web` search/fetch), browser toolset
(the upstream runtime feature Clovy does not expose).

**Browser access grant**:
The stored, on-device opt-in that allows Browser use in attended sessions.
Turning it off disables attended browser-tool access through the Clovy MCP
server and ends active browser broker sessions.
_Avoid_: browser permission, browser toggle.

**Routine Browser use opt-in**:
The default-off, per-routine grant that lets one sandboxed routine use Clovy's
anonymous ephemeral browser for public pages. It is distinct from the attended
Browser access grant and never exposes the user's signed-in browser session.
_Avoid_: routine browser permission, global routine browsing.

**Computer use**:
The consent-gated capability that lets an attended CLI turn operate authorized
macOS or Windows app windows through Clovy's Rust broker and the privately
bundled, source-pinned agent-desktop engine. Background input is the default.
An operation that cannot run in the background may use visible focus/pointer
input in the authorized window, selected before delivery or after definitive
unsupported/not-delivered evidence. Delivered or uncertain input is never
replayed in another mode. Visible mode grants no administrator, system
permission, credential or login access.
The first access to each verified target app requires one authorization for
the current attended task. Local grants, the pinned helper and real platform
readiness determine availability: macOS permissions or the normal interactive
Windows desktop and compatible integrity. Bringing a parked macOS
window into Clovy's current Stage Manager group is part of that authorized app
use and does not ask again.
_Avoid_: desktop automation (vague), computer_use toolset (that is the
upstream mechanism, not the Clovy capability).

**Computer use cursor**:
The small, semi-transparent, click-through pointer Clovy shows at Computer use's
virtual action position while an attended task is active. It follows clicks and
drags reported by the signed helper but never moves or follows the user's real
pointer.
_Avoid_: mouse cursor (ambiguous with the user's real pointer), cursor movement
(the real pointer does not move).

**Stored session id** vs **runtime session id**:
The persistent id Clovy keys all UI and history on, versus the live process's
per-resume conversation id. The CLI conversation id is saved in run
configuration; conflating it with the stored id attaches history to the wrong
identity.
_Avoid_: "the session id" (always say which).

**Archived session**:
An agent session the user has archived out of the live lists (the product
verb is **archive**; renamed from "completed" 2026-07-27). The mark is
**Clovy-owned local state** keyed by the stored session id, persisted as
`completed_sessions(session_id, completed_at)`, and set only by Clovy.
Archived sessions leave the sidebar entirely and surface under the sessions
page's Archived status filter. Distinct from both the runtime session
*status* `completed` (the agent finished running) and Hermes' `archived`
flag (a runtime read-filter Clovy never writes). See
[ADR-0032](docs/adr/0032-session-completion-june-owned-local-state.md),
including its 2026-07-27 addendum.
_Avoid_: completed (in user-facing copy; internal persistence names keep
completed_at), "archived" for the Hermes flag without saying Hermes.

**Agent run**:
The execution of one CLI chat request, including its tool activity, from
submission to a terminal event. Pending tool approvals belong to that run.
One agent run keeps one captured CLI, safety mode, skill set, and MCP policy.
A later picker choice is applied at the next agent-run boundary, never inside
the active run.
_Avoid_: response (only the visible text), turn (ambiguous with transcript and
conversation turns).

**Composer**:
The ProseMirror chat input with slash commands and attachment chips.
_Avoid_: textbox.

**Interrupted tool call**:
A tool call whose agent run ended without an observed final result. Its
`interrupted` status ends the running indicator while preserving the call's
identity and arguments. It does not establish whether the effect happened,
failed, or was rolled back. A genuine result remains the observed evidence.
_Avoid_: failed tool, cancelled effect, rolled-back action (without evidence).

**Issue report** (removed):
The in-app bug / feedback report dialog and its remote submission were
removed from the desktop; see
[ADR-0012](docs/adr/0012-direct-issue-report-submission.md). Don't reintroduce
the term for new work.

**Slash command**:
A `/name arg` handled client-side before submit — builtin `/model`, `/file`,
`/image`, and `/video`, plus skill slash commands. `/image <prompt>` starts
Clovy's image generation fast path without invoking the model (kill switch:
`IMAGE_GENERATION_ENABLED`); `/video <prompt>` starts the video generation fast
path (kill switch: `VIDEO_GENERATION_ENABLED`).
_Avoid_: gateway command.

**Steer**:
A user instruction delivered into a still-running agent session without
interrupting it. A steer is first-party and local to Clovy — never a Hermes
wire frame. A steer the run never consumed is resent as an ordinary follow-up
when the run completes cleanly, and dropped when the run fails or is
cancelled.
_Avoid_: interrupt/stop (a steer never halts the run), follow-up (that is the
fallback delivery, not the steer), mid-run message.

**Model capability**:
A flag describing a supported model action, derived from the current
catalog's modalities and supported parameters, never from marketing traits.
An aggregate catalog flag does not prove support on every effective hosting
route; see [ADR-0007](docs/adr/0007-model-capability-source-of-truth.md).
_Avoid_: trait, guaranteed hosting-route capability.

**Attachment**:
A file or image referenced by path. Agent-composer attachments are copied by
Rust into the Clovy-owned session workspace before a run starts. Image
attachments are included as vision input when the selected model supports
vision; other files are exposed through the same safety-controlled file tools.
Native-picker issue-report attachments keep their original local paths.
_Avoid_: upload (unqualified).

**Note reference**:
A plain-text token — `@note:<id>`, optionally followed by the quoted note
title — that points the agent at one specific note. Inserted as a composer
chip via `@`, seeded by "Ask Clovy" on a note, or pasted from "Copy note
reference"; the agent resolves it on demand through the Clovy MCP server's
`get_note` tool (see [docs/agent-note-tools.md](docs/agent-note-tools.md);
[ADR-0010](docs/adr/0010-note-references-in-agent-chat.md) records its origin).
_Avoid_: note link (no URL or deep link is involved), note mention (the chip
is UI; the reference is the token).

**Skill / Toolset / MCP server**:
A Skill is a bundled/installed capability pack; a Toolset is a togglable tool
group; an MCP server exposes tools to a client. Clovy-owned capabilities are
served through `clovy-mcp`, with local policy and the approval broker as their
authority ([ADR-0072](docs/adr/0072-cli-chat-replaces-the-native-agent.md)).
User-supplied external MCP servers configured in Clovy remain supported through the `mcp_`
dispatch prefix. Connector toolsets include `june_gmail`, `june_gcal`,
`june_notion`, and their `*_actions` counterparts. Linear's official hosted
MCP supplies a dynamic external tool inventory under the `mcp_linear_*`
runtime namespace; `june_linear` and `june_linear_actions` remain historical
routine toolset identities, not Clovy-owned MCP servers. `june_notion` is the
hosted Notion MCP read toolset; `june_notion_actions` is the separately
approval-gated create/update toolset.
_Avoid_: using "tool" for all three.

**Obsidian plugin**:
The Clovy-owned local capability for discovering the user-selected Obsidian vault
at task time through the `get_obsidian_vault` host tool. The vault selection is
stored in Clovy-owned `obsidian.json`; it is not a runtime environment variable.
Discovery is current state, not write authorization. In Sandboxed mode, only
`list_files`, `read_file`, and `search_files` may access the vault; writes are
host-denied. Disconnect removes future discovery but cannot revoke a path
already disclosed to a live unrestricted runtime.
_Avoid_: Obsidian connector, `OBSIDIAN_VAULT_PATH`, `june_obsidian` MCP server.

**Plugin**:
A user-facing capability bundle in Clovy's Plugins area. A plugin may combine
Skills, Toolsets, Clovy-owned host tools, routine templates, and optional
Connectors around one job. Enabling or installing the bundle is distinct from
connecting a third-party account and from choosing a routine's trust mode.
The ranked portfolio and shared product contract live in
[docs/plugins/portfolio.md](docs/plugins/portfolio.md).
_Avoid_: connector (unless specifically naming its third-party account path),
integration (too broad), plugin for a Tauri framework package.

### Connectors

**Connector**:
A private-by-architecture integration between Clovy and a third-party account
(shipped: Google Gmail + Calendar, Linear, GitHub, and the Notion hosted MCP
preview). The user authorizes the provider on their Mac; local connector
credentials remain in OS credential storage, and Rust owns direct provider or
hosted MCP calls. External MCP servers receive credentials only on
device-originated requests and never persist Clovy's local custody. Google
ships `june_gmail` and `june_gcal` read servers plus their `*_actions`
counterparts. Linear uses its official hosted MCP with workspace-wide OAuth
access; Clovy dynamically registers its valid tools and approval-gates anything
not clearly annotated read-only. Notion preview ships `june_notion` for hosted
MCP reads and `june_notion_actions` for approval-gated page creation and
updates. GitHub ships `june_github` reads plus approval-only
`june_github_actions` issue writes through a GitHub App user grant. See
[ADR-0016](docs/adr/0016-private-connectors-local-mode.md),
[ADR-0033](docs/adr/0033-notion-hosted-mcp-connect-preview.md), and
[ADR-0036](docs/adr/0036-github-connector-app-user-tokens.md), and
[ADR-0050](docs/adr/0050-linear-official-hosted-mcp.md).
_Avoid_: integration (unqualified), plugin, the Google API.

**Clovy-managed MCP OAuth grant**:
Provider authorization whose credentials Clovy holds in the operating-system
credential store for one custom MCP server. The committed primary credential
reference identifies the grant eligible for calls. A newly received grant stays
in temporary custody until its catalog and connection checks pass. An earlier
grant's unresolved revocation is a separate, durable warning tied to its
affected reference and revision; it neither authorizes nor blocks a separately
authorized current grant.
_Avoid_: treating a historical revocation warning as the current connection's
authorization state, calling temporary custody a connected server.

**Linear workspace access**:
The workspace-wide `read` and `write` OAuth grant Clovy uses with Linear's
official hosted MCP. Clovy does not add a selected-team authorization boundary
or maintain a provider-specific tool allowlist. The hosted server controls the
tool inventory; Clovy validates each descriptor, runs clearly read-only tools
directly, and requires approval for every ambiguous or mutating tool.
_Avoid_: selected teams, team scopes, team filter.

**Local mode**:
The default (and, in v1, only) connector trust model: the OAuth grant is
minted to the device and stored in the Keychain, and connector calls go
straight from the device to the provider. OpenSoftware holds no credential that
can read the user's mail and is not in the *connector* data path. (Routine
model inference runs on the installed CLI selected for the routine. The "not in the data path" claim covers token
custody and provider calls, never inference.)
_Avoid_: on-device mode, private mode (unqualified). Contrast with **away
mode** (the proposed Phase 3 TEE relay, not yet shipped).

**Trust mode**:
The per-routine governance of *outward* connector actions:
`read_only` -> `approval` (default) -> `autonomous`. Read-only routines get
only the read servers; approval routines route every mutating call through
Clovy's own approval surface; autonomous routines execute granted tools without
prompting. Distinct from **Runtime mode** (Sandboxed/Unrestricted), which
governs *local system access* — the two are never conflated.
_Avoid_: permission level, autonomy level (say **trust mode**), mixing with
Sandboxed/Unrestricted.

**Earned autonomy**:
The rule that a routine may only be switched to the `autonomous` **trust mode**
after it has run correctly under `approval` at least three times. Converts
Clovy's "the agent can make mistakes" honesty into a mechanic.
_Avoid_: auto mode, trust score.

**Trigger** (connector):
A typed event that fires a routine outside the cron schedule:
`email_received` or `event_upcoming`. Produced by the on-device trigger daemon
polling Gmail history deltas and upcoming calendar events, delivered by
claiming the active routine through Clovy's durable routine scheduler. A
far-future timer schedule prevents duplicate timer runs. A trigger is a wake-up,
not a payload: the routine re-reads state through its tools.
_Avoid_: webhook (local mode has none; that is away mode), push, notification.

**Biography**:
The editable profile Clovy builds on first connect from on-device notes and
transcripts plus the user's mail and calendar ("here's what I already know, and
it never left your Mac"). Stored locally, feeds the soul's context, fully
deletable and regenerable.
_Avoid_: profile (overloaded with **provider settings** and the account
snapshot), persona, memory (that is the Clovy **memory entry** store; the
upstream agent memory toolset is a third thing again — qualify which).

**Admin surface**:
A Clovy-native management view for skills, MCP servers, connector health, and
similar settings. Rust owns configuration, secrets, and safety policy. The UI
exposes capabilities rather than process diagnostics.
_Avoid_: admin panel, runtime UI.

### AI work & billing

**Dictation**:
A latency-critical Clovy mode where the user pushes-to-talk, speaks, releases,
and expects cleaned-up text inserted into the **paste target**. A short phrase
round-trips in a few hundred milliseconds; a sustained block of speech can take
many seconds, and everything on the paste path must stay correct across that
whole window. Distinct from **note transcription**. Audio goes directly to
Groq with the user's own key; missing keys stop the take with setup guidance.
_Avoid_: speech-to-text (too generic — covers both dictation and note
transcription).

**Note transcription**:
Clovy records a full meeting or capture session, then transcribes the saved
audio as a single batch operation and runs **note generation** on the
transcript. Higher latency tolerance than dictation; cost typically dominates
dictation by 100×+ per call.
_Avoid_: transcription (ambiguous between dictation and note transcribe — say
which).

**Note generation**:
The step that turns a note transcription and manual notes into structured
markdown using the selected text engine, with remote text routed through
OpenRouter. It follows successful note transcription and is distinct from
dictation cleanup.
_Avoid_: notes generation, AI summarisation.

**Interface language** vs **transcription language**:
The **interface language** is the language Clovy's own chrome is written in
(labels, dialogs, toasts, HUDs, native app and menu bar menus): English or
Português (Brasil), chosen in Settings > Appearance and stored per machine as
`os-clovy:interface-locale`. The **transcription language** is the optional
language hint sent with dictation and note transcription (Settings >
Dictation). The two are independent: changing one never changes the other, and
neither translates user content or model output. See
[docs/i18n.md](docs/i18n.md) and
[ADR-0056](docs/adr/0056-interface-language-uses-an-in-repo-typed-catalog.md).
_Avoid_: "language" alone when the context does not make clear which one;
"locale" for the transcription hint.

**Image generation**:
Producing a new image from a text **prompt** through fal, using an explicit
`/image` action or an assistant tool call. It is distinct from **image
editing**; see [ADR 0008](docs/adr/0008-image-generation-and-editing-tools.md).
_Avoid_: rendering, drawing (say **image generation**).

**Image editing**:
Transforming an existing image plus an instruction through fal's editing
model, preserving its oriented aspect ratio. It is distinct from **image
generation**, which starts from a text prompt.
_Avoid_: image-to-image jargon, regenerate (that's a fresh **image generation**).

**Video generation**:
Producing a new video from a text **prompt** through fal's asynchronous
job flow, using `/video` or an assistant tool call. It is distinct from
**image-to-video**; see [ADR 0015](docs/adr/0015-video-generation-tools.md).
_Avoid_: txt2vid jargon, rendering (say **video generation**).

**Image-to-video**:
Producing a video by animating an existing image plus a prompt through fal.
It is the video counterpart of **image editing**, distinct from text-only
**video generation**.
_Avoid_: img2vid jargon, animate (unqualified — say **image-to-video**).

**Safe mode**:
The shared per-device image/video preference requesting the upstream
model's supported content safeguards. Turning it off is a request, not a
promise of unfiltered output or permission to bypass the provider's
account restrictions; see [ADR 0008](docs/adr/0008-image-generation-and-editing-tools.md)
and [ADR 0015](docs/adr/0015-video-generation-tools.md).
_Avoid_: NSFW filter/toggle (say **safe mode**), censorship, "video safe
mode" (there is only one safe mode).

**Provider price**:
The provider's listed price for its model or operation, expressed in dollars
and its stated unit. A catalog estimate is not proof of an account's charge.
Clovy has no credit wallet, hold, funding gate, or settlement service.
_Avoid_: credit price, Clovy credits, free inference (from a missing price).

### Clovy companion (removed)

The desktop no longer has the companion: pairing, the encrypted transport,
**linked devices**, remote Computer use approvals and the "Linked devices"
settings tab and hosted relay were removed. Historical companion protocol
crates remain outside this cut; their wire terms live in
[docs/companion-protocol.md](docs/companion-protocol.md). Don't use the
companion terms for new desktop work.

### Desktop shell

**Release channel** / **Update manifest** (removed):
The in-app updater and its `stable` / `rc` channel choice were removed from the
desktop (see [ADR-0001](docs/adr/0001-auto-updates-via-tauri-updater.md) and
[ADR-0003](docs/adr/0003-release-candidate-channel-and-promotion.md)); the app
does not check for or install updates.

**Releases repo**:
The separate public repo `open-software-network/os-june-releases` that hosted
signed artifacts and the update manifest for the removed updater (see
[ADR-0055](docs/adr/0055-clovy-technical-identity-migrates-through-a-compatibility-bridge.md)).
_Avoid_: "GitHub release" (unqualified).

**Provider settings / Model mode**:
The persisted model selection for each remote action, separate from
**provider use** choices for CLI and endpoint engines. Current upstreams
are Groq for audio, OpenRouter for remote text, and fal for media. Web search
belongs to the CLI engine.
_Avoid_: model config (unqualified).

**Account snapshot** / **Avatar** / **AccountGate** / **FundingGate** (removed):
The account state, the generated account avatar, the sign-in wall and the
credits/upgrade wall were removed with OS Accounts sign-in; the app runs on the
**local account**.

**Agent HUD**:
The floating agent overlay window, toggled from the menu bar and separate
from the main window.
_Avoid_: pet (legacy name — survives only in an old storage key), overlay
(unqualified), floating window.

**Permission**:
A platform grant Clovy needs for native capture or insertion. On macOS these
are TCC grants: microphone, accessibility, or screen/system audio recording.
TCC grants are bundle-scoped, so the authoritative source is the bundle that
captures: the dictation helper for dictation mic + accessibility state, the
main app's own `AVCaptureDevice` authorization for note-recording mic state
(the helper's grant never covers the main app). System-audio permission is
probe-driven (there is no query-only macOS API). On Windows, microphone access
is controlled by Windows privacy settings and dictation paste does not require
macOS Accessibility.
_Avoid_: entitlement (that is the code-signing sense), treating one bundle's
mic grant as covering the other.

### Activity capture

**Activity capture**:
macOS-only, text-only recording of what the user sees and does, stored in the
**activity database** (see [docs/activity-capture.md](docs/activity-capture.md)).
On by default; turning it off is the user's lasting choice. Missing macOS
permissions block recording without turning it off. Never stores pixels.
_Avoid_: screen recording (that is the TCC permission and the meeting feature),
tracking, monitoring.

**Activity database** (`activity.sqlite3`):
The SQLCipher-encrypted SQLite file holding activity data, separate from
`notes.sqlite3`; its key lives only in the Keychain (see
[ADR-0057](docs/adr/0057-activity-database-uses-sqlcipher-linked-app-wide.md)).
_Avoid_: "the database" unqualified (that means `notes.sqlite3`).

**Frame**:
One 2-second sample of the focused window: time, app, window title, browser
URL, text, and its **text source** (`accessibility` or `ocr`). A **secondary
frame** is a ~10-second sample of another display's top window; it never
splits the focused session.
_Avoid_: screenshot, snapshot (no image is kept).

**Input event**:
A click, key, app switch, window focus, or clipboard change recorded with only
time, app, and a count. Clipboard text is the only content kept, truncated and
redacted.
_Avoid_: keystroke log, keylogging.

**Pause record**:
A row stating when capture paused and why (`manual`, `work_hours`, `low_disk`,
`protected_video`), so the timeline can tell a pause from inactivity.
_Avoid_: gap (a gap is the timeline's inferred absence of frames).

**Processing cursor**:
Per-consumer high-water mark (`frames.id`) of activity the timeline has
processed. Retention deletes only rows at or before it.
_Avoid_: checkpoint, offset.

**Activity session**:
A run of consecutive useful frames of one app and **session context**, ended
by an app switch, a context switch, or a **gap**. At most one is *active* (still
growing); a closed one has a **category**. Its duration never includes a gap
(see [docs/activity-timeline.md](docs/activity-timeline.md)).
_Avoid_: session unqualified (that is the agent's chat session), visit, block
(a block is only its drawing in the view).

**Session context**:
The browser's domain or the editor's workspace of an activity session; a new
one in the same app starts a new session.
_Avoid_: project (that is the agent's project), tab.

**Useful frame / idle frame**:
A frame is useful when the app, window, URL, or text changed since the
previous frame, or the user gave input around it; otherwise it is idle. Idle
frames extend nothing.
_Avoid_: active frame (active describes the open session).

**Gap**:
More than 5 minutes without a useful frame, classified as `idle` (idle frames
cover most of it), `sleep` (no frames: the Mac slept or capture was off), or
`paused` (a **pause record** covers most of it, with its reason: manual, work
hours, low disk, or protected video). Gaps are never session time. While it is
still going on it is an *ongoing gap*, shown up to now but stored only when
activity resumes.
_Avoid_: pause (that is the capture's own record), break.

**Category**:
One of ten deterministic classes a session gets from scored evidence (coding,
code review, meeting, communication, design, documentation, planning,
deployment/devops, research, idle/personal); below 0.35 confidence it is
idle/personal. No model is called.
_Avoid_: tag, label, classification by AI.

**Today view**:
The desktop entry point and first-level sidebar view showing one day's activity timeline (sessions,
gaps, stats, search) and, in its "Day summary" section, the day's
intelligence. "Today" is its name even when another day is selected.
_Avoid_: dashboard, journal.

**Clovy MCP server** (`clovy-mcp`):
The stdio MCP server that lets local clients read the current **data partition**'s notes,
dictations, and memories, and the installation's activity (activity has no
partition, [ADR-0059](docs/adr/0059-activity-is-per-installation-not-per-data-partition.md)).
The binary relays to the running app over an authenticated local channel
(Unix socket on macOS, private local named pipe on Windows) and
answers only while Clovy is open with the server on, which it is by default
until the user turns it off (see [docs/mcp-server.md](docs/mcp-server.md)). It
is independent of **activity capture**: with capture off it still serves notes,
dictations, and memories, without activity. Authenticated clients can request
applicable actions through an **external MCP approval**. Hosted CLI consumers
continue to use **turn bindings** and their own policy.
_Avoid_: Clovy-managed MCP server (the retired in-loop `june_*` servers), MCP
bridge, plugin.

**External MCP approval** (OpenSpec: _aprovação externa_):
A one-shot decision in Settings → MCP for an exact operation requested on an
authenticated local MCP connection. The host binds the request to its connection
and data partition. The request expires if authority is lost before the effect.
The preview shows the operation and its target, and the client gets the result.
_Avoid_: session approval, permission grant.

**Turn binding** (OpenSpec: _vínculo de turno_):
The link between one Clovy MCP server connection and the CLI chat turn that
opened it: a single-use **turn token** the app issues per run and passes to
`clovy-mcp` as `--turn <token>`, bound to the session, run, CLI, safety mode,
and whether the turn is attended (chat) or unattended (routines). Only a bound
connection sees the turn-bound tools (`turn_required` otherwise); the token is
revoked when the run ends, fails, or is cancelled (`turn_expired` after). It
never replaces the installation secret. See
[docs/mcp-server.md](docs/mcp-server.md#turn-binding).
_Avoid_: session token, MCP session (the token lives one run, not one
session), auth token (the installation secret authenticates).

**Approval broker** (OpenSpec: _broker de aprovação_):
The app service (`crate::approval_broker`) every Clovy write tool of a CLI
turn asks before acting: it records an approval or clarification card in the
session, waits up to 10 minutes for one decision, and turns denial, expiry,
or cancellation into the tool's error (`user_denied`, `approval_timed_out`,
`run_cancelled`). Unattended turns are refused without a card
(`unattended_requires_approval`). Browser use asks this same broker for
all click, fill and key-press actions; its separate session controls only manage human takeover,
resume and Stop.
See [docs/mcp-server.md](docs/mcp-server.md#approval-broker).
_Avoid_: permission prompt (that is the CLI's own; claude's is routed to the
broker through `approve_action`), interruption (the item the broker writes,
not the broker), auto-approve (nothing pre-grants).

**Coding agent**:
A third-party AI coding tool the user runs on the Mac (Claude Code, Codex,
Copilot CLI, Copilot in VS Code, Cursor, Cursor CLI, Antigravity) whose local
transcripts Clovy reads, read-only, when its source is on (see
[docs/coding-agent-sessions.md](docs/coding-agent-sessions.md)).
_Avoid_: agent unqualified (that is Clovy's own agent), assistant.

**Coding-agent block**:
A slice of one coding-agent session stored in the **activity database**. It
always starts at a user prompt: a prompt opens a new block after more than
1 hour of silence or once the open block is 1 hour long; replies and tool
output join the open block. It moves `live` → `sealed` → `summarized`; sealed blocks never
change.
_Avoid_: session (one session can span many blocks), segment (the
implementation term), chunk.

### Day intelligence

**Activity provider**:
The provider (CLI or endpoint) chosen for the activity usage in Settings,
Models. Activity text goes only to it; with none chosen, day intelligence is
inactive and nothing leaves the Mac (see
[docs/day-intelligence.md](docs/day-intelligence.md)).
_Avoid_: AI, the model.

**Distillation**:
The local reduction of an hour's captured text before any model call (junk and
prose gates, frequency cut, lexical and semantic dedup with on-device
embeddings, diverse pick, entity rescue). No network except the one-time
embedding model download.
_Avoid_: summarization (that is the model's job), compression.

**Hour report**:
One completed local hour with activity, described by the activity provider in
one call: a short summary and its activities. The minutes shown are Clovy's:
the hour's measured active minutes split by the model's estimates as weights.
_Avoid_: worklog, timesheet.

**Workstream** (pt-BR "frente de trabalho"):
A thread of work in one day, grown hour by hour by an anchored fold: an hour
joins an existing workstream or opens one; titles and other workstreams are
never rewritten. Its minutes are the sum of its hours' measured minutes.
_Avoid_: task, ticket, project.

**Sitting** (pt-BR "período"):
Where a workstream's work was located in time: the bounds of the measured
sessions its activities named, joined. It locates the work, it does not
measure it: one session can hold several workstreams, so a sitting has no
duration of its own and the workstream's active minutes never come from it.
A workstream resumed after other work has several sittings. Work no session
was named for, and hours recorded before sittings existed, are known only by
hour ("By hour").
_Avoid_: segment, block (blocks are coding-agent blocks), session (a session
is one app's stretch in the timeline), measured duration.

**Day summary**:
The day's headline, narrative, insights, and **standup** written by the
activity provider in the interface language, generated once a day at the
configured time (or right after waking) and on demand. Its panels' numbers are
read from the data each time it is shown, never from the model.
_Avoid_: daily report, journal.

**Standup**:
The day summary's three lists (done, in progress, blockers), copied as bullet
lists in one click.
_Avoid_: status update.

**Quiet hours**:
The local period in which activity notifications wait instead of showing; an
end at or before the start spans midnight.
_Avoid_: do not disturb (that is the macOS Focus feature).

## Flagged ambiguities

- **"proxy"** must name a concrete component. Direct provider calls do not
  pass through a Clovy inference service.
- **"transcribe"** is overloaded between **dictation** (short, latency-
  critical) and **note transcription** (long, batch). Always qualify which.
- **"credits"** is historical billing vocabulary, not an app balance or
  current price unit. Provider catalog prices use dollars and explicit units.
- **"the session id"** is ambiguous — say **stored** (persistent, UI-facing) or
  **runtime** (live process) session id.
- **"the model"** never means the chat engine. A CLI owns its chat model;
  Clovy's text-model selection controls remote note generation independently.
- **"channel"** is overloaded: a **Source** lane (mic/system) or a WAV
  interleave channel. Qualify.

## Example dialogue

> **Dev:** "Does picking a text model switch chat away from Claude Code?"
>
> **PM:** "No. It changes the remote model used when Notes selects OpenRouter.
> The picker names the Notes engine, and chat keeps its CLI engine."
