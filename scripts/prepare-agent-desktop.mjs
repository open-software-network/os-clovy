#!/usr/bin/env node
// Build-time preparation. The app never downloads an engine at runtime.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  allocateEngineBuildSource,
  sha256,
  verifyEngineArtifact,
} from "./agent-desktop-artifact.mjs";
import { resolveDevSigningIdentity, signatureSatisfies } from "./dev-signing-identity.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const pin = JSON.parse(readFileSync(join(root, "src-tauri", "agent-desktop-pin.json"), "utf8"));
// Snapshot mutable workspace inputs once, before any long build. Every output
// receipt and applied patch belongs to this same immutable input generation.
const sourceReview = readFileSync(join(root, "src-tauri", "agent-desktop", "source-review.md"));
const ownedRuntimeBytes = readFileSync(
  join(root, "src-tauri", "src", "computer_use_owned_runtime.rs"),
);
if (digest(sourceReview) !== pin.sourceReviewSha256)
  throw new Error("Source qualification digest mismatch");
if (
  typeof pin.ownedRuntimeSourceSha256 !== "string" ||
  !pin.ownedRuntimeSourceSha256 ||
  digest(ownedRuntimeBytes) !== pin.ownedRuntimeSourceSha256
)
  throw new Error("Owned browser runtime source digest mismatch");
let target = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
for (let i = 2; i < process.argv.length; i++) {
  const arg = process.argv[i];
  if (arg === "--" || arg === "--release") continue;
  if (arg === "--target" && process.argv[i + 1]) target = process.argv[++i];
  else if (arg.startsWith("--target=")) target = arg.slice(9);
  else throw new Error(`Unknown agent-desktop preparation argument: ${arg}`);
}
const supported = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "universal-apple-darwin",
  "x86_64-pc-windows-msvc",
  "aarch64-pc-windows-msvc",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
];
if (!supported.includes(target)) throw new Error(`Unsupported agent-desktop target: ${target}`);
if (target.includes("apple-darwin") && process.platform !== "darwin")
  throw new Error("macOS engine signing requires macOS");
if (
  target.includes("windows") &&
  (process.platform !== "win32" || !process.env.WINDOWS_CERTIFICATE_THUMBPRINT)
) {
  throw new Error(
    "Windows Computer use requires Windows and WINDOWS_CERTIFICATE_THUMBPRINT for same-publisher authenticated helpers",
  );
}
const patch = join(
  root,
  "src-tauri",
  "agent-desktop",
  "patches",
  "clovy-supervised-processes.patch",
);
const patchBytes = readFileSync(patch);
if (digest(patchBytes) !== pin.patchSha256) throw new Error("Audited engine patch digest mismatch");
const buildTargets =
  target === "universal-apple-darwin" ? ["aarch64-apple-darwin", "x86_64-apple-darwin"] : [target];
const output = join(root, ".tauri-helper", "agent-desktop", target);
let resolvedSigningIdentity;
function signingIdentity() {
  if (!resolvedSigningIdentity) {
    const configured =
      process.env.APPLE_SIGNING_IDENTITY || process.env.CLOVY_DEV_SIGNING_IDENTITY || "-";
    resolvedSigningIdentity = resolveDevSigningIdentity({
      env: { CLOVY_DEV_SIGNING_IDENTITY: configured },
    }).identity;
  }
  return resolvedSigningIdentity;
}
try {
  const manifest = verifyEngineArtifact(output, pin, target);
  const cachedExecutable = join(output, manifest.executable);
  verifySignature(cachedExecutable);
  if (target.includes("apple-darwin")) {
    const needsSigning = !signatureSatisfies(cachedExecutable, signingIdentity());
    const hashes = needsSigning ? undefined : signedCodeDirectoryHashes(cachedExecutable);
    if (needsSigning || JSON.stringify(manifest.codeDirectoryHashes) !== JSON.stringify(hashes)) {
      const resigned = mkdtempSync(join(dirname(output), `.${target}-resign-`));
      cpSync(output, resigned, { recursive: true });
      const resignedExecutable = join(resigned, manifest.executable);
      if (needsSigning) signExecutable(resignedExecutable);
      verifySignature(resignedExecutable);
      manifest.executableSha256 = sha256(resignedExecutable);
      manifest.codeDirectoryHashes = signedCodeDirectoryHashes(resignedExecutable);
      writeFileSync(
        join(resigned, "agent-desktop-manifest.json"),
        `${JSON.stringify(manifest, null, 2)}\n`,
      );
      verifyEngineArtifact(resigned, pin, target);
      renameSync(output, `${output}.previous-${Date.now()}`);
      renameSync(resigned, output);
    }
  }
  console.error(`Reusing verified agent-desktop ${pin.sourceCommit} for ${target}`);
  process.exit(0);
} catch {
  /* Missing, changed or differently signed artifacts are rebuilt. */
}

// A supplied immutable repository is read through git archive. Never patch it,
// checkout another revision in it, or modify its ignored build/evidence files.
const source = resolve(
  process.env.AGENT_DESKTOP_SOURCE || join(root, ".tauri-helper", "agent-desktop-source"),
);
if (!existsSync(source)) {
  mkdirSync(dirname(source), { recursive: true });
  run("git", ["-c", "core.autocrlf=false", "clone", "--no-checkout", pin.source, source], root);
}
const sourceGitDir = run("git", ["rev-parse", "--absolute-git-dir"], source, true).trim();
const commit = run("git", ["rev-parse", `${pin.sourceCommit}^{commit}`], source, true).trim();
if (commit !== pin.sourceCommit) throw new Error("Engine source commit mismatch");
const lock = run("git", ["show", `${commit}:Cargo.lock`], source, true);
if (sha256(join(root, "src-tauri", "agent-desktop", "Cargo.lock")) !== pin.cargoLockSha256)
  throw new Error("Vendored engine lock digest mismatch");
// Build from a fresh archive with only this reviewed patch. Do not reuse a
// writable checkout: a clean Git status does not authenticate ignored files.
const buildRoot = join(root, ".tauri-helper", "agent-desktop-build");
const actualSource = allocateEngineBuildSource(
  buildRoot,
  `${commit}-${pin.patchSha256}`,
  process.env.AGENT_DESKTOP_BUILD_DIR,
);
const archive = spawnSync("git", ["archive", "--format=tar", commit], {
  cwd: source,
  maxBuffer: 64 * 1024 * 1024,
});
if (archive.error || archive.status !== 0)
  throw archive.error || new Error("Could not archive immutable engine source");
const unpack = spawnSync("tar", ["-xf", "-", "-C", actualSource], {
  input: archive.stdout,
  stdio: ["pipe", "inherit", "inherit"],
});
if (unpack.error || unpack.status !== 0)
  throw unpack.error || new Error("Could not unpack engine source");
if (readFileSync(join(actualSource, "Cargo.lock"), "utf8") !== lock)
  throw new Error("Archived lock differs from pinned source");
if (
  sha256(join(actualSource, "Cargo.lock")) !== pin.cargoLockSha256 ||
  sha256(join(actualSource, "LICENSE")) !== pin.licenseSha256
)
  throw new Error("Pinned source lock/license digest mismatch");
const appliedPatch = join(actualSource, ".clovy-audited.patch");
writeFileSync(appliedPatch, patchBytes);
run(
  "git",
  ["--git-dir", sourceGitDir, "--work-tree", actualSource, "apply", "--check", appliedPatch],
  actualSource,
);
run(
  "git",
  ["--git-dir", sourceGitDir, "--work-tree", actualSource, "apply", appliedPatch],
  actualSource,
);
const stagedOwnedRuntime = join(actualSource, ".clovy-owned-runtime.rs");
writeFileSync(stagedOwnedRuntime, ownedRuntimeBytes, { flag: "wx" });
if (sha256(stagedOwnedRuntime) !== pin.ownedRuntimeSourceSha256)
  throw new Error("Staged owned browser runtime source digest mismatch");

const targetDir = resolve(
  process.env.AGENT_DESKTOP_TARGET_DIR ||
    process.env.CARGO_TARGET_DIR ||
    join(root, "src-tauri", "target", "agent-desktop"),
);
for (const triple of buildTargets) {
  run(
    process.env.CARGO || "cargo",
    ["build", "--locked", "--release", "--package", "agent-desktop", "--target", triple],
    actualSource,
    false,
    {
      ...process.env,
      CARGO_TARGET_DIR: targetDir,
      CARGO_INCREMENTAL: "0",
      CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP: "none",
    },
  );
}
const executableName = target.includes("windows") ? "agent-desktop.exe" : "agent-desktop";
mkdirSync(dirname(output), { recursive: true });
const preparedOutput = mkdtempSync(join(dirname(output), `.${target}-`));
const executable = join(preparedOutput, executableName);
if (buildTargets.length === 1)
  copyFileSync(join(targetDir, target, "release", executableName), executable);
else
  run(
    "/usr/bin/lipo",
    [
      "-create",
      ...buildTargets.map((triple) => join(targetDir, triple, "release", executableName)),
      "-output",
      executable,
    ],
    root,
  );
chmodSync(executable, 0o755);
signExecutable(executable);
verifySignature(executable);
copyFileSync(join(actualSource, "LICENSE"), join(preparedOutput, "LICENSE"));
writeFileSync(join(preparedOutput, "source-review.md"), sourceReview);
const manifest = {
  source: pin.source,
  sourceCommit: pin.sourceCommit,
  version: pin.version,
  protocolVersion: pin.protocolVersion,
  target,
  buildTargets,
  executable: executableName,
  executableSha256: sha256(executable),
  ...(target.includes("apple-darwin")
    ? { codeDirectoryHashes: signedCodeDirectoryHashes(executable) }
    : {}),
  cargoLockSha256: pin.cargoLockSha256,
  license: pin.license,
  licenseSha256: pin.licenseSha256,
  patchSha256: pin.patchSha256,
  ownedRuntimeSourceSha256: pin.ownedRuntimeSourceSha256,
  sourceTree: run("git", ["rev-parse", `${commit}^{tree}`], source, true).trim(),
  recentSourceReview: pin.recentSourceReview,
  sourceReviewSha256: pin.sourceReviewSha256,
};
writeFileSync(
  join(preparedOutput, "agent-desktop-manifest.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);
verifyEngineArtifact(preparedOutput, pin, target);
// Publish only a complete verified generation. Preserve any earlier output,
// including failed historical staging, instead of overwriting evidence.
if (existsSync(output)) renameSync(output, `${output}.previous-${Date.now()}`);
renameSync(preparedOutput, output);
console.error(
  `Prepared agent-desktop ${pin.sourceCommit} for ${target}: ${manifest.executableSha256}`,
);

function signedCodeDirectoryHashes(executable) {
  const hashes = {};
  for (const triple of buildTargets) {
    const architecture = triple.startsWith("aarch64-") ? "arm64" : "x86_64";
    const result = spawnSync(
      "/usr/bin/codesign",
      ["-d", "--verbose=4", "--arch", architecture, executable],
      { encoding: "utf8", shell: false },
    );
    if (result.error || result.status !== 0)
      throw result.error || new Error("Could not read signed engine CodeDirectory identity");
    const hash = `${result.stdout || ""}\n${result.stderr || ""}`.match(
      /^CDHash=([a-fA-F0-9]{40})$/m,
    )?.[1];
    if (!hash) throw new Error(`Signed engine has no ${architecture} CDHash`);
    hashes[architecture] = hash.toLowerCase();
  }
  return hashes;
}

function signExecutable(executable) {
  if (target.includes("apple-darwin")) {
    run(
      "/usr/bin/codesign",
      ["--force", "--sign", signingIdentity(), "--options", "runtime", executable],
      root,
    );
  } else if (target.includes("windows")) {
    run(
      process.env.WINDOWS_SIGNTOOL_PATH || "signtool",
      [
        "sign",
        "/sha1",
        process.env.WINDOWS_CERTIFICATE_THUMBPRINT,
        "/fd",
        "sha256",
        "/td",
        "sha256",
        "/tr",
        process.env.WINDOWS_SIGNING_TIMESTAMP_URL || "http://timestamp.digicert.com",
        executable,
      ],
      root,
    );
  }
}
function verifySignature(executable) {
  if (target.includes("apple-darwin"))
    run("/usr/bin/codesign", ["--verify", "--strict", executable], root);
  else if (target.includes("windows")) {
    run(process.env.WINDOWS_SIGNTOOL_PATH || "signtool", ["verify", "/pa", executable], root);
    const thumbprint = run(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-File",
        join(root, "scripts", "windows-authenticode.ps1"),
        "-TargetPath",
        executable,
        "-ExpectedThumbprint",
        process.env.WINDOWS_CERTIFICATE_THUMBPRINT,
      ],
      root,
      true,
    ).trim();
    if (thumbprint.toUpperCase() !== process.env.WINDOWS_CERTIFICATE_THUMBPRINT.toUpperCase())
      throw new Error("Cached engine signing publisher differs from current helper publisher");
  }
}
function run(command, argv, cwd, capture = false, env = process.env) {
  const result = spawnSync(command, argv, {
    cwd,
    env,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
    shell: false,
  });
  if (result.error || result.status !== 0)
    throw result.error || new Error(`${command} failed with exit ${result.status}`);
  return result.stdout || "";
}
function hostTriple() {
  const arch =
    process.arch === "arm64" ? "aarch64" : process.arch === "x64" ? "x86_64" : "unsupported";
  const os =
    process.platform === "darwin"
      ? "apple-darwin"
      : process.platform === "win32"
        ? "pc-windows-msvc"
        : process.platform === "linux"
          ? "unknown-linux-gnu"
          : "unsupported";
  return `${arch}-${os}`;
}

function digest(bytes) {
  return createHash("sha256").update(bytes).digest("hex");
}
