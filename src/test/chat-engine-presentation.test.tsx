import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentSessionDto } from "../lib/agent-runtime-contract";
import type { ChatEngineCatalog } from "../lib/chat-engine";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage?: (event: unknown) => void;
  },
  convertFileSrc: vi.fn((path: string) => path),
  invoke: mocks.invoke,
}));

vi.mock("@tauri-apps/api/event", () => ({
  emit: vi.fn(),
  listen: vi.fn(async () => vi.fn()),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn() }));

import { AgentWorkspace } from "../components/agent/AgentWorkspace";
import { ChatEnginePicker } from "../components/agent/composer/ChatEnginePicker";
import { resetAgentSessionDraftsForTests } from "../lib/agent-session-drafts";
import { resetCurrentDataPartitionForTests } from "../lib/data-partition";

// The three tool states a CLI used to be announced with.
const catalog = {
  clis: [
    {
      id: "claude",
      name: "Claude Code",
      installed: true,
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
    { id: "pi", name: "Pi", installed: true, reason: null, modelId: "__clovy_cli_engine__:pi" },
  ],
} as ChatEngineCatalog;

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

function answer(command: string, sessions: AgentSessionDto[]) {
  if (command === "list_agent_sessions") return Promise.resolve(sessions);
  if (command === "get_agent_session") return Promise.resolve(sessions[0]);
  if (command === "list_agent_items") return Promise.resolve([]);
  if (command === "list_agent_artifacts") return Promise.resolve([]);
  if (command === "list_agent_skills") return Promise.resolve([]);
  if (command === "chat_engine_catalog") return Promise.resolve(catalog);
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
      ],
    });
  }
  return Promise.resolve(undefined);
}

describe("chat engine presentation", () => {
  beforeEach(() => {
    resetAgentSessionDraftsForTests();
    resetCurrentDataPartitionForTests();
    window.localStorage.clear();
    mocks.invoke.mockReset();
  });

  it.each([
    "claude",
    "codex",
    "agy",
  ])("selects the only installed engine %s and disables unavailable alternatives", async (only) => {
    const user = userEvent.setup();
    const setModel = vi.fn();
    const clis = [
      { id: "claude", name: "Claude Code" },
      { id: "codex", name: "Codex" },
      { id: "agy", name: "Antigravity" },
    ].map((cli) => ({
      ...cli,
      installed: cli.id === only,
      reason: null,
      modelId: `__clovy_cli_engine__:${cli.id}`,
    }));
    mocks.invoke.mockResolvedValue({ clis });
    render(<ChatEnginePicker model="historical-model" setModel={setModel} catalog={{ clis }} />);
    const chosen = clis.find((cli) => cli.id === only);
    await user.click(screen.getByRole("button", { name: `Chat engine: ${chosen?.name}` }));
    for (const cli of clis) {
      const row = screen.getByRole("menuitemradio", { name: new RegExp(`^${cli.name}`) });
      if (cli.id !== only) {
        expect(row).toBeDisabled();
        await user.click(row);
        expect(setModel).not.toHaveBeenCalled();
      } else {
        expect(row).toHaveAttribute("aria-checked", "true");
        expect(row).toHaveTextContent("Default");
      }
    }
    await user.click(screen.getByRole("menuitemradio", { name: new RegExp(`^${chosen?.name}`) }));
    expect(setModel).toHaveBeenCalledExactlyOnceWith(`__clovy_cli_engine__:${only}`);
  });

  it("switches among all three installed engines without replacing an explicit choice", async () => {
    const user = userEvent.setup();
    const setModel = vi.fn();
    const clis = [
      { id: "claude", name: "Claude Code" },
      { id: "codex", name: "Codex" },
      { id: "agy", name: "Antigravity" },
    ].map((cli) => ({
      ...cli,
      installed: true,
      reason: null,
      modelId: `__clovy_cli_engine__:${cli.id}`,
    }));
    mocks.invoke.mockResolvedValue({ clis });
    const { rerender } = render(
      <ChatEnginePicker model="__clovy_cli_engine__:agy" setModel={setModel} catalog={{ clis }} />,
    );
    for (const cli of clis) {
      await user.click(screen.getByRole("button", { name: /Chat engine:/ }));
      await user.click(screen.getByRole("menuitemradio", { name: new RegExp(`^${cli.name}`) }));
      expect(setModel).toHaveBeenLastCalledWith(cli.modelId);
      rerender(<ChatEnginePicker model={cli.modelId} setModel={setModel} catalog={{ clis }} />);
      expect(screen.getByRole("button", { name: `Chat engine: ${cli.name}` })).toBeInTheDocument();
    }
    expect(setModel).toHaveBeenCalledTimes(3);
  });

  it("keeps the mode control exclusive to new conversations", async () => {
    mocks.invoke.mockImplementation((command: string) => answer(command, [session]));
    render(<AgentWorkspace initialSession={session} />);
    await screen.findByRole("textbox", { name: "Message Clovy" });
    expect(screen.queryByRole("combobox", { name: "Tool approval" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Sandboxed" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Unrestricted" })).not.toBeInTheDocument();
  });

  it("shows the hero suggestions without a privacy footnote", async () => {
    mocks.invoke.mockImplementation((command: string) => answer(command, []));
    const { container } = render(<AgentWorkspace />);

    await waitFor(() => expect(container.querySelector(".agent-hero-suggestions")).not.toBeNull());
    expect(container.querySelector(".agent-hero-footnote")).toBeNull();
    expect(screen.queryByText(/Clovy runs locally/i)).not.toBeInTheDocument();
  });

  it.each([
    "claude",
    "codex",
    "pi",
  ])("shows no engine notice in the composer for %s", async (cli) => {
    const cliSession = { ...session, model: `__clovy_cli_engine__:${cli}` };
    mocks.invoke.mockImplementation((command: string) => answer(command, [cliSession]));
    const { container } = render(<AgentWorkspace initialSession={cliSession} />);

    await waitFor(() =>
      expect(screen.getByRole("button", { name: /Chat engine/i })).toBeInTheDocument(),
    );
    expect(container.querySelector(".agent-composer-notice")).toBeNull();
    expect(screen.queryByText(/Clovy tools/i)).not.toBeInTheDocument();
  });

  it.each([
    "claude",
    "codex",
    "pi",
  ])("shows no engine notice in the picker menu for %s", async (cli) => {
    const user = userEvent.setup();
    const { container } = render(
      <ChatEnginePicker
        model={`__clovy_cli_engine__:${cli}`}
        setModel={vi.fn()}
        catalog={catalog}
      />,
    );

    expect(container.querySelector(".agent-composer-notice")).toBeNull();
    await user.click(screen.getByRole("button", { name: /Chat engine/i }));
    expect(document.querySelector(".agent-composer-notice")).toBeNull();
    expect(screen.queryByText(/Clovy tools|Clovy's tools/i)).not.toBeInTheDocument();
  });

  it.each([
    {
      name: "Claude session",
      sessionModel: "__clovy_cli_engine__:claude",
      expected: "Claude Code",
    },
    {
      name: "Codex run after a Claude session",
      sessionModel: "__clovy_cli_engine__:claude",
      runModel: "__clovy_cli_engine__:codex",
      expected: "Codex",
    },
    { name: "catalog engine name", sessionModel: "__clovy_cli_engine__:pi", expected: "Pi" },
    {
      name: "unavailable catalog",
      sessionModel: "__clovy_cli_engine__:claude",
      catalogUnavailable: true,
      expected: "Claude Code",
    },
    {
      name: "unavailable Codex catalog",
      sessionModel: "__clovy_cli_engine__:codex",
      catalogUnavailable: true,
      expected: "Codex",
    },
    {
      name: "historical model",
      sessionModel: "anthropic/historical-model",
      expected: "anthropic/historical-model",
    },
  ])("shows the engine name in Usage for $name", async ({
    sessionModel,
    runModel,
    catalogUnavailable,
    expected,
  }) => {
    const user = userEvent.setup();
    const storedSession = { ...session, model: sessionModel };
    mocks.invoke.mockImplementation((command: string) => {
      if (command === "chat_engine_catalog" && catalogUnavailable)
        return Promise.reject(new Error("Catalog unavailable"));
      if (command === "get_latest_agent_run" && runModel)
        return Promise.resolve({
          id: "latest-run",
          sessionId: session.id,
          status: "completed",
          model: runModel,
        });
      return answer(command, [storedSession]);
    });
    render(<AgentWorkspace initialSession={storedSession} />);
    await user.click(screen.getByRole("button", { name: "Session actions" }));
    await user.click(screen.getByRole("menuitem", { name: "Usage" }));
    const usagePanel = screen.getByRole("dialog", { name: "Usage" });
    expect(await within(usagePanel).findByText(expected, { exact: true })).toBeInTheDocument();
    expect(usagePanel).not.toHaveTextContent("__clovy_cli_engine__:");
  });

  it("draws each engine row with its brand mark and keeps the row name first", async () => {
    const user = userEvent.setup();
    render(
      <ChatEnginePicker
        model="__clovy_cli_engine__:claude"
        setModel={vi.fn()}
        catalog={{
          ...catalog,
          clis: [
            ...catalog.clis,
            {
              id: "agy",
              name: "Antigravity",
              installed: false,
              reason: null,
              modelId: "__clovy_cli_engine__:agy",
            },
            {
              id: "cursor-agent",
              name: "Cursor Agent",
              installed: false,
              reason: null,
              modelId: "__clovy_cli_engine__:cursor-agent",
            },
            {
              id: "copilot",
              name: "GitHub Copilot",
              installed: false,
              reason: null,
              modelId: "__clovy_cli_engine__:copilot",
            },
          ],
        }}
      />,
    );
    await user.click(screen.getByRole("button", { name: /Chat engine/i }));

    for (const [name, brand] of [
      ["Claude Code", "claudecode"],
      ["Codex", "codex"],
      ["Pi", "pi"],
      ["Antigravity", "antigravity"],
      ["Cursor Agent", "cursor"],
      ["GitHub Copilot", "githubcopilot"],
    ]) {
      const row = screen.getByRole("menuitemradio", { name: new RegExp(`^${name}`) });
      expect(row.querySelector(`[data-brand="${brand}"]`), name).not.toBeNull();
    }
  });
});
