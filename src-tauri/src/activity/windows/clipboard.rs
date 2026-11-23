//! Clipboard monitoring on Windows with password manager concealment detection.

use std::{
    ptr::NonNull,
    sync::{mpsc, LazyLock},
    time::{Duration, Instant},
};
use windows::core::{w, BOOL};
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, GetClipboardData, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW,
};
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::System::Threading::GetCurrentThreadId;

#[link(name = "user32")]
extern "system" {
    fn GetThreadDesktop(dwthreadid: u32) -> HANDLE;
    fn SetThreadDesktop(hdesktop: HANDLE) -> BOOL;
}

const CF_UNICODETEXT: u32 = 13;

static FORMAT_EXCLUDE: LazyLock<u32> = LazyLock::new(|| unsafe {
    RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"))
});

static FORMAT_HISTORY: LazyLock<u32> =
    LazyLock::new(|| unsafe { RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")) });

struct ClipboardGuard;

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

struct LockedClipboardData<'a> {
    memory: HGLOBAL,
    pointer: NonNull<u8>,
    length: usize,
    _clipboard: &'a ClipboardGuard,
}

impl<'a> LockedClipboardData<'a> {
    fn new(format: u32, clipboard: &'a ClipboardGuard) -> Option<Self> {
        let handle = unsafe { GetClipboardData(format) }.ok()?;
        if handle.is_invalid() {
            return None;
        }
        let memory = HGLOBAL(handle.0);
        let length = unsafe { GlobalSize(memory) };
        if length == 0 || length > isize::MAX as usize {
            return None;
        }
        let pointer = NonNull::new(unsafe { GlobalLock(memory) }.cast::<u8>())?;
        Some(Self {
            memory,
            pointer,
            length,
            _clipboard: clipboard,
        })
    }

    fn bytes(&self) -> &[u8] {
        // GlobalSize bounds the allocation; the lock and clipboard guard
        // remain alive for the entire borrowed slice.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.length) }
    }

    fn utf16(&self) -> Option<&[u16]> {
        let pointer = self.pointer.as_ptr().cast::<u16>();
        if !pointer.is_aligned() {
            return None;
        }
        Some(unsafe { std::slice::from_raw_parts(pointer, self.length / 2) })
    }
}

impl Drop for LockedClipboardData<'_> {
    fn drop(&mut self) {
        unsafe {
            let _ = GlobalUnlock(self.memory);
        }
    }
}

/// Evaluates clipboard concealment markers and only calls `text_reader` if content is allowed.
/// If content is marked concealed or history is disabled, `text_reader` is NEVER called,
/// preventing sensitive passwords/tokens from ever being requested or read into memory.
pub(crate) fn read_clipboard_content_with<F>(
    exclude_available: bool,
    can_include_in_history: bool,
    mut text_reader: F,
) -> Option<String>
where
    F: FnMut() -> Option<String>,
{
    if exclude_available {
        return None;
    }
    if !can_include_in_history {
        return None;
    }
    let text = text_reader()?;
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn history_allows_text(format_present: bool, value: Option<u32>) -> bool {
    !format_present || value.is_some_and(|value| value != 0)
}

fn parse_utf16_slice(units: &[u16]) -> Option<String> {
    let terminator = units.iter().position(|&unit| unit == 0)?;
    Some(String::from_utf16_lossy(&units[..terminator]))
}

fn parse_history_dword_slice(bytes: &[u8]) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(..4)?.try_into().ok()?))
}

/// Pure sequence-state policy for tracking clipboard updates across ticks.
#[derive(Debug, Default)]
pub(crate) struct ClipboardSequenceState {
    last_completed_sequence: u32,
}

impl ClipboardSequenceState {
    pub(crate) fn new(initial_sequence: u32) -> Self {
        Self {
            last_completed_sequence: initial_sequence,
        }
    }

    /// Returns true if `sequence` represents new content eligible for inspection.
    pub(crate) fn is_eligible(&self, sequence: u32) -> bool {
        sequence != self.last_completed_sequence
    }

    /// Marks `sequence` as completed, suppressing it until the sequence advances.
    pub(crate) fn mark_completed(&mut self, sequence: u32) {
        self.last_completed_sequence = sequence;
    }
}

struct WorkerRequest {
    epoch: u64,
    requested_sequence: u32,
}

#[derive(Debug, PartialEq, Eq)]
enum WorkerStatus {
    Success,
    Busy,
    Concealed,
    ChangedBeforeRead,
    ChangedDuringRead,
    FormatUnavailable,
    Error,
}

struct WorkerResponse {
    epoch: u64,
    sequence_after: u32,
    text: Option<String>,
    status: WorkerStatus,
}

fn process_worker_request(req: &WorkerRequest) -> WorkerResponse {
    if unsafe { OpenClipboard(None).is_err() } {
        return WorkerResponse {
            epoch: req.epoch,
            sequence_after: unsafe { GetClipboardSequenceNumber() },
            text: None,
            status: WorkerStatus::Busy,
        };
    }
    let _guard = ClipboardGuard;

    // 2. Sequence check after Open before plaintext
    let seq_after_open = unsafe { GetClipboardSequenceNumber() };
    if seq_after_open != req.requested_sequence {
        return WorkerResponse {
            epoch: req.epoch,
            sequence_after: seq_after_open,
            text: None,
            status: WorkerStatus::ChangedBeforeRead,
        };
    }

    // 3. Concealment format checks
    let exclude_fmt = *FORMAT_EXCLUDE;
    let history_fmt = *FORMAT_HISTORY;
    if exclude_fmt == 0 || history_fmt == 0 {
        let seq_after_read = unsafe { GetClipboardSequenceNumber() };
        return WorkerResponse {
            epoch: req.epoch,
            sequence_after: seq_after_read,
            text: None,
            status: WorkerStatus::Error,
        };
    }

    let exclude_available = unsafe { IsClipboardFormatAvailable(exclude_fmt).is_ok() };
    let history_available = unsafe { IsClipboardFormatAvailable(history_fmt).is_ok() };
    let history_value = if !exclude_available && history_available {
        LockedClipboardData::new(history_fmt, &_guard)
            .and_then(|data| parse_history_dword_slice(data.bytes()))
    } else {
        None
    };

    let can_include_history = history_allows_text(history_available, history_value);
    if exclude_available || !can_include_history {
        let seq_after_read = unsafe { GetClipboardSequenceNumber() };
        return WorkerResponse {
            epoch: req.epoch,
            sequence_after: seq_after_read,
            text: None,
            status: WorkerStatus::Concealed,
        };
    }

    // 4. Plaintext check and read under open clipboard lock
    let text = read_clipboard_content_with(exclude_available, can_include_history, || {
        if unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() } {
            return None;
        }
        let data = LockedClipboardData::new(CF_UNICODETEXT, &_guard)?;
        parse_utf16_slice(data.utf16()?)
    });

    let seq_after_read = unsafe { GetClipboardSequenceNumber() };
    let (status, text) = if seq_after_read != req.requested_sequence {
        (WorkerStatus::ChangedDuringRead, None)
    } else if text.is_some() {
        (WorkerStatus::Success, text)
    } else {
        (WorkerStatus::FormatUnavailable, None)
    };

    WorkerResponse {
        epoch: req.epoch,
        sequence_after: seq_after_read,
        text,
        status,
    }
}

fn run_worker(
    desktop_raw: usize,
    request_rx: mpsc::Receiver<WorkerRequest>,
    response_tx: mpsc::SyncSender<WorkerResponse>,
) {
    if desktop_raw == 0
        || !unsafe { SetThreadDesktop(HANDLE(desktop_raw as *mut core::ffi::c_void)).as_bool() }
    {
        return;
    }

    while let Ok(req) = request_rx.recv() {
        let resp = process_worker_request(&req);
        if response_tx.send(resp).is_err() {
            break;
        }
    }
}

const CLIPBOARD_READ_TIMEOUT: Duration = Duration::from_millis(50);

pub(crate) struct Clipboard {
    sequence_state: ClipboardSequenceState,
    // Dropping these channels detaches rather than joining a native read.
    worker_tx: Option<mpsc::SyncSender<WorkerRequest>>,
    worker_rx: mpsc::Receiver<WorkerResponse>,
    active_epoch: u64,
    pending_epoch: Option<u64>,
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipboard {
    pub(crate) fn new() -> Self {
        let initial = unsafe { GetClipboardSequenceNumber() };
        let caller_thread_id = unsafe { GetCurrentThreadId() };
        let desktop_handle = unsafe { GetThreadDesktop(caller_thread_id) };
        let desktop_raw = if !desktop_handle.is_invalid() && !desktop_handle.0.is_null() {
            desktop_handle.0 as usize
        } else {
            0
        };

        let (request_tx, request_rx) = mpsc::sync_channel(1);
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        let worker_tx = if desktop_raw == 0 {
            None
        } else {
            match std::thread::Builder::new()
                .name("clovy-clipboard-worker".into())
                .spawn(move || run_worker(desktop_raw, request_rx, response_tx))
            {
                Ok(_detached) => Some(request_tx),
                Err(_) => None,
            }
        };

        Self {
            sequence_state: ClipboardSequenceState::new(initial),
            worker_tx,
            worker_rx: response_rx,
            active_epoch: 0,
            pending_epoch: None,
        }
    }

    /// Retrieves the general clipboard text if the sequence number has advanced
    /// since the previous inspection, unless the clipboard declares concealed or
    /// transient formats used by password managers.
    pub(crate) fn changed_text(&mut self) -> Option<String> {
        self.reap_pending_response();
        if self.pending_epoch.is_some() {
            return None;
        }
        let current_seq = unsafe { GetClipboardSequenceNumber() };
        if !self.sequence_state.is_eligible(current_seq) {
            return None;
        }

        let worker_tx = self.worker_tx.as_ref()?;
        self.active_epoch = self.active_epoch.wrapping_add(1);
        let epoch = self.active_epoch;
        let deadline = Instant::now() + CLIPBOARD_READ_TIMEOUT;
        worker_tx
            .try_send(WorkerRequest {
                epoch,
                requested_sequence: current_seq,
            })
            .ok()?;
        self.pending_epoch = Some(epoch);
        let remaining = deadline.saturating_duration_since(Instant::now());
        match self.worker_rx.recv_timeout(remaining) {
            Ok(response) => {
                if Instant::now() >= deadline || response.epoch != epoch {
                    self.discard_expired_response(response);
                    return None;
                }
                self.pending_epoch = None;
                match response.status {
                    // A changed sequence can be read afresh on a later tick.
                    WorkerStatus::Busy
                    | WorkerStatus::ChangedBeforeRead
                    | WorkerStatus::ChangedDuringRead => None,
                    WorkerStatus::Success => {
                        if unsafe { GetClipboardSequenceNumber() } != response.sequence_after {
                            return None;
                        }
                        self.sequence_state.mark_completed(response.sequence_after);
                        response.text
                    }
                    _ => {
                        self.sequence_state.mark_completed(response.sequence_after);
                        None
                    }
                }
            }
            // A timeout leaves the single in-flight read quarantined until terminal.
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                self.pending_epoch = None;
                self.worker_tx = None;
                None
            }
        }
    }

    fn reap_pending_response(&mut self) {
        while let Ok(response) = self.worker_rx.try_recv() {
            self.discard_expired_response(response);
        }
    }

    fn discard_expired_response(&mut self, response: WorkerResponse) {
        if self.pending_epoch == Some(response.epoch) {
            self.pending_epoch = None;
        }
        // Never replay an expired read, including a delayed render that advanced
        // its sequence. Only suppress that snapshot, not a newer clipboard change.
        if !matches!(
            response.status,
            WorkerStatus::Busy | WorkerStatus::ChangedBeforeRead
        ) && unsafe { GetClipboardSequenceNumber() } == response.sequence_after
        {
            self.sequence_state.mark_completed(response.sequence_after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clipboard_pre_read_concealment_lazy_reader() {
        let mut reader_invoked = false;

        // 1. ExcludeClipboardContentFromMonitorProcessing present: text reader MUST NOT be invoked
        let res1 = read_clipboard_content_with(true, true, || {
            reader_invoked = true;
            Some("sensitive_password".to_string())
        });
        assert_eq!(res1, None);
        assert_eq!(
            reader_invoked, false,
            "Text reader closure MUST NOT be invoked when exclude format is present"
        );

        // 2. CanIncludeInClipboardHistory = 0: text reader MUST NOT be invoked
        let res2 = read_clipboard_content_with(false, false, || {
            reader_invoked = true;
            Some("one_time_token".to_string())
        });
        assert_eq!(res2, None);
        assert_eq!(
            reader_invoked, false,
            "Text reader closure MUST NOT be invoked when CanIncludeInClipboardHistory is 0"
        );

        // 3. Normal content: text reader IS invoked
        let res3 = read_clipboard_content_with(false, true, || {
            reader_invoked = true;
            Some("regular_text".to_string())
        });
        assert_eq!(res3, Some("regular_text".to_string()));
        assert_eq!(
            reader_invoked, true,
            "Text reader must be invoked for allowed content"
        );
    }

    #[test]
    fn unreadable_history_marker_never_requests_plaintext() {
        let result = read_clipboard_content_with(false, history_allows_text(true, None), || {
            panic!("An unreadable password-manager marker must conceal plaintext")
        });
        assert_eq!(result, None);
    }

    #[test]
    fn test_clipboard_bounded_utf16_unterminated_and_short_dword() {
        // 1. Unterminated UTF-16: no null terminator in allocation
        let unterminated = [b'H' as u16, b'i' as u16, b'!' as u16];
        let parsed = parse_utf16_slice(&unterminated);
        assert_eq!(
            parsed, None,
            "Unterminated UTF-16 allocation must be rejected (return None) to prevent reading out of bounds"
        );

        // 2. Properly terminated UTF-16
        let terminated = [b'H' as u16, b'i' as u16, 0, b'X' as u16];
        let parsed_ok = parse_utf16_slice(&terminated);
        assert_eq!(parsed_ok, Some("Hi".to_string()));

        // 3. Short allocation for history DWORD: size < 4 bytes
        let short_dword: [u8; 3] = [1, 0, 0];
        let parsed_dword = parse_history_dword_slice(&short_dword);
        assert_eq!(
            parsed_dword, None,
            "History allocation smaller than 4 bytes must be rejected (return None)"
        );

        // 4. Valid 4-byte history DWORD
        let valid_dword: [u8; 4] = [1, 0, 0, 0];
        let parsed_valid = parse_history_dword_slice(&valid_dword);
        assert_eq!(parsed_valid, Some(1));
    }

    #[test]
    fn test_clipboard_sequence_policy() {
        let mut policy = ClipboardSequenceState::new(100);

        // Same sequence as initial must be suppressed (ineligible)
        assert_eq!(
            policy.is_eligible(100),
            false,
            "Initial sequence number must be suppressed to avoid capturing pre-existing clipboard"
        );

        // Advanced sequence 101 is eligible
        assert_eq!(
            policy.is_eligible(101),
            true,
            "Advanced sequence number must be eligible for inspection"
        );

        // Successfully completing sequence 101 suppresses it on subsequent checks
        policy.mark_completed(101);
        assert_eq!(
            policy.is_eligible(101),
            false,
            "Completed sequence must be suppressed on subsequent ticks"
        );

        // Sequence rollover / wrap-around (e.g. u32::MAX -> 0) is eligible
        let rollover = ClipboardSequenceState::new(u32::MAX);
        assert_eq!(
            rollover.is_eligible(0),
            true,
            "Sequence rollover from u32::MAX to 0 must be eligible"
        );
    }

    use std::sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    };
    use std::time::Duration;
    use windows::core::{BOOL, PCWSTR};
    use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::DataExchange::{
        EmptyClipboard, GetClipboardOwner, SetClipboardData,
    };
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
        GetWindowLongPtrW, PostMessageW, PostQuitMessage, RegisterClassExW, SetWindowLongPtrW,
        TranslateMessage, CREATESTRUCTW, GWLP_USERDATA, HWND_MESSAGE, MSG, WINDOW_EX_STYLE,
        WINDOW_STYLE, WM_CLOSE, WM_CREATE, WM_DESTROY, WM_RENDERFORMAT, WM_USER, WNDCLASSEXW,
    };

    #[link(name = "user32")]
    extern "system" {
        fn CreateWindowStationW(
            lpwinsta: *const u16,
            dwflags: u32,
            dwdesiredaccess: u32,
            lpsa: *const core::ffi::c_void,
        ) -> HANDLE;
        fn GetProcessWindowStation() -> HANDLE;
        fn SetProcessWindowStation(hwinsta: HANDLE) -> BOOL;
        fn CloseWindowStation(hwinsta: HANDLE) -> BOOL;
        fn CreateDesktopW(
            lpszdesktop: *const u16,
            lpszdevice: *const u16,
            pdevmode: *const core::ffi::c_void,
            dwflags: u32,
            dwdesiredaccess: u32,
            lpsa: *const core::ffi::c_void,
        ) -> HANDLE;
        fn GetThreadDesktop(dwthreadid: u32) -> HANDLE;
        fn SetThreadDesktop(hdesktop: HANDLE) -> BOOL;
        fn CloseDesktop(hdesktop: HANDLE) -> BOOL;
    }

    const CWF_CREATE_ONLY: u32 = 0x00000001;
    const WINSTA_ALL_ACCESS: u32 = 0x000F037F;
    const DESKTOP_ALL_ACCESS: u32 = 0x000F01FF;
    const WM_ARM_DELAYED_RENDER: u32 = WM_USER + 1;
    const WM_PUBLISH_PLAIN_TEXT: u32 = WM_USER + 2;

    static STATION_NONCE: AtomicU64 = AtomicU64::new(0);

    struct TestWindowStationIsolation {
        prev_winsta: HANDLE,
        new_winsta: HANDLE,
        prev_desk: HANDLE,
        new_desk: HANDLE,
    }

    impl TestWindowStationIsolation {
        fn new() -> Self {
            let pid = std::process::id();
            let nonce = STATION_NONCE.fetch_add(1, Ordering::SeqCst);
            let name_str = format!("ClovyClipboardTest-{pid}-{nonce}");
            let name_w: Vec<u16> = name_str.encode_utf16().chain(Some(0)).collect();

            unsafe {
                let prev_winsta = GetProcessWindowStation();
                assert!(
                    !prev_winsta.is_invalid() && !prev_winsta.0.is_null(),
                    "GetProcessWindowStation failed"
                );

                // Explicit unique name + CWF_CREATE_ONLY=1: never open or reuse existing station
                let new_winsta = CreateWindowStationW(
                    name_w.as_ptr(),
                    CWF_CREATE_ONLY,
                    WINSTA_ALL_ACCESS,
                    core::ptr::null(),
                );
                assert!(
                    !new_winsta.is_invalid() && !new_winsta.0.is_null(),
                    "CreateWindowStationW failed to create unique private station: {name_str}"
                );

                assert!(
                    SetProcessWindowStation(new_winsta).as_bool(),
                    "SetProcessWindowStation failed for {name_str}"
                );

                let prev_desk = GetThreadDesktop(GetCurrentThreadId());
                assert!(
                    !prev_desk.is_invalid() && !prev_desk.0.is_null(),
                    "GetThreadDesktop failed"
                );

                let desk_name_str = format!("ClovyDesk-{pid}-{nonce}");
                let desk_name_w: Vec<u16> = desk_name_str.encode_utf16().chain(Some(0)).collect();

                let new_desk = CreateDesktopW(
                    desk_name_w.as_ptr(),
                    core::ptr::null(),
                    core::ptr::null(),
                    0,
                    DESKTOP_ALL_ACCESS,
                    core::ptr::null(),
                );
                assert!(
                    !new_desk.is_invalid() && !new_desk.0.is_null(),
                    "CreateDesktopW failed for {desk_name_str}"
                );

                assert!(
                    SetThreadDesktop(new_desk).as_bool(),
                    "SetThreadDesktop failed for test thread on {desk_name_str}"
                );

                Self {
                    prev_winsta,
                    new_winsta,
                    prev_desk,
                    new_desk,
                }
            }
        }

        fn desktop_handle(&self) -> isize {
            self.new_desk.0 as isize
        }
    }

    impl Drop for TestWindowStationIsolation {
        fn drop(&mut self) {
            unsafe {
                if !self.new_desk.is_invalid() && !self.new_desk.0.is_null() {
                    if !self.prev_desk.is_invalid() && !self.prev_desk.0.is_null() {
                        let _ = SetThreadDesktop(self.prev_desk);
                    }
                    let _ = CloseDesktop(self.new_desk);
                }
                if !self.new_winsta.is_invalid() && !self.new_winsta.0.is_null() {
                    if !self.prev_winsta.is_invalid() && !self.prev_winsta.0.is_null() {
                        let _ = SetProcessWindowStation(self.prev_winsta);
                    }
                    let _ = CloseWindowStation(self.new_winsta);
                }
            }
        }
    }

    struct DelayedOwnerSharedState {
        render_format_received: AtomicBool,
        render_format_tx: Mutex<mpsc::Sender<()>>,
        unblock_rx: Mutex<mpsc::Receiver<()>>,
        arm_ack_tx: Mutex<mpsc::Sender<bool>>,
        publish_ack_tx: Mutex<mpsc::Sender<bool>>,
        late_text: Mutex<Option<String>>,
        publish_text: Mutex<Option<String>>,
        cleanup_timeout: Duration,
    }

    unsafe extern "system" fn delayed_owner_wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_CREATE => {
                let cs = lparam.0 as *const CREATESTRUCTW;
                if !cs.is_null() {
                    let state_ptr = (*cs).lpCreateParams as isize;
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr);
                }
                LRESULT(0)
            }
            WM_ARM_DELAYED_RENDER => {
                let open_ok = unsafe { OpenClipboard(Some(hwnd)).is_ok() };
                let empty_ok = if open_ok {
                    unsafe { EmptyClipboard().is_ok() }
                } else {
                    false
                };
                if empty_ok {
                    unsafe {
                        let _ = SetClipboardData(CF_UNICODETEXT, None);
                        let _ = CloseClipboard();
                    }
                } else if open_ok {
                    unsafe {
                        let _ = CloseClipboard();
                    }
                }
                let format_available =
                    unsafe { IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() };
                let owner_matches = unsafe { GetClipboardOwner().ok() == Some(hwnd) };
                let arm_succeeded = open_ok && empty_ok && format_available && owner_matches;

                let state_ptr =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const DelayedOwnerSharedState;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    if let Ok(tx) = state.arm_ack_tx.lock() {
                        let _ = tx.send(arm_succeeded);
                    }
                }
                LRESULT(0)
            }
            WM_RENDERFORMAT => {
                let state_ptr =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const DelayedOwnerSharedState;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    state.render_format_received.store(true, Ordering::SeqCst);
                    if let Ok(tx) = state.render_format_tx.lock() {
                        let _ = tx.send(());
                    }
                    // Wait for unblock signal (either explicit release or fixture teardown)
                    if let Ok(rx) = state.unblock_rx.lock() {
                        let _ = rx.recv_timeout(state.cleanup_timeout);
                    }
                    // If synthetic late text was configured, supply it upon release
                    let text_opt = state.late_text.lock().ok().and_then(|mut t| t.take());
                    if let Some(text) = text_opt {
                        let utf16: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                        let bytes_len = utf16.len() * std::mem::size_of::<u16>();
                        if let Ok(hmem) = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes_len) } {
                            let ptr = unsafe { GlobalLock(hmem) } as *mut u16;
                            if !ptr.is_null() {
                                unsafe {
                                    std::ptr::copy_nonoverlapping(utf16.as_ptr(), ptr, utf16.len());
                                    let _ = GlobalUnlock(hmem);
                                    let _ = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0)));
                                }
                            }
                        }
                    }
                }
                // Returning tells Windows delayed rendering has completed or failed
                LRESULT(0)
            }
            WM_PUBLISH_PLAIN_TEXT => {
                let state_ptr =
                    GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const DelayedOwnerSharedState;
                let mut success = false;
                if !state_ptr.is_null() {
                    let state = &*state_ptr;
                    let text_opt = state.publish_text.lock().ok().and_then(|mut t| t.take());
                    if let Some(text) = text_opt {
                        let open_ok = unsafe { OpenClipboard(Some(hwnd)).is_ok() };
                        if open_ok {
                            let _ = unsafe { EmptyClipboard() };
                            let utf16: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                            let bytes_len = utf16.len() * std::mem::size_of::<u16>();
                            if let Ok(hmem) = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes_len) } {
                                let ptr = unsafe { GlobalLock(hmem) } as *mut u16;
                                if !ptr.is_null() {
                                    unsafe {
                                        std::ptr::copy_nonoverlapping(
                                            utf16.as_ptr(),
                                            ptr,
                                            utf16.len(),
                                        );
                                        let _ = GlobalUnlock(hmem);
                                        let _ =
                                            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hmem.0)));
                                    }
                                    success = true;
                                }
                            }
                            let _ = unsafe { CloseClipboard() };
                        }
                    }
                    if let Ok(tx) = state.publish_ack_tx.lock() {
                        let _ = tx.send(success);
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    struct DelayedRenderFixture {
        station_isolation: TestWindowStationIsolation,
        hwnd: isize,
        owner_thread: Option<std::thread::JoinHandle<()>>,
        reader_thread: Option<std::thread::JoinHandle<()>>,
        unblock_tx: Option<mpsc::Sender<()>>,
        arm_ack_rx: mpsc::Receiver<bool>,
        publish_ack_rx: mpsc::Receiver<bool>,
        render_format_rx: mpsc::Receiver<()>,
        shared_state: Arc<DelayedOwnerSharedState>,
    }

    impl DelayedRenderFixture {
        fn new() -> Self {
            let station_isolation = TestWindowStationIsolation::new();
            let desk_handle = station_isolation.desktop_handle();

            let (unblock_tx, unblock_rx) = mpsc::channel();
            let (arm_ack_tx, arm_ack_rx) = mpsc::channel();
            let (publish_ack_tx, publish_ack_rx) = mpsc::channel();
            let (render_format_tx, render_format_rx) = mpsc::channel();
            let (ready_tx, ready_rx) = mpsc::channel();

            let shared_state = Arc::new(DelayedOwnerSharedState {
                render_format_received: AtomicBool::new(false),
                render_format_tx: Mutex::new(render_format_tx),
                unblock_rx: Mutex::new(unblock_rx),
                arm_ack_tx: Mutex::new(arm_ack_tx),
                publish_ack_tx: Mutex::new(publish_ack_tx),
                late_text: Mutex::new(None),
                publish_text: Mutex::new(None),
                cleanup_timeout: Duration::from_secs(3),
            });

            let shared_state_clone = Arc::clone(&shared_state);
            let owner_thread = std::thread::spawn(move || {
                let hdesk = HANDLE(desk_handle as *mut core::ffi::c_void);
                assert!(
                    unsafe { SetThreadDesktop(hdesk).as_bool() },
                    "SetThreadDesktop failed for owner thread"
                );

                let class_name = w!("ClovyTestDelayedClipboardOwner");
                let h_instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };

                let mut wc = WNDCLASSEXW::default();
                wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
                wc.lpfnWndProc = Some(delayed_owner_wndproc);
                wc.hInstance = h_instance.into();
                wc.lpszClassName = class_name;

                unsafe {
                    let _ = RegisterClassExW(&wc);
                }

                let raw_ptr = Arc::into_raw(shared_state_clone);
                let hwnd_res = unsafe {
                    CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        class_name,
                        PCWSTR::null(),
                        WINDOW_STYLE(0),
                        0,
                        0,
                        0,
                        0,
                        Some(HWND_MESSAGE),
                        None,
                        Some(h_instance.into()),
                        Some(raw_ptr as *const core::ffi::c_void),
                    )
                };

                let hwnd = match hwnd_res {
                    Ok(h) => h,
                    Err(_) => {
                        let _ = unsafe { Arc::from_raw(raw_ptr) };
                        let _ = ready_tx.send(0);
                        return;
                    }
                };

                let _ = ready_tx.send(hwnd.0 as isize);

                let mut msg = MSG::default();
                while unsafe { GetMessageW(&mut msg, None, 0, 0).0 > 0 } {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }

                let _ = unsafe { Arc::from_raw(raw_ptr) };
            });

            let hwnd = ready_rx.recv_timeout(Duration::from_secs(2)).unwrap_or(0);
            assert!(hwnd != 0, "failed to create delayed-render owner window");

            Self {
                station_isolation,
                hwnd,
                owner_thread: Some(owner_thread),
                reader_thread: None,
                unblock_tx: Some(unblock_tx),
                arm_ack_rx,
                publish_ack_rx,
                render_format_rx,
                shared_state,
            }
        }

        fn arm(&mut self) {
            let hwnd = HWND(self.hwnd as *mut core::ffi::c_void);
            unsafe {
                assert!(
                    PostMessageW(Some(hwnd), WM_ARM_DELAYED_RENDER, WPARAM(0), LPARAM(0)).is_ok(),
                    "PostMessageW WM_ARM_DELAYED_RENDER failed"
                );
            }
            let ack = self
                .arm_ack_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("timed out waiting for native arm ack");
            assert!(
                ack,
                "Native clipboard arm failed: Open/Empty/format available/owner=fixture check was not satisfied"
            );
        }

        fn set_late_text(&self, text: &str) {
            if let Ok(mut lock) = self.shared_state.late_text.lock() {
                *lock = Some(text.to_string());
            }
        }

        fn release_late_render(&mut self) {
            if let Some(tx) = self.unblock_tx.take() {
                let _ = tx.send(());
            }
        }

        fn publish_plain_text(&mut self, text: &str) {
            if let Ok(mut lock) = self.shared_state.publish_text.lock() {
                *lock = Some(text.to_string());
            }
            let hwnd = HWND(self.hwnd as *mut core::ffi::c_void);
            unsafe {
                assert!(
                    PostMessageW(Some(hwnd), WM_PUBLISH_PLAIN_TEXT, WPARAM(0), LPARAM(0)).is_ok(),
                    "PostMessageW WM_PUBLISH_PLAIN_TEXT failed"
                );
            }
            let ack = self
                .publish_ack_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("timed out waiting for publish plain text ack");
            assert!(ack, "publish_plain_text failed to set clipboard data");
        }

        fn wait_clipboard_released(&self) {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            loop {
                if unsafe { OpenClipboard(None).is_ok() } {
                    unsafe { CloseClipboard().unwrap() };
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "native reader did not close private clipboard"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        fn desktop(&self) -> isize {
            self.station_isolation.desktop_handle()
        }

        fn set_reader_handle(&mut self, handle: std::thread::JoinHandle<()>) {
            self.reader_thread = Some(handle);
        }

        fn wait_render_format_received(&self, timeout: Duration) -> bool {
            self.render_format_rx.recv_timeout(timeout).is_ok()
        }

        fn is_render_format_received(&self) -> bool {
            self.shared_state
                .render_format_received
                .load(Ordering::SeqCst)
        }
    }

    impl Drop for DelayedRenderFixture {
        fn drop(&mut self) {
            // 1. Unblock delayed-render owner so any pending GetClipboardData returns immediately
            drop(self.unblock_tx.take());

            // 2. Wait for reader thread to finish and join it (preventing thread leaks)
            if let Some(reader) = self.reader_thread.take() {
                let _ = reader.join();
            }

            // 3. Close owner window and exit owner thread message loop
            if self.hwnd != 0 {
                let hwnd = HWND(self.hwnd as *mut core::ffi::c_void);
                unsafe {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }

            // 4. Join owner thread (preventing thread/window leaks)
            if let Some(owner) = self.owner_thread.take() {
                let _ = owner.join();
            }
        }
    }

    #[test]
    fn test_changed_text_bounded_on_unresponsive_delayed_render_owner() {
        if std::env::var("CLOVY_TEST_ISOLATED_CLIPBOARD").is_err() {
            let exe = std::env::current_exe().expect("current_exe");
            let output = std::process::Command::new(exe)
                .arg("--exact")
                .arg("activity::windows::clipboard::tests::test_changed_text_bounded_on_unresponsive_delayed_render_owner")
                .arg("--nocapture")
                .env("CLOVY_TEST_ISOLATED_CLIPBOARD", "1")
                .output()
                .expect("failed to spawn isolated test child process");

            assert!(
                output.status.success(),
                "Isolated test child failed with status {:?}:\nSTDOUT:\n{}\nSTDERR:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        // Child process execution begins here (isolated from parent suite WinSta):
        let mut fixture = DelayedRenderFixture::new();
        let desk = fixture.desktop();
        let (tx, rx) = mpsc::channel();

        let (ready_tx, ready_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();

        let reader_handle = std::thread::spawn(move || {
            let hdesk = HANDLE(desk as *mut core::ffi::c_void);
            assert!(
                unsafe { SetThreadDesktop(hdesk).as_bool() },
                "SetThreadDesktop failed for reader thread"
            );
            // Construct Clipboard only AFTER reader desktop is attached to private desktop
            let mut clipboard = Clipboard::new();
            ready_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            let started = std::time::Instant::now();
            let res = clipboard.changed_text();
            // Shutdown must not join the still-blocked native read either.
            drop(clipboard);
            let _ = tx.send((res, started.elapsed()));
        });
        fixture.set_reader_handle(reader_handle);
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader setup");
        fixture.arm();
        go_tx.send(()).unwrap();

        // Assert WM_RENDERFORMAT is actually received via bounded channel,
        // avoiding busy/no-change false-green.
        assert!(
            fixture.wait_render_format_received(Duration::from_millis(250)),
            "delayed-render owner must actually receive WM_RENDERFORMAT; avoiding busy/no-change false-green"
        );
        assert!(
            fixture.is_render_format_received(),
            "render_format_received atomic flag must be true"
        );

        // Assert capture-facing call returns boundedly
        let timeout = Duration::from_millis(250);
        let outcome = rx.recv_timeout(timeout);

        assert!(
            outcome.is_ok(),
            "changed_text() must return boundedly (within {:?}) when delayed-render owner does not respond to WM_RENDERFORMAT, but timed out",
            timeout
        );
        let (_, elapsed) = outcome.unwrap();
        assert!(elapsed < timeout, "capture-facing read took {elapsed:?}");
    }

    #[test]
    fn test_changed_text_discards_stale_delayed_render_and_recovers_on_fresh_sequence() {
        if std::env::var("CLOVY_TEST_ISOLATED_CLIPBOARD_STALE").is_err() {
            let exe = std::env::current_exe().expect("current_exe");
            let output = std::process::Command::new(exe)
                .arg("--exact")
                .arg("activity::windows::clipboard::tests::test_changed_text_discards_stale_delayed_render_and_recovers_on_fresh_sequence")
                .arg("--nocapture")
                .env("CLOVY_TEST_ISOLATED_CLIPBOARD_STALE", "1")
                .output()
                .expect("failed to spawn isolated test child process");

            assert!(
                output.status.success(),
                "Isolated test child failed with status {:?}:\nSTDOUT:\n{}\nSTDERR:\n{}",
                output.status,
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        // Child process execution begins here (isolated from parent suite WinSta):
        let mut fixture = DelayedRenderFixture::new();
        let desk = fixture.desktop();
        fixture.set_late_text("late_stale_text");

        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        let reader_handle = std::thread::spawn(move || {
            let hdesk = HANDLE(desk as *mut core::ffi::c_void);
            assert!(
                unsafe { SetThreadDesktop(hdesk).as_bool() },
                "SetThreadDesktop failed for reader thread"
            );
            // Construct Clipboard only AFTER reader desktop is attached to private desktop
            let mut clipboard = Clipboard::new();
            ready_tx.send(()).unwrap();
            go_rx.recv().unwrap();
            let started = std::time::Instant::now();
            let res = clipboard.changed_text();
            let _ = tx.send((clipboard, res, started.elapsed()));
        });
        fixture.set_reader_handle(reader_handle);
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader setup");
        fixture.arm();
        go_tx.send(()).unwrap();

        // Prove WM_RENDERFORMAT was actually received by fixture owner
        assert!(
            fixture.wait_render_format_received(Duration::from_millis(250)),
            "delayed-render owner must actually receive WM_RENDERFORMAT; avoiding busy/no-change false-green"
        );
        assert!(
            fixture.is_render_format_received(),
            "render_format_received atomic flag must be true"
        );

        // 1. Read caller must return boundedly before release (current sync fails here semantically)
        let timeout = Duration::from_millis(250);
        let outcome = rx.recv_timeout(timeout);

        assert!(
            outcome.is_ok(),
            "first read must return boundedly before delayed render is released, but timed out",
        );
        let (mut clipboard, first_res, elapsed) = outcome.unwrap();
        assert!(elapsed < timeout, "capture-facing read took {elapsed:?}");
        assert_eq!(
            first_res, None,
            "first read that timed out before release must return None"
        );

        // 2. Now explicitly release owner with late synthetic text
        fixture.release_late_render();
        fixture.wait_clipboard_released();

        // 3. Subsequent changed_text calls after release must NOT emit the late value!
        let post_release_res = clipboard.changed_text();
        assert_ne!(
            post_release_res.as_deref(),
            Some("late_stale_text"),
            "subsequent changed_text call must not emit stale late value"
        );

        // 4. A separately published fresh sequence must recover normally
        fixture.publish_plain_text("fresh_recovered_text");
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(recovered) = clipboard.changed_text() {
                assert_eq!(recovered, "fresh_recovered_text");
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "fresh sequence never recovered"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn test_changed_text_preserves_unread_replacement_after_expired_request() {
        if std::env::var("CLOVY_TEST_CLIPBOARD_REPLACEMENT").is_err() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "activity::windows::clipboard::tests::test_changed_text_preserves_unread_replacement_after_expired_request",
                    "--nocapture",
                ])
                .env("CLOVY_TEST_CLIPBOARD_REPLACEMENT", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "isolated replacement test failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        let mut fixture = DelayedRenderFixture::new();
        let mut clipboard = Clipboard::new();
        let sequence_a = unsafe { GetClipboardSequenceNumber() };
        fixture.publish_plain_text("replacement_B_unread");

        // Isolate the expiry transition: A times out before the worker opens B.
        // The existing native delayed-render tests cover the timeout itself.
        let response = process_worker_request(&WorkerRequest {
            epoch: 1,
            requested_sequence: sequence_a,
        });
        assert_eq!(response.status, WorkerStatus::ChangedBeforeRead);
        assert!(response.text.is_none(), "replacement was never read");
        clipboard.active_epoch = 1;
        clipboard.pending_epoch = Some(1);
        clipboard.discard_expired_response(response);

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(text) = clipboard.changed_text() {
                assert_eq!(text, "replacement_B_unread");
                break;
            }
            assert!(
                Instant::now() < deadline,
                "unread replacement was suppressed"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
