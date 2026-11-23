//! Detection of the six agent CLIs on the user's PATH (the login shell's on macOS,
//! the app's own on Windows; see [`super::shell_env`]), with the
//! resolved path and the version each CLI reports.

use super::{
    cli::CliKind,
    process::{self, ProcessSpec},
    shell_env::LoginEnv,
    StructuredOutputLevel,
};
use futures_util::future::{BoxFuture, FutureExt, Shared};
use serde::Serialize;
use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

const VERSION_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_VERSION_CHARS: usize = 80;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    pub id: CliKind,
    pub name: String,
    pub installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub structured_output: StructuredOutputLevel,
    /// The CLI can run one-shot calls (every tool off, or its least-privileged
    /// mode); only those may be chosen for notes and activity.
    pub one_shot: bool,
}

/// Detects every CLI concurrently, in catalog order.
///
/// Several screens ask at once (on startup the chat picker, the workspace,
/// and the note chat); they share one probe in flight and its result for
/// [`SHARED_PROBE_TTL`], so each CLI is started once instead of once per
/// screen.
pub async fn detect_all(env: &LoginEnv) -> Vec<CliStatus> {
    shared_probe(env).await
}

type ProbeFuture = Shared<BoxFuture<'static, Vec<CliStatus>>>;

/// How long a finished probe answers later callers with the same environment.
const SHARED_PROBE_TTL: Duration = Duration::from_secs(5);

struct SharedProbe {
    env: LoginEnv,
    started: Instant,
    probe: ProbeFuture,
}

static SHARED_PROBE: Mutex<Option<SharedProbe>> = Mutex::new(None);

fn shared_probe(env: &LoginEnv) -> ProbeFuture {
    let mut shared = SHARED_PROBE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(current) = shared.as_ref() {
        let pending = current.probe.peek().is_none();
        if current.env == *env && (pending || current.started.elapsed() < SHARED_PROBE_TTL) {
            return current.probe.clone();
        }
    }
    let owned = env.clone();
    let probe = async move {
        futures_util::future::join_all(CliKind::ALL.into_iter().map(|kind| detect(kind, &owned)))
            .await
    }
    .boxed()
    .shared();
    *shared = Some(SharedProbe {
        env: env.clone(),
        started: Instant::now(),
        probe: probe.clone(),
    });
    probe
}

pub async fn detect(kind: CliKind, env: &LoginEnv) -> CliStatus {
    let Some(path) = env.which(kind.id()) else {
        return CliStatus {
            id: kind,
            name: kind.display_name().to_string(),
            installed: false,
            path: None,
            version: None,
            reason: Some(format!("`{}` was not found on your PATH.", kind.id())),
            structured_output: kind.structured_output(),
            one_shot: kind.one_shot_allowed(),
        };
    };
    let version = read_version(kind, &path, env).await;
    CliStatus {
        id: kind,
        name: kind.display_name().to_string(),
        installed: true,
        path: Some(path.to_string_lossy().into_owned()),
        version,
        reason: None,
        structured_output: kind.structured_output(),
        one_shot: kind.one_shot_allowed(),
    }
}

async fn read_version(kind: CliKind, path: &std::path::Path, env: &LoginEnv) -> Option<String> {
    let managed = if kind == CliKind::Agy {
        Some(super::agy::HostContext::current().ok()?.one_shot().ok()?)
    } else {
        None
    };
    let scratch = if managed.is_none() {
        Some(tempfile::tempdir().ok()?)
    } else {
        None
    };
    let cwd = managed
        .as_ref()
        .map(|execution| execution.cwd())
        .or_else(|| scratch.as_ref().map(|scratch| scratch.path()))?;
    let child_env = super::shell_env::cli_environment(env.vars());
    let child_env = managed
        .as_ref()
        .map(|execution| execution.environment(&child_env))
        .unwrap_or(child_env);
    let program = if kind == CliKind::Agy {
        std::fs::canonicalize(path).ok()?
    } else {
        path.to_path_buf()
    };
    let output = process::run(ProcessSpec {
        program,
        args: vec!["--version".to_string()],
        env: child_env,
        cwd: cwd.to_path_buf(),
        stdin: None,
        timeout: VERSION_TIMEOUT,
    })
    .await
    .ok()?;
    output
        .stdout
        .lines()
        .chain(output.stderr.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(MAX_VERSION_CHARS).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use crate::llm::shell_env::resolve_login_env;
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::{os::unix::fs::PermissionsExt, path::Path};

    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A fake login shell: like a profile, it prepends `extra_dir` to PATH
    /// and exports a profile variable, then runs the `-c` script.
    #[cfg(unix)]
    fn fake_login_shell(dir: &Path, extra_dir: &Path) -> std::path::PathBuf {
        let shell = dir.join("fake-shell");
        write_script(
            &shell,
            &format!(
                "export PATH=\"{}:$PATH\"\nexport PI_CODING_AGENT_DIR=\"$HOME/.pi/agent\"\nshift\nexec /bin/sh -c \"$2\"",
                extra_dir.display()
            ),
        );
        shell
    }

    #[cfg(unix)]
    fn gui_app_env(home: &Path) -> BTreeMap<String, String> {
        BTreeMap::from([
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
            ("HOME".to_string(), home.to_string_lossy().into_owned()),
        ])
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn llm_cli_found_only_through_the_login_shell_path() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("local-bin");
        std::fs::create_dir(&bin).unwrap();
        write_script(&bin.join("claude"), "echo '2.1.0 (Claude Code)'");
        let shell = fake_login_shell(root.path(), &bin);
        let process_env = gui_app_env(root.path());

        // The GUI app's own PATH does not see the CLI.
        assert!(LoginEnv::from_vars(process_env.clone())
            .which("claude")
            .is_none());

        let env = resolve_login_env(&shell, &process_env).await;
        let status = detect(CliKind::Claude, &env).await;
        assert!(status.installed);
        assert_eq!(
            status.path.as_deref(),
            Some(bin.join("claude").to_string_lossy().as_ref())
        );
        assert_eq!(status.version.as_deref(), Some("2.1.0 (Claude Code)"));
        // The profile's environment is preserved for the CLIs Clovy runs.
        assert_eq!(
            env.vars().get("PI_CODING_AGENT_DIR").map(String::as_str),
            Some(format!("{}/.pi/agent", root.path().display()).as_str())
        );
        assert_eq!(
            env.vars().get("HOME").map(String::as_str),
            Some(root.path().to_string_lossy().as_ref())
        );
    }

    #[tokio::test]
    async fn llm_missing_cli_is_reported_with_a_reason() {
        let root = tempfile::tempdir().unwrap();
        let empty = root.path().join("empty-bin");
        std::fs::create_dir(&empty).unwrap();
        let env = LoginEnv::from_vars(BTreeMap::from([(
            "PATH".to_string(),
            empty.to_string_lossy().into_owned(),
        )]));
        let status = detect(CliKind::CursorAgent, &env).await;
        assert!(!status.installed);
        assert!(status.path.is_none());
        assert!(status.reason.unwrap().contains("cursor-agent"));
    }

    #[tokio::test]
    async fn llm_non_executable_file_is_not_a_cli() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("codex"), "not a program").unwrap();
        let env = LoginEnv::from_vars(BTreeMap::from([(
            "PATH".to_string(),
            root.path().to_string_lossy().into_owned(),
        )]));
        assert!(!detect(CliKind::Codex, &env).await.installed);
    }

    /// Several screens ask for the catalog at once on startup; each CLI is
    /// probed once for all of them.
    #[tokio::test]
    #[cfg(unix)]
    async fn llm_concurrent_detection_probes_each_cli_once() {
        let root = tempfile::tempdir().unwrap();
        let log = root.path().join("probes.log");
        for name in ["claude", "codex"] {
            write_script(
                &root.path().join(name),
                &format!(
                    "echo {name} >> '{}'\nsleep 0.2\necho '1.0 ({name})'",
                    log.display()
                ),
            );
        }
        let env = LoginEnv::from_vars(BTreeMap::from([(
            "PATH".to_string(),
            root.path().to_string_lossy().into_owned(),
        )]));
        let (first, second, third) =
            tokio::join!(detect_all(&env), detect_all(&env), detect_all(&env));
        assert_eq!(first, second);
        assert_eq!(second, third);
        assert!(first
            .iter()
            .any(|cli| cli.installed && cli.id == CliKind::Codex));
        let probes = std::fs::read_to_string(&log).unwrap();
        let mut probes: Vec<&str> = probes.lines().collect();
        probes.sort_unstable();
        assert_eq!(probes, ["claude", "codex"]);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn llm_broken_login_shell_falls_back_to_the_app_environment() {
        let root = tempfile::tempdir().unwrap();
        let shell = root.path().join("broken-shell");
        write_script(&shell, "exit 3");
        let env = resolve_login_env(&shell, &gui_app_env(root.path())).await;
        let path = env.vars().get("PATH").unwrap();
        assert!(path.starts_with("/usr/bin:/bin"));
        assert!(path.contains(".local/bin"), "{path}");
    }
}
