# Implementation plan: Notion plugin

> **Historical implementation plan.** This plan predates the runtime migration:
> "Hermes" (and any Hermes bridge commands such as
> `import_hermes_bridge_file`) refer to the removed embedded Hermes runtime.
> Those registration references below are historical, not current instructions.
> CLI turns now reach the connector through the Clovy MCP gateway and local
> approval broker, as defined by [ADR-0063](../adr/0063-cli-turns-get-write-tools-through-an-approval-broker.md)
> and [ADR-0072](../adr/0072-cli-chat-replaces-the-native-agent.md).

- **Mode:** CTO
- **Date:** 2026-07-13
- **Status:** Proposed; hosted MCP connector preview in implementation, selected-resource scoping intentionally out of scope for this pass
- **PRD:** [notion-prd.md](notion-prd.md)
- **Issue:** JUN-283

## Technical objective

Ship a Notion hosted MCP connector preview. Clicking **Connect** opens Notion's
hosted MCP OAuth flow in the default browser, stores the returned OAuth material
in the OS Keychain, and lets the user disconnect locally. When connected, Clovy
registers the read-only `june_notion` MCP server/toolset with Hermes so Clovy can
discover the hosted MCP read tools. This slice also registers the narrow
`june_notion_actions` MCP server/toolset for approved page creation through
Notion's hosted MCP `notion-create-pages` tool and approved page updates through
`notion-update-page`. This slice intentionally does not test or enforce
selected-resource scoping and does not expose move, duplicate, comment,
attachment, database, data-source, or view action tools.

The broader PRD still targets selected Notion content through read and action
MCP servers, with the authorized page boundary proven at the provider or
enforced in Rust and all writes parked for approval. That content path remains
blocked until one of the selected-resource boundaries below is proven.

## Current Phase 0 evidence

The prior handoff reports that a hosted MCP prototype proved the live transport
chain:

- Clovy Rust shell -> Keychain token -> rmcp Streamable HTTP -> Notion hosted MCP
  -> `tools/list`.
- `notion_mcp_oauth_status` returned connected state with presence indicators
  for access token, refresh token, client id, and Keychain-only custody.
- `notion_mcp_oauth_list_tools` reached `https://mcp.notion.com/mcp` and listed
  20 hosted tools, including read tools such as `notion-search`, `notion-fetch`,
  `notion-query-data-sources`, and `notion-query-database-view`, plus write-class
  tools such as page create/update.

This reported result supports transport/auth viability only. It does not prove
Clovy can safely claim that Notion access is limited to user-selected pages. The
hosted `notion-search` surface is also provider-defined and may include sources
connected to the user's Notion workspace when that workspace enables them, so the
preview copy must disclose that Clovy is not exposing a page-only search index.
The reported transport result remains provisional until the prototype source or
complete JSON evidence with secrets and private identifiers/content redacted is
recovered.

## Phase 0A: recover prototype evidence

**Checkpoint, 2026-07-16:** the prototype source is unavailable in this clone.
The referenced commits (`e3f22675`, `6670efc`, and `9425221`) are absent from
local objects, local and remote refs, reflogs, and unreachable commits. The
referenced `spike__notion-adr-0024-prototype` worktree currently resolves to
`origin/main`, and no Notion hosted-MCP source is present here. Recover the
original checkout, branch, patch, or complete JSON
evidence with secrets and private identifiers/content redacted before adding or
promoting transport code.

Before promoting prototype code:

1. Locate or recover the branch/worktree that contains the prototype commits and
   ADR referenced by the handoff. Do not reimplement blindly if the prototype
   diff exists elsewhere.
2. Capture full JSON from the status, report, and tool-list debug commands.
3. Redact tokens, user/workspace identifiers, page titles, and private Notion
   content before committing or sharing evidence.
4. Preserve tool schema fingerprints, tool classifications, byte caps, and
   endpoint metadata so later implementation reviews can compare drift.

## Phase 0B: selected-resource promotion and exit gate

Selected-resource scoping is the gate for promoting or exiting the shipped
preview, not for disabling Connect. The prototype must add or reuse
a temporary, debug-only, read-only `tools/call` probe command with a strict
allowlist:

- `notion-search`
- `notion-fetch`
- `notion-query-data-sources`
- `notion-query-database-view`
- `notion-get-comments`

Deny all write/action hosted tools in the scoping probe command, including
create, update, move, duplicate, comment, and attachment tools. Keep the command
out of production UX. The production preview may separately expose approved page
creation and page updates through `june_notion_actions`; that action path is not
evidence for the selected-resource read boundary.

Test with a non-sensitive Notion workspace matrix that uses unique canary
titles and body snippets for selected and unselected resources. Record returned
ids, parent/ancestor metadata, cursors, and result counts after redaction.

| Probe | Expected if narrow scoping works |
| --- | --- |
| Search for selected Page A title and canary body term | Found |
| Fetch selected Page A by id | Succeeds |
| Search for child page under Page A | Document expected provider semantics, then verify |
| Fetch child page under Page A by id | Same as above |
| Search for unselected Page B title and canary body term | Not found, including across pagination |
| Fetch unselected Page B by id | 403, 404, or not accessible |
| Query selected Database A | Succeeds if authorized |
| Query unselected Database B | 403, 404, or not accessible |
| Broad search terms | No unselected titles, snippets, ids, or private content leak |

Exit criteria:

- **Pass:** hosted MCP enforces selected-resource access for search, fetch, and
  data-source reads. Clovy may keep the selected-page privacy claim and promote
  the hosted transport into production code.
- **Fail:** hosted MCP behaves like broad workspace access. Keep Connect with
  explicit unverified-scope disclosure, but do not promote it until Clovy adds a
  Rust-enforced authorized-root graph that filters or rejects every
  search/query/fetch response before Hermes sees any unselected id, title,
  snippet, metadata, or content, or chooses a different selected-resource access
  model. ID blocking alone is not sufficient because search and query responses
  can leak metadata before a later fetch. Proceeding with broad workspace access
  requires an explicit product decision and PRD, threat-model, success-metric,
  non-goal, and UI-copy updates before implementation.
- **Inconclusive:** keep the connector's unverified scope explicit and do not
  promote it.

## Phase 0C: lifecycle and privacy verification

After scoping probes, verify:

- Quit and relaunch Clovy, then status and tool listing work from Keychain without
  browser reauth.
- Reconnect does not create duplicate dynamic-client state.
- Disconnect removes local Keychain material, updates runtime capability, and
  reports disconnected state.
- Provider-side revoke is offered and verified separately when supported; revoke
  invalidates the Notion grant server-side in addition to local disconnect.
- Offline and provider failure paths return typed errors, never panics.
- Token refresh is serialized or coalesced if refresh can be forced.
- Logs include only operation names/classes, endpoint host, tool names, status
  or error class, byte counts, and schema fingerprints.
- Logs never include access tokens, refresh tokens, dynamic client material, raw
  tool arguments, full result payloads, page bodies, or private titles.

Exit with a supported auth design, revoke flow, Marketplace/review requirements,
selected-resource proof, and an honest privacy claim.

## Proposed servers

| Server | Tools |
| --- | --- |
| `june_notion` | `notion-search`, `notion-fetch`, `notion-query-data-sources`, `notion-query-database-view`, `notion-get-comments`, `notion-get-enhanced-markdown-specification`, `notion-get-view-configuration-dsl` |
| `june_notion_actions` | `notion-create-pages`, `notion-update-page` |

These are the exact third-party canonical names currently returned by Notion's
hosted MCP preview. Clovy preserves them through discovery, policy checks, Rust
preflight, and provider invocation. CLI turns reach the connector through the
Clovy MCP gateway and local approval broker (ADR-0072). Historical model-facing
aliases from the retired native loop are not the current transport contract;
the gateway binds each invocation to the discovered canonical operation.
Future hosted tools are not part of this contract until they are separately
reviewed and added here.

Tool results preserve page/block/data-source ids, parent ids, canonical URLs,
last-edited time, property schema, pagination cursor, and a compact content
representation. Full block trees are bounded by depth, count, and bytes.

## Authorization boundary

The target boundary is selected-resource access, but the enforcement mechanism
is contingent on Phase 0B. Notion's hosted OAuth/page-picker flow may not return
a trustworthy list of authorized roots, and provider search/query tools may
return metadata for resources outside the user's selection. Production read tools
therefore need one of these proven boundaries before the preview is promoted:

1. The hosted MCP provider enforces selected-resource access for search, fetch,
   comments, and data-source queries.
2. Rust obtains and maintains an independent authorized-root graph, then filters
   or rejects every hosted result before Hermes sees any unselected id, title,
   snippet, metadata, or content.

Rust stores only non-secret root/workspace/account metadata. It treats provider
403/404 as permission outcomes, but never infers access from a cached path alone.
Writes additionally require an approved parent within the current accessible
graph.

## State and events

- Keychain token if the Phase 0 design preserves device custody.
- Workspace/account id, bot id, capabilities, and health in SQLite; authorized
  roots only when Phase 0B establishes a trustworthy source or independent graph.
- No page bodies or database row corpus at rest.
- V1 freshness is live fetch plus optional bounded polling of user-followed
  pages. Provider webhooks require public HTTPS and belong to away mode.

## Write model

- Create is preferred over broad in-place transformation.
- Updates operate on explicit page/block ids and include last-edited/version
  material where available.
- Approval shows workspace, destination breadcrumb, operation, property diff,
  and rendered content preview.
- A preflight re-reads destination state before commit. Stale state returns a
  conflict instead of overwriting.
- Autonomous mode is deferred.

## Delivery slices after Phase 0

0. **Hosted MCP connector preview (current slice):** add a Notion row to
   Connectors with privacy-accurate copy. The primary Connect action opens the
   hosted MCP OAuth flow, stores credentials only in the Notion Keychain
   service, reports local connection state, supports local disconnect, registers
   the read-only `june_notion` MCP server/toolset with Hermes, and registers
   `june_notion_actions` for approved page creation and approved page updates.
   It does not run selected-resource scoping probes or expose move, duplicate,
   comment, attachment, database, data-source, or view action tools.
1. **Connector page shell:** keep the Notion row visible and label the connected
   state as preview/unverified until selected-resource scoping is proven. The
   subtitle must not promise selected-page-only access.
2. **Threat model update:** extend or supersede
   [private-connectors-threat-model.md](../private-connectors-threat-model.md)
   before enabling production connect. Cover Notion token custody, direct Notion
   traffic, hosted MCP behavior, inference as a separate path, logging, local
   disconnect, and provider-side revocation.
3. **Connection shell (1 week):** workspace state, trustworthy authorized roots
   when available, revoke, health, plugin detail, and connect/reconnect/disconnect
   state transitions.
4. **Read path (2 weeks):** search, page/block read, data-source query, comments,
   limits and pagination.
5. **Approved create and update (shipped in preview):** create one page with
   exact-parent approval and update one explicit page target with bounded,
   destructive-effect disclosure.
6. **Targeted update (1-2 weeks):** properties and bounded block operations with
   conflict preflight.
7. **Skills and rc (1 week):** decision/project templates, metrics, runbook.

## Connector page behavior

The Plugins surface is the discovery/detail entry point for the Notion plugin.
The Connectors page is the single source of truth for account connection state,
so plugin connect controls should route here rather than duplicating token or
status handling. The Connectors page lists Notion alongside Google from the first
implementation slice so users and reviewers can see the intended account path.

- **Not connected, preview:** show Notion as available to connect through the
  hosted MCP preview. The subtitle must not promise selected-page-only access.
  Clicking connect starts the Rust OAuth/hosted-MCP flow; React never receives
  token material.
- **Not connected, verified:** after a future selected-resource gate passes,
  copy may graduate from preview language to the proven privacy claim.
- **Connecting:** show a waiting state while the browser flow is active.
- **Connected preview:** show that Notion is connected locally for hosted MCP
  auth, with unverified-access copy. The copy must also disclose that hosted
  Notion search may include Notion-connected sources when the user's workspace
  enables them. Apply the runtime so Hermes can discover the read-only
  `june_notion` MCP server/toolset and the approved create/update
  `june_notion_actions` MCP server/toolset. Do
  not show workspace/account content or a selected-resource privacy claim in
  this slice.
- **Connected verified:** show workspace/account metadata and the exact
  selected-resource privacy claim proven by Phase 0B. Under the current content
  PRD, this state is not reachable for broad workspace access.
- **Reconnect needed:** offer reconnect using the same Rust-owned credential
  custody path.
- **Disconnect:** remove local token custody, apply the runtime update, and
  remove Notion MCP capability from Hermes. Offer provider-side revocation as a
  separate option when Notion supports it.

Prototype commands such as `notion_mcp_oauth_status`, hosted tool inventory,
phase reports, and read-only scoping probes stay hidden from production UI unless
converted into bounded production diagnostics.

## Verification

- Auth, reconnect, revoke, removed-page-access, workspace removal, and partial
  capability matrix.
- Block-tree property tests for depth, pagination, unsupported types, mentions,
  embeds, equations, and files.
- Boundary tests that forged page/parent ids cannot escape authorized access.
- Conflict/idempotency tests for create and update around retries and restarts.
- Injection corpus in page title, rich text, comments, URLs, code blocks,
  database properties, and embedded content.
- Live workspace walkthrough with private/shared pages and schema changes.

## Rollout

Internal workspace, selected external workspaces, rc, stable. Use a provider
kill switch and content-free telemetry. Publish supported block/property types
and render unsupported content explicitly instead of dropping it.

## ADR threshold

Any backend-held secret or token, public webhook intake, or provider content
relay is a new trust boundary and requires an ADR before shipping.
