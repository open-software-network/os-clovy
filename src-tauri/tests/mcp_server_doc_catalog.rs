//! `docs/mcp-server.md` against what the server actually lists: the tool
//! tables of the doc must match `tools/list` for an unbound client and for
//! each kind of turn-bound connection (task 2.12 of the OpenSpec change
//! `cli-substitui-agente-nativo`), Claude-only tools must stay out of the
//! model's reach in every Claude turn. Removing, adding, or renaming a tool
//! without updating the documented catalog fails here.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use clovy_lib::agent_sessions::domain::AgentSafetyMode;
use clovy_lib::chat_engine::policy::{turn_policy, TurnPermissions};
use clovy_lib::connectors::agent_tools::routine_tool_allowed;
use clovy_lib::connectors::notion::runtime_name_to_provider;
use clovy_lib::connectors::policy::{
    JUNE_GCAL_ACTIONS_SERVER, JUNE_GCAL_SERVER, JUNE_GITHUB_ACTIONS_SERVER, JUNE_GITHUB_SERVER,
    JUNE_GMAIL_ACTIONS_SERVER, JUNE_GMAIL_SERVER,
};
use clovy_lib::llm::cli::CliKind;
use clovy_lib::mcp_server::protocol;
use clovy_lib::mcp_server::tools::{is_known_tool, ActivityAccess, McpData, ToolCtx};
use clovy_lib::mcp_server::turns::TurnBinding;
use serde_json::{json, Value};
use sqlx_sqlite::SqlitePool;

const DOC: &str = "docs/mcp-server.md";
const EVERY_CLIENT: &str = "### Tools for every client";
const TURN_BOUND: &str = "### Tools for turn-bound connections";
const CONNECTORS: &str = "### Connector tools";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn doc() -> String {
    std::fs::read_to_string(repo_root().join(DOC)).unwrap()
}

/// The lines of the `###` section titled `heading`, up to the next `##` or
/// `###` heading.
fn section<'a>(doc: &'a str, heading: &str) -> Vec<&'a str> {
    let mut lines = doc.lines().skip_while(|line| line.trim_end() != heading);
    assert!(lines.next().is_some(), "{DOC} has no section `{heading}`");
    lines
        .take_while(|line| !line.starts_with("## ") && !line.starts_with("### "))
        .collect()
}

/// One table row: the tool name (the first cell, in backticks) and the
/// other cells, trimmed. The last cell is free text and may hold `|`
/// inside code spans, so callers read only the leading columns.
struct Row {
    tool: String,
    cells: Vec<String>,
}

fn rows(lines: &[&str]) -> Vec<Row> {
    let rows: Vec<Row> = lines
        .iter()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("| `")?;
            let (tool, _) = rest.split_once('`')?;
            let cells = line
                .trim_matches('|')
                .split('|')
                .skip(1)
                .map(|cell| cell.trim().to_string())
                .collect();
            Some(Row {
                tool: tool.to_string(),
                cells,
            })
        })
        .collect();
    let mut seen = BTreeSet::new();
    for row in &rows {
        assert!(
            seen.insert(row.tool.clone()),
            "`{}` is listed twice",
            row.tool
        );
    }
    rows
}

/// The index of `name` among a table's columns after `Tool`.
fn column(lines: &[&str], name: &str) -> usize {
    let header = lines
        .iter()
        .find(|line| line.trim_start().starts_with("| Tool |"))
        .expect("table header");
    header
        .trim()
        .trim_matches('|')
        .split('|')
        .skip(1)
        .position(|cell| cell.trim() == name)
        .unwrap_or_else(|| panic!("no `{name}` column in `{header}`"))
}

async fn notes_pool() -> SqlitePool {
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    clovy_lib::db::migrations::run_migrations(&pool)
        .await
        .unwrap();
    pool
}

fn data(notes: &SqlitePool) -> McpData {
    McpData {
        notes: notes.clone(),
        profile: "default".into(),
        memory_enabled: true,
        // Capture on: the activity tools are part of the listed catalog.
        activity: ActivityAccess::On,
        now: Utc.with_ymd_and_hms(2026, 10, 17, 9, 0, 0).unwrap(),
    }
}

fn binding(cli: CliKind, attended: bool) -> TurnBinding {
    host_binding::binding(cli, attended, "session-doc", "run-doc")
}

#[path = "support/mcp_binding.rs"]
mod host_binding;

async fn request(ctx: &ToolCtx<'_>, method: &str, params: Value) -> Value {
    let message = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    protocol::handle_in(ctx, &message)
        .await
        .expect("a request gets an answer")
}

/// `tools/list` without app, broker, or connected accounts: connector and
/// gateway tools are discovered per account and server, so none are listed
/// here (they are checked by name below).
async fn listed(notes: &SqlitePool, binding: Option<&TurnBinding>) -> BTreeSet<String> {
    let data = data(notes);
    let ctx = ToolCtx {
        data: &data,
        app: None,
        binding,
        broker: None,
    };
    let answer = request(&ctx, "tools/list", json!({})).await;
    let tools = answer["result"]["tools"].as_array().expect("tools");
    let names: Vec<String> = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap().to_string())
        .collect();
    let set: BTreeSet<String> = names.iter().cloned().collect();
    assert_eq!(
        set.len(),
        names.len(),
        "tools/list repeats a name: {names:?}"
    );
    set
}

fn listed_to(audience: &str, binding: &TurnBinding) -> bool {
    match audience {
        "every turn" => true,
        "attended turns" => binding.attended,
        "attended or routine turns" => binding.attended || binding.surface == "routine",
        "Claude turns" => binding.cli == CliKind::Claude,
        other => panic!("unknown `Listed to` value `{other}`"),
    }
}

#[tokio::test]
async fn every_client_table_is_the_unbound_catalog() {
    let doc = doc();
    let documented: BTreeSet<String> = rows(&section(&doc, EVERY_CLIENT))
        .into_iter()
        .map(|row| row.tool)
        .collect();
    let notes = notes_pool().await;
    assert_eq!(listed(&notes, None).await, documented);
}

#[tokio::test]
async fn turn_bound_table_is_the_bound_catalog_for_each_kind_of_turn() {
    let doc = doc();
    let every_client: BTreeSet<String> = rows(&section(&doc, EVERY_CLIENT))
        .into_iter()
        .map(|row| row.tool)
        .collect();
    let lines = section(&doc, TURN_BOUND);
    let audience = column(&lines, "Listed to");
    let bound = rows(&lines);
    assert!(!bound.is_empty());
    let notes = notes_pool().await;
    // The tables describe the available catalog. Routine reads and skill
    // discovery also require the saved toolsets; a missing routine fails closed.
    let session = clovy_lib::agent_sessions::AgentRepository::new(notes.clone())
        .create_session(
            "Catalog",
            "__clovy_cli_engine__:claude",
            AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let chat_session = clovy_lib::agent_sessions::AgentRepository::new(notes.clone())
        .create_session(
            "Attended catalog",
            "__clovy_cli_engine__:claude",
            AgentSafetyMode::Sandboxed,
            None,
        )
        .await
        .unwrap();
    let now = "2026-10-29T00:00:00Z";
    sqlx::query::query("INSERT INTO routines (id, name, prompt, schedule, created_at, updated_at, metadata_json, tool_catalog_version) VALUES ('doc-routine', 'Catalog', 'List', 'every 1d', ?, ?, '{\"enabledToolsets\":[\"context_engine\",\"skills\"]}', 1)")
        .bind(now).bind(now).execute(&notes).await.unwrap();
    sqlx::query::query("INSERT INTO routine_runs (id, routine_id, agent_session_id, claim_token, trigger_kind, status, model, safety_mode, created_at, updated_at) VALUES ('doc-run', 'doc-routine', ?, 'claim', 'schedule', 'running', 'claude', 'sandboxed', ?, ?)")
        .bind(&session.id).bind(now).bind(now).execute(&notes).await.unwrap();
    for mut turn in [
        binding(CliKind::Claude, true),
        binding(CliKind::Claude, false),
        binding(CliKind::Codex, true),
        binding(CliKind::Codex, false),
        binding(CliKind::Copilot, true),
    ] {
        turn.session_id = if turn.attended {
            chat_session.id.clone()
        } else {
            session.id.clone()
        };
        let mut expected = every_client.clone();
        expected.extend(
            bound
                .iter()
                .filter(|row| listed_to(&row.cells[audience], &turn))
                .map(|row| row.tool.clone()),
        );
        if !turn.attended {
            // This fixture has context_engine + skills, not mutation grants
            // or the routines history toolset. The table describes the
            // available surface; saved policy narrows the routine catalog.
            expected.retain(|name| {
                every_client.contains(name)
                    || matches!(
                        name.as_str(),
                        "get_dictation"
                            | "list_notes"
                            | "search_memories"
                            | "list_conversations"
                            | "get_conversation"
                            | "get_conversation_run"
                            | "list_conversation_messages"
                            | "search_conversation_messages"
                            | "list_conversation_artifacts"
                            | "read_conversation_artifact"
                            | "list_skills"
                            | "load_skill"
                            | "get_obsidian_vault"
                            | "request_clarification"
                            | "approve_action"
                    )
            });
        }
        assert_eq!(
            listed(&notes, Some(&turn)).await,
            expected,
            "{:?} turn, attended {}",
            turn.cli,
            turn.attended
        );
    }
}

/// Every value given to `flag` in an argv (claude's list flags take several).
fn values_of(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut taking = false;
    for arg in args {
        if arg.starts_with('-') && arg.len() > 1 {
            taking = arg == flag;
            continue;
        }
        if taking {
            values.push(arg.clone());
        }
    }
    values
}

#[test]
fn claude_only_tools_are_kept_from_the_model_in_every_claude_turn() {
    let doc = doc();
    let lines = section(&doc, TURN_BOUND);
    let audience = column(&lines, "Listed to");
    let claude_only: Vec<String> = rows(&lines)
        .into_iter()
        .filter(|row| row.cells[audience] == "Claude turns")
        .map(|row| format!("mcp__clovy__{}", row.tool))
        .collect();
    assert!(!claude_only.is_empty());
    for safety_mode in [AgentSafetyMode::Sandboxed, AgentSafetyMode::Unrestricted] {
        for attended in [true, false] {
            let policy = turn_policy(
                CliKind::Claude,
                TurnPermissions {
                    safety_mode,
                    attended,
                    routine_tools: None,
                },
            );
            let mut args = policy.mcp_args.clone();
            args.extend(policy.args);
            let disallowed = values_of(&args, "--disallowedTools");
            for tool in &claude_only {
                assert!(
                    disallowed.contains(tool),
                    "{tool} reaches the model ({safety_mode:?}, attended {attended}): {args:?}"
                );
            }
        }
    }
}

#[tokio::test]
async fn turn_bound_tools_answer_turn_required_without_a_turn() {
    let doc = doc();
    let notes = notes_pool().await;
    let data = data(&notes);
    let ctx = ToolCtx::unbound(&data);
    let bound = rows(&section(&doc, TURN_BOUND));
    let connectors = rows(&section(&doc, CONNECTORS));
    for row in bound.iter().chain(&connectors) {
        let answer = request(
            &ctx,
            "tools/call",
            json!({ "name": row.tool, "arguments": {} }),
        )
        .await;
        let result = &answer["result"];
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        assert_eq!(result["isError"], true, "`{}`: {answer}", row.tool);
        assert!(
            text.ends_with("(turn_required)"),
            "`{}` answered {text:?}",
            row.tool
        );
    }
}

#[test]
fn connector_table_names_tools_the_server_owns_with_their_approval() {
    let doc = doc();
    let lines = section(&doc, CONNECTORS);
    let broker = column(&lines, "Broker");
    let connectors = rows(&lines);
    assert!(!connectors.is_empty());
    let reads = [
        JUNE_GMAIL_SERVER,
        JUNE_GCAL_SERVER,
        JUNE_GITHUB_SERVER,
        "june_notion",
    ]
    .map(String::from);
    let actions = [
        JUNE_GMAIL_ACTIONS_SERVER,
        JUNE_GCAL_ACTIONS_SERVER,
        JUNE_GITHUB_ACTIONS_SERVER,
        "june_notion_actions",
    ]
    .map(String::from);
    for row in &connectors {
        let tool = row.tool.as_str();
        assert!(is_known_tool(tool), "`{tool}` is not a server tool");
        let mutation = match row.cells[broker].as_str() {
            "yes" => true,
            "no" => false,
            other => panic!("`{tool}`: unknown `Broker` value `{other}`"),
        };
        if tool.starts_with("notion-") {
            assert_eq!(
                runtime_name_to_provider(tool).map(|(_, action)| action),
                Some(mutation),
                "`{tool}`"
            );
        } else {
            // A read is allowed by the read toolset alone; a mutation needs
            // the actions toolset (the same split the server's broker uses).
            assert_eq!(
                routine_tool_allowed(tool, &reads, &[], Some("approval")),
                !mutation,
                "`{tool}` with the read toolsets"
            );
            assert!(
                routine_tool_allowed(tool, &actions, &[], Some("approval")) == mutation,
                "`{tool}` with the actions toolsets"
            );
        }
    }
}
