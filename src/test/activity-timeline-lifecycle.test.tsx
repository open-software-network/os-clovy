import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Profiler } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

// Each `listen` resolves only when the test says so, like a Tauri listener
// still registering when its effect is torn down.
const listeners = vi.hoisted(
  () => [] as { event: string; unlisten: () => void; resolve: () => void }[],
);

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: async (command: string, args?: unknown) => {
    if (command === "coding_agents_blocks") return [];
    return args !== undefined ? invokeMock(command, args) : invokeMock(command);
  },
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    (event: string) =>
      new Promise((resolve) => {
        const unlisten = vi.fn();
        listeners.push({ event, unlisten, resolve: () => resolve(unlisten) });
      }),
  ),
  emit: vi.fn(async () => undefined),
}));

import { ActivityTimelineView } from "../components/activity-timeline/ActivityTimelineView";
import { ActivitySettingsSection } from "../components/settings/ActivitySettingsSection";
import { applyInterfaceLocale } from "../i18n/locale";
import { ACTIVITY_STATE_EVENT } from "../lib/activity-capture";
import { ACTIVITY_TIMELINE_EVENT, type ActivityTimelineDto } from "../lib/activity-timeline";

const now = Date.now();
const readyTimeline: ActivityTimelineDto = {
  availability: "ready",
  message: null,
  captureEnabled: true,
  sessions: [
    {
      id: 101,
      appName: "Visual Studio Code",
      bundleId: "com.microsoft.VSCode",
      contextKind: "workspace",
      context: "os-clovy",
      startedAt: new Date(now - 7200_000).toISOString(),
      endedAt: new Date(now - 3600_000).toISOString(),
      durationMs: 3600_000,
      active: false,
      category: "coding",
      confidence: 0.95,
      windowTitle: "main.rs - os-clovy",
    },
  ],
  gaps: [],
  stats: { focusedMs: 3600_000, idleMs: 0, awayMs: 0, topApps: [], categories: [] },
};

async function settleListeners() {
  await act(async () => {
    for (const listener of listeners.splice(0)) listener.resolve();
    await Promise.resolve();
  });
}

describe("Today view lifecycle", () => {
  beforeEach(() => {
    applyInterfaceLocale("en");
    listeners.length = 0;
    invokeMock.mockImplementation(async (command: string) =>
      command === "activity_timeline" ? readyTimeline : undefined,
    );
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.clearAllMocks();
  });

  it("drops a timeline listener that registers after the view unmounted", async () => {
    const { unmount } = render(<ActivityTimelineView />);
    const registered = listeners.filter((entry) => entry.event === ACTIVITY_TIMELINE_EVENT);
    expect(registered.length).toBeGreaterThan(0);

    unmount();
    await settleListeners();

    for (const entry of registered) expect(entry.unlisten).toHaveBeenCalledTimes(1);
  });

  it("drops an activity state listener that registers after settings unmounted", async () => {
    const { unmount } = render(<ActivitySettingsSection />);
    const registered = listeners.filter((entry) => entry.event === ACTIVITY_STATE_EVENT);
    expect(registered).toHaveLength(1);

    unmount();
    await settleListeners();

    expect(registered[0].unlisten).toHaveBeenCalledTimes(1);
  });

  it("clears the previous day's sessions when the new day's read fails", async () => {
    render(<ActivityTimelineView />);
    fireEvent.click(await screen.findByRole("button", { name: "Sessions" }));
    expect(await screen.findAllByText("Visual Studio Code")).not.toHaveLength(0);

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "activity_timeline") throw new Error("database is locked");
      return undefined;
    });
    fireEvent.click(screen.getByRole("button", { name: "Previous day" }));

    expect(await screen.findByText(/database is locked/)).toBeInTheDocument();
    expect(screen.queryByText("Visual Studio Code")).not.toBeInTheDocument();
  });

  it("does not re-render a past day on the now tick", async () => {
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    let commits = 0;
    render(
      <Profiler id="today" onRender={() => commits++}>
        <ActivityTimelineView />
      </Profiler>,
    );
    fireEvent.click(await screen.findByRole("button", { name: "Previous day" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "Next day" })).toBeEnabled());
    fireEvent.click(screen.getByRole("button", { name: "Sessions" }));
    await screen.findAllByText("Visual Studio Code");

    const before = commits;
    act(() => {
      vi.advanceTimersByTime(60_000);
    });

    expect(commits).toBe(before);
  });
});
