use crate::domain::types::{
    AppError, NoteDto, RecordingOrigin, RecordingOriginMetadata, RecordingSessionDto,
    RecordingVideoRequest, StartMeetingRecordingRequest,
};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager, State};

mod windows_probe;

use windows_probe::find_windows_mic_app;

const CLEAR_AFTER_INACTIVE_POLLS: u8 = 2;
const HEARTBEAT_EVERY_ACTIVE_POLLS: u8 = 5;
const MEETING_END_ABSENT_POLLS: u8 = 15;
const MEETING_END_ABSENCE_MS: u64 = 15_000;
const MEETING_END_COUNTDOWN_POLLS: u8 = 15;
const MEETING_END_COUNTDOWN_MS: u64 = 15_000;
const MEETING_END_MAX_PROBE_GAP_MS: u64 = 2_500;
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const MEETING_DETECTION_EVENT_NAME: &str = "meeting-detection-event";
const MEETING_START_REQUEST_EVENT_NAME: &str = "clovy://meeting-start-transcription";
const MEETING_START_REQUEST_TTL_MS: u64 = 30_000;
pub const MEETING_END_STATE_EVENT_NAME: &str = "meeting-end-state-event";
pub const MEETING_END_FINISH_REQUEST_EVENT_NAME: &str = "clovy://meeting-end-finish";

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMeetingStartRequest {
    request_id: String,
    note_id: String,
    requested_at_ms: u64,
    expired: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    video: Option<RecordingVideoRequest>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum MeetingStartRecordingOutcome {
    Started {
        note: Box<NoteDto>,
        recording: Box<RecordingSessionDto>,
    },
    Failed {
        error: AppError,
    },
}

#[derive(Debug)]
enum MeetingStartRequestPhase {
    Queued,
    Starting,
    Finished(Box<MeetingStartRecordingOutcome>),
}

#[derive(Debug)]
struct MeetingStartRequest {
    request_id: String,
    note_id: String,
    requested_at_ms: u64,
    bundle_families: Vec<String>,
    video: Option<RecordingVideoRequest>,
    phase: MeetingStartRequestPhase,
}

#[derive(Debug, Default)]
struct MeetingStartRequestMailbox {
    next_request_id: u64,
    pending: Option<MeetingStartRequest>,
}

#[derive(Debug, Default)]
pub struct MeetingStartRequestState {
    mailbox: Mutex<MeetingStartRequestMailbox>,
    detected_bundle_families: Mutex<BTreeSet<String>>,
    meeting_end: Mutex<Option<MeetingEndTracker>>,
    start_lock: tokio::sync::Mutex<()>,
}

impl MeetingStartRequestState {
    #[cfg(test)]
    fn queue_at(&self, requested_at_ms: u64) -> Result<String, String> {
        self.queue_with_video_at(requested_at_ms, None)
    }

    fn queue_with_video_at(
        &self,
        requested_at_ms: u64,
        video: Option<RecordingVideoRequest>,
    ) -> Result<String, String> {
        let mut mailbox = self
            .mailbox
            .lock()
            .map_err(|_| "meeting start request state is unavailable".to_string())?;
        if let Some(pending) = mailbox.pending.as_ref() {
            if matches!(pending.phase, MeetingStartRequestPhase::Starting) {
                return Ok(pending.request_id.clone());
            }
        }

        // Every queued click gets a new generation, even though the one-slot
        // mailbox still replaces the older intent. An acknowledgement based
        // on an expired snapshot can therefore never clear a newer click.
        mailbox.next_request_id = mailbox.next_request_id.saturating_add(1).max(1);
        let request_id = mailbox.next_request_id.to_string();
        mailbox.pending = Some(MeetingStartRequest {
            request_id: request_id.clone(),
            note_id: uuid::Uuid::new_v4().to_string(),
            requested_at_ms,
            bundle_families: self
                .detected_bundle_families
                .lock()
                .map_err(|_| "meeting detection state is unavailable".to_string())?
                .iter()
                .cloned()
                .collect(),
            video,
            phase: MeetingStartRequestPhase::Queued,
        });
        Ok(request_id)
    }

    fn peek_at(&self, now_ms: u64) -> Result<Option<PendingMeetingStartRequest>, String> {
        let mailbox = self
            .mailbox
            .lock()
            .map_err(|_| "meeting start request state is unavailable".to_string())?;
        Ok(mailbox
            .pending
            .as_ref()
            .map(|pending| PendingMeetingStartRequest {
                request_id: pending.request_id.clone(),
                note_id: pending.note_id.clone(),
                requested_at_ms: pending.requested_at_ms,
                video: pending.video.clone(),
                expired: matches!(pending.phase, MeetingStartRequestPhase::Queued)
                    && now_ms.saturating_sub(pending.requested_at_ms)
                        > MEETING_START_REQUEST_TTL_MS,
            }))
    }

    fn acknowledge(&self, request_id: &str) -> Result<bool, String> {
        let mut mailbox = self
            .mailbox
            .lock()
            .map_err(|_| "meeting start request state is unavailable".to_string())?;
        let matches = mailbox.pending.as_ref().is_some_and(|pending| {
            pending.request_id == request_id
                && !matches!(pending.phase, MeetingStartRequestPhase::Starting)
        });
        if matches {
            mailbox.pending = None;
        }
        Ok(matches)
    }

    pub async fn lock_start(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.start_lock.lock().await
    }

    pub fn begin_start_now(&self, request_id: &str) -> Result<String, AppError> {
        let now_ms = wall_clock_millis()
            .map_err(|message| AppError::new("meeting_start_unavailable", message))?;
        self.begin_start(request_id, now_ms)
    }

    pub fn begin_start(&self, request_id: &str, now_ms: u64) -> Result<String, AppError> {
        let mut mailbox = self.mailbox.lock().map_err(|_| {
            AppError::new(
                "meeting_start_unavailable",
                "The meeting start request is unavailable.",
            )
        })?;
        let pending = mailbox.pending.as_mut().ok_or_else(|| {
            AppError::new(
                "meeting_start_not_found",
                "The meeting start request was not found.",
            )
        })?;
        if pending.request_id != request_id {
            return Err(AppError::new(
                "meeting_start_not_found",
                "The meeting start request is no longer current.",
            ));
        }
        match &pending.phase {
            MeetingStartRequestPhase::Queued => {
                if now_ms.saturating_sub(pending.requested_at_ms) > MEETING_START_REQUEST_TTL_MS {
                    return Err(AppError::new(
                        "meeting_start_expired",
                        "The meeting start request expired before recording began.",
                    ));
                }
                pending.phase = MeetingStartRequestPhase::Starting;
                Ok(pending.note_id.clone())
            }
            MeetingStartRequestPhase::Starting => Err(AppError::new(
                "meeting_start_in_progress",
                "The meeting recording is already starting.",
            )),
            MeetingStartRequestPhase::Finished(_) => Err(AppError::new(
                "meeting_start_already_finished",
                "The meeting start request already finished.",
            )),
        }
    }

    pub fn start_bundle_families(&self, request_id: &str) -> Result<Vec<String>, AppError> {
        let mailbox = self.mailbox.lock().map_err(|_| {
            AppError::new(
                "meeting_start_unavailable",
                "The meeting start request is unavailable.",
            )
        })?;
        let pending = mailbox
            .pending
            .as_ref()
            .filter(|pending| pending.request_id == request_id)
            .ok_or_else(|| {
                AppError::new(
                    "meeting_start_not_found",
                    "The meeting start request was not found.",
                )
            })?;
        Ok(pending.bundle_families.clone())
    }

    pub fn start_video(
        &self,
        request: &StartMeetingRecordingRequest,
    ) -> Result<Option<RecordingVideoRequest>, AppError> {
        let mailbox = self.mailbox.lock().map_err(|_| {
            AppError::new(
                "meeting_start_unavailable",
                "The meeting start request is unavailable.",
            )
        })?;
        let pending = mailbox
            .pending
            .as_ref()
            .filter(|pending| pending.request_id == request.request_id)
            .ok_or_else(|| {
                AppError::new(
                    "meeting_start_not_found",
                    "The meeting start request was not found.",
                )
            })?;
        // The HUD owns the opt-in and selected window. Renderer retries may carry
        // stale data, so they cannot enable video or replace that retained choice.
        Ok(pending.video.clone())
    }

    pub fn finished_outcome(
        &self,
        request_id: &str,
    ) -> Result<Option<MeetingStartRecordingOutcome>, AppError> {
        let mailbox = self.mailbox.lock().map_err(|_| {
            AppError::new(
                "meeting_start_unavailable",
                "The meeting start request is unavailable.",
            )
        })?;
        let Some(pending) = mailbox
            .pending
            .as_ref()
            .filter(|pending| pending.request_id == request_id)
        else {
            return Err(AppError::new(
                "meeting_start_not_found",
                "The meeting start request was not found.",
            ));
        };
        Ok(match &pending.phase {
            MeetingStartRequestPhase::Finished(outcome) => Some((**outcome).clone()),
            _ => None,
        })
    }

    pub fn finish_start(
        &self,
        request_id: &str,
        outcome: MeetingStartRecordingOutcome,
    ) -> Result<(), AppError> {
        let mut mailbox = self.mailbox.lock().map_err(|_| {
            AppError::new(
                "meeting_start_unavailable",
                "The meeting start request is unavailable.",
            )
        })?;
        let Some(pending) = mailbox
            .pending
            .as_mut()
            .filter(|pending| pending.request_id == request_id)
        else {
            return Err(AppError::new(
                "meeting_start_not_found",
                "The meeting start request was not found.",
            ));
        };
        pending.phase = MeetingStartRequestPhase::Finished(Box::new(outcome));
        Ok(())
    }

    pub fn fail_start_if_running(&self, request_id: &str) -> Result<bool, AppError> {
        let mut mailbox = self.mailbox.lock().map_err(|_| {
            AppError::new(
                "meeting_start_unavailable",
                "The meeting start request is unavailable.",
            )
        })?;
        let Some(pending) = mailbox
            .pending
            .as_mut()
            .filter(|pending| pending.request_id == request_id)
        else {
            return Ok(false);
        };
        if !matches!(pending.phase, MeetingStartRequestPhase::Starting) {
            return Ok(false);
        }
        pending.phase =
            MeetingStartRequestPhase::Finished(Box::new(MeetingStartRecordingOutcome::Failed {
                error: AppError::new(
                    "meeting_start_interrupted",
                    "Meeting recording stopped before startup completed. Try again.",
                ),
            }));
        Ok(true)
    }

    fn set_detected_bundle_families(
        &self,
        bundle_families: BTreeSet<String>,
    ) -> Result<(), String> {
        *self
            .detected_bundle_families
            .lock()
            .map_err(|_| "meeting detection state is unavailable".to_string())? = bundle_families;
        Ok(())
    }

    pub fn arm_meeting_end(
        &self,
        session_id: String,
        origin: &RecordingOriginMetadata,
    ) -> Result<Option<MeetingEndStatus>, AppError> {
        if origin.origin != RecordingOrigin::MeetingPrompt || !origin.auto_finish_eligible {
            return Ok(None);
        }
        let tracked_families = origin
            .meeting_app_bundle_families
            .iter()
            .cloned()
            .filter_map(|family| {
                let family = family.trim().to_ascii_lowercase();
                (!family.is_empty()).then_some(family)
            })
            .collect::<BTreeSet<_>>();
        if tracked_families.is_empty() {
            return Ok(None);
        }
        let tracker = MeetingEndTracker::new(session_id, tracked_families);
        let status = tracker.status();
        *self.meeting_end.lock().map_err(|_| {
            AppError::new(
                "meeting_end_unavailable",
                "Meeting end detection is unavailable.",
            )
        })? = Some(tracker);
        Ok(Some(status))
    }

    /// Dev-only demo hook: install a countdown-phase tracker for the given
    /// session so the end-of-meeting UI can be exercised without a meeting
    /// app. The tracked family never holds the mic, so from here on the
    /// production machinery runs unmodified — the poll keeps observing, Keep
    /// and Stop work for real, and expiry queues a real auto-finish.
    #[cfg(debug_assertions)]
    fn force_meeting_end_countdown(
        &self,
        session_id: String,
        now_ms: u64,
    ) -> Result<MeetingEndStatus, String> {
        let mut tracker = MeetingEndTracker::new(
            session_id,
            BTreeSet::from(["clovy.debug.meeting-end".to_string()]),
        );
        tracker.phase = MeetingEndTrackerPhase::Countdown {
            expires_at_ms: now_ms.saturating_add(MEETING_END_COUNTDOWN_MS),
            countdown_polls: 0,
        };
        let status = tracker.status();
        *self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())? = Some(tracker);
        Ok(status)
    }

    fn observe_meeting_end(
        &self,
        active_bundle_families: Option<&BTreeSet<String>>,
        now_ms: u64,
    ) -> Result<Option<MeetingEndTransition>, String> {
        let mut guard = self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())?;
        Ok(guard
            .as_mut()
            .and_then(|tracker| tracker.observe(active_bundle_families, now_ms)))
    }

    fn clear_meeting_end_if_inactive<F>(&self, current_session_id: F) -> Result<bool, String>
    where
        F: FnOnce() -> Option<String>,
    {
        let mut guard = self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())?;
        let Some(tracker) = guard.as_ref() else {
            return Ok(false);
        };
        // Read capture ownership while holding the tracker lock. An arm that
        // races this reconciliation is therefore ordered entirely before the
        // live read or entirely after this no-op/clear decision.
        let active_session_id = current_session_id();
        let should_clear = Some(tracker.session_id.as_str()) != active_session_id.as_deref();
        if should_clear {
            *guard = None;
        }
        Ok(should_clear)
    }

    fn meeting_end_status(&self) -> Result<Option<MeetingEndStatus>, String> {
        let guard = self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())?;
        Ok(guard.as_ref().map(MeetingEndTracker::status))
    }

    fn queue_meeting_end_finish(
        &self,
        session_id: &str,
    ) -> Result<(MeetingEndStatus, PendingMeetingEndFinishRequest), String> {
        let mut guard = self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())?;
        let tracker = guard
            .as_mut()
            .filter(|tracker| tracker.session_id == session_id)
            .ok_or_else(|| "meeting end request is no longer current".to_string())?;
        let request = tracker.queue_finish()?;
        Ok((tracker.status(), request))
    }

    fn keep_meeting_recording(&self, session_id: &str) -> Result<MeetingEndStatus, String> {
        let mut guard = self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())?;
        let tracker = guard
            .as_mut()
            .filter(|tracker| tracker.session_id == session_id)
            .ok_or_else(|| "meeting end request is no longer current".to_string())?;
        tracker.keep_recording()?;
        Ok(tracker.status())
    }

    fn pending_meeting_end_finish(&self) -> Result<Option<PendingMeetingEndFinishRequest>, String> {
        let guard = self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())?;
        Ok(guard
            .as_ref()
            .and_then(|tracker| tracker.pending_finish.clone()))
    }

    fn acknowledge_meeting_end_finish(&self, request_id: &str) -> Result<bool, String> {
        let mut guard = self
            .meeting_end
            .lock()
            .map_err(|_| "meeting end detection state is unavailable".to_string())?;
        let Some(tracker) = guard.as_mut() else {
            return Ok(false);
        };
        let matches = tracker
            .pending_finish
            .as_ref()
            .is_some_and(|request| request.request_id == request_id);
        if matches {
            tracker.pending_finish = None;
        }
        Ok(matches)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MeetingEndPhase {
    Tracking,
    Countdown,
    Suppressed,
    FinishQueued,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingEndStatus {
    pub session_id: String,
    pub phase: MeetingEndPhase,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMeetingEndFinishRequest {
    pub request_id: String,
    pub session_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MeetingEndTrackerPhase {
    Tracking {
        absent_polls: u8,
    },
    Countdown {
        expires_at_ms: u64,
        countdown_polls: u8,
    },
    Suppressed,
    FinishQueued,
}

#[derive(Clone, Debug)]
struct MeetingEndTracker {
    session_id: String,
    tracked_families: BTreeSet<String>,
    phase: MeetingEndTrackerPhase,
    last_successful_probe_at_ms: Option<u64>,
    absence_started_at_ms: Option<u64>,
    next_request_id: u64,
    pending_finish: Option<PendingMeetingEndFinishRequest>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MeetingEndTransition {
    StateChanged(MeetingEndStatus),
    FinishQueued(MeetingEndStatus, PendingMeetingEndFinishRequest),
}

impl MeetingEndTracker {
    fn new(session_id: String, tracked_families: BTreeSet<String>) -> Self {
        Self {
            session_id,
            tracked_families,
            phase: MeetingEndTrackerPhase::Tracking { absent_polls: 0 },
            last_successful_probe_at_ms: None,
            absence_started_at_ms: None,
            next_request_id: 0,
            pending_finish: None,
        }
    }

    fn status(&self) -> MeetingEndStatus {
        let (phase, expires_at_ms) = match self.phase {
            MeetingEndTrackerPhase::Tracking { .. } => (MeetingEndPhase::Tracking, None),
            MeetingEndTrackerPhase::Countdown { expires_at_ms, .. } => {
                (MeetingEndPhase::Countdown, Some(expires_at_ms))
            }
            MeetingEndTrackerPhase::Suppressed => (MeetingEndPhase::Suppressed, None),
            MeetingEndTrackerPhase::FinishQueued => (MeetingEndPhase::FinishQueued, None),
        };
        MeetingEndStatus {
            session_id: self.session_id.clone(),
            phase,
            expires_at_ms,
        }
    }

    fn observe(
        &mut self,
        active_bundle_families: Option<&BTreeSet<String>>,
        now_ms: u64,
    ) -> Option<MeetingEndTransition> {
        let Some(active_bundle_families) = active_bundle_families else {
            self.last_successful_probe_at_ms = None;
            return self.reset_after_ambiguous_probe();
        };

        let previous_probe_at_ms = self.last_successful_probe_at_ms;
        let probe_gap = previous_probe_at_ms.map(|previous| now_ms.saturating_sub(previous));
        let contiguous_probe = probe_gap.is_some_and(|gap| gap <= MEETING_END_MAX_PROBE_GAP_MS);
        self.last_successful_probe_at_ms = Some(now_ms);
        if probe_gap.is_some_and(|gap| gap > MEETING_END_MAX_PROBE_GAP_MS) {
            let transition = self.reset_after_ambiguous_probe();
            if transition.is_some() {
                return transition;
            }
        }

        if matches!(
            self.phase,
            MeetingEndTrackerPhase::Suppressed | MeetingEndTrackerPhase::FinishQueued
        ) {
            return None;
        }

        let originating_app_active = !self.tracked_families.is_disjoint(active_bundle_families);
        if originating_app_active {
            self.absence_started_at_ms = None;
            return match self.phase {
                MeetingEndTrackerPhase::Countdown { .. } => {
                    self.phase = MeetingEndTrackerPhase::Tracking { absent_polls: 0 };
                    Some(MeetingEndTransition::StateChanged(self.status()))
                }
                MeetingEndTrackerPhase::Tracking {
                    ref mut absent_polls,
                } => {
                    *absent_polls = 0;
                    None
                }
                _ => None,
            };
        }

        match &mut self.phase {
            MeetingEndTrackerPhase::Tracking { absent_polls } => {
                let absence_started_at_ms = *self.absence_started_at_ms.get_or_insert_with(|| {
                    if contiguous_probe {
                        previous_probe_at_ms.unwrap_or(now_ms)
                    } else {
                        now_ms
                    }
                });
                *absent_polls = absent_polls.saturating_add(1);
                if *absent_polls < MEETING_END_ABSENT_POLLS
                    || now_ms.saturating_sub(absence_started_at_ms) < MEETING_END_ABSENCE_MS
                {
                    return None;
                }
                self.phase = MeetingEndTrackerPhase::Countdown {
                    expires_at_ms: now_ms.saturating_add(MEETING_END_COUNTDOWN_MS),
                    countdown_polls: 0,
                };
                Some(MeetingEndTransition::StateChanged(self.status()))
            }
            MeetingEndTrackerPhase::Countdown {
                expires_at_ms,
                countdown_polls,
            } => {
                *countdown_polls = countdown_polls.saturating_add(1);
                if *countdown_polls < MEETING_END_COUNTDOWN_POLLS || now_ms < *expires_at_ms {
                    return None;
                }
                let request = self
                    .queue_finish()
                    .expect("countdown phase can queue one finish request");
                Some(MeetingEndTransition::FinishQueued(self.status(), request))
            }
            MeetingEndTrackerPhase::Suppressed | MeetingEndTrackerPhase::FinishQueued => None,
        }
    }

    fn reset_after_ambiguous_probe(&mut self) -> Option<MeetingEndTransition> {
        self.absence_started_at_ms = None;
        match self.phase {
            MeetingEndTrackerPhase::Tracking {
                ref mut absent_polls,
            } => {
                *absent_polls = 0;
                None
            }
            MeetingEndTrackerPhase::Countdown { .. } => {
                self.phase = MeetingEndTrackerPhase::Tracking { absent_polls: 0 };
                Some(MeetingEndTransition::StateChanged(self.status()))
            }
            MeetingEndTrackerPhase::Suppressed | MeetingEndTrackerPhase::FinishQueued => None,
        }
    }

    fn queue_finish(&mut self) -> Result<PendingMeetingEndFinishRequest, String> {
        if let Some(request) = self.pending_finish.clone() {
            return Ok(request);
        }
        if !matches!(self.phase, MeetingEndTrackerPhase::Countdown { .. }) {
            return Err("meeting end countdown is not active".to_string());
        }
        self.next_request_id = self.next_request_id.saturating_add(1).max(1);
        let request = PendingMeetingEndFinishRequest {
            request_id: format!("{}:{}", self.session_id, self.next_request_id),
            session_id: self.session_id.clone(),
        };
        self.phase = MeetingEndTrackerPhase::FinishQueued;
        self.pending_finish = Some(request.clone());
        Ok(request)
    }

    fn keep_recording(&mut self) -> Result<(), String> {
        if !matches!(self.phase, MeetingEndTrackerPhase::Countdown { .. }) {
            return Err("meeting end countdown is not active".to_string());
        }
        self.phase = MeetingEndTrackerPhase::Suppressed;
        self.pending_finish = None;
        Ok(())
    }
}

fn wall_clock_millis() -> Result<u64, String> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock is before the Unix epoch".to_string())?
        .as_millis();
    u64::try_from(millis).map_err(|_| "system clock is outside the supported range".to_string())
}

/// Stores the HUD action before emitting the wake event. Tauri events are not
/// buffered while a webview listener is registering, so the main window also
/// reads this one-slot mailbox after its listener and app bootstrap are ready.
#[tauri::command]
pub fn queue_meeting_start_request(
    app: AppHandle,
    state: State<'_, MeetingStartRequestState>,
    video: Option<RecordingVideoRequest>,
) -> Result<(), String> {
    state.queue_with_video_at(wall_clock_millis()?, video)?;
    app.emit(MEETING_START_REQUEST_EVENT_NAME, ())
        .map_err(|error| format!("failed to wake the meeting-start listener: {error}"))?;
    Ok(())
}

#[tauri::command]
pub fn pending_meeting_start_request(
    state: State<'_, MeetingStartRequestState>,
) -> Result<Option<PendingMeetingStartRequest>, String> {
    state.peek_at(wall_clock_millis()?)
}

#[tauri::command]
pub fn acknowledge_meeting_start_request(
    state: State<'_, MeetingStartRequestState>,
    request_id: String,
) -> Result<bool, String> {
    state.acknowledge(&request_id)
}

#[tauri::command]
pub fn pending_meeting_end_status(
    state: State<'_, MeetingStartRequestState>,
) -> Result<Option<MeetingEndStatus>, String> {
    state.meeting_end_status()
}

#[tauri::command]
pub fn pending_meeting_end_finish_request(
    state: State<'_, MeetingStartRequestState>,
) -> Result<Option<PendingMeetingEndFinishRequest>, String> {
    state.pending_meeting_end_finish()
}

#[tauri::command]
pub fn queue_meeting_end_finish_request(
    app: AppHandle,
    state: State<'_, MeetingStartRequestState>,
    session_id: String,
) -> Result<(), String> {
    let (status, request) = state.queue_meeting_end_finish(&session_id)?;
    emit_meeting_end_state(&app, Some(status));
    emit_meeting_end_finish_request(&app, &request);
    Ok(())
}

#[tauri::command]
pub fn keep_meeting_recording(
    app: AppHandle,
    state: State<'_, MeetingStartRequestState>,
    session_id: String,
) -> Result<(), String> {
    let status = state.keep_meeting_recording(&session_id)?;
    emit_meeting_end_state(&app, Some(status));
    Ok(())
}

#[tauri::command]
pub fn acknowledge_meeting_end_finish_request(
    state: State<'_, MeetingStartRequestState>,
    request_id: String,
) -> Result<bool, String> {
    state.acknowledge_meeting_end_finish(&request_id)
}

/// Dev-only console-demo hook (`__recordingHud("end")` in the main window's
/// devtools): forces the live recording into the real meeting-end countdown.
/// No-op error in release builds.
#[tauri::command]
pub fn debug_force_meeting_end_countdown(
    app: AppHandle,
    state: State<'_, MeetingStartRequestState>,
) -> Result<MeetingEndStatus, String> {
    #[cfg(debug_assertions)]
    {
        let session_id = crate::audio::capture::current_status()
            .map(|status| status.session_id)
            .ok_or_else(|| "no live recording to end - start one first".to_string())?;
        let status = state.force_meeting_end_countdown(session_id, wall_clock_millis()?)?;
        emit_meeting_end_state(&app, Some(status.clone()));
        Ok(status)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (app, state);
        Err("debug commands are unavailable in release builds".to_string())
    }
}

pub fn arm_meeting_end_for_recording(
    app: &AppHandle,
    session_id: String,
    origin: &RecordingOriginMetadata,
) -> Result<bool, AppError> {
    let state = app.state::<MeetingStartRequestState>();
    let Some(status) = state.arm_meeting_end(session_id, origin)? else {
        return Ok(false);
    };
    emit_meeting_end_state(app, Some(status));
    Ok(true)
}

struct AllowedMicApp {
    bundle_prefix: &'static str,
    label: &'static str,
}

const fn mic_app(bundle_prefix: &'static str, label: &'static str) -> AllowedMicApp {
    AllowedMicApp {
        bundle_prefix,
        label,
    }
}

const ALLOWED_MIC_APPS: &[AllowedMicApp] = &[
    mic_app("ai.perplexity.comet", "Comet"),
    mic_app("Cisco-Systems.Spark", "Webex"),
    mic_app("com.apple.FaceTime", "FaceTime"),
    mic_app("com.apple.Safari", "Safari"),
    mic_app("com.brave.Browser", "Brave"),
    mic_app("com.cisco.webexmeetingsapp", "Webex"),
    mic_app("com.gather.GatherV2", "Gather"),
    mic_app("com.google.Chrome", "Chrome"),
    mic_app("com.hnc.Discord", "Discord"),
    mic_app("com.microsoft.edgemac", "Edge"),
    mic_app("com.microsoft.teams", "Teams"),
    mic_app("com.microsoft.teams2", "Teams"),
    mic_app("com.operasoftware.Opera", "Opera"),
    mic_app("com.tinyspeck.slackmacgap", "Slack"),
    mic_app("com.vivaldi.Vivaldi", "Vivaldi"),
    mic_app("company.thebrowser.Browser", "Arc"),
    mic_app("company.thebrowser.dia", "Dia"),
    mic_app("net.whatsapp.WhatsApp", "WhatsApp"),
    mic_app("org.mozilla.firefox", "Firefox"),
    mic_app("org.mozilla.firefoxdeveloperedition", "Firefox"),
    mic_app("org.mozilla.nightly", "Firefox"),
    mic_app("org.telegram.desktop", "Telegram"),
    mic_app("org.whispersystems.signal-desktop", "Signal"),
    mic_app("ru.keepcoder.Telegram", "Telegram"),
    mic_app("us.zoom.xos", "Zoom"),
];

pub fn setup(app: &mut tauri::App) {
    app.manage(MeetingStartRequestState::default());

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    spawn_monitor(app.handle().clone());

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = app;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MeetingDetectionEvent {
    Detected,
    /// Periodic re-emit while a meeting stays active, so a HUD webview that
    /// missed the initial event (e.g. after a reload) can still catch up.
    Heartbeat,
    Cleared,
}

#[derive(Debug, Default)]
pub(crate) struct MeetingDetectionState {
    active: bool,
    inactive_polls: u8,
    active_polls_since_emit: u8,
}

impl MeetingDetectionState {
    pub(crate) fn update(
        &mut self,
        signed_in: bool,
        active_external_input: bool,
        clovy_capture_active: bool,
    ) -> Option<MeetingDetectionEvent> {
        if !signed_in {
            return self.clear();
        }

        let should_be_active = active_external_input && !clovy_capture_active;
        if should_be_active {
            self.inactive_polls = 0;
            if !self.active {
                self.active = true;
                self.active_polls_since_emit = 0;
                return Some(MeetingDetectionEvent::Detected);
            }

            self.active_polls_since_emit = self.active_polls_since_emit.saturating_add(1);
            if self.active_polls_since_emit >= HEARTBEAT_EVERY_ACTIVE_POLLS {
                self.active_polls_since_emit = 0;
                return Some(MeetingDetectionEvent::Heartbeat);
            }
            return None;
        }

        self.active_polls_since_emit = 0;
        if !self.active {
            self.inactive_polls = 0;
            return None;
        }

        self.inactive_polls = self.inactive_polls.saturating_add(1);
        if self.inactive_polls >= CLEAR_AFTER_INACTIVE_POLLS {
            self.active = false;
            self.inactive_polls = 0;
            return Some(MeetingDetectionEvent::Cleared);
        }

        None
    }

    fn clear(&mut self) -> Option<MeetingDetectionEvent> {
        self.inactive_polls = 0;
        self.active_polls_since_emit = 0;
        if self.active {
            self.active = false;
            return Some(MeetingDetectionEvent::Cleared);
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MicrophoneInputProcess {
    pub(crate) pid: u32,
    pub(crate) bundle_id: String,
    pub(crate) app_label: String,
}

impl MicrophoneInputProcess {
    pub(crate) fn new(pid: u32, bundle_id: String) -> Option<Self> {
        let bundle_id = bundle_id.trim().to_string();
        if pid == 0 || bundle_id.is_empty() {
            return None;
        }
        let app_label = app_label_from_bundle_id(&bundle_id);
        Some(Self {
            pid,
            bundle_id,
            app_label,
        })
    }
}

pub(crate) fn active_allowed_external_processes(
    active_input_processes: &[MicrophoneInputProcess],
    owned_pids: &BTreeSet<u32>,
) -> Vec<MicrophoneInputProcess> {
    active_input_processes
        .iter()
        .filter(|process| process.pid != 0 && !owned_pids.contains(&process.pid))
        .filter(|process| is_allowed_microphone_app(&process.bundle_id))
        .cloned()
        .collect()
}

fn is_allowed_microphone_app(bundle_id: &str) -> bool {
    allowed_mic_app(bundle_id).is_some() || find_windows_mic_app(bundle_id).is_some()
}

fn bundle_id_matches_prefix(bundle_id: &str, prefix: &str) -> bool {
    let bundle_id = bundle_id.trim().to_ascii_lowercase();
    let prefix = prefix.trim().to_ascii_lowercase();
    bundle_id == prefix
        || bundle_id
            .strip_prefix(&prefix)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn app_label_from_bundle_id(bundle_id: &str) -> String {
    if let Some(app) = allowed_mic_app(bundle_id) {
        return app.label.to_string();
    }
    if let Some(app) = find_windows_mic_app(bundle_id) {
        return app.label.to_string();
    }
    // ASCII lowercasing keeps byte offsets, so the stem length slices the
    // original spelling ("OBS64.exe" -> "OBS64").
    if let Some(stem) = bundle_id
        .to_ascii_lowercase()
        .strip_suffix(".exe")
        .filter(|stem| !stem.is_empty())
    {
        return bundle_id[..stem.len()].to_string();
    }
    bundle_id
        .rsplit('.')
        .find(|part| !part.trim().is_empty())
        .unwrap_or(bundle_id)
        .to_string()
}

fn allowed_mic_app(bundle_id: &str) -> Option<&'static AllowedMicApp> {
    ALLOWED_MIC_APPS
        .iter()
        .find(|app| bundle_id_matches_prefix(bundle_id, app.bundle_prefix))
}

fn bundle_family_from_bundle_id(bundle_id: &str) -> Option<String> {
    allowed_mic_app(bundle_id)
        .map(|app| app.bundle_prefix.to_ascii_lowercase())
        .or_else(|| find_windows_mic_app(bundle_id).map(|app| app.family.to_string()))
}

fn meeting_end_bundle_families(
    active_input_processes: &[MicrophoneInputProcess],
    owned_pids: &BTreeSet<u32>,
) -> Option<BTreeSet<String>> {
    let external_processes = active_input_processes
        .iter()
        .filter(|process| process.pid != 0 && !owned_pids.contains(&process.pid))
        .collect::<Vec<_>>();
    if external_processes
        .iter()
        .any(|process| !is_allowed_microphone_app(&process.bundle_id))
    {
        return None;
    }
    Some(
        external_processes
            .into_iter()
            .filter_map(|process| bundle_family_from_bundle_id(&process.bundle_id))
            .collect(),
    )
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn spawn_monitor(app: AppHandle) {
    std::thread::spawn(move || {
        let mut state = MeetingDetectionState::default();
        let mut warned_after_probe_error = false;

        loop {
            std::thread::sleep(POLL_INTERVAL);

            let active_processes = match active_input_processes() {
                Ok(active_processes) => {
                    warned_after_probe_error = false;
                    Some(active_processes)
                }
                Err(error) => {
                    if !warned_after_probe_error {
                        tracing::warn!(%error, "meeting detection probe failed");
                        warned_after_probe_error = true;
                    }
                    None
                }
            };
            let owned_pids = owned_pids(&app);
            let allowed_processes = active_processes
                .as_ref()
                .map(|processes| active_allowed_external_processes(processes, &owned_pids));
            let bundle_families = active_processes
                .as_ref()
                .and_then(|processes| meeting_end_bundle_families(processes, &owned_pids));
            if let (Some(runtime), Some(bundle_families)) = (
                app.try_state::<MeetingStartRequestState>(),
                bundle_families.as_ref(),
            ) {
                if let Err(error) = runtime.set_detected_bundle_families(bundle_families.clone()) {
                    tracing::warn!(%error, "failed to retain detected meeting app families");
                }
            }

            let capture_status = crate::audio::capture::current_status();
            let capture_active = capture_status.is_some();
            if let Some(runtime) = app.try_state::<MeetingStartRequestState>() {
                match runtime.clear_meeting_end_if_inactive(|| {
                    crate::audio::capture::current_status().map(|status| status.session_id)
                }) {
                    Ok(true) => emit_meeting_end_state(&app, None),
                    Ok(false) => {}
                    Err(error) => {
                        tracing::warn!(%error, "failed to reconcile meeting end session")
                    }
                }
            }
            observe_meeting_end(&app, bundle_families.as_ref());

            let external_input_active = allowed_processes
                .as_ref()
                .is_some_and(|processes| !processes.is_empty());
            if let Some(event) = state.update(true, external_input_active, capture_active) {
                emit_detection_event(
                    &app,
                    event,
                    allowed_processes.as_deref().unwrap_or_default(),
                );
            }
        }
    });
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn observe_meeting_end(app: &AppHandle, active_bundle_families: Option<&BTreeSet<String>>) {
    let Some(state) = app.try_state::<MeetingStartRequestState>() else {
        return;
    };
    let now_ms = match wall_clock_millis() {
        Ok(now_ms) => now_ms,
        Err(error) => {
            tracing::warn!(%error, "meeting end clock unavailable");
            return;
        }
    };
    match state.observe_meeting_end(active_bundle_families, now_ms) {
        Ok(Some(MeetingEndTransition::StateChanged(status))) => {
            emit_meeting_end_state(app, Some(status));
        }
        Ok(Some(MeetingEndTransition::FinishQueued(status, request))) => {
            emit_meeting_end_state(app, Some(status));
            emit_meeting_end_finish_request(app, &request);
        }
        Ok(None) => {}
        Err(error) => tracing::warn!(%error, "meeting end detection update failed"),
    }
}

fn owned_pids(app: &AppHandle) -> BTreeSet<u32> {
    let mut pids = BTreeSet::from([std::process::id()]);
    if let Some(helper_pid) = crate::dictation::dictation_helper_pid(app) {
        pids.insert(helper_pid);
    }
    pids
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MeetingDetectionPayload {
    active_process_count: usize,
    /// Friendly names of the apps holding the microphone ("Zoom", "Chrome"),
    /// deduped in detection order. The HUD shows these under the prompt title.
    app_labels: Vec<String>,
}

#[derive(Debug, Serialize)]
struct MeetingDetectionEnvelope {
    #[serde(rename = "type")]
    event_type: &'static str,
    payload: MeetingDetectionPayload,
}

pub(crate) fn deduped_app_labels(processes: &[MicrophoneInputProcess]) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for process in processes {
        if !labels.contains(&process.app_label) {
            labels.push(process.app_label.clone());
        }
    }
    labels
}

fn emit_detection_event(
    app: &AppHandle,
    event: MeetingDetectionEvent,
    allowed_processes: &[MicrophoneInputProcess],
) {
    let event_type = match event {
        MeetingDetectionEvent::Detected => {
            // Wake the (possibly suspended) HUD webview without revealing
            // the window — it shows itself once the prompt is sized.
            crate::dictation::wake_hud_window(app);
            "meeting_detected"
        }
        // Heartbeats must NOT re-show the native window: after the prompt
        // auto-suppresses, the webview renders nothing, and a re-shown window
        // is just the bare vibrancy frost — a stuck gray bar the user can't
        // drag or dismiss. The HUD shows itself when it decides to render.
        MeetingDetectionEvent::Heartbeat => "meeting_detected",
        MeetingDetectionEvent::Cleared => "meeting_cleared",
    };
    let payload = MeetingDetectionEnvelope {
        event_type,
        payload: MeetingDetectionPayload {
            active_process_count: allowed_processes.len(),
            app_labels: deduped_app_labels(allowed_processes),
        },
    };
    match serde_json::to_string(&payload) {
        Ok(payload) => {
            let _ = app.emit(MEETING_DETECTION_EVENT_NAME, payload);
        }
        Err(error) => {
            tracing::warn!(%error, "failed to encode meeting detection event");
        }
    }
}

fn emit_meeting_end_state(app: &AppHandle, status: Option<MeetingEndStatus>) {
    let _ = app.emit(MEETING_END_STATE_EVENT_NAME, status);
}

fn emit_meeting_end_finish_request(app: &AppHandle, request: &PendingMeetingEndFinishRequest) {
    let _ = app.emit(MEETING_END_FINISH_REQUEST_EVENT_NAME, request);
}

#[cfg(target_os = "macos")]
pub(crate) use macos::active_input_processes;

#[cfg(target_os = "windows")]
pub(crate) use windows_probe::active_input_processes;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) fn active_input_processes() -> Result<Vec<MicrophoneInputProcess>, ProbeError> {
    Ok(Vec::new())
}

#[derive(Debug)]
pub(crate) struct ProbeError {
    operation: &'static str,
    status: i32,
}

impl ProbeError {
    fn new(operation: &'static str, status: i32) -> Self {
        Self { operation, status }
    }
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if cfg!(target_os = "windows") {
            write!(
                formatter,
                "{} failed with HRESULT {:#010x}",
                self.operation, self.status
            )
        } else {
            write!(
                formatter,
                "{} failed with OSStatus {}",
                self.operation, self.status
            )
        }
    }
}

impl std::error::Error for ProbeError {}

#[cfg(target_os = "macos")]
mod macos {
    use super::{MicrophoneInputProcess, ProbeError};
    use std::{ffi::c_void, mem, ptr};

    type AudioObjectId = u32;
    type AudioObjectPropertySelector = u32;
    type AudioObjectPropertyScope = u32;
    type AudioObjectPropertyElement = u32;
    type OsStatus = i32;

    const AUDIO_OBJECT_SYSTEM_OBJECT: AudioObjectId = 1;
    const AUDIO_OBJECT_UNKNOWN: AudioObjectId = 0;
    const AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL: AudioObjectPropertyScope = four_cc(*b"glob");
    const AUDIO_OBJECT_PROPERTY_SCOPE_INPUT: AudioObjectPropertyScope = four_cc(*b"inpt");
    const AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN: AudioObjectPropertyElement = 0;
    const AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST: AudioObjectPropertySelector =
        four_cc(*b"prs#");
    const AUDIO_PROCESS_PROPERTY_PID: AudioObjectPropertySelector = four_cc(*b"ppid");
    const AUDIO_PROCESS_PROPERTY_BUNDLE_ID: AudioObjectPropertySelector = four_cc(*b"pbid");
    const AUDIO_PROCESS_PROPERTY_DEVICES: AudioObjectPropertySelector = four_cc(*b"pdv#");
    const AUDIO_PROCESS_PROPERTY_IS_RUNNING_INPUT: AudioObjectPropertySelector = four_cc(*b"piri");

    #[repr(C)]
    struct AudioObjectPropertyAddress {
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope,
        element: AudioObjectPropertyElement,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    extern "C" {
        fn AudioObjectGetPropertyDataSize(
            object_id: AudioObjectId,
            address: *const AudioObjectPropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
        ) -> OsStatus;

        fn AudioObjectGetPropertyData(
            object_id: AudioObjectId,
            address: *const AudioObjectPropertyAddress,
            qualifier_data_size: u32,
            qualifier_data: *const c_void,
            data_size: *mut u32,
            data: *mut c_void,
        ) -> OsStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFStringGetCString(
            string: *const c_void,
            buffer: *mut i8,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;

        fn CFRelease(cf: *const c_void);
    }

    pub(crate) fn active_input_processes() -> Result<Vec<MicrophoneInputProcess>, ProbeError> {
        let mut processes = Vec::new();
        for process_object in process_objects()? {
            if process_object == AUDIO_OBJECT_UNKNOWN {
                continue;
            }
            let running_input = read_u32_property(
                process_object,
                AUDIO_PROCESS_PROPERTY_IS_RUNNING_INPUT,
                "read process input state",
            )
            .unwrap_or_default();
            if running_input == 0 {
                continue;
            }
            if !process_has_input_devices(process_object) {
                continue;
            }
            if let (Ok(Some(pid)), Ok(Some(bundle_id))) = (
                read_process_pid(process_object),
                read_process_bundle_id(process_object),
            ) {
                if let Some(process) = MicrophoneInputProcess::new(pid, bundle_id) {
                    processes.push(process);
                }
            }
        }
        processes.sort_by_key(|process| process.pid);
        processes.dedup_by_key(|process| process.pid);
        Ok(processes)
    }

    fn process_objects() -> Result<Vec<AudioObjectId>, ProbeError> {
        read_object_array_property(
            AUDIO_OBJECT_SYSTEM_OBJECT,
            AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST,
            AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL,
            "read process object list size",
            "read process object list",
        )
    }

    fn read_object_array_property(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope,
        size_operation: &'static str,
        data_operation: &'static str,
    ) -> Result<Vec<AudioObjectId>, ProbeError> {
        let address = property_address_with_scope(selector, scope);
        let mut data_size = 0_u32;
        status_result(size_operation, unsafe {
            AudioObjectGetPropertyDataSize(object_id, &address, 0, ptr::null(), &mut data_size)
        })?;

        if data_size == 0 {
            return Ok(Vec::new());
        }

        let object_count = data_size as usize / mem::size_of::<AudioObjectId>();
        let mut objects = vec![AUDIO_OBJECT_UNKNOWN; object_count];
        status_result(data_operation, unsafe {
            AudioObjectGetPropertyData(
                object_id,
                &address,
                0,
                ptr::null(),
                &mut data_size,
                objects.as_mut_ptr().cast(),
            )
        })?;

        let actual_count = data_size as usize / mem::size_of::<AudioObjectId>();
        objects.truncate(actual_count);
        Ok(objects)
    }

    fn process_devices(
        process_object: AudioObjectId,
        scope: AudioObjectPropertyScope,
    ) -> Result<Vec<AudioObjectId>, ProbeError> {
        read_object_array_property(
            process_object,
            AUDIO_PROCESS_PROPERTY_DEVICES,
            scope,
            "read process device list size",
            "read process device list",
        )
    }

    fn read_process_pid(process_object: AudioObjectId) -> Result<Option<u32>, ProbeError> {
        let pid = read_i32_property(
            process_object,
            AUDIO_PROCESS_PROPERTY_PID,
            "read process pid",
        )?;
        if pid <= 0 {
            Ok(None)
        } else {
            Ok(Some(pid as u32))
        }
    }

    fn process_has_input_devices(process_object: AudioObjectId) -> bool {
        process_devices(process_object, AUDIO_OBJECT_PROPERTY_SCOPE_INPUT)
            .map(|devices| !devices.is_empty())
            .unwrap_or(false)
    }

    fn read_process_bundle_id(process_object: AudioObjectId) -> Result<Option<String>, ProbeError> {
        let mut value: *const c_void = ptr::null();
        read_scalar_property(
            process_object,
            AUDIO_PROCESS_PROPERTY_BUNDLE_ID,
            "read process bundle id",
            &mut value,
        )?;
        if value.is_null() {
            return Ok(None);
        }

        let mut buffer = vec![0_i8; 512];
        let ok = unsafe {
            CFStringGetCString(
                value,
                buffer.as_mut_ptr(),
                buffer.len() as isize,
                0x0800_0100,
            )
        };
        unsafe {
            CFRelease(value);
        }
        if ok == 0 {
            return Ok(None);
        }
        let value = unsafe { std::ffi::CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .trim()
            .to_string();
        Ok((!value.is_empty()).then_some(value))
    }

    fn read_i32_property(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        operation: &'static str,
    ) -> Result<i32, ProbeError> {
        let mut value = 0_i32;
        read_scalar_property(object_id, selector, operation, &mut value)?;
        Ok(value)
    }

    fn read_u32_property(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        operation: &'static str,
    ) -> Result<u32, ProbeError> {
        let mut value = 0_u32;
        read_scalar_property(object_id, selector, operation, &mut value)?;
        Ok(value)
    }

    fn read_scalar_property<T>(
        object_id: AudioObjectId,
        selector: AudioObjectPropertySelector,
        operation: &'static str,
        value: &mut T,
    ) -> Result<(), ProbeError> {
        let address = property_address(selector);
        let mut data_size = mem::size_of::<T>() as u32;
        status_result(operation, unsafe {
            AudioObjectGetPropertyData(
                object_id,
                &address,
                0,
                ptr::null(),
                &mut data_size,
                (value as *mut T).cast(),
            )
        })
    }

    fn property_address(selector: AudioObjectPropertySelector) -> AudioObjectPropertyAddress {
        property_address_with_scope(selector, AUDIO_OBJECT_PROPERTY_SCOPE_GLOBAL)
    }

    fn property_address_with_scope(
        selector: AudioObjectPropertySelector,
        scope: AudioObjectPropertyScope,
    ) -> AudioObjectPropertyAddress {
        AudioObjectPropertyAddress {
            selector,
            scope,
            element: AUDIO_OBJECT_PROPERTY_ELEMENT_MAIN,
        }
    }

    fn status_result(operation: &'static str, status: OsStatus) -> Result<(), ProbeError> {
        if status == 0 {
            Ok(())
        } else {
            Err(ProbeError::new(operation, status))
        }
    }

    const fn four_cc(value: [u8; 4]) -> u32 {
        ((value[0] as u32) << 24)
            | ((value[1] as u32) << 16)
            | ((value[2] as u32) << 8)
            | value[3] as u32
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn four_cc_matches_core_audio_constants() {
            assert_eq!(AUDIO_HARDWARE_PROPERTY_PROCESS_OBJECT_LIST, 0x7072_7323);
            assert_eq!(AUDIO_PROCESS_PROPERTY_PID, 0x7070_6964);
            assert_eq!(AUDIO_PROCESS_PROPERTY_BUNDLE_ID, 0x7062_6964);
            assert_eq!(AUDIO_PROCESS_PROPERTY_DEVICES, 0x7064_7623);
            assert_eq!(AUDIO_PROCESS_PROPERTY_IS_RUNNING_INPUT, 0x7069_7269);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start_request(request_id: &str) -> StartMeetingRecordingRequest {
        StartMeetingRecordingRequest {
            request_id: request_id.to_owned(),
            source_mode: None,
            video: None,
        }
    }

    #[tokio::test]
    async fn native_start_caches_preparation_errors_before_audio_and_allows_ack() {
        for boundary in ["bundle", "video"] {
            let state = MeetingStartRequestState::default();
            let request_id = state.queue_at(1_000).unwrap();
            state.begin_start(&request_id, 1_100).unwrap();
            let audio_started = std::cell::Cell::new(false);
            let outcome = crate::commands::finish_meeting_start(&state, &request_id, async {
                if boundary == "bundle" {
                    state.start_bundle_families("stale-request")?;
                } else {
                    state.start_video(&start_request("stale-request"))?;
                }
                audio_started.set(true);
                unreachable!("preparation failure must not start audio")
            })
            .await
            .unwrap();
            assert!(!audio_started.get());
            let MeetingStartRecordingOutcome::Failed { error } = outcome else {
                panic!("expected a terminal preparation failure");
            };
            assert_eq!(error.code, "meeting_start_not_found");
            for _ in 0..2 {
                let Some(MeetingStartRecordingOutcome::Failed { error }) =
                    state.finished_outcome(&request_id).unwrap()
                else {
                    panic!("expected a replayable failure");
                };
                assert_eq!(error.code, "meeting_start_not_found");
            }
            assert_eq!(
                state.begin_start(&request_id, 1_200).unwrap_err().code,
                "meeting_start_already_finished"
            );
            assert!(state.acknowledge(&request_id).unwrap());
            assert!(state.peek_at(1_200).unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn cancelled_native_preparation_caches_interruption() {
        let state = MeetingStartRequestState::default();
        let request_id = state.queue_at(1_000).unwrap();
        state.begin_start(&request_id, 1_100).unwrap();
        let operation =
            crate::commands::finish_meeting_start(&state, &request_id, std::future::pending());
        assert!(tokio::time::timeout(Duration::from_millis(10), operation)
            .await
            .is_err());
        let Some(MeetingStartRecordingOutcome::Failed { error }) =
            state.finished_outcome(&request_id).unwrap()
        else {
            panic!("expected a cached cancellation");
        };
        assert_eq!(error.code, "meeting_start_interrupted");
        assert!(state.acknowledge(&request_id).unwrap());
    }

    #[test]
    fn meeting_video_native_choice_rejects_renderer_retarget_and_unrequested_opt_in() {
        for retained in [Some(RecordingVideoRequest { window_id: 42 }), None] {
            let state = MeetingStartRequestState::default();
            let id = state.queue_with_video_at(1000, retained.clone()).unwrap();
            let rendered = crate::domain::types::StartMeetingRecordingRequest {
                request_id: id,
                source_mode: None,
                video: Some(RecordingVideoRequest { window_id: 99 }),
            };
            assert_eq!(state.start_video(&rendered).unwrap(), retained);
        }
    }

    #[test]
    fn meeting_video_request_is_retained_and_forwarded_without_retargeting() {
        let state = MeetingStartRequestState::default();
        let video = crate::domain::types::RecordingVideoRequest { window_id: 7331 };
        let id = state.queue_with_video_at(1000, Some(video)).unwrap();
        let pending = state.peek_at(1001).unwrap().unwrap();
        assert_eq!(pending.video.as_ref().map(|v| v.window_id), Some(7331));
        assert_eq!(
            serde_json::to_value(&pending).unwrap()["video"]["windowId"],
            7331
        );
        state.begin_start(&id, 1001).unwrap();
        assert_eq!(
            state
                .queue_with_video_at(
                    1002,
                    Some(crate::domain::types::RecordingVideoRequest { window_id: 42 })
                )
                .unwrap(),
            id
        );
        assert_eq!(
            state
                .start_video(&start_request(&id))
                .unwrap()
                .map(|v| v.window_id),
            Some(7331)
        );
    }

    #[test]
    fn meeting_video_opt_out_and_acknowledgement_do_not_reuse_an_old_window() {
        let state = MeetingStartRequestState::default();
        let id = state
            .queue_with_video_at(
                1000,
                Some(crate::domain::types::RecordingVideoRequest { window_id: 7331 }),
            )
            .unwrap();
        assert!(state.acknowledge(&id).unwrap());
        let next = state.queue_with_video_at(2000, None).unwrap();
        assert!(state.peek_at(2001).unwrap().unwrap().video.is_none());
        assert!(state.start_video(&start_request(&next)).unwrap().is_none());
        assert!(state.start_video(&start_request(&id)).is_err());
    }

    #[test]
    fn pending_meeting_start_request_is_retained_until_matching_ack() {
        let state = MeetingStartRequestState::default();
        let request_id = state.queue_at(1_000).expect("queue request");
        let note_id = state
            .peek_at(1_100)
            .expect("peek queued request")
            .expect("queued request")
            .note_id;
        let expected = Some(PendingMeetingStartRequest {
            request_id: request_id.clone(),
            note_id,
            requested_at_ms: 1_000,
            expired: false,
            video: None,
        });

        assert_eq!(state.peek_at(1_100).expect("peek request"), expected);
        assert_eq!(state.peek_at(1_100).expect("peek again"), expected);
        assert!(!state.acknowledge("stale").expect("reject stale ack"));
        assert_eq!(state.peek_at(1_100).expect("peek retained"), expected);
        assert!(state.acknowledge(&request_id).expect("ack request"));
        assert_eq!(state.peek_at(1_100).expect("peek empty request"), None);
    }

    #[test]
    fn newer_queued_action_replaces_the_request_generation() {
        let state = MeetingStartRequestState::default();
        let first_request_id = state.queue_at(1_000).expect("queue first request");
        let second_request_id = state.queue_at(2_000).expect("queue second request");
        let note_id = state
            .peek_at(2_100)
            .expect("peek queued request")
            .expect("queued request")
            .note_id;

        assert_ne!(second_request_id, first_request_id);
        assert_eq!(
            state.peek_at(2_100).expect("peek request"),
            Some(PendingMeetingStartRequest {
                request_id: second_request_id,
                note_id,
                requested_at_ms: 2_000,
                expired: false,
                video: None,
            })
        );
    }

    #[test]
    fn pending_meeting_start_request_expires_on_wall_clock_time() {
        let state = MeetingStartRequestState::default();
        let request_id = state.queue_at(1_000).expect("queue request");
        let note_id = state
            .peek_at(1_000)
            .expect("peek queued request")
            .expect("queued request")
            .note_id;

        assert_eq!(
            state
                .peek_at(1_000 + MEETING_START_REQUEST_TTL_MS + 1)
                .expect("peek stale request"),
            Some(PendingMeetingStartRequest {
                request_id: request_id.clone(),
                note_id,
                requested_at_ms: 1_000,
                expired: true,
                video: None,
            })
        );
        assert_eq!(
            state
                .peek_at(1_000 + MEETING_START_REQUEST_TTL_MS + 1)
                .expect("peek retained stale request")
                .as_ref()
                .map(|request| request.request_id.as_str()),
            Some(request_id.as_str())
        );
    }

    #[test]
    fn finished_meeting_start_outcome_is_replayed_without_restarting() {
        let state = MeetingStartRequestState::default();
        let request_id = state.queue_at(1_000).expect("queue request");
        let note_id = state
            .begin_start(&request_id, 1_100)
            .expect("begin request");
        assert!(!note_id.is_empty());
        assert_eq!(
            state
                .begin_start(&request_id, 1_100)
                .expect_err("starting request cannot begin twice")
                .code,
            "meeting_start_in_progress"
        );

        state
            .finish_start(
                &request_id,
                MeetingStartRecordingOutcome::Failed {
                    error: AppError::new("source_not_ready", "Microphone is not ready."),
                },
            )
            .expect("finish request");
        for _ in 0..2 {
            let Some(MeetingStartRecordingOutcome::Failed { error }) =
                state.finished_outcome(&request_id).expect("read outcome")
            else {
                panic!("expected cached failure");
            };
            assert_eq!(error.code, "source_not_ready");
        }
    }

    #[test]
    fn stale_ack_cannot_clear_a_newer_request() {
        let state = MeetingStartRequestState::default();
        let first_request_id = state.queue_at(1_000).expect("queue first request");
        let second_request_id = state.queue_at(2_000).expect("queue second request");

        assert_ne!(second_request_id, first_request_id);
        assert!(!state
            .acknowledge(&first_request_id)
            .expect("reject stale ack"));
        assert_eq!(
            state
                .peek_at(2_100)
                .expect("peek second request")
                .map(|request| request.request_id),
            Some(second_request_id)
        );
    }

    #[test]
    fn acknowledgement_cannot_clear_a_request_while_native_start_is_running() {
        let state = MeetingStartRequestState::default();
        let request_id = state.queue_at(1_000).expect("queue request");
        state
            .begin_start(&request_id, 1_100)
            .expect("begin request");

        assert!(!state
            .acknowledge(&request_id)
            .expect("reject in-progress acknowledgement"));
        assert_eq!(
            state
                .peek_at(1_200)
                .expect("peek in-progress request")
                .map(|request| request.request_id),
            Some(request_id)
        );
    }

    #[test]
    fn interrupted_native_start_becomes_a_replayable_failure() {
        let state = MeetingStartRequestState::default();
        let request_id = state.queue_at(1_000).expect("queue request");
        state
            .begin_start(&request_id, 1_100)
            .expect("begin request");

        assert!(state
            .fail_start_if_running(&request_id)
            .expect("interrupt request"));
        let Some(MeetingStartRecordingOutcome::Failed { error }) = state
            .finished_outcome(&request_id)
            .expect("read interrupted outcome")
        else {
            panic!("expected replayable interrupted outcome");
        };
        assert_eq!(error.code, "meeting_start_interrupted");
        assert!(!state
            .fail_start_if_running(&request_id)
            .expect("finished request is unchanged"));
    }

    #[test]
    fn starting_action_coalesces_but_finished_action_can_be_replaced() {
        let state = MeetingStartRequestState::default();
        let request_id = state.queue_at(1_000).expect("queue request");
        state
            .begin_start(&request_id, 1_100)
            .expect("begin request");

        assert_eq!(
            state.queue_at(1_200).expect("coalesce starting request"),
            request_id
        );
        state
            .finish_start(
                &request_id,
                MeetingStartRecordingOutcome::Failed {
                    error: AppError::new("source_not_ready", "Microphone is not ready."),
                },
            )
            .expect("finish request");

        assert_ne!(
            state.queue_at(2_000).expect("replace finished request"),
            request_id
        );
    }

    fn input_process(pid: u32, bundle_id: &str) -> MicrophoneInputProcess {
        MicrophoneInputProcess::new(pid, bundle_id.to_string()).expect("valid process")
    }

    fn allowed_pids(processes: &[MicrophoneInputProcess]) -> Vec<u32> {
        active_allowed_external_processes(processes, &BTreeSet::new())
            .into_iter()
            .map(|process| process.pid)
            .collect()
    }

    fn families(values: &[&str]) -> BTreeSet<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn meeting_end_tracker(tracked: &[&str]) -> MeetingEndTracker {
        MeetingEndTracker::new("meeting-session".to_string(), families(tracked))
    }

    fn advance_absence_to_countdown(
        tracker: &mut MeetingEndTracker,
        start_ms: u64,
    ) -> MeetingEndStatus {
        let active = tracker.tracked_families.clone();
        assert_eq!(
            tracker.observe(Some(&active), start_ms.saturating_sub(1_000)),
            None
        );
        let empty = BTreeSet::new();
        let mut transition = None;
        for poll in 0..MEETING_END_ABSENT_POLLS {
            transition = tracker.observe(
                Some(&empty),
                start_ms.saturating_add(u64::from(poll) * 1_000),
            );
        }
        let Some(MeetingEndTransition::StateChanged(status)) = transition else {
            panic!("expected countdown transition");
        };
        status
    }

    #[test]
    fn initial_absence_samples_require_fifteen_elapsed_seconds() {
        let mut tracker = meeting_end_tracker(&["us.zoom.xos"]);
        let empty = BTreeSet::new();

        for poll in 0..MEETING_END_ABSENT_POLLS {
            assert_eq!(tracker.observe(Some(&empty), u64::from(poll) * 1_000), None);
        }
        assert_eq!(tracker.status().phase, MeetingEndPhase::Tracking);
        assert!(matches!(
            tracker.observe(Some(&empty), MEETING_END_ABSENCE_MS),
            Some(MeetingEndTransition::StateChanged(MeetingEndStatus {
                phase: MeetingEndPhase::Countdown,
                ..
            }))
        ));
    }

    #[test]
    fn meeting_start_snapshots_stable_bundle_families() {
        let state = MeetingStartRequestState::default();
        state
            .set_detected_bundle_families(families(&["us.zoom.xos", "com.google.chrome"]))
            .expect("store families");
        let request_id = state.queue_at(1_000).expect("queue meeting start");

        assert_eq!(
            state
                .start_bundle_families(&request_id)
                .expect("read snapshotted families"),
            vec!["com.google.chrome".to_string(), "us.zoom.xos".to_string()]
        );
    }

    #[test]
    fn long_audio_silence_does_not_affect_meeting_end_tracking() {
        let mut tracker = meeting_end_tracker(&["us.zoom.xos"]);
        let active = families(&["us.zoom.xos"]);

        for poll in 0..1_800 {
            assert_eq!(
                tracker.observe(Some(&active), poll * 1_000),
                None,
                "audio levels are not an input to meeting end detection"
            );
        }
        assert_eq!(tracker.status().phase, MeetingEndPhase::Tracking);
    }

    #[test]
    fn brief_microphone_release_does_not_start_countdown() {
        let mut tracker = meeting_end_tracker(&["us.zoom.xos"]);
        let empty = BTreeSet::new();
        let active = families(&["us.zoom.xos"]);

        for poll in 0..(MEETING_END_ABSENT_POLLS - 1) {
            assert_eq!(tracker.observe(Some(&empty), u64::from(poll) * 1_000), None);
        }
        assert_eq!(tracker.observe(Some(&active), 14_000), None);
        assert_eq!(tracker.status().phase, MeetingEndPhase::Tracking);
    }

    #[test]
    fn sustained_release_starts_countdown_then_queues_one_finish() {
        let mut tracker = meeting_end_tracker(&["us.zoom.xos"]);
        let empty = BTreeSet::new();
        let countdown = advance_absence_to_countdown(&mut tracker, 1_000);
        assert_eq!(countdown.phase, MeetingEndPhase::Countdown);
        assert_eq!(countdown.expires_at_ms, Some(30_000));

        let mut terminal = None;
        for poll in 1..=MEETING_END_COUNTDOWN_POLLS {
            terminal = tracker.observe(Some(&empty), 15_000 + u64::from(poll) * 1_000);
        }
        let Some(MeetingEndTransition::FinishQueued(status, request)) = terminal else {
            panic!("expected queued finish");
        };
        assert_eq!(status.phase, MeetingEndPhase::FinishQueued);
        assert_eq!(request.session_id, "meeting-session");

        for poll in 1..=30 {
            assert_eq!(tracker.observe(Some(&empty), 30_000 + poll * 1_000), None);
        }
        assert_eq!(tracker.pending_finish.as_ref(), Some(&request));
        assert_eq!(tracker.queue_finish().expect("idempotent queue"), request);
    }

    #[test]
    fn reacquiring_microphone_during_countdown_cancels_finish() {
        let mut tracker = meeting_end_tracker(&["us.zoom.xos"]);
        let countdown = advance_absence_to_countdown(&mut tracker, 1_000);
        assert_eq!(countdown.phase, MeetingEndPhase::Countdown);

        let active = families(&["us.zoom.xos"]);
        let transition = tracker.observe(Some(&active), 16_000);
        assert_eq!(
            transition,
            Some(MeetingEndTransition::StateChanged(MeetingEndStatus {
                session_id: "meeting-session".to_string(),
                phase: MeetingEndPhase::Tracking,
                expires_at_ms: None,
            }))
        );
        assert!(tracker.pending_finish.is_none());
    }

    #[test]
    fn every_originating_app_must_release_before_countdown() {
        let mut tracker = meeting_end_tracker(&["us.zoom.xos", "com.google.chrome"]);
        let chrome_only = families(&["com.google.chrome"]);

        for poll in 0..60 {
            assert_eq!(
                tracker.observe(Some(&chrome_only), poll * 1_000),
                None,
                "one tracked app still owns the microphone"
            );
        }
        assert_eq!(tracker.status().phase, MeetingEndPhase::Tracking);

        let countdown = advance_absence_to_countdown(&mut tracker, 60_000);
        assert_eq!(countdown.phase, MeetingEndPhase::Countdown);
    }

    #[test]
    fn probe_error_or_sleep_gap_cancels_countdown() {
        let mut tracker = meeting_end_tracker(&["us.zoom.xos"]);
        advance_absence_to_countdown(&mut tracker, 1_000);

        let probe_error = tracker.observe(None, 16_000);
        assert!(matches!(
            probe_error,
            Some(MeetingEndTransition::StateChanged(MeetingEndStatus {
                phase: MeetingEndPhase::Tracking,
                ..
            }))
        ));

        advance_absence_to_countdown(&mut tracker, 20_000);
        let empty = BTreeSet::new();
        let wake = tracker.observe(Some(&empty), 60_000);
        assert!(matches!(
            wake,
            Some(MeetingEndTransition::StateChanged(MeetingEndStatus {
                phase: MeetingEndPhase::Tracking,
                ..
            }))
        ));
    }

    #[test]
    fn only_eligible_meeting_recordings_arm_and_keep_suppresses_future_prompts() {
        let state = MeetingStartRequestState::default();
        assert!(state
            .arm_meeting_end(
                "manual-session".to_string(),
                &RecordingOriginMetadata::default(),
            )
            .expect("manual recording remains ineligible")
            .is_none());
        assert!(state
            .arm_meeting_end(
                "agent-session".to_string(),
                &RecordingOriginMetadata {
                    origin: RecordingOrigin::Other,
                    meeting_app_bundle_families: vec!["us.zoom.xos".to_string()],
                    auto_finish_eligible: true,
                },
            )
            .expect("non-meeting recording remains ineligible")
            .is_none());
        assert!(state
            .meeting_end_status()
            .expect("read manual state")
            .is_none());

        let origin = RecordingOriginMetadata {
            origin: RecordingOrigin::MeetingPrompt,
            meeting_app_bundle_families: vec!["us.zoom.xos".to_string()],
            auto_finish_eligible: true,
        };
        let status = state
            .arm_meeting_end("meeting-session".to_string(), &origin)
            .expect("arm meeting recording")
            .expect("eligible status");
        assert_eq!(status.phase, MeetingEndPhase::Tracking);

        {
            let mut guard = state.meeting_end.lock().expect("meeting end tracker");
            let tracker = guard.as_mut().expect("armed tracker");
            advance_absence_to_countdown(tracker, 1_000);
        }
        let suppressed = state
            .keep_meeting_recording("meeting-session")
            .expect("keep recording");
        assert_eq!(suppressed.phase, MeetingEndPhase::Suppressed);
        let empty = BTreeSet::new();
        for poll in 0..60 {
            assert_eq!(
                state
                    .observe_meeting_end(Some(&empty), 20_000 + poll * 1_000)
                    .expect("observe suppressed recording"),
                None
            );
        }
    }

    #[test]
    fn forced_countdown_runs_the_real_meeting_end_machinery() {
        let state = MeetingStartRequestState::default();
        let status = state
            .force_meeting_end_countdown("meeting-session".to_string(), 10_000)
            .expect("force countdown");
        assert_eq!(status.phase, MeetingEndPhase::Countdown);
        assert_eq!(
            status.expires_at_ms,
            Some(10_000 + MEETING_END_COUNTDOWN_MS)
        );

        // Keep recording suppresses it exactly like a detector-armed countdown.
        let suppressed = state
            .keep_meeting_recording("meeting-session")
            .expect("keep recording");
        assert_eq!(suppressed.phase, MeetingEndPhase::Suppressed);

        // Re-forced, the ordinary poll drives it to a real queued auto-finish
        // at expiry — the debug hook changes how the countdown starts, not
        // what it does.
        state
            .force_meeting_end_countdown("meeting-session".to_string(), 10_000)
            .expect("force countdown again");
        let empty = BTreeSet::new();
        let mut transition = None;
        for poll in 0..60u64 {
            transition = state
                .observe_meeting_end(Some(&empty), 10_000 + poll * 1_000)
                .expect("observe forced countdown");
            if transition.is_some() {
                break;
            }
        }
        let Some(MeetingEndTransition::FinishQueued(status, request)) = transition else {
            panic!("expected queued finish");
        };
        assert_eq!(status.phase, MeetingEndPhase::FinishQueued);
        assert_eq!(request.session_id, "meeting-session");
    }

    #[test]
    fn arm_between_monitor_snapshot_and_cleanup_survives_live_status_reread() {
        let state = MeetingStartRequestState::default();
        let start_of_iteration_session: Option<String> = None;
        let origin = RecordingOriginMetadata {
            origin: RecordingOrigin::MeetingPrompt,
            meeting_app_bundle_families: vec!["us.zoom.xos".to_string()],
            auto_finish_eligible: true,
        };

        assert_eq!(start_of_iteration_session, None);
        state
            .arm_meeting_end("meeting-session".to_string(), &origin)
            .expect("arm after stale monitor snapshot")
            .expect("eligible meeting recording");

        assert!(!state
            .clear_meeting_end_if_inactive(|| Some("meeting-session".to_string()))
            .expect("reconcile against live capture status"));
        assert_eq!(
            state
                .meeting_end_status()
                .expect("read tracker after reconciliation")
                .expect("freshly armed tracker remains")
                .session_id,
            "meeting-session"
        );
    }

    #[test]
    fn unknown_external_microphone_process_makes_meeting_end_probe_ambiguous() {
        let owned = BTreeSet::from([10]);
        let processes = vec![
            input_process(10, "com.june.owned-helper"),
            input_process(20, "com.unknown.recorder"),
        ];

        assert_eq!(meeting_end_bundle_families(&processes, &owned), None);
        assert_eq!(
            meeting_end_bundle_families(&[input_process(30, "us.zoom.xos")], &owned),
            Some(families(&["us.zoom.xos"]))
        );
    }

    #[test]
    fn deduped_app_labels_collapses_helper_processes_in_detection_order() {
        let processes = vec![
            input_process(30, "us.zoom.xos"),
            input_process(31, "us.zoom.xos.helper"),
            input_process(32, "com.google.Chrome"),
        ];

        assert_eq!(deduped_app_labels(&processes), vec!["Zoom", "Chrome"]);
        assert!(deduped_app_labels(&[]).is_empty());
    }

    #[test]
    fn active_allowed_external_processes_excludes_owned_processes() {
        let owned = BTreeSet::from([10, 20]);
        let processes = vec![
            MicrophoneInputProcess {
                pid: 0,
                bundle_id: "com.google.Chrome".to_string(),
                app_label: "Chrome".to_string(),
            },
            input_process(10, "com.google.Chrome"),
            input_process(30, "com.google.Chrome"),
            input_process(20, "company.thebrowser.Browser"),
            input_process(40, "company.thebrowser.Browser"),
        ];

        assert_eq!(
            active_allowed_external_processes(&processes, &owned)
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![30, 40]
        );
    }

    #[test]
    fn supported_mic_processes_trigger_detection_filter() {
        let cases = [
            ("ai.perplexity.comet", "ai.perplexity.comet.helper", "Comet"),
            (
                "ai.perplexity.comet.ios",
                "ai.perplexity.comet.ios.helper",
                "Comet",
            ),
            ("Cisco-Systems.Spark", "Cisco-Systems.Spark.helper", "Webex"),
            (
                "com.apple.FaceTime",
                "com.apple.FaceTime.helper",
                "FaceTime",
            ),
            ("com.apple.Safari", "com.apple.Safari.WebContent", "Safari"),
            ("com.brave.Browser", "com.brave.Browser.beta", "Brave"),
            (
                "com.cisco.webexmeetingsapp",
                "com.cisco.webexmeetingsapp.helper",
                "Webex",
            ),
            (
                "com.gather.GatherV2",
                "com.gather.GatherV2.helper",
                "Gather",
            ),
            ("com.google.Chrome", "COM.GOOGLE.CHROME.helper", "Chrome"),
            ("com.hnc.Discord", "com.hnc.Discord.helper", "Discord"),
            (
                "com.microsoft.edgemac",
                "com.microsoft.edgemac.Beta",
                "Edge",
            ),
            ("com.microsoft.teams", "com.microsoft.teams.helper", "Teams"),
            (
                "com.microsoft.teams2",
                "com.microsoft.teams2.helper",
                "Teams",
            ),
            (
                "com.operasoftware.Opera",
                "com.operasoftware.Opera.helper",
                "Opera",
            ),
            (
                "com.tinyspeck.slackmacgap",
                "com.tinyspeck.slackmacgap.helper",
                "Slack",
            ),
            (
                "com.vivaldi.Vivaldi",
                "com.vivaldi.Vivaldi.snapshot",
                "Vivaldi",
            ),
            (
                "company.thebrowser.Browser",
                "company.thebrowser.Browser.helper",
                "Arc",
            ),
            (
                "company.thebrowser.dia",
                "company.thebrowser.dia.helper",
                "Dia",
            ),
            (
                "net.whatsapp.WhatsApp",
                "net.whatsapp.WhatsApp.helper",
                "WhatsApp",
            ),
            (
                "org.mozilla.firefox",
                "org.mozilla.firefox.helper",
                "Firefox",
            ),
            (
                "org.mozilla.firefoxdeveloperedition",
                "org.mozilla.firefoxdeveloperedition.helper",
                "Firefox",
            ),
            (
                "org.mozilla.nightly",
                "org.mozilla.nightly.helper",
                "Firefox",
            ),
            (
                "org.telegram.desktop",
                "org.telegram.desktop.helper",
                "Telegram",
            ),
            (
                "org.whispersystems.signal-desktop",
                "org.whispersystems.signal-desktop.helper",
                "Signal",
            ),
            (
                "ru.keepcoder.Telegram",
                "ru.keepcoder.Telegram.helper",
                "Telegram",
            ),
            ("us.zoom.xos", "us.zoom.xos.helper", "Zoom"),
        ];

        for (index, (exact_bundle_id, helper_bundle_id, label)) in cases.into_iter().enumerate() {
            let exact_pid = 100 + (index as u32 * 2);
            let helper_pid = exact_pid + 1;
            let exact = input_process(exact_pid, exact_bundle_id);
            let helper = input_process(helper_pid, helper_bundle_id);

            assert_eq!(exact.app_label, label, "{exact_bundle_id}");
            assert_eq!(helper.app_label, label, "{helper_bundle_id}");
            assert_eq!(
                allowed_pids(&[exact, helper]),
                vec![exact_pid, helper_pid],
                "{exact_bundle_id}"
            );
        }
    }

    #[test]
    fn unlisted_mic_process_does_not_trigger_detection_filter() {
        assert!(allowed_pids(&[
            input_process(55, "com.apple.PhotoBooth"),
            input_process(56, "com.google.ChromeRemoteDesktop"),
            input_process(57, "com.apple.WebKit.WebContent"),
            input_process(58, "ai.perplexity.cometary"),
            input_process(59, "company.thebrowser.dialog"),
            input_process(60, "com.microsoft.teamsClassic"),
            input_process(61, "net.whatsapp.WhatsAppBusiness"),
            input_process(62, "org.mozilla.firefoxish"),
            input_process(63, "org.whispersystems.signal-desktopx"),
            input_process(64, "com.gather.GatherV20"),
        ])
        .is_empty());
    }

    #[test]
    fn windows_meeting_apps_have_readable_labels() {
        let cases = [
            ("Zoom.exe", "Zoom"),
            ("teams.exe", "Teams"),
            ("ms-teams.exe", "Teams"),
            ("msteams.exe", "Teams"),
            ("MSTeams_8wekyb3d8bbwe", "Teams"),
            ("slack.exe", "Slack"),
            ("Discord.exe", "Discord"),
            ("whatsapp.exe", "WhatsApp"),
            ("WhatsApp.Root.exe", "WhatsApp"),
            ("5319275A.WhatsAppDesktop_cv1g1gvanyjgm", "WhatsApp"),
            ("Telegram.exe", "Telegram"),
            ("Signal.exe", "Signal"),
            ("webex.exe", "Webex"),
            ("CiscoCollabHost.exe", "Webex"),
            ("chrome.exe", "Chrome"),
            ("msedge.exe", "Edge"),
            ("firefox.exe", "Firefox"),
            ("brave.exe", "Brave"),
            ("opera.exe", "Opera"),
            ("vivaldi.exe", "Vivaldi"),
            ("Arc.exe", "Arc"),
            ("comet.exe", "Comet"),
        ];

        for (index, (identity, label)) in cases.into_iter().enumerate() {
            let pid = 900 + index as u32;
            let process = input_process(pid, identity);
            assert_eq!(process.app_label, label, "{identity}");
            assert_eq!(allowed_pids(&[process]), vec![pid], "{identity}");
        }
    }

    #[test]
    fn every_activity_browser_executable_is_a_windows_meeting_app() {
        for executable in crate::activity::filter::BROWSER_EXECUTABLES {
            let process = input_process(1, executable);
            assert!(is_allowed_microphone_app(executable), "{executable}");
            assert_ne!(process.app_label, "exe", "{executable}");
        }
    }

    #[test]
    fn windows_teams_identities_share_one_family() {
        let family = Some("windows:teams".to_string());
        assert_eq!(bundle_family_from_bundle_id("Teams.exe"), family);
        assert_eq!(bundle_family_from_bundle_id("ms-teams.exe"), family);
        assert_eq!(
            bundle_family_from_bundle_id("MSTeams_8wekyb3d8bbwe"),
            family
        );
        assert_eq!(
            bundle_family_from_bundle_id("chrome.exe"),
            Some("windows:chrome".to_string())
        );
    }

    #[test]
    fn unlisted_windows_identities_keep_a_readable_label_and_stay_out() {
        assert_eq!(input_process(1, "obs64.exe").app_label, "obs64");
        assert!(allowed_pids(&[
            input_process(2, "obs64.exe"),
            input_process(3, "notchrome.exe"),
            input_process(4, "chrome.exe.bak"),
            input_process(5, "MSTeamsX_8wekyb3d8bbwe"),
            input_process(6, "msedgewebview2.exe"),
            input_process(7, "clovy.exe"),
        ])
        .is_empty());
    }

    #[test]
    fn windows_probe_drives_detection_and_meeting_end() {
        use windows_probe::{windows_input_processes, ProbeSnapshot, ProcessEntry};

        let processes = vec![
            ProcessEntry {
                pid: 200,
                parent_pid: 100,
                exe_name: "chrome.exe".to_string(),
            },
            ProcessEntry {
                pid: 210,
                parent_pid: 200,
                exe_name: "chrome.exe".to_string(),
            },
            ProcessEntry {
                pid: 400,
                parent_pid: 100,
                exe_name: "clovy.exe".to_string(),
            },
        ];
        let owned = BTreeSet::from([400]);
        let in_call = windows_input_processes(&ProbeSnapshot {
            active_capture_pids: vec![210, 400],
            processes: processes.clone(),
            consent: Vec::new(),
        });
        let after_call = windows_input_processes(&ProbeSnapshot {
            active_capture_pids: vec![400],
            processes,
            consent: Vec::new(),
        });

        let mut detection = MeetingDetectionState::default();
        let allowed = active_allowed_external_processes(&in_call, &owned);
        assert_eq!(deduped_app_labels(&allowed), vec!["Chrome"]);
        assert_eq!(
            detection.update(true, !allowed.is_empty(), false),
            Some(MeetingDetectionEvent::Detected)
        );

        let started_families =
            meeting_end_bundle_families(&in_call, &owned).expect("unambiguous probe");
        assert_eq!(started_families, families(&["windows:chrome"]));
        let mut tracker = MeetingEndTracker::new("meeting-session".to_string(), started_families);
        let ended_families =
            meeting_end_bundle_families(&after_call, &owned).expect("unambiguous probe");
        assert!(ended_families.is_empty());

        let mut now_ms = 0;
        assert_eq!(
            tracker.observe(Some(&families(&["windows:chrome"])), now_ms),
            None
        );
        let mut countdown = None;
        for _ in 0..MEETING_END_ABSENT_POLLS {
            now_ms += 1_000;
            countdown = tracker.observe(Some(&ended_families), now_ms).or(countdown);
        }
        assert!(matches!(
            countdown,
            Some(MeetingEndTransition::StateChanged(MeetingEndStatus {
                phase: MeetingEndPhase::Countdown,
                ..
            }))
        ));
        let mut finish = None;
        for _ in 0..MEETING_END_COUNTDOWN_POLLS {
            now_ms += 1_000;
            finish = tracker.observe(Some(&ended_families), now_ms).or(finish);
        }
        assert!(matches!(
            finish,
            Some(MeetingEndTransition::FinishQueued(
                MeetingEndStatus {
                    phase: MeetingEndPhase::FinishQueued,
                    ..
                },
                _
            ))
        ));

        let allowed_after = active_allowed_external_processes(&after_call, &owned);
        assert_eq!(
            detection.update(true, !allowed_after.is_empty(), false),
            None
        );
        assert_eq!(
            detection.update(true, !allowed_after.is_empty(), false),
            Some(MeetingDetectionEvent::Cleared)
        );
    }

    #[test]
    fn detector_clears_when_allowed_mic_process_becomes_unlisted() {
        let mut state = MeetingDetectionState::default();
        let active_allowed = allowed_pids(&[input_process(60, "com.google.Chrome")]);
        let active_unlisted = allowed_pids(&[input_process(61, "com.apple.PhotoBooth")]);

        assert_eq!(
            state.update(true, !active_allowed.is_empty(), false),
            Some(MeetingDetectionEvent::Detected)
        );
        assert_eq!(state.update(true, !active_unlisted.is_empty(), false), None);
        assert_eq!(
            state.update(true, !active_unlisted.is_empty(), false),
            Some(MeetingDetectionEvent::Cleared)
        );
    }

    #[test]
    fn detector_shows_when_external_input_starts() {
        let mut state = MeetingDetectionState::default();

        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Detected)
        );
        assert_eq!(state.update(true, true, false), None);
    }

    #[test]
    fn detector_suppresses_until_user_is_signed_in() {
        let mut state = MeetingDetectionState::default();

        assert_eq!(state.update(false, true, false), None);
        assert_eq!(state.update(false, true, false), None);
        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Detected)
        );
    }

    #[test]
    fn detector_clears_immediately_when_user_signs_out() {
        let mut state = MeetingDetectionState::default();

        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Detected)
        );
        assert_eq!(
            state.update(false, true, false),
            Some(MeetingDetectionEvent::Cleared)
        );
        assert_eq!(state.update(false, true, false), None);
    }

    #[test]
    fn detector_suppresses_while_clovy_capture_is_active() {
        let mut state = MeetingDetectionState::default();

        assert_eq!(state.update(true, true, true), None);
        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Detected)
        );
    }

    #[test]
    fn detector_clears_after_inactive_debounce() {
        let mut state = MeetingDetectionState::default();
        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Detected)
        );

        assert_eq!(state.update(true, false, false), None);
        assert_eq!(
            state.update(true, false, false),
            Some(MeetingDetectionEvent::Cleared)
        );
        assert_eq!(state.update(true, false, false), None);
    }

    #[test]
    fn detector_emits_heartbeat_while_active() {
        let mut state = MeetingDetectionState::default();
        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Detected)
        );

        for _ in 0..(HEARTBEAT_EVERY_ACTIVE_POLLS - 1) {
            assert_eq!(state.update(true, true, false), None);
        }
        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Heartbeat)
        );
    }

    #[test]
    fn detector_clears_when_clovy_capture_starts() {
        let mut state = MeetingDetectionState::default();
        assert_eq!(
            state.update(true, true, false),
            Some(MeetingDetectionEvent::Detected)
        );

        assert_eq!(state.update(true, true, true), None);
        assert_eq!(
            state.update(true, true, true),
            Some(MeetingDetectionEvent::Cleared)
        );
    }
}
