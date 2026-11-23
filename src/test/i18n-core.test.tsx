import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { catalogs } from "../i18n/catalog";
import {
  applyInterfaceLocale,
  getInterfaceLocale,
  INTERFACE_LOCALE_OPTIONS,
  INTERFACE_LOCALE_STORAGE_KEY,
  initInterfaceLocale,
  normalizeLocale,
  resetInterfaceLocaleForTests,
  setInterfaceLocale,
  subscribeInterfaceLocaleAcrossWindows,
} from "../i18n/locale";
import { useT, useTRich } from "../i18n/react";
import { formatDate, formatNumber, intlLocale, t } from "../i18n/translate";
import { markOnboardingComplete, resetOnboardingForReplay } from "../lib/onboarding";

function setNavigatorLanguages(languages: string[]) {
  vi.spyOn(navigator, "languages", "get").mockReturnValue(languages);
  vi.spyOn(navigator, "language", "get").mockReturnValue(languages[0] ?? "");
}

beforeEach(() => {
  window.localStorage.removeItem(INTERFACE_LOCALE_STORAGE_KEY);
  resetInterfaceLocaleForTests();
});

afterEach(() => {
  vi.restoreAllMocks();
  window.localStorage.removeItem(INTERFACE_LOCALE_STORAGE_KEY);
  markOnboardingComplete();
  applyInterfaceLocale("en");
  resetInterfaceLocaleForTests();
});

describe("interface locale preference", () => {
  it("offers English and Português (Brasil) by their native names", () => {
    expect(INTERFACE_LOCALE_OPTIONS).toEqual([
      { value: "en", label: "English" },
      { value: "pt-BR", label: "Português (Brasil)" },
    ]);
  });

  it("normalizes Portuguese variants and rejects unsupported or malformed values", () => {
    expect(normalizeLocale("pt-BR")).toBe("pt-BR");
    expect(normalizeLocale("pt_br")).toBe("pt-BR");
    expect(normalizeLocale("pt")).toBe("pt-BR");
    expect(normalizeLocale("en-GB")).toBe("en");
    expect(normalizeLocale("fr")).toBeUndefined();
    expect(normalizeLocale("")).toBeUndefined();
    expect(normalizeLocale(42)).toBeUndefined();
  });

  it("persists a change and restores it after a relaunch", () => {
    initInterfaceLocale();
    setInterfaceLocale("pt-BR");
    expect(window.localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("pt-BR");
    expect(document.documentElement.lang).toBe("pt-BR");

    // Simulate a relaunch: in-memory state is gone, storage survives.
    resetInterfaceLocaleForTests();
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("pt-BR");
    expect(t("common.cancel")).toBe("Cancelar");

    setInterfaceLocale("en");
    resetInterfaceLocaleForTests();
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("en");
    expect(document.documentElement.lang).toBe("en");
  });

  it("falls back to English for an invalid stored value and repairs it", () => {
    window.localStorage.setItem(INTERFACE_LOCALE_STORAGE_KEY, "klingon");
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("en");
    expect(window.localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("en");
  });

  it("falls back to English for an invalid stored value even on a fresh Portuguese system", () => {
    setNavigatorLanguages(["pt-BR"]);
    window.localStorage.removeItem("clovy.onboarding.completedVersion");
    window.localStorage.removeItem("june.onboarding.completedVersion");
    window.localStorage.setItem(INTERFACE_LOCALE_STORAGE_KEY, "fr");
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("en");
    expect(window.localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("en");
  });

  it("keeps existing installs in English even on a Portuguese system", () => {
    setNavigatorLanguages(["pt-BR", "en-US"]);
    markOnboardingComplete();
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("en");
    // The migration is written down so later onboarding changes cannot flip it.
    expect(window.localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY)).toBe("en");
  });

  it("starts a fresh install in Portuguese on a Portuguese system and keeps it after onboarding", () => {
    setNavigatorLanguages(["pt-BR"]);
    resetOnboardingForReplay();
    window.localStorage.removeItem("clovy.onboarding.completedVersion");
    window.localStorage.removeItem("june.onboarding.completedVersion");
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("pt-BR");

    markOnboardingComplete();
    resetInterfaceLocaleForTests();
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("pt-BR");
  });

  it("starts a fresh install on an unsupported system language in English", () => {
    setNavigatorLanguages(["de-DE"]);
    window.localStorage.removeItem("clovy.onboarding.completedVersion");
    window.localStorage.removeItem("june.onboarding.completedVersion");
    initInterfaceLocale();
    expect(getInterfaceLocale()).toBe("en");
  });

  it("follows a change made in another window through the storage event", () => {
    const unsubscribe = subscribeInterfaceLocaleAcrossWindows();
    expect(getInterfaceLocale()).toBe("en");
    window.localStorage.setItem(INTERFACE_LOCALE_STORAGE_KEY, "pt-BR");
    window.dispatchEvent(
      new StorageEvent("storage", { key: INTERFACE_LOCALE_STORAGE_KEY, newValue: "pt-BR" }),
    );
    expect(getInterfaceLocale()).toBe("pt-BR");
    unsubscribe();
  });

  it("never touches the dictation language", () => {
    const before = { ...window.localStorage };
    setInterfaceLocale("pt-BR");
    const changed = Object.keys(window.localStorage).filter(
      (key) => window.localStorage.getItem(key) !== (before as Record<string, string>)[key],
    );
    expect(changed.every((key) => key.includes("interface-locale") || key.startsWith("__"))).toBe(
      true,
    );
  });
});

describe("translation", () => {
  it("interpolates and pluralizes per locale", () => {
    expect(t("common.items", { count: 1 }, "en")).toBe("1 item");
    expect(t("common.items", { count: 3 }, "en")).toBe("3 items");
    expect(t("common.items", { count: 1 }, "pt-BR")).toBe("1 item");
    expect(t("common.items", { count: 3 }, "pt-BR")).toBe("3 itens");
    // Brazilian usage reads zero as plural.
    expect(t("common.items", { count: 0 }, "pt-BR")).toBe("0 itens");
    // Numbers use the locale's grouping.
    expect(t("common.items", { count: 1234 }, "pt-BR")).toBe("1.234 itens");
    // English keeps the raw digits its copy always used.
    expect(t("common.items", { count: 1234 }, "en")).toBe("1234 items");
  });

  it("returns the key itself for an unknown key instead of crashing", () => {
    // @ts-expect-error deliberately unknown key
    expect(t("nope.missing")).toBe("nope.missing");
  });

  it("formats numbers and dates in the interface language, never the system's", () => {
    expect(intlLocale("en")).toBe("en");
    expect(intlLocale("pt-BR")).toBe("pt-BR");
    expect(formatNumber(1234.5, undefined, "pt-BR")).toBe("1.234,5");
    expect(
      formatNumber(1.5, { style: "currency", currency: "USD" }, "pt-BR").replace(/\s/g, " "),
    ).toBe("US$ 1,50");
    const date = new Date(2026, 8, 26, 12);
    expect(formatDate(date, { day: "numeric", month: "long" }, "pt-BR")).toBe("26 de setembro");
  });

  it("re-renders components when the language changes", () => {
    function Probe() {
      const tr = useT();
      return <button type="button">{tr("common.cancel")}</button>;
    }
    render(<Probe />);
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    act(() => setInterfaceLocale("pt-BR"));
    expect(screen.getByRole("button", { name: "Cancelar" })).toBeInTheDocument();
    act(() => setInterfaceLocale("en"));
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
  });

  it("renders rich messages with React nodes", () => {
    function Probe() {
      const tr = useTRich();
      return <p>{tr("common.items", { count: 2 })}</p>;
    }
    applyInterfaceLocale("pt-BR");
    render(<Probe />);
    expect(screen.getByText("2 itens")).toBeInTheDocument();
  });

  it("keeps raw digits in English rich messages", () => {
    function Probe() {
      const tr = useTRich();
      return <p>{tr("common.items", { count: 1234 })}</p>;
    }
    render(<Probe />);
    expect(screen.getByText("1234 items")).toBeInTheDocument();
  });
});

describe("catalog integrity", () => {
  const PLACEHOLDER = /\{(\w+)\}|<\/?(\w+)>/g;
  function tokens(message: unknown): string[] {
    const text = typeof message === "string" ? message : Object.values(message as object).join(" ");
    return [...new Set(Array.from(text.matchAll(PLACEHOLDER), (m) => m[0]))].sort();
  }

  it("has a Portuguese translation for every English key", () => {
    const missing = Object.keys(catalogs.en).filter((key) => !(key in catalogs["pt-BR"]));
    expect(missing).toEqual([]);
  });

  it("keeps the same placeholders and markup in both languages", () => {
    const mismatched = Object.keys(catalogs.en).filter((key) => {
      const en = catalogs.en[key];
      const pt = catalogs["pt-BR"][key];
      // Plural forms may drop {count} in a singular ("Uma nota"), so compare
      // the union across forms.
      return JSON.stringify(tokens(en)) !== JSON.stringify(tokens(pt));
    });
    expect(mismatched).toEqual([]);
  });

  it("uses no typographic dashes or plural shape mismatches in Portuguese copy", () => {
    const offenders = Object.entries(catalogs["pt-BR"]).filter(([key, message]) => {
      const text = typeof message === "string" ? message : Object.values(message).join(" ");
      const shapeMismatch = typeof message !== typeof catalogs.en[key];
      return /[–—]/.test(text) || shapeMismatch;
    });
    expect(offenders.map(([key]) => key)).toEqual([]);
  });

  it("keeps every key inside its namespace prefix without duplicates", async () => {
    const { namespaces } = await import("../i18n/catalog");
    const seen = new Map<string, string>();
    const problems: string[] = [];
    for (const [name, namespace] of Object.entries(namespaces)) {
      for (const key of Object.keys(namespace.en)) {
        if (!key.startsWith(`${name}.`)) problems.push(`${key} is not prefixed with ${name}.`);
        if (seen.has(key)) problems.push(`${key} duplicated in ${seen.get(key)} and ${name}`);
        seen.set(key, name);
      }
    }
    expect(problems).toEqual([]);
  });
});
