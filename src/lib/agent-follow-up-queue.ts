export type QueuedAgentFollowUp = {
  messageId: string;
  prompt: string;
  attachments: string[];
  model: string;
  delivery?: "follow_up" | "attachments";
  steering?: "pending" | "accepted";
};

export type QueuedAgentFollowUps = Record<string, QueuedAgentFollowUp>;

export const ATTACHMENT_FOLLOW_UP_NOTE = "The files are attached to this message.";

const STORAGE_KEY = "clovy.agent.queuedFollowUps";

function queuedFollowUp(value: unknown): QueuedAgentFollowUp | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) return undefined;
  const record = value as Record<string, unknown>;
  if (
    typeof record.messageId !== "string" ||
    typeof record.prompt !== "string" ||
    typeof record.model !== "string" ||
    !Array.isArray(record.attachments) ||
    !record.attachments.every((path) => typeof path === "string") ||
    (record.delivery !== undefined &&
      record.delivery !== "follow_up" &&
      record.delivery !== "attachments") ||
    (record.steering !== undefined &&
      record.steering !== "pending" &&
      record.steering !== "accepted")
  ) {
    return undefined;
  }
  return {
    messageId: record.messageId,
    prompt: record.prompt,
    attachments: record.attachments,
    model: record.model,
    ...(record.delivery ? { delivery: record.delivery } : {}),
  };
}

export function loadQueuedAgentFollowUps(): QueuedAgentFollowUps {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return {};
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const result: QueuedAgentFollowUps = {};
    for (const [sessionId, value] of Object.entries(parsed)) {
      const queued = queuedFollowUp(value);
      if (sessionId && queued) result[sessionId] = queued;
    }
    return result;
  } catch {
    return {};
  }
}

export function saveQueuedAgentFollowUps(queued: QueuedAgentFollowUps) {
  try {
    if (Object.keys(queued).length === 0) {
      window.localStorage.removeItem(STORAGE_KEY);
      return;
    }
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(queued));
  } catch {
    // The current mounted workspace still owns the in-memory queue.
  }
}

export function mergeQueuedAgentFollowUp(
  current: QueuedAgentFollowUp | undefined,
  incoming: QueuedAgentFollowUp,
): QueuedAgentFollowUp {
  const attachments = Array.from(
    new Set([...(current?.attachments ?? []), ...incoming.attachments]),
  );
  return { ...incoming, attachments };
}

export function clearQueuedAgentFollowUpSteering(
  queued: QueuedAgentFollowUps,
  sessionId: string,
): QueuedAgentFollowUps {
  const current = queued[sessionId];
  if (!current?.steering) return queued;
  const nextItem = { ...current };
  delete nextItem.steering;
  return { ...queued, [sessionId]: nextItem };
}

/** Steering items are persisted as `steering:<messageId>`. Once native
 * history proves the active run consumed the text, retire its full fallback
 * while preserving any attachments as a contextual next turn. */
export function reconcileConsumedAgentFollowUp(
  queued: QueuedAgentFollowUps,
  sessionId: string,
  itemIds: readonly string[],
): QueuedAgentFollowUps {
  const current = queued[sessionId];
  if (
    !current ||
    current.delivery === "attachments" ||
    !itemIds.includes(`steering:${current.messageId}`)
  ) {
    return queued;
  }
  const next = { ...queued };
  if (current.attachments.length === 0) {
    delete next[sessionId];
    return next;
  }
  const attachmentsOnly: QueuedAgentFollowUp = {
    ...current,
    prompt: current.prompt.trim()
      ? `${current.prompt.trim()}\n\n${ATTACHMENT_FOLLOW_UP_NOTE}`
      : ATTACHMENT_FOLLOW_UP_NOTE,
    delivery: "attachments",
  };
  delete attachmentsOnly.steering;
  next[sessionId] = attachmentsOnly;
  return next;
}
