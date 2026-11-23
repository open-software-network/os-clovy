import type { AgentSessionDto } from "./agent-runtime-contract";
import type { SessionPartitionMap } from "./session-partition-filter";

export const AGENT_NEW_SESSION_EVENT = "clovy:agent:new-session";
export const AGENT_DELETE_SESSION_EVENT = "clovy:agent:delete-session";
export const AGENT_SESSIONS_CHANGED_EVENT = "clovy:agent:sessions-changed";
/** App → sidebar: startup or an accepted authoritative session snapshot. */
export const AGENT_SESSIONS_HYDRATED_EVENT = "clovy:agent:sessions-hydrated";
export const AGENT_NEW_SESSION_PENDING_KEY = "clovy:agent:new-session-pending";
export const AGENT_SESSION_RENAMED_EVENT = "clovy:agent:session-renamed";
export const AGENT_SESSION_STATUS_EVENT = "clovy:agent:session-status";
export const AGENT_RUN_SETTLED_EVENT = "clovy:agent:run-settled";
export const AGENT_OPEN_EVENT = "clovy:agent:open";
let sessionSnapshotEpoch = 0;

/** Native mutations supersede UI list reads already in flight. */
export function invalidateAgentSessionSnapshots() {
  sessionSnapshotEpoch += 1;
}

export function readAgentSessionSnapshotEpoch() {
  return sessionSnapshotEpoch;
}

/** Stored session id (not the runtime session id). */
export type AgentSessionRenamedDetail = {
  sessionId: string;
  title: string;
};

export type AgentSessionStatusKind =
  | "received"
  | "starting"
  | "running"
  | "waitingForUser"
  | "completed"
  | "failed"
  | "cancelled";

export type AgentSessionStatusDetail = {
  sessionId?: string;
  title?: string;
  prompt?: string;
  status: AgentSessionStatusKind;
  summary?: string;
  activeCount?: number;
  needsUserCount?: number;
};

export type AgentRunSettledDetail = {
  sessionId: string;
  title: string;
  summary: string;
  activeCount: number;
};

export type AgentSessionsHydratedDetail = {
  sessions: AgentSessionDto[];
  partitions: SessionPartitionMap;
  /** Replace metadata without changing selection, drafts or run status sets. */
  replaceExisting?: boolean;
};

export type AgentSessionsChangedDetail = {
  sessions: AgentSessionDto[];
  selectedSessionId?: string;
  workingSessionIds: string[];
  waitingSessionIds?: string[];
};

export function dispatchAgentSessionStatus(detail: AgentSessionStatusDetail) {
  window.dispatchEvent(
    new CustomEvent<AgentSessionStatusDetail>(AGENT_SESSION_STATUS_EVENT, {
      detail,
    }),
  );
  void import("@tauri-apps/api/event")
    .then((api) =>
      typeof api.emit === "function" ? api.emit(AGENT_SESSION_STATUS_EVENT, detail) : undefined,
    )
    .catch(() => {});
}

export function dispatchAgentRunSettled(detail: AgentRunSettledDetail) {
  window.dispatchEvent(
    new CustomEvent<AgentRunSettledDetail>(AGENT_RUN_SETTLED_EVENT, {
      detail,
    }),
  );
  void import("@tauri-apps/api/event")
    .then((api) =>
      typeof api.emit === "function" ? api.emit(AGENT_RUN_SETTLED_EVENT, detail) : undefined,
    )
    .catch(() => {});
}

export function dispatchAgentSessionsChanged(detail: AgentSessionsChangedDetail) {
  window.dispatchEvent(
    new CustomEvent<AgentSessionsChangedDetail>(AGENT_SESSIONS_CHANGED_EVENT, {
      detail,
    }),
  );
  emitAgentSessionsChanged(detail);
}

export function emitAgentSessionsChanged(detail: AgentSessionsChangedDetail) {
  void import("@tauri-apps/api/event")
    .then((api) =>
      typeof api.emit === "function" ? api.emit(AGENT_SESSIONS_CHANGED_EVENT, detail) : undefined,
    )
    .catch(() => {});
}
