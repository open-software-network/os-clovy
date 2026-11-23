import { t } from "../i18n/translate";
import type { AgentChatPart, AgentChatTurn } from "./agent-chat-runtime";
import {
  AGENT_RUNTIME_PROTOCOL_VERSION,
  type AgentInterruptionDto,
  type AgentItemDto,
  type AgentRunDto,
  type AgentRuntimeEvent,
  type AgentSessionDto,
  type ResolveAgentInterruptionRequest,
} from "./agent-runtime-contract";
import { stripProjectContext } from "./agent-project-context";

export type AgentRuntimeProjection = {
  session?: AgentSessionDto;
  run?: AgentRunDto;
  items: AgentItemDto[];
  lastSequenceByRun: Record<string, number>;
  processedEventIds: Set<string>;
  /** Provenance of live diagnostics, not a second transcript. */
  diagnosticEventIds: ReadonlyMap<string, string>;
  /** Terminal authority retained across stale snapshots and late CLI starts. */
  terminalRunKeys: Set<string>;
};

export function createAgentRuntimeProjection(
  input: { session?: AgentSessionDto; run?: AgentRunDto; items?: AgentItemDto[] } = {},
): AgentRuntimeProjection {
  return {
    session: input.session,
    run: input.run,
    items: [...(input.items ?? [])].sort(compareAgentItems),
    lastSequenceByRun: {},
    processedEventIds: new Set(),
    diagnosticEventIds: new Map(),
    terminalRunKeys: new Set([
      ...(input.run && isTerminalRun(input.run) ? [runKey(input.run.sessionId, input.run.id)] : []),
      ...(input.items ?? [])
        .filter((item) => item.kind === "tool_call" && item.status === "interrupted")
        .map((item) => runKey(item.sessionId, item.runId)),
    ]),
  };
}

export function reconcileAgentInterruptionResolution(
  items: AgentItemDto[],
  request: ResolveAgentInterruptionRequest,
): AgentItemDto[] {
  return items.map((item) => {
    if (
      item.kind !== "interruption" ||
      item.runId !== request.runId ||
      item.interruption.id !== request.interruptionId
    ) {
      return item;
    }

    const interruption = item.interruption;
    const resolution = request.resolution;
    if (interruption.kind === "approval" && resolution.kind === "approval") {
      return {
        ...item,
        interruption: {
          ...interruption,
          status: "resolved",
          // The approval broker grants nothing beyond this call: a CLI
          // approval for the session or always is approved once.
          resolution:
            interruption.engine === "cli" && resolution.choice !== "deny"
              ? "once"
              : resolution.choice,
        },
      };
    }
    if (interruption.kind === "clarification" && resolution.kind === "clarification") {
      return {
        ...item,
        interruption: {
          ...interruption,
          status: "resolved",
          answer: resolution.answer,
        },
      };
    }
    if (interruption.kind === "secret" && resolution.kind === "secret") {
      return {
        ...item,
        interruption: {
          ...interruption,
          status: "resolved",
        },
      };
    }
    return item;
  });
}

/**
 * Hydrates a persisted transcript without dropping deltas that arrived while
 * the snapshot request was in flight. The read-start event IDs distinguish
 * newly observed diagnostics from older omitted items. Session identity gates
 * all live state; existing tool settlements and active text retain their rules.
 */
export function mergeAgentRuntimeSnapshot(
  current: AgentRuntimeProjection,
  input: { session: AgentSessionDto; run?: AgentRunDto; items: AgentItemDto[] },
  readStartedEventIds: ReadonlySet<string>,
): AgentRuntimeProjection {
  if (
    current.session?.id === input.session.id &&
    input.run &&
    !isTerminalRun(input.run) &&
    current.terminalRunKeys.has(runKey(input.run.sessionId, input.run.id)) &&
    current.run &&
    current.run.id !== input.run.id
  ) {
    // This response still calls a known terminal run active; it predates Stop
    // and cannot replace the newer run accepted while hydration was in flight.
    return current;
  }
  const snapshot = createAgentRuntimeProjection(input);
  if (current.session?.id === input.session.id) {
    snapshot.terminalRunKeys = new Set([...snapshot.terminalRunKeys, ...current.terminalRunKeys]);
    snapshot.processedEventIds = new Set(current.processedEventIds);
    snapshot.lastSequenceByRun = { ...current.lastSequenceByRun };
    snapshot.diagnosticEventIds = new Map(
      [...current.diagnosticEventIds].filter(([id]) =>
        snapshot.items.some((item) => item.id === id),
      ),
    );

    // A request begun before Stop may return running calls after its terminal
    // event. Keep that exact call's settlement and any genuinely observed result.
    for (const liveItem of current.items) {
      if (liveItem.sessionId !== input.session.id) continue;
      if (liveItem.kind === "tool_call" && liveItem.status === "interrupted") {
        const persisted = snapshot.items.find(
          (item) => item.kind === "tool_call" && sameToolCall(item, liveItem),
        );
        if (!persisted) snapshot.items = upsertItem(snapshot.items, liveItem);
        else if (persisted.kind === "tool_call" && persisted.status === "running") {
          snapshot.items = upsertItem(snapshot.items, { ...persisted, status: "interrupted" });
        }
      } else if (
        liveItem.kind === "tool_result" &&
        current.terminalRunKeys.has(runKey(liveItem.sessionId, liveItem.runId)) &&
        !snapshot.items.some((item) => item.kind === "tool_result" && sameToolCall(item, liveItem))
      ) {
        snapshot.items = upsertItem(snapshot.items, liveItem);
      }
    }
  }
  // During first hydration the selected session is known by the consumer,
  // while its full session DTO is still pending. Only event-backed cleanup
  // diagnostics can cross that gap, scoped by each item's session identity.
  if (!current.session || current.session.id === input.session.id) {
    // Carry only an actual diagnostic event received after this read began.
    // An older omitted item is authoritative deletion/compaction, not a delta.
    for (const item of current.items) {
      const eventId = current.diagnosticEventIds.get(item.id);
      if (
        item.sessionId === input.session.id &&
        item.kind === "error" &&
        item.code === "agent_workspace_mcp_cleanup" &&
        eventId &&
        current.processedEventIds.has(eventId) &&
        !readStartedEventIds.has(eventId)
      ) {
        snapshot.items = upsertItem(snapshot.items, item);
        snapshot.diagnosticEventIds = new Map(snapshot.diagnosticEventIds).set(item.id, eventId);
        snapshot.processedEventIds.add(eventId);
      }
    }
  }
  const run = input.run;
  if (!run || (run.status !== "running" && run.status !== "waiting_for_user")) return snapshot;

  const liveItems = current.items.filter(
    (item) =>
      item.sessionId === input.session.id &&
      item.runId === run.id &&
      (item.kind === "message" || item.kind === "reasoning") &&
      item.status === "streaming",
  );
  let items = snapshot.items;
  for (const liveItem of liveItems) {
    const persisted = items.find((item) => item.id === liveItem.id);
    if (
      persisted &&
      (persisted.kind === "message" || persisted.kind === "reasoning") &&
      persisted.kind === liveItem.kind
    ) {
      const text = mergeStreamText(persisted.text, liveItem.text);
      items = upsertItem(items, { ...persisted, text, status: "streaming" });
    } else {
      items = upsertItem(items, liveItem);
    }
  }

  const currentRun =
    current.run?.id === run.id && current.run.sessionId === input.session.id
      ? current.run
      : undefined;
  const currentRunFinished = currentRun && isTerminalRun(currentRun);
  return {
    ...snapshot,
    run: currentRunFinished ? currentRun : run,
    items,
    lastSequenceByRun: {
      ...snapshot.lastSequenceByRun,
      ...(current.lastSequenceByRun[run.id] === undefined
        ? {}
        : { [run.id]: current.lastSequenceByRun[run.id] }),
    },
    processedEventIds: new Set(current.processedEventIds),
  };
}

function mergeStreamText(persisted: string, live: string) {
  if (live.startsWith(persisted)) return live;
  if (persisted.startsWith(live)) return persisted;
  const maxOverlap = Math.min(persisted.length, live.length);
  for (let overlap = maxOverlap; overlap > 0; overlap -= 1) {
    if (persisted.endsWith(live.slice(0, overlap))) {
      return persisted + live.slice(overlap);
    }
  }
  return persisted + live;
}

export function applyAgentRuntimeEvent(
  projection: AgentRuntimeProjection,
  event: AgentRuntimeEvent,
): AgentRuntimeProjection {
  if (event.protocolVersion !== AGENT_RUNTIME_PROTOCOL_VERSION) {
    throw new Error(`Unsupported agent runtime protocol version: ${event.protocolVersion}`);
  }
  if (projection.processedEventIds.has(event.eventId)) return projection;
  // The approval broker writes CLI cards while the CLI turn keeps numbering
  // its own events, so its events carry no place in the run's sequence: they
  // replace the card by id and never move the run's sequence gate.
  const brokerEvent = isCliInterruptionEvent(event);
  const terminalEvent =
    event.method === "run.completed" ||
    event.method === "run.cancelled" ||
    event.method === "run.failed";
  if (
    !brokerEvent &&
    !terminalEvent &&
    event.sequence <= (projection.lastSequenceByRun[event.runId] ?? -1)
  ) {
    return projection;
  }

  const next: AgentRuntimeProjection = {
    ...projection,
    items: [...projection.items],
    lastSequenceByRun: brokerEvent
      ? projection.lastSequenceByRun
      : {
          ...projection.lastSequenceByRun,
          [event.runId]: Math.max(event.sequence, projection.lastSequenceByRun[event.runId] ?? -1),
        },
    processedEventIds: new Set(projection.processedEventIds).add(event.eventId),
    terminalRunKeys: new Set(projection.terminalRunKeys),
  };
  const eventRunKey = runKey(event.sessionId, event.runId);
  const currentRunMatches = next.run?.id === event.runId && next.run.sessionId === event.sessionId;
  if (terminalEvent) {
    next.terminalRunKeys.add(eventRunKey);
    const interruptedCallIds = new Set(event.data.interruptedCallIds ?? []);
    next.items = next.items.map((item) =>
      item.kind === "tool_call" &&
      item.status === "running" &&
      item.sessionId === event.sessionId &&
      item.runId === event.runId &&
      interruptedCallIds.has(item.callId) &&
      !next.items.some((result) => result.kind === "tool_result" && sameToolCall(result, item))
        ? { ...item, status: "interrupted" }
        : item,
    );
  }

  switch (event.method) {
    case "run.started":
      if (next.terminalRunKeys.has(eventRunKey)) break;
      next.run = {
        ...(next.run?.id === event.runId ? next.run : {}),
        id: event.runId,
        sessionId: event.sessionId,
        status: "running",
        model: event.data.model,
        startedAt: event.data.startedAt,
      };
      if (event.data.contextSummary) {
        const removedIds = new Set(event.data.removedItemIds ?? []);
        next.items = upsertItem(
          next.items.filter((item) => !removedIds.has(item.id)),
          event.data.contextSummary,
        );
      }
      break;
    case "message.delta":
      next.items = appendTextDelta(next.items, event, "message");
      break;
    case "message.completed":
      next.items = upsertItem(next.items, {
        id: event.data.itemId,
        sessionId: event.sessionId,
        runId: event.runId,
        sequence: event.data.itemSequence,
        createdAt: event.data.createdAt,
        kind: "message",
        role: event.data.role,
        text: event.data.text,
        status: "complete",
      });
      break;
    case "reasoning.delta":
      next.items = appendTextDelta(next.items, event, "reasoning");
      break;
    case "steering.consumed":
      next.items = upsertItem(next.items, {
        id: event.data.itemId,
        sessionId: event.sessionId,
        runId: event.runId,
        sequence: event.data.itemSequence,
        createdAt: event.data.createdAt,
        kind: "steering",
        text: event.data.text,
      });
      break;
    case "run.diagnostic":
      next.diagnosticEventIds = new Map(next.diagnosticEventIds).set(
        event.data.itemId,
        event.eventId,
      );
      next.items = upsertItem(next.items, {
        id: event.data.itemId,
        sessionId: event.sessionId,
        runId: event.runId,
        sequence: event.data.itemSequence,
        createdAt: event.data.createdAt,
        kind: "error",
        message: event.data.message,
        category: event.data.category,
        code: event.data.code,
        retryable: event.data.retryable,
      });
      break;
    case "tool.started":
      if (next.terminalRunKeys.has(eventRunKey)) break;
      next.items = upsertItem(next.items, {
        id: event.data.itemId,
        sessionId: event.sessionId,
        runId: event.runId,
        sequence: event.data.itemSequence,
        createdAt: event.data.createdAt,
        kind: "tool_call",
        callId: event.data.callId,
        name: event.data.name,
        arguments: event.data.arguments,
        status: "running",
      });
      break;
    case "tool.completed":
    case "tool.failed": {
      const failed = event.method === "tool.failed";
      next.items = next.items.map((item) =>
        item.kind === "tool_call" &&
        item.sessionId === event.sessionId &&
        item.runId === event.runId &&
        item.callId === event.data.callId
          ? { ...item, status: failed ? "failed" : "complete" }
          : item,
      );
      next.items = upsertItem(next.items, {
        id: event.data.itemId,
        sessionId: event.sessionId,
        runId: event.runId,
        sequence: event.data.itemSequence,
        createdAt: event.data.createdAt,
        kind: "tool_result",
        callId: event.data.callId,
        name: event.data.name,
        output: failed ? event.data.error : event.data.output,
        isError: failed,
      });
      break;
    }
    case "interruption.requested":
    case "interruption.updated": {
      if (next.terminalRunKeys.has(eventRunKey) && event.data.interruption.status === "pending") {
        break;
      }
      const existing = next.items.find(
        (item) =>
          item.kind === "interruption" &&
          item.runId === event.runId &&
          item.interruption.id === event.data.interruption.id,
      );
      next.items = upsertItem(
        next.items.filter((item) => item !== existing),
        {
          id: event.data.itemId,
          sessionId: event.sessionId,
          runId: event.runId,
          sequence: event.data.itemSequence,
          createdAt: event.data.interruption.createdAt,
          kind: "interruption",
          interruption: event.data.interruption,
        },
      );
      if (next.run && currentRunMatches && !next.terminalRunKeys.has(eventRunKey)) {
        const waiting = next.items.some(
          (item) =>
            item.kind === "interruption" &&
            item.runId === event.runId &&
            item.interruption.status === "pending",
        );
        if (waiting) next.run = { ...next.run, status: "waiting_for_user" };
        else if (next.run.status === "waiting_for_user")
          next.run = { ...next.run, status: "running" };
      }
      break;
    }
    case "usage.updated":
      if (next.run && currentRunMatches) next.run = { ...next.run, usage: event.data };
      break;
    case "run.completed":
    case "run.cancelled":
      if (next.run && currentRunMatches && !isTerminalRun(next.run)) {
        next.run = {
          ...next.run,
          status: event.method === "run.completed" ? "completed" : "cancelled",
          completedAt: event.data.completedAt,
        };
      }
      break;
    case "run.failed":
      if (next.run && currentRunMatches && !isTerminalRun(next.run)) {
        next.run = {
          ...next.run,
          status: "failed",
          completedAt: event.data.completedAt,
          error: event.data.message,
        };
      }
      next.items = upsertItem(next.items, {
        id: `error:${event.eventId}`,
        sessionId: event.sessionId,
        runId: event.runId,
        sequence: event.data.itemSequence,
        createdAt: event.data.completedAt,
        kind: "error",
        message: event.data.message,
        category: event.data.category,
        code: event.data.code,
        retryable: event.data.retryable,
      });
      break;
  }

  return next;
}

export function agentItemsToChatTurns(items: AgentItemDto[]): AgentChatTurn[] {
  const orderedItems = [...items].sort(compareAgentItems);
  const settledToolCalls = new Set(
    orderedItems
      .filter((item) => item.kind === "tool_result")
      .map((item) => toolCallKey(item.sessionId, item.runId, item.callId)),
  );
  const runIdsByItemId = new Map(orderedItems.map((item) => [item.id, item.runId]));

  const turns = orderedItems
    .filter(
      (item) =>
        item.kind !== "tool_call" ||
        !settledToolCalls.has(toolCallKey(item.sessionId, item.runId, item.callId)),
    )
    .map((item): AgentChatTurn => {
      const base = {
        id: item.id,
        createdAt: item.createdAt,
        status: itemIsRunning(item) ? ("running" as const) : ("complete" as const),
      };
      switch (item.kind) {
        case "message":
          return {
            ...base,
            role: item.role,
            parts: [
              ...((item.attachments ?? []).map(
                (artifact): AgentChatPart => ({
                  type: "attachment",
                  name: artifact.name,
                  path: artifact.path,
                  kind: artifact.mimeType?.startsWith("image/") ? "image" : "file",
                }),
              ) ?? []),
              {
                type: "text",
                text: item.role === "user" ? stripProjectContext(item.text) : item.text,
                status: base.status,
              },
            ],
          };
        case "reasoning":
          return {
            ...base,
            role: "assistant",
            parts: [{ type: "reasoning", text: item.text, status: base.status }],
          };
        case "steering":
          return {
            ...base,
            role: "system",
            parts: [{ type: "steering", text: `Steering: ${item.text}` }],
          };
        case "context_summary":
          return {
            ...base,
            role: "system",
            parts: [
              {
                type: "context",
                text: item.text,
                preview: item.text.slice(0, 160),
                status: "complete",
              },
            ],
          };
        case "tool_call":
          return {
            ...base,
            role: "assistant",
            parts: [
              {
                type: "tool",
                id: item.callId,
                name: item.name,
                text: readableValue(item.arguments),
                status: item.status,
              },
            ],
          };
        case "tool_result": {
          const image = item.isError ? undefined : generatedImagePart(item.output);
          if (image) {
            return {
              ...base,
              role: "assistant",
              parts: [
                {
                  type: "tool",
                  id: item.callId,
                  name: item.name,
                  text: readableValue(item.output),
                  status: "complete",
                },
                image,
              ],
            };
          }
          const video = item.isError ? undefined : generatedVideoPart(item.output);
          if (video) {
            return {
              ...base,
              role: "assistant",
              parts: [
                {
                  type: "tool",
                  id: item.callId,
                  name: item.name,
                  text: readableValue(item.output),
                  status: "complete",
                },
                video,
              ],
            };
          }
          return {
            ...base,
            role: "assistant",
            parts: [
              {
                type: "tool",
                id: item.callId,
                name: item.name,
                text: readableValue(item.output),
                status: item.isError ? "failed" : "complete",
              },
            ],
          };
        }
        case "interruption":
          return {
            ...base,
            role: "assistant",
            parts: [interruptionToPart(item.interruption, item.runId)],
          };
        case "error":
          if (item.category === "context") {
            return {
              ...base,
              role: "system",
              parts: [{ type: "notice", kind: "context-overflow", text: item.message }],
            };
          }
          if (item.category === "credits") {
            return {
              ...base,
              role: "system",
              parts: [
                {
                  type: "notice",
                  kind: "runtime",
                  text: t("chat.notice.runtimeStopped"),
                  retryable: false,
                },
              ],
            };
          }
          return {
            ...base,
            role: "system",
            parts: [
              {
                type: "notice",
                kind:
                  item.category === "tool"
                    ? "tool"
                    : item.category === "runtime"
                      ? "runtime"
                      : "upstream-provider",
                text:
                  item.code === "agent_workspace_mcp_cleanup"
                    ? item.message
                    : t("lib.upstream.noticeBody"),
                ...(item.code === "agent_workspace_mcp_cleanup"
                  ? { diagnostic: item.message }
                  : {}),
                retryable: item.retryable,
              },
            ],
          };
        default:
          return assertNever(item);
      }
    });

  return coalesceAgentActivityTurns(
    turns.map((turn) => ({ runId: runIdsByItemId.get(turn.id), turn })),
  );
}

type AgentRunTurn = {
  runId?: string;
  turn: AgentChatTurn;
};

/**
 * Presents consecutive reasoning, tool, and interruption items from one agent
 * run as a single activity cluster. The latest item owns the cluster identity
 * and timestamp; when a visible assistant output follows, it owns the combined
 * turn instead. User/system turns, visible outputs, and run changes remain
 * boundaries.
 */
function coalesceAgentActivityTurns(items: AgentRunTurn[]): AgentChatTurn[] {
  const turns: AgentChatTurn[] = [];
  let pending: AgentRunTurn | undefined;

  const flushPending = () => {
    if (!pending) return;
    turns.push(pending.turn);
    pending = undefined;
  };

  for (const item of items) {
    if (isInternalAgentActivity(item.turn)) {
      if (pending && pending.runId === item.runId) {
        pending = {
          ...item,
          turn: { ...item.turn, parts: [...pending.turn.parts, ...item.turn.parts] },
        };
      } else {
        flushPending();
        pending = item;
      }
      continue;
    }

    if (pending && item.turn.role === "assistant" && pending.runId === item.runId) {
      turns.push({ ...item.turn, parts: [...pending.turn.parts, ...item.turn.parts] });
      pending = undefined;
      continue;
    }

    flushPending();
    turns.push(item.turn);
  }

  flushPending();
  return turns;
}

function isInternalAgentActivity(turn: AgentChatTurn) {
  return (
    turn.role === "assistant" &&
    turn.parts.length > 0 &&
    turn.parts.every(
      (part) =>
        part.type === "reasoning" ||
        part.type === "tool" ||
        part.type === "approval" ||
        part.type === "clarify" ||
        part.type === "sudo" ||
        part.type === "secret",
    )
  );
}

function generatedImagePart(output: unknown): AgentChatPart | undefined {
  if (!output || typeof output !== "object") return undefined;
  const value = output as Record<string, unknown>;
  if (value.mediaType !== "image" || typeof value.dataUrl !== "string") return undefined;
  return {
    type: "image",
    status: "complete",
    prompt: typeof value.prompt === "string" ? value.prompt : "",
    model: typeof value.model === "string" ? value.model : undefined,
    path: typeof value.path === "string" ? value.path : undefined,
    dataUrl: value.dataUrl,
    name: typeof value.name === "string" ? value.name : undefined,
  };
}

function generatedVideoPart(output: unknown): AgentChatPart | undefined {
  if (!output || typeof output !== "object") return undefined;
  const value = output as Record<string, unknown>;
  // The fal key could not be used, so the job waits on the server for the agent
  // to continue it by job id: this attempt shows why it paused, never a
  // finished video without an asset.
  if (value.status === "waitingForProviderKey" && typeof value.jobId === "string") {
    return {
      type: "video",
      status: "error",
      prompt: "",
      jobId: value.jobId,
      error: t("chat.media.videoWaitingForProviderKey"),
    };
  }
  if (value.mediaType !== "video" || typeof value.path !== "string") return undefined;
  return {
    type: "video",
    status: "complete",
    prompt: typeof value.prompt === "string" ? value.prompt : "",
    model: typeof value.model === "string" ? value.model : undefined,
    path: value.path,
    requestId: typeof value.requestId === "string" ? value.requestId : undefined,
    name: typeof value.name === "string" ? value.name : undefined,
  };
}

function runKey(sessionId: string, runId: string | undefined) {
  return JSON.stringify([sessionId, runId]);
}

function toolCallKey(sessionId: string, runId: string | undefined, callId: string) {
  return JSON.stringify([sessionId, runId, callId]);
}

function sameToolCall(
  a: Extract<AgentItemDto, { kind: "tool_call" | "tool_result" }>,
  b: Extract<AgentItemDto, { kind: "tool_call" | "tool_result" }>,
) {
  return a.sessionId === b.sessionId && a.runId === b.runId && a.callId === b.callId;
}

function isTerminalRun(run: AgentRunDto) {
  return ["completed", "cancelled", "failed", "interrupted"].includes(run.status);
}

function interruptionToPart(
  interruption: Extract<AgentItemDto, { kind: "interruption" }>["interruption"],
  persistedRunId?: string,
): AgentChatPart {
  const runId = persistedRunId ?? interruption.runId;
  if (interruption.kind === "clarification") {
    return {
      type: "clarify",
      id: interruption.id,
      runId,
      sessionId: interruption.sessionId,
      question: interruption.question,
      choices: interruption.choices,
      answer: interruption.answer,
      status: clarifyStatus(interruption),
      ...(interruption.engine === "cli" ? { engine: "cli" as const } : {}),
    };
  }
  if (interruption.kind === "secret") {
    return {
      type: "secret",
      id: interruption.id,
      runId,
      reason: interruption.reason,
      status: interruption.status === "pending" ? "pending" : "resolved",
    };
  }
  return {
    type: "approval",
    id: interruption.id,
    runId,
    sessionId: interruption.sessionId,
    command: interruption.command ?? interruption.toolName,
    description: interruption.description || interruption.title,
    allowPermanent: interruption.allowAlways,
    choice: interruption.resolution,
    status: interruption.status,
    ...(interruption.retiredReason ? { retiredReason: interruption.retiredReason } : {}),
    ...(interruption.engine === "cli" ? { engine: "cli" as const } : {}),
  };
}

/** Native questions only resolve; a CLI question can also expire or be
 * cancelled without an answer. */
function clarifyStatus(
  interruption: Extract<AgentInterruptionDto, { kind: "clarification" }>,
): Extract<AgentChatPart, { type: "clarify" }>["status"] {
  if (interruption.status === "pending") return "pending";
  if (interruption.engine === "cli" && interruption.status !== "resolved") {
    return interruption.status;
  }
  return "resolved";
}

function isCliInterruptionEvent(event: AgentRuntimeEvent) {
  return (
    (event.method === "interruption.requested" || event.method === "interruption.updated") &&
    event.data.interruption.engine === "cli"
  );
}

function appendTextDelta(
  items: AgentItemDto[],
  event: Extract<AgentRuntimeEvent, { method: "message.delta" | "reasoning.delta" }>,
  kind: "message" | "reasoning",
) {
  const existing = items.find((item) => item.id === event.data.itemId);
  const text = existing && "text" in existing ? existing.text + event.data.delta : event.data.delta;
  return upsertItem(
    items,
    kind === "message"
      ? {
          id: event.data.itemId,
          sessionId: event.sessionId,
          runId: event.runId,
          sequence: event.data.itemSequence,
          createdAt: event.data.createdAt,
          kind,
          role: "assistant",
          text,
          status: "streaming",
        }
      : {
          id: event.data.itemId,
          sessionId: event.sessionId,
          runId: event.runId,
          sequence: event.data.itemSequence,
          createdAt: event.data.createdAt,
          kind,
          text,
          status: "streaming",
        },
  );
}

function upsertItem(items: AgentItemDto[], item: AgentItemDto) {
  const index = items.findIndex((candidate) => candidate.id === item.id);
  const next = [...items];
  if (index >= 0) next[index] = item;
  else next.push(item);
  return next.sort(compareAgentItems);
}

function compareAgentItems(a: AgentItemDto, b: AgentItemDto) {
  return a.sequence - b.sequence;
}

function itemIsRunning(item: AgentItemDto) {
  return (
    ((item.kind === "message" || item.kind === "reasoning") && item.status === "streaming") ||
    (item.kind === "tool_call" && item.status === "running")
  );
}

function readableValue(value: unknown) {
  if (typeof value === "string") return value;
  if (value === undefined) return "";
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function assertNever(value: never): never {
  throw new Error(`Unsupported agent item: ${JSON.stringify(value)}`);
}
