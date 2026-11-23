---
status: accepted
date: 2026-10-28
supersedes: 0038, 0040, 0013 (recorder transport), 0039 (runtime integration), 0062 (legacy fallback)
---

# CLI chat replaces the native agent

## Context

Clovy's default chat already runs on installed CLI engines. Its local MCP
server now exposes the Clovy tools through a turn binding and approval broker.
Keeping a second model-and-tool loop duplicates process lifecycle, settings,
packaging, signing, policy, and tool integration. An endpoint presented as a
chat engine still depends on that second loop.

The owner authorized removing the native agent and endpoint-as-chat engine.
The 2026-10-28 orchestration decision permits implementation before the
reduced native smoke: a fresh isolated profile, synthetic data, and Claude
Code turns only. Copied-profile and paid Codex smoke are not prerequisites.
The smoke evidence and owner acceptance of the cut are recorded separately
in `openspec/changes/cli-substitui-agente-nativo/tasks.md`; this ADR does not
claim that those checks have run.

## Decision

1. Chat runs only through installed CLI engines. Claude Code is the default;
   Codex remains the existing local fallback. If the selected/default CLI is
   unavailable, report the setup requirement. Never substitute a bundled
   agent, an endpoint, or a remote chat route.
2. Clovy retains the `clovy-mcp` relay and its local tool implementations.
   A turn-bound connection gives a CLI the tools allowed by that turn's
   policy. Mutations require the local approval broker before their effects.
   Unbound external clients retain the read-only catalog. Cancellation and
   turn completion revoke authority; approving one request grants no general
   execution permission.
3. The neutral `agent_sessions` Rust module owns session records, runs,
   ordered items, events, historical imports, and persistence. `chat_engine`
   owns CLI processes. The native agent process, transport, SDK dependency,
   build/signing steps, and package targets are removed. Node remains frontend
   tooling, not a packaged agent executable.
4. Preserve history and SQL migrations. Existing sessions, messages,
   artifacts, and interruptions remain readable. A subsequent send or retry
   from an old native/endpoint session uses the CLI default. A one-time
   migration cancels native runs left waiting for a user. Historical event
   names and database columns may remain for compatibility; they do not
   authorize execution of the retired process.
5. Normalize old Clovy/endpoint chat settings to the CLI default. Text-model
   selection affects note generation, never the chat engine. Keep CLI and
   endpoint note routes and the `llm::generate` interface. Skills stay and are
   read through MCP; loading instructions does not grant their requested tools.
6. Keep tool-specific local policy, secret custody, connector grants, parser
   isolation, and recording UI coordination. Recording start/stop continues
   through the frontend's visible recording flow, now reached from the
   turn-bound recorder tools rather than a dedicated legacy proxy.

## Earlier decisions

- Supersedes ADR-0038's bundled harness and ADR-0040's host-tools-only
  integration. Clovy-owned capabilities are now reached through its MCP
  relay; Rust remains the authority for their effects.
- Supersedes the transport decision in
  [ADR-0013, agent recorder](0013-agent-recorder-tool-via-frontend-event.md).
  Its visible-recorder and frontend-acknowledgement requirements remain.
  The separately numbered [ADR-0013, inference streaming](0013-stream-inference-responses-through-june-api.md)
  no longer applies to desktop chat. This decision does not change the
  remaining note-generation API contract.
- Supersedes the runtime integration in ADR-0039. Durable routine/MCP
  configuration, credential custody, migration, and tool policy remain;
  routines use their existing unattended CLI path.
- Reanchors ADR-0053's one-shot approval principle in the local approval
  broker and its session/run/tool binding. The fork has no companion approval
  transport. This does not restore remote control or extend approval authority.
- Supersedes ADR-0062's legacy fallback. ADR-0063 describes the turn binding
  and broker; ADR-0058 describes the relay. Their acceptance evidence is
  tracked by the OpenSpec change.
- ADR-0037's immutable migration history and ADR-0068's app identity remain
  binding. The retired executable entries in ADR-0055 are historical only.

## Consequences

The desktop ships one chat execution path and no JavaScript agent payload.
Users need an installed, connected CLI for chat. Their CLI account controls
chat inference; selecting an OpenRouter note model cannot redirect it.

Tool availability now depends on the CLI's MCP support, the local server,
and the turn policy. Native live steering and manual compaction do not survive
as simulated features. Historical data compatibility is kept separately from
execution compatibility.

The alternative of keeping the native engine as a fallback was rejected
because it would preserve the dependency and duplicate authority the owner
explicitly chose to remove. Clovy API removal and direct provider routing are
tracked by the separate `remover-clovy-api` change, not claimed by this ADR.

## Verification

The removal requires CLI routing/history tests, session persistence tests,
MCP and broker tests, frontend picker tests, the full repository gates, and a
bundle inspection with no native agent executable. Native proof and paid
provider inference are distinct: only observed actions may be marked proven.
