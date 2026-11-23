import { afterEach, describe, expect, it, vi } from "vitest";
import {
  AGENT_RUN_SETTLED_EVENT,
  AGENT_SESSION_STATUS_EVENT,
  type AgentRunSettledDetail,
  type AgentSessionStatusDetail,
} from "../lib/agent-events";
import { saveQueuedAgentFollowUps } from "../lib/agent-follow-up-queue";
import type { AgentRuntimeEvent } from "../lib/agent-runtime-contract";
import { startAgentRunSettlementRelay } from "../lib/agent-run-settlement";

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
  emit: vi.fn(async () => undefined),
}));

type Handler = (event: { payload: AgentRuntimeEvent }) => void;

function frame(
  sessionId: string,
  runId: string,
  method: AgentRuntimeEvent["method"],
  sequence: number,
): AgentRuntimeEvent {
  const base = { protocolVersion: 1, sessionId, runId, sequence, eventId: `e-${sequence}` };
  switch (method) {
    case "message.delta":
      return {
        ...base,
        method,
        data: { itemId: "i", itemSequence: 1, role: "assistant", delta: "x", createdAt: "now" },
      } as AgentRuntimeEvent;
    case "run.failed":
      return {
        ...base,
        method,
        data: { itemSequence: 2, completedAt: "now", message: "boom", retryable: false },
      } as AgentRuntimeEvent;
    case "steering.consumed":
      return {
        ...base,
        method,
        data: {
          itemId: "i",
          itemSequence: 3,
          messageId: "follow-up-1",
          text: "also do X",
          createdAt: "now",
        },
      } as AgentRuntimeEvent;
    case "run.started":
      return { ...base, method, data: { startedAt: "now", model: "m" } } as AgentRuntimeEvent;
    default:
      return { ...base, method, data: { completedAt: "now" } } as AgentRuntimeEvent;
  }
}

function startRelay(titles: Record<string, string> = {}) {
  let handler: Handler | undefined;
  const dispose = startAgentRunSettlementRelay({
    resolveTitle: (sessionId) => titles[sessionId],
    listen: async (_event, next) => {
      handler = next;
      return () => {
        handler = undefined;
      };
    },
  });
  const settled: AgentRunSettledDetail[] = [];
  const onSettled = (event: Event) =>
    settled.push((event as CustomEvent<AgentRunSettledDetail>).detail);
  const statuses: AgentSessionStatusDetail[] = [];
  const onStatus = (event: Event) =>
    statuses.push((event as CustomEvent<AgentSessionStatusDetail>).detail);
  window.addEventListener(AGENT_RUN_SETTLED_EVENT, onSettled);
  window.addEventListener(AGENT_SESSION_STATUS_EVENT, onStatus);
  cleanups.push(() => {
    dispose();
    window.removeEventListener(AGENT_RUN_SETTLED_EVENT, onSettled);
    window.removeEventListener(AGENT_SESSION_STATUS_EVENT, onStatus);
  });
  const send = (event: AgentRuntimeEvent) => handler?.({ payload: event });
  return { send, settled, statuses };
}

const cleanups: Array<() => void> = [];

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) cleanup();
  saveQueuedAgentFollowUps({});
});

function queueFollowUp(sessionId: string) {
  saveQueuedAgentFollowUps({
    [sessionId]: {
      messageId: "follow-up-1",
      prompt: "also do X",
      attachments: [],
      model: "m",
    },
  });
}

describe("agent run settlement relay", () => {
  it("tells run-settled listeners exactly once when a run completes", async () => {
    const { send, settled } = startRelay({ "session-1": "Plan the launch" });
    await Promise.resolve();

    send(frame("session-1", "run-1", "run.started", 1));
    for (let sequence = 2; sequence < 12; sequence += 1) {
      send(frame("session-1", "run-1", "message.delta", sequence));
    }
    expect(settled).toHaveLength(0);

    send(frame("session-1", "run-1", "run.completed", 12));
    // A replayed terminal frame for the same run must not announce it again.
    send(frame("session-1", "run-1", "run.completed", 13));

    expect(settled).toEqual([
      {
        sessionId: "session-1",
        title: "Plan the launch",
        summary: expect.any(String),
        activeCount: 0,
      },
    ]);
  });

  it("waits for the automatic follow-up run before announcing the session is ready", async () => {
    const { send, settled } = startRelay({ "session-1": "Plan the launch" });
    await Promise.resolve();

    // The user queued a follow-up the run never consumed: the workspace
    // submits it as the next run, so the session is not idle yet.
    queueFollowUp("session-1");
    send(frame("session-1", "run-1", "run.started", 1));
    send(frame("session-1", "run-1", "run.completed", 2));
    expect(settled).toHaveLength(0);

    saveQueuedAgentFollowUps({});
    send(frame("session-1", "run-2", "run.started", 1));
    send(frame("session-1", "run-2", "run.completed", 2));

    expect(settled).toHaveLength(1);
    expect(settled[0]).toMatchObject({ sessionId: "session-1", title: "Plan the launch" });
  });

  it("announces when the queued follow-up was consumed as live steering", async () => {
    const { send, settled } = startRelay();
    await Promise.resolve();

    queueFollowUp("session-1");
    send(frame("session-1", "run-1", "run.started", 1));
    send(frame("session-1", "run-1", "steering.consumed", 2));
    send(frame("session-1", "run-1", "run.completed", 3));

    expect(settled).toHaveLength(1);
  });

  it("does not count a late diagnostic as another active run", async () => {
    const { send, settled } = startRelay();
    await Promise.resolve();
    send({
      protocolVersion: 1,
      eventId: "late-cleanup",
      sessionId: "old-session",
      runId: "old-run",
      sequence: 51,
      method: "run.diagnostic",
      data: {
        itemId: "warning",
        itemSequence: 2,
        createdAt: "now",
        message: "Workspace MCP cleanup needs recovery.",
        category: "runtime",
        code: "agent_workspace_mcp_cleanup",
        retryable: false,
      },
    });
    send(frame("session-1", "run-1", "run.started", 1));
    send(frame("session-1", "run-1", "run.completed", 2));
    expect(settled).toHaveLength(1);
    expect(settled[0].activeCount).toBe(0);
  });

  it("counts runs that are still active when another one completes", async () => {
    const { send, settled } = startRelay();
    await Promise.resolve();

    send(frame("session-1", "run-1", "run.started", 1));
    send(frame("session-2", "run-2", "run.started", 1));
    send(frame("session-1", "run-1", "run.completed", 2));

    expect(settled).toHaveLength(1);
    expect(settled[0]).toMatchObject({ sessionId: "session-1", title: "", activeCount: 1 });
  });

  it("announces a failed run once, also for a session in the background", async () => {
    const { send, settled, statuses } = startRelay({ "session-2": "Background task" });
    await Promise.resolve();

    // session-1 is the one on screen; session-2 runs in the background.
    send(frame("session-1", "run-1", "run.started", 1));
    send(frame("session-2", "run-2", "run.started", 1));
    send(frame("session-2", "run-2", "run.failed", 2));
    send(frame("session-2", "run-2", "run.failed", 3));

    expect(settled).toHaveLength(0);
    expect(statuses).toEqual([
      {
        sessionId: "session-2",
        title: "Background task",
        status: "failed",
        summary: "boom",
        activeCount: 1,
      },
    ]);
  });

  it("stays silent for a cancelled run", async () => {
    const { send, settled, statuses } = startRelay();
    await Promise.resolve();

    send(frame("session-2", "run-2", "run.started", 1));
    send(frame("session-2", "run-2", "run.cancelled", 2));

    expect(settled).toHaveLength(0);
    expect(statuses).toHaveLength(0);
  });
});
