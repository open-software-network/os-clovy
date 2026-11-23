//! Child processes without a console window on Windows.
//!
//! The release app is a GUI-subsystem program, so every console child it
//! starts (`cmd.exe` for npm `.cmd` shims, `node`, `gh`, the agent CLIs, the
//! sidecar) would open its own console, which the default terminal shows as
//! a terminal window. Ordinary child commands use these wrappers; native
//! supervisors set their own creation flags alongside process lifecycle controls.
//!
//! `CREATE_NO_WINDOW` only affects console programs; a GUI program such as
//! `explorer.exe` still shows its window.

use std::ffi::OsStr;

/// `CREATE_NO_WINDOW`: the console child runs without a console window.
#[cfg(windows)]
pub(crate) const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A process command that can be told not to open a console.
pub trait HideConsole {
    /// Starts the child without a console window on Windows; no effect
    /// elsewhere.
    fn hide_console(&mut self) -> &mut Self;
}

impl HideConsole for std::process::Command {
    fn hide_console(&mut self) -> &mut Self {
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(self, CREATE_NO_WINDOW);
        self
    }
}

impl HideConsole for tokio::process::Command {
    fn hide_console(&mut self) -> &mut Self {
        #[cfg(windows)]
        self.creation_flags(CREATE_NO_WINDOW);
        self
    }
}

/// `std::process::Command::new` without a console window.
pub fn command(program: impl AsRef<OsStr>) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.hide_console();
    command
}

/// `tokio::process::Command::new` without a console window.
pub fn tokio_command(program: impl AsRef<OsStr>) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.hide_console();
    command
}
