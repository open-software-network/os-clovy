//! Windows microphone-ownership probe.
//!
//! The reader (WASAPI capture sessions, a Toolhelp process snapshot and the
//! microphone consent store) only collects raw data; `windows_input_processes`
//! turns that data into the same `MicrophoneInputProcess` list the macOS
//! CoreAudio probe produces, so the parser is testable on any host.
#![cfg_attr(not(target_os = "windows"), allow(dead_code))]

use super::{bundle_family_from_bundle_id, MicrophoneInputProcess};
use std::collections::{BTreeSet, HashMap};

/// WebView2 runs in `msedgewebview2.exe` children of the app hosting it; a
/// capture session there belongs to that app (Teams, WhatsApp, or Clovy).
const WEBVIEW2_EXECUTABLE: &str = "msedgewebview2.exe";
/// Bounds the parent walk; real Chromium and WebView2 trees are 2-3 deep.
const PARENT_WALK_LIMIT: usize = 16;

/// A Windows meeting app, identified by its lowercase executable names or,
/// for packaged apps seen through the microphone consent store, by the
/// package name of its family name (`MSTeams` in `MSTeams_8wekyb3d8bbwe`).
/// Every identity of one app shares `family`, so the meeting-end tracker sees
/// one app however the probe found it.
pub(crate) struct WindowsMicApp {
    pub(crate) family: &'static str,
    pub(crate) label: &'static str,
    executables: &'static [&'static str],
    package_names: &'static [&'static str],
}

const fn windows_mic_app(
    family: &'static str,
    label: &'static str,
    executables: &'static [&'static str],
    package_names: &'static [&'static str],
) -> WindowsMicApp {
    WindowsMicApp {
        family,
        label,
        executables,
        package_names,
    }
}

const WINDOWS_MIC_APPS: &[WindowsMicApp] = &[
    windows_mic_app("windows:zoom", "Zoom", &["zoom.exe"], &[]),
    windows_mic_app(
        "windows:teams",
        "Teams",
        &["teams.exe", "ms-teams.exe", "msteams.exe"],
        &["MSTeams"],
    ),
    windows_mic_app(
        "windows:slack",
        "Slack",
        &["slack.exe"],
        &["91750D7E.Slack"],
    ),
    windows_mic_app(
        "windows:discord",
        "Discord",
        &["discord.exe", "discordptb.exe", "discordcanary.exe"],
        &[],
    ),
    windows_mic_app(
        "windows:whatsapp",
        "WhatsApp",
        &["whatsapp.exe", "whatsapp.root.exe"],
        &["5319275A.WhatsAppDesktop"],
    ),
    windows_mic_app(
        "windows:telegram",
        "Telegram",
        &["telegram.exe"],
        &["TelegramMessengerLLP.TelegramDesktop"],
    ),
    windows_mic_app("windows:signal", "Signal", &["signal.exe"], &[]),
    windows_mic_app(
        "windows:webex",
        "Webex",
        &[
            "webex.exe",
            "ciscocollabhost.exe",
            "atmgr.exe",
            "webexmta.exe",
        ],
        &[],
    ),
    windows_mic_app("windows:chrome", "Chrome", &["chrome.exe"], &[]),
    windows_mic_app("windows:chromium", "Chromium", &["chromium.exe"], &[]),
    windows_mic_app("windows:edge", "Edge", &["msedge.exe"], &[]),
    windows_mic_app("windows:firefox", "Firefox", &["firefox.exe"], &[]),
    windows_mic_app("windows:brave", "Brave", &["brave.exe"], &[]),
    windows_mic_app("windows:opera", "Opera", &["opera.exe"], &[]),
    windows_mic_app("windows:vivaldi", "Vivaldi", &["vivaldi.exe"], &[]),
    windows_mic_app(
        "windows:arc",
        "Arc",
        &["arc.exe"],
        &["TheBrowserCompany.Arc"],
    ),
    windows_mic_app("windows:comet", "Comet", &["comet.exe"], &[]),
    windows_mic_app("windows:zen", "Zen", &["zen.exe"], &[]),
    windows_mic_app("windows:helium", "Helium", &["helium.exe"], &[]),
];

/// Looks up a Windows identity: an executable name (`chrome.exe`), or a
/// package family name / AUMID (`MSTeams_8wekyb3d8bbwe[!App]`). macOS bundle
/// ids are neither, so the macOS and Windows tables never claim each other's
/// identities.
pub(crate) fn find_windows_mic_app(identity: &str) -> Option<&'static WindowsMicApp> {
    let identity = identity.trim().to_ascii_lowercase();
    if identity.ends_with(".exe") {
        return WINDOWS_MIC_APPS
            .iter()
            .find(|app| app.executables.contains(&identity.as_str()));
    }
    let family_name = identity.split('!').next().unwrap_or_default();
    let (package_name, _publisher_id) = family_name.rsplit_once('_')?;
    WINDOWS_MIC_APPS.iter().find(|app| {
        app.package_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(package_name))
    })
}

/// One row of the Toolhelp process snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProcessEntry {
    pub(crate) pid: u32,
    pub(crate) parent_pid: u32,
    pub(crate) exe_name: String,
}

/// One subkey of `CapabilityAccessManager\ConsentStore\microphone`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConsentEntry {
    /// Package family name for packaged apps, `#`-separated path for
    /// `NonPackaged` ones.
    pub(crate) key: String,
    pub(crate) packaged: bool,
    pub(crate) last_used_time_start: u64,
    pub(crate) last_used_time_stop: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProbeSnapshot {
    /// Process ids of capture sessions in `AudioSessionStateActive`.
    pub(crate) active_capture_pids: Vec<u32>,
    pub(crate) processes: Vec<ProcessEntry>,
    pub(crate) consent: Vec<ConsentEntry>,
}

/// Active capture sessions, each lifted to its app's root process, plus
/// allowed apps the consent store reports in use that no session showed.
/// Unlisted session owners stay in the list (as on macOS) so the meeting-end
/// probe can treat them as ambiguous; unlisted consent entries are dropped.
pub(crate) fn windows_input_processes(snapshot: &ProbeSnapshot) -> Vec<MicrophoneInputProcess> {
    let by_pid = snapshot
        .processes
        .iter()
        .map(|process| (process.pid, process))
        .collect::<HashMap<_, _>>();

    let mut processes = snapshot
        .active_capture_pids
        .iter()
        .filter(|pid| **pid != 0)
        .filter_map(|pid| by_pid.get(pid))
        .map(|process| app_root(process, &by_pid))
        .filter_map(|root| {
            MicrophoneInputProcess::new(root.pid, normalize_executable_name(&root.exe_name))
        })
        .collect::<Vec<_>>();

    let found_families = processes
        .iter()
        .filter_map(|process| bundle_family_from_bundle_id(&process.bundle_id))
        .collect::<BTreeSet<_>>();
    for entry in &snapshot.consent {
        if entry.last_used_time_start == 0 || entry.last_used_time_stop != 0 {
            continue;
        }
        let Some((family, executables)) = consent_entry_app(entry) else {
            continue;
        };
        if found_families.contains(family) {
            continue;
        }
        let root = snapshot
            .processes
            .iter()
            .filter(|process| executables.contains(&normalize_executable_name(&process.exe_name)))
            .map(|process| app_root(process, &by_pid))
            .min_by_key(|root| root.pid);
        if let Some(process) = root.and_then(|root| {
            MicrophoneInputProcess::new(root.pid, normalize_executable_name(&root.exe_name))
        }) {
            processes.push(process);
        }
    }

    processes.sort_by_key(|process| process.pid);
    processes.dedup_by_key(|process| process.pid);
    processes
}

/// The allowed app a consent-store entry names: its family and the
/// executables whose running process proves the entry is live.
fn consent_entry_app(entry: &ConsentEntry) -> Option<(&'static str, Vec<String>)> {
    if entry.packaged {
        let app = find_windows_mic_app(&entry.key)?;
        let executables = app.executables.iter().map(|exe| exe.to_string()).collect();
        return Some((app.family, executables));
    }
    let executable = normalize_executable_name(entry.key.rsplit(['#', '\\']).next()?);
    let app = find_windows_mic_app(&executable)?;
    Some((app.family, vec![executable]))
}

/// Climbs from a capture-session process to the process that owns the app:
/// out of WebView2 into its host, and from a Chromium or Firefox child
/// (same executable) to the browser process. Stops at cycles and at the
/// first parent that is a different app.
fn app_root<'a>(
    process: &'a ProcessEntry,
    by_pid: &HashMap<u32, &'a ProcessEntry>,
) -> &'a ProcessEntry {
    let mut current = process;
    let mut visited = BTreeSet::from([current.pid]);
    for _ in 0..PARENT_WALK_LIMIT {
        let Some(parent) = by_pid.get(&current.parent_pid).copied() else {
            break;
        };
        if !visited.insert(parent.pid) {
            break;
        }
        let current_exe = normalize_executable_name(&current.exe_name);
        let climbs = current_exe == WEBVIEW2_EXECUTABLE
            || normalize_executable_name(&parent.exe_name) == current_exe;
        if !climbs {
            break;
        }
        current = parent;
    }
    current
}

fn normalize_executable_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[cfg(target_os = "windows")]
pub(crate) fn active_input_processes() -> Result<Vec<MicrophoneInputProcess>, super::ProbeError> {
    let active_capture_pids = read::active_capture_pids()?;
    let processes = read::process_snapshot()?;
    // The consent store only reinforces the sessions; an unreadable store
    // leaves the WASAPI answer standing.
    let consent = read::consent_entries();
    Ok(windows_input_processes(&ProbeSnapshot {
        active_capture_pids,
        processes,
        consent,
    }))
}

#[cfg(target_os = "windows")]
mod read {
    use super::{ConsentEntry, ProcessEntry};
    use crate::meeting_detection::ProbeError;
    use windows::core::{Interface, HSTRING, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WIN32_ERROR};
    use windows::Win32::Media::Audio::{
        eCapture, AudioSessionStateActive, IAudioSessionControl2, IAudioSessionManager2,
        IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
        RRF_RT_REG_QWORD,
    };

    const MICROPHONE_CONSENT_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\microphone";
    const NON_PACKAGED_SUBKEY: &str = "NonPackaged";

    /// Same contract as `audio::system_windows::ComApartment`: MTA for the
    /// calling thread, balanced on drop. Declare it before any interface so
    /// the interfaces drop first.
    struct ComApartment;

    impl ComApartment {
        fn enter() -> Result<Self, ProbeError> {
            unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
                .ok()
                .map_err(hresult_error("initialize COM"))?;
            Ok(Self)
        }
    }

    impl Drop for ComApartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() };
        }
    }

    fn hresult_error(operation: &'static str) -> impl Fn(windows::core::Error) -> ProbeError {
        move |error| ProbeError::new(operation, error.code().0)
    }

    /// Pids of active capture sessions on every active capture endpoint. Any
    /// failure fails the whole probe: a missed endpoint would read as the
    /// meeting app releasing the microphone.
    pub(super) fn active_capture_pids() -> Result<Vec<u32>, ProbeError> {
        let _com = ComApartment::enter()?;
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) }
                .map_err(hresult_error("create audio device enumerator"))?;
        let devices = unsafe { enumerator.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) }
            .map_err(hresult_error("enumerate capture endpoints"))?;
        let device_count =
            unsafe { devices.GetCount() }.map_err(hresult_error("count capture endpoints"))?;
        let mut pids = Vec::new();
        for device_index in 0..device_count {
            let device = unsafe { devices.Item(device_index) }
                .map_err(hresult_error("read capture endpoint"))?;
            let manager: IAudioSessionManager2 = unsafe { device.Activate(CLSCTX_ALL, None) }
                .map_err(hresult_error("activate audio session manager"))?;
            let sessions = unsafe { manager.GetSessionEnumerator() }
                .map_err(hresult_error("enumerate capture sessions"))?;
            let session_count =
                unsafe { sessions.GetCount() }.map_err(hresult_error("count capture sessions"))?;
            for session_index in 0..session_count {
                let session = unsafe { sessions.GetSession(session_index) }
                    .map_err(hresult_error("read capture session"))?;
                let state =
                    unsafe { session.GetState() }.map_err(hresult_error("read session state"))?;
                if state != AudioSessionStateActive {
                    continue;
                }
                let session: IAudioSessionControl2 = session
                    .cast()
                    .map_err(hresult_error("read session process"))?;
                // Cross-process sessions report no single pid; the
                // consent-store pass covers allowed apps among them.
                if let Ok(pid) = unsafe { session.GetProcessId() } {
                    if pid != 0 {
                        pids.push(pid);
                    }
                }
            }
        }
        Ok(pids)
    }

    struct SnapshotHandle(HANDLE);

    impl Drop for SnapshotHandle {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub(super) fn process_snapshot() -> Result<Vec<ProcessEntry>, ProbeError> {
        let snapshot = SnapshotHandle(
            unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
                .map_err(hresult_error("snapshot processes"))?,
        );
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut processes = Vec::new();
        let mut next = unsafe { Process32FirstW(snapshot.0, &mut entry) };
        while next.is_ok() {
            let name_len = entry
                .szExeFile
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(entry.szExeFile.len());
            processes.push(ProcessEntry {
                pid: entry.th32ProcessID,
                parent_pid: entry.th32ParentProcessID,
                exe_name: String::from_utf16_lossy(&entry.szExeFile[..name_len]),
            });
            next = unsafe { Process32NextW(snapshot.0, &mut entry) };
        }
        Ok(processes)
    }

    struct RegistryKey(HKEY);

    impl RegistryKey {
        fn open(parent: HKEY, path: &str) -> Option<Self> {
            let mut key = HKEY::default();
            let status =
                unsafe { RegOpenKeyExW(parent, &HSTRING::from(path), None, KEY_READ, &mut key) };
            status.is_ok().then_some(Self(key))
        }

        fn subkey_names(&self) -> Vec<String> {
            let mut names = Vec::new();
            let mut buffer = [0_u16; 512];
            for index in 0.. {
                let mut len = buffer.len() as u32;
                let status: WIN32_ERROR = unsafe {
                    RegEnumKeyExW(
                        self.0,
                        index,
                        Some(PWSTR(buffer.as_mut_ptr())),
                        &mut len,
                        None,
                        None,
                        None,
                        None,
                    )
                };
                if status.is_err() {
                    break;
                }
                names.push(String::from_utf16_lossy(&buffer[..len as usize]));
            }
            names
        }

        fn qword(&self, subkey: &str, value: &str) -> u64 {
            let mut data = 0_u64;
            let mut size = std::mem::size_of::<u64>() as u32;
            let status = unsafe {
                RegGetValueW(
                    self.0,
                    &HSTRING::from(subkey),
                    &HSTRING::from(value),
                    RRF_RT_REG_QWORD,
                    None,
                    Some((&mut data as *mut u64).cast()),
                    Some(&mut size),
                )
            };
            if status.is_ok() {
                data
            } else {
                0
            }
        }
    }

    impl Drop for RegistryKey {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }

    pub(super) fn consent_entries() -> Vec<ConsentEntry> {
        let Some(microphone) = RegistryKey::open(HKEY_CURRENT_USER, MICROPHONE_CONSENT_KEY) else {
            return Vec::new();
        };
        let mut entries = Vec::new();
        for name in microphone.subkey_names() {
            if name.eq_ignore_ascii_case(NON_PACKAGED_SUBKEY) {
                continue;
            }
            entries.push(consent_entry(&microphone, name, true));
        }
        if let Some(non_packaged) = RegistryKey::open(microphone.0, NON_PACKAGED_SUBKEY) {
            for name in non_packaged.subkey_names() {
                entries.push(consent_entry(&non_packaged, name, false));
            }
        }
        entries
    }

    fn consent_entry(parent: &RegistryKey, key: String, packaged: bool) -> ConsentEntry {
        ConsentEntry {
            last_used_time_start: parent.qword(&key, "LastUsedTimeStart"),
            last_used_time_stop: parent.qword(&key, "LastUsedTimeStop"),
            key,
            packaged,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_detection::{
        active_allowed_external_processes, meeting_end_bundle_families,
    };
    use std::collections::BTreeSet;

    fn process(pid: u32, parent_pid: u32, exe_name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            exe_name: exe_name.to_string(),
        }
    }

    fn consent(key: &str, packaged: bool, stop: u64) -> ConsentEntry {
        ConsentEntry {
            key: key.to_string(),
            packaged,
            last_used_time_start: 133_000_000_000_000_000,
            last_used_time_stop: stop,
        }
    }

    fn found(snapshot: &ProbeSnapshot) -> Vec<(u32, String, String)> {
        windows_input_processes(snapshot)
            .into_iter()
            .map(|process| (process.pid, process.bundle_id, process.app_label))
            .collect()
    }

    fn row(pid: u32, bundle_id: &str, label: &str) -> (u32, String, String) {
        (pid, bundle_id.to_string(), label.to_string())
    }

    fn base_processes() -> Vec<ProcessEntry> {
        vec![
            process(4, 0, "System"),
            process(100, 4, "explorer.exe"),
            // Chrome: browser process plus its audio service utility child.
            process(200, 100, "chrome.exe"),
            process(210, 200, "chrome.exe"),
            process(211, 200, "chrome.exe"),
            // New Teams hosting WebView2 processes.
            process(300, 100, "ms-teams.exe"),
            process(310, 300, "msedgewebview2.exe"),
            process(320, 310, "msedgewebview2.exe"),
            // Clovy with its own WebView2 and the dictation helper.
            process(400, 100, "clovy.exe"),
            process(410, 400, "msedgewebview2.exe"),
            process(420, 410, "msedgewebview2.exe"),
            process(430, 400, "clovy-dictation-helper.exe"),
            // Firefox with a content child.
            process(500, 100, "firefox.exe"),
            process(510, 500, "firefox.exe"),
            process(600, 100, "obs64.exe"),
        ]
    }

    #[test]
    fn chromium_audio_service_session_resolves_to_the_browser_process() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: vec![210],
            processes: base_processes(),
            consent: Vec::new(),
        };

        assert_eq!(found(&snapshot), vec![row(200, "chrome.exe", "Chrome")]);
    }

    #[test]
    fn webview2_session_resolves_to_its_host_app() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: vec![320],
            processes: base_processes(),
            consent: Vec::new(),
        };

        assert_eq!(found(&snapshot), vec![row(300, "ms-teams.exe", "Teams")]);
    }

    #[test]
    fn sessions_of_one_app_collapse_to_one_process() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: vec![211, 210, 200, 0, 9_999],
            processes: base_processes(),
            consent: Vec::new(),
        };

        assert_eq!(found(&snapshot), vec![row(200, "chrome.exe", "Chrome")]);
    }

    #[test]
    fn clovy_and_its_helper_never_trigger_even_through_webview2() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: vec![420, 430],
            processes: base_processes(),
            consent: vec![consent("C:#Program Files#Clovy#clovy.exe", false, 0)],
        };
        let processes = windows_input_processes(&snapshot);
        let owned = BTreeSet::from([400, 430]);

        assert_eq!(
            processes
                .iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![400, 430]
        );
        assert!(active_allowed_external_processes(&processes, &owned).is_empty());
        assert_eq!(
            meeting_end_bundle_families(&processes, &owned),
            Some(BTreeSet::new())
        );
    }

    #[test]
    fn unlisted_capture_process_is_reported_but_not_allowed() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: vec![600, 210],
            processes: base_processes(),
            consent: Vec::new(),
        };
        let processes = windows_input_processes(&snapshot);

        assert_eq!(
            found(&snapshot),
            vec![
                row(200, "chrome.exe", "Chrome"),
                row(600, "obs64.exe", "obs64"),
            ]
        );
        assert_eq!(
            active_allowed_external_processes(&processes, &BTreeSet::new())
                .into_iter()
                .map(|process| process.pid)
                .collect::<Vec<_>>(),
            vec![200]
        );
        assert_eq!(
            meeting_end_bundle_families(&processes, &BTreeSet::new()),
            None
        );
    }

    #[test]
    fn consent_store_adds_a_running_allowed_browser_the_sessions_missed() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: Vec::new(),
            processes: base_processes(),
            consent: vec![consent(
                "C:#Program Files#Mozilla Firefox#firefox.exe",
                false,
                0,
            )],
        };

        assert_eq!(found(&snapshot), vec![row(500, "firefox.exe", "Firefox")]);
    }

    #[test]
    fn consent_store_adds_a_packaged_app_by_package_family_name() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: Vec::new(),
            processes: base_processes(),
            consent: vec![consent("MSTeams_8wekyb3d8bbwe", true, 0)],
        };

        assert_eq!(found(&snapshot), vec![row(300, "ms-teams.exe", "Teams")]);
    }

    #[test]
    fn consent_store_ignores_stale_stopped_and_unlisted_entries() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: Vec::new(),
            processes: base_processes(),
            consent: vec![
                // In use, but Zoom is not running: a stale entry.
                consent("C:#Users#me#AppData#Roaming#Zoom#bin#Zoom.exe", false, 0),
                // Firefox stopped using the microphone.
                consent(
                    "C:#Program Files#Mozilla Firefox#firefox.exe",
                    false,
                    133_000_000_000_000_100,
                ),
                // Running and in use, but not an allowed meeting app.
                consent("C:#Program Files#obs-studio#bin#64bit#obs64.exe", false, 0),
                ConsentEntry {
                    last_used_time_start: 0,
                    ..consent(
                        "C:#Program Files#Google#Chrome#Application#chrome.exe",
                        false,
                        0,
                    )
                },
            ],
        };

        assert!(found(&snapshot).is_empty());
    }

    #[test]
    fn consent_store_does_not_duplicate_an_app_the_sessions_found() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: vec![210],
            processes: base_processes(),
            consent: vec![consent(
                "C:#Program Files#Google#Chrome#Application#chrome.exe",
                false,
                0,
            )],
        };

        assert_eq!(found(&snapshot), vec![row(200, "chrome.exe", "Chrome")]);
    }

    #[test]
    fn parent_walk_stops_on_cycles() {
        let snapshot = ProbeSnapshot {
            active_capture_pids: vec![700],
            processes: vec![
                process(700, 701, "msedgewebview2.exe"),
                process(701, 700, "msedgewebview2.exe"),
            ],
            consent: Vec::new(),
        };

        let processes = windows_input_processes(&snapshot);
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].bundle_id, "msedgewebview2.exe");
    }
}
