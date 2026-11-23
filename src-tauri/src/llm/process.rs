//! Isolated child-process runner for one-shot CLI calls.
//!
//! Every child runs in its own process group. On timeout the whole group is
//! killed (SIGKILL), and after a normal exit the group is swept too, so a CLI
//! that forks helpers (node workers, MCP servers) never leaves an orphan.
//! Output is capped so a runaway CLI cannot exhaust memory.

use crate::process_group::ProcessTree;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

/// Upper bound kept from each of stdout and stderr (and from any output file
/// a CLI writes instead of stdout).
pub const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
/// How long to keep draining pipes after the group is swept; only a helper
/// that escaped the process group could still hold a pipe open.
const PIPE_DRAIN_GRACE: Duration = Duration::from_millis(500);

type Captured = Arc<Mutex<Vec<u8>>>;
type Reader = (tokio::task::JoinHandle<()>, Captured);

#[derive(Clone, Debug)]
pub struct ProcessSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The complete child environment (the parent's environment is cleared).
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
    pub stdin: Option<String>,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessError {
    Spawn(String),
    Io(String),
    TimedOut,
}

pub async fn run(spec: ProcessSpec) -> Result<ProcessOutput, ProcessError> {
    let mut command = crate::hide_console::tokio_command(&spec.program);
    command
        .args(&spec.args)
        .env_clear()
        .envs(&spec.env)
        .current_dir(&spec.cwd)
        .stdin(if spec.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let (mut child, tree) =
        ProcessTree::spawn(&mut command).map_err(|error| ProcessError::Spawn(error.to_string()))?;

    if let (Some(input), Some(mut stdin)) = (spec.stdin, child.stdin.take()) {
        tokio::spawn(async move {
            // A CLI may exit before reading everything; a broken pipe is not
            // an error of ours.
            let _ = stdin.write_all(input.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let stdout_task = child.stdout.take().map(spawn_reader);
    let stderr_task = child.stderr.take().map(spawn_reader);

    let status = match tokio::time::timeout(spec.timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            tree.kill();
            return Err(ProcessError::Io(error.to_string()));
        }
        Err(_) => {
            tree.kill();
            let _ = child.kill().await;
            let _ = child.wait().await;
            for (task, _) in [stdout_task, stderr_task].into_iter().flatten() {
                task.abort();
            }
            return Err(ProcessError::TimedOut);
        }
    };
    // Sweep helpers the CLI left behind first: a helper that inherited stdout
    // would otherwise keep the pipe open and the answer would never reach EOF.
    tree.kill();
    let stdout = collect(stdout_task).await;
    let stderr = collect(stderr_task).await;
    Ok(ProcessOutput {
        success: status.success(),
        code: status.code(),
        stdout,
        stderr,
    })
}

/// Reads a file a CLI wrote instead of stdout, under the same cap as the
/// pipes. A missing file reads as empty; a larger one is refused rather than
/// cut, since a truncated answer would parse as a different one.
pub fn read_output_file(path: &std::path::Path) -> Result<String, String> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_CAPTURE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err(format!(
            "the answer is too large (over {} MiB)",
            MAX_CAPTURE_BYTES / (1024 * 1024)
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn spawn_reader(pipe: impl AsyncRead + Unpin + Send + 'static) -> Reader {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let task = tokio::spawn(read_capped(pipe, captured.clone()));
    (task, captured)
}

/// Waits briefly for EOF, then returns whatever was read; bytes already
/// captured are never discarded.
async fn collect(reader: Option<Reader>) -> String {
    let Some((mut task, captured)) = reader else {
        return String::new();
    };
    if tokio::time::timeout(PIPE_DRAIN_GRACE, &mut task)
        .await
        .is_err()
    {
        task.abort();
    }
    let bytes = captured
        .lock()
        .map(|bytes| bytes.clone())
        .unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn read_capped(mut pipe: impl AsyncRead + Unpin, captured: Captured) {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                let Ok(mut kept) = captured.lock() else {
                    break;
                };
                let room = MAX_CAPTURE_BYTES.saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..read.min(room)]);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn llm_answer_survives_a_helper_that_keeps_stdout_open() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("helper.pid");
        // The CLI prints its answer, leaves a helper holding stdout, and exits.
        let script = format!(
            "sleep 60 &\necho $! > '{}'\necho 'final answer'\nexit 0",
            pid_file.display()
        );
        let started = Instant::now();
        let output = run(ProcessSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), script],
            env: BTreeMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]),
            cwd: dir.path().to_path_buf(),
            stdin: None,
            timeout: Duration::from_secs(10),
        })
        .await
        .unwrap();
        assert!(output.success);
        assert_eq!(output.stdout.trim(), "final answer");
        assert!(started.elapsed() < Duration::from_secs(5));
        let helper: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let mut alive = true;
        for _ in 0..50 {
            // SAFETY: signal 0 only checks for existence.
            alive = unsafe { libc::kill(helper, 0) } == 0;
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(!alive, "helper {helper} survived the call");
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use crate::llm::cmd_shim_tests::{
        kill_pid, npm_cmd_cli, npm_cmd_cli_with_child, process_alive, read_record, wait_for_pid,
    };

    /// `cmd.exe` → `node` → child: on timeout the whole tree ends, not only
    /// the `cmd.exe` Clovy started.
    #[tokio::test]
    async fn llm_cmd_cli_timeout_ends_the_whole_process_tree() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let program = npm_cmd_cli_with_child(dir.path(), "codex", &pid_file, "started");
        // Generous: a cold `node` on a hosted runner can take seconds.
        let call = tokio::spawn(run(ProcessSpec {
            program,
            args: vec!["exec".to_string()],
            env: std::env::vars().collect(),
            cwd: dir.path().to_path_buf(),
            stdin: None,
            timeout: Duration::from_secs(20),
        }));
        let pid_path = pid_file.clone();
        let child =
            tokio::task::spawn_blocking(move || wait_for_pid(&pid_path, Duration::from_secs(20)))
                .await
                .unwrap()
                .expect("child never started");
        assert_eq!(call.await.unwrap(), Err(ProcessError::TimedOut));
        let alive = process_alive(child);
        kill_pid(child);
        assert!(!alive, "child {child} outlived the timed-out CLI");
    }

    /// One-shot calls give the prompt on stdin; through an npm `.cmd` shim
    /// it reaches the CLI byte for byte, however long and whatever it holds.
    #[tokio::test]
    async fn llm_cmd_cli_gets_the_stdin_prompt_byte_for_byte() {
        let dir = tempfile::tempdir().unwrap();
        let record = dir.path().join("received.json");
        let program = npm_cmd_cli(dir.path(), "codex", &record);
        let prompt = format!(
            "linha 1\n\"aspas\" & 100% %PATH% ^ fim!\r\nlinha 3 > < |\n{}",
            "x".repeat(20_000)
        );
        let output = run(ProcessSpec {
            program,
            args: vec!["exec".to_string(), "-".to_string()],
            env: std::env::vars().collect(),
            cwd: dir.path().to_path_buf(),
            stdin: Some(prompt.clone()),
            timeout: Duration::from_secs(30),
        })
        .await
        .unwrap();
        assert!(output.success, "{output:?}");
        let received = read_record(&record).unwrap();
        assert_eq!(received["stdin"].as_str(), Some(prompt.as_str()));
        assert_eq!(received["args"], serde_json::json!(["exec", "-"]));
    }
}
