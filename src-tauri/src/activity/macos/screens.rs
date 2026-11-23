//! Display discovery and window enumeration across secondary monitors.

use std::collections::HashSet;

use objc2_app_kit::NSRunningApplication;

use crate::activity::macos::ax::{is_private_window, resolve_browser_url, AutoAxElement};
use crate::activity::macos::native::{
    ax_get_window_id, copy_ax_windows, copy_string_attr, enable_chromium_accessibility,
    AXUIElementCreateApplication,
};
use crate::activity::platform::WindowDescriptor;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

impl CGRect {
    pub fn contains_point(&self, px: f64, py: f64) -> bool {
        px >= self.origin.x
            && px <= (self.origin.x + self.size.width)
            && py >= self.origin.y
            && py <= (self.origin.y + self.size.height)
    }

    pub fn intersection_area(&self, other: &CGRect) -> f64 {
        let x1 = self.origin.x.max(other.origin.x);
        let y1 = self.origin.y.max(other.origin.y);
        let x2 = (self.origin.x + self.size.width).min(other.origin.x + other.size.width);
        let y2 = (self.origin.y + self.size.height).min(other.origin.y + other.size.height);
        if x2 > x1 && y2 > y1 {
            (x2 - x1) * (y2 - y1)
        } else {
            0.0
        }
    }
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGGetActiveDisplayList(
        maxDisplays: u32,
        activeDisplays: *mut u32,
        displayCount: *mut u32,
    ) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
}

/// Returns a list of active `CGDirectDisplayID`s.
pub fn active_displays() -> Vec<u32> {
    let mut displays = [0u32; 16];
    let mut count = 0u32;
    let err = unsafe { CGGetActiveDisplayList(16, displays.as_mut_ptr(), &mut count) };
    if err == 0 && count > 0 {
        displays[..count as usize].to_vec()
    } else {
        vec![main_display_id()]
    }
}

/// Returns the primary display ID.
pub fn main_display_id() -> u32 {
    unsafe { CGMainDisplayID() }
}

/// Returns the display bounds in screen coordinates for `display_id`.
pub fn display_bounds(display_id: u32) -> CGRect {
    unsafe { CGDisplayBounds(display_id) }
}

/// Identifies the display containing or having largest overlap with `rect` [x, y, w, h].
pub fn display_id_for_rect(rect: [f64; 4]) -> u32 {
    let displays = active_displays();
    if displays.is_empty() {
        return main_display_id();
    }
    if displays.len() == 1 {
        return displays[0];
    }

    let [rx, ry, rw, rh] = rect;
    let cx = rx + rw / 2.0;
    let cy = ry + rh / 2.0;

    let target_rect = CGRect {
        origin: CGPoint { x: rx, y: ry },
        size: CGSize {
            width: rw,
            height: rh,
        },
    };

    // 1. Point containment of center
    for &d in &displays {
        let b = display_bounds(d);
        if b.contains_point(cx, cy) {
            return d;
        }
    }

    // 2. Maximum intersection area
    let mut best_id = displays[0];
    let mut best_area = 0.0;
    for &d in &displays {
        let b = display_bounds(d);
        let area = b.intersection_area(&target_rect);
        if area > best_area {
            best_area = area;
            best_id = d;
        }
    }

    best_id
}

/// For each active display other than `CGMainDisplayID`, returns a descriptor of
/// its topmost normal-layer (layer 0) on-screen window.
pub fn secondary_windows(chromium_settled_pids: &mut HashSet<i32>) -> Vec<WindowDescriptor> {
    let displays = active_displays();
    let main_id = main_display_id();
    let secondary_display_ids: Vec<u32> = displays.into_iter().filter(|&d| d != main_id).collect();

    if secondary_display_ids.is_empty() {
        return Vec::new();
    }

    let visible = crate::activity::macos::native::visible_windows();
    let mut results = Vec::new();

    for sec_id in secondary_display_ids {
        let disp_b = display_bounds(sec_id);

        // Find the topmost layer 0 on-screen window for this display
        let top_win = visible.iter().find(|w| {
            if w.layer != 0 || !w.is_on_screen {
                return false;
            }
            let cx = w.bounds.x + w.bounds.width / 2.0;
            let cy = w.bounds.y + w.bounds.height / 2.0;
            if disp_b.contains_point(cx, cy) {
                return true;
            }
            let win_rect = CGRect {
                origin: CGPoint {
                    x: w.bounds.x,
                    y: w.bounds.y,
                },
                size: CGSize {
                    width: w.bounds.width,
                    height: w.bounds.height,
                },
            };
            disp_b.intersection_area(&win_rect) > (w.bounds.width * w.bounds.height * 0.5)
        });

        let Some(win) = top_win else {
            continue;
        };

        let pid = win.pid;
        if pid == std::process::id() as i32 {
            // Clovy's own window: excluded by the engine; never query our own AX tree.
            results.push(WindowDescriptor {
                pid,
                app_name: win.app_name.clone(),
                window_id: Some(win.window_id),
                display_id: Some(sec_id),
                ..WindowDescriptor::default()
            });
            continue;
        }
        let running_app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
        let bundle_id = running_app
            .as_ref()
            .and_then(|a| a.bundleIdentifier().map(|s| s.to_string()));

        let mut title = if !win.title.is_empty() {
            Some(win.title.clone())
        } else {
            None
        };

        let mut browser_url = None;

        // Inspect AX elements for this window if possible
        let app_elem = unsafe { AXUIElementCreateApplication(pid) };
        let private_window = if !app_elem.is_null() {
            let auto_app = AutoAxElement(app_elem);
            crate::activity::macos::ax::bound_messaging(app_elem);
            if !chromium_settled_pids.contains(&pid) {
                chromium_settled_pids.insert(pid);
                let settled = unsafe { enable_chromium_accessibility(app_elem) };
                if settled {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }

            let ax_windows = unsafe { copy_ax_windows(app_elem) };
            let matching_win = crate::activity::macos::ax::take_window(ax_windows, |aw| {
                (unsafe { ax_get_window_id(aw.0) }) == Some(win.window_id)
            });

            let pw = if let Some(target_ax_win) = matching_win {
                if title.is_none() {
                    title = unsafe { copy_string_attr(target_ax_win.0, "AXTitle") };
                }
                let private = is_private_window(
                    &win.app_name,
                    bundle_id.as_deref(),
                    title.as_deref(),
                    Some(target_ax_win.0),
                );
                if !private {
                    browser_url = resolve_browser_url(target_ax_win.0);
                }
                private
            } else {
                is_private_window(&win.app_name, bundle_id.as_deref(), title.as_deref(), None)
            };
            drop(auto_app);
            pw
        } else {
            is_private_window(&win.app_name, bundle_id.as_deref(), title.as_deref(), None)
        };

        results.push(WindowDescriptor {
            pid,
            app_name: win.app_name.clone(),
            bundle_id,
            window_title: title,
            browser_url,
            private_window,
            window_id: Some(win.window_id),
            display_id: Some(sec_id),
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_active_displays_not_empty() {
        let d = active_displays();
        assert!(!d.is_empty());
        let main = main_display_id();
        assert!(d.contains(&main));
    }

    #[test]
    fn test_rect_intersection() {
        let r1 = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 100.0,
                height: 100.0,
            },
        };
        let r2 = CGRect {
            origin: CGPoint { x: 50.0, y: 50.0 },
            size: CGSize {
                width: 100.0,
                height: 100.0,
            },
        };
        assert_eq!(r1.intersection_area(&r2), 2500.0);
    }
}
