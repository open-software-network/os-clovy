import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAgentDictationDelivery } from "../app/use-agent-dictation-delivery";
import {
  readAgentSessionDraft,
  resetAgentSessionDraftsForTests,
} from "../lib/agent-session-drafts";
const mocks = vi.hoisted(() => ({
  listener: undefined as undefined | ((event: { payload: Record<string, string> }) => void),
  claim: vi.fn(),
  ack: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name, listener) => {
    mocks.listener = listener;
    return () => {
      mocks.listener = undefined;
    };
  }),
}));
vi.mock("../lib/tauri", () => ({
  claimAgentDictationDelivery: mocks.claim,
  dictationHelperCommand: mocks.ack,
}));
const payload = {
  takeId: "hook-take",
  composerRequestId: "hook-composer",
  conversationId: "hook-conversation",
  profile: "default",
  text: "Delivered transcript",
};
describe("dictation delivery host claim", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetAgentSessionDraftsForTests();
    mocks.ack.mockResolvedValue(undefined);
  });
  it("keeps text out of every composer until its native delivery claim succeeds", async () => {
    let claim: (() => void) | undefined;
    mocks.claim.mockImplementation(
      () =>
        new Promise<void>((resolve) => {
          claim = resolve;
        }),
    );
    renderHook(() => useAgentDictationDelivery());
    await waitFor(() => expect(mocks.listener).toBeDefined());
    act(() => mocks.listener?.({ payload }));
    expect(readAgentSessionDraft(payload.conversationId)).toBeUndefined();
    expect(mocks.ack).not.toHaveBeenCalled();
    await act(async () => claim?.());
    await waitFor(() => expect(readAgentSessionDraft(payload.conversationId)).toBe(payload.text));
    expect(mocks.ack).toHaveBeenCalledWith({
      type: "agent_composer_delivered",
      takeId: payload.takeId,
      composerRequestId: payload.composerRequestId,
      ok: true,
    });
  });
  it("rejects an expired take without inserting or acknowledging text", async () => {
    mocks.claim.mockRejectedValue(new Error("turn_expired"));
    renderHook(() => useAgentDictationDelivery());
    await waitFor(() => expect(mocks.listener).toBeDefined());
    await act(async () => mocks.listener?.({ payload }));
    expect(readAgentSessionDraft(payload.conversationId)).toBeUndefined();
    expect(mocks.ack).not.toHaveBeenCalled();
  });
});
