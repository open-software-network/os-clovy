const SYSTEM_AUDIO_MIN_MACOS_VERSION_FILE: &str = "system-audio-min-macos-version.txt";
const DICTATION_HELPER_MIN_MACOS_VERSION: &str = "14.0";

fn main() {
    println!("cargo:rerun-if-env-changed=CARGO_TARGET_DIR");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=Entitlements.plist");
    println!("cargo:rerun-if-changed=icons/icon.icns");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    println!("cargo:rerun-if-changed=icons/128x128@2x.png");
    println!("cargo:rerun-if-env-changed=OS_ACCOUNTS_URL");
    println!("cargo:rerun-if-env-changed=OS_ACCOUNTS_API_URL");
    println!("cargo:rerun-if-env-changed=OS_ACCOUNTS_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=GOOGLE_OAUTH_CLIENT_ID");
    println!("cargo:rerun-if-env-changed=GOOGLE_OAUTH_CLIENT_SECRET");
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("macos") {
        println!("cargo:rustc-link-lib=framework=AVFoundation");
        link_swift_runtime();
    }
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("windows") {
        // Library test executables do not inherit Tauri's app resources.
        // rfd imports TaskDialogIndirect, which needs Common Controls v6.
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
        // Tauri already supplies RT_MANIFEST #1 to binary targets. Keep that
        // resource instead of asking LINK to embed a second manifest there;
        // library/integration test executables still need the generated one.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
    }
    build_system_audio_helper();
    build_video_capture_helper();
    build_dictation_helper();
    prepare_computer_use_driver();
    ensure_local_transcriber_resource();
    build_windows_dictation_helper();
    ensure_agent_browser_resources();
    ensure_mcp_relay_placeholder();
    // Relay bootstrap builds need an existing resource directory. The wrapper
    // stages a real signed Cargo artifact before bundling; no placeholder exe.
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() == Some("windows") {
        let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
        std::fs::create_dir_all(root.parent().unwrap().join(".tauri-helper/mcp-windows"))
            .expect("MCP relay resource directory should be created");
    }
    export_app_identifier();
    tauri_build::build();
}

/// Compiles the app identifier into `CLOVY_APP_IDENTIFIER` for code without an
/// AppHandle (the MCP relay). Mirrors Tauri's config merge for this
/// key: the `--config` overlays the Tauri CLI passes in `TAURI_CONFIG` (where
/// `CLOVY_DEV_APP_IDENTIFIER` lands), then the platform file, then
/// `tauri.conf.json`.
fn app_identifier_for_build() -> String {
    println!("cargo:rerun-if-env-changed=TAURI_CONFIG");
    println!("cargo:rerun-if-env-changed=CLOVY_DEV_APP_IDENTIFIER");
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"),
    );
    let identifier_in = |json: &str| {
        serde_json::from_str::<serde_json::Value>(json)
            .ok()?
            .get("identifier")?
            .as_str()
            .map(str::to_string)
    };
    let platform_config = match std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() {
        Some("macos") => Some("tauri.macos.conf.json"),
        Some("windows") => Some("tauri.windows.conf.json"),
        Some("linux") => Some("tauri.linux.conf.json"),
        _ => None,
    };
    if let Some(name) = platform_config {
        println!("cargo:rerun-if-changed={name}");
    }
    let identifier = std::env::var("TAURI_CONFIG")
        .ok()
        .and_then(|overlay| identifier_in(&overlay))
        .or_else(|| {
            (std::env::var("PROFILE").ok().as_deref() == Some("debug"))
                .then(|| std::env::var("CLOVY_DEV_APP_IDENTIFIER").ok())
                .flatten()
                .filter(|value| !value.is_empty())
        })
        .or_else(|| {
            platform_config
                .and_then(|name| std::fs::read_to_string(manifest_dir.join(name)).ok())
                .and_then(|json| identifier_in(&json))
        })
        .or_else(|| {
            std::fs::read_to_string(manifest_dir.join("tauri.conf.json"))
                .ok()
                .and_then(|json| identifier_in(&json))
        })
        .expect("tauri.conf.json should declare an identifier");
    assert!(
        identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-'),
        "invalid compiled app identifier"
    );
    identifier
}

fn export_app_identifier() {
    let identifier = app_identifier_for_build();
    println!("cargo:rustc-env=CLOVY_APP_IDENTIFIER={identifier}");
}

/// Swift-backed dependencies emit their runtime search paths from dependency
/// build scripts, but those linker arguments do not reach this package's final
/// binaries. Resolve the active toolchain through `xcrun` so both full Xcode
/// and Command Line Tools layouts work without runner-specific paths.
fn link_swift_runtime() {
    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");
    println!("cargo:rerun-if-env-changed=TOOLCHAINS");
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");

    let output = std::process::Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output()
        .unwrap_or_else(|error| {
            panic!("failed to run `xcrun --find swiftc`; Swift runtime rpath is required: {error}")
        });
    assert!(
        output.status.success(),
        "`xcrun --find swiftc` failed; Swift runtime rpath is required: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );

    let swiftc = std::path::PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    let toolchain_usr = swiftc
        .parent()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| panic!("swiftc path has no toolchain root: {}", swiftc.display()));
    let mut linked_runtime = false;
    for relative in ["lib/swift-5.5/macosx", "lib/swift/macosx"] {
        let runtime = toolchain_usr.join(relative);
        if runtime.is_dir() {
            linked_runtime = true;
            println!("cargo:rustc-link-search=native={}", runtime.display());
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", runtime.display());
        }
    }
    assert!(
        linked_runtime,
        "active Swift toolchain has no supported macOS runtime directory: {}",
        toolchain_usr.display()
    );
}

/// Plain debug Cargo checks need the resource directory, but must never fetch
/// or build a browser. Release builds fail closed unless preparation verified
/// the pinned engine, Chrome, patches, provenance and complete file inventory.
fn ensure_agent_browser_resources() {
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"),
    );
    let repo = manifest_dir
        .parent()
        .expect("repository parent should exist");
    let resources = repo.join("resources/agent-browser");
    for relative in ["scripts/agent-browser", "scripts/prepare-agent-browser.mjs"] {
        println!("cargo:rerun-if-changed={}", repo.join(relative).display());
    }
    println!("cargo:rerun-if-changed={}", resources.display());
    if std::env::var("PROFILE").ok().as_deref() == Some("release") {
        let target = std::env::var("TARGET").expect("Cargo target should be set");
        let status = std::process::Command::new("node")
            .arg(repo.join("scripts/prepare-agent-browser.mjs"))
            .args(["--check", "--release", "--target", &target])
            .current_dir(repo)
            .status()
            .expect("node is required to verify prepared private browser resources");
        assert!(
            status.success(),
            "Pinned browser resources missing or invalid; run pnpm browser:prepare -- --release"
        );
    } else {
        std::fs::create_dir_all(resources)
            .expect("debug browser resource directory should be created");
    }
}

/// Keep the `Clovy Local Transcriber.app` resource source present for
/// ordinary `cargo test` and rust-analyzer runs. `scripts/prepare-local-transcriber.mjs`
/// (run by the dev and packaging entry points) builds the real helper into the
/// same bundle; without it the app reports `local_transcriber_failed`.
fn ensure_local_transcriber_resource() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("macos") {
        return;
    }
    let app_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"),
    )
    .parent()
    .expect("src-tauri should have a repository parent")
    .join(".tauri-helper")
    .join("Clovy Local Transcriber.app");
    let contents = app_dir.join("Contents");
    // Watch `Contents`, which exists from here on: a missing watched path makes
    // Cargo rerun this script on every build. Deleting the bundle removes it
    // and brings the placeholder back on the next build.
    if !contents
        .join("MacOS")
        .join("clovy-local-transcriber")
        .exists()
    {
        let placeholder = contents.join("Resources").join("PLACEHOLDER.md");
        // Write it once: Cargo scans the watched directory recursively, so
        // rewriting it on every run would rerun this script on every build.
        if !placeholder.exists() {
            if let Some(parent) = placeholder.parent() {
                std::fs::create_dir_all(parent)
                    .expect("local transcriber placeholder directory should be created");
            }
            std::fs::write(
                placeholder,
                "No local transcriber in this build. Run pnpm local-transcriber:prepare on macOS.\n",
            )
            .expect("local transcriber placeholder should be written");
        }
    }
    println!("cargo:rerun-if-changed={}", contents.display());
}

/// Packaging prepares the real engine and helper before the desktop build.
/// Helper bootstrap creates only the resource directory required by Tauri's
/// compile-time resource glob; it never fabricates an executable or manifest.
fn prepare_computer_use_driver() {
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if os != "macos" && os != "windows" {
        return;
    }
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let root = manifest_dir.parent().expect("repository root");
    let pin: serde_json::Value = serde_json::from_slice(
        &std::fs::read(manifest_dir.join("agent-desktop-pin.json")).expect("helper identity pin"),
    )
    .expect("helper identity pin JSON");
    let helper_identifier = helper_bundle_identifier(
        &manifest_dir,
        pin["bundleIdentifier"]
            .as_str()
            .expect("helper bundle identifier"),
    );
    println!("cargo:rerun-if-env-changed=CLOVY_COMPUTER_USE_HELPER_IDENTIFIER");
    if let Ok(provided) = std::env::var("CLOVY_COMPUTER_USE_HELPER_IDENTIFIER") {
        assert_eq!(
            provided, helper_identifier,
            "prepared and compiled helper identities differ"
        );
    }
    println!("cargo:rustc-env=CLOVY_COMPUTER_USE_HELPER_IDENTIFIER={helper_identifier}");
    let resource = if os == "macos" {
        root.join(".tauri-helper/Clovy Computer Use Driver.app")
    } else {
        root.join(".tauri-helper/computer-use")
    };
    println!("cargo:rerun-if-env-changed=CLOVY_PREPARING_COMPUTER_USE");
    println!("cargo:rerun-if-env-changed=CLOVY_PREPARED_AGENT_DESKTOP_SHA256");
    println!("cargo:rerun-if-env-changed=CLOVY_PREPARED_AGENT_DESKTOP_TARGET");
    if std::env::var("CLOVY_PREPARING_COMPUTER_USE")
        .ok()
        .as_deref()
        == Some("1")
    {
        std::fs::create_dir_all(&resource).expect("helper bootstrap resource directory");
        let prepared_target = std::env::var("CLOVY_PREPARED_AGENT_DESKTOP_TARGET")
            .unwrap_or_else(|_| std::env::var("TARGET").expect("Cargo target"));
        let prepared = root
            .join(".tauri-helper/agent-desktop")
            .join(prepared_target);
        let digest = verify_computer_use_engine(&manifest_dir, &prepared, &prepared, &os);
        if let Ok(expected) = std::env::var("CLOVY_PREPARED_AGENT_DESKTOP_SHA256") {
            assert_eq!(
                digest, expected,
                "prepared engine generation changed before helper compilation"
            );
        }
        println!("cargo:rustc-env=CLOVY_AGENT_DESKTOP_EXECUTABLE_SHA256={digest}");
        return;
    }
    let (binary_dir, resource_dir) = if os == "macos" {
        (
            resource.join("Contents/MacOS"),
            resource.join("Contents/Resources"),
        )
    } else {
        (resource.clone(), resource.clone())
    };
    let executable = binary_dir.join(if os == "macos" {
        "clovy-computer-use-driver"
    } else {
        "clovy-computer-use-driver.exe"
    });
    let stamp = resource_dir.join("clovy-computer-use-pin.json");
    let engine_manifest = resource_dir.join("agent-desktop-manifest.json");
    for path in [&executable, &stamp, &engine_manifest] {
        println!("cargo:rerun-if-changed={}", path.display());
        assert!(path.is_file(), "Computer use artifact missing: {}. Run pnpm computer-use:prepare before building Clovy", path.display());
    }
    verify_computer_use_driver_source(
        &manifest_dir,
        &manifest_dir.join("agent-desktop-pin.json"),
        &stamp,
        &executable,
    );
    let engine_digest = verify_computer_use_engine(&manifest_dir, &binary_dir, &resource_dir, &os);
    let helper_stamp: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&stamp).expect("helper stamp"))
            .expect("helper stamp JSON");
    assert_eq!(
        helper_stamp
            .pointer("/clovyBuild/engineSha256")
            .and_then(serde_json::Value::as_str),
        Some(engine_digest.as_str()),
        "helper was compiled for another engine generation; prepare it again"
    );
    if os == "windows" {
        println!("cargo:rustc-env=CLOVY_AGENT_DESKTOP_EXECUTABLE_SHA256={engine_digest}");
    }
    if os == "macos" {
        sign_computer_use_driver(&resource);
    }
    if os == "macos" {
        let architecture = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("aarch64") => "arm64",
            Ok("x86_64") => "x86_64",
            _ => panic!("unsupported macOS helper architecture"),
        };
        let output = std::process::Command::new("/usr/bin/codesign")
            .args(["-d", "--verbose=4", "--arch", architecture])
            .arg(&resource)
            .output()
            .expect("read signed helper CodeDirectory identity");
        assert!(
            output.status.success(),
            "could not read signed helper CDHash"
        );
        let details = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let cdhash = details
            .lines()
            .find_map(|line| line.strip_prefix("CDHash="))
            .expect("signed helper CDHash");
        assert!(
            cdhash.len() == 40 && cdhash.bytes().all(|value| value.is_ascii_hexdigit()),
            "invalid signed helper CDHash"
        );
        println!("cargo:rustc-env=CLOVY_COMPUTER_USE_HELPER_CDHASH={cdhash}");
    }
    // Bind the signed helper bytes, after any macOS identity reconciliation.
    // The bootstrap cannot bind itself; the ordinary desktop build can.
    use sha2::{Digest, Sha256};
    let helper_digest = format!(
        "{:x}",
        Sha256::digest(std::fs::read(&executable).expect("signed helper bytes"))
    );
    println!("cargo:rustc-env=CLOVY_COMPUTER_USE_HELPER_SHA256={helper_digest}");
}

fn verify_computer_use_engine(
    manifest_dir: &std::path::Path,
    binary_dir: &std::path::Path,
    resource_dir: &std::path::Path,
    os: &str,
) -> String {
    use sha2::{Digest, Sha256};
    let pin: serde_json::Value = serde_json::from_slice(
        &std::fs::read(manifest_dir.join("agent-desktop-pin.json")).expect("engine pin"),
    )
    .expect("engine pin JSON");
    println!(
        "cargo:rerun-if-changed={}",
        resource_dir.join("agent-desktop-manifest.json").display()
    );
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(resource_dir.join("agent-desktop-manifest.json")).expect("engine manifest"),
    )
    .expect("engine manifest JSON");
    for key in [
        "source",
        "sourceCommit",
        "version",
        "protocolVersion",
        "cargoLockSha256",
        "license",
        "licenseSha256",
        "patchSha256",
        "ownedRuntimeSourceSha256",
        "sourceReviewSha256",
    ] {
        assert_eq!(
            manifest.get(key),
            pin.get(key),
            "engine manifest {key} differs from reviewed pin"
        );
        assert!(
            manifest
                .get(key)
                .and_then(serde_json::Value::as_str)
                .is_some_and(|value| !value.is_empty()),
            "engine manifest {key} missing or empty"
        );
    }
    let executable = if os == "macos" {
        "agent-desktop"
    } else {
        "agent-desktop.exe"
    };
    assert_eq!(
        manifest["executable"].as_str(),
        Some(executable),
        "engine path must be adjacent to the helper"
    );
    let binary = binary_dir.join(executable);
    println!("cargo:rerun-if-changed={}", binary.display());
    let digest = format!(
        "{:x}",
        Sha256::digest(std::fs::read(&binary).expect("bundled real engine"))
    );
    assert_eq!(
        manifest["executableSha256"].as_str(),
        Some(digest.as_str()),
        "bundled engine digest mismatch"
    );
    for (name, field) in [
        (
            if os == "macos" && resource_dir.ends_with("Resources") {
                "agent-desktop-LICENSE"
            } else {
                "LICENSE"
            },
            "licenseSha256",
        ),
        ("source-review.md", "sourceReviewSha256"),
    ] {
        let path = resource_dir.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        let value = format!(
            "{:x}",
            Sha256::digest(std::fs::read(path).expect("engine attribution bytes"))
        );
        assert_eq!(
            pin[field].as_str(),
            Some(value.as_str()),
            "engine {field} attribution mismatch"
        );
    }
    if os == "macos" {
        let architecture = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
            Ok("aarch64") => "arm64",
            Ok("x86_64") => "x86_64",
            _ => panic!("unsupported macOS engine architecture"),
        };
        let cdhash = manifest["codeDirectoryHashes"][architecture]
            .as_str()
            .expect("signed engine per-architecture CDHash");
        assert!(
            cdhash.len() == 40 && cdhash.bytes().all(|value| value.is_ascii_hexdigit()),
            "invalid signed engine CDHash"
        );
        println!("cargo:rustc-env=CLOVY_AGENT_DESKTOP_CDHASH={cdhash}");
    }
    let expected_target = std::env::var("TARGET").expect("Cargo target");
    let declared = manifest["target"].as_str().expect("engine target");
    assert!(
        declared == expected_target
            || (declared == "universal-apple-darwin" && expected_target.ends_with("-apple-darwin")),
        "engine target mismatch"
    );
    let lock_digest = format!(
        "{:x}",
        Sha256::digest(
            std::fs::read(manifest_dir.join("agent-desktop/Cargo.lock"))
                .expect("pinned engine lock")
        )
    );
    assert_eq!(
        pin["cargoLockSha256"].as_str(),
        Some(lock_digest.as_str()),
        "engine lock mismatch"
    );
    digest
}

fn verify_computer_use_driver_source(
    manifest_dir: &std::path::Path,
    pin_path: &std::path::Path,
    stamp_path: &std::path::Path,
    executable: &std::path::Path,
) {
    use sha2::{Digest, Sha256};

    let pin: serde_json::Value = serde_json::from_slice(
        &std::fs::read(pin_path).expect("computer use driver pin should be readable"),
    )
    .expect("computer use driver pin should be valid JSON");
    let expected_version = pin
        .get("version")
        .and_then(serde_json::Value::as_str)
        .expect("computer use driver pin should include version");
    let expected_commit = pin
        .get("sourceCommit")
        .and_then(serde_json::Value::as_str)
        .expect("computer use driver pin should include sourceCommit");
    let output = std::process::Command::new(executable)
        .arg("--version")
        .output()
        .expect("computer use driver version probe should run");
    assert!(
        output.status.success(),
        "computer use driver version probe failed"
    );
    let raw = format!(
        "{} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fields: Vec<&str> = raw.split_whitespace().collect();
    assert_eq!(
        fields.as_slice(),
        [
            "clovy-computer-use-driver",
            expected_version,
            expected_commit
        ],
        "computer use helper does not match the pinned upstream source"
    );

    let stamp: serde_json::Value = serde_json::from_slice(
        &std::fs::read(stamp_path).expect("computer use driver stamp should be readable"),
    )
    .expect("computer use driver stamp should be valid JSON");
    let expected_profile = std::env::var("PROFILE").expect("Cargo build profile");
    assert_eq!(
        stamp
            .pointer("/clovyBuild/profile")
            .and_then(serde_json::Value::as_str),
        Some(expected_profile.as_str()),
        "computer use helper profile differs; prepare for this build profile first"
    );
    let expected_app_identifier = app_identifier_for_build();
    assert_eq!(
        stamp
            .pointer("/clovyBuild/appIdentifier")
            .and_then(serde_json::Value::as_str),
        Some(expected_app_identifier.as_str()),
        "computer use helper parent identity differs from this compiled app"
    );
    let mut source_hash = Sha256::new();
    let inputs: Vec<String> = serde_json::from_slice(
        &std::fs::read(manifest_dir.join("computer-use-build-inputs.json"))
            .expect("helper build inputs"),
    )
    .expect("helper build inputs JSON");
    for relative in inputs {
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir
                .parent()
                .expect("root")
                .join(&relative)
                .display()
        );
        source_hash.update(relative.as_bytes());
        source_hash.update(b"\0");
        source_hash.update(
            std::fs::read(manifest_dir.parent().expect("root").join(&relative))
                .expect("helper source"),
        );
        source_hash.update(b"\0");
    }
    let expected_source_hash = format!("{:x}", source_hash.finalize());
    assert_eq!(
        stamp
            .pointer("/clovyBuild/sourceSha256")
            .and_then(serde_json::Value::as_str),
        Some(expected_source_hash.as_str()),
        "computer use helper stamp does not match Clovy's helper source"
    );
}

fn sign_computer_use_driver(app_dir: &std::path::Path) {
    let signing = helper_signing();
    if signing.is_satisfied_by(app_dir) {
        return;
    }
    let mut command = std::process::Command::new("codesign");
    command.arg("--force");
    signing.add_signing_args(&mut command);
    let status = command.arg(app_dir).status();
    if !matches!(status, Ok(status) if status.success()) {
        panic!(
            "computer use driver could not be signed: {}",
            app_dir.display()
        );
    }
}

/// The MCP relay is compiled beside the app after build.rs runs. The macOS
/// beforeBundleCommand replaces this placeholder with the real signed relay.
fn ensure_mcp_relay_placeholder() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("macos") {
        return;
    }
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"),
    );
    let helper_dir = manifest_dir
        .parent()
        .expect("repository parent should exist")
        .join(".tauri-helper");
    let relay = helper_dir.join("clovy-mcp");
    if !relay.exists() {
        std::fs::create_dir_all(helper_dir).expect("helper directory should be created");
        std::fs::write(
            relay,
            b"placeholder: replaced by the macOS beforeBundleCommand\n",
        )
        .expect("MCP relay placeholder should be written");
    }
}

fn build_video_capture_helper() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("macos") {
        return;
    }
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest directory"));
    let source = manifest_dir.join("native/mac-video-capture/main.swift");
    println!("cargo:rerun-if-changed={}", source.display());
    let app_dir = manifest_dir
        .parent()
        .expect("repository directory")
        .join(".tauri-helper/Clovy Video Capture.app");
    let executable = app_dir.join("Contents/MacOS/clovy-video-capture");
    std::fs::create_dir_all(executable.parent().expect("helper executable directory"))
        .expect("create video helper directory");
    let rebuilt = !swift_helper_executable_current(&source, &executable);
    if rebuilt {
        // This resource is packaged unconditionally. Continuing after a failed
        // rebuild could ship a stale helper with a different control protocol.
        assert!(
            build_universal_swift_executable(
                "video capture",
                &manifest_dir,
                &source,
                &executable,
                "14.0",
                &[
                    "Foundation",
                    "AppKit",
                    "AVFoundation",
                    "ScreenCaptureKit",
                    "CoreMedia"
                ]
            ),
            "video capture helper must compile"
        );
    }
    let identifier = helper_bundle_identifier(&manifest_dir, "dev.guilhermexp.clovy.video-capture");
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>clovy-video-capture</string>
<key>CFBundleIdentifier</key><string>{identifier}</string>
<key>CFBundleName</key><string>Clovy Video Capture</string>
<key>CFBundleDisplayName</key><string>Clovy Video Capture</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>LSMinimumSystemVersion</key><string>14.0</string>
<key>LSUIElement</key><true/>
<key>NSScreenCaptureUsageDescription</key><string>Clovy records the meeting window you choose to include visible slides and shared content in your notes.</string>
</dict></plist>"#
    );
    let plist_path = app_dir.join("Contents/Info.plist");
    let changed = std::fs::read_to_string(&plist_path).ok().as_deref() != Some(&plist);
    if changed {
        std::fs::write(&plist_path, plist).expect("write video helper plist");
    }
    let signing = helper_signing();
    if rebuilt
        || changed
        || !signing.is_satisfied_by(&app_dir)
        || matches!(signing, HelperSigning::Release(_))
    {
        sign_helper_app(&manifest_dir, &app_dir, &signing);
    }
}

fn build_system_audio_helper() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("macos") {
        return;
    }
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"),
    );
    let source = manifest_dir
        .join("native")
        .join("mac-system-audio-recorder")
        .join("main.swift");
    if !source.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", source.display());
    let system_audio_min_macos_version = read_system_audio_min_macos_version(&manifest_dir)
        .expect("system audio minimum macOS version should be configured");

    let helper_dir = manifest_dir
        .parent()
        .expect("src-tauri should have a repository parent")
        .join(".tauri-helper");
    let app_dir = helper_dir.join("Clovy Audio Capture.app");
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    std::fs::create_dir_all(&macos_dir).expect("system audio helper app dir should be created");
    let executable = macos_dir.join("clovy-system-audio-recorder");

    let executable_current = swift_helper_executable_current(&source, &executable);
    let mut should_sign = false;
    if !executable_current {
        let built = build_universal_swift_executable(
            "system audio helper",
            &manifest_dir,
            &source,
            &executable,
            &system_audio_min_macos_version,
            &[
                "Foundation",
                "AppKit",
                "AVFoundation",
                "CoreAudio",
                "AudioToolbox",
            ],
        );
        if !built {
            println!("cargo:warning=system audio helper could not be built; dual-source mode will report unavailable");
            return;
        }
        should_sign = true;
    }

    let bundle_identifier =
        helper_bundle_identifier(&manifest_dir, "dev.guilhermexp.clovy.audio-capture");
    let plist = contents_dir.join("Info.plist");
    let plist_contents = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Clovy</string>
  <key>CFBundleExecutable</key>
  <string>clovy-system-audio-recorder</string>
  <key>CFBundleIdentifier</key>
  <string>{bundle_identifier}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Clovy</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSMinimumSystemVersion</key>
  <string>{system_audio_min_macos_version}</string>
  <key>LSUIElement</key>
  <true/>
  <key>NSAudioCaptureUsageDescription</key>
  <string>Clovy records system audio locally so generated notes can include meeting or media audio from your Mac.</string>
</dict>
</plist>
"#
    );
    let plist_current =
        std::fs::read_to_string(&plist).is_ok_and(|current| current == plist_contents.as_str());
    if !plist_current {
        std::fs::write(plist, &plist_contents)
            .expect("system audio helper Info.plist should be written");
        should_sign = true;
    }

    let signing = helper_signing();
    if should_sign
        || matches!(signing, HelperSigning::Release(_))
        || !signing.is_satisfied_by(&app_dir)
    {
        sign_helper_app(&manifest_dir, &app_dir, &signing);
    }
}

fn build_windows_dictation_helper() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("windows") {
        return;
    }
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"),
    );
    let helper_manifest = manifest_dir
        .join("native")
        .join("windows-dictation-helper")
        .join("Cargo.toml");
    if !helper_manifest.exists() {
        panic!(
            "Windows dictation helper manifest is missing at {}",
            helper_manifest.display()
        );
    }
    println!("cargo:rerun-if-changed={}", helper_manifest.display());
    for source in [
        "main.rs",
        "protocol.rs",
        "hotkeys.rs",
        "audio.rs",
        "clipboard.rs",
        "focus.rs",
        "permissions.rs",
    ] {
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir
                .join("native")
                .join("windows-dictation-helper")
                .join("src")
                .join(source)
                .display()
        );
    }

    let target = std::env::var("TARGET").ok();
    let target_dir = manifest_dir.join("target").join("windows-dictation-helper");
    let mut command = std::process::Command::new("cargo");
    command
        .arg("build")
        .arg("--release")
        .arg("--locked")
        .arg("--manifest-path")
        .arg(&helper_manifest)
        .arg("--target-dir")
        .arg(&target_dir);
    if let Some(target) = target.as_deref() {
        command.arg("--target").arg(target);
    }
    let status = command.status().unwrap_or_else(|error| {
        panic!(
            "Windows dictation helper build command could not start for {}: {error}",
            helper_manifest.display()
        )
    });
    if !status.success() {
        panic!(
            "Windows dictation helper build failed for {} with status {status}",
            helper_manifest.display()
        );
    }

    let mut built = target_dir
        .join("release")
        .join("clovy-dictation-helper.exe");
    if let Some(target) = target.as_deref() {
        built = target_dir
            .join(target)
            .join("release")
            .join("clovy-dictation-helper.exe");
    }
    let destination = manifest_dir
        .join("native")
        .join("bin")
        .join("clovy-dictation-helper.exe");
    println!("cargo:rerun-if-changed={}", destination.display());
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .expect("Windows dictation helper destination should be created");
    }
    let should_copy = match (std::fs::read(&built), std::fs::read(&destination)) {
        (Ok(built_contents), Ok(current_contents)) => built_contents != current_contents,
        (Ok(_), Err(_)) => true,
        (Err(error), _) => {
            panic!(
                "Windows dictation helper {} should be readable before copying to {}: {error}",
                built.display(),
                destination.display()
            )
        }
    };
    if should_copy {
        std::fs::copy(&built, &destination).unwrap_or_else(|error| {
            panic!(
                "Windows dictation helper {} should copy to {}: {error}",
                built.display(),
                destination.display()
            )
        });
    }
    sign_windows_helper_if_configured(&manifest_dir, &destination);
}

fn sign_windows_helper_if_configured(manifest_dir: &std::path::Path, helper: &std::path::Path) {
    if std::env::var("WINDOWS_CERTIFICATE_PASSWORD")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .is_none()
    {
        return;
    }
    let Some(repo_dir) = manifest_dir.parent() else {
        return;
    };
    let script = repo_dir.join("scripts").join("windows-sign.ps1");
    if !script.exists() {
        panic!(
            "Windows signing requested but {} is missing",
            script.display()
        );
    }
    let status = std::process::Command::new("powershell.exe")
        .arg("-NoLogo")
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script)
        .arg(helper)
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "Windows dictation helper signing command could not start for {}: {error}",
                helper.display()
            )
        });
    if !status.success() {
        panic!(
            "Windows dictation helper {} signing failed with status {status}",
            helper.display()
        );
    }
}

fn read_system_audio_min_macos_version(manifest_dir: &std::path::Path) -> Option<String> {
    let version_file = manifest_dir.join(SYSTEM_AUDIO_MIN_MACOS_VERSION_FILE);
    println!("cargo:rerun-if-changed={}", version_file.display());
    let version = std::fs::read_to_string(version_file).ok()?;
    let version = version.trim();
    if version.is_empty() {
        return None;
    }
    Some(version.to_string())
}

fn build_dictation_helper() {
    if std::env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("macos") {
        return;
    }
    let manifest_dir = std::path::PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR should be set"),
    );
    let source = manifest_dir
        .join("native")
        .join("mac-dictation-helper")
        .join("main.swift");
    if !source.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", source.display());

    let helper_dir = manifest_dir
        .parent()
        .expect("src-tauri should have a repository parent")
        .join(".tauri-helper");
    let app_dir = helper_dir.join("Clovy Dictation Helper.app");
    let contents_dir = app_dir.join("Contents");
    let macos_dir = contents_dir.join("MacOS");
    let resources_dir = contents_dir.join("Resources");
    std::fs::create_dir_all(&macos_dir).expect("dictation helper app dir should be created");
    std::fs::create_dir_all(&resources_dir)
        .expect("dictation helper resources dir should be created");
    let executable = macos_dir.join("clovy-dictation-helper");
    let icon_source = manifest_dir.join("icons").join("icon.icns");
    let icon_destination = resources_dir.join("Clovy.icns");

    let executable_current = swift_helper_executable_current(&source, &executable);
    let mut should_sign = false;
    if !executable_current {
        let built = build_universal_swift_executable(
            "dictation helper",
            &manifest_dir,
            &source,
            &executable,
            DICTATION_HELPER_MIN_MACOS_VERSION,
            &[
                "Foundation",
                "AppKit",
                "AVFoundation",
                "Carbon",
                "CoreMedia",
                "CoreGraphics",
            ],
        );
        if !built {
            println!("cargo:warning=dictation helper could not be built; dictation will report unavailable");
            return;
        }
        should_sign = true;
    }

    let bundle_identifier =
        helper_bundle_identifier(&manifest_dir, "dev.guilhermexp.clovy.dictation-helper");
    let plist = contents_dir.join("Info.plist");
    let plist_contents = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleDisplayName</key>
  <string>Clovy Dictation Helper</string>
  <key>CFBundleExecutable</key>
  <string>clovy-dictation-helper</string>
  <key>CFBundleIdentifier</key>
  <string>{bundle_identifier}</string>
  <key>CFBundleIconFile</key>
  <string>Clovy.icns</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>Clovy Dictation Helper</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSUIElement</key>
  <true/>
  <key>LSMinimumSystemVersion</key>
  <string>{DICTATION_HELPER_MIN_MACOS_VERSION}</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>Clovy needs microphone access to turn your speech into text.</string>
</dict>
</plist>
"#
    );
    let plist_current =
        std::fs::read_to_string(&plist).is_ok_and(|current| current == plist_contents.as_str());
    if !plist_current {
        std::fs::write(plist, &plist_contents)
            .expect("dictation helper Info.plist should be written");
        should_sign = true;
    }

    let icon_bytes = std::fs::read(&icon_source).unwrap_or_else(|error| {
        panic!(
            "Clovy app icon {} should be readable: {error}",
            icon_source.display()
        )
    });
    let icon_current = std::fs::read(&icon_destination).is_ok_and(|current| current == icon_bytes);
    if !icon_current {
        std::fs::write(&icon_destination, icon_bytes).unwrap_or_else(|error| {
            panic!(
                "Clovy app icon should be copied to dictation helper resources at {}: {error}",
                icon_destination.display()
            )
        });
        should_sign = true;
    }

    let public_sounds_dir = manifest_dir
        .parent()
        .expect("src-tauri should have a repository parent")
        .join("public")
        .join("sounds");
    for sound in ["record-start.mp3", "record-end.mp3"] {
        let source = public_sounds_dir.join(sound);
        println!("cargo:rerun-if-changed={}", source.display());
        let destination = resources_dir.join(sound);
        let source_bytes = std::fs::read(&source).unwrap_or_else(|error| {
            panic!(
                "dictation sound {} should be readable: {error}",
                source.display()
            )
        });
        let destination_current =
            std::fs::read(&destination).is_ok_and(|current| current == source_bytes);
        if !destination_current {
            std::fs::write(&destination, source_bytes).unwrap_or_else(|error| {
                panic!(
                    "dictation sound {} should be copied to helper resources: {error}",
                    destination.display()
                )
            });
            should_sign = true;
        }
    }

    let signing = helper_signing();
    if should_sign
        || matches!(signing, HelperSigning::Release(_))
        || !signing.is_satisfied_by(&app_dir)
    {
        sign_helper_app(&manifest_dir, &app_dir, &signing);
    }
}

fn swift_helper_executable_current(source: &std::path::Path, executable: &std::path::Path) -> bool {
    if !executable.exists() {
        return false;
    }

    let source_modified = std::fs::metadata(source)
        .and_then(|metadata| metadata.modified())
        .ok();
    let executable_fresh = source_modified
        .and_then(|source_modified| {
            std::fs::metadata(executable)
                .and_then(|metadata| metadata.modified())
                .ok()
                .map(|executable_modified| executable_modified >= source_modified)
        })
        .unwrap_or(false);
    executable_fresh && executable_has_arches(executable, &["arm64", "x86_64"])
}

fn executable_has_arches(executable: &std::path::Path, required_arches: &[&str]) -> bool {
    let output = std::process::Command::new("lipo")
        .arg("-archs")
        .arg(executable)
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    required_arches
        .iter()
        .all(|required| stdout.split_whitespace().any(|arch| arch == *required))
}

fn build_universal_swift_executable(
    helper_name: &str,
    manifest_dir: &std::path::Path,
    source: &std::path::Path,
    executable: &std::path::Path,
    macos_version: &str,
    frameworks: &[&str],
) -> bool {
    let Some(executable_name) = executable.file_name().and_then(|name| name.to_str()) else {
        println!("cargo:warning={helper_name} executable path has no file name");
        return false;
    };
    let slice_dir = helper_target_dir(manifest_dir)
        .join("swift-helper-slices")
        .join(format!("{executable_name}-{}", std::process::id()));
    if let Err(error) = std::fs::create_dir_all(&slice_dir) {
        println!("cargo:warning={helper_name} slice dir could not be created: {error}");
        return false;
    }

    let mut slices = Vec::new();
    for swift_arch in ["arm64", "x86_64"] {
        let slice = slice_dir.join(format!("{executable_name}-{swift_arch}"));
        let mut command = std::process::Command::new("swiftc");
        configure_swift_command(&mut command, manifest_dir, swift_arch, macos_version);
        for framework in frameworks {
            command.arg("-framework").arg(framework);
        }
        let status = command.arg(source).arg("-o").arg(&slice).status();
        if !matches!(status, Ok(status) if status.success()) {
            println!("cargo:warning={helper_name} {swift_arch} slice could not be built");
            let _ = std::fs::remove_dir_all(&slice_dir);
            return false;
        }
        slices.push(slice);
    }

    let universal = slice_dir.join(format!("{executable_name}-universal"));
    let mut lipo = std::process::Command::new("lipo");
    lipo.arg("-create").arg("-output").arg(&universal);
    for slice in &slices {
        lipo.arg(slice);
    }
    let status = lipo.status();
    if !matches!(status, Ok(status) if status.success()) {
        println!("cargo:warning={helper_name} universal binary could not be created");
        let _ = std::fs::remove_dir_all(&slice_dir);
        return false;
    }

    if let Ok(permissions) = std::fs::metadata(&slices[0]).map(|metadata| metadata.permissions()) {
        let _ = std::fs::set_permissions(&universal, permissions);
    }
    if let Err(error) = install_helper_executable(&universal, executable) {
        println!("cargo:warning={helper_name} universal binary could not be installed: {error}");
        let _ = std::fs::remove_dir_all(&slice_dir);
        return false;
    }
    let _ = std::fs::remove_dir_all(&slice_dir);
    true
}

/// Stage on the bundle's filesystem, then atomically replace its executable.
/// The configured Cargo target may live on a different volume.
fn install_helper_executable(
    universal: &std::path::Path,
    executable: &std::path::Path,
) -> std::io::Result<()> {
    let staged = executable.with_extension(format!("install-{}", std::process::id()));
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged)?;
    let result = (|| {
        let mut input = std::fs::File::open(universal)?;
        std::io::copy(&mut input, &mut output)?;
        output.set_permissions(input.metadata()?.permissions())?;
        output.sync_all()?;
        drop(output);
        std::fs::rename(&staged, executable)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

/// Bundle identifier for a helper built here. Release builds keep the published
/// identifier. Debug builds use `<published>.dev.w<worktree key>`, the scheme
/// `scripts/computer-use-dev.mjs` gives the Computer use driver (SHA-256 of
/// the checkout path, first 12 hex digits), so a development grant or
/// signature never lands on an installed release or another worktree
/// (docs/development.md).
fn helper_bundle_identifier(manifest_dir: &std::path::Path, published: &str) -> String {
    use sha2::{Digest, Sha256};

    if std::env::var("PROFILE").ok().as_deref() != Some("debug") {
        return published.to_string();
    }
    let worktree = manifest_dir
        .parent()
        .expect("src-tauri should have a repository parent");
    let digest = Sha256::digest(worktree.to_string_lossy().as_bytes());
    let key: String = digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("{published}.dev.w{key}")
}

/// The identity build.rs signs helper bundles with.
enum HelperSigning {
    /// `APPLE_SIGNING_IDENTITY`: release signing with a secure timestamp and
    /// the hardened runtime.
    Release(String),
    /// `CLOVY_DEV_SIGNING_IDENTITY` in debug builds: the certificate SHA-1
    /// `pnpm tauri:dev` also signs the dev app bundle with, so helper grants
    /// keep matching their designated requirement after rebuilds
    /// (docs/development.md).
    Development(String),
    /// `CLOVY_DEV_SIGNING_IDENTITY=-`: the developer explicitly chose ad hoc,
    /// so a helper signed with a certificate is re-signed ad hoc.
    AdHoc,
    /// No identity selected (plain `cargo build`/`cargo test`, unsigned
    /// release): new or changed helpers are signed ad hoc, and an existing
    /// valid signature is kept, never downgraded.
    Unselected,
}

/// Debug builds started by `pnpm tauri:dev` carry `CLOVY_DEV_SIGNING_IDENTITY`,
/// the identity the runner also signs the app bundle with. It wins over an
/// `APPLE_SIGNING_IDENTITY` inherited from the shell, so the app and its
/// helpers keep one certificate (`-` keeps the explicit ad hoc opt-in). Release
/// builds, and debug builds outside the launcher, keep `APPLE_SIGNING_IDENTITY`.
fn helper_signing() -> HelperSigning {
    println!("cargo:rerun-if-env-changed=APPLE_SIGNING_IDENTITY");
    println!("cargo:rerun-if-env-changed=CLOVY_DEV_SIGNING_IDENTITY");
    let configured = |name: &str| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    if std::env::var("PROFILE").ok().as_deref() == Some("debug") {
        match configured("CLOVY_DEV_SIGNING_IDENTITY") {
            Some(identity) if identity == "-" => return HelperSigning::AdHoc,
            Some(identity) => return HelperSigning::Development(identity),
            None => {}
        }
    }
    match configured("APPLE_SIGNING_IDENTITY") {
        Some(identity) => HelperSigning::Release(identity),
        None => HelperSigning::Unselected,
    }
}

impl HelperSigning {
    fn add_signing_args(&self, command: &mut std::process::Command) {
        match self {
            Self::Release(identity) => {
                command
                    .arg("--sign")
                    .arg(identity)
                    .arg("--timestamp")
                    .arg("--options")
                    .arg("runtime");
            }
            Self::Development(identity) => {
                command.arg("--sign").arg(identity).arg("--timestamp=none");
            }
            Self::AdHoc | Self::Unselected => {
                command.arg("--sign").arg("-");
            }
        }
    }

    /// Whether `bundle` already carries a valid signature this selection
    /// accepts: the selected certificate, the release identity, an ad hoc
    /// signature for an explicit ad hoc choice, or any valid signature when
    /// nothing was selected.
    fn is_satisfied_by(&self, bundle: &std::path::Path) -> bool {
        let mut verify = std::process::Command::new("/usr/bin/codesign");
        verify.arg("--verify").arg("--deep").arg("--strict");
        if let Self::Development(identity) = self {
            verify.arg(format!("-R=certificate leaf = H\"{identity}\""));
        }
        if !verify
            .arg(bundle)
            .status()
            .is_ok_and(|status| status.success())
        {
            return false;
        }
        if matches!(self, Self::Development(_) | Self::Unselected) {
            return true;
        }
        let Ok(details) = std::process::Command::new("/usr/bin/codesign")
            .arg("-dv")
            .arg("--verbose=4")
            .arg(bundle)
            .output()
        else {
            return false;
        };
        let details = format!(
            "{}\n{}",
            String::from_utf8_lossy(&details.stdout),
            String::from_utf8_lossy(&details.stderr)
        );
        match self {
            Self::Release(identity) => details
                .lines()
                .any(|line| line.trim() == format!("Authority={identity}")),
            _ => details.lines().any(|line| line.trim() == "Signature=adhoc"),
        }
    }
}

fn sign_helper_app(
    manifest_dir: &std::path::Path,
    app_dir: &std::path::Path,
    signing: &HelperSigning,
) {
    let entitlements = manifest_dir.join("Entitlements.plist");
    let mut command = std::process::Command::new("codesign");
    command
        .arg("--force")
        .arg("--deep")
        .arg("--entitlements")
        .arg(entitlements);
    signing.add_signing_args(&mut command);
    let status = command.arg(app_dir).status();
    if !matches!(status, Ok(status) if status.success()) {
        // A development identity that cannot sign must stop the build: an
        // ad hoc or stale helper would silently lose its privacy grants.
        if matches!(signing, HelperSigning::Development(_)) {
            panic!(
                "helper app could not be signed with CLOVY_DEV_SIGNING_IDENTITY: {}",
                app_dir.display()
            );
        }
        println!(
            "cargo:warning=helper app could not be signed: {}",
            app_dir.display()
        );
    }
}

fn helper_target_dir(manifest_dir: &std::path::Path) -> std::path::PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| manifest_dir.join("target"))
}

fn configure_swift_command(
    command: &mut std::process::Command,
    manifest_dir: &std::path::Path,
    swift_arch: &str,
    macos_version: &str,
) {
    command
        .arg("-target")
        .arg(format!("{swift_arch}-apple-macosx{macos_version}"));
    let module_cache = helper_target_dir(manifest_dir).join("swift-module-cache");
    std::fs::create_dir_all(&module_cache).expect("swift module cache dir should be created");
    command.arg("-module-cache-path").arg(module_cache);
}
