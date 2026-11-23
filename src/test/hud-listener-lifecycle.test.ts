import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type TauriListener = (event: { payload: unknown }) => unknown;

const mocks = vi.hoisted(() => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  listeners: new Map<string, TauriListener>(),
  unlistenHandles: [] as ReturnType<typeof vi.fn>[],
  listen: vi.fn((event: string, listener: TauriListener) => {
    mocks.listeners.set(event, listener);
    const unlisten = vi.fn();
    mocks.unlistenHandles.push(unlisten);
    return Promise.resolve(unlisten);
  }),
  hide: vi.fn().mockResolvedValue(undefined),
  startDragging: vi.fn().mockResolvedValue(undefined),
  getCurrentWindow: vi.fn(() => ({
    hide: mocks.hide,
    startDragging: mocks.startDragging,
  })),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolver) => {
    resolve = resolver;
  });
  return { promise, resolve };
}

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: mocks.getCurrentWindow,
}));

describe("HUD listener lifecycle", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    mocks.listeners.clear();
    mocks.unlistenHandles.length = 0;
    mocks.invoke.mockResolvedValue(undefined);
    mocks.getCurrentWindow.mockImplementation(() => ({
      hide: mocks.hide,
      startDragging: mocks.startDragging,
    }));
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    document.body.innerHTML = `
      <main id="agent-hud"></main>
      <main id="hud"></main>
      <main id="mhud">
        <span class="mhud-bar"></span>
        <button id="mhud-end-stop" type="button">
          Stop now
          <span id="mhud-end-seconds">15s</span>
        </button>
        <button id="mhud-end-keep" type="button">Keep recording</button>
      </main>
    `;
  });

  afterEach(() => {
    window.dispatchEvent(new Event("pagehide"));
    Reflect.deleteProperty(window, "__TAURI_INTERNALS__");
  });

  it("releases every Tauri listener on beforeunload", async () => {
    // Each HUD window also follows the interface language over one Tauri
    // listener (subscribeInterfaceLocaleAcrossWindows).
    await import("../agent-hud");
    await vi.waitFor(() => {
      expect(mocks.unlistenHandles).toHaveLength(9);
    });

    await import("../hud");
    await vi.waitFor(() => {
      expect(mocks.unlistenHandles).toHaveLength(18);
    });

    await import("../meeting-hud");
    await vi.waitFor(() => {
      // Main's three meeting-hud listeners plus this branch's
      // meeting-end-state listener, plus the interface-locale listener.
      expect(mocks.unlistenHandles).toHaveLength(23);
    });

    window.dispatchEvent(new Event("beforeunload"));

    await vi.waitFor(() => {
      for (const unlisten of mocks.unlistenHandles) {
        expect(unlisten).toHaveBeenCalledOnce();
      }
    });
  });

  it("shows the meeting-end countdown and wires both safety actions", async () => {
    mocks.getCurrentWindow.mockImplementationOnce(() => {
      throw new Error("Mocked Tauri bridge has no window metadata");
    });
    await import("../meeting-hud");
    await vi.waitFor(() => {
      expect(mocks.listeners.has("meeting-end-state-event")).toBe(true);
      expect((window as unknown as Record<string, unknown>).__recordingHud).toBeTypeOf("function");
    });

    const countdown = {
      sessionId: "meeting-session",
      phase: "countdown",
      expiresAtMs: Date.now() + 15_000,
    };
    mocks.listeners.get("meeting-end-state-event")?.({ payload: countdown });

    const pill = document.querySelector<HTMLElement>("#mhud");
    const keep = document.querySelector<HTMLButtonElement>("#mhud-end-keep");
    const stop = document.querySelector<HTMLButtonElement>("#mhud-end-stop");
    expect(pill?.dataset.mode).toBe("meeting-end");
    // The Stop button's drain holds the freshly computed remaining fraction,
    // and the seconds suffix mirrors it.
    const remaining = Number.parseFloat(
      stop?.style.getPropertyValue("--meeting-end-remaining") ?? "",
    );
    expect(remaining).toBeGreaterThan(0.9);
    expect(remaining).toBeLessThanOrEqual(1);
    expect(document.querySelector("#mhud-end-seconds")?.textContent).toBe("15s");
    expect(keep?.disabled).toBe(false);
    expect(stop?.disabled).toBe(false);

    keep?.click();
    await vi.waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("keep_meeting_recording", {
        sessionId: "meeting-session",
      });
    });
    expect(pill?.dataset.mode).toBe("meeting-end");

    mocks.listeners.get("meeting-end-state-event")?.({ payload: countdown });
    stop?.click();
    await vi.waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_end_finish_request", {
        sessionId: "meeting-session",
      });
    });
    expect(pill?.dataset.mode).toBe("meeting-end");

    mocks.listeners.get("meeting-end-state-event")?.({
      payload: { sessionId: "meeting-session", phase: "tracking" },
    });
    expect(pill?.dataset.mode).toBeUndefined();

    window.dispatchEvent(new Event("pagehide"));
    expect((window as unknown as Record<string, unknown>).__recordingHud).toBeUndefined();
  });

  it("disposes recording demo timers, local buttons, and its console hook", async () => {
    const setInterval = vi.spyOn(window, "setInterval");
    const clearInterval = vi.spyOn(window, "clearInterval");
    const { registerRecordingHudDemo } = await import("../lib/recording-hud-demo");
    const demo = registerRecordingHudDemo({ local: true });
    const hook = (window as unknown as Record<string, unknown>).__recordingHud as (
      state: "recording",
    ) => string;

    hook("recording");
    expect(setInterval).toHaveBeenCalled();
    clearInterval.mockClear();

    demo.dispose();

    expect(clearInterval).toHaveBeenCalled();
    expect((window as unknown as Record<string, unknown>).__recordingHud).toBeUndefined();
    setInterval.mockClear();
    document.querySelector<HTMLButtonElement>("#mhud-end-keep")?.click();
    document.querySelector<HTMLButtonElement>("#mhud-end-stop")?.click();
    expect(setInterval).not.toHaveBeenCalled();

    setInterval.mockRestore();
    clearInterval.mockRestore();
  });

  it("does not let the initial meeting-end read overwrite a newer HUD event", async () => {
    const initialStatusRead = deferred<{
      sessionId: string;
      phase: "countdown";
      expiresAtMs: number;
    } | null>();
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "pending_meeting_end_status") return initialStatusRead.promise;
      return Promise.resolve(undefined);
    });

    await import("../meeting-hud");
    await vi.waitFor(() => {
      expect(mocks.listeners.has("meeting-end-state-event")).toBe(true);
    });
    mocks.listeners.get("meeting-end-state-event")?.({
      payload: { sessionId: "meeting-session", phase: "finishQueued" },
    });
    initialStatusRead.resolve({
      sessionId: "meeting-session",
      phase: "countdown",
      expiresAtMs: Date.now() + 15_000,
    });
    await initialStatusRead.promise;
    await Promise.resolve();

    expect(document.querySelector<HTMLElement>("#mhud")?.dataset.mode).toBeUndefined();
  });

  it("releases a Tauri listener that resolves after beforeunload", async () => {
    const { createHudLifecycle } = await import("../lib/hud-lifecycle");
    const lifecycle = createHudLifecycle();
    const unlisten = vi.fn();
    let resolveUnlisten: ((handle: () => void) => void) | undefined;

    lifecycle.trackUnlisten(
      new Promise((resolve) => {
        resolveUnlisten = resolve;
      }),
    );
    window.dispatchEvent(new Event("beforeunload"));
    resolveUnlisten?.(unlisten);
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledOnce();
  });
});
