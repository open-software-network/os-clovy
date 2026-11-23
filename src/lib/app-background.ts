import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

export const APP_BACKGROUND_STORAGE_KEY = "os-clovy:background";
export const APP_BACKGROUND_CHANGED_EVENT = "os-clovy:background-changed";

export type AppBackgroundPreference = "drift" | "image" | "none";
type AppBackgroundChangedDetail = { preference: AppBackgroundPreference };

// Preserve a live choice across navigation if this WebView rejects persistence.
let unpersistedPreference: AppBackgroundPreference | undefined;

export function normalizeAppBackground(value: unknown): AppBackgroundPreference {
  return value === "none" || value === "image" ? value : "drift";
}

export function getStoredAppBackground(): AppBackgroundPreference {
  if (unpersistedPreference) return unpersistedPreference;
  try {
    return normalizeAppBackground(window.localStorage.getItem(APP_BACKGROUND_STORAGE_KEY));
  } catch {
    return "drift";
  }
}

export function setStoredAppBackground(preference: AppBackgroundPreference) {
  const next = normalizeAppBackground(preference);
  try {
    window.localStorage.setItem(APP_BACKGROUND_STORAGE_KEY, next);
    unpersistedPreference = undefined;
  } catch {
    // Locked-down WebViews may reject storage writes. Keep the live choice.
    unpersistedPreference = next;
  }
  window.dispatchEvent(
    new CustomEvent<AppBackgroundChangedDetail>(APP_BACKGROUND_CHANGED_EVENT, {
      detail: { preference: next },
    }),
  );
}

export function useAppBackground() {
  const [preference, setPreference] = useState(getStoredAppBackground);
  useEffect(() => {
    const onChange = (event: Event) => {
      const detail = (event as CustomEvent<AppBackgroundChangedDetail>).detail;
      setPreference(normalizeAppBackground(detail?.preference));
    };
    const onStorage = (event: StorageEvent) => {
      if (event.key === APP_BACKGROUND_STORAGE_KEY || event.key === null) {
        setPreference(getStoredAppBackground());
      }
    };
    window.addEventListener(APP_BACKGROUND_CHANGED_EVENT, onChange);
    window.addEventListener("storage", onStorage);
    return () => {
      window.removeEventListener(APP_BACKGROUND_CHANGED_EVENT, onChange);
      window.removeEventListener("storage", onStorage);
    };
  }, []);
  return preference;
}

export type BackgroundImage = { fileName: string; dataUrl: string | null };
export const readBackgroundImage = () => invoke<BackgroundImage | null>("read_background_image");
export const chooseBackgroundImage = () =>
  invoke<BackgroundImage | null>("choose_background_image");
export const removeBackgroundImage = () => invoke<void>("remove_background_image");
