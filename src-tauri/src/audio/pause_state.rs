use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct PauseState {
    state: Mutex<PauseClock>,
}

#[derive(Default)]
struct PauseClock {
    paused_at: Option<Instant>,
    elapsed_paused: Duration,
}

#[derive(Clone, Copy)]
pub(super) struct PauseSnapshot {
    pub(super) paused: bool,
    pub(super) active_time: Instant,
}

impl PauseState {
    pub(super) fn pause(&self, now: Instant) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.paused_at.is_none() {
            state.paused_at = Some(now);
        }
    }
    pub(super) fn resume(&self, now: Instant) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(paused_at) = state.paused_at.take() {
            state.elapsed_paused += now.saturating_duration_since(paused_at);
        }
    }
    pub(super) fn snapshot(&self) -> PauseSnapshot {
        self.snapshot_with(Instant::now, || {})
    }
    #[cfg(test)]
    pub(super) fn snapshot_at(&self, now: Instant) -> PauseSnapshot {
        self.snapshot_with(|| now, || {})
    }
    fn snapshot_with(
        &self,
        clock: impl FnOnce() -> Instant,
        between: impl FnOnce(),
    ) -> PauseSnapshot {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let now = clock();
        let paused_at = state.paused_at;
        let elapsed = state.elapsed_paused
            + paused_at.map_or(Duration::ZERO, |at| now.saturating_duration_since(at));
        let snapshot = PauseSnapshot {
            paused: paused_at.is_some(),
            active_time: now
                .checked_sub(elapsed)
                .expect("pause clock precedes monotonic origin"),
        };
        drop(state);
        between();
        snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, Barrier},
        thread,
        time::Duration,
    };
    #[test]
    fn stop_after_unobserved_resume_preserves_active_tail() {
        let start = Instant::now();
        let state = PauseState::default();
        let mut silence = super::super::active_silence::ActiveSilence::new(start);
        state.pause(start + Duration::from_secs(2));
        assert_eq!(
            silence.pause(
                state
                    .snapshot_at(start + Duration::from_secs(2))
                    .active_time,
                100
            ),
            200
        );
        state.resume(start + Duration::from_secs(102));
        let end = state.snapshot_at(start + Duration::from_secs(103));
        assert_eq!(silence.finish(end.active_time, 100), 100);
        state.pause(start + Duration::from_secs(104));
        state.resume(start + Duration::from_secs(204));
        state.pause(start + Duration::from_secs(205));
        assert_eq!(
            silence.finish(
                state
                    .snapshot_at(start + Duration::from_secs(300))
                    .active_time,
                100
            ),
            300
        );
    }
    #[test]
    fn packet_and_multiple_unobserved_pause_cycles_do_not_duplicate_silence() {
        let start = Instant::now();
        let state = PauseState::default();
        let mut silence = super::super::active_silence::ActiveSilence::new(start);
        silence.packet(start + Duration::from_secs(1));
        state.pause(start + Duration::from_secs(2));
        assert_eq!(
            silence.pause(
                state
                    .snapshot_at(start + Duration::from_secs(2))
                    .active_time,
                100
            ),
            100
        );
        state.resume(start + Duration::from_secs(102));
        state.pause(start + Duration::from_secs(104));
        state.resume(start + Duration::from_secs(204));
        state.pause(start + Duration::from_secs(205));
        assert_eq!(
            silence.finish(
                state
                    .snapshot_at(start + Duration::from_secs(300))
                    .active_time,
                100
            ),
            300
        );
    }
    #[test]
    fn capture_clock_started_during_pause_keeps_post_resume_tail() {
        let start = Instant::now();
        let state = PauseState::default();
        state.pause(start + Duration::from_secs(1));
        let mut silence = super::super::active_silence::ActiveSilence::new(
            state
                .snapshot_at(start + Duration::from_secs(50))
                .active_time,
        );
        assert_eq!(
            silence.pause(
                state
                    .snapshot_at(start + Duration::from_secs(50))
                    .active_time,
                100
            ),
            0
        );
        state.resume(start + Duration::from_secs(101));
        assert_eq!(
            silence.finish(
                state
                    .snapshot_at(start + Duration::from_secs(103))
                    .active_time,
                100
            ),
            200
        );
    }
    #[test]
    fn resume_during_capture_pause_cannot_change_the_observed_pause_time() {
        let state = Arc::new(PauseState::default());
        let paused_at = Instant::now() - Duration::from_secs(90);
        state.pause(paused_at);
        let observed = Arc::new(Barrier::new(2));
        let resumed = Arc::new(Barrier::new(2));
        let worker = {
            let (state, observed, resumed) = (state.clone(), observed.clone(), resumed.clone());
            thread::spawn(move || {
                state.snapshot_with(Instant::now, || {
                    observed.wait();
                    resumed.wait();
                })
            })
        };
        observed.wait();
        state.resume(Instant::now());
        resumed.wait();
        let captured = worker.join().unwrap();
        assert!(captured.paused);
        assert_eq!(captured.active_time, paused_at);
        let mut silence =
            super::super::active_silence::ActiveSilence::new(paused_at - Duration::from_secs(2));
        assert_eq!(silence.pause(captured.active_time, 100), 200);
        assert_eq!(silence.finish(captured.active_time, 100), 0);
        assert!(!state.snapshot().paused);
    }
}
