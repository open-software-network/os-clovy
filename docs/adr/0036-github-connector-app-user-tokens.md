---
status: proposed
date: 2026-07-21
---

# GitHub connector: App user access tokens only, June-side write gating

## Context

The GitHub plugin ([PRD](../plugins/github-prd.md), JUN-285) needs a connector
under the private-connectors architecture
([ADR-0016](0016-private-connectors-local-mode.md)). GitHub's auth model
differs from Google's and Linear's in three load-bearing ways:

1. **A GitHub App's user authorization carries no OAuth scopes.** What a user
   access token can do is the intersection of the app's configured permissions
   and the repositories the user selected when installing the app. There is no
   per-grant way to ask for "read only" the way Google scope URLs or Linear's
   `read`/`write` scopes do.
2. **GitHub enforces the repository boundary server-side for PRIVATE
   repositories only.** A user access token ("user-to-server" token) can
   implicitly read PUBLIC repositories outside the installation's selection
   (documented GitHub behaviour). June therefore applies its own repository
   gate in the proxy layer for every repo-scoped route (amended below;
   unlike Linear's selected-teams gate, the June gate here is required
   to uphold the PRD's privacy promise, not merely to reflect a UI choice).
3. **Authorization uses GitHub's device authorization flow (RFC 8628).** No
   callback URLs, no loopback listener. The user is shown a short `user_code`
   and directed to `verification_uri`; the app polls GitHub until the user
   approves, declines, or the code expires. Authorization needs only the
   client id — no client secret is required at the device-code or token
   endpoints. Device flow must be enabled on the GitHub App settings page.

The implementation plan's Phase 0 asked where a private key could live and
whether installation-token minting needs a TEE signer.

## Decision

1. **User access tokens only; no app private key anywhere.** v1 never mints
   installation tokens, so no private key ships in the binary, on June API, or
   in any signer. Every GitHub call is made on-device with the connected
   user's token, resolved from the Keychain in Rust
   (`co.opensoftware.june.github`, `-dev` in debug). June API stays out of the
   connector data path, extending ADR-0016 unchanged.
2. **Client id required; client secret optional.** Both load from
   `GITHUB_OAUTH_CLIENT_ID` / `GITHUB_OAUTH_CLIENT_SECRET` (runtime env
   overriding build-time `option_env!`). The client id is required to start
   the device flow; the client secret is optional.
   - Secret present (recommended): GitHub App should have "expire user
     authorization tokens" enabled; expiring tokens refresh normally using
     client_id + client_secret, and API revocation is available.
   - Secret absent: the GitHub App must have "expire user authorization
     tokens" disabled so tokens are non-expiring and the refresh path is never
     triggered. API revocation is unavailable; users must revoke manually at
     GitHub if needed.
   The authorization flow uses GitHub's device authorization flow (RFC 8628)
   from the Tauri event `june://connectors-github-device-code` (camelCase
   payload: `userCode`, `verificationUri`, `expiresInSeconds`) to display the
   code in the connect dialog while opening the verification URL in the browser.
   Two new user-facing error codes: `connector_github_device_declined` (user
   declined at GitHub) and `connector_github_device_expired` (code expired
   before approval). Device flow must be enabled on the GitHub App settings
   page; `connector_github_device_flow_disabled` surfaces if it is not.
3. **June-side read/write gating AND installed-repository boundary.** Because
   GitHub has no per-grant scopes, the `connector_accounts.scopes` column
   stores June-side markers (`read`, `write`) chosen by the user in the
   connect dialog. The Rust proxy refuses every `/v1/github-actions/*` route
   unless the account carries `write`. Additionally, every repo-scoped route
   (reads: `get_issue`, `list_issue_comments`, `get_pull_request`, `read_file`;
   writes: `create_issue`, `update_issue`, `add_comment`; search routes via
   post-filtering) checks `{owner}/{repo}` against the connected account's
   installed-repository set before calling GitHub. The set is cached per
   account for 300 seconds (populated lazily via `list_repositories`).
   Truncation policy: when the set is truncated (>500-item cap), June fails
   open (allows the call with a warning) rather than hard-breaking repos
   beyond position 500. Search routes (`search_issues`, `search_code`)
   post-filter results to the installed set when not truncated and include
   a `filteredOut` count in the response so the model can detect filtering.
4. **Writes are approval-only; autonomy is deferred.** All
   `june_github_actions` tools park in the ADR-0016 approval registry. No
   `june_github_auto_*` grant servers exist in v1, matching Notion's stance.
   `update_issue` cannot change issue state (close/reopen are PRD launch
   non-goals); it edits title, body, and labels only.
5. **Accounts are keyed by the numeric GitHub user id** (stringified), with
   the login kept as the display identity, following Linear's
   workspace-id-not-email keying. One GitHub account at a time, like every
   other provider in local mode.
6. **Connect refuses a grant with no installation.** Authorization and
   installation are separate GitHub steps, and a user-to-server token reaches
   only repositories the app is installed on. So `begin_connect_github`
   checks `GET /user/installations` and, when the user has authorized but not
   installed the app, returns `connector_github_not_installed` (pointing at
   the install page) instead of storing a "connected" account whose token can
   read nothing. The check runs both after a fresh authorization and inside
   the no-op reconnect short-circuit, so a user who uninstalls the app on
   GitHub (which does not invalidate the token) surfaces the error on their
   next reconnect rather than silently reaffirming a hollow account. It fails
   open on an indeterminate probe (transient 5xx / rate limit): the real
   repository calls surface access problems, and blocking a legitimate
   connect on a momentary blip is the worse failure. Repository selection
   itself still lives on GitHub, not in June.

## Consequences

- The privacy claim of ADR-0016 holds verbatim for GitHub: no OpenSoftware
  system ever holds a credential that can read the user's repositories.
- Refresh follows the Linear rotation pattern when the client secret is
  present: GitHub rotates the refresh token on every refresh when "expire user
  authorization tokens" is enabled on the app. When that setting is off,
  GitHub returns a non-expiring token and no refresh token; June stores it
  with a far-future expiry and the refresh path is never taken. When a refresh
  is needed and no secret is configured, June marks the account
  `reconnect_required` and surfaces `connector_reconnect_required` rather than
  calling GitHub with a missing credential.
- Read-only connects exist only as June-side enforcement. The GitHub consent
  screen shows the app's full permission set regardless; the connect dialog
  copy must not imply GitHub granted less.
- Repository selection is managed on GitHub (the installation), not in June's
  settings; June links out instead of duplicating that UI.
- The installed-repository boundary is June-side enforcement backed by the
  `list_repositories` cache (300 s TTL). It upholds the PRD promise of zero
  successful reads outside the installation selection for non-truncated sets.
  Accounts with >500 installation repositories trigger fail-open (warning log)
  per the truncation policy above.
- A future away-mode or installation-token feature (e.g. acting in repos the
  user cannot) would reopen the private-key question and needs a new ADR.

## Addendum 2026-10-18: GitHub CLI login as a second source

[ADR-0066](0066-github-connector-can-ride-the-github-cli-login.md) adds
"GitHub (via gh)" accounts whose token is read from the signed-in GitHub CLI
on every call and never stored. Points 1, 2 and 6 above apply only to
accounts connected through the GitHub App; the read/write markers (point 3)
and approval-only writes (point 4) apply to both.
