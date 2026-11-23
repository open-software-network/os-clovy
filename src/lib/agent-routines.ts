import { t } from "../i18n/translate";
import { invoke } from "@tauri-apps/api/core";
import { memorySettings } from "./tauri";

export type RoutineJob = {
  job_id: string;
  name: string;
  prompt: string;
  prompt_preview: string;
  schedule: string;
  repeat: string;
  deliver: string;
  created_at: string | null;
  next_run_at: string | null;
  last_run_at: string | null;
  last_status: "ok" | "error" | null;
  last_error?: string | null;
  last_delivery_error?: string | null;
  enabled: boolean;
  state: "scheduled" | "paused" | "completed";
  paused_reason?: string | null;
  enabled_toolsets?: string[];
  script?: string | null;
  no_agent?: boolean;
  model?: string;
  safety_mode?: "sandboxed" | "unrestricted";
};

type AgentRoutineDto = {
  id: string;
  name: string;
  prompt: string;
  schedule: string;
  repeat?: string;
  deliver?: string;
  state: "scheduled" | "paused" | "completed" | "needs_review";
  enabled: boolean;
  createdAt?: string | null;
  nextRunAt?: string | null;
  lastRunAt?: string | null;
  lastStatus?: "ok" | "error" | null;
  lastError?: string | null;
  lastDeliveryError?: string | null;
  pausedReason?: string | null;
  model?: string;
  safetyMode?: "sandboxed" | "unrestricted";
  metadata?: {
    repeat?: string;
    deliver?: string;
    enabledToolsets?: string[];
    legacyScriptPresent?: boolean;
    legacyNoAgent?: boolean;
  };
};

export type RoutineUpdates = {
  name?: string;
  schedule?: string;
  prompt?: string;
  unrestricted?: boolean;
  enabledToolsets?: string[] | null;
};

export const UNRESTRICTED_ROUTINE_TOOLSETS = [
  "terminal",
  "file",
  "code_execution",
  "web",
  "vision",
  "skills",
  "memory",
  "context_engine",
  "session_search",
];

function fromDto(dto: AgentRoutineDto): RoutineJob {
  const toolsets = dto.metadata?.enabledToolsets;
  const unrestricted = dto.safetyMode === "unrestricted";
  return {
    job_id: dto.id,
    name: dto.name,
    prompt: dto.prompt,
    prompt_preview: dto.prompt.length > 100 ? `${dto.prompt.slice(0, 100)}...` : dto.prompt,
    schedule: dto.schedule,
    repeat: dto.repeat ?? dto.metadata?.repeat ?? "forever",
    deliver: dto.deliver ?? dto.metadata?.deliver ?? "local",
    created_at: dto.createdAt ?? null,
    next_run_at: dto.nextRunAt ?? null,
    last_run_at: dto.lastRunAt ?? null,
    last_status: dto.lastStatus ?? null,
    last_error: dto.lastError ?? null,
    last_delivery_error: dto.lastDeliveryError ?? null,
    enabled: dto.enabled,
    state: dto.state === "needs_review" ? "paused" : dto.state,
    paused_reason:
      dto.pausedReason ?? (dto.state === "needs_review" ? t("lib.routines.legacyPaused") : null),
    enabled_toolsets: toolsets ?? (unrestricted ? UNRESTRICTED_ROUTINE_TOOLSETS : undefined),
    script: dto.metadata?.legacyScriptPresent ? "legacy-script-disabled" : null,
    no_agent: dto.metadata?.legacyNoAgent ?? false,
    model: dto.model,
    safety_mode: dto.safetyMode,
  };
}

async function stripMemoryIfDisabled(toolsets: string[]) {
  if (!toolsets.includes("memory")) return toolsets;
  const enabled = await memorySettings()
    .then((settings) => settings.enabled)
    .catch(() => false);
  return enabled ? toolsets : toolsets.filter((toolset) => toolset !== "memory");
}

export async function listRoutines(): Promise<RoutineJob[]> {
  return (await invoke<AgentRoutineDto[]>("list_agent_routines")).map(fromDto);
}

export async function createRoutine(input: {
  prompt: string;
  schedule: string;
  name?: string;
  unrestricted?: boolean;
  enabledToolsets?: string[];
}): Promise<RoutineJob> {
  const enabledToolsets = input.enabledToolsets
    ? await stripMemoryIfDisabled(input.enabledToolsets)
    : undefined;
  return fromDto(
    await invoke<AgentRoutineDto>("create_agent_routine", {
      request: {
        prompt: input.prompt,
        schedule: input.schedule,
        name: input.name,
        safetyMode: input.unrestricted ? "unrestricted" : "sandboxed",
        enabledToolsets,
      },
    }),
  );
}

export async function updateRoutine(
  routineId: string,
  updates: RoutineUpdates,
): Promise<RoutineJob> {
  let enabledToolsets = updates.enabledToolsets;
  if (enabledToolsets) enabledToolsets = await stripMemoryIfDisabled(enabledToolsets);
  return fromDto(
    await invoke<AgentRoutineDto>("update_agent_routine", {
      request: {
        routineId,
        name: updates.name,
        prompt: updates.prompt,
        schedule: updates.schedule,
        safetyMode:
          updates.unrestricted === undefined
            ? undefined
            : updates.unrestricted
              ? "unrestricted"
              : "sandboxed",
        enabledToolsets,
      },
    }),
  );
}

export function pauseRoutine(routineId: string) {
  return invoke<AgentRoutineDto>("pause_agent_routine", { routineId }).then(fromDto);
}

export function resumeRoutine(routineId: string) {
  return invoke<AgentRoutineDto>("resume_agent_routine", { routineId }).then(fromDto);
}

export function triggerRoutine(routineId: string) {
  return invoke<void>("trigger_agent_routine", { routineId });
}

export function removeRoutine(routineId: string) {
  return invoke<void>("delete_agent_routine", { routineId });
}

export function routineUnrestricted(
  routine: Pick<RoutineJob, "enabled_toolsets" | "script" | "no_agent" | "safety_mode">,
) {
  return (
    routine.safety_mode === "unrestricted" ||
    Boolean(routine.script || routine.no_agent) ||
    (routine.enabled_toolsets ?? []).some((toolset) =>
      ["terminal", "file", "code_execution", "skills"].includes(toolset),
    )
  );
}

export async function routineCreationPrompt(
  description: string,
  options?: { unrestricted?: boolean },
) {
  return [
    "Create a new Clovy routine for me.",
    `Here is what it should do: ${description.trim()}`,
    options?.unrestricted
      ? "I chose unrestricted mode for this routine."
      : "I chose the sandboxed default for this routine.",
    "Ask me for any timing details that are unclear, then create it and confirm when it will first run.",
  ].join("\n\n");
}
