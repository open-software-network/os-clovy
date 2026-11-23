# ADR 0013: Stream inference responses through June API instead of buffering

Date: 2026-07-08
Status: accepted

## Context

June API sits inside a TEE behind an nginx-based ingress (dstack-ingress) and
Phala's gateway. Until this change, every inference response was fully
buffered server-side: the agent chat route (`/v1/chat/completions`) read the
entire upstream Venice body — even when the client had sent `stream: true` —
and note generation (`/v1/notes/generate`) returned one JSON envelope after
the upstream call completed. While an upstream inference ran, June API emitted
zero response bytes.

An idle response is exactly what reverse-proxy read timeouts kill. Long
prompts reliably failed with nginx's `504 Gateway Time-out` before June API
ever answered; the embedded Hermes runtime retried three times into the same
wall (JUN-225). Raising proxy timeouts was rejected as the primary fix: the
dstack-ingress image exposes no read-timeout knob in our compose file today,
the outer Phala gateway's limits are not ours to configure, and any fixed
ceiling just moves the cliff — a longer prompt still falls off it.

Buffering was not accidental. Metered billing settles from the upstream
`usage` block (ADR-less but documented in docs/june-api-prd.md: authorize →
upstream → charge), and the buffered body was where usage got parsed. The
groundwork for streaming metering already existed — `stream_options.
include_usage` was injected into streamed requests so the final SSE frame
carries usage — but no code path actually streamed.

## Decision

Keep response bytes flowing for every long-running inference call, and settle
charges after the stream ends.

- **Agent chat (`stream: true`)**: June API forwards upstream SSE bytes as
  they arrive. The provider pump accumulates the body on the side and
  resolves token usage from the final SSE frame once the upstream ends; the
  service settles the charge then. Both chat paths settle under an
  attempt-scoped idempotency key
  (`agent_chat:{user}:attempt:{operation_id}:{body_digest}`, the image key
  scheme): a retried identical body runs a fresh upstream completion that
  delivers its own content, so it must settle its own charge — a
  content-derived key let that second settlement replay as a no-op. Unlike
  images there is no retry-dedup ledger in front, deliberately: chat has no
  client-supplied request id, and any retry that reaches the upstream does
  real billable work. While the upstream is silent (long prompt evaluation), June
  API emits SSE comment heartbeats (`: keep-alive`) so neither proxy hop sees
  an idle response. Buffered (`stream` absent/false) behavior is unchanged.
- **Note generation**: `POST /v1/notes/generate` gains an opt-in
  `"stream": true` request field. The response becomes `text/event-stream`:
  comment heartbeats while generation runs, then a single terminal event
  (`event: result` carrying the exact buffered JSON envelope, or
  `event: error` carrying `{status, body}` of the exact buffered error).
  Old clients never send the flag; a not-yet-updated June API ignores the
  unknown field and answers buffered, so the desktop client branches on the
  response content type. Wire contracts stay backward compatible in both
  directions.
- **Billing on interrupted streams** (deliberate product calls): if the
  client disconnects mid-chat-stream, the provider keeps draining the
  upstream to capture the usage frame and the charge settles normally —
  matching buffered semantics, where a disconnect after send still charges.
  A stream that completes cleanly but carries no usage frame settles at the
  flat authorize estimate (clamped to the hold cap) with a loud error log:
  content was delivered and June does not silently absorb upstream cost, per
  the pricing policy in the June API PRD. A stream that dies on a transport
  failure settles NOTHING — the buffered path errors before its charge line
  on the same failure, and the two must not diverge on error paths. One
  exception: when June API's own metered-client timeout cuts a stream that
  had already delivered bytes (the cap that keeps settlement inside the hold
  TTL), that is a deliberate cut of delivered content and settles at the
  flat estimate like a missing usage frame. Generate
  disconnects cancel the whole call uncharged (all-or-nothing delivery,
  parity with the buffered handler being dropped).
- **Hold TTLs cover the settle-after-upstream window**: agent chat and note
  generation moved onto the same bounded metered-inference client the image
  path uses (route timeout minus the authorize + settlement budgets), and
  their shared authorization hold was raised from 300s to 540s (window +
  settlement budget, inside the OS Accounts 600s cap) with a config-load
  validation pinning the relationship. Before this, a 300-600s call —
  unreachable while the ingress killed everything at its read timeout, but
  exactly what streaming enables — would settle against an expired hold:
  June pays the upstream, the charge fails, and the user sees
  `metering_provider_failed`.

## Consequences

- Long-prompt chat and long-transcript note generation no longer depend on
  any proxy timeout being larger than inference time; first bytes flow within
  seconds regardless of prompt size.
- A streamed response can no longer change its HTTP status after failure
  mid-stream: agent chat clients see a truncated SSE body, and the generate
  endpoint reports errors inside the `event: error` payload with the status
  embedded. Clients must map that embedded status exactly as they map a
  buffered non-2xx response.
- The route-level tower timeout (600s) now bounds only the handler future
  (time to response headers); the streamed body is bounded by the
  metered-inference client's total timeout (route minus the authorize +
  settlement budgets) instead.
- The client-facing chunk channel is unbounded so the provider can always
  drain the upstream to its usage frame at upstream speed — settlement must
  never be hostage to a slow reader. Worst-case memory equals the full
  response body, the same profile buffering had.
- Settlement for a streamed response happens in a spawned task after bytes
  were delivered. June API is stateless (no durable pending-charge ledger —
  the same property the buffered path has for a crash between upstream
  completion and charge), so a process death mid-stream loses that charge;
  the hold expiring protects the user, June absorbs the upstream cost. A
  durable settlement ledger is a deliberate non-goal here.
- Other buffered inference paths (`/v1/transcribe`, `/v1/dictate`,
  `/v1/generate` legacy shape, image generation) remain exposed to proxy
  read timeouts if their upstream calls run long. They keep working today
  (audio calls are chunk-bounded, image budgets fit); if one starts timing
  out, extend it with the same opt-in SSE envelope rather than a timeout
  bump.

## Addendum - 2026-10-08 (upstream is OpenRouter)

Venice is retired as the text upstream (OpenSpec change
`replace-venice-with-groq-openrouter-fal-tavily`); `/v1/chat/completions` and
`/v1/notes/generate` stream from OpenRouter's `/api/v1/chat/completions`. The
client contract of this ADR is unchanged: opt-in `stream: true`, SSE
passthrough for chat, the `event: result` / `event: error` envelope for notes,
and settlement from the usage frame.


## Addendum - protocol failures and response evidence (2026-10-11)

HTTP 200 and a clean transport EOF do not establish a successful completion.
OpenRouter can send an error as its first and only SSE event after committing
headers. The stream pump reads complete SSE events (including multiline data,
comments and CRLF/CR framing) and recognizes a non-null top-level `error` or
`choices[].finish_reason: "error"` as failure. Like a transport fault, this
releases the authorization with a zero-credit settlement, including when
partial output preceded the error or the error carries usage. A wholly failed
response never falls through to the flat estimate.

A successful stream with valid usage retains normal usage settlement. Without
usage, the flat fallback requires delivered response content (text, refusal,
reasoning, audio or tool-call output), not just role/empty choices, comments or
`[DONE]`. The deliberate client-timeout exception likewise requires response
content and no protocol failure. Heartbeat framing, service-funded disconnect
draining and BYOK cancellation/no-metering remain unchanged.


## Addendum - 2026-10-28: direct providers only

The server inference-streaming decision is superseded for this fork by
[ADR-0073](0073-desktop-calls-user-providers-without-a-clovy-server.md). The desktop calls the user's providers directly; chat stays
on CLI engines and Clovy tools remain local through MCP. The server is removed,
not replaced or deployed with a breaking contract. Earlier decisions above
remain as historical records; current behavior follows ADR-0073 and ADR-0072.
