import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentSessionDto } from "../lib/agent-runtime-contract";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage?: (event: unknown) => void;
  },
  convertFileSrc: (path: string) => path,
  invoke: mocks.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({ emit: vi.fn(), listen: vi.fn(async () => vi.fn()) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { AgentWorkspace } from "../components/agent/AgentWorkspace";
import { AGENT_NEW_SESSION_EVENT } from "../lib/agent-events";
import { resetAgentSessionDraftsForTests } from "../lib/agent-session-drafts";
import { resetCurrentDataPartitionForTests } from "../lib/data-partition";

const model = "__clovy_cli_engine__:claude";
function fixture(initial?: AgentSessionDto) {
  let session = initial;
  mocks.invoke.mockImplementation(async (command: string) => {
    switch (command) {
      case "chat_default_engine":
        return { modelId: model, cli: "claude" };
      case "chat_engine_catalog":
        return { clis: [{ id: "claude", name: "Claude Code", installed: true, modelId: model }] };
      case "list_agent_sessions":
        return session ? [session] : [];
      case "get_agent_session":
        return session;
      case "get_latest_agent_run":
        return null;
      case "list_agent_items":
      case "list_agent_artifacts":
      case "list_agent_skills":
        return [];
      case "list_provider_models":
        return { models: [] };
      case "create_agent_session":
        session = {
          id: "new-session",
          title: "Fixture",
          status: "idle",
          model,
          safetyMode: "sandboxed",
          workspacePath: "/tmp/fixture",
          source: "user",
          createdAt: "2026-01-01",
          updatedAt: "2026-01-01",
        };
        return session;
      case "start_agent_run":
        return { id: "run-1", sessionId: session?.id, status: "running", model };
      default:
        return undefined;
    }
  });
}

describe("one runtime mode control", () => {
  beforeEach(() => {
    mocks.invoke.mockReset();
    resetAgentSessionDraftsForTests();
    resetCurrentDataPartitionForTests();
    window.localStorage.clear();
  });

  it.each([
    false,
    true,
  ])("starts the %s Home draft without a separate approval setting", async (homeMode) => {
    fixture();
    const user = userEvent.setup();
    render(<AgentWorkspace homeMode={homeMode} />);
    const editor = await screen.findByRole("textbox", { name: "Message Clovy" });
    expect(screen.queryByRole("combobox", { name: "Tool approval" })).not.toBeInTheDocument();
    if (homeMode)
      expect(screen.queryByRole("button", { name: "Sandboxed" })).not.toBeInTheDocument();
    else expect(screen.getByRole("button", { name: "Sandboxed" })).toBeVisible();
    await user.type(editor, "First message");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke.mock.calls.some(([command]) => command === "start_agent_run")).toBe(true),
    );
    const request = mocks.invoke.mock.calls.find(([command]) => command === "start_agent_run")?.[1]
      .request;
    expect(request.safetyMode).toBe("sandboxed");
    expect(request).not.toHaveProperty("approvalMode");
    expect(
      mocks.invoke.mock.calls.some(([command]) => command === "set_agent_session_approval_mode"),
    ).toBe(false);
    expect(screen.queryByRole("button", { name: "Sandboxed" })).not.toBeInTheDocument();
  });

  it("submits unrestricted through the single mode choice", async () => {
    fixture();
    const user = userEvent.setup();
    render(<AgentWorkspace />);
    await user.click(await screen.findByRole("button", { name: "Sandboxed" }));
    await user.click(screen.getByRole("menuitemradio", { name: /^Unrestricted/ }));
    await user.click(screen.getByRole("button", { name: "Turn on unrestricted" }));
    await user.type(screen.getByRole("textbox", { name: "Message Clovy" }), "Unrestricted message");
    await user.click(screen.getByRole("button", { name: "Send message" }));
    await waitFor(() =>
      expect(mocks.invoke.mock.calls.some(([command]) => command === "start_agent_run")).toBe(true),
    );
    expect(
      mocks.invoke.mock.calls.find(([command]) => command === "start_agent_run")?.[1].request
        .safetyMode,
    ).toBe("unrestricted");
    expect(
      mocks.invoke.mock.calls.some(([command]) => command === "set_agent_session_approval_mode"),
    ).toBe(false);
  });

  it("keeps the single mode choice and composer controls when opening a new conversation", async () => {
    fixture();
    render(<AgentWorkspace />);
    await screen.findByRole("button", { name: "Sandboxed" });
    expect(screen.getByRole("button", { name: "Add files or notes" })).toBeVisible();
    expect(screen.getByRole("button", { name: /Chat engine:/ })).toBeVisible();
    expect(screen.getByRole("button", { name: "Dictate" })).toBeVisible();
    act(() => window.dispatchEvent(new CustomEvent(AGENT_NEW_SESSION_EVENT)));
    expect(screen.getByRole("button", { name: "Sandboxed" })).toBeVisible();
    expect(screen.queryByRole("combobox", { name: "Tool approval" })).not.toBeInTheDocument();
  });
});
