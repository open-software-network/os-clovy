#!/usr/bin/env node

import { existsSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";

const appPath = resolve(process.argv[2] ?? "");

if (!process.argv[2] || !existsSync(appPath)) {
  throw new Error("Usage: verify-macos-app-name.mjs <path-to-Clovy.app>");
}

expectValue("app bundle path", basename(appPath), "Clovy.app");

const infoPlist = join(appPath, "Contents", "Info.plist");
expectPlistValue(infoPlist, "CFBundleDisplayName", "Clovy");
expectPlistValue(infoPlist, "CFBundleName", "Clovy");
expectPlistValue(infoPlist, "CFBundleIdentifier", "dev.guilhermexp.clovy");
expectPlistValue(infoPlist, "CFBundleExecutable", "clovy");

const displayedName = run("/usr/bin/swift", [
  "-e",
  "import Foundation; print(FileManager.default.displayName(atPath: CommandLine.arguments[1]))",
  appPath,
]);
// With Finder's "Show all filename extensions" on, macOS displays every app
// with its ".app" suffix; the name must still be Clovy.
const showsAllExtensions =
  spawnSync("/usr/bin/defaults", ["read", "-g", "AppleShowAllExtensions"], {
    encoding: "utf8",
  }).stdout.trim() === "1";
expectValue("macOS display name", displayedName, showsAllExtensions ? "Clovy.app" : "Clovy");

console.log(`Verified ${appPath} presents as Clovy.`);

function expectPlistValue(path, key, expected) {
  if (!existsSync(path)) {
    throw new Error(`Missing bundle metadata: ${path}`);
  }
  expectValue(
    `${path}:${key}`,
    run("/usr/bin/plutil", ["-extract", key, "raw", "-o", "-", path]),
    expected,
  );
}

function expectValue(label, actual, expected) {
  if (actual !== expected) {
    throw new Error(
      `Expected ${label} to be ${JSON.stringify(expected)}, got ${JSON.stringify(actual)}.`,
    );
  }
}

function run(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8" });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${command} failed with status ${result.status}: ${(result.stderr || result.stdout).trim()}`,
    );
  }
  return result.stdout.trim();
}
