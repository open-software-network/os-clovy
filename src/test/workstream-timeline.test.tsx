import { act, cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import type * as TauriCore from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof TauriCore>()),
  invoke: async (command: string, args?: unknown) => {
    if (command === "coding_agents_blocks") return [];
    return args !== undefined ? invokeMock(command, args) : invokeMock(command);
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => undefined),
}));

import { ActivityTimelineView } from "../components/activity-timeline/ActivityTimelineView";
import { applyInterfaceLocale } from "../i18n/locale";
import type { ActivityTimelineDto } from "../lib/activity-timeline";
import { type DayIntelligenceDto, dayKey, type WorkstreamDto } from "../lib/day-intelligence";

/*
 * The Today → Timeline → Workstreams column, driven through the view the user
 * opens, on synthetic days that mirror the measured scenarios of the clean-room
 * spec (sections 2 to 7 and 11). Positions are the canvas pixels the cards,
 * hour labels and sitting bars are placed at; the usable canvas height is the
 * scroller's measured content box (the spec's pane height minus 97 px).
 */

type Fixture = {
  title: string;
  minutes: number;
  sittings: [string, string][];
  notes?: string[];
};

// An ordinary local day keeps the synthetic wall-clock durations equal to
// elapsed minutes, including when the real previous day crosses DST.
const now = new Date(2026, 4, 20, 12);
const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
const at = (base: Date, clock: string) => {
  const [hour, minute] = clock.split(":").map(Number);
  return new Date(base.getFullYear(), base.getMonth(), base.getDate(), hour, minute);
};
const hourKey = (base: Date, hour: number) => `${dayKey(base)}T${String(hour).padStart(2, "0")}`;

function workstreams(base: Date, fixtures: Fixture[]): WorkstreamDto[] {
  return fixtures.map((fixture, index) => {
    const notes = [...(fixture.notes ?? [])];
    const hours: WorkstreamDto["hours"] = fixture.sittings.map(([start, end], sitting) => ({
      hour: hourKey(base, at(base, start).getHours()) + (sitting > 0 ? `:${sitting}` : ""),
      minutes: 0,
      locatedMinutes: 0,
      note: notes.shift() ?? "",
      periods: [{ startedAt: at(base, start).toISOString(), endedAt: at(base, end).toISOString() }],
    }));
    // Remaining notes ride on hours without any work of their own.
    notes.forEach((note, extra) => {
      hours.push({
        hour: `${hourKey(base, 0)}:note${extra}`,
        minutes: 0,
        locatedMinutes: 0,
        note,
        periods: [],
      });
    });
    return {
      id: index + 1,
      title: fixture.title,
      summary: "Synthetic fixture",
      minutes: fixture.minutes,
      hours,
    };
  });
}

const timeline: ActivityTimelineDto = {
  availability: "ready",
  message: null,
  captureEnabled: true,
  sessions: [],
  gaps: [],
  stats: { focusedMs: 0, idleMs: 0, awayMs: 0, topApps: [], categories: [] },
};

function serveDay(base: Date, fixtures: Fixture[]) {
  const list = workstreams(base, fixtures);
  invokeMock.mockImplementation(async (command: string, args?: { request?: unknown }) => {
    const request = (args?.request ?? {}) as Record<string, unknown>;
    if (command === "activity_timeline") return timeline;
    if (command === "day_intelligence_day") {
      const day = String(request.day);
      return {
        availability: "ready",
        day,
        provider: "ready",
        embedder: "ready",
        running: false,
        summaryTime: "18:00",
        summary: null,
        workstreams: day === dayKey(base) ? list : [],
        hourReports: [],
        panels: {
          focusedMs: 0,
          idleMs: 0,
          awayMs: 0,
          categories: [],
          topApps: [],
          hours: [],
          workstreams: [],
          meetingCount: 0,
          meetingMs: 0,
          codingAgentBlocks: 0,
          codingAgentActiveSeconds: 0,
        },
      } satisfies DayIntelligenceDto;
    }
    return undefined;
  });
}

let canvasHeight = 903;

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(now);
  applyInterfaceLocale("en");
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockImplementation(() => canvasHeight);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

/** Opens yesterday (no now line) with the fixtures on a canvas of `height`. */
async function openDay(fixtures: Fixture[], height: number) {
  canvasHeight = height;
  serveDay(yesterday, fixtures);
  render(<ActivityTimelineView />);
  fireEvent.click(screen.getByRole("button", { name: "Previous day" }));
  await screen.findByRole("button", { name: new RegExp(fixtures[0].title) });
}

const card = (title: string) => screen.getByRole("button", { name: new RegExp(`^${title},`) });
const px = (value: string) => Number.parseFloat(value);
const box = (title: string) => {
  const element = card(title);
  return { top: px(element.style.top), height: px(element.style.height) };
};
const lanes = (title: string) => {
  const element = card(title);
  return [Number(element.dataset.lane), Number(element.dataset.lanes)];
};
const grid = () => screen.getByRole("region", { name: "Workstreams of the day" });
/** Hour labels on the rail, in order, with the canvas y they center on. */
const hourLabels = () =>
  within(grid())
    .queryAllByTestId("timeline-hour")
    .map((hour) => [hour.textContent, px(hour.style.top)] as const);
const bars = (title: string) =>
  within(card(title))
    .queryAllByTestId("workstream-sitting")
    .map((bar) => [px(bar.style.top), px(bar.style.height)]);

describe("Workstream timeline scale (S2)", () => {
  const fixtures: Fixture[] = [
    { title: "Morning", minutes: 60, sittings: [["09:00", "10:00"]] },
    { title: "Afternoon", minutes: 60, sittings: [["15:00", "16:00"]] },
  ];

  it("pads the day by 20 minutes and collapses the idle hours into one hour at 1 px/min", async () => {
    await openDay(fixtures, 203);
    expect(box("Morning")).toEqual({ top: 20, height: 60 });
    expect(box("Afternoon")).toEqual({ top: 140, height: 60 });
    expect(hourLabels()).toEqual([
      ["9 AM", 20],
      ["10 AM", 80],
      ["3 PM", 140],
      ["4 PM", 200],
    ]);
  });

  it("grows idle and active hours by one factor to fill a tall canvas", async () => {
    await openDay(fixtures, 903);
    const morning = box("Morning");
    const afternoon = box("Afternoon");
    expect(morning.top).toBe(82);
    expect(morning.height).toBe(246);
    expect(afternoon.height).toBe(246);
    // Five idle hours (10:00 to 15:00) take the height of one active hour.
    expect(
      Math.abs(afternoon.top - (morning.top + morning.height) - morning.height),
    ).toBeLessThanOrEqual(1);
  });
});

describe("Workstream timeline hour labels (S4)", () => {
  const fixtures: Fixture[] = [
    { title: "A", minutes: 30, sittings: [["09:00", "09:30"]] },
    { title: "B", minutes: 30, sittings: [["13:00", "13:30"]] },
  ];

  it("labels hours with work and the hour where the work stopped, nothing in between", async () => {
    await openDay(fixtures, 303);
    expect(hourLabels()).toEqual([
      ["9 AM", 32],
      ["10 AM", 128],
      ["1 PM", 223],
      ["2 PM", 303],
    ]);
    expect(box("A")).toEqual({ top: 32, height: 48 });
  });

  it("stops at the current hour today and marks it on the rail", async () => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(at(new Date(), "13:40"));
    canvasHeight = 303;
    const today = new Date();
    serveDay(today, fixtures);
    render(<ActivityTimelineView />);
    await screen.findByRole("button", { name: /^A,/ });
    const labels = hourLabels();
    expect(labels.map(([text]) => text)).toEqual(["9 AM", "10 AM", "1 PM"]);
    const current = within(grid()).getAllByTestId("timeline-hour").at(-1);
    expect(current?.dataset.current).toBe("true");
  });
});

describe("Workstream timeline lanes (S5)", () => {
  it("splits real overlaps into columns", async () => {
    await openDay(
      [
        { title: "First", minutes: 60, sittings: [["09:00", "10:00"]] },
        { title: "Second", minutes: 60, sittings: [["09:30", "10:30"]] },
      ],
      303,
    );
    expect(lanes("First")).toEqual([0, 2]);
    expect(lanes("Second")).toEqual([1, 2]);
    expect(box("First")).toEqual({ top: 61, height: 121 });
    expect(box("Second").top).toBe(121);
  });

  it("shares the widest column count across a transitive group and reuses free columns", async () => {
    await openDay(
      [
        { title: "Long", minutes: 180, sittings: [["09:00", "12:00"]] },
        { title: "Early", minutes: 30, sittings: [["09:30", "10:00"]] },
        { title: "Later", minutes: 30, sittings: [["10:30", "11:00"]] },
        { title: "After", minutes: 30, sittings: [["13:00", "13:30"]] },
      ],
      303,
    );
    expect(lanes("Long")).toEqual([0, 2]);
    expect(lanes("Early")).toEqual([1, 2]);
    expect(lanes("Later")).toEqual([1, 2]);
    expect(lanes("After")).toEqual([0, 1]);
    expect(box("Long")).toEqual({ top: 20, height: 180 });
    expect(box("After")).toEqual({ top: 260, height: 30 });
  });

  it("keeps back-to-back work in one full-width column, touching", async () => {
    await openDay(
      [
        { title: "First", minutes: 60, sittings: [["09:00", "10:00"]] },
        { title: "Second", minutes: 60, sittings: [["10:00", "11:00"]] },
      ],
      303,
    );
    expect(lanes("First")).toEqual([0, 1]);
    expect(lanes("Second")).toEqual([0, 1]);
    const first = box("First");
    expect(first).toEqual({ top: 38, height: 114 });
    expect(box("Second").top).toBe(first.top + first.height);
  });

  it("opens three columns for three overlapping workstreams", async () => {
    await openDay(
      [
        { title: "A", minutes: 60, sittings: [["09:00", "10:00"]] },
        { title: "B", minutes: 60, sittings: [["09:10", "10:10"]] },
        { title: "C", minutes: 60, sittings: [["09:20", "10:20"]] },
      ],
      303,
    );
    expect([lanes("A"), lanes("B"), lanes("C")]).toEqual([
      [0, 3],
      [1, 3],
      [2, 3],
    ]);
  });
});

describe("Workstream timeline short cards (S6)", () => {
  const tiny = (offset: number): Fixture[] => {
    const second = `09:${String(offset).padStart(2, "0")}`;
    const secondEnd = `09:${String(offset + 1).padStart(2, "0")}`;
    return [
      { title: "Tiny one", minutes: 1, sittings: [["09:00", "09:01"]] },
      { title: "Tiny two", minutes: 1, sittings: [[second, secondEnd]] },
      { title: "Filler", minutes: 600, sittings: [["11:00", "21:00"]] },
    ];
  };

  it("keeps cards at least 22 px tall and side by side when starts are under 28 px apart", async () => {
    await openDay(tiny(27), 403);
    expect(box("Tiny one")).toEqual({ top: 20, height: 22 });
    expect(box("Tiny two")).toEqual({ top: 47, height: 22 });
    expect(lanes("Tiny one")).toEqual([0, 2]);
    expect(lanes("Tiny two")).toEqual([1, 2]);
  });

  it("stacks them in one column with 6 px between from 28 px apart", async () => {
    await openDay(tiny(28), 403);
    expect(box("Tiny one")).toEqual({ top: 20, height: 22 });
    expect(box("Tiny two")).toEqual({ top: 48, height: 22 });
    expect(lanes("Tiny one")).toEqual([0, 1]);
    expect(lanes("Tiny two")).toEqual([0, 1]);
  });

  it("applies the rule in pixels, so a grown scale stacks a 10-minute offset", async () => {
    await openDay(tiny(10).slice(0, 2), 903);
    expect(box("Tiny one")).toEqual({ top: 384, height: 22 });
    expect(box("Tiny two")).toEqual({ top: 448, height: 22 });
    expect(lanes("Tiny two")).toEqual([0, 1]);
  });
});

describe("Workstream card anatomy by drawn height (S7)", () => {
  const notes = ["Item one", "Item two", "Item three", "Item four", "Item five", "Item six"];
  // Sequential cards at 1 px/min: each lasts as many minutes as its height.
  const sweep = (durations: number[], items = notes): Fixture[] => {
    let cursor = 0;
    const cards = durations.map((minutes): Fixture => {
      const start = cursor;
      cursor += minutes + 3;
      const clock = (total: number) =>
        `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
      return {
        title: `Dur ${minutes}`,
        minutes,
        sittings: [[clock(start + 30), clock(start + 30 + minutes)]],
        notes: items,
      };
    });
    // A long filler keeps the day taller than the canvas, so 1 px is 1 minute.
    return [...cards, { title: "Filler", minutes: 600, sittings: [["13:00", "23:00"]] }];
  };
  const shown = (title: string) => {
    const element = card(title);
    return {
      duration: within(element).queryByTestId("card-duration") !== null,
      dot: within(element).queryByTestId("card-dot") !== null,
      notes: within(element)
        .queryAllByTestId("card-note")
        .map((note) => note.textContent),
      more: within(element).queryByTestId("card-more")?.textContent ?? null,
    };
  };

  it("draws a pill, then a dot, then the meta line at 46 and 62 px", async () => {
    await openDay(sweep([44, 46, 61, 62, 103]), 503);
    expect(box("Dur 44").height).toBe(44);
    expect(shown("Dur 44")).toEqual({ duration: false, dot: false, notes: [], more: null });
    expect(shown("Dur 46")).toEqual({ duration: false, dot: true, notes: [], more: null });
    expect(shown("Dur 61")).toEqual({ duration: false, dot: true, notes: [], more: null });
    expect(shown("Dur 62")).toEqual({ duration: true, dot: true, notes: [], more: null });
    expect(shown("Dur 103").notes).toEqual([]);
  });

  it("shows as many items as fit from 104 px, with a count of the rest", async () => {
    await openDay(sweep([104, 116, 132, 150]), 503);
    expect(shown("Dur 104")).toMatchObject({ notes: notes.slice(0, 2), more: "+4 more" });
    expect(shown("Dur 116")).toMatchObject({ notes: notes.slice(0, 3), more: "+3 more" });
    expect(shown("Dur 132")).toMatchObject({ notes: notes.slice(0, 4), more: "+2 more" });
    expect(shown("Dur 150")).toMatchObject({ notes, more: null });
  });

  it("shows every item when the card holds them all", async () => {
    await openDay(sweep([104], notes.slice(0, 3)), 503);
    expect(shown("Dur 104")).toMatchObject({ notes: notes.slice(0, 3), more: null });
  });
});

describe("Workstream sitting bars (S8)", () => {
  it("draws one bar per sitting at its real time, inset 10 px from the ends", async () => {
    await openDay(
      [
        {
          title: "Resumed",
          minutes: 70,
          sittings: [
            ["09:00", "09:20"],
            ["09:40", "10:00"],
            ["10:30", "11:00"],
          ],
          notes: ["a", "b"],
        },
      ],
      303,
    );
    expect(box("Resumed")).toEqual({ top: 38, height: 227 });
    expect(bars("Resumed")).toEqual([
      [10, 28],
      [76, 38],
      [170, 47],
    ]);
    expect(within(card("Resumed")).getByText("· 3 sittings")).toBeInTheDocument();
  });

  it("keeps bars at least 4 px and inside a short card", async () => {
    await openDay(
      [
        {
          title: "Dots",
          minutes: 3,
          sittings: [
            ["09:00", "09:01"],
            ["09:20", "09:21"],
            ["09:40", "09:41"],
            // 39 minutes after the last sitting: within one card (pauses over
            // 45 minutes start a new card).
            ["10:20", "10:21"],
          ],
        },
        {
          title: "Short resumed",
          minutes: 12,
          sittings: [
            ["13:00", "13:06"],
            ["13:24", "13:30"],
          ],
        },
        { title: "Filler", minutes: 600, sittings: [["15:00", "23:00"]] },
      ],
      403,
    );
    const dots = bars("Dots");
    expect(dots.slice(0, 3)).toEqual([
      [10, 4],
      [20, 4],
      [40, 4],
    ]);
    const height = box("Dots").height;
    expect(height).toBe(81);
    // The last sitting ends past the inset area: its bar stays inside the card.
    for (const [top, size] of dots) expect(top + size).toBeLessThanOrEqual(height - 2);
    expect(bars("Short resumed")).toEqual([
      [7.5, 4],
      [24, 4],
    ]);
  });
});

describe("Workstream selection (S10)", () => {
  const fixtures: Fixture[] = [
    { title: "Detail card", minutes: 120, sittings: [["09:00", "11:20"]] },
    { title: "Neighbour", minutes: 30, sittings: [["09:30", "10:30"]] },
    { title: "Filler", minutes: 600, sittings: [["13:00", "23:00"]] },
  ];

  it("selects a card, dims the others, and toggles back on a second click", async () => {
    await openDay(fixtures, 603);
    fireEvent.click(card("Detail card"));
    expect(card("Detail card")).toHaveAttribute("aria-pressed", "true");
    expect(card("Neighbour")).toHaveAttribute("aria-pressed", "false");
    expect(grid()).toHaveAttribute("data-has-selection", "true");
    expect(screen.getByRole("region", { name: "Workstream details" })).toBeInTheDocument();
    fireEvent.click(card("Detail card"));
    expect(card("Detail card")).toHaveAttribute("aria-pressed", "false");
    expect(grid()).not.toHaveAttribute("data-has-selection");
  });

  it("clears the selection on a click in the empty column and on Escape", async () => {
    await openDay(fixtures, 603);
    fireEvent.click(card("Neighbour"));
    expect(card("Neighbour")).toHaveAttribute("aria-pressed", "true");
    fireEvent.click(within(grid()).getByTestId("timeline-canvas"));
    expect(card("Neighbour")).toHaveAttribute("aria-pressed", "false");
    fireEvent.click(card("Neighbour"));
    act(() => {
      fireEvent.keyDown(window, { key: "Escape" });
    });
    expect(card("Neighbour")).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("region", { name: "Workstream details" })).not.toBeInTheDocument();
  });
});
