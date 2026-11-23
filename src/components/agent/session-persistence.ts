import { AGENT_NEW_SESSION_PENDING_KEY } from "../../lib/agent-events";
import type { NoteReferenceInput } from "./composer/noteReference";

export type AgentNewSessionDetail = {
  prompt?: string;
  /** Seeds the composer with a note chip (and skips auto-submit) so the user
   * lands ready to ask about that note instead of starting an ordinary ask. */
  noteRef?: NoteReferenceInput;
};

const AGENT_LAST_OPEN_SESSION_KEY = "clovy:agent:last-open-session";
let inMemoryPendingNewSessionPayload: string | undefined;

// How long a second startNewTask call with the same prompt counts as an echo
// of the first (marker + window event double-delivery) rather than a new ask.
// The echo lands a setTimeout(0) after the mount — milliseconds — so 1s is
// already generous. It must stay time-bounded rather than clear when the
// submission settles: a fast settle would otherwise reopen the window before
// the echo arrives. User retries are unaffected either way — a failed
// auto-submit restores the draft and re-sends go through submit(), which
// never routes through this guard.
export const AUTO_SUBMIT_ECHO_WINDOW_MS = 1_000;

export function readLastOpenSessionId(): string | undefined {
  try {
    return window.localStorage.getItem(AGENT_LAST_OPEN_SESSION_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

/** Drops the stored id only when it points at the given session, so deleting
 * a background session doesn't forget the one actually open. */
export function forgetLastOpenSessionId(sessionId: string) {
  try {
    if (readLastOpenSessionId() === sessionId) {
      window.localStorage.removeItem(AGENT_LAST_OPEN_SESSION_KEY);
    }
  } catch {
    // Storage can be unavailable in restricted webviews; restore is best-effort.
  }
}

export function writeLastOpenSessionId(sessionId: string) {
  try {
    window.localStorage.setItem(AGENT_LAST_OPEN_SESSION_KEY, sessionId);
  } catch {
    // Storage can be unavailable in restricted webviews; restore is best-effort.
  }
}

export function markAgentNewSessionPending(
  prompt?: string,
  options?: { noteRef?: NoteReferenceInput },
) {
  const payload = JSON.stringify({
    createdAt: Date.now(),
    prompt: prompt?.trim() || undefined,
    noteRef: options?.noteRef,
  });
  try {
    window.sessionStorage.setItem(AGENT_NEW_SESSION_PENDING_KEY, payload);
    inMemoryPendingNewSessionPayload = undefined;
  } catch {
    // Preserve the one-shot navigation within this WebView when session storage
    // is unavailable. Unlike localStorage, this cannot replay after a restart.
    inMemoryPendingNewSessionPayload = payload;
  }
}

function readPendingNewSessionPayload(): string | undefined {
  if (inMemoryPendingNewSessionPayload !== undefined) {
    return inMemoryPendingNewSessionPayload;
  }
  try {
    return window.sessionStorage.getItem(AGENT_NEW_SESSION_PENDING_KEY) ?? undefined;
  } catch {
    return undefined;
  }
}

// A pending marker is a navigation hint, not a durable command: it's written
// just before switching to the Agent view and consumed by the very next
// mount. Anything older is a leftover from a reload or crash — acting on it
// would hijack whatever the user had open into a new session (and re-submit
// the stale prompt).
const AGENT_NEW_SESSION_PENDING_TTL_MS = 15_000;

/** Non-consuming peek at the pending marker, for state init on a fresh
 * mount. The mount effect still consumes it via pendingNewSessionRequest();
 * peeking here must not clear it, or the auto-submit prompt would be lost. */
export function hasPendingNewSessionRequest(): boolean {
  try {
    const value = readPendingNewSessionPayload();
    if (value == null) return false;
    const parsed = JSON.parse(value) as { createdAt?: number };
    return (
      typeof parsed.createdAt === "number" &&
      Date.now() - parsed.createdAt <= AGENT_NEW_SESSION_PENDING_TTL_MS
    );
  } catch {
    return false;
  }
}

function parsePendingNoteRef(value: unknown): NoteReferenceInput | undefined {
  if (!value || typeof value !== "object") return undefined;
  const record = value as { id?: unknown; title?: unknown };
  if (typeof record.id !== "string" || record.id.trim().length === 0) return undefined;
  return {
    id: record.id,
    title: typeof record.title === "string" ? record.title : "",
  };
}

export function pendingNewSessionRequest(): AgentNewSessionDetail | undefined {
  try {
    const value = readPendingNewSessionPayload();
    if (value == null) return undefined;
    // Consume on read so a remount (HMR, rapid view switches) can't re-fire
    // the same request.
    clearPendingNewSessionRequest();
    try {
      const parsed = JSON.parse(value) as {
        createdAt?: number;
        prompt?: string;
        noteRef?: unknown;
      };
      if (
        typeof parsed.createdAt !== "number" ||
        Date.now() - parsed.createdAt > AGENT_NEW_SESSION_PENDING_TTL_MS
      ) {
        return undefined;
      }
      const noteRef = parsePendingNoteRef(parsed.noteRef);
      return {
        ...(typeof parsed.prompt === "string" ? { prompt: parsed.prompt } : {}),
        ...(noteRef ? { noteRef } : {}),
      };
    } catch {
      return undefined;
    }
  } catch {
    return undefined;
  }
}

export function clearPendingNewSessionRequest() {
  inMemoryPendingNewSessionPayload = undefined;
  try {
    window.sessionStorage.removeItem(AGENT_NEW_SESSION_PENDING_KEY);
  } catch {
    // Session storage can be unavailable in restricted webviews.
  }
}
