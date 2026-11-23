import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type * as TauriCore from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

// The "Today" view mounts the coding-agent lane (EXTRA_TIMELINE_LANES), which
// reads its own blocks; these tests cover the view, so that lane stays empty.
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
import {
  dayRange,
  type ActivityTimelineDto,
  type ActivityTimelineSearchDto,
  type TimelineSessionDetailDto,
  type TimelineSessionDto,
} from "../lib/activity-timeline";

const now = new Date();
const todayIso = now.toISOString();
const oneHourAgoIso = new Date(now.getTime() - 3600_000).toISOString();
const twoHoursAgoIso = new Date(now.getTime() - 7200_000).toISOString();
const threeHoursAgoIso = new Date(now.getTime() - 10800_000).toISOString();

const mockSessions: TimelineSessionDto[] = [
  {
    id: 101,
    appName: "Visual Studio Code",
    bundleId: "com.microsoft.VSCode",
    contextKind: "workspace",
    context: "os-clovy",
    startedAt: threeHoursAgoIso,
    endedAt: twoHoursAgoIso,
    durationMs: 3600_000, // 1h
    active: false,
    category: "coding",
    confidence: 0.95,
    windowTitle: "ActivityTimelineView.tsx - os-clovy",
  },
  {
    id: 102,
    appName: "Google Chrome",
    bundleId: "com.google.Chrome",
    contextKind: "domain",
    context: "github.com",
    startedAt: oneHourAgoIso,
    endedAt: todayIso,
    durationMs: 3600_000, // 1h
    active: true,
    category: "research",
    confidence: 0.88,
    windowTitle: "PR #42: Activity Timeline - GitHub",
  },
];

const mockReadyTimeline: ActivityTimelineDto = {
  availability: "ready",
  message: null,
  captureEnabled: true,
  sessions: mockSessions,
  gaps: [
    {
      id: 201,
      startedAt: twoHoursAgoIso,
      endedAt: oneHourAgoIso,
      durationMs: 3600_000, // 1h
      kind: "idle",
      pauseReason: null,
      ongoing: false,
    },
    {
      id: 0,
      startedAt: oneHourAgoIso,
      endedAt: todayIso,
      durationMs: 3600_000,
      kind: "paused",
      pauseReason: "protectedVideo",
      ongoing: true,
    },
  ],
  stats: {
    focusedMs: 7200_000, // 2h
    idleMs: 3600_000, // 1h
    awayMs: 0,
    topApps: [
      { appName: "Visual Studio Code", bundleId: "com.microsoft.VSCode", durationMs: 3600_000 },
      { appName: "Google Chrome", bundleId: "com.google.Chrome", durationMs: 3600_000 },
    ],
    categories: [
      { category: "coding", durationMs: 3600_000 },
      { category: "research", durationMs: 3600_000 },
    ],
  },
};

const mockNeverEnabledTimeline: ActivityTimelineDto = {
  availability: "neverEnabled",
  message: null,
  captureEnabled: false,
  sessions: [],
  gaps: [],
  stats: {
    focusedMs: 0,
    idleMs: 0,
    awayMs: 0,
    topApps: [],
    categories: [],
  },
};

const mockSessionDetail: TimelineSessionDetailDto = {
  session: mockSessions[0],
  windows: [
    {
      windowTitle: "ActivityTimelineView.tsx - os-clovy",
      browserUrl: null,
      firstSeenAt: threeHoursAgoIso,
      lastSeenAt: twoHoursAgoIso,
      frameCount: 42,
    },
    {
      windowTitle: "TypeScript Documentation",
      browserUrl: "https://www.typescriptlang.org/docs",
      firstSeenAt: threeHoursAgoIso,
      lastSeenAt: twoHoursAgoIso,
      frameCount: 18,
    },
  ],
  textExcerpt: "export function ActivityTimelineView() { return <main>...</main>; }",
};

const mockSearchResults: ActivityTimelineSearchDto = {
  availability: "ready",
  results: [
    {
      sessionId: 101,
      appName: "Visual Studio Code",
      windowTitle: "ActivityTimelineView.tsx - os-clovy",
      browserUrl: null,
      seenAt: threeHoursAgoIso,
      snippet: "export function ActivityTimelineView…",
    },
  ],
};

describe("ActivityTimelineView", () => {
  beforeEach(() => {
    applyInterfaceLocale("en");
    invokeMock.mockImplementation(async (command: string, _args?: { request?: unknown }) => {
      if (command === "activity_timeline") {
        return mockReadyTimeline;
      }
      if (command === "activity_timeline_session") {
        return mockSessionDetail;
      }
      if (command === "activity_timeline_search") {
        return mockSearchResults;
      }
      return undefined;
    });
  });

  afterEach(() => {
    cleanup();
    applyInterfaceLocale("en");
    vi.clearAllMocks();
  });

  it("renders empty state when neverEnabled and turn-on button calls onNavigateToSettings", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "activity_timeline") {
        return mockNeverEnabledTimeline;
      }
      return undefined;
    });

    const onNavigateToSettings = vi.fn();
    render(<ActivityTimelineView onNavigateToSettings={onNavigateToSettings} />);

    expect(await screen.findByText("Activity timeline")).toBeInTheDocument();
    expect(
      screen.getByText(/Text-only activity capture records window titles and visible text/),
    ).toBeInTheDocument();

    const turnOnBtn = screen.getByRole("button", { name: "Turn on in Settings" });
    expect(turnOnBtn).toBeInTheDocument();

    fireEvent.click(turnOnBtn);
    expect(onNavigateToSettings).toHaveBeenCalledTimes(1);
  });

  it("renders sessions, idle gap, active session, and stats matching the DTO", async () => {
    render(<ActivityTimelineView />);
    // The captured sessions live under Sessions; Workstreams shows the grid.
    fireEvent.click(await screen.findByRole("button", { name: "Sessions" }));

    // Sessions and gap rows in list
    const vsCodeElements = await screen.findAllByText("Visual Studio Code");
    expect(vsCodeElements.length).toBeGreaterThanOrEqual(1);
    const chromeElements = screen.getAllByText("Google Chrome");
    expect(chromeElements.length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("os-clovy")).toBeInTheDocument();
    expect(screen.getByText("github.com")).toBeInTheDocument();
    expect(screen.getAllByText("Idle").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Paused (protected video)").length).toBeGreaterThan(0);

    // Active session live badge
    const liveBadges = screen.getAllByText("Live");
    expect(liveBadges.length).toBeGreaterThan(0);

    // Categories
    expect(screen.getAllByText("Coding").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Research").length).toBeGreaterThan(0);

    // Stats numbers
    expect(screen.getByText("Focused")).toBeInTheDocument();
    expect(screen.getByText("Away")).toBeInTheDocument();
    expect(screen.getByText("Top apps")).toBeInTheDocument();
    expect(screen.getByText("Categories")).toBeInTheDocument();
  });

  it("navigates previous and next days, calling activity_timeline with updated day bounds", async () => {
    render(<ActivityTimelineView />);

    await screen.findAllByText("Visual Studio Code");
    expect(invokeMock).toHaveBeenCalledWith("activity_timeline", expect.any(Object));

    const initialCallCount = invokeMock.mock.calls.length;

    // Next day button should be disabled when on today
    const nextBtn = screen.getByRole("button", { name: "Next day" });
    expect(nextBtn).toBeDisabled();

    // Click previous day
    const prevBtn = screen.getByRole("button", { name: "Previous day" });
    fireEvent.click(prevBtn);

    await waitFor(() => {
      expect(invokeMock.mock.calls.length).toBeGreaterThan(initialCallCount);
    });

    // Next day button should now be enabled
    expect(nextBtn).not.toBeDisabled();

    // Jump to today button should appear and return to today
    const todayBtn = screen.getByText("Today", { selector: ".timeline-jump-today-btn" });
    fireEvent.click(todayBtn);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "Next day" })).toBeDisabled();
    });
  });

  it("clicking a session loads and shows session detail with window titles, URL, and text excerpt", async () => {
    render(<ActivityTimelineView />);
    // The captured sessions live under Sessions; Workstreams shows the grid.
    fireEvent.click(await screen.findByRole("button", { name: "Sessions" }));

    await screen.findAllByText("Visual Studio Code");
    const sessionBtn = screen.getByText("ActivityTimelineView.tsx - os-clovy");
    fireEvent.click(sessionBtn);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("activity_timeline_session", {
        request: { id: 101 },
      });
    });

    // Detail panel displays windows
    expect(await screen.findByText("Windows")).toBeInTheDocument();
    expect(screen.getByText("https://www.typescriptlang.org/docs")).toBeInTheDocument();
    expect(screen.getByText("42 frames")).toBeInTheDocument();

    // Text excerpt
    expect(screen.getByText("Text excerpt")).toBeInTheDocument();
    expect(
      screen.getByText("export function ActivityTimelineView() { return <main>...</main>; }"),
    ).toBeInTheDocument();

    // Close detail panel
    const closeBtn = screen.getByRole("button", { name: "Close details" });
    fireEvent.click(closeBtn);

    await waitFor(() => {
      expect(screen.queryByText("Text excerpt")).not.toBeInTheDocument();
    });
  });

  it("searches activity and clicking a result navigates to that day and opens session detail", async () => {
    const user = userEvent.setup();
    render(<ActivityTimelineView />);

    await screen.findAllByText("Visual Studio Code");

    const searchInput = screen.getByRole("searchbox", {
      name: "Search activity, apps, and window titles",
    });
    await user.type(searchInput, "ActivityTimeline");

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("activity_timeline_search", expect.any(Object));
    });

    // Search result item displayed
    const resultItem = await screen.findByText("export function ActivityTimelineView…");
    expect(resultItem).toBeInTheDocument();

    // Clicking search result navigates to that session detail
    fireEvent.click(resultItem);

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("activity_timeline_session", {
        request: { id: 101 },
      });
    });

    expect(await screen.findByText("Text excerpt")).toBeInTheDocument();
    // The results popover gets out of the way of the opened detail.
    expect(screen.queryByText("export function ActivityTimelineView…")).not.toBeInTheDocument();
  });

  it("renders pt-BR labels when Portuguese is selected", async () => {
    applyInterfaceLocale("pt-BR");
    render(<ActivityTimelineView />);

    // Header title for today
    expect(await screen.findByRole("heading", { name: "Hoje" })).toBeInTheDocument();
    fireEvent.click(await screen.findByRole("button", { name: "Sessões" }));

    // Metrics in pt-BR
    expect(screen.getByText("Focado")).toBeInTheDocument();
    const idleElements = screen.getAllByText("Ocioso");
    expect(idleElements.length).toBeGreaterThan(0);
    expect(screen.getByText("Ausente")).toBeInTheDocument();
    expect(screen.getByText("Principais apps")).toBeInTheDocument();
    expect(screen.getByText("Categorias")).toBeInTheDocument();

    // Live badge in pt-BR
    const liveBadges = screen.getAllByText("Ao vivo");
    expect(liveBadges.length).toBeGreaterThan(0);

    // Category names in pt-BR
    expect(screen.getAllByText("Programação").length).toBeGreaterThan(0);
    expect(screen.getAllByText("Pesquisa").length).toBeGreaterThan(0);
  });

  it("renders non-crashing notice when keyMissing or error occurs", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "activity_timeline") {
        return {
          ...mockReadyTimeline,
          availability: "keyMissing",
        };
      }
      return undefined;
    });

    const onNavigateToSettings = vi.fn();
    render(<ActivityTimelineView onNavigateToSettings={onNavigateToSettings} />);

    expect(await screen.findByText(/Encryption key is missing from Keychain/)).toBeInTheDocument();

    const openSettingsBtn = screen.getByRole("button", { name: "Open Settings" });
    fireEvent.click(openSettingsBtn);
    expect(onNavigateToSettings).toHaveBeenCalledTimes(1);
  });

  it("renders quiet empty-day message when ready with no sessions for the day", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "activity_timeline") {
        return {
          ...mockReadyTimeline,
          sessions: [],
          gaps: [],
        };
      }
      return undefined;
    });

    render(<ActivityTimelineView />);
    expect(await screen.findByText("No activity recorded for this day.")).toBeInTheDocument();
  });

  it("renders hint banner when captureEnabled is false on a ready timeline", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "activity_timeline") {
        return {
          ...mockReadyTimeline,
          captureEnabled: false,
        };
      }
      return undefined;
    });

    render(<ActivityTimelineView />);
    expect(await screen.findByText("Activity capture is currently off.")).toBeInTheDocument();
  });

  it("handles session detail error inline without crashing", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "activity_timeline") {
        return mockReadyTimeline;
      }
      if (command === "activity_timeline_session") {
        throw new Error("Session expired or removed by retention");
      }
      return undefined;
    });

    render(<ActivityTimelineView />);
    // The captured sessions live under Sessions; Workstreams shows the grid.
    fireEvent.click(await screen.findByRole("button", { name: "Sessions" }));
    await screen.findAllByText("Visual Studio Code");

    const sessionBtn = screen.getByText("ActivityTimelineView.tsx - os-clovy");
    fireEvent.click(sessionBtn);

    expect(await screen.findByText("Session expired or removed by retention")).toBeInTheDocument();
  });

  it("computes local day bounds via dayRange", () => {
    const sampleDate = new Date(2026, 9, 4, 15, 30);
    const range = dayRange(sampleDate);
    const start = new Date(range.from);
    const end = new Date(range.to);

    expect(start.getHours()).toBe(0);
    expect(start.getMinutes()).toBe(0);
    expect(start.getSeconds()).toBe(0);
    expect(end.getHours()).toBe(0);
    expect(end.getDate() - start.getDate()).toBe(1);
  });

  it("renders unsupported state when availability is unsupported without capture-off banner or turn-on button", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "activity_timeline") {
        return {
          availability: "unsupported",
          message: null,
          captureEnabled: false,
          sessions: [],
          gaps: [],
          stats: { focusedMs: 0, idleMs: 0, awayMs: 0, topApps: [], categories: [] },
        };
      }
      return undefined;
    });

    const onNavigateToSettings = vi.fn();
    render(<ActivityTimelineView onNavigateToSettings={onNavigateToSettings} />);

    expect(
      await screen.findByRole("heading", { name: /not supported|não suportad/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/not supported on this system|não é suportada neste sistema/i),
    ).toBeInTheDocument();

    // MUST NOT display the capture paused/off banner
    expect(
      screen.queryByText(/capture is currently off|captura está desligada/i),
    ).not.toBeInTheDocument();

    // MUST NOT display the turn-on button
    expect(screen.queryByRole("button", { name: /turn on|ativar/i })).not.toBeInTheDocument();
  });

  it("resolves and renders Windows executable app icon from activity_app_icon, falling back to icon glyph when missing", async () => {
    const windowsExeBundleId = "C:\\Program Files\\Notepad++\\notepad++.exe";
    const base64Icon =
      "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";

    invokeMock.mockImplementation(async (command: string, args?: { request?: unknown }) => {
      if (command === "activity_timeline") {
        return {
          ...mockReadyTimeline,
          stats: {
            ...mockReadyTimeline.stats,
            topApps: [
              {
                appName: "Notepad++",
                bundleId: windowsExeBundleId,
                durationMs: 3600_000,
              },
              {
                appName: "UnknownApp",
                bundleId: "C:\\Tools\\unknown.exe",
                durationMs: 1800_000,
              },
            ],
          },
        };
      }
      if (command === "activity_app_icon") {
        const req = args?.request as { bundleId: string };
        if (req.bundleId === windowsExeBundleId) {
          return base64Icon;
        }
        return null;
      }
      return undefined;
    });

    const { container } = render(<ActivityTimelineView />);

    await screen.findByText("Notepad++");
    await waitFor(() => {
      const img = container.querySelector<HTMLImageElement>("img.app-icon");
      expect(img).not.toBeNull();
      expect(img?.src).toBe(base64Icon);
    });

    expect(screen.getByText("UnknownApp")).toBeInTheDocument();
    expect(container.querySelector(".app-icon-fallback")).toBeInTheDocument();

    expect(invokeMock).toHaveBeenCalledWith("activity_app_icon", {
      request: { bundleId: windowsExeBundleId },
    });
  });
});
