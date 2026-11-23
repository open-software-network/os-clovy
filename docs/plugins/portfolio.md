# Clovy plugin portfolio: top 10

- **Owner:** CEO + CTO
- **Date:** 2026-07-13
- **Status:** Proposed portfolio for JUN-309
- **Related:** JUN-275, JUN-278, JUN-283, JUN-284, JUN-285

## Executive decision

Clovy should launch a deliberately small, first-party plugin portfolio that
turns meeting context into completed work. The first ten plugins should be:

| Rank | Plugin | Score | Portfolio role | Current state |
| ---: | --- | ---: | --- | --- |
| 1 | Google Workspace | 94 | Default personal work graph | Gmail + Calendar shipped; expansion proposed |
| 2 | Browser use | 91 | Universal fallback for web work | Accepted in JUN-278; implementation active |
| 3 | Slack | 86 | Team context and follow-through | Proposed in the connector roadmap |
| 4 | Microsoft 365 | 84 | Enterprise work graph | New proposal |
| 5 | Computer use | 81 | Universal fallback for Mac work | Accepted phase 2 in JUN-278 |
| 6 | Notion | 77 | Durable team knowledge | Tracked by JUN-283 |
| 7 | GitHub | 73 | Software delivery context and action | Tracked by JUN-285 |
| 8 | Linear | 69 | Product execution context and action | Tracked by JUN-284 |
| 9 | Documents | 67 | Local-first finished written artifacts | New proposal |
| 10 | Spreadsheets | 64 | Local-first structured analysis | New proposal |

This ordering is a portfolio priority, not an instruction to stop work already
in flight. Google remains the release baseline. Browser use can ship before
Computer use because the latter still has a driver and macOS sandbox spike.
Provider review, store review, or OAuth verification can move one workstream
around another without changing the strategic ranking.

## What changed in the benchmark

The current ChatGPT feature is not the legacy 2023 plugin manifest model. As
of 2026-07-09, OpenAI describes a plugin as a workflow package that can contain
skills, apps, and app templates. The underlying app supplies external data,
actions, interactive UI, search, sync, or deep research; installation policy
and app permission policy remain separate. The directory is available across
ChatGPT web, desktop, Work, and Codex.

That model maps cleanly to Clovy if Clovy keeps its own canonical terms:

- A **plugin** is the user-facing capability bundle in the Plugins area.
- A **Skill** provides reusable workflow guidance.
- A **Toolset** groups runtime tools.
- An **MCP server** exposes tools to the embedded runtime.
- A **connector** is specifically the private-by-architecture path to a
  third-party account. A connector may be one component of a plugin, but the
  words are not interchangeable.

Clovy should adopt the useful product layering without adopting cloud indexing
as the default. A plugin can be installed while its optional connector is not
connected. Connecting an account grants only provider scopes. A plugin's
trust mode decides how Clovy governs outward actions. None of those controls
override permissions in the source system.

## Surface inventory

The benchmark exposes six capability families:

| Family | Current benchmark behavior | Clovy interpretation |
| --- | --- | --- |
| Workflow packaging | Skills, apps, and app templates can ship together | First-party plugin bundles with a manifest, bundled skills, toolsets, and optional connectors |
| Discovery | Directory, search, categories, featured listings, installed state | The JUN-275 Plugins area plus contextual suggestions in chat |
| Invocation | Explicit selection, `@` mention, or model discovery | Plugin chips, slash/mention entry points, and a visible suggestion card |
| Data access | Live search and optional pre-indexed sync | Live, metadata-first local connector reads; no provider corpus copied to OpenSoftware |
| Action control | Read, routine write, important write, and admin policy | Existing `read_only -> approval -> autonomous` trust modes and broker-enforced approvals |
| Administration | Role availability, app setup, action controls, domains, disconnect | Personal v1 connection controls; team policy only after Clovy has an organization model |

Interactive third-party widgets and cloud-wide pre-indexing are not launch
requirements for Clovy. Clovy's local desktop surface, note graph, routines, and
approval cards already cover the higher-value parts of the job.

### Candidate census reviewed

The directory is dynamic and varies by plan, workspace, role, region, and
supported surface. This census covers the complete product surface and every
candidate or family specifically named by the official directory, sync
catalog, plugin use-case guide, app help, and workspace release notes available
during the 2026-07-13 review. OpenAI describes 66 single-app plugins without
publishing one stable, account-independent list, so this is not a claim that
every account sees every listing.

| Candidate family | Named current examples reviewed | Portfolio disposition |
| --- | --- | --- |
| Google work graph | Gmail, Calendar, Drive, Docs, Sheets, Slides, Meet, Contacts, BigQuery | Google Workspace selected; BigQuery and Slides deferred |
| Microsoft work graph | Outlook Email, Outlook Calendar, OneDrive, SharePoint, Teams | Microsoft 365 selected as one capability-granular plugin |
| Team communication and knowledge | Slack, Notion | Slack and Notion selected |
| Project and product management | Aha!, Asana, Azure Boards, Basecamp, ClickUp, Linear, Teamwork.com | Linear selected; the rest scored as alternatives |
| Software delivery | GitHub, GitHub Enterprise template, GitLab Issues, Replit, Lovable | GitHub selected; GitLab and hosted builders deferred |
| File stores | Dropbox, Box | Deferred behind ecosystem file access and local artifacts |
| CRM and sales | Salesforce, HubSpot, Pipedrive, Zoho CRM, Clay | Scored below the top ten; revisit when sales workflows are a primary ICP |
| Customer support | Intercom, Help Scout, Zoho Desk | Scored below the top ten; narrower than Clovy's current ICP |
| Structured data and warehouses | Airtable, Databricks, Hex, Snowflake, BigQuery | Deferred for narrower ICP and admin/data-model complexity |
| Creative work | Adobe, Canva, Figma | Deferred until the local artifact foundation proves itself |
| Consumer and lifestyle | AllTrails, Apple Music, Booking.com, Expedia, Instacart, Spotify, Target, Tripadvisor, Zillow | Outside Clovy's private work focus |
| Agent execution | Browser interaction and computer interaction | Browser use and Computer use selected |
| Local work products | Documents, spreadsheets, PDFs, presentations, visualizations, sites | Documents and Spreadsheets selected; adjacent formats follow the shared artifact broker |
| Hosted and remote execution | ChatGPT Sites, DigitalOcean Droplet Workspace, Codex Remote | Deferred; Clovy should first prove local artifacts and execution |
| Role packages | Sales, Data Analytics, Product Design, Creative Production, Investment Banking, Public Equity Investing | Evaluated as packaging patterns; the underlying jobs score below the first ten |

The census also reviewed the cross-cutting surfaces around each listing:
directory discovery, search and categories, explicit and model-suggested
invocation, interactive UI, live search, deep research, pre-indexed sync, write
actions, per-action confirmation, role access, domain restriction, connection,
disconnect, developer-mode custom apps, public review, app templates, locally
built plugins, workspace sharing, role packages, Sites, remote workspaces,
record-and-replay skill creation, RBAC, analytics, and organization policy.
Clovy's shared product contract below records which of those surfaces it should
adopt. Local-plugin sharing and a third-party marketplace are platform choices,
not candidates in the end-user top-ten ranking.

## Ranking method

Each candidate was scored out of 100. The rubric intentionally rewards Clovy's
core loop and private architecture over directory visibility.

| Criterion | Weight | Question |
| --- | ---: | --- |
| Core-loop fit | 25 | Does it improve capture -> understand -> act? |
| ICP frequency | 20 | How often does Clovy's confidential prosumer encounter the job? |
| Action leverage | 15 | Can Clovy complete work, not only retrieve context? |
| Retention and composition | 15 | Does it create recurring use and combine with notes, routines, and other plugins? |
| Privacy differentiation | 15 | Does local execution make Clovy meaningfully more trustworthy or useful? |
| Delivery confidence | 10 | Can the team ship a narrow, reliable v1 with known APIs and review paths? |

Scores reflect evidence available on 2026-07-13, not permanent market facts.
They should be revisited after 30-day activation, weekly use, approval, and
task-completion data exist.

### Reproducible scorecard

Values are weighted points, not unweighted ratings. Each row sums to its total,
so a reviewer can change one assumption without reconstructing the model. Every
omitted candidate within ten points of the cutoff is scored individually. The
remaining grouped rows are conservative family ceilings: their best member is
at least 13 points below the cutoff, so member-level variance cannot change the
top-ten decision without first changing the family's assumptions.

| Candidate | Core /25 | Frequency /20 | Action /15 | Retention /15 | Privacy /15 | Delivery /10 | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Google Workspace | 24 | 20 | 14 | 14 | 14 | 8 | **94** |
| Browser use | 23 | 18 | 15 | 15 | 12 | 8 | **91** |
| Slack | 22 | 18 | 14 | 14 | 12 | 6 | **86** |
| Microsoft 365 | 23 | 18 | 13 | 13 | 12 | 5 | **84** |
| Computer use | 22 | 17 | 15 | 14 | 10 | 3 | **81** |
| Notion | 20 | 15 | 12 | 13 | 11 | 6 | **77** |
| GitHub | 19 | 13 | 13 | 12 | 9 | 7 | **73** |
| Linear | 18 | 12 | 13 | 11 | 8 | 7 | **69** |
| Documents | 18 | 15 | 11 | 9 | 10 | 4 | **67** |
| Spreadsheets | 17 | 13 | 12 | 9 | 8 | 5 | **64** |
| HubSpot | 19 | 10 | 13 | 11 | 8 | 2 | 63 |
| Salesforce | 19 | 9 | 13 | 11 | 8 | 2 | 62 |
| Asana | 17 | 10 | 11 | 10 | 8 | 5 | 61 |
| Box | 16 | 11 | 8 | 9 | 9 | 7 | 60 |
| GitLab Issues | 17 | 9 | 10 | 10 | 8 | 6 | 60 |
| ClickUp | 16 | 10 | 11 | 9 | 8 | 5 | 59 |
| Dropbox | 15 | 11 | 8 | 9 | 9 | 7 | 59 |
| Pipedrive | 17 | 8 | 12 | 9 | 8 | 5 | 59 |
| Azure Boards | 16 | 9 | 11 | 9 | 7 | 5 | 57 |
| Canva | 14 | 8 | 12 | 9 | 7 | 6 | 56 |
| Zoho CRM | 16 | 8 | 11 | 9 | 8 | 4 | 56 |
| Aha! | 15 | 9 | 10 | 10 | 7 | 4 | 55 |
| Basecamp | 15 | 9 | 10 | 9 | 7 | 5 | 55 |
| Figma | 14 | 8 | 11 | 9 | 7 | 6 | 55 |
| Teamwork.com | 14 | 8 | 10 | 9 | 7 | 6 | 54 |
| Adobe | 13 | 8 | 11 | 8 | 7 | 6 | 53 |
| Clay | 14 | 7 | 11 | 9 | 7 | 4 | 52 |
| Data and warehouses family ceiling | 12 | 7 | 11 | 8 | 8 | 5 | 51 |
| Support family ceiling | 14 | 7 | 10 | 8 | 7 | 4 | 50 |
| Hosted sites and builders family ceiling | 12 | 6 | 13 | 8 | 5 | 4 | 48 |
| Presentations and visualizations | 12 | 8 | 10 | 7 | 6 | 4 | 47 |
| Remote infrastructure: DigitalOcean and remote workspaces | 8 | 4 | 11 | 6 | 4 | 4 | 37 |
| Consumer and lifestyle directory apps | 5 | 4 | 8 | 4 | 3 | 6 | 30 |

The closest omission is HubSpot at 63, one point below Spreadsheets. CRM meeting
follow-through is strong, but Clovy's current ICP encounters it less often, its
best proactive workflows need administrator setup or away-mode events, and
distribution adds a difficult credential and review path. Customer evidence
can change that decision; the threshold is now explicit.

The six role-specific plugins are not independently scored because they bundle
skills and app integrations already represented by the capability candidates;
including them would double-count the same value. They remain an important
packaging pattern for future Clovy skill collections, not a distinct connector
or execution capability competing for an implementation slot.

Provider deployment variants such as the GitHub Enterprise app template are
folded into the parent capability score. They affect delivery and rollout, but
do not represent an additional user job or portfolio slot.

## Sequencing

### Wave 0: finish the foundation

1. Preserve the shipped Google Gmail + Calendar path and Plugins foundation.
2. Ship Browser use v1 and finish the Computer use driver spike.
3. Extract a provider-neutral connector kit from the Google implementation:
   token custody, account index, provider proxy, read/action server split,
   trust enforcement, approval journal, and health diagnostics.

### Wave 1: cover the two work ecosystems

4. Add Slack local mode.
5. Expand Google Workspace with Drive and Meet artifacts.
6. Build Microsoft 365 on the provider-neutral connector kit.

### Wave 2: make context operational

7. Add Notion.
8. Add GitHub.
9. Add Linear.

### Wave 3: create finished local artifacts

10. Ship Documents, then Spreadsheets, on one artifact broker and shared
    render-and-verify pipeline.
11. Re-run the portfolio score before starting the next candidate.

The waves permit parallel engineering where provider registration, external
review, and local implementation have independent critical paths. They do not
permit shipping multiple one-off OAuth stacks.

## Shared product contract

Every launch plugin must satisfy the same contract:

1. The listing states what the plugin can read and change; which Clovy note,
   conversation, memory, device, IP-address, and approximate-location context
   can reach the provider; what leaves the device for inference; whether
   OpenSoftware is in the connector data path; and links the provider's privacy
   and retention policy. The approval preview identifies the Clovy-originated
   content disclosed by that specific action.
2. Install, connect, grant, trust mode, and runtime mode are separate states.
3. The Plugins tile, any capability-specific Settings control, and contextual
   in-chat suggestion point to one source of truth. A plugin can keep management
   exclusively in Plugins when a second Settings surface would only duplicate it.
4. Read tools return compact structured summaries first. Full bodies or files
   are fetched only when the task requires them.
5. Mutating tools are separate from read tools and enforced in Rust, not by
   prompting the model.
6. Disconnect revokes provider access where supported, removes Keychain
   material, disables the runtime surface, ends active leases, and verifies the
   disconnected state.
7. Provider content is untrusted input. Tool descriptions and the Clovy soul
   carry injection warnings, while the broker remains the enforcement point.
8. Routines receive explicit toolsets and an explicit account binding. No
   routine silently selects the first account.
9. Automatic write retries require a provider-supported idempotency key. A
   local journal records `pending`, `committed`, or `ambiguous`, but cannot by
   itself make a provider mutation idempotent. After an ambiguous timeout Clovy
   must reconcile provider state using a stable action fingerprint, or block
   replay and ask the user to confirm.
10. No plugin embeds a service-managed provider key or introduces a hosted
    intermediary for provider content. User keys and connector grants stay in
    the local credential store and go only to their own provider. The retired
    hosted/away-mode proposals do not authorize a replacement server.

## Business model

The private connection itself should remain available on Hobby. Privacy is not
the upsell. Pro value comes from high-frequency automation: event-triggered
routines, multi-step cross-plugin workflows, unattended execution where the
provider permits it, and higher run limits. Computer use can remain Pro while
its cost and support burden are measured. Local Documents and Spreadsheets
should be broadly available, with model calls charged through existing agent
usage.

## Portfolio success metrics

| Metric | 90-day target | Why it matters |
| --- | ---: | --- |
| Weekly active users with at least one enabled plugin | 50% | Plugins become a core product surface |
| Enabled-plugin users completing one plugin-backed task per week | 40% | Measures work completed, not connections |
| Median time from tile open to first successful read | under 3 minutes | Setup is not the product |
| Approved actions completed without correction | at least 95% | Trust must precede autonomy |
| Connector-derived security or token incidents | 0 | Non-negotiable |
| 30-day retention lift for plugin users | at least 15 points | Validates portfolio value |

Provider-specific PRDs add activation and outcome measures without replacing
these shared metrics.

## Explicit deferrals

The highest-scoring provider deferrals now have a separate follow-on decision
package in [integrations-next-ten.md](integrations-next-ten.md). Their scores
and ordering remain unchanged from this portfolio so the extension does not
rewrite the original top-ten decision.

- **Dropbox, Box, OneDrive-only, and SharePoint-only plugins:** valuable file
  access, but Google Workspace, Microsoft 365, Notion, and local Documents cover
  the dominant jobs first. Provider-specific file stores remain candidates.
- **Salesforce, HubSpot, Pipedrive, Zoho CRM, Clay, and other CRM/sales tools:**
  strong meeting follow-through and the closest ranked omission, but narrower
  than the first ten and likely to need admin setup or an away-mode webhook
  path for the best proactive experience.
- **Aha!, Asana, Azure Boards, Basecamp, ClickUp, GitLab Issues, and
  Teamwork.com:** credible planning alternatives, but Linear and GitHub cover
  the first product/engineering wedge without multiplying overlapping setup.
- **Intercom, Help Scout, and Zoho Desk:** useful support context, deferred
  until support teams are a validated primary segment.
- **Figma, Canva, and presentation creation:** compelling output, but Clovy
  should prove the shared artifact broker with Documents and Spreadsheets
  before adding remote design surfaces or a presentation renderer.
- **Airtable and databases:** overlap with Spreadsheets, Notion, and Linear.
- **Travel, shopping, real estate, education, music, and lifestyle:** visible
  in the public app directory but outside Clovy's private work focus.
- **A third-party plugin marketplace:** the security, signing, update, and
  policy model is a separate product. The first ten are first-party bundles.
- **Role-specific plugin packs, local-plugin sharing, and ChatGPT Sites:** useful
  packaging and platform benchmarks, but not additional account connectors.
  Clovy should revisit them after the first-party manifest, artifact broker,
  and organization model exist.
- **Cloud sync of entire provider corpora:** conflicts with the local-mode
  trust story. Live, scoped reads come first; away mode requires its own
  accepted threat model.

## Source snapshot

The source snapshot is intentionally dated because the ecosystem changed four
days before this document was written.

- [Plugins in ChatGPT and Codex](https://help.openai.com/en/articles/20001256-plugins-in-chatgpt-and-codex) - package model, directory, installation policy, app permissions, and surfaces.
- [Apps in ChatGPT](https://help.openai.com/en/articles/11487775-connectors-in-chatgpt) - interactive UI, search, deep research, sync, write actions, and admin controls.
- [Plugin use cases and prompts](https://help.openai.com/en/articles/12084614-app-use-cases-and-prompts) - current work categories and supported app examples.
- [ChatGPT release notes](https://help.openai.com/en/articles/6825453-chatgpt-release-notes) - current Google, Microsoft, Box, Dropbox, Notion, and Linear action expansion.
- [ChatGPT Business release notes](https://help.openai.com/en/articles/11391654) - current Slack, Asana, Intercom, Google, Microsoft, and workspace plugin changes.
- [Apps with sync](https://help.openai.com/en/collections/15507678-apps-with-sync) - current named sync catalog across knowledge, planning, code, CRM, and support tools.
- [ChatGPT public app directory](https://chatgpt.com/apps/) - public featured, productivity, and lifestyle inventory.
- [Apps SDK tool design](https://developers.openai.com/apps-sdk/plan/tools) - focused tools, predictable structured output, read/write separation, and discovery metadata.
- [Apps SDK guidelines](https://developers.openai.com/apps-sdk/app-guidelines) - action annotations, data minimization, reliability, and review expectations.

## Companion documents

Each ranked plugin has a CEO-mode PRD and CTO-mode implementation plan in this
directory. These documents are proposals unless their status says an existing
Clovy decision is already accepted. An implementation plan does not by itself
authorize a provider registration, a new permission, an external deploy, or a
change to an accepted ADR.
