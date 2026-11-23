import { act, cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: vi.fn(async (command: string) => {
    if (command === "list_dictionary_entries") {
      return [
        {
          id: "entry-1",
          phrase: "Jane Doe",
          createdAt: "2026-05-26T00:00:00Z",
          updatedAt: "2026-05-26T00:00:00Z",
        },
      ];
    }
    throw new Error("no tauri in tests");
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => undefined),
}));

import { AppSettings } from "../components/settings/AppSettings";
import { DictionarySettingsSection } from "../components/settings/DictionarySettingsSection";
import { MicTestControl } from "../components/settings/MicTestControl";
import { SETTINGS_TABS, type SettingsTab } from "../components/settings/settings-config";
import { applyInterfaceLocale } from "../i18n/locale";

function renderSettings(activeTab: SettingsTab) {
  return render(
    <AppSettings
      sourceMode="microphoneOnly"
      checkingSourceReadiness={false}
      onSourceModeChange={() => {}}
      onEnableSystemAudio={() => {}}
      activeTab={activeTab}
      onTabChange={() => {}}
    />,
  );
}

function renderMicTest() {
  return render(
    <MicTestControl
      state="idle"
      level={0}
      playing={false}
      durationSeconds={5}
      onStart={() => {}}
      onStartOver={() => {}}
      onPlaybackError={() => {}}
      onPlayingChange={() => {}}
    />,
  );
}

afterEach(() => {
  cleanup();
  applyInterfaceLocale("en");
});

describe("settings in English (default)", () => {
  it("keeps the English copy", () => {
    renderSettings("about");
    expect(screen.getByRole("heading", { name: "About" })).toBeInTheDocument();
  });
});

describe("settings in Portuguese (Brazil)", () => {
  beforeEach(() => applyInterfaceLocale("pt-BR"));

  it("translates the About panel", () => {
    renderSettings("about");
    expect(screen.getByRole("heading", { name: "Sobre" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Verificar servidor" })).not.toBeInTheDocument();
  });

  it("translates the Models panel headings and model picker labels", () => {
    renderSettings("models");
    expect(screen.getByRole("heading", { name: "Modelos" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Voz" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Alterar modelo de transcrição" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Mais opções de voz" })).toBeInTheDocument();
  });

  it("reads tab labels in the current language", () => {
    expect(SETTINGS_TABS.find((tab) => tab.id === "general")?.label).toBe("Geral");
  });

  it("translates the mic test control", () => {
    renderMicTest();
    expect(screen.getByRole("heading", { name: "Teste do microfone" })).toBeInTheDocument();
    expect(screen.getByText("Teste o seu microfone.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Iniciar teste" })).toBeInTheDocument();
  });

  it("interpolates dictionary aria-labels and the no-match message", async () => {
    const user = userEvent.setup();
    render(<DictionarySettingsSection />);
    expect(await screen.findByRole("button", { name: "Editar Jane Doe" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Excluir Jane Doe" })).toBeInTheDocument();
    await user.type(screen.getByRole("searchbox", { name: "Buscar no dicionário" }), "xyz");
    expect(screen.getByText('Nenhuma entrada corresponde a "xyz".')).toBeInTheDocument();
  });

  it("re-renders when the language switches back to English", () => {
    renderMicTest();
    expect(screen.getByRole("button", { name: "Iniciar teste" })).toBeInTheDocument();
    act(() => applyInterfaceLocale("en"));
    expect(screen.getByRole("button", { name: "Start test" })).toBeInTheDocument();
  });
});
