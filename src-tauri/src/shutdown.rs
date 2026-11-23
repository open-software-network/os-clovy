use crate::domain::types::AppError;
use std::process::Child;
use std::sync::mpsc;
use std::sync::{Condvar, Mutex, MutexGuard, TryLockError};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Manager;

/// App teardown gets this long in total, regardless of which individual leaf
/// is slow or stuck. The supervisor is separate from the cleanup worker, so a
/// blocking syscall in one leaf cannot postpone the final exit/restart.
///
/// Pending renderer note saves get a bounded first step (10 s), long enough for
/// an in-flight SQLite patch to return before the renderer acknowledges.
/// Dictation, Computer use, and CLI chat turns then stop concurrently under
/// the same bounded deadline.
const SHUTDOWN_AGGREGATE_DEADLINE_MS: u64 = 20_000;
const SHUTDOWN_AGGREGATE_DEADLINE: Duration = Duration::from_millis(SHUTDOWN_AGGREGATE_DEADLINE_MS);
const CLI_SHUTDOWN_BUDGET_MS: u64 = 5_000;
const EXIT_FALLBACK_DEADLINE: Duration = SHUTDOWN_AGGREGATE_DEADLINE;
#[cfg(windows)]
const UPDATER_AGGREGATE_DEADLINE_MS: u64 = SHUTDOWN_AGGREGATE_DEADLINE_MS
    + crate::note_save_flush::NOTE_SAVE_FLUSH_TIMEOUT_MS
    + CLI_SHUTDOWN_BUDGET_MS;
#[cfg(windows)]
const UPDATER_AGGREGATE_DEADLINE: Duration = Duration::from_millis(UPDATER_AGGREGATE_DEADLINE_MS);
const MUTEX_POLL_INTERVAL: Duration = Duration::from_millis(5);
const CHILD_POLL_INTERVAL: Duration = Duration::from_millis(10);

const _: () = assert!(
    SHUTDOWN_AGGREGATE_DEADLINE_MS
        >= crate::note_save_flush::NOTE_SAVE_FLUSH_TIMEOUT_MS + CLI_SHUTDOWN_BUDGET_MS
);

/// One absolute deadline shared by every leaf in a supervised cleanup run.
///
/// A leaf that has to wait for ownership stays on the cleanup worker's call
/// stack until this deadline. That makes the supervisor account for the wait:
/// successful cleanup delays finalization, while an expired deadline produces
/// an explicit leaf fallback instead of an untracked retry thread.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShutdownDeadline {
    expires_at: Instant,
}

impl ShutdownDeadline {
    pub(crate) fn after(timeout: Duration) -> Self {
        Self {
            expires_at: Instant::now() + timeout,
        }
    }

    pub(crate) fn remaining(self) -> Duration {
        self.expires_at.saturating_duration_since(Instant::now())
    }

    pub(crate) fn try_lock<'a, T>(self, mutex: &'a Mutex<T>) -> Option<MutexGuard<'a, T>> {
        try_lock_for(mutex, self.remaining())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownTarget {
    Exit(i32),
    Restart,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShutdownPhase {
    Idle,
    Running(ShutdownTarget),
    Finalizing(ShutdownTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BeginShutdown {
    Started(ShutdownTarget),
    AlreadyRunning(ShutdownTarget),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CleanupOutcome {
    Completed,
    NoteSaveFlushFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundedCleanupResult {
    Skipped,
    Completed,
    NoteSaveFlushFailed,
    TimedOut,
}

pub(crate) struct ShutdownCoordinator {
    phase: Mutex<ShutdownPhase>,
    phase_changed: Condvar,
}

impl Default for ShutdownCoordinator {
    fn default() -> Self {
        Self {
            phase: Mutex::new(ShutdownPhase::Idle),
            phase_changed: Condvar::new(),
        }
    }
}

impl ShutdownCoordinator {
    fn lock_phase(&self) -> MutexGuard<'_, ShutdownPhase> {
        self.phase
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The first request decides whether this process exits or restarts. Later
    /// requests share the in-flight cleanup and cannot change its final action.
    fn begin(&self, requested: ShutdownTarget) -> BeginShutdown {
        let mut phase = self.lock_phase();
        match *phase {
            ShutdownPhase::Idle => {
                *phase = ShutdownPhase::Running(requested);
                BeginShutdown::Started(requested)
            }
            ShutdownPhase::Running(target) | ShutdownPhase::Finalizing(target) => {
                BeginShutdown::AlreadyRunning(target)
            }
        }
    }

    fn mark_finalizing(&self, target: ShutdownTarget) {
        *self.lock_phase() = ShutdownPhase::Finalizing(target);
        self.phase_changed.notify_all();
    }

    fn cancel(&self, target: ShutdownTarget) {
        let mut phase = self.lock_phase();
        if *phase == ShutdownPhase::Running(target) {
            *phase = ShutdownPhase::Idle;
            self.phase_changed.notify_all();
        }
    }

    fn final_exit_is_allowed(&self) -> bool {
        matches!(*self.lock_phase(), ShutdownPhase::Finalizing(_))
    }

    fn is_idle(&self) -> bool {
        matches!(*self.lock_phase(), ShutdownPhase::Idle)
    }

    /// Blocks until the in-flight shutdown either finalizes or is cancelled
    /// (a failed note-save barrier returns the phase to Idle).
    fn wait_for_resolution(&self) -> ShutdownResolution {
        let mut phase = self.lock_phase();
        loop {
            match *phase {
                ShutdownPhase::Idle => return ShutdownResolution::Cancelled,
                ShutdownPhase::Finalizing(_) => return ShutdownResolution::Finalizing,
                ShutdownPhase::Running(_) => {
                    phase = self
                        .phase_changed
                        .wait(phase)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
        }
    }
}

/// How a shutdown request ended for its caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownResolution {
    Finalizing,
    Cancelled,
}

fn restart_result(resolution: ShutdownResolution) -> Result<(), AppError> {
    match resolution {
        ShutdownResolution::Finalizing => Ok(()),
        ShutdownResolution::Cancelled => Err(AppError::new(
            "update_relaunch_cancelled",
            "Pending note edits could not be saved, so Clovy did not restart. Try again.",
        )),
    }
}

/// Intercepts ordinary Tauri exit requests while the main event loop can still
/// be kept alive. `RunEvent::Exit` is too late: Tauri is already tearing down
/// the runtime and cannot reliably run asynchronous cleanup from there.
pub(crate) fn handle_exit_requested(
    app: &tauri::AppHandle,
    code: Option<i32>,
    api: &tauri::ExitRequestApi,
) {
    let coordinator = app.state::<ShutdownCoordinator>();
    if coordinator.final_exit_is_allowed() {
        return;
    }

    // Tauri ignores this for its built-in restart code, but ordinary exit must
    // remain alive while the coordinator owns teardown. Clovy's updater avoids
    // the restart exception by latching Restart directly through its command.
    api.prevent_exit();
    if code == Some(tauri::RESTART_EXIT_CODE) && coordinator.is_idle() {
        // A direct `app.restart()`/`request_restart()` caller can bypass
        // `prevent_exit` (and a main-thread restart skips ExitRequested
        // entirely). All restart callers except this coordinator's finalizer
        // must use `shutdown::request_restart`, which latches Running first.
        tracing::error!("restart reached ExitRequested without the shutdown coordinator");
        debug_assert!(
            !coordinator.is_idle(),
            "restart callers must use shutdown::request_restart, not Tauri restart directly"
        );
    }
    let target = if code == Some(tauri::RESTART_EXIT_CODE) {
        ShutdownTarget::Restart
    } else {
        ShutdownTarget::Exit(code.unwrap_or(0))
    };

    if let Err(error) = request(app, target) {
        tracing::error!(
            code = %error.code,
            "could not start the shutdown coordinator; allowing a fail-safe exit"
        );
        coordinator.mark_finalizing(target);
        finalize(app.clone(), target);
    }
}

/// Last-chance cleanup for termination paths that Tao delivers only as
/// `RunEvent::Exit` (including macOS logout/application termination). Normal
/// quits and update relaunches have already latched Running or Finalizing and
/// remain no-ops here.
pub(crate) fn handle_exit(app: &tauri::AppHandle) {
    let coordinator = app.state::<ShutdownCoordinator>();
    let cleanup_app = app.clone();
    let cleanup_deadline = ShutdownDeadline::after(EXIT_FALLBACK_DEADLINE);
    match run_bounded_cleanup_if_idle(
        &coordinator,
        ShutdownTarget::Exit(0),
        EXIT_FALLBACK_DEADLINE,
        move || tauri::async_runtime::block_on(run_cleanup(&cleanup_app, cleanup_deadline)),
    ) {
        BoundedCleanupResult::Completed => {
            tracing::info!("completed last-chance cleanup from RunEvent::Exit");
        }
        BoundedCleanupResult::NoteSaveFlushFailed => {
            // RunEvent::Exit cannot be prevented, but a failed renderer
            // barrier must still remain a failed coordinator state rather
            // than being promoted to Finalizing.
            tracing::error!(
                "last-chance note-save flush failed after OS finalization had already started"
            );
        }
        BoundedCleanupResult::Skipped | BoundedCleanupResult::TimedOut => {}
    }
}

/// Starts (or joins) a restart and resolves when it either finalizes or is
/// cancelled, so the caller can offer another try after a failed note-save
/// barrier instead of waiting forever.
pub(crate) async fn request_restart(app: &tauri::AppHandle) -> Result<(), AppError> {
    request(app, ShutdownTarget::Restart)?;
    let waiting_app = app.clone();
    let resolution = tauri::async_runtime::spawn_blocking(move || {
        waiting_app
            .state::<ShutdownCoordinator>()
            .wait_for_resolution()
    })
    .await
    .map_err(|error| AppError::new("update_relaunch_failed", error.to_string()))?;
    restart_result(resolution)
}

/// Prepares Windows for the updater's non-cancellable `install_inner` exit.
///
/// This runs before entering `Update::install`, while returning an error can
/// still prevent process finalization. A second renderer flush after native
/// cleanup catches edits queued while the helpers were shutting down.
#[cfg(windows)]
pub(crate) fn prepare_for_updater_exit(app: &tauri::AppHandle) -> Result<(), AppError> {
    let coordinator = app.state::<ShutdownCoordinator>();
    let cleanup_app = app.clone();
    match coordinator.begin(ShutdownTarget::Restart) {
        BeginShutdown::Started(target) => {
            let cleanup_deadline = ShutdownDeadline::after(UPDATER_AGGREGATE_DEADLINE);
            let outcome = run_bounded_cleanup(UPDATER_AGGREGATE_DEADLINE, move || {
                tauri::async_runtime::block_on(run_updater_cleanup(&cleanup_app, cleanup_deadline))
            });
            match outcome {
                // Stay in Running until the updater's on_before_exit callback.
                // `Update::install` can still fail while extracting the
                // package, and that failure must leave ordinary shutdown
                // retryable.
                Some(CleanupOutcome::Completed) => Ok(()),
                Some(CleanupOutcome::NoteSaveFlushFailed) => {
                    coordinator.cancel(target);
                    Err(AppError::new(
                        "update_note_save_failed",
                        "Pending note edits could not be saved. Try installing the update again.",
                    ))
                }
                None => {
                    coordinator.cancel(target);
                    tracing::warn!(
                        timeout_ms = UPDATER_AGGREGATE_DEADLINE.as_millis(),
                        "Windows updater cleanup hit its aggregate deadline"
                    );
                    Err(AppError::new(
                        "update_cleanup_timed_out",
                        "Clovy could not finish preparing for the update. Try installing it again.",
                    ))
                }
            }
        }
        BeginShutdown::AlreadyRunning(_) => Err(AppError::new(
            "update_cleanup_incomplete",
            "Clovy is already shutting down. Try installing the update again.",
        )),
    }
}

/// Called only by the updater after package extraction has succeeded and
/// immediately before it launches the installer and exits the process.
#[cfg(windows)]
pub(crate) fn finalize_updater_exit(app: &tauri::AppHandle) {
    app.state::<ShutdownCoordinator>()
        .mark_finalizing(ShutdownTarget::Restart);
    app.cleanup_before_exit();
}

/// Restores ordinary shutdown when `Update::install` fails before its
/// on-before-exit callback.
#[cfg(windows)]
pub(crate) fn cancel_updater_exit(app: &tauri::AppHandle) {
    app.state::<ShutdownCoordinator>()
        .cancel(ShutdownTarget::Restart);
}

#[cfg(windows)]
async fn run_updater_cleanup(app: &tauri::AppHandle, deadline: ShutdownDeadline) -> CleanupOutcome {
    let outcome = run_cleanup(app, deadline).await;
    if outcome != CleanupOutcome::Completed {
        return outcome;
    }

    if !crate::note_save_flush::request(app).await {
        tracing::warn!(
            timeout_ms = crate::note_save_flush::NOTE_SAVE_FLUSH_TIMEOUT_MS,
            "final Windows updater note-save flush failed"
        );
        return CleanupOutcome::NoteSaveFlushFailed;
    }
    CleanupOutcome::Completed
}

fn request(app: &tauri::AppHandle, requested: ShutdownTarget) -> Result<(), AppError> {
    let coordinator = app.state::<ShutdownCoordinator>();
    let BeginShutdown::Started(target) = coordinator.begin(requested) else {
        return Ok(());
    };

    let cleanup_app = app.clone();
    let final_app = app.clone();
    let cleanup_deadline = ShutdownDeadline::after(SHUTDOWN_AGGREGATE_DEADLINE);
    spawn_supervised_shutdown(
        SHUTDOWN_AGGREGATE_DEADLINE,
        move || tauri::async_runtime::block_on(run_cleanup(&cleanup_app, cleanup_deadline)),
        move |outcome| {
            match outcome {
                Some(CleanupOutcome::Completed) => {
                    tracing::info!(?target, "app shutdown cleanup completed");
                }
                Some(CleanupOutcome::NoteSaveFlushFailed) => {
                    tracing::warn!(
                        ?target,
                        "app shutdown cancelled because pending note saves could not be flushed"
                    );
                    final_app.state::<ShutdownCoordinator>().cancel(target);
                    return;
                }
                None => {
                    tracing::warn!(
                        ?target,
                        timeout_ms = SHUTDOWN_AGGREGATE_DEADLINE.as_millis(),
                        "app shutdown cleanup hit its aggregate deadline"
                    );
                }
            }

            final_app
                .state::<ShutdownCoordinator>()
                .mark_finalizing(target);
            finalize(final_app, target);
        },
    )
    .map_err(|error| AppError::new("shutdown_start_failed", error.to_string()))
}

fn spawn_supervised_shutdown<C, F>(deadline: Duration, cleanup: C, finish: F) -> std::io::Result<()>
where
    C: FnOnce() -> CleanupOutcome + Send + 'static,
    F: FnOnce(Option<CleanupOutcome>) + Send + 'static,
{
    thread::Builder::new()
        .name("clovy-shutdown-supervisor".to_string())
        .spawn(move || {
            let (done_tx, done_rx) = mpsc::sync_channel(1);
            let cleanup_spawn = thread::Builder::new()
                .name("clovy-shutdown-cleanup".to_string())
                .spawn(move || {
                    let _ = done_tx.send(cleanup());
                });
            let outcome = cleanup_spawn
                .ok()
                .and_then(|_| done_rx.recv_timeout(deadline).ok());
            finish(outcome);
        })
        .map(|_| ())
}

async fn run_cleanup(app: &tauri::AppHandle, deadline: ShutdownDeadline) -> CleanupOutcome {
    // Renderer autosaves must drain while the webview and command runtime are
    // still alive. Keep this ordered before native child-process teardown.
    if !crate::note_save_flush::request(app).await {
        return CleanupOutcome::NoteSaveFlushFailed;
    }

    if let Some(browser) = app.try_state::<std::sync::Arc<crate::browser_broker::BrowserBroker>>() {
        browser.terminate_sessions(deadline);
    }
    crate::providers::local_transcriber::shutdown().await;
    let dictation_app = app.clone();
    let dictation =
        tauri::async_runtime::spawn_blocking(move || crate::dictation::stop_helper(&dictation_app));
    let computer_use = crate::computer_use::shutdown(app);
    // CLI chat turns run in their own process groups, which application exit
    // does not reach: stop them and wait before exiting or restarting.
    let chat_engines = app.state::<crate::chat_engine::ChatEngineHost>();
    let (dictation_result, (), ()) = tokio::join!(dictation, computer_use, chat_engines.shutdown());
    if let Err(error) = dictation_result {
        tracing::warn!(%error, "dictation shutdown worker could not be joined");
    }
    // Every CLI turn has ended (or its wait ran out): no turn token may bind
    // an MCP connection any more.
    if let Some(mcp) = app.try_state::<crate::mcp_server::McpServerState>() {
        mcp.turns().revoke_all();
    }
    CleanupOutcome::Completed
}

fn run_bounded_cleanup<C>(deadline: Duration, cleanup: C) -> Option<CleanupOutcome>
where
    C: FnOnce() -> CleanupOutcome + Send + 'static,
{
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let spawned = thread::Builder::new()
        .name("clovy-shutdown-cleanup".to_string())
        .spawn(move || {
            let _ = done_tx.send(cleanup());
        })
        .is_ok();
    spawned
        .then(|| done_rx.recv_timeout(deadline).ok())
        .flatten()
}

fn run_bounded_cleanup_if_idle<C>(
    coordinator: &ShutdownCoordinator,
    target: ShutdownTarget,
    deadline: Duration,
    cleanup: C,
) -> BoundedCleanupResult
where
    C: FnOnce() -> CleanupOutcome + Send + 'static,
{
    let BeginShutdown::Started(target) = coordinator.begin(target) else {
        return BoundedCleanupResult::Skipped;
    };
    match run_bounded_cleanup(deadline, cleanup) {
        Some(CleanupOutcome::Completed) => {
            coordinator.mark_finalizing(target);
            BoundedCleanupResult::Completed
        }
        Some(CleanupOutcome::NoteSaveFlushFailed) => {
            coordinator.cancel(target);
            BoundedCleanupResult::NoteSaveFlushFailed
        }
        None => {
            tracing::warn!(
                ?target,
                timeout_ms = deadline.as_millis(),
                "last-chance app cleanup hit its deadline"
            );
            coordinator.cancel(target);
            BoundedCleanupResult::TimedOut
        }
    }
}

fn finalize(app: tauri::AppHandle, target: ShutdownTarget) {
    let final_app = app.clone();
    let scheduled = app.run_on_main_thread(move || match target {
        ShutdownTarget::Exit(code) => final_app.exit(code),
        ShutdownTarget::Restart => final_app.restart(),
    });
    if let Err(error) = scheduled {
        tracing::error!(%error, ?target, "could not schedule final shutdown action on the main thread");
        match target {
            ShutdownTarget::Exit(code) => app.exit(code),
            ShutdownTarget::Restart => app.request_restart(),
        }
    }
}

/// Acquires a synchronous mutex without allowing shutdown to wait forever on a
/// thread that may itself need the main event loop to make progress.
pub(crate) fn try_lock_for<T>(mutex: &Mutex<T>, timeout: Duration) -> Option<MutexGuard<'_, T>> {
    let deadline = Instant::now() + timeout;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(error)) => return Some(error.into_inner()),
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                thread::sleep(MUTEX_POLL_INTERVAL);
            }
            Err(TryLockError::WouldBlock) => return None,
        }
    }
}

/// Acquires an async mutex with a shutdown-only deadline.
pub(crate) async fn lock_async_for<T>(
    mutex: &tokio::sync::Mutex<T>,
    timeout: Duration,
) -> Option<tokio::sync::MutexGuard<'_, T>> {
    tokio::time::timeout(timeout, mutex.lock()).await.ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ChildTermination {
    Exited,
    Killed,
    TimedOut,
    WaitFailed,
}

/// Polls a child with `try_wait`, then escalates to the platform kill primitive
/// and polls again. If the bounded poll expires after SIGKILL, ownership moves
/// to a detached reaper so a late exit is still waited and cannot become a
/// zombie without holding shutdown open.
pub(crate) fn terminate_child(
    child: Child,
    graceful_timeout: Duration,
    kill_timeout: Duration,
) -> ChildTermination {
    terminate_child_with(child, graceful_timeout, kill_timeout, |child| {
        let _ = child.kill();
    })
}

pub(crate) fn terminate_child_with(
    mut child: Child,
    graceful_timeout: Duration,
    kill_timeout: Duration,
    escalate: impl FnOnce(&mut Child),
) -> ChildTermination {
    match poll_child_exit(&mut child, graceful_timeout) {
        Ok(true) => return ChildTermination::Exited,
        Err(()) => return ChildTermination::WaitFailed,
        Ok(false) => {}
    }

    escalate(&mut child);
    match poll_child_exit(&mut child, kill_timeout) {
        Ok(true) => ChildTermination::Killed,
        Ok(false) => {
            spawn_detached_child_reaper(child);
            ChildTermination::TimedOut
        }
        Err(()) => ChildTermination::WaitFailed,
    }
}

fn spawn_detached_child_reaper(mut child: Child) {
    if let Err(error) = thread::Builder::new()
        .name("clovy-child-reaper".to_string())
        .spawn(move || {
            let _ = child.wait();
        })
    {
        tracing::warn!(%error, "could not start detached child reaper");
    }
}

fn poll_child_exit(child: &mut Child, timeout: Duration) -> Result<bool, ()> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(true),
            Ok(None) if Instant::now() < deadline => thread::sleep(CHILD_POLL_INTERVAL),
            Ok(None) => return Ok(false),
            Err(_) => return Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn deadline_covers_ordered_leaves(aggregate_ms: u64, leaves_ms: &[u64]) -> bool {
        aggregate_ms >= leaves_ms.iter().sum::<u64>()
    }

    #[test]
    fn a_cancelled_restart_resolves_as_cancelled() {
        let coordinator = Arc::new(ShutdownCoordinator::default());
        coordinator.begin(ShutdownTarget::Restart);
        let waiter = {
            let coordinator = Arc::clone(&coordinator);
            thread::spawn(move || coordinator.wait_for_resolution())
        };
        thread::sleep(Duration::from_millis(20));
        coordinator.cancel(ShutdownTarget::Restart);
        assert_eq!(waiter.join().unwrap(), ShutdownResolution::Cancelled);
    }

    #[test]
    fn a_finalizing_restart_resolves_as_finalizing() {
        let coordinator = Arc::new(ShutdownCoordinator::default());
        coordinator.begin(ShutdownTarget::Restart);
        let waiter = {
            let coordinator = Arc::clone(&coordinator);
            thread::spawn(move || coordinator.wait_for_resolution())
        };
        thread::sleep(Duration::from_millis(20));
        coordinator.mark_finalizing(ShutdownTarget::Restart);
        assert_eq!(waiter.join().unwrap(), ShutdownResolution::Finalizing);
    }

    #[test]
    fn a_cancelled_restart_reports_a_retryable_error() {
        let error = restart_result(ShutdownResolution::Cancelled).unwrap_err();
        assert_eq!(error.code, "update_relaunch_cancelled");
        assert!(restart_result(ShutdownResolution::Finalizing).is_ok());
    }

    #[test]
    fn duplicate_requests_share_one_shutdown() {
        let coordinator = ShutdownCoordinator::default();
        assert_eq!(
            coordinator.begin(ShutdownTarget::Exit(0)),
            BeginShutdown::Started(ShutdownTarget::Exit(0))
        );
        assert_eq!(
            coordinator.begin(ShutdownTarget::Exit(0)),
            BeginShutdown::AlreadyRunning(ShutdownTarget::Exit(0))
        );
    }

    #[test]
    fn first_request_latches_exit_vs_restart() {
        let restart_first = ShutdownCoordinator::default();
        assert_eq!(
            restart_first.begin(ShutdownTarget::Restart),
            BeginShutdown::Started(ShutdownTarget::Restart)
        );
        assert_eq!(
            restart_first.begin(ShutdownTarget::Exit(7)),
            BeginShutdown::AlreadyRunning(ShutdownTarget::Restart)
        );

        let exit_first = ShutdownCoordinator::default();
        assert_eq!(
            exit_first.begin(ShutdownTarget::Exit(7)),
            BeginShutdown::Started(ShutdownTarget::Exit(7))
        );
        assert_eq!(
            exit_first.begin(ShutdownTarget::Restart),
            BeginShutdown::AlreadyRunning(ShutdownTarget::Exit(7))
        );
    }

    #[test]
    fn cleanup_wait_obeys_the_aggregate_deadline() {
        let (sender, receiver) = mpsc::sync_channel::<()>(1);
        let started = Instant::now();
        assert!(receiver.recv_timeout(Duration::from_millis(20)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(sender);
    }

    #[test]
    fn aggregate_deadline_covers_note_flush_and_cli_shutdown() {
        assert!(deadline_covers_ordered_leaves(
            SHUTDOWN_AGGREGATE_DEADLINE_MS,
            &[
                crate::note_save_flush::NOTE_SAVE_FLUSH_TIMEOUT_MS,
                CLI_SHUTDOWN_BUDGET_MS,
            ],
        ));
        assert!(
            EXIT_FALLBACK_DEADLINE
                >= Duration::from_millis(crate::note_save_flush::NOTE_SAVE_FLUSH_TIMEOUT_MS)
        );
    }

    #[cfg(windows)]
    #[test]
    fn updater_deadline_covers_cleanup_and_the_final_note_flush() {
        assert!(deadline_covers_ordered_leaves(
            UPDATER_AGGREGATE_DEADLINE_MS,
            &[
                SHUTDOWN_AGGREGATE_DEADLINE_MS,
                crate::note_save_flush::NOTE_SAVE_FLUSH_TIMEOUT_MS,
                CLI_SHUTDOWN_BUDGET_MS,
            ],
        ));
        assert_eq!(
            UPDATER_AGGREGATE_DEADLINE.as_millis(),
            u128::from(UPDATER_AGGREGATE_DEADLINE_MS)
        );
    }

    #[test]
    fn exit_fallback_runs_only_while_the_coordinator_is_idle() {
        let coordinator = ShutdownCoordinator::default();
        let calls = Arc::new(AtomicUsize::new(0));
        let first_calls = Arc::clone(&calls);
        assert_eq!(
            run_bounded_cleanup_if_idle(
                &coordinator,
                ShutdownTarget::Exit(0),
                Duration::from_secs(1),
                move || {
                    first_calls.fetch_add(1, Ordering::SeqCst);
                    CleanupOutcome::Completed
                },
            ),
            BoundedCleanupResult::Completed
        );
        assert!(matches!(
            *coordinator.lock_phase(),
            ShutdownPhase::Finalizing(ShutdownTarget::Exit(0))
        ));

        let duplicate_calls = Arc::clone(&calls);
        assert_eq!(
            run_bounded_cleanup_if_idle(
                &coordinator,
                ShutdownTarget::Restart,
                Duration::from_secs(1),
                move || {
                    duplicate_calls.fetch_add(1, Ordering::SeqCst);
                    CleanupOutcome::Completed
                },
            ),
            BoundedCleanupResult::Skipped
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let running = ShutdownCoordinator::default();
        assert_eq!(
            running.begin(ShutdownTarget::Restart),
            BeginShutdown::Started(ShutdownTarget::Restart)
        );
        assert_eq!(
            run_bounded_cleanup_if_idle(
                &running,
                ShutdownTarget::Exit(0),
                Duration::from_secs(1),
                || panic!("running cleanup must remain owned by its first caller"),
            ),
            BoundedCleanupResult::Skipped
        );
    }

    #[test]
    fn exit_fallback_keeps_failed_note_flush_out_of_finalizing() {
        let coordinator = ShutdownCoordinator::default();

        assert_eq!(
            run_bounded_cleanup_if_idle(
                &coordinator,
                ShutdownTarget::Exit(0),
                Duration::from_secs(1),
                || CleanupOutcome::NoteSaveFlushFailed,
            ),
            BoundedCleanupResult::NoteSaveFlushFailed
        );

        assert!(coordinator.is_idle());
        assert_eq!(
            coordinator.begin(ShutdownTarget::Exit(0)),
            BeginShutdown::Started(ShutdownTarget::Exit(0))
        );
    }

    #[test]
    fn exit_fallback_keeps_timed_out_cleanup_out_of_finalizing() {
        let coordinator = ShutdownCoordinator::default();
        let (release_tx, release_rx) = mpsc::sync_channel(1);

        assert_eq!(
            run_bounded_cleanup_if_idle(
                &coordinator,
                ShutdownTarget::Exit(0),
                Duration::from_millis(20),
                move || {
                    let _ = release_rx.recv();
                    CleanupOutcome::Completed
                },
            ),
            BoundedCleanupResult::TimedOut
        );

        assert!(coordinator.is_idle());
        assert_eq!(
            coordinator.begin(ShutdownTarget::Exit(0)),
            BeginShutdown::Started(ShutdownTarget::Exit(0))
        );
        let _ = release_tx.send(());
    }

    #[test]
    fn supervised_request_returns_before_blocked_cleanup() {
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let (final_tx, final_rx) = mpsc::sync_channel(1);
        let started = Instant::now();
        spawn_supervised_shutdown(
            Duration::from_millis(20),
            move || {
                let _ = release_rx.recv();
                CleanupOutcome::Completed
            },
            move |outcome| {
                let _ = final_tx.send(outcome);
            },
        )
        .expect("spawn supervisor");
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "request path must return control to the main event loop"
        );
        assert!(
            final_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("deadline finalizer")
                .is_none(),
            "deadline must finalize while cleanup remains blocked"
        );
        let _ = release_tx.send(());
    }

    #[test]
    fn failed_note_flush_cancels_shutdown_and_allows_retry() {
        let coordinator = ShutdownCoordinator::default();
        assert_eq!(
            coordinator.begin(ShutdownTarget::Restart),
            BeginShutdown::Started(ShutdownTarget::Restart)
        );

        coordinator.cancel(ShutdownTarget::Restart);

        assert!(coordinator.is_idle());
        assert_eq!(
            coordinator.begin(ShutdownTarget::Restart),
            BeginShutdown::Started(ShutdownTarget::Restart)
        );
    }

    #[test]
    fn mutex_acquisition_obeys_its_deadline() {
        let mutex = Mutex::new(());
        let _guard = mutex.lock().expect("test lock");
        let started = Instant::now();
        assert!(try_lock_for(&mutex, Duration::from_millis(20)).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn stopped_child_is_killed_and_reaped_without_blocking_wait() {
        use std::process::{Command, Stdio};

        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn stuck child");
        let result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGSTOP) };
        assert_eq!(result, 0, "stop child");

        let started = Instant::now();
        assert_eq!(
            terminate_child(child, Duration::from_millis(20), Duration::from_secs(1)),
            ChildTermination::Killed
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn zero_budget_termination_leaves_no_live_or_zombie_child() {
        use std::process::{Command, Stdio};

        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");
        let pid = child.id() as libc::pid_t;
        // Reaping may complete immediately or move to the detached thread.
        assert_ne!(
            terminate_child(child, Duration::ZERO, Duration::ZERO),
            ChildTermination::WaitFailed
        );

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let result = unsafe { libc::kill(pid, 0) };
            if result == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "zero-budget termination left the child alive or as a zombie"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
