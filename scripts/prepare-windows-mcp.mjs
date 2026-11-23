#!/usr/bin/env node
// Stage the Cargo-reported relay, never a guessed target/release executable.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const sha256 = (file) => createHash("sha256").update(readFileSync(file)).digest("hex");
export function sourceDigest(repo) {
  const files = [
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/build.rs",
    "src-tauri/tauri.conf.json",
    "src-tauri/tauri.windows.conf.json",
    "scripts/prepare-windows-mcp.mjs",
  ];
  const walk = (dir) => {
    for (const entry of readdirSync(join(repo, dir), { withFileTypes: true })) {
      const file = `${dir}/${entry.name}`;
      if (entry.isDirectory()) walk(file);
      else if (entry.isFile() && file.endsWith(".rs")) files.push(file);
    }
  };
  walk("src-tauri/src");
  const hash = createHash("sha256");
  for (const file of files.sort())
    hash
      .update(file)
      .update("\0")
      .update(readFileSync(join(repo, file)))
      .update("\0");
  return hash.digest("hex");
}
export function relayArtifact(events, targetDirectory, target, profile) {
  const directory = { dev: "debug", test: "debug", bench: "release" }[profile] ?? profile;
  const expected = resolve(targetDirectory, target, directory, "clovy-mcp.exe");
  const paths = new Set(
    events
      .filter(
        (event) =>
          event.reason === "compiler-artifact" &&
          event.target?.name === "clovy-mcp" &&
          event.target.kind?.includes("bin") &&
          event.executable,
      )
      .map((event) => resolve(event.executable)),
  );
  if (paths.size !== 1 || !paths.has(expected))
    throw new Error("Cargo did not report exactly the requested target/profile MCP relay");
  return expected;
}
export function verifyRelayManifest(manifest, { repo, executable, target, profile, publisher }) {
  if (
    manifest.schema !== 1 ||
    manifest.target !== target ||
    manifest.profile !== profile ||
    manifest.executable !== "clovy-mcp.exe" ||
    manifest.publisher !== publisher ||
    manifest.sourceSha256 !== sourceDigest(repo) ||
    manifest.executableSha256 !== sha256(executable) ||
    statSync(executable).size < 2 ||
    readFileSync(executable).subarray(0, 2).toString() !== "MZ"
  )
    throw new Error(
      "MCP relay bytes/provenance do not match this source, target, profile and publisher",
    );
}
function step(command, args, capture = false) {
  const result = spawnSync(command, args, {
    cwd: root,
    shell: false,
    encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error || result.status !== 0)
    throw result.error ?? new Error(`${command} failed with exit ${result.status}`);
  return result.stdout;
}
function option(args, name) {
  const inline = args.find((arg) => arg.startsWith(`${name}=`));
  const index = args.indexOf(name);
  return inline ? inline.slice(name.length + 1) : index < 0 ? undefined : args[index + 1];
}
export function main(args) {
  if (process.platform !== "win32")
    throw new Error(
      "MCP relay packaging requires native Windows and the configured signing publisher",
    );
  const target =
    option(args, "--target") ??
    process.env.CARGO_BUILD_TARGET ??
    step("rustc", ["-vV"], true).match(/^host: (.+)$/m)?.[1];
  const profile = option(args, "--profile") ?? (args.includes("--debug") ? "dev" : "release");
  const publisher = process.env.WINDOWS_CERTIFICATE_THUMBPRINT?.trim().toUpperCase();
  if (!publisher || !target?.includes("windows") || !/^[a-zA-Z0-9_-]+$/.test(profile))
    throw new Error("A Windows target/profile and WINDOWS_CERTIFICATE_THUMBPRINT are required");
  const stage = join(root, ".tauri-helper", "mcp-windows");
  const executable = option(args, "--executable") ?? join(stage, "clovy-mcp.exe");
  const manifestPath = option(args, "--manifest") ?? join(stage, "clovy-mcp-manifest.json");
  if (args.includes("--check")) {
    verifyRelayManifest(JSON.parse(readFileSync(manifestPath, "utf8")), {
      repo: root,
      executable,
      target,
      profile,
      publisher,
    });
    return;
  }
  mkdirSync(stage, { recursive: true }); // Resource directory only; never fake exe.
  for (const entry of readdirSync(stage, { withFileTypes: true })) {
    if (!entry.isFile() || !["clovy-mcp.exe", "clovy-mcp-manifest.json"].includes(entry.name))
      throw new Error(`Unexpected MCP relay staging entry: ${entry.name}`);
  }
  const sourceSha256 = sourceDigest(root);
  const metadata = JSON.parse(
    step(
      "cargo",
      [
        "metadata",
        "--manifest-path",
        "src-tauri/Cargo.toml",
        "--locked",
        "--no-deps",
        "--format-version",
        "1",
      ],
      true,
    ),
  );
  const cargoArgs = [
    "build",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--locked",
    "--target",
    target,
    "--profile",
    profile,
    "--features",
    "helper-binaries",
    "--bin",
    "clovy-mcp",
    "--message-format=json-render-diagnostics",
  ];
  const features = option(args, "--features");
  if (features) cargoArgs.push("--features", features);
  const output = step("cargo", cargoArgs, true);
  const events = output
    .trim()
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  for (const event of events)
    if (event.reason === "compiler-message" && event.message?.rendered)
      process.stderr.write(event.message.rendered);
  const artifact = relayArtifact(events, metadata.target_directory, target, profile);
  const powershell = (script, extra = []) =>
    step("powershell.exe", [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      join(root, "scripts", script),
      "-TargetPath",
      executable,
      ...extra,
    ]);
  copyFileSync(artifact, executable);
  powershell("windows-sign.ps1");
  powershell("windows-authenticode.ps1", ["-ExpectedThumbprint", publisher]);
  if (sourceSha256 !== sourceDigest(root))
    throw new Error("Sources changed while the MCP relay was built");
  writeFileSync(
    manifestPath,
    `${JSON.stringify({ schema: 1, executable: "clovy-mcp.exe", target, profile, publisher, sourceSha256, executableSha256: sha256(executable) }, null, 2)}\n`,
  );
}
if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
