import { render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { AgentToolStack } from "../components/agent/chat-turns/ThinkingAndTools";
import { applyInterfaceLocale } from "../i18n/locale";
import {
  agentItemsToChatTurns,
  applyAgentRuntimeEvent,
  createAgentRuntimeProjection,
  mergeAgentRuntimeSnapshot,
} from "../lib/agent-runtime-adapter";
import type { AgentItemDto, AgentRuntimeEvent } from "../lib/agent-runtime-contract";

const createdAt = "2026-11-03T12:00:00Z";
const session = {
  id: "session-1",
  title: "Synthetic Stop",
  status: "running" as const,
  model: "auto",
  safetyMode: "sandboxed" as const,
  workspacePath: "/tmp/synthetic-stop",
  source: "user" as const,
  createdAt,
  updatedAt: createdAt,
};
const run = { id: "run-1", sessionId: session.id, status: "running" as const, model: "auto" };
const call: Extract<AgentItemDto, { kind: "tool_call" }> = {
  id: "outer-call",
  sessionId: session.id,
  runId: run.id,
  sequence: 2,
  createdAt,
  kind: "tool_call",
  callId: "item_1",
  name: "clovy.create_note",
  arguments: { title: "Synthetic Stop" },
  status: "running",
};
const approval: Extract<AgentItemDto, { kind: "interruption" }> = {
  id: "approval-receipt",
  sessionId: session.id,
  runId: run.id,
  sequence: 3,
  createdAt,
  kind: "interruption",
  interruption: {
    id: "8434465c-37fc-4eb0-844a-3bc0e4c192af",
    sessionId: session.id,
    runId: run.id,
    kind: "approval",
    engine: "cli",
    status: "cancelled",
    retiredReason: "cancelled",
    createdAt,
    toolName: "clovy.create_note",
    title: "Create note",
    description: "Synthetic Stop",
    allowAlways: false,
  },
};
type TerminalEvent = Extract<
  AgentRuntimeEvent,
  { method: "run.cancelled" | "run.completed" | "run.failed" }
>;
function terminal(method: TerminalEvent["method"]): TerminalEvent {
  return {
    protocolVersion: 1,
    sessionId: session.id,
    runId: run.id,
    eventId: `terminal:${method}`,
    sequence: 4,
    method,
    data: {
      completedAt: createdAt,
      interruptedCallIds: ["item_1"],
      ...(method === "run.failed"
        ? { itemSequence: 4, message: "CLI stopped", retryable: true }
        : {}),
    },
  } as TerminalEvent;
}
function started(sequence = 5): Extract<AgentRuntimeEvent, { method: "tool.started" }> {
  return {
    protocolVersion: 1,
    sessionId: session.id,
    runId: run.id,
    eventId: `late-start:${sequence}`,
    sequence,
    method: "tool.started",
    data: {
      itemId: call.id,
      itemSequence: call.sequence,
      callId: call.callId,
      name: call.name,
      arguments: call.arguments,
      createdAt,
    },
  };
}

describe("tool terminality after Stop", () => {
  afterEach(() => applyInterfaceLocale("en"));

  it.each([
    "run.cancelled",
    "run.completed",
    "run.failed",
  ] as const)("%s settles the outer call independently of the approval identity and hydrates identically", (method) => {
    const projection = applyAgentRuntimeEvent(
      createAgentRuntimeProjection({ session, run, items: [call, approval] }),
      terminal(method),
    );
    expect(projection.items[0]).toEqual({ ...call, status: "interrupted" });
    expect(projection.items[1]).toEqual(approval);
    expect(projection.items.filter((item) => item.kind === "tool_result")).toHaveLength(0);
    const hydrated = createAgentRuntimeProjection({
      session,
      run: projection.run,
      items: projection.items,
    });
    expect(agentItemsToChatTurns(hydrated.items)).toEqual(agentItemsToChatTurns(projection.items));
    expect(agentItemsToChatTurns(hydrated.items)[0]).toMatchObject({
      status: "complete",
      parts: expect.arrayContaining([
        {
          type: "tool",
          id: "item_1",
          name: call.name,
          text: JSON.stringify(call.arguments, null, 2),
          status: "interrupted",
        },
      ]),
    });
    expect(applyAgentRuntimeEvent(projection, terminal(method))).toBe(projection);
  });

  it("settles historical calls without touching a newer run, another session, or genuine results", () => {
    const otherRun: AgentItemDto = { ...call, id: "new-call", runId: "run-2", sequence: 5 };
    const otherSession: AgentItemDto = { ...call, id: "other-call", sessionId: "session-2" };
    const observed: AgentItemDto = {
      ...call,
      id: "observed-call",
      callId: "observed",
      sequence: 6,
    };
    const result: AgentItemDto = {
      ...call,
      id: "observed-result",
      sequence: 7,
      kind: "tool_result",
      callId: "observed",
      name: call.name,
      output: { noteId: "committed-note" },
      isError: false,
    };
    const currentRun = { ...run, id: "run-2" };
    const event = terminal("run.cancelled");
    event.data.interruptedCallIds = ["item_1", "observed"];
    const projection = applyAgentRuntimeEvent(
      createAgentRuntimeProjection({
        session,
        run: currentRun,
        items: [call, otherSession, otherRun, observed, result],
      }),
      event,
    );
    expect(projection.run).toEqual(currentRun);
    expect(projection.items.find((item) => item.id === call.id)).toMatchObject({
      status: "interrupted",
    });
    expect(projection.items.find((item) => item.id === otherRun.id)).toEqual(otherRun);
    expect(projection.items.find((item) => item.id === otherSession.id)).toEqual(otherSession);
    expect(projection.items.find((item) => item.id === observed.id)).toEqual(observed);
    expect(projection.items.find((item) => item.id === result.id)).toEqual(result);
  });

  it.each([
    false,
    true,
  ])("retains real late output (isError=%s) scoped to the original run", (isError) => {
    const otherRun: AgentItemDto = { ...call, id: "other-run-call", runId: "run-2", sequence: 5 };
    let projection = applyAgentRuntimeEvent(
      createAgentRuntimeProjection({ session, run, items: [call, otherRun] }),
      terminal("run.cancelled"),
    );
    projection = applyAgentRuntimeEvent(projection, {
      ...started(6),
      eventId: "late-result",
      method: isError ? "tool.failed" : "tool.completed",
      data: {
        itemId: "late-result",
        itemSequence: 6,
        callId: "item_1",
        name: call.name,
        createdAt,
        ...(isError ? { error: "Observed error" } : { output: { noteId: "committed-note" } }),
      },
    } as AgentRuntimeEvent);
    expect(projection.run?.status).toBe("cancelled");
    expect(projection.items.find((item) => item.id === otherRun.id)).toEqual(otherRun);
    expect(agentItemsToChatTurns(projection.items).flatMap((turn) => turn.parts)).toEqual([
      expect.objectContaining({ id: "item_1", status: "running" }),
      expect.objectContaining({ id: "item_1", status: isError ? "failed" : "complete" }),
    ]);
    expect(projection.items.find((item) => item.kind === "tool_result")).toMatchObject({
      output: isError ? "Observed error" : { noteId: "committed-note" },
      isError,
    });
    const hydratedStale = mergeAgentRuntimeSnapshot(
      projection,
      {
        session,
        run,
        items: [call, otherRun],
      },
      projection.processedEventIds,
    );
    expect(hydratedStale.run?.status).toBe("cancelled");
    expect(hydratedStale.items.find((item) => item.kind === "tool_result")).toEqual(
      projection.items.find((item) => item.kind === "tool_result"),
    );
  });

  it("rejects late starts and stale snapshots after a terminal event, including historical runs", () => {
    let projection = applyAgentRuntimeEvent(
      createAgentRuntimeProjection({ session, run, items: [call] }),
      terminal("run.cancelled"),
    );
    projection = applyAgentRuntimeEvent(projection, started());
    projection = applyAgentRuntimeEvent(projection, {
      ...started(6),
      data: { ...started().data, itemId: "late-new-call", callId: "item_2", itemSequence: 5 },
    } as AgentRuntimeEvent);
    expect(projection.items).toEqual([{ ...call, status: "interrupted" }]);
    projection = mergeAgentRuntimeSnapshot(
      projection,
      { session, run, items: [call] },
      projection.processedEventIds,
    );
    expect(projection.run?.status).toBe("cancelled");
    expect(projection.items).toEqual([{ ...call, status: "interrupted" }]);
    const newerRun = { ...run, id: "run-2" };
    projection = mergeAgentRuntimeSnapshot(
      projection,
      { session, run: newerRun, items: [call] },
      projection.processedEventIds,
    );
    projection = applyAgentRuntimeEvent(projection, started(7));
    expect(projection.run).toEqual(newerRun);
    expect(projection.items).toEqual([{ ...call, status: "interrupted" }]);
  });

  it("does not revive the cancelled run for stale CLI start or pending approval frames", () => {
    let projection = applyAgentRuntimeEvent(
      createAgentRuntimeProjection({ session, run, items: [call, approval] }),
      terminal("run.cancelled"),
    );
    projection = applyAgentRuntimeEvent(projection, {
      protocolVersion: 1,
      sessionId: session.id,
      runId: run.id,
      sequence: 5,
      eventId: "late-run-start",
      method: "run.started",
      data: { startedAt: createdAt, model: "auto" },
    });
    projection = applyAgentRuntimeEvent(projection, {
      protocolVersion: 1,
      sessionId: session.id,
      runId: run.id,
      sequence: 0,
      eventId: "stale-pending-approval",
      method: "interruption.requested",
      data: {
        itemId: approval.id,
        itemSequence: approval.sequence,
        interruption: { ...approval.interruption, status: "pending" },
      },
    });
    expect(projection.run?.status).toBe("cancelled");
    expect(projection.items.find((item) => item.id === call.id)).toMatchObject({
      status: "interrupted",
    });
    expect(projection.items.find((item) => item.id === approval.id)).toEqual(approval);
  });

  it("accepts terminal settlement after a higher-cursor result without moving the cursor backwards", () => {
    const observed: AgentItemDto = { ...call, id: "observed", callId: "observed", sequence: 1 };
    let projection = applyAgentRuntimeEvent(
      createAgentRuntimeProjection({ session, run, items: [observed, call] }),
      {
        protocolVersion: 1,
        sessionId: session.id,
        runId: run.id,
        eventId: "result-before-terminal",
        sequence: 6,
        method: "tool.completed",
        data: {
          itemId: "observed-result",
          itemSequence: 4,
          callId: "observed",
          name: call.name,
          output: "Observed output",
          createdAt,
        },
      },
    );
    projection = applyAgentRuntimeEvent(projection, terminal("run.cancelled"));
    expect(projection.run?.status).toBe("cancelled");
    expect(projection.lastSequenceByRun[run.id]).toBe(6);
    expect(projection.items.find((item) => item.id === call.id)).toMatchObject({
      status: "interrupted",
    });
    expect(projection.items.find((item) => item.id === observed.id)).toMatchObject({
      status: "complete",
    });
  });

  it("keeps a newer run when the pre-Stop snapshot returns the cancelled run as active", () => {
    let projection = applyAgentRuntimeEvent(
      createAgentRuntimeProjection({ session, run, items: [call] }),
      terminal("run.cancelled"),
    );
    projection = applyAgentRuntimeEvent(projection, {
      protocolVersion: 1,
      sessionId: session.id,
      runId: "run-2",
      eventId: "newer-run-started",
      sequence: 1,
      method: "run.started",
      data: { startedAt: createdAt, model: "auto" },
    });
    const beforeStaleSnapshot = projection;
    projection = mergeAgentRuntimeSnapshot(
      projection,
      { session, run, items: [call] },
      projection.processedEventIds,
    );
    expect(projection).toBe(beforeStaleSnapshot);
    expect(projection.run).toMatchObject({ id: "run-2", status: "running" });
    expect(projection.items).toEqual([{ ...call, status: "interrupted" }]);
  });

  it("preserves hydrated interrupted status against starts and does not leak settlement to another session", () => {
    const interrupted: AgentItemDto = { ...call, status: "interrupted" };
    const current = createAgentRuntimeProjection({ session, run, items: [interrupted] });
    expect(applyAgentRuntimeEvent(current, started()).items).toEqual([interrupted]);
    const switched = mergeAgentRuntimeSnapshot(
      current,
      {
        session: { ...session, id: "session-2" },
        run: { ...run, sessionId: "session-2" },
        items: [{ ...call, sessionId: "session-2" }],
      },
      current.processedEventIds,
    );
    expect(switched.items).toEqual([{ ...call, sessionId: "session-2" }]);
  });

  it.each([
    "en",
    "pt-BR",
  ] as const)("renders standalone and folded interruption explicitly in %s", (locale) => {
    applyInterfaceLocale(locale);
    const label = locale === "en" ? "Interrupted" : "Interrompida";
    const parts = [
      {
        type: "tool" as const,
        id: "item_1",
        name: call.name,
        text: "Synthetic arguments",
        status: "interrupted" as const,
      },
    ];
    const view = render(<AgentToolStack parts={parts} />);
    expect(view.getByText(label)).toBeInTheDocument();
    expect(view.container.querySelector('[data-status="interrupted"]')).not.toBeNull();
    expect(view.container.querySelector(".agent-tool-spinner")).toBeNull();
    expect(view.getByText("Synthetic arguments")).toBeInTheDocument();
    view.rerender(
      <AgentToolStack
        parts={[
          ...parts,
          ...["a", "b", "c"].map((id) => ({
            type: "tool" as const,
            id,
            name: id,
            text: "",
            status: "complete" as const,
          })),
        ]}
      />,
    );
    const fold = view.container.querySelector(".agent-tool-fold");
    expect(fold).toHaveAttribute("data-status", "interrupted");
    expect(fold?.querySelector("summary")?.textContent).toContain(
      locale === "en" ? "1 interrupted" : "1 interrompida",
    );
    expect(view.container.querySelector(".agent-tool-spinner")).toBeNull();
  });
});
