import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const tauriMocks = vi.hoisted(() => ({
  computerUseStatus: vi.fn(),
  computerUseRequestPermissions: vi.fn(),
  setComputerUseGrant: vi.fn(),
  computerUseStop: vi.fn(),
  openPrivacySettings: vi.fn(),
  setComputerUsePermissionDragBounds: vi.fn(),
}));

vi.mock("../lib/tauri", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../lib/tauri")>()),
  computerUseStatus: tauriMocks.computerUseStatus,
  computerUseRequestPermissions: tauriMocks.computerUseRequestPermissions,
  setComputerUseGrant: tauriMocks.setComputerUseGrant,
  computerUseStop: tauriMocks.computerUseStop,
  openPrivacySettings: tauriMocks.openPrivacySettings,
  setComputerUsePermissionDragBounds: tauriMocks.setComputerUsePermissionDragBounds,
}));

import { ComputerUseControl } from "../components/plugins/ComputerUseControl";
import { SETTINGS_TABS } from "../components/settings/AppSettings";
import type { ComputerUseStatusDto } from "../lib/tauri";

function status(overrides: Partial<ComputerUseStatusDto> = {}): ComputerUseStatusDto {
  return {
    platformSupported: true,
    grantEnabled: false,
    driverAvailable: true,
    driverVersion: "1.0.2",
    accessibility: false,
    screenRecording: false,
    ready: false,
    state: "off",
    ...overrides,
  };
}

beforeEach(() => {
  tauriMocks.computerUseStatus.mockResolvedValue(status());
  tauriMocks.computerUseRequestPermissions.mockResolvedValue(
    status({ grantEnabled: true, state: "permission_missing" }),
  );
  tauriMocks.setComputerUseGrant.mockResolvedValue(
    status({ grantEnabled: true, state: "permission_missing" }),
  );
  tauriMocks.computerUseStop.mockResolvedValue({ stopped: true });
  tauriMocks.openPrivacySettings.mockResolvedValue(undefined);
  tauriMocks.setComputerUsePermissionDragBounds.mockResolvedValue(undefined);
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ComputerUseControl", () => {
  it("shows Windows desktop requirements and preserves Stop without macOS setup", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ platform: "windows", grantEnabled: true, state: "permission_missing" }),
    );
    render(<ComputerUseControl />);
    expect(await screen.findByText("Desktop unavailable")).toBeInTheDocument();
    expect(screen.getByText(/Unlock your Windows session/)).toBeInTheDocument();
    expect(screen.getByText(/Uses the background when supported/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open Accessibility settings" })).toBeNull();
    expect(screen.queryByText("Screen recording")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Stop current task" }));
    expect(tauriMocks.computerUseStop).toHaveBeenCalledOnce();
    expect(tauriMocks.computerUseRequestPermissions).not.toHaveBeenCalled();
  });

  it("enables the Clovy grant without prompting for macOS access", async () => {
    render(<ComputerUseControl />);
    const toggle = await screen.findByRole("switch", { name: "Enable Computer use" });

    await userEvent.click(toggle);

    expect(tauriMocks.setComputerUseGrant).toHaveBeenCalledWith(true);
    expect(tauriMocks.openPrivacySettings).not.toHaveBeenCalled();
    expect(screen.queryByText(/Computer use is enabled/)).toBeNull();
  });

  it("guides the user through Accessibility as the first macOS step", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, state: "permission_missing" }),
    );
    render(<ComputerUseControl />);

    expect(await screen.findByText("Allow Accessibility")).toBeInTheDocument();
    expect(screen.getByText("Screen recording")).toBeInTheDocument();
    expect(screen.queryByText(/Captures are never analytics/)).not.toBeInTheDocument();
    expect(
      screen.getByText((_, element) =>
        Boolean(
          element?.tagName === "P" &&
            element.textContent?.includes(
              "Open System Settings, find Clovy Computer Use Driver, and turn it on.",
            ),
        ),
      ),
    ).toBeInTheDocument();

    await userEvent.hover(
      screen.getByRole("button", { name: "Computer use privacy and permissions" }),
    );
    expect(await screen.findByRole("tooltip")).toHaveTextContent(/Captures are never analytics/);

    await userEvent.click(screen.getByRole("button", { name: "Open Accessibility settings" }));
    expect(tauriMocks.computerUseRequestPermissions).toHaveBeenCalledTimes(1);
    expect(tauriMocks.openPrivacySettings).toHaveBeenCalledWith("accessibility");
    expect(screen.queryByRole("button", { name: "Continue to macOS access" })).toBeNull();
  });

  it("advances to Screen recording after Accessibility is allowed", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, accessibility: true, state: "permission_missing" }),
    );
    render(<ComputerUseControl />);

    expect(await screen.findByText("Step 2 of 2")).toBeInTheDocument();
    expect(screen.getByText("Allow Screen recording")).toBeInTheDocument();
    expect(screen.getByText(/assigns Screen recording to Clovy itself/)).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: "Drag Clovy to the open System Settings list",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Drag Clovy below/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Open Screen recording settings" }));
    expect(tauriMocks.computerUseRequestPermissions).toHaveBeenCalledTimes(1);
    expect(tauriMocks.openPrivacySettings).toHaveBeenCalledWith("screenRecording");
  });

  it("coalesces repeated permission probes before opening Screen recording settings", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, accessibility: true, state: "permission_missing" }),
    );
    let finishPermissionProbe: ((value: ComputerUseStatusDto) => void) | undefined;
    tauriMocks.computerUseRequestPermissions.mockImplementationOnce(
      () =>
        new Promise<ComputerUseStatusDto>((resolve) => {
          finishPermissionProbe = resolve;
        }),
    );
    render(<ComputerUseControl />);

    const button = await screen.findByRole("button", { name: "Open Screen recording settings" });
    await userEvent.click(button);
    await userEvent.click(button);

    expect(tauriMocks.computerUseRequestPermissions).toHaveBeenCalledTimes(1);
    expect(tauriMocks.openPrivacySettings).not.toHaveBeenCalled();

    const readyStatus = status({
      grantEnabled: true,
      accessibility: true,
      screenRecording: true,
      ready: true,
      state: "ready",
    });
    tauriMocks.computerUseStatus.mockResolvedValue(readyStatus);
    finishPermissionProbe?.(readyStatus);
    await waitFor(() => expect(tauriMocks.openPrivacySettings).toHaveBeenCalledTimes(2));
    expect(tauriMocks.openPrivacySettings).toHaveBeenLastCalledWith("screenRecording");
    await screen.findByText("Ready");
  });

  it("waits for each status poll before scheduling the next one", async () => {
    vi.useFakeTimers();
    try {
      const incomplete = status({ grantEnabled: true, state: "permission_missing" });
      let finishPoll: ((value: ComputerUseStatusDto) => void) | undefined;
      tauriMocks.computerUseStatus
        .mockResolvedValueOnce(incomplete)
        .mockImplementationOnce(
          () =>
            new Promise<ComputerUseStatusDto>((resolve) => {
              finishPoll = resolve;
            }),
        )
        .mockResolvedValue(incomplete);

      render(<ComputerUseControl />);
      await act(async () => {
        await Promise.resolve();
      });
      expect(tauriMocks.computerUseStatus).toHaveBeenCalledTimes(1);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(tauriMocks.computerUseStatus).toHaveBeenCalledTimes(2);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });
      expect(tauriMocks.computerUseStatus).toHaveBeenCalledTimes(2);

      await act(async () => {
        finishPoll?.(incomplete);
        await Promise.resolve();
      });
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(tauriMocks.computerUseStatus).toHaveBeenCalledTimes(3);
    } finally {
      vi.useRealTimers();
    }
  });

  it("pauses status polling while the settings page is hidden", async () => {
    vi.useFakeTimers();
    let visibility: DocumentVisibilityState = "visible";
    const visibilitySpy = vi
      .spyOn(document, "visibilityState", "get")
      .mockImplementation(() => visibility);
    try {
      const incomplete = status({ grantEnabled: true, state: "permission_missing" });
      tauriMocks.computerUseStatus.mockResolvedValue(incomplete);
      render(<ComputerUseControl />);
      await act(async () => {
        await Promise.resolve();
      });

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(tauriMocks.computerUseStatus).toHaveBeenCalledTimes(2);

      visibility = "hidden";
      act(() => document.dispatchEvent(new Event("visibilitychange")));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });
      expect(tauriMocks.computerUseStatus).toHaveBeenCalledTimes(2);

      visibility = "visible";
      await act(async () => {
        document.dispatchEvent(new Event("visibilitychange"));
        await Promise.resolve();
      });
      expect(tauriMocks.computerUseStatus).toHaveBeenCalledTimes(3);
    } finally {
      visibilitySpy.mockRestore();
      vi.useRealTimers();
    }
  });

  it("coalesces a permission probe across an unmount and remount", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, accessibility: true, state: "permission_missing" }),
    );
    let finishPermissionProbe: ((value: ComputerUseStatusDto) => void) | undefined;
    tauriMocks.computerUseRequestPermissions.mockImplementationOnce(
      () =>
        new Promise<ComputerUseStatusDto>((resolve) => {
          finishPermissionProbe = resolve;
        }),
    );
    const first = render(<ComputerUseControl />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Open Screen recording settings" }),
    );

    first.unmount();
    render(<ComputerUseControl />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Open Screen recording settings" }),
    );

    expect(tauriMocks.computerUseRequestPermissions).toHaveBeenCalledTimes(1);
    const readyStatus = status({
      grantEnabled: true,
      accessibility: true,
      screenRecording: true,
      ready: true,
      state: "ready",
    });
    tauriMocks.computerUseStatus.mockResolvedValue(readyStatus);
    finishPermissionProbe?.(readyStatus);
    await screen.findByText("Ready");
  });

  it("queues a newly reached permission pane behind an active probe", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, state: "permission_missing" }),
    );
    let finishAccessibilityProbe: ((value: ComputerUseStatusDto) => void) | undefined;
    let finishScreenRecordingProbe: ((value: ComputerUseStatusDto) => void) | undefined;
    tauriMocks.computerUseRequestPermissions
      .mockImplementationOnce(
        () =>
          new Promise<ComputerUseStatusDto>((resolve) => {
            finishAccessibilityProbe = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise<ComputerUseStatusDto>((resolve) => {
            finishScreenRecordingProbe = resolve;
          }),
      );
    const first = render(<ComputerUseControl />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Open Accessibility settings" }),
    );

    first.unmount();
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, accessibility: true, state: "permission_missing" }),
    );
    render(<ComputerUseControl />);
    await userEvent.click(
      await screen.findByRole("button", { name: "Open Screen recording settings" }),
    );

    expect(tauriMocks.computerUseRequestPermissions).toHaveBeenCalledTimes(1);
    finishAccessibilityProbe?.(
      status({ grantEnabled: true, accessibility: true, state: "permission_missing" }),
    );
    await waitFor(() => expect(tauriMocks.computerUseRequestPermissions).toHaveBeenCalledTimes(2));
    const readyStatus = status({
      grantEnabled: true,
      accessibility: true,
      screenRecording: true,
      ready: true,
      state: "ready",
    });
    tauriMocks.computerUseStatus.mockResolvedValue(readyStatus);
    finishScreenRecordingProbe?.(readyStatus);
    await screen.findByText("Ready");
  });

  it("keeps Accessibility labeled as step 1 when Screen recording was allowed first", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, screenRecording: true, state: "permission_missing" }),
    );
    render(<ComputerUseControl />);

    expect(await screen.findByText("Step 1 of 2")).toBeInTheDocument();
    expect(screen.getByText("Allow Accessibility")).toBeInTheDocument();
  });

  it("offers the real helper app as a drag source when macOS access is missing", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, state: "permission_missing" }),
    );
    render(<ComputerUseControl />);

    expect(await screen.findByText("Driver is not in the list?")).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: "Drag Clovy Computer Use Driver to the open System Settings list",
      }),
    ).toBeInTheDocument();
    expect(screen.getByText(/Drag the helper below/)).toBeInTheDocument();
  });

  it("registers the permission owner represented by each drag card", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, state: "permission_missing" }),
    );
    const { unmount } = render(<ComputerUseControl />);

    await screen.findByRole("button", {
      name: "Drag Clovy Computer Use Driver to the open System Settings list",
    });
    await waitFor(() =>
      expect(tauriMocks.setComputerUsePermissionDragBounds).toHaveBeenCalledWith(
        expect.objectContaining({ width: expect.any(Number), height: expect.any(Number) }),
        "helper",
      ),
    );

    unmount();
    tauriMocks.setComputerUsePermissionDragBounds.mockClear();
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ grantEnabled: true, accessibility: true, state: "permission_missing" }),
    );
    render(<ComputerUseControl />);

    await screen.findByRole("button", {
      name: "Drag Clovy to the open System Settings list",
    });
    await waitFor(() =>
      expect(tauriMocks.setComputerUsePermissionDragBounds).toHaveBeenCalledWith(
        expect.objectContaining({ width: expect.any(Number), height: expect.any(Number) }),
        "host",
      ),
    );
  });

  it("does not expose helper transport errors while permissions are incomplete", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({
        grantEnabled: true,
        state: "permission_missing",
        error: "Broken pipe (os error 32)",
      }),
    );
    render(<ComputerUseControl />);

    expect(await screen.findByText("Allow Accessibility")).toBeInTheDocument();
    expect(screen.queryByText(/Broken pipe/)).toBeNull();
  });

  it("shows helper errors that are not permission setup failures", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({
        grantEnabled: true,
        state: "error",
        error: "The Computer use driver did not respond in time.",
      }),
    );
    render(<ComputerUseControl />);

    expect(await screen.findByText(/driver did not respond in time/)).toBeInTheDocument();
  });

  it("keeps the switch unavailable on unsupported platforms", async () => {
    tauriMocks.computerUseStatus.mockResolvedValue(
      status({ platformSupported: false, driverAvailable: false, state: "unsupported" }),
    );
    render(<ComputerUseControl />);

    const toggle = await screen.findByRole("switch", { name: "Enable Computer use" });
    await waitFor(() => expect(toggle).toBeDisabled());
    expect(screen.getByText(/requires macOS or Windows/)).toBeInTheDocument();
  });

  it("renders Computer use as a plugin row without a Pro tag", async () => {
    render(<ComputerUseControl />);

    expect(await screen.findByRole("switch", { name: "Enable Computer use" })).toBeInTheDocument();
    expect(screen.getByText("Computer use").closest("li")).toHaveClass("connector-row");
    expect(screen.queryByText("Pro")).not.toBeInTheDocument();
    expect(SETTINGS_TABS).toContainEqual({ id: "connectors", label: "Plugins" });
  });

  describe("without plan, vision model or rollout (ADR-0067)", () => {
    // What the backend reports now: only local facts.
    const local = (overrides: Record<string, unknown> = {}) =>
      ({
        platformSupported: true,
        grantEnabled: true,
        driverAvailable: true,
        driverVersion: "0.5.0",
        accessibility: true,
        screenRecording: true,
        ready: true,
        state: "ready",
        ...overrides,
      }) as unknown as ComputerUseStatusDto;

    it("is Ready from the grant, the driver and macOS access alone", async () => {
      tauriMocks.computerUseStatus.mockResolvedValue(local());
      render(<ComputerUseControl />);

      expect(await screen.findByText("Ready")).toBeInTheDocument();
      expect(screen.queryByText("Vision-capable model")).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Choose model" })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "View plans" })).not.toBeInTheDocument();
    });

    it("lets the user turn it on with no Clovy account state", async () => {
      tauriMocks.computerUseStatus.mockResolvedValue(
        local({
          grantEnabled: false,
          ready: false,
          accessibility: false,
          screenRecording: false,
          state: "off",
        }),
      );
      render(<ComputerUseControl />);

      const toggle = await screen.findByRole("switch", { name: "Enable Computer use" });
      await waitFor(() => expect(toggle).toBeEnabled());
      for (const gone of [
        "Pro plan required",
        "Needs a vision model",
        "Temporarily unavailable",
        "Pro feature",
      ]) {
        expect(screen.queryByText(gone)).not.toBeInTheDocument();
      }
    });

    it("says captures go to the agent CLI running the turn", async () => {
      tauriMocks.computerUseStatus.mockResolvedValue(local());
      render(<ComputerUseControl />);

      await userEvent.hover(
        await screen.findByRole("button", { name: "Computer use privacy and permissions" }),
      );
      expect(await screen.findByText(/agent CLI running the turn/)).toBeInTheDocument();
      expect(screen.queryByText(/your selected model/)).not.toBeInTheDocument();
    });
  });
});
