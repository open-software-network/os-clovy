import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentRuntimeEvent, AgentSessionDto } from "../lib/agent-runtime-contract";

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
import { AGENT_DELETE_SESSION_EVENT, AGENT_NEW_SESSION_EVENT } from "../lib/agent-events";
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
            editable: true,
          },
          {
            id: "disabled",
            name: "Disabled",
            description: "Disabled skill.",
            source: "managed",
            enabled: false,
            editable: true,
          },
        ]);
      }
      if (command === "list_venice_models") {
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
              inputCreditsPerMillionTokens: 2_000,
              outputCreditsPerMillionTokens: 4_000,
            },
          ],
        });
      }
      if (command === "provider_model_settings") {
        return Promise.resolve({
          settings: { costQuality: 100 },
          effectiveSettings: { veniceApiKeyConfigured: false },
        });
      }
      if (command === "set_cost_quality") {
        const value = (args as { request?: { value?: number } } | undefined)?.request?.value ?? 100;
        return Promise.resolve({ costQuality: value });
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
        model: "fast",
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
      if (command === "clovy_home_chat") return Promise.resolve({ reply: "Working on it." });
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
      if (command === "list_venice_models") {
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
      if (command === "list_venice_models") {
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

  it("protects live queued and Home handoff paths before staging a new drop", async () => {
    const currentPath = "/tmp/current.pdf";
    const queuedPath = "/tmp/clovy-agent-attachment-staging/queued/queued.pdf";
    const homePath = "/tmp/clovy-agent-attachment-staging/home/handoff.pdf";
    const stagedPath = "/tmp/clovy-agent-attachment-staging/new/new.pdf";
    saveQueuedAgentFollowUps({
      [newSession.id]: {
        messageId: "queued-other-session",
        prompt: "Use queued file",
        attachments: [queuedPath],
        model: "fast",
        thinkingLevel: "medium",
      },
    });
    window.localStorage.setItem(
      "clovy:home:task-handoffs:v1",
      JSON.stringify({
        "home-other-session": [
          {
            id: "home-task-protected",
            title: "Protected task",
            prompt: "Use Home file",
            status: "failed",
            attachments: [homePath],
          },
        ],
      }),
    );
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
          protectedPaths: expect.arrayContaining([currentPath, queuedPath, homePath]),
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

  it("keeps an earlier unsent Home message while blocking another submission", async () => {
    const firstPath = "/tmp/clovy-agent-attachment-staging/home-first/first.pdf";
    const secondPath = "/tmp/clovy-agent-attachment-staging/home-second/second.pdf";
    const thirdPath = "/tmp/clovy-agent-attachment-staging/home-third/third.pdf";
    let rejectCreate!: (error: Error) => void;
    const create = new Promise<AgentSessionDto>((_, reject) => {
      rejectCreate = reject;
    });
    let stagedCount = 0;
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_sessions") return Promise.resolve([]);
      if (command === "stage_agent_attachment_bytes") {
        stagedCount += 1;
        return Promise.resolve(
          stagedCount === 1 ? firstPath : stagedCount === 2 ? secondPath : thirdPath,
        );
      }
      if (command === "create_agent_session") return create;
      return defaultInvoke?.(command, args);
    });
    const user = userEvent.setup();
    render(<AgentWorkspace homeMode />);
    const form = document.querySelector<HTMLFormElement>(".agent-composer");
    expect(form).not.toBeNull();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["first"], "first.pdf")] },
    });
    expect(await screen.findByText("first.pdf")).toBeVisible();
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "First request");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["second"], "second.pdf")] },
    });
    expect(await screen.findByText("second.pdf")).toBeVisible();
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Second request");
    fireEvent.submit(form as HTMLFormElement);

    expect(
      await screen.findByText("Wait for Home to finish starting, then send again."),
    ).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
      "Second request",
    );
    expect(within(form as HTMLFormElement).getByText("second.pdf")).toBeVisible();

    await act(async () => rejectCreate(new Error("Home session creation failed")));

    expect(await screen.findByText("Unsent Home message")).toBeVisible();
    expect(screen.getByText("First request")).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
      "Second request",
    );
    expect(within(form as HTMLFormElement).getByText("second.pdf")).toBeVisible();
    expect(within(form as HTMLFormElement).queryByText("first.pdf")).not.toBeInTheDocument();

    fireEvent.submit(form as HTMLFormElement);
    expect(
      await screen.findByText("Retry or discard the unsent Home message before sending another."),
    ).toBeVisible();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "create_agent_session"),
    ).toHaveLength(1);
    expect(screen.getByText("First request")).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
      "Second request",
    );
    expect(within(form as HTMLFormElement).getByText("second.pdf")).toBeVisible();

    fireEvent.drop(form as HTMLFormElement, {
      dataTransfer: { files: [new File(["third"], "third.pdf")] },
    });
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("prune_staged_agent_attachments", {
        request: { protectedPaths: expect.arrayContaining([firstPath, secondPath]) },
      }),
    );
    expect(await screen.findByText("third.pdf")).toBeVisible();

    await user.clear(screen.getByRole("textbox", { name: "Message Clovy" }));
    await user.click(screen.getByRole("button", { name: "Remove second.pdf" }));
    await user.click(screen.getByRole("button", { name: "Remove third.pdf" }));
    const retry = screen.getByRole("button", { name: "Retry unsent Home message" });
    expect(retry).toBeEnabled();
    await user.click(retry);
    expect(screen.queryByText("Unsent Home message")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent(
      "First request",
    );
    expect(within(form as HTMLFormElement).getByText("first.pdf")).toBeVisible();
    expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled();
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

  it("hands Home attachments to a focused Clovy runtime session with send-time profile", async () => {
    const user = userEvent.setup();
    const homeSession: AgentSessionDto = {
      ...session,
      id: "home-session",
      title: "Home",
      workspacePath: "/tmp/home-session",
    };
    const focusedSession: AgentSessionDto = {
      ...newSession,
      id: "focused-session",
      title: "Summarize this file",
      workspacePath: "/tmp/focused-session",
    };
    setCurrentDataPartitionName("work");
    mocks.openDialog.mockResolvedValue(["/tmp/brief.pdf"]);
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_agent_sessions") return [homeSession, focusedSession];
      if (command === "get_agent_session") return homeSession;
      if (command === "list_agent_items" || command === "list_agent_artifacts") return [];
      if (command === "list_agent_skills") return [];
      if (command === "list_venice_models") {
        return {
          mode: "generation",
          selectedModel: "open-software/auto",
          modelType: "text",
          models: [],
        };
      }
      if (command === "provider_model_settings") {
        return {
          settings: { costQuality: 20 },
          effectiveSettings: { veniceApiKeyConfigured: false },
        };
      }
      if (command === "create_agent_session") return focusedSession;
      if (command === "start_agent_run") {
        return {
          id: "focused-run",
          sessionId: focusedSession.id,
          status: "running",
          model: "fast",
        };
      }
      return undefined;
    });

    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });
    expect(screen.queryByRole("button", { name: /Model:/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    await waitFor(() => expect(screen.getByText("brief.pdf")).toBeVisible());
    await user.type(composer, "Summarize this file");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", {
        request: {
          title: "Summarize this file",
          model: "__june_auto_generation__:20",
          safetyMode: "sandboxed",
          profile: "work",
        },
      }),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          sessionId: "focused-session",
          prompt: "Summarize this file",
          model: "__june_auto_generation__:20",
          reasoningEffort: "medium",
          safetyMode: "sandboxed",
          attachments: ["/tmp/brief.pdf"],
        }),
      }),
    );
    expect(await screen.findByRole("button", { name: "Open session" })).toBeVisible();
  });

  it("does not create another Home task for conversation after a handoff", async () => {
    const user = userEvent.setup();
    const homeSession: AgentSessionDto = {
      ...session,
      id: "home-ack-session",
      title: "Home",
      workspacePath: "/tmp/home-ack-session",
    };
    const focusedSession: AgentSessionDto = {
      ...newSession,
      id: "focused-wine-session",
      title: "Wine research",
      workspacePath: "/tmp/focused-wine-session",
    };
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_agent_sessions") return [homeSession, focusedSession];
      if (command === "get_agent_session") return homeSession;
      if (command === "list_agent_items" || command === "list_agent_artifacts") return [];
      if (command === "list_agent_skills") return [];
      if (command === "list_venice_models") {
        return {
          mode: "generation",
          selectedModel: "open-software/auto",
          modelType: "text",
          models: [],
        };
      }
      if (command === "provider_model_settings") {
        return {
          settings: { costQuality: 20 },
          effectiveSettings: { veniceApiKeyConfigured: false },
        };
      }
      if (command === "clovy_home_chat") {
        return {
          task: {
            title: "Wine research",
            prompt: "Research good wines near southern France.",
          },
        };
      }
      if (command === "create_agent_session") return focusedSession;
      if (command === "start_agent_run") {
        return {
          id: "focused-run",
          sessionId: focusedSession.id,
          status: "running",
          model: "fast",
        };
      }
      return undefined;
    });

    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Research good wines near southern France");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByRole("button", { name: "Open session" })).toBeVisible();
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.filter(([command]) => command === "create_agent_session"),
      ).toHaveLength(1),
    );

    const nextComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(nextComposer, "ok");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByText("Got it.")).toBeVisible();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "clovy_home_chat"),
    ).toHaveLength(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "create_agent_session"),
    ).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Open session" })).toHaveLength(1);

    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Hey there, Clovy 👋");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByText("Hey! What can I help with?")).toBeVisible();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "clovy_home_chat"),
    ).toHaveLength(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "create_agent_session"),
    ).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Open session" })).toHaveLength(1);

    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Greetings, Clovy");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() => expect(screen.getAllByText("Hey! What can I help with?")).toHaveLength(2));
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "clovy_home_chat"),
    ).toHaveLength(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "create_agent_session"),
    ).toHaveLength(1);
    expect(screen.getAllByRole("button", { name: "Open session" })).toHaveLength(1);

    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Plan a trip to Rome");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByText("I'm here. What can I help with?")).toBeVisible();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "create_agent_session"),
    ).toHaveLength(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run"),
    ).toHaveLength(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "clovy_home_chat"),
    ).toHaveLength(2);
  });

  it("repairs a stale Home mapping when its Clovy-owned session is missing", async () => {
    writeClovyHomeStoredSessionId("default", "missing-home-session");
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_agent_sessions") return [];
      if (command === "get_agent_session") throw new Error("Session not found");
      if (command === "list_agent_items" || command === "list_agent_artifacts") return [];
      if (command === "list_venice_models") {
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

  it("keeps a failed Home message when the user has already drafted another", async () => {
    const user = userEvent.setup();
    const homeSession: AgentSessionDto = {
      ...session,
      id: "home-failure-session",
      title: "Home",
      workspacePath: "/tmp/home-failure-session",
    };
    let rejectHome: ((error: Error) => void) | undefined;
    const pendingHome = new Promise((_, reject) => {
      rejectHome = reject;
    });
    mocks.invoke.mockImplementation(async (command: string) => {
      if (command === "list_agent_sessions") return [homeSession];
      if (command === "get_agent_session") return homeSession;
      if (command === "list_agent_items" || command === "list_agent_artifacts") return [];
      if (command === "list_venice_models") {
        return { mode: "generation", selectedModel: "fast", modelType: "text", models: [] };
      }
      if (command === "clovy_home_chat") return pendingHome;
      return undefined;
    });

    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    const composer = await screen.findByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "First message");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("clovy_home_chat", expect.anything()),
    );
    const activeComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    activeComposer.textContent = "New draft";
    fireEvent.input(activeComposer);
    expect(activeComposer).toHaveTextContent("New draft");
    await waitFor(() => expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled());
    rejectHome?.(new Error("Home is temporarily unavailable"));

    expect(await screen.findByText("First message")).toBeVisible();
    const errorNotice = await screen.findByRole("alert");
    expect(errorNotice).toHaveTextContent("Home is temporarily unavailable");
    expect(errorNotice.closest(".agent-composer")).not.toBeNull();
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent("New draft"),
    );
  });

  it("hydrates history, shows an optimistic turn, and cancels", async () => {
    const user = userEvent.setup();
    const { container } = render(<AgentWorkspace initialSession={session} />);

    expect(await screen.findByText("Earlier answer")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sandboxed" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Model: Fast" })).toBeEnabled();
    expect(container.querySelector(".agent-scroll .agent-main > .agent-composer")).not.toBeNull();
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.click(composer);
    await user.type(composer, "New request");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    expect(await screen.findByText("New request")).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          model: "fast",
          safetyMode: "sandboxed",
          enabledSkillIds: ["notes"],
        }),
      }),
    );
    const activeRunPicker = screen.getByRole("button", { name: "Model: Fast" });
    expect(activeRunPicker).toBeEnabled();
    await user.click(activeRunPicker);
    await user.click(
      within(screen.getByRole("listbox", { name: "Suggested text models" })).getByRole("option", {
        name: /Auto/,
      }),
    );
    expect(screen.getByRole("button", { name: "Model: Auto" })).toBeEnabled();

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

    const nextRunPicker = await screen.findByRole("button", { name: "Model: Auto" });
    expect(nextRunPicker).toBeEnabled();
    const nextComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(nextComposer, "Use the staged model");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          prompt: "Use the staged model",
          model: "__june_auto_generation__:100",
        }),
      }),
    );
  });

  it("restores Auto-only suggestions, root model search, and compact effort choices", async () => {
    const user = userEvent.setup();
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_venice_models") {
        return Promise.resolve({
          mode: "generation",
          selectedModel: "fast",
          modelType: "text",
          models: [
            {
              provider: "june",
              id: "fast",
              name: "Fast",
              modelType: "text",
              traits: [],
              capabilities: ["tool-calling"],
            },
            {
              provider: "venice",
              id: "kimi-k3",
              name: "Kimi K3",
              modelType: "text",
              traits: [],
              capabilities: ["tool-calling"],
            },
            {
              provider: "venice",
              id: "zai-org-glm-5-2",
              name: "GLM 5.2",
              modelType: "text",
              traits: [],
              capabilities: ["tool-calling"],
            },
          ],
        });
      }
      return defaultInvoke?.(command, args);
    });

    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");
    await user.click(await screen.findByRole("button", { name: "Model: Fast" }));

    const suggested = screen.getByRole("listbox", { name: "Suggested text models" });
    const autoOption = within(suggested).getByRole("option", { name: /Auto/ });
    expect(autoOption).toBeVisible();
    expect(autoOption.querySelector('.model-row-privacy[data-mode="private"]')).toHaveAttribute(
      "aria-label",
      expect.stringMatching(/^Private mode:/),
    );
    expect(within(suggested).queryByText("GLM 5.2")).not.toBeInTheDocument();
    expect(within(suggested).queryByText("Kimi K3")).not.toBeInTheDocument();

    const search = screen.getByRole("combobox", { name: "Search models" });
    await user.type(search, "Kimi");
    expect(
      within(screen.getByRole("listbox", { name: "Matching models" })).getByRole("option", {
        name: /Kimi K3/,
      }),
    ).toBeVisible();

    await user.clear(search);
    await user.type(search, "no-such-model");
    const pickerDialog = screen.getByRole("dialog", { name: "Choose text model" });
    expect(within(pickerDialog).getByRole("status")).toHaveTextContent(
      "No results match your search.",
    );
    await user.clear(search);
    await user.type(search, "Kimi");

    await user.keyboard("{Escape}");
    expect(search).toHaveValue("");
    expect(search).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "All models" }));
    expect(screen.getByRole("group", { name: "All text models" })).toBeVisible();
    const allModelsList = screen.getByRole("listbox", { name: "All text models" });
    const allModelOptions = within(allModelsList).getAllByRole("option");
    allModelOptions[0]?.focus();
    fireEvent.keyDown(allModelsList, { key: "End" });
    expect(allModelOptions.at(-1)).toHaveFocus();
    fireEvent.keyDown(allModelsList, { key: "Home" });
    expect(allModelOptions[0]).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("group", { name: "All text models" })).not.toBeInTheDocument();
    expect(search).toHaveFocus();

    const effortTrigger = screen.getByRole("button", { name: /Effort.*Medium/ });
    fireEvent.mouseEnter(effortTrigger);
    expect(await screen.findByRole("group", { name: "Thinking level" })).toBeVisible();
    expect(search).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("group", { name: "Thinking level" })).not.toBeInTheDocument();
    expect(search).toHaveFocus();

    await user.click(effortTrigger);
    const effort = screen.getByRole("group", { name: "Thinking level" });
    expect(effort).toHaveClass("agent-composer-model-effort-panel");
    const lowEffort = within(effort).getByRole("menuitemradio", {
      name: /Low.*Faster responses/,
    });
    const mediumEffort = within(effort).getByRole("menuitemradio", {
      name: /Medium.*Balances speed/,
    });
    const highEffort = within(effort).getByRole("menuitemradio", {
      name: /High.*Deeper reasoning/,
    });
    expect(lowEffort).toBeVisible();
    expect(highEffort).toBeVisible();
    await waitFor(() => expect(mediumEffort).toHaveFocus());
    await user.keyboard("{ArrowDown}");
    expect(highEffort).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("group", { name: "Thinking level" })).not.toBeInTheDocument();
    const trigger = screen.getByRole("button", { name: "Model: Fast" });
    await waitFor(() => expect(trigger).toHaveFocus());

    await user.click(trigger);
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("combobox", { name: "Search models" })).not.toBeInTheDocument();
    await waitFor(() => expect(trigger).toHaveFocus());
  });

  it("restores Auto Economy, Balanced, and Quality with session-persistent routing", async () => {
    const user = userEvent.setup();
    const autoSession = {
      ...session,
      model: "__june_auto_generation__:50",
    };
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_sessions") return Promise.resolve([autoSession]);
      if (command === "get_agent_session") return Promise.resolve(autoSession);
      if (command === "provider_model_settings") {
        return Promise.resolve({
          settings: { costQuality: 100 },
          effectiveSettings: { veniceApiKeyConfigured: false },
        });
      }
      return defaultInvoke?.(command, args);
    });

    render(<AgentWorkspace initialSession={autoSession} />);
    await screen.findByText("Earlier answer");
    const trigger = await screen.findByRole("button", { name: "Model: Auto" });
    expect(trigger).toHaveAttribute("title", expect.stringContaining("Preference: Balanced"));

    await user.click(trigger);
    await user.click(screen.getByRole("button", { name: /Preference.*Balanced/ }));
    const preferences = screen.getByRole("group", { name: "Auto preference" });
    expect(
      within(preferences).getByRole("menuitemradio", {
        name: /Economy.*Favors cheaper models/,
      }),
    ).toBeVisible();
    expect(
      within(preferences).getByRole("menuitemradio", {
        name: /Balanced.*Weighs quality against cost/,
      }),
    ).toBeVisible();
    expect(
      within(preferences).getByRole("menuitemradio", {
        name: /Quality.*Routes to the strongest model/,
      }),
    ).toBeVisible();

    await user.click(within(preferences).getByRole("menuitemradio", { name: /Economy/ }));
    expect(trigger).toHaveAttribute("title", expect.stringContaining("Preference: Economy"));
    expect(mocks.invoke).not.toHaveBeenCalledWith("set_cost_quality", expect.anything());

    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("group", { name: "Auto preference" })).not.toBeInTheDocument(),
    );
    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Choose text model" })).not.toBeInTheDocument(),
    );
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Use Economy";
    fireEvent.input(composer);
    await waitFor(() => expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          sessionId: autoSession.id,
          prompt: "Use Economy",
          model: "__june_auto_generation__:20",
        }),
      }),
    );
  });

  it("persists a fresh Auto preference and applies it to the first run", async () => {
    const user = userEvent.setup();
    render(<AgentWorkspace />);

    const trigger = await screen.findByRole("button", { name: "Model: Auto" });
    expect(trigger).toHaveAttribute("title", expect.stringContaining("Preference: Quality"));
    await user.click(trigger);
    await user.click(screen.getByRole("button", { name: /Preference.*Quality/ }));
    await user.click(
      within(screen.getByRole("group", { name: "Auto preference" })).getByRole("menuitemradio", {
        name: /Economy/,
      }),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("set_cost_quality", {
        request: { value: 20 },
      }),
    );

    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("group", { name: "Auto preference" })).not.toBeInTheDocument(),
    );
    await user.keyboard("{Escape}");
    await waitFor(() =>
      expect(screen.queryByRole("dialog", { name: "Choose text model" })).not.toBeInTheDocument(),
    );
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Fresh Economy request";
    fireEvent.input(composer);
    await waitFor(() => expect(screen.getByRole("button", { name: "Send message" })).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("create_agent_session", {
        request: expect.objectContaining({
          title: "Fresh Economy request",
          model: "__june_auto_generation__:20",
        }),
      }),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          prompt: "Fresh Economy request",
          model: "__june_auto_generation__:20",
        }),
      }),
    );
  });

  it("keeps a freshly selected Auto preference when settings hydration finishes late", async () => {
    const user = userEvent.setup();
    let resolveSettings: ((value: unknown) => void) | undefined;
    const pendingSettings = new Promise((resolve) => {
      resolveSettings = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "provider_model_settings") return pendingSettings;
      return defaultInvoke?.(command, args);
    });

    render(<AgentWorkspace />);
    const trigger = await screen.findByRole("button", { name: "Model: Auto" });
    await user.click(trigger);
    await user.click(screen.getByRole("button", { name: /Preference.*Quality/ }));
    await user.click(
      within(screen.getByRole("group", { name: "Auto preference" })).getByRole("menuitemradio", {
        name: /Economy/,
      }),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("set_cost_quality", {
        request: { value: 20 },
      }),
    );

    await act(async () => {
      resolveSettings?.({
        settings: { costQuality: 100 },
        effectiveSettings: { veniceApiKeyConfigured: false },
      });
      await pendingSettings;
    });

    expect(trigger).toHaveAttribute("title", expect.stringContaining("Preference: Economy"));
  });

  it("does not overwrite a session preference when a global save finishes after navigation", async () => {
    const user = userEvent.setup();
    const balancedSession = { ...session, model: "__june_auto_generation__:50" };
    let resolveSave: ((value: { costQuality: number }) => void) | undefined;
    const pendingSave = new Promise<{ costQuality: number }>((resolve) => {
      resolveSave = resolve;
    });
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_sessions") return Promise.resolve([balancedSession]);
      if (command === "get_agent_session") return Promise.resolve(balancedSession);
      if (command === "set_cost_quality") return pendingSave;
      return defaultInvoke?.(command, args);
    });

    const { rerender } = render(<AgentWorkspace />);
    const freshTrigger = await screen.findByRole("button", { name: "Model: Auto" });
    await user.click(freshTrigger);
    await user.click(screen.getByRole("button", { name: /Preference.*Quality/ }));
    await user.click(
      within(screen.getByRole("group", { name: "Auto preference" })).getByRole("menuitemradio", {
        name: /Economy/,
      }),
    );
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("set_cost_quality", {
        request: { value: 20 },
      }),
    );

    rerender(<AgentWorkspace initialSession={balancedSession} />);
    const sessionTrigger = await screen.findByRole("button", { name: "Model: Auto" });
    await waitFor(() =>
      expect(sessionTrigger).toHaveAttribute(
        "title",
        expect.stringContaining("Preference: Balanced"),
      ),
    );

    await act(async () => {
      resolveSave?.({ costQuality: 20 });
      await pendingSave;
    });

    expect(sessionTrigger).toHaveAttribute(
      "title",
      expect.stringContaining("Preference: Balanced"),
    );
  });

  it("shows context, estimated charge, and per-tool usage for the latest run", async () => {
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
    expect(usagePanel).toHaveTextContent("10,000 of 200,000 (5.0%)");
    expect(usagePanel).toHaveTextContent("28 credits (about $0.0280)");
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

  it("keeps the background files panel open when Usage claims Escape", async () => {
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_artifacts") {
        return Promise.resolve([
          {
            id: "artifact-1",
            sessionId: session.id,
            name: "results.md",
            path: "/tmp/session-1/results.md",
            mimeType: "text/markdown",
            action: "created",
            available: true,
            createdAt: session.createdAt,
          },
        ]);
      }
      return defaultInvoke?.(command, args);
    });

    const user = userEvent.setup();
    render(
      <div className="app-shell">
        <AgentWorkspace initialSession={session} />
      </div>,
    );
    await screen.findByText("Earlier answer");

    await user.click(screen.getByRole("button", { name: "View files (1)" }));
    expect(screen.getByRole("complementary", { name: "Files" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Session actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Usage" }));
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("dialog", { name: "Usage" })).not.toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Files" })).toBeInTheDocument();
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
    expect(usagePanel).toHaveTextContent("Auto");
    expect(usagePanel).not.toHaveTextContent("__june_auto_generation__");
    expect(usagePanel).toHaveTextContent("Token counts were not reported for this request.");
  });

  it("steers an active run at the next model boundary and retires the fallback queue", async () => {
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Start the analysis");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Stop Clovy" })).toBeVisible());

    const scroller = document.querySelector<HTMLElement>(".agent-scroll");
    expect(scroller).not.toBeNull();
    const scrollTo = vi.fn();
    Object.defineProperties(scroller as HTMLElement, {
      scrollHeight: { configurable: true, get: () => 1000 },
      clientHeight: { configurable: true, get: () => 400 },
      scrollTop: { configurable: true, writable: true, value: 100 },
      scrollTo: { configurable: true, value: scrollTo },
    });

    const activeComposer = screen.getByRole("textbox", { name: "Message Clovy" });
    activeComposer.textContent = "Use the launch plan";
    fireEvent.input(activeComposer);
    await user.click(await screen.findByRole("button", { name: "Steer active run" }));

    await waitFor(() => expect(scrollTo).toHaveBeenCalledWith({ top: 1000, behavior: "smooth" }));

    const steerCall = mocks.invoke.mock.calls.find(([command]) => command === "steer_agent_run");
    expect(steerCall?.[1]).toMatchObject({
      runId: "run-1",
      text: "Use the launch plan",
      messageId: expect.any(String),
    });
    expect(await screen.findByText("Steering active run")).toBeVisible();

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-steering",
          sessionId: session.id,
          runId: "run-1",
          sequence: 3,
          method: "steering.consumed",
          data: {
            itemId: "steering-1",
            messageId: String((steerCall?.[1] as { messageId?: string })?.messageId),
            text: "Use the launch plan",
            createdAt: "2026-07-22T12:01:00Z",
          },
        },
      });
    });

    expect(await screen.findByText("Steering: Use the launch plan")).toBeVisible();
    expect(screen.queryByText("Steering active run")).not.toBeInTheDocument();
  });

  it("steers text with attachments and submits the attachments once after consumption", async () => {
    const user = userEvent.setup();
    mocks.openDialog.mockResolvedValue(["/tmp/brief.pdf"]);
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    let composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Start the analysis");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await screen.findByRole("button", { name: "Stop Clovy" });

    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Use the attached brief";
    fireEvent.input(composer);
    await user.click(await screen.findByRole("button", { name: "Steer active run" }));

    const steerCall = mocks.invoke.mock.calls.find(([command]) => command === "steer_agent_run");
    expect(steerCall?.[1]).toMatchObject({
      runId: "run-1",
      text: "Use the attached brief",
      messageId: expect.any(String),
    });
    expect(
      await screen.findByText("Steering active run. 1 attachment queued for next turn"),
    ).toBeVisible();

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-steering-with-attachment",
          sessionId: session.id,
          runId: "run-1",
          sequence: 3,
          method: "steering.consumed",
          data: {
            itemId: "steering-with-attachment",
            messageId: String((steerCall?.[1] as { messageId?: string })?.messageId),
            text: "Use the attached brief",
            createdAt: "2026-07-22T12:01:00Z",
          },
        },
      });
    });

    expect(await screen.findByText("Steering: Use the attached brief")).toBeVisible();
    expect(screen.getByText("1 attachment queued for next turn")).toBeVisible();
    expect(screen.getByText("brief.pdf")).toBeVisible();

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-completed-after-attachment-steer",
          sessionId: session.id,
          runId: "run-1",
          sequence: 4,
          method: "run.completed",
          data: { completedAt: "2026-07-22T12:02:00Z" },
        },
      });
    });

    await waitFor(() => {
      const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
      expect(starts).toHaveLength(2);
      expect(starts[1]?.[1]).toMatchObject({
        request: expect.objectContaining({
          prompt: `Use the attached brief\n\n${ATTACHMENT_FOLLOW_UP_NOTE}`,
          attachments: ["/tmp/brief.pdf"],
        }),
      });
    });
    await act(async () => new Promise((resolve) => window.setTimeout(resolve, 0)));
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run"),
    ).toHaveLength(2);
  });

  it("merges a later steer into an attachments-only queue without dropping files", async () => {
    const user = userEvent.setup();
    mocks.openDialog
      .mockResolvedValueOnce(["/tmp/brief.pdf"])
      .mockResolvedValueOnce(["/tmp/notes.md"]);
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    let composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Start the analysis");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await screen.findByRole("button", { name: "Stop Clovy" });

    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Use the first brief";
    fireEvent.input(composer);
    await user.click(await screen.findByRole("button", { name: "Steer active run" }));

    let steerCalls = mocks.invoke.mock.calls.filter(([command]) => command === "steer_agent_run");
    const firstMessageId = String(
      (steerCalls[0]?.[1] as { messageId?: string } | undefined)?.messageId,
    );
    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-consumed-first-brief",
          sessionId: session.id,
          runId: "run-1",
          sequence: 3,
          method: "steering.consumed",
          data: {
            itemId: `steering:${firstMessageId}`,
            messageId: firstMessageId,
            text: "Use the first brief",
            createdAt: "2026-07-22T12:01:00Z",
          },
        },
      });
    });
    expect(await screen.findByText("1 attachment queued for next turn")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Use the latest plan";
    fireEvent.input(composer);
    await user.click(await screen.findByRole("button", { name: "Steer active run" }));

    steerCalls = mocks.invoke.mock.calls.filter(([command]) => command === "steer_agent_run");
    expect(steerCalls).toHaveLength(2);
    const secondMessageId = String(
      (steerCalls[1]?.[1] as { messageId?: string } | undefined)?.messageId,
    );
    await waitFor(() => {
      const stored = JSON.parse(
        window.localStorage.getItem("clovy.agent.queuedFollowUps") ?? "{}",
      ) as Record<string, { messageId?: string; prompt?: string; attachments?: string[] }>;
      expect(stored[session.id]).toMatchObject({
        messageId: secondMessageId,
        prompt: "Use the latest plan",
        attachments: ["/tmp/brief.pdf", "/tmp/notes.md"],
      });
    });

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-consumed-latest-plan",
          sessionId: session.id,
          runId: "run-1",
          sequence: 4,
          method: "steering.consumed",
          data: {
            itemId: `steering:${secondMessageId}`,
            messageId: secondMessageId,
            text: "Use the latest plan",
            createdAt: "2026-07-22T12:01:30Z",
          },
        },
      });
    });
    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-completed-after-merged-steer",
          sessionId: session.id,
          runId: "run-1",
          sequence: 5,
          method: "run.completed",
          data: { completedAt: "2026-07-22T12:02:00Z" },
        },
      });
    });

    await waitFor(() => {
      const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
      expect(starts).toHaveLength(2);
      expect(starts[1]?.[1]).toMatchObject({
        request: expect.objectContaining({
          prompt: `Use the latest plan\n\n${ATTACHMENT_FOLLOW_UP_NOTE}`,
          attachments: ["/tmp/brief.pdf", "/tmp/notes.md"],
        }),
      });
    });
  });

  it("falls back to the full queued follow-up when steering is rejected", async () => {
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "steer_agent_run") {
        return Promise.resolve({ accepted: false, reason: "not_active" });
      }
      return defaultInvoke?.(command, args);
    });
    mocks.openDialog.mockResolvedValue(["/tmp/brief.pdf"]);
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    try {
      const user = userEvent.setup();
      render(<AgentWorkspace initialSession={session} />);
      await screen.findByText("Earlier answer");

      let composer = screen.getByRole("textbox", { name: "Message Clovy" });
      await user.type(composer, "Start the analysis");
      await user.click(screen.getByRole("button", { name: "Send message" }));
      await screen.findByRole("button", { name: "Stop Clovy" });

      await user.click(screen.getByRole("button", { name: "Add files or notes" }));
      await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
      composer = screen.getByRole("textbox", { name: "Message Clovy" });
      composer.textContent = "Use the attached brief";
      fireEvent.input(composer);
      await user.click(await screen.findByRole("button", { name: "Steer active run" }));

      expect(
        await screen.findByText("Queued follow-up. 1 attachment queued for next turn"),
      ).toBeVisible();
      expect(warn).toHaveBeenCalledWith(
        "Live steering was rejected; queued the full follow-up instead.",
        expect.objectContaining({ reason: "not_active", messageId: expect.any(String) }),
      );

      act(() => {
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "event-completed-after-rejected-steer",
            sessionId: session.id,
            runId: "run-1",
            sequence: 3,
            method: "run.completed",
            data: { completedAt: "2026-07-22T12:02:00Z" },
          },
        });
      });

      await waitFor(() => {
        const starts = mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run");
        expect(starts).toHaveLength(2);
        expect(starts[1]?.[1]).toMatchObject({
          request: expect.objectContaining({
            prompt: "Use the attached brief",
            attachments: ["/tmp/brief.pdf"],
          }),
        });
      });
    } finally {
      warn.mockRestore();
    }
  });

  it("submits an unconsumed live instruction as the next run after settlement", async () => {
    const user = userEvent.setup();
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    let composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Start the analysis");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Stop Clovy" })).toBeVisible());

    await user.click(screen.getByRole("button", { name: "Model: Fast" }));
    await user.click(
      within(screen.getByRole("listbox", { name: "Suggested text models" })).getByRole("option", {
        name: /Auto/,
      }),
    );

    composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Send this next";
    fireEvent.input(composer);
    await user.click(await screen.findByRole("button", { name: "Steer active run" }));

    await user.click(screen.getByRole("button", { name: "Model: Auto" }));
    await user.click(screen.getByRole("button", { name: "All models" }));
    await user.click(
      within(screen.getByRole("group", { name: "All text models" })).getByRole("option", {
        name: /Fast/,
      }),
    );

    act(() => {
      mocks.runtimeListener?.({
        payload: {
          protocolVersion: 1,
          eventId: "event-completed",
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
      expect(starts).toHaveLength(2);
      expect(starts[1]?.[1]).toMatchObject({
        request: expect.objectContaining({
          prompt: "Send this next",
          model: "__june_auto_generation__:100",
        }),
      });
    });
    expect(screen.getByRole("button", { name: "Model: Fast" })).toBeEnabled();
  });

  it("downgrades terminal steering and clears the queued snapshot when delivery races a new run", async () => {
    const user = userEvent.setup();
    mocks.openDialog.mockResolvedValue(["/tmp/brief.pdf"]);
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByText("Earlier answer");

    let composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await user.type(composer, "Start the analysis");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await screen.findByRole("button", { name: "Stop Clovy" });

    await user.click(screen.getByRole("button", { name: "Add files or notes" }));
    await user.click(screen.getByRole("menuitem", { name: "Attach files" }));
    composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Deliver this after settlement";
    fireEvent.input(composer);
    await user.click(await screen.findByRole("button", { name: "Steer active run" }));
    expect(
      await screen.findByText("Steering active run. 1 attachment queued for next turn"),
    ).toBeVisible();

    const frameCallbacks: FrameRequestCallback[] = [];
    const animationFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => {
        frameCallbacks.push(callback);
        return frameCallbacks.length;
      });
    try {
      act(() => {
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "event-terminal-before-queued-delivery",
            sessionId: session.id,
            runId: "run-1",
            sequence: 3,
            method: "run.completed",
            data: { completedAt: "2026-07-22T12:01:00Z" },
          },
        });
      });

      expect(
        await screen.findByText("Queued follow-up. 1 attachment queued for next turn"),
      ).toBeVisible();
      await waitFor(() => expect(frameCallbacks.length).toBeGreaterThan(0));

      act(() => {
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "event-new-run-before-queued-delivery",
            sessionId: session.id,
            runId: "run-2",
            sequence: 0,
            method: "run.started",
            data: {
              startedAt: "2026-07-22T12:01:01Z",
              model: "fast",
            },
          },
        });
      });
      const queuedFrames = frameCallbacks.splice(0);
      act(() => {
        for (const callback of queuedFrames) callback(0);
      });

      await waitFor(() => {
        const steerCalls = mocks.invoke.mock.calls.filter(
          ([command]) => command === "steer_agent_run",
        );
        expect(steerCalls).toHaveLength(2);
        expect(steerCalls[1]?.[1]).toMatchObject({
          runId: "run-2",
          text: "Deliver this after settlement",
        });
      });

      composer = screen.getByRole("textbox", { name: "Message Clovy" });
      composer.textContent = "Latest live correction";
      fireEvent.input(composer);
      await user.click(await screen.findByRole("button", { name: "Steer active run" }));

      await waitFor(() => {
        const steerCalls = mocks.invoke.mock.calls.filter(
          ([command]) => command === "steer_agent_run",
        );
        expect(steerCalls).toHaveLength(3);
        expect(steerCalls[2]?.[1]).toMatchObject({
          runId: "run-2",
          text: "Latest live correction",
        });
        const stored = JSON.parse(
          window.localStorage.getItem("clovy.agent.queuedFollowUps") ?? "{}",
        ) as Record<string, { prompt?: string; attachments?: string[] }>;
        expect(stored[session.id]).toMatchObject({
          prompt: "Latest live correction",
          attachments: ["/tmp/brief.pdf"],
        });
      });
    } finally {
      animationFrame.mockRestore();
    }
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
    await user.click(await screen.findByRole("button", { name: "Steer active run" }));
    expect(
      await screen.findByText("Steering active run. 1 attachment queued for next turn"),
    ).toBeVisible();

    rerender(<AgentWorkspace initialSession={newSession} />);
    await waitFor(() => expect(screen.queryByRole("button", { name: "Stop Clovy" })).toBeNull());
    expect(
      screen.queryByText("Steering active run. 1 attachment queued for next turn"),
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
          model: "fast",
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
        model: "open-software/auto",
        thinkingLevel: "hard",
      },
    });

    render(<AgentWorkspace initialSession={session} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          sessionId: session.id,
          prompt: "Resume with this file",
          attachments: ["/tmp/restored.pdf"],
          model: "open-software/auto",
          reasoningEffort: "high",
        }),
      }),
    );
    expect(await screen.findByText("Resume with this file")).toBeVisible();
  });

  it("releases a queued attempt when submit becomes blocked before its frame", async () => {
    saveQueuedAgentFollowUps({
      [session.id]: {
        messageId: "blocked-frame",
        prompt: "Retry after funding refresh",
        attachments: ["/tmp/retry.pdf"],
        model: "fast",
        thinkingLevel: "medium",
      },
    });
    const frameCallbacks: FrameRequestCallback[] = [];
    const animationFrame = vi
      .spyOn(window, "requestAnimationFrame")
      .mockImplementation((callback) => {
        frameCallbacks.push(callback);
        return frameCallbacks.length;
      });
    try {
      const view = render(<AgentWorkspace initialSession={session} />);
      await waitFor(() => expect(frameCallbacks).toHaveLength(1));

      view.rerender(
        <AgentWorkspace
          initialSession={session}
          creditActionsDisabledReason="Add credits to continue"
        />,
      );
      act(() => frameCallbacks.shift()?.(0));
      await waitFor(() =>
        expect(
          mocks.invoke.mock.calls.filter(([command]) => command === "start_agent_run"),
        ).toHaveLength(0),
      );

      view.rerender(<AgentWorkspace initialSession={session} />);
      await waitFor(() => expect(frameCallbacks).toHaveLength(1));
      act(() => frameCallbacks.shift()?.(0));
      await waitFor(() =>
        expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
          request: expect.objectContaining({
            prompt: "Retry after funding refresh",
            attachments: ["/tmp/retry.pdf"],
          }),
        }),
      );
    } finally {
      animationFrame.mockRestore();
    }
  });

  it("keeps a restored follow-up when its run start fails after navigation", async () => {
    saveQueuedAgentFollowUps({
      [session.id]: {
        messageId: "retry-after-navigation",
        prompt: "Retry this in session A",
        attachments: ["/tmp/retry.pdf"],
        model: "fast",
        thinkingLevel: "medium",
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
        model: "fast",
        thinkingLevel: "medium",
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
      if (command === "list_venice_models") return pendingCatalog;
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
    expect(screen.getByRole("button", { name: "Model: Fast" })).toBeEnabled();
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
    expect(screen.getByRole("button", { name: "Model: Fast" })).toBeEnabled();

    act(() => rememberSessionModel(session.id, "__june_auto_generation__:20"));

    expect(screen.getByRole("button", { name: "Model: Auto" }).getAttribute("title")).toContain(
      "Preference: Economy",
    );
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

    fireEvent.click(await screen.findByRole("button", { name: /Clovy/ }));
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
      if (command === "list_venice_models") {
        return Promise.resolve({ mode: "generation", models: [] });
      }
      if (command === "provider_model_settings") {
        return Promise.resolve({
          settings: { costQuality: 100 },
          effectiveSettings: { veniceApiKeyConfigured: false },
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
      if (command === "list_venice_models") {
        return Promise.resolve({ mode: "generation", models: [] });
      }
      if (command === "retry_agent_run") {
        return Promise.resolve({
          id: "run-retry",
          sessionId: session.id,
          status: "running",
          model: "fast",
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

  it("stages model and effort changes made while a fresh session is still being created", async () => {
    const user = userEvent.setup();
    let resolveCreate: ((value: AgentSessionDto) => void) | undefined;
    const pendingCreate = new Promise<AgentSessionDto>((resolve) => {
      resolveCreate = resolve;
    });
    const createdSession = { ...newSession, model: "fast" };
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      const sessionId = (args as { sessionId?: string } | undefined)?.sessionId;
      if (command === "create_agent_session") return pendingCreate;
      if (command === "get_agent_session" && sessionId === createdSession.id) {
        return Promise.resolve(createdSession);
      }
      if (command === "list_agent_items" && sessionId === createdSession.id) {
        return Promise.resolve([]);
      }
      return defaultInvoke?.(command, args);
    });

    const { container, rerender } = render(<AgentWorkspace />);
    await user.click(await screen.findByRole("button", { name: "Model: Auto" }));
    await user.click(screen.getByRole("button", { name: "All models" }));
    await user.click(
      within(screen.getByRole("group", { name: "All text models" })).getByRole("option", {
        name: /Fast/,
      }),
    );
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    composer.textContent = "Create this slowly";
    fireEvent.input(composer);
    const send = screen.getByRole("button", { name: "Send message" });
    await waitFor(() => expect(send).toBeEnabled());
    await user.click(send);
    await waitFor(
      () =>
        expect(container.querySelector(".agent-user-turn")).toHaveTextContent("Create this slowly"),
      { timeout: 3_000 },
    );
    expect(screen.getByText("Thinking…")).toBeVisible();

    await user.click(screen.getByRole("button", { name: "Model: Fast" }));
    await user.click(
      within(screen.getByRole("listbox", { name: "Suggested text models" })).getByRole("option", {
        name: /Auto/,
      }),
    );
    await user.click(screen.getByRole("button", { name: "Model: Auto" }));
    await user.click(screen.getByRole("button", { name: /Effort.*Medium/ }));
    await user.click(
      within(screen.getByRole("group", { name: "Thinking level" })).getByRole("menuitemradio", {
        name: /High.*Deeper reasoning/,
      }),
    );

    await act(async () => resolveCreate?.(createdSession));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({
          sessionId: createdSession.id,
          model: "fast",
          reasoningEffort: "medium",
        }),
      }),
    );
    expect(screen.getByRole("button", { name: "Model: Auto" })).toHaveAttribute(
      "title",
      expect.stringContaining("Effort: High"),
    );
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "set_venice_model"),
    ).toHaveLength(1);

    rerender(<AgentWorkspace initialSession={session} />);
    expect(await screen.findByRole("button", { name: "Model: Fast" })).toBeEnabled();
    rerender(<AgentWorkspace initialSession={createdSession} />);
    expect(await screen.findByRole("button", { name: "Model: Auto" })).toHaveAttribute(
      "title",
      expect.stringContaining("Effort: High"),
    );
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

  it("uses the priced Clovy Auto model id for a fresh workspace", async () => {
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
          model: "__june_auto_generation__:100",
          title: "Fresh request",
          profile: "private",
        }),
      }),
    );
  });

  it("keeps explicit Venice BYOK text available when Clovy credits are unavailable", async () => {
    const user = userEvent.setup();
    const veniceSession = { ...session, model: "venice-text" };
    const defaultInvoke = mocks.invoke.getMockImplementation();
    mocks.invoke.mockImplementation((command: string, args?: unknown) => {
      if (command === "list_agent_sessions") return Promise.resolve([veniceSession]);
      if (command === "get_agent_session") return Promise.resolve(veniceSession);
      if (command === "list_venice_models") {
        return Promise.resolve({
          mode: "generation",
          selectedModel: "venice-text",
          modelType: "text",
          models: [
            {
              provider: "venice",
              id: "venice-text",
              name: "Venice text",
              modelType: "text",
              traits: [],
              capabilities: ["tool-calling"],
              privacy: "private",
              contextTokens: 128_000,
            },
          ],
        });
      }
      if (command === "provider_model_settings") {
        return Promise.resolve({
          settings: { costQuality: 100 },
          effectiveSettings: { veniceApiKeyConfigured: true },
        });
      }
      return defaultInvoke?.(command, args);
    });

    render(
      <AgentWorkspace
        initialSession={veniceSession}
        creditActionsDisabledReason="Add credits to continue"
      />,
    );
    await screen.findByText("Earlier answer");
    const composer = screen.getByRole("textbox", { name: "Message Clovy" });
    await waitFor(() => expect(composer).toBeEnabled());
    await user.type(composer, "Use my Venice key");
    await user.click(screen.getByRole("button", { name: "Send message" }));

    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("start_agent_run", {
        request: expect.objectContaining({ model: "venice-text" }),
      }),
    );
  });
});
