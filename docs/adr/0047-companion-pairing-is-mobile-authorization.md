---
status: accepted
date: 2026-07-17
supersedes: 0046 (mobile account authentication only)
---

# Companion pairing is the mobile authorization

## Context

June Desktop is already authenticated with OS Accounts when it creates a
pairing. The five-minute QR capability identifies that exact pending pairing,
and the user must still approve the presented phone on the signed-in Desktop.
A second account login on the phone adds an OAuth client, callback, refresh
token, and account bearer without adding authority over the Mac: explicit
Desktop approval remains the actual grant.

Removing the mobile login must not copy the Desktop session to the phone or
turn the QR into a long-lived credential. Pairing must remain account-bound,
short-lived, single-device, explicitly approved, and independent from the
credential used for later relay connections.

## Decision

- June Companion has no OS Accounts client, callback, browser login, or account
  token. OS Accounts authentication remains owned by June Desktop.
- An authenticated Desktop creates each pending pairing under its current OS
  Accounts user. The QR carries a random 32-byte secret, pairing id, relay URL,
  and expiry; June API receives only SHA-256 of the secret.
- Possession of the matching proof authorizes one phone to propose its device
  identity and locally generated credential hash to that pending pairing. The
  proof is compared in constant time, expires after five minutes, and cannot
  select or change the account attached by Desktop.
- The signed-in Desktop presents the phone identity and fixed capability set
  for explicit approval. Approval persists the phone under the pairing's user
  and activates its separate revocable device credential.
- The phone never receives the user's account id or Desktop bearer. Reconnect,
  push registration, and self-revocation use the device credential plus the
  linked device key; content remains protected by Noise.
- Desktop sign-out remains the account boundary: it stops account transport and
  revokes or locally retires the account-scoped Desktop identity and links.
- The existing `/propose` endpoint continues to authenticate with the
  short-lived QR capability. The superseded `/propose-authenticated` draft
  endpoint is removed before release because no mobile account bearer exists.

## Consequences

- The first-run companion surface is pairing, not login, and production no
  longer requires a separate public OAuth registration or callback allowlist.
- A copied QR can submit one candidate device during its short lifetime, but it
  cannot activate the device without the user approving it on Desktop. A
  competing proposal consumes the one-device claim and is visibly named before
  approval rather than silently gaining access.
- The relay learns the OS Accounts user from the authenticated Desktop-created
  pairing and never trusts a user id supplied by the phone.
- Reinstalling, revoking, or losing the device credential still requires a new
  Desktop-approved pairing; the QR secret is never a reconnect credential.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
