//! Passive, privacy-preserving input monitoring event tap.
//!
//! Captures only event timings for key-down and mouse-clicks without ever
//! reading keycodes, scancodes, or typed characters.

use chrono::Utc;
use core_foundation::runloop::kCFRunLoopCommonModes;
use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::activity::platform::RawInputEvent;

type CFMachPortRef = *mut c_void;
type CFRunLoopRef = *mut c_void;
type CFRunLoopSourceRef = *mut c_void;
type CGEventRef = *mut c_void;
type CGEventTapProxy = *mut c_void;

const K_CG_SESSION_EVENT_TAP: u32 = 1;
const K_CG_HEAD_INSERT_EVENT_TAP: u32 = 0;
const K_CG_EVENT_TAP_OPTION_LISTEN_ONLY: u32 = 1;

const K_CG_EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const K_CG_EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const K_CG_EVENT_KEY_DOWN: u32 = 10;
const K_CG_EVENT_OTHER_MOUSE_DOWN: u32 = 25;
const K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;

const MAX_BUFFERED_EVENTS: usize = 10_000;

type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        eventsOfInterest: u64,
        callback: CGEventTapCallBack,
        userInfo: *mut c_void,
    ) -> CFMachPortRef;

    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: *const c_void);
    fn CFRunLoopRun();
    fn CFRunLoopStop(rl: CFRunLoopRef);
    fn CFRelease(cf: *const c_void);
}

struct TapContext {
    buffer: Arc<Mutex<VecDeque<RawInputEvent>>>,
    mach_port: AtomicPtr<c_void>,
}

unsafe extern "C" fn event_tap_callback(
    _proxy: CGEventTapProxy,
    event_type: u32,
    event: CGEventRef,
    user_info: *mut c_void,
) -> CGEventRef {
    if user_info.is_null() {
        return event;
    }
    let ctx = &*(user_info as *const TapContext);
    let now = Utc::now();

    match event_type {
        K_CG_EVENT_KEY_DOWN => {
            // Privacy by construction: never read keycodes, characters, or modifiers
            let mut q = match ctx.buffer.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if q.len() >= MAX_BUFFERED_EVENTS {
                q.pop_front();
            }
            q.push_back(RawInputEvent::Key {
                at: now,
                characters: None,
            });
        }
        K_CG_EVENT_LEFT_MOUSE_DOWN | K_CG_EVENT_RIGHT_MOUSE_DOWN | K_CG_EVENT_OTHER_MOUSE_DOWN => {
            let mut q = match ctx.buffer.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if q.len() >= MAX_BUFFERED_EVENTS {
                q.pop_front();
            }
            q.push_back(RawInputEvent::Click { at: now });
        }
        K_CG_EVENT_TAP_DISABLED_BY_TIMEOUT => {
            let port = ctx.mach_port.load(Ordering::SeqCst);
            if !port.is_null() {
                CGEventTapEnable(port, true);
            }
        }
        _ => {}
    }

    event
}

/// State and thread controller for the input event tap.
pub struct InputTapController {
    buffer: Arc<Mutex<VecDeque<RawInputEvent>>>,
    context: Arc<TapContext>,
    run_loop: Arc<AtomicPtr<c_void>>,
    thread_handle: Option<JoinHandle<()>>,
    enabled: Arc<AtomicBool>,
}

impl InputTapController {
    /// Creates and spawns the dedicated event tap thread.
    pub fn start(buffer: Arc<Mutex<VecDeque<RawInputEvent>>>) -> Option<Self> {
        let context = Arc::new(TapContext {
            buffer: Arc::clone(&buffer),
            mach_port: AtomicPtr::new(std::ptr::null_mut()),
        });
        let run_loop = Arc::new(AtomicPtr::new(std::ptr::null_mut()));
        let enabled = Arc::new(AtomicBool::new(true));

        let thread_ctx = Arc::clone(&context);
        let thread_rl = Arc::clone(&run_loop);
        let thread_enabled = Arc::clone(&enabled);

        let (init_tx, init_rx) = std::sync::mpsc::channel();

        let thread_handle = std::thread::Builder::new()
            .name("clovy-activity-input-tap".to_string())
            .spawn(move || {
                let rl = unsafe { CFRunLoopGetCurrent() };
                thread_rl.store(rl, Ordering::SeqCst);

                let events_of_interest: u64 = (1u64 << K_CG_EVENT_LEFT_MOUSE_DOWN)
                    | (1u64 << K_CG_EVENT_RIGHT_MOUSE_DOWN)
                    | (1u64 << K_CG_EVENT_OTHER_MOUSE_DOWN)
                    | (1u64 << K_CG_EVENT_KEY_DOWN);

                let ctx_ptr = Arc::as_ptr(&thread_ctx) as *mut c_void;

                let mach_port = unsafe {
                    CGEventTapCreate(
                        K_CG_SESSION_EVENT_TAP,
                        K_CG_HEAD_INSERT_EVENT_TAP,
                        K_CG_EVENT_TAP_OPTION_LISTEN_ONLY,
                        events_of_interest,
                        event_tap_callback,
                        ctx_ptr,
                    )
                };

                if mach_port.is_null() {
                    let _ = init_tx.send(false);
                    return;
                }

                thread_ctx.mach_port.store(mach_port, Ordering::SeqCst);

                let source =
                    unsafe { CFMachPortCreateRunLoopSource(std::ptr::null(), mach_port, 0) };

                if source.is_null() {
                    unsafe {
                        CFRelease(mach_port);
                    }
                    let _ = init_tx.send(false);
                    return;
                }

                unsafe {
                    CFRunLoopAddSource(rl, source, kCFRunLoopCommonModes as *const c_void);
                    CGEventTapEnable(mach_port, thread_enabled.load(Ordering::SeqCst));
                }

                let _ = init_tx.send(true);

                // Run the thread's CFRunLoop
                unsafe {
                    CFRunLoopRun();
                }

                // Cleanup on run loop exit
                unsafe {
                    CGEventTapEnable(mach_port, false);
                    CFRelease(source);
                    CFRelease(mach_port);
                }
                thread_ctx
                    .mach_port
                    .store(std::ptr::null_mut(), Ordering::SeqCst);
            })
            .ok()?;

        let Ok(true) = init_rx.recv_timeout(std::time::Duration::from_secs(2)) else {
            return None;
        };

        Some(Self {
            buffer,
            context,
            run_loop,
            thread_handle: Some(thread_handle),
            enabled,
        })
    }

    /// Toggles the event tap active state.
    pub fn set_enabled(&self, enable: bool) {
        self.enabled.store(enable, Ordering::SeqCst);
        let port = self.context.mach_port.load(Ordering::SeqCst);
        if !port.is_null() {
            unsafe {
                CGEventTapEnable(port, enable);
            }
        }
    }

    /// Drains all accumulated input events and empties the buffer.
    pub fn drain(&self) -> Vec<RawInputEvent> {
        let mut q = match self.buffer.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        q.drain(..).collect()
    }
}

impl Drop for InputTapController {
    fn drop(&mut self) {
        self.set_enabled(false);
        let rl = self.run_loop.load(Ordering::SeqCst);
        if !rl.is_null() {
            unsafe {
                CFRunLoopStop(rl);
            }
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_capacity_cap() {
        let buffer = Arc::new(Mutex::new(VecDeque::new()));
        let ctx = TapContext {
            buffer: Arc::clone(&buffer),
            mach_port: AtomicPtr::new(std::ptr::null_mut()),
        };

        let now = Utc::now();
        for _ in 0..(MAX_BUFFERED_EVENTS + 50) {
            let mut q = match ctx.buffer.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if q.len() >= MAX_BUFFERED_EVENTS {
                q.pop_front();
            }
            q.push_back(RawInputEvent::Click { at: now });
        }

        let q = match buffer.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        assert_eq!(q.len(), MAX_BUFFERED_EVENTS);
    }
}
