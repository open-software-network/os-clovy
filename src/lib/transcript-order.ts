type TranscriptTurn = {
  id: string;
  recordingSessionId?: string;
  startMs?: number;
  endMs?: number;
  turnIndex?: number;
};

/** Preserve authoritative recording order: counters and relative times restart
 * per session. Legacy rows retain their own identity and input order. */
export function orderTranscriptTurns<T extends TranscriptTurn>(turns: readonly T[]): T[] {
  const key = (turn: T) =>
    turn.recordingSessionId ? `session:${turn.recordingSessionId}` : `legacy:${turn.id}`;
  const sessions = new Map<string, number>();
  for (const turn of turns) {
    if (!sessions.has(key(turn))) sessions.set(key(turn), sessions.size);
  }
  const compareNumber = (left?: number, right?: number) =>
    (left ?? Number.MAX_SAFE_INTEGER) - (right ?? Number.MAX_SAFE_INTEGER);
  return turns
    .map((turn, index) => ({ turn, index }))
    .sort(
      (left, right) =>
        (sessions.get(key(left.turn)) ?? 0) - (sessions.get(key(right.turn)) ?? 0) ||
        compareNumber(left.turn.startMs, right.turn.startMs) ||
        compareNumber(left.turn.turnIndex, right.turn.turnIndex) ||
        compareNumber(left.turn.endMs, right.turn.endMs) ||
        left.index - right.index,
    )
    .map(({ turn }) => turn);
}
