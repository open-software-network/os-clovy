#!/usr/bin/env node
import { spawn, execFileSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
} from "node:fs";
import { createServer } from "node:net";
import { homedir, tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const DEFAULT_HOST = "127.0.0.1";
const DEFAULT_URL = "http://127.0.0.1:1421/";
const DEFAULT_PROMPT = "hi";
const DEFAULT_TIMEOUT_MS = 120_000;
const DEFAULT_VIEWPORT = "1280x720";

function parseArgs(argv) {
  const args = {
    url: DEFAULT_URL,
    prompt: DEFAULT_PROMPT,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    viewport: DEFAULT_VIEWPORT,
    outDir: ".tmp/qa-recordings",
    headless: true,
    video: true,
    keepHermesHome: false,
    expectText: "",
    hermesCommand: process.env.JUNE_HERMES_COMMAND || "",
    sourceHermesHome: process.env.JUNE_HERMES_HOME || "",
    chromeExecutable: process.env.CHROME_EXECUTABLE || "",
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = () => {
      index += 1;
      if (index >= argv.length) throw new Error(`missing value for ${arg}`);
      return argv[index];
    };
    if (arg === "--url") args.url = next();
    else if (arg === "--prompt") args.prompt = next();
    else if (arg === "--timeout-ms") args.timeoutMs = Number(next());
    else if (arg === "--viewport") args.viewport = next();
    else if (arg === "--out-dir") args.outDir = next();
    else if (arg === "--hermes-command") args.hermesCommand = next();
    else if (arg === "--source-hermes-home") args.sourceHermesHome = next();
    else if (arg === "--expect-text") args.expectText = next();
    else if (arg === "--chrome-executable") args.chromeExecutable = next();
    else if (arg === "--headed") args.headless = false;
    else if (arg === "--no-video") args.video = false;
    else if (arg === "--keep-hermes-home") args.keepHermesHome = true;
    else if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    } else {
      throw new Error(`unknown argument: ${arg}`);
    }
  }

  if (!Number.isFinite(args.timeoutMs) || args.timeoutMs <= 0) {
    throw new Error("--timeout-ms must be a positive number");
  }
  return args;
}

function printHelp() {
  console.log(`Usage:
  run_background_agent_prompt.mjs [options]

Options:
  --url <url>                    App URL. Default: ${DEFAULT_URL}
  --prompt <text>                Prompt to type. Default: ${DEFAULT_PROMPT}
  --timeout-ms <ms>              Completion timeout. Default: ${DEFAULT_TIMEOUT_MS}
  --viewport <width>x<height>    Browser viewport and video size. Default: ${DEFAULT_VIEWPORT}
  --out-dir <path>               Artifact directory. Default: .tmp/qa-recordings
  --hermes-command <path>        Hermes binary. Default: JUNE_HERMES_COMMAND or app dev runtime
  --source-hermes-home <path>    Hermes home to copy config from. Default: JUNE_HERMES_HOME or app dev home
  --expect-text <text>           Require the visible assistant reply to contain this text
  --chrome-executable <path>     Chrome executable. Default: CHROME_EXECUTABLE or common macOS paths
  --headed                       Run a visible browser instead of headless
  --no-video                     Disable Playwright video recording
  --keep-hermes-home             Keep the temporary Hermes home for debugging`);
}

function repoRoot() {
  try {
    return execFileSync("git", ["rev-parse", "--show-toplevel"], {
      cwd: SCRIPT_DIR,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {
    return resolve(SCRIPT_DIR, "../../..");
  }
}

function loadPlaywright(root) {
  const candidateRequires = [
    createRequire(import.meta.url),
    createRequire(join(root, "package.json")),
  ];
  for (const req of candidateRequires) {
    try {
      return req("playwright");
    } catch {
      // Try the next location.
    }
  }
  throw new Error(
    "Playwright is required. Restore the repository's pinned dependencies with " +
      "`pnpm install --frozen-lockfile`.",
  );
}

function parseViewport(raw) {
  const match = /^(\d+)x(\d+)$/.exec(raw.trim());
  if (!match) throw new Error("--viewport must look like 1280x720");
  return { width: Number(match[1]), height: Number(match[2]) };
}

function firstExisting(paths) {
  const existing = paths.find((path) => path && existsSync(path));
  return existing ? resolve(existing) : "";
}

function resolveHermesCommand(requested) {
  return firstExisting([
    requested,
    join(
      homedir(),
      "Library/Application Support/co.opensoftware.june-dev/hermes-runtime/hermes-agent/venv/bin/hermes",
    ),
    join(homedir(), ".hermes/hermes-agent/venv/bin/hermes"),
    join(homedir(), ".local/bin/hermes"),
  ]);
}

function resolveSourceHermesHome(requested) {
  return firstExisting([
    requested,
    join(homedir(), "Library/Application Support/co.opensoftware.june-dev/hermes"),
    join(homedir(), ".hermes"),
  ]);
}

function resolveChromeExecutable(requested) {
  return firstExisting([
    requested,
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
  ]);
}

function randomToken() {
  return randomBytes(24).toString("hex");
}

function allocatePort(host) {
  return new Promise((resolvePort, reject) => {
    const server = createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, host, () => {
      const address = server.address();
      server.close(() => {
        if (address && typeof address === "object") resolvePort(address.port);
        else reject(new Error("could not allocate a localhost port"));
      });
    });
  });
}

async function delay(ms) {
  await new Promise((resolveDelay) => setTimeout(resolveDelay, ms));
}

async function waitForStatus({ host, port, token, child, getChildError = () => null }) {
  const deadline = Date.now() + 45_000;
  let lastError = "timeout";
  while (Date.now() < deadline) {
    const childError = getChildError();
    if (childError) {
      throw new Error(`Hermes failed to launch: ${describeError(childError)}`);
    }
    if (child.exitCode !== null) {
      throw new Error(`Hermes exited before readiness, code ${child.exitCode}`);
    }
    try {
      const response = await fetch(`http://${host}:${port}/api/status`, {
        headers: {
          authorization: `Bearer ${token}`,
          "X-Hermes-Session-Token": token,
        },
        signal: AbortSignal.timeout(5_000),
      });
      if (response.ok) {
        await response.text();
        return;
      }
      lastError = `HTTP ${response.status}`;
    } catch (error) {
      lastError = describeError(error);
    }
    await delay(500);
  }
  throw new Error(`Hermes did not become ready: ${lastError}`);
}

function timestamp() {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\..+$/, "");
}

function slugFor(text) {
  return (
    text
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 40) || "prompt"
  );
}

function qaNote() {
  const now = new Date().toISOString();
  return {
    id: "qa-note-1",
    title: "QA note",
    preview: "",
    processingStatus: "draft",
    folderIds: [],
    createdAt: now,
    updatedAt: now,
    editedContent: "",
    generatedContent: "",
    sourceTranscripts: [],
  };
}

function connectionFor({ host, port, token, hermesCommand, hermesHome }) {
  return {
    baseUrl: `http://${host}:${port}`,
    wsUrl: `ws://${host}:${port}/api/ws?token=${encodeURIComponent(token)}`,
    token,
    port,
    command: hermesCommand,
    hermesHome,
    cwd: join(hermesHome, "workspace"),
    providerProxyPort: 0,
    pid: 0,
    sandboxed: false,
    fullMode: false,
  };
}

function browserInitScript() {
  return ({ connection, prompt }) => {
    const callbacks = new Map();
    let callbackId = 1;
    const makeNote = () => {
      const now = new Date().toISOString();
      return {
        id: "qa-note-1",
        title: "QA note",
        preview: "",
        processingStatus: "draft",
        folderIds: [],
        createdAt: now,
        updatedAt: now,
        editedContent: "",
        generatedContent: "",
        sourceTranscripts: [],
      };
    };

    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
      unregisterListener() {},
    };
    window.__TAURI_INTERNALS__ = {
      callbacks,
      transformCallback: (callback, once = false) => {
        const id = callbackId;
        callbackId += 1;
        callbacks.set(id, { callback, once });
        return id;
      },
      unregisterCallback: (id) => callbacks.delete(id),
      runCallback: (id, payload) => {
        const entry = callbacks.get(id);
        if (!entry) return;
        entry.callback(payload);
        if (entry.once) callbacks.delete(id);
      },
      convertFileSrc: (path) => path,
      metadata: {
        currentWindow: { label: "main" },
        currentWebview: { label: "main" },
      },
    };

    window.__qaWsEvents = [];
    window.__qaComplete = false;
    const NativeWebSocket = window.WebSocket;
    function WrappedWebSocket(url, protocols) {
      const socket =
        protocols === undefined
          ? new NativeWebSocket(url)
          : new NativeWebSocket(url, protocols);
      socket.addEventListener("message", (event) => {
        try {
          const frame = JSON.parse(String(event.data));
          const type =
            frame?.params?.type ||
            frame?.params?.event ||
            frame?.method ||
            frame?.type ||
            "unknown";
          window.__qaWsEvents.push(type);
          if (
            type === "message.complete" ||
            type === "error" ||
            type === "session.idle" ||
            type === "turn.complete" ||
            type === "turn.completed" ||
            type === "turn.error" ||
            type === "session.completed"
          ) {
            window.__qaComplete = true;
          }
        } catch {
          // Ignore non-JSON frames.
        }
      });
      return socket;
    }
    WrappedWebSocket.prototype = NativeWebSocket.prototype;
    for (const key of ["CONNECTING", "OPEN", "CLOSING", "CLOSED"]) {
      WrappedWebSocket[key] = NativeWebSocket[key];
    }
    window.WebSocket = WrappedWebSocket;

    window.__TAURI_INTERNALS__.invoke = async (cmd, args = {}) => {
      window.__qaInvokes = [...(window.__qaInvokes || []), cmd];
      if (cmd === "plugin:event|listen") return Math.floor(Math.random() * 1_000_000);
      if (
        cmd === "plugin:event|unlisten" ||
        cmd === "plugin:event|emit" ||
        cmd === "dictation_helper_command" ||
        cmd === "computer_use_begin_run"
      ) {
        return undefined;
      }
      if (cmd === "plugin:window|is_focused") return true;
      if (cmd === "bootstrap_app") {
        return {
          folders: [],
          notes: [makeNote()],
          activeRecoveries: [],
          providerConfigured: true,
        };
      }
      if (cmd === "get_note" || cmd === "create_note") return makeNote();
      if (cmd === "list_notes") return { items: [makeNote()] };
      if (
        cmd === "list_folders" ||
        cmd === "list_session_folders" ||
        cmd === "hermes_bridge_skills" ||
        cmd === "hermes_bridge_toolsets"
      ) {
        return [];
      }
      if (cmd === "list_agent_tasks") return { items: [] };
      if (cmd === "dictation_hotkey_status") {
        return { registered: false, shortcut: null, error: null };
      }
      if (cmd === "dictation_settings") {
        return { shortcut: null, microphone: null, style: "standard", language: null };
      }
      if (cmd === "check_recording_source_readiness") {
        return { sourceMode: "microphoneOnly", ready: true, sources: [] };
      }
      if (cmd === "os_accounts_status") {
        return {
          signedIn: true,
          configured: true,
          localDev: true,
          user: { id: "usr_local_qa", handle: "local-qa" },
          balance: { credits: 999999, usdMillis: 0 },
          subscription: { subscribed: true, status: "active" },
        };
      }
      if (cmd === "provider_model_settings") {
        return {
          settings: {
            transcriptionProvider: "groq",
            transcriptionModel: "whisper-large-v3-turbo",
            generationModel: "z-ai/glm-5.2",
          },
        };
      }
      if (cmd === "list_provider_models") {
        const mode = args?.request?.mode || "generation";
        const modelType = { generation: "text", transcription: "asr", image: "image", video: "video" }[mode];
        // This browser harness simulates a configured service. Never return
        // text rows for media now that the production command fetches them.
        const ids = {
          generation: ["z-ai/glm-5.2"],
          transcription: ["whisper-large-v3-turbo"],
          image: ["fal-ai/flux/dev", "fal-ai/flux/schnell", "fal-ai/qwen-image"],
          video: ["fal-ai/wan/v2.2-a14b/text-to-video", "alibaba/wan-3.0/text-to-video"],
        }[mode] || [];
        return {
          mode,
          modelType,
          selectedModel: ids[0] || "",
          models: ids.map((id) => ({
            provider: mode === "generation" ? "openrouter" : mode === "transcription" ? "groq" : "fal",
            id,
            name: id === "z-ai/glm-5.2" ? "GLM 5.2" : id,
            modelType,
            serviceAvailable: true,
            privacy: null,
            traits: [],
            capabilities: [],
          })),
        };
      }
      if (cmd === "hermes_bridge_status" || cmd === "start_hermes_bridge") {
        return { running: true, connection, connections: [connection] };
      }
      if (
        cmd === "ensure_hermes_bridge_gateway" ||
        cmd === "ensure_hermes_bridge_session" ||
        cmd === "save_agent_hermes_session" ||
        cmd === "save_agent_assistant_message"
      ) {
        return {};
      }
      if (cmd === "hermes_bridge_sessions") return { sessions: [], items: [] };
      if (cmd === "hermes_bridge_session_messages") return { messages: [], items: [] };
      if (cmd === "list_completed_sessions") return [];
      if (cmd === "hermes_agent_cli_access") return { enabled: false };
      if (cmd === "suggest_agent_session_title") {
        return { title: prompt.slice(0, 40) || "Agent session" };
      }
      if (cmd === "hermes_bridge_filesystem_snapshot") return { entries: [] };
      throw new Error(`Unhandled QA browser invoke: ${cmd}`);
    };

    localStorage.setItem("june.onboarding.completedVersion", "999");
    localStorage.setItem("june.agent.riskAcknowledged", "true");
  };
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.killed) return;
  child.kill("SIGTERM");
  await delay(1000);
  if (child.exitCode === null) child.kill("SIGKILL");
}

function removeHermesHome(hermesHome, keepHermesHome) {
  if (!keepHermesHome && hermesHome) rmSync(hermesHome, { recursive: true, force: true });
}

function registerHermesCleanup({ hermesHome, keepHermesHome, childRef }) {
  let cleaned = false;
  const cleanup = () => {
    if (cleaned) return;
    cleaned = true;
    const child = childRef.current;
    if (child && child.exitCode === null && !child.killed) {
      try {
        child.kill("SIGTERM");
      } catch {
        // Best-effort cleanup during process exit.
      }
    }
    removeHermesHome(hermesHome, keepHermesHome);
  };
  const handleSigint = () => {
    cleanup();
    process.exit(130);
  };
  const handleSigterm = () => {
    cleanup();
    process.exit(143);
  };
  process.once("SIGINT", handleSigint);
  process.once("SIGTERM", handleSigterm);
  process.once("exit", cleanup);
  return () => {
    process.off("SIGINT", handleSigint);
    process.off("SIGTERM", handleSigterm);
    process.off("exit", cleanup);
  };
}

function describeError(error) {
  return error instanceof Error ? error.message : String(error);
}

function moveVideo(rawPath, outDir, stamp, prompt) {
  if (!rawPath) return "";
  const target = join(outDir, `${stamp}-background-agent-${slugFor(prompt)}.webm`);
  if (resolve(rawPath) === resolve(target)) return target;
  renameSync(rawPath, target);
  return target;
}

function latestWebm(outDir) {
  const entries = readdirSync(outDir)
    .filter((name) => name.endsWith(".webm"))
    .map((name) => {
      const path = join(outDir, name);
      return { path, mtimeMs: statSync(path).mtimeMs };
    })
    .sort((a, b) => b.mtimeMs - a.mtimeMs);
  return entries[0]?.path || "";
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const root = repoRoot();
  const outDir = resolve(root, args.outDir);
  mkdirSync(outDir, { recursive: true });

  const hermesCommand = resolveHermesCommand(args.hermesCommand);
  if (!hermesCommand) throw new Error("Hermes binary not found");
  const sourceHermesHome = resolveSourceHermesHome(args.sourceHermesHome);
  if (!sourceHermesHome) throw new Error("source Hermes home not found");
  const sourceConfig = join(sourceHermesHome, "config.yaml");
  if (!existsSync(sourceConfig)) throw new Error(`missing Hermes config: ${sourceConfig}`);
  const chromeExecutable = resolveChromeExecutable(args.chromeExecutable);
  if (!chromeExecutable) throw new Error("Chrome executable not found");

  const { chromium } = loadPlaywright(root);
  const viewport = parseViewport(args.viewport);
  const stamp = timestamp();
  const host = DEFAULT_HOST;
  const port = await allocatePort(host);
  const token = randomToken();
  // Hermes syncs bundled skills into its home. Keep that churn outside Vite's
  // watched workspace so a QA turn cannot trigger a full-page reload.
  const hermesHome = mkdtempSync(join(tmpdir(), "clovy-qa-hermes-"));
  const childRef = { current: null };
  const unregisterCleanup = registerHermesCleanup({
    hermesHome,
    keepHermesHome: args.keepHermesHome,
    childRef,
  });
  mkdirSync(join(hermesHome, "workspace"), { recursive: true });
  copyFileSync(sourceConfig, join(hermesHome, "config.yaml"));
  const sourceEnv = join(sourceHermesHome, ".env");
  if (existsSync(sourceEnv)) copyFileSync(sourceEnv, join(hermesHome, ".env"));

  let childLaunchError = null;
  const child = spawn(hermesCommand, ["dashboard", "--no-open", "--host", host, "--port", String(port)], {
    cwd: hermesHome,
    env: {
      ...process.env,
      HERMES_HOME: hermesHome,
      JUNE_HERMES_HOME: hermesHome,
      HERMES_DASHBOARD_SESSION_TOKEN: token,
      NO_PROXY: "127.0.0.1,localhost,::1",
      no_proxy: "127.0.0.1,localhost,::1",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  childRef.current = child;
  child.once("error", (error) => {
    childLaunchError = error;
  });
  let hermesStderr = "";
  child.stderr?.on("data", (chunk) => {
    hermesStderr = (hermesStderr + chunk.toString()).slice(-12_000);
  });
  child.stdout?.on("data", () => {});

  let browser;
  let context;
  let page;
  let rawVideoPath = "";
  let screenshotPath = "";
  let assistantText = "";
  let completed = false;
  const consoleMessages = [];

  try {
    await waitForStatus({ host, port, token, child, getChildError: () => childLaunchError });
    const connection = connectionFor({ host, port, token, hermesCommand, hermesHome });
    browser = await chromium.launch({
      executablePath: chromeExecutable,
      headless: args.headless,
    });
    context = await browser.newContext({
      viewport,
      ...(args.video ? { recordVideo: { dir: outDir, size: viewport } } : {}),
    });
    await context.addInitScript(browserInitScript(), {
      connection,
      prompt: args.prompt,
    });
    page = await context.newPage();
    page.on("console", (message) => {
      consoleMessages.push(`${message.type()}: ${message.text()}`);
    });
    page.on("pageerror", (error) => {
      consoleMessages.push(`pageerror: ${error.message}`);
    });

    await page.goto(args.url, { waitUntil: "domcontentloaded" });
    const editor = page.getByRole("textbox", { name: "Message Clovy" });
    await editor.waitFor({ timeout: 30_000 });
    await editor.fill(args.prompt);
    const startButton = page.getByRole("button", { name: "Start session" });
    if (await startButton.isDisabled()) {
      throw new Error(`Start session stayed disabled for prompt ${JSON.stringify(args.prompt)}`);
    }
    await startButton.click();
    await page.waitForFunction(
      () => window.__qaComplete === true,
      undefined,
      { timeout: args.timeoutMs },
    );
    await page.waitForFunction(
      () =>
        Array.from(document.querySelectorAll(".agent-assistant-turn-body")).some((element) => {
          const text = element.textContent?.trim();
          const style = window.getComputedStyle(element);
          const rect = element.getBoundingClientRect();
          return (
            text &&
            style.display !== "none" &&
            style.visibility !== "hidden" &&
            rect.width > 0 &&
            rect.height > 0
          );
        }),
      undefined,
      { timeout: args.timeoutMs },
    );

    const assistantTexts = await page
      .locator(".agent-assistant-turn-body")
      .allInnerTexts();
    assistantText = assistantTexts.join("\n\n").trim();
    if (!assistantText) {
      throw new Error("No visible assistant reply rendered after completion");
    }
    screenshotPath = join(outDir, `${stamp}-background-agent-${slugFor(args.prompt)}.png`);
    await page.screenshot({ path: screenshotPath, fullPage: true });
    if (args.expectText && !assistantText.includes(args.expectText)) {
      throw new Error(
        `Visible assistant reply did not contain expected text ${JSON.stringify(args.expectText)}: ` +
          JSON.stringify(assistantText),
      );
    }
    completed = true;
  } catch (error) {
    if (page) {
      const failureScreenshotPath = join(
        outDir,
        `${stamp}-background-agent-${slugFor(args.prompt)}-failure.png`,
      );
      await page.screenshot({ path: failureScreenshotPath, fullPage: true }).catch(() => {});
      if (existsSync(failureScreenshotPath)) {
        screenshotPath = failureScreenshotPath;
        console.error(`failure_screenshot_path=${failureScreenshotPath}`);
      }
      const diagnostics = await page
        .evaluate(() => ({
          invokes: window.__qaInvokes || [],
          wsEvents: window.__qaWsEvents || [],
        }))
        .catch(() => undefined);
      if (diagnostics) {
        console.error(`browser_diagnostics=${JSON.stringify(diagnostics)}`);
      }
    }
    if (consoleMessages.length) {
      console.error(`browser_console=${JSON.stringify(consoleMessages.slice(-20))}`);
    }
    if (hermesStderr) {
      console.error(`hermes_stderr=${hermesStderr}`);
    }
    throw error;
  } finally {
    const video = page?.video?.();
    if (page) await page.close().catch(() => {});
    if (context) await context.close().catch(() => {});
    if (video) {
      rawVideoPath = await video.path().catch(() => "");
      rawVideoPath = moveVideo(rawVideoPath || latestWebm(outDir), outDir, stamp, args.prompt);
    }
    if (browser) await browser.close().catch(() => {});
    await stopChild(child);
    removeHermesHome(hermesHome, args.keepHermesHome);
    unregisterCleanup();
  }

  const result = {
    completed,
    surface: "background browser",
    url: args.url,
    prompt: args.prompt,
    assistantText,
    rawVideoPath,
    screenshotPath,
    hermesHome: args.keepHermesHome ? hermesHome : "",
    consoleMessages: consoleMessages.slice(-12),
  };
  console.log(`assistant_text=${assistantText.replace(/\s+/g, " ").trim()}`);
  if (rawVideoPath) console.log(`raw_video_path=${rawVideoPath}`);
  if (screenshotPath) console.log(`screenshot_path=${screenshotPath}`);
  console.log(`background_qa_result=${JSON.stringify(result)}`);

  if (!completed) {
    if (hermesStderr) console.error(hermesStderr);
    process.exit(1);
  }
}

main().catch((error) => {
  console.error(describeError(error));
  process.exit(1);
});
