#!/usr/bin/env node
import { spawn } from "node:child_process";
import { constants } from "node:os";
import { dirname, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";

const scripts = dirname(fileURLToPath(import.meta.url));
const binaries = ["clovy", "clovy-mcp"];

export async function runWindowsDevCargo(
  argv,
  {
    env = process.env,
    cwd = process.cwd(),
    spawnProcess = spawn,
    sign,
    output = process.stdout,
    diagnostics = process.stderr,
  } = {},
) {
  let active;
  let interrupted;
  const handlers = new Map(
    ["SIGINT", "SIGTERM"].map((signal) => [
      signal,
      () => {
        interrupted = signal;
        active?.kill(signal);
      },
    ]),
  );
  for (const [signal, handler] of handlers) process.on(signal, handler);
  const status = (code, signal) => code ?? 128 + (constants.signals[signal] ?? 1);
  const step = (command, args, onLine) =>
    new Promise((done, reject) => {
      if (interrupted) return done(status(null, interrupted));
      const child = spawnProcess(command, args, {
        cwd,
        env,
        shell: false,
        stdio: ["inherit", onLine ? "pipe" : "inherit", "inherit"],
      });
      active = child;
      if (onLine) createInterface({ input: child.stdout }).on("line", onLine);
      child.once("error", reject);
      child.once("close", (code, signal) => {
        active = undefined;
        done(status(code, signal));
      });
    });
  try {
    const [operation, ...args] = argv;
    if (operation !== "run") return await step("cargo", argv);
    if (!env.WINDOWS_CERTIFICATE_THUMBPRINT?.trim())
      throw new Error("Windows development requires WINDOWS_CERTIFICATE_THUMBPRINT");
    const separator = args.indexOf("--");
    const cargoArgs = separator < 0 ? args : args.slice(0, separator);
    const appArgs = separator < 0 ? [] : args.slice(separator + 1);
    const buildArgs = [];
    for (let i = 0; i < cargoArgs.length; i++) {
      const arg = cargoArgs[i];
      if (arg === "--bin") {
        if (cargoArgs[++i] !== "clovy")
          throw new Error("The development runner only launches clovy");
      } else if (arg === "--message-format") {
        i++;
      } else if (arg.startsWith("--bin=")) {
        if (arg !== "--bin=clovy") throw new Error("The development runner only launches clovy");
      } else if (!arg.startsWith("--message-format=")) buildArgs.push(arg);
    }
    const artifacts = new Map();
    const built = await step(
      "cargo",
      [
        "build",
        ...buildArgs,
        "--features",
        "helper-binaries",
        "--message-format=json-render-diagnostics",
        ...binaries.flatMap((name) => ["--bin", name]),
      ],
      (line) => {
        let event;
        try {
          event = JSON.parse(line);
        } catch {
          output.write(`${line}\n`);
          return;
        }
        if (
          event.reason === "compiler-artifact" &&
          event.target?.kind?.includes("bin") &&
          binaries.includes(event.target.name) &&
          event.executable
        )
          artifacts.set(event.target.name, resolve(event.executable));
        if (event.reason === "compiler-message" && event.message?.rendered)
          diagnostics.write(event.message.rendered);
      },
    );
    if (built !== 0 || interrupted) return interrupted ? status(null, interrupted) : built;
    for (const name of binaries) {
      const executable = artifacts.get(name);
      if (!executable) throw new Error(`Cargo did not report the ${name} executable`);
      if (sign) await sign(executable);
      else {
        for (const [script, extra] of [
          ["windows-sign.ps1", []],
          ["windows-authenticode.ps1", ["-ExpectedThumbprint", env.WINDOWS_CERTIFICATE_THUMBPRINT]],
        ]) {
          const signed = await step("powershell.exe", [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
            resolve(scripts, script),
            "-TargetPath",
            executable,
            ...extra,
          ]);
          if (signed !== 0) return signed;
        }
      }
      if (interrupted) return status(null, interrupted);
    }
    return await step(artifacts.get("clovy"), appArgs);
  } finally {
    for (const [signal, handler] of handlers) process.removeListener(signal, handler);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  runWindowsDevCargo(process.argv.slice(2)).then(
    (code) => {
      process.exitCode = code;
    },
    (error) => {
      console.error(error.message);
      process.exitCode = 1;
    },
  );
}
