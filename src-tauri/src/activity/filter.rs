//! Privacy decisions taken before anything is read or written: ignored apps
//! and domains, private windows, Clovy's own windows, protected (DRM) video,
//! and work hours. Pure functions over descriptors and a supplied clock.

use chrono::{Datelike, Duration, NaiveDateTime, Timelike};

use super::platform::WindowDescriptor;
use super::settings::{parse_clock, ActivitySettings, WorkHours};

/// Bundle id prefixes of browsers. Together with `BROWSER_NAME_WORDS` this is
/// the one browser predicate (`is_browser`) used both for private-window
/// detection (`macos::ax`) and for the unknown-URL rule below, so the two
/// sets can never drift apart.
const BROWSER_BUNDLE_PREFIXES: &[&str] = &[
    "com.google.chrome",
    "com.apple.safari",
    "com.apple.safaritechnologypreview",
    "com.brave.browser",
    "com.microsoft.edgemac",
    "company.thebrowser.browser",
    "com.operasoftware.opera",
    "com.vivaldi.vivaldi",
    "org.chromium.chromium",
    // Firefox, Developer Edition (`org.mozilla.firefoxdeveloperedition`),
    // and Nightly; not Thunderbird.
    "org.mozilla.firefox",
    "org.mozilla.nightly",
    "net.imput.helium",
    "com.kagi.kagimacos",
    "app.zen-browser.zen",
];

// Executable identities are exact, unlike macOS bundle-id families.
pub(crate) const BROWSER_EXECUTABLES: &[&str] = &[
    "msedge.exe",
    "chrome.exe",
    "chromium.exe",
    "firefox.exe",
    "brave.exe",
    "opera.exe",
    "vivaldi.exe",
    "arc.exe",
    "zen.exe",
    "helium.exe",
];

/// Whole words of browser app names, for apps whose bundle id is unknown.
/// Word match, not substring: "Knowledge" must not read as "edge".
const BROWSER_NAME_WORDS: &[&str] = &[
    "chrome", "chromium", "safari", "brave", "edge", "arc", "opera", "vivaldi", "firefox",
    "helium", "orion", "zen",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    ClovyWindow,
    IgnoredApp,
    IgnoredDomain,
    PrivateWindow,
    /// A browser window whose tab URL is not known yet (accessibility still
    /// starting up): with ignored domains configured, it cannot be cleared.
    UnknownBrowserUrl,
}

/// Why `window` must not be captured, or `None` when it may be.
pub fn skip_reason(
    window: &WindowDescriptor,
    settings: &ActivitySettings,
    own_pid: i32,
) -> Option<SkipReason> {
    if is_clovy_window(window, own_pid) {
        return Some(SkipReason::ClovyWindow);
    }
    if app_is_ignored(
        &window.app_name,
        window.bundle_id.as_deref(),
        &settings.ignored_apps,
    ) {
        return Some(SkipReason::IgnoredApp);
    }
    if window.private_window {
        return Some(SkipReason::PrivateWindow);
    }
    if window
        .browser_url
        .as_deref()
        .is_some_and(|url| url_matches_domains(url, &settings.ignored_domains))
    {
        return Some(SkipReason::IgnoredDomain);
    }
    if window.browser_url.is_none()
        && !settings.ignored_domains.is_empty()
        && is_browser(&window.app_name, window.bundle_id.as_deref())
    {
        return Some(SkipReason::UnknownBrowserUrl);
    }
    None
}

/// The canonical browser predicate: a known browser bundle id, or an app name
/// containing a browser's name as a whole word.
pub fn is_browser(app_name: &str, bundle_id: Option<&str>) -> bool {
    let by_bundle = bundle_id.is_some_and(|bundle| {
        let bundle = bundle.to_ascii_lowercase();
        BROWSER_BUNDLE_PREFIXES
            .iter()
            .any(|prefix| bundle.starts_with(prefix))
            || BROWSER_EXECUTABLES.contains(&bundle.as_str())
    });
    by_bundle
        || app_name
            .to_lowercase()
            .split(|ch: char| !ch.is_alphanumeric())
            .any(|word| BROWSER_NAME_WORDS.contains(&word))
}

/// Normalizes browser URL strings from AXURL attributes or address fields.
pub fn normalize_browser_url(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Supported schemes
    let allowed_schemes = [
        "http://",
        "https://",
        "file://",
        "about:",
        "chrome://",
        "chrome-extension://",
        "edge://",
        "brave://",
        "arc://",
        "safari://",
    ];

    for scheme in &allowed_schemes {
        if trimmed.starts_with(scheme) {
            return Some(trimmed.to_string());
        }
    }

    // Domain-like pattern without scheme: e.g. "github.com/repo" or "news.ycombinator.com"
    if !trimmed.contains(' ') && (trimmed.contains('.') || trimmed.starts_with("localhost")) {
        return Some(format!("https://{trimmed}"));
    }

    None
}

/// Checks window title for private/incognito browsing indicators across
/// Chrome-family, Safari, and other browsers in multiple locales (en, pt-BR, es, fr, de).
pub fn has_private_window_marker(title: &str) -> bool {
    let lower = title.to_lowercase();
    let markers = [
        "incognito",
        "inprivate",
        "private browsing",
        "(private)",
        "[private]",
        "navegação privada",
        "navegacao privada",
        // Firefox pt-BR: "… — Navegação privativa do Mozilla Firefox".
        "navegação privativa",
        "navegacao privativa",
        "janela privativa",
        "janela privada",
        "aba privada",
        "guia privada",
        "anônima",
        // Chrome pt-BR incognito window titles end in "(Modo anônimo)"
        // (observed live on Chrome with a pt-BR UI).
        "anônimo",
        "anonimo",
        "anonima",
        "janela anônima",
        "janela anonima",
        "guia anônima",
        "guia anonima",
        "navegação anônima",
        "navegacao anonima",
        "incógnito",
        "navegación privada",
        "navegacion privada",
        "ventana privada",
        "navigation privée",
        "navigation privee",
        "fenêtre privée",
        "fenetre privee",
        "privates surfen",
        "inkognito",
    ];

    markers.iter().any(|&m| lower.contains(m))
}

pub fn is_clovy_window(window: &WindowDescriptor, own_pid: i32) -> bool {
    window.pid == own_pid
        || window.bundle_id.as_deref().is_some_and(|bundle| {
            // Release, dev (`.dev`), proof identities and helpers.
            let bundle = bundle.to_ascii_lowercase();
            let own = crate::app_identity::RELEASE_APP_IDENTIFIER;
            bundle == own || bundle.starts_with(&format!("{own}."))
        })
}

/// Exact, case-insensitive match on the app name (or its bundle id).
pub fn app_is_ignored(app_name: &str, bundle_id: Option<&str>, ignored: &[String]) -> bool {
    let app = app_name.trim().to_lowercase();
    ignored.iter().any(|entry| {
        entry.trim().to_lowercase() == app
            || bundle_id.is_some_and(|bundle| entry.trim().eq_ignore_ascii_case(bundle))
    })
}

/// The URL's host, lowercased, without `www.`.
pub fn url_host(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url.trim()).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    Some(host.trim_start_matches("www.").to_string())
}

/// True when the URL's host is one of `domains` or a subdomain of one.
pub fn url_matches_domains(url: &str, domains: &[String]) -> bool {
    let Some(host) = url_host(url) else {
        return false;
    };
    domains.iter().any(|domain| host_matches(&host, domain))
}

fn host_matches(host: &str, domain: &str) -> bool {
    let domain = domain
        .trim()
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    !domain.is_empty()
        && (host == domain
            || host
                .strip_suffix(domain.as_str())
                .is_some_and(|prefix| prefix.ends_with('.')))
}

/// Streaming apps that render DRM video black under screen capture.
const PROTECTED_VIDEO_APPS: &[&str] = &[
    "netflix",
    "disney+",
    "hulu",
    "prime video",
    "apple tv",
    "tv",
    "peacock",
    "paramount+",
    "hbo max",
    "max",
    "crunchyroll",
    "dazn",
];

const PROTECTED_VIDEO_DOMAINS: &[&str] = &[
    "netflix.com",
    "disneyplus.com",
    "hulu.com",
    "primevideo.com",
    "tv.apple.com",
    "peacocktv.com",
    "paramountplus.com",
    "max.com",
    "hbomax.com",
    "crunchyroll.com",
    "dazn.com",
];

/// (host, path prefix) pairs where only part of a site streams DRM video.
const PROTECTED_VIDEO_PATHS: &[(&str, &str)] = &[("amazon.com", "/gp/video/")];

/// Known DRM streaming app focused, or a browser on a known DRM streaming
/// site. App names match exactly (case-insensitive) to avoid "Maximum".
pub fn is_protected_video(app_name: &str, url: Option<&str>) -> bool {
    let app = app_name.trim().to_lowercase();
    if PROTECTED_VIDEO_APPS.iter().any(|known| app == *known) {
        return true;
    }
    let Some(url) = url else {
        return false;
    };
    let Some(host) = url_host(url) else {
        return false;
    };
    if PROTECTED_VIDEO_DOMAINS
        .iter()
        .any(|domain| host_matches(&host, domain))
    {
        return true;
    }
    let path = url::Url::parse(url.trim())
        .map(|parsed| parsed.path().to_string())
        .unwrap_or_default();
    PROTECTED_VIDEO_PATHS
        .iter()
        .any(|(domain, prefix)| host_matches(&host, domain) && path.starts_with(prefix))
}

/// Whether `local` (the user's wall clock) falls inside the configured work
/// hours. Disabled work hours never pause capture. A window whose end is at
/// or before its start spans midnight and belongs to the day it started.
pub fn within_work_hours(hours: &WorkHours, local: NaiveDateTime) -> bool {
    if !hours.enabled {
        return true;
    }
    let (Some(start), Some(end)) = (parse_clock(&hours.start), parse_clock(&hours.end)) else {
        return true;
    };
    let minute = local.hour() * 60 + local.minute();
    let day_enabled = |at: NaiveDateTime| {
        let weekday = at.weekday().number_from_monday() as u8;
        hours.days.contains(&weekday)
    };
    if start < end {
        return day_enabled(local) && minute >= start && minute < end;
    }
    // Overnight (or 24 h when start == end).
    if minute >= start {
        day_enabled(local)
    } else if minute < end {
        day_enabled(local - Duration::days(1))
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn window(app: &str, url: Option<&str>) -> WindowDescriptor {
        WindowDescriptor {
            pid: 42,
            app_name: app.into(),
            bundle_id: Some("com.google.Chrome".into()),
            window_title: Some("Title".into()),
            browser_url: url.map(str::to_string),
            ..WindowDescriptor::default()
        }
    }

    fn settings() -> ActivitySettings {
        ActivitySettings {
            ignored_apps: vec!["Slack".into()],
            ignored_domains: vec!["youtube.com".into()],
            ..ActivitySettings::default()
        }
    }

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn ignored_domain_matches_subdomains_but_not_lookalikes() {
        let settings = settings();
        let skip = |url| skip_reason(&window("Google Chrome", Some(url)), &settings, 1);
        assert_eq!(
            skip("https://www.youtube.com/watch?v=1"),
            Some(SkipReason::IgnoredDomain)
        );
        assert_eq!(
            skip("https://music.youtube.com/"),
            Some(SkipReason::IgnoredDomain)
        );
        assert_eq!(skip("https://youtube.com"), Some(SkipReason::IgnoredDomain));
        assert_eq!(skip("https://notyoutube.com/"), None);
        assert_eq!(skip("https://youtube.com.evil.example/"), None);
    }

    #[test]
    fn ignored_apps_match_exactly_without_case() {
        let settings = settings();
        assert_eq!(
            skip_reason(&window("slack", None), &settings, 1),
            Some(SkipReason::IgnoredApp)
        );
        assert_eq!(
            skip_reason(
                &window("Slack Helper", Some("https://example.com")),
                &settings,
                1
            ),
            None
        );
    }

    #[test]
    fn browser_predicate_covers_known_bundles_and_whole_name_words() {
        for (name, bundle) in [
            ("Firefox", Some("org.mozilla.firefox")),
            (
                "Firefox Developer Edition",
                Some("org.mozilla.firefoxdeveloperedition"),
            ),
            ("Helium", Some("net.imput.helium")),
            ("Vivaldi", Some("com.vivaldi.Vivaldi")),
            ("Chromium", Some("org.chromium.Chromium")),
            ("Arc", Some("company.thebrowser.Browser")),
            ("Some Browser", Some("com.google.Chrome.canary")),
            ("Microsoft Edge", None),
            ("Brave Browser", None),
        ] {
            assert!(is_browser(name, bundle), "{name} should be a browser");
        }
        for (name, bundle) in [
            ("Knowledge", None),
            ("Search", None),
            ("Thunderbird", Some("org.mozilla.thunderbird")),
            ("Zed", Some("dev.zed.Zed")),
        ] {
            assert!(!is_browser(name, bundle), "{name} is not a browser");
        }
    }

    #[test]
    fn windows_browser_executables_apply_domain_privacy_rules() {
        for executable in [
            "msedge.exe",
            "chrome.exe",
            "firefox.exe",
            "brave.exe",
            "opera.exe",
            "vivaldi.exe",
        ] {
            let browser = WindowDescriptor {
                pid: 42,
                app_name: "Localized browser description".into(),
                bundle_id: Some(executable.into()),
                ..WindowDescriptor::default()
            };
            assert_eq!(
                skip_reason(&browser, &settings(), 1),
                Some(SkipReason::UnknownBrowserUrl),
                "{executable}"
            );
        }
        assert!(!is_browser("Document", Some("msedge.exe.backup")));
    }

    #[test]
    fn browser_without_a_known_url_is_skipped_only_when_domains_are_ignored() {
        let chrome_without_url = window("Google Chrome", None);
        assert_eq!(
            skip_reason(&chrome_without_url, &settings(), 1),
            Some(SkipReason::UnknownBrowserUrl)
        );
        assert_eq!(
            skip_reason(&chrome_without_url, &ActivitySettings::default(), 1),
            None
        );
        let editor = WindowDescriptor {
            bundle_id: Some("dev.zed.Zed".into()),
            ..window("Zed", None)
        };
        assert_eq!(skip_reason(&editor, &settings(), 1), None);
    }

    #[test]
    fn private_windows_are_skipped_whatever_the_url() {
        let mut private = window("Google Chrome", Some("https://example.com"));
        private.private_window = true;
        assert_eq!(
            skip_reason(&private, &settings(), 1),
            Some(SkipReason::PrivateWindow)
        );
    }

    #[test]
    fn clovy_windows_are_skipped_by_pid_or_bundle() {
        let mut own = window("Clovy", None);
        own.pid = 7;
        assert_eq!(
            skip_reason(&own, &settings(), 7),
            Some(SkipReason::ClovyWindow)
        );
        own.pid = 8;
        own.bundle_id = Some("dev.guilhermexp.clovy.dev".into());
        assert_eq!(
            skip_reason(&own, &settings(), 7),
            Some(SkipReason::ClovyWindow)
        );
        own.bundle_id = Some("dev.guilhermexp.clovybug".into());
        assert_eq!(skip_reason(&own, &settings(), 7), None);
    }

    #[test]
    fn protected_video_by_app_or_site() {
        assert!(is_protected_video("Netflix", None));
        assert!(!is_protected_video("Maximum", None));
        assert!(is_protected_video(
            "Safari",
            Some("https://www.netflix.com/watch/1")
        ));
        assert!(is_protected_video(
            "Arc",
            Some("https://www.amazon.com/gp/video/detail/1")
        ));
        assert!(!is_protected_video(
            "Arc",
            Some("https://www.amazon.com/dp/1")
        ));
        assert!(!is_protected_video("Zed", None));
    }

    #[test]
    fn tuesday_evening_is_outside_weekday_office_hours() {
        let hours = WorkHours {
            enabled: true,
            ..WorkHours::default()
        };
        // 2026-09-29 is a Tuesday.
        assert!(!within_work_hours(&hours, local(2026, 9, 29, 20, 0)));
        assert!(within_work_hours(&hours, local(2026, 9, 29, 9, 0)));
        assert!(!within_work_hours(&hours, local(2026, 9, 29, 18, 0)));
        // Saturday.
        assert!(!within_work_hours(&hours, local(2026, 10, 3, 10, 0)));
        // Disabled work hours never pause.
        assert!(within_work_hours(
            &WorkHours::default(),
            local(2026, 10, 3, 23, 0)
        ));
    }

    #[test]
    fn overnight_hours_belong_to_the_start_day() {
        let hours = WorkHours {
            enabled: true,
            days: vec![5], // Friday night shift
            start: "22:00".into(),
            end: "06:00".into(),
        };
        // Friday 2026-10-02 23:00 and Saturday 03:00 are in; Saturday 23:00 is not.
        assert!(within_work_hours(&hours, local(2026, 10, 2, 23, 0)));
        assert!(within_work_hours(&hours, local(2026, 10, 3, 3, 0)));
        assert!(!within_work_hours(&hours, local(2026, 10, 3, 23, 0)));
        assert!(!within_work_hours(&hours, local(2026, 10, 2, 12, 0)));
    }
}
