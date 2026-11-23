# Companion relay runbook

> **Historical document (2026-10-28).** Server, Auto/funding, hosted relay and
> native-agent claims below describe an earlier design, not the current app.
> The inference server is removed; chat/routines use installed CLI engines,
> Groq/OpenRouter/fal operations use the user's keys directly, and sharing has
> no replacement. Current boundaries are in
> [ADR-0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md),
> [llm-providers.md](llm-providers.md), and [configuration.md](configuration.md).


> **Fork note:** this fork's desktop app no longer has the companion (removed by
> the `desvincular-servicos-open-software` change). The Clovy API relay and the
> `crates/clovy-companion-*` / `june-companion-*` crates are removed in a later change.

## Required production configuration

Set `CLOVY__COMPANION__DATABASE_URL`. Companion endpoints are disabled outside
local development if Postgres connect, migration, or snapshot load fails. Run
the Clovy API migration before traffic and verify active device/link counts load
without identifiers in logs.

Run exactly one companion-enabled Clovy API replica. Pairing attempts and live
WebSocket senders are process-local in the MVP; multiple replicas can split the
phone and desktop onto different processes. Scale-out is blocked until the
shared pairing store and cross-instance ciphertext router in ADR 0045 exist.

Expose the existing Clovy API HTTPS port with WebSocket upgrade support. The
desktop calls `/v1/companion/relay?deviceId=...` with its normal OS Accounts
bearer. A desktop-approved phone generates and uses its opaque `Device`
credential; only its hash reaches the relay and is stored with the linked
device. Do not expose a desktop port.

## Limits

Relay JSON is 64 KiB maximum, each socket allows 120 inbound frames/minute,
has one 64-frame outbound queue, and one connection per device. Backpressure
disconnects rather than growing memory. Offline ciphertext retention is zero.
Controls expire after 30 seconds. Opaque APNs wakes have a 30-second per-device
cooldown. Pairings expire after five minutes and are capped at eight pending
attempts per account and 4,096 pending attempts per relay process. Completed
idempotency responses expire after seven days and are capped at 1,024 per
device. Unresolved mutation reservations use a separate 128-entry cap and are
never evicted to admit completed reads. Each account is capped at 32 active
companion device records.

## Health and canary

Use `/livez` and `/readyz`, then pair two test devices. Verify a 65 KiB frame,
cross-user route, revoked device, and duplicate socket are rejected. Hold an
idle WebSocket longer than the expected mobile/desktop heartbeat window and
force reconnects during an instance replacement. Phala has no session affinity
on reconnect, so confirm the Postgres snapshot authorizes the replacement
single instance.

Before enabling APNs, run an HTTP/2 egress canary to Apple's sandbox endpoint.
Phala documents outbound connectivity diagnostics but does not guarantee APNs
egress or a maximum WebSocket lifetime; keep both checks in deploy promotion.

## Incident response

For suspected routing/auth compromise, disable the companion database config
and redeploy to fail closed, preserve redacted request/security logs, rotate
APNs and database credentials, revoke affected device rows, and require fresh
pairing. Never turn on plaintext payload logging for diagnosis.
