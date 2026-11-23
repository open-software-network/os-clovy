---
status: accepted
date: 2026-10-18
amends: ADR-0036
---

# GitHub connector can ride the GitHub CLI login

## Context

[ADR-0036](0036-github-connector-app-user-tokens.md) connects GitHub only
through the Clovy GitHub App's device flow: a user access token in the
Keychain (`co.opensoftware.clovy(-dev).github`), refreshed by Clovy, and an
installation check on connect. Many users of the CLI chat engines are
already signed in to the GitHub CLI (`gh`) on the same Mac, and the owner
asked on 2026-10-18 that Clovy use that login instead of a second
authorization (openspec change `runtimes-cli-na-ui`).

The alternatives were to keep the device flow only, or to copy the `gh`
token into the Clovy Keychain once. Copying duplicates the secret, ages
without the refresh `gh` does for itself, and keeps working after the user
runs `gh auth logout`, which is exactly when they expect access to stop.

## Decision

1. **A second credential source for the same provider.** A GitHub account row
   may carry `credentialSource: "gh"` in its non-secret metadata ("GitHub
   (via gh)"). It lives in the same `connector_accounts` table, keyed by the
   numeric GitHub user id from `GET /user`, with the same one-account rule.
2. **The token is read from `gh` on every call and never stored.**
   `github_access_token` returns the stdout of
   `gh auth token --hostname github.com` for such an account, run from the
   login shell's PATH and environment with a 2 s timeout, stdin closed and
   stderr discarded; the buffer is zeroized. Nothing of it goes to the
   Keychain, SQLite, a file, a log, or the argv, environment or files of an
   agent CLI. When `gh` yields no token the account turns
   `reconnect_required` and the tool call fails with
   `connector_reconnect_required`.
3. **Automatic connect, read only.** At app start and when Plugins opens,
   Clovy connects a "via gh" account when `gh` is on the login PATH,
   `gh auth status --hostname github.com` succeeds and no GitHub account
   exists. The account starts with the read marker only; write is added with
   "Add access" as a Clovy-side grant (no device flow) and every write still
   parks for approval. A "via gh" account that went `reconnect_required`
   heals on the next detection when `gh` is signed in again as the same user.
4. **Disconnect stops the automation.** Disconnecting a "via gh" account
   deletes the row (there is nothing to revoke or delete in custody) and
   writes `githubCliAutoConnect: false` to `github-cli-connector.json` in the
   app data directory. "Connect with GitHub CLI" connects explicitly and turns
   it back on.
5. **No installation check, `/user/repos` for listing.** A `gh` token is an
   OAuth token, not a GitHub App user token: the installation endpoints refuse
   it, so `list_repositories` reads `GET /user/repos` for these accounts and
   the connect skips `connector_github_not_installed`.

## Consequences

- The repository reach of a "via gh" account is whatever the user's `gh`
  login reaches, not a GitHub App installation selection. Clovy's
  `read`/`write` markers and write approvals still bound what its tools do,
  but the installation-scoped privacy promise of ADR-0036 holds only for
  accounts connected through the App. The Plugins row says "via gh" so the
  difference is visible.
- The token's GitHub scopes may be broad; Clovy does not narrow them, it only
  exposes its own tools.
- Every GitHub tool call spawns one short `gh` process. If that ever measures
  slow, an in-memory cache of at most 60 s is the allowed remedy, never disk.
- App-connected accounts are unchanged: device flow, Keychain custody,
  refresh, revoke and the installation check stay as ADR-0036 decides.
