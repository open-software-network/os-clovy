---
status: accepted
date: 2026-10-17
supersedes: ADR-0062 (in part: Home's triage in decision 3 and the "Home triage still needs Clovy API" consequence)
---

# Home runs on the default CLI engine

## Context

[ADR-0062](0062-cli-engines-are-the-default-agent-native-harness-is-legacy.md)
moved every new-work entry point to the default CLI engine but kept Home's
lightweight triage on Clovy API: each Home message went to
`POST /v1/chat/completions` (`june_home_chat`), which either replied in Home or
called a synthetic `start_task` tool that opened a focused session on the CLI.
With no OpenRouter key on Clovy API, Home failed with `home_chat_failed`
(`openrouter_not_configured`) while a new session on Claude Code worked. The
owner decided on 2026-10-16 that Home runs entirely on Claude Code, without
Clovy API (openspec change `inicio-no-claude-code`).

Alternatives considered:

- **Keep the triage and swap its model for a one-shot CLI call** (`claude -p`
  with no tools, returning a `reply`/`task` JSON). Rejected: a second CLI path
  next to `start_cli_turn`, a fragile JSON contract, and the Home history still
  duplicated in local storage.
- **Drop the old Home history.** Rejected: it loses user data for no gain.

## Decision

1. **Home is an ordinary CLI session, hidden from the session list.** A Home
   message is `start_agent_run` on the Home session with the default CLI engine
   (`chat_default_engine`: Claude Code, else Codex) in `sandboxed` mode, through
   the same `start_cli_run` → `start_cli_turn` path as any CLI session. The
   conversation is the session's items, with the same projection, approvals,
   and errors. With no CLI installed, Home still asks Claude Code so the turn
   fails with the CLI sessions' missing-engine error.
2. **An older Home session adopts the engine with its next turn.**
   `start_agent_run` stores the turn's model on the session, so a Home session
   created on `open-software/auto` needs no migration. New Home sessions are
   created on the default engine.
3. **One-time import of the old Home turns.** On open, the turns older builds
   kept in `clovy:home:direct-turns:v1` are imported as items of the Home
   session (`import_legacy_home_turns`), in order and at their original time.
   The marker is the item's external id prefix (`home-import:<session id>:`) in
   the database, not local storage, so the import never repeats. Unreadable
   entries are skipped and counted. Local storage is never cleared (a rollback
   still finds it), and the task cards in `clovy:home:task-handoffs:v1` keep
   rendering, placed by the time of the stored turn that carried them.
4. **No Home persona, no fixed identity reply.** A Home turn is the same as any
   CLI turn. A persona for CLI turns, if wanted, applies to every CLI session.
5. **The triage is removed:** `clovy_home_chat`, `june_home_chat`, their prompt,
   streaming, `home_chat_*` codes, the current-information heuristic, the
   focused-session handoff, and the client code that only they used.
   `proxy_agent_chat_completions` stays for the image prompt safety check.

## Consequences

- Home no longer calls Clovy API and works without an OpenRouter key; it
  depends on Claude Code (or Codex) being installed and signed in.
- Home answers more slowly than the streamed triage: each turn starts the CLI.
- Home now does real work (files, shell) in `sandboxed` mode, with the same
  approvals as any CLI session. Nothing opens a separate session for it.
- Turns imported from local storage reach the CLI as prefixed history on the
  first turn, since there is no CLI conversation to resume; later turns resume.
- `POST /v1/chat/completions` keeps its other callers and stays (ADR-0021).
