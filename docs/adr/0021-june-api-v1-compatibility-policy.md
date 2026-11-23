# ADR 0021: June API /v1 compatibility policy for shipped clients

Date: 2026-07-14
Status: accepted

## Context

June API deploys continuously: every merge to main that touches `june-api/`
builds an image and auto-deploys it to staging, and a manual promote re-tags
that image to production. Desktop releases move on a completely separate
cadence: DMGs and installers are cut from a version bump, ship through the
updater, and then live in the wild for weeks. Every stable build keeps
calling the same production `/v1` origin with the request shapes, response
expectations, and error-code numbers that were compiled into it at release
time.

Nothing defended that contract. The wire types are hand-written on both
sides (`src-tauri/src/june_api.rs` vs `june-api/crates/api/src/handlers/`),
the server tests exercised only the server's own current DTOs, and the
client did not send its version, so the server could neither observe nor
gate old builds. We shipped production incidents where an API change worked
for the latest RC but broke the previous stable app version, and CI was
green the whole way.

## Decision

`/v1` is a frozen, additive-only surface. A deployed June API must accept
every request an in-support stable app version sends and answer with every
field that version reads.

Rules for changing anything under `/v1`:

- New request fields must be optional: `Option<T>` or `#[serde(default)]`
  on the server, and older servers must tolerate their absence. New
  multipart fields follow the same rule (optional, ignored when absent).
- Never remove or rename an existing request or response field, and never
  change a field's type or JSON casing. Additive response fields are fine;
  old clients ignore unknown keys (`deny_unknown_fields` stays banned on
  `/v1` request DTOs).
- Never renumber or reuse an error code. The registry is pinned in
  `june-api/crates/api/tests/fixtures/client-contract/error-codes.json`;
  clients hardcode these numbers forever. New codes get new numbers.
- Never tighten validation on an existing field (length caps, allowed
  values, newly-required non-empty). Relaxing is fine. If an old value must
  stop being served (a retired model id, an unknown app context), map it to
  a working fallback server-side rather than rejecting it; see
  `resolve_priced_text_model` and `recognized_app_context` for the pattern.
- A genuinely breaking change gets a new endpoint (or, if broad, `/v2`)
  while `/v1` keeps serving in-support versions until they age out.

Enforcement is the client contract suite
(`june-api/crates/api/tests/client_contract.rs`). Each directory under
`tests/fixtures/client-contract/` snapshots one shipped stable version:
the exact requests it sends (including fields that ride as JSON `null`)
and the response fields its DTOs require. The suite replays every snapshot
against the real router on every PR, and the production promote workflow
re-runs it against the exact commit being promoted before the deploy job
starts. Promotes always replay the fixture registry as of the workflow's
own ref, overlaid onto the promoted commit: a rollback target predates
fixtures added for versions shipped since, and must still prove it serves
those clients before it reaches production. A red contract test means
"this breaks a shipped app version"; the fix is to change the API change,
not the fixture.

Fixture lifecycle:

- When a new stable desktop version ships with any change to what it sends
  or reads, copy the newest fixture directory to `v<new-version>` and edit
  the delta. Versions whose wire behavior is identical to an existing
  snapshot do not need their own directory. Fixtures replay exactly what
  the released build sends, headers included: v0.0.33 predates
  `x-june-app-version`, so its fixtures carry no such header, and versions
  released with the header pin it via the fixture `headers` map.
- The client sends `x-june-app-version` (its real version, from the crate
  version kept in lockstep with `tauri.conf.json`) on every June API
  request. Server logs segmented by that header are the retirement signal:
  a fixture directory may be deleted only when production traffic from that
  version and everything older has gone to zero, with a PR note to that
  effect.
- Support window: every stable version still writing production traffic is
  in support. There is no time-based cutoff while the auto-updater cannot
  guarantee every install has moved.

## Consequences

- An API change that would break a shipped build now fails PR CI and the
  promote gate instead of production. The failure names the app version and
  the exact field.
- Adding a `/v1` capability costs one habit: new fields are optional, and
  a new stable release with wire changes adds a fixture directory.
- The fixtures double as executable documentation of what each shipped
  version actually sends, which the duplicated client/server types never
  provided.
- Version-segmented server logs make "did anyone on 0.0.x hit this" a
  query instead of a support-ticket archaeology exercise, and give us the
  option of a server-side minimum-version gate with a clean error later.
- The suite does not verify the client side of the contract (that the app
  parses what the server sends); that remains covered by the client's own
  tests. It also cannot catch semantic changes that keep the same shape.

## Addendum - local provider cutover and retired credentials (2026-10-08)

The owner selected a clean Venice removal for local development: Groq audio,
OpenRouter remote text, fal media, and Tavily web. The updated client uses
the new provider credentials; a Venice key cannot authenticate to those
vendors and is never copied, forwarded to them, or silently ignored in
favor of a charge to Clovy credits.

The API rejects a non-empty provider-key header that does not belong to the
route through the common credential boundary. This includes the retired
`x-venice-api-key`, which receives `venice_api_key_not_accepted` before
funding or forwarding; there is no Venice execution branch or compatibility
alias. This is a semantic break for a client that still sends its old
Venice key, not evidence that every shipped client remains compatible.

Request/response bodies, casing, the numeric error registry, and historical
client-contract fixtures remain subject to the original policy. Retired
model IDs use the configured working defaults rather than reactivating
Venice; old web search selector fields are accepted as unknown input and
do not select an old provider. Passing those fixtures does not prove the
retired-key case is compatible.

This records the local cutover and its safety boundary, not permission to
publish a breaking production API. A publication/support decision is
required before any deployment that affects in-support clients. The
accepted production compatibility policy above and its historical text
remain unchanged.


## Addendum - 2026-10-28: direct providers only

The additive-only server compatibility requirement is superseded for this fork by
[ADR-0073](0073-desktop-calls-user-providers-without-a-clovy-server.md). The desktop calls the user's providers directly; chat stays
on CLI engines and Clovy tools remain local through MCP. The server is removed,
not replaced or deployed with a breaking contract. Earlier decisions above
remain as historical records; current behavior follows ADR-0073 and ADR-0072.
