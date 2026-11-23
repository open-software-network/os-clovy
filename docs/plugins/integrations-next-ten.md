# Clovy integration portfolio: next ten

- **Owner:** CEO + CTO
- **Date:** 2026-07-14
- **Status:** Proposed follow-on to JUN-309
- **Related:** [Top-ten plugin portfolio](portfolio.md)

## Executive decision

After Clovy's first ten plugins, the next ten provider integrations should be:

| Overall rank | Integration | Score | Portfolio role | Primary gate |
| ---: | --- | ---: | --- | --- |
| 11 | HubSpot | 63 | Accessible CRM follow-through | Confidential OAuth exchange and broad portal scopes |
| 12 | Salesforce | 62 | Enterprise CRM system of record | Org administration, schema variance, and packaging |
| 13 | Asana | 61 | Cross-functional project execution | Confidential OAuth exchange |
| 14 | Box | 60 | Governed enterprise content | Auth feasibility and enterprise policy matrix |
| 15 | GitLab Issues | 60 | Alternative software delivery issue graph | Self-managed instance and version variance |
| 16 | ClickUp | 59 | All-in-one project execution | Confidential OAuth and long-lived token posture |
| 17 | Dropbox | 59 | Broad file access for smaller teams | Content bounds and app review |
| 18 | Pipedrive | 59 | Lightweight sales follow-through | Confidential OAuth exchange and webhook boundary |
| 19 | Azure Boards | 57 | Microsoft-centered engineering execution | Entra migration and tenant administration |
| 20 | Canva | 56 | Reviewed visual follow-through | Server-side secret custody and API review limits |

This is a research and sequencing decision, not approval to implement all ten
at once. GitHub, Linear, and Google remain existing Clovy workstreams and are
not rescored here. The original top ten remain ahead of this follow-on set.

## Method

The scores are the unchanged values from the reproducible scorecard in
[portfolio.md](portfolio.md). That scorecard weights core-loop fit (25), ICP
frequency (20), action leverage (15), retention and composition (15), privacy
differentiation (15), and delivery confidence (10). Ties retain the original
order after considering fit with Clovy's current customers and the amount of
new architecture required.

| Integration | Core /25 | Frequency /20 | Action /15 | Retention /15 | Privacy /15 | Delivery /10 | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| HubSpot | 19 | 10 | 13 | 11 | 8 | 2 | **63** |
| Salesforce | 19 | 9 | 13 | 11 | 8 | 2 | **62** |
| Asana | 17 | 10 | 11 | 10 | 8 | 5 | **61** |
| Box | 16 | 11 | 8 | 9 | 9 | 7 | **60** |
| GitLab Issues | 17 | 9 | 10 | 10 | 8 | 6 | **60** |
| ClickUp | 16 | 10 | 11 | 9 | 8 | 5 | **59** |
| Dropbox | 15 | 11 | 8 | 9 | 9 | 7 | **59** |
| Pipedrive | 17 | 8 | 12 | 9 | 8 | 5 | **59** |
| Azure Boards | 16 | 9 | 11 | 9 | 7 | 5 | **57** |
| Canva | 14 | 8 | 12 | 9 | 7 | 6 | **56** |

## Portfolio thesis

The follow-on set expands Clovy in four directions without pretending every
provider deserves a one-off architecture:

- HubSpot, Salesforce, and Pipedrive turn customer meetings into reviewed CRM
  updates and next steps.
- Asana, ClickUp, and Azure Boards cover project systems beyond Linear.
- Box and Dropbox add explicit, live file access without copying provider
  corpora into OpenSoftware infrastructure.
- GitLab Issues covers teams whose software delivery issue graph is not on
  GitHub.
- Canva turns approved meeting outcomes into bounded visual artifacts and
  export jobs.

The repeated product advantage is not generic search. Clovy combines the user's
local note graph with a live provider object, then proposes one bounded action
whose destination and disclosure are visible before approval.

## Cross-provider findings

### Authentication is the first portfolio gate

The ten APIs do not share one safe desktop OAuth profile. Dropbox explicitly
supports authorization code with PKCE and refresh tokens for desktop apps.
GitLab documents PKCE as its recommended public-client flow. Salesforce's
current External Client App path can require PKCE and omit a client secret for
supported clients. In contrast, the published HubSpot, Asana, ClickUp, and
Canva exchanges rely on confidential credentials or server-side custody.

No implementation may embed a reusable provider secret in the desktop binary.
Each plan therefore begins with an auth matrix and ends the spike with one of:

1. supported public-client OAuth with Keychain token custody;
2. a separately approved TEE exchange that returns the user's token to the
   device, or a provider-required server connector whose token and content
   path is explicitly accepted and documented;
3. a provider-supported user-created app for a limited technical preview; or
4. deferral.

### Local mode and events stay separate

Provider webhooks require public HTTPS endpoints. Local mode uses live fetch,
bounded polling, or a provider-supported on-device long poll while Clovy is
awake. Webhooks belong to an accepted away-mode relay design and cannot become
an undocumented backend content path.

### Authorization is narrower than provider permission

Every integration binds one account and an explicit object boundary: HubSpot
pipelines, Salesforce record types, Asana projects, Box/Dropbox folders,
GitLab groups/projects, ClickUp spaces/lists, Pipedrive pipelines, Azure DevOps
projects, or Canva folders/designs. The component that holds the provider
credential enforces that selection after resolving provider ids: the Rust
provider proxy for local mode. The proposed hosted connector broker was
superseded by ADR-0073; no replacement is authorized. A model-supplied id
cannot widen the local boundary.

## Sequencing

### Wave 0: prove the connector kit across auth shapes

1. Implement Dropbox as the clean public-client and file-boundary reference.
2. Spike Salesforce External Client Apps and GitLab PKCE without committing to
   broad product scope.
3. Run one confidential-exchange spike, starting with HubSpot because it has
   the strongest follow-through score.

### Wave 1: customer and project follow-through

4. Ship HubSpot if its credential boundary is approved.
5. Add Asana on the shared project schema.
6. Add Pipedrive only if customer demand justifies a second CRM before
   Salesforce.

### Wave 2: enterprise variants

7. Add Salesforce and Box after tenant-admin and policy pilots.
8. Add GitLab with GitLab.com first, then explicitly tested self-managed
   versions.
9. Add Azure Boards through Microsoft Entra ID, not deprecated Azure DevOps
   OAuth registration.

### Wave 3: overlapping and creative surfaces

10. Add ClickUp only after Asana validates the shared project schema.
11. Add Canva after its server credential and public-integration review path
    pass, with export rather than arbitrary design editing as v1.

## Shared launch contract

All ten inherit the privacy, install/connect separation, read/action split,
approval, retry, revocation, prompt-injection, billing, and measurement rules
in [portfolio.md](portfolio.md). In addition:

1. A provider-specific auth matrix is release evidence, not a planning note.
2. Approval previews show both the provider destination and Clovy-originated
   note content that will leave the device.
3. Broad CRM or file scopes are never treated as proof that broad use is
   appropriate. Clovy enforces the narrower user selection.
4. Schema discovery is cached as non-secret metadata only and invalidated on
   permission or provider-schema changes.
5. Provider webhooks remain off until an accepted relay threat model names
   payload lifetime, routing metadata, replay defense, and revocation.

## Revisit conditions

The ordering should be rescored when any of these happens:

- a provider adds or removes public-client PKCE, materially changing delivery
  confidence;
- Clovy identifies a primary sales, support, creative, or enterprise file ICP;
- the first three provider pilots produce activation and weekly task data;
- away mode has an accepted threat model and implementation; or
- user research shows one omitted workflow is more frequent than the assumed
  project and CRM follow-through jobs.

## Source snapshot

Research was checked against official provider documentation on 2026-07-14.
Each PRD links its provider-specific sources. Key cross-provider references:

- [Dropbox OAuth guide](https://developers.dropbox.com/oauth-guide)
- [GitLab OAuth 2.0 provider API](https://docs.gitlab.com/api/oauth2/)
- [Salesforce External Client Apps](https://developer.salesforce.com/docs/platform/hosted-mcp-servers/guide/create-external-client-app.html)
- [Azure DevOps service authorization](https://learn.microsoft.com/en-us/azure/devops/service-hooks/authorize?view=azure-devops)
- [Canva Connect API security](https://www.canva.dev/docs/connect/guidelines/security/)
