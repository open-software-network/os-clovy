//! Deterministic session categorizer: ten categories scored from evidence
//! (app, window titles and URLs weighted by focus share, captured text, and
//! meeting audio). No model is called. The winner's share of the total score
//! is its confidence; under `CONFIDENCE_FLOOR` the session is idle/personal.

use serde::{Deserialize, Serialize};

use crate::activity::filter::is_browser;

/// Below this share of the evidence, no category is trusted.
pub const CONFIDENCE_FLOOR: f32 = 0.35;
/// Text beyond this many characters adds nothing but cost.
const TEXT_SCAN_LIMIT: usize = 20_000;
/// Weight of the window titles/URLs, split by each window's share of frames.
const WINDOW_WEIGHT: f32 = 35.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Coding,
    CodeReview,
    Meeting,
    Communication,
    Design,
    Documentation,
    Planning,
    DeploymentDevops,
    Research,
    IdlePersonal,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Coding,
        Category::CodeReview,
        Category::Meeting,
        Category::Communication,
        Category::Design,
        Category::Documentation,
        Category::Planning,
        Category::DeploymentDevops,
        Category::Research,
        Category::IdlePersonal,
    ];

    /// The value stored in `timeline_sessions.category`.
    pub fn as_db(self) -> &'static str {
        match self {
            Category::Coding => "coding",
            Category::CodeReview => "code_review",
            Category::Meeting => "meeting",
            Category::Communication => "communication",
            Category::Design => "design",
            Category::Documentation => "documentation",
            Category::Planning => "planning",
            Category::DeploymentDevops => "deployment_devops",
            Category::Research => "research",
            Category::IdlePersonal => "idle_personal",
        }
    }

    pub fn from_db(value: &str) -> Self {
        Category::ALL
            .into_iter()
            .find(|category| category.as_db() == value)
            .unwrap_or(Category::IdlePersonal)
    }

    fn index(self) -> usize {
        Category::ALL
            .iter()
            .position(|category| *category == self)
            .unwrap_or(9)
    }
}

/// One window title/URL pair seen in a session and how many frames showed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowSample {
    pub title: String,
    pub url: String,
    pub frames: u32,
}

pub struct SessionEvidence<'a> {
    pub app_name: &'a str,
    pub bundle_id: Option<&'a str>,
    pub windows: &'a [WindowSample],
    pub text: &'a str,
    /// A Clovy recording (meeting audio) overlapped the session.
    pub meeting_audio: bool,
}

/// `(app name or bundle id fragment, category, weight)`, most specific first.
/// Specific apps weigh more; general ones leave room for content to decide.
const APP_PRIORS: &[(&str, Category, f32)] = &[
    ("zoom", Category::Meeting, 40.0),
    ("webex", Category::Meeting, 40.0),
    ("whereby", Category::Meeting, 40.0),
    ("google meet", Category::Meeting, 40.0),
    ("facetime", Category::Meeting, 30.0),
    ("microsoft teams", Category::Communication, 30.0),
    ("slack", Category::Communication, 30.0),
    ("discord", Category::Communication, 30.0),
    ("telegram", Category::Communication, 30.0),
    ("whatsapp", Category::Communication, 30.0),
    ("signal", Category::Communication, 30.0),
    ("messages", Category::Communication, 30.0),
    ("outlook", Category::Communication, 30.0),
    ("superhuman", Category::Communication, 30.0),
    ("spark", Category::Communication, 30.0),
    ("mail", Category::Communication, 25.0),
    ("figma", Category::Design, 35.0),
    ("sketch", Category::Design, 35.0),
    ("framer", Category::Design, 35.0),
    ("canva", Category::Design, 35.0),
    ("affinity", Category::Design, 35.0),
    ("pixelmator", Category::Design, 35.0),
    ("photoshop", Category::Design, 35.0),
    ("illustrator", Category::Design, 35.0),
    ("notion", Category::Documentation, 30.0),
    ("obsidian", Category::Documentation, 30.0),
    ("confluence", Category::Documentation, 30.0),
    ("bear", Category::Documentation, 30.0),
    ("ulysses", Category::Documentation, 30.0),
    ("pages", Category::Documentation, 30.0),
    ("microsoft word", Category::Documentation, 30.0),
    ("linear", Category::Planning, 30.0),
    ("jira", Category::Planning, 30.0),
    ("asana", Category::Planning, 30.0),
    ("trello", Category::Planning, 30.0),
    ("clickup", Category::Planning, 30.0),
    ("things", Category::Planning, 25.0),
    ("omnifocus", Category::Planning, 25.0),
    ("datadog", Category::DeploymentDevops, 35.0),
    ("grafana", Category::DeploymentDevops, 35.0),
    ("docker", Category::DeploymentDevops, 35.0),
    ("orbstack", Category::DeploymentDevops, 35.0),
    ("lens", Category::DeploymentDevops, 30.0),
    ("cursor", Category::Coding, 25.0),
    ("windsurf", Category::Coding, 25.0),
    ("zed", Category::Coding, 25.0),
    ("xcode", Category::Coding, 25.0),
    ("intellij", Category::Coding, 25.0),
    ("pycharm", Category::Coding, 25.0),
    ("webstorm", Category::Coding, 25.0),
    ("goland", Category::Coding, 25.0),
    ("rustrover", Category::Coding, 25.0),
    ("android studio", Category::Coding, 25.0),
    ("sublime text", Category::Coding, 25.0),
    ("nova", Category::Coding, 25.0),
    ("emacs", Category::Coding, 25.0),
    ("vim", Category::Coding, 25.0),
    ("code", Category::Coding, 20.0),
    ("spotify", Category::IdlePersonal, 30.0),
    ("music", Category::IdlePersonal, 30.0),
    ("tv", Category::IdlePersonal, 30.0),
    ("photos", Category::IdlePersonal, 30.0),
    ("finder", Category::IdlePersonal, 25.0),
    ("system settings", Category::IdlePersonal, 25.0),
    ("app store", Category::IdlePersonal, 25.0),
];

const TERMINALS: &[&str] = &[
    "terminal",
    "iterm",
    "warp",
    "ghostty",
    "alacritty",
    "kitty",
    "wezterm",
    "hyper",
];

// Feed executable identities into the same priors as their macOS/name
// equivalents, so localized FileDescription values do not change categories.
const WINDOWS_APP_NAMES: &[(&str, &str)] = &[
    ("code.exe", "code"),
    ("code-insiders.exe", "code"),
    ("devenv.exe", "code"),
    ("notepad++.exe", "code"),
    ("sublime_text.exe", "sublime text"),
    ("windowsterminal.exe", "terminal"),
    ("cmd.exe", "terminal"),
    ("powershell.exe", "terminal"),
    ("pwsh.exe", "terminal"),
    ("winword.exe", "microsoft word"),
    ("teams.exe", "microsoft teams"),
    ("ms-teams.exe", "microsoft teams"),
    ("explorer.exe", "finder"),
];

const MEETING_AUDIO_APPS: &[&str] = &[
    "zoom",
    "webex",
    "whereby",
    "google meet",
    "microsoft teams",
    "facetime",
    "slack",
];

const PR_TOKENS: &[&str] = &[
    "pull request",
    "/pull/",
    "merge request",
    "/merge_requests/",
    "code review",
];
const DEVOPS_TOKENS: &[&str] = &[
    "console.aws",
    "console.cloud.google",
    "portal.azure",
    "circleci.com",
    "app.datadoghq",
    "vercel.com",
    "netlify.com",
    "dokploy",
    "grafana",
    "dockerfile",
    "docker-compose",
    ".tf",
    ".yaml",
    ".yml",
    "k8s",
    "kubernetes",
    "helm",
    "jenkinsfile",
    "/actions",
    "deploy",
];
const PLANNING_TOKENS: &[&str] = &[
    "linear.app",
    "jira",
    "atlassian.net/browse",
    "asana.com",
    "trello.com",
    "monday.com",
    "clickup.com",
    "/issues",
    "/projects",
    "roadmap",
    "backlog",
];
const DESIGN_TOKENS: &[&str] = &["figma.com", "framer.com", "canva.com", "dribbble.com"];
const DOCS_TOKENS: &[&str] = &[
    "notion.so",
    "docs.google.com",
    "confluence",
    "gitbook",
    "readme.io",
    "readme.md",
    ".md ",
];
const COMMUNICATION_TOKENS: &[&str] = &[
    "mail.google.com",
    "outlook.live",
    "outlook.office",
    "app.slack.com",
    "chat.google.com",
    "web.whatsapp.com",
    "web.telegram.org",
    "discord.com/channels",
];
const MEETING_TOKENS: &[&str] = &[
    "meet.google.com",
    "zoom.us/j",
    "teams.microsoft.com/l/meetup",
    "whereby.com",
];
const RESEARCH_TOKENS: &[&str] = &[
    "stackoverflow.com",
    "docs.rs",
    "developer.mozilla.org",
    "developer.apple.com",
    "pkg.go.dev",
    "npmjs.com",
    "pypi.org",
    "crates.io",
    "github.com",
    "gitlab.com",
    "arxiv.org",
    "news.ycombinator.com",
    "wikipedia.org",
    "medium.com",
    "dev.to",
    "chatgpt.com",
    "claude.ai",
    "perplexity.ai",
    "google.com/search",
];
const IDLE_TOKENS: &[&str] = &[
    "youtube.com",
    "netflix.com",
    "twitch.tv",
    "instagram.com",
    "facebook.com",
    "tiktok.com",
    "x.com",
    "twitter.com",
    "spotify.com",
    "primevideo.com",
    "globo.com",
];
const DEV_SUBREDDITS: &[&str] = &[
    "/r/rust",
    "/r/programming",
    "/r/webdev",
    "/r/devops",
    "/r/python",
    "/r/javascript",
    "/r/typescript",
    "/r/golang",
    "/r/swift",
    "/r/machinelearning",
];

const TEXT_DEVOPS_TOKENS: &[&str] = &[
    "kubectl",
    "docker ",
    "terraform",
    "helm ",
    "ansible",
    "gcloud",
    "aws ",
    "flyctl",
    "vercel ",
];
const TEXT_CODE_TOKENS: &[&str] = &[
    "fn ", "def ", "class ", "import ", "const ", "async ", "#include", "func ", "impl ",
];
const TEXT_MEETING_TOKENS: &[&str] = &[
    "mute",
    "unmute",
    "leave meeting",
    "share screen",
    "silenciar",
    "sair da reuni",
    "compartilhar tela",
];
const TEXT_DIFF_TOKENS: &[&str] = &["+++ ", "@@ ", "files changed", "approve"];

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

struct Scores([f32; 10]);

impl Scores {
    fn add(&mut self, category: Category, weight: f32) {
        self.0[category.index()] += weight;
    }

    fn winner(&self) -> (Category, f32) {
        let total: f32 = self.0.iter().sum();
        if total <= 0.0 {
            return (Category::IdlePersonal, 0.0);
        }
        let (index, best) = self
            .0
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, best)| (index, *best))
            .unwrap_or((9, 0.0));
        (Category::ALL[index], best / total)
    }
}

fn score_app(evidence: &SessionEvidence<'_>, scores: &mut Scores) {
    let name = evidence
        .bundle_id
        .and_then(|id| {
            WINDOWS_APP_NAMES
                .iter()
                .find(|(exe, _)| id.eq_ignore_ascii_case(exe))
        })
        .map_or(evidence.app_name, |(_, name)| *name);
    let app = name.trim().to_lowercase();
    if TERMINALS.iter().any(|name| app.contains(name)) {
        scores.add(Category::Coding, 10.0);
        return;
    }
    if is_browser(evidence.app_name, evidence.bundle_id) {
        return;
    }
    if let Some((_, category, weight)) = APP_PRIORS
        .iter()
        .find(|(pattern, _, _)| app.contains(pattern))
    {
        scores.add(*category, *weight);
    }
}

fn window_category(window: &str) -> Option<(Category, f32)> {
    let category = if contains_any(window, MEETING_TOKENS) {
        Category::Meeting
    } else if contains_any(window, PR_TOKENS) {
        Category::CodeReview
    } else if contains_any(window, DEVOPS_TOKENS) {
        Category::DeploymentDevops
    } else if contains_any(window, PLANNING_TOKENS) {
        Category::Planning
    } else if contains_any(window, DESIGN_TOKENS) {
        Category::Design
    } else if contains_any(window, DOCS_TOKENS) {
        Category::Documentation
    } else if contains_any(window, COMMUNICATION_TOKENS) {
        Category::Communication
    } else if window.contains("reddit.com/r/") {
        if contains_any(window, DEV_SUBREDDITS) {
            Category::Research
        } else {
            Category::IdlePersonal
        }
    } else if window.contains("localhost") || window.contains("127.0.0.1") {
        Category::Coding
    } else if contains_any(window, RESEARCH_TOKENS) {
        Category::Research
    } else if contains_any(window, IDLE_TOKENS) {
        Category::IdlePersonal
    } else {
        // Unknown title: a small research nudge, as benefit of the doubt.
        return Some((Category::Research, 0.4));
    };
    Some((category, 1.0))
}

fn score_windows(windows: &[WindowSample], scores: &mut Scores) {
    let total: u32 = windows.iter().map(|window| window.frames).sum();
    if total == 0 {
        return;
    }
    for window in windows {
        let share = window.frames as f32 / total as f32;
        let haystack = format!("{} {}", window.title, window.url).to_lowercase();
        if haystack.trim().is_empty() {
            continue;
        }
        if let Some((category, factor)) = window_category(&haystack) {
            scores.add(category, share * WINDOW_WEIGHT * factor);
        }
    }
}

fn score_text(text: &str, scores: &mut Scores) {
    let end = text
        .char_indices()
        .nth(TEXT_SCAN_LIMIT)
        .map_or(text.len(), |(index, _)| index);
    let text = text[..end].to_lowercase();
    if text.trim().is_empty() {
        return;
    }
    for (tokens, category, weight) in [
        (TEXT_DEVOPS_TOKENS, Category::DeploymentDevops, 10.0),
        (TEXT_CODE_TOKENS, Category::Coding, 5.0),
        (TEXT_MEETING_TOKENS, Category::Meeting, 10.0),
        (TEXT_DIFF_TOKENS, Category::CodeReview, 7.0),
    ] {
        for token in tokens {
            if text.contains(token) {
                scores.add(category, weight);
            }
        }
    }
}

fn score_audio(evidence: &SessionEvidence<'_>, scores: &mut Scores) {
    if !evidence.meeting_audio {
        return;
    }
    let app = evidence.app_name.to_lowercase();
    let meeting_window = evidence.windows.iter().any(|window| {
        contains_any(
            &format!("{} {}", window.title, window.url).to_lowercase(),
            MEETING_TOKENS,
        )
    });
    if MEETING_AUDIO_APPS.iter().any(|name| app.contains(name)) || meeting_window {
        scores.add(Category::Meeting, 50.0);
    } else {
        scores.add(Category::Meeting, 5.0);
    }
}

/// The session's category and the confidence behind it (0..1).
pub fn categorize(evidence: &SessionEvidence<'_>) -> (Category, f32) {
    let mut scores = Scores([0.0; 10]);
    score_audio(evidence, &mut scores);
    score_app(evidence, &mut scores);
    score_windows(evidence.windows, &mut scores);
    score_text(evidence.text, &mut scores);
    let (category, confidence) = scores.winner();
    if confidence < CONFIDENCE_FLOOR {
        (Category::IdlePersonal, confidence)
    } else {
        (category, confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(title: &str, url: &str, frames: u32) -> WindowSample {
        WindowSample {
            title: title.into(),
            url: url.into(),
            frames,
        }
    }

    fn classify(
        app: &str,
        bundle: Option<&str>,
        windows: &[WindowSample],
        text: &str,
        audio: bool,
    ) -> Category {
        categorize(&SessionEvidence {
            app_name: app,
            bundle_id: bundle,
            windows,
            text,
            meeting_audio: audio,
        })
        .0
    }

    #[test]
    fn zoom_with_meeting_audio_is_a_meeting() {
        let windows = [window("Zoom Meeting", "", 300)];
        assert_eq!(
            classify("zoom.us", Some("us.zoom.xos"), &windows, "", true),
            Category::Meeting
        );
    }

    #[test]
    fn windows_executable_identity_categorizes_localized_apps() {
        for (executable, expected) in [
            ("code.exe", Category::Coding),
            ("devenv.exe", Category::Coding),
            ("windowsterminal.exe", Category::Coding),
            ("winword.exe", Category::Documentation),
            ("ms-teams.exe", Category::Communication),
            ("explorer.exe", Category::IdlePersonal),
        ] {
            assert_eq!(
                classify("Localized application", Some(executable), &[], "", false),
                expected,
                "{executable}"
            );
        }
    }

    #[test]
    fn one_sample_per_category() {
        let samples: Vec<(Category, Category)> = vec![
            (
                classify(
                    "Cursor",
                    None,
                    &[window("store.rs \u{2014} os-clovy", "", 100)],
                    "pub async fn open(path: &Path) impl ActivityStore",
                    false,
                ),
                Category::Coding,
            ),
            (
                classify(
                    "Google Chrome",
                    Some("com.google.Chrome"),
                    &[window(
                        "Add timeline by dev \u{b7} Pull Request #42",
                        "https://github.com/org/repo/pull/42/files",
                        100,
                    )],
                    "12 files changed @@ -1,4 +1,9 @@",
                    false,
                ),
                Category::CodeReview,
            ),
            (
                classify(
                    "Google Chrome",
                    Some("com.google.Chrome"),
                    &[window(
                        "Daily standup",
                        "https://meet.google.com/abc-defg-hij",
                        100,
                    )],
                    "mute share screen",
                    true,
                ),
                Category::Meeting,
            ),
            (
                classify("Slack", Some("com.tinyspeck.slackmacgap"), &[], "", false),
                Category::Communication,
            ),
            (
                classify("Figma", Some("com.figma.Desktop"), &[], "", false),
                Category::Design,
            ),
            (
                classify("Notion", Some("notion.id"), &[], "", false),
                Category::Documentation,
            ),
            (
                classify(
                    "Arc",
                    Some("company.thebrowser.Browser"),
                    &[window(
                        "JUN-12 Timeline",
                        "https://linear.app/acme/issue/JUN-12",
                        50,
                    )],
                    "",
                    false,
                ),
                Category::Planning,
            ),
            (
                classify(
                    "Ghostty",
                    None,
                    &[window("deploy", "", 50)],
                    "kubectl rollout status deploy/api terraform plan",
                    false,
                ),
                Category::DeploymentDevops,
            ),
            (
                classify(
                    "Safari",
                    Some("com.apple.Safari"),
                    &[window(
                        "How to use FTS5",
                        "https://stackoverflow.com/questions/1",
                        80,
                    )],
                    "",
                    false,
                ),
                Category::Research,
            ),
            (
                classify(
                    "Google Chrome",
                    Some("com.google.Chrome"),
                    &[window(
                        "Music video",
                        "https://www.youtube.com/watch?v=1",
                        80,
                    )],
                    "",
                    false,
                ),
                Category::IdlePersonal,
            ),
        ];
        for (index, (got, expected)) in samples.into_iter().enumerate() {
            assert_eq!(got, expected, "sample {index}");
        }
    }

    #[test]
    fn weak_mixed_evidence_falls_below_the_floor() {
        // A browser with three unrelated pages of equal weight: no category
        // holds enough of the evidence.
        let windows = [
            window("Pull request", "https://github.com/o/r/pull/1", 10),
            window("Board", "https://linear.app/acme", 10),
            window("Inbox", "https://mail.google.com/", 10),
        ];
        let (category, confidence) = categorize(&SessionEvidence {
            app_name: "Google Chrome",
            bundle_id: Some("com.google.Chrome"),
            windows: &windows,
            text: "",
            meeting_audio: false,
        });
        assert!(confidence < CONFIDENCE_FLOOR, "confidence {confidence}");
        assert_eq!(category, Category::IdlePersonal);
    }

    #[test]
    fn no_evidence_is_idle_personal() {
        assert_eq!(
            classify("Unknown", None, &[], "", false),
            Category::IdlePersonal
        );
    }
}
