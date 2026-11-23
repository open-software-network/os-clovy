import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import markup from "../../hud.html?raw";

type Listener = (event: { payload: unknown }) => unknown;
const mocks = vi.hoisted(() => ({
  listeners: new Map<string, Listener>(),
  invoke: vi.fn(),
  hide: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn((name: string, listener: Listener) => {
    mocks.listeners.set(name, listener);
    return Promise.resolve(vi.fn());
  }),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ hide: mocks.hide, startDragging: vi.fn() }),
}));
const a = { app: "Finder", bundleIdentifier: "com.apple.finder", processIdentifier: 100 };
const b = { app: "Terminal", bundleIdentifier: "com.apple.Terminal", processIdentifier: 200 };
const c = { app: "Other", bundleIdentifier: "test.other", processIdentifier: 300 };
const iconA = "data:image/png;base64,Zmlyc3Q=";
const iconB = "data:image/png;base64,c2Vjb25k";
function target() {
  return document.querySelector<HTMLElement>("#hud-target");
}
function img() {
  return target()?.querySelector("img");
}
async function emit(type: string, payload: Record<string, unknown> = {}) {
  await mocks.listeners.get("dictation-event")?.({ payload: { type, payload } });
  await Promise.resolve();
}
async function start(takeId = "take-1", app: unknown = a) {
  await emit("listening_started", { takeId, target: app });
}
function expectTarget(app: string, icon: string) {
  expect(target()).toHaveAttribute("aria-label", `Dictation destination: ${app}`);
  expect(target()).toHaveAttribute("title", `Dictation destination: ${app}`);
  expect(img()).toHaveAttribute("src", icon);
  expect(img()).not.toHaveAttribute("hidden");
}

describe("dictation target indicator", () => {
  beforeEach(async () => {
    vi.resetModules();
    vi.clearAllMocks();
    mocks.listeners.clear();
    localStorage.clear();
    document.body.innerHTML = new DOMParser().parseFromString(markup, "text/html").body.innerHTML;
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    mocks.invoke.mockImplementation((command, args) =>
      Promise.resolve(
        command === "activity_app_icon"
          ? args.request.bundleId === a.bundleIdentifier
            ? iconA
            : iconB
          : undefined,
      ),
    );
    await import("../hud");
    await Promise.resolve();
  });
  afterEach(() => {
    window.dispatchEvent(new Event("pagehide"));
    vi.unstubAllGlobals();
  });

  it("shows A from listening, follows B, and keeps pinned B through processing, paste and error", async () => {
    await start();
    expectTarget("Finder", iconA);
    await emit("focus_target", { takeId: "take-1", target: b });
    expectTarget("Terminal", iconB);
    await emit("finalizing_transcript", { takeId: "take-1", target: b });
    await emit("focus_target", { takeId: "take-1", target: c });
    expect(document.querySelector("#hud")).toHaveAttribute("data-state", "transcribing");
    expectTarget("Terminal", iconB);
    await emit("paste_target", { takeId: "take-1", app: "Terminal", target: b });
    expectTarget("Terminal", iconB);
    await emit("error", {
      takeId: "take-1",
      code: "paste_target_unavailable",
      message: "App closed",
    });
    expectTarget("Terminal", iconB);
  });

  it("uses the stop snapshot even if the last candidate event was missed", async () => {
    await start();
    await emit("finalizing_transcript", { takeId: "take-1", target: b });
    expectTarget("Terminal", iconB);
  });

  it("late icon A cannot overwrite B or a new take", async () => {
    let resolveA: (value: string) => void = () => {};
    mocks.invoke.mockImplementation((command, args) => {
      if (command !== "activity_app_icon") return Promise.resolve();
      return args.request.bundleId === a.bundleIdentifier
        ? new Promise<string>((resolve) => {
            resolveA = resolve;
          })
        : Promise.resolve(iconB);
    });
    await start();
    await emit("focus_target", { takeId: "take-1", target: b });
    expectTarget("Terminal", iconB);
    await emit("recording_discarded", { takeId: "take-1" });
    await start("take-2", b);
    resolveA(iconA);
    await Promise.resolve();
    expectTarget("Terminal", iconB);
  });

  it("rejects obsolete and preserveActiveTake updates, and does not resolve per audio level", async () => {
    await start();
    await emit("focus_target", { takeId: "old", target: b });
    await emit("focus_target", { takeId: "take-1", target: b, preserveActiveTake: true });
    await emit("focus_target", { target: b });
    await emit("audio_level", { takeId: "take-1", level: "0.8" });
    expectTarget("Finder", iconA);
    expect(mocks.invoke.mock.calls.filter(([cmd]) => cmd === "activity_app_icon")).toHaveLength(1);
  });

  it("clears a terminal take and never revives it on a late target event", async () => {
    await start();
    await emit("paste_completed", { takeId: "take-1" });
    await emit("focus_target", { takeId: "take-1", target: b });
    await vi.waitFor(() =>
      expect(document.querySelector("#hud")).toHaveAttribute("data-state", "idle"),
    );
    expect(img()).not.toHaveAttribute("src");
    await start("take-2", null);
    expect(target()).toHaveAttribute("aria-label", "Dictation destination unidentified");
    expect(img()).toHaveAttribute("hidden");
    await emit("finalizing_transcript", { takeId: "take-1", target: b });
    expect(document.querySelector("#hud")).toHaveAttribute("data-state", "listening");
  });

  it("uses an honest fallback for old events and failed or broken icons without blocking cancel", async () => {
    await emit("listening_started");
    expect(target()).toHaveAttribute("title", "Dictation destination unidentified");
    expect(img()).toHaveAttribute("hidden");
    mocks.invoke.mockImplementation((command) =>
      command === "activity_app_icon"
        ? Promise.reject(new Error("unavailable"))
        : Promise.resolve(),
    );
    await start();
    expect(img()).not.toHaveAttribute("src");
    expect(img()).toHaveAttribute("hidden");
    await start("take-2", b);
    document.querySelector<HTMLButtonElement>("#hud-cancel")?.click();
    await vi.waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("dictation_helper_command", {
        command: { type: "discard_recording", takeId: "take-2" },
      }),
    );
  });

  it("hides a broken image and retains its known destination", async () => {
    await start();
    img()?.dispatchEvent(new Event("error"));
    expect(img()).toHaveAttribute("hidden");
    expect(img()).not.toHaveAttribute("src");
    expect(target()?.getAttribute("aria-label")).toContain("Finder");
  });

  it("does not leave a target in the meeting prompt", async () => {
    await start();
    await emit("recording_discarded", { takeId: "take-1" });
    await vi.waitFor(() =>
      expect(document.querySelector("#hud")).toHaveAttribute("data-state", "idle"),
    );
    const { markOnboardingComplete } = await import("../lib/onboarding");
    markOnboardingComplete();
    await mocks.listeners.get("meeting-detection-event")?.({
      payload: { type: "meeting_detected", payload: { appLabels: ["Meeting app"] } },
    });
    expect(target()).toHaveAttribute("hidden");
    expect(img()).not.toHaveAttribute("src");
  });
});
