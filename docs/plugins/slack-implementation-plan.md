# Implementation plan: Slack plugin

> **Stale integration point.** This plan predates the runtime migration:
> "Hermes" (and any Hermes bridge commands such as
> `import_hermes_bridge_file`) refer to the removed embedded Hermes runtime.
> Agent-facing MCP registration is now Clovy-owned per
> [ADR-0038](../adr/0038-june-owned-openai-agents-runtime.md) and
> [ADR-0039](../adr/0039-june-owned-routines-and-mcp.md); read "Hermes" below
> as "the Clovy agent runtime" and route MCP registration through the ADR-0039
> mechanism. The MCP-server integration shape itself is superseded: Clovy-owned
> capabilities are built as in-loop host tools per
> [ADR-0040](../adr/0040-plugin-capabilities-as-host-tools.md).

- **Mode:** CTO
- **Date:** 2026-07-13
- **Status:** Proposed; auth spike required
- **PRD:** [slack-prd.md](slack-prd.md)

## Technical objective

Build Slack on the provider-neutral connector kit, keeping token custody and
provider calls on-device if Slack's public distribution model permits it.
Enforce channel selection and action approval in Rust.

## Phase 0: release-blocking auth spike

Slack's OAuth v2 authorization-code exchange uses a client secret. A shipped
desktop app cannot protect a reusable confidential secret. Before coding the
connector:

1. Confirm with Slack's current distribution requirements whether a public app
   can use a desktop-safe PKCE flow or another supported non-confidential flow.
2. Evaluate Socket Mode and OAuth separately; Socket Mode avoids a public
   Events API URL but does not by itself solve installation secret custody.
3. Reject embedding the secret, sending Slack tokens through a hosted intermediary, or
   using a shared static user token.
4. If no local-safe public install exists, write an ADR for the chosen boundary
   before implementation. Options within the local-only boundary are a
   bring-your-own Slack app for technical users or deferring the connector.

Exit: one supported, reviewable auth design and verified disconnect/revoke
path. No downstream estimate is a commitment until this passes.

## Proposed connector surface

Subject to the spike, add:

| Server | Tools |
| --- | --- |
| `june_slack` | `list_channels`, `search_messages`, `read_thread`, `list_mentions`, `get_message` |
| `june_slack_actions` | `post_message`, `reply_to_thread` |

All read tools require an account-bound channel allowlist checked in Rust after
provider resolution. Search results return message id, channel id, author,
timestamp, compact text, and canonical link. Full threads require an explicit
call. Bots and edited/deleted messages are represented, not silently flattened.

## Local state

- Keychain token material if the auth design allows local custody.
- Non-secret workspace/account index and selected channel ids in SQLite.
- Per-channel polling cursor and backoff state.
- Existing pending action journal for posts/replies, including stable client
  message id where Slack accepts it.
- No message corpus, channel transcript, or file body cache.

## Events

V1 local mode uses one of two on-device approaches selected by the spike:

- Socket Mode while Clovy is running, if public distribution and token custody
  are compatible; or
- bounded polling of mentions and selected conversations with persisted cursors.

HTTP Events API webhooks require a public endpoint and therefore belong to an
away-mode design, not a quiet exception in local mode.

## Trust policy

- `read_only`: `june_slack` only.
- `approval`: action server visible; every post/reply parks.
- `autonomous`: deferred. The first release does not permit unattended posts.
- Channel allowlist applies in every mode and cannot be widened by tool input.
- Cross-workspace and Slack Connect channel identity is explicit in approvals.
- Automatic retry is enabled only where Slack honors the stable client message
  id as an idempotency key. Otherwise an ambiguous timeout is reconciled
  against recent channel history by action fingerprint, or replay is blocked
  until the user confirms; the local journal alone is insufficient.

## Delivery slices after Phase 0

1. **Account and channel grant (1 week):** connection state, channel picker,
   allowlist enforcement, revoke and health checks.
2. **Read path (2 weeks):** channel list, search, message/thread reads, links,
   pagination, rate-limit handling.
3. **Approved actions (1 week):** draft/post/reply through the approval journal.
4. **Local events (1 week):** mentions/selected-channel wakeups while awake.
5. **Skills and rollout (1 week):** recap and standup templates, rc dogfood,
   support and kill switch.

## Verification

- Auth matrix: first install, workspace-admin denial, token rotation, restart,
  revoke at Slack, disconnect in Clovy, and reauthorization.
- Rust tests that provider-resolved channel ids cannot escape the allowlist.
- Pagination, rate-limit, deleted-message, edited-message, thread, bot, and
  Slack Connect fixtures.
- Prompt-injection corpus in message text, usernames, link unfurls, attachments,
  and channel topics.
- Action-journal tests for timeout, retry, duplicate acknowledgement, and app
  restart.
- Live workspace walkthrough with public/private channels and denied scopes.
- Sandbox proof for Keychain material if local custody ships.

## Rollout and operations

Use an internal Slack workspace, then a small external pilot with explicit
admin consent, then rc. Collect coarse operation, rate-limit, latency, approval,
and error buckets only. A provider kill switch disables calls and event intake
without deleting the user's local notes.

## Architecture decision gate

The local-mode shape proposed by ADR-0016 is the starting hypothesis, not an
accepted decision while that ADR remains proposed. Accept or supersede it
before implementation depends on the boundary. Any backend credential or event
path independently satisfies the repo's ADR threshold and must be recorded.
