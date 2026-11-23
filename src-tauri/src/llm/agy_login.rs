//! Native interactive authentication. No task prompt, token payload or consent
//! automation; opening the terminal says nothing about authentication success.
use super::{
    agy::{Execution, HostContext},
    cli::CliKind,
    shell_env::LoginEnv,
    LlmError,
};
use std::{collections::BTreeMap, io, path::PathBuf, process::Command};

#[cfg(unix)]
use std::{path::Path, process::Stdio};

pub struct Login {
    execution: Execution,
    program: PathBuf,
    environment: BTreeMap<String, String>,
}
impl Login {
    pub fn prepare(host: &HostContext, env: &LoginEnv) -> Result<Self, LlmError> {
        let program = env
            .which(CliKind::Agy.id())
            .ok_or(LlmError::CliNotInstalled(CliKind::Agy))?;
        let program = std::fs::canonicalize(program).map_err(failed)?;
        let execution = host.login().map_err(failed)?;
        let environment = execution.environment(&super::shell_env::cli_environment(env.vars()));
        Ok(Self {
            execution,
            program,
            environment,
        })
    }

    /// The platform opens a new owned terminal, never an existing shell or
    /// terminal session chosen by title/PID. OAuth remains native and manual.
    pub fn open(self) -> Result<(), LlmError> {
        self.execution.validate().map_err(failed)?;
        #[cfg(unix)]
        {
            let script = self.write_posix_launcher().map_err(failed)?;
            #[cfg(target_os = "macos")]
            let mut command = {
                let mut command = Command::new("/usr/bin/open");
                command.args(["-n", "-a", "Terminal"]).arg(&script);
                command
            };
            #[cfg(not(target_os = "macos"))]
            let mut command = {
                let mut command = Command::new("x-terminal-emulator");
                command.arg("-e").arg(&script);
                command
            };
            let output = command
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .map_err(failed)?;
            if !output.status.success() {
                return Err(failed(io::Error::other(
                    "The system could not open the Antigravity authentication terminal.",
                )));
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows::Win32::System::Threading::CREATE_NEW_CONSOLE;
            self.windows_command()?
                .creation_flags(CREATE_NEW_CONSOLE.0)
                .spawn()
                .map_err(failed)?;
        }
        Ok(())
    }

    #[cfg(windows)]
    fn windows_command(&self) -> Result<Command, LlmError> {
        let args = [
            "--settings".to_string(),
            serde_json::json!({"useG1Credits":false,"modelProvider":""}).to_string(),
        ];
        super::cli::check_command_line(CliKind::Agy, &self.program, &args, true)?;
        // Rust owns argv quoting, including installed .cmd shims. No existing
        // terminal receives a shell command string.
        let mut command = Command::new(&self.program);
        command
            .args(args)
            .env_clear()
            .envs(&self.environment)
            .current_dir(self.execution.cwd());
        Ok(command)
    }

    #[cfg(unix)]
    fn write_posix_launcher(&self) -> io::Result<PathBuf> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = self.execution.cwd().join("authenticate.command");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)?;
        // This file contains only OS/path metadata, never inherited provider
        // keys. The SDK owns OAuth and its credential persistence.
        super::agy_private::private_executable(&file)?;
        let script = self.posix_script(&path);
        file.write_all(script.as_bytes())?;
        file.sync_all()?;
        Ok(path)
    }

    #[cfg(unix)]
    fn posix_script(&self, path: &Path) -> String {
        let env = self
            .environment
            .iter()
            .map(|(k, v)| posix_quote(&format!("{k}={v}")))
            .collect::<Vec<_>>()
            .join(" ");
        format!("#!/bin/sh\ncd {} || exit 1\n/usr/bin/env -i {} {} --settings {}\nclovy_status=$?\n/bin/rm -f -- {}\nexit \"$clovy_status\"\n",
            posix_quote(&self.execution.cwd().to_string_lossy()),env,posix_quote(&self.program.to_string_lossy()),
            posix_quote(&serde_json::json!({"useG1Credits":false,"modelProvider":""}).to_string()),posix_quote(&path.to_string_lossy()))
    }
}

fn failed(error: io::Error) -> LlmError {
    LlmError::CliFailed {
        cli: CliKind::Agy,
        detail: format!("Could not open Antigravity authentication in Clovy: {error}"),
    }
}
#[cfg(unix)]
fn posix_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn managed_login_launcher_executes_only_native_interactive_cli_in_private_profile() {
        let root = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let bin = root.path().join("bin with 'quotes $dollar `literal`");
        std::fs::create_dir(&bin).unwrap();
        let program = bin.join("agy");
        std::fs::write(
            &program,
            r#"#!/bin/sh
[ "$#" = 2 ] && [ "$1" = --settings ] || exit 31
[ -z "${GEMINI_API_KEY:-}${GOOGLE_API_KEY:-}${ANTHROPIC_API_KEY:-}" ] || exit 32
[ ! -f "$HOME/personal-hook" ] || exit 33
printf '%s' 'synthetic native authentication' > "$HOME/synthetic-auth"
pwd > "$HOME/login-cwd"
exit 0
"#,
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let vars = BTreeMap::from([
            ("PATH".into(), format!("{}:/usr/bin:/bin", bin.display())),
            ("HOME".into(), root.path().display().to_string()),
            (
                "GEMINI_API_KEY".into(),
                "synthetic forbidden billing key".into(),
            ),
        ]);
        std::fs::write(root.path().join("personal-hook"), b"personal").unwrap();
        let host = HostContext::new(root.path().canonicalize().unwrap(), &vars).unwrap();
        let login = Login::prepare(&host, &LoginEnv::from_vars(vars)).unwrap();
        let home = login.execution.home().to_path_buf();
        let cwd = login.execution.cwd().to_path_buf();
        let script = login.write_posix_launcher().unwrap();
        assert!(Command::new(&script)
            .env_clear()
            .status()
            .unwrap()
            .success());
        assert!(!script.exists());
        assert_eq!(
            std::fs::read_to_string(home.join("login-cwd"))
                .unwrap()
                .trim(),
            cwd.to_str().unwrap()
        );
        drop(login);
        let execution = host.one_shot().unwrap();
        assert_eq!(execution.home(), home);
        assert_eq!(
            std::fs::read(home.join("synthetic-auth")).unwrap(),
            b"synthetic native authentication"
        );
        assert!(root.path().join("personal-hook").exists());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    #[test]
    fn managed_login_windows_command_preserves_json_and_private_native_locations() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("bin 'quoted & $literal");
        std::fs::create_dir(&bin).unwrap();
        let record = root.path().join("record.json");
        super::super::cmd_shim_tests::npm_cmd_cli(&bin, "agy", &record);
        let script = format!(
            r#"require('fs').writeFileSync({},JSON.stringify({{args:process.argv.slice(2),cwd:process.cwd(),env:process.env}}));"#,
            serde_json::Value::String(record.display().to_string())
        );
        std::fs::write(bin.join("fake-cli.js"), script).unwrap();
        let path = std::env::var_os("PATH").unwrap();
        let path =
            std::env::join_paths(std::iter::once(bin.clone()).chain(std::env::split_paths(&path)))
                .unwrap();
        let vars = BTreeMap::from([
            ("Path".into(), path.to_string_lossy().into_owned()),
            ("PATHEXT".into(), ".COM;.EXE;.BAT;.CMD".into()),
            ("SystemRoot".into(), std::env::var("SystemRoot").unwrap()),
            ("Home".into(), "C:\\synthetic-personal".into()),
            ("UserProfile".into(), "C:\\synthetic-personal".into()),
            ("AppData".into(), "C:\\synthetic-personal\\roaming".into()),
            (
                "LocalAppData".into(),
                "C:\\synthetic-personal\\local".into(),
            ),
            ("HomeDrive".into(), "Z:".into()),
            ("HomePath".into(), "\\synthetic-personal".into()),
            ("GEMINI_API_KEY".into(), "synthetic forbidden key".into()),
        ]);
        let host = HostContext::new(root.path().canonicalize().unwrap(), &vars).unwrap();
        let login = Login::prepare(&host, &LoginEnv::from_vars(vars)).unwrap();
        let output = login.windows_command().unwrap().output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let record = super::super::cmd_shim_tests::read_record(&record).unwrap();
        let args = record["args"].as_array().unwrap();
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "--settings");
        let settings: serde_json::Value = serde_json::from_str(args[1].as_str().unwrap()).unwrap();
        assert_eq!(settings["useG1Credits"], false);
        assert_eq!(settings["modelProvider"], "");
        let env = record["env"].as_object().unwrap();
        for name in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "HOMEDRIVE",
            "HOMEPATH",
        ] {
            let values: Vec<_> = env
                .iter()
                .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                .collect();
            assert_eq!(values.len(), 1);
            assert!(!values[0].1.as_str().unwrap().contains("synthetic-personal"));
        }
        assert!(!env.contains_key("GEMINI_API_KEY"));
        assert_eq!(
            PathBuf::from(record["cwd"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            login.execution.cwd()
        );
    }
}
