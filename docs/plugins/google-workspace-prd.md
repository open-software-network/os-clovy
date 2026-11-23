# PRD: Google Workspace plugin

- **Mode:** CEO
- **Rank:** 1 of 10
- **Score:** 94/100
- **Date:** 2026-07-13
- **Status:** Gmail + Calendar local mode shipped; expansion proposed
- **Related:** JUN-277, ADR-0016

## Thesis

Google Workspace should be Clovy's default work graph. The existing Gmail and
Calendar connector proves the private architecture. The plugin should now turn
that connection into a complete meeting loop: prepare from mail, calendar,
files, and prior notes; capture the conversation; then draft the follow-up,
update the source document, and schedule the next step.

This is the highest-priority plugin because it combines broad adoption, daily
frequency, meeting adjacency, and an already-shipped foundation. It is also the
clearest demonstration of Clovy's positioning: the account grant stays on the
Mac and provider calls originate on-device.

## Customer and problem

The primary customer runs their work from Gmail, Calendar, Drive, Docs, and
Meet. Context is scattered across threads, invitations, documents, and meeting
artifacts. Before a meeting they reconstruct history manually. Afterward they
copy action items into replies, documents, and events. Existing assistants can
index this corpus in the cloud, but that is unacceptable for the confidential
prosumer Clovy is built for.

## Product promise

Connect Google once, choose the parts Clovy may use, and move from meeting
context to completed follow-through without giving OpenSoftware a credential
that can read the account.

## V1 experience

1. The user opens Google Workspace in Plugins and sees Gmail and Calendar as
   connected, plus optional Drive and Meet capabilities.
2. Each capability explains its requested access before the system browser
   opens Google's consent flow.
3. Clovy can prepare a briefing from relevant threads, attendees, calendar
   context, Drive files, prior notes, and available Meet transcripts.
4. During or after the meeting, Clovy can cite source items and create a local
   note with stable provider references.
5. Clovy can draft a reply, create or update an event, and create a new Docs or
   Sheets artifact. Every outward action follows the routine's trust mode.
6. The user can disconnect one Google account and verify that every Google
   capability is off.

## Scope

### Launch expansion

- Preserve shipped Gmail search/read/draft/send and Calendar read/event flows.
- Add Drive metadata search, explicit file read/export, and file references.
- Add Docs creation and targeted edit operations.
- Add Sheets creation and range-level updates; rich spreadsheet work remains
  the separate Spreadsheets plugin.
- Add Contacts lookup for attendee resolution.
- Add Meet space, recording, and transcript discovery where the account and
  edition expose them.
- Add meeting-prep and follow-up skills plus gallery routine templates.

### Later

- Slides generation after the artifact foundation exists.
- BigQuery, admin-wide deployment, multi-account routing, and away-mode events.
- Proactive Drive change triggers after the event architecture is approved.

## Non-goals

- Mirroring the user's Google corpus into OpenSoftware infrastructure.
- Replacing Google editors with Clovy-native collaboration.
- Domain-wide delegation or an administrator grant in v1.
- Silently broadening the scopes of already-connected accounts.
- Treating Google sign-in as Clovy identity; OS Accounts remains Clovy identity.

## Packaging

The plugin contains:

- Required connector capability: none. Users may enable any one Google family.
- Optional connector capabilities: Gmail, Calendar, Drive, Contacts, Meet.
- Skills: meeting prep, follow-up, inbox-to-note, agenda builder, decision log.
- Templates: morning briefing, next-meeting brief, promised follow-ups, weekly
  relationship recap.

The listing remains one Google Workspace plugin instead of separate Gmail,
Calendar, Drive, Docs, and Sheets tiles. Capability-level grants stay visible
inside it.

## Privacy and trust

The shipped Google path implements the local-mode design proposed in ADR-0016.
Refresh tokens live in the Keychain. Google API calls go from the device to
Google through the Rust provider proxy, with no hosted intermediary. Model inference remains a separate path and the consent copy says so.

Provider content is untrusted. Full email bodies and file contents are fetched
only when needed. Mutating tools live in separate action servers. Approval and
earned autonomy are enforced in Rust.

## Business model

Local Google capabilities are available on Hobby. Event-triggered and
multi-step routines are Pro. Existing agent usage meters model calls; Google
API calls do not create a new Clovy credit action.

## Success measures

| Metric | Target |
| --- | ---: |
| Connected Google users enabling Drive within 30 days | 35% |
| Weekly connected users running a meeting brief | 30% |
| Briefs followed by at least one completed follow-up action | 25% |
| Median first successful Google read after opening the tile | under 2 minutes |
| Google actions completed without correction | at least 97% |
| Token-material incidents | 0 |

## Risks and gates

- Google verification for sensitive or restricted scopes is the external
  release gate. Scope expansion cannot hide behind the existing approval.
- Google's installed-app guidance currently says incremental authorization is
  not supported for installed apps, while its broader OAuth guidance recommends
  requesting access at the moment it is needed. Engineering must validate the
  exact consent behavior before promising add-one-capability upgrades.
- Meet artifacts depend on workspace edition, recording policy, and organizer
  access. Missing artifacts are an expected state, not an error.
- Large Drive files can exhaust model context. Metadata-first search and bounded
  extraction are launch requirements.

## Decision requested

Approve one Google Workspace plugin with capability-level grants; prioritize
Drive + meeting artifacts next; keep local connection free and routine
automation Pro.

## Sources

- [Google OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Google OAuth policies](https://developers.google.com/identity/protocols/oauth2/policies)
- [Google app data controls in ChatGPT](https://help.openai.com/en/articles/10408842)
