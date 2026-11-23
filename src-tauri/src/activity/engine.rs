//! The capture engine: one `tick` every 2 s on a dedicated thread. Each tick
//! reads permissions, evaluates the capture state, records pause transitions,
//! and, while active, captures the focused window (accessibility text first,
//! OCR fallback), secondary displays, input events, and clipboard changes,
//! dropping everything that an exclusion rule matches before it is read.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDateTime, Utc};

use super::filter::{is_browser, is_protected_video, skip_reason, within_work_hours};
use super::input::{clipboard_event, normalize_input, FocusTracker};
use super::key::ActivityKeyStore;
use super::platform::{
    ActivityPermission, ActivityPlatform, PermissionSnapshot, RawInputEvent, WindowDescriptor,
};
use super::schedule::{
    evaluate, low_disk_next, missing_required, plan_tick, CaptureState, GateInputs, StoreStatus,
    DISK_CHECK_EVERY_TICKS,
};
use super::settings::ActivitySettings;
use super::store::{
    timestamp, ActivityStore, NewFrame, NewInputEvent, NewSecondaryFrame, PauseReason, StoreError,
    TextSource,
};

/// Accessibility text shorter than this (non-whitespace chars) is treated as
/// "no text": usually only window chrome, so OCR reads the content instead.
pub const MIN_ACCESSIBILITY_CHARS: usize = 20;

pub trait Clock: Send {
    fn now_utc(&self) -> DateTime<Utc>;
    fn now_local(&self) -> NaiveDateTime;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> DateTime<Utc> {
        Utc::now()
    }

    fn now_local(&self) -> NaiveDateTime {
        Local::now().naive_local()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

struct StoreSlot {
    store: Option<ActivityStore>,
    status: StoreStatus,
    /// Try to open on the next tick (initially, after enabling, after a
    /// recreate, and periodically after a non-key failure).
    needs_open: bool,
}

/// State shared by the capture thread, Tauri commands, and the tray.
pub struct ActivityShared {
    settings: Mutex<ActivitySettings>,
    /// Held across a read-modify-write of the settings file
    /// (`update_settings`) so two writers never undo each other.
    settings_update: Mutex<()>,
    manual_pause: AtomicBool,
    state: Mutex<CaptureState>,
    permissions: Mutex<PermissionSnapshot>,
    last_frame_at: Mutex<Option<String>>,
    permission_requests: Mutex<Vec<ActivityPermission>>,
    slot: Mutex<StoreSlot>,
    wake: (Mutex<bool>, Condvar),
}

impl ActivityShared {
    pub fn new(settings: ActivitySettings) -> Self {
        Self {
            settings: Mutex::new(settings),
            settings_update: Mutex::new(()),
            manual_pause: AtomicBool::new(false),
            state: Mutex::new(CaptureState::Off),
            permissions: Mutex::new(PermissionSnapshot::UNSUPPORTED),
            last_frame_at: Mutex::new(None),
            permission_requests: Mutex::new(Vec::new()),
            slot: Mutex::new(StoreSlot {
                store: None,
                status: StoreStatus::Ready,
                needs_open: true,
            }),
            wake: (Mutex::new(false), Condvar::new()),
        }
    }

    pub fn settings(&self) -> ActivitySettings {
        lock(&self.settings).clone()
    }

    /// Changes the settings from their latest value, persists them to
    /// `path`, then publishes them; concurrent updates run one at a time.
    pub fn update_settings(
        &self,
        path: &std::path::Path,
        change: impl FnOnce(&mut ActivitySettings),
    ) -> std::io::Result<ActivitySettings> {
        let _serialized = lock(&self.settings_update);
        let mut settings = self.settings();
        change(&mut settings);
        super::settings::save(path, &settings)?;
        self.set_settings(settings.clone());
        Ok(settings)
    }

    pub(crate) fn update_settings_checked(
        &self,
        path: &std::path::Path,
        authorize: impl FnOnce() -> Result<(), crate::domain::types::AppError>,
        change: impl FnOnce(&mut ActivitySettings),
    ) -> Result<ActivitySettings, crate::domain::types::AppError> {
        let _serialized = lock(&self.settings_update);
        authorize()?;
        let mut settings = self.settings();
        change(&mut settings);
        super::settings::save(path, &settings).map_err(|error| {
            crate::domain::types::AppError::new("activity_settings_save_failed", error.to_string())
        })?;
        self.set_settings(settings.clone());
        Ok(settings)
    }

    pub fn set_settings(&self, settings: ActivitySettings) {
        let enabling = settings.needs_store() && !lock(&self.settings).needs_store();
        *lock(&self.settings) = settings;
        if enabling {
            lock(&self.slot).needs_open = true;
        }
        self.wake();
    }

    pub fn manual_pause(&self) -> bool {
        self.manual_pause.load(Ordering::SeqCst)
    }

    pub fn set_manual_pause(&self, paused: bool) {
        self.manual_pause.store(paused, Ordering::SeqCst);
        self.wake();
    }

    pub fn state(&self) -> CaptureState {
        lock(&self.state).clone()
    }

    pub fn permissions(&self) -> PermissionSnapshot {
        *lock(&self.permissions)
    }

    pub fn last_frame_at(&self) -> Option<String> {
        lock(&self.last_frame_at).clone()
    }

    pub fn request_permission(&self, permission: ActivityPermission) {
        lock(&self.permission_requests).push(permission);
        self.wake();
    }

    pub fn store(&self) -> Option<ActivityStore> {
        lock(&self.slot).store.clone()
    }

    /// Replaces the store after a user-confirmed recreate (or clears it).
    pub fn replace_store(&self, store: Option<ActivityStore>, status: StoreStatus) {
        let mut slot = lock(&self.slot);
        slot.needs_open = store.is_none() && status == StoreStatus::Ready;
        slot.store = store;
        slot.status = status;
        drop(slot);
        self.wake();
    }

    pub fn take_store(&self) -> Option<ActivityStore> {
        lock(&self.slot).store.take()
    }

    /// Makes the capture thread run its next tick now.
    pub fn wake(&self) {
        let (flag, condvar) = &self.wake;
        *lock(flag) = true;
        condvar.notify_all();
    }

    /// Sleeps until `deadline` or an explicit wake.
    pub fn wait_until(&self, deadline: Instant) {
        let (flag, condvar) = &self.wake;
        let mut woken = lock(flag);
        while !*woken {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (guard, _) = condvar
                .wait_timeout(woken, deadline - now)
                .unwrap_or_else(PoisonError::into_inner);
            woken = guard;
        }
        *woken = false;
    }

    fn set_state(&self, state: CaptureState) -> bool {
        let mut current = lock(&self.state);
        let changed = *current != state;
        *current = state;
        changed
    }

    fn set_permissions(&self, permissions: PermissionSnapshot) -> bool {
        let mut current = lock(&self.permissions);
        let changed = *current != permissions;
        *current = permissions;
        changed
    }

    fn set_last_frame_at(&self, at: String) {
        *lock(&self.last_frame_at) = Some(at);
    }

    fn take_permission_requests(&self) -> Vec<ActivityPermission> {
        std::mem::take(&mut *lock(&self.permission_requests))
    }
}

/// What a tick changed that observers (frontend, tray) care about.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub state_changed: bool,
    pub permissions_changed: bool,
    pub frames: usize,
}

pub struct CaptureEngine<P: ActivityPlatform> {
    platform: P,
    shared: Arc<ActivityShared>,
    keys: Arc<dyn ActivityKeyStore>,
    db_path: PathBuf,
    clock: Box<dyn Clock>,
    own_pid: i32,
    tick: u64,
    focus: FocusTracker,
    open_pause: Option<(i64, PauseReason)>,
    low_disk: bool,
    input_capture_on: bool,
    /// The window whose interval the buffered input belongs to; see
    /// `capture_focused`.
    epoch: Option<WindowEpoch>,
}

impl<P: ActivityPlatform> CaptureEngine<P> {
    pub fn new(
        platform: P,
        shared: Arc<ActivityShared>,
        keys: Arc<dyn ActivityKeyStore>,
        db_path: PathBuf,
        clock: Box<dyn Clock>,
        own_pid: i32,
    ) -> Self {
        Self {
            platform,
            shared,
            keys,
            db_path,
            clock,
            own_pid,
            tick: 0,
            focus: FocusTracker::default(),
            open_pause: None,
            low_disk: false,
            input_capture_on: false,
            epoch: None,
        }
    }

    pub async fn tick(&mut self) -> TickReport {
        let settings = self.shared.settings();
        let now = self.clock.now_utc();
        let plan = plan_tick(self.tick, &settings);
        let tick = self.tick;
        self.tick = self.tick.wrapping_add(1);

        for permission in self.shared.take_permission_requests() {
            self.platform.request_permission(permission);
        }
        let permissions = self.platform.permissions();
        let permissions_changed = self.shared.set_permissions(permissions);

        let store_status = if settings.needs_store() {
            self.ensure_store(now, tick).await
        } else {
            StoreStatus::Ready
        };
        let store = self.shared.store();
        let observing =
            settings.enabled && store.is_some() && missing_required(&permissions).is_empty();

        if observing && plan.disk_check {
            let directory = self.db_path.parent().unwrap_or(&self.db_path).to_path_buf();
            self.low_disk = low_disk_next(self.low_disk, self.platform.free_disk_bytes(&directory));
        }
        let focused = if observing {
            let input_idle_ms = self.platform.system_idle_ms();
            self.platform
                .focused_window()
                .map(|window| (window, input_idle_ms))
        } else {
            None
        };
        let protected_video = settings.pause_on_protected_video
            && focused.as_ref().is_some_and(|(window, _)| {
                is_protected_video(&window.app_name, window.browser_url.as_deref())
            });

        let state = evaluate(&GateInputs {
            enabled: settings.enabled,
            store: store_status,
            permissions,
            manual_pause: self.shared.manual_pause(),
            within_work_hours: within_work_hours(&settings.work_hours, self.clock.now_local()),
            low_disk: self.low_disk,
            protected_video,
        });

        if let Some(store) = &store {
            self.record_pause_transition(store, state.pause_reason(), now)
                .await;
        }

        let active = state == CaptureState::Active;
        let want_input =
            active && settings.input_events && permissions.input_monitoring.is_granted();
        if want_input != self.input_capture_on {
            self.platform.set_input_capture(want_input);
            self.input_capture_on = want_input;
        }
        // Always drain and poll so input and clipboard changes made while
        // paused or excluded are discarded rather than stored later.
        let raw_input = self.platform.drain_input_events();
        let clipboard = self.platform.clipboard_text_if_changed();

        let mut frames = 0;
        match (&store, active) {
            (Some(store), true) => {
                if let Some(window) = focused {
                    frames += self
                        .capture_focused(store, &settings, window, now, &raw_input, clipboard)
                        .await;
                } else {
                    self.focus.reset();
                    self.epoch = None;
                }
                if plan.secondary {
                    frames += self.capture_secondary(store, &settings, now).await;
                }
            }
            _ => {
                self.focus.reset();
                self.epoch = None;
            }
        }

        if plan.retention {
            if let Some(store) = &store {
                if let Err(error) = store.prune(now, settings.retention_days).await {
                    tracing::warn!(%error, "activity retention sweep failed");
                }
            }
        }

        TickReport {
            state_changed: self.shared.set_state(state),
            permissions_changed,
            frames,
        }
    }

    async fn ensure_store(&mut self, now: DateTime<Utc>, tick: u64) -> StoreStatus {
        let should_open = {
            let mut slot = lock(&self.shared.slot);
            if slot.store.is_some() {
                return StoreStatus::Ready;
            }
            if matches!(slot.status, StoreStatus::Failed(_)) && tick % DISK_CHECK_EVERY_TICKS == 0 {
                slot.needs_open = true;
            }
            std::mem::replace(&mut slot.needs_open, false)
        };
        if !should_open {
            return lock(&self.shared.slot).status.clone();
        }
        let (store, status) = match ActivityStore::open(&self.db_path, self.keys.as_ref()).await {
            Ok(store) => {
                if let Err(error) = store.close_open_pauses(now).await {
                    tracing::warn!(%error, "activity: could not close stale pauses");
                }
                if let Ok(Some(latest)) = store.latest_frame_at().await {
                    self.shared.set_last_frame_at(latest);
                }
                (Some(store), StoreStatus::Ready)
            }
            Err(StoreError::KeyMissing | StoreError::KeyRejected) => {
                (None, StoreStatus::KeyMissing)
            }
            Err(error) => {
                tracing::warn!(%error, "activity database unavailable");
                (None, StoreStatus::Failed(error.to_string()))
            }
        };
        let mut slot = lock(&self.shared.slot);
        slot.store = store;
        slot.status = status.clone();
        status
    }

    async fn record_pause_transition(
        &mut self,
        store: &ActivityStore,
        desired: Option<PauseReason>,
        now: DateTime<Utc>,
    ) {
        if let Some((id, reason)) = self.open_pause {
            if Some(reason) == desired {
                return;
            }
            if let Err(error) = store.close_pause(id, now).await {
                tracing::warn!(%error, "activity: could not close pause");
            }
            self.open_pause = None;
        }
        if let Some(reason) = desired {
            match store.open_pause(reason, now).await {
                Ok(id) => self.open_pause = Some((id, reason)),
                Err(error) => tracing::warn!(%error, "activity: could not record pause"),
            }
        }
    }

    /// Text of a window that passed the filters: accessibility first, OCR
    /// when the tree exposes (almost) nothing.
    fn window_text(&mut self, window: &WindowDescriptor) -> (TextSource, Option<String>) {
        if let Some(text) = self
            .platform
            .accessibility_text(window)
            .filter(|text| meaningful_chars(text) >= MIN_ACCESSIBILITY_CHARS)
        {
            return (TextSource::Accessibility, Some(text));
        }
        match self
            .platform
            .ocr_text(window)
            .filter(|text| !text.trim().is_empty())
        {
            Some(text) => (TextSource::Ocr, Some(text)),
            None => (TextSource::None, None),
        }
    }

    async fn capture_focused(
        &mut self,
        store: &ActivityStore,
        settings: &ActivitySettings,
        focused: (WindowDescriptor, Option<u64>),
        now: DateTime<Utc>,
        raw_input: &[RawInputEvent],
        clipboard: Option<String>,
    ) -> usize {
        let (window, input_idle_ms) = focused;
        if skip_reason(&window, settings, self.own_pid).is_some() {
            // Excluded: no text read, no frame, and the tick's input and
            // clipboard are discarded with it.
            self.focus.reset();
            self.epoch = None;
            return 0;
        }
        let key = WindowEpoch::of(&window);
        // Clicks, keys, and clipboard changes are buffered between ticks
        // without their source window. They are kept only when the same
        // capturable window (and page) was focused at both ends of the
        // interval; any switch in between could hide an excluded source.
        let same_epoch = self.epoch.as_ref() == Some(&key);
        let (text_source, text) = self.window_text(&window);
        if needs_revalidation(&window) {
            let after = self.platform.focused_window();
            if !unchanged_after_read(&window, after.as_ref(), settings, self.own_pid) {
                self.focus.reset();
                self.epoch = None;
                return 0;
            }
        }
        let frame = NewFrame {
            captured_at: now,
            app_name: window.app_name.clone(),
            bundle_id: window.bundle_id.clone(),
            window_title: window.window_title.clone(),
            browser_url: window.browser_url.clone(),
            text_source,
            text,
            input_idle_ms,
        };
        let stored = match store.insert_frame(&frame).await {
            Ok(_) => {
                self.shared.set_last_frame_at(timestamp(now));
                1
            }
            Err(error) => {
                tracing::warn!(%error, "activity: could not store frame");
                0
            }
        };
        self.epoch = Some(key);

        let app = Some(window.app_name.as_str());
        let mut events: Vec<NewInputEvent> = Vec::new();
        events.extend(self.focus.observe(&window, now));
        if same_epoch {
            if settings.input_events {
                events.extend(normalize_input(raw_input, app));
            }
            if let Some(text) = clipboard {
                events.extend(clipboard_event(&text, app, now));
            }
        }
        if let Err(error) = store.insert_input_events(&events).await {
            tracing::warn!(%error, "activity: could not store input events");
        }
        stored
    }

    async fn capture_secondary(
        &mut self,
        store: &ActivityStore,
        settings: &ActivitySettings,
        now: DateTime<Utc>,
    ) -> usize {
        let mut stored = 0;
        for window in self.platform.secondary_windows() {
            if skip_reason(&window, settings, self.own_pid).is_some()
                || (settings.pause_on_protected_video
                    && is_protected_video(&window.app_name, window.browser_url.as_deref()))
            {
                continue;
            }
            let (text_source, text) = self.window_text(&window);
            if needs_revalidation(&window) {
                let after = self
                    .platform
                    .secondary_windows()
                    .into_iter()
                    .find(|candidate| candidate.window_id == window.window_id);
                if !unchanged_after_read(&window, after.as_ref(), settings, self.own_pid) {
                    continue;
                }
            }
            let frame = NewSecondaryFrame {
                captured_at: now,
                display_id: window.display_id,
                app_name: window.app_name.clone(),
                bundle_id: window.bundle_id.clone(),
                window_title: window.window_title.clone(),
                browser_url: window.browser_url.clone(),
                text_source,
                text,
            };
            match store.insert_secondary_frame(&frame).await {
                Ok(_) => stored += 1,
                Err(error) => tracing::warn!(%error, "activity: could not store secondary frame"),
            }
        }
        stored
    }
}

/// Identity of the focused window (and page) for one capture interval.
#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowEpoch {
    pid: i32,
    window_id: Option<u32>,
    browser_url: Option<String>,
}

impl WindowEpoch {
    fn of(window: &WindowDescriptor) -> Self {
        Self {
            pid: window.pid,
            window_id: window.window_id,
            browser_url: window.browser_url.clone(),
        }
    }
}

/// Browser windows can navigate or switch tabs while their text is read.
fn needs_revalidation(window: &WindowDescriptor) -> bool {
    window.browser_url.is_some() || is_browser(&window.app_name, window.bundle_id.as_deref())
}

/// The descriptor re-read after text extraction still names the same window
/// and page, and that page is still capturable. Unknown counts as changed.
fn unchanged_after_read(
    before: &WindowDescriptor,
    after: Option<&WindowDescriptor>,
    settings: &ActivitySettings,
    own_pid: i32,
) -> bool {
    after.is_some_and(|after| {
        after.pid == before.pid
            && after.window_id == before.window_id
            && after.browser_url == before.browser_url
            && !after.private_window
            && skip_reason(after, settings, own_pid).is_none()
    })
}

fn meaningful_chars(text: &str) -> usize {
    text.chars().filter(|ch| !ch.is_whitespace()).count()
}

/// Runs `engine` forever on the calling thread: one tick, then sleep until
/// the next 2 s boundary or an explicit wake. `on_tick` observes each report.
pub fn run_capture_loop<P: ActivityPlatform>(
    mut engine: CaptureEngine<P>,
    tick: Duration,
    mut on_tick: impl FnMut(&TickReport),
) -> ! {
    loop {
        let started = Instant::now();
        let shared = Arc::clone(&engine.shared);
        let report = tauri::async_runtime::block_on(engine.tick());
        on_tick(&report);
        shared.wait_until(started + tick);
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use crate::activity::key::MemoryKeyStore;
    use crate::activity::platform::PermissionState;
    use crate::activity::settings::WorkHours;
    use crate::activity::store::{InputEventKind, ACTIVITY_DB_FILE};
    use chrono::{NaiveDate, TimeZone};
    use std::collections::VecDeque;
    use std::path::Path;

    #[derive(Default)]
    struct Calls {
        accessibility: Vec<String>,
        ocr: Vec<String>,
    }

    pub(crate) struct FakePlatform {
        pub(crate) permissions: PermissionSnapshot,
        focused: Option<WindowDescriptor>,
        secondary: Vec<WindowDescriptor>,
        ax_text: Option<String>,
        ocr_text: Option<String>,
        clipboard: VecDeque<String>,
        input: Vec<RawInputEvent>,
        free_disk: Option<u64>,
        input_idle_ms: Option<u64>,
        calls: Arc<Mutex<Calls>>,
        input_capture: Arc<AtomicBool>,
        /// Scripted `focused_window` results, consumed before `focused`.
        focused_reads: VecDeque<Option<WindowDescriptor>>,
        /// Scripted `secondary_windows` results, consumed before `secondary`.
        secondary_reads: VecDeque<Vec<WindowDescriptor>>,
    }

    impl FakePlatform {
        pub(crate) fn granted() -> Self {
            Self {
                permissions: PermissionSnapshot {
                    accessibility: PermissionState::Granted,
                    screen_recording: PermissionState::Granted,
                    input_monitoring: PermissionState::Granted,
                },
                focused: Some(zed()),
                secondary: Vec::new(),
                ax_text: Some("fn main() { println!(\"hello activity\"); }".into()),
                ocr_text: Some("text read by vision".into()),
                clipboard: VecDeque::new(),
                input: Vec::new(),
                free_disk: Some(100 << 30),
                input_idle_ms: None,
                calls: Arc::default(),
                input_capture: Arc::default(),
                focused_reads: VecDeque::new(),
                secondary_reads: VecDeque::new(),
            }
        }
    }

    impl ActivityPlatform for FakePlatform {
        fn permissions(&self) -> PermissionSnapshot {
            self.permissions
        }
        fn request_permission(&self, _permission: ActivityPermission) {}
        fn focused_window(&mut self) -> Option<WindowDescriptor> {
            self.focused_reads
                .pop_front()
                .unwrap_or_else(|| self.focused.clone())
        }
        fn system_idle_ms(&self) -> Option<u64> {
            self.input_idle_ms
        }
        fn secondary_windows(&mut self) -> Vec<WindowDescriptor> {
            self.secondary_reads
                .pop_front()
                .unwrap_or_else(|| self.secondary.clone())
        }
        fn accessibility_text(&mut self, window: &WindowDescriptor) -> Option<String> {
            lock(&self.calls)
                .accessibility
                .push(window.app_name.clone());
            self.ax_text.clone()
        }
        fn ocr_text(&mut self, window: &WindowDescriptor) -> Option<String> {
            lock(&self.calls).ocr.push(window.app_name.clone());
            self.ocr_text.clone()
        }
        fn clipboard_text_if_changed(&mut self) -> Option<String> {
            self.clipboard.pop_front()
        }
        fn set_input_capture(&mut self, enabled: bool) {
            self.input_capture.store(enabled, Ordering::SeqCst);
        }
        fn drain_input_events(&mut self) -> Vec<RawInputEvent> {
            std::mem::take(&mut self.input)
        }
        fn free_disk_bytes(&self, _path: &Path) -> Option<u64> {
            self.free_disk
        }
    }

    struct FixedClock(DateTime<Utc>, NaiveDateTime);

    impl Clock for FixedClock {
        fn now_utc(&self) -> DateTime<Utc> {
            self.0
        }
        fn now_local(&self) -> NaiveDateTime {
            self.1
        }
    }

    fn zed() -> WindowDescriptor {
        WindowDescriptor {
            pid: 100,
            app_name: "Zed".into(),
            bundle_id: Some("dev.zed.Zed".into()),
            window_title: Some("main.rs - clovy".into()),
            window_id: Some(9),
            ..WindowDescriptor::default()
        }
    }

    fn chrome(url: &str) -> WindowDescriptor {
        WindowDescriptor {
            pid: 200,
            app_name: "Google Chrome".into(),
            bundle_id: Some("com.google.Chrome".into()),
            window_title: Some("Page".into()),
            browser_url: Some(url.into()),
            window_id: Some(10),
            ..WindowDescriptor::default()
        }
    }

    fn enabled_settings() -> ActivitySettings {
        ActivitySettings {
            enabled: true,
            ignored_domains: vec!["youtube.com".into()],
            ..ActivitySettings::default()
        }
    }

    pub(crate) struct Harness {
        _dir: tempfile::TempDir,
        pub(crate) engine: CaptureEngine<FakePlatform>,
        pub(crate) shared: Arc<ActivityShared>,
        keys: Arc<MemoryKeyStore>,
        calls: Arc<Mutex<Calls>>,
    }

    impl Harness {
        pub(crate) fn new(platform: FakePlatform, settings: ActivitySettings) -> Self {
            // Tuesday 2026-09-29 10:00 local.
            let local = NaiveDate::from_ymd_opt(2026, 9, 29)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap();
            Self::at(platform, settings, local)
        }

        fn at(platform: FakePlatform, settings: ActivitySettings, local: NaiveDateTime) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let shared = Arc::new(ActivityShared::new(settings));
            let keys = Arc::new(MemoryKeyStore::default());
            let calls = Arc::clone(&platform.calls);
            let engine = CaptureEngine::new(
                platform,
                Arc::clone(&shared),
                keys.clone(),
                dir.path().join(ACTIVITY_DB_FILE),
                Box::new(FixedClock(
                    Utc.with_ymd_and_hms(2026, 9, 29, 13, 0, 0).unwrap(),
                    local,
                )),
                1,
            );
            Self {
                _dir: dir,
                engine,
                shared,
                keys,
                calls,
            }
        }

        fn store(&self) -> ActivityStore {
            self.shared.store().expect("store open")
        }

        pub(crate) async fn frames(&self) -> Vec<crate::activity::store::StoredFrame> {
            self.store().frames_after(0, 100).await.unwrap()
        }

        async fn events(&self) -> Vec<crate::activity::store::StoredInputEvent> {
            let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
            let to = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
            self.store().input_events_between(from, to).await.unwrap()
        }
    }

    #[tokio::test]
    async fn capture_turned_off_touches_nothing() {
        let settings = ActivitySettings {
            enabled: false,
            ..ActivitySettings::default()
        };
        let mut harness = Harness::new(FakePlatform::granted(), settings);
        harness.engine.tick().await;
        assert_eq!(harness.shared.state(), CaptureState::Off);
        assert!(harness.shared.store().is_none());
        assert!(harness.keys.current().is_none(), "no key minted while off");
        assert!(lock(&harness.calls).accessibility.is_empty());
    }

    #[tokio::test]
    async fn without_permissions_nothing_is_captured() {
        let mut platform = FakePlatform::granted();
        platform.permissions.accessibility = PermissionState::Denied;
        platform.permissions.screen_recording = PermissionState::NotDetermined;
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        assert_eq!(
            harness.shared.state(),
            CaptureState::NeedsPermissions {
                missing: vec![
                    ActivityPermission::Accessibility,
                    ActivityPermission::ScreenRecording
                ]
            }
        );
        assert!(harness.frames().await.is_empty());
        assert!(lock(&harness.calls).accessibility.is_empty());
    }

    #[tokio::test]
    async fn accessibility_frame_records_app_title_and_source() {
        let mut harness = Harness::new(FakePlatform::granted(), enabled_settings());
        let report = harness.engine.tick().await;
        assert_eq!(harness.shared.state(), CaptureState::Active);
        assert_eq!(report.frames, 1);
        let frames = harness.frames().await;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].app_name, "Zed");
        assert_eq!(frames[0].window_title.as_deref(), Some("main.rs - clovy"));
        assert_eq!(frames[0].text_source, TextSource::Accessibility);
        assert!(
            lock(&harness.calls).ocr.is_empty(),
            "no OCR when AX has text"
        );
        assert!(harness.shared.last_frame_at().is_some());
    }

    #[tokio::test]
    async fn tick_persists_system_idle_without_input_monitoring() {
        let mut platform = FakePlatform::granted();
        platform.permissions.input_monitoring = PermissionState::Denied;
        platform.input_idle_ms = Some(500);
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        let frames = harness.frames().await;
        assert_eq!(frames.len(), 1);
        let exported = serde_json::to_value(&frames[0]).unwrap();
        assert_eq!(exported["inputIdleMs"], serde_json::json!(500));
        assert!(!harness.engine.input_capture_on);
        assert!(harness
            .events()
            .await
            .iter()
            .all(|event| { !matches!(event.kind, InputEventKind::Click | InputEventKind::Key) }));
    }

    #[tokio::test]
    async fn tick_keeps_capturing_when_system_idle_is_unavailable() {
        let mut harness = Harness::new(FakePlatform::granted(), enabled_settings());
        harness.engine.tick().await;
        let frames = harness.frames().await;
        assert_eq!(frames.len(), 1);
        let exported = serde_json::to_value(&frames[0]).unwrap();
        assert_eq!(exported["inputIdleMs"], serde_json::Value::Null);
        assert_eq!(frames[0].app_name, "Zed");
    }

    #[tokio::test]
    async fn empty_accessibility_text_falls_back_to_ocr() {
        let mut platform = FakePlatform::granted();
        platform.ax_text = Some("Close Minimize".into());
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        let frames = harness.frames().await;
        assert_eq!(frames[0].text_source, TextSource::Ocr);
        assert_eq!(frames[0].text.as_deref(), Some("text read by vision"));
    }

    #[tokio::test]
    async fn ignored_domain_and_private_windows_are_never_read() {
        let mut platform = FakePlatform::granted();
        platform.focused = Some(chrome("https://www.youtube.com/watch?v=1"));
        platform.clipboard.push_back("copied on youtube".into());
        platform.input = vec![RawInputEvent::Click { at: Utc::now() }];
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        assert!(harness.frames().await.is_empty());
        assert!(harness.events().await.is_empty());

        let mut private = chrome("https://example.com");
        private.private_window = true;
        harness.engine.platform.focused = Some(private);
        harness.engine.tick().await;
        assert!(harness.frames().await.is_empty());
        let calls = lock(&harness.calls);
        assert!(
            calls.accessibility.is_empty() && calls.ocr.is_empty(),
            "no AX and no OCR"
        );
    }

    #[tokio::test]
    async fn clovy_windows_are_excluded() {
        let mut platform = FakePlatform::granted();
        platform.focused = Some(WindowDescriptor {
            pid: 1,
            app_name: "Clovy".into(),
            ..WindowDescriptor::default()
        });
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        assert!(harness.frames().await.is_empty());
    }

    #[tokio::test]
    async fn manual_pause_stops_frames_and_records_one_pause() {
        let mut harness = Harness::new(FakePlatform::granted(), enabled_settings());
        harness.engine.tick().await;
        assert_eq!(harness.frames().await.len(), 1);

        harness.shared.set_manual_pause(true);
        harness.engine.tick().await;
        harness.engine.tick().await;
        assert_eq!(
            harness.shared.state(),
            CaptureState::Paused {
                reason: PauseReason::Manual
            }
        );
        assert_eq!(harness.frames().await.len(), 1, "no frames while paused");
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        let pauses = harness.store().pauses_between(from, to).await.unwrap();
        assert_eq!(pauses.len(), 1);
        assert_eq!(pauses[0].reason, PauseReason::Manual);
        assert!(pauses[0].ended_at.is_none());

        harness.shared.set_manual_pause(false);
        harness.engine.tick().await;
        assert_eq!(harness.frames().await.len(), 2);
        let pauses = harness.store().pauses_between(from, to).await.unwrap();
        assert!(pauses[0].ended_at.is_some());
    }

    #[tokio::test]
    async fn outside_work_hours_capture_pauses_with_reason() {
        let settings = ActivitySettings {
            work_hours: WorkHours {
                enabled: true,
                ..WorkHours::default()
            },
            ..enabled_settings()
        };
        // Tuesday 20:00.
        let local = NaiveDate::from_ymd_opt(2026, 9, 29)
            .unwrap()
            .and_hms_opt(20, 0, 0)
            .unwrap();
        let mut harness = Harness::at(FakePlatform::granted(), settings, local);
        harness.engine.tick().await;
        assert_eq!(
            harness.shared.state(),
            CaptureState::Paused {
                reason: PauseReason::WorkHours
            }
        );
        assert!(harness.frames().await.is_empty());
    }

    #[tokio::test]
    async fn low_disk_and_protected_video_pause_capture() {
        let mut platform = FakePlatform::granted();
        platform.free_disk = Some(100 << 20);
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        assert_eq!(
            harness.shared.state().pause_reason(),
            Some(PauseReason::LowDisk)
        );

        let mut platform = FakePlatform::granted();
        platform.focused = Some(chrome("https://www.netflix.com/watch/1"));
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        assert_eq!(
            harness.shared.state().pause_reason(),
            Some(PauseReason::ProtectedVideo)
        );
        assert!(harness.frames().await.is_empty());
    }

    #[tokio::test]
    async fn input_events_are_stored_without_content_and_clipboard_redacted() {
        let platform = FakePlatform::granted();
        let input_capture = Arc::clone(&platform.input_capture);
        let mut harness = Harness::new(platform, enabled_settings());
        // First tick opens the interval in Zed.
        harness.engine.tick().await;
        assert!(
            input_capture.load(Ordering::SeqCst),
            "tap started while active"
        );
        let now = Utc::now();
        harness.engine.platform.input = vec![
            RawInputEvent::Key {
                at: now,
                characters: Some("hunter2".into()),
            },
            RawInputEvent::Click { at: now },
        ];
        harness
            .engine
            .platform
            .clipboard
            .push_back("token ghp_1234567890abcdefghijABCDEFGHIJ123456".into());
        harness.engine.tick().await;
        let events = harness.events().await;
        let kinds: Vec<InputEventKind> = events.iter().map(|event| event.kind).collect();
        assert!(kinds.contains(&InputEventKind::AppSwitch));
        assert!(kinds.contains(&InputEventKind::Key));
        assert!(kinds.contains(&InputEventKind::Click));
        let clipboard = events
            .iter()
            .find(|event| event.kind == InputEventKind::Clipboard)
            .unwrap();
        assert!(!clipboard
            .clipboard_text
            .as_deref()
            .unwrap()
            .contains("ghp_1234567890abcdefghijABCDEFGHIJ123456"));
        assert!(events
            .iter()
            .filter(|event| event.kind != InputEventKind::Clipboard)
            .all(|event| event.clipboard_text.is_none()));

        harness.shared.set_manual_pause(true);
        harness.engine.tick().await;
        assert!(
            !input_capture.load(Ordering::SeqCst),
            "tap stopped while paused"
        );
    }

    #[tokio::test]
    async fn input_from_an_excluded_window_is_not_attributed_to_the_next_one() {
        let mut platform = FakePlatform::granted();
        platform.focused = Some(chrome("https://www.youtube.com/watch?v=1"));
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;

        // Typed and copied on the ignored site, then switched to Zed before
        // the next tick drained the buffer.
        let now = Utc::now();
        harness.engine.platform.input = vec![
            RawInputEvent::Key {
                at: now,
                characters: None,
            },
            RawInputEvent::Click { at: now },
        ];
        harness
            .engine
            .platform
            .clipboard
            .push_back("copied on the ignored site".into());
        harness.engine.platform.focused = Some(zed());
        harness.engine.tick().await;
        let kinds: Vec<InputEventKind> = harness
            .events()
            .await
            .iter()
            .map(|event| event.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![InputEventKind::AppSwitch],
            "only the switch itself"
        );
        assert_eq!(
            harness.frames().await.len(),
            1,
            "the Zed frame is still captured"
        );

        // Input made while Zed stays focused is kept.
        harness.engine.platform.input = vec![RawInputEvent::Click { at: now }];
        harness.engine.tick().await;
        assert!(harness
            .events()
            .await
            .iter()
            .any(|event| event.kind == InputEventKind::Click));
    }

    #[tokio::test]
    async fn browser_navigating_to_an_ignored_domain_while_read_is_discarded() {
        let mut platform = FakePlatform::granted();
        let allowed = chrome("https://example.com/");
        platform.focused = Some(allowed.clone());
        platform.focused_reads = VecDeque::from([
            Some(allowed.clone()),
            Some(chrome("https://www.youtube.com/watch?v=1")),
        ]);
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        assert!(
            harness.frames().await.is_empty(),
            "text read during navigation must not be stored under the old URL"
        );

        // A tab that became private, or a different window, is discarded too.
        let mut private = allowed.clone();
        private.private_window = true;
        harness.engine.platform.focused_reads =
            VecDeque::from([Some(allowed.clone()), Some(private)]);
        harness.engine.tick().await;
        let mut other_window = allowed.clone();
        other_window.window_id = Some(99);
        harness.engine.platform.focused_reads =
            VecDeque::from([Some(allowed.clone()), Some(other_window)]);
        harness.engine.tick().await;
        harness.engine.platform.focused_reads = VecDeque::from([Some(allowed.clone()), None]);
        harness.engine.tick().await;
        assert!(harness.frames().await.is_empty());

        // Unchanged page: stored.
        harness.engine.tick().await;
        let frames = harness.frames().await;
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].browser_url.as_deref(),
            Some("https://example.com/")
        );
    }

    #[tokio::test]
    async fn secondary_browser_window_is_revalidated_before_storing() {
        let mut platform = FakePlatform::granted();
        let mut allowed = chrome("https://example.com/");
        allowed.display_id = Some(2);
        let mut navigated = allowed.clone();
        navigated.browser_url = Some("https://music.youtube.com/".into());
        platform.secondary_reads = VecDeque::from([vec![allowed.clone()], vec![navigated]]);
        let settings = ActivitySettings {
            secondary_monitors: true,
            ..enabled_settings()
        };
        let mut harness = Harness::new(platform, settings);
        harness.engine.tick().await;
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        assert!(harness
            .store()
            .secondary_frames_between(from, to)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn secondary_displays_are_sampled_only_when_enabled() {
        let mut platform = FakePlatform::granted();
        platform.secondary = vec![WindowDescriptor {
            pid: 300,
            app_name: "Slack".into(),
            display_id: Some(2),
            ..WindowDescriptor::default()
        }];
        let mut harness = Harness::new(platform, enabled_settings());
        harness.engine.tick().await;
        let from = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        assert!(harness
            .store()
            .secondary_frames_between(from, to)
            .await
            .unwrap()
            .is_empty());

        harness.shared.set_settings(ActivitySettings {
            secondary_monitors: true,
            ..enabled_settings()
        });
        for _ in 0..5 {
            harness.engine.tick().await;
        }
        let secondary = harness
            .store()
            .secondary_frames_between(from, to)
            .await
            .unwrap();
        assert_eq!(secondary.len(), 1);
        assert_eq!(secondary[0].display_id, Some(2));
        // The focused session is untouched: one primary frame per tick.
        assert_eq!(harness.frames().await.len(), 6);
    }

    #[tokio::test]
    async fn lost_key_blocks_capture_until_recreated() {
        let mut harness = Harness::new(FakePlatform::granted(), enabled_settings());
        harness.engine.tick().await;
        let store = harness.shared.take_store().unwrap();
        store.close().await;
        harness.keys.delete().unwrap();
        harness.shared.replace_store(None, StoreStatus::Ready);

        harness.engine.tick().await;
        assert_eq!(harness.shared.state(), CaptureState::KeyMissing);
        assert!(harness.shared.store().is_none());
    }
}

#[cfg(test)]
mod agent_settings_admission_tests {
    use super::*;
    #[test]
    fn revoked_authority_is_checked_after_the_settings_lock_and_does_not_write() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let shared = Arc::new(ActivityShared::new(ActivitySettings::default()));
        let guard = shared.settings_update.lock().unwrap();
        let revoked = Arc::new(AtomicBool::new(false));
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (checked_tx, checked_rx) = std::sync::mpsc::channel();
        let worker = {
            let shared = shared.clone();
            let revoked = revoked.clone();
            let path = path.clone();
            std::thread::spawn(move || {
                entered_tx.send(()).unwrap();
                shared.update_settings_checked(
                    &path,
                    || {
                        checked_tx.send(()).unwrap();
                        if revoked.load(Ordering::SeqCst) {
                            Err(crate::domain::types::AppError::new(
                                "turn_expired",
                                "revoked",
                            ))
                        } else {
                            Ok(())
                        }
                    },
                    |settings| settings.enabled = false,
                )
            })
        };
        entered_rx.recv().unwrap();
        assert!(checked_rx.try_recv().is_err());
        revoked.store(true, Ordering::SeqCst);
        drop(guard);
        assert_eq!(worker.join().unwrap().unwrap_err().code, "turn_expired");
        assert!(shared.settings().enabled);
        assert!(!path.exists());
    }
}

#[cfg(test)]
mod agent_settings_durable_stop_tests {
    use super::*;
    #[tokio::test]
    async fn durable_stop_blocks_a_waiting_save_even_before_registry_revocation() {
        let broker = crate::approval_broker::testing::TestBroker::new([]).await;
        let binding = broker.turn(crate::llm::cli::CliKind::Claude, true).await;
        let registry = crate::mcp_server::turns::TurnRegistry::default();
        registry.issue(binding.clone());
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings.json");
        let shared = Arc::new(ActivityShared::new(ActivitySettings::default()));
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let _guard = shared.settings_update.lock().unwrap();
                locked_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        };
        locked_rx.recv().unwrap();
        let pool = broker.pool().clone();
        let run_id = binding.run_id.clone();
        let runtime = tokio::runtime::Handle::current();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let worker = {
            let shared = shared.clone();
            let path = path.clone();
            let binding = binding.clone();
            std::thread::spawn(move || {
                entered_tx.send(()).unwrap();
                shared.update_settings_checked(
                    &path,
                    || {
                        runtime.block_on(crate::mcp_server::tools::revalidate_binding(
                            None, &pool, &binding, "default",
                        ))
                    },
                    |settings| settings.enabled = false,
                )
            })
        };
        entered_rx.recv().unwrap();
        sqlx::query::query("UPDATE agent_runs SET status='cancelled' WHERE id=?")
            .bind(run_id)
            .execute(broker.pool())
            .await
            .unwrap();
        assert!(registry.contains_binding(&binding));
        release_tx.send(()).unwrap();
        let result = tokio::task::spawn_blocking(move || {
            holder.join().unwrap();
            worker.join().unwrap()
        })
        .await
        .unwrap();
        assert_eq!(result.unwrap_err().code, "turn_expired");
        assert!(shared.settings().enabled);
        assert!(!path.exists());
    }
}
