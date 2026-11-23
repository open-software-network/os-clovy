# PRD: Canva plugin

> **Historical server proposal (2026-10-28).** The hosted connector broker
> described below is superseded by
> [ADR-0073](../adr/0073-desktop-calls-user-providers-without-a-clovy-server.md).
> The server has been removed; this proposal is not authorization to add a
> replacement service or implement the old server path. No replacement design
> is introduced by this retirement notice.


- **Mode:** CEO
- **Overall rank:** 20 of 20
- **Score:** 56/100
- **Date:** 2026-07-14
- **Status:** Proposed; server credential and review gates

## Thesis

Canva can turn meeting outcomes into reviewed visual work: find a relevant
design, create from an approved brand template when supported, and export a
presentation or graphic. V1 should focus on design discovery and export, not
promise arbitrary visual editing through an incomplete API.

Canva closes the follow-on set because creative output has strong leverage but
lower ICP frequency, Connect API credentials are designed for secure server
storage, and webhook features are preview-only and unavailable to public
integrations.

## Customer and problem

Small teams leave planning meetings with decks, one-pagers, and social assets to
produce. Recreating content manually is slow, while unrestricted design
automation risks brand drift and accidental publication.

## Product promise

Connect Canva, choose folders/designs, and let Clovy prepare a bounded draft or
export job with the source note content and output format visible before action.

## V1 experience

- Connect one Canva account and choose folders/designs.
- Search/list selected design and folder metadata.
- Start a reviewed export of an existing design to a supported format.
- If public APIs and review permit it, create from an approved brand template
  using explicit autofill fields.
- Download the completed export to a user-chosen local destination.

## Scope

### V1

- Design/folder metadata and explicit design selection.
- Asynchronous export jobs with supported-format discovery.
- Local download through Clovy's artifact broker and native save approval.
- Template autofill only after Phase 0 confirms general availability and review.

### Later

- Comments, approvals, asset upload, folders, brand templates, bulk creation,
  multiple teams, and webhook-driven collaboration routines.

## Non-goals

- Arbitrary element-level editing or replacing the Canva editor.
- Silent publication, sharing, permission changes, deletion, or team admin.
- Claiming preview webhook/API capabilities can ship publicly.
- Storing Canva credentials in the desktop binary.

## Privacy and trust

Canva requires Connect API integrations to be associated with a web app and
explicitly does not support desktop integrations. It also requires secure
server custody for the client secret and user tokens. Clovy cannot use the
Google-style local connector model. The architecture spike must either approve
a provider-supported TEE web connector, with explicit token custody and Canva
content/action transit through Clovy API, or defer the integration. Selected
designs/folders remain a narrower broker-enforced boundary after provider
authorization.

Design metadata, comments, template fields, and imported text are untrusted.
Every export/create action is approval-only and displays the exact Clovy content
sent to Canva.

## Business model

Canva is Pro because server exchange, API review, format support, and creative
quality validation create ongoing cost. Model and image-generation work uses
existing billing.

## Success measures

| Metric | Target |
| --- | ---: |
| Connected users selecting a first design/folder | 80% |
| Started export jobs completing successfully | 95% |
| Exports needing format or destination correction | under 5% |
| Actions outside selected designs/folders | 0 successful |
| Public release depending on preview APIs | 0 |

## Risks and gates

- Provider-supported web-app credential and user-token custody requires a new
  approved trust boundary and Clovy API data path.
- Public integration review rejects preview webhook use.
- Export support varies by design type and format.
- Autofill and brand-template availability may constrain the creation promise.

## Decision requested

Approve Canva research with export as the narrow v1. Do not schedule public
implementation until credential custody and API review are resolved.

## Sources

- [Canva Connect API security](https://www.canva.dev/docs/connect/guidelines/security/)
- [Canva export API](https://www.canva.dev/docs/connect/api-reference/exports/)
- [Canva webhooks](https://www.canva.dev/docs/connect/webhooks/)
- [Canva Connect APIs](https://www.canva.dev/docs/connect/)
