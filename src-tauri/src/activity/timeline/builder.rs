//! The session builder: a pure state machine that turns frames (oldest
//! first) into timeline events. One session per app and context (browser
//! domain, editor workspace); more than 5 minutes without a useful frame is a
//! gap that closes the session, so gaps never count as session time.
//!
//! A frame is *useful* when something changed since the previous frame (app,
//! window, URL, or text) or the user gave input around it. Unchanged frames
//! without input are *idle*: they extend nothing, and when they fill most of a
//! gap the gap is idle rather than system sleep (no frames at all).

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use super::context::{frame_context, SessionContext};
use crate::activity::store::{parse_time, PauseReason, StoredFrame};

/// Longer than this without a useful frame is a gap.
pub const GAP_THRESHOLD: Duration = Duration::seconds(300);
/// The time one frame stands for (the 2 s capture tick).
pub const FRAME_SPAN: Duration = Duration::seconds(2);
/// Input stored with a frame happened during the tick before it.
const INPUT_BEFORE: Duration = Duration::seconds(3);
const INPUT_AFTER: Duration = Duration::seconds(1);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AwayPlatform {
    Macos,
    Windows,
}

/// Lock screen and screen saver: never a session, never useful.
const AWAY_APPS: &[(&str, AwayPlatform)] = &[
    ("loginwindow", AwayPlatform::Macos),
    ("screensaverengine", AwayPlatform::Macos),
    ("lockapp", AwayPlatform::Windows),
    ("lockapp.exe", AwayPlatform::Windows),
    ("logonui", AwayPlatform::Windows),
    ("logonui.exe", AwayPlatform::Windows),
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenSession {
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub context: Option<SessionContext>,
    pub started_at: DateTime<Utc>,
    /// End of the last useful frame while open; the final end once closed.
    pub ended_at: DateTime<Utc>,
    pub first_frame_id: i64,
    pub last_frame_id: i64,
    pub frame_count: u32,
    pub idle_frame_count: u32,
}

impl OpenSession {
    pub fn duration(&self) -> Duration {
        (self.ended_at - self.started_at).max(Duration::zero())
    }
}

/// What the builder remembers between frames (and across restarts).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuilderState {
    pub open: Option<OpenSession>,
    pub last_frame: Option<FrameMark>,
    pub last_useful_at: Option<DateTime<Utc>>,
    /// Idle frames since the last useful one.
    pub idle_frames: u32,
    /// After frames went back in time, the newest time seen before that:
    /// crossing it again restarts too, so no gap spans already-built time.
    #[serde(default)]
    pub rewound_from: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameMark {
    pub at: DateTime<Utc>,
    pub app_name: String,
    pub bundle_id: Option<String>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub text_id: Option<i64>,
}

impl FrameMark {
    fn of(frame: &StoredFrame, at: DateTime<Utc>) -> Self {
        Self {
            at,
            app_name: frame.app_name.clone(),
            bundle_id: frame.bundle_id.clone(),
            window_title: frame.window_title.clone(),
            browser_url: frame.browser_url.clone(),
            text_id: frame.text_id,
        }
    }

    fn same_window(&self, frame: &StoredFrame) -> bool {
        self.app_name == frame.app_name
            && self.bundle_id == frame.bundle_id
            && self.window_title == frame.window_title
            && self.browser_url == frame.browser_url
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GapKind {
    Idle,
    Sleep,
    Paused,
}

impl GapKind {
    pub fn as_db(self) -> &'static str {
        match self {
            GapKind::Idle => "idle",
            GapKind::Sleep => "sleep",
            GapKind::Paused => "paused",
        }
    }

    pub fn from_db(value: &str) -> Self {
        match value {
            "idle" => GapKind::Idle,
            "paused" => GapKind::Paused,
            _ => GapKind::Sleep,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GapRecord {
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub kind: GapKind,
    pub pause_reason: Option<PauseReason>,
}

/// One useful frame of the open session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowSeen {
    pub at: DateTime<Utc>,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    /// Present when the text may be new to the session's search document.
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimelineEvent {
    Opened(OpenSession),
    Window(WindowSeen),
    Closed(OpenSession),
    Gap(GapRecord),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PauseSpan {
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub reason: PauseReason,
}

/// Context the store supplies for a batch: input times (sorted) and pauses.
#[derive(Clone, Debug, Default)]
pub struct BatchSignals {
    pub input_times: Vec<DateTime<Utc>>,
    pub pauses: Vec<PauseSpan>,
}

impl BatchSignals {
    fn input_near(&self, at: DateTime<Utc>) -> bool {
        let from = at - INPUT_BEFORE;
        let start = self.input_times.partition_point(|time| *time <= from);
        self.input_times
            .get(start)
            .is_some_and(|time| *time <= at + INPUT_AFTER)
    }
}

fn away_platform(frame: &StoredFrame) -> Option<AwayPlatform> {
    let name = frame.app_name.trim();
    let bundle = frame.bundle_id.as_deref();
    AWAY_APPS.iter().find_map(|(app, platform)| {
        (name.eq_ignore_ascii_case(app) || bundle.is_some_and(|id| id.eq_ignore_ascii_case(app)))
            .then_some(*platform)
    })
}

/// Classifies `[from, to)`: a capture pause covering at least half of it,
/// else idle when idle frames cover at least half, else system sleep.
pub fn classify_gap(
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    idle_frames: u32,
    pauses: &[PauseSpan],
) -> GapRecord {
    let total = (to - from).num_milliseconds().max(1);
    let mut by_reason: Vec<(PauseReason, i64)> = Vec::new();
    for pause in pauses {
        let start = pause.started_at.max(from);
        let end = pause.ended_at.unwrap_or(to).min(to);
        let overlap = (end - start).num_milliseconds();
        if overlap <= 0 {
            continue;
        }
        match by_reason
            .iter_mut()
            .find(|(reason, _)| *reason == pause.reason)
        {
            Some((_, sum)) => *sum += overlap,
            None => by_reason.push((pause.reason, overlap)),
        }
    }
    let paused = by_reason
        .into_iter()
        .max_by_key(|(_, overlap)| *overlap)
        .filter(|(_, overlap)| overlap * 2 >= total);
    let (kind, pause_reason) = match paused {
        Some((reason, _)) => (GapKind::Paused, Some(reason)),
        None if i64::from(idle_frames) * FRAME_SPAN.num_milliseconds() * 2 >= total => {
            (GapKind::Idle, None)
        }
        None => (GapKind::Sleep, None),
    };
    GapRecord {
        started_at: from,
        ended_at: to,
        kind,
        pause_reason,
    }
}

fn close_open(state: &mut BuilderState, out: &mut Vec<TimelineEvent>) {
    if let Some(open) = state.open.take() {
        out.push(TimelineEvent::Closed(open));
    }
}

fn context_splits(current: Option<&SessionContext>, next: Option<&SessionContext>) -> bool {
    matches!((current, next), (Some(current), Some(next)) if current != next)
}

/// Feeds one frame. Frames must come in id order. A frame older than the
/// previous one (clock change, imported data) closes the open session and
/// restarts without inventing a gap; so does the first frame past the time
/// reached before that rewind.
pub fn push_frame(
    state: &mut BuilderState,
    frame: &StoredFrame,
    signals: &BatchSignals,
    out: &mut Vec<TimelineEvent>,
) {
    let Some(at) = parse_time(&frame.captured_at) else {
        return;
    };
    if let Some(last) = state.last_frame.as_ref().filter(|last| at < last.at) {
        let rewound_from = state.rewound_from.map_or(last.at, |mark| mark.max(last.at));
        close_open(state, out);
        *state = BuilderState {
            rewound_from: Some(rewound_from),
            ..BuilderState::default()
        };
    } else if state.rewound_from.is_some_and(|mark| at > mark) {
        close_open(state, out);
        *state = BuilderState::default();
    }

    let previous = state.last_frame.as_ref();
    let window_changed = previous.map_or(true, |last| !last.same_window(frame));
    let text_changed = previous.map_or(true, |last| last.text_id != frame.text_id);
    let system_input_near = frame
        .input_idle_ms
        .is_some_and(|idle| idle <= (FRAME_SPAN + INPUT_AFTER).num_milliseconds() as u64);
    let away = away_platform(frame);
    let useful = away.is_none()
        && (window_changed || text_changed || signals.input_near(at) || system_input_near);
    state.last_frame = Some(FrameMark::of(frame, at));

    if !useful {
        // Windows lock descriptors are absent time, not a visible idle desktop.
        // Keep the existing macOS frame-density behavior.
        if away != Some(AwayPlatform::Windows) {
            state.idle_frames = state.idle_frames.saturating_add(1);
        }
        if let Some(open) = state.open.as_mut() {
            open.frame_count += 1;
            open.idle_frame_count += 1;
            open.last_frame_id = frame.id;
        }
        if state
            .open
            .as_ref()
            .is_some_and(|open| at - open.ended_at > GAP_THRESHOLD)
        {
            close_open(state, out);
        }
        return;
    }

    if let Some(last_useful) = state.last_useful_at {
        let gap_start = last_useful + FRAME_SPAN;
        if at - gap_start > GAP_THRESHOLD {
            close_open(state, out);
            out.push(TimelineEvent::Gap(classify_gap(
                gap_start,
                at,
                state.idle_frames,
                &signals.pauses,
            )));
        }
    }

    let context = frame_context(
        &frame.app_name,
        frame.bundle_id.as_deref(),
        frame.window_title.as_deref(),
        frame.browser_url.as_deref(),
    );
    let continues = state.open.as_ref().is_some_and(|open| {
        open.app_name == frame.app_name
            && open.bundle_id == frame.bundle_id
            && !context_splits(open.context.as_ref(), context.as_ref())
    });
    let new_session = !continues;
    if continues {
        if let Some(open) = state.open.as_mut() {
            open.ended_at = open.ended_at.max(at + FRAME_SPAN);
            open.last_frame_id = frame.id;
            open.frame_count += 1;
            if open.context.is_none() {
                open.context = context;
            }
        }
    } else {
        if let Some(mut open) = state.open.take() {
            // Contiguous switch: the old session lasts until the new frame.
            open.ended_at = open.ended_at.max(at);
            out.push(TimelineEvent::Closed(open));
        }
        let open = OpenSession {
            app_name: frame.app_name.clone(),
            bundle_id: frame.bundle_id.clone(),
            context,
            started_at: at,
            ended_at: at + FRAME_SPAN,
            first_frame_id: frame.id,
            last_frame_id: frame.id,
            frame_count: 1,
            idle_frame_count: 0,
        };
        out.push(TimelineEvent::Opened(open.clone()));
        state.open = Some(open);
    }
    out.push(TimelineEvent::Window(WindowSeen {
        at,
        window_title: frame.window_title.clone(),
        browser_url: frame.browser_url.clone(),
        text: (new_session || window_changed || text_changed)
            .then(|| frame.text.clone())
            .flatten(),
    }));
    state.last_useful_at = Some(at);
    state.idle_frames = 0;
}

/// Closes the open session once `now` is past the gap threshold after its
/// last useful frame (the Mac slept, capture paused, or the user left).
pub fn expire(state: &mut BuilderState, now: DateTime<Utc>, out: &mut Vec<TimelineEvent>) {
    if state
        .open
        .as_ref()
        .is_some_and(|open| now - open.ended_at > GAP_THRESHOLD)
    {
        close_open(state, out);
    }
}

/// The gap still going on at `now`: more than the threshold since the last
/// useful frame, with no session still open within it. Not persisted (the
/// next useful frame writes the real one); reads add it so idle or away time
/// shows while it lasts. `pauses` must cover the gap up to `now`.
pub fn ongoing_gap(
    state: &BuilderState,
    now: DateTime<Utc>,
    pauses: &[PauseSpan],
) -> Option<GapRecord> {
    let from = state.last_useful_at? + FRAME_SPAN;
    let session_live = state
        .open
        .as_ref()
        .is_some_and(|open| now - open.ended_at <= GAP_THRESHOLD);
    (!session_live && now - from > GAP_THRESHOLD)
        .then(|| classify_gap(from, now, state.idle_frames, pauses))
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::activity::store::TextSource;
    use chrono::TimeZone;

    pub fn at(hour: u32, minute: u32, second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, hour, minute, second)
            .unwrap()
    }

    pub fn frame(
        id: i64,
        when: DateTime<Utc>,
        app: &str,
        title: &str,
        url: Option<&str>,
        text_id: i64,
    ) -> StoredFrame {
        StoredFrame {
            id,
            captured_at: crate::activity::store::timestamp(when),
            app_name: app.into(),
            bundle_id: None,
            window_title: Some(title.into()),
            browser_url: url.map(str::to_string),
            text_source: TextSource::Accessibility,
            text_id: Some(text_id),
            text: Some(format!("text {text_id}")),
            input_idle_ms: None,
        }
    }

    /// `count` frames every 2 s from `start`, each with new text (useful).
    pub fn run(
        next_id: &mut i64,
        start: DateTime<Utc>,
        count: i64,
        app: &str,
        title: &str,
        url: Option<&str>,
    ) -> Vec<StoredFrame> {
        (0..count)
            .map(|index| {
                *next_id += 1;
                frame(
                    *next_id,
                    start + Duration::seconds(index * 2),
                    app,
                    title,
                    url,
                    *next_id,
                )
            })
            .collect()
    }

    fn feed(frames: &[StoredFrame], signals: &BatchSignals) -> (BuilderState, Vec<TimelineEvent>) {
        let mut state = BuilderState::default();
        let mut out = Vec::new();
        for frame in frames {
            push_frame(&mut state, frame, signals, &mut out);
        }
        (state, out)
    }

    fn closed(events: &[TimelineEvent]) -> Vec<&OpenSession> {
        events
            .iter()
            .filter_map(|event| match event {
                TimelineEvent::Closed(session) => Some(session),
                _ => None,
            })
            .collect()
    }

    fn gaps(events: &[TimelineEvent]) -> Vec<&GapRecord> {
        events
            .iter()
            .filter_map(|event| match event {
                TimelineEvent::Gap(gap) => Some(gap),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn switching_app_closes_the_editor_and_keeps_the_browser_active() {
        let mut id = 0;
        let mut frames = run(
            &mut id,
            at(9, 0, 0),
            300,
            "Code",
            "main.rs \u{2014} os-clovy",
            None,
        );
        frames.extend(run(
            &mut id,
            at(9, 10, 0),
            150,
            "Google Chrome",
            "Docs",
            Some("https://docs.rs/sqlx"),
        ));
        let (state, events) = feed(&frames, &BatchSignals::default());

        let closed = closed(&events);
        assert_eq!(closed.len(), 1);
        assert_eq!(closed[0].app_name, "Code");
        assert_eq!(closed[0].duration(), Duration::minutes(10));
        assert_eq!(
            closed[0]
                .context
                .as_ref()
                .map(|context| context.value.as_str()),
            Some("os-clovy")
        );
        let open = state.open.expect("browser session stays active");
        assert_eq!(open.app_name, "Google Chrome");
        assert_eq!(
            open.context.as_ref().map(|context| context.value.as_str()),
            Some("docs.rs")
        );
        assert_eq!(open.duration(), Duration::minutes(5));
        assert!(gaps(&events).is_empty());
    }

    #[test]
    fn a_new_domain_in_the_same_browser_starts_a_new_session() {
        let mut id = 0;
        let mut frames = run(
            &mut id,
            at(9, 0, 0),
            10,
            "Safari",
            "A",
            Some("https://github.com/a"),
        );
        frames.extend(run(
            &mut id,
            at(9, 0, 20),
            10,
            "Safari",
            "B",
            Some("https://github.com/b"),
        ));
        frames.extend(run(
            &mut id,
            at(9, 0, 40),
            10,
            "Safari",
            "C",
            Some("https://linear.app/x"),
        ));
        let (state, events) = feed(&frames, &BatchSignals::default());
        let closed = closed(&events);
        assert_eq!(closed.len(), 1, "same domain continues; new domain splits");
        assert_eq!(closed[0].frame_count, 20);
        assert_eq!(
            state.open.unwrap().context.unwrap().value,
            "linear.app".to_string()
        );
    }

    #[test]
    fn mac_sleep_becomes_a_sleep_gap_outside_every_session() {
        let mut id = 0;
        let mut frames = run(&mut id, at(11, 0, 0), 1800, "Code", "a \u{2014} p", None);
        frames.extend(run(
            &mut id,
            at(13, 10, 0),
            30,
            "Code",
            "a \u{2014} p",
            None,
        ));
        let (_, events) = feed(&frames, &BatchSignals::default());
        let gaps = gaps(&events);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Sleep);
        assert_eq!(gaps[0].started_at, at(12, 0, 0));
        assert_eq!(gaps[0].ended_at, at(13, 10, 0));
        let closed = closed(&events);
        assert_eq!(closed[0].ended_at, at(12, 0, 0));
        assert_eq!(closed[0].duration(), Duration::hours(1));
    }

    #[test]
    fn unchanged_frames_without_input_make_an_idle_gap() {
        let mut id = 0;
        let mut frames = run(&mut id, at(10, 0, 0), 30, "Preview", "doc.pdf", None);
        // 10 minutes of the same window and text, no input.
        let last_text = frames.last().unwrap().text_id.unwrap();
        for index in 1..=300 {
            id += 1;
            frames.push(frame(
                id,
                at(10, 1, 0) + Duration::seconds(index * 2),
                "Preview",
                "doc.pdf",
                None,
                last_text,
            ));
        }
        frames.extend(run(&mut id, at(10, 12, 0), 5, "Preview", "doc.pdf", None));
        let (_, events) = feed(&frames, &BatchSignals::default());
        let gaps = gaps(&events);
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].kind, GapKind::Idle);
        assert_eq!(gaps[0].started_at, at(10, 1, 0));
    }

    #[test]
    fn input_keeps_unchanged_frames_useful() {
        let mut id = 0;
        let mut frames = run(&mut id, at(10, 0, 0), 1, "Terminal", "zsh", None);
        let text = frames[0].text_id.unwrap();
        let mut inputs = Vec::new();
        for index in 1..=300 {
            id += 1;
            let when = at(10, 0, 0) + Duration::seconds(index * 2);
            frames.push(frame(id, when, "Terminal", "zsh", None, text));
            inputs.push(when - Duration::seconds(1));
        }
        let signals = BatchSignals {
            input_times: inputs,
            pauses: Vec::new(),
        };
        let (state, events) = feed(&frames, &signals);
        assert!(gaps(&events).is_empty());
        assert_eq!(state.open.unwrap().duration(), Duration::seconds(602));
    }

    #[test]
    fn windows_lock_and_logon_frames_never_become_work_even_with_input() {
        for app in ["LockApp", "lockapp.exe", "LogonUI", "logonui.exe"] {
            let frames = [
                frame(1, at(10, 0, 0), app, "Locked", None, 1),
                frame(2, at(10, 0, 2), app, "Sign in", None, 2),
            ];
            let signals = BatchSignals {
                input_times: vec![at(10, 0, 0), at(10, 0, 2)],
                pauses: Vec::new(),
            };
            let (state, events) = feed(&frames, &signals);
            assert!(state.open.is_none(), "{app} must not open a work session");
            assert!(state.last_useful_at.is_none(), "{app} is never useful");
            assert!(closed(&events).is_empty(), "{app} must not record work");
        }
    }

    #[test]
    fn windows_locked_executable_is_away_even_with_a_localized_description() {
        let mut locked = frame(
            1,
            at(10, 0, 0),
            "Tela de bloqueio do Windows",
            "Entrar",
            None,
            1,
        );
        locked.bundle_id = Some("lockapp.exe".into());
        locked.input_idle_ms = Some(0);
        let signals = BatchSignals {
            input_times: vec![at(10, 0, 0)],
            pauses: Vec::new(),
        };
        let (state, events) = feed(&[locked], &signals);
        assert!(state.open.is_none());
        assert!(state.last_useful_at.is_none());
        assert!(closed(&events).is_empty());
    }

    #[test]
    fn windows_lock_samples_count_as_away_without_changing_macos_gap_semantics() {
        for (app, expected) in [
            ("LockApp.exe", GapKind::Sleep),
            ("LogonUI.exe", GapKind::Sleep),
            ("loginwindow", GapKind::Idle),
            ("ScreenSaverEngine", GapKind::Idle),
        ] {
            let mut frames = vec![frame(1, at(10, 0, 0), "Code", "main.rs", None, 1)];
            frames.extend((0..600).map(|index| StoredFrame {
                input_idle_ms: Some(0),
                ..frame(
                    index + 2,
                    at(10, 0, 2) + Duration::seconds(index * 2),
                    app,
                    "Locked",
                    None,
                    2,
                )
            }));
            let now = at(10, 20, 2);
            let (state, _) = feed(&frames, &BatchSignals::default());
            let ongoing = ongoing_gap(&state, now, &[]).expect("locked interval is a gap");
            assert_eq!(ongoing.kind, expected, "ongoing {app}");
            assert_eq!(ongoing.ended_at - ongoing.started_at, Duration::minutes(20));

            frames.push(frame(602, now, "Code", "main.rs", None, 3));
            let (_, events) = feed(&frames, &BatchSignals::default());
            let finished = gaps(&events);
            assert_eq!(finished.len(), 1, "{app}");
            assert_eq!(finished[0].kind, expected, "finished {app}");
            assert_eq!(
                finished[0].ended_at - finished[0].started_at,
                Duration::minutes(20)
            );
        }
    }

    #[test]
    fn scrolling_keeps_ten_minutes_of_unchanged_frames_focused() {
        let frames: Vec<_> = (0..=300)
            .map(|index| StoredFrame {
                input_idle_ms: Some(1000),
                ..frame(
                    index + 1,
                    at(10, 0, 0) + Duration::seconds(index * 2),
                    "Preview",
                    "doc.pdf",
                    None,
                    1,
                )
            })
            .collect();
        let (state, events) = feed(&frames, &BatchSignals::default());
        assert!(gaps(&events).is_empty());
        let session = state.open.expect("scrolling keeps the session open");
        assert_eq!(session.duration(), Duration::seconds(602));
        assert_eq!(session.idle_frame_count, 0);
    }

    #[test]
    fn growing_idle_and_legacy_frames_still_make_an_idle_gap() {
        for legacy in [false, true] {
            let mut frames: Vec<_> = (0..=180)
                .map(|index| StoredFrame {
                    input_idle_ms: (!legacy).then_some(4000 + index as u64 * 2000),
                    ..frame(
                        index + 1,
                        at(10, 0, 0) + Duration::seconds(index * 2),
                        "Preview",
                        "doc.pdf",
                        None,
                        1,
                    )
                })
                .collect();
            frames.push(frame(182, at(10, 6, 2), "Zed", "main.rs", None, 2));
            let (_, events) = feed(&frames, &BatchSignals::default());
            let gaps = gaps(&events);
            assert_eq!(gaps.len(), 1, "legacy={legacy}");
            assert_eq!(gaps[0].kind, GapKind::Idle);
            assert_eq!(gaps[0].started_at, at(10, 0, 2));
            assert_eq!(gaps[0].ended_at, at(10, 6, 2));
        }
    }

    #[test]
    fn system_idle_boundary_is_inclusive_at_three_seconds() {
        for (idle, expected_seconds) in [(Some(3000), 4), (Some(3001), 2), (None, 2)] {
            let frames = [
                frame(1, at(10, 0, 0), "Preview", "doc.pdf", None, 1),
                StoredFrame {
                    input_idle_ms: idle,
                    ..frame(2, at(10, 0, 2), "Preview", "doc.pdf", None, 1)
                },
            ];
            let (state, _) = feed(&frames, &BatchSignals::default());
            assert_eq!(
                state.open.unwrap().duration(),
                Duration::seconds(expected_seconds),
                "input_idle_ms={idle:?}"
            );
        }
    }

    #[test]
    fn lock_screen_input_never_starts_a_work_session() {
        for app in ["loginwindow", "ScreenSaverEngine"] {
            let frames = [StoredFrame {
                input_idle_ms: Some(0),
                ..frame(1, at(10, 0, 0), app, "Unlock", None, 1)
            }];
            let signals = BatchSignals {
                input_times: vec![at(10, 0, 0)],
                pauses: Vec::new(),
            };
            let (state, events) = feed(&frames, &signals);
            assert!(state.open.is_none(), "{app}");
            assert!(state.last_useful_at.is_none(), "{app}");
            assert!(events.is_empty(), "{app}");
        }
    }

    #[test]
    fn a_manual_pause_classifies_the_gap_with_its_reason() {
        let pauses = [PauseSpan {
            started_at: at(14, 0, 30),
            ended_at: Some(at(14, 20, 0)),
            reason: PauseReason::Manual,
        }];
        let gap = classify_gap(at(14, 0, 2), at(14, 20, 2), 0, &pauses);
        assert_eq!(gap.kind, GapKind::Paused);
        assert_eq!(gap.pause_reason, Some(PauseReason::Manual));

        let low_disk = [PauseSpan {
            started_at: at(14, 0, 30),
            ended_at: None,
            reason: PauseReason::LowDisk,
        }];
        assert_eq!(
            classify_gap(at(14, 0, 2), at(14, 20, 2), 0, &low_disk).pause_reason,
            Some(PauseReason::LowDisk)
        );
        // A short pause inside a long sleep does not make it a pause.
        let short = [PauseSpan {
            started_at: at(14, 0, 30),
            ended_at: Some(at(14, 2, 0)),
            reason: PauseReason::WorkHours,
        }];
        assert_eq!(
            classify_gap(at(14, 0, 2), at(14, 20, 2), 0, &short).kind,
            GapKind::Sleep
        );
    }

    #[test]
    fn four_minutes_without_frames_is_not_a_gap() {
        let mut id = 0;
        let mut frames = run(&mut id, at(9, 0, 0), 10, "Code", "a \u{2014} p", None);
        frames.extend(run(&mut id, at(9, 4, 30), 10, "Code", "a \u{2014} p", None));
        let (state, events) = feed(&frames, &BatchSignals::default());
        assert!(gaps(&events).is_empty());
        assert!(closed(&events).is_empty());
        assert_eq!(state.open.unwrap().frame_count, 20);
    }

    #[test]
    fn expire_closes_a_session_left_open_past_the_threshold() {
        let mut id = 0;
        let frames = run(&mut id, at(9, 0, 0), 10, "Code", "a \u{2014} p", None);
        let (mut state, _) = feed(&frames, &BatchSignals::default());
        let mut out = Vec::new();
        expire(&mut state, at(9, 4, 0), &mut out);
        assert!(out.is_empty(), "still within the threshold");
        expire(&mut state, at(9, 6, 0), &mut out);
        assert!(
            matches!(out.as_slice(), [TimelineEvent::Closed(session)] if session.ended_at == at(9, 0, 20))
        );
        assert!(state.open.is_none());
    }

    #[test]
    fn frames_going_back_in_time_restart_without_gaps_over_built_time() {
        let mut id = 0;
        let mut frames = run(&mut id, at(10, 0, 0), 5, "Code", "a \u{2014} p", None);
        // Imported (or clock-shifted) older frames, then live frames again.
        frames.extend(run(&mut id, at(8, 0, 0), 5, "Slack", "general", None));
        frames.extend(run(&mut id, at(8, 30, 0), 5, "Slack", "general", None));
        frames.extend(run(&mut id, at(10, 20, 0), 5, "Code", "a \u{2014} p", None));
        let (state, events) = feed(&frames, &BatchSignals::default());
        let gaps = gaps(&events);
        assert_eq!(gaps.len(), 1, "only the gap inside the older block");
        assert_eq!(gaps[0].started_at, at(8, 0, 10));
        assert_eq!(gaps[0].ended_at, at(8, 30, 0));
        let apps: Vec<&str> = closed(&events)
            .iter()
            .map(|session| session.app_name.as_str())
            .collect();
        assert_eq!(apps, vec!["Code", "Slack", "Slack"]);
        assert_eq!(state.open.unwrap().started_at, at(10, 20, 0));
    }
}
