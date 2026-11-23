import { describe, expect, it, vi } from "vitest";
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({
  invoke,
  convertFileSrc: vi.fn(),
  Channel: class {
    onmessage = (_value: unknown) => {};
  },
}));
import {
  localTranscriptionModelState,
  installLocalTranscriptionModel,
  cancelLocalTranscriptionModelInstall,
  removeLocalTranscriptionModel,
} from "../lib/tauri";
describe("local transcription bindings", () => {
  it("reads native state and forwards byte progress through a Channel", async () => {
    invoke.mockResolvedValueOnce({ state: "ready", bytes: 632162964 });
    expect(await localTranscriptionModelState()).toEqual({ state: "ready", bytes: 632162964 });
    const received = vi.fn();
    invoke.mockImplementationOnce(async (_name, args) => {
      args.channel.onmessage({ state: "downloading", received: 12, total: 100 });
    });
    await installLocalTranscriptionModel(received);
    expect(invoke).toHaveBeenLastCalledWith(
      "install_local_transcription_model",
      expect.objectContaining({ channel: expect.anything() }),
    );
    expect(received).toHaveBeenCalledWith({ state: "downloading", received: 12, total: 100 });
  });
  it("cancels and returns the persisted removal transition", async () => {
    invoke.mockResolvedValueOnce(undefined);
    await cancelLocalTranscriptionModelInstall();
    expect(invoke).toHaveBeenLastCalledWith("cancel_local_transcription_model_install");
    invoke.mockResolvedValueOnce({ switched_to_groq: true });
    expect(await removeLocalTranscriptionModel()).toEqual({ switched_to_groq: true });
    expect(invoke).toHaveBeenLastCalledWith("remove_local_transcription_model");
  });
});
