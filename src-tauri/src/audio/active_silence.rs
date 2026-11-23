use std::time::Instant;

/// Tracks only an active trailing gap. Pausing commits the active portion;
/// resuming starts a new gap without counting the paused wall time.
pub(super) struct ActiveSilence {
    since: Instant,
    paused: bool,
}

impl ActiveSilence {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            since: now,
            paused: false,
        }
    }
    pub(super) fn packet(&mut self, now: Instant) {
        self.since = now;
    }
    pub(super) fn pause(&mut self, now: Instant, rate: u32) -> u32 {
        if self.paused {
            return 0;
        }
        let frames = self.frames(now, rate);
        self.paused = true;
        self.since = now;
        frames
    }
    pub(super) fn resume(&mut self) {
        self.paused = false;
    }
    pub(super) fn finish(&self, now: Instant, rate: u32) -> u32 {
        self.frames(now, rate)
    }
    fn frames(&self, now: Instant, rate: u32) -> u32 {
        (now.saturating_duration_since(self.since).as_secs_f64() * f64::from(rate))
            .floor()
            .clamp(0.0, f64::from(u32::MAX)) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn pause_stop_and_resume_exclude_paused_time_but_keep_active_silence() {
        let start = Instant::now();
        let mut clock = ActiveSilence::new(start);
        clock.packet(start + Duration::from_secs(1));
        assert_eq!(clock.pause(start + Duration::from_secs(3), 100), 200);
        assert_eq!(clock.finish(start + Duration::from_secs(3), 100), 0);
        clock.resume();
        assert_eq!(clock.finish(start + Duration::from_secs(6), 100), 300);
    }
}
