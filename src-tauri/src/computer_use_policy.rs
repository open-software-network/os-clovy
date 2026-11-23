//! Shared desktop application safety boundary, including pre-launch resolution.
#[cfg(any(target_os = "windows", test))]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
#[derive(Debug)]
pub(crate) struct PrivateBrowserLaunchDenied;

#[cfg(any(target_os = "windows", test))]
impl std::fmt::Display for PrivateBrowserLaunchDenied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Use Browser use or human takeover for the private browser.")
    }
}

#[cfg(any(target_os = "windows", test))]
impl std::error::Error for PrivateBrowserLaunchDenied {}

pub(crate) fn blocked_app(app: &str) -> bool {
    let normalized: String = app
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    [
        "clovy",
        "terminal",
        "windowsterminal",
        "cmdexe",
        "powershell",
        "pwsh",
        "conhost",
        "openconsole",
        "iterm",
        "warp",
        "alacritty",
        "kitty",
        "ghostty",
        "wezterm",
        "hyper",
        "codex",
        "chatgpt",
        "systemsettings",
        "systempreferences",
        "universalaccessauthwarn",
        "securityagent",
        "credentialui",
        "consentexe",
        "logonuiexe",
        "taskmgr",
        "regedit",
        "rundll32",
        "controlexe",
        "mmcexe",
        "keychainaccess",
        "loginwindow",
        "installer",
        "activitymonitor",
        "scripteditor",
        "automator",
        "shortcuts",
        "1password",
        "bitwarden",
        "dashlane",
        "keepass",
        "keeperpassword",
        "nordpass",
        "protonpass",
        "roboform",
        "strongbox",
        "enpass",
    ]
    .iter()
    .any(|blocked| normalized.contains(blocked))
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct WindowsIntegrity {
    pub(crate) rid: u32,
    pub(crate) elevated: bool,
}

/// Token observations belong to the retained process instance, never to a new
/// lookup of its numeric PID. Missing observations confer no target authority.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_target_integrity_allowed(
    caller: Option<WindowsIntegrity>,
    target: Option<WindowsIntegrity>,
) -> bool {
    match (caller, target) {
        (Some(caller), Some(target)) => {
            !caller.elevated
                && !target.elevated
                && caller.rid < 0x3000
                && target.rid < 0x3000
                && target.rid <= caller.rid
        }
        _ => false,
    }
}

/// Clovy does not create or remove compatibility overrides. Only explicitly
/// non-elevating layers are accepted here; Windows still evaluates its full
/// effective policy at the non-elevating CreateProcess boundary.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn windows_compatibility_layers_allowed(value: Option<&str>) -> bool {
    value.map_or(true, |value| {
        value.split_whitespace().all(|token| {
            let token = token.trim_start_matches('~');
            token.is_empty()
                || ["HIGHDPIAWARE", "DXMAXIMIZEDWINDOWEDMODE"]
                    .iter()
                    .any(|allowed| token.eq_ignore_ascii_case(allowed))
        })
    })
}

#[cfg(any(target_os = "windows", test))]
pub(crate) fn private_windows_launch_variable(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    [
        "CLOVY_COMPUTER_USE_",
        "CLOVY_AGENT_DESKTOP_",
        "AGENT_DESKTOP_",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod windows_integrity_tests {
    use super::{
        private_windows_launch_variable, windows_compatibility_layers_allowed,
        windows_target_integrity_allowed, WindowsIntegrity,
    };

    #[test]
    fn unknown_or_elevated_target_never_grants_focus_authority() {
        let medium = WindowsIntegrity {
            rid: 0x2000,
            elevated: false,
        };
        for (caller, target) in [
            (Some(medium), None),
            (None, Some(medium)),
            (
                Some(medium),
                Some(WindowsIntegrity {
                    rid: 0x3000,
                    elevated: true,
                }),
            ),
            (
                Some(medium),
                Some(WindowsIntegrity {
                    rid: 0x2000,
                    elevated: true,
                }),
            ),
            (
                Some(medium),
                Some(WindowsIntegrity {
                    rid: 0x2100,
                    elevated: false,
                }),
            ),
        ] {
            assert!(!windows_target_integrity_allowed(caller, target));
        }
        assert!(windows_target_integrity_allowed(Some(medium), Some(medium)));
        assert!(windows_target_integrity_allowed(
            Some(medium),
            Some(WindowsIntegrity {
                rid: 0x1000,
                elevated: false
            }),
        ));
    }

    #[test]
    fn compatibility_policy_preserves_only_known_non_elevating_layers() {
        for layers in [
            "RUNASADMIN",
            "~ RUNASADMIN",
            "RUNASINVOKER",
            "runashighest",
            "HIGHDPIAWARE ELEVATECREATEPROCESS",
            "unknown-layer",
        ] {
            assert!(!windows_compatibility_layers_allowed(Some(layers)));
        }
        for layers in [None, Some(""), Some("~ HIGHDPIAWARE")] {
            assert!(windows_compatibility_layers_allowed(layers));
        }
        assert!(!private_windows_launch_variable("__COMPAT_LAYER"));
        assert!(!private_windows_launch_variable("PATH"));
        assert!(private_windows_launch_variable(
            "Clovy_Computer_Use_Capability"
        ));
        assert!(private_windows_launch_variable("AGENT_DESKTOP_HOME"));
    }
}
