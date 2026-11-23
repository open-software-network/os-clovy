import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type * as TauriCore from "@tauri-apps/api/core";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof TauriCore>()),
  invoke: (command: string, args?: unknown) =>
    args !== undefined ? invokeMock(command, args) : invokeMock(command),
}));

const listeners = new Map<string, () => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: () => void) => {
    listeners.set(event, handler);
    return () => listeners.delete(event);
  }),
  emit: vi.fn(async () => undefined),
}));

import { TimelineDayStrip } from "../components/activity-timeline/TimelineDayStrip";
import { ActivitySettingsSection } from "../components/settings/ActivitySettingsSection";
import { applyInterfaceLocale } from "../i18n/locale";
import { type ActivityStatusDto, DEFAULT_ACTIVITY_SETTINGS } from "../lib/activity-capture";
import {
  CODING_AGENTS_UPDATED_EVENT,
  type CodingAgentBlockDto,
  type CodingAgentsStatusDto,
} from "../lib/coding-agents";

const activityStatus: ActivityStatusDto = {
  supported: true,
  settings: { ...DEFAULT_ACTIVITY_SETTINGS, ignoredApps: ["Terminal"] },
  state: { kind: "off" },
  permissions: {
    accessibility: "granted",
    screenRecording: "granted",
    inputMonitoring: "granted",
  },
  manualPause: false,
  lastFrameAt: null,
  debugExportAvailable: false,
};

const agentsStatus: CodingAgentsStatusDto = {
  supported: true,
  sources: [
    { id: "claude_code", name: "Claude Code", enabled: false, present: true },
    { id: "codex", name: "Codex", enabled: false, present: true },
    { id: "copilot_cli", name: "Copilot CLI", enabled: false, present: false },
    { id: "copilot_vscode", name: "Copilot in VS Code", enabled: false, present: false },
    { id: "cursor", name: "Cursor", enabled: false, present: false },
    { id: "cursor_cli", name: "Cursor CLI", enabled: false, present: false },
    { id: "antigravity", name: "Antigravity", enabled: false, present: false },
  ],
  databaseReady: false,
  lastScanAt: null,
  lastError: null,
};

function block(overrides: Partial<CodingAgentBlockDto>): CodingAgentBlockDto {
  return {
    id: 1,
    source: "codex",
    sessionId: "s1",
    startedAt: new Date(2026, 9, 4, 10, 0).toISOString(),
    endedAt: new Date(2026, 9, 4, 10, 40).toISOString(),
    cwd: "/Users/me/api",
    project: "api",
    title: null,
    firstPrompt: "add a health check",
    promptCount: 3,
    replyCount: 3,
    activeSeconds: 1200,
    state: "summarized",
    sealedAt: null,
    summary: "Added a /health endpoint with a database ping and its test.",
    summarySource: "cli:codex",
    summaryAttempts: 0,
    summaryError: null,
    ...overrides,
  };
}

afterEach(() => {
  cleanup();
  applyInterfaceLocale("en");
  listeners.clear();
  vi.clearAllMocks();
});

describe("CodingAgentSourcesSection in Settings, Activity", () => {
  let current: ActivityStatusDto;

  beforeEach(() => {
    current = structuredClone(activityStatus);
    invokeMock.mockImplementation(async (command: string, args?: { request?: unknown }) => {
      if (command === "activity_status") return current;
      if (command === "coding_agents_status") return agentsStatus;
      if (command === "activity_save_settings") {
        const { settings } = args?.request as { settings: ActivityStatusDto["settings"] };
        current = { ...current, settings };
        return current;
      }
      throw new Error(`Unhandled command: ${command}`);
    });
  });

  it("lists the seven sources, all off, with whether each was found", async () => {
    render(<ActivitySettingsSection />);

    const section = await screen.findByRole("region", { name: "Coding agents" });
    const switches = within(section).getAllByRole("switch");
    expect(switches).toHaveLength(7);
    for (const toggle of switches) expect(toggle).not.toBeChecked();
    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalledWith("coding_agents_status");
    });
    expect(
      within(section).getByRole("switch", { name: "Read Claude Code sessions" }),
    ).not.toBeChecked();
    expect(within(section).getByRole("switch", { name: "Read Cursor sessions" })).not.toBeChecked();
    expect(within(section).getByText(/summarized by the agent's own CLI/i)).toBeInTheDocument();
  });

  it("shows each source with its brand mark", async () => {
    render(<ActivitySettingsSection />);

    const section = await screen.findByRole("region", { name: "Coding agents" });
    for (const [name, brand] of [
      ["Claude Code", "claudecode"],
      ["Codex", "codex"],
      ["Copilot CLI", "githubcopilot"],
      ["Copilot in VS Code", "githubcopilot"],
      ["Cursor", "cursor"],
      ["Cursor CLI", "cursor"],
      ["Antigravity", "antigravity"],
    ]) {
      const title = within(section).getByRole("heading", { name });
      expect(title.querySelector(`[data-brand="${brand}"]`), name).not.toBeNull();
    }
  });

  it("turning a source on saves it with the rest of the activity settings", async () => {
    const user = userEvent.setup();
    render(<ActivitySettingsSection />);

    await user.click(await screen.findByRole("switch", { name: "Read Claude Code sessions" }));
    await user.click(screen.getByRole("switch", { name: "Read Codex sessions" }));

    const saves = invokeMock.mock.calls.filter(([command]) => command === "activity_save_settings");
    expect(saves).toHaveLength(2);
    expect(saves[1][1]).toEqual({
      request: {
        settings: {
          ...activityStatus.settings,
          codingAgents: { ...activityStatus.settings.codingAgents, claudeCode: true, codex: true },
        },
      },
    });
    expect(screen.getByRole("switch", { name: "Read Codex sessions" })).toBeChecked();
    expect(await screen.findByText(/The activity database is not available/)).toBeInTheDocument();
  });

  it("is in Portuguese when the interface is", async () => {
    applyInterfaceLocale("pt-BR");
    render(<ActivitySettingsSection />);
    expect(await screen.findByRole("region", { name: "Agentes de código" })).toBeInTheDocument();
    expect(screen.getByRole("switch", { name: "Ler sessões do Cursor" })).toBeInTheDocument();
  });

  it("preserves OS-context invariant on Windows for coding-agent sources in English", async () => {
    const originalPlatform = navigator.platform;
    const originalUserAgent = navigator.userAgent;

    try {
      Object.defineProperty(navigator, "platform", {
        configurable: true,
        get: () => "Win32",
      });
      Object.defineProperty(navigator, "userAgent", {
        configurable: true,
        get: () => "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
      });

      render(<ActivitySettingsSection />);

      const section = await screen.findByRole("region", { name: "Coding agents" });
      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("coding_agents_status");
      });
      const descriptions = section.querySelectorAll(
        ".settings-group-description, .settings-row-description",
      );
      for (const el of descriptions) {
        expect(el.textContent).not.toMatch(/\b(Mac|macOS)\b/i);
      }
    } finally {
      Object.defineProperty(navigator, "platform", {
        configurable: true,
        get: () => originalPlatform,
      });
      Object.defineProperty(navigator, "userAgent", {
        configurable: true,
        get: () => originalUserAgent,
      });
    }
  });

  it("preserves OS-context invariant on Windows for coding-agent sources in Portuguese", async () => {
    const originalPlatform = navigator.platform;
    const originalUserAgent = navigator.userAgent;

    try {
      Object.defineProperty(navigator, "platform", {
        configurable: true,
        get: () => "Win32",
      });
      Object.defineProperty(navigator, "userAgent", {
        configurable: true,
        get: () => "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
      });

      applyInterfaceLocale("pt-BR");
      render(<ActivitySettingsSection />);

      const section = await screen.findByRole("region", { name: "Agentes de código" });
      await waitFor(() => {
        expect(invokeMock).toHaveBeenCalledWith("coding_agents_status");
      });
      const descriptions = section.querySelectorAll(
        ".settings-group-description, .settings-row-description",
      );
      for (const el of descriptions) {
        expect(el.textContent).not.toMatch(/\b(Mac|macOS)\b/i);
      }
    } finally {
      Object.defineProperty(navigator, "platform", {
        configurable: true,
        get: () => originalPlatform,
      });
      Object.defineProperty(navigator, "userAgent", {
        configurable: true,
        get: () => originalUserAgent,
      });
    }
  });
});

describe("coding-agent lane in the Today view", () => {
  const dayStartMs = new Date(2026, 9, 4).getTime();
  const dayEndMs = new Date(2026, 9, 5).getTime();
  const from = new Date(dayStartMs).toISOString();
  const to = new Date(dayEndMs).toISOString();

  function renderDayStrip() {
    render(
      <TimelineDayStrip
        sessions={[]}
        gaps={[]}
        dayStartMs={dayStartMs}
        dayEndMs={dayEndMs}
        from={from}
        to={to}
        selectedSessionId={null}
        onSelectSession={() => {}}
        nowMs={dayEndMs}
        isToday={false}
      />,
    );
  }

  const clock = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

  it("draws each block at its time on the day's scale; hover shows agent, project, time, and summary", async () => {
    const user = userEvent.setup();
    invokeMock.mockResolvedValue([
      block({}),
      block({
        id: 2,
        source: "claude_code",
        project: null,
        state: "sealed",
        summary: null,
        firstPrompt: "fix the login bug",
        promptCount: 1,
        startedAt: new Date(2026, 9, 4, 11, 5).toISOString(),
        endedAt: new Date(2026, 9, 4, 11, 30).toISOString(),
      }),
      block({ id: 3, source: "cursor_cli", state: "live", summary: null, firstPrompt: null }),
    ]);

    renderDayStrip();

    expect(screen.getByText("Coding agents")).toBeInTheDocument();
    const lane = await screen.findByRole("list", { name: "Coding agent sessions" });
    const items = within(lane).getAllByRole("listitem");
    expect(items).toHaveLength(3);
    // 10:00 to 10:40 of a 24-hour day.
    expect(Number.parseFloat(items[0].style.left)).toBeCloseTo((10 / 24) * 100, 3);
    expect(Number.parseFloat(items[0].style.width)).toBeCloseTo((40 / 1440) * 100, 3);

    const codexTime = `${clock.format(new Date(2026, 9, 4, 10, 0))} to ${clock.format(new Date(2026, 9, 4, 10, 40))}`;
    await user.hover(screen.getByRole("button", { name: `Codex, api, ${codexTime}` }));
    const tip = await screen.findByRole("tooltip");
    expect(tip).toHaveTextContent("Codex");
    expect(tip).toHaveTextContent(codexTime);
    expect(tip).toHaveTextContent("3 prompts");
    expect(tip).toHaveTextContent("Added a /health endpoint with a database ping and its test.");
    await user.unhover(screen.getByRole("button", { name: `Codex, api, ${codexTime}` }));

    const claudeTime = `${clock.format(new Date(2026, 9, 4, 11, 5))} to ${clock.format(new Date(2026, 9, 4, 11, 30))}`;
    await user.hover(
      screen.getByRole("button", { name: `Claude Code, No project, ${claudeTime}` }),
    );
    expect(await screen.findByText("Waiting for a summary")).toBeInTheDocument();
    expect(screen.getByText("fix the login bug")).toBeInTheDocument();
    expect(screen.getByText("1 prompt")).toBeInTheDocument();

    expect(items[2]).toHaveAttribute("data-state", "live");
    expect(invokeMock).toHaveBeenCalledWith("coding_agents_blocks", { request: { from, to } });
  });

  it("reloads when the scanner reports new blocks", async () => {
    invokeMock.mockResolvedValueOnce([]);
    renderDayStrip();
    expect(await screen.findByText("No coding agent sessions on this day")).toBeInTheDocument();

    invokeMock.mockResolvedValueOnce([block({})]);
    await waitFor(() => expect(listeners.has(CODING_AGENTS_UPDATED_EVENT)).toBe(true));
    listeners.get(CODING_AGENTS_UPDATED_EVENT)?.();

    expect(await screen.findByRole("list", { name: "Coding agent sessions" })).toBeInTheDocument();
  });
});
