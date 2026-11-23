# Configuration reference

The desktop `.env` (copied from `.env.example`) holds build and connector
configuration. Provider keys are saved through Settings > Models > More
options > Provider API keys, in the operating system credential store. There
is no Clovy inference server, server config file, service-key fallback, or
local bearer token. See
[ADR-0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md).

## Desktop configuration

| Variable | Purpose | Default |
|---|---|---|
| `OS_CLOVY_DEV_PLAINTEXT_TOKEN_STORE` | Debug connector-token storage for isolated fixtures | `0` |
| `GOOGLE_OAUTH_CLIENT_ID` / `_SECRET` | Google Desktop OAuth client credentials; user grants remain in the credential store | unset |
| `OS_NOTETAKER_TRANSCRIPTION_LANGUAGE` | Optional ISO-639-1 transcription language hint | unset |
| `CLOVY_DEV_APP_IDENTIFIER` | Run-owned compiled debug identity, bundle and data directories | default development identity |

Debug/release paths must use `app_paths::app_data_dir` and
`app_paths::app_config_dir`. Never enable `OS_CLOVY_USE_PROD_DATA_DIR` in tests
or verification. See [development.md](development.md) for isolated native proofs
and the signed dev launcher. The current `.env.example` and code are the
source of truth for supported variables.

On macOS, Settings > General > Hide from Dock controls the local
`app-preferences.json` preference `hideFromDock`, off by default. When enabled,
the app switches to accessory activation only while the main window is hidden;
it restores regular activation before revealing the window. The menu-bar app
continues running. Other platforms do not expose this setting.

## Provider keys and routes

| Capability | Provider and credential |
|---|---|
| Dictation, saved-note transcription, live preview | Groq, with the user's Groq key |
| Remote note generation and transcript cleanup | OpenRouter, with the user's OpenRouter key |
| Optional dictation cleanup | OpenRouter only, with the user's OpenRouter key |
| Image generation/editing and video generation | fal, with the user's fal key |
| Chat and routine turns | Installed CLI and its own login |
| CLI/endpoint note generation | Selected engine through `llm::generate` |

The desktop checks saved provider keys against the provider's own key-check
endpoint. Keys are held by `llm::secrets` under `builtin:<provider>`;
`provider-settings.json` and DTOs contain configured flags, never keys.
A missing Groq/OpenRouter/fal key fails before a request. Unreadable saved
credentials report a credential-store error. There is no switch to another
provider or shared account. The CLI supplies its own web search; the desktop
has no separate web-search service.

Provider-key changes hold settings/published-view locks across the secret
operation and persistence. Candidate settings are written through a same-directory
temporary file and atomic replace. Only success publishes the new flags. A
persistence failure restores the previous secret and cache; a failed restore
reports `provider_api_key_rollback_failed` and asks the user to save the old key
again. These guarantees also apply when removing a key.

## Text catalog and engine selection

The text-model picker reads `https://openrouter.ai/api/v1/models`, optionally
with the saved OpenRouter key. It includes every model whose output modalities
contain text, including models without tools. Mapping preserves ID, name,
description, context length, input vision support, supported parameters, and
USD token prices. No unknown privacy tier is invented.

`providers/catalog.rs` uses a 10-second total deadline, including cache-lock
wait, an 8 MiB response cap, and a 10-minute memory cache. Saving/removing the
key invalidates that cache and reloads the picker. Failed loads show an error
and retry action. Catalog access is discovery, not permission to run inference.

Changing the selected text model updates only the remote generation model.
It does not change the Notes engine or chat engine. The picker description
names the current Notes engine. Chat is CLI-only, Claude Code by default with
the existing Codex fallback; notes support OpenRouter, CLI, or an explicit
OpenAI-compatible endpoint. Auto and the cost/quality control are removed.
Transcription, image and video pickers use local curated lists, even without
keys; attempting the operation still requires its provider key.

A one-time persisted migration changes old server-based note selections to
OpenRouter and the old Auto model to the generation default. CLI/endpoint
note choices, endpoint keys and saved history are preserved. Old native or
endpoint chat selections become the CLI default (ADR-0072). SQL migrations
remain append-only.

## fal media

`providers/fal/` owns the queue client and operation state. An image request
has a two-minute deadline; a video job has a 15-minute deadline that is not
reset by polls. Submit uses the full allowed model endpoint; status/result
requests use its app alias and a validated provider request ID. The caller's
`requestId` identifies a replay of the same operation shape. Replays and
recoverable polls never automatically submit a second paid job.

Each video poll reads the current fal key. Missing/rejected/inaccessible keys
leave the job available for `generate_video` with `jobId` after the user saves
a key for the same account. Expired, evicted or restart-lost jobs produce
`video_job_unavailable`; the app does not recreate them. Completed video
artifacts use a stable local filename for the opaque job UUID.

Queue calls carry only the fal key. Media downloads carry no credentials,
disable redirects, require an allowed HTTPS fal CDN host, reject private
addresses, and pin validated DNS results. Image output is bounded to 50 MiB
and video output to 100 MiB, including chunked responses. Inline image base64
has both encoded and exact decoded-size limits. MIME comes from magic bytes.

Image editing preserves oriented source proportions. The request is at most
1024x1024 pixels, with each side between 512 and 2048. A source that needs no
resize/orientation/padding is sent unchanged. Larger sources are scaled as a
whole frame, never cropped or stretched; extreme ratios receive neutral
padding that is removed after the result is validated. Decoding retains the
512 MiB allocation bound and runs off the async executor; cancellation does
not interrupt a decode already executing.

The result must match the requested geometry. For `fal-ai/flux-2/edit` only,
the previously observed exception accepts both dimensions rounded up to the
next multiple of 32 and scales the whole frame back before removing padding.
Any other output geometry is rejected. Pure local word screening informs
safe mode; no model-based image classifier makes a second request.

## Transcription context

Groq receives a short spelling hint in `prompt`: whole custom dictionary
terms, in order, joined by `, ` within 200 UTF-8 bytes. A term that does not fit
is skipped without truncation. The hint contains no instructions, style or
previous transcript. The full dictionary and earlier same-source turns go to
note transcript cleanup, using the selected Notes engine and the prompt in
`src-tauri/src/prompts/note_transcript_cleanup.md`.

Dictation cleanup is separate: optional, off by default, and always OpenRouter.
When it cannot run, dictation delivers the raw transcript with an explicit HUD
notice. See [dictation.md](dictation.md) and [llm-providers.md](llm-providers.md).
