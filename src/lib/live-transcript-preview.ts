import type { DiscardedTranscriptSpanDto, LiveTranscriptEventDto, TranscriptDto } from "./tauri";

const LIVE_TRANSCRIPT_COHERENCE_GAP_MS = 500;
const LIVE_TRANSCRIPT_WINDOW_MS = 2 * 60 * 60 * 1000;
export const LIVE_TRANSCRIPT_MAX_EVENTS_PER_SESSION = 2048;
const LIVE_TRANSCRIPT_MAX_EVENTS_TOTAL = 4096;

export function upsertLiveTranscriptEvent(
  current: LiveTranscriptEventDto[],
  next: LiveTranscriptEventDto,
) {
  const events = current
    .filter((event) => !isSameLiveSegment(event, next))
    .concat(next)
    .sort(compareLiveTranscriptEvents);
  const sameSession = events.filter((event) => event.sessionId === next.sessionId);
  const newestEndMs = Math.max(...sameSession.map((event) => event.endMs));
  const windowStartMs = newestEndMs - LIVE_TRANSCRIPT_WINDOW_MS;
  const sessionWindow = sameSession
    .filter((event) => event.endMs >= windowStartMs)
    .slice(-LIVE_TRANSCRIPT_MAX_EVENTS_PER_SESSION);
  const remainingCapacity = LIVE_TRANSCRIPT_MAX_EVENTS_TOTAL - sessionWindow.length;
  const retainedOtherSessions =
    remainingCapacity > 0
      ? events.filter((event) => event.sessionId !== next.sessionId).slice(-remainingCapacity)
      : [];
  return retainedOtherSessions.concat(sessionWindow).sort(compareLiveTranscriptEvents);
}

/**
 * Coalescing is presentation-only. The stored preview events retain their
 * segment ids so persisted transcript spans can replace exactly the preview
 * time range they supersede.
 */
export function coalesceLiveTranscriptEventsForDisplay(events: LiveTranscriptEventDto[]) {
  const coalesced: LiveTranscriptEventDto[] = [];
  for (const event of [...events].sort(compareLiveTranscriptEvents)) {
    const previous = coalesced.at(-1);
    if (
      previous &&
      isSameLiveTurn(previous, event) &&
      event.startMs - previous.endMs <= LIVE_TRANSCRIPT_COHERENCE_GAP_MS
    ) {
      coalesced[coalesced.length - 1] = mergeLiveTranscriptEvents(previous, event);
    } else {
      coalesced.push(event);
    }
  }
  return coalesced;
}

/**
 * Saved-audio transcript rows are authoritative. A row may only supersede a
 * live preview from the same recording session, Source, and time span;
 * legacy rows without a recording session id deliberately reconcile nothing.
 */
export function reconcileLiveTranscriptEvents(
  events: LiveTranscriptEventDto[],
  persisted: TranscriptDto[],
  discarded: DiscardedTranscriptSpanDto[] = [],
) {
  return preserveReferenceWhenUnchanged(
    events,
    events.filter((event) => !hasAuthoritativeCoverage(event, persisted, discarded)),
  );
}

/**
 * Drop only previews that authoritative saved-audio rows replaced. A terminal
 * batch failure must not erase an unmatched live span: it may be the only text
 * the user can still see while the saved WAV remains retryable.
 */
export function clearTerminalLiveTranscriptEvents(
  events: LiveTranscriptEventDto[],
  noteId: string,
  persisted: TranscriptDto[],
  protectedSessionIds: readonly string[] = [],
) {
  const protectedSessions = new Set(protectedSessionIds);
  return preserveReferenceWhenUnchanged(
    events,
    events.filter(
      (event) =>
        event.noteId !== noteId ||
        protectedSessions.has(event.sessionId) ||
        !hasAuthoritativeCoverage(event, persisted, []),
    ),
  );
}

/**
 * Stable dependency key for the persisted coverage that can replace live
 * preview events. Polling rebuilds TranscriptDto arrays even when their
 * coverage is unchanged, so depending on the array reference would rerun the
 * cleanup effect on every response.
 */
export function authoritativeTranscriptCoverageKey(persisted: TranscriptDto[]) {
  const spans = persisted
    .filter((turn) => turn.status === "succeeded" && turn.text.trim().length > 0)
    .map((turn) => [
      turn.recordingSessionId ?? null,
      turn.source ?? null,
      turn.startMs ?? null,
      turn.endMs ?? null,
    ])
    .sort((left, right) => JSON.stringify(left).localeCompare(JSON.stringify(right)));
  return JSON.stringify(spans);
}

/**
 * Stable dependency key for changes that can alter the visible transcription.
 * App polling reconstructs DTO objects, so React effects must depend on values,
 * not array identity.
 */
export function transcriptFollowLatestKey(
  live: LiveTranscriptEventDto[],
  persisted: TranscriptDto[],
) {
  return JSON.stringify({
    live: live.map((event) => [
      event.noteId,
      event.sessionId,
      event.sourceMode,
      event.source,
      event.segmentId,
      event.startMs,
      event.endMs,
      event.text,
      event.language ?? null,
      event.stability,
    ]),
    persisted: persisted.map((turn) => [
      turn.id,
      turn.recordingSessionId ?? null,
      turn.spanId ?? null,
      turn.sourceMode ?? null,
      turn.source ?? null,
      turn.startMs ?? null,
      turn.endMs ?? null,
      turn.turnIndex ?? null,
      turn.text,
      turn.language ?? null,
      turn.status,
      turn.lastError ?? null,
      turn.recordedSilence ?? false,
    ]),
  });
}

function preserveReferenceWhenUnchanged(
  current: LiveTranscriptEventDto[],
  filtered: LiveTranscriptEventDto[],
) {
  return filtered.length === current.length ? current : filtered;
}

function hasAuthoritativeCoverage(
  event: LiveTranscriptEventDto,
  persisted: TranscriptDto[],
  discarded: DiscardedTranscriptSpanDto[],
) {
  const spans = [
    ...discarded.filter(
      (span) => span.sessionId === event.sessionId && span.source === event.source,
    ),
    ...persisted
      .filter(
        (turn) =>
          turn.status === "succeeded" &&
          turn.text.trim().length > 0 &&
          turn.recordingSessionId === event.sessionId &&
          turn.source === event.source &&
          typeof turn.startMs === "number" &&
          typeof turn.endMs === "number",
      )
      .map((turn) => ({ startMs: turn.startMs as number, endMs: turn.endMs as number })),
  ].sort((a, b) => a.startMs - b.startMs);
  let covered = event.startMs;
  for (const span of spans) {
    if (span.startMs > covered) break;
    covered = Math.max(covered, span.endMs);
    if (covered >= event.endMs) return true;
  }
  return false;
}

function isSameLiveSegment(left: LiveTranscriptEventDto, right: LiveTranscriptEventDto) {
  return (
    left.noteId === right.noteId &&
    left.sessionId === right.sessionId &&
    left.source === right.source &&
    left.segmentId === right.segmentId
  );
}

function isSameLiveTurn(left: LiveTranscriptEventDto, right: LiveTranscriptEventDto) {
  return (
    left.noteId === right.noteId &&
    left.sessionId === right.sessionId &&
    left.sourceMode === right.sourceMode &&
    left.source === right.source
  );
}

function mergeLiveTranscriptEvents(
  left: LiveTranscriptEventDto,
  right: LiveTranscriptEventDto,
): LiveTranscriptEventDto {
  return {
    ...left,
    startMs: Math.min(left.startMs, right.startMs),
    endMs: Math.max(left.endMs, right.endMs),
    text: appendLiveTranscriptText(left.text, right.text),
    language: right.language ?? left.language,
    stability: right.stability,
  };
}

function appendLiveTranscriptText(left: string, right: string) {
  const leftText = left.trim();
  const rightText = right.trim();
  if (!leftText) return rightText;
  if (!rightText) return leftText;
  return `${leftText} ${rightText}`;
}

function compareLiveTranscriptEvents(left: LiveTranscriptEventDto, right: LiveTranscriptEventDto) {
  return (
    left.startMs - right.startMs ||
    left.endMs - right.endMs ||
    left.segmentId.localeCompare(right.segmentId)
  );
}
