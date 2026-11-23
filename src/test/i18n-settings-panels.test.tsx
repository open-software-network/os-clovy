import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { applyInterfaceLocale } from "../i18n/locale";
import type { AgentMcpServerDto } from "../lib/agent-mcp";
import type { MemoryDto } from "../lib/tauri";

const mcp = vi.hoisted(() => ({
  list: vi.fn(),
  test: vi.fn(),
}));

const tauri = vi.hoisted(() => ({
  listMemories: vi.fn(),
  memorySettings: vi.fn(),
}));

vi.mock("../lib/agent-mcp", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/agent-mcp")>()),
  listAgentMcpServers: mcp.list,
  testAgentMcpServer: mcp.test,
}));

vi.mock("../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/tauri")>()),
  listMemories: tauri.listMemories,
  memorySettings: tauri.memorySettings,
}));

import { AgentMcpServersSection } from "../components/settings/AgentMcpServersSection";
import { MemorySettingsSection } from "../components/settings/MemorySettingsSection";
import { contextLabel, modelSpecEntries } from "../components/settings/ModelPickerDialog";

const server: AgentMcpServerDto = {
  id: "mcp-tasks",
  name: "Tasks",
  enabled: true,
  transport: "stdio",
  command: "node",
  args: ["server.js"],
  metadata: {},
  toolVisibility: { include: [], exclude: [] },
  safety: {
    requiresApproval: false,
    allowSandboxed: true,
    timeoutMs: 30_000,
    maxOutputBytes: 1_048_576,
    approvalTools: [],
  },
};

const memory: MemoryDto = {
  id: "m1",
  content: "Prefers metric units",
  source: "agent",
  createdAt: "2026-07-04T00:00:00Z",
  updatedAt: "2026-07-04T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  mcp.list.mockResolvedValue([server]);
  mcp.test.mockResolvedValue([
    { name: "list_tasks", description: "", inputSchema: {} },
    { name: "add_task", description: "", inputSchema: {} },
  ]);
  tauri.listMemories.mockResolvedValue([memory]);
  tauri.memorySettings.mockResolvedValue({ enabled: true });
});

afterEach(() => {
  cleanup();
  applyInterfaceLocale("en");
});

describe("settings panels in pt-BR", () => {
  it("renders the MCP servers section in Portuguese with interpolated labels and plurals", async () => {
    applyInterfaceLocale("pt-BR");
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);

    expect(await screen.findByText("Tasks")).toBeInTheDocument();
    expect(screen.getByText("Servidores MCP personalizados")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Configurar Tasks" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Tasks ativado" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Testar Tasks" }));
    expect(await screen.findByText(/2 ferramentas disponíveis/)).toBeInTheDocument();
  });

  it("renders the memory manager in Portuguese and switches back to English live", async () => {
    applyInterfaceLocale("pt-BR");
    render(<MemorySettingsSection folders={[]} />);

    expect(await screen.findByText("Prefers metric units")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Memória" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Deixar o Clovy lembrar das coisas" })).toBeEnabled();
    expect(screen.getByRole("searchbox", { name: "Buscar memórias" })).toBeInTheDocument();
    expect(screen.getByText("Adicionada pelo Clovy")).toBeInTheDocument();

    act(() => applyInterfaceLocale("en"));
    await waitFor(() => expect(screen.getByText("Added by Clovy")).toBeInTheDocument());
    expect(screen.getByRole("searchbox", { name: "Search memories" })).toBeInTheDocument();
  });

  it("formats model context and spec labels with Portuguese words and digits", () => {
    const model = {
      provider: "openrouter",
      id: "test-model",
      name: "Test model",
      modelType: "text",
      traits: [],
      capabilities: [],
      contextTokens: 1_500_000,
    };
    expect(contextLabel(model)).toBe("1.5M context");

    applyInterfaceLocale("pt-BR");
    expect(contextLabel(model)).toBe("Contexto de 1,5M");
    expect(modelSpecEntries(model)).toContainEqual({ label: "Contexto", value: "1,5M tokens" });
  });
});
