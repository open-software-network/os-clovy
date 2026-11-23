import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: vi.fn(async () => {
    throw new Error("no tauri in tests");
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => undefined),
}));

import { AppSettings } from "../components/settings/AppSettings";
import {
  applyInterfaceLocale,
  getInterfaceLocale,
  INTERFACE_LOCALE_STORAGE_KEY,
  initInterfaceLocale,
  resetInterfaceLocaleForTests,
} from "../i18n/locale";

const DICTATION_LANGUAGE_KEY_PATTERN = /dictation|transcription/i;

function renderAppearance() {
  return render(
    <AppSettings
      sourceMode="microphoneOnly"
      checkingSourceReadiness={false}
      onSourceModeChange={() => {}}
      onEnableSystemAudio={() => {}}
      activeTab="appearance"
      onTabChange={() => {}}
    />,
  );
}

beforeEach(() => {
  window.localStorage.removeItem(INTERFACE_LOCALE_STORAGE_KEY);
  resetInterfaceLocaleForTests();
  applyInterfaceLocale("en");
});

afterEach(() => {
  cleanup();
  window.localStorage.removeItem(INTERFACE_LOCALE_STORAGE_KEY);
  applyInterfaceLocale("en");
  resetInterfaceLocaleForTests();
});

describe("interface language setting", () => {
  it("switches the interface to Português (Brasil), persists it, and can switch back", async () => {
    const user = userEvent.setup();
    const keysBefore = new Set(Object.keys(window.localStorage));
    renderAppearance();

    expect(screen.getByRole("heading", { name: "Appearance" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Interface language: English/ }));
    await user.click(await screen.findByRole("option", { name: "Português (Brasil)" }));

    await waitFor(() =>
      expect(screen.getByRole("heading", { name: "Aparência" })).toBeInTheDocument(),
    );
    expect(screen.getByRole("heading", { name: "Idioma da interface" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Tema" })).toBeInTheDocument();
    expect(window.localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("pt-BR");
    expect(document.documentElement.lang).toBe("pt-BR");
    // The interface language is not the dictation language.
    const newKeys = Object.keys(window.localStorage).filter((key) => !keysBefore.has(key));
    expect(newKeys.filter((key) => DICTATION_LANGUAGE_KEY_PATTERN.test(key))).toEqual([]);

    // Relaunch: the stored choice is restored.
    cleanup();
    resetInterfaceLocaleForTests();
    act(() => initInterfaceLocale());
    expect(getInterfaceLocale()).toBe("pt-BR");
    renderAppearance();
    expect(screen.getByRole("heading", { name: "Aparência" })).toBeInTheDocument();

    // The way back stays reachable: the option is always labeled in English.
    await user.click(screen.getByRole("button", { name: /Idioma da interface: Português/ }));
    await user.click(await screen.findByRole("option", { name: "English" }));
    await waitFor(() =>
      expect(screen.getByRole("heading", { name: "Appearance" })).toBeInTheDocument(),
    );
    expect(window.localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("en");
  });
});
