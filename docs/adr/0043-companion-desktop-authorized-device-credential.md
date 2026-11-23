---
status: accepted
date: 2026-07-16
supersedes: 0041 and 0042 (mobile account authentication only)
---

# June Companion is authorized by a signed-in desktop

## Context

June Desktop already has the OS Accounts identity required to create and
approve a linked device. Requiring the phone to repeat account login adds a
second OAuth registration and exposes an account bearer to the mobile device,
but it does not prove permission to control a particular Mac. The explicit
desktop approval is the actual grant.

Removing mobile login must not copy the desktop session, make a QR code a
long-lived credential, or allow knowledge of a device id to open the relay.

## Decision

- Only June Desktop authenticates pairing creation, status, approval, and
  administrative revocation with OS Accounts.
- The desktop generates the 32-byte Noise pairing secret before creating the
  relay pairing. It sends only SHA-256 of that secret as a short-lived pairing
  proof. The QR carries the secret, pairing id, relay URL, and expiry.
- The phone submits the pairing proof, its device id, and its Curve25519 public
  key. The proof is compared in constant time and cannot be used after the
  five-minute pairing expires.
- After the user approves the presented phone on the signed-in desktop, June
  API issues a random opaque device credential only through the proof-gated
  mobile status endpoint. The relay persists only SHA-256 of the credential.
- The phone stores the credential with its linked configuration in Keychain
  and uses the `Device` authorization scheme for its relay socket, APNs token
  registration, and self-revocation. The desktop continues to use its OS
  Accounts bearer.
- Relay authorization still requires a non-revoked explicit link. Noise XXpsk3
  completes pairing and authenticates both static keys; linked reconnects use
  Noise KK. A device credential never grants plaintext access by itself.

## Consequences

- June Companion has no OS Accounts login UI, OAuth callback, refresh token,
  client id, or copied desktop token. The first action is scanning a code from
  June Desktop.
- Revocation invalidates both the relay link and the device credential. A
  leaked credential remains bound to one device id and still lacks the private
  key required by the end-to-end encrypted transport.
- Production persistence adds a nullable 32-byte credential hash to companion
  device trust metadata. Plaintext credentials and pairing secrets are never
  stored by June API.
- The pairing and device credential endpoints are additive to the existing
  June API surface; desktop OS Accounts contracts remain unchanged.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
