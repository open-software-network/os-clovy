import { IconArrowsRepeat } from "central-icons/IconArrowsRepeat";
import { IconBranch } from "central-icons/IconBranch";
import { IconChevronRightSmall } from "central-icons/IconChevronRightSmall";
import { IconConcise } from "central-icons/IconConcise";
import { useEffect, useRef, useState } from "react";
import {
  displayedComposerUserMessageText,
  stripRenderedMediaReferences,
  type AgentApprovalChoice,
  type AgentChatPart,
  type AgentChatTurn,
} from "../../../lib/agent-chat-runtime";
import {
  hasAgentCliAccessRequest,
  stripAgentCliAccessRequest,
} from "../../../lib/agent-cli-access";
import {
  hasBrowserAccessRequest,
  stripBrowserAccessRequest,
  useBrowserAccess,
} from "../../../lib/browser-access";
import { intlLocale, useT } from "../../../i18n";
import { agentFilePreview } from "../../../lib/tauri";
import { CopyStateIcon } from "../../ui/CopyStateIcon";
import { DotSpinner } from "../../DotSpinner";
import { HoverTip } from "../../ui/HoverTip";
import { relativeDate } from "../agent-workspace-helpers";
import { FileTypeIcon } from "../FileTypeIcon";
import { MarkdownContent } from "../MarkdownContent";
import { SmoothedStreamingMarkdown } from "../SmoothedStreamingMarkdown";
import {
  AgentBrowserAccessCard,
  AgentCliAccessCard,
  ApprovalPart,
  ClarifyPart,
  type AgentBrowserAccessCardProps,
  type AgentCliAccessCardProps,
} from "./AgentActionCards";
import { AgentArtifactList, type AgentArtifact } from "./AgentArtifactPanel";
import {
  SecretPart,
  SudoPart,
  TURN_ACTION_TIP_DELAY_MS,
  turnIsConcreteResponse,
} from "./TurnPresentation";
import { AgentGeneratedImage, AgentGeneratedVideo } from "./GeneratedMedia";
import {
  ContextOverflowNoticePart,
  SteeringPart,
  UpstreamProviderFailureNoticePart,
} from "./RunNotices";
import { AgentThinkingGroup, AgentToolStack } from "./ThinkingAndTools";
import type { HomeTaskHandoff } from "../home-thread";

export function AgentChatTurnRow({
  activeThinkingKey,
  approvalSubmitting,
  artifacts,
  clarifySubmitting,
  sudoSubmitting,
  secretSubmitting,
  cliAccess,
  browserAccess,
  thinkingOpen,
  onApproval,
  onClarify,
  onSudo,
  onSecret,
  onDownloadArtifact,
  onOpenArtifact,
  onDownloadImage,
  onOpenImage,
  onRetryImage,
  onDownloadVideo,
  onRetryVideo,
  onRetryUpstreamFailure,
  upstreamFailureRetryAttempted,
  upstreamFailureRetryDisabled,
  onThinkingOpenChange,
  onVisibleMarkdownChange,
  onBranch,
  branching,
  homeTaskHandoff,
  onOpenHomeTaskSession,
  onRetryHomeTask,
  homeUserRunEnd,
  turn,
}: {
  activeThinkingKey?: string;
  approvalSubmitting: Partial<Record<string, AgentApprovalChoice>>;
  artifacts?: AgentArtifact[];
  clarifySubmitting: Record<string, string>;
  sudoSubmitting: Record<string, "approve" | "deny">;
  secretSubmitting: Record<string, true>;
  /** State + handler for Clovy's in-chat Agent CLI access request card.
   * Optional so the dev gallery can render rows without the live setting. */
  cliAccess?: AgentCliAccessCardProps;
  /** State + handler for Clovy's in-chat Browser use request card. Optional
   * for the same reason. */
  browserAccess?: AgentBrowserAccessCardProps;
  thinkingOpen: (key: string) => boolean;
  onApproval: (
    part: Extract<AgentChatPart, { type: "approval" }>,
    choice: AgentApprovalChoice,
  ) => void;
  onClarify: (part: Extract<AgentChatPart, { type: "clarify" }>, answer: string) => void;
  onSudo: (part: Extract<AgentChatPart, { type: "sudo" }>, approved: boolean) => void;
  onSecret: (part: Extract<AgentChatPart, { type: "secret" }>, value: string) => void;
  onDownloadArtifact?: (artifact: AgentArtifact) => void;
  onOpenArtifact?: (artifact: AgentArtifact) => void;
  /** Save a `/image` result to disk; enlarge it in the file viewer. Optional so
   * the dev gallery can render image rows without the live bridge. */
  onDownloadImage?: (part: Extract<AgentChatPart, { type: "image" }>) => void;
  onOpenImage?: (part: Extract<AgentChatPart, { type: "image" }>) => void;
  onRetryImage?: (assistantTurnId: string, part: Extract<AgentChatPart, { type: "image" }>) => void;
  onDownloadVideo?: (part: Extract<AgentChatPart, { type: "video" }>) => void;
  onRetryVideo?: (assistantTurnId: string, part: Extract<AgentChatPart, { type: "video" }>) => void;
  onRetryUpstreamFailure?: (assistantTurnId: string) => void;
  upstreamFailureRetryAttempted?: boolean;
  upstreamFailureRetryDisabled?: boolean;
  onThinkingOpenChange: (key: string, open: boolean) => void;
  onVisibleMarkdownChange?: (visibleMarkdown: string) => void;
  onBranch?: (itemId: string) => void;
  branching?: boolean;
  homeTaskHandoff?: HomeTaskHandoff;
  onOpenHomeTaskSession?: (storedSessionId: string, title: string) => void;
  onRetryHomeTask?: (handoff: HomeTaskHandoff) => void;
  homeUserRunEnd?: boolean;
  turn: AgentChatTurn;
}) {
  const t = useT();
  const requestsBrowserAccess = turn.parts.some(
    (part) => part.type === "text" && hasBrowserAccessRequest(part.text),
  );
  const storedBrowserAccess = useBrowserAccess(
    browserAccess === undefined && requestsBrowserAccess,
  );
  const effectiveBrowserAccess = browserAccess ?? {
    enabled: storedBrowserAccess.enabled,
    submitting: storedBrowserAccess.submitting,
    error: storedBrowserAccess.error,
    onEnable: () => void storedBrowserAccess.setGrant(true),
  };
  const textParts = turn.parts.filter(
    (part): part is Extract<AgentChatPart, { type: "text" }> => part.type === "text",
  );
  const reasoningParts = turn.parts.filter(
    (part): part is Extract<AgentChatPart, { type: "reasoning" }> => part.type === "reasoning",
  );
  const toolParts = turn.parts.filter(
    (part): part is Extract<AgentChatPart, { type: "tool" }> => part.type === "tool",
  );
  // Adjacent tool rows render as one stack, keyed by the stack's first row.
  const toolStacks = new Map<string, typeof toolParts>();
  let toolStack: typeof toolParts | undefined;
  for (const part of turn.parts) {
    if (part.type !== "tool") {
      toolStack = undefined;
      continue;
    }
    if (toolStack) {
      toolStack.push(part);
      continue;
    }
    toolStack = [part];
    toolStacks.set(part.id, toolStack);
  }
  const reasoningGroups = new Map<
    Extract<AgentChatPart, { type: "reasoning" }>,
    { parts: typeof reasoningParts; completedKey: string; startIndex: number }
  >();
  let reasoningGroup:
    | { parts: typeof reasoningParts; completedKey: string; startIndex: number }
    | undefined;
  for (const [index, part] of turn.parts.entries()) {
    if (part.type !== "reasoning") {
      reasoningGroup = undefined;
      continue;
    }
    if (reasoningGroup) {
      reasoningGroup.parts.push(part);
      continue;
    }
    reasoningGroup = {
      parts: [part],
      completedKey:
        reasoningGroups.size === 0
          ? `turn:${turn.id}:thinking`
          : `turn:${turn.id}:thinking:${index}`,
      startIndex: index,
    };
    reasoningGroups.set(part, reasoningGroup);
  }
  // The disclosure owns internal reasoning only. Tool/action rows stay visible
  // outside it so users can see what Clovy is doing without expanding Thought.
  const thinkingRunning = reasoningParts.some((part) => part.status === "running");
  const runningReasoningGroup = [...reasoningGroups.values()].find((group) =>
    group.parts.some((part) => part.status === "running"),
  );
  const lastRunningCompletedKeyRef = useRef(
    runningReasoningGroup?.completedKey ?? `turn:${turn.id}:thinking`,
  );
  if (runningReasoningGroup) {
    lastRunningCompletedKeyRef.current = runningReasoningGroup.completedKey;
  }
  const completedThinkingKey = lastRunningCompletedKeyRef.current;
  const wasThinkingRunningRef = useRef(thinkingRunning);
  const carriedOpen =
    !thinkingRunning &&
    wasThinkingRunningRef.current &&
    activeThinkingKey !== undefined &&
    thinkingOpen(activeThinkingKey);
  const [copied, setCopied] = useState(false);
  const [dismissedAccessRequestCards, setDismissedAccessRequestCards] = useState<
    ReadonlySet<string>
  >(() => new Set());
  const copyResetTimerRef = useRef<number | undefined>(undefined);

  function renderReasoningGroup(part: Extract<AgentChatPart, { type: "reasoning" }>) {
    const group = reasoningGroups.get(part);
    if (!group) return null;
    const running = group.parts.some((reasoningPart) => reasoningPart.status === "running");
    const key = running && activeThinkingKey ? activeThinkingKey : group.completedKey;
    return (
      <AgentThinkingGroup
        key={`${turn.id}:reasoning:${group.startIndex}`}
        reasoning={group.parts}
        running={running}
        open={thinkingOpen(key) || (carriedOpen && group.completedKey === completedThinkingKey)}
        onOpenChange={(open) => onThinkingOpenChange(key, open)}
      />
    );
  }

  useEffect(() => {
    const wasRunning = wasThinkingRunningRef.current;
    wasThinkingRunningRef.current = thinkingRunning;
    if (
      !wasRunning ||
      thinkingRunning ||
      activeThinkingKey === undefined ||
      reasoningParts.length === 0 ||
      !thinkingOpen(activeThinkingKey)
    ) {
      return;
    }
    onThinkingOpenChange(completedThinkingKey, true);
    onThinkingOpenChange(activeThinkingKey, false);
  }, [
    activeThinkingKey,
    completedThinkingKey,
    onThinkingOpenChange,
    reasoningParts.length,
    thinkingOpen,
    thinkingRunning,
    toolParts.length,
  ]);

  useEffect(
    () => () => {
      if (copyResetTimerRef.current !== undefined) {
        window.clearTimeout(copyResetTimerRef.current);
      }
    },
    [],
  );

  const contextParts = turn.parts.filter(
    (part): part is Extract<AgentChatPart, { type: "context" }> => part.type === "context",
  );
  const attachmentParts = turn.parts.filter(
    (part): part is Extract<AgentChatPart, { type: "attachment" }> => part.type === "attachment",
  );
  const nonTextParts = turn.parts.filter((part) => part.type !== "text");
  const concreteResponse = turnIsConcreteResponse(turn);
  const copyText = copyableTextForTurn(turn);
  const hasActionCard = turn.parts.some((part, index) => {
    if (
      part.type === "approval" ||
      part.type === "clarify" ||
      part.type === "sudo" ||
      part.type === "secret"
    ) {
      return part.status === "pending";
    }
    if (part.type !== "text") return false;
    return (
      (hasAgentCliAccessRequest(part.text) &&
        cliAccess?.enabled !== true &&
        !dismissedAccessRequestCards.has(accessRequestCardKey(turn.id, index, "cli"))) ||
      (hasBrowserAccessRequest(part.text) &&
        effectiveBrowserAccess.enabled !== true &&
        !dismissedAccessRequestCards.has(accessRequestCardKey(turn.id, index, "browser")))
    );
  });

  function dismissAccessRequestCard(index: number, kind: AccessRequestCardKind) {
    const key = accessRequestCardKey(turn.id, index, kind);
    setDismissedAccessRequestCards((current) => {
      if (current.has(key)) return current;
      const next = new Set(current);
      next.add(key);
      return next;
    });
  }

  async function copyTurn() {
    if (!copyText) return;
    try {
      if (!navigator.clipboard?.writeText) return;
      await navigator.clipboard.writeText(copyText);
      setCopied(true);
      if (copyResetTimerRef.current !== undefined) {
        window.clearTimeout(copyResetTimerRef.current);
      }
      copyResetTimerRef.current = window.setTimeout(() => {
        setCopied(false);
        copyResetTimerRef.current = undefined;
      }, 1600);
    } catch {
      // Clipboard can fail in restricted contexts; leave the transcript alone.
    }
  }

  const copyAction = copyText ? (
    <HoverTip
      compact
      width={104}
      delay={TURN_ACTION_TIP_DELAY_MS}
      tip={copied ? t("common.copied") : t("chat.turn.copyMessage")}
      forceOpen={copied}
      className="agent-turn-action-tip"
    >
      <button
        type="button"
        className="agent-turn-action"
        aria-label={copied ? t("chat.turn.copiedMessage") : t("chat.turn.copyMessage")}
        data-copied={copied ? "true" : undefined}
        onClick={() => void copyTurn()}
      >
        <CopyStateIcon copied={copied} />
      </button>
    </HoverTip>
  ) : null;
  const branchAction =
    concreteResponse && onBranch ? (
      <HoverTip
        compact
        width={136}
        delay={TURN_ACTION_TIP_DELAY_MS}
        tip={t("chat.turn.branchFromHere")}
        className="agent-turn-action-tip"
      >
        <button
          type="button"
          className="agent-turn-action"
          aria-label={branching ? t("chat.turn.creatingBranch") : t("chat.turn.branchFromHere")}
          disabled={branching}
          onClick={() => onBranch(turn.id)}
        >
          <IconBranch size={14} aria-hidden />
        </button>
      </HoverTip>
    ) : null;
  // Timestamp for the row. relativeDate returns "" for an unparseable value, so
  // we only render the <time> when there's a real date to show.
  const timestampLabel = relativeDate(turn.createdAt);
  const timestampAction = timestampLabel ? (
    <HoverTip
      compact
      width={200}
      delay={TURN_ACTION_TIP_DELAY_MS}
      tip={new Date(turn.createdAt).toLocaleString(intlLocale())}
      className="agent-turn-action-tip"
    >
      <time className="agent-turn-timestamp" dateTime={turn.createdAt}>
        {timestampLabel}
      </time>
    </HoverTip>
  ) : null;
  const turnActions =
    concreteResponse && (copyAction || branchAction || timestampAction) ? (
      <div className="agent-turn-actions">
        <div className="agent-turn-actions-inner">
          {/* The timestamp sits on the outer/far side of the row: before the
           * icons on right-aligned user turns, after them on left-aligned
           * assistant turns, so the icons always stay nearest the message. */}
          {turn.role === "user" ? timestampAction : null}
          {copyAction}
          {branchAction}
          {turn.role === "user" ? null : timestampAction}
        </div>
      </div>
    ) : null;

  if (contextParts.length && turn.parts.every((part) => part.type === "context")) {
    return (
      <>
        {contextParts.map((part, index) => (
          <ContextCompactionPart key={`${turn.id}:context:${index}`} part={part} />
        ))}
      </>
    );
  }

  if (homeTaskHandoff) {
    const copy =
      homeTaskHandoff.status === "failed"
        ? t("chat.homeTask.failed", { title: homeTaskHandoff.title })
        : homeTaskHandoff.status === "starting"
          ? t("chat.homeTask.starting", { title: homeTaskHandoff.title })
          : t("chat.homeTask.created", { title: homeTaskHandoff.title });
    return (
      <article className="agent-assistant-turn" data-status={homeTaskHandoff.status}>
        <div className="agent-assistant-turn-body">
          <div className="agent-home-task-message" data-status={homeTaskHandoff.status}>
            <span>{copy}</span>
            {homeTaskHandoff.status === "failed" ? (
              onRetryHomeTask ? (
                <button type="button" onClick={() => onRetryHomeTask(homeTaskHandoff)}>
                  {t("common.tryAgain")}
                </button>
              ) : null
            ) : homeTaskHandoff.storedSessionId ? (
              <button
                type="button"
                onClick={() =>
                  onOpenHomeTaskSession?.(
                    homeTaskHandoff.storedSessionId as string,
                    homeTaskHandoff.title,
                  )
                }
              >
                {t("chat.homeTask.openSession")}
                <IconChevronRightSmall size={14} aria-hidden />
              </button>
            ) : (
              <span
                className="agent-home-task-pending"
                role="status"
                aria-label={t("chat.homeTask.creatingSession")}
              >
                <DotSpinner />
              </span>
            )}
          </div>
        </div>
      </article>
    );
  }

  if (turn.role === "user") {
    return (
      <article
        className="agent-user-turn"
        data-scheduled-run={turn.isScheduledRun ? "true" : undefined}
        data-user-run-end={homeUserRunEnd ? "true" : undefined}
      >
        {turn.isScheduledRun ? (
          <span className="agent-user-turn-eyebrow">
            <IconArrowsRepeat size={12} aria-hidden />
            {t("chat.turn.scheduledRoutineRun")}
          </span>
        ) : null}
        <div className="agent-user-turn-body">
          {textParts.map((part, index) => {
            const markdown = displayedComposerUserMessageText(part.text);
            return markdown ? (
              <MarkdownContent
                key={`${turn.id}:text:${index}`}
                // Issue-report sessions open with the wrapped investigation
                // prompt; the transcript shows only what the user typed.
                markdown={markdown}
              />
            ) : null;
          })}
          {attachmentParts.length ? (
            <AgentUserAttachmentList attachments={attachmentParts} onOpen={onOpenArtifact} />
          ) : null}
        </div>
        {turnActions}
      </article>
    );
  }

  return (
    <article className="agent-assistant-turn" data-status={turn.status}>
      <div
        className={`agent-assistant-turn-body${
          hasActionCard ? " agent-assistant-turn-body-action-card" : ""
        }`}
      >
        {turn.parts.map((part, index) =>
          part.type === "reasoning" ? (
            renderReasoningGroup(part)
          ) : part.type === "tool" ? (
            toolStacks.has(part.id) ? (
              <AgentToolStack
                key={`${turn.id}:tools:${part.id}`}
                parts={toolStacks.get(part.id) ?? []}
              />
            ) : null
          ) : part.type === "text" ? (
            hasAgentCliAccessRequest(part.text) || hasBrowserAccessRequest(part.text) ? (
              // Clovy's soul emits a literal token to request the Agent CLI
              // access or Browser use setting; each token renders as an
              // approval card, never as text. A reply carrying both tokens
              // gets both cards.
              <div key={`${turn.id}:text:${index}`}>
                {stripBrowserAccessRequest(stripAgentCliAccessRequest(part.text)) ? (
                  <MarkdownContent
                    markdown={stripBrowserAccessRequest(stripAgentCliAccessRequest(part.text))}
                    repairProse
                  />
                ) : null}
                {hasAgentCliAccessRequest(part.text) ? (
                  <AgentCliAccessCard
                    cliAccess={cliAccess}
                    dismissed={dismissedAccessRequestCards.has(
                      accessRequestCardKey(turn.id, index, "cli"),
                    )}
                    onDismiss={() => dismissAccessRequestCard(index, "cli")}
                  />
                ) : null}
                {hasBrowserAccessRequest(part.text) ? (
                  <AgentBrowserAccessCard
                    browserAccess={effectiveBrowserAccess}
                    dismissed={dismissedAccessRequestCards.has(
                      accessRequestCardKey(turn.id, index, "browser"),
                    )}
                    onDismiss={() => dismissAccessRequestCard(index, "browser")}
                  />
                ) : null}
              </div>
            ) : (
              <div key={`${turn.id}:text:${index}`}>
                {/* A part can retain raw MEDIA deltas while streaming or when
                    a terminal/error event arrives without message.complete.
                    Those transport references never belong in assistant prose. */}
                <SmoothedStreamingMarkdown
                  markdown={stripRenderedMediaReferences(part.text, part.status === "running")}
                  running={part.status === "running"}
                  repairProse
                  onVisibleMarkdownChange={onVisibleMarkdownChange}
                />
              </div>
            )
          ) : part.type === "context" ? (
            <ContextCompactionPart key={`${turn.id}:context:${index}`} part={part} />
          ) : part.type === "approval" ? (
            <ApprovalPart
              key={`${turn.id}:approval:${part.id}`}
              part={part}
              submitting={approvalSubmitting[part.id]}
              onApproval={onApproval}
            />
          ) : part.type === "clarify" ? (
            <ClarifyPart
              key={`${turn.id}:clarify:${part.id}`}
              part={part}
              submitting={clarifySubmitting[part.id]}
              onClarify={onClarify}
            />
          ) : part.type === "sudo" ? (
            <SudoPart
              key={`${turn.id}:sudo:${part.id}`}
              part={part}
              submitting={sudoSubmitting[part.id]}
              onSudo={onSudo}
            />
          ) : part.type === "secret" ? (
            <SecretPart
              key={`${turn.id}:secret:${part.id}`}
              part={part}
              submitting={secretSubmitting[part.id]}
              onSecret={onSecret}
            />
          ) : part.type === "notice" ? (
            part.kind === "context-overflow" ? (
              <ContextOverflowNoticePart key={`${turn.id}:notice:${index}`} />
            ) : part.kind === "upstream-provider" ||
              part.kind === "tool" ||
              part.kind === "runtime" ? (
              <UpstreamProviderFailureNoticePart
                key={`${turn.id}:notice:${index}`}
                kind={part.kind}
                diagnostic={part.diagnostic}
                attempted={upstreamFailureRetryAttempted}
                disabled={upstreamFailureRetryDisabled}
                onRetry={
                  part.retryable !== false && onRetryUpstreamFailure
                    ? () => onRetryUpstreamFailure(turn.id)
                    : undefined
                }
              />
            ) : null
          ) : part.type === "steering" ? (
            <SteeringPart key={`${turn.id}:steering:${index}`} part={part} />
          ) : part.type === "image" ? (
            <AgentGeneratedImage
              key={`${turn.id}:image:${index}`}
              part={part}
              onOpen={onOpenImage}
              onDownload={onDownloadImage}
              onRetry={onRetryImage ? () => onRetryImage(turn.id, part) : undefined}
            />
          ) : part.type === "video" ? (
            <AgentGeneratedVideo
              key={`${turn.id}:video:${index}`}
              part={part}
              onDownload={onDownloadVideo}
              onRetry={onRetryVideo ? () => onRetryVideo(turn.id, part) : undefined}
            />
          ) : null,
        )}
        <AgentArtifactList
          artifacts={artifacts ?? []}
          onDownload={onDownloadArtifact}
          onOpen={onOpenArtifact}
        />
        {textParts.length === 0 && nonTextParts.length === 0 ? (
          <p className="agent-assistant-empty">
            <span className="text-shimmer shimmer">{t("chat.turn.thinkingEllipsis")}</span>
          </p>
        ) : (
          // No actions on an empty/in-flight turn. There is nothing useful to
          // copy or fork from yet.
          turnActions
        )}
      </div>
    </article>
  );
}

type AccessRequestCardKind = "browser" | "cli";

function accessRequestCardKey(turnId: string, partIndex: number, kind: AccessRequestCardKind) {
  return `${turnId}:${partIndex}:${kind}`;
}

function AgentUserAttachmentList({
  attachments,
  onOpen,
}: {
  attachments: Extract<AgentChatPart, { type: "attachment" }>[];
  onOpen?: (artifact: AgentArtifact) => void;
}) {
  const t = useT();
  return (
    <div className="agent-user-attachments" role="group" aria-label={t("chat.turn.attachments")}>
      {attachments.map((attachment) => (
        <AgentUserAttachment key={attachment.path} attachment={attachment} onOpen={onOpen} />
      ))}
    </div>
  );
}

function AgentUserAttachment({
  attachment,
  onOpen,
}: {
  attachment: Extract<AgentChatPart, { type: "attachment" }>;
  onOpen?: (artifact: AgentArtifact) => void;
}) {
  const t = useT();
  const [previewDataUrl, setPreviewDataUrl] = useState<string | null | undefined>(undefined);

  useEffect(() => {
    if (attachment.kind !== "image") return;
    let cancelled = false;
    agentFilePreview(attachment.path)
      .then((dataUrl) => {
        if (!cancelled) setPreviewDataUrl(dataUrl);
      })
      .catch(() => {
        if (!cancelled) setPreviewDataUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [attachment.kind, attachment.path]);

  const artifact: AgentArtifact = {
    name: attachment.name,
    path: attachment.path,
    rootLabel: t("chat.turn.workspace"),
  };
  const content =
    attachment.kind === "image" ? (
      previewDataUrl ? (
        <img src={previewDataUrl} alt="" aria-hidden="true" draggable={false} />
      ) : previewDataUrl === undefined ? (
        <span className="agent-user-attachment-loading text-shimmer shimmer">
          {t("chat.media.loadingImage")}
        </span>
      ) : (
        <>
          <span className="agent-attachment-file-icon" aria-hidden="true">
            <FileTypeIcon name={attachment.name} size={18} />
          </span>
          <span className="agent-user-attachment-name">{attachment.name}</span>
        </>
      )
    ) : (
      <>
        <span className="agent-attachment-file-icon" aria-hidden="true">
          <FileTypeIcon name={attachment.name} size={18} />
        </span>
        <span className="agent-user-attachment-name">{attachment.name}</span>
      </>
    );

  return onOpen ? (
    <button
      type="button"
      className="agent-user-attachment"
      data-kind={attachment.kind}
      aria-label={t("chat.openNamed", { name: attachment.name })}
      title={attachment.name}
      onClick={() => onOpen(artifact)}
    >
      {content}
    </button>
  ) : (
    <div className="agent-user-attachment" data-kind={attachment.kind} title={attachment.name}>
      {content}
    </div>
  );
}

export function copyableTextForTurn(turn: AgentChatTurn): string {
  if (turn.role === "user") return userPromptTextForTurn(turn);
  if (turn.role !== "assistant") return "";
  return turn.parts
    .filter((part): part is Extract<AgentChatPart, { type: "text" }> => part.type === "text")
    .map((part) => stripBrowserAccessRequest(stripAgentCliAccessRequest(part.text)).trim())
    .filter(Boolean)
    .join("\n\n")
    .trim();
}

function userPromptTextForTurn(turn: AgentChatTurn): string {
  return turn.parts
    .filter((part): part is Extract<AgentChatPart, { type: "text" }> => part.type === "text")
    .map((part) => displayedComposerUserMessageText(part.text).trim())
    .filter(Boolean)
    .join("\n\n")
    .trim();
}

function ContextCompactionPart({ part }: { part: Extract<AgentChatPart, { type: "context" }> }) {
  const t = useT();
  return (
    <details className="agent-context-summary">
      <summary>
        {/* Same hover affordance as the tool rows: the glyph cross-fades to a
         * plain-text "+"/"−" so the row reads as one quiet, expandable line.
         * IconConcise (thinned via CSS) marks the squeeze of compaction. No
         * timestamp: this is a system marker, not a concrete message. */}
        <span className="agent-tool-icon">
          <IconConcise size={15} className="agent-context-icon-glyph" />
          <span className="agent-tool-icon-expand">+</span>
          <span className="agent-tool-icon-minimize">−</span>
        </span>
        <span className="agent-context-label">{t("chat.turn.contextCompacted")}</span>
      </summary>
      <MarkdownContent markdown={part.text} />
    </details>
  );
}
