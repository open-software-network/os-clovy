import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const runner = path.join(repoRoot, "scripts", "cargo-clovy-runner.sh");
const sandboxes = [];

afterEach(() => {
  for (const sandbox of sandboxes.splice(0)) {
    rmSync(sandbox, { recursive: true, force: true });
  }
});

// Stands in for Clovy: reports what macOS sees for the running process.
const PROBE_SOURCE = `
#include <CoreFoundation/CoreFoundation.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
extern pid_t responsibility_get_pid_responsible_for_pid(pid_t);
int main(int argc, char **argv) {
  char identifier[256] = "";
  CFStringRef bundle = CFBundleGetIdentifier(CFBundleGetMainBundle());
  if (bundle) CFStringGetCString(bundle, identifier, sizeof identifier, kCFStringEncodingUTF8);
  FILE *report = fopen(getenv("PROBE_REPORT"), "a");
  fprintf(report, "{\\"variant\\":%d,\\"pid\\":%d,\\"responsible\\":%d,\\"bundle\\":\\"%s\\",\\"argv0\\":\\"%s\\",\\"args\\":[",
          VARIANT, getpid(), responsibility_get_pid_responsible_for_pid(getpid()), identifier, argv[0]);
  for (int i = 1; i < argc; i++) fprintf(report, "%s\\"%s\\"", i > 1 ? "," : "", argv[i]);
  fprintf(report, "]}\\n");
  fclose(report);
  return atoi(getenv("PROBE_EXIT"));
}
`;

// Stands in for Cargo: records its arguments and, when asked, links a new app.
const FAKE_CARGO = `#!/bin/sh
printf '%s\\n' "$*" >> "$FAKE_CARGO_LOG"
if [ -n "$FAKE_CARGO_OUTPUT" ]; then
  mkdir -p "$FAKE_CARGO_OUT_DIR"
  for bin in clovy clovy-mcp; do
    cp "$FAKE_CARGO_OUTPUT" "$FAKE_CARGO_OUT_DIR/.$bin.new"
    mv -f "$FAKE_CARGO_OUT_DIR/.$bin.new" "$FAKE_CARGO_OUT_DIR/$bin"
  done
fi
`;

function createSandbox() {
  const root = realpathSync(mkdtempSync(path.join(tmpdir(), "clovy-runner-")));
  sandboxes.push(root);
  const bin = path.join(root, "bin");
  mkdirSync(bin);
  writeFileSync(path.join(bin, "cargo"), FAKE_CARGO);
  chmodSync(path.join(bin, "cargo"), 0o755);
  const probeSource = path.join(root, "probe.c");
  writeFileSync(probeSource, PROBE_SOURCE);
  const probes = {};
  for (const variant of [1, 2]) {
    probes[variant] = path.join(root, `probe-${variant}`);
    const built = spawnSync(
      "xcrun",
      [
        "clang",
        `-DVARIANT=${variant}`,
        "-framework",
        "CoreFoundation",
        "-o",
        probes[variant],
        probeSource,
      ],
      { encoding: "utf8" },
    );
    expect(built.status, built.stderr).toBe(0);
  }
  const home = path.join(root, "home");
  const legacyStore = path.join(home, "Library", "WebKit", "Clovy", "WebsiteData");
  mkdirSync(legacyStore, { recursive: true });
  writeFileSync(path.join(legacyStore, "LocalStorage"), "onboarding complete");
  const crate = path.join(root, "src-tauri");
  mkdirSync(crate);
  return { root, bin, probes, home, crate };
}

function runRunner(sandbox, { args, output, outDir, exitCode = 0 }) {
  const report = path.join(sandbox.root, `report-${Date.now()}-${Math.random()}.jsonl`);
  writeFileSync(report, "");
  const env = {
    ...process.env,
    PATH: `${sandbox.bin}:${process.env.PATH}`,
    HOME: sandbox.home,
    CLOVY_DEV_SIGNING_IDENTITY: "-",
    OS_CLOVY_DEV_APP_NAME: "Clovy",
    OS_CLOVY_DEV_APP_IDENTIFIER: "dev.guilhermexp.clovy.runnertest",
    FAKE_CARGO_LOG: path.join(sandbox.root, "cargo.log"),
    FAKE_CARGO_OUTPUT: output ?? "",
    FAKE_CARGO_OUT_DIR: outDir,
    PROBE_REPORT: report,
    PROBE_EXIT: String(exitCode),
  };
  delete env.CARGO_TARGET_DIR;
  return new Promise((resolveRun, reject) => {
    const child = spawn(runner, args, {
      cwd: sandbox.crate,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stderr = "";
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.on("error", reject);
    child.on("exit", (status) => {
      const lines = readFileSync(report, "utf8").split("\n").filter(Boolean);
      resolveRun({ pid: child.pid, status, stderr, app: lines.map((line) => JSON.parse(line)) });
    });
  });
}

describe.runIf(process.platform === "darwin")("Tauri dev runner", () => {
  // This exercises real clang and codesign repeatedly; the deadline is not a startup benchmark.
  it("runs the app from its own signed bundle as the process Tauri supervises", async () => {
    const sandbox = createSandbox();
    const outDir = path.join(sandbox.crate, "target", "debug");
    const bundle = path.join(outDir, "Clovy Dev runnertest.app");
    const executable = path.join(bundle, "Contents", "MacOS", "clovy");

    const first = await runRunner(sandbox, {
      args: ["run", "--no-default-features", "--color", "always", "--", "--replay", "a b"],
      output: sandbox.probes[1],
      outDir,
      exitCode: 3,
    });

    expect(first.status, first.stderr).toBe(3);
    expect(readFileSync(path.join(sandbox.root, "cargo.log"), "utf8").trim()).toBe(
      "build --bin clovy --bin clovy-mcp --no-default-features --color always",
    );
    expect(first.app).toEqual([
      {
        variant: 1,
        pid: first.pid,
        responsible: first.pid,
        bundle: "dev.guilhermexp.clovy.runnertest.dev",
        argv0: executable,
        args: ["--replay", "a b"],
      },
    ]);
    expect(spawnSync("/usr/bin/codesign", ["--verify", "--strict", bundle]).status).toBe(0);
    const plist = JSON.parse(
      spawnSync(
        "/usr/bin/plutil",
        ["-convert", "json", "-o", "-", path.join(bundle, "Contents", "Info.plist")],
        { encoding: "utf8" },
      ).stdout,
    );
    expect(plist).toMatchObject({
      CFBundleExecutable: "clovy",
      CFBundleIdentifier: "dev.guilhermexp.clovy.runnertest.dev",
      CFBundleName: "Clovy Dev runnertest",
    });
    expect(plist.NSMicrophoneUsageDescription).toEqual(expect.any(String));
    expect(plist.CFBundleURLTypes).toBeUndefined();
    expect(
      readFileSync(
        path.join(
          sandbox.home,
          "Library",
          "WebKit",
          "dev.guilhermexp.clovy.runnertest.dev",
          "WebsiteData",
          "LocalStorage",
        ),
        "utf8",
      ),
    ).toBe("onboarding complete");
    expect(existsSync(path.join(sandbox.home, "Library", "WebKit", "Clovy"))).toBe(true);

    const signed = statSync(executable).ino;
    const restart = await runRunner(sandbox, { args: ["run", "--"], outDir });
    expect(restart.status, restart.stderr).toBe(0);
    expect(
      restart.app.map(({ variant, pid, responsible }) => ({ variant, pid, responsible })),
    ).toEqual([{ variant: 1, pid: restart.pid, responsible: restart.pid }]);
    expect(statSync(executable).ino).toBe(signed);

    const rebuilt = await runRunner(sandbox, {
      args: ["run", "--"],
      output: sandbox.probes[2],
      outDir,
    });
    expect(rebuilt.status, rebuilt.stderr).toBe(0);
    expect(rebuilt.app.map(({ variant, argv0 }) => ({ variant, argv0 }))).toEqual([
      { variant: 2, argv0: executable },
    ]);
    expect(statSync(executable).ino).not.toBe(signed);
    expect(spawnSync("/usr/bin/codesign", ["--verify", "--strict", bundle]).status).toBe(0);
  }, 30_000);

  it("places the bundle in the Cargo profile directory for the requested target", async () => {
    const sandbox = createSandbox();
    const outDir = path.join(sandbox.crate, "target", "aarch64-apple-darwin", "release");

    const result = await runRunner(sandbox, {
      args: ["run", "--release", "--target", "aarch64-apple-darwin", "--"],
      output: sandbox.probes[1],
      outDir,
    });

    expect(result.status, result.stderr).toBe(0);
    expect(result.app.map(({ argv0 }) => argv0)).toEqual([
      path.join(outDir, "Clovy Dev runnertest.app", "Contents", "MacOS", "clovy"),
    ]);
  }, 30_000);
});
