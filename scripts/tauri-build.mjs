#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const platformBundles = {
  darwin: ["app", "dmg"],
  win32: ["nsis"],
};

const platformConfigs = {
  darwin: "src-tauri/tauri.macos.conf.json",
  win32: "src-tauri/tauri.windows.conf.json",
};

const rawUserArgs = process.argv.slice(2);
const userArgs = rawUserArgs[0] === "--" ? rawUserArgs.slice(1) : rawUserArgs;
const target = optionValue(userArgs, "--target");
const buildPlatform = platformForTarget(target) ?? process.platform;
const browserPrepareArgs = [
  resolve(dirname(fileURLToPath(import.meta.url)), "prepare-agent-browser.mjs"),
  "--release",
];
if (target) browserPrepareArgs.push("--target", target);
const browserPrepare = spawnSync(process.execPath, browserPrepareArgs, { stdio: "inherit" });
if (browserPrepare.status !== 0) process.exit(browserPrepare.status ?? 1);
if (buildPlatform === "darwin") {
  const localPrepare = spawnSync(
    process.execPath,
    [
      resolve(dirname(fileURLToPath(import.meta.url)), "prepare-local-transcriber.mjs"),
      ...(userArgs.includes("--debug") ? [] : ["--release"]),
    ],
    { stdio: "inherit" },
  );
  if (localPrepare.status !== 0) process.exit(localPrepare.status ?? 1);
}
if (buildPlatform === "darwin" || buildPlatform === "win32") {
  const prepareArgs = [
    resolve(dirname(fileURLToPath(import.meta.url)), "prepare-computer-use.mjs"),
    ...(userArgs.includes("--debug") ? [] : ["--release"]),
  ];
  if (target) prepareArgs.push("--target", target);
  const prepare = spawnSync(process.execPath, prepareArgs, {
    stdio: "inherit",
  });
  if (prepare.status !== 0) process.exit(prepare.status ?? 1);
}
const bundles = platformBundles[buildPlatform];
if (buildPlatform === "win32") {
  const prepareArgs = [resolve(dirname(fileURLToPath(import.meta.url)), "prepare-windows-mcp.mjs")];
  for (const option of ["--target", "--profile", "--features"]) {
    const value = optionValue(userArgs, option);
    if (value) prepareArgs.push(option, value);
  }
  if (userArgs.includes("--debug")) prepareArgs.push("--debug");
  const prepared = spawnSync(process.execPath, prepareArgs, { stdio: "inherit" });
  if (prepared.status !== 0) process.exit(prepared.status ?? 1);
}
const config = platformConfigs[buildPlatform];
const hasBundleOverride = userArgs.some(
  (arg) => arg === "--bundles" || arg.startsWith("--bundles="),
);
const hasConfigOverride = userArgs.some((arg) => arg === "--config" || arg.startsWith("--config="));
const args = ["build"];
if (config && !hasConfigOverride) {
  args.push("--config", config);
}
if (bundles && !hasBundleOverride) {
  args.push("--bundles", bundles.join(","));
}
args.push(...userArgs);
// Trailing args after `--` go to the cargo runner. `--locked` keeps release
// builds from re-resolving dependencies past Cargo.lock, so a stale lockfile
// fails the build instead of silently pulling crates that never saw the
// release-age cooldown (spec/package-install-security.md).
if (!userArgs.includes("--")) {
  args.push("--");
}
args.push("--locked");

const tauri = tauriInvocation();
const child = spawn(tauri.command, [...tauri.args, ...args], {
  shell: false,
  stdio: "inherit",
});

child.on("exit", (code, signal) => {
  process.exit(code ?? (signal ? 1 : 0));
});

child.on("error", (error) => {
  console.error(error);
  process.exit(1);
});

function optionValue(args, option) {
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === option) {
      return args[index + 1];
    }
    if (arg.startsWith(`${option}=`)) {
      return arg.slice(option.length + 1);
    }
  }
  return undefined;
}

function platformForTarget(targetTriple) {
  if (!targetTriple) {
    return undefined;
  }
  if (targetTriple.includes("windows")) {
    return "win32";
  }
  if (targetTriple.includes("apple-darwin")) {
    return "darwin";
  }
  return undefined;
}

function tauriInvocation() {
  const scriptDir = dirname(fileURLToPath(import.meta.url));
  if (process.platform === "win32") {
    const localScript = resolve(scriptDir, "..", "node_modules", "@tauri-apps", "cli", "tauri.js");
    if (!existsSync(localScript)) {
      throw new Error(
        `Tauri CLI entry point not found at "${localScript}". Run pnpm install first.`,
      );
    }
    return { command: process.execPath, args: [localScript] };
  }

  const localBinary = resolve(scriptDir, "..", "node_modules", ".bin", "tauri");
  return { command: existsSync(localBinary) ? localBinary : "tauri", args: [] };
}
