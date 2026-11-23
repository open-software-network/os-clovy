#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import net from "node:net";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { devAppIdentityForBranch } from "./dev-app-identity.mjs";
import { chooseDevPort } from "./dev-ports.mjs";
import {
  DEV_SIGNING_IDENTITY_ENV,
  DevSigningIdentityError,
  resolveDevSigningIdentity,
} from "./dev-signing-identity.mjs";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const ROOT_DIR = resolve(SCRIPT_DIR, "..");
const developerDir =
  process.platform === "darwin" && existsSync("/Applications/Xcode.app/Contents/Developer")
    ? "/Applications/Xcode.app/Contents/Developer"
    : process.env.DEVELOPER_DIR;
const branch = spawnSync("git", ["branch", "--show-current"], {
  cwd: ROOT_DIR,
  encoding: "utf8",
});
const defaultIdentity = devAppIdentityForBranch("");
const devAppIdentity = devAppIdentityForBranch(branch.status === 0 ? branch.stdout : "", {
  identifierOverride: process.env.CLOVY_DEV_APP_IDENTIFIER,
});

if (devAppIdentity.identifier !== defaultIdentity.identifier) {
  console.error(
    `Using development app identity ${devAppIdentity.productName} (${devAppIdentity.identifier}).`,
  );
}

// macOS keeps the grants of the dev app bundle and its helpers across restarts
// and rebuilds only while every build is signed by the same certificate
// (docs/development.md). Resolve it once and hand the same SHA-1 to the
// Computer use helper, build.rs, and the runner that signs the app bundle.
let devSigningIdentity;
let windowsDevRunner;
if (process.platform === "darwin") {
  try {
    devSigningIdentity = resolveDevSigningIdentity().identity;
  } catch (error) {
    if (!(error instanceof DevSigningIdentityError)) throw error;
    console.error(error.message);
    process.exit(1);
  }

  // Cargo validates bundled resources while preparing the Computer use driver.
  // The Swift helper must exist before that first Cargo invocation on a fresh checkout.
  const localPrepare = spawnSync(
    process.execPath,
    [resolve(SCRIPT_DIR, "prepare-local-transcriber.mjs")],
    {
      cwd: ROOT_DIR,
      env: {
        ...process.env,
        ...(developerDir ? { DEVELOPER_DIR: developerDir } : {}),
        CLOVY_DEV_APP_IDENTIFIER: devAppIdentity.identifier,
        [DEV_SIGNING_IDENTITY_ENV]: devSigningIdentity,
      },
      stdio: "inherit",
    },
  );
  if (localPrepare.status !== 0) process.exit(localPrepare.status ?? 1);

  const browserPrepare = spawnSync(
    process.execPath,
    [resolve(SCRIPT_DIR, "prepare-agent-browser.mjs")],
    {
      cwd: ROOT_DIR,
      env: { ...process.env, [DEV_SIGNING_IDENTITY_ENV]: devSigningIdentity },
      stdio: "inherit",
    },
  );
  if (browserPrepare.status !== 0) process.exit(browserPrepare.status ?? 1);

  const prepare = spawnSync(process.execPath, [resolve(SCRIPT_DIR, "prepare-computer-use.mjs")], {
    cwd: ROOT_DIR,
    env: {
      ...process.env,
      ...(developerDir ? { DEVELOPER_DIR: developerDir } : {}),
      CLOVY_DEV_APP_IDENTIFIER: devAppIdentity.identifier,
      [DEV_SIGNING_IDENTITY_ENV]: devSigningIdentity,
    },
    stdio: "inherit",
  });
  if (prepare.status !== 0) process.exit(prepare.status ?? 1);
}

if (process.platform !== "darwin") {
  const browserPrepare = spawnSync(
    process.execPath,
    [resolve(SCRIPT_DIR, "prepare-agent-browser.mjs")],
    { cwd: ROOT_DIR, stdio: "inherit" },
  );
  if (browserPrepare.status !== 0) process.exit(browserPrepare.status ?? 1);
}

if (process.platform === "win32") {
  const prepare = spawnSync(process.execPath, [resolve(SCRIPT_DIR, "prepare-computer-use.mjs")], {
    cwd: ROOT_DIR,
    env: { ...process.env, CLOVY_DEV_APP_IDENTIFIER: devAppIdentity.identifier },
    stdio: "inherit",
  });
  if (prepare.status !== 0) process.exit(prepare.status ?? 1);
  const runnerDirectory = resolve(ROOT_DIR, ".tauri-helper");
  mkdirSync(runnerDirectory, { recursive: true });
  windowsDevRunner = resolve(runnerDirectory, "windows-dev-runner.exe");
  const builtRunner = spawnSync(
    "rustc",
    [
      "--edition=2021",
      resolve(ROOT_DIR, "src-tauri/native/windows-dev-runner/main.rs"),
      "-C",
      "opt-level=1",
      "-o",
      windowsDevRunner,
    ],
    {
      cwd: ROOT_DIR,
      env: {
        ...process.env,
        CLOVY_WINDOWS_DEV_NODE: process.execPath,
        CLOVY_WINDOWS_DEV_SCRIPT: resolve(SCRIPT_DIR, "cargo-windows-dev-runner.mjs"),
      },
      stdio: "inherit",
    },
  );
  if (builtRunner.status !== 0) process.exit(builtRunner.status ?? 1);
}

// A port is "free" when a connection is refused. Mirrors the probe in
// tauri-before-dev.mjs so both scripts agree on which port to use.
function portIsFree(port) {
  return new Promise((resolveProbe) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    const done = (free) => {
      socket.removeAllListeners();
      socket.destroy();
      resolveProbe(free);
    };
    socket.once("connect", () => done(false));
    socket.once("error", () => done(true));
    socket.setTimeout(300, () => done(true));
  });
}

// Pick the frontend port before Tauri reads its config, so Vite, the devUrl,
// and before-dev's probe all agree. An explicit VITE_PORT is honored as-is;
// otherwise scan upward from 1421 for a free port so parallel worktrees never
// collide (and never silently reuse each other's Vite).
async function resolveFrontendPort() {
  return chooseDevPort({
    name: "Vite",
    explicitValue: process.env.VITE_PORT,
    base: 1421,
    portIsFree,
  });
}

const REPLAY_ONBOARDING_FLAG = "--replay-onboarding";
const platformConfigs = {
  darwin: "src-tauri/tauri.macos.conf.json",
  win32: "src-tauri/tauri.windows.conf.json",
};

let replayOnboarding = false;
const tauriArgs = [];
const rawUserArgs = process.argv.slice(2);
const userArgs = rawUserArgs[0] === "--" ? rawUserArgs.slice(1) : rawUserArgs;

for (const arg of userArgs) {
  if (arg === REPLAY_ONBOARDING_FLAG) {
    replayOnboarding = true;
  } else {
    tauriArgs.push(arg);
  }
}

const config = platformConfigs[process.platform];
const hasConfigOverride = tauriArgs.some(
  (arg) => arg === "--config" || arg.startsWith("--config="),
);
if (config && !hasConfigOverride) {
  tauriArgs.unshift("--config", config);
}

const frontendPort = await resolveFrontendPort();
// Write a tiny config overlay file so Windows shell invocation does not have to
// preserve inline JSON quoting for `--config`.
const scriptDir = dirname(fileURLToPath(import.meta.url));
const devConfigPath = resolve(scriptDir, "..", "src-tauri", ".tauri.dev.generated.json");
writeFileSync(
  devConfigPath,
  JSON.stringify({
    productName: devAppIdentity.productName,
    identifier: devAppIdentity.identifier,
    build: {
      devUrl: `http://127.0.0.1:${frontendPort}`,
      ...(windowsDevRunner ? { runner: windowsDevRunner } : {}),
    },
  }),
);
// Merge a devUrl override last so it wins over the file configs, pointing the
// native window at the Vite server that before-dev will start on this port.
tauriArgs.push("--config", devConfigPath);

const tauri = tauriInvocation();
const child = spawn(tauri.command, [...tauri.args, "dev", ...tauriArgs], {
  env: {
    ...process.env,
    VITE_PORT: String(frontendPort),
    ...(developerDir ? { DEVELOPER_DIR: developerDir } : {}),
    OS_CLOVY_DEV_APP_NAME: devAppIdentity.productName,
    OS_CLOVY_DEV_APP_IDENTIFIER: devAppIdentity.identifier,
    CLOVY_DEV_APP_IDENTIFIER: devAppIdentity.identifier,
    ...(devSigningIdentity ? { [DEV_SIGNING_IDENTITY_ENV]: devSigningIdentity } : {}),
    ...(replayOnboarding ? { VITE_CLOVY_REPLAY_ONBOARDING: "1" } : {}),
  },
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

function tauriInvocation() {
  if (process.platform === "win32") {
    const localScript = findNodeModulesPath("@tauri-apps", "cli", "tauri.js");
    if (!localScript) {
      throw new Error(
        `Tauri CLI entry point not found from "${SCRIPT_DIR}". Run pnpm install first.`,
      );
    }
    return { command: process.execPath, args: [localScript] };
  }

  const localBinary = findNodeModulesPath(".bin", "tauri");
  return { command: localBinary ?? "tauri", args: [] };
}

function findNodeModulesPath(...segments) {
  let current = resolve(SCRIPT_DIR, "..");
  while (true) {
    const candidate = resolve(current, "node_modules", ...segments);
    if (existsSync(candidate)) {
      return candidate;
    }
    const parent = resolve(current, "..");
    if (parent === current) {
      return undefined;
    }
    current = parent;
  }
}
