# Private connectors: local mode threat model

**Status:** source of truth for connector privacy copy. Every user-facing
privacy claim about connectors must be traceable to a line on this page; if a
claim is not supported here, the claim is wrong, not the page. Scope is **local
mode** (the only connector mode that ships in Phases 1-2). Away mode (the
Phase 3 TEE relay) will publish its own, larger threat model when it ships.

## What local mode is

You authorize Google on your Mac. The refresh token Google mints is stored in
your Mac's Keychain. Every Gmail and Calendar API call Clovy makes originates
from your device, using that token, and goes straight to Google. Clovy's
inference providers are not involved in those connector calls.

## What OpenSoftware can and cannot see

**Cannot see, by architecture:**

- Your Google refresh or access tokens. They are in your Keychain, protected by
  Keychain access control and Clovy's code-signing identity, and never
  transmitted to OpenSoftware. We hold no credential that can read your mail,
  so there is nothing to hand over under a subpoena and nothing to steal in a
  breach of our servers.
- The content of your mail or calendar as it flows through a connector call.
  Connector requests go device -> Google, directly.

**Can see, and you should know it:**

- **Model inference is a separate path.** When a routine runs, its prompt (which
  can include mail or calendar content the routine chose to read) goes to
  the routine's installed CLI and its configured model provider/account.
  That can be a network call off your device. The connector privacy claim
  covers token custody and direct connector API calls, not model inference.
  Clovy has no server billing or telemetry path; the CLI/provider's account
  and retention rules apply to its inference requests.

## Trust surface for local mode

Local mode adds exactly these things to what you already trust by running Clovy:

1. **Google.** You are granting Clovy's OAuth client access to the scopes you
   approve. Google sees the same API calls any mail client would.
2. **Your device's Keychain and Clovy's code signature.** Token secrecy rests on
   macOS Keychain access control and Clovy being correctly signed. A local
   attacker with your unlocked machine and your login keychain can reach the
   tokens, the same as for any app's Keychain items.
3. **The embedded agent.** Clovy's agent can call the connector tools you enable.
   The protections below bound what it can do without you.

## Agent-facing protections

- The agent cannot read the token store while Clovy's sandbox is engaged. The
  profile denies both direct reads of Keychain database paths and Mach lookup
  of the `securityd` services used by Keychain APIs. Tokens live in the
  unsandboxed Rust host, and MCP tool servers hold only a scoped loopback token,
  never a Google token. Signed rc builds verify this with both the `security`
  CLI and a direct `SecItemCopyMatching` probe before release.
- Connector tool descriptions mark email and calendar content as untrusted
  input, because a hostile email can carry instructions (prompt injection).
- Mutating actions (send, draft, label changes, event changes, invite
  responses) are gated by **trust mode**. Plain and read-only routines cannot
  call mutating tools. A routine that enables actions starts in approval: the
  action parks in Clovy's own approval surface, shows the exact recipients or
  object and change, and waits for you. Autonomous execution must be earned
  (three successful approval-mode runs) and is granted per tool.

## Interactive-session isolation

The pinned runtime auto-includes globally enabled MCP servers unless Clovy pins
the interactive toolset. Clovy does pin it: normal chat receives the base read
and action servers, whose mutations always park for approval, but never receives
the per-routine `june_*_auto_*` servers that carry autonomy grants. Cron jobs use
their own per-job `enabled_toolsets`, so this exclusion does not weaken a grant
the user intentionally gave a routine. See
[ADR-0016](adr/0016-private-connectors-local-mode.md).

## Revocation

Disconnecting an account deletes its tokens from the Keychain immediately.
"Also revoke Clovy's access with Google" additionally calls Google's revoke
endpoint so the grant is dead server-side. Both paths are in Settings ->
Connectors.
