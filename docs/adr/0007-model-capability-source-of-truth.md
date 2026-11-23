---
status: accepted
date: 2026-07-01
---

# Model capabilities come from the live Venice catalog, not traits

Whether a model supports **tools** (`supportsFunctionCalling`) or **image input**
(`supportsVision`) is derived only from the **live Venice model catalog's
`capabilities`**, harvested generically by June API's `collect_capability_names`
(which flattens the capability object to the keys whose value is boolean `true`).
The frontend checks (`modelSupportsTools`, `modelSupportsImageInput` in
`src/lib/model-privacy.ts`) match only these precise, normalized capability
names — never the model's marketing `traits`, and never a model-name heuristic.

## Why

- `traits` is a descriptive/marketing field that **conflates image *output*
  (creative/generation models) with image *input* (vision)**. Matching it is
  actively wrong: a loose `"multimodal"` match once selected a non-vision model
  (Fable 5) as the image fallback (JUN-165), and a wrong tools match bricks the
  agent (prompts run but no tool ever executes).
- Harvesting **whatever bool-true keys Venice returns** (no allowlist) means new
  capability flags appear without a June code change.
- The catalog is fetched at boot and **extends** a built-in fallback, so the
  authoritative flags track the provider.

## Trade-off

- June **trusts the catalog's flags**. If Venice mis-flags a genuinely
  vision-capable model as `supportsVision:false`, the fix is on the **data**
  side (the catalog) — do **not** re-loosen the matcher back to `traits` /
  `"multimodal"`, which reintroduces the JUN-165 bug.

## Consequences

- The image fallback filters by `supportsVision && supportsFunctionCalling`
  (the agent needs tools too) and prefers a known-good vision model.
- The **built-in fallback catalog lives in two places** — `default_pricing()`
  and `config.toml` (which overrides via Figment and ships in the Docker image)
  — kept in sync by tests; a model's vision flag must be correct in both.
- Capability strings are matched normalized (lowercased, non-alpha stripped) so a
  Venice rename (`supportsVision` → `supports_vision`) keeps working.

## Addendum - 2026-10-08 (OpenRouter replaces Venice for remote text)

Venice is no longer a Clovy provider (OpenSpec change
`replace-venice-with-groq-openrouter-fal-tavily`). Remote text capabilities now
come from the OpenRouter catalog (`architecture.input_modalities` and
`supported_parameters`), normalized by Clovy API into the same capability
names the clients already match (`supportsVision`, `supportsFunctionCalling`,
...). The rule of this ADR stands unchanged: capabilities come only from the
catalog data, never from traits, model names, or branding. The desktop's
local-dev Auto vision fallback is `moonshotai/kimi-k2.6` (text and image input,
tools in the public catalog); GLM 5.2 and Nemotron 3 Nano are text-only and are
not treated as vision models.
