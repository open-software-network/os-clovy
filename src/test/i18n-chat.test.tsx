import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AgentChatTurnRow } from "../components/agent/chat-turns/AgentChatTurnRow";
import { AgentSessionBar } from "../components/agent/chat-turns/AgentSessionBar";
import { AgentToolStack } from "../components/agent/chat-turns/ThinkingAndTools";
import { reportCategoryDef } from "../components/agent/composer/reportCategory";
import { applyInterfaceLocale } from "../i18n/locale";
import type { AgentChatPart, AgentChatTurn } from "../lib/agent-chat-runtime";

const approvalTurn: AgentChatTurn = {
  id: "turn-1",
  role: "assistant",
  createdAt: "2026-08-07T21:00:04Z",
  status: "running",
  parts: [
    { type: "reasoning", text: "Checking the file first.", status: "complete" },
    {
      type: "approval",
      id: "approval-1",
      runId: "run-1",
      command: "run_shell ls",
      description: "List the folder.",
      allowPermanent: true,
      status: "pending",
    },
  ],
};

function renderTurn(turn: AgentChatTurn) {
  return render(
    <AgentChatTurnRow
      turn={turn}
      approvalSubmitting={{}}
      clarifySubmitting={{}}
      sudoSubmitting={{}}
      secretSubmitting={{}}
      thinkingOpen={() => false}
      onThinkingOpenChange={vi.fn()}
      onApproval={vi.fn()}
      onClarify={vi.fn()}
      onSudo={vi.fn()}
      onSecret={vi.fn()}
    />,
  );
}

function tool(id: string, status: "complete" | "failed"): Extract<AgentChatPart, { type: "tool" }> {
  return { type: "tool", id, name: `tool_${id}`, text: "", status };
}

describe("chat i18n", () => {
  describe("in English", () => {
    it("keeps the English copy by default", () => {
      renderTurn(approvalTurn);
      expect(screen.getByText("Approval required")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Deny" })).toBeInTheDocument();
      expect(screen.getByLabelText("Thought")).toBeInTheDocument();
    });
  });

  describe("in Portuguese", () => {
    beforeEach(() => applyInterfaceLocale("pt-BR"));
    afterEach(() => applyInterfaceLocale("en"));

    it("translates the approval card and thinking label, never the model content", () => {
      renderTurn(approvalTurn);
      expect(screen.getByText("Aprovação necessária")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Aprovar" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Negar" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Opções de aprovação" })).toBeInTheDocument();
      expect(screen.getByLabelText("Pensou")).toBeInTheDocument();
      // Model-authored content stays as written.
      expect(screen.getByText("List the folder.")).toBeInTheDocument();
      expect(screen.getByText("run_shell ls")).toBeInTheDocument();
    });

    it("pluralizes the folded tool count", () => {
      render(
        <AgentToolStack
          parts={[
            tool("a", "complete"),
            tool("b", "failed"),
            tool("c", "complete"),
            tool("d", "complete"),
          ]}
        />,
      );
      expect(screen.getByText("4 ações")).toBeInTheDocument();
      expect(screen.getByText("1 falhou")).toBeInTheDocument();
    });

    it("translates the session bar menu and keeps the session title", () => {
      render(<AgentSessionBar title="Plano de lançamento" onRename={vi.fn()} onDelete={vi.fn()} />);
      expect(screen.getByText("Plano de lançamento")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Ações da sessão" }));
      expect(screen.getByRole("menuitem", { name: "Renomear" })).toBeInTheDocument();
      expect(screen.getByRole("menuitem", { name: "Excluir sessão" })).toBeInTheDocument();
    });

    it("reads report category labels at call time", () => {
      expect(reportCategoryDef("bug")?.label).toBe("Relatório de bug");
    });
  });

  it("switches a mounted row when the language changes", () => {
    renderTurn(approvalTurn);
    expect(screen.getByText("Approval required")).toBeInTheDocument();
    act(() => applyInterfaceLocale("pt-BR"));
    try {
      expect(screen.getByText("Aprovação necessária")).toBeInTheDocument();
    } finally {
      act(() => applyInterfaceLocale("en"));
    }
  });
});
