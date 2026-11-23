import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentRuntimeEvent } from "../lib/agent-runtime-contract";
import { AGENT_SESSION_STATUS_EVENT } from "../lib/agent-events";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  createSession: vi.fn(),
  getSession: vi.fn(),
  startRun: vi.fn(),
  listItems: vi.fn(),
  getLatestRun: vi.fn(),
  runtimeListener: undefined as ((event: { payload: AgentRuntimeEvent }) => void) | undefined,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (_name: string, listener: (event: { payload: AgentRuntimeEvent }) => void) => {
      mocks.runtimeListener = listener;
      return vi.fn();
    },
  ),
}));

vi.mock("../lib/tauri", () => ({
  invoke: mocks.invoke,
  agentRuntimeBindings: {
    createSession: mocks.createSession,
    startRun: mocks.startRun,
    getSession: mocks.getSession,
    listItems: mocks.listItems,
    getLatestRun: mocks.getLatestRun,
    cancelRun: vi.fn(),
    retryRun: vi.fn(),
  },
}));

import { useNoteChat } from "../components/note-chat/useNoteChat";
import { chatModelId } from "../lib/chat-engine";

import { rememberNoteChatSession } from "../components/note-chat/noteChatSessions";
import { loadSessionModels, rememberSessionModel } from "../lib/agent-session-models";

const note = { id: "note-7", title: "Kickoff" };

describe("note chat engine", () => {
  it.each([
    "diagnostic",
    "clean",
    "switch",
  ] as const)("F10b remembered note hydration preserves only its own real diagnostic: %s", async (scenario) => {
    const message = "Initial note cleanup needs recovery.";
    const stored = {
      id: "note-session",
      title: "Kickoff",
      status: "idle",
      model: "__clovy_cli_engine__:agy",
      safetyMode: "sandboxed",
      workspacePath: "/tmp/note-session",
      source: "user",
      createdAt: "now",
      updatedAt: "now",
    };
    const cancelled = {
      id: "initial-cancelled-run",
      sessionId: stored.id,
      status: "cancelled",
      model: stored.model,
    };
    rememberNoteChatSession(note.id, stored.id);
    let release: (value: typeof stored) => void = () => undefined;
    mocks.getSession.mockImplementation(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    mocks.getLatestRun.mockResolvedValue(cancelled);
    const view = renderHook(({ selected }) => useNoteChat(selected), {
      initialProps: { selected: note },
    });
    await waitFor(() => expect(mocks.runtimeListener).toBeDefined());
    await waitFor(() => expect(mocks.listItems).toHaveBeenCalledWith(stored.id));
    await act(async () => Promise.resolve());
    expect(view.result.current.loading).toBe(true);
    if (scenario !== "clean") {
      act(() =>
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "initial-note-warning",
            sessionId: stored.id,
            runId: cancelled.id,
            sequence: 51,
            method: "run.diagnostic",
            data: {
              itemId: "initial-warning",
              itemSequence: 2,
              createdAt: "now",
              message,
              category: "runtime",
              code: "agent_workspace_mcp_cleanup",
              retryable: false,
            },
          },
        }),
      );
    }
    if (scenario === "switch")
      view.rerender({ selected: { id: "different-note", title: "Different" } });
    await act(async () => release(stored));
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    if (scenario === "diagnostic") {
      expect(JSON.stringify(view.result.current.turns)).toContain(message);
      expect(view.result.current.turns).toHaveLength(1);
    } else {
      expect(view.result.current.turns).toEqual([]);
    }
    expect(view.result.current.working).toBe(false);
    expect(mocks.startRun).not.toHaveBeenCalled();
  });

  it("F10 preserves a late diagnostic over an older cancelled note-chat hydration", async () => {
    const message = "Late note workspace cleanup needs recovery.";
    const stored = {
      id: "note-session",
      title: "Kickoff",
      status: "idle",
      model: "__clovy_cli_engine__:agy",
      safetyMode: "sandboxed",
      workspacePath: "/tmp/note-session",
      source: "user",
      createdAt: "now",
      updatedAt: "now",
    };
    const cancelled = {
      id: "cancelled-run",
      sessionId: stored.id,
      status: "cancelled",
      model: stored.model,
    };
    rememberNoteChatSession(note.id, stored.id);
    mocks.getSession.mockResolvedValue(stored);
    mocks.getLatestRun.mockResolvedValue(cancelled);
    const { result, unmount } = renderHook(() => useNoteChat(note));
    await waitFor(() => expect(result.current.loading).toBe(false));
    await waitFor(() => expect(result.current.storedSessionId).toBe(stored.id));
    let release: (value: typeof stored) => void = () => undefined;
    mocks.getSession.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "note-cancel-refresh",
          sessionId: stored.id,
          runId: cancelled.id,
          sequence: 50,
          method: "run.cancelled",
          data: { completedAt: "now" },
        },
      }),
    );
    await waitFor(() => expect(mocks.listItems).toHaveBeenCalledTimes(2));
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "note-late-f10",
          sessionId: stored.id,
          runId: cancelled.id,
          sequence: 51,
          method: "run.diagnostic",
          data: {
            itemId: "note-late-warning",
            itemSequence: 2,
            createdAt: "now",
            message,
            category: "runtime",
            code: "agent_workspace_mcp_cleanup",
            retryable: false,
          },
        },
      }),
    );
    expect(JSON.stringify(result.current.turns)).toContain(message);
    await act(async () => release(stored));
    expect(JSON.stringify(result.current.turns)).toContain(message);
    expect(result.current.turns).toHaveLength(1);
    expect(result.current.working).toBe(false);
    expect(mocks.startRun).not.toHaveBeenCalled();
    // Reopen from the authoritative stored item, without duplicating the event.
    unmount();
    mocks.listItems.mockResolvedValue([
      {
        id: "note-late-warning",
        sessionId: stored.id,
        runId: cancelled.id,
        sequence: 2,
        createdAt: "now",
        kind: "error",
        message,
        category: "runtime",
        code: "agent_workspace_mcp_cleanup",
        retryable: false,
      },
    ]);
    const reopened = renderHook(() => useNoteChat(note));
    await waitFor(() => expect(reopened.result.current.turns).toHaveLength(1));
    expect(JSON.stringify(reopened.result.current.turns)).toContain(message);
    expect(reopened.result.current.working).toBe(false);
  });

  it("F10 discards an old pending diagnostic hydration after switching notes and never invents an error", async () => {
    const stored = {
      id: "note-session",
      title: "Kickoff",
      status: "idle",
      model: "__clovy_cli_engine__:agy",
      safetyMode: "sandboxed",
      workspacePath: "/tmp/note-session",
      source: "user",
      createdAt: "now",
      updatedAt: "now",
    };
    rememberNoteChatSession(note.id, stored.id);
    mocks.getSession.mockResolvedValue(stored);
    const view = renderHook(({ selected }) => useNoteChat(selected), {
      initialProps: { selected: note },
    });
    await waitFor(() => expect(view.result.current.loading).toBe(false));
    expect(view.result.current.turns).toEqual([]);
    let release: (value: typeof stored) => void = () => undefined;
    mocks.getSession.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          release = resolve;
        }),
    );
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "switch-cancel",
          sessionId: stored.id,
          runId: "old-run",
          sequence: 50,
          method: "run.cancelled",
          data: { completedAt: "now" },
        },
      }),
    );
    await waitFor(() => expect(mocks.listItems).toHaveBeenCalledTimes(2));
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "switch-diagnostic",
          sessionId: stored.id,
          runId: "old-run",
          sequence: 51,
          method: "run.diagnostic",
          data: {
            itemId: "old-warning",
            itemSequence: 2,
            createdAt: "now",
            message: "Old note cleanup",
            category: "runtime",
            code: "agent_workspace_mcp_cleanup",
            retryable: false,
          },
        },
      }),
    );
    expect(JSON.stringify(view.result.current.turns)).toContain("Old note cleanup");
    view.rerender({ selected: { id: "different-note", title: "Different" } });
    await waitFor(() => expect(view.result.current.turns).toEqual([]));
    await act(async () => release(stored));
    expect(view.result.current.turns).toEqual([]);
    expect(view.result.current.working).toBe(false);
    expect(mocks.startRun).not.toHaveBeenCalled();
  });

  beforeEach(() => {
    window.localStorage.clear();
    mocks.invoke.mockReset();
    mocks.runtimeListener = undefined;
    mocks.createSession.mockReset();
    mocks.getSession.mockReset();
    mocks.startRun.mockReset();
    mocks.listItems.mockReset().mockResolvedValue([]);
    mocks.getLatestRun.mockReset().mockResolvedValue(null);
    mocks.createSession.mockImplementation(async (request: { model: string }) => ({
      id: "note-session",
      title: "Kickoff",
      status: "idle",
      model: request.model,
      safetyMode: "sandboxed",
      workspacePath: "/tmp/note-session",
      source: "user",
      createdAt: "2026-10-15T12:00:00Z",
      updatedAt: "2026-10-15T12:00:00Z",
    }));
    mocks.startRun.mockResolvedValue({
      id: "run-1",
      sessionId: "note-session",
      status: "running",
      model: "__clovy_cli_engine__:claude",
    });
  });

  it.each([
    "claude",
    "codex",
    "agy",
  ])("starts a new note chat with the only installed CLI %s and the note referenced", async (only) => {
    // The bridge is a boundary stub; resolve its choice with the production
    // classifier. Rust selection/persistence has its own isolated proof.
    const clis = ["claude", "codex", "agy"].map((id) => ({
      id,
      name: id,
      installed: id === only,
      reason: null,
      modelId: `__clovy_cli_engine__:${id}`,
    }));
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "chat_default_engine" ? { modelId: chatModelId("auto", { clis }) } : undefined,
    );
    const { result } = renderHook(() => useNoteChat(note));
    await waitFor(() => expect(result.current.model).toBe(`__clovy_cli_engine__:${only}`));
    await act(async () => {
      await result.current.submit("What did we decide?");
    });
    expect(mocks.createSession).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({ model: `__clovy_cli_engine__:${only}` }),
    );
    expect(mocks.startRun).toHaveBeenCalledExactlyOnceWith(
      expect.objectContaining({
        model: `__clovy_cli_engine__:${only}`,
        prompt: '@note:note-7 ("Kickoff") What did we decide?',
      }),
    );
  });

  it("shows cleanup diagnostics without announcing note chat as running or starting another turn", async () => {
    const message = "Workspace MCP cleanup needs recovery: configuration changed.";
    rememberNoteChatSession(note.id, "note-session");
    mocks.getSession.mockResolvedValue({
      id: "note-session",
      title: "Kickoff",
      status: "idle",
      model: "__clovy_cli_engine__:claude",
      safetyMode: "sandboxed",
      workspacePath: "/tmp/note-session",
      source: "user",
      createdAt: "now",
      updatedAt: "now",
    });
    const { result } = renderHook(() => useNoteChat(note));
    await waitFor(() => expect(result.current.loading).toBe(false));
    await waitFor(() => expect(result.current.storedSessionId).toBe("note-session"));
    const statuses: string[] = [];
    const status = (event: Event) => statuses.push((event as CustomEvent).detail.status);
    window.addEventListener(AGENT_SESSION_STATUS_EVENT, status);
    try {
      act(() =>
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "late-cleanup",
            sessionId: "note-session",
            runId: "cancelled-run",
            sequence: 51,
            method: "run.diagnostic",
            data: {
              itemId: "warning",
              itemSequence: 2,
              createdAt: "now",
              message,
              category: "runtime",
              code: "agent_workspace_mcp_cleanup",
              retryable: false,
            },
          },
        }),
      );
      expect(result.current.turns[0].parts).toContainEqual(
        expect.objectContaining({ type: "notice", text: message }),
      );
      expect(result.current.working).toBe(false);
      expect(statuses).not.toContain("running");
      expect(mocks.createSession).not.toHaveBeenCalled();
      expect(mocks.startRun).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener(AGENT_SESSION_STATUS_EVENT, status);
    }
  });

  it("uses the default CLI even when the message is sent before it was read", async () => {
    let resolveEngine: (value: unknown) => void = () => undefined;
    let calls = 0;
    mocks.invoke.mockImplementation((command: string) => {
      if (command !== "chat_default_engine") return Promise.resolve(undefined);
      calls += 1;
      return calls === 1
        ? new Promise((resolve) => {
            resolveEngine = resolve;
          })
        : Promise.resolve({ modelId: "__clovy_cli_engine__:codex" });
    });
    const { result } = renderHook(() => useNoteChat(note));

    await act(async () => {
      await result.current.submit("Summarize it");
    });
    resolveEngine({ modelId: "__clovy_cli_engine__:codex" });

    expect(mocks.startRun).toHaveBeenCalledWith(
      expect.objectContaining({ model: "__clovy_cli_engine__:codex" }),
    );
  });

  it("defaults to Claude Code when the stored engine is native", async () => {
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "chat_default_engine"
        ? { modelId: null, cli: null, legacyFallback: true }
        : undefined,
    );
    const { result } = renderHook(() => useNoteChat(note));
    await act(async () => {
      await result.current.submit("Hello");
    });
    expect(mocks.startRun).toHaveBeenCalledWith(
      expect.objectContaining({ model: "__clovy_cli_engine__:claude" }),
    );
  });

  it.each([
    "panel",
    "external",
  ])("keeps a newer %s engine choice while a legacy session default resolves", async (choiceSource) => {
    rememberNoteChatSession(note.id, "note-session");
    mocks.getSession.mockResolvedValue({
      id: "note-session",
      title: "Kickoff",
      status: "idle",
      model: "legacy-model",
      safetyMode: "sandboxed",
      workspacePath: "/tmp/note-session",
      source: "user",
      createdAt: "2026-10-15T12:00:00Z",
      updatedAt: "2026-10-15T12:00:00Z",
    });
    let resolveDefault: (value: unknown) => void = () => undefined;
    mocks.invoke.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveDefault = resolve;
        }),
    );
    const { result } = renderHook(() => useNoteChat(note));
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith("chat_default_engine"));
    act(() => {
      if (choiceSource === "panel") result.current.setModel("__clovy_cli_engine__:codex");
      else rememberSessionModel("note-session", "__clovy_cli_engine__:codex");
    });
    await act(async () => {
      resolveDefault({ modelId: "__clovy_cli_engine__:claude" });
    });
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.model).toBe("__clovy_cli_engine__:codex");
    expect(loadSessionModels()["note-session"]).toBe("__clovy_cli_engine__:codex");
    await act(async () => {
      await result.current.submit("Use my choice");
    });
    expect(mocks.startRun).toHaveBeenCalledWith(
      expect.objectContaining({ model: "__clovy_cli_engine__:codex" }),
    );
  });

  it("ignores an old legacy default after leaving and returning to the same note session", async () => {
    rememberNoteChatSession(note.id, "note-session");
    mocks.getSession.mockResolvedValue({
      id: "note-session",
      title: "Kickoff",
      status: "idle",
      model: "legacy-model",
      safetyMode: "sandboxed",
      workspacePath: "/tmp/note-session",
      source: "user",
      createdAt: "2026-10-15T12:00:00Z",
      updatedAt: "2026-10-15T12:00:00Z",
    });
    let resolveFirstDefault: (value: unknown) => void = () => undefined;
    mocks.invoke.mockResolvedValue({ modelId: "__clovy_cli_engine__:codex" });
    mocks.invoke.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirstDefault = resolve;
        }),
    );
    const { result, rerender } = renderHook((currentNote) => useNoteChat(currentNote), {
      initialProps: note,
    });
    await waitFor(() => expect(mocks.invoke).toHaveBeenCalledTimes(1));
    rerender({ id: "another-note", title: "Another note" });
    await waitFor(() => expect(result.current.model).toBe("__clovy_cli_engine__:codex"));
    rerender(note);
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.model).toBe("__clovy_cli_engine__:codex");
    await act(async () => {
      resolveFirstDefault({ modelId: "__clovy_cli_engine__:claude" });
    });
    expect(result.current.model).toBe("__clovy_cli_engine__:codex");
    expect(loadSessionModels()["note-session"]).toBe("__clovy_cli_engine__:codex");
  });

  it("keeps an engine picked in the panel", async () => {
    mocks.invoke.mockImplementation(async (command: string) =>
      command === "chat_default_engine" ? { modelId: "__clovy_cli_engine__:claude" } : undefined,
    );
    const { result } = renderHook(() => useNoteChat(note));
    act(() => result.current.setModel("__clovy_cli_engine__:codex"));
    await act(async () => {
      await result.current.submit("Hello");
    });
    expect(mocks.startRun).toHaveBeenCalledWith(
      expect.objectContaining({ model: "__clovy_cli_engine__:codex" }),
    );
  });
});
