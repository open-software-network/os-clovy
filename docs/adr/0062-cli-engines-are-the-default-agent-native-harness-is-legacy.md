---
status: accepted
date: 2026-10-15
supersedes: ADR-0038 (in part, as the default engine), ADR-0039 (in part, the engine routines run on)
superseded_by: ADR-0064 (in part, Home)
---

# CLI engines are the default agent; the native harness is legacy

## Context

[ADR-0038](0038-june-owned-openai-agents-runtime.md) made the Clovy-owned agent
harness (the `agent-runtime/` sidecar on the OpenAI Agents SDK, with remote
models through Clovy API) the engine of every agent surface, and
[ADR-0039](0039-june-owned-routines-and-mcp.md) kept routines on it. Since
then the CLI chat engine (`crate::chat_engine`, see
[llm-providers.md](../llm-providers.md#cli-chat-engine)) runs a chat session on
an agent CLI installed on the Mac, with Clovy's own tools through the Clovy MCP
server ([ADR-0058](0058-clovy-mcp-server-relays-to-the-running-app.md)).

The owner decided: everything that runs on the native agent defaults to Claude
Code or Codex. The native harness is to be removed later, once the tools only
it has reach the MCP server. Until then it must keep working for the sessions
and features that still need it.

## Decision

1. **The default chat engine is a CLI.** Claude Code
   (`__clovy_cli_engine__:claude`), else Codex when Claude Code is not on the
   login-shell PATH. With neither installed, chat stays on the Clovy agent and
   Settings says why.
2. **One recorded move of the chat setting.** At launch, and before the default
   engine is read (`chat_default_engine`), chat on the Clovy agent (the old
   default, new installs included) moves once to that CLI
   (`llm::registry::apply_cli_default`), recorded in
   `provider-settings.json` as `llmCliDefaultApplied`. Any other chat choice
   (an endpoint, another CLI) is kept and recorded. A later explicit chat
   choice, Clovy included, is recorded and never moved again. With no default
   CLI installed nothing is recorded, so the next launch retries.
3. **Every new-work entry point starts on the default engine** through the
   existing `chat_engine` turn (`start_cli_turn`), never a second CLI path:
   new desktop sessions, Home task handoffs, note chat, and sessions started
   from the companion. Home's lightweight triage (`june_home_chat`) stays on
   Clovy API; only the task it hands off moves.
4. **Routines run unattended on the default CLI.** A routine whose model is
   Auto runs on the chat engine when it is Claude Code or Codex
   (`providers::default_unattended_cli`), through the same `chat_engine` turn
   with `TurnPolicy::Unattended`: claude gets only `Read`, `Grep`, `Glob`,
   `WebSearch`, `WebFetch` (`--tools`), pre-allowed with Clovy's MCP tools
   (`--allowedTools ... mcp__clovy__*`), and no other MCP server
   (`--strict-mcp-config`); codex runs with `sandbox_mode="read-only"` and
   `approval_policy="never"`. No other CLI runs a routine. Writing and shell
   commands in routines wait for an approval broker.
5. **The Clovy agent stays listed, last, as legacy.** Engine pickers list the
   CLIs first with the default marked, then endpoints, then Clovy. Existing
   native sessions keep their engine, and companion approval of Computer use
   still applies only to them.

## Consequences

- An unattended turn is the one place Clovy pre-grants CLI tools. This narrows
  the chat engine's "never pre-grant" rule to interactive turns; the
  unattended set is read-only by construction (`--tools` removes every other
  built-in, and the codex sandbox refuses writes and network).
- Features only the native harness has (Computer use, the browser plugin,
  skills, connectors with approvals, image and video tools, compaction) are not
  available on a CLI session until they are offered over MCP.
- Home triage still needs Clovy API and credits; a handoff then runs on the
  user's CLI subscription.
- A routine with an explicit model, or on a machine where chat is not on
  Claude Code or Codex, still runs on the native harness with its toolsets.

## Addendum (2026-10-17): superseded in part by ADR-0064

Home's triage on Clovy API (decision 3) and the consequence "Home triage still
needs Clovy API and credits" no longer hold: Home runs entirely on the default
CLI engine, in its hidden session
([ADR-0064](0064-home-runs-on-the-default-cli-engine.md)).


## Addendum - 2026-10-28: direct providers only

The remaining server/native fallback portions are superseded for this fork by
[ADR-0073](0073-desktop-calls-user-providers-without-a-clovy-server.md). The desktop calls the user's providers directly; chat stays
on CLI engines and Clovy tools remain local through MCP. The server is removed,
not replaced or deployed with a breaking contract. Earlier decisions above
remain as historical records; current behavior follows ADR-0073 and ADR-0072.
