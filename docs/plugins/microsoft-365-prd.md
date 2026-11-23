# PRD: Microsoft 365 plugin

- **Mode:** CEO
- **Rank:** 4 of 10
- **Score:** 84/100
- **Date:** 2026-07-13
- **Status:** Proposed

## Thesis

Microsoft 365 is the enterprise counterpart to Google Workspace and the
largest remaining gap in Clovy's work graph. One plugin should cover Outlook
mail and calendar, OneDrive and SharePoint files, and selected Teams context
through Microsoft Graph. The product outcome is not "connect Microsoft"; it is
private meeting preparation and follow-through for users whose organizations
standardize on Microsoft.

It ranks below Slack because enterprise tenant approval and Graph permission
complexity slow activation, but it ranks above narrower knowledge and software
delivery apps because it unlocks an entire work ecosystem.

## Customer and problem

Consultants, attorneys, accountants, recruiters, and enterprise operators live
in Outlook, Teams, OneDrive, and SharePoint. Their meeting history, documents,
and follow-ups are distributed across tenant-controlled systems. They need a
personal assistant but cannot authorize a cloud service to create an
unbounded secondary index of that work.

## Product promise

Connect a Microsoft account with delegated access, keep the refresh grant on
the Mac, and let Clovy prepare and act only within the scopes and resources the
user and tenant have approved.

## V1 experience

- One Microsoft 365 tile with capability states for Outlook mail, Calendar,
  Files, and Teams.
- System-browser authorization with clear personal-account versus work/school
  account and admin-consent outcomes.
- Meeting brief from invite, attendees, recent mail, relevant files, Teams
  context exposed by supported delegated APIs, and local Clovy notes.
- Draft mail, create/update a calendar event, and create or update a bounded
  file with approval.
- Stable links and source labels in Clovy output.
- Disconnect locally and handle tenant-side revoke or conditional-access
  failure without partial phantom connection states.

## Scope

### V1

- Outlook mail search/read/draft/send.
- Calendar read, availability, event create/update.
- OneDrive and SharePoint file metadata search and explicit read.
- Selected Teams/channel context only where stable delegated Graph APIs and
  tenant policy permit it.
- Meeting prep and follow-up skills.

### Later

- Shared/delegated mailboxes and calendars, Planner/To Do, Teams meeting
  transcripts/recordings, sensitivity labels, multiple tenants, sovereign
  clouds, and away-mode change notifications.

## Non-goals

- Application permissions, domain-wide admin impersonation, or tenant-wide
  indexing.
- Replacing Microsoft Purview, retention, or eDiscovery controls.
- Promising every Teams resource is available through one user grant.
- Storing Microsoft content in a Clovy-hosted service.
- Hiding admin-consent or conditional-access requirements from users.

## Packaging

- Required connector: none; each capability is optional within one plugin.
- Skills: Outlook meeting prep, client follow-up, file brief, Teams recap.
- Templates: tomorrow's meetings, unanswered client mail, weekly commitments.
- Composition: Documents and Spreadsheets can produce local files; Microsoft
  actions may publish an approved copy to the user's drive.

## Privacy and trust

Use delegated user permissions, not application permissions. Tokens stay in
Keychain and Graph calls originate on-device. Resource permissions and tenant
policy remain authoritative. Clovy must state that inference follows the
selected model path even though OpenSoftware is outside the connector path.

Mail, files, and Teams messages are untrusted input. Write operations park in
the Rust approval broker. Sharing, deletion, external recipients, and changes
to broadly visible resources remain approval-only.

## Business model

Local read and approved actions are available on Hobby. Cross-capability
routines and event-driven workflows are Pro. Enterprise admin deployment and
policy are future products, not prerequisites for personal delegated access.

## Success measures

| Metric | Target |
| --- | ---: |
| Users completing auth after opening the tile | 55% |
| Connected users with at least two capabilities working | 60% |
| Weekly connected users running a meeting brief | 30% |
| Completed follow-up action after a brief | 20% |
| Support cases with an unexplained admin-consent failure | under 5% of connects |
| Cross-tenant or permission-boundary incidents | 0 |

## Risks and gates

- Entra admin consent, conditional access, tenant policy, account type, and
  sovereign cloud produce a large state matrix.
- Graph webhook subscriptions require a public HTTPS endpoint and renewal;
  local v1 should poll or use delta queries while awake.
- Teams APIs and permissions are uneven. V1 must be scoped from verified Graph
  support, not parity marketing.
- Enterprise sensitivity labels and retention obligations may constrain file
  content and caching.

## Decision requested

Approve one Microsoft 365 plugin built on delegated Graph access, with Outlook
and Calendar first, Files second, and Teams only after an API/permission spike.

## Sources

- [Microsoft authorization code flow with PKCE](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow)
- [Microsoft Graph mail overview](https://learn.microsoft.com/en-us/graph/api/resources/mail-api-overview)
- [Microsoft Graph calendar overview](https://learn.microsoft.com/en-us/graph/api/resources/calendar-overview)
- [Microsoft Graph Teams overview](https://learn.microsoft.com/en-us/graph/api/resources/teams-api-overview)
