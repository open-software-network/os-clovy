//! Windows test support: a fake CLI installed the way npm installs one, a
//! `.cmd` shim in front of a Node script. The script records the argv and
//! stdin it received as JSON (`{"args": [...], "stdin": "..."}`).

use serde_json::Value;
use std::path::{Path, PathBuf};

/// The template npm writes for a package binary (`cmd-shim`), as found in
/// `%APPDATA%\npm\codex.cmd`.
const NPM_SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\fake-cli.js\" %*\r\n";

/// Writes `<dir>/<name>.cmd`; each run overwrites `record`.
pub(crate) fn npm_cmd_cli(dir: &Path, name: &str, record: &Path) -> PathBuf {
    let script = format!(
        "let data = \"\";\nprocess.stdin.setEncoding(\"utf8\");\nprocess.stdin.on(\"data\", (chunk) => (data += chunk));\nprocess.stdin.on(\"end\", () => require(\"fs\").writeFileSync({record}, JSON.stringify({{ args: process.argv.slice(2), stdin: data }})));\n",
        record = Value::String(record.display().to_string())
    );
    std::fs::write(dir.join("fake-cli.js"), script).unwrap();
    let shim = dir.join(format!("{name}.cmd"));
    std::fs::write(&shim, NPM_SHIM).unwrap();
    shim
}

/// Writes `<dir>/<name>.cmd`, a CLI that starts a long-lived Node child (a
/// stand-in for `codex.exe` under npm's `node`), writes the child's pid to
/// `child_pid`, prints one line, and then never exits.
pub(crate) fn npm_cmd_cli_with_child(
    dir: &Path,
    name: &str,
    child_pid: &Path,
    line: &str,
) -> PathBuf {
    let script = format!(
        "const {{ spawn }} = require(\"child_process\");\nconst child = spawn(process.execPath, [\"-e\", \"setInterval(() => {{}}, 1000)\"], {{ stdio: \"ignore\" }});\nrequire(\"fs\").writeFileSync({pid}, String(child.pid));\nprocess.stdout.write({line} + \"\\n\");\nsetInterval(() => {{}}, 1000);\n",
        pid = Value::String(child_pid.display().to_string()),
        line = Value::String(line.to_string()),
    );
    std::fs::write(dir.join("fake-cli.js"), script).unwrap();
    let shim = dir.join(format!("{name}.cmd"));
    std::fs::write(&shim, NPM_SHIM).unwrap();
    shim
}

/// Waits up to `timeout` for the fake CLI to write its child's pid.
pub(crate) fn wait_for_pid(path: &Path, timeout: std::time::Duration) -> Option<u32> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Some(pid) = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| text.trim().parse().ok())
        {
            return Some(pid);
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    None
}

/// Whether a process with this pid is still running, after a short grace
/// for the kill to land.
pub(crate) fn process_alive(pid: u32) -> bool {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_TIMEOUT},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
    };
    // SAFETY: the handle is only waited on and closed.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return false;
        };
        let alive = WaitForSingleObject(handle, 2_000) == WAIT_TIMEOUT;
        let _ = CloseHandle(handle);
        alive
    }
}

/// Ends a leftover test process so a failing assertion does not leak it.
pub(crate) fn kill_pid(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output();
}

/// What the fake CLI recorded, if it ran.
pub(crate) fn read_record(record: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(record).ok()?;
    serde_json::from_str(&text).ok()
}
