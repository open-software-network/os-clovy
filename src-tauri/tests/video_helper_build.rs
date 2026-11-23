//! Execute the real build-script orchestration with isolated build/sign boundaries.
//! No Swift compiler, codesign, app launch or signing identity is used by this test.

#[test]
fn video_helper_rebuild_reapplies_configured_signing() {
    let build = include_str!("../build.rs");
    let start = build.find("fn build_video_capture_helper() {").unwrap();
    let end = start
        + build[start..]
            .find("\nfn build_system_audio_helper()")
            .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("video_helper_policy.rs");
    let executable = directory.path().join(format!(
        "video_helper_policy{}",
        std::env::consts::EXE_SUFFIX
    ));
    std::fs::write(
        &source,
        format!("{}\n{}", &build[start..end], SIGNING_BOUNDARIES),
    )
    .unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compiled = std::process::Command::new(rustc)
        .args(["--edition=2021", "--test"])
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let result = std::process::Command::new(&executable)
        .arg("--test-threads=1")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

// Keep the cache, compiler and signer as boundaries; execute the actual function
// above, including its plist I/O and invalidation decisions. Accepted signature
// deliberately models a linker signature without the configured entitlements.
const SIGNING_BOUNDARIES: &str = r##"
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
static CURRENT: AtomicBool = AtomicBool::new(false);
static ACCEPTED_SIGNATURE: AtomicBool = AtomicBool::new(true);
static RELEASE: AtomicBool = AtomicBool::new(false);
static BUILDS: AtomicUsize = AtomicUsize::new(0);
static SIGNS: AtomicUsize = AtomicUsize::new(0);
enum HelperSigning { Release(()), Unselected }
impl HelperSigning {
    fn is_satisfied_by(&self, _: &Path) -> bool { ACCEPTED_SIGNATURE.load(Ordering::SeqCst) }
}
fn helper_signing() -> HelperSigning {
    if RELEASE.load(Ordering::SeqCst) { HelperSigning::Release(()) } else { HelperSigning::Unselected }
}
fn helper_bundle_identifier(_: &Path, _: &str) -> String { "dev.clovy.unit.signingfixture".into() }
fn swift_helper_executable_current(_: &Path, _: &Path) -> bool { CURRENT.load(Ordering::SeqCst) }
fn build_universal_swift_executable(_: &str, _: &Path, _: &Path, executable: &Path, _: &str, _: &[&str]) -> bool {
    BUILDS.fetch_add(1, Ordering::SeqCst);
    std::fs::write(executable, b"rebuilt binary with linker signature only").unwrap();
    let marker = executable.parent().unwrap().parent().unwrap().join("expected-entitlements");
    let _ = std::fs::remove_file(marker);
    true
}
fn sign_helper_app(_: &Path, app: &Path, _: &HelperSigning) {
    SIGNS.fetch_add(1, Ordering::SeqCst);
    std::fs::write(app.join("Contents/expected-entitlements"), b"configured signing entitlements applied").unwrap();
}
fn reset_counts() { BUILDS.store(0, Ordering::SeqCst); SIGNS.store(0, Ordering::SeqCst); }
#[test]
fn rebuilt_helper_must_reapply_configured_signing_with_unchanged_plist_and_accepted_signature() {
    let root = std::env::temp_dir().join(format!("clovy-signing-harness-{}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup { fn drop(&mut self) { let _=std::fs::remove_dir_all(&self.0); } }
    let _cleanup = Cleanup(root.clone());
    let manifest = root.join("src-tauri");
    std::fs::create_dir(&manifest).unwrap();
    std::env::set_var("CARGO_CFG_TARGET_OS", "macos");
    std::env::set_var("CARGO_MANIFEST_DIR", &manifest);
    let contents = root.join(".tauri-helper/Clovy Video Capture.app/Contents");
    build_video_capture_helper();
    assert_eq!((BUILDS.load(Ordering::SeqCst), SIGNS.load(Ordering::SeqCst)), (1,1));
    let plist = std::fs::read(contents.join("Info.plist")).unwrap();
    reset_counts();
    // Rebuild removes configured entitlements but preserves the accepted linker signature.
    build_video_capture_helper();
    assert_eq!(std::fs::read(contents.join("Info.plist")).unwrap(), plist);
    assert_eq!(BUILDS.load(Ordering::SeqCst), 1);
    assert_eq!(SIGNS.load(Ordering::SeqCst), 1, "rebuild must pass through configured signer");
    assert!(contents.join("expected-entitlements").exists());

    reset_counts(); CURRENT.store(true, Ordering::SeqCst);
    build_video_capture_helper();
    assert_eq!((BUILDS.load(Ordering::SeqCst), SIGNS.load(Ordering::SeqCst)), (0,0), "unchanged valid cached helper stays untouched");
    reset_counts(); std::fs::write(contents.join("Info.plist"), b"changed").unwrap();
    build_video_capture_helper();
    assert_eq!((BUILDS.load(Ordering::SeqCst), SIGNS.load(Ordering::SeqCst)), (0,1));
    reset_counts(); ACCEPTED_SIGNATURE.store(false, Ordering::SeqCst);
    build_video_capture_helper();
    assert_eq!((BUILDS.load(Ordering::SeqCst), SIGNS.load(Ordering::SeqCst)), (0,1));
    reset_counts(); ACCEPTED_SIGNATURE.store(true, Ordering::SeqCst); RELEASE.store(true, Ordering::SeqCst);
    build_video_capture_helper();
    assert_eq!((BUILDS.load(Ordering::SeqCst), SIGNS.load(Ordering::SeqCst)), (0,1));
}
"##;

#[test]
fn helper_install_stages_beside_destination_and_preserves_old_binary_on_failure() {
    let build = include_str!("../build.rs");
    let start = build.find("fn install_helper_executable(").unwrap();
    let end = start + build[start..].find("\n/// Bundle identifier").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("install.rs");
    let binary = temp
        .path()
        .join(format!("install{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(
        &source,
        format!("{}\n{}", &build[start..end], INSTALL_HARNESS),
    )
    .unwrap();
    let compiled =
        std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2021"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let result = std::process::Command::new(binary)
        .arg(temp.path())
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

const INSTALL_HARNESS: &str = r##"
fn main() {
    let root = std::path::PathBuf::from(std::env::args_os().nth(1).unwrap());
    let cache = root.join("cache");
    let bundle = root.join("bundle");
    std::fs::create_dir(&cache).unwrap();
    std::fs::create_dir(&bundle).unwrap();
    let input = cache.join("universal");
    let output = bundle.join("helper");
    std::fs::write(&input, b"new helper").unwrap();
    std::fs::write(&output, b"old helper").unwrap();
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&input, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    install_helper_executable(&input, &output).unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), b"new helper");
    assert!(input.exists(), "installation copies from the other filesystem before rename");
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&output).unwrap().permissions().mode() & 0o777, 0o755);
    }
    assert!(install_helper_executable(&cache.join("missing"), &output).is_err());
    assert_eq!(std::fs::read(&output).unwrap(), b"new helper");
    assert_eq!(std::fs::read_dir(&bundle).unwrap().count(), 1, "failed staging must be removed");
    let blocked = bundle.join("blocked");
    std::fs::create_dir(&blocked).unwrap();
    assert!(install_helper_executable(&input, &blocked).is_err());
    assert!(blocked.is_dir());
    assert_eq!(std::fs::read_dir(&bundle).unwrap().count(), 2);
}
"##;
