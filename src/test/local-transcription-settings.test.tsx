import { render, screen, waitFor, cleanup } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { LocalTranscriptionModelState } from "../lib/tauri";
import { applyInterfaceLocale } from "../i18n/locale";
const api = vi.hoisted(() => ({
  state: vi.fn(),
  install: vi.fn(),
  cancel: vi.fn(),
  remove: vi.fn(),
  listen: vi.fn(),
}));
vi.mock("../lib/tauri", () => ({
  localTranscriptionModelState: api.state,
  installLocalTranscriptionModel: api.install,
  cancelLocalTranscriptionModelInstall: api.cancel,
  removeLocalTranscriptionModel: api.remove,
  onLocalTranscriptionModelState: api.listen,
}));
import { LocalTranscriptionSettings } from "../components/settings/LocalTranscriptionSettings";
const props = () => ({
  supported: true,
  provider: "groq",
  onSelect: vi.fn().mockResolvedValue(undefined),
  onRemoved: vi.fn().mockResolvedValue(undefined),
});
beforeEach(() => {
  vi.clearAllMocks();
  applyInterfaceLocale("pt-BR");
  api.install.mockResolvedValue(undefined);
  api.cancel.mockResolvedValue(undefined);
  api.state.mockResolvedValue({ state: "absent" });
  api.listen.mockResolvedValue(() => {});
});
afterEach(() => {
  cleanup();
  applyInterfaceLocale("en");
});
it("hides Local on unsupported platforms", () => {
  render(<LocalTranscriptionSettings {...props()} supported={false} />);
  expect(screen.queryByRole("radio")).toBeNull();
  expect(api.state).not.toHaveBeenCalled();
});
it("requires installation before Local can be selected", async () => {
  const p = props();
  render(<LocalTranscriptionSettings {...p} />);
  expect(await screen.findByRole("radio", { name: /Local/ })).toBeDisabled();
  expect(screen.getByText("Instale o modelo para usar")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: /Instalar modelo/ }));
  expect(api.install).toHaveBeenCalledOnce();
  expect(p.onSelect).not.toHaveBeenCalled();
});
it("shows byte progress, cancels, then allows a retry", async () => {
  let progress: (state: LocalTranscriptionModelState) => void = () => {};
  api.listen.mockImplementation(async (listener) => {
    progress = listener;
    return () => {};
  });
  api.state.mockResolvedValue({ state: "downloading", received: 123000000, total: 632162964 });
  render(<LocalTranscriptionSettings {...props()} />);
  expect(await screen.findByRole("progressbar")).toHaveAttribute("value", "123000000");
  expect(screen.getByText(/123.*632/)).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Cancelar" }));
  expect(api.cancel).toHaveBeenCalledOnce();
  const { act } = await import("@testing-library/react");
  act(() => progress({ state: "absent" }));
  expect(await screen.findByRole("button", { name: /Instalar modelo/ })).toBeEnabled();
});
it("selects ready Local and warns when removing the active model", async () => {
  api.state.mockResolvedValue({ state: "ready", bytes: 632162964 });
  api.remove.mockResolvedValue({ switched_to_groq: true });
  const p = props();
  render(<LocalTranscriptionSettings {...p} provider="local" />);
  expect(await screen.findByRole("radio", { name: /Local/ })).toBeChecked();
  expect(screen.getByText(/Pronto/)).toBeVisible();
  await userEvent.click(screen.getByRole("radio", { name: /Groq/ }));
  expect(p.onSelect).toHaveBeenCalledWith("whisper-large-v3-turbo");
  await userEvent.click(screen.getByRole("button", { name: "Remover" }));
  expect(await screen.findByText(/mudou para Groq/)).toBeVisible();
  expect(p.onRemoved).toHaveBeenCalledOnce();
});
it("keeps successful removal visible when refreshing provider settings fails", async () => {
  api.state.mockResolvedValue({ state: "ready", bytes: 632162964 });
  api.remove.mockResolvedValue({ switched_to_groq: true });
  const p = props();
  p.onRemoved.mockRejectedValue(new Error("Provider refresh failed"));
  render(<LocalTranscriptionSettings {...p} provider="local" />);
  await userEvent.click(await screen.findByRole("button", { name: "Remover" }));
  expect(await screen.findByText(/mudou para Groq/)).toBeVisible();
  expect(screen.getByRole("radio", { name: /Local/ })).toBeDisabled();
  expect(screen.getByRole("button", { name: /Instalar modelo/ })).toBeEnabled();
  expect(api.state).toHaveBeenCalledOnce();
  expect(p.onRemoved).toHaveBeenCalledWith({ switched_to_groq: true });
});
it("preserves the removal error if refreshing provider settings also fails", async () => {
  api.state.mockResolvedValueOnce({ state: "ready", bytes: 632162964 });
  api.state.mockResolvedValue({ state: "absent" });
  api.remove.mockRejectedValue(new Error("Files could not be removed"));
  const p = props();
  p.onRemoved.mockRejectedValue(new Error("Provider refresh failed"));
  render(<LocalTranscriptionSettings {...p} provider="local" />);
  await userEvent.click(await screen.findByRole("button", { name: "Remover" }));
  expect(await screen.findByText("Files could not be removed")).toBeVisible();
  expect(screen.getByRole("radio", { name: /Local/ })).toBeDisabled();
});
it("shows the failed file and retries installation", async () => {
  api.state.mockResolvedValue({ state: "failed", reason: "Encoder hash mismatch" });
  render(<LocalTranscriptionSettings {...props()} />);
  expect(await screen.findByText(/Encoder hash mismatch/)).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "Tentar de novo" }));
  await waitFor(() => expect(api.install).toHaveBeenCalledOnce());
});
it("explains network failures in the interface language", async () => {
  api.state.mockResolvedValue({
    state: "failed",
    reason: "network: Encoder.mlmodelc/weights/weight.bin",
  });
  render(<LocalTranscriptionSettings {...props()} />);
  expect(await screen.findByText(/Verifique sua conexão/)).toBeVisible();
  expect(screen.getByText(/Encoder.mlmodelc/)).toBeVisible();
});
it("does not repeat the raw network error when installation rejects", async () => {
  const reason = "network: Encoder.mlmodelc/weights/weight.bin";
  api.state
    .mockResolvedValueOnce({ state: "absent" })
    .mockResolvedValue({ state: "failed", reason });
  api.install.mockRejectedValue({ code: "local_model_install_failed", message: reason });
  render(<LocalTranscriptionSettings {...props()} />);
  await userEvent.click(await screen.findByRole("button", { name: /Instalar modelo/ }));
  expect(await screen.findByText(/Verifique sua conexão/)).toBeVisible();
  expect(screen.queryByText(/^network:/)).toBeNull();
});
it("refreshes the selected engine even if deleting model files fails", async () => {
  api.state.mockResolvedValue({ state: "ready", bytes: 632162964 });
  api.remove.mockRejectedValue(new Error("Files could not be removed"));
  const p = props();
  render(<LocalTranscriptionSettings {...p} provider="local" />);
  await userEvent.click(await screen.findByRole("button", { name: "Remover" }));
  expect(await screen.findByText("Files could not be removed")).toBeVisible();
  expect(p.onRemoved).toHaveBeenCalledOnce();
});

it.each([
  false,
  true,
])("disables stale ready Local after removal fails (snapshot unavailable: %s)", async (snapshotUnavailable) => {
  api.state.mockResolvedValueOnce({ state: "ready", bytes: 632162964 });
  if (snapshotUnavailable) api.state.mockRejectedValue(new Error("Snapshot unavailable"));
  else api.state.mockResolvedValue({ state: "failed", reason: "Files could not be removed" });
  api.remove.mockRejectedValue(new Error("Files could not be removed"));
  const p = props();
  render(<LocalTranscriptionSettings {...p} provider="local" />);
  await userEvent.click(await screen.findByRole("button", { name: "Remover" }));
  expect(await screen.findByText("Files could not be removed")).toBeVisible();
  await waitFor(() => expect(screen.getByRole("radio", { name: /Local/ })).toBeDisabled());
  expect(screen.queryByText(/Pronto/)).toBeNull();
  expect(api.state).toHaveBeenCalledTimes(2);
  expect(p.onRemoved).toHaveBeenCalledOnce();
  expect(
    screen.getByRole("button", {
      name: snapshotUnavailable ? /Instalar modelo/ : "Tentar de novo",
    }),
  ).toBeEnabled();
});

it("restores the Groq model selected before switching to Local", async () => {
  api.state.mockResolvedValue({ state: "ready", bytes: 632162964 });
  const p = props();
  const { rerender } = render(<LocalTranscriptionSettings {...p} model="whisper-large-v3" />);
  await userEvent.click(await screen.findByRole("radio", { name: /Local/ }));
  expect(p.onSelect).toHaveBeenLastCalledWith("parakeet-ultra");
  rerender(<LocalTranscriptionSettings {...p} provider="local" model="parakeet-ultra" />);
  await userEvent.click(screen.getByRole("radio", { name: /Groq/ }));
  expect(p.onSelect).toHaveBeenLastCalledWith("whisper-large-v3");
});
