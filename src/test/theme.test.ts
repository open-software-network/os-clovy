import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyTheme, getStoredTheme, initTheme } from "../lib/theme";

describe("initial theme", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.stubGlobal(
      "matchMedia",
      vi.fn(() => ({
        matches: false,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
  });
  afterEach(() => {
    applyTheme("dark");
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it.each([null, "invalid"])("defaults to dark with stored value %s", (stored) => {
    if (stored) localStorage.setItem("os-clovy:theme", stored);
    expect(getStoredTheme()).toBe("dark");
    initTheme();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it.each(["light", "dark", "system"] as const)("honors saved %s", (theme) => {
    localStorage.setItem("os-clovy:theme", theme);
    expect(getStoredTheme()).toBe(theme);
    initTheme();
    expect(document.documentElement.dataset.theme).toBe(theme === "system" ? "light" : theme);
  });

  describe.each([
    "index.html",
    "styleguide.html",
    "onboarding-preview.html",
  ])("%s before paint", (file) => {
    const html = readFileSync(file, "utf8");
    const script = html.match(/<script>([\s\S]*?)<\/script>/)?.[1] ?? "";
    it.each([
      null,
      "invalid",
      "light",
      "dark",
      "system",
    ])("resolves %s before React loads", (saved) => {
      if (saved) localStorage.setItem("os-clovy:theme", saved);
      document.documentElement.removeAttribute("data-theme");
      new Function(script)();
      expect(document.documentElement.dataset.theme).toBe(
        saved === "light" || saved === "system" ? "light" : "dark",
      );
    });
    it("stays dark when storage is unavailable", () => {
      vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
        throw new Error("storage blocked");
      });
      document.documentElement.removeAttribute("data-theme");
      new Function(script)();
      expect(document.documentElement.dataset.theme).toBe("dark");
      expect(getStoredTheme()).toBe("dark");
    });
  });
});
