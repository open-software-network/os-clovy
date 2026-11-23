The `clovy-agent-browser` binary takes no arguments. Build the pinned upstream
crate with `--locked --features clovy-private --bin clovy-agent-browser`, after
applying both reviewed patches and copying the Rust payload files and
`clovy_private_guard.js` to `cli/src/`. Ship only this private binary.

The parent creates the engine's process group or Windows Job before the engine
can create Chrome. The engine and all Chrome children inherit that group/Job.
The parent clears the environment and supplies the first stdin line
`{"protocol":1,"token":"<32 random bytes encoded as 64 lowercase hex characters>","grant":{...}}`.
The grant binds the profile, modality, nonce, root/profile file identities and
the parent PID and native birth. Before acknowledging, the host validates the
grant and the live parent. For an anonymous profile the parent releases its
lease immediately before `Hello`; the host acquires the existing kernel lease,
revalidates `Starting`, and durably publishes its process and group or Job
identity as `Owned` before acknowledging. The parent verifies that identity
against the spawned child and receipt before sending `start`. Persistent
profiles retain the parent lease. `start` must use the exact granted path and
mode, and the host revalidates before a launch or relaunch. The host echoes the
token over the inherited private stdout pipe. It opens
no socket, discovery service, debugger port, provider plugin or streaming server.
The authentication capability is possession of the inherited private pipes;
the token correlates the child, and is never an argument or environment value.

Anonymous recovery shares the supervisor's handle-relative collector. Under
the existing exclusive lease, dead-parent `Starting` becomes durably
`Cancelled`; a late bootstrap must fail its post-lock phase/parent validation.
`Owned` additionally requires original host/parent exit, matching native boot
identity and an empty original group/Job. No recovery path kills or replays.
Stop uses its cancelled Slot and completed tree termination instead of parent
death. Unix moves the profile into a private identified holder. Windows instead
pins its original UUID directory without DELETE access or DELETE sharing;
open controls and the same lease remain in place. NTFS forbids moving a source
directory with open descendants, even when those opens share DELETE.
A locked root `PayloadGone` record keeps cleanup resumable after controls
disappear. Its internal version 2 uses a typed `location`: `unixHolder` with
physical holder/receipt IDs, or `windowsPinnedOriginal` with the real profile
at the original UUID. Version 1, missing/ambiguous locations and locations for
another platform are retained, never inferred from names or mapped to a fake
holder. Active grant/receipt/Owned and spawn/ACK formats are unchanged.
Terminal recovery removes only
identified controls and empty directories, preserving unexpected payload,
unknown entries and all persistent profiles. A lease is never recreated.
Cancelled publication closes its old receipt reader under the same lease,
then captures and validates the new durable reader before deletion is possible.
Windows releases every profile pin only after controls are retired and the
terminal record owns exclusion, retains an identified reference across that
release, and acquires DELETE operationally by reopening that same directory
object. Payload entries keep their original DELETE/share-3 handle through
removal; they are not reopened for DELETE.
See the 2026-11-02 addendum in `docs/adr/browser-private-agent-host.md`.

The anonymous host retains the same profile capability as its lease, including
receipt publication and subsequent ownership validation. These operations do
not resolve a replacement profile by its former pathname.

Every subsequent JSON line is bounded to 16 MiB and contains `id:u64` and an
`action`. Unknown action fields are rejected. Responses are
`{id,delivery,data? ,error?:{code,message}}`; error codes come from a finite local
allowlist, and upstream diagnostic text never crosses the wire. Successful reads
are `not_attempted`, successful mutations are `delivered`, and errors after a
possible mutation are `unknown`. An unknown result invalidates every reference.
There is no retry or automatic change of modality.

Actions:

- `start`: `chromePath`, `profilePath`, `proxyPort:u16`, `persistent:bool`,
  `headed:bool`. Persistent profiles retain native credential storage; anonymous
  ephemeral profiles use only run-owned mock storage. Paths and the proxy are host-owned. Returns integer `tabId`,
  `targetId` and `url`. A fresh task tab is created directly; restored profile
  tabs are never adopted.
- `navigate`: `tab_id:u32`, `url`. Only credential-free HTTP(S) URLs. Returns
  final `url` and `tabId`.
- `back`, `snapshot`, `screenshot`: `tab_id`. Snapshot returns `snapshot`, `url`,
  `tabId`. Screenshot returns PNG `base64`, `url` and `tabId`; non-HTTP pages
  are refused before capture (except the empty about:blank page), and document,
  URL, field geometry and viewport must remain unchanged before emission.
  A document whose actual native visibility is hidden returns `screenshot_hidden`,
  `not_attempted`, with no image. Stock Chrome requires a newly presented frame
  and can stall on a mutable hidden page under the coherent observation barrier.
  The host refuses before Animation/capture and releases its own pause normally;
  it never activates the target or substitutes another window's image. Redacted
  snapshot/inspection and exact guarded interactions remain available on hidden tabs.
- `tab_open`: optional `url`; `tab_list`; `tab_switch`, `tab_close`: `tab_id`.
  Only explicitly created task tabs are listed or addressable. IDs remain
  monotonic through a human takeover restart. Closed targets fail `tab_gone`.
- `inspect_reference`: `tab_id`, `ref`. Returns `url`, `origin`, `documentId`, `metadataId`, `interactionClass`,
  `backendNodeId`, and `element` containing strings `tag`, `inputType`, `role`,
  `name`, `id`, `label`, `autocomplete`, and booleans `inForm`, `contentEditable`.
  `name` is the HTML name attribute; `label` is the accessible caption.
  No field value is included. Free metadata is sanitized against the coherent
  field inventory. `metadataId` is a random private-cache revision, reused only
  while the original metadata and inspection facts remain unchanged. It is not
  a digest of field content. Original metadata stays private to the host.
  The required closed `interactionClass` is `human_only` or `approval_required`,
  derived from the original facts before metadata redaction. The parent consumes
  this trusted classification; the model cannot supply or override it. The final
  guard uses the same original-fact classification and binds it to the revision.
- `guarded_action`: `tab_id`, `ref`, `operation`, full `expected` inspection,
  optional `text` or `key`. Supported operations are `click`, `fill`, `press`.
  Press currently supports Enter, Space, Escape, Backspace and Delete; other keys
  fail definitively before dispatch. The exact node, document, origin and
  metadata are rechecked in an isolated-world DOM task immediately before the
  semantic effect. The full expected revision and original metadata are checked,
  including when redaction makes two different captions look identical. Focus
  is followed by another check before typing/key events. Password, code and
  payment criteria apply to every interaction, including clicks on text inputs
  with password autocomplete tokens; approval cannot override this refusal.
  This uses DOM events, whose `isTrusted` is false; sites requiring trusted input
  need a human interaction. No role/name recovery, generic selectors, JavaScript,
  CDP, cookies, arbitrary browser arguments or file upload API is exposed.
- `suspend`, `resume`: suspend all page reads and actions; resume discards refs.
- `takeover`: host-only. Suspend immediately, close Chrome with a short flush
  grace, reopen the same profile visibly, create a fresh tab, return its IDs,
  `mode:"visible"`, `suspended:true`. Resume is explicit. Never exposed to a model.
- `shutdown`: short graceful Chrome close, then process termination. The parent
  kills/reaps the entire owned group/Job on completion or cancellation.

References are random opaque IDs minted per snapshot and bound in memory to a
single tab, exact main-frame id, loader id, document root and backend node.
Snapshot and inspection observations use an owned debugger breakpoint while the
native field inventory and accessibility data are collected. Subscription precedes
setup; an actual hit of the registered random breakpoint establishes ownership.
No stack, locals, arbitrary breakpoint identifiers or source data are retained.
AX values and editable
descendants are also derived from each raw AX response before any formatting.
The own breakpoint is removed and only the host's debugger agent is disabled;
completion of the fixed no-op proves release. No debugger resume command is sent.
A pre-existing or concurrent foreign pause is never resumed. Uncertain ownership
or release closes the browser with a terminal unknown result. This preserves the
original native page visibility and focus. Release completes before the synchronous final guard and effect,
so ordinary network handlers retain their normal behavior. Private snapshots do
not annotate the page or recurse into frames.

Screenshots use the same owned task barrier and temporarily pause native animations,
restoring the observed finite playback rate on every path. Virtual time is never
enabled: its advance policy would retain a synthetic clock. Native DOM
layout and text-fragment bounds cover controls, inherited editable descendants,
closed shadow content, pseudo elements, iframes and frames. Overflow outside an editable
ancestor's own box is covered independently. Native geometry and the viewport
are reread before emission. Unsupported text shadows, filters, reflections,
transforms, strokes or emphasis that cannot be bounded fail closed rather than
producing an image. Every scheduler restoration is attempted on every path;
failure closes the browser and returns a terminal unknown result so the parent
invalidates the task. The parent owns lifecycle cancellation, grants, approvals,
network policy, artifacts and resume authorization. The parent holds persistent
profile leases; an admitted host holds its anonymous profile lease.

Native macOS and Windows acceptance remains separate from the synthetic Linux
fixture. `private-host-smoke.py` accepts the exact host and Chrome paths and
writes real receipts/screenshots; it uses only a run-owned profile and a local
proxy which rejects every hostname except `fixture.invalid`.
The source-only `cdp_fixture.py` relay is not shipped. Its real HTTP image latch
exercises mutation between inventory and AX without adding a production hook or
injecting any CDP command.
The separately admitted visibility helper exercises the rebuilt host in both visible and hidden tabs,
including real button effects, paint refusal and a native CDP error, and checks
actual focus, RAF and native clock pacing. Visible capture produces a real masked PNG;
hidden capture is explicitly unavailable and must refuse without poisoning handles.
It does not emulate visibility or
restore foreground focus after observations. Native DOM snapshot empty strings
accept the protocol's signed `-1` only in permitted text/value fields; invalid
indices and missing required node/frame/style facts fail closed.
