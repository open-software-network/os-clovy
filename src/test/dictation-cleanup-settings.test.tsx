import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { dispatchProviderModelSettingsChanged } from "../lib/model-privacy";
import { StyleSettingsSection } from "../components/settings/StyleSettingsSection";

const mocks = vi.hoisted(() => ({
  dictationSettings: vi.fn(),
  setDictationStyle: vi.fn(),
  setDictationCleanup: vi.fn(),
  setDictationHudPlacement: vi.fn(),
  listProviderModels: vi.fn(),
}));

vi.mock("../lib/tauri", () => ({
  dictationSettings: mocks.dictationSettings,
  setDictationStyle: mocks.setDictationStyle,
  setDictationCleanup: mocks.setDictationCleanup,
  setDictationHudPlacement: mocks.setDictationHudPlacement,
  listProviderModels: mocks.listProviderModels,
}));

function model(id: string, name: string, provider = "openrouter") {
  return { id, name, provider, modelType: "text", traits: [], capabilities: [] };
}

function settings(cleanupEnabled: boolean, hudPlacement: "bottom" | "top" = "bottom") {
  return {
    pushToTalkShortcut: {
      code: "Fn",
      label: "Fn",
      pressCount: 1,
      modifiers: { command: false, control: false, option: false, shift: false, function: true },
    },
    toggleShortcut: {
      code: "KeyT",
      label: "Ctrl+Opt+T",
      pressCount: 1,
      modifiers: { command: false, control: true, option: true, shift: false, function: false },
    },
    microphone: {},
    style: "standard",
    cleanupEnabled,
    cleanupModel: "nvidia/nemotron-3-nano-30b-a3b",
    hudPlacement,
  };
}

describe("dictation cleanup settings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.dictationSettings.mockResolvedValue({ settings: settings(false) });
    mocks.setDictationCleanup.mockImplementation(
      async (enabled: boolean, cleanupModel?: string) => ({
        ...settings(enabled),
        cleanupModel: cleanupModel ?? "nvidia/nemotron-3-nano-30b-a3b",
      }),
    );
    mocks.setDictationHudPlacement.mockImplementation(async (placement: "bottom" | "top") =>
      settings(false, placement),
    );
    mocks.listProviderModels.mockResolvedValue({
      mode: "generation",
      modelType: "text",
      selectedModel: "open-software/auto",
      models: [
        model("open-software/auto", "Auto", "open-software"),
        model("nvidia/nemotron-3-nano-30b-a3b", "Nemotron 3 Nano"),
        model("openai/gpt-5-mini", "GPT-5 mini"),
        model("cli:claude", "Claude Code", "cli"),
      ],
    });
  });

  it("refreshes an open cleanup selector after the OpenRouter key changes", async () => {
    mocks.dictationSettings.mockResolvedValue({ settings: settings(true) });
    render(<StyleSettingsSection />);
    await waitFor(() => expect(mocks.listProviderModels).toHaveBeenCalledTimes(1));
    mocks.listProviderModels.mockResolvedValue({
      models: [model("vendor/new", "New catalog model")],
    });
    act(() => dispatchProviderModelSettingsChanged({ mode: "generation", modelId: "vendor/new" }));
    expect(await screen.findByRole("option", { name: "New catalog model" })).toBeInTheDocument();
    expect(mocks.listProviderModels).toHaveBeenCalledTimes(2);
  });

  it("starts off and hides the model and style until cleanup is turned on", async () => {
    render(<StyleSettingsSection />);

    const toggle = await screen.findByRole("switch", { name: "Clean up dictation text" });
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "false"));
    expect(screen.queryByRole("combobox", { name: "Cleanup model" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Formal" })).not.toBeInTheDocument();
  });

  it("turns cleanup on and offers only OpenRouter models", async () => {
    const user = userEvent.setup();
    render(<StyleSettingsSection />);

    const toggle = await screen.findByRole("switch", { name: "Clean up dictation text" });
    await user.click(toggle);

    await waitFor(() => expect(mocks.setDictationCleanup).toHaveBeenCalledWith(true));
    await waitFor(() => expect(toggle).toHaveAttribute("aria-checked", "true"));
    const picker = await screen.findByRole("combobox", { name: "Cleanup model" });
    const labels = within(picker)
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(labels).toEqual(["Nemotron 3 Nano", "GPT-5 mini"]);
    expect(screen.getByRole("button", { name: "Formal" })).toBeInTheDocument();

    await user.selectOptions(picker, "openai/gpt-5-mini");
    await waitFor(() =>
      expect(mocks.setDictationCleanup).toHaveBeenLastCalledWith(true, "openai/gpt-5-mini"),
    );
  });

  it("loads indicator position as top, attempts switching to bottom, and reverts to top if persistence fails", async () => {
    const user = userEvent.setup();
    mocks.dictationSettings.mockResolvedValue({ settings: settings(false, "top") });
    mocks.setDictationHudPlacement.mockRejectedValueOnce(new Error("Failed to save placement"));

    render(<StyleSettingsSection />);

    const topButton = await screen.findByRole("button", { name: "Top" });
    const bottomButton = screen.getByRole("button", { name: "Bottom" });
    await waitFor(() => expect(topButton).toHaveAttribute("aria-pressed", "true"));
    expect(bottomButton).toHaveAttribute("aria-pressed", "false");

    await user.click(bottomButton);

    await waitFor(() => expect(mocks.setDictationHudPlacement).toHaveBeenCalledWith("bottom"));
    await waitFor(() => expect(topButton).toHaveAttribute("aria-pressed", "true"));
    expect(bottomButton).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByText("Failed to save placement")).toBeInTheDocument();
  });
});
