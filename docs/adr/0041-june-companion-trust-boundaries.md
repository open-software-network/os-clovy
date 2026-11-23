---
status: accepted
date: 2026-07-16
---

# June Companion uses a native security boundary and relay-first E2EE

## Context

June needs an iPhone and iPad companion without exposing Tauri IPC, SQLite,
the filesystem, or the Hermes Gateway to the internet. iOS suspends ordinary
WebSockets, and OS Accounts login proves an account rather than permission to
control one particular Mac.

The cryptographic dependency spike considered Noise through `snow`, separate
Swift and Rust protocol implementations, and ad-hoc CryptoKit composition.
`snow` is maintained, implements the reviewed Noise framework, is licensed
Apache-2.0 OR MIT, and can compile into a narrow static-library C ABI for iOS.
The known `snow` 0.9.6-0.9.7 out-of-bounds advisory does not affect the pinned
0.10 line. A shared state machine avoids cross-language transcript drift.

## Decision

- Ship a separate bare React Native New Architecture app, not Tauri mobile, a
  WebView, a managed runtime, or a SwiftUI-only duplicate of all presentation.
- React Native owns adaptive presentation and typed view state. Swift owns OS
  Accounts PKCE, Keychain, device authentication, QR scanning, transport,
  lifecycle, APNs, payload validation, and the TurboModule boundary.
- Keep the Mac authoritative for notes, recordings, and agent orchestration.
- Use outbound TLS WebSockets from phone and Mac through a blind June API
  companion relay. Direct LAN and peer-to-peer transports remain optional
  future implementations behind the transport boundary.
- Encrypt application frames end to end with a shared Rust Noise state
  machine: XXpsk3 for QR pairing and KK for linked reconnects, using
  25519/ChaChaPoly/BLAKE2s. Require a fresh handshake after 24 hours or
  2^20 messages.
- Pairing and OS Accounts authentication remain separate. Desktop approval
  creates the linked-device grant and its explicit capability set.
- Store Curve25519 private keys and tokens with Keychain access controls. Do
  not claim Secure Enclave backing because Secure Enclave does not directly
  hold the selected Curve25519 key type.
- Use content-free APNs background wake hints only. Foreground reconnect and
  cursor resynchronization remain the correctness path.

## Consequences

- The relay sees account/device routing metadata, IP addresses, timing, and
  ciphertext sizes, but not application plaintext or content keys.
- The mobile cache is encrypted and render-only. It is not multi-device sync.
- Offline control requests fail and are never replayed later.
- Production requires a mobile OAuth registration, Postgres, Apple signing,
  APNs credentials, relay DNS/deployment, and independent security review.
- Android, remote recording start, remote approvals, multi-desktop routing,
  and direct transports are deliberately deferred.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
