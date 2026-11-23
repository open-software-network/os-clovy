import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentMcpServersSection } from "../components/settings/AgentMcpServersSection";
import type { AgentMcpServerDto } from "../lib/agent-mcp";
import { setInterfaceLocale } from "../i18n";

const mocks = vi.hoisted(() => ({
  list: vi.fn(),
  create: vi.fn(),
  update: vi.fn(),
  remove: vi.fn(),
  test: vi.fn(),
  connectOauth: vi.fn(),
  cancelOauth: vi.fn(),
  disconnectOauth: vi.fn(),
}));

vi.mock("../lib/agent-mcp", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/agent-mcp")>()),
  listAgentMcpServers: mocks.list,
  createAgentMcpServer: mocks.create,
  updateAgentMcpServer: mocks.update,
  deleteAgentMcpServer: mocks.remove,
  testAgentMcpServer: mocks.test,
  connectAgentMcpOauth: mocks.connectOauth,
  cancelAgentMcpOauth: mocks.cancelOauth,
  disconnectAgentMcpOauth: mocks.disconnectOauth,
}));

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

describe("AgentMcpServersSection", () => {
  beforeEach(() => {
    vi.resetAllMocks();
    mocks.list.mockResolvedValue([server]);
    mocks.update.mockImplementation(async (input) => input);
    mocks.remove.mockResolvedValue({ localDeleted: true, remoteRevoked: null });
    mocks.test.mockResolvedValue([{ name: "list_tasks", description: "", inputSchema: {} }]);
    mocks.connectOauth.mockImplementation(async () => ({
      ...server,
      id: "legacy-oauth",
      name: "Legacy OAuth tools",
      transport: "streamable_http",
      command: undefined,
      url: "https://example.test/mcp",
      metadata: { auth: "oauth", oauthConnected: true },
    }));
  });

  it("hydrates, tests, toggles, and removes persisted servers", async () => {
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);

    expect(await screen.findByText("Tasks")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test Tasks" }));
    expect(await screen.findByText(/1 tool available/)).toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "Tasks enabled" }));
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith(
        expect.objectContaining({ id: "mcp-tasks", enabled: false }),
      ),
    );

    await user.click(screen.getByRole("button", { name: "Delete Tasks" }));
    await user.click(screen.getByRole("button", { name: "Delete server" }));
    await waitFor(() => expect(mocks.remove).toHaveBeenCalledWith("mcp-tasks"));
    expect(screen.queryByText("Tasks")).not.toBeInTheDocument();
  });

  it("classifies an explicit health check without treating every failure as a login error", async () => {
    mocks.test
      .mockRejectedValueOnce({ code: "agent_mcp_reconnect_required", message: "Grant expired" })
      .mockRejectedValueOnce({ code: "agent_mcp_unauthorized", message: "Permission denied" });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Test Tasks" }));
    expect(await screen.findByText(/Last check: reconnect required/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test Tasks" }));
    expect(await screen.findByText(/Last check: unavailable/)).toHaveTextContent(
      "Permission denied",
    );
  });

  it("removes local configuration and reports remote revocation that needs provider action", async () => {
    mocks.remove.mockResolvedValue({ localDeleted: true, remoteRevoked: false });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Delete Tasks" }));
    await user.click(screen.getByRole("button", { name: "Delete server" }));
    expect(
      await screen.findByText(/Remote revocation failed\. Revoke the grant at the provider/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Tasks")).not.toBeInTheDocument();
    expect(mocks.remove).toHaveBeenCalledTimes(1);
  });

  it("reloads the disabled state when implicit OAuth disconnect reports incomplete cleanup", async () => {
    const connected = {
      ...server,
      transport: "streamable_http" as const,
      url: "https://example.test/mcp",
      metadata: { auth: "oauth", oauthConnected: true },
    };
    mocks.list
      .mockResolvedValueOnce([connected])
      .mockResolvedValue([
        { ...connected, enabled: false, metadata: { auth: "oauth", oauthCleanupPending: true } },
      ]);
    mocks.update.mockRejectedValue({
      code: "agent_mcp_disconnect_incomplete",
      message: "Credential cleanup pending",
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("switch", { name: "Tasks enabled" }));
    expect(await screen.findByText("Credential cleanup pending")).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Tasks enabled" })).not.toBeChecked();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeInTheDocument();
  });

  it("shows the observed discovery cause and recovery without affecting another connection", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        diagnostic: {
          origin: "clovy_managed",
          server: "Tasks",
          phase: "discover",
          code: "agent_mcp_remote_error",
          remoteCode: -32601,
          state: "unavailable",
          action: "configure",
          message: "The server does not support tool discovery.",
        },
      },
      { ...server, id: "healthy", name: "Healthy connection" },
    ]);
    render(<AgentMcpServersSection />);
    expect(
      await screen.findByText(/The server does not support tool discovery/),
    ).toBeInTheDocument();
    expect(screen.getByText(/Discovery.*agent_mcp_remote_error.*-32601/)).toHaveTextContent(
      "Review server configuration",
    );
    expect(screen.getAllByText(/Managed by Clovy/)).toHaveLength(2);
    expect(screen.getByRole("switch", { name: "Healthy connection enabled" })).toBeChecked();
    expect(mocks.update).not.toHaveBeenCalled();
    expect(mocks.disconnectOauth).not.toHaveBeenCalled();
  });

  it("reconciles a failed reconnect and renders its phase and recovery", async () => {
    const oauth = {
      ...server,
      id: "oauth",
      name: "OAuth",
      transport: "streamable_http" as const,
      url: "https://example.test",
      metadata: { auth: "oauth", oauthConnected: true },
    };
    mocks.list
      .mockResolvedValueOnce([oauth])
      .mockResolvedValueOnce([{ ...oauth, enabled: false, metadata: { auth: "oauth" } }]);
    mocks.connectOauth.mockRejectedValue({
      code: "agent_mcp_reconnect_required",
      message: "The authorization grant was rejected.",
      details: {
        origin: "clovy_managed",
        server: "OAuth",
        phase: "connect",
        code: "agent_mcp_reconnect_required",
        state: "reconnect_required",
        action: "reconnect",
        message: "The authorization grant was rejected.",
      },
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Reconnect" }));
    expect(await screen.findByRole("button", { name: "Connect" })).toBeInTheDocument();
    expect(screen.getByText(/Last check: reconnect required/)).toBeInTheDocument();
    expect(screen.getByText(/Connection.*agent_mcp_reconnect_required/)).toHaveTextContent(
      "Reconnect this server",
    );
    expect(screen.getByRole("switch", { name: "OAuth enabled" })).not.toBeChecked();
  });

  it("clears a previous discovery failure only after a successful explicit check", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        diagnostic: {
          origin: "clovy_managed",
          server: "Tasks",
          phase: "discover",
          code: "agent_mcp_transport_failed",
          state: "unavailable",
          action: "check_connection",
          message: "The server could not be reached.",
        },
      },
    ]);
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    expect(await screen.findByText(/The server could not be reached/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Test Tasks" }));
    expect(await screen.findByText(/Last check: connected/)).toHaveTextContent("1 tool available");
    expect(screen.queryByText(/The server could not be reached/)).not.toBeInTheDocument();
    expect(screen.queryByText(/agent_mcp_transport_failed/)).not.toBeInTheDocument();
  });

  it("explains provider client rejection without substituting an API key", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        diagnostic: {
          origin: "clovy_managed",
          server: "Tasks",
          phase: "connect",
          code: "agent_mcp_oauth_provider_rejected",
          state: "unavailable",
          action: "check_provider_client_approval",
          message: "The provider rejected the Clovy OAuth client.",
        },
      },
    ]);
    render(<AgentMcpServersSection />);
    expect(
      await screen.findByText(/Confirm provider approval of the Clovy OAuth client/),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText(/API key/)).not.toBeInTheDocument();
  });

  it("keeps cancellation reachable while another server cannot replace the pending operation", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        id: "oauth",
        name: "OAuth",
        enabled: false,
        transport: "streamable_http",
        url: "https://example.test",
        metadata: { auth: "oauth" },
      },
      server,
    ]);
    let rejectConnect: (reason: Error) => void = () => {};
    mocks.connectOauth.mockImplementation(
      () =>
        new Promise((_resolve, reject) => {
          rejectConnect = reject;
        }),
    );
    mocks.cancelOauth.mockImplementation(async () => {
      rejectConnect(new Error("Connection cancelled"));
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Connect" }));
    expect(screen.getByRole("button", { name: "Test Tasks" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Configure Tasks" })).toBeDisabled();
    expect(screen.getByRole("switch", { name: "Tasks enabled" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText(/Connection cancelled/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Test Tasks" })).toBeEnabled();
  });

  it("sends secrets only through the secure create input", async () => {
    mocks.list.mockResolvedValue([]);
    mocks.create.mockImplementation(async (input) => ({
      ...server,
      ...input,
      id: "created",
    }));
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);

    await screen.findByText("No custom servers");
    await user.click(screen.getByRole("button", { name: "Add server" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Private tools");
    await user.type(within(dialog).getByLabelText("Command"), "node");
    fireEvent.change(within(dialog).getByLabelText("Environment variables (JSON)"), {
      target: { value: '{"TOKEN":"secret"}' },
    });
    await user.click(within(dialog).getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mocks.create).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Private tools",
          command: "node",
          secrets: { env: { TOKEN: "secret" }, headers: {} },
        }),
      ),
    );
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("creates an OAuth HTTP server disabled until browser sign-in", async () => {
    mocks.list.mockResolvedValue([]);
    mocks.create.mockImplementation(async (input) => ({
      ...server,
      ...input,
      id: "oauth-created",
    }));
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);

    await screen.findByText("No custom servers");
    await user.click(screen.getByRole("button", { name: "Add server" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "OAuth tools");
    await user.selectOptions(within(dialog).getByLabelText("Transport"), "streamable_http");
    await user.type(within(dialog).getByLabelText("URL"), "https://example.test/mcp");
    await user.click(within(dialog).getByLabelText("Authenticate with OAuth"));
    await user.click(within(dialog).getByRole("button", { name: "Add server" }));

    await waitFor(() =>
      expect(mocks.create).toHaveBeenCalledWith(
        expect.objectContaining({
          enabled: false,
          metadata: { auth: "oauth" },
          transport: "streamable_http",
        }),
      ),
    );
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });

  it("adds the official Vercel preset without claiming it is connected", async () => {
    mocks.list.mockResolvedValue([]);
    mocks.create.mockImplementation(async (input) => ({ ...input, id: "vercel" }));
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await screen.findByText("No custom servers");
    expect(screen.getByText(/Vercel must accept Clovy/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add Vercel" }));
    expect(await screen.findByText("Vercel")).toBeInTheDocument();
    expect(mocks.create).toHaveBeenCalledWith(
      expect.objectContaining({
        enabled: false,
        url: "https://mcp.vercel.com",
        metadata: { auth: "oauth", preset: "vercel" },
        safety: expect.objectContaining({ requiresApproval: true }),
      }),
    );
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add Vercel" })).not.toBeInTheDocument();
  });

  it("reports remote revocation separately from local credential removal", async () => {
    const oauth = {
      ...server,
      id: "vercel",
      name: "Vercel",
      transport: "streamable_http" as const,
      url: "https://mcp.vercel.com",
      metadata: { auth: "oauth", oauthConnected: true },
    };
    mocks.list.mockResolvedValue([oauth]);
    mocks.disconnectOauth.mockResolvedValue({
      server: { ...oauth, enabled: false, metadata: { auth: "oauth" } },
      remoteRevoked: false,
      localCleared: true,
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(
      await screen.findByText(/Local credentials cleared · Remote revocation failed/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });

  it("keeps cleanup available when local removal failed", async () => {
    const oauth = {
      ...server,
      id: "vercel",
      name: "Vercel",
      transport: "streamable_http" as const,
      url: "https://mcp.vercel.com",
      metadata: { auth: "oauth", oauthConnected: true },
    };
    mocks.list.mockResolvedValue([oauth]);
    mocks.disconnectOauth.mockResolvedValue({
      server: { ...oauth, enabled: false, metadata: { auth: "oauth", oauthCleanupPending: true } },
      remoteRevoked: null,
      localCleared: false,
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(await screen.findByText(/Remote revocation unavailable/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeInTheDocument();
  });

  it("updates disconnect outcome labels when the interface language changes", async () => {
    const oauth = {
      ...server,
      id: "oauth",
      name: "OAuth",
      transport: "streamable_http" as const,
      url: "https://example.test",
      metadata: { auth: "oauth", oauthConnected: true },
    };
    mocks.list.mockResolvedValue([oauth]);
    mocks.disconnectOauth.mockResolvedValue({
      server: { ...oauth, enabled: false, metadata: { auth: "oauth" } },
      remoteRevoked: false,
      localCleared: true,
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(await screen.findByText(/Local credentials cleared/)).toBeInTheDocument();
    try {
      act(() => setInterfaceLocale("pt-BR"));
      expect(await screen.findByText(/Credenciais locais removidas/)).toHaveTextContent(
        "Falha na revogação remota",
      );
      expect(screen.queryByText(/Local credentials cleared/)).not.toBeInTheDocument();
    } finally {
      act(() => setInterfaceLocale("en"));
    }
  });

  it("allows cancelling a pending browser connection", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        id: "oauth",
        name: "OAuth",
        enabled: false,
        transport: "streamable_http",
        url: "https://example.test",
        metadata: { auth: "oauth" },
      },
    ]);
    let rejectConnect: (reason: Error) => void = () => {};
    mocks.connectOauth.mockImplementation(
      () =>
        new Promise((_resolve, reject) => {
          rejectConnect = reject;
        }),
    );
    mocks.cancelOauth.mockImplementation(async () => {
      rejectConnect(new Error("Connection cancelled"));
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Connect" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    const operationId = mocks.connectOauth.mock.calls[0]?.[1];
    expect(operationId).toEqual(expect.any(String));
    expect(mocks.cancelOauth).toHaveBeenCalledWith("oauth", operationId);
    expect(await screen.findByText(/Connection cancelled/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
  });

  it("reloads a connection when the previous credential still needs cleanup", async () => {
    const oauth = {
      ...server,
      id: "oauth",
      name: "OAuth",
      enabled: false,
      transport: "streamable_http" as const,
      url: "https://example.test",
      metadata: { auth: "oauth" },
    };
    mocks.list.mockResolvedValueOnce([oauth]).mockResolvedValueOnce([
      {
        ...oauth,
        enabled: true,
        metadata: { auth: "oauth", oauthConnected: true, oauthRetiredSecretRef: "retired" },
      },
    ]);
    mocks.connectOauth.mockRejectedValue({
      code: "agent_mcp_oauth_cleanup_pending",
      message: "Cleanup pending",
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Connect" }));
    expect(await screen.findByText(/Previous credential still needs removal/)).toHaveTextContent(
      "Cleanup pending",
    );
    expect(screen.getByRole("button", { name: "Disconnect" })).toBeInTheDocument();
  });

  it("renders structured Tauri errors instead of object coercion", async () => {
    mocks.list.mockResolvedValue([]);
    mocks.create.mockRejectedValue({
      code: "agent_mcp_failed",
      message: "Server configuration could not be saved.",
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);

    await screen.findByText("No custom servers");
    await user.click(screen.getByRole("button", { name: "Add server" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Name"), "Private tools");
    await user.type(within(dialog).getByLabelText("Command"), "node");
    await user.click(within(dialog).getByRole("button", { name: "Add server" }));

    expect(
      await within(dialog).findByText("Server configuration could not be saved."),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText("[object Object]")).not.toBeInTheDocument();
  });

  it("edits tool visibility without replacing existing keychain credentials", async () => {
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);

    await screen.findByText("Tasks");
    await user.click(screen.getByRole("button", { name: "Configure Tasks" }));
    const dialog = screen.getByRole("dialog");
    await user.type(within(dialog).getByLabelText("Allowed tools, one per line"), "list_tasks");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith(
        expect.objectContaining({
          id: "mcp-tasks",
          toolVisibility: { include: ["list_tasks"], exclude: [] },
        }),
      ),
    );
    expect(mocks.update.mock.calls[0]?.[0]).not.toHaveProperty("secrets");
  });

  it("keeps remote revocation guidance after renaming and saving a disconnected server", async () => {
    const diagnostic = {
      origin: "clovy_managed" as const,
      server: "Private tools",
      phase: "disconnect",
      code: "agent_mcp_oauth_revoke_failed",
      state: "unavailable",
      action: "revoke_at_provider",
      message: "Local credentials were cleared, but remote revocation failed.",
    };
    const oauth: AgentMcpServerDto = {
      ...server,
      id: "oauth",
      name: "Private tools",
      transport: "streamable_http",
      command: undefined,
      url: "https://example.test/mcp",
      metadata: { auth: "oauth", oauthConnected: true },
    };
    const disconnected = {
      ...oauth,
      enabled: false,
      metadata: { auth: "oauth", oauthConnected: false, oauthRemoteRevocationFailed: true },
      diagnostic,
    };
    mocks.list.mockResolvedValue([oauth]);
    mocks.disconnectOauth.mockResolvedValue({
      server: disconnected,
      localCleared: true,
      remoteRevoked: false,
    });
    mocks.update.mockImplementation(async (input) => ({
      ...disconnected,
      ...input,
      diagnostic: { ...diagnostic, server: input.name },
    }));
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(await screen.findByText(/Revoke the grant at the provider/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Configure Private tools" }));
    const dialog = screen.getByRole("dialog");
    await user.clear(within(dialog).getByLabelText("Name"));
    await user.type(within(dialog).getByLabelText("Name"), "Renamed tools");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
    expect(await screen.findByText("Renamed tools")).toBeInTheDocument();
    expect(screen.getByText(/Revoke the grant at the provider/)).toHaveTextContent(
      "agent_mcp_oauth_revoke_failed",
    );
    expect(screen.getByText(/Last check: unavailable/)).toHaveTextContent(
      "Local credentials were cleared, but remote revocation failed.",
    );
    expect(screen.getByRole("switch", { name: "Renamed tools enabled" })).not.toBeChecked();
    expect(mocks.connectOauth).not.toHaveBeenCalled();
  });

  it("requests header mode explicitly after a complete OAuth disconnect", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        id: "custom-http",
        name: "Private tools",
        enabled: false,
        transport: "streamable_http",
        command: undefined,
        url: "https://example.test/mcp",
        metadata: { auth: "oauth", oauthConnected: false },
      },
    ]);
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Configure Private tools" }));
    const dialog = screen.getByRole("dialog");
    await user.click(within(dialog).getByLabelText("Authenticate with OAuth"));
    fireEvent.change(within(dialog).getByLabelText("HTTP headers (JSON)"), {
      target: { value: '{"Authorization":"Bearer synthetic-header-fixture"}' },
    });
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "custom-http",
        metadata: {},
        secrets: { env: {}, headers: { Authorization: "Bearer synthetic-header-fixture" } },
      }),
    );
    expect(screen.queryByRole("button", { name: "Connect" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Private tools enabled" }));
    await waitFor(() =>
      expect(screen.getByRole("switch", { name: "Private tools enabled" })).toBeChecked(),
    );
    expect(mocks.update).toHaveBeenLastCalledWith(
      expect.objectContaining({ id: "custom-http", enabled: true, metadata: {} }),
    );
    expect(mocks.connectOauth).not.toHaveBeenCalled();
  });

  it("keeps an earlier grant warning while the new connection is usable after reconnect, Test, save and reload", async () => {
    const history = [
      {
        id: "earlier-grant-fixture",
        affectedSecretRef: "earlier-ref-fixture",
        affectedRevision: "earlier-revision-fixture",
        reason: "rotation_unconfirmed",
      },
    ];
    let persisted: AgentMcpServerDto = {
      ...server,
      id: "oauth-history",
      name: "Private tools",
      enabled: false,
      transport: "streamable_http",
      command: undefined,
      url: "https://example.test/mcp",
      metadata: { auth: "oauth", oauthConnected: false },
    };
    mocks.list.mockImplementation(async () => [persisted]);
    mocks.connectOauth.mockImplementation(async () => {
      persisted = {
        ...persisted,
        enabled: true,
        metadata: { auth: "oauth", oauthConnected: true, oauthRevocationHistory: history },
      };
      return persisted;
    });
    mocks.update.mockImplementation(async (input) => {
      persisted = { ...persisted, ...input };
      return persisted;
    });
    const user = userEvent.setup();
    const view = render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Connect" }));
    expect(await screen.findByText(/Last check: connected/)).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(
      "An earlier OAuth authorization may still be active. Review and revoke it at the provider.",
    );
    await user.click(screen.getByRole("button", { name: "Test Private tools" }));
    expect(await screen.findByText(/1 tool available/)).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    await user.click(screen.getByRole("button", { name: "Configure Private tools" }));
    const dialog = screen.getByRole("dialog");
    await user.clear(within(dialog).getByLabelText("Name"));
    await user.type(within(dialog).getByLabelText("Name"), "Renamed tools");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
    expect(await screen.findByText("Renamed tools")).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    expect(mocks.update.mock.calls[0]?.[0].metadata.oauthRevocationHistory).toEqual(history);
    view.unmount();
    render(<AgentMcpServersSection />);
    expect(await screen.findByText("Renamed tools")).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    expect(screen.getByRole("switch", { name: "Renamed tools enabled" })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "Test Renamed tools" }));
    expect(await screen.findByText(/Last check: connected/)).toHaveTextContent("1 tool available");
    expect(screen.getByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    expect(mocks.disconnectOauth).not.toHaveBeenCalled();
  });

  it.each([
    { history: null },
    { history: {} },
    { history: [{}] },
    { history: "malformed-history-fixture" },
  ])("keeps malformed historical state visible without exposing its payload (%j)", async ({
    history,
  }) => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        metadata: { oauthRevocationHistory: history },
      },
    ]);
    render(<AgentMcpServersSection />);
    expect(await screen.findByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    expect(screen.queryByText("malformed-history-fixture")).not.toBeInTheDocument();
  });

  it("does not show a historical warning when history is absent or empty", async () => {
    mocks.list.mockResolvedValue([
      server,
      {
        ...server,
        id: "empty-history",
        name: "Empty history",
        metadata: { oauthRevocationHistory: [] },
      },
    ]);
    render(<AgentMcpServersSection />);
    expect(await screen.findByText("Empty history")).toBeInTheDocument();
    expect(screen.queryByRole("note")).not.toBeInTheDocument();
  });

  it("translates the earlier grant warning when the interface language changes", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        metadata: {
          oauthRevocationHistory: [
            {
              id: "history-fixture",
              affectedSecretRef: null,
              affectedRevision: null,
              reason: "connect_exchange_unconfirmed",
            },
          ],
        },
      },
    ]);
    render(<AgentMcpServersSection />);
    expect(await screen.findByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    try {
      act(() => setInterfaceLocale("pt-BR"));
      expect(screen.getByRole("note")).toHaveTextContent(
        "Uma autorização OAuth anterior ainda pode estar ativa. Revise e revogue no provedor.",
      );
    } finally {
      act(() => setInterfaceLocale("en"));
    }
  });

  it("reconnects migrated OAuth servers through the browser flow", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        id: "legacy-oauth",
        name: "Legacy OAuth tools",
        enabled: false,
        transport: "streamable_http",
        command: undefined,
        url: "https://example.test/mcp",
        metadata: { legacyAuth: "oauth", needsReview: true },
      },
    ]);
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);

    await user.click(await screen.findByRole("button", { name: "Connect" }));
    await waitFor(() =>
      expect(mocks.connectOauth).toHaveBeenCalledWith("legacy-oauth", expect.any(String)),
    );
    expect(await screen.findByText(/OAuth connected/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reconnect" })).toBeInTheDocument();
  });

  it.each([
    "revocation_failed",
    "revoked_but_local_cleanup_pending",
  ])("keeps OAuth connect cleanup visible after failed reconnect, healthy Test, save and reload (%s)", async (phase) => {
    const attempt = {
      id: "failed-connect-fixture",
      secretRef: "unactivated-candidate-fixture",
      revision: "candidate-policy-fixture",
      epoch: 1,
      phase,
      superseded: false,
    };
    let persisted: AgentMcpServerDto = {
      ...server,
      name: "Healthy primary",
      transport: "streamable_http",
      command: undefined,
      url: "https://example.test/mcp",
      secretRef: "healthy-primary-fixture",
      metadata: { auth: "oauth", oauthConnected: true },
    };
    mocks.list.mockImplementation(async () => [persisted]);
    mocks.connectOauth.mockImplementation(async () => {
      persisted = {
        ...persisted,
        metadata: { ...persisted.metadata, oauthConnectAttempts: [attempt] },
      };
      throw { code: "agent_mcp_protocol", message: "Synthetic catalog rejected" };
    });
    mocks.update.mockImplementation(async (input) => {
      persisted = { ...persisted, ...input };
      return persisted;
    });
    const user = userEvent.setup();
    const view = render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Reconnect" }));
    expect(await screen.findByText(/Synthetic catalog rejected/)).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(
      "An incomplete OAuth connection still needs cleanup. Disconnect to retry, or revoke its grant at the provider.",
    );
    expect(screen.getByRole("switch", { name: "Healthy primary enabled" })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "Test Healthy primary" }));
    expect(await screen.findByText(/Last check: connected/)).toHaveTextContent("1 tool available");
    expect(screen.getByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    await user.click(screen.getByRole("button", { name: "Configure Healthy primary" }));
    const dialog = screen.getByRole("dialog");
    await user.clear(within(dialog).getByLabelText("Name"));
    await user.type(within(dialog).getByLabelText("Name"), "Renamed primary");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
    expect(await screen.findByText("Renamed primary")).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    expect(mocks.update.mock.calls[0]?.[0].metadata.oauthConnectAttempts).toEqual([attempt]);
    view.unmount();
    render(<AgentMcpServersSection />);
    expect(await screen.findByText("Renamed primary")).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    await user.click(screen.getByRole("button", { name: "Test Renamed primary" }));
    expect(await screen.findByText(/Last check: connected/)).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    expect(screen.getByRole("switch", { name: "Renamed primary enabled" })).toBeChecked();
    expect(screen.queryByText("unactivated-candidate-fixture")).not.toBeInTheDocument();
    expect(mocks.disconnectOauth).not.toHaveBeenCalled();
  });

  it.each([
    { attempts: null },
    { attempts: {} },
    { attempts: [{}] },
    { attempts: "malformed-connect-fixture" },
  ])("keeps malformed OAuth connect cleanup visible without payload (%j)", async ({ attempts }) => {
    mocks.list.mockResolvedValue([{ ...server, metadata: { oauthConnectAttempts: attempts } }]);
    render(<AgentMcpServersSection />);
    expect(await screen.findByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    expect(screen.queryByText("malformed-connect-fixture")).not.toBeInTheDocument();
  });

  it("does not invent OAuth connect cleanup for absent, empty or superseded attempts", async () => {
    const phases = [
      "authorizing",
      "exchange_pending",
      "received",
      "revocation_failed",
      "revoked_but_local_cleanup_pending",
    ];
    mocks.list.mockResolvedValue([
      server,
      {
        ...server,
        id: "empty-attempts",
        name: "Empty attempts",
        metadata: { oauthConnectAttempts: [] },
      },
      ...phases.map((phase, index) => ({
        ...server,
        id: `superseded-attempt-${index}`,
        name: `Superseded attempt ${index}`,
        metadata: {
          oauthConnectAttempts: [
            {
              id: `attempt-${index}`,
              secretRef: `candidate-${index}`,
              revision: null,
              epoch: 0,
              phase,
              superseded: true,
            },
          ],
        },
      })),
    ]);
    render(<AgentMcpServersSection />);
    expect(await screen.findByText("Superseded attempt 4")).toBeInTheDocument();
    expect(screen.queryByRole("note")).not.toBeInTheDocument();
  });

  it("translates OAuth connect cleanup while historical warning remains independent", async () => {
    mocks.list.mockResolvedValue([
      {
        ...server,
        metadata: {
          oauthRevocationHistory: [
            {
              id: "history-fixture",
              affectedSecretRef: null,
              affectedRevision: null,
              reason: "rotation_unconfirmed",
            },
          ],
          oauthConnectAttempts: [
            {
              id: "attempt-fixture",
              secretRef: "candidate-fixture",
              revision: null,
              epoch: 0,
              phase: "revocation_failed",
            },
          ],
        },
      },
    ]);
    render(<AgentMcpServersSection />);
    expect(await screen.findAllByRole("note")).toHaveLength(2);
    expect(screen.getByText(/incomplete OAuth connection/)).toBeInTheDocument();
    expect(screen.getByText(/earlier OAuth authorization/)).toBeInTheDocument();
    try {
      act(() => setInterfaceLocale("pt-BR"));
      expect(
        screen.getByText(
          "Uma conexão OAuth incompleta ainda precisa de limpeza. Desconecte para tentar novamente ou revogue a autorização no provedor.",
        ),
      ).toBeInTheDocument();
      expect(screen.getByText(/Uma autorização OAuth anterior/)).toBeInTheDocument();
    } finally {
      act(() => setInterfaceLocale("en"));
    }
  });
  it.each([
    "revocation_failed",
    "revoked_but_local_cleanup_pending",
  ])("offers Disconnect after first OAuth Connect fails with cleanup pending (%s)", async (phase) => {
    let persisted: AgentMcpServerDto = {
      ...server,
      id: `first-connect-${phase}`,
      name: "First connection",
      enabled: false,
      transport: "streamable_http",
      command: undefined,
      url: "https://example.test/mcp",
      metadata: {
        auth: "oauth",
        ...(phase === "revoked_but_local_cleanup_pending" ? { oauthConnected: false } : {}),
      },
    };
    mocks.list.mockImplementation(async () => [persisted]);
    mocks.connectOauth.mockImplementation(async () => {
      persisted = {
        ...persisted,
        metadata: {
          ...persisted.metadata,
          oauthConnectAttempts: [
            {
              id: "first-attempt-fixture",
              secretRef: "unactivated-first-candidate-fixture",
              revision: "first-policy-fixture",
              epoch: 1,
              phase,
              superseded: false,
            },
          ],
        },
      };
      throw { code: "agent_mcp_protocol", message: "Synthetic first catalog rejected" };
    });
    mocks.disconnectOauth.mockImplementation(async () => {
      persisted = {
        ...persisted,
        enabled: false,
        metadata: { auth: "oauth", oauthConnected: false },
      };
      return { server: persisted, localCleared: true, remoteRevoked: true };
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: /^Connect$/ }));
    expect(await screen.findByText(/Synthetic first catalog rejected/)).toBeInTheDocument();
    expect(await screen.findByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    expect(mocks.list).toHaveBeenCalledTimes(2);
    expect(mocks.connectOauth).toHaveBeenCalledWith(persisted.id, expect.any(String));
    expect(persisted.metadata.oauthCleanupPending).toBeUndefined();
    expect(persisted.metadata.oauthPendingSecretRef).toBeUndefined();
    expect(persisted.metadata.oauthRetiredSecretRef).toBeUndefined();
    expect(persisted.secretRef).toBeUndefined();
    await user.click(screen.getByRole("button", { name: /^Disconnect$/ }));
    await waitFor(() => expect(mocks.disconnectOauth).toHaveBeenCalledWith(persisted.id));
    expect(mocks.disconnectOauth).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/Local credentials cleared/)).toBeInTheDocument();
    expect(screen.queryByRole("note")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Connect$/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Disconnect$/ })).not.toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "First connection enabled" })).not.toBeChecked();
    expect(screen.queryByText(/Last check: connected/)).not.toBeInTheDocument();
    expect(mocks.connectOauth).toHaveBeenCalledTimes(1);
    expect(mocks.cancelOauth).not.toHaveBeenCalled();
  });

  it.each([
    "authorizing",
    "exchange_pending",
    "received",
  ])("preserves draft/error and exposes persisted OAuth recovery after restart (%s)", async (phase) => {
    const originalName = "Recoverable tools";
    const originalUrl = "https://example.test/mcp";
    const diagnostic = {
      origin: "clovy_managed" as const,
      server: originalName,
      phase: "connect",
      code: "agent_mcp_secure_storage_failed",
      state: "unavailable",
      action: "check_local_storage",
      message: "MCP secure storage operation failed",
    };
    const attempt = {
      id: "orphan-attempt-fixture",
      secretRef: "orphan-candidate-fixture",
      revision: "orphan-policy-fixture",
      epoch: 1,
      phase,
      superseded: false,
    };
    let persisted: AgentMcpServerDto = {
      ...server,
      id: `restored-${phase}`,
      name: originalName,
      enabled: false,
      transport: "streamable_http",
      command: undefined,
      args: [],
      url: originalUrl,
      metadata: { auth: "oauth", oauthConnectAttempts: [attempt] },
      diagnostic,
    };
    let storageAvailable = false;
    mocks.list.mockImplementation(async () => [persisted]);
    mocks.update.mockRejectedValue({
      code: diagnostic.code,
      message: "Synthetic secure storage save failed",
      details: { ...diagnostic, phase: "disconnect" },
    });
    mocks.disconnectOauth.mockImplementation(async () => {
      if (!storageAvailable) {
        throw { code: diagnostic.code, message: diagnostic.message, details: diagnostic };
      }
      persisted = {
        ...persisted,
        diagnostic:
          phase === "exchange_pending"
            ? {
                origin: "clovy_managed",
                server: originalName,
                phase: "disconnect",
                code: "agent_mcp_oauth_revoke_failed",
                state: "unavailable",
                action: "revoke_at_provider",
                message:
                  "Local OAuth credentials were removed, but remote revocation failed. Revoke the grant at the provider.",
              }
            : undefined,
        metadata: {
          auth: "oauth",
          oauthConnected: false,
          ...(phase === "exchange_pending"
            ? {
                oauthRevocationHistory: [
                  {
                    id: "unknown-exchange-fixture",
                    affectedSecretRef: attempt.secretRef,
                    affectedRevision: attempt.revision,
                    reason: "connect_exchange_unconfirmed",
                  },
                ],
              }
            : {}),
        },
      };
      return {
        server: persisted,
        localCleared: true,
        remoteRevoked: phase === "received" ? true : phase === "exchange_pending" ? false : null,
      };
    });
    const user = userEvent.setup();
    const view = render(<AgentMcpServersSection />);
    expect(await screen.findByText(originalName)).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(diagnostic.code);
    expect(screen.getByRole("status")).toHaveTextContent(/secure storage access/);
    expect(screen.getByText(diagnostic.message, { exact: false })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Disconnect$/ })).toBeEnabled();
    expect(screen.getByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    expect(screen.getByRole("switch", { name: `${originalName} enabled` })).not.toBeChecked();
    await user.click(screen.getByRole("button", { name: `Configure ${originalName}` }));
    const dialog = screen.getByRole("dialog");
    await user.clear(within(dialog).getByLabelText("Name"));
    await user.type(within(dialog).getByLabelText("Name"), "Unsaved recovery draft");
    await user.clear(within(dialog).getByLabelText("URL"));
    await user.type(within(dialog).getByLabelText("URL"), "https://example.test/unsaved");
    await user.click(within(dialog).getByRole("button", { name: "Save changes" }));
    expect(
      await within(dialog).findByText("Synthetic secure storage save failed"),
    ).toBeInTheDocument();
    expect(screen.getByRole("dialog")).toBe(dialog);
    expect(within(dialog).getByLabelText("Name")).toHaveValue("Unsaved recovery draft");
    expect(within(dialog).getByLabelText("URL")).toHaveValue("https://example.test/unsaved");
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        id: persisted.id,
        name: "Unsaved recovery draft",
        url: "https://example.test/unsaved",
      }),
    );
    expect(persisted.name).toBe(originalName);
    expect(persisted.url).toBe(originalUrl);
    expect(mocks.connectOauth).not.toHaveBeenCalled();
    expect(mocks.disconnectOauth).not.toHaveBeenCalled();
    expect(mocks.remove).not.toHaveBeenCalled();
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText(originalName)).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(diagnostic.code);
    expect(screen.getByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    storageAvailable = true;
    await user.click(screen.getByRole("button", { name: /^Disconnect$/ }));
    await waitFor(() => expect(mocks.disconnectOauth).toHaveBeenCalledWith(persisted.id));
    expect(mocks.disconnectOauth).toHaveBeenCalledTimes(1);
    expect(await screen.findByText(/Local credentials cleared/)).toBeInTheDocument();
    expect(screen.queryByText(/incomplete OAuth connection/)).not.toBeInTheDocument();
    expect(screen.queryByText(diagnostic.message, { exact: false })).not.toBeInTheDocument();
    if (phase === "exchange_pending") {
      expect(screen.getByRole("status")).toHaveTextContent("agent_mcp_oauth_revoke_failed");
      expect(screen.getByRole("status")).toHaveTextContent(/Revoke the grant at the provider/);
      expect(persisted.diagnostic).toEqual(
        expect.objectContaining({
          code: "agent_mcp_oauth_revoke_failed",
          phase: "disconnect",
          state: "unavailable",
          action: "revoke_at_provider",
          message:
            "Local OAuth credentials were removed, but remote revocation failed. Revoke the grant at the provider.",
        }),
      );
      expect(screen.getByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
      expect(screen.getByText(/Remote revocation failed/)).toBeInTheDocument();
      expect(screen.queryByText(/Provider accepted revocation/)).not.toBeInTheDocument();
    } else if (phase === "authorizing") {
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
      expect(screen.getByText(/Remote revocation unavailable/)).toBeInTheDocument();
      expect(screen.queryByRole("note")).not.toBeInTheDocument();
    } else {
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
      expect(screen.getByText(/Provider accepted revocation/)).toBeInTheDocument();
      expect(screen.queryByRole("note")).not.toBeInTheDocument();
    }
    expect(persisted.name).toBe(originalName);
    expect(persisted.url).toBe(originalUrl);
    expect(persisted.metadata.oauthConnectAttempts).toBeUndefined();
    expect(mocks.connectOauth).not.toHaveBeenCalled();
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(mocks.create).not.toHaveBeenCalled();
    view.unmount();
    render(<AgentMcpServersSection />);
    expect(await screen.findByText(originalName)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Connect$/ })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: `${originalName} enabled` })).not.toBeChecked();
    expect(screen.queryByText(/incomplete OAuth connection/)).not.toBeInTheDocument();
    if (phase === "exchange_pending") {
      expect(screen.getByRole("status")).toHaveTextContent("agent_mcp_oauth_revoke_failed");
      expect(screen.getByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    } else {
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    }
  });

  it.each([
    "authorizing",
    "exchange_pending",
    "received",
  ])("keeps live OAuth Connect quiet and preserves correlated Cancel controls (%s)", async (phase) => {
    const live = {
      ...server,
      id: `pending-${phase}`,
      name: "Pending tools",
      transport: "streamable_http" as const,
      command: undefined,
      url: "https://example.test/mcp",
      secretRef: "existing-primary-fixture",
      metadata: {
        auth: "oauth",
        oauthConnected: true,
        oauthConnectAttempts: [
          {
            id: "pending-attempt-fixture",
            secretRef: "pending-candidate-fixture",
            revision: null,
            epoch: 0,
            phase,
            superseded: false,
          },
        ],
        oauthRevocationHistory: [
          {
            id: "history-fixture",
            affectedSecretRef: "old-ref-fixture",
            affectedRevision: null,
            reason: "rotation_unconfirmed",
          },
        ],
      },
    };
    const other = {
      ...server,
      id: "other-orphan",
      name: "Other recoverable tools",
      enabled: false,
      transport: "streamable_http" as const,
      command: undefined,
      url: "https://example.test/other",
      metadata: {
        auth: "oauth",
        oauthConnectAttempts: [
          {
            id: "other-attempt-fixture",
            secretRef: "other-candidate-fixture",
            revision: null,
            epoch: 0,
            phase: "received",
            superseded: false,
          },
        ],
      },
    };
    mocks.list.mockResolvedValue([live, other]);
    let rejectConnect: (reason: Error) => void = () => {};
    mocks.connectOauth.mockImplementation(
      () =>
        new Promise((_resolve, reject) => {
          rejectConnect = reject;
        }),
    );
    mocks.cancelOauth.mockImplementation(async () => {
      rejectConnect(new Error("Synthetic pending connection cancelled"));
    });
    const user = userEvent.setup();
    render(<AgentMcpServersSection />);
    await user.click(await screen.findByRole("button", { name: "Reconnect" }));
    const liveRow = screen.getByText(live.name).closest(".settings-row") as HTMLElement;
    const otherRow = screen.getByText(other.name).closest(".settings-row") as HTMLElement;
    expect(within(liveRow).getByRole("button", { name: "Cancel" })).toBeEnabled();
    expect(within(liveRow).queryByText(/incomplete OAuth connection/)).not.toBeInTheDocument();
    expect(within(liveRow).getByRole("note")).toHaveTextContent(/earlier OAuth authorization/);
    expect(within(liveRow).getByRole("button", { name: "Disconnect" })).toBeDisabled();
    expect(within(otherRow).getByRole("note")).toHaveTextContent(/incomplete OAuth connection/);
    expect(within(otherRow).getByRole("button", { name: "Disconnect" })).toBeDisabled();
    for (const name of [live.name, other.name]) {
      expect(screen.getByRole("button", { name: `Configure ${name}` })).toBeDisabled();
      expect(screen.getByRole("button", { name: `Test ${name}` })).toBeDisabled();
      expect(screen.getByRole("button", { name: `Delete ${name}` })).toBeDisabled();
      expect(screen.getByRole("switch", { name: `${name} enabled` })).toBeDisabled();
    }
    const operationId = mocks.connectOauth.mock.calls[0]?.[1];
    expect(operationId).toEqual(expect.any(String));
    await user.click(within(liveRow).getByRole("button", { name: "Cancel" }));
    expect(mocks.cancelOauth).toHaveBeenCalledWith(live.id, operationId);
    expect(await screen.findByText(/Synthetic pending connection cancelled/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reconnect" })).toBeEnabled();
    expect(mocks.connectOauth).toHaveBeenCalledTimes(1);
    expect(mocks.disconnectOauth).not.toHaveBeenCalled();
    expect(mocks.remove).not.toHaveBeenCalled();
    expect(mocks.update).not.toHaveBeenCalled();
  });
});
