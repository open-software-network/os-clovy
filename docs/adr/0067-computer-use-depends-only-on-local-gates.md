---
status: accepted
date: 2026-10-18
amends: ADR-0028
---

# Computer use depends only on local gates

## Context

[ADR-0028](0028-private-stdio-broker-for-computer-use.md) made Computer use
"ready" only when the user's grant, a Pro or Max plan on the Clovy account,
the remote `GET /v1/computer-use/rollout` decision, the pinned helper, both
macOS permissions and a vision-capable generation model in the Clovy API
catalog all held. Those last three existed because the native agent sent
captures to Clovy's own generation model and billed the user's Clovy plan.

Chat now runs on agent CLIs ([ADR-0062](0062-cli-engines-are-the-default-agent-native-harness-is-legacy.md)).
In a CLI turn the capture goes back to the CLI's own model through
`clovy-mcp` ([ADR-0063](0063-cli-turns-get-write-tools-through-an-approval-broker.md)),
so the plan and the Clovy vision model gate nothing real, and the fork has no
production rollout operator. The owner asked on 2026-10-18 that Computer use
not depend on anything outside the Mac (openspec change `runtimes-cli-na-ui`).

## Decision

1. **Ready is local.** Status and the grant depend only on the user's
   Computer use grant, the pinned bundled driver (signature and version) and
   the macOS Accessibility and Screen Recording permissions. The states
   `plan_required`, `model_unsupported` and `rollout_disabled` are no longer
   emitted, and `planEligible`, `modelSupportsVision` and `generationModel`
   leave the status payload. `computer_use_request_permissions` and action
   eligibility drop the same checks.
2. **Only CLI turns with `clovy-mcp` get the tool.** `computer_use` leaves the
   native agent's tool catalog and its dispatch; the native-agent eligibility
   branch (`ActionOrigin`) goes with it. CLI turns keep the turn lease, the
   per-app authorization and the per-action approval card.
3. **No remote kill switch on the desktop.** The desktop no longer calls
   `/v1/computer-use/rollout`; the route stays in Clovy API for older clients
   ([ADR-0021](0021-june-api-v1-compatibility-policy.md)).

The private stdio broker, the driver pin, the attended-run lease, per-app
authorization, the approval cards, Stop and the driver prewarm of ADR-0028
are unchanged.

## Consequences

- No capture goes to a Clovy API or OpenRouter model because of Computer use;
  it goes only to the CLI running the turn. The Plugins privacy note says so.
- A user without a Clovy account can use Computer use from a CLI turn.
- Operators can no longer disable Computer use per app or macOS version from
  Clovy API; a bad driver build is stopped by shipping a new pin. The user can
  turn it off locally and every action still asks for approval.
- The native agent, while it remains as the legacy engine, cannot operate the
  computer.
