import { describe, expect, it, vi } from "vitest";
import {
  INITIAL_UPDATE_STATUS_DISPLAY,
  UP_TO_DATE_STATUS,
  checkForClovyUpdate,
  nextUpdateStep,
  prepareClovyUpdate,
  updateCheckShowsStatus,
  updateStatusDisplayReducer,
  type UpdaterUpdate,
} from "../app/update-decision";

describe("update status display", () => {
  it("only exposes manual checks as visible busy statuses", () => {
    expect(updateCheckShowsStatus("manual")).toBe(true);
    expect(updateCheckShowsStatus("launch")).toBe(false);
  });

  it("does not let stale success timers fade or clear a newer status", () => {
    const success = updateStatusDisplayReducer(INITIAL_UPDATE_STATUS_DISPLAY, {
      type: "show",
      status: UP_TO_DATE_STATUS,
    });
    const leaving = updateStatusDisplayReducer(success, { type: "beginUpToDateExit" });
    expect(leaving.leaving).toBe(true);

    const checking = updateStatusDisplayReducer(leaving, {
      type: "show",
      status: "Checking for updates...",
    });
    expect(checking).toEqual({
      status: "Checking for updates...",
      leaving: false,
      failed: false,
    });
    expect(updateStatusDisplayReducer(checking, { type: "clearUpToDate" })).toBe(checking);
  });

  it("carries failure styling independently of message wording", () => {
    const failure = updateStatusDisplayReducer(INITIAL_UPDATE_STATUS_DISPLAY, {
      type: "show",
      status: "Could not check for updates.",
      failed: true,
    });

    expect(failure.failed).toBe(true);
  });
});

function update(body?: string): UpdaterUpdate {
  return {
    version: "0.2.0",
    body,
    downloadAndInstall: vi.fn(async (onEvent) => {
      onEvent?.({ event: "Started", data: { contentLength: 100 } });
      onEvent?.({ event: "Progress", data: { chunkLength: 40 } });
      onEvent?.({ event: "Finished" });
    }),
  };
}

describe("checkForClovyUpdate", () => {
  it("offers the version and release notes when an update is available", async () => {
    const offer = vi.fn();

    await checkForClovyUpdate(
      {
        check: async () => update(" Fixes transcription. "),
        offer,
        reportNoUpdate: vi.fn(),
        reportFailure: vi.fn(),
      },
      "launch",
    );

    expect(offer).toHaveBeenCalledWith(
      expect.objectContaining({
        version: "0.2.0",
        notes: "Fixes transcription.",
      }),
    );
  });

  it("stays silent at launch when there is no update", async () => {
    const offer = vi.fn();
    const reportNoUpdate = vi.fn();

    await checkForClovyUpdate(
      {
        check: async () => null,
        offer,
        reportNoUpdate,
        reportFailure: vi.fn(),
      },
      "launch",
    );

    expect(offer).not.toHaveBeenCalled();
    expect(reportNoUpdate).not.toHaveBeenCalled();
  });

  it("reports no update for a manual check", async () => {
    const reportNoUpdate = vi.fn();

    await checkForClovyUpdate(
      {
        check: async () => null,
        offer: vi.fn(),
        reportNoUpdate,
        reportFailure: vi.fn(),
      },
      "manual",
    );

    expect(reportNoUpdate).toHaveBeenCalledTimes(1);
  });

  it("keeps a failed launch check silent", async () => {
    const reportFailure = vi.fn();

    await checkForClovyUpdate(
      {
        check: async () => {
          throw new Error("error sending request");
        },
        offer: vi.fn(),
        reportNoUpdate: vi.fn(),
        reportFailure,
      },
      "launch",
    );

    expect(reportFailure).not.toHaveBeenCalled();
  });

  it("reports a failed manual check without offering anything", async () => {
    const offer = vi.fn();
    const reportFailure = vi.fn();

    await checkForClovyUpdate(
      {
        check: async () => {
          throw new Error("signature mismatch");
        },
        offer,
        reportNoUpdate: vi.fn(),
        reportFailure,
      },
      "manual",
    );

    expect(offer).not.toHaveBeenCalled();
    expect(reportFailure).toHaveBeenCalledWith("signature mismatch");
  });

  it("reports the message of a native update error", async () => {
    const reportFailure = vi.fn();

    await checkForClovyUpdate(
      {
        check: async () => {
          throw {
            code: "update_check_failed",
            message: "Could not fetch a valid release JSON from the remote",
          };
        },
        offer: vi.fn(),
        reportNoUpdate: vi.fn(),
        reportFailure,
      },
      "manual",
    );

    expect(reportFailure).toHaveBeenCalledWith(
      "Could not fetch a valid release JSON from the remote",
    );
  });

  it("uses neutral detail for an unstructured failure", async () => {
    const reportFailure = vi.fn();

    await checkForClovyUpdate(
      {
        check: async () => {
          throw {};
        },
        offer: vi.fn(),
        reportNoUpdate: vi.fn(),
        reportFailure,
      },
      "manual",
    );

    expect(reportFailure).toHaveBeenCalledWith("An unknown error occurred.");
  });
});

describe("prepareClovyUpdate", () => {
  it("reports download progress and marks the update ready without relaunching", async () => {
    const candidate = update(" Ready after relaunch. ");
    const reportProgress = vi.fn();
    const reportReady = vi.fn();

    await prepareClovyUpdate({
      update: candidate,
      reportProgress,
      reportReady,
      reportFailure: vi.fn(),
    });

    expect(candidate.downloadAndInstall).toHaveBeenCalledTimes(1);
    expect(reportProgress).toHaveBeenCalledWith({
      state: "downloading",
      downloadedBytes: 40,
      contentLength: 100,
    });
    expect(reportProgress).toHaveBeenCalledWith({
      state: "installing",
      downloadedBytes: 40,
      contentLength: 100,
    });
    expect(reportReady).toHaveBeenCalledWith({
      update: candidate,
      version: "0.2.0",
      notes: "Ready after relaunch.",
    });
  });

  it("uses neutral detail for an unstructured install failure", async () => {
    const reportFailure = vi.fn();

    await prepareClovyUpdate({
      update: {
        ...update(),
        downloadAndInstall: async () => {
          throw {};
        },
      },
      reportProgress: vi.fn(),
      reportReady: vi.fn(),
      reportFailure,
    });

    expect(reportFailure).toHaveBeenCalledWith("An unknown error occurred.");
  });
});

describe("nextUpdateStep", () => {
  const idle = {
    offered: true,
    accepted: false,
    preparing: false,
    ready: false,
    relaunching: false,
    captureActive: false,
  };

  it("downloads nothing until the owner accepts", () => {
    expect(nextUpdateStep(idle)).toBe("none");
  });

  it("downloads and installs once accepted with nothing recording", () => {
    expect(nextUpdateStep({ ...idle, accepted: true })).toBe("install");
  });

  it("waits for a recording or dictation to end before installing", () => {
    expect(nextUpdateStep({ ...idle, accepted: true, captureActive: true })).toBe("wait");
  });

  it("relaunches an installed update only when nothing is recording", () => {
    const installed = { ...idle, accepted: true, ready: true };
    expect(nextUpdateStep(installed)).toBe("relaunch");
    expect(nextUpdateStep({ ...installed, captureActive: true })).toBe("wait");
  });

  it("leaves an installed update for later when the owner postpones the restart", () => {
    expect(nextUpdateStep({ ...idle, ready: true })).toBe("none");
  });

  it("never starts a second step while one is running", () => {
    expect(nextUpdateStep({ ...idle, accepted: true, preparing: true })).toBe("none");
    expect(nextUpdateStep({ ...idle, accepted: true, ready: true, relaunching: true })).toBe(
      "none",
    );
  });
});
