// Interface language preference. This is the language Clovy's own chrome is
// written in (labels, dialogs, menus, HUDs). It is deliberately independent of
// the dictation/transcription language hint in `lib/dictation-languages.ts`:
// someone can read Clovy in Portuguese while dictating in English, or the
// reverse, and changing one never changes the other.
//
// Persisted like the theme/brand preferences: localStorage in the shared
// webview origin, read by the main window and every HUD entry point, plus a
// Tauri event so already-open HUD windows and the native menus follow a change
// without a relaunch.

import { hasCompletedAnyOnboardingVersion } from "../lib/onboarding";

export const SUPPORTED_LOCALES = ["en", "pt-BR"] as const;
export type InterfaceLocale = (typeof SUPPORTED_LOCALES)[number];

export const DEFAULT_LOCALE: InterfaceLocale = "en";
export const INTERFACE_LOCALE_STORAGE_KEY = "os-clovy:interface-locale";
/** Same-window change notification (React subscribers, non-React surfaces). */
export const INTERFACE_LOCALE_CHANGED_EVENT = "clovy://interface-locale-change";
/** Cross-window Tauri event: HUD webviews and the native app/tray menus. */
export const INTERFACE_LOCALE_TAURI_EVENT = "clovy://interface-locale";
/** Rust asks the main window for the language once its listener exists. */
export const INTERFACE_LOCALE_REQUEST_EVENT = "clovy://interface-locale-request";

/** Native names: each option is written in its own language so a reader who
 * picked the wrong one can always find their way back. */
export const INTERFACE_LOCALE_OPTIONS: { value: InterfaceLocale; label: string }[] = [
  { value: "en", label: "English" },
  { value: "pt-BR", label: "Português (Brasil)" },
];

let currentLocale: InterfaceLocale | undefined;

function inTauri() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Map any stored or system value onto a supported locale. Unknown, empty, or
 * malformed values fall back to English; every Portuguese variant maps to
 * pt-BR (the only Portuguese catalog).
 */
export function normalizeLocale(value: unknown): InterfaceLocale | undefined {
  if (typeof value !== "string") return undefined;
  const tag = value.trim().replace(/_/g, "-").toLowerCase();
  if (!tag) return undefined;
  if (tag === "pt" || tag.startsWith("pt-")) return "pt-BR";
  if (tag === "en" || tag.startsWith("en-")) return "en";
  return undefined;
}

function readRawStoredLocale(): string | null {
  try {
    return window.localStorage.getItem(INTERFACE_LOCALE_STORAGE_KEY);
  } catch {
    return null;
  }
}

function systemLocale(): InterfaceLocale {
  if (typeof navigator === "undefined") return DEFAULT_LOCALE;
  const candidates = [...(navigator.languages ?? []), navigator.language];
  for (const candidate of candidates) {
    const locale = normalizeLocale(candidate);
    if (locale) return locale;
  }
  return DEFAULT_LOCALE;
}

/**
 * First-run default when nothing is stored yet. Installs that already finished
 * onboarding (at any version) predate the language setting and have always
 * seen English, so they keep English instead of flipping under the user. A
 * genuinely fresh install follows the system language when it is supported.
 */
export function initialLocale(): InterfaceLocale {
  return hasCompletedAnyOnboardingVersion() ? DEFAULT_LOCALE : systemLocale();
}

/**
 * The locale a window starts in. The first-run default applies only when
 * nothing is stored; a stored value that is not a supported language ("fr",
 * garbage) falls back to English rather than to the system language.
 */
function resolveStartupLocale(raw: string | null = readRawStoredLocale()): InterfaceLocale {
  if (raw === null) return initialLocale();
  return normalizeLocale(raw) ?? DEFAULT_LOCALE;
}

export function getInterfaceLocale(): InterfaceLocale {
  if (currentLocale) return currentLocale;
  currentLocale = resolveStartupLocale();
  return currentLocale;
}

function applyDocumentLanguage(locale: InterfaceLocale) {
  if (typeof document === "undefined") return;
  document.documentElement.lang = locale;
}

function notify(locale: InterfaceLocale) {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent<InterfaceLocale>(INTERFACE_LOCALE_CHANGED_EVENT, { detail: locale }),
  );
}

/** Apply a locale in this window only (no persistence, no broadcast). */
export function applyInterfaceLocale(value: unknown) {
  const locale = normalizeLocale(value) ?? DEFAULT_LOCALE;
  const changed = locale !== currentLocale;
  currentLocale = locale;
  applyDocumentLanguage(locale);
  if (changed) notify(locale);
}

function persist(locale: InterfaceLocale) {
  try {
    window.localStorage.setItem(INTERFACE_LOCALE_STORAGE_KEY, locale);
  } catch {
    // Locked-down WebViews may reject storage writes; the live choice still
    // applies for this session.
  }
}

function broadcast(locale: InterfaceLocale) {
  if (!inTauri()) return;
  void import("@tauri-apps/api/event")
    .then(({ emit }) => emit(INTERFACE_LOCALE_TAURI_EVENT, locale))
    .catch(() => {});
}

export function setInterfaceLocale(value: InterfaceLocale) {
  const locale = normalizeLocale(value) ?? DEFAULT_LOCALE;
  persist(locale);
  applyInterfaceLocale(locale);
  broadcast(locale);
}

/**
 * Main-window startup. Resolves the stored preference (or the first-run
 * default), writes it back so the choice is stable across relaunches and
 * onboarding completion, and tells the native menus which language to use.
 */
export function initInterfaceLocale() {
  const raw = readRawStoredLocale();
  const locale = resolveStartupLocale(raw);
  // Also rewrites an invalid or non-canonical stored value ("pt_BR", "fr").
  if (raw !== locale) persist(locale);
  applyInterfaceLocale(locale);
  broadcast(locale);
  answerLocaleRequests();
}

/**
 * The window can load before Rust registers its listener (Tauri builds the
 * configured window before `setup`), losing the broadcast above. Rust asks
 * once its listener exists; the window answers, and broadcasts once more when
 * it can hear the request, so one of the two always lands.
 */
function answerLocaleRequests() {
  if (!inTauri()) return;
  void import("@tauri-apps/api/event")
    .then(({ listen }) =>
      listen(INTERFACE_LOCALE_REQUEST_EVENT, () => broadcast(getInterfaceLocale())),
    )
    .then(() => broadcast(getInterfaceLocale()))
    .catch(() => {});
}

/**
 * Secondary windows (HUDs): apply the stored locale and follow changes made in
 * the main window. Returns an unsubscribe function.
 */
export function subscribeInterfaceLocaleAcrossWindows(): () => void {
  applyInterfaceLocale(resolveStartupLocale());
  const onStorage = (event: StorageEvent) => {
    if (event.key === INTERFACE_LOCALE_STORAGE_KEY) applyInterfaceLocale(event.newValue);
  };
  window.addEventListener("storage", onStorage);
  let unlisten: (() => void) | undefined;
  let disposed = false;
  if (inTauri()) {
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen<string>(INTERFACE_LOCALE_TAURI_EVENT, (event) =>
          applyInterfaceLocale(event.payload),
        ),
      )
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {});
  }
  return () => {
    disposed = true;
    window.removeEventListener("storage", onStorage);
    unlisten?.();
  };
}

export function subscribeInterfaceLocale(onChange: (locale: InterfaceLocale) => void) {
  const handler = (event: Event) => onChange((event as CustomEvent<InterfaceLocale>).detail);
  window.addEventListener(INTERFACE_LOCALE_CHANGED_EVENT, handler);
  return () => window.removeEventListener(INTERFACE_LOCALE_CHANGED_EVENT, handler);
}

/** Test-only: forget the cached locale so the next read hits storage. */
export function resetInterfaceLocaleForTests() {
  currentLocale = undefined;
}
