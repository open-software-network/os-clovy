//! Windows window identity and enumeration (D2, D6).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND};
use windows::Win32::Graphics::Gdi::{HDC, HMONITOR};

use crate::activity::platform::WindowDescriptor;

const EXEC_PATH_CACHE_LIMIT: usize = 256;

static EXEC_PATH_CACHE: LazyLock<Mutex<HashMap<String, PathBuf>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// RAII wrapper for Win32 `HANDLE` ensuring `CloseHandle` is called on drop.
struct AutoHandle(HANDLE);

impl Drop for AutoHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() && !self.0 .0.is_null() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Normalizes executable path to a lowercase filename (e.g. `chrome.exe`).
pub(crate) fn normalize_executable_name(path: &Path) -> String {
    path.file_name()
        .and_then(|f| f.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Resolves `(app_name, bundle_id)` from observed process image path, FileDescription,
/// and optional AppUserModelID (AUMID).
///
/// Rules (D2):
/// - `bundle_id`: AUMID preferred for packaged apps (UWP/MSIX); lowercase executable name otherwise.
/// - `app_name`: `FileDescription` from executable version info; fallback to file stem without extension.
pub(crate) fn resolve_identity(
    image_path: Option<&Path>,
    file_description: Option<&str>,
    aumid: Option<&str>,
) -> (String, Option<String>) {
    let bundle_id = if let Some(aumid) = aumid.filter(|s| !s.trim().is_empty()) {
        Some(aumid.trim().to_string())
    } else if let Some(path) = image_path {
        let norm = normalize_executable_name(path);
        if norm.is_empty() {
            None
        } else {
            Some(norm)
        }
    } else {
        None
    };

    let app_name = if let Some(desc) = file_description.filter(|s| !s.trim().is_empty()) {
        desc.trim().to_string()
    } else if let Some(path) = image_path {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Unknown".to_string())
    } else {
        "Unknown".to_string()
    };

    (app_name, bundle_id)
}

/// Records an observed mapping from `bundle_id` to executable `path` in the bounded cache.
pub(crate) fn record_executable_path(bundle_id: &str, path: PathBuf) {
    if bundle_id.is_empty() || !path.is_file() {
        return;
    }
    if let Ok(mut cache) = EXEC_PATH_CACHE.lock() {
        if cache.len() >= EXEC_PATH_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(bundle_id.to_string(), path);
    }
}

/// Validates that a bundle_id string is safe for lookup and contains no path traversal elements.
fn is_safe_bundle_id(bundle_id: &str) -> bool {
    !bundle_id.is_empty()
        && bundle_id.len() <= 255
        && !bundle_id.contains("..")
        && !bundle_id.contains('/')
        && !bundle_id.contains('\\')
        && bundle_id.chars().all(|c| !c.is_control())
}

/// Resolves the executable path on disk for a given `bundle_id` (executable name or AUMID).
///
/// Bounded cache of observed identities -> paths, with fallback to SearchPath / App Paths.
/// NEVER executes the file.
pub(crate) fn executable_path(bundle_id: &str) -> Option<PathBuf> {
    if !is_safe_bundle_id(bundle_id) {
        return None;
    }

    if let Ok(cache) = EXEC_PATH_CACHE.lock() {
        if let Some(path) = cache.get(bundle_id) {
            return Some(path.clone());
        }
    }

    // Fallback 1: App Paths registry lookup (HKLM and HKCU)
    if let Some(path) = resolve_from_app_paths(bundle_id) {
        if path.is_file() {
            record_executable_path(bundle_id, path.clone());
            return Some(path);
        }
    }

    // Fallback 2: SearchPathW
    if let Some(path) = resolve_from_search_path(bundle_id) {
        if path.is_file() {
            record_executable_path(bundle_id, path.clone());
            return Some(path);
        }
    }

    None
}

/// Expands `%VAR%` environment variable references within a string.
fn expand_environment_variables(input: &str) -> Option<String> {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;

    let source: Vec<u16> = input.encode_utf16().chain(std::iter::once(0)).collect();
    let source = windows::core::PCWSTR(source.as_ptr());
    let required = unsafe { ExpandEnvironmentStringsW(source, None) };
    if required == 0 {
        return None;
    }
    let mut expanded = vec![0u16; required as usize];
    let written = unsafe { ExpandEnvironmentStringsW(source, Some(&mut expanded)) };
    if written == 0 || written > required {
        return None;
    }
    Some(String::from_utf16_lossy(&expanded[..written as usize - 1]))
}

/// Decodes and validates an executable path from an `App Paths` registry value.
fn decode_app_paths_registry_value(
    val_type: windows::Win32::System::Registry::REG_VALUE_TYPE,
    buf: &[u16],
) -> Option<PathBuf> {
    use windows::Win32::System::Registry::{REG_EXPAND_SZ, REG_SZ};

    if (val_type == REG_SZ || val_type == REG_EXPAND_SZ) && !buf.is_empty() {
        let char_len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let raw_str = String::from_utf16_lossy(&buf[..char_len]);
        let trimmed = raw_str.trim().trim_matches('"');
        if !trimmed.is_empty() {
            let resolved_str = if val_type == REG_EXPAND_SZ {
                expand_environment_variables(trimmed)?
            } else {
                trimmed.to_string()
            };
            let p = PathBuf::from(resolved_str);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Fallback resolver via Windows Registry `App Paths`.
fn resolve_from_app_paths(bundle_id: &str) -> Option<PathBuf> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_READ, REG_VALUE_TYPE,
    };

    let subkey = format!(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\{}\0",
        bundle_id
    );
    let wide_subkey: Vec<u16> = subkey.encode_utf16().collect();

    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let mut hkey = HKEY::default();
        let status = unsafe {
            RegOpenKeyExW(
                root,
                PCWSTR(wide_subkey.as_ptr()),
                None,
                KEY_READ,
                &mut hkey,
            )
        };
        if status.is_ok() {
            let mut buf = [0u16; 1024];
            let mut size = (buf.len() * 2) as u32;
            let mut val_type = REG_VALUE_TYPE::default();
            let query = unsafe {
                RegQueryValueExW(
                    hkey,
                    PCWSTR::null(),
                    None,
                    Some(&mut val_type),
                    Some(buf.as_mut_ptr() as *mut u8),
                    Some(&mut size),
                )
            };
            unsafe {
                let _ = RegCloseKey(hkey);
            };
            if query.is_ok() && size > 2 {
                let char_len = size as usize / 2;
                if let Some(p) = decode_app_paths_registry_value(val_type, &buf[..char_len]) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// Fallback resolver via `SearchPathW`.
fn resolve_from_search_path(bundle_id: &str) -> Option<PathBuf> {
    use windows::Win32::Storage::FileSystem::SearchPathW;

    let wide_name: Vec<u16> = bundle_id.encode_utf16().chain(Some(0)).collect();
    let wide_ext: Vec<u16> = if bundle_id.ends_with(".exe") {
        Vec::new()
    } else {
        ".exe\0".encode_utf16().collect()
    };
    let ext_ptr = if wide_ext.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(wide_ext.as_ptr())
    };

    let mut buffer = [0u16; 1024];
    let len = unsafe {
        SearchPathW(
            PCWSTR::null(),
            PCWSTR(wide_name.as_ptr()),
            ext_ptr,
            Some(&mut buffer),
            None,
        )
    };
    if len > 0 && (len as usize) < buffer.len() {
        let found = String::from_utf16_lossy(&buffer[..len as usize]);
        let p = PathBuf::from(found);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Returns the focused window descriptor on the main display, or `None` if
/// no foreign window is focused.
pub(crate) fn focused_window() -> Option<WindowDescriptor> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId, IsWindow, IsWindowVisible,
    };

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null()
        || !unsafe { IsWindow(Some(hwnd)) }.as_bool()
        || !unsafe { IsWindowVisible(hwnd) }.as_bool()
    {
        return None;
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    if pid == 0 || pid == std::process::id() {
        return None;
    }

    describe_window_handle(hwnd, pid)
}

/// Process that owns the foreground window, including Clovy itself; `None`
/// when there is no foreground window.
pub(crate) fn foreground_window_pid() -> Option<u32> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

/// App name and identity (executable or AUMID) of the window the dictation
/// helper pinned, by its handle and process. Records the executable so the
/// app icon resolver can draw the same app's icon.
pub(crate) fn app_of_window(hwnd: isize, pid: u32) -> Option<(String, String)> {
    if hwnd == 0 || pid == 0 {
        return None;
    }
    let descriptor = describe_window_handle(HWND(hwnd as *mut std::ffi::c_void), pid)?;
    Some((descriptor.app_name, descriptor.bundle_id?))
}

/// Inspects a window handle and process ID to produce a `WindowDescriptor`.
fn describe_window_handle(hwnd: HWND, pid: u32) -> Option<WindowDescriptor> {
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_FORMAT,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let _process_guard = AutoHandle(process);

    // Process image path
    let mut image_buf = [0u16; 1024];
    let mut image_len = image_buf.len() as u32;
    let image_path = if unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_FORMAT(0),
            PWSTR(image_buf.as_mut_ptr()),
            &mut image_len,
        )
    }
    .is_ok()
        && image_len > 0
    {
        Some(PathBuf::from(String::from_utf16_lossy(
            &image_buf[..image_len as usize],
        )))
    } else {
        None
    };

    // AUMID (AppUserModelID)
    let aumid = get_process_aumid(process);

    // FileDescription from executable version info
    let file_desc = image_path.as_deref().and_then(get_file_description);

    // Identity resolution
    let (app_name, bundle_id) = resolve_identity(
        image_path.as_deref(),
        file_desc.as_deref(),
        aumid.as_deref(),
    );

    // Record in executable_path cache if both identity and path are available
    if let (Some(bid), Some(path)) = (&bundle_id, &image_path) {
        record_executable_path(bid, path.clone());
    }

    // Window title
    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    let window_title = if title_len > 0 {
        let mut title_buf = vec![0u16; (title_len + 1) as usize];
        let read = unsafe { GetWindowTextW(hwnd, &mut title_buf) };
        if read > 0 {
            let t = String::from_utf16_lossy(&title_buf[..read as usize]);
            let trimmed = t.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        } else {
            None
        }
    } else {
        None
    };

    // Window ID: HWND converted to u32 (low 32 bits, restored with sign extension)
    let window_id = (hwnd.0 as isize) as u32;

    // Display ID: monitor index
    let hmon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let display_id = get_monitor_index(hmon);

    Some(WindowDescriptor {
        pid: pid as i32,
        app_name,
        bundle_id,
        window_title,
        browser_url: None,
        private_window: false,
        window_id: Some(window_id),
        display_id: Some(display_id),
    })
}

/// Reads AppUserModelID (AUMID) for packaged Windows store / UWP apps.
fn get_process_aumid(process: HANDLE) -> Option<String> {
    use windows::Win32::Foundation::WIN32_ERROR;
    use windows::Win32::Storage::Packaging::Appx::GetApplicationUserModelId;

    let mut len: u32 = 0;
    let err = unsafe { GetApplicationUserModelId(process, &mut len, None) };
    // ERROR_INSUFFICIENT_BUFFER is 122
    if err == WIN32_ERROR(122) && len > 0 {
        let mut buf = vec![0u16; len as usize];
        let err2 =
            unsafe { GetApplicationUserModelId(process, &mut len, Some(PWSTR(buf.as_mut_ptr()))) };
        if err2 == WIN32_ERROR(0) {
            let null_pos = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let s = String::from_utf16_lossy(&buf[..null_pos]);
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Reads localized `FileDescription` from an executable file's version resource block.
fn get_file_description(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW,
    };

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let size = unsafe { GetFileVersionInfoSizeW(PCWSTR(wide_path.as_ptr()), None) };
    if size == 0 {
        return None;
    }

    let mut data = vec![0u8; size as usize];
    if unsafe {
        GetFileVersionInfoW(
            PCWSTR(wide_path.as_ptr()),
            Some(0),
            size,
            data.as_mut_ptr() as *mut _,
        )
    }
    .is_err()
    {
        return None;
    }

    // Query \VarFileInfo\Translation to find lang-codepage pairs
    let mut trans_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut trans_len: u32 = 0;
    let trans_subblock: Vec<u16> = "\\VarFileInfo\\Translation\0".encode_utf16().collect();
    let has_trans = unsafe {
        VerQueryValueW(
            data.as_ptr() as *const _,
            PCWSTR(trans_subblock.as_ptr()),
            &mut trans_ptr,
            &mut trans_len,
        )
    };

    let mut lang_codepage_pairs = Vec::new();
    if has_trans.as_bool() && trans_len >= 4 && !trans_ptr.is_null() {
        let count = (trans_len as usize) / 4;
        let slice = unsafe { std::slice::from_raw_parts(trans_ptr as *const u16, count * 2) };
        for i in 0..count {
            let lang = slice[i * 2];
            let codepage = slice[i * 2 + 1];
            lang_codepage_pairs.push(format!("{:04x}{:04x}", lang, codepage));
        }
    }
    // Fallback standard pairs:
    lang_codepage_pairs.push("040904b0".to_string()); // US English Unicode
    lang_codepage_pairs.push("040904e4".to_string()); // US English Windows Multilingual
    lang_codepage_pairs.push("000004b0".to_string()); // Neutral Unicode

    for pair in lang_codepage_pairs {
        let subblock: Vec<u16> = format!("\\StringFileInfo\\{}\\FileDescription\0", pair)
            .encode_utf16()
            .collect();
        let mut desc_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
        let mut desc_len: u32 = 0;
        let ok = unsafe {
            VerQueryValueW(
                data.as_ptr() as *const _,
                PCWSTR(subblock.as_ptr()),
                &mut desc_ptr,
                &mut desc_len,
            )
        };
        if ok.as_bool() && desc_len > 0 && !desc_ptr.is_null() {
            let slice =
                unsafe { std::slice::from_raw_parts(desc_ptr as *const u16, desc_len as usize) };
            let null_pos = slice.iter().position(|&c| c == 0).unwrap_or(slice.len());
            let text = String::from_utf16_lossy(&slice[..null_pos]);
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    None
}

struct DisplayMonitor {
    hmonitor: HMONITOR,
    index: u32,
    is_primary: bool,
}

/// Enumerates display monitors via Win32 `EnumDisplayMonitors`.
fn enumerate_display_monitors() -> Vec<DisplayMonitor> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, MONITORINFO};

    extern "system" fn enum_mon_proc(
        hmon: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        lparam: LPARAM,
    ) -> BOOL {
        let monitors = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
        monitors.push(hmon);
        BOOL(1)
    }

    let mut raw_monitors = Vec::new();
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enum_mon_proc),
            LPARAM(&mut raw_monitors as *mut _ as isize),
        )
    };

    let mut result = Vec::new();
    for (i, &hmon) in raw_monitors.iter().enumerate() {
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let is_primary = if unsafe { GetMonitorInfoW(hmon, &mut mi) }.as_bool() {
            // MONITORINFOF_PRIMARY is 0x00000001
            (mi.dwFlags & 1) != 0
        } else {
            i == 0
        };
        result.push(DisplayMonitor {
            hmonitor: hmon,
            index: i as u32,
            is_primary,
        });
    }
    result
}

/// Finds the index of a given monitor among active displays.
fn get_monitor_index(target_hmon: HMONITOR) -> u32 {
    let monitors = enumerate_display_monitors();
    monitors
        .iter()
        .find(|m| m.hmonitor == target_hmon)
        .map(|m| m.index)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SecondaryWindowAction {
    Skip,
    Stop,
    Capture,
}

/// Distinguishes an irrelevant window from one that already covers this monitor.
fn secondary_window_action(
    hwnd: HWND,
    focused_hwnd: Option<HWND>,
    is_visible: bool,
    is_iconic: bool,
    ex_style: u32,
    rect: windows::Win32::Foundation::RECT,
    win_monitor: HMONITOR,
    sec_monitor: HMONITOR,
    pid: u32,
    own_pid: u32,
) -> SecondaryWindowAction {
    if !is_visible || is_iconic {
        return SecondaryWindowAction::Skip;
    }
    if (ex_style & windows::Win32::UI::WindowsAndMessaging::WS_EX_TOOLWINDOW.0) != 0 {
        return SecondaryWindowAction::Skip;
    }
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return SecondaryWindowAction::Skip;
    }
    if win_monitor != sec_monitor {
        return SecondaryWindowAction::Skip;
    }
    if pid == 0 {
        return SecondaryWindowAction::Skip;
    }

    // This window still covers the display even if it must not be captured.
    if pid == own_pid || Some(hwnd) == focused_hwnd {
        return SecondaryWindowAction::Stop;
    }

    SecondaryWindowAction::Capture
}

/// Returns the topmost visible window on each secondary monitor (D6).
///
/// Enumerates monitors via `EnumDisplayMonitors` and finds the topmost window in Z-order
/// via `EnumWindows`.
pub(crate) fn secondary_windows() -> Vec<WindowDescriptor> {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONULL};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
        IsIconic, IsWindowVisible, GWL_EXSTYLE,
    };

    let monitors = enumerate_display_monitors();
    let mut secondary_monitors = monitors.into_iter().filter(|m| !m.is_primary).peekable();
    if secondary_monitors.peek().is_none() {
        return Vec::new();
    }

    let focused_hwnd = {
        let h = unsafe { GetForegroundWindow() };
        if h.0.is_null() {
            None
        } else {
            Some(h)
        }
    };

    // Collect top-level windows in Z-order
    extern "system" fn enum_win_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let list = unsafe { &mut *(lparam.0 as *mut Vec<HWND>) };
        list.push(hwnd);
        BOOL(1)
    }

    let mut z_windows = Vec::new();
    let _ = unsafe {
        EnumWindows(
            Some(enum_win_proc),
            LPARAM(&mut z_windows as *mut _ as isize),
        )
    };

    let own_pid = std::process::id();
    let mut descriptors = Vec::new();

    for sec in secondary_monitors {
        for &hwnd in &z_windows {
            let is_visible = unsafe { IsWindowVisible(hwnd) }.as_bool();
            let is_iconic = unsafe { IsIconic(hwnd) }.as_bool();
            let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) } as u32;
            let mut rect = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut rect) }.is_err() {
                continue;
            }
            let hmon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL) };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };

            match secondary_window_action(
                hwnd,
                focused_hwnd,
                is_visible,
                is_iconic,
                ex_style,
                rect,
                hmon,
                sec.hmonitor,
                pid,
                own_pid,
            ) {
                SecondaryWindowAction::Skip => continue,
                SecondaryWindowAction::Stop => break,
                SecondaryWindowAction::Capture => {}
            }

            // Found topmost candidate on this monitor
            if let Some(desc) = describe_window_handle(hwnd, pid) {
                descriptors.push(desc);
            }
            // A failed descriptor must not expose a covered window either.
            break;
        }
    }

    descriptors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_executable_name_handles_mixed_case_and_path() {
        let path1 = Path::new(r"C:\Program Files\Google\Chrome\Application\CHROME.EXE");
        assert_eq!(normalize_executable_name(path1), "chrome.exe");

        let path2 = Path::new(r"msedge.exe");
        assert_eq!(normalize_executable_name(path2), "msedge.exe");

        let path3 = Path::new(r"C:\Windows\System32\CMD.EXE");
        assert_eq!(normalize_executable_name(path3), "cmd.exe");

        let path4 = Path::new(r"C:\Program Files\Microsoft VS Code\Code.exe");
        assert_eq!(normalize_executable_name(path4), "code.exe");
    }

    #[test]
    fn test_resolve_identity_prefers_aumid_for_packaged_apps() {
        let image = Path::new(
            r"C:\Program Files\WindowsApps\Microsoft.WindowsTerminal_1.18.10301.0_x64__8wekyb3d8bbwe\WindowsTerminal.exe",
        );
        let (app_name, bundle_id) = resolve_identity(
            Some(image),
            Some("Windows Terminal"),
            Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App"),
        );
        assert_eq!(app_name, "Windows Terminal");
        assert_eq!(
            bundle_id.as_deref(),
            Some("Microsoft.WindowsTerminal_8wekyb3d8bbwe!App")
        );
    }

    #[test]
    fn test_resolve_identity_uses_lowercase_exe_when_no_aumid() {
        let image = Path::new(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe");
        let (app_name, bundle_id) = resolve_identity(Some(image), Some("Microsoft Edge"), None);
        assert_eq!(app_name, "Microsoft Edge");
        assert_eq!(bundle_id.as_deref(), Some("msedge.exe"));
    }

    #[test]
    fn test_resolve_identity_falls_back_to_stem_when_file_description_empty_or_whitespace() {
        let image = Path::new(r"C:\Windows\System32\cmd.exe");
        let (app_name, bundle_id) = resolve_identity(Some(image), Some("   "), None);
        assert_eq!(app_name, "cmd");
        assert_eq!(bundle_id.as_deref(), Some("cmd.exe"));
    }

    #[test]
    fn test_resolve_identity_falls_back_to_stem_when_file_description_none() {
        let image = Path::new(r"C:\Tools\custom_utility.exe");
        let (app_name, bundle_id) = resolve_identity(Some(image), None, None);
        assert_eq!(app_name, "custom_utility");
        assert_eq!(bundle_id.as_deref(), Some("custom_utility.exe"));
    }

    #[test]
    fn test_executable_path_caching_and_lookup() {
        // Querying non-existent or invalid bundle_id returns None
        assert_eq!(executable_path("non_existent_app_12345.exe"), None);
        assert_eq!(executable_path("../../Windows/explorer.exe"), None);
        assert_eq!(executable_path(""), None);

        // Recording in cache and retrieving
        let dummy_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("dummy.exe"));
        if dummy_path.is_file() {
            record_executable_path("my_dummy_app.exe", dummy_path.clone());
            assert_eq!(executable_path("my_dummy_app.exe"), Some(dummy_path));
        }
    }

    #[test]
    fn test_observed_valid_unicode_and_spaced_exe_resolves_executable_path_and_icon() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spaced_exe = dir.path().join("my spaced ápp.exe");
        std::fs::write(&spaced_exe, b"MZ").expect("write temp file");

        let bundle_id = "my spaced ápp.exe";
        record_executable_path(bundle_id, spaced_exe.clone());

        assert_eq!(
            executable_path(bundle_id),
            Some(spaced_exe),
            "observed executable with space and Unicode must resolve in executable_path"
        );

        #[cfg(target_os = "windows")]
        {
            let sys_cmd = Path::new(r"C:\Windows\System32\cmd.exe");
            if sys_cmd.is_file() {
                let real_spaced_exe = dir.path().join("cmd spaced ápp.exe");
                let _ = std::fs::copy(sys_cmd, &real_spaced_exe);
                let real_bundle = "cmd spaced ápp.exe";
                record_executable_path(real_bundle, real_spaced_exe);
                let icon = crate::activity::app_icon::app_icon_data_url(real_bundle);
                assert!(
                    icon.is_some(),
                    "app_icon_data_url must resolve icon for valid spaced/unicode observed exe"
                );
            }
        }
    }

    #[test]
    fn test_registry_app_paths_decoding_expands_environment_strings() {
        use windows::Win32::System::Registry::REG_EXPAND_SZ;

        let raw_val = r"%SystemRoot%\notepad.exe";
        let wide: Vec<u16> = raw_val.encode_utf16().chain(Some(0)).collect();

        let decoded = decode_app_paths_registry_value(REG_EXPAND_SZ, &wide);

        // Before expansion fix, decode_app_paths_registry_value tests is_file() on unexpanded
        // "%SystemRoot%\notepad.exe", which fails and returns None.
        assert!(
            decoded.is_some(),
            "REG_EXPAND_SZ must expand environment strings to resolve installed executable"
        );
        let path = decoded.unwrap();
        assert!(path.is_file(), "expanded path must point to an actual file");
    }

    #[test]
    fn test_secondary_window_selection_stops_at_focused_window() {
        use windows::Win32::Foundation::RECT;

        let secondary_hmon = HMONITOR(0x1000 as *mut _);
        let focused_hwnd = HWND(0x2000 as *mut _);

        let action = secondary_window_action(
            focused_hwnd, // Candidate window IS the currently focused window on this secondary monitor
            Some(focused_hwnd),
            true,  // is_visible
            false, // is_iconic
            0,     // ex_style
            RECT {
                left: 0,
                top: 0,
                right: 800,
                bottom: 600,
            },
            secondary_hmon,
            secondary_hmon,
            1234, // pid
            9999, // own_pid
        );

        // Skipping would continue down the Z-order into a covered window.
        assert_eq!(
            action,
            SecondaryWindowAction::Stop,
            "a monitor already covered by the focused window must not sample behind it"
        );
    }

    #[test]
    fn test_secondary_window_selection_stops_at_own_window() {
        use windows::Win32::Foundation::RECT;

        let monitor = HMONITOR(0x1000 as *mut _);
        let action = secondary_window_action(
            HWND(0x3000 as *mut _),
            None,
            true,
            false,
            0,
            RECT {
                left: 0,
                top: 0,
                right: 800,
                bottom: 600,
            },
            monitor,
            monitor,
            9999,
            9999,
        );
        assert_eq!(
            action,
            SecondaryWindowAction::Stop,
            "an excluded Clovy window still covers the window behind it"
        );
    }
}
