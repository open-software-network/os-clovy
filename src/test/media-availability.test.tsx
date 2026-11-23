import { cleanup, render, screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: (command: string, args?: unknown) =>
    args !== undefined ? invokeMock(command, args) : invokeMock(command),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => undefined),
}));

import { AppSettings } from "../components/settings/AppSettings";
import type { ProviderModelSettingsDto } from "../lib/tauri";

const baseSettings: ProviderModelSettingsDto = {
  transcriptionProvider: "groq",
  generationProvider: "openrouter",
  transcriptionModel: "whisper-large-v3-turbo",
  generationModel: "z-ai/glm-5.2",
  remoteGenerationModel: "z-ai/glm-5.2",
  imageModel: "fal-ai/flux/dev",
  videoModel: "fal-ai/wan/v2.2-a14b/text-to-video",
  providerApiKeysConfigured: {
    groq: false,
    openrouter: false,
    fal: false,
    tavily: false,
  },
  localGeneration: {
    baseUrl: "",
    modelId: "",
  },
  imageSafeMode: true,
  imageSafeModePromptDismissed: false,
  liveTranscription: true,
};

import { IMAGE_MODELS } from "../lib/image-models";
import { VIDEO_MODELS } from "../lib/video-models";

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

function backend(availability: boolean | "error" | undefined, hasKey = false) {
  let settings = {
    ...baseSettings,
    providerApiKeysConfigured: { ...baseSettings.providerApiKeysConfigured, fal: hasKey },
  };
  const state = { availability };
  invokeMock.mockImplementation(
    (command: string, args?: { request?: { mode?: string; modelId?: string } }) => {
      if (command === "dictation_settings")
        return Promise.resolve({
          settings: {
            language: "en",
            microphone: {},
            style: "standard",
            pushToTalkShortcut: { modifiers: {} },
            toggleShortcut: { modifiers: {} },
          },
        });
      if (command === "dictation_hotkey_status") return Promise.resolve({ type: "ready" });
      if (command === "provider_model_settings")
        return Promise.resolve({ settings, effectiveSettings: settings });
      if (command === "list_provider_models") {
        if (state.availability === "error") return Promise.reject(new Error("Catalog unavailable"));
        return Promise.resolve({
          mode: args?.request?.mode,
          models: (args?.request?.mode === "image"
            ? IMAGE_MODELS
            : args?.request?.mode === "video"
              ? VIDEO_MODELS
              : []
          ).map((model) => ({ ...model, serviceAvailable: state.availability })),
        });
      }
      if (command === "set_provider_model") {
        settings = { ...settings, [`${args?.request?.mode}Model`]: args?.request?.modelId };
        return Promise.resolve(settings);
      }
      if (command === "set_provider_api_key" || command === "clear_provider_api_key") {
        settings = {
          ...settings,
          providerApiKeysConfigured: {
            ...settings.providerApiKeysConfigured,
            fal: command === "set_provider_api_key",
          },
        };
        return Promise.resolve(settings);
      }
      return Promise.reject(new Error("unavailable test command"));
    },
  );
  return state;
}

afterEach(cleanup);

it.each([
  [undefined, false],
  [undefined, true],
  [false, false],
  [false, true],
  [true, false],
  [true, true],
  ["error", false],
  ["error", true],
] as const)("keeps curated media selectable with catalog=%s and fal key=%s", async (availability, key) => {
  backend(availability, key);
  renderSettings();
  const user = userEvent.setup();
  for (const [mode, models] of [
    ["image", IMAGE_MODELS],
    ["video", VIDEO_MODELS],
  ] as const) {
    await user.click(
      await screen.findByRole("button", { name: new RegExp(`change ${mode} model`, "i") }),
    );
    const dialog = screen.getByRole("dialog");
    for (const model of models) {
      expect(await within(dialog).findByRole("option", { name: model.name })).toBeEnabled();
    }
    expect(screen.queryByText(/Clovy has no fal service key here/)).not.toBeInTheDocument();
    expect(screen.queryByText(/could not confirm media availability/)).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
  }
});

it.each([
  ["image", IMAGE_MODELS[1]],
  ["video", VIDEO_MODELS[1]],
] as const)("persists a %s choice without a fal key and keeps it after catalog refresh failure", async (mode, selected) => {
  const state = backend(undefined);
  const view = renderSettings();
  const user = userEvent.setup();
  const buttonName = new RegExp(`change ${mode} model`, "i");
  await user.click(await screen.findByRole("button", { name: buttonName }));
  await user.click(
    await within(screen.getByRole("dialog")).findByRole("option", { name: selected.name }),
  );
  await waitFor(() =>
    expect(invokeMock).toHaveBeenCalledWith("set_provider_model", {
      request: { mode, modelId: selected.id },
    }),
  );
  state.availability = "error";
  view.unmount();
  renderSettings();
  const button = await screen.findByRole("button", { name: buttonName });
  await waitFor(() => expect(button).toHaveTextContent(selected.name));
  await user.click(button);
  expect(
    await within(screen.getByRole("dialog")).findByRole("option", { name: selected.name }),
  ).toHaveAttribute("aria-selected", "true");
});

it("keeps both media catalogs selectable after saving and clearing the fal key", async () => {
  backend(undefined);
  renderSettings();
  const user = userEvent.setup();
  await user.click(await screen.findByRole("button", { name: /more options for text/i }));
  const keyRow = (await screen.findByLabelText(/fal api key/i)).closest(
    ".settings-row",
  ) as HTMLElement;
  await user.type(within(keyRow).getByLabelText(/fal api key/i), "fixture-key");
  await user.click(within(keyRow).getByRole("button", { name: "Save" }));
  expect(await within(keyRow).findByText("Configured")).toBeInTheDocument();
  await user.click(within(keyRow).getByRole("button", { name: "Remove" }));
  await waitFor(() => expect(within(keyRow).queryByText("Configured")).not.toBeInTheDocument());
  for (const mode of ["image", "video"]) {
    await user.click(screen.getByRole("button", { name: new RegExp(`change ${mode} model`, "i") }));
    expect(within(screen.getByRole("dialog")).getAllByRole("option").length).toBeGreaterThan(0);
    await user.keyboard("{Escape}");
  }
});
