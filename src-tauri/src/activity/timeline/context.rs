//! Session context: the browser's domain or the editor's workspace. A change
//! of context inside the same app starts a new session, so a session reads as
//! "Chrome on github.com" or "Cursor in os-clovy" rather than just the app.

use serde::{Deserialize, Serialize};

use crate::activity::filter::{is_browser, url_host};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContextKind {
    Domain,
    Workspace,
}

impl ContextKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ContextKind::Domain => "domain",
            ContextKind::Workspace => "workspace",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "domain" => Some(ContextKind::Domain),
            "workspace" => Some(ContextKind::Workspace),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionContext {
    pub kind: ContextKind,
    pub value: String,
}

/// Editors whose window title ends with the workspace ("file — project").
const TITLE_LAST_EDITORS: &[&str] = &[
    "code",
    "visual studio code",
    "cursor",
    "windsurf",
    "vscodium",
    "positron",
    "trae",
    "kiro",
    "antigravity",
];
const TITLE_LAST_BUNDLES: &[&str] = &[
    "com.microsoft.vscode",
    "com.todesktop.230313mzl4w4u92",
    "com.exafunction.windsurf",
    "com.vscodium",
    "com.trae.app",
    "dev.kiro.desktop",
    "com.google.antigravity",
];
const TITLE_LAST_EXECUTABLES: &[&str] = &[
    "code.exe",
    "code-insiders.exe",
    "cursor.exe",
    "windsurf.exe",
    "vscodium.exe",
    "positron.exe",
    "trae.exe",
    "kiro.exe",
    "antigravity.exe",
];
/// Editors whose window title starts with the workspace ("project — file").
const TITLE_FIRST_EDITORS: &[&str] = &["zed", "xcode"];
const TITLE_FIRST_BUNDLES: &[&str] = &["dev.zed.zed", "com.apple.dt.xcode", "com.jetbrains."];
const TITLE_FIRST_EXECUTABLES: &[&str] = &[
    "zed.exe",
    "idea64.exe",
    "pycharm64.exe",
    "webstorm64.exe",
    "goland64.exe",
    "rustrover64.exe",
    "rider64.exe",
];
/// Title separators, the editors' own (em dash, en dash) before the hyphen.
const SEPARATORS: &[&str] = &[" \u{2014} ", " \u{2013} ", " - "];

#[derive(Clone, Copy, PartialEq, Eq)]
enum TitleLayout {
    WorkspaceLast,
    WorkspaceFirst,
}

fn editor_layout(app_name: &str, bundle_id: Option<&str>) -> Option<TitleLayout> {
    let app = app_name.trim().to_lowercase();
    let bundle = bundle_id.map(str::to_ascii_lowercase).unwrap_or_default();
    let bundle_matches = |prefixes: &[&str]| {
        !bundle.is_empty() && prefixes.iter().any(|prefix| bundle.starts_with(prefix))
    };
    if bundle_matches(TITLE_LAST_BUNDLES)
        || TITLE_LAST_EXECUTABLES.contains(&bundle.as_str())
        || TITLE_LAST_EDITORS.contains(&app.as_str())
    {
        Some(TitleLayout::WorkspaceLast)
    } else if bundle_matches(TITLE_FIRST_BUNDLES)
        || TITLE_FIRST_EXECUTABLES.contains(&bundle.as_str())
        || TITLE_FIRST_EDITORS.contains(&app.as_str())
    {
        Some(TitleLayout::WorkspaceFirst)
    } else {
        None
    }
}

/// The workspace an editor window title names, if the app is a known editor
/// and the title has one. Decorations ("●" unsaved marker, "(Workspace)",
/// "[SSH: host]") are dropped.
pub fn editor_workspace(
    app_name: &str,
    bundle_id: Option<&str>,
    window_title: &str,
) -> Option<String> {
    let layout = editor_layout(app_name, bundle_id)?;
    let mut title = window_title.trim();
    // Windows editor titles may append the product after the workspace.
    // Leave the existing macOS title interpretation untouched.
    if bundle_id.is_some_and(|id| {
        TITLE_LAST_EXECUTABLES
            .iter()
            .any(|exe| id.eq_ignore_ascii_case(exe))
    }) {
        if let Some((index, separator)) = SEPARATORS
            .iter()
            .filter_map(|separator| title.rfind(separator).map(|index| (index, *separator)))
            .max_by_key(|(index, _)| *index)
        {
            let brand = &title[index + separator.len()..];
            if brand.eq_ignore_ascii_case(app_name.trim())
                || TITLE_LAST_EDITORS
                    .iter()
                    .any(|name| brand.eq_ignore_ascii_case(name))
            {
                title = &title[..index];
            }
        }
    }
    let segment = match layout {
        TitleLayout::WorkspaceLast => {
            let (index, separator) = SEPARATORS
                .iter()
                .filter_map(|separator| title.rfind(separator).map(|index| (index, *separator)))
                .max_by_key(|(index, _)| *index)?;
            &title[index + separator.len()..]
        }
        TitleLayout::WorkspaceFirst => {
            let (index, _) = SEPARATORS
                .iter()
                .filter_map(|separator| title.find(separator).map(|index| (index, *separator)))
                .min_by_key(|(index, _)| *index)?;
            &title[..index]
        }
    };
    let cleaned = segment
        .split(" (")
        .next()
        .unwrap_or(segment)
        .split(" [")
        .next()
        .unwrap_or(segment)
        .trim_start_matches(['\u{25cf}', '\u{2022}', '*'])
        .trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

/// The context of one frame: the domain for a browser with a known URL, the
/// workspace for a known editor, otherwise none.
pub fn frame_context(
    app_name: &str,
    bundle_id: Option<&str>,
    window_title: Option<&str>,
    browser_url: Option<&str>,
) -> Option<SessionContext> {
    if let Some(url) = browser_url {
        if is_browser(app_name, bundle_id) {
            return url_host(url).map(|host| SessionContext {
                kind: ContextKind::Domain,
                value: host,
            });
        }
    }
    let title = window_title?;
    editor_workspace(app_name, bundle_id, title).map(|value| SessionContext {
        kind: ContextKind::Workspace,
        value,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_frames_take_the_domain_without_www() {
        let context = frame_context(
            "Google Chrome",
            Some("com.google.Chrome"),
            Some("Pull request #12"),
            Some("https://www.github.com/org/repo/pull/12"),
        );
        assert_eq!(
            context,
            Some(SessionContext {
                kind: ContextKind::Domain,
                value: "github.com".into()
            })
        );
    }

    #[test]
    fn vscode_family_titles_end_with_the_workspace() {
        for (app, title) in [
            ("Code", "main.rs \u{2014} os-clovy"),
            ("Cursor", "\u{25cf} store.rs - os-clovy"),
            ("Code", "build.rs \u{2014} os-clovy (crates/app)"),
            ("Code", "lib.rs \u{2014} os-clovy [SSH: devbox]"),
        ] {
            assert_eq!(
                editor_workspace(app, None, title).as_deref(),
                Some("os-clovy"),
                "{app}: {title}"
            );
        }
    }

    #[test]
    fn windows_editor_identity_extracts_the_workspace_without_english_app_names() {
        for executable in [
            "code.exe",
            "cursor.exe",
            "windsurf.exe",
            "vscodium.exe",
            "antigravity.exe",
        ] {
            assert_eq!(
                editor_workspace("Localized editor", Some(executable), "main.rs - billing")
                    .as_deref(),
                Some("billing"),
                "{executable}"
            );
        }
        assert_eq!(
            editor_workspace("Localized editor", Some("zed.exe"), "billing - main.rs").as_deref(),
            Some("billing")
        );
        assert_eq!(
            editor_workspace("Document", Some("code.exe.backup"), "main.rs - billing"),
            None
        );
    }

    #[test]
    fn windows_editor_titles_do_not_use_product_branding_as_the_workspace() {
        assert_eq!(
            editor_workspace(
                "Visual Studio Code",
                Some("code.exe"),
                "main.rs - billing - Visual Studio Code",
            )
            .as_deref(),
            Some("billing")
        );
        assert_eq!(
            editor_workspace("Cursor", Some("cursor.exe"), "main.rs - billing - Cursor").as_deref(),
            Some("billing")
        );
    }

    #[test]
    fn zed_xcode_and_jetbrains_titles_start_with_the_workspace() {
        assert_eq!(
            editor_workspace("Zed", Some("dev.zed.Zed"), "os-clovy \u{2014} src/main.rs")
                .as_deref(),
            Some("os-clovy")
        );
        assert_eq!(
            editor_workspace(
                "IntelliJ IDEA",
                Some("com.jetbrains.intellij"),
                "billing \u{2013} Invoice.kt"
            )
            .as_deref(),
            Some("billing")
        );
    }

    #[test]
    fn titles_without_a_separator_and_other_apps_have_no_workspace() {
        assert_eq!(editor_workspace("Code", None, "Welcome"), None);
        assert_eq!(editor_workspace("Slack", None, "general - Acme"), None);
        assert_eq!(
            frame_context("Slack", None, Some("general - Acme"), None),
            None
        );
    }
}
