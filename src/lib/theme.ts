export type ThemePreference = "system" | "light" | "dark";
type ResolvedTheme = "light" | "dark";

type StartViewTransition = (callback: () => void) => unknown;

const STORAGE_KEY = "os-clovy:theme";
const VALID: ThemePreference[] = ["system", "light", "dark"];

let systemMedia: MediaQueryList | undefined;
let systemListener: ((event: MediaQueryListEvent) => void) | undefined;

export function getStoredTheme(): ThemePreference {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw && (VALID as string[]).includes(raw)) {
      return raw as ThemePreference;
    }
  } catch {
    // localStorage can throw in sandboxed contexts.
  }
  return "dark";
}

export function setStoredTheme(theme: ThemePreference) {
  try {
    localStorage.setItem(STORAGE_KEY, theme);
  } catch {
    // Apply still works for this session.
  }
  withTransition(() => applyTheme(theme));
}

export function applyTheme(theme: ThemePreference) {
  const resolved = resolveTheme(theme);
  document.documentElement.setAttribute("data-theme", resolved);
  if (theme === "system") {
    attachSystemListener();
  } else {
    detachSystemListener();
  }
}

export function initTheme() {
  applyTheme(getStoredTheme());
}

function resolveTheme(theme: ThemePreference): ResolvedTheme {
  if (theme === "light" || theme === "dark") return theme;
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return "light";
  }
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function attachSystemListener() {
  if (systemListener) return;
  if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
    return;
  }
  systemMedia = window.matchMedia("(prefers-color-scheme: dark)");
  systemListener = (event) => {
    withTransition(() => {
      document.documentElement.setAttribute("data-theme", event.matches ? "dark" : "light");
    });
  };
  systemMedia.addEventListener("change", systemListener);
}

function detachSystemListener() {
  if (!systemMedia || !systemListener) return;
  systemMedia.removeEventListener("change", systemListener);
  systemMedia = undefined;
  systemListener = undefined;
}

function withTransition(mutate: () => void) {
  const start = (document as Document & { startViewTransition?: StartViewTransition })
    .startViewTransition;
  if (typeof start === "function") {
    start.call(document, mutate);
  } else {
    mutate();
  }
}
