//! Native macOS permission checks attributed to the signed private helper.
//! Automation itself lives exclusively in the supervised engine process.
use core_foundation::{base::TCFType, string::CFString};
use serde_json::{json, Value};
use std::ffi::c_void;
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}
pub fn request_startup_permission() {
    if crate::computer_use_cancel::current_run_cancelled() {
        return;
    }
    match std::env::var("CLOVY_COMPUTER_USE_PERMISSION_PROMPT").as_deref() {
        Ok("accessibility") => {
            use core_foundation::{boolean::CFBoolean, dictionary::CFDictionary};
            let options = CFDictionary::from_CFType_pairs(&[(
                CFString::new("AXTrustedCheckOptionPrompt").as_CFType(),
                CFBoolean::true_value().as_CFType(),
            )]);
            unsafe {
                AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast());
            }
        }
        Ok("screen-recording") => unsafe {
            CGRequestScreenCaptureAccess();
        },
        _ => {}
    }
}
pub fn permission_result() -> Value {
    let accessibility = unsafe { AXIsProcessTrusted() };
    let screen_recording = unsafe { CGPreflightScreenCaptureAccess() };
    json!({"content":[{"type":"text","text":format!("Accessibility: {}. Screen Recording: {}.",if accessibility{"granted"}else{"not granted"},if screen_recording{"granted"}else{"not granted"})}],"structuredContent":{"accessibility":accessibility,"screen_recording":screen_recording,"screen_recording_capturable":screen_recording,"source":{"attribution":"driver-app","pid":std::process::id(),"executable":std::env::current_exe().ok().and_then(|p|p.into_os_string().into_string().ok()).unwrap_or_default()}},"isError":false})
}

/// Terminate only a live group that still contains one of this run's exact
/// controller images and random namespace. A stale LaunchServices PID alone
/// never grants authority to signal a group that may have been reused.
pub fn terminate_owned_group(pid: u32, run_root: &std::path::Path) -> bool {
    use std::{
        mem::{size_of, MaybeUninit},
        path::{Path, PathBuf},
    };
    crate::computer_use_cancel::cooperative_cancel(run_root);
    let Ok(group) = i32::try_from(pid) else {
        return false;
    };
    if group <= 1 || !run_root.is_absolute() {
        return false;
    }
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    let mut helpers:Vec<PathBuf>=current.ancestors().filter(|p|p.extension().is_some_and(|e|e.eq_ignore_ascii_case("app"))).map(|outer|outer.join("Contents/Resources/native/bin/Clovy Computer Use Driver.app/Contents/MacOS/clovy-computer-use-driver")).collect();
    if cfg!(debug_assertions) {
        if let Some(repository) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
            helpers.push(repository.join(".tauri-helper/Clovy Computer Use Driver.app/Contents/MacOS/clovy-computer-use-driver"));
        }
    }
    let mut images = helpers.clone();
    images.extend(helpers.iter().filter_map(|helper| {
        helper
            .parent()
            .map(|directory| directory.join("agent-desktop"))
    }));
    // Authentication runs the system signature verifier inside the same owned
    // group. No user application is launched directly inside this group.
    images.push(PathBuf::from("/usr/bin/codesign"));
    let mut members = vec![0i32; 4096];
    let count = unsafe {
        libc::proc_listpgrppids(
            group,
            members.as_mut_ptr().cast(),
            (members.len() * size_of::<i32>()) as i32,
        )
    };
    if count <= 0 || count as usize > members.len() {
        return false;
    }
    // libproc returns a PID count here, unlike proc_listpids which returns bytes.
    members.truncate(count as usize);
    fn info(pid: i32) -> Option<libc::proc_bsdinfo> {
        let mut info = MaybeUninit::<libc::proc_bsdinfo>::zeroed();
        let size = size_of::<libc::proc_bsdinfo>() as i32;
        (unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                info.as_mut_ptr().cast(),
                size,
            )
        } == size)
            .then(|| unsafe { info.assume_init() })
    }
    fn image(pid: i32) -> Option<PathBuf> {
        let mut bytes = vec![0u8; 4096];
        let count =
            unsafe { libc::proc_pidpath(pid, bytes.as_mut_ptr().cast(), bytes.len() as u32) };
        if count <= 0 {
            return None;
        }
        bytes.truncate(count as usize);
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        Some(PathBuf::from(String::from_utf8(bytes).ok()?))
    }
    fn same_file(a: &Path, b: &Path) -> bool {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
            _ => false,
        }
    }
    for member in members.into_iter().filter(|pid| *pid > 0) {
        let Some(before) = info(member) else { continue };
        if before.pbi_pgid != pid || before.pbi_uid != unsafe { libc::geteuid() } {
            continue;
        }
        let Some(path) = image(member) else { continue };
        if !images.iter().any(|expected| same_file(&path, expected)) {
            continue;
        }
        if !process_has_namespace(member, run_root) {
            continue;
        }
        let Some(after) = info(member) else { continue };
        if after.pbi_pgid == pid
            && after.pbi_pid == before.pbi_pid
            && after.pbi_start_tvsec == before.pbi_start_tvsec
            && after.pbi_start_tvusec == before.pbi_start_tvusec
        {
            return unsafe { libc::kill(-group, libc::SIGKILL) } == 0;
        }
    }
    false
}
fn process_has_namespace(pid: i32, run_root: &std::path::Path) -> bool {
    // KERN_PROCARGS2 is read only for an already image-verified member of the
    // candidate group. Nothing from the process environment is logged/stored.
    let mut argument_limit = 0i32;
    let mut length = std::mem::size_of::<i32>();
    let mut limit_mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    if unsafe {
        libc::sysctl(
            limit_mib.as_mut_ptr(),
            2,
            (&mut argument_limit as *mut i32).cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || !(4..=4 * 1024 * 1024).contains(&argument_limit)
    {
        return false;
    }
    let mut bytes = vec![0u8; argument_limit as usize];
    let mut length = bytes.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            bytes.as_mut_ptr().cast(),
            &mut length,
            std::ptr::null_mut(),
            0,
        )
    } != 0
        || length < 4
    {
        bytes.fill(0);
        return false;
    }
    bytes.truncate(length);
    let count = i32::from_ne_bytes(bytes[..4].try_into().expect("argument count bytes"));
    let mut cursor = 4usize;
    fn skip_string(bytes: &[u8], cursor: &mut usize) -> bool {
        if let Some(end) = bytes
            .get(*cursor..)
            .and_then(|tail| tail.iter().position(|c| *c == 0))
        {
            *cursor += end + 1;
            true
        } else {
            false
        }
    }
    if count < 0 || count as usize > bytes.len() / 2 || !skip_string(&bytes, &mut cursor) {
        bytes.fill(0);
        return false;
    }
    while bytes.get(cursor) == Some(&0) {
        cursor += 1;
    }
    for _ in 0..count {
        if !skip_string(&bytes, &mut cursor) {
            bytes.fill(0);
            return false;
        }
    }
    let root = run_root.as_os_str().as_encoded_bytes();
    let owned = bytes[cursor.min(bytes.len())..]
        .split(|c| *c == 0)
        .any(|entry| {
            entry
                .strip_prefix(b"CLOVY_COMPUTER_USE_RUN_DIR=")
                .is_some_and(|value| value == root)
                || entry
                    .strip_prefix(b"AGENT_DESKTOP_HOME=")
                    .is_some_and(|value| {
                        value.strip_prefix(root).is_some_and(|tail| {
                            tail.starts_with(b"/engine-") && !tail[1..].contains(&b'/')
                        })
                    })
        });
    bytes.fill(0);
    owned
}

pub fn process_creation(pid: u32) -> Option<u64> {
    let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::zeroed();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
    if unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    } != size
    {
        return None;
    }
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid as u32 || info.pbi_start_tvusec >= 1_000_000 {
        return None;
    }
    info.pbi_start_tvsec
        .checked_mul(1_000_000)?
        .checked_add(info.pbi_start_tvusec)
        .filter(|birth| *birth > 0)
}

/// A QA target must be genuine same-Team signed fixture code. Renaming or
/// re-signing the Clovy app with a fixture-looking bundle name is insufficient.
pub fn verify_fixture(path: &std::path::Path, expected_id: &str) -> bool {
    use core_foundation::{
        base::{CFGetTypeID, CFRelease},
        string::{CFStringGetLength, CFStringGetTypeID},
        url::CFURL,
    };
    use security_framework::os::macos::code_signing::{
        Flags, SecCode, SecRequirement, SecStaticCode,
    };
    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        fn SecCodeCopySigningInformation(
            code: *const c_void,
            flags: u32,
            out: *mut *const c_void,
        ) -> i32;
        static kSecCodeInfoTeamIdentifier: *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFDictionaryGetTypeID() -> usize;
        fn CFDictionaryGetValue(dictionary: *const c_void, key: *const c_void) -> *const c_void;
    }
    struct Owned(*const c_void);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    CFRelease(self.0);
                }
            }
        }
    }
    if !matches!(
        expected_id,
        "dev.guilhermexp.clovy.computer-use-self-test.target"
            | "dev.guilhermexp.clovy.computer-use-self-test.observer"
    ) || !path.is_absolute()
    {
        return false;
    }
    let Ok(current) = SecCode::for_self(Flags::NONE) else {
        return false;
    };
    let mut dictionary = std::ptr::null();
    // kSecCSSigningInformation = 1 << 1, from Apple's SecCode.h.
    if unsafe {
        SecCodeCopySigningInformation(current.as_concrete_TypeRef().cast(), 2, &mut dictionary)
    } != 0
        || dictionary.is_null()
    {
        return false;
    }
    let dictionary = Owned(dictionary);
    if unsafe { CFGetTypeID(dictionary.0) } != unsafe { CFDictionaryGetTypeID() } {
        return false;
    }
    let team = unsafe { CFDictionaryGetValue(dictionary.0, kSecCodeInfoTeamIdentifier) };
    if team.is_null() || unsafe { CFGetTypeID(team) } != unsafe { CFStringGetTypeID() } {
        return false;
    }
    let length = unsafe { CFStringGetLength(team.cast()) };
    if !(1..=64).contains(&length) {
        return false;
    }
    let team = unsafe { CFString::wrap_under_get_rule(team.cast()) }.to_string();
    if !team.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return false;
    }
    let trust = format!("anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
    let Ok(current_requirement) = trust.parse::<SecRequirement>() else {
        return false;
    };
    let flags = Flags::STRICT_VALIDATE | Flags::NO_NETWORK_ACCESS;
    if current.check_validity(flags, &current_requirement).is_err() {
        return false;
    }
    let Some(url) = CFURL::from_path(path, path.is_dir()) else {
        return false;
    };
    let Ok(fixture) = SecStaticCode::from_path(&url, Flags::NONE) else {
        return false;
    };
    let Ok(requirement) =
        format!("{trust} and identifier \"{expected_id}\"").parse::<SecRequirement>()
    else {
        return false;
    };
    fixture
        .check_validity(flags | Flags::CHECK_ALL_ARCHITECTURES, &requirement)
        .is_ok()
}

/// The kernel's mapped-image CDHash, unlike a pathname hash, remains bound to
/// the executable that actually ran. Preparation embeds the signed slice's
/// CDHash for this Cargo architecture in the helper.
fn parse_cdhash(encoded: &str) -> Option<[u8; 20]> {
    if encoded.len() != 40 {
        return None;
    }
    let mut bytes = [0u8; 20];
    for (index, chunk) in encoded.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(chunk).ok()?;
        bytes[index] = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(bytes)
}
pub fn compiled_engine_cdhash() -> Option<[u8; 20]> {
    parse_cdhash(option_env!("CLOVY_AGENT_DESKTOP_CDHASH")?)
}
pub(crate) fn verify_mapped_image(pid: u32, expected: &str) -> anyhow::Result<()> {
    #[link(name = "System")]
    unsafe extern "C" {
        fn csops(
            pid: libc::pid_t,
            operations: u32,
            address: *mut c_void,
            size: usize,
        ) -> libc::c_int;
    }
    let pid = libc::pid_t::try_from(pid)?;
    let expected = parse_cdhash(expected)
        .ok_or_else(|| anyhow::anyhow!("The signed owner has no valid mapped-image identity"))?;
    let mut actual = [0u8; 20];
    // CS_OPS_CDHASH (5) reads the live kernel code-signing identity, not a
    // reopened file. No private frame can reach an older or swapped image.
    if pid <= 0
        || unsafe { csops(pid, 5, actual.as_mut_ptr().cast(), actual.len()) } != 0
        || actual != expected
    {
        anyhow::bail!("The running Computer use image does not match the signed owner");
    }
    Ok(())
}
pub fn verify_engine_process(pid: u32) -> bool {
    option_env!("CLOVY_AGENT_DESKTOP_CDHASH")
        .is_some_and(|expected| verify_mapped_image(pid, expected).is_ok())
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    #[test]
    fn process_creation_is_stable_for_the_live_process() {
        let pid = std::process::id();
        let birth = process_creation(pid).unwrap();
        assert!(birth > 0);
        assert_eq!(process_creation(pid), Some(birth));
    }
    #[test]
    fn unrelated_group_is_not_authorized_by_a_copied_namespace() {
        use std::os::unix::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("60")
            .env("CLOVY_COMPUTER_USE_RUN_DIR", root.path())
            .process_group(0)
            .spawn()
            .unwrap();
        assert!(!terminate_owned_group(child.id(), root.path()));
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
