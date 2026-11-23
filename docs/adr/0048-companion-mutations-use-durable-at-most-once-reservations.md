---
status: accepted
date: 2026-07-17
---

# Companion mutations use durable at-most-once reservations

## Context

A stable mobile operation id is not enough to prevent duplicate side effects.
If Desktop submits a mutation and crashes before saving the response, a retry
can submit the same agent message, recording control, settings change, or note
edit again. An in-memory in-flight lock closes only the concurrent-request
window and is lost with the process.

There is no generic transaction shared by SQLite, the frontend, recording
state, and Hermes. Desktop therefore cannot always reconstruct whether an
interrupted cross-boundary mutation completed.

## Decision

- The companion saves a mutation's stable operation id to Keychain before it
  sends the encrypted request. A Keychain failure blocks the mutation.
- Desktop durably reserves every mutating operation id in SQLite before it
  crosses a side-effect boundary.
- A completed response atomically replaces the reservation. Ordinary retries
  return that saved response.
- If Desktop crashes after reservation but before completion, the reservation
  remains as a distinct outcome-unknown response. The same operation id is not
  dispatched again. The companion tells the user to inspect June on the Mac;
  only a later, explicit repeat of the action creates a new operation id.
- Reservations and completed responses share the seven-day retention bound,
  but not an eviction pool. Completed responses are capped at 1,024 per device.
  Up to 128 unresolved reservations are retained separately; reaching that
  limit refuses new mutation dispatch rather than evicting an ambiguity guard.
  Revocation deletes both.
- The schema upgrade recognizes the prior retryable busy reservation payload,
  relabels it as pending, and rewrites it to the non-retryable outcome-unknown
  result. Upgrading cannot turn an existing ambiguity guard into evictable
  completed history.
- OS Accounts sign-out stops new companion work and waits for the active
  account-operation barrier, including relay dispatch and pairing approval,
  before it revokes account-scoped local authorization. Frontend execution is
  tracked independently of the relay waiter, so aborting transport cannot hide
  an emitted or queued React operation from that barrier. A frontend queue item
  releases the barrier on expiry only when the queue atomically proves no
  consumer took it. The app shell owns the only native companion listener;
  AgentWorkspace consumes only that operation-id-keyed internal queue, avoiding
  a dual-listener handoff that could run one reserved request twice. Relay
  connection and send operations are time-bounded; the transport is
  cancellation-aware and force-joined within the shutdown bound. If claimed
  frontend, pairing, or other account work cannot stop within its bound, logout
  fails without clearing tokens.

## Consequences

The protocol chooses at-most-once mutation dispatch over automatic recovery
when an operation's outcome cannot be proven after a crash. A rare interrupted
operation can remain unresolved instead of being transparently retried, but a
reconnect cannot silently duplicate an agent run or another side effect.

Read-only requests may still be repeated. Retryable failures that are known not
to be final are not saved as completed results; a mutation reservation remains
until a definitive result replaces it, the user explicitly repeats the action
after an outcome-unknown response, or retention expires.

Persisting only final responses, relying on an in-memory lock, and replaying
all ambiguous failures were rejected because each leaves a crash window for a
duplicate side effect.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
