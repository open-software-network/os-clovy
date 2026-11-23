import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BackgroundSettings } from "../components/settings/BackgroundSettings";
import { getStoredAppBackground, setStoredAppBackground } from "../lib/app-background";
import { applyInterfaceLocale } from "../i18n/locale";
const api = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => api);
beforeEach(() => {
  localStorage.clear();
  setStoredAppBackground("drift");
  applyInterfaceLocale("en");
  api.invoke.mockReset().mockResolvedValue(null);
});
afterEach(cleanup);
it("chooses a local copy, shows its name and removes it to None", async () => {
  api.invoke.mockImplementation(async (command) =>
    command === "choose_background_image"
      ? { fileName: "lake.png", dataUrl: "data:image/png;base64,AA==" }
      : null,
  );
  await act(async () => {
    render(<BackgroundSettings />);
  });
  fireEvent.click(screen.getByRole("button", { name: "Image" }));
  await screen.findByText("lake.png");
  expect(getStoredAppBackground()).toBe("image");
  fireEvent.click(screen.getByRole("button", { name: "Remove" }));
  await waitFor(() => expect(getStoredAppBackground()).toBe("none"));
  expect(api.invoke).toHaveBeenCalledWith("remove_background_image");
});
it("keeps the previous choice on cancellation or rejected replacement", async () => {
  await act(async () => {
    render(<BackgroundSettings />);
  });
  fireEvent.click(screen.getByRole("button", { name: "Image" }));
  await waitFor(() => expect(api.invoke).toHaveBeenCalledWith("choose_background_image"));
  expect(getStoredAppBackground()).toBe("drift");
  api.invoke.mockRejectedValue({ code: "background_invalid" });
  fireEvent.click(screen.getByRole("button", { name: "Image" }));
  await screen.findByRole("alert");
  expect(getStoredAppBackground()).toBe("drift");
});
it("shows missing image with choose and remove actions", async () => {
  setStoredAppBackground("image");
  api.invoke.mockResolvedValue({ fileName: "lost.webp", dataUrl: null });
  await act(async () => {
    render(<BackgroundSettings />);
  });
  await screen.findByText("lost.webp");
  expect(screen.getByText("Image unavailable. Choose another file.")).toBeTruthy();
  expect(screen.getByRole("button", { name: "Choose image" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Remove" })).toBeTruthy();
});

it("keeps the selected image and its name if a replacement is rejected", async () => {
  setStoredAppBackground("image");
  api.invoke.mockResolvedValue({
    fileName: "previous.jpg",
    dataUrl: "data:image/jpeg;base64,AA==",
  });
  await act(async () => {
    render(<BackgroundSettings />);
  });
  await screen.findByText("previous.jpg");
  api.invoke.mockRejectedValue({ code: "background_invalid" });
  fireEvent.click(screen.getByRole("button", { name: "Choose image" }));
  await screen.findByRole("alert");
  expect(getStoredAppBackground()).toBe("image");
  expect(screen.getByText("previous.jpg")).toBeTruthy();
});
