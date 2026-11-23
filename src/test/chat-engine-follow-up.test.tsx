import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
import { resetAgentSessionDraftsForTests } from "../lib/agent-session-drafts";
import { resetCurrentDataPartitionForTests } from "../lib/data-partition";

const session: AgentSessionDto = {
  id: "session-cli-1",
  title: "Claude Code session",
  status: "idle",
  model: "__clovy_cli_engine__:claude",
  safetyMode: "sandboxed",
  workspacePath: "/tmp/session-cli-1",
  source: "user",
  createdAt: "2026-07-22T12:00:00Z",
  updatedAt: "2026-07-22T12:00:00Z",
};

describe("CLI chat engine follow-up queuing", () => {
  beforeEach(() => {
    resetAgentSessionDraftsForTests();
    resetCurrentDataPartitionForTests();
    window.localStorage.clear();
    mocks.runtimeListener = undefined;
    mocks.invoke.mockReset();
    mocks.openDialog.mockReset();
    mocks.invoke.mockImplementation((command: string) => {
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
            text: "Hello from Claude Code",
            status: "complete",
          },
        ]);
      }
      if (command === "list_agent_artifacts") return Promise.resolve([]);
      if (command === "list_agent_skills") return Promise.resolve([]);
      if (command === "list_provider_models") {
        return Promise.resolve({
          mode: "generation",
          selectedModel: "__clovy_cli_engine__:claude",
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
          ],
        });
      }
      if (command === "chat_engine_catalog") {
        return Promise.resolve({
          clis: [
            {
              id: "claude",
              name: "Claude Code",
              installed: true,
              reason: null,
              modelId: "__clovy_cli_engine__:claude",
            },
          ],
          endpoints: [],
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
      if (command === "start_agent_run") {
        return Promise.resolve({
          id: "run-cli-1",
          sessionId: session.id,
          status: "running",
          model: "__clovy_cli_engine__:claude",
        });
      }
      if (command === "steer_agent_run") {
        return Promise.resolve({ accepted: false, reason: "cli_engine" });
      }
      return Promise.resolve(undefined);
    });
  });

  it("queues a follow-up without sidecar IPC and sends it as the next CLI run", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);

    try {
      const user = userEvent.setup();
      render(<AgentWorkspace initialSession={session} />);
      await screen.findByText("Hello from Claude Code");

      let composer = screen.getByRole("textbox", { name: "Message Clovy" });
      await user.type(composer, "First task for Claude");
      await user.click(screen.getByRole("button", { name: "Send message" }));
      await screen.findByRole("button", { name: "Stop Clovy" });

      composer = screen.getByRole("textbox", { name: "Message Clovy" });
      composer.textContent = "Second task after completion";
      fireEvent.input(composer);
      await user.click(await screen.findByRole("button", { name: "Queue next turn" }));

      expect(await screen.findByText("Queued follow-up")).toBeVisible();
      expect(warn).not.toHaveBeenCalled();
      expect(mocks.invoke.mock.calls.some(([command]) => command === "steer_agent_run")).toBe(
        false,
      );

      act(() => {
        mocks.runtimeListener?.({
          payload: {
            protocolVersion: 1,
            eventId: "event-completed-after-cli-steer",
            sessionId: session.id,
            runId: "run-cli-1",
            sequence: 2,
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
            prompt: "Second task after completion",
            model: "__clovy_cli_engine__:claude",
          }),
        });
      });
    } finally {
      warn.mockRestore();
    }
  });
});
