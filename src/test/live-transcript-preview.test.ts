import { describe, expect, it } from "vitest";
import {
  authoritativeTranscriptCoverageKey,
  clearTerminalLiveTranscriptEvents,
  coalesceLiveTranscriptEventsForDisplay,
  LIVE_TRANSCRIPT_MAX_EVENTS_PER_SESSION,
  reconcileLiveTranscriptEvents,
  transcriptFollowLatestKey,
  upsertLiveTranscriptEvent,
} from "../lib/live-transcript-preview";
import type { LiveTranscriptEventDto, TranscriptDto } from "../lib/tauri";

function liveEvent(overrides: Partial<LiveTranscriptEventDto> = {}): LiveTranscriptEventDto {
  return {
    noteId: "note-1",
    sessionId: "session-1",
    sourceMode: "microphonePlusSystem",
    source: "microphone",
    segmentId: "microphone-0",
    startMs: 0,
    endMs: 8000,
    text: "First chunk",
    language: "en",
    stability: "final",
    ...overrides,
  };
}

function persistedTurn(overrides: Partial<TranscriptDto> = {}): TranscriptDto {
  return {
    id: "turn-1",
    text: "Authoritative saved-audio words",
    recordingSessionId: "session-1",
    spanId: "span-1",
    sourceMode: "microphonePlusSystem",
    source: "microphone",
    startMs: 0,
    endMs: 8000,
    status: "succeeded",
    ...overrides,
  };
}

describe("live transcript preview", () => {
  it("preserves segment identity in state and only coalesces adjacent chunks for display", () => {
    const events = [
      liveEvent(),
      liveEvent({
        segmentId: "microphone-1",
        startMs: 8000,
        endMs: 16_000,
        text: "Second chunk",
      }),
    ].reduce(upsertLiveTranscriptEvent, [] as LiveTranscriptEventDto[]);

    expect(events.map((event) => event.segmentId)).toEqual(["microphone-0", "microphone-1"]);
    expect(coalesceLiveTranscriptEventsForDisplay(events)).toEqual([
      expect.objectContaining({
        source: "microphone",
        segmentId: "microphone-0",
        startMs: 0,
        endMs: 16_000,
        text: "First chunk Second chunk",
      }),
    ]);
  });

  it("keeps an ordinary long meeting's provisional segments", () => {
    const events = Array.from({ length: 300 }, (_, index) =>
      liveEvent({
        segmentId: `microphone-${index}`,
        startMs: index * 8000,
        endMs: (index + 1) * 8000,
        text: `Chunk ${index}`,
      }),
    ).reduce(upsertLiveTranscriptEvent, [] as LiveTranscriptEventDto[]);

    expect(events).toHaveLength(300);
    expect(events.at(0)?.segmentId).toBe("microphone-0");
    expect(events.at(-1)?.segmentId).toBe("microphone-299");
  });

  it("bounds provisional segments for an exceptionally long session", () => {
    const eventCount = LIVE_TRANSCRIPT_MAX_EVENTS_PER_SESSION + 20;
    const events = Array.from({ length: eventCount }, (_, index) =>
      liveEvent({
        segmentId: `microphone-${index.toString().padStart(4, "0")}`,
        startMs: index * 1000,
        endMs: (index + 1) * 1000,
        text: `Chunk ${index}`,
      }),
    ).reduce(upsertLiveTranscriptEvent, [] as LiveTranscriptEventDto[]);

    expect(events).toHaveLength(LIVE_TRANSCRIPT_MAX_EVENTS_PER_SESSION);
    expect(events.at(0)?.segmentId).toBe("microphone-0020");
    expect(events.at(-1)?.segmentId).toBe(`microphone-${eventCount - 1}`);
  });

  it("windows provisional segments to the latest two hours", () => {
    const events = [
      liveEvent({ segmentId: "old", startMs: 0, endMs: 1000 }),
      liveEvent({
        segmentId: "recent",
        startMs: 2 * 60 * 60 * 1000 + 2000,
        endMs: 2 * 60 * 60 * 1000 + 3000,
      }),
    ].reduce(upsertLiveTranscriptEvent, [] as LiveTranscriptEventDto[]);

    expect(events.map((event) => event.segmentId)).toEqual(["recent"]);
  });

  it("keeps same-source chunks separated across a material time gap", () => {
    const events = [
      liveEvent({ endMs: 4000, text: "Before pause" }),
      liveEvent({
        segmentId: "microphone-1",
        startMs: 6000,
        endMs: 8000,
        text: "After pause",
      }),
    ].reduce(upsertLiveTranscriptEvent, [] as LiveTranscriptEventDto[]);

    expect(coalesceLiveTranscriptEventsForDisplay(events).map((event) => event.text)).toEqual([
      "Before pause",
      "After pause",
    ]);
  });

  it("starts a new display turn when the source changes", () => {
    const events = [
      liveEvent({ segmentId: "microphone-0", text: "Mic one" }),
      liveEvent({
        source: "system",
        segmentId: "system-0",
        startMs: 8000,
        endMs: 16_000,
        text: "System one",
      }),
      liveEvent({
        source: "system",
        segmentId: "system-1",
        startMs: 16_000,
        endMs: 24_000,
        text: "System two",
      }),
      liveEvent({
        segmentId: "microphone-1",
        startMs: 24_000,
        endMs: 32_000,
        text: "Mic two",
      }),
    ].reduce(upsertLiveTranscriptEvent, [] as LiveTranscriptEventDto[]);

    const displayed = coalesceLiveTranscriptEventsForDisplay(events);
    expect(displayed.map((event) => event.text)).toEqual([
      "Mic one",
      "System one System two",
      "Mic two",
    ]);
    expect(displayed.map((event) => event.source)).toEqual(["microphone", "system", "microphone"]);
  });

  it("orders out-of-order chunks for display without destroying their ids", () => {
    const events = [
      liveEvent({
        segmentId: "microphone-1",
        startMs: 8000,
        endMs: 16_000,
        text: "Second chunk",
      }),
      liveEvent({
        segmentId: "microphone-0",
        startMs: 0,
        endMs: 8000,
        text: "First chunk",
      }),
    ].reduce(upsertLiveTranscriptEvent, [] as LiveTranscriptEventDto[]);

    expect(events).toHaveLength(2);
    expect(coalesceLiveTranscriptEventsForDisplay(events)[0]).toEqual(
      expect.objectContaining({
        text: "First chunk Second chunk",
        startMs: 0,
        endMs: 16_000,
      }),
    );
  });

  it("lets an overlapping saved-audio row replace different preview text", () => {
    const events = [liveEvent({ text: "Provider guessed this text" })];

    expect(reconcileLiveTranscriptEvents(events, [persistedTurn()])).toEqual([]);
  });

  it("never reconciles a preview against another recording session", () => {
    const sessionA = liveEvent({ sessionId: "session-a", text: "Session A preview" });
    const sessionB = liveEvent({ sessionId: "session-b", text: "Session B preview" });

    expect(
      reconcileLiveTranscriptEvents(
        [sessionA, sessionB],
        [persistedTurn({ recordingSessionId: "session-a" })],
      ),
    ).toEqual([sessionB]);
  });

  it("removes only previews fully covered by an explicit echo discard", () => {
    const mic = liveEvent();
    const otherSession = liveEvent({ sessionId: "session-2" });
    const partial = liveEvent({ segmentId: "partial", startMs: 7000, endMs: 9000 });
    const system = liveEvent({ source: "system", segmentId: "system" });
    const discard = {
      sessionId: "session-1",
      source: "microphone",
      startMs: 0,
      endMs: 8000,
    } as const;
    expect(
      reconcileLiveTranscriptEvents(
        [mic, otherSession, partial, system],
        [persistedTurn({ source: "system", status: "failed", text: "" })],
        [discard],
      ),
    ).toEqual([otherSession, partial, system]);
  });

  it("preserves partial final coverage and combines contiguous final and discard spans", () => {
    const preview = liveEvent();
    const final = persistedTurn({ startMs: 4000, endMs: 8000 });
    expect(reconcileLiveTranscriptEvents([preview], [final])).toEqual([preview]);
    expect(
      reconcileLiveTranscriptEvents(
        [preview],
        [final],
        [{ sessionId: "session-1", source: "microphone", startMs: 0, endMs: 4000 }],
      ),
    ).toEqual([]);
  });

  it("never lets legacy unscoped rows erase a preview", () => {
    const legacy = persistedTurn({ recordingSessionId: undefined });
    expect(reconcileLiveTranscriptEvents([liveEvent()], [legacy])).toHaveLength(1);
  });

  it("never lets a failed or empty saved-audio row erase a preview", () => {
    const failed = persistedTurn({ status: "failed", text: "" });
    const empty = persistedTurn({ text: "   " });

    expect(reconcileLiveTranscriptEvents([liveEvent()], [failed])).toHaveLength(1);
    expect(reconcileLiveTranscriptEvents([liveEvent()], [empty])).toHaveLength(1);
  });

  it("clears only replaced terminal previews and preserves unmatched or active spans", () => {
    const inactive = liveEvent({ sessionId: "session-a" });
    const active = liveEvent({ sessionId: "session-b", segmentId: "microphone-b" });
    const otherNote = liveEvent({ noteId: "note-2", sessionId: "session-c" });
    const replaced = persistedTurn({ recordingSessionId: "session-a" });

    expect(
      clearTerminalLiveTranscriptEvents(
        [inactive, active, otherNote],
        "note-1",
        [replaced],
        ["session-b"],
      ),
    ).toEqual([active, otherNote]);
    expect(clearTerminalLiveTranscriptEvents([inactive, otherNote], "note-1", [])).toEqual([
      inactive,
      otherNote,
    ]);
  });

  it("preserves the event array when reconciliation removes nothing", () => {
    const events = [liveEvent()];

    expect(reconcileLiveTranscriptEvents(events, [])).toBe(events);
    expect(clearTerminalLiveTranscriptEvents(events, "note-1", [])).toBe(events);
  });

  it("keeps the authoritative coverage key stable across polling array rebuilds", () => {
    const microphone = persistedTurn();
    const system = persistedTurn({
      id: "turn-2",
      recordingSessionId: "session-2",
      source: "system",
      startMs: 10_000,
      endMs: 18_000,
    });
    const key = authoritativeTranscriptCoverageKey([microphone, system]);

    expect(
      authoritativeTranscriptCoverageKey([{ ...system }, { ...microphone, text: "Revised" }]),
    ).toBe(key);
    expect(
      authoritativeTranscriptCoverageKey([{ ...microphone, status: "failed" }, system]),
    ).not.toBe(key);
    expect(
      authoritativeTranscriptCoverageKey([
        microphone,
        { ...system, recordingSessionId: "session-3" },
      ]),
    ).not.toBe(key);
  });

  it("keeps the follow-latest key stable across equivalent polling objects", () => {
    const preview = [liveEvent()];
    const persisted = [persistedTurn()];

    expect(transcriptFollowLatestKey(preview, persisted)).toBe(
      transcriptFollowLatestKey(
        preview.map((event) => ({ ...event })),
        persisted.map((turn) => ({ ...turn })),
      ),
    );
  });

  it("changes the follow-latest key when visible transcript content changes", () => {
    const initial = transcriptFollowLatestKey([liveEvent()], []);
    const revisedPreview = transcriptFollowLatestKey(
      [liveEvent({ text: "Revised preview words", endMs: 9000 })],
      [],
    );
    const persistedReplacement = transcriptFollowLatestKey([], [persistedTurn()]);

    expect(revisedPreview).not.toBe(initial);
    expect(persistedReplacement).not.toBe(initial);
  });
});
