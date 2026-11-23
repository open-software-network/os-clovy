import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AGENT_SESSION_STATUS_EVENT } from "../lib/agent-events";

type TauriListener = (event: { payload: unknown }) => unknown;

const LOCALE_KEY = "os-clovy:interface-locale";

const mocks = vi.hoisted(() => ({
  listeners: new Map<string, TauriListener>(),
  invoke: vi.fn().mockResolvedValue(undefined),
  listen: vi.fn((event: string, listener: TauriListener) => {
    mocks.listeners.set(event, listener);
    return Promise.resolve(vi.fn());
  }),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen, emit: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    hide: vi.fn().mockResolvedValue(undefined),
    startDragging: vi.fn().mockResolvedValue(undefined),
  }),
}));

// The real window markup, so the static chrome is exercised as shipped.
function loadPage(file: string) {
  const html = readFileSync(resolve(process.cwd(), file), "utf8");
  document.body.innerHTML = new DOMParser()
    .parseFromString(html, "text/html")
    .body.innerHTML.replace(/<script[\s\S]*?<\/script>/g, "");
}

async function flush() {
  for (let i = 0; i < 12; i += 1) await Promise.resolve();
}

// Modules are reset per test, so the locale module must be the instance the
// HUD itself imported.
async function switchLocale(locale: "en" | "pt-BR") {
  const { applyInterfaceLocale } = await import("../i18n/locale");
  applyInterfaceLocale(locale);
}

describe("HUD windows in pt-BR", () => {
  beforeEach(async () => {
    vi.resetModules();
    vi.clearAllMocks();
    mocks.listeners.clear();
    mocks.invoke.mockResolvedValue(undefined);
    localStorage.clear();
    const { markOnboardingComplete } = await import("../lib/onboarding");
    markOnboardingComplete();
    localStorage.setItem(LOCALE_KEY, "pt-BR");
  });

  afterEach(() => {
    window.dispatchEvent(new Event("pagehide"));
    localStorage.removeItem(LOCALE_KEY);
  });

  it("renders the meeting prompt in Portuguese and follows a language change", async () => {
    loadPage("hud.html");
    await import("../hud");
    await flush();

    expect(document.documentElement.lang).toBe("pt-BR");
    expect(document.querySelector("#hud-cancel")).toHaveAttribute("aria-label", "Cancelar ditado");
    expect(document.querySelector("#hud-stop")).toHaveAttribute("aria-label", "Parar ditado");

    await mocks.listeners.get("meeting-detection-event")?.({
      payload: JSON.stringify({ type: "meeting_detected", payload: {} }),
    });

    expect(document.querySelector("#hud-meeting-label")).toHaveTextContent("Reunião detectada");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Microfone em uso");
    expect(document.querySelector("#hud-meeting-start")).toHaveTextContent("Gravar");
    expect(document.querySelector("#hud-meeting-start .hud-meeting-start-icon svg")).toBeTruthy();
    expect(document.querySelector("#hud-meeting-dismiss")).toHaveAttribute(
      "aria-label",
      "Dispensar aviso de reunião",
    );
    expect(document.querySelector("#hud-status")).toHaveTextContent("Reunião detectada");

    await switchLocale("en");

    expect(document.documentElement.lang).toBe("en");
    expect(document.querySelector("#hud-meeting-label")).toHaveTextContent("Meeting detected");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent("Microphone in use");
    expect(document.querySelector("#hud-meeting-start")).toHaveTextContent("Record");
    expect(document.querySelector("#hud-status")).toHaveTextContent("Meeting detected");
  });

  it("keeps the selected meeting window title when the language changes", async () => {
    const meetingWindow = {
      windowId: 42,
      appName: "Chrome",
      bundleId: "com.google.Chrome",
      title: "Meet - Product review",
    };
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "detect_meeting_window" ? meetingWindow : undefined,
    );
    loadPage("hud.html");
    await import("../hud");
    await flush();
    await mocks.listeners.get("meeting-detection-event")?.({
      payload: JSON.stringify({ type: "meeting_detected", payload: { appLabels: ["Chrome"] } }),
    });
    document.querySelector<HTMLButtonElement>("#hud-meeting-video")?.click();
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent(
      "Chrome: Meet - Product review",
    );

    await switchLocale("en");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent(
      "Chrome: Meet - Product review",
    );
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute(
      "aria-label",
      "Record meeting video",
    );
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute("aria-checked", "true");
    await switchLocale("pt-BR");
    expect(document.querySelector("#hud-meeting-app")).toHaveTextContent(
      "Chrome: Meet - Product review",
    );
    expect(document.querySelector("#hud-meeting-video")).toHaveAttribute(
      "aria-label",
      "Gravar vídeo da reunião",
    );
  });

  it("translates dictation status lines, including interpolation", async () => {
    loadPage("hud.html");
    await import("../hud");
    await flush();

    await mocks.listeners.get("dictation-event")?.({
      payload: JSON.stringify({ type: "paste_target", payload: { app: "Notes" } }),
    });
    expect(document.querySelector("#hud-status")).toHaveTextContent("Colando em Notes");

    await switchLocale("en");
    expect(document.querySelector("#hud-status")).toHaveTextContent("Pasting into Notes");
  });

  it("words dictation errors by code, keeping the native message as fallback", async () => {
    loadPage("hud.html");
    await import("../hud");
    await flush();

    await mocks.listeners.get("dictation-event")?.({
      payload: JSON.stringify({
        type: "error",
        payload: {
          code: "accessibility_permission_missing",
          message:
            "Clovy couldn't paste automatically. Your transcript is on the clipboard, so you can paste it with Cmd+V.",
        },
      }),
    });
    await flush();
    expect(document.querySelector("#hud-error-text")).toHaveTextContent(
      "O Clovy não conseguiu colar automaticamente. Sua transcrição está na área de transferência, então você pode colar com Cmd+V.",
    );

    await mocks.listeners.get("dictation-event")?.({
      payload: JSON.stringify({
        type: "error",
        payload: { code: "provider_said_no", message: "Upstream said no." },
      }),
    });
    await flush();
    expect(document.querySelector("#hud-error-text")).toHaveTextContent("Upstream said no.");
  });

  it.each([
    "en",
    "pt-BR",
  ] as const)("shows the missing Groq key setup path in %s", async (locale) => {
    loadPage("hud.html");
    await import("../hud");
    await flush();
    await switchLocale(locale);
    await mocks.listeners.get("dictation-event")?.({
      payload: {
        type: "error",
        payload: { code: "groq_api_key_missing", message: "native fallback" },
      },
    });
    await flush();
    expect(document.querySelector("#hud-error-text")).toHaveTextContent(
      locale === "en"
        ? "Add your Groq API key in Settings > Models > More options > Provider API keys to transcribe recordings."
        : "Adicione sua chave de API da Groq em Configurações > Modelos > Mais opções > Chaves de API de provedores para transcrever gravações.",
    );
    expect(document.querySelector("#hud-error-text")).not.toHaveTextContent("native fallback");
  });

  it.each([
    "en",
    "pt-BR",
  ] as const)("names the missing OpenRouter key while pasting raw dictation in %s", async (locale) => {
    loadPage("hud.html");
    await import("../hud");
    await flush();
    await switchLocale(locale);
    await mocks.listeners.get("dictation-event")?.({
      payload: {
        type: "dictation_cleanup_skipped",
        payload: { provider: "openrouter", code: "openrouter_api_key_missing" },
      },
    });
    await mocks.listeners.get("dictation-event")?.({
      payload: { type: "paste_target", payload: { app: "Notes" } },
    });
    expect(document.querySelector("#hud-status")).toHaveTextContent(
      locale === "en"
        ? "Pasting into Notes without cleanup: OpenRouter API key missing."
        : "Colando em Notes sem limpeza: falta a chave de API da OpenRouter.",
    );
    expect(document.querySelector("#hud")).toHaveAttribute("data-state", "pasting");
  });

  it("translates the persistent destination name and unknown fallback", async () => {
    loadPage("hud.html");
    await import("../hud");
    await flush();
    await mocks.listeners.get("dictation-event")?.({
      payload: {
        type: "listening_started",
        payload: {
          takeId: "locale-1",
          target: {
            app: "Finder",
            bundleIdentifier: "com.apple.finder",
            processIdentifier: 1,
          },
        },
      },
    });
    expect(document.querySelector("#hud-target")).toHaveAttribute(
      "title",
      "Destino do ditado: Finder",
    );
    await switchLocale("en");
    expect(document.querySelector("#hud-target")).toHaveAttribute(
      "aria-label",
      "Dictation destination: Finder",
    );
    await mocks.listeners.get("dictation-event")?.({
      payload: { type: "listening_started", payload: { takeId: "locale-2" } },
    });
    await switchLocale("pt-BR");
    expect(document.querySelector("#hud-target")).toHaveAttribute(
      "aria-label",
      "Destino do ditado não identificado",
    );
  });

  it("hydrates a processing destination from the native snapshot", async () => {
    mocks.invoke.mockImplementation((command: string) =>
      Promise.resolve(
        command === "latest_dictation_event"
          ? JSON.stringify({
              type: "finalizing_transcript",
              payload: {
                takeId: "replay",
                target: {
                  app: "Terminal",
                  bundleIdentifier: "com.apple.Terminal",
                  processIdentifier: 2,
                },
              },
            })
          : undefined,
      ),
    );
    loadPage("hud.html");
    await import("../hud");
    await flush();
    expect(document.querySelector("#hud")).toHaveAttribute("data-state", "transcribing");
    expect(document.querySelector("#hud-target")).toHaveAttribute(
      "title",
      "Destino do ditado: Terminal",
    );
    await mocks.listeners.get("dictation-event")?.({
      payload: { type: "focus_target", payload: { takeId: "replay", target: { app: "Other" } } },
    });
    expect(document.querySelector("#hud-target")).toHaveAttribute(
      "title",
      "Destino do ditado: Terminal",
    );
  });

  it("does not replay an older snapshot over a live take", async () => {
    let resolveLatest: (value: string) => void = () => {};
    mocks.invoke.mockImplementation((command: string) =>
      command === "latest_dictation_event"
        ? new Promise<string>((resolve) => {
            resolveLatest = resolve;
          })
        : Promise.resolve(),
    );
    loadPage("hud.html");
    await import("../hud");
    await flush();
    await mocks.listeners.get("dictation-event")?.({
      payload: {
        type: "listening_started",
        payload: {
          takeId: "new",
          target: { app: "Terminal", bundleIdentifier: "com.apple.Terminal", processIdentifier: 2 },
        },
      },
    });
    resolveLatest(
      JSON.stringify({
        type: "listening_started",
        payload: {
          takeId: "old",
          target: { app: "Finder", bundleIdentifier: "com.apple.finder", processIdentifier: 1 },
        },
      }),
    );
    await flush();
    expect(document.querySelector("#hud-target")).toHaveAttribute(
      "title",
      "Destino do ditado: Terminal",
    );
  });

  it("does not revive a cancelled target when finalizing arrives after the HUD has hidden", async () => {
    loadPage("hud.html");
    await import("../hud");
    await flush();
    const emit = (type: string) =>
      mocks.listeners.get("dictation-event")?.({
        payload: {
          type,
          payload: {
            takeId: "cancelled",
            target: {
              app: "Terminal",
              bundleIdentifier: "com.apple.Terminal",
              processIdentifier: 2,
            },
          },
        },
      });
    await emit("listening_started");
    await emit("recording_discarded");
    await vi.waitFor(() =>
      expect(document.querySelector("#hud")).toHaveAttribute("data-state", "idle"),
    );
    await emit("finalizing_transcript");
    expect(document.querySelector("#hud")).toHaveAttribute("data-state", "idle");
    expect(document.querySelector("#hud-target")).toHaveAttribute("hidden");
  });

  it("still hydrates the active target after an unrelated helper event", async () => {
    let resolveLatest: (value: string) => void = () => {};
    mocks.invoke.mockImplementation((command: string) =>
      command === "latest_dictation_event"
        ? new Promise<string>((resolve) => {
            resolveLatest = resolve;
          })
        : Promise.resolve(),
    );
    loadPage("hud.html");
    await import("../hud");
    await flush();
    await mocks.listeners.get("dictation-event")?.({
      payload: { type: "microphone_selected", payload: { name: "Microphone" } },
    });
    resolveLatest(
      JSON.stringify({
        type: "listening_started",
        payload: {
          takeId: "active",
          target: { app: "Terminal", bundleIdentifier: "com.apple.Terminal", processIdentifier: 2 },
        },
      }),
    );
    await flush();
    expect(document.querySelector("#hud")).toHaveAttribute("data-state", "listening");
    expect(document.querySelector("#hud-target")).toHaveAttribute(
      "title",
      "Destino do ditado: Terminal",
    );
  });

  it("pluralizes the agent HUD pill and re-renders on a language change", async () => {
    loadPage("agent-hud.html");
    await import("../agent-hud");
    await flush();

    for (const title of ["First task", "Second task"]) {
      window.dispatchEvent(
        new CustomEvent(AGENT_SESSION_STATUS_EVENT, {
          detail: { status: "waitingForUser", title, prompt: title },
        }),
      );
    }
    await flush();

    const pill = document.querySelector("#agent-hud-pill");
    expect(document.querySelector("#agent-hud-pill-label")).toHaveTextContent(
      "2 precisam de resposta",
    );
    expect(pill).toHaveAttribute("aria-label", "Recolher atividade dos agentes");
    expect(document.querySelector(".agent-hud-surface")).toHaveAttribute(
      "aria-label",
      "Atividade dos agentes",
    );
    expect(document.querySelector("#agent-hud-hide")).toHaveTextContent("Ocultar HUD de sessões");

    await switchLocale("en");

    expect(document.querySelector("#agent-hud-pill-label")).toHaveTextContent("2 need input");
    expect(pill).toHaveAttribute("aria-label", "Collapse agent activity");
    expect(document.querySelector("#agent-hud-hide")).toHaveTextContent("Hide sessions HUD");
  });

  it("labels the recording pill and meeting-end prompt in Portuguese", async () => {
    loadPage("meeting-hud.html");
    await import("../meeting-hud");
    await flush();

    expect(document.querySelector("#mhud")).toHaveAttribute(
      "aria-label",
      "Gravando. Clique para abrir o Clovy",
    );
    const stop = document.querySelector("#mhud-end-stop");
    expect(stop).toHaveTextContent("Parar agora");
    expect(stop).toHaveAttribute("aria-label", "Parar a gravação agora");
    expect(document.querySelector("#mhud-end-seconds")).toBeTruthy();
    expect(document.querySelector("#mhud-end-keep")).toHaveTextContent("Continuar gravando");

    await switchLocale("en");
    expect(document.querySelector("#mhud")).toHaveAttribute(
      "aria-label",
      "Recording. Click to open Clovy",
    );
    expect(stop).toHaveTextContent("Stop now");
  });
});
