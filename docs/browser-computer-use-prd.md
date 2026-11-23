# PRD: Browser use and computer use plugins

Browser contract updated 2026-11-01 for the approved
`melhorias-agente-audio-automacao` change, design D6 and `browser-control` delta.
The historical extension decision remains in ADR-0017; its extension, pairing
and tab-sharing implementation is superseded by this cutover. Accepted ADRs
are append-only. This document owns browser tool names and trust boundaries;
[the portfolio PRD](plugins/browser-use-prd.md) describes the product promise.

## Browser use

Clovy operates its own browser, using the pinned integrated agent-browser
engine through a private authenticated host. It never attaches to the user's
usual browser or copies personal cookies or profiles. Attended tasks use a
separate persistent profile with an exclusive lease. Logins survive Stop,
session close and process shutdown. Browser handles, references and authority
do not survive a task or engine restart.

Background is the default. A visible window is permitted when an operation
cannot be performed in the background, before dispatch or after a definitive
`unsupported` / `not_attempted` result. A delivered or uncertain mutation is
never replayed in another mode. Visible operation grants no extra capability,
authentication authority, system privilege or approval exemption.

### Consent and human control

The Browser access grant is stored on this device and checked by the native
broker for every call. The Plugins capability switch and in-chat request card
change that same grant. Experimental visibility remains separate from consent.
Enabling access does not replay a previous action or restart a turn.

Plugins and the shell session controls show the effective background/visible
mode. **Take over to sign in** suspends all agent snapshots, screenshots and
actions before exposing the browser window. Passwords, one-time codes and
payment data remain human-only. Closing the window does not resume automation;
**Resume agent control** is an explicit user gesture. Stop terminates browser
control and its processes while preserving the persistent login profile.

The CLI keeps the bound turn waiting with the central `request_clarification`
tool while the user signs in and resumes control. It then lists the task's tabs
and takes a fresh snapshot. Ending the turn tears down the browser, including
its visible window; a sign-in handoff must not end the turn prematurely.

Page text and screenshots required for reasoning can be sent to the user's
configured AI model. Consent explains this before access is enabled. Clovy
never presents a saved login as permission to automate sensitive fields.
Diagnostic logs contain no visited URLs, page text, screenshots or field
values. Outcome records describe delivery and error codes, without treating
the model's claim or an unrelated capture as verification of a website action.

Host UI commands are separate from the model-facing tools:

| Command | Contract |
| --- | --- |
| `browser_access_get` | Returns the attended grant boolean |
| `browser_access_set` | Accepts `enabled: boolean`; returns the stored boolean |
| `browser_sessions` | Returns `{sessionId, mode: "background" or "visible", suspended: boolean}[]` |
| `browser_human_takeover` | Accepts `sessionId`; suspends agent access before human login |
| `browser_resume_control` | Accepts `sessionId`; explicitly resumes control |
| `browser_stop_session` | Accepts `sessionId`; stops control and processes, retains logins |

These commands are host UI controls. They are not arbitrary model-callable
operations and do not grant session ownership to another turn.

### Model-facing tool contract

The Clovy MCP browser group exposes exactly 13 tools. There is no shared-tab,
share-code, extension-pairing or generic upstream-MCP path.

| Tool | Required arguments |
| --- | --- |
| `start_session` | None; the host binds conversation/run ownership |
| `close_session` | `session_id` |
| `navigate` | `session_id`, integer `tab_id`, `url` |
| `snapshot` | `session_id`, integer `tab_id` |
| `screenshot` | `session_id`, integer `tab_id` |
| `click` | `session_id`, integer `tab_id`, `ref` |
| `fill` | `session_id`, integer `tab_id`, `ref`, `text` |
| `press` | `session_id`, integer `tab_id`, `ref`, `key` |
| `back` | `session_id`, integer `tab_id` |
| `list_tabs` | `session_id` |
| `open_tab` | `session_id` (`url` is optional) |
| `switch_tab` | `session_id`, integer `tab_id` |
| `close_tab` | `session_id`, integer `tab_id` |

`start_session` accepts no model-selected owner, profile, workspace or runtime
namespace. Every subsequent operation revalidates the bound conversation/run,
including after an approval wait. An external caller cannot substitute another
turn's session. Tab ids identify exact Clovy-owned targets; a closed or stale
tab fails, without selecting an active, neighbouring or newly created tab.
`fill.text` is the sole fill value and `press.ref` is required: key input
focuses and revalidates that exact target rather than typing globally.

### References, approvals and delivery

Snapshots return visible text and opaque references scoped to
session/tab/document/epoch. Sensitive values are redacted before emission.
Private `inspect_reference` and `guarded_action` host operations compare exact
origin, document, backend node and element facts immediately before input.
A replaced same-name element is stale, never recovered by role or name.
These operations are not additional model-facing tools.

Field inventory and AX observations share an owned debugger pause boundary;
the native inventory includes closed shadow roots and inherited editable
content. Image masks cover native editable-subtree layout/text geometry,
including overflow and pseudo-elements, and reject unsupported paint geometry.
The host verifies its own breakpoint hit in the exact session and releases only
its own debugger agent before dispatching an event. Observation preserves actual
visibility and focus, including a background tab. A pre-existing or unexpected
pause, lost ownership or failed restoration terminates the task with unknown
delivery rather than leaving an uncertain page advertised as usable.
The pinned stock browser's screenshot API waits for frame presentation. A hidden
headed tab cannot reliably present a fresh frame while page tasks are paused,
so its image is refused before capture with a definite visibility requirement.
Snapshots and inspection remain available, and observation never activates the
tab. Default headless background sessions and visible headed tabs retain native
PNG capture. This limitation is not proof of a hidden-tab image.
Inspection exposes value-redacted metadata and an opaque random `metadataId`
revision. Exact original facts remain private in the reference cache, so two
different raw labels that both display as redacted cannot share approval.
The revision changes when those facts change; no raw field content or
guessable digest of it is published.

The private host classifies original field facts before redaction and returns
only the closed `interactionClass` value. `human_only` routes directly to human
takeover before the central approval broker; `approval_required` uses the existing
attended approval path. Missing or invalid classification fails closed. The
broker never infers sensitivity again from redacted attributes, and a model
cannot supply this private result as a tool argument. The class is bound into
the exact metadata revision and rechecked by the same host guard before input.

The native broker owns grants, session leases, reference expiry, network
policy, artifact roots, input approvals and lifecycle. The
central approval broker is the only approval authority. Approvals require
post-wait owner/tab/document revalidation. Every click, fill and key press uses
the central approval card: page labels, element roles and absence of known
consequential words cannot prove that an event handler is harmless. The card
shows a capped, quoted label from the inspected element as untrusted page text,
along with its origin. No field value or proposed input text is displayed.
There is no browser-specific always-allow or site-wide approval. Approval never
authorizes interaction with sensitive fields, including a password displayed
as text through current-password or new-password autocomplete.
Transport failure after dispatch returns an uncertain outcome, without retry
or a fabricated success verification. Stop or shutdown cancels startup and
in-flight work and reaps both daemon and browser processes before releasing
the persistent profile lease. Only ephemeral profiles are deleted.

### Routines

A routine needs its own explicit browsing grant and bound routine/run identity.
It uses an anonymous ephemeral profile, never the attended login profile.
Clicks, fills and key presses remain blocked because nobody is present to
approve them. Reads and explicit public-page navigation retain their grants.
Public destination resolution, validation and DNS/IP pinning remain enforced,
including redirects and subresources. Private, loopback, link-local and
non-HTTP destinations remain denied. The connection proxy does not inspect
HTTPS contents, so final URL/origin and grant checks remain necessary.

### Distribution and isolation

The engine source, downstream host patch and Chrome for Testing version are
pinned with auditable hashes/provenance, built and packaged with Clovy. Runtime
does not install a global CLI, detect a personal Chrome profile, auto-connect,
or fall back to the retired extension backend. The host accepts typed private
operations only; no public command TCP endpoint, dashboard, streaming, eval,
plugins, WebMCP or unrestricted upstream MCP is exposed to a model.

Persistent profiles use native credential storage, without basic password-store
or mock-keychain flags. The profile lease lasts from reservation until the
browser and engine have actually exited. A second conversation receives a
profile-busy error. Restored pages can retain logins but never old tool handles.

### Computer use (phase 2)

- Productize the pinned runtime's macOS capture and input implementation behind
  Clovy's private helper and Rust policy broker. The runtime receives only the
  single app-owned Computer use action surface, never the upstream registry or
  helper transport.
- Bundle a pinned, signed cua-driver as an app resource and point the
  runtime at it through its supported binary-path and version overrides; the
  upstream network installer never runs.
- TCC onboarding (Accessibility, Screen recording) follows the dictation
  helper pattern: bundle-scoped, prompting variant, polled re-checks.
- First app access parks in Rust and surfaces as a native Clovy authorization
  card. The agent invokes the operation immediately and never asks for a
  textual approval. Authorization clears when the task ends.
- App lifecycle is narrow: background launch accepts only an app display name;
  current-stage restoration accepts only the exact PID and window selected and
  revalidated by Rust after app authorization. Paths, URLs, launch arguments,
  debug options, and arbitrary process activation are unavailable.
- The capability hard-requires a vision-capable model; otherwise the plugin
  is unavailable with a switch-model notice. Routines never get the toolset.
- A release self-test starts the bundled driver and fails the build if the
  private system interfaces it relies on break on a macOS update.

## Verification

Required executable proof covers the real patched engine and adapter against
synthetic pages: navigation/back, snapshots, screenshots, target-bound
click/fill/press, tab operations, replaced nodes, stale documents, closed tabs,
owner rejection, profile leases, startup cancellation, uncertain dispatch,
network restrictions, persistent profile reopening and human takeover/resume.
Frontend tests cover consent, effective mode and explicit control transitions.
Browser-shim screenshots prove rendering only. Native macOS/Windows packaging,
genuine external-provider login and paid inference require their own authorized
native acceptance; Linux fixtures do not establish those outcomes.

## Boundaries

No personal-browser attachment, profile/cookie import, shared-tab authority,
automated password/OTP/payment entry, persistent routine logins, arbitrary
model-selected profiles, generic evaluation, or retry of an uncertain action.
Browser use and Computer use remain separate grants and contracts.
