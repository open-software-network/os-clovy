---
status: accepted
date: 2026-07-28
---

# Companion media results use canonical artifacts and bounded full fetches

## Context

June's agent already persists generated images and videos in
`agent_artifacts`, with stored session and run ownership, MIME type, path, byte
size, availability, and creation time. June Desktop renders those same rows in
agent chat. Companion chat previously carried only text and status, so a
generated result could not appear on the phone.

Inlining media in ordinary companion frames would exceed the 44 KiB plaintext
cap and make status/history pagination unpredictable. Creating a second
thumbnail or export store would also introduce another artifact truth and a
new retention lifecycle.

## Decision

- A **companion media result** is a bounded metadata reference to an available,
  tool-produced image or video in `agent_artifacts`. Paths never cross the
  protocol.
- Agent history and status events may carry up to eight references. Fetching
  bytes is a separate `mediaFetch` request under the additive `mediaRead`
  capability.
- A fetch addresses an opaque artifact id within a stored session. Desktop
  rejects non-media, unavailable, empty, oversized, symbolic-link, or
  workspace-external files and rechecks the active data-partition boundary
  after file IO before returning bytes.
- Desktop transfers only the canonical full artifact. It does not persist a
  companion thumbnail tier. The phone may derive a local thumbnail after
  verifying the full file.
- Source artifacts are limited to 100 MiB. A response carries at most 31 KiB
  of raw bytes plus offset, total size, completion, and the same SHA-256 digest
  on every chunk. The phone verifies the assembled file before rendering or
  saving it.
- A memory-only transfer cache retains at most 16 validated open file handles
  for 30 idle minutes. Holding the handle pins the inspected file identity
  across path replacement and avoids hashing the same transfer on every
  chunk. It stores no second byte copy; a deleted source may continue occupying
  disk until its cached handle expires.
- Existing generation policy remains authoritative. Fetch cannot initiate
  generation or bypass image safe mode or video consent.

## Consequences

The companion reuses the artifact the user sees on the Mac, and the blind
relay continues to receive only routing metadata and bounded Noise ciphertext.
A linked phone can make a new durable copy in its photo library; that copy has
the phone's user-controlled retention rather than June Desktop's artifact
retention.

Full-only transfer can use more bandwidth than a thumbnail-first design and
cannot show a canonical preview before download completes. That cost is
accepted to avoid derivative-cache ambiguity. A future thumbnail tier must
define its own canonical identity, integrity, and retention before changing
the wire contract.

## Superada pela change desvincular-servicos-open-software

Neste fork o app desktop não tem mais o companheiro (pareamento, transporte, aprovações remotas do Computer Use e a aba "Linked devices" foram removidos em 2026-10-21). O relay do Clovy API e os crates `crates/clovy-companion-*`/`june-companion-*` ainda existem e saem numa change futura.
