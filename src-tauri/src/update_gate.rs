//! Keeps an update install from ending Clovy in the middle of a capture.
//!
//! On Windows `Update::install` exits the process, and on macOS the restart
//! that follows the install does too. A meeting recording or a dictation take
//! started while the package downloads must therefore hold the install back
//! until it ends, and once the install has begun no new capture may start.
//! Capture starts and the install decision take the same lock, so neither can
//! slip past the other.

use std::collections::BTreeSet;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use crate::domain::types::AppError;

pub static UPDATE_GATE: UpdateGate = UpdateGate::new();

struct GateState {
    installing: bool,
    starting_recordings: usize,
    /// Dictation takes in flight, by take id: from the admitted start command
    /// (or the helper's start event) to the take's terminal event.
    dictation_takes: BTreeSet<String>,
    /// The owner chose "later" for the current install attempt. Read and set
    /// under the same lock as the install slot, so a postpone either stops
    /// the install or is refused because the install already began.
    postponed: bool,
}

impl GateState {
    /// Takes the install slot when no capture runs or starts.
    fn admit_install(&mut self, capture_active: impl FnOnce() -> bool) -> bool {
        if self.starting_recordings > 0 || !self.dictation_takes.is_empty() || capture_active() {
            return false;
        }
        self.installing = true;
        true
    }
}

pub struct UpdateGate {
    state: Mutex<GateState>,
}

/// The install could not begin: a capture is running or starting.
#[derive(Debug, PartialEq, Eq)]
pub struct CaptureBusy;

/// Held from the start of a recording until it is published as active.
pub struct RecordingStart<'a> {
    gate: &'a UpdateGate,
}

/// Held while the install runs. Dropping it (a failed install) lets captures
/// start again.
pub struct InstallSlot<'a> {
    gate: &'a UpdateGate,
}

impl UpdateGate {
    pub const fn new() -> Self {
        Self {
            state: Mutex::new(GateState {
                installing: false,
                starting_recordings: 0,
                dictation_takes: BTreeSet::new(),
                postponed: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, GateState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn begin_recording_start(&self) -> Result<RecordingStart<'_>, AppError> {
        let mut state = self.lock();
        if state.installing {
            return Err(installing_error());
        }
        state.starting_recordings += 1;
        Ok(RecordingStart { gate: self })
    }

    /// Admits a dictation take before its start command reaches the helper.
    /// It holds the install back, with no deadline, until its terminal event.
    pub fn begin_dictation_take(&self, take_id: &str) -> Result<(), AppError> {
        let mut state = self.lock();
        if state.installing {
            return Err(installing_error());
        }
        state.dictation_takes.insert(take_id.to_string());
        Ok(())
    }

    /// The helper reported a take as started. A take that did not pass
    /// through `begin_dictation_take` is registered the same way, unless an
    /// update is installing: then it must be stopped.
    pub fn observe_dictation_start(&self, take_id: &str) -> DictationStart {
        let mut state = self.lock();
        if state.dictation_takes.contains(take_id) {
            return DictationStart::Keep;
        }
        if state.installing {
            return DictationStart::Stop;
        }
        state.dictation_takes.insert(take_id.to_string());
        DictationStart::Keep
    }

    /// The take reached its terminal event, or its start never reached the
    /// helper.
    pub fn end_dictation_take(&self, take_id: &str) {
        self.lock().dictation_takes.remove(take_id);
    }

    #[cfg(test)]
    pub fn has_dictation_take(&self, take_id: &str) -> bool {
        self.lock().dictation_takes.contains(take_id)
    }

    /// The helper is gone: no take can still be capturing or delivering.
    pub fn end_all_dictation_takes(&self) {
        self.lock().dictation_takes.clear();
    }

    /// `capture_active` runs under the gate lock, so a capture that is not
    /// active yet is still registered as starting.
    pub fn try_begin_install(
        &self,
        capture_active: impl FnOnce() -> bool,
    ) -> Result<InstallSlot<'_>, CaptureBusy> {
        let mut state = self.lock();
        if !state.admit_install(capture_active) {
            return Err(CaptureBusy);
        }
        Ok(InstallSlot { gate: self })
    }

    /// Waits until no capture runs, then takes the install slot. `on_wait`
    /// runs once, the first time a capture holds the install back. The owner
    /// can postpone while it waits.
    pub async fn wait_for_install_slot(
        &self,
        capture_active: impl Fn() -> bool,
        poll: Duration,
        on_wait: impl FnOnce(),
    ) -> Result<InstallSlot<'_>, InstallPostponed> {
        let mut on_wait = Some(on_wait);
        loop {
            {
                let mut state = self.lock();
                if state.postponed {
                    return Err(InstallPostponed);
                }
                if state.admit_install(&capture_active) {
                    return Ok(InstallSlot { gate: self });
                }
            }
            if let Some(on_wait) = on_wait.take() {
                on_wait();
            }
            tokio::time::sleep(poll).await;
        }
    }
}

impl UpdateGate {
    /// A new install attempt starts: an earlier "later" no longer applies.
    pub fn begin_install_attempt(&self) {
        self.lock().postponed = false;
    }

    /// The owner chose "later" while the install waits for a capture. Once
    /// the install holds its slot it can no longer be stopped.
    pub fn postpone_install(&self) -> Result<(), AppError> {
        let mut state = self.lock();
        if state.installing {
            return Err(AppError::new(
                "update_install_started",
                "The update is already installing. Clovy restarts in a moment.",
            ));
        }
        state.postponed = true;
        Ok(())
    }
}

fn installing_error() -> AppError {
    AppError::new(
        "update_installing",
        "Clovy is installing an update and restarts in a moment.",
    )
}

/// What to do with a take the helper reports as started.
#[derive(Debug, PartialEq, Eq)]
pub enum DictationStart {
    Keep,
    /// An update is installing: the take must be discarded.
    Stop,
}

/// The owner postponed an install that was waiting for a capture to end.
#[derive(Debug, PartialEq, Eq)]
pub struct InstallPostponed;

impl Default for UpdateGate {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for RecordingStart<'_> {
    fn drop(&mut self) {
        let mut state = self.gate.lock();
        state.starting_recordings = state.starting_recordings.saturating_sub(1);
    }
}

impl Drop for InstallSlot<'_> {
    fn drop(&mut self) {
        self.gate.lock().installing = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn an_active_capture_holds_the_install() {
        let gate = UpdateGate::new();
        assert_eq!(gate.try_begin_install(|| true).err(), Some(CaptureBusy));
    }

    #[test]
    fn a_recording_still_starting_holds_the_install() {
        let gate = UpdateGate::new();
        let start = gate.begin_recording_start().unwrap();
        assert_eq!(gate.try_begin_install(|| false).err(), Some(CaptureBusy));
        drop(start);
        assert!(gate.try_begin_install(|| false).is_ok());
    }

    #[test]
    fn an_admitted_dictation_take_holds_the_install_with_no_deadline() {
        let gate = UpdateGate::new();
        gate.begin_dictation_take("take-a").unwrap();
        // A permission prompt can keep the helper from confirming for minutes;
        // the gate keeps no clock, so only the take's end releases it.
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(gate.try_begin_install(|| false).err(), Some(CaptureBusy));
    }

    #[test]
    fn a_dictation_take_holds_the_install_until_its_own_end() {
        let gate = UpdateGate::new();
        gate.begin_dictation_take("take-a").unwrap();
        gate.begin_dictation_take("take-b").unwrap();
        gate.end_dictation_take("take-a");
        assert_eq!(gate.try_begin_install(|| false).err(), Some(CaptureBusy));
        gate.end_dictation_take("take-b");
        assert!(gate.try_begin_install(|| false).is_ok());
    }

    #[test]
    fn a_take_the_helper_started_on_its_own_holds_the_install() {
        let gate = UpdateGate::new();
        assert_eq!(gate.observe_dictation_start("take-h"), DictationStart::Keep);
        assert_eq!(gate.try_begin_install(|| false).err(), Some(CaptureBusy));
        gate.end_all_dictation_takes();
        assert!(gate.try_begin_install(|| false).is_ok());
    }

    #[test]
    fn a_take_the_helper_starts_during_the_install_is_stopped() {
        let gate = UpdateGate::new();
        let _install = gate.try_begin_install(|| false).unwrap();
        assert_eq!(gate.observe_dictation_start("take-h"), DictationStart::Stop);
    }

    #[test]
    fn no_capture_starts_while_the_update_installs() {
        let gate = UpdateGate::new();
        let _install = gate.try_begin_install(|| false).unwrap();
        let recording = gate.begin_recording_start().err().map(|error| error.code);
        assert_eq!(recording.as_deref(), Some("update_installing"));
        let dictation = gate
            .begin_dictation_take("take-a")
            .err()
            .map(|error| error.code);
        assert_eq!(dictation.as_deref(), Some("update_installing"));
    }

    #[tokio::test]
    async fn the_install_waits_for_the_capture_to_end() {
        let gate = UpdateGate::new();
        let polls = std::sync::atomic::AtomicUsize::new(0);
        let waited = std::cell::Cell::new(0);
        let slot = gate
            .wait_for_install_slot(
                || polls.fetch_add(1, Ordering::SeqCst) < 2,
                Duration::from_millis(1),
                || waited.set(waited.get() + 1),
            )
            .await;
        assert!(slot.is_ok());
        assert_eq!(polls.load(Ordering::SeqCst), 3);
        assert_eq!(waited.get(), 1);
    }

    #[tokio::test]
    async fn the_owner_can_postpone_a_waiting_install() {
        let gate = UpdateGate::new();
        gate.begin_install_attempt();
        let slot = gate
            .wait_for_install_slot(
                || true,
                Duration::from_millis(1),
                || gate.postpone_install().unwrap(),
            )
            .await;
        assert_eq!(slot.err(), Some(InstallPostponed));
        assert!(gate.begin_recording_start().is_ok());
    }

    #[tokio::test]
    async fn a_postpone_that_arrives_first_keeps_the_install_from_starting() {
        let gate = UpdateGate::new();
        gate.begin_install_attempt();
        gate.postpone_install().unwrap();
        let slot = gate
            .wait_for_install_slot(|| false, Duration::from_millis(1), || {})
            .await;
        assert_eq!(slot.err(), Some(InstallPostponed));
    }

    #[test]
    fn a_postpone_after_the_install_began_is_refused() {
        let gate = UpdateGate::new();
        gate.begin_install_attempt();
        let _install = gate.try_begin_install(|| false).unwrap();
        let refused = gate.postpone_install().err().map(|error| error.code);
        assert_eq!(refused.as_deref(), Some("update_install_started"));
    }

    #[test]
    fn a_new_attempt_forgets_an_earlier_postpone() {
        let gate = UpdateGate::new();
        gate.postpone_install().unwrap();
        gate.begin_install_attempt();
        assert!(gate.try_begin_install(|| false).is_ok());
    }

    #[test]
    fn a_failed_install_lets_captures_start_again() {
        let gate = UpdateGate::new();
        drop(gate.try_begin_install(|| false).unwrap());
        assert!(gate.begin_recording_start().is_ok());
        assert!(gate.begin_dictation_take("take-a").is_ok());
    }
}
