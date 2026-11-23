import { agentRuntimeBindings, invoke } from "./tauri";
import type { AgentSessionDto } from "./agent-runtime-contract";

export type RoutineRunSession = AgentSessionDto & {
  routineId?: string;
  preview?: string;
  endedAt?: string | null;
  ended_at?: string | null;
  active?: boolean;
  is_active?: boolean;
};

export function routineIdFromSession(
  session: Pick<RoutineRunSession, "id" | "source" | "routineId">,
) {
  if (session.source !== "routine" && session.source !== "legacy_routine") return undefined;
  if ("routineId" in session && session.routineId) return session.routineId;
  return /^(?:routine|cron)_(.+)_\d{8}_\d{6}(?:_[a-z0-9-]+)?$/i.exec(session.id)?.[1];
}

export function isRunningRoutineSession(session: RoutineRunSession) {
  return Boolean(session.active || session.is_active || session.status === "running");
}

export function sessionTimestamp(session: RoutineRunSession) {
  return session.completedAt ?? session.updatedAt ?? session.createdAt;
}

export function isReplaceableRoutineRunTitle(title?: string | null) {
  const value = title?.trim() ?? "";
  return (
    !value ||
    /^\[IMPORTANT:/i.test(value) ||
    /^(?:Routine run|Untitled session|Imported session)$/i.test(value)
  );
}

export async function listRoutineRunSessions(options: { includeActive?: boolean } = {}) {
  const [sessions, mappings] = await Promise.all([
    agentRuntimeBindings.listSessions() as Promise<RoutineRunSession[]>,
    invoke<Array<{ routineId: string; agentSessionId?: string | null }>>(
      "list_agent_routine_runs",
      { request: null },
    ),
  ]);
  const routineBySession = new Map(
    mappings.flatMap((run) => (run.agentSessionId ? [[run.agentSessionId, run.routineId]] : [])),
  );
  return sessions
    .filter(
      (session) =>
        (session.source === "routine" || session.source === "legacy_routine") &&
        (options.includeActive || !isRunningRoutineSession(session)),
    )
    .map((session) => ({
      ...session,
      routineId: routineBySession.get(session.id),
      endedAt: session.status === "running" ? null : (session.completedAt ?? session.updatedAt),
      preview: session.preview ?? session.title,
    }))
    .sort((left, right) => sessionTimestamp(right).localeCompare(sessionTimestamp(left)));
}

export const ROUTINE_OPEN_EVENT = "clovy:routine:open";
const ROUTINE_OPEN_REQUEST = "clovy:routine:open-request";
let pendingRoutineOpen: string | null = null;

export function requestRoutineRunOpen(sessionId: string) {
  pendingRoutineOpen = sessionId;
  window.dispatchEvent(new Event(ROUTINE_OPEN_REQUEST));
}

/** A native notification may arrive before the lazy-loaded Routines view. */
export function onRoutineRunOpen(callback: (sessionId: string) => void) {
  function consume() {
    if (!pendingRoutineOpen) return;
    const sessionId = pendingRoutineOpen;
    pendingRoutineOpen = null;
    callback(sessionId);
  }
  window.addEventListener(ROUTINE_OPEN_REQUEST, consume);
  consume();
  return () => window.removeEventListener(ROUTINE_OPEN_REQUEST, consume);
}
