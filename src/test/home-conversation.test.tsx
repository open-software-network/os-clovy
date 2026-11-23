import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentRuntimeEvent, AgentSessionDto } from "../lib/agent-runtime-contract";

// Home runs on the default CLI engine, in its own hidden session, like any
// CLI session (ADR-0064, openspec change inicio-no-claude-code).

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
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

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { AgentWorkspace } from "../components/agent/AgentWorkspace";
import { resetAgentSessionDraftsForTests } from "../lib/agent-session-drafts";
import { resetCurrentDataPartitionForTests } from "../lib/data-partition";

const CLAUDE = "__clovy_cli_engine__:claude";
const CODEX = "__clovy_cli_engine__:codex";

const homeSession: AgentSessionDto = {
  id: "home-session",
  title: "Home",
  status: "idle",
  // A Home session from before the change.
  model: "open-software/auto",
  safetyMode: "sandboxed",
  workspacePath: "/tmp/home-session",
  source: "user",
  createdAt: "2026-10-01T09:00:00.000Z",
  updatedAt: "2026-10-01T09:00:00.000Z",
};

type Handler = (args?: unknown) => unknown;

function mockInvoke(overrides: Record<string, Handler> = {}) {
  mocks.invoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command in overrides) return overrides[command](args);
    switch (command) {
      case "chat_default_engine":
        return { modelId: CLAUDE, cli: "claude", legacyFallback: false };
      case "list_agent_sessions":
        return [homeSession];
      case "get_agent_session":
        return homeSession;
      case "list_agent_items":
      case "list_agent_artifacts":
      case "list_agent_skills":
        return [];
      case "list_provider_models":
        return { mode: "generation", selectedModel: "open-software/auto", models: [] };
      case "provider_model_settings":
        return {
          settings: {},
          effectiveSettings: {
            providerApiKeysConfigured: {
              groq: false,
              openrouter: false,
              fal: false,
              tavily: false,
            },
          },
        };
      case "start_agent_run":
        return { id: "home-run", sessionId: homeSession.id, status: "running", model: CLAUDE };
      case "import_legacy_home_turns":
        return { imported: 0, skipped: 0, alreadyImported: false };
      default:
        return undefined;
    }
  });
}

function calls(command: string) {
  return mocks.invoke.mock.calls.filter(([name]) => name === command);
}

async function send(text: string) {
  const user = userEvent.setup();
  const composer = await screen.findByRole("textbox", { name: "Message Clovy" });
  await user.click(composer);
  await user.type(composer, text);
  await user.click(screen.getByRole("button", { name: "Send message" }));
}

function emit(payload: Omit<AgentRuntimeEvent, "protocolVersion" | "sessionId" | "runId">) {
  act(() => {
    mocks.runtimeListener?.({
      payload: {
        protocolVersion: 1,
        sessionId: homeSession.id,
        runId: "home-run",
        ...payload,
      } as AgentRuntimeEvent,
    });
  });
}

describe("Home conversation on the default CLI engine", () => {
  beforeEach(() => {
    resetAgentSessionDraftsForTests();
    resetCurrentDataPartitionForTests();
    window.localStorage.clear();
    mocks.runtimeListener = undefined;
    mocks.invoke.mockReset();
    mockInvoke();
  });

  it("keeps Home sandboxed without a separate approval control", async () => {
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await screen.findByRole("textbox", { name: "Message Clovy" });
    expect(screen.queryByRole("combobox", { name: "Tool approval" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sandboxed" })).not.toBeInTheDocument();
    expect(calls("set_agent_session_approval_mode")).toHaveLength(0);
  });

  it("runs a message from an old Home session as a turn of the default CLI", async () => {
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await send("eae");

    await waitFor(() => expect(calls("start_agent_run")).toHaveLength(1));
    expect(calls("start_agent_run")[0][1]).toEqual({
      request: expect.objectContaining({
        sessionId: homeSession.id,
        prompt: "eae",
        model: CLAUDE,
        safetyMode: "sandboxed",
        attachments: [],
      }),
    });
    expect(calls("create_agent_session")).toHaveLength(0);
    expect(calls("clovy_home_chat")).toHaveLength(0);
    expect(calls("june_home_chat")).toHaveLength(0);
    expect(await screen.findByText("eae")).toBeVisible();
  });

  it("creates the Home session on the default CLI before its first turn", async () => {
    const created = { ...homeSession, id: "new-home", model: CLAUDE };
    mockInvoke({
      list_agent_sessions: () => [],
      create_agent_session: () => created,
      start_agent_run: () => ({ id: "home-run", sessionId: created.id, status: "running" }),
    });
    const onHomeSessionCreated = vi.fn();
    render(<AgentWorkspace homeMode onHomeSessionCreated={onHomeSessionCreated} />);
    await send("eae");

    await waitFor(() => expect(calls("start_agent_run")).toHaveLength(1));
    expect(calls("create_agent_session")[0][1]).toEqual({
      request: expect.objectContaining({ title: "Home", model: CLAUDE, safetyMode: "sandboxed" }),
    });
    expect(calls("start_agent_run")[0][1]).toEqual({
      request: expect.objectContaining({ sessionId: "new-home", model: CLAUDE }),
    });
    expect(onHomeSessionCreated).toHaveBeenCalledWith("new-home");
  });

  it("runs on Codex when only Codex is installed", async () => {
    mockInvoke({
      chat_default_engine: () => ({ modelId: CODEX, cli: "codex", legacyFallback: false }),
    });
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await send("eae");

    await waitFor(() =>
      expect(calls("start_agent_run")[0]?.[1]).toEqual({
        request: expect.objectContaining({ model: CODEX }),
      }),
    );
  });

  it("streams the CLI reply into the Home conversation", async () => {
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await send("eae");
    await waitFor(() => expect(calls("start_agent_run")).toHaveLength(1));

    emit({
      eventId: "started",
      sequence: 1,
      method: "run.started",
      data: { model: CLAUDE },
    } as never);
    emit({
      eventId: "delta",
      sequence: 2,
      method: "message.delta",
      data: {
        itemSequence: 2,
        itemId: "assistant:home-run",
        role: "assistant",
        delta: "Fala, tudo certo?",
        createdAt: new Date().toISOString(),
      },
    } as never);

    // Streaming text is split into animated words.
    await waitFor(() =>
      expect(screen.getByRole("main", { name: "Home conversation" })).toHaveTextContent(
        "Fala, tudo certo?",
      ),
    );
  });

  it("shows a CLI approval card in the Home conversation", async () => {
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await send("Apague o arquivo velho");
    await waitFor(() => expect(calls("start_agent_run")).toHaveLength(1));

    emit({
      eventId: "approval",
      sequence: 2,
      method: "interruption.requested",
      data: {
        itemSequence: 2,
        itemId: "approval-item",
        interruption: {
          id: "approval-1",
          kind: "approval",
          sessionId: homeSession.id,
          runId: "home-run",
          status: "pending",
          createdAt: new Date().toISOString(),
          toolName: "Bash",
          title: "Approval required",
          description: "Claude Code wants to run rm old.txt.",
          command: "rm old.txt",
          allowAlways: false,
        },
      },
    } as never);

    expect(await screen.findByText("Claude Code wants to run rm old.txt.")).toBeVisible();
    expect(calls("create_agent_session")).toHaveLength(0);
  });

  it("keeps the message in the composer and shows the CLI error when no CLI is installed", async () => {
    mockInvoke({
      chat_default_engine: () => ({ modelId: null, cli: null, legacyFallback: true }),
      start_agent_run: () =>
        Promise.reject({
          code: "llm_cli_not_installed",
          message: "Claude Code is not installed on this computer.",
        }),
    });
    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await send("eae");

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Claude Code is not installed on this computer.",
    );
    expect(calls("start_agent_run")[0][1]).toEqual({
      request: expect.objectContaining({ model: CLAUDE }),
    });
    expect(screen.getByRole("textbox", { name: "Message Clovy" })).toHaveTextContent("eae");
  });

  it("shows a failed CLI turn in the conversation with a retry", async () => {
    mockInvoke({
      list_agent_items: () => [
        {
          id: "message-1",
          sessionId: homeSession.id,
          runId: "home-run",
          sequence: 0,
          createdAt: "2026-10-17T10:00:00.000Z",
          kind: "message",
          role: "user",
          text: "eae",
          status: "complete",
        },
        {
          id: "error-1",
          sessionId: homeSession.id,
          runId: "home-run",
          sequence: 1,
          createdAt: "2026-10-17T10:00:01.000Z",
          kind: "error",
          message: "Claude Code failed: exit status 1",
          retryable: true,
        },
      ],
      retry_agent_run: () => ({ id: "retry", sessionId: homeSession.id, status: "running" }),
    });
    render(<AgentWorkspace homeMode initialSession={homeSession} />);

    // The same notice a CLI session shows for a failed turn.
    expect(
      await screen.findByText("The model service could not finish this request."),
    ).toBeVisible();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("retry_agent_run", { runId: "home-run" }),
    );
  });

  it("imports the old Home turns once and keeps their task cards", async () => {
    const directTurns = [
      {
        id: "home:direct:user:1",
        role: "user",
        createdAt: "2026-10-01T09:00:00.000Z",
        status: "complete",
        parts: [{ type: "text", text: "Research wines", status: "complete" }],
      },
      {
        id: "home:direct:assistant:1",
        role: "assistant",
        createdAt: "2026-10-01T09:00:01.000Z",
        status: "complete",
        parts: [
          {
            type: "tool",
            id: "direct:1",
            name: "clovy_home_start_task",
            text: "",
            status: "complete",
          },
        ],
      },
      "unreadable",
    ];
    const handoffs = [
      {
        id: "home-task-direct:1",
        title: "Wine research",
        prompt: "Research wines.",
        status: "running",
        storedSessionId: "focused-wine",
      },
    ];
    const directStorage = JSON.stringify({ [homeSession.id]: directTurns });
    const handoffStorage = JSON.stringify({ [homeSession.id]: handoffs });
    window.localStorage.setItem("clovy:home:direct-turns:v1", directStorage);
    window.localStorage.setItem("clovy:home:task-handoffs:v1", handoffStorage);
    let imported = false;
    mockInvoke({
      import_legacy_home_turns: () => {
        const result = { imported: imported ? 0 : 1, skipped: 1, alreadyImported: imported };
        imported = true;
        return result;
      },
      list_agent_items: () =>
        imported
          ? [
              {
                id: "imported-1",
                sessionId: homeSession.id,
                sequence: 0,
                createdAt: "2026-10-01T09:00:00.000Z",
                kind: "message",
                role: "user",
                text: "Research wines",
                status: "complete",
              },
            ]
          : [],
    });
    const onOpenHomeTaskSession = vi.fn();
    const first = render(
      <AgentWorkspace
        homeMode
        initialSession={homeSession}
        onOpenHomeTaskSession={onOpenHomeTaskSession}
      />,
    );

    await waitFor(() => expect(calls("import_legacy_home_turns")).toHaveLength(1));
    expect(calls("import_legacy_home_turns")[0][1]).toEqual({
      request: { sessionId: homeSession.id, turns: directTurns },
    });
    expect(await screen.findByText("Research wines")).toBeVisible();
    await userEvent.setup().click(await screen.findByRole("button", { name: "Open session" }));
    expect(onOpenHomeTaskSession).toHaveBeenCalledWith("focused-wine", "Wine research");
    first.unmount();

    render(<AgentWorkspace homeMode initialSession={homeSession} />);
    await waitFor(() => expect(calls("import_legacy_home_turns")).toHaveLength(2));
    expect(await screen.findByText("Research wines")).toBeVisible();
    expect(screen.getAllByText("Research wines")).toHaveLength(1);
    // The old local copy stays as recovery.
    expect(window.localStorage.getItem("clovy:home:direct-turns:v1")).toBe(directStorage);
    expect(JSON.parse(window.localStorage.getItem("clovy:home:task-handoffs:v1") ?? "{}")).toEqual(
      JSON.parse(handoffStorage),
    );
  });
});
