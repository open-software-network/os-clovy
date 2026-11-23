import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
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
import { formatDuration } from "../components/activity-timeline/timeline-utils";
import { layoutWorkstreamTimeline } from "../components/activity-timeline/workstream-timeline-layout";
import { applyInterfaceLocale } from "../i18n/locale";
import { type DayIntelligenceDto, dayKey, type WorkstreamDto } from "../lib/day-intelligence";

/*
 * A workstream gathers work from the whole day. A pause longer than 45
 * minutes starts a new card of the same workstream at its real time; shorter
 * pauses stay gaps between the bars of one card. Each card carries the active
 * minutes and notes of its own hours; selecting any card selects the whole
 * workstream.
 */

const now = new Date();
const day = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
const MINUTE = 60_000;
const at = (clock: string) => {
  const [hour, minute] = clock.split(":").map(Number);
  return new Date(day.getFullYear(), day.getMonth(), day.getDate(), hour, minute).getTime();
};

/** Periods as "HH:MM+minutes"; each period's minutes are active in its hour. */
function workstream(id: number, title: string, periods: string[]): WorkstreamDto {
  const byHour = new Map<string, WorkstreamDto["hours"][number]>();
  for (const period of periods) {
    const [start, length] = period.split("+");
    const startMs = at(start);
    const minutes = Number(length);
    const key = `${dayKey(day)}T${start.slice(0, 2)}`;
    const hour = byHour.get(key) ?? {
      hour: key,
      minutes: 0,
      locatedMinutes: 0,
      note: `${title}: note at ${start.slice(0, 2)}h`,
      periods: [],
    };
    hour.minutes += minutes;
    hour.locatedMinutes += minutes;
    hour.periods?.push({
      startedAt: new Date(startMs).toISOString(),
      endedAt: new Date(startMs + minutes * MINUTE).toISOString(),
    });
    byHour.set(key, hour);
  }
  const hours = [...byHour.values()];
  return {
    id,
    title,
    summary: "Synthetic fixture",
    minutes: hours.reduce((sum, hour) => sum + hour.minutes, 0),
    hours,
  };
}

/** A day shaped like the owner's: 12 workstreams spread from 10:35 to 23:54. */
const REAL_DAY: WorkstreamDto[] = [
  workstream(1, "Clinic booking assistant over chat", [
    "10:35+2",
    "10:50+2",
    "11:40+3",
    "15:10+2",
    "15:30+2",
    "20:45+3",
    "20:50+7",
  ]),
  workstream(2, "New hire paperwork and accounts", [
    "10:35+3",
    "11:30+3",
    "12:57+3",
    "18:45+4",
    "19:10+4",
  ]),
  workstream(3, "Mail and calendar connectors", [
    "10:36+1",
    "10:50+1",
    "11:30+2",
    "12:10+1",
    "12:59+1",
    "14:20+1",
    "15:05+1",
    "16:40+1",
    "17:15+2",
    "18:30+1",
    "19:45+1",
    "20:20+1",
    "21:10+1",
    "22:30+1",
    "23:53+1",
  ]),
  workstream(4, "Customer support thread for an order", [
    "10:39+4",
    "11:05+5",
    "12:43+4",
    "22:10+4",
  ]),
  workstream(5, "Fixing the panel divider drag", ["10:57+4", "11:15+5", "11:40+5"]),
  workstream(6, "Comparing note-taking tools", ["11:12+8", "12:28+8"]),
  workstream(7, "Shopping for a headset and microphone", ["11:14+6"]),
  workstream(8, "Secrets manager fork setup", ["11:45+5"]),
  workstream(9, "Previewing the workstreams tab", ["11:49+7", "12:05+7", "18:10+6", "20:54+6"]),
  workstream(10, "Disk cleanup and maintenance", [
    "12:42+1",
    "12:47+1",
    "12:52+1",
    "18:20+1",
    "18:40+1",
    "19:05+1",
    "23:10+1",
  ]),
  workstream(11, "Review of an automation repository", ["18:45+17"]),
  workstream(12, "Looking into a video clipping tool", ["18:48+7", "23:40+3"]),
];

function serve(list: WorkstreamDto[]) {
  invokeMock.mockImplementation(async (command: string, args?: { request?: unknown }) => {
    const request = (args?.request ?? {}) as Record<string, unknown>;
    if (command === "activity_timeline") {
      return {
        availability: "ready",
        message: null,
        captureEnabled: true,
        sessions: [],
        gaps: [],
        stats: { focusedMs: 0, idleMs: 0, awayMs: 0, topApps: [], categories: [] },
      };
    }
    if (command === "day_intelligence_day") {
      return {
        availability: "ready",
        day: String(request.day),
        provider: "ready",
        embedder: "ready",
        running: false,
        summaryTime: "18:00",
        summary: null,
        workstreams: String(request.day) === dayKey(day) ? list : [],
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

async function openDay(list: WorkstreamDto[]) {
  serve(list);
  render(<ActivityTimelineView />);
  fireEvent.click(screen.getByRole("button", { name: "Previous day" }));
  await screen.findAllByRole("button", { name: new RegExp(`^${list[0].title},`) });
}

const cardsOf = (title: string) =>
  screen.queryAllByRole("button", { name: new RegExp(`^${title},`) });
const layoutOf = (list: WorkstreamDto[]) =>
  layoutWorkstreamTimeline(list, {
    dayStartMs: at("00:00"),
    dayEndMs: at("00:00") + 24 * 60 * MINUTE,
    canvasHeight: 903,
  });

beforeEach(() => {
  applyInterfaceLocale("en");
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(903);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Workstream cards split by long pauses", () => {
  const pauses = [
    // 9:10 to 9:55 is a 45-minute pause: still one card.
    workstream(1, "Pause of 45 minutes", ["09:00+10", "09:55+10"]),
    // 13:10 to 13:56 is 46 minutes: a second card at its real time.
    workstream(2, "Pause of 46 minutes", ["13:00+10", "13:56+4"]),
  ];

  it("keeps a pause of up to 45 minutes inside one card and splits a longer one", async () => {
    await openDay(pauses);
    const [kept] = cardsOf("Pause of 45 minutes");
    expect(cardsOf("Pause of 45 minutes")).toHaveLength(1);
    expect(within(kept).getAllByTestId("workstream-sitting")).toHaveLength(2);

    const split = cardsOf("Pause of 46 minutes");
    expect(split).toHaveLength(2);
    const [first, second] = split;
    expect(within(first).getAllByTestId("workstream-sitting")).toHaveLength(1);
    expect(within(second).getAllByTestId("workstream-sitting")).toHaveLength(1);
    // The second card starts at 13:56, below where the first one ends (13:10).
    expect(Number.parseFloat(second.style.top)).toBeGreaterThan(
      Number.parseFloat(first.style.top) + Number.parseFloat(first.style.height),
    );
    expect(screen.getByText("2 workstreams")).toBeInTheDocument();
  });

  it("gives each card the active minutes and notes of its own stretch", async () => {
    const resumed = workstream(1, "Resumed three times", ["09:00+7", "11:00+12", "14:05+5"]);
    await openDay([resumed]);
    const cards = cardsOf("Resumed three times");
    expect(cards.map((card) => card.getAttribute("aria-label")?.split(", ")[1])).toEqual([
      `${formatDuration(7 * MINUTE, "en")} active`,
      `${formatDuration(12 * MINUTE, "en")} active`,
      `${formatDuration(5 * MINUTE, "en")} active`,
    ]);
    // The header counts the workstream once, with its whole active time.
    expect(screen.getByText("1 workstream")).toBeInTheDocument();
    expect(screen.getByText(`${formatDuration(24 * MINUTE, "en")} active`)).toBeInTheDocument();

    const layout = layoutOf([resumed]);
    expect(layout.cards.map((card) => card.notes)).toEqual([
      ["Resumed three times: note at 09h"],
      ["Resumed three times: note at 11h"],
      ["Resumed three times: note at 14h"],
    ]);
  });

  it("selects the whole workstream from any of its cards", async () => {
    await openDay(pauses);
    const [first, second] = cardsOf("Pause of 46 minutes");
    fireEvent.click(second);
    expect(first).toHaveAttribute("aria-pressed", "true");
    expect(second).toHaveAttribute("aria-pressed", "true");
    expect(cardsOf("Pause of 45 minutes")[0]).toHaveAttribute("aria-pressed", "false");
    const detail = screen.getByRole("region", { name: "Workstream details" });
    expect(within(detail).getByRole("heading", { name: "Pause of 46 minutes" })).toBeVisible();
    // The details keep the whole day of the workstream: both sittings.
    expect(within(detail).getAllByTestId("workstream-period")).toHaveLength(2);
    fireEvent.click(first);
    expect(second).toHaveAttribute("aria-pressed", "false");
  });

  it("never draws a card over more than 45 minutes without work on a real-shaped day", async () => {
    const layout = layoutOf(REAL_DAY);
    expect(layout.cards.length).toBeGreaterThan(REAL_DAY.length);
    for (const card of layout.cards) {
      let reached = card.segments[0].endMs;
      for (const segment of card.segments.slice(1)) {
        expect(segment.startMs - reached).toBeLessThanOrEqual(45 * MINUTE);
        reached = Math.max(reached, segment.endMs);
      }
    }
    // Every workstream's cards add up to its whole active time.
    for (const work of REAL_DAY) {
      const total = layout.cards
        .filter((card) => card.workstream.id === work.id)
        .reduce((sum, card) => sum + card.activeMs, 0);
      expect(total).toBe(work.minutes * MINUTE);
    }

    await openDay(REAL_DAY);
    expect(screen.getByText("12 workstreams")).toBeInTheDocument();
    const minutes = REAL_DAY.reduce((sum, work) => sum + work.minutes, 0);
    expect(
      screen.getByText(`${formatDuration(minutes * MINUTE, "en")} active`),
    ).toBeInTheDocument();
    // The connectors workstream (15 periods, 10:36 to 23:54) is many short cards.
    expect(cardsOf("Mail and calendar connectors").length).toBeGreaterThan(5);
  });
});
