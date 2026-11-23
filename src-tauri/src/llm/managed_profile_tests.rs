use super::*;
use std::{collections::BTreeMap, os::unix::fs::PermissionsExt, path::PathBuf};

struct Fixture {
    root: tempfile::TempDir,
    host: agy::HostContext,
    env: shell_env::LoginEnv,
}
impl Fixture {
    fn new(body: &str) -> Self {
        let root = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let bin = root.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let personal = root.path().join("personal");
        std::fs::create_dir(&personal).unwrap();
        std::fs::create_dir_all(personal.join(".gemini/config")).unwrap();
        std::fs::write(
            personal.join(".gemini/config/mcp_config.json"),
            r#"{"mcpServers":{"personal":{"command":"must-never-run"}}}"#,
        )
        .unwrap();
        std::fs::write(personal.join("personal-marker"), b"personal unchanged").unwrap();
        let program = bin.join("agy");
        std::fs::write(&program, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let vars = BTreeMap::from([
            ("HOME".into(), personal.display().to_string()),
            ("PATH".into(), format!("{}:/usr/bin:/bin", bin.display())),
            ("GEMINI_API_KEY".into(), "synthetic-api-billing".into()),
            ("ANTHROPIC_API_KEY".into(), "synthetic-other-key".into()),
        ]);
        let host = agy::HostContext::new(root.path().canonicalize().unwrap(), &vars).unwrap();
        Self {
            root,
            host,
            env: shell_env::LoginEnv::from_vars(vars),
        }
    }
    fn home(&self) -> PathBuf {
        self.root.path().join("runtime-profiles/agy/home")
    }
    async fn call(&self, timeout: Duration) -> Result<String, LlmError> {
        run_cli_in_context(
            CliKind::Agy,
            &GenerateRequest {
                prompt: "Synthetic note content".into(),
                timeout: Some(timeout),
                ..Default::default()
            },
            &self.env,
            Some(&self.host),
            None,
        )
        .await
    }
}

#[tokio::test]
async fn managed_oneshot_uses_shared_native_state_but_exclusive_disposable_workspaces() {
    let f = Fixture::new(
        r#"
[ -z "${GEMINI_API_KEY:-}${ANTHROPIC_API_KEY:-}" ] || exit 51
[ ! -f "$HOME/personal-marker" ] || exit 52
[ ! -f .agents/mcp_config.json ] || exit 53
[ "$CLOVY_ONESHOT" = 1 ] || exit 54
printf '%s\n' "$PWD" >> "$HOME/observed-workspaces"
if [ -f "$HOME/synthetic-auth" ]; then response=retained; else printf native > "$HOME/synthetic-auth"; response=created; fi
printf '{"response":"%s"}\n' "$response"
"#,
    );
    assert_eq!(f.call(Duration::from_secs(5)).await.unwrap(), "created");
    assert_eq!(f.call(Duration::from_secs(5)).await.unwrap(), "retained");
    let workspaces = std::fs::read_to_string(f.home().join("observed-workspaces")).unwrap();
    let paths: Vec<_> = workspaces.lines().map(PathBuf::from).collect();
    assert_eq!(paths.len(), 2);
    assert_ne!(paths[0], paths[1]);
    assert!(paths
        .iter()
        .all(|path| path.is_absolute() && !path.exists()));
    assert_eq!(
        std::fs::read(f.home().join("synthetic-auth")).unwrap(),
        b"native"
    );
    assert_eq!(
        std::fs::read(f.root.path().join("personal/personal-marker")).unwrap(),
        b"personal unchanged"
    );
}

#[tokio::test]
async fn managed_oneshot_timeout_kills_descendants_and_keeps_native_authentication() {
    let f = Fixture::new(
        r#"
printf '%s' "$PWD" > "$HOME/timeout-workspace"
/bin/sh -c '
printf "%s" "$$" > "$HOME/descendant-ready.tmp"
mv "$HOME/descendant-ready.tmp" "$HOME/descendant-ready"
while [ ! -f "$HOME/release-descendant" ]; do sleep 0.01; done
printf escaped > "$HOME/escaped-descendant"
' &
printf '%s' "$$" > "$HOME/cli-ready"
wait
"#,
    );
    let preparation = f.host.one_shot().unwrap();
    let home = preparation.home().to_path_buf();
    std::fs::write(home.join("synthetic-auth"), b"native").unwrap();
    drop(preparation);
    let readiness = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let pid = |name| {
                std::fs::read_to_string(home.join(name))
                    .ok()?
                    .parse::<i32>()
                    .ok()
                    .filter(|pid| *pid > 1)
            };
            if let (Some(cli), Some(descendant)) = (pid("cli-ready"), pid("descendant-ready")) {
                // SAFETY: signal0 observes only these fixture-reported PIDs;
                // getpgid confirms the descendant belongs to this CLI's group.
                if unsafe { libc::kill(cli, 0) == 0 && libc::kill(descendant, 0) == 0 } {
                    return (cli, descendant, unsafe { libc::getpgid(descendant) });
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    });
    // Always await the call even if readiness fails, so the real timeout sweeps
    // its group rather than abandoning an in-flight fixture on assertion panic.
    let (result, ready) = tokio::join!(f.call(Duration::from_secs(10)), readiness);
    assert_eq!(result, Err(LlmError::TimedOut));
    let (cli, descendant, group) =
        ready.expect("CLI/descendant readiness was not established before the timeout window");
    assert_ne!(cli, descendant);
    assert_eq!(
        group, cli,
        "the observed child must belong to the owned process group"
    );
    let stopped = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            // SAFETY: existence checks only, following the process runner tests.
            if unsafe { libc::kill(cli, 0) != 0 && libc::kill(descendant, 0) != 0 } {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok();
    if !stopped {
        // Keep a failing test from leaking its own fixture; retain the failure.
        unsafe {
            libc::killpg(cli, libc::SIGKILL);
        }
    }
    assert!(
        stopped,
        "the ready CLI {cli}/descendant {descendant} survived timeout"
    );
    std::fs::write(home.join("release-descendant"), b"released after timeout").unwrap();
    assert!(!f.home().join("escaped-descendant").exists());
    assert_eq!(
        std::fs::read(f.home().join("synthetic-auth")).unwrap(),
        b"native"
    );
    let cwd = std::fs::read_to_string(f.home().join("timeout-workspace")).unwrap();
    assert!(!std::path::Path::new(&cwd).exists());
}

#[tokio::test]
async fn managed_oneshot_invalid_output_is_an_error_without_another_provider_or_auth_cleanup() {
    let f = Fixture::new("printf native > \"$HOME/synthetic-auth\"\nprintf invalid");
    assert!(f.call(Duration::from_secs(5)).await.is_err());
    assert_eq!(
        std::fs::read(f.home().join("synthetic-auth")).unwrap(),
        b"native"
    );
}
