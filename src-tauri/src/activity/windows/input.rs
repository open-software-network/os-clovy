//! Passive, privacy-preserving input monitoring on Windows using Raw Input.
//!
//! Captures only event timings for key-down and mouse-clicks without ever
//! reading virtual keys, scancodes, or typed characters.

use std::collections::VecDeque;
use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use chrono::{DateTime, Utc};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
    RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEKEYBOARD, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, PostMessageW, PostQuitMessage, RegisterClassExW, SetWindowLongPtrW,
    TranslateMessage, CREATESTRUCTW, GWLP_USERDATA, HWND_MESSAGE, MSG, RI_KEY_BREAK,
    RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_LEFT_BUTTON_DOWN,
    RI_MOUSE_MIDDLE_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_DOWN, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLOSE, WM_CREATE, WM_DESTROY, WM_INPUT, WNDCLASSEXW,
};

use crate::activity::platform::RawInputEvent;

const MAX_BUFFERED_EVENTS: usize = 10_000;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum HandshakeOutcome {
    Proceed,
    Abandoned,
}

/// Production publish + ACK handshake executed by the window creation thread.
/// Publishes the window handle to the parent, then waits for the parent's ACK.
/// If sending fails, or if the parent times out/abandons without sending ACK,
/// returns `HandshakeOutcome::Abandoned`, signaling that the window must be destroyed
/// and the message loop must NOT be entered.
pub(crate) fn worker_publish_and_wait_ack(
    hwnd_val: isize,
    hwnd_tx: &std::sync::mpsc::SyncSender<isize>,
    ack_rx: &std::sync::mpsc::Receiver<()>,
    timeout: Duration,
) -> HandshakeOutcome {
    if hwnd_tx.send(hwnd_val).is_err() {
        return HandshakeOutcome::Abandoned;
    }

    match ack_rx.recv_timeout(timeout) {
        Ok(()) => HandshakeOutcome::Proceed,
        Err(_) => HandshakeOutcome::Abandoned,
    }
}

/// Production parent side of the window creation handshake.
/// Receives the HWND and sends an ACK to authorize the worker to enter GetMessageW.
pub(crate) fn parent_await_hwnd_and_ack(
    hwnd_rx: &std::sync::mpsc::Receiver<isize>,
    ack_tx: &std::sync::mpsc::SyncSender<()>,
    timeout: Duration,
) -> Option<isize> {
    let hwnd_raw = hwnd_rx.recv_timeout(timeout).ok()?;
    let _ = ack_tx.send(());
    Some(hwnd_raw)
}

/// Normalizes raw keyboard input flags into a Key event.
/// Strictly uses only key-down/up flags; NEVER inspects virtual keys, scancodes, or characters.
pub(crate) fn normalize_keyboard_input(flags: u16, at: DateTime<Utc>) -> Option<RawInputEvent> {
    if (flags as u32 & RI_KEY_BREAK) == 0 {
        Some(RawInputEvent::Key {
            at,
            characters: None,
        })
    } else {
        None
    }
}

/// Normalizes raw mouse button flags into a Click event.
/// Strictly uses only button flags; NEVER inspects coordinates or motion deltas.
pub(crate) fn normalize_mouse_input(button_flags: u16, at: DateTime<Utc>) -> Option<RawInputEvent> {
    let btn = button_flags as u32;
    if (btn
        & (RI_MOUSE_LEFT_BUTTON_DOWN
            | RI_MOUSE_RIGHT_BUTTON_DOWN
            | RI_MOUSE_MIDDLE_BUTTON_DOWN
            | RI_MOUSE_BUTTON_4_DOWN
            | RI_MOUSE_BUTTON_5_DOWN))
        != 0
    {
        Some(RawInputEvent::Click { at })
    } else {
        None
    }
}

/// Production buffer state used by the RawInput window callback.
#[derive(Debug, Default)]
pub(crate) struct InputBuffer {
    enabled: bool,
    buffer: VecDeque<RawInputEvent>,
}

impl InputBuffer {
    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.buffer.clear();
        }
    }

    pub(crate) fn record_event(&mut self, event: RawInputEvent) {
        if self.enabled {
            if self.buffer.len() >= MAX_BUFFERED_EVENTS {
                self.buffer.pop_front();
            }
            self.buffer.push_back(event);
        }
    }

    pub(crate) fn drain(&mut self) -> Vec<RawInputEvent> {
        self.buffer.drain(..).collect()
    }
}

pub(crate) struct InputCapture {
    hwnd: Option<HWND>,
    thread: Option<JoinHandle<()>>,
    buffer: Arc<Mutex<InputBuffer>>,
}

impl Default for InputCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl InputCapture {
    pub(crate) fn new() -> Self {
        let buffer = Arc::new(Mutex::new(InputBuffer::default()));
        let thread_buffer = Arc::clone(&buffer);
        let (hwnd_tx, hwnd_rx) = sync_channel::<isize>(1);
        let (ack_tx, ack_rx) = sync_channel::<()>(1);

        let thread = std::thread::Builder::new()
            .name("clovy-raw-input".into())
            .spawn(move || {
                run_message_loop(thread_buffer, hwnd_tx, ack_rx);
            })
            .ok();

        let hwnd_raw = parent_await_hwnd_and_ack(&hwnd_rx, &ack_tx, Duration::from_secs(2));
        let hwnd = hwnd_raw.map(|raw| HWND(raw as *mut core::ffi::c_void));

        Self {
            hwnd,
            thread,
            buffer,
        }
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool) {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.set_enabled(enabled);
        }
    }

    pub(crate) fn drain(&mut self) -> Vec<RawInputEvent> {
        if let Ok(mut buf) = self.buffer.lock() {
            buf.drain()
        } else {
            Vec::new()
        }
    }
}

impl Drop for InputCapture {
    fn drop(&mut self) {
        if let Some(hwnd) = self.hwnd {
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_message_loop(
    buffer: Arc<Mutex<InputBuffer>>,
    hwnd_tx: std::sync::mpsc::SyncSender<isize>,
    ack_rx: std::sync::mpsc::Receiver<()>,
) {
    let class_name = w!("ClovyRawInputClass");
    let h_instance = unsafe { GetModuleHandleW(None).unwrap_or_default() };

    let mut wc = WNDCLASSEXW::default();
    wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
    wc.lpfnWndProc = Some(raw_input_wndproc);
    wc.hInstance = h_instance.into();
    wc.lpszClassName = class_name;

    unsafe {
        RegisterClassExW(&wc);
    }

    let raw_ptr = Arc::into_raw(buffer);

    let hwnd = unsafe {
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

    if hwnd.is_err() {
        let _ = unsafe { Arc::from_raw(raw_ptr) };
        return;
    }
    let hwnd = hwnd.unwrap();

    let devices = [
        RAWINPUTDEVICE {
            usUsagePage: 1, // Generic Desktop Controls
            usUsage: 2,     // Mouse
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
        RAWINPUTDEVICE {
            usUsagePage: 1, // Generic Desktop Controls
            usUsage: 6,     // Keyboard
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        },
    ];

    let registered = unsafe {
        RegisterRawInputDevices(&devices, std::mem::size_of::<RAWINPUTDEVICE>() as u32).is_ok()
    };

    if !registered {
        let _ = unsafe { DestroyWindow(hwnd) };
        let _ = unsafe { Arc::from_raw(raw_ptr) };
        return;
    }

    // Publish HWND and wait for parent ACK.
    // If sending fails or parent abandons, destroy window and exit without entering message loop.
    let outcome =
        worker_publish_and_wait_ack(hwnd.0 as isize, &hwnd_tx, &ack_rx, Duration::from_secs(2));
    if outcome == HandshakeOutcome::Abandoned {
        let _ = unsafe { DestroyWindow(hwnd) };
        let _ = unsafe { Arc::from_raw(raw_ptr) };
        return;
    }

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&mut msg, None, 0, 0).as_bool() } {
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _ = unsafe { Arc::from_raw(raw_ptr) };
}

unsafe extern "system" fn raw_input_wndproc(
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
        WM_INPUT => {
            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Mutex<InputBuffer>;
            if !state_ptr.is_null() {
                if let Ok(mut buffer) = (&*state_ptr).lock() {
                    if buffer.enabled {
                        let mut raw = RAWINPUT::default();
                        let mut size = std::mem::size_of::<RAWINPUT>() as u32;
                        let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;
                        let result = GetRawInputData(
                            HRAWINPUT(lparam.0 as *mut core::ffi::c_void),
                            RID_INPUT,
                            Some(&mut raw as *mut RAWINPUT as *mut core::ffi::c_void),
                            &mut size,
                            header_size,
                        );
                        if result != u32::MAX && result > 0 {
                            let event = if raw.header.dwType == RIM_TYPEKEYBOARD.0 {
                                normalize_keyboard_input(raw.data.keyboard.Flags, Utc::now())
                            } else if raw.header.dwType == RIM_TYPEMOUSE.0 {
                                normalize_mouse_input(
                                    raw.data.mouse.Anonymous.Anonymous.usButtonFlags,
                                    Utc::now(),
                                )
                            } else {
                                None
                            };
                            if let Some(event) = event {
                                buffer.record_event(event);
                            }
                        }
                    }
                }
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_input_keyboard_normalization_and_privacy() {
        let now = Utc::now();

        // Key-down (RI_KEY_MAKE = 0, no RI_KEY_BREAK bit): MUST return Key with characters: None
        let down_event = normalize_keyboard_input(0, now);
        assert!(
            down_event.is_some(),
            "Key-down must produce a RawInputEvent::Key"
        );
        match down_event.unwrap() {
            RawInputEvent::Key { at, characters } => {
                assert_eq!(at, now);
                assert!(
                    characters.is_none(),
                    "Windows RawInput MUST NEVER record virtualkeys, scancodes or characters"
                );
            }
            _ => panic!("Expected RawInputEvent::Key"),
        }

        // Key-up (RI_KEY_BREAK = 1): MUST return None
        let up_event = normalize_keyboard_input(1, now);
        assert_eq!(
            up_event, None,
            "Key-up events must be discarded by normalizer"
        );
    }

    #[test]
    fn test_raw_input_mouse_normalization() {
        let now = Utc::now();

        // Mouse-down (RI_MOUSE_LEFT_BUTTON_DOWN = 1): MUST return Click with at
        let click_event = normalize_mouse_input(1, now);
        assert!(
            click_event.is_some(),
            "Mouse button down must produce a RawInputEvent::Click"
        );
        match click_event.unwrap() {
            RawInputEvent::Click { at } => {
                assert_eq!(at, now);
            }
            _ => panic!("Expected RawInputEvent::Click"),
        }

        // Mouse-up (RI_MOUSE_LEFT_BUTTON_UP = 2): MUST return None
        let up_event = normalize_mouse_input(2, now);
        assert_eq!(
            up_event, None,
            "Mouse button up events must be discarded by normalizer"
        );

        // Mouse move without buttons (0): MUST return None
        let move_event = normalize_mouse_input(0, now);
        assert_eq!(
            move_event, None,
            "Mouse movement without clicks must be discarded by normalizer"
        );
    }

    #[test]
    fn test_raw_input_buffer_lifecycle_and_discard() {
        let mut buffer = InputBuffer::default();
        buffer.set_enabled(true);

        let t1 = Utc::now();
        let event1 = RawInputEvent::Key {
            at: t1,
            characters: None,
        };
        buffer.record_event(event1);

        // Disabling without draining must discard event1
        buffer.set_enabled(false);

        // Re-enabling and recording event2
        buffer.set_enabled(true);
        let t2 = Utc::now();
        let event2 = RawInputEvent::Click { at: t2 };
        buffer.record_event(event2.clone());

        let drained = buffer.drain();
        assert_eq!(
            drained,
            vec![event2],
            "Drained events must match EXACTLY [ClickT2] recorded after re-enable"
        );
    }

    #[test]
    fn test_input_handshake_positive_ack() {
        let (hwnd_tx, hwnd_rx) = sync_channel::<isize>(1);
        let (ack_tx, ack_rx) = sync_channel::<()>(1);

        // Worker publishes HWND
        let worker_handle = std::thread::spawn(move || {
            worker_publish_and_wait_ack(4242, &hwnd_tx, &ack_rx, Duration::from_secs(1))
        });

        // Parent receives HWND and sends ACK
        let parent_received = parent_await_hwnd_and_ack(&hwnd_rx, &ack_tx, Duration::from_secs(1));
        assert_eq!(parent_received, Some(4242));

        let outcome = worker_handle.join().expect("join");
        assert_eq!(
            outcome,
            HandshakeOutcome::Proceed,
            "Worker must proceed when positive ACK is received"
        );
    }

    #[test]
    fn test_input_handshake_abandonment_on_late_or_missing_ack() {
        let (hwnd_tx, hwnd_rx) = sync_channel::<isize>(1);
        let (ack_tx, ack_rx) = sync_channel::<()>(1);

        // Worker publishes HWND
        let worker_handle = std::thread::spawn(move || {
            worker_publish_and_wait_ack(9999, &hwnd_tx, &ack_rx, Duration::from_millis(50))
        });

        // Parent receives HWND but times out / drops ACK channel without sending ACK
        let hwnd = hwnd_rx.recv().expect("recv hwnd");
        assert_eq!(hwnd, 9999);
        drop(ack_tx);

        let outcome = worker_handle.join().expect("join");
        assert_eq!(
            outcome,
            HandshakeOutcome::Abandoned,
            "Worker must detect abandonment when parent drops ACK channel"
        );
    }

    #[test]
    fn test_input_handshake_send_failure_on_dropped_receiver() {
        let (hwnd_tx, hwnd_rx) = sync_channel::<isize>(1);
        let (_ack_tx, ack_rx) = sync_channel::<()>(1);

        // Parent drops hwnd_rx before worker sends
        drop(hwnd_rx);

        let outcome =
            worker_publish_and_wait_ack(5555, &hwnd_tx, &ack_rx, Duration::from_millis(50));
        assert_eq!(
            outcome,
            HandshakeOutcome::Abandoned,
            "Worker must immediately abandon when HWND channel is dropped"
        );
    }
}
