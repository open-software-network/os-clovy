import { invoke } from "./tauri";

export type AgentMcpTransport = "stdio" | "streamable_http";

export type AgentMcpSafetyPolicy = {
  requiresApproval: boolean;
  allowSandboxed: boolean;
  timeoutMs: number;
  maxOutputBytes: number;
  approvalTools: string[];
};

/** Last observed server result, never a claim based only on a stored grant. */
export type AgentMcpDiagnostic = {
  delivery?: "unknown" | null;
  retrySafe?: boolean | null;
  cause?: AgentMcpDiagnostic | null;
  origin: "clovy_managed";
  server: string;
  tool?: string | null;
  phase: string;
  code: string;
  remoteCode?: number | null;
  state: string;
  action: string;
  message: string;
};

export type AgentMcpServerDto = {
  id: string;
  name: string;
  enabled: boolean;
  transport: AgentMcpTransport;
  command?: string;
  args: string[];
  url?: string;
  secretRef?: string;
  metadata: Record<string, unknown>;
  toolVisibility: {
    include: string[];
    exclude: string[];
  };
  safety: AgentMcpSafetyPolicy;
  diagnostic?: AgentMcpDiagnostic | null;
};

export type AgentMcpServerInput = Omit<AgentMcpServerDto, "id" | "secretRef" | "diagnostic"> & {
  id?: string;
  secrets?: {
    env: Record<string, string>;
    headers: Record<string, string>;
  };
};

export type AgentMcpToolDto = {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
};

export function listAgentMcpServers() {
  return invoke<AgentMcpServerDto[]>("list_agent_mcp_servers");
}

export function createAgentMcpServer(input: AgentMcpServerInput) {
  return invoke<AgentMcpServerDto>("create_agent_mcp_server", { input });
}

export function updateAgentMcpServer(input: AgentMcpServerInput & { id: string }) {
  return invoke<AgentMcpServerDto>("update_agent_mcp_server", { input });
}

export function deleteAgentMcpServer(serverId: string) {
  return invoke<{ localDeleted: boolean; remoteRevoked: boolean | null }>(
    "delete_agent_mcp_server",
    {
      serverId,
    },
  );
}

export function testAgentMcpServer(serverId: string) {
  return invoke<AgentMcpToolDto[]>("test_agent_mcp_server", { serverId });
}

export function connectAgentMcpOauth(serverId: string, operationId?: string) {
  return invoke<AgentMcpServerDto>("connect_agent_mcp_oauth", { serverId, operationId });
}

export function cancelAgentMcpOauth(serverId: string, operationId?: string) {
  return invoke<void>("cancel_agent_mcp_oauth", { serverId, operationId });
}

export function disconnectAgentMcpOauth(serverId: string) {
  return invoke<{
    server: AgentMcpServerDto;
    remoteRevoked: boolean | null;
    localCleared: boolean;
  }>("disconnect_agent_mcp_oauth", { serverId });
}

export const DEFAULT_AGENT_MCP_SAFETY: AgentMcpSafetyPolicy = {
  requiresApproval: true,
  allowSandboxed: true,
  timeoutMs: 30_000,
  maxOutputBytes: 1_048_576,
  approvalTools: [],
};
