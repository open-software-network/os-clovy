---
status: accepted
date: 2026-10-05
---

# Activity belongs to the installation, not to a data partition

## Context

The **data partition** (the `profile` column in code) separates notes,
projects, agent sessions, memories, and their assignments
([CONTEXT.md](../../CONTEXT.md)). Activity capture is a different kind of
data: it records what happens on the Mac (apps, windows, sites, captured text,
and coding-agent blocks) in `activity.sqlite3`
([ADR-0057](0057-activity-database-uses-sqlcipher-linked-app-wide.md)), with
one set of activity settings per installation. Its schema has no partition
column; the "Today" view and the in-app agent's `search_activity` and
`get_activity_timeline` read all of it, whichever partition is open.

The Clovy MCP server ([ADR-0058](0058-clovy-mcp-server-relays-to-the-running-app.md))
answers notes, dictations, and memories in the current data partition. A
review asked for the activity tools to follow the same boundary, so activity
recorded while one partition was open would not show from another.

Alternatives considered:

- **Tag captured activity with the partition open at capture time** and filter
  by it. It needs a migration of frames, sessions, gaps, and coding-agent
  blocks, touches capture, the timeline ETL, and coding-agent ingestion, and
  would make the MCP server disagree with the "Today" view and the in-app
  agent unless they changed too. The capture happens on the Mac regardless of
  which data set the user has open in Clovy's window, so the tag would record
  window state, not the work's owner.
- **Offer activity tools only in the `default` partition.** Small, but
  arbitrary: the same activity would still be visible there, wherever it was
  captured.

## Decision

Activity is per installation. It is not partitioned, and the MCP activity
tools (including `list_coding_agent_sessions`) read the installation's
activity like the "Today" view and the in-app agent, applying exclusions,
retention, and the coding-agent source switches. Notes, dictations, and
memories stay limited to the current data partition. The MCP guide resource,
the server instructions, and the Settings copy say so.

## Consequences

- Switching data partitions does not change what activity tools return.
- If activity ever needs partitioning, it is a capture-level change for every
  reader at once (a superseding ADR), not an MCP filter.
- The `clovy-mcp-server` spec's "no perfil ativo" covers the data tools; the
  activity tools are scoped by the installation.
