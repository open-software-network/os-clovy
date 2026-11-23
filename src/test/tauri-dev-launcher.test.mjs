import { spawnSync } from "node:child_process";
import {
  chmodSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { computerUseBundleIdentifier } from "../../scripts/computer-use-dev.mjs";

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const sandboxes = [];

afterEach(() => {
  for (const sandbox of sandboxes.splice(0)) {
    rmSync(sandbox, { recursive: true, force: true });
  }
});

// Sandbox stand-in for /usr/bin/tccutil. `tccutil reset SERVICE [BUNDLE_ID]`
// removes the matching authorizations from the sandbox store, as the real tool
// does for TCC, so a launcher that revokes grants changes observable state.
const SANDBOX_TCCUTIL = `#!/usr/bin/env node
const fs = require("node:fs");
const store = process.env.CLOVY_SANDBOX_TCC_STORE;
const [command, service, bundle] = process.argv.slice(2);
if (command !== "reset" || !service) process.exit(64);
const grants = JSON.parse(fs.readFileSync(store, "utf8"));
const kept = grants.filter(
  (grant) =>
    !((service === "All" || grant.service === service) && (!bundle || grant.bundle === bundle)),
);
fs.writeFileSync(store, JSON.stringify(kept));
`;

// Loaded with `node --import` before the launcher: macOS system tools named by
// absolute path run against the sandbox instead (tccutil against the sandbox
// store, LaunchServices registration as a no-op). Everything else runs as is.
const SANDBOX_SYSTEM_TOOLS = `
import { createRequire, syncBuiltinESMExports } from "node:module";

const require = createRequire(import.meta.url);
const childProcess = require("node:child_process");
const redirects = new Map([
  ["/usr/bin/tccutil", process.env.CLOVY_SANDBOX_TCCUTIL],
  [
    "/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister",
    "/usr/bin/true",
  ],
]);
for (const name of ["spawn", "spawnSync", "execFile", "execFileSync"]) {
  const original = childProcess[name];
  childProcess[name] = (command, ...rest) => original(redirects.get(command) ?? command, ...rest);
}
syncBuiltinESMExports();
`;

const FAKE_TAURI = `#!/bin/sh
printf '%s\\n' "$@" > "$CLOVY_SANDBOX_TAURI_ARGS"
`;

function createSandbox() {
  const sandbox = realpathSync(mkdtempSync(path.join(tmpdir(), "clovy-tauri-dev-launcher-")));
  sandboxes.push(sandbox);
  cpSync(path.join(repoRoot, "scripts"), path.join(sandbox, "scripts"), { recursive: true });
  // Native helper builds are covered separately; assert the launcher prepares them.
  writeFileSync(
    path.join(sandbox, "scripts", "prepare-computer-use.mjs"),
    'import { existsSync } from "node:fs"; if (!existsSync("prepared-local-transcriber")) throw new Error("Local helper resource must exist before Cargo validation");',
  );
  writeFileSync(
    path.join(sandbox, "scripts", "prepare-local-transcriber.mjs"),
    'import { writeFileSync } from "node:fs"; writeFileSync("prepared-local-transcriber", "yes"); writeFileSync("local-helper-developer-dir", process.env.DEVELOPER_DIR ?? "");',
  );
  writeFileSync(
    path.join(sandbox, "scripts", "prepare-agent-browser.mjs"),
    'import { writeFileSync } from "node:fs"; writeFileSync("prepared-private-browser", "synthetic launcher fixture");',
  );
  mkdirSync(path.join(sandbox, "src-tauri"), { recursive: true });
  for (const file of ["agent-desktop-pin.json", "tauri.conf.json", "tauri.macos.conf.json"]) {
    cpSync(path.join(repoRoot, "src-tauri", file), path.join(sandbox, "src-tauri", file));
  }
  const bin = path.join(sandbox, "bin");
  mkdirSync(bin);
  for (const [name, source] of [
    ["tccutil", SANDBOX_TCCUTIL],
    ["tauri", FAKE_TAURI],
  ]) {
    writeFileSync(path.join(bin, name), source);
    chmodSync(path.join(bin, name), 0o755);
  }
  writeFileSync(path.join(sandbox, "system-tools.mjs"), SANDBOX_SYSTEM_TOOLS);
  return { sandbox, bin };
}

describe.runIf(process.platform === "darwin")("pnpm tauri:dev launcher", () => {
  it.each([
    "",
    "dev.guilhermexp.clovy.fixture",
  ])("uses the same resolved helper identity before and inside Tauri (override=%s)", (identifierOverride) => {
    const { sandbox, bin } = createSandbox();
    writeFileSync(
      path.join(bin, "git"),
      '#!/bin/sh\nprintf "%s\\n" "codex/jun-123-parakeet-fixture"\n',
    );
    chmodSync(path.join(bin, "git"), 0o755);
    const helper = path.join(sandbox, "scripts", "prepare-local-transcriber.mjs");
    writeFileSync(
      helper,
      'import { appendFileSync, writeFileSync } from "node:fs"; writeFileSync("prepared-local-transcriber", "yes"); appendFileSync("helper-identifiers", (process.env.CLOVY_DEV_APP_IDENTIFIER ?? "") + "\\n");',
    );
    // Stand in for Tauri invoking beforeDevCommand with the child's environment.
    writeFileSync(
      path.join(bin, "tauri"),
      '#!/bin/sh\nexec "$CLOVY_SANDBOX_NODE" "$CLOVY_SANDBOX_LOCAL_HELPER"\n',
    );
    const launch = spawnSync(process.execPath, [path.join(sandbox, "scripts", "tauri-dev.mjs")], {
      cwd: sandbox,
      encoding: "utf8",
      env: {
        ...process.env,
        PATH: `${bin}:${process.env.PATH}`,
        CLOVY_DEV_APP_IDENTIFIER: identifierOverride,
        CLOVY_DEV_SIGNING_IDENTITY: "-",
        CLOVY_SANDBOX_NODE: process.execPath,
        CLOVY_SANDBOX_LOCAL_HELPER: helper,
        VITE_PORT: "1999",
      },
      timeout: 30_000,
    });
    expect(launch.status, launch.stderr).toBe(0);
    const configured = JSON.parse(
      readFileSync(path.join(sandbox, "src-tauri", ".tauri.dev.generated.json"), "utf8"),
    );
    if (identifierOverride) expect(configured.identifier).toBe(identifierOverride);
    else expect(configured.identifier).not.toBe("dev.guilhermexp.clovy");
    expect(readFileSync(path.join(sandbox, "helper-identifiers"), "utf8").split("\n")).toEqual([
      configured.identifier,
      configured.identifier,
      "",
    ]);
  });

  it("starts Tauri while existing authorizations and signed helper copies survive", () => {
    const { sandbox, bin } = createSandbox();
    const pin = JSON.parse(
      readFileSync(path.join(sandbox, "src-tauri", "agent-desktop-pin.json"), "utf8"),
    );
    const helperIdentifier = computerUseBundleIdentifier({
      baseIdentifier: pin.bundleIdentifier,
      profile: "debug",
      worktreeRoot: sandbox,
    });
    const helpers = [
      path.join(sandbox, ".tauri-helper", pin.bundleName),
      path.join(sandbox, "src-tauri", "target", "debug", "native", "bin", pin.bundleName),
    ];
    for (const helper of helpers) {
      mkdirSync(helper, { recursive: true });
      writeFileSync(path.join(helper, "granted-copy"), "signed helper the user authorized");
    }
    const store = path.join(sandbox, "authorizations.json");
    const authorizations = [
      { service: "Accessibility", bundle: helperIdentifier },
      { service: "ScreenCapture", bundle: "dev.guilhermexp.clovy" },
      { service: "Accessibility", bundle: "dev.guilhermexp.clovy.dev" },
      { service: "ScreenCapture", bundle: "dev.guilhermexp.clovy.dev" },
      { service: "Microphone", bundle: "dev.guilhermexp.clovy.dev" },
    ];
    writeFileSync(store, JSON.stringify(authorizations));
    const tauriArgs = path.join(sandbox, "tauri-args.txt");

    const launch = spawnSync(
      process.execPath,
      [
        "--import",
        path.join(sandbox, "system-tools.mjs"),
        path.join(sandbox, "scripts", "tauri-dev.mjs"),
      ],
      {
        cwd: sandbox,
        encoding: "utf8",
        env: {
          ...process.env,
          PATH: `${bin}:${process.env.PATH}`,
          CARGO_TARGET_DIR: "",
          DEVELOPER_DIR: "/fixture/older-swift",
          CLOVY_DEV_APP_IDENTIFIER: "",
          CLOVY_DEV_SIGNING_IDENTITY: "-",
          CLOVY_SANDBOX_TAURI_ARGS: tauriArgs,
          CLOVY_SANDBOX_TCC_STORE: store,
          CLOVY_SANDBOX_TCCUTIL: path.join(bin, "tccutil"),
          VITE_PORT: "1999",
        },
        timeout: 30_000,
      },
    );

    expect(launch.status, launch.stderr).toBe(0);
    expect(readFileSync(path.join(sandbox, "local-helper-developer-dir"), "utf8")).toBe(
      existsSync("/Applications/Xcode.app/Contents/Developer")
        ? "/Applications/Xcode.app/Contents/Developer"
        : "/fixture/older-swift",
    );
    expect(readFileSync(tauriArgs, "utf8").split("\n")).toContain("dev");
    expect(JSON.parse(readFileSync(store, "utf8"))).toEqual(authorizations);
    for (const helper of helpers) {
      expect(existsSync(path.join(helper, "granted-copy")), helper).toBe(true);
    }
  });
});

describe("development frontend startup", () => {
  it("runs Vite without a backend checkout or backend skip flag", () => {
    const { sandbox, bin } = createSandbox();
    const commands = path.join(sandbox, "frontend-command.json");
    const capture = path.join(bin, "capture.cjs");
    writeFileSync(
      capture,
      'require("node:fs").writeFileSync(process.env.CLOVY_SANDBOX_COMMANDS, JSON.stringify(process.argv.slice(2)));',
    );
    const windows = process.platform === "win32";
    const stub = path.join(bin, windows ? "pnpm.cmd" : "pnpm");
    writeFileSync(
      stub,
      windows
        ? `@echo off\r\n"${process.execPath}" "${capture}" %*\r\n`
        : `#!/bin/sh\nexec '${process.execPath.replaceAll("'", "'\\''")}' '${capture.replaceAll("'", "'\\''")}' "$@"\n`,
    );
    chmodSync(stub, 0o755);
    const env = {
      ...process.env,
      PATH: [bin, process.env.PATH ?? ""].join(path.delimiter),
      VITE_PORT: "0",
      CLOVY_SANDBOX_COMMANDS: commands,
    };
    delete env.CLOVY_DEV_SKIP_LOCAL_API;
    delete env.CLOVY_DEV_PREPARE_ONLY;
    const launch = spawnSync(
      process.execPath,
      [path.join(sandbox, "scripts", "tauri-before-dev.mjs")],
      { cwd: sandbox, env, encoding: "utf8", timeout: 10_000 },
    );
    expect(launch.status, launch.stderr).toBe(0);
    expect(JSON.parse(readFileSync(commands, "utf8"))).toEqual(["run", "dev"]);
    expect(existsSync(path.join(sandbox, "prepared-local-transcriber"))).toBe(true);
    expect(launch.stderr).not.toContain("Clovy API");
  });
});
