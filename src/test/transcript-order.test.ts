import { describe, expect, it } from "vitest";
import { orderTranscriptTurns } from "../lib/transcript-order";

describe("authoritative transcript presentation", () => {
  it("keeps recording order when later sessions restart counters and time", () => {
    const turns = [
      { id: "a3", recordingSessionId: "a", turnIndex: 3, startMs: 3000 },
      { id: "a2", recordingSessionId: "a", turnIndex: 2, startMs: 2000 },
      { id: "b0", recordingSessionId: "b", turnIndex: 0, startMs: 0 },
      { id: "legacy-one", text: "repeated" },
      { id: "legacy-two", text: "repeated" },
      { id: "live", recordingSessionId: "c", startMs: 0 },
    ];
    expect(orderTranscriptTurns(turns).map((turn) => turn.id)).toEqual([
      "a2",
      "a3",
      "b0",
      "legacy-one",
      "legacy-two",
      "live",
    ]);
    expect(turns[0].id).toBe("a3");
  });
});
