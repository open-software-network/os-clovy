import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { tabMeta } from "../app/app-shell";
import { createWorkspaceLoader } from "../app/workspace-lazy";
import { t } from "../i18n";
import { applyInterfaceLocale } from "../i18n/locale";

afterEach(() => {
  cleanup();
  applyInterfaceLocale("en");
});

describe("app shell copy in English (default)", () => {
  it("keeps the English tab titles", () => {
    expect(tabMeta({ view: "all-notes" }, [], []).title).toBe("All notes");
  });
});

describe("app shell copy in pt-BR", () => {
  beforeEach(() => applyInterfaceLocale("pt-BR"));

  it("translates tab titles at call time", () => {
    expect(tabMeta({ view: "all-notes" }, [], []).title).toBe("Todas as notas");
    expect(tabMeta({ view: "home" }, [], []).title).toBe("Hoje");
    expect(tabMeta({ view: "settings" }, [], []).title).toBe("Configurações");
    expect(tabMeta({ view: "agent" }, [], []).title).toBe("Hoje");
  });

  it("pluralizes app toasts and countdowns", () => {
    expect(t("app.projects.added", { count: 1 })).toBe("1 projeto adicionado");
    expect(t("app.projects.added", { count: 3 })).toBe("3 projetos adicionados");
    expect(t("app.inactivity.countdown", { count: 30 })).toBe(
      "O Clovy vai pausar esta gravação em 30 segundos se você não responder.",
    );
  });

  it("shows the workspace load failure in Portuguese", async () => {
    const consoleError = vi.spyOn(console, "error").mockImplementation(() => undefined);
    const workspace = createWorkspaceLoader(
      async (): Promise<{ Probe: () => null }> => {
        throw new Error("Chunk unavailable");
      },
      (module) => module.Probe,
    );
    render(<workspace.Component />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Não foi possível abrir esta visualização",
    );
    expect(screen.getByRole("button", { name: "Tentar de novo" })).toBeInTheDocument();
    consoleError.mockRestore();
  });
});
