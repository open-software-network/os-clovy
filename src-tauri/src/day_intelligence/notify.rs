//! Activity notifications: "day summary ready" and failures (the summary
//! could not be generated, capture stopped). Delivery honors the
//! notifications switch and quiet hours: a notice raised in quiet hours waits
//! for their end. Pending notices live in memory (the app keeps running in
//! the menu bar); one per kind and day. Clicking opens the "Today" view at
//! that day's summary; "Snooze 1h" posts it again an hour later.

use chrono::{DateTime, Duration, Utc};

use super::schedule::{quiet_until, LocalZone};
use crate::activity::settings::ActivityNotificationSettings;
use crate::interface_locale::UiLocale;

pub const SNOOZE: Duration = Duration::hours(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    SummaryReady,
    SummaryFailed,
    CaptureFailed,
}

impl NoticeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NoticeKind::SummaryReady => "summaryReady",
            NoticeKind::SummaryFailed => "summaryFailed",
            NoticeKind::CaptureFailed => "captureFailed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "summaryReady" => NoticeKind::SummaryReady,
            "summaryFailed" => NoticeKind::SummaryFailed,
            "captureFailed" => NoticeKind::CaptureFailed,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub kind: NoticeKind,
    /// Local day the notice opens ("YYYY-MM-DD").
    pub day: String,
    /// Headline or error text for the body.
    pub detail: Option<String>,
    pub deliver_at: DateTime<Utc>,
}

/// When a notice raised at `now` may show: now, the end of the current
/// quiet hours, or never (notifications off).
pub fn delivery_time(
    now: DateTime<Utc>,
    zone: &dyn LocalZone,
    settings: &ActivityNotificationSettings,
) -> Option<DateTime<Utc>> {
    if !settings.enabled {
        return None;
    }
    match quiet_until(zone.local(now), &settings.quiet_hours) {
        Some(end) => zone.instant(end).or(Some(now)),
        None => Some(now),
    }
}

#[derive(Debug, Default)]
pub struct NoticeQueue {
    pending: Vec<Notice>,
}

impl NoticeQueue {
    /// Queues a notice (replacing a pending one of the same kind and day).
    /// Dropped when notifications are off.
    pub fn raise(
        &mut self,
        kind: NoticeKind,
        day: String,
        detail: Option<String>,
        now: DateTime<Utc>,
        zone: &dyn LocalZone,
        settings: &ActivityNotificationSettings,
    ) {
        let Some(deliver_at) = delivery_time(now, zone, settings) else {
            return;
        };
        self.pending
            .retain(|notice| !(notice.kind == kind && notice.day == day));
        self.pending.push(Notice {
            kind,
            day,
            detail,
            deliver_at,
        });
    }

    /// The same notice again in an hour.
    pub fn snooze(
        &mut self,
        kind: NoticeKind,
        day: String,
        detail: Option<String>,
        now: DateTime<Utc>,
    ) {
        self.pending
            .retain(|notice| !(notice.kind == kind && notice.day == day));
        self.pending.push(Notice {
            kind,
            day,
            detail,
            deliver_at: now + SNOOZE,
        });
    }

    /// Notices to show now. With notifications turned off since they were
    /// raised, they are dropped; inside quiet hours they wait for the end.
    pub fn take_due(
        &mut self,
        now: DateTime<Utc>,
        zone: &dyn LocalZone,
        settings: &ActivityNotificationSettings,
    ) -> Vec<Notice> {
        if !settings.enabled {
            self.pending.clear();
            return Vec::new();
        }
        if self.pending.iter().all(|notice| notice.deliver_at > now) {
            return Vec::new();
        }
        if let Some(end) = quiet_until(zone.local(now), &settings.quiet_hours) {
            let end = zone.instant(end).unwrap_or(now);
            for notice in &mut self.pending {
                notice.deliver_at = notice.deliver_at.max(end);
            }
            return Vec::new();
        }
        let (due, waiting) = std::mem::take(&mut self.pending)
            .into_iter()
            .partition(|notice| notice.deliver_at <= now);
        self.pending = waiting;
        due
    }

    pub fn next_delivery(&self) -> Option<DateTime<Utc>> {
        self.pending.iter().map(|notice| notice.deliver_at).min()
    }
}

/// Title, body, and the snooze button label in the interface language.
pub fn copy(notice: &Notice, locale: UiLocale) -> (String, String, String) {
    let pt = locale == UiLocale::PtBr;
    let detail = notice
        .detail
        .clone()
        .filter(|detail| !detail.trim().is_empty());
    let (title, fallback) = match (notice.kind, pt) {
        (NoticeKind::SummaryReady, false) => (
            "Your day summary is ready",
            "Open Today to review your day.",
        ),
        (NoticeKind::SummaryReady, true) => (
            "O resumo do seu dia está pronto",
            "Abra Hoje para revisar o seu dia.",
        ),
        (NoticeKind::SummaryFailed, false) => (
            "The day summary could not be generated",
            "Open Today to try again.",
        ),
        (NoticeKind::SummaryFailed, true) => (
            "Não foi possível gerar o resumo do dia",
            "Abra Hoje para tentar de novo.",
        ),
        (NoticeKind::CaptureFailed, false) => (
            "Activity capture stopped",
            "Open Today to see what happened.",
        ),
        (NoticeKind::CaptureFailed, true) => (
            "A captura de atividade parou",
            "Abra Hoje para ver o que aconteceu.",
        ),
    };
    let snooze = if pt { "Adiar 1h" } else { "Snooze 1h" };
    (
        title.to_string(),
        detail.unwrap_or_else(|| fallback.to_string()),
        snooze.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::super::schedule::FixedZone;
    use super::*;
    use crate::activity::settings::QuietHours;

    fn zone() -> FixedZone {
        FixedZone(chrono::FixedOffset::west_opt(3 * 3600).unwrap())
    }

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        zone()
            .instant(
                chrono::NaiveDate::from_ymd_opt(2026, 10, day)
                    .unwrap()
                    .and_hms_opt(hour, 0, 0)
                    .unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn notifications_off_drop_everything() {
        let zone = zone();
        let settings = ActivityNotificationSettings {
            enabled: false,
            ..ActivityNotificationSettings::default()
        };
        let mut queue = NoticeQueue::default();
        queue.raise(
            NoticeKind::SummaryReady,
            "2026-10-04".into(),
            None,
            at(4, 18),
            &zone,
            &settings,
        );
        assert_eq!(queue.next_delivery(), None);
    }

    #[test]
    fn snoozed_and_replaced_notices() {
        let zone = zone();
        let settings = ActivityNotificationSettings::default();
        let mut queue = NoticeQueue::default();
        let now = at(4, 18);
        queue.raise(
            NoticeKind::SummaryReady,
            "2026-10-04".into(),
            None,
            now,
            &zone,
            &settings,
        );
        queue.raise(
            NoticeKind::SummaryReady,
            "2026-10-04".into(),
            Some("h".into()),
            now,
            &zone,
            &settings,
        );
        let due = queue.take_due(now, &zone, &settings);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].detail.as_deref(), Some("h"));
        queue.snooze(NoticeKind::SummaryReady, "2026-10-04".into(), None, now);
        assert!(queue
            .take_due(now + Duration::minutes(59), &zone, &settings)
            .is_empty());
        assert_eq!(queue.take_due(now + SNOOZE, &zone, &settings).len(), 1);
    }

    #[test]
    fn quiet_hours_set_while_waiting_still_hold_the_notice() {
        let zone = zone();
        let mut settings = ActivityNotificationSettings::default();
        let mut queue = NoticeQueue::default();
        queue.raise(
            NoticeKind::CaptureFailed,
            "2026-10-04".into(),
            None,
            at(4, 21),
            &zone,
            &settings,
        );
        settings.quiet_hours = QuietHours {
            enabled: true,
            start: "21:00".into(),
            end: "07:00".into(),
        };
        assert!(queue
            .take_due(at(4, 21) + Duration::minutes(1), &zone, &settings)
            .is_empty());
        assert_eq!(queue.next_delivery(), Some(at(5, 7)));
    }

    #[test]
    fn copy_follows_the_interface_language() {
        let notice = Notice {
            kind: NoticeKind::SummaryReady,
            day: "2026-10-04".into(),
            detail: None,
            deliver_at: at(4, 18),
        };
        let (title, body, snooze) = copy(&notice, UiLocale::PtBr);
        assert_eq!(title, "O resumo do seu dia está pronto");
        assert_eq!(body, "Abra Hoje para revisar o seu dia.");
        assert_eq!(snooze, "Adiar 1h");
        assert_eq!(copy(&notice, UiLocale::En).2, "Snooze 1h");
    }
}
