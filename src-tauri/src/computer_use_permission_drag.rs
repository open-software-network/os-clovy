//! Native macOS file drag for Computer use permission onboarding.
//!
//! System Settings accepts a real `.app` file URL, not web drag data. The
//! frontend registers the visible helper tile's bounds and the main NSWindow
//! event bridge starts an AppKit drag session when the pointer moves from that
//! tile. The helper bundle never becomes a model-visible path.

use serde::Deserialize;
use std::{path::Path, sync::Mutex};

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PermissionDragTarget {
    Helper,
    Host,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PermissionDragBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl PermissionDragBounds {
    fn is_valid(self) -> bool {
        self.x.is_finite()
            && self.y.is_finite()
            && self.width.is_finite()
            && self.height.is_finite()
            && self.width > 0.0
            && self.height > 0.0
    }

    fn contains(self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.width && y >= self.y && y <= self.y + self.height
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetComputerUsePermissionDragBoundsRequest {
    bounds: Option<PermissionDragBounds>,
    target: Option<PermissionDragTarget>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PermissionDragRegistration {
    bounds: PermissionDragBounds,
    target: PermissionDragTarget,
}

static PERMISSION_DRAG_REGISTRATION: Mutex<Option<PermissionDragRegistration>> = Mutex::new(None);

#[tauri::command]
pub(crate) fn set_computer_use_permission_drag_bounds(
    request: SetComputerUsePermissionDragBoundsRequest,
) {
    if let Ok(mut current) = PERMISSION_DRAG_REGISTRATION.lock() {
        *current = request
            .bounds
            .filter(|bounds| bounds.is_valid())
            .zip(request.target)
            .map(|(bounds, target)| PermissionDragRegistration { bounds, target });
    }
}

pub(crate) fn permission_drag_target() -> Option<PermissionDragTarget> {
    PERMISSION_DRAG_REGISTRATION
        .lock()
        .ok()
        .and_then(|registration| registration.map(|registration| registration.target))
}

pub(crate) fn app_bundle_path(executable: &Path) -> Option<&Path> {
    executable.ancestors().find(|path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"))
    })
}

#[cfg(target_os = "macos")]
mod macos {
    use super::PERMISSION_DRAG_REGISTRATION;
    use objc2::{
        define_class, msg_send,
        rc::Retained,
        runtime::{AnyObject, ProtocolObject},
        AllocAnyThread, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
        NSEvent, NSPasteboardWriting, NSWindow, NSWorkspace,
    };
    use objc2_foundation::{
        MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
        NSURL,
    };
    use std::{path::Path, sync::OnceLock};

    #[derive(Default)]
    struct PermissionDragSourceIvars;

    define_class!(
        // SAFETY: NSObject has no subclassing requirements and this class has
        // no Drop implementation or thread-crossing state.
        #[unsafe(super = NSObject)]
        #[thread_kind = MainThreadOnly]
        #[ivars = PermissionDragSourceIvars]
        struct PermissionDragSource;

        // SAFETY: NSObjectProtocol has no additional safety requirements.
        unsafe impl NSObjectProtocol for PermissionDragSource {}

        // SAFETY: The selector and generated protocol signature match AppKit.
        unsafe impl NSDraggingSource for PermissionDragSource {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn source_operation_mask(
                &self,
                _session: &NSDraggingSession,
                _context: NSDraggingContext,
            ) -> NSDragOperation {
                NSDragOperation::Copy
            }
        }
    );

    impl PermissionDragSource {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(PermissionDragSourceIvars);
            // SAFETY: NSObject's init signature is stable and has no arguments.
            unsafe { msg_send![super(this), init] }
        }
    }

    static PERMISSION_DRAG_SOURCE: OnceLock<usize> = OnceLock::new();

    fn permission_drag_source(mtm: MainThreadMarker) -> &'static PermissionDragSource {
        let pointer = *PERMISSION_DRAG_SOURCE
            .get_or_init(|| Retained::into_raw(PermissionDragSource::new(mtm)) as usize);
        // SAFETY: The retained singleton is intentionally kept for the app
        // lifetime and is only read from the macOS main thread.
        unsafe { &*(pointer as *const PermissionDragSource) }
    }

    pub(super) unsafe fn begin(bundle: &Path, window: &AnyObject, event: *mut AnyObject) -> bool {
        let Some(mtm) = MainThreadMarker::new() else {
            return false;
        };
        if event.is_null() {
            return false;
        }

        // SAFETY: The caller is the NSWindow sendEvent: bridge and passes that
        // window plus the NSEvent currently being dispatched on the main thread.
        let window = unsafe { &*(window as *const AnyObject as *const NSWindow) };
        let event = unsafe { &*(event as *const NSEvent) };
        let Some(content) = window.contentView() else {
            return false;
        };
        let point = event.locationInWindow();
        let frame = content.frame();
        let web_x = point.x;
        let web_y = frame.size.height - point.y;
        let hit = PERMISSION_DRAG_REGISTRATION
            .lock()
            .ok()
            .and_then(|registration| *registration)
            .is_some_and(|registration| registration.bounds.contains(web_x, web_y));
        if !hit {
            return false;
        }

        let Some(bundle_path) = bundle.to_str() else {
            return false;
        };
        let bundle_path = NSString::from_str(bundle_path);
        let url = NSURL::fileURLWithPath_isDirectory(&bundle_path, true);
        let writer = ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*url);
        let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), writer);

        let icon = NSWorkspace::sharedWorkspace().iconForFile(&bundle_path);
        let icon_size = NSSize::new(56.0, 56.0);
        icon.setSize(icon_size);
        let dragging_frame = NSRect::new(
            NSPoint::new(
                point.x - icon_size.width / 2.0,
                point.y - icon_size.height / 2.0,
            ),
            icon_size,
        );
        // SAFETY: NSImage is a valid AppKit dragging-frame contents object.
        unsafe { item.setDraggingFrame_contents(dragging_frame, Some(&icon)) };

        let items = NSArray::from_slice(&[&*item]);
        let source = ProtocolObject::<dyn NSDraggingSource>::from_ref(permission_drag_source(mtm));
        let _session = content.beginDraggingSessionWithItems_event_source(&items, event, source);
        true
    }
}

#[cfg(target_os = "macos")]
pub(crate) unsafe fn begin_permission_drag(
    bundle: &Path,
    window: &objc2::runtime::AnyObject,
    event: *mut objc2::runtime::AnyObject,
) -> bool {
    // SAFETY: Forwarded unchanged to the main-thread AppKit implementation.
    unsafe { macos::begin(bundle, window, event) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn helper_executable_resolves_to_the_app_bundle() {
        let executable = Path::new(
            "/tmp/Clovy Computer Use Driver.app/Contents/MacOS/clovy-computer-use-driver",
        );
        assert_eq!(
            app_bundle_path(executable),
            Some(PathBuf::from("/tmp/Clovy Computer Use Driver.app").as_path())
        );
    }

    #[test]
    fn invalid_or_empty_bounds_are_not_registered() {
        set_computer_use_permission_drag_bounds(SetComputerUsePermissionDragBoundsRequest {
            bounds: Some(PermissionDragBounds {
                x: 0.0,
                y: 0.0,
                width: 0.0,
                height: 20.0,
            }),
            target: Some(PermissionDragTarget::Helper),
        });
        assert_eq!(
            *PERMISSION_DRAG_REGISTRATION.lock().expect("bounds lock"),
            None
        );
    }

    #[test]
    fn valid_registration_keeps_the_selected_bundle_target() {
        set_computer_use_permission_drag_bounds(SetComputerUsePermissionDragBoundsRequest {
            bounds: Some(PermissionDragBounds {
                x: 1.0,
                y: 2.0,
                width: 30.0,
                height: 40.0,
            }),
            target: Some(PermissionDragTarget::Host),
        });
        assert_eq!(permission_drag_target(), Some(PermissionDragTarget::Host));

        set_computer_use_permission_drag_bounds(SetComputerUsePermissionDragBoundsRequest {
            bounds: None,
            target: None,
        });
        assert_eq!(permission_drag_target(), None);
    }
}
