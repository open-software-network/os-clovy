#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { computerUseBundleIdentifier } from "./computer-use-dev.mjs";
import {
  helperBuildInputsMatch,
  verifyEngineArtifact,
  verifyEngineSbom,
} from "./agent-desktop-artifact.mjs";
import {
  AD_HOC_IDENTITY,
  DEV_SIGNING_IDENTITY_ENV,
  signatureSatisfies,
} from "./dev-signing-identity.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pinPath = path.join(rootDir, "src-tauri", "agent-desktop-pin.json");
const pin = JSON.parse(readFileSync(pinPath, "utf8"));
const sbomSource = path.join(rootDir, "src-tauri", "agent-desktop", "sbom.spdx.json");
const sbomBytes = readFileSync(sbomSource);
verifyEngineSbom(sbomBytes, pin);
const licenseSource = path.join(rootDir, "src-tauri", "agent-desktop", "LICENSE");
const iconSource = path.join(rootDir, "src-tauri", "icons", "icon.icns");
const iconFileName = "Clovy.icns";

const tauriPlatform = process.env.TAURI_ENV_PLATFORM?.trim();
if (
  !["darwin", "win32"].includes(process.platform) ||
  (tauriPlatform && !["darwin", "windows"].includes(tauriPlatform))
) {
  throw new Error("Computer use helper packaging requires macOS or Windows");
}

const parsed = parseArguments(process.argv.slice(2));
const release = parsed.release;
const target =
  parsed.target ||
  tauriTargetTriple() ||
  (process.platform === "win32"
    ? `${process.arch === "arm64" ? "aarch64" : "x86_64"}-pc-windows-msvc`
    : `${process.arch === "arm64" ? "aarch64" : "x86_64"}-apple-darwin`);
const preparedEngineDir = path.join(rootDir, ".tauri-helper", "agent-desktop", target);
const preparedEngineManifest = verifyEngineArtifact(preparedEngineDir, pin, target);
const engineSha256 = preparedEngineManifest.executableSha256;
const preparedEngineBytes = readFileSync(
  path.join(preparedEngineDir, preparedEngineManifest.executable),
);
if (createHash("sha256").update(preparedEngineBytes).digest("hex") !== engineSha256)
  throw new Error("Prepared engine changed while helper preparation started");
const preparedReview = readFileSync(path.join(preparedEngineDir, "source-review.md"));
if (createHash("sha256").update(preparedReview).digest("hex") !== pin.sourceReviewSha256)
  throw new Error("Prepared source qualification changed while helper preparation started");
const profile = release ? "release" : "debug";
const cargoTargetDir = path.resolve(
  process.env.CARGO_TARGET_DIR || path.join(rootDir, "src-tauri", "target"),
);
const overlayIdentifier = process.env.TAURI_CONFIG
  ? JSON.parse(process.env.TAURI_CONFIG).identifier
  : undefined;
const compiledAppIdentifier =
  overlayIdentifier ||
  (!release ? process.env.CLOVY_DEV_APP_IDENTIFIER : undefined) ||
  JSON.parse(readFileSync(path.join(rootDir, "src-tauri", "tauri.conf.json"), "utf8")).identifier;
if (!/^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/.test(compiledAppIdentifier))
  throw new Error("Invalid Computer use helper parent identity");
const bundleIdentifier = computerUseBundleIdentifier({
  baseIdentifier: pin.bundleIdentifier,
  profile,
  worktreeRoot: rootDir,
});
if (process.platform === "win32") {
  prepareWindowsHelper(target, release);
  process.exit(0);
}
const rustTargets =
  target === "universal-apple-darwin"
    ? ["aarch64-apple-darwin", "x86_64-apple-darwin"]
    : [target].filter(Boolean);
const architectures =
  rustTargets.length > 0
    ? rustTargets.map(architectureForTarget)
    : [process.arch === "x64" ? "x86_64" : "arm64"];
const sourceSha256 = helperSourceSha256();
const helperDir = path.join(rootDir, ".tauri-helper");
const bundleDir = path.join(helperDir, pin.bundleName);
const contentsDir = path.join(bundleDir, "Contents");
const executableDir = path.join(contentsDir, "MacOS");
const resourcesDir = path.join(contentsDir, "Resources");
const executable = path.join(executableDir, pin.executable);
const stampPath = path.join(resourcesDir, "clovy-computer-use-pin.json");
const iconDestination = path.join(resourcesDir, iconFileName);
// Debug helpers launched by `pnpm tauri:dev` carry the same certificate as the
// dev app bundle so their Accessibility grant survives rebuilds
// (docs/development.md); an explicit `CLOVY_DEV_SIGNING_IDENTITY=-` re-signs
// even an unchanged helper ad hoc. Without a selection (release, or a manual
// `pnpm computer-use:prepare`), new bundles are signed ad hoc and an existing
// valid signature is kept; build.rs re-signs release with APPLE_SIGNING_IDENTITY.
const selectedIdentity = release
  ? process.env.APPLE_SIGNING_IDENTITY?.trim() || undefined
  : process.env[DEV_SIGNING_IDENTITY_ENV]?.trim() || undefined;

if (preparedBundleContentMatches()) {
  if (bundleSignatureMatches()) {
    console.error(
      `Reusing ${profile} Computer use helper for ${architectures.join("+")} from ${pin.sourceCommit}.`,
    );
  } else {
    signBundle();
    console.error(`Re-signed the unchanged ${profile} Computer use helper.`);
  }
  stageUniversalCargoBin();
  process.exit(0);
}

// The helper bootstrap excludes runtime artifact validation in build.rs;
// the final desktop build validates the completed resource before bundling.
rmSync(bundleDir, { recursive: true, force: true });

const developerDir = existsSync("/Applications/Xcode.app/Contents/Developer")
  ? "/Applications/Xcode.app/Contents/Developer"
  : process.env.DEVELOPER_DIR;
const buildTargets = rustTargets.length > 0 ? rustTargets : [undefined];
const builtExecutables = [];
for (const rustTarget of buildTargets) {
  const cargoArgs = [
    "build",
    "--manifest-path",
    path.join(rootDir, "src-tauri", "Cargo.toml"),
    "--bin",
    pin.executable,
    "--locked",
  ];
  if (release) cargoArgs.push("--release");
  if (rustTarget) cargoArgs.push("--target", rustTarget);
  run(process.env.CARGO || "cargo", cargoArgs, {
    cwd: rootDir,
    env: {
      ...process.env,
      CLOVY_PREPARING_COMPUTER_USE: "1",
      CLOVY_COMPUTER_USE_HELPER_IDENTIFIER: bundleIdentifier,
      CLOVY_PREPARED_AGENT_DESKTOP_SHA256: engineSha256,
      CLOVY_PREPARED_AGENT_DESKTOP_TARGET: target,
      ...(!release ? { CLOVY_DEV_APP_IDENTIFIER: compiledAppIdentifier } : {}),
      CARGO_INCREMENTAL: "0",
      CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP: "none",
      CARGO_PROFILE_DEV_BUILD_OVERRIDE_STRIP: "none",
      MACOSX_DEPLOYMENT_TARGET: pin.minimumMacOSVersion,
      ...(developerDir ? { DEVELOPER_DIR: developerDir } : {}),
    },
  });
  builtExecutables.push(
    path.join(cargoTargetDir, ...(rustTarget ? [rustTarget] : []), profile, pin.executable),
  );
}

if (helperSourceSha256() !== sourceSha256)
  throw new Error(
    "Computer use helper source changed during compilation; prepare again from a stable checkout",
  );

rmSync(bundleDir, { recursive: true, force: true });
mkdirSync(executableDir, { recursive: true });
mkdirSync(resourcesDir, { recursive: true });
if (builtExecutables.length === 1) {
  cpSync(builtExecutables[0], executable);
} else {
  run("/usr/bin/lipo", ["-create", ...builtExecutables, "-output", executable]);
}
run("/bin/chmod", ["755", executable]);

writeFileSync(
  path.join(contentsDir, "Info.plist"),
  `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleDisplayName</key><string>Clovy Computer Use Driver</string>
  <key>CFBundleExecutable</key><string>${pin.executable}</string>
  <key>CFBundleIdentifier</key><string>${bundleIdentifier}</string>
  <key>CFBundleIconFile</key><string>${iconFileName}</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>Clovy Computer Use Driver</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${pin.version}</string>
  <key>CFBundleVersion</key><string>${pin.version}</string>
  <key>LSMinimumSystemVersion</key><string>${pin.minimumMacOSVersion}</string>
  <key>LSUIElement</key><true/>
  <key>NSAccessibilityUsageDescription</key><string>Clovy uses Accessibility only to operate the app windows you ask it to control.</string>
  <key>NSScreenCaptureUsageDescription</key><string>Clovy captures only the app windows you ask it to operate so Clovy can understand and complete your task.</string>
</dict>
</plist>
`,
);
writeFileSync(
  stampPath,
  `${JSON.stringify(
    {
      ...pin,
      bundleIdentifier,
      clovyBuild: {
        profile,
        appIdentifier: compiledAppIdentifier,
        engineSha256,
        architectures: [...architectures].sort(),
        sourceSha256,
      },
    },
    null,
    2,
  )}\n`,
);
stageEngine(executableDir, resourcesDir, target);
writeFileSync(path.join(resourcesDir, "agent-desktop.spdx.json"), sbomBytes);
cpSync(licenseSource, path.join(resourcesDir, "agent-desktop-LICENSE"));
cpSync(iconSource, iconDestination);
signBundle();

const version = helperVersion(executable);
if (version.version !== pin.version || version.commit !== pin.sourceCommit) {
  throw new Error(
    `Built helper reports ${version.version || "no version"} ${version.commit || "no commit"}; expected ${pin.version} ${pin.sourceCommit}`,
  );
}
console.error(
  `Prepared authenticated Computer use helper using agent-desktop ${pin.version} (${pin.sourceCommit}).`,
);
stageUniversalCargoBin();

// Tauri bundles every cargo binary of the crate and, for the pseudo-target
// universal-apple-darwin, expects each one already lipo-merged under
// target/universal-apple-darwin/<profile>/. Cargo only produces per-arch
// binaries and Tauri merges only the main one, so stage the merged helper
// executable there ourselves.
function stageUniversalCargoBin() {
  if (target !== "universal-apple-darwin") return;
  const stagedBin = path.join(cargoTargetDir, "universal-apple-darwin", profile, pin.executable);
  mkdirSync(path.dirname(stagedBin), { recursive: true });
  cpSync(executable, stagedBin);
  run("/bin/chmod", ["755", stagedBin]);
}

function helperVersion(file) {
  const result = spawnSync(file, ["--version"], { encoding: "utf8" });
  if (result.status !== 0) return {};
  const match = `${result.stdout}\n${result.stderr}`.match(
    /clovy-computer-use-driver\s+([^\s]+)\s+([0-9a-f]{40})/,
  );
  return { version: match?.[1], commit: match?.[2] };
}

function helperSourceSha256() {
  const hash = createHash("sha256");
  for (const relative of JSON.parse(
    readFileSync(path.join(rootDir, "src-tauri", "computer-use-build-inputs.json"), "utf8"),
  )) {
    hash.update(relative);
    hash.update("\0");
    hash.update(readFileSync(path.join(rootDir, relative)));
    hash.update("\0");
  }
  return hash.digest("hex");
}

// The prepared bundle is reusable when its stamp, executable, and icon match
// this checkout and its existing seal still verifies.
function preparedBundleContentMatches() {
  if (!existsSync(executable) || !existsSync(stampPath)) return false;
  let stamp;
  try {
    stamp = JSON.parse(readFileSync(stampPath, "utf8"));
  } catch {
    return false;
  }
  const version = helperVersion(executable);
  const actualArchitectures = new Set(
    spawnSync("/usr/bin/lipo", ["-archs", executable], { encoding: "utf8" })
      .stdout?.trim()
      .split(/\s+/)
      .filter(Boolean) || [],
  );
  return (
    stamp.version === pin.version &&
    stamp.sourceCommit === pin.sourceCommit &&
    stamp.bundleIdentifier === bundleIdentifier &&
    helperBuildInputsMatch(stamp, {
      profile,
      appIdentifier: compiledAppIdentifier,
      engineSha256,
      sourceSha256,
    }) &&
    architectures.every((architecture) => actualArchitectures.has(architecture)) &&
    version.version === pin.version &&
    version.commit === pin.sourceCommit &&
    engineMatches() &&
    bundleIconMatches() &&
    bundleSealMatches()
  );
}

function bundleIconMatches() {
  if (!existsSync(iconDestination)) return false;
  if (!readFileSync(iconDestination).equals(readFileSync(iconSource))) return false;
  const plist = path.join(contentsDir, "Info.plist");
  const result = spawnSync(
    "/usr/bin/plutil",
    ["-extract", "CFBundleIconFile", "raw", "-o", "-", plist],
    { encoding: "utf8" },
  );
  return result.status === 0 && result.stdout.trim() === iconFileName;
}

function bundleSealMatches() {
  const verification = spawnSync(
    "/usr/bin/codesign",
    ["--verify", "--deep", "--strict", bundleDir],
    { encoding: "utf8" },
  );
  if (verification.status !== 0) return false;

  const details = spawnSync("/usr/bin/codesign", ["-dv", "--verbose=4", bundleDir], {
    encoding: "utf8",
  });
  return (
    details.status === 0 &&
    `${details.stdout || ""}\n${details.stderr || ""}`.includes(`Identifier=${bundleIdentifier}`)
  );
}

function bundleSignatureMatches() {
  return selectedIdentity === undefined || signatureSatisfies(bundleDir, selectedIdentity);
}

function signBundle() {
  const releaseSigning = release && selectedIdentity && selectedIdentity !== AD_HOC_IDENTITY;
  run("/usr/bin/codesign", [
    "--force",
    "--sign",
    selectedIdentity ?? AD_HOC_IDENTITY,
    ...(releaseSigning ? ["--timestamp", "--options", "runtime"] : ["--timestamp=none"]),
    "--identifier",
    bundleIdentifier,
    bundleDir,
  ]);
}

function architectureForTarget(rustTarget) {
  if (rustTarget === "aarch64-apple-darwin") return "arm64";
  if (rustTarget === "x86_64-apple-darwin") return "x86_64";
  throw new Error(`Unsupported Computer use helper target: ${rustTarget}`);
}

function tauriTargetTriple() {
  const target = process.env.TAURI_ENV_TARGET_TRIPLE?.trim();
  if (!target) return undefined;
  if (
    target === "universal-apple-darwin" ||
    target === "aarch64-apple-darwin" ||
    target === "x86_64-apple-darwin" ||
    target === "x86_64-pc-windows-msvc" ||
    target === "aarch64-pc-windows-msvc"
  ) {
    return target;
  }
  throw new Error(`Unsupported Computer use helper target from Tauri: ${target}`);
}

function parseArguments(values) {
  let release = false;
  let target;
  for (let index = 0; index < values.length; index += 1) {
    const argument = values[index];
    // pnpm forwards the conventional `--` separator verbatim (npm strips it).
    if (argument === "--") {
      continue;
    }
    if (argument === "--release") {
      release = true;
      continue;
    }
    if (argument === "--target") {
      target = values[index + 1];
      if (!target || target.startsWith("--")) {
        throw new Error("--target requires a Rust target");
      }
      index += 1;
      continue;
    }
    if (argument.startsWith("--target=")) {
      target = argument.slice("--target=".length);
      if (!target) throw new Error("--target requires a Rust target");
      continue;
    }
    throw new Error(`Unknown Computer use helper preparation argument: ${argument}`);
  }
  return { release, target };
}

function run(command, commandArgs, options = {}) {
  const result = spawnSync(command, commandArgs, { ...options, stdio: "inherit" });
  if (result.status !== 0) {
    throw new Error(`${command} exited with ${result.status ?? "no status"}`);
  }
}

function engineMatches() {
  try {
    const staged = JSON.parse(
      readFileSync(path.join(resourcesDir, "agent-desktop-manifest.json"), "utf8"),
    );
    return (
      JSON.stringify(staged) === JSON.stringify(preparedEngineManifest) &&
      createHash("sha256")
        .update(readFileSync(path.join(executableDir, "agent-desktop")))
        .digest("hex") === engineSha256
    );
  } catch {
    return false;
  }
}
function stageEngine(binaryDir, resourceDir, triple) {
  if (triple !== target) throw new Error("Prepared engine target changed");
  const executableName = triple.includes("windows") ? "agent-desktop.exe" : "agent-desktop";
  writeFileSync(path.join(binaryDir, executableName), preparedEngineBytes, { mode: 0o755 });
  writeFileSync(
    path.join(resourceDir, "agent-desktop-manifest.json"),
    `${JSON.stringify(preparedEngineManifest, null, 2)}\n`,
  );
  writeFileSync(path.join(resourceDir, "source-review.md"), preparedReview);
}
function prepareWindowsHelper(triple, isRelease) {
  if (!triple.endsWith("-pc-windows-msvc") || !process.env.WINDOWS_CERTIFICATE_THUMBPRINT)
    throw new Error("Windows helper requires MSVC target and WINDOWS_CERTIFICATE_THUMBPRINT");
  const sourceSha256 = helperSourceSha256();
  const destination = path.join(rootDir, ".tauri-helper", "computer-use");
  mkdirSync(destination, { recursive: true });
  const args = [
    "build",
    "--manifest-path",
    path.join(rootDir, "src-tauri", "Cargo.toml"),
    "--bin",
    pin.executable,
    "--locked",
    "--target",
    triple,
    ...(isRelease ? ["--release"] : []),
  ];
  run(process.env.CARGO || "cargo", args, {
    cwd: rootDir,
    env: {
      ...process.env,
      CLOVY_PREPARING_COMPUTER_USE: "1",
      CLOVY_COMPUTER_USE_HELPER_IDENTIFIER: bundleIdentifier,
      CLOVY_PREPARED_AGENT_DESKTOP_SHA256: engineSha256,
      CLOVY_PREPARED_AGENT_DESKTOP_TARGET: target,
      ...(!isRelease ? { CLOVY_DEV_APP_IDENTIFIER: compiledAppIdentifier } : {}),
      CARGO_INCREMENTAL: "0",
    },
  });
  if (helperSourceSha256() !== sourceSha256)
    throw new Error(
      "Computer use helper source changed during compilation; prepare again from a stable checkout",
    );
  const targetDir = process.env.CARGO_TARGET_DIR || path.join(rootDir, "src-tauri", "target");
  const helper = path.join(destination, `${pin.executable}.exe`);
  cpSync(
    path.join(targetDir, triple, isRelease ? "release" : "debug", `${pin.executable}.exe`),
    helper,
  );
  stageEngine(destination, destination, triple);
  cpSync(licenseSource, path.join(destination, "LICENSE"));
  writeFileSync(path.join(destination, "agent-desktop.spdx.json"), sbomBytes);
  run(process.env.WINDOWS_SIGNTOOL_PATH || "signtool", [
    "sign",
    "/sha1",
    process.env.WINDOWS_CERTIFICATE_THUMBPRINT,
    "/fd",
    "sha256",
    "/td",
    "sha256",
    "/tr",
    process.env.WINDOWS_SIGNING_TIMESTAMP_URL || "http://timestamp.digicert.com",
    helper,
  ]);
  run(process.env.WINDOWS_SIGNTOOL_PATH || "signtool", ["verify", "/pa", helper]);
  writeFileSync(
    path.join(destination, "clovy-computer-use-pin.json"),
    `${JSON.stringify({ ...pin, clovyBuild: { profile: isRelease ? "release" : "debug", appIdentifier: compiledAppIdentifier, engineSha256, sourceSha256 } }, null, 2)}\n`,
  );
}
