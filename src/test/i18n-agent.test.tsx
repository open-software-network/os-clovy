import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  computerUseApprovalsPending: vi.fn(),
  computerUseCaptureSrc: vi.fn((path: string) => `asset://${path}`),
  computerUseStop: vi.fn(),
  respondComputerUseApproval: vi.fn(),
  deleteAgentSession: vi.fn(),
}));

vi.mock("../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/tauri")>()),
  ...tauriMocks,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => {}),
}));

import { AgentSessionsList } from "../components/agent/AgentSessionsList";
import { ComputerUseApprovalsTray } from "../components/agent/ComputerUseApprovalsTray";
import { HERO_GREETINGS, heroGreeting } from "../components/agent/agent-workspace-config";
import { applyInterfaceLocale } from "../i18n/locale";
import type { AgentSessionDto } from "../lib/agent-runtime-contract";

function session(id: string, title: string): AgentSessionDto {
  return {
    id,
    title,
    model: "auto",
    safetyMode: "sandboxed",
    createdAt: "2026-09-20T10:00:00.000Z",
    updatedAt: "2026-09-20T10:00:00.000Z",
  } as AgentSessionDto;
}

function renderSessions(sessions: AgentSessionDto[]) {
  return render(
    <AgentSessionsList
      sessions={sessions}
      folders={[]}
      sessionFolderIds={{}}
      onSelectSession={() => undefined}
      onNewSession={() => undefined}
      onRenameSession={() => undefined}
      onOpenMoveDialog={() => undefined}
      onOpenMoveSessions={() => undefined}
      onRemoveFromProject={() => undefined}
    />,
  );
}

beforeEach(() => {
  tauriMocks.computerUseApprovalsPending.mockResolvedValue([]);
});

afterEach(() => {
  cleanup();
  act(() => applyInterfaceLocale("en"));
  vi.clearAllMocks();
});

describe("agent i18n", () => {
  it("renders the sessions list in English by default", () => {
    renderSessions([]);
    expect(screen.getByRole("heading", { name: "Sessions" })).toBeInTheDocument();
    expect(screen.getByText("Put Clovy to work")).toBeInTheDocument();
  });

  it("renders the sessions list and bulk selection in Portuguese", async () => {
    act(() => applyInterfaceLocale("pt-BR"));
    const user = userEvent.setup();
    renderSessions([session("s1", "Plano"), session("s2", "")]);

    expect(screen.getByRole("heading", { name: /Sessões/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Nova sessão/ })).toBeInTheDocument();
    expect(screen.getByText("Sessão sem título")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Ações para Plano" })).toBeInTheDocument();

    await user.click(screen.getByRole("checkbox", { name: "Selecionar Plano" }));
    expect(screen.getByText("1 selecionada")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Selecionar tudo" }));
    expect(screen.getByText("2 selecionadas")).toBeInTheDocument();
  });

  it("switches the empty state live when the language changes", () => {
    renderSessions([]);
    expect(screen.getByText("Put Clovy to work")).toBeInTheDocument();
    act(() => applyInterfaceLocale("pt-BR"));
    expect(screen.getByText("Coloque o Clovy para trabalhar")).toBeInTheDocument();
  });

  it("translates the Computer use approval tray with interpolation", async () => {
    act(() => applyInterfaceLocale("pt-BR"));
    tauriMocks.computerUseApprovalsPending.mockResolvedValue([
      {
        approvalId: "approval-1",
        actionId: "action-1",
        action: "use_app",
        targetApp: "TextEdit",
        summary: "Summary",
        capturePath: null,
        requestedAtMs: 1,
        expiresAtMs: Date.now() + 60_000,
      },
    ]);
    render(<ComputerUseApprovalsTray />);
    await waitFor(() =>
      expect(screen.getByLabelText("Aprovações de uso do computador")).toBeInTheDocument(),
    );
    expect(screen.getByText("O Clovy quer usar o TextEdit")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Permitir nesta tarefa" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Negar" })).toBeInTheDocument();
    expect(screen.getByText(/Expira às/)).toBeInTheDocument();
  });

  it("keeps the English greeting pool exported and translates at render", () => {
    expect(heroGreeting(0)).toBe(HERO_GREETINGS[0]);
    act(() => applyInterfaceLocale("pt-BR"));
    expect(heroGreeting(0)).toBe("O que o Clovy pode fazer por você?");
  });
});
