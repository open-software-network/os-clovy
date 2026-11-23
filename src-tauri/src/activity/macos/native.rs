//! The small set of macOS system APIs used by activity capture.
//! Kept here so activity capture does not link a Computer use engine.

#![allow(non_snake_case, non_upper_case_globals)]

use core_foundation::{
    base::{CFGetTypeID, CFRelease, CFRetain, CFTypeRef, TCFType},
    string::{CFString, CFStringGetTypeID, CFStringRef},
};
use std::ffi::c_void;

pub type AXUIElementRef = *const c_void;
pub const kAXErrorSuccess: i32 = 0;

#[repr(C)]
#[derive(Default)]
struct Pair {
    x: f64,
    y: f64,
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    pub fn AXUIElementCopyAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: AXUIElementRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXValueGetType(value: CFTypeRef) -> u32;
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, output: *mut c_void) -> bool;
    fn _AXUIElementGetWindow(element: AXUIElementRef, window: *mut u32) -> i32;
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetTypeID() -> usize;
    fn CFArrayGetCount(array: CFTypeRef) -> isize;
    fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFDictionaryGetValue(dictionary: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetTypeID() -> usize;
    fn CFNumberGetValue(number: CFTypeRef, kind: i32, value: *mut c_void) -> bool;
    fn CFBooleanGetTypeID() -> usize;
    fn CFBooleanGetValue(value: CFTypeRef) -> bool;
    static kCFBooleanTrue: CFTypeRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn CGWindowListCopyWindowInfo(options: u32, relative_to: u32) -> CFTypeRef;
}

pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
}
pub fn screen_recording_granted() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}
pub fn request_screen_recording() -> bool {
    unsafe { CGRequestScreenCaptureAccess() }
}
pub fn request_accessibility() -> bool {
    use core_foundation::{boolean::CFBoolean, dictionary::CFDictionary};
    let options = CFDictionary::from_CFType_pairs(&[(
        CFString::new("AXTrustedCheckOptionPrompt"),
        CFBoolean::true_value(),
    )]);
    unsafe { AXIsProcessTrustedWithOptions(options.as_CFTypeRef()) }
}

unsafe fn copy_attr(element: AXUIElementRef, name: &str) -> Option<CFTypeRef> {
    let mut value = std::ptr::null();
    let name = CFString::new(name);
    (AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef(), &mut value) == 0
        && !value.is_null())
    .then_some(value)
}

/// Copy a string attribute from a live AX element.
///
/// # Safety
/// `element` must remain a valid AXUIElement for the duration of this call.
pub unsafe fn copy_string_attr(element: AXUIElementRef, name: &str) -> Option<String> {
    let value = copy_attr(element, name)?;
    let text = string_value(value);
    CFRelease(value);
    text
}

unsafe fn string_value(value: CFTypeRef) -> Option<String> {
    if value.is_null() || CFGetTypeID(value) != CFStringGetTypeID() {
        return None;
    }
    Some(CFString::wrap_under_get_rule(value.cast()).to_string())
}

unsafe fn children(element: AXUIElementRef, name: &str) -> Vec<AXUIElementRef> {
    let Some(array) = copy_attr(element, name) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    if CFGetTypeID(array) == CFArrayGetTypeID() {
        for index in 0..CFArrayGetCount(array).min(100_000) {
            let value = CFArrayGetValueAtIndex(array, index);
            if !value.is_null() {
                result.push(CFRetain(value));
            }
        }
    }
    CFRelease(array);
    result
}

/// Copy and retain the application's AX windows. Release each returned reference.
///
/// # Safety
/// `element` must be a live application AXUIElement.
pub unsafe fn copy_ax_windows(element: AXUIElementRef) -> Vec<AXUIElementRef> {
    children(element, "AXWindows")
}
/// Copy and retain AX children. Release each returned reference.
///
/// # Safety
/// `element` must remain a valid AXUIElement for the duration of this call.
pub unsafe fn copy_children(element: AXUIElementRef) -> Vec<AXUIElementRef> {
    children(element, "AXChildren")
}
/// Read the window identifier from a live AX window.
///
/// # Safety
/// `element` must remain a valid AXUIElement for the duration of this call.
pub unsafe fn ax_get_window_id(element: AXUIElementRef) -> Option<u32> {
    let mut window = 0;
    (_AXUIElementGetWindow(element, &mut window) == 0 && window != 0).then_some(window)
}

/// Copy a live AX element's position and size.
///
/// # Safety
/// `element` must remain a valid AXUIElement for the duration of this call.
pub unsafe fn element_screen_rect(element: AXUIElementRef) -> Option<[f64; 4]> {
    unsafe fn pair(element: AXUIElementRef, name: &str, kind: u32) -> Option<Pair> {
        let value = copy_attr(element, name)?;
        let mut pair = Pair::default();
        let ok = CFGetTypeID(value) == AXValueGetTypeID()
            && AXValueGetType(value) == kind
            && AXValueGetValue(value, kind, (&mut pair as *mut Pair).cast());
        CFRelease(value);
        ok.then_some(pair)
    }
    let position = pair(element, "AXPosition", 1)?;
    let size = pair(element, "AXSize", 2)?;
    let rect = [position.x, position.y, size.x, size.y];
    rect.iter().all(|v| v.is_finite()).then_some(rect)
}

/// Enable the application's optional Chromium accessibility attributes.
///
/// # Safety
/// `element` must be a live application AXUIElement.
pub unsafe fn enable_chromium_accessibility(element: AXUIElementRef) -> bool {
    let mut changed = false;
    for name in ["AXManualAccessibility", "AXEnhancedUserInterface"] {
        let name = CFString::new(name);
        changed |=
            AXUIElementSetAttributeValue(element, name.as_concrete_TypeRef(), kCFBooleanTrue) == 0;
    }
    changed
}

#[derive(Default)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
pub struct Window {
    pub pid: i32,
    pub window_id: u32,
    pub layer: i32,
    pub is_on_screen: bool,
    pub bounds: Bounds,
    pub app_name: String,
    pub title: String,
}

pub fn visible_windows() -> Vec<Window> {
    unsafe fn field(dictionary: CFTypeRef, key: &str) -> CFTypeRef {
        if dictionary.is_null() || CFGetTypeID(dictionary) != CFDictionaryGetTypeID() {
            return std::ptr::null();
        }
        CFDictionaryGetValue(dictionary, CFString::new(key).as_CFTypeRef())
    }
    unsafe fn number(dictionary: CFTypeRef, key: &str) -> Option<f64> {
        let value = field(dictionary, key);
        if value.is_null() || CFGetTypeID(value) != CFNumberGetTypeID() {
            return None;
        }
        let mut output = 0.0f64;
        (CFNumberGetValue(value, 13, (&mut output as *mut f64).cast()) && output.is_finite())
            .then_some(output)
    }
    unsafe {
        let array = CGWindowListCopyWindowInfo(1 | 16, 0);
        if array.is_null() {
            return Vec::new();
        }
        let mut windows = Vec::new();
        if CFGetTypeID(array) == CFArrayGetTypeID() {
            for index in 0..CFArrayGetCount(array).min(100_000) {
                let dictionary = CFArrayGetValueAtIndex(array, index);
                let Some(pid) = number(dictionary, "kCGWindowOwnerPID") else {
                    continue;
                };
                let Some(window_id) = number(dictionary, "kCGWindowNumber") else {
                    continue;
                };
                let bounds = field(dictionary, "kCGWindowBounds");
                let visible = field(dictionary, "kCGWindowIsOnscreen");
                windows.push(Window {
                    pid: pid as i32,
                    window_id: window_id as u32,
                    layer: number(dictionary, "kCGWindowLayer").unwrap_or(-1.0) as i32,
                    is_on_screen: !visible.is_null()
                        && CFGetTypeID(visible) == CFBooleanGetTypeID()
                        && CFBooleanGetValue(visible),
                    app_name: string_value(field(dictionary, "kCGWindowOwnerName"))
                        .unwrap_or_default(),
                    title: string_value(field(dictionary, "kCGWindowName")).unwrap_or_default(),
                    bounds: Bounds {
                        x: number(bounds, "X").unwrap_or(0.0),
                        y: number(bounds, "Y").unwrap_or(0.0),
                        width: number(bounds, "Width").unwrap_or(0.0),
                        height: number(bounds, "Height").unwrap_or(0.0),
                    },
                });
            }
        }
        CFRelease(array);
        windows
    }
}
