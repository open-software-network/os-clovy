import { cleanup, render, screen, within } from "@testing-library/react";
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
import { type DayIntelligenceDto, dayKey, type WorkstreamDto } from "../lib/day-intelligence";

/*
 * Today with little work: the window is recentered on the work to reach its
 * 150-minute minimum, and must still run to now, so the now line and the
 * current hour sit at their real time instead of being pinned to the bottom.
 */

const clock = (base: Date, hour: number, minute = 0) =>
  new Date(base.getFullYear(), base.getMonth(), base.getDate(), hour, minute);

function serveToday(today: Date) {
  const work: WorkstreamDto = {
    id: 1,
    title: "Short morning task",
    summary: "Synthetic fixture",
    minutes: 10,
    hours: [
      {
        hour: `${dayKey(today)}T09`,
        minutes: 10,
        locatedMinutes: 10,
        note: "",
        periods: [
          {
            startedAt: clock(today, 9).toISOString(),
            endedAt: clock(today, 9, 10).toISOString(),
          },
        ],
      },
    ],
  };
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
        workstreams: String(request.day) === dayKey(today) ? [work] : [],
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

beforeEach(() => {
  applyInterfaceLocale("en");
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(903);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe("Workstream timeline today with a short day", () => {
  it("keeps the window running to now after recentering on the work", async () => {
    // Work 9:00 to 9:10, now 10:50: 8:40 to 10:50 is under 150 minutes, and
    // the window recentered on the work alone would end at 10:20.
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(clock(new Date(), 10, 50));
    serveToday(new Date());
    render(<ActivityTimelineView />);
    await screen.findByRole("button", { name: /^Short morning task,/ });

    const grid = screen.getByRole("region", { name: "Workstreams of the day" });
    const canvas = within(grid).getByTestId("timeline-canvas");
    const height = Number.parseFloat(canvas.style.height);
    const hours = within(grid).getAllByTestId("timeline-hour");
    const top = (element: HTMLElement | null | undefined) =>
      Number.parseFloat(element?.style.top ?? "");
    const nine = hours.find((hour) => hour.textContent === "9 AM");
    const current = hours.find((hour) => hour.dataset.current === "true");
    const nowLine = canvas.querySelector<HTMLElement>(".ws-timeline-now");

    expect(current?.textContent).toBe("10 AM");
    expect(top(nowLine)).toBeLessThanOrEqual(height);
    // The active 9 AM hour is drawn at its real scale, so 10:00 to now (50
    // minutes) measures 50/60 of it: the now line is at 10:50, not at 10:20.
    const hourPx = top(current) - top(nine);
    expect((top(nowLine) - top(current)) / hourPx).toBeCloseTo(50 / 60, 1);
  });
});
