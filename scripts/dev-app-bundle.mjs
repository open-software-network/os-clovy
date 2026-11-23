#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  constants,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  AD_HOC_IDENTITY,
  resolveDevSigningIdentity,
  signatureSatisfies,
} from "./dev-signing-identity.mjs";

// The development app runs from a signed bundle beside Cargo's output so macOS
// can show, grant, and keep its privacy permissions across restarts and
// rebuilds (docs/development.md). The bundle keeps the packaged layout:
// Contents/MacOS/clovy is the main executable the Computer use helper
// authenticates, with the MCP relay beside it.
const BUNDLE_FORMAT = 1;
const MAIN_EXECUTABLE = "clovy";
const SIBLING_EXECUTABLES = ["clovy-mcp"];
const BUNDLE_NAME_SUFFIX = " Dev";
const BUNDLE_IDENTIFIER_SUFFIX = ".dev";
const LAUNCHER = ".clovy-dev-launch";
const LSREGISTER =
  "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister";
const IDENTIFIER = /^[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)+$/;

const defaultRepoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

export function devAppBundleIdentity({ appName, appIdentifier, baseIdentifier }) {
  if (
    !appName ||
    appName.includes("/") ||
    appName.includes(":") ||
    appName.includes("\n") ||
    appName.length > 80
  ) {
    throw new Error(`Invalid development app name: ${appName}`);
  }
  if (!IDENTIFIER.test(appIdentifier ?? "")) {
    throw new Error(`Invalid development app identifier: ${appIdentifier}`);
  }
  // Each compiled identity (branch or CLOVY_DEV_APP_IDENTIFIER) gets its own
  // bundle path, so an isolated proof never replaces the everyday dev bundle.
  const variant =
    appIdentifier === baseIdentifier
      ? ""
      : ` ${appIdentifier.startsWith(`${baseIdentifier}.`) ? appIdentifier.slice(baseIdentifier.length + 1) : appIdentifier}`;
  const displayName = `${appName}${BUNDLE_NAME_SUFFIX}${variant}`;
  return {
    displayName,
    bundleDirectoryName: `${displayName}.app`,
    // Never the release identifier: the dev bundle must not take over the
    // installed app's privacy grants, LaunchServices entry, or URL schemes.
    bundleIdentifier: `${appIdentifier}${BUNDLE_IDENTIFIER_SUFFIX}`,
  };
}

/**
 * Creates or reuses `<profileDir>/<App> Dev.app`. An unchanged Cargo output,
 * resource set, Info.plist, and signing identity reuse the signed bundle as is;
 * anything else replaces it at the same path, signed with the same identity.
 * `appIdentifier` is the compiled Tauri identifier (and data directory).
 */
export function prepareDevAppBundle({
  profileDir,
  appName,
  appIdentifier,
  signingIdentity,
  repoRoot = defaultRepoRoot,
  run = spawnSync,
}) {
  const tauriConfig = readJson(path.join(repoRoot, "src-tauri", "tauri.conf.json"));
  const baseIdentifier = tauriConfig.identifier;
  const identity = devAppBundleIdentity({ appName, appIdentifier, baseIdentifier });
  const bundlePath = path.join(profileDir, identity.bundleDirectoryName);
  const executable = path.join(bundlePath, "Contents", "MacOS", MAIN_EXECUTABLE);
  const infoPlist = devInfoPlist({ repoRoot, identity, run });
  const icon = path.join(repoRoot, "src-tauri", "icons", "icon.icns");
  // Tauri resolves resources from Contents/Resources once the app runs from a
  // bundle, so the dev bundle carries the resources tauri.conf.json declares
  // for every platform (private browser, agent skills) at their bundle
  // paths. Helpers stay out: debug builds resolve them from this checkout.
  const declaredResources = tauriConfig.bundle?.resources ?? {};
  if (Array.isArray(declaredResources)) {
    throw new Error("tauri.conf.json bundle.resources must map sources to bundle paths");
  }
  const resources = Object.entries(declaredResources).map(([source, target]) => [
    path.resolve(repoRoot, "src-tauri", source),
    target,
  ]);
  const sources = [MAIN_EXECUTABLE, ...SIBLING_EXECUTABLES].map((name) => {
    const source = path.join(profileDir, name);
    if (!existsSync(source)) throw new Error(`Cargo did not produce ${source}`);
    return [name, source];
  });

  const fingerprint = sha256(
    JSON.stringify({
      format: BUNDLE_FORMAT,
      signingIdentity,
      infoPlist,
      icon: sha256(readFileSync(icon)),
      sources: sources.map(([name, source]) => [name, fileIdentity(source)]),
      resources: resources.map(([source, target]) => [target, resourceIdentity(source)]),
    }),
  );
  const stampPath = path.join(profileDir, `.${identity.bundleDirectoryName}.stamp`);
  if (
    readText(stampPath) === fingerprint &&
    existsSync(executable) &&
    signatureSatisfies(bundlePath, signingIdentity, run)
  ) {
    return { ...identity, bundlePath, executable, reused: true };
  }

  const staging = path.join(profileDir, `.${identity.bundleDirectoryName}.staging`);
  rmSync(staging, { recursive: true, force: true });
  const stagedBundle = path.join(staging, identity.bundleDirectoryName);
  const contents = path.join(stagedBundle, "Contents");
  mkdirSync(path.join(contents, "MacOS"), { recursive: true });
  mkdirSync(path.join(contents, "Resources"), { recursive: true });
  writeFileSync(path.join(contents, "Info.plist"), infoPlist);
  copyFileSync(icon, path.join(contents, "Resources", "icon.icns"));
  for (const [source, target] of resources) {
    if (existsSync(source)) {
      cpSync(source, path.join(contents, "Resources", target), { recursive: true });
    }
  }
  for (const [name, source] of sources) {
    const destination = path.join(contents, "MacOS", name);
    copyFileSync(source, destination, constants.COPYFILE_FICLONE);
    chmodSync(destination, 0o755);
  }
  for (const name of SIBLING_EXECUTABLES) {
    codesign(path.join(contents, "MacOS", name), signingIdentity, run);
  }
  codesign(stagedBundle, signingIdentity, run);
  if (!signatureSatisfies(stagedBundle, signingIdentity, run)) {
    throw new Error(`The signed development app does not verify: ${stagedBundle}`);
  }

  const previous = path.join(staging, "previous.app");
  if (existsSync(bundlePath)) renameSync(bundlePath, previous);
  renameSync(stagedBundle, bundlePath);
  rmSync(staging, { recursive: true, force: true });
  writeFileSync(stampPath, fingerprint);
  // Lets System Settings resolve the bundle identifier to this name and icon.
  run(LSREGISTER, ["-f", bundlePath], { encoding: "utf8" });
  return { ...identity, bundlePath, executable, reused: false };
}

/** Builds the responsibility launcher (src-tauri/native/dev-launcher) once per source change. */
export function prepareDevLauncher({ profileDir, repoRoot = defaultRepoRoot, run = spawnSync }) {
  const source = path.join(repoRoot, "src-tauri", "native", "dev-launcher", "clovy-dev-launch.c");
  const launcher = path.join(profileDir, LAUNCHER);
  const stampPath = `${launcher}.stamp`;
  const digest = sha256(readFileSync(source));
  if (readText(stampPath) === digest && existsSync(launcher)) return launcher;

  const temporary = `${launcher}.tmp`;
  const result = run("xcrun", ["clang", "-O2", "-Wall", "-Werror", "-o", temporary, source], {
    encoding: "utf8",
  });
  if (result.status !== 0) {
    throw new Error(
      `Could not build the development launcher: ${`${result.stderr || result.stdout || ""}`.trim()}`,
    );
  }
  renameSync(temporary, launcher);
  writeFileSync(stampPath, digest);
  return launcher;
}

/**
 * The unbundled dev app kept its web storage under ~/Library/WebKit/<process
 * name>. A bundled app uses its bundle identifier instead, so copy that store
 * once (never overwrite, never delete the original) to keep onboarding,
 * session titles, and other local UI state.
 */
export function migrateDevWebKitStore({ home, legacyName, bundleIdentifier }) {
  const webkit = path.join(home, "Library", "WebKit");
  const legacy = path.join(webkit, legacyName);
  const current = path.join(webkit, bundleIdentifier);
  if (existsSync(current) || !existsSync(legacy)) return false;
  const temporary = `${current}.migrating`;
  rmSync(temporary, { recursive: true, force: true });
  cpSync(legacy, temporary, { recursive: true });
  renameSync(temporary, current);
  return true;
}

function devInfoPlist({ repoRoot, identity, run }) {
  const tauri = readJson(path.join(repoRoot, "src-tauri", "tauri.conf.json"));
  const macos = readJson(path.join(repoRoot, "src-tauri", "tauri.macos.conf.json"));
  const converted = run(
    "/usr/bin/plutil",
    ["-convert", "json", "-o", "-", path.join(repoRoot, "src-tauri", "Info.plist")],
    { encoding: "utf8" },
  );
  if (converted.status !== 0) {
    throw new Error(`Could not read src-tauri/Info.plist: ${`${converted.stderr || ""}`.trim()}`);
  }
  // Only the privacy usage strings carry over from the release Info.plist:
  // macOS refuses a protected resource to a responsible app without them.
  // Release URL schemes and the localized display name stay out.
  const usageDescriptions = Object.entries(JSON.parse(converted.stdout))
    .filter(([key, value]) => key.endsWith("UsageDescription") && typeof value === "string")
    .sort(([left], [right]) => left.localeCompare(right));
  const entries = [
    ["CFBundleDevelopmentRegion", "en"],
    ["CFBundleDisplayName", identity.displayName],
    ["CFBundleExecutable", MAIN_EXECUTABLE],
    ["CFBundleIconFile", "icon.icns"],
    ["CFBundleIdentifier", identity.bundleIdentifier],
    ["CFBundleInfoDictionaryVersion", "6.0"],
    ["CFBundleName", identity.displayName],
    ["CFBundlePackageType", "APPL"],
    ["CFBundleShortVersionString", tauri.version],
    ["CFBundleVersion", tauri.version],
    ["LSMinimumSystemVersion", macos.bundle?.macOS?.minimumSystemVersion ?? "14.0"],
    ["NSHighResolutionCapable", true],
    ...usageDescriptions,
  ];
  const body = entries
    .map(
      ([key, value]) =>
        `  <key>${escapeXml(key)}</key>\n  ${value === true ? "<true/>" : `<string>${escapeXml(value)}</string>`}`,
    )
    .join("\n");
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
${body}
</dict>
</plist>
`;
}

function codesign(target, signingIdentity, run) {
  const result = run(
    "/usr/bin/codesign",
    ["--force", "--sign", signingIdentity, "--timestamp=none", target],
    { encoding: "utf8" },
  );
  if (result.status !== 0) {
    throw new Error(
      `codesign could not sign ${target}: ${`${result.stderr || result.stdout || ""}`.trim()}`,
    );
  }
}

function fileIdentity(file) {
  const stats = statSync(file);
  return [stats.dev, stats.ino, stats.size, stats.mtimeMs];
}

function resourceIdentity(source) {
  if (!existsSync(source)) return null;
  return statSync(source).isDirectory() ? treeIdentity(source) : fileIdentity(source);
}

function treeIdentity(directory) {
  if (!existsSync(directory)) return null;
  const entries = [];
  const walk = (current) => {
    for (const entry of readdirSync(current, { withFileTypes: true }).sort((a, b) =>
      a.name.localeCompare(b.name),
    )) {
      const full = path.join(current, entry.name);
      if (entry.isDirectory()) walk(full);
      else entries.push([path.relative(directory, full), ...fileIdentity(full)]);
    }
  };
  walk(directory);
  return sha256(JSON.stringify(entries));
}

function escapeXml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function readJson(file) {
  return JSON.parse(readFileSync(file, "utf8"));
}

function readText(file) {
  try {
    return readFileSync(file, "utf8");
  } catch {
    return undefined;
  }
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function parseArguments(values) {
  let profileDir;
  for (let index = 0; index < values.length; index += 1) {
    if (values[index] === "--profile-dir" && values[index + 1]) {
      profileDir = path.resolve(values[index + 1]);
      index += 1;
    } else {
      throw new Error(`Unknown development app bundle argument: ${values[index]}`);
    }
  }
  if (!profileDir) throw new Error("--profile-dir is required");
  return { profileDir };
}

// Runner entry point: prints the launcher and the bundle executable, one per line.
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    const { profileDir } = parseArguments(process.argv.slice(2));
    const env = process.env;
    const appName = env.OS_CLOVY_DEV_APP_NAME ?? "Clovy";
    const appIdentifier =
      env.OS_CLOVY_DEV_APP_IDENTIFIER ??
      readJson(path.join(defaultRepoRoot, "src-tauri", "tauri.conf.json")).identifier;
    const signingIdentity = resolveDevSigningIdentity({ env }).identity;
    if (signingIdentity === AD_HOC_IDENTITY) {
      console.error(
        "Signing the development app ad hoc: macOS asks for every privacy permission again after each rebuild.",
      );
    }
    const bundle = prepareDevAppBundle({ profileDir, appName, appIdentifier, signingIdentity });
    const webStorageCopied = migrateDevWebKitStore({
      home: homedir(),
      legacyName: appName,
      bundleIdentifier: bundle.bundleIdentifier,
    });
    if (webStorageCopied) {
      console.error(`Copied the development web storage to ${bundle.bundleIdentifier}.`);
    }
    const launcher = prepareDevLauncher({ profileDir });
    console.error(
      `${bundle.reused ? "Reusing" : "Signed"} ${bundle.bundlePath} (${bundle.bundleIdentifier}).`,
    );
    process.stdout.write(`${launcher}\n${bundle.executable}\n`);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
