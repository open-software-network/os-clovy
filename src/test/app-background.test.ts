import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  APP_BACKGROUND_CHANGED_EVENT,
  APP_BACKGROUND_STORAGE_KEY,
  getStoredAppBackground,
  normalizeAppBackground,
  setStoredAppBackground,
} from "../lib/app-background";

describe("app background preference", () => {
  beforeEach(() => localStorage.clear());
  it("defaults missing and invalid preferences to Drift", () => {
    expect(getStoredAppBackground()).toBe("drift");
    localStorage.setItem(APP_BACKGROUND_STORAGE_KEY, "other");
    expect(getStoredAppBackground()).toBe("drift");
    expect(normalizeAppBackground(undefined)).toBe("drift");
  });
  it.each(["drift", "image", "none"] as const)("persists and announces %s", (preference) => {
    const listener = vi.fn();
    window.addEventListener(APP_BACKGROUND_CHANGED_EVENT, listener);
    setStoredAppBackground(preference);
    expect(getStoredAppBackground()).toBe(preference);
    expect(listener).toHaveBeenCalledOnce();
    expect(listener.mock.calls[0][0].detail).toEqual({ preference });
    window.removeEventListener(APP_BACKGROUND_CHANGED_EVENT, listener);
  });
  it("announces the live choice even if storage rejects the write", () => {
    const listener = vi.fn();
    window.addEventListener(APP_BACKGROUND_CHANGED_EVENT, listener);
    const write = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("storage blocked");
    });
    expect(() => setStoredAppBackground("none")).not.toThrow();
    expect(listener.mock.calls[0][0].detail.preference).toBe("none");
    expect(getStoredAppBackground()).toBe("none");
    write.mockRestore();
    setStoredAppBackground("drift");
    window.removeEventListener(APP_BACKGROUND_CHANGED_EVENT, listener);
  });
});
