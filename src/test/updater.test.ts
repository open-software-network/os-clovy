import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DownloadEvent } from "../lib/updater";
import { checkClovyUpdate, relaunchClovy, updatesEnabled } from "../lib/updater";

// A minimal stand-in for @tauri-apps/api/core's Channel: the install command
// drives progress by calling the channel's onmessage, exactly as Rust's
// `Channel::send` does at runtime.
vi.mock("@tauri-apps/api/core", () => {
  class Channel<T> {
    onmessage: ((message: T) => void) | null = null;
  }
  return { Channel, invoke: vi.fn() };
});

const invokeMock = vi.mocked(invoke);

// install_update receives a progress Channel under `onEvent`; invoke's args are
// loosely typed, so name the shape we assert against rather than inline-casting.
type InstallUpdateArgs = {
  onEvent: { onmessage: ((message: DownloadEvent) => void) | null };
};

beforeEach(() => {
  invokeMock.mockReset();
});

describe("updatesEnabled", () => {
  it("asks the native shell whether this build checks for updates", async () => {
    invokeMock.mockResolvedValueOnce(false);

    expect(await updatesEnabled()).toBe(false);
    expect(invokeMock).toHaveBeenCalledWith("updates_enabled");
  });
});

describe("checkClovyUpdate", () => {
  it("returns null when fetch_update reports nothing", async () => {
    invokeMock.mockResolvedValueOnce(null);

    expect(await checkClovyUpdate()).toBeNull();
    // One channel: the endpoint lives in tauri.conf.json, so no arguments.
    expect(invokeMock).toHaveBeenCalledWith("fetch_update");
  });

  it("returns a synthetic update carrying version and notes", async () => {
    invokeMock.mockResolvedValueOnce({ version: "0.1.1", body: "notes" });

    const update = await checkClovyUpdate();

    expect(update?.version).toBe("0.1.1");
    expect(update?.body).toBe("notes");
  });

  it("installs through install_update, forwarding progress events", async () => {
    invokeMock.mockResolvedValueOnce({ version: "0.1.1" });
    const update = await checkClovyUpdate();

    const seen: DownloadEvent[] = [];
    invokeMock.mockImplementationOnce(async (_command, args) => {
      const { onEvent } = args as unknown as InstallUpdateArgs;
      onEvent.onmessage?.({ event: "Started", data: { contentLength: 10 } });
      onEvent.onmessage?.({ event: "Finished" });
    });

    await update?.downloadAndInstall((event) => seen.push(event));

    expect(invokeMock).toHaveBeenLastCalledWith(
      "install_update",
      expect.objectContaining({ onEvent: expect.anything() }),
    );
    expect(seen).toEqual([
      { event: "Started", data: { contentLength: 10 } },
      { event: "Finished" },
    ]);
  });
});

describe("relaunchClovy", () => {
  it("routes through the Rust command that tears down children first", async () => {
    invokeMock.mockResolvedValueOnce(undefined);

    await relaunchClovy();

    // Not a bare process relaunch: the command runs the dictation-helper and
    // agent runtime teardown before restarting so the relaunched instance is
    // not blocked by an orphaned helper.
    expect(invokeMock).toHaveBeenCalledWith("relaunch_for_update");
  });
});
