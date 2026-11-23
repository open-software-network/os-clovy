---
status: accepted
date: 2026-07-16
supersedes: 0041 (mobile presentation technology only)
---

# June Companion uses native SwiftUI presentation

## Context

ADR 0041 selected React Native for companion presentation while reserving
authentication, keys, pairing, transport, lifecycle, and push handling for
Swift. The resulting boundary added a JavaScript runtime, CocoaPods, codegen,
and a second presentation state model without reducing the native security
surface. The Open Software native iOS design system is already implemented in
SwiftUI and provides established iPhone/iPad navigation, Dynamic Type,
semantic color, typography, motion, and XcodeGen patterns.

The product direction changed before release: June Companion should be a fully
native Swift application and OS Accounts login should open the hosted login in
the system browser.

## Decision

- Build June Companion as a separate SwiftUI iOS/iPadOS application generated
  by XcodeGen. It remains independent from the Tauri desktop target and has no
  WebView or managed mobile runtime.
- Use a main-actor application model over high-level Swift companion services.
  SwiftUI receives typed decrypted note, agent, settings, recording, and device
  values, but never receives access tokens, refresh tokens, private keys,
  session keys, APNs tokens, or raw encrypted frames.
- Keep the shared Rust Noise state machine behind the existing narrow C ABI.
  This ADR does not change the relay-first E2EE, pairing, desktop authority,
  capability, storage, backgrounding, or revocation decisions in ADR 0041.
- Use `ASWebAuthenticationSession` for OS Accounts Authorization Code with
  PKCE. The phone signs in separately; linking to an already signed-in Mac
  remains a second, explicit desktop-approved decision.
- Follow the native Open Software design system: semantic platform colors,
  ABC Diatype for product copy, Berkeley Mono only for technical values,
  regular and medium weights, system navigation/search/sheets, 44-point tap
  targets, and reduced-motion support.

## Consequences

- There is one native lifecycle and presentation state model instead of a
  Swift-to-JavaScript bridge and duplicate reducers.
- iPhone and iPad builds use Xcode/XcodeGen and XCTest; pnpm, Metro, CocoaPods,
  and React Native codegen are not part of the canonical companion workflow.
- A separately registered public OAuth client with the exact callback
  `junecompanion://auth/callback` is still required. The desktop client or
  desktop session is not reused.
- Native SF Symbols are allowed for this application because the repository's
  central icon packages are web-only. User-facing copy and weight rules still
  apply.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
