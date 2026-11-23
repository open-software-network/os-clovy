//! `clovy-mcp`: the stdio MCP server Claude Code, Cursor, and the CLI chat
//! engines spawn. It relays to the running Clovy app; all behavior lives in
//! `clovy_lib::mcp_server::stdio` so tests drive the same code.

fn main() {
    std::process::exit(clovy_lib::mcp_server::stdio::run());
}
