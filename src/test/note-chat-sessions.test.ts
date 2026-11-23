import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AGENT_SESSION_STATUS_EVENT, type AgentSessionStatusDetail } from "../lib/agent-events";
import type {
  AgentRunDto,
  AgentRuntimeEvent,
  AgentSessionDto,
} from "../lib/agent-runtime-contract";
const AGENT_RUN_AUTO_MODEL_PREFIX = "__june_auto_generation__:";
import {
  resetCurrentDataPartitionForTests,
  setCurrentDataPartitionName,
} from "../lib/data-partition";
import { loadSessionModels, rememberSessionModel } from "../lib/agent-session-models";
import {
  forgetNoteChatSession,
  noteChatSessionIdFor,
  rememberNoteChatSession,
} from "../components/note-chat/noteChatSessions";

const mocks = vi.hoisted(() => ({
  createSession: vi.fn(),
  getSession: vi.fn(),
  getLatestRun: vi.fn(),
  listItems: vi.fn(),
  startRun: vi.fn(),
  retryRun: vi.fn(),
  cancelRun: vi.fn(),
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));

vi.mock("../lib/tauri", () => ({
  agentRuntimeBindings: {
    createSession: mocks.createSession,
    getSession: mocks.getSession,
    getLatestRun: mocks.getLatestRun,
    listItems: mocks.listItems,
    startRun: mocks.startRun,
    retryRun: mocks.retryRun,
    cancelRun: mocks.cancelRun,
  },
}));

import { useNoteChat } from "../components/note-chat/useNoteChat";

function session(overrides: Partial<AgentSessionDto> = {}): AgentSessionDto {
  return {
    id: "note-session",
    title: "Planning",
    status: "idle",
    model: "model-a",
    safetyMode: "sandboxed",
    workspacePath: "/tmp/clovy/workspace",
    source: "user",
    createdAt: "2026-07-22T00:00:00.000Z",
    updatedAt: "2026-07-22T00:00:00.000Z",
    ...overrides,
  };
}

function run(overrides: Partial<AgentRunDto> = {}): AgentRunDto {
  return {
    id: "run-1",
    sessionId: "note-session",
    status: "queued",
    model: "model-a",
    ...overrides,
  };
}

describe("note chat sessions", () => {
  beforeEach(() => {
    window.localStorage.clear();
    resetCurrentDataPartitionForTests();
    mocks.createSession.mockReset();
    mocks.getSession.mockReset();
    mocks.getLatestRun.mockReset();
    mocks.getLatestRun.mockResolvedValue(null);
    mocks.listItems.mockReset();
    mocks.startRun.mockReset();
    mocks.retryRun.mockReset();
    mocks.cancelRun.mockReset();
    mocks.cancelRun.mockResolvedValue(undefined);
    mocks.listen.mockReset();
    mocks.listen.mockResolvedValue(() => undefined);
  });

  it("dispatches a session status when it changes, not once per streamed token", async () => {
    rememberNoteChatSession("note-1", "note-session");
    mocks.getSession.mockResolvedValue(session());
    mocks.listItems.mockResolvedValue([]);
    let runtimeListener: ((event: { payload: AgentRuntimeEvent }) => void) | undefined;
    mocks.listen.mockImplementation(async (_event, handler) => {
      runtimeListener = handler;
      return () => undefined;
    });
    const statuses: string[] = [];
    const onStatus = (event: Event) =>
      statuses.push((event as CustomEvent<AgentSessionStatusDetail>).detail.status);
    window.addEventListener(AGENT_SESSION_STATUS_EVENT, onStatus);
    try {
      const { result } = renderHook(() => useNoteChat({ id: "note-1", title: "Planning" }));
      await waitFor(() => expect(result.current.loading).toBe(false));
      await waitFor(() => expect(runtimeListener).toBeDefined());

      const frame = { protocolVersion: 1, sessionId: "note-session", runId: "run-1" } as const;
      act(() => {
        runtimeListener?.({
          payload: {
            ...frame,
            sequence: 1,
            eventId: "started",
            method: "run.started",
            data: { startedAt: "now", model: "model-a" },
          },
        });
        for (let sequence = 2; sequence < 22; sequence += 1) {
          runtimeListener?.({
            payload: {
              ...frame,
              sequence,
              eventId: `delta-${sequence}`,
              method: "message.delta",
              data: {
                itemSequence: 1,
                itemId: "answer",
                role: "assistant",
                delta: "x",
                createdAt: "now",
              },
            },
          });
        }
        runtimeListener?.({
          payload: {
            ...frame,
            sequence: 22,
            eventId: "completed",
            method: "run.completed",
            data: { completedAt: "now" },
          },
        });
      });

      // One dispatch (window event plus Tauri emit) per status change.
      expect(statuses).toEqual(["running", "completed"]);
    } finally {
      window.removeEventListener(AGENT_SESSION_STATUS_EVENT, onStatus);
    }
  });

  it("remembers and forgets a note session pairing", () => {
    rememberNoteChatSession("note-a", "session-a");
    rememberNoteChatSession("note-b", "session-b");

    expect(noteChatSessionIdFor("note-a")).toBe("session-a");
    expect(noteChatSessionIdFor("note-b")).toBe("session-b");

    forgetNoteChatSession("note-a");
    expect(noteChatSessionIdFor("note-a")).toBeUndefined();
    expect(noteChatSessionIdFor("note-b")).toBe("session-b");
  });

  it("creates a sandboxed session and starts a run with the note reference", async () => {
    setCurrentDataPartitionName("private");
    mocks.createSession.mockResolvedValue(session());
    mocks.startRun.mockResolvedValue(run());
    const { result } = renderHook(() => useNoteChat({ id: "note-1", title: "Planning" }));

    let response: Awaited<ReturnType<typeof result.current.submit>> | undefined;
    await act(async () => {
      response = await result.current.submit("What did we decide?", [
        { id: "file-1", name: "agenda.pdf", path: "/tmp/agenda.pdf" },
      ]);
    });

    expect(response).toEqual({ accepted: true, current: true });
    expect(mocks.createSession).toHaveBeenCalledWith({
      title: "Planning",
      model: "__clovy_cli_engine__:claude",
      safetyMode: "sandboxed",
      profile: "private",
    });
    expect(mocks.startRun).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "note-session",
        model: "__clovy_cli_engine__:claude",
        safetyMode: "sandboxed",
        attachments: ["/tmp/agenda.pdf"],
        prompt: expect.stringContaining("@note:note-1"),
      }),
    );
    expect(noteChatSessionIdFor("note-1")).toBe("note-session");
  });

  it("hydrates the saved session transcript", async () => {
    rememberNoteChatSession("note-1", "note-session");
    mocks.getSession.mockResolvedValue(session());
    mocks.listItems.mockResolvedValue([
      {
        id: "message-1",
        sessionId: "note-session",
        sequence: 1,
        createdAt: "2026-07-22T00:00:00.000Z",
        kind: "message",
        role: "assistant",
        text: "The decision is to ship Friday.",
        status: "complete",
      },
    ]);

    const { result } = renderHook(() => useNoteChat({ id: "note-1", title: "Planning" }));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.storedSessionId).toBe("note-session");
    expect(result.current.turns).toHaveLength(1);
    expect(result.current.turns[0]?.parts[0]).toMatchObject({
      type: "text",
      text: "The decision is to ship Friday.",
    });
  });

  it("uses a companion-staged model at the next note chat run boundary", async () => {
    rememberNoteChatSession("note-1", "note-session");
    rememberSessionModel("note-session", "__clovy_cli_engine__:codex");
    mocks.getSession.mockResolvedValue(session());
    mocks.listItems.mockResolvedValue([]);
    mocks.startRun.mockResolvedValue(run({ model: "__clovy_cli_engine__:codex" }));
    const { result } = renderHook(() => useNoteChat({ id: "note-1", title: "Planning" }));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.model).toBe("__clovy_cli_engine__:codex");
    await act(async () => {
      await result.current.submit("Use the staged model.");
    });

    expect(mocks.startRun).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "note-session", model: "__clovy_cli_engine__:codex" }),
    );
    expect(loadSessionModels()["note-session"]).toBeUndefined();
  });

  it("resumes a historical native Auto session with Claude Code across later turns", async () => {
    const stagedAuto = `${AGENT_RUN_AUTO_MODEL_PREFIX}20`;
    rememberNoteChatSession("note-1", "note-session");
    rememberSessionModel("note-session", stagedAuto);
    mocks.getSession.mockResolvedValue(session());
    mocks.listItems.mockResolvedValue([]);
    mocks.startRun.mockResolvedValue(
      run({ model: "__clovy_cli_engine__:claude", status: "completed" }),
    );
    const { result } = renderHook(() => useNoteChat({ id: "note-1", title: "Planning" }));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.model).toBe("__clovy_cli_engine__:claude");
    await act(async () => {
      await result.current.submit("Use the staged Auto policy.");
    });

    expect(mocks.startRun).toHaveBeenNthCalledWith(
      1,
      expect.objectContaining({ sessionId: "note-session", model: "__clovy_cli_engine__:claude" }),
    );
    expect(loadSessionModels()["note-session"]).toBeUndefined();

    await act(async () => {
      await result.current.submit("Keep using the same Auto policy.");
    });

    expect(mocks.startRun).toHaveBeenNthCalledWith(
      2,
      expect.objectContaining({ sessionId: "note-session", model: "__clovy_cli_engine__:claude" }),
    );
  });

  it("cancels the active runtime run", async () => {
    mocks.createSession.mockResolvedValue(session());
    mocks.startRun.mockResolvedValue(run());
    const { result } = renderHook(() => useNoteChat({ id: "note-1", title: "Planning" }));

    await act(async () => {
      await result.current.submit("Summarize this note.");
    });
    act(() => result.current.stop());

    await waitFor(() => expect(mocks.cancelRun).toHaveBeenCalledWith("run-1"));
  });

  it("retries the persisted failed run for a reopened note chat", async () => {
    rememberNoteChatSession("note-1", "note-session");
    mocks.getSession.mockResolvedValue(session({ status: "failed" }));
    mocks.listItems.mockResolvedValue([]);
    mocks.getLatestRun.mockResolvedValue(run({ status: "failed" }));
    mocks.retryRun.mockResolvedValue(run({ id: "run-2", status: "running" }));
    const { result } = renderHook(() => useNoteChat({ id: "note-1", title: "Planning" }));

    await waitFor(() => expect(result.current.loading).toBe(false));
    await act(async () => {
      expect(await result.current.retry()).toBe(true);
    });

    expect(mocks.retryRun).toHaveBeenCalledWith("run-1");
    expect(result.current.working).toBe(true);
  });
});
