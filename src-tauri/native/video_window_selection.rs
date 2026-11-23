//! Pure selection policy; enumeration order is front to back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: u32,
    pub app: String,
    pub bundle: String,
    pub title: String,
}

pub fn microphone_owned(window: &Window, microphone_bundles: &[String]) -> bool {
    microphone_bundles
        .iter()
        .any(|bundle| same_family(bundle, &window.bundle))
}

pub fn is_candidate(window: &Window, microphone_bundles: &[String]) -> bool {
    if window.id == 0 {
        return false;
    }
    let title = window.title.trim().to_lowercase();
    let bundle = window.bundle.to_ascii_lowercase();
    if title.is_empty() {
        return false;
    }
    let zoom = bundle == "us.zoom.xos" || bundle.starts_with("us.zoom.xos.");
    let teams = ["com.microsoft.teams", "com.microsoft.teams2"]
        .iter()
        .any(|prefix| {
            bundle == *prefix
                || bundle
                    .strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.starts_with('.'))
        });
    if zoom || teams {
        if is_utility_title(&title) {
            return false;
        }
        let explicit = if zoom {
            [
                "zoom meeting",
                "zoom webinar",
                "reunião do zoom",
                "reunião zoom",
                "webinar do zoom",
            ]
            .iter()
            .any(|token| title.contains(token))
        } else {
            [
                "teams meeting",
                "meeting | microsoft teams",
                "meeting - microsoft teams",
                "reunião do teams",
                "reunião do microsoft teams",
            ]
            .iter()
            .any(|token| title.contains(token))
        };
        return explicit
            || (microphone_owned(window, microphone_bundles)
                && ["meeting", "webinar", "reunião"].contains(&title.as_str()));
    }
    let browser = [
        "company.thebrowser.browser",
        "com.google.chrome",
        "com.apple.safari",
        "org.mozilla.firefox",
        "com.microsoft.edgemac",
        "com.brave.browser",
        "com.operasoftware.opera",
    ]
    .iter()
    .any(|family| {
        bundle == *family
            || bundle
                .strip_prefix(family)
                .is_some_and(|suffix| suffix.starts_with('.'))
    });
    browser && is_browser_meeting_title(&title)
}

fn is_browser_meeting_title(title: &str) -> bool {
    let mut core = title;
    for browser in [
        "google chrome",
        "chrome",
        "arc",
        "safari",
        "firefox",
        "microsoft edge",
        "brave",
        "opera",
    ] {
        for separator in [" - ", " – ", " — "] {
            if let Some(value) = core.strip_suffix(&format!("{separator}{browser}")) {
                core = value.trim();
            }
        }
    }
    if [" - google search", " - bing", " - duckduckgo", " - search"]
        .iter()
        .any(|suffix| core.ends_with(suffix))
    {
        return false;
    }
    if core
        .strip_prefix("meet - ")
        .is_some_and(|subject| !subject.trim().is_empty())
    {
        return true;
    }
    if [
        "zoom meeting",
        "teams meeting",
        "meeting | microsoft teams",
        "reunião do teams",
        "reunião do microsoft teams",
    ]
    .contains(&core)
    {
        return true;
    }
    let url = core.strip_prefix("https://").unwrap_or(core);
    if url.chars().any(char::is_whitespace) {
        return false;
    }
    url.strip_prefix("meet.google.com/").is_some_and(|path| {
        let code = path.split(['?', '#']).next().unwrap_or("");
        code.len() == 12
            && code.bytes().enumerate().all(|(index, byte)| {
                if index == 3 || index == 8 {
                    byte == b'-'
                } else {
                    byte.is_ascii_lowercase()
                }
            })
    })
}

// Match utility window names after removing known application labels. Words in a
// meeting subject are not exclusions: "Calendar planning" can be a real meeting.
fn is_utility_title(title: &str) -> bool {
    let words = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let labels: &[&[&str]] = &[
        &["microsoft", "teams"],
        &["teams", "meeting"],
        &["zoom", "workplace"],
        &["zoom", "meeting"],
        &["teams"],
        &["zoom"],
    ];
    let mut core = words.as_slice();
    loop {
        let previous_len = core.len();
        for label in labels {
            if core.starts_with(label) {
                core = &core[label.len()..];
                break;
            }
            if core.ends_with(label) {
                core = &core[..core.len() - label.len()];
                break;
            }
        }
        if core.len() == previous_len {
            break;
        }
    }
    matches!(
        core,
        ["settings"]
            | ["preferences"]
            | ["chat"]
            | ["chats"]
            | ["team", "chat"]
            | ["meeting", "chat"]
            | ["meeting", "settings"]
            | ["meeting", "preferences"]
            | ["calendar"]
            | ["contacts"]
            | ["sign", "in"]
            | ["login"]
            | ["configurações"]
            | ["preferências"]
            | ["conversas"]
            | ["calendário"]
            | ["contatos"]
    )
}

pub fn choose(windows: Vec<Window>, microphone_bundles: &[String]) -> Option<Window> {
    windows
        .into_iter()
        .enumerate()
        .filter(|(_, window)| is_candidate(window, microphone_bundles))
        .min_by_key(|(index, window)| (!microphone_owned(window, microphone_bundles), *index))
        .map(|(_, window)| window)
}

fn same_family(left: &str, right: &str) -> bool {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    left == right
        || left
            .strip_prefix(&right)
            .is_some_and(|suffix| suffix.starts_with('.'))
        || right
            .strip_prefix(&left)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn window(id: u32, bundle: &str, title: &str) -> Window {
        Window {
            id,
            app: bundle.into(),
            bundle: bundle.into(),
            title: title.into(),
        }
    }
    #[test]
    fn native_settings_chat_and_home_are_not_meetings_even_with_microphone() {
        for (bundle, title) in [
            ("us.zoom.xos", "Settings"),
            ("us.zoom.xos", "Zoom Meeting Settings"),
            ("us.zoom.xos", "Zoom Workplace"),
            ("us.zoom.xos", "Zoom Team Chat"),
            ("com.microsoft.teams2", "Chat | Microsoft Teams"),
            ("com.microsoft.teams2", "Microsoft Teams Settings"),
            ("com.microsoft.teams2", "Calendar"),
        ] {
            assert!(
                choose(vec![window(1, bundle, title)], &[bundle.into()]).is_none(),
                "{bundle}: {title}"
            );
        }
    }
    #[test]
    fn meeting_subjects_can_include_utility_words() {
        for (bundle, title) in [
            (
                "com.microsoft.teams2",
                "Calendar planning meeting | Microsoft Teams",
            ),
            (
                "com.microsoft.teams2",
                "Chat roadmap meeting | Microsoft Teams",
            ),
            ("us.zoom.xos", "Calendar planning - Zoom Meeting"),
            ("us.zoom.xos", "Chat roadmap - Zoom Meeting"),
        ] {
            assert!(is_candidate(&window(1, bundle, title), &[]), "{title}");
        }
    }
    #[test]
    fn actual_utility_titles_with_meeting_app_affixes_are_excluded() {
        for title in [
            "Meeting chat | Microsoft Teams",
            "Settings | Teams meeting",
            "Calendar | Teams meeting",
            "Zoom Meeting Settings",
            "Preferences - Zoom Meeting",
            "Meeting chat - Zoom Meeting",
            "Zoom Team Chat",
        ] {
            for bundle in ["us.zoom.xos", "com.microsoft.teams2"] {
                assert!(
                    !is_candidate(&window(1, bundle, title), &[bundle.into()]),
                    "{title}"
                );
            }
        }
        assert!(!is_candidate(
            &window(1, "com.microsoft.teams2", "Calendar planning meeting"),
            &["com.microsoft.teams2".into()]
        ));
    }
    #[test]
    fn generic_native_meeting_requires_microphone_and_settings_cannot_win_frontmost() {
        assert!(choose(vec![window(1, "us.zoom.xos", "Meeting")], &[]).is_none());
        assert_eq!(
            choose(
                vec![window(1, "us.zoom.xos", "Meeting")],
                &["us.zoom.xos.helper".into()]
            )
            .unwrap()
            .id,
            1
        );
        let windows = vec![
            window(1, "us.zoom.xos", "Settings"),
            window(2, "us.zoom.xos", "Zoom Meeting"),
        ];
        assert_eq!(choose(windows, &["us.zoom.xos".into()]).unwrap().id, 2);
    }
    #[test]
    fn microphone_helpers_match_parent_windows_without_matching_unrelated_prefixes() {
        let windows = vec![
            window(1, "us.zoom.xos", "Zoom Meeting"),
            window(2, "com.google.Chrome", "Meet - abc"),
        ];
        assert_eq!(
            choose(windows.clone(), &["COM.GOOGLE.CHROME.helper".into()])
                .unwrap()
                .id,
            2
        );
        assert_eq!(
            choose(windows, &["com.google.ChromeRemoteDesktop".into()])
                .unwrap()
                .id,
            1
        );
        let windows = vec![
            window(1, "us.zoom.xos", "Zoom Meeting"),
            window(2, "com.microsoft.teams2", "Teams meeting"),
        ];
        assert_eq!(
            choose(windows, &["com.microsoft.teams2.helper".into()])
                .unwrap()
                .id,
            2
        );
    }
    #[test]
    fn meet_in_chrome_and_zoom_are_candidates() {
        assert_eq!(
            choose(
                vec![window(1, "com.google.Chrome", "Meet - abc-defg-hij")],
                &[]
            )
            .unwrap()
            .id,
            1
        );
        assert_eq!(
            choose(vec![window(2, "us.zoom.xos", "Zoom Meeting")], &[])
                .unwrap()
                .id,
            2
        );
    }
    #[test]
    fn unrelated_browser_title_is_not_a_meeting() {
        assert!(choose(vec![window(1, "com.google.Chrome", "Email - Inbox")], &[]).is_none());
    }
    #[test]
    fn microphone_owner_wins_then_front_window() {
        let windows = vec![
            window(1, "us.zoom.xos", "Zoom Meeting"),
            window(2, "com.google.Chrome", "Meet - abc"),
        ];
        assert_eq!(
            choose(windows.clone(), &["com.google.Chrome".into()])
                .unwrap()
                .id,
            2
        );
        assert_eq!(choose(windows, &[]).unwrap().id, 1);
    }
    #[test]
    fn browser_meeting_titles_exclude_searches_guides_and_landing_pages() {
        for title in [
            "Meet - planning",
            "Meet - abc-defg-hij - Google Chrome",
            "https://meet.google.com/abc-defg-hij",
            "Zoom Meeting",
            "Teams meeting",
            "Meeting | Microsoft Teams",
        ] {
            assert!(
                is_candidate(&window(1, "com.google.Chrome", title), &[]),
                "{title}"
            );
        }
        for title in [
            "Google Meet",
            "Google Meet - Google Search",
            "Google Meet API documentation",
            "How to use Google Meet",
            "Zoom meeting guide",
            "Teams meeting help",
            "Meet - abc-defg-hij - Google Search",
            "meet.google.com documentation",
            "https://meet.google.com/landing",
            "https://meet.google.com/?abc-defg-hij",
            "https://meet.google.com/#abc-defg-hij",
            "https://meet.google.com/abc-defg-hij?guide text",
        ] {
            assert!(
                !is_candidate(
                    &window(1, "company.thebrowser.Browser", title),
                    &["company.thebrowser.Browser".into()]
                ),
                "{title}"
            );
        }
    }

    #[test]
    fn every_browser_requires_an_exact_or_dotted_bundle_family() {
        for family in [
            "company.thebrowser.browser",
            "com.google.chrome",
            "com.apple.safari",
            "org.mozilla.firefox",
            "com.microsoft.edgemac",
            "com.brave.browser",
            "com.operasoftware.opera",
        ] {
            for bundle in [family.to_uppercase(), format!("{family}.helper")] {
                assert!(
                    is_candidate(&window(1, &bundle, "Meet - planning"), &[]),
                    "{bundle}"
                );
                assert!(
                    !is_candidate(&window(1, &bundle, "Email - Inbox"), &[]),
                    "{bundle}"
                );
            }
            let unrelated = format!("{family}archive");
            assert!(
                !is_candidate(&window(1, &unrelated, "Google Meet notes"), &[]),
                "{unrelated}"
            );
        }
        for bundle in [
            "com.example.knowledgebase",
            "com.google.chromeremotedesktop",
            "com.example.firefox",
        ] {
            assert!(
                !is_candidate(&window(1, bundle, "Google Meet notes"), &[]),
                "{bundle}"
            );
        }
    }

    #[test]
    fn arc_bundle_and_dot_helpers_are_meeting_browser_candidates() {
        for bundle in [
            "company.thebrowser.Browser",
            "company.thebrowser.Browser.helper",
        ] {
            assert!(
                is_candidate(&window(1, bundle, "Meet - planning"), &[]),
                "{bundle}"
            );
            assert!(
                !is_candidate(&window(1, bundle, "Email - Inbox"), &[]),
                "{bundle}"
            );
        }
    }
    #[test]
    fn arc_substrings_and_similar_bundle_prefixes_are_not_browsers() {
        for bundle in [
            "com.apple.searchapp",
            "com.apple.archiveutility",
            "company.thebrowser.BrowserArchive",
            "com.example.arc",
        ] {
            assert!(
                !is_candidate(&window(1, bundle, "Meet - planning"), &[]),
                "{bundle}"
            );
        }
    }
}
