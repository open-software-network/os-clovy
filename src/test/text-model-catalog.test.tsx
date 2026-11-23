import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AppSettings } from "../components/settings/AppSettings";
import { applyInterfaceLocale } from "../i18n/locale";
import { setCurrentDataPartitionName } from "../lib/data-partition";
import type { LlmProvidersDto } from "../lib/llm-providers";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", async (original) => ({
  ...(await original<typeof import("@tauri-apps/api/core")>()),
  invoke: mocks.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));

const models = ["vendor/one", "vendor/two", "vendor/three"].map((id, index) => ({
  id,
  name: `Text model ${index + 1}`,
  description: `Catalog details for ${id}`,
  provider: "openrouter",
  modelType: "text",
  capabilities: index === 1 ? [] : ["supportsFunctionCalling"],
  traits: [],
  contextTokens: 128000,
}));
let settings: Record<string, unknown>;
let effectiveSettings: Record<string, unknown> | undefined;
let providers: LlmProvidersDto;
let catalogFails: boolean;
function renderSettings() {
  return render(
    <AppSettings
      sourceMode="microphoneOnly"
      checkingSourceReadiness={false}
      onSourceModeChange={() => {}}
      onEnableSystemAudio={() => {}}
      activeTab="models"
      onTabChange={() => {}}
    />,
  );
}
function generationReads() {
  return mocks.invoke.mock.calls.filter(
    ([command, args]) => command === "list_provider_models" && args?.request.mode === "generation",
  ).length;
}

beforeEach(() => {
  mocks.invoke.mockReset();
  setCurrentDataPartitionName("default");
  applyInterfaceLocale("en");
  catalogFails = false;
  effectiveSettings = undefined;
  settings = {
    transcriptionProvider: "groq",
    generationProvider: "openrouter",
    transcriptionModel: "whisper-large-v3-turbo",
    imageModel: "fal-ai/flux/dev",
    videoModel: "fal-ai/wan/v2.2-a14b/text-to-video",
    remoteGenerationModel: "vendor/saved",
    localGeneration: { baseUrl: "", modelId: "" },
    generationModel: "vendor/saved",
    providerApiKeysConfigured: { groq: false, openrouter: false, fal: false, tavily: false },
  };
  providers = {
    endpoints: [],
    usage: {
      chat: { kind: "cli", id: "claude" },
      notes: { kind: "openrouter" },
      activity: { kind: "none" },
    },
    cliLevels: {},
  };
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === "dictation_settings") return { settings: { microphone: {} } };
    if (command === "dictation_hotkey_status") return { type: "ready", payload: {} };
    if (command === "provider_model_settings") return { settings, effectiveSettings };
    if (command === "llm_providers") return providers;
    if (command === "llm_detect_clis")
      return [
        {
          id: "claude",
          name: "Claude Code",
          installed: true,
          oneShot: true,
          structuredOutput: "strict",
        },
      ];
    if (command === "list_provider_models") {
      if (args.request.mode === "generation" && catalogFails) throw new Error("catalog offline");
      return { models: args.request.mode === "generation" ? models : [] };
    }
    if (command === "set_provider_api_key" || command === "clear_provider_api_key") {
      settings = {
        ...settings,
        providerApiKeysConfigured: {
          groq: false,
          openrouter: command === "set_provider_api_key",
          fal: false,
          tavily: false,
        },
      };
      return settings;
    }
    if (command === "set_provider_model") {
      settings = { ...settings, generationModel: args.request.modelId };
      return settings;
    }
    if (command === "llm_set_usage") {
      providers = { ...providers, usage: { ...providers.usage, [args.usage]: args.provider } };
      return providers;
    }
    return undefined;
  });
});
afterEach(() => {
  cleanup();
  setCurrentDataPartitionName("default");
  applyInterfaceLocale("en");
});

describe("OpenRouter text model catalog", () => {
  it("retries a failed catalog in a read-only named partition", async () => {
    setCurrentDataPartitionName("Research");
    catalogFails = true;
    const user = userEvent.setup();
    renderSettings();
    const retry = await screen.findByRole("button", { name: "Retry model list" });
    const reads = generationReads();
    catalogFails = false;
    await user.click(retry);
    await waitFor(() => expect(generationReads()).toBeGreaterThan(reads));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Retry model list" })).not.toBeInTheDocument(),
    );
    expect(screen.getByRole("button", { name: "Change text model" })).toBeDisabled();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "set_provider_model")).toBe(
      false,
    );
  });

  it.each([
    "save",
    "remove",
  ] as const)("preserves partition model overrides when a global key is %s", async (operation) => {
    setCurrentDataPartitionName("Research");
    settings.providerApiKeysConfigured = {
      groq: false,
      openrouter: operation === "remove",
      fal: false,
      tavily: false,
    };
    effectiveSettings = {
      ...settings,
      transcriptionModel: "research-transcription",
      imageModel: "research-image",
      videoModel: "research-video",
    };
    const user = userEvent.setup();
    renderSettings();
    const transcription = await screen.findByRole("button", { name: "Change transcription model" });
    await waitFor(() => expect(transcription).toHaveTextContent("research-transcription"));
    await user.click(screen.getByRole("button", { name: "More options for text" }));
    const input = screen.getByLabelText("OpenRouter API key");
    const row = input.closest(".settings-row") as HTMLElement;
    if (operation === "save") await user.type(input, "fixture-key");
    await user.click(
      within(row).getByRole("button", { name: operation === "save" ? "Save" : "Remove" }),
    );
    await waitFor(() =>
      expect(
        within(row).getByRole("button", { name: operation === "save" ? "Remove" : "Save" }),
      ).toBeVisible(),
    );
    expect(transcription).toHaveTextContent("research-transcription");
    expect(screen.getByRole("button", { name: "Change image model" })).toHaveTextContent(
      "research-image",
    );
    expect(screen.getByRole("button", { name: "Change video model" })).toHaveTextContent(
      "research-video",
    );
  });

  it.each([
    "pointer",
    "focus",
  ] as const)("shows generation model details on %s", async (interaction) => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(await screen.findByRole("button", { name: "Change text model" }));
    const option = await screen.findByRole("option", { name: "Text model 2" });
    if (interaction === "pointer") await user.hover(option);
    else fireEvent.focus(option);
    expect(await screen.findByText("Catalog details for vendor/two")).toBeVisible();
  });

  it("offers all text models without tools or keys and saves only the chosen text model", async () => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(await screen.findByRole("button", { name: "Change text model" }));
    for (const model of models)
      expect(await screen.findByRole("option", { name: model.name })).toBeEnabled();
    expect(screen.queryByRole("switch", { name: /auto/i })).not.toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: "Text model 2" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("set_provider_model", {
        request: { mode: "generation", modelId: "vendor/two" },
      }),
    );
    expect(mocks.invoke.mock.calls.some(([command]) => command === "llm_set_usage")).toBe(false);
    expect(providers.usage.chat).toEqual({ kind: "cli", id: "claude" });
  });
  it("keeps the saved model visible on catalog failure and retries", async () => {
    catalogFails = true;
    const user = userEvent.setup();
    renderSettings();
    await user.click(await screen.findByRole("button", { name: "Change text model" }));
    expect(
      await screen.findByText(
        "The model list could not be loaded. Your saved model is still selected.",
      ),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Change text model" })).toHaveTextContent(
      "vendor/saved",
    );
    catalogFails = false;
    await user.click(screen.getByRole("button", { name: "Retry model list" }));
    expect(await screen.findByRole("option", { name: "Text model 2" })).toBeEnabled();
    expect(
      screen.queryByText("The model list could not be loaded. Your saved model is still selected."),
    ).not.toBeInTheDocument();
  });
  it("reloads generation after saving and removing the OpenRouter key", async () => {
    const user = userEvent.setup();
    renderSettings();
    await user.click(await screen.findByRole("button", { name: "More options for text" }));
    const input = screen.getByLabelText("OpenRouter API key");
    const row = input.closest(".settings-row") as HTMLElement;
    const initial = generationReads();
    await user.type(input, "test-key");
    await user.click(within(row).getByRole("button", { name: "Save" }));
    await waitFor(() => expect(generationReads()).toBeGreaterThan(initial));
    const saved = generationReads();
    await user.click(within(row).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(generationReads()).toBeGreaterThan(saved));
  });
  it("updates the description when the Notes engine changes without changing chat", async () => {
    const user = userEvent.setup();
    renderSettings();
    const notes = await screen.findByRole("combobox", { name: "Notes" });
    await user.selectOptions(notes, "cli:claude");
    expect(
      await screen.findByText(
        "Notes are generated by Claude Code. This text model is not used for notes.",
      ),
    ).toBeVisible();
    expect(providers.usage.chat).toEqual({ kind: "cli", id: "claude" });
  });
  it.each([
    ["en", "openrouter", "This model generates notes when Notes uses OpenRouter."],
    ["pt-BR", "openrouter", "Este modelo gera as notas quando o motor de Notas é OpenRouter."],
    ["en", "claude", "Notes are generated by Claude Code. This text model is not used for notes."],
    [
      "pt-BR",
      "claude",
      "As notas são geradas por Claude Code. Este modelo de texto não é usado nas notas.",
    ],
  ] as const)("describes the %s notes engine %s", async (locale, engine, copy) => {
    providers.usage.notes =
      engine === "openrouter" ? { kind: "openrouter" } : { kind: "cli", id: "claude" };
    applyInterfaceLocale(locale);
    renderSettings();
    expect(await screen.findByText(copy)).toBeVisible();
  });
});
