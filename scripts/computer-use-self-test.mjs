#!/usr/bin/env node

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { computerUseBundleIdentifier } from "./computer-use-dev.mjs";
import { sha256, verifyEngineArtifact, verifyEngineSbom } from "./agent-desktop-artifact.mjs";
import { resolveComputerUseHost } from "./computer-use-host-path.mjs";

const rootDir = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pin = readJson(path.join(rootDir, "src-tauri", "agent-desktop-pin.json"));
const contract = readJson(path.join(rootDir, "src-tauri", "computer-use-contract.json"));
const options = parseArguments(process.argv.slice(2));
const bundleDir = path.resolve(
  options.bundle ||
    path.join(
      rootDir,
      ".tauri-helper",
      process.platform === "win32" ? "computer-use" : pin.bundleName,
    ),
);
const executable =
  process.platform === "win32"
    ? path.join(bundleDir, `${pin.executable}.exe`)
    : path.join(bundleDir, "Contents", "MacOS", pin.executable);
const stampPath =
  process.platform === "win32"
    ? path.join(bundleDir, "clovy-computer-use-pin.json")
    : path.join(bundleDir, "Contents", "Resources", "clovy-computer-use-pin.json");

async function main() {
  if (!["darwin", "win32"].includes(process.platform))
    throw new Error("Computer use helper self-test requires macOS or Windows");

  if (options.promptPermissions && !options.live && !options.permissionsOnly) {
    throw new Error("--prompt-permissions requires --live or --permissions-only");
  }

  let driver;
  try {
    if (process.platform === "win32") validateWindowsBundle();
    else validateBundle();
    validateDirectLaunchRefusal();
    if (options.requireReleaseProfile) {
      if (options.live || options.permissionsOnly)
        throw new Error("Native Computer use checks require a non-ad-hoc signed host and helper");
      // The driver authenticates its host by Apple team (computer_use_driver.rs),
      // so an ad hoc release cannot start it: only the static checks apply.
      console.error(
        "Computer use static checks passed for an ad hoc release; the driver needs a Team ID signature to run.",
      );
      return;
    }
    const host = resolveSelfTestHost();
    const startHost = (permissionPrompt) =>
      startMcp(host, [
        "--computer-use-release-self-test-host",
        executable,
        ...(permissionPrompt ? [`--permission-prompt=${permissionPrompt}`] : []),
      ]);
    driver = await startHost();
    if (options.promptPermissions) {
      const current = structured(
        await callTool(driver, "check_permissions", { prompt: false }, 45_000),
      );
      const permissionPrompt =
        current.accessibility !== true
          ? "accessibility"
          : current.screen_recording !== true
            ? "screen-recording"
            : undefined;
      if (permissionPrompt) {
        await driver.close();
        driver = await startHost(permissionPrompt);
        console.error(`Requested ${permissionPrompt} for Clovy Computer Use Driver.`);
      }
    }
    const toolsResult = await driver.request("tools/list", {});
    validateDriverContract(toolsResult.tools);
    if (options.permissionsOnly) {
      const result = await callTool(
        driver,
        "check_permissions",
        { prompt: options.promptPermissions },
        options.promptPermissions ? 60_000 : 45_000,
      );
      console.log(JSON.stringify(structured(result), null, 2));
    } else if (options.live) {
      if (process.platform === "win32") await runWindowsLiveSelfTest(driver);
      else await runLiveSelfTest(driver, false);
    }
    console.error(
      `Computer use ${options.live ? "live " : ""}self-test passed for Clovy's helper built using agent-desktop ${pin.version} (${pin.sourceCommit}).`,
    );
  } finally {
    await driver?.close();
  }
}

function parseArguments(args) {
  const parsed = {
    bundle: undefined,
    host: undefined,
    target: undefined,
    live: false,
    permissionsOnly: false,
    promptPermissions: false,
    requireDeveloperId: false,
    requireReleaseProfile: false,
  };
  for (let index = 0; index < args.length; index += 1) {
    switch (args[index]) {
      case "--bundle":
        parsed.bundle = args[index + 1];
        if (!parsed.bundle) throw new Error("--bundle requires a path");
        index += 1;
        break;
      case "--host":
        parsed.host = args[index + 1];
        if (!parsed.host) throw new Error("--host requires a path");
        index += 1;
        break;
      case "--target":
        parsed.target = args[index + 1];
        if (!parsed.target || !/^(aarch64|x86_64)-pc-windows-msvc$/.test(parsed.target))
          throw new Error("--target requires an explicit Windows MSVC target");
        index += 1;
        break;
      case "--live":
        parsed.live = true;
        break;
      case "--permissions-only":
        parsed.permissionsOnly = true;
        break;
      case "--prompt-permissions":
        parsed.promptPermissions = true;
        break;
      case "--require-developer-id":
        parsed.requireDeveloperId = true;
        break;
      // An ad hoc release: the helper must still be the release build, but it
      // carries no Developer ID.
      case "--require-release-profile":
        parsed.requireReleaseProfile = true;
        break;
      default:
        throw new Error(`Unknown Computer use self-test argument: ${args[index]}`);
    }
  }
  return parsed;
}

function validateBundle() {
  if (!existsSync(executable)) {
    throw new Error(`Pinned Computer use driver is missing: ${executable}`);
  }
  if (contract.version !== pin.version) {
    throw new Error(
      `Driver contract targets ${contract.version}, but the bundle pin is ${pin.version}.`,
    );
  }
  const version = run(executable, ["--version"]).combined.match(
    /clovy-computer-use-driver\s+([^\s]+)\s+([0-9a-f]{40})/,
  );
  if (version?.[1] !== pin.version || version?.[2] !== pin.sourceCommit) {
    throw new Error(
      `Bundled helper reports ${version?.[1] || "no version"} ${version?.[2] || "no commit"}; expected ${pin.version} ${pin.sourceCommit}.`,
    );
  }

  const engineManifest = readJson(
    path.join(bundleDir, "Contents", "Resources", "agent-desktop-manifest.json"),
  );
  for (const field of [
    "source",
    "sourceCommit",
    "version",
    "protocolVersion",
    "cargoLockSha256",
    "license",
    "licenseSha256",
    "patchSha256",
    "ownedRuntimeSourceSha256",
    "sourceReviewSha256",
  ]) {
    if (typeof pin[field] !== "string" || !pin[field] || engineManifest[field] !== pin[field])
      throw new Error(`Bundled engine ${field} differs from reviewed pin`);
  }
  if (
    sha256(path.join(bundleDir, "Contents", "Resources", "source-review.md")) !==
    pin.sourceReviewSha256
  )
    throw new Error("Bundled engine source qualification digest mismatch");
  const engine = path.join(bundleDir, "Contents", "MacOS", "agent-desktop");
  if (
    engineManifest.executable !== "agent-desktop" ||
    sha256(engine) !== engineManifest.executableSha256
  )
    throw new Error("Bundled engine digest mismatch");
  run("/usr/bin/codesign", ["--verify", "--strict", engine]);
  const stamp = readJson(stampPath);
  const profile = stamp.clovyBuild?.profile;
  const expectedBundleIdentifier = computerUseBundleIdentifier({
    baseIdentifier: pin.bundleIdentifier,
    profile,
    worktreeRoot: rootDir,
  });
  if (stamp.bundleIdentifier !== expectedBundleIdentifier) {
    throw new Error(
      `Computer use helper stamp identifies ${stamp.bundleIdentifier || "nothing"}; expected ${expectedBundleIdentifier}.`,
    );
  }

  const plist = path.join(bundleDir, "Contents", "Info.plist");
  for (const [key, expected] of [
    ["CFBundleIdentifier", expectedBundleIdentifier],
    ["CFBundleDisplayName", "Clovy Computer Use Driver"],
    ["CFBundleIconFile", "Clovy.icns"],
    ["CFBundleName", "Clovy Computer Use Driver"],
    ["LSMinimumSystemVersion", pin.minimumMacOSVersion],
  ]) {
    const actual = run("/usr/bin/plutil", ["-extract", key, "raw", "-o", "-", plist]).stdout.trim();
    if (actual !== expected) {
      throw new Error(`Computer use helper ${key} is ${actual}; expected ${expected}.`);
    }
  }
  const bundledIcon = path.join(bundleDir, "Contents", "Resources", "Clovy.icns");
  if (
    readFileSync(bundledIcon).compare(
      readFileSync(path.join(rootDir, "src-tauri", "icons", "icon.icns")),
    ) !== 0
  ) {
    throw new Error("Computer use helper icon does not match Clovy's app icon.");
  }
  const screenReason = run("/usr/bin/plutil", [
    "-extract",
    "NSScreenCaptureUsageDescription",
    "raw",
    "-o",
    "-",
    plist,
  ]).stdout.trim();
  if (!screenReason.startsWith("Clovy captures only the app windows")) {
    throw new Error("Computer use helper is missing Clovy's Screen Recording usage description.");
  }

  if (
    stamp.version !== pin.version ||
    stamp.sourceCommit !== pin.sourceCommit ||
    stamp.clovyBuild?.sourceSha256 !== helperSourceSha256() ||
    stamp.clovyBuild?.engineSha256 !== engineManifest.executableSha256
  ) {
    throw new Error("Computer use helper stamp does not match Clovy's pinned source build.");
  }
  if (
    (options.requireDeveloperId || options.requireReleaseProfile) &&
    stamp.clovyBuild?.profile !== "release"
  ) {
    throw new Error("A Computer use release must contain a release-profile helper.");
  }
  verifyEngineSbom(path.join(bundleDir, "Contents", "Resources", "agent-desktop.spdx.json"), pin);
  const bundledLicense = path.join(bundleDir, "Contents", "Resources", "agent-desktop-LICENSE");
  if (
    readFileSync(bundledLicense, "utf8") !==
    readFileSync(path.join(rootDir, "src-tauri", "agent-desktop", "LICENSE"), "utf8")
  ) {
    throw new Error("Bundled Computer use helper is missing its pinned Apache-2.0 license notice.");
  }

  const architectures = new Set(
    run("/usr/bin/lipo", ["-archs", executable]).stdout.trim().split(/\s+/),
  );
  const declaredArchitectures = stamp.clovyBuild?.architectures || [];
  if (declaredArchitectures.length === 0 || architectures.size !== declaredArchitectures.length) {
    throw new Error("Computer use helper architecture declarations do not match its binary.");
  }
  for (const architecture of declaredArchitectures) {
    if (!architectures.has(architecture)) {
      throw new Error(`Computer use helper is missing its ${architecture} slice.`);
    }
    const engineSignature = run("/usr/bin/codesign", [
      "-d",
      "--verbose=4",
      "--arch",
      architecture,
      engine,
    ]).combined;
    const cdhash = engineSignature.match(/^CDHash=([a-fA-F0-9]{40})$/m)?.[1]?.toLowerCase();
    if (!cdhash || cdhash !== engineManifest.codeDirectoryHashes?.[architecture])
      throw new Error(`Engine ${architecture} CodeDirectory identity mismatch`);
  }

  run("/usr/bin/codesign", ["--verify", "--deep", "--strict", bundleDir]);
  const signature = run("/usr/bin/codesign", ["-dv", "--verbose=4", bundleDir]).combined;
  if (!signature.includes(`Identifier=${expectedBundleIdentifier}`)) {
    throw new Error("Computer use helper signature has the wrong bundle identifier.");
  }
  if (
    options.requireDeveloperId &&
    (signature.includes("Signature=adhoc") || signature.includes("TeamIdentifier=not set"))
  ) {
    throw new Error("Computer use helper must carry a Developer ID signature in this build.");
  }
}

function validateWindowsBundle() {
  verifyEngineSbom(path.join(bundleDir, "agent-desktop.spdx.json"), pin);
  const stamp = readJson(stampPath);
  const target =
    options.target ||
    (process.arch === "arm64" ? "aarch64-pc-windows-msvc" : "x86_64-pc-windows-msvc");
  const engineManifest = verifyEngineArtifact(bundleDir, pin, target);
  if (
    stamp.sourceCommit !== pin.sourceCommit ||
    stamp.version !== pin.version ||
    stamp.clovyBuild?.sourceSha256 !== helperSourceSha256() ||
    stamp.clovyBuild?.engineSha256 !== engineManifest.executableSha256
  )
    throw new Error("Windows helper source stamp mismatch");
  const version = run(executable, ["--version"]).combined;
  if (version.trim() !== `clovy-computer-use-driver ${pin.version} ${pin.sourceCommit}`)
    throw new Error("Windows helper version mismatch");
  const publisher = (file) =>
    run("powershell.exe", [
      "-NoProfile",
      "-NonInteractive",
      "-ExecutionPolicy",
      "Bypass",
      "-File",
      path.join(rootDir, "scripts", "windows-authenticode.ps1"),
      "-TargetPath",
      file,
    ]).stdout.trim();
  const helperPublisher = publisher(executable);
  if (
    !helperPublisher ||
    helperPublisher !== publisher(path.join(bundleDir, "agent-desktop.exe")) ||
    helperPublisher !== publisher(resolveSelfTestHost())
  )
    throw new Error("Computer use helper, engine and host signing publisher mismatch");
  if (
    Object.keys(contract.driverTools).length !== 14 ||
    contract.version !== pin.version ||
    contract.engineProtocolVersion !== pin.protocolVersion
  )
    throw new Error("Computer use engine/tool contract mismatch");
}

function validateDirectLaunchRefusal() {
  const direct = spawnSync(executable, ["mcp"], {
    encoding: "utf8",
    timeout: 10_000,
    env: {
      ...driverEnvironment(),
      CLOVY_COMPUTER_USE_HELPER_CAPABILITY: "a".repeat(64),
    },
  });
  const output = `${direct.stdout || ""}\n${direct.stderr || ""}`;
  if (
    direct.status === 0 ||
    !/(must be launched directly by Clovy|private to Clovy|Only the exact bundled Clovy process)/.test(
      output,
    )
  ) {
    throw new Error("Bundled Computer use helper accepted a direct MCP launch.");
  }
}

function resolveSelfTestHost() {
  if (options.host) return resolveComputerUseHost({ host: options.host });
  const stamp = readJson(stampPath);
  const profile = stamp.clovyBuild?.profile === "release" ? "release" : "debug";
  return resolveComputerUseHost({
    host: options.host,
    targetDir: process.env.CARGO_TARGET_DIR || path.join(rootDir, "src-tauri", "target"),
    profile,
    platform: process.platform,
    targetTriple: options.target,
  });
}

async function startMcp(command, args = [], env = driverEnvironment()) {
  const client = new JsonRpcClient(command, args, env);
  const initialized = await client.request("initialize", {
    protocolVersion: contract.protocolVersion,
    capabilities: {},
    clientInfo: { name: "Clovy Computer use self-test", version: "1" },
  });
  if (!initialized?.serverInfo?.name) {
    await client.close();
    throw new Error("Computer use driver did not complete the MCP handshake.");
  }
  client.notify("notifications/initialized", {});
  return client;
}

function validateDriverContract(tools) {
  if (!Array.isArray(tools)) throw new Error("Driver tools/list did not return a tool list.");
  const byName = new Map(tools.map((tool) => [tool.name, tool]));
  if (!sameMembers([...byName.keys()], Object.keys(contract.driverTools)))
    throw new Error("Computer use helper tool allowlist changed");
  for (const [name, expected] of Object.entries(contract.driverTools)) {
    const tool = byName.get(name);
    if (!tool) throw new Error(`Pinned driver no longer exposes required tool ${name}.`);
    const schema = tool.inputSchema || tool.input_schema;
    if (schema?.additionalProperties !== false) {
      throw new Error(`${name} must keep a closed input schema.`);
    }
    const required = new Set(schema.required || []);
    for (const property of expected.required) {
      if (!required.has(property)) {
        throw new Error(`${name} no longer requires ${property}.`);
      }
    }
    for (const [property, type] of Object.entries(expected.properties)) {
      const actual = schema.properties?.[property]?.type;
      if (actual !== type) {
        throw new Error(`${name}.${property} changed type from ${type} to ${actual || "missing"}.`);
      }
    }
  }
  const captureModes = byName.get("get_window_state")?.inputSchema?.properties?.capture_mode?.enum;
  if (!sameMembers(captureModes, ["som", "vision", "ax"])) {
    throw new Error(
      "Pinned driver capture modes changed; review the Computer use broker contract.",
    );
  }
}

async function runWindowsLiveSelfTest(client) {
  const readiness = structured(await callTool(client, "check_permissions", { prompt: false }));
  if (readiness.accessibility !== true || readiness.screen_recording_capturable !== true)
    throw new Error("Windows live fixture requires a non-elevated unlocked interactive desktop");
  const host = resolveSelfTestHost();
  const fixtureDir = path.join(path.dirname(host), "native", "fixtures");
  mkdirSync(fixtureDir, { recursive: true });
  const fixture = path.join(fixtureDir, "clovy-computer-use-fixture.exe");
  if (!process.env.WINDOWS_CERTIFICATE_THUMBPRINT)
    throw new Error("Windows live fixture signing requires WINDOWS_CERTIFICATE_THUMBPRINT");
  if (existsSync(fixture))
    throw new Error(
      `Preserve the previous fixture and choose a fresh run-owned host build directory: ${fixture}`,
    );
  const csc =
    process.env.CLOVY_QA_CSC ||
    path.join(
      process.env.WINDIR || "C:\\Windows",
      "Microsoft.NET",
      "Framework64",
      "v4.0.30319",
      "csc.exe",
    );
  run(csc, [
    "/nologo",
    "/target:exe",
    `/out:${fixture}`,
    "/reference:System.Windows.Forms.dll",
    "/reference:System.Drawing.dll",
    `/win32manifest:${path.join(rootDir, "src-tauri", "native", "computer-use-fixture", "windows", "app.manifest")}`,
    path.join(rootDir, "src-tauri", "native", "computer-use-fixture", "windows", "Fixture.cs"),
  ]);
  run(process.env.WINDOWS_SIGNTOOL_PATH || "signtool", [
    "sign",
    "/sha1",
    process.env.WINDOWS_CERTIFICATE_THUMBPRINT,
    "/fd",
    "sha256",
    fixture,
  ]);
  run(process.env.WINDOWS_SIGNTOOL_PATH || "signtool", ["verify", "/pa", fixture]);
  const tempDir = mkdtempSync(path.join(tmpdir(), "clovy-windows-cu-proof-"));
  const statePath = path.join(tempDir, "target-state.txt");
  writeFileSync(statePath, "waiting");
  const children = [];
  try {
    for (const role of ["target", "observer"]) {
      const pidPath = path.join(tempDir, `${role}.pid`);
      const child = spawn(fixture, ["--role", role, "--state", statePath, "--pid-file", pidPath], {
        stdio: "ignore",
        windowsHide: true,
      });
      children.push(child);
      if (Number(await waitForFile(pidPath)) !== child.pid)
        throw new Error("Unexpected Windows fixture PID");
    }
    const [target, observer] = children;
    run(fixture, ["--activate-pid", String(observer.pid)]);
    const before = await waitForProbe(
      fixture,
      (probe) => probe.frontmostPid === observer.pid,
      3_000,
    );
    const windows = await waitForWindows(client, [target.pid, observer.pid]);
    const window = findFixtureWindow(windows, target.pid, "Clovy Computer Use Target");
    if (!window) throw new Error("Exact Windows fixture target is missing");
    const capture = await callTool(client, "get_window_state", {
      pid: target.pid,
      window_id: window.window_id,
      capture_mode: "som",
    });
    const actionLine = captureTree(capture)
      .split("\n")
      .find((line) => line.includes("Apply fixture action"));
    const element = actionLine?.match(/\[(?:element_index\s+)?(\d+)\]/)?.[1];
    if (
      !element ||
      !capture.content?.some((part) => part.type === "image" && part.data?.startsWith("iVBOR"))
    )
      throw new Error("Windows capture must contain PNG and a numbered semantic button");
    await callTool(client, "click", {
      pid: target.pid,
      window_id: window.window_id,
      element_index: Number(element),
    });
    await waitFor(
      () => readFileSync(statePath, "utf8").trim() === "clicked",
      "Windows semantic fixture click",
    );
    const after = probeFixture(fixture);
    if (
      after.frontmostPid !== before.frontmostPid ||
      after.cursorX !== before.cursorX ||
      after.cursorY !== before.cursorY
    )
      throw new Error("Windows semantic background click changed pointer or focus");
    console.error(
      "Windows live fixture proved exact-window capture and semantic click without pointer/focus change. Text, mixed DPI, secure desktop, Stop and Job Object matrix remains required.",
    );
  } finally {
    for (const child of children) {
      child.kill();
      child.unref();
    }
    // Keep the signed QA fixture and synthetic result as run-owned evidence.
    console.error(`Windows synthetic fixture evidence retained: ${tempDir}`);
  }
}

async function runLiveSelfTest(client, promptPermissions) {
  fixtureSigningIdentity();
  const permissionResult = await callTool(
    client,
    "check_permissions",
    { prompt: promptPermissions },
    promptPermissions ? 300_000 : 45_000,
  );
  const permissions = structured(permissionResult);
  if (
    permissions.accessibility !== true ||
    permissions.screen_recording !== true ||
    permissions.screen_recording_capturable !== true
  ) {
    throw new Error(
      "Live Computer use self-test needs Accessibility and a live Screen Recording grant for the signed Clovy Computer Use Driver helper.",
    );
  }

  const tempDir = mkdtempSync(path.join(tmpdir(), "clovy-computer-use-self-test-"));
  let targetPid = 0;
  let observerPid = 0;
  let targetChild;
  let observerChild;
  try {
    const fixtureExecutable = buildFixture(tempDir);
    const statePath = path.join(tempDir, "target-state.txt");
    const targetPidPath = path.join(tempDir, "target.pid");
    const observerPidPath = path.join(tempDir, "observer.pid");
    writeFileSync(statePath, "waiting\n");
    const targetApp = makeFixtureApp(tempDir, fixtureExecutable, "target");
    const observerApp = makeFixtureApp(tempDir, fixtureExecutable, "observer");
    targetChild = spawn(
      fixtureAppExecutable(targetApp),
      ["--role", "target", "--state", statePath, "--pid-file", targetPidPath],
      { stdio: "ignore" },
    );
    targetPid = targetChild.pid;
    if (Number(await waitForFile(targetPidPath)) !== targetPid) {
      throw new Error("Computer use target fixture reported an unexpected pid.");
    }
    observerChild = spawn(
      fixtureAppExecutable(observerApp),
      ["--role", "observer", "--pid-file", observerPidPath],
      { stdio: "ignore" },
    );
    observerPid = observerChild.pid;
    if (Number(await waitForFile(observerPidPath)) !== observerPid) {
      throw new Error("Computer use observer fixture reported an unexpected pid.");
    }
    run(fixtureExecutable, ["--activate-pid", String(observerPid)]);

    let before;
    try {
      before = await waitForProbe(
        fixtureExecutable,
        (probe) => probe.frontmostPid === observerPid,
        3_000,
      );
    } catch (error) {
      // A macOS privacy notification can remain frontmost after the permission
      // prompt even when both grants are active. That is still a valid focus
      // sentinel: prove it remains unchanged rather than dismissing a system UI.
      before = probeFixture(fixtureExecutable);
      await delay(500);
      const confirmation = probeFixture(fixtureExecutable);
      if (
        before.frontmostPid <= 0 ||
        before.frontmostPid === targetPid ||
        confirmation.frontmostPid !== before.frontmostPid
      ) {
        throw error;
      }
      before = confirmation;
      console.error(
        `Observer could not become frontmost; using stable ${before.frontmostName || "foreground app"} (${before.frontmostPid}) as the focus sentinel.`,
      );
    }
    const windowsBefore = await waitForWindows(client, [targetPid, observerPid]);
    const target = findFixtureWindow(windowsBefore, targetPid, "Clovy Computer Use Target");
    if (!target) {
      throw new Error(
        `Computer use fixture target window did not appear. Process windows: ${windowDiagnostic(windowsBefore, targetPid)}`,
      );
    }

    const capture = await callTool(client, "get_window_state", {
      pid: targetPid,
      window_id: target.window_id,
      capture_mode: "som",
    });
    const tree = captureTree(capture);
    const actionLine = tree.split("\n").find((line) => line.includes("Apply fixture action"));
    const element = actionLine?.match(/\[(?:element_index\s+)?(\d+)\]/)?.[1];
    if (!element) {
      throw new Error(
        `Live capture did not expose the fixture button as a numbered element. ${captureTreeDiagnostic(capture)}`,
      );
    }
    const inputLine = tree.split("\n").find((line) => line.includes("Fixture text input"));
    const inputElement = inputLine?.match(/\[(?:element_index\s+)?(\d+)\]/)?.[1];
    if (!inputElement) {
      throw new Error(
        `Live capture did not expose the fixture text field as a numbered element. ${captureTreeDiagnostic(capture)}`,
      );
    }
    const image = capture.content?.find((part) => part.type === "image")?.data;
    if (typeof image !== "string" || !image.startsWith("iVBOR")) {
      const payload = (capture.content || []).map((part) => ({
        type: part.type,
        keys: Object.keys(part).sort(),
        mimeType: part.mimeType || part.mime_type,
        dataLength: typeof part.data === "string" ? part.data.length : undefined,
        dataPrefix: typeof part.data === "string" ? part.data.slice(0, 16) : undefined,
      }));
      const captureMetadata = { ...structured(capture) };
      delete captureMetadata.tree_markdown;
      throw new Error(
        `Live Computer use capture did not return a PNG screenshot. Target: ${JSON.stringify(target)}. Process windows: ${windowDiagnostic(windowsBefore, targetPid)}. Content: ${JSON.stringify(payload)}. Structured: ${JSON.stringify(captureMetadata)}. Driver stderr: ${client.stderr.slice(-2_000)}`,
      );
    }

    await callTool(client, "click", {
      pid: targetPid,
      window_id: target.window_id,
      element_index: Number(element),
    });
    await waitFor(
      () => readFileSync(statePath, "utf8").trim() === "clicked",
      "background fixture action",
    );
    await callTool(client, "type_text", {
      pid: targetPid,
      window_id: target.window_id,
      element_index: Number(inputElement),
      text: "background input",
    });
    await waitFor(
      () => readFileSync(statePath, "utf8").trim() === "typed:background input",
      "background fixture text input",
    );

    const after = probeFixture(fixtureExecutable);
    if (after.frontmostPid !== before.frontmostPid) {
      throw new Error("Computer use background action stole application focus.");
    }
    if (
      Math.abs(after.cursorX - before.cursorX) > 0.5 ||
      Math.abs(after.cursorY - before.cursorY) > 0.5
    ) {
      throw new Error("Computer use background action moved the user's pointer.");
    }
    if (after.spaceKeyDown !== before.spaceKeyDown) {
      throw new Error("Computer use background action changed the real Space key state.");
    }
    if (after.modifierFlags !== before.modifierFlags) {
      throw new Error("Computer use background action changed the real modifier-key state.");
    }
    const windowsAfter = await waitForWindows(client, [targetPid, observerPid]);
    for (const [pid, title] of [
      [targetPid, "Clovy Computer Use Target"],
      [observerPid, "Clovy Computer Use Observer"],
    ]) {
      const prior = findFixtureWindow(windowsBefore, pid, title);
      const current = findFixtureWindow(windowsAfter, pid, title);
      if (
        prior &&
        current &&
        prior.is_on_current_space !== undefined &&
        prior.is_on_current_space !== current.is_on_current_space
      ) {
        throw new Error("Computer use background action changed the active Space.");
      }
    }
  } finally {
    terminate(targetPid);
    terminate(observerPid);
    targetChild?.unref();
    observerChild?.unref();
    rmSync(tempDir, { recursive: true, force: true });
  }
}

function buildFixture(tempDir) {
  const output = path.join(tempDir, "computer-use-fixture");
  const source = path.join(rootDir, "src-tauri", "native", "computer-use-fixture", "main.swift");
  const developerDir = existsSync("/Applications/Xcode.app/Contents/Developer")
    ? "/Applications/Xcode.app/Contents/Developer"
    : process.env.DEVELOPER_DIR;
  const swiftArchitecture = process.arch === "x64" ? "x86_64" : "arm64";
  run(
    "/usr/bin/xcrun",
    [
      "swiftc",
      "-target",
      `${swiftArchitecture}-apple-macosx${pin.minimumMacOSVersion}`,
      "-module-cache-path",
      path.join(tempDir, "swift-module-cache"),
      source,
      "-framework",
      "AppKit",
      "-framework",
      "CoreGraphics",
      "-o",
      output,
    ],
    {
      env: {
        ...process.env,
        ...(developerDir ? { DEVELOPER_DIR: developerDir } : {}),
        CLANG_MODULE_CACHE_PATH: path.join(tempDir, "clang-module-cache"),
      },
    },
  );
  return output;
}

function signatureTeam(file) {
  const details = run("/usr/bin/codesign", ["-dv", "--verbose=4", file]).combined;
  if (details.includes("Signature=adhoc"))
    throw new Error("Native fixtures require a non-ad-hoc signature");
  const team = details.match(/^TeamIdentifier=([A-Za-z0-9]+)$/m)?.[1];
  if (!team) throw new Error("Native fixture acceptance requires an Apple Team identifier");
  return team;
}
function fixtureSigningIdentity() {
  const identity =
    process.env.APPLE_SIGNING_IDENTITY?.trim() || process.env.CLOVY_DEV_SIGNING_IDENTITY?.trim();
  if (!identity || identity === "-")
    throw new Error(
      "Native fixture signing requires explicit APPLE_SIGNING_IDENTITY or CLOVY_DEV_SIGNING_IDENTITY; ad hoc signing is refused",
    );
  signatureTeam(resolveSelfTestHost());
  return identity;
}

function makeFixtureApp(tempDir, fixtureExecutable, role) {
  const displayName = role === "observer" ? "Clovy CU Observer" : "Clovy CU Target";
  const app = path.join(tempDir, `${displayName}.app`);
  const macos = path.join(app, "Contents", "MacOS");
  mkdirSync(macos, { recursive: true });
  const executableName = "computer-use-fixture";
  copyFileSync(fixtureExecutable, path.join(macos, executableName));
  chmodSync(path.join(macos, executableName), 0o755);
  writeFileSync(
    path.join(app, "Contents", "Info.plist"),
    `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleDisplayName</key><string>${displayName}</string>
<key>CFBundleExecutable</key><string>${executableName}</string>
<key>CFBundleIdentifier</key><string>dev.guilhermexp.clovy.computer-use-self-test.${role}</string>
<key>CFBundleName</key><string>${displayName}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>LSMinimumSystemVersion</key><string>13.0</string>
</dict></plist>
`,
  );
  run("/usr/bin/codesign", [
    "--force",
    "--sign",
    fixtureSigningIdentity(),
    "--timestamp=none",
    "--options",
    "runtime",
    "--identifier",
    `dev.guilhermexp.clovy.computer-use-self-test.${role}`,
    app,
  ]);
  run("/usr/bin/codesign", ["--verify", "--strict", app]);
  if (signatureTeam(app) !== signatureTeam(resolveSelfTestHost()))
    throw new Error("Native fixture must share the signed Clovy host's Apple Team");
  return app;
}

function fixtureAppExecutable(app) {
  return path.join(app, "Contents", "MacOS", "computer-use-fixture");
}

async function waitForWindows(client, pids) {
  let found;
  await waitFor(async () => {
    const windows = [];
    for (const pid of pids) {
      const result = await callTool(client, "list_windows", { pid, on_screen_only: false });
      windows.push(...(structured(result).windows || []));
    }
    if (pids.every((pid) => windows.some((window) => Number(window.pid) === pid))) {
      found = windows;
      return true;
    }
    return false;
  }, "fixture windows");
  return found;
}

function findFixtureWindow(windows, pid, title) {
  return windows.find((window) => Number(window.pid) === pid && window.title === title);
}

function windowDiagnostic(windows, pid) {
  const owned = windows
    .filter((window) => Number(window.pid) === pid)
    .map((window) => ({ title: window.title, window_id: window.window_id }));
  return JSON.stringify(owned);
}

async function waitForProbe(executablePath, predicate, timeoutMs = 15_000) {
  let found;
  let lastProbe;
  try {
    await waitFor(
      () => {
        const probe = probeFixture(executablePath);
        lastProbe = probe;
        if (predicate(probe)) {
          found = probe;
          return true;
        }
        return false;
      },
      "observer focus",
      timeoutMs,
    );
  } catch (error) {
    throw new Error(`${error.message} Last probe: ${JSON.stringify(lastProbe)}`);
  }
  return found;
}

function probeFixture(executablePath) {
  return JSON.parse(run(executablePath, ["--probe"]).stdout);
}

async function waitForFile(file) {
  await waitFor(() => existsSync(file) && readFileSync(file, "utf8").trim(), path.basename(file));
  return readFileSync(file, "utf8").trim();
}

async function waitFor(predicate, description, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      if (await predicate()) return;
    } catch (error) {
      lastError = error;
    }
    await delay(150);
  }
  throw new Error(
    `Timed out waiting for ${description}.${lastError ? ` ${lastError.message}` : ""}`,
  );
}

async function callTool(client, name, arguments_, timeoutMs = 45_000) {
  const result = await client.request("tools/call", { name, arguments: arguments_ }, timeoutMs);
  if (result?.isError || result?.is_error) {
    const message = result.content?.find((part) => part.type === "text")?.text;
    throw new Error(`${name} failed: ${message || "unknown driver error"}`);
  }
  return result;
}

function structured(result) {
  return result?.structuredContent || result?.structured_content || {};
}

function captureTree(result) {
  const structuredTree = structured(result).tree_markdown;
  if (typeof structuredTree === "string" && structuredTree.length > 0) return structuredTree;
  return (
    result?.content?.find((part) => part?.type === "text" && typeof part.text === "string")?.text ||
    ""
  );
}

function captureTreeDiagnostic(result) {
  const rawTree = captureTree(result);
  const interesting = rawTree
    .split("\n")
    .filter((line) => /AXWindow|AXButton|AXTextField|fixture|Background/i.test(line))
    .slice(-40)
    .join(" ")
    .replaceAll(/\s+/g, " ")
    .trim();
  const tree = rawTree.replaceAll(/\s+/g, " ").trim();
  const structuredKeys = Object.keys(structured(result)).sort().join(", ") || "none";
  return `Structured keys: ${structuredKeys}. Tree length: ${rawTree.length}. Relevant tree: ${interesting || "<none>"}. Tree tail: ${tree.slice(-1_000) || "<empty>"}`;
}

function driverEnvironment() {
  const env = { ...process.env };
  for (const name of Object.keys(env)) {
    if (
      name.startsWith("AGENT_DESKTOP_") ||
      name.startsWith("CUA_DRIVER_RS_") ||
      name.startsWith("CLOVY_CUA_DRIVER") ||
      name.startsWith("JUNE_CUA_DRIVER") ||
      name === "CLOVY_COMPUTER_USE_BACKEND" ||
      ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"].includes(
        name,
      )
    ) {
      delete env[name];
    }
  }
  return env;
}

class JsonRpcClient {
  constructor(command, args, env = process.env) {
    this.child = spawn(command, args, { env, stdio: ["pipe", "pipe", "pipe"] });
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = "";
    this.stderr = "";
    this.closed = false;
    this.child.stdout.setEncoding("utf8");
    this.child.stderr.setEncoding("utf8");
    this.child.stdout.on("data", (chunk) => this.onData(chunk));
    this.child.stderr.on("data", (chunk) => {
      this.stderr = `${this.stderr}${chunk}`.slice(-8_000);
    });
    this.child.on("error", (error) => this.rejectAll(error));
    this.child.on("exit", (code, signal) => {
      if (!this.closed) {
        this.rejectAll(
          new Error(
            `MCP process exited (${code ?? signal ?? "unknown"}). ${this.stderr.trim()}`.trim(),
          ),
        );
      }
    });
  }

  request(method, params, timeoutMs = 30_000) {
    const id = this.nextId;
    this.nextId += 1;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out. ${this.stderr.trim()}`.trim()));
      }, timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.write({ jsonrpc: "2.0", id, method, params });
    });
  }

  notify(method, params) {
    this.write({ jsonrpc: "2.0", method, params });
  }

  write(message) {
    this.child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  onData(chunk) {
    this.buffer += chunk;
    for (;;) {
      const newline = this.buffer.indexOf("\n");
      if (newline < 0) return;
      const line = this.buffer.slice(0, newline).trim();
      this.buffer = this.buffer.slice(newline + 1);
      if (!line) continue;
      let message;
      try {
        message = JSON.parse(line);
      } catch {
        continue;
      }
      const entry = this.pending.get(message.id);
      if (!entry) continue;
      clearTimeout(entry.timer);
      this.pending.delete(message.id);
      if (message.error)
        entry.reject(new Error(message.error.message || JSON.stringify(message.error)));
      else entry.resolve(message.result);
    }
  }

  rejectAll(error) {
    for (const entry of this.pending.values()) {
      clearTimeout(entry.timer);
      entry.reject(error);
    }
    this.pending.clear();
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    this.child.stdin.end();
    if (this.child.exitCode !== null || this.child.signalCode !== null) return;
    this.child.kill("SIGTERM");
    await Promise.race([new Promise((resolve) => this.child.once("exit", resolve)), delay(2_000)]);
    if (this.child.exitCode === null && this.child.signalCode === null) {
      this.child.kill("SIGKILL");
    }
  }
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    windowsHide: true,
    timeout: 120_000,
    maxBuffer: 24 * 1024 * 1024,
    ...options,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(
      `${command} ${args.join(" ")} exited with ${result.status ?? result.signal}.\n${result.stderr}`,
    );
  }
  return {
    stdout: result.stdout || "",
    stderr: result.stderr || "",
    combined: `${result.stdout || ""}\n${result.stderr || ""}`,
  };
}

function readJson(file) {
  return JSON.parse(readFileSync(file, "utf8"));
}

function helperSourceSha256() {
  const hash = createHash("sha256");
  for (const relative of readJson(
    path.join(rootDir, "src-tauri", "computer-use-build-inputs.json"),
  )) {
    hash.update(relative);
    hash.update("\0");
    hash.update(readFileSync(path.join(rootDir, relative)));
    hash.update("\0");
  }
  return hash.digest("hex");
}

function sameMembers(actual, expected) {
  return (
    Array.isArray(actual) &&
    actual.length === expected.length &&
    expected.every((value) => actual.includes(value))
  );
}

function terminate(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return;
  try {
    process.kill(pid, "SIGTERM");
  } catch {}
}

function delay(milliseconds) {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

await main();
