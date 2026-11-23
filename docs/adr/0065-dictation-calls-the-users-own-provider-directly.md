---
status: accepted
date: 2026-10-17
supersedes: none (narrows the BYOK forwarding rule in AGENTS.md Boundaries)
---

# Dictation calls the user's own provider directly

## Context

Dictation was slow next to a lightweight dictation app using the same Groq
Whisper model (openspec change `ditado-rapido`). Measured in the `-dev`
profile's `dictation-events.log`: 2.9 to 5.3 s from release to paste for 14 to
25 s of audio, against 0.4 to 1.7 s in the other app. Most of the gap was
Clovy's own path, not the model: cleanup on an agent CLI for every take, a
start cue that held the microphone, and every call taking an extra hop through
Clovy API even when the user had saved their own Groq or OpenRouter key.

The Boundaries rule said user BYOK keys "are forwarded through Clovy API only
for their matching eligible provider route". With a BYOK key Clovy API only
relays: the user's provider account pays, no Clovy credits move, and the
request it sends upstream is fixed (Groq transcription parameters, the
dictation cleanup prompt at temperature 0). The hop still costs a TLS
connection to Clovy API, an OS Accounts token on the request, and Clovy API's
own upstream call.

The owner decided on 2026-10-17 that dictation should be as quick as pasting
Whisper's transcript, with cleanup optional and, when on, a single OpenRouter
chat completion.

Alternatives considered:

- **Keep the hop and only warm the connection to Clovy API.** Rejected: it
  keeps a second network leg and the token dependency on every take, and the
  other app shows the direct call is enough.
- **Let dictation call the provider with a service-managed key.** Rejected:
  service keys never leave Clovy API; that rule stays as it is.

## Decision

1. **A user's own key may go straight from the desktop to its own provider
   for dictation.** With a saved Groq key, dictation transcription posts to
   `api.groq.com/openai/v1/audio/transcriptions` with the parameters Clovy API
   uses for `/v1/dictate`. With a saved OpenRouter key, dictation cleanup posts
   one chat completion to `openrouter.ai/api/v1/chat/completions` with the
   dictation cleanup prompt and `temperature: 0`. Without the key, the call
   goes through Clovy API exactly as before (`/v1/dictate`,
   `/v1/dictate/cleanup`).
2. **A direct request carries only the called provider's key.** No Clovy API
   token, no Clovy app-version headers, and no `x-*-api-key` forwarding
   headers; tests with fake Groq, OpenRouter, and Clovy API hosts check it.
3. **Service-managed keys still live only in Clovy API.** Nothing in this ADR
   puts a Clovy-owned key in the desktop.
4. **The cleanup prompt is a desktop copy of Clovy API's**
   (`src-tauri/src/prompts/dictate_cleanup.md`), kept equal by a test, so both
   routes send the same contract. Clovy API's v1 routes and contracts do not
   change (ADR-0021).
5. **The scope is dictation.** Note transcription, note generation, chat,
   media, and web keep their Clovy API routes with BYOK forwarding.

## Consequences

- Dictation with a saved Groq key works without reaching Clovy API for the
  transcription itself. Sign-in is still required to start a take (the
  existing dictation auth gate is unchanged).
- Clovy API no longer sees BYOK dictation requests, so its logs and metrics
  undercount dictation for those users; there was no charge to record.
- Groq and OpenRouter errors reach the desktop directly. They map to the same
  `<provider>_api_key_rejected` / `_usage_limit_exceeded` / `_model_unavailable`
  codes and messages Clovy API uses, so the UI does not change.
- A prompt change in Clovy API must be copied to the desktop; the equality
  test fails until it is.
- Extending the direct route to another feature needs its own decision.

## Addendum (2026-10-17): verbose transcription on the direct route

The direct Groq request differs from Clovy API's in one parameter: it asks
for `response_format=verbose_json` and drops segments whose `no_speech_prob`
is 0.6 or more, because Whisper invents text over near-silence and the
helper's speech classifier no longer runs before upload. Takes under 0.5 s
are not uploaded at all. The Clovy API route keeps `json` (v1 contract). See
the `ditado-rapido` design, D5.

## Addendum (2026-10-27): notes take the direct route too

§5 is narrowed by [ADR-0070](0070-notes-call-the-users-own-provider-directly.md):
note transcription, note generation (not Auto) and note transcript cleanup
also go straight to the user's own Groq or OpenRouter key. Chat, media and
web keep their Clovy API routes.


## Addendum - 2026-10-28: direct providers only

The missing-key server fallback and shared-server prompt source are superseded for this fork by
[ADR-0073](0073-desktop-calls-user-providers-without-a-clovy-server.md). The desktop calls the user's providers directly; chat stays
on CLI engines and Clovy tools remain local through MCP. The server is removed,
not replaced or deployed with a breaking contract. Earlier decisions above
remain as historical records; current behavior follows ADR-0073 and ADR-0072.
