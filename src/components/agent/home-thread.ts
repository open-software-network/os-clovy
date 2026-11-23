import type { AgentChatTurn } from "../../lib/agent-chat-runtime";
import {
  CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY,
  CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY,
  isClovyHomeStartTaskTool,
  LEGACY_CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY,
  LEGACY_CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY,
  resolveClovyHomeThreadSessionId,
  type ClovyHomeTaskRequest,
} from "../../lib/clovy-home";
import { t } from "../../i18n";

// Home runs on the default CLI engine in its own session (ADR-0064). Older
// builds kept the Home conversation and its task cards only in local storage;
// this module reads that history, read-only: the direct turns are imported
// once into the Home session, and the task cards keep rendering from here.

export type HomeTaskHandoff = ClovyHomeTaskRequest & {
  id: string;
  status: "starting" | "running" | "failed";
  storedSessionId?: string;
  sourceUserTurnId?: string;
  error?: string;
  profile?: string;
  attachments?: string[];
};

const LEGACY_HOME_STORAGE_KEYS: Partial<Record<string, string>> = {
  [CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY]: LEGACY_CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY,
  [CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY]: LEGACY_CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY,
};

function parseRecord(value: string | null): Record<string, unknown> {
  try {
    const parsed = JSON.parse(value ?? "{}") as unknown;
    return parsed && typeof parsed === "object" && !Array.isArray(parsed)
      ? (parsed as Record<string, unknown>)
      : {};
  } catch {
    return {};
  }
}

function readRecord(storageKey: string): Record<string, unknown> {
  try {
    const current = parseRecord(window.localStorage.getItem(storageKey));
    const legacyStorageKey = LEGACY_HOME_STORAGE_KEYS[storageKey];
    if (!legacyStorageKey) return current;
    return { ...parseRecord(window.localStorage.getItem(legacyStorageKey)), ...current };
  } catch {
    return {};
  }
}

function homeThreadValues(storageKey: string, storedSessionId: string | undefined): unknown[] {
  const activeSessionId = storedSessionId
    ? resolveClovyHomeThreadSessionId(storedSessionId)
    : undefined;
  if (!activeSessionId) return [];
  const values = readRecord(storageKey)[activeSessionId];
  return Array.isArray(values) ? values : [];
}

/** The Home turns older builds stored, as stored, for the one-time import. */
export function readLegacyHomeDirectTurns(storedSessionId: string | undefined): unknown[] {
  return homeThreadValues(CLOVY_HOME_DIRECT_TURNS_STORAGE_KEY, storedSessionId);
}

/** The task cards older builds created. Nothing starts a card anymore, so one
 * left starting was interrupted. */
export function readHomeTaskHandoffs(storedSessionId: string | undefined): HomeTaskHandoff[] {
  return homeThreadValues(CLOVY_HOME_TASK_HANDOFFS_STORAGE_KEY, storedSessionId)
    .map((value) => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return value;
      const legacy = value as Record<string, unknown>;
      if (typeof legacy.sessionId !== "string" || typeof legacy.storedSessionId === "string") {
        return value;
      }
      const { sessionId, ...rest } = legacy;
      return { ...rest, storedSessionId: sessionId };
    })
    .filter((value): value is HomeTaskHandoff => {
      if (!value || typeof value !== "object" || Array.isArray(value)) return false;
      const handoff = value as HomeTaskHandoff;
      return (
        typeof handoff.id === "string" &&
        typeof handoff.title === "string" &&
        typeof handoff.prompt === "string" &&
        (handoff.status === "starting" ||
          handoff.status === "running" ||
          handoff.status === "failed") &&
        (handoff.status !== "running" || typeof handoff.storedSessionId === "string")
      );
    })
    .map((handoff) =>
      handoff.status === "starting"
        ? { ...handoff, status: "failed" as const, error: t("agent.home.handoffInterrupted") }
        : handoff,
    );
}

function validHomeTurn(value: unknown): value is AgentChatTurn {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const turn = value as AgentChatTurn;
  return (
    typeof turn.id === "string" &&
    (turn.role === "user" || turn.role === "assistant") &&
    typeof turn.createdAt === "string" &&
    Array.isArray(turn.parts)
  );
}

/** The stored turns that carry a task card, keyed to their card, so the cards
 * keep their place in time among the Home session's items. */
export function homeTaskCardTurns(
  directTurns: unknown[],
  handoffs: HomeTaskHandoff[],
): Array<{ turn: AgentChatTurn; handoff: HomeTaskHandoff }> {
  const cards: Array<{ turn: AgentChatTurn; handoff: HomeTaskHandoff }> = [];
  for (const turn of directTurns.filter(validHomeTurn)) {
    if (turn.role !== "assistant") continue;
    for (const part of turn.parts) {
      if (part.type !== "tool" || !isClovyHomeStartTaskTool(part.name)) continue;
      const handoff = handoffs.find((candidate) => candidate.id === `home-task-${part.id}`);
      if (handoff) cards.push({ turn: { ...turn, parts: [part] }, handoff });
    }
  }
  return cards;
}

export function compareHomeTurnOrder(left: AgentChatTurn, right: AgentChatTurn): number {
  // Array.sort is stable. Equal timestamps therefore retain persisted causal
  // order instead of letting role-prefixed ids put an assistant before its user.
  return left.createdAt.localeCompare(right.createdAt);
}
