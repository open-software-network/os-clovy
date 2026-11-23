import { invoke } from "./tauri";

export const CLI_ENGINE_MODEL_PREFIX = "__clovy_cli_engine__:";

export type ChatEngineCliStatus = {
  id: string;
  name: string;
  installed: boolean;
  reason: string | null;
  modelId: string;
};

export type ChatEngineCatalog = { clis: ChatEngineCliStatus[] };

/** The engines a chat session can use. Missing lists read as empty, so a
 * partial answer never breaks the composer. */
export async function chatEngineCatalog(): Promise<ChatEngineCatalog> {
  const catalog = await invoke<Partial<ChatEngineCatalog> | null | undefined>(
    "chat_engine_catalog",
  );
  return {
    clis: Array.isArray(catalog?.clis) ? catalog.clis : [],
  };
}

export type ChatEngineClassification = { kind: "cli"; cli: string };

/** Historical native and endpoint models resume on the available default CLI. */
export function classifyChatEngineModel(
  modelId: string | null | undefined,
  catalog?: Pick<ChatEngineCatalog, "clis"> | null,
): ChatEngineClassification {
  const cli = modelId?.startsWith(CLI_ENGINE_MODEL_PREFIX)
    ? modelId.slice(CLI_ENGINE_MODEL_PREFIX.length)
    : defaultChatCliId(catalog?.clis ?? []);
  return { kind: "cli", cli: cli || "claude" };
}

export function chatModelId(
  modelId: string | null | undefined,
  catalog?: ChatEngineCatalog | null,
): string {
  return `${CLI_ENGINE_MODEL_PREFIX}${classifyChatEngineModel(modelId, catalog).cli}`;
}

/** The CLIs that are the default chat engine, in order of preference
 * Claude Code, then Codex, then Antigravity. Unattended eligibility is separate. */
export const DEFAULT_CHAT_CLI_IDS = ["claude", "codex", "agy"] as const;

export type ChatDefaultEngine = { modelId: string; cli: string };

/** The engine new chat work starts on: sessions, note chat, Home handoffs,
 * and companion sessions. Needs no network. */
export async function chatDefaultEngine(): Promise<ChatDefaultEngine> {
  const engine = await invoke<Partial<ChatDefaultEngine> | null | undefined>("chat_default_engine");
  const modelId =
    typeof engine?.modelId === "string" && engine.modelId.startsWith(CLI_ENGINE_MODEL_PREFIX)
      ? engine.modelId
      : `${CLI_ENGINE_MODEL_PREFIX}claude`;
  return {
    modelId,
    cli: modelId.slice(CLI_ENGINE_MODEL_PREFIX.length),
  };
}

/** New work always starts on a CLI, including when the bridge is unavailable. */
export async function defaultChatModelId(): Promise<string> {
  try {
    return (await chatDefaultEngine()).modelId ?? `${CLI_ENGINE_MODEL_PREFIX}claude`;
  } catch {
    return `${CLI_ENGINE_MODEL_PREFIX}claude`;
  }
}

/** The CLI the engine picker marks as the default: Claude Code when it is
 * installed, else Codex, else Antigravity, else Claude Code for setup. */
export function defaultChatCliId(
  clis: ReadonlyArray<Pick<ChatEngineCliStatus, "id" | "installed">>,
): string {
  return (
    DEFAULT_CHAT_CLI_IDS.find((id) => clis.some((cli) => cli.id === id && cli.installed)) ??
    DEFAULT_CHAT_CLI_IDS[0]
  );
}
