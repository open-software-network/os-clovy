# Clovy Companion protocol

> **Historical document (2026-10-28).** Server, Auto/funding, hosted relay and
> native-agent claims below describe an earlier design, not the current app.
> The inference server is removed; chat/routines use installed CLI engines,
> Groq/OpenRouter/fal operations use the user's keys directly, and sharing has
> no replacement. Current boundaries are in
> [ADR-0073](adr/0073-desktop-calls-user-providers-without-a-clovy-server.md),
> [llm-providers.md](llm-providers.md), and [configuration.md](configuration.md).


> **Fork note:** this fork's desktop app no longer has the companion (removed by
> the `desvincular-servicos-open-software` change). The Clovy API relay and the
> `crates/clovy-companion-*` / `june-companion-*` crates are removed in a later change.

The protocol below is retained as historical integration documentation, not a
current desktop entry point. CLI tool approvals now belong to the local broker
([ADR-0072](adr/0072-cli-chat-replaces-the-native-agent.md)); no companion
transport gains approval authority.

The shared crate is `crates/clovy-companion-protocol`; desktop and iOS crypto
use `crates/clovy-companion-crypto`.

## Envelope and bounds

Protocol version 1 frames carry an operation id, monotonic per-session
sequence, issue and expiry times, required capability, and one typed body.
Control TTL is 30 seconds. Encoded plaintext is capped at 44 KiB, ciphertext
at 45 KiB, relay JSON at 64 KiB, text at 32 KiB, and pages at 100 items.
Unknown versions fail closed. Additive optional fields or new variants require
a version-aware compatibility test before shipping.

The relay envelope contains only routing metadata and base64 ciphertext. It
cannot express a desktop command or application payload.

## Crypto sessions

Pairing uses `Noise_XXpsk3_25519_ChaChaPoly_BLAKE2s`; the scanned or manually
entered pairing code contributes a 32-byte single-use PSK and the transcript
authenticates both static identities. Linked reconnects use
`Noise_KK_25519_ChaChaPoly_BLAKE2s` with the approved static keys. Noise nonces
reject replay and reordering/tampering. A fresh handshake is required after
2^20 messages or 24 hours.

The relay pairing API receives only SHA-256 of the pairing secret as a
five-minute proof. The phone generates an opaque device credential and submits only its
encoded UTF-8 value's hash during pairing. Desktop approval activates that
hash. The phone later presents the same encoded value with the `Device` scheme;
the relay hashes that representation and compares it without retaining the
plaintext. Noise separately authenticates the device's private key and protects
all content.

The linked device may put a bounded `PeerHello` JSON payload in its first Noise
handshake message. Its `capabilities` array advertises optional event and
response features that this app build can actually handle. An empty payload
remains valid for older clients and advertises no optional capabilities.
Unknown capability strings are ignored within the bounded array so a newer
version-1 peer can add an optional feature without disabling capabilities both
peers already understand.
Desktop records the declaration only after the Noise static identity
authenticates and clears it when that peer session or relay connection ends.

The signed-in Desktop creates the pending pairing under its OS Accounts user.
The matching pairing proof authorizes one phone proposal to that exact pairing,
so the phone neither supplies an account id nor carries an account bearer. The
relay binds the proposed device to the user already fixed by Desktop creation,
and explicit Desktop approval remains required before the device credential or
link becomes active.

During an explicit pairing, the authenticated desktop may establish its relay
socket while the pairing is still pending, but pending phones remain unable to
connect or route frames. Before the relay exposes approval to the phone, the
desktop validates and stores the proposed device identity, marks the Noise
pairing secret ready locally, and confirms that relay socket is connected. A
confirmed remote approval failure rolls back that local readiness; an unknown
network outcome preserves it so an approved phone is never stranded between
the two boundaries. The relay also refuses to start its bounded persistence
step in the final 16 seconds of the pairing window. Postgres checks the pairing
expiry in the same transaction that activates the durable device link. Only a
durably activated link may finish in memory after the wall-clock expiry passes;
an expired transaction rolls back the device and link writes together. An
approval retry recognizes an already committed matching link and reconciles the
in-memory pairing instead of treating a lost commit response as an identity
conflict.

## Capabilities

The only grants are notes read/edit, agent read/chat/cancel, model read/edit,
generated-media read, safe settings read/edit, existing-recording
state/pause/resume/stop, app focus, and self-device read/revoke. Linked devices
may also receive the separate `filesUpload` and `filesBrowse` grants described
below, plus one-shot Computer use approval. Model discovery and current-session
reads require `modelRead`; changing a session's next-run model requires
`modelEdit`. Body-to-capability equality is validated before dispatch.

The encrypted `deviceGetSelf` result may include the Desktop's user-facing
device name as the optional `desktopDisplayName` field, bounded to 128 UTF-8
bytes. Desktop resolves and caches the value at startup, then supplies it only
after the Noise-authenticated `devicesReadSelf` request. Older companions ignore
the additive field, and new companions retain their generic Mac fallback when
an older Desktop omits it.

Agent session and message reads go through typed frontend intents backed by
the former desktop session APIs. The companion receives the same
sanitized display text as Clovy Desktop: machine context, provider routing
details, reasoning, tool calls/results, approvals, secrets, and media internals
stay on the Mac. The always-mounted app shell serves reads even when the Agent
screen is closed. Send and cancel intents wake the existing Agent workspace.
Wire session identifiers are qualified explicitly: agent data uses
`storedSessionId`, while active-recording snapshots and controls use
`recordingSessionId`. The companion protocol does not expose a desktop agent
session id.

Model control uses the `modelsList`, `sessionModelGet`, and `sessionModelSet`
body tags. The first two return `models` and `sessionModel` results under
`modelRead`; the mutation also returns `sessionModel`, under `modelEdit`, so
the phone can render the exact accepted selection. `sessionModelChanged`
events publish desktop-originated picker changes under `modelRead`. A peer
receives that additive event only after it has demonstrated `modelRead` or
`modelEdit` on its current Noise connection, so an older companion is not sent
a body tag it does not understand.

The model catalog is bounded to eight entries. Desktop currently exposes Auto
plus the available subset of its four recommended generation models, using the
live catalog for canonical names, providers, privacy classes, and price
labels. Model ids and names are capped at 256 UTF-8 bytes, providers at 64,
descriptions at 512, and privacy/price labels at 128. A set request for any
model outside that curated live set returns `unsupported`; a missing or
partition-inaccessible stored session returns `not_found`.

Desktop remains authoritative for model persistence and run boundaries.
`sessionModelSet` stages the accepted selection in the same per-session store
as the Mac picker. It does not cancel, restart, or reroute an active run. The
staged value becomes authoritative when the next run starts, matching
ADR-0018; later desktop and companion reads report that staged value.

Agent transcript pagination starts with the newest page and walks backward;
items within each page remain chronological so the mobile client can prepend
older pages without reordering a conversation. Pages keep encoded results
below the frame budget. An individual oversized display message is clearly
marked as truncated. Notes
whose editable title or content cannot fit safely in one frame are rejected
with an instruction to open them on the Mac; the companion never loads a
truncated note into its editor, which prevents an edit from overwriting unseen
content.

Agent history and status events may add up to eight **companion media result**
references for canonical, tool-produced image/video artifacts. A reference
contains an opaque artifact id, kind, MIME type, byte size, optional paired
dimensions, and optional video duration. Empty media arrays are omitted, so
version 1 clients that know only text/status keep their existing shape.
References never contain paths or inline bytes.

`mediaFetch` requires `mediaRead` and returns `mediaChunk` under the same
capability. The source is capped at 100 MiB and each response at 31 KiB of raw
bytes. Base64 expands a full chunk to 42,328 bytes; the contract's worst-case
response test includes maximum identifiers, digest, and JSON syntax and keeps
the plaintext below 44 KiB with at least 750 bytes spare. Each chunk repeats
the source's lowercase SHA-256 digest, total size, offset, and exact completion
state. The client advances by decoded byte count and verifies the complete
file before display or photo-library save.

Only the canonical full artifact is fetchable. Clovy does not persist a
canonical media thumbnail, so a thumbnail tier would create a second artifact
and retention contract. Mobile may derive a local thumbnail only after the
full artifact passes integrity verification.

History and fetch are typed frontend intents. They check that the stored
session is in the current data partition before artifact work and check again
after awaited inspection or file IO; a partition switch discards the result.
Fetch resolution always combines the stored session id with the opaque
artifact id and never accepts a phone-supplied path.

## Attachments and granted Mac roots

Phone attachments use `uploadBegin`, `uploadChunk`, and `uploadCommit` under
`filesUpload`. Begin declares a UUID reservation id, UTF-8 file name, optional
media type, exact byte count, and lowercase hexadecimal SHA-256. A file is at
most 25 MiB and a raw chunk is at most 32 KiB. Offsets are contiguous. Commit
returns a short-lived opaque attachment reference only after the byte count and
digest match.

Mac browsing uses `browseRootsList`, paginated `browseDirList`, and
`browseFileStat` under `filesBrowse`. Roots exist only after the signed-in user
selects a directory in Clovy Desktop Settings. Requests carry an opaque root id
and a relative path; responses contain bounded display labels, names, entry
kinds, sizes, modification times, and cursors. `browseFileStat` mints a
short-lived opaque Mac-file attachment reference. Absolute paths never cross
the companion protocol.

`agentSend` accepts an additive optional `attachmentReferenceIds` array of at
most eight unique non-nil UUIDs. Legacy payloads without the field still decode
and an empty array is omitted when encoding. Desktop resolves each reference
for the authenticated account and linked device, revalidates a Mac target
against a still-granted canonical root, and passes the local path to the normal
agent attachment copier. There is no `fileRead`, download, write, move, delete,
symlink traversal, hidden-file traversal, or implicit root in protocol v1.

The maximum 32 KiB chunk expands to 43,692 base64 characters. A worst-case
request frame encodes to 43,988 bytes, leaving 1,068 bytes below the 44 KiB
plaintext ceiling. Existing Noise and relay-envelope ceilings therefore remain
unchanged.

`computerUseApprove` is limited to the existing `computer_use` desktop agent
approval interruption. A pending event carries a request id, stored session id,
bounded action and description, optional target app/URL, and a 60-second
deadline. After receiving the event, a linked device may send the additive
`computerUseApprovalReceived` mutation with both ids; approve/deny repeats the
same pair. Status events report approved, denied, executing, succeeded, failed,
expired, or cancelled. Desktop binds the request id to the distinct SDK
tool-call id rather than assuming equality. Before
publishing, Desktop resolves the exact process, window, and app identity; a
contradictory app/window selector, changed target, or target that is not yet
verifiable keeps the interruption desktop-local. Permit consumption compares
that exact target again and falls back to the Mac-local authorization surface
on mismatch. It consumes one app-authorization decision; a second decision in
the same action remains desktop-local, and the permit never becomes a task or
app grant. Desktop never truncates approval fields to fit those bounds: an
oversized action, description, app, or URL keeps the interruption
desktop-local. The default-off Linked devices toggle, Computer/Browser use
experiment, authenticated live peer advertising `computerUseApprove`, active
link, and Rust broker policy all gate it. The authoritative expiry uses a
monotonic deadline; wall-clock values are display metadata. Auto-deny retries
bounded transient failures, then retires the remote request and leaves the
interruption Mac-local if dispatch cannot be completed. The timer is armed only
after an authenticated `computerUseApprovalReceived` frame proves that a live,
capable peer received this exact request; a historical handshake or successful
relay write is insufficient. See ADR-0053.

There is no variant for arbitrary Tauri or agent-harness calls, recording
start, note delete, other approvals, unrestricted mode, general filesystem
access, shell, credentials, connectors, updates, account deletion, or adding a
device. The bounded browse variants above are not a general filesystem
capability and do not return file contents.

## Idempotency and reconnect

Mutations carry stable client operation ids. The native client keeps an
unresolved id in Keychain for seven days and reuses it after an ambiguous
disconnect or relaunch, and it does not dispatch until that id is durably
stored. The desktop writes an outcome-unknown reservation before every mutation
crosses a side-effect boundary. A final response replaces the reservation and
is returned on retry. If Desktop crashes in between, the reservation is
returned as a distinct outcome-unknown error instead of dispatching the
mutation again; the user checks Clovy on the Mac before explicitly choosing the
action again. Results and reservations expire after seven days. Completed
responses are capped at 1,024 per device; up to 128 unresolved reservations are
retained separately, and reaching that bound refuses new mutations instead of
evicting a reservation. Revocation removes both. Sequence state resets only
after a fresh authenticated Noise session. The client reuses
a healthy transport, retires stale Noise keys when a fresh authenticated
handshake arrives, and refreshes cursor-based lists after foreground/reconnect.
No offline control request is replayed.

Desktop upgrades rewrite the earlier retryable-busy reservation payload to the
pending, non-retryable outcome-unknown form before normal pruning runs. Existing
ambiguity guards therefore keep their at-most-once meaning across the schema
upgrade.

Note edits carry `expectedRevision`; SQLite updates atomically only at that
revision. A mismatch returns a typed conflict with the current note.
