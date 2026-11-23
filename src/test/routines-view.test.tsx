import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { RoutinesView } from "../components/routines/RoutinesView";
import type { AgentItemDto } from "../lib/agent-runtime-contract";
import type { RoutineJob } from "../lib/agent-routines";
import { requestRoutineRunOpen, type RoutineRunSession } from "../lib/agent-routine-history";
import { representativeConnectorPolicy } from "./fixtures/connector-policy";

const mocks = vi.hoisted(() => ({
  listRoutines: vi.fn<() => Promise<RoutineJob[]>>(),
  pauseRoutine: vi.fn(),
  resumeRoutine: vi.fn(),
  removeRoutine: vi.fn(),
  createRoutine: vi.fn<() => Promise<RoutineJob>>(),
  updateRoutine: vi.fn<() => Promise<RoutineJob>>(),
  triggerRoutine: vi.fn(),
  routineDetailOnRunNow: undefined as (() => Promise<void>) | undefined,
}));

// Pin BROWSER_USE_ENABLED on so the routine Browser use opt-in stays testable
// regardless of the committed flag value.
vi.mock("../lib/feature-flags", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/feature-flags")>()),
  BROWSER_USE_ENABLED: true,
}));

vi.mock("../lib/agent-routines", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/agent-routines")>()),
  ...mocks,
}));

vi.mock("../components/routines/RoutineDetail", async (importOriginal) => {
  const original = await importOriginal<typeof import("../components/routines/RoutineDetail")>();
  const OriginalRoutineDetail = original.RoutineDetail;
  const RoutineDetail: typeof OriginalRoutineDetail = (props) => {
    mocks.routineDetailOnRunNow = props.onRunNow;
    return <OriginalRoutineDetail {...props} />;
  };
  return { ...original, RoutineDetail };
});

const adapterMocks = vi.hoisted(() => ({
  listScheduledRunSessions:
    vi.fn<(options?: { includeActive?: boolean }) => Promise<RoutineRunSession[]>>(),
}));

vi.mock("../lib/agent-routine-history", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/agent-routine-history")>()),
  listRoutineRunSessions: adapterMocks.listScheduledRunSessions,
}));

// Connector commands the create/detail pages call. Defaults mimic a build
// with the connectors module present but nothing connected.
const tauriMocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  listItems: vi.fn<(sessionId: string) => Promise<AgentItemDto[]>>(),
  connectorsList: vi.fn(),
  connectorsConnect: vi.fn(),
  routineTrustGet: vi.fn(),
  routineTrustRecordRun: vi.fn(),
  routineTrustSet: vi.fn(),
  routineBrowserAccessGet: vi.fn(),
  routineBrowserAccessSet: vi.fn(),
  connectorTriggersList: vi.fn(),
  connectorTriggerSet: vi.fn(),
  connectorTriggerDelete: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: tauriMocks.invoke,
}));

const eventMocks = vi.hoisted(() => ({
  listen: vi.fn(),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: eventMocks.listen,
}));

vi.mock("../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/tauri")>()),
  ...tauriMocks,
  agentRuntimeBindings: { listItems: tauriMocks.listItems },
}));

function job(overrides: Partial<RoutineJob> = {}): RoutineJob {
  return {
    job_id: "abc123",
    name: "Morning summary",
    prompt: "Summarize my unread notes and flag anything urgent.",
    prompt_preview: "Summarize my unread notes",
    schedule: "0 9 * * *",
    repeat: "forever",
    deliver: "local",
    created_at: "2026-06-01T09:00:00",
    next_run_at: "2026-06-10T09:00:00",
    last_run_at: null,
    last_status: null,
    enabled: true,
    state: "scheduled",
    ...overrides,
  };
}

function run(overrides: Partial<RoutineRunSession> = {}): RoutineRunSession {
  return {
    id: "cron_abc123_20260610_090000",
    source: "legacy_routine",
    title: "Morning Summary Digest",
    preview: "Here is today's summary of your unread notes.",
    status: "completed",
    model: "open-software/auto",
    safetyMode: "sandboxed",
    workspacePath: "",
    createdAt: "2026-06-10T09:00:00Z",
    updatedAt: "2026-06-10T09:00:30Z",
    completedAt: "2026-06-10T09:00:30Z",
    endedAt: "2026-06-10T09:00:30Z",
    ...overrides,
  };
}

function renderView() {
  return render(<RoutinesView />);
}

async function openDetail(name: string) {
  const list = await screen.findByRole("list", { name: "Routines" });
  await userEvent.click(within(list).getByText(name));
  return screen.findByRole("textbox", { name: "Instructions" });
}

beforeEach(() => {
  vi.clearAllMocks();
  tauriMocks.listItems.mockResolvedValue([]);
  mocks.routineDetailOnRunNow = undefined;
  mocks.pauseRoutine.mockResolvedValue({});
  mocks.resumeRoutine.mockResolvedValue({});
  mocks.removeRoutine.mockResolvedValue({});
  mocks.triggerRoutine.mockResolvedValue({});
  mocks.createRoutine.mockResolvedValue(job());
  mocks.updateRoutine.mockResolvedValue(job());
  adapterMocks.listScheduledRunSessions.mockResolvedValue([]);
  tauriMocks.connectorsList.mockResolvedValue([]);
  tauriMocks.invoke.mockImplementation(async (command: string) =>
    command === "connectors_policy" ? representativeConnectorPolicy() : undefined,
  );
  tauriMocks.connectorsConnect.mockResolvedValue(googleAccount());
  tauriMocks.routineTrustGet.mockResolvedValue(null);
  tauriMocks.routineBrowserAccessGet.mockResolvedValue({ enabled: false });
  tauriMocks.routineBrowserAccessSet.mockImplementation(async (input: { enabled: boolean }) =>
    input.enabled
      ? { enabled: true, serverName: "clovy_browser_routine_abc123" }
      : { enabled: false },
  );
  tauriMocks.routineTrustRecordRun.mockResolvedValue(null);
  tauriMocks.routineTrustSet.mockImplementation(
    async (input: { trustMode: string; autonomousTools?: string[] }) => ({
      trustMode: input.trustMode,
      approvalRunCount: 0,
      autonomousTools: input.autonomousTools ?? [],
    }),
  );
  tauriMocks.connectorTriggersList.mockResolvedValue([]);
  tauriMocks.connectorTriggerSet.mockImplementation(
    async (input: { jobId: string; kind: string; accountId: string; config: object }) => ({
      id: "trig-1",
      ...input,
    }),
  );
  tauriMocks.connectorTriggerDelete.mockResolvedValue(undefined);
  eventMocks.listen.mockResolvedValue(() => {});
});

function googleAccount(overrides: Record<string, unknown> = {}) {
  return {
    accountId: "acc-1",
    provider: "google" as const,
    email: "alex@example.com",
    scopes: [
      "https://www.googleapis.com/auth/gmail.readonly",
      "https://www.googleapis.com/auth/gmail.compose",
      "https://www.googleapis.com/auth/gmail.modify",
      "https://www.googleapis.com/auth/calendar.events",
    ],
    status: "connected" as const,
    ...overrides,
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("RoutinesView list", () => {
  it("lists routines with schedule and state", async () => {
    mocks.listRoutines.mockResolvedValue([
      job(),
      job({
        job_id: "def456",
        name: "Weekly digest",
        prompt_preview: "Compile a digest of the week",
        state: "paused",
        last_status: "error",
      }),
    ]);
    renderView();

    expect(await screen.findByText("Morning summary")).toBeInTheDocument();
    expect(screen.getByText("Weekly digest")).toBeInTheDocument();
    expect(screen.getByText("Paused")).toBeInTheDocument();
    expect(screen.getByText("Last run failed")).toBeInTheDocument();
    expect(screen.queryByText("Summarize my unread notes")).toBeNull();
  });

  it("shows cron schedules as plain language and matches it in search", async () => {
    mocks.listRoutines.mockResolvedValue([
      job({ schedule: "0 9 * * 1-5" }),
      job({
        job_id: "def456",
        name: "Weekly digest",
        prompt_preview: "Compile a digest of the week",
        schedule: "0 8 * * 1",
      }),
    ]);
    renderView();

    const nine = new Date(2000, 0, 1, 9, 0).toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
    });
    expect(await screen.findByText(`Weekdays ${nine}`, { exact: false })).toBeInTheDocument();
    expect(screen.queryByText("0 9 * * 1-5", { exact: false })).toBeNull();

    // The search box matches the displayed wording, not just the raw cron.
    await userEvent.type(screen.getByRole("searchbox"), "weekdays");
    expect(screen.getByText("Morning summary")).toBeInTheDocument();
    expect(screen.queryByText("Weekly digest")).toBeNull();
  });

  it("badges routines that carry machine toolsets or a cron script", async () => {
    mocks.listRoutines.mockResolvedValue([
      job(),
      job({
        job_id: "def456",
        name: "Nightly cleanup",
        enabled_toolsets: ["terminal", "file", "web"],
      }),
      // Scripts run as shell subprocesses of the unjailed gateway, outside
      // the toolset gate, so a script-backed job is unrestricted even with
      // no enabled_toolsets override.
      job({
        job_id: "ghi789",
        name: "Disk watchdog",
        script: "/Users/alex/bin/check-disk.sh",
        no_agent: true,
      }),
    ]);
    renderView();

    expect(await screen.findByText("Nightly cleanup")).toBeInTheDocument();
    // Badges for the toolset-widened and the script-backed routines, none
    // for the sandboxed one.
    const list = screen.getByRole("list", { name: "Routines" });
    expect(within(list).getAllByText("Unrestricted")).toHaveLength(2);
  });

  it("shows starter routines once routines exist", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();

    expect(await screen.findByText("Morning summary")).toBeInTheDocument();
    const starters = screen.getByRole("region", { name: "Starter routines" });
    expect(within(starters).getByText("Morning brief")).toBeInTheDocument();
  });

  it("pauses a scheduled routine from its detail toggle and reloads", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    mocks.listRoutines.mockResolvedValue([job({ state: "paused" })]);
    const toggle = screen.getByRole("switch", {
      name: "Morning summary active",
    });
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);

    await waitFor(() => expect(mocks.pauseRoutine).toHaveBeenCalledWith("abc123"));
    expect(await screen.findByText("Paused")).toBeInTheDocument();
  });

  it("resumes a paused routine from its detail toggle", async () => {
    mocks.listRoutines.mockResolvedValue([job({ state: "paused" })]);
    renderView();
    await openDetail("Morning summary");

    const toggle = screen.getByRole("switch", {
      name: "Morning summary active",
    });
    expect(toggle).not.toBeChecked();
    await userEvent.click(toggle);
    await waitFor(() => expect(mocks.resumeRoutine).toHaveBeenCalledWith("abc123"));
  });

  it("surfaces a failed reload after a successful pause", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    mocks.listRoutines.mockRejectedValue(new Error("reload failed"));
    await userEvent.click(screen.getByRole("switch", { name: "Morning summary active" }));

    expect(await screen.findByText("reload failed")).toBeInTheDocument();
  });

  it("surfaces a load error", async () => {
    mocks.listRoutines.mockRejectedValue(new Error("gateway down"));
    renderView();
    expect(await screen.findByText("gateway down")).toBeInTheDocument();
  });

  it("wraps an agent server load error and retries the list", async () => {
    const rawError = "Agent API returned 500: Internal Server Error";
    mocks.listRoutines.mockRejectedValueOnce(new Error(rawError)).mockResolvedValueOnce([job()]);
    renderView();

    expect(
      await screen.findByText("Clovy ran into a problem with that request."),
    ).toBeInTheDocument();
    expect(screen.queryByText(rawError, { exact: false })).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));

    await waitFor(() => expect(mocks.listRoutines).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("Morning summary")).toBeInTheDocument();
  });

  it("keeps ordinary load errors unchanged", async () => {
    mocks.listRoutines.mockRejectedValue(new Error("gateway down"));
    renderView();

    expect(await screen.findByText("gateway down")).toBeInTheDocument();
  });
});

describe("RoutinesView templates and creation", () => {
  it("shows starter templates while no routine exists", async () => {
    mocks.listRoutines.mockResolvedValue([]);
    renderView();

    expect(await screen.findByText("Morning brief")).toBeVisible();
    expect(screen.getByText("Morning brief")).toBeInTheDocument();
    expect(screen.getByText("Weekly review")).toBeInTheDocument();
  });

  it("opens the editor prefilled from a starter template", async () => {
    mocks.listRoutines.mockResolvedValue([]);
    renderView();
    await screen.findByText("Morning brief");

    await userEvent.click(screen.getByRole("button", { name: "Add Morning brief" }));

    expect(screen.getByRole("textbox", { name: "Routine name" })).toHaveValue("Morning brief");
    const instructions = screen.getByRole("textbox", {
      name: "Instructions",
    }) as HTMLTextAreaElement;
    expect(instructions.value).toContain("morning brief");
    // The template schedule "0 8 * * 1-5" lands on the Weekdays preset.
    expect(screen.getByRole("button", { name: "Schedule type" })).toHaveTextContent("Weekdays");
  });

  it("creates a routine from the editor and opens its detail page", async () => {
    mocks.listRoutines.mockResolvedValueOnce([]);
    renderView();
    await screen.findByText("Morning brief");

    await userEvent.click(screen.getByRole("button", { name: "New routine" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Instructions" }),
      "Summarize my unread notes and flag anything urgent.",
    );
    mocks.listRoutines.mockResolvedValue([job()]);
    await userEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() =>
      expect(mocks.createRoutine).toHaveBeenCalledWith({
        prompt: "Summarize my unread notes and flag anything urgent.",
        schedule: "0 9 * * *",
        name: undefined,
        unrestricted: false,
        enabledToolsets: [
          "web",
          "vision",
          "skills",
          "todo",
          "memory",
          "session_search",
          "context_engine",
        ],
      }),
    );
    // Creation lands on the new routine's detail page.
    expect(await screen.findByRole("textbox", { name: "Routine name" })).toHaveValue(
      "Morning summary",
    );
  });
});

describe("RoutinesView connector templates", () => {
  it("shows the connector templates with their tool summary and trust mode", async () => {
    mocks.listRoutines.mockResolvedValue([]);
    renderView();

    expect(await screen.findByText("Morning briefing")).toBeInTheDocument();
    expect(screen.getByText("Auto-inbox")).toBeInTheDocument();
    expect(screen.getByText("Meeting prep")).toBeInTheDocument();
    expect(
      screen.getAllByText(/This routine can: read your mail, read your calendar/).length,
    ).toBeGreaterThan(0);
    expect(
      screen.getByText(/This routine can: read your mail, draft replies, label and archive/),
    ).toBeInTheDocument();
    expect(screen.getAllByText(/Trust: approval/).length).toBeGreaterThan(0);
  });

  it("gates a connector template on the required Google scopes and connects inline", async () => {
    tauriMocks.connectorsList.mockResolvedValueOnce([]).mockResolvedValue([googleAccount()]);
    mocks.listRoutines.mockResolvedValue([]);
    renderView();
    await screen.findByText("Morning briefing");

    await userEvent.click(screen.getByRole("button", { name: "Add Morning briefing" }));

    // Blocked: no account with the template's bundles is connected yet.
    expect(await screen.findByText(/needs a connected Google account/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();

    await userEvent.click(screen.getByRole("button", { name: "Connect Google account" }));

    await waitFor(() =>
      expect(tauriMocks.connectorsConnect).toHaveBeenCalledWith({
        scopes: ["gmail_read", "calendar_read"],
      }),
    );
    await waitFor(() => expect(screen.getByRole("button", { name: "Create" })).toBeEnabled());
  });

  it("gates a template on the first connected account, not any account", async () => {
    // The routine runs against the first connected account, so a later account
    // holding the scopes must not enable Create while the first one lacks them.
    tauriMocks.connectorsList.mockResolvedValue([
      googleAccount({
        accountId: "acc-first",
        email: "first@example.com",
        scopes: ["https://www.googleapis.com/auth/gmail.readonly"],
      }),
      googleAccount({
        accountId: "acc-second",
        email: "second@example.com",
      }),
    ]);
    mocks.listRoutines.mockResolvedValue([]);
    renderView();
    await screen.findByText("Morning briefing");

    await userEvent.click(screen.getByRole("button", { name: "Add Morning briefing" }));

    // Morning briefing needs calendar read, which only the second account has,
    // so Create stays blocked because the first account is the one that runs.
    expect(await screen.findByText(/needs a connected Google account/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Create" })).toBeDisabled();
  });

  it("creates a scheduled connector routine with trust and an immediate first run", async () => {
    tauriMocks.connectorsList.mockResolvedValue([googleAccount()]);
    mocks.listRoutines.mockResolvedValueOnce([]);
    renderView();
    await screen.findByText("Morning briefing");

    await userEvent.click(screen.getByRole("button", { name: "Add Morning briefing" }));
    mocks.listRoutines.mockResolvedValue([job()]);
    await userEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(mocks.createRoutine).toHaveBeenCalled());
    const createArgs = (mocks.createRoutine.mock.calls[0] as unknown[])[0] as {
      schedule: string;
      enabledToolsets?: string[];
    };
    expect(createArgs.schedule).toBe("0 8 * * *");
    // Read-only trust: read servers ambient, no actions servers.
    expect(createArgs.enabledToolsets).toContain("june_gmail");
    expect(createArgs.enabledToolsets).toContain("june_gcal");
    expect(createArgs.enabledToolsets).not.toContain("june_gmail_actions");

    await waitFor(() =>
      expect(tauriMocks.routineTrustSet).toHaveBeenCalledWith({
        jobId: "abc123",
        trustMode: "read_only",
        autonomousTools: undefined,
      }),
    );
    // The first run fires right away for scheduled connector routines.
    await waitFor(() => expect(mocks.triggerRoutine).toHaveBeenCalledWith("abc123"));
  });

  it("creates an active event-triggered routine with the trigger subscription", async () => {
    tauriMocks.connectorsList.mockResolvedValue([googleAccount()]);
    mocks.listRoutines.mockResolvedValueOnce([]);
    renderView();
    await screen.findByText("Auto-inbox");

    await userEvent.click(screen.getByRole("button", { name: "Add Auto-inbox" }));
    // The "When" picker is preset to the email trigger, not a schedule.
    expect(screen.getByRole("button", { name: "Trigger type" })).toHaveTextContent(
      "When new email arrives",
    );
    mocks.listRoutines.mockResolvedValue([job()]);
    await userEvent.click(screen.getByRole("button", { name: "Create" }));

    await waitFor(() => expect(mocks.createRoutine).toHaveBeenCalled());
    const createArgs = (mocks.createRoutine.mock.calls[0] as unknown[])[0] as {
      schedule: string;
      enabledToolsets?: string[];
    };
    // Event routines get the far-future one-time schedule, never a cron.
    expect(createArgs.schedule).toBe("2099-01-01T09:00:00Z");
    // Approval trust: the actions servers ride along.
    expect(createArgs.enabledToolsets).toContain("june_gmail_actions");

    expect(mocks.pauseRoutine).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(tauriMocks.connectorTriggerSet).toHaveBeenCalledWith({
        jobId: "abc123",
        kind: "email_received",
        accountId: "acc-1",
        config: {},
      }),
    );
    // Event template installs still fire an immediate first run (approval-gated),
    // so value shows in the first session per the install contract; the one-off
    // trigger leaves the routine active for its event trigger to own later runs.
    await waitFor(() => expect(mocks.triggerRoutine).toHaveBeenCalledWith("abc123"));
  });

  it("removes a newly created routine when trust setup fails", async () => {
    tauriMocks.connectorsList.mockResolvedValue([googleAccount()]);
    mocks.listRoutines.mockResolvedValue([]);
    tauriMocks.routineTrustSet.mockRejectedValueOnce(new Error("trust store unavailable"));
    renderView();
    await screen.findByText("Morning briefing");

    await userEvent.click(screen.getByRole("button", { name: "Add Morning briefing" }));
    await userEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(await screen.findByText("trust store unavailable")).toBeInTheDocument();
    await waitFor(() => expect(mocks.removeRoutine).toHaveBeenCalledWith("abc123"));
    expect(mocks.triggerRoutine).not.toHaveBeenCalled();
  });

  it("removes an event routine when trigger setup fails", async () => {
    tauriMocks.connectorsList.mockResolvedValue([googleAccount()]);
    mocks.listRoutines.mockResolvedValue([]);
    tauriMocks.connectorTriggerSet.mockRejectedValueOnce(new Error("trigger store unavailable"));
    renderView();
    await screen.findByText("Auto-inbox");

    await userEvent.click(screen.getByRole("button", { name: "Add Auto-inbox" }));
    await userEvent.click(screen.getByRole("button", { name: "Create" }));

    expect(await screen.findByText("trigger store unavailable")).toBeInTheDocument();
    expect(mocks.pauseRoutine).not.toHaveBeenCalled();
    expect(mocks.removeRoutine).toHaveBeenCalledWith("abc123");
    expect(mocks.triggerRoutine).not.toHaveBeenCalled();
  });

  it("reports a finished run to the backend crediting path with its id and end time", async () => {
    // The backend decides whether to credit (approval mode + after the window);
    // the view just reports every finished run once, id and end time included.
    mocks.listRoutines.mockResolvedValue([job()]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      run({
        id: "cron_abc123_20260709_100000",
        status: "completed",
        ended_at: "2026-07-09T10:05:00Z",
        active: false,
      }),
    ]);
    renderView();
    await waitFor(() =>
      expect(tauriMocks.routineTrustRecordRun).toHaveBeenCalledWith({
        jobId: "abc123",
        runId: "cron_abc123_20260709_100000",
        runEndedAt: "2026-07-09T10:05:00Z",
      }),
    );
  });

  it("does not report a still-running or failed run", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      run({ id: "cron_abc123_20260709_090000", status: "running", active: true }),
      run({
        id: "cron_abc123_20260709_100000",
        status: "failed",
        ended_at: "2026-07-09T10:05:00Z",
      }),
    ]);
    renderView();
    await waitFor(() => expect(adapterMocks.listScheduledRunSessions).toHaveBeenCalled());
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(tauriMocks.routineTrustRecordRun).not.toHaveBeenCalled();
  });

  it("badges rows whose toolsets imply an action-capable trust mode", async () => {
    mocks.listRoutines.mockResolvedValue([
      job(),
      job({
        job_id: "def456",
        name: "Inbox triage",
        enabled_toolsets: ["web", "june_gmail", "june_gmail_actions"],
      }),
      job({
        job_id: "ghi789",
        name: "Auto archiver",
        enabled_toolsets: ["web", "june_gmail", "june_gmail_auto_ab12cd34"],
      }),
    ]);
    renderView();

    const list = await screen.findByRole("list", { name: "Routines" });
    expect(within(list).getByText("Approval")).toBeInTheDocument();
    expect(within(list).getByText("Autonomous")).toBeInTheDocument();
  });
});

describe("RoutinesView detail", () => {
  it("keeps routine browser use off by default and persists an explicit opt-in", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    const browser = await screen.findByRole("switch", {
      name: "Allow browser use for this routine",
    });
    expect(browser).not.toBeChecked();

    await userEvent.click(browser);
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(tauriMocks.routineBrowserAccessSet).toHaveBeenCalledWith({
        jobId: "abc123",
        enabled: true,
      }),
    );
    expect(mocks.updateRoutine).toHaveBeenCalledWith(
      "abc123",
      expect.objectContaining({
        enabledToolsets: expect.arrayContaining(["clovy_browser_routine_abc123"]),
      }),
    );
  });

  it("opens a routine with its full instructions", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();

    const instructions = await openDetail("Morning summary");
    expect(instructions).toHaveValue("Summarize my unread notes and flag anything urgent.");
    expect(screen.getByRole("textbox", { name: "Routine name" })).toHaveValue("Morning summary");
  });

  it("saves only the changed fields", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();

    const instructions = await openDetail("Morning summary");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();

    await userEvent.clear(instructions);
    await userEvent.type(instructions, "List my unread notes only.");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateRoutine).toHaveBeenCalledWith("abc123", {
        prompt: "List my unread notes only.",
      }),
    );
  });

  it("rolls the trust change back when the cron save fails", async () => {
    // An autonomous routine (autonomy already unlocked) with one granted tool.
    tauriMocks.routineTrustGet.mockResolvedValue({
      trustMode: "autonomous",
      approvalRunCount: 3,
      autonomousTools: ["create_draft"],
      autonomousServers: ["june_gmail_auto_x"],
    });
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    // Downgrade to read only, then the schedule update fails.
    await userEvent.click(screen.getByRole("button", { name: /Read only/ }));
    mocks.updateRoutine.mockRejectedValueOnce(new Error("gateway down"));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    // The failed save surfaces, and the trust is restored to autonomous so the
    // DB trust and the job's toolsets do not diverge (the read-only attempt is
    // rolled back rather than left with a deleted grant and stale toolsets).
    expect(await screen.findByText("gateway down")).toBeInTheDocument();
    await waitFor(() =>
      expect(tauriMocks.routineTrustSet).toHaveBeenLastCalledWith({
        jobId: "abc123",
        trustMode: "autonomous",
        autonomousTools: ["create_draft"],
      }),
    );
  });

  it("does not write the event trigger when the cron save fails", async () => {
    // Switching a scheduled routine to an email trigger, but the cron update
    // fails. The trigger row and pause must not be applied: they run only after
    // onSave succeeds, so a failed save can never leave a trigger firing a
    // routine whose far-future schedule never persisted.
    tauriMocks.connectorsList.mockResolvedValue([googleAccount()]);
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    await userEvent.click(screen.getByRole("button", { name: "Trigger type" }));
    await userEvent.click(screen.getByRole("option", { name: "When new email arrives" }));
    mocks.updateRoutine.mockRejectedValueOnce(new Error("gateway down"));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("gateway down")).toBeInTheDocument();
    // No trigger subscription written, and the routine was never paused: the
    // save aborted before any trigger side effect.
    expect(tauriMocks.connectorTriggerSet).not.toHaveBeenCalled();
    expect(mocks.pauseRoutine).not.toHaveBeenCalled();
  });

  it("does not mint trust when an event trigger fails validation", async () => {
    // An autonomous routine downgraded to read only while also switched to an
    // email trigger, but no Google account is connected. The trigger validation
    // aborts the save, and it must abort before the trust/grant write so a
    // rejected save never leaves a half-applied trust change behind.
    tauriMocks.routineTrustGet.mockResolvedValue({
      trustMode: "autonomous",
      approvalRunCount: 3,
      autonomousTools: ["create_draft"],
      autonomousServers: ["june_gmail_auto_x"],
    });
    tauriMocks.connectorsList.mockResolvedValue([]);
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    await userEvent.click(screen.getByRole("button", { name: /Read only/ }));
    await userEvent.click(screen.getByRole("button", { name: "Trigger type" }));
    await userEvent.click(screen.getByRole("option", { name: "When new email arrives" }));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(
      await screen.findByText("Connect a Google account before using an event trigger."),
    ).toBeInTheDocument();
    // Validation aborted first: neither the trust downgrade nor the cron update
    // was attempted, so nothing needs rolling back.
    expect(tauriMocks.routineTrustSet).not.toHaveBeenCalled();
    expect(mocks.updateRoutine).not.toHaveBeenCalled();
  });

  it("keeps the event trigger when resuming its scheduled replacement fails", async () => {
    const storedTrigger = {
      id: "trig-1",
      jobId: "abc123",
      kind: "email_received" as const,
      accountId: "acc-1",
      config: {},
    };
    tauriMocks.connectorTriggersList.mockResolvedValue([storedTrigger]);
    tauriMocks.connectorsList.mockResolvedValue([googleAccount()]);
    mocks.listRoutines.mockResolvedValue([
      job({ schedule: "2099-01-01T09:00:00Z", state: "paused" }),
    ]);
    mocks.resumeRoutine.mockRejectedValueOnce(new Error("gateway down"));
    renderView();
    await openDetail("Morning summary");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Trigger type" })).toHaveTextContent(
        "When new email arrives",
      ),
    );

    await userEvent.click(screen.getByRole("button", { name: "Trigger type" }));
    await userEvent.click(screen.getByRole("option", { name: "On a schedule" }));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("gateway down")).toBeInTheDocument();
    expect(mocks.updateRoutine).toHaveBeenCalledWith("abc123", {
      schedule: "0 9 * * *",
    });
    expect(mocks.resumeRoutine).toHaveBeenCalledWith("abc123");
    expect(tauriMocks.connectorTriggerDelete).not.toHaveBeenCalled();
    expect(tauriMocks.connectorTriggerSet).not.toHaveBeenCalled();
  });

  it("re-pauses an event routine when deleting its trigger fails", async () => {
    tauriMocks.connectorTriggersList.mockResolvedValue([
      {
        id: "trig-1",
        jobId: "abc123",
        kind: "email_received",
        accountId: "acc-1",
        config: {},
      },
    ]);
    tauriMocks.connectorsList.mockResolvedValue([googleAccount()]);
    tauriMocks.connectorTriggerDelete.mockRejectedValueOnce(new Error("trigger delete failed"));
    mocks.listRoutines.mockResolvedValue([
      job({ schedule: "2099-01-01T09:00:00Z", state: "paused" }),
    ]);
    renderView();
    await openDetail("Morning summary");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Trigger type" })).toHaveTextContent(
        "When new email arrives",
      ),
    );

    await userEvent.click(screen.getByRole("button", { name: "Trigger type" }));
    await userEvent.click(screen.getByRole("option", { name: "On a schedule" }));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText("trigger delete failed")).toBeInTheDocument();
    expect(mocks.resumeRoutine).toHaveBeenCalledWith("abc123");
    expect(tauriMocks.connectorTriggerDelete).toHaveBeenCalledWith("trig-1");
    expect(mocks.pauseRoutine).toHaveBeenCalledWith("abc123");
  });

  it("restores a blank local name after saving unrelated changes", async () => {
    mocks.listRoutines.mockResolvedValueOnce([job()]).mockResolvedValueOnce([job()]);
    renderView();

    const instructions = await openDetail("Morning summary");
    const name = screen.getByRole("textbox", { name: "Routine name" });

    await userEvent.clear(name);
    await userEvent.clear(instructions);
    await userEvent.type(instructions, "List my unread notes only.");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateRoutine).toHaveBeenCalledWith("abc123", {
        prompt: "List my unread notes only.",
      }),
    );
    await waitFor(() => expect(name).toHaveValue("Morning summary"));
  });

  it("saves a schedule preset change as a cron expression", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    // The stored "0 9 * * *" lands on Daily; switch the preset to Weekdays.
    const trigger = screen.getByRole("button", { name: "Schedule type" });
    expect(trigger).toHaveTextContent("Daily");
    await userEvent.click(trigger);
    await userEvent.click(screen.getByRole("option", { name: "Weekdays" }));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateRoutine).toHaveBeenCalledWith("abc123", {
        schedule: "0 9 * * 1-5",
      }),
    );
  });

  it("widens access after the explicit unrestricted opt-in", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    await userEvent.click(screen.getByRole("button", { name: "Unrestricted" }));
    await userEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocks.updateRoutine).toHaveBeenCalledWith("abc123", {
        unrestricted: true,
      }),
    );
  });

  it("queues an immediate run", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    await userEvent.click(screen.getByRole("button", { name: "Run now" }));
    await waitFor(() => expect(mocks.triggerRoutine).toHaveBeenCalledWith("abc123"));
    expect(screen.getByRole("button", { name: "Queued" })).toBeDisabled();
  });

  it("refreshes the open routine when an agent run finishes", async () => {
    mocks.listRoutines
      .mockResolvedValueOnce([
        job({
          last_status: "error",
          last_error: "The previous run failed.",
        }),
      ])
      .mockResolvedValueOnce([
        job({
          last_run_at: "2026-06-10T09:00:30Z",
          last_status: "ok",
          last_error: null,
        }),
      ]);
    renderView();
    await openDetail("Morning summary");

    expect(screen.getByText("Last run failed.")).toBeInTheDocument();
    await waitFor(() =>
      expect(eventMocks.listen).toHaveBeenCalledWith(
        "clovy://agent-runtime-event",
        expect.any(Function),
      ),
    );
    const runtimeListener = eventMocks.listen.mock.calls.find(
      ([eventName]) => eventName === "clovy://agent-runtime-event",
    )?.[1] as ((event: { payload: { method: string } }) => void) | undefined;
    expect(runtimeListener).toBeDefined();

    await act(async () => {
      runtimeListener?.({ payload: { method: "run.started" } });
      await Promise.resolve();
    });
    expect(mocks.listRoutines).toHaveBeenCalledTimes(1);

    await act(async () => {
      runtimeListener?.({ payload: { method: "run.completed" } });
      await Promise.resolve();
    });

    await waitFor(() => expect(screen.queryByText("Last run failed.")).toBeNull());
    expect(mocks.listRoutines).toHaveBeenCalledTimes(2);
  });

  it("shows this routine's runs and opens one on click", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    const mine = run();
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      mine,
      run({
        id: "cron_other77_20260609_080000",
        title: "Someone else's run",
      }),
    ]);
    renderView();
    await openDetail("Morning summary");
    await userEvent.click(screen.getByRole("tab", { name: "Run history" }));

    const history = screen.getByRole("tabpanel", { name: "Run history" });
    expect(within(history).getByText("Morning summary")).toBeInTheDocument();
    expect(within(history).queryByText("Someone else's run")).toBeNull();

    await userEvent.click(within(history).getByRole("button", { name: /morning summary/i }));
    expect(await screen.findByRole("heading", { name: "Run result" })).toBeInTheDocument();
    expect(tauriMocks.listItems).toHaveBeenCalledWith(mine.id);
  });

  it("labels normalized detail runs with the routine name", async () => {
    mocks.listRoutines.mockResolvedValue([job({ name: "Morning brief" })]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      run({
        title: "Deliver the Morning Brief",
        preview: "Send today's important updates.",
      }),
    ]);
    renderView();
    await openDetail("Morning brief");
    await userEvent.click(screen.getByRole("tab", { name: "Run history" }));

    const history = screen.getByRole("tabpanel", { name: "Run history" });
    expect(within(history).getByText("Morning brief")).toBeInTheDocument();
    expect(within(history).queryByText("Deliver the Morning Brief")).toBeNull();
  });

  it("surfaces the last run failure", async () => {
    mocks.listRoutines.mockResolvedValue([
      job({ last_status: "error", last_error: "Model quota exhausted" }),
    ]);
    renderView();
    await openDetail("Morning summary");

    expect(screen.getByText(/Model quota exhausted/)).toBeInTheDocument();
  });

  it("wraps a detail reload agent server error and retries the detail", async () => {
    const rawError = "Agent API returned 500: Internal Server Error";
    mocks.listRoutines.mockResolvedValueOnce([job()]);
    renderView();
    await openDetail("Morning summary");

    mocks.listRoutines
      .mockRejectedValueOnce(new Error(rawError))
      .mockResolvedValueOnce([job({ state: "paused" })]);
    await userEvent.click(screen.getByRole("switch", { name: "Morning summary active" }));

    expect(
      await screen.findByText("Clovy ran into a problem with that request."),
    ).toBeInTheDocument();
    expect(screen.queryByText(rawError, { exact: false })).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Try again" }));

    await waitFor(() => expect(mocks.listRoutines).toHaveBeenCalledTimes(3));
    expect(await screen.findByText("Paused")).toBeInTheDocument();
  });

  it("deletes a routine after confirmation and returns to the list", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();
    await openDetail("Morning summary");

    await userEvent.click(screen.getByRole("button", { name: "Routine actions" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Delete routine" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(mocks.removeRoutine).toHaveBeenCalledWith("abc123"));
    // Back on the (now empty) list page.
    expect(await screen.findByText("Morning brief")).toBeInTheDocument();
  });

  it("surfaces a failed delete and keeps the routine", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    mocks.removeRoutine.mockRejectedValue(new Error("remove failed"));
    renderView();
    await openDetail("Morning summary");

    await userEvent.click(screen.getByRole("button", { name: "Routine actions" }));
    await userEvent.click(screen.getByRole("menuitem", { name: "Delete routine" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("remove failed")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Routine name" })).toHaveValue("Morning summary");
  });
});

describe("RoutinesView run history", () => {
  it("opens the stored result from a notification before the view mounts", async () => {
    const session = run();
    adapterMocks.listScheduledRunSessions.mockResolvedValue([session]);
    requestRoutineRunOpen(session.id);
    renderView();
    expect(await screen.findByRole("heading", { name: "Run result" })).toBeVisible();
    expect(tauriMocks.listItems).toHaveBeenCalledWith(session.id);
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("returns focus to the same run in a routine's history", async () => {
    const session = run();
    mocks.listRoutines.mockResolvedValue([job()]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([session]);
    renderView();
    await openDetail("Morning summary");
    await userEvent.click(screen.getByRole("tab", { name: "Run history" }));
    const history = screen.getByRole("tabpanel", { name: "Run history" });
    await userEvent.click(within(history).getByRole("button", { name: /morning summary/i }));
    expect(await screen.findByRole("heading", { name: "Run result" })).toHaveFocus();
    await userEvent.click(screen.getByRole("button", { name: "Back to routines" }));
    expect(screen.getByRole("tab", { name: "Run history" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("button", { name: /morning summary/i })).toHaveFocus();
  });

  it("opens a stored routine result without a message composer", async () => {
    const session = run();
    adapterMocks.listScheduledRunSessions.mockResolvedValue([session]);
    tauriMocks.listItems.mockResolvedValue([
      {
        id: "output",
        sessionId: session.id,
        sequence: 2,
        createdAt: session.createdAt,
        kind: "message",
        role: "assistant",
        text: "The fixture routine stored its result.",
        status: "complete",
      },
      {
        id: "prompt",
        sessionId: session.id,
        sequence: 1,
        createdAt: session.createdAt,
        kind: "message",
        role: "user",
        text: "Private routine instructions",
        status: "complete",
      },
    ]);
    renderView();
    const history = await screen.findByRole("region", { name: "Run history" });
    await userEvent.click(within(history).getByRole("button", { name: /morning summary/i }));
    expect(await screen.findByRole("heading", { name: "Run result" })).toBeInTheDocument();
    expect(await screen.findByText("The fixture routine stored its result.")).toBeVisible();
    expect(screen.getByRole("heading", { name: "Run result" })).toHaveFocus();
    expect(screen.queryByText("Private routine instructions")).not.toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back to routines" }));
    expect(
      within(await screen.findByRole("region", { name: "Run history" })).getByRole("button", {
        name: /morning summary/i,
      }),
    ).toHaveFocus();
  });

  it("lists run history under the routines and opens a run on click", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    const session = run();
    adapterMocks.listScheduledRunSessions.mockResolvedValue([session]);
    renderView();

    const history = await screen.findByRole("region", { name: "Run history" });
    // The run is labeled with its routine's name (matched via the job id
    // embedded in the cron session id), not the session's own title.
    expect(within(history).getByText("Morning summary")).toBeInTheDocument();
    expect(
      within(history).getByText("Here is today's summary of your unread notes."),
    ).toBeInTheDocument();

    await userEvent.click(within(history).getByRole("button", { name: /morning summary/i }));
    expect(await screen.findByRole("heading", { name: "Run result" })).toBeInTheDocument();
    expect(tauriMocks.listItems).toHaveBeenCalledWith(session.id);
    expect(adapterMocks.listScheduledRunSessions).toHaveBeenCalledWith({
      includeActive: true,
    });
  });

  it("shows the stored error for a failed run", async () => {
    const session = run({ status: "failed", error: "Fixture backend failure" });
    adapterMocks.listScheduledRunSessions.mockResolvedValue([session]);
    renderView();
    const history = await screen.findByRole("region", { name: "Run history" });
    await userEvent.click(within(history).getByRole("button", { name: /morning summary/i }));
    expect(await screen.findByText("Fixture backend failure")).toBeVisible();
    expect(screen.getByText("Failed", { selector: '[role="status"]' })).toBeVisible();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("marks active routine runs in history", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      run({
        active: true,
        preview: "",
        updatedAt: "2026-06-10T09:00:05Z",
      }),
    ]);
    renderView();

    const history = await screen.findByRole("region", { name: "Run history" });
    expect(within(history).getByText("Morning summary")).toBeInTheDocument();
    expect(within(history).getByText("Running")).toBeInTheDocument();
    expect(within(history).getByText("Running now")).toBeInTheDocument();
  });

  it("refreshes run history and routine status while the view stays open", async () => {
    vi.useFakeTimers();
    mocks.listRoutines
      .mockResolvedValueOnce([job({ last_status: "error" })])
      .mockResolvedValueOnce([job({ last_status: "ok" })]);
    adapterMocks.listScheduledRunSessions
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([run({ active: true, preview: "" })]);
    renderView();

    await act(async () => {
      await Promise.resolve();
    });
    const history = screen.getByRole("region", { name: "Run history" });
    expect(within(history).getByText(/No runs yet/)).toBeInTheDocument();
    expect(screen.getByText("Last run failed")).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(screen.getByText("Running")).toBeInTheDocument();
    expect(screen.queryByText("Last run failed")).toBeNull();
  });

  it("keeps polling when terminal event listener registration fails", async () => {
    vi.useFakeTimers();
    eventMocks.listen.mockRejectedValueOnce(new Error("event listener unavailable"));
    mocks.listRoutines
      .mockResolvedValueOnce([job({ last_status: "error" })])
      .mockResolvedValueOnce([job({ last_status: "ok" })]);
    adapterMocks.listScheduledRunSessions
      .mockResolvedValueOnce([])
      .mockResolvedValueOnce([run({ active: true, preview: "" })]);
    renderView();

    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("Last run failed")).toBeInTheDocument();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(screen.getByText("Running")).toBeInTheDocument();
    expect(screen.queryByText("Last run failed")).toBeNull();
  });

  it("does not let a background refresh supersede a visible load", async () => {
    vi.useFakeTimers();
    let resolveFirstLoad: (routines: RoutineJob[]) => void = () => {};
    mocks.listRoutines.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveFirstLoad = resolve;
      }),
    );
    renderView();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });
    expect(mocks.listRoutines).toHaveBeenCalledTimes(1);

    await act(async () => {
      resolveFirstLoad([job({ name: "Visible routine" })]);
      await Promise.resolve();
    });

    expect(screen.getByText("Visible routine")).toBeInTheDocument();
  });

  it("keeps usable routine data when a background refresh fails", async () => {
    vi.useFakeTimers();
    mocks.listRoutines
      .mockResolvedValueOnce([job()])
      .mockRejectedValueOnce(new Error("temporary refresh failure"));
    renderView();

    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("Morning summary")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(screen.getByText("Morning summary")).toBeInTheDocument();
    expect(screen.queryByText("temporary refresh failure")).toBeNull();
  });

  it("ignores stale run history responses after a newer refresh", async () => {
    vi.useFakeTimers();
    mocks.listRoutines.mockResolvedValue([job()]);
    let resolveFirstLoad: (runs: RoutineRunSession[]) => void = () => {};
    adapterMocks.listScheduledRunSessions
      .mockReturnValueOnce(
        new Promise((resolve) => {
          resolveFirstLoad = resolve;
        }),
      )
      .mockResolvedValueOnce([run({ active: true, preview: "" })]);
    renderView();

    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
      await Promise.resolve();
    });
    expect(screen.getByText("Running")).toBeInTheDocument();

    await act(async () => {
      resolveFirstLoad([]);
      await Promise.resolve();
    });

    expect(screen.getByText("Running")).toBeInTheDocument();
  });

  it("labels a run by its session title once the routine is deleted", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      run({
        id: "cron_gone99_20260609_080000",
        title: "Weekly Metrics Digest",
        preview: "Metrics are flat week over week.",
      }),
    ]);
    renderView();

    const history = await screen.findByRole("region", { name: "Run history" });
    expect(within(history).getByText("Weekly Metrics Digest")).toBeInTheDocument();
  });

  it("labels orphaned placeholder-title runs generically", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      run({
        id: "cron_gone99_20260609_080000",
        title: "Untitled session",
        preview: "",
      }),
    ]);
    renderView();

    const history = await screen.findByRole("region", { name: "Run history" });
    expect(within(history).getByText("Routine run")).toBeInTheDocument();
    expect(within(history).queryByText("Untitled session")).toBeNull();
  });

  it("filters run history with the search query", async () => {
    mocks.listRoutines.mockResolvedValue([job(), job({ job_id: "def456", name: "Weekly digest" })]);
    adapterMocks.listScheduledRunSessions.mockResolvedValue([
      run(),
      run({
        id: "cron_def456_20260609_080000",
        preview: "Compiled the weekly digest.",
        updatedAt: "2026-06-09T08:00:30Z",
      }),
    ]);
    renderView();
    await screen.findByRole("region", { name: "Run history" });

    await userEvent.type(screen.getByRole("searchbox"), "weekly");
    const history = screen.getByRole("region", { name: "Run history" });
    expect(within(history).getByText("Weekly digest")).toBeInTheDocument();
    expect(within(history).queryByText("Morning summary")).toBeNull();

    // A query matching no runs hides the section instead of leaving an
    // empty shell under the routines results.
    await userEvent.clear(screen.getByRole("searchbox"));
    await userEvent.type(screen.getByRole("searchbox"), "no such run");
    expect(screen.queryByRole("region", { name: "Run history" })).toBeNull();
  });

  it("shows a quiet hint while no routine has run yet", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    renderView();

    const history = await screen.findByRole("region", { name: "Run history" });
    expect(within(history).getByText(/No runs yet/)).toBeInTheDocument();
  });

  it("keeps routines usable when run history fails to load", async () => {
    mocks.listRoutines.mockResolvedValue([job()]);
    adapterMocks.listScheduledRunSessions.mockRejectedValue(new Error("session store down"));
    renderView();

    expect(await screen.findByText("Morning summary")).toBeInTheDocument();
    const history = screen.getByRole("region", { name: "Run history" });
    expect(within(history).getByText("Run history is unavailable right now.")).toBeInTheDocument();
  });
});
