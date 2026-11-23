---
status: accepted
date: 2026-07-15
supersedes: 0023
---

# June, Open Software API, and Chat are verified independently

## Context

ADR-0023 coupled June availability to an exact, attested Open Software API
image. That design made routine deployments a coordinated multi-repository
release and could make a healthy product unavailable because another product
changed its image identity. The operational cost and failure mode are not
proportionate to the verification goal.

Users need understandable, inspectable evidence for each product. They do not
need one product to cryptographically vouch for every other product it may use.

## Decision

June, Open Software API, and Chat each publish their own verification surface:

- June publishes its source, signed and notarized desktop releases, and June
  API source, image, and attestation evidence.
- Open Software API publishes its source and fresh, nonce-bound confidential
  compute evidence.
- Chat publishes its source, running image identity, confidential compute
  evidence, and the key bound to its encrypted inference channel.

These surfaces state the claim their evidence supports and its limits. A
product may display another product's reported privacy class or evidence, but
it does not pin that product's release identity or refuse to operate solely
because its source commit or image digest changed.

## Consequences

- Each product can release, roll back, and be inspected independently.
- Past June builds do not depend on an Open Software API digest rotation.
- Verification is easier to explain and less likely to cause an outage.
- This provides transparency and useful evidence, not a transitive end-to-end
  guarantee. Users still trust each selected provider for behavior outside an
  attested boundary.
- ADR-0023 is superseded. Its cross-product startup proof, exact digest pin,
  and coordinated release requirements are not implemented.

Superada pela change desvincular-repo-ci.
