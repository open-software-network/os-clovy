import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  helperListener: undefined as ((event: { payload: unknown }) => void) | undefined,
  dictationHelperCommand: vi.fn(),
  setDictationShortcut: vi.fn(),
}));

const settings = {
  pushToTalkShortcut: {
    keyCode: 2,
    code: "KeyD",
    modifiers: { command: false, control: true, option: true, shift: false, function: false },
    label: "Ctrl+Alt+D",
    pressCount: 1,
  },
  toggleShortcut: {
    keyCode: 17,
    code: "KeyT",
    modifiers: { command: false, control: true, option: true, shift: false, function: false },
    label: "Ctrl+Alt+T",
    pressCount: 1,
  },
  microphone: {},
  style: "standard",
  hudPlacement: "bottom",
  cleanupEnabled: false,
  cleanupModel: "nvidia/nemotron-3-nano-30b-a3b",
};

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: vi.fn(async () => {
    throw new Error("no tauri in tests");
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, callback: (event: { payload: unknown }) => void) => {
    if (name === "dictation-event") mocks.helperListener = callback;
    return () => {};
  }),
  emit: vi.fn(async () => undefined),
}));

vi.mock("../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/tauri")>()),
  dictationCapabilities: vi.fn(async () => ({
    capabilities: { platform: "windows", shortcuts: true, systemAudio: false },
  })),
  dictationSettings: vi.fn(async () => ({ settings })),
  dictationHotkeyStatus: vi.fn(async () => ({ type: "hotkey_trigger_ready" })),
  dictationHelperCommand: mocks.dictationHelperCommand,
  setDictationShortcut: mocks.setDictationShortcut,
}));

import { AppSettings } from "../components/settings/AppSettings";
import { applyInterfaceLocale } from "../i18n/locale";

function helperEvent(type: string, payload: Record<string, unknown>) {
  act(() => mocks.helperListener?.({ payload: JSON.stringify({ type, payload }) }));
}

async function renderShortcuts(changeLabel = "Change") {
  render(
    <AppSettings
      sourceMode="microphoneOnly"
      checkingSourceReadiness={false}
      onSourceModeChange={() => {}}
      onEnableSystemAudio={() => {}}
      activeTab="shortcuts"
      onTabChange={() => {}}
    />,
  );
  await waitFor(() => expect(mocks.helperListener).toBeDefined());
  await screen.findAllByRole("button", { name: changeLabel });
}

async function startPushToTalkCapture() {
  const [change] = screen.getAllByRole("button", { name: "Change" });
  await userEvent.setup().click(change);
  await waitFor(() =>
    expect(mocks.dictationHelperCommand).toHaveBeenCalledWith(
      expect.objectContaining({ type: "start_shortcut_capture", kind: "push_to_talk" }),
    ),
  );
}

describe("shortcut capture on Windows", () => {
  beforeEach(() => {
    mocks.helperListener = undefined;
    mocks.dictationHelperCommand.mockReset().mockResolvedValue(undefined);
    mocks.setDictationShortcut.mockReset().mockImplementation(async (_kind, shortcut) => ({
      ...settings,
      pushToTalkShortcut: { keyCode: 0, ...shortcut },
    }));
  });

  afterEach(() => {
    cleanup();
    applyInterfaceLocale("en");
  });

  it("leaves the keys to the helper: one source, one save", async () => {
    await renderShortcuts();
    await startPushToTalkCapture();

    fireEvent.keyDown(window, { key: "D", code: "KeyD", ctrlKey: true, shiftKey: true });
    expect(mocks.setDictationShortcut).not.toHaveBeenCalled();
    expect(mocks.dictationHelperCommand).not.toHaveBeenCalledWith({
      type: "cancel_shortcut_capture",
    });

    const shortcut = {
      code: "ControlRight",
      label: "Right Ctrl",
      modifiers: { command: false, control: false, option: false, shift: false, function: false },
      pressCount: 1,
    };
    helperEvent("shortcut_captured", { kind: "push_to_talk", shortcut });
    await waitFor(() => expect(mocks.setDictationShortcut).toHaveBeenCalledTimes(1));
    expect(mocks.setDictationShortcut).toHaveBeenCalledWith(
      "push_to_talk",
      expect.objectContaining({ code: "ControlRight" }),
    );
  });

  it("says when the pressed key never reaches Windows, and suggests Right Ctrl", async () => {
    await renderShortcuts();
    await startPushToTalkCapture();

    helperEvent("shortcut_capture_error", {
      kind: "push_to_talk",
      code: "no_key_detected",
      message: "No key reached Windows.",
    });
    const error = await screen.findByText(/doesn't reach Windows/);
    expect(error.textContent).toMatch(/Right Ctrl/);
  });

  it("records Ctrl, Alt or Shift of the left side alone and names it in Portuguese", async () => {
    applyInterfaceLocale("pt-BR");
    await renderShortcuts("Alterar");
    const [change] = screen.getAllByRole("button", { name: "Alterar" });
    await userEvent.setup().click(change);
    expect(
      await screen.findByText(/Ctrl, Alt ou Shift sozinho, de qualquer lado/),
    ).toBeInTheDocument();
    expect(screen.getByText(/segure para ditar/)).toBeInTheDocument();

    helperEvent("shortcut_captured", {
      kind: "push_to_talk",
      shortcut: {
        code: "AltLeft",
        label: "Left Alt",
        modifiers: { command: false, control: false, option: false, shift: false, function: false },
        pressCount: 1,
      },
    });
    await waitFor(() =>
      expect(mocks.setDictationShortcut).toHaveBeenCalledWith(
        "push_to_talk",
        expect.objectContaining({ code: "AltLeft", pressCount: 1 }),
      ),
    );
    expect(await screen.findByText("Alt esquerdo")).toBeInTheDocument();
  });

  it("saves a lone modifier on the toggle as a double tap", async () => {
    applyInterfaceLocale("pt-BR");
    mocks.setDictationShortcut.mockImplementation(async (_kind, shortcut) => ({
      ...settings,
      toggleShortcut: { keyCode: 0, ...shortcut },
    }));
    await renderShortcuts("Alterar");
    const [, change] = screen.getAllByRole("button", { name: "Alterar" });
    await userEvent.setup().click(change);
    expect(await screen.findByText(/dois toques ligam e desligam/)).toBeInTheDocument();

    helperEvent("shortcut_captured", {
      kind: "toggle",
      shortcut: {
        code: "ControlLeft",
        label: "Left Ctrl+Left Ctrl",
        modifiers: { command: false, control: false, option: false, shift: false, function: false },
        pressCount: 2,
      },
    });
    await waitFor(() =>
      expect(mocks.setDictationShortcut).toHaveBeenCalledWith(
        "toggle",
        expect.objectContaining({ code: "ControlLeft", pressCount: 2 }),
      ),
    );
    expect(await screen.findAllByText("Ctrl esquerdo")).toHaveLength(2);
  });

  it("names lone right-hand keys in Portuguese", async () => {
    applyInterfaceLocale("pt-BR");
    await renderShortcuts("Alterar");
    const [change] = screen.getAllByRole("button", { name: "Alterar" });
    await userEvent.setup().click(change);

    helperEvent("shortcut_captured", {
      kind: "push_to_talk",
      shortcut: {
        code: "ControlRight",
        label: "Right Ctrl",
        modifiers: { command: false, control: false, option: false, shift: false, function: false },
        pressCount: 1,
      },
    });
    expect(await screen.findByText("Ctrl direito")).toBeInTheDocument();
  });

  it("keeps a registration error on the row after the capture ends", async () => {
    await renderShortcuts();
    await startPushToTalkCapture();

    helperEvent("hotkey_trigger_unavailable", {
      kind: "push_to_talk",
      message: "Windows could not register Ctrl+Shift+D. It may already be used by another app.",
    });
    helperEvent("shortcut_capture_cancelled", { kind: "push_to_talk" });
    expect(screen.getByText(/could not register Ctrl\+Shift\+D/)).toBeInTheDocument();
  });
});
