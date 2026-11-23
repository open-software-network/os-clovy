---
status: proposed
date: 2026-11-01
---

# Manage external MCP OAuth in the host

## Context

The Vercel MCP server uses browser OAuth and requires client approval by
Vercel. A CLI's separately managed connection is not authority to use its
client identity or credentials in Clovy. Disconnect can race an in-flight
refresh or tool call, and an HTTP failure after a tool call does not establish
that a mutation was not applied.

## Decision

Use the existing custom MCP gateway and operating-system credential store for
Clovy-managed OAuth. Discover the provider's metadata, use a Clovy client,
PKCE and a local callback, and require a valid tool catalog before saving a
connection. Invalidate earlier work on reconnect and disconnect. Report local
credential cleanup and remote revocation separately. Do not automatically
replay a tool call after an uncertain delivery. CLI-owned connections retain
their own identity, credentials and catalog.

The exact official Vercel MCP resource has a fixed metadata profile at
`https://mcp.vercel.com/.well-known/oauth-authorization-server`, validated
against the protected-resource issuer `https://vercel.com` and the official
HTTPS endpoint set. The issuer-root document describes a different contract
with only `web_message.opener`. Using it for a loopback client is incompatible;
Clovy rejects it rather than ignoring its advertised modes. Only the exact
MCP resource receives this profile, with no configurable override or generic
fallback. This deliberately couples the integration to the provider's public
MCP contract; an endpoint change fails closed and requires review.

SQLite journals a pending credential reference before the secure-store write,
then atomically activates the new reference while retaining the previous one
until cleanup succeeds. This cannot make SQLite and OS credentials one
transaction, but it makes all crash windows recoverable without plaintext
tokens, an extra credential backend or a new migration. Local cleanup and
remote revocation remain separate outcomes, including after restart.

## Consequences and acceptance

Vercel must review and accept Clovy's client and the user must consent before a
live connection can be established. An advertised refresh grant does not prove
that a particular authorization includes one. Native Mac and Windows tests,
including both CLI engines, remain required. This proposed record is not
evidence that those prerequisites or the full lifecycle tests have passed.

## 2026-11-01 addendum: custody before activation

Connection activation and credential custody are separate boundaries. A grant
can be issued before catalog discovery completes, so storing credentials only
after a valid catalog loses custody on cancellation, timeout or rejected
activation. Connect reserves an operation-specific reference, journals possible
issuance before the token request, and holds the received bundle in OS storage
before awaiting the catalog. It activates that reference only after the catalog,
epoch and configuration checks pass. Recovery compensates the actual candidate
or retains its reference and honest remote uncertainty. An exchange still able
to issue a grant prevents a complete remote-cleanup claim and deletion of its
only journal.

Revocation history is bound to the affected credential reference and revision.
A new authorized primary can be used while the earlier grant still needs
provider revocation; successful reconnect preserves the historical warning and
does not inherit the earlier grant's active-eligibility latch. Current-token
revocation never clears an earlier unknown grant by implication. These records
remain host-owned and separate from model arguments and Settings input.

The previous healthy primary also remains in historical custody when a new
grant replaces it. It is not automatically revoked during reconnect:
[RFC 7009](https://www.rfc-editor.org/rfc/rfc7009#section-2.1) permits revocation
of related tokens, and Clovy cannot assume the new grant has an independent
token family. Explicit Disconnect/Delete instead reconciles every saved grant.
Confirmation and local removal of one known superseded token clear only that
record; they do not resolve an unknown rotated token or exchange at the same
reference. The current grant's generation anchor remains stable across policy
revision changes made before an authorization attempt finishes.

This extends the existing JSON metadata journal without a schema migration or
an additional credential backend. Compensation alone cannot cover a dropped
exchange response; durable possible-issuance intent is deliberately conservative.
It is not a new client-registration/redirect policy or evidence of provider or
native acceptance.

On 2026-11-01 public GETs and the production discovery helper confirmed the MCP
metadata profile. Its omission of response modes permits query according to
RFC 8414; the authorization URL explicitly requests query. This is metadata
compatibility evidence only. No live browser grant, client approval or refresh
concession is asserted. DCR and callback/lifecycle fixtures use synthetic
servers; OS credential-store behavior still needs native acceptance.
