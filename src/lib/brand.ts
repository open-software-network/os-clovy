// Brand accent preference. The whole UI derives from the --brand token
// (src/styles/tokens.css) via var(--brand) and color-mix, so overriding that
// one custom property at runtime recolors buttons, washes, hovers, and the
// recorder accent in one shot. Five curated presets ship; sage is Clovy's
// default supporting accent, with a brighter leaf value for small controls and
// a low-chroma wash for large surfaces. The native dock icon swaps to the
// selected accent in Tauri builds. Sage keeps the canonical lime Clovy; the
// other presets tint the character itself as well as its dark icon tile.
//
// Each preset also carries a `wash`: the accent with its oklch chroma capped at
// 0.07 (same lightness/hue). The bundled app icon uses the bright lime-green
// Clovy mark; Appearance-selected dock icons follow the rule above.
// Surfaces tint from the wash, not the accent, so a
// high-chroma pick (clay) doesn't over-cream the greys while a low-chroma pick
// (rose) stays close to its accent. applyBrandVar sets both --brand and
// --brand-wash.
//
// Keep the storage key + the id->hex map in sync with the pre-paint bootstrap
// in index.html, which sets --brand and --brand-wash before the bundle runs to
// avoid a flash.

import { invoke } from "@tauri-apps/api/core";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { type MessageKey, t } from "../i18n/translate";

export type BrandId = "rose" | "clay" | "sage" | "ocean" | "plum";

export const BRAND_PRESETS: {
  id: BrandId;
  label: string;
  value: string;
  wash: string;
}[] = [
  brandPreset("sage", "lib.brand.sage", "#3f812f", "#5a7c56"),
  brandPreset("clay", "lib.brand.clay", "#b5551f", "#976851"),
  brandPreset("rose", "lib.brand.rose", "#a5655c", "#9e6961"),
  brandPreset("ocean", "lib.brand.ocean", "#3d7b9a", "#467a95"),
  brandPreset("plum", "lib.brand.plum", "#965d84", "#8f6380"),
];

function brandPreset(id: BrandId, key: MessageKey, value: string, wash: string) {
  return {
    id,
    get label() {
      return t(key);
    },
    value,
    wash,
  };
}

const STORAGE_KEY = "os-clovy:brand";
export const DEFAULT_BRAND: BrandId = "sage";

// Stored ids that have since been renamed or dropped. "blue" became "ocean"
// (same swatch slot); "amber" was removed and maps to "clay" so anyone who
// picked either keeps a matching warm accent instead of snapping back to the
// default. Mirror any entry added here in the index.html pre-paint map and
// theme_icon.rs.
const LEGACY_BRAND_IDS: Record<string, BrandId> = { blue: "ocean", amber: "clay" };

function presetFor(id: string | null) {
  const canonical = (id && LEGACY_BRAND_IDS[id]) ?? id;
  return (
    BRAND_PRESETS.find((preset) => preset.id === canonical) ??
    BRAND_PRESETS.find((preset) => preset.id === DEFAULT_BRAND) ??
    BRAND_PRESETS[0]
  );
}

export function getStoredBrand(): BrandId {
  try {
    return presetFor(localStorage.getItem(STORAGE_KEY)).id;
  } catch {
    // localStorage can throw in sandboxed contexts.
    return DEFAULT_BRAND;
  }
}

const ACCENT_EVENT = "clovy://accent";
const BRAND_TRANSITION_MS = 220;
const BRAND_TRANSITION_BUFFER_MS = 80;
let brandTransitionTimer: number | undefined;

function inTauri() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function prefersReducedMotion() {
  return (
    typeof window !== "undefined" && window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function applyWithTransition(apply: () => void) {
  const root = document.documentElement;
  if (prefersReducedMotion()) {
    apply();
    return;
  }

  window.clearTimeout(brandTransitionTimer);
  root.setAttribute("data-brand-transition", "true");
  window.requestAnimationFrame(() => {
    apply();
    brandTransitionTimer = window.setTimeout(() => {
      root.removeAttribute("data-brand-transition");
    }, BRAND_TRANSITION_MS + BRAND_TRANSITION_BUFFER_MS);
  });
}

export function setStoredBrand(id: BrandId) {
  try {
    localStorage.setItem(STORAGE_KEY, id);
  } catch {
    // Apply still works for this session.
  }
  applyBrand(id, { animate: true });
  // Tell the separate HUD webviews (agent/meeting/recording) to recolor too.
  if (inTauri()) {
    window.setTimeout(() => {
      void import("@tauri-apps/api/event")
        .then(({ emit }) => emit(ACCENT_EVENT, id))
        .catch(() => {});
    }, BRAND_TRANSITION_MS);
  }
}

// CSS-only apply: inline style on <html> overrides the :root defaults in
// tokens.css and cascades to every var(--brand) / var(--brand-wash) consumer.
// Fixed across light/dark, so a single pair covers both themes.
export function applyBrandVar(id: BrandId, options: { animate?: boolean } = {}) {
  const apply = () => {
    const preset = presetFor(id);
    const root = document.documentElement;
    root.dataset.brand = preset.id;
    root.style.setProperty("--brand", preset.value);
    root.style.setProperty("--brand-wash", preset.wash);
  };
  if (options.animate) {
    applyWithTransition(apply);
  } else {
    apply();
  }
}

// Main window: recolor + swap the native dock icon.
export function applyBrand(id: BrandId, options: { animate?: boolean } = {}) {
  applyBrandVar(id, options);
  syncDockIcon(id, options.animate ? BRAND_TRANSITION_MS : 0);
}

// Swap the native macOS dock/Cmd-Tab icon to match the accent. No-op on the
// web preview (no Tauri) and on builds that predate the command.
function syncDockIcon(id: BrandId, delayMs = 0) {
  if (!inTauri()) return;
  window.setTimeout(() => {
    void invoke("set_dock_icon", { brand: id }).catch(() => {
      // Best-effort: keep the bundled default icon if the command is absent.
    });
  }, delayMs);
}

export function initBrand() {
  applyBrand(getStoredBrand());
}

// Secondary windows (HUDs): apply the stored accent on load and keep it in
// sync when the main window changes it.
export function subscribeBrand(): Promise<UnlistenFn> {
  applyBrandVar(getStoredBrand());
  if (!inTauri()) return Promise.resolve(() => {});
  return import("@tauri-apps/api/event").then(({ listen }) =>
    listen<BrandId>(ACCENT_EVENT, (event) => applyBrandVar(event.payload, { animate: true })),
  );
}
