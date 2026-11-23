# Implementation plan: Linear hosted MCP

**Status:** Implemented; full verification and live development-app QA pending

**Date:** 2026-07-28

**Scope:** Linear only

**Supersedes:** The agent-facing GraphQL tool and selected-team portions of
[linear-implementation-plan.md](linear-implementation-plan.md). The existing
Linear OAuth, local token custody, refresh, identity, and disconnect behavior
remain the connection foundation.

## Decision

Clovy will connect a Linear account and expose the complete tool inventory from
Linear's official hosted MCP server. Clovy will not maintain a separate
Linear-specific agent tool contract or selected-team grant.

- The fixed provider endpoint is `https://mcp.linear.app/mcp`.
- The transport is MCP Streamable HTTP. The deprecated `/sse` endpoint is not
  used.
- Connection requests workspace-wide `read` and `write` scopes.
- The existing Linear OAuth access token is sent to the hosted MCP as a bearer
  token. Linear documents this as a supported alternative to a second
  interactive MCP OAuth flow.
- Every valid tool returned by `tools/list` is exposed to the Clovy agent under
  the `mcp_linear_<tool>` namespace.
- Tools that are explicitly safe read-only operations may run directly.
  Mutating, destructive, missing-annotation, and ambiguous tools require the
  existing Clovy approval interruption before invocation.
- Clovy owns account connect, token refresh and custody, disconnect, MCP
  transport safety, and generic approval policy. Linear owns the tool names,
  schemas, descriptions, and provider behavior.

This design deliberately accepts that Linear can add, remove, or change its
MCP tools independently of a Clovy release.

Official provider references:

- [Linear MCP server documentation](https://linear.app/docs/mcp)
- [Linear OAuth 2.0 documentation](https://linear.app/developers/oauth-2-0-authentication)
- [Streamable HTTP migration and MCP tool expansion](https://linear.app/changelog/2026-02-05-linear-mcp-for-product-management)

## Root cause addressed

The current Settings flow calls the retired Tauri command
`connectors_apply_runtime` after a Linear mutation. The Rust command was
removed with the Hermes runtime, but the frontend caller remained. A
disconnect can therefore commit successfully and then display
`Command connectors_apply_runtime not found`.

The Clovy-owned runtime now derives tool availability from current local state.
Linear connect, reconnect, and disconnect must not restart or apply a separate
runtime configuration.

## Product behavior

### Disconnected

Settings shows one **Connect Linear** action. No team picker is shown.
No Linear MCP tools are advertised.

### Connected

Settings shows the connected Linear workspace identity and one **Disconnect**
action. No team count, team editor, or "finish setup" state is shown.

The next agent run discovers the hosted MCP inventory and makes all valid
Linear tools available. New tools published by Linear become available after
the next successful discovery without a Clovy release.

### Approval

The runtime applies a generic MCP safety rule:

1. A tool may run without approval only when `readOnlyHint` is explicitly true
   and `destructiveHint` is either false or absent.
2. A tool with mutating or destructive annotations requires approval.
3. An absent or false `readOnlyHint`, or malformed, contradictory, or unknown
   annotations, require approval. An absent `destructiveHint` does not override
   an explicit `readOnlyHint: true`.
4. Clovy may apply generic mutation-name safeguards as an additional
   fail-closed check, but it must not maintain a Linear tool allowlist.

The approval shows the provider, remote tool name, and bounded arguments before
dispatch. Approval is per invocation, using the existing persisted
interruption protocol. The persisted run policy binds the normalized runtime
name to a fingerprint of the exact remote name, bounded description, input
schema, and raw approval annotations. Inventory drift fails closed and requires
a fresh turn and approval against the new tool contract.

## Architecture

```text
Settings
  -> Linear connect/disconnect commands
  -> connector account row + Keychain OAuth material
  -> managed Linear MCP adapter
  -> https://mcp.linear.app/mcp
  -> dynamic MCP tool descriptors
  -> Clovy agent runtime and generic approval policy
```

### Connection authority

`connector_accounts` and the existing Linear Keychain bundle remain the
source of truth. A second OAuth token or a user-editable
`agent_mcp_servers` row is not created.

The managed server identity is fixed as `builtin:linear`. It is synthesized
from a healthy connected Linear account and cannot be edited, renamed, or
deleted from the custom MCP settings surface.

The historical `linear_mcp_connection` prerelease table is not an authority
for connectivity. Its state must never override the connector account or
Keychain.

### Credential flow

The managed adapter requests a valid access token from the existing Linear
connector token helper immediately before MCP initialization or dispatch.
That helper keeps the current proactive refresh and atomic refresh-token
rotation behavior.

The bearer token is attached only by Rust to the fixed Linear MCP origin. It
is never stored in SQLite, included in a runtime descriptor, sent to
TypeScript, or logged.

An HTTP 401 before a tool has been dispatched may refresh the credential and
re-establish the MCP session once. A 401, timeout, transport failure, or
incomplete response after tool dispatch must not replay the call because the
tool may have mutated Linear.

### MCP transport

The managed adapter reuses the host-owned Streamable HTTP implementation
rather than implementing a second protocol client. The reusable boundary must
support:

- MCP initialize and initialized notification;
- `tools/list` discovery;
- `tools/call` dispatch;
- MCP session id management;
- JSON and event-stream responses;
- protocol-version headers;
- strict response and schema size limits;
- timeouts and cancellation;
- sanitized protocol errors; and
- explicit session invalidation.

The user-configured MCP registry remains unchanged. The managed Linear source
feeds the same normalized tool registry through a transient definition and a
connector-backed credential provider.

### Tool registration

At agent-run preparation:

1. Read current connector accounts.
2. If no healthy Linear account exists, register no Linear tools.
3. Resolve a valid access token.
4. Initialize or reuse the in-memory Linear MCP session.
5. Discover `tools/list`.
6. Validate and bound each tool name, description, input schema, and
   annotations.
7. Register every valid tool as `mcp_linear_<normalized-remote-name>`.
8. Apply the generic approval rule from the tool annotations.

An invalid tool is skipped with a content-free diagnostic. One invalid tool
must not remove the other valid Linear tools or fail the whole agent run.
Runtime-name collisions fail closed for the colliding tool.

Existing routines that already store the historical `june_linear` or
`june_linear_actions` toolset identity receive the corresponding directly
runnable or approval-required hosted tools during run preparation. Enabling
both identities exposes the complete hosted inventory. This is compatibility
for saved routine configuration, not a new Linear polling trigger, routine
template, or autonomous mutation policy.

Before every dispatch, Rust rechecks that the Linear account is still
connected and healthy. A descriptor captured before disconnect therefore
cannot continue using a removed credential.

### Session lifecycle

The MCP session is in memory only.

- Connect makes the account eligible immediately. A transient discovery
  failure does not undo a successful OAuth connection.
- Reconnect invalidates the old MCP session before the new account credential
  becomes eligible.
- Disconnect retires the MCP session before deleting local credentials.
- App restart performs a fresh MCP initialization and discovery.
- A dropped or invalid provider session may be reinitialized for a later
  request, but Clovy never automatically replays an already-dispatched tool
  call.

No lifecycle path calls `connectors_apply_runtime` or starts another app or
agent process.

## Disconnect semantics

Disconnect is locally authoritative:

1. Mark the account unavailable to new Linear MCP dispatch.
2. Retire the in-memory MCP session.
3. Remove local Keychain credentials and connector-account state.
4. Attempt provider-side token revocation after local removal.
5. Refresh Settings from the persisted account list.

If provider revocation cannot be confirmed, local disconnect still completes.
The UI reports a warning that the user may revoke the authorization in Linear
settings. It must not report that the account remains connected after local
credentials have been removed.

Repeated disconnect is idempotent.

## Selected-team retirement

Team selection is removed as a product and authorization boundary:

- Settings no longer opens or renders the Linear team picker.
- Connect completion no longer waits for a team selection.
- Agent eligibility no longer requires `selectedTeams`.
- MCP tool arguments and results are not filtered by saved team ids.
- Existing saved team rows remain dormant for the first release so rollback
  does not destroy user state.
- The existing team-list and team-save backend commands may remain
  temporarily unused. Removing their schema and stored rows is a separate
  cleanup after the hosted MCP release is stable.

UI copy must state that the connected Linear workspace is available to Clovy.
It must not promise selected-team isolation.

## Native Linear tool retirement

The hosted MCP inventory replaces the current Clovy-authored Linear agent
tools.

- Remove Linear eligibility from native connector descriptors.
- Remove Linear branches from shared issue tool dispatch without changing
  GitHub behavior.
- Stop registering the native Linear planning and mutation capabilities.
- Keep the OAuth, identity, refresh, revoke, and account persistence portions
  of the Linear connector.
- Provider-specific GraphQL operation code that becomes unreachable may be
  deleted after the runtime switch is covered by tests. Shared connector
  policy and action-journal schema used by other providers must remain.

There must never be simultaneous native and hosted-MCP versions of the same
Linear capability in one run.

## Migration compatibility

The current development database has prerelease migration identities
`linear_managed_mcp` and `linear_managed_mcp_repair` that originally occupied
versions 45 and 46. The calendar release now owns version 45, and the existing
repair moves those exact prerelease identities to versions 46 and 47.

The hosted MCP implementation must make the migration catalog converge with
that repaired state before another unrelated migration claims versions 46 or
47:

- promote `linear_managed_mcp` as migration 46;
- promote `linear_managed_mcp_repair` as migration 47;
- make both migrations idempotent for a clean database and for the
  prerelease `linear_mcp_connection` table shape;
- preserve existing connector accounts, Keychain material, and dormant team
  selections; and
- treat the prerelease table as compatibility data, not connection authority.

Migration tests must cover:

- a clean database through version 47;
- the exact prerelease stamps at old versions 45 and 46;
- the repaired prerelease stamps at versions 46 and 47;
- a current release database ending at version 45;
- repeated startup after repair; and
- rollback on an intentionally malformed near-match.

The installed production app and its database are never used for migration
testing. Automated tests use temporary databases. Live validation uses only
the development app already running from the right terminal pane.

## Error handling

### Connect

OAuth and local persistence errors fail the connection. Once OAuth material
and identity are stored, a transient MCP handshake or discovery failure is a
Linear health problem, not an authentication rollback. Settings remains
connected and may show a concise availability warning.

### Discovery

Discovery failure omits Linear tools from that run and emits a sanitized
diagnostic. It does not prevent healthy native or custom tools from loading.

### Invocation

Provider JSON-RPC errors are returned as bounded tool errors. Authentication
expiration requests reconnect when refresh is no longer possible. Transport
and ambiguous outcome failures are never retried after dispatch.

### Logging

Logs may include the fixed provider name, operation phase, HTTP status, MCP
error code, and bounded timing. They must not include tokens, authorization
headers, Linear content, tool arguments, tool results, or complete provider
error bodies.

## Implementation sequence

### 1. Record the architecture

- Add ADR-0050 describing Linear as a managed external MCP source behind the
  Clovy host runtime.
- State that this is a targeted exception to ADR-0039's current native Linear
  transport while preserving host-owned credentials and approvals.
- Update `docs/index.md`, the Linear PRD privacy language, and the stale
  integration note in the original Linear plan.

### 2. Lock the regressions

- Add a frontend test proving Linear disconnect succeeds even when the retired
  runtime command would be unavailable.
- Change Linear connect and reconnect tests to require no runtime apply.
- Add tests proving connect never opens the team picker and connected state
  has only disconnect.
- Preserve existing Google, GitHub, and Notion expectations in this
  Linear-only change.

### 3. Normalize hosted MCP metadata

- Extend discovered MCP tool data to retain bounded tool annotations.
- Add generic approval classification with fail-closed defaults.
- Cover malformed, missing, and contradictory annotations.
- Keep the custom MCP behavior backward-compatible.

### 4. Add the managed Linear MCP source

- Synthesize `builtin:linear` from the connected account.
- Bind it to the fixed hosted endpoint and connector-backed token provider.
- Reuse the Streamable HTTP session implementation.
- Register all valid discovered tools with the `mcp_linear_` namespace.
- Dispatch opaque arguments and results without Linear-specific translation.

### 5. Switch runtime registration

- Add managed Linear MCP descriptors during each agent-run preparation.
- Route managed MCP calls through the host-owned MCP dispatcher.
- Revalidate account state for every call.
- Remove Linear from native connector descriptor and dispatch eligibility.
- Verify GitHub's shared issue tools remain unchanged.

### 6. Simplify Settings lifecycle

- Remove Linear team-selection state, dialog, retry state, and copy.
- Skip `connectors_apply_runtime` for Linear connect, reconnect, and
  disconnect.
- Refresh account state directly after the persisted operation.
- Preserve accurate success and warning toasts when local disconnect and
  provider revocation have different outcomes.

### 7. Converge migrations

- Add catalog entries 46 and 47 with the prerelease identities.
- Update the existing repair tests for the now-current catalog.
- Verify clean, release, prerelease, repaired, ahead, and malformed histories.

### 8. Remove unreachable native behavior

- Delete Linear GraphQL agent operations and tests that no remaining
  connection-lifecycle code uses.
- Retain identity lookup, refresh, revoke, and their tests.
- Leave dormant selected-team persistence intact for rollback.

### 9. Verify and perform development-app QA

- Run focused frontend and Rust tests after each red-green slice.
- Run frontend formatting, lint, typecheck, and the relevant Rust format,
  Clippy, and test gates.
- Run the repository's full verification gate before completion.
- Let the existing right-pane development process rebuild the app. Do not
  start another app process.
- In that development app, verify connect, workspace-wide read discovery,
  one approval-gated write only with explicit user approval, disconnect, and
  absence of the retired-command error.

## Test matrix

### Frontend

- Linear connect succeeds without runtime apply.
- Linear reconnect succeeds without runtime apply.
- Linear disconnect refreshes to **Connect Linear** without a false error.
- Linear never requests team selection.
- Connected copy promises workspace access, not selected-team isolation.
- Other provider behavior is unchanged.

### Rust connector and MCP

- A healthy Linear account synthesizes one managed MCP source.
- No account synthesizes no source.
- Existing bearer credentials authenticate MCP without a second OAuth flow.
- Refresh rotation updates Keychain atomically.
- All valid discovered tools are registered.
- Invalid tools are skipped independently.
- Names are normalized and collision-safe.
- Read-only annotations bypass approval.
- Mutating and ambiguous annotations require approval.
- A disconnected account fails dispatch even with a stale descriptor.
- Session invalidation occurs on reconnect and disconnect.
- Post-dispatch failures are not replayed.
- Tokens and payload content are absent from captured logs.

### Migration

- Clean version 45 upgrades to 47.
- Old prerelease 45/46 repairs and converges.
- Already repaired 46/47 validates without replay.
- Repeated startup is idempotent.
- Near-match histories fail without partial mutation.

### Live development QA

- Use only the app already started by the right terminal pane.
- Do not open or modify the installed production app.
- Do not start a second Tauri or Vite process.
- Use only the user's own Linear workspace and approved test objects.
- Capture the exact tool inventory and annotations as sanitized contract
  evidence.

## Verification commands

The detailed implementation plan should select the narrowest command after
each task, then finish with the repository gates documented in `AGENTS.md`:

- focused Vitest suites for `ConnectorsSection`;
- focused Rust migration, connector, MCP, and runtime suites;
- `pnpm check`;
- `pnpm typecheck`;
- `cargo fmt --check` and applicable Clippy checks;
- `make verify`.

Node 26 frontend tests use
`NODE_OPTIONS=--no-experimental-webstorage` as documented by the repository.

## Deployment and rollout

This is a desktop-only change and requires no backend deployment.

Rollout should begin with the development app, then an internal build. The
release check must confirm:

- the official endpoint accepts Clovy's existing OAuth bearer token;
- `tools/list` returns usable annotations for the approval policy;
- at least one read tool succeeds across the workspace;
- an approved write has a clearly visible Linear result; and
- disconnect removes local access immediately.

If the official server rejects the existing bearer token in live validation,
stop the rollout and revise connect to use the hosted MCP's OAuth discovery.
Do not silently retain both credential systems.

## Out of scope

- Changes to Google, GitHub, or Notion.
- A Clovy-maintained Linear tool allowlist.
- Selected-team enforcement.
- Linear webhooks, polling routines, or away-mode relay.
- Provider-side administration or permission management.
- Adding a hosted backend.
- Starting another development app process.
- Any operation against the installed production app.

## Completion criteria

The change is complete when:

- Settings provides only Linear connect and disconnect;
- a connected account exposes the complete valid hosted MCP inventory;
- read-only tools can run directly and all other tools require approval;
- native Linear agent tools are not advertised;
- account state changes require no runtime apply or restart;
- migration histories converge through version 47;
- focused and full verification gates pass; and
- the existing right-pane development app completes the approved live QA
  without touching the installed production app.
