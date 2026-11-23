# Clovy docs index

Read first, in order: **[CONTEXT.md](../CONTEXT.md)** (domain glossary) →
**[AGENTS.md](../AGENTS.md)** (agent guide) →
**[specs/003-conversation-turns/plan.md](../specs/003-conversation-turns/plan.md)**
(tech stack + structure).

## Architecture & decisions (ADRs)

Append-only; supersede with a new number or a dated addendum, never rewrite the
decision. See "When to add an ADR" in [AGENTS.md](../AGENTS.md).

- [adr/0001](adr/0001-auto-updates-via-tauri-updater.md) — auto-updates via the Tauri updater on a separate public releases repo (amended by ADR-0069: the fork publishes its own releases)
- [adr/0002](adr/0002-live-transcript-preview-strategy.md) — live transcript preview as an ephemeral companion, not the source of truth
- [adr/0003](adr/0003-release-candidate-channel-and-promotion.md) — rc channel + promote-to-stable (every stable release starts as an RC) (superseded by desvincular-repo-ci; the in-app channel choice was removed by desvincular-servicos-open-software)
- [adr/0004](adr/0004-out-of-process-system-audio-helper.md) — macOS system audio via an out-of-process helper (file IPC + Unix signals)
- [adr/0005](adr/0005-source-separated-audio-capture.md) — one WAV per source, re-interleaved as turns
- [adr/0006](adr/0006-embed-hermes-sandboxed-runtime.md) — embed the pinned Hermes runtime as sandboxed child processes (superseded by ADR-0038)
- [adr/0007](adr/0007-model-capability-source-of-truth.md) — capabilities come from the provider catalog, not marketing traits; the 2026-10-08 addendum replaces Venice with OpenRouter
- [adr/0008](adr/0008-image-generation-and-editing-tools.md) — image generation/editing: `/image` fast path + LLM tools; the 2026-10-08 addendum replaces the historical Venice adapter with fal
- [adr/0009](adr/0009-hermes-config-shared-ownership-merge.md) — config.yaml is shared with the Hermes dashboard; Clovy deep-merges on spawn, never overwrites (superseded by ADR-0038)
- [adr/0010](adr/0010-note-references-in-agent-chat.md) — note references in agent chat: `@note:<id>` text token + `get_meeting_note` fetch-by-id
- [adr/0011](adr/0011-bundled-hermes-skills.md) — selected Hermes skills ship as read-only app resources when the runtime pin cannot move (superseded by ADR-0038)
- [adr/0012](adr/0012-direct-issue-report-submission.md) — issue reports submit directly (no client model turn, nothing to charge); the former server generated the team-facing diagnosis (superseded in this fork: the desktop no longer sends issue reports)
- [adr/0013](adr/0013-stream-inference-responses-through-june-api.md) — historical server streaming and settlement (superseded by ADR-0073)
- [adr/0013](adr/0013-agent-recorder-tool-via-frontend-event.md) — agent recording control: the recorder tool asks the main webview to start or stop through a frontend event, so capture always runs the visible UI path (shares the 0013 number with the streaming ADR)
- [adr/0014](adr/0014-pinned-dictation-paste-target.md) — the dictation paste target is pinned when the recording stops, never re-resolved at paste time
- [adr/0015](adr/0015-video-generation-tools.md) — `/video` fast path + LLM tools, async job + poll; the 2026-10-08 addendum replaces Venice quotes with endpoint-specific fal estimates
- [adr/0016](adr/0016-private-connectors-local-mode.md) — private connectors (local mode): Keychain-only token custody, app-proxied MCP calls straight to Google, trust modes enforced in the Rust proxy, earned autonomy, event-trigger daemon
- [adr/0017](adr/0017-browser-use-via-june-extension.md) — browser use in the user's own browser via the Clovy extension, two tracks behind one broker; computer use productizes the pinned toolset (MCP-server shape superseded by ADR-0040)
- [adr/0018](adr/0018-session-model-changes-apply-at-agent-run-boundaries.md) — session model changes are staged at Send and applied only at the next idle agent-run boundary
- [adr/0019](adr/0019-windows-dictation-helper.md) — Windows dictation uses a platform-native helper process
- [adr/0020](adr/0020-windows-dictation-keyboard-hook.md) — Windows shortcuts combine `RegisterHotKey` with a narrow keyboard hook
- [adr/0021](adr/0021-june-api-v1-compatibility-policy.md) — `/v1` is additive-only for shipped app versions: per-version contract fixtures gate CI and production promotes, clients send `x-june-app-version`, error codes never renumber
- [adr/0022](adr/0022-venice-private-first-model-routing.md) — historical Venice/Phala routing and privacy decision; the 2026-10-08 addendum replaces it with OpenRouter without inherited TEE or zero-retention claims
- [adr/0023](adr/0023-attested-os-api-service-chain.md) - superseded by ADR-0024
- [adr/0024](adr/0024-independent-product-verification.md) - Clovy, Open Software API, and Chat publish independent verification evidence without cross-product release pinning (superseded by desvincular-repo-ci)
- [adr/0025](adr/0025-targeted-hermes-approval-protocol.md) - MCP approvals use stable request identity, targeted resolution, bounded queues, and fail-closed retirement (superseded by ADR-0038)
- [adr/0026](adr/0026-durable-note-transcription-jobs.md) - saved-audio Source spans use durable, fingerprinted, idempotent note-transcription jobs
- [adr/0027](adr/0027-june-owned-project-memory-store.md) — memory entries live in Clovy's SQLite (not the Hermes memory toolset), scoped by project, agent writes via the loopback proxy, project context by prompt injection
- [adr/0028](adr/0028-private-stdio-broker-for-computer-use.md) - Computer use runs through a Clovy-owned private stdio driver broker with signed-helper TCC identity, task-scoped app authorization, and exact-window Stage Manager restoration (MCP-server shape superseded by ADR-0040)
- [adr/0029](adr/0029-dual-architecture-hermes-runtime.md) - the universal macOS app carries complete arm64 and x86_64 Hermes runtime trees and executes both before release (superseded by ADR-0038)
- [adr/0030](adr/0030-explicit-per-session-profile-targeting.md) — profile switching writes the sticky active profile AND threads it explicitly on session.create; no per-profile Hermes process
- [adr/0031](adr/0031-per-profile-data-isolation.md) — profiles isolate user data (notes/dictation/projects via a `profile` column, chat sessions via a `session_profiles` map); profile is the first data-partition key; delete prompts move-to-default vs delete
- [adr/0032](adr/0032-session-completion-june-owned-local-state.md) — marking a session complete is Clovy-owned local SQLite state keyed by the stored Hermes session id, orthogonal to Hermes' archive flag; mirrors the `session_folders` stack
- [adr/0033](adr/0033-notion-hosted-mcp-connect-preview.md) - Notion hosted MCP connector preview with read-only `june_notion`, approved page creation and updates, and no selected-resource claim
- [adr/0034](adr/0034-obsidian-vault-discovery-mcp.md) - Obsidian vault discovery uses a Clovy-owned MCP server, not an ambient environment variable (MCP-server shape superseded by ADR-0040)
- [adr/0035](adr/0035-extension-releases-follow-desktop-rc-promotion.md) - Chrome Web Store packages are reviewed during desktop RC and the exact staged bytes publish after stable desktop promotion (superseded by desvincular-repo-ci)
- [adr/0036](adr/0036-github-connector-app-user-tokens.md) - GitHub connector uses GitHub App user access tokens only (no app private key on device or backend), local-mode custody per ADR-0016, Clovy-side read/write gating, approval-only writes
- [adr/0037](adr/0037-versioned-local-sqlite-migrations.md) - Clovy's local SQLite schema uses an append-only release-ordered catalog, introspection-based legacy stamping, and one transaction for all pending migrations
- [adr/0038](adr/0038-june-owned-openai-agents-runtime.md) - historical bundled agent decision, superseded by ADR-0072; session persistence stays in Rust
- [adr/0039](adr/0039-june-owned-routines-and-mcp.md) - Clovy owns routine scheduling and user-configured MCP transport, persistence, safety, and migration; runtime integration superseded by ADR-0072
- [adr/0040](adr/0040-plugin-capabilities-as-host-tools.md) - historical host-tool-only integration, superseded by ADR-0072; risky parsers retain brokered isolation
- [adr/0041](adr/0041-june-companion-trust-boundaries.md) - original companion trust boundaries, relay-first E2EE, and desktop authority (superseded in this fork: the desktop companion was removed)
- [adr/0042](adr/0042-june-companion-native-swiftui.md) - native SwiftUI companion presentation; its mobile-login decision is superseded by ADRs 0043, 0046, and 0047 (superseded in this fork: the desktop companion was removed)
- [adr/0043](adr/0043-companion-desktop-authorized-device-credential.md) - desktop-authorized pairing and revocable device credentials; its authorization model is current again through ADR 0047 (superseded in this fork: the desktop companion was removed)
- [adr/0044](adr/0044-companion-device-generates-relay-credential.md) - companion-generated relay credentials; the former relay received and stored only their hashes (superseded in this fork: the desktop companion was removed)
- [adr/0045](adr/0045-companion-relay-single-replica-mvp.md) - the MVP relay is single-replica until pairing and live routing have shared cross-instance state (superseded in this fork: the desktop companion was removed)
- [adr/0046](adr/0046-companion-native-os-accounts-login.md) - superseded by ADR-0047 (superseded in this fork: the desktop companion was removed)
- [adr/0047](adr/0047-companion-pairing-is-mobile-authorization.md) - the authenticated Desktop-created QR pairing is the phone's short-lived authorization; the phone has no account session (superseded in this fork: the desktop companion was removed)
- [adr/0048](adr/0048-companion-mutations-use-durable-at-most-once-reservations.md) - companion mutations reserve their operation id durably before side effects and never redispatch an outcome-unknown request (superseded in this fork: the desktop companion was removed)
- [adr/0049](adr/0049-cancelled-dictation-takes-never-deliver.md) - each helper-owned dictation take has one cancellation token, and cancelled takes cannot start later cleanup, history, or text delivery
- [adr/0050](adr/0050-linear-official-hosted-mcp.md) - Linear agent capabilities come from Linear's official hosted MCP while Clovy owns connection state, credential custody, and conservative approvals
- [adr/0051](adr/0051-companion-attachments-use-bounded-staging-and-opaque-mac-references.md) - companion attachments use bounded app-data staging and opaque, user-granted Mac file references (superseded in this fork: the desktop companion was removed)
- [adr/0052](adr/0052-companion-media-results-use-canonical-artifacts.md) - companion image/video results reuse canonical agent artifacts and cross E2EE only as bounded references plus verified full-file chunks (superseded in this fork: the desktop companion was removed)
- [adr/0053](adr/0053-companion-computer-use-approvals-are-one-shot.md) - linked companions may approve one exact Computer use interruption under the desktop policy ceiling, with a short-lived one-shot permit (superseded in this fork: the desktop companion was removed)
- [adr/0054](adr/0054-clovy-presentation-retains-june-era-technical-identities.md) - Clovy is the product name while shipped June-era bundle, storage, API, update, and tool identities remain stable
- [adr/0055](adr/0055-clovy-technical-identity-migrates-through-a-compatibility-bridge.md) - (superseded by ADR-0068) supersedes ADR-0054: Clovy becomes canonical through idempotent migration and rollback-safe aliases while immutable OS identities preserve existing installs
- [adr/0056](adr/0056-interface-language-uses-an-in-repo-typed-catalog.md) - interface language (English, Português (Brasil)) uses an in-repo typed catalog, a localStorage preference independent of the transcription language, and a Tauri event mirrored into the native menus
- [adr/0057](adr/0057-activity-database-uses-sqlcipher-linked-app-wide.md) - the activity database is a separate SQLCipher file with a Keychain key; SQLCipher is linked app-wide on macOS and the unkeyed main database is unaffected
- [adr/0058](adr/0058-clovy-mcp-server-relays-to-the-running-app.md) - the Clovy MCP server is a stdio relay (`clovy-mcp`) to the running app over a same-user Unix socket or private local Windows named pipe with an installation secret; the binary never opens the databases
- [adr/0059](adr/0059-activity-is-per-installation-not-per-data-partition.md) - activity (timeline and coding-agent blocks) belongs to the installation, not to a data partition; MCP activity tools read it like the "Today" view while notes, dictations, and memories stay in the current partition
- [adr/0060](adr/0060-development-app-runs-as-a-signed-bundle-that-owns-its-grants.md) - `pnpm tauri:dev` runs a signed `Clovy Dev.app` beside Cargo's output that owns its privacy grants: no TCC resets, one Apple Development identity for the app and its helpers, and a launcher that disclaims the terminal's responsibility
- [adr/0061](adr/0061-dates-and-numbers-follow-the-interface-language.md) - dates, times, and numbers always format in the interface language (supersedes ADR-0056 decision 4); the date-format preference only picks day/month order; native errors are worded by stable code in the frontend
- [adr/0062](adr/0062-cli-engines-are-the-default-agent-native-harness-is-legacy.md) - Claude Code, else Codex, is the default engine of new sessions, note chat, Home (since ADR-0064), companion sessions, and routines (unattended, read-only tools plus Clovy's MCP and web); one recorded settings move; the legacy fallback is superseded by ADR-0072 (earlier superseded ADR-0038/0039 in part)
- [adr/0063](adr/0063-cli-turns-get-write-tools-through-an-approval-broker.md) - CLI chat turns get Clovy's write tools through a turn-bound `clovy-mcp` connection and an in-app approval broker; external clients stay read-only; user MCP servers and Linear reach CLIs through the same gateway
- [adr/0064](adr/0064-home-runs-on-the-default-cli-engine.md) - Home runs on the default CLI engine in its hidden session, without a remote chat route; old Home turns are imported once from local storage; the former remote Home commands are removed (supersedes ADR-0062 in part)
- [adr/0065](adr/0065-dictation-calls-the-users-own-provider-directly.md) - direct dictation with the user's own key; server fallback superseded by ADR-0073
- [adr/0066](adr/0066-github-connector-can-ride-the-github-cli-login.md) - the GitHub connector can connect on its own through a signed-in GitHub CLI (`credentialSource: "gh"`), reading the token from `gh` on every call and never storing it; read only at first, disconnect turns the automation off (amends ADR-0036)
- [adr/0067](adr/0067-computer-use-depends-only-on-local-gates.md) - Computer use is ready from the grant, the pinned driver and macOS permissions alone: no Pro/Max plan, Clovy vision model or remote rollout; the tool is offered only to CLI turns through `clovy-mcp` (amends ADR-0028)
- [adr/0068](adr/0068-clovy-takes-its-own-identity-with-a-one-time-migration.md) - supersedes ADR-0055: the app becomes `dev.guilhermexp.clovy` (`Clovy.app`, `clovy`), the extension gets the owner's key pair, and the first launch on macOS copies the June-era profile and credentials once (Windows starts clean); the compatibility bridges are removed
- [adr/0069](adr/0069-the-owner-publishes-own-releases-with-a-one-channel-updater.md) - amends ADR-0001: releases on `guilhermexp/os-clovy` with one stable channel and the owner's minisign key; only release builds check, every install waits for the owner and for recordings to end; Mac signing optional
- [adr/0070](adr/0070-notes-call-the-users-own-provider-directly.md) - direct note transcription/generation with the user's key; server fallback and Auto exception superseded by ADR-0073

- [adr/0072](adr/0072-cli-chat-replaces-the-native-agent.md) - removes the native agent and endpoint chat engines; CLI turns use Clovy MCP tools and the local approval broker, while neutral session persistence preserves history
- [adr/0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md) - remove the inference server and local token; direct user-key providers, independent text catalog, fal queue recovery, and no sharing/funding service
- [adr/0074](adr/0074-local-transcription-uses-a-pinned-offline-helper.md) - optional Parakeet Ultra engine, explicit pinned installation, offline helper and no provider fallback
- [adr/0075](adr/0075-desktop-opens-today-and-approves-external-mcp-actions.md) - Hoje como entrada do desktop, histórico de Rotinas e autoridade por operação para clientes MCP externos
- [Browser private host](adr/browser-private-agent-host.md) - pinned private browser, profile custody, coherent observation and anonymous crash recovery with physical leases and terminal records

## Enforceable rules (spec/)

Coding rules that should fail review if violated (distinct from the `specs/`
feature specs). Full index: [spec/index.md](../spec/index.md).

- UI copy: [spec/sentence-case](../spec/sentence-case.md), [spec/no-typographic-dashes](../spec/no-typographic-dashes.md), [spec/no-all-caps](../spec/no-all-caps.md)
- UI styling: [spec/icons-central-only](../spec/icons-central-only.md), [spec/design-tokens](../spec/design-tokens.md), [spec/no-tabular-numerals](../spec/no-tabular-numerals.md)
- Typography: [spec/type-scale](../spec/type-scale.md), [spec/font-weights](../spec/font-weights.md), [spec/font-families](../spec/font-families.md)
- Controls: [spec/control-sizes](../spec/control-sizes.md)
- Agent surface: [spec/mcp-tool-naming](../spec/mcp-tool-naming.md)

## Design system

The extracted design system: tokens, primitives, and the rules around them (see
the `spec/` entries above for the enforceable versions).

- [design/foundations.md](design/foundations.md) — theming model, token roles, and the type system (scale, heading mapping, two-weight system, family roles); ends with the pass-2 deviations worklist
- [design/components.md](design/components.md) — pattern-to-canonical-answer map for the shared `src/components/ui/` primitives, the settings markup contract, and what is not yet systematized
- [design/conventions.md](design/conventions.md) — flat-namespace naming, interaction and visual rules, theming pipeline, and how to open or extend the styleguide page
- [design/taste.md](design/taste.md) — the sensibility behind the rules (quiet by default, weight as punctuation, color spent not sprayed); the portable layer for other projects

## Agent skill config (docs/agents/)

Per-repo config the engineering skills read before acting (see the
"Agent skills" section in [AGENTS.md](../AGENTS.md)).

- [agents/issue-tracker.md](agents/issue-tracker.md) — work is tracked as harness tickets with OpenSpec changes, not GitHub Issues
- [agents/triage-labels.md](agents/triage-labels.md) — the five triage roles mapped to harness ticket states
- [agents/domain.md](agents/domain.md) — single-context layout; how skills consume CONTEXT.md and ADRs
- [agents/collaboration.md](agents/collaboration.md) — how agents build, delegate, and review across harnesses: the skill family map, reviewer-is-never-the-author, trust levels

## Subsystems

- [home-assistant.md](home-assistant.md) - registro histórico da interface Início, sua sessão oculta e a importação antiga
- [companion-protocol.md](companion-protocol.md) - versioned encrypted application protocol of the companion crates (the desktop companion and relay are removed; protocol crates remain historical)
- [browser-computer-use-prd.md](browser-computer-use-prd.md) — PRD: Browser use + Computer use plugins (JUN-278); extension in the user's browser + routines-only managed browser, phase-2 computer use
- [release-extension.md](release-extension.md) - browser extension store identity and local packaging checks (no automated store publication)
- [private-sharing-design.md](private-sharing-design.md) — retirement notice for private link sharing; no replacement service
- [computer-use-cua-driver-spike.md](computer-use-cua-driver-spike.md) — spike (JUN-288): run the bundled cua-driver outside the write jail as a broker-owned, separately-signed daemon (recommended on identity/lifecycle grounds; a fully relocated daemon does start in-jail, up to the TCC gate); pinning + installer-never-runs confirmed
- [plugins/portfolio.md](plugins/portfolio.md) — JUN-309 portfolio: current ChatGPT plugin surface inventory, ranking rubric, Clovy's top 10, shared product contract, sequencing, metrics, and explicit deferrals
  - [Google Workspace](plugins/google-workspace-prd.md) — [implementation plan](plugins/google-workspace-implementation-plan.md)
  - [Browser use](plugins/browser-use-prd.md) — [implementation plan](plugins/browser-use-implementation-plan.md)
  - [Slack](plugins/slack-prd.md) — [implementation plan](plugins/slack-implementation-plan.md)
  - [Microsoft 365](plugins/microsoft-365-prd.md) — [implementation plan](plugins/microsoft-365-implementation-plan.md)
  - [Computer use](plugins/computer-use-prd.md) — [implementation plan](plugins/computer-use-implementation-plan.md)
  - [Notion](plugins/notion-prd.md) — [implementation plan](plugins/notion-implementation-plan.md)
  - [GitHub](plugins/github-prd.md) — [implementation plan](plugins/github-implementation-plan.md)
  - [Linear](plugins/linear-prd.md) — [hosted MCP implementation plan](plugins/linear-hosted-mcp-implementation-plan.md), [original implementation plan](plugins/linear-implementation-plan.md), [Phase 0 OAuth spike findings](plugins/linear-oauth-spike.md)
  - [Documents](plugins/documents-prd.md) — [implementation plan](plugins/documents-implementation-plan.md)
  - [Spreadsheets](plugins/spreadsheets-prd.md) — [implementation plan](plugins/spreadsheets-implementation-plan.md)
- [plugins/integrations-next-ten.md](plugins/integrations-next-ten.md) - follow-on provider portfolio: ranks 11-20, auth feasibility, sequencing, and decision gates
  - [HubSpot](plugins/hubspot-prd.md) - [implementation plan](plugins/hubspot-implementation-plan.md)
  - [Salesforce](plugins/salesforce-prd.md) - [implementation plan](plugins/salesforce-implementation-plan.md)
  - [Asana](plugins/asana-prd.md) - [implementation plan](plugins/asana-implementation-plan.md)
  - [Box](plugins/box-prd.md) - [implementation plan](plugins/box-implementation-plan.md)
  - [GitLab](plugins/gitlab-prd.md) - [implementation plan](plugins/gitlab-implementation-plan.md)
  - [ClickUp](plugins/clickup-prd.md) - [implementation plan](plugins/clickup-implementation-plan.md)
  - [Dropbox](plugins/dropbox-prd.md) - [implementation plan](plugins/dropbox-implementation-plan.md)
  - [Pipedrive](plugins/pipedrive-prd.md) - [implementation plan](plugins/pipedrive-implementation-plan.md)
  - [Azure Boards](plugins/azure-boards-prd.md) - [implementation plan](plugins/azure-boards-implementation-plan.md)
  - [Canva](plugins/canva-prd.md) - [implementation plan](plugins/canva-implementation-plan.md)
- [activity-capture.md](activity-capture.md) - activity capture: capture loop and privacy rules, the encrypted `activity.sqlite3` schema, processing cursor, retention, and the store API later slices use
- [activity-timeline.md](activity-timeline.md) - activity timeline: incremental ETL from frames to sessions and gaps, categorizer, day stats, FTS5 search, the "Today" view and its lane extension point, the `search_activity` / `get_activity_timeline` agent tools, and the query API for later slices
- [day-intelligence.md](day-intelligence.md) - day intelligence: local distillation with on-device embeddings, hour reports with measured minutes, workstream fold, day summary and standup, daily schedule with wake recovery, notifications and quiet hours, schema and the read API for the MCP server
- [mcp-server.md](mcp-server.md) - relay autenticado `clovy-mcp`, configuração de clientes, aprovações externas, ferramentas de leitura e de turno, recursos e limites
- [agent-note-tools.md](agent-note-tools.md) - in-app agent note tools: `search_june` returns ids plus bounded excerpts, `get_note` / `get_dictation` page the full text in 256 KiB character-safe chunks; owns these tool names
- [coding-agent-sessions.md](coding-agent-sessions.md) - coding-agent session ingestion: read-only readers for seven local agents, block cutting and lifecycle, summaries by the agent's own CLI or the activity provider, the `coding_agent_blocks` schema (activity migration 3) and its query API, the "Today" view lane
- [i18n.md](i18n.md) - interface language: catalogs, `useT`/`t`, plurals and formatting, what is never translated, coverage inventory, adding a language
- [audio-pipeline.md](audio-pipeline.md) — capture → source separation → turns → transcription → note
- [dictation.md](dictation.md) - dictation take: shortcut to capture start, the direct Groq transcription route with the user's key, optional OpenRouter cleanup, "no speech" after Whisper, paste, warm-up, and latency targets
- [llm-providers.md](llm-providers.md) - own providers: agent CLIs and named OpenAI-compatible endpoints, per-use selection, one-shot generation API, structured-output levels, CLI isolation contract, and the CLI chat engine (per-CLI turn, resume, stream, cancel, permissions, Clovy's tools)
- [clovy-api-prd.md](clovy-api-prd.md) — historical server PRD; retired by ADR-0073, not a current implementation/deployment contract
- [telemetry-p3a-implementation-plan.md](telemetry-p3a-implementation-plan.md) — historical implementation plan for Clovy P3A (removed from the desktop in this fork)
- [private-connectors-prd.md](private-connectors-prd.md) — private connectors & away-mode relay: an assistant that acts in email and calendar without OpenSoftware readable data (local mode remains; hosted/away-mode portions are historical)
- [private-connectors-implementation-plan.md](private-connectors-implementation-plan.md) — implementation plan for private connectors phases (Phases 1-2 local mode implemented; see [adr/0016](adr/0016-private-connectors-local-mode.md))
- [private-connectors-threat-model.md](private-connectors-threat-model.md) — local-mode threat model: the source of truth for all connector privacy copy (what OpenSoftware can and cannot see, the trust surface, agent protections, the known runtime limitation)
- [configuration.md](configuration.md) — desktop configuration, direct provider keys, text catalog and fal queue limits
- [auto-model-rollout.md](auto-model-rollout.md) — historical canary, enablement, and rollback plan for retired automatic private model routing
- [development.md](development.md) — local development: quick start, local data, permissions, agent skills, verification commands
- [onboarding-design.md](onboarding-design.md) — onboarding flow design (verify against what shipped)

## Release & ops runbooks

- [companion-relay-runbook.md](companion-relay-runbook.md) - relay deployment, health, limits, and incident response (relay removal pending; the desktop companion is gone)
- [companion-apns-setup.md](companion-apns-setup.md) - Apple push setup for content-free wake hints (relay removal pending; the desktop companion is gone)
- [release-macos.md](release-macos.md) / [release-windows.md](release-windows.md) — building and validating the desktop app locally (no published releases)
- [computer-use-support.md](computer-use-support.md) - state guide, TCC recovery, signed helper self-test, and macOS release regression response
- [local-ci-signoff.md](local-ci-signoff.md) — local commit-status signoff that replaces the slow hosted PR jobs during agent iteration

## QA

- [qa/agent-driven-integration.md](qa/agent-driven-integration.md) — QA strategy (3 layers, skill-first agent-driven)
- [qa/computer-use-parity.md](qa/computer-use-parity.md) - JUN-278 parity, stricter safety differences, evidence map, and signed/manual release matrix
- [qa/jun-334-note-transcription-latency.md](qa/jun-334-note-transcription-latency.md) — measured baseline and proof for note transcription latency
- [qa/windows-audio-backlog.md](qa/windows-audio-backlog.md) - pendências de áudio de reunião no Windows, com bugs identificados, reprodução e limites da investigação
- [qa/agent-runtime-backlog.md](qa/agent-runtime-backlog.md) - pendências do chat Claude Code/Codex: aprovações, bypass, paridade das ferramentas e diagnóstico das integrações
- `qa/feature-user-stories.tsv` — story → code → test traceability matrix
- `qa/agent-e2e-qa-runs/` — dated end-to-end QA run logs

## Historical server records

- [history/clovy-api-migrations/README.md](history/clovy-api-migrations/README.md) - byte-for-byte archive of the two retired server SQL migrations (shares and companion devices); never executed by the app or added to its desktop migration catalog (ADR-0073)

## Feature designs and implementation plans

- [superpowers/specs/2026-07-15-faster-note-transcription-design.md](superpowers/specs/2026-07-15-faster-note-transcription-design.md) — approved JUN-334 design for reducing note transcription latency
- [superpowers/plans/2026-07-15-faster-note-transcription.md](superpowers/plans/2026-07-15-faster-note-transcription.md) — executable JUN-334 implementation plan for the approved design
- [superpowers/specs/2026-07-22-transcription-follow-latest-design.md](superpowers/specs/2026-07-22-transcription-follow-latest-design.md) — approved design for keeping the Transcription tab on the live edge while the reader follows it
- [superpowers/plans/2026-07-22-transcription-follow-latest.md](superpowers/plans/2026-07-22-transcription-follow-latest.md) — implementation plan for the follow-latest design

## Feature specs (Spec Kit)

Each spec folder holds `spec / plan / research / data-model / quickstart /
tasks / contracts / checklists`.

- `specs/001-tauri-note-mvp` — Notes MVP (shipped)
- `specs/002-system-audio-source-mode` — audio source modes (shipped)
- `specs/003-conversation-turns` — dual-source conversation turns (current; the tech + structure entrypoint)

## Gaps (no doc yet — candidates for new docs/ADRs)

- **Roadmap / MVP scope** — no single sequenced source of truth across the active tracks (admin surfaces, reliability).
- **Dictation ADR** — the low-latency request shape + charge timing (flagged in CONTEXT.md).

## Security

- [../SECURITY.md](../SECURITY.md) — vulnerability reporting + supported versions
