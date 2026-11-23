export const AGENT_RUNTIME_PROTOCOL_VERSION = 1 as const;

export type AgentSafetyMode = "sandboxed" | "unrestricted";
export type AgentSessionStatus =
  | "idle"
  | "running"
  | "waiting_for_user"
  | "completed"
  | "interrupted"
  | "failed";

export type AgentSessionSource = "user" | "routine" | "legacy_routine" | "legacy_task";

export type AgentSessionDto = {
  id: string;
  title: string;
  status: AgentSessionStatus;
  model: string;
  safetyMode: AgentSafetyMode;
  workspacePath: string;
  source: AgentSessionSource;
  createdAt: string;
  updatedAt: string;
  completedAt?: string;
  error?: string;
};

export type AgentRunStatus =
  | "queued"
  | "running"
  | "waiting_for_user"
  | "completed"
  | "cancelled"
  | "interrupted"
  | "failed";

export type AgentUsageDto = {
  inputTokens?: number;
  outputTokens?: number;
  totalTokens?: number;
  provider?: string;
  privacyLevel?: string;
  endpoint?: string;
};

export type AgentRunDto = {
  id: string;
  sessionId: string;
  status: AgentRunStatus;
  model: string;
  approvalMode?: "approvals" | "bypass";
  reasoningEffort?: "minimal" | "medium" | "high";
  startedAt?: string;
  completedAt?: string;
  usage?: AgentUsageDto;
  error?: string;
};

type AgentItemBase = {
  id: string;
  sessionId: string;
  runId?: string;
  /** Authoritative position in the stored conversation, across all runs. */
  sequence: number;
  createdAt: string;
};

export type AgentMessageItemDto = AgentItemBase & {
  kind: "message";
  role: "user" | "assistant" | "system";
  text: string;
  status: "streaming" | "complete";
  attachments?: AgentArtifactDto[];
};

export type AgentReasoningItemDto = AgentItemBase & {
  kind: "reasoning";
  text: string;
  status: "streaming" | "complete";
};

export type AgentContextSummaryItemDto = AgentItemBase & {
  kind: "context_summary";
  text: string;
  metadata?: {
    fallback?: boolean;
  };
};

export type AgentSteeringItemDto = AgentItemBase & {
  kind: "steering";
  text: string;
};

export type AgentToolCallItemDto = AgentItemBase & {
  kind: "tool_call";
  callId: string;
  name: string;
  arguments: unknown;
  /** Interrupted means the run ended without an observed final result. */
  status: "running" | "complete" | "failed" | "interrupted";
};

export type AgentToolResultItemDto = AgentItemBase & {
  kind: "tool_result";
  callId: string;
  name: string;
  output: unknown;
  isError: boolean;
};

export type AgentInterruptionItemDto = AgentItemBase & {
  kind: "interruption";
  interruption: AgentInterruptionDto;
};

export type AgentErrorItemDto = AgentItemBase & {
  kind: "error";
  message: string;
  category?: "tool" | "provider" | "runtime" | "context" | "credits";
  code?: string;
  retryable: boolean;
};

export type AgentItemDto =
  | AgentMessageItemDto
  | AgentReasoningItemDto
  | AgentSteeringItemDto
  | AgentContextSummaryItemDto
  | AgentToolCallItemDto
  | AgentToolResultItemDto
  | AgentInterruptionItemDto
  | AgentErrorItemDto;

type AgentInterruptionBase = {
  id: string;
  sessionId: string;
  runId: string;
  /** `cancelled` only on items asked by a CLI turn (`engine: "cli"`). */
  status: "pending" | "resolved" | "expired" | "cancelled";
  createdAt: string;
  resolvedAt?: string;
  /** Set when a CLI turn asked through the approval broker. */
  engine?: "cli";
  /** Why a CLI item left `pending` without an answer (`timeout`,
   * `cancelled`, `restarted`). */
  retiredReason?: string;
};

export type AgentApprovalInterruptionDto = AgentInterruptionBase & {
  kind: "approval";
  toolName: string;
  title: string;
  description: string;
  command?: string;
  allowAlways: boolean;
  resolution?: "once" | "session" | "always" | "deny";
};

export type AgentClarificationInterruptionDto = AgentInterruptionBase & {
  kind: "clarification";
  question: string;
  choices: string[];
  answer?: string;
};

export type AgentSecretInterruptionDto = AgentInterruptionBase & {
  kind: "secret";
  reason: string;
};

export type AgentInterruptionDto =
  | AgentApprovalInterruptionDto
  | AgentClarificationInterruptionDto
  | AgentSecretInterruptionDto;

export type AgentArtifactDto = {
  id: string;
  sessionId: string;
  runId?: string;
  itemId?: string;
  name: string;
  path: string;
  mimeType?: string;
  sizeBytes?: number;
  action: "created" | "updated" | "imported";
  available: boolean;
  createdAt: string;
};

export type AgentSkillDto = {
  id: string;
  name: string;
  description: string;
  source: "managed" | "user_global";
  enabled: boolean;
  configuredEnabled: boolean;
  readable: boolean;
  error: { code: string; message: string } | null;
  editable: boolean;
};

type RuntimeFrameBase = {
  protocolVersion: typeof AGENT_RUNTIME_PROTOCOL_VERSION;
  sessionId: string;
  runId: string;
  /** Event cursor within this run; used only for replay/deduplication. */
  sequence: number;
};

type RuntimeItemData = {
  itemId: string;
  /** Conversation ordinal assigned by the host when persisting the item. */
  itemSequence: number;
};

export type AgentRuntimeEvent = RuntimeFrameBase &
  (
    | {
        eventId: string;
        method: "run.started";
        data: {
          startedAt: string;
          model: string;
          removedItemIds?: string[];
          contextSummary?: AgentContextSummaryItemDto;
        };
      }
    | {
        eventId: string;
        method: "message.delta";
        data: RuntimeItemData & { role: "assistant"; delta: string; createdAt: string };
      }
    | {
        eventId: string;
        method: "message.completed";
        data: RuntimeItemData & {
          role: "assistant";
          text: string;
          createdAt: string;
        };
      }
    | {
        eventId: string;
        method: "reasoning.delta";
        data: RuntimeItemData & { delta: string; createdAt: string };
      }
    | {
        eventId: string;
        method: "steering.consumed";
        data: RuntimeItemData & { messageId: string; text: string; createdAt: string };
      }
    | {
        eventId: string;
        method: "run.diagnostic";
        data: RuntimeItemData & {
          message: string;
          category: "runtime";
          code: "agent_workspace_mcp_cleanup";
          retryable: false;
          createdAt: string;
        };
      }
    | {
        eventId: string;
        method: "tool.started";
        data: RuntimeItemData & {
          callId: string;
          name: string;
          arguments: unknown;
          createdAt: string;
        };
      }
    | {
        eventId: string;
        method: "tool.completed";
        data: RuntimeItemData & {
          callId: string;
          name: string;
          output: unknown;
          createdAt: string;
        };
      }
    | {
        eventId: string;
        method: "tool.failed";
        data: RuntimeItemData & { callId: string; name: string; error: string; createdAt: string };
      }
    | {
        eventId: string;
        method: "interruption.requested";
        data: RuntimeItemData & { interruption: AgentInterruptionDto };
      }
    | {
        /** A CLI item was answered, expired, or cancelled (approval broker). */
        eventId: string;
        method: "interruption.updated";
        data: RuntimeItemData & { interruption: AgentInterruptionDto };
      }
    | { eventId: string; method: "usage.updated"; data: AgentUsageDto }
    | {
        eventId: string;
        method: "run.completed" | "run.cancelled";
        data: { completedAt: string; interruptedCallIds?: string[] };
      }
    | {
        eventId: string;
        method: "run.failed";
        data: {
          itemSequence: number;
          completedAt: string;
          interruptedCallIds?: string[];
          message: string;
          category?: "tool" | "provider" | "runtime" | "context" | "credits";
          code?: string;
          retryable: boolean;
        };
      }
  );

export type StartAgentRunRequest = {
  sessionId: string;
  prompt: string;
  model: string;
  reasoningEffort?: "minimal" | "medium" | "high";
  safetyMode: AgentSafetyMode;
  surface?: "home" | "chat";
  workspacePath: string;
  enabledSkillIds: string[];
  attachments: string[];
  attachmentMetadata?: Array<{ name: string; mediaType?: string }>;
};

export type ResolveAgentInterruptionRequest = {
  runId: string;
  interruptionId: string;
  resolution:
    | { kind: "approval"; choice: "once" | "session" | "always" | "deny" }
    | { kind: "clarification"; answer: string }
    | { kind: "secret"; secret?: string; choice?: "once" | "deny" };
};

/** Dependency-injected boundary implemented by the Tauri bindings. Keeping the
 * UI against this interface makes native command renames a single integration
 * change instead of another workspace-wide transport refactor. */
export type AgentRuntimeBindings = {
  listSessions(): Promise<AgentSessionDto[]>;
  getSession(sessionId: string): Promise<AgentSessionDto>;
  getLatestRun?(sessionId: string): Promise<AgentRunDto | null>;
  createSession(input: {
    title?: string;
    model: string;
    safetyMode: AgentSafetyMode;
    profile?: string;
  }): Promise<AgentSessionDto>;
  renameSession(sessionId: string, title: string): Promise<AgentSessionDto>;
  branchSession?(sessionId: string, itemId: string): Promise<AgentSessionDto>;
  deleteSession(sessionId: string): Promise<void>;
  listItems(sessionId: string): Promise<AgentItemDto[]>;
  startRun(input: StartAgentRunRequest): Promise<AgentRunDto>;
  cancelRun(runId: string): Promise<void>;
  retryRun(runId: string): Promise<AgentRunDto>;
  resolveInterruption(input: ResolveAgentInterruptionRequest): Promise<AgentRunDto>;
  listArtifacts(sessionId: string): Promise<AgentArtifactDto[]>;
  listSkills(): Promise<AgentSkillDto[]>;
  setSkillEnabled(skillId: string, enabled: boolean): Promise<AgentSkillDto>;
};
