#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import net from "node:net";
import path from "node:path";
import { fileURLToPath } from "node:url";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const frontendPort = Number.parseInt(process.env.VITE_PORT ?? "1421", 10);
const shell = process.platform === "win32";

let frontendChild = null;
let shuttingDown = false;

function portIsOpen(port) {
  return new Promise((resolve) => {
    const socket = net.createConnection({ host: "127.0.0.1", port });
    const done = (open) => {
      socket.removeAllListeners();
      socket.destroy();
      resolve(open);
    };
    socket.once("connect", () => done(true));
    socket.once("error", () => done(false));
    socket.setTimeout(300, () => done(false));
  });
}

function spawnManaged(name, command, args, cwd) {
  const child = spawn(command, args, {
    cwd,
    env: process.env,
    shell,
    stdio: "inherit",
  });

  child.on("error", (error) => {
    console.error(`${name} failed to start: ${error.message}`);
    cleanup();
    process.exit(1);
  });

  return child;
}

function cleanup() {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of [frontendChild]) {
    if (child && !child.killed) {
      child.kill();
    }
  }
}

function exitFromChild(code, signal) {
  cleanup();
  process.exit(code ?? (signal ? 1 : 0));
}

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => exitFromChild(0, signal));
}

const browserHelper = spawnSync(
  process.execPath,
  [path.join(rootDir, "scripts/prepare-agent-browser.mjs")],
  { cwd: rootDir, env: process.env, stdio: "inherit" },
);
if (browserHelper.status !== 0) process.exit(browserHelper.status ?? 1);

const localHelper = spawnSync(
  process.execPath,
  [path.join(rootDir, "scripts/prepare-local-transcriber.mjs")],
  { cwd: rootDir, env: process.env, stdio: "inherit" },
);
if (localHelper.status !== 0) process.exit(localHelper.status ?? 1);

if (process.env.CLOVY_DEV_PREPARE_ONLY === "1") {
  process.exit(0);
}

if (await portIsOpen(frontendPort)) {
  console.error(
    `Vite port ${frontendPort} became occupied before startup. Restart make dev to select another port.`,
  );
  process.exit(1);
} else {
  frontendChild = spawnManaged("Vite", "pnpm", ["run", "dev"], rootDir);
  frontendChild.on("exit", exitFromChild);
}
