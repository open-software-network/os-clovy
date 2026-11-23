#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

if (!["darwin", "win32"].includes(process.platform)) {
  throw new Error(
    "Clovy Computer use packaging requires macOS or Windows; engine-only Linux proof uses prepare-agent-desktop.mjs",
  );
}
const scripts = dirname(fileURLToPath(import.meta.url));
const rawArgs = process.argv.slice(2);
const buildHook = rawArgs.includes("--build-hook");
const args = rawArgs.filter((arg) => arg !== "--build-hook");
if (buildHook && process.env.TAURI_ENV_DEBUG !== "true") args.push("--release");
for (const name of ["prepare-agent-desktop.mjs", "prepare-computer-use-helper.mjs"]) {
  const result = spawnSync(process.execPath, [resolve(scripts, name), ...args], {
    stdio: "inherit",
    shell: false,
  });
  if (result.error || result.status !== 0)
    throw result.error || new Error(`${name} failed with exit ${result.status}`);
}
