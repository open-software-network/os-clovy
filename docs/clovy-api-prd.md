# Clovy API — proxy all upstream AI traffic and fully integrate OS Accounts authorize/charge

> **Historical document (2026-10-28).** Server, Auto/funding, hosted relay and
> native-agent claims below describe an earlier design, not the current app.
> The inference server is removed; chat/routines use installed CLI engines,
> Groq/OpenRouter/fal operations use the user's keys directly, and sharing has
> no replacement. Current boundaries are in
> [ADR-0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md),
> [llm-providers.md](llm-providers.md), and [configuration.md](configuration.md).


> Read [`/CONTEXT.md`](../CONTEXT.md) first for the glossary. Terms in **bold**
> below (Clovy, Clovy API, OS Accounts, upstream provider, dictation, note
> transcription, note generation, credit price) are defined there.
>
> **Status (2026-10-21):** historical PRD. The `desvincular-repo-ci` change
> removed the OS Accounts token verifier and authorize/charge client, the OS
> Platform issue sink, the attestation page, and the GHCR build, promotion and
> deployment pipeline. Clovy API now runs only in local mode (one local bearer
> token, no charges, `127.0.0.1` by default). The provider routing, pricing
> table, envelope and crate layout below still describe the code; the OS
> Accounts, CI and deployment parts are kept only as history and are marked
> where they no longer apply.

## Problem Statement

> **Historical context.** This section records the situation that motivated
> Clovy API, before it existed. It is not the current state of the product.

A user installing Clovy before Clovy API could record meetings, get
transcripts, and generate notes, but only if **they** supplied their own
OpenAI and Venice API keys in a local `.env`. Distributing Clovy as a `.app` to
anyone who isn't a developer had three problems:

- The user had no way to acquire or configure provider keys.
- Any keys baked into the binary would be extractable: anyone running
  `strings` on the `.app` could siphon the service budget and run unlimited
  inference on Clovy's bill.
- **OS Accounts credits** were wired up for identity and balance display
  (`os_accounts.rs`), but nothing charged them: the wallet read correctly and
  never decremented.

The result: Clovy could not be shipped publicly as a paid product, because
there was no safe way to bill metered AI usage from a public client, and it
could not be used by anyone without the technical knowledge to register their
own upstream provider accounts.

## Solution

Build **Clovy API**, a confidential backend service that:

1. Holds the upstream service keys (Groq for audio, OpenRouter for text, fal for
   images and video, Tavily for web search and fetch), which Clovy never ships.
2. Holds the OS Accounts App API key (`osk_…`) that authenticates Clovy to OS
   Accounts as a billing-capable app.
3. Verifies every incoming request's OS Accounts access token locally
   (JWKS / ES256), extracting the **user ID** (`usr_…`).
4. For each metered call, runs the B-shaped contract: `POST /authorize` →
   call the upstream provider → `POST /charge`, deterministic
   `idempotency_key` per logical operation.
5. Returns the work result (transcript, note, cleaned text, model list) to
   Clovy.

After this lands, Clovy ships with **zero upstream provider keys**. Every
Clovy-funded call that goes through Clovy API is billed against the signed-in
user's OS Accounts wallet at the configured **credit price** for the chosen
**upstream model**, and the existing "Top up credits" affordance in the Account
settings funds that usage. Users see "Insufficient credits → Top up" instead of
a generic provider error when they run dry.

Usage outside the wallet stays available:

- A user's own provider key, sent per route as `x-<provider>-api-key`, runs that
  call on the user's provider account; Clovy API forwards it and skips wallet
  metering. An explicit Auto request is the exception: Clovy picks the model, so
  it drops the key and funds the call.
- Agent CLIs and the user's own OpenAI-compatible endpoints run on the desktop
  and never pass through Clovy API.

Credit prices relate to upstream cost as follows: usage-priced text and audio
models pass upstream cost through without a Clovy markup; image generation and
editing are flat prices set at about 2x fal's list price; video applies a 2.0x
markup to fal's per-request quote; web search and fetch are flat prices. The
media prices are list-price estimates, not charges observed on an account.

## User Stories

### End user — first run and sign-in

1. As a **first-time Clovy user**, I want to install the app without
   configuring any third-party API keys, so that I can get to recording within
   one minute of double-clicking the `.app`.
2. As a **first-time Clovy user**, I want to **sign in with Open Software**
   directly from the Account settings, so that my identity and credit balance
   are tied to my OS Accounts wallet from the start.
3. As a **signed-in Clovy user**, I want to see my current credit balance in
   the Account settings, so that I know what I can spend before recording.
4. As a **signed-in Clovy user**, I want a one-click "Top up" button that
   opens the OS Accounts portal in my browser, so that I can buy more credits
   without leaving Clovy's flow.
5. As a **signed-in Clovy user**, I want my balance in the Account settings
   to refresh automatically after a successful top-up returns me to the app,
   so that I don't have to manually check whether the purchase went through.

### End user — note transcription and generation

6. As a **Clovy user with sufficient credits**, I want to record a meeting,
   stop, and have the **note transcription** appear without seeing any error
   about provider keys, so that the recording-to-note flow feels seamless.
7. As a **Clovy user with sufficient credits**, I want the cost of each
   transcription to be deducted from my OS Accounts balance after it
   completes, so that I pay for what I used and not for upload failures.
8. As a **Clovy user**, I want **note generation** (turning the transcript
   into structured markdown) to also debit the same OS Accounts wallet, so
   that there is one place to see all my AI spend.
9. As a **Clovy user with insufficient credits**, I want to see a clear
   "You're out of credits — top up to continue" message instead of a generic
   error when I try to transcribe or generate, so that I know exactly what to
   do.
10. As a **Clovy user with insufficient credits**, the "Top up" affordance
    from the error message should take me straight to the OS Accounts portal,
    so that I'm not hunting through settings.
11. As a **Clovy user retrying a failed network call**, I want my second
    attempt to bill me only once, so that flaky wifi doesn't double-charge me.

### End user — dictation

12. As a **Clovy user with sufficient credits**, I want **dictation**
    (push-to-talk → cleaned-up text) to work without me supplying any
    provider key, so that dictation is usable on a fresh install.
13. As a **Clovy user using dictation**, I want each dictation event to
    debit my OS Accounts balance, so that dictation is metered the same way
    as note transcription.
14. As a **Clovy user using dictation**, I want the latency of dictation to
    stay tight (text appears within a fraction of a second of release), so
    that the new billing pipeline doesn't make the feature feel sluggish.
15. As a **Clovy user**, I want to choose between the Groq Whisper models
    for transcription in Settings, so that I can trade speed for accuracy.
16. As a **Clovy user**, I want models that Clovy hasn't priced to NOT
    appear in the picker, so that I never select an option that will fail at
    request time.

### Developer — running and contributing to Clovy

17. As a **Clovy developer running the app locally**, I want to point at
    either a local Clovy API or a staging deployment via a single env var,
    so that I can test against production-shape infra without rebuilding.
18. As a **Clovy developer**, I want to clone the repo and build both the
    Tauri client and Clovy API from the same `git clone`, so that
    end-to-end changes are one PR.
19. As a **Clovy developer**, I want no Clovy service keys for upstream
    providers anywhere in the Tauri-side `.env` or codebase, so that a leaked
    Clovy build cannot drain our provider budgets. A user's own provider key
    stays in that user's local secret store and is sent only to the route
    that uses it.

### Developer — Clovy API backend

20. As a **Clovy API maintainer**, I want every paid request to atomically
    `authorize → upstream → charge` with a deterministic idempotency key, so
    that retries are safe and concurrent requests for the same user respect
    the wallet.
21. As a **Clovy API maintainer**, I want the JWKS for OS Accounts cached
    in-process and refreshed only on `kid` miss, so that every request
    doesn't hammer OS Accounts.
22. As a **Clovy API maintainer**, I want each upstream provider sitting
    behind a `Transcriber` / `Generator` / `Cleaner` trait, so that I can
    swap or add a provider without changing the orchestration code.
23. As a **Clovy API maintainer**, I want a typed pricing table loaded from
    config, so that adding or repricing an upstream model is a config diff
    not a code change. Usage prices pass through upstream cost without an
    additional Clovy markup.
24. As a **Clovy API maintainer**, I want unknown models (no credit price)
    to be rejected at the API boundary with a clear error code, so that we
    never silently charge $0 or absorb unpriced usage.
25. As a **Clovy API maintainer**, I want structured `tracing` logs with
    `usr_…`, action, model, and credits charged on every settled request, so
    that I can investigate billing disputes without re-deriving anything.

### Developer — CI and deployment

26. As a **Clovy API maintainer**, I want PRs that touch only the Tauri
    side to NOT trigger Rust CI, so that a UI tweak doesn't run a 5-minute
    cargo build.
_Stories 27 to 29 (GHCR images, manual promotion to `:production`, a hosted
health probe) described the removed build and deployment pipeline._

### OS Accounts perspective (for completeness)

_Historical: Clovy API no longer calls OS Accounts; local mode authorizes every
hold and never charges._

30. As an **OS Accounts admin**, I want each metered call from Clovy to
    include an action slug that distinguishes dictation from note flows
    (`dictate_transcribe`, `dictate_cleanup`, `note_transcribe`,
    `note_generate`), so that per-action billing analytics are legible.
31. As an **OS Accounts platform**, I expect Clovy API to never call
    `/charge` without a preceding `/authorize`, to use a deterministic
    `idempotency_key` per logical operation, and to send both the App API
    key (header) and the Action token (body) on every charge, so that the
    contract holds and replays are deduped correctly.

## Implementation Decisions

### Repo layout and crate split

- **Clovy API lives in this repo**, not a separate repo. Cargo workspace
  rooted at `clovy-api/` (sibling of `src-tauri/` and `src/`). One `git
  clone` for client + backend; CI workflows in the monorepo gate on
  `paths: clovy-api/**`.
- **Seven crates.** Clovy API began as a stateless service and later added the
  `clovy-persistence` crate for durable sharing and companion state. Crates:
  `clovy-domain`, `clovy-services`, `clovy-providers`, `clovy-config`,
  `clovy-persistence`, `clovy-api`, and `clovy-api-server`. The binary is named
  `clovy-api`.
- **Service prefix `clovy`.** All library crate names start with `clovy-`.
  Match the convention used by `accounts-*` (`os-accounts`) and
  `fellow-*` (`os-platform`).
- **Follow the house backend recipe** (formerly vendored as the
  `os-rust-backend` skill, removed with the organization skills) for
  everything not explicitly
  overridden: edition 2024, pinned toolchain `1.95.0`, workspace deps,
  workspace lints (`-D warnings`, no `unwrap`/`expect`/`panic`/`dbg`,
  `unsafe_code = forbid`), `ApiResponse<T>` envelope on every endpoint,
  `tracing` only with structured fields, no `std::env::var` outside
  `clovy-config`, error code bands per the standard table.

### Deep modules and their interfaces (no file paths)

- **`Transcriber` (trait, `clovy-domain`)** — `transcribe(audio, model) ->
  Result<Transcript, DomainError>`. One impl in `clovy-providers`: Groq
  Whisper. Interface unchanged across upstreams. A request's `context` goes to
  Whisper verbatim as its `prompt` (the **ASR hint**), never truncated or
  rewritten server-side; a prompt Groq refuses (400/422) is reported as
  `transcription_rejected`, without its body, the key or the audio. Groq
  documents a 224-token prompt limit, so the desktop builds a bounded,
  dictionary-only hint; other API consumers own their own bound.
- **`Generator` (trait, `clovy-domain`)** — `generate(prompt_inputs, model)
  -> Result<GeneratedNote, DomainError>`. One impl (OpenRouter).
- **`Cleaner` (trait, `clovy-domain`)** — `cleanup(kind, text, context,
  style, model) -> Result<String, DomainError>`. One impl (OpenRouter). Two
  kinds with their own instructions: dictation cleanup, and note-transcript
  correction (`style = "note_transcript_cleanup"`), which receives the **full
  correction context** (whole dictionary plus previous transcript), unlike the
  bounded ASR hint.
- **`OsAccountsClient` (trait, `clovy-domain`)** — `authorize(user, action,
  estimate, hold_ttl) -> Result<Authorization, _>` and `charge(token,
  credits, idempotency_key) -> Result<Receipt, _>`. One impl
  (`OsAccountsHttpClient` in `clovy-providers`) — HTTP, App API key
  bearer, envelope-aware, no retries beyond what the platform guarantees.
  _Now: the only impl is `LocalDevOsAccountsClient`, which authorizes every
  hold and never charges; `OsAccountsHttpClient` was removed._
- **`TokenVerifier` (trait, `clovy-domain`)** — `verify(jwt) ->
  Result<UserId, AuthError>`. One impl (`JwksTokenVerifier` in
  `clovy-providers`) — caches JWKS in-process, refreshes on `kid` miss,
  ES256-only, strict `iss`/`aud`, `exp` with small leeway. Never trust
  the token's `alg` header; never fetch JWKS per request.
  _Now: the only impl is `LocalDevTokenVerifier` (the local bearer token);
  `JwksTokenVerifier` was removed._
- **`PricingTable` (`clovy-services`)** — `price_credits(model_id, units)
  -> Result<Credits, NotPriced>`. Pure function over the config table.
  Single source of truth for credit cost. Loaded from `clovy-config`.
- **`WavDurationProbe` (`clovy-providers`)** — `probe(bytes) ->
  Result<Duration, ProbeError>`. Pure, uses the `hound` crate (already a
  workspace dep in `src-tauri`; should be added to the Clovy API
  workspace too).
- **`NoteTranscribeService`, `NoteGenerateService`, `DictateService`
  (`clovy-services`)** — orchestrators. Each takes a `UserId` (from the
  verified token), resolves the **credit price** via `PricingTable`,
  computes `estimate_credits`, calls `OsAccountsClient::authorize`, calls
  the relevant `Transcriber` / `Generator` / `Cleaner`, then
  `OsAccountsClient::charge` with the actual credits and a deterministic
  `idempotency_key` derived from the logical operation. **`DictateService`
  owns both dictate transcribe and dictate cleanup** (decision: merge what
  would have been two services into one — same domain, same UX).
- **Tauri-side `clovy_api`** — single module exposing `transcribe`,
  `generate`, `dictate`, `dictate_cleanup`, `list_models`. Each function
  reads the OS Accounts access token from the macOS Keychain (helpers
  promoted out of `os_accounts.rs` into a shared submodule), builds a
  `Bearer <jwt>` request to `{CLOVY_API_URL}/v1/…`, on `401` refreshes
  tokens via the existing refresh path and retries once, on `402` with
  `error_code: 4301` returns a typed `AppError::InsufficientCredits` the
  UI maps to the existing top-up affordance.

### Public HTTP surface

All routes prefixed with `/v1/`. All non-public responses wrapped in
`ApiResponse<T>`. All metered routes require `Authorization: Bearer
<access_jwt>`.

| Method | Path | Action slug | Body / Query | Notes |
|---|---|---|---|---|
| `POST` | `/v1/transcribe` | `note_transcribe` | multipart audio + `{title, context?, model?}` | Server probes WAV duration → authorize → upstream → charge. |
| `POST` | `/v1/generate` | `note_generate` | JSON `{title, transcript, manualNotes?, language?, existingGeneratedNote?, model?}` | Charge based on actual `prompt_tokens + completion_tokens` from upstream `usage`. |
| `POST` | `/v1/dictate` | `dictate_transcribe` | multipart audio + `{model}` | Latency-sensitive; same shape as `/v1/transcribe` but separate action slug for OS Accounts analytics. |
| `POST` | `/v1/dictate/cleanup` | `dictate_cleanup` | JSON `{text, dictionaryContext?, style, model}` | Dictation cleanup; `style = "note_transcript_cleanup"` runs note-transcript correction over the full `dictionaryContext` instead. |
| `GET` | `/v1/models?type=asr\|text\|image\|video` | *(unmetered)* | — | Configured Groq ASR and OpenRouter text rows; explicit image/video filters list priced fal generation endpoints with `serviceAvailable`. Omitting type keeps the original text/ASR list. No service credentials are exposed. |
| `GET` | `/livez` | *(unmetered)* | — | Liveness probe. |

**Provider keys.** A user's own key rides as `x-<provider>-api-key`
(`x-groq-api-key`, `x-openrouter-api-key`, `x-fal-api-key`,
`x-tavily-api-key`), and each route accepts only its own provider's header.
Any other non-blank `x-<name>-api-key` is refused with
`400 <name>_api_key_not_accepted` before funding or forwarding, so a key never
pays for, or reaches, a provider it was not meant for.

**Retired Venice key: a breaking header.** Released clients that stored a
Venice key still send `x-venice-api-key`. It is refused
(`venice_api_key_not_accepted`) rather than ignored, because ignoring it would
silently move a user who chose their own key onto Clovy credits. Those
installs fail every keyed route until they update. Request and response bodies,
the historical client-contract fixtures and the error-code registry are
unchanged, and retired model ids are still served by a fallback. This break is
in tension with the v1 compatibility policy (ADR 0021): the cutover covers
local development with the current client and the new credentials only. A
publication and support decision for installs still holding a Venice key is
required before any production deploy.

### Estimate strategy (server-side only)

- **Audio (`/v1/transcribe`, `/v1/dictate`):** probe WAV duration from the
  uploaded bytes via `WavDurationProbe`. `estimate = ceil(seconds ×
  credits_per_second_for(model))`. Same number for `charge` (transcription
  cost is duration-bound).
- **Text (`/v1/generate`, `/v1/dictate/cleanup`):** `estimate =
  ceil((chars/4 + max_output_tokens) × credits_per_token_for(model))`. On
  `charge`, settle with actual `prompt_tokens + completion_tokens` from
  the upstream's `usage` block.
- **Never trust a client-supplied estimate.** Lying on the estimate
  defeats the wallet hold's concurrency protection.

### Error mapping (envelope `error_code` → HTTP)

Follow the standard house error-code bands. Notable specifics:

- OS Accounts `4301` (insufficient credits) → `402` with `error_code: 4301`.
- JWKS verify failure / missing token → `401` with `error_code: 3001`.
- Unknown model (no credit price configured) → `422` with `error_code:
  4201` (or similar in the validation band) and `message:
  "model_not_priced"`.
- Upstream provider failure (5xx, timeout, malformed body) → `502` with
  `error_code: 5001` and a redacted message — never leak the upstream's
  body verbatim.

### Idempotency key shape

Deterministic, derived from a stable per-operation identifier in Clovy:

- `/v1/transcribe`: `note_transcribe:<usr_>:<noteId>`
- `/v1/generate`: `note_generate:<usr_>:<noteId>:<promptVersion>`
- `/v1/dictate`: `dictate_transcribe:<usr_>:<sessionId>:<utteranceId>`
- `/v1/dictate/cleanup`: `dictate_cleanup:<usr_>:<sessionId>:<utteranceId>`

The Clovy client passes the stable identifier in the request (`noteId` /
`sessionId+utteranceId`); Clovy API constructs the full key
server-side from the verified `usr_` + the client-supplied operation id.
Clients never compose the full key themselves.

### Configuration

Clovy API (typed `AppConfig` in `clovy-config`, figment-merged from
defaults + `config.toml` + env, no `std::env::var` calls outside):

- `[server]` — `host` (`127.0.0.1` by default), `port`, `request_timeout_secs`.
- `[os_accounts]` — now only credit estimates and hold TTLs; the original
  `api_url`, `app_api_key`, `iss`, `aud` and `jwks_refresh_secs` were removed
  with OS Accounts verification and charging.
- `[local_dev]` — the local bearer token and user id (local mode is the only
  mode).
- `[upstreams.groq]`, `[upstreams.openrouter]`, `[upstreams.fal]`,
  `[upstreams.tavily]` — `api_key` (env-only, optional), `base_url`.
- `[pricing]` — table keyed by `model_id`, each entry `{ unit: "seconds" |
  "tokens", credits_per_unit: u64 }`. Values pass through upstream cost
  without an additional Clovy markup. Loaded from
  `config.toml` baked into the image; `CLOVY__PRICING__…` env overrides
  supported via figment for per-env tweaking.

Clovy (Tauri client):

- `CLOVY_API_URL` — baked production default in a constant; `.env`
  override supported (same model recommended for `OS_ACCOUNTS_URL` /
  `OS_ACCOUNTS_API_URL` going forward).
- `OPENAI_API_KEY`, `VENICE_API_KEY`, `OPENAI_API_BASE_URL`,
  `VENICE_API_BASE_URL`, `VENICE_DICTATION_CLEANUP_MODEL` — **removed**
  from `.env.example` and from `providers/mod.rs` accessors.

### CI

One workflow under `.github/workflows/`: `clovy-api.yml` — PR + push to
`main`, gated on `paths: clovy-api/**`; fmt, clippy and tests on GitHub-hosted
runners. It never publishes. The original GHCR build and promotion workflows
(`build-clovy-api.yml`, `promote-clovy-api.yml`) were removed by the
`desvincular-repo-ci` change.

### Tauri-side refactor

- Add `src-tauri/src/clovy_api.rs` with the functions listed above.
- Extract `load_tokens()` / `refresh()` out of the private API of
  `os_accounts.rs` into a shared submodule (e.g. `os_accounts::tokens`)
  so both modules use the same token storage.
- Delete `src-tauri/src/providers/transcription.rs`,
  `src-tauri/src/providers/generation.rs`, and the upstream-key accessors
  in `providers/mod.rs` (`openai_api_key`, `venice_api_key`,
  `openai_api_base_url`, `venice_api_base_url`,
  `dictation_cleanup_model`). Move the model-list shape used by
  Settings into `clovy_api::list_models` (or a small `model_picker.rs`
  cache if a separate cache is wanted).
- Update callers in `commands.rs` and `dictation.rs` to import from
  `clovy_api` instead of `providers::*`.
- Map Clovy API `error_code: 4301` to whatever existing "insufficient
  credits" UI affordance the Account/Note flows already use (or add one
  if missing) — should open the system browser at OS Accounts via the
  existing `os_accounts_top_up` command.

### Out-of-repo prerequisites

_Historical: OS Accounts registration, Railway projects and GHCR permissions
were prerequisites of the hosted deployment that was removed._

## Testing Decisions

### What makes a good test for this work

- **Test external behavior, not implementation details.** For Clovy API,
  that means HTTP request/response shape, envelope contents, error codes,
  the sequence of OS Accounts calls observed at a wiremock — never "this
  function called that function".
- **No I/O in unit tests.** If a test needs HTTP, it's an integration
  test and lives in `crates/<x>/tests/`.
- **Mock externals with `wiremock`, not hand-rolled fakes.** Verifies the
  real HTTP shape, not our mental model of it.
- **Tests assert on the envelope, not just the HTTP status.** A
  successful response is `{ success: true, data: ... }`; a known failure
  is `{ success: false, error_code: 4301, message: "..." }`. Tests
  should match the same `error_code` Clovy will branch on.

### Modules with tests in scope

- **`PricingTable` (unit, `clovy-services`)** — pure function over the
  config table. Cases: known model returns expected credits for a given
  unit count; unknown model returns `NotPriced`; integer overflow / very
  large unit counts handled cleanly; per-second and per-token tables
  resolved correctly when both are configured.
- **`OsAccountsHttpClient` (integration vs `wiremock`, `clovy-providers`)** —
  exercises the real HTTP shape against a wiremock OS Accounts. Cases:
  successful `authorize` returns a parsed `Authorization` with the
  Action token; `authorize` with insufficient balance returns
  `Authorization { allowed: false, reason: "insufficient_available_balance" }`;
  successful `charge` returns `Receipt` with `idempotent_replay: false`;
  same `charge` replayed with same key returns `idempotent_replay:
  true`; missing App API key on `charge` returns the expected error
  envelope; expired hold returns the expected error.

All other modules ship without dedicated tests at v1 — the wiremock'd
`OsAccountsHttpClient` integration covers the most error-prone
boundary, and the `PricingTable` unit tests cover the only pure logic
that materially affects user bills. Handler-level end-to-end tests can
be added incrementally as bugs surface.

### Prior art

- This repo has no Rust backend yet — no prior art in-repo for the test
  shape.
- The `os-accounts` and `os-platform` (`fellow`) repos use the same
  test stack: `rstest` for parametrization, `wiremock` for HTTP mocks,
  `pretty_assertions` for diff output, integration tests in
  `crates/<x>/tests/` per the house backend recipe. Mirror that style.

## Out of Scope

- **Streaming responses** (SSE / chunked) for `/v1/generate`. Current
  Clovy UX is non-streaming; add later as its own feature.
- **Postgres / persistence in Clovy API.** No DB v1; `clovy-persistence`
  crate not created. Idempotency dedup is provided by OS Accounts; the
  rare network-retry-causes-duplicate-upstream-call cost is accepted.
- **In-memory result cache.** Out of scope v1; consider only if observed
  retry rates make wasted upstream calls material.
- **Splitting `clovy-api/` into its own repo.** Monorepo is the decision
  for v1; can be hoisted out later if the repos want independent
  versioning.
- **ADRs for the architectural deviations from the house backend
  recipe** (no persistence crate, monorepo, baked URL defaults). User
  explicitly declined to write ADRs.
- **Async-charge optimisation for dictation.** The latency cost of two
  OS Accounts round-trips per dictation is real but tolerable; an
  optimisation (return result first, charge in the background) is a
  separate enhancement once we have telemetry to justify it.
- **Building any kind of top-up / checkout UI in Clovy or Clovy API.**
  Credits are granted only by OS Accounts after a verified Stripe
  webhook. Top-ups always hand off to the OS Accounts portal in the
  system browser.
- **Migrating identity off OS Accounts / supporting alternative IdPs.**
  Out of scope forever — owning identity is OS Accounts' job.
- **Resolving the `bun.lock` vs `pnpm-lock.yaml` divergence** in this
  repo. Unrelated cleanup, worth picking one source of truth but not
  blocking on it. _(Since resolved: `bun.lock` was removed and pnpm is
  the only package manager - see `spec/package-install-security.md`.)_

## Further Notes

- **Glossary alignment.** Throughout the implementation, prefer the
  vocabulary from `CONTEXT.md`: **Clovy** (the app), **Clovy API**
  (the backend), **OS Accounts** (the platform), **upstream provider**
  (Groq / OpenRouter / fal / Tavily), **dictation** vs **note transcription** vs **note
  generation** (never "transcribe" or "generate" alone), **credit
  price** (per upstream model). Avoid: "proxy" (overloaded with
  HTTP-CONNECT), "transcribe" without qualifier, "credits" referring to
  upstream cost.
- **The seven-crate vs six-crate decision is a real deviation from house
  style** (the house recipe mandates seven). It is intentional and
  documented here; a maintainer who needs persistence later should add
  the missing crate at that time rather than wedging persistence into
  `clovy-services` or `clovy-providers`.
- **Hosting target was Railway**, later a TEE deployment; both were removed by
  the `desvincular-repo-ci` change. Clovy API now runs locally next to the
  desktop app.
- **The Clovy binary will bake `CLOVY_API_URL` as a production
  default**, env-overridable for dev/staging. This is a small but real
  deviation from the current `os_accounts.rs` pattern (which requires
  env config and fails closed). The same change should eventually be
  made to `OS_ACCOUNTS_URL` / `OS_ACCOUNTS_API_URL` for the same reason
  — the moment Clovy is distributed to non-developers, env-only
  configuration breaks.
- **The `model_picker` / Settings UI does not need server changes
  beyond the new `/v1/models` endpoint.** The existing TypeScript shape
  in `AppSettings.tsx` already iterates a flat list of model DTOs.
