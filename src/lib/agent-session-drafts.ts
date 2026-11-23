/**
 * Unsent composer text follows its stored agent session while Clovy stays open.
 * Keep this process-local: drafts can contain sensitive material and should
 * not survive an app restart.
 */
type AgentSessionDraft = {
  revision: number;
  text: string;
};

const draftsByStoredSessionId = new Map<string, AgentSessionDraft>();
const draftsByCreationRequestId = new Map<string, AgentSessionDraft>();
const invalidatedStoredSessionIds = new Set<string>();
let nextRevision = 0;

function snapshot(text: string): AgentSessionDraft {
  return { revision: ++nextRevision, text };
}

export function readAgentSessionDraft(storedSessionId: string): string | undefined {
  return draftsByStoredSessionId.get(storedSessionId)?.text;
}

export function readAgentSessionDraftRevision(storedSessionId: string): number | undefined {
  return draftsByStoredSessionId.get(storedSessionId)?.revision;
}

export function writeAgentSessionDraft(storedSessionId: string, draft: string) {
  if (!storedSessionId || invalidatedStoredSessionIds.has(storedSessionId)) return;
  if (draft.length === 0) {
    draftsByStoredSessionId.delete(storedSessionId);
    return;
  }
  draftsByStoredSessionId.set(storedSessionId, snapshot(draft));
}

export function writePendingAgentSessionDraft(creationRequestId: string, draft: string) {
  if (!creationRequestId) return;
  if (draft.length === 0) {
    draftsByCreationRequestId.delete(creationRequestId);
    return;
  }
  draftsByCreationRequestId.set(creationRequestId, snapshot(draft));
}

export function transferPendingAgentSessionDraft(
  creationRequestId: string,
  storedSessionId: string,
): boolean {
  const pending = draftsByCreationRequestId.get(creationRequestId);
  draftsByCreationRequestId.delete(creationRequestId);
  if (!pending || invalidatedStoredSessionIds.has(storedSessionId)) return false;
  draftsByStoredSessionId.set(storedSessionId, pending);
  return true;
}

export function clearPendingAgentSessionDraft(creationRequestId: string) {
  draftsByCreationRequestId.delete(creationRequestId);
}

/** Stored ids are never reused. Fence delayed list/hydration results after deletion. */
export function isAgentSessionDeleted(storedSessionId: string): boolean {
  return invalidatedStoredSessionIds.has(storedSessionId);
}

export function invalidateAgentSessionDraft(storedSessionId: string) {
  invalidatedStoredSessionIds.add(storedSessionId);
  draftsByStoredSessionId.delete(storedSessionId);
}

export function clearAgentSessionDraftRevision(storedSessionId: string, revision?: number) {
  if (revision === undefined) return;
  if (draftsByStoredSessionId.get(storedSessionId)?.revision !== revision) return;
  draftsByStoredSessionId.delete(storedSessionId);
}

/** Test-only reset because workspace tests intentionally reuse stored session ids. */
export function resetAgentSessionDraftsForTests() {
  deliveredDictationRequests.clear();
  draftsByStoredSessionId.clear();
  draftsByCreationRequestId.clear();
  invalidatedStoredSessionIds.clear();
  nextRevision = 0;
}

export const AGENT_DICTATION_DRAFT_WILL_CHANGE = "clovy:agent-dictation-draft-will-change";
export const AGENT_DICTATION_DRAFT_CHANGED = "clovy:agent-dictation-draft-changed";
const deliveredDictationRequests = new Set<string>();

/** Pinned host destination, appended once without submitting a new chat turn. */
export function deliverAgentDictation(delivery: {
  takeId: string;
  composerRequestId: string;
  conversationId: string;
  profile: string;
  text: string;
}): boolean {
  const { takeId, composerRequestId, conversationId, profile, text } = delivery;
  if (
    !takeId ||
    !composerRequestId ||
    !conversationId ||
    !profile ||
    !text.trim() ||
    invalidatedStoredSessionIds.has(conversationId)
  )
    return false;
  const key = `${takeId}:${composerRequestId}`;
  if (deliveredDictationRequests.has(key)) return true;
  // Flush the destination editor synchronously, preserving text typed during
  // transcription. An unfinished IME composition refuses replacement.
  if (
    !window.dispatchEvent(
      new CustomEvent(AGENT_DICTATION_DRAFT_WILL_CHANGE, {
        cancelable: true,
        detail: { conversationId, profile },
      }),
    )
  )
    return false;
  const previous = readAgentSessionDraft(conversationId) ?? "";
  writeAgentSessionDraft(conversationId, previous ? `${previous}\n${text}` : text);
  deliveredDictationRequests.add(key);
  if (deliveredDictationRequests.size > 128) {
    const first = deliveredDictationRequests.values().next().value;
    if (first) deliveredDictationRequests.delete(first);
  }
  window.dispatchEvent(
    new CustomEvent(AGENT_DICTATION_DRAFT_CHANGED, {
      detail: { conversationId, profile },
    }),
  );
  return true;
}
