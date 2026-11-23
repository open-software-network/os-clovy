// Text-size preference. The whole app's type derives from the --fs-* tokens
// (src/styles/tokens.css), and each of those is `base * var(--font-scale)`, so
// overriding that one custom property at runtime rescales every label, body,
// and heading in one shot — the same override-one-var trick the brand accent
// uses. Line-heights are unitless and spacing/control heights stay fixed, so
// text grows within the existing chrome instead of reflowing the shell.
//
// Persisted like the theme/brand preferences: localStorage only, plus a
// pre-paint bootstrap in index.html that sets --font-scale before the bundle
// runs so a non-default size doesn't flash the base scale first. Keep the
// storage key and the id->multiplier map in sync with that bootstrap.

import { useSyncExternalStore } from "react";
import { type MessageKey, t } from "../i18n/translate";
import { isMacLikePlatform } from "./platform";

export type FontScaleId = "default" | "large" | "larger";

// Up-only ladder: Clovy is a reading app, so there's no "denser" step — the two
// larger stops cover "comfortable" and "big" without crowding the fixed chrome
// (spacing, control heights, and icon sizes stay put at these deltas).
export const FONT_SCALE_PRESETS: {
  id: FontScaleId;
  label: string;
  value: number;
}[] = [
  preset("default", "lib.fontScale.default", 1),
  preset("large", "lib.fontScale.large", 1.1),
  preset("larger", "lib.fontScale.larger", 1.2),
];

function preset(id: FontScaleId, key: MessageKey, value: number) {
  return {
    id,
    get label() {
      return t(key);
    },
    value,
  };
}

const STORAGE_KEY = "os-clovy:font-scale";
export const FONT_SCALE_CHANGED_EVENT = "clovy://font-scale-change";
export const DEFAULT_FONT_SCALE: FontScaleId = "default";
let sessionFontScale: FontScaleId = DEFAULT_FONT_SCALE;
let storageWriteFailed = false;
let storageReadFailed = false;

function presetFor(id: string | null) {
  // Unknown ids (including a stored "small" from the dropped preset) resolve
  // to the default scale.
  return (
    FONT_SCALE_PRESETS.find((preset) => preset.id === id) ??
    FONT_SCALE_PRESETS.find((preset) => preset.id === DEFAULT_FONT_SCALE) ??
    FONT_SCALE_PRESETS[0]
  );
}

export function getStoredFontScale(): FontScaleId {
  if (storageWriteFailed) return sessionFontScale;
  try {
    const next = presetFor(localStorage.getItem(STORAGE_KEY)).id;
    const recoveredFromReadFailure = storageReadFailed;
    storageReadFailed = false;
    sessionFontScale = next;
    if (recoveredFromReadFailure) applyFontScale(next);
    return sessionFontScale;
  } catch {
    // A transient read failure should not prevent a later snapshot from retrying.
    storageReadFailed = true;
    return sessionFontScale;
  }
}

export function applyFontScale(id: FontScaleId) {
  document.documentElement.style.setProperty("--font-scale", String(presetFor(id).value));
}

export function setStoredFontScale(id: FontScaleId) {
  const next = presetFor(id).id;
  sessionFontScale = next;
  try {
    localStorage.setItem(STORAGE_KEY, next);
    storageWriteFailed = false;
    storageReadFailed = false;
  } catch {
    // The in-memory value keeps stepping and reset working for this session.
    storageWriteFailed = true;
  }
  applyFontScale(next);
  window.dispatchEvent(new CustomEvent<FontScaleId>(FONT_SCALE_CHANGED_EVENT, { detail: next }));
}

export function initFontScale() {
  storageWriteFailed = false;
  storageReadFailed = false;
  applyFontScale(getStoredFontScale());
}

function subscribeFontScale(onChange: () => void) {
  window.addEventListener(FONT_SCALE_CHANGED_EVENT, onChange);
  return () => window.removeEventListener(FONT_SCALE_CHANGED_EVENT, onChange);
}

export function useFontScaleId() {
  return useSyncExternalStore(subscribeFontScale, getStoredFontScale, () => DEFAULT_FONT_SCALE);
}

function stepStoredFontScale(delta: -1 | 1) {
  const current = getStoredFontScale();
  const currentIndex = FONT_SCALE_PRESETS.findIndex((preset) => preset.id === current);
  const nextIndex = Math.min(FONT_SCALE_PRESETS.length - 1, Math.max(0, currentIndex + delta));
  const next = FONT_SCALE_PRESETS[nextIndex]?.id ?? DEFAULT_FONT_SCALE;
  if (next !== current) setStoredFontScale(next);
}

function shortcutAction(event: KeyboardEvent): "increase" | "decrease" | "reset" | undefined {
  if (event.altKey) return undefined;

  const hasPrimaryModifier = isMacLikePlatform()
    ? event.metaKey && !event.ctrlKey
    : event.ctrlKey && !event.metaKey;
  if (!hasPrimaryModifier) return undefined;

  if (event.key === "+" || event.key === "=") return "increase";
  if (event.key === "-") return "decrease";
  if (event.key === "0") return "reset";
  if (event.code === "Equal" || event.code === "NumpadAdd") return "increase";
  if (event.code === "Minus" || event.code === "NumpadSubtract") return "decrease";
  if (event.code === "Digit0" || event.code === "Numpad0") return "reset";
  return undefined;
}

/**
 * Installs the standard desktop text zoom shortcuts. Clovy's scale is
 * intentionally up-only, so zooming out reverses a prior zoom-in and stops at
 * the default size.
 */
export function installFontScaleShortcuts() {
  const onKeyDown = (event: KeyboardEvent) => {
    const action = shortcutAction(event);
    if (!action) return;

    event.preventDefault();
    if (action === "increase") {
      stepStoredFontScale(1);
    } else if (action === "decrease") {
      stepStoredFontScale(-1);
    } else if (getStoredFontScale() !== DEFAULT_FONT_SCALE) {
      setStoredFontScale(DEFAULT_FONT_SCALE);
    }
  };

  window.addEventListener("keydown", onKeyDown);
  return () => window.removeEventListener("keydown", onKeyDown);
}
