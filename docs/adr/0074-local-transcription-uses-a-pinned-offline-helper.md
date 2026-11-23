# ADR-0074: Local transcription uses a pinned offline helper

Status: accepted. Date: 2026-10-29.

## Context

Dictation, saved notes, chunks, turn packs and live preview need one selectable
local engine without changing Groq's direct-provider contract (ADR-0073).
Core ML inference needs Apple Silicon/macOS 14+, substantial memory, and a
Swift runtime. Model installation must be explicit, resumable and inspectable.

## Decision

Use Parakeet Ultra through a separate SwiftPM helper pinned to FluidAudio
v0.18.0. Rust owns model download/hash verification, helper lifetime, serialization
and the backend choice. The Swift helper only loads the supplied directory using
`loadLocal`; it never fetches models. Pin the HF commit and every file hash.

Choose the backend before reading provider credentials. Local errors and missing
models fail locally, with no Groq fallback. This follows the change's top-level
privacy requirement over the contradictory fallback sentence in design D6.
Groq remains the default on unsupported platforms. Removing an active local model
explicitly persists Groq and warns the user; simply failing a Local request does not.

Keep the existing 600-second chunking, durable jobs and turn-pack split. Use token
times and a 400 ms pause boundary for packs. Local has no Groq upload cap or pacing.
Warm and reuse one serialized helper, then terminate it when Local is deselected,
the model is removed, or the app shuts down. Text generation/cleanup stays separate.

## Alternatives and consequences

A Rust wrapper that downloads models implicitly would bypass installation consent,
progress, verification and the profile directory. Core ML inside the app would
couple inference memory and crashes to the desktop. Implementing the decoder in
Rust would duplicate the runtime's signal processing and decoding. The helper
avoids those costs at the price of a pinned Swift dependency, a JSON-lines protocol,
and process lifecycle tests. Windows and Intel remain Groq-only. Initial loading
cost and memory are visible trade-offs; real Portuguese quality and warm latency
must be proven separately from fake-helper contract tests.

## Clarification, 2026-10-29: installation receipt

Design D5 deliberately treats `installed.json` as a receipt for files verified at
installation. Startup compares the pinned manifest and file sizes, rehashing only
when that receipt is missing or a size differs. This avoids reading and hashing
632 MB on every launch. External edits that preserve file sizes are not detected
by this readiness check; inference failures remain local and never switch to Groq.
The recovery verification runs on a worker, serialized with installation/removal.

## Addendum, 2026-10-29: individual Local note turns

The synthetic three-turn regression with token ends 160 ms late confirmed the
design's pack-boundary risk: a 400 ms grouping threshold can bridge the 500 ms
pack gap, leaving two turns empty and assigning all text to the middle turn.
Use the alternative already accepted in Risks / Trade-offs: one Local request
per note turn. Keep the timed adapter's 400 ms rule, 600-second chunking, durable
jobs and retries. Groq packing is unchanged. This trades fewer helper calls for
correct attribution; all Local calls still reuse the resident model without
network access or provider fallback.
