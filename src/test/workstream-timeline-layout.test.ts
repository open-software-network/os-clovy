import { describe, expect, it } from "vitest";
import {
  densityFor,
  layoutEmptyTimeline,
  layoutWorkstreamTimeline,
  visibleNotes,
} from "../components/activity-timeline/workstream-timeline-layout";
import type { WorkstreamDto } from "../lib/day-intelligence";

// Pure layout of the workstream timeline, pinned to a DST zone. New York local days,
// given as their UTC instants (local midnight to next local midnight).
describe("Workstream timeline layout", () => {
  const ZONE = "America/New_York";

  it("places overlapping workstreams in separate lanes and separate ones in the same lane", () => {
    const start = Date.parse("2026-06-15T04:00:00Z");
    const end = Date.parse("2026-06-16T04:00:00Z");
    const period = (from: string, to: string) => ({
      startedAt: `2026-06-15T${from}:00Z`,
      endedAt: `2026-06-15T${to}:00Z`,
    });
    const ws = (id: number, from: string, to: string): WorkstreamDto => ({
      id,
      title: `W${id}`,
      summary: "",
      minutes: 10,
      hours: [
        {
          hour: "2026-06-15T09",
          minutes: 10,
          note: "",
          periods: [period(from, to)],
          locatedMinutes: 10,
        },
      ],
    });
    const layout = layoutWorkstreamTimeline(
      [ws(1, "13:00", "13:40"), ws(2, "13:20", "13:50"), ws(3, "16:00", "16:30")],
      {
        dayStartMs: start,
        dayEndMs: end,
        timeZone: ZONE,
      },
    );
    const lane = (id: number) => layout.cards.find((c) => c.workstream.id === id);
    expect(lane(1)?.laneCount).toBe(2);
    expect(lane(1)?.lane).not.toBe(lane(2)?.lane);
    expect(lane(3)?.laneCount).toBe(1);
    expect(lane(3)?.lane).toBe(0);
  });

  it("measures a spring-forward day by real time and places a legacy hour at its real local start", () => {
    // 2026-03-08 in New York has no 02:00: 01:30 to 03:30 local is one real hour.
    const start = Date.parse("2026-03-08T05:00:00Z");
    const end = Date.parse("2026-03-09T04:00:00Z");
    const precise: WorkstreamDto = {
      id: 1,
      title: "Across the change",
      summary: "",
      minutes: 60,
      hours: [
        {
          hour: "2026-03-08T01",
          minutes: 60,
          note: "",
          periods: [{ startedAt: "2026-03-08T06:30:00Z", endedAt: "2026-03-08T07:30:00Z" }],
          locatedMinutes: 60,
        },
      ],
    };
    const old: WorkstreamDto = {
      id: 2,
      title: "Legacy",
      summary: "",
      minutes: 20,
      hours: [{ hour: "2026-03-08T03", minutes: 20, note: "", periods: null, locatedMinutes: 0 }],
    };
    const layout = layoutWorkstreamTimeline([precise, old], {
      dayStartMs: start,
      dayEndMs: end,
      timeZone: ZONE,
    });
    const legacyCard = layout.cards.find((c) => c.workstream.id === 2);
    expect(legacyCard?.approximate).toBe(true);
    expect(legacyCard?.segments[0].startMs).toBe(Date.parse("2026-03-08T07:00:00Z"));
    expect(legacyCard?.segments[0].endMs).toBe(Date.parse("2026-03-08T08:00:00Z"));
    const preciseCard = layout.cards.find((c) => c.workstream.id === 1);
    expect(preciseCard?.approximate).toBe(false);
    expect(preciseCard?.endMs).toBe(preciseCard ? preciseCard.startMs + 3_600_000 : NaN);
    expect(
      layout.toPx(Date.parse("2026-03-08T07:30:00Z")) -
        layout.toPx(Date.parse("2026-03-08T06:30:00Z")),
    ).toBe(
      layout.toPx(Date.parse("2026-03-08T08:00:00Z")) -
        layout.toPx(Date.parse("2026-03-08T07:00:00Z")),
    );
    // The rail labels real local hours only: 1 AM, then 3 AM, never 2 AM.
    expect(layout.hours.map((hour) => hour.hour)).toEqual([1, 3, 4]);
  });

  it("keeps an hour's work that no session located known by hour instead of treating it as located", () => {
    const start = Date.parse("2026-06-15T04:00:00Z");
    const end = Date.parse("2026-06-16T04:00:00Z");
    const partial: WorkstreamDto = {
      id: 1,
      title: "Partly located",
      summary: "",
      minutes: 30,
      hours: [
        {
          hour: "2026-06-15T09",
          minutes: 30,
          note: "",
          periods: [{ startedAt: "2026-06-15T13:10:00Z", endedAt: "2026-06-15T13:20:00Z" }],
          locatedMinutes: 10,
        },
      ],
    };
    const [card] = layoutWorkstreamTimeline([partial], {
      dayStartMs: start,
      dayEndMs: end,
      timeZone: ZONE,
    }).cards;
    expect(card.approximate).toBe(true);
    expect(card.sittings).toBe(1);
    const unlocated = card.segments.find((segment) => segment.kind === "unlocated");
    expect(unlocated).toMatchObject({
      startMs: Date.parse("2026-06-15T13:00:00Z"),
      endMs: Date.parse("2026-06-15T14:00:00Z"),
      minutes: 20,
    });
  });

  it("draws a legacy repeated fall-back hour over its whole two-hour window", () => {
    // 2026-11-01 in New York repeats 01:00: the key "T01" spans 01:00 EDT to 02:00 EST.
    const start = Date.parse("2026-11-01T04:00:00Z");
    const end = Date.parse("2026-11-02T05:00:00Z");
    const old: WorkstreamDto = {
      id: 1,
      title: "Legacy",
      summary: "",
      minutes: 50,
      hours: [{ hour: "2026-11-01T01", minutes: 50, note: "", periods: null, locatedMinutes: 0 }],
    };
    const [card] = layoutWorkstreamTimeline([old], {
      dayStartMs: start,
      dayEndMs: end,
      timeZone: ZONE,
    }).cards;
    expect(card.segments).toHaveLength(1);
    expect(card.startMs).toBe(Date.parse("2026-11-01T05:00:00Z"));
    expect(card.endMs).toBe(Date.parse("2026-11-01T07:00:00Z"));
  });

  describe("scale (spec S1)", () => {
    const start = Date.parse("2026-06-15T04:00:00Z");
    const end = Date.parse("2026-06-16T04:00:00Z");
    const one = (from: string, to: string): WorkstreamDto => ({
      id: 1,
      title: "One",
      summary: "",
      minutes: 60,
      hours: [
        {
          hour: "2026-06-15T10",
          minutes: 60,
          note: "",
          periods: [{ startedAt: `2026-06-15T${from}:00Z`, endedAt: `2026-06-15T${to}:00Z` }],
          locatedMinutes: 60,
        },
      ],
    });
    const layout = (work: WorkstreamDto, canvasHeight: number, nowMs?: number) =>
      layoutWorkstreamTimeline([work], {
        dayStartMs: start,
        dayEndMs: end,
        timeZone: ZONE,
        canvasHeight,
        nowMs,
      });

    it.each([
      [403, 121, 161],
      [703, 211, 281],
      [903, 271, 361],
      [1303, 391, 521],
    ])("fills a %i px canvas with a 150-minute window centered on one hour", (height, top, size) => {
      // 10:00 to 11:00 local (14:00 to 15:00 UTC).
      const result = layout(one("14:00", "15:00"), height);
      expect(result.height).toBe(height);
      expect(result.cards[0]).toMatchObject({ top, height: size, density: "notes" });
    });

    it("caps growth at 9 px per minute", () => {
      const result = layout(one("14:00", "14:10"), 1303);
      expect(result.pxPerMin).toBe(9);
      expect(result.height).toBe(1260);
      expect(result.cards[0]).toMatchObject({ top: 540, height: 90 });
    });

    it("extends the window to now today and marks the current hour", () => {
      // Work 9:00 to 9:30 local; now is 15:10 local.
      const result = layout(one("13:00", "13:30"), 303, Date.parse("2026-06-15T19:10:00Z"));
      expect(result.toPx(Date.parse("2026-06-15T19:10:00Z"))).toBe(result.height);
      const current = result.hours.filter((hour) => hour.current);
      expect(current.map((hour) => hour.hour)).toEqual([15]);
      expect(result.hours.at(-1)?.hour).toBe(15);
    });
  });

  it("thresholds and note slots follow the text size and the measured header", () => {
    expect([45, 46, 61, 62, 103, 104].map((height) => densityFor(height))).toEqual([
      "pill",
      "title",
      "title",
      "meta",
      "meta",
      "notes",
    ]);
    expect(densityFor(104, 1.2)).toBe("meta");
    // Six notes under a two-line title (55 px header), spec S7f.
    expect(visibleNotes(6, 104, 55)).toEqual({ shown: 1, more: 5 });
    expect(visibleNotes(6, 152, 55)).toEqual({ shown: 4, more: 2 });
    // A wrapped meta line can leave no slot: then neither notes nor a count.
    expect(visibleNotes(6, 104, 90)).toEqual({ shown: 0, more: 0 });
  });

  describe("empty grid", () => {
    const start = Date.parse("2026-06-15T04:00:00Z");
    const end = Date.parse("2026-06-16T04:00:00Z");
    const range = (from: string, to: string) => ({
      startMs: Date.parse(`2026-06-15T${from}:00Z`),
      endMs: Date.parse(`2026-06-15T${to}:00Z`),
    });

    it("draws the axis the grid would draw for the same captured time", () => {
      const options = { dayStartMs: start, dayEndMs: end, timeZone: ZONE, canvasHeight: 600 };
      const empty = layoutEmptyTimeline(
        [range("13:05", "13:40"), range("15:10", "15:30")],
        options,
      );
      const ws: WorkstreamDto = {
        id: 1,
        title: "W",
        summary: "",
        minutes: 55,
        hours: [
          {
            hour: "2026-06-15T09",
            minutes: 55,
            note: "",
            periods: [
              { startedAt: "2026-06-15T13:05:00Z", endedAt: "2026-06-15T13:40:00Z" },
              { startedAt: "2026-06-15T15:10:00Z", endedAt: "2026-06-15T15:30:00Z" },
            ],
            locatedMinutes: 55,
          },
        ],
      };
      const grid = layoutWorkstreamTimeline([ws], options);
      expect(empty.cards).toHaveLength(0);
      expect(empty.hours.map((hour) => hour.hour)).toEqual(grid.hours.map((hour) => hour.hour));
      expect(empty.pxPerMin).toBe(grid.pxPerMin);
      expect(empty.toPx(Date.parse("2026-06-15T15:10:00Z"))).toBe(
        grid.toPx(Date.parse("2026-06-15T15:10:00Z")),
      );
    });

    it("ends today's window at now when nothing was captured", () => {
      const now = Date.parse("2026-06-15T14:30:00Z");
      const layout = layoutEmptyTimeline([], {
        dayStartMs: start,
        dayEndMs: end,
        nowMs: now,
        timeZone: ZONE,
      });
      expect(layout.hours.at(-1)).toMatchObject({ hour: 10, current: true });
      expect(layout.toPx(now)).toBeGreaterThan(0);
      expect(layout.height).toBeGreaterThan(layout.toPx(now));
    });

    it("has nothing to anchor on a past day with nothing captured", () => {
      const layout = layoutEmptyTimeline([], { dayStartMs: start, dayEndMs: end, timeZone: ZONE });
      expect(layout.hours).toHaveLength(0);
      expect(layout.height).toBe(0);
    });
  });
});
