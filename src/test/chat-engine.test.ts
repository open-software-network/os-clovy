import { describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke: mocks.invoke,
}));

import {
  CLI_ENGINE_MODEL_PREFIX,
  type ChatEngineCatalog,
  chatDefaultEngine,
  chatEngineCatalog,
  chatModelId,
  classifyChatEngineModel,
  defaultChatCliId,
  defaultChatModelId,
} from "../lib/chat-engine";

describe("chat-engine helpers", () => {
  const sampleCatalog: ChatEngineCatalog = {
    clis: [
      {
        id: "claude",
        name: "Claude Code",
        installed: true,
        reason: null,
        modelId: "__clovy_cli_engine__:claude",
      },
      {
        id: "pi",
        name: "Pi",
        installed: false,
        reason: "Executable not found",
        modelId: "__clovy_cli_engine__:pi",
      },
    ],
  };

  it.each([
    undefined,
    null,
    "",
    "auto",
    "gpt-4o",
    "__june_local_generation__:llama3",
  ])("routes historical model %s to a CLI", (model) => {
    expect(classifyChatEngineModel(model)).toEqual({ kind: "cli", cli: "claude" });
  });

  it("retains explicit CLI selections", () => {
    expect(classifyChatEngineModel(`${CLI_ENGINE_MODEL_PREFIX}codex`)).toEqual({
      kind: "cli",
      cli: "codex",
    });
  });

  it("invokes chat_engine_catalog command", async () => {
    mocks.invoke.mockResolvedValueOnce({
      ...sampleCatalog,
      endpoints: [{ id: "old", modelId: "llama3" }],
    });
    const result = await chatEngineCatalog();
    expect(mocks.invoke).toHaveBeenCalledWith("chat_engine_catalog");
    expect(result).toEqual(sampleCatalog);
  });

  it("reads the default chat engine and falls back when chat is not on a CLI", async () => {
    mocks.invoke.mockResolvedValueOnce({
      modelId: "__clovy_cli_engine__:claude",
      cli: "claude",
    });
    expect(await chatDefaultEngine()).toEqual({
      modelId: "__clovy_cli_engine__:claude",
      cli: "claude",
    });
    expect(mocks.invoke).toHaveBeenLastCalledWith("chat_default_engine");

    mocks.invoke.mockResolvedValueOnce({ modelId: "__clovy_cli_engine__:codex" });
    expect(await defaultChatModelId()).toBe("__clovy_cli_engine__:codex");
    // Old native preferences and an unavailable bridge still select a CLI.
    mocks.invoke.mockResolvedValueOnce({ modelId: null, cli: null, legacyFallback: true });
    expect(await defaultChatModelId()).toBe("__clovy_cli_engine__:claude");
    mocks.invoke.mockResolvedValueOnce(undefined);
    expect(await defaultChatModelId()).toBe("__clovy_cli_engine__:claude");
    mocks.invoke.mockRejectedValueOnce(new Error("unknown command"));
    expect(await defaultChatModelId()).toBe("__clovy_cli_engine__:claude");
  });

  it.each([
    "claude",
    "codex",
    "agy",
  ])("resolves historical session models onto the only installed CLI %s", (only) => {
    const clis = ["claude", "codex", "agy"].map((id) => ({
      id,
      name: id,
      installed: id === only,
      reason: null,
      modelId: `${CLI_ENGINE_MODEL_PREFIX}${id}`,
    }));
    expect(chatModelId("historical-model", { clis })).toBe(`${CLI_ENGINE_MODEL_PREFIX}${only}`);
    // Explicit choices stay explicit, including a now unavailable engine.
    for (const chosen of ["claude", "codex", "agy"]) {
      expect(chatModelId(`${CLI_ENGINE_MODEL_PREFIX}${chosen}`, { clis })).toBe(
        `${CLI_ENGINE_MODEL_PREFIX}${chosen}`,
      );
    }
  });

  it("marks Claude Code as the default CLI, else Codex", () => {
    const status = (id: string, installed: boolean) => ({ id, installed });
    expect(defaultChatCliId([status("codex", true), status("claude", true)])).toBe("claude");
    expect(defaultChatCliId([status("claude", false), status("codex", true)])).toBe("codex");
    expect(defaultChatCliId([status("pi", true)])).toBe("claude");
  });
});
