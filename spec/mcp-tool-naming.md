# Internal agent tool naming

**Rule.** June-owned tools exposed to the agent as in-loop host tools are named
**`verb_object`**: the verb first, in `snake_case`, with no capability or
provider prefix repeated inside the tool name.

[ADR-0040](../docs/adr/0040-plugin-capabilities-as-host-tools.md) defines the
integration shape. June-managed `june_*` MCP servers are retired; they are
historical architecture, not the namespace or template for a new internal
tool surface.

Good: `start_recording`, `stop_recording`, `generate_image`, `edit_image`,
`search_threads`, `read_thread`, `get_meeting_note`, `start_session`,
`accept_shared_tab`, `list_tabs`.

Not: `session_start`, `tab_accept_shared`, `recording_start`,
`june_browser_navigate`, `june_gmail_search_threads`.

Two carve-outs, both pre-existing and deliberate: a host-tool surface may use a
`namespace_verb` form when the namespace disambiguates an otherwise generic
verb (`web_search`, `web_fetch`), and a status reader may be named for what it
returns (`recording_status`, `status`).

**And: the name of every tool in a contract is fixed in exactly one document,
before the code is written.** For a June-owned host-tool surface that document
is the subsystem PRD or contract that owns the capability. Other documents
reference those names; they never coin them.

**Why.** Two reasons, one aesthetic and one that cost real work.

The aesthetic one: a tool list is a menu the model reads on every turn. Mixed
conventions (`start_session` beside `tab_accept_shared`) make the surface look
like it was assembled by different people who never spoke, and the model has to
guess the shape of a name it has not seen.

The one that cost real work: in JUN-278 the canonical PRD described the
Browser use surface in prose ("session start and close ... accept a user-shared
tab") without naming the tools. The portfolio implementation plan then coined
`start_session` / `accept_shared_tab`; the implementing slice independently
coined `session_start` / `tab_accept_shared`. Both were reasonable. Both were
merged into different documents. Neither was wrong, which is exactly why
nobody caught it. A contract described but not *named* will be named twice.
That incident happened while Browser use was framed as the `june_browser` MCP
server. ADR-0040 retired that server shape, but the naming lesson remains.

**How to apply.** Before implementing a June-owned host-tool surface, put the
names in the owning PRD or contract as a table, then implement against that
table. When adding a tool to an existing surface, match its established
convention and add the name to the owning document in the same change. When a
name in a document and a name in code disagree, the document that *owns* the
contract wins and the code is the bug; if no document owns it, that is the
defect to fix first.

The note tools (`search_june`, `get_note`, `get_dictation`) are named in
[docs/agent-note-tools.md](../docs/agent-note-tools.md).

Clovy's tools for CLI chat turns are served by the Clovy MCP server. Its read
tools are named in the `clovy-mcp-server` specs of the OpenSpec changes
`add-activity-intelligence` and `default-capture-and-session-mcp`; its
turn-bound tools (memory, media, recorder, Browser use, Computer use, vault
and skills, routines, connectors, interaction) are named in the
`clovy-mcp-server` delta of the OpenSpec change `cli-substitui-agente-nativo`
([ADR-0063](../docs/adr/0063-cli-turns-get-write-tools-through-an-approval-broker.md)),
which owns them, and implemented one group per file in
`src-tauri/src/mcp_server/tools/`. They keep the names of the native host
tools they replace. [docs/mcp-server.md](../docs/mcp-server.md) references
those names without coining them, and
`src-tauri/tests/mcp_server_doc_catalog.rs` fails when its tables and the
server's `tools/list` disagree.

Inspect the current catalogs and dispatch before coining anything: the
Clovy MCP server's groups in `src-tauri/src/mcp_server/tools/`, and the
legacy host tools in `src-tauri/src/agent_runtime/api.rs`,
`src-tauri/src/agent_runtime/native_connectors.rs`, and
`src-tauri/src/agent_runtime/tools.rs`.

**Exceptions.** Tools supplied by external MCP servers, including
user-connected and provider-hosted servers, are not ours to name; use them as
they come (Notion's `notion-*`, and the gateway's `mcp_<server>_<tool>` and
`mcp_linear_*`, which only prefix the server's own names). This rule binds
June-owned in-loop host tools and the Clovy MCP server's own tools only.

**Native transport identity.** Canonical names above remain the identities in
the common catalog, dispatcher, snapshots, history and approval/policy keys.
Bound Claude Code, Codex and Antigravity MCP transports use one passive codec
per grant at `tools/list` and `tools/call`, shared with native event translation.
Safe ASCII names up to 36 bytes pass unchanged except for the reserved `_c_`
prefix. Longer, unsafe or reserved names are represented by `_c_` plus
unpadded URL-safe base64 of SHA-256(canonical bytes)[0..24], a 35-byte wire
name. Descriptions retain the full canonical identity and original meaning;
schemas and annotations are unchanged. Publication registers the encoded
pairs atomically and refuses digest collisions. Only an advertised wire name
decodes, once, in that grant. Raw names requiring encoding, unknown encoded
names and previous Antigravity servers have no fallback or authority.

This codec neither truncates nor filters the canonical pool, changes the
gateway's 128-byte canonical limit, nor renames an external tool. Public MCP,
internal tool APIs and other CLI kinds retain canonical behavior. Native
SDK/UI qualification remains pending separately from pure codec verification.
