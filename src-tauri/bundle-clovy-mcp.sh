#!/usr/bin/env bash
# macOS beforeBundleCommand (tauri.macos.conf.json): builds the clovy-mcp [[bin]] (the Clovy
# MCP server's stdio relay, src/mcp_server/stdio.rs) into .tauri-helper/ so the
# bundle.resources mapping ships the real binary instead of the build.rs
# placeholder, then signs it (Developer ID when
# APPLE_SIGNING_IDENTITY is set, ad-hoc otherwise).
#
# Cargo reports the executable, including custom target directories. A universal
# build combines two freshly built real-triple binaries before signing.
set -euo pipefail

cd "$(dirname "$0")/.."

profile="release"
if [[ "${TAURI_ENV_DEBUG:-false}" == "true" ]]; then
  profile="debug"
fi

triple="${TAURI_ENV_TARGET_TRIPLE:-}"
mkdir -p .tauri-helper
out=".tauri-helper/clovy-mcp"

build_relay() {
  node --input-type=module - "$1" "$profile" "${2:-false}" <<'NODE'
import { spawnSync } from "node:child_process";
import { statSync } from "node:fs";
import { resolve } from "node:path";

// Match Tauri's Cargo working directory so relative environment paths and
// .cargo/config.toml apply equally to metadata and the relay build.
const cwd = resolve("src-tauri");
function step(command, args) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.error || result.status !== 0)
    throw result.error ?? new Error(`${command} failed with exit ${result.status}`);
  return result.stdout;
}
const host = step(process.env.RUSTC || "rustc", ["-vV"]).match(/^host: (.+)$/m)?.[1];
if (!host) throw new Error("rustc did not report its host target");
const target = process.argv[2] || process.env.CARGO_BUILD_TARGET || host;
const arch = { "aarch64-apple-darwin": "arm64", "x86_64-apple-darwin": "x86_64" }[target];
if (!arch) throw new Error(`Unsupported macOS MCP relay target: ${target}`);
const profile = process.argv[3];
// Tauri reports the native host even without --target. Keep Cargo's default
// layout/cache in that case; cross and universal slices need explicit targets.
const explicitTarget = target !== host || process.argv[4] === "true" ||
  (process.env.CARGO_BUILD_TARGET && process.env.CARGO_BUILD_TARGET !== target);
const metadata = JSON.parse(step("cargo", [
  "metadata", "--manifest-path", "Cargo.toml", "--locked", "--no-deps", "--format-version", "1",
]));
const pkg = metadata.packages.find((entry) => resolve(entry.manifest_path) === resolve(cwd, "Cargo.toml"));
if (!pkg) throw new Error("Cargo metadata did not report the Clovy package");
const output = step("cargo", [
  "build", "--manifest-path", "Cargo.toml", "--locked",
  "--profile", profile === "debug" ? "dev" : "release",
  ...(explicitTarget ? ["--target", target] : []),
  "--features", "helper-binaries", "--bin", "clovy-mcp", "--message-format=json-render-diagnostics",
]);
const events = output.split("\n").filter(Boolean).map((line) => JSON.parse(line));
const paths = new Set(events.filter((event) =>
  event.reason === "compiler-artifact" && event.package_id === pkg.id &&
  event.target?.name === "clovy-mcp" && event.target.kind?.includes("bin") &&
  event.profile?.test === false && event.executable,
).map((event) => resolve(cwd, event.executable)));
const targetDir = resolve(cwd, metadata.target_directory);
const expected = [resolve(targetDir, target, profile, "clovy-mcp")];
if (!explicitTarget) expected.push(resolve(targetDir, profile, "clovy-mcp"));
const [executable] = paths;
if (paths.size !== 1 || !expected.includes(executable) || !statSync(executable).isFile())
  throw new Error("Cargo did not report exactly the requested target/profile MCP relay");
// Reject placeholders, non-Mach-O files and a conflicting Cargo default target.
step("lipo", [executable, "-verify_arch", arch]);
process.stdout.write(executable);
NODE
}

if [[ "$triple" == "universal-apple-darwin" ]]; then
  arm="$(build_relay aarch64-apple-darwin true)"
  x86="$(build_relay x86_64-apple-darwin true)"
  lipo -create "$arm" "$x86" -output "$out"
  archs="$(lipo -archs "$out")"
  if [[ "$archs" != *arm64* || "$archs" != *x86_64* ]]; then
    echo "universal clovy-mcp has wrong architectures: $archs" >&2
    exit 1
  fi
  src="lipo($arm, $x86)"
else
  src="$(build_relay "$triple")"
  cp -f "$src" "$out"
fi

chmod +x "$out"

identity="${APPLE_SIGNING_IDENTITY:-}"
if [[ -n "${identity// /}" ]]; then
  codesign --force --entitlements src-tauri/Entitlements.plist \
    --sign "$identity" --timestamp --options runtime "$out"
else
  codesign --force --entitlements src-tauri/Entitlements.plist \
    --sign - "$out"
fi

echo "bundled clovy-mcp from $src"
