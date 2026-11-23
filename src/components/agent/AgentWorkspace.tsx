import { AgentBackground } from "./AgentBackground";
import { listen } from "@tauri-apps/api/event";
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { IconArrowDown } from "central-icons/IconArrowDown";
import { IconArrowRotateClockwise } from "central-icons/IconArrowRotateClockwise";
import { IconArrowUp } from "central-icons/IconArrowUp";
import { IconCheckmark2Small } from "central-icons/IconCheckmark2Small";
import { IconChevronDownSmall } from "central-icons/IconChevronDownSmall";
import { IconCrossSmall } from "central-icons/IconCrossSmall";
import { IconFileText } from "central-icons/IconFileText";
import { IconMicrophone } from "central-icons/IconMicrophone";
import { IconNoteText } from "central-icons/IconNoteText";
import { IconPlusMedium } from "central-icons/IconPlusMedium";
import { IconShieldCheck } from "central-icons/IconShieldCheck";
import { IconShieldCrossed } from "central-icons/IconShieldCrossed";
import { IconStop } from "central-icons/IconStop";
import {
  type CSSProperties,
  type DragEvent,
  Fragment,
  type FormEvent,
  type RefObject,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import {
  agentItemsToChatTurns,
  applyAgentRuntimeEvent,
  createAgentRuntimeProjection,
  mergeAgentRuntimeSnapshot,
  reconcileAgentInterruptionResolution,
  type AgentRuntimeProjection,
} from "../../lib/agent-runtime-adapter";
import type {
  AgentArtifactDto,
  AgentItemDto,
  AgentRuntimeEvent,
  AgentSafetyMode,
  AgentSessionDto,
  ResolveAgentInterruptionRequest,
} from "../../lib/agent-runtime-contract";
import { stageDroppedAgentFiles } from "../../lib/agent-file-drop";
import {
  agentRuntimeBindings,
  discardStagedAgentAttachments,
  downloadAgentArtifact,
  dictationHelperCommand,
  importLegacyHomeTurns,
  pruneStagedAgentAttachments,
} from "../../lib/tauri";
import {
  AGENT_SESSIONS_HYDRATED_EVENT,
  type AgentSessionsHydratedDetail,
  type AgentSessionStatusDetail,
  type AgentSessionStatusKind,
  dispatchAgentSessionStatus,
  dispatchAgentSessionsChanged,
  readAgentSessionSnapshotEpoch,
} from "../../lib/agent-events";
import { filterAgentSessionsForDataPartition } from "../../lib/session-partition-filter";
import { messageFromError } from "../../lib/errors";
import {
  clearQueuedAgentFollowUpSteering,
  loadQueuedAgentFollowUps,
  mergeQueuedAgentFollowUp,
  reconcileConsumedAgentFollowUp,
  saveQueuedAgentFollowUps,
  type QueuedAgentFollowUp,
  type QueuedAgentFollowUps,
} from "../../lib/agent-follow-up-queue";
import {
  AGENT_SESSION_MODEL_CHANGED_EVENT,
  type AgentSessionModelChangedDetail,
  clearSessionModelIfApplied,
  forgetSessionModel,
  loadSessionModels,
  rememberSessionModel,
} from "../../lib/agent-session-models";
import {
  AGENT_DICTATION_DRAFT_CHANGED,
  AGENT_DICTATION_DRAFT_WILL_CHANGE,
  clearAgentSessionDraftRevision,
  clearPendingAgentSessionDraft,
  invalidateAgentSessionDraft,
  isAgentSessionDeleted,
  readAgentSessionDraft,
  readAgentSessionDraftRevision,
  transferPendingAgentSessionDraft,
  writePendingAgentSessionDraft,
  writeAgentSessionDraft,
} from "../../lib/agent-session-drafts";
import {
  prepareProjectPrompt,
  ProjectContextSignatureStore,
  stripProjectContext,
} from "../../lib/agent-project-context";
import { formatNumber, t as translate, useT } from "../../i18n";
import { AgentChatTurnRow } from "./chat-turns/AgentChatTurnRow";
import {
  AgentArtifactPanel,
  type AgentArtifact,
  type AgentArtifactPanelState,
} from "./chat-turns/AgentArtifactPanel";
import { AgentSessionBar } from "./chat-turns/AgentSessionBar";
import { AgentThinking } from "./AgentThinking";
import {
  advanceHeroGreetingIndex,
  AGENT_DELETE_SESSION_EVENT,
  AGENT_NEW_SESSION_EVENT,
  AGENT_SHORTCUTS,
  heroGreeting,
  rememberUnrestrictedAcknowledged,
  SANDBOX_OPTIONS,
  unrestrictedAcknowledged,
} from "./agent-workspace-config";
import { ComposerEditor, type ComposerEditorHandle } from "./composer/ComposerEditor";
import { AgentAttachmentTile } from "./composer/AgentAttachmentTile";
import { agentComposerClearance } from "./composer/layout";
import { ChatEnginePicker } from "./composer/ChatEnginePicker";
import {
  CLI_ENGINE_MODEL_PREFIX,
  type ChatEngineCatalog,
  chatDefaultEngine,
  DEFAULT_CHAT_CLI_IDS,
  chatEngineCatalog,
  classifyChatEngineModel,
  chatModelId,
  defaultChatModelId,
} from "../../lib/chat-engine";
import { type LlmCliId, llmSetUsage } from "../../lib/llm-providers";
import { getCurrentDataPartitionName } from "../../lib/data-partition";
import { Dialog } from "../ui/Dialog";
import { Spinner } from "../ui/Spinner";
import { ClovyAlive } from "../brand/ClovyAlive";
import {
  type AgentNewSessionDetail,
  pendingNewSessionRequest,
  writeLastOpenSessionId,
  forgetLastOpenSessionId,
} from "./session-persistence";
import type { AgentWorkspaceProps } from "./agent-workspace-types";
import {
  forgetClovyHomeStoredSessionId,
  clovyHomeDailyCheckIn,
  clovyHomeDayKey,
  clovyHomeDayLabel,
  clovyHomeGreetingParts,
  clovyHomeNudgePrompts,
  CLOVY_HOME_THREAD_CHANGED_EVENT,
  stripClovyHomeContextFromPreview,
} from "../../lib/clovy-home";
import type { AgentChatTurn } from "../../lib/agent-chat-runtime";
import {
  compareHomeTurnOrder,
  homeTaskCardTurns,
  readHomeTaskHandoffs,
  readLegacyHomeDirectTurns,
  type HomeTaskHandoff,
} from "./home-thread";

export type { AgentWorkspaceOrigin } from "./agent-workspace-types";
export { markAgentNewSessionPending } from "./session-persistence";
export { pendingNewSessionRequest, type AgentNewSessionDetail } from "./session-persistence";
export {
  AGENT_DELETE_SESSION_EVENT,
  AGENT_NEW_SESSION_EVENT,
  AGENT_NEW_SESSION_PENDING_KEY,
  AGENT_SESSIONS_CHANGED_EVENT,
  AGENT_SESSION_RENAMED_EVENT,
  HERO_GREETINGS,
  type AgentSessionRenamedDetail,
  type AgentSessionsChangedDetail,
} from "./agent-workspace-config";

export const AGENT_RUNTIME_EVENT = "clovy://agent-runtime-event";
const DEFAULT_MODEL = `${CLI_ENGINE_MODEL_PREFIX}claude`;
/** Home runs on the default CLI engine (ADR-0064). With no CLI installed it
 * still asks Claude Code, so the turn fails with the CLI sessions' error. */
const HOME_FALLBACK_ENGINE_MODEL = `${CLI_ENGINE_MODEL_PREFIX}${DEFAULT_CHAT_CLI_IDS[0]}`;

async function homeEngineModelId(): Promise<string> {
  try {
    return (await chatDefaultEngine()).modelId ?? HOME_FALLBACK_ENGINE_MODEL;
  } catch {
    return HOME_FALLBACK_ENGINE_MODEL;
  }
}
const projectContextSignaturesBySessionId = new ProjectContextSignatureStore();
const USAGE_FOCUSABLE =
  'a[href], button:not([disabled]), textarea:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), [tabindex]:not([tabindex="-1"])';

export function composerInSteerStateFor(input: {
  selectedSessionId?: string;
  provisional: boolean;
  working: boolean;
  submitting: boolean;
  submittingSessionId: string | null;
  demo: boolean;
}): boolean {
  return Boolean(
    input.selectedSessionId &&
      !input.provisional &&
      (input.working ||
        (input.submitting && input.submittingSessionId === input.selectedSessionId) ||
        input.demo),
  );
}

function titleFromPrompt(prompt: string) {
  const normalized = prompt.replace(/\s+/g, " ").trim();
  return normalized.length > 52 ? `${normalized.slice(0, 51).trimEnd()}…` : normalized;
}

function queuedAttachmentStatus(attachments: readonly string[]) {
  return translate("agent.queued.attachments", { count: attachments.length });
}

function queuedFollowUpStatus(queued: QueuedAgentFollowUp, failed: boolean) {
  const withAttachmentStatus = (status: string) =>
    queued.attachments.length
      ? translate("agent.queued.withAttachments", {
          status,
          attachments: queuedAttachmentStatus(queued.attachments),
        })
      : status;
  if (failed) {
    return withAttachmentStatus(
      queued.delivery === "attachments"
        ? translate("agent.queued.attachmentsFailed")
        : translate("agent.queued.followUpFailed"),
    );
  }
  if (queued.delivery === "attachments") {
    return queuedAttachmentStatus(queued.attachments);
  }
  if (queued.steering) {
    const steeringStatus =
      queued.steering === "accepted"
        ? translate("agent.queued.steering")
        : translate("agent.queued.sending");
    return withAttachmentStatus(steeringStatus);
  }
  return withAttachmentStatus(translate("agent.queued.followUp"));
}

function queuedFollowUpText(queued: QueuedAgentFollowUp) {
  if (queued.delivery !== "attachments") return queued.prompt;
  return queued.attachments.map((path) => path.split(/[\\/]/).pop() || path).join(", ");
}

function artifactView(artifact: AgentArtifactDto): AgentArtifact {
  return {
    name: artifact.name,
    path: artifact.path,
    rootLabel: translate("agent.artifacts.rootLabel"),
    size: artifact.sizeBytes,
  };
}

export function AgentWorkspace({
  initialSession,
  initialSessionId,
  homeMode = false,
  onHomeSessionCreated,
  onOpenHomeTaskSession,
  origin,
  onSessionSelected,
  onMoveSessionToProject,
  sessionInProject = false,
  projectContext,
  resolveSessionProjectContext,
}: AgentWorkspaceProps = {}) {
  const t = useT();
  const initialAgentSession =
    initialSession && !isAgentSessionDeleted(initialSession.id) ? initialSession : undefined;
  const initialStoredSessionId =
    initialAgentSession?.id ??
    (initialSessionId && !isAgentSessionDeleted(initialSessionId) ? initialSessionId : undefined);
  const pendingRequestRef = useRef(pendingNewSessionRequest());
  const [sessions, setSessions] = useState<AgentSessionDto[]>(
    initialAgentSession ? [initialAgentSession] : [],
  );
  const sessionsRef = useRef(sessions);
  sessionsRef.current = sessions;
  const sessionListRevisionRef = useRef(0);
  const [selectedId, setSelectedId] = useState(initialStoredSessionId);
  const [newSessionMode, setNewSessionMode] = useState(!initialStoredSessionId);
  const [projection, setProjection] = useState<AgentRuntimeProjection>(() =>
    createAgentRuntimeProjection({ session: initialAgentSession }),
  );
  const projectionEventsRef = useRef(projection.processedEventIds);
  projectionEventsRef.current = projection.processedEventIds;
  const [hydratedSelectionId, setHydratedSelectionId] = useState<string>();
  const [artifacts, setArtifacts] = useState<AgentArtifactDto[]>([]);
  const [artifactPanel, setArtifactPanel] = useState<AgentArtifactPanelState | null>(null);
  const [usageOpen, setUsageOpen] = useState(false);
  const usagePanelRef = useRef<HTMLElement>(null);
  const usageReturnFocusRef = useRef<HTMLElement | null>(null);
  const usageTitleId = useId();

  useLayoutEffect(() => {
    if (!usageOpen || !usagePanelRef.current) return;
    usageReturnFocusRef.current = document.activeElement as HTMLElement | null;
    usagePanelRef.current.querySelector<HTMLElement>(USAGE_FOCUSABLE)?.focus();
    return () => {
      usageReturnFocusRef.current?.focus?.();
      usageReturnFocusRef.current = null;
    };
  }, [usageOpen]);

  useEffect(() => {
    if (!usageOpen) return;
    function onKey(event: KeyboardEvent) {
      if (event.key === "Escape") {
        event.preventDefault();
        setUsageOpen(false);
        return;
      }
      if (event.key !== "Tab" || !usagePanelRef.current) return;
      const focusables = Array.from(
        usagePanelRef.current.querySelectorAll<HTMLElement>(USAGE_FOCUSABLE),
      );
      if (focusables.length === 0) return;
      const first = focusables[0];
      const last = focusables[focusables.length - 1];
      if (!usagePanelRef.current.contains(document.activeElement)) {
        event.preventDefault();
        first.focus();
      } else if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
    window.addEventListener("keydown", onKey);
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = previousOverflow;
    };
  }, [usageOpen]);

  const homeEngineModelRef = useRef(HOME_FALLBACK_ENGINE_MODEL);
  const initialModel = initialAgentSession?.model ?? DEFAULT_MODEL;
  const [model, setModel] = useState(homeMode ? HOME_FALLBACK_ENGINE_MODEL : initialModel);
  const modelRef = useRef(model);
  modelRef.current = model;
  const [engineCatalog, setEngineCatalog] = useState<ChatEngineCatalog | null>(null);
  const engineCatalogRef = useRef(engineCatalog);
  engineCatalogRef.current = engineCatalog;
  const refreshEngineCatalog = useCallback(async () => {
    try {
      const catalog = await chatEngineCatalog();
      setEngineCatalog(catalog);
      return catalog;
    } catch {
      return null;
    }
  }, []);
  useEffect(() => {
    void refreshEngineCatalog();
  }, [refreshEngineCatalog]);
  const [safetyMode, setSafetyMode] = useState<AgentSafetyMode>(
    initialAgentSession?.safetyMode ?? "sandboxed",
  );
  const safetyModeRef = useRef(safetyMode);
  safetyModeRef.current = safetyMode;
  const [draft, setDraft] = useState(pendingRequestRef.current?.prompt ?? "");
  const [draftRevision, setDraftRevision] = useState(0);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const draftHasContentRef = useRef(Boolean(draft.trim()));
  const setComposerDraft = useCallback((value: string, options?: { persist?: boolean }) => {
    draftRef.current = value;
    draftHasContentRef.current = Boolean(value.trim());
    if (options?.persist !== false && selectedIdRef.current) {
      writeAgentSessionDraft(selectedIdRef.current, value);
    }
    setDraft(value);
    setDraftRevision((revision) => revision + 1);
  }, []);
  useEffect(() => {
    const onDictation = (event: Event) => {
      const detail = (event as CustomEvent<{ conversationId: string }>).detail;
      if (detail.conversationId !== selectedIdRef.current) return;
      setComposerDraft(readAgentSessionDraft(detail.conversationId) ?? "", { persist: false });
    };
    window.addEventListener(AGENT_DICTATION_DRAFT_CHANGED, onDictation);
    return () => window.removeEventListener(AGENT_DICTATION_DRAFT_CHANGED, onDictation);
  }, [setComposerDraft]);
  const [attachments, setAttachments] = useState<string[]>([]);
  const attachmentsRef = useRef(attachments);
  attachmentsRef.current = attachments;
  const attachmentGenerationRef = useRef(0);
  const dropOwnerRef = useRef<string>();
  const inFlightAttachmentLeasesRef = useRef(new Map<string, string[]>());
  const [attaching, setAttaching] = useState(false);
  const setComposerAttachments = useCallback(
    (update: string[] | ((current: string[]) => string[])) => {
      const next = typeof update === "function" ? update(attachmentsRef.current) : update;
      attachmentsRef.current = next;
      setAttachments(next);
    },
    [],
  );
  const abandonComposerAttachments = useCallback(() => {
    attachmentGenerationRef.current += 1;
    dropOwnerRef.current = undefined;
    setAttaching(false);
    const paths = attachmentsRef.current;
    attachmentsRef.current = [];
    setAttachments([]);
    if (paths.length > 0) {
      void discardStagedAgentAttachments(paths).catch(() => undefined);
    }
  }, []);
  useEffect(
    () => () => {
      attachmentGenerationRef.current += 1;
      dropOwnerRef.current = undefined;
      const paths = attachmentsRef.current;
      attachmentsRef.current = [];
      if (paths.length > 0) {
        void discardStagedAgentAttachments(paths).catch(() => undefined);
      }
    },
    [],
  );
  const [queuedFollowUps, setQueuedFollowUpsState] =
    useState<QueuedAgentFollowUps>(loadQueuedAgentFollowUps);
  const queuedFollowUpsRef = useRef(queuedFollowUps);
  queuedFollowUpsRef.current = queuedFollowUps;
  const attemptedQueuedMessageIdsRef = useRef(new Set<string>());
  const [failedQueuedMessageIds, setFailedQueuedMessageIds] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const updateQueuedFollowUps = useCallback(
    (update: (current: QueuedAgentFollowUps) => QueuedAgentFollowUps) => {
      setQueuedFollowUpsState((current) => {
        const next = update(current);
        queuedFollowUpsRef.current = next;
        if (next !== current) saveQueuedAgentFollowUps(next);
        return next;
      });
    },
    [],
  );
  const queuedFollowUp = selectedId ? queuedFollowUps[selectedId] : undefined;
  const queuedSubmissionSnapshotRef = useRef<{
    sessionId: string;
    messageId: string;
    prompt: string;
    attachments: string[];
    model: string;
  }>();
  const [recoverableSubmission, setRecoverableSubmission] = useState<{
    id: string;
    prompt: string;
    attachments: string[];
    model: string;
  }>();
  const recoverableSubmissionRef = useRef(recoverableSubmission);
  recoverableSubmissionRef.current = recoverableSubmission;
  const recoverableSubmissionSnapshotRef = useRef<typeof recoverableSubmission>();
  const [pendingInitialTurn, setPendingInitialTurn] = useState<{
    prompt: string;
    storedSessionId?: string;
    title: string;
    turn: AgentChatTurn;
  }>();
  const pendingSessionCreationRef = useRef<string>();
  const creationDraftDestinationsRef = useRef(new Map<string, string | null | undefined>());
  const hydrationRequestRef = useRef<string>();
  const submissionOwnerRef = useRef<string>();
  const [submitting, setSubmitting] = useState(false);
  const [stoppingRunId, setStoppingRunId] = useState<string>();
  const stoppingRunIdRef = useRef<string>();
  const [error, setError] = useState<string>();

  const [approvalSubmitting, setApprovalSubmitting] = useState<
    Partial<Record<string, "once" | "session" | "always" | "deny">>
  >({});
  const [clarifySubmitting, setClarifySubmitting] = useState<Record<string, string>>({});
  const [secretSubmitting, setSecretSubmitting] = useState<Record<string, true>>({});
  const [retryingFailureIds, setRetryingFailureIds] = useState<Record<string, true>>({});
  const [branchingItemId, setBranchingItemId] = useState<string>();
  const [thinkingOpen, setThinkingOpen] = useState<Record<string, boolean>>({});
  const [heroGreetingIndex] = useState(advanceHeroGreetingIndex);
  const scrollRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<HTMLFormElement>(null);
  const [composerClearance, setComposerClearance] = useState(0);
  const [submittedScrollRevision, setSubmittedScrollRevision] = useState(0);
  const selectedIdRef = useRef(selectedId);
  // The last status this workspace announced per session, so a run's stream
  // of deltas announces "running" once instead of on every token.
  const announcedStatusRef = useRef(new Map<string, AgentSessionStatusKind>());
  selectedIdRef.current = selectedId;
  const projectContextRef = useRef(projectContext);
  projectContextRef.current = projectContext;
  const resolveSessionProjectContextRef = useRef(resolveSessionProjectContext);
  resolveSessionProjectContextRef.current = resolveSessionProjectContext;
  const preparePromptForSession = useCallback((storedSessionId: string, prompt: string) => {
    const targetProjectContext =
      resolveSessionProjectContextRef.current?.(storedSessionId) ??
      (selectedIdRef.current === storedSessionId ? projectContextRef.current : undefined);
    return prepareProjectPrompt(
      prompt,
      targetProjectContext,
      projectContextSignaturesBySessionId.get(storedSessionId),
    );
  }, []);
  const requestSubmittedMessageScroll = useCallback(() => {
    setSubmittedScrollRevision((revision) => revision + 1);
  }, []);
  const [homeTaskCards, setHomeTaskCards] = useState<
    Array<{ turn: AgentChatTurn; handoff: HomeTaskHandoff }>
  >([]);
  const homeSessionPromiseRef = useRef<Promise<AgentSessionDto> | null>(null);
  const homeSessionCreationDraftOwnerRef = useRef<string>();
  const [homeSessionCreating, setHomeSessionCreating] = useState(false);

  const startNewSession = useCallback(
    (request?: AgentNewSessionDetail) => {
      hydrationRequestRef.current = undefined;
      setHydratedSelectionId(undefined);
      pendingSessionCreationRef.current = undefined;
      submissionOwnerRef.current = undefined;
      abandonComposerAttachments();
      setSelectedId(undefined);
      selectedIdRef.current = undefined;
      setNewSessionMode(true);

      setProjection(createAgentRuntimeProjection());
      setArtifacts([]);
      setComposerDraft(request?.prompt ?? "");
      setPendingInitialTurn(undefined);
      setSubmitting(false);
      setError(undefined);
      onSessionSelected?.(undefined);
    },
    [abandonComposerAttachments, onSessionSelected, setComposerDraft],
  );

  const selectedSession =
    sessions.find((session) => session.id === selectedId) ?? projection.session;
  const heroMode = !homeMode && newSessionMode && !selectedSession && !pendingInitialTurn;
  const agentSurfaceVisible = heroMode || homeMode;
  useLayoutEffect(() => {
    if (!agentSurfaceVisible) return;
    document.body.dataset.agentSurface = "true";
    return () => {
      delete document.body.dataset.agentSurface;
    };
  }, [agentSurfaceVisible]);
  const running = projection.run?.status === "running" || projection.run?.status === "queued";
  const waiting = projection.run?.status === "waiting_for_user";
  const turns = useMemo(() => agentItemsToChatTurns(projection.items), [projection.items]);
  const visibleTurns = useMemo(() => {
    if (!pendingInitialTurn) return turns;
    if (turns.some((turn) => turn.id === pendingInitialTurn.turn.id)) return turns;
    return [pendingInitialTurn.turn, ...turns];
  }, [pendingInitialTurn, turns]);

  useEffect(() => {
    if (!pendingInitialTurn) return;
    const persisted = projection.items.some(
      (item) =>
        item.kind === "message" &&
        item.role === "user" &&
        !item.id.startsWith("optimistic:") &&
        stripProjectContext(item.text).trim() === pendingInitialTurn.prompt,
    );
    if (persisted) setPendingInitialTurn(undefined);
  }, [pendingInitialTurn, projection.items]);

  const readHomeTaskCards = useCallback(
    (storedSessionId: string) =>
      homeTaskCardTurns(
        readLegacyHomeDirectTurns(storedSessionId),
        readHomeTaskHandoffs(storedSessionId),
      ),
    [],
  );

  useEffect(() => {
    if (!homeMode || !selectedId) return;
    setHomeTaskCards(readHomeTaskCards(selectedId));
    onHomeSessionCreated?.(selectedId);
  }, [homeMode, onHomeSessionCreated, readHomeTaskCards, selectedId]);

  useEffect(() => {
    if (!homeMode) return;
    const refreshHomeThread = (event: Event) => {
      const storedSessionId = (event as CustomEvent<{ storedSessionId?: string }>).detail
        ?.storedSessionId;
      if (!storedSessionId || storedSessionId !== selectedIdRef.current) return;
      setHomeTaskCards(readHomeTaskCards(storedSessionId));
    };
    window.addEventListener(CLOVY_HOME_THREAD_CHANGED_EVENT, refreshHomeThread);
    return () => window.removeEventListener(CLOVY_HOME_THREAD_CHANGED_EVENT, refreshHomeThread);
  }, [homeMode, readHomeTaskCards]);

  const publishSessions = useCallback((loaded: AgentSessionDto[]) => {
    const next = loaded.filter((session) => !isAgentSessionDeleted(session.id));
    sessionsRef.current = next;
    setSessions(next);
    dispatchAgentSessionsChanged({
      sessions: next,
      selectedSessionId: selectedIdRef.current,
      workingSessionIds: next
        .filter((session) => session.status === "running")
        .map((session) => session.id),
      waitingSessionIds: next
        .filter((session) => session.status === "waiting_for_user")
        .map((session) => session.id),
    });
  }, []);

  const refreshSessions = useCallback(async () => {
    const revision = ++sessionListRevisionRef.current;
    const nativeEpoch = readAgentSessionSnapshotEpoch();
    const partition = getCurrentDataPartitionName();
    const loaded = await agentRuntimeBindings.listSessions().catch((cause) => {
      if (
        revision !== sessionListRevisionRef.current ||
        nativeEpoch !== readAgentSessionSnapshotEpoch() ||
        partition !== getCurrentDataPartitionName()
      )
        return undefined;
      throw cause;
    });
    if (
      revision !== sessionListRevisionRef.current ||
      nativeEpoch !== readAgentSessionSnapshotEpoch() ||
      partition !== getCurrentDataPartitionName() ||
      !loaded
    )
      return undefined;
    const next = loaded.filter((session) => !isAgentSessionDeleted(session.id));
    publishSessions(next);
    return next;
  }, [publishSessions]);

  useEffect(() => {
    const acceptSessionSnapshot = (event: Event) => {
      const detail = (event as CustomEvent<AgentSessionsHydratedDetail>).detail;
      if (!detail?.replaceExisting) return;
      // The native-event epoch already fences older reads. A terminal refresh
      // started in that epoch must still publish after this snapshot arrives.
      const next = filterAgentSessionsForDataPartition(
        detail.sessions,
        detail.partitions,
        getCurrentDataPartitionName(),
      ).filter((session) => !isAgentSessionDeleted(session.id));
      sessionsRef.current = next;
      setSessions(next);
    };
    window.addEventListener(AGENT_SESSIONS_HYDRATED_EVENT, acceptSessionSnapshot);
    return () => {
      sessionListRevisionRef.current += 1;
      window.removeEventListener(AGENT_SESSIONS_HYDRATED_EVENT, acceptSessionSnapshot);
    };
  }, []);

  const announceSessionStatus = useCallback((detail: AgentSessionStatusDetail) => {
    if (detail.sessionId && isAgentSessionDeleted(detail.sessionId)) return;
    if (detail.sessionId) announcedStatusRef.current.set(detail.sessionId, detail.status);
    dispatchAgentSessionStatus(detail);
  }, []);

  const applySessionModel = useCallback((storedModel: string) => {
    setModel(storedModel);
  }, []);

  useEffect(() => {
    const applyExternalSessionModel = (event: Event) => {
      const detail = (event as CustomEvent<AgentSessionModelChangedDetail>).detail;
      if (!detail || detail.sessionId !== selectedIdRef.current) return;
      applySessionModel(detail.storedModel);
    };
    window.addEventListener(AGENT_SESSION_MODEL_CHANGED_EVENT, applyExternalSessionModel);
    return () =>
      window.removeEventListener(AGENT_SESSION_MODEL_CHANGED_EVENT, applyExternalSessionModel);
  }, [applySessionModel]);

  const hydrate = useCallback(
    async (sessionId: string) => {
      if (isAgentSessionDeleted(sessionId)) return;
      const readStartedEventIds = projectionEventsRef.current;
      const requestId = crypto.randomUUID();
      hydrationRequestRef.current = requestId;
      if (selectedIdRef.current === sessionId) setHydratedSelectionId(undefined);
      const hydration = await Promise.all([
        agentRuntimeBindings.getSession(sessionId),
        agentRuntimeBindings.listItems(sessionId),
        agentRuntimeBindings.listArtifacts(sessionId),
        agentRuntimeBindings.getLatestRun?.(sessionId) ?? Promise.resolve(null),
      ]).catch((cause) => {
        if (
          isAgentSessionDeleted(sessionId) ||
          selectedIdRef.current !== sessionId ||
          hydrationRequestRef.current !== requestId
        )
          return undefined;
        throw cause;
      });
      if (!hydration) return;
      const [session, items, files, latestRun] = hydration;
      if (
        isAgentSessionDeleted(sessionId) ||
        selectedIdRef.current !== sessionId ||
        hydrationRequestRef.current !== requestId
      ) {
        return;
      }
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
      updateQueuedFollowUps((current) =>
        reconcileConsumedAgentFollowUp(
          current,
          sessionId,
          items.map((item) => item.id),
        ),
      );
      setHydratedSelectionId(sessionId);
      setArtifacts(files);
      if (homeMode) {
        setModel(homeEngineModelRef.current);
      } else {
        applySessionModel(loadSessionModels()[session.id] ?? session.model);
      }
      setSafetyMode(homeMode ? "sandboxed" : session.safetyMode);
      setNewSessionMode(false);
      if (!homeMode) writeLastOpenSessionId(sessionId);
      onSessionSelected?.(session);
    },
    [applySessionModel, homeMode, onSessionSelected, updateQueuedFollowUps],
  );

  // Older builds kept the Home conversation only in local storage: import it
  // into the Home session. The import is idempotent in the database, so every
  // open may ask; the local copy stays as recovery (ADR-0064).
  useEffect(() => {
    if (!homeMode || !selectedId) return;
    const legacyTurns = readLegacyHomeDirectTurns(selectedId);
    if (legacyTurns.length === 0) return;
    void importLegacyHomeTurns(selectedId, legacyTurns)
      .then((result) => {
        if (result.imported > 0 && selectedIdRef.current === selectedId) {
          return hydrate(selectedId);
        }
      })
      .catch(() => undefined);
  }, [homeMode, hydrate, selectedId]);

  useEffect(() => {
    void refreshSessions()
      .then((next) => {
        if (!next) return;
        const selected = selectedIdRef.current;
        if (!homeMode || !selected || next.some((session) => session.id === selected)) return;
        forgetClovyHomeStoredSessionId(getCurrentDataPartitionName(), selected);
        selectedIdRef.current = undefined;
        setSelectedId(undefined);
        setNewSessionMode(true);

        setProjection(createAgentRuntimeProjection());
        setArtifacts([]);
        setError(undefined);
      })
      .catch((cause) => setError(messageFromError(cause)));
    // New work starts on the default chat engine (ADR-0062): a CLI when chat
    // is on one, read without the network, else the saved text model.
    const applyDefaultModel = (selected: string | undefined) => {
      if (homeMode) {
        // Home's turns resolve the engine again at send (homeEngineModelId).
        if (selected?.startsWith(CLI_ENGINE_MODEL_PREFIX)) {
          homeEngineModelRef.current = selected;
          setModel(selected);
        }
      } else if (!initialAgentSession && !initialSessionId && selected) {
        setModel(selected);
      }
    };
    const defaultEngine = chatDefaultEngine().catch(() => null);
    void defaultEngine.then((engine) => {
      if (engine?.modelId) applyDefaultModel(engine.modelId);
    });
  }, [homeMode, initialAgentSession, initialSessionId, refreshSessions]);

  const pendingDraftTakesPriorityRef = useRef(Boolean(pendingRequestRef.current?.prompt));
  useEffect(() => {
    if (!selectedId) return;
    // A request that opens a new session deliberately pre-fills the composer;
    // do not let a stale session draft replace that navigation intent.
    if (pendingDraftTakesPriorityRef.current) {
      pendingDraftTakesPriorityRef.current = false;
      return;
    }
    // The first session id is assigned while its initial send is still in
    // flight. Its composer may already contain a follow-up, which is not a
    // stored draft to restore over.
    if (pendingSessionCreationRef.current) return;
    setComposerDraft(readAgentSessionDraft(selectedId) ?? "", { persist: false });
  }, [selectedId, setComposerDraft]);

  useEffect(() => {
    const nextId = initialSession?.id ?? initialSessionId;
    if (!nextId || isAgentSessionDeleted(nextId)) return;
    if (selectedIdRef.current !== nextId) {
      pendingSessionCreationRef.current = undefined;
      submissionOwnerRef.current = undefined;
      setSubmitting(false);
      abandonComposerAttachments();
    }
    setPendingInitialTurn((current) =>
      current && current.storedSessionId !== nextId ? undefined : current,
    );
    setSelectedId(nextId);
    selectedIdRef.current = nextId;
    if (initialSession) {
      setSessions((current) => [
        initialSession,
        ...current.filter((session) => session.id !== initialSession.id),
      ]);
      setProjection((current) => ({ ...current, session: initialSession }));
      if (!homeMode) {
        applySessionModel(
          loadSessionModels()[initialSession.id] ?? (initialSession.model || DEFAULT_MODEL),
        );
      }
      setSafetyMode(initialSession.safetyMode);
      setNewSessionMode(false);
    }
    void hydrate(nextId).catch((cause) => {
      if (homeMode && selectedIdRef.current !== nextId) return;
      setError(messageFromError(cause));
    });
  }, [
    abandonComposerAttachments,
    applySessionModel,
    homeMode,
    hydrate,
    initialSession?.id,
    initialSessionId,
  ]);

  useEffect(() => {
    const handleNewSession = (event: Event) => {
      const pending = pendingNewSessionRequest();
      const detail = (event as CustomEvent<AgentNewSessionDetail>).detail;
      startNewSession(detail ?? pending);
    };
    window.addEventListener(AGENT_NEW_SESSION_EVENT, handleNewSession);
    return () => window.removeEventListener(AGENT_NEW_SESSION_EVENT, handleNewSession);
  }, [startNewSession]);

  const invalidateDeletedSession = useCallback(
    (storedSessionId: string) => {
      invalidateAgentSessionDraft(storedSessionId);
      projectContextSignaturesBySessionId.delete(storedSessionId);
      forgetSessionModel(storedSessionId);
      forgetLastOpenSessionId(storedSessionId);
      announcedStatusRef.current.delete(storedSessionId);
      setSessions((current) => current.filter((session) => session.id !== storedSessionId));
      updateQueuedFollowUps((current) => {
        if (!(storedSessionId in current)) return current;
        const next = { ...current };
        delete next[storedSessionId];
        return next;
      });
      for (const [request, destination] of creationDraftDestinationsRef.current) {
        if (destination === storedSessionId)
          creationDraftDestinationsRef.current.set(request, null);
      }
      setPendingInitialTurn((current) =>
        current?.storedSessionId === storedSessionId ? undefined : current,
      );
      if (queuedSubmissionSnapshotRef.current?.sessionId === storedSessionId) {
        queuedSubmissionSnapshotRef.current = undefined;
      }
      if (selectedIdRef.current !== storedSessionId) return;
      hydrationRequestRef.current = undefined;
      setHydratedSelectionId(undefined);
      recoverableSubmissionRef.current = undefined;
      recoverableSubmissionSnapshotRef.current = undefined;
      setRecoverableSubmission(undefined);
      setArtifactPanel(null);
      startNewSession();
    },
    [startNewSession, updateQueuedFollowUps],
  );

  useEffect(() => {
    const handleDeletedSession = (event: Event) => {
      const storedSessionId = (event as CustomEvent<{ sessionId?: string }>).detail?.sessionId;
      if (storedSessionId) invalidateDeletedSession(storedSessionId);
    };
    window.addEventListener(AGENT_DELETE_SESSION_EVENT, handleDeletedSession);
    return () => window.removeEventListener(AGENT_DELETE_SESSION_EVENT, handleDeletedSession);
  }, [invalidateDeletedSession]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const backgroundStatus = new Map<string, AgentSessionStatusKind>();
    // One list read at a time; status changes during it ask for one more.
    let refreshInFlight = false;
    let refreshQueued = false;
    const refreshSessionsCoalesced = () => {
      if (refreshInFlight) {
        refreshQueued = true;
        return;
      }
      refreshInFlight = true;
      void refreshSessions()
        .catch(() => undefined)
        .finally(() => {
          refreshInFlight = false;
          if (!refreshQueued || disposed) return;
          refreshQueued = false;
          refreshSessionsCoalesced();
        });
    };
    void listen<AgentRuntimeEvent>(AGENT_RUNTIME_EVENT, ({ payload }) => {
      if (isAgentSessionDeleted(payload.sessionId)) return;
      if (payload.method === "run.diagnostic") {
        if (payload.sessionId === selectedIdRef.current) {
          setProjection((current) => applyAgentRuntimeEvent(current, payload));
        }
        return;
      }
      if (payload.method === "steering.consumed") {
        attemptedQueuedMessageIdsRef.current.delete(payload.data.messageId);
        setFailedQueuedMessageIds((current) => {
          if (!current.has(payload.data.messageId)) return current;
          const next = new Set(current);
          next.delete(payload.data.messageId);
          return next;
        });
        updateQueuedFollowUps((current) =>
          reconcileConsumedAgentFollowUp(current, payload.sessionId, [
            `steering:${payload.data.messageId}`,
          ]),
        );
      }
      const terminal =
        payload.method === "run.completed" ||
        payload.method === "run.cancelled" ||
        payload.method === "run.failed";
      if (terminal) {
        updateQueuedFollowUps((current) =>
          clearQueuedAgentFollowUpSteering(current, payload.sessionId),
        );
      }
      const status: AgentSessionStatusKind =
        payload.method === "interruption.requested"
          ? "waitingForUser"
          : payload.method === "run.completed"
            ? "completed"
            : payload.method === "run.cancelled"
              ? "cancelled"
              : payload.method === "run.failed"
                ? "failed"
                : "running";
      if (payload.sessionId !== selectedIdRef.current) {
        // A background session's row only changes with its status: reread
        // the list when the status moves, not on every streamed token.
        if (backgroundStatus.get(payload.sessionId) !== status) {
          backgroundStatus.set(payload.sessionId, status);
          refreshSessionsCoalesced();
        }
        return;
      }
      backgroundStatus.delete(payload.sessionId);
      setProjection((current) => applyAgentRuntimeEvent(current, payload));
      if (announcedStatusRef.current.get(payload.sessionId) !== status) {
        // The App-level run relay announces failures of every session.
        if (status === "failed") announcedStatusRef.current.set(payload.sessionId, status);
        else announceSessionStatus({ sessionId: payload.sessionId, status });
      }
      if (terminal) {
        setSubmitting(false);
        void Promise.all([hydrate(payload.sessionId), refreshSessions()]).catch((cause) =>
          setError(messageFromError(cause)),
        );
      }
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [announceSessionStatus, hydrate, refreshSessions, updateQueuedFollowUps]);

  useEffect(() => {
    const scroller = scrollRef.current;
    if (scroller && typeof scroller.scrollTo === "function") {
      scroller.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
    }
  }, [projection.items]);

  useLayoutEffect(() => {
    if (submittedScrollRevision === 0) return;
    const scroller = scrollRef.current;
    if (!scroller || typeof scroller.scrollTo !== "function") return;
    scroller.scrollTo({ top: scroller.scrollHeight, behavior: "smooth" });
  }, [submittedScrollRevision]);

  useEffect(() => {
    if (
      running ||
      waiting ||
      submitting ||
      !queuedFollowUp ||
      !selectedId ||
      hydratedSelectionId !== selectedId ||
      failedQueuedMessageIds.has(queuedFollowUp.messageId) ||
      attemptedQueuedMessageIdsRef.current.has(queuedFollowUp.messageId)
    ) {
      return;
    }
    const queued = queuedFollowUp;
    const ownerSessionId = selectedId;
    attemptedQueuedMessageIdsRef.current.add(queued.messageId);
    queuedSubmissionSnapshotRef.current = {
      sessionId: ownerSessionId,
      messageId: queued.messageId,
      prompt: queued.prompt,
      attachments: queued.attachments,
      model: queued.model,
    };
    requestAnimationFrame(() => {
      if (selectedIdRef.current !== ownerSessionId) {
        queuedSubmissionSnapshotRef.current = undefined;
        attemptedQueuedMessageIdsRef.current.delete(queued.messageId);
        return;
      }
      composerRef.current?.requestSubmit();
    });
  }, [
    failedQueuedMessageIds,
    hydratedSelectionId,
    queuedFollowUp,
    running,
    selectedId,
    submitting,
    waiting,
  ]);

  useLayoutEffect(() => {
    const scroller = scrollRef.current;
    const composer = composerRef.current;
    // Home renders its composer before it has a persisted backing session.
    // It is still fixed over the scroller, so skipping clearance in that
    // state lets the greeting and suggestions settle underneath the input.
    // The focused new-session hero is inline and remains the only mode that
    // intentionally needs no fixed-composer clearance.
    if (heroMode || !scroller || !composer) {
      setComposerClearance(0);
      return;
    }
    const measure = () => {
      const next = agentComposerClearance(
        scroller.getBoundingClientRect().bottom,
        composer.getBoundingClientRect().top,
      );
      setComposerClearance((current) => (current === next ? current : next));
    };
    measure();
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(measure) : undefined;
    observer?.observe(scroller);
    observer?.observe(composer);
    window.addEventListener("resize", measure);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [heroMode]);

  useLayoutEffect(() => {
    const shell = document.querySelector(".app-shell");
    shell?.classList.toggle("app-shell-artifact-panel-open", artifactPanel !== null);
    return () => shell?.classList.remove("app-shell-artifact-panel-open");
  }, [artifactPanel]);

  useEffect(() => {
    if (!artifactPanel) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented) {
        setArtifactPanel(null);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [artifactPanel]);

  async function submit(event?: FormEvent) {
    event?.preventDefault();
    if (dropOwnerRef.current) {
      setError(t("agent.error.waitForAttachments"));
      return;
    }
    if (homeMode && !selectedIdRef.current && homeSessionPromiseRef.current) {
      setError(t("agent.home.error.starting"));
      return;
    }
    const queuedSubmission = queuedSubmissionSnapshotRef.current;
    const clearQueuedSubmissionAttempt = () => {
      queuedSubmissionSnapshotRef.current = undefined;
      if (queuedSubmission) {
        attemptedQueuedMessageIdsRef.current.delete(queuedSubmission.messageId);
      }
    };
    if (queuedSubmission && queuedSubmission.sessionId !== selectedIdRef.current) {
      clearQueuedSubmissionAttempt();
      return;
    }
    const recoveredSubmission = recoverableSubmissionSnapshotRef.current;
    const prompt = (
      queuedSubmission?.prompt ??
      recoveredSubmission?.prompt ??
      draftRef.current
    ).trim();
    if (!prompt || waiting || submitting || submissionOwnerRef.current) {
      clearQueuedSubmissionAttempt();
      return;
    }
    const submittedStoredSessionId = selectedIdRef.current;
    let submittedDraftRevision =
      !queuedSubmission &&
      !recoveredSubmission &&
      submittedStoredSessionId &&
      readAgentSessionDraft(submittedStoredSessionId)?.trim() === prompt
        ? readAgentSessionDraftRevision(submittedStoredSessionId)
        : undefined;
    if (running) {
      const submittedAttachments = queuedSubmission?.attachments ?? attachmentsRef.current;
      const submittedModel = queuedSubmission?.model ?? model;
      clearQueuedSubmissionAttempt();
      const ownerStoredSessionId = selectedIdRef.current;
      if (!ownerStoredSessionId) return;
      const messageId = crypto.randomUUID();
      updateQueuedFollowUps((current) => ({
        ...current,
        [ownerStoredSessionId]: mergeQueuedAgentFollowUp(current[ownerStoredSessionId], {
          messageId,
          prompt,
          attachments: submittedAttachments,
          model: submittedModel,
          delivery: "follow_up",
        }),
      }));
      setComposerDraft("", { persist: false });
      setComposerAttachments([]);
      clearAgentSessionDraftRevision(ownerStoredSessionId, submittedDraftRevision);
      requestSubmittedMessageScroll();
      return;
    }
    queuedSubmissionSnapshotRef.current = undefined;
    recoverableSubmissionSnapshotRef.current = undefined;
    const queuedSnapshot = queuedSubmission;
    const recoveredSnapshot = recoveredSubmission;
    const submittedModel = queuedSnapshot?.model ?? recoveredSnapshot?.model ?? model;
    const submissionId = crypto.randomUUID();
    submissionOwnerRef.current = submissionId;
    setSubmitting(true);
    setError(undefined);
    // Home's session is hidden and created on its first send (ensureHomeSession).
    const creatingSession = !homeMode && (!selectedSession || newSessionMode);
    const creationRequestId = creatingSession ? crypto.randomUUID() : undefined;
    const optimisticId = `optimistic:${crypto.randomUUID()}`;
    const optimisticCreatedAt = new Date().toISOString();
    const attachedPaths =
      queuedSnapshot?.attachments ?? recoveredSnapshot?.attachments ?? attachmentsRef.current;
    if (attachedPaths.length > 0) {
      inFlightAttachmentLeasesRef.current.set(submissionId, [...new Set(attachedPaths)]);
    }
    if (creatingSession) {
      pendingSessionCreationRef.current = creationRequestId;
      if (creationRequestId) {
        creationDraftDestinationsRef.current.set(creationRequestId, undefined);
      }
      setPendingInitialTurn({
        prompt,
        title: titleFromPrompt(prompt),
        turn: {
          id: optimisticId,
          createdAt: optimisticCreatedAt,
          role: "user",
          status: "complete",
          parts: [
            ...attachedPaths.map((path): AgentChatTurn["parts"][number] => ({
              type: "attachment",
              name: path.split(/[\\/]/).pop() || path,
              path,
              kind: /\.(?:avif|gif|jpe?g|png|webp)$/i.test(path) ? "image" : "file",
            })),
            { type: "text", text: prompt, status: "complete" },
          ],
        },
      });
      if (submissionOwnerRef.current === submissionId && !recoveredSnapshot) {
        setComposerDraft("", { persist: false });
        setComposerAttachments([]);
      }
    }
    try {
      let session = selectedSession;
      if (homeMode) {
        session = await ensureHomeSession();
        if (!submittedStoredSessionId && readAgentSessionDraft(session.id)?.trim() === prompt) {
          submittedDraftRevision = readAgentSessionDraftRevision(session.id);
        }
      } else if (!session || newSessionMode) {
        const createdSession = await agentRuntimeBindings.createSession({
          title: titleFromPrompt(prompt),
          model: submittedModel,
          safetyMode,
          profile: getCurrentDataPartitionName(),
        });
        if (isAgentSessionDeleted(createdSession.id)) {
          throw new Error(t("agent.error.conversationDeleted"));
        }
        session = createdSession;
        sessionListRevisionRef.current += 1;
        const latestModel = chatModelId(modelRef.current, engineCatalogRef.current);
        if (latestModel !== submittedModel) {
          rememberSessionModel(createdSession.id, latestModel);
        }
        const nextSessions = [
          createdSession,
          ...sessionsRef.current.filter((item) => item.id !== createdSession.id),
        ];
        sessionsRef.current = nextSessions;
        setSessions(nextSessions);
        if (creationRequestId) {
          creationDraftDestinationsRef.current.set(creationRequestId, createdSession.id);
        }
        const transferredPendingDraft = creationRequestId
          ? transferPendingAgentSessionDraft(creationRequestId, createdSession.id)
          : false;
        const shouldPresentCreatedSession =
          pendingSessionCreationRef.current === creationRequestId &&
          selectedIdRef.current === undefined;
        if (shouldPresentCreatedSession) {
          setSelectedId(createdSession.id);
          selectedIdRef.current = createdSession.id;
          if (!transferredPendingDraft) {
            writeAgentSessionDraft(createdSession.id, draftRef.current);
          }
          setNewSessionMode(false);
          setPendingInitialTurn((current) =>
            current ? { ...current, storedSessionId: createdSession.id } : current,
          );
          onSessionSelected?.(createdSession);
          writeLastOpenSessionId(createdSession.id);
        }
      }
      const activeSession = session;
      if (isAgentSessionDeleted(activeSession.id)) {
        throw new Error(t("agent.error.conversationDeleted"));
      }
      const optimistic: AgentItemDto = {
        id: optimisticId,
        sessionId: activeSession.id,
        sequence: Math.max(0, ...projection.items.map((item) => item.sequence)) + 1,
        createdAt: optimisticCreatedAt,
        kind: "message",
        role: "user",
        text: prompt,
        status: "complete",
        attachments: attachedPaths.map((path, index) => ({
          id: `attachment:${index}:${path}`,
          sessionId: activeSession.id,
          name: path.split(/[\\/]/).pop() || path,
          path,
          action: "imported",
          available: true,
          createdAt: new Date().toISOString(),
        })),
      };
      if (selectedIdRef.current === activeSession.id) {
        setProjection((current) => ({
          ...current,
          session: activeSession,
          items: [...current.items, optimistic],
        }));
        requestSubmittedMessageScroll();
      }
      if (
        !queuedSnapshot &&
        !recoveredSnapshot &&
        !creatingSession &&
        submissionOwnerRef.current === submissionId
      ) {
        setComposerDraft("", { persist: false });
        setComposerAttachments([]);
      }
      const enabledSkillIds = (await agentRuntimeBindings.listSkills())
        .filter((skill) => skill.enabled)
        .map((skill) => skill.id);
      const preparedPrompt = preparePromptForSession(activeSession.id, prompt);
      const needsLocalDefault =
        homeMode || (!engineCatalog && !submittedModel.startsWith(CLI_ENGINE_MODEL_PREFIX));
      const runModel = needsLocalDefault
        ? await defaultChatModelId()
        : chatModelId(submittedModel, engineCatalog);
      if (isAgentSessionDeleted(activeSession.id)) {
        throw new Error(t("agent.error.conversationDeleted"));
      }
      const run = await agentRuntimeBindings.startRun({
        sessionId: activeSession.id,
        prompt: preparedPrompt.text,
        model: runModel,
        safetyMode: homeMode ? "sandboxed" : safetyMode,
        surface: homeMode ? "home" : "chat",
        workspacePath: activeSession.workspacePath,
        enabledSkillIds,
        attachments: attachedPaths,
      });
      void discardStagedAgentAttachments(attachedPaths).catch(() => undefined);
      clearAgentSessionDraftRevision(activeSession.id, submittedDraftRevision);
      inFlightAttachmentLeasesRef.current.delete(submissionId);
      if (queuedSnapshot) {
        attemptedQueuedMessageIdsRef.current.delete(queuedSnapshot.messageId);
        setFailedQueuedMessageIds((current) => {
          if (!current.has(queuedSnapshot.messageId)) return current;
          const next = new Set(current);
          next.delete(queuedSnapshot.messageId);
          return next;
        });
        updateQueuedFollowUps((current) => {
          if (current[queuedSnapshot.sessionId]?.messageId !== queuedSnapshot.messageId) {
            return current;
          }
          const next = { ...current };
          delete next[queuedSnapshot.sessionId];
          return next;
        });
      }
      if (recoveredSnapshot) {
        setRecoverableSubmission((current) =>
          current?.id === recoveredSnapshot.id ? undefined : current,
        );
      }
      clearSessionModelIfApplied(activeSession.id, submittedModel);
      projectContextSignaturesBySessionId.set(activeSession.id, preparedPrompt.contextSignature);
      if (selectedIdRef.current === activeSession.id) {
        setProjection((current) => ({ ...current, run }));
      }
      if (pendingSessionCreationRef.current === creationRequestId) {
        pendingSessionCreationRef.current = undefined;
      }
      if (submissionOwnerRef.current === submissionId) {
        submissionOwnerRef.current = undefined;
        setSubmitting(false);
      }
      announceSessionStatus({
        sessionId: activeSession.id,
        title: activeSession.title,
        status: "starting",
      });
      await refreshSessions();
    } catch (cause) {
      if (
        creationRequestId &&
        creationDraftDestinationsRef.current.has(creationRequestId) &&
        creationDraftDestinationsRef.current.get(creationRequestId) === undefined
      ) {
        clearPendingAgentSessionDraft(creationRequestId);
        creationDraftDestinationsRef.current.set(creationRequestId, null);
      }
      if (queuedSnapshot) {
        setFailedQueuedMessageIds((current) => new Set([...current, queuedSnapshot.messageId]));
      }
      if (submissionOwnerRef.current !== submissionId) {
        inFlightAttachmentLeasesRef.current.delete(submissionId);
        if (!queuedSnapshot && !recoveredSnapshot) {
          void discardStagedAgentAttachments(attachedPaths).catch(() => undefined);
        }
        return;
      }
      submissionOwnerRef.current = undefined;
      setSubmitting(false);
      const operationIsVisible = homeMode
        ? true
        : creationRequestId
          ? pendingSessionCreationRef.current === creationRequestId
          : selectedIdRef.current === selectedSession?.id;
      if (!operationIsVisible) {
        inFlightAttachmentLeasesRef.current.delete(submissionId);
        if (!queuedSnapshot && !recoveredSnapshot) {
          void discardStagedAgentAttachments(attachedPaths).catch(() => undefined);
        }
        return;
      }
      if (creatingSession) {
        if (pendingSessionCreationRef.current === creationRequestId) {
          pendingSessionCreationRef.current = undefined;
        }
        if (!selectedIdRef.current) setPendingInitialTurn(undefined);
        if (!selectedIdRef.current) {
          setNewSessionMode(true);
        }
      }
      if (!queuedSnapshot) {
        const failedSubmission = {
          id: recoveredSnapshot?.id ?? crypto.randomUUID(),
          prompt,
          attachments: attachedPaths,
          model: submittedModel,
        };
        if (recoveredSnapshot || draftRef.current.trim() || attachmentsRef.current.length > 0) {
          setRecoverableSubmission(failedSubmission);
        } else {
          setComposerDraft(prompt);
          setComposerAttachments(attachedPaths);
        }
      }
      inFlightAttachmentLeasesRef.current.delete(submissionId);
      setError(messageFromError(cause));
    }
  }

  /** The Home session, created on the default CLI engine at the first send
   * (ADR-0064). An older Home session adopts the engine with its next turn:
   * `start_agent_run` stores the turn's model on the session. */
  async function ensureHomeSession(): Promise<AgentSessionDto> {
    const selected = selectedIdRef.current;
    if (selected) {
      const existing =
        selectedSession?.id === selected
          ? selectedSession
          : await agentRuntimeBindings.getSession(selected);
      if (isAgentSessionDeleted(existing.id)) {
        throw new Error(t("agent.error.conversationDeleted"));
      }
      return existing;
    }
    if (homeSessionPromiseRef.current) return homeSessionPromiseRef.current;

    const draftOwnerId = crypto.randomUUID();
    homeSessionCreationDraftOwnerRef.current = draftOwnerId;
    creationDraftDestinationsRef.current.set(draftOwnerId, undefined);
    const creation = homeEngineModelId()
      .then((engineModel) =>
        agentRuntimeBindings.createSession({
          title: "Home",
          model: engineModel,
          safetyMode: "sandboxed",
          profile: getCurrentDataPartitionName(),
        }),
      )
      .then((createdSession) => {
        if (isAgentSessionDeleted(createdSession.id)) {
          throw new Error(t("agent.error.conversationDeleted"));
        }
        sessionListRevisionRef.current += 1;
        creationDraftDestinationsRef.current.set(draftOwnerId, createdSession.id);
        const transferred = transferPendingAgentSessionDraft(draftOwnerId, createdSession.id);
        if (!transferred) {
          writeAgentSessionDraft(createdSession.id, draftRef.current);
        }
        setSelectedId(createdSession.id);
        selectedIdRef.current = createdSession.id;
        setNewSessionMode(false);
        const nextSessions = [
          createdSession,
          ...sessionsRef.current.filter((session) => session.id !== createdSession.id),
        ];
        sessionsRef.current = nextSessions;
        setSessions(nextSessions);
        setProjection(createAgentRuntimeProjection({ session: createdSession }));
        onHomeSessionCreated?.(createdSession.id);
        return createdSession;
      })
      .catch((cause) => {
        clearPendingAgentSessionDraft(draftOwnerId);
        creationDraftDestinationsRef.current.set(draftOwnerId, null);
        throw cause;
      });
    homeSessionPromiseRef.current = creation;
    setHomeSessionCreating(true);
    try {
      return await creation;
    } finally {
      if (homeSessionPromiseRef.current === creation) {
        homeSessionPromiseRef.current = null;
        if (homeSessionCreationDraftOwnerRef.current === draftOwnerId) {
          homeSessionCreationDraftOwnerRef.current = undefined;
        }
        setHomeSessionCreating(false);
      }
    }
  }

  async function stop() {
    const runId = projection.run?.id;
    if (!runId || stoppingRunIdRef.current === runId) return;
    stoppingRunIdRef.current = runId;
    setStoppingRunId(runId);
    let cancellationFailed = false;
    try {
      await agentRuntimeBindings.cancelRun(runId);
    } catch (cause) {
      cancellationFailed = true;
      setError(messageFromError(cause));
    } finally {
      const sessionId = selectedIdRef.current;
      try {
        if (sessionId) {
          await Promise.all([hydrate(sessionId), refreshSessions()]);
        }
      } catch (cause) {
        if (!cancellationFailed) setError(messageFromError(cause));
      } finally {
        if (stoppingRunIdRef.current === runId) {
          stoppingRunIdRef.current = undefined;
          setStoppingRunId(undefined);
        }
      }
    }
  }

  async function retryFailure(itemId: string) {
    const failedItem = projection.items.find((item) => item.id === itemId && item.kind === "error");
    if (!failedItem?.runId || running || waiting || submitting || retryingFailureIds[itemId])
      return;
    setRetryingFailureIds((current) => ({ ...current, [itemId]: true }));
    setError(undefined);
    try {
      const run = await agentRuntimeBindings.retryRun(failedItem.runId);
      setProjection((current) => ({ ...current, run }));
      announceSessionStatus({
        sessionId: failedItem.sessionId,
        title: selectedSession?.title,
        status: "starting",
      });
      await refreshSessions();
    } catch (cause) {
      setRetryingFailureIds((current) => {
        const next = { ...current };
        delete next[itemId];
        return next;
      });
      setError(messageFromError(cause));
    }
  }

  async function respondToApproval(
    runId: string,
    interruptionId: string,
    choice: "once" | "session" | "always" | "deny",
  ) {
    setApprovalSubmitting((current) => ({ ...current, [interruptionId]: choice }));
    const request: ResolveAgentInterruptionRequest = {
      runId,
      interruptionId,
      resolution: { kind: "approval", choice },
    };
    try {
      const run = await agentRuntimeBindings.resolveInterruption(request);
      setProjection((current) => ({
        ...current,
        run,
        items: reconcileAgentInterruptionResolution(current.items, request),
      }));
    } catch (cause) {
      setError(messageFromError(cause));
    } finally {
      setApprovalSubmitting((current) => {
        const next = { ...current };
        delete next[interruptionId];
        return next;
      });
    }
  }

  async function respondToClarification(runId: string, interruptionId: string, answer: string) {
    setClarifySubmitting((current) => ({ ...current, [interruptionId]: answer }));
    const request: ResolveAgentInterruptionRequest = {
      runId,
      interruptionId,
      resolution: { kind: "clarification", answer },
    };
    try {
      const run = await agentRuntimeBindings.resolveInterruption(request);
      setProjection((current) => ({
        ...current,
        run,
        items: reconcileAgentInterruptionResolution(current.items, request),
      }));
    } catch (cause) {
      setError(messageFromError(cause));
    } finally {
      setClarifySubmitting((current) => {
        const next = { ...current };
        delete next[interruptionId];
        return next;
      });
    }
  }

  async function respondToSecret(runId: string, interruptionId: string, secret: string) {
    setSecretSubmitting((current) => ({ ...current, [interruptionId]: true }));
    const request: ResolveAgentInterruptionRequest = {
      runId,
      interruptionId,
      resolution: secret
        ? { kind: "secret", secret, choice: "once" }
        : { kind: "secret", choice: "deny" },
    };
    try {
      const run = await agentRuntimeBindings.resolveInterruption(request);
      setProjection((current) => ({
        ...current,
        run,
        items: reconcileAgentInterruptionResolution(current.items, request),
      }));
    } catch (cause) {
      setError(messageFromError(cause));
    } finally {
      setSecretSubmitting((current) => {
        const next = { ...current };
        delete next[interruptionId];
        return next;
      });
    }
  }

  async function branchFrom(itemId: string) {
    if (!selectedId || !agentRuntimeBindings.branchSession) return;
    setBranchingItemId(itemId);
    setError(undefined);
    try {
      const branch = await agentRuntimeBindings.branchSession(selectedId, itemId);
      if (isAgentSessionDeleted(branch.id)) {
        throw new Error(t("agent.error.createdConversationDeleted"));
      }
      sessionListRevisionRef.current += 1;
      abandonComposerAttachments();
      const nextSessions = [branch, ...sessionsRef.current.filter((item) => item.id !== branch.id)];
      sessionsRef.current = nextSessions;
      setSessions(nextSessions);
      setSelectedId(branch.id);
      selectedIdRef.current = branch.id;
      setNewSessionMode(false);
      await hydrate(branch.id);
    } catch (cause) {
      setError(messageFromError(cause));
    } finally {
      setBranchingItemId(undefined);
    }
  }

  async function pickAttachments() {
    const selected = await openFileDialog({
      multiple: true,
      title: t("agent.composer.attachFiles"),
    });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    setComposerAttachments((current) => [...new Set([...current, ...paths])].slice(0, 8));
  }

  async function addDroppedAttachments(files: File[]) {
    if (dropOwnerRef.current) {
      setError(t("agent.error.dropBusy"));
      return;
    }
    const owner = crypto.randomUUID();
    const generation = attachmentGenerationRef.current;
    dropOwnerRef.current = owner;
    setAttaching(true);
    try {
      const protectedPaths = [
        ...attachmentsRef.current,
        ...Object.values(queuedFollowUpsRef.current).flatMap((followUp) => followUp.attachments),
        ...(recoverableSubmissionRef.current?.attachments ?? []),
        ...[...inFlightAttachmentLeasesRef.current.values()].flat(),
      ];
      await pruneStagedAgentAttachments([...new Set(protectedPaths)]);
      if (attachmentGenerationRef.current !== generation || dropOwnerRef.current !== owner) return;
      const paths = await stageDroppedAgentFiles(files, attachmentsRef.current.length);
      if (attachmentGenerationRef.current !== generation || dropOwnerRef.current !== owner) {
        void discardStagedAgentAttachments(paths).catch(() => undefined);
        return;
      }
      if (attachmentsRef.current.length + paths.length > 8) {
        void discardStagedAgentAttachments(paths).catch(() => undefined);
        setError(t("agent.error.tooManyFiles"));
        return;
      }
      setComposerAttachments((current) => [...new Set([...current, ...paths])]);
      setError(undefined);
    } catch (cause) {
      if (attachmentGenerationRef.current === generation && dropOwnerRef.current === owner) {
        setError(messageFromError(cause));
      }
    } finally {
      if (dropOwnerRef.current === owner) {
        dropOwnerRef.current = undefined;
        setAttaching(false);
      }
    }
  }

  function removeAttachment(path: string) {
    setComposerAttachments((current) => current.filter((item) => item !== path));
    void discardStagedAgentAttachments([path]).catch(() => undefined);
  }

  async function startDictation() {
    try {
      await dictationHelperCommand({ type: "start_listening" });
    } catch (cause) {
      setError(messageFromError(cause));
    }
  }

  async function rename(title: string) {
    if (!selectedId) return;
    const updated = await agentRuntimeBindings.renameSession(selectedId, title);
    setSessions((current) => current.map((item) => (item.id === updated.id ? updated : item)));
    setProjection((current) => ({ ...current, session: updated }));
    onSessionSelected?.(updated);
  }

  async function remove() {
    if (!selectedId) return;
    const removedStoredSessionId = selectedId;
    await agentRuntimeBindings.deleteSession(removedStoredSessionId);
    invalidateDeletedSession(removedStoredSessionId);
    await refreshSessions();
  }

  const sessionActionsAvailable = Boolean(selectedId && selectedSession);
  const homeConversationTurns = useMemo(
    () =>
      [
        ...turns.map((turn) =>
          turn.role === "user"
            ? {
                ...turn,
                parts: turn.parts.map((part) =>
                  part.type === "text"
                    ? {
                        ...part,
                        text: stripClovyHomeContextFromPreview(part.text) ?? part.text,
                      }
                    : part,
                ),
              }
            : turn,
        ),
        ...homeTaskCards.map((card) => card.turn),
      ].sort(compareHomeTurnOrder),
    [homeTaskCards, turns],
  );
  const homeHandoffsByTurnId = useMemo(
    () => new Map(homeTaskCards.map((card) => [card.turn.id, card.handoff])),
    [homeTaskCards],
  );
  const homeUserRunEndIds = useMemo(() => {
    const ids = new Set<string>();
    homeConversationTurns.forEach((turn, index) => {
      if (turn.role === "user" && homeConversationTurns[index + 1]?.role !== "user") {
        ids.add(turn.id);
      }
    });
    return ids;
  }, [homeConversationTurns]);
  const homeCheckIn = homeMode
    ? clovyHomeDailyCheckIn(getCurrentDataPartitionName())
    : { createdAt: "", text: "" };
  const lastHomeTurn = homeConversationTurns.at(-1);
  const homeGreetingVisible =
    homeMode && (!lastHomeTurn || lastHomeTurn.createdAt.localeCompare(homeCheckIn.createdAt) < 0);
  const homeGreeting = clovyHomeGreetingParts(new Date(), {
    returning: homeConversationTurns.length > 0,
  });
  const homeNudgePrompts = clovyHomeNudgePrompts(new Date());
  const renderedArtifacts = artifacts.filter((artifact) => artifact.available).map(artifactView);
  const openArtifact = (artifact: AgentArtifact) => setArtifactPanel({ view: "file", artifact });
  const downloadArtifact = async (artifact: AgentArtifact) => {
    try {
      await downloadAgentArtifact(artifact.path);
    } catch (cause) {
      setError(messageFromError(cause));
    }
  };
  const sessionModel = projection.run?.model ?? selectedSession?.model ?? model;
  const sessionCli = sessionModel.startsWith(CLI_ENGINE_MODEL_PREFIX)
    ? sessionModel.slice(CLI_ENGINE_MODEL_PREFIX.length)
    : undefined;
  const sessionDisplayModel =
    engineCatalog?.clis.find((cli) => cli.id === sessionCli)?.name ??
    (sessionCli === "claude"
      ? "Claude Code"
      : sessionCli === "codex"
        ? "Codex"
        : (sessionCli ?? sessionModel));
  const toolUsage = [...projection.items]
    .filter(
      (item) =>
        item.kind === "tool_call" &&
        (projection.run?.id === undefined || item.runId === projection.run.id),
    )
    .reduce<Map<string, { calls: number; failures: number }>>((summary, item) => {
      if (item.kind !== "tool_call") return summary;
      const current = summary.get(item.name) ?? { calls: 0, failures: 0 };
      current.calls += 1;
      if (item.status === "failed") current.failures += 1;
      summary.set(item.name, current);
      return summary;
    }, new Map());
  const recoverableSubmissionRow = recoverableSubmission ? (
    <div className="agent-follow-up-row" role="status">
      <span className="agent-follow-up-copy">
        <span className="agent-follow-up-announcement">{t("agent.unsent.label")}</span>
        <span className="agent-follow-up-text">{recoverableSubmission.prompt}</span>
      </span>
      <span className="agent-follow-up-actions">
        <button
          type="button"
          aria-label={t("agent.unsent.retry")}
          disabled={running || waiting || submitting}
          onClick={() => {
            recoverableSubmissionSnapshotRef.current = recoverableSubmission;
            setError(undefined);
            requestAnimationFrame(() => composerRef.current?.requestSubmit());
          }}
        >
          <IconArrowRotateClockwise size={12} aria-hidden />
        </button>
        <button
          type="button"
          aria-label={t("agent.unsent.discard")}
          onClick={() => {
            if (recoverableSubmissionSnapshotRef.current?.id === recoverableSubmission.id) {
              recoverableSubmissionSnapshotRef.current = undefined;
            }
            void discardStagedAgentAttachments(recoverableSubmission.attachments).catch(
              () => undefined,
            );
            setRecoverableSubmission(undefined);
          }}
        >
          <IconCrossSmall size={12} aria-hidden />
        </button>
      </span>
    </div>
  ) : null;
  function persistComposerDraftForOwner(
    text: string,
    draftOwnerId?: string | null,
  ): string | undefined {
    if (!draftOwnerId) return undefined;
    if (creationDraftDestinationsRef.current.has(draftOwnerId)) {
      const destinationStoredSessionId = creationDraftDestinationsRef.current.get(draftOwnerId);
      if (destinationStoredSessionId) {
        writeAgentSessionDraft(destinationStoredSessionId, text);
        return destinationStoredSessionId;
      } else if (destinationStoredSessionId === undefined) {
        writePendingAgentSessionDraft(draftOwnerId, text);
        return draftOwnerId;
      }
      return undefined;
    }
    writeAgentSessionDraft(draftOwnerId, text);
    return draftOwnerId;
  }

  const draftOwnerId =
    selectedId ?? pendingSessionCreationRef.current ?? homeSessionCreationDraftOwnerRef.current;
  const composer = (
    <AgentComposer
      formRef={composerRef}
      scrollRef={scrollRef}
      draft={draft}
      draftRevision={draftRevision}
      setDraft={setComposerDraft}
      draftOwnerId={draftOwnerId}
      onEditorDraftChange={(text, changedDraftOwnerId) => {
        const resolvedDraftOwnerId = persistComposerDraftForOwner(text, changedDraftOwnerId);
        const currentDraftOwnerId =
          selectedIdRef.current ??
          pendingSessionCreationRef.current ??
          homeSessionCreationDraftOwnerRef.current;
        if (
          resolvedDraftOwnerId === currentDraftOwnerId ||
          (!resolvedDraftOwnerId && !currentDraftOwnerId && !changedDraftOwnerId)
        ) {
          setComposerDraft(text, { persist: false });
        }
        return resolvedDraftOwnerId;
      }}
      onPendingDraftPersist={persistComposerDraftForOwner}
      onDraftContentChange={(hasContent) => {
        draftHasContentRef.current = hasContent;
      }}
      model={model}
      setModel={(nextModel) => {
        setModel(nextModel);
        if (selectedId) {
          rememberSessionModel(selectedId, nextModel);
          return;
        }
        if (pendingSessionCreationRef.current) return;
        const engine = classifyChatEngineModel(nextModel);
        void llmSetUsage("chat", { kind: "cli", id: engine.cli as LlmCliId }).catch((cause) => {
          if (modelRef.current === nextModel) setError(messageFromError(cause));
        });
      }}
      safetyMode={safetyMode}
      setSafetyMode={setSafetyMode}
      attachments={attachments}
      onDropAttachments={addDroppedAttachments}
      onRemoveAttachment={removeAttachment}
      onPickAttachments={pickAttachments}
      onDictate={startDictation}
      onSubmit={submit}
      onStop={stop}
      running={running}
      waiting={waiting}
      stopping={stoppingRunId === projection.run?.id}
      submitting={submitting || (homeMode && homeSessionCreating)}
      attaching={attaching}
      notice={!heroMode ? error : undefined}
      hero={heroMode}
      showModelPicker={!homeMode}
      engineCatalog={engineCatalog}
      onRefreshEngineCatalog={refreshEngineCatalog}
    />
  );
  return (
    <>
      <section
        className="agent-workspace"
        aria-label={homeMode ? t("agent.workspace.homeLabel") : t("agent.workspace.sessionLabel")}
        data-hero={heroMode ? "true" : undefined}
        data-home={homeMode ? "true" : undefined}
      >
        {heroMode || homeMode ? <AgentBackground key={homeMode ? "home" : "hero"} /> : null}
        {!heroMode && !homeMode ? (
          <AgentSessionBar
            origin={origin}
            title={selectedSession?.title ?? pendingInitialTurn?.title ?? ""}
            fullMode={selectedSession?.safetyMode === "unrestricted"}
            artifactCount={renderedArtifacts.length}
            artifactsOpen={artifactPanel !== null}
            onToggleArtifacts={
              sessionActionsAvailable
                ? () => setArtifactPanel((current) => (current ? null : { view: "list" }))
                : undefined
            }
            inProject={sessionInProject}
            projectContext={projectContext}
            onUsage={sessionActionsAvailable ? () => setUsageOpen(true) : undefined}
            onRename={sessionActionsAvailable ? rename : undefined}
            onMoveToProject={
              sessionActionsAvailable && selectedId && onMoveSessionToProject
                ? () => onMoveSessionToProject(selectedId)
                : undefined
            }
            onDelete={sessionActionsAvailable ? remove : undefined}
          />
        ) : null}
        {homeMode ? (
          <div
            ref={scrollRef}
            className="agent-scroll"
            style={{ "--agent-composer-clearance": `${composerClearance}px` } as CSSProperties}
          >
            <main className="agent-main" aria-label={t("agent.home.conversationLabel")}>
              <div className="agent-timeline" data-home="true">
                {homeConversationTurns.map((turn, index) => {
                  const previous = index > 0 ? homeConversationTurns[index - 1] : undefined;
                  const dayKey = clovyHomeDayKey(turn.createdAt);
                  const dayMarker =
                    previous && dayKey && dayKey !== clovyHomeDayKey(previous.createdAt) ? (
                      <div className="agent-home-day">{clovyHomeDayLabel(turn.createdAt)}</div>
                    ) : null;
                  return (
                    <Fragment key={turn.id}>
                      {dayMarker}
                      <AgentChatTurnRow
                        turn={turn}
                        approvalSubmitting={approvalSubmitting}
                        clarifySubmitting={clarifySubmitting}
                        sudoSubmitting={{}}
                        secretSubmitting={secretSubmitting}
                        thinkingOpen={(key) => thinkingOpen[key] ?? false}
                        onThinkingOpenChange={(key, open) =>
                          setThinkingOpen((current) => ({ ...current, [key]: open }))
                        }
                        onApproval={(part, choice) =>
                          void respondToApproval(part.runId, part.id, choice)
                        }
                        onClarify={(part, answer) =>
                          void respondToClarification(part.runId, part.id, answer)
                        }
                        onSudo={() => undefined}
                        onSecret={(part, secret) =>
                          void respondToSecret(part.runId, part.id, secret)
                        }
                        onOpenArtifact={openArtifact}
                        onRetryUpstreamFailure={(turnId) => void retryFailure(turnId)}
                        upstreamFailureRetryAttempted={Boolean(retryingFailureIds[turn.id])}
                        upstreamFailureRetryDisabled={running || waiting || submitting}
                        homeTaskHandoff={homeHandoffsByTurnId.get(turn.id)}
                        onOpenHomeTaskSession={onOpenHomeTaskSession}
                        homeUserRunEnd={homeUserRunEndIds.has(turn.id)}
                      />
                    </Fragment>
                  );
                })}
                {homeGreetingVisible ? (
                  <>
                    {homeConversationTurns.length > 0 &&
                    clovyHomeDayKey(homeCheckIn.createdAt) !==
                      clovyHomeDayKey(homeConversationTurns.at(-1)?.createdAt ?? "") ? (
                      <div className="agent-home-day">
                        {clovyHomeDayLabel(homeCheckIn.createdAt)}
                      </div>
                    ) : null}
                    <div className="agent-home-greeting">
                      <span className="agent-home-greeting-mark" aria-hidden>
                        <ClovyAlive
                          className="clovy-home-mark"
                          palette="appearance"
                          width={30}
                          height={31}
                        />
                      </span>
                      <h2>{homeGreeting.salutation}</h2>
                      <p>{homeGreeting.question}</p>
                    </div>
                  </>
                ) : null}
                {homeConversationTurns.length === 0 ? (
                  <section className="agent-home-nudges" aria-label={t("agent.home.suggestions")}>
                    {homeNudgePrompts.map((prompt) => (
                      <button key={prompt} type="button" onClick={() => setComposerDraft(prompt)}>
                        {prompt}
                      </button>
                    ))}
                  </section>
                ) : null}
                <AgentThinking
                  variant="typing-bubble"
                  visible={(running || submitting) && homeConversationTurns.at(-1)?.role === "user"}
                />
              </div>
              {recoverableSubmissionRow}
              {composer}
            </main>
          </div>
        ) : heroMode ? (
          <main
            className="agent-main"
            aria-label={t("agent.workspace.taskDetails")}
            data-hero="true"
          >
            {error ? (
              <div className="agent-composer-notice" role="alert">
                {error}
              </div>
            ) : null}
            <div className="agent-hero-heading">
              <h2 className="agent-hero-title">{heroGreeting(heroGreetingIndex)}</h2>
            </div>
            {recoverableSubmissionRow}
            {composer}
            <div className="agent-hero-suggestions">
              <div className="agent-hero-chips" data-hidden={draft.trim() ? "true" : undefined}>
                {AGENT_SHORTCUTS.slice(0, 3).map((shortcut, index) => (
                  <button
                    key={shortcut.key}
                    type="button"
                    className="agent-hero-chip"
                    style={{ "--chip-i": index } as CSSProperties}
                    title={shortcut.description}
                    disabled={submitting}
                    onClick={() => setComposerDraft(shortcut.prompt)}
                  >
                    <span className="agent-hero-chip-icon" aria-hidden>
                      {shortcut.icon}
                    </span>
                    {shortcut.title}
                  </button>
                ))}
              </div>
            </div>
          </main>
        ) : (
          <div
            ref={scrollRef}
            className="agent-scroll"
            style={{ "--agent-composer-clearance": `${composerClearance}px` } as CSSProperties}
          >
            <main className="agent-main" aria-label={t("agent.workspace.taskDetails")}>
              <div className="agent-timeline">
                {visibleTurns.map((turn) => (
                  <AgentChatTurnRow
                    key={turn.id}
                    turn={turn}
                    approvalSubmitting={approvalSubmitting}
                    clarifySubmitting={clarifySubmitting}
                    sudoSubmitting={{}}
                    secretSubmitting={secretSubmitting}
                    thinkingOpen={(key) => thinkingOpen[key] ?? false}
                    onThinkingOpenChange={(key, open) =>
                      setThinkingOpen((current) => ({ ...current, [key]: open }))
                    }
                    onApproval={(part, choice) =>
                      void respondToApproval(part.runId, part.id, choice)
                    }
                    onClarify={(part, answer) =>
                      void respondToClarification(part.runId, part.id, answer)
                    }
                    onSudo={() => undefined}
                    onSecret={(part, secret) => void respondToSecret(part.runId, part.id, secret)}
                    onOpenArtifact={openArtifact}
                    onRetryUpstreamFailure={(turnId) => void retryFailure(turnId)}
                    onBranch={(itemId) => void branchFrom(itemId)}
                    branching={branchingItemId === turn.id}
                    upstreamFailureRetryAttempted={Boolean(retryingFailureIds[turn.id])}
                    upstreamFailureRetryDisabled={running || waiting || submitting}
                  />
                ))}
                <AgentThinking
                  visible={(running || submitting) && visibleTurns.at(-1)?.role === "user"}
                />
              </div>
              {recoverableSubmissionRow}
              {queuedFollowUp ? (
                <div className="agent-follow-up-row" role="status">
                  <span className="agent-follow-up-copy">
                    <span className="agent-follow-up-text">
                      {queuedFollowUpText(queuedFollowUp)}
                    </span>
                    <span className="agent-follow-up-status">
                      {queuedFollowUpStatus(
                        queuedFollowUp,
                        failedQueuedMessageIds.has(queuedFollowUp.messageId),
                      )}
                    </span>
                  </span>
                  <span className="agent-follow-up-actions">
                    {failedQueuedMessageIds.has(queuedFollowUp.messageId) ? (
                      <button
                        type="button"
                        aria-label={
                          queuedFollowUp.delivery === "attachments"
                            ? t("agent.queued.retryAttachments")
                            : t("agent.queued.retryFollowUp")
                        }
                        disabled={running || waiting || submitting}
                        onClick={() => {
                          attemptedQueuedMessageIdsRef.current.delete(queuedFollowUp.messageId);
                          setFailedQueuedMessageIds((current) => {
                            if (!current.has(queuedFollowUp.messageId)) return current;
                            const next = new Set(current);
                            next.delete(queuedFollowUp.messageId);
                            return next;
                          });
                          setError(undefined);
                        }}
                      >
                        <IconArrowRotateClockwise size={12} aria-hidden />
                      </button>
                    ) : null}
                    <button
                      type="button"
                      aria-label={
                        queuedFollowUp.delivery === "attachments"
                          ? t("agent.queued.removeAttachments")
                          : t("agent.queued.removeFollowUp")
                      }
                      onClick={() => {
                        if (!selectedId) return;
                        if (
                          queuedSubmissionSnapshotRef.current?.messageId ===
                          queuedFollowUp.messageId
                        ) {
                          queuedSubmissionSnapshotRef.current = undefined;
                        }
                        void discardStagedAgentAttachments(queuedFollowUp.attachments).catch(
                          () => undefined,
                        );
                        attemptedQueuedMessageIdsRef.current.delete(queuedFollowUp.messageId);
                        setFailedQueuedMessageIds((current) => {
                          if (!current.has(queuedFollowUp.messageId)) return current;
                          const next = new Set(current);
                          next.delete(queuedFollowUp.messageId);
                          return next;
                        });
                        updateQueuedFollowUps((current) => {
                          if (!(selectedId in current)) return current;
                          const next = { ...current };
                          delete next[selectedId];
                          return next;
                        });
                      }}
                    >
                      <IconCrossSmall size={12} aria-hidden />
                    </button>
                  </span>
                </div>
              ) : null}
              {composer}
            </main>
          </div>
        )}
      </section>
      {artifactPanel ? (
        <AgentArtifactPanel
          artifacts={renderedArtifacts}
          state={artifactPanel}
          onShowList={() => setArtifactPanel({ view: "list" })}
          onOpen={openArtifact}
          onDownload={(artifact) => void downloadArtifact(artifact)}
          onClose={() => setArtifactPanel(null)}
        />
      ) : null}
      {usageOpen && selectedSession
        ? createPortal(
            <div
              className="agent-usage-overlay"
              role="presentation"
              onClick={(event) => {
                if (event.target === event.currentTarget) setUsageOpen(false);
              }}
            >
              <aside
                ref={usagePanelRef}
                className="agent-usage-panel"
                role="dialog"
                aria-modal="true"
                aria-labelledby={usageTitleId}
              >
                <div className="agent-usage-header">
                  <h2 id={usageTitleId} className="agent-usage-title">
                    {t("agent.usage.title")}
                  </h2>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label={t("agent.usage.close")}
                    onClick={() => setUsageOpen(false)}
                  >
                    <IconCrossSmall size={14} />
                  </button>
                </div>
                <div className="agent-usage-body">
                  <div className="agent-usage-row">
                    <span className="agent-usage-primary">{t("agent.usage.model")}</span>
                    <span className="agent-usage-value">{sessionDisplayModel}</span>
                  </div>
                  {projection.run?.usage?.provider ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">{t("agent.usage.provider")}</span>
                      <span className="agent-usage-value">{projection.run?.usage?.provider}</span>
                    </div>
                  ) : null}
                  {projection.run?.usage?.privacyLevel ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">{t("agent.usage.privacy")}</span>
                      <span className="agent-usage-value">
                        {projection.run?.usage?.privacyLevel}
                      </span>
                    </div>
                  ) : null}
                  {projection.run?.usage?.endpoint ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">{t("agent.usage.route")}</span>
                      <span className="agent-usage-value">{projection.run.usage.endpoint}</span>
                    </div>
                  ) : null}
                  {projection.run?.reasoningEffort ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">
                        {t("agent.usage.reasoningEffort")}
                      </span>
                      <span className="agent-usage-value">{projection.run.reasoningEffort}</span>
                    </div>
                  ) : null}
                  {projection.run?.usage ? (
                    <>
                      {projection.run.usage.inputTokens !== undefined ? (
                        <div className="agent-usage-row">
                          <span className="agent-usage-primary">{t("agent.usage.input")}</span>
                          <span className="agent-usage-value">
                            {formatNumber(projection.run.usage.inputTokens)}
                          </span>
                        </div>
                      ) : null}
                      {projection.run.usage.outputTokens !== undefined ? (
                        <div className="agent-usage-row">
                          <span className="agent-usage-primary">{t("agent.usage.output")}</span>
                          <span className="agent-usage-value">
                            {formatNumber(projection.run.usage.outputTokens)}
                          </span>
                        </div>
                      ) : null}
                      {projection.run.usage.totalTokens !== undefined ? (
                        <div className="agent-usage-row">
                          <span className="agent-usage-primary">{t("agent.usage.total")}</span>
                          <span className="agent-usage-value">
                            {formatNumber(projection.run.usage.totalTokens)}
                          </span>
                        </div>
                      ) : null}
                      {projection.run.usage.inputTokens === undefined &&
                      projection.run.usage.outputTokens === undefined &&
                      projection.run.usage.totalTokens === undefined ? (
                        <p className="agent-usage-empty">{t("agent.usage.noTokenCounts")}</p>
                      ) : null}
                      {toolUsage.size > 0 ? (
                        <div className="agent-usage-tools">
                          <p className="agent-usage-section-title">{t("agent.usage.tools")}</p>
                          {[...toolUsage.entries()].map(([name, usage]) => (
                            <div className="agent-usage-row" key={name}>
                              <span className="agent-usage-primary">{name}</span>
                              <span className="agent-usage-value">
                                {usage.failures > 0
                                  ? t("agent.usage.toolCallsWithFailures", {
                                      count: usage.calls,
                                      failures: usage.failures,
                                    })
                                  : t("agent.usage.toolCalls", { count: usage.calls })}
                              </span>
                            </div>
                          ))}
                        </div>
                      ) : null}
                    </>
                  ) : (
                    <p className="agent-usage-empty">{t("agent.usage.empty")}</p>
                  )}
                </div>
              </aside>
            </div>,
            document.querySelector(".app-shell") ?? document.body,
          )
        : null}
    </>
  );
}

function AgentComposer({
  formRef,
  scrollRef,
  draft,
  draftRevision,
  setDraft,
  draftOwnerId,
  onEditorDraftChange,
  onPendingDraftPersist,
  onDraftContentChange,
  model,
  setModel,
  safetyMode,
  setSafetyMode,
  attachments,
  onDropAttachments,
  onRemoveAttachment,
  onPickAttachments,
  onDictate,
  onSubmit,
  onStop,
  running,
  waiting,
  stopping,
  submitting,
  attaching,
  disabledReason,
  notice,
  hero = false,
  showModelPicker = true,
  engineCatalog,
  onRefreshEngineCatalog,
}: {
  formRef: RefObject<HTMLFormElement>;
  scrollRef: RefObject<HTMLDivElement>;
  draft: string;
  draftRevision: number;
  setDraft: (value: string) => void;
  draftOwnerId?: string;
  onEditorDraftChange: (
    text: string,
    draftOwnerId: string | null | undefined,
  ) => string | undefined;
  onPendingDraftPersist: (text: string, draftOwnerId: string | null | undefined) => void;
  onDraftContentChange: (hasContent: boolean) => void;
  model: string;
  setModel: (value: string) => void;
  safetyMode: AgentSafetyMode;
  setSafetyMode: (value: AgentSafetyMode) => void;
  attachments: string[];
  onDropAttachments: (files: File[]) => Promise<void>;
  onRemoveAttachment: (path: string) => void;
  onPickAttachments: () => Promise<void>;
  onDictate: () => Promise<void>;
  onSubmit: (event?: FormEvent) => Promise<void>;
  onStop: () => Promise<void>;
  running: boolean;
  waiting: boolean;
  stopping: boolean;
  submitting: boolean;
  attaching: boolean;
  disabledReason?: string;
  notice?: string;
  hero?: boolean;
  showModelPicker?: boolean;
  engineCatalog?: ChatEngineCatalog | null;
  onRefreshEngineCatalog?: () => Promise<ChatEngineCatalog | null>;
}) {
  const t = useT();
  const editorRef = useRef<ComposerEditorHandle>(null);
  const [editorDraftOwnerId, setEditorDraftOwnerId] = useState(draftOwnerId);
  useEffect(() => {
    const flushDictationDestination = (event: Event) => {
      const detail = (event as CustomEvent<{ conversationId: string }>).detail;
      if (
        detail.conversationId === editorDraftOwnerId &&
        editorRef.current?.flushPendingChange() === false
      )
        event.preventDefault();
    };
    window.addEventListener(AGENT_DICTATION_DRAFT_WILL_CHANGE, flushDictationDestination);
    return () =>
      window.removeEventListener(AGENT_DICTATION_DRAFT_WILL_CHANGE, flushDictationDestination);
  }, [editorDraftOwnerId]);

  const ownerTransitionHandledRef = useRef(false);
  const publishedDraftRef = useRef(draft);
  const appliedDraftRevisionRef = useRef(draftRevision);
  const [hasEditorContent, setHasEditorContent] = useState(Boolean(draft.trim()));
  const [safetyOpen, setSafetyOpen] = useState(false);
  const [attachOpen, setAttachOpen] = useState(false);
  const [dropActive, setDropActive] = useState(false);
  const [confirmUnrestricted, setConfirmUnrestricted] = useState(false);
  const attachTriggerRef = useRef<HTMLButtonElement>(null);
  const attachMenuRef = useRef<HTMLDivElement>(null);
  const safetyTriggerRef = useRef<HTMLButtonElement>(null);
  const safetyMenuRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (draftOwnerId === editorDraftOwnerId) return;
    const editor = editorRef.current;
    if (!editor) {
      setEditorDraftOwnerId(draftOwnerId);
      return;
    }
    // An active IME composition cannot be serialized yet. Keep the old owner
    // and document in place; compositionend publishes them, then the onChange
    // path below advances to the latest requested owner.
    ownerTransitionHandledRef.current = false;
    if (!editor.flushPendingChange() || ownerTransitionHandledRef.current) return;
    appliedDraftRevisionRef.current = draftRevision;
    publishedDraftRef.current = draft;
    editor.setContent(draft, null, { focus: false, changeKey: draftOwnerId });
    setEditorDraftOwnerId(draftOwnerId);
  }, [draft, draftOwnerId, draftRevision, editorDraftOwnerId]);

  useEffect(() => {
    if (draftOwnerId !== editorDraftOwnerId) return;
    if (appliedDraftRevisionRef.current === draftRevision && draft === publishedDraftRef.current) {
      return;
    }
    appliedDraftRevisionRef.current = draftRevision;
    if (draft === publishedDraftRef.current) return;
    publishedDraftRef.current = draft;
    editorRef.current?.setContent(draft, null, {
      focus: false,
      changeKey: editorDraftOwnerId,
    });
  }, [draft, draftOwnerId, draftRevision, editorDraftOwnerId]);

  useEffect(() => {
    if (!safetyOpen && !attachOpen) return;
    const close = (event: MouseEvent) => {
      const target = event.target as Node;
      if (safetyTriggerRef.current?.contains(target)) return;
      if (safetyMenuRef.current?.contains(target)) return;
      if (attachTriggerRef.current?.contains(target) || attachMenuRef.current?.contains(target)) {
        return;
      }
      if (target instanceof Element && target.closest(".agent-composer-model-hovercard")) return;
      setSafetyOpen(false);
      setAttachOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [attachOpen, safetyOpen]);

  useEffect(() => {
    if (!safetyOpen && !attachOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      if (safetyOpen) {
        setSafetyOpen(false);
        requestAnimationFrame(() => safetyTriggerRef.current?.focus());
        return;
      }
      setAttachOpen(false);
      requestAnimationFrame(() => attachTriggerRef.current?.focus());
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [attachOpen, safetyOpen]);

  function referenceNote() {
    const prefix = draft && !/\s$/.test(draft) ? " @" : "@";
    const next = `${draft}${prefix}`;
    publishedDraftRef.current = next;
    setDraft(next);
    editorRef.current?.setContent(next, null, { focus: true });
  }

  function handleDragOver(event: DragEvent<HTMLFormElement>) {
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDropActive(true);
  }

  function handleDrop(event: DragEvent<HTMLFormElement>) {
    event.preventDefault();
    setDropActive(false);
    void onDropAttachments(Array.from(event.dataTransfer.files));
  }

  return (
    <form
      ref={formRef}
      className="agent-composer"
      data-hero={hero ? "true" : undefined}
      data-drop-active={dropActive ? "true" : undefined}
      onSubmit={(event) => {
        event.preventDefault();
        if (draftOwnerId !== editorDraftOwnerId) return;
        if (editorRef.current?.flushPendingChange() === false) return;
        void onSubmit(event);
      }}
      onDragOver={handleDragOver}
      onDragEnter={() => setDropActive(true)}
      onDragLeave={() => setDropActive(false)}
      onDrop={handleDrop}
    >
      {hero ? null : (
        <AgentScrollToLatestButton
          scrollRef={scrollRef}
          onJump={() =>
            scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" })
          }
        />
      )}
      {notice ? (
        <div className="agent-composer-notice" role="alert">
          {notice}
        </div>
      ) : null}
      <div className="agent-composer-box" data-stacked={attachments.length ? "true" : undefined}>
        {attachments.length ? (
          <div className="agent-composer-attachments">
            {attachments.map((path) => {
              const name = path.split(/[\\/]/).pop() || path;
              return (
                <AgentAttachmentTile
                  key={path}
                  name={name}
                  onRemove={() => onRemoveAttachment(path)}
                />
              );
            })}
          </div>
        ) : null}
        <ComposerEditor
          ref={editorRef}
          placeholder={hero ? t("agent.composer.heroPlaceholder") : t("agent.composer.placeholder")}
          changeKey={editorDraftOwnerId}
          onChange={(text, _category, changedDraftOwnerId) => {
            publishedDraftRef.current = text;
            const resolvedDraftOwnerId = onEditorDraftChange(text, changedDraftOwnerId);
            if (draftOwnerId !== editorDraftOwnerId && changedDraftOwnerId === editorDraftOwnerId) {
              ownerTransitionHandledRef.current = true;
              if (resolvedDraftOwnerId !== draftOwnerId) {
                appliedDraftRevisionRef.current = draftRevision;
                publishedDraftRef.current = draft;
                editorRef.current?.setContent(draft, null, {
                  focus: false,
                  changeKey: draftOwnerId,
                });
              }
              setEditorDraftOwnerId(draftOwnerId);
            }
          }}
          onPendingChangePersist={(text, _category, changedDraftOwnerId) => {
            publishedDraftRef.current = text;
            onPendingDraftPersist(text, changedDraftOwnerId);
          }}
          onContentChange={(hasContent) => {
            setHasEditorContent(hasContent);
            onDraftContentChange(hasContent);
          }}
          onSubmit={() => {
            if (draftOwnerId !== editorDraftOwnerId) return;
            void onSubmit();
          }}
        />
        <div className="agent-composer-toolbar">
          <button
            type="button"
            ref={attachTriggerRef}
            className="agent-composer-attach"
            aria-label={t("agent.composer.addFilesOrNotes")}
            title={t("common.add")}
            aria-haspopup="menu"
            aria-expanded={attachOpen}
            data-open={attachOpen || undefined}
            onClick={() => setAttachOpen((open) => !open)}
          >
            <IconPlusMedium size={18} />
          </button>
          {hero ? (
            <button
              ref={safetyTriggerRef}
              type="button"
              className="agent-sandbox-trigger"
              data-unrestricted={safetyMode === "unrestricted" ? "true" : undefined}
              aria-haspopup="menu"
              aria-expanded={safetyOpen}
              title={t("agent.safety.triggerTitle")}
              onClick={() => setSafetyOpen((open) => !open)}
            >
              {safetyMode === "sandboxed" ? (
                <IconShieldCheck size={14} />
              ) : (
                <IconShieldCrossed size={14} />
              )}
              {safetyMode === "sandboxed"
                ? t("agent.safety.sandboxed")
                : t("agent.safety.unrestricted")}
              <IconChevronDownSmall size={12} aria-hidden />
            </button>
          ) : null}
          <div className="agent-composer-actions">
            {showModelPicker ? (
              <ChatEnginePicker
                model={model}
                setModel={setModel}
                catalog={engineCatalog}
                onRefreshCatalog={onRefreshEngineCatalog}
                anchorRef={formRef}
              />
            ) : null}
            <button
              type="button"
              className="agent-composer-mic"
              aria-label={t("agent.composer.dictate")}
              title={disabledReason ?? t("agent.composer.startDictation")}
              disabled={Boolean(disabledReason)}
              onClick={() => {
                editorRef.current?.focus();
                void onDictate();
              }}
            >
              <IconMicrophone size={18} />
            </button>
            {running ? (
              <>
                {hasEditorContent ? (
                  <button
                    type="submit"
                    className="agent-composer-send"
                    aria-label={t("agent.composer.steer")}
                    disabled={attaching}
                    title={attaching ? t("agent.composer.waitForAttachments") : undefined}
                  >
                    <IconArrowUp size={16} />
                  </button>
                ) : null}
                <button
                  type="button"
                  className="agent-composer-stop"
                  aria-label={t("agent.composer.stop")}
                  disabled={stopping}
                  onClick={() => void onStop()}
                >
                  <IconStop size={16} />
                </button>
              </>
            ) : waiting ? (
              <button
                type="button"
                className="agent-composer-stop"
                aria-label={t("agent.composer.stop")}
                disabled={stopping}
                onClick={() => void onStop()}
              >
                <IconStop size={16} />
              </button>
            ) : (
              <button
                type="submit"
                className="agent-composer-send"
                aria-label={t("agent.composer.send")}
                disabled={
                  submitting ||
                  attaching ||
                  draftOwnerId !== editorDraftOwnerId ||
                  !hasEditorContent ||
                  Boolean(disabledReason)
                }
                title={attaching ? t("agent.composer.waitForAttachments") : disabledReason}
              >
                {submitting ? <Spinner /> : <IconArrowUp size={16} />}
              </button>
            )}
          </div>
        </div>
      </div>
      {attachOpen ? (
        <div
          ref={attachMenuRef}
          className="agent-attach-menu"
          role="menu"
          aria-label={t("agent.composer.addFilesOrNotes")}
        >
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setAttachOpen(false);
              void onPickAttachments();
            }}
          >
            <span className="agent-attach-menu-icon">
              <IconFileText size={16} aria-hidden />
            </span>
            <span className="agent-attach-menu-label">{t("agent.composer.attachFiles")}</span>
          </button>
          <button
            type="button"
            role="menuitem"
            onClick={() => {
              setAttachOpen(false);
              referenceNote();
            }}
          >
            <span className="agent-attach-menu-icon">
              <IconNoteText size={16} aria-hidden />
            </span>
            <span className="agent-attach-menu-label">{t("agent.composer.referenceNote")}</span>
          </button>
        </div>
      ) : null}
      {hero && safetyOpen ? (
        <div
          ref={safetyMenuRef}
          className="agent-sandbox-menu"
          role="menu"
          aria-label={t("agent.safety.menuLabel")}
        >
          <p className="agent-sandbox-menu-title">{t("agent.safety.menuTitle")}</p>
          {SANDBOX_OPTIONS.map((option) => {
            const value: AgentSafetyMode = option.unrestricted ? "unrestricted" : "sandboxed";
            return (
              <button
                key={value}
                type="button"
                role="menuitemradio"
                aria-checked={safetyMode === value}
                onClick={() => {
                  setSafetyOpen(false);
                  if (value === "unrestricted" && !unrestrictedAcknowledged()) {
                    setConfirmUnrestricted(true);
                    return;
                  }
                  setSafetyMode(value);
                }}
              >
                {option.icon}
                <span className="agent-sandbox-option">
                  <span className="agent-sandbox-option-title">{option.title}</span>
                  <span className="agent-sandbox-option-desc">{option.description}</span>
                </span>
                {safetyMode === value ? (
                  <IconCheckmark2Small
                    size={14}
                    aria-hidden
                    className="agent-sandbox-option-check"
                  />
                ) : null}
              </button>
            );
          })}
        </div>
      ) : null}
      <Dialog
        open={confirmUnrestricted}
        onClose={() => setConfirmUnrestricted(false)}
        title={t("agent.safety.confirmTitle")}
        description={t("agent.safety.confirmDescription")}
        footer={
          <>
            <button
              type="button"
              className="primary-action"
              onClick={() => setConfirmUnrestricted(false)}
            >
              {t("common.cancel")}
            </button>
            <button
              type="button"
              className="primary-action primary-solid"
              onClick={() => {
                rememberUnrestrictedAcknowledged();
                setSafetyMode("unrestricted");
                setConfirmUnrestricted(false);
              }}
            >
              {t("agent.safety.confirmAction")}
            </button>
          </>
        }
      >
        {null}
      </Dialog>
    </form>
  );
}

function AgentScrollToLatestButton({
  scrollRef,
  onJump,
}: {
  scrollRef: RefObject<HTMLDivElement>;
  onJump: () => void;
}) {
  const t = useT();
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const scroller = scrollRef.current;
    if (!scroller) return;
    const recheck = () => {
      const distanceFromBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
      setVisible(scroller.scrollHeight > scroller.clientHeight && distanceFromBottom > 48);
    };
    recheck();
    scroller.addEventListener("scroll", recheck, { passive: true });
    const observer = typeof ResizeObserver === "function" ? new ResizeObserver(recheck) : undefined;
    observer?.observe(scroller);
    for (const child of Array.from(scroller.children)) observer?.observe(child);
    return () => {
      scroller.removeEventListener("scroll", recheck);
      observer?.disconnect();
    };
  }, [scrollRef]);

  return (
    <button
      type="button"
      className="agent-scroll-to-latest"
      data-visible={visible ? "true" : undefined}
      aria-label={t("agent.composer.scrollToLatest")}
      aria-hidden={visible ? undefined : true}
      tabIndex={visible ? undefined : -1}
      onClick={onJump}
    >
      <IconArrowDown size={16} ariaHidden />
    </button>
  );
}
