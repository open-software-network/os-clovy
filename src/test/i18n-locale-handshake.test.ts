import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  INTERFACE_LOCALE_STORAGE_KEY,
  INTERFACE_LOCALE_TAURI_EVENT,
  initInterfaceLocale,
} from "../i18n/locale";

/** The Tauri IPC the real `@tauri-apps/api/event` talks to. */
function fakeTauri() {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  const callbacks = new Map<number, (event: unknown) => void>();
  let next = 1;
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {
    transformCallback: (callback: (event: unknown) => void) => {
      const id = next++;
      callbacks.set(id, callback);
      return id;
    },
    invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
      calls.push({ cmd, args });
      return cmd === "plugin:event|listen" ? next++ : undefined;
    },
  };
  return {
    broadcasts: () =>
      calls.filter(
        ({ cmd, args }) =>
          cmd === "plugin:event|emit" && args.event === INTERFACE_LOCALE_TAURI_EVENT,
      ),
    /** Delivers `event` to the window's listener for it, as Rust would. */
    deliver: (event: string) => {
      const listen = calls.find(
        ({ cmd, args }) => cmd === "plugin:event|listen" && args.event === event,
      );
      const handler = callbacks.get(listen?.args.handler as number);
      handler?.({ event, id: 1, payload: null });
      return Boolean(handler);
    },
  };
}

/**
 * The main window can load before Rust registers its listener (Tauri builds
 * the configured window before `setup`), so the startup broadcast alone may be
 * lost: Rust then asks, and the window answers.
 */
describe("interface language handshake with Rust", () => {
  let tauri: ReturnType<typeof fakeTauri>;

  beforeEach(() => {
    tauri = fakeTauri();
    window.localStorage.setItem(INTERFACE_LOCALE_STORAGE_KEY, "pt-BR");
  });

  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    window.localStorage.removeItem(INTERFACE_LOCALE_STORAGE_KEY);
  });

  it("broadcasts again once it can hear Rust's request", async () => {
    initInterfaceLocale();
    await vi.waitFor(() => expect(tauri.broadcasts().length).toBe(2));
    expect(tauri.broadcasts().every(({ args }) => args.payload === "pt-BR")).toBe(true);
  });

  it("answers Rust's request with the current language", async () => {
    initInterfaceLocale();
    await vi.waitFor(() => expect(tauri.broadcasts().length).toBe(2));
    expect(tauri.deliver("clovy://interface-locale-request")).toBe(true);
    await vi.waitFor(() => expect(tauri.broadcasts().length).toBe(3));
    expect(tauri.broadcasts().at(-1)?.args.payload).toBe("pt-BR");
  });
});
