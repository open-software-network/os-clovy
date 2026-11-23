import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Sidebar } from "../components/sidebar/Sidebar";
import { TabBar } from "../components/tabs/TabBar";
import { PermissionBanner } from "../components/permissions/PermissionBanner";
import { applyInterfaceLocale } from "../i18n/locale";
import { t } from "../i18n";

vi.mock("../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/tauri")>()),
  deleteAgentSession: vi.fn(async () => undefined),
  listAgentSessions: vi.fn(async () => []),
  listSessionPartitions: vi.fn(async () => []),
}));

function renderSidebar(activeView: "notes" | "settings" = "notes") {
  return render(
    <Sidebar notes={[]} activeView={activeView} onChangeView={vi.fn()} onSelectNote={vi.fn()} />,
  );
}

const tabs = [
  { id: "a", title: "Alpha", icon: null },
  { id: "b", title: "Beta", icon: null },
];

function renderTabBar() {
  return render(
    <TabBar
      tabs={tabs}
      activeTabId="a"
      onActivate={vi.fn()}
      onClose={vi.fn()}
      onCloseOthers={vi.fn()}
      onNew={vi.fn()}
      onReorder={vi.fn()}
    />,
  );
}

describe("shell i18n (English default)", () => {
  it("renders the sidebar and tab bar in English", () => {
    renderSidebar();
    expect(screen.getByRole("button", { name: "Meeting notes" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Today" })).toBeInTheDocument();
    renderTabBar();
    expect(screen.getByRole("button", { name: "New tab" })).toBeInTheDocument();
    expect(screen.getByRole("tablist", { name: "Open tabs" })).toBeInTheDocument();
  });

  it("keeps plural and interpolated English copy", () => {
    expect(t("shell.time.minutes", { count: 5 })).toBe("5m");
  });
});

describe("shell i18n (pt-BR)", () => {
  beforeEach(() => applyInterfaceLocale("pt-BR"));
  afterEach(() => applyInterfaceLocale("en"));

  it("renders the sidebar navigation in Portuguese", () => {
    renderSidebar();
    expect(screen.getByRole("button", { name: "Notas de reunião" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hoje" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Principal" })).toBeInTheDocument();
    expect(screen.getByRole("searchbox", { name: "Buscar" })).toBeInTheDocument();
  });

  it("renders the settings nav in Portuguese", () => {
    renderSidebar("settings");
    expect(screen.getByRole("button", { name: "Voltar ao app" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Aparência" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Configurações: Pessoal" })).toBeInTheDocument();
  });

  it("renders the tab bar in Portuguese with interpolated labels", () => {
    const { container } = renderTabBar();
    expect(screen.getByRole("tablist", { name: "Abas abertas" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Nova aba" })).toBeInTheDocument();
    expect(container.querySelector(".tab-close")).toHaveAttribute("aria-label", "Fechar Alpha");
  });

  it("formats plurals and compact times in Portuguese", () => {
    expect(t("shell.time.hours", { count: 3 })).toBe("3 h");
  });

  it("switches language live after render", () => {
    act(() => applyInterfaceLocale("en"));
    render(<PermissionBanner onDismiss={vi.fn()} onEnableAccessibility={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Grant access" })).toBeInTheDocument();
    act(() => applyInterfaceLocale("pt-BR"));
    expect(screen.getByRole("button", { name: "Conceder acesso" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Dispensar lembrete de acessibilidade" }),
    ).toBeInTheDocument();
  });
});
