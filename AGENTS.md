<!-- SPECKIT START -->

For additional context about technologies to be used, project structure,
shell commands, and other important information, read
`specs/003-conversation-turns/plan.md`.

<!-- SPECKIT END -->

# Clovy — Agent Instructions

## Project

Clovy is a local-first **Tauri desktop app** for meeting notes: it records a
meeting or dictation, transcribes audio, turns transcripts into structured
notes, and hosts CLI chat over the user's notes. The frontend is **React**
(`src/`), and the native shell is **Rust** (`src-tauri/`). Chat uses installed
CLI engines (Claude Code, Codex and Antigravity), with Claude Code as the initial
default when available. The app owns canonical history and the common tool
gateway; execution failure never silently switches engines. The desktop calls
**Groq**, **OpenRouter**, and **fal** directly with
the user's own keys; transcription may instead use the user-installed local
Parakeet engine (macOS Apple Silicon only, ADR-0074). CLI and endpoint note routes remain separate. There is
no Clovy inference server, native agent sidecar, local bearer token, sign-in,
sharing service, or credit balance. Clovy tools remain available through MCP.

> Read **[CONTEXT.md](CONTEXT.md)** before naming anything, and
> **[docs/index.md](docs/index.md)** to find the doc for the area you touch.

## Structure

```
os-clovy/
├── src/                     # React frontend
│   ├── app/                 # app shell, routing, onboarding, bootstrap
│   ├── components/          # agent (chat), settings, onboarding, note-editor, recorder, sidebar, ...
│   ├── lib/                 # agent runtime contracts, model privacy, Tauri bindings, ...
│   ├── styles/              # app.css + tokens.css (design tokens)
│   └── test/                # vitest suites (all frontend tests live here)
├── src-tauri/               # Rust native shell (Cargo package `clovy`)
│   ├── src/audio/           # recording, source separation, turn detection, live preview
│   ├── src/agent_sessions/  # engine-neutral sessions, events, persistence, and migration
│   ├── src/providers/       # model settings, direct Groq/OpenRouter/fal clients and catalogs; `transcription.rs` resolves Groq vs Local, `local_transcriber.rs` drives the helper (docs/audio-pipeline.md)
│   ├── src/llm/             # own providers: agent CLIs (resolved from the login-shell PATH on macOS; the app's `Path` + `PATHEXT` + installer dirs on Windows, `shell_env.rs`), OpenAI-compatible endpoints, per-use selection (docs/llm-providers.md; tests: `cargo test llm`)
│   ├── src/activity/        # text-only capture: macOS AX/Vision, windows/ UIA/WGC/OCR; encrypted activity.sqlite3, exclusions, retention (docs/activity-capture.md, ADR-0057); timeline/ = ETL, gaps, categories, FTS search, agent tools (docs/activity-timeline.md); tests: `cargo test activity`
│   ├── src/coding_agents/   # read-only ingest of local coding-agent transcripts, blocks, tool-free summaries (docs/coding-agent-sessions.md; tests: `cargo test coding_agents`)
│   ├── src/day_intelligence/ # distillation + local embeddings, hourly reports, workstreams, day summary/standup, schedule, notifications (docs/day-intelligence.md; tests: `cargo test day_intelligence`)
│   ├── src/mcp_server/      # Clovy MCP server: `clovy-mcp` stdio relay, authenticated private IPC (macOS Unix socket; Windows named pipe), unbound read-only tools and host-bound tools (docs/mcp-server.md, ADR-0058/0059; tests: `cargo test mcp_server`)
│   ├── src/chat_engine/     # agent CLIs as a chat session's engine (Home included, ADR-0064): resume, stream → agent items, cancel, Clovy tools via MCP (docs/llm-providers.md; tests: `cargo test chat_engine`)
│   ├── src/commands.rs      # the Tauri command surface; every command invoked from `src/` must be in `lib.rs` `generate_handler!` (guard: `src/test/tauri-command-registry.test.ts`)
│   └── native/              # macOS system-audio helper (Swift) + dictation helper + dev launcher (`dev-launcher/`, ADR-0060) + `mac-local-transcriber/` (SwiftPM + FluidAudio, pinned model manifest in `resources/local-transcriber-model.json`, built by `scripts/prepare-local-transcriber.mjs`; plain `cargo` builds bundle only a placeholder, so local transcription needs `pnpm local-transcriber:prepare`)
├── docs/                    # see docs/index.md — ADRs, subsystem docs, runbooks, PRDs, QA
├── specs/                   # Spec Kit feature specs (001-003)
├── spec/                    # enforceable coding rules (see spec/index.md) — distinct from specs/
├── scripts/                 # build / dev / release tooling
├── CONTEXT.md               # domain glossary — canonical names
├── AGENTS.md                # this file (canonical); CLAUDE.md is a symlink to it
└── .agents/skills/          # vendored agent skills, symlinked into .claude/skills/
```

## Domain & decisions — read before writing code

- **[CONTEXT.md](CONTEXT.md)** — the domain glossary / ubiquitous language.
  Read before naming anything; terms are canonical and the `_Avoid_` lists are
  binding (dictation vs note transcription, Source vs channel, chat engine
  vs model, provider price vs observed charge, stored vs runtime session id).
- **[docs/index.md](docs/index.md)** — the annotated index of every doc: ADRs,
  subsystem docs, release/ops runbooks, PRDs, QA, and the feature specs.
- **[docs/adr/](docs/adr/)** — Architecture Decision Records. Read the ADRs for
  the area you are touching before proposing structural change; **do not
  re-litigate accepted decisions.** Append-only: supersede with a new ADR (or a
  dated addendum), never rewrite the decision. Numbering: scan `docs/adr/` for
  the highest `NNNN-*.md` and increment.
- **[specs/003-conversation-turns/plan.md](specs/003-conversation-turns/plan.md)**
  — the current feature spec; its plan doubles as the tech-stack and
  shell-command reference for new agents.

### When to add an ADR (proactive)

Record a decision as an ADR when **all three** hold:

1. **Hard to reverse** — real cost to change later (architectural shape, an
   integration/wire contract, tech lock-in, a boundary).
2. **Surprising without context** — a future reader will ask "why on earth is
   it done this way?".
3. **A real trade-off** — genuine alternatives existed and one was chosen for
   specific reasons.

Skip it if the change is easily reversible, the obvious choice, or had no real
alternative. Offer an ADR proactively (do not wait to be asked) when you reject
a refactor for a load-bearing reason, deviate deliberately from the obvious
path, or encode a constraint not visible in the code. If you sharpen or add a
domain term mid-discussion, update **CONTEXT.md** in the same change.

## Specs (enforceable rules)

Enforceable coding rules live in **[spec/index.md](spec/index.md)**, one file
per rule (Rule / Why / How to apply / Exceptions). **Read every spec in your
scope before writing code; violations should fail review.** When you add,
rename, or remove a spec, update `spec/index.md` in the same commit. (These are
distinct from the `specs/` Spec Kit feature specs.)

- [sentence-case](spec/sentence-case.md) — sentence case for all UI labels (never ALL CAPS / uppercase)
- [no-typographic-dashes](spec/no-typographic-dashes.md) — no en/em dashes in user-facing copy (hyphen or "to")
- [no-all-caps](spec/no-all-caps.md) — no ALL CAPS in UI, no `text-transform: uppercase`
- [icons-central-only](spec/icons-central-only.md) — icons from `central-icons` / `central-icons-filled` only (never lucide)
- [design-tokens](spec/design-tokens.md) — use the variables in `src/styles/tokens.css`
- [no-tabular-numerals](spec/no-tabular-numerals.md) — UI numbers use proportional figures, never `tabular-nums`
- [type-scale](spec/type-scale.md) — font sizes only from `--fs-*`; headings follow the mapping table
- [font-weights](spec/font-weights.md) — only 400 and `var(--fw-medium)`, never raw 500/600/700
- [font-families](spec/font-families.md) — sans is the voice; serif for headings/display, mono for code
- [control-sizes](spec/control-sizes.md) — control heights from `--control-*`, no raw min/max-heights
- [scroll-fade](spec/scroll-fade.md) — clipped scrollers use the shared `useScrollFade` + `.scroll-fade` / `.scroll-fade-mask` primitive
- [package-install-security](spec/package-install-security.md) — pnpm-only; new package installs go through `sfw`; 7-day `minimumReleaseAge` cooldown
- [mcp-tool-naming](spec/mcp-tool-naming.md) — Clovy-owned in-loop host tools are `verb_object`; the owning PRD or contract names them before the code is written

## PR and description conventions

When drafting PR titles, PR descriptions, issue summaries, release notes, or
other project copy, avoid naming or comparing against other products unless the
user explicitly asks for that context or the reference is required for a
concrete integration, compatibility note, migration, or legal attribution.
Prefer describing the behavior, workflow, or category generically.

Every PR description should state (the template in
`.github/pull_request_template.md` has these sections):

- whether the change was **tested visually** — for UI changes, attach a
  screenshot or recording;
- whether it changes a provider contract or requires user configuration;
- the **root cause**, for bug fixes (the actual cause, not just the symptom);
- what is deliberately **out of scope**;
- any **followups** it sets up or defers (link issues where possible).

## Skills

Vendored agent skills live in **`.agents/skills/`** (the single source of truth)
and **every skill is symlinked into `.claude/skills/`**. A skill must never exist
only under `.claude/`, and a `.claude/skills/<name>` entry must always be a
symlink to `../../.agents/skills/<name>` — never a real directory. Add a new
skill under `.agents/skills/<name>/` and create the `.claude/skills/<name>`
symlink in the same change. Current project skills: `os-design`,
`repo-build-pr`, `repo-review`, `repo-delegate`, `repo-orchestrate`,
`repo-retrospect`, `browser-test-tauri-fe`, `agent-e2e-qa`, plus the Spec Kit
workflow skills (`speckit-*`). Skills are vendored files; there is no remote
skills lockfile.

## Agent skills

### Issue tracker

Work is tracked as harness tickets (`~/orchestrator`), not GitHub Issues. See
`docs/agents/issue-tracker.md`.

### Triage labels

Triage roles map onto harness ticket states. See
`docs/agents/triage-labels.md`.

### Domain docs

Single-context: one root `CONTEXT.md` (canonical glossary, binding _Avoid_
lists) + `docs/adr/`. See `docs/agents/domain.md`.

### Collaboration (build, delegate, review)

`repo-build-pr` is the entry-point skill; implementation can be delegated
across harnesses (`with codex`), reviews always run on a harness that did not
write the diff, and trust levels are explicit (OS sandbox vs policy-level).
See `docs/agents/collaboration.md` for the map.

## Build, test, lint

Package manager: `pnpm`, the only package manager for this repo, pinned by the
`packageManager` field in `package.json` (CI's `pnpm/action-setup` reads the
same pin — bump it in one place, to the newest release at least 7 days old).
Supply-chain rules — the `sfw` install
wrapper, the 7-day `minimumReleaseAge` cooldown, and deny-by-default dependency
build scripts in `pnpm-workspace.yaml` — live in
[spec/package-install-security.md](spec/package-install-security.md).

- **Run the app:** `pnpm tauri:dev` (builds `src-tauri` and launches the native
  app; the first build is slow). `pnpm dev` runs the Vite frontend only.
- **Frontend tests:** `pnpm test` (vitest; all suites live in `src/test/**`).
  The runner can exit non-zero from `hud-meeting.test.ts` teardown noise despite
  0 real failures — judge by the failure count. Composer/ProseMirror tests can
  flake with a `localsInner` crash under machine load (a `@tiptap/pm` duplicate,
  not a regression). On **Node 26** the `font-scale`
  storage test fails locally (experimental web storage shadows jsdom's
  `localStorage`); run `NODE_OPTIONS=--no-experimental-webstorage pnpm test`
  and do not "fix" the tests.
- **Rust tests:** `pnpm test:rust` (src-tauri).
- **Windows native activity tests:** clipboard fixtures must run in a child
  process with a private window station/desktop, never the console clipboard.
  The installed-package icon test is ignored by default and requires Calculator
  and Notepad: `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib activity::app_icon::windows_tests:: -- --include-ignored`.
- **Windows desktop build:** `scripts/build-windows.ps1`; needs Visual Studio
  C++ Build Tools, Strawberry Perl and NASM on `PATH` (vendored OpenSSL/SQLCipher).
- **Windows-only Rust tests:** `activity::windows`, the Windows `llm` cases
  (`LoginEnv`, `cmd_shim_tests.rs`) and `chat_engine/windows_tests.rs` compile
  only on Windows, so `make verify` on the Mac does not run them. Run
  `cargo test --manifest-path src-tauri/Cargo.toml --locked <activity|llm|chat_engine>`
  on a Windows host; CI's `windows-rust` job (manual dispatch) runs the same three.
- **Windows MCP:** also run `cargo test --manifest-path src-tauri/Cargo.toml --locked mcp_server`
  on Windows. Unix transport tests and GNU metadata checks do not qualify named-pipe
  runtime, other-SID denial, Authenticode or the installed NSIS artifact.
- **Chat and tools gate:** `cargo test chat_engine`, `cargo test mcp_server`,
  and `cargo test approval_broker` cover CLI turns and local tool authority.
- **Agent item ordering:** `event.sequence` is a per-run replay cursor, not a
  timeline ordinal. Item-producing events carry `data.itemSequence` from the
  persisted conversation-wide item. Keep cursor gates and event-ID deduplication
  separate; never substitute a cursor or timestamp for item ordering.
- **Lint / format:** `pnpm check` (Biome: format + lint for `src/` and
  `scripts/`, including the lucide import ban) and `pnpm typecheck`
  (`tsc --noEmit`); `pnpm format` / `pnpm check:write` apply Biome fixes. Rust
  uses `cargo fmt` / `cargo clippy` (config lives under `src-tauri/`). Biome ratchets high-volume retrofit rules (a11y, hook-deps,
  non-null assertions) to `warn` in `biome.json`; keep new code clean and fix
  the warnings incrementally. Never leave checks broken.
- **CI parity:** `make verify` runs the full gate locally (Biome, typecheck,
  vitest, and `cargo fmt`/`clippy`/`test` for the desktop Rust crate); `make help`
  lists every target. A green `make verify` should mean green CI. It adds
  `cargo clippy --all-targets`, which the narrower targets skip — "green" from
  `cargo test` + `pnpm test` alone is **not** CI-green.
- **A red gate that is not your bug:** see [TROUBLESHOOTING.md](TROUBLESHOOTING.md)
  before chasing one — Node 26 storage tests, vitest teardown exit codes,
  ProseMirror flake, `cargo test --lib` missing the integration suites, pnpm's
  non-TTY purge prompt (`CI=true`), and 1Password-locked signing/push failures
  are all documented false alarms.

## Boundaries

- **Clovy has its own identity.** The app is `dev.guilhermexp.clovy`
  (`Clovy.app`, executable `clovy`); Rust spells the identifier only in
  `src-tauri/src/app_identity.rs`, and credential services derive from the
  compiled identifier. The June-era names survive only as sources of the
  one-time `identity_migration` (macOS only; Windows always starts with an
  empty profile) and of one-way reads; never write a June-era
  name again ([ADR-0068](docs/adr/0068-clovy-takes-its-own-identity-with-a-one-time-migration.md),
  superseding ADR-0055).
- **The app never calls a Clovy inference server.** Groq transcription,
  OpenRouter text, and fal media use the user's own provider key directly.
  Transcription has one user-chosen engine for dictation, notes and live
  preview: Groq, or the Local Parakeet model installed from Settings; with
  Local chosen no audio leaves the machine and a missing model is an error,
  never a switch to Groq. Missing or unreadable keys fail clearly before a
  request; never substitute a service key, another provider, a local bearer
  token, or a retired route.
  Keys remain in the credential store and go only to their own provider;
  media downloads carry none. Chat stays on CLI engines. Notes may use a CLI,
  an explicitly configured endpoint, or OpenRouter. Keep `llm::generate`
  and the CLI/endpoint note routes intact. See
  [ADR-0073](docs/adr/0073-desktop-calls-user-providers-without-a-clovy-server.md).
- **Clovy presents as Clovy.** CLI execution is an implementation detail;
  product instructions assert Clovy's identity.
- **macOS automatic dictation destination:** fix the exact app and available
  AX window/field when the helper accepts the take's start, before microphone
  permission and recorder reset. Carry the take-owned snapshot through that
  reset; stop must not repin, and indicator/cleanup metadata must use the pin.
  Keep updating the idle candidate for the next take. Restore the original
  window and try to focus its field before automatic Command+V; unavailable
  window or expired activation means clipboard recovery, never another window.
  Preserve the existing deadlines and 700 ms clipboard restoration. See the
  macOS start-time addendum in [ADR-0014](docs/adr/0014-pinned-dictation-paste-target.md).
- **macOS dictation reinsertion:** Control+V inserts the last eligible dictation
  into the current focus using Unicode events, independently of the clipboard.
  Rust owns one volatile, data-partition-qualified slot, updated only after
  delivery authorization; cancelled, empty, quarantined and stale results cannot
  replace it. Internal helper replies are correlated to a native key request;
  the renderer cannot supply reinsertion text. Preserve Command+V, automatic
  paste/700 ms restoration, Carbon capture suspension and recording-shortcut
  priority. No Input Monitoring or clipboard fallback. See [docs/dictation.md](docs/dictation.md).
- **Tool terminality is evidence-based.** `agent_sessions::terminal` atomically
  settles unresolved running calls as `interrupted` in the exact session/run,
  including when Stop already won. This means no final result was observed,
  not that an effect failed or rolled back. Preserve real results and keep
  outer CLI call IDs separate from approval-broker IDs (docs/llm-providers.md).
- **Dock visibility is independent of autostart.** On macOS, opt-in hiding
  uses Accessory only while the main window is hidden; showing it restores
  Regular and focus. Closing a window is not quitting the app. Keep tray
  reopening and the preference-off behavior intact
  (`openspec/specs/dock-presence/spec.md`).
- **No Open Software services in the desktop (fork).** The app has no OS
  Accounts sign-in, billing, credits funding, telemetry, issue reports,
  companion, or June updater; it needs no account identity or server token. `src/`,
  `src-tauri/src/` and `src-tauri/*.json` (outside `src/test/`) must not name
  `opensoftware.co`, `june.link` or `open-software-network`
  (`make org-references-check`, part of `make verify`). The same full gate
  rejects retired inference-server identifiers and routes; direct provider
  URLs remain allowed.

## Fork guilhermexp (harness ~/orchestrator)

This checkout is Guilherme's personal fork (`origin` = guilhermexp/os-clovy,
the only remote). Issues and branches use the harness tickets, not `JUN-*`.

- **OpenSpec** (`openspec/`, pt-BR) holds what to build for fork features;
  `specs/` stays the upstream Spec Kit history. Scenarios carry `Test:` stamps.
- **Data safety:** never run against the real install data
  (`~/Library/Application Support/dev.guilhermexp.clovy`, and the June-era
  `co.opensoftware.june` it migrates from). Debug builds use the isolated
  `dev.guilhermexp.clovy-dev` dirs (a default-identity dev build migrates the
  owner's `co.opensoftware.june-dev` into it on first launch, so never launch
  one before the owner does); never set
  `OS_CLOVY_USE_PROD_DATA_DIR` in tests or verification. Keychain items created
  by dev builds must use a `-dev` service suffix.
  Persisted Rust paths must use `app_paths::app_data_dir` or
  `app_paths::app_config_dir`, not the raw Tauri resolver. A signed Dev bundle
  alone does not isolate `app.path().app_config_dir()` or the frontend's
  `appConfigDir()`; those can still resolve the release directory.
- **Dev permissions:** `pnpm tauri:dev` runs `src-tauri/target/<profile>/Clovy Dev.app`
  (bundle id `<identifier>.dev`), signed with the single Apple Development
  identity or `CLOVY_DEV_SIGNING_IDENTITY`, and never resets TCC grants
  ([ADR-0060](docs/adr/0060-development-app-runs-as-a-signed-bundle-that-owns-its-grants.md)).
  `CLOVY_DEV_APP_IDENTIFIER=<id>` gives a run its own compiled identity, data
  directory (`<id>-dev`), and bundle. Never `tccutil reset` a release
  identifier.
  Computer use debug peer authentication trusts real Cargo outputs under this
  checkout's `src-tauri/target/<profile>` (optional target triple), with the
  signed dev bundle directly in that profile directory. A packaged debug
  app under `debug/bundle/macos` or a custom target root is not that layout;
  do not weaken the path, audit-token or same-Team checks to qualify it.
  Dynamic `SecCode` validation uses `STRICT_VALIDATE | NO_NETWORK_ACCESS`;
  `DO_NOT_VALIDATE_RESOURCES` is a static-code flag and makes this dynamic
  call fail with `errSecCSInvalidFlags` (-67070), before a permission query.
  The private `tools/list` schema for `get_window_state.capture_mode` must
  expose `som`, `vision`, and `ax`, matching the adapter's accepted modes.
  Keep its closed schema and the signed-helper self-test contract intact.
- **Managed Antigravity context:** `llm/agy.rs` owns the durable
  `<app_data>/runtime-profiles/agy/home` and private execution workspaces.
  HOME/XDG and Windows profile aliases change only in the CLI child; personal
  transcript ingestion and the relay's host identity stay unchanged.
  Chat, Test, note/activity calls and `llm_open_agy_login` use that same context.
  Native authentication is retained, not copied or removed with a turn.
  Workspace discovery is bounded by an app-owned empty Git root created through
  filesystem primitives, not a Git runtime dependency. Legacy/foreign native IDs
  replay canonical history rather than resuming another profile
  (`docs/llm-providers.md`).
  Agy qualifies MCP names as `mcp_<server>_<tool>` under a 64-character limit;
  keep the per-turn server nonce compact without discarding UUID bits.
  New receipts use v3; v1/v2 remain strict owned-cleanup inputs only.
  Only the exact active server normalizes as Clovy, never an older namespace.
  After MCP install, supply that namespace as transient Agy prompt context;
  never persist it as canonical input or treat it as permission.
- **Native MCP wire names:** `mcp_server/wire_names.rs` is passive per-grant
  metadata shared by private Claude/Codex/Agy bindings and stream translators.
  Keep full canonical names in catalog, dispatch, policies and history; encode
  only at the protocol boundary and resolve once after the managed-origin guard.
  Unknown/unpublished aliases and raw names needing encoding are refused.
  Public/unbound MCP and other CLI kinds retain their existing behavior.
- **One runtime-mode control:** each run (including retries) derives Clovy tool
  approval from the saved safety mode: `sandboxed` requires approval;
  `unrestricted` bypasses it only when attended. The legacy session
  `approval_mode` column has no authority and no setter command. Home stays
  sandboxed; the selector stays on new focused conversations. Clarifications
  still require a person, and unattended Clovy write requests remain refused.
  Only Codex + `unrestricted` uses `danger-full-access`, including routines
  explicitly configured with that mode. Other approval-skipping flags remain
  forbidden. Activity selection of an installed one-shot CLI without a measured
  structured-output level runs the provider test before applying the selection;
  authentication remains an explicit user action (docs/llm-providers.md).
- **CLI approval ordering:** the event envelope's `sequence` is run-local;
  `data.itemSequence` is the persisted session-wide item position. Keep the
  same item position when settling a card; hydration must not repair live
  ordering (`docs/llm-providers.md`).
- **Meeting video:** the default-off macOS capture uses a separate
  `Clovy Video Capture.app`; capture or analysis failure must preserve the
  audio/transcript path. Only Claude note generation receives the scoped video
  tools grant; other CLI invocations retain their existing isolation. See
  ADR-0071. Real ScreenCaptureKit/Meet capture and paid visual acceptance remain
  explicitly pending in `gravacao-video-reuniao/tasks.md`; do not infer them
  from portable tests or compilation.

## Verification

Gates (run from the repo root, judge by failure count per the notes above):

- Frontend: `pnpm check`, `pnpm typecheck`, `NODE_OPTIONS=--no-experimental-webstorage pnpm test`.
- Desktop Rust: `cargo fmt --manifest-path src-tauri/Cargo.toml --all --check`,
  `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --locked -- -D warnings`,
  `pnpm test:rust`.
- Full CI parity: `make verify`.

Functional proof (Reality) for desktop features:

1. Isolation: debug builds only. The owner's `dev.guilhermexp.clovy-dev` profile
   is not a throwaway fixture. For automation compile a separate identifier:
   `pnpm tauri:build -- --debug --bundles app --config src-tauri/tauri.macos.conf.json --config '{"identifier":"dev.clovy.timelineproof"}'`
   (Node 24 PATH below). Choose a run-owned identifier/profile and create a fresh synthetic
   profile before launch. Only use a copied profile when that scenario is
   explicitly authorized; never copy its live MCP socket. This example uses
   `~/Library/Application Support/dev.clovy.timelineproof-dev`.
   Keep the full UTF-8 path to `<profile>/mcp/clovy.sock` below the macOS
   `sockaddr_un.sun_path` limit (104 bytes including the trailing NUL);
   choose a short proof identifier, not a socket-path or authentication bypass.
   A proof identifier never migrates the June-era profile; to prove the
   migration, set the debug-only `CLOVY_DEV_IDENTITY_MIGRATION_SOURCE=<id>` to
   a synthetic source profile `<id>-dev` (and `<id>-dev.<name>` credential
   services) the run created; `CLOVY_DEV_IDENTITY_MIGRATION_SOURCE_APP=<name>`
   names that profile's "old app" executable, which blocks the copy while it
   runs.
   Keep Apple Development signatures on the host and helpers when proving
   private helper authentication; an ad hoc copy can prove UI only, not
   same-Team helper readiness. Open by exact bundle path and verify the
   process's database paths with `lsof` before changing settings or reports.
   Changing only `Info.plist` does not isolate Tauri's compiled data identity.
   Keep the normal HOME: a fake HOME hides the default Keychain.
   Bind Computer Use to that exact bundle path. If the main window is hidden,
   `open -a <exact-bundle-path>` reopens it. macOS permissions belong to the
   proof app itself; the owner grants them in Privacy & Security when needed.
   Never open or look up bundle id `dev.guilhermexp.clovy` or the June-era
   `co.opensoftware.june`; never launch an old
   proof binary against a profile with a newer database schema. Stop only
   processes started by this run.
   Startup may create the shared `~/Library/LaunchAgents/Clovy.plist` even for
   a proof bundle. Preserve the whole existing unit (or its absence) and
   inspect both `ProgramArguments` and the loaded launchd service's program.
   Use Settings → General to undo autostart only when both belong to this
   run. An owned plist can coexist with another session's loaded service:
   never toggle off or bootout that service; restore only this run's created
   file to the preserved prior state. Never remove another app's login entry.
   Never create, unlock, or switch keychains, and never change the default
   keychain or its search list: the app reads its keys from `login`
   (`<identifier>-dev.<name>` services). Only one native proof runs on the machine at a time; parallel
   workers do not open the app. A macOS password dialog on the owner's screen
   is an incident: stop this run's processes and report it.
2. Readiness: wait for the main window; macOS permissions the feature needs
   (Accessibility, Screen Recording, Microphone, Notifications) must already be
   granted, otherwise the proof is blocked, not faked. A newly signed app
   identity may ask for the protected folder holding the checkout (Files and
   Folders > Documents for this repository), possibly before its window
   appears, and may ask the owner to authorize reading an existing `-dev`
   Keychain item. Confirm what macOS actually shows; both are the owner's
   local decisions, never unlocked or changed by the run, and folder access
   alone does not make the app ready.
3. Action: drive the real UI as the user would (computer use / clicks), or the
   Tauri command the UI calls when the UI path is the thing under test.
4. Observable: screenshot of the window showing the result, plus persisted
   state read back (SQLite in the `-dev` data dir; encrypted DBs through the
   app's own debug export command, never by copying keys into the transcript).
5. Evidence under `.tmp/verify/<ticket>/run-<id>/` with a `report.md`;
   cleanup leaves the `-dev` data intact unless the run created a throwaway copy.
6. Windows: launch the dev app (`pnpm tauri:dev` with a run-owned
   `CLOVY_DEV_APP_IDENTIFIER`) in the logged-in console session through an
   interactive scheduled task. With the console locked, screen capture shows
   only the lock screen: drive and capture the WebView over CDP
   (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=<port>`,
   reached through an SSH tunnel). Use one `CARGO_TARGET_DIR` and stop the app
   before `cargo test`: the running executable locks the build.

UI-only checks may use the browser shim from `.agents/skills/browser-test-tauri-fe`;
that proves rendering, not the native flow.

For **dev-permission persistence**, use the signed dev launcher from
[development.md](docs/development.md) and ADR-0060 with a run-owned
`CLOVY_DEV_APP_IDENTIFIER` and the authorized stable signing identity. The
ad-hoc debug snapshot above is not proof that grants survive the normal
launcher, restart or rebuild. Observe the actual window and permission state,
then restart/rebuild that same identity; do not reset TCC or grant privileges
by editing OS databases.

For the **provider cutover**, use a fresh isolated profile under a run-owned
identifier and synthetic data. Exercise dictation with and without a Groq key,
notes through Claude Code and OpenRouter, the text catalog, and model selection
that leaves chat on Claude Code. A fake listener on `127.0.0.1:8080`, bound only
when free, must receive zero requests. Automated tests may inject an ephemeral
retired origin. Check migration without changing endpoints/history; never copy
or launch the owner's production profile. Only one native proof runs at once.
Media proof includes valid final assets, oriented/non-square/extreme-ratio
edits, and resuming a recoverable video job without resubmission. Paid media,
OpenRouter inference, and Codex turns require explicit owner authorization;
anything not run remains unproven. Public catalog reads and local contract
tests do not prove live inference or an account's price.

### Build

**Publish a release:** on `main`, run `pnpm version:bump X.Y.Z`, commit
`release: vX.Y.Z`, then push `main` and tag `vX.Y.Z`.
`.github/workflows/release.yml` publishes the GitHub assets and `latest.json`
for the installed app's updater. A local build is not a publication. Do not
produce or install a local bundle with the production identity unless explicitly
requested. Local proof uses an isolated debug identity; choose a fresh id for
each run, replacing both occurrences in the example below.
For an owner-approved macOS-only publication, preserve the full repository
variable `CLOVY_RELEASE_PLATFORMS`, select `macos` before pushing the tag, then
restore its previous value or absence after the run. The default `all` still
requires a valid Windows signing certificate. See `docs/release-macos.md`.

```json
{
  "command": {
    "argv": ["sh", "-c", "PATH=\"$HOME/.local/share/node-official/node24/bin:$PATH\" CLOVY_DEV_APP_IDENTIFIER=dev.clovy.buildproof CARGO_PROFILE_DEV_BUILD_OVERRIDE_STRIP=none CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none pnpm tauri:build -- --debug --bundles app --config src-tauri/tauri.macos.conf.json --config '{\"identifier\":\"dev.clovy.buildproof\"}'"],
    "cwd": "."
  }
}
```

Node builds the frontend; the app does not bundle a JavaScript agent executable.
This machine keeps Node 24 at `~/.local/share/node-official/node24`.
With Xcode 27, release builds under `MACOSX_DEPLOYMENT_TARGET=14.0` corrupt
proc-macro dylibs when Cargo strips build dependencies ("mis-aligned LINKEDIT
string pool", then `can't find crate for zerofrom_derive`);
`CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none` avoids it without touching the
shipped binary; debug proof builds need `CARGO_PROFILE_DEV_BUILD_OVERRIDE_STRIP=none`
(otherwise `can't find crate for bytemuck_derive`) and the release override too,
because the computer use driver always compiles in release. Updater artifacts
stay off locally (`bundle.createUpdaterArtifacts` is unset), so local builds
need no updater key; only the release workflow adds
`src-tauri/tauri.release.conf.json` with the owner's key
([docs/release-macos.md](docs/release-macos.md), ADR-0069).

<!-- repowise:start -->
## Repowise — diagnóstico de código

Navegação (onde fica, quem chama, assinaturas) continua no Graft. O Repowise responde diagnóstico, pelo MCP `repowise` ou pelo adapter do harness com a raiz absoluta do checkout (`<raiz>`):

- Revisar ou aceitar mudança: `get_change_risk` com revspec `base..head` (saúde introduzida, testes sugeridos).
- Antes de mexer em arquivo crítico: `get_risk` (correções recentes, churn e arquivos que costumam mudar junto; confira se precisam acompanhar).
- Saúde e refatoração: `get_health`. Código morto: `get_dead_code`; só apague achado de nível seguro e confirmado por busca.
- Por que o código mudou: `get_why` (commits). Decisões de produto e do harness não moram aqui.
- DOX desatualizado: `/Users/guilhermevarela/orchestrator/scripts/project-repowise.sh query doc-drift <raiz>`.
- Índice atrasado (`_meta.index_behind`) ou ausente: `/Users/guilhermevarela/orchestrator/scripts/project-repowise.sh ensure <raiz>`. Worktree não tem índice: `/Users/guilhermevarela/orchestrator/scripts/project-repowise.sh query risk <checkout-principal> <base>..<branch>`. Não rode `repowise` direto; sem o adapter ele registra MCP e hooks globais.
<!-- repowise:end -->

<!-- graft:start -->
## Graft — repo context graph

This repo is indexed in `graft/`: small linked markdown nodes that explain each
system and carry exact file:line spans, kept in sync with the code through git.

For ANY task here — understanding how something works, finding where code lives,
or scoping a change — get context from the graph before grepping or opening
source files. Re-ask freely (it's cheap) and reuse literal identifiers you
already have (symbol, error string, file name) as the query. New to this repo?
Run `graft map` first — a token-budgeted orientation (dir clusters, hubs,
hotspots), no LLM, no key.

- Run `graft ask "<your question>" --source` → ranked nodes with the relevant
  code spans inlined (each hit's ≤8-line crux by default; `--full` for whole
  definitions when the crux isn't enough). Match the tool to the task shape:
  for understanding or editing, the top node IS the answer — cite its
  `covers:` file:line spans and edit straight from `--source`. For
  exhaustive tasks ("every occurrence / every caller of this pattern"), ranked
  results are top-N, not complete — run `graft grep "<literal>"` instead
  (exhaustive over indexed files, grouped by enclosing symbol), falling back
  to raw `grep -rn` only for unindexed files.
- `graft skeleton <file>` → every definition's signature + span, ~10× cheaper
  than reading the file; use it to skim an API surface.
- `graft callers <symbol>` gives precomputed, exact edges — who calls this.
  Add `--direction out` for what it calls, or `--depth N` to walk
  transitively for the full blast radius. For structural questions, skip
  ranking and use this directly.
- Or browse: `graft/INDEX.md` lists every node; follow the links.
- Monorepos and folders of multiple repos rank fairly across sub-projects —
  hits carry `[scope/]` labels naming which one they're from. Narrow with
  `graft ask "<task>" --in <scope>/` once you know where you're working.

If a returned span is truncated ("+N more lines"), open the file at that exact
range before finalizing. Only open source files when a node genuinely lacks a
needed detail, and then at the exact file:line the node points to — never
re-read whole files.

After big code changes, refresh the graph with `graft build` (deterministic,
no API key, $0).
<!-- graft:end -->

## Contexto por área

- [src-tauri/src/activity/windows/AGENTS.md](src-tauri/src/activity/windows/AGENTS.md): Windows capture adapter.
