# AI Self-Identification — Launch Checklist Spec

**Created**: 2026-06-12
**Status**: Draft
**Source**: OpenSoftware Launch Checklist (Google Doc)

---

## Overview

Every consumer-facing interaction that routes through Venice AI must begin with an explicit self-identification from the OS-Agent (June's AI). This is a launch requirement: the agent must identify itself as an AI powered by Venice at the start of every conversation, dictation cleanup, note generation, or other inference-backed interaction. The requirement applies across all modalities — chat, dictation, meeting notes, and agent tasks — wherever the user interacts with AI-generated output.

## Requirements

### REQ-1: Venice AI self-identification prompt in system message

The OS-Agent system prompt must include a self-identification directive that causes the model to identify itself as an AI powered by Venice at the start of every consumer-facing interaction.

**Acceptance Criteria**:

1. The system prompt for every Venice-backed inference call includes a self-identification instruction.
2. The model's first response in a new conversation session identifies itself as an AI powered by Venice.
3. Self-identification is present in all consumer-facing modalities: agent chat, dictation cleanup, note generation, and any future Venice-backed features.
4. Internal / non-consumer-facing calls (e.g., programmatic tool use with no user-visible output) are exempt.

### REQ-2: Consistent identification language

The identification must use consistent, predictable language so users can recognize it across sessions.

**Acceptance Criteria**:

1. The identification phrase is defined in a single, shared constant (not duplicated per modality).
2. The language is plain and non-technical (e.g., "I'm June, your AI assistant powered by Venice" — exact wording TBD by product).
3. The identification appears only once per session, not on every subsequent turn.

### REQ-3: No degradation to latency or UX

Self-identification must not meaningfully degrade the user experience.

**Acceptance Criteria**:

1. For dictation cleanup and note generation, the identification is included in the system prompt, not prepended to the output — the user sees only the cleaned text / generated note.
2. For agent chat, the identification is part of the first assistant message and does not require a separate round-trip.
3. No additional network call is introduced to satisfy this requirement.

## Implementation Notes

- **Where to add the prompt**: The self-identification instruction belongs in the system message template for each Venice-backed inference path. In the codebase, this means updating the system prompts in:
  - Agent conversation (`src-tauri/src/` agent/chat modules)
  - Dictation cleanup (`dictation.rs` / `scribe_api` cleanup path)
  - Note generation (`scribe_api` generate path)
- **Shared constant**: Define the identification instruction string once (e.g., in a shared prompt module or config) and import it in each system prompt builder. Avoid copy-paste across modules.
- **Venice API consideration**: If Venice's API supports a `system` role or equivalent metadata field, prefer that over injecting the instruction into the user message. This keeps the identification out of the visible transcript unless the modality calls for it (chat).
- **Modality-specific behavior**:
  - **Agent chat**: The model's first reply includes the identification naturally as part of its greeting.
  - **Dictation / note generation**: The identification is in the system prompt only; the user-facing output is the cleaned text or generated note without an AI preamble.
- **Testing**: Verify that every Venice-backed call includes the self-identification instruction in its system message by checking the prompt construction in unit tests. For agent chat, also verify the model output includes the identification phrase on the first turn.

## Out of Scope

- Modifying the Venice API itself — this is a client-side prompt requirement only.
- Adding identification to OpenAI-backed calls (only Venice is in scope per the launch checklist).
- Persisting whether a session has already shown the identification (the system prompt always includes it; the model handles deduplication naturally in multi-turn chat).

## References

- OpenSoftware Launch Checklist (Google Doc) — "AI Self-Identification" section
- [`/docs/scribe-api-prd.md`](./scribe-api-prd.md) — Scribe API inference paths
- [`/docs/onboarding-design.md`](./onboarding-design.md) — Agent first-run and honesty screen
