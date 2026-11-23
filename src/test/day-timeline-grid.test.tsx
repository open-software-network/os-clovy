import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
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
import { formatDuration, formatTime } from "../components/activity-timeline/timeline-utils";
import { applyInterfaceLocale } from "../i18n/locale";
import type { ActivityTimelineDto, TimelineSessionDto } from "../lib/activity-timeline";
import { type DayIntelligenceDto, dayKey, type WorkstreamDto } from "../lib/day-intelligence";

// Fixture, not captured data: a local day with one workstream resumed after a
// break, one overlapping it, and one legacy record known only by hour.
const today = new Date();
const at = (hour: number, minute = 0, base = today) =>
  new Date(base.getFullYear(), base.getMonth(), base.getDate(), hour, minute);
const iso = (hour: number, minute = 0, base = today) => at(hour, minute, base).toISOString();
const hourKey = (hour: number, base = today) => `${dayKey(base)}T${String(hour).padStart(2, "0")}`;
const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);

const resumed: WorkstreamDto = {
  id: 1,
  title: "Timeline grid refactor",
  summary: "Moved the day into a vertical grid.",
  minutes: 38,
  hours: [
    {
      hour: hourKey(9),
      minutes: 24,
      note: "Drew the hour axis",
      periods: [{ startedAt: iso(9, 0), endedAt: iso(9, 30) }],
      locatedMinutes: 24,
    },
    {
      hour: hourKey(11),
      minutes: 14,
      note: "Placed the cards in lanes",
      periods: [{ startedAt: iso(11, 0), endedAt: iso(11, 20) }],
      locatedMinutes: 14,
    },
  ],
};

const overlapping: WorkstreamDto = {
  id: 2,
  title: "Release review",
  summary: "Reviewed the release checklist.",
  minutes: 30,
  hours: [
    {
      hour: hourKey(9),
      minutes: 30,
      note: "Checked the release notes",
      periods: [{ startedAt: iso(9, 10), endedAt: iso(9, 50) }],
      locatedMinutes: 30,
    },
  ],
};

const legacy: WorkstreamDto = {
  id: 3,
  title: "Inbox triage",
  summary: "Answered email.",
  minutes: 35,
  hours: [
    {
      hour: hourKey(14),
      minutes: 35,
      note: "Answered the support queue",
      periods: null,
      locatedMinutes: 0,
    },
  ],
};

const session: TimelineSessionDto = {
  id: 101,
  appName: "Visual Studio Code",
  bundleId: "com.microsoft.VSCode",
  contextKind: "workspace",
  context: "os-clovy",
  startedAt: iso(9, 0),
  endedAt: iso(9, 30),
  durationMs: 30 * 60_000,
  active: false,
  category: "coding",
  confidence: 0.9,
  windowTitle: "DayGrid.tsx - os-clovy",
};

const timeline: ActivityTimelineDto = {
  availability: "ready",
  message: null,
  captureEnabled: true,
  sessions: [session],
  gaps: [],
  stats: {
    focusedMs: 103 * 60_000,
    idleMs: 0,
    awayMs: 0,
    topApps: [
      { appName: "Visual Studio Code", bundleId: "com.microsoft.VSCode", durationMs: 60 * 60_000 },
      { appName: "Removed App", bundleId: "com.example.removed", durationMs: 20 * 60_000 },
    ],
    categories: [{ category: "coding", durationMs: 103 * 60_000 }],
  },
};

function dayOf(day: string, workstreams: WorkstreamDto[], provider: "ready" | "missing" = "ready") {
  return {
    availability: "ready",
    day,
    provider,
    embedder: "ready",
    running: false,
    summaryTime: "18:00",
    summary: null,
    workstreams,
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

const ICON = "data:image/png;base64,iVBORw0KGgo=";

function serve(workstreamsByDay: Record<string, WorkstreamDto[]>, provider?: "ready" | "missing") {
  invokeMock.mockImplementation(async (command: string, args?: { request?: unknown }) => {
    const request = (args?.request ?? {}) as Record<string, unknown>;
    if (command === "activity_timeline") return timeline;
    if (command === "day_intelligence_day") {
      const day = String(request.day);
      return dayOf(day, workstreamsByDay[day] ?? [], provider);
    }
    if (command === "activity_app_icon") {
      return request.bundleId === "com.microsoft.VSCode" ? ICON : null;
    }
    if (command === "activity_timeline_session") {
      return { session, windows: [], textExcerpt: null };
    }
    return undefined;
  });
}

// A workstream resumed after a long pause has one card per stretch: this is its first.
const card = async (title: string) =>
  (await screen.findAllByRole("button", { name: new RegExp(title) }))[0];

describe("Today grid of workstreams", () => {
  beforeEach(() => {
    applyInterfaceLocale("en");
    serve({ [dayKey(today)]: [resumed, overlapping, legacy] });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
  });

  it("adapts to the scroller's usable height and preserves selection through resize", async () => {
    const callbacks = new Map<Element, Set<() => void>>();
    class ViewportObserver {
      constructor(private callback: ResizeObserverCallback) {}
      observe(element: Element) {
        const listeners = callbacks.get(element) ?? new Set<() => void>();
        listeners.add(() => this.callback([], this));
        callbacks.set(element, listeners);
      }
      unobserve(element: Element) {
        callbacks.delete(element);
      }
      disconnect() {}
    }
    vi.stubGlobal("ResizeObserver", ViewportObserver);
    const first = {
      ...overlapping,
      title: "Sequential first",
      hours: [
        {
          ...overlapping.hours[0],
          hour: hourKey(9, yesterday),
          periods: [{ startedAt: iso(9, 0, yesterday), endedAt: iso(9, 10, yesterday) }],
        },
      ],
    };
    const second = {
      ...overlapping,
      id: 4,
      title: "Sequential second",
      hours: [
        {
          ...overlapping.hours[0],
          hour: hourKey(9, yesterday),
          periods: [{ startedAt: iso(9, 30, yesterday), endedAt: iso(9, 40, yesterday) }],
        },
      ],
    };
    serve({ [dayKey(yesterday)]: [first, second] });
    render(<ActivityTimelineView />);
    fireEvent.click(screen.getByRole("button", { name: "Previous day" }));
    const firstButton = await card("Sequential first");
    const secondButton = await card("Sequential second");
    const scroller = firstButton.closest(".ws-timeline-scroller");
    if (!scroller) throw new Error("Calendar scroller missing");
    let height = 180;
    Object.defineProperty(scroller, "clientHeight", { configurable: true, get: () => height });
    act(() =>
      callbacks.get(scroller)?.forEach((callback) => {
        callback();
      }),
    );
    const smallHeight = Number.parseFloat(firstButton.style.height);
    fireEvent.click(firstButton);
    expect(screen.getByRole("region", { name: "Workstream details" })).toBeInTheDocument();
    height = 600;
    act(() =>
      callbacks.get(scroller)?.forEach((callback) => {
        callback();
      }),
    );
    expect(Number.parseFloat(firstButton.style.height)).toBeGreaterThan(smallHeight);
    expect(firstButton.style.left).toBe(secondButton.style.left);
    expect(
      Number.parseFloat(firstButton.style.top) + Number.parseFloat(firstButton.style.height),
    ).toBeLessThan(Number.parseFloat(secondButton.style.top));
    expect(firstButton).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("region", { name: "Workstream details" })).toBeInTheDocument();
    const stableHeight = firstButton.style.height;
    act(() => {
      for (let index = 0; index < 10; index++)
        callbacks.get(scroller)?.forEach((callback) => {
          callback();
        });
    });
    expect(firstButton.style.height).toBe(stableHeight);
  });

  it("splits a workstream resumed after a long pause into one card per stretch", async () => {
    render(<ActivityTimelineView />);
    await card("Timeline grid refactor");

    // 9:00 to 9:30, then 11:00 to 11:20: the 90-minute pause starts a new card.
    const [morning, later] = screen.getAllByRole("button", { name: /Timeline grid refactor/ });
    expect(screen.getAllByRole("button", { name: /Timeline grid refactor/ })).toHaveLength(2);
    expect(within(morning).getAllByTestId("workstream-sitting")).toHaveLength(1);
    expect(within(later).getAllByTestId("workstream-sitting")).toHaveLength(1);
    // Each card carries the active minutes of its own hours (24 + 14 = 38)
    // and its own range, whatever its drawn density.
    expect(morning.getAttribute("aria-label")).toBe(
      `Timeline grid refactor, ${formatDuration(24 * 60_000, "en")} active, ${formatTime(iso(9, 0))} - ${formatTime(iso(9, 30))}`,
    );
    expect(later.getAttribute("aria-label")).toBe(
      `Timeline grid refactor, ${formatDuration(14 * 60_000, "en")} active, ${formatTime(iso(11, 0))} - ${formatTime(iso(11, 20))}`,
    );
    // The header still counts workstreams, once each.
    expect(screen.getByText("3 workstreams")).toBeInTheDocument();
  });

  it("puts overlapping workstreams side by side so both stay selectable", async () => {
    render(<ActivityTimelineView />);
    const first = await card("Timeline grid refactor");
    const second = await card("Release review");

    expect(first.dataset.lanes).toBe("2");
    expect(first.dataset.lane).not.toBe(second.dataset.lane);
    fireEvent.click(second);
    expect(
      within(screen.getByRole("region", { name: "Workstream details" })).getByRole("heading", {
        name: "Release review",
      }),
    ).toBeInTheDocument();
  });

  it("shows a legacy record by hour, without invented minutes", async () => {
    render(<ActivityTimelineView />);
    const inbox = await card("Inbox triage");

    // Too short for the meta line: the dashed bar and its name say "by hour".
    expect(inbox.getAttribute("aria-label")).toContain("By hour");
    expect(inbox.querySelector("[data-approximate]")).not.toBeNull();
    expect(within(inbox).queryAllByTestId("workstream-sitting")).toHaveLength(0);
    expect(within(inbox).queryByText(/sitting/)).not.toBeInTheDocument();
    expect(inbox.textContent).not.toContain(formatTime(iso(14, 0)));
  });

  it("marks a partly located workstream by hour and lists its minutes without an exact time", async () => {
    serve({
      [dayKey(today)]: [
        {
          id: 5,
          title: "Partly located work",
          summary: "",
          minutes: 30,
          hours: [
            {
              hour: hourKey(10),
              minutes: 30,
              note: "Reviewed the plan",
              periods: [{ startedAt: iso(10, 10), endedAt: iso(10, 20) }],
              locatedMinutes: 10,
            },
          ],
        },
      ],
    });
    // A tall canvas grows the lone card enough for its meta line and badge.
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(903);
    render(<ActivityTimelineView />);
    const partial = await card("Partly located work");
    expect(within(partial).getByText("By hour")).toBeInTheDocument();

    fireEvent.click(partial);
    const detail = screen.getByRole("region", { name: "Workstream details" });
    expect(
      within(detail).getByText(
        `${formatDuration(20 * 60_000, "en")} in this hour without an exact time`,
      ),
    ).toBeVisible();
    // Unlocated minutes may sit anywhere in the hour: no break is claimed.
    expect(within(detail).queryByText(/Break/)).not.toBeInTheDocument();
  });

  it("opens the workstream in the side panel with sittings, breaks and what was done, then goes back", async () => {
    render(<ActivityTimelineView />);
    const grid = await card("Timeline grid refactor");
    expect(screen.getByText("Top apps")).toBeInTheDocument();

    fireEvent.click(grid);

    const detail = screen.getByRole("region", { name: "Workstream details" });
    expect(grid).toHaveAttribute("aria-pressed", "true");
    expect(within(detail).getByRole("heading", { name: "Timeline grid refactor" })).toBeVisible();
    expect(within(detail).getByText(`${formatDuration(38 * 60_000, "en")} active`)).toBeVisible();
    expect(within(detail).getAllByTestId("workstream-period")).toHaveLength(2);
    expect(within(detail).getByText(`Break · ${formatDuration(90 * 60_000, "en")}`)).toBeVisible();
    // Session bounds say where the work was, not how long it took: the
    // 9:00 to 9:30 session is not shown as 30 minutes of this workstream.
    for (const period of within(detail).getAllByTestId("workstream-period")) {
      expect(period.textContent).not.toContain(formatDuration(30 * 60_000, "en"));
      expect(period.textContent).not.toContain(formatDuration(20 * 60_000, "en"));
    }
    expect(within(detail).getByText("Drew the hour axis")).toBeVisible();
    expect(within(detail).getByText("Placed the cards in lanes")).toBeVisible();
    expect(screen.queryByText("Top apps")).not.toBeInTheDocument();

    fireEvent.click(within(detail).getByRole("button", { name: "Back to the day overview" }));
    expect(screen.queryByRole("region", { name: "Workstream details" })).not.toBeInTheDocument();
    expect(screen.getByText("Top apps")).toBeInTheDocument();

    fireEvent.click(grid);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("region", { name: "Workstream details" })).not.toBeInTheDocument();
  });

  it("drops the selection when the day changes and reads the other day's workstreams", async () => {
    serve({
      [dayKey(today)]: [resumed],
      [dayKey(yesterday)]: [
        {
          ...legacy,
          id: 9,
          title: "Yesterday planning",
          hours: [{ ...legacy.hours[0], hour: hourKey(14, yesterday) }],
        },
      ],
    });
    render(<ActivityTimelineView />);
    fireEvent.click(await card("Timeline grid refactor"));
    expect(screen.getByRole("region", { name: "Workstream details" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Previous day" }));

    expect(await card("Yesterday planning")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Workstream details" })).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Timeline grid refactor/ }),
    ).not.toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("day_intelligence_day", {
      request: { day: dayKey(yesterday) },
    });
  });

  it("keeps the captured sessions under Sessions when no workstream exists yet", async () => {
    serve({}, "missing");
    render(<ActivityTimelineView />);

    expect(await screen.findByText(/Workstreams need an activity provider/)).toBeInTheDocument();
    expect(screen.queryByText("DayGrid.tsx - os-clovy")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sessions" }));
    fireEvent.click(await screen.findByText("DayGrid.tsx - os-clovy"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("activity_timeline_session", {
        request: { id: 101 },
      }),
    );
  });

  it("shows each top app with its own local icon and a neutral fallback when none is readable", async () => {
    render(<ActivityTimelineView />);
    const list = await screen.findByRole("list", { name: "Top apps" });
    const [code, removed] = within(list).getAllByRole("listitem");

    // The icon is decorative (alt=""): the app's name is written beside it.
    await waitFor(() => expect(code.querySelector("img")).toHaveAttribute("src", ICON));
    expect(within(code).getByText("Visual Studio Code")).toBeInTheDocument();
    // Identity, not name: only the bundle id that has an icon gets one.
    await act(async () => {});
    expect(removed.querySelector("img")).toBeNull();
    expect(within(removed).getByTestId("app-icon-fallback")).toBeInTheDocument();
    expect(within(removed).getByText("Removed App")).toBeInTheDocument();
    expect(within(removed).getByText(formatDuration(20 * 60_000, "en"))).toBeInTheDocument();
  });

  it("shows '< 1 min' for a 40s workstream and includes exact seconds in the total (40s + 40s = 1 min)", async () => {
    applyInterfaceLocale("pt-BR");
    const ws1: WorkstreamDto = {
      id: 10,
      title: "Tarefa curta A",
      summary: "Primeira tarefa",
      minutes: 0,
      seconds: 40,
      hours: [
        {
          hour: hourKey(10),
          minutes: 0,
          seconds: 40,
          locatedMinutes: 0,
          locatedSeconds: 40,
          note: "A",
          periods: [{ startedAt: iso(10, 0), endedAt: iso(10, 1) }],
        },
      ],
    };
    const ws2: WorkstreamDto = {
      id: 11,
      title: "Tarefa curta B",
      summary: "Segunda tarefa",
      minutes: 0,
      seconds: 40,
      hours: [
        {
          hour: hourKey(10),
          minutes: 0,
          seconds: 40,
          locatedMinutes: 0,
          locatedSeconds: 40,
          note: "B",
          periods: [{ startedAt: iso(10, 5), endedAt: iso(10, 6) }],
        },
      ],
    };
    serve({ [dayKey(today)]: [ws1, ws2] });
    render(<ActivityTimelineView />);

    const cardA = await card("Tarefa curta A");
    const cardB = await card("Tarefa curta B");

    // Both cards include "< 1 min ativos" in aria-label, never "0 min"
    expect(cardA.getAttribute("aria-label")).toContain("< 1 min ativos");
    expect(cardB.getAttribute("aria-label")).toContain("< 1 min ativos");
    expect(cardA.getAttribute("aria-label")).not.toContain("0 min");
    expect(cardB.getAttribute("aria-label")).not.toContain("0 min");

    // Opening detail displays the visible duration "< 1 min ativos", never "0 min"
    fireEvent.click(cardA);
    const detail = screen.getByRole("region", { name: /Detalhes da frente/ });
    expect(within(detail).getByText("< 1 min ativos")).toBeVisible();
    expect(within(detail).queryByText("0 min")).not.toBeInTheDocument();

    // Total combines 40s + 40s = 80s -> rounds to 1 min ("1 min ativos"), not "0 min"
    expect(screen.getByText("1 min ativos")).toBeInTheDocument();
  });
});
// A day with captured sessions and no workstream yet keeps the grid's hour
// rail, empty, under the notice that says why; the session list stays under
// Sessions.
describe("Today grid without workstreams", () => {
  beforeEach(() => {
    applyInterfaceLocale("en");
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  const expectEmptyGrid = async () => {
    const grid = await screen.findByRole("region", { name: "Workstreams of the day" });
    expect(within(grid).getByTestId("timeline-canvas")).toBeInTheDocument();
    expect(within(grid).getAllByTestId("timeline-hour").length).toBeGreaterThan(0);
    expect(within(grid).queryAllByRole("button")).toHaveLength(0);
    expect(screen.queryByText("DayGrid.tsx - os-clovy")).not.toBeInTheDocument();
    expect(document.querySelector(".timeline-session-list")).toBeNull();
  };

  it("shows the empty grid while the hours wait to be summarized", async () => {
    serve({});
    render(<ActivityTimelineView />);

    expect(await screen.findByText(/Workstreams appear after each hour ends/)).toBeInTheDocument();
    await expectEmptyGrid();
    // Today keeps the current hour on the rail.
    expect(
      screen.getAllByTestId("timeline-hour").some((hour) => hour.dataset.current === "true"),
    ).toBe(true);
  });

  it("shows the empty grid with the provider action when no provider is set", async () => {
    serve({}, "missing");
    const onNavigateToSettings = vi.fn();
    render(<ActivityTimelineView onNavigateToSettings={onNavigateToSettings} />);

    expect(await screen.findByText(/Workstreams need an activity provider/)).toBeInTheDocument();
    await expectEmptyGrid();
    fireEvent.click(screen.getByRole("button", { name: "Choose a provider" }));
    expect(onNavigateToSettings).toHaveBeenCalledWith("models");
  });

  it("shows the empty grid under the load error", async () => {
    serve({});
    const served = invokeMock.getMockImplementation();
    invokeMock.mockImplementation(async (command: string, args?: unknown) => {
      if (command === "day_intelligence_day") throw new Error("database locked");
      return served?.(command, args);
    });
    render(<ActivityTimelineView />);

    expect(
      await screen.findByText("Could not load workstreams: database locked"),
    ).toBeInTheDocument();
    await expectEmptyGrid();
  });
});
