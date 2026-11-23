import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_SESSION_STATUS_EVENT } from "../lib/agent-events";
import { markOnboardingComplete, resetOnboardingForReplay } from "../lib/onboarding";

type TauriListener = (event: { payload: unknown }) => unknown;
const originalNavigatorPlatform = navigator.platform;

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, TauriListener>(),
  hide: vi.fn().mockResolvedValue(undefined),
  invoke: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn((event: string, listener: TauriListener) => {
    mocks.listeners.set(event, listener);
    return Promise.resolve(vi.fn());
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    hide: mocks.hide,
  }),
}));

describe("meeting detection HUD", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
    mocks.hide.mockResolvedValue(undefined);
    mocks.invoke.mockResolvedValue(undefined);
    mocks.listeners.clear();
    document.body.innerHTML = hudMarkup();
  });

  afterEach(() => {
    window.dispatchEvent(new Event("pagehide"));
    delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: originalNavigatorPlatform,
    });
    vi.useRealTimers();
  });

  it("offers detected meeting video off by default and queues the selected window", async () => {
    Object.defineProperty(navigator, "platform", { configurable: true, value: "MacIntel" });
    const window = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Review",
    };
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "detect_meeting_window" ? window : undefined,
    );
    await loadHud();
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Chrome"] },
    });
    const toggle = document.querySelector<HTMLButtonElement>("#hud-meeting-video");
    expect(toggle).not.toHaveAttribute("hidden");
    expect(toggle).toHaveAttribute("aria-checked", "false");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Chrome: Meet - Review");
    toggle?.click();
    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();
    await vi.waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_start_request", {
        video: { windowId: 42 },
      }),
    );
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "detect_meeting_window"),
    ).toHaveLength(1);
  });

  it("queues the selected video immediately without waiting for another detection", async () => {
    const selectedWindow = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Review",
    };
    let detections = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "detect_meeting_window") return Promise.resolve(undefined);
      detections += 1;
      return detections === 1 ? Promise.resolve(selectedWindow) : new Promise(() => {});
    });
    await loadHud();
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Chrome"] },
    });
    document.querySelector<HTMLButtonElement>("#hud-meeting-video")?.click();
    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();
    expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_start_request", {
      video: { windowId: 42 },
    });
    expect(detections).toBe(1);
  });

  it("ignores delayed window detection from a prompt that was cleared and reopened", async () => {
    const older = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Earlier call",
    };
    const current = {
      windowId: 99,
      appName: "Teams",
      bundleId: "com.microsoft.teams2",
      title: "Weekly meeting",
    };
    let resolveOlder: ((window: typeof older) => void) | undefined;
    const delayedDetection = new Promise<typeof older>((resolve) => {
      resolveOlder = resolve;
    });
    let detectionCount = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "detect_meeting_window") return Promise.resolve(undefined);
      detectionCount += 1;
      return detectionCount === 1 ? delayedDetection : Promise.resolve(current);
    });
    await loadHud();
    const olderPrompt = emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Chrome"] },
    });
    await vi.waitFor(() => expect(detectionCount).toBe(1));
    await emit("meeting-detection-event", { type: "meeting_cleared" });
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Teams"] },
    });
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Teams: Weekly meeting");

    resolveOlder?.(older);
    await olderPrompt;
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Teams: Weekly meeting");
    document.querySelector<HTMLButtonElement>("#hud-meeting-video")?.click();
    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();
    await vi.waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_start_request", {
        video: { windowId: 99 },
      }),
    );
  });

  it.each([
    "null",
    "error",
  ] as const)("retries an initial %s window result on a heartbeat without enabling video", async (result) => {
    const meetingWindow = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Review",
    };
    let detections = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "detect_meeting_window") return Promise.resolve(undefined);
      detections += 1;
      if (detections === 1)
        return result === "error"
          ? Promise.reject(new Error("lookup failed"))
          : Promise.resolve(null);
      return Promise.resolve(meetingWindow);
    });
    await loadHud();
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Chrome"] },
    });
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("hidden");
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Chrome"] },
    });
    expect(detections).toBe(2);
    expect(document.querySelector("#hud-meeting-video")).not.toHaveAttribute("hidden");
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("aria-checked", "false");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Chrome: Meet - Review");
    expect(
      mocks.invoke.mock.calls.some(([command]) => command === "queue_meeting_start_request"),
    ).toBe(false);
  });

  it("keeps only one retry pending during overlapping heartbeats", async () => {
    const meetingWindow = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Review",
    };
    let resolveRetry: ((window: typeof meetingWindow) => void) | undefined;
    const pendingRetry = new Promise<typeof meetingWindow>((resolve) => {
      resolveRetry = resolve;
    });
    let detections = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "detect_meeting_window") return Promise.resolve(undefined);
      detections += 1;
      return detections === 1 ? Promise.resolve(null) : pendingRetry;
    });
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    const retry = emit("meeting-detection-event", { type: "meeting_detected" });
    await vi.waitFor(() => expect(detections).toBe(2));
    await emit("meeting-detection-event", { type: "meeting_detected" });
    await emit("meeting-detection-event", { type: "meeting_detected" });
    expect(detections).toBe(2);
    resolveRetry?.(meetingWindow);
    await retry;
    expect(document.querySelector("#hud-meeting-video")).not.toHaveAttribute("hidden");
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("aria-checked", "false");
  });

  it("cannot clear a newer prompt's pending lookup when an older lookup completes", async () => {
    const olderWindow = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Earlier call",
    };
    const currentWindow = {
      windowId: 99,
      appName: "Teams",
      bundleId: "com.microsoft.teams2",
      title: "Weekly meeting",
    };
    let resolveOlder: ((window: typeof olderWindow) => void) | undefined;
    let resolveCurrent: ((window: typeof currentWindow) => void) | undefined;
    const pendingOlder = new Promise<typeof olderWindow>((resolve) => {
      resolveOlder = resolve;
    });
    const pendingCurrent = new Promise<typeof currentWindow>((resolve) => {
      resolveCurrent = resolve;
    });
    let detections = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "detect_meeting_window") return Promise.resolve(undefined);
      detections += 1;
      return detections === 1 ? pendingOlder : pendingCurrent;
    });
    await loadHud();
    const olderPrompt = emit("meeting-detection-event", { type: "meeting_detected" });
    await vi.waitFor(() => expect(detections).toBe(1));
    await emit("meeting-detection-event", { type: "meeting_cleared" });
    const currentPrompt = emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Teams"] },
    });
    await vi.waitFor(() => expect(detections).toBe(2));
    resolveOlder?.(olderWindow);
    await olderPrompt;
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Teams");
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("hidden");
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Teams"] },
    });
    expect(detections).toBe(2);
    resolveCurrent?.(currentWindow);
    await currentPrompt;
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Teams: Weekly meeting");
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("aria-checked", "false");
  });

  it("preserves an opted-in target through heartbeats without looking up another window", async () => {
    const selectedWindow = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Review",
    };
    let detections = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "detect_meeting_window") return Promise.resolve(undefined);
      detections += 1;
      return Promise.resolve(
        detections === 1
          ? selectedWindow
          : { ...selectedWindow, windowId: 99, title: "Other meeting" },
      );
    });
    await loadHud();
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Chrome"] },
    });
    document.querySelector<HTMLButtonElement>("#hud-meeting-video")?.click();
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Teams"] },
    });
    expect(detections).toBe(1);
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Chrome: Meet - Review");
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("aria-checked", "true");
    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();
    expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_start_request", {
      video: { windowId: 42 },
    });
  });

  it("queues audio immediately while a retry is pending and ignores its later success", async () => {
    const meetingWindow = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Review",
    };
    let resolveRetry: ((window: typeof meetingWindow) => void) | undefined;
    const pendingRetry = new Promise<typeof meetingWindow>((resolve) => {
      resolveRetry = resolve;
    });
    let detections = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "detect_meeting_window") return Promise.resolve(undefined);
      detections += 1;
      return detections === 1 ? Promise.resolve(null) : pendingRetry;
    });
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    const retry = emit("meeting-detection-event", { type: "meeting_detected" });
    await vi.waitFor(() => expect(detections).toBe(2));
    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();
    expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_start_request");
    expect(detections).toBe(2);
    resolveRetry?.(meetingWindow);
    await retry;
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("hidden");
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("aria-checked", "false");
  });

  it("keeps the prompt audio-only when window detection is denied", async () => {
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "detect_meeting_window") throw new Error("permission_denied");
      return undefined;
    });
    await loadHud();
    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { appLabels: ["Zoom"] },
    });
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("hidden");
    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();
    await vi.waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_start_request"),
    );
  });

  it("shows the take notes prompt when a meeting is detected", async () => {
    await loadHud();

    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { activeProcessCount: 1, appLabels: ["Zoom"] },
    });

    expect(hudElement().dataset.state).toBe("meeting");
    expect(document.querySelector("#hud-meeting-label")).toHaveTextContent("Meeting detected");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Zoom");
    expect(document.querySelector("#hud-meeting-start")).toHaveTextContent("Record");
    expect(hudShowCalls()).toBe(1);
    expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_set_stop_bounds", {
      rect: null,
    });
    // The window is resized to the measured pill (jsdom rects are zero)
    // plus the meeting card's transparent shadow gutter on each side.
    expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_set_size", {
      width: 36,
      height: 36,
      animate: false,
    });
  });

  it("delays the meeting prompt while onboarding is active", async () => {
    resetOnboardingForReplay();
    await loadHud();

    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { activeProcessCount: 1, appLabels: ["Zoom"] },
    });

    expect(hudElement().dataset.state).toBe("idle");
    expect(hudShowCalls()).toBe(0);
    expect(mocks.hide).toHaveBeenCalledOnce();

    markOnboardingComplete();
    await Promise.resolve();
    await Promise.resolve();

    expect(hudElement().dataset.state).toBe("meeting");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Zoom");
    await vi.waitFor(() => expect(hudShowCalls()).toBe(1));
  });

  it("drops a delayed meeting prompt if the meeting clears during onboarding", async () => {
    resetOnboardingForReplay();
    await loadHud();

    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { activeProcessCount: 1, appLabels: ["Zoom"] },
    });
    await emit("meeting-detection-event", { type: "meeting_cleared" });

    markOnboardingComplete();
    await Promise.resolve();
    await Promise.resolve();

    expect(hudElement().dataset.state).toBe("idle");
    expect(hudShowCalls()).toBe(0);
  });

  it("joins multiple app labels and falls back when none arrive", async () => {
    await loadHud();

    await emit("meeting-detection-event", {
      type: "meeting_detected",
      payload: { activeProcessCount: 2, appLabels: ["Zoom", "Chrome"] },
    });
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Zoom, Chrome");

    await emit("meeting-detection-event", { type: "meeting_detected" });
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Microphone in use");
  });

  it("accepts local window events from the demo driver", async () => {
    await loadHud();

    window.dispatchEvent(
      new CustomEvent("meeting-detection-event", {
        detail: {
          type: "meeting_detected",
          payload: { activeProcessCount: 1, appLabels: ["Teams"] },
        },
      }),
    );
    await Promise.resolve();

    expect(hudElement().dataset.state).toBe("meeting");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Teams");
  });

  it("durably queues a start transcription request before hiding", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    mocks.invoke.mockClear();

    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();

    await Promise.resolve();
    expect(mocks.invoke).toHaveBeenCalledWith("queue_meeting_start_request");
    // A bounded advance, not runAllTimersAsync: jsdom drives rAF off the
    // faked setTimeout while the alpha ramp measures real time, so running
    // "all" timers re-queues the ramp until sinon's 10000-timer abort.
    await vi.advanceTimersByTimeAsync(220);
    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.disabled).toBe(false);
    vi.useRealTimers();
  });

  it("keeps the meeting prompt available when queuing fails", async () => {
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "queue_meeting_start_request") {
        return Promise.reject(new Error("native queue unavailable"));
      }
      return Promise.resolve(undefined);
    });
    mocks.hide.mockClear();

    document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.click();

    await vi.waitFor(() =>
      expect(document.querySelector<HTMLButtonElement>("#hud-meeting-start")?.disabled).toBe(false),
    );
    expect(hudElement().dataset.state).toBe("meeting");
    expect(mocks.hide).not.toHaveBeenCalled();
    await emit("meeting-detection-event", { type: "meeting_cleared" });
  });

  it("dismisses the prompt and stays quiet until the meeting clears", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });

    document.querySelector<HTMLButtonElement>("#hud-meeting-dismiss")?.click();
    await Promise.resolve();

    expect(hudElement().dataset.state).toBe("exiting");
    await vi.advanceTimersByTimeAsync(220);
    expect(mocks.hide).toHaveBeenCalledOnce();

    // Detection heartbeats keep arriving while the same meeting is live; the
    // dismissed prompt must not come back for any of them.
    mocks.invoke.mockClear();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    expect(hudShowCalls()).toBe(0);

    // The next meeting prompts again.
    await emit("meeting-detection-event", { type: "meeting_cleared" });
    await emit("meeting-detection-event", { type: "meeting_detected" });
    expect(hudElement().dataset.state).toBe("meeting");
    expect(hudShowCalls()).toBe(1);
  });

  it("preserves the meeting prompt layout during native meeting exit", async () => {
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });

    let resolveExit: (() => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_hud_exit") {
        return new Promise<void>((resolve) => {
          resolveExit = resolve;
        });
      }
      return Promise.resolve(undefined);
    });

    document.querySelector<HTMLButtonElement>("#hud-meeting-dismiss")?.click();
    await Promise.resolve();

    expect(hudElement().dataset.state).toBe("exiting");
    expect(hudElement().dataset.exitState).toBe("meeting");
    expect(hudElement().classList.contains("hud-exit-up")).toBe(true);

    resolveExit?.();

    await vi.waitFor(() => expect(mocks.hide).toHaveBeenCalledOnce());
    await vi.waitFor(() => expect(hudElement().dataset.state).toBe("idle"));
    expect(hudElement().dataset.exitState).toBeUndefined();
  });

  it("ignores a dismiss click outside the meeting prompt state", async () => {
    await loadHud();
    hudElement().dataset.state = "listening";
    mocks.hide.mockClear();

    document.querySelector<HTMLButtonElement>("#hud-meeting-dismiss")?.click();
    await Promise.resolve();

    expect(hudElement().dataset.state).toBe("listening");
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("clears the prompt when microphone usage stops", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });

    await emit("meeting-detection-event", { type: "meeting_cleared" });

    // The meeting exit is native motion (dictation_hud_exit resolves when
    // the window is hidden), so the pill lands back on idle immediately
    // under the mocked invoke.
    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(hudElement().dataset.state).toBe("idle");
    expect(chromeCalls()).toEqual([]);
  });

  it("hides and suppresses the prompt after 30 seconds without a click", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });

    await vi.advanceTimersByTimeAsync(29_999);
    expect(hudElement().dataset.state).toBe("meeting");
    expect(mocks.hide).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    // The native exit (mocked invoke) completes immediately.
    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(hudElement().dataset.state).toBe("idle");

    mocks.invoke.mockClear();
    await emit("meeting-detection-event", { type: "meeting_detected" });

    expect(hudShowCalls()).toBe(0);
  });

  it("puts a re-shown window back down when a heartbeat arrives while suppressed", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    await vi.advanceTimersByTimeAsync(30_220);
    expect(mocks.hide).toHaveBeenCalledOnce();

    // While the prompt is suppressed, Rust may have re-shown the native
    // window before this event arrives. The pill renders nothing, so the HUD
    // must answer by hiding the window — a bare frosted window is otherwise
    // stuck on screen as an undraggable gray bar.
    mocks.invoke.mockClear();
    mocks.hide.mockClear();
    await emit("meeting-detection-event", { type: "meeting_detected" });

    expect(hudShowCalls()).toBe(0);
    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(hudElement().dataset.state).not.toBe("meeting");
  });

  it("returns the pill to idle once the exit completes", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    await emit("meeting-detection-event", { type: "meeting_cleared" });

    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(hudElement().dataset.state).toBe("idle");
  });

  it("hides a contentless window when the meeting clears", async () => {
    await loadHud();

    await emit("meeting-detection-event", { type: "meeting_cleared" });

    expect(mocks.hide).toHaveBeenCalledOnce();
  });

  it("allows the prompt again after a timed-out meeting clears", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("meeting-detection-event", { type: "meeting_detected" });
    await vi.advanceTimersByTimeAsync(30_160);
    await emit("meeting-detection-event", { type: "meeting_cleared" });

    mocks.invoke.mockClear();
    await emit("meeting-detection-event", { type: "meeting_detected" });

    expect(hudElement().dataset.state).toBe("meeting");
    expect(hudShowCalls()).toBe(1);
  });

  it("does not override an active dictation HUD state", async () => {
    await loadHud();
    hudElement().dataset.state = "transcribing";
    mocks.invoke.mockClear();

    await emit("meeting-detection-event", { type: "meeting_detected" });

    expect(hudElement().dataset.state).toBe("transcribing");
    expect(hudShowCalls()).toBe(0);
  });

  it("surfaces a long dictation notice", async () => {
    vi.useFakeTimers();
    await loadHud();

    await emit("dictation-event", { type: "finalizing_transcript" });

    expect(hudElement().dataset.state).toBe("transcribing");
    expect(document.querySelector("#hud-status")).toHaveTextContent("Transcribing");

    await vi.advanceTimersByTimeAsync(6_000);

    expect(hudElement().dataset.state).toBe("transcribing");
    expect(document.querySelector("#hud-status")).toHaveTextContent("Still transcribing");
  });

  it("re-arms the long dictation notice on the next dictation's clock", async () => {
    vi.useFakeTimers();
    await loadHud();

    // A quick dictation: it leaves the notice window long before 6s.
    await emit("dictation-event", { type: "finalizing_transcript" });
    await vi.advanceTimersByTimeAsync(1_000);
    await emit("dictation-event", {
      type: "final_transcript",
      payload: { text: "hi" },
    });
    expect(document.querySelector("#hud-status")).not.toHaveTextContent("Still transcribing");
    await emit("dictation-event", { type: "paste_completed" });

    // A second dictation starts 2s after the first armed its timer. If the
    // first timer leaked, it fires 4s from here and mislabels this dictation
    // as slow, and this dictation never arms a timer of its own.
    await vi.advanceTimersByTimeAsync(1_000);
    await emit("dictation-event", { type: "finalizing_transcript" });

    await vi.advanceTimersByTimeAsync(4_500);
    expect(hudElement().dataset.state).toBe("transcribing");
    expect(document.querySelector("#hud-status")).toHaveTextContent("Transcribing");
    expect(document.querySelector("#hud-status")).not.toHaveTextContent("Still transcribing");

    // 6s after *this* dictation began, the notice appears.
    await vi.advanceTimersByTimeAsync(1_500);
    expect(document.querySelector("#hud-status")).toHaveTextContent("Still transcribing");
  });

  it("silently dismisses the HUD when nothing was recorded", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "finalizing_transcript" });
    mocks.invoke.mockClear();

    await emit("dictation-event", {
      type: "error",
      payload: {
        code: "missing_recording",
        message: "No recorded audio was available to transcribe.",
        silent: true,
      },
    });

    // No "Nothing recorded" toast: a silent end takes the normal exit fade
    // and says nothing, never surfacing the error treatment.
    expect(hudElement().dataset.state).toBe("exiting");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent("");
    expect(hudShowCalls()).toBe(0);

    // Drain the in-flight exit. Its fallback timeout dies with the fake
    // clock, but the rAF alpha ramp keeps running on real time after this
    // test ends and would land its hide() inside the next test's counts.
    await vi.advanceTimersByTimeAsync(320);
    expect(mocks.hide).toHaveBeenCalledOnce();
  });

  it("quietly dismisses the HUD when stop finds nothing listening", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockClear();

    await emit("dictation-event", {
      type: "error",
      payload: {
        code: "not_listening",
        message: "Dictation is not listening.",
      },
    });

    // The desired end state (not listening) already holds, so no toast:
    // the pill takes the same quiet exit as a silent end.
    expect(hudElement().dataset.state).toBe("exiting");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent("");

    await vi.advanceTimersByTimeAsync(320);
    expect(mocks.hide).toHaveBeenCalledOnce();
  });

  it("preserves the actionable start error when key-up finds nothing listening", async () => {
    vi.useFakeTimers();
    await loadHud();

    let resolvePlacement: ((placement: string) => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_hud_preferred_error_placement") {
        return new Promise<string>((resolve) => {
          resolvePlacement = resolve;
        });
      }
      return Promise.resolve(undefined);
    });

    const startError = emit("dictation-event", {
      type: "error",
      payload: {
        code: "microphone_permission_missing",
        message: "Microphone permission is required.",
      },
    });

    // The key-up error can arrive before native window placement resolves.
    expect(hudElement().dataset.state).toBe("error");
    await emit("dictation-event", {
      type: "error",
      payload: {
        code: "not_listening",
        message: "Dictation is not listening.",
      },
    });

    expect(hudElement().dataset.state).toBe("error");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent(
      "Microphone permission is required.",
    );
    mocks.hide.mockClear();

    resolvePlacement?.("below");
    await startError;

    expect(hudElement().dataset.state).toBe("error");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent(
      "Microphone permission is required.",
    );
    expect(mocks.hide).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1_800);
    await vi.advanceTimersByTimeAsync(320);
    expect(mocks.hide).toHaveBeenCalledOnce();
  });

  it("does not let a delayed start error hide a newer listening state", async () => {
    vi.useFakeTimers();
    await loadHud();
    mocks.hide.mockClear();

    let resolvePlacement: ((placement: string) => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_hud_preferred_error_placement") {
        return new Promise<string>((resolve) => {
          resolvePlacement = resolve;
        });
      }
      return Promise.resolve(undefined);
    });

    const startError = emit("dictation-event", {
      type: "error",
      payload: {
        code: "microphone_permission_missing",
        message: "Microphone permission is required.",
      },
    });
    expect(hudElement().dataset.state).toBe("error");

    void emit("dictation-event", { type: "listening_started" });
    expect(hudElement().dataset.state).toBe("listening");

    resolvePlacement?.("below");
    await startError;
    await vi.advanceTimersByTimeAsync(2_200);

    expect(hudElement().dataset.state).toBe("listening");
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("uses agent-style frostless chrome for the compact listening pill", async () => {
    await loadHud();

    await emit("dictation-event", { type: "listening_started" });

    expect(hudElement().dataset.state).toBe("listening");
    expect(chromeCalls()).toEqual([]);
    // 36 = zero-rect pill + the shadow gutter on each side; the height adds
    // the Esc tooltip's mirrored slot (2 × the --sp-2 gap; the tip itself
    // measures zero in jsdom), reserved for the whole listening session so
    // the hover reveal never has to resize a visible window.
    expect(mocks.invoke).toHaveBeenCalledWith(
      "dictation_hud_set_size",
      expect.objectContaining({ width: 36, height: 48 }),
    );
  });

  it("fades the processing HUD after paste input is submitted", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", {
      type: "paste_target",
      payload: { app: "Notes" },
    });
    mocks.invoke.mockClear();

    await emit("dictation-event", {
      type: "paste_completed",
      payload: { inputSubmitted: true, deliveryConfirmed: false, eventsSubmitted: 4 },
    });

    expect(hudElement().dataset.state).toBe("exiting");
    expect(hudElement().dataset.exitState).toBe("pasting");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent("");

    await vi.advanceTimersByTimeAsync(320);

    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(hudElement().dataset.state).toBe("idle");
    expect(hudElement().dataset.exitState).toBeUndefined();
  });

  it("switches from listening to transcribing without a morph flash", async () => {
    // jsdom reports a zero-width pill, so collapseToSpinner takes its atomic
    // fallback: the same snap-resize-then-show the processing chain uses.
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockClear();

    let resolveResize: (() => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_hud_set_size") {
        return new Promise<void>((resolve) => {
          resolveResize = resolve;
        });
      }
      return Promise.resolve(undefined);
    });

    const finalizing = emit("dictation-event", {
      type: "finalizing_transcript",
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(hudElement().dataset.state).toBe("transcribing");
    expect(sizeCalls()).toHaveLength(1);
    expect(sizeCalls()[0]?.[1]).toMatchObject({ animate: false });
    expect(hudShowCalls()).toBe(0);

    resolveResize?.();
    await finalizing;

    expect(sizeCalls()).toHaveLength(1);
    expect(hudShowCalls()).toBe(1);
  });

  it("tweens the pill width down to the spinner square on stop", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockClear();
    const collapseClassAdds: Array<{ tokens: string[]; state: string | undefined }> = [];
    const hud = hudElement();
    const addClass = hud.classList.add.bind(hud.classList);
    vi.spyOn(hud.classList, "add").mockImplementation((...tokens: string[]) => {
      collapseClassAdds.push({ tokens, state: hud.dataset.state });
      addClass(...tokens);
    });

    // A measurable pill takes the CSS collapse: inline width tweens from the
    // listening size to the square while the native frame holds still, then
    // one snap resize fits the frame to the settled square.
    vi.spyOn(hud, "getBoundingClientRect").mockImplementation(
      () =>
        ({
          width: hud.dataset.state === "listening" ? 124 : 32,
          height: 32,
          top: 0,
          left: 0,
          right: hud.dataset.state === "listening" ? 124 : 32,
          bottom: 32,
          x: 0,
          y: 0,
          toJSON: () => ({}),
        }) as DOMRect,
    );

    const finalizing = emit("dictation-event", {
      type: "finalizing_transcript",
    });
    await vi.advanceTimersByTimeAsync(0);
    expect(hud.dataset.state).toBe("transcribing");
    expect(hud.classList.contains("hud-collapse")).toBe(true);
    expect(hud.classList.contains("hud-collapse-active")).toBe(true);
    expect(hud.style.width).toBe("32px");
    expect(collapseClassAdds.find(({ tokens }) => tokens.includes("hud-collapse"))?.state).toBe(
      "listening",
    );
    expect(sizeCalls()).toHaveLength(0);

    // jsdom fires no transitionend, so the collapse settles on its bounded
    // 600ms fallback.
    await vi.advanceTimersByTimeAsync(600);
    await finalizing;

    expect(hud.classList.contains("hud-collapse")).toBe(false);
    expect(hud.classList.contains("hud-collapse-active")).toBe(false);
    expect(hud.style.width).toBe("");
    expect(sizeCalls().length).toBeGreaterThanOrEqual(1);
    expect(sizeCalls()[0]?.[1]).toMatchObject({ animate: false });
  });

  it("restarts the spinner animation on the first and repeated condense", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });

    const hud = hudElement();
    const spinner = document.querySelector<HTMLElement>("#hud-spinner");
    expect(spinner).not.toBeNull();
    vi.spyOn(hud, "getBoundingClientRect").mockReturnValue({
      width: 124,
      height: 32,
      top: 0,
      left: 0,
      right: 124,
      bottom: 32,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);

    const reflowStates: Array<{ state: string | undefined; reset: boolean }> = [];
    vi.spyOn(spinner as HTMLElement, "offsetWidth", "get").mockImplementation(() => {
      reflowStates.push({
        state: hud.dataset.state,
        reset: spinner?.classList.contains("hud-spinner-reset") ?? false,
      });
      return 24;
    });

    const firstCondense = emit("dictation-event", { type: "finalizing_transcript" });
    await vi.advanceTimersByTimeAsync(0);

    expect(reflowStates.filter(({ reset }) => reset)).toHaveLength(1);
    expect(spinner?.classList.contains("hud-spinner-reset")).toBe(false);

    await vi.advanceTimersByTimeAsync(600);
    await firstCondense;
    await emit("dictation-event", { type: "paste_completed" });
    await vi.advanceTimersByTimeAsync(320);
    await emit("dictation-event", { type: "listening_started" });

    const secondCondense = emit("dictation-event", { type: "finalizing_transcript" });
    await vi.advanceTimersByTimeAsync(0);

    expect(reflowStates.filter(({ reset }) => reset)).toHaveLength(2);
    expect(spinner?.classList.contains("hud-spinner-reset")).toBe(false);

    await vi.advanceTimersByTimeAsync(600);
    await secondCondense;
  });

  it("restores listening when the stop command does not reach the helper", async () => {
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_helper_command") {
        return Promise.reject(new Error("helper stdin closed"));
      }
      return Promise.resolve(undefined);
    });
    mocks.invoke.mockClear();
    mocks.hide.mockClear();

    document.querySelector<HTMLButtonElement>("#hud-stop")?.click();

    await vi.waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_shake", undefined);
    });
    expect(hudElement().dataset.state).toBe("listening");
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("correlates the stop command with the visible take", async () => {
    await loadHud();
    await emit("dictation-event", {
      type: "listening_started",
      payload: { takeId: "take-1" },
    });
    mocks.invoke.mockClear();

    document.querySelector<HTMLButtonElement>("#hud-stop")?.click();
    await Promise.resolve();

    expect(mocks.invoke).toHaveBeenCalledWith("dictation_helper_command", {
      command: { type: "stop_and_paste", takeId: "take-1" },
    });
  });

  it("starts a fresh waveform loop after a hidden webview stalls the previous frame", async () => {
    const pendingFrameIds: number[] = [];
    let nextFrameId = 1;
    const requestFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((_callback: FrameRequestCallback) => {
        const id = nextFrameId++;
        // A hidden WKWebView may never service the frame that was queued while
        // the first dictation was leaving. Keep every callback pending here so
        // the test models that suspension instead of Chromium's eager replay.
        pendingFrameIds.push(id);
        return id;
      });
    const cancelFrame = vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});

    try {
      await loadHud();
      const framesBeforeListening = pendingFrameIds.length;
      await emit("dictation-event", { type: "listening_started" });
      expect(pendingFrameIds.length).toBeGreaterThan(framesBeforeListening);

      await emit("dictation-event", { type: "finalizing_transcript" });
      const cancelledFrameIds = cancelFrame.mock.calls.map(([id]) => id);
      expect(cancelledFrameIds.some((id) => pendingFrameIds.includes(id))).toBe(true);

      // The stale first frame must not suppress the next recording's loop.
      const framesBeforeRestart = pendingFrameIds.length;
      await emit("dictation-event", { type: "listening_started" });
      expect(pendingFrameIds.length).toBeGreaterThan(framesBeforeRestart);
    } finally {
      requestFrame.mockRestore();
      cancelFrame.mockRestore();
    }
  });

  it("discards the recording when the cancel X is clicked", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", {
      type: "listening_started",
      payload: { takeId: "take-1" },
    });
    mocks.invoke.mockClear();

    document.querySelector<HTMLButtonElement>("#hud-cancel")?.click();
    await Promise.resolve();
    expect(mocks.invoke).toHaveBeenCalledWith("dictation_helper_command", {
      command: { type: "discard_recording", takeId: "take-1" },
    });
    expect(hudElement().dataset.state).toBe("listening");
    expect(mocks.hide).not.toHaveBeenCalled();

    await emit("dictation-event", {
      type: "recording_discarded",
      payload: { takeId: "take-1" },
    });
    await vi.advanceTimersByTimeAsync(320);

    expect(mocks.hide).toHaveBeenCalledOnce();
  });

  it("does not let a stale discard hide a replacement take", async () => {
    await loadHud();
    await emit("dictation-event", {
      type: "listening_started",
      payload: { takeId: "take-1" },
    });
    await emit("dictation-event", {
      type: "listening_started",
      payload: { takeId: "take-2" },
    });
    mocks.hide.mockClear();

    await emit("dictation-event", {
      type: "recording_discarded",
      payload: { takeId: "take-1" },
    });

    expect(hudElement().dataset.state).toBe("listening");
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("ignores stale take-owned lifecycle events after a replacement starts", async () => {
    await loadHud();
    await emit("dictation-event", {
      type: "listening_started",
      payload: { takeId: "take-1" },
    });
    await emit("dictation-event", {
      type: "listening_started",
      payload: { takeId: "take-2" },
    });
    mocks.hide.mockClear();

    for (const event of [
      { type: "audio_level", payload: { takeId: "take-1", level: 0.8 } },
      { type: "finalizing_transcript", payload: { takeId: "take-1" } },
      { type: "final_transcript", payload: { takeId: "take-1", text: "old" } },
      { type: "paste_target", payload: { takeId: "take-1", app: "Old app" } },
      { type: "paste_completed", payload: { takeId: "take-1" } },
      {
        type: "error",
        payload: { takeId: "take-1", code: "old_take_failed", message: "Old failure" },
      },
      {
        type: "error",
        payload: {
          preserveActiveTake: true,
          code: "global_helper_error",
          message: "Unrelated helper failure",
        },
      },
    ]) {
      await emit("dictation-event", event);
    }

    expect(hudElement().dataset.state).toBe("listening");
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("keeps an active recording visible when the cancel command fails", async () => {
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_helper_command") {
        return Promise.reject(new Error("helper stdin closed"));
      }
      return Promise.resolve(undefined);
    });
    mocks.invoke.mockClear();
    mocks.hide.mockClear();

    document.querySelector<HTMLButtonElement>("#hud-cancel")?.click();

    await vi.waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_shake", undefined);
    });
    expect(hudElement().dataset.state).toBe("listening");
    expect(mocks.hide).not.toHaveBeenCalled();
    expect(mocks.invoke).not.toHaveBeenCalledWith("dictation_hud_set_cancel_bounds", {
      rect: null,
    });
  });

  it("discards the recording on Escape while listening", async () => {
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockClear();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    await vi.waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("dictation_helper_command", {
        command: { type: "discard_recording" },
      });
    });
  });

  it("ignores Escape once transcription has started", async () => {
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    await emit("dictation-event", { type: "finalizing_transcript" });
    mocks.invoke.mockClear();

    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await Promise.resolve();

    expect(mocks.invoke).not.toHaveBeenCalledWith("dictation_helper_command", {
      command: { type: "discard_recording" },
    });
  });

  it("responds to cancel hover immediately but waits for tooltip intent", async () => {
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });

    mocks.listeners.get("hud-cancel-hover")?.({ payload: true });
    expect(document.querySelector("#hud-cancel")?.classList.contains("is-hovered")).toBe(true);
    expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(false);

    mocks.listeners.get("hud-cancel-tooltip-hover")?.({ payload: true });
    await vi.waitFor(() => {
      expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(true);
    });
    expect(hudElement().dataset.tipPlacement).toBe("below");

    mocks.listeners.get("hud-cancel-hover")?.({ payload: false });
    expect(document.querySelector("#hud-cancel")?.classList.contains("is-hovered")).toBe(false);
    expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(true);

    mocks.listeners.get("hud-cancel-tooltip-hover")?.({ payload: false });
    expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(false);
  });

  it("keeps the macOS tooltip outside the Dictation HUD hit-tested frame", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "MacIntel",
    });
    await loadHud();
    await emit("dictation-event", {
      type: "listening_started",
      payload: { escapeCancelAvailable: true },
    });

    expect(mocks.invoke).toHaveBeenCalledWith(
      "dictation_hud_set_size",
      expect.objectContaining({ width: 36, height: 36 }),
    );
    mocks.invoke.mockClear();

    mocks.listeners.get("hud-cancel-tooltip-hover")?.({ payload: true });

    await vi.waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_set_tooltip_visible", {
        visible: true,
        placement: "below",
      });
    });
    expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(false);
  });

  it("does not teach Escape when the helper could not register it", async () => {
    await loadHud();
    await emit("dictation-event", {
      type: "listening_started",
      payload: { escapeCancelAvailable: false },
    });

    mocks.listeners.get("hud-cancel-hover")?.({ payload: true });
    mocks.listeners.get("hud-cancel-tooltip-hover")?.({ payload: true });
    await Promise.resolve();

    expect(document.querySelector("#hud-cancel")?.classList.contains("is-hovered")).toBe(true);
    expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(false);
  });

  it("does not teach Escape when a native helper omits the capability", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "Win32",
    });
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });

    expect(mocks.invoke).toHaveBeenCalledWith(
      "dictation_hud_set_size",
      expect.objectContaining({ width: 36, height: 36 }),
    );
    mocks.listeners.get("hud-cancel-hover")?.({ payload: true });
    mocks.listeners.get("hud-cancel-tooltip-hover")?.({ payload: true });
    await Promise.resolve();

    expect(document.querySelector("#hud-cancel")?.classList.contains("is-hovered")).toBe(true);
    expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(false);
    expect(mocks.invoke).not.toHaveBeenCalledWith(
      "dictation_hud_set_tooltip_visible",
      expect.objectContaining({ visible: true }),
    );
  });

  it("snap-sizes an interrupted fresh listening show before revealing it", async () => {
    await loadHud();
    hudElement().dataset.state = "exiting";
    mocks.invoke.mockClear();

    await emit("dictation-event", { type: "listening_started" });

    const setAlphaIndex = invokeCallIndex("dictation_hud_set_alpha");
    const setSizeIndex = invokeCallIndex("dictation_hud_set_size");
    const showIndex = invokeCallIndex("dictation_hud_show");

    expect(setAlphaIndex).toBeGreaterThanOrEqual(0);
    expect(setSizeIndex).toBeGreaterThan(setAlphaIndex);
    expect(showIndex).toBeGreaterThan(setSizeIndex);
    expect(sizeCalls()[0]?.[1]).toMatchObject({ animate: false });
    expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_set_alpha", {
      alpha: 0,
    });
  });

  it("measures the pill with in-flight transitions snapped to their end values", async () => {
    await loadHud();
    const hud = hudElement();
    // Mid-exit: the scale(0.94)/opacity transition is still in play (or
    // frozen by a hidden webview) when the next show measures the pill.
    hud.dataset.state = "exiting";
    const snappedAtMeasure: boolean[] = [];
    const measure = hud.getBoundingClientRect.bind(hud);
    vi.spyOn(hud, "getBoundingClientRect").mockImplementation(() => {
      snappedAtMeasure.push(hud.classList.contains("hud-snap"));
      return measure();
    });

    await emit("dictation-event", { type: "listening_started" });

    // The window-sizing measurement reads the rect under .hud-snap
    // (transition: none), so a frozen exit scale can't undersize the frame.
    expect(snappedAtMeasure[0]).toBe(true);
    expect(hud.classList.contains("hud-snap")).toBe(false);
  });

  it("shakes the listening pill instead of toasting when dictation is already listening", async () => {
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockClear();

    await emit("dictation-event", {
      type: "error",
      payload: {
        code: "already_listening",
        message: "Dictation is already listening.",
      },
    });

    // The pill stays in its listening state — no toast, no show/hide cycle.
    expect(hudElement().dataset.state).toBe("listening");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent("");
    expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_shake", undefined);
    expect(hudShowCalls()).toBe(0);
  });

  it("falls back to the error toast for already_listening outside the listening state", async () => {
    await loadHud();

    await emit("dictation-event", {
      type: "error",
      payload: {
        code: "already_listening",
        message: "Dictation is already listening.",
      },
    });

    expect(hudElement().dataset.state).toBe("error");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent(
      "Dictation is already listening.",
    );
    expect(hudShowCalls()).toBe(1);
  });

  it("reveals the error message below the pill by default", async () => {
    await loadHud();
    await emit("dictation-event", { type: "finalizing_transcript" });
    mocks.invoke.mockClear();

    await emit("dictation-event", {
      type: "error",
      payload: {
        code: "transcription_failed",
        message: "Dictation recorded no text. Try again.",
      },
    });

    expect(hudElement().dataset.state).toBe("error");
    const message = document.querySelector("#hud-error-text");
    expect(message).toHaveTextContent("Dictation recorded no text. Try again.");
    expect(hudElement().dataset.errorPlacement).toBe("below");
    // The message lives inside the HUD window now (the layer opens from the
    // pill), not as a detached caption sibling.
    expect(document.querySelector("#hud #hud-error-text")).not.toBeNull();
    // The HUD is frostless from native setup through every state, so this
    // transition no longer swaps native chrome mid-flow.
    expect(chromeCalls()).toEqual([]);
    expect(mocks.invoke).not.toHaveBeenCalledWith(
      "dictation_hud_caption_fits_below",
      expect.anything(),
    );
    expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_preferred_error_placement");
    // The window is sized to fit the message layer mirrored above/below the
    // pill, plus the shadow gutter (jsdom rects are zero: 2 gaps tall and
    // 2 compact HUD gutters all round).
    expect(mocks.invoke).toHaveBeenCalledWith(
      "dictation_hud_set_size",
      expect.objectContaining({ width: 36, height: 48 }),
    );
  });

  it("reveals the error message above the pill when native placement asks for it", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_hud_preferred_error_placement") {
        return Promise.resolve("above");
      }
      return Promise.resolve(undefined);
    });
    await loadHud();

    await emit("dictation-event", {
      type: "error",
      payload: {
        code: "transcription_failed",
        message: "Dictation recorded no text. Try again.",
      },
    });

    expect(hudElement().dataset.state).toBe("error");
    expect(hudElement().dataset.errorPlacement).toBe("above");
    expect(document.querySelector("#hud-error-text")).toHaveTextContent(
      "Dictation recorded no text. Try again.",
    );
  });

  it("fades the expanded error in place when the error exits", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", {
      type: "error",
      payload: { message: "Dictation recorded no text. Try again." },
    });
    mocks.invoke.mockClear();

    // Error exits by fading the expanded panel in place, not retracting it
    // back into the compact recorder pill.
    await vi.advanceTimersByTimeAsync(1800);
    expect(hudElement().dataset.state).toBe("exiting");
    expect(hudElement().classList.contains("hud-error-exit")).toBe(true);
    expect(hudElement().classList.contains("hud-reveal-collapsed")).toBe(false);
    expect(document.querySelector("#hud-error-text")).toHaveTextContent(
      "Dictation recorded no text. Try again.",
    );

    await vi.advanceTimersByTimeAsync(320);

    expect(mocks.hide).toHaveBeenCalledOnce();
    expect(hudElement().dataset.state).toBe("idle");
    expect(chromeCalls()).toEqual([]);
    // The message survives the exit fade (it dissolves with the window) and
    // clears once the pill parks on idle.
    expect(document.querySelector("#hud-error-text")).toHaveTextContent("");
  });

  it("hides when a Hey Clovy prompt starts an agent session", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "finalizing_transcript" });

    await emit("dictation-event", {
      type: "agent_session_prompt",
      payload: { prompt: "summarize the open document." },
    });

    expect(hudElement().dataset.state).toBe("exiting");
    await vi.advanceTimersByTimeAsync(320);
    expect(mocks.hide).toHaveBeenCalledOnce();
  });

  it("does not finish an in-flight hide after the HUD webview is destroyed", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "finalizing_transcript" });

    await emit("dictation-event", {
      type: "agent_session_prompt",
      payload: { prompt: "summarize the open document." },
    });
    expect(hudElement().dataset.state).toBe("exiting");

    window.dispatchEvent(new Event("pagehide"));
    await vi.advanceTimersByTimeAsync(320);

    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("cancels an in-flight condense when the HUD webview is destroyed", async () => {
    vi.useFakeTimers();
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    const hud = hudElement();
    vi.spyOn(hud, "getBoundingClientRect").mockImplementation(
      () =>
        ({
          width: hud.dataset.state === "listening" ? 124 : 32,
          height: 32,
          top: 0,
          left: 0,
          right: hud.dataset.state === "listening" ? 124 : 32,
          bottom: 32,
          x: 0,
          y: 0,
          toJSON: () => ({}),
        }) as DOMRect,
    );
    mocks.invoke.mockClear();
    mocks.hide.mockClear();

    const finalizing = emit("dictation-event", { type: "finalizing_transcript" });
    await vi.advanceTimersByTimeAsync(0);
    expect(hud.classList.contains("hud-collapse")).toBe(true);

    window.dispatchEvent(new Event("pagehide"));
    await finalizing;
    await vi.advanceTimersByTimeAsync(7000);

    expect(hud.classList.contains("hud-collapse")).toBe(false);
    expect(sizeCalls()).toEqual([]);
    expect(hudShowCalls()).toBe(0);
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("does not finish an in-flight error reveal after the HUD webview is destroyed", async () => {
    vi.useFakeTimers();
    let resolvePlacement: ((placement: string) => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_hud_preferred_error_placement") {
        return new Promise<string>((resolve) => {
          resolvePlacement = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    await loadHud();
    mocks.invoke.mockClear();

    const handlingError = emit("dictation-event", {
      type: "error",
      payload: { message: "Dictation failed." },
    });
    await Promise.resolve();
    expect(hudElement().dataset.state).toBe("error");

    window.dispatchEvent(new Event("pagehide"));
    resolvePlacement?.("below");
    await handlingError;
    await vi.advanceTimersByTimeAsync(2200);

    expect(mocks.invoke).not.toHaveBeenCalledWith("dictation_hud_show", expect.anything());
    expect(sizeCalls()).toEqual([]);
    expect(mocks.invoke).not.toHaveBeenCalledWith("dictation_hud_shake");
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("ignores a delayed latest event after the HUD webview is destroyed", async () => {
    let resolveLatestEvent: ((payload: string) => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "latest_dictation_event") {
        return new Promise<string>((resolve) => {
          resolveLatestEvent = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    await loadHud();
    mocks.invoke.mockClear();

    window.dispatchEvent(new Event("pagehide"));
    resolveLatestEvent?.(JSON.stringify({ type: "listening_started" }));
    await Promise.resolve();
    await Promise.resolve();

    expect(hudElement().dataset.state).toBe("idle");
    expect(hudShowCalls()).toBe(0);
    expect(sizeCalls()).toEqual([]);
  });

  it("does not show a delayed macOS tooltip after the HUD webview is destroyed", async () => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      configurable: true,
      value: {},
    });
    Object.defineProperty(navigator, "platform", {
      configurable: true,
      value: "MacIntel",
    });
    let resolvePlacement: ((placement: string) => void) | undefined;
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "dictation_hud_preferred_error_placement") {
        return new Promise<string>((resolve) => {
          resolvePlacement = resolve;
        });
      }
      return Promise.resolve(undefined);
    });
    await loadHud();
    await emit("dictation-event", {
      type: "listening_started",
      payload: { escapeCancelAvailable: true },
    });
    mocks.invoke.mockClear();

    mocks.listeners.get("hud-cancel-tooltip-hover")?.({ payload: true });
    window.dispatchEvent(new Event("pagehide"));
    resolvePlacement?.("below");
    await Promise.resolve();
    await Promise.resolve();

    expect(mocks.invoke).toHaveBeenCalledWith("dictation_hud_set_tooltip_visible", {
      visible: false,
    });
    expect(mocks.invoke).not.toHaveBeenCalledWith("dictation_hud_set_tooltip_visible", {
      visible: true,
      placement: "below",
    });
    expect(hudElement().classList.contains("hud-esc-tip-open")).toBe(false);
  });

  it("removes the Escape handler when the HUD webview is destroyed", async () => {
    await loadHud();
    await emit("dictation-event", { type: "listening_started" });
    mocks.invoke.mockClear();
    mocks.hide.mockClear();

    window.dispatchEvent(new Event("pagehide"));
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await Promise.resolve();

    expect(mocks.invoke).not.toHaveBeenCalledWith("dictation_helper_command", {
      command: { type: "discard_recording" },
    });
    expect(mocks.hide).not.toHaveBeenCalled();
  });

  it("does not listen for agent status sounds", async () => {
    await loadHud();
    expect(mocks.listeners.has(AGENT_SESSION_STATUS_EVENT)).toBe(false);
  });
});

async function loadHud() {
  await import("../hud");
  await Promise.resolve();
}

async function emit(event: string, payload: unknown) {
  const listener = mocks.listeners.get(event);
  expect(listener).toBeDefined();
  await listener?.({
    payload: JSON.stringify(payload),
  });
}

// The pill shows itself via the dictation_hud_show command (Rust positions
// the hidden window, then makes it visible) rather than appWindow.show().
function hudShowCalls() {
  return mocks.invoke.mock.calls.filter(([command]) => command === "dictation_hud_show").length;
}

function sizeCalls() {
  return mocks.invoke.mock.calls.filter(([command]) => command === "dictation_hud_set_size");
}

function invokeCallIndex(commandName: string) {
  return mocks.invoke.mock.calls.findIndex(([command]) => command === commandName);
}

function chromeCalls() {
  return mocks.invoke.mock.calls
    .filter(([command]) => command === "dictation_hud_set_chrome")
    .map(([, args]) => (args as { frostless: boolean }).frostless);
}

function hudElement() {
  const hud = document.querySelector<HTMLDivElement>("#hud");
  expect(hud).toBeTruthy();
  return hud as HTMLDivElement;
}

function hudMarkup() {
  return `
    <div
      id="hud"
      class="hud"
      data-state="idle"
      style="--control-lg: 32px; --sp-2: 6px"
    >
      <button id="hud-cancel" class="hud-cancel" type="button" aria-label="Cancel dictation"></button>
      <div class="hud-viz">
        <div class="hud-bars" aria-hidden="true">
          <span class="hud-bar"></span>
          <span class="hud-bar"></span>
          <span class="hud-bar"></span>
          <span class="hud-bar"></span>
          <span class="hud-bar"></span>
          <span class="hud-bar"></span>
          <span class="hud-bar"></span>
        </div>
      </div>
      <span id="hud-esc-tip" class="hud-esc-tip" aria-hidden="true">Esc to cancel</span>
      <span id="hud-spinner" class="hud-spinner" aria-hidden="true"></span>
      <span class="hud-error-icon" aria-hidden="true"></span>
      <span class="hud-error-layer" aria-hidden="true">
        <span id="hud-error-text" class="hud-error-message"></span>
      </span>
      <span class="hud-meeting-body">
        <span class="hud-meeting-mark" aria-hidden="true"></span>
        <span class="hud-meeting-text">
          <span id="hud-meeting-label" class="hud-meeting-label">Meeting detected</span>
          <span id="hud-meeting-app" class="hud-meeting-app"></span>
          <button id="hud-meeting-video" class="hud-meeting-video" role="switch" aria-checked="false" hidden type="button"><span id="hud-meeting-video-label"></span></button>
        </span>
      </span>
      <button id="hud-meeting-start" class="hud-meeting-start" type="button">
        <span class="hud-meeting-start-icon" aria-hidden="true"></span>
        Record
      </button>
      <button id="hud-meeting-dismiss" class="hud-meeting-dismiss" type="button" aria-label="Dismiss meeting prompt"></button>
      <button id="hud-stop" class="hud-stop" type="button" aria-label="Stop dictation">
        <span class="hud-stop-glyph" aria-hidden="true"></span>
      </button>
      <span id="hud-status" class="hud-status">Idle</span>
    </div>
  `;
}
