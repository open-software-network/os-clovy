//! The permission table (`design.md` §3) per `(cli, safety_mode, attended)`,
//! and a sweep of every argv the engine can build for forbidden flags.

use super::*;
use crate::chat_engine::cli::turn_invocation;
use crate::mcp_server::McpLaunch;

const MODES: [AgentSafetyMode; 2] = [AgentSafetyMode::Sandboxed, AgentSafetyMode::Unrestricted];

fn permissions(safety_mode: AgentSafetyMode, attended: bool) -> TurnPermissions {
    TurnPermissions {
        safety_mode,
        attended,
        routine_tools: (!attended).then_some(RoutineBuiltinTools {
            file: true,
            web: true,
        }),
    }
}

#[test]
fn policy_claude_routine_without_a_capability_snapshot_has_no_builtins() {
    for safety_mode in MODES {
        let policy = turn_policy(
            CliKind::Claude,
            TurnPermissions {
                safety_mode,
                attended: false,
                routine_tools: None,
            },
        );
        assert_eq!(values_of(&policy.args, "--tools"), vec![String::new()]);
        assert_eq!(
            values_of(&policy.args, "--allowedTools"),
            vec![CLAUDE_CLOVY_TOOLS.to_string()]
        );
    }
}

fn launch() -> McpLaunch {
    McpLaunch {
        command: PathBuf::from("/bin/clovy-mcp"),
        args: strings(&["--dir", "/data/mcp", "--turn", "ab12"]),
    }
}

/// Every value given to `flag` (claude's list flags take several).
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
fn policy_claude_attended_turns_ask_through_the_default_mode_and_grant_only_clovy_tools() {
    for (mode, expected) in [
        (AgentSafetyMode::Sandboxed, "default"),
        (AgentSafetyMode::Unrestricted, "acceptEdits"),
    ] {
        let policy = policy_with(CliKind::Claude, permissions(mode, true), false);
        assert_eq!(policy.args, strings(&["--permission-mode", expected]));
        assert_eq!(
            policy.mcp_args,
            strings(&["--allowedTools", "mcp__clovy__*"])
        );
        assert_eq!(policy.env, vec![("MCP_TOOL_TIMEOUT", "900000".to_string())]);

        // With `approve_action` and `request_clarification` (slice 2.11).
        let ready = policy_with(CliKind::Claude, permissions(mode, true), true);
        assert_eq!(ready.args, policy.args);
        assert_eq!(
            ready.mcp_args,
            strings(&[
                "--allowedTools",
                "mcp__clovy__*",
                "--permission-prompt-tool",
                "mcp__clovy__approve_action",
                "--disallowedTools",
                "mcp__clovy__approve_action",
                "AskUserQuestion",
            ])
        );
    }
    // On: the prompt tool names a tool the Clovy server lists to claude turns.
    for mode in MODES {
        assert_eq!(
            turn_policy(CliKind::Claude, permissions(mode, true)),
            policy_with(CliKind::Claude, permissions(mode, true), true)
        );
    }
}

/// The prompt tool the claude row names is one the Clovy server lists to a
/// claude turn.
#[tokio::test]
async fn policy_claude_interaction_tools_are_listed_to_claude_turns() {
    let broker = crate::approval_broker::testing::TestBroker::new([]).await;
    let binding = broker.turn(CliKind::Claude, true).await;
    let data = crate::mcp_server::tools::McpData {
        notes: broker.pool().clone(),
        profile: binding.data_partition.clone(),
        memory_enabled: true,
        activity: crate::mcp_server::tools::ActivityAccess::Off,
        now: chrono::Utc::now(),
    };
    let ctx = crate::mcp_server::tools::ToolCtx {
        binding: Some(&binding),
        ..crate::mcp_server::tools::ToolCtx::unbound(&data)
    };
    let listed = crate::mcp_server::tools::descriptors(&ctx).await;
    for name in ["approve_action", "request_clarification"] {
        assert!(
            listed.iter().any(|tool| tool["name"] == name),
            "{name} is not listed to a claude turn"
        );
    }
}

#[test]
fn policy_claude_unattended_turns_keep_the_read_only_set_whatever_the_safety_mode() {
    for mode in MODES {
        for interaction in [false, true] {
            let policy = policy_with(CliKind::Claude, permissions(mode, false), interaction);
            assert_eq!(
                policy.args,
                strings(&[
                    "--permission-mode",
                    "default",
                    "--tools",
                    "Read,Grep,Glob,WebSearch,WebFetch",
                    "--allowedTools",
                    "Read,Grep,Glob,WebSearch,WebFetch,mcp__clovy__*",
                    "--strict-mcp-config",
                ])
            );
            assert_eq!(
                policy.mcp_args,
                strings(&["--disallowedTools", "mcp__clovy__approve_action"])
            );
        }
    }
}

/// The Clovy server lists `approve_action` to every claude turn, but only an
/// attended turn has a permission prompt to answer: an unattended claude turn
/// keeps the tool from the model too (the contract's "only for
/// `--permission-prompt-tool`"), with no prompt tool.
#[test]
fn policy_claude_unattended_turns_keep_approve_action_from_the_model() {
    let scratch = tempfile::tempdir().unwrap();
    for mode in MODES {
        let routine = turn_invocation(
            CliKind::Claude,
            "hello",
            None,
            Some(&launch()),
            scratch.path(),
            permissions(mode, false),
            &[],
        );
        assert_eq!(
            values_of(&routine.args, "--disallowedTools"),
            [CLAUDE_PROMPT_TOOL]
        );
        assert!(values_of(&routine.args, "--permission-prompt-tool").is_empty());
        assert!(!routine
            .args
            .contains(&"--permission-prompt-tool".to_string()));
        // The list flag still ends before the read-only set's own flags.
        assert_eq!(
            values_of(&routine.args, "--allowedTools"),
            ["Read,Grep,Glob,WebSearch,WebFetch,mcp__clovy__*"]
        );
        assert_eq!(routine.stdin.as_deref(), Some("hello"));
    }
}

#[test]
fn policy_codex_full_access_requires_unrestricted_including_routines() {
    let mcp = strings(&[
        "-c",
        "mcp_servers.clovy.default_tools_approval_mode=\"approve\"",
        "-c",
        "mcp_servers.clovy.tool_timeout_sec=900",
    ]);
    let cases = [
        (
            permissions(AgentSafetyMode::Sandboxed, true),
            [
                "sandbox_mode=\"workspace-write\"",
                "sandbox_workspace_write.network_access=false",
            ]
            .as_slice(),
        ),
        (
            permissions(AgentSafetyMode::Unrestricted, true),
            ["sandbox_mode=\"danger-full-access\""].as_slice(),
        ),
        (
            permissions(AgentSafetyMode::Sandboxed, false),
            ["sandbox_mode=\"read-only\""].as_slice(),
        ),
        (
            permissions(AgentSafetyMode::Unrestricted, false),
            ["sandbox_mode=\"danger-full-access\""].as_slice(),
        ),
    ];
    for (permissions, sandbox) in cases {
        let policy = turn_policy(CliKind::Codex, permissions);
        let mut expected = sandbox.to_vec();
        expected.extend(["approval_policy=\"never\"", "web_search=\"live\""]);
        assert_eq!(
            policy.args,
            config_args(expected.into_iter().map(str::to_string)),
            "{permissions:?}"
        );
        assert_eq!(policy.mcp_args, mcp, "{permissions:?}");
        assert!(policy.env.is_empty());
    }
}

#[test]
fn policy_other_clis_get_no_flags() {
    for kind in [
        CliKind::Pi,
        CliKind::Agy,
        CliKind::CursorAgent,
        CliKind::Copilot,
    ] {
        for mode in MODES {
            for attended in [true, false] {
                assert_eq!(
                    turn_policy(kind, permissions(mode, attended)),
                    TurnPolicy::default()
                );
            }
        }
    }
}

#[test]
fn policy_tool_timeouts_outlast_the_approval_wait() {
    assert!(CLI_TOOL_TIMEOUT > APPROVAL_TIMEOUT);
    let claude = turn_policy(
        CliKind::Claude,
        permissions(AgentSafetyMode::Sandboxed, true),
    );
    let millis: u64 = claude.env[0].1.parse().unwrap();
    assert!(Duration::from_millis(millis) > APPROVAL_TIMEOUT);
    let codex = turn_policy(
        CliKind::Codex,
        permissions(AgentSafetyMode::Sandboxed, true),
    );
    let seconds: u64 = codex
        .mcp_args
        .iter()
        .find_map(|arg| arg.strip_prefix("mcp_servers.clovy.tool_timeout_sec="))
        .unwrap()
        .parse()
        .unwrap();
    assert!(Duration::from_secs(seconds) > APPROVAL_TIMEOUT);
}

#[test]
fn policy_codex_gets_one_image_flag_per_image_before_its_other_flags() {
    let images = [
        PathBuf::from("/ws/attachments/a.png"),
        PathBuf::from("/ws/attachments/b, c.jpg"),
        PathBuf::from("/ws/attachments/d.webp"),
    ];
    assert!(image_args(CliKind::Claude, &images).is_empty());
    let scratch = tempfile::tempdir().unwrap();
    for conversation in [None, Some("thread-1")] {
        let invocation = turn_invocation(
            CliKind::Codex,
            "hello",
            conversation,
            Some(&launch()),
            scratch.path(),
            permissions(AgentSafetyMode::Sandboxed, true),
            &images,
        );
        let args = &invocation.args;
        let at = |value: &str| args.iter().position(|arg| arg == value).unwrap();
        assert_eq!(
            values_of(args, "--image"),
            ["/ws/attachments/a.png", "/ws/attachments/d.webp"]
        );
        assert_eq!(args.iter().filter(|arg| *arg == "--image").count(), 2);
        // `exec --image` takes every value that follows: a flag must end it
        // before the resume id and the stdin marker.
        assert!(at("/ws/attachments/d.webp") < at("-c"));
        assert_eq!(args.last().map(String::as_str), Some("-"));
    }
}

#[test]
fn policy_no_turn_ever_carries_a_forbidden_flag() {
    let scratch = tempfile::tempdir().unwrap();
    let images = [PathBuf::from("/ws/attachments/a.png")];
    let mut checked = 0;
    for kind in CliKind::ALL {
        for mode in MODES {
            for attended in [true, false] {
                let permissions = permissions(mode, attended);
                for interaction in [false, true] {
                    let policy = policy_with(kind, permissions, interaction);
                    for arg in policy.args.iter().chain(&policy.mcp_args) {
                        if arg == "sandbox_mode=\"danger-full-access\"" {
                            assert_eq!(kind, CliKind::Codex);
                            assert_eq!(mode, AgentSafetyMode::Unrestricted);
                        } else {
                            assert_eq!(forbidden_in(arg), None, "{kind:?} {permissions:?}: {arg}");
                        }
                    }
                }
                for mcp in [None, Some(launch())] {
                    for conversation in [None, Some("id-1")] {
                        let invocation = turn_invocation(
                            kind,
                            "hello --dangerously-skip-permissions",
                            conversation,
                            mcp.as_ref(),
                            scratch.path(),
                            permissions,
                            &images,
                        );
                        let args = &invocation.args;
                        // The prompt is data: only agy, cursor-agent, and
                        // copilot take it as an argument.
                        for arg in args.iter().filter(|arg| !arg.contains("hello")) {
                            if arg == "sandbox_mode=\"danger-full-access\"" {
                                assert_eq!(kind, CliKind::Codex);
                                assert_eq!(mode, AgentSafetyMode::Unrestricted);
                            } else {
                                assert_eq!(
                                    forbidden_in(arg),
                                    None,
                                    "{kind:?} {permissions:?}: {arg}"
                                );
                            }
                        }
                        for mode in values_of(args, "--permission-mode") {
                            assert!(
                                ["default", "acceptEdits"].contains(&mode.as_str()),
                                "{mode}"
                            );
                        }
                        for (_, value) in &invocation.set_env {
                            assert_eq!(forbidden_in(value), None);
                        }
                        // Attended turns grant only Clovy's tools.
                        if attended {
                            for allowed in values_of(args, "--allowedTools") {
                                assert_eq!(allowed, "mcp__clovy__*", "{kind:?}");
                            }
                            assert!(!args.contains(&"--strict-mcp-config".to_string()));
                        }
                        // Every Codex turn explicitly selects its sandbox policy.
                        if kind == CliKind::Codex {
                            assert!(args.iter().any(|arg| arg.starts_with("sandbox_mode=")));
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, CliKind::ALL.len() * 2 * 2 * 2 * 2);
}

#[test]
fn policy_flags_reach_the_argv_with_the_prompt_on_stdin() {
    let scratch = tempfile::tempdir().unwrap();
    let claude = turn_invocation(
        CliKind::Claude,
        "hello",
        Some("conv-1"),
        Some(&launch()),
        scratch.path(),
        permissions(AgentSafetyMode::Unrestricted, true),
        &[],
    );
    // S1: the list flags take every value that follows, so nothing
    // positional comes after them; the prompt goes on stdin.
    assert_eq!(claude.stdin.as_deref(), Some("hello"));
    assert!(!claude.args.contains(&"hello".to_string()));
    assert_eq!(
        values_of(&claude.args, "--permission-mode"),
        ["acceptEdits"]
    );
    assert_eq!(values_of(&claude.args, "--allowedTools"), ["mcp__clovy__*"]);
    assert_eq!(
        claude.set_env,
        vec![("MCP_TOOL_TIMEOUT", "900000".to_string())]
    );
    // Without Clovy's server there is nothing to grant.
    let bare = turn_invocation(
        CliKind::Claude,
        "hello",
        None,
        None,
        scratch.path(),
        permissions(AgentSafetyMode::Sandboxed, true),
        &[],
    );
    assert!(values_of(&bare.args, "--allowedTools").is_empty());
    assert_eq!(values_of(&bare.args, "--permission-mode"), ["default"]);

    let codex = turn_invocation(
        CliKind::Codex,
        "hello",
        None,
        Some(&launch()),
        scratch.path(),
        permissions(AgentSafetyMode::Sandboxed, true),
        &[],
    );
    assert_eq!(codex.stdin.as_deref(), Some("hello"));
    let config = values_of(&codex.args, "-c");
    // The server's own settings follow its command and args.
    let command = config
        .iter()
        .position(|value| value.starts_with("mcp_servers.clovy.command="))
        .unwrap();
    let approve = config
        .iter()
        .position(|value| value.starts_with("mcp_servers.clovy.default_tools_approval_mode="))
        .unwrap();
    assert!(command < approve);
    let unbound = turn_invocation(
        CliKind::Codex,
        "hello",
        None,
        None,
        scratch.path(),
        permissions(AgentSafetyMode::Sandboxed, true),
        &[],
    );
    assert!(!unbound.args.iter().any(|arg| arg.contains("mcp_servers")));
}
