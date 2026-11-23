//! Narrow exact-window Stage Manager bridge. No app-wide activation and no
//! Space-follow fallback. The broker verifies the resulting window geometry.
#[cfg(target_os = "macos")]
mod macos {
    use core_foundation::string::CFStringRef;
    use serde_json::{json, Value};
    use std::{
        ffi::{c_void, CStr},
        ptr,
    };
    type Ax = *const c_void;
    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXUIElementCreateApplication(pid: i32) -> Ax;
        fn AXUIElementCopyAttributeValue(
            element: Ax,
            attribute: CFStringRef,
            value: *mut Ax,
        ) -> i32;
        fn AXUIElementPerformAction(element: Ax, action: Ax) -> i32;
        fn AXUIElementSetMessagingTimeout(element: Ax, timeout: f32) -> i32;
        fn _AXUIElementGetWindow(element: Ax, window: *mut u32) -> i32;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: Ax);
        fn CFStringCreateWithCString(allocator: Ax, text: *const i8, encoding: u32) -> Ax;
        fn CFArrayGetCount(array: Ax) -> isize;
        fn CFArrayGetValueAtIndex(array: Ax, index: isize) -> Ax;
    }
    struct Owned(Ax);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CFRelease(self.0) }
            }
        }
    }
    unsafe fn string(value: &CStr) -> Owned {
        Owned(unsafe { CFStringCreateWithCString(ptr::null(), value.as_ptr(), 0x08000100) })
    }
    unsafe fn symbol(name: &CStr) -> *mut c_void {
        unsafe {
            libc::dlopen(
                c"/System/Library/PrivateFrameworks/SkyLight.framework/SkyLight".as_ptr(),
                libc::RTLD_LAZY | libc::RTLD_GLOBAL,
            );
            libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr())
        }
    }
    fn process_matches(pid: i32, expected: &str) -> bool {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
        let count = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if count != size {
            return false;
        }
        let info = unsafe { info.assume_init() };
        info.pbi_pid == pid as u32
            && format!(
                "macos-proc-v1:{}:{}",
                info.pbi_start_tvsec, info.pbi_start_tvusec
            ) == expected
    }
    pub(crate) fn join(pid: i32, window_id: u32, expected_process: &str) -> Value {
        if !process_matches(pid, expected_process) {
            return super::super::stale_target();
        }
        // Acquire the matching AX window before changing focus; no process-wide
        // fallback is allowed if this exact WindowServer window has no AX peer.
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        if app.0.is_null() {
            return super::super::failure(
                "computer_use_stage_unavailable",
                "The exact app window is unavailable to Accessibility.",
                false,
            );
        }
        unsafe {
            AXUIElementSetMessagingTimeout(app.0, 1.0);
        }
        let attribute = unsafe { string(c"AXWindows") };
        let mut windows = ptr::null();
        if attribute.0.is_null()
            || unsafe { AXUIElementCopyAttributeValue(app.0, attribute.0.cast(), &mut windows) }
                != 0
            || windows.is_null()
        {
            return super::super::failure(
                "computer_use_stage_unavailable",
                "The exact app window is unavailable to Accessibility.",
                false,
            );
        }
        let windows = Owned(windows);
        let mut exact = ptr::null();
        for i in 0..unsafe { CFArrayGetCount(windows.0) } {
            let candidate = unsafe { CFArrayGetValueAtIndex(windows.0, i) };
            let mut candidate_id = 0;
            if !candidate.is_null()
                && unsafe { _AXUIElementGetWindow(candidate, &mut candidate_id) } == 0
                && candidate_id == window_id
            {
                exact = candidate;
                break;
            }
        }
        if exact.is_null() {
            return super::super::failure(
                "computer_use_stage_unavailable",
                "The selected window has no exact Accessibility window.",
                false,
            );
        }
        let front = unsafe { symbol(c"_SLPSGetFrontProcess") };
        let target = unsafe { symbol(c"GetProcessForPID") };
        let post = unsafe { symbol(c"SLPSPostEventRecordTo") };
        if front.is_null() || target.is_null() || post.is_null() {
            return super::super::failure(
                "computer_use_stage_unsupported",
                "The operating system does not support exact-window stage restoration.",
                false,
            );
        }
        let front: unsafe extern "C" fn(*mut c_void) -> i32 = unsafe { std::mem::transmute(front) };
        let target: unsafe extern "C" fn(i32, *mut c_void) -> i32 =
            unsafe { std::mem::transmute(target) };
        let post: unsafe extern "C" fn(*const c_void, *const u8) -> i32 =
            unsafe { std::mem::transmute(post) };
        let mut current = [0u32; 2];
        let mut destination = [0u32; 2];
        if unsafe { front(current.as_mut_ptr().cast()) } != 0
            || unsafe { target(pid, destination.as_mut_ptr().cast()) } != 0
        {
            return super::super::failure(
                "computer_use_stage_unavailable",
                "The exact stage target could not be resolved.",
                false,
            );
        }
        if !process_matches(pid, expected_process) {
            return super::super::stale_target();
        }
        let mut event = [0u8; 248];
        event[4] = 248;
        event[8] = 13;
        event[60..64].copy_from_slice(&window_id.to_le_bytes());
        event[138] = 2;
        if crate::computer_use_cancel::current_run_cancelled() {
            return super::super::failure(
                "computer_use_stopped",
                "The computer-use run was stopped.",
                false,
            );
        }
        let defocused = unsafe { post(current.as_ptr().cast(), event.as_ptr()) } == 0;
        event[138] = 1;
        if crate::computer_use_cancel::current_run_cancelled() {
            return super::super::failure(
                "computer_use_stopped",
                "The computer-use run stopped after stage dispatch.",
                true,
            );
        }
        let focused = unsafe { post(destination.as_ptr().cast(), event.as_ptr()) } == 0;
        let action = unsafe { string(c"AXRaise") };
        if crate::computer_use_cancel::current_run_cancelled() {
            return super::super::failure(
                "computer_use_stopped",
                "The computer-use run stopped after stage dispatch.",
                true,
            );
        }
        let raised =
            !action.0.is_null() && unsafe { AXUIElementPerformAction(exact, action.0) } == 0;
        let dispatched = defocused || focused || raised;
        let ok = focused && raised;
        let disposition = if dispatched {
            json!({"delivery":"delivered_unverified","retry":"unsafe"})
        } else {
            json!({"delivery":"not_delivered","retry":"safe"})
        };
        let data = json!({"pid":pid,"window_id":window_id,"raised":ok,"activated_application":false,"focused_without_space_follow":focused,"raised_with_accessibility":raised,"disposition":disposition});
        let response = if ok {
            json!({"version":"2.5","ok":true,"command":"join_current_stage","data":data})
        } else {
            json!({"version":"2.5","ok":false,"command":"join_current_stage","error":{"code":"computer_use_stage_unverified","message":"The exact window stage restoration could not be verified. Inspect the target before continuing.","disposition":disposition,"details":data}})
        };
        super::super::normalize(response, "visible")
    }
}
#[cfg(target_os = "macos")]
pub(super) use macos::join;
