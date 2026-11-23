//! Local distillation of one hour of captured text before any model call:
//! segment run-on lines, drop interface chrome and noise (junk and prose
//! gates), cut lines repeated across sessions (document frequency), dedupe
//! lexically and then semantically with on-device embeddings, pick a diverse
//! set of 3 to 14 lines per session (facility location), and rescue lines
//! that carry entities (ticket keys, pull requests, file paths) the picks
//! missed. No network: the embedder runs locally, and without it the vector
//! stages degrade to lexical dedup and longest-first picks.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use serde::Serialize;

use super::embedder::Embedder;

/// Cosine above which two lines are near-duplicates (BGE small).
pub const SEM_DEDUP_THRESHOLD: f32 = 0.86;
/// A line seen in at least this share of the hour's sessions (and at least
/// three) is boilerplate, unless it names an entity.
const DF_FRACTION: f64 = 0.25;
/// Lines kept per session by the diversity pick.
const FLOOR: usize = 3;
const CEIL: usize = 14;
/// Entity lines rescued per session.
const ENTITY_RESCUE_CAP: usize = 4;
/// Sessions shorter than this inside the hour are alt-tab flicker.
pub const MIN_SESSION_SECONDS: i64 = 15;
/// Characters kept per rendered line.
const LINE_CHARS: usize = 220;
/// Lines longer than this are split on separators before the gates.
const SEGMENT_TRIGGER: usize = 140;
const EMBED_TIMEOUT: Duration = Duration::from_secs(120);

/// Ticket keys (`KAN-123`), pull requests (`PR #44`, `#443`), and file paths
/// with a known extension.
pub static ENTITY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b[A-Z][A-Z0-9]{1,9}-\d+\b|(?i:PR\s*#?|#)\d{2,6}\b|\b\w[\w./\\-]{3,}\.(?i:rs|py|ts|tsx|js|jsx|md|json|toml|sh|sql|ya?ml|go|rb|java|kt|swift|c|h|cpp|css|html|vue|svelte)\b",
    )
    .expect("ENTITY_RE")
});
/// Commit hashes: hex runs that mix digits and letters.
static HASH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[0-9a-f]{7,40}\b").expect("HASH_RE"));
const JUNK_SUBSTRINGS: &[&str] = &[
    "ctrl+o to expand",
    "esc to interrupt",
    "shift+tab to cycle",
    "tokens · ",
    "Type to search",
];
static SPINNER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[⠂⠄⠆⠇⠋⠙⠸⠴⠦⠧✶✳⏵]").expect("SPINNER_RE"));
static NONWORD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}]+").expect("NONWORD_RE"));
static WORD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{L}[\p{L}']+").expect("WORD_RE"));
static CODEISH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[/\\.]|::|->|\b(?:def|fn|let|const|import|cargo|git|npm|pnpm|python|SELECT|FROM|WHERE)\b",
    )
    .expect("CODEISH_RE")
});
static EXT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\.(?:py|md|sh|json|toml|rs|tsx?|jsx?|lock|txt|ya?ml|cfg|ini|sql|db|env|rb|go|c|h)\b",
    )
    .expect("EXT_RE")
});
static SEGMENT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[•·▸►▶‣⁃◦●○➤➔→⟶»«›‹❯❮|©®✓✦✱✶✳※❘┃│]+|\s{2,}").expect("SEGMENT_RE")
});
/// English and Portuguese function words: prose has them, chrome does not.
static STOP_WORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "the a an and or but to of in on for with at by from is are was were be been this that \
     these those it its as into out up down over we i you he she they have has had do does \
     did not no so if then than when while there here can will would should could about \
     which who what your my our their let me now \
     o os as um uma uns umas e ou mas de do da dos das em no na nos nas para por com que se \
     não nao é foi ser está estão era como mais muito este esta isso isto ele ela eles elas \
     eu você voce nós seu sua ao aos à às pelo pela também já quando onde sem sobre entre"
        .split_whitespace()
        .collect()
});

/// One session's captured text inside the hour.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionText {
    pub session_id: i64,
    pub app: String,
    pub window: String,
    /// Local "HH:MM" of the session's first moment in the hour.
    pub time: String,
    /// Session time inside the hour.
    pub seconds: i64,
    /// Window segments in order: local "HH:MM" and the distinct lines seen.
    pub documents: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DistillStats {
    pub sessions: usize,
    pub raw_chars: usize,
    pub out_chars: usize,
    pub after_gates: usize,
    pub after_df: usize,
    pub after_lexical: usize,
    pub after_semantic: usize,
    pub selected: usize,
    pub entity_rescued: usize,
    /// False when the embedder was unavailable (lexical stages only).
    pub semantic: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Distilled {
    pub body: String,
    pub stats: DistillStats,
}

#[derive(Clone, Debug, PartialEq)]
struct Span {
    session: i64,
    app: String,
    window: String,
    time: String,
    line: String,
}

pub fn norm(line: &str) -> String {
    NONWORD_RE
        .replace_all(&line.to_lowercase(), " ")
        .trim()
        .to_string()
}

fn lexical_key(line: &str) -> String {
    norm(line).chars().take(80).collect()
}

fn segment(line: &str) -> Vec<String> {
    if line.chars().count() <= SEGMENT_TRIGGER {
        return vec![line.to_string()];
    }
    SEGMENT_RE
        .split(line)
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn letters(text: &str) -> usize {
    text.chars().filter(|c| c.is_alphabetic()).count()
}

/// Interface chrome, spinners, and lines without enough words.
fn is_junk(line: &str) -> bool {
    let chars = line.chars().count();
    if chars < 18 || JUNK_SUBSTRINGS.iter().any(|junk| line.contains(junk)) {
        return true;
    }
    if SPINNER_RE.is_match(line) {
        return true;
    }
    let letters = letters(line);
    letters < 10 || (letters as f64) / (chars as f64) < 0.45
}

/// Lists of names or labels without function words are chrome; code and
/// commands pass.
fn fails_prose_gate(line: &str) -> bool {
    let words: Vec<String> = WORD_RE
        .find_iter(line)
        .map(|word| word.as_str().to_lowercase())
        .collect();
    if words.is_empty() {
        return true;
    }
    let stops = words
        .iter()
        .filter(|word| STOP_WORDS.contains(word.as_str()))
        .count();
    if EXT_RE.find_iter(line).count() >= 3 && stops < 2 {
        return true;
    }
    if CODEISH_RE.is_match(line) {
        return false;
    }
    stops < 2 || (stops as f64) / (words.len() as f64) < 0.08
}

fn is_hash(candidate: &str) -> bool {
    candidate.bytes().any(|byte| byte.is_ascii_digit())
        && candidate.bytes().any(|byte| byte.is_ascii_alphabetic())
}

fn entities(line: &str) -> Vec<String> {
    ENTITY_RE
        .find_iter(line)
        .chain(
            HASH_RE
                .find_iter(line)
                .filter(|found| is_hash(found.as_str())),
        )
        .map(|found| found.as_str().to_lowercase())
        .collect()
}

fn has_entity(line: &str) -> bool {
    !entities(line).is_empty()
}

/// A kept line with its embedding (when the embedder ran).
type Candidate = (Span, Option<Vec<f32>>);

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Drops lines repeated across many sessions (navigation, menus, footers);
/// entity lines always stay.
fn df_cut(spans: Vec<Span>, sessions: usize) -> Vec<Span> {
    let mut seen_in: HashMap<String, HashSet<i64>> = HashMap::new();
    for span in &spans {
        seen_in
            .entry(norm(&span.line))
            .or_default()
            .insert(span.session);
    }
    let cut = 3.max((DF_FRACTION * sessions as f64) as usize);
    spans
        .into_iter()
        .filter(|span| {
            seen_in.get(&norm(&span.line)).map_or(0, HashSet::len) < cut || has_entity(&span.line)
        })
        .collect()
}

/// First occurrence (in time order) of each normalized 80-character prefix.
fn lexical_dedup(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_by(|a, b| (a.time.as_str(), a.session).cmp(&(b.time.as_str(), b.session)));
    let mut seen = HashSet::new();
    spans
        .into_iter()
        .filter(|span| seen.insert(lexical_key(&span.line)))
        .collect()
}

/// Keeps a line unless an earlier kept line is a near-duplicate; across
/// sessions, an entity line is kept anyway (the same key in two contexts).
fn semantic_keep(spans: &[Span], vectors: &[Vec<f32>]) -> Vec<bool> {
    let mut keep = vec![true; spans.len()];
    if vectors.len() != spans.len() {
        return keep;
    }
    let mut kept: Vec<usize> = Vec::new();
    for index in 0..spans.len() {
        let best = kept
            .iter()
            .map(|&other| (dot(&vectors[index], &vectors[other]), other))
            .max_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((similarity, other)) = best {
            if similarity > SEM_DEDUP_THRESHOLD
                && (spans[other].session == spans[index].session || !has_entity(&spans[index].line))
            {
                keep[index] = false;
                continue;
            }
        }
        kept.push(index);
    }
    keep
}

/// Facility-location pick of `k` lines: the longest first, then repeatedly
/// the line farthest from everything picked; without vectors, longest first.
fn diverse_pick(items: Vec<Candidate>, k: usize) -> (Vec<Span>, Vec<Span>) {
    if items.len() <= k {
        return (
            items.into_iter().map(|(span, _)| span).collect(),
            Vec::new(),
        );
    }
    let length = |index: usize| items[index].0.line.chars().count();
    let seed = (0..items.len())
        .max_by_key(|&index| length(index))
        .unwrap_or(0);
    let mut picked = vec![seed];
    if items.iter().all(|(_, vector)| vector.is_some()) {
        while picked.len() < k {
            let next = (0..items.len())
                .filter(|index| !picked.contains(index))
                .map(|index| {
                    let vector = items[index].1.as_deref().unwrap_or_default();
                    let nearest = picked
                        .iter()
                        .map(|&other| dot(vector, items[other].1.as_deref().unwrap_or_default()))
                        .fold(f32::MIN, f32::max);
                    (index, 1.0 - nearest)
                })
                .max_by(|a, b| a.1.total_cmp(&b.1));
            match next {
                Some((index, _)) => picked.push(index),
                None => break,
            }
        }
    } else {
        let mut order: Vec<usize> = (0..items.len()).filter(|&index| index != seed).collect();
        order.sort_by_key(|&index| std::cmp::Reverse(length(index)));
        picked.extend(order.into_iter().take(k - 1));
    }
    let picked: HashSet<usize> = picked.into_iter().collect();
    let mut keep = Vec::new();
    let mut drop = Vec::new();
    for (index, (span, _)) in items.into_iter().enumerate() {
        if picked.contains(&index) {
            keep.push(span);
        } else {
            drop.push(span);
        }
    }
    (keep, drop)
}

/// Adds dropped lines whose entities the selection does not mention yet.
fn entity_rescue(mut selected: Vec<Span>, dropped: &[(i64, Vec<Span>)]) -> (Vec<Span>, usize) {
    let mut covered: HashMap<i64, String> = HashMap::new();
    for span in &selected {
        let text = covered.entry(span.session).or_default();
        text.push(' ');
        text.push_str(&span.line.to_lowercase());
    }
    let mut rescued = Vec::new();
    for (session, candidates) in dropped {
        let mut text = covered.remove(session).unwrap_or_default();
        let mut added = 0;
        for span in candidates {
            if added == ENTITY_RESCUE_CAP {
                break;
            }
            if entities(&span.line)
                .iter()
                .any(|entity| !text.contains(entity.as_str()))
            {
                text.push(' ');
                text.push_str(&span.line.to_lowercase());
                rescued.push(span.clone());
                added += 1;
            }
        }
    }
    let count = rescued.len();
    selected.extend(rescued);
    (selected, count)
}

/// Lines grouped into threads of consecutive app and window.
fn render(mut selected: Vec<Span>, header: &str) -> String {
    selected.sort_by(|a, b| (a.time.as_str(), a.session).cmp(&(b.time.as_str(), b.session)));
    let mut out = String::from(header);
    let mut current: Option<(String, String)> = None;
    for span in selected {
        let thread = (span.app.clone(), span.window.clone());
        if current.as_ref() != Some(&thread) {
            let window = if span.window.is_empty() {
                "-"
            } else {
                &span.window
            };
            out.push_str(&format!("\n\n[{} · {} · {}]", span.time, span.app, window));
            current = Some(thread);
        }
        let line: String = span.line.chars().take(LINE_CHARS).collect();
        out.push_str("\n  ");
        out.push_str(&line);
    }
    out
}

/// Distills the hour's sessions into the excerpt the hour report reads.
/// `header` opens the excerpt (hour, sessions, measured minutes).
pub async fn distill(sessions: &[SessionText], header: &str, embedder: &dyn Embedder) -> Distilled {
    let sessions: Vec<&SessionText> = sessions
        .iter()
        .filter(|session| session.seconds >= MIN_SESSION_SECONDS)
        .collect();
    let mut stats = DistillStats {
        sessions: sessions.len(),
        ..DistillStats::default()
    };
    if sessions.is_empty() {
        return Distilled {
            body: String::new(),
            stats,
        };
    }

    let mut spans = Vec::new();
    for session in &sessions {
        for (time, body) in &session.documents {
            stats.raw_chars += body.chars().count();
            for raw in body.lines() {
                for line in segment(raw.trim()) {
                    // Ticket keys, pull requests, paths, and hashes survive
                    // however short or terse the line is.
                    if !has_entity(&line) && (is_junk(&line) || fails_prose_gate(&line)) {
                        continue;
                    }
                    spans.push(Span {
                        session: session.session_id,
                        app: session.app.clone(),
                        window: session.window.clone(),
                        time: time.clone(),
                        line,
                    });
                }
            }
        }
    }
    stats.after_gates = spans.len();
    let spans = df_cut(spans, sessions.len());
    stats.after_df = spans.len();
    let spans = lexical_dedup(spans);
    stats.after_lexical = spans.len();

    let vectors = if spans.is_empty() {
        None
    } else {
        let texts = spans.iter().map(|span| span.line.clone()).collect();
        tokio::time::timeout(EMBED_TIMEOUT, embedder.embed(texts))
            .await
            .ok()
            .flatten()
            .filter(|vectors| vectors.len() == spans.len())
    };
    stats.semantic = vectors.is_some();
    let vectors = vectors.unwrap_or_default();
    let keep = semantic_keep(&spans, &vectors);
    let mut by_session: HashMap<i64, Vec<Candidate>> = HashMap::new();
    for (index, span) in spans.into_iter().enumerate() {
        if keep[index] {
            stats.after_semantic += 1;
            let vector = vectors.get(index).cloned();
            by_session
                .entry(span.session)
                .or_default()
                .push((span, vector));
        }
    }

    let mut selected = Vec::new();
    let mut dropped = Vec::new();
    for session in &sessions {
        let items = by_session.remove(&session.session_id).unwrap_or_default();
        if items.is_empty() {
            selected.push(Span {
                session: session.session_id,
                app: session.app.clone(),
                window: session.window.clone(),
                time: session.time.clone(),
                line: "(no readable text)".into(),
            });
            continue;
        }
        let k = FLOOR.max(CEIL.min(items.len()));
        let (keep, drop) = diverse_pick(items, k);
        selected.extend(keep);
        dropped.push((session.session_id, drop));
    }
    let (selected, rescued) = entity_rescue(selected, &dropped);
    stats.entity_rescued = rescued;
    stats.selected = selected.len();
    let body = render(selected, header);
    stats.out_chars = body.chars().count();
    Distilled { body, stats }
}

#[cfg(test)]
mod tests {
    use super::super::embedder::tests::FixedEmbedder;
    use super::super::embedder::NoEmbedder;
    use super::*;

    fn session(id: i64, app: &str, lines: &[&str]) -> SessionText {
        SessionText {
            session_id: id,
            app: app.into(),
            window: format!("{app} window"),
            time: "14:0".to_string() + &id.to_string(),
            seconds: 300,
            documents: vec![("14:0".to_string() + &id.to_string(), lines.join("\n"))],
        }
    }

    const CHROME: &str = "Pull requests Issues Marketplace Explore Codespaces Sign out";
    /// Reads like prose, so only the cross-session frequency cut removes it.
    const BANNER: &str = "You are signed in as gui and you have no new notifications";

    #[tokio::test]
    async fn interface_chrome_goes_and_entities_stay() {
        let sessions = vec![
            session(
                1,
                "Arc",
                &[
                    CHROME,
                    BANNER,
                    "Home",
                    "Fixed the login redirect for KAN-123 in src/auth/login.rs today",
                ],
            ),
            session(
                2,
                "Arc",
                &[
                    CHROME,
                    BANNER,
                    "Settings",
                    "We reviewed the pagination change with the team and agreed on it",
                ],
            ),
            session(
                3,
                "Arc",
                &[
                    CHROME,
                    BANNER,
                    "Search",
                    "The release notes for this week are ready to be published",
                ],
            ),
            session(
                4,
                "Arc",
                &[
                    CHROME,
                    BANNER,
                    "Reading the incident report about the queue that was stuck overnight",
                ],
            ),
        ];
        let out = distill(&sessions, "=== HOUR 14:00 ===", &NoEmbedder).await;
        assert!(!out.body.contains("Marketplace"), "{}", out.body);
        assert!(!out.body.contains("no new notifications"), "{}", out.body);
        assert!(!out.body.contains("\n  Home"), "{}", out.body);
        assert!(out.body.contains("KAN-123"));
        assert!(out.body.contains("src/auth/login.rs"));
        assert!(out.body.starts_with("=== HOUR 14:00 ==="));
        assert!(!out.stats.semantic);
    }

    #[tokio::test]
    async fn near_duplicates_within_a_session_collapse_with_embeddings() {
        let sessions = vec![session(
            1,
            "Zed",
            &[
                "We are refactoring the session distiller in the Rust backend",
                "We were refactoring the session distiller in the Rust backend now",
                "Cooked pasta and watched a film with the family in the evening",
            ],
        )];
        let embedder = FixedEmbedder(vec![vec![1.0, 0.0], vec![0.99, 0.141], vec![0.0, 1.0]]);
        let out = distill(&sessions, "h", &embedder).await;
        assert!(out.stats.semantic);
        assert_eq!(out.stats.after_lexical, 3);
        assert_eq!(out.stats.after_semantic, 2);
        assert!(!out.body.contains("backend now"));
        assert!(out.body.contains("Cooked pasta"));
    }

    #[tokio::test]
    async fn entity_lines_dropped_by_the_pick_are_rescued() {
        let mut lines: Vec<String> = (0..20)
            .map(|i| {
                format!(
                    "This is a long descriptive sentence number {i} about the ongoing work here"
                )
            })
            .collect();
        lines.push("See also the fix in PR #4431 for that".into());
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let out = distill(&[session(1, "Arc", &refs)], "h", &NoEmbedder).await;
        assert_eq!(out.stats.entity_rescued, 1);
        assert!(out.body.contains("PR #4431"));
    }

    #[tokio::test]
    async fn short_sessions_and_empty_hours_produce_nothing() {
        let mut flicker = session(
            1,
            "Finder",
            &["A sentence that is long enough to pass the gates for sure"],
        );
        flicker.seconds = 5;
        let out = distill(&[flicker], "h", &NoEmbedder).await;
        assert!(out.body.is_empty());
        assert_eq!(out.stats.sessions, 0);
    }

    #[tokio::test]
    async fn terse_entity_lines_pass_the_gates_and_chrome_without_entities_does_not() {
        let sessions = vec![session(
            1,
            "Zed",
            &[
                "KAN-123 login regression",
                "src/auth.rs",
                "a1b2c3d",
                "Home Settings Help",
                "We traced the failure to the session cookie being dropped on redirect",
            ],
        )];
        let out = distill(&sessions, "h", &NoEmbedder).await;
        assert!(
            out.body.contains("KAN-123 login regression"),
            "{}",
            out.body
        );
        assert!(out.body.contains("src/auth.rs"), "{}", out.body);
        assert!(out.body.contains("a1b2c3d"), "{}", out.body);
        assert!(out.body.contains("session cookie"), "{}", out.body);
        assert!(!out.body.contains("Home Settings Help"), "{}", out.body);
    }

    #[test]
    fn portuguese_prose_passes_the_gate() {
        assert!(!fails_prose_gate(
            "Revisamos a mudança de paginação com o time e aprovamos a versão"
        ));
        assert!(fails_prose_gate(
            "Início Configurações Ajuda Perfil Notificações Sair"
        ));
    }
}
