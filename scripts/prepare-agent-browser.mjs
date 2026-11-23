#!/usr/bin/env node
// Build-time only. Runtime never downloads a browser or searches the user's PATH.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  closeSync,
  createReadStream,
  createWriteStream,
  cpSync,
  existsSync,
  mkdirSync,
  openSync,
  readFileSync,
  readlinkSync,
  readSync,
  readdirSync,
  renameSync,
  rmSync,
  statfsSync,
  statSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { Readable } from "node:stream";
import { pipeline } from "node:stream/promises";
import { fileURLToPath } from "node:url";
import { acquirePreparationLock } from "./agent-browser/preparation-lock.mjs";
import { windowsSigningIdentity } from "./agent-browser/windows-signing.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const definition = path.join(root, "scripts/agent-browser");
const manifest = JSON.parse(readFileSync(path.join(definition, "manifest.json"), "utf8"));
const args = process.argv.slice(2).filter((arg) => arg !== "--");
const valueOptions = new Set(["--target", "--platform", "--resource-root"]);
for (let index = 0; index < args.length; index += 1) {
  if (valueOptions.has(args[index])) {
    if (!args[index + 1] || args[index + 1].startsWith("--"))
      throw new Error(`${args[index]} requires a value.`);
    index += 1;
  } else if (!["--release", "--check"].includes(args[index]))
    throw new Error(`Unknown browser preparation argument: ${args[index]}`);
}
const release = args.includes("--release");
const check = args.includes("--check");
const option = (key) => (args.includes(key) ? args[args.indexOf(key) + 1] : undefined);
const target = option("--target") || process.env.TAURI_ENV_TARGET_TRIPLE;
const host = {
  "linux:x64": "linux64",
  "darwin:arm64": "mac-arm64",
  "darwin:x64": "mac-x64",
  "win32:x64": "win64",
}[`${process.platform}:${process.arch}`];
const platforms =
  target === "universal-apple-darwin"
    ? ["mac-arm64", "mac-x64"]
    : [
        option("--platform") ||
          Object.keys(manifest.chrome.platforms).find(
            (key) => manifest.chrome.platforms[key].target === target,
          ) ||
          host,
      ];
if (!platforms.every((key) => manifest.chrome.platforms[key]))
  throw new Error("Unsupported browser target; use a pinned native platform.");
if (
  target &&
  target !== "universal-apple-darwin" &&
  !Object.values(manifest.chrome.platforms).some((pin) => pin.target === target)
) {
  throw new Error(`No pinned browser resource exists for target ${target}.`);
}
if (
  option("--platform") &&
  target &&
  (target === "universal-apple-darwin" ||
    manifest.chrome.platforms[option("--platform")].target !== target)
) {
  throw new Error("--platform conflicts with the selected target.");
}
const cache = path.join(root, ".tauri-agent-browser");
if (option("--resource-root") && !check)
  throw new Error("--resource-root is only supported with --check.");
const output = option("--resource-root")
  ? path.resolve(option("--resource-root"))
  : path.join(root, "resources/agent-browser");
const buildTarget = process.env.CLOVY_AGENT_BROWSER_TARGET_DIR || path.join(cache, "target");
const profile = release ? "release" : "debug";
const hashBytes = (bytes) => createHash("sha256").update(bytes).digest("hex");
const hashFile = async (file) => {
  const hash = createHash("sha256");
  for await (const bytes of createReadStream(file)) hash.update(bytes);
  return hash.digest("hex");
};
const run = (command, argv, options = {}) => {
  const result = spawnSync(command, argv, {
    cwd: root,
    encoding: "utf8",
    stdio: options.input ? ["pipe", "inherit", "inherit"] : "inherit",
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} failed (${result.status})`);
  return result.stdout?.trim();
};
const inputFiles = [
  ...manifest.source.patches,
  ...manifest.source.hostFiles,
  ...manifest.source.preparationFiles,
];
const inputBytes = new Map(
  inputFiles.map((file) => [file, readFileSync(path.join(definition, file))]),
);
const inputs = Object.fromEntries(
  inputFiles.map((file) => [file, hashBytes(inputBytes.get(file))]),
);
const preparerSha256 = hashBytes(readFileSync(fileURLToPath(import.meta.url)));
const preparationId = hashBytes(JSON.stringify({ manifest, inputs, profile, preparerSha256 }));

// Heavy preparation uses the same ordered locks as the other local build work.
if (!check && process.platform === "linux" && !process.env.CLOVY_BROWSER_BUILD_LOCKED) {
  mkdirSync(path.join(os.homedir(), ".cache"), { recursive: true });
  run(
    "flock",
    [
      "/tmp/macro-heavy.lock",
      "flock",
      path.join(os.homedir(), ".cache/clovy-melhorias-20261101-build.lock"),
      process.execPath,
      fileURLToPath(import.meta.url),
      ...args,
    ],
    { env: { ...process.env, CLOVY_BROWSER_BUILD_LOCKED: "1" } },
  );
  process.exit(0);
}

const releasePreparationLock = await acquirePreparationLock(cache);
try {
  if (check && existsSync(output)) {
    for (const entry of readdirSync(output, { withFileTypes: true })) {
      if (!entry.isDirectory() || !manifest.chrome.platforms[entry.name])
        throw new Error(`Unexpected browser resource root entry: ${entry.name}`);
    }
  }
  for (const platform of platforms) {
    const pin = manifest.chrome.platforms[platform];
    const destination = path.join(output, platform);
    const provenanceFile = path.join(destination, "provenance.json");
    let previous;
    try {
      previous = JSON.parse(readFileSync(provenanceFile, "utf8"));
    } catch {
      /* unprepared */
    }
    const filesMatch =
      previous?.preparationId === preparationId &&
      previous.platform === platform &&
      previous.profile === profile &&
      previous.protocol === manifest.protocol &&
      previous.source?.commit === manifest.source.commit &&
      previous.chrome?.version === manifest.chrome.version &&
      previous.chrome?.sha256 === pin.sha256 &&
      previous.files?.[pin.engine] &&
      previous.files?.[pin.executable] &&
      (await verifyInventory(destination, previous.files));
    if (check) {
      if (!filesMatch)
        throw new Error(
          `Pinned browser resources are missing or modified for ${platform}. Run pnpm browser:prepare.`,
        );
      console.error(`Verified pinned browser resources: ${platform}`);
      continue;
    }
    const identity = signingIdentity();
    if (filesMatch && previous.signingIdentity === identity) {
      if (process.platform === "win32" && identity !== null)
        verifyWindowsSignature(path.join(destination, pin.engine), identity);
      console.error(`Reusing pinned browser resources: ${platform}`);
      continue;
    }
    if (Number(statfsSync(root).bavail) * Number(statfsSync(root).bsize) < 20 * 1024 ** 3)
      throw new Error("Browser preparation needs at least 20 GiB free disk space.");
    const nativeOS = platform.startsWith("mac-")
      ? "darwin"
      : platform === "win64"
        ? "win32"
        : "linux";
    if (process.platform !== nativeOS)
      throw new Error(
        `Prepare ${platform} on its native OS; cross-OS helper builds are unsupported.`,
      );
    mkdirSync(cache, { recursive: true });
    const archive = path.join(cache, `chrome-${manifest.chrome.version}-${platform}.zip`);
    if (!existsSync(archive) || (await hashFile(archive)) !== pin.sha256) {
      rmSync(archive, { force: true });
      const response = await fetch(pin.url, { redirect: "error" });
      if (!response.ok || !response.body)
        throw new Error(`Chrome archive download failed: ${response.status}`);
      rmSync(`${archive}.partial`, { force: true });
      await pipeline(
        Readable.fromWeb(response.body),
        createWriteStream(`${archive}.partial`, { flags: "wx" }),
      );
      if ((await hashFile(`${archive}.partial`)) !== pin.sha256)
        throw new Error("Chrome archive digest mismatch; refusing preparation.");
      renameSync(`${archive}.partial`, archive);
    }
    const source = await prepareSource();
    const cargoManifest = path.join(source, "cli/Cargo.toml");
    const rustcDetails = run("rustc", ["--version", "--verbose"], {
      stdio: ["ignore", "pipe", "inherit"],
    });
    const rustHost = rustcDetails.match(/^host: (.+)$/m)?.[1];
    if (process.env.CARGO_BUILD_TARGET && process.env.CARGO_BUILD_TARGET !== pin.target)
      throw new Error("CARGO_BUILD_TARGET conflicts with the pinned browser target.");
    const nativeTarget = pin.target === rustHost && !process.env.CARGO_BUILD_TARGET;
    if (Number(statfsSync(root).bavail) * Number(statfsSync(root).bsize) < 20 * 1024 ** 3)
      throw new Error("Browser compilation needs at least 20 GiB free disk space.");
    run(
      process.env.CARGO || "cargo",
      [
        "build",
        "--manifest-path",
        cargoManifest,
        "--locked",
        "--features",
        "clovy-private",
        "--bin",
        "clovy-agent-browser",
        ...(nativeTarget ? [] : ["--target", pin.target]),
        ...(release ? ["--release"] : []),
      ],
      {
        env: {
          ...process.env,
          CARGO_TARGET_DIR: buildTarget,
          CARGO_INCREMENTAL: "0",
          CARGO_PROFILE_DEV_DEBUG: "0",
          CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP: "none",
          CARGO_PROFILE_DEV_BUILD_OVERRIDE_STRIP: "none",
        },
      },
    );
    const metadata = JSON.parse(
      run(
        process.env.CARGO || "cargo",
        [
          "metadata",
          "--manifest-path",
          cargoManifest,
          "--locked",
          "--format-version",
          "1",
          "--filter-platform",
          pin.target,
        ],
        { stdio: ["ignore", "pipe", "inherit"], maxBuffer: 32 * 1024 * 1024 },
      ),
    );
    const stage = `${destination}.preparing`;
    if (Number(statfsSync(root).bavail) * Number(statfsSync(root).bsize) < 20 * 1024 ** 3)
      throw new Error("Chrome extraction needs at least 20 GiB free disk space.");
    rmSync(stage, { recursive: true, force: true });
    mkdirSync(path.join(stage, "chrome"), { recursive: true });
    extract(archive, path.join(stage, "chrome"), pin.archiveRoot);
    if (!existsSync(path.join(stage, pin.executable)))
      throw new Error("Chrome archive lacks the pinned executable.");
    cpSync(
      path.join(buildTarget, ...(nativeTarget ? [] : [pin.target]), profile, pin.engine),
      path.join(stage, pin.engine),
    );
    mkdirSync(path.join(stage, "licenses"), { recursive: true });
    cpSync(path.join(source, "LICENSE"), path.join(stage, "licenses/agent-browser-LICENSE"));
    cpSync(
      path.join(stage, "chrome", pin.archiveRoot, "ABOUT"),
      path.join(stage, "licenses/chrome-ABOUT"),
    );
    writeFileSync(
      path.join(stage, "sbom.spdx.json"),
      `${JSON.stringify(sbom(metadata, platform, pin), null, 2)}\n`,
    );
    if (process.platform === "darwin") signMac(stage);
    if (process.platform === "win32" && identity !== null) {
      run("powershell.exe", [
        "-NoProfile",
        "-File",
        path.join(root, "scripts/windows-sign.ps1"),
        path.join(stage, pin.engine),
      ]);
      verifyWindowsSignature(path.join(stage, pin.engine), identity);
    }
    const provenance = {
      schemaVersion: 1,
      protocol: manifest.protocol,
      preparationId,
      preparerSha256,
      source: {
        ...manifest.source,
        inputSha256: inputs,
        cargoLockSha256: await hashFile(path.join(source, "cli/Cargo.lock")),
      },
      chrome: { version: manifest.chrome.version, revision: manifest.chrome.revision, ...pin },
      platform,
      profile,
      rustc: rustcDetails,
      signingIdentity: identity,
      files: await inventory(stage),
    };
    writeFileSync(path.join(stage, "provenance.json"), `${JSON.stringify(provenance, null, 2)}\n`);
    mkdirSync(output, { recursive: true });
    rmSync(destination, { recursive: true, force: true });
    renameSync(stage, destination);
    console.error(
      `Prepared private browser ${platform}: ${manifest.source.commit}, Chrome ${manifest.chrome.version}`,
    );
  }
} finally {
  releasePreparationLock();
}

async function prepareSource() {
  const objects = path.join(cache, "upstream.git");
  if (!existsSync(objects)) run("git", ["init", "--bare", objects]);
  const hasCommit =
    spawnSync(
      "git",
      ["--git-dir", objects, "cat-file", "-e", `${manifest.source.commit}^{commit}`],
      { stdio: "ignore" },
    ).status === 0;
  if (!hasCommit)
    run("git", [
      "--git-dir",
      objects,
      "fetch",
      "--depth=1",
      manifest.source.repository,
      manifest.source.commit,
    ]);
  const actual = run(
    "git",
    ["--git-dir", objects, "rev-parse", `${manifest.source.commit}^{commit}`],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  if (actual !== manifest.source.commit) throw new Error("Upstream commit mismatch.");
  run("git", [
    "--git-dir",
    objects,
    "update-ref",
    "refs/heads/clovy-pinned",
    manifest.source.commit,
  ]);
  const source = path.join(cache, `source-${preparationId}`);
  rmSync(source, { recursive: true, force: true });
  run("git", ["clone", "--shared", "--no-checkout", "--branch", "clovy-pinned", objects, source]);
  run("git", ["checkout", "--detach", manifest.source.commit], { cwd: source });
  for (const patch of manifest.source.patches) {
    run("git", ["apply", "--unidiff-zero", "--check", "-"], {
      cwd: source,
      input: inputBytes.get(patch),
    });
    run("git", ["apply", "--unidiff-zero", "-"], { cwd: source, input: inputBytes.get(patch) });
  }
  for (const file of manifest.source.hostFiles)
    writeFileSync(path.join(source, "cli/src", path.basename(file)), inputBytes.get(file));
  return source;
}

function extract(archive, destination, archiveRoot) {
  if (process.platform === "win32") {
    // Archive bytes are authenticated above; PowerShell preserves the Windows tree.
    const quote = (value) => `'${value.replaceAll("'", "''")}'`;
    run("powershell.exe", [
      "-NoProfile",
      "-Command",
      `Expand-Archive -LiteralPath ${quote(archive)} -DestinationPath ${quote(destination)}`,
    ]);
  } else {
    const members = run("unzip", ["-Z1", archive], {
      stdio: ["ignore", "pipe", "inherit"],
      maxBuffer: 16 * 1024 * 1024,
    }).split("\n");
    if (
      members.some(
        (member) => !member.startsWith(`${archiveRoot}/`) || member.split("/").includes(".."),
      )
    )
      throw new Error("Invalid Chrome archive paths.");
    run("unzip", ["-q", archive, "-d", destination]);
  }
}

function signingIdentity() {
  if (process.platform === "win32") return windowsSigningIdentity(process.env);
  if (process.platform !== "darwin") return null;
  return (
    (release
      ? process.env.APPLE_SIGNING_IDENTITY
      : process.env.CLOVY_DEV_SIGNING_IDENTITY
    )?.trim() || "-"
  );
}
function verifyWindowsSignature(file, identity) {
  const thumbprint = identity.startsWith("store-sha1:") ? identity.slice("store-sha1:".length) : "";
  run(
    "powershell.exe",
    [
      "-NoProfile",
      "-NonInteractive",
      "-Command",
      '$s = Get-AuthenticodeSignature -LiteralPath $env:CLOVY_BROWSER_SIGNATURE_FILE; if ($s.Status -ne "Valid" -or ($env:CLOVY_BROWSER_SIGNATURE_THUMBPRINT -and $s.SignerCertificate.Thumbprint -ne $env:CLOVY_BROWSER_SIGNATURE_THUMBPRINT)) { throw "Browser signature verification failed" }',
    ],
    {
      env: {
        ...process.env,
        CLOVY_BROWSER_SIGNATURE_FILE: file,
        CLOVY_BROWSER_SIGNATURE_THUMBPRINT: thumbprint,
      },
    },
  );
}
function signMac(directory) {
  const identity = signingIdentity();
  const targets = [];
  const visit = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        visit(file);
        if (/\.(app|framework|xpc)$/.test(entry.name)) targets.push(file);
      } else {
        const header = Buffer.alloc(4);
        const fd = openSync(file, "r");
        readSync(fd, header, 0, 4, 0);
        closeSync(fd);
        const magic = header.toString("hex");
        if (["feedfacf", "cffaedfe", "cafebabe", "bebafeca"].includes(magic)) targets.push(file);
      }
    }
  };
  visit(directory);
  // Sign inside-out: a bundle's main executable seals its nested code, so
  // deeper items (e.g. framework helper apps that ship unsigned in some
  // Chrome for Testing builds) must be signed before the binaries and
  // bundles that contain them. At equal depth, bundles go before files.
  const depth = (file) => file.split(path.sep).length;
  const isBundle = (file) => /\.(app|framework|xpc)$/.test(file);
  targets.sort((a, b) => depth(b) - depth(a) || Number(isBundle(b)) - Number(isBundle(a)));
  for (const file of targets)
    run("/usr/bin/codesign", [
      "--force",
      "--sign",
      identity,
      "--preserve-metadata=entitlements",
      ...(identity === "-" ? ["--timestamp=none"] : ["--timestamp", "--options", "runtime"]),
      file,
    ]);
  for (const file of targets) run("/usr/bin/codesign", ["--verify", "--strict", file]);
}

async function inventory(directory) {
  const files = {};
  const visit = async (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      const relative = path.relative(directory, file).split(path.sep).join("/");
      if (relative === "provenance.json") continue;
      if (entry.isSymbolicLink()) {
        const target = readlinkSync(file);
        const resolved = path.relative(directory, path.resolve(path.dirname(file), target));
        if (resolved.startsWith("..") || path.isAbsolute(resolved))
          throw new Error("Resource symlink leaves the browser tree.");
        files[relative] = { symlink: target };
      } else if (entry.isDirectory()) await visit(file);
      else files[relative] = { sha256: await hashFile(file), mode: statSync(file).mode & 0o777 };
    }
  };
  await visit(directory);
  return Object.fromEntries(Object.entries(files).sort(([a], [b]) => a.localeCompare(b)));
}
async function verifyInventory(directory, files) {
  if (!files || Object.keys(files).length < 5) return false;
  try {
    return JSON.stringify(await inventory(directory)) === JSON.stringify(files);
  } catch {
    return false;
  }
}
function sbom(metadata, platform, pin) {
  const packages = metadata.packages.map((pkg, index) => ({
    SPDXID: `SPDXRef-Crate-${index}`,
    name: pkg.name,
    versionInfo: pkg.version,
    downloadLocation: pkg.source?.startsWith("registry+")
      ? `https://crates.io/api/v1/crates/${pkg.name}/${pkg.version}/download`
      : manifest.source.repository,
    filesAnalyzed: false,
    licenseConcluded: "NOASSERTION",
    licenseDeclared: pkg.license || "NOASSERTION",
    copyrightText: "NOASSERTION",
  }));
  packages.push({
    SPDXID: "SPDXRef-Chrome",
    name: "Google Chrome for Testing",
    versionInfo: manifest.chrome.version,
    downloadLocation: pin.url,
    filesAnalyzed: false,
    licenseConcluded: "NOASSERTION",
    licenseDeclared: "NOASSERTION",
    copyrightText: "NOASSERTION",
    checksums: [{ algorithm: "SHA256", checksumValue: pin.sha256 }],
  });
  return {
    spdxVersion: "SPDX-2.3",
    dataLicense: "CC0-1.0",
    SPDXID: "SPDXRef-DOCUMENT",
    name: `Clovy private browser ${platform}`,
    documentNamespace: `https://clovy.invalid/sbom/agent-browser/${preparationId}/${platform}`,
    creationInfo: {
      created: new Date().toISOString(),
      creators: ["Tool: Clovy prepare-agent-browser"],
    },
    documentDescribes: packages.map((pkg) => pkg.SPDXID),
    packages,
  };
}
