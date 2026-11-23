# Phase 0 findings: Linear OAuth and API spike

- **Mode:** CTO
- **Date:** 2026-07-15
- **Status:** Documentation spike complete; live-workspace verification pending
  OAuth app registration
- **Plan:** [linear-implementation-plan.md](linear-implementation-plan.md)
- **Issue:** JUN-284

## Verdict

Local token custody works. Linear's authorization-code flow supports PKCE with
an optional client secret, a loopback redirect, rotating refresh tokens, and a
revoke endpoint. No confidential exchange, hosted credential store, or intermediary
is needed, so the Linear connector extends the ADR-0016 local-mode
pattern as-is and **no new ADR is required for v1**. The plan's ADR threshold
(confidential token exchange, app credential, or webhook relay) is not
triggered.

## Question-by-question

### 1. Public desktop client without an embedded secret

Yes. The token exchange at `POST https://api.linear.app/oauth/token` marks
`client_secret` optional for PKCE flows; `client_id`, `code`, `code_verifier`,
and `redirect_uri` complete the exchange. `code_challenge_method` supports
`plain` and `S256`; Clovy always uses S256. PKCE is optional on Linear's side,
so Clovy must enforce it client-side rather than rely on the provider requiring
it. `http://localhost:<port>/...` redirect URIs are accepted as registered
callback URLs.

**Redirect URI matching caveat:** unlike Google, Linear matches the redirect
URI against the registered callback URLs exactly - it does not implement RFC
8252's rule to ignore the port on loopback addresses (observed in the wild:
Linear's MCP OAuth rejects ephemeral loopback ports). Clovy therefore binds
one of a fixed candidate port list (`44741`, `44742`, `44743`; override with
`LINEAR_OAUTH_LOOPBACK_PORT`) and the OAuth application must register a
`http://127.0.0.1:<port>/callback` URL for each candidate. Confirm during
live verification.

### 2. Refresh-token rotation, lifetime, revoke

- Access tokens live 24 hours; each refresh returns a new access token **and a
  new refresh token** (rotation).
- Refresh-token consumption has a 30-minute grace period for network-error
  recovery, which tolerates the retry races that strict one-shot rotation
  would turn into lockouts.
- `POST https://api.linear.app/oauth/revoke` revokes (200), with
  `token_type_hint` of `access_token` or `refresh_token`; disconnect must call
  it and treat 400 (already revoked) as success.

### 3. User actor vs app actor

`actor=user` (default) creates resources as the authorizing user and inherits
that user's team visibility. `actor=app` is for agents/service accounts, is
issued via the client-credentials flow (which needs a secret, so it is not
desktop-safe), and sees all public teams. **V1 uses the user actor**, per the
plan's default; app actor would both break the no-secret constraint and widen
visibility beyond the selected-team grant.

### 4. Scopes

`read` (always present), `write`, `issues:create`, `comments:create`,
`timeSchedule:write`, `admin` (avoid). The v1 write set (`create_issue`,
`update_issue`, `add_comment`, `create_project_update`) requires `write`
because issue updates and project updates have no granular scope. So v1
requests `read,write` in one consent, with every mutation still parked for
approval in the Rust proxy. There is no teams-scoped provider grant; selected
teams remain a Clovy-side authorization enforced in Rust.

### 5. Rate limits and GraphQL complexity

- OAuth apps: 5,000 requests/user/hour and 2,000,000 complexity points/hour;
  a single query may not exceed 10,000 points.
- Budget headers: `X-RateLimit-Requests-Remaining`,
  `X-RateLimit-Complexity-Remaining`, plus `*-Reset` epoch-ms headers; exceeding
  returns HTTP 400 with the `RATELIMITED` error code.
- Complexity is roughly 1 point/object, 0.1/property, multiplied by the
  pagination argument on connections, so the plan's bounded selections with
  explicit `first:` arguments fit comfortably. The proxy should surface the
  two remaining-budget headers in health diagnostics and back off on
  `RATELIMITED`.

### 6. Pagination

Relay-style cursors (`first`/`after`, `pageInfo.hasNextPage`/`endCursor`),
default page size 50, `nodes` shorthand available, `orderBy: updatedAt` for
recency reads. Cursors persist per the plan's SQLite state.

### 7. Idempotency for mutations

Provider-supported: `IssueCreateInput.id`, `CommentCreateInput.id`, and
`ProjectUpdateCreateInput.id` each accept a client-supplied UUID v4 ("The
identifier in UUID v4 format. If none is provided, the backend will generate
one."). Clovy mints the object UUID as the journal's action id, so an ambiguous
timeout reconciles by querying that exact id: a replay with the same id cannot
double-create. This satisfies shared-contract point 9 without the
fingerprint-only fallback the plan reserved for a provider without idempotency
support. `IssueUpdateInput` contains exactly the v1 allowlist fields (`title`,
`description`, `stateId`, `priority`, `assigneeId`, `projectId`, `cycleId`);
conflict detection re-reads `updatedAt` before commit.

### 8. Webhooks (confirming the deferral)

Webhooks require a public HTTPS, non-localhost URL, signed with HMAC-SHA256 in
`Linear-Signature`, with `webhookTimestamp` replay protection and a
`Linear-Delivery` UUID for dedupe. This confirms the plan's decision: local
mode uses live reads plus bounded polling; webhooks stay reserved for a future
away-mode relay with its own ADR.

## Residual Phase 0 items

- **OAuth app registration is a human step.** A public OAuth2 application must
  be created in a Linear workspace (client id ships in the binary; that is fine
  for a public client). Until then the flow cannot be exercised live.
- **Live verification** of rotation, grace-period behavior, revoke, and scope
  errors against a real workspace once the app exists, per the plan's
  verification list.

## Sources

- [Linear OAuth 2.0 authentication](https://linear.app/developers/oauth-2-0-authentication)
- [Linear rate limiting](https://linear.app/developers/rate-limiting)
- [Linear pagination](https://linear.app/developers/pagination)
- [Linear webhooks](https://linear.app/developers/webhooks)
- [Linear SDK schema](https://github.com/linear/linear/blob/master/packages/sdk/src/schema.graphql) (`IssueCreateInput`, `CommentCreateInput`, `ProjectUpdateCreateInput`, `IssueUpdateInput`)
