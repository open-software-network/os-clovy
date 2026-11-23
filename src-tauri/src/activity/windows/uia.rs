//! Windows UI Automation (UIA) tree inspection, browser URL extraction,
//! and private window detection (D3).

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::activity::filter::has_private_window_marker;
use crate::activity::platform::WindowDescriptor;

/// Description extracted from a window via UI Automation (URL and privacy state).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct UiaDescription {
    pub url: Option<String>,
    pub private: bool,
}

/// Abstract control types relevant for UI Automation extraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiaControlType {
    Window,
    Pane,
    Document,
    Edit,
    Text,
    Button,
    Toolbar,
    Other,
}

/// Minimal abstraction over a UI Automation element node.
///
/// Implemented by `ComUiaNode` on Windows (using lazy `IUIAutomationTreeWalker` navigation)
/// and by `FakeUiaNode` in unit tests. Traversal never materializes the whole tree in advance,
/// protecting password privacy, performance, and avoiding COM stalls.
pub(crate) trait UiaNode: Sized {
    fn control_type(&self) -> UiaControlType;
    fn is_password(&self) -> bool;
    fn name(&self) -> Option<String>;
    fn value(&self) -> Option<String>;
    /// URI metadata of a document, never its text or an editable address.
    fn document_url(&self) -> Option<String>;
    fn is_offscreen(&self) -> Option<bool>;
    fn text_content(&self, max_chars: usize) -> Option<String>;
    fn first_child(&self) -> Option<Self>;
    fn next_sibling(&self) -> Option<Self>;
}

/// RAII guard ensuring `CoUninitialize` is called only after COM interfaces are dropped.
#[cfg(target_os = "windows")]
struct ComApartmentGuard {
    initialized: bool,
}

#[cfg(target_os = "windows")]
impl Drop for ComApartmentGuard {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                windows::Win32::System::Com::CoUninitialize();
            }
        }
    }
}

/// The UI Automation adapter for the Windows capture thread.
pub(crate) struct Uia {
    #[cfg(target_os = "windows")]
    automation: Option<windows::Win32::UI::Accessibility::IUIAutomation2>,
    #[cfg(target_os = "windows")]
    walker: Option<windows::Win32::UI::Accessibility::IUIAutomationTreeWalker>,
    // Declared last so COM interfaces are dropped BEFORE apartment uninitialization
    #[cfg(target_os = "windows")]
    _com_guard: Option<ComApartmentGuard>,
}

impl Uia {
    /// Creates a new UIA instance on the capture thread.
    pub(crate) fn new() -> Option<Self> {
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::System::Com::{
                CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
            };
            use windows::Win32::UI::Accessibility::{CUIAutomation8, IUIAutomation2};

            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            let initialized = hr.is_ok();
            let com_guard = ComApartmentGuard { initialized };

            let automation: IUIAutomation2 =
                match unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) } {
                    Ok(auto) => auto,
                    Err(_) => return None,
                };

            unsafe {
                let _ = automation.SetConnectionTimeout(250);
                let _ = automation.SetTransactionTimeout(250);
            }

            let walker = match unsafe { automation.ControlViewWalker() } {
                Ok(w) => w,
                Err(_) => return None,
            };

            Some(Self {
                automation: Some(automation),
                walker: Some(walker),
                _com_guard: Some(com_guard),
            })
        }
        #[cfg(not(target_os = "windows"))]
        {
            Some(Self {})
        }
    }

    /// Enriches a browser window descriptor by extracting the active URL and
    /// detecting private/incognito browsing mode.
    pub(crate) fn describe(&mut self, window: &WindowDescriptor) -> UiaDescription {
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::Foundation::HWND;

            let Some(wid) = window.window_id else {
                return UiaDescription::default();
            };
            let hwnd = HWND((wid as i32 as isize) as *mut _);

            if let (Some(auto), Some(walker)) = (&self.automation, &self.walker) {
                if let Ok(element) = unsafe { auto.ElementFromHandle(hwnd) } {
                    if !windows::core::Interface::as_raw(&element).is_null() {
                        let root_node = ComUiaNode {
                            element,
                            walker: walker.clone(),
                        };
                        return extract_uia_description(&root_node, window.window_title.as_deref());
                    }
                }
            }

            UiaDescription::default()
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window;
            UiaDescription::default()
        }
    }

    /// Extracts visible text from the window's UIA tree within budget constraints.
    pub(crate) fn text(&mut self, window: &WindowDescriptor) -> Option<String> {
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::Foundation::HWND;

            let Some(wid) = window.window_id else {
                return None;
            };
            let hwnd = HWND((wid as i32 as isize) as *mut _);

            if let (Some(auto), Some(walker)) = (&self.automation, &self.walker) {
                if let Ok(element) = unsafe { auto.ElementFromHandle(hwnd) } {
                    if !windows::core::Interface::as_raw(&element).is_null() {
                        let root_node = ComUiaNode {
                            element,
                            walker: walker.clone(),
                        };
                        return extract_uia_text(&root_node);
                    }
                }
            }

            None
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = window;
            None
        }
    }
}

/// Win32 COM implementation of lazy `UiaNode` navigation.
#[cfg(target_os = "windows")]
pub(crate) struct ComUiaNode {
    element: windows::Win32::UI::Accessibility::IUIAutomationElement,
    walker: windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
}

#[cfg(target_os = "windows")]
impl UiaNode for ComUiaNode {
    fn control_type(&self) -> UiaControlType {
        use windows::Win32::UI::Accessibility::{
            UIA_ButtonControlTypeId, UIA_DocumentControlTypeId, UIA_EditControlTypeId,
            UIA_PaneControlTypeId, UIA_TextControlTypeId, UIA_ToolBarControlTypeId,
            UIA_WindowControlTypeId,
        };

        let raw_type = unsafe { self.element.CurrentControlType() }.unwrap_or_default();
        if raw_type == UIA_WindowControlTypeId {
            UiaControlType::Window
        } else if raw_type == UIA_PaneControlTypeId {
            UiaControlType::Pane
        } else if raw_type == UIA_DocumentControlTypeId {
            UiaControlType::Document
        } else if raw_type == UIA_EditControlTypeId {
            UiaControlType::Edit
        } else if raw_type == UIA_TextControlTypeId {
            UiaControlType::Text
        } else if raw_type == UIA_ButtonControlTypeId {
            UiaControlType::Button
        } else if raw_type == UIA_ToolBarControlTypeId {
            UiaControlType::Toolbar
        } else {
            UiaControlType::Other
        }
    }

    fn is_password(&self) -> bool {
        unsafe { self.element.CurrentIsPassword() }
            .map(|b| b.as_bool())
            .unwrap_or(true)
    }

    fn name(&self) -> Option<String> {
        let bstr = unsafe { self.element.CurrentName() }.ok()?;
        let s = bstr.to_string();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn value(&self) -> Option<String> {
        use windows::Win32::UI::Accessibility::{IUIAutomationValuePattern, UIA_ValuePatternId};

        let val_pattern: IUIAutomationValuePattern =
            unsafe { self.element.GetCurrentPatternAs(UIA_ValuePatternId) }.ok()?;
        let bstr = unsafe { val_pattern.CurrentValue() }.ok()?;
        let s = bstr.to_string();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn text_content(&self, max_chars: usize) -> Option<String> {
        use windows::Win32::UI::Accessibility::{IUIAutomationTextPattern, UIA_TextPatternId};

        let text_pattern: IUIAutomationTextPattern =
            unsafe { self.element.GetCurrentPatternAs(UIA_TextPatternId) }.ok()?;
        let range = unsafe { text_pattern.DocumentRange() }.ok()?;
        let bstr = unsafe { range.GetText(max_chars as i32) }.ok()?;
        let s = bstr.to_string();
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    fn document_url(&self) -> Option<String> {
        use windows::Win32::UI::Accessibility::{
            IUIAutomationLegacyIAccessiblePattern, UIA_LegacyIAccessiblePatternId,
            ROLE_SYSTEM_DOCUMENT,
        };

        let document: IUIAutomationLegacyIAccessiblePattern = unsafe {
            self.element
                .GetCurrentPatternAs(UIA_LegacyIAccessiblePatternId)
        }
        .ok()?;
        if unsafe { document.CurrentRole() }.ok()? != ROLE_SYSTEM_DOCUMENT {
            return None;
        }
        unsafe { document.CurrentValue() }
            .ok()
            .map(|value| value.to_string())
    }

    fn is_offscreen(&self) -> Option<bool> {
        unsafe { self.element.CurrentIsOffscreen() }
            .ok()
            .map(|value| value.as_bool())
    }

    fn first_child(&self) -> Option<Self> {
        let child = unsafe { self.walker.GetFirstChildElement(&self.element) }.ok()?;
        if windows::core::Interface::as_raw(&child).is_null() {
            None
        } else {
            Some(ComUiaNode {
                element: child,
                walker: self.walker.clone(),
            })
        }
    }

    fn next_sibling(&self) -> Option<Self> {
        let sibling = unsafe { self.walker.GetNextSiblingElement(&self.element) }.ok()?;
        if windows::core::Interface::as_raw(&sibling).is_null() {
            None
        } else {
            Some(ComUiaNode {
                element: sibling,
                walker: self.walker.clone(),
            })
        }
    }
}

/// Accept only an explicit HTTP(S) URI from document metadata.
fn confirmed_document_url(value: &str) -> Option<String> {
    let url = url::Url::parse(value.trim()).ok()?;
    if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() {
        Some(url.to_string())
    } else {
        None
    }
}

/// Returns whether this node's children may be inspected for browser chrome.
fn inspect_description_node<N: UiaNode>(
    node: &N,
    private: &mut bool,
    documents: &mut usize,
    url: &mut Option<String>,
    complete: &mut bool,
) -> bool {
    if node.is_password() {
        *complete = false;
        return false;
    }
    if node.control_type() == UiaControlType::Document {
        match node.is_offscreen() {
            Some(true) => {}
            visibility => {
                *documents += 1;
                *url = if *documents == 1 && visibility == Some(false) {
                    node.document_url()
                        .as_deref()
                        .and_then(confirmed_document_url)
                } else {
                    None
                };
            }
        }
        // Never read document names, text or descendants to infer identity.
        return false;
    }
    if !*private
        && node
            .name()
            .as_deref()
            .is_some_and(has_private_window_marker)
    {
        *private = true;
    }
    !*private
}

// ---------------------------------------------------------------------------
// Traversal algorithms over UiaNode
// ---------------------------------------------------------------------------

/// Reads one visible document's confirmed URI and private browser chrome.
///
/// A 250 ms deadline, 200-node budget and depth 40 bound traversal. Password
/// subtrees and document contents are never visited. Missing/ambiguous metadata
/// or an incomplete walk leaves identity unknown; address edits are not identity.
pub(crate) fn extract_uia_description<N: UiaNode>(root: &N, title: Option<&str>) -> UiaDescription {
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut private = title.is_some_and(has_private_window_marker);
    if private {
        return UiaDescription { url: None, private };
    }
    let mut url = None;
    let mut documents = 0;
    let mut complete = true;
    let mut queue = VecDeque::new();
    if inspect_description_node(root, &mut private, &mut documents, &mut url, &mut complete) {
        if let Some(child) = root.first_child() {
            queue.push_back((child, 1usize));
        }
    }

    let mut visited = 1usize; // Root was already inspected.
    const MAX_NODES: usize = 200;
    const MAX_DEPTH: usize = 40;
    while let Some((node, depth)) = queue.pop_front() {
        if visited >= MAX_NODES || Instant::now() >= deadline {
            complete = false;
            break;
        }
        visited += 1;
        let descend =
            inspect_description_node(&node, &mut private, &mut documents, &mut url, &mut complete);
        if private {
            break;
        }
        if descend {
            if depth < MAX_DEPTH {
                if let Some(child) = node.first_child() {
                    queue.push_back((child, depth + 1));
                }
            } else {
                complete = false;
            }
        }
        if let Some(sibling) = node.next_sibling() {
            queue.push_back((sibling, depth));
        }
    }
    if private || !complete || documents != 1 || Instant::now() >= deadline {
        url = None;
    }
    UiaDescription { url, private }
}

/// Extracts visible text from an abstract UIA tree node within budgets.
///
/// Budgets: 250ms deadline, 3,000 nodes, depth 40, 20,000 characters.
/// Rules:
/// - Skips password elements and their subtrees entirely.
/// - Skips offscreen or unknown-visibility nodes and their entire subtrees.
/// - Skips window chrome (buttons, menus, toolbars, scrollbars).
/// - Reads Document, Edit, and Text controls.
/// - Deduplicates adjacent chunks and caps at 20,000 characters.
pub(crate) fn extract_uia_text<N: UiaNode>(root: &N) -> Option<String> {
    let start_time = Instant::now();
    let deadline = start_time + Duration::from_millis(250);

    const MAX_NODES: usize = 3000;
    const MAX_DEPTH: usize = 40;
    const MAX_CHARS: usize = 20_000;

    let mut chunks: Vec<String> = Vec::new();
    let mut total_chars = 0usize;
    let mut visited_nodes = 0usize;

    let mut queue = VecDeque::new();

    // Check root node
    if !root.is_password() && root.is_offscreen() == Some(false) {
        let is_content = matches!(
            root.control_type(),
            UiaControlType::Document | UiaControlType::Edit | UiaControlType::Text
        );
        if is_content {
            let extracted = root
                .text_content(MAX_CHARS.saturating_sub(total_chars))
                .or_else(|| {
                    if root.control_type() == UiaControlType::Edit {
                        root.value()
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    if root.control_type() == UiaControlType::Text {
                        root.name()
                    } else {
                        None
                    }
                });

            if let Some(text) = extracted {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    total_chars += trimmed.chars().count();
                    chunks.push(trimmed.to_string());
                }
            }
        }

        if let Some(child) = root.first_child() {
            queue.push_back((Some(child), 0usize));
        }
    }

    while let Some((maybe_node, depth)) = queue.pop_front() {
        let Some(node) = maybe_node else {
            continue;
        };

        visited_nodes += 1;
        if visited_nodes >= MAX_NODES || Instant::now() >= deadline || total_chars >= MAX_CHARS {
            break;
        }

        if node.is_password() || node.is_offscreen() != Some(false) {
            // Hidden/unknown content and password subtrees must never be read.
            if let Some(sibling) = node.next_sibling() {
                queue.push_back((Some(sibling), depth));
            }
            continue;
        }

        let is_chrome = matches!(
            node.control_type(),
            UiaControlType::Button | UiaControlType::Toolbar
        );

        let is_content = matches!(
            node.control_type(),
            UiaControlType::Document | UiaControlType::Edit | UiaControlType::Text
        );

        if is_content {
            let extracted = node
                .text_content(MAX_CHARS.saturating_sub(total_chars))
                .or_else(|| {
                    if node.control_type() == UiaControlType::Edit {
                        node.value()
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    if node.control_type() == UiaControlType::Text {
                        node.name()
                    } else {
                        None
                    }
                });

            if let Some(text) = extracted {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    total_chars += trimmed.chars().count();
                    chunks.push(trimmed.to_string());
                }
            }
        }

        // Chrome subtrees (toolbars, buttons) do not contain document text
        if depth < MAX_DEPTH && !is_chrome {
            if let Some(child) = node.first_child() {
                queue.push_back((Some(child), depth + 1));
            }
        }

        if let Some(sibling) = node.next_sibling() {
            queue.push_back((Some(sibling), depth));
        }
    }

    if chunks.is_empty() {
        return None;
    }

    // Deduplicate adjacent identical chunks
    let mut deduped: Vec<String> = Vec::new();
    for chunk in chunks {
        if deduped.last().map(|s| s.as_str()) != Some(&chunk) {
            deduped.push(chunk);
        }
    }

    if deduped.is_empty() {
        return None;
    }

    let mut result = String::new();
    let mut current_chars = 0usize;
    for chunk in deduped {
        if !result.is_empty() {
            if current_chars + 1 >= MAX_CHARS {
                break;
            }
            result.push(' ');
            current_chars += 1;
        }
        let remaining_chars = MAX_CHARS.saturating_sub(current_chars);
        let chunk_chars = chunk.chars().count();
        if chunk_chars <= remaining_chars {
            result.push_str(&chunk);
            current_chars += chunk_chars;
        } else {
            for c in chunk.chars().take(remaining_chars) {
                result.push(c);
            }
            break;
        }
    }

    let trimmed = result.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

// ---------------------------------------------------------------------------
// Test utilities and behavioral tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Node representation for test trees, implementing lazy `UiaNode` traversal.
    #[derive(Clone)]
    struct FakeUiaNode {
        inner: Arc<FakeNodeInner>,
    }

    struct FakeNodeInner {
        control_type: UiaControlType,
        is_password: bool,
        name: Option<String>,
        value: Option<String>,
        text: Option<String>,
        delay: Option<Duration>,
        offscreen: Option<bool>,
        text_reads: Arc<AtomicUsize>,
        first_child: Option<FakeUiaNode>,
        next_sibling: Option<FakeUiaNode>,
    }

    impl FakeUiaNode {
        fn leaf(
            control_type: UiaControlType,
            name: Option<&str>,
            value: Option<&str>,
            text: Option<&str>,
            is_password: bool,
            delay: Option<Duration>,
        ) -> Self {
            Self {
                inner: Arc::new(FakeNodeInner {
                    control_type,
                    is_password,
                    name: name.map(String::from),
                    value: value.map(String::from),
                    text: text.map(String::from),
                    delay,
                    offscreen: Some(false),
                    text_reads: Arc::new(AtomicUsize::new(0)),
                    first_child: None,
                    next_sibling: None,
                }),
            }
        }

        fn with_children(
            control_type: UiaControlType,
            name: Option<&str>,
            is_password: bool,
            children: Vec<FakeUiaNode>,
        ) -> Self {
            let mut linked_children: Vec<FakeUiaNode> = Vec::new();
            let mut next: Option<FakeUiaNode> = None;
            for child in children.into_iter().rev() {
                let mut node = child;
                if let Some(nxt) = next {
                    let mut inner_clone = (*node.inner).clone_fields();
                    inner_clone.next_sibling = Some(nxt);
                    node = FakeUiaNode {
                        inner: Arc::new(inner_clone),
                    };
                }
                next = Some(node.clone());
                linked_children.push(node);
            }
            linked_children.reverse();

            let first_child = linked_children.into_iter().next();

            Self {
                inner: Arc::new(FakeNodeInner {
                    control_type,
                    is_password,
                    name: name.map(String::from),
                    value: None,
                    text: None,
                    delay: None,
                    offscreen: Some(false),
                    text_reads: Arc::new(AtomicUsize::new(0)),
                    first_child,
                    next_sibling: None,
                }),
            }
        }

        fn with_document_url(self, url: &str) -> Self {
            let mut inner = self.inner.clone_fields();
            inner.value = Some(url.to_owned());
            Self {
                inner: Arc::new(inner),
            }
        }

        fn with_visibility(self, offscreen: Option<bool>) -> Self {
            let mut inner = self.inner.clone_fields();
            inner.offscreen = offscreen;
            Self {
                inner: Arc::new(inner),
            }
        }
    }

    impl FakeNodeInner {
        fn clone_fields(&self) -> Self {
            Self {
                control_type: self.control_type,
                is_password: self.is_password,
                name: self.name.clone(),
                value: self.value.clone(),
                text: self.text.clone(),
                delay: self.delay,
                offscreen: self.offscreen,
                text_reads: self.text_reads.clone(),
                first_child: self.first_child.clone(),
                next_sibling: self.next_sibling.clone(),
            }
        }
    }

    impl UiaNode for FakeUiaNode {
        fn control_type(&self) -> UiaControlType {
            if let Some(d) = self.inner.delay {
                std::thread::sleep(d);
            }
            self.inner.control_type
        }

        fn is_password(&self) -> bool {
            self.inner.is_password
        }

        fn name(&self) -> Option<String> {
            self.inner.name.clone()
        }

        fn value(&self) -> Option<String> {
            self.inner.value.clone()
        }

        fn text_content(&self, _max_chars: usize) -> Option<String> {
            self.inner.text_reads.fetch_add(1, Ordering::SeqCst);
            self.inner.text.clone()
        }

        fn document_url(&self) -> Option<String> {
            self.inner.value.clone()
        }

        fn is_offscreen(&self) -> Option<bool> {
            self.inner.offscreen
        }

        fn first_child(&self) -> Option<Self> {
            self.inner.first_child.clone()
        }

        fn next_sibling(&self) -> Option<Self> {
            self.inner.next_sibling.clone()
        }
    }

    #[test]
    fn windows_uia_offscreen_document_never_reads_text_under_allowed_url() {
        let visible = FakeUiaNode::leaf(
            UiaControlType::Document,
            None,
            None,
            Some("Visible allowed article"),
            false,
            None,
        )
        .with_document_url("https://allowed.example/article");
        let hidden_child = FakeUiaNode::leaf(
            UiaControlType::Text,
            None,
            None,
            Some("Excluded tab descendant"),
            false,
            None,
        );
        let child_reads = hidden_child.inner.text_reads.clone();
        let hidden =
            FakeUiaNode::with_children(UiaControlType::Document, None, false, vec![hidden_child])
                .with_document_url("https://excluded.example/private")
                .with_visibility(Some(true));
        let hidden_reads = hidden.inner.text_reads.clone();
        let tree =
            FakeUiaNode::with_children(UiaControlType::Window, None, false, vec![visible, hidden]);
        assert_eq!(
            extract_uia_description(&tree, None).url.as_deref(),
            Some("https://allowed.example/article")
        );
        let text = extract_uia_text(&tree);
        assert_eq!(
            hidden_reads.load(Ordering::SeqCst),
            0,
            "offscreen document text reader was invoked"
        );
        assert_eq!(
            child_reads.load(Ordering::SeqCst),
            0,
            "offscreen subtree text reader was invoked"
        );
        assert_eq!(text.as_deref(), Some("Visible allowed article"));
    }

    #[test]
    fn windows_uia_offscreen_content_is_not_read_inside_visible_document() {
        for visibility in [Some(true), None] {
            let hidden = FakeUiaNode::leaf(
                UiaControlType::Text,
                None,
                None,
                Some("Hidden content"),
                false,
                None,
            )
            .with_visibility(visibility);
            let hidden_reads = hidden.inner.text_reads.clone();
            let visible = FakeUiaNode::leaf(
                UiaControlType::Text,
                None,
                None,
                Some("Visible content"),
                false,
                None,
            );
            let tree = FakeUiaNode::with_children(
                UiaControlType::Document,
                None,
                false,
                vec![hidden, visible],
            );
            assert_eq!(extract_uia_text(&tree).as_deref(), Some("Visible content"));
            assert_eq!(hidden_reads.load(Ordering::SeqCst), 0);
        }
    }

    #[test]
    fn windows_uia_offscreen_root_never_reads_text() {
        let root = FakeUiaNode::leaf(
            UiaControlType::Document,
            None,
            None,
            Some("Hidden root content"),
            false,
            None,
        )
        .with_visibility(Some(true));
        assert_eq!(extract_uia_text(&root), None);
        assert_eq!(root.inner.text_reads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn windows_uia_confirmed_document_preserves_http_when_bar_omits_scheme() {
        let bar = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address and search bar"),
            Some("127.0.0.1:19362"),
            None,
            false,
            None,
        );
        let document = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Owned page"),
            Some("http://127.0.0.1:19362/"),
            None,
            false,
            None,
        );
        let tree = FakeUiaNode::with_children(
            UiaControlType::Window,
            Some("Microsoft Edge"),
            false,
            vec![bar, document],
        );
        assert_eq!(
            extract_uia_description(&tree, None).url.as_deref(),
            Some("http://127.0.0.1:19362/")
        );
    }

    #[test]
    fn windows_uia_edited_address_does_not_replace_the_displayed_document() {
        let bar = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address and search bar"),
            Some("https://allowed.invalid/not-submitted"),
            None,
            false,
            None,
        );
        let document = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Excluded page still displayed"),
            Some("https://blocked.example/private"),
            None,
            false,
            None,
        );
        let tree =
            FakeUiaNode::with_children(UiaControlType::Window, None, false, vec![bar, document]);
        assert_eq!(
            extract_uia_description(&tree, None).url.as_deref(),
            Some("https://blocked.example/private")
        );
    }

    #[test]
    fn windows_uia_address_without_document_identity_stays_unknown() {
        let bar = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address and search bar"),
            Some("https://allowed.invalid/"),
            None,
            false,
            None,
        );
        let tree = FakeUiaNode::with_children(UiaControlType::Window, None, false, vec![bar]);
        assert_eq!(extract_uia_description(&tree, None).url, None);
    }

    #[test]
    fn windows_uia_multiple_document_identities_stay_unknown() {
        let bar = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address and search bar"),
            Some("https://allowed.invalid/"),
            None,
            false,
            None,
        );
        let main = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Main"),
            Some("https://blocked.example/"),
            None,
            false,
            None,
        );
        let sidebar = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Sidebar"),
            Some("https://allowed.invalid/"),
            None,
            false,
            None,
        );
        let tree = FakeUiaNode::with_children(
            UiaControlType::Window,
            None,
            false,
            vec![bar, main, sidebar],
        );
        assert_eq!(extract_uia_description(&tree, None).url, None);
    }

    #[test]
    fn windows_uia_scheme_less_document_metadata_never_invents_https() {
        let bar = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address and search bar"),
            Some("example.test/path"),
            None,
            false,
            None,
        );
        let document = FakeUiaNode::leaf(
            UiaControlType::Document,
            None,
            Some("example.test/path"),
            None,
            false,
            None,
        );
        let tree =
            FakeUiaNode::with_children(UiaControlType::Window, None, false, vec![bar, document]);
        assert_eq!(extract_uia_description(&tree, None).url, None);
    }

    #[test]
    fn test_uia_fake_tree_never_extracts_url_from_document_text() {
        let document = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Web content"),
            None,
            Some("Visit https://phishing-site.test for more info"),
            false,
            None,
        );
        let tree = FakeUiaNode::with_children(
            UiaControlType::Window,
            Some("Page - Google Chrome"),
            false,
            vec![document],
        );

        let desc = extract_uia_description(&tree, Some("Page - Google Chrome"));
        assert_eq!(desc.url, None);
    }

    #[test]
    fn test_uia_fake_tree_ignores_generic_page_edit_fields_containing_url() {
        let form_input = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("User Comment"),
            Some("https://user-comment-link.test"),
            None,
            false,
            None,
        );
        let document = FakeUiaNode::with_children(
            UiaControlType::Document,
            Some("Web content"),
            false,
            vec![form_input],
        );
        let tree = FakeUiaNode::with_children(
            UiaControlType::Window,
            Some("Blog - Edge"),
            false,
            vec![document],
        );

        let desc = extract_uia_description(&tree, Some("Blog - Edge"));
        assert_eq!(
            desc.url, None,
            "Generic page edit fields must not be treated as address bar"
        );
    }

    #[test]
    fn test_uia_fake_tree_detects_inprivate_badge_and_clears_url() {
        let inprivate_badge = FakeUiaNode::leaf(
            UiaControlType::Button,
            Some("InPrivate"),
            None,
            None,
            false,
            None,
        );
        let address_bar = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address"),
            Some("https://sensitive-bank.test"),
            None,
            false,
            None,
        );
        let toolbar = FakeUiaNode::with_children(
            UiaControlType::Toolbar,
            Some("Toolbar"),
            false,
            vec![inprivate_badge, address_bar],
        );
        let tree =
            FakeUiaNode::with_children(UiaControlType::Window, Some("Edge"), false, vec![toolbar]);

        let desc = extract_uia_description(&tree, Some("Edge"));
        assert!(desc.private, "InPrivate badge must mark private window");
        assert_eq!(desc.url, None, "Private window URL must be cleared");
    }

    #[test]
    fn test_uia_fake_tree_extracts_visible_content_from_document_and_edit_controls() {
        let doc = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Article"),
            None,
            Some("This is the main article content."),
            false,
            None,
        );
        let edit = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Notes"),
            Some("User draft notes"),
            None,
            false,
            None,
        );
        let tree = FakeUiaNode::with_children(
            UiaControlType::Window,
            Some("Editor"),
            false,
            vec![doc, edit],
        );

        let text = extract_uia_text(&tree).expect("visible text must be extracted");
        assert!(text.contains("This is the main article content."));
        assert!(text.contains("User draft notes"));
    }

    #[test]
    fn test_uia_fake_tree_skips_password_subtree_entirely() {
        let password_child = FakeUiaNode::leaf(
            UiaControlType::Text,
            None,
            None,
            Some("my_secret_pass_child"),
            false,
            None,
        );
        let password_field = FakeUiaNode::with_children(
            UiaControlType::Edit,
            Some("Password"),
            true,
            vec![password_child],
        );
        let username_field = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Username"),
            Some("alice"),
            None,
            false,
            None,
        );
        let form = FakeUiaNode::with_children(
            UiaControlType::Pane,
            Some("Login form"),
            false,
            vec![username_field, password_field],
        );
        let tree =
            FakeUiaNode::with_children(UiaControlType::Window, Some("Login"), false, vec![form]);

        let text = extract_uia_text(&tree).expect("visible non-password text must be extracted");
        assert!(text.contains("alice"), "public username must be extracted");
        assert!(
            !text.contains("my_secret_pass_child"),
            "password subtree must be skipped"
        );
    }

    #[test]
    fn test_uia_fake_tree_respects_node_budget() {
        let head = FakeUiaNode::leaf(
            UiaControlType::Text,
            None,
            None,
            Some("head_marker_public"),
            false,
            None,
        );
        let mut empty_panes = Vec::new();
        for _ in 0..3200 {
            empty_panes.push(FakeUiaNode::leaf(
                UiaControlType::Pane,
                None,
                None,
                None,
                false,
                None,
            ));
        }
        let tail = FakeUiaNode::leaf(
            UiaControlType::Text,
            None,
            None,
            Some("tail_sentinel_beyond_budget"),
            false,
            None,
        );

        let mut children = vec![head];
        children.extend(empty_panes);
        children.push(tail);

        let tree =
            FakeUiaNode::with_children(UiaControlType::Document, Some("Doc"), false, children);

        let text = extract_uia_text(&tree).expect("head text must be extracted");
        assert!(
            text.contains("head_marker_public"),
            "head marker must be read within budget"
        );
        assert!(
            !text.contains("tail_sentinel_beyond_budget"),
            "tail sentinel beyond 3000 nodes budget must not be processed"
        );
    }

    #[test]
    fn test_uia_fake_tree_respects_deadline_budget() {
        let head = FakeUiaNode::leaf(
            UiaControlType::Text,
            None,
            None,
            Some("fast_head_marker"),
            false,
            None,
        );
        let slow_node = FakeUiaNode::leaf(
            UiaControlType::Pane,
            Some("Slow"),
            None,
            None,
            false,
            Some(Duration::from_millis(300)),
        );
        let unreachable_tail = FakeUiaNode::leaf(
            UiaControlType::Text,
            None,
            None,
            Some("unreachable_tail_sentinel"),
            false,
            None,
        );
        let tree = FakeUiaNode::with_children(
            UiaControlType::Window,
            Some("Window"),
            false,
            vec![head, slow_node, unreachable_tail],
        );

        let start = Instant::now();
        let text =
            extract_uia_text(&tree).expect("fast head text must be extracted before deadline");
        let elapsed = start.elapsed();
        assert!(
            text.contains("fast_head_marker"),
            "fast head text must be extracted"
        );
        assert!(
            !text.contains("unreachable_tail_sentinel"),
            "tail sentinel after deadline must not be reached"
        );
        assert!(
            elapsed < Duration::from_millis(700),
            "deadline must bound traversal"
        );
    }

    #[test]
    fn test_uia_fake_tree_ignores_inpage_spoofed_address_and_private_markers_inside_document() {
        let spoofed_address = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address and search bar"),
            Some("https://attacker.invalid"),
            None,
            false,
            None,
        );
        let inpage_private_text = FakeUiaNode::leaf(
            UiaControlType::Text,
            Some("InPrivate"),
            None,
            Some("This page mentions InPrivate browsing"),
            false,
            None,
        );
        let document = FakeUiaNode::with_children(
            UiaControlType::Document,
            Some("Web page content"),
            false,
            vec![spoofed_address, inpage_private_text],
        )
        .with_document_url("https://example.org/docs");

        let real_omnibox = FakeUiaNode::leaf(
            UiaControlType::Edit,
            Some("Address and search bar"),
            Some("https://example.org"),
            None,
            false,
            None,
        );
        let toolbar = FakeUiaNode::with_children(
            UiaControlType::Toolbar,
            Some("Browser chrome"),
            false,
            vec![real_omnibox],
        );

        let window = FakeUiaNode::with_children(
            UiaControlType::Window,
            Some("Edge"),
            false,
            vec![toolbar, document],
        );

        let desc = extract_uia_description(&window, Some("Edge"));
        assert_eq!(desc.url.as_deref(), Some("https://example.org/docs"));
        assert!(
            !desc.private,
            "In-page text inside Document must not trigger private mode"
        );
    }

    #[test]
    fn test_uia_fake_tree_unicode_20k_char_cap_does_not_panic() {
        let euro_body: String = "€".repeat(7000);
        let doc = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Euro document"),
            None,
            Some(&euro_body),
            false,
            None,
        );
        let tree =
            FakeUiaNode::with_children(UiaControlType::Window, Some("Window"), false, vec![doc]);

        let text = extract_uia_text(&tree).expect("text extracted without panic");
        // 7,000 characters is well within the MAX_CHARS (20,000 characters) budget.
        // It must NOT be truncated down to 6,666 characters.
        assert_eq!(
            text.chars().count(),
            7000,
            "7000 characters must be fully preserved within the 20k character budget"
        );
    }
    #[test]
    fn test_uia_fake_tree_window_and_toolbar_only_text_returns_none_for_ocr() {
        let button = FakeUiaNode::leaf(
            UiaControlType::Button,
            Some("Save"),
            None,
            None,
            false,
            None,
        );
        let toolbar = FakeUiaNode::with_children(
            UiaControlType::Toolbar,
            Some("Toolbar"),
            false,
            vec![button],
        );
        let tree = FakeUiaNode::with_children(
            UiaControlType::Window,
            Some("A Long Window Title With More Than Twenty Characters"),
            false,
            vec![toolbar],
        );

        let text = extract_uia_text(&tree);
        assert_eq!(
            text, None,
            "window title and toolbar chrome alone must not count as accessible document text"
        );
    }

    #[test]
    fn test_uia_fake_tree_unicode_exceeding_20k_chars_is_capped_at_20k_characters() {
        let large_euro_body: String = "€".repeat(25000);
        let doc = FakeUiaNode::leaf(
            UiaControlType::Document,
            Some("Large Euro document"),
            None,
            Some(&large_euro_body),
            false,
            None,
        );
        let tree =
            FakeUiaNode::with_children(UiaControlType::Window, Some("Window"), false, vec![doc]);

        let text = extract_uia_text(&tree).expect("text extracted without panic");
        // When exceeding 20,000 characters, it must be capped at 20,000 characters (not bytes).
        assert_eq!(
            text.chars().count(),
            20_000,
            "document text exceeding 20k characters must be capped at exactly 20,000 characters"
        );
    }
}
