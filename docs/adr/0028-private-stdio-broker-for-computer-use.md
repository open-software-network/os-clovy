# Computer use runs through a private stdio driver broker

## Status

Accepted - 2026-07-15, JUN-278 / JUN-288 / JUN-293 / JUN-296.

## Context

June needs to operate a selected Mac app in the background while keeping the
user's real pointer, keyboard focus, and active Space available. The pinned
`cua-driver-rs` implementation provides the required background capture and
input paths, but its complete MCP surface also includes process launch,
termination, configuration, recording, replay, and update tools. Giving that
surface or its daemon socket directly to Hermes would let the runtime bypass
June's grant, sensitive-target policy, per-action approval cards, and emergency
stop.

The driver is also the process that macOS evaluates for Accessibility and
Screen Recording. A loose command-line binary or an upstream daemon identity
would make TCC attribution dependent on the terminal, development tool, or a
separately installed app. The shipped identity must instead be stable across
June updates and must be exercised as part of the signed release.

The obvious alternatives were:

1. Enable Hermes' upstream `computer_use` toolset and configure its driver
   socket. This is the shortest integration, but policy and approval would no
   longer be an app-owned structural boundary.
2. Expose the complete pinned driver as a June MCP server and rely on the model
   to call only approved tools. This reduces glue code but retains dangerous
   tools and makes approval prompt-level policy.
3. Install or update the driver at runtime. This follows the upstream setup,
   but adds an unpinned network installer and a second app identity outside the
   June release chain.
4. Bundle the driver and put a narrow Rust broker between it and Hermes. This
   costs a maintained adapter, but gives June one enforceable choke point.

## Decision

June uses option 4.

### Signed helper and provenance

- Release tooling compiles June's `june-computer-use-driver` binary against
  the exact `trycua/cua` Git commit in `src-tauri/cua-driver-pin.json`. It never
  downloads or exposes the upstream driver executable, installer, CLI, daemon,
  updater, or complete MCP registry.
- The June-owned helper links only the pinned macOS implementation and
  publishes an explicit allowlist of capture and input tools. Its source,
  `Cargo.toml`, and `Cargo.lock` are fingerprinted into the bundle stamp so a
  stale build cannot be reused after June changes the trust boundary.
- Universal release preparation compiles the arm64 and x86_64 Rust targets and
  merges them into `June Computer Use Driver.app`, with bundle identifier
  `co.opensoftware.june.computer-use-driver`.
- The nested app is signed with June's release identity before Tauri signs the
  outer app. Development builds use an ad-hoc signature.
- The source commit pin, SPDX SBOM, and upstream MIT notice ship inside the
  helper.
- Accessibility and Screen Recording attach to this helper identity. A live
  signed release fixture must prove both grants, capture, background input,
  unchanged frontmost app, and unchanged real pointer before an RC or stable
  macOS release can publish.

### Private driver transport

- Rust is the only production component allowed to launch the helper. It
  starts `june-computer-use-driver mcp` as a private stdio child and supplies a
  fresh 256-bit initialization capability that never reaches Hermes.
- The helper also verifies its direct parent. A development helper accepts only
  this checkout's `target/**/os-june`; a packaged helper accepts only the main
  executable in the containing `June.app`, after validating both app
  signatures, their fixed identifiers, and the same non-ad-hoc signing team.
  Copying the helper under a lookalike app is therefore not sufficient.
- No socket path, driver command, driver environment override, or direct
  driver toolset reaches Hermes.
- Proxy, driver override, updater, telemetry, and inherited CUA environment
  variables are removed before launch. The upstream network update path stays
  disabled.
- The public agent surface is one app-owned MCP server,
  `june_computer_use`, which forwards to an authenticated loopback route with a
  dedicated random token. Provider, recorder, and connector tokens cannot open
  that route.
- Hermes' upstream `browser` and `computer_use` toolsets stay disabled. The
  app-owned MCP server is enabled only when the June grant, Pro or Max plan,
  remote rollout decision, pinned helper, both TCC grants, and a vision-capable
  model are ready.
- The loopback capability is injected only into the visible chat dashboard,
  never the routine gateway. Each submitted visible turn also opens a unique
  in-process attended-run lease; terminal events, Stop, unmount, and revocation
  close it. Naming the MCP server in a routine cannot make it usable.
- Computer use is absent from routine toolsets. V1 is attended and macOS-only.

### Emergency rollout control

- June API serves the backward-compatible public
  `GET /v1/computer-use/rollout` decision. Operators can disable Computer use
  globally, for exact June/macOS versions, or for a trailing-wildcard version
  prefix without shipping another desktop build.
- The desktop sends its real app and macOS versions, caches successful
  decisions for five minutes, preserves a received disable through an outage,
  and fails closed briefly if no decision can be fetched.
- A transition from ready to disabled uses the same native stop path. Direct
  permission requests and direct broker actions recheck the decision, so the
  UI is not the enforcement boundary.

### Broker policy and approval binding

- The broker exposes only capture, app listing/selection, wait, and the narrow
  click, drag, scroll, text, key, and value operations June supports.
- Every operation that can change app state parks in Rust for a separate
  expiring `Allow once` or `Deny` decision. V1 has no approve-all, allow-always,
  or autonomous path.
- An approval is bound to a stable action id, exact process/window/app tuple,
  action summary, capture generation, and relevant capture reference.
- Immediately before execution, the broker lists windows again and privately
  recaptures the target. Element actions require the same accessibility role
  and label at the same numbered index. Coordinate and unscoped key actions
  require the same screenshot digest. Changed targets fail closed and must be
  captured again.
- June, terminals, security/privacy settings, keychains, password managers,
  credential and one-time-code fields, payment fields, destructive shell text,
  clipboard shortcuts, and destructive/system shortcuts are blocked in Rust.
- Stop, grant revocation, permission loss, shutdown, or driver failure kills
  the child, denies all parked actions, invalidates the current target, and
  removes task captures.
- App identity is bound by PID, window id, bundle identifier, and executable
  path. Sensitive-field detection considers the complete capped accessibility
  metadata, and text/value operations require an editable role.

## Consequences

- June maintains a small adapter for the pinned driver's schemas. A checked-in
  contract fixture and release handshake make schema drift a hard failure when
  the pin changes.
- Background behavior is safer than a direct runtime integration, but a driver
  or macOS update still requires a signed live test on the supported OS matrix.
- The helper is a new signed trust boundary and a new TCC identity. Support
  must distinguish the June grant from the two macOS grants.
- Capture data follows the selected model route. It is never telemetry. The
  local approval copy is private to the user, bounded to the latest capture,
  and removed on stop or shutdown.
- A release runner needs an interactive login and pre-granted TCC access for
  the stable helper identity. GitHub-hosted runners can verify packaging and
  schemas but cannot replace the live release gate.
- The signed app has one hidden release self-test mode. It is not a general
  proxy: it accepts only the bundled helper and only the two disposable fixture
  bundle identifiers. This lets the real signed June parent exercise TCC and
  background input without adding a production bypass.

## Addendum - Tauri development launcher identity (2026-07-16)

Tauri's development runner expects the product-name executable `June`, while
Cargo produces the canonical `os-june` binary. The repo runner materializes
`target/**/June` as a byte-identical copy of `target/**/os-june` before launch.
The development helper therefore accepts either exact basename inside this
checkout's Cargo target tree. It continues to reject every other basename and
every path outside that target tree; the peer PID check and fresh initialization
capability remain mandatory. Packaged helper verification is unchanged and
still requires the signed outer `June.app` executable and matching signing team.

## Addendum - policy-brokered app lifecycle (2026-07-16)

The initial allowlist could operate only windows that were already available.
That made an ordinary request depend on the user opening the target app first,
and a Stage Manager shelf thumbnail could be mistaken for the real window. The
broker now owns two narrow lifecycle operations:

- `open_app` accepts only an installed app display name. Rust rejects blocked
  targets, paths, and URLs before approval, then verifies the launched PID,
  bundle identifier, executable path, and returned windows. The helper maps it
  to the pinned driver's background launch path. Its published schema excludes
  file/URL handoff, environment variables, arguments, debugging ports, and
  forced new-process options.
- `focus_app` with `raise_window: true` is the sole foreground exception. It
  requires a separate Allow once decision, revalidates the exact selected
  process/window/app identity after approval, and sends only that PID to a
  helper-local activation operation. The model cannot activate an arbitrary
  process. Selecting a target without `raise_window` remains background-only
  and does not require an action decision.

Window listings preserve bounds and visibility metadata. A tiny Stage Manager
shelf surface fails capture with a structured instruction to use the approved
restore path instead of treating the thumbnail as the document. Normal capture
and input retain the no-cursor, no-focus, and no-Space-change contract. Bringing
a window forward is allowed only when the user requested it or the selected
window must be restored, and the approval card states that foreground change.

The agent-facing schema names the capability only as Computer use. It instructs
the runtime to invoke mutations immediately and wait on June's native approval
decision, never to request a textual `yes` or expose the internal transport in
conversation.

## Addendum - task-scoped app authorization and current-stage restore (2026-07-16)

This addendum supersedes the per-action approval binding and separately
approved `focus_app` restore described above. Repeated prompts made ordinary
attended editing feel like a sequence of unrelated permissions even though the
user had already chosen the app and task.

The broker now asks once before the first access to each target app in an
attended task. Existing apps are keyed by verified bundle identifier and
executable path. A display-name authorization used for `open_app` is paired
with that verified identity immediately after launch. Captures and mutations
for the same identity then proceed without another decision until the final
run lease ends. Stop, revoke, readiness loss, shutdown, or the start of a new
task clears all app authorizations. Blocked targets, sensitive-field policy,
exact-window capture binding, and pre-mutation stale-target revalidation remain
enforced for every operation.

Stage Manager grouping has no public application API; AppKit activation can
switch to the target's existing group instead of joining it to June. The
private helper therefore exposes one narrower `join_current_stage` operation
instead of arbitrary process activation. Rust first brings June's main window
to its stage. The helper focuses the exact target PID/window without Space
follow using the already pinned SkyLight path, then performs `AXRaise` on the
matching Accessibility window. The broker re-lists the exact window and
accepts the result only when it is no longer a shelf-sized or off-current-Space
surface. Failure is explicit and closed; there is no fallback that switches
Spaces or moves the real pointer.

WindowServer can publish a Stage Manager app twice: a full-size hidden window
that maps to the real Accessibility window, plus a small titled shelf proxy
that does not. The broker collapses that pair to the real window before target
selection, treats the hidden window as requiring restore, and never exposes
the proxy as an operable target. A failed restore is terminal for that window
during the current task so the runtime cannot alternate between the proxy and
the hidden window.

This keeps the user-visible contract simple: one authorization to use an app
for the current task, no separate prompt to restore it, and June plus the
target window in the same Stage Manager group when restoration succeeds.

## Addendum - isolated development app names (2026-07-17)

Parallel agent worktrees need distinct development app names and bundle
identifiers so macOS, Tauri, and the user can distinguish them. Issue branches
owned by a supported harness therefore launch as `June JUN-<number> Codex` or
`June JUN-<number> Claude` while the normal development identity remains
`June`.

The Computer use helper accepts those names only inside this checkout's Cargo
target tree and only when the launcher is the same filesystem object as the
canonical `os-june` binary. The development runner creates that launcher as a
hard link after every build. A copied or independently built executable with
an allowed-looking name is rejected. Packaged helper verification remains
unchanged.

## Addendum - virtual action cursor (2026-07-21)

This addendum clarifies the earlier no-cursor wording: Computer use must never
move or follow the user's real pointer. Once an attended task sends its first
valid, eligible Computer use action, June may show a separate, semi-transparent
Computer use cursor at the virtual action position. The cursor is a
nonactivating, click-through panel and does not change the background input,
focus, or active-Space invariants above.

The signed helper remains the only process that resolves Accessibility element
centers. It sends June a private pointer notification for click positions and
drag endpoints; the main process converts the screenshot-local coordinates to
screen coordinates using the selected window's complete bounds. Every show and
move is bound to the current task epoch. Stop increments that epoch and hides
the panel before helper teardown so queued notifications cannot make an old
cursor reappear.

## Addendum - macOS Screen Recording responsible app (2026-07-21)

This addendum corrects the earlier statement that both TCC grants attach to the
nested helper identity. Accessibility attaches to `June Computer Use Driver`,
but macOS attributes Screen Recording to the signed outer June app that launches
the helper. The helper's preflight observes that responsible-app grant, and its
native request creates the outer June entry in System Settings.

June therefore requests the next missing grant before opening its matching
privacy pane. Accessibility setup names and, when needed, offers the nested
helper as a drag source. Screen Recording setup names and offers the signed
outer June app as its separate drag source; it never tells the user to add the
helper to that list. The private transport, helper parent authentication,
capture execution, and app-owned broker policy are unchanged.

## Addendum - agent-run-scoped tool exposure and driver prewarm (2026-07-23)

Under June's default profile, an explicit request to use Computer use narrows
the Hermes agent snapshot to the single `june_computer_use` MCP server for that
agent run. The desktop may only subtract names from the gateway process
allowlist; the gateway rejects an attempt to add a toolset. Named profiles keep
their own tool policy and reject this launch-profile scope. It waits only for
requested local MCP servers while unrelated MCP discovery continues in the
process-wide background. A later ordinary agent run restores the normal June
allowlist by refreshing the existing agent snapshot, without rebuilding the
agent or reconnecting MCP clients. Queued prompts with different scopes do not
merge. The optional compute-host frame cannot carry the restriction, so a
session that uses the scope stays on its inline executor thereafter. This
preserves one owner for history, steering, and interruption instead of switching
between stale live agents. The separate broad slash-command worker remains lazy
for a scoped session and starts only if the user later dispatches a slash
command.

June deliberately does not use Hermes' deferred tool-search bridge for this
path. Computer use must remain a direct app-owned tool with its native approval
and attended-run semantics visible to the model. The upstream
`macos-computer-use` skill is also disabled: its driver instructions describe a
different trust boundary and duplicate the concise workflow already present in
June's tool schema.

Once every readiness gate is already satisfied, June starts the persistent
private driver off the first-action path. App startup, enabling Computer use,
status refresh, and main-window focus all enter the same driver mutex and cache
used by a real action, so concurrent attempts remain single-flight. A captured
task epoch makes Stop, grant revocation, or readiness loss win over an in-flight
prewarm. Normal attended-lease cleanup clears every attended lease, pending
approval, target, app authorization, capture, and visible companion state, but
retains the initialized no-authority driver as the next attended task's prewarm.
Explicit Stop, grant revocation, readiness loss, and shutdown still invalidate
its epoch and terminate it. Authorization, target validation, rollout, plan,
model, billing, and approval behavior remain unchanged.

2026-07-27 addendum: ADR-0040 supersedes this ADR's public `june_computer_use` MCP-server integration shape; the brokered-helper isolation pattern and Computer use product scope remain binding.

## Addendum - local gates only (2026-10-18)

[ADR-0067](0067-computer-use-depends-only-on-local-gates.md) removes the Pro or
Max plan, the remote rollout decision and the vision-capable model from the
readiness gates, and takes `computer_use` out of the native agent's catalog.
The broker, pin, lease, per-app authorization and per-action approval above
stay binding.
