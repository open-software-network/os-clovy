# Browser use owns a private pinned engine and a separate persistent profile

Date: 2026-11-01. Accepted by the owner in the browser-control change.
Supersedes the extension, shared-tab and detected-browser mechanisms of
ADR-0017. The capability, network and consequential-action boundaries remain.

The extension path does not provide a separate persistent login profile and
was never wired into the production broker. A stock browser CLI would expose
unprotected local commands and recover stale references by role/name, which
cannot preserve a task's authority after approval.

Clovy bundles agent-browser source commit
`5868f7c05020e047c00e68e4abf9253cce864f41` with two reviewable patches and a
private host entry point. A closed JSON protocol runs over inherited stdio
pipes with a nonce handshake. Chrome uses inherited CDP pipes; the host cannot
connect to a personal browser or expose streaming, dashboard, plugins, generic
evaluation or upstream MCP commands. The upstream browser, snapshot and
reference implementations remain the engine; private glue adds exact
document/node/origin/element guards without role/name recovery.

The app creates the supervised process group (Windows kill-on-close Job)
before the host can launch Chrome. Chrome stays inside that group/Job rather
than detaching into a separate group. This deliberately changes upstream's
group behavior: cancellation before CDP readiness can always terminate both
host and Chrome without learning a child PID from a delayed response. The
process-resource mutex never covers wire/CDP I/O. An interrupted dispatched
effect returns unknown and is never retried or verified by an unrelated image.

Attended sessions lease one dedicated persistent app-data profile exclusively.
No cookies, browser profile, storage state or password store are imported.
Human takeover stops capture/input, flushes and reopens this profile visibly,
discards old tab/reference authority and requires explicit resume. Native
credential storage is retained; no basic password store or mock keychain is
used. Routine grants select separate ephemeral profiles, capped at two and
reaped after five minutes idle. Turning off a grant, ending a run or quitting
terminates control before releasing leases. Only ephemeral profiles are deleted.

The cost is a larger signed application and a downstream protocol to maintain.
Exact source, Chrome archive digests, lockfile, patch/payload hashes, SPDX
metadata and per-file inventories make each prepared artifact reviewable.
Preparation is a build/dev/release step; runtime never installs a CLI or
downloads a browser. Native macOS/Windows signing, credential-store behavior
and both CLI walkthroughs require their own platform evidence.

## 2026-11-01 addendum: page metadata cannot exempt input from approval

The review confirmed that the inherited keyword classifier could silently
authorize a neutral or localized button whose handler deletes or publishes.
This is an inherited policy gap, not a regression introduced by the private
host. Every click, fill and key press now asks the central approval broker;
field-change and key handlers can have effects just like click handlers.
Page labels identify the target in a capped, quoted, explicitly untrusted
preview, but cannot supply authorization. Sensitive controls remain human-only,
including password autocomplete on a text input. Reads and explicit navigation
retain their existing grants. Unattended routines therefore keep reading and
navigating but cannot perform agent input without a person. This accepts more
attended approval cards rather than treating page-authored semantics as proof
of harmlessness; there is no browser-specific bypass or new approval surface.

The same source audit found that pinned CFT uses Chromium's default native
Safe Storage namespace. Current re-sign-only packaging does not establish a
`-dev` Keychain service. Persistent macOS development/proof launch stays blocked
until a supported namespace integration is chosen and qualified with the owner.
This is an explicit unresolved native boundary, not permission to use mock/basic
crypto, touch an existing owner's item or assume a profile/bundle rename changes
the credential service. Anonymous fixture evidence does not qualify that custody.

## 2026-11-01 addendum: coherent redaction and exact private metadata

An inventory taken before AX can miss a changed or newly inserted field. Native
field inventory and AX observation therefore share a frozen lifecycle boundary;
AX redaction also derives field values from the same raw response before
removing them. Native layout/text geometry covers inherited editable overflow,
closed shadow trees and pseudo-elements. Unsupported editable paint fails
closed instead of returning an unverified image. Every pause is restored, or
the task becomes terminal/unknown and is reaped. Mutation dispatch runs after
restoration with the exact synchronous element guard.

Sanitized labels can collide even when original metadata changes. Original
facts remain only in the private per-reference cache; the wire carries an
opaque random metadata revision, reused only while those facts are unchanged.
This preserves the approval binding without publishing a low-entropy secret
digest that could be guessed offline. This is an internal typed-wire extension,
not another model-facing operation or authority grant.

## 2026-11-01 addendum: classify original facts before redaction

An attribute such as current-password can become current-[redacted] when its
field content is removed from metadata. Reclassifying that DTO in Rust could
therefore request approval for a human-only field, even though the final guard
still refused execution. The private host now computes the single classification
on original facts and returns a required closed enum with the sanitized
inspection. The broker consumes it before requesting approval; it has no second
DOM heuristic or default for missing/unknown values. The enum is bound into the
opaque metadata revision, remains outside model tool schemas, and does not carry
raw facts or field content. Ordinary actions still require the same central
approval; this adds no authorization or browser bypass.

## 2026-11-01 addendum: preserve visibility during coherent observation

This supersedes the lifecycle mechanism in the coherent-redaction addendum,
while retaining its privacy boundary. Chromium156's frozen lifecycle command
calls WasHidden; active only unfreezes the page. It cannot restore an originally
visible page by itself. Bringing the target to the foreground would instead
change focus and activate an originally hidden tab.

The private host therefore uses a transient debugger agent with an owned random
source-URL breakpoint and a fixed, isolated void evaluation. It subscribes before
enabling, accepts only its exact breakpoint hit in the selected session, and
rejects pre-existing or unexpected pauses. Only registered breakpoint IDs pass
the private event filter; stack frames, source, locals and foreign IDs are
discarded. No debugger operation or evaluation becomes a model-facing tool.

Release removes the owned breakpoint and disables only this debugger agent.
V8 retains the pause when another enabled agent accepts it; the host never sends
a global resume command. Completion of its fixed evaluation confirms release.
Uncertain ownership, lost notifications or failed release makes the task
terminal/unknown and reaps it. Page tasks remain paused during inventory, AX,
inspection and capture, and ordinary mutation dispatch follows release with the
same exact synchronous guard. Screenshot animation, geometry and document
checks remain in force. The host does not enable virtual time: switching its
policy from pause to advance still enables a synthetic clock in Chromium156.
Screenshot capture instead pauses native animation playback while the debugger
barrier holds page tasks, then restores the observed finite playback rate.
No visibility emulation, foreground activation, browser
fork or product sandbox/network/crypto override is introduced.

Stock Chromium156's screenshot path requests a redraw and waits for frame
presentation, including with fromSurface=false. A hidden page with pending paint
can stall that request while the debugger's page scheduler pause holds. There
is no established supported parameter for a fresh hidden-frame capture without
that dependency. The host therefore refuses a hidden headed tab's screenshot
before changing animation state or requesting pixels, then releases its owned
pause normally. Snapshot/inspection and the original visibility/focus survive;
default headless background and visible headed captures remain supported.
The parent exposes a definite visibility requirement, with snapshots as the
read alternative and human takeover when an image is required. It never
activates the tab automatically or substitutes a retained surface image. Native
hidden-image capture remains a documented API limit, not a passing PNG claim.

## 2026-11-02 addendum: anonymous crash recovery retains physical authority

Startup, Stop and the idle reaper use the same handle-relative collector.
Startup opens the entire root ancestry without following links, then acquires
the existing physical lease and rereads the receipt under that lock. It never
creates a missing lease. A dead original parent with `Starting` and no `Owned`
is cancelled durably before collection. A bootstrap that has not acquired the
lease has no authority; its post-lock validation refuses the cancelled phase.
The host reads anonymous controls and publishes `Owned` relative to its captured
profile capability and retains that capability beside the original lease.
Replacing the profile's name during publication cannot redirect its write.
`Cancelled` is recoverable after another crash. Stop instead derives authority
from its cancelled Slot and the exit of the original supervised tree; the local
parent need not die.

`Owned` recovery additionally requires the original host and parent to have
exited, an empty original Unix group or Windows Job, matching boot identity,
and the exclusive lease. Free kernel locks after SIGKILL do not establish tree
exit. PID reuse, unknown birth/boot, denied or incomplete observations preserve
the profile. Recovery neither kills processes nor replays browser operations.
Linux uses its boot UUID, Darwin `kern.bootsessionuuid`, and Windows the native
boot-environment GUID. Older receipts lacking a comparable boot identity stay
uncollected. The Windows native query layout follows
[PHNT's declarations](https://github.com/winsiderss/phnt/blob/master/ntexapi.h).

Before moving a profile, the collector publishes a private holder containing
a versioned receipt of its root, holder and profile identities. Relative
no-replace rename plus checks before and after the move preserve collisions
and changed entries. Payload traversal preserves the three controls only at
the profile's first level. Each traversed entry is opened without following
links and checked against its enumerated identity.

After payload removal and directory synchronization, a versioned
`.clovy-gc-complete-<profile UUID>.json` record is published without replacement
in the root. It binds all directory/control identities and the full authorized
receipt. The collector locks this exact record before removing any controls;
the original lease remains held through profile retirement. The record is the
remaining exclusion authority when the lease disappears. A restarted collector
validates and locks the record, removes only identified remaining controls and
empty directories, and retires the record last after synchronizing absence.
It never traverses unexpected terminal payload. Legacy, corrupt, linked,
foreign or unidentifiable temporary objects are preserved, not adopted by name.

This adds small durable control records rather than losing crash recovery when
the last in-profile receipt is removed. Unix operations use directory FDs,
`openat`, no-follow opens, no-replace rename and `unlinkat`. They do not promise
atomic inode-conditioned unlink against an arbitrary hostile filesystem owner
with the same UID. Windows retains enumerated file IDs, deletes through the
same identified handle, and requests write/append rights only on directories
that need mutation or synchronization. Payload handles deny delete sharing;
retained control handles permit it to allow retirement through their identified
objects. Flush uses
[NtFlushBuffersFileEx with flags zero](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntflushbuffersfileex),
propagating failures. Type checks are not native filesystem, crash-power-loss,
sharing-mode or signed application acceptance.

## 2026-11-02 addendum: Windows pins the original recovery location

Windows uses logical quarantine at the original UUID, while Unix retains the
physical holder described above. This supersedes the earlier cross-platform
holder placement, not the common authorization/payload/terminal algorithm.
[MS-FSA FileRenameInformation](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/87f86c9b-6c2a-4803-84b7-131a74a434fa)
requires rejecting a source-directory rename with open descendants; the
[open-descendant algorithm](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/133840e4-778e-44ca-9b41-da2323615075)
does not exempt DELETE-sharing handles. POSIX replacement semantics relax the
destination's existing opens, not those source descendants. Closing/recreating
the lease to enable a rename would break physical exclusion and is rejected.

After the unchanged R1/R2 or Stop authorization under the same exclusive lease,
Windows reopens the captured directory object, compares its physical ID and
pins it with reference rights, without DELETE access or DELETE sharing. Root
and normal directory references also omit DELETE; Windows rename opens its
destination internally with sharing that can conflict with an existing DELETE
access handle ([MS-FSA Appendix A, note 185](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-fsa/4e3695bd-7574-4f24-a223-b4679c065b63#Appendix_A_185)). Destructive directory operations
acquire DELETE temporarily with ReOpenFile, compare IDs and release it before
another rename can use that directory as a destination. This does not apply
to payload handles: their DELETE/share-3 open is the actual removal capability,
and reopening it for DELETE would conflict with itself.

Starting-to-Cancelled consumes the typed Cleanup, revalidates and closes the old
receipt reader while retaining the original exclusive lease, publishes the new
receipt durably, then captures and validates the replacement before recreating
Cleanup. A crash before publication leaves Starting recoverable; after it,
Cancelled resumes. Stop uses this same transition for Starting. Neither phase
nor receipt absence grants a bootstrap permission to resume.

The internal terminal format is version 2. Its tagged location is either
`unixHolder`, binding real holder and holder-receipt IDs, or
`windowsPinnedOriginal`, binding the actual profile at its original UUID through
the root/profile IDs and nonce. There is no synthetic holder, zero-ID alias or
error fallback. Old/ambiguous records and incompatible platform locations are
preserved. Active grants, receipts, Owned and the spawn/ACK protocol are unchanged.

Both placements use one control retirement routine. PayloadGone is durable and
the exact root record exclusively locked before any control is removed. Windows
keeps the directory pinned while retiring controls and the same lease, then
reopens a reference to that same object and closes all pins only at empty
directory retirement. The record remains locked throughout. The operational
DELETE reopen and parent-relative ID check cannot redirect removal to a
replacement. An unexpected entry, replacement or failed observation preserves
the record and object; terminal recovery never recurses reappeared payload.
Only synchronized absence permits removal of the root record.

Native Windows regression consumers cover publication, pinned collection,
same-handle payload removal, concurrency, swaps and crash boundaries. GNU
compilation on Linux is not Windows execution or power-loss proof. Windows
filesystem/sharing and application acceptance still require the native host;
the existing Unix matrix and live Chrome proof remain separate evidence.
