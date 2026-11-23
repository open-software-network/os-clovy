//! Per-note serial processing queue.
//!
//! Audio *capture* is globally single-instance (see `audio::capture`), but
//! *processing* (transcribe → generate) runs asynchronously after a recording
//! stops. Stopping frees the capture slot immediately, so a user can record
//! another message on the same note while the previous one is still being
//! transcribed or generated.
//!
//! Those follow-up recordings must still be processed **in order**: generation
//! is incremental and feeds the note's existing generated content back in as
//! context, so job N has to finish before job N+1 reads that context. This
//! queue serializes processing per note and tracks how many jobs are waiting so
//! the UI can surface a count.

use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        Arc, LazyLock, Mutex,
    },
};
use tokio::sync::Mutex as AsyncMutex;

struct NoteQueue {
    /// Held for the duration of a single job; the next job awaits it.
    lock: Arc<AsyncMutex<()>>,
    /// Jobs queued or running for this note (1 = running, 2 = one waiting, …).
    pending: Arc<AtomicI64>,
    /// Jobs on this note that have finished, for a queued job to see what
    /// ran after it was enqueued.
    completed: Arc<AtomicU64>,
}

static QUEUES: LazyLock<Mutex<HashMap<String, NoteQueue>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// A registered processing job. Acquire the lock before doing work; call
/// [`ProcessingTicket::finish`] when the job completes.
pub struct ProcessingTicket {
    note_id: String,
    lock: Arc<AsyncMutex<()>>,
    pending: Arc<AtomicI64>,
    completed: Arc<AtomicU64>,
    completed_at_enqueue: u64,
    finished: AtomicBool,
}

impl ProcessingTicket {
    /// Clone the per-note lock. Await `.lock()` on the returned handle to wait
    /// for any earlier job on the same note to finish before processing.
    pub fn lock(&self) -> Arc<AsyncMutex<()>> {
        self.lock.clone()
    }

    /// Runs on this note that finished after this one was enqueued.
    pub fn runs_completed_since_enqueue(&self) -> u64 {
        self.completed
            .load(Ordering::SeqCst)
            .saturating_sub(self.completed_at_enqueue)
    }

    /// Mark this job done: drop it from the pending count and prune the note's
    /// queue entry once nothing else is waiting.
    pub fn finish(&self) {
        if self.finished.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut map = QUEUES.lock().expect("processing queue mutex poisoned");
        self.completed.fetch_add(1, Ordering::SeqCst);
        let remaining = self.pending.fetch_sub(1, Ordering::SeqCst) - 1;
        if remaining <= 0 {
            // Only remove if the map still points at our queue (a concurrent
            // enqueue takes the same `QUEUES` lock, so this can't race).
            if let Some(queue) = map.get(&self.note_id) {
                if Arc::ptr_eq(&queue.pending, &self.pending)
                    && queue.pending.load(Ordering::SeqCst) <= 0
                {
                    map.remove(&self.note_id);
                }
            }
        }
    }
}

impl Drop for ProcessingTicket {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Register a processing job for `note_id`. Returns the ticket and the queue
/// depth *including* this job (1 = runs immediately, 2 = one job ahead, …).
pub fn enqueue(note_id: &str) -> (ProcessingTicket, i64) {
    let mut map = QUEUES.lock().expect("processing queue mutex poisoned");
    let queue = map.entry(note_id.to_string()).or_insert_with(|| NoteQueue {
        lock: Arc::new(AsyncMutex::new(())),
        pending: Arc::new(AtomicI64::new(0)),
        completed: Arc::new(AtomicU64::new(0)),
    });
    let depth = queue.pending.fetch_add(1, Ordering::SeqCst) + 1;
    let ticket = ProcessingTicket {
        note_id: note_id.to_string(),
        lock: queue.lock.clone(),
        pending: queue.pending.clone(),
        completed: queue.completed.clone(),
        completed_at_enqueue: queue.completed.load(Ordering::SeqCst),
        finished: AtomicBool::new(false),
    };
    (ticket, depth)
}

/// Number of recordings queued *behind* the one currently processing for this
/// note (0 when nothing extra is waiting). Drives the UI count chip.
pub fn queued_behind(note_id: &str) -> i64 {
    let map = QUEUES.lock().expect("processing queue mutex poisoned");
    map.get(note_id)
        .map(|queue| (queue.pending.load(Ordering::SeqCst) - 1).max(0))
        .unwrap_or(0)
}

/// Who asked for a retry run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryOrigin {
    /// The note's Retry button.
    Manual,
    /// The automatic resume of a note paused by a provider rate limit, after
    /// it claimed the pause.
    Resume,
}

/// Whether a retry run that just took the note's processing lock should
/// still run.
///
/// Checked under the lock for every run, whatever its queue depth: the
/// order runs take the lock is not the order they were queued. A ready note
/// is never processed again. A manual retry that marked the still-failed
/// note transcribing itself (`marked_by_this_run`) always runs, so a run
/// that changed the note is never skipped and never leaves it transcribing.
/// Otherwise, when another run finished after this one was queued, a manual
/// retry runs only on a note that run left failed, and a resume does not run
/// at all; with nothing finished in between, the Retry runs, and a resume
/// runs on the note its claim left transcribing.
pub fn queued_retry_should_run(
    origin: RetryOrigin,
    status: crate::domain::types::ProcessingStatus,
    runs_completed_since_enqueue: u64,
    marked_by_this_run: bool,
) -> bool {
    use crate::domain::types::ProcessingStatus;
    if status == ProcessingStatus::Ready {
        return false;
    }
    let nothing_ran_since = runs_completed_since_enqueue == 0;
    match origin {
        RetryOrigin::Manual => {
            marked_by_this_run || nothing_ran_since || status == ProcessingStatus::Failed
        }
        RetryOrigin::Resume => nothing_ran_since && status == ProcessingStatus::Transcribing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::ProcessingStatus;

    #[test]
    fn a_queued_retry_runs_only_while_its_note_still_needs_it() {
        use ProcessingStatus::{Failed, Generating, Ready, Transcribing};
        use RetryOrigin::{Manual, Resume};
        let runs = |origin, status, finished, marked| {
            queued_retry_should_run(origin, status, finished, marked)
        };
        // Nothing ran since it was queued: the Retry of a failed note runs.
        assert!(runs(Manual, Failed, 0, false));
        assert!(runs(Manual, Transcribing, 0, false));
        // It marked the failed note transcribing itself: it always runs, so
        // a note it marked is never left transcribing by a skipped run.
        assert!(runs(Manual, Transcribing, 0, true));
        assert!(runs(Manual, Transcribing, 1, true));
        // A run finished meanwhile: only a note it left failed runs again.
        assert!(runs(Manual, Failed, 1, false));
        assert!(!runs(Manual, Ready, 1, false));
        assert!(!runs(Manual, Generating, 1, false));
        // A ready note is never processed again by a retry.
        assert!(!runs(Manual, Ready, 0, false));
        // A resume runs only on the note its claim left transcribing, with no
        // run in between.
        assert!(runs(Resume, Transcribing, 0, false));
        assert!(!runs(Resume, Transcribing, 1, false));
        assert!(!runs(Resume, Ready, 1, false));
        assert!(!runs(Resume, Failed, 1, false));
    }

    #[test]
    fn a_retry_queued_first_but_run_second_sees_the_run_that_finished_first() {
        // The resume claimed the pause; the manual retry queued first (depth
        // 1) but was suspended, and the resume (depth 2) took the lock first.
        let note_id = format!("note-race-{}", uuid::Uuid::new_v4());
        let (manual, manual_depth) = enqueue(&note_id);
        let (resume, resume_depth) = enqueue(&note_id);
        assert_eq!((manual_depth, resume_depth), (1, 2));

        assert_eq!(resume.runs_completed_since_enqueue(), 0);
        assert!(queued_retry_should_run(
            RetryOrigin::Resume,
            ProcessingStatus::Transcribing,
            resume.runs_completed_since_enqueue(),
            false
        ));
        resume.finish();

        // The resume left the note ready: the manual retry must not generate
        // it again, whatever its queue depth was.
        assert_eq!(manual.runs_completed_since_enqueue(), 1);
        assert!(!queued_retry_should_run(
            RetryOrigin::Manual,
            ProcessingStatus::Ready,
            manual.runs_completed_since_enqueue(),
            false
        ));
        // Had the resume failed again, the Retry still runs.
        assert!(queued_retry_should_run(
            RetryOrigin::Manual,
            ProcessingStatus::Failed,
            manual.runs_completed_since_enqueue(),
            false
        ));
        manual.finish();
    }

    #[test]
    fn first_job_runs_immediately_and_reports_nothing_queued() {
        let (ticket, depth) = enqueue("note-immediate");
        assert_eq!(depth, 1);
        assert_eq!(queued_behind("note-immediate"), 0);
        ticket.finish();
        assert_eq!(queued_behind("note-immediate"), 0);
    }

    #[test]
    fn stacked_jobs_increment_depth_and_queued_count() {
        let (first, first_depth) = enqueue("note-stacked");
        let (second, second_depth) = enqueue("note-stacked");
        let (third, third_depth) = enqueue("note-stacked");
        assert_eq!((first_depth, second_depth, third_depth), (1, 2, 3));
        // Two recordings wait behind the one currently processing.
        assert_eq!(queued_behind("note-stacked"), 2);

        first.finish();
        assert_eq!(queued_behind("note-stacked"), 1);
        second.finish();
        assert_eq!(queued_behind("note-stacked"), 0);
        third.finish();
        assert_eq!(queued_behind("note-stacked"), 0);
    }

    #[test]
    fn queues_are_isolated_per_note() {
        let (a, _) = enqueue("note-a");
        let (b1, _) = enqueue("note-b");
        let (b2, _) = enqueue("note-b");
        assert_eq!(queued_behind("note-a"), 0);
        assert_eq!(queued_behind("note-b"), 1);
        a.finish();
        b1.finish();
        b2.finish();
    }

    #[test]
    fn dropped_ticket_releases_pending_job() {
        {
            let (_ticket, depth) = enqueue("note-drop");
            assert_eq!(depth, 1);
        }

        let (next, depth) = enqueue("note-drop");
        assert_eq!(depth, 1);
        next.finish();
    }

    #[test]
    fn finish_is_idempotent() {
        let (first, _) = enqueue("note-idempotent");
        let (second, _) = enqueue("note-idempotent");

        first.finish();
        first.finish();

        let (third, depth) = enqueue("note-idempotent");
        assert_eq!(depth, 2);
        second.finish();
        third.finish();
    }

    #[tokio::test]
    async fn lock_serializes_jobs_on_the_same_note() {
        let (first, _) = enqueue("note-serial");
        let (second, _) = enqueue("note-serial");
        let first_lock = first.lock();
        let second_lock = second.lock();

        let held = first_lock.lock().await;
        // The second job cannot acquire the lock while the first holds it.
        assert!(second_lock.try_lock().is_err());
        drop(held);
        assert!(second_lock.try_lock().is_ok());

        first.finish();
        second.finish();
    }
}
