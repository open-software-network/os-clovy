# Clovy P3A — implementation plan

> **Fork note:** P3A telemetry was removed from this fork's desktop app (the
> `desvincular-servicos-open-software` change), along with its PRD, question
> catalog and user doc. This plan is kept as history only.
>
> Originally the engineering companion to the P3A PRD, whose "What is never
> collected" section was a hard constraint on every design choice below.

## Architecture at a glance

```
┌──────────────── desktop (os-clovy) ───────────────┐
│ feature code ──► p3a::record(Question::…)         │
│                    │ (no-op unless consented)     │
│                    ▼                              │
│   local counter + reported cursor (sqlite)        │
│                    │ coalesced wake-up signal     │
│                    ▼                              │
│   one reporter ──► one POST per event increment   │
│     (serialized, bounded retry backoff)            │
│                 user-authenticated to Clovy API    │
└────────────────────┬──────────────────────────────┘
                     ▼
        Clovy API  POST /v1/p3a/reports   (TEE, attested)
          validate against catalog, discard user id
                     │ forward to OS Accounts with service token
                     ▼
        OS Accounts  p3a_aggregates (Postgres)
          UPSERT counter++, raw report discarded
          k>=50 suppression at read; 12-month prune
                     ▼
              Grafana dashboards (iac/ stack)
```

Three properties fall out of this shape:

- **Content firewall is type-level.** The report struct contains only a
  question enum, a bucket index (`u8`), a platform enum, a version-series
  string matched against `^\d+\.\d+\.x$`, and an ISO-week string. There is
  no field a transcript could travel in. `#[serde(deny_unknown_fields)]`
  on the server closes the other direction.
- **Identity stripped at ingestion.** The desktop uses the existing user
  token only so Clovy API can reject unauthenticated writes. The token is not
  part of the report schema, is not forwarded to OS Accounts telemetry
  storage, and is not stored as telemetry data.
- **Nothing durable exists but aggregates.** Clovy API is already a
  stateless proxy; OS Accounts stores only `(question, epoch, platform,
  version, bucket) -> count` in telemetry aggregate tables that are not
  joined to OS Accounts users, wallets, balances, or subscriptions. Desktop
  counters are local retry cursors, not the team-visible data source.

### Why this trust model and not client-side crypto (STAR/Constellation)

Brave layers STAR/Constellation threshold encryption on top so that even
their server can't read an answer unless >= 50 identical answers exist.
That is the right end state, but it needs a randomness server, threshold
aggregator, and epoch key infrastructure — disproportionate at Clovy's
scale. Our interim equivalent: validation and forwarding run inside the
same Intel TDX TEE as Clovy API, so "validate, forward, discard" is part of
the attested image users can already verify via `/verify`. Durable aggregate
counters live in OS Accounts, where the telemetry schema stores only
aggregate cells. Constellation-style encryption is Phase 4, gated on volume
that makes k >= 50 routine.

## Phase 0 — consent + catalog

This established the consent flag, public catalog, and UI before network
reporting. Everything remains inspectable in code and docs.

### Desktop: consent flag (Rust-side, durable)

Copy the `providers/mod.rs` persisted-settings pattern exactly:

- New `src-tauri/src/p3a/mod.rs`:
  - `P3aSettings { enabled: bool, consent_version: u32, consented_at_week: Option<String> }`
    persisted to `p3a-settings.json` in `app_config_dir()`.
  - Managed `Mutex<P3aSettings>` registered in `lib.rs` `setup(app)`.
  - Commands `p3a_settings()` and `set_p3a_enabled(bool)` in
    `commands.rs`. Disabling deletes local counters in the same call —
    "off means off" is one transaction, not two steps.
- Consent lives Rust-side (not localStorage) because the sender is Rust
  and must not depend on a webview being open; frontend mirrors it via the
  command + a `clovy:p3a` CustomEvent, same pattern as
  `src/lib/rampart-privacy.ts`.

### Desktop: question catalog as code

- `src-tauri/src/p3a/questions.rs`: a `Question` enum with, per variant:
  stable wire id (`general.active-days`), bucket boundaries, cadence,
  and a doc link. Bucketization is a pure function
  `fn bucket(q: Question, raw: u64) -> u8` — trivially unit-testable.
- CI parity test: walks the enum and asserts every variant appears in
  `docs/telemetry-questions.md` with matching id and bucket labels (same
  spirit as the icon/dash lint rules — conventions enforced, not hoped).

### Desktop: UI

- `src/components/settings/PrivacySettingsSection.tsx`, rendered inside the
  General settings panel. Toggle + explanation + link. Model the section on
  `AgentSettingsSection.tsx` (rampart toggle is the house precedent for a
  privacy control).
- Onboarding: one new step in `src/components/onboarding/`, unchecked by
  default, per PRD copy rules.

### Docs

- `docs/telemetry-questions.md` (public catalog, human-readable).
- `PRIVACY.md` at repo root (currently missing) summarizing the device-local
  contract and linking the catalog.

**Exit criteria:** toggle persists across restarts; disabled state
verified to make zero network calls (wiremock test asserting no requests);
catalog CI test green.

## Phase 1 — record, report, aggregate

### Desktop: local counters

- sqlite migration `src-tauri/migrations/010_p3a_counters.sql`:
  `p3a_counters (question_id TEXT, epoch TEXT, raw_value INTEGER,
  reported_value INTEGER, PRIMARY KEY (question_id, epoch))` + repository in
  `src-tauri/src/db/repositories.rs`. Raw values stay local; the team sees
  only the aggregate increments that reach OS Accounts.
- `p3a::record(question)` / `p3a::record_value(question, v)`: cheap, sync
  signature, internally fire-and-forget; **no-op before consent check**.
  Call sites (all Rust, where the events already flow):
  - dictation session completed → `dictation.rs`
  - recording completed + audio source mode → recording finalize path in
    `commands.rs` / `audio/`
  - agent session started → `agent_runtime/api.rs`: record `agent.sessions`
    only after `create_agent_session` successfully creates and returns the
    stored session (lines 202-230). Do not record it from `start_agent_run`
    (lines 498-550), which runs for every user turn. The plan predates the
    ADR-0038 runtime migration and originally named the removed
    `hermes_bridge.rs`.
  - model/privacy-mode selection → `providers/mod.rs`
  - app foreground day → `lib.rs` setup / focus handler
  - Frontend-originated signals (onboarding completed) go through one
    tauri command `p3a_record(question_id: String)` that hard-validates
    the id against the enum and accepts **no value argument** — the
    webview can only tick predefined counters, never send content.

### Desktop: transport

- Event producers persist the local counter increment, then send a non-blocking
  wake-up through a bounded coalescing channel. One process-owned reporter is
  the only delivery drain: it reads the durable cursor, sends increments
  serially, persists `p3a_counters.reported_value` after every accepted report,
  and retries failures with bounded exponential backoff. A startup wake-up
  resumes any pending cursor after restart, and transient repository-open
  failures retry without terminating the reporter. Producers never wait for
  backlog delivery, and concurrent events coalesce into the active or next
  drain. Consent transitions invalidate in-flight attempts: network I/O never
  holds the transition gate, and a stale result cannot advance the cursor after
  opt-out, including across a later re-enable.
  Reports still include an ISO week so OS Accounts can aggregate by reporting
  period, but no precise event timestamp is sent.
- Transport uses `clovy_api.rs`'s authenticated JSON helper. This protects the
  public Clovy API route from unauthenticated writes while keeping user identity
  out of the telemetry report and out of the OS Accounts aggregate write.
- Wire format:

```json
POST {CLOVY_API_URL}/v1/p3a/reports
{ "schema": 1, "questionId": "dictation.sessions", "bucket": 0,
  "platform": "macos", "versionSeries": "0.0.x", "epoch": "2026-W28" }
```

- Kill switches: HTTP 410 per question → mark retired locally; global
  `p3a.enabled=false` served from Clovy API config → client stops sending
  entirely (checked once per epoch).

### Clovy API: ingestion (mirrors the issue-reports pipeline end to end)

Per house style (seven-crate split, `ApiResponse<T>` envelope, figment
config, no breaking `/v1/*` changes — additive only):

- `crates/domain`: `P3aReport` and `P3aSink`. The domain report contains
  only product slug, question id, epoch, platform, version series, and bucket.
- `crates/api/src/handlers/p3a.rs`: `POST /v1/p3a/reports`, wired in
  `crates/api/src/lib.rs` `router()`. It verifies the user token with
  `authenticated_user`, drops the identity, rejects unknown question ids or
  out-of-range buckets with 422, and returns the standard `ApiResponse<T>`
  envelope.
- `crates/services`: `P3aReportService` owns the Clovy-side question catalog
  and validation.
- `crates/providers`: `OsAccountsP3aSink` forwards to OS Accounts with
  `CLOVY__OS_ACCOUNTS__P3A_INGEST_TOKEN`. The sink must not forward an OS
  Accounts user token. Local dev uses `LogP3aSink`.

### OS Accounts: storage + dashboards

- Migration: `p3a_aggregates (question_id TEXT, epoch TEXT, platform TEXT,
  version_series TEXT, bucket SMALLINT, count BIGINT, PRIMARY KEY (...))`
  with snake_case + CHECK constraints per house style.
- Ingest endpoint (service credential auth, Clovy API is the only caller):
  `INSERT ... ON CONFLICT ... SET count = count + 1`. The request is the
  entire retention of the raw report. This endpoint must not resolve,
  create, or join an OS Accounts user, wallet, balance, or subscription.
- Read API / Grafana datasource applies `HAVING count >= 50` (published
  views) and a nightly prune of epochs older than 12 months.

**Exit criteria:** end-to-end rstest + wiremock integration tests (house
style: real Postgres for OS Accounts, wiremock for Clovy API's sink);
manual verification that a consenting debug build produces one aggregate
increment per recorded event and a non-consenting build produces zero.

## Phase 2 — remote question catalog

Question definitions graduate from compiled-in constants to a served
catalog, following the `/v1/models` pattern (server definitions override
local defaults at boot):

- `GET /v1/p3a/questions` on Clovy API, defined in `config.toml`, figment-typed.
- Client fetches at startup, intersects with its compiled enum: the server
  can **retire or re-bucket** questions without a desktop release, but can
  never introduce a question the shipped binary doesn't know — new
  questions still require an app update and a catalog-doc PR. This keeps
  "the code you can read is the ceiling of what's collected" true, which
  matters more than remote-add convenience. (Also required practically:
  the updater endpoint is immutable per build, ADR 0001.)

## Phase 3 — publish aggregates

Quarterly public roll-up (PRD success metric): a small generator in
OS Accounts exporting suppressed aggregates to a public JSON/markdown
artifact. No new collection.

## Phase 4 (deferred) — cryptographic aggregation

Adopt STAR/Constellation-style threshold encryption (or randomized
response for any future sensitive boolean) once volume sustains k >= 50
per cell organically. Tracked as a design doc, explicitly out of scope
now; the wire format's `schema` field exists so this can version cleanly.

## Test plan summary

| Layer | Test |
|---|---|
| Bucketization / epoch math | pure-fn unit tests, `src-tauri` |
| Consent gating | wiremock: zero requests when disabled; counters wiped on disable |
| Content firewall | type-level (struct has no string content fields) + serde reject tests for extra fields |
| Catalog/doc parity | CI test: enum ⇄ `telemetry-questions.md` |
| Clovy API handler | HTTP boundary test: 200 happy path; 401 missing user auth; 422 unknown question or bad bucket; body-limit |
| Sink | wiremock OS Accounts: forward shape, drop-on-failure, no retry storm |
| OS Accounts | real-Postgres repo tests: upsert increment, prune job, k-suppression view |
| Release gate | debug-build manual run: observe requests with mitmproxy; confirm user auth appears only on the Desktop to Clovy API hop and is not forwarded to OS Accounts telemetry storage |

## Sequencing and estimate

| Phase | Scope | Estimate |
|---|---|---|
| 0 | consent flag + UI + catalog + docs | ~3-4 days |
| 1 | client pipeline + Clovy API endpoint + OS Accounts aggregates + dashboards | ~1.5-2 weeks |
| 2 | remote catalog | ~2-3 days |
| 3 | public roll-up | ~2 days, quarterly thereafter |

Phases 0 and 1 can land as separate PRs per the repo's additive-`/v1/*`
rule; nothing here blocks or is blocked by other in-flight work.

## Review checklist for every future P3A change

- [ ] New question has a PRD-linked decision it informs
- [ ] Buckets are the coarsest that still answer the question
- [ ] `telemetry-questions.md` updated in the same PR (CI enforces)
- [ ] No new wire fields; if unavoidable, schema version bumped + PRD amended
- [ ] Grafana cell suppression still >= 50 for the new dimension
