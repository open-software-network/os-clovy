# Configuration reference

Clovy has two independent env surfaces: the **desktop app / client** (repo-root
`.env`, copied from `.env.example`) and **Clovy API**
(`clovy-api/.env` + `clovy-api/config.toml`). The client env must never hold a
server secret — upstream provider keys and the OS Accounts App API key live only
in Clovy API.

The fully-commented sources of truth are `.env.example`,
`clovy-api/.env.example`, and `clovy-api/config.toml`. This page summarizes them;
when they disagree, the files win.

## How Clovy API config resolves (Figment)

`AppConfig::default()` ← `config.toml` (ships in the Docker image) ← legacy
`JUNE__SECTION__FIELD` env vars ← canonical `CLOVY__SECTION__FIELD` env vars (a
double underscore separates sections; single underscores in field names stay
literal, so `APP_API_KEY` → `app_api_key`).
Secrets are env-only and redacted in logs. Pricing layers once more:
built-in fallback ← `config.toml` ← the **live Venice catalog at boot**.

## Desktop app / client (repo-root `.env`)

| Var | Purpose | Default |
|-----|---------|---------|
| `CLOVY_API_URL` | Clovy API base URL the app calls | `https://june-api.opensoftware.co` in code; `.env` (from `.env.example`) sets `http://127.0.0.1:8080` for local dev |
| `OS_CLOVY_LOCAL_DEV` | Use a local bearer token instead of Login with Open Software | `1` (example) |
| `OS_CLOVY_LOCAL_DEV_BEARER_TOKEN` / `_USER_ID` | The local-mode identity | `local-dev-token` / `usr_local_dev` |
| `OS_ACCOUNTS_URL` / `OS_ACCOUNTS_API_URL` | OS Accounts portal + API (optional in local mode) | unset |
| `OS_ACCOUNTS_CLIENT_ID` | OAuth client id (`ocl_...`) sent on `/login` | unset |
| `OS_ACCOUNTS_LOOPBACK_PORT` | Login redirect loopback port; must match the registered `http://127.0.0.1:<port>/callback` | `8765` |
| `OS_CLOVY_DEV_PLAINTEXT_TOKEN_STORE` | Debug builds: store tokens in a file, not the Keychain (avoids prompts) | `0` |
| `GOOGLE_OAUTH_CLIENT_ID` / `_SECRET` | Google Desktop OAuth credential for Gmail/Calendar connectors. Google requires both at token exchange; the second value is not confidential in an installed app and grants no user-data access by itself | unset |
| `VENICE_TRANSCRIPTION_MODEL` / `OPENAI_TRANSCRIPTION_MODEL` | Initial transcription model (Settings can override) | see `.env.example` |
| `VENICE_GENERATION_MODEL` | Initial note-generation model | `zai-org-glm-5-2` |
| `VENICE_TITLE_SUGGESTION_MODEL` | Fast model for note-title suggestions | fallback to cleanup model |
| `OS_NOTETAKER_TRANSCRIPTION_LANGUAGE` | Optional ISO-639-1 language hint | unset |

Dev-only toggles also read in code: `OS_CLOVY_ENABLE_DEV_SINGLE_INSTANCE`,
`OS_CLOVY_USE_PROD_ACCOUNTS_TOKENS`, `OS_CLOVY_USE_PROD_DATA_DIR`.

When set to `1`, `true`, `yes`, or `on`, `OS_CLOVY_USE_PROD_DATA_DIR` opts a
debug build into the production app data directory and the production
`provider-settings.json` location. Otherwise, app data and provider settings
use debug-only paths with the `-dev` suffix. Other files read directly from the
raw Tauri app config directory are unaffected.

June-era `OS_JUNE_*` variables remain lower-precedence compatibility inputs
through the bridge release.

## Clovy API (`clovy-api/.env`, `CLOVY__…`)

**Secrets — env only, never in `config.toml` or the client `.env`:**

| Var | Purpose |
|-----|---------|
| `CLOVY__UPSTREAMS__VENICE__API_KEY` | Venice key (default text / dictation / private transcription) |
| `CLOVY__UPSTREAMS__OPENAI__API_KEY` | OpenAI key (optional; OpenAI ASR only) |
| `CLOVY__OS_ACCOUNTS__APP_API_KEY` | The `osk_` App API key Clovy API uses to authorize/charge |
| `CLOVY__ISSUE_REPORTS__OS_PLATFORM_API_KEY` | Bot key to file issue reports as os-platform Issues (else log-only) |
| `CLOVY__COMPANION__DATABASE_URL` | Postgres trust-metadata store; required to enable the production companion relay |
| `CLOVY__COMPANION__APNS_PRIVATE_KEY_PEM` | Apple APNs `.p8` key; literal newlines or `\\n` escapes are accepted |

Non-secret (usually left to `config.toml`): `CLOVY__SERVER__HOST` / `PORT`,
`CLOVY__OS_ACCOUNTS__API_URL`, `CLOVY__LOCAL_DEV__ENABLED` / `BEARER_TOKEN` /
`USER_ID`, `CLOVY__UPSTREAMS__*__BASE_URL`.

Companion APNs configuration also needs
`CLOVY__COMPANION__APNS_TEAM_ID`, `CLOVY__COMPANION__APNS_KEY_ID`,
`CLOVY__COMPANION__APNS_BUNDLE_ID`, and
`CLOVY__COMPANION__APNS_PRODUCTION`. If any APNs value is missing, relay and
foreground reconnect still work but background wake hints are disabled.

June-era `JUNE__*` variables remain lower-precedence compatibility inputs.

## Clovy API settings (`clovy-api/config.toml`)

- **Server:** `request_timeout_secs` 600, `max_audio_bytes` 25 MiB, `max_json_bytes` 512 KiB, `max_agent_chat_bytes` 12 MiB (dedicated `/v1/chat/completions` cap, aligned with the desktop client boundary and sized for a 1M-token context window; must be ≥ the 12 MiB client cap), `max_issue_report_bytes` 301 MiB total (one 300 MiB os-platform attachment plus multipart overhead), `max_image_edit_bytes` sized for a 50 MiB source image after base64 expansion.
- **Metering estimate:** `flat_estimate_credits` 250 — the flat credit Hold per metered action; skips per-request estimation.
- **Hold TTLs (secs):** `note_transcribe` 60, `note_generate` 300, `dictate_transcribe` 30, `dictate_cleanup` 30, `web` 30, `image` defaults to `request_timeout_secs` + 30 (630) — validation rejects an image TTL that cannot outlive the request timeout, so a slow generation can still settle its charge.
- **Web tools:** `web_search_credits` 20, `web_fetch_credits` 20 (flat).
- **Preview cap:** `note_transcribe_preview_max_audio_secs` 30.
- **OS Accounts token contract:** `iss` `https://accounts.opensoftware.co`, `aud` `open-software-apps`, `jwks_refresh_secs` 300, `jwks_miss_min_backoff_secs` 5.
- **Pricing:** one `[pricing."<model_id>"]` table per priced model (unit, credits, provider, model_type, capabilities, ...). A model with no pricing entry is rejected at the boundary; the live Venice catalog extends this at boot (see [ADR-0007](adr/0007-model-capability-source-of-truth.md)).
- **Attestation / issue reports:** the TEE trust-center URL + the fixed os-platform destination (`open-software` / `june`).

## Upstream retry limits

Provider calls allow at most two attempts and keep the existing 300 ms retry
allowance. For retryable HTTP failures, `Retry-After` is read as delta seconds
or an HTTP date. If the requested cooldown exceeds that allowance, Clovy API
returns the provider failure without retrying early. Missing or malformed
headers use the existing 300 ms backoff. Longer waits require coordinating the
wallet hold lifetime and settlement budget; the desktop request timeout alone
is not a safe retry budget.

For service-managed traffic routed through Open Software API, its separate
`SERVICE_REQUESTS_PER_MINUTE` budget controls aggregate service demand. Changing
that gateway budget requires an Open Software API deployment; this retry policy
requires a Clovy API deployment.
