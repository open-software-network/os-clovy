import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: (command: string, args?: unknown) =>
    args !== undefined ? invokeMock(command, args) : invokeMock(command),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => undefined),
}));

import { ActivityTimelineView } from "../components/activity-timeline/ActivityTimelineView";
import { DaySummaryPanel } from "../components/activity-timeline/DaySummaryPanel";
import { DaySummarySettingsSection } from "../components/settings/DaySummarySettingsSection";
import { applyInterfaceLocale } from "../i18n/locale";
import { DEFAULT_ACTIVITY_SETTINGS } from "../lib/activity-capture";
import type { ActivityTimelineDto } from "../lib/activity-timeline";
import { type DayIntelligenceDto, requestTodayOpen } from "../lib/day-intelligence";

const DAY = "2026-10-04";

const readyDay: DayIntelligenceDto = {
  availability: "ready",
  day: DAY,
  provider: "ready",
  embedder: "ready",
  running: false,
  summaryTime: "18:00",
  summary: {
    day: DAY,
    headline: "A day focused on the login fix",
    narrative: "Fixed the login redirect and joined the Weekly sync meeting.",
    insights: [{ title: "Long focus", text: "Most of the day went into one workstream." }],
    standup: {
      done: ["Fixed the login redirect (KAN-123)", "Reviewed the pagination PR"],
      inProgress: ["Release notes"],
      blockers: [],
    },
    hoursCovered: 2,
    locale: "en",
    provider: "cli:claude",
    trigger: "manual",
    generatedAt: "2026-10-04T21:00:00.000000Z",
  },
  workstreams: [
    {
      id: 1,
      title: "Login fix",
      summary: "Planned and fixed the login redirect.",
      minutes: 72,
      hours: [
        {
          hour: "2026-10-04T13",
          minutes: 30,
          note: "Planned the fix",
          periods: null,
          locatedMinutes: 0,
        },
        {
          hour: "2026-10-04T14",
          minutes: 42,
          note: "Fixed the redirect",
          periods: [
            { startedAt: "2026-10-04T17:00:00.000000Z", endedAt: "2026-10-04T17:42:00.000000Z" },
          ],
          locatedMinutes: 42,
        },
      ],
    },
  ],
  hourReports: [
    {
      hour: "2026-10-04T14",
      startedAt: "2026-10-04T17:00:00.000000Z",
      endedAt: "2026-10-04T18:00:00.000000Z",
      activeMinutes: 42,
      summary: "Fixed the login redirect and reviewed a pull request.",
      activities: [
        { description: "Fixed the login redirect for KAN-123", minutes: 21, periods: null },
        { description: "Reviewed the pagination pull request", minutes: 21, periods: null },
      ],
      provider: "cli:claude",
      generatedAt: "2026-10-04T18:01:00.000000Z",
    },
  ],
  panels: {
    focusedMs: 72 * 60_000,
    idleMs: 0,
    awayMs: 4 * 3600_000,
    categories: [{ category: "coding", durationMs: 72 * 60_000 }],
    topApps: [{ appName: "Zed", bundleId: "dev.zed.Zed", durationMs: 42 * 60_000 }],
    hours: Array.from({ length: 24 }, (_, hour) => ({
      hour: `${DAY}T${String(hour).padStart(2, "0")}`,
      focusedMs: hour === 13 ? 30 * 60_000 : hour === 14 ? 42 * 60_000 : 0,
    })),
    workstreams: [{ id: 1, title: "Login fix", minutes: 72 }],
    meetingCount: 1,
    meetingMs: 30 * 60_000,
    codingAgentBlocks: 2,
    codingAgentActiveSeconds: 25 * 60,
  },
};

const emptyTimeline: ActivityTimelineDto = {
  availability: "ready",
  message: null,
  captureEnabled: true,
  sessions: [],
  gaps: [],
  stats: { focusedMs: 0, idleMs: 0, awayMs: 0, topApps: [], categories: [] },
};

const writeText = vi.fn(async () => undefined);

describe("Day summary", () => {
  beforeEach(() => {
    applyInterfaceLocale("en");
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "day_intelligence_day") return readyDay;
      if (command === "activity_timeline") return emptyTimeline;
      if (command === "coding_agents_blocks") return [];
      return undefined;
    });
  });

  afterEach(() => {
    cleanup();
    applyInterfaceLocale("en");
    vi.clearAllMocks();
  });

  it("shows the summary, standup, panels from the data, workstreams, and hour reports", async () => {
    render(<DaySummaryPanel day={DAY} />);

    expect(await screen.findByText("A day focused on the login fix")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("day_intelligence_day", { request: { day: DAY } });
    expect(screen.getByText(/joined the Weekly sync meeting/)).toBeInTheDocument();
    expect(screen.getByText("Fixed the login redirect (KAN-123)")).toBeInTheDocument();
    expect(screen.getByText(/Generated at .* by claude/)).toBeInTheDocument();

    const focused = document.querySelector('[data-panel="focused"]') as HTMLElement;
    expect(within(focused).getByText("1h 12m")).toBeInTheDocument();
    const meetings = document.querySelector('[data-panel="meetings"]') as HTMLElement;
    expect(within(meetings).getByText("30m")).toBeInTheDocument();
    expect(within(meetings).getByText("1 meeting")).toBeInTheDocument();
    const agents = document.querySelector('[data-panel="codingAgents"]') as HTMLElement;
    expect(within(agents).getByText("2 sessions")).toBeInTheDocument();

    const report = document.querySelector('[data-hour="2026-10-04T14"]') as HTMLElement;
    expect(within(report).getByText("42m")).toBeInTheDocument();
    expect(within(report).getAllByText("21m")).toHaveLength(2);
    expect(screen.getByText("13:00 · 30m")).toBeInTheDocument();
  });

  it("copies the standup as bullet lists in one click", async () => {
    render(<DaySummaryPanel day={DAY} />);
    fireEvent.click(await screen.findByRole("button", { name: "Copy standup" }));

    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    expect(writeText).toHaveBeenCalledWith(
      [
        "Done:",
        "- Fixed the login redirect (KAN-123)",
        "- Reviewed the pagination PR",
        "",
        "In progress:",
        "- Release notes",
        "",
        "Blockers:",
        "- None",
      ].join("\n"),
    );
    expect(await screen.findByRole("button", { name: "Standup copied" })).toBeInTheDocument();
  });

  it("says plainly that it is off without an activity provider", async () => {
    invokeMock.mockImplementation(async () => ({
      ...readyDay,
      provider: "missing",
      summary: null,
      workstreams: [],
      hourReports: [],
    }));
    const onNavigateToSettings = vi.fn();
    render(<DaySummaryPanel day={DAY} onNavigateToSettings={onNavigateToSettings} />);

    expect(await screen.findByText("Day summaries need an activity provider")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Generate summary" })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Choose a provider" }));
    expect(onNavigateToSettings).toHaveBeenCalledWith("models");
  });

  it("generates on demand and shows the new summary", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "day_intelligence_day") return { ...readyDay, summary: null };
      if (command === "day_intelligence_generate") return readyDay;
      return undefined;
    });
    render(<DaySummaryPanel day={DAY} />);

    expect(await screen.findByText(/It is generated at 18:00/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Generate summary" }));
    expect(await screen.findByText("A day focused on the login fix")).toBeInTheDocument();
    expect(invokeMock).toHaveBeenCalledWith("day_intelligence_generate", {
      request: { day: DAY },
    });
  });

  it("shows a generation failure", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "day_intelligence_day") return { ...readyDay, summary: null };
      throw { code: "llm_timeout", message: "The provider did not answer in time." };
    });
    render(<DaySummaryPanel day={DAY} />);
    fireEvent.click(await screen.findByRole("button", { name: "Generate summary" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not generate the summary: The provider did not answer in time.",
    );
  });

  it("says in the interface language that the day is not ready when the backend refuses", async () => {
    applyInterfaceLocale("pt-BR");
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "day_intelligence_day") return { ...readyDay, summary: null };
      throw {
        code: "day_summary_incomplete",
        message: "The summary was not written because part of the day is not ready.",
      };
    });
    render(<DaySummaryPanel day={DAY} />);
    fireEvent.click(await screen.findByRole("button", { name: "Gerar resumo" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Parte deste dia ainda está sendo processada, então o resumo não foi escrito.",
    );
  });

  it("shows a failed first read with a retry instead of loading forever", async () => {
    let fail = true;
    invokeMock.mockImplementation(async (command: string) => {
      if (command !== "day_intelligence_day") return undefined;
      if (fail) throw { code: "activity_database_closed", message: "The database is closed." };
      return readyDay;
    });
    render(<DaySummaryPanel day={DAY} />);

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not load the day summary: The database is closed.",
    );
    fail = false;
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("A day focused on the login fix")).toBeInTheDocument();
  });

  it("keeps the selected day when the previous day's read answers last", async () => {
    let answerPrevious: (value: DayIntelligenceDto) => void = () => {};
    invokeMock.mockImplementation(async (command: string, args?: { request: { day: string } }) => {
      if (command !== "day_intelligence_day") return undefined;
      if (args?.request.day === "2026-10-03") {
        return new Promise<DayIntelligenceDto>((resolve) => {
          answerPrevious = resolve;
        });
      }
      return readyDay;
    });
    const { rerender } = render(<DaySummaryPanel day="2026-10-03" />);
    rerender(<DaySummaryPanel day={DAY} />);
    expect(await screen.findByText("A day focused on the login fix")).toBeInTheDocument();

    await act(async () =>
      answerPrevious({
        ...readyDay,
        day: "2026-10-03",
        summary: {
          ...(readyDay.summary as NonNullable<DayIntelligenceDto["summary"]>),
          day: "2026-10-03",
          headline: "The previous day",
        },
      }),
    );
    expect(screen.queryByText("The previous day")).not.toBeInTheDocument();
    expect(screen.getByText("A day focused on the login fix")).toBeInTheDocument();
  });

  it("reads in Portuguese and copies the standup with Portuguese headings", async () => {
    applyInterfaceLocale("pt-BR");
    render(<DaySummaryPanel day={DAY} />);

    expect(await screen.findByRole("heading", { name: "Resumo do dia" })).toBeInTheDocument();
    expect(screen.getByText("Em andamento")).toBeInTheDocument();
    expect(screen.getByText("Frentes de trabalho")).toBeInTheDocument();
    const report = document.querySelector('[data-hour="2026-10-04T14"]') as HTMLElement;
    expect(within(report).getByText("42 min")).toBeInTheDocument();
    expect(within(report).getAllByText("21 min")).toHaveLength(2);
    expect(screen.getByText("13:00 · 30 min")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Copiar standup" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const copied = (writeText.mock.calls[0] as unknown as [string])[0];
    expect(copied).toContain("Feito:\n- Fixed the login redirect (KAN-123)");
    expect(copied).toContain("Bloqueios:\n- Nenhum");
  });

  it("opens the summary of the day a notification asked for", async () => {
    requestTodayOpen("2026-10-03");
    render(<ActivityTimelineView />);

    expect(await screen.findByRole("tab", { name: "Day summary" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("day_intelligence_day", {
        request: { day: "2026-10-03" },
      }),
    );

    // A later click while the view is open switches to that day's summary.
    fireEvent.click(screen.getByRole("tab", { name: "Timeline" }));
    act(() => requestTodayOpen("2026-10-02"));
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("day_intelligence_day", {
        request: { day: "2026-10-02" },
      }),
    );
    expect(screen.getByRole("tab", { name: "Day summary" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("shows '< 1 min' for a 40s activity and never '0 min' in DaySummaryPanel", async () => {
    applyInterfaceLocale("pt-BR");
    const dayWithShortActivity: DayIntelligenceDto = {
      ...readyDay,
      day: DAY,
      workstreams: [
        {
          id: 1,
          title: "Frente rápida",
          summary: "Ajuste rápido",
          minutes: 0,
          seconds: 40,
          hours: [
            {
              hour: "2026-10-04T15",
              minutes: 0,
              seconds: 40,
              note: "40s de trabalho",
              periods: null,
              locatedMinutes: 0,
              locatedSeconds: 0,
            },
          ],
        },
      ],
      hourReports: [
        {
          hour: "2026-10-04T15",
          startedAt: "2026-10-04T18:00:00.000000Z",
          endedAt: "2026-10-04T19:00:00.000000Z",
          activeMinutes: 1,
          summary: "Ajuste rápido em segundos.",
          activities: [
            {
              description: "Ajuste de 40 segundos",
              minutes: 0,
              seconds: 40,
              periods: null,
            },
          ],
          provider: "cli:claude",
          generatedAt: "2026-10-04T19:01:00.000000Z",
        },
      ],
    };
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "day_intelligence_day") return dayWithShortActivity;
      if (command === "activity_timeline") return emptyTimeline;
      if (command === "coding_agents_blocks") return [];
      return undefined;
    });

    render(<DaySummaryPanel day={DAY} />);

    const report = (await screen.findByText("Ajuste rápido em segundos.")).closest(
      ".day-hour-report",
    ) as HTMLElement;
    expect(report).toBeInTheDocument();

    // The activity with 40s displays "< 1 min" and NEVER "0 min"
    const activityMinutes = report.querySelector(".day-hour-activity-minutes") as HTMLElement;
    expect(activityMinutes).toHaveTextContent("< 1 min");
    expect(report).not.toHaveTextContent("0 min");

    // Both the workstream duration and its hour display "< 1 min", never "0 min"
    const ws = screen.getByText("Frente rápida").closest(".day-workstream") as HTMLElement;
    const wsMinutes = ws.querySelector(".day-workstream-minutes") as HTMLElement;
    const wsHour = ws.querySelector(".day-workstream-hour") as HTMLElement;
    expect(wsMinutes).toHaveTextContent("< 1 min");
    expect(wsHour).toHaveTextContent("< 1 min");
    expect(ws).not.toHaveTextContent("0 min");
  });
});

describe("Day summary settings", () => {
  afterEach(() => {
    cleanup();
    applyInterfaceLocale("en");
  });

  it("saves the summary time, the notification switch, and quiet hours", () => {
    const onSave = vi.fn();
    const settings = {
      ...DEFAULT_ACTIVITY_SETTINGS,
      notifications: {
        enabled: true,
        quietHours: { enabled: true, start: "22:00", end: "08:00" },
      },
    };
    render(<DaySummarySettingsSection settings={settings} saving={false} onSave={onSave} />);

    fireEvent.change(screen.getByLabelText("Summary time"), { target: { value: "17:30" } });
    expect(onSave).toHaveBeenLastCalledWith({ ...settings, daySummary: { time: "17:30" } });

    fireEvent.change(screen.getByLabelText("Quiet until"), { target: { value: "07:00" } });
    expect(onSave).toHaveBeenLastCalledWith({
      ...settings,
      notifications: {
        enabled: true,
        quietHours: { enabled: true, start: "22:00", end: "07:00" },
      },
    });

    fireEvent.click(screen.getByRole("switch", { name: "Notifications" }));
    expect(onSave).toHaveBeenLastCalledWith({
      ...settings,
      notifications: { ...settings.notifications, enabled: false },
    });
  });
});
