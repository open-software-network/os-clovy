---
status: accepted
date: 2026-07-17
supersedes: 0043 (mobile account authentication only)
---

# June Companion signs in separately with OS Accounts

## Context

ADR 0043 removed mobile account login because desktop approval is the grant
that authorizes a phone to control one June Desktop installation. That remains
true, but the companion also needs to prove the user's identity and relay
eligibility independently. A desktop session cannot be copied to the phone,
and possession of an account bearer cannot replace device pairing.

The original companion requirements called for a public native OS Accounts
client. Product direction also requires the hosted OS Accounts login to open in
the system browser rather than an embedded web view.

## Decision

- Register June Companion as a separate public OAuth client. It has no client
  secret and uses the exact callback `junecompanion://auth/callback`.
- Swift opens the hosted login with `ASWebAuthenticationSession` and uses
  Authorization Code with PKCE S256, cryptographically random state, exact
  callback validation, native code exchange, and refresh-token rotation.
- Access and refresh tokens remain in a device-only Keychain item owned by the
  authentication service. SwiftUI receives only the account profile.
- Mobile account login and desktop pairing remain separate grants. A new
  additive authenticated proposal endpoint accepts the mobile bearer and the
  short-lived pairing proof. June API requires the bearer user to match the
  user who created the desktop pairing before it presents the device for
  desktop approval.
- Linked configuration records the OS Accounts user id. A legacy or
  differently owned link must be revoked and linked again rather than silently
  moving between accounts.
- Signing out invalidates the refresh token when possible, revokes this linked
  device, and clears local account tokens. Device revocation alone does not
  sign the user out.

## Consequences

- A separately registered `ocl_` client and allowlisted mobile callback are
  deployment prerequisites. The desktop client registration is not reused.
- A stolen account token cannot control June Desktop without the QR secret,
  device key, locally generated device credential, and explicit desktop
  approval. A stolen QR secret cannot pair from a different OS Accounts user.
- The existing unauthenticated `/propose` route remains available for older
  clients. New companion builds use `/propose-authenticated`, preserving the
  additive `/v1` compatibility rule.
- The relay verifies mobile access tokens through the existing strict OS
  Accounts verifier. No `osk_` key or other shared secret enters the app.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
