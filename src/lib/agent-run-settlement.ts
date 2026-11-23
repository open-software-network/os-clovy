import { listen as tauriListen, type UnlistenFn } from "@tauri-apps/api/event";
import { t } from "../i18n/translate";
import { dispatchAgentRunSettled, dispatchAgentSessionStatus } from "./agent-events";
import {
  loadQueuedAgentFollowUps,
  reconcileConsumedAgentFollowUp,
  type QueuedAgentFollowUps,
} from "./agent-follow-up-queue";
import type { AgentRuntimeEvent } from "./agent-runtime-contract";

const AGENT_RUNTIME_EVENT = "clovy://agent-runtime-event";

/** Bounded memory of runs already settled, so a replayed terminal frame for
 * the same run cannot announce it twice. */
const SETTLED_RUN_MEMORY = 200;

type ListenToRuntimeEvents = (
  event: string,
  handler: (event: { payload: AgentRuntimeEvent }) => void,
) => Promise<UnlistenFn>;

export type AgentRunSettlementRelayOptions = {
  /** Current title for a stored session id, when the caller knows one. */
  resolveTitle: (sessionId: string) => string | undefined;
  /** The persisted follow-up queue; the Agent workspace submits a session's
   * queued follow-up as the next run once the current one completes. */
  readQueuedFollowUps?: () => QueuedAgentFollowUps;
  listen?: ListenToRuntimeEvents;
};

/**
 * Announces each successfully completed Agent run once, independently of the
 * React surface that submitted it, so the Agent HUD and the "ready"
 * notification still hear about runs that finish while the user is elsewhere.
 *
 * Only `run.completed` settles, and only once the session is really idle: a
 * completed run whose session still holds a queued follow-up (not consumed
 * as live steering) continues automatically, so the next run's completion
 * announces instead. A failed run of any session, open or in the
 * background, is announced here once as a `failed` session status (the
 * surfaces showing the session leave that status to this relay); an explicit
 * Stop stays silent, matching the run monitor this replaces. Streamed frames
 * only track which runs are still active.
 */
export function startAgentRunSettlementRelay(options: AgentRunSettlementRelayOptions) {
  const listen = options.listen ?? (tauriListen as ListenToRuntimeEvents);
  const readQueuedFollowUps = options.readQueuedFollowUps ?? loadQueuedAgentFollowUps;
  const activeRuns = new Map<string, string>();
  /** `steering:<messageId>` items each active run consumed, by run id. */
  const consumedSteering = new Map<string, string[]>();
  const finishedRuns = new Set<string>();
  let disposed = false;
  let unlisten: UnlistenFn | undefined;

  const rememberFinished = (runId: string) => {
    finishedRuns.add(runId);
    if (finishedRuns.size > SETTLED_RUN_MEMORY) {
      const oldest = finishedRuns.values().next().value;
      if (oldest !== undefined) finishedRuns.delete(oldest);
    }
  };

  void listen(AGENT_RUNTIME_EVENT, ({ payload }) => {
    if (!payload?.runId || payload.method === "run.diagnostic" || finishedRuns.has(payload.runId))
      return;
    const terminal =
      payload.method === "run.completed" ||
      payload.method === "run.cancelled" ||
      payload.method === "run.failed";
    if (!terminal) {
      activeRuns.set(payload.runId, payload.sessionId);
      if (payload.method === "steering.consumed") {
        consumedSteering.set(payload.runId, [
          ...(consumedSteering.get(payload.runId) ?? []),
          `steering:${payload.data.messageId}`,
        ]);
      }
      return;
    }
    const consumed = consumedSteering.get(payload.runId) ?? [];
    activeRuns.delete(payload.runId);
    consumedSteering.delete(payload.runId);
    rememberFinished(payload.runId);
    if (payload.method === "run.failed") {
      const title = options.resolveTitle(payload.sessionId)?.trim();
      dispatchAgentSessionStatus({
        sessionId: payload.sessionId,
        ...(title ? { title } : {}),
        status: "failed",
        summary: payload.data.message,
        activeCount: activeRuns.size,
      });
      return;
    }
    if (payload.method !== "run.completed") return;
    const continuation = reconcileConsumedAgentFollowUp(
      readQueuedFollowUps(),
      payload.sessionId,
      consumed,
    )[payload.sessionId];
    if (continuation) return;
    dispatchAgentRunSettled({
      sessionId: payload.sessionId,
      title: options.resolveTitle(payload.sessionId)?.trim() ?? "",
      summary: t("lib.agentNotify.finished"),
      // The HUD sweeps anonymous pending rows only when no other run can
      // still own one of them.
      activeCount: activeRuns.size,
    });
  })
    .then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    })
    .catch(() => {});

  return () => {
    disposed = true;
    unlisten?.();
  };
}
