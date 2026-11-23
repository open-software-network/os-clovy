//! Input events without content. Raw clicks and keys from the platform are
//! reduced to "kind, time, app, count" per capture tick; any typed characters
//! a platform might report are dropped here, before the store sees them.
//! App switches and window focus changes are derived from the focused window.

use chrono::{DateTime, Utc};

use super::platform::{RawInputEvent, WindowDescriptor};
use super::redact::redact_clipboard;
use super::store::{InputEventKind, NewInputEvent};

/// Coalesces one tick of raw input into at most one row per kind, attributed
/// to `app` (the focused app when the tick drained the buffer).
pub fn normalize_input(raw: &[RawInputEvent], app: Option<&str>) -> Vec<NewInputEvent> {
    let mut rows: Vec<NewInputEvent> = Vec::new();
    for event in raw {
        let kind = match event {
            RawInputEvent::Click { .. } => InputEventKind::Click,
            // `characters` is deliberately never read.
            RawInputEvent::Key { .. } => InputEventKind::Key,
        };
        match rows.iter_mut().find(|row| row.kind == kind) {
            Some(row) => {
                row.count = row.count.saturating_add(1);
                row.occurred_at = row.occurred_at.min(event.at());
            }
            None => rows.push(NewInputEvent {
                occurred_at: event.at(),
                kind,
                app_name: app.map(str::to_string),
                count: 1,
                clipboard_text: None,
            }),
        }
    }
    rows
}

pub fn clipboard_event(text: &str, app: Option<&str>, at: DateTime<Utc>) -> Option<NewInputEvent> {
    let redacted = redact_clipboard(text);
    (!redacted.trim().is_empty()).then(|| NewInputEvent {
        occurred_at: at,
        kind: InputEventKind::Clipboard,
        app_name: app.map(str::to_string),
        count: 1,
        clipboard_text: Some(redacted),
    })
}

/// Turns successive focused windows into app-switch and window-focus events.
#[derive(Default)]
pub struct FocusTracker {
    last: Option<(i32, String, Option<String>)>,
}

impl FocusTracker {
    pub fn observe(
        &mut self,
        window: &WindowDescriptor,
        at: DateTime<Utc>,
    ) -> Option<NewInputEvent> {
        let current = (
            window.pid,
            window.app_name.clone(),
            window.window_title.clone(),
        );
        let kind = match &self.last {
            None => InputEventKind::AppSwitch,
            Some((pid, _, _)) if *pid != current.0 => InputEventKind::AppSwitch,
            Some((_, _, title)) if *title != current.2 => InputEventKind::WindowFocus,
            Some(_) => return None,
        };
        self.last = Some(current);
        Some(NewInputEvent {
            occurred_at: at,
            kind,
            app_name: Some(window.app_name.clone()),
            count: 1,
            clipboard_text: None,
        })
    }

    /// Forget the last window (after a pause or an excluded window), so the
    /// next captured window is recorded as an app switch.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 1, 10, 0, second).unwrap()
    }

    #[test]
    fn typed_characters_never_survive_normalization() {
        let raw = vec![
            RawInputEvent::Key {
                at: at(1),
                characters: Some("hunter2".into()),
            },
            RawInputEvent::Key {
                at: at(0),
                characters: Some("p".into()),
            },
            RawInputEvent::Click { at: at(1) },
        ];
        let rows = normalize_input(&raw, Some("1Password"));
        assert_eq!(rows.len(), 2);
        let keys = rows
            .iter()
            .find(|row| row.kind == InputEventKind::Key)
            .unwrap();
        assert_eq!(keys.count, 2);
        assert_eq!(keys.occurred_at, at(0));
        assert_eq!(keys.app_name.as_deref(), Some("1Password"));
        assert_eq!(keys.clipboard_text, None);
        let debug = format!("{rows:?}");
        assert!(!debug.contains("hunter2") && !debug.contains("\"p\""));
    }

    #[test]
    fn clipboard_tokens_are_redacted_before_storage() {
        let event = clipboard_event(
            "my key sk-proj-AbCdEf0123456789ghIJklMNopQR",
            Some("Zed"),
            at(0),
        )
        .expect("event");
        let text = event.clipboard_text.unwrap();
        assert!(!text.contains("sk-proj-AbCdEf0123456789ghIJklMNopQR"));
        assert!(text.starts_with("my key "));
    }

    #[test]
    fn focus_changes_become_switch_and_focus_events() {
        let mut tracker = FocusTracker::default();
        let zed = WindowDescriptor {
            pid: 1,
            app_name: "Zed".into(),
            window_title: Some("a.rs".into()),
            ..WindowDescriptor::default()
        };
        assert_eq!(
            tracker.observe(&zed, at(0)).unwrap().kind,
            InputEventKind::AppSwitch
        );
        assert!(tracker.observe(&zed, at(2)).is_none());
        let other_file = WindowDescriptor {
            window_title: Some("b.rs".into()),
            ..zed.clone()
        };
        assert_eq!(
            tracker.observe(&other_file, at(4)).unwrap().kind,
            InputEventKind::WindowFocus
        );
        let chrome = WindowDescriptor {
            pid: 2,
            app_name: "Google Chrome".into(),
            ..WindowDescriptor::default()
        };
        assert_eq!(
            tracker.observe(&chrome, at(6)).unwrap().kind,
            InputEventKind::AppSwitch
        );
    }
}
