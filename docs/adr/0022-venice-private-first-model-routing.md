# ADR 0022: Venice private first for service-managed text routing

Status: accepted

## Context

June API's production text inference URL points at the Open Software model
routing service. That service can run supported models through Venice private
zero-retention endpoints or through Phala TEEs. June's normal note generation,
dictation cleanup, and agent chat workloads do not require E2EE or TEE-backed
execution. Requiring a TEE for every call adds latency and reduces throughput
without satisfying a product requirement.

The old API contract exposed `provider: "venice"`. Shipped desktop builds rely
on the additive-only `/v1` compatibility policy, and billing settles against
the requested model ID rather than the selected route.

## Decision

- Service-managed text calls send `X-Confidential-Compute: preferred`.
- The routing policy uses Venice private zero-retention as the primary route
  and Phala TEE as fallback. It never falls below zero-retention.
- `required` remains available to callers whose workload truly requires TEE
  execution; June does not request it for its current product workloads.
- Venice BYOK calls remain direct and omit the routing header.
- Existing response `provider` fields retain their Venice adapter semantics.
  The selected provider, privacy level, and endpoint are exposed only as
  additive route metadata and `X-OS-*` response headers.
- Until settlement can use authenticated selected-route pricing, canonical and
  legacy routed model IDs are priced at the most expensive route eligible for
  `preferred`.

## Consequences

This is a deliberate privacy/performance choice, not an accidental loss of a
TEE guarantee. June continues to promise zero retention for these calls, while
Phala remains available when Venice private cannot serve them. Old desktop
builds keep working because routing changes server-side and existing response
fields do not change meaning; deploying this decision requires a June API
release, not a desktop release.

Static fallback-safe pricing may charge more than the selected route costs.
Route-authenticated settlement is a follow-up if precise per-route billing is
needed. Operational rollback is to point June API's service-managed text base
URL back to the direct Venice endpoint or remove the `preferred` header while
the routing service is corrected.

## Addendum - 2026-10-08 (Venice retired; remote text on OpenRouter)

The owner retired Venice as a provider (OpenSpec change
`replace-venice-with-groq-openrouter-fal-tavily`). Service-managed remote text
(chat, note generation, dictation cleanup) now runs on OpenRouter through Clovy
API. The Venice private-first routing above, the `X-Confidential-Compute`
header, and the Venice BYOK exception no longer apply. OpenRouter does not
inherit Venice's zero-retention or TEE route, so Clovy no longer presents these
calls as zero-retention or TEE-backed: clients show only the privacy
properties the catalog actually reports and drop the retired claims. Clovy API
itself still runs in its TEE; that is a separate guarantee. A user's own
OpenRouter key travels as `x-openrouter-api-key` on text routes only, and
Clovy Auto never uses it. Stored notes and transcripts keep the provider they
recorded.
