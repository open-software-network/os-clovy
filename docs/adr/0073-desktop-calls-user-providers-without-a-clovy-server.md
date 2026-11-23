---
status: accepted
date: 2026-10-28
supersedes: 0021, 0008 (server transport), 0013 (inference streaming), 0015 (server transport), 0062 (server fallback), 0065 (server fallback), 0070 (server fallback)
---

# Desktop calls user providers without a Clovy server

## Context

The local Clovy API still relayed provider requests after chat moved to CLI
engines (ADR-0072). Packaged desktop installs did not include that server;
missing keys could therefore turn into failed loopback requests. Keeping its
workspace also retained sharing, billing vocabulary, model-based auxiliary
calls, backend deployment rules, and two versions of provider behavior.

The owner accepted the cut on 2026-10-28:

> "no chat, é só pra usar o claude code, não iremos nunca usar a api clovy...
> nada de fora... tudo nosso, única coisa que mantém são as tools do agente
> clovy... chamar api deles externa NUNCA, você pode deletar completamente essa
> dependência, pra tudo que estiver usando"

The owner-authorized implementation retains direct Groq, OpenRouter, and fal
operations with the user's keys and existing explicit endpoint/CLI note routes.
It introduces no new service. The reduced smoke and its limitations are
recorded in the two OpenSpec changes; acceptance does not claim paid calls,
Codex smoke, copied-profile testing, or final native proof succeeded.

## Decision

1. Remove the server workspace, desktop server URL, local bearer token, proxy
   routes, server workflows/build targets and backend deployment requirement.
   The app never contacts a Clovy inference server. There is no replacement
   service, compatibility shim or automatic provider fallback.
2. Groq handles dictation, saved-note transcription and live preview directly.
   OpenRouter handles remote note generation, note transcript cleanup and
   optional dictation cleanup directly. Only the corresponding user key is
   sent. A missing/unreadable key fails before network access; live preview
   stays off without a Groq key. Dictation cleanup failure preserves the raw
   transcript and reports that cleanup was skipped.
3. Chat remains CLI-only: Claude Code is the default and Codex the existing
   local fallback. Clovy MCP tools, local approval, session persistence and
   artifacts remain. Routines require an installed supported CLI. Notes retain
   their CLI/endpoint routes and `llm::generate` API.
4. The text picker uses OpenRouter's models catalog, filtered only for text
   output. Tool support is not required. It maps capabilities/context/prices,
   uses a bounded request and ten-minute cache, invalidates on key changes,
   and exposes failure/retry. Selecting a text model changes only the remote
   model, never chat or the Notes engine; the description names the Notes
   engine. Transcription and media use curated local lists.
5. Migrate the old server note selection once to OpenRouter and the old Auto
   model to the explicit default. Remove Auto and cost/quality controls.
   Preserve explicit CLI/endpoint note choices and history. SQL migrations
   are never deleted (ADR-0037).
6. Port fal queue rules into `src-tauri/src/providers/fal/` before deleting the
   server. Preserve allowed models, request IDs and replay deduplication,
   bounded timeouts/downloads, credential isolation, and image-edit geometry.
   Queue calls carry only the fal key; downloads carry none, disable redirects,
   and validate CDN origins/resolved addresses. A recoverable video key error
   keeps the same job for a later poll; no automatic resubmission occurs.
   Local word screening remains; the model-based image classifier is removed.
7. Remove sharing and its keys/UI, About verification, model-generated titles,
   credit/funding messages, server-availability gating, and desktop web proxy
   calls. The CLI supplies web tools. Do not replace removed services.
8. Keep prompts inside the desktop source tree. A scan in `make verify`
   rejects retired server identifiers/routes while permitting explicit direct
   provider URLs. Local fake-provider integration and native proof must record
   zero requests to a retired-origin listener. Paid inference needs explicit
   owner authorization; unexecuted scenarios are declared unproven.

## Earlier decisions

- ADR-0021's additive-only server contract no longer governs this fork: the
  owner authorized deleting the server rather than deploying a breaking
  replacement. This is not a commitment to serve older external clients.
- ADR-0008 and ADR-0015 retain their local tool/artifact and safety principles;
  server transport, funding, and pricing decisions are superseded here.
- The inference-streaming ADR-0013 is superseded; the separate recorder
  ADR-0013 retains the UI coordination clarified by ADR-0072.
- ADR-0062's remaining server/native fallback ends. ADR-0072 continues to own
  the CLI session and native-sidecar retirement decision.
- ADR-0065 and ADR-0070 retain direct provider payload/key boundaries; their
  missing-key server fallback, server-owned prompt source, and Auto exception
  are superseded.

## Consequences

Users must configure the provider required by each requested capability.
There is no shared funding or service availability to conceal a missing key.
Sharing stops being a product capability. Provider privacy and pricing are
those of the selected provider/account, not guarantees made by Clovy.

The desktop now owns queue recovery and image preparation previously handled
by the server. Fake-provider tests must cover those rules, while live paid
media and native audio behavior require separately authorized evidence. The
removed server cannot silently be restored by a later refactor: the static
gate and direct-provider contracts make that boundary reviewable.

## Migration history archive

The two retired server SQL migrations (shares and companion devices) are
preserved byte-for-byte in
[the historical migration archive](../history/clovy-api-migrations/README.md).
They are documentation records, not desktop migrations or an app dependency,
and the app never executes them. Desktop SQLite migrations remain in
`src-tauri/migrations/`, unchanged and append-only under ADR-0037.
