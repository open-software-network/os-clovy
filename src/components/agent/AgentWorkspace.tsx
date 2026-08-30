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
  companionCompleteFrontendRequest,
  companionConsumeAttachments,
  companionPublishAgentEvent,
  type CompanionAgentStatus,
  type CompanionFrontendRequest,
  assignSessionToProfile,
  discardStagedAgentAttachments,
  downloadAgentArtifact,
  dictationHelperCommand,
  clovyHomeChat,
  type ClovyHomeChatResponse,
  listVeniceModels,
  providerModelSettings,
  pruneStagedAgentAttachments,
  setCostQuality as setProviderCostQuality,
  type VeniceModelDto,
} from "../../lib/tauri";
import { shouldBlockTextOnFunding } from "../../lib/account-gate";
import { dispatchAgentSessionStatus, dispatchAgentSessionsChanged } from "../../lib/agent-events";
import { messageFromError } from "../../lib/errors";
import { useExperimentalFlags } from "../../lib/experimental-flags";
import {
  COMPANION_FRONTEND_QUEUE_EVENT,
  registerCompanionFrontendConsumer,
  takeCompanionFrontendRequests,
} from "../../lib/companion-frontend-router";
import { persistAgentDefaultModel } from "../../lib/agent-default-model";
import { agentModelSelection, agentRunModelId } from "../../lib/agent-model-selection";
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
  clearAgentSessionDraftRevision,
  clearPendingAgentSessionDraft,
  invalidateAgentSessionDraft,
  readAgentSessionDraft,
  readAgentSessionDraftRevision,
  transferPendingAgentSessionDraft,
  writePendingAgentSessionDraft,
  writeAgentSessionDraft,
} from "../../lib/agent-session-drafts";
import {
  forgetSessionThinkingLevel,
  loadSessionThinkingLevels,
  loadThinkingLevel,
  rememberSessionThinkingLevel,
  saveThinkingLevel,
  thinkingEffortForLevel,
  type ThinkingLevel,
} from "../../lib/thinking-level";
import {
  prepareProjectPrompt,
  ProjectContextSignatureStore,
  stripProjectContext,
} from "../../lib/agent-project-context";
import { AgentChatTurnRow } from "./chat-turns/AgentChatTurnRow";
import {
  AgentArtifactPanel,
  type AgentArtifact,
  type AgentArtifactPanelState,
} from "./chat-turns/AgentArtifactPanel";
import { AgentSessionBar } from "./chat-turns/AgentSessionBar";
import { AgentThinking } from "./AgentThinking";
import {
  advanceHeroGreeting,
  AGENT_DELETE_SESSION_EVENT,
  AGENT_NEW_SESSION_EVENT,
  AGENT_SHORTCUTS,
  rememberUnrestrictedAcknowledged,
  SANDBOX_OPTIONS,
  unrestrictedAcknowledged,
} from "./agent-workspace-config";
import { ComposerEditor, type ComposerEditorHandle } from "./composer/ComposerEditor";
import { AgentAttachmentTile } from "./composer/AgentAttachmentTile";
import { agentComposerClearance } from "./composer/layout";
import {
  ComposerModelPicker,
  heroPrivacyFootnote,
  useComposerModelPopoverPosition,
} from "./composer/ModelPicker";
import { modelPrivacyBadge } from "../../lib/model-privacy";
import { autoPillDesignation } from "../../lib/suggested-models";
import { getCurrentDataPartitionName } from "../../lib/data-partition";
import { companionSessionInActivePartition } from "../../lib/companion-partition";
import { createCompanionPublicationQueue } from "../../lib/companion-publication-queue";
import { AUTO_MODEL_ID, modelOptions, selectedModel } from "../settings/ModelPickerDialog";
import { ModelPickerPopover, type ModelPickerFlyout } from "../settings/ModelPickerPopover";
import { Dialog } from "../ui/Dialog";
import { Spinner } from "../ui/Spinner";
import { ClovyAlive } from "../brand/ClovyAlive";
import { ShareDialog } from "../share/ShareDialog";
import { ReportDialog } from "./ReportDialog";
import type { ReportCategory } from "./composer/reportCategory";
import { buildSessionPayload } from "../../lib/share-payload";
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
  resolveClovyHomeThreadSessionId,
  stripClovyHomeContextFromPreview,
  withClovyHomeCurrentResearch,
  withClovyHomeLatestTaskIntent,
  type ClovyHomeConversationContext,
  type ClovyHomeTaskRequest,
} from "../../lib/clovy-home";
import type { AgentChatTurn } from "../../lib/agent-chat-runtime";
import {
  clearHomeTaskHandoffActive,
  compareHomeTurnOrder,
  enqueueHomeDirectChat,
  existingHomeTaskHandoffForSourceTurn,
  homeConversationGreetingReply,
  homeConversationContextFromTurns,
  isHomeTaskReplayWithoutNewIntent,
  isHomeTaskHandoffAcknowledgement,
  homeDemoReply,
  insertHomeDirectReply,
  markHomeTaskHandoffActive,
  persistHomeDirectTurns,
  persistHomeTaskHandoffs,
  readAllHomeTaskHandoffAttachments,
  readHomeDirectTurns,
  readHomeTaskHandoffs,
  recoverInterruptedHomeTaskHandoffs,
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
const DEFAULT_MODEL = AUTO_MODEL_ID;
const AGENT_SUGGESTED_MODEL_IDS = [AUTO_MODEL_ID] as const;
const AGENT_AUTO_MODEL: VeniceModelDto = {
  provider: "",
  id: AUTO_MODEL_ID,
  name: "Auto",
  description: "Chooses the best available model for each request.",
  modelType: "text",
  privacy: "private",
  traits: [],
  capabilities: [],
};
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

export function canShareAgentSession(input: {
  selectedSessionId?: string;
  newSessionMode: boolean;
  provisional: boolean;
  historyLoaded: boolean;
  working: boolean;
}): boolean {
  return Boolean(
    input.selectedSessionId &&
      !input.newSessionMode &&
      !input.provisional &&
      input.historyLoaded &&
      !input.working,
  );
}

function titleFromPrompt(prompt: string) {
  const normalized = prompt.replace(/\s+/g, " ").trim();
  return normalized.length > 52 ? `${normalized.slice(0, 51).trimEnd()}…` : normalized;
}

function queuedAttachmentStatus(attachments: readonly string[]) {
  const count = attachments.length;
  return `${count} attachment${count === 1 ? "" : "s"} queued for next turn`;
}

function queuedFollowUpStatus(queued: QueuedAgentFollowUp, failed: boolean) {
  const withAttachmentStatus = (status: string) =>
    queued.attachments.length ? `${status}. ${queuedAttachmentStatus(queued.attachments)}` : status;
  if (failed) {
    return withAttachmentStatus(
      queued.delivery === "attachments"
        ? "Couldn't send queued attachments"
        : "Couldn't send queued follow-up",
    );
  }
  if (queued.delivery === "attachments") {
    return queuedAttachmentStatus(queued.attachments);
  }
  if (queued.steering) {
    const steeringStatus =
      queued.steering === "accepted" ? "Steering active run" : "Sending to active run";
    return withAttachmentStatus(steeringStatus);
  }
  return withAttachmentStatus("Queued follow-up");
}

function queuedFollowUpText(queued: QueuedAgentFollowUp) {
  if (queued.delivery !== "attachments") return queued.prompt;
  return queued.attachments.map((path) => path.split(/[\\/]/).pop() || path).join(", ");
}

function withQueuedSteering(
  queued: QueuedAgentFollowUps,
  sessionId: string,
  messageId: string,
  steering: QueuedAgentFollowUp["steering"],
) {
  const current = queued[sessionId];
  if (
    !current ||
    current.messageId !== messageId ||
    current.delivery === "attachments" ||
    current.steering === steering
  ) {
    return queued;
  }
  const nextFollowUp = { ...current };
  if (steering) nextFollowUp.steering = steering;
  else delete nextFollowUp.steering;
  return { ...queued, [sessionId]: nextFollowUp };
}

function artifactView(artifact: AgentArtifactDto): AgentArtifact {
  return {
    name: artifact.name,
    path: artifact.path,
    rootLabel: "Clovy workspace",
    size: artifact.sizeBytes,
  };
}

export function AgentWorkspace({
  initialSession,
  initialSessionId,
  homeMode = false,
  homeUserDisplayName,
  onHomeSessionCreated,
  onOpenHomeTaskSession,
  origin,
  onSessionSelected,
  onMoveSessionToProject,
  sessionInProject = false,
  projectContext,
  resolveSessionProjectContext,
  creditActionsDisabledReason,
}: AgentWorkspaceProps = {}) {
  const { companionPairingEnabled } = useExperimentalFlags();
  const initialAgentSession = initialSession;
  const pendingRequestRef = useRef(pendingNewSessionRequest());
  const [reportDialogCategory, setReportDialogCategory] = useState<ReportCategory | undefined>(
    pendingRequestRef.current?.category,
  );
  const [reportStoredSessionId, setReportStoredSessionId] = useState<string | undefined>(
    pendingRequestRef.current?.category ? (initialSession?.id ?? initialSessionId) : undefined,
  );
  const [sessions, setSessions] = useState<AgentSessionDto[]>(
    initialAgentSession ? [initialAgentSession] : [],
  );
  const [selectedId, setSelectedId] = useState(initialSession?.id ?? initialSessionId);
  const [newSessionMode, setNewSessionMode] = useState(!initialSession && !initialSessionId);
  const [projection, setProjection] = useState<AgentRuntimeProjection>(() =>
    createAgentRuntimeProjection({ session: initialAgentSession }),
  );
  const [hydratedSelectionId, setHydratedSelectionId] = useState<string>();
  const [artifacts, setArtifacts] = useState<AgentArtifactDto[]>([]);
  const [artifactPanel, setArtifactPanel] = useState<AgentArtifactPanelState | null>(null);
  const [shareOpen, setShareOpen] = useState(false);
  const [shareUrl, setShareUrl] = useState<string>();
  const [usageOpen, setUsageOpen] = useState(false);
  const usagePanelRef = useRef<HTMLElement>(null);
  const usageReturnFocusRef = useRef<HTMLElement | null>(null);
  const usageTitleId = useId();
  const [compactOpen, setCompactOpen] = useState(false);
  const [compacting, setCompacting] = useState(false);
  const [compactResult, setCompactResult] = useState<string>();
  const [models, setModels] = useState<VeniceModelDto[]>([]);
  const [veniceApiKeyConfigured, setVeniceApiKeyConfigured] = useState(false);

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
        event.stopPropagation();
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
    window.addEventListener("keydown", onKey, true);
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey, true);
      document.body.style.overflow = previousOverflow;
    };
  }, [usageOpen]);

  const focusedHomeModelRef = useRef(DEFAULT_MODEL);
  const focusedHomeThinkingLevelRef = useRef(loadThinkingLevel());
  const initialModelSelection = agentModelSelection(initialAgentSession?.model || DEFAULT_MODEL);
  const [model, setModel] = useState(homeMode ? AUTO_MODEL_ID : initialModelSelection.modelId);
  const modelRef = useRef(model);
  modelRef.current = model;
  const [costQuality, setCostQuality] = useState(initialModelSelection.costQuality ?? 100);
  const costQualityRef = useRef(costQuality);
  costQualityRef.current = costQuality;
  const confirmedCostQualityRef = useRef(costQuality);
  const costQualitySaveChainRef = useRef<Promise<void>>(Promise.resolve());
  const latestCostQualitySaveRef = useRef(0);
  const costQualityIntentRevisionRef = useRef(0);
  const sessionCostQualityExplicitRef = useRef(initialModelSelection.costQuality !== undefined);
  const [thinkingLevel, setThinkingLevel] = useState<ThinkingLevel>(() => {
    if (homeMode) return "instant";
    const sessionLevel = initialAgentSession?.id
      ? loadSessionThinkingLevels()[initialAgentSession.id]
      : undefined;
    return sessionLevel ?? loadThinkingLevel();
  });
  const thinkingLevelRef = useRef(thinkingLevel);
  thinkingLevelRef.current = thinkingLevel;
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
    thinkingLevel: ThinkingLevel;
  }>();
  const [recoverableSubmission, setRecoverableSubmission] = useState<{
    id: string;
    prompt: string;
    attachments: string[];
    model: string;
    thinkingLevel: ThinkingLevel;
  }>();
  const recoverableSubmissionRef = useRef(recoverableSubmission);
  recoverableSubmissionRef.current = recoverableSubmission;
  const recoverableSubmissionSnapshotRef = useRef<typeof recoverableSubmission>();
  const [recoverableHomeSubmission, setRecoverableHomeSubmissionState] = useState<{
    id: string;
    prompt: string;
    attachments: string[];
  }>();
  const recoverableHomeSubmissionRef = useRef(recoverableHomeSubmission);
  recoverableHomeSubmissionRef.current = recoverableHomeSubmission;
  const setRecoverableHomeSubmission = useCallback((value: typeof recoverableHomeSubmission) => {
    recoverableHomeSubmissionRef.current = value;
    setRecoverableHomeSubmissionState(value);
  }, []);
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
  const [heroGreeting] = useState(advanceHeroGreeting);
  const scrollRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<HTMLFormElement>(null);
  const [composerClearance, setComposerClearance] = useState(0);
  const [submittedScrollRevision, setSubmittedScrollRevision] = useState(0);
  const selectedIdRef = useRef(selectedId);
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
  const [homeDirectTurns, setHomeDirectTurns] = useState<AgentChatTurn[]>(() =>
    homeMode ? readHomeDirectTurns(initialSessionId) : [],
  );
  const homeDirectTurnsRef = useRef(homeDirectTurns);
  const [homeTaskHandoffs, setHomeTaskHandoffs] = useState<HomeTaskHandoff[]>(() =>
    homeMode ? readHomeTaskHandoffs(initialSessionId) : [],
  );
  const [homeStreamingReply, setHomeStreamingReply] = useState<AgentChatTurn | null>(null);
  const [homeDirectPendingCount, setHomeDirectPendingCount] = useState(0);
  const homeSessionPromiseRef = useRef<Promise<string> | null>(null);
  const homeSessionCreationDraftOwnerRef = useRef<string>();
  const [homeSessionCreating, setHomeSessionCreating] = useState(false);
  const handledHomeTaskToolCallsRef = useRef(new Set<string>());

  useEffect(() => {
    homeDirectTurnsRef.current = homeDirectTurns;
  }, [homeDirectTurns]);

  const startNewSession = useCallback(
    (request?: AgentNewSessionDetail) => {
      if (request?.category) {
        setReportStoredSessionId(selectedIdRef.current);
        setReportDialogCategory(request.category);
      }
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
      setShareOpen(false);
      setShareUrl(undefined);
      setThinkingLevel(loadThinkingLevel());
      setComposerDraft(request?.prompt ?? "");
      setPendingInitialTurn(undefined);
      setSubmitting(false);
      setError(undefined);
      onSessionSelected?.(undefined);
    },
    [abandonComposerAttachments, onSessionSelected, setComposerDraft],
  );

  const applyCostQuality = useCallback((value: number) => {
    const normalized = Math.max(0, Math.min(100, Math.round(value)));
    costQualityIntentRevisionRef.current += 1;
    costQualityRef.current = normalized;
    setCostQuality(normalized);
    const sessionId = selectedIdRef.current;
    if (sessionId) {
      sessionCostQualityExplicitRef.current = true;
      rememberSessionModel(sessionId, agentRunModelId(modelRef.current, normalized));
      return;
    }

    sessionCostQualityExplicitRef.current = false;
    const sessionIdAtSave = selectedIdRef.current;
    const version = ++latestCostQualitySaveRef.current;
    const save = costQualitySaveChainRef.current.then(() => setProviderCostQuality(normalized));
    costQualitySaveChainRef.current = save.then(
      () => undefined,
      () => undefined,
    );
    void save.then(
      (next) => {
        confirmedCostQualityRef.current = next.costQuality;
        if (
          version !== latestCostQualitySaveRef.current ||
          selectedIdRef.current !== sessionIdAtSave
        ) {
          return;
        }
        costQualityRef.current = next.costQuality;
        setCostQuality(next.costQuality);
        setError(undefined);
      },
      (cause) => {
        if (version !== latestCostQualitySaveRef.current) return;
        costQualityRef.current = confirmedCostQualityRef.current;
        setCostQuality(confirmedCostQualityRef.current);
        setError(messageFromError(cause));
      },
    );
  }, []);

  const selectedSession =
    sessions.find((session) => session.id === selectedId) ?? projection.session;
  const heroMode = !homeMode && newSessionMode && !selectedSession && !pendingInitialTurn;
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
  const activeModel = selectedModel(models, model);
  const textActionsDisabledReason = shouldBlockTextOnFunding(Boolean(creditActionsDisabledReason), {
    activeModelId: model || undefined,
    activeModel,
    veniceApiKeyConfigured,
  })
    ? creditActionsDisabledReason
    : undefined;

  useEffect(() => {
    if (!homeMode || !selectedId) return;
    const restoredTurns = readHomeDirectTurns(selectedId);
    const restoredHandoffs = recoverInterruptedHomeTaskHandoffs(selectedId);
    homeDirectTurnsRef.current = restoredTurns;
    setHomeDirectTurns(restoredTurns);
    setHomeTaskHandoffs(restoredHandoffs);
    onHomeSessionCreated?.(selectedId);
  }, [homeMode, onHomeSessionCreated, selectedId]);

  useEffect(() => {
    if (!homeMode) return;
    const refreshHomeThread = (event: Event) => {
      const storedSessionId = (event as CustomEvent<{ storedSessionId?: string }>).detail
        ?.storedSessionId;
      if (!storedSessionId || storedSessionId !== selectedIdRef.current) return;
      const restoredTurns = readHomeDirectTurns(storedSessionId);
      homeDirectTurnsRef.current = restoredTurns;
      setHomeDirectTurns(restoredTurns);
      setHomeTaskHandoffs(readHomeTaskHandoffs(storedSessionId));
    };
    window.addEventListener(CLOVY_HOME_THREAD_CHANGED_EVENT, refreshHomeThread);
    return () => window.removeEventListener(CLOVY_HOME_THREAD_CHANGED_EVENT, refreshHomeThread);
  }, [homeMode]);

  const publishSessions = useCallback((next: AgentSessionDto[]) => {
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
    const next = await agentRuntimeBindings.listSessions();
    publishSessions(next);
    return next;
  }, [publishSessions]);

  const applySessionModel = useCallback((storedModel: string) => {
    const selection = agentModelSelection(storedModel);
    setModel(selection.modelId || DEFAULT_MODEL);
    sessionCostQualityExplicitRef.current = selection.costQuality !== undefined;
    if (selection.costQuality !== undefined) {
      costQualityRef.current = selection.costQuality;
      setCostQuality(selection.costQuality);
    } else {
      costQualityRef.current = confirmedCostQualityRef.current;
      setCostQuality(confirmedCostQualityRef.current);
    }
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
      const requestId = crypto.randomUUID();
      hydrationRequestRef.current = requestId;
      if (selectedIdRef.current === sessionId) setHydratedSelectionId(undefined);
      const [session, items, files, latestRun] = await Promise.all([
        agentRuntimeBindings.getSession(sessionId),
        agentRuntimeBindings.listItems(sessionId),
        agentRuntimeBindings.listArtifacts(sessionId),
        agentRuntimeBindings.getLatestRun?.(sessionId) ?? Promise.resolve(null),
      ]);
      if (selectedIdRef.current !== sessionId || hydrationRequestRef.current !== requestId) {
        return;
      }
      setProjection((current) =>
        mergeAgentRuntimeSnapshot(current, {
          session,
          items,
          run: latestRun ?? undefined,
        }),
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
        setModel(AUTO_MODEL_ID);
      } else {
        applySessionModel(loadSessionModels()[session.id] ?? session.model);
      }
      setThinkingLevel(
        homeMode ? "instant" : (loadSessionThinkingLevels()[session.id] ?? loadThinkingLevel()),
      );
      setSafetyMode(homeMode ? "sandboxed" : session.safetyMode);
      setNewSessionMode(false);
      if (!homeMode) writeLastOpenSessionId(sessionId);
      onSessionSelected?.(session);
    },
    [applySessionModel, homeMode, onSessionSelected, updateQueuedFollowUps],
  );

  useEffect(() => {
    void refreshSessions()
      .then((next) => {
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
    void listVeniceModels("generation")
      .then((response) => {
        setModels(response.models);
        if (homeMode) {
          focusedHomeModelRef.current = response.selectedModel || DEFAULT_MODEL;
        }
        if (!homeMode && !initialAgentSession && !initialSessionId && response.selectedModel) {
          setModel(response.selectedModel);
        }
      })
      .catch(() => undefined);
    const costQualityRequestRevision = costQualityIntentRevisionRef.current;
    void providerModelSettings()
      .then((response) => {
        setVeniceApiKeyConfigured(response.effectiveSettings.veniceApiKeyConfigured);
        if (costQualityRequestRevision !== costQualityIntentRevisionRef.current) return;
        confirmedCostQualityRef.current = response.settings.costQuality;
        if (!sessionCostQualityExplicitRef.current) {
          costQualityRef.current = response.settings.costQuality;
          setCostQuality(response.settings.costQuality);
        }
      })
      .catch(() => setVeniceApiKeyConfigured(false));
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
    if (!nextId) return;
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
      applySessionModel(
        loadSessionModels()[initialSession.id] ?? (initialSession.model || DEFAULT_MODEL),
      );
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

  useEffect(() => {
    const handleDeletedSession = (event: Event) => {
      const storedSessionId = (event as CustomEvent<{ sessionId?: string }>).detail?.sessionId;
      if (!storedSessionId) return;
      invalidateAgentSessionDraft(storedSessionId);
    };
    window.addEventListener(AGENT_DELETE_SESSION_EVENT, handleDeletedSession);
    return () => window.removeEventListener(AGENT_DELETE_SESSION_EVENT, handleDeletedSession);
  }, []);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    const companionPublicationQueue = createCompanionPublicationQueue({
      isSessionPublishable: async (storedSessionId) =>
        Boolean(await companionSessionInActivePartition(storedSessionId)) && !disposed,
      onFailure: ({ error, eventType, storedSessionId }) => {
        // biome-ignore lint/suspicious/noConsole: stream publication failures need a content-free diagnostic
        console.warn("Failed to publish companion agent event", {
          eventType,
          reason: messageFromError(error),
          storedSessionId,
        });
      },
    });
    const enqueueCompanionPublication = (
      storedSessionId: string,
      eventType: "delta" | "status",
      publish: () => Promise<void>,
    ) => {
      void companionPublicationQueue.enqueue({ eventType, publish, storedSessionId });
    };
    const publishCompanionStatus = (
      storedSessionId: string,
      status: CompanionAgentStatus,
      runId?: string,
    ) => {
      enqueueCompanionPublication(storedSessionId, "status", () =>
        companionPublishAgentEvent({
          type: "status",
          data: {
            storedSessionId,
            status,
            ...(runId ? { runId } : {}),
          },
        }),
      );
    };
    const publishCompanionDelta = (storedSessionId: string, text: string) => {
      enqueueCompanionPublication(storedSessionId, "delta", () =>
        companionPublishAgentEvent({
          type: "delta",
          data: { storedSessionId, text },
        }),
      );
    };
    void listen<AgentRuntimeEvent>(AGENT_RUNTIME_EVENT, ({ payload }) => {
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
      if (companionPairingEnabled && payload.method === "message.delta" && payload.data.delta) {
        publishCompanionDelta(payload.sessionId, payload.data.delta);
      }
      if (companionPairingEnabled && payload.method === "tool.completed") {
        publishCompanionStatus(payload.sessionId, "running", payload.runId);
      }
      const companionStatus: CompanionAgentStatus | undefined =
        payload.method === "interruption.requested"
          ? "waitingForUser"
          : payload.method === "run.completed"
            ? "completed"
            : payload.method === "run.cancelled"
              ? "cancelled"
              : payload.method === "run.failed"
                ? "failed"
                : payload.method === "run.started"
                  ? "running"
                  : undefined;
      if (companionPairingEnabled && companionStatus) {
        publishCompanionStatus(
          payload.sessionId,
          companionStatus,
          terminal ? payload.runId : undefined,
        );
      }
      if (payload.sessionId !== selectedIdRef.current) {
        void refreshSessions().catch(() => undefined);
        return;
      }
      setProjection((current) => applyAgentRuntimeEvent(current, payload));
      dispatchAgentSessionStatus({
        sessionId: payload.sessionId,
        status:
          payload.method === "interruption.requested"
            ? "waitingForUser"
            : payload.method === "run.completed"
              ? "completed"
              : payload.method === "run.cancelled"
                ? "cancelled"
                : payload.method === "run.failed"
                  ? "failed"
                  : "running",
      });
      if (terminal) {
        setSubmitting(false);
        void Promise.all([hydrate(payload.sessionId), refreshSessions()]);
      }
    }).then((dispose) => {
      if (disposed) dispose();
      else unlisten = dispose;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [companionPairingEnabled, hydrate, refreshSessions, updateQueuedFollowUps]);

  useEffect(() => {
    if (!companionPairingEnabled) return;
    async function rejectUnavailableCompanionSession(operationId: string) {
      await companionCompleteFrontendRequest(operationId, {
        type: "error",
        data: {
          code: "not_found",
          message: "That agent session is no longer available.",
          retryable: false,
        },
      });
    }

    async function handleCompanionRequest(payload: CompanionFrontendRequest) {
      try {
        switch (payload.intent.type) {
          case "agentSessionsList":
          case "agentMessagesList":
          case "modelsList":
          case "sessionModelGet":
          case "sessionModelSet":
            return;
          case "agentSend": {
            const {
              storedSessionId: requestedStoredSessionId,
              message,
              attachments = [],
              attachmentReferenceIds = [],
            } = payload.intent.data;
            let session: AgentSessionDto | undefined;
            let createdSessionPartition: string | undefined;
            if (requestedStoredSessionId) {
              session = await companionSessionInActivePartition(requestedStoredSessionId).catch(
                () => undefined,
              );
              if (!session) {
                await rejectUnavailableCompanionSession(payload.operationId);
                return;
              }
            } else {
              createdSessionPartition = getCurrentDataPartitionName();
              session = await agentRuntimeBindings.createSession({
                title: titleFromPrompt(message),
                model: DEFAULT_MODEL,
                safetyMode: "sandboxed",
                profile: createdSessionPartition,
              });
              void refreshSessions().catch(() => undefined);
            }
            const enabledSkillIds = (await agentRuntimeBindings.listSkills())
              .filter((skill) => skill.enabled)
              .map((skill) => skill.id);
            const authorizedSession = requestedStoredSessionId
              ? await companionSessionInActivePartition(session.id).catch(() => undefined)
              : createdSessionPartition === getCurrentDataPartitionName()
                ? session
                : undefined;
            if (!authorizedSession) {
              await rejectUnavailableCompanionSession(payload.operationId);
              return;
            }
            const preparedPrompt = preparePromptForSession(authorizedSession.id, message);
            const stagedModel = loadSessionModels()[authorizedSession.id];
            const submittedModel = stagedModel ?? authorizedSession.model;
            await agentRuntimeBindings.startRun({
              sessionId: authorizedSession.id,
              prompt: preparedPrompt.text,
              model: submittedModel,
              reasoningEffort: thinkingEffortForLevel(thinkingLevelRef.current) as
                | "minimal"
                | "medium"
                | "high",
              safetyMode: authorizedSession.safetyMode,
              workspacePath: authorizedSession.workspacePath,
              enabledSkillIds,
              attachments: attachments.map((attachment) => attachment.path),
              attachmentMetadata: attachments.map(({ name, mediaType }) => ({ name, mediaType })),
            });
            clearSessionModelIfApplied(authorizedSession.id, submittedModel);
            await companionConsumeAttachments(attachmentReferenceIds).catch(() => undefined);
            projectContextSignaturesBySessionId.set(
              authorizedSession.id,
              preparedPrompt.contextSignature,
            );
            await companionCompleteFrontendRequest(payload.operationId, {
              type: "agentAccepted",
              data: { storedSessionId: authorizedSession.id },
            });
            return;
          }
          case "agentCancel": {
            const { storedSessionId } = payload.intent.data;
            if (
              !(await companionSessionInActivePartition(storedSessionId).catch(() => undefined))
            ) {
              await rejectUnavailableCompanionSession(payload.operationId);
              return;
            }
            const run = await agentRuntimeBindings.getLatestRun?.(storedSessionId);
            if (
              run &&
              (run.status === "queued" ||
                run.status === "running" ||
                run.status === "waiting_for_user")
            ) {
              if (
                !(await companionSessionInActivePartition(storedSessionId).catch(() => undefined))
              ) {
                await rejectUnavailableCompanionSession(payload.operationId);
                return;
              }
              await agentRuntimeBindings.cancelRun(run.id);
            }
            await companionCompleteFrontendRequest(payload.operationId, { type: "accepted" });
            return;
          }
        }
      } catch (error) {
        await companionCompleteFrontendRequest(payload.operationId, {
          type: "error",
          data: {
            code: "internal",
            message: messageFromError(error),
            retryable: true,
          },
        }).catch(() => undefined);
      }
    }

    function consumeQueuedRequests() {
      for (const request of takeCompanionFrontendRequests()) {
        void handleCompanionRequest(request);
      }
    }

    const unregisterConsumer = registerCompanionFrontendConsumer();
    window.addEventListener(COMPANION_FRONTEND_QUEUE_EVENT, consumeQueuedRequests);
    consumeQueuedRequests();
    return () => {
      unregisterConsumer();
      window.removeEventListener(COMPANION_FRONTEND_QUEUE_EVENT, consumeQueuedRequests);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [companionPairingEnabled, preparePromptForSession, refreshSessions]);

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
      textActionsDisabledReason ||
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
      thinkingLevel: queued.thinkingLevel,
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
    textActionsDisabledReason,
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
      setError("Wait for files to finish attaching, then send again.");
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
    if (!prompt || waiting || submitting || textActionsDisabledReason) {
      clearQueuedSubmissionAttempt();
      return;
    }
    const submittedStoredSessionId = selectedIdRef.current;
    const submittedDraftRevision =
      !queuedSubmission &&
      !recoveredSubmission &&
      submittedStoredSessionId &&
      readAgentSessionDraft(submittedStoredSessionId)?.trim() === prompt
        ? readAgentSessionDraftRevision(submittedStoredSessionId)
        : undefined;
    if (running) {
      const submittedAttachments = queuedSubmission?.attachments ?? attachmentsRef.current;
      const submittedModel = queuedSubmission?.model ?? agentRunModelId(model, costQuality);
      const submittedThinkingLevel = queuedSubmission?.thinkingLevel ?? thinkingLevel;
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
          thinkingLevel: submittedThinkingLevel,
          delivery: "follow_up",
          steering: "pending",
        }),
      }));
      setComposerDraft("", { persist: false });
      setComposerAttachments([]);
      clearAgentSessionDraftRevision(ownerStoredSessionId, submittedDraftRevision);
      requestSubmittedMessageScroll();
      if (projection.run) {
        const activeRunId = projection.run.id;
        void agentRuntimeBindings
          .steerRun(activeRunId, messageId, prompt)
          .then((result) => {
            if (result.accepted) {
              updateQueuedFollowUps((current) =>
                withQueuedSteering(current, ownerStoredSessionId, messageId, "accepted"),
              );
              return;
            }
            // biome-ignore lint/suspicious/noConsole: steering fallback needs a non-sensitive diagnostic
            console.warn("Live steering was rejected; queued the full follow-up instead.", {
              reason: result.reason ?? "unknown",
              runId: activeRunId,
              messageId,
            });
            updateQueuedFollowUps((current) =>
              withQueuedSteering(current, ownerStoredSessionId, messageId, undefined),
            );
          })
          .catch((cause: unknown) => {
            // biome-ignore lint/suspicious/noConsole: steering fallback needs a non-sensitive diagnostic
            console.warn("Live steering failed; queued the full follow-up instead.", {
              reason: messageFromError(cause),
              runId: activeRunId,
              messageId,
            });
            updateQueuedFollowUps((current) =>
              withQueuedSteering(current, ownerStoredSessionId, messageId, undefined),
            );
          });
      }
      return;
    }
    queuedSubmissionSnapshotRef.current = undefined;
    recoverableSubmissionSnapshotRef.current = undefined;
    const queuedSnapshot = queuedSubmission;
    const recoveredSnapshot = recoveredSubmission;
    const submittedModel =
      queuedSnapshot?.model ?? recoveredSnapshot?.model ?? agentRunModelId(model, costQuality);
    const submittedThinkingLevel =
      queuedSnapshot?.thinkingLevel ?? recoveredSnapshot?.thinkingLevel ?? thinkingLevel;
    const submissionId = crypto.randomUUID();
    submissionOwnerRef.current = submissionId;
    setSubmitting(true);
    setError(undefined);
    const creatingSession = !selectedSession || newSessionMode;
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
      if (!session || newSessionMode) {
        const createdSession = await agentRuntimeBindings.createSession({
          title: titleFromPrompt(prompt),
          model: submittedModel,
          safetyMode,
          profile: getCurrentDataPartitionName(),
        });
        session = createdSession;
        const latestModel = agentRunModelId(modelRef.current, costQualityRef.current);
        if (latestModel !== submittedModel) {
          rememberSessionModel(createdSession.id, latestModel);
        }
        const latestThinkingLevel = thinkingLevelRef.current;
        if (latestThinkingLevel !== submittedThinkingLevel) {
          rememberSessionThinkingLevel(createdSession.id, latestThinkingLevel);
        }
        setSessions((current) => [
          createdSession,
          ...current.filter((item) => item.id !== createdSession.id),
        ]);
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
      const run = await agentRuntimeBindings.startRun({
        sessionId: activeSession.id,
        prompt: preparedPrompt.text,
        model: submittedModel,
        reasoningEffort: thinkingEffortForLevel(submittedThinkingLevel) as
          | "minimal"
          | "medium"
          | "high",
        safetyMode,
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
      const storedThinkingLevel = loadSessionThinkingLevels()[activeSession.id];
      if (!storedThinkingLevel || storedThinkingLevel === submittedThinkingLevel) {
        rememberSessionThinkingLevel(activeSession.id, submittedThinkingLevel);
      }
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
      dispatchAgentSessionStatus({
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
      const operationIsVisible = creationRequestId
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
        if (!selectedIdRef.current) {
          setPendingInitialTurn(undefined);
          setNewSessionMode(true);
        }
      }
      if (!queuedSnapshot) {
        const failedSubmission = {
          id: recoveredSnapshot?.id ?? crypto.randomUUID(),
          prompt,
          attachments: attachedPaths,
          model: submittedModel,
          thinkingLevel: submittedThinkingLevel,
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

  async function ensureHomeSession(): Promise<string> {
    const selected = selectedIdRef.current;
    if (selected) return selected;
    if (homeSessionPromiseRef.current) return homeSessionPromiseRef.current;

    const draftOwnerId = crypto.randomUUID();
    homeSessionCreationDraftOwnerRef.current = draftOwnerId;
    creationDraftDestinationsRef.current.set(draftOwnerId, undefined);
    const creation = agentRuntimeBindings
      .createSession({
        title: "Home",
        model: AUTO_MODEL_ID,
        safetyMode: "sandboxed",
        profile: getCurrentDataPartitionName(),
      })
      .then((createdSession) => {
        creationDraftDestinationsRef.current.set(draftOwnerId, createdSession.id);
        transferPendingAgentSessionDraft(draftOwnerId, createdSession.id);
        setSelectedId(createdSession.id);
        selectedIdRef.current = createdSession.id;
        setNewSessionMode(false);
        setSessions((current) => [
          createdSession,
          ...current.filter((session) => session.id !== createdSession.id),
        ]);
        setProjection(createAgentRuntimeProjection({ session: createdSession }));
        rememberSessionThinkingLevel(createdSession.id, "instant");
        onHomeSessionCreated?.(createdSession.id);
        return createdSession.id;
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

  function commitHomeDirectTurns(storedSessionId: string, nextTurns: AgentChatTurn[]) {
    homeDirectTurnsRef.current = nextTurns;
    setHomeDirectTurns(nextTurns);
    persistHomeDirectTurns(storedSessionId, nextTurns);
  }

  async function startHomeTask(
    request: ClovyHomeTaskRequest,
    toolCallId: string,
    conversation: ClovyHomeConversationContext,
    homeStoredSessionId: string,
    profile: string,
    taskAttachments: string[] = [],
    sourceUserTurnId?: string,
  ) {
    const activeHomeSessionId = resolveClovyHomeThreadSessionId(homeStoredSessionId);
    if (!activeHomeSessionId) return;
    if (handledHomeTaskToolCallsRef.current.has(toolCallId)) return;
    const handoffId = `home-task-${toolCallId}`;
    const storedHandoffs = readHomeTaskHandoffs(homeStoredSessionId);
    const existing = storedHandoffs.find((handoff) => handoff.id === handoffId);
    if (existing && existing.status !== "failed") {
      handledHomeTaskToolCallsRef.current.add(toolCallId);
      return;
    }
    const existingForSourceTurn = existingHomeTaskHandoffForSourceTurn(
      storedHandoffs,
      sourceUserTurnId,
    );
    if (existingForSourceTurn) {
      handledHomeTaskToolCallsRef.current.add(toolCallId);
      return;
    }
    handledHomeTaskToolCallsRef.current.add(toolCallId);
    markHomeTaskHandoffActive(homeStoredSessionId, handoffId);
    const starting: HomeTaskHandoff = {
      ...request,
      id: handoffId,
      status: "starting",
      profile,
      ...(sourceUserTurnId ? { sourceUserTurnId } : {}),
      ...(taskAttachments.length ? { attachments: taskAttachments } : {}),
    };
    const nextHandoffs = storedHandoffs.some((handoff) => handoff.id === handoffId)
      ? storedHandoffs.map((handoff) => (handoff.id === handoffId ? starting : handoff))
      : [...storedHandoffs, starting];
    setHomeTaskHandoffs(nextHandoffs);
    persistHomeTaskHandoffs(homeStoredSessionId, nextHandoffs);

    const updateHandoff = (patch: Partial<HomeTaskHandoff>) => {
      const next = readHomeTaskHandoffs(homeStoredSessionId).map((handoff) =>
        handoff.id === handoffId ? { ...handoff, ...patch } : handoff,
      );
      setHomeTaskHandoffs(next);
      persistHomeTaskHandoffs(homeStoredSessionId, next);
    };

    let focusedSession: AgentSessionDto | undefined;
    try {
      const focusedModel = agentRunModelId(focusedHomeModelRef.current, costQualityRef.current);
      const focusedThinkingLevel = focusedHomeThinkingLevelRef.current;
      focusedSession = await agentRuntimeBindings.createSession({
        title: request.title,
        model: focusedModel,
        safetyMode: "sandboxed",
        profile: activeHomeSessionId === homeStoredSessionId ? profile : "default",
      });
      const activeAfterCreation = resolveClovyHomeThreadSessionId(homeStoredSessionId);
      if (!activeAfterCreation) {
        await agentRuntimeBindings.deleteSession(focusedSession.id);
        return;
      }
      if (activeAfterCreation !== activeHomeSessionId) {
        await assignSessionToProfile(focusedSession.id, "default");
      }
      const enabledSkillIds = (await agentRuntimeBindings.listSkills())
        .filter((skill) => skill.enabled)
        .map((skill) => skill.id);
      const runtimePrompt = request.requiresCurrentResearch
        ? withClovyHomeCurrentResearch(request.prompt, conversation)
        : request.prompt;
      await agentRuntimeBindings.startRun({
        sessionId: focusedSession.id,
        prompt: runtimePrompt,
        model: focusedModel,
        reasoningEffort: thinkingEffortForLevel(focusedThinkingLevel) as
          | "minimal"
          | "medium"
          | "high",
        safetyMode: "sandboxed",
        workspacePath: focusedSession.workspacePath,
        enabledSkillIds,
        attachments: taskAttachments,
      });
      void discardStagedAgentAttachments(taskAttachments).catch(() => undefined);
      updateHandoff({ status: "running", storedSessionId: focusedSession.id });
      void refreshSessions().catch(() => undefined);
    } catch (cause) {
      if (focusedSession) {
        await agentRuntimeBindings.deleteSession(focusedSession.id).catch(() => undefined);
      }
      updateHandoff({ status: "failed", error: messageFromError(cause) });
    } finally {
      clearHomeTaskHandoffActive(homeStoredSessionId, handoffId);
    }
  }

  function retryHomeTask(handoff: HomeTaskHandoff) {
    if (handoff.status !== "failed") return;
    const toolCallId = handoff.id.replace(/^home-task-/, "");
    handledHomeTaskToolCallsRef.current.delete(toolCallId);
    const homeStoredSessionId = selectedIdRef.current;
    if (!homeStoredSessionId) return;
    const conversation = homeConversationContextFromTurns([
      ...turns,
      ...readHomeDirectTurns(homeStoredSessionId),
    ]);
    void startHomeTask(
      {
        title: handoff.title,
        prompt: handoff.prompt,
        ...(handoff.summary ? { summary: handoff.summary } : {}),
        ...(handoff.requiresCurrentResearch ? { requiresCurrentResearch: true } : {}),
      },
      toolCallId,
      conversation,
      homeStoredSessionId,
      handoff.profile ?? getCurrentDataPartitionName(),
      handoff.attachments ?? [],
      handoff.sourceUserTurnId,
    );
  }

  async function submitHomeMessage(event?: FormEvent) {
    event?.preventDefault();
    if (recoverableHomeSubmissionRef.current) {
      setError("Retry or discard the unsent Home message before sending another.");
      return;
    }
    if (dropOwnerRef.current) {
      setError("Wait for files to finish attaching, then send again.");
      return;
    }
    if (!selectedIdRef.current && homeSessionPromiseRef.current) {
      setError("Wait for Home to finish starting, then send again.");
      return;
    }
    const message = draftRef.current.trim();
    if (!message || submitting || textActionsDisabledReason) return;
    if (Array.from(message).length > 64_000) {
      setError("Home messages must be 64,000 characters or less.");
      return;
    }
    const submittedHomeStoredSessionId = selectedIdRef.current;
    const submittedHomeDraftRevision =
      submittedHomeStoredSessionId &&
      readAgentSessionDraft(submittedHomeStoredSessionId)?.trim() === message
        ? readAgentSessionDraftRevision(submittedHomeStoredSessionId)
        : undefined;
    const profile = getCurrentDataPartitionName();
    const messageAttachments = attachmentsRef.current;
    const attachmentLeaseId = `home:${crypto.randomUUID()}`;
    if (messageAttachments.length > 0) {
      inFlightAttachmentLeasesRef.current.set(attachmentLeaseId, [...new Set(messageAttachments)]);
    }
    const messageAttachmentGeneration = attachmentGenerationRef.current;
    setComposerDraft("", { persist: false });
    setComposerAttachments([]);
    setHomeDirectPendingCount((count) => count + 1);

    let storedSessionId: string | undefined;
    try {
      storedSessionId = await ensureHomeSession();
      const suffix = `${Date.now()}:${Math.random().toString(36).slice(2)}`;
      const userTurn: AgentChatTurn = {
        id: `home:direct:user:${suffix}`,
        role: "user",
        createdAt: new Date().toISOString(),
        status: "complete",
        parts: [
          { type: "text", text: message, status: "complete" },
          ...messageAttachments.map((path) => ({
            type: "attachment" as const,
            name: path.split(/[\\/]/).pop() || path,
            path,
            kind: /\.(?:png|jpe?g|gif|webp|tiff?|bmp|avif)$/i.test(path)
              ? ("image" as const)
              : ("file" as const),
          })),
        ],
      };
      const priorDirectTurns = readHomeDirectTurns(storedSessionId);
      const acknowledgesTaskHandoff = isHomeTaskHandoffAcknowledgement(
        message,
        priorDirectTurns,
        readHomeTaskHandoffs(storedSessionId),
      );
      const greetingReply = homeConversationGreetingReply(message);
      const directConversationReply = acknowledgesTaskHandoff ? "Got it." : greetingReply;
      commitHomeDirectTurns(storedSessionId, [...priorDirectTurns, userTurn]);
      clearAgentSessionDraftRevision(storedSessionId, submittedHomeDraftRevision);
      requestSubmittedMessageScroll();

      if (directConversationReply && messageAttachments.length === 0) {
        const assistantTurn: AgentChatTurn = {
          id: `home:direct:assistant:${suffix}`,
          role: "assistant",
          createdAt: new Date().toISOString(),
          status: "complete",
          parts: [{ type: "text", text: directConversationReply, status: "complete" }],
        };
        const nextTurns = insertHomeDirectReply(storedSessionId, userTurn.id, assistantTurn);
        homeDirectTurnsRef.current = nextTurns;
        setHomeDirectTurns(nextTurns);
        setError(undefined);
        return;
      }

      // Attachments and commands need the full Clovy tool/runtime context. Home
      // hands them to a focused session deterministically instead of running a
      // second hidden agent turn and hoping it emits a legacy bridge tool.
      if (messageAttachments.length > 0 || message.startsWith("/")) {
        const toolCallId = `focused:${suffix}`;
        const assistantTurn: AgentChatTurn = {
          id: `home:direct:assistant:${suffix}`,
          role: "assistant",
          createdAt: new Date().toISOString(),
          status: "complete",
          parts: [
            {
              type: "tool",
              id: toolCallId,
              name: "clovy_home_start_task",
              text: "",
              status: "complete",
            },
          ],
        };
        const nextTurns = insertHomeDirectReply(storedSessionId, userTurn.id, assistantTurn);
        homeDirectTurnsRef.current = nextTurns;
        setHomeDirectTurns(nextTurns);
        const conversation = homeConversationContextFromTurns([...turns, ...nextTurns]);
        void startHomeTask(
          { title: titleFromPrompt(message), prompt: message },
          toolCallId,
          conversation,
          storedSessionId,
          profile,
          messageAttachments,
          userTurn.id,
        ).finally(() => inFlightAttachmentLeasesRef.current.delete(attachmentLeaseId));
        setError(undefined);
        return;
      }

      await enqueueHomeDirectChat(storedSessionId, async () => {
        const currentTurns = readHomeDirectTurns(storedSessionId as string);
        const userIndex = currentTurns.findIndex((turn) => turn.id === userTurn.id);
        const contextTurns = userIndex < 0 ? currentTurns : currentTurns.slice(0, userIndex + 1);
        const conversation = homeConversationContextFromTurns([...turns, ...contextTurns]);
        const streamingTurnId = `home:direct:assistant:${suffix}`;
        const streamingStartedAt = new Date().toISOString();
        let streamedContent = "";
        let acceptingDeltas = true;
        const onDelta = (content: string) => {
          if (!acceptingDeltas) return;
          streamedContent += content;
          setHomeStreamingReply({
            id: `${streamingTurnId}:stream`,
            role: "assistant",
            createdAt: streamingStartedAt,
            status: "running",
            parts: [{ type: "text", text: streamedContent, status: "running" }],
          });
        };
        let response: ClovyHomeChatResponse;
        try {
          response =
            (await homeDemoReply(profile, onDelta)) ??
            (await clovyHomeChat(conversation.recentMessages, {
              profile,
              ...(conversation.earlierContext
                ? { historyContext: conversation.earlierContext }
                : {}),
              onDelta,
            }));
        } finally {
          acceptingDeltas = false;
        }
        const latestHandoffs = readHomeTaskHandoffs(storedSessionId as string);
        const rejectedStaleTask = Boolean(
          response.task && isHomeTaskReplayWithoutNewIntent(response.task, message, latestHandoffs),
        );
        const responseTask =
          rejectedStaleTask || !response.task
            ? undefined
            : {
                ...response.task,
                prompt: withClovyHomeLatestTaskIntent(response.task.prompt, message),
              };
        const toolCallId = responseTask ? `direct:${suffix}` : undefined;
        const assistantTurn: AgentChatTurn =
          responseTask && toolCallId
            ? {
                id: streamingTurnId,
                role: "assistant",
                createdAt: new Date().toISOString(),
                status: "complete",
                parts: [
                  {
                    type: "tool",
                    id: toolCallId,
                    name: "clovy_home_start_task",
                    text: "",
                    status: "complete",
                  },
                ],
              }
            : {
                id: streamingTurnId,
                role: "assistant",
                createdAt: new Date().toISOString(),
                status: "complete",
                parts: [
                  {
                    type: "text",
                    text: rejectedStaleTask
                      ? "I'm here. What can I help with?"
                      : response.content?.trim() || streamedContent.trim() || "I'm here.",
                    status: "complete",
                  },
                ],
              };
        const nextTurns = insertHomeDirectReply(
          storedSessionId as string,
          userTurn.id,
          assistantTurn,
        );
        homeDirectTurnsRef.current = nextTurns;
        setHomeDirectTurns(nextTurns);
        setHomeStreamingReply(null);
        if (responseTask && toolCallId) {
          void startHomeTask(
            responseTask,
            toolCallId,
            conversation,
            storedSessionId as string,
            profile,
            [],
            userTurn.id,
          );
        }
      });
      setError(undefined);
    } catch (cause) {
      setHomeStreamingReply(null);
      // The persisted user bubble is the durable retry record. Never remove it
      // when a newer draft exists or when this workspace unmounted while the
      // request was in flight. Restore the text only when the composer is free.
      if (attachmentGenerationRef.current === messageAttachmentGeneration) {
        if (draftHasContentRef.current || attachmentsRef.current.length > 0) {
          setRecoverableHomeSubmission({
            id: attachmentLeaseId,
            prompt: message,
            attachments: messageAttachments,
          });
        } else {
          setComposerDraft(message);
          setComposerAttachments(messageAttachments);
        }
      } else if (messageAttachments.length > 0) {
        void discardStagedAgentAttachments(messageAttachments).catch(() => undefined);
      }
      inFlightAttachmentLeasesRef.current.delete(attachmentLeaseId);
      setError(messageFromError(cause));
    } finally {
      setHomeDirectPendingCount((count) => Math.max(0, count - 1));
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
      dispatchAgentSessionStatus({
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

  async function compactContext() {
    if (!selectedId || !agentRuntimeBindings.compactSession) return;
    setCompacting(true);
    setCompactResult(undefined);
    try {
      const result = await agentRuntimeBindings.compactSession(selectedId);
      setCompactResult(
        result.compacted
          ? `Context compacted. ${result.removedItems} earlier items were replaced with a summary.`
          : "There is not enough earlier context to compact yet.",
      );
      await hydrate(selectedId);
    } catch (cause) {
      setCompactResult(messageFromError(cause));
    } finally {
      setCompacting(false);
    }
  }

  async function branchFrom(itemId: string) {
    if (!selectedId || !agentRuntimeBindings.branchSession) return;
    setBranchingItemId(itemId);
    setError(undefined);
    try {
      const branch = await agentRuntimeBindings.branchSession(selectedId, itemId);
      abandonComposerAttachments();
      setSessions((current) => [branch, ...current.filter((item) => item.id !== branch.id)]);
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
    const selected = await openFileDialog({ multiple: true, title: "Attach files" });
    if (!selected) return;
    const paths = Array.isArray(selected) ? selected : [selected];
    setComposerAttachments((current) => [...new Set([...current, ...paths])].slice(0, 8));
  }

  async function addDroppedAttachments(files: File[]) {
    if (dropOwnerRef.current) {
      setError("Wait for the current files to finish attaching, then drop these files again.");
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
        ...(recoverableHomeSubmissionRef.current?.attachments ?? []),
        ...readAllHomeTaskHandoffAttachments(),
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
        setError("You can attach up to 8 files at a time.");
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
    if (creditActionsDisabledReason) {
      setError(creditActionsDisabledReason);
      return;
    }
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
    projectContextSignaturesBySessionId.delete(removedStoredSessionId);
    forgetSessionThinkingLevel(removedStoredSessionId);
    forgetSessionModel(removedStoredSessionId);
    forgetLastOpenSessionId(removedStoredSessionId);
    invalidateAgentSessionDraft(removedStoredSessionId);
    updateQueuedFollowUps((current) => {
      if (!(removedStoredSessionId in current)) return current;
      const next = { ...current };
      delete next[removedStoredSessionId];
      return next;
    });
    abandonComposerAttachments();
    setSelectedId(undefined);
    selectedIdRef.current = undefined;
    setComposerDraft("", { persist: false });
    setProjection(createAgentRuntimeProjection());
    setArtifacts([]);
    setNewSessionMode(true);
    onSessionSelected?.(undefined);
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
        ...homeDirectTurns,
        ...(homeStreamingReply ? [homeStreamingReply] : []),
      ].sort(compareHomeTurnOrder),
    [homeDirectTurns, homeStreamingReply, turns],
  );
  const homeHandoffsByTurnId = useMemo(() => {
    const handoffs = new Map<string, HomeTaskHandoff>();
    for (const turn of homeConversationTurns) {
      const tool = turn.parts.find(
        (part) =>
          part.type === "tool" &&
          homeTaskHandoffs.some((handoff) => handoff.id === `home-task-${part.id}`),
      );
      const handoff =
        tool?.type === "tool"
          ? homeTaskHandoffs.find((candidate) => candidate.id === `home-task-${tool.id}`)
          : undefined;
      if (handoff) handoffs.set(turn.id, handoff);
    }
    return handoffs;
  }, [homeConversationTurns, homeTaskHandoffs]);
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
    displayName: homeUserDisplayName,
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
  const runDisplayModel = agentModelSelection(projection.run?.model ?? "").modelId;
  const sessionDisplayModel = agentModelSelection(selectedSession?.model ?? model).modelId;
  const usageModel = selectedModel(
    models,
    models.some((candidate) => candidate.id === runDisplayModel)
      ? runDisplayModel
      : sessionDisplayModel,
  );
  const runUsage = projection.run?.usage;
  const contextLimit = usageModel?.contextTokens;
  const contextUsed = runUsage?.inputTokens;
  const contextPercent =
    contextUsed !== undefined && contextLimit !== undefined && contextLimit > 0
      ? Math.min(100, (contextUsed / contextLimit) * 100)
      : undefined;
  const estimatedCredits =
    runUsage?.inputTokens !== undefined &&
    runUsage.outputTokens !== undefined &&
    usageModel?.inputCreditsPerMillionTokens !== undefined &&
    usageModel.outputCreditsPerMillionTokens !== undefined
      ? (runUsage.inputTokens * usageModel.inputCreditsPerMillionTokens +
          runUsage.outputTokens * usageModel.outputCreditsPerMillionTokens) /
        1_000_000
      : undefined;
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
        <span className="agent-follow-up-announcement">Unsent message</span>
        <span className="agent-follow-up-text">{recoverableSubmission.prompt}</span>
      </span>
      <span className="agent-follow-up-actions">
        <button
          type="button"
          aria-label="Retry unsent message"
          disabled={running || waiting || submitting || Boolean(textActionsDisabledReason)}
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
          aria-label="Discard unsent message"
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
  const recoverableHomeSubmissionRow = recoverableHomeSubmission ? (
    <div className="agent-follow-up-row" role="status">
      <span className="agent-follow-up-copy">
        <span className="agent-follow-up-announcement">Unsent Home message</span>
        <span className="agent-follow-up-text">{recoverableHomeSubmission.prompt}</span>
      </span>
      <span className="agent-follow-up-actions">
        <button
          type="button"
          aria-label="Retry unsent Home message"
          disabled={
            homeSessionCreating || Boolean(draftRef.current.trim()) || attachments.length > 0
          }
          onClick={() => {
            setComposerDraft(recoverableHomeSubmission.prompt);
            setComposerAttachments(recoverableHomeSubmission.attachments);
            setRecoverableHomeSubmission(undefined);
            setError(undefined);
          }}
        >
          <IconArrowRotateClockwise size={12} aria-hidden />
        </button>
        <button
          type="button"
          aria-label="Discard unsent Home message"
          onClick={() => {
            void discardStagedAgentAttachments(recoverableHomeSubmission.attachments).catch(
              () => undefined,
            );
            setRecoverableHomeSubmission(undefined);
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
      setModel={(nextModel, nextCostQuality) => {
        const selectedCostQuality =
          nextModel === AUTO_MODEL_ID ? (nextCostQuality ?? costQualityRef.current) : undefined;
        setModel(nextModel);
        if (selectedId) {
          if (selectedCostQuality !== undefined) {
            costQualityRef.current = selectedCostQuality;
            setCostQuality(selectedCostQuality);
          }
          sessionCostQualityExplicitRef.current = selectedCostQuality !== undefined;
          rememberSessionModel(selectedId, agentRunModelId(nextModel, selectedCostQuality));
          return;
        }
        if (pendingSessionCreationRef.current) return;
        if (nextCostQuality !== undefined) applyCostQuality(nextCostQuality);
        void persistAgentDefaultModel(nextModel).catch((cause) => {
          if (modelRef.current === nextModel) setError(messageFromError(cause));
        });
      }}
      costQuality={costQuality}
      onCostQualityChange={applyCostQuality}
      thinkingLevel={thinkingLevel}
      setThinkingLevel={(level) => {
        setThinkingLevel(level);
        saveThinkingLevel(level);
        if (selectedId) rememberSessionThinkingLevel(selectedId, level);
      }}
      models={models}
      safetyMode={safetyMode}
      setSafetyMode={setSafetyMode}
      attachments={attachments}
      onDropAttachments={addDroppedAttachments}
      onRemoveAttachment={removeAttachment}
      onPickAttachments={pickAttachments}
      onDictate={startDictation}
      onSubmit={homeMode ? submitHomeMessage : submit}
      onStop={stop}
      running={running}
      waiting={waiting}
      stopping={stoppingRunId === projection.run?.id}
      submitting={
        submitting || (homeMode && (homeSessionCreating || Boolean(recoverableHomeSubmission)))
      }
      attaching={attaching}
      disabledReason={textActionsDisabledReason}
      notice={!heroMode ? error : undefined}
      hero={heroMode}
      showModelPicker={!homeMode}
    />
  );
  return (
    <>
      <section
        className="agent-workspace"
        aria-label={homeMode ? "Home" : "Session"}
        data-hero={heroMode ? "true" : undefined}
        data-home={homeMode ? "true" : undefined}
      >
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
            shareUrl={shareUrl}
            onShare={
              canShareAgentSession({
                selectedSessionId: selectedId,
                newSessionMode,
                provisional: false,
                historyLoaded: true,
                working: running || waiting,
              })
                ? () => setShareOpen(true)
                : undefined
            }
            onUsage={sessionActionsAvailable ? () => setUsageOpen(true) : undefined}
            onCompact={
              sessionActionsAvailable && agentRuntimeBindings.compactSession && !running && !waiting
                ? () => {
                    setCompactResult(undefined);
                    setCompactOpen(true);
                  }
                : undefined
            }
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
            <main className="agent-main" aria-label="Home conversation">
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
                        homeTaskHandoff={homeHandoffsByTurnId.get(turn.id)}
                        onOpenHomeTaskSession={onOpenHomeTaskSession}
                        onRetryHomeTask={retryHomeTask}
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
                  <section className="agent-home-nudges" aria-label="Suggestions">
                    {homeNudgePrompts.map((prompt) => (
                      <button key={prompt} type="button" onClick={() => setComposerDraft(prompt)}>
                        {prompt}
                      </button>
                    ))}
                  </section>
                ) : null}
                <AgentThinking
                  variant="typing-bubble"
                  visible={!homeStreamingReply && homeDirectPendingCount > 0}
                />
              </div>
              {recoverableHomeSubmissionRow}
              {composer}
            </main>
          </div>
        ) : heroMode ? (
          <main className="agent-main" aria-label="Agent task details" data-hero="true">
            {error ? (
              <div className="agent-composer-notice" role="alert">
                {error}
              </div>
            ) : null}
            <div className="agent-hero-heading">
              <h2 className="agent-hero-title">{heroGreeting}</h2>
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
              <p className="agent-hero-footnote">
                {heroPrivacyFootnote(
                  activeModel,
                  activeModel ? modelPrivacyBadge(activeModel) : undefined,
                )}
              </p>
            </div>
          </main>
        ) : (
          <div
            ref={scrollRef}
            className="agent-scroll"
            style={{ "--agent-composer-clearance": `${composerClearance}px` } as CSSProperties}
          >
            <main className="agent-main" aria-label="Agent task details">
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
                            ? "Retry queued attachments"
                            : "Retry queued follow-up"
                        }
                        disabled={
                          running || waiting || submitting || Boolean(textActionsDisabledReason)
                        }
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
                          ? "Remove queued attachments"
                          : "Remove queued follow-up"
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
      {reportDialogCategory ? (
        <ReportDialog
          category={reportDialogCategory}
          storedSessionId={reportStoredSessionId}
          onCategoryChange={setReportDialogCategory}
          onClose={() => {
            setReportDialogCategory(undefined);
            setReportStoredSessionId(undefined);
          }}
        />
      ) : null}
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
                    Usage
                  </h2>
                  <button
                    type="button"
                    className="icon-button"
                    aria-label="Close usage"
                    onClick={() => setUsageOpen(false)}
                  >
                    <IconCrossSmall size={14} />
                  </button>
                </div>
                <div className="agent-usage-body">
                  <div className="agent-usage-row">
                    <span className="agent-usage-primary">Model</span>
                    <span className="agent-usage-value">
                      {usageModel?.name ?? sessionDisplayModel}
                    </span>
                  </div>
                  {projection.run?.usage?.provider || usageModel?.provider ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">Provider</span>
                      <span className="agent-usage-value">
                        {projection.run?.usage?.provider ?? usageModel?.provider}
                      </span>
                    </div>
                  ) : null}
                  {projection.run?.usage?.privacyLevel || usageModel?.privacy ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">Privacy</span>
                      <span className="agent-usage-value">
                        {projection.run?.usage?.privacyLevel ?? usageModel?.privacy}
                      </span>
                    </div>
                  ) : null}
                  {projection.run?.usage?.endpoint ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">Route</span>
                      <span className="agent-usage-value">{projection.run.usage.endpoint}</span>
                    </div>
                  ) : null}
                  {projection.run?.reasoningEffort ? (
                    <div className="agent-usage-row">
                      <span className="agent-usage-primary">Reasoning effort</span>
                      <span className="agent-usage-value">{projection.run.reasoningEffort}</span>
                    </div>
                  ) : null}
                  {projection.run?.usage ? (
                    <>
                      {projection.run.usage.inputTokens !== undefined ? (
                        <div className="agent-usage-row">
                          <span className="agent-usage-primary">Input</span>
                          <span className="agent-usage-value">
                            {projection.run.usage.inputTokens.toLocaleString()}
                          </span>
                        </div>
                      ) : null}
                      {projection.run.usage.outputTokens !== undefined ? (
                        <div className="agent-usage-row">
                          <span className="agent-usage-primary">Output</span>
                          <span className="agent-usage-value">
                            {projection.run.usage.outputTokens.toLocaleString()}
                          </span>
                        </div>
                      ) : null}
                      {projection.run.usage.totalTokens !== undefined ? (
                        <div className="agent-usage-row">
                          <span className="agent-usage-primary">Total</span>
                          <span className="agent-usage-value">
                            {projection.run.usage.totalTokens.toLocaleString()}
                          </span>
                        </div>
                      ) : null}
                      {projection.run.usage.inputTokens === undefined &&
                      projection.run.usage.outputTokens === undefined &&
                      projection.run.usage.totalTokens === undefined ? (
                        <p className="agent-usage-empty">
                          Token counts were not reported for this request.
                        </p>
                      ) : null}
                      {contextPercent !== undefined && contextUsed !== undefined && contextLimit ? (
                        <div className="agent-usage-context">
                          <div className="agent-usage-row">
                            <span className="agent-usage-primary">Latest request context</span>
                            <span className="agent-usage-value">
                              {contextUsed.toLocaleString()} of {contextLimit.toLocaleString()} (
                              {contextPercent.toFixed(1)}%)
                            </span>
                          </div>
                          <div
                            className="agent-usage-context-track"
                            role="progressbar"
                            aria-label="Context used"
                            aria-valuemin={0}
                            aria-valuemax={100}
                            aria-valuenow={Math.round(contextPercent)}
                          >
                            <span style={{ transform: `scaleX(${contextPercent / 100})` }} />
                          </div>
                        </div>
                      ) : null}
                      {estimatedCredits !== undefined ? (
                        <div className="agent-usage-row">
                          <span className="agent-usage-primary">Estimated charge</span>
                          <span className="agent-usage-value">
                            {estimatedCredits.toLocaleString(undefined, {
                              maximumFractionDigits: estimatedCredits < 1 ? 3 : 1,
                            })}{" "}
                            credits (about ${(estimatedCredits / 1_000).toFixed(4)})
                          </span>
                        </div>
                      ) : null}
                      {toolUsage.size > 0 ? (
                        <div className="agent-usage-tools">
                          <p className="agent-usage-section-title">Tools</p>
                          {[...toolUsage.entries()].map(([name, usage]) => (
                            <div className="agent-usage-row" key={name}>
                              <span className="agent-usage-primary">{name}</span>
                              <span className="agent-usage-value">
                                {usage.calls} {usage.calls === 1 ? "call" : "calls"}
                                {usage.failures > 0 ? `, ${usage.failures} failed` : ""}
                              </span>
                            </div>
                          ))}
                        </div>
                      ) : null}
                    </>
                  ) : (
                    <p className="agent-usage-empty">No usage reported for this session yet.</p>
                  )}
                </div>
              </aside>
            </div>,
            document.querySelector(".app-shell") ?? document.body,
          )
        : null}
      {selectedSession ? (
        <ShareDialog
          key={selectedSession.id}
          open={shareOpen}
          onClose={() => setShareOpen(false)}
          onLinkChange={(url) => setShareUrl(url ?? undefined)}
          item={{
            kind: "session",
            itemId: selectedSession.id,
            title: selectedSession.title,
            buildPayload: () =>
              buildSessionPayload({
                title: selectedSession.title,
                messages: projection.items.flatMap((item) =>
                  item.kind === "message" && (item.role === "user" || item.role === "assistant")
                    ? [
                        {
                          role: item.role,
                          content:
                            item.role === "user" ? stripProjectContext(item.text) : item.text,
                        },
                      ]
                    : [],
                ),
              }),
          }}
        />
      ) : null}
      <Dialog
        open={compactOpen}
        onClose={() => {
          if (!compacting) setCompactOpen(false);
        }}
        title="Compact context?"
        description="Clovy will replace older conversation turns with one visible summary and keep recent turns unchanged."
        footer={
          <>
            <button
              type="button"
              className="primary-action"
              disabled={compacting}
              onClick={() => setCompactOpen(false)}
            >
              {compactResult ? "Close" : "Cancel"}
            </button>
            {!compactResult ? (
              <button
                type="button"
                className="primary-action primary-solid"
                disabled={compacting}
                onClick={() => void compactContext()}
              >
                {compacting ? "Compacting" : "Compact context"}
              </button>
            ) : null}
          </>
        }
      >
        {compactResult ? <p role="status">{compactResult}</p> : null}
      </Dialog>
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
  costQuality,
  onCostQualityChange,
  thinkingLevel,
  setThinkingLevel,
  models,
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
  setModel: (value: string, costQuality?: number) => void;
  costQuality: number;
  onCostQualityChange: (value: number) => void;
  thinkingLevel: ThinkingLevel;
  setThinkingLevel: (value: ThinkingLevel) => void;
  models: VeniceModelDto[];
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
}) {
  const editorRef = useRef<ComposerEditorHandle>(null);
  const [editorDraftOwnerId, setEditorDraftOwnerId] = useState(draftOwnerId);
  const ownerTransitionHandledRef = useRef(false);
  const publishedDraftRef = useRef(draft);
  const appliedDraftRevisionRef = useRef(draftRevision);
  const [hasEditorContent, setHasEditorContent] = useState(Boolean(draft.trim()));
  const [modelOpen, setModelOpen] = useState(false);
  const [modelFlyout, setModelFlyout] = useState<ModelPickerFlyout>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [modelRootSearch, setModelRootSearch] = useState("");
  const modelTriggerRef = useRef<HTMLButtonElement>(null);
  const modelPopoverRef = useRef<HTMLDivElement>(null);
  const modelSearchRef = useRef<HTMLInputElement>(null);
  const modelRootSearchRef = useRef<HTMLInputElement>(null);
  const [safetyOpen, setSafetyOpen] = useState(false);
  const [attachOpen, setAttachOpen] = useState(false);
  const [dropActive, setDropActive] = useState(false);
  const [confirmUnrestricted, setConfirmUnrestricted] = useState(false);
  const attachTriggerRef = useRef<HTMLButtonElement>(null);
  const attachMenuRef = useRef<HTMLDivElement>(null);
  const safetyTriggerRef = useRef<HTMLButtonElement>(null);
  const safetyMenuRef = useRef<HTMLDivElement>(null);
  const activeModel = selectedModel(models, model);
  const pickerModels = models.some((option) => option.id === AUTO_MODEL_ID)
    ? models
    : [AGENT_AUTO_MODEL, ...models];

  useComposerModelPopoverPosition({
    open: modelOpen,
    triggerRef: modelTriggerRef,
    popoverRef: modelPopoverRef,
    anchorRef: formRef,
  });

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
    if (!modelOpen && !safetyOpen && !attachOpen) return;
    const close = (event: MouseEvent) => {
      const target = event.target as Node;
      if (modelPopoverRef.current?.contains(target) || modelTriggerRef.current?.contains(target)) {
        return;
      }
      if (safetyTriggerRef.current?.contains(target)) return;
      if (safetyMenuRef.current?.contains(target)) return;
      if (attachTriggerRef.current?.contains(target) || attachMenuRef.current?.contains(target)) {
        return;
      }
      if (target instanceof Element && target.closest(".agent-composer-model-hovercard")) return;
      setModelOpen(false);
      setSafetyOpen(false);
      setAttachOpen(false);
    };
    window.addEventListener("mousedown", close);
    return () => window.removeEventListener("mousedown", close);
  }, [attachOpen, modelOpen, safetyOpen]);

  useEffect(() => {
    if (!modelOpen && !safetyOpen && !attachOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      event.preventDefault();
      if (modelOpen) {
        if (modelFlyout) {
          if (modelFlyout.kind === "all") setModelSearch("");
          setModelFlyout(null);
          requestAnimationFrame(() => modelRootSearchRef.current?.focus());
          return;
        }
        if (modelRootSearch) {
          setModelRootSearch("");
          requestAnimationFrame(() => modelRootSearchRef.current?.focus());
          return;
        }
        setModelOpen(false);
        requestAnimationFrame(() => modelTriggerRef.current?.focus());
        return;
      }
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
  }, [attachOpen, modelFlyout, modelOpen, modelRootSearch, safetyOpen]);

  useLayoutEffect(() => {
    if (modelOpen && modelFlyout === null) modelRootSearchRef.current?.focus();
  }, [modelFlyout, modelOpen]);

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
          placeholder={hero ? "Ask Clovy anything, run / commands" : "Send a message"}
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
            aria-label="Add files or notes"
            title="Add"
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
              title="Change what Clovy can touch"
              onClick={() => setSafetyOpen((open) => !open)}
            >
              {safetyMode === "sandboxed" ? (
                <IconShieldCheck size={14} />
              ) : (
                <IconShieldCrossed size={14} />
              )}
              {safetyMode === "sandboxed" ? "Sandboxed" : "Unrestricted"}
              <IconChevronDownSmall size={12} aria-hidden />
            </button>
          ) : null}
          <div className="agent-composer-actions">
            {showModelPicker ? (
              <ComposerModelPicker
                open={modelOpen}
                model={activeModel}
                detail={model === AUTO_MODEL_ID ? autoPillDesignation(costQuality) : undefined}
                effort={thinkingLevel}
                triggerRef={modelTriggerRef}
                onToggleOpen={() => {
                  if (modelOpen) {
                    setModelOpen(false);
                    return;
                  }
                  setModelFlyout(null);
                  setModelSearch("");
                  setModelRootSearch("");
                  setModelOpen(true);
                }}
              />
            ) : null}
            <button
              type="button"
              className="agent-composer-mic"
              aria-label="Dictate"
              title={disabledReason ?? "Start dictation"}
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
                    aria-label="Steer active run"
                    disabled={attaching}
                    title={attaching ? "Wait for files to finish attaching" : undefined}
                  >
                    <IconArrowUp size={18} />
                  </button>
                ) : null}
                <button
                  type="button"
                  className="agent-composer-stop"
                  aria-label="Stop Clovy"
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
                aria-label="Stop Clovy"
                disabled={stopping}
                onClick={() => void onStop()}
              >
                <IconStop size={16} />
              </button>
            ) : (
              <button
                type="submit"
                className="agent-composer-send"
                aria-label="Send message"
                disabled={
                  submitting ||
                  attaching ||
                  draftOwnerId !== editorDraftOwnerId ||
                  !hasEditorContent ||
                  Boolean(disabledReason)
                }
                title={attaching ? "Wait for files to finish attaching" : disabledReason}
              >
                {submitting ? <Spinner /> : <IconArrowUp size={18} />}
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
          aria-label="Add files or notes"
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
            <span className="agent-attach-menu-label">Attach files</span>
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
            <span className="agent-attach-menu-label">Reference a note</span>
          </button>
        </div>
      ) : null}
      {hero && safetyOpen ? (
        <div
          ref={safetyMenuRef}
          className="agent-sandbox-menu"
          role="menu"
          aria-label="Safety mode"
        >
          <p className="agent-sandbox-menu-title">Choose what Clovy can touch</p>
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
      {showModelPicker && modelOpen ? (
        <ModelPickerPopover
          mode="generation"
          flyout={modelFlyout}
          model={activeModel}
          options={modelOptions(pickerModels, model)}
          costQuality={costQuality}
          search={modelSearch}
          popoverRef={modelPopoverRef}
          searchRef={modelSearchRef}
          rootSearchRef={modelRootSearchRef}
          rootSearch={modelRootSearch}
          onRootSearchChange={setModelRootSearch}
          catalogLoaded={models.length > 0}
          suggestedModelIds={AGENT_SUGGESTED_MODEL_IDS}
          showAutoToggle={false}
          thinkingLevel={thinkingLevel}
          onFlyoutChange={setModelFlyout}
          onSearchChange={setModelSearch}
          onSelect={(nextModel, nextCostQuality, options) => {
            setModel(nextModel, nextCostQuality);
            if (!options?.keepOpen) {
              setModelOpen(false);
              requestAnimationFrame(() => modelTriggerRef.current?.focus());
            }
          }}
          onCostQualityChange={onCostQualityChange}
          onSelectThinking={(level) => {
            setThinkingLevel(level);
            setModelFlyout(null);
            setModelOpen(false);
            requestAnimationFrame(() => modelTriggerRef.current?.focus());
          }}
        />
      ) : null}
      <Dialog
        open={confirmUnrestricted}
        onClose={() => setConfirmUnrestricted(false)}
        title="Turn on unrestricted?"
        description="Clovy will be able to change any file your account can, not just its own workspace. This comes with risks like data loss if something goes wrong."
        footer={
          <>
            <button
              type="button"
              className="primary-action"
              onClick={() => setConfirmUnrestricted(false)}
            >
              Cancel
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
              Turn on unrestricted
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
      aria-label="Scroll to latest"
      aria-hidden={visible ? undefined : true}
      tabIndex={visible ? undefined : -1}
      onClick={onJump}
    >
      <IconArrowDown size={16} ariaHidden />
    </button>
  );
}
