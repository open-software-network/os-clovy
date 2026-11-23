---
status: accepted
date: 2026-07-22
---

# Replace the embedded Hermes runtime with a June-owned agent harness

## Context

June currently embeds a pinned Hermes checkout, a relocatable CPython runtime,
two mode-specific gateways, a WebSocket control plane, locally patched approval
semantics, and a broad compatibility suite. This let June reach a capable agent
experience quickly, but the runtime now owns behavior central to the product:
sessions, streaming, tool dispatch, approvals, skills, persistence, and
lifecycle events.

That boundary makes ordinary product changes depend on upstream protocol and
configuration behavior. It also adds substantial installer weight, startup
work, idle processes, release signing work, and compatibility maintenance.

## Decision

June will own the agent harness and use the TypeScript OpenAI Agents SDK for the
model and tool loop.

- A signed local TypeScript service runs as one trusted child process and talks
  to the Tauri host over versioned, newline-delimited JSON-RPC on stdio.
- The trusted service owns orchestration only. Rust remains the authority for
  secrets, June API access, persistence, tool execution, path validation,
  safety modes, approvals, and artifacts.
- Model requests continue through June API's existing metered Chat Completions
  route. Direct OpenAI credentials and OpenAI-hosted trace export are not part
  of this design.
- June's SQLite database becomes the source of truth for sessions, runs,
  ordered items, interruptions, artifacts, skill settings, and local traces.
- The harness is not placed inside the execution sandbox. Sandboxed file and
  command capabilities are tools implemented by the trusted host. On macOS,
  command execution keeps the Seatbelt write-jail. On Windows, host file tools
  enforce workspace roots and sandboxed shell execution is unavailable until a
  real operating-system isolation boundary exists.
- Existing Hermes sessions are imported once from the preserved read-only
  state database. The old Hermes home remains recovery data but is never
  started or read by the new runtime after a successful import.
- The production cutover is atomic. June does not ship a Hermes fallback or a
  dual-runtime feature flag.

This decision supersedes ADR-0006, ADR-0009, ADR-0011, ADR-0025, and ADR-0029.
ADR-0018, ADR-0027, ADR-0028, ADR-0031, and ADR-0032 remain binding at the June
product boundary and are reimplemented against the June-owned runtime.

## Consequences

- June can change agent instructions, tools, state, approvals, and presentation
  without waiting for or patching Hermes.
- The desktop bundle no longer carries Hermes, CPython, its dashboard, or its
  compatibility patch set. Release jobs instead build and sign a Node 24
  single-executable service for each desktop target.
- The stdio protocol and persisted item schema become load-bearing June-owned
  contracts. Both are versioned and covered by fixture tests.
- The cutover restores June-owned routines, branching, live steering, manual
  compaction, and run usage. A live instruction is injected at the next model
  boundary and persisted as visible history; if the active run settles before
  consuming it, June submits it as the next turn instead. Messaging channels,
  background delegation, and raw Hermes runtime debugging remain intentionally
  retired. Browser use, Computer use, private connectors, project memory,
  profiles, attachments, and generated media remain June capabilities and
  continue through Rust-owned tools.
- Performance is an acceptance criterion rather than an assumption. The PR
  records before-and-after installer size, runtime-ready latency, idle memory,
  send acknowledgement, first visible stream event, cancellation latency, and
  time to first model token.

## Cutover baseline

The first local release build on an Apple Silicon Mac establishes the following
baseline for the new sidecar. These measurements are not presented as a Hermes
comparison because the old and new runtimes were not measured under an
equivalent harness.

- Node 24 single executable: 120,790,720 bytes
- Sidecar resident memory after initialization: 98,762,752 bytes
- Process spawn to initialized response: 74.02 ms
- Initialization request round trip: 72.47 ms
- First protocol frame: 73.93 ms
- Graceful shutdown request round trip: 0.27 ms
- Complete smoke-test process lifetime: 78.12 ms
- Unsigned local macOS application bundle: 160 MB

The benchmark is reproducible with `pnpm agent-runtime:benchmark` after the
sidecar has been built. Release CI separately verifies signing, checksums,
startup from a path containing spaces, and the absence of Hermes or Python
payloads.

## Alternatives considered

- Continue patching and upgrading Hermes. Rejected because it preserves the
  product-iteration and packaging costs that motivated the change.
- Run the harness in June API. Rejected because local files, private
  connectors, approvals, and execution should remain on the user's device.
- Run the Agents SDK in the webview. Rejected because the webview must not own
  service credentials, unrestricted tools, or persistence authority.
- Implement the entire agent loop in Rust. Rejected because it would duplicate
  a maintained SDK while adding a slower path to feature parity.

## Addendum (2026-10-15)

Superseded in part by
[ADR-0062](0062-cli-engines-are-the-default-agent-native-harness-is-legacy.md):
Claude Code or Codex is now the default engine of new agent work; this
harness stays as the legacy Clovy engine until its tools reach the Clovy MCP
server.
