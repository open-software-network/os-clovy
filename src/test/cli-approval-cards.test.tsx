import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApprovalPart, ClarifyPart } from "../components/agent/chat-turns/AgentActionCards";
import { setInterfaceLocale } from "../i18n";
import type { AgentChatPart } from "../lib/agent-chat-runtime";
import {
  agentItemsToChatTurns,
  applyAgentRuntimeEvent,
  createAgentRuntimeProjection,
  reconcileAgentInterruptionResolution,
} from "../lib/agent-runtime-adapter";
import {
  AGENT_RUNTIME_PROTOCOL_VERSION,
  type AgentInterruptionDto,
  type AgentItemDto,
  type AgentRunDto,
} from "../lib/agent-runtime-contract";

vi.mock("../lib/tauri", () => ({}));

const createdAt = "2026-10-15T12:00:00Z";

/** The item the approval broker records (`src-tauri/src/approval_broker`). */
function cliApproval(overrides: Partial<AgentInterruptionDto> = {}): AgentItemDto {
  return {
    id: "interruption:run-1:req-1",
    sessionId: "session-1",
    runId: "run-1",
    sequence: 4,
    createdAt,
    kind: "interruption",
    interruption: {
      id: "req-1",
      sessionId: "session-1",
      runId: "run-1",
      status: "pending",
      createdAt,
      kind: "approval",
      engine: "cli",
      toolName: "create_routine",
      title: "Create a routine",
      description: 'Clovy wants to create the routine "Daily digest".',
      command: 'create_routine {"name": "Daily digest"}',
      allowAlways: false,
      ...overrides,
    } as AgentInterruptionDto,
  };
}

function cliQuestion(overrides: Partial<AgentInterruptionDto> = {}): AgentItemDto {
  return {
    id: "interruption:run-1:req-2",
    sessionId: "session-1",
    runId: "run-1",
    sequence: 5,
    createdAt,
    kind: "interruption",
    interruption: {
      id: "req-2",
      sessionId: "session-1",
      runId: "run-1",
      status: "pending",
      createdAt,
      kind: "clarification",
      engine: "cli",
      question: "Which calendar?",
      choices: ["Work", "Personal"],
      ...overrides,
    } as AgentInterruptionDto,
  };
}

function partOf<T extends AgentChatPart["type"]>(item: AgentItemDto, type: T) {
  const part = agentItemsToChatTurns([item])
    .flatMap((turn) => turn.parts)
    .find((candidate) => candidate.type === type);
  if (!part) throw new Error(`no ${type} part`);
  return part as Extract<AgentChatPart, { type: T }>;
}

function renderApproval(item: AgentItemDto, onApproval = vi.fn()) {
  return render(<ApprovalPart part={partOf(item, "approval")} onApproval={onApproval} />);
}

function renderClarify(item: AgentItemDto, onClarify = vi.fn()) {
  return render(<ClarifyPart part={partOf(item, "clarify")} onClarify={onClarify} />);
}

afterEach(() => {
  cleanup();
  setInterfaceLocale("en");
});

describe("CLI approval cards", () => {
  it("renders a pending card from the broker item with the tool and preview", () => {
    const onApproval = vi.fn();
    const { container } = renderApproval(cliApproval(), onApproval);

    expect(screen.getByText("Approval required")).toBeTruthy();
    expect(screen.getByText('Clovy wants to create the routine "Daily digest".')).toBeTruthy();
    expect(container.querySelector("pre")?.textContent).toBe(
      'create_routine {"name": "Daily digest"}',
    );
    // One approval per call.
    expect(screen.queryByText("Explain first")).toBeNull();
    expect(screen.queryByLabelText("Approve options")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Approve" }));
    expect(onApproval).toHaveBeenCalledWith(
      expect.objectContaining({ id: "req-1", runId: "run-1", engine: "cli" }),
      "once",
    );
    fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    expect(onApproval).toHaveBeenLastCalledWith(expect.anything(), "deny");
  });

  it("keeps the native scope menu without the retired model explanation", () => {
    const item = cliApproval();
    if (item.kind === "interruption") {
      const { engine: _engine, ...native } = item.interruption;
      item.interruption = native as AgentInterruptionDto;
    }
    renderApproval(item);
    // `explain_agent_approval` left the command surface with the native
    // runtime, so the card must not offer a button that can only fail.
    expect(screen.queryByText("Explain first")).toBeNull();
    expect(screen.getByLabelText("Approve options")).toBeTruthy();
  });

  it("renders approved, denied, expired, and cancelled outcomes", () => {
    const cases: Array<[Partial<AgentInterruptionDto>, string, string]> = [
      [{ status: "resolved", resolution: "once" }, "Approved once", "done"],
      [{ status: "resolved", resolution: "deny" }, "Denied", "deny"],
      [{ status: "expired", retiredReason: "timeout" }, "Approval expired", "deny"],
      [{ status: "cancelled", retiredReason: "cancelled" }, "Approval cancelled", "deny"],
    ];
    for (const [overrides, label, choice] of cases) {
      const { container } = renderApproval(cliApproval(overrides));
      const row = container.querySelector(".agent-resolved-row");
      expect(row?.getAttribute("data-choice"), label).toBe(choice);
      expect(container.querySelector(".agent-resolved-label")?.textContent).toBe(label);
      expect(screen.queryByRole("button", { name: "Approve" })).toBeNull();
      cleanup();
    }
  });

  it("says why a CLI card ended without an answer", () => {
    const expired = renderApproval(cliApproval({ status: "expired", retiredReason: "timeout" }));
    expect(expired.container.querySelector(".agent-resolved-body")?.textContent).toContain(
      "Nobody answered in time. Clovy did not run this action.",
    );
    cleanup();
    const cancelled = renderApproval(cliApproval({ status: "cancelled" }));
    expect(cancelled.container.querySelector(".agent-resolved-body")?.textContent).toContain(
      "The run stopped before anyone answered. Clovy did not run this action.",
    );
  });

  it("renders in Brazilian Portuguese", () => {
    setInterfaceLocale("pt-BR");
    const pending = renderApproval(cliApproval());
    expect(screen.getByText("Aprovação necessária")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Aprovar" })).toBeTruthy();
    expect(screen.queryByText("Explicar antes")).toBeNull();
    pending.unmount();
    const { container } = renderApproval(cliApproval({ status: "cancelled" }));
    expect(container.querySelector(".agent-resolved-label")?.textContent).toBe(
      "Aprovação cancelada",
    );
    cleanup();
    const question = renderClarify(cliQuestion({ status: "expired" }));
    expect(question.container.querySelector(".agent-resolved-label")?.textContent).toBe(
      "Pergunta expirada",
    );
  });
});

describe("CLI clarification cards", () => {
  it("answers a pending question with the chosen option", () => {
    const onClarify = vi.fn();
    renderClarify(cliQuestion(), onClarify);
    expect(screen.getByText("Which calendar?")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Personal/ }));
    expect(onClarify).toHaveBeenCalledWith(
      expect.objectContaining({ id: "req-2", engine: "cli" }),
      "Personal",
    );
  });

  it("renders answered, expired, and cancelled questions", () => {
    const cases: Array<[Partial<AgentInterruptionDto>, string, string]> = [
      [{ status: "resolved", answer: "Personal" }, "Answered", "done"],
      [{ status: "expired", retiredReason: "timeout" }, "Question expired", "deny"],
      [{ status: "cancelled", retiredReason: "cancelled" }, "Question cancelled", "deny"],
    ];
    for (const [overrides, label, choice] of cases) {
      const { container } = renderClarify(cliQuestion(overrides));
      expect(container.querySelector(".agent-resolved-row")?.getAttribute("data-choice")).toBe(
        choice,
      );
      expect(container.querySelector(".agent-resolved-label")?.textContent).toBe(label);
      cleanup();
    }
  });
});

describe("CLI interruption events", () => {
  const run: AgentRunDto = {
    id: "run-1",
    sessionId: "session-1",
    status: "running",
    model: "__clovy_cli_engine__:claude",
    startedAt: createdAt,
  };

  function event(
    method: "interruption.requested" | "interruption.updated",
    item: AgentItemDto,
    sequence: number,
    eventId: string,
  ) {
    if (item.kind !== "interruption") throw new Error("interruption expected");
    return {
      protocolVersion: AGENT_RUNTIME_PROTOCOL_VERSION,
      sessionId: "session-1",
      runId: "run-1",
      sequence,
      eventId,
      method,
      data: { itemId: item.id, itemSequence: item.sequence, interruption: item.interruption },
    } as const;
  }

  it("applies broker events outside the turn's sequence gate", () => {
    let projection = createAgentRuntimeProjection({ run });
    projection = applyAgentRuntimeEvent(projection, {
      protocolVersion: AGENT_RUNTIME_PROTOCOL_VERSION,
      sessionId: "session-1",
      runId: "run-1",
      sequence: 7,
      eventId: "tool-started",
      method: "tool.started",
      data: {
        itemSequence: 7,
        itemId: "tool-call:1",
        callId: "call-1",
        name: "create_routine",
        arguments: {},
        createdAt,
      },
    });
    // The broker reports the run's last persisted sequence, which can be
    // behind what the webview already saw.
    projection = applyAgentRuntimeEvent(
      projection,
      event("interruption.requested", cliApproval(), 6, "broker-1"),
    );
    expect(projection.run?.status).toBe("waiting_for_user");
    expect(projection.lastSequenceByRun["run-1"]).toBe(7);
    const card = projection.items.find((item) => item.kind === "interruption");
    expect(card?.kind === "interruption" && card.interruption.status).toBe("pending");

    projection = applyAgentRuntimeEvent(
      projection,
      event(
        "interruption.updated",
        cliApproval({ status: "resolved", resolution: "once" }),
        6,
        "broker-2",
      ),
    );
    const cards = projection.items.filter((item) => item.kind === "interruption");
    expect(cards).toHaveLength(1);
    expect(cards[0]?.kind === "interruption" && cards[0].interruption.status).toBe("resolved");
    expect(projection.run?.status).toBe("running");

    // The turn's next event still passes.
    projection = applyAgentRuntimeEvent(projection, {
      protocolVersion: AGENT_RUNTIME_PROTOCOL_VERSION,
      sessionId: "session-1",
      runId: "run-1",
      sequence: 8,
      eventId: "tool-completed",
      method: "tool.completed",
      data: {
        itemSequence: 8,
        itemId: "tool-result:1",
        callId: "call-1",
        name: "create_routine",
        output: "ok",
        createdAt,
      },
    });
    expect(projection.items.some((item) => item.kind === "tool_result")).toBe(true);
  });

  it("marks an expired or cancelled card without another request", () => {
    let projection = createAgentRuntimeProjection({ run, items: [cliApproval()] });
    projection = { ...projection, run: { ...run, status: "waiting_for_user" } };
    projection = applyAgentRuntimeEvent(
      projection,
      event("interruption.updated", cliApproval({ status: "expired" }), 0, "broker-3"),
    );
    expect(partOf(projection.items[0] as AgentItemDto, "approval").status).toBe("expired");
    expect(projection.run?.status).toBe("running");
  });

  it("records a session or always answer to a CLI card as once", () => {
    for (const choice of ["session", "always"] as const) {
      const [item] = reconcileAgentInterruptionResolution([cliApproval()], {
        runId: "run-1",
        interruptionId: "req-1",
        resolution: { kind: "approval", choice },
      });
      expect(item?.kind === "interruption" && item.interruption.status).toBe("resolved");
      expect(
        item?.kind === "interruption" &&
          item.interruption.kind === "approval" &&
          item.interruption.resolution,
      ).toBe("once");
    }
  });
});
