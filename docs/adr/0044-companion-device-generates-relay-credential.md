---
status: accepted
date: 2026-07-16
supersedes: 0043 (device credential issuance only)
---

# The companion device generates its relay credential

## Context

ADR 0043 moved account authentication to the signed-in desktop and introduced
a separate revocable credential for the approved phone. Its initial issuance
flow had June API generate the credential and return it through a proof-gated
pairing status response. The relay persisted only a hash, but the reusable
plaintext credential still existed in relay process memory until the pairing
expired.

The relay does not need the plaintext to authorize a device. The companion can
generate the same high-entropy material locally and present only its hash while
proposing its public identity.

## Decision

- The companion generates a random 32-byte device credential with the system
  cryptographic random source before it proposes pairing.
- The random bytes are encoded as an unpadded base64url credential. The
  proposal sends SHA-256 of that exact UTF-8 credential representation together
  with the companion device id, Curve25519 public key, display name, and pairing
  proof. The relay hashes the same authorization-header representation.
- June API holds and persists only the 32-byte hash. Desktop approval activates
  that hash and the explicit device link; no API response contains the
  plaintext credential.
- The companion stores the plaintext with a device-only Keychain access class
  before proposing its hash. An unapproved value has no relay authority and is
  reused after a crash so the stable device identity does not drift. If setup
  fails after desktop approval, the companion requests revocation and deletes
  the credential, linked configuration, and device identity.
- OS Accounts bearer authorization is accepted for desktop relay connections
  only. A device record with a credential hash must use the `Device` scheme.
- Pairing attempts are capped per account and globally. Expired attempts and
  their unpersisted device records are removed from relay memory.

## Consequences

- June API does not generate, return, or retain the reusable plaintext
  credential during pairing. It receives the encoded credential later only in
  the `Device` authorization header and compares its hash without storing it.
- Possession of a QR secret still does not activate the credential hash. The
  signed-in desktop must approve the presented companion identity.
- Reinstalling or revoking the companion discards the credential and requires
  a new pairing attempt. There is no server-side credential recovery path.
- Credential rotation is a new pairing with a new device identity, rather than
  an in-place bearer replacement.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
