---
status: accepted
date: 2026-10-15
supersedes: ADR-0058 ("everything is read-only", for turn-bound connections only); ADR-0040 ("June ships no MCP server of its own", for CLI chat engines only)
---

# CLI chat turns get Clovy's write tools through the MCP server and an approval broker

## Context

The owner decided on 2026-10-15 that everything the native agent does moves to
the CLI chat engines (Claude Code and Codex), and that the native agent is then
removed. The CLI engines reach Clovy only through `clovy-mcp`
([ADR-0058](0058-clovy-mcp-server-relays-to-the-running-app.md)), which is
read-only, stateless, and does not know which session a call belongs to. A CLI
turn runs in print mode and cannot ask the user anything mid-turn; an action
that needs approval fails today. More than fifty capabilities (memory writes,
media, recorder, Browser use, Computer use, routines, connectors, user MCP
servers) exist only as native host tools
([ADR-0040](0040-plugin-capabilities-as-host-tools.md)), whose safety depends
on the Agents SDK approval interruption.

Alternatives considered:

- **Pass the CLI pre-granting flags** (`--dangerously-skip-permissions`,
  `--dangerously-bypass-approvals-and-sandbox`). Rejected: removes the human
  approval boundary for every action, including ones reachable by prompt
  injection from notes and transcripts.
- **Give each capability its own MCP server or pass connector credentials in
  the CLI's `--mcp-config`.** Rejected: credentials would leave the app,
  breaking [ADR-0039](0039-june-owned-routines-and-mcp.md) and
  [ADR-0050](0050-linear-official-hosted-mcp.md), and each server would need
  its own approval story.
- **Keep the native agent for write actions.** Rejected by the owner's
  decision to remove it.

## Decision

- Each CLI turn gets a single-use **turn token** in `clovy-mcp`'s arguments.
  The relay sends it in the handshake (additive `turn` field, protocol version
  unchanged). The app binds the connection to the session, run, CLI, safety
  mode, and attended flag, and revokes the token when the run ends.
- Connections **without** a turn token (external clients) keep exactly the
  read-only surface of ADR-0058. Write tools are listed and callable only on
  turn-bound connections.
- Every Clovy write tool asks an in-app **approval broker** before any effect.
  The broker writes the same interruption item the native agent used, so the
  existing cards render it, waits with a timeout shorter than the CLI's tool
  timeout, accepts one decision, and turns denial, expiry, or cancellation
  into a tool error. Unattended turns are denied without a prompt.
  Clarification uses the same broker.
- When supported by the CLI, Claude Code's own permission prompts are routed
  to the broker through `--permission-prompt-tool`. No flag that skips or
  pre-grants approvals is ever passed; `--allowedTools` only names Clovy's own
  MCP tools, whose authority is the broker inside them.
- User MCP servers and Linear reach CLI turns through `clovy-mcp` as a gateway,
  so credentials stay in the Keychain and host policy (visibility, approval,
  Seatbelt) still applies.
- Companion remote approval keeps the scope of
  [ADR-0053](0053-companion-computer-use-approvals-are-one-shot.md): only
  `computer_use`, one shot.

Contract and tool names: OpenSpec change `cli-substitui-agente-nativo`
(`specs/clovy-mcp-server/spec.md` owns the names).

## Consequences

- One approval boundary for every engine that remains; the native SDK
  interruption can be deleted with the sidecar.
- `clovy-mcp` stops being read-only for turn-bound connections; its threat
  model now includes write authority gated by the turn token plus a human
  decision. The token is visible to same-user processes, which can already
  read the installation secret.
- Codex has no permission-prompt hook in `exec`; its own shell and file actions
  stay limited by its sandbox, while Clovy's tools still go through the broker.
- CLIs without MCP (pi, agy, cursor-agent) get no Clovy tools.
- A later ADR records the removal of the native runtime and supersedes
  ADR-0038, ADR-0040, ADR-0013, and the runtime parts of ADR-0039.

## 2026-10-28 acceptance

The owner accepted removal of the native agent and all server inference routes.
The exact quote and reduced-smoke evidence are recorded in
`openspec/changes/cli-substitui-agente-nativo/tasks.md`, task 3.3. This accepts
the architectural decision; it does not claim that the recorded native MCP
smoke passed. Codex turns and a copied profile are not required for that smoke
under the orchestration instruction of the same date.

## 2026-11-01 addendum: unsupported approval scopes

CLI approval resolutions accept `once` or `deny`. The `session` and `always`
choices are rejected without settling the pending request; they must not be
silently converted into one-shot approval. This does not change CLI sandbox,
operating-system, connector, or authentication permissions.

The desktop can also configure a conversation's approval mode. The host
snapshots it for each run; a bypass authorization is recorded as policy-origin
before a tool may continue, and clarifications still require a person.
Unattended runs do not inherit bypass. This policy remains independent of
CLI execution restrictions and connector, operating-system and login grants.


## 2026-11-03 addendum: Antigravity workspace transport

The earlier consequence listing agy among CLIs without MCP reflects the
capability known at the original decision. The installed CLI now discovers
`<workspace>/.agents/mcp_config.json`; native synthetic canaries also resumed
one conversation with a new server name and observed the second server's real
response. The transport uses this local configuration with the existing
`McpLaunch`, `TurnRegistry`, turn token and approval broker. Each turn receives
a distinct server name and a 900-second tool timeout; names are not grants.

Because callers can supply a shared workspace, a per-workspace OS lease owns
the entry until teardown. A private durable receipt preserves the original
unit before atomic installation. Teardown restores an unchanged unit exactly,
or removes only its still-identical entry while retaining concurrent edits.
Unsafe/malformed current content retains the receipt and fails explicitly.
Host revocation does not depend on cleanup; the next lease recovers crash
receipts before installing another entry. No global configuration is edited.
Only events naming the exact leased server receive Clovy's normalized tool
identity. Other MCP servers cannot claim host-owned diagnostics by using a
similar name or copying metadata.

This transport decision does not qualify Antigravity's unattended policy,
one-shot isolation, default selection, or full platform parity. Those are
separate acceptance tasks in `antigravity-paridade-completa`; native Windows
execution remains required. The original broker/authority decision is unchanged.


### 2026-11-03 review addendum: retained configuration units

Optimistic comparison followed by rename/unlink is not CAS against editors
outside Clovy's lease. Unix mutations now persist a small intent before a
native exchange/capture and retain the actual displaced unit in a private
workspace recovery directory. A race returns a conflict and keeps its bytes
and metadata; cleanup never rolls a later publication back. Unambiguous crash
states can complete their receipt; ambiguous states remain for inspection.
These recovery units are intentionally not swept or automatically pruned.

Receipt v2 must prove the complete before/installed relation and original
metadata against the retained unit. Full restore additionally requires the
identity and metadata of the actually installed unit to match; equal bytes
alone do not justify restoring earlier permissions. Revocation and explicit
cleanup precede terminal publication even on preparation returns.

Windows has no qualified implementation of this mutation contract yet.
MoveFileExW discards the raced target, while ReplaceFileW can merge its DACL
into the token file. Preparation therefore fails before workspace mutation;
Windows acceptance remains blocked until a safe native operation is qualified.
This safety condition does not narrow the intended three-engine parity goal.


## 2026-11-03 approved A1–A5 addendum: app-owned Antigravity profile

The owner approved a common native SDK profile for Antigravity chat, one-shot
calls and interactive authentication. This supersedes the shared-workspace
transport and retained-unit mutation algorithm described in the two earlier
2026-11-03 addenda. Their history is retained; the shared mutation code is
removed, including its unresolved Windows replacement limitation.

The app resolves its real data root before replacing only the final child
HOME/XDG environment. The SDK uses `runtime-profiles/agy/home` under that root;
chat uses a stable private execution directory per stored session and one-shot
calls use exclusive disposable directories. Personal workspaces remain in the
host gateway's authorization context, never SDK discovery. This avoids editing
user configuration, coordinating with unrelated editors or hiding native
login on every call. The trade-off is an explicit first authentication in
Clovy's profile, opened in an owned OS terminal with native OAuth and consent.
No credentials or SDK brain/history are copied.

The existing gateway and broker remain the only host tool boundary. A turn
receives one unique server in its own private `.agents/mcp_config.json`, with
absolute transport arguments and a private recovery receipt. Revocation always
precedes transport cleanup; altered state is preserved and diagnosed. Cleanup
never traverses native authentication. The server's supported `env.HOME`
restores the relay's host identity while the SDK keeps its private HOME, so the
existing debug production-data refusal remains effective without a bypass.

Existing run JSON records the private profile origin alongside the native
conversation ID. Legacy or foreign-profile IDs trigger canonical replay into a
new conversation; matching origins resume. No schema or second memory is added.
Native command/file/URL actions are denied by transient `request-review`
settings, with only the current MCP server allowed; one-shot calls deny MCP as
well. Native OAuth remains separate from API-key billing. These settings and a
private HOME do not establish an OS sandbox or unattended admission. Native
app/terminal/Stop/browser and Windows acceptance remain separate evidence
requirements. The broker, canonical history and routine policy stay unchanged.


## 2026-11-06 addendum: one runtime-mode control

The approved change `composer-modo-unico-e-agy-atividade` supersedes the separate
conversation approval setting described on 2026-11-01. Each new run, including a
retry, derives its immutable approval snapshot from the session runtime mode:
`unrestricted` means `bypass`, and `sandboxed` means `approvals`. The old session
`approval_mode` column remains as legacy data without authority; its setter and
the second composer control are removed. The mode selector stays on new focused
conversations only. Home remains sandboxed.

This makes "Unrestricted" a single explicit choice for attended Clovy tool
authority. It avoids two controls whose independent combinations obscured that
choice. Policy-origin receipts, active-run checks, clarification prompts,
connector grants, login and operating-system permissions stay intact. Unattended
runs always require approval and still fail write requests without a person.

By the owner's explicit decision, Codex alone also uses `danger-full-access`
with `approval_policy="never"` in `unrestricted`, including routines configured
with that mode. This supersedes the original sandbox-only Codex consequence for
that mode: native Codex actions have full filesystem access, while unattended
Clovy writes remain refused by the broker. Sandboxed Codex retains its restricted
policy. Claude and Antigravity keep their existing flags; `bypassPermissions`,
`--yolo` and other approval-skipping flags remain prohibited. The trade-off is
full native Codex authority in return for an explicit unrestricted choice.
