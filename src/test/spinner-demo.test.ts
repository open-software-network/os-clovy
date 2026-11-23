import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_HUD_VISIBILITY_CHANGED_EVENT } from "../lib/agent-hud-settings";
import { registerSpinnerDemo } from "../lib/spinner-demo";

const mocks = vi.hoisted(() => ({
  dismissToast: vi.fn(),
  emit: vi.fn().mockResolvedValue(undefined),
  showLoadingToast: vi.fn(() => "spinner-demo-toast"),
}));

vi.mock("@tauri-apps/api/event", () => ({
  emit: mocks.emit,
}));

vi.mock("../components/ui/Toaster", () => ({
  toast: {
    dismiss: mocks.dismissToast,
    loading: mocks.showLoadingToast,
  },
}));

type SpinnerDemoWindow = Window & {
  __agentHud?: (state?: string, count?: number) => string;
  __sidebarStates?: (show?: boolean) => string;
  __spinnerDemo?: (show?: boolean) => string;
};

describe("spinner demo", () => {
  const win = window as SpinnerDemoWindow;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.clear();
    localStorage.setItem("clovy:agent-hud:enabled", "false");
    win.__agentHud = vi.fn(() => "hud");
    win.__sidebarStates = vi.fn(() => "sidebar");
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    delete win.__agentHud;
    delete win.__sidebarStates;
    delete win.__spinnerDemo;
  });

  it("shows representative real spinner contexts and clears them together", async () => {
    ({ dispose } = registerSpinnerDemo());

    expect(win.__spinnerDemo).toBeTypeOf("function");
    expect(win.__spinnerDemo?.()).toBe(
      "Spinner demo shown: sidebar, Agent HUD, and loading toast. Run __spinnerDemo(false) to clear.",
    );
    expect(win.__sidebarStates).toHaveBeenCalledWith(true);
    expect(win.__agentHud).toHaveBeenCalledWith("mixed");
    expect(mocks.showLoadingToast).toHaveBeenCalledWith("Spinner showcase");
    await vi.waitFor(() =>
      expect(mocks.emit).toHaveBeenCalledWith(AGENT_HUD_VISIBILITY_CHANGED_EVENT, {
        enabled: true,
      }),
    );

    expect(win.__spinnerDemo?.(false)).toBe("Spinner demo cleared.");
    expect(win.__agentHud).toHaveBeenCalledWith("clear");
    expect(win.__sidebarStates).toHaveBeenCalledWith(false);
    expect(mocks.dismissToast).toHaveBeenCalledWith("spinner-demo-toast");
    await vi.waitFor(() =>
      expect(mocks.emit).toHaveBeenCalledWith(AGENT_HUD_VISIBILITY_CHANGED_EVENT, {
        enabled: false,
      }),
    );
  });

  it("does not create duplicate loading toasts when shown twice", () => {
    ({ dispose } = registerSpinnerDemo());

    win.__spinnerDemo?.();
    win.__spinnerDemo?.();

    expect(mocks.showLoadingToast).toHaveBeenCalledOnce();
  });

  it("removes its console hook and clears active demo state on disposal", () => {
    ({ dispose } = registerSpinnerDemo());
    win.__spinnerDemo?.();

    dispose();
    dispose = undefined;

    expect(win.__spinnerDemo).toBeUndefined();
    expect(win.__agentHud).toHaveBeenCalledWith("clear");
    expect(mocks.dismissToast).toHaveBeenCalledWith("spinner-demo-toast");
  });
});
