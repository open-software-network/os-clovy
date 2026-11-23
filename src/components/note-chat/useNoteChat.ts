import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  agentItemsToChatTurns,
  applyAgentRuntimeEvent,
  createAgentRuntimeProjection,
  mergeAgentRuntimeSnapshot,
  type AgentRuntimeProjection,
} from "../../lib/agent-runtime-adapter";
import type { AgentItemDto, AgentRuntimeEvent } from "../../lib/agent-runtime-contract";
import {
  dispatchAgentSessionStatus,
  type AgentSessionStatusDetail,
  type AgentSessionStatusKind,
} from "../../lib/agent-events";
import { chatModelId, defaultChatModelId, CLI_ENGINE_MODEL_PREFIX } from "../../lib/chat-engine";
import {
  AGENT_SESSION_MODEL_CHANGED_EVENT,
  type AgentSessionModelChangedDetail,
  clearSessionModelIfApplied,
  loadSessionModels,
  rememberSessionModel,
} from "../../lib/agent-session-models";
import { messageFromError } from "../../lib/errors";
import { agentRuntimeBindings } from "../../lib/tauri";
import { getCurrentDataPartitionName } from "../../lib/data-partition";
import { noteReferenceToken, type NoteReferenceInput } from "../agent/composer/noteReference";
import {
  forgetNoteChatSession,
  noteChatSessionIdFor,
  rememberNoteChatSession,
} from "./noteChatSessions";

const AGENT_RUNTIME_EVENT = "clovy://agent-runtime-event";
/** Claude Code is the default when no configured CLI can be resolved. */
const DEFAULT_MODEL = `${CLI_ENGINE_MODEL_PREFIX}claude`;

export type NoteChatAttachment = {
  id: string;
  name: string;
  path: string;
};

export type NoteChatSubmitResult = {
  accepted: boolean;
  current: boolean;
};

export const NOTE_CHAT_SUBMIT_REJECTED: NoteChatSubmitResult = {
  accepted: false,
  current: false,
};

export type NoteChat = {
  turns: ReturnType<typeof agentItemsToChatTurns>;
  working: boolean;
  submissionPending: boolean;
  loading: boolean;
  error: string | null;
  storedSessionId: string | undefined;
  model: string;
  setModel: (model: string) => void;
  submit: (text: string, attachments?: NoteChatAttachment[]) => Promise<NoteChatSubmitResult>;
  retry: (failedItemId?: string) => Promise<boolean>;
  stop: () => void;
};

function titleFromNote(note: NoteReferenceInput) {
  return note.title.trim() || "Note chat";
}

function runIsActive(projection: AgentRuntimeProjection) {
  return projection.run?.status === "queued" || projection.run?.status === "running";
}

/** A note-scoped conversation on the default chat engine (a CLI, which reads
 * the note through Clovy's `get_note` tool).
 * The pairing is retained locally so reopening the panel resumes the same
 * session. */
export function useNoteChat(note: NoteReferenceInput | null): NoteChat {
  const noteId = note?.id;
  const [storedSessionId, setStoredSessionId] = useState<string>();
  const [projection, setProjection] = useState<AgentRuntimeProjection>(() =>
    createAgentRuntimeProjection(),
  );
  const projectionEventsRef = useRef(projection.processedEventIds);
  projectionEventsRef.current = projection.processedEventIds;
  const hydrationRequestRef = useRef(0);
  const [model, setModelState] = useState(DEFAULT_MODEL);
  const [loading, setLoading] = useState(false);
  const [submissionPending, setSubmissionPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const generationRef = useRef(0);
  const sessionIdRef = useRef<string>();
  const activeSubmitRef = useRef<symbol>();
  const selectedStoredModelRef = useRef(DEFAULT_MODEL);
  const modelSelectionRevisionRef = useRef(0);
  /** The engine was picked in this panel (or came from the stored session),
   * so the default chat engine no longer applies. */
  const modelChosenRef = useRef(false);
  const announcedStatusRef = useRef(new Map<string, AgentSessionStatusKind>());
  /** Dispatches a session status only when it changes, not once per
   * streamed token. */
  const announceStatus = useCallback((detail: AgentSessionStatusDetail) => {
    if (!detail.sessionId) return;
    if (announcedStatusRef.current.get(detail.sessionId) === detail.status) return;
    announcedStatusRef.current.set(detail.sessionId, detail.status);
    dispatchAgentSessionStatus(detail);
  }, []);
  const applyStoredModel = useCallback((storedModel: string) => {
    modelSelectionRevisionRef.current += 1;
    modelChosenRef.current = true;
    selectedStoredModelRef.current = chatModelId(storedModel);
    setModelState(chatModelId(storedModel));
  }, []);
  const setModel = useCallback((nextModel: string) => {
    modelSelectionRevisionRef.current += 1;
    modelChosenRef.current = true;
    selectedStoredModelRef.current = nextModel;
    setModelState(nextModel);
    const sessionId = sessionIdRef.current;
    if (sessionId) rememberSessionModel(sessionId, nextModel);
  }, []);

  const working = runIsActive(projection);
  const turns = useMemo(() => agentItemsToChatTurns(projection.items), [projection.items]);

  const hydrate = useCallback(
    async (sessionId: string) => {
      const generation = generationRef.current;
      const request = ++hydrationRequestRef.current;
      const readStartedEventIds = projectionEventsRef.current;
      const modelSelectionRevision = modelSelectionRevisionRef.current;
      const isCurrentSession = () =>
        generationRef.current === generation &&
        sessionIdRef.current === sessionId &&
        hydrationRequestRef.current === request;
      const [session, items, latestRun] = await Promise.all([
        agentRuntimeBindings.getSession(sessionId),
        agentRuntimeBindings.listItems(sessionId),
        agentRuntimeBindings.getLatestRun?.(sessionId) ?? Promise.resolve(null),
      ]);
      if (!isCurrentSession()) return;
      setProjection((current) =>
        mergeAgentRuntimeSnapshot(
          current,
          {
            session,
            items,
            run: latestRun ?? undefined,
          },
          readStartedEventIds,
        ),
      );
      if (modelSelectionRevisionRef.current !== modelSelectionRevision) return;
      const storedModel = loadSessionModels()[session.id] ?? session.model ?? DEFAULT_MODEL;
      const resumedModel = storedModel.startsWith(CLI_ENGINE_MODEL_PREFIX)
        ? storedModel
        : await defaultChatModelId();
      if (isCurrentSession() && modelSelectionRevisionRef.current === modelSelectionRevision) {
        if (storedModel !== resumedModel) rememberSessionModel(session.id, resumedModel);
        applyStoredModel(resumedModel);
      }
    },
    [applyStoredModel],
  );

  useEffect(() => {
    const generation = ++generationRef.current;
    const rememberedSessionId = noteId ? noteChatSessionIdFor(noteId) : undefined;
    sessionIdRef.current = rememberedSessionId;
    activeSubmitRef.current = undefined;
    setStoredSessionId(rememberedSessionId);
    setProjection(createAgentRuntimeProjection());
    selectedStoredModelRef.current = DEFAULT_MODEL;
    modelChosenRef.current = false;
    setModelState(DEFAULT_MODEL);
    setError(null);
    setSubmissionPending(false);
    if (!rememberedSessionId) {
      setLoading(false);
      // A new note chat starts on the default chat engine.
      void defaultChatModelId().then((engineModel) => {
        if (generation !== generationRef.current || modelChosenRef.current) return;
        selectedStoredModelRef.current = engineModel;
        setModelState(engineModel);
      });
      return;
    }

    setLoading(true);
    void hydrate(rememberedSessionId)
      .catch(() => {
        if (generation !== generationRef.current) return;
        forgetNoteChatSession(noteId!);
        sessionIdRef.current = undefined;
        setStoredSessionId(undefined);
      })
      .finally(() => {
        if (generation === generationRef.current) setLoading(false);
      });
  }, [hydrate, noteId]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<AgentRuntimeEvent>(AGENT_RUNTIME_EVENT, ({ payload }) => {
      if (payload.sessionId !== sessionIdRef.current) return;
      setProjection((current) => applyAgentRuntimeEvent(current, payload));
      if (payload.method === "run.diagnostic") return;
      const terminal =
        payload.method === "run.completed" ||
        payload.method === "run.cancelled" ||
        payload.method === "run.failed";
      // The App-level run relay announces failures of every session.
      if (payload.method === "run.failed") {
        announcedStatusRef.current.set(payload.sessionId, "failed");
      } else {
        announceStatus({
          sessionId: payload.sessionId,
          title: projection.session?.title ?? "Note chat",
          status:
            payload.method === "run.completed"
              ? "completed"
              : payload.method === "run.cancelled"
                ? "cancelled"
                : "running",
        });
      }
      if (terminal) void hydrate(payload.sessionId).catch(() => undefined);
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [announceStatus, hydrate, projection.session?.title]);

  useEffect(() => {
    const applyExternalSessionModel = (event: Event) => {
      const detail = (event as CustomEvent<AgentSessionModelChangedDetail>).detail;
      if (!detail || detail.sessionId !== sessionIdRef.current) return;
      applyStoredModel(detail.storedModel);
    };
    window.addEventListener(AGENT_SESSION_MODEL_CHANGED_EVENT, applyExternalSessionModel);
    return () =>
      window.removeEventListener(AGENT_SESSION_MODEL_CHANGED_EVENT, applyExternalSessionModel);
  }, [applyStoredModel]);

  const submit = useCallback(
    async (
      rawText: string,
      attachments: NoteChatAttachment[] = [],
    ): Promise<NoteChatSubmitResult> => {
      const question = rawText.trim();
      if (
        (!question && !attachments.length) ||
        !note ||
        loading ||
        activeSubmitRef.current ||
        working
      ) {
        return NOTE_CHAT_SUBMIT_REJECTED;
      }
      const token = Symbol("note-chat-submit");
      const generation = generationRef.current;
      const current = () =>
        generation === generationRef.current && activeSubmitRef.current === token;
      activeSubmitRef.current = token;
      setSubmissionPending(true);
      setError(null);
      let accepted = false;
      try {
        let session = projection.session;
        const existingSession = Boolean(session);
        if (!session) {
          if (!modelChosenRef.current) {
            // Sent before the default engine was read: read it now.
            selectedStoredModelRef.current = await defaultChatModelId();
          }
          session = await agentRuntimeBindings.createSession({
            title: titleFromNote(note),
            model: selectedStoredModelRef.current,
            safetyMode: "sandboxed",
            profile: getCurrentDataPartitionName(),
          });
          rememberNoteChatSession(note.id, session.id);
          if (current()) {
            sessionIdRef.current = session.id;
            setStoredSessionId(session.id);
            setProjection(createAgentRuntimeProjection({ session }));
          }
        }
        const prompt = existingSession
          ? question || "Use the attached file(s)."
          : `${noteReferenceToken(note)} ${question || "Use the attached file(s)."}`;
        const optimistic: AgentItemDto = {
          id: `optimistic:${crypto.randomUUID()}`,
          sessionId: session.id,
          sequence: Math.max(0, ...projection.items.map((item) => item.sequence)) + 1,
          createdAt: new Date().toISOString(),
          kind: "message",
          role: "user",
          text: prompt,
          status: "complete",
          attachments: attachments.map((attachment) => ({
            id: `attachment:${attachment.id}`,
            sessionId: session.id,
            name: attachment.name,
            path: attachment.path,
            action: "imported",
            available: true,
            createdAt: new Date().toISOString(),
          })),
        };
        if (current()) {
          setProjection((existing) => ({
            ...existing,
            session,
            items: [...existing.items, optimistic],
          }));
        }
        const submittedModel = loadSessionModels()[session.id] ?? selectedStoredModelRef.current;
        const run = await agentRuntimeBindings.startRun({
          sessionId: session.id,
          prompt,
          model: submittedModel,
          safetyMode: "sandboxed",
          workspacePath: session.workspacePath,
          enabledSkillIds: [],
          attachments: attachments.map((attachment) => attachment.path),
        });
        clearSessionModelIfApplied(session.id, submittedModel);
        accepted = true;
        if (current()) {
          setProjection((existing) => ({ ...existing, session, run }));
          announceStatus({
            sessionId: session.id,
            title: session.title,
            status: "starting",
          });
        }
        return { accepted: true, current: current() };
      } catch (cause) {
        if (current()) setError(messageFromError(cause));
        return { accepted, current: current() };
      } finally {
        if (activeSubmitRef.current === token) {
          activeSubmitRef.current = undefined;
          setSubmissionPending(false);
        }
      }
    },
    [announceStatus, loading, note, projection.items, projection.session, working],
  );

  const stop = useCallback(() => {
    if (!projection.run) return;
    void agentRuntimeBindings
      .cancelRun(projection.run.id)
      .catch((cause) => setError(messageFromError(cause)));
  }, [projection.run]);

  const retry = useCallback(
    async (failedItemId?: string) => {
      const failedItem = failedItemId
        ? projection.items.find(
            (item) => item.id === failedItemId && item.kind === "error" && item.retryable,
          )
        : undefined;
      const runId =
        failedItem?.runId ?? (projection.run?.status === "failed" ? projection.run.id : undefined);
      const session = projection.session;
      if (!runId || !session || loading || activeSubmitRef.current) {
        return false;
      }
      const token = Symbol("note-chat-retry");
      const generation = generationRef.current;
      const current = () =>
        generation === generationRef.current && activeSubmitRef.current === token;
      activeSubmitRef.current = token;
      setSubmissionPending(true);
      setError(null);
      try {
        const nextRun = await agentRuntimeBindings.retryRun(runId);
        if (current()) {
          setProjection((existing) => ({ ...existing, run: nextRun }));
          announceStatus({
            sessionId: session.id,
            title: session.title,
            status: "starting",
          });
        }
        return true;
      } catch (cause) {
        if (current()) setError(messageFromError(cause));
        return false;
      } finally {
        if (activeSubmitRef.current === token) {
          activeSubmitRef.current = undefined;
          setSubmissionPending(false);
        }
      }
    },
    [announceStatus, loading, projection.items, projection.run, projection.session],
  );

  return {
    turns,
    working,
    submissionPending,
    loading,
    error,
    storedSessionId,
    model,
    setModel,
    submit,
    retry,
    stop,
  };
}
