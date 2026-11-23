//! In-memory window screenshot capture via ScreenCaptureKit and on-device
//! OCR text recognition using Apple's Vision framework.

use std::ffi::c_void;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSArray, NSData, NSDictionary, NSRect};

use crate::activity::platform::WindowDescriptor;

#[link(name = "Vision", kind = "framework")]
extern "C" {}

/// A recognized text observation with normalized coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct TextObservation {
    pub text: String,
    /// Normalized top Y coordinate (0.0 = bottom, 1.0 = top of image).
    pub top_y: f64,
    /// Normalized left X coordinate (0.0 = left, 1.0 = right of image).
    pub left_x: f64,
    /// Normalized height.
    pub height: f64,
}

/// Orders observations top-to-bottom, left-to-right on the same line, then
/// joins them one per line.
///
/// After a total top-to-bottom order, the first observation of a line anchors
/// it: each following observation joins while it sits less than half the
/// taller of the two heights (at least 0.005) below the anchor, and the first
/// one that does not anchors the next line. Each line is then ordered left to
/// right. Both orders are total: position ties go to the taller observation,
/// then to text order, so the anchor and the result do not depend on the
/// order Vision returns observations in.
pub fn sort_and_join_observations(mut obs: Vec<TextObservation>) -> String {
    if obs.is_empty() {
        return String::new();
    }

    // Higher Y in Vision means closer to the top of the image.
    obs.sort_unstable_by(|a, b| {
        b.top_y
            .total_cmp(&a.top_y)
            .then_with(|| a.left_x.total_cmp(&b.left_x))
            .then_with(|| b.height.total_cmp(&a.height))
            .then_with(|| a.text.cmp(&b.text))
    });
    let mut start = 0;
    while start < obs.len() {
        let anchor = &obs[start];
        let line_len = 1 + obs[start + 1..]
            .iter()
            .take_while(|item| {
                let tol = 0.5 * anchor.height.max(item.height).max(0.01);
                anchor.top_y - item.top_y < tol
            })
            .count();
        let line = &mut obs[start..start + line_len];
        line.sort_unstable_by(|a, b| {
            a.left_x
                .total_cmp(&b.left_x)
                .then_with(|| b.top_y.total_cmp(&a.top_y))
                .then_with(|| b.height.total_cmp(&a.height))
                .then_with(|| a.text.cmp(&b.text))
        });
        start += line_len;
    }

    let mut result = String::new();
    for item in obs {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&item.text);
    }
    result
}

/// Runs Apple Vision text recognition on encoded image bytes (PNG, JPEG, etc.).
///
/// Fully in-memory, requires no screen recording permission or disk access.
pub fn recognize_text_from_data(image_bytes: &[u8]) -> Option<String> {
    if image_bytes.is_empty() {
        return None;
    }

    let handler_class = AnyClass::get(c"VNImageRequestHandler")?;
    let options = NSDictionary::<AnyObject, AnyObject>::new();
    let ns_data = NSData::with_bytes(image_bytes);

    let handler_alloc: *mut AnyObject = unsafe { msg_send![handler_class, alloc] };
    let handler: *mut AnyObject =
        unsafe { msg_send![handler_alloc, initWithData: &*ns_data, options: &*options] };
    if handler.is_null() {
        return None;
    }

    let res = run_vision_request(handler);
    unsafe {
        let _: () = msg_send![handler, release];
    }
    res
}

/// Runs Apple Vision text recognition on a retained CoreGraphics `CGImageRef`.
pub fn recognize_text_from_cg_image(cg_image: *mut c_void) -> Option<String> {
    if cg_image.is_null() {
        return None;
    }

    let handler_class = AnyClass::get(c"VNImageRequestHandler")?;
    let options = NSDictionary::<AnyObject, AnyObject>::new();

    let handler_alloc: *mut AnyObject = unsafe { msg_send![handler_class, alloc] };
    let handler: *mut AnyObject =
        unsafe { msg_send![handler_alloc, initWithCGImage: cg_image, options: &*options] };
    if handler.is_null() {
        return None;
    }

    let res = run_vision_request(handler);
    unsafe {
        let _: () = msg_send![handler, release];
    }
    res
}

fn run_vision_request(handler: *mut AnyObject) -> Option<String> {
    let request_class = AnyClass::get(c"VNRecognizeTextRequest")?;
    let request_alloc: *mut AnyObject = unsafe { msg_send![request_class, alloc] };
    let request: *mut AnyObject = unsafe { msg_send![request_alloc, init] };
    if request.is_null() {
        return None;
    }

    // Accurate level (0 = VNRequestTextRecognitionLevelAccurate)
    let _: () = unsafe { msg_send![request, setRecognitionLevel: 0isize] };
    // Uses language correction
    let _: () = unsafe { msg_send![request, setUsesLanguageCorrection: true] };

    let request_ref = unsafe { &*request };
    let requests_arr = NSArray::from_slice(&[request_ref]);
    let mut error: *mut AnyObject = std::ptr::null_mut();
    let success: bool =
        unsafe { msg_send![handler, performRequests: &*requests_arr, error: &mut error] };

    if !success {
        unsafe {
            let _: () = msg_send![request, release];
        }
        return None;
    }

    let results: *mut AnyObject = unsafe { msg_send![request, results] };
    if results.is_null() {
        unsafe {
            let _: () = msg_send![request, release];
        }
        return Some(String::new());
    }

    let count: usize = unsafe { msg_send![results, count] };
    let mut observations = Vec::with_capacity(count);

    for i in 0..count {
        let obs: *mut AnyObject = unsafe { msg_send![results, objectAtIndex: i] };
        if obs.is_null() {
            continue;
        }

        let bbox: NSRect = unsafe { msg_send![obs, boundingBox] };
        let candidates: *mut AnyObject = unsafe { msg_send![obs, topCandidates: 1usize] };
        if candidates.is_null() {
            continue;
        }
        let cand_count: usize = unsafe { msg_send![candidates, count] };
        if cand_count == 0 {
            continue;
        }
        let top_cand: *mut AnyObject = unsafe { msg_send![candidates, objectAtIndex: 0usize] };
        if top_cand.is_null() {
            continue;
        }
        let str_val: *mut AnyObject = unsafe { msg_send![top_cand, string] };
        if str_val.is_null() {
            continue;
        }
        let utf8_ptr: *const std::os::raw::c_char = unsafe { msg_send![str_val, UTF8String] };
        if !utf8_ptr.is_null() {
            let c_str = unsafe { std::ffi::CStr::from_ptr(utf8_ptr) };
            if let Ok(text) = c_str.to_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    observations.push(TextObservation {
                        text: trimmed.to_string(),
                        top_y: bbox.origin.y + bbox.size.height,
                        left_x: bbox.origin.x,
                        height: bbox.size.height,
                    });
                }
            }
        }
    }

    unsafe {
        let _: () = msg_send![request, release];
    }

    if observations.is_empty() {
        None
    } else {
        Some(sort_and_join_observations(observations))
    }
}

/// Longest side of the captured image, in pixels. Enough for Vision to read
/// UI text; bounded so a 6K display does not cost a 6K OCR pass.
const MAX_CAPTURE_PIXELS: f64 = 2000.0;
/// Points to pixels for the capture: Retina density keeps small UI text
/// legible to Vision.
const CAPTURE_POINT_SCALE: f64 = 2.0;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

#[link(name = "ScreenCaptureKit", kind = "framework")]
extern "C" {}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGImageRetain(image: *mut c_void) -> *mut c_void;
    fn CGImageRelease(image: *mut c_void);
}

/// A retained Objective-C object or CGImage handed across the completion
/// handler's dispatch queue.
struct Retained(*mut c_void);
// SAFETY: the pointer is retained by the sender and only used, then released,
// by the single receiving thread.
unsafe impl Send for Retained {}

struct OwnedCgImage(*mut c_void);

impl Drop for OwnedCgImage {
    fn drop(&mut self) {
        unsafe { CGImageRelease(self.0) };
    }
}

struct OwnedObject(*mut AnyObject);

impl Drop for OwnedObject {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _: () = msg_send![self.0, release];
            }
        }
    }
}

/// `SCShareableContent` for on-screen windows, retained.
fn shareable_content() -> Option<OwnedObject> {
    let class = AnyClass::get(c"SCShareableContent")?;
    let (sender, receiver) = mpsc::sync_channel::<Option<Retained>>(1);
    let handler = RcBlock::new(move |content: *mut AnyObject, _error: *mut AnyObject| {
        let retained = (!content.is_null()).then(|| {
            let _: *mut AnyObject = unsafe { msg_send![content, retain] };
            Retained(content.cast())
        });
        // A late result after the timeout has no receiver: release it here.
        if let Err(mpsc::TrySendError::Disconnected(Some(late))) = sender.try_send(retained) {
            let _: () = unsafe { msg_send![late.0.cast::<AnyObject>(), release] };
        }
    });
    unsafe {
        let _: () = msg_send![
            class,
            getShareableContentExcludingDesktopWindows: true,
            onScreenWindowsOnly: true,
            completionHandler: &*handler
        ];
    }
    let content = receiver.recv_timeout(CAPTURE_TIMEOUT).ok()??;
    Some(OwnedObject(content.0.cast()))
}

/// Captures exactly `window_id` into a CGImage held in memory only.
fn capture_window_image(window_id: u32) -> Option<OwnedCgImage> {
    let content = shareable_content()?;
    let windows: *mut AnyObject = unsafe { msg_send![content.0, windows] };
    if windows.is_null() {
        return None;
    }
    let count: usize = unsafe { msg_send![windows, count] };
    let window = (0..count)
        .map(|index| -> *mut AnyObject { unsafe { msg_send![windows, objectAtIndex: index] } })
        .find(|candidate| {
            !candidate.is_null() && {
                let id: u32 = unsafe { msg_send![*candidate, windowID] };
                id == window_id
            }
        })?;

    let frame: NSRect = unsafe { msg_send![window, frame] };
    let width = frame.size.width.max(1.0) * CAPTURE_POINT_SCALE;
    let height = frame.size.height.max(1.0) * CAPTURE_POINT_SCALE;
    let scale = (MAX_CAPTURE_PIXELS / width.max(height)).min(1.0);
    let target_width = ((width * scale).round() as usize).max(1);
    let target_height = ((height * scale).round() as usize).max(1);

    let filter_class = AnyClass::get(c"SCContentFilter")?;
    let config_class = AnyClass::get(c"SCStreamConfiguration")?;
    let manager_class = AnyClass::get(c"SCScreenshotManager")?;
    let filter = OwnedObject(unsafe {
        let allocated: *mut AnyObject = msg_send![filter_class, alloc];
        msg_send![allocated, initWithDesktopIndependentWindow: window]
    });
    // The filter retains the window; the content list can go.
    drop(content);
    if filter.0.is_null() {
        return None;
    }
    let config = OwnedObject(unsafe { msg_send![config_class, new] });
    if config.0.is_null() {
        return None;
    }
    unsafe {
        let _: () = msg_send![config.0, setWidth: target_width];
        let _: () = msg_send![config.0, setHeight: target_height];
        let _: () = msg_send![config.0, setShowsCursor: false];
    }

    let (sender, receiver) = mpsc::sync_channel::<Option<Retained>>(1);
    let handler = RcBlock::new(move |image: *mut c_void, _error: *mut AnyObject| {
        let retained = (!image.is_null()).then(|| Retained(unsafe { CGImageRetain(image) }));
        if let Err(mpsc::TrySendError::Disconnected(Some(late))) = sender.try_send(retained) {
            unsafe { CGImageRelease(late.0) };
        }
    });
    unsafe {
        let _: () = msg_send![
            manager_class,
            captureImageWithFilter: filter.0,
            configuration: config.0,
            completionHandler: &*handler
        ];
    }
    let image = receiver.recv_timeout(CAPTURE_TIMEOUT).ok()??;
    Some(OwnedCgImage(image.0))
}

/// Captures the window in memory with ScreenCaptureKit and runs Apple Vision
/// OCR on it. The image never leaves this function and is never encoded or
/// written anywhere.
pub fn ocr_text(window: &WindowDescriptor) -> Option<String> {
    let image = capture_window_image(window.window_id?)?;
    recognize_text_from_cg_image(image.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vision_ocr_fixture_recognition() {
        let fixture_bytes = include_bytes!("fixtures/ocr_test.png");
        let recognized = recognize_text_from_data(fixture_bytes);
        assert!(
            recognized.is_some(),
            "Vision OCR failed to recognize text in fixture"
        );
        let text = recognized.unwrap();
        assert!(
            text.contains("Clovy Activity Capture"),
            "Expected 'Clovy Activity Capture' in recognized text, got: {text}"
        );
        assert!(
            text.contains("Optical Character Recognition Test"),
            "Expected 'Optical Character Recognition Test' in recognized text, got: {text}"
        );
        assert!(
            text.contains("Line 3: Accurate Text"),
            "Expected 'Line 3: Accurate Text' in recognized text, got: {text}"
        );
    }

    #[test]
    fn test_sort_and_join_observations() {
        let obs = vec![
            TextObservation {
                text: "Second line".to_string(),
                top_y: 0.5,
                left_x: 0.1,
                height: 0.05,
            },
            TextObservation {
                text: "First line".to_string(),
                top_y: 0.9,
                left_x: 0.1,
                height: 0.05,
            },
            TextObservation {
                text: "Third line".to_string(),
                top_y: 0.1,
                left_x: 0.1,
                height: 0.05,
            },
        ];

        let joined = sort_and_join_observations(obs);
        assert_eq!(joined, "First line\nSecond line\nThird line");
    }

    #[test]
    #[ignore = "requires live Screen Recording TCC grant"]
    fn test_live_screen_capture_ocr() {
        let win = WindowDescriptor {
            window_id: Some(1),
            ..Default::default()
        };
        let _ = ocr_text(&win);
    }
}

#[cfg(test)]
#[path = "ocr_sort_regression_tests.rs"]
mod sort_regression_tests;
