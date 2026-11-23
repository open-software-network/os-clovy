# Domain Docs

How the engineering skills should consume this repo's domain documentation.

This is a **single-context** repo: one glossary covers the React frontend
(`src/`), the Tauri shell (`src-tauri/`), and the clovy-api backend
(`clovy-api/`). There is no `CONTEXT-MAP.md`.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root — the canonical glossary / ubiquitous
  language. Glossary sections: Platform, Notes, Audio & recording, Agent
  sessions, AI work & billing, Clovy companion (removed), Desktop shell. The `_Avoid_`
  lines are **binding**. Its **Flagged ambiguities** section is the quick
  disambiguation table for the overloaded terms: proxy, transcribe,
  credits, session id, "the model", channel — check it before naming anything.
- **`docs/adr/`** — read the ADRs touching the area you're about to work in.
  ADRs are **append-only**: supersede with a new ADR or dated addendum, never
  rewrite. Numbering: scan for the highest `NNNN-*.md` and increment.

## ADR routing by area

| Working on | Read first |
| --- | --- |
| Releases (the in-app updater and channels were removed in this fork) | 0001 (public releases repo), 0003 (rc channel + promotion); both carry a supersession note |
| Live transcript preview | 0002 (ephemeral, never source of truth) |
| Audio capture, sources, turns | 0004 (out-of-process system audio helper), 0005 (one WAV per source) |
| CLI chat, sandboxing, sessions, tools | 0072 (CLI-only chat and neutral session persistence), 0063 (approval broker), 0058 (MCP relay); 0038/0039 are historical runtime decisions |
| Model picker, capabilities, pricing | 0007 (provider catalog, never `traits`; read the 2026-10-08 OpenRouter addendum) |
| Image generation / editing | 0008 (`/image` fast path + LLM tools) |
| Clovy companion relay and crates (the desktop companion was removed in this fork) | 0041-0048, 0051-0053 (each carries a supersession note) |

## Don't conflate the three doc families

- `CONTEXT.md` + `docs/adr/` — domain language and decisions (this file's
  scope).
- `spec/` — enforceable coding rules (sentence-case, no typographic dashes,
  central icons only, design tokens); violations fail review.
- `specs/` — Spec Kit feature specs; `specs/003-conversation-turns/plan.md`
  doubles as the tech-stack reference.

## Use the glossary's vocabulary

When output names a domain concept (issue title, refactor proposal,
hypothesis, test name), use the term as defined in `CONTEXT.md`. Don't drift
to synonyms the glossary explicitly avoids. If a concept isn't in the
glossary yet, that's a signal — either reconsider the term or note the gap
for `/domain-modeling`. If you sharpen or add a domain term mid-change,
update `CONTEXT.md` in the same change (per AGENTS.md). `CONTEXT.md` states
point-in-time facts (flags, PR status) — verify against code before repeating
them.

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather
than silently overriding:

> _Contradicts ADR-0072 (CLI-only chat), but worth
> reopening because..._
