import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

import { ChatEnginePicker } from "../components/agent/composer/ChatEnginePicker";
import appCss from "../styles/app.css?raw";
import { applyInterfaceLocale, resetInterfaceLocaleForTests } from "../i18n/locale";
import type { ChatEngineCatalog } from "../lib/chat-engine";

const sampleCatalog: ChatEngineCatalog = {
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
    {
      id: "pi",
      name: "Pi",
      installed: false,
      reason: "`pi` was not found in your login shell PATH.",
      modelId: "__clovy_cli_engine__:pi",
    },
  ],
};

describe("ChatEnginePicker", () => {
  beforeEach(() => {
    resetInterfaceLocaleForTests();
    applyInterfaceLocale("en");
    mocks.invoke.mockReset();
    mocks.invoke.mockImplementation((cmd: string) => {
      if (cmd === "chat_engine_catalog") {
        return Promise.resolve(sampleCatalog);
      }
      return Promise.resolve(null);
    });
  });

  afterEach(() => {
    cleanup();
    resetInterfaceLocaleForTests();
    applyInterfaceLocale("en");
  });

  it("lists only CLI engines for historical native sessions", async () => {
    const user = userEvent.setup();
    const setModel = vi.fn();

    render(<ChatEnginePicker model="auto" setModel={setModel} catalog={sampleCatalog} />);

    const trigger = screen.getByRole("button", { name: /Chat engine/i });
    expect(trigger).toHaveTextContent("Claude Code");

    await user.click(trigger);

    expect(screen.getByRole("menu", { name: "Chat engine" })).toBeInTheDocument();
    expect(screen.queryByRole("menuitemradio", { name: /^Clovy/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("menuitemradio", { name: /^Local Ollama/ })).not.toBeInTheDocument();
    expect(screen.getByRole("menuitemradio", { name: /^Claude Code/ })).toBeInTheDocument();
    expect(screen.getByRole("menuitemradio", { name: /^Codex/ })).toBeInTheDocument();
    expect(screen.getByRole("menuitemradio", { name: /Pi/ })).toBeInTheDocument();
  });

  it("marks Claude Code as the default without a legacy engine", async () => {
    const user = userEvent.setup();
    render(
      <ChatEnginePicker
        model="__clovy_cli_engine__:claude"
        setModel={vi.fn()}
        catalog={sampleCatalog}
      />,
    );

    await user.click(screen.getByRole("button", { name: /Chat engine/i }));
    const rows = screen.getAllByRole("menuitemradio");
    expect(
      rows.map((row) => row.querySelector(".agent-composer-model-choice-copy")?.textContent),
    ).toEqual(["Claude CodeDefault", "Codex", "PiNot found on your PATH."]);
    expect(screen.queryByText("Legacy")).not.toBeInTheDocument();
  });

  it("marks Codex as the default when Claude Code is not installed", async () => {
    const user = userEvent.setup();
    const catalog: ChatEngineCatalog = {
      ...sampleCatalog,
      clis: sampleCatalog.clis.map((cli) =>
        cli.id === "claude" ? { ...cli, installed: false, reason: "Not found in PATH" } : cli,
      ),
    };
    render(<ChatEnginePicker model="auto" setModel={vi.fn()} catalog={catalog} />);

    await user.click(screen.getByRole("button", { name: /Chat engine/i }));
    expect(screen.getByRole("menuitemradio", { name: /^Codex/ })).toHaveTextContent("Default");
    expect(screen.getByRole("menuitemradio", { name: /^Claude Code/ })).not.toHaveTextContent(
      "Default",
    );
  });

  it("disables a not-installed CLI with a translated reason, not the backend's", async () => {
    const user = userEvent.setup();
    const setModel = vi.fn();

    render(<ChatEnginePicker model="auto" setModel={setModel} catalog={sampleCatalog} />);

    await user.click(screen.getByRole("button", { name: /Chat engine/i }));

    const piOption = screen.getByRole("menuitemradio", { name: /Pi/ });
    expect(piOption).toBeDisabled();
    expect(piOption).toHaveTextContent("Not found on your PATH.");
    expect(piOption).not.toHaveTextContent("login shell");
  });

  it("shows the not-installed reason in the interface language", async () => {
    applyInterfaceLocale("pt-BR");
    const user = userEvent.setup();

    render(<ChatEnginePicker model="auto" setModel={vi.fn()} catalog={sampleCatalog} />);

    await user.click(screen.getByRole("button", { name: /Mecanismo de chat/i }));

    const piOption = screen.getByRole("menuitemradio", { name: /Pi/ });
    expect(piOption).toBeDisabled();
    expect(piOption).toHaveTextContent("Não encontrado no PATH da sua sessão.");
  });

  it("choosing a CLI calls setModel with __clovy_cli_engine__:<id>", async () => {
    const user = userEvent.setup();
    const setModel = vi.fn();

    render(<ChatEnginePicker model="auto" setModel={setModel} catalog={sampleCatalog} />);

    await user.click(screen.getByRole("button", { name: /Chat engine/i }));
    await user.click(screen.getByRole("menuitemradio", { name: /^Claude Code/ }));

    expect(setModel).toHaveBeenCalledWith("__clovy_cli_engine__:claude");
  });

  it("keeps an unavailable CLI's logo, name and reason on its own row", async () => {
    const user = userEvent.setup();
    const reason = (bin: string) => `\`${bin}\` was not found in your login shell PATH.`;
    const catalog: ChatEngineCatalog = {
      clis: [
        ...sampleCatalog.clis,
        {
          id: "cursor-agent",
          name: "Cursor Agent",
          installed: false,
          reason: reason("cursor-agent"),
          modelId: "__clovy_cli_engine__:cursor-agent",
        },
        {
          id: "copilot",
          name: "GitHub Copilot",
          installed: false,
          reason: reason("copilot"),
          modelId: "__clovy_cli_engine__:copilot",
        },
      ],
    };
    render(<ChatEnginePicker model="auto" setModel={vi.fn()} catalog={catalog} />);
    await user.click(screen.getByRole("button", { name: /Chat engine/i }));

    const cursor = screen.getByRole("menuitemradio", { name: /^Cursor Agent/ });
    const copilot = screen.getByRole("menuitemradio", { name: /^GitHub Copilot/ });
    expect(cursor).not.toContainElement(copilot);
    expect(cursor).not.toHaveTextContent("GitHub Copilot");
    for (const [row, name] of [
      [cursor, "Cursor Agent"],
      [copilot, "GitHub Copilot"],
    ] as const) {
      expect(row.querySelector(".agent-composer-model-row-logo svg")).not.toBeNull();
      expect(row.querySelector(".agent-composer-model-row-name")).toHaveTextContent(name);
      expect(row.querySelector(".agent-composer-model-choice-desc")).toHaveTextContent(
        "Not found on your PATH.",
      );
    }

    // jsdom has no layout: the stylesheet carries the contract. Engine rows
    // keep their natural height (a capped popover scrolls instead of
    // squeezing two-line rows into one), the copy takes the free width and
    // the reason wraps inside it.
    const rule = (selector: string) => {
      const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(appCss);
      return match?.[1] ?? "";
    };
    expect(rule(".agent-engine-popover")).toContain("overflow-y: auto;");
    expect(rule(".agent-engine-popover .agent-composer-model-menu")).toContain("flex: none;");
    expect(rule(".agent-engine-popover .agent-composer-model-row")).toContain("flex: none;");
    expect(rule(".agent-engine-popover .agent-composer-model-choice-copy")).toContain(
      "flex: 1 1 auto;",
    );
    expect(rule(".agent-engine-popover .agent-composer-model-choice-desc")).toContain(
      "overflow-wrap: anywhere;",
    );
  });

  it("shows the translated not-found reason instead of the backend text", async () => {
    applyInterfaceLocale("pt-BR");
    const user = userEvent.setup();
    const catalog: ChatEngineCatalog = {
      clis: [
        {
          id: "copilot",
          name: "GitHub Copilot",
          installed: false,
          reason: "`copilot` was not found in your login shell PATH.",
          modelId: "__clovy_cli_engine__:copilot",
        },
      ],
    };
    render(<ChatEnginePicker model="auto" setModel={vi.fn()} catalog={catalog} />);
    await user.click(screen.getByRole("button"));

    const copilot = screen.getByRole("menuitemradio", { name: /^GitHub Copilot/ });
    expect(copilot).toHaveTextContent("Não encontrado no PATH da sua sessão.");
    expect(copilot).not.toHaveTextContent("was not found");
  });
});
