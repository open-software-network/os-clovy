import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({ get: vi.fn(), set: vi.fn() }));
vi.mock("../lib/tauri", async (original) => ({
  ...(await original<typeof import("../lib/tauri")>()),
  appPreferencesGet: mocks.get,
  appPreferencesSet: mocks.set,
}));
vi.mock("../lib/platform", async (original) => ({
  ...(await original<typeof import("../lib/platform")>()),
  isMacLikePlatform: () => true,
}));
vi.mock("../lib/autostart", () => ({
  autostartSupported: () => false,
  autostartEnabled: vi.fn(),
  setAutostartEnabled: vi.fn(),
}));
import { StartupSettingsSection } from "../components/settings/AppSettings";

describe("Dock preference", () => {
  it("persists the switch value", async () => {
    mocks.get.mockResolvedValue({ hideFromDock: false });
    mocks.set.mockResolvedValue({ hideFromDock: true });
    render(<StartupSettingsSection />);
    const toggle = await screen.findByRole("switch", { name: "Hide from Dock" });
    await waitFor(() => expect(toggle).toBeEnabled());
    await userEvent.click(toggle);
    await waitFor(() => expect(toggle).toBeChecked());
    expect(mocks.set).toHaveBeenCalledWith({ hideFromDock: true });
  });

  it("saves and reports a failed update even without autostart", async () => {
    mocks.set.mockClear();
    mocks.get.mockResolvedValue({ hideFromDock: false });
    mocks.set.mockRejectedValue(new Error("write failed"));
    render(<StartupSettingsSection />);
    const toggle = await screen.findByRole("switch", { name: "Hide from Dock" });
    await waitFor(() => expect(toggle).toBeEnabled());
    await userEvent.click(toggle);
    expect(mocks.set).toHaveBeenCalledWith({ hideFromDock: true });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not update the Dock preference",
    );
    expect(toggle).not.toBeChecked();
  });
});
