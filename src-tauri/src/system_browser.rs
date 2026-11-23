//! Opens a URL or a system settings target in the default handler.

use crate::domain::types::AppError;

#[cfg(target_os = "windows")]
pub(crate) fn open_in_browser(url: &str) -> Result<(), AppError> {
    open_with_windows_shell(url, |target| {
        use windows::{
            core::{w, PCWSTR},
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        };

        // `target` remains alive for the duration of this synchronous call.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(target.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        result.0 as isize
    })
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn open_in_browser(url: &str) -> Result<(), AppError> {
    let mut command = browser_open_command(url);
    let mut child = command
        .spawn()
        .map_err(|e| AppError::new("browser_open_failed", e.to_string()))?;
    // Reap the short-lived platform launcher off-thread so it doesn't linger
    // as a zombie until the app exits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

#[cfg(any(target_os = "windows", test))]
fn open_with_windows_shell(
    url: &str,
    execute: impl FnOnce(&[u16]) -> isize,
) -> Result<(), AppError> {
    if url.contains('\0') {
        return Err(AppError::new(
            "browser_open_failed",
            "Browser target contains an embedded NUL",
        ));
    }

    let mut target: Vec<u16> = url.encode_utf16().collect();
    target.push(0);
    let result = execute(&target);
    if result > 32 {
        return Ok(());
    }

    let reason = match result {
        0 => "system resource failure",
        2 => "target path not found",
        3 => "target path not found",
        5 => "access denied",
        8 => "insufficient memory",
        27 => "incomplete association",
        28 => "DDE timeout",
        29 => "DDE transaction failure",
        30 => "DDE busy",
        31 => "no registered association",
        32 => "required DLL unavailable",
        _ => "shell activation failure",
    };
    Err(AppError::new(
        "browser_open_failed",
        format!("Could not open browser: {reason} (ShellExecuteW code {result})"),
    ))
}

#[cfg(target_os = "macos")]
fn browser_open_command(url: &str) -> std::process::Command {
    let mut command = crate::hide_console::command("open");
    command.arg(url);
    command
}

#[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
fn browser_open_command(url: &str) -> std::process::Command {
    let mut command = crate::hide_console::command("xdg-open");
    command.arg(url);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_shell_preserves_complex_browser_target() {
        let url = format!(
            "https://accounts.example.test/login?redirect=https%3A%2F%2Fapp.example.test%2Fcallback%3Fa%3D1%26b%3Dtwo&state={}&label=Zażółć🚀%20gęślą",
            "x".repeat(2048)
        );

        open_with_windows_shell(&url, |target| {
            assert_eq!(target.last(), Some(&0));
            assert!(!target[..target.len() - 1].contains(&0));
            assert_eq!(
                String::from_utf16(&target[..target.len() - 1]).unwrap(),
                url
            );
            33
        })
        .unwrap();
    }

    #[test]
    fn windows_shell_preserves_long_browser_target() {
        let query = "pkce%2Fvalue%26with%3Descapes&part=".repeat(256);
        let url = format!("https://accounts.example.test/authorize?{query}");

        open_with_windows_shell(&url, |target| {
            let decoded = String::from_utf16(&target[..target.len() - 1]).unwrap();
            assert_eq!(decoded.len(), url.len());
            assert_eq!(decoded, url);
            33
        })
        .unwrap();
    }

    #[test]
    fn windows_shell_preserves_custom_scheme_target() {
        let target_url = "ms-settings:privacy-microphone";

        open_with_windows_shell(target_url, |target| {
            assert_eq!(target.last(), Some(&0));
            assert_eq!(
                String::from_utf16(&target[..target.len() - 1]).unwrap(),
                target_url
            );
            33
        })
        .unwrap();
    }

    #[test]
    fn windows_shell_rejects_embedded_nul_without_executing() {
        let error = open_with_windows_shell("https://example.test/\0secret", |_| {
            panic!("executor must not be called")
        })
        .unwrap_err();

        assert_eq!(error.code, "browser_open_failed");
        assert!(error.message.contains("embedded NUL"));
    }

    #[test]
    fn windows_shell_classifies_result_boundaries() {
        assert!(open_with_windows_shell("https://example.test", |_| 33).is_ok());

        for (code, expected_reason) in [
            (32, "required DLL unavailable"),
            (31, "no registered association"),
            (0, "system resource failure"),
            (26, "shell activation failure"),
        ] {
            let error = open_with_windows_shell("https://example.test", |_| code).unwrap_err();
            assert_eq!(error.code, "browser_open_failed");
            assert!(error.message.contains(expected_reason));
            assert!(error.message.contains(&format!("code {code}")));
        }
    }

    #[test]
    fn windows_shell_failure_does_not_expose_browser_target() {
        let secret = "oauth-secret-do-not-log";
        let url = format!("https://example.test/login?token={secret}");
        let error = open_with_windows_shell(&url, |_| 31).unwrap_err();

        assert_eq!(error.code, "browser_open_failed");
        assert!(!error.message.contains(&url));
        assert!(!error.message.contains(secret));
        assert!(error
            .details
            .as_ref()
            .map(|details| !details.to_string().contains(secret))
            .unwrap_or(true));
    }
}
