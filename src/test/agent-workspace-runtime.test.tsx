import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AgentRuntimeEvent,
  AgentSessionDto,
  AgentSkillDto,
} from "../lib/agent-runtime-contract";
import { applyInterfaceLocale } from "../i18n/locale";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  openDialog: vi.fn(),
  runtimeListener: undefined as ((event: { payload: AgentRuntimeEvent }) => void) | undefined,
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage?: (event: unknown) => void;
  },
  convertFileSrc: vi.fn((path: string) => path),
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(),
  listen: vi.fn(
    async (_name: string, listener: (event: { payload: AgentRuntimeEvent }) => void) => {
      mocks.runtimeListener = listener;
      return vi.fn();
    },
  ),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: mocks.openDialog }));

import { AgentWorkspace } from "../components/agent/AgentWorkspace";
import { markAgentNewSessionPending } from "../components/agent/session-persistence";
import { agentComposerClearance } from "../components/agent/composer/layout";
import {
  AGENT_DELETE_SESSION_EVENT,
  AGENT_NEW_SESSION_EVENT,
  AGENT_SESSION_STATUS_EVENT,
} from "../lib/agent-events";
import {
  resetCurrentDataPartitionForTests,
  setCurrentDataPartitionName,
} from "../lib/data-partition";
import { rememberSessionModel } from "../lib/agent-session-models";
import {
  readAgentSessionDraft,
  resetAgentSessionDraftsForTests,
  writeAgentSessionDraft,
} from "../lib/agent-session-drafts";
import { readClovyHomeStoredSessionId, writeClovyHomeStoredSessionId } from "../lib/clovy-home";
import { ATTACHMENT_FOLLOW_UP_NOTE, saveQueuedAgentFollowUps } from "../lib/agent-follow-up-queue";

const session: AgentSessionDto = {
  id: "session-1",
  title: "Existing session",
  status: "idle",
  model: "fast",
  safetyMode: "sandboxed",
  workspacePath: "/tmp/session-1",
  source: "user",
  createdAt: "2026-07-22T12:00:00Z",
  updatedAt: "2026-07-22T12:00:00Z",
};

const newSession: AgentSessionDto = {
  ...session,
  id: "session-2",
  title: "Fresh request",
  workspacePath: "/tmp/session-2",
};

function linearApprovalEvent(): AgentRuntimeEvent {
  return {
    protocolVersion: 1,
    eventId: "event-approval",
    sessionId: session.id,
    runId: "run-linear",
    sequence: 2,
    method: "interruption.requested",
    data: {
      itemSequence: 2,
      itemId: "approval-item",
      interruption: {
        id: "functions.mcp_linear_save_issue:0",
        kind: "approval",
        sessionId: session.id,
        runId: "run-linear",
        status: "pending",
        createdAt: "2026-07-22T12:00:02Z",
        toolName: "mcp_linear_save_issue",
        title: "Approval required",
        description: "Clovy wants to create a Linear issue.",
        command: "mcp_linear_save_issue",
        allowAlways: false,
      },
    },
  };
}

function mockAgentLayoutBounds() {
  return vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLElement,
  ) {
    const top = this.classList.contains("agent-composer") ? 520 : 0;
    const bottom = this.classList.contains("agent-scroll") ? 640 : top;
    return {
      x: 0,
      y: top,
      top,
      right: 0,
      bottom,
      left: 0,
      width: 0,
      height: Math.max(0, bottom - top),
      toJSON: () => ({}),
    } as DOMRect;
  });
}

describe("AgentWorkspace runtime wiring", () => {
  it.each([
    "diagnostic",
    "clean",
    "switch",
  ] as const)("F10b initialSessionId hydration preserves only its own real diagnostic: %s", async (scenario) => {
    const message = "Initial workspace cleanup needs recovery.";
    const base = mocks.invoke.getMockImplementation();
    let release: (value: typeof session) => void = () => undefined;
    const initial = new Promise<typeof session>((resolve) => {
      release = resolve;
    });
    const cancelled = {
      id: "initial-cancelled-run",
      sessionId: session.id,
      status: "cancelled",
      model: session.model,
    };
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      const id = (args as { sessionId?: string } | undefined)?.sessionId;
      if (command === "get_agent_session")
        return id === session.id ? initial : Promise.resolve(newSession);
      if (command === "list_agent_items") return Promise.resolve([]);
      if (command === "get_latest_agent_run")
        return Promise.resolve(id === session.id ? cancelled : null);
      return base?.(command, args);
    });
    const { rerender } = render(<AgentWorkspace initialSessionId={session.id} />);
    await waitFor(() => expect(mocks.runtimeListener).toBeDefined());
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("list_agent_items", { sessionId: session.id }),
    );
    // listItems is already empty; getSession is still the initial held read.
    await act(async () => Promise.resolve());
    if (scenario !== "clean") {
      act(() =>
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "initial-workspace-warning",
            sessionId: session.id,
            runId: cancelled.id,
            sequence: 51,
            method: "run.diagnostic",
            data: {
              itemId: "initial-warning",
              itemSequence: 2,
              createdAt: session.createdAt,
              message,
              category: "runtime",
              code: "agent_workspace_mcp_cleanup",
              retryable: false,
            },
          },
        }),
      );
    }
    if (scenario === "switch") {
      rerender(<AgentWorkspace initialSessionId={newSession.id} />);
      await waitFor(() =>
        expect(mocks.invoke).toHaveBeenCalledWith("get_agent_session", {
          sessionId: newSession.id,
        }),
      );
    }
    await act(async () => release(session));
    if (scenario === "diagnostic") {
      expect(await screen.findByText(message)).toBeInTheDocument();
      expect(screen.getAllByText(message)).toHaveLength(1);
    } else {
      expect(screen.queryByText(message)).not.toBeInTheDocument();
    }
    expect(screen.queryByRole("button", { name: "Stop Clovy" })).not.toBeInTheDocument();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "start_agent_run")).toBe(false);
  });

  it("F10 preserves a late cleanup diagnostic while a cancelled snapshot finishes hydrating", async () => {
    const message = "Late workspace cleanup needs recovery.";
    const cancelled = {
      id: "cancelled-run",
      sessionId: session.id,
      status: "cancelled",
      model: "__clovy_cli_engine__:agy",
    };
    const base = mocks.invoke.getMockImplementation();
    let pending = false;
    let release: (value: unknown[]) => void = () => undefined;
    const artifacts = new Promise<unknown[]>((resolve) => {
      release = resolve;
    });
    let storedItems: unknown[] = [];
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      const id = (args as { sessionId?: string } | undefined)?.sessionId;
      if (command === "get_agent_session")
        return Promise.resolve(id === newSession.id ? newSession : session);
      if (command === "get_latest_agent_run")
        return Promise.resolve(id === session.id ? cancelled : null);
      if (command === "list_agent_items")
        return Promise.resolve(id === session.id ? storedItems : []);
      if (command === "list_agent_artifacts" && pending) return artifacts;
      return base?.(command, args);
    });
    const { rerender } = render(<AgentWorkspace initialSession={session} />);
    await screen.findByRole("textbox", { name: "Message Clovy" });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_latest_agent_run", { sessionId: session.id }),
    );
    await act(async () => Promise.resolve());
    pending = true;
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "cancel-refresh",
          sessionId: session.id,
          runId: cancelled.id,
          sequence: 50,
          method: "run.cancelled",
          data: { completedAt: session.createdAt },
        },
      }),
    );
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.filter(([command]) => command === "list_agent_items"),
      ).toHaveLength(2),
    );
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "late-f10",
          sessionId: session.id,
          runId: cancelled.id,
          sequence: 51,
          method: "run.diagnostic",
          data: {
            itemId: "late-warning",
            itemSequence: 2,
            createdAt: session.createdAt,
            message,
            category: "runtime",
            code: "agent_workspace_mcp_cleanup",
            retryable: false,
          },
        },
      }),
    );
    expect(await screen.findByText(message)).toBeInTheDocument();
    await act(async () => {
      release([]);
    });
    expect(screen.getAllByText(message)).toHaveLength(1);
    expect(screen.queryByRole("button", { name: "Stop Clovy" })).not.toBeInTheDocument();
    storedItems = [
      {
        id: "late-warning",
        sessionId: session.id,
        runId: cancelled.id,
        sequence: 2,
        createdAt: session.createdAt,
        kind: "error",
        message,
        category: "runtime",
        code: "agent_workspace_mcp_cleanup",
        retryable: false,
      },
    ];
    pending = false;
    rerender(<AgentWorkspace initialSession={newSession} />);
    await waitFor(() => expect(screen.queryByText(message)).not.toBeInTheDocument());
    rerender(<AgentWorkspace initialSession={session} />);
    expect(await screen.findByText(message)).toBeInTheDocument();
    expect(screen.getAllByText(message)).toHaveLength(1);
  });

  it("F10 rejects a pending old-session diagnostic snapshot after switching to a session without errors", async () => {
    const base = mocks.invoke.getMockImplementation();
    let pending = false;
    let release: (value: unknown[]) => void = () => undefined;
    const artifacts = new Promise<unknown[]>((resolve) => {
      release = resolve;
    });
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      const id = (args as { sessionId?: string } | undefined)?.sessionId;
      if (command === "get_agent_session")
        return Promise.resolve(id === newSession.id ? newSession : session);
      if (command === "get_latest_agent_run") return Promise.resolve(null);
      if (command === "list_agent_items") return Promise.resolve([]);
      if (command === "list_agent_artifacts" && pending && id === session.id) return artifacts;
      return base?.(command, args);
    });
    const { rerender } = render(<AgentWorkspace initialSession={session} />);
    await screen.findByRole("textbox", { name: "Message Clovy" });
    await act(async () => Promise.resolve());
    pending = true;
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "switch-cancel",
          sessionId: session.id,
          runId: "old-run",
          sequence: 50,
          method: "run.cancelled",
          data: { completedAt: session.createdAt },
        },
      }),
    );
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.filter(([command]) => command === "list_agent_items"),
      ).toHaveLength(2),
    );
    act(() =>
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "switch-diagnostic",
          sessionId: session.id,
          runId: "old-run",
          sequence: 51,
          method: "run.diagnostic",
          data: {
            itemId: "old-warning",
            itemSequence: 2,
            createdAt: session.createdAt,
            message: "Old workspace cleanup",
            category: "runtime",
            code: "agent_workspace_mcp_cleanup",
            retryable: false,
          },
        },
      }),
    );
    expect(await screen.findByText("Old workspace cleanup")).toBeInTheDocument();
    rerender(<AgentWorkspace initialSession={newSession} />);
    await waitFor(() =>
      expect(screen.queryByText("Old workspace cleanup")).not.toBeInTheDocument(),
    );
    await act(async () => release([]));
    expect(screen.queryByText("Old workspace cleanup")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop Clovy" })).not.toBeInTheDocument();
  });

  it.each([
    ["en", "Workspace connection needs recovery."],
    ["pt-BR", "A conexão da pasta de trabalho precisa de recuperação."],
  ] as const)("shows a late cleanup diagnostic in %s without announcing a cancelled conversation as running", async (locale, heading) => {
    applyInterfaceLocale(locale);
    const message = "Workspace MCP cleanup needs recovery: configuration changed.";
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");
    await waitFor(() => expect(mocks.runtimeListener).toBeDefined());
    const statuses: string[] = [];
    const status = (event: Event) => statuses.push((event as CustomEvent).detail.status);
    window.addEventListener(AGENT_SESSION_STATUS_EVENT, status);
    try {
      act(() =>
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "late-cleanup",
            sessionId: session.id,
            runId: "cancelled-run",
            sequence: 51,
            method: "run.diagnostic",
            data: {
              itemId: "warning",
              itemSequence: 2,
              createdAt: session.createdAt,
              message,
              category: "runtime",
              code: "agent_workspace_mcp_cleanup",
              retryable: false,
            },
          },
        }),
      );
      expect(await screen.findByText(message)).toBeInTheDocument();
      expect(screen.getByText(heading)).toBeInTheDocument();
      expect(statuses).not.toContain("running");
      expect(screen.queryByRole("button", { name: "Stop Clovy" })).not.toBeInTheDocument();
    } finally {
      window.removeEventListener(AGENT_SESSION_STATUS_EVENT, status);
      act(() => applyInterfaceLocale("en"));
    }
  });

  beforeEach(() => {
    resetAgentSessionDraftsForTests();
    resetCurrentDataPartitionForTests();
    window.localStorage.clear();
    mocks.runtimeListener = undefined;
    mocks.invoke.mockReset();
    mocks.openDialog.mockReset();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_sessions") return Promise.resolve([session]);
      if (command === "get_agent_session") return Promise.resolve(session);
      if (command === "list_agent_items") {
        return Promise.resolve([
          {
            id: "message-1",
            sessionId: session.id,
            sequence: 1,
            createdAt: session.createdAt,
            kind: "message",
            role: "assistant",
            text: "Earlier answer",
            status: "complete",
          },
        ]);
      }
      if (command === "list_agent_artifacts") return Promise.resolve([]);
      if (command === "list_agent_skills") {
        return Promise.resolve([
          {
            id: "notes",
            name: "Notes",
            description: "Work with Clovy notes.",
            source: "managed",
            enabled: true,
            configuredEnabled: true,
            readable: true,
            error: null,
            editable: true,
          },
          {
            id: "disabled",
            name: "Disabled",
            description: "Disabled skill.",
            source: "managed",
            enabled: false,
            configuredEnabled: false,
            readable: true,
            error: null,
            editable: true,
          },
        ] satisfies AgentSkillDto[]);
      }
      if (command === "list_provider_models") {
        return Promise.resolve({
          mode: "generation",
          selectedModel: "open-software/auto",
          modelType: "text",
          models: [
            {
              provider: "june",
              id: "open-software/auto",
              name: "Auto",
              modelType: "text",
              traits: [],
              capabilities: [],
            },
            {
              provider: "june",
              id: "fast",
              name: "Fast",
              modelType: "text",
              traits: [],
              capabilities: ["tool-calling"],
              privacy: "private",
              contextTokens: 200_000,
            },
          ],
        });
      }
      if (command === "provider_model_settings") {
        return Promise.resolve({
          settings: {},
          effectiveSettings: {
            providerApiKeysConfigured: {
              groq: false,
              openrouter: false,
              fal: false,
              tavily: false,
            },
          },
        });
      }

      if (command === "create_agent_session") return Promise.resolve(newSession);
      if (command === "start_agent_run") {
        return Promise.resolve({
          id: "run-1",
          sessionId: session.id,
          status: "running",
          model: "auto",
        });
      }
      if (command === "steer_agent_run") {
        return Promise.resolve({ accepted: true });
      }
      return Promise.resolve(undefined);
    });
  });

  it("reserves the overlap between the transcript and fixed composer", () => {
    expect(agentComposerClearance(800, 620)).toBe(180);
    expect(agentComposerClearance(600, 620)).toBe(0);
  });

  it("keeps persisted files on their user message instead of repeating them after the thread", async () => {
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_items") {
        return Promise.resolve([
          {
            id: "message-with-attachment",
            sessionId: session.id,
            runId: "run-1",
            sequence: 1,
            createdAt: session.createdAt,
            kind: "message",
            role: "user",
            text: "Review this contract.",
            status: "complete",
            attachments: [
              {
                id: "attachment-1",
                sessionId: session.id,
                runId: "run-1",
                itemId: "message-with-attachment",
                name: "Pool Day.pdf",
                path: "/tmp/session-1/attachments/internal-Pool Day.pdf",
                mimeType: "application/pdf",
                sizeBytes: 115_000,
                action: "imported",
                available: true,
                createdAt: session.createdAt,
              },
            ],
          },
        ]);
      }
      if (command === "list_agent_artifacts") {
        return Promise.resolve([
          {
            id: "attachment-1",
            sessionId: session.id,
            runId: "run-1",
            itemId: "message-with-attachment",
            name: "Pool Day.pdf",
            path: "/tmp/session-1/attachments/internal-Pool Day.pdf",
            mimeType: "application/pdf",
            sizeBytes: 115_000,
            action: "imported",
            available: true,
            createdAt: session.createdAt,
          },
        ]);
      }
      return defaultInvoke?.(command, args);
    });

    const user = userEvent.setup();
    const { container } = render(<AgentWorkspace initialSession={session} />);
    const userTurn = await screen
      .findByText("Review this contract.")
      .then((text) => text.closest(".agent-user-turn"));
    expect(userTurn).not.toBeNull();
    const attachmentGroup = within(userTurn as HTMLElement).getByRole("group", {
      name: "Attachments",
    });
    expect(attachmentGroup).toHaveTextContent("Pool Day.pdf");
    const turnActions = userTurn?.querySelector(".agent-turn-actions");
    expect(turnActions).toBeTruthy();
    expect(
      attachmentGroup.compareDocumentPosition(turnActions as Node) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(container.querySelector(".agent-timeline > .agent-artifact-list")).toBeNull();
    expect(screen.getAllByText("Pool Day.pdf")).toHaveLength(1);

    await user.click(
      within(userTurn as HTMLElement).getByRole("button", { name: "Open Pool Day.pdf" }),
    );
    expect(screen.getByRole("complementary", { name: "Files" })).toHaveTextContent("Pool Day.pdf");
    await user.click(screen.getByRole("button", { name: "Close files" }));

    await user.click(screen.getByRole("button", { name: "View files (1)" }));
    expect(screen.getByRole("complementary", { name: "Files" })).toHaveTextContent("Pool Day.pdf");
  });

  it("keeps pending serialized drafts isolated by stored session through an immediate switch", async () => {
    const user = userEvent.setup();
    const draft = 'Review @note:note-7 ("Research")';
    const { rerender } = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    await user.type(composer, draft);
    // Do not wait for ComposerEditor's 75ms trailing publish: switching must
    // preserve the live document using its teardown persistence callback.
    rerender(<AgentWorkspace initialSession={newSession} />);
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Message Clovy" }).textContent).toBe("");
    });

    rerender(<AgentWorkspace initialSession={session} />);
    await waitFor(() => {
      expect(readAgentSessionDraft(session.id)).toBe(draft);
    });
  });

  it("finishes IME composition under the source draft owner before switching", async () => {
    const user = userEvent.setup();
    const { rerender } = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    fireEvent.compositionStart(composer);
    await user.type(composer, "編集中");
    rerender(<AgentWorkspace initialSession={newSession} />);

    await waitFor(() => expect(composer).toHaveTextContent("編集中"));
    fireEvent.compositionEnd(composer);
    await waitFor(() => expect(readAgentSessionDraft(session.id)).toBe("編集中"));
    await waitFor(() => expect(screen.getByRole("textbox").textContent).toBe(""));

    rerender(<AgentWorkspace initialSession={session} />);
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveTextContent("編集中"));
  });

  it("does not submit a destination draft during an IME owner handoff", async () => {
    const user = userEvent.setup();
    writeAgentSessionDraft(newSession.id, "Destination draft");
    const { rerender } = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    fireEvent.compositionStart(composer);
    await user.type(composer, "Source composition");
    rerender(<AgentWorkspace initialSession={newSession} />);

    fireEvent.compositionEnd(composer);
    fireEvent.keyDown(composer, { key: "Enter" });
    expect(mocks.invoke).not.toHaveBeenCalledWith("start_agent_run", expect.anything());
    await waitFor(() => expect(composer).toHaveTextContent("Destination draft"));
  });

  it("persists a pending draft when the workspace unmounts before the editor debounce", async () => {
    const user = userEvent.setup();
    const draft = "Keep this unfinished message";
    const view = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    await user.type(composer, draft);
    view.unmount();

    render(<AgentWorkspace initialSession={session} />);
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Message Clovy" }).textContent).toBe(draft);
    });
  });

  it("does not restore a session draft after its message is accepted", async () => {
    const user = userEvent.setup();
    const view = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    await user.type(composer, "Send this message");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => {
      expect(mocks.invoke).toHaveBeenCalledWith(
        "start_agent_run",
        expect.objectContaining({
          request: expect.objectContaining({ prompt: "Send this message" }),
        }),
      );
      expect(readAgentSessionDraft(session.id)).toBeUndefined();
    });

    view.unmount();
    render(<AgentWorkspace initialSession={session} />);
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "Message Clovy" }).textContent).toBe("");
    });
  });

  it("keeps a newer same-text draft when an older send finishes", async () => {
    let resolveStart: ((value: unknown) => void) | undefined;
    const pendingStart = new Promise((resolve) => {
      resolveStart = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "start_agent_run") return pendingStart;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    const view = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    await user.type(composer, "Same message");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", expect.anything()),
    );
    await waitFor(() => expect(composer.textContent).toBe(""));
    await user.type(composer, "Same message");
    await waitFor(() => expect(readAgentSessionDraft(session.id)).toBe("Same message"));

    await act(async () => {
      resolveStart?.({
        id: "run-same-message",
        sessionId: session.id,
        status: "running",
        model: "__clovy_cli_engine__:claude",
      });
    });
    await waitFor(() => expect(readAgentSessionDraft(session.id)).toBe("Same message"));

    view.unmount();
    render(<AgentWorkspace initialSession={session} />);
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveTextContent("Same message"));
  });

  it("transfers a fresh-session follow-up when navigation unmounts the workspace", async () => {
    let resolveCreate: ((value: AgentSessionDto) => void) | undefined;
    const pendingCreate = new Promise<AgentSessionDto>((resolve) => {
      resolveCreate = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "create_agent_session") return pendingCreate;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    const view = render(<AgentWorkspace />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    await user.type(composer, "Start a fresh session");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", expect.anything()),
    );
    const followUpComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(followUpComposer, "Keep this follow-up");
    await new Promise((resolve) => window.setTimeout(resolve, 100));
    view.unmount();

    await act(async () => resolveCreate?.(newSession));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", expect.anything()),
    );
    render(<AgentWorkspace initialSession={newSession} />);
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
        "Keep this follow-up",
      ),
    );
  });

  it("transfers a first Home follow-up after its editor publishes normally", async () => {
    const homeSession: AgentSessionDto = {
      ...session,
      id: "home-first-session",
      title: "Home",
      workspacePath: "/tmp/home-first-session",
    };
    let resolveCreate: ((value: AgentSessionDto) => void) | undefined;
    const pendingCreate = new Promise<AgentSessionDto>((resolve) => {
      resolveCreate = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_sessions") return Promise.resolve([]);
      if (command === "create_agent_session") return pendingCreate;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    const view = render(<AgentWorkspace homeMode />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    await user.type(composer, "First Home message");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", expect.anything()),
    );
    await user.type(screen.getByRole("textbox"), "Keep this Home follow-up");
    await new Promise((resolve) => window.setTimeout(resolve, 100));
    view.unmount();

    await act(async () => resolveCreate?.(homeSession));
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
        "Keep this Home follow-up",
      ),
    );
  });

  it("clears and fences drafts deleted by another session surface", async () => {
    const user = userEvent.setup();
    const view = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Published draft");
    await waitFor(() => expect(readAgentSessionDraft(session.id)).toBe("Published draft"));

    act(() => {
      window.dispatchEvent(
        new CustomEvent(AGENT_DELETE_SESSION_EVENT, { detail: { sessionId: session.id } }),
      );
    });
    expect(readAgentSessionDraft(session.id)).toBeUndefined();

    composer.textContent = "Late pending draft";
    fireEvent.input(composer);
    view.unmount();
    await new Promise((resolve) => window.setTimeout(resolve, 100));
    expect(readAgentSessionDraft(session.id)).toBeUndefined();
  });

  it("leaves an externally deleted workspace and rejects its late hydration", async () => {
    let finishSession: ((value: AgentSessionDto) => void) | undefined;
    const late = new Promise<AgentSessionDto>((resolve) => {
      finishSession = resolve;
    });
    const fallback = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) =>
      command === "get_agent_session" ? late : fallback?.(command, args),
    );
    const selected = vi.fn();
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} onSessionSelected={selected} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_agent_session", { sessionId: session.id }),
    );
    act(() =>
      window.dispatchEvent(
        new CustomEvent(AGENT_DELETE_SESSION_EVENT, { detail: { sessionId: session.id } }),
      ),
    );
    await act(async () => finishSession?.(session));
    expect(screen.queryByRole("button", { name: "Session actions" })).not.toBeInTheDocument();
    expect(selected).toHaveBeenCalledWith(undefined);
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "New conversation");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", expect.anything()),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "start_agent_run",
        expect.objectContaining({ request: expect.objectContaining({ sessionId: newSession.id }) }),
      ),
    );
    const starts = mocks.invoke.mock.calls.filter(([name]) => name === "start_agent_run");
    expect(
      starts.every(
        ([, args]) => (args as { request: { sessionId: string } }).request.sessionId !== session.id,
      ),
    ).toBe(true);
  });

  it("ignores a hydration error returned after that conversation was deleted", async () => {
    let rejectSession: ((reason: Error) => void) | undefined;
    const late = new Promise<AgentSessionDto>((_, reject) => {
      rejectSession = reject;
    });
    const fallback = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) =>
      command === "get_agent_session" ? late : fallback?.(command, args),
    );
    render(<AgentWorkspace initialSession={session} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_agent_session", { sessionId: session.id }),
    );
    act(() =>
      window.dispatchEvent(
        new CustomEvent(AGENT_DELETE_SESSION_EVENT, { detail: { sessionId: session.id } }),
      ),
    );
    await act(async () => rejectSession?.(new Error("Deleted conversation hydration failed")));
    expect(screen.queryByText("Deleted conversation hydration failed")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Session actions" })).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toBeInTheDocument();
  });

  it("does not let a late failed send recreate an externally deleted draft", async () => {
    let rejectStart: ((cause: Error) => void) | undefined;
    const pendingStart = new Promise((_, reject) => {
      rejectStart = reject;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "start_agent_run") return pendingStart;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    const view = render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });

    await user.type(composer, "Do not resurrect this");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", expect.anything()),
    );
    act(() => {
      window.dispatchEvent(
        new CustomEvent(AGENT_DELETE_SESSION_EVENT, { detail: { sessionId: session.id } }),
      );
    });
    await act(async () => rejectStart?.(new Error("Runtime failed after deletion")));
    expect(readAgentSessionDraft(session.id)).toBeUndefined();

    view.unmount();
    render(<AgentWorkspace initialSession={session} />);
    await waitFor(() => expect(screen.getByRole("textbox").textContent).toBe(""));
  });

  it("does not re-cache a pending draft after its stored session is deleted", async () => {
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });
    await user.click(screen.getByRole("button", { name: "Session actions" }));

    composer.textContent = "Delete this pending draft";
    fireEvent.input(composer);
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete session" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("delete_agent_session", {
        sessionId: session.id,
      }),
    );
    await new Promise((resolve) => window.setTimeout(resolve, 100));
    expect(readAgentSessionDraft(session.id)).toBeUndefined();
  });

  it("clears a published draft from the composer when its session is deleted", async () => {
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Delete this published draft");
    await waitFor(() =>
      expect(readAgentSessionDraft(session.id)).toBe("Delete this published draft"),
    );

    await user.click(screen.getByRole("button", { name: "Session actions" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Delete session" }));

    await waitFor(() => expect(readAgentSessionDraft(session.id)).toBeUndefined());
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveTextContent(""));
  });

  it("reserves the fixed composer before Home has a persisted session", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_agent_sessions") return Promise.resolve([]);
      if (command === "list_provider_models") {
        return Promise.resolve({
          mode: "generation",
          selectedModel: "open-software/auto",
          modelType: "text",
          models: [],
        });
      }
      return Promise.resolve(undefined);
    });
    const bounds = mockAgentLayoutBounds();

    try {
      const { container } = render(<AgentWorkspace homeMode />);
      const scroller = container.querySelector<HTMLElement>(".agent-scroll");

      await waitFor(() =>
        expect(scroller?.style.getPropertyValue("--agent-composer-clearance")).toBe("120px"),
      );
    } finally {
      bounds.mockRestore();
    }
  });

  it("reserves the fixed composer while a focused session is being created", async () => {
    const user = userEvent.setup();
    const pendingSession = new Promise<AgentSessionDto>(() => {});
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_agent_sessions") return Promise.resolve([]);
      if (command === "list_provider_models") {
        return Promise.resolve({
          mode: "generation",
          selectedModel: "open-software/auto",
          modelType: "text",
          models: [],
        });
      }
      if (command === "create_agent_session") return pendingSession;
      return Promise.resolve(undefined);
    });
    const bounds = mockAgentLayoutBounds();

    try {
      const { container } = render(<AgentWorkspace />);
      await user.type(await screen.findByRole("textbox", { name: "Message Clovy" }), "New task");
      await user.click(screen.getByRole("button", { name: "Send message" }));
      const scroller = await waitFor(() => {
        const element = container.querySelector<HTMLElement>(".agent-scroll");
        expect(element).not.toBeNull();
        return element as HTMLElement;
      });

      await waitFor(() =>
        expect(scroller.style.getPropertyValue("--agent-composer-clearance")).toBe("120px"),
      );
    } finally {
      bounds.mockRestore();
    }
  });

  it("renders selected files with the canonical attachment tile and removes them", async () => {
    const user = userEvent.setup();
    const filename = "ChatGPT Image Aug 6, 2026, 01_10_40 PM.png";
    mocks.openDialog.mockResolvedValue([`/tmp/${filename}`]);

    render(<AgentWorkspace />);
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));

    const name = await screen.findByText(filename);
    const tile = name.closest(".agent-attachment-chip");
    expect(tile).not.toBeNull();
    expect(tile).toHaveAttribute("data-kind", "file");
    expect(within(tile as HTMLElement).getByText("PNG")).toBeVisible();
    expect(tile?.closest(".agent-composer-box")).toHaveAttribute("data-stacked", "true");

    await user.click(screen.getByRole("button", { name: `Remove ${filename}` }));
    expect(screen.queryByText(filename)).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toBeVisible();
  });

  it("keeps unknown attachment extensions lowercase", async () => {
    const user = userEvent.setup();
    mocks.openDialog.mockResolvedValue(["/tmp/model.BLeNd"]);

    render(<AgentWorkspace />);
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));

    const name = await screen.findByText("model.BLeNd");
    const tile = name.closest(".agent-attachment-chip");
    expect(within(tile as HTMLElement).getByText("blend")).toBeVisible();
    expect(within(tile as HTMLElement).queryByText("BLEND")).not.toBeInTheDocument();
  });

  it("stages a dropped PDF and forwards its staged path to the agent run", async () => {
    const stagedPath = "/tmp/clovy-agent-attachment-staging/drop-1/brief.pdf";
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") return Promise.resolve(stagedPath);
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);

    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();
    const pdf = new File(["%PDF-1.7"], "brief.pdf", { type: "application/pdf" });
    fireEvent.drop(form as HTMLFormElement, { dataTransfer: { files: [pdf] } });

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith(
        "stage_agent_attachment_bytes",
        expect.any(Uint8Array),
        { headers: { "x-file-name": "brief.pdf" } },
      ),
    );
    expect(await screen.findByText("brief.pdf")).toBeVisible();

    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Review this brief");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ attachments: [stagedPath] }),
      }),
    );
    expect(mocks.invoke).toHaveBeenCalledWith("discard_staged_agent_attachments", {
      request: { paths: [stagedPath] },
    });
  });

  it("requires resubmitting after a dropped file finishes staging", async () => {
    const stagedPath = "/tmp/clovy-agent-attachment-staging/drop-wait/brief.pdf";
    let resolveStage!: (path: string) => void;
    const stage = new Promise<string>((resolve) => {
      resolveStage = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") return stage;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["%PDF-1.7"], "brief.pdf")] },
    });
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Review this brief");
    fireEvent.submit(form as HTMLFormElement);

    expect(
      await screen.findByText("Wait for files to finish attaching, then send again."),
    ).toBeVisible();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "start_agent_run")).toBe(false);

    await act(async () => resolveStage(stagedPath));
    expect(await screen.findByText("brief.pdf")).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ attachments: [stagedPath] }),
      }),
    );
  });

  it("protects live queued paths before staging a new drop", async () => {
    const currentPath = "/tmp/current.pdf";
    const queuedPath = "/tmp/clovy-agent-attachment-staging/queued/queued.pdf";
    const stagedPath = "/tmp/clovy-agent-attachment-staging/new/new.pdf";
    saveQueuedAgentFollowUps({
      [newSession.id]: {
        messageId: "queued-other-session",
        prompt: "Use queued file",
        attachments: [queuedPath],
        model: "__clovy_cli_engine__:claude",
      },
    });
    mocks.openDialog.mockResolvedValue([currentPath]);
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") return Promise.resolve(stagedPath);
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["new"], "new.pdf")] },
    });

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("prune_staged_agent_attachments", {
        request: {
          protectedPaths: expect.arrayContaining([currentPath, queuedPath]),
        },
      }),
    );
    expect(await screen.findByText("new.pdf")).toBeVisible();
  });

  it("protects attachments leased by an in-flight submission before another drop", async () => {
    const submittedPath = "/tmp/clovy-agent-attachment-staging/submitted/brief.pdf";
    const secondPath = "/tmp/clovy-agent-attachment-staging/second/notes.pdf";
    let resolveSkills!: (skills: []) => void;
    const skills = new Promise<[]>((resolve) => {
      resolveSkills = resolve;
    });
    let stagedCount = 0;
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") {
        stagedCount += 1;
        return Promise.resolve(stagedCount === 1 ? submittedPath : secondPath);
      }
      if (command === "list_agent_skills") return skills;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["brief"], "brief.pdf")] },
    });
    expect(await screen.findByText("brief.pdf")).toBeVisible();
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Review this brief");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke.mock.calls.some(([command]) => command === "list_agent_skills")).toBe(
        true,
      ),
    );

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["notes"], "notes.pdf")] },
    });

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("prune_staged_agent_attachments", {
        request: { protectedPaths: expect.arrayContaining([submittedPath]) },
      }),
    );
    await act(async () => resolveSkills([]));
  });

  it("shows a retry error when another drop arrives during staging", async () => {
    let resolveStage!: (path: string) => void;
    const stage = new Promise<string>((resolve) => {
      resolveStage = resolve;
    });
    let stageCalls = 0;
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") {
        stageCalls += 1;
        return stage;
      }
      return defaultInvoke?.(command, args);
    });
    render(<AgentWorkspace initialSession={session} />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["first"], "first.pdf")] },
    });
    await waitFor(() => expect(stageCalls).toBe(1));
    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["second"], "second.pdf")] },
    });

    expect(
      await screen.findByText(
        "Wait for the current files to finish attaching, then drop these files again.",
      ),
    ).toBeVisible();
    expect(stageCalls).toBe(1);
    await act(async () => resolveStage("/tmp/clovy-agent-attachment-staging/first/first.pdf"));
  });

  it("discards a late dropped-file result after starting a new session", async () => {
    const stagedPath = "/tmp/clovy-agent-attachment-staging/drop-late/brief.pdf";
    let resolveStage!: (path: string) => void;
    const stage = new Promise<string>((resolve) => {
      resolveStage = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") return stage;
      return defaultInvoke?.(command, args);
    });
    render(<AgentWorkspace initialSession={session} />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["%PDF-1.7"], "brief.pdf")] },
    });
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.some(([command]) => command === "stage_agent_attachment_bytes"),
      ).toBe(true),
    );
    act(() => window.dispatchEvent(new CustomEvent(AGENT_NEW_SESSION_EVENT)));
    await act(async () => resolveStage(stagedPath));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("discard_staged_agent_attachments", {
        request: { paths: [stagedPath] },
      }),
    );
    expect(screen.queryByText("brief.pdf")).not.toBeInTheDocument();
  });

  it("blocks Home submission while a dropped file is still staging", async () => {
    const homeSession: AgentSessionDto = {
      ...session,
      id: "home-staging-session",
      title: "Home",
    };
    let resolveStage!: (path: string) => void;
    const stage = new Promise<string>((resolve) => {
      resolveStage = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") return stage;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["%PDF-1.7"], "brief.pdf")] },
    });
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Review this brief");
    fireEvent.submit(form as HTMLFormElement);

    expect(
      await screen.findByText("Wait for files to finish attaching, then send again."),
    ).toBeVisible();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "create_agent_session")).toBe(
      false,
    );
    await act(async () => resolveStage("/tmp/clovy-agent-attachment-staging/home/brief.pdf"));
  });

  it("restores a dropped Home attachment when initial session creation fails", async () => {
    const stagedPath = "/tmp/clovy-agent-attachment-staging/home-failed/brief.pdf";
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_sessions") return Promise.resolve([]);
      if (command === "stage_agent_attachment_bytes") return Promise.resolve(stagedPath);
      if (command === "create_agent_session") {
        return Promise.reject(new Error("Home session creation failed"));
      }
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace homeMode />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["%PDF-1.7"], "brief.pdf")] },
    });
    expect(await screen.findByText("brief.pdf")).toBeVisible();
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Review this brief");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByText("Home session creation failed")).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
      "Review this brief",
    );
    expect(screen.getByText("brief.pdf")).toBeVisible();
    expect(mocks.invoke).not.toHaveBeenCalledWith("discard_staged_agent_attachments", {
      request: { paths: [stagedPath] },
    });
  });

  it("discards live composer paths and invalidates an in-flight drop on unmount", async () => {
    const currentPath = "/tmp/clovy-agent-attachment-staging/current/brief.pdf";
    const latePath = "/tmp/clovy-agent-attachment-staging/late/notes.pdf";
    let resolveLateStage!: (path: string) => void;
    const lateStage = new Promise<string>((resolve) => {
      resolveLateStage = resolve;
    });
    let stageCount = 0;
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") {
        return stageCount++ === 0 ? Promise.resolve(currentPath) : lateStage;
      }
      return defaultInvoke?.(command, args);
    });
    const view = render(<AgentWorkspace initialSession={session} />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["%PDF-1.7"], "brief.pdf")] },
    });
    expect(await screen.findByText("brief.pdf")).toBeVisible();
    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["notes"], "notes.pdf")] },
    });
    await waitFor(() => expect(stageCount).toBe(2));

    view.unmount();
    expect(mocks.invoke).toHaveBeenCalledWith("discard_staged_agent_attachments", {
      request: { paths: [currentPath] },
    });
    await act(async () => resolveLateStage(latePath));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("discard_staged_agent_attachments", {
        request: { paths: [latePath] },
      }),
    );
  });

  it("repairs a stale Home mapping when its Clovy-owned session is missing", async () => {
    writeClovyHomeStoredSessionId("default", "missing-home-session");
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_agent_sessions") return [];
      if (command === "get_agent_session") throw new Error("Session not found");
      if (command === "list_agent_items" || command === "list_agent_artifacts") return [];
      if (command === "list_provider_models") {
        return {
          mode: "generation",
          selectedModel: "open-software/auto",
          modelType: "text",
          models: [],
        };
      }
      return undefined;
    });

    render(<AgentWorkspace homeMode initialSessionId="missing-home-session" />);

    await waitFor(() => expect(readClovyHomeStoredSessionId("default")).toBeUndefined());
    expect(screen.queryByText("Session not found")).not.toBeInTheDocument();
  });

  it.each([
    { name: "native history", model: "fast", defaultCli: "codex", expectedCli: "claude" },
    {
      name: "endpoint history",
      model: "__clovy_endpoint_engine__:local",
      defaultCli: "codex",
      expectedCli: "claude",
    },
    {
      name: "saved CLI choice",
      model: "__clovy_cli_engine__:codex",
      defaultCli: "claude",
      expectedCli: "codex",
    },
    {
      name: "Home default",
      model: "fast",
      defaultCli: "codex",
      expectedCli: "codex",
      homeMode: true,
    },
    {
      name: "Codex fallback",
      model: "fast",
      defaultCli: "codex",
      expectedCli: "codex",
      claudeInstalled: false,
    },
  ])("submits the engine displayed for $name", async ({
    model,
    defaultCli,
    expectedCli,
    homeMode,
    claudeInstalled = true,
  }) => {
    const user = userEvent.setup();
    const storedSession = { ...session, model };
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "get_agent_session") return Promise.resolve(storedSession);
      if (command === "chat_default_engine")
        return Promise.resolve({ modelId: `__clovy_cli_engine__:${defaultCli}`, cli: defaultCli });
      if (command === "chat_engine_catalog")
        return Promise.resolve({
          clis: [
            {
              id: "claude",
              name: "Claude Code",
              installed: claudeInstalled,
              reason: null,
              modelId: "__clovy_cli_engine__:claude",
            },
            {
              id: "codex",
              name: "Codex",
              installed: true,
              reason: null,
              modelId: "__clovy_cli_engine__:codex",
            },
          ],
        });
      return defaultInvoke?.(command, args);
    });
    render(<AgentWorkspace initialSession={storedSession} homeMode={homeMode} />);
    await screen.findByText("Earlier answer");
    const label = expectedCli === "claude" ? "Claude Code" : "Codex";
    if (homeMode)
      expect(screen.queryByRole("button", { name: /^Chat engine:/ })).not.toBeInTheDocument();
    else
      expect(
        await screen.findByRole("button", { name: `Chat engine: ${label}` }),
      ).toBeInTheDocument();
    await user.type(
      screen.getByRole("textbox", { name: "Message Clovy" }),
      "Continue this conversation",
    );
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ model: `__clovy_cli_engine__:${expectedCli}` }),
      }),
    );
  });

  it("hydrates history, shows an optimistic turn, and cancels", async () => {
    const user = userEvent.setup();
    const { container } = render(<AgentWorkspace initialSession={session} />);

    expect(await screen.findByText("Earlier answer")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sandboxed" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Chat engine: Claude Code" })).toBeEnabled();
    expect(container.querySelector(".agent-scroll .agent-main > .agent-composer")).not.toBeNull();
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.click(composer);
    await user.type(composer, "New request");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByText("New request")).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          model: "__clovy_cli_engine__:claude",
          safetyMode: "sandboxed",
          enabledSkillIds: ["notes"],
        }),
      }),
    );
    expect(screen.queryByRole("button", { name: /^Model:/ })).not.toBeInTheDocument();
    expect(mocks.invoke.mock.calls.some(([command]) => command === "list_provider_models")).toBe(
      false,
    );

    fireEvent.click(screen.getByRole("button", { name: "Stop Clovy" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("cancel_agent_run", { runId: "run-1" }),
    );

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-cancelled",
          sessionId: session.id,
          runId: "run-1",
          sequence: 3,
          method: "run.cancelled",
          data: { completedAt: "2026-07-22T12:01:00Z" },
        },
      });
    });

    const nextRunPicker = await screen.findByRole("button", { name: "Chat engine: Claude Code" });
    expect(nextRunPicker).toBeEnabled();
    const nextComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(nextComposer, "Use the staged model");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          prompt: "Use the staged model",
          model: "__clovy_cli_engine__:claude",
        }),
      }),
    );
  });

  it("shows reported tokens and per-tool usage without a model-based credit estimate", async () => {
    const user = userEvent.setup();
    render(
      <div className="app-shell">
        <AgentWorkspace initialSession={session} />
      </div>,
    );
    await screen.findByText("Earlier answer");
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Use a tool");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", expect.anything()),
    );
    await screen.findByRole("button", { name: "Stop Clovy" });

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "tool-start",
          sessionId: session.id,
          runId: "run-1",
          sequence: 2,
          method: "tool.started",
          data: {
            itemSequence: 2,
            itemId: "tool-item-1",
            callId: "call-1",
            name: "read_file",
            arguments: { path: "notes.md" },
            createdAt: "2026-07-25T12:00:01Z",
          },
        },
      });
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "usage",
          sessionId: session.id,
          runId: "run-1",
          sequence: 3,
          method: "usage.updated",
          data: {
            inputTokens: 10_000,
            outputTokens: 2_000,
            totalTokens: 12_000,
            provider: "phala",
            privacyLevel: "tee",
            endpoint: "phala-glm-5.2",
          },
        },
      });
    });

    await user.click(screen.getByRole("button", { name: "Session actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Usage" }));
    const usagePanel = screen.getByRole("dialog", { name: "Usage" });
    const usageOverlay = usagePanel.closest(".agent-usage-overlay");
    expect(usageOverlay).not.toBeNull();
    expect(usageOverlay?.parentElement).toBe(document.querySelector(".app-shell"));
    expect(usagePanel).toHaveTextContent("10,000");
    expect(usagePanel).not.toHaveTextContent("28 credits (about $0.0280)");
    expect(usagePanel).toHaveTextContent("read_file");
    expect(usagePanel).toHaveTextContent("1 call");
    expect(usagePanel).toHaveTextContent("phala");
    expect(usagePanel).toHaveTextContent("tee");
    expect(usagePanel).toHaveTextContent("phala-glm-5.2");

    await user.click(usagePanel);
    expect(screen.getByRole("dialog", { name: "Usage" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close usage" }));
    expect(screen.queryByRole("dialog", { name: "Usage" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Session actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Usage" }));
    const reopenedOverlay = screen
      .getByRole("dialog", { name: "Usage" })
      .closest(".agent-usage-overlay");
    expect(reopenedOverlay).not.toBeNull();
    await user.click(reopenedOverlay as HTMLElement);
    expect(screen.queryByRole("dialog", { name: "Usage" })).not.toBeInTheDocument();
  });

  it("traps focus in Usage and restores it to Session actions after Escape", async () => {
    const user = userEvent.setup();
    render(
      <div className="app-shell">
        <AgentWorkspace initialSession={session} />
      </div>,
    );
    await screen.findByText("Earlier answer");

    const sessionActions = screen.getByRole("button", { name: "Session actions" });
    await user.click(sessionActions);
    await user.click(screen.getByRole("menuitem", { name: "Usage" }));

    const usageDialog = screen.getByRole("dialog", { name: "Usage" });
    const closeUsage = within(usageDialog).getByRole("button", { name: "Close usage" });
    expect(usageDialog).toHaveAttribute("aria-modal", "true");
    expect(closeUsage).toHaveFocus();

    await user.tab();
    expect(closeUsage).toHaveFocus();
    await user.tab({ shift: true });
    expect(closeUsage).toHaveFocus();

    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: "Usage" })).not.toBeInTheDocument();
    expect(sessionActions).toHaveFocus();
  });

  it("shows route-only persisted usage without crashing", async () => {
    const autoSession = { ...session, model: "__june_auto_generation__:20" };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_agent_sessions") return [autoSession];
      if (command === "get_agent_session") return autoSession;
      if (command === "list_agent_items") {
        return [
          {
            id: "message-route-only",
            sessionId: session.id,
            sequence: 1,
            createdAt: session.createdAt,
            kind: "message",
            role: "assistant",
            text: "Earlier answer",
            status: "complete",
          },
        ];
      }
      if (command === "list_agent_artifacts") return [];
      if (command === "get_latest_agent_run") {
        return {
          id: "run-route-only",
          sessionId: session.id,
          status: "completed",
          model: "__june_auto_generation__:20",
          usage: {
            provider: "qa-fixture",
            privacyLevel: "isolated",
            endpoint: "localhost",
          },
        };
      }
      return undefined;
    });
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={autoSession} />);
    await screen.findByText("Earlier answer");

    await user.click(screen.getByRole("button", { name: "Session actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Usage" }));

    const usagePanel = screen.getByRole("dialog", { name: "Usage" });
    expect(usagePanel).toHaveTextContent("qa-fixture");
    expect(usagePanel).toHaveTextContent("isolated");
    expect(usagePanel).toHaveTextContent("localhost");
    expect(usagePanel).toHaveTextContent("Token counts were not reported for this request.");
  });

  it("keeps a queued follow-up owned by its running session across navigation", async () => {
    const defaultInvoke = mocks.invoke.getMockImplementation();
    let resolveRevisitItems: ((items: unknown[]) => void) | undefined;
    const revisitItems = new Promise<unknown[]>((resolve) => {
      resolveRevisitItems = resolve;
    });
    let sessionAItemsCalls = 0;
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      const sessionId = (args as { sessionId?: string } | undefined)?.sessionId;
      if (command === "list_agent_sessions") return Promise.resolve([session, newSession]);
      if (command === "list_agent_items" && sessionId === session.id) {
        sessionAItemsCalls += 1;
        if (sessionAItemsCalls > 1) return revisitItems;
      }
      if (command === "get_agent_session" && sessionId === newSession.id) {
        return Promise.resolve(newSession);
      }
      if (command === "list_agent_items" && sessionId === newSession.id) {
        return Promise.resolve([]);
      }
      return defaultInvoke?.(command, args);
    });

    const user = userEvent.setup();
    mocks.openDialog.mockResolvedValue(["/tmp/follow-up.pdf"]);
    const { rerender } = render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");
    let composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Start in session A");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await screen.findByRole("button", { name: "Stop Clovy" });

    composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    expect(await screen.findByText("follow-up.pdf")).toBeVisible();
    composer.textContent = "Only send this in session A";
    fireEvent.input(composer);
    await user.click(await screen.findByRole("button", { name: "Queue next turn" }));
    expect(
      await screen.findByText("Queued follow-up. 1 attachment queued for next turn"),
    ).toBeVisible();

    rerender(<AgentWorkspace initialSession={newSession} />);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Stop Clovy" })).toBeNull());
    expect(
      screen.queryByText("Queued follow-up. 1 attachment queued for next turn"),
    ).not.toBeInTheDocument();

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-session-a-completed",
          sessionId: session.id,
          runId: "run-1",
          sequence: 3,
          method: "run.completed",
          data: { completedAt: "2026-07-22T12:01:00Z" },
        },
      });
    });

    await waitFor(() => {
      const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
      expect(starts).toHaveLength(1);
    });
    expect(screen.queryByText("Only send this in session A")).not.toBeInTheDocument();

    rerender(<AgentWorkspace initialSession={session} />);
    await act(async () => Promise.resolve());
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run"),
    ).toHaveLength(1);
    await act(async () =>
      resolveRevisitItems?.([
        {
          id: "message-1",
          sessionId: session.id,
          sequence: 1,
          createdAt: session.createdAt,
          kind: "message",
          role: "assistant",
          text: "Earlier answer",
          status: "complete",
        },
      ]),
    );
    await waitFor(() => {
      const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
      expect(starts).toHaveLength(2);
      expect(starts[1]?.[1]).toMatchObject({
        request: expect.objectContaining({
          sessionId: session.id,
          prompt: "Only send this in session A",
          model: "__clovy_cli_engine__:claude",
          attachments: ["/tmp/follow-up.pdf"],
        }),
      });
    });
  });

  it("restores an attachment follow-up after the workspace remounts", async () => {
    saveQueuedAgentFollowUps({
      [session.id]: {
        messageId: "restored-message",
        prompt: "Resume with this file",
        attachments: ["/tmp/restored.pdf"],
        model: "__clovy_cli_engine__:claude",
      },
    });

    render(<AgentWorkspace initialSession={session} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          sessionId: session.id,
          prompt: "Resume with this file",
          attachments: ["/tmp/restored.pdf"],
          model: "__clovy_cli_engine__:claude",
        }),
      }),
    );
    expect(await screen.findByText("Resume with this file")).toBeVisible();
  });

  it("keeps a restored follow-up when its run start fails after navigation", async () => {
    saveQueuedAgentFollowUps({
      [session.id]: {
        messageId: "retry-after-navigation",
        prompt: "Retry this in session A",
        attachments: ["/tmp/retry.pdf"],
        model: "__clovy_cli_engine__:claude",
      },
    });
    let rejectFirstStart: ((error: Error) => void) | undefined;
    const firstStart = new Promise((_, reject) => {
      rejectFirstStart = reject;
    });
    let startCount = 0;
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      const sessionId = (args as { sessionId?: string } | undefined)?.sessionId;
      if (command === "list_agent_sessions") return Promise.resolve([session, newSession]);
      if (command === "get_agent_session" && sessionId === newSession.id) {
        return Promise.resolve(newSession);
      }
      if (command === "list_agent_items" && sessionId === newSession.id) {
        return Promise.resolve([]);
      }
      if (command === "start_agent_run" && startCount++ === 0) return firstStart;
      return defaultInvoke?.(command, args);
    });

    const { rerender } = render(<AgentWorkspace initialSession={session} />);
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run"),
      ).toHaveLength(1),
    );

    rerender(<AgentWorkspace initialSession={newSession} />);
    await waitFor(() => expect(screen.queryByText("Retry this in session A")).toBeNull());
    await act(async () => rejectFirstStart?.(new Error("runtime unavailable")));

    rerender(<AgentWorkspace initialSession={session} />);
    expect(await screen.findByRole("button", { name: "Retry queued follow-up" })).toBeVisible();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run"),
    ).toHaveLength(1);

    await userEvent.setup().click(screen.getByRole("button", { name: "Retry queued follow-up" }));
    await waitFor(() => {
      const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
      expect(starts).toHaveLength(2);
      expect(starts[1]?.[1]).toMatchObject({
        request: expect.objectContaining({
          sessionId: session.id,
          prompt: "Retry this in session A",
          attachments: ["/tmp/retry.pdf"],
        }),
      });
    });
  });

  it("hydrates a public steering item and submits only its pending attachments", async () => {
    saveQueuedAgentFollowUps({
      [session.id]: {
        messageId: "already-consumed",
        prompt: "Use this restored brief",
        attachments: ["/tmp/restored-brief.pdf"],
        model: "__clovy_cli_engine__:claude",
        delivery: "follow_up",
        steering: "accepted",
      },
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_items") {
        return Promise.resolve([
          {
            id: "steering:already-consumed",
            sessionId: session.id,
            runId: "run-consumed",
            sequence: 1,
            createdAt: session.createdAt,
            kind: "steering",
            text: "Use this restored brief",
          },
        ]);
      }
      return defaultInvoke?.(command, args);
    });

    render(<AgentWorkspace initialSession={session} />);
    expect(await screen.findByText("Steering: Use this restored brief")).toBeVisible();
    await waitFor(() => {
      const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
      expect(starts).toHaveLength(1);
      expect(starts[0]?.[1]).toMatchObject({
        request: expect.objectContaining({
          prompt: `Use this restored brief\n\n${ATTACHMENT_FOLLOW_UP_NOTE}`,
          attachments: ["/tmp/restored-brief.pdf"],
        }),
      });
    });
  });

  it.each([
    "loading",
    "failed",
  ])("uses the installed default for a historical session while the engine catalog is %s", async (catalogState) => {
    const base = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "chat_engine_catalog") {
        return catalogState === "loading"
          ? new Promise(() => {})
          : Promise.reject(new Error("catalog unavailable"));
      }
      if (command === "chat_default_engine") {
        return Promise.resolve({ modelId: "__clovy_cli_engine__:codex", cli: "codex" });
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Continue history");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ model: "__clovy_cli_engine__:codex" }),
      }),
    );
  });

  it("preserves an explicit CLI choice when the engine catalog fails", async () => {
    const base = mocks.invoke.getMockImplementation();
    const selected = { ...session, model: "__clovy_cli_engine__:claude" };
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "chat_engine_catalog")
        return Promise.reject(new Error("catalog unavailable"));
      if (command === "get_agent_session") return Promise.resolve(selected);
      if (command === "chat_default_engine") {
        return Promise.resolve({ modelId: "__clovy_cli_engine__:codex", cli: "codex" });
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={selected} />);
    await screen.findByText("Earlier answer");
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Keep my engine");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ model: selected.model }),
      }),
    );
  });

  it("does not let a slow global catalog overwrite a restored session model", async () => {
    let resolveCatalog:
      | ((value: {
          mode: "generation";
          selectedModel: string;
          modelType: string;
          models: Array<{
            provider: string;
            id: string;
            name: string;
            modelType: string;
            traits: string[];
            capabilities: string[];
          }>;
        }) => void)
      | undefined;
    const pendingCatalog = new Promise<{
      mode: "generation";
      selectedModel: string;
      modelType: string;
      models: Array<{
        provider: string;
        id: string;
        name: string;
        modelType: string;
        traits: string[];
        capabilities: string[];
      }>;
    }>((resolve) => {
      resolveCatalog = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_provider_models") return pendingCatalog;
      return defaultInvoke?.(command, args);
    });

    render(<AgentWorkspace initialSessionId={session.id} />);
    await screen.findByText("Earlier answer");

    await act(async () =>
      resolveCatalog?.({
        mode: "generation",
        selectedModel: "open-software/auto",
        modelType: "text",
        models: [
          {
            provider: "june",
            id: "open-software/auto",
            name: "Auto",
            modelType: "text",
            traits: [],
            capabilities: [],
          },
          {
            provider: "june",
            id: "fast",
            name: "Fast",
            modelType: "text",
            traits: [],
            capabilities: ["tool-calling"],
          },
        ],
      }),
    );
    expect(screen.getByRole("button", { name: "Chat engine: Claude Code" })).toBeEnabled();
  });

  it("resolves approval interruptions against their originating run", async () => {
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    act(() => {
      mocks.runtimeListener?.({ payload: linearApprovalEvent() });
    });

    fireEvent.click(await screen.findByRole("button", { name: "Approve" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("resolve_agent_interruption", {
        request: {
          runId: "run-linear",
          interruptionId: "functions.mcp_linear_save_issue:0",
          resolution: { kind: "approval", choice: "once" },
        },
      }),
    );
    expect(await screen.findByText("Approved once")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Approve" })).not.toBeInTheDocument();
  });

  it.each([
    ["en", "Deny", "Denied"],
    ["pt-BR", "Negar", "Negado"],
  ] as const)("shows concrete untrusted browser targets before the real decision callback in %s", async (locale, denyLabel, deniedLabel) => {
    applyInterfaceLocale(locale);
    try {
      const preview = (label: string) =>
        `click Untrusted page element label: ${JSON.stringify(label)}\nElement tag: "button"; role: "button"; operation: click\nOrigin: https://mail.example.com`;
      const event = (label: string, id: string, sequence: number): AgentRuntimeEvent => ({
        protocolVersion: 1,
        eventId: `browser-event-${id}`,
        sessionId: session.id,
        runId: "run-browser",
        sequence,
        method: "interruption.requested",
        data: {
          itemSequence: sequence,
          itemId: `browser-item-${id}`,
          interruption: {
            id,
            kind: "approval",
            engine: "cli",
            sessionId: session.id,
            runId: "run-browser",
            status: "pending",
            createdAt: "2026-11-01T00:00:00Z",
            toolName: "click",
            title: "Use the browser",
            description: "Clovy will perform one consequential action on the inspected page.",
            command: preview(label),
            allowAlways: false,
          },
        },
      });
      render(<AgentWorkspace initialSession={session} />);
      await screen.findByText("Earlier answer");
      act(() => {
        mocks.runtimeListener?.({ payload: event("Delete draft", "delete-draft", 2) });
        mocks.runtimeListener?.({ payload: event("Delete account", "delete-account", 3) });
      });
      const draftPreview = await screen.findByText(preview("Delete draft"), {
        exact: true,
        normalizer: (text) => text,
      });
      const accountPreview = await screen.findByText(preview("Delete account"), {
        exact: true,
        normalizer: (text) => text,
      });
      // Concrete targets are visible while both choices are live, without opening Details.
      expect(draftPreview).toBeVisible();
      expect(accountPreview).toBeVisible();
      expect(draftPreview.tagName).toBe("PRE");
      expect(accountPreview.tagName).toBe("PRE");
      const draftCard = draftPreview.closest("article");
      const accountCard = accountPreview.closest("article");
      if (!draftCard || !accountCard) throw new Error("Missing central browser approval cards");
      expect(draftCard).not.toBe(accountCard);
      expect(within(draftCard).getByRole("button", { name: denyLabel })).toBeEnabled();
      expect(within(accountCard).getByRole("button", { name: denyLabel })).toBeEnabled();
      await userEvent.click(within(accountCard).getByRole("button", { name: denyLabel }));
      await waitFor(() =>
        expect(mocks.invoke).toHaveBeenCalledWith("resolve_agent_interruption", {
          request: {
            runId: "run-browser",
            interruptionId: "delete-account",
            resolution: { kind: "approval", choice: "deny" },
          },
        }),
      );
      expect(await screen.findByText(deniedLabel)).toBeInTheDocument();
      expect(within(draftCard).getByRole("button", { name: denyLabel })).toBeEnabled();
      const decisions = mocks.invoke.mock.calls.filter(
        ([command]) => command === "resolve_agent_interruption",
      );
      expect(decisions).toHaveLength(1);
      expect(mocks.invoke.mock.calls.some(([command]) => command === "start_agent_run")).toBe(
        false,
      );
    } finally {
      act(() => applyInterfaceLocale("en"));
    }
  });

  it("keeps an approval retryable when interruption resolution fails", async () => {
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "resolve_agent_interruption") {
        return Promise.reject(new Error("Resume failed"));
      }
      return defaultInvoke?.(command, args);
    });
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    act(() => {
      mocks.runtimeListener?.({ payload: linearApprovalEvent() });
    });

    fireEvent.click(await screen.findByRole("button", { name: "Approve" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Resume failed");
    await waitFor(() => expect(screen.getByRole("button", { name: "Approve" })).toBeEnabled());
    expect(screen.queryByText("Approved once")).not.toBeInTheDocument();
  });

  it("updates the desktop picker when a companion stages a session model", async () => {
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");
    expect(screen.getByRole("button", { name: "Chat engine: Claude Code" })).toBeEnabled();

    act(() => rememberSessionModel(session.id, "__clovy_cli_engine__:codex"));

    expect(screen.getByRole("button", { name: "Chat engine: Codex" })).toBeEnabled();
  });

  it("resolves clarification interruptions through the typed host command", async () => {
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-2",
          sessionId: session.id,
          runId: "run-2",
          sequence: 2,
          method: "interruption.requested",
          data: {
            itemSequence: 2,
            itemId: "clarify-item",
            interruption: {
              id: "clarify-1",
              kind: "clarification",
              sessionId: session.id,
              runId: "run-2",
              status: "pending",
              createdAt: "2026-07-22T12:00:02Z",
              question: "Which project?",
              choices: ["Clovy", "Platform"],
            },
          },
        },
      });
    });

    fireEvent.click(await screen.findByRole("button", { name: /^1\s*Clovy/ }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("resolve_agent_interruption", {
        request: {
          runId: "run-2",
          interruptionId: "clarify-1",
          resolution: { kind: "clarification", answer: "Clovy" },
        },
      }),
    );
  });

  it("lets a user stop a waiting clarification and continue with a new message", async () => {
    const user = userEvent.setup();
    const waitingSession: AgentSessionDto = {
      ...session,
      status: "waiting_for_user",
    };
    let cancelled = false;
    let finishCancellation: (() => void) | undefined;
    const cancellation = new Promise<void>((resolve) => {
      finishCancellation = resolve;
    });
    const interruption = () => ({
      id: "clarify-item",
      sessionId: session.id,
      runId: "run-waiting",
      sequence: 2,
      createdAt: "2026-07-22T12:00:02Z",
      kind: "interruption" as const,
      interruption: {
        id: "clarify-1",
        kind: "clarification" as const,
        sessionId: session.id,
        runId: "run-waiting",
        status: cancelled ? ("expired" as const) : ("pending" as const),
        createdAt: "2026-07-22T12:00:02Z",
        question: "Which project?",
        choices: [],
      },
    });
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_agent_sessions") {
        return Promise.resolve([
          cancelled ? { ...waitingSession, status: "idle" } : waitingSession,
        ]);
      }
      if (command === "get_agent_session") {
        return Promise.resolve(cancelled ? { ...waitingSession, status: "idle" } : waitingSession);
      }
      if (command === "get_latest_agent_run") {
        return Promise.resolve({
          id: "run-waiting",
          sessionId: session.id,
          status: cancelled ? "cancelled" : "waiting_for_user",
          model: "__june_auto_generation__:20",
          startedAt: "2026-07-22T12:00:01Z",
          ...(cancelled ? { completedAt: "2026-07-22T12:00:03Z" } : {}),
        });
      }
      if (command === "list_agent_items") return Promise.resolve([interruption()]);
      if (command === "list_agent_artifacts" || command === "list_agent_skills") {
        return Promise.resolve([]);
      }
      if (command === "list_provider_models") {
        return Promise.resolve({ mode: "generation", models: [] });
      }
      if (command === "provider_model_settings") {
        return Promise.resolve({
          settings: {},
          effectiveSettings: {
            providerApiKeysConfigured: {
              groq: false,
              openrouter: false,
              fal: false,
              tavily: false,
            },
          },
        });
      }
      if (command === "cancel_agent_run") {
        return cancellation.then(() => {
          cancelled = true;
        });
      }
      return Promise.resolve(undefined);
    });

    render(<AgentWorkspace initialSession={waitingSession} />);

    expect(await screen.findByText("Which project?")).toBeVisible();
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Continue from here");
    expect(screen.queryByRole("button", { name: "Send message" })).not.toBeInTheDocument();
    const stopButton = screen.getByRole("button", { name: "Stop Clovy" });
    await user.click(stopButton);
    expect(stopButton).toBeDisabled();
    fireEvent.click(stopButton);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "cancel_agent_run"),
    ).toHaveLength(1);
    finishCancellation?.();

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("cancel_agent_run", { runId: "run-waiting" }),
    );
    expect(await screen.findByText("Skipped")).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
      "Continue from here",
    );
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
  });

  it("presents retryable runtime failures as a retry action and resumes through the typed host command", async () => {
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "list_agent_sessions") return Promise.resolve([session]);
      if (command === "get_agent_session") return Promise.resolve(session);
      if (command === "list_agent_items") {
        return Promise.resolve([
          {
            id: "message-1",
            sessionId: session.id,
            runId: "run-failed",
            sequence: 1,
            createdAt: session.createdAt,
            kind: "message",
            role: "user",
            text: "Retry this",
            status: "complete",
          },
          {
            id: "error-1",
            sessionId: session.id,
            runId: "run-failed",
            sequence: 2,
            createdAt: session.updatedAt,
            kind: "error",
            message: "upstream_provider_failed",
            retryable: true,
          },
        ]);
      }
      if (command === "list_agent_artifacts") return Promise.resolve([]);
      if (command === "list_agent_skills") return Promise.resolve([]);
      if (command === "list_provider_models") {
        return Promise.resolve({ mode: "generation", models: [] });
      }
      if (command === "retry_agent_run") {
        return Promise.resolve({
          id: "run-retry",
          sessionId: session.id,
          status: "running",
          model: "__clovy_cli_engine__:claude",
        });
      }
      return Promise.resolve(undefined);
    });

    render(<AgentWorkspace initialSession={session} />);

    expect(
      await screen.findByText("The model service could not finish this request."),
    ).toBeVisible();
    expect(screen.queryByText("upstream_provider_failed")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("retry_agent_run", { runId: "run-failed" }),
    );
  });

  it("resets an open conversation when a new session is requested", async () => {
    const user = userEvent.setup();
    const onSessionSelected = vi.fn();
    const { container } = render(
      <AgentWorkspace initialSession={session} onSessionSelected={onSessionSelected} />,
    );
    await screen.findByText("Earlier answer");

    act(() => {
      markAgentNewSessionPending();
      window.dispatchEvent(new CustomEvent(AGENT_NEW_SESSION_EVENT));
    });

    expect(await screen.findByRole("heading", { level: 2 })).toBeVisible();
    expect(screen.queryByText("Earlier answer")).not.toBeInTheDocument();
    expect(onSessionSelected).toHaveBeenLastCalledWith(undefined);
    expect(
      container.querySelector(".agent-workspace > .agent-main[data-hero='true']"),
    ).not.toBeNull();
    expect(container.querySelector(".agent-scroll")).toBeNull();

    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    expect(screen.getByRole("menuitem", { name: "Attach files" })).toBeVisible();
    expect(screen.getByRole("menuitem", { name: "Reference a note" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));

    await user.click(screen.getByRole("button", { name: "Sandboxed" }));
    await user.click(screen.getByRole("menuitemradio", { name: /Unrestricted/ }));
    expect(screen.getByRole("dialog", { name: "Turn on unrestricted?" })).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Cancel" }));

    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.click(composer);
    await user.type(composer, "Fresh request");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", {
        request: expect.objectContaining({ title: "Fresh request" }),
      }),
    );
    expect(onSessionSelected).toHaveBeenLastCalledWith(newSession);
  });

  it("shows the first message and thinking before fresh session creation finishes", async () => {
    const user = userEvent.setup();
    let resolveCreate: ((value: AgentSessionDto) => void) | undefined;
    const pendingCreate = new Promise<AgentSessionDto>((resolve) => {
      resolveCreate = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "create_agent_session") return pendingCreate;
      if (
        command === "list_agent_items" &&
        (args as { sessionId?: string } | undefined)?.sessionId === newSession.id
      ) {
        return Promise.resolve([]);
      }
      return defaultInvoke?.(command, args);
    });

    const onSessionSelected = vi.fn();
    const { container, rerender } = render(
      <AgentWorkspace onSessionSelected={onSessionSelected} />,
    );
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Fresh request");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(container.querySelector(".agent-user-turn")).toHaveTextContent("Fresh request"),
    );
    expect(screen.getByText("Thinking…")).toBeVisible();
    expect(container.querySelector(".agent-workspace[data-hero='true']")).toBeNull();
    expect(screen.queryByRole("button", { name: "Session actions" })).not.toBeInTheDocument();
    const followUpComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(followUpComposer, "Follow-up draft");

    await act(async () => resolveCreate?.(newSession));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", expect.anything()),
    );
    expect(screen.getByRole("button", { name: "Session actions" })).toBeVisible();
    expect(followUpComposer).toHaveTextContent("Follow-up draft");
    expect(readAgentSessionDraft(newSession.id)).toBe("Follow-up draft");
    rerender(<AgentWorkspace initialSession={newSession} onSessionSelected={onSessionSelected} />);
    await waitFor(() =>
      expect(container.querySelector(".agent-user-turn")).toHaveTextContent("Fresh request"),
    );
  });

  it("keeps a failed first submission recoverable without replacing a newer draft", async () => {
    const user = userEvent.setup();
    let rejectCreate: ((error: Error) => void) | undefined;
    const firstCreate = new Promise<AgentSessionDto>((_, reject) => {
      rejectCreate = reject;
    });
    let createCount = 0;
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "create_agent_session" && createCount++ === 0) return firstCreate;
      return defaultInvoke?.(command, args);
    });
    mocks.openDialog
      .mockResolvedValueOnce(["/tmp/first.pdf"])
      .mockResolvedValueOnce(["/tmp/later.pdf"]);

    render(<AgentWorkspace />);
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "First submission");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await screen.findByText("Thinking…");

    const laterComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(laterComposer, "Later draft");
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    await act(async () => rejectCreate?.(new Error("session creation failed")));

    expect(await screen.findByText("Unsent message")).toBeVisible();
    expect(screen.getByText("First submission")).toBeVisible();
    expect(laterComposer).toHaveTextContent("Later draft");
    expect(screen.getByText("later.pdf")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Retry unsent message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          prompt: "First submission",
          attachments: ["/tmp/first.pdf"],
        }),
      }),
    );
    expect(laterComposer).toHaveTextContent("Later draft");
    expect(screen.getByText("later.pdf")).toBeVisible();
    expect(screen.queryByText("Unsent message")).not.toBeInTheDocument();
  });

  it("restores a branch draft after a first session run fails and is retried", async () => {
    const user = userEvent.setup();
    let rejectFirstStart: ((error: Error) => void) | undefined;
    const firstStart = new Promise((_, reject) => {
      rejectFirstStart = reject;
    });
    let startCount = 0;
    const branchSession: AgentSessionDto = {
      ...newSession,
      id: "session-branch",
      title: "Branch",
      workspacePath: "/tmp/session-branch",
    };
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "start_agent_run" && startCount++ === 0) return firstStart;
      if (command === "start_agent_run") {
        return Promise.resolve({
          id: "run-retry",
          sessionId: newSession.id,
          status: "running",
          model: "auto",
        });
      }
      if (command === "branch_agent_session") return Promise.resolve(branchSession);
      if (
        command === "get_agent_session" &&
        [newSession.id, branchSession.id].includes(
          (args as { sessionId?: string } | undefined)?.sessionId ?? "",
        )
      ) {
        return Promise.resolve(
          (args as { sessionId?: string }).sessionId === branchSession.id
            ? branchSession
            : newSession,
        );
      }
      if (
        command === "list_agent_items" &&
        [newSession.id, branchSession.id].includes(
          (args as { sessionId?: string } | undefined)?.sessionId ?? "",
        )
      ) {
        return Promise.resolve([]);
      }
      return defaultInvoke?.(command, args);
    });

    render(<AgentWorkspace />);
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "First submission");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", expect.anything()),
    );
    const followUpComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(followUpComposer, "Later draft");
    await waitFor(() => expect(followUpComposer).toHaveTextContent("Later draft"));
    await waitFor(() => expect(readAgentSessionDraft(newSession.id)).toBe("Later draft"));
    await act(async () => rejectFirstStart?.(new Error("first run failed")));

    await user.click(await screen.findByRole("button", { name: "Retry unsent message" }));
    await waitFor(() => {
      const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
      expect(starts).toHaveLength(2);
    });
    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "retry-message-completed",
          sessionId: newSession.id,
          runId: "run-retry",
          sequence: 2,
          method: "message.completed",
          data: {
            itemSequence: 2,
            itemId: "retry-answer",
            role: "assistant",
            text: "Retry answer",
            createdAt: "2026-07-22T12:01:00Z",
          },
        },
      });
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "retry-run-completed",
          sessionId: newSession.id,
          runId: "run-retry",
          sequence: 3,
          method: "run.completed",
          data: { completedAt: "2026-07-22T12:02:00Z" },
        },
      });
    });

    writeAgentSessionDraft(branchSession.id, "Branch draft");
    await screen.findAllByRole("button", { name: "Branch from here" });
    fireEvent.click(screen.getAllByRole("button", { name: "Branch from here" }).at(-1) as Element);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("branch_agent_session", expect.anything()),
    );
    await screen.findByText("Branch");
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveTextContent("Branch draft"));
    expect(readAgentSessionDraft(branchSession.id)).toBe("Branch draft");
  });

  it("discards staged attachments when an unsent message is explicitly discarded", async () => {
    const stagedPath = "/tmp/clovy-agent-attachment-staging/recoverable/brief.pdf";
    let rejectCreate!: (error: Error) => void;
    const create = new Promise<AgentSessionDto>((_, reject) => {
      rejectCreate = reject;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "stage_agent_attachment_bytes") return Promise.resolve(stagedPath);
      if (command === "create_agent_session") return create;
      return defaultInvoke?.(command, args);
    });
    mocks.openDialog.mockResolvedValue(["/tmp/later.pdf"]);
    const user = userEvent.setup();
    render(<AgentWorkspace />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["%PDF-1.7"], "brief.pdf")] },
    });
    expect(await screen.findByText("brief.pdf")).toBeVisible();
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Review this brief");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await user.type(
      screen.getByRole("textbox", { name: "Message Clovy" }),
      "Keep this newer draft",
    );
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    await act(async () => rejectCreate(new Error("session creation failed")));
    expect(await screen.findByText("Unsent message")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Discard unsent message" }));
    expect(mocks.invoke).toHaveBeenCalledWith("discard_staged_agent_attachments", {
      request: { paths: [stagedPath] },
    });
    expect(screen.queryByText("Unsent message")).not.toBeInTheDocument();
  });

  it("does not take over the workspace when a delayed fresh session finishes after navigation", async () => {
    const user = userEvent.setup();
    let resolveCreate: ((value: AgentSessionDto) => void) | undefined;
    const pendingCreate = new Promise<AgentSessionDto>((resolve) => {
      resolveCreate = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "create_agent_session") return pendingCreate;
      return defaultInvoke?.(command, args);
    });

    const onSessionSelected = vi.fn();
    const { container, rerender } = render(
      <AgentWorkspace onSessionSelected={onSessionSelected} />,
    );
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Fresh request");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(container.querySelector(".agent-user-turn")).toHaveTextContent("Fresh request"),
    );

    rerender(<AgentWorkspace initialSession={session} onSessionSelected={onSessionSelected} />);
    expect(await screen.findByText("Earlier answer")).toBeVisible();
    await waitFor(() => expect(screen.queryByText("Fresh request")).not.toBeInTheDocument());
    const activeComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(activeComposer, "Keep this draft");
    await waitFor(() => expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled());

    await act(async () => resolveCreate?.(newSession));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ sessionId: newSession.id }),
      }),
    );
    expect(screen.getByText("Earlier answer")).toBeVisible();
    expect(screen.queryByText("Fresh request")).not.toBeInTheDocument();
    expect(activeComposer).toHaveTextContent("Keep this draft");
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
    expect(onSessionSelected).not.toHaveBeenCalledWith(newSession);
  });

  it("uses Claude Code for a fresh workspace when the default command is unavailable", async () => {
    setCurrentDataPartitionName("private");
    const user = userEvent.setup();
    render(<AgentWorkspace />);

    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.click(composer);
    await user.type(composer, "Fresh request");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", {
        request: expect.objectContaining({
          model: "__clovy_cli_engine__:claude",
          title: "Fresh request",
          profile: "private",
        }),
      }),
    );
  });

  it("starts a fresh workspace session on the default CLI engine", async () => {
    const engineModel = "__clovy_cli_engine__:claude";
    const base = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "chat_default_engine") {
        return Promise.resolve({ modelId: engineModel, cli: "claude", legacyFallback: false });
      }
      // The remote catalog is unreachable: the default needs no network.
      if (command === "list_provider_models") return Promise.reject(new Error("offline"));
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Chat engine: Claude Code" })).toBeVisible(),
    );
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.click(composer);
    await user.type(composer, "Fresh request");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", {
        request: expect.objectContaining({ model: engineModel, title: "Fresh request" }),
      }),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ model: engineModel }),
      }),
    );
  });

  describe("streamed tokens", () => {
    const event = (sessionId: string, sequence: number, payload: Partial<AgentRuntimeEvent>) =>
      ({
        protocolVersion: 1,
        eventId: `event-${sessionId}-${sequence}`,
        sessionId,
        runId: `run-${sessionId}`,
        sequence,
        ...payload,
      }) as AgentRuntimeEvent;
    const stream = (sessionId: string, deltas: number) => {
      const events: AgentRuntimeEvent[] = [
        event(sessionId, 0, {
          method: "run.started",
          data: { startedAt: "2026-07-22T12:01:00Z", model: "fast" },
        }),
      ];
      for (let index = 1; index <= deltas; index++) {
        events.push(
          event(sessionId, index, {
            method: "message.delta",
            data: {
              itemSequence: 1,
              itemId: `item-${sessionId}`,
              role: "assistant",
              delta: "token ",
              createdAt: "2026-07-22T12:01:00Z",
            },
          }),
        );
      }
      return events;
    };
    const listCalls = () =>
      mocks.invoke.mock.calls.filter(([command]) => command === "list_agent_sessions").length;

    it("rereads the list for a background session only when its status moves", async () => {
      render(<AgentWorkspace initialSession={session} />);
      expect(await screen.findByText("Earlier answer")).toBeInTheDocument();
      await waitFor(() => expect(listCalls()).toBeGreaterThan(0));
      const before = listCalls();

      await act(async () => {
        for (const payload of stream("session-background", 20))
          mocks.runtimeListener?.({ payload });
        await Promise.resolve();
      });
      await waitFor(() => expect(listCalls()).toBe(before + 1));
      await act(async () => {
        mocks.runtimeListener?.({
          payload: event("session-background", 21, {
            method: "run.completed",
            data: { completedAt: "2026-07-22T12:02:00Z" },
          }),
        });
        await Promise.resolve();
      });

      await waitFor(() => expect(listCalls()).toBe(before + 2));
    });

    it("shows a failed reread after the open session's run ends", async () => {
      render(<AgentWorkspace initialSession={session} />);
      expect(await screen.findByText("Earlier answer")).toBeInTheDocument();
      const fallback = mocks.invoke.getMockImplementation();
      mocks.invoke.mockImplementation((command: string, args?: unknown) =>
        command === "list_agent_items"
          ? Promise.reject(new Error("Agent store is locked"))
          : fallback?.(command, args),
      );

      act(() => {
        mocks.runtimeListener?.({
          payload: event(session.id, 1, {
            method: "run.completed",
            data: { completedAt: "2026-07-22T12:02:00Z" },
          }),
        });
      });

      expect(await screen.findByText(/Agent store is locked/)).toBeInTheDocument();
    });

    it("announces the open session's running status once per run", async () => {
      const statuses: string[] = [];
      const onStatus = (statusEvent: Event) =>
        statuses.push((statusEvent as CustomEvent<{ status: string }>).detail.status);
      window.addEventListener(AGENT_SESSION_STATUS_EVENT, onStatus);
      try {
        render(<AgentWorkspace initialSession={session} />);
        expect(await screen.findByText("Earlier answer")).toBeInTheDocument();

        act(() => {
          for (const payload of stream(session.id, 20)) mocks.runtimeListener?.({ payload });
        });

        expect(statuses).toEqual(["running"]);
      } finally {
        window.removeEventListener(AGENT_SESSION_STATUS_EVENT, onStatus);
      }
    });
  });
});
