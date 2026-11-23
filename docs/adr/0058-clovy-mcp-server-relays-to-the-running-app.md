---
status: accepted
date: 2026-10-04
---

# The Clovy MCP server relays to the running app over an authenticated Unix socket

## Context

MCP clients on the Mac (Claude Code, Cursor, and the CLI chat engines) should
read the user's notes, dictations, memories, and activity. They start MCP
servers themselves, as stdio child processes, so Clovy has to ship an
executable they can run. That executable runs as the user but outside the app.

The data it needs sits in `notes.sqlite3` and in the SQLCipher-encrypted
`activity.sqlite3`, whose key is in the Keychain for the app
([ADR-0057](0057-activity-database-uses-sqlcipher-linked-app-wide.md)). The
capture thread owns the activity store while capture runs, and every read must
apply the current exclusions, retention, and active profile.

This is an outbound server for external clients. It does not bring back
Clovy-managed MCP servers inside the agent loop, which
[ADR-0040](0040-plugin-capabilities-as-host-tools.md) retired; the in-app agent
keeps using host tools.

Alternatives considered:

- **The binary opens the databases itself.** It would need the activity key
  outside the app (a second Keychain identity or an exported key), would race
  the capture thread and the timeline ETL, and would duplicate the exclusion,
  retention, and profile rules.
- **A loopback TCP or HTTP MCP endpoint in the app.** Any local process (any
  user, any sandboxed app) can reach a port; clients would need a token in
  their configuration, and Streamable HTTP adds a second transport to keep
  correct.

## Decision

Ship a small stdio binary, `clovy-mcp`, that relays newline-delimited JSON-RPC
to the running app over a Unix socket in `<app data dir>/mcp/` (directory 0700,
socket 0600). The first line on a connection must carry the installation
secret, a random value stored next to the socket (file 0600); the app also
checks that the peer runs as the same user. The app answers MCP with the same
queries the agent tools and the "Today" view use. The server is off by
default, runs only while the app is open, and the relay answers by itself with
a clear error when it cannot reach the app.

Contract and protocol: [docs/mcp-server.md](../mcp-server.md).

## Consequences

- Tools answer only while Clovy is open with the server on; clients see an
  error saying so otherwise.
- One copy of the read rules: the app's queries, exclusions, retention, and
  active profile apply to MCP clients unchanged.
- Another bundled helper binary to build, sign, and keep at a stable path
  (`Resources/native/bin/clovy-mcp`); moving Clovy.app means copying the
  configuration again.
- The socket path must fit the 104-byte Unix socket limit; very deep data
  directories cannot run the server.

## Addendum: on by default (2026-10-07)

The decision above shipped the server off by default. From 2026-10-07 it is on
by default, together with activity capture (OpenSpec change
`default-capture-and-session-mcp`), so CLI chat sessions get Clovy's tools
without a visit to Settings. Turning it off is persistent: no restart or save
of other settings turns it back on. Profiles saved before this change cannot
tell an old default `false` from an explicit opt-out, so on the owner's
decision they are turned on once, recorded by `defaultsVersion` in
`activity-settings.json` (see [docs/activity-capture.md](../activity-capture.md)).
The relay, socket, secret, and same-user checks are unchanged; tools still
answer only once the listener is ready.

## Addendum: turn-bound write tools (2026-10-17)

The decision above serves read-only tools to every client. Since the parity
slices of the OpenSpec change `cli-substitui-agente-nativo` reached `main`
(2026-10-17), a connection opened by a Clovy CLI chat turn carries a turn
token in the handshake (additive `turn` field, protocol version still `1`)
and also gets turn-bound tools, including writes that ask the in-app approval
broker first. That decision and its trade-offs are
[ADR-0063](0063-cli-turns-get-write-tools-through-an-approval-broker.md),
still `proposed` until the owner accepts the parity proof; it supersedes
"everything is read-only" for turn-bound connections only.

Unchanged by it: the relay, the socket and its 0600 mode, the installation
secret, the same-user check, and the read-only surface for every connection
without a turn token (Cursor, `claude mcp add`). The binary still never opens
a database. The token is checked only after the secret and user id, so it
never opens the socket by itself. Contract: [docs/mcp-server.md](../mcp-server.md)
("Turn binding", "Approval broker", and the tool tables).

## Addendum: private Windows transport (2026-11-02)

The approved Windows expansion in D6 of
`openspec/changes/melhorias-agente-audio-automacao/design.md` uses local byte-mode
named pipes in place of the Unix endpoint. It shares the same framing,
handshake, quotas, turn registry, revocation and receipt handling. The endpoint
is derived from the canonical installation directory; protected explicit
user/SYSTEM DACLs, remote-client rejection and process TokenUser checks precede
secret/token validation. Existing ambiguous storage is refused, not repaired.
The CLI-owned relay does not authenticate against its parent process.

This preserves the decision to keep databases and tool authority in the app
without introducing a TCP or public MCP endpoint. The installer includes a
real target/profile-specific relay signed by the configured publisher, checked
after extraction. Native Windows security, I/O and packaging qualification
remains separate from cross-target typechecking; see the subsystem doc.
