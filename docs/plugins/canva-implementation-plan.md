# Implementation plan: Canva plugin

> **Historical server proposal (2026-10-28).** The hosted connector broker
> described below is superseded by
> [ADR-0073](../adr/0073-desktop-calls-user-providers-without-a-clovy-server.md).
> The server has been removed; this proposal is not authorization to add a
> replacement service or implement the old server path. No replacement design
> is introduced by this retirement notice.


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
- **Date:** 2026-07-14
- **Status:** Proposed; credential and API review gates
- **PRD:** [canva-prd.md](canva-prd.md)

## Technical objective

Evaluate and, only after an accepted ADR, expose selected Canva designs through
a provider-supported TEE web connector with approval-gated export jobs. Add
template autofill only when generally available, publicly reviewable, and
bounded by an explicit field schema.

## Phase 0: credential and capability matrix

1. Test Connect API web-app authorization, refresh, revoke, integration review,
   team selection, redirect constraints, and credential requirements.
2. Reject desktop token custody and an exchange-only workaround. Evaluate a
   provider-supported Clovy API web connector that stores the client secret and
   user tokens sealed inside the TEE and proxies Canva calls.
3. Test design/folder scope availability, export formats by design type,
   asynchronous job expiry, download URL lifetime, and rate limits.
4. Mark every preview endpoint as unavailable to public v1, including preview
   webhooks that cannot pass review.
5. Test brand-template/autofill availability and schema constraints separately
   from export.

Exit with an ADR-approved server-side credential, token, provider-call, and
content-lifetime design or deferral, plus a generally-available endpoint/scope
matrix. The ADR must state that Canva is not local mode and describe what
OpenSoftware infrastructure can observe.

## Proposed servers

| Server | Tools |
| --- | --- |
| `june_canva` | `list_designs`, `get_design`, `list_folders`, `get_export_formats`, `get_export_status` |
| `june_canva_actions` | `create_export`, `create_from_brand_template` (gated) |

No tool exposes arbitrary element editing. Preview-only tools are omitted from
the runtime schema, not merely hidden in UI.

Unlike local connector MCP servers, these servers call authenticated Clovy API
routes. Clovy API verifies the OS Accounts identity and selection boundary, then
uses the sealed Canva token inside the TEE for the provider call.

## Boundary and state

- Client secret and per-user Canva tokens sealed inside the Clovy API TEE; no
  Canva token is returned to the desktop.
- Desktop SQLite stores non-secret Canva user/team, selected design/folder ids,
  capability/version matrix, export jobs, and health.
- Clovy API stores the minimum account binding and enforced selection needed to
  authorize Canva calls. It retains no design body, asset corpus, or completed
  export by default.
- The Clovy API broker verifies selected design/folder identity on every call.
  The desktop separately validates download metadata before native save.

## Action and artifact model

- Export approval shows design, format/options, page selection, destination,
  estimated disclosure, and local save behavior.
- Poll async jobs with bounded backoff and terminal-state handling.
- Stream the export from Canva through the Clovy API TEE to a task-scoped local
  file, inspect type/size, then use native save approval. Do not persist export
  bytes in backend storage; delete temporary local data after completion/cancel.
- Template autofill validates exact provider field keys and type/length bounds;
  approval shows every Clovy-originated value sent.
- Job creation is not blindly retried after timeout. Reconcile against recent
  jobs or require confirmation.

## Events

Public v1 has no Canva webhook dependency. Current webhook support is preview
and public integrations using it cannot pass review. Collaboration triggers
remain future away-mode work after general availability and threat review.

## Delivery slices after Phase 0

1. Auth exchange, team/design selection, revoke, and health.
2. Metadata reads and export-format discovery.
3. Approved export job, polling, download, and native save.
4. Optional brand-template spike and gated autofill action.
5. Creative-quality fixtures, pilot, metrics, and kill switch.

## Verification

- Auth, refresh, revoke, team removal, design access removal, scope denial,
  review-mode restrictions, and disconnect.
- Forged ids/hosts, moved designs, unsupported format, multi-page limits,
  expired downloads, failed/cancelled jobs, and rate limits.
- Duplicate export, timeout, polling interruption, malicious MIME/name, and
  restart tests.
- Injection corpus in design metadata, comments, template fields, and links.
- Live walkthrough across Docs, Presentation, and one unsupported format.

## Architecture decision gate

A Canva integration meets the repo's ADR threshold because provider support
requires a web app, backend-held user tokens, and provider content/actions to
transit Clovy API. No implementation begins before that full boundary is
accepted, documented in the connector threat model, and reflected in consent
copy. If that boundary is unacceptable, Canva remains deferred.
