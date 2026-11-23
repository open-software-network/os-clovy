---
status: accepted
date: 2026-07-16
---

# The MVP companion relay runs as one replica

## Context

The companion relay keeps WebSocket senders and five-minute pairing attempts
in process memory. Postgres persists device identities, credential hashes,
links, push tokens, and revocations, but it is not a cross-instance message
bus or connection directory.

Phala keeps one WebSocket on one instance for that connection's lifetime but
does not promise affinity across separate phone and desktop connections. With
multiple relay replicas, linked peers can land on different processes and
cannot route ciphertext to each other. Replicated pairing requests can also
land away from the process that created the attempt.

## Decision

- Deploy the MVP companion relay as exactly one June API replica.
- Treat in-process pairing and connection state as ephemeral. A restart closes
  sockets and expires in-progress pairing; clients reconnect or start a fresh
  five-minute code.
- Keep durable authorization in Postgres so a restarted single replica can
  authorize linked devices without re-pairing.
- Do not describe the relay as highly available or horizontally scalable.
- Before adding a second replica, add a shared expiring pairing store and an
  authenticated cross-instance ciphertext router or equivalent connection
  ownership mechanism, then test revocation propagation between instances.

## Consequences

- The complete local and single-replica path is secure and functional, but a
  relay restart causes a temporary companion disconnect.
- Rolling multi-replica deployment is not a safe availability optimization for
  this MVP. Deployment automation must replace the single replica deliberately
  and let clients reconnect afterward.
- Production promotion requires a single-replica configuration, capacity
  canaries, alerting, and an explicit scale-out gate.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
