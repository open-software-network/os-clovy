import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const CODING_AGENTS_UPDATED_EVENT = "clovy://coding-agents-updated";

export type CodingAgentSourceId =
  | "claude_code"
  | "codex"
  | "copilot_cli"
  | "copilot_vscode"
  | "cursor"
  | "cursor_cli"
  | "antigravity";

/** `codingAgents` in the activity settings: which transcripts Clovy reads. */
export type CodingAgentSourcesDto = {
  claudeCode: boolean;
  codex: boolean;
  copilotCli: boolean;
  copilotVscode: boolean;
  cursor: boolean;
  cursorCli: boolean;
  antigravity: boolean;
};

export const DEFAULT_CODING_AGENT_SOURCES: CodingAgentSourcesDto = {
  claudeCode: false,
  codex: false,
  copilotCli: false,
  copilotVscode: false,
  cursor: false,
  cursorCli: false,
  antigravity: false,
};

/** Settings field of each source, in display order. */
export const CODING_AGENT_SOURCE_FIELDS: ReadonlyArray<
  [CodingAgentSourceId, keyof CodingAgentSourcesDto]
> = [
  ["claude_code", "claudeCode"],
  ["codex", "codex"],
  ["copilot_cli", "copilotCli"],
  ["copilot_vscode", "copilotVscode"],
  ["cursor", "cursor"],
  ["cursor_cli", "cursorCli"],
  ["antigravity", "antigravity"],
];

/** Product names, not translated. */
export const CODING_AGENT_NAMES: Record<CodingAgentSourceId, string> = {
  claude_code: "Claude Code",
  codex: "Codex",
  copilot_cli: "Copilot CLI",
  copilot_vscode: "Copilot in VS Code",
  cursor: "Cursor",
  cursor_cli: "Cursor CLI",
  antigravity: "Antigravity",
};

export type CodingAgentSourceStatusDto = {
  id: CodingAgentSourceId;
  name: string;
  enabled: boolean;
  present: boolean;
};

export type CodingAgentsStatusDto = {
  supported: boolean;
  sources: CodingAgentSourceStatusDto[];
  databaseReady: boolean;
  lastScanAt: string | null;
  lastError: string | null;
};

export type CodingAgentBlockState = "live" | "sealed" | "summarized";

export type CodingAgentBlockDto = {
  id: number;
  source: CodingAgentSourceId;
  sessionId: string;
  startedAt: string;
  endedAt: string;
  cwd: string | null;
  project: string | null;
  title: string | null;
  firstPrompt: string | null;
  promptCount: number;
  replyCount: number;
  activeSeconds: number;
  state: CodingAgentBlockState;
  sealedAt: string | null;
  summary: string | null;
  summarySource: string | null;
  summaryAttempts: number;
  summaryError: string | null;
};

export async function codingAgentsStatus(): Promise<CodingAgentsStatusDto> {
  return invoke<CodingAgentsStatusDto>("coding_agents_status");
}

/** Blocks overlapping `[from, to)`, oldest first. */
export async function codingAgentBlocks(from: Date, to: Date): Promise<CodingAgentBlockDto[]> {
  return invoke<CodingAgentBlockDto[]>("coding_agents_blocks", {
    request: { from: from.toISOString(), to: to.toISOString() },
  });
}

export async function onCodingAgentsUpdated(callback: () => void): Promise<UnlistenFn> {
  return listen(CODING_AGENTS_UPDATED_EVENT, () => {
    callback();
  });
}
