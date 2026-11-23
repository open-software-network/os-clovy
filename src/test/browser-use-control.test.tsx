import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { BrowserUseControl } from "../components/plugins/BrowserUseControl";
import { BrowserSessionsTray } from "../components/agent/BrowserSessionsTray";
import { AgentChatTurnRow } from "../components/agent/chat-turns/AgentChatTurnRow";
import type { BrowserSession } from "../lib/tauri";
import { dismissBrowserSessionError } from "../lib/use-browser-sessions";

const mocks = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }));
vi.mock("../lib/feature-flags", () => ({ BROWSER_USE_ENABLED: true }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: mocks.listen }));

let sessions: BrowserSession[];
let granted: boolean;

beforeEach(() => {
  dismissBrowserSessionError();
  vi.clearAllMocks();
  granted = false;
  sessions = [{ sessionId: "owned-session", mode: "background", suspended: false }];
  mocks.listen.mockResolvedValue(() => {});
  mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
    if (command === "browser_access_get") return granted;
    if (command === "browser_access_set") {
      granted = args?.enabled === true;
      return granted;
    }
    if (command === "browser_sessions") return sessions.map((session) => ({ ...session }));
    if (command === "browser_human_takeover") {
      sessions = sessions.map((session) => ({ ...session, mode: "visible", suspended: true }));
      return;
    }
    if (command === "browser_resume_control") {
      sessions = sessions.map((session) => ({ ...session, mode: "background", suspended: false }));
      return;
    }
    if (command === "browser_stop_session") {
      sessions = [];
      return;
    }
    throw new Error(`Unexpected command: ${command}`);
  });
});

describe("Clovy browser control", () => {
  it("keeps an uncertain takeover failure visible when the host closes its session", async () => {
    const original = mocks.invoke.getMockImplementation();
    if (!original) throw new Error("Missing native fixture");
    mocks.invoke.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command === "browser_human_takeover") {
        sessions = [];
        throw new Error("The browser response was lost. Do not repeat the operation.");
      }
      return original(command, args);
    });
    render(<BrowserSessionsTray />);
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: "Take over to sign in" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("The browser response was lost");
    expect(screen.queryByRole("button", { name: "Stop browser task" })).not.toBeInTheDocument();
    expect(
      mocks.invoke.mock.calls.filter(([name]) => name === "browser_human_takeover"),
    ).toHaveLength(1);
  });
  it("keeps an operation receipt in the shell after Plugins unmounts and lets the user close it", async () => {
    const original = mocks.invoke.getMockImplementation();
    let rejectTakeover: ((error: Error) => void) | undefined;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "browser_human_takeover") {
        return new Promise<void>((_resolve, reject) => {
          rejectTakeover = reject;
        });
      }
      return original?.(command, args);
    });
    const user = userEvent.setup();
    const view = render(
      <>
        <ul>
          <BrowserUseControl />
        </ul>
        <BrowserSessionsTray />
      </>,
    );
    await user.click((await screen.findAllByRole("button", { name: "Take over to sign in" }))[0]);
    view.rerender(
      <>
        <ul />
        <BrowserSessionsTray />
      </>,
    );
    sessions = [];
    await act(async () =>
      rejectTakeover?.(new Error("The browser outcome is unknown. Do not repeat it.")),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("The browser outcome is unknown");
    expect(screen.queryByRole("button", { name: "Stop browser task" })).not.toBeInTheDocument();
    const beforeClose = mocks.invoke.mock.calls.length;
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(mocks.invoke.mock.calls).toHaveLength(beforeClose);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "browser_human_takeover"),
    ).toHaveLength(1);
  });

  it("shares Stop generations between Plugins and the shell", async () => {
    const original = mocks.invoke.getMockImplementation();
    let rejectTakeover: ((error: Error) => void) | undefined;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "browser_human_takeover") {
        return new Promise<void>((_resolve, reject) => {
          rejectTakeover = reject;
        });
      }
      return original?.(command, args);
    });
    const user = userEvent.setup();
    render(
      <>
        <ul>
          <BrowserUseControl />
        </ul>
        <BrowserSessionsTray />
      </>,
    );
    await user.click((await screen.findAllByRole("button", { name: "Take over to sign in" }))[0]);
    const stops = screen.getAllByRole("button", { name: "Stop browser task" });
    expect(stops[1]).toBeEnabled();
    await user.click(stops[1]);
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Stop browser task" })).not.toBeInTheDocument(),
    );
    const reads = mocks.invoke.mock.calls.filter(
      ([command]) => command === "browser_sessions",
    ).length;
    await act(async () => rejectTakeover?.(new Error("Obsolete Plugins takeover failure")));
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "browser_sessions"),
    ).toHaveLength(reads);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "browser_stop_session"),
    ).toHaveLength(1);
  });

  it("changes the stored consent from Plugins and keeps the chat card in sync", async () => {
    const user = userEvent.setup();
    render(
      <>
        <ul>
          <BrowserUseControl />
        </ul>
        <AgentChatTurnRow
          turn={{
            id: "request",
            role: "assistant",
            status: "complete",
            createdAt: "2026-11-01T00:00:00Z",
            parts: [{ type: "text", text: "[REQUEST:BROWSER_ACCESS]" }],
          }}
          approvalSubmitting={{}}
          clarifySubmitting={{}}
          sudoSubmitting={{}}
          secretSubmitting={{}}
          thinkingOpen={() => false}
          onThinkingOpenChange={vi.fn()}
          onApproval={vi.fn()}
          onClarify={vi.fn()}
          onSudo={vi.fn()}
          onSecret={vi.fn()}
        />
      </>,
    );
    const toggle = await screen.findByRole("switch", { name: "Enable browser use" });
    await waitFor(() => expect(toggle).toBeEnabled());
    await user.click(toggle);
    await waitFor(() => expect(screen.getByText("Browser use enabled")).toBeInTheDocument());
    expect(mocks.invoke).toHaveBeenCalledWith("browser_access_set", { enabled: true });
    await user.click(toggle);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Enable Browser use" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Enable Browser use" }));
    await waitFor(() => expect(toggle).toBeChecked());
    expect(mocks.invoke.mock.calls.filter(([name]) => name === "browser_access_set")).toHaveLength(
      3,
    );
  });

  it("exposes takeover and explicit resume from the shell, using host state", async () => {
    const user = userEvent.setup();
    render(<BrowserSessionsTray />);
    expect(await screen.findByText("Clovy browser - background")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Take over to sign in" }));
    expect(
      await screen.findByText("Clovy browser - visible, you have control"),
    ).toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledWith("browser_human_takeover", {
      sessionId: "owned-session",
    });
    expect(screen.getByText(/Agent actions and capture are paused/)).toBeInTheDocument();
    expect(mocks.invoke.mock.calls.some(([name]) => name === "browser_resume_control")).toBe(false);
    await user.click(screen.getByRole("button", { name: "Resume agent control" }));
    expect(await screen.findByText("Clovy browser - background")).toBeInTheDocument();
    expect(mocks.invoke).toHaveBeenCalledWith("browser_resume_control", {
      sessionId: "owned-session",
    });
    await user.click(screen.getByRole("button", { name: "Stop browser task" }));
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Stop browser task" })).not.toBeInTheDocument(),
    );
    expect(mocks.invoke).toHaveBeenCalledWith("browser_stop_session", {
      sessionId: "owned-session",
    });
  });

  it("keeps suspended background control honest while the window is not ready", async () => {
    sessions = [{ sessionId: "opening-session", mode: "background", suspended: true }];
    render(<BrowserSessionsTray />);
    expect(await screen.findByText("Clovy browser - control paused")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Agent actions and capture are paused. The browser window has not opened yet.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Clovy browser - visible, you have control")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop browser task" })).toBeEnabled();
  });

  it("retains suspended state while resume is pending and never retries an uncertain result", async () => {
    sessions = [{ sessionId: "paused-session", mode: "visible", suspended: true }];
    const base = mocks.invoke.getMockImplementation();
    let rejectResume: ((error: Error) => void) | undefined;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "browser_resume_control")
        return new Promise((_resolve, reject) => {
          rejectResume = reject;
        });
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(
      <ul>
        <BrowserUseControl />
      </ul>,
    );
    // The app-owned store retains its last snapshot between mounted surfaces.
    // Wait for this fixture's fresh host state before choosing the target.
    await screen.findByText("Clovy browser - visible, you have control");
    const resume = await screen.findByRole("button", { name: "Resume agent control" });
    await user.click(resume);
    expect(mocks.invoke).toHaveBeenCalledWith("browser_resume_control", {
      sessionId: "paused-session",
    });
    expect(resume).toBeDisabled();
    expect(screen.getByText("Clovy browser - visible, you have control")).toBeInTheDocument();
    await act(async () =>
      rejectResume?.(new Error("Control outcome unknown. Inspect the browser state.")),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("Control outcome unknown");
    expect(resume).toBeEnabled();
    expect(screen.getByText("Clovy browser - visible, you have control")).toBeInTheDocument();
    expect(
      mocks.invoke.mock.calls.filter(([name]) => name === "browser_resume_control"),
    ).toHaveLength(1);
  });

  it.each([
    "resolve",
    "reject",
  ] as const)("stops immediately while takeover is pending and ignores its stale %s", async (outcome) => {
    const base = mocks.invoke.getMockImplementation();
    let resolveTakeover: (() => void) | undefined;
    let rejectTakeover: ((error: Error) => void) | undefined;
    mocks.invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
      if (command === "browser_human_takeover") {
        return new Promise<void>((resolve, reject) => {
          resolveTakeover = resolve;
          rejectTakeover = reject;
        });
      }
      return base?.(command, args);
    });
    const user = userEvent.setup();
    render(<BrowserSessionsTray />);
    await user.click(await screen.findByRole("button", { name: "Take over to sign in" }));
    expect(screen.getByRole("button", { name: "Take over to sign in" })).toBeDisabled();
    const stop = screen.getByRole("button", { name: "Stop browser task" });
    expect(stop).toBeEnabled();
    await user.click(stop);
    expect(mocks.invoke).toHaveBeenCalledWith("browser_stop_session", {
      sessionId: "owned-session",
    });
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Stop browser task" })).not.toBeInTheDocument(),
    );
    const reads = mocks.invoke.mock.calls.filter(
      ([command]) => command === "browser_sessions",
    ).length;
    await act(async () => {
      if (outcome === "resolve") resolveTakeover?.();
      else rejectTakeover?.(new Error("Stale takeover error"));
    });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.queryByText(/Clovy browser/)).not.toBeInTheDocument();
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "browser_sessions"),
    ).toHaveLength(reads);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "browser_human_takeover"),
    ).toHaveLength(1);
    expect(
      mocks.invoke.mock.calls.filter(([command]) => command === "browser_stop_session"),
    ).toHaveLength(1);
  });

  it("shows the visible mode and keeps takeover scoped to the selected session", async () => {
    sessions = [
      { sessionId: "session-a", mode: "background", suspended: false },
      { sessionId: "session-b", mode: "visible", suspended: false },
    ];
    const user = userEvent.setup();
    render(
      <ul>
        <BrowserUseControl />
      </ul>,
    );
    const visible = await screen.findByText("Clovy browser - visible");
    const section = visible.closest("section");
    expect(section).not.toBeNull();
    if (!section) throw new Error("Missing browser session controls");
    await user.click(within(section).getByRole("button", { name: "Take over to sign in" }));
    expect(mocks.invoke).toHaveBeenCalledWith("browser_human_takeover", { sessionId: "session-b" });
  });
});
