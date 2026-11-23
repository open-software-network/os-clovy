# Agent note tools

CLI chat turns read notes, transcripts, and dictations through the Clovy MCP
server. The tool catalog and exact paging contracts live in
[mcp-server.md](mcp-server.md); names follow
[spec/mcp-tool-naming.md](../spec/mcp-tool-naming.md). The implementation is in
`src-tauri/src/mcp_server/tools/read.rs` and `tools/notes.rs`.

| Tool | Availability | Purpose |
|---|---|---|
| `search_notes` | Every client | Find notes by title, text, or transcript; bounded results and snippets. |
| `get_note` | Every client | Read a note and its authoritative transcripts with character-based paging. |
| `list_dictations` | Every client | Read recent retained dictations under the server's retention bounds. |
| `get_dictation` | Bound CLI turns | Read one retained dictation with character-based paging. |
| `list_note_transcripts` | Every client | Page authoritative transcript records for one note. |
| `list_notes` | Bound CLI turns | Page notes in the bound data partition. |
| `create_note` | Bound CLI turns | Create a note in the bound data partition. |
| `update_note` | Bound CLI turns | Update a note only at the expected revision. |
| `delete_note` | Bound CLI turns | Delete a note and its owned recording files. |

A note reference in a chat message asks the CLI to read it with `get_note`.
Bound reads use the turn's data partition; unbound reads use the current data partition. Mutating note tools are available only
to an authenticated bound turn and require the approval broker before their
side effects. Unbound external clients remain read-only. Cancellation revokes
the turn binding and settles pending approvals.

Historical records may contain the retired `search_june` name; retaining those
records does not expose a second tool catalog or execute a historical call.
Skills and routines use the same MCP surface under their existing policy.
See [ADR-0072](adr/0072-cli-chat-replaces-the-native-agent.md).
