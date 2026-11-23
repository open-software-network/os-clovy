import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { MotionGlobalConfig } from "framer-motion";
import { createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, vi } from "vitest";
import { Toaster, toast } from "../components/ui/Toaster";
import { markOnboardingComplete } from "../lib/onboarding";

// Resolve framer-motion animations instantly. Without this the frameloop
// stalls when tests swap fake/real timers, leaving AnimatePresence exits
// stuck in the DOM and exit-dependent assertions flaky.
MotionGlobalConfig.skipAnimations = true;

// A formatter without a locale follows the machine's LANG, so a pt-BR machine
// would read "Segunda-feira" and "10.000" wherever code formats with the
// system's locale. Pin that default to en-US so every suite formats the same
// on every machine; an explicit locale (the interface language) still wins.
// A suite that proves the interface language beats the system's sets
// `globalThis.__clovyTestSystemLocale` to a different tag for its duration.
const TEST_DEFAULT_LOCALE = "en-US";
declare global {
  var __clovyTestSystemLocale: string | undefined;
}
const testSystemLocale = () => globalThis.__clovyTestSystemLocale ?? TEST_DEFAULT_LOCALE;
function withTestDefaultLocale<T extends object>(formatter: T): T {
  return new Proxy(formatter, {
    construct: (target, [locales, ...rest]) =>
      Reflect.construct(target as new (...args: unknown[]) => object, [
        locales ?? testSystemLocale(),
        ...rest,
      ]),
    apply: (target, thisArg, [locales, ...rest]) =>
      Reflect.apply(target as (...args: unknown[]) => unknown, thisArg, [
        locales ?? testSystemLocale(),
        ...rest,
      ]),
  });
}
for (const name of [
  "DateTimeFormat",
  "NumberFormat",
  "RelativeTimeFormat",
  "PluralRules",
  "ListFormat",
] as const) {
  Object.defineProperty(Intl, name, {
    configurable: true,
    writable: true,
    value: withTestDefaultLocale(Intl[name]),
  });
}
for (const method of ["toLocaleString", "toLocaleDateString", "toLocaleTimeString"] as const) {
  const native = Date.prototype[method];
  Date.prototype[method] = function (
    this: Date,
    locales?: Intl.LocalesArgument,
    options?: Intl.DateTimeFormatOptions,
  ) {
    return native.call(this, locales ?? testSystemLocale(), options);
  };
}
const nativeNumberToLocaleString = Number.prototype.toLocaleString;
Number.prototype.toLocaleString = function (
  this: number,
  locales?: Intl.LocalesArgument,
  options?: Intl.NumberFormatOptions,
) {
  return nativeNumberToLocaleString.call(this, locales ?? testSystemLocale(), options);
};

// jsdom ships no ResizeObserver; the SegmentedControl relies on one.
if (!("ResizeObserver" in globalThis)) {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

if (!document.elementFromPoint) {
  document.elementFromPoint = () => document.body;
}

if (!window.matchMedia) {
  window.matchMedia = vi.fn().mockImplementation((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }));
}

// Web Storage polyfill for Nodes where jsdom's localStorage never reaches the
// test global. Node 24+ ships its own `localStorage` global that is present
// but unavailable without `--localstorage-file` (the getter returns undefined
// and emits an ExperimentalWarning). Vitest's populateGlobal skips window keys
// that already exist on the Node global, so jsdom's real Storage is shadowed
// and unrecoverable there; this shim is the only storage the suite gets. It
// must therefore be spec-faithful: an internal Map is the source of truth,
// and stored keys are mirrored as own enumerable properties (so
// `Object.keys(localStorage)` returns stored keys, like a real Storage) -
// except keys that collide with an interface member (e.g. "getItem"), which
// per WebIDL named-property semantics never shadow the method and stay hidden
// from enumeration while remaining retrievable via getItem.
// Note it is still not `instanceof Storage`; tests that spy on storage keep
// the `instanceof Storage ? Storage.prototype : window.localStorage` branch.
if (
  !("localStorage" in globalThis) ||
  !globalThis.localStorage ||
  typeof globalThis.localStorage.clear !== "function"
) {
  const values = new Map<string, string>();
  const storageProto = {
    clear(this: Record<string, string>) {
      values.clear();
      for (const key of Object.keys(this)) {
        delete this[key];
      }
    },
    getItem(key: string) {
      return values.get(String(key)) ?? null;
    },
    key(index: number) {
      return Array.from(values.keys())[index] ?? null;
    },
    get length() {
      return values.size;
    },
    removeItem(this: Record<string, string>, key: string) {
      values.delete(String(key));
      if (!(key in storageProto)) {
        delete this[key];
      }
    },
    setItem(this: Record<string, string>, key: string, value: string) {
      values.set(String(key), String(value));
      if (!(key in storageProto)) {
        this[key] = String(value);
      }
    },
  };
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: Object.create(storageProto),
  });
}

if (!Range.prototype.getClientRects) {
  Range.prototype.getClientRects = () =>
    ({
      length: 0,
      item: () => null,
      [Symbol.iterator]: function* () {},
    }) as DOMRectList;
}

if (!Range.prototype.getBoundingClientRect) {
  Range.prototype.getBoundingClientRect = () => new DOMRect();
}

if (!HTMLElement.prototype.getClientRects) {
  HTMLElement.prototype.getClientRects = () =>
    ({
      length: 0,
      item: () => null,
      [Symbol.iterator]: function* () {},
    }) as DOMRectList;
}

if (!HTMLElement.prototype.getBoundingClientRect) {
  HTMLElement.prototype.getBoundingClientRect = () => new DOMRect();
}

function setNavigatorPlatform(platform: string, userAgent: string) {
  Object.defineProperty(navigator, "platform", {
    configurable: true,
    get: () => platform,
  });
  Object.defineProperty(navigator, "userAgent", {
    configurable: true,
    get: () => userAgent,
  });
}

// Existing App tests exercise the signed-in main shell; pre-complete the
// first-run onboarding so the wizard doesn't gate them. Onboarding tests
// opt back in by clearing localStorage.
beforeEach(() => {
  setNavigatorPlatform("MacIntel", "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5)");
  markOnboardingComplete();
});

// The app mounts the toast host once in the App shell, but component tests
// render surfaces (e.g. AgentWorkspace) without it. Mount a single Toaster once,
// into its own host outside React Testing Library's container so cleanup()
// never unmounts it — any toast(...) a surface fires portals to document.body
// where `screen` queries can still find it. Mounting once (not per test) keeps
// zero per-test render churn, so timing-sensitive tests aren't perturbed.
// Surface-fired toasts carry stable ids (model switch, branch, issue-report
// sent, busy) so a toast re-fired in a later test updates the earlier one in
// place rather than stacking a duplicate node.
const toasterHost = document.createElement("div");
document.body.appendChild(toasterHost);
createRoot(toasterHost).render(createElement(Toaster));

// Clear any toasts a test left behind so they never leak into the next one.
// Sonner keeps a dismissed toast mounted for its 200 ms exit transition. Let
// that removal finish before Vitest can tear down jsdom; otherwise the delayed
// React state update can run after `window` has been removed and fail an
// otherwise-passing release suite. Tests that never create a toast pay no wait.
afterEach(async () => {
  const activeToasts = toast.getToasts();
  if (activeToasts.length > 0) vi.useRealTimers();
  for (const activeToast of activeToasts) toast.dismiss(activeToast.id);
  cleanup();
  if (activeToasts.length > 0) {
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
});
