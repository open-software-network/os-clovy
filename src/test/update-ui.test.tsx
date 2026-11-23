import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/core")>()),
  invoke: vi.fn(async () => {
    throw new Error("no tauri in tests");
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
  emit: vi.fn(async () => undefined),
}));

import { UpdateHub } from "../app/app-effects/update-ui";
import { AppSettings } from "../components/settings/AppSettings";
import { applyInterfaceLocale } from "../i18n/locale";
import type { ClovyUpdate } from "../lib/updater";

const offer = {
  update: { version: "0.1.1", downloadAndInstall: async () => {} } satisfies ClovyUpdate,
  version: "0.1.1",
};

function renderHub(overrides: Partial<Parameters<typeof UpdateHub>[0]> = {}) {
  const handlers = {
    onInstall: vi.fn(),
    onPostpone: vi.fn(),
    onRelaunch: vi.fn(),
    onDismissStatus: vi.fn(),
  };
  render(
    <UpdateHub
      offer={offer}
      accepted={false}
      ready={false}
      waitingForCapture={false}
      status={null}
      failed={false}
      statusLeaving={false}
      checking={false}
      preparing={false}
      relaunching={false}
      progress={null}
      {...handlers}
      {...overrides}
    />,
  );
  return handlers;
}

function renderGeneral(onCheckForUpdates?: () => void) {
  return render(
    <AppSettings
      sourceMode="microphoneOnly"
      checkingSourceReadiness={false}
      onSourceModeChange={() => {}}
      onEnableSystemAudio={() => {}}
      activeTab="general"
      onTabChange={() => {}}
      onCheckForUpdates={onCheckForUpdates}
    />,
  );
}

afterEach(() => {
  cleanup();
  applyInterfaceLocale("en");
});

describe("update offer", () => {
  it("names the new version and waits for the owner", async () => {
    const { onInstall, onPostpone } = renderHub();

    expect(screen.getByText("Clovy 0.1.1 is available")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(onPostpone).toHaveBeenCalledTimes(1);
    expect(onInstall).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Update" }));
    expect(onInstall).toHaveBeenCalledTimes(1);
  });

  it("says the update waits for the recording to end", async () => {
    const { onPostpone } = renderHub({ accepted: true, waitingForCapture: true });

    expect(screen.getByText("Clovy updates when the recording ends.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Later" }));
    expect(onPostpone).toHaveBeenCalledTimes(1);
  });

  it("offers a restart for an update installed and left for later", async () => {
    const { onRelaunch } = renderHub({ ready: true });

    await userEvent.click(screen.getByRole("button", { name: "Restart to update to Clovy 0.1.1" }));
    expect(onRelaunch).toHaveBeenCalledTimes(1);
  });

  it("shows nothing once the offer is gone", () => {
    renderHub({ offer: null });
    expect(screen.queryByRole("status")).toBeNull();
  });
});

describe("updates in Settings > General", () => {
  it("checks on demand in a release build", async () => {
    const check = vi.fn();
    renderGeneral(check);

    expect(screen.getByRole("heading", { name: "Updates" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Check for updates" }));
    expect(check).toHaveBeenCalledTimes(1);
  });

  it("has no update action in a development build", () => {
    renderGeneral(undefined);
    expect(screen.queryByRole("heading", { name: "Updates" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Check for updates" })).toBeNull();
  });

  describe("in Portuguese (Brazil)", () => {
    beforeEach(() => applyInterfaceLocale("pt-BR"));

    it("translates the section and the offer", () => {
      renderGeneral(() => {});
      expect(screen.getByRole("heading", { name: "Atualizações" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Procurar atualizações" })).toBeInTheDocument();
      cleanup();

      renderHub();
      expect(screen.getByText("Clovy 0.1.1 disponível")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Atualizar" })).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Depois" })).toBeInTheDocument();
    });
  });
});
